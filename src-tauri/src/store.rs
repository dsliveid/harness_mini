use crate::models::*;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

pub fn open_db(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA busy_timeout = 5000;
         PRAGMA synchronous = NORMAL;",
    )
    .map_err(|e| e.to_string())?;
    init_schema(&conn)?;
    // 仅在无活跃后台守护进程时才执行遗留状态清理，避免误杀正在运行的 Agent
    let is_daemon_alive = path.parent().and_then(|dir| {
        let p = dir.join("daemon.json");
        let s = std::fs::read_to_string(p).ok()?;
        let info: serde_json::Value = serde_json::from_str(&s).ok()?;
        let port = info.get("port")?.as_u64()? as u16;
        let stream = std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
            std::time::Duration::from_millis(200),
        );
        Some(stream.is_ok())
    }).unwrap_or(false);

    if !is_daemon_alive {
        let _ = cleanup_orphaned_running_states(&conn);
    }
    Ok(conn)
}

/// 初始化表结构（可在内存库上复用，供测试）
pub fn init_schema(conn: &Connection) -> Result<(), String> {
    init(conn)
}

fn init(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        PRAGMA journal_mode=WAL;
        PRAGMA foreign_keys=ON;

        CREATE TABLE IF NOT EXISTS settings (
          key TEXT PRIMARY KEY,
          value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS projects (
          id TEXT PRIMARY KEY,
          name TEXT NOT NULL,
          path TEXT,
          pinned INTEGER NOT NULL DEFAULT 0,
          created_at TEXT NOT NULL,
          constraints TEXT NOT NULL DEFAULT '',
          sop_verify_cmd TEXT,
          sop_enabled INTEGER NOT NULL DEFAULT 1,
          plan_mode TEXT NOT NULL DEFAULT 'standard'
        );

        CREATE TABLE IF NOT EXISTS project_links (
          id TEXT PRIMARY KEY,
          project_id TEXT NOT NULL,
          path TEXT NOT NULL,
          description TEXT NOT NULL DEFAULT '',
          created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_project_links_project ON project_links(project_id);

        CREATE TABLE IF NOT EXISTS sessions (
          id TEXT PRIMARY KEY,
          title TEXT NOT NULL,
          workspace_path TEXT NOT NULL,
          access_mode TEXT,
          project_id TEXT,
          status TEXT NOT NULL DEFAULT 'active',
          last_message_at TEXT,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS messages (
          id TEXT PRIMARY KEY,
          session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
          run_id TEXT,
          seq INTEGER NOT NULL,
          role TEXT NOT NULL,
          content TEXT,
          reasoning TEXT,
          tool_calls_json TEXT,
          tool_call_id TEXT,
          queued INTEGER NOT NULL DEFAULT 0,
          usage_json TEXT,
          created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, seq);

        CREATE TABLE IF NOT EXISTS session_rules_t (
          id TEXT PRIMARY KEY,
          session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
          kind TEXT NOT NULL,
          pattern TEXT NOT NULL,
          created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_session_rules_session ON session_rules_t(session_id);

        CREATE TABLE IF NOT EXISTS tool_events (
          id TEXT PRIMARY KEY,
          message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
          tool_name TEXT NOT NULL,
          tool_call_id TEXT,
          params_json TEXT NOT NULL,
          result_text TEXT,
          status TEXT NOT NULL,
          approval_scope TEXT,
          created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_tool_events_message ON tool_events(message_id);

        CREATE TABLE IF NOT EXISTS runs (
          id TEXT PRIMARY KEY,
          session_id TEXT NOT NULL,
          trigger_type TEXT NOT NULL DEFAULT 'manual',
          status TEXT NOT NULL,
          started_at TEXT NOT NULL,
          ended_at TEXT
        );

        CREATE TABLE IF NOT EXISTS session_kv (
          session_id TEXT NOT NULL,
          key TEXT NOT NULL,
          value TEXT NOT NULL,
          PRIMARY KEY (session_id, key)
        );

        CREATE TABLE IF NOT EXISTS secrets (
          id TEXT PRIMARY KEY,
          enc TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS agent_growths (
          id TEXT PRIMARY KEY,
          project_id TEXT,
          session_id TEXT,
          message_id TEXT,
          run_id TEXT,
          trigger_type TEXT NOT NULL,
          trigger_context TEXT NOT NULL,
          reflection_thought TEXT NOT NULL,
          category TEXT NOT NULL,
          title TEXT NOT NULL,
          rule_content TEXT NOT NULL,
          status TEXT NOT NULL DEFAULT 'proposed',
          applied_count INTEGER NOT NULL DEFAULT 0,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_growths_project ON agent_growths(project_id, status);
        CREATE INDEX IF NOT EXISTS idx_growths_session ON agent_growths(session_id);

        CREATE TABLE IF NOT EXISTS session_compactions (
          id TEXT PRIMARY KEY,
          session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
          start_seq INTEGER NOT NULL,
          end_seq INTEGER NOT NULL,
          summary_markdown TEXT NOT NULL,
          tokens_before INTEGER NOT NULL,
          created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_session_compactions_session ON session_compactions(session_id);

        CREATE TABLE IF NOT EXISTS long_tasks (
          id TEXT PRIMARY KEY,
          session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
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
        CREATE INDEX IF NOT EXISTS idx_long_tasks_session ON long_tasks(session_id, status);

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
        CREATE INDEX IF NOT EXISTS idx_task_checkpoints_task ON task_checkpoints(task_id, step_number);

        CREATE TABLE IF NOT EXISTS tool_file_snapshots (
          id TEXT PRIMARY KEY,
          session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
          message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
          tool_event_id TEXT NOT NULL REFERENCES tool_events(id) ON DELETE CASCADE,
          file_path TEXT NOT NULL,
          before_hash TEXT,
          after_hash TEXT NOT NULL,
          is_new_file INTEGER NOT NULL DEFAULT 0,
          reverted_at TEXT,
          created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_snapshots_event ON tool_file_snapshots(tool_event_id);
        CREATE INDEX IF NOT EXISTS idx_snapshots_message ON tool_file_snapshots(message_id);
        CREATE INDEX IF NOT EXISTS idx_snapshots_session ON tool_file_snapshots(session_id, created_at);
        "#,
    )
    .map_err(|e| e.to_string())?;
    // 旧库迁移：会话访问模式回填为具体值。访问模式已改为纯会话级（无“跟随全局”
    // 状态），遗留的 NULL 用当前全局默认值填充，未设置过则用 confirm。
    // 回填后运行时代码不再遇到 NULL；此语句在无 NULL 时不影响任何行。
    let global_mode: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![SETTINGS_KEY],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str::<SettingsData>(&s).ok())
        .map(|s| s.global_access_mode)
        .unwrap_or_else(|| "confirm".into());
    conn.execute(
        "UPDATE sessions SET access_mode = ?1 WHERE access_mode IS NULL",
        params![normalize_access_mode(&global_mode)],
    )
    .map_err(|e| e.to_string())?;
    // 审批规则已改为会话级（session_rules_t）：旧的全局限用规则表整体丢弃
    // （规则不再跨对话生效，保留后又无法归属到具体对话，因此不迁移）
    conn.execute("DROP TABLE IF EXISTS approval_rules_t", [])
        .map_err(|e| e.to_string())?;
    // 旧库迁移：projects 补 path / pinned / constraints 列（新建库的 CREATE TABLE 已含该列时跳过）
    ensure_column(conn, "projects", "path", "path TEXT")?;
    ensure_column(conn, "projects", "pinned", "pinned INTEGER NOT NULL DEFAULT 0")?;
    ensure_column(conn, "projects", "constraints", "constraints TEXT NOT NULL DEFAULT ''")?;
    ensure_column(conn, "projects", "sop_verify_cmd", "sop_verify_cmd TEXT")?;
    ensure_column(conn, "projects", "sop_enabled", "sop_enabled INTEGER NOT NULL DEFAULT 1")?;
    ensure_column(conn, "projects", "plan_mode", "plan_mode TEXT NOT NULL DEFAULT 'standard'")?;
    // 临时空间对话字段
    ensure_column(conn, "sessions", "is_temp", "is_temp INTEGER NOT NULL DEFAULT 0")?;
    ensure_column(conn, "sessions", "temp_code", "temp_code TEXT")?;
    ensure_column(conn, "sessions", "temp_root", "temp_root TEXT")?;
    ensure_column(conn, "sessions", "source_workspace", "source_workspace TEXT")?;
    ensure_column(conn, "sessions", "merged_seq", "merged_seq INTEGER")?;
    ensure_column(conn, "sessions", "merged_pending", "merged_pending INTEGER NOT NULL DEFAULT 0")?;
    // 旧库迁移：messages 补 reasoning 列（模型思考过程）
    ensure_column(conn, "messages", "reasoning", "reasoning TEXT")?;
    // Token 消耗与耗时统计字段
    ensure_column(conn, "messages", "prompt_tokens", "prompt_tokens INTEGER DEFAULT 0")?;
    ensure_column(conn, "messages", "completion_tokens", "completion_tokens INTEGER DEFAULT 0")?;
    ensure_column(conn, "messages", "total_tokens", "total_tokens INTEGER DEFAULT 0")?;
    ensure_column(conn, "messages", "cached_tokens", "cached_tokens INTEGER DEFAULT 0")?;
    ensure_column(conn, "messages", "is_estimated", "is_estimated INTEGER DEFAULT 0")?;
    ensure_column(conn, "messages", "duration_ms", "duration_ms INTEGER DEFAULT 0")?;
    ensure_column(conn, "messages", "turn_duration_ms", "turn_duration_ms INTEGER DEFAULT 0")?;
    ensure_column(conn, "runs", "trigger_message_id", "trigger_message_id TEXT")?;
    ensure_column(conn, "runs", "duration_ms", "duration_ms INTEGER DEFAULT 0")?;
    ensure_column(conn, "runs", "total_tokens", "total_tokens INTEGER DEFAULT 0")?;
    ensure_column(conn, "runs", "prompt_tokens", "prompt_tokens INTEGER DEFAULT 0")?;
    ensure_column(conn, "runs", "completion_tokens", "completion_tokens INTEGER DEFAULT 0")?;
    ensure_column(conn, "runs", "cached_tokens", "cached_tokens INTEGER DEFAULT 0")?;
    // 子 Agent 会话关联与角色任务字段
    ensure_column(conn, "sessions", "parent_session_id", "parent_session_id TEXT")?;
    ensure_column(conn, "sessions", "session_type", "session_type TEXT NOT NULL DEFAULT 'main'")?;
    ensure_column(conn, "sessions", "subagent_role", "subagent_role TEXT")?;
    ensure_column(conn, "sessions", "subagent_task", "subagent_task TEXT")?;
    ensure_column(conn, "sessions", "context_token_limit", "context_token_limit INTEGER")?;
    ensure_column(conn, "sessions", "last_reported_msg_id", "last_reported_msg_id TEXT")?;
    ensure_column(conn, "sessions", "auto_report", "auto_report INTEGER DEFAULT 1")?;
    ensure_column(conn, "sessions", "trigger_tool_event_id", "trigger_tool_event_id TEXT")?;
    ensure_column(conn, "sessions", "provider_id", "provider_id TEXT")?;
    ensure_column(conn, "sessions", "model_id", "model_id TEXT")?;
    ensure_column(conn, "sessions", "dispatch_rule", "dispatch_rule TEXT")?;
    ensure_column(conn, "sessions", "image_provider_id", "image_provider_id TEXT")?;
    ensure_column(conn, "sessions", "image_model_id", "image_model_id TEXT")?;
    ensure_column(conn, "sessions", "vision_provider_id", "vision_provider_id TEXT")?;
    ensure_column(conn, "sessions", "vision_model_id", "vision_model_id TEXT")?;
    ensure_column(conn, "sessions", "forked_from_session_id", "forked_from_session_id TEXT")?;
    ensure_column(conn, "sessions", "forked_from_message_id", "forked_from_message_id TEXT")?;
    ensure_column(conn, "sessions", "reasoning_effort", "reasoning_effort TEXT")?;
    ensure_column(conn, "messages", "attachments_json", "attachments_json TEXT")?;
    ensure_column(conn, "tool_events", "subprocess_id", "subprocess_id TEXT")?;
    ensure_column(conn, "messages", "reverted_at", "reverted_at TEXT")?;
    ensure_column(conn, "tool_file_snapshots", "reverted_at", "reverted_at TEXT")?;
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_sessions_trigger_tool ON sessions(trigger_tool_event_id)", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_sessions_forked_from ON sessions(forked_from_session_id)", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_tool_events_subprocess ON tool_events(subprocess_id)", []);
    let _ = backfill_message_tokens(conn);
    Ok(())
}

/// 若表中不存在指定列则补齐（用于旧库升级）
fn ensure_column(conn: &Connection, table: &str, column: &str, decl: &str) -> Result<(), String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({})", table))
        .map_err(|e| e.to_string())?;
    let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let name: String = row.get(1).map_err(|e| e.to_string())?;
        if name == column {
            return Ok(());
        }
    }
    drop(rows);
    drop(stmt);
    conn.execute(&format!("ALTER TABLE {} ADD COLUMN {}", table, decl), [])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 兼容回填：解析既有消息中的 usage_json 并填入结构化 token / duration / cached 列
fn backfill_message_tokens(conn: &Connection) -> Result<(), String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, usage_json, cached_tokens FROM messages \
             WHERE usage_json IS NOT NULL \
               AND usage_json != ''",
        )
        .map_err(|e| e.to_string())?;
    let rows: Vec<(String, String, Option<i64>)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    for (id, uj, existing_cached) in rows {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&uj) {
            let pt = v
                .get("promptTokens")
                .or_else(|| v.get("prompt_tokens"))
                .or_else(|| v.get("inputEst"))
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            let ct = v
                .get("completionTokens")
                .or_else(|| v.get("completion_tokens"))
                .or_else(|| v.get("outputEst"))
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            let tt = v
                .get("totalTokens")
                .or_else(|| v.get("total_tokens"))
                .and_then(|x| x.as_u64())
                .unwrap_or(pt + ct);
            let cached = v
                .get("cachedTokens")
                .or_else(|| v.get("cached_tokens"))
                .or_else(|| v.get("prompt_tokens_details").and_then(|d| d.get("cached_tokens")))
                .or_else(|| v.get("prompt_cache_hit_tokens"))
                .or_else(|| v.get("cache_read_input_tokens"))
                .or_else(|| {
                    v.get("rawUsage").and_then(|u| {
                        u.get("prompt_tokens_details")
                            .and_then(|d| d.get("cached_tokens"))
                            .or_else(|| u.get("prompt_cache_hit_tokens"))
                            .or_else(|| u.get("cache_read_input_tokens"))
                    })
                })
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            let dur = v
                .get("durationMs")
                .or_else(|| v.get("duration_ms"))
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            let is_est = v
                .get("isEstimated")
                .and_then(|x| x.as_bool())
                .unwrap_or(false);

            if existing_cached.unwrap_or(0) == 0 && cached > 0 {
                let _ = conn.execute(
                    "UPDATE messages SET cached_tokens = ?2, is_estimated = ?3 WHERE id = ?1",
                    params![id, cached as i64, is_est as i64],
                );
            }
            if tt > 0 || dur > 0 {
                let _ = conn.execute(
                    "UPDATE messages SET prompt_tokens = CASE WHEN prompt_tokens > 0 THEN prompt_tokens ELSE ?2 END, \
                                        completion_tokens = CASE WHEN completion_tokens > 0 THEN completion_tokens ELSE ?3 END, \
                                        total_tokens = CASE WHEN total_tokens > 0 THEN total_tokens ELSE ?4 END, \
                                        duration_ms = CASE WHEN duration_ms > 0 THEN duration_ms ELSE ?5 END \
                     WHERE id = ?1",
                    params![id, pt as i64, ct as i64, tt as i64, dur as i64],
                );
            }
        }
    }
    Ok(())
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

// ---------- settings ----------

const SETTINGS_KEY: &str = "app_settings";

pub fn get_settings(conn: &Connection) -> Result<SettingsData, String> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![SETTINGS_KEY],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let mut data: SettingsData = match raw {
        Some(s) => serde_json::from_str(&s).unwrap_or_default(),
        None => SettingsData::default(),
    };
    data.normalize();
    Ok(data)
}

/// 读取设置并注入加密 secrets 表中的 API Key（Agent 调用 LLM / UI 显示 / 连接测试用）。
/// 注意：JSON 中的 apiKey 已在保存时清空（真 Key 加密存于 secrets 表），直接用 get_settings 会拿到空 Key。
pub fn get_settings_with_secrets(conn: &Connection, master: &[u8; 32]) -> Result<SettingsData, String> {
    let mut s = get_settings(conn)?;
    for p in s.providers.iter_mut() {
        if p.api_key.is_empty() {
            if let Ok(Some(k)) = secret_get(conn, master, &p.id) {
                p.api_key = k;
            }
        }
    }
    Ok(s)
}

pub fn save_settings(conn: &Connection, data: &SettingsData) -> Result<(), String> {
    let mut clean = data.clone();
    clean.normalize();
    let json = serde_json::to_string(&clean).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO settings(key, value) VALUES(?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![SETTINGS_KEY, json],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------- approval rules（会话级：仅对所属对话生效） ----------

pub fn list_session_rules(conn: &Connection, session_id: &str) -> Result<Vec<ApprovalRule>, String> {
    let mut stmt = conn
        .prepare("SELECT id, session_id, kind, pattern, created_at FROM session_rules_t WHERE session_id = ?1 ORDER BY created_at")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![session_id], |r| {
            Ok(ApprovalRule {
                id: r.get(0)?,
                session_id: r.get(1)?,
                kind: r.get(2)?,
                pattern: r.get(3)?,
                created_at: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// 新增会话规则；同一对话下 kind+pattern 已存在时直接返回已有规则，避免重复堆积
pub fn add_session_rule(
    conn: &Connection,
    session_id: &str,
    kind: &str,
    pattern: &str,
) -> Result<ApprovalRule, String> {
    if let Some(existing) = list_session_rules(conn, session_id)?
        .into_iter()
        .find(|r| r.kind == kind && r.pattern == pattern)
    {
        return Ok(existing);
    }
    let rule = ApprovalRule {
        id: uuid::Uuid::new_v4().to_string(),
        session_id: session_id.to_string(),
        kind: kind.to_string(),
        pattern: pattern.to_string(),
        created_at: now(),
    };
    conn.execute(
        "INSERT INTO session_rules_t(id, session_id, kind, pattern, created_at) VALUES(?1,?2,?3,?4,?5)",
        params![rule.id, rule.session_id, rule.kind, rule.pattern, rule.created_at],
    )
    .map_err(|e| e.to_string())?;
    Ok(rule)
}

/// 删除单条会话规则；返回该对话剩余的规则列表
pub fn delete_session_rule(
    conn: &Connection,
    session_id: &str,
    id: &str,
) -> Result<Vec<ApprovalRule>, String> {
    conn.execute(
        "DELETE FROM session_rules_t WHERE id = ?1 AND session_id = ?2",
        params![id, session_id],
    )
    .map_err(|e| e.to_string())?;
    list_session_rules(conn, session_id)
}

// ---------- projects ----------

/// 路径比较键：去掉尾部分隔符；Windows 下统一分隔符并忽略大小写
fn path_key(p: &str) -> String {
    let t = p.trim_end_matches(|c| c == '/' || c == '\\');
    if cfg!(windows) {
        t.replace('\\', "/").to_lowercase()
    } else {
        t.to_string()
    }
}

pub fn same_path(a: &str, b: &str) -> bool {
    path_key(a) == path_key(b)
}

/// 取路径最后一段非空目录名作为项目名
pub fn dir_name_of(p: &str) -> String {
    let t = p.trim_end_matches(|c| c == '/' || c == '\\');
    let name = t
        .rsplit(|c| c == '/' || c == '\\')
        .find(|s| !s.is_empty())
        .unwrap_or(t);
    if name.is_empty() {
        t.to_string()
    } else {
        name.to_string()
    }
}

pub fn list_projects(conn: &Connection) -> Result<Vec<Project>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT p.id, p.name, p.path, p.pinned, p.created_at, p.constraints,
                    p.sop_verify_cmd, p.sop_enabled,
                    (SELECT MAX(s.last_message_at) FROM sessions s WHERE s.project_id = p.id),
                    COALESCE(p.plan_mode, 'standard')
             FROM projects p",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Project {
                id: r.get(0)?,
                name: r.get(1)?,
                path: r.get(2)?,
                pinned: r.get::<_, Option<i64>>(3)?.unwrap_or(0) != 0,
                created_at: r.get(4)?,
                constraints: r.get(5)?,
                sop_verify_cmd: r.get(6)?,
                sop_enabled: r.get::<_, Option<i64>>(7)?.unwrap_or(1) != 0,
                last_activity_at: r.get(8)?,
                plan_mode: r.get::<_, Option<String>>(9)?.unwrap_or_else(|| "standard".into()),
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

pub fn get_project(conn: &Connection, id: &str) -> Result<Option<Project>, String> {
    conn.query_row(
        "SELECT id, name, path, pinned, created_at, constraints, sop_verify_cmd, sop_enabled, COALESCE(plan_mode, 'standard') FROM projects WHERE id = ?1",
        params![id],
        |r| {
            Ok(Project {
                id: r.get(0)?,
                name: r.get(1)?,
                path: r.get(2)?,
                pinned: r.get::<_, Option<i64>>(3)?.unwrap_or(0) != 0,
                created_at: r.get(4)?,
                constraints: r.get(5)?,
                sop_verify_cmd: r.get(6)?,
                sop_enabled: r.get::<_, Option<i64>>(7)?.unwrap_or(1) != 0,
                last_activity_at: None,
                plan_mode: r.get::<_, Option<String>>(8)?.unwrap_or_else(|| "standard".into()),
            })
        },
    )
    .optional()
    .map_err(|e| e.to_string())
}

pub fn create_project(conn: &Connection, name: &str, path: Option<&str>) -> Result<Project, String> {
    let p = Project {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.to_string(),
        path: path.map(|s| s.to_string()),
        pinned: false,
        created_at: now(),
        last_activity_at: None,
        constraints: String::new(),
        sop_verify_cmd: None,
        sop_enabled: true,
        plan_mode: "standard".to_string(),
    };
    conn.execute(
        "INSERT INTO projects(id, name, path, pinned, created_at, constraints, sop_verify_cmd, sop_enabled, plan_mode) VALUES(?1,?2,?3,0,?4,'',NULL,1,'standard')",
        params![p.id, p.name, p.path, p.created_at],
    )
    .map_err(|e| e.to_string())?;
    Ok(p)
}

/// 按路径查找项目（只读，不创建；路径比较同 same_path）
pub fn find_project_by_path(conn: &Connection, path: &str) -> Result<Option<Project>, String> {
    for p in list_projects(conn)? {
        if let Some(pp) = p.path.as_deref() {
            if same_path(pp, path) {
                return Ok(Some(p));
            }
        }
    }
    Ok(None)
}

/// 工作区即项目：按路径查找项目，不存在则自动创建（项目名取目录名）
pub fn find_or_create_project_by_path(conn: &Connection, path: &str) -> Result<Project, String> {
    if let Some(p) = find_project_by_path(conn, path)? {
        return Ok(p);
    }
    create_project(conn, &dir_name_of(path), Some(path))
}

/// 保存项目约束（Markdown 原文）；首尾空白裁剪，空串表示清除
pub fn set_project_constraints(conn: &Connection, id: &str, constraints: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE projects SET constraints = ?2 WHERE id = ?1",
        params![id, constraints.trim()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 设置项目交付 SOP 自检命令与启用状态
pub fn set_project_sop(
    conn: &Connection,
    id: &str,
    verify_cmd: Option<&str>,
    enabled: bool,
) -> Result<(), String> {
    let cmd = verify_cmd.map(|s| s.trim()).filter(|s| !s.is_empty());
    conn.execute(
        "UPDATE projects SET sop_verify_cmd = ?2, sop_enabled = ?3 WHERE id = ?1",
        params![id, cmd, if enabled { 1 } else { 0 }],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 设置项目任务规划与执行策略模式："standard" | "always_plan" | "always_proceed"
pub fn set_project_plan_mode(
    conn: &Connection,
    id: &str,
    mode: &str,
) -> Result<(), String> {
    let m = match mode {
        "always_plan" | "always_proceed" => mode,
        _ => "standard",
    };
    conn.execute(
        "UPDATE projects SET plan_mode = ?2 WHERE id = ?1",
        params![id, m],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_existing_project_by_path() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let p = create_project(&conn, "harness_mini", Some("D:\\WorkSpace\\harness_mini")).unwrap();
        let found = find_or_create_project_by_path(&conn, "d:/workspace/harness_mini/").unwrap();
        assert_eq!(found.id, p.id);

        // 不同路径创建不同项目，项目名取目录名
        let p2 = find_or_create_project_by_path(&conn, "D:/WorkSpace/other").unwrap();
        assert_ne!(p2.id, p.id);
        assert_eq!(p2.name, "other");
        assert_eq!(p2.path.as_deref(), Some("D:/WorkSpace/other"));

        // 再次查找复用，不重复创建
        let again = find_or_create_project_by_path(&conn, "D:\\WorkSpace\\other").unwrap();
        assert_eq!(again.id, p2.id);
        assert_eq!(list_projects(&conn).unwrap().len(), 2);
    }

    #[test]
    fn dir_name_handles_trailing_separators() {
        assert_eq!(dir_name_of("D:\\a\\b\\"), "b");
        assert_eq!(dir_name_of("/home/user/proj"), "proj");
        assert_eq!(dir_name_of("D:\\"), "D:");
    }

    #[test]
    fn migrate_adds_path_column_to_legacy_projects_table() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE projects (
               id TEXT PRIMARY KEY,
               name TEXT NOT NULL,
               pinned INTEGER NOT NULL DEFAULT 0,
               created_at TEXT NOT NULL
             );",
        )
        .unwrap();
        ensure_column(&conn, "projects", "path", "path TEXT").unwrap();
        ensure_column(&conn, "projects", "constraints", "constraints TEXT NOT NULL DEFAULT ''").unwrap();
        ensure_column(&conn, "projects", "sop_verify_cmd", "sop_verify_cmd TEXT").unwrap();
        ensure_column(&conn, "projects", "sop_enabled", "sop_enabled INTEGER NOT NULL DEFAULT 1").unwrap();
        ensure_column(&conn, "projects", "plan_mode", "plan_mode TEXT NOT NULL DEFAULT 'standard'").unwrap();
        let p = create_project(&conn, "demo", Some("D:\\demo")).unwrap();
        assert_eq!(p.path.as_deref(), Some("D:\\demo"));
        assert_eq!(p.plan_mode, "standard");
    }

    #[test]
    fn test_project_sop_crud() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let p = create_project(&conn, "sop_demo", None).unwrap();
        assert!(p.sop_enabled);
        assert_eq!(p.sop_verify_cmd, None);

        set_project_sop(&conn, &p.id, Some("cargo check"), true).unwrap();
        let updated = get_project(&conn, &p.id).unwrap().unwrap();
        assert_eq!(updated.sop_verify_cmd.as_deref(), Some("cargo check"));
        assert!(updated.sop_enabled);

        set_project_sop(&conn, &p.id, None, false).unwrap();
        let updated2 = get_project(&conn, &p.id).unwrap().unwrap();
        assert_eq!(updated2.sop_verify_cmd, None);
        assert!(!updated2.sop_enabled);
    }

    #[test]
    fn test_project_plan_mode_crud() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let p = create_project(&conn, "plan_mode_demo", None).unwrap();
        assert_eq!(p.plan_mode, "standard");

        set_project_plan_mode(&conn, &p.id, "always_plan").unwrap();
        let updated = get_project(&conn, &p.id).unwrap().unwrap();
        assert_eq!(updated.plan_mode, "always_plan");

        set_project_plan_mode(&conn, &p.id, "always_proceed").unwrap();
        let updated2 = get_project(&conn, &p.id).unwrap().unwrap();
        assert_eq!(updated2.plan_mode, "always_proceed");

        // 非法值 fallback 为 standard
        set_project_plan_mode(&conn, &p.id, "invalid_mode").unwrap();
        let updated3 = get_project(&conn, &p.id).unwrap().unwrap();
        assert_eq!(updated3.plan_mode, "standard");
    }

    #[test]
    fn test_tool_event_lifecycle_and_cleanup() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let s = create_session(&conn, "D:\\ws", None, "test_session", "confirm").unwrap();
        let m = new_message(&conn, &s.id, "assistant", Some("run".into()), false).unwrap();

        let ev = ToolEvent {
            id: "ev-1".into(),
            message_id: m.id.clone(),
            tool_name: "run_command".into(),
            tool_call_id: None,
            params: serde_json::json!({"command": "echo 1"}),
            result_text: None,
            status: "running".into(),
            approval_scope: None,
            created_at: now(),
            subprocess_id: None,
            reverted_at: None,
        };
        insert_tool_event(&conn, &ev).unwrap();

        // 测试根据 event_id 获取事件与关联 session_id
        let fetched = get_tool_event_with_session(&conn, "ev-1").unwrap();
        assert!(fetched.is_some());
        let (found_ev, sid) = fetched.unwrap();
        assert_eq!(found_ev.id, "ev-1");
        assert_eq!(sid, s.id);

        // 测试中止会话时的 fail_open_tool_events
        let aborted = fail_open_tool_events(&conn, &s.id).unwrap();
        assert_eq!(aborted.len(), 1);
        assert_eq!(aborted[0].id, "ev-1");

        let after_abort = get_tool_event_with_session(&conn, "ev-1").unwrap().unwrap().0;
        assert_eq!(after_abort.status, "failed");

        // 测试重启时的 cleanup_orphaned_running_states
        let ev2 = ToolEvent {
            id: "ev-2".into(),
            message_id: m.id.clone(),
            tool_name: "run_command".into(),
            tool_call_id: None,
            params: serde_json::json!({"command": "echo 2"}),
            result_text: None,
            status: "running".into(),
            approval_scope: None,
            created_at: now(),
            subprocess_id: None,
            reverted_at: None,
        };
        insert_tool_event(&conn, &ev2).unwrap();
        cleanup_orphaned_running_states(&conn).unwrap();

        let after_cleanup = get_tool_event_with_session(&conn, "ev-2").unwrap().unwrap().0;
        assert_eq!(after_cleanup.status, "failed");
    }

    #[test]
    fn test_session_context_limit_crud() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();

        // 1. 创建会话后设置自定义 context_token_limit
        let s = create_session(&conn, "D:\\ws", None, "test_session", "confirm").unwrap();
        assert_eq!(s.context_token_limit, None);

        set_session_context_limit(&conn, &s.id, Some(128_000)).unwrap();
        let fetched = get_session(&conn, &s.id).unwrap().unwrap();
        assert_eq!(fetched.context_token_limit, Some(128_000));

        // 2. 更新为新的数值
        set_session_context_limit(&conn, &s.id, Some(256_000)).unwrap();
        let updated = get_session(&conn, &s.id).unwrap().unwrap();
        assert_eq!(updated.context_token_limit, Some(256_000));

        // 3. 清除会话级自定义，恢复为 None (跟随模型默认)
        set_session_context_limit(&conn, &s.id, None).unwrap();
        let cleared = get_session(&conn, &s.id).unwrap().unwrap();
        assert_eq!(cleared.context_token_limit, None);

        // 4. 模拟旧库迁移：无 context_token_limit 列时自动补充
        let legacy_conn = Connection::open_in_memory().unwrap();
        legacy_conn.execute_batch(
            "CREATE TABLE sessions (
               id TEXT PRIMARY KEY,
               title TEXT NOT NULL,
               workspace_path TEXT NOT NULL,
               access_mode TEXT,
               project_id TEXT,
               status TEXT NOT NULL DEFAULT 'active',
               last_message_at TEXT,
               created_at TEXT NOT NULL,
               updated_at TEXT NOT NULL
             );"
        ).unwrap();
        init_schema(&legacy_conn).unwrap();
        let legacy_s = create_session(&legacy_conn, "D:\\ws", None, "legacy", "confirm").unwrap();
        set_session_context_limit(&legacy_conn, &legacy_s.id, Some(64_000)).unwrap();
        let fetched_legacy = get_session(&legacy_conn, &legacy_s.id).unwrap().unwrap();
        assert_eq!(fetched_legacy.context_token_limit, Some(64_000));
    }

    #[test]
    fn legacy_projects_table_gets_constraints_column() {
        let conn = Connection::open_in_memory().unwrap();
        // 模拟旧库：projects 无 path / constraints 列，init_schema 应补齐
        conn.execute_batch(
            "CREATE TABLE projects (
               id TEXT PRIMARY KEY,
               name TEXT NOT NULL,
               pinned INTEGER NOT NULL DEFAULT 0,
               created_at TEXT NOT NULL
             );",
        )
        .unwrap();
        init_schema(&conn).unwrap();
        let p = create_project(&conn, "demo", None).unwrap();
        assert_eq!(p.constraints, "");
        set_project_constraints(&conn, &p.id, "使用 pnpm；禁止改 dist/").unwrap();
        let got = get_project(&conn, &p.id).unwrap().unwrap();
        assert_eq!(got.constraints, "使用 pnpm；禁止改 dist/");
        assert!(list_projects(&conn).unwrap().iter().any(|x| x.id == p.id));
    }

    #[test]
    fn test_collaborator_dispatched_flag_lifecycle() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let p = create_session(&conn, "D:\\ws", None, "parent_session", "confirm").unwrap();
        let c = create_collaborator_session(&conn, &p.id, "designer", "图像生成", "生图", None, "D:\\ws", None, None, true, None, None, None, None, None, None).unwrap();

        // 1. 初始状态未被主进程委派
        let is_dispatched = get_kv(&conn, &c.id, "dispatched_by_parent").unwrap().map(|v| v == "true").unwrap_or(false);
        assert!(!is_dispatched);

        // 2. 主进程委派时标记为 true
        set_kv(&conn, &c.id, "dispatched_by_parent", "true").unwrap();
        let is_dispatched = get_kv(&conn, &c.id, "dispatched_by_parent").unwrap().map(|v| v == "true").unwrap_or(false);
        assert!(is_dispatched);

        // 3. 运行完毕后消费该标记重置为 false
        set_kv(&conn, &c.id, "dispatched_by_parent", "false").unwrap();
        let is_dispatched = get_kv(&conn, &c.id, "dispatched_by_parent").unwrap().map(|v| v == "true").unwrap_or(false);
        assert!(!is_dispatched);
    }

    #[test]
    fn project_pinned_crud_and_legacy_migration() {
        let conn = Connection::open_in_memory().unwrap();
        // 模拟更早旧库：projects 连 pinned 列都没有
        conn.execute_batch(
            "CREATE TABLE projects (
               id TEXT PRIMARY KEY,
               name TEXT NOT NULL,
               created_at TEXT NOT NULL
             );",
        )
        .unwrap();
        init_schema(&conn).unwrap();
        let p = create_project(&conn, "demo", None).unwrap();
        assert!(!p.pinned);

        // 设为固定
        set_project_pinned(&conn, &p.id, true).unwrap();
        let updated = get_project(&conn, &p.id).unwrap().unwrap();
        assert!(updated.pinned);

        // 取消固定
        set_project_pinned(&conn, &p.id, false).unwrap();
        let unpinned = get_project(&conn, &p.id).unwrap().unwrap();
        assert!(!unpinned.pinned);
    }

    #[test]
    fn project_links_crud_and_cleanup_on_remove() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let a = create_project(&conn, "A", Some("D:\\a")).unwrap();
        // B 目录此前作为工作区出现过 → 有实体；C 没有
        let _b = create_project(&conn, "B", Some("D:\\b")).unwrap();

        let l1 = add_project_link(&conn, &a.id, "D:\\b", "依赖其接口").unwrap();
        add_project_link(&conn, &a.id, "D:\\c", "工具库").unwrap();
        assert_eq!(list_project_links(&conn, &a.id).unwrap().len(), 2);

        // 重复目录（分隔符/大小写差异）与自身目录均拒绝
        assert!(add_project_link(&conn, &a.id, "d:/b/", "x").is_err());
        assert!(add_project_link(&conn, &a.id, "D:\\a", "self").is_err());
        assert!(add_project_link(&conn, "no-such", "D:\\x", "x").is_err());

        // 更新：换目录 + 改说明
        let u = update_project_link(&conn, &l1.id, "D:\\c2", "新说明").unwrap();
        assert_eq!(u.path, "D:\\c2");
        assert_eq!(u.description, "新说明");
        // 更新成已关联目录 / 自身目录同样拒绝
        assert!(update_project_link(&conn, &l1.id, "D:\\c", "x").is_err());
        assert!(update_project_link(&conn, &l1.id, "D:\\a", "x").is_err());

        // 删除单条
        delete_project_link(&conn, &l1.id).unwrap();
        assert_eq!(list_project_links(&conn, &a.id).unwrap().len(), 1);

        // 移除项目时清空其关联条目
        add_project_link(&conn, &a.id, "D:\\b", "again").unwrap();
        remove_project(&conn, &a.id).unwrap();
        assert!(list_project_links(&conn, &a.id).unwrap().is_empty());
        assert!(get_project(&conn, &a.id).unwrap().is_none());
    }

    #[test]
    fn find_project_by_path_does_not_create() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        assert!(find_project_by_path(&conn, "D:\\nope").unwrap().is_none());
        assert_eq!(list_projects(&conn).unwrap().len(), 0);
        create_project(&conn, "A", Some("D:\\a")).unwrap();
        assert!(find_project_by_path(&conn, "d:/a/").unwrap().is_some());
    }

    #[test]
    fn session_temp_and_merge_state_roundtrip() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let s = create_session(&conn, "D:\\tmp\\x", None, "t", "confirm").unwrap();
        assert!(!s.is_temp);
        set_session_temp(&conn, &s.id, "abc123", "D:\\data\\temp-project\\abc123", "D:\\proj").unwrap();
        let got = get_session(&conn, &s.id).unwrap().unwrap();
        assert!(got.is_temp);
        assert_eq!(got.temp_code.as_deref(), Some("abc123"));
        assert_eq!(got.temp_root.as_deref(), Some("D:\\data\\temp-project\\abc123"));
        assert_eq!(got.source_workspace.as_deref(), Some("D:\\proj"));

        set_session_merge_state(&conn, &s.id, Some(9), true).unwrap();
        let got = get_session(&conn, &s.id).unwrap().unwrap();
        assert_eq!(got.merged_seq, Some(9));
        assert!(got.merged_pending);

        // 清空临时空间后解除禁止发送，但编辑边界保留
        set_session_merge_state(&conn, &s.id, got.merged_seq, false).unwrap();
        let got = get_session(&conn, &s.id).unwrap().unwrap();
        assert!(!got.merged_pending);
        assert_eq!(got.merged_seq, Some(9));
    }

    #[test]
    fn message_reasoning_roundtrip() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let s = create_session(&conn, ".", None, "t", "confirm").unwrap();
        let m = new_message(&conn, &s.id, "assistant", Some(String::new()), false).unwrap();
        assert_eq!(m.reasoning, None);

        update_message_reasoning(&conn, &m.id, "先分析文件结构，再决定修改方案。").unwrap();
        let got = get_message(&conn, &m.id).unwrap().unwrap();
        assert_eq!(got.reasoning.as_deref(), Some("先分析文件结构，再决定修改方案。"));

        let list = get_messages(&conn, &s.id, None, 10).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].reasoning.as_deref(), Some("先分析文件结构，再决定修改方案。"));
    }

    #[test]
    fn legacy_messages_table_gets_reasoning_column() {
        let conn = Connection::open_in_memory().unwrap();
        // 模拟旧库：messages 无 reasoning 列
        conn.execute_batch(
            "CREATE TABLE messages (
               id TEXT PRIMARY KEY,
               session_id TEXT NOT NULL,
               run_id TEXT,
               seq INTEGER NOT NULL,
               role TEXT NOT NULL,
               content TEXT,
               tool_calls_json TEXT,
               tool_call_id TEXT,
               queued INTEGER NOT NULL DEFAULT 0,
               usage_json TEXT,
               created_at TEXT NOT NULL
             );",
        )
        .unwrap();
        init_schema(&conn).unwrap();

        let s = create_session(&conn, ".", None, "t", "confirm").unwrap();
        let m = new_message(&conn, &s.id, "assistant", Some(String::new()), false).unwrap();
        update_message_reasoning(&conn, &m.id, "迁移后思考过程可写入").unwrap();
        let got = get_message(&conn, &m.id).unwrap().unwrap();
        assert_eq!(got.reasoning.as_deref(), Some("迁移后思考过程可写入"));
    }

    #[test]
    fn session_rules_are_scoped_to_their_session() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let a = create_session(&conn, "D:\\a", None, "a", "confirm").unwrap();
        let b = create_session(&conn, "D:\\b", None, "b", "full_access").unwrap();

        add_session_rule(&conn, &a.id, "command_prefix", "npm").unwrap();
        add_session_rule(&conn, &b.id, "command_prefix", "cargo").unwrap();

        let ra = list_session_rules(&conn, &a.id).unwrap();
        let rb = list_session_rules(&conn, &b.id).unwrap();
        assert_eq!(ra.len(), 1);
        assert_eq!(rb.len(), 1);
        assert_eq!(ra[0].pattern, "npm");
        assert_eq!(ra[0].session_id, a.id);
        assert_eq!(rb[0].pattern, "cargo");
    }

    #[test]
    fn add_session_rule_dedups_same_kind_and_pattern() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let s = create_session(&conn, ".", None, "t", "confirm").unwrap();

        let first = add_session_rule(&conn, &s.id, "command_prefix", "npm").unwrap();
        let again = add_session_rule(&conn, &s.id, "command_prefix", "npm").unwrap();
        assert_eq!(first.id, again.id);
        assert_eq!(list_session_rules(&conn, &s.id).unwrap().len(), 1);

        // 同 pattern 不同 kind 视为两条规则
        add_session_rule(&conn, &s.id, "tool", "npm").unwrap();
        assert_eq!(list_session_rules(&conn, &s.id).unwrap().len(), 2);
    }

    #[test]
    fn delete_session_rule_returns_remaining_and_delete_session_cleans_up() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let s = create_session(&conn, ".", None, "t", "confirm").unwrap();
        let r1 = add_session_rule(&conn, &s.id, "command_prefix", "npm").unwrap();
        add_session_rule(&conn, &s.id, "command_prefix", "cargo").unwrap();

        let left = delete_session_rule(&conn, &s.id, &r1.id).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].pattern, "cargo");

        // 对话删除后规则一并清理，不残留孤儿规则
        delete_session(&conn, &s.id).unwrap();
        assert!(list_session_rules(&conn, &s.id).unwrap().is_empty());
    }

    #[test]
    fn legacy_global_rule_table_is_dropped_on_open() {
        let conn = Connection::open_in_memory().unwrap();
        // 模拟旧库：存在全局限用规则表且已有数据
        conn.execute_batch(
            "CREATE TABLE approval_rules_t (
               id TEXT PRIMARY KEY,
               kind TEXT NOT NULL,
               pattern TEXT NOT NULL,
               scope TEXT NOT NULL DEFAULT 'global',
               created_at TEXT NOT NULL
             );
             INSERT INTO approval_rules_t(id, kind, pattern, scope, created_at)
             VALUES('r1','command_prefix','npm','global','2026-01-01T00:00:00Z');",
        )
        .unwrap();
        init_schema(&conn).unwrap();

        // 旧全局规则整体丢弃（无法归属到具体对话），表结构一并移除
        let left: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='approval_rules_t'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn create_session_stores_normalized_access_mode() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();

        let full = create_session(&conn, "D:\\a", None, "a", "full_access").unwrap();
        assert_eq!(full.access_mode.as_deref(), Some("full_access"));
        assert_eq!(
            get_session(&conn, &full.id).unwrap().unwrap().access_mode.as_deref(),
            Some("full_access")
        );

        // 非法/空值一律规范为 confirm（访问模式无“跟随全局”状态）
        let odd = create_session(&conn, "D:\\b", None, "b", "").unwrap();
        assert_eq!(odd.access_mode.as_deref(), Some("confirm"));
        let odd2 = create_session(&conn, "D:\\c", None, "c", "global").unwrap();
        assert_eq!(odd2.access_mode.as_deref(), Some("confirm"));
    }

    #[test]
    fn set_session_mode_never_writes_null() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let s = create_session(&conn, "D:\\a", None, "a", "confirm").unwrap();

        set_session_mode(&conn, &s.id, "full_access").unwrap();
        assert_eq!(
            get_session(&conn, &s.id).unwrap().unwrap().access_mode.as_deref(),
            Some("full_access")
        );
        // 传入无法识别的值也不会退回 NULL（旧实现的“跟随全局”）
        set_session_mode(&conn, &s.id, "follow_global").unwrap();
        assert_eq!(
            get_session(&conn, &s.id).unwrap().unwrap().access_mode.as_deref(),
            Some("confirm")
        );
    }

    #[test]
    fn legacy_null_access_mode_is_backfilled_with_global_default() {
        let conn = Connection::open_in_memory().unwrap();
        // 模拟旧库：会话访问模式为 NULL（旧实现表示“跟随全局”），settings 中已有全局默认值
        conn.execute_batch(
            "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO settings(key, value) VALUES('app_settings', '{\"globalAccessMode\":\"full_access\"}');
             CREATE TABLE sessions (
               id TEXT PRIMARY KEY,
               title TEXT NOT NULL,
               workspace_path TEXT NOT NULL,
               access_mode TEXT,
               project_id TEXT,
               status TEXT NOT NULL DEFAULT 'active',
               last_message_at TEXT,
               created_at TEXT NOT NULL,
               updated_at TEXT NOT NULL
             );
             INSERT INTO sessions(id, title, workspace_path, access_mode, status, created_at, updated_at)
             VALUES('s1','t','D:\\a',NULL,'active','2026-01-01T00:00:00Z','2026-01-01T00:00:00Z');",
        )
        .unwrap();

        init_schema(&conn).unwrap();

        // 回填为当时的全局默认值，运行时代码不再遇到 NULL
        let got = get_session(&conn, "s1").unwrap().unwrap();
        assert_eq!(got.access_mode.as_deref(), Some("full_access"));
    }

    #[test]
    fn test_fork_session_at_message() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let s = create_session(&conn, "D:\\workspace", None, "主会话", "confirm").unwrap();

        let m1 = Message {
            id: "m1".into(),
            session_id: s.id.clone(),
            run_id: None,
            seq: 1,
            role: "user".into(),
            content: Some("你好".into()),
            reasoning: None,
            tool_calls: None,
            tool_call_id: None,
            queued: false,
            usage: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            tool_events: vec![],
            duration_ms: None,
            turn_duration_ms: None,
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
            cached_tokens: None,
            is_estimated: None,
            attachments: None,
            reverted_at: None,
        };
        insert_message(&conn, &m1).unwrap();

        let tc = serde_json::json!([{"id": "call_1", "type": "function", "function": {"name": "read_file", "arguments": "{}"}}]);
        let m2 = Message {
            id: "m2".into(),
            session_id: s.id.clone(),
            run_id: None,
            seq: 2,
            role: "assistant".into(),
            content: Some("思考中".into()),
            reasoning: Some("我需要读文件".into()),
            tool_calls: Some(tc),
            tool_call_id: None,
            queued: false,
            usage: None,
            created_at: "2026-01-01T00:00:01Z".into(),
            tool_events: vec![],
            duration_ms: None,
            turn_duration_ms: None,
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
            cached_tokens: None,
            is_estimated: None,
            attachments: None,
            reverted_at: None,
        };
        insert_message(&conn, &m2).unwrap();

        let ev1 = ToolEvent {
            id: "ev1".into(),
            message_id: "m2".into(),
            tool_name: "read_file".into(),
            tool_call_id: Some("call_1".into()),
            params: serde_json::json!({"path": "a.txt"}),
            result_text: Some("file contents".into()),
            status: "success".into(),
            approval_scope: None,
            created_at: "2026-01-01T00:00:02Z".into(),
            subprocess_id: None,
            reverted_at: None,
        };
        insert_tool_event(&conn, &ev1).unwrap();

        let m3 = Message {
            id: "m3".into(),
            session_id: s.id.clone(),
            run_id: None,
            seq: 3,
            role: "tool".into(),
            content: Some("file contents".into()),
            reasoning: None,
            tool_calls: None,
            tool_call_id: Some("call_1".into()),
            queued: false,
            usage: None,
            created_at: "2026-01-01T00:00:02Z".into(),
            tool_events: vec![],
            duration_ms: None,
            turn_duration_ms: None,
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
            cached_tokens: None,
            is_estimated: None,
            attachments: None,
            reverted_at: None,
        };
        insert_message(&conn, &m3).unwrap();

        let m4 = Message {
            id: "m4".into(),
            session_id: s.id.clone(),
            run_id: None,
            seq: 4,
            role: "assistant".into(),
            content: Some("这是文件结果".into()),
            reasoning: None,
            tool_calls: None,
            tool_call_id: None,
            queued: false,
            usage: None,
            created_at: "2026-01-01T00:00:03Z".into(),
            tool_events: vec![],
            duration_ms: None,
            turn_duration_ms: None,
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
            cached_tokens: None,
            is_estimated: None,
            attachments: None,
            reverted_at: None,
        };
        insert_message(&conn, &m4).unwrap();

        let m5 = Message {
            id: "m5".into(),
            session_id: s.id.clone(),
            run_id: None,
            seq: 5,
            role: "user".into(),
            content: Some("第二轮问题".into()),
            reasoning: None,
            tool_calls: None,
            tool_call_id: None,
            queued: false,
            usage: None,
            created_at: "2026-01-01T00:00:04Z".into(),
            tool_events: vec![],
            duration_ms: None,
            turn_duration_ms: None,
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
            cached_tokens: None,
            is_estimated: None,
            attachments: None,
            reverted_at: None,
        };
        insert_message(&conn, &m5).unwrap();

        add_session_rule(&conn, &s.id, "tool", "read_file").unwrap();

        // 1. 从 m4 (Assistant) 创建分支
        let forked = fork_session_at_message(&conn, &s.id, "m4", None, true).unwrap();
        assert_eq!(forked.title, "[分支] 主会话");
        assert_eq!(forked.forked_from_session_id.as_deref(), Some(s.id.as_str()));
        assert_eq!(forked.forked_from_message_id.as_deref(), Some("m4"));
        assert_eq!(forked.workspace_path, "D:\\workspace");

        let forked_msgs = all_messages(&conn, &forked.id).unwrap();
        assert_eq!(forked_msgs.len(), 4);
        assert_eq!(forked_msgs[0].seq, 1);
        assert_eq!(forked_msgs[0].role, "user");
        assert_eq!(forked_msgs[1].seq, 2);
        assert_eq!(forked_msgs[1].role, "assistant");
        assert_eq!(forked_msgs[1].tool_events.len(), 1);
        assert_eq!(forked_msgs[1].tool_events[0].tool_name, "read_file");
        assert_eq!(forked_msgs[1].tool_events[0].message_id, forked_msgs[1].id);
        assert_ne!(forked_msgs[1].tool_events[0].id, "ev1");

        assert_eq!(forked_msgs[2].seq, 3);
        assert_eq!(forked_msgs[2].role, "tool");
        assert_eq!(forked_msgs[3].seq, 4);
        assert_eq!(forked_msgs[3].role, "assistant");

        let rules = list_session_rules(&conn, &forked.id).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].pattern, "read_file");

        // 2. 从 m5 (User) 且 include_target=false 创建分支（分叉并修改此提问场景）
        let forked_user = fork_session_at_message(&conn, &s.id, "m5", Some("新探索"), false).unwrap();
        assert_eq!(forked_user.title, "新探索");
        let forked_user_msgs = all_messages(&conn, &forked_user.id).unwrap();
        assert_eq!(forked_user_msgs.len(), 4);
    }

    #[test]
    fn test_create_session_with_models_persists_capability_models() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let s = create_session_with_models(
            &conn,
            "D:\\workspace",
            None,
            "能力模型测试会话",
            "confirm",
            Some("volcengine"),
            Some("doubao-seedream-5.0-lite"),
            Some("openai"),
            Some("gpt-4o"),
            None,
        )
        .unwrap();

        assert_eq!(s.image_provider_id.as_deref(), Some("volcengine"));
        assert_eq!(s.image_model_id.as_deref(), Some("doubao-seedream-5.0-lite"));
        assert_eq!(s.vision_provider_id.as_deref(), Some("openai"));
        assert_eq!(s.vision_model_id.as_deref(), Some("gpt-4o"));

        // 从数据库重新读取验证
        let loaded = get_session(&conn, &s.id).unwrap().unwrap();
        assert_eq!(loaded.image_provider_id.as_deref(), Some("volcengine"));
        assert_eq!(loaded.image_model_id.as_deref(), Some("doubao-seedream-5.0-lite"));
        assert_eq!(loaded.vision_provider_id.as_deref(), Some("openai"));
        assert_eq!(loaded.vision_model_id.as_deref(), Some("gpt-4o"));
    }

    #[test]
    fn test_tool_file_snapshots_crud() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();

        let s = create_session(&conn, "D:\\workspace", None, "快照测试会话", "confirm").unwrap();
        let m = new_message(&conn, &s.id, "assistant", Some("修改代码".into()), false).unwrap();
        let ev = ToolEvent {
            id: "ev-snap-1".into(),
            message_id: m.id.clone(),
            tool_name: "edit_file".into(),
            tool_call_id: Some("call-1".into()),
            params: serde_json::json!({"path": "src/main.rs"}),
            result_text: Some("ok".into()),
            status: "success".into(),
            approval_scope: Some("none".into()),
            created_at: now(),
            subprocess_id: None,
            reverted_at: None,
        };
        insert_tool_event(&conn, &ev).unwrap();

        let snap = ToolFileSnapshot {
            id: "snap-1".into(),
            session_id: s.id.clone(),
            message_id: m.id.clone(),
            tool_event_id: ev.id.clone(),
            file_path: "src/main.rs".into(),
            before_hash: Some("hash_before_123".into()),
            after_hash: "hash_after_456".into(),
            is_new_file: false,
            reverted_at: None,
            created_at: now(),
        };

        insert_tool_file_snapshot(&conn, &snap).unwrap();

        let msg_snaps = list_snapshots_for_message(&conn, &m.id).unwrap();
        assert_eq!(msg_snaps.len(), 1);
        assert_eq!(msg_snaps[0].file_path, "src/main.rs");
        assert_eq!(msg_snaps[0].before_hash.as_deref(), Some("hash_before_123"));
        assert_eq!(msg_snaps[0].after_hash, "hash_after_456");
        assert_eq!(msg_snaps[0].reverted_at, None);

        let ev_snaps = list_snapshots_for_event(&conn, &ev.id).unwrap();
        assert_eq!(ev_snaps.len(), 1);

        let sess_snaps = list_snapshots_for_session(&conn, &s.id).unwrap();
        assert_eq!(sess_snaps.len(), 1);

        // 标记已撤回
        mark_snapshots_reverted_for_message(&conn, &m.id, Some("2026-09-24T16:00:00Z")).unwrap();
        let updated_snaps = list_snapshots_for_message(&conn, &m.id).unwrap();
        assert_eq!(updated_snaps[0].reverted_at.as_deref(), Some("2026-09-24T16:00:00Z"));

        let updated_msg = get_message(&conn, &m.id).unwrap().unwrap();
        assert_eq!(updated_msg.reverted_at.as_deref(), Some("2026-09-24T16:00:00Z"));

        // 恢复重做 (revert_at 置为 None)
        mark_snapshots_reverted_for_message(&conn, &m.id, None).unwrap();
        let redone_snaps = list_snapshots_for_message(&conn, &m.id).unwrap();
        assert_eq!(redone_snaps[0].reverted_at, None);
        let redone_msg = get_message(&conn, &m.id).unwrap().unwrap();
        assert_eq!(redone_msg.reverted_at, None);
    }

    #[test]
    fn test_turn_snapshots_and_edit_resend_revert_flow() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();

        let s = create_session(&conn, "D:\\workspace", None, "test", "confirm").unwrap();
        let u = new_message(&conn, &s.id, "user", Some("请帮我修改代码".into()), false).unwrap();

        // 模拟多步 assistant 轮次，共享同一 run_id
        let a1 = new_message(&conn, &s.id, "assistant", Some("正在修改 step 1".into()), false).unwrap();
        set_message_run_id(&conn, &a1.id, "run-turn-100").unwrap();

        let a2 = new_message(&conn, &s.id, "assistant", Some("正在修改 step 2".into()), false).unwrap();
        set_message_run_id(&conn, &a2.id, "run-turn-100").unwrap();

        let a3 = new_message(&conn, &s.id, "assistant", Some("修改已完成交付".into()), false).unwrap();
        set_message_run_id(&conn, &a3.id, "run-turn-100").unwrap();

        let ev1 = ToolEvent {
            id: "ev-1".into(),
            message_id: a1.id.clone(),
            tool_name: "edit_file".into(),
            tool_call_id: Some("call-1".into()),
            params: serde_json::json!({"path": "src/a.rs"}),
            result_text: Some("ok".into()),
            status: "success".into(),
            approval_scope: Some("none".into()),
            created_at: now(),
            subprocess_id: None,
            reverted_at: None,
        };
        let ev2 = ToolEvent {
            id: "ev-2".into(),
            message_id: a2.id.clone(),
            tool_name: "edit_file".into(),
            tool_call_id: Some("call-2".into()),
            params: serde_json::json!({"path": "src/b.rs"}),
            result_text: Some("ok".into()),
            status: "success".into(),
            approval_scope: Some("none".into()),
            created_at: now(),
            subprocess_id: None,
            reverted_at: None,
        };
        insert_tool_event(&conn, &ev1).unwrap();
        insert_tool_event(&conn, &ev2).unwrap();

        // 为 a1 与 a2 插入修改快照（a3 仅交付纯文本，无快照）
        let snap1 = ToolFileSnapshot {
            id: "snap-1".into(),
            session_id: s.id.clone(),
            message_id: a1.id.clone(),
            tool_event_id: "ev-1".into(),
            file_path: "src/a.rs".into(),
            before_hash: Some("bh1".into()),
            after_hash: "ah1".into(),
            is_new_file: false,
            reverted_at: None,
            created_at: "2026-09-24T18:00:00Z".into(),
        };
        let snap2 = ToolFileSnapshot {
            id: "snap-2".into(),
            session_id: s.id.clone(),
            message_id: a2.id.clone(),
            tool_event_id: "ev-2".into(),
            file_path: "src/b.rs".into(),
            before_hash: Some("bh2".into()),
            after_hash: "ah2".into(),
            is_new_file: false,
            reverted_at: None,
            created_at: "2026-09-24T18:01:00Z".into(),
        };
        insert_tool_file_snapshot(&conn, &snap1).unwrap();
        insert_tool_file_snapshot(&conn, &snap2).unwrap();

        // 1. 测试从纯文本交付消息 a3 查询整轮快照：应聚合 a1 与 a2 的全部 2 个快照
        let turn_snaps_from_a3 = list_snapshots_for_turn(&conn, &a3.id).unwrap();
        assert_eq!(turn_snaps_from_a3.len(), 2);
        assert_eq!(turn_snaps_from_a3[0].file_path, "src/a.rs");
        assert_eq!(turn_snaps_from_a3[1].file_path, "src/b.rs");

        // 2. 测试从 a1 查询整轮快照：同样聚合全轮快照
        let turn_snaps_from_a1 = list_snapshots_for_turn(&conn, &a1.id).unwrap();
        assert_eq!(turn_snaps_from_a1.len(), 2);

        // 3. 测试整轮软撤回与重做广播
        let affected = mark_snapshots_reverted_for_turn(&conn, &a3.id, Some("2026-09-24T18:02:00Z")).unwrap();
        assert!(affected.contains(&a1.id));
        assert!(affected.contains(&a2.id));
        assert!(affected.contains(&a3.id));
        assert!(affected.contains(&u.id));

        let updated_a3 = get_message(&conn, &a3.id).unwrap().unwrap();
        assert_eq!(updated_a3.reverted_at.as_deref(), Some("2026-09-24T18:02:00Z"));
        let updated_u = get_message(&conn, &u.id).unwrap().unwrap();
        assert_eq!(updated_u.reverted_at.as_deref(), Some("2026-09-24T18:02:00Z"));

        // 4. 测试重新编辑提问时的后序快照获取（按 LIFO 逆序返回）
        mark_snapshots_reverted_for_turn(&conn, &a3.id, None).unwrap();
        let redone_u = get_message(&conn, &u.id).unwrap().unwrap();
        assert_eq!(redone_u.reverted_at, None);
        let snaps_after_u_active = list_snapshots_after_seq(&conn, &s.id, u.seq).unwrap();
        assert_eq!(snaps_after_u_active.len(), 2);
        assert_eq!(snaps_after_u_active[0].file_path, "src/b.rs"); // 后生成的排在前面 (LIFO)
        assert_eq!(snaps_after_u_active[1].file_path, "src/a.rs");
    }
}

