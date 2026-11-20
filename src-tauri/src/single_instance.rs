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

/// 真写探针：确认目录能否创建并写入文件
///
/// WebView2 创建浏览器进程前必须能写用户数据目录，否则 `CreateCoreWebView2Environment`
/// 会静默失败（仅打印 "can't open file for write !"），表现为进程存活、无渲染子进程、
/// 窗口内容空白。Windows 强制完整性控制（MIC）下，低完整性级别（Low IL）进程
/// 无法写入中完整性（Medium IL）的 `%LOCALAPPDATA%`，因此不能凭假设，必须实测。
pub fn probe_writable(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".harness_write_probe");
    match std::fs::write(&probe, b"probe") {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// 按候选链挑选 WebView2 用户数据目录，并把结果写入环境变量
///
/// 候选顺序（逐级回退，第一个可写者胜出）：
/// 1. 进程启动时已存在的 `WEBVIEW2_USER_DATA_FOLDER`（尊重外部显式配置）
/// 2. `%LOCALAPPDATA%\com.harness.mini\instances\ebwebview_<hash>`（常规路径，多开隔离）
/// 3. `<exe 同级>\webview_cache_<hash>`（受限环境下 exe 所在目录通常可写）
/// 4. `<系统临时目录>\com.harness.mini\ebwebview_<hash>`（最后兜底）
///
/// 返回值：最终生效的目录。即使全部不可写也返回首选路径，由 WebView2 自行报错，
/// 避免调用方拿到 None 后陷入难以定位的空白窗口。
pub fn setup_webview_isolation(dir_hash: &str) -> PathBuf {
    #[cfg(windows)]
    {
        // 候选 1：外部已显式指定时优先尊重（例如 CI 或用户自定义缓存目录）
        if let Some(existing) = std::env::var_os("WEBVIEW2_USER_DATA_FOLDER") {
            let path = PathBuf::from(existing);
            if probe_writable(&path) {
                return path;
            }
            eprintln!(
                "[single_instance][WARN] 继承的 WEBVIEW2_USER_DATA_FOLDER 不可写，改用隔离目录: {}",
                path.display()
            );
        }

        // 候选 2：常规隔离路径
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(base) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            candidates.push(
                base.join("com.harness.mini")
                    .join("instances")
                    .join(format!("ebwebview_{dir_hash}")),
            );
        }

        // 候选 3：exe 同级目录（受限环境下该目录通常仍可写）
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                candidates.push(parent.join(format!("webview_cache_{dir_hash}")));
            }
        }

        // 候选 4：系统临时目录兜底
        candidates.push(
            std::env::temp_dir()
                .join("com.harness.mini")
                .join(format!("ebwebview_{dir_hash}")),
        );

        let fallback = candidates.last().cloned().unwrap_or_default();
        for (idx, candidate) in candidates.iter().enumerate() {
            if probe_writable(candidate) {
                if idx > 0 {
                    eprintln!(
                        "[single_instance][WARN] 首选 WebView2 用户目录不可写（常见原因：进程完整性级别被降级，无法写 %LOCALAPPDATA%），已回退至: {}",
                        candidate.display()
                    );
                }
                std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", candidate);
                return candidate.clone();
            }
            eprintln!(
                "[single_instance][WARN] WebView2 候选目录不可写: {}",
                candidate.display()
            );
        }

        // 全部不可写：仍设为首选，让 WebView2 自己报错，但留下明确痕迹
        eprintln!(
            "[single_instance][ERROR] 所有 WebView2 候选目录均不可写，窗口可能空白；最后尝试: {}",
            fallback.display()
        );
        std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", &fallback);
        fallback
    }
    #[cfg(not(windows))]
    {
        let _ = dir_hash;
        std::env::temp_dir()
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
        #[cfg(windows)]
        {
            let test_hash = "abc123456789def0";
            // 该测试必须环境无关：宿主进程可能已设置 WEBVIEW2_USER_DATA_FOLDER，
            // 新实现会先验证其可写性，不可写则回退。这里先清除继承的同名变量，测完再还原。
            let saved = std::env::var_os("WEBVIEW2_USER_DATA_FOLDER");
            std::env::remove_var("WEBVIEW2_USER_DATA_FOLDER");

            let chosen = setup_webview_isolation(test_hash);

            // 返回值与环境变量必须一致，且都带上本次哈希，保证多实例隔离不串味
            let val = std::env::var("WEBVIEW2_USER_DATA_FOLDER").unwrap();
            assert_eq!(chosen.to_string_lossy(), val);
            assert!(val.contains(test_hash), "隔离目录名必须包含实例哈希: {val}");
            // 常规路径为 ebwebview_<hash>，受限环境回退为 webview_cache_<hash>
            assert!(
                val.contains("ebwebview_") || val.contains("webview_cache_"),
                "隔离目录命名不符合预期: {val}"
            );
            // 选中的目录必须真的可写（这正是本改造的核心保证）
            assert!(probe_writable(&chosen), "选中的目录必须可写: {}", chosen.display());

            match saved {
                Some(v) => std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", v),
                None => std::env::remove_var("WEBVIEW2_USER_DATA_FOLDER"),
            }
        }
        #[cfg(not(windows))]
        {
            setup_webview_isolation("abc123456789def0");
        }
    }

    #[test]
    fn test_probe_writable_detects_writable_and_unwritable() {
        // 注意：本测试不假设 %TEMP% 一定可写。受限环境下（进程完整性级别被降级）
        // 连 %TEMP% 都会被拒绝，此时探针必须如实返回 false —— 那同样是正确行为。
        // 核心保证是：probe_writable 的返回值必须与「真写一次」的实际结果一致。
        let tmp = std::env::temp_dir().join("harness_probe_test_dir");
        let reported = probe_writable(&tmp);
        let actually_writable = std::fs::write(tmp.join(".verify"), b"x").is_ok();
        assert_eq!(
            reported, actually_writable,
            "探针返回值与实际可写性不一致（目录: {}）",
            tmp.display()
        );
        let _ = std::fs::remove_file(tmp.join(".verify"));
        let _ = std::fs::remove_dir_all(&tmp);

        // 深层嵌套：可创建则必须可写，不可创建则必须报告不可写（一致性同样成立）
        let nested = std::env::temp_dir().join("harness_probe_test_dir/a/b/c");
        let reported_deep = probe_writable(&nested);
        if reported_deep {
            assert!(
                std::fs::write(nested.join(".verify"), b"x").is_ok(),
                "报告可写但实际写失败: {}",
                nested.display()
            );
        }
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join("harness_probe_test_dir"));
    }
}
