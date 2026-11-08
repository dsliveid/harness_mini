//! 按月轮转日志库（logs/logs-YYYYMM.db）——审计态数据独立成库
//!
//! 背景与实测依据（详见 docs/SQLITE_THREE_DB_SCHEMA_REFERENCE.md §4）：
//! 1. 混库时日志长事务阻塞核心写 549ms，分库后 0.2ms → 日志必须独立成库；
//! 2. 窄行批量事务 16.4~31 倍加速，宽行仅 1.2 倍 → 行内只留摘要，>8KB 字段外置；
//! 3. `incremental_vacuum` 循环实测无法回收空间 → 日志库 auto_vacuum=NONE，
//!    空间回收靠「按 TTL 删整月文件 + 孤儿 blob GC」。
//!
//! 架构：
//! - 单写线程（std::thread，不依赖 tokio 运行时），调用方仅入队，永不阻塞在 DB/文件 IO 上；
//! - 有界队列（256），入队带 100ms 超时，队列满丢弃该条审计记录并告警（业务态不受影响）；
//! - 每 200 条或 200ms 触发一次 `BEGIN IMMEDIATE ... COMMIT` 批量提交；
//! - 跨月自动切换新文件（按记录 created_at 的 YYYYMM 路由）；
//! - TTL 清理走同一队列串行执行，避免与写线程争抢文件句柄（Windows 上打开中的 DB 无法删除）。

use super::blob;
use crate::models::LlmRequestTrace;
use chrono::Datelike;
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, RecvTimeoutError, SyncSender};
use std::time::{Duration, Instant};

/// 单批最大条数（达到即提前提交）
const BATCH_MAX: usize = 200;
/// 批量提交窗口：首批入队后最多等这么久
const BATCH_WINDOW: Duration = Duration::from_millis(200);
/// 队列容量：有界，防极端突发拖垮内存；投递非阻塞，满即丢弃审计记录
const QUEUE_CAPACITY: usize = 256;
/// blob 硬上限：超过直接截断，防病态输入写爆磁盘
const BLOB_HARD_CAP: usize = 4 * 1024 * 1024;
/// 日志外置 blob 的子目录名（相对 logs 根目录）
const LOGS_BLOBS_DIR: &str = "blobs";

/// 计算数据目录下的日志库根目录
pub fn logs_root(data_dir: &Path) -> PathBuf {
    data_dir.join("logs")
}

/// 从 RFC3339 时间串提取月份键（YYYYMM，如 "202511"）。
/// 非法输入按当前时间兜底（审计记录宁可落错月也不丢）。
pub fn month_of(created_at: &str) -> String {
    let b = created_at.as_bytes();
    if b.len() >= 7
        && b[0].is_ascii_digit()
        && b[1].is_ascii_digit()
        && b[2].is_ascii_digit()
        && b[3].is_ascii_digit()
        && b[4] == b'-'
        && b[5].is_ascii_digit()
        && b[6].is_ascii_digit()
    {
        format!("{}{}", &created_at[0..4], &created_at[5..7])
    } else {
        month_now()
    }
}

/// 当前月份键（本地时区）
pub fn month_now() -> String {
    chrono::Local::now().format("%Y%m").to_string()
}

// ---------- 队列记录 ----------

/// LLM 请求审计记录（写入 monthly llm_requests 表）
pub struct LlmRequestRecord {
    pub session_id: String,
    pub message_id: String,
    pub run_id: Option<String>,
    pub model: String,
    pub endpoint: String,
    pub payload_json: String,
    pub created_at: String,
}

/// LLM 生成性能回填（ttft_ms / tokens_per_sec 标量列，流式结束时一次性写）
pub struct LlmPerfUpdate {
    pub llm_request_id: String,
    pub ttft_ms: i64,
    pub tokens_per_sec: f64,
}

/// 工具调用出入参审计记录（写入 monthly tool_call_logs 表）
pub struct ToolCallLogRecord {
    pub session_id: String,
    pub message_id: String,
    pub tool_event_id: String,
    pub tool_name: String,
    /// 完整入参 JSON（>8KB 落库时外置）
    pub params_json: String,
    /// 完整出参（>8KB 落库时外置）
    pub result_text: Option<String>,
    pub status: String,
    pub duration_ms: i64,
    pub created_at: String,
}

