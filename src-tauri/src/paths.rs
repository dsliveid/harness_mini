//! 程序数据目录解析与迁移。
//!
//! 默认策略：数据存放于程序同级目录（release 为 exe 同级 `data\`，开发构建为
//! 项目根目录 `.dev-data\`），不依赖用户目录。
//! 仅当用户在设置中显式选择自定义目录时，才在应用数据目录（AppData）写入一个
//! `data_dir.txt` 指针文件作为下次启动的引导——这是唯一会落到用户目录的文件。
//! 旧版本将 SQLite 库存在 AppData 下，首次启动时用 VACUUM INTO 快照一次性迁入新目录。

use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

const DB_FILE: &str = "harness_mini.db";
const POINTER_FILE: &str = "data_dir.txt";

/// 默认数据目录：release 为 exe 同级 data\；debug 构建为项目根目录 .dev-data\。
/// dev 不放 exe 旁（target\debug\）是怕被 cargo clean 清掉；不放 src-tauri\ 内是怕
/// 被 tauri dev 的文件监视器当成源码变更，SQLite WAL 文件一更新就触发无限重编译。
pub fn default_data_dir() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        return manifest.parent().map(|p| p.join(".dev-data"));
    }
    #[cfg(not(debug_assertions))]
    {
        let exe = std::env::current_exe().ok()?;
        return exe.parent().map(|p| p.join("data"));
    }
}

/// 探测目录可写（不存在则尝试创建）
pub fn ensure_writable(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".write_probe");
    match std::fs::write(&probe, b"ok") {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// 两个路径是否指向同一目录（best-effort，Windows 下忽略大小写）
pub fn same_dir(a: &Path, b: &Path) -> bool {
    if let (Ok(x), Ok(y)) = (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        return x == y;
    }
    let ka = a.to_string_lossy().replace('/', "\\").to_lowercase();
    let kb = b.to_string_lossy().replace('/', "\\").to_lowercase();
    ka.trim_end_matches('\\') == kb.trim_end_matches('\\')
}

/// 应用数据目录（AppData\Roaming\<identifier>）：旧库迁移来源 + 指针文件所在
fn legacy_data_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok()
}

/// 读取自定义数据目录指针（用户在设置中选择目录时写入）
pub fn read_pointer(app: &AppHandle) -> Option<PathBuf> {
    let p = legacy_data_dir(app)?.join(POINTER_FILE);
    let s = std::fs::read_to_string(p).ok()?;
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(PathBuf::from(t))
    }
}

pub fn write_pointer(app: &AppHandle, dir: &Path) -> Result<(), String> {
    let base = legacy_data_dir(app).ok_or("无法定位应用数据目录")?;
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    std::fs::write(base.join(POINTER_FILE), dir.to_string_lossy().as_bytes())
        .map_err(|e| e.to_string())
}

pub fn clear_pointer(app: &AppHandle) -> Result<(), String> {
    let base = legacy_data_dir(app).ok_or("无法定位应用数据目录")?;
    match std::fs::remove_file(base.join(POINTER_FILE)) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// 旧版 AppData 库 → 新数据目录一次性迁移（VACUUM INTO 一致性快照；目标已有库时跳过）
pub fn migrate_legacy_db(new_dir: &Path, legacy_dir: Option<&Path>) -> Result<(), String> {
    let Some(legacy) = legacy_dir else { return Ok(()) };
    if same_dir(legacy, new_dir) {
        return Ok(());
    }
    let legacy_db = legacy.join(DB_FILE);
    if !legacy_db.exists() {
        return Ok(());
    }
    let target = new_dir.join(DB_FILE);
    if target.exists() {
        return Ok(());
    }
    let conn = rusqlite::Connection::open(&legacy_db).map_err(|e| e.to_string())?;
    let sql = format!("VACUUM INTO '{}'", target.to_string_lossy().replace('\'', "''"));
    let result = conn.execute(&sql, []);
    let _ = conn.close();
    result.map_err(|e| format!("迁移旧数据库失败: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("hm_paths_{tag}_{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn ensure_writable_detects_readonly_dir() {
        let dir = temp_dir("writable");
        assert!(ensure_writable(&dir));
        assert!(dir.join(DB_FILE).parent().is_some());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn same_dir_ignores_case_and_trailing_sep() {
        let a = Path::new(if cfg!(windows) { "D:\\A\\b" } else { "/a/b" });
        let b = Path::new(if cfg!(windows) { "d:\\a\\b\\" } else { "/a/b" });
        // 路径不存在时走字符串比较分支
        assert_eq!(same_dir(a, b), cfg!(windows));
    }

    #[test]
    fn migrates_legacy_db_with_vacuum_into() {
        let legacy = temp_dir("legacy");
        let new_dir = temp_dir("new");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::create_dir_all(&new_dir).unwrap();
        // 构造一个"旧库"
        let conn = rusqlite::Connection::open(legacy.join(DB_FILE)).unwrap();
        conn.execute_batch(
            "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO settings(key, value) VALUES('app_settings', '{\"providers\":[]}');",
        )
        .unwrap();
        drop(conn);

        migrate_legacy_db(&new_dir, Some(&legacy)).unwrap();
        // 目标库已生成且数据完整
        let target = rusqlite::Connection::open(new_dir.join(DB_FILE)).unwrap();
        let v: String = target
            .query_row("SELECT value FROM settings WHERE key='app_settings'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, "{\"providers\":[]}");

        // 目标已存在时重复调用为 no-op（不会报错、不会覆盖）
        migrate_legacy_db(&new_dir, Some(&legacy)).unwrap();
        std::fs::remove_dir_all(&legacy).ok();
        std::fs::remove_dir_all(&new_dir).ok();
    }
}
