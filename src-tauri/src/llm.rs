use serde_json::{json, Value};
use std::sync::{OnceLock, RwLock};

#[derive(Clone, Debug)]
pub struct LlmCfg {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub protocol: crate::models::RequestProtocol,
    pub reasoning_effort: Option<String>,
    pub proxy_url: Option<String>,
    pub session_id: Option<String>,
}

impl LlmCfg {
    /// 获取当前会话 ID；若无则自动生成具有唯一性的会话 ID，保证 OpenCode / 缓存路由层的一致性与命中率
    pub fn effective_session_id(&self) -> String {
        self.session_id
            .as_deref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("sess-{}", uuid::Uuid::new_v4().simple()))
    }
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

/// 规整代理 URL：若未提供协议头，自动补齐 http://
pub fn normalize_proxy_url(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if !trimmed.starts_with("http://")
        && !trimmed.starts_with("https://")
        && !trimmed.starts_with("socks5://")
        && !trimmed.starts_with("socks5h://")
    {
        format!("http://{trimmed}")
    } else {
        trimmed.to_string()
    }
}

/// 根据可选的代理 URL 构建 reqwest::Client。
/// 若 proxy_url 为 Some 且非空，则配置全局代理并排除本地回环（localhost, 127.0.0.1, ::1）。
pub fn build_client(proxy_url: Option<&str>) -> reqwest::Client {
    let mut builder = reqwest::Client::builder()
        .user_agent("harness-mini/0.1.0")
        .connect_timeout(std::time::Duration::from_secs(25))
        .tcp_keepalive(std::time::Duration::from_secs(15))
        .pool_idle_timeout(std::time::Duration::from_secs(60));

    if let Some(raw) = proxy_url {
        let norm = normalize_proxy_url(raw);
        if !norm.is_empty() {
            if let Ok(proxy) = reqwest::Proxy::all(&norm) {
                // 排除本地回环地址，防止拦截内部通讯和本地 Ollama / vLLM
                let proxy = proxy.no_proxy(reqwest::NoProxy::from_string("localhost,127.0.0.1,::1"));
                builder = builder.proxy(proxy);
            }
        }
    }

    builder.build().unwrap_or_else(|_| reqwest::Client::new())
}

static CACHED_CLIENT: OnceLock<RwLock<(Option<String>, reqwest::Client)>> = OnceLock::new();

/// 获取带缓存的 reqwest 客户端（支持根据代理 URL 变化热刷新）
pub fn get_client(proxy_url: Option<&str>) -> reqwest::Client {
    let normalized = proxy_url.map(normalize_proxy_url).filter(|s| !s.is_empty());
    let lock = CACHED_CLIENT.get_or_init(|| {
        let c = build_client(None);
        RwLock::new((None, c))
    });

    {
        let r = lock.read().unwrap();
        if r.0 == normalized {
            return r.1.clone();
        }
    }

    let mut w = lock.write().unwrap();
    if w.0 == normalized {
        return w.1.clone();
    }
    let new_client = build_client(normalized.as_deref());
    *w = (normalized, new_client.clone());
    new_client
}

/// 调用大模型（流式），内部根据 cfg.protocol 自动路由至对应协议适配器
pub async fn chat_stream(
    cfg: &LlmCfg,
    messages: &[Value],
    tools: &[Value],
    on_text: impl FnMut(&str) + Send,
    on_reasoning: impl FnMut(&str) + Send,
) -> Result<LlmResult, String> {
    crate::protocol::dispatch_chat_stream(cfg, messages, tools, on_text, on_reasoning).await
}

pub fn finish(mut result: LlmResult, calls: &mut Vec<Option<ToolCallAcc>>) -> LlmResult {
    for c in calls.iter_mut() {
        if let Some(c) = c.take() {
            result.tool_calls.push(c);
        }
    }
    result
}

