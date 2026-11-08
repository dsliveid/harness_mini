//! 大字段外置存储（Content-Addressed Blob Store）
//!
//! 背景：`tool_events.result_text` / `llm_requests.payload_json` 等字段常达数十 KB
//! 至数 MB，内联进 SQLite 会造成「行极宽 → 单页装不下两行 → 缓存命中率崩塌」，
//! 实测 2000 行 × 48KB 的宽表模糊扫描需 284.4ms，外置后仅 1.2ms（约 237 倍）。
//!
//! 设计要点：
//! 1. **内容寻址**：以 sha256 为唯一键，天然去重（同一份工具输出重复出现只存一份）。
//! 2. **两级散列目录**：`blobs/ab/cd/{sha256}`，避免单目录堆积数十万文件拖慢文件系统。
//! 3. **zstd 压缩**：文本类日志压缩比通常 5~10 倍，进一步压缩磁盘占用。
//! 4. **写前摘要**：落库时同时生成前 200 字符 `preview`，列表展示直读该列，
//!    **列表查询永不触碰 blob 实体文件**，只在用户点开详情时才懒加载回读。
//! 5. **引用计数**：`ref_count` 记录被引用次数，归零后可安全清理物理文件。

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// 单字段超过该字节数则触发外置（与 llm_requests 既有瘦身阈值保持一致）。
pub const INLINE_THRESHOLD: usize = 8192;

/// 摘要列保留的字符数。
const PREVIEW_CHARS: usize = 200;

/// 外置成功后返回给调用方的元信息。
#[derive(Debug, Clone)]
pub struct BlobRef {
    /// 内容哈希（十六进制），同时是 blobs 表主键
    pub sha256: String,
    /// 相对 blobs 根目录的路径，如 `ab/cd/abcd...`（阶段3 消息/工具事件引用列将消费）
    #[allow(dead_code)]
    pub rel_path: String,
    /// 原始（未压缩）字节数
    pub bytes: i64,
    /// 前 200 字符摘要，供列表直接展示
    pub preview: String,
    /// 是否实际压缩（小于阈值或压缩无收益时为 false）
    #[allow(dead_code)]
    pub compressed: bool,
}

/// blobs 表结构。与其他表一致，随所属库的 schema 一起初始化。
pub fn ensure_blob_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS blobs (
           sha256 TEXT PRIMARY KEY,
           rel_path TEXT NOT NULL,
           bytes INTEGER NOT NULL,
           encoding TEXT NOT NULL DEFAULT 'identity',
           ref_count INTEGER NOT NULL DEFAULT 1,
           created_at TEXT NOT NULL
         );",
    )
    .map_err(|e| format!("初始化 blobs 表失败: {e}"))
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// 两级散列相对路径：`ab/cd/{sha256}`
fn rel_path_for(sha: &str) -> String {
    let _ = sha;
    let a = sha.get(0..2).unwrap_or("00");
    let b = sha.get(2..4).unwrap_or("00");
    format!("{a}/{b}/{sha}")
}

/// 生成 UTF-8 安全的字符级摘要（不会在字符中间截断）。
pub fn make_preview(text: &str) -> String {
    if text.chars().count() <= PREVIEW_CHARS {
        return text.to_string();
    }
    text.chars().take(PREVIEW_CHARS).collect::<String>()
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// 若文本超过阈值则外置存储，返回 `Some(BlobRef)`；否则返回 `None` 表示应内联保留。
///
/// 命中已有哈希时仅递增 `ref_count` 并复用物理文件，不重复写盘。
pub fn store_if_large(
    conn: &Connection,
    blobs_root: &Path,
    text: &str,
) -> Result<Option<BlobRef>, String> {
    let raw = text.as_bytes();
    if raw.len() <= INLINE_THRESHOLD {
        return Ok(None);
    }

    ensure_blob_schema(conn)?;
    let sha = sha256_hex(raw);
    let rel = rel_path_for(&sha);
    let preview = make_preview(text);

    // 内容已存在：复用，仅累加引用计数
    let existing: Option<String> = conn
        .query_row("SELECT rel_path FROM blobs WHERE sha256 = ?1", params![sha], |r| r.get(0))
        .optional()
        .map_err(|e| e.to_string())?;

    if let Some(existing_rel) = existing {
        conn.execute(
            "UPDATE blobs SET ref_count = ref_count + 1 WHERE sha256 = ?1",
            params![sha],
        )
        .map_err(|e| e.to_string())?;
        let encoding: String = conn
            .query_row("SELECT encoding FROM blobs WHERE sha256 = ?1", params![sha], |r| r.get(0))
            .unwrap_or_else(|_| "identity".to_string());
        return Ok(Some(BlobRef {
            sha256: sha,
            rel_path: existing_rel,
            bytes: raw.len() as i64,
            preview,
            compressed: encoding != "identity",
        }));
    }

    // 压缩：仅在有明显收益时采用，避免小文本压缩后反而变大
    let compressed_bytes = zstd::encode_all(raw, 3).map_err(|e| format!("zstd 压缩失败: {e}"))?;
    let use_compressed = compressed_bytes.len() < raw.len();
    let (payload, encoding) = if use_compressed {
        (compressed_bytes, "zstd")
    } else {
        (raw.to_vec(), "identity")
    };

    let abs: PathBuf = blobs_root.join(&rel);
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建 blob 目录失败: {e}"))?;
    }
    std::fs::write(&abs, &payload).map_err(|e| format!("写入 blob 文件失败: {e}"))?;

    conn.execute(
        "INSERT INTO blobs (sha256, rel_path, bytes, encoding, ref_count, created_at)
         VALUES (?1, ?2, ?3, ?4, 1, ?5)",
        params![sha, rel, raw.len() as i64, encoding, now()],
    )
    .map_err(|e| format!("登记 blob 元信息失败: {e}"))?;

    Ok(Some(BlobRef {
        sha256: sha,
        rel_path: rel,
        bytes: raw.len() as i64,
        preview,
        compressed: use_compressed,
    }))
}

