use super::resource::SessionResource;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// 单个会话自包含存储包（Session-as-a-Bundle）
#[derive(Debug, Clone)]
pub struct SessionBundle {
    pub session_id: String,
    pub data_dir: PathBuf,
}

impl SessionBundle {
    pub fn new(session_id: impl Into<String>, data_dir: impl Into<PathBuf>) -> Self {
        Self {
            session_id: session_id.into(),
            data_dir: data_dir.into(),
        }
    }

    pub fn resource(&self) -> SessionResource {
        SessionResource::new(&self.session_id, &self.data_dir)
    }

    /// 检查该会话的专属 session.db 是否存在
    pub fn exists(&self) -> bool {
        self.resource().bundle_db_path().exists()
    }

    /// 检查该会话是否处于归档状态（存在归档目录或归档包）
    pub fn is_archived(&self) -> bool {
        self.resource().archive_bundle_dir().exists()
            || self.resource().archive_package_path().exists()
    }

    /// 打开专属 session.db 连接，并自动初始化其表结构与 PRAGMA 配置
    pub fn open_db(&self) -> Result<Connection, String> {
        self.resource().ensure_bundle_dir()?;
        let db_path = self.resource().bundle_db_path();
        let conn = Connection::open(&db_path)
            .map_err(|e| format!("打开会话专属数据库失败 ({}): {e}", db_path.display()))?;

        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA auto_vacuum = INCREMENTAL;",
        )
        .map_err(|e| format!("配置会话数据库 PRAGMA 失败: {e}"))?;

        init_session_bundle_schema(&conn)?;
        Ok(conn)
    }

    /// 以只读模式打开（用于查看归档会话或只读冷查）
    pub fn open_db_readonly(&self) -> Result<Connection, String> {
        let db_path = if self.exists() {
            self.resource().bundle_db_path()
        } else if self.resource().archive_bundle_dir().exists() {
            self.resource().archive_bundle_dir().join("session.db")
        } else {
            return Err(format!("找不到会话数据库: {}", self.session_id));
        };

        let conn = Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| format!("只读打开会话数据库失败 ({}): {e}", db_path.display()))?;

        Ok(conn)
    }
}

