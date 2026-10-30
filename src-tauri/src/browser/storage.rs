use super::cdp::CdpClient;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StorageState {
    #[serde(default)]
    pub landing_url: Option<String>,
    pub cookies: Vec<Value>,
    #[serde(default)]
    pub local_storage: HashMap<String, String>,
    #[serde(default)]
    pub session_storage: HashMap<String, String>,
    pub updated_at: String,
}

/// 智能解析工作区路径：优先使用有效工作区，若为空则自动回退至当前进程工作目录或全局数据目录，确保凭证永不丢失
pub fn resolve_storage_workspace(workspace: &Path) -> PathBuf {
    if !workspace.as_os_str().is_empty() && workspace.exists() {
        return workspace.to_path_buf();
    }
    if let Ok(cur) = std::env::current_dir() {
        if cur.exists() {
            return cur;
        }
    }
    std::env::temp_dir().join("harness_storage")
}

pub fn storage_filename_for_url(url: &str) -> String {
    if let Ok(parsed) = Url::parse(url) {
        if let Some(host) = parsed.host_str() {
            let port = parsed.port_or_known_default().unwrap_or(80);
            let safe_host = host.replace(|c: char| !c.is_alphanumeric() && c != '.', "_");
            return format!("storage_{safe_host}_{port}.json");
        }
    }
    "storage_state.json".to_string()
}

pub fn get_storage_state_path(workspace: &Path, url: Option<&str>) -> PathBuf {
    let ws = resolve_storage_workspace(workspace);
    let dir = ws.join(".harness").join("storage_states");
    if let Some(u) = url {
        let name = storage_filename_for_url(u);
        dir.join(&name)
    } else {
        dir.join("storage_state.json")
    }
}

pub const EXTRACT_WEB_STORAGE_SCRIPT: &str = r#"
(() => {
    try {
        const ls = {};
        for (let i = 0; i < localStorage.length; i++) {
            const k = localStorage.key(i);
            if (k !== null && k !== undefined) {
                const v = localStorage.getItem(k);
                ls[String(k)] = v !== null && v !== undefined ? String(v) : "";
            }
        }
        const ss = {};
        for (let i = 0; i < sessionStorage.length; i++) {
            const k = sessionStorage.key(i);
            if (k !== null && k !== undefined) {
                const v = sessionStorage.getItem(k);
                ss[String(k)] = v !== null && v !== undefined ? String(v) : "";
            }
        }
        return { local_storage: ls, session_storage: ss, url: window.location.href };
    } catch (e) {
        return { local_storage: {}, session_storage: {}, error: String(e) };
    }
})()
"#;

pub fn build_inject_web_storage_script(
    local_storage: &HashMap<String, String>,
    session_storage: &HashMap<String, String>,
) -> String {
    let ls_json = serde_json::to_string(local_storage).unwrap_or_else(|_| "{}".to_string());
    let ss_json = serde_json::to_string(session_storage).unwrap_or_else(|_| "{}".to_string());

    format!(
        r#"
(() => {{
    try {{
        const ls = {ls_json};
        for (const [k, v] of Object.entries(ls)) {{
            try {{ localStorage.setItem(k, v); }} catch (_) {{}}
        }}
        const ss = {ss_json};
        for (const [k, v] of Object.entries(ss)) {{
            try {{ sessionStorage.setItem(k, v); }} catch (_) {{}}
        }}
    }} catch (_) {{}}

    // 自愈保护：针对企业级 SPA 全屏遮罩 (#flashload / .yh-loading-mask / .el-loading-mask) 及路由死锁
    try {{
        const dismissMaskAndGuard = () => {{
            const fl = document.getElementById('flashload');
            if (fl) {{
                fl.style.display = 'none';
                try {{ fl.remove(); }} catch (_) {{}}
            }}
            const masks = document.querySelectorAll('.yh-loading-mask.is-fullscreen, .el-loading-mask.is-fullscreen');
            masks.forEach(m => {{
                m.style.display = 'none';
                try {{ m.remove(); }} catch (_) {{}}
            }});

            // 路由自愈防白屏：若首屏路由守卫回退至无效根路径导致视图完全空白，自动重定向到登录页让用户能看到表单
            try {{
                const hash = window.location.hash;
                const pathname = window.location.pathname;
                const isBlankRoute = !hash || hash === '#' || hash === '#/' || hash === '#/null';
                const app = document.getElementById('app');
                const isAppEmpty = !app || !app.innerText || app.innerText.trim().length === 0;
                if (isBlankRoute && isAppEmpty && (pathname.includes('/YWPT') || pathname.includes('/login'))) {{
                    window.location.hash = '#/login';
                }}
            }} catch (_) {{}}
        }};
        setTimeout(dismissMaskAndGuard, 1200);
        setTimeout(dismissMaskAndGuard, 2400);
        if (document.readyState === 'complete') {{
            setTimeout(dismissMaskAndGuard, 400);
        }} else {{
            window.addEventListener('load', () => setTimeout(dismissMaskAndGuard, 400), {{ once: true }});
        }}
    }} catch (_) {{}}
}})()
"#
    )
}

