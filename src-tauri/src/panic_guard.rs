//! 全局 panic 捕获与落盘。
//!
//! 背景：release 构建开启了 `strip = true` + `lto = true`，一旦发生 panic，
//! 用户侧既看不到控制台也没有符号信息，表现为「无提示闪退」，线上无法定位。
//! 本模块安装全局 panic hook，把 panic 的位置、消息与线程名追加写入
//! 数据目录下的 `crash.log`，将静默闪退转化为可追溯事件。
//!
//! 设计约束：
//! 1. panic 处理路径自身绝不允许再 panic —— 所有 IO 失败一律静默吞掉；
//! 2. 不引入额外依赖，仅用 std + chrono；
//! 3. 日志路径在数据目录解析完成后注入（自定义数据目录场景下才能拿到正确位置）。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// crash.log 的绝对路径；启动时由 `set_log_dir` 注入（此时才知道生效的数据目录）。
static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// 单个日志文件的体积上限，超过后轮转为 crash.log.1，避免长期运行无限增长。
const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;

/// 指定 panic 日志的落盘目录。需在数据目录解析完成后调用。
/// 重复调用只有首次生效（OnceLock 语义），保证全局只绑定一个路径。
pub fn set_log_dir(dir: &Path) {
    let _ = LOG_PATH.set(dir.join("crash.log"));
}

/// 安装全局 panic hook。应在进程启动最早期调用，越早覆盖越全。
pub fn install() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // 1. 保留默认行为（stderr 输出），不降低开发期的可观测性
        default_hook(info);

        // 2. 组装人可读的崩溃记录
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".to_string());

        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string panic payload>".to_string()
        };

        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("<unnamed>").to_string();

        let record = format!(
            "[{}] PANIC thread={} at {}\n  message: {}\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
            thread_name,
            location,
            payload
        );

        write_record(&record);
    }));
}

/// 尽力写入；任何失败都必须静默吞掉——panic 处理路径不允许二次 panic。
fn write_record(record: &str) {
    let Some(path) = LOG_PATH.get() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    rotate_if_needed(path);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = f.write_all(record.as_bytes());
        let _ = f.flush();
    }
}

fn rotate_if_needed(path: &Path) {
    let too_big = std::fs::metadata(path)
        .map(|m| m.len() > MAX_LOG_BYTES)
        .unwrap_or(false);
    if too_big {
        // crash.log -> crash.log.1（覆盖上一份归档，保留最近两代）
        let _ = std::fs::rename(path, path.with_extension("log.1"));
    }
}
