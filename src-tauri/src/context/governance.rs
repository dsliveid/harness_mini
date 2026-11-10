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

    let model_limit = if let Some((pid, model)) = resolved_model {
        settings.resolve_context_limit(Some(&pid), &model)
    } else if settings.context_token_limit > 0 {
        settings.context_token_limit
    } else {
        64_000
    };

    // 实用安全水位封顶：在未显式为特定会话设置 context_token_limit 时，
    // 治理引擎将有效上限封顶在 64,000 Tokens（75% 触发线为 48,000），杜绝 800k 超大上限导致自动压缩机制彻底哑火
    model_limit.min(64_000)
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
/// 针对未压缩消息流，评估可供压缩的候选历史消息
///
/// 核心准则：
/// 1. 若当前会话正在运行中，或最后一条消息为等待回复的 user 消息：
///    必须保护当前未决/进行中的轮次，只压缩该轮次之前的历史消息（..last_user_idx）；
/// 2. 若当前会话处于空闲完成态（is_running = false 且末条非 user 消息）：
///    末轮任务（如长任务或多步自治）已彻底交付收工，全量未压缩消息皆为可总结的历史工件，
///    全部纳入候选，彻底释放自治执行积累的大量 Token 空间！
pub fn partition_compaction_candidates<'a, T: std::borrow::Borrow<crate::models::Message>>(
    uncompacted: &'a [T],
    is_running: bool,
) -> Vec<&'a crate::models::Message> {
    if uncompacted.is_empty() {
        return Vec::new();
    }

    let is_last_msg_user = uncompacted.last().map(|m| m.borrow().role == "user").unwrap_or(false);
    let need_preserve_active_turn = is_running || is_last_msg_user;

    if need_preserve_active_turn {
        // 定位当前活跃/未决轮次起点（最后一个 user 消息）
        let last_user_idx = uncompacted.iter().rposition(|m| m.borrow().role == "user");
        match last_user_idx {
            Some(idx) if idx > 0 => uncompacted[..idx].iter().map(|m| m.borrow()).collect(),
            _ => {
                if uncompacted.len() >= 2 {
                    uncompacted[..uncompacted.len().saturating_sub(1)]
                        .iter()
                        .map(|m| m.borrow())
                        .collect()
                } else {
                    Vec::new()
                }
            }
        }
    } else {
        // 空闲完成态：末轮长任务已收工，全量未压缩消息皆为可总结的历史工件
        uncompacted.iter().map(|m| m.borrow()).collect()
    }
}