/// 保存会话的 Cookie、LocalStorage 与 SessionStorage 到工作区轻量 JSON，并返回保存好的快照实体
pub async fn save_storage_state(
    cdp: &Arc<CdpClient>,
    target_session_id: &str,
    workspace: &Path,
    current_url: Option<&str>,
) -> Result<StorageState, String> {
    let ws = resolve_storage_workspace(workspace);
    let sid = Some(target_session_id);

    // 1. 获取全量 Cookies (通道 A: Storage.getCookies + 通道 B: Network.getCookies)
    let mut cookies = Vec::new();
    let mut cookie_keys = std::collections::HashSet::new();

    // 通道 A: Storage.getCookies (当前 BrowserContext 下所有站点 Cookies)
    if let Ok(res) = cdp.send("Storage.getCookies", None, sid, Duration::from_secs(5)).await {
        if let Some(arr) = res.get("cookies").and_then(|c| c.as_array()) {
            for c in arr {
                let name = c.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let domain = c.get("domain").and_then(|d| d.as_str()).unwrap_or("");
                let path = c.get("path").and_then(|p| p.as_str()).unwrap_or("");
                let key = format!("{name}@{domain}{path}");
                if !name.is_empty() && cookie_keys.insert(key) {
                    cookies.push(c.clone());
                }
            }
        }
    }

    // 通道 B: Network.getCookies 带当前 URL 参数进行精确补充
    let net_params = current_url.map(|u| json!({ "urls": [u] }));
    if let Ok(res) = cdp.send("Network.getCookies", net_params, sid, Duration::from_secs(5)).await {
        if let Some(arr) = res.get("cookies").and_then(|c| c.as_array()) {
            for c in arr {
                let name = c.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let domain = c.get("domain").and_then(|d| d.as_str()).unwrap_or("");
                let path = c.get("path").and_then(|p| p.as_str()).unwrap_or("");
                let key = format!("{name}@{domain}{path}");
                if !name.is_empty() && cookie_keys.insert(key) {
                    cookies.push(c.clone());
                }
            }
        }
    }

    // 2. 提取 LocalStorage 与 SessionStorage (逐项深层解析，容错所有类型)
    let eval_res = cdp
        .send(
            "Runtime.evaluate",
            Some(json!({
                "expression": EXTRACT_WEB_STORAGE_SCRIPT,
                "returnByValue": true
            })),
            sid,
            Duration::from_secs(5),
        )
        .await
        .ok();

    let mut detected_url = None;
    let (local_storage, session_storage) = if let Some(val) = eval_res
        .and_then(|r| r.get("result").cloned())
        .and_then(|res| res.get("value").cloned())
    {
        if let Some(u) = val.get("url").and_then(|u| u.as_str()) {
            if !u.is_empty() && !u.starts_with("about:") {
                detected_url = Some(u.to_string());
            }
        }

        let mut ls = HashMap::new();
        if let Some(obj) = val.get("local_storage").and_then(|l| l.as_object()) {
            for (k, v) in obj {
                let v_str = match v {
                    Value::String(s) => s.clone(),
                    Value::Null => String::new(),
                    other => other.to_string(),
                };
                ls.insert(k.clone(), v_str);
            }
        }

        let mut ss = HashMap::new();
        if let Some(obj) = val.get("session_storage").and_then(|s| s.as_object()) {
            for (k, v) in obj {
                let v_str = match v {
                    Value::String(s) => s.clone(),
                    Value::Null => String::new(),
                    other => other.to_string(),
                };
                ss.insert(k.clone(), v_str);
            }
        }
        (ls, ss)
    } else {
        (HashMap::new(), HashMap::new())
    };

    // 优先使用页面运行时真实探测到的 URL，若无则使用外部传入的 URL
    let effective_url = detected_url.as_deref().or(current_url);

    let state = StorageState {
        landing_url: effective_url
            .filter(|u| !u.starts_with("about:") && !u.is_empty())
            .map(|u| u.to_string()),
        cookies,
        local_storage,
        session_storage,
        updated_at: chrono::Utc::now().to_rfc3339(),
    };

    let target_path = get_storage_state_path(&ws, effective_url);
    if let Some(parent) = target_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    let json_bytes = serde_json::to_vec_pretty(&state)
        .map_err(|e| format!("序列化 storage state 失败: {e}"))?;

    tokio::fs::write(&target_path, &json_bytes)
        .await
        .map_err(|e| format!("写入 storage state 失败: {e}"))?;

    let _ = ensure_gitignore_entries(&ws);
    Ok(state)
}

