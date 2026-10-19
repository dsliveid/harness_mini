use super::file::record_file_snapshot;
use tauri::{Emitter, Manager};
use super::helpers::*;
use super::types::*;
use serde_json::{json, Value};
use std::path::Path;

pub(crate) fn build_briefing_packet(
    db: &rusqlite::Connection,
    workspace: &Path,
    parent_id: &str,
    role: &str,
    title: &str,
    task: &str,
    args: &Value,
) -> String {
    let mut packet = format!("【任务协同委派简报】\n- 角色定位：{role}\n- 任务标题：{title}\n- 详细需求描述：\n{task}\n");

    // 1. 全局权威计划与进度切片
    if let Some((plan_path, meta, body)) = crate::plan::find_plan_file(workspace, parent_id, None, Some(db)) {
        let rel_path = plan_path.strip_prefix(workspace).map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| format!(".harness/plans/{}", plan_path.file_name().and_then(|s| s.to_str()).unwrap_or("plan.md")));
        let steps = crate::plan::parse_steps_from_markdown(&body);
        let done_count = steps.iter().filter(|s| s.status == "done").count();
        let in_progress_step = steps.iter().find(|s| s.status == "in_progress").map(|s| format!("第 {} 步: {}", s.index, s.content)).unwrap_or_else(|| "（推进中）".into());

        packet.push_str(&format!(
            "\n【项目权威计划与全局约束】\n- 计划文档路径：`{rel_path}` (版本: v{})\n- 计划全局总目标：{}\n- 全局推进状态：执行中 ({}/{})\n- 当前主线阶段：{}\n- 协同准则：本任务为上述权威计划的支撑环节。如需了解全局架构，使用 `read_file` 查阅该计划文件；严禁做出违背该计划的改动或推翻全局设计！\n",
            meta.version, meta.title, done_count, steps.len(), in_progress_step
        ));
    }

    // 2. 重点已知文件（显式传入 或 自动从父会话前序最近读写工具中提取）
    let mut known_files: Vec<String> = args.get("relevant_files")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|x| x.as_str().map(|s| s.trim().to_string())).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();

    if known_files.is_empty() {
        if let Ok(msgs) = crate::store::get_messages(db, parent_id, None, 10) {
            let mut seen = std::collections::BTreeSet::new();
            for m in msgs.iter().rev() {
                for te in &m.tool_events {
                    if ["read_file", "edit_file", "write_file", "file_outline"].contains(&te.tool_name.as_str()) {
                        if let Some(p) = te.params.get("path").and_then(|v| v.as_str()) {
                            if !seen.contains(p) {
                                seen.insert(p.to_string());
                                known_files.push(p.to_string());
                                if known_files.len() >= 5 {
                                    break;
                                }
                            }
                        }
                    }
                }
                if known_files.len() >= 5 {
                    break;
                }
            }
        }
    }

    if !known_files.is_empty() {
        packet.push_str("\n【主进程已知重点文件（请直接定位切入，免去盲目探索）】\n");
        for f in &known_files {
            packet.push_str(&format!("- `{f}`\n"));
        }
    }

    // 3. 上下文图钉 / 行号锚点 (pinned_context)
    if let Some(pinned) = args.get("pinned_context").and_then(|v| v.as_array()) {
        if !pinned.is_empty() {
            packet.push_str("\n【精确上下文图钉与行号锚点】\n");
            for item in pinned {
                if let Some(obj) = item.as_object() {
                    let p = obj.get("path").and_then(|v| v.as_str()).unwrap_or("");
                    let lines = obj.get("focus_lines").map(|v| v.to_string()).unwrap_or_default();
                    let intent = obj.get("intent").and_then(|v| v.as_str()).unwrap_or("");
                    packet.push_str(&format!("- 文件 `{p}` (关注行: {lines}): {intent}\n"));
                } else if let Some(s) = item.as_str() {
                    packet.push_str(&format!("- {s}\n"));
                }
            }
        }
    }

    // 4. 交付验收标准与严格约束
    if let Some(ac) = args.get("acceptance_criteria").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()) {
        packet.push_str(&format!("\n【交付验收标准与验证命令】\n{ac}\n"));
    }
    if let Some(c) = args.get("constraints").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()) {
        packet.push_str(&format!("\n【严格禁止项与负向约束】\n{c}\n"));
    }

    packet.push_str("\n【施工工人准则】：你作为实施工人，请优先对已知目标文件进行手术级精读（read_file 局部 30~50 行）并完成代码修改或验证；严禁脱离蓝图全库漫游！");
    packet.push_str("\n【分级验证提示】：查验数据请直接用只读命令；代码检查优先用轻量静态检查（如 TS 用 `tsc --noEmit`、Rust 用 `cargo check`、Go 用 `go vet`）；单测需指定精准目标；严禁修改生产源码当作临时探针。");
    packet
}

