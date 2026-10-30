use super::cdp::CdpClient;
use super::detector::detect_chromium_path;
use super::dom_script::{build_interact_script, build_scroll_script, EXTRACT_TREE_SCRIPT};
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
                    "> 💡 **长页面提示**: 当前视口下方仍有未展示内容（视口外折叠约 {} 个交互元素）。如需继续向下探索，可调用 `browser_scroll(direction=\"down\")`。\n",
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

        md.push_str("#### 可交互元素清单 (可通过 @eX 进行操作):\n");
        if self.elements.is_empty() {
            md.push_str("(当前视口内未检测到可交互元素)\n\n");
        } else {
            for el in &self.elements {
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
                md.push_str(&desc);
                md.push('\n');
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

pub struct BrowserSession {
    _child: Arc<Mutex<Option<Child>>>,
    profile_dir: Option<PathBuf>,
    context_id: Option<String>,
    workspace: Arc<Mutex<Option<PathBuf>>>,
    cdp: Arc<CdpClient>,
    target_id: String,
    target_session_id: String,
    ws_url: String,
}

impl BrowserSession {
    pub async fn launch(headless: bool, workspace: Option<PathBuf>) -> Result<Self, String> {
        // 1. 默认无头模式：走全局常驻 BrowserDaemon + BrowserContext 毫秒级轻量沙箱隔离
        if headless {
            let daemon = super::daemon::global_browser_daemon();
            let cdp = daemon.ensure_running().await?;
            let context_id = daemon.create_context().await?;

            // 在该隔离上下文创建 Target 页面
            let target_id = daemon.create_target_in_context(Some(&context_id), "about:blank").await?;
            let target_session_id = daemon.attach_to_target(&target_id).await?;

            // 激活所需领域
            let sid = Some(target_session_id.as_str());
            let _ = cdp.send("Page.enable", None, sid, Duration::from_secs(5)).await;
            let _ = cdp.send("Runtime.enable", None, sid, Duration::from_secs(5)).await;
            let _ = cdp.send("Network.enable", None, sid, Duration::from_secs(5)).await;
            let _ = cdp.send("DOM.enable", None, sid, Duration::from_secs(5)).await;
            let _ = cdp.send("Log.enable", None, sid, Duration::from_secs(5)).await;

            // 激活当前标签页并设置视口，开启焦点模拟，防止后台节流和 0x0 渲染
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
            });
        }

        // 2. 有头调试模式 (headless = false)：启动独立实体窗口实例
        let browser_bin = detect_chromium_path()?;
        let profile_dir = std::env::temp_dir().join(format!("harness_browser_gui_{}", uuid::Uuid::new_v4()));
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
                    // readyState 已 complete，再缓冲 300ms 供 Vue/React 组件树完成初始 mount
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
        // 若坐标下命中输入控件（如 input / textarea / contenteditable 或 UI 组件库包装层），显式触发 focus() 获得光标
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
            let daemon = super::daemon::global_browser_daemon();
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