/// 非流式连接测试，根据协议自动适配
pub async fn test_connection(cfg: &LlmCfg) -> Result<String, String> {
    let session_id = cfg.effective_session_id();
    match cfg.protocol {
        crate::models::RequestProtocol::Messages => {
            let url = crate::protocol::anthropic::endpoint(&cfg.base_url);
            let body = json!({
                "model": cfg.model,
                "messages": [{"role": "user", "content": [{"type": "text", "text": "ping"}]}],
                "max_tokens": 5,
            });
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
                .map_err(|e| format!("连接失败: {e}"))?;
            let status = resp.status();
            if status.is_success() {
                Ok(format!("Claude Messages 连接成功（{status}）"))
            } else {
                let text = resp.text().await.unwrap_or_default();
                Err(format!("服务端返回 {status}: {}", truncate(&text, 500)))
            }
        }
        crate::models::RequestProtocol::Response => {
            let url = crate::protocol::openai_responses::endpoint(&cfg.base_url);
            let body = json!({
                "model": cfg.model,
                "input": [{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "ping"}]}],
            });
            let resp = get_client(cfg.proxy_url.as_deref())
                .post(&url)
                .bearer_auth(&cfg.api_key)
                .header("content-type", "application/json")
                .header("x-opencode-session", &session_id)
                .header("x-session-id", &session_id)
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("连接失败: {e}"))?;
            let status = resp.status();
            if status.is_success() {
                Ok(format!("OpenAI Responses 连接成功（{status}）"))
            } else {
                let text = resp.text().await.unwrap_or_default();
                Err(format!("服务端返回 {status}: {}", truncate(&text, 500)))
            }
        }
        crate::models::RequestProtocol::SystemOne => {
            let client = crate::jev::JevClient::new(
                cfg.base_url.clone(),
                cfg.api_key.clone(),
                cfg.model.clone(),
                5000,
                cfg.proxy_url.clone(),
            );
            let mut q = std::collections::HashMap::new();
            q.insert(
                "ping".to_string(),
                crate::jev::JevQuestion::Noul {
                    instructions: "ping test".to_string(),
                },
            );
            match client.systemone("test connectivity", q).await {
                Ok(_) => Ok("SystemOne 决策端点连通成功".into()),
                Err(e) => Err(format!("SystemOne 连通失败: {e}")),
            }
        }
        crate::models::RequestProtocol::ChatCompletions => {
            let url = endpoint(&cfg.base_url);
            let body = json!({
                "model": cfg.model,
                "messages": [{"role": "user", "content": "ping"}],
                "max_tokens": 1,
            });
            let resp = get_client(cfg.proxy_url.as_deref())
                .post(&url)
                .bearer_auth(&cfg.api_key)
                .header("x-opencode-session", &session_id)
                .header("x-session-id", &session_id)
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("连接失败: {e}"))?;
            let status = resp.status();
            if status.is_success() {
                Ok(format!("OpenAI Chat 连接成功（{status}）"))
            } else {
                let text = resp.text().await.unwrap_or_default();
                Err(format!("服务端返回 {status}: {}", truncate(&text, 500)))
            }
        }
    }
}

pub fn extract_model_ids(val: &Value) -> Vec<String> {
    let mut raw_ids = Vec::new();

    fn collect_from_array(arr: &[Value], out: &mut Vec<String>) {
        for item in arr {
            if let Some(s) = item.as_str() {
                out.push(s.to_string());
            } else if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                out.push(id.to_string());
            } else if let Some(name) = item.get("name").and_then(|v| v.as_str()) {
                let clean = name.strip_prefix("models/").unwrap_or(name);
                out.push(clean.to_string());
            } else if let Some(model) = item.get("model").and_then(|v| v.as_str()) {
                out.push(model.to_string());
            }
        }
    }

    if let Some(arr) = val.as_array() {
        collect_from_array(arr, &mut raw_ids);
    } else if let Some(obj) = val.as_object() {
        if let Some(Value::Array(arr)) = obj.get("data") {
            collect_from_array(arr, &mut raw_ids);
        } else if let Some(Value::Array(arr)) = obj.get("models") {
            collect_from_array(arr, &mut raw_ids);
        } else if let Some(Value::Array(arr)) = obj.get("items") {
            collect_from_array(arr, &mut raw_ids);
        }
    }

    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for id in raw_ids {
        let trimmed = id.trim().to_string();
        if !trimmed.is_empty() && seen.insert(trimmed.clone()) {
            result.push(trimmed);
        }
    }
    result
}

