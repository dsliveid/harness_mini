//! 即时协议转译器（JIT Context Transpiler）
//!
//! 将存储层及内部规范的通用消息模型即时转译为目标协议私有 Payload：
//! - Anthropic Claude Messages (/v1/messages)
//! - OpenAI Responses (/v1/responses)
//! - 自动处理角色交替合并 (Squashing)、孤立 Tool Call 自愈、图片多模态转换与 System 提取。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicPayload {
    pub system: Option<String>,
    pub messages: Vec<Value>,
    pub tools: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesPayload {
    pub instructions: Option<String>,
    pub input: Vec<Value>,
    pub tools: Vec<Value>,
}

/// 将内部通用消息序列转译为 Anthropic Claude /v1/messages 格式
pub fn transpile_to_anthropic(messages: &[Value], tools: &[Value]) -> AnthropicPayload {
    let mut system_parts = Vec::new();
    let mut raw_converted: Vec<Value> = Vec::new();

    for m in messages {
        let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("");
        match role {
            "system" => {
                if let Some(content) = m.get("content").and_then(|c| c.as_str()) {
                    let trimmed = content.trim();
                    if !trimmed.is_empty() {
                        system_parts.push(trimmed.to_string());
                    }
                }
            }
            "user" => {
                let mut blocks = Vec::new();
                if let Some(content_str) = m.get("content").and_then(|c| c.as_str()) {
                    if !content_str.is_empty() {
                        blocks.push(json!({
                            "type": "text",
                            "text": content_str
                        }));
                    }
                } else if let Some(parts) = m.get("content").and_then(|c| c.as_array()) {
                    for part in parts {
                        let p_type = part.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        if p_type == "text" {
                            if let Some(t) = part.get("text").and_then(|s| s.as_str()) {
                                blocks.push(json!({
                                    "type": "text",
                                    "text": t
                                }));
                            }
                        } else if p_type == "image_url" {
                            if let Some(url) = part
                                .get("image_url")
                                .and_then(|u| u.get("url"))
                                .and_then(|s| s.as_str())
                            {
                                if let Some((mime, b64)) = parse_data_url(url) {
                                    blocks.push(json!({
                                        "type": "image",
                                        "source": {
                                            "type": "base64",
                                            "media_type": mime,
                                            "data": b64
                                        }
                                    }));
                                }
                            }
                        }
                    }
                }

                if blocks.is_empty() {
                    blocks.push(json!({"type": "text", "text": "..."}));
                }

                raw_converted.push(json!({
                    "role": "user",
                    "content": blocks
                }));
            }
            "assistant" => {
                let mut blocks = Vec::new();
                if let Some(content_str) = m.get("content").and_then(|c| c.as_str()) {
                    if !content_str.is_empty() {
                        blocks.push(json!({
                            "type": "text",
                            "text": content_str
                        }));
                    }
                }

                if let Some(tcs) = m.get("tool_calls").and_then(|t| t.as_array()) {
                    for tc in tcs {
                        let id = tc.get("id").and_then(|i| i.as_str()).unwrap_or("");
                        let name = tc
                            .get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(|n| n.as_str())
                            .unwrap_or("");
                        let args_str = tc
                            .get("function")
                            .and_then(|f| f.get("arguments"))
                            .and_then(|a| a.as_str())
                            .unwrap_or("{}");
                        let input_val: Value =
                            serde_json::from_str(args_str).unwrap_or_else(|_| json!({}));

                        blocks.push(json!({
                            "type": "tool_use",
                            "id": id,
                            "name": name,
                            "input": input_val
                        }));
                    }
                }

                if blocks.is_empty() {
                    blocks.push(json!({"type": "text", "text": "..."}));
                }

                raw_converted.push(json!({
                    "role": "assistant",
                    "content": blocks
                }));
            }
            "tool" => {
                // Claude 协议中工具执行结果属于 user 角色的 tool_result block
                let call_id = m.get("tool_call_id").and_then(|i| i.as_str()).unwrap_or("");
                let text = m.get("content").and_then(|c| c.as_str()).unwrap_or("");
                raw_converted.push(json!({
                    "role": "user",
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": call_id,
                            "content": text
                        }
                    ]
                }));
            }
            _ => {}
        }
    }

    // 1. 连续相同角色消息合并 (Message Squashing)
    let mut squashed: Vec<Value> = Vec::new();
    for msg in raw_converted {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
        let content_blocks = msg
            .get("content")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default();

        if let Some(last) = squashed.last_mut() {
            if last.get("role").and_then(|r| r.as_str()) == Some(role) {
                if let Some(last_blocks) = last.get_mut("content").and_then(|c| c.as_array_mut()) {
                    last_blocks.extend(content_blocks);
                    continue;
                }
            }
        }

        squashed.push(json!({
            "role": role,
            "content": content_blocks
        }));
    }

    // 2. 首条消息必须为 user 角色保证
    if squashed.is_empty() {
        squashed.push(json!({
            "role": "user",
            "content": [{"type": "text", "text": "开始对话"}]
        }));
    } else if squashed[0].get("role").and_then(|r| r.as_str()) != Some("user") {
        squashed.insert(
            0,
            json!({
                "role": "user",
                "content": [{"type": "text", "text": "开始对话"}]
            }),
        );
    }

    // 3. 孤立 Tool Call 自动自愈 (Orphan Tool Call Auto-Repair)
    // 确保任何 assistant 中的 tool_use block 在紧随其后的 user 消息中都有匹配的 tool_result block
    let mut i = 0;
    while i < squashed.len() {
        if squashed[i].get("role").and_then(|r| r.as_str()) == Some("assistant") {
            let mut expected_tool_ids = Vec::new();
            if let Some(blocks) = squashed[i].get("content").and_then(|c| c.as_array()) {
                for b in blocks {
                    if b.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                        if let Some(id) = b.get("id").and_then(|s| s.as_str()) {
                            if !id.is_empty() {
                                expected_tool_ids.push(id.to_string());
                            }
                        }
                    }
                }
            }

            if !expected_tool_ids.is_empty() {
                // 检查下一条消息是否是 user
                let has_next_user = if i + 1 < squashed.len() {
                    squashed[i + 1].get("role").and_then(|r| r.as_str()) == Some("user")
                } else {
                    false
                };

                if !has_next_user {
                    // 若无后续 user 消息，则创建一条虚拟 user 消息承载所有 missing tool_result
                    let missing_results: Vec<Value> = expected_tool_ids
                        .into_iter()
                        .map(|id| {
                            json!({
                                "type": "tool_result",
                                "tool_use_id": id,
                                "content": "(工具调用已被中断，无执行结果)"
                            })
                        })
                        .collect();
                    squashed.insert(
                        i + 1,
                        json!({
                            "role": "user",
                            "content": missing_results
                        }),
                    );
                    i += 1;
                } else {
                    // 下一条是 user，检查哪些 tool_use 没有被匹配
                    let user_msg = &mut squashed[i + 1];
                    let mut existing_result_ids = std::collections::HashSet::new();
                    if let Some(blocks) = user_msg.get("content").and_then(|c| c.as_array()) {
                        for b in blocks {
                            if b.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                                if let Some(id) = b.get("tool_use_id").and_then(|s| s.as_str()) {
                                    existing_result_ids.insert(id.to_string());
                                }
                            }
                        }
                    }

                    if let Some(user_blocks) = user_msg.get_mut("content").and_then(|c| c.as_array_mut()) {
                        for expected_id in expected_tool_ids {
                            if !existing_result_ids.contains(&expected_id) {
                                user_blocks.push(json!({
                                    "type": "tool_result",
                                    "tool_use_id": expected_id,
                                    "content": "(工具调用已被中断，无执行结果)"
                                }));
                            }
                        }
                    }
                }
            }
        }
        i += 1;
    }

    // 4. 工具 Schema 转译 (OpenAI -> Anthropic)
    let mut anthropic_tools = Vec::new();
    for t in tools {
        if let Some(f) = t.get("function") {
            let name = f.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let desc = f.get("description").and_then(|d| d.as_str()).unwrap_or("");
            let schema = f
                .get("parameters")
                .cloned()
                .unwrap_or_else(|| json!({"type": "object", "properties": {}}));

            anthropic_tools.push(json!({
                "name": name,
                "description": desc,
                "input_schema": schema
            }));
        }
    }

    AnthropicPayload {
        system: if system_parts.is_empty() {
            None
        } else {
            Some(system_parts.join("\n\n"))
        },
        messages: squashed,
        tools: anthropic_tools,
    }
}

