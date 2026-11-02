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
                // 核心准则：历史 reasoning 仅用于 UI 展示与入库审计，发往模型的消息流严禁回传历史思维链反刍
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
                        let has_reasoning = m.reasoning.as_deref().map(|r| !r.trim().is_empty()).unwrap_or(false);
                        if !content.is_empty() || has_reasoning {
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

    // ---- 动态上下文感知工具输出语义分级滑动衰减 (Tiered Ephemeral Horizon) ----
    // 核心准则：
    // 1. 滑动保护窗放宽至最近 10 步 (Horizon = 10)，近 10 步内所有工具结果 100% 完整原貌；
    // 2. 超 10 步历史工具实施语义分级衰减：
    //    - 参考类工具 (read_file <= 5000 字符)：100% 永久保持原文，小文件免衰减；
    //    - 超大参考类工具 (read_file > 5000 字符)：温和收敛至 2,000 字符核心骨架（头 1200 + 尾 800）；
    //    - 瞬态工具 (run_command / grep / list_dir / glob / browser 等 > 2500 字符)：
    //      非对称折叠至 2,500 字符（头 500 + 尾 2000 报错栈底与退出诊断）；
    // 3. 活跃轮次 (is_active) 绝对保护不衰减。
    let safe_threshold = token_limit * 80 / 100;
    {
        struct ToolInvocation {
            index: usize,
            tool_name: String,
        }

        struct AssistantToolGroup {
            assistant_idx: usize,
            tools: Vec<ToolInvocation>,
            is_active: bool,
        }

        fn fold_tool_output(tool_name: &str, text: &str) -> Option<String> {
            if tool_name == "read_file" {
                if text.len() > 5000 {
                    let head_safe = text.char_indices().nth(1200).map(|(i, _)| i).unwrap_or(1200.min(text.len()));
                    let total_chars = text.chars().count();
                    let tail_start_char = total_chars.saturating_sub(800);
                    let tail_safe = text.char_indices().nth(tail_start_char).map(|(i, _)| i).unwrap_or(text.len().saturating_sub(800));
                    Some(format!(
                        "{}\n\n...[历史参考文件已温和收敛（原始共 {} 字符，保留骨架与核心签名），如需精读特定区域请调用 read_file 指定行号]...\n\n{}",
                        &text[..head_safe],
                        text.len(),
                        &text[tail_safe..]
                    ))
                } else {
                    None
                }
            } else {
                if text.len() > 2500 {
                    let head_safe = text.char_indices().nth(500).map(|(i, _)| i).unwrap_or(500.min(text.len()));
                    let total_chars = text.chars().count();
                    let tail_start_char = total_chars.saturating_sub(2000);
                    let tail_safe = text.char_indices().nth(tail_start_char).map(|(i, _)| i).unwrap_or(text.len().saturating_sub(2000));
                    Some(format!(
                        "{}\n\n...[历史工具输出过长（原始共 {} 字符，已折叠中间日志，保留首尾关键输出与退出诊断）]...\n\n{}",
                        &text[..head_safe],
                        text.len(),
                        &text[tail_safe..]
                    ))
                } else {
                    None
                }
            }
        }

        fn apply_decay(msg: &mut Value, tool_name: &str, est: &mut usize) {
            if let Some(content_str) = msg.get("content").and_then(|v| v.as_str()).map(|s| s.to_string()) {
                if let Some(slimmed) = fold_tool_output(tool_name, &content_str) {
                    let old_tokens = estimate_tokens(&content_str);
                    let new_tokens = estimate_tokens(&slimmed);
                    *est = est.saturating_sub(old_tokens).saturating_add(new_tokens);
                    msg["content"] = Value::String(slimmed);
                }
            } else if let Some(arr) = msg.get_mut("content").and_then(|v| v.as_array_mut()) {
                if let Some(first) = arr.first_mut() {
                    if first.get("type").and_then(|t| t.as_str()) == Some("text") {
                        if let Some(text) = first.get("text").and_then(|t| t.as_str()).map(|s| s.to_string()) {
                            if let Some(slimmed) = fold_tool_output(tool_name, &text) {
                                let old_tokens = estimate_tokens(&text);
                                let new_tokens = estimate_tokens(&slimmed);
                                *est = est.saturating_sub(old_tokens).saturating_add(new_tokens);
                                first["text"] = Value::String(slimmed);
                            }
                        }
                    }
                }
            }
        }

        let mut groups: Vec<AssistantToolGroup> = Vec::new();
        let mut curr_group: Option<AssistantToolGroup> = None;
        let mut id_to_name: HashMap<String, String> = HashMap::new();

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
                    if let Some(tcs) = m.get("tool_calls").and_then(|tc| tc.as_array()) {
                        for tc in tcs {
                            if let (Some(id), Some(name)) = (
                                tc.get("id").and_then(|v| v.as_str()),
                                tc.get("function").and_then(|f| f.get("name")).and_then(|v| v.as_str()),
                            ) {
                                id_to_name.insert(id.to_string(), name.to_string());
                            }
                        }
                    }
                    curr_group = Some(AssistantToolGroup {
                        assistant_idx: idx,
                        tools: Vec::new(),
                        is_active: false,
                    });
                }
            } else if role == "tool" {
                if let Some(ref mut g) = curr_group {
                    let call_id = m.get("tool_call_id").and_then(|v| v.as_str()).unwrap_or("");
                    let tool_name = id_to_name.get(call_id).cloned().unwrap_or_default();
                    g.tools.push(ToolInvocation {
                        index: idx,
                        tool_name,
                    });
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
            if last_g.assistant_idx + last_g.tools.len() + 1 == out.len() {
                last_g.is_active = true;
            }
        }

        let recent_cutoff = groups.len().saturating_sub(10);

        // 阶段 1：对超过最近 10 组前的远期历史工具组执行语义分级滑动衰减
        for g in groups.iter().take(recent_cutoff) {
            for item in &g.tools {
                apply_decay(&mut out[item.index], &item.tool_name, &mut est);
            }
        }

        // 阶段 2：若远期折叠后总 Token 仍然超出安全水位 (80%)，允许对近期非活跃瞬态工具实施同样折叠（保护 read_file 与 is_active）
        if est > safe_threshold {
            for g in groups.iter().skip(recent_cutoff) {
                if g.is_active {
                    continue;
                }
                for item in &g.tools {
                    if est <= safe_threshold {
                        break;
                    }
                    if item.tool_name != "read_file" {
                        apply_decay(&mut out[item.index], &item.tool_name, &mut est);
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
    fn test_tool_decay_10_step_horizon_and_tiered_rules() {
        let conn = Connection::open_in_memory().unwrap();
        store::init_schema(&conn).unwrap();
        let s = store::create_session(&conn, "D:\\workspace", None, "衰减测试", "confirm").unwrap();

        let _u1 = store::new_message(&conn, &s.id, "user", Some("开始探索".into()), false).unwrap();

        // 模拟 12 轮工具调用：
        // 第 1 步 (step 1, 最远期)：read_file 短文件 (3,000 字符，<= 5,000) -> 必须 100% 完整保留！
        // 第 2 步 (step 2, 最远期)：read_file 超大文件 (10,000 字符，> 5,000) -> 必须温和收敛至 2,000 字符骨架！
        // 第 3 步 (step 3, 最远期)：run_command 超长编译日志 (8,000 字符，> 2,500) -> 必须保留头部 500 字符 + 尾部 2,000 字符！
        // 第 4~12 步 (近 9 步)：各类工具 -> 位于 10 步滑窗保护期内，无论多大 100% 原文保留！

        let short_code = "fn small() {\n    println!(\"hello\");\n}\n".repeat(80); // ~3,000 字符
        let long_code = "pub fn large_module_feature_code_block() {\n    let x = 1;\n}\n".repeat(200); // ~10,000 字符
        let long_log = format!("cargo build --verbose\n{}\nerror[E0308]: mismatched types\nexpected `bool`, found `usize`\nFAIL: 1 test failed", "middle compile log line\n".repeat(300)); // ~8,000 字符

        // Step 1: read_file short
        let tc1 = json!([{"id": "call_1", "type": "function", "function": {"name": "read_file", "arguments": "{\"path\":\"short.rs\"}"}}]);
        let a1 = store::new_message(&conn, &s.id, "assistant", None, false).unwrap();
        store::update_message_tool_calls(&conn, &a1.id, &tc1, "", None).unwrap();
        let m_t1 = crate::models::Message {
            id: "t1".into(), session_id: s.id.clone(), run_id: None, seq: 3, role: "tool".into(),
            content: Some(short_code.clone()), reasoning: None, tool_calls: None, tool_call_id: Some("call_1".into()),
            queued: false, usage: None, created_at: "2026-01-01T00:00:01Z".into(), tool_events: vec![],
            duration_ms: None, turn_duration_ms: None, prompt_tokens: None, completion_tokens: None, total_tokens: None,
            cached_tokens: None, is_estimated: None, attachments: None, reverted_at: None, runtime_anchor: None,
        };
        store::insert_message(&conn, &m_t1).unwrap();

        // Step 2: read_file long
        let tc2 = json!([{"id": "call_2", "type": "function", "function": {"name": "read_file", "arguments": "{\"path\":\"long.rs\"}"}}]);
        let a2 = store::new_message(&conn, &s.id, "assistant", None, false).unwrap();
        store::update_message_tool_calls(&conn, &a2.id, &tc2, "", None).unwrap();
        let m_t2 = crate::models::Message {
            id: "t2".into(), session_id: s.id.clone(), run_id: None, seq: 5, role: "tool".into(),
            content: Some(long_code.clone()), reasoning: None, tool_calls: None, tool_call_id: Some("call_2".into()),
            queued: false, usage: None, created_at: "2026-01-01T00:00:02Z".into(), tool_events: vec![],
            duration_ms: None, turn_duration_ms: None, prompt_tokens: None, completion_tokens: None, total_tokens: None,
            cached_tokens: None, is_estimated: None, attachments: None, reverted_at: None, runtime_anchor: None,
        };
        store::insert_message(&conn, &m_t2).unwrap();

        // Step 3: run_command long log
        let tc3 = json!([{"id": "call_3", "type": "function", "function": {"name": "run_command", "arguments": "{\"command\":\"cargo test\"}"}}]);
        let a3 = store::new_message(&conn, &s.id, "assistant", None, false).unwrap();
        store::update_message_tool_calls(&conn, &a3.id, &tc3, "", None).unwrap();
        let m_t3 = crate::models::Message {
            id: "t3".into(), session_id: s.id.clone(), run_id: None, seq: 7, role: "tool".into(),
            content: Some(long_log.clone()), reasoning: None, tool_calls: None, tool_call_id: Some("call_3".into()),
            queued: false, usage: None, created_at: "2026-01-01T00:00:03Z".into(), tool_events: vec![],
            duration_ms: None, turn_duration_ms: None, prompt_tokens: None, completion_tokens: None, total_tokens: None,
            cached_tokens: None, is_estimated: None, attachments: None, reverted_at: None, runtime_anchor: None,
        };
        store::insert_message(&conn, &m_t3).unwrap();

        // Step 4 ~ 12: 追加 9 个工具组（使 Step 1~3 处于 10 步之前）
        for step in 4..=14 {
            let tc_id = format!("call_{}", step);
            let tc = json!([{"id": tc_id, "type": "function", "function": {"name": "list_dir", "arguments": "{}"}}]);
            let a = store::new_message(&conn, &s.id, "assistant", None, false).unwrap();
            store::update_message_tool_calls(&conn, &a.id, &tc, "", None).unwrap();
            let m_t = crate::models::Message {
                id: format!("t{}", step), session_id: s.id.clone(), run_id: None, seq: (step * 2) as i64, role: "tool".into(),
                content: Some(format!("dir output for step {}", step)), reasoning: None, tool_calls: None, tool_call_id: Some(tc_id),
                queued: false, usage: None, created_at: format!("2026-01-01T00:00:{:02}Z", step), tool_events: vec![],
                duration_ms: None, turn_duration_ms: None, prompt_tokens: None, completion_tokens: None, total_tokens: None,
                cached_tokens: None, is_estimated: None, attachments: None, reverted_at: None, runtime_anchor: None,
            };
            store::insert_message(&conn, &m_t).unwrap();
        }

        let (msgs, _, _) = assemble_execution_context(&conn, &s.id, "sys", 128000);
        let msgs = msgs.unwrap();

        // 验证 Step 1 (t1, read_file short): 必须 100% 完整保留！
        let t1_msg = msgs.iter().find(|m| m.get("tool_call_id").and_then(|v| v.as_str()) == Some("call_1")).unwrap();
        assert_eq!(t1_msg["content"].as_str().unwrap(), &short_code, "read_file <= 5000 字符必须 100% 免疫衰减");

        // 验证 Step 2 (t2, read_file long): 必须包含温和收敛标记，且保留首尾骨架
        let t2_msg = msgs.iter().find(|m| m.get("tool_call_id").and_then(|v| v.as_str()) == Some("call_2")).unwrap();
        let t2_content = t2_msg["content"].as_str().unwrap();
        assert!(t2_content.contains("历史参考文件已温和收敛"), "超大 read_file 必须温和收敛");
        assert!(t2_content.len() < long_code.len(), "超大文件必须被削减");
        assert!(t2_content.contains("pub fn large_module_feature_code_block"), "必须保留代码头部骨架");

        // 验证 Step 3 (t3, run_command long log): 必须非对称保留头部与尾部报错
        let t3_msg = msgs.iter().find(|m| m.get("tool_call_id").and_then(|v| v.as_str()) == Some("call_3")).unwrap();
        let t3_content = t3_msg["content"].as_str().unwrap();
        assert!(t3_content.contains("历史工具输出过长"), "超过 2500 字符的瞬态工具必须折叠");
        assert!(t3_content.contains("cargo build --verbose"), "必须保留头部 500 字符的执行入口");
        assert!(t3_content.contains("FAIL: 1 test failed"), "必须保留尾部 2000 字符的退出诊断与错误栈底");

        // 验证最近 10 步内的工具（如 call_11）：100% 完整无衰减
        let t13_msg = msgs.iter().find(|m| m.get("tool_call_id").and_then(|v| v.as_str()) == Some("call_13")).unwrap();
        assert_eq!(t13_msg["content"].as_str().unwrap(), "dir output for step 13");
    }

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