pub fn evaluate_context_viewport(
    session_conn: &rusqlite::Connection,
    limit: usize,
    session_id: &str,
    is_running_hint: Option<bool>,
) -> Result<ContextViewport, String> {
    let msgs = store::all_messages(session_conn, session_id).map_err(|e| e.to_string())?;
    let compactions =
        store::list_session_compactions(session_conn, session_id).map_err(|e| e.to_string())?;
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

    // 寻找候选压缩历史消息（当前活跃轮次起点之前的已完成交互，或空闲态下的已完结大历史）
    // 运行态判定：优先使用调用方传入的内存实时状态，未传时回退到会话专属库 runs 表
    let is_running = is_running_hint.unwrap_or_else(|| {
        session_conn
            .query_row(
                "SELECT count(*) FROM runs WHERE session_id = ?1 AND status = 'running'",
                rusqlite::params![session_id],
                |r| r.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false)
    });
    let candidate_msgs = partition_compaction_candidates(&uncompacted, is_running);

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
        let compactions = state.compactions.lock();
        if compactions.values().any(|p| p.session_id == session_id) {
            return Err("当前会话已有正在处理中的上下文压缩确认请求，请先完成或等待其生效".into());
        }
    }

    // 2. 评估候选消息
    let (candidate_msgs, candidate_tokens) = {
        // 先在 Catalog 主库读取 settings 并解析生效 limit，锁立即释放（杜绝锁嵌套与自死锁）
        let limit = {
            let db = state.db.lock();
            let settings = store::get_settings(&db).unwrap_or_default();
            resolve_session_context_limit(&db, &settings, session_id)
        };
        let is_running = crate::agent::is_run_active(state, session_id);

        // 统一在会话专属分库执行消息与历史评估
        state.with_session_db(session_id, |conn| {
            let viewport = evaluate_context_viewport(conn, limit, session_id, Some(is_running))?;

            if !force && !viewport.is_compaction_recommended {
                return Ok((Vec::new(), 0));
            }
            if !viewport.can_compact {
                if force {
                    return Err("当前会话暂无可供压缩的早期历史消息（至少需要 1 轮已完成对话）".into());
                } else {
                    return Ok((Vec::new(), 0));
                }
            }

            let compactions = store::list_session_compactions(conn, session_id).unwrap_or_default();
            let max_compacted_seq = compactions.iter().map(|c| c.end_seq).max().unwrap_or(0);
            let msgs = store::all_messages(conn, session_id).unwrap_or_default();
            let uncompacted: Vec<_> = msgs
                .into_iter()
                .filter(|m| !m.queued && m.seq > max_compacted_seq && m.reverted_at.is_none())
                .collect();

            let candidates: Vec<crate::models::Message> =
                partition_compaction_candidates(&uncompacted, is_running)
                    .into_iter()
                    .cloned()
                    .collect();

            let tokens: usize = candidates
                .iter()
                .map(|m| {
                    estimate_tokens(m.content.as_deref().unwrap_or(""))
                        + m.tool_calls.as_ref().map(estimate_value_tokens).unwrap_or(0)
                })
                .sum();

            Ok((candidates, tokens))
        })?
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

    let is_running = crate::agent::is_run_active(state, session_id);
    let decision = if is_running {
        // 自治执行态自动压缩：直接应用生成的结构化摘要，零等待零阻塞继续运行
        state.emit(
            "compaction:auto_applied",
            &json!({
                "eventId": event_id,
                "sessionId": session_id,
                "summary": initial_summary,
            }),
        );
        CompactionDecision {
            approved: true,
            final_summary: initial_summary.clone(),
        }
    } else {
        state.compactions.lock().insert(
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
        match tokio::time::timeout(Duration::from_secs(timeout_secs), rx).await {
            Ok(Ok(d)) => {
                state.snapshot.set_pending_compaction(session_id, None);
                d
            }
            _ => {
                state.compactions.lock().remove(&event_id);
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
        // 核心：写入会话专属分库（保证装配上下文 assemble_execution_context 与前端查询可读出）
        state.with_session_db(session_id, |conn| {
            store::create_session_compaction(conn, &compaction)
        })?;
        // 主库双写兜底（保证单库/跨库归档兼容）
        {
            let db = state.db.lock();
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
        let limit = resolve_session_context_limit(&conn, &settings, &s.id);
        let vp = evaluate_context_viewport(&conn, limit, &s.id, None).unwrap();

        assert_eq!(vp.candidate_messages_count, 2);
        assert!(vp.can_compact);
        assert_eq!(vp.compaction_count, 0);
    }

    #[test]
    fn test_partition_compaction_candidates_idle_and_running() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::store::init_schema(&conn).unwrap();
        let s = crate::store::create_session(&conn, ".", None, "test", "confirm").unwrap();

        let _ = crate::store::new_message(&conn, &s.id, "user", Some("长任务启动".into()), false).unwrap();
        let _ = crate::store::new_message(&conn, &s.id, "assistant", Some("开始执行步骤1".into()), false).unwrap();
        let _ = crate::store::new_message(&conn, &s.id, "tool", Some("步骤1结果".into()), false).unwrap();
        let _ = crate::store::new_message(&conn, &s.id, "assistant", Some("长任务交付结案".into()), false).unwrap();

        let msgs = crate::store::all_messages(&conn, &s.id).unwrap();

        // 运行态：最后一条消息正在执行，保留正在生成的末条（候选为前 3 条）
        let running_candidates = partition_compaction_candidates(&msgs, true);
        assert_eq!(running_candidates.len(), 3);

        // 空闲态（is_running = false 且最后一条为 assistant 交付结论）：全量 4 条消息全部纳入压缩候选
        let idle_candidates = partition_compaction_candidates(&msgs, false);
        assert_eq!(idle_candidates.len(), 4);

        // 保护特性验证：若末条为刚发出、等待回复的 user 消息，严禁将其作为压缩候选（保护该未决轮次）
        let _ = crate::store::new_message(&conn, &s.id, "user", Some("新一轮未决问题".into()), false).unwrap();
        let msgs2 = crate::store::all_messages(&conn, &s.id).unwrap();
        let candidates_with_pending = partition_compaction_candidates(&msgs2, false);
        assert_eq!(candidates_with_pending.len(), 4);
    }

    #[test]
    fn test_evaluate_context_viewport_session_bundle_isolation() {
        // catalog 库：只有 session 与 settings，没有真实消息
        let catalog_conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::store::init_schema(&catalog_conn).unwrap();
        let s = crate::store::create_session(&catalog_conn, ".", None, "test", "confirm").unwrap();

        // session 专属分库：采用会话专属 Schema，存有真实会话历史
        let session_conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::session_bundle::bundle::init_session_bundle_schema(&session_conn).unwrap();

        let _ = crate::store::new_message(&session_conn, &s.id, "user", Some("历史问题".into()), false).unwrap();
        let _ = crate::store::new_message(&session_conn, &s.id, "assistant", Some("历史回答".into()), false).unwrap();
        let _ = crate::store::new_message(&session_conn, &s.id, "user", Some("当前问题".into()), false).unwrap();

        // 验证旧缺陷：若错误地在 catalog_conn 上评估，消息数为 0，can_compact 为 false（误报无可压缩历史）
        let settings = crate::models::SettingsData::default();
        let limit = resolve_session_context_limit(&catalog_conn, &settings, &s.id);
        let bug_vp = evaluate_context_viewport(&catalog_conn, limit, &s.id, None).unwrap();
        assert_eq!(bug_vp.candidate_messages_count, 0);
        assert!(!bug_vp.can_compact);

        // 验证新重构的正确性：在 session_conn 上评估，从分库读取到消息，can_compact 为 true！
        let fixed_vp = evaluate_context_viewport(&session_conn, limit, &s.id, None).unwrap();
        assert_eq!(fixed_vp.candidate_messages_count, 2);
        assert!(fixed_vp.can_compact);

        // 验证持久化写分库后，assemble_execution_context 能够成功从分库提取并挂载备忘录
        let compaction = SessionCompaction {
            id: "comp-1".to_string(),
            session_id: s.id.clone(),
            start_seq: 1,
            end_seq: 2,
            summary_markdown: "历史备忘录内容".to_string(),
            tokens_before: 500,
            created_at: "2026-11-10T00:00:00Z".to_string(),
        };
        crate::store::create_session_compaction(&session_conn, &compaction).unwrap();

        let (exec_msgs, _, _) = crate::context::assemble_execution_context(&session_conn, &s.id, "系统提示", 32000);
        let msgs_val = exec_msgs.unwrap();
        let has_memo = msgs_val.iter().any(|v| {
            v.get("content").and_then(|c| c.as_str()).map(|t| t.contains("历史备忘录内容")).unwrap_or(false)
        });
        assert!(has_memo);
    }
}