pub fn remove_project(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute("UPDATE sessions SET project_id = NULL WHERE project_id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM project_links WHERE project_id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM projects WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_project_pinned(conn: &Connection, id: &str, pinned: bool) -> Result<(), String> {
    let affected = conn
        .execute(
            "UPDATE projects SET pinned = ?2 WHERE id = ?1",
            params![id, pinned as i64],
        )
        .map_err(|e| e.to_string())?;
    if affected == 0 {
        return Err("项目不存在".to_string());
    }
    Ok(())
}

// ---------- project links（关联项目） ----------

fn row_to_link(r: &rusqlite::Row) -> rusqlite::Result<ProjectLink> {
    Ok(ProjectLink {
        id: r.get(0)?,
        project_id: r.get(1)?,
        path: r.get(2)?,
        description: r.get(3)?,
        created_at: r.get(4)?,
    })
}

const LINK_COLS: &str = "id, project_id, path, description, created_at";

pub fn list_project_links(conn: &Connection, project_id: &str) -> Result<Vec<ProjectLink>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {LINK_COLS} FROM project_links WHERE project_id = ?1 ORDER BY created_at ASC, rowid ASC"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![project_id], row_to_link)
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

pub fn get_project_link(conn: &Connection, id: &str) -> Result<Option<ProjectLink>, String> {
    conn.query_row(
        &format!("SELECT {LINK_COLS} FROM project_links WHERE id = ?1"),
        params![id],
        row_to_link,
    )
    .optional()
    .map_err(|e| e.to_string())
}

pub fn add_project_link(
    conn: &Connection,
    project_id: &str,
    path: &str,
    description: &str,
) -> Result<ProjectLink, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("关联目录不能为空".into());
    }
    let proj = get_project(conn, project_id)?.ok_or("项目不存在")?;
    // 同一项目下同一目录不可重复关联；也不允许关联自身目录
    if let Some(self_path) = proj.path {
        if same_path(&self_path, path) {
            return Err("不能关联项目自身的目录".into());
        }
    }
    if list_project_links(conn, project_id)?
        .iter()
        .any(|l| same_path(&l.path, path))
    {
        return Err("该目录已关联".into());
    }
    let l = ProjectLink {
        id: uuid::Uuid::new_v4().to_string(),
        project_id: project_id.to_string(),
        path: path.to_string(),
        description: description.trim().to_string(),
        created_at: now(),
    };
    conn.execute(
        "INSERT INTO project_links(id, project_id, path, description, created_at) VALUES(?1,?2,?3,?4,?5)",
        params![l.id, l.project_id, l.path, l.description, l.created_at],
    )
    .map_err(|e| e.to_string())?;
    Ok(l)
}

