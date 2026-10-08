//! OpenAI Responses 协议流式适配器 (/v1/responses)

use super::transpiler::transpile_to_responses;
use crate::llm::{get_client, finish, truncate, LlmCfg, LlmResult, ToolCallAcc};
use futures::StreamExt;
use serde_json::{json, Value};

/// 规范化 OpenAI /v1/responses 端点
pub fn endpoint(base_url: &str) -> String {
    let b = base_url.trim().trim_end_matches('/');
    let b = b
        .strip_suffix("/v1/responses")
        .or_else(|| b.strip_suffix("/responses"))
        .unwrap_or(b);
    if b.ends_with("/v1") {
        format!("{b}/responses")
    } else {
        format!("{b}/v1/responses")
    }
}

pub async fn chat_stream(
    cfg: &LlmCfg,
    messages: &[Value],
    tools: &[Value],
    mut on_text: impl FnMut(&str) + Send,
    mut on_reasoning: impl FnMut(&str) + Send,
) -> Result<LlmResult, String> {
    let url = endpoint(&cfg.base_url);
    let payload = transpile_to_responses(messages, tools);

    let mut body = json!({
        "model": cfg.model,
        "input": payload.input,
        "stream": true,
    });

    if let Some(inst) = payload.instructions {
        body["instructions"] = Value::String(inst);
    }

    if !payload.tools.is_empty() {
        body["tools"] = Value::Array(payload.tools);
    }

    let session_id = cfg.effective_session_id();
    let resp = get_client(cfg.proxy_url.as_deref())
        .post(&url)
        .bearer_auth(&cfg.api_key)
        .header("content-type", "application/json")
        .header("x-opencode-session", &session_id)
        .header("x-session-id", &session_id)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Responses 请求失败: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Responses API 返回 {status}: {}", truncate(&text, 2000)));
    }

    let mut stream = resp.bytes_stream();
    let mut buffer: Vec<u8> = Vec::new();
    let mut result = LlmResult::default();
    let mut calls: Vec<Option<ToolCallAcc>> = Vec::new();

    let mut curr_event_name = String::new();
    let mut curr_call_idx: Option<usize> = None;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("流读取失败: {e}"))?;
        buffer.extend_from_slice(&chunk);

        while let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = buffer.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end();

            if let Some(event) = line.strip_prefix("event:") {
                curr_event_name = event.trim().to_string();
                continue;
            }

            let Some(data) = line.strip_prefix("data:") else { continue };
            let data = data.trim();
            if data.is_empty() || data == "[DONE]" {
                continue;
            }

            let v: Value = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // 获取事件类型（可能在 event 行，也可能在 JSON 内部的 type 字段）
            let event_type = v
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or(&curr_event_name);

            match event_type {
                "response.output_item.added" => {
                    if let Some(item) = v.get("item") {
                        if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                            let call_id = item
                                .get("call_id")
                                .or_else(|| item.get("id"))
                                .and_then(|i| i.as_str())
                                .unwrap_or("")
                                .to_string();
                            let name = item
                                .get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or("")
                                .to_string();
                            let idx = calls.len();
                            calls.push(Some(ToolCallAcc {
                                id: call_id,
                                name,
                                args: String::new(),
                            }));
                            curr_call_idx = Some(idx);
                        }
                    }
                }
                "response.text.delta" => {
                    if let Some(delta) = v.get("delta").and_then(|d| d.as_str()) {
                        if !delta.is_empty() {
                            result.content.push_str(delta);
                            on_text(delta);
                        }
                    }
                }
                "response.reasoning.delta" | "response.thought.delta" => {
                    if let Some(delta) = v.get("delta").and_then(|d| d.as_str()) {
                        if !delta.is_empty() {
                            result.reasoning.push_str(delta);
                            on_reasoning(delta);
                        }
                    }
                }
                "response.function_call_arguments.delta" => {
                    if let Some(delta) = v.get("delta").and_then(|d| d.as_str()) {
                        if let Some(idx) = curr_call_idx {
                            if idx < calls.len() {
                                if let Some(ref mut c) = calls[idx] {
                                    c.args.push_str(delta);
                                }
                            }
                        } else if let Some(last) = calls.last_mut().and_then(|c| c.as_mut()) {
                            last.args.push_str(delta);
                        }
                    }
                }
                "response.output_item.done" => {
                    curr_call_idx = None;
                }
                "response.completed" | "response.done" => {
                    if let Some(response_obj) = v.get("response") {
                        if let Some(usage) = response_obj.get("usage") {
                            result.usage = Some(usage.clone());
                        }
                    }
                    return Ok(finish(result, &mut calls));
                }
                "error" => {
                    let msg = v
                        .get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|m| m.as_str())
                        .unwrap_or("Responses API 返回错误");
                    return Err(msg.to_string());
                }
                _ => {}
            }
        }
    }

    Ok(finish(result, &mut calls))
}
