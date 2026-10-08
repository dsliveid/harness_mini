//! Anthropic Claude Messages 协议流式适配器 (/v1/messages)

use super::transpiler::transpile_to_anthropic;
use crate::llm::{get_client, finish, truncate, LlmCfg, LlmResult, ToolCallAcc};
use futures::StreamExt;
use serde_json::{json, Value};

/// 规范化 Anthropic /v1/messages 端点
pub fn endpoint(base_url: &str) -> String {
    let b = base_url.trim().trim_end_matches('/');
    let b = b
        .strip_suffix("/v1/messages")
        .or_else(|| b.strip_suffix("/messages"))
        .unwrap_or(b);
    if b.ends_with("/v1") {
        format!("{b}/messages")
    } else {
        format!("{b}/v1/messages")
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
    let payload = transpile_to_anthropic(messages, tools);

    let mut body = json!({
        "model": cfg.model,
        "messages": payload.messages,
        "max_tokens": 8192,
        "stream": true,
    });

    if let Some(sys) = payload.system {
        body["system"] = Value::String(sys);
    }

    if !payload.tools.is_empty() {
        body["tools"] = Value::Array(payload.tools);
    }

    // 思考程度适配（针对 Claude 3.7+ 原生 extended thinking）
    if let Some(ref effort) = cfg.reasoning_effort {
        let trimmed = effort.trim();
        if trimmed == "low" || trimmed == "medium" || trimmed == "high" {
            let budget = match trimmed {
                "low" => 2048,
                "medium" => 4096,
                "high" => 8192,
                _ => 2048,
            };
            body["thinking"] = json!({
                "type": "enabled",
                "budget_tokens": budget
            });
            // 启用 thinking 时 max_tokens 必须大于 budget_tokens
            body["max_tokens"] = json!(budget + 4096);
        }
    }

    let session_id = cfg.effective_session_id();
    let resp = get_client(cfg.proxy_url.as_deref())
        .post(&url)
        .header("x-api-key", &cfg.api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .header("x-opencode-session", &session_id)
        .header("x-session-id", &session_id)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Anthropic 请求失败: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Anthropic 返回 {status}: {}", truncate(&text, 2000)));
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
            if data.is_empty() || data == "[DONE]" {
                continue;
            }

            let v: Value = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let event_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
            match event_type {
                "content_block_start" => {
                    let idx = v.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                    if let Some(block) = v.get("content_block") {
                        let b_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        if b_type == "tool_use" {
                            let id = block.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string();
                            let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
                            while calls.len() <= idx {
                                calls.push(None);
                            }
                            calls[idx] = Some(ToolCallAcc {
                                id,
                                name,
                                args: String::new(),
                            });
                        }
                    }
                }
                "content_block_delta" => {
                    let idx = v.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                    if let Some(delta) = v.get("delta") {
                        let d_type = delta.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        match d_type {
                            "text_delta" => {
                                if let Some(text) = delta.get("text").and_then(|s| s.as_str()) {
                                    if !text.is_empty() {
                                        result.content.push_str(text);
                                        on_text(text);
                                    }
                                }
                            }
                            "thinking_delta" => {
                                if let Some(thinking) = delta.get("thinking").and_then(|s| s.as_str()) {
                                    if !thinking.is_empty() {
                                        result.reasoning.push_str(thinking);
                                        on_reasoning(thinking);
                                    }
                                }
                            }
                            "input_json_delta" => {
                                if let Some(partial) = delta.get("partial_json").and_then(|s| s.as_str()) {
                                    if idx < calls.len() {
                                        if let Some(ref mut call) = calls[idx] {
                                            call.args.push_str(partial);
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                "content_block_stop" => {}
                "message_delta" => {
                    if let Some(usage) = v.get("usage") {
                        result.usage = Some(usage.clone());
                    }
                }
                "message_stop" => {
                    return Ok(finish(result, &mut calls));
                }
                "error" => {
                    let msg = v.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()).unwrap_or("Anthropic 流返回错误");
                    return Err(msg.to_string());
                }
                _ => {}
            }
        }
    }

    Ok(finish(result, &mut calls))
}
