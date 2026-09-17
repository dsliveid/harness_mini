use serde_json::{json, Value};
use std::sync::OnceLock;

pub struct LlmCfg {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

/// 规整 endpoint：容忍用户把完整路径 /chat/completions 填进 Base URL
fn endpoint(base_url: &str) -> String {
    let b = base_url.trim().trim_end_matches('/');
    let b = b.strip_suffix("/chat/completions").unwrap_or(b);
    format!("{b}/chat/completions")
}

pub struct ToolCallAcc {
    pub id: String,
    pub name: String,
    pub args: String,
}

#[derive(Default)]
pub struct LlmResult {
    pub content: String,
    /// 模型思考过程（reasoning_content / reasoning），与正文分开累积
    pub reasoning: String,
    pub tool_calls: Vec<ToolCallAcc>,
    pub usage: Option<Value>,
}

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(20))
            .build()
            .expect("reqwest client")
    })
}

/// 调用 OpenAI 兼容 /chat/completions（流式）。
/// `on_text` 在每个内容增量到达时被调用（用于向 UI 推送）。
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

    let resp = client()
        .post(&url)
        .bearer_auth(&cfg.api_key)
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

    use futures::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("流读取失败: {e}"))?;
        buffer.extend_from_slice(&chunk);
        // SSE 事件以空行分隔，但按行解析 "data:" 更简单可靠
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
            // 推理型模型（DeepSeek R1、GLM-Reasoning 等）的思考过程：
            // OpenAI 兼容端点常见字段为 reasoning_content，部分实现用 reasoning
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

fn finish(mut result: LlmResult, calls: &mut Vec<Option<ToolCallAcc>>) -> LlmResult {
    for c in calls.iter_mut() {
        if let Some(c) = c.take() {
            result.tool_calls.push(c);
        }
    }
    result
}

/// 非流式连接测试
pub async fn test_connection(cfg: &LlmCfg) -> Result<String, String> {
    let url = endpoint(&cfg.base_url);
    let body = json!({
        "model": cfg.model,
        "messages": [{"role": "user", "content": "ping"}],
        "max_tokens": 1,
    });
    let resp = client()
        .post(&url)
        .bearer_auth(&cfg.api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("连接失败: {e}"))?;
    let status = resp.status();
    if status.is_success() {
        Ok(format!("连接成功（{status}）"))
    } else {
        let text = resp.text().await.unwrap_or_default();
        Err(format!("服务端返回 {status}: {}", truncate(&text, 500)))
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut end = n;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}
