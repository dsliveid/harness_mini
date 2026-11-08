//! core.db（元数据库）——sessions 轻量元数据、覆盖索引与增量维护
//!
//! 注：现有 `harness_mini.db` 即三库架构中的 core.db（沿用文件名，避免迁移期双文件歧义）。
//! 本模块负责：
//! 1. sessions 表的分库元数据列演进（msg_count / last_preview / storage_state 等，幂等）；
//! 2. 首屏覆盖索引（list_sessions 0.1ms 级的物理基础）；
//! 3. 老库 auto_vacuum 修复（实测结论 5：建库后设定无效，必须 VACUUM 一次才能从 0 变 2）；
//! 4. 增量维护助手（写消息路径调用）与启动对账（以 session.db 为事实源修正偏差）。

use super::blob::make_preview;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

/// sessions 分库元数据列与覆盖索引演进（幂等，随 init_schema 每次启动执行）
pub fn ensure_core_evolution(conn: &Connection) -> Result<(), String> {
    super::ensure_column(conn, "sessions", "msg_count", "msg_count INTEGER NOT NULL DEFAULT 0")?;
    super::ensure_column(conn, "sessions", "tool_event_count", "tool_event_count INTEGER NOT NULL DEFAULT 0")?;
    super::ensure_column(conn, "sessions", "last_preview", "last_preview TEXT")?;
    super::ensure_column(conn, "sessions", "storage_state", "storage_state TEXT NOT NULL DEFAULT 'hot'")?;
    super::ensure_column(conn, "sessions", "bundle_bytes", "bundle_bytes INTEGER NOT NULL DEFAULT 0")?;
    super::ensure_column(conn, "sessions", "schema_version", "schema_version INTEGER NOT NULL DEFAULT 1")?;
    super::ensure_column(conn, "sessions", "logical_deleted_at", "logical_deleted_at TEXT")?;

    // 首屏覆盖索引（计划 §2.4）。idx_sessions_parent 已在 init_schema 建立，此处不重复。
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_sessions_status_lastmsg ON sessions(status, last_message_at DESC, id DESC);
         CREATE INDEX IF NOT EXISTS idx_sessions_project ON sessions(project_id, status);
         CREATE INDEX IF NOT EXISTS idx_sessions_storage_state ON sessions(storage_state);

         -- 迁移版本管理（计划 §2.9）：记录三库拆分各阶段完成状态，支持断点续迁。
         -- 会话级拆分记录以 name = 'split:<session_id>' 落行，version 取毫秒时间戳。
         CREATE TABLE IF NOT EXISTS schema_migrations (
           version INTEGER PRIMARY KEY,
           name TEXT NOT NULL,
           applied_at TEXT NOT NULL
         );",
    )
    .map_err(|e| format!("创建 sessions 覆盖索引失败: {e}"))?;

    // 排序键归一化：last_message_at 为空的存量会话回落到创建时间，
    // 使 list_sessions 的 ORDER BY last_message_at DESC 可稳定走覆盖索引。
    conn.execute(
        "UPDATE sessions SET last_message_at = created_at WHERE last_message_at IS NULL",
        [],
    )
    .map_err(|e| format!("归一化 last_message_at 失败: {e}"))?;
    Ok(())
}