/// 更新关联条目（目录与说明均整体替换）
pub fn update_project_link(
    conn: &Connection,
    id: &str,
    path: &str,
    description: &str,
) -> Result<ProjectLink, String> {
    let old = get_project_link(conn, id)?.ok_or("关联条目不存在")?;
    let path = path.trim();
    if path.is_empty() {
        return Err("关联目录不能为空".into());
    }
    if let Some(self_path) = get_project(conn, &old.project_id)?.and_then(|p| p.path) {
        if same_path(&self_path, path) {
            return Err("不能关联项目自身的目录".into());
        }
    }
    if list_project_links(conn, &old.project_id)?
        .iter()
        .any(|l| l.id != old.id && same_path(&l.path, path))
    {
        return Err("该目录已关联".into());
    }
    conn.execute(
        "UPDATE project_links SET path = ?2, description = ?3 WHERE id = ?1",
        params![id, path, description.trim()],
    )
    .map_err(|e| e.to_string())?;
    get_project_link(conn, id)?.ok_or_else(|| "关联条目不存在".to_string())
}

pub fn delete_project_link(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM project_links WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------- sessions ----------

fn row_to_session(r: &rusqlite::Row) -> rusqlite::Result<Session> {
    Ok(Session {
        id: r.get(0)?,
        title: r.get(1)?,
        workspace_path: r.get(2)?,
        access_mode: r.get(3)?,
        project_id: r.get(4)?,
        status: r.get(5)?,
        last_message_at: r.get(6)?,
        created_at: r.get(7)?,
        updated_at: r.get(8)?,
        is_temp: r.get::<_, i64>(9)? != 0,
        temp_code: r.get(10)?,
        temp_root: r.get(11)?,
        source_workspace: r.get(12)?,
        merged_seq: r.get(13)?,
        merged_pending: r.get::<_, i64>(14)? != 0,
        total_tokens: None,
        prompt_tokens: None,
        completion_tokens: None,
        cached_tokens: None,
        cache_hit_rate: None,
        parent_session_id: r.get(15)?,
        session_type: r.get::<_, Option<String>>(16)?.unwrap_or_else(|| "main".into()),
        subagent_role: r.get(17)?,
        subagent_task: r.get(18)?,
        context_token_limit: r.get::<_, Option<i64>>(19).ok().flatten().map(|v| v as usize),
        last_reported_msg_id: r.get(20)?,
        auto_report: r.get::<_, Option<i64>>(21)?.map(|v| v != 0),
        trigger_tool_event_id: r.get(22)?,
        provider_id: r.get(23)?,
        model_id: r.get(24)?,
        dispatch_rule: r.get(25)?,
        image_provider_id: r.get(26)?,
        image_model_id: r.get(27)?,
        vision_provider_id: r.get(28)?,
        vision_model_id: r.get(29)?,
        forked_from_session_id: r.get(30)?,
        forked_from_message_id: r.get(31)?,
        reasoning_effort: r.get(32)?,
        last_run_status: None,
    })
}

const SESSION_COLS: &str =
    "id, title, workspace_path, access_mode, project_id, status, last_message_at, created_at, updated_at, \
     is_temp, temp_code, temp_root, source_workspace, merged_seq, merged_pending, \
     parent_session_id, session_type, subagent_role, subagent_task, context_token_limit, \
     last_reported_msg_id, auto_report, trigger_tool_event_id, provider_id, model_id, \
     dispatch_rule, image_provider_id, image_model_id, vision_provider_id, vision_model_id, \
     forked_from_session_id, forked_from_message_id, reasoning_effort";

fn attach_session_tokens(conn: &Connection, sessions: &mut [Session]) -> Result<(), String> {
    if sessions.is_empty() {
        return Ok(());
    }
    let mut stmt = conn
        .prepare(
            "SELECT session_id, \
                    COALESCE(SUM(total_tokens), 0), \
                    COALESCE(SUM(prompt_tokens), 0), \
                    COALESCE(SUM(completion_tokens), 0), \
                    COALESCE(SUM(cached_tokens), 0) \
             FROM messages \
             GROUP BY session_id",
        )
        .map_err(|e| e.to_string())?;
    let map: std::collections::HashMap<String, (u64, u64, u64, u64)> = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                (
                    r.get::<_, i64>(1)? as u64,
                    r.get::<_, i64>(2)? as u64,
                    r.get::<_, i64>(3)? as u64,
                    r.get::<_, i64>(4)? as u64,
                ),
            ))
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    for s in sessions.iter_mut() {
        if let Some((tt, pt, ct, cached)) = map.get(&s.id) {
            s.total_tokens = Some(*tt);
            s.prompt_tokens = Some(*pt);
            s.completion_tokens = Some(*ct);
            s.cached_tokens = Some(*cached);
            s.cache_hit_rate = Some(if *pt > 0 {
                ((*cached as f64 / *pt as f64) * 1000.0).round() / 10.0
            } else {
                0.0
            });
        } else {
            s.total_tokens = Some(0);
            s.prompt_tokens = Some(0);
            s.completion_tokens = Some(0);
            s.cached_tokens = Some(0);
            s.cache_hit_rate = Some(0.0);
        }
    }
    Ok(())
}