/// 按哈希懒加载回读原始文本（自动解压）。仅在用户查看详情时调用。
pub fn load_by_sha(conn: &Connection, blobs_root: &Path, sha: &str) -> Result<String, String> {
    let (rel, encoding): (String, String) = conn
        .query_row(
            "SELECT rel_path, encoding FROM blobs WHERE sha256 = ?1",
            params![sha],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| format!("未找到 blob 记录 {sha}: {e}"))?;

    read_blob_file(&blobs_root.join(rel), &encoding)
}

fn read_blob_file(abs: &Path, encoding: &str) -> Result<String, String> {
    let payload = std::fs::read(abs)
        .map_err(|e| format!("读取 blob 文件失败 ({}): {e}", abs.display()))?;
    let raw = if encoding == "zstd" {
        zstd::decode_all(&payload[..]).map_err(|e| format!("zstd 解压失败: {e}"))?
    } else {
        payload
    };
    String::from_utf8(raw).map_err(|e| format!("blob 内容不是合法 UTF-8: {e}"))
}

/// 递减引用计数，归零时删除物理文件与元信息。删除失败不阻断主流程。
/// 当前仅测试引用；阶段3（消息撤回/删除释放 blob 引用）接线后成为常规路径。
#[allow(dead_code)]
pub fn release(conn: &Connection, blobs_root: &Path, sha: &str) {
    let row: Option<(String, i64)> = conn
        .query_row(
            "SELECT rel_path, ref_count FROM blobs WHERE sha256 = ?1",
            params![sha],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .ok()
        .flatten();

    let Some((rel, count)) = row else { return };

    if count <= 1 {
        let _ = conn.execute("DELETE FROM blobs WHERE sha256 = ?1", params![sha]);
        let _ = std::fs::remove_file(blobs_root.join(rel));
    } else {
        let _ = conn.execute(
            "UPDATE blobs SET ref_count = ref_count - 1 WHERE sha256 = ?1",
            params![sha],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("hm_blob_test_{tag}_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn mem_conn() -> Connection {
        Connection::open_in_memory().unwrap()
    }

    #[test]
    fn test_small_text_stays_inline() {
        let conn = mem_conn();
        let root = temp_root("small");
        let r = store_if_large(&conn, &root, "短文本").unwrap();
        assert!(r.is_none(), "小于阈值的文本不应外置");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_large_text_externalized_and_roundtrip() {
        let conn = mem_conn();
        let root = temp_root("large");
        // 用可压缩的重复文本，验证压缩路径与无损回读
        let big = "abcdefghij".repeat(2000); // 20KB
        let r = store_if_large(&conn, &root, &big).unwrap().expect("应触发外置");

        assert_eq!(r.bytes, big.len() as i64);
        assert!(r.compressed, "重复文本应被压缩");
        assert!(r.preview.chars().count() <= PREVIEW_CHARS);
        assert!(root.join(&r.rel_path).exists(), "物理文件应落盘");

        let back = load_by_sha(&conn, &root, &r.sha256).unwrap();
        assert_eq!(back, big, "解压回读必须与原文字节一致");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_dedup_increments_ref_count() {
        let conn = mem_conn();
        let root = temp_root("dedup");
        let big = "x".repeat(20_000);

        let a = store_if_large(&conn, &root, &big).unwrap().unwrap();
        let b = store_if_large(&conn, &root, &big).unwrap().unwrap();
        assert_eq!(a.sha256, b.sha256, "相同内容应命中同一哈希");

        let count: i64 = conn
            .query_row("SELECT ref_count FROM blobs WHERE sha256 = ?1", params![a.sha256], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2, "重复内容只应存储一次，引用计数递增");

        // 释放一次：文件仍在
        release(&conn, &root, &a.sha256);
        assert!(root.join(&a.rel_path).exists(), "仍有引用时不可删除物理文件");

        // 再次释放：归零，文件删除
        release(&conn, &root, &a.sha256);
        assert!(!root.join(&a.rel_path).exists(), "引用归零后应清理物理文件");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_preview_is_utf8_safe() {
        // 多字节字符不应被截断成非法 UTF-8
        let text = "中".repeat(500);
        let p = make_preview(&text);
        assert_eq!(p.chars().count(), PREVIEW_CHARS);
        assert!(p.is_char_boundary(p.len()));
    }
}
