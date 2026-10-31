use crate::llm::{self, LlmCfg};
use crate::models::{
    estimate_tokens, estimate_value_tokens, CompactionDecision, CompactionRequest,
    SessionCompaction,
};
use crate::store;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

fn truncate_preview(s: &str, max_chars: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{}...", truncated)
    }
}

fn extract_msg_tokens(m: &crate::models::Message) -> (usize, usize) {
    let pt = m
        .prompt_tokens
        .or_else(|| {
            m.usage.as_ref().and_then(|u| {
                u.get("prompt_tokens")
                    .or_else(|| u.get("promptTokens"))
                    .and_then(|v| v.as_u64())
            })
        })
        .unwrap_or(0) as usize;
    let ct = m
        .completion_tokens
        .or_else(|| {
            m.usage.as_ref().and_then(|u| {
                u.get("completion_tokens")
                    .or_else(|| u.get("completionTokens"))
                    .and_then(|v| v.as_u64())
            })
        })
        .unwrap_or(0) as usize;
    (pt, ct)
}

/// 统一的上下文视口状态（领域聚合，Single Source of Truth）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextViewport {
    /// 活跃基准 Token 消耗（优先取最近一次真实 Usage + 准确增量）
    pub baseline_tokens: usize,
    /// 当前生效的上下文总上限
    pub limit: usize,
    /// 负载比例 (0.0 ~ 1.0)
    pub watermark_ratio: f32,
    /// 负载百分比 (0.0 ~ 100.0)
    pub percentage: f32,
    /// 剩余可用空间
    pub remaining_tokens: usize,
    /// 智能语义压缩触发阈值 (75%)
    pub compaction_threshold: usize,
    /// 距触发压缩剩余 Token (0 表示已达到或超出)
    pub tokens_to_compaction: usize,
    /// 可压缩候选消息数量
    pub candidate_messages_count: usize,
    /// 可压缩候选消息 Token 量
    pub candidate_tokens: usize,
    /// 是否具备可压缩条件（至少存在 1 轮可安全折叠的历史）
    pub can_compact: bool,
    /// 系统是否建议执行压缩
    pub is_compaction_recommended: bool,
    /// 是否基于字符经验估算（false 表示包含大模型真实 Usage）
    pub is_estimated: bool,
    /// 历史已压缩次数
    pub compaction_count: usize,
}

/// 解析某会话生效的上下文 Token 上限
pub fn resolve_session_context_limit(
    conn: &rusqlite::Connection,
    settings: &crate::models::SettingsData,
    session_id: &str,
) -> usize {
    let session = crate::store::get_session(conn, session_id).ok().flatten();
    if let Some(limit) = session.as_ref().and_then(|s| s.context_token_limit) {
        return limit;
    }
    let resolved_model = if let Some(s) = &session {
        if let (Some(pid), Some(mid)) = (s.provider_id.as_deref(), s.model_id.as_deref()) {
            Some((pid.to_string(), mid.to_string()))
        } else {
            crate::models::resolve_active_model(settings)
                .map(|(p, m)| (p.id.clone(), m.to_string()))
        }
    } else {
        crate::models::resolve_active_model(settings).map(|(p, m)| (p.id.clone(), m.to_string()))
    };

    if let Some((pid, model)) = resolved_model {
        settings.resolve_context_limit(Some(&pid), &model)
    } else if settings.context_token_limit > 0 {
        settings.context_token_limit
    } else {
        64_000
    }
}