fn attach_session_last_runs(conn: &Connection, sessions: &mut [Session]) -> Result<(), String> {
    if sessions.is_empty() {
        return Ok(());
    }
    let mut stmt = conn
        .prepare(
            "SELECT session_id, status FROM (
                SELECT session_id, status, ROW_NUMBER() OVER (PARTITION BY session_id ORDER BY started_at DESC, id DESC) as rn
                FROM runs
            ) WHERE rn = 1",
        )
        .map_err(|e| e.to_string())?;
    let map: std::collections::HashMap<String, String> = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    for s in sessions.iter_mut() {
        s.last_run_status = map.get(&s.id).cloned();
    }
    Ok(())
}

pub fn get_session(conn: &Connection, id: &str) -> Result<Option<Session>, String> {
    let mut s = conn
        .query_row(
            &format!("SELECT {SESSION_COLS} FROM sessions WHERE id = ?1"),
            params![id],
            row_to_session,
        )
        .optional()
        .map_err(|e| e.to_string())?;

    if let Some(ref mut session) = s {
        session.last_run_status = get_last_run_status(conn, id);

        let tokens: Option<(u64, u64, u64, u64)> = conn
            .query_row(
                "SELECT COALESCE(SUM(total_tokens), 0), \
                        COALESCE(SUM(prompt_tokens), 0), \
                        COALESCE(SUM(completion_tokens), 0), \
                        COALESCE(SUM(cached_tokens), 0) \
                 FROM messages WHERE session_id = ?1",
                params![id],
                |r| Ok((
                    r.get::<_, i64>(0)? as u64,
                    r.get::<_, i64>(1)? as u64,
                    r.get::<_, i64>(2)? as u64,
                    r.get::<_, i64>(3)? as u64,
                )),
            )
            .optional()
            .map_err(|e| e.to_string())?;

        if let Some((tt, pt, ct, cached)) = tokens {
            session.total_tokens = Some(tt);
            session.prompt_tokens = Some(pt);
            session.completion_tokens = Some(ct);
            session.cached_tokens = Some(cached);
            session.cache_hit_rate = Some(if pt > 0 {
                ((cached as f64 / pt as f64) * 1000.0).round() / 10.0
            } else {
                0.0
            });
        } else {
            session.total_tokens = Some(0);
            session.prompt_tokens = Some(0);
            session.completion_tokens = Some(0);
            session.cached_tokens = Some(0);
            session.cache_hit_rate = Some(0.0);
        }
    }
    Ok(s)
}

pub fn list_sessions(conn: &Connection, status: &str) -> Result<Vec<Session>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {SESSION_COLS} FROM sessions WHERE status = ?1 AND (parent_session_id IS NULL OR parent_session_id = '') ORDER BY COALESCE(last_message_at, created_at) DESC"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![status], row_to_session)
        .map_err(|e| e.to_string())?;
    let mut sessions = rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
    attach_session_tokens(conn, &mut sessions)?;
    attach_session_last_runs(conn, &mut sessions)?;
    Ok(sessions)
}

pub fn list_subagents(conn: &Connection, parent_session_id: &str) -> Result<Vec<Session>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {SESSION_COLS} FROM sessions WHERE parent_session_id = ?1 ORDER BY created_at ASC"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![parent_session_id], row_to_session)
        .map_err(|e| e.to_string())?;
    let mut sessions = rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
    attach_session_tokens(conn, &mut sessions)?;
    attach_session_last_runs(conn, &mut sessions)?;
    Ok(sessions)
}

pub fn list_collaborators(conn: &Connection, parent_session_id: &str) -> Result<Vec<Session>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {SESSION_COLS} FROM sessions WHERE parent_session_id = ?1 AND session_type = 'collaborator' ORDER BY created_at ASC"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![parent_session_id], row_to_session)
        .map_err(|e| e.to_string())?;
    let mut sessions = rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
    attach_session_tokens(conn, &mut sessions)?;
    attach_session_last_runs(conn, &mut sessions)?;
    Ok(sessions)
}

pub fn list_subprocesses(conn: &Connection, parent_session_id: &str) -> Result<Vec<Session>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {SESSION_COLS} FROM sessions WHERE parent_session_id = ?1 AND session_type IN ('subprocess', 'subagent') ORDER BY created_at ASC"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![parent_session_id], row_to_session)
        .map_err(|e| e.to_string())?;
    let mut sessions = rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
    attach_session_tokens(conn, &mut sessions)?;
    attach_session_last_runs(conn, &mut sessions)?;
    Ok(sessions)
}

/// 查询父会话下的所有衍生子会话（包含 subprocess、subagent、collaborator）
pub fn list_all_child_sessions(conn: &Connection, parent_session_id: &str) -> Result<Vec<Session>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {SESSION_COLS} FROM sessions WHERE parent_session_id = ?1 ORDER BY created_at ASC"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![parent_session_id], row_to_session)
        .map_err(|e| e.to_string())?;
    let mut sessions = rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
    attach_session_tokens(conn, &mut sessions)?;
    attach_session_last_runs(conn, &mut sessions)?;
    Ok(sessions)
}


