pub mod batch;
pub mod credential;
pub mod detector;
pub mod diag;
pub mod dom_script;
pub mod evaluator;
pub mod fs_probe;
pub mod login_sop;
pub mod perceive;
pub mod session;
pub mod snapshot;
pub mod storage;
pub mod transport;

pub use perceive::model::{
    ElementDescriptor, ElementState, Lod, Obstruction, Rect,
};
pub use transport::{global_browser_daemon, BrowserDaemon, CdpClient};
pub use evaluator::{evaluate_page_state, PageHealthState};
pub use session::{BrowserDebugState, BrowserSession, ViewportInfo};
pub use storage::{
    clear_storage_state, ensure_gitignore_entries, get_storage_state_path,
    get_storage_state_summary, restore_storage_state, save_storage_state,
    storage_filename_for_url, StorageState, StorageStateSummary,
};

use crate::tools::ToolCtx;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tauri::Manager;
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
        // 严格按 session_id 命中。
        //
        // 历史缺陷：此处曾在未命中时「回退到任意存活实例」，导致 A 会话的 Agent 可能
        // 操作到 B 会话的浏览器（人多会话/父子会话场景下表现为「界面上看到的是 A，
        // 实际在动的是 B」）。多会话隔离优先于容错便利，故删除该回退。
        if let Some(s) = map.get(session_id) {
            if s.is_alive().await {
                return Some(s.clone());
            }
        }
        None
    }

    /// 当前是否有任意存活会话（供错误提示区分「没开过」与「标识不匹配」）
    pub async fn has_any_alive(&self) -> bool {
        let map = self.sessions.read().await;
        for s in map.values() {
            if s.is_alive().await {
                return true;
            }
        }
        false
    }

    pub async fn close(&self, session_id: &str) {
        let mut map = self.sessions.write().await;
        if let Some(session) = map.remove(session_id) {
            session.close().await;
        }
    }
}

// ---------------- Agent 工具接口实现 ----------------

/// 统一的会话 key 解析
///
/// 历史缺陷：Agent 工具侧用 `ctx.host.session_id`（缺省 "default_browser_session"），
/// 而前端 `browser_navigate_url` 命令用传入的 `session_id` 参数。两者不等时
/// `get_or_create` 会各自新建实例 → 人在看 A、Agent 在操作 B。
///
/// 约定：凡是能拿到 host 上下文的调用一律以它为准；前端命令若已按同一 session_id
/// 传参则自然命中同一实例。
pub fn resolve_session_key(host_session_id: Option<&str>, fallback: Option<&str>) -> String {
    if let Some(h) = host_session_id.filter(|s| !s.is_empty()) {
        return h.to_string();
    }
    if let Some(f) = fallback.filter(|s| !s.is_empty()) {
        return f.to_string();
    }
    "default_browser_session".to_string()
}

fn get_session_key(ctx: &ToolCtx) -> String {
    resolve_session_key(ctx.host.as_ref().map(|h| h.session_id.as_str()), None)
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
        Ok(debug_state) => Ok(debug_state.to_markdown()),
        Err(e) => {
            // 若导航发生严重异常（如超时或通道断开），清理此异常会话，保障下次自动重试或新操作自动拉起健康实例
            manager.close(&session_key).await;

            // 联动前端诊断总线：自动探测目标本地服务状态，辅助 Agent 精准归因
            let probe_info = match crate::diagnostics::probe_dev_server(url).await {
                crate::diagnostics::DevServerStatus::CompilationFailed { stage, error_snippet } => {
                    format!("\n\n> 🛠️ **【本地开发构建诊断】**: 检测到本地 {stage} 服务处于编译报错状态！\n> 错误简述: `{error_snippet}`\n> 请重点检查最新改动的代码文件是否存在语法/导入错误。")
                }
                crate::diagnostics::DevServerStatus::NotRunning => {
                    format!("\n\n> ⚠️ **【本地服务诊断】**: 目标端口当前无法连接，请确认本地开发服务 (如 npm run serve / vite) 是否正在运行。")
                }
                _ => String::new(),
            };

            Err(format!("{e}{probe_info}"))
        }
    }
}