pub(crate) fn save_subtask_report_artifact(
    workspace: &Path,
    subagent_id: &str,
    title: &str,
    role: &str,
    full_content: &str,
    touched_files: &[String],
) -> Result<String, String> {
    if workspace.as_os_str().is_empty() {
        return Err("工作区为空".into());
    }
    let subtasks_dir = workspace.join(".harness").join("subtasks");
    let _ = std::fs::create_dir_all(&subtasks_dir);
    let filename = format!("{}_report.md", subagent_id.replace('-', "_"));
    let file_path = subtasks_dir.join(&filename);

    let files_section = if touched_files.is_empty() {
        "无".to_string()
    } else {
        touched_files.iter().map(|f| format!("- `{f}`")).collect::<Vec<_>>().join("\n")
    };

    let report_md = format!(
        "# 协同任务交付报告: {title}\n\n- **子任务 ID**: `{subagent_id}`\n- **角色定位**: {role}\n- **生成时间**: {}\n- **涉及改动文件**:\n{files_section}\n\n---\n\n## 详细报告与产出内容\n\n{full_content}\n",
        chrono::Utc::now().to_rfc3339()
    );

    let _ = std::fs::write(&file_path, report_md).map_err(|e| format!("保存工件失败: {e}"))?;
    Ok(format!(".harness/subtasks/{}", filename))
}

pub(crate) fn extract_compact_summary(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut count = 0;
    let mut end_idx = trimmed.len();
    for (i, _) in trimmed.char_indices() {
        if count >= max_chars {
            end_idx = i;
            break;
        }
        count += 1;
    }
    format!("{}...\n\n*(注：内容已自动压缩提炼为决策摘要，完整报告细节请查阅上方工件文件)*", &trimmed[..end_idx])
}

// ---------- 子 Agent 协作工具具体实现 ----------

pub(crate) async fn spawn_subagent_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let host = ctx.host.as_ref().ok_or("内部错误：缺少宿主上下文")?;
    let role = args.get("role").and_then(|v| v.as_str()).ok_or("缺少 role 参数")?;
    let title = args.get("title").and_then(|v| v.as_str()).ok_or("缺少 title 参数")?;
    let task = args.get("task").and_then(|v| v.as_str()).ok_or("缺少 task 参数")?;

    let state = host.app.state::<crate::AppState>();
    let parent_id = &host.session_id;

    let (parent_session, current_subs_count) = {
        let db = state.db.lock().unwrap();
        let p = crate::store::get_session(&db, parent_id)?.ok_or("父会话不存在")?;
        // 校验递归深度：子 Agent 会话禁止再次创建子 Agent
        if p.session_type == "subagent" || p.parent_session_id.is_some() {
            return Err("子 Agent 不允许递归创建新的子 Agent，请直接由当前子 Agent 完成指定任务。".into());
        }
        let subs = crate::store::list_subagents(&db, parent_id)?;
        (p, subs.len())
    };

    if current_subs_count >= 10 {
        return Err("子 Agent 数量已达上限 (10)，请等待部分子任务完成或停止后再创建。".into());
    }

    // 子 Agent 工作区严格与父会话保持完全一致
    let sub_workspace = parent_session.workspace_path.clone();

    let (sub, user_msg) = {
        let db = state.db.lock().unwrap();
        let sub = crate::store::create_subagent_session(
            &db,
            parent_id,
            role,
            title,
            task,
            &sub_workspace,
            parent_session.access_mode.as_deref(),
            parent_session.project_id.as_deref(),
            ctx.event_id.as_deref(),
        )?;
        // 关键强绑定：立即将生成的真实 sub.id 回填并持久化至当前 tool_event
        if let Some(ref ev_id) = ctx.event_id {
            let mut params_with_id = args.clone();
            if let Some(obj) = params_with_id.as_object_mut() {
                obj.insert("subprocess_id".into(), json!(sub.id));
            }
            let _ = crate::store::set_tool_event_subprocess_id(
                &db,
                ev_id,
                &sub.id,
                Some(&params_with_id.to_string()),
            );
        }
        let initial_prompt = build_briefing_packet(&db, &ctx.workspace, parent_id, role, title, task, args);
        let user_msg = crate::store::new_message(&db, &sub.id, "user", Some(initial_prompt), false)?;
        (sub, user_msg)
    };

    // 启动子 Agent 异步运行循环
    crate::agent::spawn_session_task(host.app.clone(), sub.id.clone(), Some(user_msg.id));

    // 广播事件通知前端刷新子 Agent / 子进程列表（附带 toolEventId 与 subprocess 强关联）
    let _ = host.app.emit("subprocess:created", json!({
        "parentId": parent_id,
        "subprocess": sub,
        "toolEventId": ctx.event_id,
    }));
    let _ = host.app.emit("subprocesses:changed", json!({
        "parentId": parent_id,
    }));
    let _ = host.app.emit("subagent:created", json!({
        "parentId": parent_id,
        "subagent": sub,
        "toolEventId": ctx.event_id,
    }));
    let _ = host.app.emit("subagents:changed", json!({
        "parentId": parent_id,
    }));

    Ok(format!(
        "已成功创建并启动子 Agent 进程！\n- ID: `{}`\n- 角色: {}\n- 标题: {}\n- 状态: 运行中 (running)\n\n子 Agent 正在独立上下文中执行，用户点击界面右侧可实时查看其完整对话流程与工具卡片。后续可通过 `wait_subagents` 或 `get_subagent_status` 协同跟进。",
        sub.id, role, title
    ))
}