/// 老库 auto_vacuum 修复。
/// 实测（结论 5）：对已建表的老库执行 `PRAGMA auto_vacuum=INCREMENTAL` 后读回仍为 0(NONE)，
/// 必须「先设定 → 再 VACUUM 一次」模式才会真正变为 2(INCREMENTAL)。
/// 返回是否执行了修复（含一次全量 VACUUM，约 39MB/933ms，仅在老库首次启动时发生一次）。
pub fn ensure_auto_vacuum_effective(conn: &Connection) -> Result<bool, String> {
    let mode: i64 = conn
        .query_row("PRAGMA auto_vacuum", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if mode == 2 {
        return Ok(false); // 已是 INCREMENTAL，无需处理
    }
    if mode != 0 {
        // FULL(1) 也能自动回收空间，不值得为此再付一次全量 VACUUM
        return Ok(false);
    }
    conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL;")
        .map_err(|e| e.to_string())?;
    conn.execute_batch("VACUUM").map_err(|e| format!("老库 auto_vacuum 修复 VACUUM 失败: {e}"))?;
    let after: i64 = conn
        .query_row("PRAGMA auto_vacuum", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if after != 2 {
        return Err(format!("auto_vacuum 修复后仍为 {after}（期望 2）"));
    }
    Ok(true)
}

/// 写消息路径调用的增量维护（步骤13）。
/// 消息本体写在 session.db，元数据写在 core.db——跨库无法共享同一事务，
/// 因此「同一操作内」先后提交两库，偏差由启动对账（reconcile_*）兜底修正。
/// `content` 传该消息的最终展示正文；`update_preview=false` 用于流式占位消息
/// （只计数与推进时间，不把空内容写进预览、不覆盖用户消息的预览）。
pub fn apply_message_metadata(
    catalog: &Connection,
    session_id: &str,
    created_at: &str,
    content: &str,
    update_preview: bool,
) -> Result<(), String> {
    let preview = make_preview(content);
    let result = if update_preview {
        catalog.execute(
            "UPDATE sessions SET
               msg_count = msg_count + 1,
               last_message_at = ?2,
               last_preview = ?3,
               updated_at = ?2
             WHERE id = ?1",
            params![session_id, created_at, preview],
        )
    } else {
        catalog.execute(
            "UPDATE sessions SET
               msg_count = msg_count + 1,
               last_message_at = ?2,
               updated_at = ?2
             WHERE id = ?1",
            params![session_id, created_at],
        )
    };
    result.map_err(|e| format!("增量维护 sessions 元数据失败: {e}"))?;
    Ok(())
}

/// 消息内容更新路径调用（流式收尾/编辑重发）：只刷新预览与时间，不动 msg_count。
pub fn refresh_message_preview(
    catalog: &Connection,
    session_id: &str,
    created_at: &str,
    content: &str,
) -> Result<(), String> {
    let preview = make_preview(content);
    catalog
        .execute(
            "UPDATE sessions SET last_message_at = ?2, last_preview = ?3 WHERE id = ?1",
            params![session_id, created_at, preview],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 单会话元数据对账：以 `sessions/{id}/session.db` 为事实源，回填/修正
/// msg_count / tool_event_count / last_preview / last_message_at / bundle_bytes。
/// 幂等可重复执行；session.db 缺失（未迁移的老会话）时回落到 catalog 自身统计。
pub fn reconcile_session_metadata(
    catalog: &Connection,
    data_dir: &Path,
    session_id: &str,
) -> Result<(), String> {
    let db_path = data_dir.join("sessions").join(session_id).join("session.db");
    let (msg_count, tool_event_count, last_created_at, last_content) = if db_path.exists() {
        let sdb = Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| format!("只读打开会话库失败: {e}"))?;
        let msg_count: i64 = sdb
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE session_id = ?1 AND reverted_at IS NULL",
                params![session_id],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let tool_event_count: i64 = sdb
            .query_row("SELECT COUNT(*) FROM tool_events", [], |r| r.get(0))
            .unwrap_or(0);
        let last: Option<(String, String)> = sdb
            .query_row(
                "SELECT created_at, COALESCE(content, '') FROM messages
                 WHERE session_id = ?1 AND reverted_at IS NULL
                 ORDER BY seq DESC, created_at DESC LIMIT 1",
                params![session_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        match last {
            Some((created_at, content)) => (msg_count, tool_event_count, Some(created_at), content),
            None => (0, tool_event_count, None, String::new()),
        }
    } else {
        // 未迁移老会话：catalog 自身统计（messages 表仍在本库）
        let has_messages: bool = catalog
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='messages'",
                [],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if !has_messages {
            return Ok(());
        }
        let msg_count: i64 = catalog
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE session_id = ?1 AND reverted_at IS NULL",
                params![session_id],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let last: Option<(String, String)> = catalog
            .query_row(
                "SELECT created_at, COALESCE(content, '') FROM messages
                 WHERE session_id = ?1 AND reverted_at IS NULL
                 ORDER BY seq DESC, created_at DESC LIMIT 1",
                params![session_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        match last {
            Some((created_at, content)) => (msg_count, 0, Some(created_at), content),
            None => (0, 0, None, String::new()),
        }
    };

    // bundle 占用字节：db + wal 文件合计
    let mut bundle_bytes: i64 = 0;
    for suffix in ["", "-wal"] {
        let mut p = db_path.clone().into_os_string();
        p.push(suffix);
        if let Ok(m) = std::fs::metadata(&p) {
            bundle_bytes += m.len() as i64;
        }
    }

    let preview = make_preview(&last_content);
    catalog
        .execute(
            "UPDATE sessions SET
               msg_count = ?2, tool_event_count = ?3,
               last_preview = ?4, last_message_at = COALESCE(?5, last_message_at),
               bundle_bytes = ?6
             WHERE id = ?1",
            params![session_id, msg_count, tool_event_count, preview, last_created_at, bundle_bytes],
        )
        .map_err(|e| format!("回填会话元数据失败: {e}"))?;
    Ok(())
}

/// 全量对账（启动后台任务）：遍历 catalog 中全部会话逐一修正。返回 (成功, 跳过)。
pub fn reconcile_all_session_metadata(catalog: &Connection, data_dir: &Path) -> Result<(usize, usize), String> {
    let ids: Vec<String> = {
        let mut stmt = catalog
            .prepare("SELECT id FROM sessions")
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| r.get(0)).map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?
    };
    let (mut ok, mut skip) = (0usize, 0usize);
    for id in ids {
        match reconcile_session_metadata(catalog, data_dir, &id) {
            Ok(()) => ok += 1,
            Err(_) => skip += 1,
        }
    }
    Ok((ok, skip))
}

/// 归档/恢复时更新存储状态（步骤16）：hot | archived | migrated
pub fn set_storage_state(conn: &Connection, session_id: &str, state: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE sessions SET storage_state = ?2 WHERE id = ?1",
        params![session_id, state],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------- storage.mode 开关（步骤14） ----------

pub const STORAGE_MODE_KEY: &str = "storage.mode";
pub const MODE_SINGLE: u8 = 0; // 读写一律 catalog（旧单库模式，回滚通路）
pub const MODE_HYBRID: u8 = 1; // 有 bundle 走分库，无 bundle 回落 catalog（默认，兼容现状）
pub const MODE_SPLIT: u8 = 2; // 同 hybrid 路由 + 日志库为审计读的权威源

/// 读取存储模式（settings 表 storage.mode 行；默认 hybrid）
pub fn get_storage_mode(conn: &Connection) -> u8 {
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![STORAGE_MODE_KEY],
        |r| r.get::<_, String>(0),
    )
    .ok()
    .map(|s| match s.as_str() {
        "single" => MODE_SINGLE,
        "split" => MODE_SPLIT,
        _ => MODE_HYBRID,
    })
    .unwrap_or(MODE_HYBRID)
}

/// 写入存储模式（显式确认入口调用；非法值拒绝）
pub fn set_storage_mode(conn: &Connection, mode: &str) -> Result<(), String> {
    if !matches!(mode, "single" | "hybrid" | "split") {
        return Err(format!("非法存储模式: {mode}（仅支持 single|hybrid|split）"));
    }
    conn.execute(
        "INSERT INTO settings(key, value) VALUES(?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![STORAGE_MODE_KEY, mode],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 逻辑删除标记（物理清理由后台任务执行，删除可撤销期内文件保留）
pub fn mark_logical_deleted(conn: &Connection, session_id: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE sessions SET logical_deleted_at = ?2 WHERE id = ?1",
        params![session_id, super::now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        super::super::init_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn test_core_evolution_adds_columns_and_indexes() {
        let conn = setup();
        ensure_core_evolution(&conn).unwrap();

        let cols: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(sessions)").unwrap();
            let rows = stmt.query_map([], |r| r.get::<_, String>(1)).unwrap();
            rows.collect::<Result<Vec<_>, _>>().unwrap()
        };
        for c in ["msg_count", "tool_event_count", "last_preview", "storage_state", "bundle_bytes", "schema_version", "logical_deleted_at"] {
            assert!(cols.iter().any(|x| x == c), "缺少列 {c}");
        }

        let idx: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='sessions'")
                .unwrap();
            let rows = stmt.query_map([], |r| r.get(0)).unwrap();
            rows.collect::<Result<Vec<_>, _>>().unwrap()
        };
        assert!(idx.iter().any(|x| x == "idx_sessions_status_lastmsg"));
        assert!(idx.iter().any(|x| x == "idx_sessions_storage_state"));

        // 幂等：重复执行不报错
        ensure_core_evolution(&conn).unwrap();
    }

    #[test]
    fn test_apply_message_metadata_increments() {
        let conn = setup();
        ensure_core_evolution(&conn).unwrap();
        let s = super::super::create_session(&conn, "D:/ws", None, "会话", "confirm").unwrap();

        apply_message_metadata(&conn, &s.id, "2025-11-07T10:00:00Z", "第一条消息内容", true).unwrap();
        apply_message_metadata(&conn, &s.id, "2025-11-07T10:01:00Z", "第二条消息内容", true).unwrap();

        let (count, last_at, preview): (i64, String, String) = conn
            .query_row(
                "SELECT msg_count, last_message_at, last_preview FROM sessions WHERE id = ?1",
                params![s.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(count, 2);
        assert_eq!(last_at, "2025-11-07T10:01:00Z");
        assert_eq!(preview, "第二条消息内容");
    }

    #[test]
    fn test_refresh_message_preview_keeps_count() {
        let conn = setup();
        ensure_core_evolution(&conn).unwrap();
        let s = super::super::create_session(&conn, "D:/ws", None, "会话", "confirm").unwrap();
        apply_message_metadata(&conn, &s.id, "2025-11-07T10:00:00Z", "占位", true).unwrap();
        refresh_message_preview(&conn, &s.id, "2025-11-07T10:00:05Z", "流式结束后的完整正文").unwrap();

        let (count, preview): (i64, String) = conn
            .query_row(
                "SELECT msg_count, last_preview FROM sessions WHERE id = ?1",
                params![s.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(count, 1, "preview 刷新不应改变计数");
        assert_eq!(preview, "流式结束后的完整正文");
    }

    #[test]
    fn test_reconcile_from_session_db() {
        let conn = setup();
        ensure_core_evolution(&conn).unwrap();
        let s = super::super::create_session(&conn, "D:/ws", None, "会话", "confirm").unwrap();

        // 构造一个真实 session.db：2 条消息 + 1 个工具事件
        let bundle = crate::session_bundle::SessionBundle::new(&s.id, std::env::temp_dir().join(format!("hm_core_test_{}", uuid::Uuid::new_v4())));
        let sdb = bundle.open_db().unwrap();
        sdb.execute(
            "INSERT INTO messages (id, session_id, seq, role, content, created_at) VALUES ('m1', ?1, 1, 'user', '用户提问内容', '2025-11-07T10:00:00Z')",
            params![s.id],
        ).unwrap();
        sdb.execute(
            "INSERT INTO messages (id, session_id, seq, role, content, created_at) VALUES ('m2', ?1, 2, 'assistant', '助手回答内容', '2025-11-07T10:00:10Z')",
            params![s.id],
        ).unwrap();
        sdb.execute(
            "INSERT INTO tool_events (id, message_id, tool_name, params_json, status, created_at) VALUES ('t1', 'm2', 'bash', '{}', 'success', '2025-11-07T10:00:11Z')",
            [],
        ).unwrap();
        drop(sdb);

        reconcile_session_metadata(&conn, &bundle.data_dir, &s.id).unwrap();

        let (msg_count, te_count, preview, bundle_bytes): (i64, i64, String, i64) = conn
            .query_row(
                "SELECT msg_count, tool_event_count, last_preview, bundle_bytes FROM sessions WHERE id = ?1",
                params![s.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(msg_count, 2);
        assert_eq!(te_count, 1);
        assert_eq!(preview, "助手回答内容");
        assert!(bundle_bytes > 0, "bundle_bytes 应统计 db+wal 文件占用");

        let _ = std::fs::remove_dir_all(&bundle.data_dir);
    }
}