enum Task {
    Llm { month: String, id: String, rec: LlmRequestRecord },
    Perf { month: String, upd: LlmPerfUpdate },
    Tool { month: String, id: String, rec: ToolCallLogRecord },
    Cleanup { retention_months: u32 },
}

// ---------- LogWriter 句柄 ----------

/// 日志写线程句柄。克隆廉价（内部仅 Sender）；`tx=None` 表示禁用
/// （数据目录尚未就绪的内存哨兵态，所有记录静默丢弃）。
pub struct LogWriter {
    tx: Option<Mutex<SyncSender<Task>>>,
}

impl LogWriter {
    /// 禁用态写句柄（记录全部丢弃）
    pub fn disabled() -> Self {
        Self { tx: None }
    }

    /// 创建写线程并绑定日志根目录
    pub fn new(root: PathBuf) -> Self {
        let (tx, rx) = sync_channel::<Task>(QUEUE_CAPACITY);
        std::thread::Builder::new()
            .name("log-writer".into())
            .spawn(move || writer_loop(rx, root))
            .expect("启动日志写线程失败");
        Self { tx: Some(Mutex::new(tx)) }
    }

    /// 热切换数据目录：旧写线程把队列存量刷完即退出，新线程接管新目录。
    pub fn rebind(&mut self, root: Option<PathBuf>) {
        // 先置 None 再重建：旧 Sender 随 Mutex 被 drop，写线程 recv 到 Disconnected
        // 后会把剩余队列刷完再退出，不丢已入队数据。
        self.tx = None;
        if let Some(root) = root {
            *self = LogWriter::new(root);
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.tx.is_some()
    }

    fn send(&self, task: Task) {
        if let Some(ref tx) = self.tx {
            let guard = tx.lock();
            let sender: &SyncSender<Task> = &guard;
            // 有界队列 + 非阻塞投递：队列满说明写线程积压严重（磁盘卡顿/线程退出），
            // 审计记录宁丢不堵——绝不阻塞业务写路径（这是本模块的硬性设计约束）。
            if sender.try_send(task).is_err() {
                eprintln!("[LogWriter] 队列满或写线程退出，丢弃一条审计记录");
            }
        }
    }

    /// 入队一条 LLM 请求审计；返回 (id, month)，id 供后续 perf 回填定位同一行，
    /// month 供 perf 回填路由到正确的月份文件。
    pub fn record_llm_request(&self, rec: LlmRequestRecord) -> (String, String) {
        let id = uuid::Uuid::new_v4().to_string();
        let month = month_of(&rec.created_at);
        self.send(Task::Llm { month: month.clone(), id: id.clone(), rec });
        (id, month)
    }

    /// 回填 LLM 生成性能标量（首 token 延迟与生成速度）
    pub fn record_llm_perf(&self, month: &str, upd: LlmPerfUpdate) {
        self.send(Task::Perf { month: month.to_string(), upd });
    }

    /// 入队一条工具调用审计
    pub fn record_tool_call(&self, rec: ToolCallLogRecord) {
        let id = uuid::Uuid::new_v4().to_string();
        let month = month_of(&rec.created_at);
        self.send(Task::Tool { month, id, rec });
    }

    /// 请求执行一次 TTL 清理（在写线程内串行执行）
    pub fn request_cleanup(&self, retention_months: u32) {
        self.send(Task::Cleanup { retention_months });
    }
}

// ---------- 写线程 ----------

fn writer_loop(rx: std::sync::mpsc::Receiver<Task>, root: PathBuf) {
    let mut conns: HashMap<String, Connection> = HashMap::new();
    loop {
        let first = match rx.recv() {
            Ok(t) => t,
            Err(_) => break, // 所有句柄丢弃：退出（队列已空）
        };
        let mut batch = vec![first];
        let deadline = Instant::now() + BATCH_WINDOW;
        let mut disconnected = false;
        while batch.len() < BATCH_MAX {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            match rx.recv_timeout(deadline - now) {
                Ok(t) => batch.push(t),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        flush_batch(&mut conns, &root, &batch);
        if disconnected {
            break;
        }
    }
}

/// 把一批任务按月分组落库。每个月份库一个事务；Cleanup 在全部写入提交后串行执行。
fn flush_batch(conns: &mut HashMap<String, Connection>, root: &Path, batch: &[Task]) {
    // 按月分组（保持到达顺序）
    let mut by_month: HashMap<&str, Vec<&Task>> = HashMap::new();
    let mut cleanups: Vec<u32> = Vec::new();
    for task in batch {
        match task {
            Task::Cleanup { retention_months } => cleanups.push(*retention_months),
            t => {
                let month = match t {
                    Task::Llm { month, .. } | Task::Perf { month, .. } | Task::Tool { month, .. } => month.as_str(),
                    Task::Cleanup { .. } => unreachable!(),
                };
                by_month.entry(month).or_default().push(t);
            }
        }
    }

    for (month, tasks) in by_month {
        if let Err(e) = flush_month(conns, root, month, &tasks) {
            eprintln!("[LogWriter] 月份 {month} 批量落库失败: {e}");
        }
    }

    for retention in cleanups {
        if let Err(e) = cleanup_expired(root, retention) {
            eprintln!("[LogWriter] 日志 TTL 清理失败: {e}");
        }
        // 清理可能删除了写线程仍持有的连接对应的文件，驱逐这些连接
        let cutoff = cutoff_month(retention);
        conns.retain(|month, _| month.as_str() >= cutoff.as_str());
    }
}

fn flush_month(
    conns: &mut HashMap<String, Connection>,
    root: &Path,
    month: &str,
    tasks: &[&Task],
) -> Result<(), String> {
    let conn = match conns.get_mut(month) {
        Some(c) => c,
        None => {
            let c = open_month_db(root, month)?;
            conns.insert(month.to_string(), c);
            conns.get_mut(month).unwrap()
        }
    };

    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| format!("开启事务失败: {e}"))?;
    let result = (|| {
        for task in tasks {
            match task {
                Task::Llm { id, rec, .. } => insert_llm_request(conn, root, id, rec)?,
                Task::Perf { upd, .. } => {
                    conn.execute(
                        "UPDATE llm_requests SET ttft_ms = ?2, tokens_per_sec = ?3 WHERE id = ?1",
                        params![upd.llm_request_id, upd.ttft_ms, upd.tokens_per_sec],
                    )
                    .map_err(|e| format!("回填 LLM 性能失败: {e}"))?;
                }
                Task::Tool { id, rec, .. } => insert_tool_call_log(conn, root, id, rec)?,
                Task::Cleanup { .. } => unreachable!(),
            }
        }
        Ok::<(), String>(())
    })();

    match result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(|e| format!("提交事务失败: {e}")),
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

// ---------- 月份库打开与 schema ----------

/// 打开（或创建）某月日志库。PRAGMA 建库前设定（实测结论 4/5）。
pub fn open_month_db(root: &Path, month: &str) -> Result<Connection, String> {
    std::fs::create_dir_all(root).map_err(|e| format!("创建 logs 目录失败: {e}"))?;
    let path = root.join(format!("logs-{month}.db"));
    let conn = Connection::open(&path)
        .map_err(|e| format!("打开日志库失败 ({}): {e}", path.display()))?;

    conn.execute_batch(
        "PRAGMA page_size = 8192;
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA busy_timeout = 5000;
         PRAGMA auto_vacuum = NONE;",
    )
    .map_err(|e| format!("配置日志库 PRAGMA 失败: {e}"))?;

    init_log_schema(&conn)?;
    Ok(conn)
}

/// 月度日志库 schema（权威定义见 docs/SQLITE_THREE_DB_SCHEMA_REFERENCE.md §4）
pub fn init_log_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS llm_requests (
          id TEXT PRIMARY KEY,
          session_id TEXT NOT NULL,
          message_id TEXT NOT NULL,
          run_id TEXT,
          model TEXT NOT NULL,
          endpoint TEXT NOT NULL,
          payload_json TEXT NOT NULL,
          payload_bytes INTEGER NOT NULL DEFAULT 0,
          payload_sha256 TEXT,
          ttft_ms INTEGER,
          tokens_per_sec REAL,
          created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_logs_llm_session ON llm_requests(session_id, created_at);
        CREATE INDEX IF NOT EXISTS idx_logs_llm_created ON llm_requests(created_at);
        CREATE INDEX IF NOT EXISTS idx_logs_llm_message ON llm_requests(message_id);

        CREATE TABLE IF NOT EXISTS tool_call_logs (
          id TEXT PRIMARY KEY,
          session_id TEXT NOT NULL,
          message_id TEXT NOT NULL,
          tool_event_id TEXT NOT NULL,
          tool_name TEXT NOT NULL,
          params_preview TEXT,
          params_blob TEXT,
          result_preview TEXT,
          result_blob TEXT,
          bytes INTEGER NOT NULL DEFAULT 0,
          status TEXT NOT NULL,
          duration_ms INTEGER NOT NULL DEFAULT 0,
          created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_tool_logs_session ON tool_call_logs(session_id, created_at);
        CREATE INDEX IF NOT EXISTS idx_tool_logs_created ON tool_call_logs(created_at);
        CREATE INDEX IF NOT EXISTS idx_tool_logs_name ON tool_call_logs(tool_name);
        "#,
    )
    .map_err(|e| format!("初始化日志库 Schema 失败: {e}"))?;
    blob::ensure_blob_schema(conn)
}

/// 列出全部月份文件，按月份键降序（新→旧）。
pub fn list_month_files(root: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(month) = name
                .strip_prefix("logs-")
                .and_then(|s| s.strip_suffix(".db"))
                .filter(|m| m.len() == 6 && m.bytes().all(|b| b.is_ascii_digit()))
            {
                out.push((month.to_string(), entry.path()));
            }
        }
    }
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out
}

// ---------- 落库与外置 ----------

/// >8KB 外置到日志命名空间 blobs；≤8KB 内联。返回 (行内文本, sha引用, 原始字节)。
fn externalize(conn: &Connection, blobs_root: &Path, text: &str) -> Result<(String, Option<String>, i64), String> {
    let capped = hard_cap(text);
    match blob::store_if_large(conn, blobs_root, &capped)? {
        Some(r) => Ok((r.preview, Some(r.sha256), r.bytes)),
        None => {
            let bytes = capped.len() as i64;
            Ok((capped, None, bytes))
        }
    }
}

/// 4MB 硬截断（字符边界安全）
fn hard_cap(text: &str) -> String {
    if text.len() <= BLOB_HARD_CAP {
        return text.to_string();
    }
    let mut end = BLOB_HARD_CAP;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n\n[...⚠️ 超过 {}MB 审计硬上限，已截断 ...]", &text[..end], BLOB_HARD_CAP / (1024 * 1024))
}

fn insert_llm_request(conn: &Connection, root: &Path, id: &str, rec: &LlmRequestRecord) -> Result<(), String> {
    let blobs_root = root.join(LOGS_BLOBS_DIR);
    let (payload_col, payload_sha, payload_bytes) = externalize(conn, &blobs_root, &rec.payload_json)?;
    conn.execute(
        r#"INSERT INTO llm_requests
             (id, session_id, message_id, run_id, model, endpoint, payload_json, payload_bytes, payload_sha256, created_at)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"#,
        params![
            id,
            rec.session_id,
            rec.message_id,
            rec.run_id,
            rec.model,
            rec.endpoint,
            payload_col,
            payload_bytes,
            payload_sha,
            rec.created_at
        ],
    )
    .map_err(|e| format!("写入 llm_requests 失败: {e}"))?;
    Ok(())
}

fn insert_tool_call_log(conn: &Connection, root: &Path, id: &str, rec: &ToolCallLogRecord) -> Result<(), String> {
    let blobs_root = root.join(LOGS_BLOBS_DIR);
    let (params_col, params_sha, _) = externalize(conn, &blobs_root, &rec.params_json)?;
    let (result_col, result_sha, result_bytes) = match &rec.result_text {
        Some(t) => externalize(conn, &blobs_root, t)?,
        None => (String::new(), None, 0i64),
    };
    let total_bytes = rec.params_json.len() as i64 + result_bytes;
    conn.execute(
        r#"INSERT INTO tool_call_logs
             (id, session_id, message_id, tool_event_id, tool_name,
              params_preview, params_blob, result_preview, result_blob,
              bytes, status, duration_ms, created_at)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"#,
        params![
            id,
            rec.session_id,
            rec.message_id,
            rec.tool_event_id,
            rec.tool_name,
            params_col,
            params_sha,
            result_col,
            result_sha,
            total_bytes,
            rec.status,
            rec.duration_ms,
            rec.created_at
        ],
    )
    .map_err(|e| format!("写入 tool_call_logs 失败: {e}"))?;
    Ok(())
}

// ---------- TTL 清理与 blob GC ----------

/// 计算保留边界月份键：早于（不含）该月的文件可删除。
/// 语义：保留「最近 N 个整月 + 当月」（N=retention_months）。
fn cutoff_month(retention_months: u32) -> String {
    let now = chrono::Local::now().date_naive();
    let total = now.year() as i64 * 12 + (now.month() as i64 - 1) - retention_months as i64;
    let y = total.div_euclid(12);
    let m = total.rem_euclid(12) + 1;
    format!("{y:04}{m:02}")
}

/// 按月 TTL 清理：删除过期月份库文件（含 -wal/-shm），随后 GC 日志命名空间孤儿 blob。
/// 返回删除的月份库数量。
pub fn cleanup_expired(root: &Path, retention_months: u32) -> Result<usize, String> {
    let cutoff = cutoff_month(retention_months);
    let mut removed = 0usize;
    for (month, path) in list_month_files(root) {
        if month < cutoff {
            for suffix in ["", "-wal", "-shm"] {
                let mut p = path.clone().into_os_string();
                p.push(suffix);
                let _ = std::fs::remove_file(&p);
            }
            removed += 1;
        }
    }
    if removed > 0 {
        gc_log_blobs(root);
    }
    Ok(removed)
}

/// GC 日志命名空间孤儿 blob：删除未被任何剩余月份库引用的物理文件。
fn gc_log_blobs(root: &Path) {
    let blobs_root = root.join(LOGS_BLOBS_DIR);
    if !blobs_root.is_dir() {
        return;
    }
    // 收集全部剩余月份库引用的 rel_path
    let mut referenced: HashSet<String> = HashSet::new();
    for (_, path) in list_month_files(root) {
        if let Ok(conn) = Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) {
            if let Ok(mut stmt) = conn.prepare("SELECT rel_path FROM blobs") {
                if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
                    for rel in rows.flatten() {
                        referenced.insert(rel);
                    }
                }
            }
        }
    }
    // 遍历物理文件，删除未引用项
    for depth_a in std::fs::read_dir(&blobs_root).into_iter().flatten() {
        let Ok(depth_a) = depth_a else { continue };
        if !depth_a.path().is_dir() {
            continue;
        }
        for depth_b in std::fs::read_dir(depth_a.path()).into_iter().flatten() {
            let Ok(depth_b) = depth_b else { continue };
            if !depth_b.path().is_dir() {
                continue;
            }
            for file in std::fs::read_dir(depth_b.path()).into_iter().flatten() {
                let Ok(file) = file else { continue };
                let fpath = file.path();
                if !fpath.is_file() {
                    continue;
                }
                let rel = match fpath.strip_prefix(&blobs_root) {
                    Ok(p) => p.to_string_lossy().replace('\\', "/"),
                    Err(_) => continue,
                };
                if !referenced.contains(&rel) {
                    let _ = std::fs::remove_file(&fpath);
                }
            }
            let _ = std::fs::remove_dir(depth_b.path()); // 空目录才会成功
        }
        let _ = std::fs::remove_dir(depth_a.path());
    }
}

