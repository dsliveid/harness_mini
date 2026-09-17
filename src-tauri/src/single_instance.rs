//! 基于路径作用域的单实例控制与多开隔离模块。
//!
//! 1. 获取当前程序规范化父目录哈希作为实例标识；
//! 2. 尝试连接命名管道并唤醒已有窗口（若同目录已启动则退出）；
//! 3. 隔离 WebView2 用户数据目录，避免不同目录实例相互踩踏导致渲染崩溃白屏；
//! 4. 在首实例中开启命名管道后台监听，响应后续同目录启动并激活窗口。

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// 计算指定目录的规范化唯一哈希字符串（忽略大小写与结尾斜杠）
pub fn dir_to_hash(dir: &Path) -> String {
    let norm = dir.to_string_lossy().replace('/', "\\").to_lowercase();
    let norm_clean = norm.trim_end_matches('\\');

    let mut hasher = DefaultHasher::new();
    norm_clean.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// 获取当前程序所在目录的规范化唯一哈希字符串（忽略大小写与结尾斜杠）
pub fn get_app_dir_hash() -> String {
    let exe_path = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    let app_dir = exe_path
        .parent()
        .map(|p| p.canonicalize().unwrap_or_else(|_| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));

    dir_to_hash(&app_dir)
}

/// 尝试向已存在的同目录实例发送唤醒指令
///
/// 若成功连接并发送 "WAKEUP\n"，返回 `true`，调用者应直接退出进程；
/// 若不存在已有实例或连接失败，返回 `false`，由调用者作为首个实例继续启动。
pub fn try_wakeup_existing_instance(pipe_name: &str) -> bool {
    #[cfg(windows)]
    {
        use std::fs::OpenOptions;
        use std::io::Write;

        // 尝试打开已存在的命名管道（最多重试 3 次，防止刚好在两轮监听切换中）
        for _ in 0..3 {
            match OpenOptions::new().write(true).open(pipe_name) {
                Ok(mut pipe) => {
                    let _ = pipe.write_all(b"WAKEUP\n");
                    let _ = pipe.flush();
                    return true;
                }
                Err(e) => {
                    // Windows 错误码 231 为 ERROR_PIPE_BUSY，等待极短时间重试
                    if e.raw_os_error() == Some(231) {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                        continue;
                    }
                    break;
                }
            }
        }
        false
    }
    #[cfg(not(windows))]
    {
        let _ = pipe_name;
        false
    }
}

/// 基于目录哈希为当前进程动态隔离 WebView2 用户数据目录
///
/// 避免不同目录启动的多个实例争抢同一个 EBWebView 目录导致渲染进程崩溃（界面消失变白屏）。
pub fn setup_webview_isolation(dir_hash: &str) {
    #[cfg(windows)]
    {
        if std::env::var_os("WEBVIEW2_USER_DATA_FOLDER").is_none() {
            let base_dir = std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            let isolated_path = base_dir
                .join("com.harness.mini")
                .join("instances")
                .join(format!("ebwebview_{}", dir_hash));
            std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", isolated_path);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = dir_hash;
    }
}

/// 启动后台命名管道监听服务端
///
/// 当同目录下的新进程启动并向该管道发送唤醒信息时，将主窗口从最小化恢复并置顶聚焦。
pub fn start_pipe_listener(app: tauri::AppHandle, pipe_name: String) {
    #[cfg(windows)]
    {
        tauri::async_runtime::spawn(async move {
            use tokio::io::AsyncReadExt;
            use tokio::net::windows::named_pipe::ServerOptions;

            let mut is_first = true;
            loop {
                let server = match ServerOptions::new()
                    .first_pipe_instance(is_first)
                    .create(&pipe_name)
                {
                    Ok(s) => {
                        is_first = false;
                        s
                    }
                    Err(err) => {
                        // 若创建失败，略作退避后重试
                        eprintln!("[single_instance] 创建管道实例失败: {err}");
                        tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                        continue;
                    }
                };

                if server.connect().await.is_ok() {
                    let mut stream = server;
                    let mut buf = [0u8; 64];
                    if let Ok(n) = stream.read(&mut buf).await {
                        let msg = String::from_utf8_lossy(&buf[..n]);
                        if msg.starts_with("WAKEUP") {
                            crate::show_main_window(&app);
                        }
                    }
                }
            }
        });
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        let _ = pipe_name;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dir_to_hash_case_and_trailing_slashes() {
        let p1 = Path::new("D:\\Software\\HarnessMini");
        let p2 = Path::new("d:\\software\\harnessmini\\");
        let p3 = Path::new("D:/software/harnessmini");
        let p4 = Path::new("D:\\Software\\OtherApp");

        let h1 = dir_to_hash(p1);
        let h2 = dir_to_hash(p2);
        let h3 = dir_to_hash(p3);
        let h4 = dir_to_hash(p4);

        assert_eq!(h1, h2, "Trailing slash and lower/upper case should yield identical hash");
        assert_eq!(h1, h3, "Forward and backward slashes should yield identical hash");
        assert_ne!(h1, h4, "Different directories must produce distinct hashes");
    }

    #[test]
    fn test_setup_webview_isolation_env_var() {
        let test_hash = "abc123456789def0";
        setup_webview_isolation(test_hash);

        #[cfg(windows)]
        {
            let val = std::env::var("WEBVIEW2_USER_DATA_FOLDER").unwrap();
            assert!(val.contains(test_hash));
            assert!(val.contains("ebwebview_"));
        }
    }
}
