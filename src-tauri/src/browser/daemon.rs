use super::cdp::CdpClient;
use super::detector::detect_chromium_path;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Child;
use tokio::sync::{Mutex, RwLock};

#[cfg(windows)]
fn is_pid_alive(pid: u32) -> bool {
    extern "system" {
        fn OpenProcess(dwDesiredAccess: u32, bInheritHandle: i32, dwProcessId: u32) -> *mut std::ffi::c_void;
        fn CloseHandle(hObject: *mut std::ffi::c_void) -> i32;
        fn GetExitCodeProcess(hProcess: *mut std::ffi::c_void, lpExitCode: *mut u32) -> i32;
    }
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut exit_code = 0u32;
        let ok = GetExitCodeProcess(handle, &mut exit_code);
        CloseHandle(handle);
        ok != 0 && exit_code == STILL_ACTIVE
    }
}

#[cfg(not(windows))]
fn is_pid_alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

pub struct BrowserDaemon {
    child: Arc<Mutex<Option<Child>>>,
    cdp: Arc<RwLock<Option<Arc<CdpClient>>>>,
    ws_url: Arc<RwLock<Option<String>>>,
    daemon_profile_dir: Arc<RwLock<PathBuf>>,
}

static GLOBAL_BROWSER_DAEMON: OnceLock<BrowserDaemon> = OnceLock::new();

pub fn global_browser_daemon() -> &'static BrowserDaemon {
    GLOBAL_BROWSER_DAEMON.get_or_init(BrowserDaemon::new)
}

impl Default for BrowserDaemon {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserDaemon {
    pub fn new() -> Self {
        let my_pid = std::process::id();
        let p = std::env::temp_dir().join(format!("harness_browser_daemon_{}", my_pid));

        // 后台异步扫描并清理以往异常崩溃遗留的旧临时目录
        tokio::spawn(async move {
            Self::cleanup_stale_profiles(my_pid).await;
        });

        Self {
            child: Arc::new(Mutex::new(None)),
            cdp: Arc::new(RwLock::new(None)),
            ws_url: Arc::new(RwLock::new(None)),
            daemon_profile_dir: Arc::new(RwLock::new(p)),
        }
    }

    /// 清理以往因崩溃或强制退出留下的无主守护目录
    async fn cleanup_stale_profiles(current_pid: u32) {
        let temp_dir = std::env::temp_dir();
        // 尝试清理以往遗留的无 PID 后缀的旧全局目录（若当前未被锁定）
        let legacy_dir = temp_dir.join("harness_browser_daemon");
        if legacy_dir.exists() {
            let _ = tokio::fs::remove_dir_all(&legacy_dir).await;
        }

        let mut entries = match tokio::fs::read_dir(&temp_dir).await {
            Ok(e) => e,
            Err(_) => return,
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let file_name = entry.file_name().to_string_lossy().to_string();
            if file_name.starts_with("harness_browser_daemon_") {
                let remainder = &file_name["harness_browser_daemon_".len()..];
                let pid_str = remainder.split('_').next().unwrap_or("");
                if let Ok(pid) = pid_str.parse::<u32>() {
                    if pid != current_pid && !is_pid_alive(pid) {
                        let _ = tokio::fs::remove_dir_all(entry.path()).await;
                    }
                }
            }
        }
    }

    pub async fn is_alive(&self) -> bool {
        let cdp_opt = self.cdp.read().await.clone();
        if let Some(cdp) = cdp_opt {
            if !cdp.is_alive() {
                return false;
            }
        } else {
            return false;
        }

        let mut child_guard = self.child.lock().await;
        if let Some(ref mut child) = *child_guard {
            match child.try_wait() {
                Ok(None) => true,
                _ => false,
            }
        } else {
            false
        }
    }

    /// 底层实际拉起 Chromium 子进程并等待 DevTools 端口就绪
    async fn try_spawn_daemon(
        browser_bin: &Path,
        profile_dir: &Path,
    ) -> Result<(Child, String), String> {
        let _ = tokio::fs::create_dir_all(profile_dir).await;

        // 清理旧锁文件与 ActivePort
        let lock_file = profile_dir.join("SingletonLock");
        if lock_file.exists() {
            let _ = tokio::fs::remove_file(&lock_file).await;
        }
        #[cfg(windows)]
        {
            let lock_win = profile_dir.join("lockfile");
            if lock_win.exists() {
                let _ = tokio::fs::remove_file(&lock_win).await;
            }
        }
        let active_port_file = profile_dir.join("DevToolsActivePort");
        if active_port_file.exists() {
            let _ = tokio::fs::remove_file(&active_port_file).await;
        }

        let mut cmd = tokio::process::Command::new(browser_bin);
        cmd.arg("--remote-debugging-port=0")
            .arg(format!("--user-data-dir={}", profile_dir.to_string_lossy()))
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-gpu")
            .arg("--disable-extensions")
            .arg("--disable-sync")
            .arg("--disable-background-networking")
            .arg("--disable-component-update")
            .arg("--disable-default-apps")
            .arg("--proxy-bypass-list=<-loopback>")
            .arg("--window-size=1440,960")
            .arg("--headless=new")
            .arg("about:blank");

        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("启动浏览器守护进程失败: {e}"))?;

