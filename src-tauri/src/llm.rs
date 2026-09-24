use serde_json::{json, Value};
use std::sync::OnceLock;

pub struct LlmCfg {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub reasoning_effort: Option<String>,
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
            .tcp_keepalive(std::time::Duration::from_secs(15))
            .pool_idle_timeout(std::time::Duration::from_secs(60))
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
    // 思考程度处理：仅当明确指定且非 "default" 时才传入 reasoning_effort（支持模型默认不传，防止非推理模型报错 400）
    if let Some(ref effort) = cfg.reasoning_effort {
        let trimmed = effort.trim();
        if !trimmed.is_empty() && trimmed != "default" {
            body["reasoning_effort"] = json!(trimmed);
        }
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

/// 调用 OpenAI 兼容 /images/generations 接口生成图片，并返回图片二进制数据
pub async fn generate_image_api(
    cfg: &LlmCfg,
    prompt: &str,
    size: Option<&str>,
) -> Result<Vec<u8>, String> {
    use base64::Engine;
    if cfg.api_key.trim().is_empty() {
        return Err("当前厂商的 API Key 为空，请在设置中配置并保存有效的 API Key 后重试".into());
    }

    let b = cfg.base_url.trim().trim_end_matches('/');
    let b = b.strip_suffix("/chat/completions").unwrap_or(b);
    let b = b.strip_suffix("/images/generations").unwrap_or(b);
    let url = format!("{b}/images/generations");
    let mut body = json!({
        "model": cfg.model,
        "prompt": prompt,
        "n": 1,
    });
    if let Some(s) = size {
        body["size"] = json!(s);
    }

    let resp = client()
        .post(&url)
        .bearer_auth(cfg.api_key.trim())
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("生图请求失败: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.as_u16() == 400 && (text.contains("image generation is only supported by certain models") || text.contains("InvalidParameter")) {
            return Err(format!(
                "生图接口返回 400 Bad Request：模型【{}】不是该服务商支持的生图模型。\n云端提示：{}\n\n请在【设置 -> 厂商配置】中检查并配置该服务商正确的生图模型（如 doubao-seedream-5.0-lite、cogview-3-plus、dall-e-3 等），或在创建/编辑协作者时指定生图模型。",
                cfg.model,
                truncate(&text, 400)
            ));
        }
        if status.as_u16() == 401 {
            return Err(format!(
                "生图接口返回 401 Unauthorized：API Key 无效或未授权。\n云端提示：{}\n\n请在【设置 -> 厂商配置】中检查并重新填写该厂商的有效 API Key。",
                truncate(&text, 400)
            ));
        }
        return Err(format!("生图接口返回 {status}: {}", truncate(&text, 2000)));
    }

    let val: Value = resp.json().await.map_err(|e| format!("生图响应解析失败: {e}"))?;

    // 1. 尝试提取 OpenAI 标准 data[0].b64_json
    if let Some(b64) = val.get("data")
        .and_then(|d| d.get(0))
        .and_then(|item| item.get("b64_json"))
        .and_then(|v| v.as_str())
    {
        return base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| format!("Base64 解码失败: {e}"));
    }

    // 2. 尝试提取 images[0] (部分 WebUI 或本地生图服务返回)
    if let Some(b64) = val.get("images")
        .and_then(|d| d.get(0))
        .and_then(|v| v.as_str())
    {
        let clean_b64 = if let Some(idx) = b64.find(',') {
            &b64[idx + 1..]
        } else {
            b64
        };
        return base64::engine::general_purpose::STANDARD
            .decode(clean_b64)
            .map_err(|e| format!("Base64 解码失败: {e}"));
    }

    // 3. 尝试提取 data[0].url 并发起 HTTP GET 下载
    if let Some(img_url) = val.get("data")
        .and_then(|d| d.get(0))
        .and_then(|item| item.get("url"))
        .and_then(|v| v.as_str())
    {
        let img_resp = client()
            .get(img_url)
            .send()
            .await
            .map_err(|e| format!("下载生成的图片失败: {e}"))?;
        if !img_resp.status().is_success() {
            return Err(format!("下载图片失败，HTTP 状态: {}", img_resp.status()));
        }
        let bytes = img_resp.bytes().await.map_err(|e| format!("读取图片数据失败: {e}"))?;
        return Ok(bytes.to_vec());
    }

    Err(format!("未能从生图响应中解析到图片数据: {}", truncate(&val.to_string(), 500)))
}

