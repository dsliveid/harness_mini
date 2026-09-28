//! 影子快照还原与重做引擎（含人工修改冲突防护与多轮时光机）

use crate::diffutil;
use crate::models::{ReapplyResult, RevertResult, SnapshotFileDiff, ToolFileSnapshot};
use crate::snapshot_fs::{compute_sha256, read_blob_async};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum RevertError {
    Conflict {
        file: String,
        current_hash: String,
        expected_hash: String,
    },
    FileNotFound(String),
    Io(String),
    Database(String),
}

impl std::fmt::Display for RevertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RevertError::Conflict {
                file,
                current_hash,
                expected_hash,
            } => write!(
                f,
                "文件 [{file}] 在生成后已被外部手动修改（当前哈希 {current_hash} != 预期哈希 {expected_hash}）"
            ),
            RevertError::FileNotFound(file) => write!(f, "文件未找到: {file}"),
            RevertError::Io(e) => write!(f, "文件 IO 错误: {e}"),
            RevertError::Database(e) => write!(f, "数据库错误: {e}"),
        }
    }
}

/// 规范化解析相对路径至工作区物理路径
fn resolve_in_workspace(workspace: &Path, rel: &str) -> PathBuf {
    let clean = rel.replace('\\', "/");
    let trimmed = clean.trim_start_matches('/');
    workspace.join(trimmed)
}

/// 检查单文件在撤回前是否存在人为冲突
async fn check_revert_conflict(
    workspace: &Path,
    snap: &ToolFileSnapshot,
) -> Result<Option<String>, RevertError> {
    let target = resolve_in_workspace(workspace, &snap.file_path);
    if !target.exists() {
        if snap.is_new_file {
            return Ok(None);
        } else {
            return Ok(Some(snap.file_path.clone()));
        }
    }

    let bytes = tokio::fs::read(&target)
        .await
        .map_err(|e| RevertError::Io(e.to_string()))?;
    let cur_hash = compute_sha256(&bytes);

    if cur_hash != snap.after_hash {
        return Ok(Some(snap.file_path.clone()));
    }

    Ok(None)
}

/// 检查单文件在重做前是否存在人为冲突
async fn check_reapply_conflict(
    workspace: &Path,
    snap: &ToolFileSnapshot,
) -> Result<Option<String>, RevertError> {
    let target = resolve_in_workspace(workspace, &snap.file_path);
    if !target.exists() {
        if snap.is_new_file {
            return Ok(None);
        } else {
            return Ok(Some(snap.file_path.clone()));
        }
    }

    let bytes = tokio::fs::read(&target)
        .await
        .map_err(|e| RevertError::Io(e.to_string()))?;
    let cur_hash = compute_sha256(&bytes);

    // 重做时物理文件应还原为 before_hash（或为新建前状态）
    if let Some(ref bh) = snap.before_hash {
        if cur_hash != *bh {
            return Ok(Some(snap.file_path.clone()));
        }
    } else {
        // 新建文件在撤回后磁盘上应不存在；若已存在且非空则存在冲突
        if !bytes.is_empty() {
            return Ok(Some(snap.file_path.clone()));
        }
    }

    Ok(None)
}

/// 执行单文件的物理撤回
async fn apply_single_revert(
    workspace: &Path,
    data_dir: &Path,
    snap: &ToolFileSnapshot,
) -> Result<(), RevertError> {
    let target = resolve_in_workspace(workspace, &snap.file_path);

    if snap.is_new_file {
        if target.exists() {
            tokio::fs::remove_file(&target)
                .await
                .map_err(|e| RevertError::Io(format!("删除新建文件失败: {e}")))?;
        }
    } else {
        let before_hash = snap
            .before_hash
            .as_ref()
            .ok_or_else(|| RevertError::Io("缺少 before_hash".into()))?;
        let before_bytes = read_blob_async(data_dir, before_hash)
            .await
            .map_err(|e| RevertError::Io(format!("读取 CAS 原始快照失败: {e}")))?;

        if let Some(parent) = target.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }

        tokio::fs::write(&target, &before_bytes)
            .await
            .map_err(|e| RevertError::Io(format!("写回原始快照失败: {e}")))?;
    }

    Ok(())
}

