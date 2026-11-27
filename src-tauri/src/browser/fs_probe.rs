//! 可写临时目录探测
//!
//! 背景（2026-11-24 线上事故定案）：
//! 本程序在某些运行方式下（WebView2 沙箱、受管终端、部分安全软件介入场景）会以
//! **低完整性级别（Low IL）** 进程运行，此时 `%LOCALAPPDATA%\Temp` 不可写。
//! Chromium 启动时会在其 user-data-dir 里创建 SingletonLock，一旦写入被拒就会：
//!
//! ```text
//! CreateFile ...\Microsoft\Edge\User Data\Crashpad\throttle_store.dat: 拒绝访问。(0x5)
//! Lock file can not be created! Error code: 5
//! Failed to create a ProcessSingleton for your profile directory.
//! → 进程立即以 ExitCode 21 退出
//! ```
//!
//! 表现为「浏览器一直显示启动中」：daemon 反复以 ExitCode 21 退出并触发自愈重试，
//! 而上层若缺超时保护就会永久挂起。
//!
//! 因此不能硬编码 `std::env::temp_dir()`，必须**实际探测可写性**后再选用目录。

use std::path::PathBuf;

/// 探测一个真正可写的目录（写入并删除探针文件验证）
pub fn is_writable(dir: &std::path::Path) -> bool {
    if !dir.exists() {
        return false;
    }
    let probe = dir.join(format!(".harness_write_probe_{}", uuid::Uuid::new_v4().simple()));
    match std::fs::write(&probe, b"ok") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// 尝试创建目录并确认可写
fn try_prepare(dir: &PathBuf) -> bool {
    if !dir.exists() && std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    is_writable(dir)
}

/// 解析出一个可用于浏览器 profile / 临时文件的**可写**目录
///
/// 候选顺序（按优先级）：
/// 1. 显式传入的 `preferred`（如数据目录 / 工作区下的 .harness/tmp）
/// 2. 系统 TEMP 下的子目录
/// 3. 当前进程工作目录下的 `.harness/tmp`
/// 4. 用户主目录下的 `.harness/tmp`
///
/// 全部失败时返回最后一次尝试的路径（由调用方在创建时得到明确错误）。
pub fn resolve_writable_dir(preferred: Option<PathBuf>, sub: &str) -> PathBuf {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Some(p) = preferred.clone() {
        candidates.push(p);
    }
    candidates.push(std::env::temp_dir().join(sub));
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join(".harness").join("tmp").join(sub));
    }
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        candidates.push(PathBuf::from(home).join(".harness").join("tmp").join(sub));
    }

    for c in &candidates {
        if try_prepare(c) {
            return c.clone();
        }
    }

    // 全部不可写：返回最后一个候选，让上层在创建时暴露真实错误
    candidates
        .pop()
        .unwrap_or_else(|| std::env::temp_dir().join(sub))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_writable_true_for_target_dir() {
        // target 目录一定可写（cargo 就在里面写产物）
        let dir = std::env::current_dir().unwrap().join("target");
        assert!(is_writable(&dir), "target 目录应可写");
    }

    #[test]
    fn test_is_writable_false_for_missing_dir() {
        let dir = std::env::current_dir()
            .unwrap()
            .join("target")
            .join("definitely_not_exists_dir_xyz");
        assert!(!is_writable(&dir));
    }

    #[test]
    fn test_resolve_returns_writable_dir() {
        let dir = resolve_writable_dir(None, "harness_probe_test");
        assert!(
            is_writable(&dir),
            "resolve_writable_dir 必须返回真正可写的目录，实得: {dir:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_resolve_prefers_explicit_writable() {
        // 显式给一个不可写路径时应跳过，最终仍返回可写目录
        let bogus = std::env::current_dir()
            .unwrap()
            .join("target")
            .join("no_such_parent_xyz")
            .join("deeper");
        // bogus 的父目录可创建，因此可能被选中；只断言最终结果可写
        let dir = resolve_writable_dir(Some(bogus), "harness_probe_pref");
        assert!(is_writable(&dir), "最终目录必须可写，实得: {dir:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
