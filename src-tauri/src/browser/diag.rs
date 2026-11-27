//! 浏览器模块诊断日志（临时排障用，默认开启轻量记录）
//!
//! 目的：浏览器启动链路跨多个层级（前端 command → BrowserManager → BrowserDaemon →
//! Chromium 子进程 → CDP 握手），任一层静默失败都会表现为「一直显示启动中」。
//! 这里提供一条不依赖 tracing 框架的最简落盘日志，确保任何环境下都能拿到现场。
//!
//! 落盘位置：与崩溃日志同源的数据目录下 `browser_debug.log`（2MB 轮转）。
//! 若数据目录不可写，自动降级到可写探测目录。

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

static LOG_PATH: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

fn slot() -> &'static Mutex<Option<PathBuf>> {
    LOG_PATH.get_or_init(|| Mutex::new(None))
}

/// 绑定日志文件路径（通常在启动早期调用一次）
pub fn set_log_dir(dir: &std::path::Path) {
    let mut guard = match slot().lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    let target = dir.join("browser_debug.log");
    if let Some(parent) = target.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    *guard = Some(target);
}

fn resolve_path() -> Option<PathBuf> {
    // 1. 显式绑定的路径
    {
        let guard = match slot().lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        if let Some(p) = guard.as_ref() {
            return Some(p.clone());
        }
    }
    // 2. 降级：可写探测目录（保证低完整性级别下也能落盘）
    let dir = super::fs_probe::resolve_writable_dir(None, "harness_browser_logs");
    Some(dir.join("browser_debug.log"))
}

/// 记录一条诊断日志
pub fn log(stage: &str, msg: impl AsRef<str>) {
    let Some(path) = resolve_path() else {
        return;
    };

    // 2MB 轮转
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > 2 * 1024 * 1024 {
            let backup = path.with_extension("log.1");
            let _ = std::fs::rename(&path, backup);
        }
    }

    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let line = format!("[{ts}] [{stage}] {}\n", msg.as_ref());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(line.as_bytes());
    }
}

/// 返回当前诊断日志路径（供错误信息中提示用户）
pub fn log_path_hint() -> String {
    resolve_path()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| "(不可用)".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_writes_to_writable_dir() {
        let dir = super::super::fs_probe::resolve_writable_dir(None, "harness_browser_logs_test");
        let _ = std::fs::create_dir_all(&dir);
        set_log_dir(&dir);
        log("test", "hello-diagnostic");
        let p = dir.join("browser_debug.log");
        assert!(p.exists(), "诊断日志应已落盘: {p:?}");
        let content = std::fs::read_to_string(&p).unwrap();
        assert!(content.contains("hello-diagnostic"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