/// 执行单文件的物理重做（Reapply）
async fn apply_single_reapply(
    workspace: &Path,
    data_dir: &Path,
    snap: &ToolFileSnapshot,
) -> Result<(), RevertError> {
    let target = resolve_in_workspace(workspace, &snap.file_path);
    let after_bytes = read_blob_async(data_dir, &snap.after_hash)
        .await
        .map_err(|e| RevertError::Io(format!("读取 CAS 新版快照失败: {e}")))?;

    if let Some(parent) = target.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    tokio::fs::write(&target, &after_bytes)
        .await
        .map_err(|e| RevertError::Io(format!("写回新版快照失败: {e}")))?;

    Ok(())
}

/// 批量撤回快照切片中的文件修改（按 LIFO 逆序还原）
pub async fn revert_snapshots(
    workspace: &Path,
    data_dir: &Path,
    snapshots: &[ToolFileSnapshot],
    force: bool,
) -> Result<RevertResult, String> {
    if snapshots.is_empty() {
        return Ok(RevertResult {
            success: true,
            reverted_files: vec![],
            has_conflict: false,
            conflicted_files: vec![],
            message: "无文件修改快照记录".into(),
        });
    }

    // 1. 冲突检测阶段：若未指定 force，对每个不同文件仅检测其最新快照（当前磁盘应当对应的版本）
    if !force {
        let mut conflicted = Vec::new();
        let mut checked_files = std::collections::HashSet::new();
        for s in snapshots.iter().rev() {
            if checked_files.insert(&s.file_path) {
                if let Ok(Some(cf)) = check_revert_conflict(workspace, s).await {
                    if !conflicted.contains(&cf) {
                        conflicted.push(cf);
                    }
                }
            }
        }
        if !conflicted.is_empty() {
            return Ok(RevertResult {
                success: false,
                reverted_files: vec![],
                has_conflict: true,
                conflicted_files: conflicted,
                message: "检测到部分文件在生成后曾被手动修改，已安全中止撤回。可选择强制覆盖。".into(),
            });
        }
    }

    // 2. 物理还原阶段：按 LIFO 倒序执行文件还原（最后改动的文件先还原）
    let mut reverted_files = Vec::new();
    for s in snapshots.iter().rev() {
        apply_single_revert(workspace, data_dir, s)
            .await
            .map_err(|e| e.to_string())?;
        if !reverted_files.contains(&s.file_path) {
            reverted_files.push(s.file_path.clone());
        }
    }

    Ok(RevertResult {
        success: true,
        reverted_files,
        has_conflict: false,
        conflicted_files: vec![],
        message: "已成功撤回修改".into(),
    })
}

/// 批量重做快照切片中的文件修改（按 FIFO 顺序写回新版代码）
pub async fn reapply_snapshots(
    workspace: &Path,
    data_dir: &Path,
    snapshots: &[ToolFileSnapshot],
    force: bool,
) -> Result<ReapplyResult, String> {
    if snapshots.is_empty() {
        return Ok(ReapplyResult {
            success: true,
            reapplied_files: vec![],
            has_conflict: false,
            conflicted_files: vec![],
            message: "无文件修改快照记录".into(),
        });
    }

    // 1. 冲突检测：对每个不同文件仅检测其最老快照（重做起始状态）是否与磁盘匹配
    if !force {
        let mut conflicted = Vec::new();
        let mut checked_files = std::collections::HashSet::new();
        for s in snapshots {
            if checked_files.insert(&s.file_path) {
                if let Ok(Some(cf)) = check_reapply_conflict(workspace, s).await {
                    if !conflicted.contains(&cf) {
                        conflicted.push(cf);
                    }
                }
            }
        }
        if !conflicted.is_empty() {
            return Ok(ReapplyResult {
                success: false,
                reapplied_files: vec![],
                has_conflict: true,
                conflicted_files: conflicted,
                message: "检测到外部修改冲突，已安全中止重做。可选择强制覆盖。".into(),
            });
        }
    }

    // 2. 物理重做阶段：按顺序写入新版代码
    let mut reapplied_files = Vec::new();
    for s in snapshots {
        apply_single_reapply(workspace, data_dir, s)
            .await
            .map_err(|e| e.to_string())?;
        if !reapplied_files.contains(&s.file_path) {
            reapplied_files.push(s.file_path.clone());
        }
    }

    Ok(ReapplyResult {
        success: true,
        reapplied_files,
        has_conflict: false,
        conflicted_files: vec![],
        message: "已成功重新应用修改".into(),
    })
}

