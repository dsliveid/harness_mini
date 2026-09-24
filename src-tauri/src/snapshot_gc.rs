//! CAS 影子快照垃圾回收引擎（Orphan Blob Garbage Collection）

use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, SystemTime};

/// 扫描并清理孤儿 CAS Blob（未被任何 tool_file_snapshots 引用，且修改时间超过 min_age 的文件）
pub async fn gc_orphan_blobs(
    data_dir: &Path,
    active_hashes: &HashSet<String>,
    min_age: Duration,
) -> Result<usize, String> {
    let blobs_dir = crate::snapshot_fs::blobs_dir(data_dir);
    if !blobs_dir.exists() {
        return Ok(0);
    }

    let mut removed_count = 0usize;
    let now = SystemTime::now();

    let mut dir_entries = tokio::fs::read_dir(&blobs_dir)
        .await
        .map_err(|e| format!("读取 CAS blobs 目录失败: {e}"))?;

    while let Ok(Some(prefix_entry)) = dir_entries.next_entry().await {
        let prefix_path = prefix_entry.path();
        if prefix_path.is_dir() {
            let prefix = prefix_entry
                .file_name()
                .to_str()
                .unwrap_or("")
                .to_string();

            if let Ok(mut sub_entries) = tokio::fs::read_dir(&prefix_path).await {
                while let Ok(Some(file_entry)) = sub_entries.next_entry().await {
                    let file_path = file_entry.path();
                    if !file_path.is_file() {
                        continue;
                    }

                    let rest = file_entry
                        .file_name()
                        .to_str()
                        .unwrap_or("")
                        .to_string();

                    let full_hash = format!("{}{}", prefix, rest);
                    if full_hash.len() != 64 {
                        continue;
                    }

                    // 检查是否被数据库中任何有效快照引用
                    if active_hashes.contains(&full_hash) {
                        continue;
                    }

                    // 宽限期检查：避免并发写入中的临时文件被意外删除
                    if let Ok(meta) = file_entry.metadata().await {
                        if let Ok(modified) = meta.modified() {
                            if let Ok(age) = now.duration_since(modified) {
                                if age < min_age {
                                    continue; // 尚在宽限期内，暂不回收
                                }
                            }
                        }
                    }

                    // 执行安全删除
                    if tokio::fs::remove_file(&file_path).await.is_ok() {
                        removed_count += 1;
                    }
                }
            }
        }
    }

    Ok(removed_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;
    use crate::snapshot_fs::*;
    use crate::store::*;
    use rusqlite::Connection;
    use std::path::PathBuf;

    fn temp_test_env(tag: &str) -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("hm_gc_test_{tag}_{}", uuid::Uuid::new_v4()));
        let ws = base.join("ws");
        let data = base.join("data");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        (ws, data)
    }

    #[test]
    fn test_50_turn_sliding_window_eviction() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let s = create_session(&conn, "D:\\test", None, "test", "confirm").unwrap();

        let mut turn_msg_ids = Vec::new();

        // 模拟产生 55 轮交互对话，每轮产生 2 个快照
        for i in 1..=55 {
            let msg = new_message(&conn, &s.id, "assistant", Some("test".into()), false).unwrap();
            turn_msg_ids.push(msg.id.clone());

            let ev1 = ToolEvent {
                id: format!("ev-{}-1", i),
                message_id: msg.id.clone(),
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
            let ev2 = ToolEvent {
                id: format!("ev-{}-2", i),
                message_id: msg.id.clone(),
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
            insert_tool_event(&conn, &ev2).unwrap();

            let time_str = format!("2026-09-01T{:02}:{:02}:{:02}Z", (i / 3600) % 24, (i / 60) % 60, i % 60);

            let snap1 = ToolFileSnapshot {
                id: format!("snap-{}-1", i),
                session_id: s.id.clone(),
                message_id: msg.id.clone(),
                tool_event_id: ev1.id.clone(),
                file_path: "file1.txt".into(),
                before_hash: None,
                after_hash: format!("hash_after_1_{i:04}"),
                is_new_file: true,
                reverted_at: None,
                created_at: time_str.clone(),
            };
            let snap2 = ToolFileSnapshot {
                id: format!("snap-{}-2", i),
                session_id: s.id.clone(),
                message_id: msg.id.clone(),
                tool_event_id: ev2.id.clone(),
                file_path: "file2.txt".into(),
                before_hash: None,
                after_hash: format!("hash_after_2_{i:04}"),
                is_new_file: true,
                reverted_at: None,
                created_at: time_str,
            };

            insert_tool_file_snapshot(&conn, &snap1).unwrap();
            insert_tool_file_snapshot(&conn, &snap2).unwrap();
        }

        // 验证快照表总共只保留 50 轮消息，最早的 5 轮（轮次 1~5，共 10 条快照）被精准淘汰
        let remaining = list_snapshots_for_session(&conn, &s.id).unwrap();
        let distinct_turns: std::collections::HashSet<_> = remaining.iter().map(|s| &s.message_id).collect();
        assert_eq!(distinct_turns.len(), 50);
        assert_eq!(remaining.len(), 100);

        // 最早的第 1~5 轮已被淘汰
        for i in 0..5 {
            let mid = &turn_msg_ids[i];
            assert!(!distinct_turns.contains(mid));
        }

        // 第 6~55 轮（索引 5..55）完整保留
        for i in 5..55 {
            let mid = &turn_msg_ids[i];
            assert!(distinct_turns.contains(mid));
        }
    }

    #[tokio::test]
    async fn test_gc_orphan_blobs_cleans_unreferenced() {
        let (_ws, data) = temp_test_env("gc");
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let s = create_session(&conn, "D:\\test", None, "test", "confirm").unwrap();
        let m = new_message(&conn, &s.id, "assistant", Some("content".into()), false).unwrap();
        let ev = ToolEvent {
            id: "ev1".into(),
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

        // 1. 写入两个 Blob：一个被有效引用，另一个未被引用（孤儿）
        let active_content = b"active referenced blob content";
        let orphan_content = b"orphan unreferenced blob content";

        let active_hash = save_blob(&data, active_content).unwrap();
        let orphan_hash = save_blob(&data, orphan_content).unwrap();

        assert!(blob_exists(&data, &active_hash));
        assert!(blob_exists(&data, &orphan_hash));

        // 2. 在数据库中只记录 active_hash
        let snap = ToolFileSnapshot {
            id: "snap-active".into(),
            session_id: s.id.clone(),
            message_id: m.id.clone(),
            tool_event_id: ev.id.clone(),
            file_path: "app.rs".into(),
            before_hash: None,
            after_hash: active_hash.clone(),
            is_new_file: true,
            reverted_at: None,
            created_at: now(),
        };
        insert_tool_file_snapshot(&conn, &snap).unwrap();

        let active_hashes = collect_active_snapshot_hashes(&conn).unwrap();
        assert!(active_hashes.contains(&active_hash));
        assert!(!active_hashes.contains(&orphan_hash));

        // 3. 运行 GC（min_age 设为 0 以立即回收）
        let cleaned = gc_orphan_blobs(&data, &active_hashes, Duration::from_secs(0))
            .await
            .unwrap();
        assert_eq!(cleaned, 1);

        // 4. 验证：活跃 Blob 完好无损，孤儿 Blob 被删除
        assert!(blob_exists(&data, &active_hash));
        assert!(!blob_exists(&data, &orphan_hash));

        let _ = std::fs::remove_dir_all(data.parent().unwrap());
    }
}
