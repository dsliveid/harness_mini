use crate::models::TaskCheckpoint;

/// 安全按字节上限截断字符串，确保落在 UTF-8 字符边界上
pub fn safe_truncate(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// 组装工作记忆块（Working Memory Block），包含历史检查点总结
pub fn build_working_memory_block(checkpoints: &[TaskCheckpoint], goal: &str) -> String {
    if checkpoints.is_empty() {
        return format!("【总目标锚点】: {}\n尚未有已归档的历史阶段检查点，当前为首个启动阶段。", goal);
    }
    let mut text = format!("【总目标锚点】: {}\n\n【已完成历史阶段检查点与沉淀事实 (Working Memory)】:\n", goal);
    for cp in checkpoints {
        // 允许保留更丰富完整的阶段产出契约（扩容至 1500 字节），彻底根除因信息过切导致下游盲目探索
        let summary_preview = if cp.summary.len() > 1500 {
            format!("{}…", safe_truncate(&cp.summary, 1500))
        } else {
            cp.summary.clone()
        };
        text.push_str(&format!(
            "- 阶段 #{}: 结论摘要 -> {}\n",
            cp.step_number,
            summary_preview
        ));
    }
    text.push_str("\n⚠️ 请基于上述已完成的成果继续推进，严禁无故推翻或重复已完成的修改！");
    text
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn test_safe_truncate_ascii() {
        let s = "Hello, world!";
        assert_eq!(safe_truncate(s, 5), "Hello");
        assert_eq!(safe_truncate(s, 20), "Hello, world!");
    }

    #[test]
    fn test_safe_truncate_multibyte_chinese() {
        let s = "你好，世界！";
        // '你' 是 3 字节，'好' 是 3 字节
        assert_eq!(safe_truncate(s, 3), "你");
        assert_eq!(safe_truncate(s, 4), "你"); // 截断在非边界处时安全回退到 3 字节
        assert_eq!(safe_truncate(s, 5), "你");
        assert_eq!(safe_truncate(s, 6), "你好");
    }

    #[test]
    fn test_build_working_memory_block_with_multibyte_boundary() {
        let cp = TaskCheckpoint {
            id: "cp-1".to_string(),
            task_id: "task-1".to_string(),
            step_number: 1,
            subtask_id: Some("sub-1".to_string()),
            status: "completed".to_string(),
            summary: "这".repeat(600), // 1800 bytes > 1500 limit
            working_memory: String::new(),
            git_commit_hash: None,
            created_at: "2026-09-24".to_string(),
        };
        // This should not panic!
        let block = build_working_memory_block(&[cp], "测试多字节截断");
        assert!(block.contains("阶段 #1"));
        assert!(block.contains("…"));
    }
}