// ---------- 读路径 ----------

/// 按 message_id 查询最近的 LLM 请求审计（新月份优先）。
/// payload 已外置时懒加载 blob 全文；blob 缺失（TTL 已清）则降级为 preview。
pub fn load_llm_trace(root: &Path, message_id: &str) -> Result<Option<LlmRequestTrace>, String> {
    for (month, path) in list_month_files(root) {
        let _ = month;
        let conn = match Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let row = conn
            .query_row(
                r#"SELECT id, session_id, message_id, run_id, model, endpoint,
                          payload_json, payload_bytes, payload_sha256, created_at
                   FROM llm_requests
                   WHERE message_id = ?1
                   ORDER BY created_at DESC
                   LIMIT 1"#,
                params![message_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, String>(6)?,
                        r.get::<_, i64>(7)?,
                        r.get::<_, Option<String>>(8)?,
                        r.get::<_, String>(9)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| e.to_string())?;

        if let Some((id, session_id, message_id, run_id, model, endpoint, payload_json, payload_bytes, payload_sha256, created_at)) = row {
            let payload = match payload_sha256 {
                Some(sha) => {
                    let blobs_root = root.join(LOGS_BLOBS_DIR);
                    blob::load_by_sha(&conn, &blobs_root, &sha)
                        .unwrap_or_else(|_| payload_json.clone())
                }
                None => payload_json,
            };
            let _ = payload_bytes;
            return Ok(Some(LlmRequestTrace {
                id,
                session_id,
                message_id,
                run_id,
                model,
                endpoint,
                payload_json: payload,
                created_at,
            }));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::blob::INLINE_THRESHOLD;

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("hm_logs_test_{tag}_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn rec(session: &str, created_at: &str) -> LlmRequestRecord {
        LlmRequestRecord {
            session_id: session.into(),
            message_id: "msg-1".into(),
            run_id: Some("run-1".into()),
            model: "test-model".into(),
            endpoint: "https://api.example.com".into(),
            payload_json: r#"{"messages":[{"role":"user","content":"hi"}]}"#.into(),
            created_at: created_at.into(),
        }
    }

    #[test]
    fn test_month_of() {
        assert_eq!(month_of("2025-11-07T09:00:00+00:00"), "202511");
        assert_eq!(month_of("2026-01-01T00:00:00Z"), "202601");
        assert_eq!(month_of("garbage").len(), 6, "非法输入应兜底为当前月");
    }

    #[test]
    fn test_batch_write_and_month_routing() {
        let root = temp_root("routing");
        let writer = LogWriter::new(logs_root(&root));

        // 两个不同月份的记录 → 应路由到两个文件
        writer.record_llm_request(rec("s1", "2025-10-15T10:00:00Z"));
        writer.record_llm_request(rec("s1", "2025-11-01T10:00:00Z"));

        // 等待批量窗口（200ms）落库
        std::thread::sleep(Duration::from_millis(500));
        drop(writer); // 触发 flush-and-exit

        let months = list_month_files(&logs_root(&root));
        let names: Vec<&str> = months.iter().map(|(m, _)| m.as_str()).collect();
        assert!(names.contains(&"202510"), "应存在 10 月文件: {names:?}");
        assert!(names.contains(&"202511"), "应存在 11 月文件: {names:?}");

        let conn = Connection::open(&logs_root(&root).join("logs-202511.db")).unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM llm_requests", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_perf_update_and_trace_roundtrip() {
        let root = temp_root("perf");
        let writer = LogWriter::new(logs_root(&root));

        let (req_id, month) = writer.record_llm_request(rec("s1", "2025-11-07T10:00:00Z"));
        writer.record_llm_perf(&month, LlmPerfUpdate { llm_request_id: req_id, ttft_ms: 320, tokens_per_sec: 58.5 });
        std::thread::sleep(Duration::from_millis(500));
        drop(writer);

        let trace = load_llm_trace(&logs_root(&root), "msg-1").unwrap().expect("应能查到 trace");
        assert_eq!(trace.model, "test-model");
        assert_eq!(trace.payload_json, r#"{"messages":[{"role":"user","content":"hi"}]}"#);

        let conn = Connection::open(&logs_root(&root).join("logs-202511.db")).unwrap();
        let (ttft, tps): (i64, f64) = conn
            .query_row("SELECT ttft_ms, tokens_per_sec FROM llm_requests", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!(ttft, 320);
        assert!((tps - 58.5).abs() < f64::EPSILON);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_large_payload_externalized_narrow_row() {
        let root = temp_root("externalize");
        let writer = LogWriter::new(logs_root(&root));

        let mut rec = rec("s1", "2025-11-07T10:00:00Z");
        rec.payload_json = "工具输出内容。".repeat(2000); // ~96KB
        writer.record_llm_request(rec);
        std::thread::sleep(Duration::from_millis(500));
        drop(writer);

        let conn = Connection::open(&logs_root(&root).join("logs-202511.db")).unwrap();
        let (inline_len, bytes, sha): (i64, i64, Option<String>) = conn
            .query_row(
                "SELECT LENGTH(payload_json), payload_bytes, payload_sha256 FROM llm_requests",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert!(sha.is_some(), "大 payload 应外置并记录哈希");
        assert_eq!(bytes, "工具输出内容。".repeat(2000).len() as i64);
        assert!(
            (inline_len as i64) < bytes,
            "行内应只存 preview（{} < {}）",
            inline_len,
            bytes
        );

        // blob 文件存在且回读一致
        let (rel,): (String,) = conn
            .query_row("SELECT rel_path FROM blobs", [], |r| Ok((r.get(0)?,)))
            .unwrap();
        assert!(logs_root(&root).join(LOGS_BLOBS_DIR).join(&rel).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_tool_call_log_writes() {
        let root = temp_root("toollog");
        let writer = LogWriter::new(logs_root(&root));

        writer.record_tool_call(ToolCallLogRecord {
            session_id: "s1".into(),
            message_id: "m1".into(),
            tool_event_id: "te1".into(),
            tool_name: "bash".into(),
            params_json: r#"{"command":"ls"}"#.into(),
            result_text: Some("file1\nfile2".to_string()),
            status: "success".into(),
            duration_ms: 42,
            created_at: "2025-11-07T10:00:00Z".into(),
        });
        std::thread::sleep(Duration::from_millis(500));
        drop(writer);

        let conn = Connection::open(&logs_root(&root).join("logs-202511.db")).unwrap();
        let (status, dur, bytes): (String, i64, i64) = conn
            .query_row(
                "SELECT status, duration_ms, bytes FROM tool_call_logs WHERE tool_name='bash'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(status, "success");
        assert_eq!(dur, 42);
        assert_eq!(bytes, (r#"{"command":"ls"}"#.len() + "file1\nfile2".len()) as i64);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_ttl_cleanup_removes_old_month_and_orphan_blobs() {
        let root = temp_root("ttl");

        // 手工造两个月份库：202501（应删）与当前月（应保留）。
        // 两月外置**不同**内容（内容寻址下同内容共享同一物理文件，无法构造“孤儿”）。
        let old_conn = open_month_db(&logs_root(&root), "202501").unwrap();
        let old_big = "旧月专属审计内容。".repeat(2000); // >8KB → 外置
        blob::store_if_large(&old_conn, &logs_root(&root).join(LOGS_BLOBS_DIR), &old_big).unwrap();
        drop(old_conn);

        let cur_month = month_now();
        let cur_conn = open_month_db(&logs_root(&root), &cur_month).unwrap();
        let cur_big = "当月专属审计内容。".repeat(2000);
        blob::store_if_large(&cur_conn, &logs_root(&root).join(LOGS_BLOBS_DIR), &cur_big).unwrap();
        drop(cur_conn);

        let blobs_dir = logs_root(&root).join(LOGS_BLOBS_DIR);
        let file_count = || -> usize {
            std::fs::read_dir(&blobs_dir)
                .unwrap()
                .flatten()
                .filter(|e| e.path().is_dir())
                .flat_map(|d| std::fs::read_dir(d.path()).unwrap().flatten())
                .flat_map(|d| std::fs::read_dir(d.path()).unwrap().flatten())
                .filter(|f| f.path().is_file())
                .count()
        };
        assert_eq!(file_count(), 2, "两个月份各外置一份不同内容");

        let removed = cleanup_expired(&logs_root(&root), 3).unwrap();
        assert_eq!(removed, 1, "应删除 1 个过期月份");
        assert!(!logs_root(&root).join("logs-202501.db").exists());
        assert!(logs_root(&root).join(format!("logs-{cur_month}.db")).exists());

        // 旧月的孤儿 blob 应被 GC，当前月引用的 blob 应保留
        assert_eq!(file_count(), 1, "仅当前月引用的 blob 应保留");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_disabled_writer_drops_records() {
        let writer = LogWriter::disabled();
        assert!(!writer.is_enabled());
        writer.record_llm_request(rec("s1", "2025-11-07T10:00:00Z")); // 不应 panic
    }

    #[test]
    fn test_threshold_boundary_stays_inline() {
        let root = temp_root("boundary");
        let writer = LogWriter::new(logs_root(&root));

        let mut rec = rec("s1", "2025-11-07T10:00:00Z");
        rec.payload_json = "a".repeat(INLINE_THRESHOLD); // 恰好等于阈值 → 不外置
        writer.record_llm_request(rec);
        std::thread::sleep(Duration::from_millis(500));
        drop(writer);

        let conn = Connection::open(&logs_root(&root).join("logs-202511.db")).unwrap();
        let sha: Option<String> = conn
            .query_row("SELECT payload_sha256 FROM llm_requests", [], |r| r.get(0))
            .unwrap();
        assert!(sha.is_none(), "阈值边界文本应内联");
        let _ = std::fs::remove_dir_all(&root);
    }
}