pub fn update_collaborator_watermark(
    conn: &Connection,
    collaborator_id: &str,
    last_reported_msg_id: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE sessions SET last_reported_msg_id = ?2, updated_at = ?3 WHERE id = ?1",
        params![collaborator_id, last_reported_msg_id, now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn update_collaborator_auto_report(
    conn: &Connection,
    collaborator_id: &str,
    auto_report: bool,
) -> Result<(), String> {
    conn.execute(
        "UPDATE sessions SET auto_report = ?2, updated_at = ?3 WHERE id = ?1",
        params![collaborator_id, if auto_report { 1 } else { 0 }, now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn create_session(
    conn: &Connection,
    workspace_path: &str,
    project_id: Option<&str>,
    title: &str,
    access_mode: &str,
) -> Result<Session, String> {
    create_session_with_models(
        conn,
        workspace_path,
        project_id,
        title,
        access_mode,
        None,
        None,
        None,
        None,
        None,
    )
}

pub fn create_session_with_models(
    conn: &Connection,
    workspace_path: &str,
    project_id: Option<&str>,
    title: &str,
    access_mode: &str,
    image_provider_id: Option<&str>,
    image_model_id: Option<&str>,
    vision_provider_id: Option<&str>,
    vision_model_id: Option<&str>,
    reasoning_effort: Option<&str>,
) -> Result<Session, String> {
    let t = now();
    let s = Session {
        id: uuid::Uuid::new_v4().to_string(),
        title: title.to_string(),
        workspace_path: workspace_path.to_string(),
        // 访问模式为会话级：创建时必须给定具体值（由前端按“上一条对话”继承或传默认值）
        access_mode: Some(normalize_access_mode(access_mode)),
        project_id: project_id.map(|s| s.to_string()),
        status: "active".into(),
        last_message_at: Some(t.clone()),
        created_at: t.clone(),
        updated_at: t,
        is_temp: false,
        temp_code: None,
        temp_root: None,
        source_workspace: None,
        merged_seq: None,
        merged_pending: false,
        total_tokens: Some(0),
        prompt_tokens: Some(0),
        completion_tokens: Some(0),
        cached_tokens: Some(0),
        cache_hit_rate: Some(0.0),
        parent_session_id: None,
        session_type: "main".into(),
        subagent_role: None,
        subagent_task: None,
        context_token_limit: None,
        last_reported_msg_id: None,
        auto_report: None,
        trigger_tool_event_id: None,
        provider_id: None,
        model_id: None,
        dispatch_rule: None,
        image_provider_id: image_provider_id.map(|s| s.to_string()),
        image_model_id: image_model_id.map(|s| s.to_string()),
        vision_provider_id: vision_provider_id.map(|s| s.to_string()),
        vision_model_id: vision_model_id.map(|s| s.to_string()),
        forked_from_session_id: None,
        forked_from_message_id: None,
        reasoning_effort: reasoning_effort.map(|s| s.to_string()),
        last_run_status: None,
    };
    conn.execute(
        "INSERT INTO sessions(
            id, title, workspace_path, access_mode, project_id, status, 
            last_message_at, created_at, updated_at, 
            parent_session_id, session_type, subagent_role, subagent_task,
            last_reported_msg_id, auto_report,
            image_provider_id, image_model_id, vision_provider_id, vision_model_id,
            reasoning_effort
         ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,NULL,'main',NULL,NULL,NULL,NULL,?10,?11,?12,?13,?14)",
        params![
            s.id,
            s.title,
            s.workspace_path,
            s.access_mode,
            s.project_id,
            s.status,
            s.last_message_at,
            s.created_at,
            s.updated_at,
            s.image_provider_id,
            s.image_model_id,
            s.vision_provider_id,
            s.vision_model_id,
            s.reasoning_effort,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(s)
}

pub fn create_collaborator_session(
    conn: &Connection,
    parent_session_id: &str,
    role: &str,
    title: &str,
    task: &str,
    dispatch_rule: Option<&str>,
    workspace_path: &str,
    access_mode: Option<&str>,
    project_id: Option<&str>,
    auto_report: bool,
    provider_id: Option<&str>,
    model_id: Option<&str>,
    image_provider_id: Option<&str>,
    image_model_id: Option<&str>,
    vision_provider_id: Option<&str>,
    vision_model_id: Option<&str>,
) -> Result<Session, String> {
    let t = now();
    let norm_mode = access_mode.map(normalize_access_mode).unwrap_or_else(|| "confirm".into());
    let s = Session {
        id: uuid::Uuid::new_v4().to_string(),
        title: title.to_string(),
        workspace_path: workspace_path.to_string(),
        access_mode: Some(norm_mode),
        project_id: project_id.map(|s| s.to_string()),
        status: "active".into(),
        last_message_at: Some(t.clone()),
        created_at: t.clone(),
        updated_at: t,
        is_temp: false,
        temp_code: None,
        temp_root: None,
        source_workspace: None,
        merged_seq: None,
        merged_pending: false,
        total_tokens: Some(0),
        prompt_tokens: Some(0),
        completion_tokens: Some(0),
        cached_tokens: Some(0),
        cache_hit_rate: Some(0.0),
        parent_session_id: Some(parent_session_id.to_string()),
        session_type: "collaborator".into(),
        subagent_role: Some(role.to_string()),
        subagent_task: Some(task.to_string()),
        context_token_limit: None,
        last_reported_msg_id: None,
        auto_report: Some(auto_report),
        trigger_tool_event_id: None,
        provider_id: provider_id.map(|s| s.to_string()),
        model_id: model_id.map(|s| s.to_string()),
        dispatch_rule: dispatch_rule.map(|s| s.to_string()),
        image_provider_id: image_provider_id.map(|s| s.to_string()),
        image_model_id: image_model_id.map(|s| s.to_string()),
        vision_provider_id: vision_provider_id.map(|s| s.to_string()),
        vision_model_id: vision_model_id.map(|s| s.to_string()),
        forked_from_session_id: None,
        forked_from_message_id: None,
        reasoning_effort: None,
        last_run_status: None,
    };
    conn.execute(
        "INSERT INTO sessions(
            id, title, workspace_path, access_mode, project_id, status, 
            last_message_at, created_at, updated_at, 
            parent_session_id, session_type, subagent_role, subagent_task,
            last_reported_msg_id, auto_report, provider_id, model_id,
            dispatch_rule, image_provider_id, image_model_id,
            vision_provider_id, vision_model_id
         ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'collaborator',?11,?12,NULL,?13,?14,?15,?16,?17,?18,?19,?20)",
        params![
            s.id,
            s.title,
            s.workspace_path,
            s.access_mode,
            s.project_id,
            s.status,
            s.last_message_at,
            s.created_at,
            s.updated_at,
            s.parent_session_id,
            s.subagent_role,
            s.subagent_task,
            if auto_report { 1 } else { 0 },
            s.provider_id,
            s.model_id,
            s.dispatch_rule,
            s.image_provider_id,
            s.image_model_id,
            s.vision_provider_id,
            s.vision_model_id,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(s)
}

pub fn update_collaborator_session(
    conn: &Connection,
    collaborator_id: &str,
    title: &str,
    role: &str,
    task: &str,
    dispatch_rule: Option<&str>,
    workspace_path: &str,
    auto_report: bool,
    provider_id: Option<&str>,
    model_id: Option<&str>,
    image_provider_id: Option<&str>,
    image_model_id: Option<&str>,
    vision_provider_id: Option<&str>,
    vision_model_id: Option<&str>,
) -> Result<Session, String> {
    let t = now();
    conn.execute(
        "UPDATE sessions SET 
            title = ?1,
            subagent_role = ?2,
            subagent_task = ?3,
            dispatch_rule = ?4,
            workspace_path = ?5,
            auto_report = ?6,
            provider_id = ?7,
            model_id = ?8,
            image_provider_id = ?9,
            image_model_id = ?10,
            vision_provider_id = ?11,
            vision_model_id = ?12,
            updated_at = ?13
         WHERE id = ?14 AND session_type = 'collaborator'",
        params![
            title,
            role,
            task,
            dispatch_rule,
            workspace_path,
            if auto_report { 1 } else { 0 },
            provider_id,
            model_id,
            image_provider_id,
            image_model_id,
            vision_provider_id,
            vision_model_id,
            t,
            collaborator_id,
        ],
    )
    .map_err(|e| e.to_string())?;

    get_session(conn, collaborator_id)?.ok_or_else(|| "协作者不存在".to_string())
}

pub fn update_session_models(
    conn: &Connection,
    session_id: &str,
    provider_id: Option<&str>,
    model_id: Option<&str>,
    image_provider_id: Option<&str>,
    image_model_id: Option<&str>,
    vision_provider_id: Option<&str>,
    vision_model_id: Option<&str>,
) -> Result<Session, String> {
    let t = now();
    conn.execute(
        "UPDATE sessions SET 
            provider_id = ?1,
            model_id = ?2,
            image_provider_id = ?3,
            image_model_id = ?4,
            vision_provider_id = ?5,
            vision_model_id = ?6,
            updated_at = ?7
         WHERE id = ?8",
        params![
            provider_id,
            model_id,
            image_provider_id,
            image_model_id,
            vision_provider_id,
            vision_model_id,
            t,
            session_id,
        ],
    )
    .map_err(|e| e.to_string())?;

    get_session(conn, session_id)?.ok_or_else(|| "会话不存在".to_string())
}

pub fn set_session_reasoning_effort(
    conn: &Connection,
    session_id: &str,
    reasoning_effort: Option<&str>,
) -> Result<Session, String> {
    let t = now();
    conn.execute(
        "UPDATE sessions SET reasoning_effort = ?1, updated_at = ?2 WHERE id = ?3",
        params![reasoning_effort, t, session_id],
    )
    .map_err(|e| e.to_string())?;

    get_session(conn, session_id)?.ok_or_else(|| "会话不存在".to_string())
}

pub fn create_subprocess_session(
    conn: &Connection,
    parent_session_id: &str,
    role: &str,
    title: &str,
    task: &str,
    workspace_path: &str,
    access_mode: Option<&str>,
    project_id: Option<&str>,
    trigger_tool_event_id: Option<&str>,
) -> Result<Session, String> {
    let t = now();
    let norm_mode = access_mode.map(normalize_access_mode).unwrap_or_else(|| "confirm".into());
    let s = Session {
        id: uuid::Uuid::new_v4().to_string(),
        title: title.to_string(),
        workspace_path: workspace_path.to_string(),
        access_mode: Some(norm_mode),
        project_id: project_id.map(|s| s.to_string()),
        status: "active".into(),
        last_message_at: Some(t.clone()),
        created_at: t.clone(),
        updated_at: t,
        is_temp: false,
        temp_code: None,
        temp_root: None,
        source_workspace: None,
        merged_seq: None,
        merged_pending: false,
        total_tokens: Some(0),
        prompt_tokens: Some(0),
        completion_tokens: Some(0),
        cached_tokens: Some(0),
        cache_hit_rate: Some(0.0),
        parent_session_id: Some(parent_session_id.to_string()),
        session_type: "subprocess".into(),
        subagent_role: Some(role.to_string()),
        subagent_task: Some(task.to_string()),
        context_token_limit: None,
        last_reported_msg_id: None,
        auto_report: Some(false),
        trigger_tool_event_id: trigger_tool_event_id.map(|s| s.to_string()),
        provider_id: None,
        model_id: None,
        dispatch_rule: None,
        image_provider_id: None,
        image_model_id: None,
        vision_provider_id: None,
        vision_model_id: None,
        forked_from_session_id: None,
        forked_from_message_id: None,
        reasoning_effort: None,
        last_run_status: None,
    };
    conn.execute(
        "INSERT INTO sessions(
            id, title, workspace_path, access_mode, project_id, status, 
            last_message_at, created_at, updated_at, 
            parent_session_id, session_type, subagent_role, subagent_task,
            last_reported_msg_id, auto_report, trigger_tool_event_id
         ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'subprocess',?11,?12,NULL,0,?13)",
        params![
            s.id,
            s.title,
            s.workspace_path,
            s.access_mode,
            s.project_id,
            s.status,
            s.last_message_at,
            s.created_at,
            s.updated_at,
            s.parent_session_id,
            s.subagent_role,
            s.subagent_task,
            s.trigger_tool_event_id,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(s)
}

pub fn create_subagent_session(
    conn: &Connection,
    parent_session_id: &str,
    role: &str,
    title: &str,
    task: &str,
    workspace_path: &str,
    access_mode: Option<&str>,
    project_id: Option<&str>,
    trigger_tool_event_id: Option<&str>,
) -> Result<Session, String> {
    create_subprocess_session(
        conn,
        parent_session_id,
        role,
        title,
        task,
        workspace_path,
        access_mode,
        project_id,
        trigger_tool_event_id,
    )
}

pub fn fork_session_at_message(
    conn: &Connection,
    source_session_id: &str,
    target_message_id: &str,
    new_title: Option<&str>,
    include_target: bool,
) -> Result<Session, String> {
    let source_session = get_session(conn, source_session_id)?
        .ok_or_else(|| format!("未找到原会话: {}", source_session_id))?;

    let (target_seq, target_role): (i64, String) = conn
        .query_row(
            "SELECT seq, role FROM messages WHERE id = ?1 AND session_id = ?2",
            params![target_message_id, source_session_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| format!("未找到指定消息节点: {}", e))?;

    let max_seq = if !include_target {
        target_seq - 1
    } else if target_role == "assistant" {
        let mut check_seq = target_seq;
        let mut stmt = conn
            .prepare(
                "SELECT seq, role FROM messages WHERE session_id = ?1 AND seq > ?2 ORDER BY seq ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![source_session_id, target_seq], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        for row in rows.flatten() {
            if row.1 == "tool" {
                check_seq = row.0;
            } else {
                break;
            }
        }
        check_seq
    } else {
        target_seq
    };

    let title = if let Some(t) = new_title.filter(|s| !s.trim().is_empty()) {
        t.trim().to_string()
    } else {
        format!("[分支] {}", source_session.title)
    };

    let new_session_id = uuid::Uuid::new_v4().to_string();
    let now_str = now();

    let ws_path = if source_session.is_temp {
        source_session
            .source_workspace
            .clone()
            .unwrap_or_else(|| source_session.workspace_path.clone())
    } else {
        source_session.workspace_path.clone()
    };

    conn.execute(
        "INSERT INTO sessions (
            id, title, workspace_path, access_mode, project_id, status,
            last_message_at, created_at, updated_at, is_temp,
            temp_code, temp_root, source_workspace, merged_seq, merged_pending,
            parent_session_id, session_type, subagent_role, subagent_task,
            context_token_limit, last_reported_msg_id, auto_report,
            trigger_tool_event_id, provider_id, model_id, dispatch_rule,
            image_provider_id, image_model_id, vision_provider_id, vision_model_id,
            forked_from_session_id, forked_from_message_id, reasoning_effort
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, 'active',
            ?6, ?6, ?6, 0,
            NULL, NULL, NULL, NULL, 0,
            NULL, 'main', NULL, NULL,
            ?7, NULL, 1,
            NULL, ?8, ?9, NULL,
            ?10, ?11, ?12, ?13,
            ?14, ?15, ?16
        )",
        params![
            new_session_id,
            title,
            ws_path,
            source_session.access_mode,
            source_session.project_id,
            now_str,
            source_session.context_token_limit.map(|v| v as i64),
            source_session.provider_id,
            source_session.model_id,
            source_session.image_provider_id,
            source_session.image_model_id,
            source_session.vision_provider_id,
            source_session.vision_model_id,
            source_session_id,
            target_message_id,
            source_session.reasoning_effort,
        ],
    )
    .map_err(|e| format!("创建分支会话失败: {}", e))?;

    let mut stmt = conn
        .prepare(
            "SELECT id, run_id, seq, role, content, reasoning, tool_calls_json, tool_call_id,
                    queued, usage_json, created_at, prompt_tokens, completion_tokens,
                    total_tokens, duration_ms, turn_duration_ms, attachments_json,
                    cached_tokens, is_estimated
             FROM messages
             WHERE session_id = ?1 AND seq <= ?2 AND queued = 0
             ORDER BY seq ASC",
        )
        .map_err(|e| e.to_string())?;

    let old_msgs = stmt
        .query_map(params![source_session_id, max_seq], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, i64>(8)?,
                r.get::<_, Option<String>>(9)?,
                r.get::<_, String>(10)?,
                r.get::<_, Option<i64>>(11)?,
                r.get::<_, Option<i64>>(12)?,
                r.get::<_, Option<i64>>(13)?,
                r.get::<_, Option<i64>>(14)?,
                r.get::<_, Option<i64>>(15)?,
                r.get::<_, Option<String>>(16)?,
                r.get::<_, Option<i64>>(17)?,
                r.get::<_, Option<i64>>(18)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let mut id_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut new_seq: i64 = 1;

    for old in &old_msgs {
        let old_id = &old.0;
        let new_id = uuid::Uuid::new_v4().to_string();
        id_map.insert(old_id.clone(), new_id.clone());

        conn.execute(
            "INSERT INTO messages (
                id, session_id, run_id, seq, role, content, reasoning,
                tool_calls_json, tool_call_id, queued, usage_json, created_at,
                prompt_tokens, completion_tokens, total_tokens, duration_ms,
                turn_duration_ms, attachments_json, cached_tokens, is_estimated
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7,
                ?8, ?9, ?10, ?11, ?12,
                ?13, ?14, ?15, ?16,
                ?17, ?18, ?19, ?20
            )",
            params![
                new_id,
                new_session_id,
                old.1,
                new_seq,
                old.3,
                old.4,
                old.5,
                old.6,
                old.7,
                old.8,
                old.9,
                old.10,
                old.11,
                old.12,
                old.13,
                old.14,
                old.15,
                old.16,
                old.17.unwrap_or(0),
                old.18.unwrap_or(0),
            ],
        )
        .map_err(|e| format!("复制消息失败: {}", e))?;

        new_seq += 1;
    }

    for (old_id, new_id) in &id_map {
        let mut ev_stmt = conn
            .prepare(
                "SELECT tool_name, tool_call_id, params_json, result_text, status,
                        approval_scope, created_at, subprocess_id
                 FROM tool_events WHERE message_id = ?1 ORDER BY rowid ASC",
            )
            .map_err(|e| e.to_string())?;

        let ev_rows = ev_stmt
            .query_map(params![old_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, Option<String>>(7)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        for ev in ev_rows {
            let new_ev_id = uuid::Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO tool_events (
                    id, message_id, tool_name, tool_call_id, params_json,
                    result_text, status, approval_scope, created_at, subprocess_id
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    new_ev_id,
                    new_id,
                    ev.0,
                    ev.1,
                    ev.2,
                    ev.3,
                    ev.4,
                    ev.5,
                    ev.6,
                    ev.7,
                ],
            )
            .map_err(|e| format!("复制工具事件失败: {}", e))?;
        }
    }

    let mut comp_stmt = conn
        .prepare(
            "SELECT start_seq, end_seq, summary_markdown, tokens_before, created_at
             FROM session_compactions WHERE session_id = ?1 AND end_seq <= ?2 ORDER BY end_seq ASC",
        )
        .map_err(|e| e.to_string())?;

    let comp_rows = comp_stmt
        .query_map(params![source_session_id, max_seq], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    for c in comp_rows {
        let new_comp_id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO session_compactions (
                id, session_id, start_seq, end_seq, summary_markdown, tokens_before, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![new_comp_id, new_session_id, c.0, c.1, c.2, c.3, c.4],
        )
        .map_err(|e| format!("复制压缩备忘失败: {}", e))?;
    }

    let mut rules_stmt = conn
        .prepare("SELECT kind, pattern, created_at FROM session_rules_t WHERE session_id = ?1")
        .map_err(|e| e.to_string())?;

    let rule_rows = rules_stmt
        .query_map(params![source_session_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    for r in rule_rows {
        let new_rule_id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO session_rules_t (id, session_id, kind, pattern, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![new_rule_id, new_session_id, r.0, r.1, r.2],
        )
        .map_err(|e| format!("复制审批规则失败: {}", e))?;
    }

    get_session(conn, &new_session_id)?.ok_or_else(|| "无法获取新创建的分支会话".to_string())
}

pub fn touch_session(conn: &Connection, id: &str) -> Result<(), String> {
    let t = now();
    conn.execute(
        "UPDATE sessions SET last_message_at = ?2, updated_at = ?2 WHERE id = ?1",
        params![id, t],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn rename_session(conn: &Connection, id: &str, title: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE sessions SET title = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, title, now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn delete_session(conn: &Connection, id: &str) -> Result<(), String> {
    // 级联删除其下的子 Agent 会话
    if let Ok(mut stmt) = conn.prepare("SELECT id FROM sessions WHERE parent_session_id = ?1") {
        let sub_ids: Vec<String> = stmt
            .query_map(params![id], |r| r.get(0))
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default();
        for sub_id in sub_ids {
            let _ = delete_session(conn, &sub_id);
        }
    }
    conn.execute("DELETE FROM tool_events WHERE message_id IN (SELECT id FROM messages WHERE session_id = ?1)", params![id]).map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM messages WHERE session_id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM runs WHERE session_id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM session_kv WHERE session_id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    // 会话规则随对话删除一并清理（不残留孤儿规则）
    conn.execute("DELETE FROM session_rules_t WHERE session_id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_session_status(conn: &Connection, id: &str, status: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE sessions SET status = ?2 WHERE id = ?1",
        params![id, status],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 设置会话访问模式：始终写入具体值（confirm / full_access），
/// 拒绝写入 NULL——访问模式为纯会话级，不存在“跟随全局”状态。
pub fn set_session_mode(conn: &Connection, id: &str, mode: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE sessions SET access_mode = ?2 WHERE id = ?1",
        params![id, normalize_access_mode(mode)],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 设置会话专属上下文 Token 上限（None 表示清除专属设置，恢复跟随模型与全局默认）
pub fn set_session_context_limit(conn: &Connection, id: &str, limit: Option<usize>) -> Result<(), String> {
    conn.execute(
        "UPDATE sessions SET context_token_limit = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, limit.map(|v| v as i64), now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 标记临时空间会话（workspace_path 已是临时副本路径）
pub fn set_session_temp(conn: &Connection, id: &str, code: &str, root: &str, source_workspace: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE sessions SET is_temp = 1, temp_code = ?2, temp_root = ?3, source_workspace = ?4, updated_at = ?5 WHERE id = ?1",
        params![id, code, root, source_workspace, now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 更新合并状态：merged_seq 记录消息编辑边界（永久生效），merged_pending 表示待清空临时空间（清空时解除）
pub fn set_session_merge_state(conn: &Connection, id: &str, merged_seq: Option<i64>, merged_pending: bool) -> Result<(), String> {
    conn.execute(
        "UPDATE sessions SET merged_seq = ?2, merged_pending = ?3, updated_at = ?4 WHERE id = ?1",
        params![id, merged_seq, merged_pending as i64, now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_session_project(conn: &Connection, id: &str, project_id: Option<&str>) -> Result<(), String> {
    conn.execute(
        "UPDATE sessions SET project_id = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, project_id, now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_message_tool_call_id(conn: &Connection, id: &str, tool_call_id: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE messages SET tool_call_id = ?2 WHERE id = ?1",
        params![id, tool_call_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
// ---------- messages ----------

pub fn next_seq(conn: &Connection, session_id: &str) -> Result<i64, String> {
    conn.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM messages WHERE session_id = ?1",
        params![session_id],
        |r| r.get(0),
    )
    .map_err(|e| e.to_string())
}

pub fn insert_message(conn: &Connection, m: &Message) -> Result<(), String> {
    conn.execute(
        "INSERT INTO messages(id, session_id, run_id, seq, role, content, reasoning, tool_calls_json, tool_call_id, queued, usage_json, created_at, prompt_tokens, completion_tokens, total_tokens, cached_tokens, is_estimated, duration_ms, turn_duration_ms, attachments_json, reverted_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21)",
        params![
            m.id,
            m.session_id,
            m.run_id,
            m.seq,
            m.role,
            m.content,
            m.reasoning,
            m.tool_calls.as_ref().and_then(|v| serde_json::to_string(v).ok()),
            m.tool_call_id,
            m.queued as i64,
            m.usage.as_ref().and_then(|v| serde_json::to_string(v).ok()),
            m.created_at,
            m.prompt_tokens.unwrap_or(0) as i64,
            m.completion_tokens.unwrap_or(0) as i64,
            m.total_tokens.unwrap_or(0) as i64,
            m.cached_tokens.unwrap_or(0) as i64,
            m.is_estimated.unwrap_or(false) as i64,
            m.duration_ms.unwrap_or(0) as i64,
            m.turn_duration_ms.unwrap_or(0) as i64,
            m.attachments.as_ref().and_then(|v| serde_json::to_string(v).ok()),
            m.reverted_at,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn update_message_turn_duration(
    conn: &Connection,
    id: &str,
    turn_duration_ms: u64,
) -> Result<(), String> {
    conn.execute(
        "UPDATE messages SET turn_duration_ms = ?2 WHERE id = ?1",
        params![id, turn_duration_ms as i64],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_message_run_id(
    conn: &Connection,
    message_id: &str,
    run_id: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE messages SET run_id = ?2 WHERE id = ?1",
        params![message_id, run_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn new_message(
    conn: &Connection,
    session_id: &str,
    role: &str,
    content: Option<String>,
    queued: bool,
) -> Result<Message, String> {
    new_message_with_run(conn, session_id, role, content, queued, None)
}

pub fn new_message_with_attachments(
    conn: &Connection,
    session_id: &str,
    role: &str,
    content: Option<String>,
    attachments: Option<Vec<Attachment>>,
    queued: bool,
    run_id: Option<&str>,
) -> Result<Message, String> {
    let seq = next_seq(conn, session_id)?;
    let m = Message {
        id: uuid::Uuid::new_v4().to_string(),
        session_id: session_id.to_string(),
        run_id: run_id.map(|s| s.to_string()),
        seq,
        role: role.to_string(),
        content,
        reasoning: None,
        tool_calls: None,
        tool_call_id: None,
        queued,
        usage: None,
        created_at: now(),
        tool_events: vec![],
        duration_ms: None,
        turn_duration_ms: None,
        prompt_tokens: None,
        completion_tokens: None,
        total_tokens: None,
        cached_tokens: None,
        is_estimated: None,
        attachments,
        reverted_at: None,
    };
    insert_message(conn, &m)?;
    Ok(m)
}

pub fn new_message_with_run(
    conn: &Connection,
    session_id: &str,
    role: &str,
    content: Option<String>,
    queued: bool,
    run_id: Option<&str>,
) -> Result<Message, String> {
    new_message_with_attachments(conn, session_id, role, content, None, queued, run_id)
}

pub fn update_message_content(
    conn: &Connection,
    id: &str,
    content: &str,
    usage: Option<&serde_json::Value>,
) -> Result<(), String> {
    let (pt, ct, tt, cached, dur, is_est) = if let Some(u) = usage {
        let pt = u.get("promptTokens").or_else(|| u.get("prompt_tokens")).or_else(|| u.get("inputEst")).and_then(|v| v.as_u64()).unwrap_or(0);
        let ct = u.get("completionTokens").or_else(|| u.get("completion_tokens")).or_else(|| u.get("outputEst")).and_then(|v| v.as_u64()).unwrap_or(0);
        let tt = u.get("totalTokens").or_else(|| u.get("total_tokens")).and_then(|v| v.as_u64()).unwrap_or(pt + ct);
        let cached = u.get("cachedTokens").or_else(|| u.get("cached_tokens"))
            .or_else(|| u.get("prompt_tokens_details").and_then(|d| d.get("cached_tokens")))
            .or_else(|| u.get("prompt_cache_hit_tokens"))
            .or_else(|| u.get("cache_read_input_tokens"))
            .and_then(|v| v.as_u64()).unwrap_or(0);
        let dur = u.get("durationMs").or_else(|| u.get("duration_ms")).and_then(|v| v.as_u64()).unwrap_or(0);
        let is_est = u.get("isEstimated").and_then(|v| v.as_bool()).unwrap_or(false);
        (pt, ct, tt, cached, dur, is_est)
    } else {
        (0, 0, 0, 0, 0, false)
    };
    conn.execute(
        "UPDATE messages SET
            content = ?2,
            usage_json = COALESCE(?3, usage_json),
            prompt_tokens = CASE WHEN ?4 > 0 THEN ?4 ELSE prompt_tokens END,
            completion_tokens = CASE WHEN ?5 > 0 THEN ?5 ELSE completion_tokens END,
            total_tokens = CASE WHEN ?6 > 0 THEN ?6 ELSE total_tokens END,
            cached_tokens = CASE WHEN ?7 > 0 THEN ?7 ELSE cached_tokens END,
            duration_ms = CASE WHEN ?8 > 0 THEN ?8 ELSE duration_ms END,
            is_estimated = CASE WHEN ?9 = 1 THEN 1 ELSE is_estimated END
         WHERE id = ?1",
        params![
            id,
            content,
            usage.and_then(|v| serde_json::to_string(v).ok()),
            pt as i64,
            ct as i64,
            tt as i64,
            cached as i64,
            dur as i64,
            is_est as i64,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 更新消息内容与附件列表（用于编辑重发）
pub fn update_message_content_and_attachments(
    conn: &Connection,
    id: &str,
    content: &str,
    attachments: Option<&[Attachment]>,
) -> Result<(), String> {
    let att_json = attachments.and_then(|v| {
        if v.is_empty() {
            None
        } else {
            serde_json::to_string(v).ok()
        }
    });
    conn.execute(
        "UPDATE messages SET
            content = ?2,
            attachments_json = ?3
         WHERE id = ?1",
        params![id, content, att_json],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 更新 assistant 消息的思考过程（reasoning_content）；仅落库展示，不进入上下文组装
pub fn update_message_reasoning(conn: &Connection, id: &str, reasoning: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE messages SET reasoning = ?2 WHERE id = ?1",
        params![id, reasoning],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn update_message_tool_calls(
    conn: &Connection,
    id: &str,
    tool_calls: &serde_json::Value,
    content: &str,
    usage: Option<&serde_json::Value>,
) -> Result<(), String> {
    let (pt, ct, tt, cached, dur, is_est) = if let Some(u) = usage {
        let pt = u.get("promptTokens").or_else(|| u.get("prompt_tokens")).or_else(|| u.get("inputEst")).and_then(|v| v.as_u64()).unwrap_or(0);
        let ct = u.get("completionTokens").or_else(|| u.get("completion_tokens")).or_else(|| u.get("outputEst")).and_then(|v| v.as_u64()).unwrap_or(0);
        let tt = u.get("totalTokens").or_else(|| u.get("total_tokens")).and_then(|v| v.as_u64()).unwrap_or(pt + ct);
        let cached = u.get("cachedTokens").or_else(|| u.get("cached_tokens"))
            .or_else(|| u.get("prompt_tokens_details").and_then(|d| d.get("cached_tokens")))
            .or_else(|| u.get("prompt_cache_hit_tokens"))
            .or_else(|| u.get("cache_read_input_tokens"))
            .and_then(|v| v.as_u64()).unwrap_or(0);
        let dur = u.get("durationMs").or_else(|| u.get("duration_ms")).and_then(|v| v.as_u64()).unwrap_or(0);
        let is_est = u.get("isEstimated").and_then(|v| v.as_bool()).unwrap_or(false);
        (pt, ct, tt, cached, dur, is_est)
    } else {
        (0, 0, 0, 0, 0, false)
    };
    conn.execute(
        "UPDATE messages SET
            tool_calls_json = ?2,
            content = ?3,
            usage_json = COALESCE(?4, usage_json),
            prompt_tokens = CASE WHEN ?5 > 0 THEN ?5 ELSE prompt_tokens END,
            completion_tokens = CASE WHEN ?6 > 0 THEN ?6 ELSE completion_tokens END,
            total_tokens = CASE WHEN ?7 > 0 THEN ?7 ELSE total_tokens END,
            cached_tokens = CASE WHEN ?8 > 0 THEN ?8 ELSE cached_tokens END,
            duration_ms = CASE WHEN ?9 > 0 THEN ?9 ELSE duration_ms END,
            is_estimated = CASE WHEN ?10 = 1 THEN 1 ELSE is_estimated END
         WHERE id = ?1",
        params![
            id,
            tool_calls.to_string(),
            content,
            usage.and_then(|v| serde_json::to_string(v).ok()),
            pt as i64,
            ct as i64,
            tt as i64,
            cached as i64,
            dur as i64,
            is_est as i64,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_messages(
    conn: &Connection,
    session_id: &str,
    before_seq: Option<i64>,
    limit: i64,
) -> Result<Vec<Message>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, run_id, seq, role, content, reasoning, tool_calls_json, tool_call_id, queued, usage_json, created_at, prompt_tokens, completion_tokens, total_tokens, duration_ms, turn_duration_ms, attachments_json, cached_tokens, is_estimated, reverted_at
             FROM messages WHERE session_id = ?1 AND seq < COALESCE(?2, 9223372036854775807)
             ORDER BY seq DESC LIMIT ?3",
        )
        .map_err(|e| e.to_string())?;
    let mut msgs: Vec<Message> = stmt
        .query_map(params![session_id, before_seq, limit], |r| {
            let tc: Option<String> = r.get(6)?;
            let usage: Option<String> = r.get(9)?;
            let pt: Option<i64> = r.get(11)?;
            let ct: Option<i64> = r.get(12)?;
            let tt: Option<i64> = r.get(13)?;
            let dur: Option<i64> = r.get(14)?;
            let turn_dur: Option<i64> = r.get(15)?;
            let att: Option<String> = r.get(16)?;
            let cached: Option<i64> = r.get(17)?;
            let is_est: Option<i64> = r.get(18)?;
            let rev_at: Option<String> = r.get(19)?;
            Ok(Message {
                id: r.get(0)?,
                session_id: session_id.to_string(),
                run_id: r.get(1)?,
                seq: r.get(2)?,
                role: r.get(3)?,
                content: r.get(4)?,
                reasoning: r.get(5)?,
                tool_calls: tc.and_then(|s| serde_json::from_str(&s).ok()),
                tool_call_id: r.get(7)?,
                queued: r.get::<_, i64>(8)? != 0,
                usage: usage.and_then(|s| serde_json::from_str(&s).ok()),
                created_at: r.get(10)?,
                tool_events: vec![],
                duration_ms: dur.map(|v| v as u64),
                turn_duration_ms: turn_dur.map(|v| v as u64),
                prompt_tokens: pt.map(|v| v as u64),
                completion_tokens: ct.map(|v| v as u64),
                total_tokens: tt.map(|v| v as u64),
                cached_tokens: cached.map(|v| v as u64),
                is_estimated: is_est.map(|v| v != 0),
                attachments: att.and_then(|s| serde_json::from_str(&s).ok()),
                reverted_at: rev_at,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    msgs.reverse();

    // join tool events（挂到所属 assistant 消息上）
    let ids: Vec<String> = msgs.iter().map(|m| m.id.clone()).collect();
    for m in msgs.iter_mut() {
        m.tool_events = tool_events_for(conn, &ids, &m.id)?;
    }
    Ok(msgs)
}

fn tool_events_for(
    conn: &Connection,
    message_ids: &[String],
    message_id: &str,
) -> Result<Vec<ToolEvent>, String> {
    // 简单起见按单个 message 查询（消息量小）
    let _ = message_ids;
    let mut stmt = conn
        .prepare(
            "SELECT te.id, te.message_id, te.tool_name, te.tool_call_id, te.params_json, te.result_text, te.status, te.approval_scope, te.created_at, te.subprocess_id,
                    (SELECT reverted_at FROM tool_file_snapshots WHERE tool_event_id = te.id ORDER BY rowid DESC LIMIT 1)
             FROM tool_events te WHERE te.message_id = ?1 ORDER BY te.rowid ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![message_id], |r| {
            let pj: String = r.get(4)?;
            Ok(ToolEvent {
                id: r.get(0)?,
                message_id: r.get(1)?,
                tool_name: r.get(2)?,
                tool_call_id: r.get(3)?,
                params: serde_json::from_str(&pj).unwrap_or(serde_json::Value::Null),
                result_text: r.get(5)?,
                status: r.get(6)?,
                approval_scope: r.get(7)?,
                created_at: r.get(8)?,
                subprocess_id: r.get(9)?,
                reverted_at: r.get(10)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

pub fn all_messages(conn: &Connection, session_id: &str) -> Result<Vec<Message>, String> {
    get_messages(conn, session_id, None, i64::MAX)
}

pub fn delete_messages_after(conn: &Connection, session_id: &str, seq: i64) -> Result<(), String> {
    conn.execute(
        "DELETE FROM tool_events WHERE message_id IN
           (SELECT id FROM messages WHERE session_id = ?1 AND seq > ?2)",
        params![session_id, seq],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM messages WHERE session_id = ?1 AND seq > ?2",
        params![session_id, seq],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM session_compactions WHERE session_id = ?1 AND end_seq > ?2",
        params![session_id, seq],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn create_session_compaction(conn: &Connection, comp: &SessionCompaction) -> Result<(), String> {
    conn.execute(
        "INSERT INTO session_compactions (id, session_id, start_seq, end_seq, summary_markdown, tokens_before, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            comp.id,
            comp.session_id,
            comp.start_seq,
            comp.end_seq,
            comp.summary_markdown,
            comp.tokens_before as i64,
            comp.created_at
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn list_session_compactions(conn: &Connection, session_id: &str) -> Result<Vec<SessionCompaction>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, session_id, start_seq, end_seq, summary_markdown, tokens_before, created_at
             FROM session_compactions
             WHERE session_id = ?1
             ORDER BY start_seq ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![session_id], |r| {
            let tb: i64 = r.get(5)?;
            Ok(SessionCompaction {
                id: r.get(0)?,
                session_id: r.get(1)?,
                start_seq: r.get(2)?,
                end_seq: r.get(3)?,
                summary_markdown: r.get(4)?,
                tokens_before: tb.max(0) as usize,
                created_at: r.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

pub fn get_message(conn: &Connection, id: &str) -> Result<Option<Message>, String> {
    let opt = conn
        .query_row(
            "SELECT id, session_id, run_id, seq, role, content, reasoning, tool_calls_json, tool_call_id, queued, usage_json, created_at, prompt_tokens, completion_tokens, total_tokens, duration_ms, turn_duration_ms, attachments_json, cached_tokens, is_estimated, reverted_at
             FROM messages WHERE id = ?1",
            params![id],
            |r| {
                let tc: Option<String> = r.get(7)?;
                let usage: Option<String> = r.get(10)?;
                let pt: Option<i64> = r.get(12)?;
                let ct: Option<i64> = r.get(13)?;
                let tt: Option<i64> = r.get(14)?;
                let dur: Option<i64> = r.get(15)?;
                let turn_dur: Option<i64> = r.get(16)?;
                let att: Option<String> = r.get(17)?;
                let cached: Option<i64> = r.get(18)?;
                let is_est: Option<i64> = r.get(19)?;
                let rev_at: Option<String> = r.get(20)?;
                Ok(Message {
                    id: r.get(0)?,
                    session_id: r.get(1)?,
                    run_id: r.get(2)?,
                    seq: r.get(3)?,
                    role: r.get(4)?,
                    content: r.get(5)?,
                    reasoning: r.get(6)?,
                    tool_calls: tc.and_then(|s| serde_json::from_str(&s).ok()),
                    tool_call_id: r.get(8)?,
                    queued: r.get::<_, i64>(9)? != 0,
                    usage: usage.and_then(|s| serde_json::from_str(&s).ok()),
                    created_at: r.get(11)?,
                    tool_events: vec![],
                    duration_ms: dur.map(|v| v as u64),
                    turn_duration_ms: turn_dur.map(|v| v as u64),
                    prompt_tokens: pt.map(|v| v as u64),
                    completion_tokens: ct.map(|v| v as u64),
                    total_tokens: tt.map(|v| v as u64),
                    cached_tokens: cached.map(|v| v as u64),
                    is_estimated: is_est.map(|v| v != 0),
                    attachments: att.and_then(|s| serde_json::from_str(&s).ok()),
                    reverted_at: rev_at,
                })
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(opt)
}

// ---------- 待执行队列 ----------

pub fn list_queued(conn: &Connection, session_id: &str) -> Result<Vec<Message>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, session_id, run_id, seq, role, content, reasoning, tool_calls_json, tool_call_id, queued, usage_json, created_at, prompt_tokens, completion_tokens, total_tokens, duration_ms, turn_duration_ms, attachments_json, cached_tokens, is_estimated, reverted_at
             FROM messages WHERE session_id = ?1 AND queued = 1 ORDER BY seq ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![session_id], |r| {
            let tc: Option<String> = r.get(7)?;
            let usage: Option<String> = r.get(10)?;
            let pt: Option<i64> = r.get(12)?;
            let ct: Option<i64> = r.get(13)?;
            let tt: Option<i64> = r.get(14)?;
            let dur: Option<i64> = r.get(15)?;
            let turn_dur: Option<i64> = r.get(16)?;
            let att: Option<String> = r.get(17)?;
            let cached: Option<i64> = r.get(18)?;
            let is_est: Option<i64> = r.get(19)?;
            let rev_at: Option<String> = r.get(20)?;
            Ok(Message {
                id: r.get(0)?,
                session_id: r.get(1)?,
                run_id: r.get(2)?,
                seq: r.get(3)?,
                role: r.get(4)?,
                content: r.get(5)?,
                reasoning: r.get(6)?,
                tool_calls: tc.and_then(|s| serde_json::from_str(&s).ok()),
                tool_call_id: r.get(8)?,
                queued: true,
                usage: usage.and_then(|s| serde_json::from_str(&s).ok()),
                created_at: r.get(11)?,
                tool_events: vec![],
                duration_ms: dur.map(|v| v as u64),
                turn_duration_ms: turn_dur.map(|v| v as u64),
                prompt_tokens: pt.map(|v| v as u64),
                completion_tokens: ct.map(|v| v as u64),
                total_tokens: tt.map(|v| v as u64),
                cached_tokens: cached.map(|v| v as u64),
                is_estimated: is_est.map(|v| v != 0),
                attachments: att.and_then(|s| serde_json::from_str(&s).ok()),
                reverted_at: rev_at,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// 取出最早一条排队消息（queued=0 使其进入对话流），返回该消息
pub fn pop_queued(conn: &Connection, session_id: &str) -> Result<Option<Message>, String> {
    let first: Option<String> = conn
        .query_row(
            "SELECT id FROM messages WHERE session_id = ?1 AND queued = 1 ORDER BY seq ASC LIMIT 1",
            params![session_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some(id) = first else { return Ok(None) };
    conn.execute("UPDATE messages SET queued = 0 WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    get_message(conn, &id)
}

pub fn dequeue_message(conn: &Connection, message_id: &str) -> Result<Option<Message>, String> {
    conn.execute(
        "UPDATE messages SET queued = 0 WHERE id = ?1 AND queued = 1",
        params![message_id],
    )
    .map_err(|e| e.to_string())?;
    get_message(conn, message_id)
}

pub fn remove_queued(conn: &Connection, message_id: &str) -> Result<bool, String> {
    let n = conn
        .execute("DELETE FROM messages WHERE id = ?1 AND queued = 1", params![message_id])
        .map_err(|e| e.to_string())?;
    Ok(n > 0)
}

// ---------- tool events ----------

pub fn insert_tool_event(conn: &Connection, ev: &ToolEvent) -> Result<(), String> {
    conn.execute(
        "INSERT INTO tool_events(id, message_id, tool_name, tool_call_id, params_json, result_text, status, approval_scope, created_at, subprocess_id)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            ev.id,
            ev.message_id,
            ev.tool_name,
            ev.tool_call_id,
            serde_json::to_string(&ev.params).map_err(|e| e.to_string())?,
            ev.result_text,
            ev.status,
            ev.approval_scope,
            ev.created_at,
            ev.subprocess_id,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn update_tool_event(
    conn: &Connection,
    id: &str,
    status: &str,
    result_text: Option<&str>,
    approval_scope: Option<&str>,
) -> Result<(), String> {
    update_tool_event_full(conn, id, status, result_text, approval_scope, None, None)
}

pub fn update_tool_event_full(
    conn: &Connection,
    id: &str,
    status: &str,
    result_text: Option<&str>,
    approval_scope: Option<&str>,
    subprocess_id: Option<&str>,
    params_json: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "UPDATE tool_events SET status = ?2,
            result_text = COALESCE(?3, result_text),
            approval_scope = COALESCE(?4, approval_scope),
            subprocess_id = COALESCE(?5, subprocess_id),
            params_json = COALESCE(?6, params_json)
         WHERE id = ?1",
        params![id, status, result_text, approval_scope, subprocess_id, params_json],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_tool_event_subprocess_id(
    conn: &Connection,
    id: &str,
    subprocess_id: &str,
    params_json: Option<&str>,
) -> Result<(), String> {
    if let Some(pj) = params_json {
        conn.execute(
            "UPDATE tool_events SET subprocess_id = ?2, params_json = ?3 WHERE id = ?1",
            params![id, subprocess_id, pj],
        )
    } else {
        conn.execute(
            "UPDATE tool_events SET subprocess_id = ?2 WHERE id = ?1",
            params![id, subprocess_id],
        )
    }
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------- runs ----------

pub fn create_run(conn: &Connection, session_id: &str, trigger_message_id: Option<&str>) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO runs(id, session_id, trigger_message_id, trigger_type, status, started_at) VALUES(?1,?2,?3,'manual','running',?4)",
        params![id, session_id, trigger_message_id, now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(id)
}

pub fn finish_run(
    conn: &Connection,
    id: &str,
    status: &str,
    duration_ms: Option<u64>,
    total_tokens: Option<u64>,
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    cached_tokens: Option<u64>,
) -> Result<(), String> {
    conn.execute(
        "UPDATE runs SET status = ?2, ended_at = ?3, duration_ms = ?4, total_tokens = ?5, prompt_tokens = ?6, completion_tokens = ?7, cached_tokens = ?8 WHERE id = ?1",
        params![
            id,
            status,
            now(),
            duration_ms.map(|v| v as i64),
            total_tokens.map(|v| v as i64),
            prompt_tokens.map(|v| v as i64),
            completion_tokens.map(|v| v as i64),
            cached_tokens.map(|v| v as i64),
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 会话退出运行循环时兜底：把该会话残留的 running 状态 run 标记为 interrupted
/// （如 stop 中止任务未走到 finish_run、或异常退出留下的记录），避免状态永远漂在“运行中”
pub fn fail_open_runs(conn: &Connection, session_id: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE runs SET status = 'interrupted', ended_at = ?2 WHERE session_id = ?1 AND status = 'running'",
        params![session_id, now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 查询会话最新一次 Run 的运行状态（如 done / interrupted / failed / cancelled / running 等）
pub fn get_last_run_status(conn: &Connection, session_id: &str) -> Option<String> {
    conn.query_row(
        "SELECT status FROM runs WHERE session_id = ?1 ORDER BY started_at DESC LIMIT 1",
        params![session_id],
        |r| r.get(0),
    )
    .ok()
}

/// 会话退出运行或被中止时兜底：把该会话残留的 running / pending_approval 状态 tool_events 标记为 failed
pub fn fail_open_tool_events(conn: &Connection, session_id: &str) -> Result<Vec<ToolEvent>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, message_id, tool_name, tool_call_id, params_json, result_text, status, approval_scope, created_at, subprocess_id
             FROM tool_events
             WHERE message_id IN (SELECT id FROM messages WHERE session_id = ?1)
               AND status IN ('running', 'pending_approval')",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![session_id], |r| {
            let pj: String = r.get(4)?;
            Ok(ToolEvent {
                id: r.get(0)?,
                message_id: r.get(1)?,
                tool_name: r.get(2)?,
                tool_call_id: r.get(3)?,
                params: serde_json::from_str(&pj).unwrap_or(serde_json::Value::Null),
                result_text: Some("[任务已终止]".to_string()),
                status: "failed".to_string(),
                approval_scope: r.get(7)?,
                created_at: r.get(8)?,
                subprocess_id: r.get(9)?,
                reverted_at: None,
            })
        })
        .map_err(|e| e.to_string())?;
    let events: Vec<ToolEvent> = rows.filter_map(|r| r.ok()).collect();

    conn.execute(
        "UPDATE tool_events
         SET status = 'failed',
             result_text = COALESCE(result_text, '[任务已终止]')
         WHERE message_id IN (SELECT id FROM messages WHERE session_id = ?1)
           AND status IN ('running', 'pending_approval')",
        params![session_id],
    )
    .map_err(|e| e.to_string())?;

    Ok(events)
}

/// 应用启动时兜底清理：将上一次运行遗留的 running 状态 run 与 tool_events 标记为中断/失败
pub fn cleanup_orphaned_running_states(conn: &Connection) -> Result<(), String> {
    conn.execute(
        "UPDATE runs SET status = 'interrupted', ended_at = ?1 WHERE status = 'running'",
        params![now()],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE tool_events SET status = 'failed', result_text = COALESCE(result_text, '[应用重启，未完成的命令已终止]') WHERE status IN ('running', 'pending_approval')",
        [],
    )
    .map_err(|e| e.to_string())?;
    let _ = conn.execute(
        "UPDATE long_tasks SET status = 'paused', updated_at = ?1 WHERE status IN ('running', 'planning')",
        params![now()],
    );
    Ok(())
}

/// 获取单个工具事件及其关联的 session_id
pub fn get_tool_event_with_session(conn: &Connection, event_id: &str) -> Result<Option<(ToolEvent, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT te.id, te.message_id, te.tool_name, te.tool_call_id, te.params_json, te.result_text, te.status, te.approval_scope, te.created_at, m.session_id, te.subprocess_id,
                    (SELECT reverted_at FROM tool_file_snapshots WHERE tool_event_id = te.id ORDER BY rowid DESC LIMIT 1)
             FROM tool_events te
             JOIN messages m ON te.message_id = m.id
             WHERE te.id = ?1",
        )
        .map_err(|e| e.to_string())?;
    let mut rows = stmt
        .query_map(params![event_id], |r| {
            let pj: String = r.get(4)?;
            let ev = ToolEvent {
                id: r.get(0)?,
                message_id: r.get(1)?,
                tool_name: r.get(2)?,
                tool_call_id: r.get(3)?,
                params: serde_json::from_str(&pj).unwrap_or(serde_json::Value::Null),
                result_text: r.get(5)?,
                status: r.get(6)?,
                approval_scope: r.get(7)?,
                created_at: r.get(8)?,
                subprocess_id: r.get(10)?,
                reverted_at: r.get(11)?,
            };
            let session_id: String = r.get(9)?;
            Ok((ev, session_id))
        })
        .map_err(|e| e.to_string())?;

    if let Some(r) = rows.next() {
        r.map(Some).map_err(|e| e.to_string())
    } else {
        Ok(None)
    }
}

/// 全部运行中的 run（跨会话）：前端界面刷新后据此恢复各会话的运行状态
pub fn all_running_runs(conn: &Connection) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare("SELECT session_id, id FROM runs WHERE status = 'running'")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

// ---------- session kv ----------

pub fn set_kv(conn: &Connection, session_id: &str, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO session_kv(session_id, key, value) VALUES(?1,?2,?3)
         ON CONFLICT(session_id, key) DO UPDATE SET value = excluded.value",
        params![session_id, key, value],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_kv(conn: &Connection, session_id: &str, key: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT value FROM session_kv WHERE session_id = ?1 AND key = ?2",
        params![session_id, key],
        |r| r.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

// ---------- secrets（API Key 加密落库） ----------

/// 保存 API Key：AES-256-GCM 加密后存入 secrets 表（主密钥在数据目录 secret.key）
pub fn secret_set(conn: &Connection, master: &[u8; 32], id: &str, secret: &str) -> Result<(), String> {
    let enc = crate::secrets::encrypt(master, secret)?;
    conn.execute(
        "INSERT INTO secrets(id, enc) VALUES(?1, ?2)
         ON CONFLICT(id) DO UPDATE SET enc = excluded.enc",
        params![id, enc],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn secret_get(conn: &Connection, master: &[u8; 32], id: &str) -> Result<Option<String>, String> {
    let enc: Option<String> = conn
        .query_row("SELECT enc FROM secrets WHERE id = ?1", params![id], |r| r.get(0))
        .optional()
        .map_err(|e| e.to_string())?;
    match enc {
        Some(e) => Ok(Some(crate::secrets::decrypt(master, &e)?)),
        None => Ok(None),
    }
}

pub fn secret_delete(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM secrets WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------- 旧版 keyring 凭据一次性迁移 ----------

fn keyring_get(id: &str) -> Result<Option<String>, String> {
    let entry = keyring::Entry::new("harness_mini", id).map_err(|e| e.to_string())?;
    match entry.get_password() {
        Ok(k) => Ok(Some(k)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn keyring_delete(id: &str) {
    if let Ok(entry) = keyring::Entry::new("harness_mini", id) {
        let _ = entry.delete_credential();
    }
}

/// 旧版本把 API Key 存在系统凭据库（keyring）；升级后一次性迁入 secrets 表并删除凭据库条目
pub fn migrate_secrets_from_keyring(conn: &Connection, master: &[u8; 32]) -> Result<usize, String> {
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM secrets", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if n > 0 {
        return Ok(0);
    }
    let settings = get_settings(conn)?;
    let mut moved = 0;
    for p in settings.providers {
        if let Ok(Some(k)) = keyring_get(&p.id) {
            if secret_set(conn, master, &p.id, &k).is_ok() {
                keyring_delete(&p.id);
                moved += 1;
            }
        }
    }
    Ok(moved)
}

/// 内存哨兵库 → 新目录库：搬运设置项，并用新主密钥重加密 API Key
pub fn copy_settings_and_secrets(
    src: &Connection,
    dst: &Connection,
    src_master: &[u8; 32],
    dst_master: &[u8; 32],
) -> Result<(), String> {
    let mut stmt = src.prepare("SELECT key, value FROM settings").map_err(|e| e.to_string())?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);
    for (k, v) in rows {
        dst.execute(
            "INSERT INTO settings(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![k, v],
        )
        .map_err(|e| e.to_string())?;
    }

    let mut stmt = src.prepare("SELECT id, enc FROM secrets").map_err(|e| e.to_string())?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);
    for (id, enc) in rows {
        let plain = crate::secrets::decrypt(src_master, &enc)?;
        secret_set(dst, dst_master, &id, &plain)?;
    }
    Ok(())
}

// ---------- agent_growths ----------

pub fn insert_growth(conn: &Connection, item: &GrowthItem) -> Result<(), String> {
    conn.execute(
        "INSERT INTO agent_growths (
            id, project_id, session_id, message_id, run_id,
            trigger_type, trigger_context, reflection_thought,
            category, title, rule_content, status, applied_count,
            created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            item.id,
            item.project_id,
            item.session_id,
            item.message_id,
            item.run_id,
            item.trigger_type,
            item.trigger_context,
            item.reflection_thought,
            item.category,
            item.title,
            item.rule_content,
            item.status,
            item.applied_count,
            item.created_at,
            item.updated_at,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn update_growth_status(conn: &Connection, id: &str, status: &str) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE agent_growths SET status = ?1, updated_at = ?2 WHERE id = ?3",
        params![status, now, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn update_growth_rule(
    conn: &Connection,
    id: &str,
    title: &str,
    rule_content: &str,
    category: &str,
) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE agent_growths SET title = ?1, rule_content = ?2, category = ?3, updated_at = ?4 WHERE id = ?5",
        params![title, rule_content, category, now, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn delete_growth(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM agent_growths WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_growth(conn: &Connection, id: &str) -> Result<Option<GrowthItem>, String> {
    let sql = "SELECT g.id, g.project_id, g.session_id, s.title, g.message_id, g.run_id,
                      g.trigger_type, g.trigger_context, g.reflection_thought,
                      g.category, g.title, g.rule_content, g.status, g.applied_count,
                      g.created_at, g.updated_at
               FROM agent_growths g
               LEFT JOIN sessions s ON g.session_id = s.id
               WHERE g.id = ?1";
    let res = conn
        .query_row(sql, params![id], |r| {
            Ok(GrowthItem {
                id: r.get(0)?,
                project_id: r.get(1)?,
                session_id: r.get(2)?,
                session_title: r.get(3)?,
                message_id: r.get(4)?,
                run_id: r.get(5)?,
                trigger_type: r.get(6)?,
                trigger_context: r.get(7)?,
                reflection_thought: r.get(8)?,
                category: r.get(9)?,
                title: r.get(10)?,
                rule_content: r.get(11)?,
                status: r.get(12)?,
                applied_count: r.get(13)?,
                created_at: r.get(14)?,
                updated_at: r.get(15)?,
            })
        })
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(res)
}

pub fn list_growths(
    conn: &Connection,
    project_id: Option<&str>,
    status: Option<&str>,
) -> Result<Vec<GrowthItem>, String> {
    let mut sql = "SELECT g.id, g.project_id, g.session_id, s.title, g.message_id, g.run_id,
                          g.trigger_type, g.trigger_context, g.reflection_thought,
                          g.category, g.title, g.rule_content, g.status, g.applied_count,
                          g.created_at, g.updated_at
                   FROM agent_growths g
                   LEFT JOIN sessions s ON g.session_id = s.id
                   WHERE 1=1".to_string();
    let mut params_vec: Vec<rusqlite::types::Value> = Vec::new();

    if let Some(pid) = project_id {
        if !pid.is_empty() {
            sql.push_str(" AND (g.project_id = ? OR g.project_id IS NULL)");
            params_vec.push(pid.to_string().into());
        }
    }

    if let Some(st) = status {
        if !st.is_empty() {
            sql.push_str(" AND g.status = ?");
            params_vec.push(st.to_string().into());
        }
    }

    sql.push_str(" ORDER BY g.updated_at DESC, g.created_at DESC");

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let params_slice: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|v| v as &dyn rusqlite::ToSql).collect();

    let rows = stmt
        .query_map(params_slice.as_slice(), |r| {
            Ok(GrowthItem {
                id: r.get(0)?,
                project_id: r.get(1)?,
                session_id: r.get(2)?,
                session_title: r.get(3)?,
                message_id: r.get(4)?,
                run_id: r.get(5)?,
                trigger_type: r.get(6)?,
                trigger_context: r.get(7)?,
                reflection_thought: r.get(8)?,
                category: r.get(9)?,
                title: r.get(10)?,
                rule_content: r.get(11)?,
                status: r.get(12)?,
                applied_count: r.get(13)?,
                created_at: r.get(14)?,
                updated_at: r.get(15)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(rows)
}

pub fn list_accepted_growths_for_project(
    conn: &Connection,
    project_id: &str,
) -> Result<Vec<GrowthItem>, String> {
    let sql = "SELECT g.id, g.project_id, g.session_id, s.title, g.message_id, g.run_id,
                      g.trigger_type, g.trigger_context, g.reflection_thought,
                      g.category, g.title, g.rule_content, g.status, g.applied_count,
                      g.created_at, g.updated_at
               FROM agent_growths g
               LEFT JOIN sessions s ON g.session_id = s.id
               WHERE (g.project_id = ?1 OR g.project_id IS NULL) AND g.status = 'accepted'
               ORDER BY g.applied_count DESC, g.created_at ASC";
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![project_id], |r| {
            Ok(GrowthItem {
                id: r.get(0)?,
                project_id: r.get(1)?,
                session_id: r.get(2)?,
                session_title: r.get(3)?,
                message_id: r.get(4)?,
                run_id: r.get(5)?,
                trigger_type: r.get(6)?,
                trigger_context: r.get(7)?,
                reflection_thought: r.get(8)?,
                category: r.get(9)?,
                title: r.get(10)?,
                rule_content: r.get(11)?,
                status: r.get(12)?,
                applied_count: r.get(13)?,
                created_at: r.get(14)?,
                updated_at: r.get(15)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(rows)
}

pub fn increment_growth_applied_count(conn: &Connection, ids: &[String]) -> Result<(), String> {
    for id in ids {
        let _ = conn.execute(
            "UPDATE agent_growths SET applied_count = applied_count + 1 WHERE id = ?1",
            params![id],
        );
    }
    Ok(())
}

// ---------- Token 消耗统计 ----------

pub fn get_token_stats(
    conn: &Connection,
    project_id: Option<&str>,
    days: Option<u32>,
) -> Result<TokenStatsReport, String> {
    let now_str = now();
    let today_prefix = if now_str.len() >= 10 {
        &now_str[0..10]
    } else {
        ""
    };

    // 1. 全局概览汇总
    let (tot_pt, tot_ct, tot_tt, tot_cached, tot_msgs): (i64, i64, i64, i64, i64) = conn
        .query_row(
            "SELECT 
                COALESCE(SUM(prompt_tokens), 0),
                COALESCE(SUM(completion_tokens), 0),
                COALESCE(SUM(total_tokens), 0),
                COALESCE(SUM(cached_tokens), 0),
                COUNT(*)
             FROM messages
             WHERE role = 'assistant'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap_or((0, 0, 0, 0, 0));

    let (today_pt, today_ct, today_tt, today_cached): (i64, i64, i64, i64) = conn
        .query_row(
            "SELECT 
                COALESCE(SUM(prompt_tokens), 0),
                COALESCE(SUM(completion_tokens), 0),
                COALESCE(SUM(total_tokens), 0),
                COALESCE(SUM(cached_tokens), 0)
             FROM messages
             WHERE role = 'assistant' AND substr(created_at, 1, 10) = ?1",
            params![today_prefix],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap_or((0, 0, 0, 0));

    let tot_sessions: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT session_id) FROM messages WHERE total_tokens > 0",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let summary = TokenStatsSummary {
        total_prompt_tokens: tot_pt as u64,
        total_completion_tokens: tot_ct as u64,
        total_tokens: tot_tt as u64,
        total_cached_tokens: tot_cached as u64,
        overall_cache_hit_rate: if tot_pt > 0 {
            ((tot_cached as f64 / tot_pt as f64) * 1000.0).round() / 10.0
        } else {
            0.0
        },
        today_prompt_tokens: today_pt as u64,
        today_completion_tokens: today_ct as u64,
        today_tokens: today_tt as u64,
        today_cached_tokens: today_cached as u64,
        today_cache_hit_rate: if today_pt > 0 {
            ((today_cached as f64 / today_pt as f64) * 1000.0).round() / 10.0
        } else {
            0.0
        },
        total_sessions: tot_sessions as u64,
        total_messages: tot_msgs as u64,
    };

    // 2. 按项目聚合
    let all_projects = list_projects(conn)?;
    let mut by_project: Vec<ProjectTokenStats> = Vec::new();

    let mut proj_stmt = conn
        .prepare(
            "SELECT 
                s.project_id,
                COALESCE(SUM(m.prompt_tokens), 0),
                COALESCE(SUM(m.completion_tokens), 0),
                COALESCE(SUM(m.total_tokens), 0),
                COALESCE(SUM(m.cached_tokens), 0),
                COUNT(DISTINCT s.id),
                COUNT(m.id),
                MAX(m.created_at)
             FROM sessions s
             JOIN messages m ON m.session_id = s.id
             WHERE m.role = 'assistant'
             GROUP BY s.project_id",
        )
        .map_err(|e| e.to_string())?;

    let proj_rows = proj_stmt
        .query_map([], |r| {
            let pid: Option<String> = r.get(0)?;
            let pt: i64 = r.get(1)?;
            let ct: i64 = r.get(2)?;
            let tt: i64 = r.get(3)?;
            let cached: i64 = r.get(4)?;
            let sc: i64 = r.get(5)?;
            let mc: i64 = r.get(6)?;
            let last_at: Option<String> = r.get(7)?;
            Ok((pid, pt as u64, ct as u64, tt as u64, cached as u64, sc as u64, mc as u64, last_at))
        })
        .map_err(|e| e.to_string())?;

    let mut mapped_stats: std::collections::HashMap<
        Option<String>,
        (u64, u64, u64, u64, u64, u64, Option<String>),
    > = std::collections::HashMap::new();

    for row in proj_rows.filter_map(|r| r.ok()) {
        mapped_stats.insert(row.0, (row.1, row.2, row.3, row.4, row.5, row.6, row.7));
    }

    for p in &all_projects {
        let stats = mapped_stats
            .remove(&Some(p.id.clone()))
            .unwrap_or((0, 0, 0, 0, 0, 0, None));
        let pt = stats.0;
        let cached = stats.3;
        let rate = if pt > 0 {
            ((cached as f64 / pt as f64) * 1000.0).round() / 10.0
        } else {
            0.0
        };
        by_project.push(ProjectTokenStats {
            project_id: Some(p.id.clone()),
            project_name: p.name.clone(),
            project_path: p.path.clone(),
            prompt_tokens: stats.0,
            completion_tokens: stats.1,
            total_tokens: stats.2,
            cached_tokens: cached,
            cache_hit_rate: rate,
            session_count: stats.4,
            message_count: stats.5,
            last_used_at: stats.6.or_else(|| p.last_activity_at.clone()),
        });
    }

    if let Some(stats) = mapped_stats.remove(&None) {
        if stats.2 > 0 || stats.5 > 0 {
            let pt = stats.0;
            let cached = stats.3;
            let rate = if pt > 0 {
                ((cached as f64 / pt as f64) * 1000.0).round() / 10.0
            } else {
                0.0
            };
            by_project.push(ProjectTokenStats {
                project_id: None,
                project_name: "未归类 / 纯对话".to_string(),
                project_path: None,
                prompt_tokens: stats.0,
                completion_tokens: stats.1,
                total_tokens: stats.2,
                cached_tokens: cached,
                cache_hit_rate: rate,
                session_count: stats.4,
                message_count: stats.5,
                last_used_at: stats.6,
            });
        }
    }

    by_project.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));

    // 3. 按时间聚合（每日趋势）
    let day_limit_sql = if let Some(d) = days {
        if d > 0 {
            let cutoff = chrono::Utc::now() - chrono::Duration::days(d as i64);
            format!("AND m.created_at >= '{}'", cutoff.to_rfc3339())
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    let proj_filter_sql = if let Some(pid) = project_id {
        if !pid.is_empty() {
            if pid == "unassigned" {
                "AND (s.project_id IS NULL OR s.project_id = '')".to_string()
            } else {
                format!("AND s.project_id = '{pid}'")
            }
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    let time_query = format!(
        "SELECT 
            substr(m.created_at, 1, 10) as day,
            COALESCE(SUM(m.prompt_tokens), 0),
            COALESCE(SUM(m.completion_tokens), 0),
            COALESCE(SUM(m.total_tokens), 0),
            COALESCE(SUM(m.cached_tokens), 0),
            COUNT(m.id)
         FROM messages m
         JOIN sessions s ON s.id = m.session_id
         WHERE m.role = 'assistant' {day_limit_sql} {proj_filter_sql}
         GROUP BY day
         ORDER BY day ASC"
    );

    let mut time_stmt = conn.prepare(&time_query).map_err(|e| e.to_string())?;
    let by_time: Vec<DailyTokenStats> = time_stmt
        .query_map([], |r| {
            let day: String = r.get(0)?;
            let pt: i64 = r.get(1)?;
            let ct: i64 = r.get(2)?;
            let tt: i64 = r.get(3)?;
            let cached: i64 = r.get(4)?;
            let mc: i64 = r.get(5)?;
            let rate = if pt > 0 {
                ((cached as f64 / pt as f64) * 1000.0).round() / 10.0
            } else {
                0.0
            };
            Ok(DailyTokenStats {
                date: day,
                prompt_tokens: pt as u64,
                completion_tokens: ct as u64,
                total_tokens: tt as u64,
                cached_tokens: cached as u64,
                cache_hit_rate: rate,
                message_count: mc as u64,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    // 4. 会话排行 (Top 50)
    let session_query = format!(
        "SELECT 
            s.id,
            s.title,
            s.project_id,
            p.name,
            COALESCE(SUM(m.prompt_tokens), 0) as pt,
            COALESCE(SUM(m.completion_tokens), 0) as ct,
            COALESCE(SUM(m.total_tokens), 0) as tt,
            COALESCE(SUM(m.cached_tokens), 0) as cached,
            COUNT(m.id) as mc,
            MAX(m.created_at) as last_msg
         FROM sessions s
         LEFT JOIN projects p ON p.id = s.project_id
         JOIN messages m ON m.session_id = s.id
         WHERE m.role = 'assistant' {proj_filter_sql}
         GROUP BY s.id
         HAVING tt > 0
         ORDER BY tt DESC
         LIMIT 50"
    );

    let mut sess_stmt = conn.prepare(&session_query).map_err(|e| e.to_string())?;
    let by_session: Vec<SessionTokenStats> = sess_stmt
        .query_map([], |r| {
            let sid: String = r.get(0)?;
            let title: String = r.get(1)?;
            let pid: Option<String> = r.get(2)?;
            let pname: Option<String> = r.get(3)?;
            let pt: i64 = r.get(4)?;
            let ct: i64 = r.get(5)?;
            let tt: i64 = r.get(6)?;
            let cached: i64 = r.get(7)?;
            let mc: i64 = r.get(8)?;
            let last_at: Option<String> = r.get(9)?;
            let rate = if pt > 0 {
                ((cached as f64 / pt as f64) * 1000.0).round() / 10.0
            } else {
                0.0
            };
            Ok(SessionTokenStats {
                session_id: sid,
                title,
                project_id: pid,
                project_name: pname,
                prompt_tokens: pt as u64,
                completion_tokens: ct as u64,
                total_tokens: tt as u64,
                cached_tokens: cached as u64,
                cache_hit_rate: rate,
                message_count: mc as u64,
                last_message_at: last_at,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    Ok(TokenStatsReport {
        summary,
        by_project,
        by_time,
        by_session,
    })
}

// ==================== 长任务（Long-Running Task）存储接口 ====================

pub fn create_long_task(conn: &Connection, task: &LongTask) -> Result<(), String> {
    let subtasks_json = serde_json::to_string(&task.subtasks).unwrap_or_else(|_| "[]".to_string());
    conn.execute(
        "INSERT INTO long_tasks (
            id, session_id, workspace_path, goal, status,
            current_subtask_index, subtasks_json, max_budget_tokens,
            total_tokens_used, current_step, max_steps, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            task.id,
            task.session_id,
            task.workspace_path,
            task.goal,
            task.status,
            task.current_subtask_index as i64,
            subtasks_json,
            task.max_budget_tokens.map(|b| b as i64),
            task.total_tokens_used as i64,
            task.current_step as i64,
            task.max_steps as i64,
            task.created_at,
            task.updated_at,
        ],
    )
    .map_err(|e| format!("创建长任务失败: {e}"))?;
    Ok(())
}

pub fn get_long_task(conn: &Connection, id: &str) -> Result<Option<LongTask>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, session_id, workspace_path, goal, status,
                    current_subtask_index, subtasks_json, max_budget_tokens,
                    total_tokens_used, current_step, max_steps, created_at, updated_at
             FROM long_tasks WHERE id = ?1",
        )
        .map_err(|e| e.to_string())?;

    let res = stmt
        .query_row(params![id], |r| {
            let subtasks_str: String = r.get(6)?;
            let subtasks: Vec<TaskSubItem> = serde_json::from_str(&subtasks_str).unwrap_or_default();
            let budget_tok: Option<i64> = r.get(7)?;
            let total_tok: i64 = r.get(8)?;
            let cur_step: i64 = r.get(9)?;
            let max_s: i64 = r.get(10)?;
            let cur_sub_idx: i64 = r.get(5)?;
            Ok(LongTask {
                id: r.get(0)?,
                session_id: r.get(1)?,
                workspace_path: r.get(2)?,
                goal: r.get(3)?,
                status: r.get(4)?,
                current_subtask_index: cur_sub_idx as usize,
                subtasks,
                max_budget_tokens: budget_tok.map(|b| b as u64),
                total_tokens_used: total_tok as u64,
                current_step: cur_step as usize,
                max_steps: max_s as usize,
                created_at: r.get(11)?,
                updated_at: r.get(12)?,
            })
        })
        .optional()
        .map_err(|e| e.to_string())?;

    Ok(res)
}

pub fn get_active_long_task(conn: &Connection, session_id: &str) -> Result<Option<LongTask>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, session_id, workspace_path, goal, status,
                    current_subtask_index, subtasks_json, max_budget_tokens,
                    total_tokens_used, current_step, max_steps, created_at, updated_at
             FROM long_tasks
             WHERE session_id = ?1 AND status IN ('planning', 'running', 'paused', 'waiting_approval', 'failed', 'interrupted')
             ORDER BY updated_at DESC LIMIT 1",
        )
        .map_err(|e| e.to_string())?;

    let res = stmt
        .query_row(params![session_id], |r| {
            let subtasks_str: String = r.get(6)?;
            let subtasks: Vec<TaskSubItem> = serde_json::from_str(&subtasks_str).unwrap_or_default();
            let budget_tok: Option<i64> = r.get(7)?;
            let total_tok: i64 = r.get(8)?;
            let cur_step: i64 = r.get(9)?;
            let max_s: i64 = r.get(10)?;
            let cur_sub_idx: i64 = r.get(5)?;
            Ok(LongTask {
                id: r.get(0)?,
                session_id: r.get(1)?,
                workspace_path: r.get(2)?,
                goal: r.get(3)?,
                status: r.get(4)?,
                current_subtask_index: cur_sub_idx as usize,
                subtasks,
                max_budget_tokens: budget_tok.map(|b| b as u64),
                total_tokens_used: total_tok as u64,
                current_step: cur_step as usize,
                max_steps: max_s as usize,
                created_at: r.get(11)?,
                updated_at: r.get(12)?,
            })
        })
        .optional()
        .map_err(|e| e.to_string())?;

    Ok(res)
}

pub fn update_long_task(conn: &Connection, task: &LongTask) -> Result<(), String> {
    let subtasks_json = serde_json::to_string(&task.subtasks).unwrap_or_else(|_| "[]".to_string());
    conn.execute(
        "UPDATE long_tasks SET
            status = ?1,
            current_subtask_index = ?2,
            subtasks_json = ?3,
            total_tokens_used = ?4,
            current_step = ?5,
            max_steps = ?6,
            updated_at = ?7
         WHERE id = ?8",
        params![
            task.status,
            task.current_subtask_index as i64,
            subtasks_json,
            task.total_tokens_used as i64,
            task.current_step as i64,
            task.max_steps as i64,
            task.updated_at,
            task.id,
        ],
    )
    .map_err(|e| format!("更新长任务失败: {e}"))?;
    Ok(())
}

pub fn update_long_task_status(conn: &Connection, task_id: &str, status: &str) -> Result<(), String> {
    let now = now();
    conn.execute(
        "UPDATE long_tasks SET status = ?1, updated_at = ?2 WHERE id = ?3",
        params![status, now, task_id],
    )
    .map_err(|e| format!("更新长任务状态失败: {e}"))?;
    Ok(())
}

pub fn save_task_checkpoint(conn: &Connection, cp: &TaskCheckpoint) -> Result<(), String> {
    conn.execute(
        "INSERT INTO task_checkpoints (
            id, task_id, step_number, subtask_id, status,
            summary, working_memory, git_commit_hash, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            cp.id,
            cp.task_id,
            cp.step_number as i64,
            cp.subtask_id,
            cp.status,
            cp.summary,
            cp.working_memory,
            cp.git_commit_hash,
            cp.created_at,
        ],
    )
    .map_err(|e| format!("保存任务检查点失败: {e}"))?;
    Ok(())
}

pub fn list_task_checkpoints(conn: &Connection, task_id: &str) -> Result<Vec<TaskCheckpoint>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, task_id, step_number, subtask_id, status,
                    summary, working_memory, git_commit_hash, created_at
             FROM task_checkpoints
             WHERE task_id = ?1
             ORDER BY step_number ASC",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map(params![task_id], |r| {
            let step_num: i64 = r.get(2)?;
            Ok(TaskCheckpoint {
                id: r.get(0)?,
                task_id: r.get(1)?,
                step_number: step_num as usize,
                subtask_id: r.get(3)?,
                status: r.get(4)?,
                summary: r.get(5)?,
                working_memory: r.get(6)?,
                git_commit_hash: r.get(7)?,
                created_at: r.get(8)?,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut list = Vec::new();
    for r in rows {
        if let Ok(cp) = r {
            list.push(cp);
        }
    }
    Ok(list)
}

#[allow(dead_code)]
pub fn get_latest_checkpoint(conn: &Connection, task_id: &str) -> Result<Option<TaskCheckpoint>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, task_id, step_number, subtask_id, status,
                    summary, working_memory, git_commit_hash, created_at
             FROM task_checkpoints
             WHERE task_id = ?1
             ORDER BY step_number DESC LIMIT 1",
        )
        .map_err(|e| e.to_string())?;

    let res = stmt
        .query_row(params![task_id], |r| {
            let step_num: i64 = r.get(2)?;
            Ok(TaskCheckpoint {
                id: r.get(0)?,
                task_id: r.get(1)?,
                step_number: step_num as usize,
                subtask_id: r.get(3)?,
                status: r.get(4)?,
                summary: r.get(5)?,
                working_memory: r.get(6)?,
                git_commit_hash: r.get(7)?,
                created_at: r.get(8)?,
            })
        })
        .optional()
        .map_err(|e| e.to_string())?;

    Ok(res)
}

/// 应用启动时对齐并收敛上次异常退出/崩溃的遗留状态
pub fn startup_reconcile(conn: &Connection) -> Result<(), String> {
    let current_time = now();
    // 1. 批量收敛残留的 running runs 为 interrupted
    let _ = conn.execute(
        "UPDATE runs SET status = 'interrupted', finished_at = ?1 WHERE status = 'running'",
        params![current_time],
    );

    // 2. 批量收敛残留的 running tool_events 为 failed
    let _ = conn.execute(
        "UPDATE tool_events SET status = 'failed', result_text = '[宿主进程异常退出，工具执行中断]' WHERE status = 'running'",
        [],
    );

    // 3. 批量收敛残留的 running/planning long_tasks 为 interrupted
    let _ = conn.execute(
        "UPDATE long_tasks SET status = 'interrupted', updated_at = ?1 WHERE status IN ('running', 'planning')",
        params![current_time],
    );

    // 4. 批量收敛残留的 running sessions (subprocess, subagent, collaborator) 为 interrupted
    let _ = conn.execute(
        "UPDATE sessions SET status = 'interrupted' WHERE status = 'running' AND (session_type IN ('subprocess', 'subagent', 'collaborator') OR parent_session_id IS NOT NULL)",
        [],
    );

    Ok(())
}

// ---------- Tool File Snapshots (影子快照元数据) ----------

pub fn insert_tool_file_snapshot(conn: &Connection, snap: &ToolFileSnapshot) -> Result<(), String> {
    conn.execute(
        "INSERT INTO tool_file_snapshots (
            id, session_id, message_id, tool_event_id, file_path, before_hash, after_hash, is_new_file, reverted_at, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            snap.id,
            snap.session_id,
            snap.message_id,
            snap.tool_event_id,
            snap.file_path,
            snap.before_hash,
            snap.after_hash,
            snap.is_new_file as i64,
            snap.reverted_at,
            snap.created_at,
        ],
    )
    .map_err(|e| e.to_string())?;

    // 50 轮滑动窗口自动淘汰
    let _ = prune_session_snapshots_sliding_window(conn, &snap.session_id, 50);

    Ok(())
}

pub fn list_snapshots_for_message(conn: &Connection, message_id: &str) -> Result<Vec<ToolFileSnapshot>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, session_id, message_id, tool_event_id, file_path, before_hash, after_hash, is_new_file, reverted_at, created_at
             FROM tool_file_snapshots WHERE message_id = ?1 ORDER BY created_at ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![message_id], |r| {
            Ok(ToolFileSnapshot {
                id: r.get(0)?,
                session_id: r.get(1)?,
                message_id: r.get(2)?,
                tool_event_id: r.get(3)?,
                file_path: r.get(4)?,
                before_hash: r.get(5)?,
                after_hash: r.get(6)?,
                is_new_file: r.get::<_, i64>(7)? != 0,
                reverted_at: r.get(8)?,
                created_at: r.get(9)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// 获取整轮对话（同一个 run_id 或指定 message_id）关联的所有快照
pub fn list_snapshots_for_turn(conn: &Connection, message_id: &str) -> Result<Vec<ToolFileSnapshot>, String> {
    let run_id: Option<String> = conn
        .query_row(
            "SELECT run_id FROM messages WHERE id = ?1",
            params![message_id],
            |r| r.get(0),
        )
        .optional()
        .unwrap_or(None)
        .flatten();

    if let Some(rid) = run_id {
        if !rid.is_empty() {
            let mut stmt = conn
                .prepare(
                    "SELECT s.id, s.session_id, s.message_id, s.tool_event_id, s.file_path, s.before_hash, s.after_hash, s.is_new_file, s.reverted_at, s.created_at
                     FROM tool_file_snapshots s
                     JOIN messages m ON s.message_id = m.id
                     WHERE m.run_id = ?1 ORDER BY s.created_at ASC",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(params![rid], |r| {
                    Ok(ToolFileSnapshot {
                        id: r.get(0)?,
                        session_id: r.get(1)?,
                        message_id: r.get(2)?,
                        tool_event_id: r.get(3)?,
                        file_path: r.get(4)?,
                        before_hash: r.get(5)?,
                        after_hash: r.get(6)?,
                        is_new_file: r.get::<_, i64>(7)? != 0,
                        reverted_at: r.get(8)?,
                        created_at: r.get(9)?,
                    })
                })
                .map_err(|e| e.to_string())?;
            let snaps = rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
            if !snaps.is_empty() {
                return Ok(snaps);
            }
        }
    }

    list_snapshots_for_message(conn, message_id)
}

/// 查询某个 seq 之后的所有有效快照（用于重新编辑时回退此后的全部修改，按 LIFO 逆序排列）
pub fn list_snapshots_after_seq(
    conn: &Connection,
    session_id: &str,
    seq: i64,
) -> Result<Vec<ToolFileSnapshot>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT s.id, s.session_id, s.message_id, s.tool_event_id, s.file_path, s.before_hash, s.after_hash, s.is_new_file, s.reverted_at, s.created_at
             FROM tool_file_snapshots s
             JOIN messages m ON s.message_id = m.id
             WHERE m.session_id = ?1 AND m.seq > ?2 AND s.reverted_at IS NULL
             ORDER BY s.created_at DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![session_id, seq], |r| {
            Ok(ToolFileSnapshot {
                id: r.get(0)?,
                session_id: r.get(1)?,
                message_id: r.get(2)?,
                tool_event_id: r.get(3)?,
                file_path: r.get(4)?,
                before_hash: r.get(5)?,
                after_hash: r.get(6)?,
                is_new_file: r.get::<_, i64>(7)? != 0,
                reverted_at: r.get(8)?,
                created_at: r.get(9)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

pub fn list_snapshots_for_event(conn: &Connection, tool_event_id: &str) -> Result<Vec<ToolFileSnapshot>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, session_id, message_id, tool_event_id, file_path, before_hash, after_hash, is_new_file, reverted_at, created_at
             FROM tool_file_snapshots WHERE tool_event_id = ?1 ORDER BY created_at ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![tool_event_id], |r| {
            Ok(ToolFileSnapshot {
                id: r.get(0)?,
                session_id: r.get(1)?,
                message_id: r.get(2)?,
                tool_event_id: r.get(3)?,
                file_path: r.get(4)?,
                before_hash: r.get(5)?,
                after_hash: r.get(6)?,
                is_new_file: r.get::<_, i64>(7)? != 0,
                reverted_at: r.get(8)?,
                created_at: r.get(9)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

pub fn list_snapshots_for_session(conn: &Connection, session_id: &str) -> Result<Vec<ToolFileSnapshot>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, session_id, message_id, tool_event_id, file_path, before_hash, after_hash, is_new_file, reverted_at, created_at
             FROM tool_file_snapshots WHERE session_id = ?1 ORDER BY created_at ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![session_id], |r| {
            Ok(ToolFileSnapshot {
                id: r.get(0)?,
                session_id: r.get(1)?,
                message_id: r.get(2)?,
                tool_event_id: r.get(3)?,
                file_path: r.get(4)?,
                before_hash: r.get(5)?,
                after_hash: r.get(6)?,
                is_new_file: r.get::<_, i64>(7)? != 0,
                reverted_at: r.get(8)?,
                created_at: r.get(9)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

pub fn mark_snapshots_reverted_for_message(conn: &Connection, message_id: &str, reverted_at: Option<&str>) -> Result<(), String> {
    conn.execute(
        "UPDATE tool_file_snapshots SET reverted_at = ?2 WHERE message_id = ?1",
        params![message_id, reverted_at],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE messages SET reverted_at = ?2 WHERE id = ?1",
        params![message_id, reverted_at],
    )
    .map_err(|e| e.to_string())?;
    let _ = conn.execute(
        "UPDATE messages SET reverted_at = ?2 WHERE tool_call_id IN (SELECT tool_call_id FROM tool_events WHERE message_id = ?1 AND tool_call_id IS NOT NULL)",
        params![message_id, reverted_at],
    );
    Ok(())
}

/// 标记整轮对话（包括同一 run_id 下的所有消息及触发该 run 的用户提问消息）及其快照为已撤回/激活态，并返回受影响的所有 message_id
pub fn mark_snapshots_reverted_for_turn(
    conn: &Connection,
    message_id: &str,
    reverted_at: Option<&str>,
) -> Result<Vec<String>, String> {
    let mut run_id: Option<String> = conn
        .query_row(
            "SELECT run_id FROM messages WHERE id = ?1",
            params![message_id],
            |r| r.get(0),
        )
        .optional()
        .unwrap_or(None)
        .flatten();

    if run_id.is_none() {
        run_id = conn
            .query_row(
                "SELECT id FROM runs WHERE trigger_message_id = ?1 ORDER BY rowid DESC LIMIT 1",
                params![message_id],
                |r| r.get(0),
            )
            .optional()
            .unwrap_or(None);
    }

    let target_msg_ids: Vec<String> = if let Some(ref rid) = run_id {
        if !rid.is_empty() {
            let mut ids = Vec::new();
            let mut stmt = conn
                .prepare("SELECT id FROM messages WHERE run_id = ?1")
                .map_err(|e| e.to_string())?;
            let rows = stmt.query_map(params![rid], |r| r.get(0)).map_err(|e| e.to_string())?;
            for r in rows {
                if let Ok(id) = r {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
            }

            // 查找 runs 表中的 trigger_message_id (用户提问)
            let trigger_id: Option<String> = conn
                .query_row(
                    "SELECT trigger_message_id FROM runs WHERE id = ?1",
                    params![rid],
                    |r| r.get(0),
                )
                .optional()
                .unwrap_or(None)
                .flatten();
            if let Some(tid) = trigger_id {
                if !ids.contains(&tid) {
                    ids.push(tid);
                }
            }

            // 兜底检查：若仍未包含 user 角色消息，向前寻找到该轮次最近的 user 消息
            let has_user = {
                let mut found = false;
                for mid in &ids {
                    let role: Option<String> = conn
                        .query_row("SELECT role FROM messages WHERE id = ?1", params![mid], |r| r.get(0))
                        .optional()
                        .unwrap_or(None);
                    if role.as_deref() == Some("user") {
                        found = true;
                        break;
                    }
                }
                found
            };
            if !has_user {
                let min_seq_res: Option<(i64, String)> = conn
                    .query_row(
                        "SELECT MIN(seq), session_id FROM messages WHERE run_id = ?1 GROUP BY session_id",
                        params![rid],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()
                    .unwrap_or(None);
                if let Some((mseq, sid)) = min_seq_res {
                    let prev_user: Option<String> = conn
                        .query_row(
                            "SELECT id FROM messages WHERE session_id = ?1 AND role = 'user' AND seq <= ?2 ORDER BY seq DESC LIMIT 1",
                            params![sid, mseq],
                            |r| r.get(0),
                        )
                        .optional()
                        .unwrap_or(None);
                    if let Some(puid) = prev_user {
                        if !ids.contains(&puid) {
                            ids.push(puid);
                        }
                    }
                }
            }

            if ids.is_empty() {
                vec![message_id.to_string()]
            } else {
                ids
            }
        } else {
            vec![message_id.to_string()]
        }
    } else {
        vec![message_id.to_string()]
    };

    for mid in &target_msg_ids {
        let _ = mark_snapshots_reverted_for_message(conn, mid, reverted_at);
    }

    Ok(target_msg_ids)
}

pub fn mark_snapshot_reverted_for_event(conn: &Connection, tool_event_id: &str, reverted_at: Option<&str>) -> Result<(), String> {
    conn.execute(
        "UPDATE tool_file_snapshots SET reverted_at = ?2 WHERE tool_event_id = ?1",
        params![tool_event_id, reverted_at],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 按照人机交互轮次（Turn，以 message_id 为原子单元）维护 50 轮滑动窗口。
/// 当会话中快照所属的不同轮次数超过 max_turns 时，原子淘汰最老的多余轮次快照元数据。
pub fn prune_session_snapshots_sliding_window(
    conn: &Connection,
    session_id: &str,
    max_turns: usize,
) -> Result<usize, String> {
    if max_turns == 0 {
        return Ok(0);
    }

    // 1. 查询当前会话中所有包含快照的 message_id，按最早创建时间升序排列
    let mut stmt = conn
        .prepare(
            "SELECT message_id, MIN(created_at) as first_seen
             FROM tool_file_snapshots
             WHERE session_id = ?1
             GROUP BY message_id
             ORDER BY first_seen ASC",
        )
        .map_err(|e| e.to_string())?;

    let turn_ids: Vec<String> = stmt
        .query_map(params![session_id], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    if turn_ids.len() <= max_turns {
        return Ok(0);
    }

    let excess_count = turn_ids.len() - max_turns;
    let evicted_turns = &turn_ids[..excess_count];

    let mut deleted_count = 0usize;
    for mid in evicted_turns {
        let n = conn
            .execute(
                "DELETE FROM tool_file_snapshots WHERE session_id = ?1 AND message_id = ?2",
                params![session_id, mid],
            )
            .map_err(|e| e.to_string())?;
        deleted_count += n;
    }

    Ok(deleted_count)
}

/// 收集数据库中所有有效引用的 CAS 哈希集合（用于后台 GC 对比）
pub fn collect_active_snapshot_hashes(conn: &Connection) -> Result<std::collections::HashSet<String>, String> {
    let mut hashes = std::collections::HashSet::new();
    let mut stmt = conn
        .prepare(
            "SELECT before_hash FROM tool_file_snapshots WHERE before_hash IS NOT NULL
             UNION
             SELECT after_hash FROM tool_file_snapshots",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;

    for h in rows {
        if let Ok(hash) = h {
            if !hash.is_empty() {
                hashes.insert(hash);
            }
        }
    }

    Ok(hashes)
}

