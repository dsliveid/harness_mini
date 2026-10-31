use crate::models::{estimate_tokens, estimate_value_tokens, Attachment, TruncationNotice};
use crate::store;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ExecutionContext {
    pub messages: Vec<Value>,
    pub est_tokens: usize,
    pub truncation_notice: Option<TruncationNotice>,
}

fn truncate_preview(s: &str, max_chars: usize) -> String {
    let s = s.trim().replace(['\r', '\n'], " ");
    let mut chars = s.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

/// 组装不可变执行上下文流 (Layer 1 ~ Layer 4)
/// 
/// 遵循不可变前缀范式：
/// 1. messages[0] 为纯静态 System Prompt；
/// 2. 历史消息（User/Assistant/Tool）严格保持 Append-only，废除事后篡改；
/// 3. 用户消息附带其产生时刻冻结的 runtime_anchor，保证多轮次间前缀 100% 字节级一致；
/// 4. 仅在极端 token 压力（超 80% 水位）时才对远期历史实施保护性收敛。
pub fn assemble_execution_context(
    conn: &Connection,
    session_id: &str,
    system_prompt: &str,
    token_limit: usize,
) -> (Result<Vec<Value>, String>, usize, Option<TruncationNotice>) {
    let msgs = match store::all_messages(conn, session_id) {
        Ok(m) => m,
        Err(e) => return (Err(e), 0, None),
    };
    let compactions = store::list_session_compactions(conn, session_id).unwrap_or_default();
    let max_compacted_seq = compactions.iter().map(|c| c.end_seq).max().unwrap_or(0);

    // tool_call_id -> (结果文本, 附件)
    let mut tool_results: HashMap<String, (String, Option<Vec<Attachment>>)> = HashMap::new();
    for m in &msgs {
        if m.reverted_at.is_some() {
            continue;
        }
        if m.role == "tool" {
            if let Some(id) = &m.tool_call_id {
                tool_results.insert(id.clone(), (m.content.clone().unwrap_or_default(), m.attachments.clone()));
            }
        }
    }

    let mut out = vec![json!({"role": "system", "content": system_prompt})];
    let mut est = estimate_tokens(system_prompt);

    // 如果存在已压缩的历史，将结构化备忘录作为上下文前置背景注入
    if max_compacted_seq > 0 && !compactions.is_empty() {
        let mut summaries = Vec::new();
        for c in &compactions {
            summaries.push(format!(
                "### 历史阶段备忘（第 {} ~ {} 条消息已压缩）\n{}",
                c.start_seq, c.end_seq, c.summary_markdown.trim()
            ));
        }
        let merged_summary = summaries.join("\n\n---\n\n");
        let memo_prompt = format!(
            "【系统历史对话结构化备忘录（以下为前文执行进展与关键信息归纳，已由用户确认并压缩替换早期消息）】:\n{}",
            merged_summary
        );
        est += estimate_tokens(&memo_prompt);
        out.push(json!({"role": "user", "content": memo_prompt}));
        let ack = "已掌握前文核心背景、技术约定及历史执行进展，我将在此基础上继续完成后续任务。";
        est += estimate_tokens(ack);
        out.push(json!({"role": "assistant", "content": ack}));
    }

    let last_user_seq = msgs
        .iter()
        .filter(|m| !m.queued && m.seq > max_compacted_seq && m.reverted_at.is_none() && m.role == "user")
        .map(|m| m.seq)
        .max()
        .unwrap_or(0);

    // 顺序消费未被压缩的消息（严格过滤已被软撤回的消息）
    for m in &msgs {
        if m.queued || m.seq <= max_compacted_seq || m.reverted_at.is_some() {
            continue;
        }
        match m.role.as_str() {
            "user" => {
                let mut text_content = m.content.clone().unwrap_or_default();

                // 挂载本消息产生时冻结的运行时环境附注（任务计划、最新知识索引）
                // 由于 runtime_anchor 在本轮写入后永久固化，未来轮次不再变动，完美保证历史前缀一致性
                if let Some(ref anchor) = m.runtime_anchor {
                    if !anchor.trim().is_empty() {
                        text_content.push_str("\n\n");
                        text_content.push_str(anchor.trim());
                    }
                }

                let mut image_parts: Vec<Value> = Vec::new();

                if let Some(ref atts) = m.attachments {
                    for att in atts {
                        if att.is_image {
                            if let Ok(bytes) = std::fs::read(&att.path) {
                                use base64::Engine;
                                let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                                let mime = if att.mime_type.is_empty() { "image/png" } else { &att.mime_type };
                                let data_url = format!("data:{};base64,{}", mime, b64);
                                image_parts.push(json!({
                                    "type": "image_url",
                                    "image_url": {
                                        "url": data_url
                                    }
                                }));
                                est += 1000;
                            }
                        } else {
                            let file_path = std::path::Path::new(&att.path);
                            if let Ok(meta) = std::fs::metadata(file_path) {
                                let size = meta.len();
                                if size < 50 * 1024 {
                                    if let Ok(bytes) = std::fs::read(file_path) {
                                        let is_binary = bytes.iter().take(1024).any(|&b| b == 0);
                                        if !is_binary {
                                            let text = String::from_utf8_lossy(&bytes);
                                            let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("txt");
                                            text_content.push_str(&format!(
                                                "\n\n### 📎 附件参考文件: {} (大小: {} 字节)\n```{ext}\n{}\n```",
                                                att.name, size, text
                                            ));
                                            est += estimate_tokens(&text);
                                            continue;
                                        }
                                    }
                                }
                                text_content.push_str(&format!(
                                    "\n\n### 📎 附件参考文件: {} (大小: {} 字节)\n本地物理路径: `{}`",
                                    att.name, size, att.path
                                ));
                            }
                        }
                    }
                }

                est += estimate_tokens(&text_content);

                if !image_parts.is_empty() {
                    let mut parts = vec![json!({"type": "text", "text": text_content})];
                    parts.extend(image_parts);
                    out.push(json!({"role": "user", "content": parts}));
                } else {
                    out.push(json!({"role": "user", "content": text_content}));
                }
            }
            "assistant" => {
                let mut obj = json!({"role": "assistant"});
                let content = m.content.clone().unwrap_or_default();
                if let Some(ref r) = m.reasoning {
                    if !r.trim().is_empty() {
                        obj["reasoning_content"] = Value::String(r.clone());
                        est += estimate_tokens(r);
                    }
                }
                match &m.tool_calls {
                    Some(tcs) if !tcs.is_null() && tcs.as_array().map(|a| !a.is_empty()).unwrap_or(false) => {
                        if !content.is_empty() {
                            obj["content"] = Value::String(content.clone());
                        } else {
                            obj["content"] = Value::Null;
                        }
                        obj["tool_calls"] = tcs.clone();
                        est += estimate_tokens(&content) + estimate_value_tokens(tcs);
                        out.push(obj);

                        // 紧随其后补齐每个调用的 tool 结果（缺失则合成，避免协议错误）
                        if let Some(arr) = tcs.as_array() {
                            for tc in arr {
                                let id = tc.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string();
                                let (text, atts) = tool_results
                                    .remove(&id)
                                    .unwrap_or_else(|| ("(执行被中断，无结果)".into(), None));

                                // ★ 核心准则：保持工具结果不可变！彻底废除 is_historical_turn 事后篡改逻辑 ★
                                // 工具输出在入库时已由 normalizer 规整，此处直接稳定消费，保护全前缀 KV Cache
                                est += estimate_tokens(&text);
                                let mut image_parts: Vec<Value> = Vec::new();

                                let is_active_turn = m.seq >= last_user_seq;
                                if is_active_turn {
                                    if let Some(ref att_list) = atts {
                                        for att in att_list {
                                            if att.is_image {
                                                if let Ok(bytes) = std::fs::read(&att.path) {
                                                    use base64::Engine;
                                                    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                                                    let mime = if att.mime_type.is_empty() { "image/png" } else { &att.mime_type };
                                                    let data_url = format!("data:{};base64,{}", mime, b64);
                                                    image_parts.push(json!({
                                                        "type": "image_url",
                                                        "image_url": {
                                                            "url": data_url
                                                        }
                                                    }));
                                                    est += 1000;
                                                }
                                            }
                                        }
                                    }
                                }

                                if !image_parts.is_empty() {
                                    let mut parts = vec![json!({"type": "text", "text": text})];
                                    parts.extend(image_parts);
                                    out.push(json!({"role": "tool", "tool_call_id": id, "content": parts}));
                                } else {
                                    out.push(json!({"role": "tool", "tool_call_id": id, "content": text}));
                                }
                            }
                        }
                    }
                    _ => {
                        if !content.is_empty() || obj.get("reasoning_content").is_some() {
                            obj["content"] = Value::String(content.clone());
                            est += estimate_tokens(&content);
                            out.push(obj);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // ---- 动态上下文感知工具输出瘦身与瞬态衰减 (Ephemeral Output Decay) ----
    // 核心准则：活跃轮次 (is_active) 保留 100% 完整工具结果以供精准决策；
    // 已完成决策的远期历史工具组（超过 2 轮前）实施就地衰减折叠，阻断长会话死日志堆积与梯形积分爆炸。
    let safe_threshold = token_limit * 80 / 100;
    {
        struct AssistantToolGroup {
            assistant_idx: usize,
            tool_indices: Vec<usize>,
            is_active: bool,
        }

        let mut groups: Vec<AssistantToolGroup> = Vec::new();
        let mut curr_group: Option<AssistantToolGroup> = None;

        for (idx, m) in out.iter().enumerate() {
            let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("");
            if role == "assistant" {
                if let Some(g) = curr_group.take() {
                    groups.push(g);
                }
                let has_tools = m
                    .get("tool_calls")
                    .and_then(|tc| tc.as_array())
                    .map(|a| !a.is_empty())
                    .unwrap_or(false);
                if has_tools {
                    curr_group = Some(AssistantToolGroup {
                        assistant_idx: idx,
                        tool_indices: Vec::new(),
                        is_active: false,
                    });
                }
            } else if role == "tool" {
                if let Some(ref mut g) = curr_group {
                    g.tool_indices.push(idx);
                }
            } else if role == "user" {
                if let Some(g) = curr_group.take() {
                    groups.push(g);
                }
            }
        }
        if let Some(g) = curr_group.take() {
            groups.push(g);
        }

        if let Some(last_g) = groups.last_mut() {
            if last_g.assistant_idx + last_g.tool_indices.len() + 1 == out.len() {
                last_g.is_active = true;
            }
        }

        let recent_cutoff = groups.len().saturating_sub(2);

        // 阶段 1：对远期历史工具组（> 2 轮前）实施就地衰减折叠（> 1000 字符收敛至 600；压力高时收敛至 350）
        for g in groups.iter().take(recent_cutoff) {
            for &t_idx in &g.tool_indices {
                let text = out[t_idx].get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let fold_limit = if est > safe_threshold { 350 } else { 1000 };
                if text.len() > fold_limit {
                    let safe_len = text.char_indices().nth(fold_limit).map(|(i, _)| i).unwrap_or(text.len());
                    let slimmed = format!(
                        "{}\n\n...[历史工具输出过长（共 {} 字符），已就地折叠衰减以保护活跃上下文空间]...",
                        &text[..safe_len],
                        text.len()
                    );
                    let old_tokens = estimate_tokens(&text);
                    let new_tokens = estimate_tokens(&slimmed);
                    est = est.saturating_sub(old_tokens).saturating_add(new_tokens);
                    out[t_idx]["content"] = Value::String(slimmed);
                }
            }
        }

        // 阶段 2：若折叠完远期历史后仍然超过 safe_threshold，允许对「近期但非活跃」工具输出折叠（绝对保护 is_active）
        if est > safe_threshold {
            for g in groups.iter().skip(recent_cutoff) {
                if g.is_active {
                    continue;
                }
                for &t_idx in &g.tool_indices {
                    if est <= safe_threshold {
                        break;
                    }
                    let text = out[t_idx].get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    if text.len() > 1200 {
                        let safe_len = text.char_indices().nth(1200).map(|(i, _)| i).unwrap_or(text.len());
                        let slimmed = format!(
                            "{}\n\n...[近期非活跃工具输出过长（共 {} 字符），已折叠前序详情以节省上下文空间]...",
                            &text[..safe_len],
                            text.len()
                        );
                        let old_tokens = estimate_tokens(&text);
                        let new_tokens = estimate_tokens(&slimmed);
                        est = est.saturating_sub(old_tokens).saturating_add(new_tokens);
                        out[t_idx]["content"] = Value::String(slimmed);
                    }
                }
                if est <= safe_threshold {
                    break;
                }
            }
        }
    }

    // ---- 轮次级原子截断防护（Turn-based Atomic Pruning） ----
    let mut notice: Option<TruncationNotice> = None;
    if est > token_limit {
        let est_before = est;
        let mut user_indices: Vec<usize> = out
            .iter()
            .enumerate()
            .filter(|(idx, m)| *idx > 0 && m.get("role").and_then(|r| r.as_str()) == Some("user"))
            .map(|(idx, _)| idx)
            .collect();

        let mut dropped_turns = 0usize;
        let mut dropped_messages = 0usize;
        let mut total_dropped_tokens = 0usize;
        let mut first_preview = String::new();

        while est > token_limit && user_indices.len() > 1 {
            let turn_start = user_indices[0];
            let turn_end = user_indices[1];
            if first_preview.is_empty() {
                let first_c = out[turn_start].get("content").and_then(|v| v.as_str()).unwrap_or("");
                first_preview = truncate_preview(first_c, 50);
            }
            let dropped_tokens: usize = out[turn_start..turn_end]
                .iter()
                .map(|m| {
                    let c = m.get("content").and_then(|v| v.as_str()).unwrap_or("");
                    let tc = m.get("tool_calls").map(estimate_value_tokens).unwrap_or(0);
                    estimate_tokens(c) + tc
                })
                .sum();
            dropped_turns += 1;
            dropped_messages += turn_end - turn_start;
            total_dropped_tokens += dropped_tokens;

            est = est.saturating_sub(dropped_tokens);
            out.drain(turn_start..turn_end);

            user_indices = out
                .iter()
                .enumerate()
                .filter(|(idx, m)| *idx > 0 && m.get("role").and_then(|r| r.as_str()) == Some("user"))
                .map(|(idx, _)| idx)
                .collect();
        }

        if dropped_turns > 0 {
            notice = Some(TruncationNotice {
                id: uuid::Uuid::new_v4().to_string(),
                session_id: session_id.to_string(),
                dropped_turns,
                dropped_messages,
                dropped_tokens: total_dropped_tokens,
                token_limit,
                est_tokens_before: est_before,
                est_tokens_after: est,
                first_preview,
                created_at: chrono::Utc::now().to_rfc3339(),
            });
        }
    }

    // 协议防御：末尾不能以 assistant 消息结束
    while let Some(last) = out.last() {
        if last.get("role").and_then(|r| r.as_str()) == Some("assistant") {
            out.pop();
        } else {
            break;
        }
    }

    (Ok(out), est, notice)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn test_prefix_immutability_across_turns() {
        let conn = Connection::open_in_memory().unwrap();
        store::init_schema(&conn).unwrap();
        let s = store::create_session(&conn, "D:\\workspace", None, "主会话", "confirm").unwrap();

        // Turn 1: User message with frozen runtime anchor
        let u1 = store::new_message(&conn, &s.id, "user", Some("第一轮请求：请阅读文件".into()), false).unwrap();
        store::update_message_runtime_anchor(&conn, &u1.id, "<runtime_environment_anchor>\nPlan: 0/3\n</runtime_environment_anchor>").unwrap();

        // Turn 1: Assistant calls tool
        let tc = json!([{"id": "call_1", "type": "function", "function": {"name": "read_file", "arguments": "{\"path\":\"a.txt\"}"}}]);
        let a1 = store::new_message(&conn, &s.id, "assistant", Some("我将读取文件".into()), false).unwrap();
        store::update_message_tool_calls(&conn, &a1.id, &tc, "", None).unwrap();

        // Turn 1: Tool message
        let m_tool = crate::models::Message {
            id: "t1".into(),
            session_id: s.id.clone(),
            run_id: None,
            seq: 3,
            role: "tool".into(),
            content: Some("文件内容: Hello World".into()),
            reasoning: None,
            tool_calls: None,
            tool_call_id: Some("call_1".into()),
            queued: false,
            usage: None,
            created_at: "2026-01-01T00:00:02Z".into(),
            tool_events: vec![],
            duration_ms: None,
            turn_duration_ms: None,
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
            cached_tokens: None,
            is_estimated: None,
            attachments: None,
            reverted_at: None,
            runtime_anchor: None,
        };
        store::insert_message(&conn, &m_tool).unwrap();

        // Turn 1: Assistant final response
        let _a2 = store::new_message(&conn, &s.id, "assistant", Some("这是读取结果：Hello World".into()), false).unwrap();

        let sys_prompt = "你是 harness_mini 纯静态系统基座。";
        let (msgs_turn1, est1, _) = assemble_execution_context(&conn, &s.id, sys_prompt, 32000);
        let msgs_turn1 = msgs_turn1.unwrap();

        // System + User1 + Assistant1(calls) + Tool1 + Assistant2(final)
        assert!(!msgs_turn1.is_empty());

        // Turn 2: User sends second request with updated plan in runtime_anchor
        let u2 = store::new_message(&conn, &s.id, "user", Some("第二轮请求：请修改文件".into()), false).unwrap();
        store::update_message_runtime_anchor(&conn, &u2.id, "<runtime_environment_anchor>\nPlan: 1/3\n</runtime_environment_anchor>").unwrap();

        let (msgs_turn2, est2, _) = assemble_execution_context(&conn, &s.id, sys_prompt, 32000);
        let msgs_turn2 = msgs_turn2.unwrap();

        // Prefix Verification:
        // msgs_turn2 must contain all messages of Turn 1 as its EXACT, BIT-IDENTICAL prefix!
        assert!(msgs_turn2.len() > msgs_turn1.len());
        for i in 0..msgs_turn1.len() {
            assert_eq!(
                msgs_turn2[i], msgs_turn1[i],
                "Prefix divergence detected at index {}: Turn 1 has {:?}, Turn 2 has {:?}",
                i, msgs_turn1[i], msgs_turn2[i]
            );
        }

        // Verify that Turn 1's tool message was NOT mutated or altered by Turn 2
        let tool_msg_in_turn2 = msgs_turn2.iter().find(|m| m.get("role").and_then(|r| r.as_str()) == Some("tool")).unwrap();
        assert_eq!(tool_msg_in_turn2["content"], "文件内容: Hello World");

        // Verify that Turn 2's User message has its own new anchor, but Turn 1's User message retained its original anchor
        let u1_rendered = msgs_turn2[1]["content"].as_str().unwrap();
        assert!(u1_rendered.contains("Plan: 0/3"));
        assert!(!u1_rendered.contains("Plan: 1/3"));

        let u2_rendered = msgs_turn2.last().unwrap()["content"].as_str().unwrap();
        assert!(u2_rendered.contains("Plan: 1/3"));

        assert!(est2 > est1);
    }

    #[test]
    fn test_empty_anchor_does_not_pollute_user_message() {
        let conn = Connection::open_in_memory().unwrap();
        store::init_schema(&conn).unwrap();
        let s = store::create_session(&conn, "D:\\workspace", None, "主会话", "confirm").unwrap();

        let u1 = store::new_message(&conn, &s.id, "user", Some("纯净消息".into()), false).unwrap();
        // runtime_anchor is None
        let (msgs, _, _) = assemble_execution_context(&conn, &s.id, "sys", 32000);
        let msgs = msgs.unwrap();
        assert_eq!(msgs[1]["content"], "纯净消息");

        // runtime_anchor is empty whitespace
        store::update_message_runtime_anchor(&conn, &u1.id, "   \n  ").unwrap();
        let (msgs2, _, _) = assemble_execution_context(&conn, &s.id, "sys", 32000);
        let msgs2 = msgs2.unwrap();
        assert_eq!(msgs2[1]["content"], "纯净消息");
    }
}