pub(crate) fn determine_subagent_status(
    is_running: bool,
    session_status: &str,
    has_terminal_reply: bool,
    has_max_steps_msg: bool,
) -> (&'static str, &'static str) {
    if is_running {
        ("running", "🟡 仍在运行 (running)")
    } else if session_status == "cancelled" {
        ("cancelled", "⚪ 已取消/停止 (cancelled)")
    } else if session_status == "failed" {
        ("failed", "🔴 运行失败出错 (failed - 可调用 resume_subprocess 恢复)")
    } else if session_status == "interrupted" {
        ("interrupted", "🟠 执行中断未完成 (interrupted - 可调用 resume_subprocess 恢复)")
    } else if has_max_steps_msg {
        ("interrupted", "🟠 已达最大步数上限中止 (max_steps_reached - 可调用 resume_subprocess 继续推进)")
    } else if !has_terminal_reply {
        ("interrupted", "🟠 执行中断未完全收敛 (interrupted - 可调用 resume_subprocess 恢复)")
    } else {
        ("completed", "🟢 已完成交付 (completed)")
    }
}

pub(crate) async fn get_subagent_status_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let host = ctx.host.as_ref().ok_or("内部错误：缺少宿主上下文")?;
    let target_id = args.get("subagent_id")
        .or_else(|| args.get("subprocess_id"))
        .and_then(|v| v.as_str());
    let state = host.app.state::<crate::AppState>();
    let parent_id = &host.session_id;

    let subs = {
        let db = state.db.lock().unwrap();
        crate::store::list_subagents(&db, parent_id)?
    };

    if subs.is_empty() {
        return Ok("当前会话尚未创建任何子 Agent 进程。".into());
    }

    let filtered: Vec<&crate::models::Session> = if let Some(tid) = target_id {
        subs.iter().filter(|s| s.id == tid).collect()
    } else {
        subs.iter().collect()
    };

    if filtered.is_empty() {
        return Ok(format!("未找到指定 ID 的子 Agent: {}", target_id.unwrap_or_default()));
    }

    let mut out = format!("共查询到 {} 个子 Agent 状态：\n\n", filtered.len());
    let db = state.db.lock().unwrap();
    for s in filtered {
        let is_running = crate::agent::is_run_active(&state, &s.id);
        let msgs = crate::store::get_messages(&db, &s.id, None, 50).unwrap_or_default();
        let last_reply = msgs
            .iter()
            .rev()
            .find(|m| m.role == "assistant" && !m.content.as_deref().unwrap_or("").is_empty())
            .and_then(|m| m.content.as_deref())
            .unwrap_or("(尚在准备或执行工具中)");
        let last_asst = msgs.iter().rev().find(|m| m.role == "assistant");
        let has_terminal_reply = last_asst.map(|m| {
            m.tool_calls.is_none()
                || m.tool_calls.as_ref().map(|t| t.is_null() || t.as_array().map(|a| a.is_empty()).unwrap_or(false)).unwrap_or(false)
        }).unwrap_or(false);
        let has_max_steps_msg = msgs.iter().rev().any(|m| m.role == "system" && m.content.as_deref().unwrap_or("").contains("已达到最大步数"));

        let (_code, status_display) = determine_subagent_status(
            is_running,
            &s.status,
            has_terminal_reply,
            has_max_steps_msg,
        );

        let preview = if last_reply.len() > 300 {
            format!("{}...", &last_reply[..last_reply.char_indices().nth(300).map(|(i,_)| i).unwrap_or(last_reply.len())])
        } else {
            last_reply.to_string()
        };

        out.push_str(&format!(
            "- **【{}】** (ID: `{}`)\n  角色: {}\n  状态: {}\n  Token消耗: {}\n  最新输出摘要: {}\n\n",
            s.title,
            s.id,
            s.subagent_role.as_deref().unwrap_or("协作助手"),
            status_display,
            s.total_tokens.unwrap_or(0),
            preview
        ));
    }

    Ok(out)
}

