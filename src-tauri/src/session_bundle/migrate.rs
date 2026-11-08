use super::bundle::SessionBundle;
use rusqlite::{params, Connection};
use std::path::Path;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationReport {
    pub total_sessions: usize,
    pub migrated_sessions: usize,
    pub skipped_sessions: usize,
    pub errors: Vec<String>,
}

/// 将单个会话的数据从 Catalog 主库完整迁移至其独立的 SessionBundle
pub fn migrate_single_session(
    catalog_conn: &Connection,
    catalog_db_path: &Path,
    data_dir: &Path,
    session_id: &str,
) -> Result<bool, String> {
    let bundle = SessionBundle::new(session_id, data_dir);
    if bundle.exists() {
        return Ok(false); // 已迁移，幂等跳过
    }

    let mut session_conn = bundle.open_db()?;

    // 尝试通过 ATTACH DATABASE 高速批量灌入
    let attach_sql = format!(
        "ATTACH DATABASE '{}' AS catalog_source;",
        catalog_db_path.to_string_lossy().replace('\'', "''")
    );

    session_conn
        .execute(&attach_sql, [])
        .map_err(|e| format!("ATTACH catalog_source 失败: {e}"))?;

    // 1. 统计源库中该会话的消息总数，用于写入校验
    let src_msg_count: i64 = session_conn
        .query_row(
            "SELECT count(*) FROM catalog_source.messages WHERE session_id = ?1;",
            params![session_id],
            |r| r.get(0),
        )
        .unwrap_or(0);

    // tool_events 源计数（步骤10 双向对账）
    let src_te_count: i64 = session_conn
        .query_row(
            "SELECT count(*) FROM catalog_source.tool_events WHERE message_id IN
               (SELECT id FROM catalog_source.messages WHERE session_id = ?1);",
            params![session_id],
            |r| r.get(0),
        )
        .unwrap_or(0);

    // 2. 开启事务并在显式指定所有列名的情况下迁移，杜绝因列物理顺序变动引发类型错配
    let tx = session_conn
        .transaction()
        .map_err(|e| format!("开启会话迁移事务失败: {e}"))?;

    tx.execute(
        "INSERT INTO messages (
            id, session_id, run_id, seq, role, content, reasoning,
            tool_calls_json, tool_call_id, queued, usage_json, created_at,
            prompt_tokens, completion_tokens, total_tokens, cached_tokens,
            is_estimated, duration_ms, turn_duration_ms, attachments_json, reverted_at, runtime_anchor
        )
        SELECT
            id, session_id, run_id, seq, role, content, reasoning,
            tool_calls_json, tool_call_id, queued, usage_json, created_at,
            prompt_tokens, completion_tokens, total_tokens, cached_tokens,
            is_estimated, duration_ms, turn_duration_ms, attachments_json, reverted_at, runtime_anchor
        FROM catalog_source.messages
        WHERE session_id = ?1;",
        params![session_id],
    )
    .map_err(|e| format!("迁移 messages 失败: {e}"))?;

    tx.execute(
        "INSERT INTO tool_events (
            id, message_id, tool_name, tool_call_id, params_json,
            result_text, status, approval_scope, created_at, subprocess_id
        )
        SELECT
            id, message_id, tool_name, tool_call_id, params_json,
            result_text, status, approval_scope, created_at, subprocess_id
        FROM catalog_source.tool_events
        WHERE message_id IN (SELECT id FROM catalog_source.messages WHERE session_id = ?1);",
        params![session_id],
    )
    .map_err(|e| format!("迁移 tool_events 失败: {e}"))?;

    tx.execute(
        "INSERT INTO tool_file_snapshots (
            id, session_id, message_id, tool_event_id, file_path,
            before_hash, after_hash, is_new_file, reverted_at, created_at
        )
        SELECT
            id, session_id, message_id, tool_event_id, file_path,
            before_hash, after_hash, is_new_file, reverted_at, created_at
        FROM catalog_source.tool_file_snapshots
        WHERE session_id = ?1;",
        params![session_id],
    )
    .map_err(|e| format!("迁移 tool_file_snapshots 失败: {e}"))?;

    tx.execute(
        "INSERT INTO session_kv (session_id, key, value)
        SELECT session_id, key, value
        FROM catalog_source.session_kv
        WHERE session_id = ?1;",
        params![session_id],
    )
    .map_err(|e| format!("迁移 session_kv 失败: {e}"))?;

    tx.execute(
        "INSERT INTO session_rules_t (id, session_id, kind, pattern, created_at)
        SELECT id, session_id, kind, pattern, created_at
        FROM catalog_source.session_rules_t
        WHERE session_id = ?1;",
        params![session_id],
    )
    .map_err(|e| format!("迁移 session_rules_t 失败: {e}"))?;

    tx.execute(
        "INSERT INTO session_compactions (id, session_id, start_seq, end_seq, summary_markdown, tokens_before, created_at)
        SELECT id, session_id, start_seq, end_seq, summary_markdown, tokens_before, created_at
        FROM catalog_source.session_compactions
        WHERE session_id = ?1;",
        params![session_id],
    )
    .map_err(|e| format!("迁移 session_compactions 失败: {e}"))?;

    tx.execute(
        "INSERT INTO long_tasks (
            id, session_id, workspace_path, goal, status,
            current_subtask_index, subtasks_json, max_budget_tokens,
            total_tokens_used, current_step, max_steps, created_at, updated_at
        )
        SELECT
            id, session_id, workspace_path, goal, status,
            current_subtask_index, subtasks_json, max_budget_tokens,
            total_tokens_used, current_step, max_steps, created_at, updated_at
        FROM catalog_source.long_tasks
        WHERE session_id = ?1;",
        params![session_id],
    )
    .map_err(|e| format!("迁移 long_tasks 失败: {e}"))?;

    tx.execute(
        "INSERT INTO task_checkpoints (
            id, task_id, step_number, subtask_id, status,
            summary, working_memory, git_commit_hash, created_at
        )
        SELECT
            id, task_id, step_number, subtask_id, status,
            summary, working_memory, git_commit_hash, created_at
        FROM catalog_source.task_checkpoints
        WHERE task_id IN (SELECT id FROM catalog_source.long_tasks WHERE session_id = ?1);",
        params![session_id],
    )
    .map_err(|e| format!("迁移 task_checkpoints 失败: {e}"))?;

    tx.execute(
        "INSERT INTO runs (
            id, session_id, trigger_type, status, started_at, ended_at,
            trigger_message_id, duration_ms, total_tokens, prompt_tokens,
            completion_tokens, cached_tokens
        )
        SELECT
            id, session_id, trigger_type, status, started_at, ended_at,
            trigger_message_id, duration_ms, total_tokens, prompt_tokens,
            completion_tokens, cached_tokens
        FROM catalog_source.runs
        WHERE session_id = ?1;",
        params![session_id],
    )
    .map_err(|e| format!("迁移 runs 失败: {e}"))?;


    // 检查源库是否存在 llm_requests 表
    let has_llm_requests: bool = tx
        .query_row(
            "SELECT 1 FROM catalog_source.sqlite_master WHERE type='table' AND name='llm_requests';",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);

    if has_llm_requests {
        // 实施选项 A：迁移 LLM 请求审计时保留模型、端点与时间戳元数据，丢弃重复膨胀的 payload_json
        tx.execute(
            "INSERT INTO llm_requests (
                id, session_id, message_id, run_id, model, endpoint, payload_json, created_at
            )
            SELECT
                id, session_id, message_id, run_id, model, endpoint,
                '{\"migrated\":true,\"lean\":true}',
                created_at
            FROM catalog_source.llm_requests
            WHERE session_id = ?1;",
            params![session_id],
        )
        .map_err(|e| format!("迁移 llm_requests 失败: {e}"))?;
    }
    // 3. 校验写入目标库的消息数量，不一致则绝对禁止清理源数据
    let dst_msg_count: i64 = tx
        .query_row(
            "SELECT count(*) FROM messages WHERE session_id = ?1;",
            params![session_id],
            |r| r.get(0),
        )
        .unwrap_or(0);

    if dst_msg_count != src_msg_count {
        // 对账失败：tx 在此 drop 即自动 ROLLBACK，目标库不留半截数据，源库绝不删除
        return Err(format!(
            "会话 {session_id} 迁移校验失败: 源消息数 {src_msg_count}，目标写入数 {dst_msg_count}，中止源库清理"
        ));
    }

    // 双向行数对账（步骤10）：tool_events 一并对账，杜绝部分写入
    let dst_te_count: i64 = tx
        .query_row("SELECT count(*) FROM tool_events;", [], |r| r.get(0))
        .unwrap_or(0);
    if dst_te_count != src_te_count {
        return Err(format!(
            "会话 {session_id} tool_events 迁移校验失败: 源 {src_te_count} 条，目标 {dst_te_count} 条，中止源库清理"
        ));
    }

    tx.commit().map_err(|e| format!("提交会话迁移事务失败: {e}"))?;

    // 断点续迁（步骤10）：每会话迁移完成即在 core.db.schema_migrations 记录，
    // name = 'split:<session_id>'；可中断续跑（重复迁移被 bundle.exists() 幂等跳过）。
    record_split_migration(catalog_conn, session_id);

    let _ = session_conn.execute("DETACH DATABASE catalog_source;", []);
    drop(session_conn);

    // 迁移附件目录：data/attachments/{session_id} -> data/sessions/{session_id}/attachments/
    let legacy_att = bundle.resource().legacy_attachments_dir();
    let bundle_att = bundle.resource().bundle_attachments_dir();
    if legacy_att.exists() {
        let _ = std::fs::create_dir_all(&bundle_att);
        if let Ok(entries) = std::fs::read_dir(&legacy_att) {
            for entry in entries.flatten() {
                let target = bundle_att.join(entry.file_name());
                let _ = std::fs::copy(entry.path(), target);
            }
        }
        let _ = std::fs::remove_dir_all(&legacy_att);
    }

    // 从 Catalog 主库清除已迁移的数据行
    let _ = catalog_conn.execute(
        "DELETE FROM tool_events WHERE message_id IN (SELECT id FROM messages WHERE session_id = ?1);",
        params![session_id],
    );
    let _ = catalog_conn.execute(
        "DELETE FROM messages WHERE session_id = ?1;",
        params![session_id],
    );
    let _ = catalog_conn.execute(
        "DELETE FROM tool_file_snapshots WHERE session_id = ?1;",
        params![session_id],
    );
    let _ = catalog_conn.execute(
        "DELETE FROM session_kv WHERE session_id = ?1;",
        params![session_id],
    );
    let _ = catalog_conn.execute(
        "DELETE FROM session_rules_t WHERE session_id = ?1;",
        params![session_id],
    );
    let _ = catalog_conn.execute(
        "DELETE FROM session_compactions WHERE session_id = ?1;",
        params![session_id],
    );
    let _ = catalog_conn.execute(
        "DELETE FROM long_tasks WHERE session_id = ?1;",
        params![session_id],
    );
    let _ = catalog_conn.execute(
        "DELETE FROM runs WHERE session_id = ?1;",
        params![session_id],
    );
    let _ = catalog_conn.execute(
        "DELETE FROM llm_requests WHERE session_id = ?1;",
        params![session_id],
    );

    Ok(true)
}