/// 解析会话关联的大模型调用配置
pub fn resolve_session_llm_cfg(
    conn: &rusqlite::Connection,
    settings: &crate::models::SettingsData,
    session_id: &str,
) -> Result<LlmCfg, String> {
    let session = crate::store::get_session(conn, session_id)?
        .ok_or_else(|| format!("会话不存在: {session_id}"))?;

    let parent_session = if let Some(ref pid) = session.parent_session_id {
        crate::store::get_session(conn, pid).ok().flatten()
    } else {
        None
    };

    let resolved_model = if let (Some(pid), Some(mid)) =
        (session.provider_id.as_deref(), session.model_id.as_deref())
    {
        settings
            .providers
            .iter()
            .find(|p| p.id == pid && p.models.iter().any(|m| m == mid))
            .map(|p| (p.clone(), mid.to_string()))
            .or_else(|| {
                crate::models::resolve_active_model(settings)
                    .map(|(p, m)| (p.clone(), m.to_string()))
            })
    } else if let Some(parent) = &parent_session {
        if let (Some(pid), Some(mid)) =
            (parent.provider_id.as_deref(), parent.model_id.as_deref())
        {
            settings
                .providers
                .iter()
                .find(|p| p.id == pid && p.models.iter().any(|m| m == mid))
                .map(|p| (p.clone(), mid.to_string()))
                .or_else(|| {
                    crate::models::resolve_active_model(settings)
                        .map(|(p, m)| (p.clone(), m.to_string()))
                })
        } else {
            crate::models::resolve_active_model(settings).map(|(p, m)| (p.clone(), m.to_string()))
        }
    } else {
        crate::models::resolve_active_model(settings).map(|(p, m)| (p.clone(), m.to_string()))
    };

    let Some((pc, model)) = resolved_model else {
        return Err("尚未配置模型厂商，请先在设置中添加厂商与模型并选择。".into());
    };

    let effective_reasoning_effort = session
        .reasoning_effort
        .as_deref()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            parent_session
                .as_ref()
                .and_then(|p| p.reasoning_effort.as_deref())
                .filter(|s| !s.is_empty())
        })
        .or(settings.reasoning_effort.as_deref())
        .filter(|s| !s.is_empty() && *s != "default")
        .map(|s| s.to_string());

    let effective_proxy_url = settings.effective_proxy_url();
    Ok(LlmCfg {
        base_url: pc.base_url.clone(),
        api_key: pc.api_key.clone(),
        model: model.clone(),
        protocol: pc.get_model_protocol(&model),
        reasoning_effort: effective_reasoning_effort,
        proxy_url: effective_proxy_url,
        session_id: Some(session_id.to_string()),
    })
}

