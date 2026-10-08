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
    mut on_text: impl FnMut(&str) + Send,
    mut on_reasoning: impl FnMut(&str) + Send,
) -> Result<LlmResult, String> {
    let url = endpoint(&cfg.base_url);
    let mut body = json!({
        "model": cfg.model,
        "messages": messages,
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools.to_vec());
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

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("流读取失败: {e}"))?;
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
                    if slot.is_none() {
                        *slot = Some(ToolCallAcc {
                            id: String::new(),
                            name: String::new(),
                            args: String::new(),
                        });
                    }
                    let slot = slot.as_mut().unwrap();
                    if let Some(id) = tc.get("id").and_then(|i| i.as_str()) {
                        slot.id.push_str(id);
                    }
                    if let Some(name) = tc
                        .get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(|n| n.as_str())
                    {
                        slot.name.push_str(name);
                    }
                    if let Some(args) = tc
                        .get("function")
                        .and_then(|f| f.get("arguments"))
                        .and_then(|a| a.as_str())
                    {
                        slot.args.push_str(args);
                    }
                }
            }
        }
    }
    Ok(finish(result, &mut calls))
}
