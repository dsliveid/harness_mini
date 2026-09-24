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

    // 1. 冲突检测阶段：若未指定 force，任何一个文件冲突都安全拦截，不产生部分回滚
    if !force {
        let mut conflicted = Vec::new();
        for s in snapshots {
            if let Ok(Some(cf)) = check_revert_conflict(workspace, s).await {
                if !conflicted.contains(&cf) {
                    conflicted.push(cf);
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

    // 1. 冲突检测
    if !force {
        let mut conflicted = Vec::new();
        for s in snapshots {
            if let Ok(Some(cf)) = check_reapply_conflict(workspace, s).await {
                if !conflicted.contains(&cf) {
                    conflicted.push(cf);
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

/// 解析快照列表并生成对比 diff 详情
pub async fn get_snapshots_diff_details(
    data_dir: &Path,
    snapshots: &[ToolFileSnapshot],
) -> Result<Vec<SnapshotFileDiff>, String> {
    let mut diffs = Vec::new();
    for s in snapshots {
        let before_text = if let Some(ref bh) = s.before_hash {
            let bytes = read_blob_async(data_dir, bh).await.unwrap_or_default();
            Some(String::from_utf8_lossy(&bytes).to_string())
        } else {
            None
        };

        let after_bytes = read_blob_async(data_dir, &s.after_hash).await.unwrap_or_default();
        let after_text = String::from_utf8_lossy(&after_bytes).to_string();

        let old_str = before_text.as_deref().unwrap_or("");
        let d = diffutil::diff_lines(old_str, &after_text, 64 * 1024);

        diffs.push(SnapshotFileDiff {
            file_path: s.file_path.clone(),
            is_new_file: s.is_new_file,
            added: d.added,
            removed: d.removed,
            diff_text: d.text,
            before_content: before_text,
            after_content: after_text,
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
}