/// 获取指定厂商的模型列表
pub async fn list_models(
    provider: &crate::models::ProviderCfg,
    api_key: &str,
    proxy_url: Option<&str>,
) -> Result<Vec<String>, String> {
    let raw_base = provider.base_url.trim();
    if raw_base.is_empty() {
        return Err("Base URL 不能为空".into());
    }

    let clean_base = raw_base.trim_end_matches('/');
    let clean_base = clean_base.strip_suffix("/chat/completions").unwrap_or(clean_base);
    let clean_base = clean_base.strip_suffix("/messages").unwrap_or(clean_base);
    let clean_base = clean_base.strip_suffix("/responses").unwrap_or(clean_base);
    let clean_base = clean_base.trim_end_matches('/');

    let mut candidate_urls = Vec::new();
    if clean_base.ends_with("/models") {
        candidate_urls.push(clean_base.to_string());
    } else {
        candidate_urls.push(format!("{clean_base}/models"));
        if !clean_base.ends_with("/v1") && !clean_base.contains("/v1/") && !clean_base.contains("/v4") {
            candidate_urls.push(format!("{clean_base}/v1/models"));
        }
        candidate_urls.push(format!("{clean_base}/api/tags"));
    }

    let client = get_client(proxy_url);
    let is_anthropic = clean_base.to_lowercase().contains("anthropic");
    let mut last_error = String::new();

    for url in &candidate_urls {
        let mut req = client.get(url);
        let key = api_key.trim();
        if !key.is_empty() {
            req = req.bearer_auth(key);
            if is_anthropic || matches!(provider.model_protocols.values().next(), Some(crate::models::RequestProtocol::Messages)) {
                req = req.header("x-api-key", key).header("anthropic-version", "2023-06-01");
            }
        } else if is_anthropic {
            req = req.header("anthropic-version", "2023-06-01");
        }

        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                last_error = format!("网络请求失败: {e}");
                continue;
            }
        };

        let status = resp.status();
        if status.is_success() {
            let val: Value = match resp.json().await {
                Ok(v) => v,
                Err(e) => {
                    last_error = format!("解析 JSON 响应失败: {e}");
                    continue;
                }
            };

            let models = extract_model_ids(&val);
            if !models.is_empty() {
                return Ok(models);
            } else {
                last_error = "响应成功但未找到模型数据".to_string();
            }
        } else if status.as_u16() == 401 {
            return Err("鉴权失败 (HTTP 401)：请检查 API Key 是否正确".into());
        } else if status.as_u16() == 403 {
            return Err("访问被拒绝 (HTTP 403)：请检查 API Key 权限".into());
        } else if status.as_u16() == 404 {
            let text = resp.text().await.unwrap_or_default();
            last_error = format!("端点 404 ({url}): {}", truncate(&text, 200));
            continue;
        } else {
            let text = resp.text().await.unwrap_or_default();
            last_error = format!("服务端返回 {status}: {}", truncate(&text, 200));
        }
    }

    if last_error.is_empty() {
        Err("未能获取到模型列表，请检查网络或厂商接口配置".into())
    } else {
        Err(format!("获取模型列表失败: {last_error}"))
    }
}

/// 非流式基础对话调用
pub async fn call_simple_completion(
    cfg: &LlmCfg,
    messages: &[Value],
    temperature: Option<f32>,
) -> Result<String, String> {
    let session_id = cfg.effective_session_id();
    let url = endpoint(&cfg.base_url);
    let mut body = json!({
        "model": cfg.model,
        "messages": messages,
        "stream": false,
    });
    if let Some(temp) = temperature {
        body["temperature"] = json!(temp);
    }
    if let Some(ref effort) = cfg.reasoning_effort {
        let trimmed = effort.trim();
        if !trimmed.is_empty() && trimmed != "default" {
            body["reasoning_effort"] = json!(trimmed);
        }
    }
    let resp = get_client(cfg.proxy_url.as_deref())
        .post(&url)
        .bearer_auth(&cfg.api_key)
        .header("x-opencode-session", &session_id)
        .header("x-session-id", &session_id)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("LLM请求失败: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("LLM返回 {status}: {}", truncate(&text, 1000)));
    }
    let val: Value = resp.json().await.map_err(|e| format!("解析响应失败: {e}"))?;
    let content = val.get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|s| s.as_str())
        .unwrap_or_default()
        .to_string();
    Ok(content)
}