/// 从工作区恢复 Cookie 与 Storage 到当前隔离的 BrowserContext，并可选触发页面重载以即时激活鉴权
pub async fn restore_storage_state(
    cdp: &Arc<CdpClient>,
    target_session_id: &str,
    workspace: &Path,
    target_url: Option<&str>,
) -> Result<bool, String> {
    let ws = resolve_storage_workspace(workspace);
    let target_path = get_storage_state_path(&ws, target_url);
    if !target_path.exists() {
        return Ok(false);
    }

    let content = match tokio::fs::read_to_string(&target_path).await {
        Ok(c) => c,
        Err(_) => return Ok(false),
    };

    let state: StorageState = match serde_json::from_str(&content) {
        Ok(s) => s,
        Err(_) => return Ok(false),
    };

    let sid = Some(target_session_id);

    // 1. 恢复 Cookies
    if !state.cookies.is_empty() {
        let _ = cdp
            .send(
                "Network.setCookies",
                Some(json!({ "cookies": state.cookies })),
                sid,
                Duration::from_secs(5),
            )
            .await;
    }

    // 2. 注入 LocalStorage 与 SessionStorage 恢复脚本 (在首行业务 JS 执行前恢复)
    if !state.local_storage.is_empty() || !state.session_storage.is_empty() {
        let inject_script = build_inject_web_storage_script(&state.local_storage, &state.session_storage);

        // 注册到后续新文档
        let _ = cdp
            .send(
                "Page.addScriptToEvaluateOnNewDocument",
                Some(json!({ "source": inject_script })),
                sid,
                Duration::from_secs(5),
            )
            .await;

        // 若当前页面已经存在，也立即就地执行一次注入
        let _ = cdp
            .send(
                "Runtime.evaluate",
                Some(json!({
                    "expression": inject_script,
                    "returnByValue": false
                })),
                sid,
                Duration::from_secs(5),
            )
            .await;
    }

    // 3. 智能重载/落地页导航：
    // 若当前停留在登录页（URL 包含 /login、#/login 或 signin），且快照中包含合法的工作台落地页（landing_url），
    // 优先通过 Page.navigate 导航至真实工作台落地页，彻底消除 SPA 路由守卫在首屏重载时的 401 死锁与回环
    let current_href: Option<String> = cdp
        .send(
            "Runtime.evaluate",
            Some(json!({
                "expression": "window.location.href",
                "returnByValue": true
            })),
            sid,
            Duration::from_secs(3),
        )
        .await
        .ok()
        .and_then(|r| r.get("result").and_then(|res| res.get("value")).and_then(|v| v.as_str()).map(ToString::to_string))
        .or_else(|| target_url.map(ToString::to_string));

    let current_is_login = current_href.as_deref().map_or(false, |u| {
        let lower = u.to_lowercase();
        lower.contains("/login") || lower.contains("#/login") || lower.contains("signin")
    });

    let target_landing = state.landing_url.as_deref().filter(|l| {
        let lower = l.to_lowercase();
        !lower.contains("/login") && !lower.contains("#/login") && !lower.contains("signin") && !lower.starts_with("about:")
    });

    if current_is_login && target_landing.is_some() {
        let dest = target_landing.unwrap();
        let _ = cdp
            .send(
                "Page.navigate",
                Some(json!({ "url": dest })),
                sid,
                Duration::from_secs(10),
            )
            .await;
    } else {
        let _ = cdp
            .send(
                "Page.reload",
                Some(json!({ "ignoreCache": false })),
                sid,
                Duration::from_secs(10),
            )
            .await;
    }

    // 给予 600ms 供单页应用完成重定向和工作台加载
    tokio::time::sleep(Duration::from_millis(600)).await;

    Ok(true)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StorageStateSummary {
    pub exists: bool,
    pub file_path: String,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub landing_url: Option<String>,
    pub cookie_count: usize,
    pub local_storage_count: usize,
    pub session_storage_count: usize,
    pub local_storage_keys: Vec<String>,
    pub session_storage_keys: Vec<String>,
    pub updated_at: Option<String>,
}

pub fn get_storage_state_summary(workspace: &Path, url: Option<&str>) -> StorageStateSummary {
    let path = get_storage_state_path(workspace, url);
    let mut host = None;
    let mut port = None;
    if let Some(u) = url {
        if let Ok(parsed) = Url::parse(u) {
            host = parsed.host_str().map(|s| s.to_string());
            port = parsed.port_or_known_default();
        }
    }
    if !path.exists() {
        return StorageStateSummary {
            exists: false,
            file_path: path.to_string_lossy().to_string(),
            host,
            port,
            landing_url: None,
            cookie_count: 0,
            local_storage_count: 0,
            session_storage_count: 0,
            local_storage_keys: vec![],
            session_storage_keys: vec![],
            updated_at: None,
        };
    }

    if let Ok(content) = std::fs::read_to_string(&path) {
        if let Ok(state) = serde_json::from_str::<StorageState>(&content) {
            let ls_keys = state.local_storage.keys().cloned().collect();
            let ss_keys = state.session_storage.keys().cloned().collect();
            return StorageStateSummary {
                exists: true,
                file_path: path.to_string_lossy().to_string(),
                host,
                port,
                landing_url: state.landing_url,
                cookie_count: state.cookies.len(),
                local_storage_count: state.local_storage.len(),
                session_storage_count: state.session_storage.len(),
                local_storage_keys: ls_keys,
                session_storage_keys: ss_keys,
                updated_at: Some(state.updated_at),
            };
        }
    }

    StorageStateSummary {
        exists: false,
        file_path: path.to_string_lossy().to_string(),
        host,
        port,
        landing_url: None,
        cookie_count: 0,
        local_storage_count: 0,
        session_storage_count: 0,
        local_storage_keys: vec![],
        session_storage_keys: vec![],
        updated_at: None,
    }
}

pub fn clear_storage_state(workspace: &Path, url: Option<&str>) -> Result<bool, String> {
    let ws = resolve_storage_workspace(workspace);
    let mut removed = false;

    // 1. 删除特定站点的快照文件
    if let Some(u) = url {
        let name = storage_filename_for_url(u);
        let path = ws.join(".harness").join("storage_states").join(&name);
        if path.exists() {
            let _ = std::fs::remove_file(&path);
            removed = true;
        }
    }

    // 2. 清除工作区根目录中遗留的 storage_state.json，彻底杜绝幽灵复活
    let legacy = ws.join(".harness").join("storage_state.json");
    if legacy.exists() {
        let _ = std::fs::remove_file(&legacy);
        removed = true;
    }

    // 3. 若 url 为 None，清空 storage_states 目录中所有快照
    if url.is_none() {
        let states_dir = ws.join(".harness").join("storage_states");
        if states_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&states_dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_file() {
                        let _ = std::fs::remove_file(p);
                        removed = true;
                    }
                }
            }
        }
    }

    Ok(removed)
}