        #[cfg(windows)]
        if let Some(pid) = child.id() {
            crate::tools::assign_pid_to_job(pid);
        }

        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "无法获取浏览器守护进程 stderr".to_string())?;

        let mut reader = BufReader::new(stderr).lines();
        let mut ws_url = None;
        let mut accumulated_stderr = Vec::new();

        let timeout = Duration::from_secs(12);
        let start = std::time::Instant::now();

        while start.elapsed() < timeout {
            // 1. 检查子进程是否已提前异常退出
            if let Ok(Some(status)) = child.try_wait() {
                let err_detail = accumulated_stderr.join("\n");
                return Err(format!(
                    "EXIT_CODE:{:?}|{}",
                    status.code(),
                    if err_detail.is_empty() { "无 stderr 输出" } else { &err_detail }
                ));
            }

            // 2. 双轨检测：优先检测 DevToolsActivePort 文件
            if active_port_file.exists() {
                if let Ok(content) = std::fs::read_to_string(&active_port_file) {
                    let lines: Vec<&str> = content.lines().collect();
                    if lines.len() >= 2 {
                        let port = lines[0].trim();
                        let path = lines[1].trim();
                        if !port.is_empty() && !path.is_empty() {
                            let url_str = format!("ws://127.0.0.1:{port}{path}");
                            ws_url = Some(url_str);
                            break;
                        }
                    }
                }
            }

            // 3. 从 stderr 流中读取
            if let Ok(Ok(Some(line))) = tokio::time::timeout(Duration::from_millis(100), reader.next_line()).await {
                if let Some(idx) = line.find("DevTools listening on ws://") {
                    let ws = line[idx + "DevTools listening on ".len()..].trim().to_string();
                    ws_url = Some(ws);
                    break;
                }
                accumulated_stderr.push(line);
            }
        }

        let final_ws_url = match ws_url {
            Some(u) => u,
            None => {
                let err_detail = accumulated_stderr.join("\n");
                #[cfg(windows)]
                if let Some(pid) = child.id() {
                    crate::tools::kill_process_tree(pid);
                }
                let _ = child.kill().await;
                return Err(format!(
                    "等待浏览器 DevTools WebSocket 地址超时 (12s)！\n错误详情:\n{}",
                    if err_detail.is_empty() { "无 stderr 输出" } else { &err_detail }
                ));
            }
        };

        // 丢弃后续 stderr 防止管道阻塞
        tokio::spawn(async move {
            while let Ok(Some(_)) = reader.next_line().await {}
        });

