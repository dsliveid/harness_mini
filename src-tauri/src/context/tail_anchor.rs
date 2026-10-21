use crate::models::Session;
use rusqlite::Connection;
use std::path::Path;

/// 构建尾部动态运行时环境附注 (Layer 4)
/// 
/// 核心准则：将任务计划、最新知识碎记、中断恢复与特定决策指令等高频变动信息，
/// 集中收敛并挂载在最新一条 User 消息的尾部。
/// 这样前序数十万 Token 的完整历史前缀不会受到任何破坏，保持 100% 缓存命中！
pub fn build_runtime_anchor(
    conn: &Connection,
    session: &Session,
    user_prompt: Option<&str>,
) -> Option<String> {
    let mut sections = Vec::new();

    // 1. 动态权威任务计划进度与步骤清单
    if !session.workspace_path.is_empty() {
        let ws = Path::new(&session.workspace_path);
        if let Some(plan_text) = crate::plan::load_active_plan_context(ws, &session.id, Some(conn)) {
            sections.push(plan_text);
        }
    }

    // 2. 近期高价值边读边记重点沉淀（digests）
    if !session.workspace_path.is_empty() {
        let ws = Path::new(&session.workspace_path);
        if let Some(digests_text) = crate::memory::load_recent_digests(ws, 3) {
            sections.push(format!("## 💡 本项目近期边读边记重点沉淀 (可秒级召回)\n{}", digests_text));
        }
    }

    // 3. 运行中断与进度重对齐标记
    let last_run_was_interrupted: bool = conn
        .query_row(
            "SELECT status FROM runs WHERE session_id = ?1 ORDER BY started_at DESC LIMIT 1",
            rusqlite::params![session.id],
            |r| r.get::<_, String>(0),
        )
        .map(|st| st == "cancelled" || st == "failed" || st == "interrupted")
        .unwrap_or(false);

    if last_run_was_interrupted {
        sections.push(
            "## ⚠️ 断点恢复与进度重对齐提示\n\
            > 检测到上一轮对话曾被中途停止或异常中断。当前工作区中可能已经实施了部分文件修改或测试。\n\
            > **重要工作原则**：请在编写新代码前，首先快速核对当前工作区实际已修改的文件与构建状态；若原方案/任务清单中的前序步骤在上一轮已实际完成，请先调用 `update_plan` 推进步骤状态至 done，确保任务清单与实际代码进度严格同步，再继续推进后续任务。"
                .to_string(),
        );
    }

    // 4. Jev 决策先行指令感知
    let is_jev_command = user_prompt
        .map(|c| crate::agent::extract_jev_intent(c).0)
        .unwrap_or(false);

    if is_jev_command {
        sections.push(
            "## 🎯 Jev 场景化决策先行准则（必须严格遵守）\n\
            > 用户显式发起了 `/jev:决策` 指令，要求对当前诉求进行严谨的意图定性与方案质检决策。\n\
            > **执行约束**：\n\
            > 1. 【首步必须发起决策工具调用】：在当前轮次的第一步（Step 1），你必须深入剖析用户核心意图与交付边界，并在 `judge_decision`（是非判断）、`choice_decision`（多方案选择）、`score_decision`（方案质量与风险评分）中自主选择最适宜的一个决策工具发起调用！\n\
            > 2. 【决策完成后自动向下推进工程落地】：决策工具调用通过后，必须紧接着围绕确认的核心方向，自动继续调用后续生产工具顺畅向下推进工程落地，直至阶段交付！"
                .to_string(),
        );
    }

    if sections.is_empty() {
        None
    } else {
        Some(format!(
            "<runtime_environment_anchor>\n{}\n</runtime_environment_anchor>",
            sections.join("\n\n")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tail_anchor_empty_when_no_ephemeral_state() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::store::init_schema(&conn).unwrap();
        let session = Session {
            id: "s1".into(),
            workspace_path: "".into(),
            ..Default::default()
        };
        let anchor = build_runtime_anchor(&conn, &session, Some("普通对话"));
        assert!(anchor.is_none());
    }

    #[test]
    fn test_tail_anchor_includes_jev_when_intent_specified() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::store::init_schema(&conn).unwrap();
        let session = Session {
            id: "s1".into(),
            workspace_path: "".into(),
            ..Default::default()
        };
        let anchor = build_runtime_anchor(&conn, &session, Some("/jev:决策 评估重构方案"));
        assert!(anchor.is_some());
        let a = anchor.unwrap();
        assert!(a.contains("<runtime_environment_anchor>"));
        assert!(a.contains("Jev 场景化决策先行准则"));
        assert!(a.contains("judge_decision"));
    }
}