/// 集中评估当前会话的上下文视口指标（前后端统一唯一指标源）
pub fn evaluate_context_viewport(
    conn: &rusqlite::Connection,
    settings: &crate::models::SettingsData,
    session_id: &str,
) -> Result<ContextViewport, String> {
    let limit = resolve_session_context_limit(conn, settings, session_id);
    let msgs = store::all_messages(conn, session_id).map_err(|e| e.to_string())?;
    let compactions =
        store::list_session_compactions(conn, session_id).map_err(|e| e.to_string())?;
    let max_compacted_seq = compactions.iter().map(|c| c.end_seq).max().unwrap_or(0);
    let compaction_count = compactions.len();

    let uncompacted: Vec<_> = msgs
        .iter()
        .filter(|m| !m.queued && m.seq > max_compacted_seq && m.reverted_at.is_none())
        .collect();

    // 优先倒序寻找最近一条具备真实 Usage 且在合理物理上限范围内的 assistant 消息
    // 自动过滤被历史长任务累计账单污染的异常值（pt <= limit * 3）
    let max_sane_pt = limit.saturating_mul(3);
    let last_asst_idx = uncompacted.iter().rposition(|m| {
        if m.role != "assistant" {
            return false;
        }
        let (pt, _) = extract_msg_tokens(m);
        pt > 0 && pt <= max_sane_pt
    });

    let (baseline_tokens, is_estimated) = if let Some(idx) = last_asst_idx {
        let last_asst = uncompacted[idx];
        let (pt, ct) = extract_msg_tokens(last_asst);
        let mut active = pt + ct;
        // 累加该 assistant 之后的后续增量消息
        for m in &uncompacted[idx + 1..] {
            let text = format!(
                "{}{}",
                m.content.as_deref().unwrap_or(""),
                m.reasoning.as_deref().unwrap_or("")
            );
            active += estimate_tokens(&text);
            if let Some(ref tcs) = m.tool_calls {
                active += estimate_value_tokens(tcs);
            }
            if let Some(ref atts) = m.attachments {
                for att in atts {
                    active += if att.is_image { 1000 } else { 200 };
                }
            }
        }
        (active, false)
    } else {
        // 无真实 Usage，从头估算：基线开销（含系统提示词与全量工具 Schemas 预估约 3500 Tokens）
        let mut est = if !uncompacted.is_empty() { 3500 } else { 0 };
        for m in &uncompacted {
            let text = format!(
                "{}{}",
                m.content.as_deref().unwrap_or(""),
                m.reasoning.as_deref().unwrap_or("")
            );
            est += estimate_tokens(&text);
            if let Some(ref tcs) = m.tool_calls {
                est += estimate_value_tokens(tcs);
            }
            if let Some(ref atts) = m.attachments {
                for att in atts {
                    est += if att.is_image { 1000 } else { 200 };
                }
            }
        }
        for c in &compactions {
            est += estimate_tokens(&c.summary_markdown);
        }
        (est, true)
    };

    // 寻找候选压缩历史消息（当前活跃轮次起点之前的已完成交互）
    let last_user_idx = uncompacted.iter().rposition(|m| m.role == "user");
    let candidate_msgs: Vec<&crate::models::Message> = match last_user_idx {
        Some(idx) if idx > 0 => uncompacted[..idx].to_vec(),
        _ => {
            if uncompacted.len() >= 2 {
                uncompacted[..uncompacted.len().saturating_sub(1)].to_vec()
            } else {
                Vec::new()
            }
        }
    };

    let candidate_messages_count = candidate_msgs.len();
    let candidate_tokens: usize = candidate_msgs
        .iter()
        .map(|m| {
            estimate_tokens(m.content.as_deref().unwrap_or(""))
                + m.tool_calls.as_ref().map(estimate_value_tokens).unwrap_or(0)
        })
        .sum();

    let compaction_threshold = (limit * 75) / 100;
    let tokens_to_compaction = compaction_threshold.saturating_sub(baseline_tokens);
    let remaining_tokens = limit.saturating_sub(baseline_tokens);
    let watermark_ratio = (baseline_tokens as f32 / limit.max(1) as f32).min(1.0);
    let percentage = (watermark_ratio * 1000.0).round() / 10.0;
    let can_compact = candidate_messages_count > 0;
    let is_compaction_recommended =
        baseline_tokens >= compaction_threshold || (limit > 0 && candidate_tokens >= limit / 2);

    Ok(ContextViewport {
        baseline_tokens,
        limit,
        watermark_ratio,
        percentage,
        remaining_tokens,
        compaction_threshold,
        tokens_to_compaction,
        candidate_messages_count,
        candidate_tokens,
        can_compact,
        is_compaction_recommended,
        is_estimated,
        compaction_count,
    })
}

