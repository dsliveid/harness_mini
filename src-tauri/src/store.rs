use crate::models::*;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

pub fn open_db(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    init_schema(&conn)?;
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
          constraints TEXT NOT NULL DEFAULT ''
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
    // 旧库迁移：projects 补 path / constraints 列（新建库的 CREATE TABLE 已含该列时跳过）
    ensure_column(conn, "projects", "path", "path TEXT")?;
    ensure_column(conn, "projects", "constraints", "constraints TEXT NOT NULL DEFAULT ''")?;
    // 临时空间对话字段
    ensure_column(conn, "sessions", "is_temp", "is_temp INTEGER NOT NULL DEFAULT 0")?;
    ensure_column(conn, "sessions", "temp_code", "temp_code TEXT")?;
    ensure_column(conn, "sessions", "temp_root", "temp_root TEXT")?;
    ensure_column(conn, "sessions", "source_workspace", "source_workspace TEXT")?;
    ensure_column(conn, "sessions", "merged_seq", "merged_seq INTEGER")?;
    ensure_column(conn, "sessions", "merged_pending", "merged_pending INTEGER NOT NULL DEFAULT 0")?;
    // 旧库迁移：messages 补 reasoning 列（模型思考过程）
    ensure_column(conn, "messages", "reasoning", "reasoning TEXT")?;
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

fn now() -> String {
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
                    (SELECT MAX(s.last_message_at) FROM sessions s WHERE s.project_id = p.id)
             FROM projects p",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Project {
                id: r.get(0)?,
                name: r.get(1)?,
                path: r.get(2)?,
                pinned: r.get::<_, i64>(3)? != 0,
                created_at: r.get(4)?,
                constraints: r.get(5)?,
                last_activity_at: r.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

pub fn get_project(conn: &Connection, id: &str) -> Result<Option<Project>, String> {
    conn.query_row(
        "SELECT id, name, path, pinned, created_at, constraints FROM projects WHERE id = ?1",
        params![id],
        |r| {
            Ok(Project {
                id: r.get(0)?,
                name: r.get(1)?,
                path: r.get(2)?,
                pinned: r.get::<_, i64>(3)? != 0,
                created_at: r.get(4)?,
                constraints: r.get(5)?,
                last_activity_at: None,
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
    };
    conn.execute(
        "INSERT INTO projects(id, name, path, pinned, created_at, constraints) VALUES(?1,?2,?3,0,?4,'')",
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
        let p = create_project(&conn, "demo", Some("D:\\demo")).unwrap();
        assert_eq!(p.path.as_deref(), Some("D:\\demo"));
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
    conn.execute(
        "UPDATE projects SET pinned = ?2 WHERE id = ?1",
        params![id, pinned as i64],
    )
    .map_err(|e| e.to_string())?;
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
    })
}

const SESSION_COLS: &str =
    "id, title, workspace_path, access_mode, project_id, status, last_message_at, created_at, updated_at, \
     is_temp, temp_code, temp_root, source_workspace, merged_seq, merged_pending";

pub fn get_session(conn: &Connection, id: &str) -> Result<Option<Session>, String> {
    conn.query_row(
        &format!("SELECT {SESSION_COLS} FROM sessions WHERE id = ?1"),
        params![id],
        row_to_session,
    )
    .optional()
    .map_err(|e| e.to_string())
}

pub fn list_sessions(conn: &Connection, status: &str) -> Result<Vec<Session>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {SESSION_COLS} FROM sessions WHERE status = ?1 ORDER BY COALESCE(last_message_at, created_at) DESC"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![status], row_to_session)
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

pub fn create_session(
    conn: &Connection,
    workspace_path: &str,
    project_id: Option<&str>,
    title: &str,
    access_mode: &str,
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
    };
    conn.execute(
        "INSERT INTO sessions(id, title, workspace_path, access_mode, project_id, status, last_message_at, created_at, updated_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            s.id,
            s.title,
            s.workspace_path,
            s.access_mode,
            s.project_id,
            s.status,
            s.last_message_at,
            s.created_at,
            s.updated_at
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(s)
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
        "INSERT INTO messages(id, session_id, run_id, seq, role, content, reasoning, tool_calls_json, tool_call_id, queued, usage_json, created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
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
            m.created_at
        ],
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
    let seq = next_seq(conn, session_id)?;
    let m = Message {
        id: uuid::Uuid::new_v4().to_string(),
        session_id: session_id.to_string(),
        run_id: None,
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
    };
    insert_message(conn, &m)?;
    Ok(m)
}

pub fn update_message_content(
    conn: &Connection,
    id: &str,
    content: &str,
    usage: Option<&serde_json::Value>,
) -> Result<(), String> {
    conn.execute(
        "UPDATE messages SET content = ?2, usage_json = COALESCE(?3, usage_json) WHERE id = ?1",
        params![id, content, usage.and_then(|v| serde_json::to_string(v).ok())],
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
) -> Result<(), String> {
    conn.execute(
        "UPDATE messages SET tool_calls_json = ?2, content = ?3 WHERE id = ?1",
        params![id, tool_calls.to_string(), content],
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
            "SELECT id, run_id, seq, role, content, reasoning, tool_calls_json, tool_call_id, queued, usage_json, created_at
             FROM messages WHERE session_id = ?1 AND seq < COALESCE(?2, 9223372036854775807)
             ORDER BY seq DESC LIMIT ?3",
        )
        .map_err(|e| e.to_string())?;
    let mut msgs: Vec<Message> = stmt
        .query_map(params![session_id, before_seq, limit], |r| {
            let tc: Option<String> = r.get(6)?;
            let usage: Option<String> = r.get(9)?;
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
            "SELECT id, message_id, tool_name, tool_call_id, params_json, result_text, status, approval_scope, created_at
             FROM tool_events WHERE message_id = ?1 ORDER BY rowid ASC",
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
    Ok(())
}

pub fn get_message(conn: &Connection, id: &str) -> Result<Option<Message>, String> {
    let opt = conn
        .query_row(
            "SELECT id, session_id, run_id, seq, role, content, reasoning, tool_calls_json, tool_call_id, queued, usage_json, created_at
             FROM messages WHERE id = ?1",
            params![id],
            |r| {
                let tc: Option<String> = r.get(7)?;
                let usage: Option<String> = r.get(10)?;
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
            "SELECT id, session_id, run_id, seq, role, content, reasoning, tool_calls_json, tool_call_id, queued, usage_json, created_at
             FROM messages WHERE session_id = ?1 AND queued = 1 ORDER BY seq ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![session_id], |r| {
            let tc: Option<String> = r.get(7)?;
            let usage: Option<String> = r.get(10)?;
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
        "INSERT INTO tool_events(id, message_id, tool_name, tool_call_id, params_json, result_text, status, approval_scope, created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            ev.id,
            ev.message_id,
            ev.tool_name,
            ev.tool_call_id,
            serde_json::to_string(&ev.params).map_err(|e| e.to_string())?,
            ev.result_text,
            ev.status,
            ev.approval_scope,
            ev.created_at
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
    conn.execute(
        "UPDATE tool_events SET status = ?2,
            result_text = COALESCE(?3, result_text),
            approval_scope = COALESCE(?4, approval_scope)
         WHERE id = ?1",
        params![id, status, result_text, approval_scope],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------- runs ----------

pub fn create_run(conn: &Connection, session_id: &str) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO runs(id, session_id, trigger_type, status, started_at) VALUES(?1,?2,'manual','running',?3)",
        params![id, session_id, now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(id)
}

pub fn finish_run(conn: &Connection, id: &str, status: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE runs SET status = ?2, ended_at = ?3 WHERE id = ?1",
        params![id, status, now()],
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
