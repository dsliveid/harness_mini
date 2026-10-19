use super::auth::{load_storage_state, save_storage_state, AuthMacro, AuthStep, CookieData, StorageState};
use super::cdp::CdpClient;
use super::detector::detect_chromium_path;
use super::dom_script::{
    build_interact_script, EXTRACT_RECORDED_STEPS_SCRIPT, EXTRACT_TREE_SCRIPT, INJECT_RECORDER_SCRIPT,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserDebugState {
    pub url: String,
    pub title: String,
    pub elements: Vec<InteractiveElement>,
    pub console_errors: Vec<String>,
    pub network_errors: Vec<String>,
    #[serde(default)]
    pub tables: Vec<String>,
    #[serde(default)]
    pub notices: Vec<String>,
}

impl BrowserDebugState {
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();
        md.push_str(&format!("### 当前网页状态\n- **URL**: {}\n- **标题**: {}\n\n", self.url, self.title));

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
    _child: Arc<Mutex<Child>>,
    temp_dir: PathBuf,
    workspace: Arc<Mutex<Option<PathBuf>>>,
    cdp: Arc<CdpClient>,
    target_session_id: String,
    pub is_recording: Arc<AtomicBool>,
    storage_injected: Arc<AtomicBool>,
    replay_lock: Arc<Mutex<()>>,
}

impl BrowserSession {
    pub async fn launch(headless: bool, workspace: Option<PathBuf>) -> Result<Self, String> {
        let browser_bin = detect_chromium_path()?;
        let temp_dir = std::env::temp_dir().join(format!("harness_browser_{}", uuid::Uuid::new_v4()));
        let _ = tokio::fs::create_dir_all(&temp_dir).await;

        let mut cmd = tokio::process::Command::new(browser_bin);
        cmd.arg("--remote-debugging-port=0")
            .arg(format!("--user-data-dir={}", temp_dir.to_string_lossy()))
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-gpu")
            .arg("--disable-extensions")
            .arg("--disable-sync")
            .arg("--disable-background-networking")
            .arg("--disable-component-update")
            .arg("--disable-default-apps")
            .arg("--proxy-bypass-list=<-loopback>")
            .arg("--window-size=1280,900");

        if headless {
            cmd.arg("--headless=new");
        }

        cmd.arg("about:blank");

        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            if headless {
                cmd.creation_flags(CREATE_NO_WINDOW);
            }
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("启动浏览器进程失败: {e}"))?;

        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "无法获取浏览器 stderr".to_string())?;

        // 读取 stderr 获取 DevTools listening on ws://...
        let mut reader = BufReader::new(stderr).lines();
        let mut ws_url = None;

        let timeout = Duration::from_secs(15);
        let start = std::time::Instant::now();

        while start.elapsed() < timeout {
            if let Ok(Ok(Some(l))) = tokio::time::timeout(Duration::from_millis(500), reader.next_line()).await {
                if let Some(idx) = l.find("DevTools listening on ws://") {
                    let ws = l[idx + "DevTools listening on ".len()..].trim().to_string();
                    ws_url = Some(ws);
                    break;
                }
            }
        }

        let ws_url = ws_url.ok_or_else(|| "等待浏览器 DevTools WebSocket 地址超时".to_string())?;

        // 异步丢弃 stderr 剩余数据，防止挂起
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

        // 挂载到 Target 获得 sessionId
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

        // 激活所需领域
        let sid = Some(target_session_id.as_str());
        let _ = cdp.send("Page.enable", None, sid, Duration::from_secs(5)).await;
        let _ = cdp.send("Runtime.enable", None, sid, Duration::from_secs(5)).await;
        let _ = cdp.send("Network.enable", None, sid, Duration::from_secs(5)).await;
        let _ = cdp.send("DOM.enable", None, sid, Duration::from_secs(5)).await;

        // 激活当前标签页并设置视口，防止后台节流和 0x0 渲染
        let _ = cdp.send("Target.activateTarget", Some(json!({ "targetId": target_id })), None, Duration::from_secs(5)).await;
        let _ = cdp.send(
            "Emulation.setDeviceMetricsOverride",
            Some(json!({
                "width": 1280,
                "height": 900,
                "deviceScaleFactor": 1,
                "mobile": false
            })),
            sid,
            Duration::from_secs(5),
        ).await;

        Ok(Self {
            _child: Arc::new(Mutex::new(child)),
            temp_dir,
            workspace: Arc::new(Mutex::new(workspace)),
            cdp,
            target_session_id,
            is_recording: Arc::new(AtomicBool::new(false)),
            storage_injected: Arc::new(AtomicBool::new(false)),
            replay_lock: Arc::new(Mutex::new(())),
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

    pub async fn is_alive(&self) -> bool {
        if !self.cdp.is_alive() {
            return false;
        }
        let mut child = self._child.lock().await;
        match child.try_wait() {
            Ok(None) => true,
            _ => false,
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

        // 0. 日常自主测试：若工作区已固化 storage_state.json 且当前实例尚未注入，自动预注入已选好机构的 Cookies 与 LocalStorage
        if !self.storage_injected.load(Ordering::SeqCst) {
            let ws_opt = self.get_workspace().await;
            if let Some(ref ws) = ws_opt {
                if let Ok(Some(state)) = load_storage_state(ws) {
                    let _ = self.inject_storage_state(&state).await;
                    self.storage_injected.store(true, Ordering::SeqCst);
                }
            }
        }

        self.cdp
            .send("Page.navigate", Some(json!({ "url": normalized_url })), sid, Duration::from_secs(30))
            .await?;

        // 等待单页应用初始化加载 (Vue/React 挂载)
        tokio::time::sleep(Duration::from_millis(1500)).await;

        // 若处于录制模式，确保新页面自动载入录制探针
        if self.is_recording.load(Ordering::SeqCst) {
            let _ = self.cdp.send(
                "Runtime.evaluate",
                Some(json!({ "expression": INJECT_RECORDER_SCRIPT, "returnByValue": false })),
                sid,
                Duration::from_secs(5),
            ).await;
        }

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
                .filter(|l| l.contains("[Console.error]") || l.contains("[Uncaught Exception]"))
                .cloned()
                .collect()
        };

        let network_errors = {
            let net = self.cdp.network_errors.lock().await;
            net.clone()
        };

        Ok(BrowserDebugState {
            url,
            title,
            elements,
            console_errors,
            network_errors,
            tables,
            notices,
        })
    }

    pub async fn capture_screenshot(&self) -> Result<String, String> {
        let sid = Some(self.target_session_id.as_str());
        let res = self
            .cdp
            .send(
                "Page.captureScreenshot",
                Some(json!({ "format": "jpeg", "quality": 80 })),
                sid,
                Duration::from_secs(15),
            )
            .await?;

        res.get("data")
            .and_then(|d| d.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "截图失败: 未返回图像数据".to_string())
    }

    pub async fn user_click(&self, x: f64, y: f64) -> Result<(), String> {
        let sid = Some(self.target_session_id.as_str());
        let press = json!({
            "type": "mousePressed",
            "x": x,
            "y": y,
            "button": "left",
            "clickCount": 1
        });
        self.cdp.send("Input.dispatchMouseEvent", Some(press), sid, Duration::from_secs(5)).await?;
        let release = json!({
            "type": "mouseReleased",
            "x": x,
            "y": y,
            "button": "left"
        });
        self.cdp.send("Input.dispatchMouseEvent", Some(release), sid, Duration::from_secs(5)).await?;
        Ok(())
    }

    pub async fn user_type_text(&self, text: &str) -> Result<(), String> {
        let sid = Some(self.target_session_id.as_str());
        let params = json!({ "text": text });
        self.cdp.send("Input.insertText", Some(params), sid, Duration::from_secs(5)).await?;
        Ok(())
    }

    pub async fn capture_storage_state(&self) -> Result<StorageState, String> {
        let sid = Some(self.target_session_id.as_str());

        // 1. 获取全量 Cookies
        let cookies_res = self
            .cdp
            .send("Network.getCookies", None, sid, Duration::from_secs(10))
            .await?;

        let mut cookies = Vec::new();
        if let Some(arr) = cookies_res.get("cookies").and_then(|c| c.as_array()) {
            for item in arr {
                if let (Some(name), Some(value), Some(domain), Some(path)) = (
                    item.get("name").and_then(|n| n.as_str()),
                    item.get("value").and_then(|v| v.as_str()),
                    item.get("domain").and_then(|d| d.as_str()),
                    item.get("path").and_then(|p| p.as_str()),
                ) {
                    cookies.push(CookieData {
                        name: name.to_string(),
                        value: value.to_string(),
                        domain: domain.to_string(),
                        path: path.to_string(),
                        expires: item.get("expires").and_then(|e| e.as_f64()),
                        http_only: item.get("httpOnly").and_then(|h| h.as_bool()),
                        secure: item.get("secure").and_then(|s| s.as_bool()),
                        same_site: item.get("sameSite").and_then(|s| s.as_str()).map(|s| s.to_string()),
                    });
                }
            }
        }

        // 2. 提取 LocalStorage
        let ls_res = self
            .cdp
            .send(
                "Runtime.evaluate",
                Some(json!({
                    "expression": r#"(() => {
                        try {
                            const res = {};
                            for (let i = 0; i < localStorage.length; i++) {
                                const k = localStorage.key(i);
                                if (k) res[k] = localStorage.getItem(k);
                            }
                            return res;
                        } catch(e) {
                            return {};
                        }
                    })()"#,
                    "returnByValue": true
                })),
                sid,
                Duration::from_secs(10),
            )
            .await?;

        let mut local_storage = HashMap::new();
        if let Some(obj) = ls_res
            .get("result")
            .and_then(|r| r.get("value"))
            .and_then(|v| v.as_object())
        {
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    local_storage.insert(k.clone(), s.to_string());
                } else {
                    local_storage.insert(k.clone(), v.to_string());
                }
            }
        }

        // 3. 提取 SessionStorage
        let ss_res = self
            .cdp
            .send(
                "Runtime.evaluate",
                Some(json!({
                    "expression": r#"(() => {
                        try {
                            const res = {};
                            for (let i = 0; i < sessionStorage.length; i++) {
                                const k = sessionStorage.key(i);
                                if (k) res[k] = sessionStorage.getItem(k);
                            }
                            return res;
                        } catch(e) {
                            return {};
                        }
                    })()"#,
                    "returnByValue": true
                })),
                sid,
                Duration::from_secs(10),
            )
            .await?;

        let mut session_storage = HashMap::new();
        if let Some(obj) = ss_res
            .get("result")
            .and_then(|r| r.get("value"))
            .and_then(|v| v.as_object())
        {
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    session_storage.insert(k.clone(), s.to_string());
                } else {
                    session_storage.insert(k.clone(), v.to_string());
                }
            }
        }

        Ok(StorageState {
            updated_at: chrono::Utc::now().to_rfc3339(),
            cookies,
            local_storage,
            session_storage: if session_storage.is_empty() { None } else { Some(session_storage) },
        })
    }

    pub async fn inject_storage_state(&self, state: &StorageState) -> Result<(), String> {
        let sid = Some(self.target_session_id.as_str());

        // 1. 注入 Cookies
        let mut cdp_cookies = Vec::new();
        for c in &state.cookies {
            let mut c_val = json!({
                "name": c.name,
                "value": c.value,
                "domain": c.domain,
                "path": c.path,
            });
            if let Some(exp) = c.expires {
                c_val["expires"] = json!(exp);
            }
            if let Some(ho) = c.http_only {
                c_val["httpOnly"] = json!(ho);
            }
            if let Some(sec) = c.secure {
                c_val["secure"] = json!(sec);
            }
            if let Some(ref ss) = c.same_site {
                c_val["sameSite"] = json!(ss);
            }
            cdp_cookies.push(c_val);
        }
        if !cdp_cookies.is_empty() {
            let _ = self
                .cdp
                .send(
                    "Network.setCookies",
                    Some(json!({ "cookies": cdp_cookies })),
                    sid,
                    Duration::from_secs(10),
                )
                .await;
        }

        // 2. 注入 LocalStorage 与 SessionStorage (在每个新文档首行执行)
        let ls_json = serde_json::to_string(&state.local_storage).unwrap_or_else(|_| "{}".to_string());
        let ss_json = serde_json::to_string(&state.session_storage.clone().unwrap_or_default())
            .unwrap_or_else(|_| "{}".to_string());

        let script = format!(
            r#"(() => {{
                try {{
                    const ls = {ls_json};
                    for (const [k, v] of Object.entries(ls)) {{
                        try {{ localStorage.setItem(k, v); }} catch(e) {{}}
                    }}
                }} catch(e) {{}}
                try {{
                    const ss = {ss_json};
                    for (const [k, v] of Object.entries(ss)) {{
                        try {{ sessionStorage.setItem(k, v); }} catch(e) {{}}
                    }}
                }} catch(e) {{}}
            }})()"#
        );

        let _ = self
            .cdp
            .send(
                "Page.addScriptToEvaluateOnNewDocument",
                Some(json!({ "source": script })),
                sid,
                Duration::from_secs(5),
            )
            .await;

        let _ = self
            .cdp
            .send(
                "Runtime.evaluate",
                Some(json!({ "expression": script, "returnByValue": false })),
                sid,
                Duration::from_secs(5),
            )
            .await;

        Ok(())
    }

    pub async fn start_recording(&self) -> Result<(), String> {
        self.is_recording.store(true, Ordering::SeqCst);
        let sid = Some(self.target_session_id.as_str());

        // 清空之前的录制缓存
        let _ = self
            .cdp
            .send(
                "Runtime.evaluate",
                Some(json!({
                    "expression": "window.__harness_recorded_steps = []; window.__harness_recorder_installed = false;",
                    "returnByValue": false
                })),
                sid,
                Duration::from_secs(5),
            )
            .await;

        // 注册到后续导航
        let _ = self
            .cdp
            .send(
                "Page.addScriptToEvaluateOnNewDocument",
                Some(json!({ "source": INJECT_RECORDER_SCRIPT })),
                sid,
                Duration::from_secs(5),
            )
            .await;

        // 立即在当前页面注入监听器
        let _ = self
            .cdp
            .send(
                "Runtime.evaluate",
                Some(json!({ "expression": INJECT_RECORDER_SCRIPT, "returnByValue": false })),
                sid,
                Duration::from_secs(5),
            )
            .await;

        Ok(())
    }

    pub async fn get_recorded_steps(&self) -> Result<Vec<AuthStep>, String> {
        let sid = Some(self.target_session_id.as_str());
        let res = self
            .cdp
            .send(
                "Runtime.evaluate",
                Some(json!({
                    "expression": EXTRACT_RECORDED_STEPS_SCRIPT,
                    "returnByValue": true
                })),
                sid,
                Duration::from_secs(5),
            )
            .await?;

        let mut steps = Vec::new();
        if let Some(arr) = res.get("result").and_then(|r| r.get("value")).and_then(|v| v.as_array()) {
            for (idx, item) in arr.iter().enumerate() {
                let action = item.get("action").and_then(|a| a.as_str()).unwrap_or("click").to_string();
                let selector = item.get("selector").and_then(|s| s.as_str()).map(|s| s.to_string());
                let value = item.get("value").and_then(|v| v.as_str()).map(|s| s.to_string());
                let is_encrypted = item.get("is_encrypted").and_then(|b| b.as_bool());
                let desc = item.get("desc").and_then(|d| d.as_str()).map(|d| d.to_string());
                let order = item.get("order").and_then(|o| o.as_u64()).map(|o| o as usize).unwrap_or(idx + 1);

                steps.push(AuthStep {
                    order,
                    action,
                    selector,
                    value,
                    is_encrypted,
                    desc,
                    ms: None,
                    exclude: None,
                });
            }
        }
        Ok(steps)
    }

    pub async fn stop_recording(&self) -> Result<(), String> {
        self.is_recording.store(false, Ordering::SeqCst);
        Ok(())
    }

    pub async fn replay_auth_macro(&self, macro_data: &AuthMacro) -> Result<(), String> {
        let sid = Some(self.target_session_id.as_str());

        // 1. 导航至登录页面
        self.navigate(&macro_data.login_url).await?;
        tokio::time::sleep(Duration::from_millis(1000)).await;

        // 2. 依次重放动作
        for step in &macro_data.steps {
            match step.action.as_str() {
                "wait" => {
                    let ms = step.ms.unwrap_or(1000);
                    tokio::time::sleep(Duration::from_millis(ms)).await;
                }
                "wait_for_url" => {
                    let exclude = step.exclude.as_deref().unwrap_or("#/login");
                    let start = std::time::Instant::now();
                    let timeout = Duration::from_secs(10);
                    while start.elapsed() < timeout {
                        let res = self.cdp.send(
                            "Runtime.evaluate",
                            Some(json!({ "expression": "window.location.href", "returnByValue": true })),
                            sid,
                            Duration::from_secs(5),
                        ).await;
                        if let Ok(val) = res {
                            let curr_url = val.get("result").and_then(|r| r.get("value")).and_then(|v| v.as_str()).unwrap_or("");
                            if !curr_url.is_empty() && !curr_url.contains(exclude) {
                                break;
                            }
                        }
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                }
                "click" => {
                    if let Some(ref target) = step.selector {
                        self.interact("click", target, None).await?;
                    }
                }
                "type" => {
                    if let Some(ref target) = step.selector {
                        self.interact("type", target, step.value.as_deref()).await?;
                    }
                }
                "press_enter" => {
                    let target = step.selector.as_deref().unwrap_or("body");
                    self.interact("press_enter", target, None).await?;
                }
                _ => {}
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }

        tokio::time::sleep(Duration::from_millis(1200)).await;
        Ok(())
    }

    pub async fn check_and_heal_auth(&self, master_key: &[u8; 32]) -> Result<bool, String> {
        let expired = self.cdp.auth_expired.swap(false, Ordering::SeqCst);
        let is_login_page = {
            let res = self.cdp.send(
                "Runtime.evaluate",
                Some(json!({ "expression": "window.location.href", "returnByValue": true })),
                Some(self.target_session_id.as_str()),
                Duration::from_secs(5),
            ).await;
            res.ok()
                .and_then(|v| v.get("result").and_then(|r| r.get("value")).and_then(|u| u.as_str()).map(|s| s.contains("/login") || s.contains("#/login")))
                .unwrap_or(false)
        };

        if !expired && !is_login_page {
            return Ok(false);
        }

        let ws_opt = self.get_workspace().await;
        let ws = match ws_opt {
            Some(w) => w,
            None => return Ok(false),
        };

        let macro_data = match super::auth::load_auth_macro(&ws, master_key)? {
            Some(m) => m,
            None => return Ok(false),
        };

        let _guard = self.replay_lock.lock().await;
        self.replay_auth_macro(&macro_data).await?;

        // 提取并更新 storage_state.json
        if let Ok(new_state) = self.capture_storage_state().await {
            let _ = save_storage_state(&ws, &new_state);
        }
        Ok(true)
    }

    pub async fn close(&self) {
        let mut child = self._child.lock().await;
        let _ = child.kill().await;
        let _ = tokio::fs::remove_dir_all(&self.temp_dir).await;
    }
}