pub fn truncate(s: &str, n: usize) -> String {
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

    let session_id = cfg.effective_session_id();
    let resp = get_client(cfg.proxy_url.as_deref())
        .post(&url)
        .bearer_auth(cfg.api_key.trim())
        .header("x-opencode-session", &session_id)
        .header("x-session-id", &session_id)
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

    // 1. 尝试提取 OpenAI 标准 data[0].b64_json（需非空）
    if let Some(b64) = val.get("data")
        .and_then(|d| d.get(0))
        .and_then(|item| item.get("b64_json"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| format!("Base64 解码失败: {e}"))?;
        if !bytes.is_empty() {
            return Ok(bytes);
        }
    }

    // 2. 尝试提取 images[0] (部分 WebUI 或本地生图服务返回，需非空)
    if let Some(b64) = val.get("images")
        .and_then(|d| d.get(0))
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        let clean_b64 = if let Some(idx) = b64.find(',') {
            &b64[idx + 1..]
        } else {
            b64
        };
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(clean_b64)
            .map_err(|e| format!("Base64 解码失败: {e}"))?;
        if !bytes.is_empty() {
            return Ok(bytes);
        }
    }

    // 3. 尝试提取 data[0].url 并发起 HTTP GET 下载
    if let Some(img_url) = val.get("data")
        .and_then(|d| d.get(0))
        .and_then(|item| item.get("url"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        let img_resp = get_client(cfg.proxy_url.as_deref())
            .get(img_url)
            .send()
            .await
            .map_err(|e| format!("下载生成的图片失败: {e}"))?;
        if !img_resp.status().is_success() {
            return Err(format!("下载图片失败，HTTP 状态: {}", img_resp.status()));
        }
        let bytes = img_resp.bytes().await.map_err(|e| format!("读取图片数据失败: {e}"))?;
        if !bytes.is_empty() {
            return Ok(bytes.to_vec());
        }
    }

    Err(format!("未能从生图响应中解析到有效图片数据: {}", truncate(&val.to_string(), 500)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_proxy_url() {
        assert_eq!(normalize_proxy_url("127.0.0.1:7890"), "http://127.0.0.1:7890");
        assert_eq!(normalize_proxy_url("http://127.0.0.1:7890"), "http://127.0.0.1:7890");
        assert_eq!(normalize_proxy_url("https://proxy.example.com:8443"), "https://proxy.example.com:8443");
        assert_eq!(normalize_proxy_url("socks5://127.0.0.1:1080"), "socks5://127.0.0.1:1080");
        assert_eq!(normalize_proxy_url(""), "");
        assert_eq!(normalize_proxy_url("   "), "");
    }

    #[test]
    fn test_extract_model_ids_openai() {
        let data = json!({
            "object": "list",
            "data": [
                {"id": "gpt-4o", "object": "model"},
                {"id": "gpt-4o-mini", "object": "model"}
            ]
        });
        assert_eq!(extract_model_ids(&data), vec!["gpt-4o", "gpt-4o-mini"]);
    }

    #[test]
    fn test_extract_model_ids_ollama() {
        let data = json!({
            "models": [
                {"name": "llama3:latest", "model": "llama3:latest"},
                {"name": "qwen2.5:latest"}
            ]
        });
        assert_eq!(extract_model_ids(&data), vec!["llama3:latest", "qwen2.5:latest"]);
    }

    #[test]
    fn test_extract_model_ids_gemini() {
        let data = json!({
            "models": [
                {"name": "models/gemini-1.5-pro"},
                {"name": "models/gemini-1.5-flash"}
            ]
        });
        assert_eq!(extract_model_ids(&data), vec!["gemini-1.5-pro", "gemini-1.5-flash"]);
    }
}