/// 解析快照列表并生成对比 diff 详情（同一轮对话中同一文件的多次修改自动聚合为一条全量 Diff）
pub async fn get_snapshots_diff_details(
    data_dir: &Path,
    snapshots: &[ToolFileSnapshot],
) -> Result<Vec<SnapshotFileDiff>, String> {
    let mut file_order: Vec<String> = Vec::new();
    let mut file_map: std::collections::HashMap<String, Vec<&ToolFileSnapshot>> = std::collections::HashMap::new();

    for s in snapshots {
        if !file_map.contains_key(&s.file_path) {
            file_order.push(s.file_path.clone());
        }
        file_map.entry(s.file_path.clone()).or_default().push(s);
    }

    let mut diffs = Vec::new();
    for file_path in file_order {
        let snaps = match file_map.get(&file_path) {
            Some(list) if !list.is_empty() => list,
            _ => continue,
        };

        let first = snaps[0];
        let last = snaps[snaps.len() - 1];

        let before_text = if first.is_new_file {
            None
        } else if let Some(ref bh) = first.before_hash {
            let bytes = read_blob_async(data_dir, bh).await.unwrap_or_default();
            Some(String::from_utf8_lossy(&bytes).to_string())
        } else {
            None
        };

        let after_bytes = read_blob_async(data_dir, &last.after_hash).await.unwrap_or_default();
        let after_text = String::from_utf8_lossy(&after_bytes).to_string();

        let old_str = before_text.as_deref().unwrap_or("");
        let d = diffutil::diff_lines(old_str, &after_text, 64 * 1024);

        let is_all_reverted = snaps.iter().all(|s| s.reverted_at.is_some());
        let reverted_at = if is_all_reverted {
            last.reverted_at.clone()
        } else {
            None
        };

        diffs.push(SnapshotFileDiff {
            file_path: file_path.clone(),
            is_new_file: first.is_new_file,
            added: d.added,
            removed: d.removed,
            diff_text: d.text,
            before_content: before_text,
            after_content: after_text,
            tool_event_id: Some(last.tool_event_id.clone()),
            snapshot_id: Some(last.id.clone()),
            reverted_at,
            modify_count: snaps.len(),
            tool_event_ids: snaps.iter().map(|s| s.tool_event_id.clone()).collect(),
        });
    }
    Ok(diffs)
}

/// 撤回整轮消息产生的所有文件改动（按 LIFO 逆序执行）
pub async fn revert_message_turn(
    workspace: &Path,
    data_dir: &Path,
    conn: &Connection,
    message_id: &str,
    force: bool,
) -> Result<RevertResult, String> {
    let snapshots = crate::store::list_snapshots_for_turn(conn, message_id)?;
    let res = revert_snapshots(workspace, data_dir, &snapshots, force).await?;
    if res.success && !snapshots.is_empty() {
        let now = crate::store::now();
        crate::store::mark_snapshots_reverted_for_turn(conn, message_id, Some(&now))?;
    }
    Ok(res)
}

/// 重新应用（重做）整轮修改（按 FIFO 顺序执行）
pub async fn reapply_message_turn(
    workspace: &Path,
    data_dir: &Path,
    conn: &Connection,
    message_id: &str,
    force: bool,
) -> Result<ReapplyResult, String> {
    let snapshots = crate::store::list_snapshots_for_turn(conn, message_id)?;
    let res = reapply_snapshots(workspace, data_dir, &snapshots, force).await?;
    if res.success && !snapshots.is_empty() {
        crate::store::mark_snapshots_reverted_for_turn(conn, message_id, None)?;
    }
    Ok(res)
}