/// 上下文压缩核心执行引擎（控制面统一核心，供 UI/命令/工具/自动策略无差别调度）
pub async fn propose_and_execute_compaction(
    app: &AppHandle,
    state: &crate::AppState,
    session_id: &str,
    cfg: &LlmCfg,
    manual_focus: Option<&str>,
    force: bool,
) -> Result<bool, String> {
    // 1. 检查是否已有未决的压缩请求
    {
        let compactions = state.compactions.lock().unwrap();
        if compactions.values().any(|p| p.session_id == session_id) {
            return Err("当前会话已有正在处理中的上下文压缩确认请求，请先完成或等待其生效".into());
        }
    }

    // 2. 评估候选消息
    let (candidate_msgs, candidate_tokens) = {
        let db = state.db.lock().unwrap();
        let settings = store::get_settings(&db).unwrap_or_default();
        let viewport = evaluate_context_viewport(&db, &settings, session_id)?;

        if !force && !viewport.is_compaction_recommended {
            return Ok(false);
        }
        if !viewport.can_compact {
            if force {
                return Err("当前会话暂无可供压缩的早期历史消息（至少需要 1 轮已完成对话）".into());
            } else {
                return Ok(false);
            }
        }

        let compactions = store::list_session_compactions(&db, session_id).unwrap_or_default();
        let max_compacted_seq = compactions.iter().map(|c| c.end_seq).max().unwrap_or(0);
        let msgs = store::all_messages(&db, session_id).unwrap_or_default();
        let uncompacted: Vec<_> = msgs
            .into_iter()
            .filter(|m| !m.queued && m.seq > max_compacted_seq && m.reverted_at.is_none())
            .collect();

        let last_user_idx = uncompacted.iter().rposition(|m| m.role == "user");
        let candidates = match last_user_idx {
            Some(idx) if idx > 0 => uncompacted[..idx].to_vec(),
            _ => {
                if uncompacted.len() >= 2 {
                    uncompacted[..uncompacted.len().saturating_sub(1)].to_vec()
                } else {
                    Vec::new()
                }
            }
        };

        let tokens: usize = candidates
            .iter()
            .map(|m| {
                estimate_tokens(m.content.as_deref().unwrap_or(""))
                    + m.tool_calls.as_ref().map(estimate_value_tokens).unwrap_or(0)
            })
            .sum();

        (candidates, tokens)
    };

    if candidate_msgs.is_empty() {
        return if force {
            Err("未筛选到有效的可压缩历史轮次".into())
        } else {
            Ok(false)
        };
    }

    // 3. 格式化待压缩的历史对话文本
    let mut history_str = String::new();
    for m in &candidate_msgs {
        let role_name = match m.role.as_str() {
            "user" => "【用户】",
            "assistant" => "【助手】",
            "tool" => "【工具返回】",
            _ => "【系统】",
        };
        let c = m.content.as_deref().unwrap_or("");
        if !c.is_empty() {
            let truncated = if c.len() > 800 {
                let safe_len = c.char_indices().nth(800).map(|(i, _)| i).unwrap_or(c.len());
                format!("{}...[已截断]", &c[..safe_len])
            } else {
                c.to_string()
            };
            history_str.push_str(&format!("{role_name}: {truncated}\n\n"));
        }
    }

    let focus_clause = if let Some(focus) = manual_focus.filter(|f| !f.trim().is_empty()) {
        format!(
            "\n⚠️ 用户特别要求的总结聚焦重点（请务必优先突出保留此部分）：\n{}\n",
            focus.trim()
        )
    } else {
        String::new()
    };

    let summary_prompt = format!(
        r#"你是一个专业的编码项目备忘助手。请将以下对话历史进行结构化精炼总结，提炼为一份高质量 Markdown 格式的上下文备忘录。
该备忘录将在后续对话中替换这段历史，帮助你与用户保持关键记忆。
{}
待压缩的对话历史：
---
{}
---

请严格按以下结构输出 Markdown（语言简洁干练，重点保留技术决策与代码变更，剔除无用寒暄）：
### 🎯 任务背景与核心目标
（简要说明用户最初的诉求与技术背景）
### 🔑 关键技术决策与约定
（架构设计、技术选型、接口约定或不可违背的约束）
### 📁 涉及文件与修改记录
（已读取、创建或编辑的文件列表及主要变更点）
### 📌 历史遗留与注意事项
（之前步骤中发现的坑、未决问题或后续需要注意的事项）"#,
        focus_clause, history_str
    );

    let summary_messages = vec![
        json!({"role": "system", "content": "你是一个严谨客观的技术项目总结助手，擅长提取代码开发对话中的关键事实。"}),
        json!({"role": "user", "content": summary_prompt}),
    ];

    let summary_res = llm::chat_stream(cfg, &summary_messages, &[], |_| {}, |_| {}).await;

    let initial_summary = match summary_res {
        Ok(r) if !r.content.trim().is_empty() => r.content,
        Ok(_) => return Err("模型生成的摘要内容为空，压缩中止".into()),
        Err(e) => return Err(format!("调用模型生成压缩摘要失败: {e}")),
    };

    let event_id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = tokio::sync::oneshot::channel();

    let start_seq = candidate_msgs.first().map(|m| m.seq).unwrap_or(0);
    let end_seq = candidate_msgs.last().map(|m| m.seq).unwrap_or(0);
    let start_preview = truncate_preview(
        candidate_msgs
            .first()
            .and_then(|m| m.content.as_deref())
            .unwrap_or(""),
        40,
    );
    let end_preview = truncate_preview(
        candidate_msgs
            .last()
            .and_then(|m| m.content.as_deref())
            .unwrap_or(""),
        40,
    );

    let timeout_secs = 30u64;

    let req = CompactionRequest {
        event_id: event_id.clone(),
        session_id: session_id.to_string(),
        start_seq,
        end_seq,
        start_preview,
        end_preview,
        message_count: candidate_msgs.len(),
        tokens_before: candidate_tokens,
        summary: initial_summary.clone(),
        timeout_seconds: timeout_secs,
    };

    state.compactions.lock().unwrap().insert(
        event_id.clone(),
        crate::PendingCompaction {
            session_id: session_id.to_string(),
            tx,
        },
    );

    state
        .snapshot
        .set_pending_compaction(session_id, Some(req.clone()));
    state.emit("compaction:request", &req);

    // 等待用户确认（默认阻塞等待 30 秒；超时自动应用）
    let decision = match tokio::time::timeout(Duration::from_secs(timeout_secs), rx).await {
        Ok(Ok(d)) => {
            state.snapshot.set_pending_compaction(session_id, None);
            d
        }
        _ => {
            state.compactions.lock().unwrap().remove(&event_id);
            state.snapshot.set_pending_compaction(session_id, None);
            state.emit(
                "compaction:timeout",
                &json!({
                    "eventId": event_id,
                    "sessionId": session_id,
                    "autoApplied": true,
                }),
            );
            CompactionDecision {
                approved: true,
                final_summary: initial_summary.clone(),
            }
        }
    };

    if decision.approved {
        let final_markdown = if decision.final_summary.trim().is_empty() {
            initial_summary
        } else {
            decision.final_summary
        };
        let compaction = SessionCompaction {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            start_seq,
            end_seq,
            summary_markdown: final_markdown,
            tokens_before: candidate_tokens,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        {
            let db = state.db.lock().unwrap();
            let _ = store::create_session_compaction(&db, &compaction);
        }
        let _ = app.emit("compaction:applied", &compaction);
        let _ = app.emit("session:compacted", json!({ "sessionId": session_id }));
        Ok(true)
    } else {
        Ok(false)
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evaluate_context_viewport() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::store::init_schema(&conn).unwrap();
        let s = crate::store::create_session(&conn, ".", None, "test", "confirm").unwrap();

        // 插入消息
        let _ = crate::store::new_message(&conn, &s.id, "user", Some("问题1".into()), false).unwrap();
        let _ = crate::store::new_message(&conn, &s.id, "assistant", Some("回答1".into()), false).unwrap();
        let _ = crate::store::new_message(&conn, &s.id, "user", Some("问题2".into()), false).unwrap();

        let settings = crate::models::SettingsData::default();
        let vp = evaluate_context_viewport(&conn, &settings, &s.id).unwrap();

        assert_eq!(vp.candidate_messages_count, 2);
        assert!(vp.can_compact);
        assert_eq!(vp.compaction_count, 0);
    }
}