        Ok((child, final_ws_url))
    }

    /// 确保常驻 Chromium 守护进程存活并返回底层 root CDP 客户端
    pub async fn ensure_running(&self) -> Result<Arc<CdpClient>, String> {
        if self.is_alive().await {
            let cdp_opt = self.cdp.read().await.clone();
            if let Some(cdp) = cdp_opt {
                return Ok(cdp);
            }
        }

        // 清理旧状态与可能的僵尸进程
        self.stop().await;

        let browser_bin = detect_chromium_path()?;
        let mut current_profile = self.daemon_profile_dir.read().await.clone();

        // 尝试拉起守护进程
        // 若遇到 ExitCode 21 (RESULT_CODE_PROFILE_IN_USE，即配置文件被旧残留进程或其它实例锁定)，
        // 则自动自愈切换至全新独立临时目录重试，彻底杜绝多工具冲突与退出报错。
        let spawn_res = match Self::try_spawn_daemon(&browser_bin, &current_profile).await {
            Ok(pair) => Ok(pair),
            Err(e) if e.contains("EXIT_CODE:Some(21)") => {
                let fallback_dir = std::env::temp_dir().join(format!(
                    "harness_browser_daemon_{}_{}",
                    std::process::id(),
                    uuid::Uuid::new_v4().simple()
                ));
                current_profile = fallback_dir.clone();
                *self.daemon_profile_dir.write().await = fallback_dir;
                Self::try_spawn_daemon(&browser_bin, &current_profile).await
            }
            Err(e) => Err(e),
        };

        let (child, final_ws_url) = spawn_res.map_err(|e| {
            if let Some(rest) = e.strip_prefix("EXIT_CODE:") {
                let parts: Vec<&str> = rest.splitn(2, '|').collect();
                format!(
                    "浏览器守护进程提前退出 (ExitCode: {})，启动失败！\n输出日志:\n{}",
                    parts.get(0).unwrap_or(&"未知"),
                    parts.get(1).unwrap_or(&"无 stderr 输出")
                )
            } else {
                e
            }
        })?;

        let cdp = Arc::new(CdpClient::connect(&final_ws_url).await?);

        {
            let mut c_guard = self.child.lock().await;
            *c_guard = Some(child);
        }
        {
            let mut w_guard = self.ws_url.write().await;
            *w_guard = Some(final_ws_url);
        }
        {
            let mut cdp_guard = self.cdp.write().await;
            *cdp_guard = Some(cdp.clone());
        }

        Ok(cdp)
    }

    /// 在受管 Chromium 中创建全新的隔离上下文 (BrowserContext)
    pub async fn create_context(&self) -> Result<String, String> {
        let cdp = self.ensure_running().await?;
        let res = cdp
            .send("Target.createBrowserContext", None, None, Duration::from_secs(10))
            .await?;

        let context_id = res
            .get("browserContextId")
            .and_then(|c| c.as_str())
            .ok_or_else(|| "Target.createBrowserContext 未返回 browserContextId".to_string())?
            .to_string();

        Ok(context_id)
    }

    /// 销毁指定的 BrowserContext
    pub async fn dispose_context(&self, context_id: &str) -> Result<(), String> {
        if let Ok(cdp) = self.ensure_running().await {
            let _ = cdp
                .send(
                    "Target.disposeBrowserContext",
                    Some(json!({ "browserContextId": context_id })),
                    None,
                    Duration::from_secs(5),
                )
                .await;
        }
        Ok(())
    }

    /// 在指定上下文（或默认上下文）中新建标签页 (Page Target)
    pub async fn create_target_in_context(
        &self,
        context_id: Option<&str>,
        url: &str,
    ) -> Result<String, String> {
        let cdp = self.ensure_running().await?;
        let mut params = json!({ "url": url });
        if let Some(cid) = context_id {
            params["browserContextId"] = json!(cid);
        }

        let res = cdp
            .send("Target.createTarget", Some(params), None, Duration::from_secs(10))
            .await?;

        let target_id = res
            .get("targetId")
            .and_then(|t| t.as_str())
            .ok_or_else(|| "Target.createTarget 未返回 targetId".to_string())?
            .to_string();

        Ok(target_id)
    }

    /// 挂载到 Page Target 获得 sessionId
    pub async fn attach_to_target(&self, target_id: &str) -> Result<String, String> {
        let cdp = self.ensure_running().await?;
        let res = cdp
            .send(
                "Target.attachToTarget",
                Some(json!({ "targetId": target_id, "flatten": true })),
                None,
                Duration::from_secs(10),
            )
            .await?;

        let session_id = res
            .get("sessionId")
            .and_then(|s| s.as_str())
            .ok_or_else(|| "Target.attachToTarget 未返回 sessionId".to_string())?
            .to_string();

        Ok(session_id)
    }

    /// 关闭标签页 Target
    pub async fn close_target(&self, target_id: &str) -> Result<(), String> {
        if let Ok(cdp) = self.ensure_running().await {
            let _ = cdp
                .send("Target.closeTarget", Some(json!({ "targetId": target_id })), None, Duration::from_secs(5))
                .await;
        }
        Ok(())
    }

    pub async fn get_ws_url(&self) -> Option<String> {
        self.ws_url.read().await.clone()
    }

    pub async fn stop(&self) {
        let mut child_guard = self.child.lock().await;
        if let Some(mut child) = child_guard.take() {
            #[cfg(windows)]
            if let Some(pid) = child.id() {
                crate::tools::kill_process_tree(pid);
            }
            let _ = child.kill().await;
        }
        {
            let mut cdp_guard = self.cdp.write().await;
            *cdp_guard = None;
        }
        {
            let mut w_guard = self.ws_url.write().await;
            *w_guard = None;
        }
        let profile_dir = self.daemon_profile_dir.read().await.clone();
        let _ = tokio::fs::remove_dir_all(&profile_dir).await;
    }
}
