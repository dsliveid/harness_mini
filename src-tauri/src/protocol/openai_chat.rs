//! OpenAI 兼容 Chat Completions 协议流式适配器 (/chat/completions)

use crate::llm::{get_client, finish, truncate, LlmCfg, LlmResult, ToolCallAcc};
use futures::StreamExt;
use serde_json::{json, Value};

/// 规整 endpoint：容忍用户把完整路径 /chat/completions 填进 Base URL
pub fn endpoint(base_url: &str) -> String {
    let b = base_url.trim().trim_end_matches('/');
    let b = b.strip_suffix("/chat/completions").unwrap_or(b);
    format!("{b}/chat/completions")
}

pub async fn chat_stream(
    cfg: &LlmCfg,
    messages: &[Value],
    tools: &[Value],
    on_text: impl FnMut(&str) + Send,
    on_reasoning: impl FnMut(&str) + Send,
) -> Result<LlmResult, String> {
    chat_stream_full(cfg, messages, tools, on_text, on_reasoning, |_| {}).await
}

pub async fn chat_stream_full(
    cfg: &LlmCfg,
    messages: &[Value],
    tools: &[Value],
    mut on_text: impl FnMut(&str) + Send,
    mut on_reasoning: impl FnMut(&str) + Send,
    mut on_tool: impl FnMut(crate::llm::ToolCallChunk) + Send,
) -> Result<LlmResult, String> {
    let url = endpoint(&cfg.base_url);
    let payload = crate::protocol::transpiler::transpile_to_openai_chat(
        messages,
        tools,
        &cfg.model,
        cfg.reasoning_effort.as_deref(),
    );
    let mut body = json!({
        "model": cfg.model,
        "messages": payload.messages,
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    if !payload.tools.is_empty() {
        body["tools"] = Value::Array(payload.tools);
    }
    // 思考程度处理：仅当明确指定且非 "default" 时才传入 reasoning_effort
    if let Some(ref effort) = cfg.reasoning_effort {
        let trimmed = effort.trim();
        if !trimmed.is_empty() && trimmed != "default" {
            body["reasoning_effort"] = json!(trimmed);
        }
    }

    let session_id = cfg.effective_session_id();
    let resp = get_client(cfg.proxy_url.as_deref())
        .post(&url)
        .bearer_auth(&cfg.api_key)
        .header("x-opencode-session", &session_id)
        .header("x-session-id", &session_id)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("LLM 返回 {status}: {}", truncate(&text, 2000)));
    }

    let mut stream = resp.bytes_stream();
    let mut buffer: Vec<u8> = Vec::new();
    let mut result = LlmResult::default();
    let mut calls: Vec<Option<ToolCallAcc>> = Vec::new();

    let mut is_first_chunk = true;
    loop {
        let chunk_timeout = if is_first_chunk {
            std::time::Duration::from_secs(60) // 首字预填充允许最长 60s
        } else {
            std::time::Duration::from_secs(25) // 吐字中途若停顿 25s 判定网络半开挂死
        };

        let chunk = match tokio::time::timeout(chunk_timeout, stream.next()).await {
            Ok(Some(c)) => {
                is_first_chunk = false;
                c.map_err(|e| format!("流读取失败: {e}"))?
            }
            Ok(None) => break,
            Err(_) => {
                let reason = if is_first_chunk {
                    "首字等待超时 (60s 无响应)"
                } else {
                    "数据块传输停滞 (25s 无新数据)"
                };
                return Err(format!("流读取失败: 网络响应超时（{reason}），远程模型服务停止响应"));
            }
        };
        buffer.extend_from_slice(&chunk);

        while let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = buffer.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end();
            let Some(data) = line.strip_prefix("data:") else { continue };
            let data = data.trim();
            if data == "[DONE]" {
                return Ok(finish(result, &mut calls));
            }
            if data.is_empty() {
                continue;
            }
            let v: Value = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if let Some(u) = v.get("usage") {
                if u.is_object() {
                    result.usage = Some(u.clone());
                }
            }
            let Some(choices) = v.get("choices").and_then(|c| c.as_array()) else { continue };
            let Some(delta) = choices.get(0).and_then(|c| c.get("delta")) else { continue };
            if let Some(text) = delta.get("content").and_then(|c| c.as_str()) {
                if !text.is_empty() {
                    result.content.push_str(text);
                    on_text(text);
                }
            }
            // 推理型模型（DeepSeek R1、GLM-Reasoning 等）思考过程
            if let Some(text) = delta
                .get("reasoning_content")
                .or_else(|| delta.get("reasoning"))
                .and_then(|c| c.as_str())
            {
                if !text.is_empty() {
                    result.reasoning.push_str(text);
                    on_reasoning(text);
                }
            }
            if let Some(tcs) = delta.get("tool_calls").and_then(|c| c.as_array()) {
                for tc in tcs {
                    let idx = tc
                        .get("index")
                        .and_then(|i| i.as_u64())
                        .unwrap_or(calls.len() as u64) as usize;
                    while calls.len() <= idx {
                        calls.push(None);
                    }
                    let slot = &mut calls[idx];
                    let is_new = slot.is_none();
                    if slot.is_none() {
                        *slot = Some(ToolCallAcc {
                            id: String::new(),
                            name: String::new(),
                            args: String::new(),
                        });
                    }
                    let slot = slot.as_mut().unwrap();
                    let mut id_opt = None;
                    if let Some(id) = tc.get("id").and_then(|i| i.as_str()) {
                        slot.id.push_str(id);
                        id_opt = Some(id);
                    }
                    let mut name_opt = None;
                    if let Some(name) = tc
                        .get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(|n| n.as_str())
                    {
                        slot.name.push_str(name);
                        name_opt = Some(name);
                    }
                    let mut args_opt = None;
                    if let Some(args) = tc
                        .get("function")
                        .and_then(|f| f.get("arguments"))
                        .and_then(|a| a.as_str())
                    {
                        slot.args.push_str(args);
                        args_opt = Some(args);
                    }
                    on_tool(crate::llm::ToolCallChunk {
                        index: idx,
                        id: id_opt.or(if is_new && !slot.id.is_empty() { Some(&slot.id) } else { None }),
                        name: name_opt.or(if is_new && !slot.name.is_empty() { Some(&slot.name) } else { None }),
                        args_delta: args_opt,
                    });
                }
            }
        }
    }
    Ok(finish(result, &mut calls))
}