pub(crate) async fn wait_subagents_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let host = ctx.host.as_ref().ok_or("内部错误：缺少宿主上下文")?;
    let timeout_secs = get_u64_arg(args, "timeout_seconds", 60).clamp(5, 900);
    let specified_ids: Option<Vec<String>> = args.get("subagent_ids")
        .or_else(|| args.get("subprocess_ids"))
        .and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()
            })
        });

    let state = host.app.state::<crate::AppState>();
    let parent_id = &host.session_id;

    let target_ids: Vec<String> = {
        let db = state.db.lock().unwrap();
        let subs = crate::store::list_subagents(&db, parent_id)?;
        if let Some(ids) = specified_ids {
            subs.into_iter().filter(|s| ids.contains(&s.id)).map(|s| s.id).collect()
        } else {
            subs.into_iter().map(|s| s.id).collect()
        }
    };

    if target_ids.is_empty() {
        return Ok("没有找到需要等待的子 Agent。".into());
    }

    let start_wait = std::time::Instant::now();
    let max_wait = std::time::Duration::from_secs(timeout_secs);
    let mut auto_recovered = std::collections::HashSet::new();

    loop {
        let mut any_running = false;
        for id in &target_ids {
            let is_running = crate::agent::is_run_active(&state, id);
            if is_running {
                any_running = true;
            } else if !auto_recovered.contains(id) {
                // 检查是否是非正常中断停止（未交付且未显式取消），若发生意外中断且尚有等待时间，自动触发一次自愈恢复
                let should_recover = {
                    let db = state.db.lock().unwrap();
                    if let Ok(Some(s)) = crate::store::get_session(&db, id) {
                        if s.status == "interrupted" || s.status == "failed" {
                            true
                        } else if s.status != "cancelled" {
                            let msgs = crate::store::get_messages(&db, id, None, 10).unwrap_or_default();
                            let last_asst = msgs.iter().rev().find(|m| m.role == "assistant");
                            let has_terminal_reply = last_asst.map(|m| {
                                m.tool_calls.is_none()
                                    || m.tool_calls.as_ref().map(|t| t.is_null() || t.as_array().map(|a| a.is_empty()).unwrap_or(false)).unwrap_or(false)
                            }).unwrap_or(false);
                            !has_terminal_reply
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                };

                if should_recover && start_wait.elapsed() < max_wait {
                    auto_recovered.insert(id.clone());
                    let _ = crate::agent::restart_subagent(&host.app, id);
                    any_running = true;
                }
            }
        }

        if !any_running || start_wait.elapsed() >= max_wait {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    let mut out = String::new();
    let is_timed_out = start_wait.elapsed() >= max_wait;

    let (sub_results, any_still_running, any_interrupted_or_failed, memory_effects) = {
        let db = state.db.lock().unwrap();
        let mut any_interrupted_or_failed = false;
        let mut any_still_running = false;
        let mut sub_results = Vec::new();
        let mut memory_effects = Vec::new();

        for id in &target_ids {
            let is_running = crate::agent::is_run_active(&state, id);
            if is_running {
                any_still_running = true;
            }
            let s = crate::store::get_session(&db, id)?.unwrap_or_else(|| {
                crate::models::Session {
                    id: id.clone(),
                    title: "未知子Agent".into(),
                    parent_session_id: Some(parent_id.clone()),
                    session_type: "subagent".into(),
                    ..Default::default()
                }
            });
            let msgs = crate::store::get_messages(&db, id, None, 50).unwrap_or_default();
            let last_reply = msgs
                .iter()
                .rev()
                .find(|m| m.role == "assistant" && !m.content.as_deref().unwrap_or("").is_empty())
                .and_then(|m| m.content.as_deref())
                .unwrap_or("(未产生文本回复)");

            let last_asst = msgs.iter().rev().find(|m| m.role == "assistant");
            let has_terminal_reply = last_asst.map(|m| {
                m.tool_calls.is_none()
                    || m.tool_calls.as_ref().map(|t| t.is_null() || t.as_array().map(|a| a.is_empty()).unwrap_or(false)).unwrap_or(false)
            }).unwrap_or(false);
            let has_max_steps_msg = msgs.iter().rev().any(|m| m.role == "system" && m.content.as_deref().unwrap_or("").contains("已达到最大步数"));

            let (status_code, status_display) = determine_subagent_status(
                is_running,
                &s.status,
                has_terminal_reply,
                has_max_steps_msg,
            );
            if status_code == "interrupted" || status_code == "failed" {
                any_interrupted_or_failed = true;
            }

            let display_reply = if has_max_steps_msg {
                format!("⚠️ 该子任务已达到最大步数上限中止，未生成最终交付报告。最后思考或动作：\n{}", last_reply)
            } else if status_code == "interrupted" && last_reply != "(未产生文本回复)" {
                format!("⚠️ 该子任务执行中断或未完全收敛交付。最后思考或动作：\n{}", last_reply)
            } else {
                last_reply.to_string()
            };

            let mut touched_files = std::collections::BTreeSet::new();
            for m in &msgs {
                for te in &m.tool_events {
                    if ["write_file", "edit_file", "apply_diff"].contains(&te.tool_name.as_str()) {
                        if let Some(p) = te.params.get("path").and_then(|v| v.as_str()) {
                            touched_files.insert(p.to_string());
                        }
                    }
                }
            }
            let touched_vec: Vec<String> = touched_files.into_iter().collect();
            let touched_line = if touched_vec.is_empty() {
                String::new()
            } else {
                let list = touched_vec.iter().map(|f| format!("`{f}`")).collect::<Vec<_>>().join(", ");
                format!("- 涉及改动文件: {}\n", list)
            };

            // 自动沉淀技术分析类子任务成果至项目技术大盘 (profile.md)
            let is_tech_analysis = {
                let role_str = s.subagent_role.as_deref().unwrap_or("");
                let title_str = s.title.as_str();
                let task_str = s.subagent_task.as_deref().unwrap_or("");
                role_str.contains("技术栈") || role_str.contains("架构")
                    || title_str.contains("技术栈") || title_str.contains("架构")
                    || task_str.contains("技术栈") || task_str.contains("依赖分析")
            };
            if is_tech_analysis && !is_running && has_terminal_reply && last_reply.len() > 50 && last_reply != "(未产生文本回复)" {
                let already_recorded = msgs.iter().any(|m| {
                    m.tool_events.iter().any(|te| te.tool_name == "record_memory")
                });
                if !already_recorded && !ctx.workspace.as_os_str().is_empty() {
                    if let Ok(effect) = crate::memory::record_memory_with_effect(
                        &ctx.workspace,
                        "profile",
                        &s.title,
                        last_reply,
                    ) {
                        memory_effects.push(effect);
                    }
                }
            }

            // 双轨交付：将长文本落地到工件文件 .harness/subtasks/{subagent_id}_report.md
            let artifact_path_opt = if !ctx.workspace.as_os_str().is_empty() && (display_reply.len() > 250 || display_reply.contains("```") || !touched_vec.is_empty()) {
                save_subtask_report_artifact(
                    &ctx.workspace,
                    id,
                    &s.title,
                    s.subagent_role.as_deref().unwrap_or("协作助手"),
                    &display_reply,
                    &touched_vec,
                ).ok()
            } else {
                None
            };

            let compact_summary = extract_compact_summary(&display_reply, 350);
            let artifact_line = if let Some(ref ap) = artifact_path_opt {
                format!("- 📄 完整技术报告工件: [查看完整详细报告](file:///{})\n", ap.replace('\\', "/"))
            } else {
                String::new()
            };

            sub_results.push(format!(
                "### 协同子 Agent: {} ({})\n- 状态: {}\n- Token 消耗: {}\n{}{}- 交付核心结论：\n```markdown\n{}\n```\n\n",
                s.title,
                s.subagent_role.as_deref().unwrap_or("协作助手"),
                status_display,
                s.total_tokens.unwrap_or(0),
                touched_line,
                artifact_line,
                compact_summary
            ));
        }
        (sub_results, any_still_running, any_interrupted_or_failed, memory_effects)
    };

    for effect in memory_effects {
        record_file_snapshot(ctx, &effect.rel_path, effect.old_bytes.as_deref(), &effect.new_bytes).await;
    }

    if is_timed_out || any_still_running {
        out.push_str(&format!("⚠️ 等待达到超时上限（{timeout_secs}s），部分子 Agent 可能仍在后台继续运行。\n\n"));
    } else if any_interrupted_or_failed {
        out.push_str("⚠️ 部分子 Agent 执行中断或未完成交付（可在下一步调用 resume_subprocess 继续恢复推进）：\n\n");
    } else {
        out.push_str("✅ 所有目标子 Agent 执行完毕！汇总结果如下：\n\n");
    }

    for sr in sub_results {
        out.push_str(&sr);
    }

    Ok(out)
}

pub(crate) async fn stop_subagent_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let host = ctx.host.as_ref().ok_or("内部错误：缺少宿主上下文")?;
    let subagent_id = args.get("subagent_id").and_then(|v| v.as_str()).ok_or("缺少 subagent_id 参数")?;
    {
        let state = host.app.state::<crate::AppState>();
        let db = state.db.lock().unwrap();
        let _ = crate::store::set_session_status(&db, subagent_id, "cancelled");
    }
    crate::agent::stop_session(&host.app, subagent_id);
    let _ = host.app.emit("subagent:update", json!({
        "parentId": host.session_id,
        "subagentId": subagent_id,
        "status": "cancelled"
    }));
    let _ = host.app.emit("subprocess:update", json!({
        "parentId": host.session_id,
        "parentSessionId": host.session_id,
        "subprocessId": subagent_id,
        "status": "cancelled"
    }));
    let _ = host.app.emit("subagents:changed", json!({
        "parentId": host.session_id
    }));
    let _ = host.app.emit("subprocesses:changed", json!({
        "parentId": host.session_id
    }));
    Ok(format!("已成功停止子 Agent【{subagent_id}】。"))
}

pub(crate) async fn resume_subagent_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let host = ctx.host.as_ref().ok_or("内部错误：缺少宿主上下文")?;
    let subagent_id = args.get("subagent_id")
        .or_else(|| args.get("subprocess_id"))
        .and_then(|v| v.as_str())
        .ok_or("缺少 subagent_id 或 subprocess_id 参数")?;

    let state = host.app.state::<crate::AppState>();
    let parent_id = &host.session_id;

    // 校验子 Agent 是否存在且属于当前主会话
    {
        let db = state.db.lock().unwrap();
        let s = crate::store::get_session(&db, subagent_id)?.ok_or("未找到指定的子 Agent")?;
        if s.parent_session_id.as_deref() != Some(parent_id) {
            return Err("指定的子 Agent 不属于当前主会话".into());
        }
    }

    crate::agent::restart_subagent(&host.app, subagent_id)?;

    Ok(format!(
        "已成功恢复并重新启动子 Agent【{}】。它正在后台继续推进执行，你可以调用 wait_subagents 工具等待其最新执行成果。",
        subagent_id
    ))
}

pub(crate) async fn dispatch_collaborator_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let host = ctx.host.as_ref().ok_or("内部错误：缺少宿主上下文")?;
    let collaborator_id = args.get("collaborator_id").and_then(|v| v.as_str()).ok_or("缺少 collaborator_id 参数")?;
    let task = args.get("task").and_then(|v| v.as_str()).ok_or("缺少 task 参数")?;

    let state = host.app.state::<crate::AppState>();
    let parent_id = &host.session_id;

    let collab = {
        let db = state.db.lock().unwrap();
        let s = crate::store::get_session(&db, collaborator_id)?.ok_or("未找到指定的协作者")?;
        if s.parent_session_id.as_deref() != Some(parent_id) {
            return Err("指定的协作者不属于当前主会话".into());
        }
        s
    };

    if crate::agent::is_run_active(&state, collaborator_id) {
        return Err(format!("协作者【{}】当前正在运行中，请先调用 wait_collaborators 等待其完成，再指派新任务。", collab.title));
    }

    let user_prompt = {
        let db = state.db.lock().unwrap();
        build_briefing_packet(
            &db,
            &ctx.workspace,
            parent_id,
            collab.subagent_role.as_deref().unwrap_or("协作者"),
            &collab.title,
            task,
            args,
        )
    };
    let user_msg = {
        let db = state.db.lock().unwrap();
        let _ = crate::store::set_kv(&db, collaborator_id, "dispatched_by_parent", "true");
        crate::store::new_message(&db, collaborator_id, "user", Some(user_prompt), false)?
    };

    crate::agent::spawn_session_task(host.app.clone(), collaborator_id.to_string(), Some(user_msg.id));

    Ok(format!(
        "已成功向协作者【{}】(角色: {}) 委派任务！\n- 任务要求: {}\n- 状态: 运行中 (running)\n\n协作者已开始独立执行。你可以紧接着调用 wait_collaborators 等待并获取其增量产出报告。",
        collab.title,
        collab.subagent_role.as_deref().unwrap_or("协作者"),
        task
    ))
}