pub async fn browser_batch(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let session_key = get_session_key(ctx);
    let manager = global_browser_manager();
    let session = manager
        .get(&session_key)
        .await
        .ok_or_else(|| "未找到活动的浏览器调试会话，请先调用 browser_navigate 打开网页".to_string())?;

    // 操作互斥：防并发批次与人工接管交错在同一 CDP target 上
    let _guard = session.acquire_op_lock().await?;

    // 登录硬门禁：未登录时直接返回结构化 AUTH_REQUIRED + SOP 路径，
    // 避免 Agent 忽略软提示继续点业务按钮（原设计只在 markdown 里放一行 ⚠️）。
    // 例外：批次自身已包含登录动作时不拦截，避免自锁。
    if !batch::batch_looks_like_login(args) {
        if let Ok(st) = session.get_debug_state().await {
            if let crate::browser::evaluator::PageHealthState::Unauthenticated { current_url, reason } = &st.page_state {
                let ws = if ctx.workspace.as_os_str().is_empty() {
                    std::path::PathBuf::new()
                } else {
                    ctx.workspace.clone()
                };
                return Err(login_sop::auth_required_message(&ws, current_url, reason));
            }
        }
    }

    // 凭据查询回调：从工作区加密凭据库按 key 解密明文（仅在内存中使用）
    let ws = if ctx.workspace.as_os_str().is_empty() {
        None
    } else {
        Some(ctx.workspace.clone())
    };
    let master_key = if let Some(host) = &ctx.host {
        let state = host.app.state::<crate::AppState>();
        let guard = state.master_key.lock();
        Some(*guard)
    } else {
        None
    };
    let lookup = move |key: &str| -> Option<String> {
        let ws = ws.as_ref()?;
        let mk = master_key.as_ref()?;
        crate::browser::credential::load_secret(ws, mk, key).ok().flatten()
    };

    let outcome = match batch::execute_batch(&session, args, &lookup).await {
        Ok(o) => o,
        Err(e) => {
            if e.contains("channel closed") || e.contains("CDP 连接已断开") {
                manager.close(&session_key).await;
            }
            return Err(e);
        }
    };

    // 通道级异常时摘除故障会话，便于下次自动重建
    if !outcome.ok && outcome.code.as_deref() == Some("CHANNEL_CLOSED") {
        manager.close(&session_key).await;
    }

    Ok(batch::render_outcome(&outcome))
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

pub async fn browser_screenshot(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let full_page = args
        .get("full_page")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let session_key = get_session_key(ctx);
    let manager = global_browser_manager();
    let session = manager
        .get(&session_key)
        .await
        .ok_or_else(|| "未找到活动的浏览器调试会话，请先调用 browser_navigate 打开网页".to_string())?;

    let b64 = match session.capture_screenshot(full_page).await {
        Ok(data) => data,
        Err(e) => {
            if e.contains("channel closed") || e.contains("CDP 连接已断开") {
                manager.close(&session_key).await;
            }
            return Err(e);
        }
    };

    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&b64)
        .map_err(|e| format!("截图 Base64 解码失败: {e}"))?;

    // 1. 获取宿主环境的数据目录与会话 ID
    let (data_dir, session_id) = if let Some(host) = &ctx.host {
        let state = host.app.state::<crate::AppState>();
        let dd = state
            .data_dir
            .lock()
            .clone()
            .unwrap_or_else(|| state.default_data_dir.clone());
        (dd, host.session_id.clone())
    } else {
        // 无宿主上下文时（如单元测试）用可写探测，避免 TEMP 不可写导致截图落盘失败
        (
            fs_probe::resolve_writable_dir(None, "harness_mini_data"),
            session_key.clone(),
        )
    };

    // 2. 委托资产中台：计算 CAS 内容寻址哈希去重并应用滚动配额管理
    let artifact = crate::artifacts::ArtifactManager::store_screenshot(
        &data_dir,
        &session_id,
        &bytes,
        Some("browser"),
    )
    .await?;

    let original_str = artifact.original_path.to_string_lossy().replace('\\', "/");
    let thumb_str = artifact
        .thumbnail_path
        .as_ref()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| original_str.clone());

    // 3. 同时更新一份快捷文件到工作区，保持向下兼容与调试便捷性（扩展名与真实格式一致）
    let legacy_ext = crate::models::detect_image_mime_and_ext(&bytes)
        .map(|(_, ext)| ext)
        .unwrap_or("png");
    let legacy_rel_path = format!(".harness/tmp/browser_screenshot.{legacy_ext}");
    let legacy_target = if ctx.workspace.as_os_str().is_empty() {
        fs_probe::resolve_writable_dir(None, "harness_screenshot")
            .join(format!("browser_screenshot.{legacy_ext}"))
    } else {
        ctx.workspace.join(&legacy_rel_path)
    };
    if let Some(parent) = legacy_target.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let _ = tokio::fs::write(&legacy_target, &bytes).await;

    let approx_kb = bytes.len() / 1024;
    Ok(format!(
        "### 浏览器页面屏幕快照已成功捕获\n\
         - **资产快照路径**: `{original_str}`\n\
         - **缩略图路径**: `{thumb_str}`\n\
         - **内容指纹 (CAS)**: `{}`\n\
         - **图像体积**: ~{approx_kb} KB\n\n\
         > **注意**：请优先依据 `browser_navigate` 与 `browser_batch` 返回的可交互元素、表格与文本进行纯语义断言；确需排查复杂 CSS 排版或渲染瑕疵时，请直接调用 `recognize_image(path=\"{original_str}\")`。",
        artifact.hash
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

    #[test]
    fn test_build_scroll_script() {
        use super::dom_script::build_scroll_script;
        let s_down = build_scroll_script("down", Some(500.0), None);
        assert!(s_down.contains("scrollBy"));
        assert!(s_down.contains("500"));

        let s_target = build_scroll_script("down", None, Some("@e12"));
        assert!(s_target.contains("data-harness-ref"));
        assert!(s_target.contains("@e12"));
        assert!(s_target.contains("scrollIntoView"));

        let s_top = build_scroll_script("top", None, None);
        assert!(s_top.contains("scrollTo"));
    }

    #[tokio::test]
    async fn test_browser_daemon_and_context_lifecycle() {
        let daemon = super::transport::global_browser_daemon();
        let cdp = daemon.ensure_running().await;
        assert!(cdp.is_ok(), "Daemon 应能正常拉起 Chromium 并连接 CDP: {:?}", cdp.err());

        // 1. 测试创建隔离上下文
        let ctx = daemon.create_context().await;
        assert!(ctx.is_ok(), "应当成功创建独立的 BrowserContext: {:?}", ctx.err());
        let context_id = ctx.unwrap();

        // 2. 在该上下文中创建页面 Target
        let target = daemon.create_target_in_context(Some(&context_id), "about:blank").await;
        assert!(target.is_ok(), "应当成功在 Context 下创建 Target: {:?}", target.err());
        let target_id = target.unwrap();

        // 3. 挂载会话
        let session = daemon.attach_to_target(&target_id).await;
        assert!(session.is_ok(), "应当成功挂载至 Target 获得 sessionId: {:?}", session.err());

        // 4. 清理 Target 与 Context
        let _ = daemon.close_target(&target_id).await;
        let _ = daemon.dispose_context(&context_id).await;
        daemon.stop().await;
    }
}