/// 确保工作区 .gitignore 包含登录态持久化快照目录，防止 Cookie 或 Token 被误提交至仓库
pub fn ensure_gitignore_entries(workspace: &Path) -> Result<(), String> {
    let gitignore_path = workspace.join(".gitignore");
    let mut to_append = Vec::new();

    let content = if gitignore_path.exists() {
        std::fs::read_to_string(&gitignore_path).unwrap_or_default()
    } else {
        String::new()
    };

    if !content.contains("storage_states") && !content.contains(".harness/") {
        to_append.push(".harness/storage_states/");
    }
    if !content.contains("storage_state.json") && !content.contains(".harness/") {
        to_append.push(".harness/storage_state.json");
    }

    if to_append.is_empty() {
        return Ok(());
    }

    let mut new_content = content;
    if !new_content.is_empty() && !new_content.ends_with('\n') {
        new_content.push('\n');
    }
    new_content.push_str("\n# Harness Browser Storage State Snapshots\n");
    for item in to_append {
        new_content.push_str(item);
        new_content.push('\n');
    }

    std::fs::write(&gitignore_path, new_content)
        .map_err(|e| format!("更新 .gitignore 失败: {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_storage_filename_and_path_isolation() {
        let name_18086 = storage_filename_for_url("http://127.0.0.1:18086/#/login");
        assert_eq!(name_18086, "storage_127.0.0.1_18086.json");

        let name_5611 = storage_filename_for_url("http://localhost:5611/dashboard");
        assert_eq!(name_5611, "storage_localhost_5611.json");

        assert_ne!(name_18086, name_5611, "不同 Origin 的 Storage 文件名必须严格隔离");

        let temp = std::env::temp_dir().join(format!("test_harness_ws_{}", std::process::id()));
        let p_18086 = get_storage_state_path(&temp, Some("http://127.0.0.1:18086/"));
        assert!(p_18086.to_string_lossy().contains("storage_127.0.0.1_18086.json"));
    }

    #[test]
    fn test_storage_summary_and_clear_lifecycle() {
        let temp = std::env::temp_dir().join(format!("test_harness_summary_{}", std::process::id()));
        let _ = std::fs::create_dir_all(temp.join(".harness").join("storage_states"));

        let test_url = "http://127.0.0.1:19999/test";
        let summary_init = get_storage_state_summary(&temp, Some(test_url));
        assert!(!summary_init.exists);
        assert_eq!(summary_init.host, Some("127.0.0.1".to_string()));
        assert_eq!(summary_init.port, Some(19999));

        // 写入一份模拟快照
        let mut ls = HashMap::new();
        ls.insert("access_token".to_string(), "jwt.xxx.yyy".to_string());
        let mut ss = HashMap::new();
        ss.insert("dept_id".to_string(), "dept_001".to_string());
        let mock_state = StorageState {
            landing_url: Some(test_url.to_string()),
            cookies: vec![json!({"name": "session_id", "value": "s123"})],
            local_storage: ls,
            session_storage: ss,
            updated_at: "2026-10-30T10:00:00Z".to_string(),
        };

        let target_path = get_storage_state_path(&temp, Some(test_url));
        std::fs::write(&target_path, serde_json::to_string_pretty(&mock_state).unwrap()).unwrap();

        // 验证 summary 读取
        let summary = get_storage_state_summary(&temp, Some(test_url));
        assert!(summary.exists);
        assert_eq!(summary.landing_url, Some(test_url.to_string()));
        assert_eq!(summary.cookie_count, 1);
        assert_eq!(summary.local_storage_count, 1);
        assert_eq!(summary.session_storage_count, 1);
        assert_eq!(summary.local_storage_keys, vec!["access_token".to_string()]);
        assert_eq!(summary.session_storage_keys, vec!["dept_id".to_string()]);

        // 验证清除：单次清除即刻彻底消失，绝不需要点击两次
        let cleared = clear_storage_state(&temp, Some(test_url)).unwrap();
        assert!(cleared);
        assert!(!target_path.exists());
        let summary_after_clear = get_storage_state_summary(&temp, Some(test_url));
        assert!(!summary_after_clear.exists, "单次清除后必须立即返回 false，绝不回退至旧文件");

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_build_inject_web_storage_script() {
        let mut ls = HashMap::new();
        ls.insert("token".to_string(), "abc".to_string());
        let ss = HashMap::new();
        let script = build_inject_web_storage_script(&ls, &ss);
        assert!(script.contains("localStorage.setItem"));
        assert!(script.contains("\"token\":\"abc\""));
    }
}