/// 将内部通用消息序列转译为 OpenAI Responses (/v1/responses) 格式
pub fn transpile_to_responses(messages: &[Value], tools: &[Value]) -> ResponsesPayload {
    let mut system_parts = Vec::new();
    let mut input_items = Vec::new();

    for m in messages {
        let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("");
        match role {
            "system" => {
                if let Some(content) = m.get("content").and_then(|c| c.as_str()) {
                    let trimmed = content.trim();
                    if !trimmed.is_empty() {
                        system_parts.push(trimmed.to_string());
                    }
                }
            }
            "user" => {
                if let Some(content_str) = m.get("content").and_then(|c| c.as_str()) {
                    input_items.push(json!({
                        "type": "message",
                        "role": "user",
                        "content": [
                            {
                                "type": "input_text",
                                "text": content_str
                            }
                        ]
                    }));
                } else if let Some(parts) = m.get("content").and_then(|c| c.as_array()) {
                    let mut block_parts = Vec::new();
                    for part in parts {
                        let p_type = part.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        if p_type == "text" {
                            if let Some(t) = part.get("text").and_then(|s| s.as_str()) {
                                block_parts.push(json!({"type": "input_text", "text": t}));
                            }
                        } else if p_type == "image_url" {
                            if let Some(url) = part
                                .get("image_url")
                                .and_then(|u| u.get("url"))
                                .and_then(|s| s.as_str())
                            {
                                block_parts.push(json!({"type": "input_image", "image_url": url}));
                            }
                        }
                    }
                    if !block_parts.is_empty() {
                        input_items.push(json!({
                            "type": "message",
                            "role": "user",
                            "content": block_parts
                        }));
                    }
                }
            }
            "assistant" => {
                let text = m.get("content").and_then(|c| c.as_str()).unwrap_or("");
                if !text.is_empty() {
                    input_items.push(json!({
                        "type": "message",
                        "role": "assistant",
                        "content": [
                            {
                                "type": "output_text",
                                "text": text
                            }
                        ]
                    }));
                }

                if let Some(tcs) = m.get("tool_calls").and_then(|t| t.as_array()) {
                    for tc in tcs {
                        let id = tc.get("id").and_then(|i| i.as_str()).unwrap_or("");
                        let name = tc
                            .get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(|n| n.as_str())
                            .unwrap_or("");
                        let args = tc
                            .get("function")
                            .and_then(|f| f.get("arguments"))
                            .and_then(|a| a.as_str())
                            .unwrap_or("{}");
                        input_items.push(json!({
                            "type": "function_call",
                            "call_id": id,
                            "name": name,
                            "arguments": args
                        }));
                    }
                }
            }
            "tool" => {
                let id = m.get("tool_call_id").and_then(|i| i.as_str()).unwrap_or("");
                let text = m.get("content").and_then(|c| c.as_str()).unwrap_or("");
                input_items.push(json!({
                    "type": "function_call_output",
                    "call_id": id,
                    "output": text
                }));
            }
            _ => {}
        }
    }

    let mut resp_tools = Vec::new();
    for t in tools {
        if let Some(f) = t.get("function") {
            let name = f.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let desc = f.get("description").and_then(|d| d.as_str()).unwrap_or("");
            let schema = f
                .get("parameters")
                .cloned()
                .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
            resp_tools.push(json!({
                "type": "function",
                "name": name,
                "description": desc,
                "parameters": schema
            }));
        }
    }

    ResponsesPayload {
        instructions: if system_parts.is_empty() {
            None
        } else {
            Some(system_parts.join("\n\n"))
        },
        input: input_items,
        tools: resp_tools,
    }
}

/// 解析 Data URL 提取 mime 类型与 Base64 编码文本
fn parse_data_url(data_url: &str) -> Option<(&str, &str)> {
    if !data_url.starts_with("data:") {
        return None;
    }
    let rest = &data_url["data:".len()..];
    let mut parts = rest.splitn(2, ";base64,");
    let mime = parts.next()?;
    let b64 = parts.next()?;
    Some((mime, b64))
}
