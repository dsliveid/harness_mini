use crate::agent;
use crate::models::{LongTask, TaskCheckpoint, TaskSubItem, ToolEvent};
use crate::store;
use crate::task::memory::build_working_memory_block;
use crate::task::planner::{decompose_goal, replan_subtasks};
use crate::task::verifier::{create_git_snapshot, execute_verify_command};
use crate::task::{get_next_subtask_index, remove_control};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{watch, Semaphore};

/// 默认最大子任务并发度（防止瞬间打满 API Rate Limit 或过载本地 CPU）
const DEFAULT_MAX_CONCURRENCY: usize = 3;

/// 单个子任务执行结果
struct SubtaskExecutionResult {
    subtask_idx: usize,
    sub_id: String,
    status: String, // "completed" | "failed" | "interrupted"
    summary: String,
    error: Option<String>,
    verify_output: Option<String>,
    total_tokens: u64,
    phase_event_id: Option<String>,
}

/// 执行单个子任务的核心逻辑
async fn run_single_subtask_worker(
    app: AppHandle,
    task_id: String,
    session_id: String,
    workspace_path: String,
    subtask: TaskSubItem,
    subtask_idx: usize,
    total_subtasks: usize,
    max_steps: usize,
    working_memory: String,
    rx: watch::Receiver<bool>,
    semaphore: Arc<Semaphore>,
) -> SubtaskExecutionResult {
    let _permit = semaphore.acquire().await.ok();
    let state = app.state::<crate::AppState>();

    let parent_session = {
        let db = state.db.lock().unwrap();
        match store::get_session(&db, &session_id) {
            Ok(Some(s)) => s,
            _ => {
                return SubtaskExecutionResult {
                    subtask_idx,
                    sub_id: subtask.id.clone(),
                    status: "failed".to_string(),
                    summary: String::new(),
                    error: Some("父会话不存在".to_string()),
                    verify_output: None,
                    total_tokens: 0,
                    phase_event_id: None,
                };
            }
        }
    };

    let role = if subtask.title.contains("前端") || subtask.title.contains("UI") {
        "frontend"
    } else if subtask.title.contains("后端") || subtask.title.contains("接口") || subtask.title.contains("数据") {
        "backend"
    } else if subtask.title.contains("测试") || subtask.title.contains("验收") || subtask.title.contains("校验") {
        "testing"
    } else if subtask.title.contains("审阅") || subtask.title.contains("评审") {
        "review"
    } else {
        "fullstack"
    };

    // 检查该阶段是否已有绑定的子会话实体（例如恢复推进已暂停或中断的阶段）
    let existing_sub = {
        let db = state.db.lock().unwrap();
        if subtask.id.len() >= 30 && !subtask.id.starts_with("sub-") {
            store::get_session(&db, &subtask.id).ok().flatten()
        } else {
            // 容错兜底：若 subtask.id 尚未写入真实 UUID，从父会话下已有子会话列表中匹配同名任务
            let subs = store::list_all_child_sessions(&db, &session_id).unwrap_or_default();
            subs.into_iter().find(|s| s.title == subtask.title)
        }
    };

    let mut scope_instruction = String::new();
    if let Some(ref sp) = subtask.scope_path {
        scope_instruction = format!("\n4. 本阶段已指派模块目录范围：`{}`，请重点对该目录及相关模块进行修改，避免越界改动无关代码。", sp);
    }

    let mut verify_instruction = String::new();
    if let Some(ref vc) = subtask.verify_command {
        verify_instruction = format!("\n5. 本阶段挂载了自动化验证门禁：`{}`，请务必保证相关代码在执行该命令时语法与测试通过。", vc);
    }

    let prompt = format!(
        "【长任务自主推进阶段 {}/{}】: {}\n\n{}\n\n目标详情: {}\n\n【自主执行指令】:\n1. 请独立调用相关工具完成本阶段任务（本阶段单子任务轮次预算硬上限为 {} 步）；\n2. 产出具体改动或验证结论，严禁停留在空泛设想；\n3. 完成后在最终答复中明确总结本次子任务所完成的具体成果。{}{}",
        subtask_idx + 1,
        total_subtasks,
        subtask.title,
        working_memory,
        subtask.description.as_deref().unwrap_or(""),
        max_steps,
        scope_instruction,
        verify_instruction
    );

    let (sub, created_phase_ev, phase_event_id) = if let Some(sub) = existing_sub {
        // 复用并接续已有子任务会话（幂等性保证）
        let (ev_opt, ev_id) = {
            let db = state.db.lock().unwrap();
            let mut ev_found = None;
            let mut id_found = None;
            if let Ok(mut stmt) = db.prepare(
                "SELECT id, message_id, tool_name, tool_call_id, params_json, result_text, status, approval_scope, created_at, subprocess_id, reverted_at 
                 FROM tool_events 
                 WHERE subprocess_id = ?1 AND tool_name = 'spawn_subagent' 
                 ORDER BY created_at DESC LIMIT 1"
            ) {
                if let Ok(mut rows) = stmt.query(rusqlite::params![&sub.id]) {
                    if let Ok(Some(row)) = rows.next() {
                        let id: String = row.get(0).unwrap_or_default();
                        id_found = Some(id.clone());
                        let ev = ToolEvent {
                            id,
                            message_id: row.get(1).unwrap_or_default(),
                            tool_name: row.get(2).unwrap_or_default(),
                            tool_call_id: row.get(3).ok(),
                            params: serde_json::from_str(&row.get::<_, String>(4).unwrap_or_default()).unwrap_or(json!({})),
                            result_text: Some(format!("阶段子进程恢复推进 (ID: {})", sub.id)),
                            status: "running".to_string(),
                            approval_scope: row.get(7).ok(),
                            created_at: row.get(8).unwrap_or_default(),
                            subprocess_id: row.get(9).ok(),
                            reverted_at: row.get(10).ok(),
                        };
                        ev_found = Some(ev);
                    }
                }
            }
            if let Some(ref ev) = ev_found {
                let _ = db.execute(
                    "UPDATE tool_events SET status = 'running', result_text = ?1 WHERE id = ?2",
                    rusqlite::params![&ev.result_text, &ev.id],
                );
                let _ = store::set_session_status(&db, &sub.id, "running");
            }
            (ev_found, id_found)
        };

        if let Some(ref ev) = ev_opt {
            crate::emit_event(
                &app,
                "tool:update",
                &json!({ "sessionId": &session_id, "event": &ev }),
            );
        }

        let _ = app.emit("subprocess:created", json!({
            "parentId": &session_id,
            "subprocess": &sub,
        }));
        let _ = app.emit("subprocesses:changed", json!({
            "parentId": &session_id,
        }));
        let _ = app.emit("subagents:changed", json!({
            "parentId": &session_id,
        }));

        if let Err(e) = agent::restart_subagent(&app, &sub.id) {
            eprintln!("长任务恢复子会话执行失败: {e}");
        }

        // 确保 long_tasks 表中持久化了该子会话 ID
        {
            let db = state.db.lock().unwrap();
            if let Ok(Some(mut lt)) = store::get_long_task(&db, &task_id) {
                if let Some(s) = lt.subtasks.get_mut(subtask_idx) {
                    if s.id != sub.id {
                        s.id = sub.id.clone();
                        lt.updated_at = store::now();
                        let _ = store::update_long_task(&db, &lt);
                    }
                }
            }
        }

        (sub, ev_opt, ev_id)
    } else {
        let (sub, user_msg) = {
            let db = state.db.lock().unwrap();
            let s = match store::create_subagent_session(
                &db,
                &session_id,
                role,
                &subtask.title,
                subtask.description.as_deref().unwrap_or(&subtask.title),
                &workspace_path,
                parent_session.access_mode.as_deref(),
                parent_session.project_id.as_deref(),
                None,
            ) {
                Ok(s) => s,
                Err(e) => {
                    return SubtaskExecutionResult {
                        subtask_idx,
                        sub_id: subtask.id.clone(),
                        status: "failed".to_string(),
                        summary: String::new(),
                        error: Some(e),
                        verify_output: None,
                        total_tokens: 0,
                        phase_event_id: None,
                    };
                }
            };
            let m = match store::new_message(&db, &s.id, "user", Some(prompt), false) {
                Ok(m) => m,
                Err(e) => {
                    return SubtaskExecutionResult {
                        subtask_idx,
                        sub_id: s.id,
                        status: "failed".to_string(),
                        summary: String::new(),
                        error: Some(e),
                        verify_output: None,
                        total_tokens: 0,
                        phase_event_id: None,
                    };
                }
            };
            (s, m)
        };

        // 关键固化：新创建子会话后，第一时间将子会话 ID 写回数据库 long_tasks 表
        // 保证即使随后立即发生暂停、异常或中断，该阶段也永久锚定该子会话，绝不重复创建
        {
            let db = state.db.lock().unwrap();
            if let Ok(Some(mut lt)) = store::get_long_task(&db, &task_id) {
                if let Some(s) = lt.subtasks.get_mut(subtask_idx) {
                    s.id = sub.id.clone();
                }
                lt.updated_at = store::now();
                let _ = store::update_long_task(&db, &lt);
            }
        }

        let _ = app.emit("subprocess:created", json!({
            "parentId": &session_id,
            "subprocess": &sub,
        }));
        let _ = app.emit("subprocesses:changed", json!({
            "parentId": &session_id,
        }));
        let _ = app.emit("subagent:created", json!({
            "parentId": &session_id,
            "subagent": &sub,
        }));
        let _ = app.emit("subagents:changed", json!({
            "parentId": &session_id,
        }));

        let phase_start_msg = format!(
            "🚀 **开始推进阶段 [{}/{}]**：【{}】\n\n- 任务说明：{}\n- 派生独立子进程：`{}`（点击下方子任务卡片或顶栏透明查看执行过程）",
            subtask_idx + 1,
            total_subtasks,
            subtask.title,
            subtask.description.as_deref().unwrap_or("按阶段计划推进"),
            sub.id
        );

        let phase_event_id = format!("ev-phase-{}", uuid::Uuid::new_v4().simple());
        let phase_call_id = format!("call-phase-{}", uuid::Uuid::new_v4().simple());

        let ev = {
            let db = state.db.lock().unwrap();
            if let Ok(m) = store::new_message(&db, &session_id, "assistant", Some(phase_start_msg), false) {
                let ev = ToolEvent {
                    id: phase_event_id.clone(),
                    message_id: m.id.clone(),
                    tool_name: "spawn_subagent".to_string(),
                    tool_call_id: Some(phase_call_id),
                    params: json!({
                        "subprocess_id": sub.id,
                        "title": subtask.title,
                        "task": subtask.description.as_deref().unwrap_or(&subtask.title),
                        "role": role,
                    }),
                    result_text: Some(format!("派生阶段子进程就绪 (ID: {})", sub.id)),
                    status: "running".to_string(),
                    approval_scope: None,
                    created_at: store::now(),
                    subprocess_id: Some(sub.id.clone()),
                    reverted_at: None,
                };
                let _ = store::insert_tool_event(&db, &ev);
                let mut m_with_ev = m.clone();
                m_with_ev.tool_events = vec![ev.clone()];
                let _ = app.emit("message:final", &m_with_ev);
                crate::emit_event(
                    &app,
                    "tool:update",
                    &json!({ "sessionId": &session_id, "event": &ev }),
                );
                Some(ev)
            } else {
                None
            }
        };

        agent::spawn_session_task(app.clone(), sub.id.clone(), Some(user_msg.id));
        (sub, ev, Some(phase_event_id))
    };

    // 等待子会话执行完成（并实时拉取同步父会话 Token 增量，秒级驱动长任务进度条）
    tokio::time::sleep(Duration::from_millis(500)).await;
    let mut last_seen_parent_tokens: u64 = 0;
    while agent::is_run_active(&state, &sub.id) {
        if *rx.borrow() {
            agent::stop_session(&app, &sub.id);
            if let Some(ref ev) = created_phase_ev {
                let mut ev_failed = ev.clone();
                ev_failed.status = "interrupted".to_string();
                ev_failed.result_text = Some("阶段已中断".to_string());
                {
                    let db = state.db.lock().unwrap();
                    let _ = db.execute("UPDATE tool_events SET status = 'interrupted' WHERE id = ?1", rusqlite::params![&ev.id]);
                }
                crate::emit_event(&app, "tool:update", &json!({ "sessionId": &session_id, "event": &ev_failed }));
            }
            return SubtaskExecutionResult {
                subtask_idx,
                sub_id: sub.id,
                status: "interrupted".to_string(),
                summary: String::new(),
                error: Some("任务已中断".to_string()),
                verify_output: None,
                total_tokens: 0,
                phase_event_id,
            };
        }

        // 实时感知与同步 Token 增量广播
        let (current_tot, current_cached, current_prompt, current_comp): (u64, u64, u64, u64) = {
            let db = state.db.lock().unwrap();
            db.query_row(
                "SELECT COALESCE(total_tokens, 0), COALESCE(cached_tokens, 0), COALESCE(prompt_tokens, 0), COALESCE(completion_tokens, 0) FROM sessions WHERE id = ?1",
                rusqlite::params![&session_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            ).unwrap_or((0, 0, 0, 0))
        };
        if current_tot > last_seen_parent_tokens {
            last_seen_parent_tokens = current_tot;
            let db = state.db.lock().unwrap();
            let _ = db.execute(
                "UPDATE long_tasks SET total_tokens_used = ?1, updated_at = ?2 WHERE id = ?3",
                rusqlite::params![current_tot as i64, store::now(), &task_id],
            );
            if let Ok(Some(mut lt)) = store::get_long_task(&db, &task_id) {
                lt.total_tokens_used = current_tot;
                lt.cached_tokens_used = current_cached;
                lt.prompt_tokens_used = current_prompt;
                lt.completion_tokens_used = current_comp;
                lt.cache_hit_rate = if current_prompt > 0 {
                    ((current_cached as f64 / current_prompt as f64) * 1000.0).round() / 10.0
                } else {
                    0.0
                };
                let _ = app.emit("task:update", &lt);
            }
            if let Ok(Some(parent_sess)) = store::get_session(&db, &session_id) {
                let _ = app.emit("session:update", &parent_sess);
                let _ = app.emit("session:updated", &parent_sess);
            }
        }

        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    if *rx.borrow() {
        return SubtaskExecutionResult {
            subtask_idx,
            sub_id: sub.id,
            status: "interrupted".to_string(),
            summary: String::new(),
            error: Some("任务已中断".to_string()),
            verify_output: None,
            total_tokens: 0,
            phase_event_id,
        };
    }

    // 检查子会话运行状态
    let (is_interrupted, last_status) = {
        let db = state.db.lock().unwrap();
        let lr = store::get_last_run_status(&db, &sub.id);
        let tdb = store::get_long_task(&db, &task_id).ok().flatten();
        let is_paused_or_cancelled = tdb
            .as_ref()
            .map(|t| t.status == "interrupted" || t.status == "cancelled" || t.status == "paused")
            .unwrap_or(false);
        (is_paused_or_cancelled, lr)
    };

    if is_interrupted || last_status.as_deref() != Some("done") {
        let is_failed = last_status.as_deref() == Some("failed");
        let new_st = if is_failed { "failed" } else { "interrupted" };
        if let Some(ref ev) = created_phase_ev {
            let mut ev_failed = ev.clone();
            ev_failed.status = new_st.to_string();
            ev_failed.result_text = Some(format!("阶段推进遇阻: {}", last_status.as_deref().unwrap_or("unknown")));
            {
                let db = state.db.lock().unwrap();
                let _ = db.execute("UPDATE tool_events SET status = ?1 WHERE id = ?2", rusqlite::params![new_st, &ev.id]);
            }
            crate::emit_event(&app, "tool:update", &json!({ "sessionId": &session_id, "event": &ev_failed }));
        }
        return SubtaskExecutionResult {
            subtask_idx,
            sub_id: sub.id,
            status: new_st.to_string(),
            summary: String::new(),
            error: Some(format!("子进程异常退出: {}", last_status.as_deref().unwrap_or("unknown"))),
            verify_output: None,
            total_tokens: 0,
            phase_event_id,
        };
    }

    // 质量门禁执行（带互斥排队与一次定向自愈闭环）
    let ws_path = PathBuf::from(&workspace_path);
    let mut verification_passed = true;
    let mut verify_res_output = String::new();

    if let Some(ref v_cmd) = subtask.verify_command {
        let (passed, out) = execute_verify_command(&ws_path, v_cmd).await;
        verification_passed = passed;
        verify_res_output = out.clone();

        if !verification_passed {
            let fix_prompt = format!(
                "【🚨 自动化质量门禁校验未通过】\n验证命令 `{}` 执行失败，报错如下：\n```\n{}\n```\n请立即根据上述报错信息排查并修复代码缺陷，确保验证命令通过！",
                v_cmd,
                if verify_res_output.len() > 1500 { format!("{}…[截断]", &verify_res_output[..1500]) } else { verify_res_output.clone() }
            );
            let fix_msg = {
                let db = state.db.lock().unwrap();
                store::new_message(&db, &sub.id, "user", Some(fix_prompt), false).ok()
            };
            if let Some(fm) = fix_msg {
                agent::spawn_session_task(app.clone(), sub.id.clone(), Some(fm.id));

                tokio::time::sleep(Duration::from_millis(500)).await;
                while agent::is_run_active(&state, &sub.id) {
                    if *rx.borrow() {
                        agent::stop_session(&app, &sub.id);
                        break;
                    }
                    let (current_tot, current_cached, current_prompt, current_comp): (u64, u64, u64, u64) = {
                        let db = state.db.lock().unwrap();
                        db.query_row(
                            "SELECT COALESCE(total_tokens, 0), COALESCE(cached_tokens, 0), COALESCE(prompt_tokens, 0), COALESCE(completion_tokens, 0) FROM sessions WHERE id = ?1",
                            rusqlite::params![&session_id],
                            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                        ).unwrap_or((0, 0, 0, 0))
                    };
                    if current_tot > last_seen_parent_tokens {
                        last_seen_parent_tokens = current_tot;
                        let db = state.db.lock().unwrap();
                        let _ = db.execute(
                            "UPDATE long_tasks SET total_tokens_used = ?1, updated_at = ?2 WHERE id = ?3",
                            rusqlite::params![current_tot as i64, store::now(), &task_id],
                        );
                        if let Ok(Some(mut lt)) = store::get_long_task(&db, &task_id) {
                            lt.total_tokens_used = current_tot;
                            lt.cached_tokens_used = current_cached;
                            lt.prompt_tokens_used = current_prompt;
                            lt.completion_tokens_used = current_comp;
                            lt.cache_hit_rate = if current_prompt > 0 {
                                ((current_cached as f64 / current_prompt as f64) * 1000.0).round() / 10.0
                            } else {
                                0.0
                            };
                            let _ = app.emit("task:update", &lt);
                        }
                        if let Ok(Some(parent_sess)) = store::get_session(&db, &session_id) {
                            let _ = app.emit("session:update", &parent_sess);
                            let _ = app.emit("session:updated", &parent_sess);
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(300)).await;
                }
                let (re_passed, re_out) = execute_verify_command(&ws_path, v_cmd).await;
                verification_passed = re_passed;
                verify_res_output = re_out;
            }
        }
    }

    if !verification_passed {
        return SubtaskExecutionResult {
            subtask_idx,
            sub_id: sub.id,
            status: "failed".to_string(),
            summary: String::new(),
            error: Some("自动化质量门禁校验未通过".to_string()),
            verify_output: Some(verify_res_output),
            total_tokens: 0,
            phase_event_id,
        };
    }

    let (summary, total_tokens) = {
        let db = state.db.lock().unwrap();
        let msgs = store::all_messages(&db, &sub.id).unwrap_or_default();
        let last_asst = msgs.iter().rev().find(|m| m.role == "assistant");
        let sum = last_asst.and_then(|m| m.content.clone()).unwrap_or_else(|| "本阶段已顺利执行完毕。".to_string());
        let tok_sum: u64 = {
            let sess_tok = store::get_session(&db, &sub.id).ok().flatten().and_then(|s| s.total_tokens).unwrap_or(0);
            if sess_tok > 0 {
                sess_tok
            } else {
                db.query_row(
                    "SELECT COALESCE(SUM(total_tokens), 0) FROM runs WHERE session_id = ?1",
                    rusqlite::params![&sub.id],
                    |r| r.get(0),
                ).unwrap_or(0)
            }
        };
        (sum, tok_sum)
    };

    // 保存阶段交付工件
    let touched_files = Vec::new();
    let _ = crate::tools::subagent::save_subtask_report_artifact(
        &ws_path,
        &sub.id,
        &subtask.title,
        role,
        &summary,
        &touched_files,
    );

    if let Some(ref ev) = created_phase_ev {
        let mut ev_done = ev.clone();
        ev_done.status = "success".to_string();
        ev_done.result_text = Some(format!("阶段达成：{}", subtask.title));
        {
            let db = state.db.lock().unwrap();
            let _ = db.execute(
                "UPDATE tool_events SET status = 'success', result_text = ?1 WHERE id = ?2",
                rusqlite::params![&ev_done.result_text, &ev.id],
            );
        }
        crate::emit_event(&app, "tool:update", &json!({ "sessionId": &session_id, "event": &ev_done }));
    }

    SubtaskExecutionResult {
        subtask_idx,
        sub_id: sub.id,
        status: "completed".to_string(),
        summary,
        error: None,
        verify_output: if subtask.verify_command.is_some() { Some(verify_res_output) } else { None },
        total_tokens,
        phase_event_id,
    }
}

/// 长任务主调度执行循环（支持轻量原地波次 Wavefront 并发推进）
pub async fn run_task_loop(app: AppHandle, mut task: LongTask, rx: watch::Receiver<bool>) {
    let state = app.state::<crate::AppState>();

    // 1. Planning 阶段：创建承载拆解思考过程与计划蓝图的助手消息
    if task.subtasks.is_empty() {
        let plan_m = {
            let db = state.db.lock().unwrap();
            store::new_message(&db, &task.session_id, "assistant", None, false).ok()
        };
        let plan_mid = plan_m.as_ref().map(|m| m.id.clone())
            .unwrap_or_else(|| format!("msg-plan-{}", uuid::Uuid::new_v4().simple()));

        let initial_reasoning = "正在深入分析长任务总目标，拆解架构逻辑、执行阶段与验证门禁策略...\n\n".to_string();
        let _ = app.emit(
            "message:reasoning:delta",
            json!({
                "sessionId": &task.session_id,
                "messageId": &plan_mid,
                "delta": &initial_reasoning,
            }),
        );

        let (subtasks, reasoning_result, plan_usage) = decompose_goal(&app, &task.session_id, &plan_mid, &task.goal).await;
        task.subtasks = subtasks;
        if *rx.borrow() {
            task.status = "paused".to_string();
            task.updated_at = store::now();
            {
                let db = state.db.lock().unwrap();
                let _ = store::update_long_task(&db, &task);
            }
            let _ = app.emit("task:update", &task);
            return;
        }
        let plan_tokens: u64 = {
            let db = state.db.lock().unwrap();
            db.query_row(
                "SELECT COALESCE(total_tokens, 0) FROM sessions WHERE id = ?1",
                rusqlite::params![&task.session_id],
                |r| r.get(0),
            ).unwrap_or(0)
        };
        let final_plan_tokens = plan_tokens.max(plan_usage.total_tokens);
        task.total_tokens_used = final_plan_tokens;
        task.cached_tokens_used = plan_usage.cached_tokens;
        task.prompt_tokens_used = plan_usage.prompt_tokens;
        task.completion_tokens_used = plan_usage.completion_tokens;
        task.cache_hit_rate = if plan_usage.prompt_tokens > 0 {
            ((plan_usage.cached_tokens as f64 / plan_usage.prompt_tokens as f64) * 1000.0).round() / 10.0
        } else {
            0.0
        };

        task.status = "running".to_string();
        task.current_step = 1;
        task.updated_at = store::now();

        {
            let db = state.db.lock().unwrap();
            let _ = store::update_long_task(&db, &task);
            if let Ok(Some(parent_session)) = store::get_session(&db, &task.session_id) {
                let _ = app.emit("session:update", &parent_session);
                let _ = app.emit("session:updated", &parent_session);
            }
        }
        let _ = app.emit("task:update", &task);

        let mut plan_msg = format!("🎯 **长任务规划蓝图已制定**\n\n**总目标**：{}\n\n**规划推进路线图**：\n", task.goal);
        for sub in &task.subtasks {
            let verify_info = if let Some(ref v) = sub.verify_command {
                format!(" `[验证: {}]`", v)
            } else {
                String::new()
            };
            let wave_info = if sub.wave > 0 {
                format!(" `[Wave: {}]`", sub.wave)
            } else {
                String::new()
            };
            plan_msg.push_str(&format!("{}. **{}**{}{}：{}\n", sub.index, sub.title, wave_info, verify_info, sub.description.as_deref().unwrap_or("")));
        }
        plan_msg.push_str("\n🚀 开始自动推进任务路线图...");

        let full_reasoning = if reasoning_result.trim().is_empty() {
            initial_reasoning
        } else {
            format!("{}{}", initial_reasoning, reasoning_result)
        };

        let plan_usage_json = json!({
            "promptTokens": plan_usage.prompt_tokens,
            "completionTokens": plan_usage.completion_tokens,
            "totalTokens": plan_usage.total_tokens,
            "cachedTokens": plan_usage.cached_tokens,
            "reasoningTokens": plan_usage.reasoning_tokens,
            "isEstimated": plan_usage.is_estimated,
        });

        {
            let db = state.db.lock().unwrap();
            let _ = store::update_message_content(&db, &plan_mid, &plan_msg, Some(&plan_usage_json));
            if !full_reasoning.is_empty() {
                let _ = store::update_message_reasoning(&db, &plan_mid, &full_reasoning);
            }
        }
        if let Some(mut m) = plan_m {
            m.content = Some(plan_msg);
            m.reasoning = Some(full_reasoning);
            m.prompt_tokens = Some(plan_usage.prompt_tokens);
            m.completion_tokens = Some(plan_usage.completion_tokens);
            m.total_tokens = Some(plan_usage.total_tokens);
            m.cached_tokens = Some(plan_usage.cached_tokens);
            m.usage = Some(plan_usage_json);
            let _ = app.emit("message:final", &m);
        }
    }

    let semaphore = Arc::new(Semaphore::new(DEFAULT_MAX_CONCURRENCY));

    // 2. 波次推进循环（Wavefront Execution）
    while task.subtasks.iter().any(|s| s.status != "completed" && s.status != "skipped") {
        if *rx.borrow() {
            task.status = "paused".to_string();
            task.updated_at = store::now();
            {
                let db = state.db.lock().unwrap();
                let _ = store::update_long_task(&db, &task);
            }
            let _ = app.emit("task:update", &task);
            return;
        }

        // 预算硬熔断防护
        if let Some(budget) = task.max_budget_tokens {
            if task.total_tokens_used >= budget {
                task.status = "paused".to_string();
                task.updated_at = store::now();
                {
                    let db = state.db.lock().unwrap();
                    let _ = store::update_long_task(&db, &task);
                    let alert = format!(
                        "⚠️ **长任务已触发 Token 预算硬熔断保护！**\n已消耗 {} tokens，超过预设上限 {} tokens。任务已自动暂停，等待人工确认。",
                        task.total_tokens_used, budget
                    );
                    let _ = store::new_message(&db, &task.session_id, "assistant", Some(alert), false);
                }
                let _ = app.emit("task:update", &task);
                let _ = app.emit("task:budget_exceeded", json!({ "taskId": task.id, "tokensUsed": task.total_tokens_used, "budget": budget }));
                return;
            }
        }

        // 找到当前未完成子任务中的最小 Wave 编号（effective wave：若 wave == 0 则视为 index）
        let active_wave = task.subtasks.iter()
            .filter(|s| s.status != "completed" && s.status != "skipped")
            .map(|s| if s.wave == 0 { s.index } else { s.wave })
            .min()
            .unwrap_or(1);

        // 收集该 Wave 内部所有未完成的子任务下标
        let wave_indices: Vec<usize> = task.subtasks.iter().enumerate()
            .filter(|(_, s)| {
                let eff_wave = if s.wave == 0 { s.index } else { s.wave };
                eff_wave == active_wave && s.status != "completed" && s.status != "skipped"
            })
            .map(|(idx, _)| idx)
            .collect();

        if wave_indices.is_empty() {
            break;
        }

        // 将该波次中处于 pending/failed/interrupted 状态的任务标记为 in_progress
        for &idx in &wave_indices {
            task.subtasks[idx].status = "in_progress".to_string();
            task.subtasks[idx].error = None;
        }
        if let Some(&first_idx) = wave_indices.first() {
            task.current_subtask_index = first_idx;
            task.current_step = first_idx + 1;
        }
        task.status = "running".to_string();
        task.updated_at = store::now();
        {
            let db = state.db.lock().unwrap();
            let _ = store::update_long_task(&db, &task);
        }
        let _ = app.emit("task:update", &task);

        // 获取当前工作记忆
        let cps = {
            let db = state.db.lock().unwrap();
            store::list_task_checkpoints(&db, &task.id).unwrap_or_default()
        };
        let working_memory = build_working_memory_block(&cps, &task.goal);

        // 启动该波次并发任务
        let mut handles = Vec::new();
        let total_subtasks = task.subtasks.len();
        let max_steps = task.max_steps;

        for &idx in &wave_indices {
            let subtask = task.subtasks[idx].clone();
            let app_c = app.clone();
            let task_id_c = task.id.clone();
            let session_id_c = task.session_id.clone();
            let workspace_c = task.workspace_path.clone();
            let wm_c = working_memory.clone();
            let rx_c = rx.clone();
            let sem_c = semaphore.clone();

            handles.push(tokio::spawn(async move {
                run_single_subtask_worker(
                    app_c,
                    task_id_c,
                    session_id_c,
                    workspace_c,
                    subtask,
                    idx,
                    total_subtasks,
                    max_steps,
                    wm_c,
                    rx_c,
                    sem_c,
                ).await
            }));
        }

        // 等待当前波次所有任务完成（Wavefront Barrier 同步屏障）
        let mut wave_results = Vec::new();
        for h in handles {
            if let Ok(res) = h.await {
                wave_results.push(res);
            }
        }

        let is_rx_interrupted = *rx.borrow();

        // 统一合并波次执行结果并落地（无论是否发生中断，各子会话 ID、已耗费 tokens 与当前状态全量持久化）
        let mut any_failed = false;
        let mut failure_reason = String::new();

        let ws_path = PathBuf::from(&task.workspace_path);

        for res in wave_results {
            let idx = res.subtask_idx;
            task.subtasks[idx].id = res.sub_id.clone();
            task.total_tokens_used += res.total_tokens;

            if res.status == "completed" {
                task.subtasks[idx].status = "completed".to_string();
                task.subtasks[idx].summary = if !res.summary.is_empty() { Some(res.summary.clone()) } else { None };
                task.subtasks[idx].error = None;
                task.subtasks[idx].verify_output = res.verify_output;

                let git_hash = create_git_snapshot(&ws_path, &task.id, idx + 1);
                let cp = TaskCheckpoint {
                    id: format!("cp-{}", uuid::Uuid::new_v4().simple()),
                    task_id: task.id.clone(),
                    step_number: idx + 1,
                    subtask_id: Some(res.sub_id.clone()),
                    status: "completed".to_string(),
                    summary: res.summary.clone(),
                    working_memory: format!("阶段 [{}] {} 已完成。产出摘要：{}", idx + 1, task.subtasks[idx].title, res.summary),
                    git_commit_hash: git_hash,
                    created_at: store::now(),
                };
                {
                    let db = state.db.lock().unwrap();
                    let _ = store::save_task_checkpoint(&db, &cp);
                    let _ = store::set_session_status(&db, &res.sub_id, "completed");
                }
                let _ = app.emit("task:checkpoint", &cp);
                let _ = app.emit("subagent:update", json!({
                    "parentId": &task.session_id,
                    "parentSessionId": &task.session_id,
                    "subagentId": &res.sub_id,
                    "status": "completed",
                }));

                // 在主会话注入阶段交付成果卡片（赋予真实的阶段子任务 Token 统计与缓存命中率）
                let (phase_tok, phase_prompt, phase_comp, phase_cached, phase_rate) = {
                    let db = state.db.lock().unwrap();
                    if let Ok(Some(sub_s)) = store::get_session(&db, &res.sub_id) {
                        let p = sub_s.prompt_tokens.unwrap_or(0);
                        let c = sub_s.cached_tokens.unwrap_or(0);
                        let rate = sub_s.cache_hit_rate.unwrap_or_else(|| {
                            if p > 0 { ((c as f64 / p as f64) * 1000.0).round() / 10.0 } else { 0.0 }
                        });
                        (
                            sub_s.total_tokens.unwrap_or(res.total_tokens),
                            p,
                            sub_s.completion_tokens.unwrap_or(0),
                            c,
                            rate,
                        )
                    } else {
                        (res.total_tokens, 0, 0, 0, 0.0)
                    }
                };

                let token_details_str = if phase_tok > 0 {
                    if phase_rate > 0.0 {
                        format!(" · 本阶段消耗: {} tokens (缓存命中: {}%)", phase_tok, phase_rate)
                    } else {
                        format!(" · 本阶段消耗: {} tokens", phase_tok)
                    }
                } else {
                    String::new()
                };

                let phase_done_msg = format!(
                    "✅ **阶段 [{}/{}] 顺利达成**：【{}】\n\n**成果摘要**：\n{}\n\n*(门禁验证已通过{} · 检查点已归档)*",
                    idx + 1,
                    task.subtasks.len(),
                    task.subtasks[idx].title,
                    res.summary,
                    token_details_str,
                );

                let phase_usage_json = json!({
                    "promptTokens": phase_prompt,
                    "completionTokens": phase_comp,
                    "totalTokens": phase_tok,
                    "cachedTokens": phase_cached,
                    "isEstimated": false,
                });

                {
                    let db = state.db.lock().unwrap();
                    if let Ok(m) = store::new_message(&db, &task.session_id, "assistant", Some(phase_done_msg.clone()), false) {
                        let _ = store::update_message_content(&db, &m.id, &phase_done_msg, Some(&phase_usage_json));
                        if let Ok(Some(mut full_m)) = store::get_message(&db, &m.id) {
                            full_m.prompt_tokens = Some(phase_prompt);
                            full_m.completion_tokens = Some(phase_comp);
                            full_m.total_tokens = Some(phase_tok);
                            full_m.cached_tokens = Some(phase_cached);
                            full_m.usage = Some(phase_usage_json);
                            let _ = app.emit("message:final", &full_m);
                        }
                    }
                }
            } else if is_rx_interrupted {
                task.subtasks[idx].status = "paused".to_string();
                task.subtasks[idx].error = None;
            } else {
                task.subtasks[idx].status = res.status.clone();
                task.subtasks[idx].summary = if !res.summary.is_empty() { Some(res.summary.clone()) } else { None };
                task.subtasks[idx].error = res.error.clone();
                task.subtasks[idx].verify_output = res.verify_output;
                any_failed = true;
                failure_reason = res.error.clone().unwrap_or_else(|| "未知错误".to_string());
            }
        }

        // 实时获取长任务主会话及所有派生阶段子会话的实际消耗 Token（已自动归集至父会话）
        let (aggregate_task_tokens, agg_cached, agg_prompt, agg_comp): (u64, u64, u64, u64) = {
            let db = state.db.lock().unwrap();
            db.query_row(
                "SELECT COALESCE(total_tokens, 0), COALESCE(cached_tokens, 0), COALESCE(prompt_tokens, 0), COALESCE(completion_tokens, 0) FROM sessions WHERE id = ?1",
                rusqlite::params![&task.session_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            ).unwrap_or((0, 0, 0, 0))
        };
        if aggregate_task_tokens > 0 {
            task.total_tokens_used = aggregate_task_tokens;
            task.cached_tokens_used = agg_cached;
            task.prompt_tokens_used = agg_prompt;
            task.completion_tokens_used = agg_comp;
            task.cache_hit_rate = if agg_prompt > 0 {
                ((agg_cached as f64 / agg_prompt as f64) * 1000.0).round() / 10.0
            } else {
                0.0
            };
        }

        if is_rx_interrupted {
            task.status = "paused".to_string();
            for sub in &mut task.subtasks {
                if sub.status == "in_progress" || sub.status == "running" || sub.status == "verifying" {
                    sub.status = "paused".to_string();
                    sub.error = None;
                }
            }
            task.updated_at = store::now();
            if let Some(next_idx) = get_next_subtask_index(&task) {
                task.current_subtask_index = next_idx;
            }
            {
                let db = state.db.lock().unwrap();
                let _ = store::update_long_task(&db, &task);
                if let Ok(Some(parent_session)) = store::get_session(&db, &task.session_id) {
                    let _ = app.emit("session:update", &parent_session);
                    let _ = app.emit("session:updated", &parent_session);
                }
            }
            let _ = app.emit("task:update", &task);
            let _ = app.emit("subprocesses:changed", json!({ "parentId": &task.session_id }));
            return;
        }

        task.updated_at = store::now();
        if let Some(next_idx) = get_next_subtask_index(&task) {
            task.current_subtask_index = next_idx;
        }
        {
            let db = state.db.lock().unwrap();
            let _ = store::update_long_task(&db, &task);
            if let Ok(Some(parent_session)) = store::get_session(&db, &task.session_id) {
                let _ = app.emit("session:update", &parent_session);
                let _ = app.emit("session:updated", &parent_session);
            }
        }
        let _ = app.emit("task:update", &task);
        let _ = app.emit("subprocesses:changed", json!({ "parentId": &task.session_id }));

        if is_rx_interrupted {
            return;
        }

        // 若当前波次有失败项，尝试自纠动态重规划或平稳暂停
        if any_failed {
            let replanned = replan_subtasks(&app, &task.id, task.current_subtask_index, &failure_reason).await.unwrap_or(false);
            if !replanned {
                task.status = "paused".to_string();
                task.updated_at = store::now();
                {
                    let db = state.db.lock().unwrap();
                    let _ = store::update_long_task(&db, &task);
                    let pause_alert = format!("⚠️ **波次推进遇阻**：{}\n任务已自动暂停，等待人工确认或恢复执行。", failure_reason);
                    let _ = store::new_message(&db, &task.session_id, "assistant", Some(pause_alert), false);
                }
                let _ = app.emit("task:update", &task);
                return;
            }
            continue;
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // 3. 全部完成
    task.status = "completed".to_string();
    task.updated_at = store::now();
    {
        let db = state.db.lock().unwrap();
        let _ = store::update_long_task(&db, &task);
        let _ = db.execute(
            "UPDATE runs SET status = 'done', ended_at = ?1 WHERE session_id = ?2 AND status IN ('running', 'interrupted')",
            rusqlite::params![store::now(), &task.session_id],
        );
        let (final_sess_tokens, final_cached, final_prompt, final_rate): (u64, u64, u64, f64) = {
            if let Ok(Some(ps)) = store::get_session(&db, &task.session_id) {
                let p = ps.prompt_tokens.unwrap_or(0);
                let c = ps.cached_tokens.unwrap_or(0);
                let r = ps.cache_hit_rate.unwrap_or_else(|| {
                    if p > 0 { ((c as f64 / p as f64) * 1000.0).round() / 10.0 } else { 0.0 }
                });
                (ps.total_tokens.unwrap_or(task.total_tokens_used), c, p, r)
            } else {
                (task.total_tokens_used, task.cached_tokens_used, task.prompt_tokens_used, task.cache_hit_rate)
            }
        };

        let token_summary_str = if final_sess_tokens > 0 {
            if final_rate > 0.0 {
                format!(" · 全生命周期消耗: {} tokens · 综合缓存命中率: {}%", final_sess_tokens, final_rate)
            } else {
                format!(" · 全生命周期消耗: {} tokens", final_sess_tokens)
            }
        } else {
            String::new()
        };

        let final_text = format!(
            "🎉 **长任务全部子任务已顺利达成！**\n\n**总目标**：{}\n已通过全部 {} 个阶段性验证检查点，代码与成果已完成归档{}。",
            task.goal,
            task.subtasks.len(),
            token_summary_str,
        );
        let final_usage_json = json!({
            "promptTokens": final_prompt,
            "completionTokens": final_sess_tokens.saturating_sub(final_prompt),
            "totalTokens": final_sess_tokens,
            "cachedTokens": final_cached,
            "isEstimated": false,
        });
        if let Ok(finish_m) = store::new_message(&db, &task.session_id, "assistant", Some(final_text.clone()), false) {
            let _ = store::update_message_content(&db, &finish_m.id, &final_text, Some(&final_usage_json));
            if let Ok(Some(mut full_fm)) = store::get_message(&db, &finish_m.id) {
                full_fm.prompt_tokens = Some(final_prompt);
                full_fm.completion_tokens = Some(final_sess_tokens.saturating_sub(final_prompt));
                full_fm.total_tokens = Some(final_sess_tokens);
                full_fm.cached_tokens = Some(final_cached);
                full_fm.usage = Some(final_usage_json);
                let _ = app.emit("message:final", &full_fm);
            }
        }
    }
    let _ = app.emit("task:update", &task);
    let _ = app.emit("task:finished", json!({ "taskId": task.id, "status": "completed", "goal": task.goal }));
    let _ = app.emit("task:notification", json!({ "title": "长任务达成", "body": format!("目标【{}】已顺利完成！", task.goal) }));
    crate::emit_event(
        &app,
        "run:status",
        &json!({
            "sessionId": &task.session_id,
            "status": "done"
        }),
    );
    remove_control(&task.id);
}
