use super::detector::detect_chromium_path;
use super::dom_script::{build_interact_script, build_scroll_script, EXTRACT_TREE_SCRIPT};
use super::transport::CdpClient;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Child;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractiveElement {
    pub ref_id: String,
    pub tag: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    /// 是否位于「可见弹窗」容器内
    ///
    /// 实测痛点（2026-11-24）：弹窗内与外层页面常有同 placeholder 的输入框，
    /// 指纹相同只能靠 `_2`/`_3` 后缀区分，Agent 无法判断该填哪一个。
    #[serde(default, skip_serializing_if = "is_false")]
    pub in_dialog: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// 渲染单个可交互元素为一行 markdown
fn fmt_element_line(el: &InteractiveElement) -> String {
    let mut desc = format!("- `{}` `<{}`", el.ref_id, el.tag);
    if let Some(ref t) = el.r#type {
        desc.push_str(&format!(" type=\"{t}\""));
    }
    if let Some(ref n) = el.name {
        desc.push_str(&format!(" name=\"{n}\""));
    }
    if let Some(ref p) = el.placeholder {
        desc.push_str(&format!(" placeholder=\"{p}\""));
    }
    if let Some(ref v) = el.value {
        if !v.is_empty() {
            desc.push_str(&format!(" value=\"{v}\""));
        }
    }
    desc.push('>');
    if let Some(ref txt) = el.text {
        if !txt.is_empty() {
            desc.push_str(&format!(" \"{txt}\""));
        }
    }
    desc
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ViewportInfo {
    pub scroll_x: f64,
    pub scroll_y: f64,
    pub viewport_width: f64,
    pub viewport_height: f64,
    pub total_width: f64,
    pub total_height: f64,
    pub current_page: u32,
    pub total_pages: u32,
    pub has_more_down: bool,
    pub has_more_up: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserDebugState {
    pub url: String,
    pub title: String,
    #[serde(default)]
    pub viewport: ViewportInfo,
    pub elements: Vec<InteractiveElement>,
    #[serde(default)]
    pub hidden_elements_count: usize,
    pub console_errors: Vec<String>,
    pub network_errors: Vec<String>,
    #[serde(default)]
    pub tables: Vec<String>,
    #[serde(default)]
    pub notices: Vec<String>,
    #[serde(default)]
    pub dom_overlays: Vec<String>,
    #[serde(default)]
    pub page_state: super::evaluator::PageHealthState,
}

impl BrowserDebugState {
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();
        md.push_str(&format!("### 当前网页状态\n- **URL**: {}\n- **标题**: {}\n", self.url, self.title));

        let has_critical_error = !self.dom_overlays.is_empty()
            || self.console_errors.iter().any(|e| e.contains("[Uncaught Exception]") || e.contains("[Console.error]"));
        if has_critical_error {
            md.push_str("> 🚨 **【前端运行时异常感知告警】**: 检测到页面渲染异常或未捕获错误，请优先排查下方错误详情！\n\n");
        }

        let vp = &self.viewport;
        if vp.total_height > 0.0 {
            let start_y = vp.scroll_y.round() as u64;
            let end_y = (vp.scroll_y + vp.viewport_height).min(vp.total_height).round() as u64;
            let total_y = vp.total_height.round() as u64;
            md.push_str(&format!(
                "- **视口范围**: Y: {start_y}px ~ {end_y}px / 总高: {total_y}px (第 {}/{} 屏)\n",
                vp.current_page, vp.total_pages
            ));
            if vp.has_more_down {
                md.push_str(&format!(
                    "> 💡 **长页面提示**: 当前视口下方仍有未展示内容（视口外折叠约 {} 个交互元素）。如需继续向下探索，可调用 `browser_batch` 并传入 `{{\"op\":\"scroll\",\"value\":\"900\"}}`。\n",
                    self.hidden_elements_count
                ));
            } else if vp.current_page > 1 {
                md.push_str("> ℹ️ **视口状态**: 当前已滑至页面最底部。\n");
            }
        }
        md.push('\n');

        match &self.page_state {
            super::evaluator::PageHealthState::Unauthenticated { current_url, reason } => {
                md.push_str(&format!("> ⚠️ **鉴权拦截提示**: 页面处于未登录或鉴权已失效状态 (原因: `{reason}`, 当前URL: `{current_url}`)\n\n"));
            }
            super::evaluator::PageHealthState::ActionBlocked { title, description } => {
                md.push_str(&format!("> ⏸️ **全局阻断提示**: 检测到全局前置弹窗【{title}】({description})\n\n"));
            }
            super::evaluator::PageHealthState::Error { code, message } => {
                md.push_str(&format!("> ❌ **页面异常**: HTTP {:?} - {}\n\n", code, message));
            }
            _ => {}
        }

        if !self.dom_overlays.is_empty() {
            md.push_str("#### 💥 页面报错遮罩/构建异常 (Error Overlays):\n```text\n");
            for overlay in &self.dom_overlays {
                md.push_str(overlay);
                md.push('\n');
            }
            md.push_str("```\n\n");
        }

        if !self.notices.is_empty() {
            md.push_str("#### 页面提示与通知 (Alerts & Notices):\n");
            for notice in &self.notices {
                md.push_str(&format!("- ℹ️ {notice}\n"));
            }
            md.push('\n');
        }

        if !self.tables.is_empty() {
            md.push_str("#### 页面数据表格概览 (Data Tables):\n");
            for (idx, tbl) in self.tables.iter().enumerate() {
                if self.tables.len() > 1 {
                    md.push_str(&format!("**表格 {}**:\n", idx + 1));
                }
                md.push_str(tbl);
                md.push_str("\n\n");
            }
        }

        // 元素清单按「弹窗内 / 页面」分组展示
        //
        // 实测痛点（2026-11-24）：弹窗里与外层页面同 placeholder 的 input 指纹相同，
        // 列表只靠 `_2`/`_3` 后缀区分，Agent 无法判断该填哪一个，只能截图。
        // 分组后归属一目了然。
        let in_dialog_els: Vec<&InteractiveElement> = self.elements.iter().filter(|e| e.in_dialog).collect();
        let page_els: Vec<&InteractiveElement> = self.elements.iter().filter(|e| !e.in_dialog).collect();

        if self.elements.is_empty() {
            md.push_str("#### 可交互元素清单 (可通过 @eX 进行操作):\n");
            md.push_str("(当前视口内未检测到可交互元素)\n\n");
        } else if in_dialog_els.is_empty() {
            md.push_str("#### 可交互元素清单 (可通过 @eX 进行操作):\n");
            for el in &page_els {
                md.push_str(&fmt_element_line(el));
                md.push('\n');
            }
            md.push('\n');
        } else {
            md.push_str(&format!(
                "#### 可交互元素清单 (可通过 @eX 进行操作) — 共 {} 个，其中弹窗内 {} 个:\n",
                self.elements.len(),
                in_dialog_els.len()
            ));
            md.push_str("\n**弹窗内元素**（当前弹窗提示/表单优先操作这些）:\n");
            for el in &in_dialog_els {
                md.push_str(&fmt_element_line(el));
                md.push('\n');
            }
            md.push_str("\n**页面元素**（弹窗之外）:\n");
            if page_els.is_empty() {
                md.push_str("(无)\n");
            } else {
                for el in &page_els {
                    md.push_str(&fmt_element_line(el));
                    md.push('\n');
                }
            }
            md.push('\n');
        }

        if !self.console_errors.is_empty() {
            md.push_str("#### 控制台错误日志 (Console Errors):\n```\n");
            for err in self.console_errors.iter().rev().take(5) {
                md.push_str(err);
                md.push('\n');
            }
            md.push_str("```\n\n");
        }

        if !self.network_errors.is_empty() {
            md.push_str("#### 网络请求异常 (Failed Network Requests):\n```\n");
            for net in self.network_errors.iter().rev().take(8) {
                md.push_str(net);
                md.push('\n');
            }
            md.push_str("```\n\n");
        }

        md
    }
}

/// 把 `@eXXXX` 形式的 ref 或裸 CSS 选择器规范化为 CSS 选择器
fn selector_for(target: &str) -> String {
    let t = target.trim();
    if t.starts_with('@') {
        format!("[data-harness-ref=\"{t}\"]")
    } else {
        t.to_string()
    }
}

pub struct BrowserSession {
    _child: Arc<Mutex<Option<Child>>>,
    profile_dir: Option<PathBuf>,
    context_id: Option<String>,
    workspace: Arc<Mutex<Option<PathBuf>>>,
    cdp: Arc<CdpClient>,
    target_id: String,
    target_session_id: String,
    ws_url: String,
    /// 每会话操作互斥锁：防止 Agent 并发批次与人工接管交错操作同一 CDP target
    op_lock: Arc<tokio::sync::Mutex<()>>,
    /// 上一次页面快照游标（供 diff 增量返回使用）
    last_snapshot: Arc<tokio::sync::Mutex<Option<super::snapshot::PageSnapshot>>>,
    /// console / network 日志读取游标（只回新增，不反复回吐陈旧日志）
    log_cursor: Arc<tokio::sync::Mutex<(usize, usize)>>,
}

impl BrowserSession {
    /// 获取操作互斥锁（带超时，避免长批次饿死人工操作）
    pub async fn acquire_op_lock(&self) -> Result<tokio::sync::OwnedMutexGuard<()>, String> {
        let lock = self.op_lock.clone();
        match tokio::time::timeout(Duration::from_secs(30), lock.lock_owned()).await {
            Ok(guard) => Ok(guard),
            Err(_) => Err("浏览器操作繁忙：等待操作锁超时（30s），请稍后重试".to_string()),
        }
    }

    pub fn snapshot_cursor(&self) -> Arc<tokio::sync::Mutex<Option<super::snapshot::PageSnapshot>>> {
        self.last_snapshot.clone()
    }

    pub fn log_cursor(&self) -> Arc<tokio::sync::Mutex<(usize, usize)>> {
        self.log_cursor.clone()
    }

    pub async fn launch(headless: bool, workspace: Option<PathBuf>) -> Result<Self, String> {
        // 1. 默认无头模式：走全局常驻 BrowserDaemon + BrowserContext 毫秒级轻量沙箱隔离
        if headless {
            super::diag::log("session.launch", "无头模式启动：ensure_running 开始");
            let daemon = super::transport::global_browser_daemon();
            let cdp = match daemon.ensure_running().await {
                Ok(c) => {
                    super::diag::log("session.launch", "ensure_running 成功");
                    c
                }
                Err(e) => {
                    super::diag::log("session.launch", format!("ensure_running 失败: {e}"));
                    return Err(e);
                }
            };
            super::diag::log("session.launch", "create_context 开始");
            let context_id = match daemon.create_context().await {
                Ok(c) => {
                    super::diag::log("session.launch", format!("create_context 成功: {c}"));
                    c
                }
                Err(e) => {
                    super::diag::log("session.launch", format!("create_context 失败: {e}"));
                    return Err(e);
                }
            };

            // 在该隔离上下文创建 Target 页面
            super::diag::log("session.launch", "create_target_in_context 开始");
            let target_id = daemon.create_target_in_context(Some(&context_id), "about:blank").await?;
            super::diag::log("session.launch", format!("target 创建成功: {target_id}"));
            let target_session_id = daemon.attach_to_target(&target_id).await?;
            super::diag::log("session.launch", format!("attach 成功: {target_session_id}"));

            // 激活所需领域
            let sid = Some(target_session_id.as_str());
            let _ = cdp.send("Page.enable", None, sid, Duration::from_secs(5)).await;
            let _ = cdp.send("Runtime.enable", None, sid, Duration::from_secs(5)).await;
            let _ = cdp.send("Network.enable", None, sid, Duration::from_secs(5)).await;
            let _ = cdp.send("DOM.enable", None, sid, Duration::from_secs(5)).await;
            let _ = cdp.send("Log.enable", None, sid, Duration::from_secs(5)).await;

            // 激活当前标签页并设置视口，开启焦点模拟，防止后台节流与 0x0 渲染
            let _ = cdp.send("Target.activateTarget", Some(json!({ "targetId": target_id })), None, Duration::from_secs(5)).await;
            let _ = cdp.send("Page.bringToFront", None, sid, Duration::from_secs(5)).await;
            let _ = cdp.send("Emulation.setFocusEmulationEnabled", Some(json!({ "enabled": true })), sid, Duration::from_secs(5)).await;
            let _ = cdp.send(
                "Emulation.setDeviceMetricsOverride",
                Some(json!({
                    "width": 1440,
                    "height": 960,
                    "deviceScaleFactor": 1,
                    "mobile": false
                })),
                sid,
                Duration::from_secs(5),
            ).await;

            let ws_url = daemon.get_ws_url().await.unwrap_or_default();

            return Ok(Self {
                _child: Arc::new(Mutex::new(None)),
                profile_dir: None,
                context_id: Some(context_id),
                workspace: Arc::new(Mutex::new(workspace)),
                cdp,
                target_id,
                target_session_id,
                ws_url,
                op_lock: Arc::new(tokio::sync::Mutex::new(())),
                last_snapshot: Arc::new(tokio::sync::Mutex::new(None)),
                log_cursor: Arc::new(tokio::sync::Mutex::new((0, 0))),
            });
        }

        // 2. 有头调试模式 (headless = false)：启动独立实体窗口实例
        let browser_bin = detect_chromium_path()?;
        // 持久化 profile：复用工作区下 `.harness/browser_profile`
        //
        // 历史缺陷：此处曾用 temp_dir/harness_browser_gui_{uuid} 每次随机新建，
        // 导致每次开有头窗口都是全新 profile，**登录态 100% 丢失**，必然重登。
        // 改为工作区固定目录后，Cookie/LocalStorage/IndexedDB 得以跨次保留。
        //
        // 注意：工作区路径必须实际可写，否则 Chromium 会因 SingletonLock 创建失败
        // 而以 ExitCode 21 退出（低完整性级别进程写不了 LOCALAPPDATA 的同类问题）。
        let profile_dir = match workspace.as_ref() {
            Some(ws) if !ws.as_os_str().is_empty() => {
                let candidate = ws.join(".harness").join("browser_profile");
                if super::fs_probe::is_writable(ws) {
                    candidate
                } else {
                    super::fs_probe::resolve_writable_dir(None, "harness_browser_gui")
                        .join(format!("gui_{}", uuid::Uuid::new_v4().simple()))
                }
            }
            _ => super::fs_probe::resolve_writable_dir(None, "harness_browser_gui")
                .join(format!("gui_{}", uuid::Uuid::new_v4().simple())),
        };
        let _ = tokio::fs::create_dir_all(&profile_dir).await;

        let mut cmd = tokio::process::Command::new(browser_bin);
        cmd.arg("--remote-debugging-port=0")
            .arg(format!("--user-data-dir={}", profile_dir.to_string_lossy()))
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-extensions")
            .arg("--disable-sync")
            .arg("--proxy-bypass-list=<-loopback>")
            .arg("--window-size=1440,960")
            .arg("about:blank");

        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        #[cfg(windows)]
        {
            // 剥离 Windows 兼容层并注入跳过重启哨兵，防止 Edge 脱壳重启导致句柄脱钩
            cmd.env_remove("__COMPAT_LAYER");
            cmd.arg("--edge-skip-compat-layer-relaunch");
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("启动浏览器窗口实例失败: {e}"))?;

        #[cfg(windows)]
        if let Some(pid) = child.id() {
            crate::tools::assign_pid_to_job(pid);
        }

        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "无法获取浏览器 stderr".to_string())?;

        let mut reader = BufReader::new(stderr).lines();
        let mut ws_url = None;
        let mut accumulated_stderr = Vec::new();
        let timeout = Duration::from_secs(12);
        let start = std::time::Instant::now();

        while start.elapsed() < timeout {
            if let Ok(Some(status)) = child.try_wait() {
                let err_detail = accumulated_stderr.join("\n");
                return Err(format!(
                    "浏览器进程异常退出 (ExitCode: {:?})，启动失败！\n输出日志:\n{}",
                    status.code(),
                    if err_detail.is_empty() { "无 stderr 输出" } else { &err_detail }
                ));
            }

            let active_port_file = profile_dir.join("DevToolsActivePort");
            if active_port_file.exists() {
                if let Ok(content) = std::fs::read_to_string(&active_port_file) {
                    let lines: Vec<&str> = content.lines().collect();
                    if lines.len() >= 2 {
                        let port = lines[0].trim();
                        let path = lines[1].trim();
                        if !port.is_empty() && !path.is_empty() {
                            ws_url = Some(format!("ws://127.0.0.1:{port}{path}"));
                            break;
                        }
                    }
                }
            }

            if let Ok(Ok(Some(l))) = tokio::time::timeout(Duration::from_millis(100), reader.next_line()).await {
                if let Some(idx) = l.find("DevTools listening on ws://") {
                    let ws = l[idx + "DevTools listening on ".len()..].trim().to_string();
                    ws_url = Some(ws);
                    break;
                }
                accumulated_stderr.push(l);
            }
        }

        let ws_url = ws_url.ok_or_else(|| {
            let err_detail = accumulated_stderr.join("\n");
            format!(
                "等待浏览器 DevTools WebSocket 地址超时！\n详情:\n{}",
                if err_detail.is_empty() { "无 stderr 输出" } else { &err_detail }
            )
        })?;

        tokio::spawn(async move {
            while let Ok(Some(_)) = reader.next_line().await {}
        });

        let cdp = Arc::new(CdpClient::connect(&ws_url).await?);

        // 查找或创建 Page Target
        let targets_res = cdp
            .send("Target.getTargets", None, None, Duration::from_secs(5))
            .await?;

        let existing_target_id = targets_res
            .get("targetInfos")
            .and_then(|t| t.as_array())
            .and_then(|arr| {
                arr.iter().find(|info| {
                    info.get("type").and_then(|tp| tp.as_str()) == Some("page")
                })
            })
            .and_then(|info| info.get("targetId").and_then(|id| id.as_str()))
            .map(|s| s.to_string());

        let target_id = if let Some(tid) = existing_target_id {
            tid
        } else {
            let create_res = cdp
                .send("Target.createTarget", Some(json!({ "url": "about:blank" })), None, Duration::from_secs(10))
                .await?;
            create_res
                .get("targetId")
                .and_then(|t| t.as_str())
                .ok_or_else(|| "Target.createTarget 未返回 targetId".to_string())?
                .to_string()
        };

        let attach_res = cdp
            .send(
                "Target.attachToTarget",
                Some(json!({ "targetId": target_id, "flatten": true })),
                None,
                Duration::from_secs(10),
            )
            .await?;
        let target_session_id = attach_res
            .get("sessionId")
            .and_then(|s| s.as_str())
            .ok_or_else(|| "Target.attachToTarget 未返回 sessionId".to_string())?
            .to_string();

        let sid = Some(target_session_id.as_str());
        let _ = cdp.send("Page.enable", None, sid, Duration::from_secs(5)).await;
        let _ = cdp.send("Runtime.enable", None, sid, Duration::from_secs(5)).await;
        let _ = cdp.send("Network.enable", None, sid, Duration::from_secs(5)).await;
        let _ = cdp.send("DOM.enable", None, sid, Duration::from_secs(5)).await;
        let _ = cdp.send("Log.enable", None, sid, Duration::from_secs(5)).await;

        let _ = cdp.send("Target.activateTarget", Some(json!({ "targetId": target_id })), None, Duration::from_secs(5)).await;
        let _ = cdp.send("Page.bringToFront", None, sid, Duration::from_secs(5)).await;
        let _ = cdp.send("Emulation.setFocusEmulationEnabled", Some(json!({ "enabled": true })), sid, Duration::from_secs(5)).await;
        let _ = cdp.send(
            "Emulation.setDeviceMetricsOverride",
            Some(json!({
                "width": 1440,
                "height": 960,
                "deviceScaleFactor": 1,
                "mobile": false
            })),
            sid,
            Duration::from_secs(5),
        ).await;

        Ok(Self {
            _child: Arc::new(Mutex::new(Some(child))),
            profile_dir: Some(profile_dir),
            context_id: None,
            workspace: Arc::new(Mutex::new(workspace)),
            cdp,
            target_id,
            target_session_id,
            ws_url,
            op_lock: Arc::new(tokio::sync::Mutex::new(())),
            last_snapshot: Arc::new(tokio::sync::Mutex::new(None)),
            log_cursor: Arc::new(tokio::sync::Mutex::new((0, 0))),
        })
    }

    pub async fn set_workspace(&self, ws: PathBuf) {
        let mut lock = self.workspace.lock().await;
        *lock = Some(ws);
    }

    pub async fn get_workspace(&self) -> Option<PathBuf> {
        self.workspace.lock().await.clone()
    }

    pub fn cdp(&self) -> &Arc<CdpClient> {
        &self.cdp
    }

    pub fn target_session_id(&self) -> &str {
        &self.target_session_id
    }

    pub async fn is_alive(&self) -> bool {
        if !self.cdp.is_alive() {
            return false;
        }
        if self.context_id.is_some() {
            let check = self
                .cdp
                .send(
                    "Target.getTargetInfo",
                    Some(json!({ "targetId": self.target_id })),
                    None,
                    Duration::from_millis(500),
                )
                .await;
            return check.is_ok();
        }

        let mut child = self._child.lock().await;
        if let Some(ref mut c) = *child {
            match c.try_wait() {
                Ok(None) => true,
                _ => false,
            }
        } else {
            false
        }
    }

    pub async fn start_screencast(&self) -> Result<(), String> {
        let sid = Some(self.target_session_id.as_str());
        let screencast_params = json!({
            "format": "jpeg",
            "quality": 70,
            "maxWidth": 1280,
            "maxHeight": 800,
            "everyNthFrame": 1
        });
        self.cdp.send("Page.startScreencast", Some(screencast_params), sid, Duration::from_secs(5)).await?;
        Ok(())
    }

    pub async fn stop_screencast(&self) -> Result<(), String> {
        let sid = Some(self.target_session_id.as_str());
        self.cdp.send("Page.stopScreencast", None, sid, Duration::from_secs(5)).await?;
        Ok(())
    }

    pub async fn navigate(&self, url: &str) -> Result<BrowserDebugState, String> {
        let normalized_url = if !url.contains("://") && !url.starts_with("about:") {
            format!("http://{url}")
        } else {
            url.to_string()
        };

        let sid = Some(self.target_session_id.as_str());

        // 1. 关键：在导航发起之前提前恢复 StorageState (Cookies + addScriptToEvaluateOnNewDocument)
        // 这样首行业务 JS 执行时即具备完整鉴权凭证，彻底避免未登录闪烁与二次重定向
        if let Some(ref ws) = *self.workspace.lock().await {
            let _ = super::storage::restore_storage_state(&self.cdp, &self.target_session_id, ws, Some(&normalized_url)).await;
        }

        // 2. 发起页面导航
        self.cdp
            .send("Page.navigate", Some(json!({ "url": normalized_url })), sid, Duration::from_secs(30))
            .await?;

        // 3. 智能就绪等待 (Smart Readiness Wait)：
        // 轮询等待 document.readyState === 'complete'，并确保 SPA 框架完成挂载
        let wait_start = std::time::Instant::now();
        let max_wait = Duration::from_millis(3500);
        let min_wait = Duration::from_millis(500);

        while wait_start.elapsed() < max_wait {
            tokio::time::sleep(Duration::from_millis(200)).await;
            if wait_start.elapsed() < min_wait {
                continue;
            }

            let check_res = self.cdp.send(
                "Runtime.evaluate",
                Some(json!({
                    "expression": "document.readyState === 'complete'",
                    "returnByValue": true
                })),
                sid,
                Duration::from_millis(500),
            ).await;

            if let Ok(val) = check_res {
                if val.get("result").and_then(|r| r.get("value")).and_then(|v| v.as_bool()) == Some(true) {
                    // readyState 已是 complete，再缓冲 300ms 等 Vue/React 组件树完成初次 mount
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    break;
                }
            }
        }

        // 4. 确保窗口激活与焦点模拟
        let _ = self.cdp.send("Page.bringToFront", None, sid, Duration::from_secs(2)).await;
        let _ = self.cdp.send("Emulation.setFocusEmulationEnabled", Some(json!({ "enabled": true })), sid, Duration::from_secs(2)).await;

        self.get_debug_state().await
    }

    /// 通过 CDP Input 域派发真实鼠标点击（isTrusted = true）
    ///
    /// 为什么必须走真实事件：`Runtime.evaluate` 里调 `el.click()` 产生的是合成事件，
    /// `isTrusted === false`，多数风控与表单校验库会直接忽略 —— 这正是「人工点能过、
    /// Agent 点不过」的原因。
    ///
    /// 2026-11-24 实测定案（保存按钮点两次都不生效）：
    /// 旧实现取 getBoundingClientRect 中心坐标后直接发事件，**不校验该坐标实际命中了谁**：
    /// 弹窗动画未结束 / 遮罩覆盖时，点击落在遮罩层上，目标元素收不到事件却静默返回 Ok。
    /// 现改为：命中检测 → 三次重试（等动画/重滚/偏移点）→ 完整事件序列 → 点击后复核。
    pub async fn real_click_by_selector(&self, target: &str) -> Result<(), String> {
        let sid = Some(self.target_session_id.as_str());
        let sel_escaped = selector_for(target).replace('"', "\\\"");

        // 三次尝试：(等待毫秒, 是否使用偏移点)
        let attempts: [(u64, bool); 3] = [(0, false), (300, false), (200, true)];
        let mut last_hit: String = String::new();

        for (idx, (wait_ms, use_offset)) in attempts.iter().enumerate() {
            if *wait_ms > 0 {
                tokio::time::sleep(Duration::from_millis(*wait_ms)).await;
            }

            // ---- 1. 解析坐标 + 命中检测 ----
            let coord_script = format!(
                r#"(() => {{
                    {find}
                    const el = findTarget("{sel}");
                    if (!el) return {{ ok: false, err: "未找到目标元素" }};
                    el.scrollIntoView({{ behavior: 'instant', block: 'center' }});
                    const r = el.getBoundingClientRect();
                    if (!(r.width > 0 && r.height > 0)) return {{ ok: false, err: "元素不可见（尺寸为零）" }};
                    if (el.disabled || el.getAttribute('aria-disabled') === 'true') {{
                        return {{ ok: false, err: "元素处于 disabled 状态" }};
                    }}

                    // 偏移模式下取元素内部左上一点，绕开中心被遮挡的情况
                    const useOffset = {use_offset};
                    const x = useOffset ? (r.left + Math.min(6, r.width / 3)) : (r.left + r.width / 2);
                    const y = useOffset ? (r.top + Math.min(6, r.height / 3)) : (r.top + r.height / 2);

                    // 命中检测：该坐标上最顶层的元素是不是目标（或其后代）
                    const hit = document.elementFromPoint(x, y);
                    const hitSelf = !!(hit && (hit === el || el.contains(hit)));
                    const desc = (n) => n ? (n.tagName.toLowerCase() + (typeof n.className === 'string' && n.className
                        ? '.' + n.className.trim().split(/\s+/)[0] : '')) : null;
                    return {{
                        ok: true, x: x, y: y, hitSelf: hitSelf,
                        hitDesc: desc(hit)
                    }};
                }})()"#,
                find = super::dom_script::FIND_TARGET_JS,
                sel = sel_escaped,
                use_offset = use_offset
            );

            let res = self
                .cdp
                .send(
                    "Runtime.evaluate",
                    Some(json!({ "expression": coord_script, "returnByValue": true })),
                    sid,
                    Duration::from_secs(5),
                )
                .await?;

            let val = res.get("result").and_then(|r| r.get("value")).cloned().unwrap_or(json!({}));
            if val.get("ok").and_then(|v| v.as_bool()) != Some(true) {
                let msg = val
                    .get("err")
                    .and_then(|e| e.as_str())
                    .unwrap_or("元素坐标解析失败")
                    .to_string();
                // disabled / 不可见属于确定性失败，无需重试
                if msg.contains("disabled") || msg.contains("不可见") || msg.contains("未找到") {
                    return Err(msg);
                }
                last_hit = msg;
                continue;
            }

            let hit_self = val.get("hitSelf").and_then(|v| v.as_bool()).unwrap_or(false);
            if !hit_self {
                // 被遮挡：记录命中者，继续下一次重试
                last_hit = val
                    .get("hitDesc")
                    .and_then(|v| v.as_str())
                    .unwrap_or("未知元素")
                    .to_string();
                continue;
            }

            let x = val.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let y = val.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);

            // ---- 2. 完整事件序列：移动 → 按下 → 微延迟 → 抬起 ----
            let _ = self
                .cdp
                .send(
                    "Input.dispatchMouseEvent",
                    Some(json!({ "type": "mouseMoved", "x": x, "y": y, "button": "none", "buttons": 0 })),
                    sid,
                    Duration::from_secs(3),
                )
                .await;
            self.cdp
                .send(
                    "Input.dispatchMouseEvent",
                    Some(json!({
                        "type": "mousePressed", "x": x, "y": y,
                        "button": "left", "buttons": 1, "clickCount": 1
                    })),
                    sid,
                    Duration::from_secs(5),
                )
                .await?;
            tokio::time::sleep(Duration::from_millis(30)).await;
            // mouseReleased 必须带 clickCount，否则部分 Chromium 版本不合成 click 事件
            self.cdp
                .send(
                    "Input.dispatchMouseEvent",
                    Some(json!({
                        "type": "mouseReleased", "x": x, "y": y,
                        "button": "left", "buttons": 0, "clickCount": 1
                    })),
                    sid,
                    Duration::from_secs(5),
                )
                .await?;

            // ---- 3. 点击后复核：确认事件确实落在目标上 ----
            tokio::time::sleep(Duration::from_millis(120)).await;
            let verify_script = format!(
                r#"(() => {{
                    {find}
                    const el = findTarget("{sel}");
                    if (!el) return {{ gone: true }};
                    return {{ gone: false }};
                }})()"#,
                find = super::dom_script::FIND_TARGET_JS,
                sel = sel_escaped
            );
            let _ = self
                .cdp
                .send(
                    "Runtime.evaluate",
                    Some(json!({ "expression": verify_script, "returnByValue": true })),
                    sid,
                    Duration::from_secs(3),
                )
                .await;

            let _ = idx;
            return Ok(());
        }

        // 三次都被遮挡：明确失败，绝不静默返回成功
        Err(format!(
            "CLICK_BLOCKED: 元素被遮挡无法真实点击（3 次重试后坐标仍命中的是 {last_hit}）。\
             常见原因：弹窗遮罩未关闭、元素正在动画中、或按钮被其它浮层覆盖。",
        ))
    }

    /// 通过 CDP 真实键盘事件输入文本（聚焦 + insertText），isTrusted = true
    pub async fn real_fill_by_selector(&self, target: &str, text: &str) -> Result<(), String> {
        let sid = Some(self.target_session_id.as_str());

        // 1. 聚焦元素并清空
        let focus_script = format!(
            r#"(() => {{
                {find}
                const el = findTarget("{sel}");
                if (!el) return {{ ok: false, err: "未找到目标元素" }};
                if (el.disabled) return {{ ok: false, err: "元素处于 disabled 状态" }};
                if (el.readOnly) return {{ ok: false, err: "元素处于 readonly 状态" }};
                el.scrollIntoView({{ behavior: 'instant', block: 'center' }});
                el.focus();
                try {{ el.select && el.select(); }} catch (_) {{}}
                return {{ ok: true }};
            }})()"#,
            find = super::dom_script::FIND_TARGET_JS,
            sel = selector_for(target).replace('"', "\\\"")
        );
        let res = self
            .cdp
            .send(
                "Runtime.evaluate",
                Some(json!({ "expression": focus_script, "returnByValue": true })),
                sid,
                Duration::from_secs(5),
            )
            .await?;
        let val = res.get("result").and_then(|r| r.get("value")).cloned().unwrap_or(json!({}));
        if val.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            return Err(val
                .get("err")
                .and_then(|e| e.as_str())
                .unwrap_or("元素聚焦失败")
                .to_string());
        }

        // 2. 真实文本输入
        self.cdp
            .send("Input.insertText", Some(json!({ "text": text })), sid, Duration::from_secs(5))
            .await?;

        // 3. 同步派发 input/change/blur，兜住 Vue v-model 与 Element UI 校验
        let sync_script = r#"(() => {
            const a = document.activeElement;
            if (a && (a.tagName === 'INPUT' || a.tagName === 'TEXTAREA' || a.isContentEditable)) {
                a.dispatchEvent(new Event('input', { bubbles: true, composed: true }));
                a.dispatchEvent(new Event('change', { bubbles: true, composed: true }));
                a.dispatchEvent(new Event('blur', { bubbles: true, composed: true }));
            }
        })()"#;
        let _ = self
            .cdp
            .send(
                "Runtime.evaluate",
                Some(json!({ "expression": sync_script, "returnByValue": false })),
                sid,
                Duration::from_secs(3),
            )
            .await;

        Ok(())
    }

    /// 轮询单次判定脚本，直到返回 success=true 或超时
    ///
    /// 替代固定 sleep：慢的时候不误判失败，快的时候不无谓空等。
    pub async fn poll_until(
        &self,
        script: &str,
        timeout_ms: u64,
        poll_interval_ms: u64,
    ) -> Result<(bool, String, String), String> {
        let sid = Some(self.target_session_id.as_str());
        let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
        let mut last_err = String::new();

        loop {
            let res = self
                .cdp
                .send(
                    "Runtime.evaluate",
                    Some(json!({ "expression": script, "returnByValue": true })),
                    sid,
                    Duration::from_secs(10),
                )
                .await;

            match res {
                Ok(eval_res) => {
                    let val = eval_res
                        .get("result")
                        .and_then(|r| r.get("value"))
                        .cloned()
                        .unwrap_or(json!({}));
                    let ok = val.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
                    if ok {
                        let extra = val.get("url").and_then(|u| u.as_str()).unwrap_or("").to_string();
                        return Ok((true, extra, String::new()));
                    }
                    if let Some(e) = val.get("error").and_then(|e| e.as_str()) {
                        last_err = e.to_string();
                    }
                }
                Err(e) => {
                    last_err = e;
                }
            }

            if std::time::Instant::now() >= deadline {
                let waited = timeout_ms;
                return Ok((
                    false,
                    String::new(),
                    if last_err.is_empty() {
                        format!("等待超时（{waited}ms）")
                    } else {
                        format!("等待超时（{waited}ms）：{last_err}")
                    },
                ));
            }
            tokio::time::sleep(Duration::from_millis(poll_interval_ms)).await;
        }
    }

    /// 在途网络请求数快照（总数 + 前几条 "METHOD URL" 样本）
    ///
    /// 用于 `wait_for_network_idle` 做真实「网络静默」判定：
    /// 只看 `document.readyState` 无法反映 SPA 的 XHR/fetch 在途状态。
    pub async fn inflight_requests(&self, sample: usize) -> (usize, Vec<String>) {
        self.cdp.inflight_requests(sample).await
    }

    pub async fn interact(&self, action: &str, target: &str, value: Option<&str>) -> Result<BrowserDebugState, String> {
        let sid = Some(self.target_session_id.as_str());
        let script = build_interact_script(action, target, value);

        let eval_res = self
            .cdp
            .send(
                "Runtime.evaluate",
                Some(json!({
                    "expression": script,
                    "returnByValue": true,
                    "awaitPromise": true
                })),
                sid,
                Duration::from_secs(10),
            )
            .await?;

        // 检查执行结果
        if let Some(err) = eval_res
            .get("result")
            .and_then(|r| r.get("value"))
            .and_then(|v| v.get("error"))
            .and_then(|e| e.as_str())
        {
            return Err(format!("操作失败: {err}"));
        }

        // 若是点击或按回车操作，等待单页应用路由跳转与网络请求响应（1000ms 智能防抖）
        let wait_ms = if action == "click" || action == "press_enter" {
            1000
        } else {
            300
        };
        tokio::time::sleep(Duration::from_millis(wait_ms)).await;

        self.get_debug_state().await
    }

    pub async fn get_debug_state(&self) -> Result<BrowserDebugState, String> {
        let sid = Some(self.target_session_id.as_str());
        let eval_res = self
            .cdp
            .send(
                "Runtime.evaluate",
                Some(json!({
                    "expression": EXTRACT_TREE_SCRIPT,
                    "returnByValue": true
                })),
                sid,
                Duration::from_secs(10),
            )
            .await?;

        let val = eval_res
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(Value::Null);

        let url = val.get("url").and_then(|u| u.as_str()).unwrap_or("").to_string();
        let title = val.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string();

        let mut elements = Vec::new();
        if let Some(arr) = val.get("elements").and_then(|e| e.as_array()) {
            for item in arr {
                elements.push(InteractiveElement {
                    ref_id: item.get("ref").and_then(|r| r.as_str()).unwrap_or("").to_string(),
                    tag: item.get("tag").and_then(|t| t.as_str()).unwrap_or("").to_string(),
                    r#type: item.get("type").and_then(|t| t.as_str()).map(|s| s.to_string()),
                    name: item.get("name").and_then(|n| n.as_str()).map(|s| s.to_string()),
                    placeholder: item.get("placeholder").and_then(|p| p.as_str()).map(|s| s.to_string()),
                    text: item.get("text").and_then(|t| t.as_str()).map(|s| s.to_string()),
                    value: item.get("value").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    href: item.get("href").and_then(|h| h.as_str()).map(|s| s.to_string()),
                    in_dialog: item.get("inDialog").and_then(|d| d.as_bool()).unwrap_or(false),
                });
            }
        }

        let mut notices = Vec::new();
        if let Some(arr) = val.get("notices").and_then(|n| n.as_array()) {
            for item in arr {
                if let Some(s) = item.as_str() {
                    notices.push(s.to_string());
                }
            }
        }

        let mut dom_overlays = Vec::new();
        if let Some(arr) = val.get("dom_overlays").and_then(|o| o.as_array()) {
            for item in arr {
                if let Some(s) = item.as_str() {
                    dom_overlays.push(s.to_string());
                }
            }
        }

        let mut tables = Vec::new();
        if let Some(arr) = val.get("tables").and_then(|t| t.as_array()) {
            for item in arr {
                if let Some(s) = item.as_str() {
                    tables.push(s.to_string());
                }
            }
        }

        let console_errors = {
            let logs = self.cdp.console_logs.lock().await;
            logs.iter()
                .filter(|l| l.contains("[Console.error]") || l.contains("[Uncaught Exception]") || l.contains("[Log.error]"))
                .cloned()
                .collect()
        };

        let network_errors = {
            let net = self.cdp.network_errors.lock().await;
            net.clone()
        };

        let page_state = super::evaluator::evaluate_page_state(self).await;

        let viewport = val
            .get("viewport")
            .and_then(|vp| serde_json::from_value::<ViewportInfo>(vp.clone()).ok())
            .unwrap_or_default();

        let hidden_elements_count = val
            .get("hidden_elements_count")
            .and_then(|h| h.as_u64())
            .unwrap_or(0) as usize;

        Ok(BrowserDebugState {
            url,
            title,
            viewport,
            elements,
            hidden_elements_count,
            console_errors,
            network_errors,
            tables,
            notices,
            dom_overlays,
            page_state,
        })
    }

    pub async fn capture_screenshot(&self, full_page: bool) -> Result<String, String> {
        let sid = Some(self.target_session_id.as_str());
        let mut params = json!({ "format": "jpeg", "quality": 85 });
        if full_page {
            params["captureBeyondViewport"] = json!(true);
        }
        let res = self
            .cdp
            .send(
                "Page.captureScreenshot",
                Some(params),
                sid,
                Duration::from_secs(20),
            )
            .await?;

        res.get("data")
            .and_then(|d| d.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "截图失败: 未返回图像数据".to_string())
    }

    pub async fn scroll(
        &self,
        direction: Option<&str>,
        distance: Option<f64>,
        target: Option<&str>,
    ) -> Result<BrowserDebugState, String> {
        let sid = Some(self.target_session_id.as_str());
        let script = build_scroll_script(direction.unwrap_or("down"), distance, target);

        let eval_res = self
            .cdp
            .send(
                "Runtime.evaluate",
                Some(json!({
                    "expression": script,
                    "returnByValue": true,
                    "awaitPromise": true
                })),
                sid,
                Duration::from_secs(10),
            )
            .await?;

        if let Some(err) = eval_res
            .get("result")
            .and_then(|r| r.get("value"))
            .and_then(|v| v.get("error"))
            .and_then(|e| e.as_str())
        {
            return Err(format!("滚动操作失败: {err}"));
        }

        tokio::time::sleep(Duration::from_millis(400)).await;
        self.get_debug_state().await
    }

    pub async fn user_wheel(&self, x: f64, y: f64, delta_x: f64, delta_y: f64) -> Result<(), String> {
        let sid = Some(self.target_session_id.as_str());
        let params = json!({
            "type": "mouseWheel",
            "x": x,
            "y": y,
            "deltaX": delta_x,
            "deltaY": delta_y
        });
        self.cdp.send("Input.dispatchMouseEvent", Some(params), sid, Duration::from_secs(5)).await?;
        Ok(())
    }

    pub async fn open_devtools_window(&self) -> Result<(), String> {
        let clean_ws = self.ws_url.trim_start_matches("ws://").trim_start_matches("http://");
        let host_port = clean_ws.split('/').next().unwrap_or("127.0.0.1");

        let inspector_url = format!(
            "http://{host_port}/devtools/inspector.html?ws={host_port}/devtools/page/{}",
            self.target_id
        );

        #[cfg(windows)]
        {
            let res = std::process::Command::new("cmd")
                .args(["/c", "start", "", &inspector_url])
                .spawn();
            if let Err(e) = res {
                return Err(format!("唤起独立调试窗口失败: {e}"));
            }
        }

        #[cfg(not(windows))]
        {
            let res = std::process::Command::new("xdg-open")
                .arg(&inspector_url)
                .spawn();
            if let Err(e) = res {
                return Err(format!("唤起独立调试窗口失败: {e}"));
            }
        }

        Ok(())
    }

    pub async fn user_click(&self, x: f64, y: f64) -> Result<(), String> {
        let sid = Some(self.target_session_id.as_str());

        // 1. 确保页面获得窗口前台焦点 (Focus Emulation)
        let _ = self.cdp.send("Emulation.setFocusEmulationEnabled", Some(json!({ "enabled": true })), sid, Duration::from_secs(2)).await;

        // 2. 模拟真实光标移动 (mouseMoved) 触发 hover / active 态
        let move_evt = json!({
            "type": "mouseMoved",
            "x": x,
            "y": y
        });
        let _ = self.cdp.send("Input.dispatchMouseEvent", Some(move_evt), sid, Duration::from_secs(3)).await;

        // 3. 模拟鼠标左键按下 (mousePressed)
        let press = json!({
            "type": "mousePressed",
            "x": x,
            "y": y,
            "button": "left",
            "clickCount": 1
        });
        self.cdp.send("Input.dispatchMouseEvent", Some(press), sid, Duration::from_secs(5)).await?;

        // 真实人类物理按压微延迟 35ms，确保框架事件监听器可靠捕获完整点击动作
        tokio::time::sleep(Duration::from_millis(35)).await;

        // 4. 模拟鼠标左键抬起 (mouseReleased)
        let release = json!({
            "type": "mouseReleased",
            "x": x,
            "y": y,
            "button": "left"
        });
        self.cdp.send("Input.dispatchMouseEvent", Some(release), sid, Duration::from_secs(5)).await?;

        // 5. 智能聚焦双重兜底 (Dual-Guaranteed Focus)：
        // 若坐标下命中输入控件（如 input / textarea / contenteditable 等 UI 组件库包装层），显式触发 focus() 获得光标
        let focus_script = format!(
            r#"(() => {{
                try {{
                    const el = document.elementFromPoint({x}, {y});
                    if (!el) return false;
                    const isInput = el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.tagName === 'SELECT' || el.isContentEditable;
                    if (isInput) {{
                        el.focus();
                        return true;
                    }}
                    const innerInput = el.querySelector('input:not([type="hidden"]), textarea, [contenteditable="true"]');
                    if (innerInput) {{
                        innerInput.focus();
                        return true;
                    }}
                    const parentInput = el.closest('label, .el-input, .ant-input-affix-wrapper, .n-input, .t-input, .arco-input-wrapper');
                    if (parentInput) {{
                        const inp = parentInput.querySelector('input:not([type="hidden"]), textarea');
                        if (inp) {{
                            inp.focus();
                            return true;
                        }}
                    }}
                    return false;
                }} catch (e) {{
                    return false;
                }}
            }})()"#
        );
        let _ = self.cdp.send(
            "Runtime.evaluate",
            Some(json!({
                "expression": focus_script,
                "returnByValue": true
            })),
            sid,
            Duration::from_secs(2),
        ).await;

        Ok(())
    }

    pub async fn user_type_text(&self, text: &str) -> Result<(), String> {
        let sid = Some(self.target_session_id.as_str());

        // 1. 确保页面获得焦点
        let _ = self.cdp.send("Emulation.setFocusEmulationEnabled", Some(json!({ "enabled": true })), sid, Duration::from_secs(2)).await;

        // 2. 若当前 activeElement 不是有效输入框，尝试自动聚焦当前视口内首个可见输入框
        let ensure_focus_script = r#"(() => {
            const active = document.activeElement;
            if (active && (active.tagName === 'INPUT' || active.tagName === 'TEXTAREA' || active.isContentEditable)) {
                return true;
            }
            const inputs = Array.from(document.querySelectorAll('input:not([type="hidden"]):not([disabled]), textarea:not([disabled])'));
            for (let inp of inputs) {
                const rect = inp.getBoundingClientRect();
                if (rect.width > 0 && rect.height > 0 && rect.top < window.innerHeight && rect.bottom > 0) {
                    inp.focus();
                    return true;
                }
            }
            return false;
        })()"#;
        let _ = self.cdp.send(
            "Runtime.evaluate",
            Some(json!({ "expression": ensure_focus_script, "returnByValue": true })),
            sid,
            Duration::from_secs(2),
        ).await;

        // 3. 调用 CDP 输入文字
        let params = json!({ "text": text });
        self.cdp.send("Input.insertText", Some(params), sid, Duration::from_secs(5)).await?;

        // 4. 派发 input 与 change 事件，确保 Vue 3 / React 的响应式受控表单 (v-model) 彻底同步
        let sync_script = r#"(() => {
            const active = document.activeElement;
            if (active && (active.tagName === 'INPUT' || active.tagName === 'TEXTAREA')) {
                active.dispatchEvent(new Event('input', { bubbles: true }));
                active.dispatchEvent(new Event('change', { bubbles: true }));
            }
        })()"#;
        let _ = self.cdp.send(
            "Runtime.evaluate",
            Some(json!({ "expression": sync_script, "returnByValue": false })),
            sid,
            Duration::from_secs(2),
        ).await;

        Ok(())
    }

    pub async fn close(&self) {
        // 保存全量 storage_state (Cookies/LocalStorage/SessionStorage)
        let ws = self.get_workspace().await.unwrap_or_default();
        let curr_url = self.get_debug_state().await.ok().map(|d| d.url);
        let _ = super::storage::save_storage_state(&self.cdp, &self.target_session_id, &ws, curr_url.as_deref()).await;

        // 若是通过全局 BrowserDaemon 运行的隔离上下文
        if let Some(ref cid) = self.context_id {
            let daemon = super::transport::global_browser_daemon();
            let _ = daemon.close_target(&self.target_id).await;
            let _ = daemon.dispose_context(cid).await;
            return;
        }

        // 若是独立实体窗口进程
        let mut child_guard = self._child.lock().await;
        if let Some(mut child) = child_guard.take() {
            #[cfg(windows)]
            if let Some(pid) = child.id() {
                crate::tools::kill_process_tree(pid);
            }
            let _ = child.kill().await;
        }
        if let Some(ref p) = self.profile_dir {
            let _ = tokio::fs::remove_dir_all(p).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(ref_id: &str, placeholder: &str, in_dialog: bool) -> InteractiveElement {
        InteractiveElement {
            ref_id: ref_id.to_string(),
            tag: "input".to_string(),
            r#type: Some("text".to_string()),
            name: None,
            placeholder: Some(placeholder.to_string()),
            text: None,
            value: None,
            href: None,
            in_dialog,
        }
    }

    fn state_with(elements: Vec<InteractiveElement>) -> BrowserDebugState {
        BrowserDebugState {
            url: "http://a".to_string(),
            title: "T".to_string(),
            viewport: ViewportInfo::default(),
            elements,
            hidden_elements_count: 0,
            console_errors: Vec::new(),
            network_errors: Vec::new(),
            tables: Vec::new(),
            notices: Vec::new(),
            dom_overlays: Vec::new(),
            page_state: Default::default(),
        }
    }

    /// 复现实测痛点：弹窗内与外层页面存在同 placeholder 的 input（指纹仅差 _2/_3 后缀），
    /// 必须分组展示，否则 Agent 无法判断该填哪个。
    #[test]
    fn test_elements_grouped_by_dialog_membership() {
        let st = state_with(vec![
            el("@eaaa", "请输入", false),
            el("@eaaa_2", "请输入", true),
            el("@eaaa_3", "请输入", true),
        ]);
        let md = st.to_markdown();
        assert!(md.contains("**弹窗内元素**"), "应有弹窗内元素分组: {md}");
        assert!(md.contains("**页面元素**"), "应有页面元素分组: {md}");
        assert!(md.contains("其中弹窗内 2 个"), "应统计弹窗内元素数: {md}");
        // 弹窗内分组必须出现在页面元素分组之前
        let dlg_pos = md.find("**弹窗内元素**").unwrap();
        let page_pos = md.find("**页面元素**").unwrap();
        assert!(dlg_pos < page_pos, "弹窗内元素应优先展示");
    }

    #[test]
    fn test_elements_ungrouped_when_no_dialog() {
        let st = state_with(vec![el("@eaaa", "请输入", false)]);
        let md = st.to_markdown();
        assert!(!md.contains("**弹窗内元素**"), "无弹窗内元素时不应出现空分组: {md}");
        assert!(md.contains("@eaaa"), "普通元素仍须列出: {md}");
    }

    #[test]
    fn test_in_dialog_omitted_from_json_when_false() {
        // inDialog=false 不应污染既有 JSON 契约
        let v = serde_json::to_value(el("@e1", "请输入", false)).unwrap();
        assert!(v.get("in_dialog").is_none(), "false 时不应序列化 in_dialog: {v}");
        let v2 = serde_json::to_value(el("@e2", "请输入", true)).unwrap();
        assert_eq!(v2.get("in_dialog").and_then(|x| x.as_bool()), Some(true));
    }
}
