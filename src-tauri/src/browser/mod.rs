pub mod auth;
pub mod cdp;
pub mod detector;
pub mod dom_script;
pub mod session;

pub use auth::{AuthMacro, AuthMacroStatus, AuthStep, CookieData, StorageState};
pub use session::{BrowserDebugState, BrowserSession};

use crate::tools::ToolCtx;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tokio::sync::RwLock;

pub struct BrowserManager {
    sessions: RwLock<HashMap<String, Arc<BrowserSession>>>,
}

impl Default for BrowserManager {
    fn default() -> Self {
        Self::new()
    }
}

static GLOBAL_BROWSER_MANAGER: OnceLock<BrowserManager> = OnceLock::new();

pub fn global_browser_manager() -> &'static BrowserManager {
    GLOBAL_BROWSER_MANAGER.get_or_init(BrowserManager::new)
}

impl BrowserManager {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
        }
    }

    pub async fn get_or_create(
        &self,
        session_id: &str,
        headless: bool,
        workspace: Option<std::path::PathBuf>,
    ) -> Result<Arc<BrowserSession>, String> {
        {
            let map = self.sessions.read().await;
            if let Some(s) = map.get(session_id) {
                if s.is_alive().await {
                    if let Some(ref ws) = workspace {
                        s.set_workspace(ws.clone()).await;
                    }
                    return Ok(s.clone());
                }
            }
        }

        // 若旧会话已断开或失效，先安全清理
        {
            let mut map = self.sessions.write().await;
            if let Some(s) = map.remove(session_id) {
                s.close().await;
            }
        }

        let session = Arc::new(BrowserSession::launch(headless, workspace).await?);
        let mut map = self.sessions.write().await;
        map.insert(session_id.to_string(), session.clone());
        Ok(session)
    }

    pub async fn get(&self, session_id: &str) -> Option<Arc<BrowserSession>> {
        let map = self.sessions.read().await;
        if let Some(s) = map.get(session_id) {
            if s.is_alive().await {
                return Some(s.clone());
            }
        }
        // 容错回退：若未精确命中传入的 session_id（如草稿或父子会话场景），尝试回退到任意当前依然存活的调试实例
        for (_, s) in map.iter() {
            if s.is_alive().await {
                return Some(s.clone());
            }
        }
        None
    }

    pub async fn close(&self, session_id: &str) {
        let mut map = self.sessions.write().await;
        if let Some(session) = map.remove(session_id) {
            session.close().await;
        }
    }
}

// ---------------- Agent 工具接口实现 ----------------

fn get_session_key(ctx: &ToolCtx) -> String {
    ctx.host
        .as_ref()
        .map(|h| h.session_id.clone())
        .unwrap_or_else(|| "default_browser_session".to_string())
}

fn get_master_key(ctx: &ToolCtx) -> Option<[u8; 32]> {
    use tauri::Manager;
    let host = ctx.host.as_ref()?;
    let state = host.app.try_state::<crate::AppState>()?;
    let lock = state.master_key.lock().ok()?;
    Some(*lock)
}

pub async fn browser_navigate(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "缺少必填参数: url".to_string())?
        .trim();

    let headless = args
        .get("headless")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let session_key = get_session_key(ctx);
    let manager = global_browser_manager();
    let ws_path = if ctx.workspace.as_os_str().is_empty() {
        None
    } else {
        Some(ctx.workspace.clone())
    };
    let session = manager.get_or_create(&session_key, headless, ws_path).await?;

    let res = session.navigate(url).await;
    match res {
        Ok(debug_state) => {
            // 401 或路由退回登录态自愈机制
            if let Some(master) = get_master_key(ctx) {
                if let Ok(true) = session.check_and_heal_auth(&master).await {
                    if let Ok(healed_state) = session.navigate(url).await {
                        return Ok(healed_state.to_markdown());
                    }
                }
            }
            Ok(debug_state.to_markdown())
        }
        Err(e) => {
            // 若导航发生严重异常（如超时或通道断开），清理此异常会话，保障下次自动重试或新操作自动拉起健康实例
            manager.close(&session_key).await;
            Err(e)
        }
    }
}