/// 会话专属独立数据库表结构初始化
pub fn init_session_bundle_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS messages (
          id TEXT PRIMARY KEY,
          session_id TEXT NOT NULL,
          run_id TEXT,
          seq INTEGER NOT NULL,
          role TEXT NOT NULL,
          content TEXT,
          reasoning TEXT,
          tool_calls_json TEXT,
          tool_call_id TEXT,
          queued INTEGER NOT NULL DEFAULT 0,
          usage_json TEXT,
          created_at TEXT NOT NULL,
          prompt_tokens INTEGER DEFAULT 0,
          completion_tokens INTEGER DEFAULT 0,
          total_tokens INTEGER DEFAULT 0,
          cached_tokens INTEGER DEFAULT 0,
          is_estimated INTEGER DEFAULT 0,
          duration_ms INTEGER DEFAULT 0,
          turn_duration_ms INTEGER DEFAULT 0,
          attachments_json TEXT,
          reverted_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_bundle_messages_session ON messages(session_id, seq);
        CREATE INDEX IF NOT EXISTS idx_bundle_messages_run ON messages(run_id);

        CREATE TABLE IF NOT EXISTS tool_events (
          id TEXT PRIMARY KEY,
          message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
          tool_name TEXT NOT NULL,
          tool_call_id TEXT,
          params_json TEXT NOT NULL,
          result_text TEXT,
          status TEXT NOT NULL,
          approval_scope TEXT,
          created_at TEXT NOT NULL,
          subprocess_id TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_bundle_tool_events_msg ON tool_events(message_id);

        CREATE TABLE IF NOT EXISTS session_rules_t (
          id TEXT PRIMARY KEY,
          session_id TEXT NOT NULL,
          kind TEXT NOT NULL,
          pattern TEXT NOT NULL,
          created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_bundle_session_rules ON session_rules_t(session_id);

        CREATE TABLE IF NOT EXISTS session_kv (
          session_id TEXT NOT NULL,
          key TEXT NOT NULL,
          value TEXT NOT NULL,
          PRIMARY KEY (session_id, key)
        );

        CREATE TABLE IF NOT EXISTS session_compactions (
          id TEXT PRIMARY KEY,
          session_id TEXT NOT NULL,
          start_seq INTEGER NOT NULL,
          end_seq INTEGER NOT NULL,
          summary_markdown TEXT NOT NULL,
          tokens_before INTEGER NOT NULL,
          created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS long_tasks (
          id TEXT PRIMARY KEY,
          session_id TEXT NOT NULL,
          workspace_path TEXT NOT NULL,
          goal TEXT NOT NULL,
          status TEXT NOT NULL,
          current_subtask_index INTEGER NOT NULL DEFAULT 0,
          subtasks_json TEXT NOT NULL DEFAULT '[]',
          max_budget_tokens INTEGER,
          total_tokens_used INTEGER NOT NULL DEFAULT 0,
          current_step INTEGER NOT NULL DEFAULT 0,
          max_steps INTEGER NOT NULL DEFAULT 50,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS task_checkpoints (
          id TEXT PRIMARY KEY,
          task_id TEXT NOT NULL REFERENCES long_tasks(id) ON DELETE CASCADE,
          step_number INTEGER NOT NULL,
          subtask_id TEXT,
          status TEXT NOT NULL,
          summary TEXT NOT NULL,
          working_memory TEXT NOT NULL,
          git_commit_hash TEXT,
          created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS tool_file_snapshots (
          id TEXT PRIMARY KEY,
          session_id TEXT NOT NULL,
          message_id TEXT NOT NULL,
          tool_event_id TEXT NOT NULL,
          file_path TEXT NOT NULL,
          before_hash TEXT,
          after_hash TEXT NOT NULL,
          is_new_file INTEGER NOT NULL DEFAULT 0,
          reverted_at TEXT,
          created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_bundle_snapshots_session ON tool_file_snapshots(session_id, created_at);

        CREATE TABLE IF NOT EXISTS runs (
          id TEXT PRIMARY KEY,
          session_id TEXT NOT NULL,
          trigger_type TEXT NOT NULL DEFAULT 'manual',
          status TEXT NOT NULL,
          started_at TEXT NOT NULL,
          ended_at TEXT,
          trigger_message_id TEXT,
          duration_ms INTEGER DEFAULT 0,
          total_tokens INTEGER DEFAULT 0,
          prompt_tokens INTEGER DEFAULT 0,
          completion_tokens INTEGER DEFAULT 0,
          cached_tokens INTEGER DEFAULT 0
        );
        "#,
    )
    .map_err(|e| format!("初始化会话专属数据库 Schema 失败: {e}"))?;
    Ok(())
}

/// 自适应会话连接路由：
/// 1. 若该会话已有专属 session.db，优先在其专属库执行
/// 2. 若尚未切片迁移，则平滑回退到全局主库执行
pub fn with_session_db<F, R>(
    catalog_conn: &Connection,
    data_dir_opt: Option<&Path>,
    session_id: &str,
    f: F,
) -> Result<R, String>
where
    F: FnOnce(&Connection) -> Result<R, String>,
{
    if let Some(data_dir) = data_dir_opt {
        let bundle = SessionBundle::new(session_id, data_dir);
        if bundle.exists() {
            let conn = bundle.open_db()?;
            return f(&conn);
        } else if bundle.is_archived() {
            if let Ok(conn) = bundle.open_db_readonly() {
                return f(&conn);
            }
        }
    }
    f(catalog_conn)
}

/// 扫描并自愈孤儿 Bundle 目录与附件（如果某会话已从 Catalog 删除，则安全清理其残留物理目录）
pub fn reconcile_orphan_bundles(catalog_conn: &Connection, data_dir: &Path) -> Result<usize, String> {
    let mut cleaned_count = 0;

    // 1. 检查 sessions/ 目录
    let sessions_dir = data_dir.join("sessions");
    if sessions_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&sessions_dir) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    let sid = entry.file_name().to_string_lossy().to_string();
                    let exists: bool = catalog_conn
                        .query_row("SELECT 1 FROM sessions WHERE id = ?1", rusqlite::params![sid], |_| Ok(true))
                        .unwrap_or(false);
                    if !exists {
                        let _ = std::fs::remove_dir_all(entry.path());
                        cleaned_count += 1;
                    }
                }
            }
        }
    }

    // 2. 检查 legacy attachments/ 目录
    let att_dir = data_dir.join("attachments");
    if att_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&att_dir) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    let sid = entry.file_name().to_string_lossy().to_string();
                    if sid != "draft" {
                        let exists: bool = catalog_conn
                            .query_row("SELECT 1 FROM sessions WHERE id = ?1", rusqlite::params![sid], |_| Ok(true))
                            .unwrap_or(false);
                        if !exists {
                            let _ = std::fs::remove_dir_all(entry.path());
                            cleaned_count += 1;
                        }
                    }
                }
            }
        }
    }

    Ok(cleaned_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_bundle_crud_lifecycle() {
        let temp_dir = std::env::temp_dir().join(format!("hm_bundle_test_{}", uuid::Uuid::new_v4()));
        let bundle = SessionBundle::new("session_123", &temp_dir);

        assert!(!bundle.exists());
        let conn = bundle.open_db().unwrap();
        assert!(bundle.exists());

        // 验证表已成功初始化并可读写
        conn.execute(
            "INSERT INTO messages (id, session_id, seq, role, content, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params!["msg-1", "session_123", 1, "user", "hello bundle", "2026-10-19T00:00:00Z"],
        ).unwrap();

        let count: i64 = conn.query_row("SELECT count(*) FROM messages WHERE session_id = 'session_123'", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 1);

        drop(conn);

        // 验证只读打开
        let ro_conn = bundle.open_db_readonly().unwrap();
        let ro_count: i64 = ro_conn.query_row("SELECT count(*) FROM messages", [], |r| r.get(0)).unwrap();
        assert_eq!(ro_count, 1);

        // 清理测试目录
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_reconcile_orphan_bundles() {
        let temp_dir = std::env::temp_dir().join(format!("hm_rec_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let catalog_path = temp_dir.join("catalog.db");
        let catalog_conn = Connection::open(&catalog_path).unwrap();
        crate::store::init_schema(&catalog_conn).unwrap();

        // 创建一个在 Catalog 中的有效会话
        let s = crate::store::create_session(&catalog_conn, "D:/ws", None, "有效会话", "confirm").unwrap();
        let valid_bundle = SessionBundle::new(&s.id, &temp_dir);
        valid_bundle.resource().ensure_bundle_dir().unwrap();

        // 创建一个孤儿 Bundle 目录（不在 Catalog 中）
        let orphan_sid = "orphan-sess-999";
        let orphan_bundle = SessionBundle::new(orphan_sid, &temp_dir);
        orphan_bundle.resource().ensure_bundle_dir().unwrap();
        assert!(orphan_bundle.resource().bundle_dir().exists());

        // 创建一个孤儿附件目录
        let orphan_att = temp_dir.join("attachments").join("orphan-sess-888");
        std::fs::create_dir_all(&orphan_att).unwrap();
        assert!(orphan_att.exists());

        // 执行自愈检查
        let cleaned = reconcile_orphan_bundles(&catalog_conn, &temp_dir).unwrap();
        assert_eq!(cleaned, 2);

        // 验证有效 Bundle 完好，孤儿已被清理
        assert!(valid_bundle.resource().bundle_dir().exists());
        assert!(!orphan_bundle.resource().bundle_dir().exists());
        assert!(!orphan_att.exists());

        drop(catalog_conn);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