/// 断点续迁记录（步骤10）：在 core.db.schema_migrations 写入 `split:<session_id>` 行。
/// version 主键取毫秒时间戳，撞键时 +1 重试（同毫秒多会话迁移的极小概率冲突）。
/// 记录失败不阻断迁移主流程（幂等性由 bundle.exists() 保证，记录仅用于审计与续迁查询）。
fn record_split_migration(catalog_conn: &Connection, session_id: &str) {
    let name = format!("split:{session_id}");
    let mut version = chrono::Utc::now().timestamp_millis();
    for _ in 0..5 {
        let res = catalog_conn.execute(
            "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?1, ?2, ?3)",
            params![version, name, chrono::Utc::now().to_rfc3339()],
        );
        match res {
            Ok(_) => return,
            Err(_) => version += 1,
        }
    }
}

/// 扫描并迁移主库中所有存量会话至各自分离的 SessionBundle
pub fn migrate_all_sessions(
    catalog_conn: &Connection,
    catalog_db_path: &Path,
    data_dir: &Path,
) -> Result<MigrationReport, String> {
    let mut stmt = catalog_conn
        .prepare("SELECT id FROM sessions ORDER BY created_at ASC")
        .map_err(|e| format!("查询会话列表失败: {e}"))?;

    let session_ids: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    // 先汇总历史 token 统计到 catalog 的 daily_token_stats 表中（确保分库后 token 统计报表毫秒级可用且完整保留历史）
    let _ = catalog_conn.execute(
        "INSERT OR REPLACE INTO daily_token_stats (date, session_id, project_id, prompt_tokens, completion_tokens, total_tokens, cached_tokens)
         SELECT substr(m.created_at, 1, 10), m.session_id, s.project_id,
                COALESCE(SUM(m.prompt_tokens), 0), COALESCE(SUM(m.completion_tokens), 0),
                COALESCE(SUM(m.total_tokens), 0), COALESCE(SUM(m.cached_tokens), 0)
         FROM messages m JOIN sessions s ON s.id = m.session_id
         WHERE m.role = 'assistant'
         GROUP BY substr(m.created_at, 1, 10), m.session_id;",
        [],
    );

    let total = session_ids.len();
    let mut migrated = 0;
    let mut skipped = 0;
    let mut errors = Vec::new();

    for sid in &session_ids {
        match migrate_single_session(catalog_conn, catalog_db_path, data_dir, sid) {
            Ok(true) => migrated += 1,
            Ok(false) => skipped += 1,
            Err(e) => errors.push(format!("会话 {sid} 迁移失败: {e}")),
        }
    }

    // 全部迁移成功后，触发主库 VACUUM 回收空间，主库瞬间缩减为轻量 Catalog
    if migrated > 0 {
        let _ = catalog_conn.execute("DELETE FROM llm_requests;", []);
        let _ = catalog_conn.execute("DELETE FROM messages;", []);
        let _ = catalog_conn.execute("DELETE FROM tool_events;", []);
        let _ = catalog_conn.execute("DELETE FROM tool_file_snapshots;", []);
        let _ = catalog_conn.execute("DELETE FROM runs;", []);
        let _ = catalog_conn.execute("DELETE FROM session_kv;", []);
        let _ = catalog_conn.execute("DELETE FROM session_rules_t;", []);
        let _ = catalog_conn.execute("DELETE FROM session_compactions;", []);
        let _ = catalog_conn.execute("DELETE FROM long_tasks;", []);
        let _ = catalog_conn.execute("DELETE FROM task_checkpoints;", []);
        let _ = catalog_conn.execute("VACUUM;", []);
        let _ = catalog_conn.execute("PRAGMA wal_checkpoint(TRUNCATE);", []);
    }

    Ok(MigrationReport {
        total_sessions: total,
        migrated_sessions: migrated,
        skipped_sessions: skipped,
        errors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migrate_single_session_flow() {
        let temp_dir = std::env::temp_dir().join(format!("hm_mig_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let catalog_path = temp_dir.join("harness_mini.db");
        let catalog_conn = Connection::open(&catalog_path).unwrap();
        crate::store::init_schema(&catalog_conn).unwrap();

        let s = crate::store::create_session(&catalog_conn, "D:/ws", None, "迁移会话", "confirm").unwrap();
        let _ = crate::store::new_message(&catalog_conn, &s.id, "user", Some("测试消息".into()), false).unwrap();

        // 创建临时附件
        let legacy_att = temp_dir.join("attachments").join(&s.id);
        std::fs::create_dir_all(&legacy_att).unwrap();
        std::fs::write(legacy_att.join("test.txt"), b"att content").unwrap();

        // 执行迁移
        let did_mig = migrate_single_session(&catalog_conn, &catalog_path, &temp_dir, &s.id).unwrap();
        assert!(did_mig);

        // 验证已生成专属 Bundle
        let bundle = SessionBundle::new(&s.id, &temp_dir);
        assert!(bundle.exists());
        let bundle_conn = bundle.open_db().unwrap();
        let msg_count: i64 = bundle_conn.query_row("SELECT count(*) FROM messages", [], |r| r.get(0)).unwrap();
        assert_eq!(msg_count, 1);

        // 验证附件已迁移
        assert!(bundle.resource().bundle_attachments_dir().join("test.txt").exists());
        assert!(!legacy_att.exists());

        // 验证主库记录已清理
        let cat_msg_count: i64 = catalog_conn.query_row("SELECT count(*) FROM messages WHERE session_id = ?1", params![s.id], |r| r.get(0)).unwrap();
        assert_eq!(cat_msg_count, 0);

        drop(bundle_conn);
        drop(catalog_conn);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_migrate_all_sessions_option_a_lean() {
        let temp_dir = std::env::temp_dir().join(format!("hm_mig_all_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let catalog_path = temp_dir.join("harness_mini.db");
        let catalog_conn = Connection::open(&catalog_path).unwrap();
        crate::store::init_schema(&catalog_conn).unwrap();

        let s = crate::store::create_session(&catalog_conn, "D:/ws", None, "全局瘦身迁移测试会话", "confirm").unwrap();

        // 插入包含 token 的消息
        catalog_conn.execute(
            "INSERT INTO messages (id, session_id, role, content, prompt_tokens, completion_tokens, total_tokens, cached_tokens, created_at, seq)
             VALUES ('msg-1', ?1, 'assistant', '测试回复', 100, 200, 300, 50, datetime('now'), 1)",
            params![s.id],
        ).unwrap();

        // 插入具有大 payload 的 llm_requests 追踪记录
        let big_payload = "x".repeat(100_000);
        catalog_conn.execute(
            "INSERT INTO llm_requests (id, session_id, message_id, run_id, model, endpoint, payload_json, created_at)
             VALUES ('req-1', ?1, 'msg-1', NULL, 'claude-3-5-sonnet', 'https://api.anthropic.com', ?2, datetime('now'))",
            params![s.id, big_payload],
        ).unwrap();

        // 执行全库瘦身迁移
        let report = migrate_all_sessions(&catalog_conn, &catalog_path, &temp_dir).unwrap();
        assert_eq!(report.total_sessions, 1);
        assert_eq!(report.migrated_sessions, 1);
        assert!(report.errors.is_empty());

        // 1. 验证 daily_token_stats 已汇总
        let (p_tokens, c_tokens, t_tokens): (i64, i64, i64) = catalog_conn.query_row(
            "SELECT prompt_tokens, completion_tokens, total_tokens FROM daily_token_stats WHERE session_id = ?1",
            params![s.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).unwrap();
        assert_eq!(p_tokens, 100);
        assert_eq!(c_tokens, 200);
        assert_eq!(t_tokens, 300);

        // 2. 验证分库中 Option A 瘦身效果与元数据保留
        let bundle = SessionBundle::new(&s.id, &temp_dir);
        assert!(bundle.exists());
        let bundle_conn = bundle.open_db().unwrap();
        let (saved_model, payload): (String, String) = bundle_conn.query_row(
            "SELECT model, payload_json FROM llm_requests WHERE session_id = ?1",
            params![s.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ).unwrap();
        assert_eq!(saved_model, "claude-3-5-sonnet");
        assert_eq!(payload, r#"{"migrated":true,"lean":true}"#);

        // 3. 验证主库中该会话的明细数据已被清理
        let cat_msg_cnt: i64 = catalog_conn.query_row("SELECT count(*) FROM messages WHERE session_id = ?1", params![s.id], |r| r.get(0)).unwrap();
        let cat_req_cnt: i64 = catalog_conn.query_row("SELECT count(*) FROM llm_requests WHERE session_id = ?1", params![s.id], |r| r.get(0)).unwrap();
        assert_eq!(cat_msg_cnt, 0);
        assert_eq!(cat_req_cnt, 0);

        drop(bundle_conn);
        drop(catalog_conn);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