pub async fn browser_interact(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "缺少必填参数: action (click, type, press_enter, wait_for_selector)".to_string())?;

    let target = args
        .get("target")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "缺少必填参数: target (如 @e1 或 CSS 选择器)".to_string())?;

    let value = args.get("value").and_then(|v| v.as_str());

    let session_key = get_session_key(ctx);
    let manager = global_browser_manager();
    let session = manager
        .get(&session_key)
        .await
        .ok_or_else(|| "未找到活动的浏览器调试会话，请先调用 browser_navigate 打开网页".to_string())?;

    let res = session.interact(action, target, value).await;
    match res {
        Ok(debug_state) => {
            // 检查交互后是否触发接口 401 或退回 login 页，如有则自动后台自愈
            if let Some(master) = get_master_key(ctx) {
                if let Ok(true) = session.check_and_heal_auth(&master).await {
                    if let Ok(healed_state) = session.get_debug_state().await {
                        return Ok(healed_state.to_markdown());
                    }
                }
            }
            Ok(debug_state.to_markdown())
        }
        Err(e) => {
            if e.contains("channel closed") || e.contains("CDP 连接已断开") {
                manager.close(&session_key).await;
            }
            Err(e)
        }
    }
}

pub async fn browser_get_debug_state(ctx: &ToolCtx) -> Result<String, String> {
    let session_key = get_session_key(ctx);
    let manager = global_browser_manager();
    let session = manager
        .get(&session_key)
        .await
        .ok_or_else(|| "未找到活动的浏览器调试会话，请先调用 browser_navigate 打开网页".to_string())?;

    let res = session.get_debug_state().await;
    match res {
        Ok(debug_state) => Ok(debug_state.to_markdown()),
        Err(e) => {
            if e.contains("channel closed") || e.contains("CDP 连接已断开") {
                manager.close(&session_key).await;
            }
            Err(e)
        }
    }
}

pub async fn browser_screenshot(ctx: &ToolCtx) -> Result<String, String> {
    let session_key = get_session_key(ctx);
    let manager = global_browser_manager();
    let session = manager
        .get(&session_key)
        .await
        .ok_or_else(|| "未找到活动的浏览器调试会话，请先调用 browser_navigate 打开网页".to_string())?;

    let b64 = match session.capture_screenshot().await {
        Ok(data) => data,
        Err(e) => {
            if e.contains("channel closed") || e.contains("CDP 连接已断开") {
                manager.close(&session_key).await;
            }
            return Err(e);
        }
    };

    let rel_path = ".harness/tmp/browser_screenshot.png";
    let target_file = if ctx.workspace.as_os_str().is_empty() {
        std::env::temp_dir().join("harness_browser_screenshot.png")
    } else {
        ctx.workspace.join(rel_path)
    };

    if let Some(parent) = target_file.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    use base64::Engine;
    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&b64) {
        let _ = tokio::fs::write(&target_file, bytes).await;
    }

    let approx_kb = (b64.len() * 3) / (4 * 1024);
    Ok(format!(
        "### 浏览器页面屏幕快照已成功捕获并保存\n\
         - **本地文件保存路径**: `{rel_path}`\n\
         - **绝对路径**: `{}`\n\
         - **图像体积**: ~{approx_kb} KB\n\n\
         > **注意**：请优先依据 `browser_navigate` 与 `browser_interact` 返回的可交互元素、表格与文本进行纯语义断言；仅在确需排查复杂 CSS 排版或渲染瑕疵时才调用 `recognize_image`。",
        target_file.to_string_lossy()
    ))
}

pub async fn browser_close(ctx: &ToolCtx) -> Result<String, String> {
    let session_key = get_session_key(ctx);
    global_browser_manager().close(&session_key).await;
    Ok("浏览器调试会话已成功关闭".to_string())
}

#[cfg(test)]
mod tests {
    use super::detector::detect_chromium_path;
    use super::dom_script::build_interact_script;

    #[test]
    fn test_detect_chromium() {
        let path = detect_chromium_path();
        assert!(path.is_ok(), "应当成功检测到本地安装的 Edge 或 Chrome: {:?}", path);
        let p = path.unwrap();
        assert!(p.exists());
    }

    #[test]
    fn test_build_interact_script() {
        let script = build_interact_script("type", "@e1", Some("zyt 123"));
        assert!(script.contains("data-harness-ref"));
        assert!(script.contains("@e1"));
        assert!(script.contains("zyt 123"));
        assert!(script.contains("dispatchEvent"));
    }
}