/// 撤回单个工具事件对应的文件修改
pub async fn revert_tool_event(
    workspace: &Path,
    data_dir: &Path,
    conn: &Connection,
    tool_event_id: &str,
    force: bool,
) -> Result<RevertResult, String> {
    let snapshots = crate::store::list_snapshots_for_event(conn, tool_event_id)?;
    let res = revert_snapshots(workspace, data_dir, &snapshots, force).await?;
    if res.success && !snapshots.is_empty() {
        let now = crate::store::now();
        crate::store::mark_snapshot_reverted_for_event(conn, tool_event_id, Some(&now))?;
    }
    Ok(res)
}

/// 获取本轮消息产生的所有文件变更差异详情（供审查 Diff 弹窗使用）
pub async fn get_turn_diff_details(
    data_dir: &Path,
    conn: &Connection,
    message_id: &str,
) -> Result<Vec<SnapshotFileDiff>, String> {
    let snapshots = crate::store::list_snapshots_for_turn(conn, message_id)?;
    get_snapshots_diff_details(data_dir, &snapshots).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;
    use crate::snapshot_fs::save_blob;
    use crate::store::*;
    use rusqlite::Connection;

    fn temp_test_env(tag: &str) -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("hm_revert_test_{tag}_{}", uuid::Uuid::new_v4()));
        let ws = base.join("ws");
        let data = base.join("data");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        (ws, data)
    }

    #[tokio::test]
    async fn test_revert_and_reapply_modified_file() {
        let (ws, data) = temp_test_env("mod");
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();

        let target_file = ws.join("hello.txt");
        let old_content = b"original content line 1\nline 2";
        std::fs::write(&target_file, old_content).unwrap();

        let new_content = b"modified content line 1\nline 2 appended";
        std::fs::write(&target_file, new_content).unwrap();

        let before_hash = save_blob(&data, old_content).unwrap();
        let after_hash = save_blob(&data, new_content).unwrap();

        let s = create_session(&conn, &ws.to_string_lossy(), None, "test", "confirm").unwrap();
        let m = new_message(&conn, &s.id, "assistant", Some("test".into()), false).unwrap();
        let ev = ToolEvent {
            id: "ev-1".into(),
            message_id: m.id.clone(),
            tool_name: "edit_file".into(),
            tool_call_id: None,
            params: serde_json::json!({}),
            result_text: None,
            status: "success".into(),
            approval_scope: None,
            created_at: now(),
            subprocess_id: None,
            reverted_at: None,
        };
        insert_tool_event(&conn, &ev).unwrap();

        let snap = ToolFileSnapshot {
            id: "snap-mod".into(),
            session_id: s.id.clone(),
            message_id: m.id.clone(),
            tool_event_id: ev.id.clone(),
            file_path: "hello.txt".into(),
            before_hash: Some(before_hash),
            after_hash,
            is_new_file: false,
            reverted_at: None,
            created_at: now(),
        };
        insert_tool_file_snapshot(&conn, &snap).unwrap();

        // 1. 撤回测试
        let rev_res = revert_message_turn(&ws, &data, &conn, &m.id, false).await.unwrap();
        assert!(rev_res.success);
        assert_eq!(rev_res.reverted_files, vec!["hello.txt"]);
        let disk_bytes = std::fs::read(&target_file).unwrap();
        assert_eq!(disk_bytes, old_content);

        // 验证消息被打标 reverted_at
        let loaded_msg = get_message(&conn, &m.id).unwrap().unwrap();
        assert!(loaded_msg.reverted_at.is_some());

        // 2. 重做测试
        let reap_res = reapply_message_turn(&ws, &data, &conn, &m.id, false).await.unwrap();
        assert!(reap_res.success);
        assert_eq!(reap_res.reapplied_files, vec!["hello.txt"]);
        let redone_bytes = std::fs::read(&target_file).unwrap();
        assert_eq!(redone_bytes, new_content);

        let reloaded_msg = get_message(&conn, &m.id).unwrap().unwrap();
        assert!(reloaded_msg.reverted_at.is_none());

        let _ = std::fs::remove_dir_all(ws.parent().unwrap());
    }

    #[tokio::test]
    async fn test_revert_new_created_file_deletes_it() {
        let (ws, data) = temp_test_env("new");
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();

        let target_file = ws.join("brand_new.rs");
        let new_content = b"fn main() {}";
        std::fs::write(&target_file, new_content).unwrap();

        let after_hash = save_blob(&data, new_content).unwrap();

        let s = create_session(&conn, &ws.to_string_lossy(), None, "test", "confirm").unwrap();
        let m = new_message(&conn, &s.id, "assistant", Some("test".into()), false).unwrap();
        let ev = ToolEvent {
            id: "ev-2".into(),
            message_id: m.id.clone(),
            tool_name: "write_file".into(),
            tool_call_id: None,
            params: serde_json::json!({}),
            result_text: None,
            status: "success".into(),
            approval_scope: None,
            created_at: now(),
            subprocess_id: None,
            reverted_at: None,
        };
        insert_tool_event(&conn, &ev).unwrap();

        let snap = ToolFileSnapshot {
            id: "snap-new".into(),
            session_id: s.id.clone(),
            message_id: m.id.clone(),
            tool_event_id: ev.id.clone(),
            file_path: "brand_new.rs".into(),
            before_hash: None,
            after_hash,
            is_new_file: true,
            reverted_at: None,
            created_at: now(),
        };
        insert_tool_file_snapshot(&conn, &snap).unwrap();

        // 撤回：新建文件物理删除
        let rev_res = revert_message_turn(&ws, &data, &conn, &m.id, false).await.unwrap();
        assert!(rev_res.success);
        assert!(!target_file.exists());

        // 重做：新建文件重新创建
        let reap_res = reapply_message_turn(&ws, &data, &conn, &m.id, false).await.unwrap();
        assert!(reap_res.success);
        assert!(target_file.exists());
        assert_eq!(std::fs::read(&target_file).unwrap(), new_content);

        let _ = std::fs::remove_dir_all(ws.parent().unwrap());
    }

    #[tokio::test]
    async fn test_conflict_guard_prevents_accidental_overwrite() {
        let (ws, data) = temp_test_env("conflict");
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();

        let target_file = ws.join("auth.rs");
        let ai_content = b"pub fn login() {}";
        std::fs::write(&target_file, ai_content).unwrap();
        let after_hash = save_blob(&data, ai_content).unwrap();

        let s = create_session(&conn, &ws.to_string_lossy(), None, "test", "confirm").unwrap();
        let m = new_message(&conn, &s.id, "assistant", Some("test".into()), false).unwrap();
        let ev = ToolEvent {
            id: "ev-3".into(),
            message_id: m.id.clone(),
            tool_name: "write_file".into(),
            tool_call_id: None,
            params: serde_json::json!({}),
            result_text: None,
            status: "success".into(),
            approval_scope: None,
            created_at: now(),
            subprocess_id: None,
            reverted_at: None,
        };
        insert_tool_event(&conn, &ev).unwrap();

        let snap = ToolFileSnapshot {
            id: "snap-cf".into(),
            session_id: s.id.clone(),
            message_id: m.id.clone(),
            tool_event_id: ev.id.clone(),
            file_path: "auth.rs".into(),
            before_hash: None,
            after_hash,
            is_new_file: true,
            reverted_at: None,
            created_at: now(),
        };
        insert_tool_file_snapshot(&conn, &snap).unwrap();

        // 外部人为手工修改了该文件！
        std::fs::write(&target_file, b"pub fn login() { /* manual edit */ }").unwrap();

        // 在未指定 force 的情况下尝试撤回 -> 必须检测到冲突并拦截
        let rev_res = revert_message_turn(&ws, &data, &conn, &m.id, false).await.unwrap();
        assert!(!rev_res.success);
        assert!(rev_res.has_conflict);
        assert_eq!(rev_res.conflicted_files, vec!["auth.rs"]);
        assert!(target_file.exists()); // 保护生效，文件未被误删！

        // 传入 force=true -> 强制撤回
        let force_res = revert_message_turn(&ws, &data, &conn, &m.id, true).await.unwrap();
        assert!(force_res.success);
        assert!(!target_file.exists());

        let _ = std::fs::remove_dir_all(ws.parent().unwrap());
    }

    #[tokio::test]
    async fn test_multi_turn_revert_and_reapply() {
        let (ws, data) = temp_test_env("multi_turn");
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();

        let s = create_session(&conn, &ws.to_string_lossy(), None, "test", "confirm").unwrap();

        // 轮次 1：用户提问 u1 -> 助手回复 a1 新建 a.txt (version 1)
        let u1 = new_message(&conn, &s.id, "user", Some("create a.txt".into()), false).unwrap();
        let a1 = new_message(&conn, &s.id, "assistant", Some("created a.txt".into()), false).unwrap();
        let file_a = ws.join("a.txt");
        let a_v1 = b"a content v1";
        std::fs::write(&file_a, a_v1).unwrap();
        let ev1 = ToolEvent {
            id: "ev-1".into(),
            message_id: a1.id.clone(),
            tool_name: "write_file".into(),
            tool_call_id: None,
            params: serde_json::json!({}),
            result_text: None,
            status: "success".into(),
            approval_scope: None,
            created_at: now(),
            subprocess_id: None,
            reverted_at: None,
        };
        insert_tool_event(&conn, &ev1).unwrap();

        let snap1 = ToolFileSnapshot {
            id: "snap-1".into(),
            session_id: s.id.clone(),
            message_id: a1.id.clone(),
            tool_event_id: "ev-1".into(),
            file_path: "a.txt".into(),
            before_hash: None,
            after_hash: save_blob(&data, a_v1).unwrap(),
            is_new_file: true,
            reverted_at: None,
            created_at: "2026-09-28T10:00:00Z".into(),
        };
        insert_tool_file_snapshot(&conn, &snap1).unwrap();

        // 轮次 2：用户提问 u2 -> 助手回复 a2 修改 a.txt (version 2) 并新建 b.txt
        let u2 = new_message(&conn, &s.id, "user", Some("update a and create b".into()), false).unwrap();
        let a2 = new_message(&conn, &s.id, "assistant", Some("done".into()), false).unwrap();
        let a_v2 = b"a content v2 modified";
        std::fs::write(&file_a, a_v2).unwrap();

        let ev2a = ToolEvent {
            id: "ev-2a".into(),
            message_id: a2.id.clone(),
            tool_name: "edit_file".into(),
            tool_call_id: None,
            params: serde_json::json!({}),
            result_text: None,
            status: "success".into(),
            approval_scope: None,
            created_at: now(),
            subprocess_id: None,
            reverted_at: None,
        };
        insert_tool_event(&conn, &ev2a).unwrap();

        let snap2_a = ToolFileSnapshot {
            id: "snap-2a".into(),
            session_id: s.id.clone(),
            message_id: a2.id.clone(),
            tool_event_id: "ev-2a".into(),
            file_path: "a.txt".into(),
            before_hash: Some(save_blob(&data, a_v1).unwrap()),
            after_hash: save_blob(&data, a_v2).unwrap(),
            is_new_file: false,
            reverted_at: None,
            created_at: "2026-09-28T10:01:00Z".into(),
        };
        insert_tool_file_snapshot(&conn, &snap2_a).unwrap();

        let file_b = ws.join("b.txt");
        let b_v1 = b"b content";
        std::fs::write(&file_b, b_v1).unwrap();

        let ev2b = ToolEvent {
            id: "ev-2b".into(),
            message_id: a2.id.clone(),
            tool_name: "write_file".into(),
            tool_call_id: None,
            params: serde_json::json!({}),
            result_text: None,
            status: "success".into(),
            approval_scope: None,
            created_at: now(),
            subprocess_id: None,
            reverted_at: None,
        };
        insert_tool_event(&conn, &ev2b).unwrap();

        let snap2_b = ToolFileSnapshot {
            id: "snap-2b".into(),
            session_id: s.id.clone(),
            message_id: a2.id.clone(),
            tool_event_id: "ev-2b".into(),
            file_path: "b.txt".into(),
            before_hash: None,
            after_hash: save_blob(&data, b_v1).unwrap(),
            is_new_file: true,
            reverted_at: None,
            created_at: "2026-09-28T10:01:05Z".into(),
        };
        insert_tool_file_snapshot(&conn, &snap2_b).unwrap();

        // 1. 测试撤回对话到 u2
        let snaps_to_revert = list_snapshots_after_seq(&conn, &s.id, u2.seq).unwrap();
        assert_eq!(snaps_to_revert.len(), 2);
        let rev_res = revert_snapshots(&ws, &data, &snaps_to_revert, false).await.unwrap();
        assert!(rev_res.success);

        let now_str = "2026-09-28T10:02:00Z";
        let affected = mark_messages_and_snapshots_reverted_after_seq(&conn, &s.id, u2.seq, Some(now_str)).unwrap();
        assert_eq!(affected.len(), 2); // u2 and a2

        // 验证文件回退状态：a.txt 还原为 v1，b.txt 被安全删除
        assert_eq!(std::fs::read(&file_a).unwrap(), a_v1);
        assert!(!file_b.exists());

        // 验证数据库状态
        let loaded_u1 = get_message(&conn, &u1.id).unwrap().unwrap();
        let loaded_u2 = get_message(&conn, &u2.id).unwrap().unwrap();
        let loaded_a2 = get_message(&conn, &a2.id).unwrap().unwrap();
        assert!(loaded_u1.reverted_at.is_none());
        assert_eq!(loaded_u2.reverted_at.as_deref(), Some(now_str));
        assert_eq!(loaded_a2.reverted_at.as_deref(), Some(now_str));

        // 2. 测试重新应用（后悔药）
        let snaps_to_reapply = list_reverted_snapshots_after_seq(&conn, &s.id, u2.seq).unwrap();
        assert_eq!(snaps_to_reapply.len(), 2);
        let reap_res = reapply_snapshots(&ws, &data, &snaps_to_reapply, false).await.unwrap();
        assert!(reap_res.success);

        mark_messages_and_snapshots_reverted_after_seq(&conn, &s.id, u2.seq, None).unwrap();

        // 验证文件与消息重做恢复
        assert_eq!(std::fs::read(&file_a).unwrap(), a_v2);
        assert_eq!(std::fs::read(&file_b).unwrap(), b_v1);

        let reloaded_u2 = get_message(&conn, &u2.id).unwrap().unwrap();
        let reloaded_a2 = get_message(&conn, &a2.id).unwrap().unwrap();
        assert!(reloaded_u2.reverted_at.is_none());
        assert!(reloaded_a2.reverted_at.is_none());

        // 3. 测试再次撤回后发送新消息（覆盖截断旧分支）
        let snaps_to_revert2 = list_snapshots_after_seq(&conn, &s.id, u2.seq).unwrap();
        revert_snapshots(&ws, &data, &snaps_to_revert2, false).await.unwrap();
        mark_messages_and_snapshots_reverted_after_seq(&conn, &s.id, u2.seq, Some(now_str)).unwrap();

        // 用户编辑 u2 内容并发送（清除原后续消息）
        update_message_content_and_attachments(&conn, &u2.id, "u2 rewritten", None).unwrap();
        delete_messages_after(&conn, &s.id, u2.seq).unwrap();

        let final_u2 = get_message(&conn, &u2.id).unwrap().unwrap();
        let final_a2 = get_message(&conn, &a2.id).unwrap();
        assert_eq!(final_u2.content.as_deref(), Some("u2 rewritten"));
        assert!(final_u2.reverted_at.is_none()); // 必须被自动重置为非撤回状态
        assert!(final_a2.is_none()); // a2 已被物理删除

        let _ = std::fs::remove_dir_all(ws.parent().unwrap());
    }

    #[tokio::test]
    async fn test_turn_diff_aggregation_and_single_file_revert() {
        let (ws, data) = temp_test_env("agg");
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();

        let target_file = ws.join("sample.txt");
        let v0_content = b"line 1\nline 2";
        std::fs::write(&target_file, v0_content).unwrap();

        let v1_content = b"line 1\nline 2 modified\nline 3";
        let v2_content = b"line 1\nline 2 modified\nline 3\nline 4 added";

        let h0 = save_blob(&data, v0_content).unwrap();
        let h1 = save_blob(&data, v1_content).unwrap();
        let h2 = save_blob(&data, v2_content).unwrap();

        let s = create_session(&conn, &ws.to_string_lossy(), None, "test", "confirm").unwrap();
        let m = new_message(&conn, &s.id, "assistant", Some("test turn".into()), false).unwrap();

        let ev1 = ToolEvent {
            id: "ev-step-1".into(),
            message_id: m.id.clone(),
            tool_name: "edit_file".into(),
            tool_call_id: None,
            params: serde_json::json!({"path": "sample.txt"}),
            result_text: None,
            status: "success".into(),
            approval_scope: None,
            created_at: now(),
            subprocess_id: None,
            reverted_at: None,
        };
        insert_tool_event(&conn, &ev1).unwrap();

        let snap1 = ToolFileSnapshot {
            id: "snap-1".into(),
            session_id: s.id.clone(),
            message_id: m.id.clone(),
            tool_event_id: ev1.id.clone(),
            file_path: "sample.txt".into(),
            before_hash: Some(h0.clone()),
            after_hash: h1.clone(),
            is_new_file: false,
            reverted_at: None,
            created_at: "2026-09-28T10:00:00Z".into(),
        };
        insert_tool_file_snapshot(&conn, &snap1).unwrap();

        let ev2 = ToolEvent {
            id: "ev-step-2".into(),
            message_id: m.id.clone(),
            tool_name: "edit_file".into(),
            tool_call_id: None,
            params: serde_json::json!({"path": "sample.txt"}),
            result_text: None,
            status: "success".into(),
            approval_scope: None,
            created_at: now(),
            subprocess_id: None,
            reverted_at: None,
        };
        insert_tool_event(&conn, &ev2).unwrap();

        let snap2 = ToolFileSnapshot {
            id: "snap-2".into(),
            session_id: s.id.clone(),
            message_id: m.id.clone(),
            tool_event_id: ev2.id.clone(),
            file_path: "sample.txt".into(),
            before_hash: Some(h1.clone()),
            after_hash: h2.clone(),
            is_new_file: false,
            reverted_at: None,
            created_at: "2026-09-28T10:01:00Z".into(),
        };
        insert_tool_file_snapshot(&conn, &snap2).unwrap();

        // Write disk to v2 (current final state)
        std::fs::write(&target_file, v2_content).unwrap();

        // 1. 验证聚合：同一轮改动弹窗中，只应看到 1 条记录，而不是 2 条
        let diffs = get_turn_diff_details(&data, &conn, &m.id).await.unwrap();
        assert_eq!(diffs.len(), 1);
        let d = &diffs[0];
        assert_eq!(d.file_path, "sample.txt");
        assert_eq!(d.modify_count, 2);
        assert_eq!(d.tool_event_ids, vec!["ev-step-1", "ev-step-2"]);
        assert_eq!(d.before_content.as_deref(), Some("line 1\nline 2"));
        assert_eq!(d.after_content, "line 1\nline 2 modified\nline 3\nline 4 added");

        // 2. 撤回该文件
        let snaps_for_file = vec![snap1.clone(), snap2.clone()];
        let rev_res = revert_snapshots(&ws, &data, &snaps_for_file, false).await.unwrap();
        assert!(rev_res.success);
        assert_eq!(std::fs::read(&target_file).unwrap(), v0_content);

        // 标记已撤回
        mark_snapshots_reverted_by_ids(&conn, &["snap-1".into(), "snap-2".into()], Some("2026-09-28T10:05:00Z")).unwrap();

        // 再次获取 Diff，应显示为已撤回状态
        let diffs_rev = get_turn_diff_details(&data, &conn, &m.id).await.unwrap();
        assert_eq!(diffs_rev.len(), 1);
        assert!(diffs_rev[0].reverted_at.is_some());

        // 3. 重新应用
        let reap_res = reapply_snapshots(&ws, &data, &snaps_for_file, false).await.unwrap();
        assert!(reap_res.success);
        assert_eq!(std::fs::read(&target_file).unwrap(), v2_content);

        mark_snapshots_reverted_by_ids(&conn, &["snap-1".into(), "snap-2".into()], None).unwrap();
        let diffs_reap = get_turn_diff_details(&data, &conn, &m.id).await.unwrap();
        assert!(diffs_reap[0].reverted_at.is_none());

        let _ = std::fs::remove_dir_all(ws.parent().unwrap());
    }
}