pub(crate) async fn wait_collaborators_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let host = ctx.host.as_ref().ok_or("内部错误：缺少宿主上下文")?;
    let timeout_secs = get_u64_arg(args, "timeout_seconds", 90).clamp(5, 300);
    let specified_ids: Option<Vec<String>> = args.get("collaborator_ids").and_then(|v| {
        v.as_array().map(|arr| {
            arr.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()
        })
    });

    let state = host.app.state::<crate::AppState>();
    let parent_id = &host.session_id;

    let target_ids: Vec<String> = {
        let db = state.db.lock().unwrap();
        let collabs = crate::store::list_collaborators(&db, parent_id)?;
        if let Some(ids) = specified_ids {
            collabs.into_iter().filter(|s| ids.contains(&s.id)).map(|s| s.id).collect()
        } else {
            collabs.into_iter().map(|s| s.id).collect()
        }
    };

    if target_ids.is_empty() {
        return Ok("当前项目暂无需要等待的协作者。".into());
    }

    let start_wait = std::time::Instant::now();
    let max_wait = std::time::Duration::from_secs(timeout_secs);

    loop {
        let any_running = target_ids.iter().any(|id| crate::agent::is_run_active(&state, id));
        if !any_running || start_wait.elapsed() >= max_wait {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    let mut out = String::new();
    let is_timed_out = start_wait.elapsed() >= max_wait;
    if is_timed_out {
        out.push_str(&format!("⚠️ 等待达到超时上限（{timeout_secs}s），部分协作者可能仍在后台运行。\n\n"));
    } else {
        out.push_str("✅ 目标协作者本轮执行完毕！汇总增量成果如下：\n\n");
    }

    let db = state.db.lock().unwrap();
    for id in &target_ids {
        let is_running = crate::agent::is_run_active(&state, id);
        let s = match crate::store::get_session(&db, id)? {
            Some(s) => s,
            None => continue,
        };
        let msgs = crate::store::get_messages(&db, id, None, 50).unwrap_or_default();

        let slice: Vec<&crate::models::Message> = if let Some(ref w_id) = s.last_reported_msg_id {
            if let Some(pos) = msgs.iter().position(|m| &m.id == w_id) {
                msgs[pos + 1..].iter().collect()
            } else {
                msgs.iter().collect()
            }
        } else {
            msgs.iter().collect()
        };

        let last_assistant = slice
            .iter()
            .rev()
            .find(|m| m.role == "assistant" && !m.content.as_deref().unwrap_or("").is_empty())
            .copied()
            .or_else(|| {
                // 兜底保护：若水位线之后无增量匹配，取该协作者全量消息中最新一条有效回复
                msgs.iter().rev().find(|m| m.role == "assistant" && !m.content.as_deref().unwrap_or("").is_empty())
            });

        let last_reply = match last_assistant {
            Some(m) => m.content.as_deref().unwrap_or("(未产生文本回复)"),
            None => "(本轮暂无新增文本产出)",
        };

        let mut touched_files = std::collections::BTreeSet::new();
        let files_source: Vec<&crate::models::Message> = if !slice.is_empty() {
            slice
        } else {
            msgs.iter().collect()
        };
        for m in &files_source {
            for te in &m.tool_events {
                if ["write_file", "edit_file", "apply_diff"].contains(&te.tool_name.as_str()) {
                    if let Some(p) = te.params.get("path").and_then(|v| v.as_str()) {
                        touched_files.insert(p.to_string());
                    }
                }
            }
        }
        let touched_vec: Vec<String> = touched_files.into_iter().collect();
        let touched_line = if touched_vec.is_empty() {
            String::new()
        } else {
            let list = touched_vec.iter().map(|f| format!("`{f}`")).collect::<Vec<_>>().join(", ");
            format!("- 涉及改动文件: {}\n", list)
        };

        // 双轨交付：将长文本落地到工件文件 .harness/subtasks/{collaborator_id}_report.md
        let artifact_path_opt = if !ctx.workspace.as_os_str().is_empty() && (last_reply.len() > 250 || last_reply.contains("```") || !touched_vec.is_empty()) {
            save_subtask_report_artifact(
                &ctx.workspace,
                id,
                &s.title,
                s.subagent_role.as_deref().unwrap_or("协作者"),
                last_reply,
                &touched_vec,
            ).ok()
        } else {
            None
        };

        let compact_summary = extract_compact_summary(last_reply, 350);
        let artifact_line = if let Some(ref ap) = artifact_path_opt {
            format!("- 📄 完整技术报告工件: [查看完整详细报告](file:///{})\n", ap.replace('\\', "/"))
        } else {
            String::new()
        };

        let last_asst = msgs.iter().rev().find(|m| m.role == "assistant");
        let has_terminal_reply = last_asst.map(|m| {
            m.tool_calls.is_none()
                || m.tool_calls.as_ref().map(|t| t.is_null() || t.as_array().map(|a| a.is_empty()).unwrap_or(false)).unwrap_or(false)
        }).unwrap_or(false);
        let has_max_steps_msg = msgs.iter().rev().any(|m| m.role == "system" && m.content.as_deref().unwrap_or("").contains("已达到最大步数"));
        let (_code, status_display) = determine_subagent_status(
            is_running,
            &s.status,
            has_terminal_reply,
            has_max_steps_msg,
        );

        if let Some(m) = last_assistant {
            let _ = crate::store::update_collaborator_watermark(&db, id, &m.id);
        }

        out.push_str(&format!(
            "### 协作者: {} ({})\n- 状态: {}\n- Token 消耗: {}\n{}{}- 增量交付结论：\n```markdown\n{}\n```\n\n",
            s.title,
            s.subagent_role.as_deref().unwrap_or("协作者"),
            status_display,
            s.total_tokens.unwrap_or(0),
            touched_line,
            artifact_line,
            compact_summary
        ));
    }

    Ok(out)
}

pub(crate) async fn get_collaborators_tool(ctx: &ToolCtx) -> Result<String, String> {
    let host = ctx.host.as_ref().ok_or("内部错误：缺少宿主上下文")?;
    let state = host.app.state::<crate::AppState>();
    let parent_id = &host.session_id;

    let collabs = {
        let db = state.db.lock().unwrap();
        crate::store::list_collaborators(&db, parent_id)?
    };

    if collabs.is_empty() {
        return Ok("当前主会话尚未配置任何常驻协作者。如需并行处理任务，可由用户在界面顶部创建协作者，或由你调用 spawn_subprocess 派生临时子进程。".into());
    }

    let mut out = format!("当前已配置 {} 位项目协作者：\n\n", collabs.len());
    let db = state.db.lock().unwrap();
    for c in collabs {
        let is_running = crate::agent::is_run_active(&state, &c.id);
        let msgs = crate::store::get_messages(&db, &c.id, None, 10).unwrap_or_default();
        let last_reply = msgs
            .iter()
            .rev()
            .find(|m| m.role == "assistant" && !m.content.as_deref().unwrap_or("").is_empty())
            .and_then(|m| m.content.as_deref())
            .unwrap_or("(就绪待命)");
        let preview = if last_reply.len() > 200 {
            format!("{}...", &last_reply[..last_reply.char_indices().nth(200).map(|(i,_)| i).unwrap_or(last_reply.len())])
        } else {
            last_reply.to_string()
        };

        out.push_str(&format!(
            "- **【{}】** (ID: `{}`)\n  角色: {}\n  状态: {}\n  初始职责: {}\n  最新进展: {}\n\n",
            c.title,
            c.id,
            c.subagent_role.as_deref().unwrap_or("协作者"),
            if is_running { "🟡 运行中 (busy)" } else { "🟢 空闲中 (idle)" },
            c.subagent_task.as_deref().unwrap_or("负责该领域工作"),
            preview
        ));
    }

    Ok(out)
}

