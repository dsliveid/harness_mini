use crate::approval::{self, Decision};
use crate::llm::{self, LlmCfg};
use crate::models::*;
use crate::store;
use crate::tools::{self, Risk, ToolCtx, ToolSpec};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

pub struct SessionHandle {
    pub abort: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    pub active_run: Mutex<Option<String>>,
}

impl SessionHandle {
    fn new() -> Self {
        Self {
            abort: Mutex::new(None),
            active_run: Mutex::new(None),
        }
    }
}

#[derive(PartialEq, Clone, Copy, Eq, Debug)]
pub enum RunOutcome {
    Done,
    Failed,
}

/// 工具级连续失败与自纠错追踪器
#[derive(Default, Debug, Clone)]
pub struct ToolErrorTracker {
    pub consecutive_failures: HashMap<String, u32>,
    pub max_retry_limit: u32,
}

impl ToolErrorTracker {
    pub fn new(max_retry_limit: u32) -> Self {
        Self {
            consecutive_failures: HashMap::new(),
            max_retry_limit,
        }
    }

    pub fn record_success(&mut self, tool_name: &str) -> bool {
        self.consecutive_failures.remove(tool_name).is_some()
    }

    pub fn record_failure(&mut self, tool_name: &str) -> (bool, u32) {
        let count = self.consecutive_failures.entry(tool_name.to_string()).or_insert(0);
        *count += 1;
        (*count <= self.max_retry_limit, *count)
    }
}

pub fn build_tool_reflexion_prompt(
    tool_name: &str,
    error_msg: &str,
    attempt: u32,
    max_retries: u32,
) -> String {
    let advice = match tool_name {
        "edit_file" => {
            "【自纠建议】: 目标文本未匹配到。请务必先调用 `read_file` 重新读取该文件相关代码行，获取准确的缩进、换行与上下文后再发起编辑。"
        }
        "read_file" | "glob" | "list_dir" => {
            "【自纠建议】: 文件或路径不存在。请检查路径拼写，或先使用 `glob` 搜索工作区以确认真实相对路径。"
        }
        "run_command" | "run_skill" => {
            "【自纠建议】: 命令执行报错。请仔细阅读错误流（stderr），检查命令参数或 Windows PowerShell 语法兼容性。"
        }
        _ => "【自纠建议】: 请检查工具参数是否符合规范，修正后重新尝试。",
    };

    format!(
        "{error_msg}\n\n\
         【⚠️ 工具调用错误自纠提示（第 {attempt}/{max_retries} 次尝试）】\n\
         工具 `{tool_name}` 执行遇到问题。\n\
         {advice}\n\
         请根据上述原因，自主分析并修正参数后再次调用；严禁不作分析直接重复调用相同参数！"
    )
}

pub fn is_run_active(state: &crate::AppState, session_id: &str) -> bool {
    state
        .handles
        .lock()
        .unwrap()
        .get(session_id)
        .map(|h| h.active_run.lock().unwrap().is_some())
        .unwrap_or(false)
}

/// 是否存在任意正在运行的会话任务（切换数据目录前须确认全部空闲）
pub fn any_run_active(state: &crate::AppState) -> bool {
    state
        .handles
        .lock()
        .unwrap()
        .values()
        .any(|h| h.active_run.lock().unwrap().is_some())
}

/// 确保会话句柄存在，并启动/恢复运行任务（trigger=None 时从待执行队列取）
pub fn spawn_session_task(app: AppHandle, session_id: String, trigger: Option<String>) {
    {
        let state = app.state::<crate::AppState>();
        let mut handles = state.handles.lock().unwrap();
        let h = handles.entry(session_id.clone()).or_insert_with(SessionHandle::new);
        if h.active_run.lock().unwrap().is_none() {
            *h.active_run.lock().unwrap() = Some("starting".into());
        }
    }
    let app2 = app.clone();
    let sid2 = session_id.clone();
    // 注意：send_message 是同步命令，不在 tokio 上下文中，必须用 tauri::async_runtime::spawn
    let jh = tauri::async_runtime::spawn(async move {
        run_loop(app2, sid2, trigger).await;
    });
    let state = app.state::<crate::AppState>();
    let handles = state.handles.lock().unwrap();
    if let Some(h) = handles.get(&session_id) {
        let mut g = h.abort.lock().unwrap();
        *g = Some(jh);
    }
}

/// 中断当前运行；根据文档约定，待执行队列不受影响，继续自动依次执行
pub fn stop_session(app: &AppHandle, session_id: &str) {
    stop_session_ext(app, session_id, true);
}

pub fn stop_session_ext(app: &AppHandle, session_id: &str, cascade_subagents: bool) {
    let state = app.state::<crate::AppState>();

    // 0. 级联停止属于该主会话的所有子 Agent 进程
    if cascade_subagents {
        let subs = {
            let db = state.db.lock().unwrap();
            store::list_all_child_sessions(&db, session_id).unwrap_or_default()
        };
        for sub in subs {
            {
                let db = state.db.lock().unwrap();
                let _ = store::set_session_status(&db, &sub.id, "cancelled");
            }
            stop_session_ext(app, &sub.id, false);
            let _ = app.emit("subagent:update", json!({
                "parentId": session_id,
                "subagentId": sub.id,
                "status": "cancelled"
            }));
            let _ = app.emit("subprocess:update", json!({
                "parentId": session_id,
                "parentSessionId": session_id,
                "subprocessId": sub.id,
                "status": "cancelled"
            }));
            let _ = app.emit("collaborator:update", json!({
                "parentId": session_id,
                "parentSessionId": session_id,
                "collaboratorId": sub.id,
                "status": "cancelled"
            }));
            let _ = app.emit("run:status", json!({
                "sessionId": sub.id,
                "status": "cancelled"
            }));
        }
        let _ = app.emit("subagents:changed", json!({"parentId": session_id}));
        let _ = app.emit("subprocesses:changed", json!({"parentId": session_id}));
        let _ = app.emit("collaborators:changed", json!({"parentId": session_id}));
    }

    // 1. 优先强杀该会话下全部正在运行的控制台进程树（必须在 abort 协程前强杀，防止协程终止后子进程孤立遗留）
    {
        let mut cmds = state.running_commands.lock().unwrap();
        let to_cancel: Vec<String> = cmds
            .iter()
            .filter(|(_, rc)| rc.session_id == session_id)
            .map(|(k, _)| k.clone())
            .collect();
        for k in to_cancel {
            if let Some(rc) = cmds.remove(&k) {
                if let Some(pid) = rc.pid {
                    crate::tools::kill_process_tree(pid);
                }
                let _ = rc.tx.send(());
            }
        }
    }

    // 2. 清理该会话挂起的审批
    state
        .approvals
        .lock()
        .unwrap()
        .retain(|_, p| p.session_id != session_id);

    // 3. 中止 Agent 循环任务
    let aborted_run = {
        let handles = state.handles.lock().unwrap();
        let Some(h) = handles.get(session_id) else { return };
        let run = h.active_run.lock().unwrap().take();
        if let Some(jh) = h.abort.lock().unwrap().take() {
            jh.abort();
        }
        run
    };
    if let Some(run_id) = aborted_run {
        // 用户主动停止任务：收尾残留的 in_progress 任务为 pending，避免界面继续转圈
        stop_session_todos(&state, app, session_id);
        // starting = 任务尚未拿到 run_id；无论哪种情况都把库中残留的 running 记录收尾，
        // 避免停止后数据库里留下永久“运行中”的 run（运行状态恢复以数据库为准）
        {
            let db = state.db.lock().unwrap();
            let _ = store::fail_open_runs(&db, session_id);
            if let Ok(events) = store::fail_open_tool_events(&db, session_id) {
                drop(db);
                for ev in events {
                    let _ = app.emit(
                        "tool:update",
                        json!({"sessionId": session_id, "event": ev}),
                    );
                }
            }
        }
        let _ = app.emit(
            "run:status",
            json!({"sessionId": session_id, "runId": run_id, "status": "cancelled"}),
        );
    } else {
        // 会话本就空闲（如前端状态漂移后误点停止）：补发空闲事件让前端复位
        let _ = app.emit(
            "run:status",
            json!({"sessionId": session_id, "status": "idle"}),
        );
    }

    // 4. 若当前会话关联活跃长任务，协同中止长任务，将其运行中的子任务标记为 interrupted
    {
        let db = state.db.lock().unwrap();
        if let Ok(Some(mut lt)) = store::get_active_long_task(&db, session_id) {
            crate::task::set_control(&lt.id, true);
            let _ = store::update_long_task_status(&db, &lt.id, "interrupted");
            if let Some(cur_idx) = crate::task::get_next_subtask_index(&lt) {
                if lt.subtasks[cur_idx].status == "running"
                    || lt.subtasks[cur_idx].status == "verifying"
                    || lt.subtasks[cur_idx].status == "in_progress"
                {
                    lt.subtasks[cur_idx].status = "interrupted".to_string();
                }
            }
            lt.status = "interrupted".to_string();
            lt.updated_at = store::now();
            let _ = store::update_long_task(&db, &lt);
            let _ = app.emit("task:update", &lt);
        }
    }

    // 仅在主会话主动停止且存在排队时消费队列；子会话停止时不自发恢复
    let is_sub = {
        let db = state.db.lock().unwrap();
        store::get_session(&db, session_id)
            .ok()
            .flatten()
            .map(|s| s.session_type == "subagent" || s.parent_session_id.is_some())
            .unwrap_or(false)
    };
    if !is_sub && cascade_subagents {
        spawn_session_task(app.clone(), session_id.to_string(), None);
    }
}

/// 重启指定子 Agent 进程
pub fn restart_subagent(app: &AppHandle, subagent_id: &str) -> Result<(), String> {
    let state = app.state::<crate::AppState>();
    // 先停止并收敛残留状态
    stop_session_ext(app, subagent_id, false);

    let (parent_id, trigger_id) = {
        let db = state.db.lock().unwrap();
        let s = store::get_session(&db, subagent_id)?.ok_or("子 Agent 不存在")?;
        let msgs = store::all_messages(&db, subagent_id)?;
        let last_msg = msgs.last();
        let user_msg_id = match last_msg {
            Some(m) if m.role == "tool" => {
                let nm = store::new_message(&db, subagent_id, "user", Some("请根据上述工具执行结果，继续推进并输出最终结论。".into()), false)?;
                let _ = app.emit("message:final", &nm);
                nm.id
            }
            Some(m) if m.role == "assistant" => {
                let nm = store::new_message(&db, subagent_id, "user", Some("请承接前文未完成的内容与思路，直接继续往下执行，无需重复前文已输出的内容。".into()), false)?;
                let _ = app.emit("message:final", &nm);
                nm.id
            }
            Some(m) if m.role == "user" => {
                m.id.clone()
            }
            _ => {
                let nm = store::new_message(&db, subagent_id, "user", Some("请继续恢复并执行任务。".into()), false)?;
                let _ = app.emit("message:final", &nm);
                nm.id
            }
        };
        (s.parent_session_id, user_msg_id)
    };

    if !trigger_id.is_empty() {
        let _ = app.emit("messages:changed", json!({"sessionId": subagent_id}));
        spawn_session_task(app.clone(), subagent_id.to_string(), Some(trigger_id));
        if let Some(ref pid) = parent_id {
            let _ = app.emit("subagent:update", json!({
                "parentId": pid,
                "subagentId": subagent_id,
                "status": "running"
            }));
            let _ = app.emit("subprocess:update", json!({
                "parentId": pid,
                "parentSessionId": pid,
                "subprocessId": subagent_id,
                "status": "running"
            }));
            let _ = app.emit("subagents:changed", json!({"parentId": pid}));
            let _ = app.emit("subprocesses:changed", json!({"parentId": pid}));
        }
    }
    Ok(())
}

/// 重启主会话下的全部已停止子 Agent
pub fn restart_all_subagents(app: &AppHandle, parent_session_id: &str) -> Result<usize, String> {
    let state = app.state::<crate::AppState>();
    let subs = {
        let db = state.db.lock().unwrap();
        store::list_subagents(&db, parent_session_id)?
    };
    let mut count = 0;
    for s in subs {
        let is_running = is_run_active(&state, &s.id);
        if !is_running {
            let _ = restart_subagent(app, &s.id);
            count += 1;
        }
    }
    Ok(count)
}

/// 重试会话的最后一步（回滚本轮所有后续输出，级联清理本轮子任务，重新触发回复）
pub fn retry_turn(app: &AppHandle, session_id: &str) -> Result<(), String> {
    let state = app.state::<crate::AppState>();
    // 先停止当前运行
    stop_session_ext(app, session_id, false);

    let (trigger_id, user_seq, child_ids_to_clean) = {
        let db = state.db.lock().unwrap();
        let msgs = store::all_messages(&db, session_id)?;
        // 寻找最后一个非排队的 user 消息作为本轮重试的起点
        let last_user = msgs.iter()
            .rev()
            .find(|m| m.role == "user" && !m.queued)
            .ok_or("未找到可重试的用户消息")?;

        let mut child_ids: Vec<String> = Vec::new();
        // 查找通过本轮产生（即父消息 seq > user_seq）的 tool_event 派生的子会话，
        // 或在该 user 消息创建时间之后创建的关联子任务/子进程
        if let Ok(mut stmt) = db.prepare(
            "SELECT id FROM sessions 
             WHERE trigger_tool_event_id IN (
                 SELECT id FROM tool_events WHERE message_id IN (
                     SELECT id FROM messages WHERE session_id = ?1 AND seq > ?2
                 )
             )
             OR (parent_session_id = ?1 AND session_type IN ('subprocess', 'subagent') AND created_at >= ?3)"
        ) {
            if let Ok(rows) = stmt.query_map(rusqlite::params![session_id, last_user.seq, last_user.created_at], |r| r.get(0)) {
                for r in rows.flatten() {
                    child_ids.push(r);
                }
            }
        }

        (last_user.id.clone(), last_user.seq, child_ids)
    };

    // 1. 级联强杀并清理本轮派生的全部子任务/子进程（防止留下僵尸任务或在重试时出现重复派生）
    for child_id in &child_ids_to_clean {
        stop_session_ext(app, child_id, false);
        {
            let db = state.db.lock().unwrap();
            let _ = store::delete_session(&db, child_id);
        }
        let _ = app.emit("sessions:changed", json!({"deleted": child_id}));
    }
    if !child_ids_to_clean.is_empty() {
        let _ = app.emit("subprocesses:changed", json!({"parentId": session_id}));
        let _ = app.emit("subagents:changed", json!({"parentId": session_id}));
        let _ = app.emit("collaborators:changed", json!({"parentId": session_id}));
    }

    // 2. 清理该 user 消息之后产生的所有残留消息（包括旧的 assistant 消息、工具事件等）
    // 防止 LLM 看到末尾的 assistant 消息判定该轮已回答完毕而返回空回复
    {
        let db = state.db.lock().unwrap();
        store::delete_messages_after(&db, session_id, user_seq)?;
        store::touch_session(&db, session_id)?;

        // 3. 长任务子任务状态协同重置：若当前长任务子步骤处于失败或中断态，重置为 pending
        if let Ok(Some(mut lt)) = store::get_active_long_task(&db, session_id) {
            let mut changed = false;
            for st in &mut lt.subtasks {
                if st.status == "interrupted" || st.status == "failed" || st.status == "running" {
                    st.status = "pending".to_string();
                    st.error = None;
                    changed = true;
                }
            }
            if changed {
                let _ = store::update_long_task(&db, &lt);
                let _ = app.emit("task:update", &lt);
            }
        }
    }

    // 清理该会话挂起的审批请求
    state
        .approvals
        .lock()
        .unwrap()
        .retain(|_, p| p.session_id != session_id);

    // 通知前端会话消息已回滚重置
    let _ = app.emit("messages:changed", json!({"sessionId": session_id}));

    spawn_session_task(app.clone(), session_id.to_string(), Some(trigger_id.clone()));
    let _ = app.emit(
        "run:status",
        json!({"sessionId": session_id, "status": "running", "triggerId": trigger_id}),
    );
    Ok(())
}

/// 继续推进会话（承接上一步未完成内容或工具结果，自动恢复未完成的子任务与长任务）
pub fn continue_turn(app: &AppHandle, session_id: &str) -> Result<(), String> {
    let state = app.state::<crate::AppState>();
    if is_run_active(&state, session_id) {
        return Err("当前会话正在运行中，无需继续".into());
    }

    // 1. 自动协同恢复关联的未完成子任务/子进程
    let subs_to_resume = {
        let db = state.db.lock().unwrap();
        let subs = store::list_subagents(&db, session_id).unwrap_or_default();
        subs.into_iter()
            .filter(|s| s.status == "interrupted" || s.status == "failed" || s.status == "cancelled")
            .map(|s| s.id)
            .collect::<Vec<_>>()
    };
    for sub_id in subs_to_resume {
        let _ = restart_subagent(app, &sub_id);
    }

    // 2. 自动协同恢复关联的未完成长任务
    {
        let db = state.db.lock().unwrap();
        if let Ok(Some(lt)) = store::get_active_long_task(&db, session_id) {
            if lt.status == "interrupted" || lt.status == "failed" || lt.status == "paused" {
                drop(db);
                let _ = crate::task::resume_long_task(app, &lt.id);
            }
        }
    }

    // 3. 为主会话准备触发消息
    let (trigger_id, new_msg) = {
        let db = state.db.lock().unwrap();
        // 清理末尾悬挂的报错系统消息
        let _ = db.execute(
            "DELETE FROM messages WHERE session_id = ?1 AND role = 'system' AND content LIKE '⚠️%'",
            rusqlite::params![session_id],
        );

        let msgs = store::all_messages(&db, session_id)?;
        let last_msg = msgs.last();

        match last_msg {
            Some(m) if m.role == "tool" => {
                // 上一步工具执行完毕，注入引导让助手做总结回复
                let new_m = store::new_message(
                    &db,
                    session_id,
                    "user",
                    Some("请根据上述工具执行结果，继续推进并输出最终结论。".into()),
                    false,
                )?;
                (new_m.id.clone(), Some(new_m))
            }
            Some(m) if m.role == "assistant" => {
                // 助手输出中途停止或已完成，注入接续提示词
                let new_m = store::new_message(
                    &db,
                    session_id,
                    "user",
                    Some("请承接前文未完成的内容与思路，直接继续往下执行，无需重复前文已输出的内容。".into()),
                    false,
                )?;
                (new_m.id.clone(), Some(new_m))
            }
            Some(m) if m.role == "user" => {
                (m.id.clone(), None)
            }
            _ => {
                let new_m = store::new_message(
                    &db,
                    session_id,
                    "user",
                    Some("请继续执行任务。".into()),
                    false,
                )?;
                (new_m.id.clone(), Some(new_m))
            }
        }
    };

    if let Some(ref m) = new_msg {
        let _ = app.emit("message:final", m);
    }
    let _ = app.emit("messages:changed", json!({"sessionId": session_id}));

    spawn_session_task(app.clone(), session_id.to_string(), Some(trigger_id.clone()));
    let _ = app.emit(
        "run:status",
        json!({"sessionId": session_id, "status": "running", "triggerId": trigger_id}),
    );
    Ok(())
}

async fn run_loop(app: AppHandle, session_id: String, mut trigger: Option<String>) {
    let state = app.state::<crate::AppState>();
    let mut last_outcome = RunOutcome::Done;
    loop {
        // 1. 取触发消息：显式指定，或从待执行队列取出最早一条
        let trigger_id = match trigger.take() {
            Some(t) => Some(t),
            None => {
                let popped = {
                    let db = state.db.lock().unwrap();
                    store::pop_queued(&db, &session_id)
                };
                match popped {
                    Ok(Some(m)) => {
                        let _ = app.emit("message:final", &m);
                        let _ = app.emit(
                            "queue:update",
                            json!({"sessionId": session_id, "items": queued_payload(&state, &session_id)}),
                        );
                        Some(m.id)
                    }
                    _ => None,
                }
            }
        };
        let Some(trigger_id) = trigger_id else { break };

        // 2. 创建 Run 并记录开始时间与关联触发消息
        let run_start_instant = Instant::now();
        let run_id = {
            let db = state.db.lock().unwrap();
            let rid = store::create_run(&db, &session_id, Some(&trigger_id)).ok();
            if let Some(ref r) = rid {
                let _ = store::set_message_run_id(&db, &trigger_id, r);
            }
            rid
        };
        let Some(run_id) = run_id else { break };
        if let Some(h) = state.handles.lock().unwrap().get(&session_id) {
            *h.active_run.lock().unwrap() = Some(run_id.clone());
        }
        let _ = app.emit(
            "run:status",
            json!({
                "sessionId": session_id,
                "runId": run_id,
                "status": "running",
                "triggerId": trigger_id,
                "startedAt": store::now(),
            }),
        );

        // 3. 运行一次 Agent 主循环
        let (outcome, last_assistant_id, run_tokens) = run_once(&app, &session_id, &run_id).await;
        last_outcome = outcome;
        let run_duration_ms = run_start_instant.elapsed().as_millis() as u64;
        let status = match outcome {
            RunOutcome::Done => "done",
            RunOutcome::Failed => "failed",
        };
        {
            let db = state.db.lock().unwrap();
            let _ = store::finish_run(
                &db,
                &run_id,
                status,
                Some(run_duration_ms),
                Some(run_tokens.total_tokens),
                Some(run_tokens.prompt_tokens),
                Some(run_tokens.completion_tokens),
            );
            if let Some(ref aid) = last_assistant_id {
                let _ = store::update_message_turn_duration(&db, aid, run_duration_ms);
                if let Ok(Some(m)) = store::get_message(&db, aid) {
                    let _ = app.emit("message:final", &m);
                }
            }
        }
        let _ = app.emit(
            "run:status",
            json!({
                "sessionId": session_id,
                "runId": run_id,
                "status": status,
                "durationMs": run_duration_ms,
                "totalTokens": run_tokens.total_tokens,
                "lastAssistantId": last_assistant_id,
            }),
        );
        // 方案 1：在会话正常完成交付后，异步触发后台主题碎记自动提炼兜底
        if outcome == RunOutcome::Done {
            crate::memory::trigger_auto_distillation(app.clone(), session_id.clone(), run_id.clone());
        }
        // 失败时不再自动消费队列，等待用户处理
        if outcome == RunOutcome::Failed {
            break;
        }
    }
    if let Some(h) = state.handles.lock().unwrap().get(&session_id) {
        *h.active_run.lock().unwrap() = None;
    }
    // 兜底：任务被中止（stop）时走不到 finish_run，把残留的 running run 标记为 interrupted，
    // 否则以数据库为准的运行状态恢复会把该会话永远显示为“运行中”
    {
        let db = state.db.lock().unwrap();
        let _ = store::fail_open_runs(&db, &session_id);
        if let Ok(events) = store::fail_open_tool_events(&db, &session_id) {
            drop(db);
            for ev in events {
                let _ = app.emit(
                    "tool:update",
                    json!({"sessionId": session_id, "event": ev}),
                );
            }
        }
    }

    // 若当前会话属于子进程或协作者，将其执行结论持久化并广播给主会话和前端界面
    let mut auto_report_task: Option<(String, bool)> = None;
    {
        let db = state.db.lock().unwrap();
        if let Ok(Some(s)) = store::get_session(&db, &session_id) {
            if s.session_type == "collaborator" || s.session_type == "subprocess" || s.session_type == "subagent" || s.parent_session_id.is_some() {
                // 若该会话已被显式取消、终止或中断，严禁反向冲刷为 completed！
                if s.status == "cancelled" || s.status == "stopped" || s.status == "interrupted" {
                    return;
                }
                let last_run_status = store::get_last_run_status(&db, &session_id);
                if matches!(last_run_status.as_deref(), Some("interrupted") | Some("cancelled")) {
                    let _ = store::set_session_status(&db, &session_id, "cancelled");
                    return;
                }
                // 若关联的父会话已被取消、中断或停止，严禁子任务独立冲刷为 completed！
                if let Some(ref pid) = s.parent_session_id {
                    if let Ok(Some(ps)) = store::get_session(&db, pid) {
                        if ps.status == "cancelled" || ps.status == "stopped" || ps.status == "interrupted" {
                            let _ = store::set_session_status(&db, &session_id, "cancelled");
                            return;
                        }
                    }
                }

                let parent_id = s.parent_session_id.clone().unwrap_or_default();
                let sub_status = match last_outcome {
                    RunOutcome::Done => "completed",
                    RunOutcome::Failed => "failed",
                };
                let _ = store::set_session_status(&db, &session_id, sub_status);
                let _ = app.emit("collaborator:update", json!({
                    "parentId": parent_id,
                    "parentSessionId": parent_id,
                    "collaboratorId": session_id,
                    "status": sub_status,
                }));
                let _ = app.emit("collaborators:changed", json!({
                    "parentId": parent_id,
                }));
                let _ = app.emit("subagent:update", json!({
                    "parentId": parent_id,
                    "parentSessionId": parent_id,
                    "subagentId": session_id,
                    "status": sub_status,
                }));
                let _ = app.emit("subagents:changed", json!({
                    "parentId": parent_id,
                    "parentSessionId": parent_id,
                }));
                let _ = app.emit("subprocess:update", json!({
                    "parentId": parent_id,
                    "parentSessionId": parent_id,
                    "subprocessId": session_id,
                    "status": sub_status,
                }));
                let _ = app.emit("subprocesses:changed", json!({
                    "parentId": parent_id,
                    "parentSessionId": parent_id,
                }));

                let is_dispatched_by_parent = store::get_kv(&db, &session_id, "dispatched_by_parent")
                    .ok()
                    .flatten()
                    .map(|v| v == "true")
                    .unwrap_or(false);

                if is_dispatched_by_parent {
                    // 本次任务由主进程 dispatch_collaborator 主动委派，主进程在 wait_collaborators 中拉取结果，跳过自动推送
                    let _ = store::set_kv(&db, &session_id, "dispatched_by_parent", "false");
                } else if s.session_type == "collaborator" && s.auto_report.unwrap_or(true) && matches!(last_outcome, RunOutcome::Done) && !parent_id.is_empty() {
                    auto_report_task = Some((session_id.clone(), true));
                }
            }
        }
    }

    if let Some((collab_id, _)) = auto_report_task {
        let app_clone = app.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            let _ = crate::commands::do_report_collaborator_increment(&app_clone, &collab_id);
        });
    }

    let _ = app.emit(
        "queue:update",
        json!({"sessionId": session_id, "items": queued_payload(&state, &session_id)}),
    );
}

fn queued_payload(state: &crate::AppState, session_id: &str) -> Vec<Value> {
    let db = state.db.lock().unwrap();
    store::list_queued(&db, session_id)
        .unwrap_or_default()
        .iter()
        .map(|m| json!({"id": m.id, "content": m.content.clone().unwrap_or_default(), "createdAt": m.created_at}))
        .collect()
}

/// Agent 单次运行主循环（§5.1）
async fn run_once(app: &AppHandle, session_id: &str, run_id: &str) -> (RunOutcome, Option<String>, RunTokenMetrics) {
    let mut last_assistant_id: Option<String> = None;
    let mut run_tokens = RunTokenMetrics::default();
    let state = app.state::<crate::AppState>();
    let (session, settings) = {
        let db = state.db.lock().unwrap();
        let master = state.master_key.lock().unwrap();
        (
            store::get_session(&db, session_id).ok().flatten(),
            // 必须注入加密 secrets 表中的 API Key，否则以空 Key 请求导致 401
            store::get_settings_with_secrets(&db, &master).unwrap_or_default(),
        )
    };
    let Some(session) = session else { return (RunOutcome::Failed, None, run_tokens); };
    let data_dir = state.data_dir.lock().unwrap().clone();
    let workspace = if session.is_temp {
        if let Some(d) = &data_dir {
            crate::temp::adapt_temp_path(Path::new(&session.workspace_path), d)
        } else {
            PathBuf::from(&session.workspace_path)
        }
    } else {
        PathBuf::from(&session.workspace_path)
    };
    // 临时空间会话：每次运行前检查/重建临时空间（拷贝缺失的项目副本 + 基线提交）
    if session.is_temp {
        if let Err(e) = crate::temp::ensure_space(&state, app, &session) {
            emit_error(app, session_id, "temp", format!("临时空间准备失败: {e}"));
            return (RunOutcome::Failed, None, run_tokens);
        }
    }
    // 未绑定工作区（空字符串）的会话为纯对话模式，跳过存在性检查
    if !session.workspace_path.is_empty() && !workspace.exists() {
        emit_error(app, session_id, "workspace", format!("工作区不存在: {}", workspace.display()));
        return (RunOutcome::Failed, None, run_tokens);
    }

    // 检查当前会话是否包含图片附件
    let has_images = {
        let db = state.db.lock().unwrap();
        store::all_messages(&db, session_id).unwrap_or_default().iter().any(|m| {
            m.attachments.as_ref().map(|atts| atts.iter().any(|a| a.is_image)).unwrap_or(false)
        })
    };

    // 解析模型配置：
    // 1. 若当前会话包含图片附件且配置了专属视觉模型（或全局视觉模型），优先使用视觉模型
    // 2. 否则优先使用会话专属绑定的厂商 + 模型（若已指定），未指定或失效时回退到全局激活模型
    // 注意：对话引擎必须具备 chat 能力；若用户绑定的模型只具备 image_gen，自动回退到厂商内具备 chat 能力的模型或全局激活模型
    let resolved_vision_model = if has_images {
        if let (Some(v_pid), Some(v_mid)) = (&session.vision_provider_id, &session.vision_model_id) {
            settings.providers.iter().find(|p| &p.id == v_pid && p.models.iter().any(|m| m == v_mid))
                .map(|p| (p.clone(), v_mid.clone()))
        } else {
            crate::models::resolve_active_vision_model(&settings)
                .map(|(p, m)| (p.clone(), m.to_string()))
        }
    } else {
        None
    };

    let resolved_model = if let Some(vm) = resolved_vision_model {
        Some(vm)
    } else if let (Some(pid), Some(mid)) = (&session.provider_id, &session.model_id) {
        if let Some(p) = settings.providers.iter().find(|p| &p.id == pid) {
            if p.models.contains(mid) && settings.has_capability(Some(pid), mid, "chat") {
                Some((p.clone(), mid.clone()))
            } else if let Some(chat_m) = p.models.iter().find(|m| settings.has_capability(Some(pid), m, "chat")) {
                Some((p.clone(), chat_m.clone()))
            } else {
                resolve_active_model(&settings).map(|(p, m)| (p.clone(), m.to_string()))
            }
        } else {
            resolve_active_model(&settings).map(|(p, m)| (p.clone(), m.to_string()))
        }
    } else {
        resolve_active_model(&settings).map(|(p, m)| (p.clone(), m.to_string()))
    };

    let Some((pc, model)) = resolved_model else {
        emit_error(
            app,
            session_id,
            "no_model",
            "尚未配置模型厂商，请先在设置中添加厂商与模型并选择。".into(),
        );
        return (RunOutcome::Failed, None, run_tokens);
    };
    let cfg = LlmCfg {
        base_url: pc.base_url.clone(),
        api_key: pc.api_key.clone(),
        model: model.clone(),
    };
    let effective_ctx_limit = session.context_token_limit.unwrap_or_else(|| {
        settings.resolve_context_limit(Some(&pc.id), &model)
    });

    // 临时空间上下文：temp_* 工具依赖的清单（非临时会话为 None，对应工具不下发也不可用）
    let temp_ctx = if session.is_temp {
        let db = state.db.lock().unwrap();
        match crate::temp::load_manifest(&db, session_id) {
            Ok(Some(m)) => Some(crate::temp::TempAgentCtx { manifest: m }),
            _ => None,
        }
    } else {
        None
    };
    let tool_ctx = ToolCtx {
        workspace: workspace.clone(),
        // 临时空间会话：沙箱为整个临时空间根目录（AI 可访问主项目与关联项目的临时副本）
        sandbox_root: if session.is_temp {
            session.temp_root.as_ref().map(|r| {
                if let Some(d) = &data_dir {
                    crate::temp::adapt_temp_path(Path::new(r), d)
                } else {
                    PathBuf::from(r)
                }
            })
        } else {
            None
        },
        command_timeout: Duration::from_secs(settings.command_timeout_secs.max(5)),
        temp: temp_ctx,
        host: Some(tools::HostCtx {
            app: app.clone(),
            session_id: session_id.to_string(),
        }),
        event_id: None,
    };
    let is_subagent = session.session_type == "subagent" || session.parent_session_id.is_some();

    // 工具能力准入管控：
    // 1. 若为主进程且名录中已配置【图像生成协作者】，主进程物理屏蔽直接生图工具 generate_image，强制走 dispatch_collaborator 委派；
    // 2. 只有当前会话本身绑定了生图模型、或属于生图协作者、或主进程未配置生图协作者但全局配有 image_gen 模型时，才下发生图工具。
    let has_collab_image_gen = if !is_subagent {
        let db = state.db.lock().unwrap();
        store::list_collaborators(&db, session_id)
            .map(|list| list.iter().any(|c| c.subagent_role.as_deref() == Some("image_gen") || c.image_model_id.is_some()))
            .unwrap_or(false)
    } else {
        false
    };

    let session_has_image_gen = if session.session_type == "collaborator" && session.subagent_role.as_deref() == Some("image_gen") {
        true
    } else if session.image_model_id.is_some() {
        true
    } else if !has_collab_image_gen {
        settings.active_image_model_id.is_some()
            || session.model_id.as_deref().map(|m| settings.has_capability(session.provider_id.as_deref(), m, "image_gen")).unwrap_or(false)
            || resolve_active_model(&settings).map(|(p, m)| settings.has_capability(Some(&p.id), m, "image_gen")).unwrap_or(false)
    } else {
        false
    };

    let specs: Vec<ToolSpec> = tools::tool_specs()
        .into_iter()
        // 临时空间专用工具仅对临时空间会话下发
        .filter(|s| session.is_temp || !tools::TEMP_TOOL_NAMES.contains(&s.name))
        // 递归防爆：子 Agent 自身不下发子 Agent 创建与管理工具
        .filter(|s| !is_subagent || !tools::SUBAGENT_TOOL_NAMES.contains(&s.name))
        // 工具能力准入：只有具备生图能力的会话才下发 generate_image
        .filter(|s| s.name != "generate_image" || session_has_image_gen)
        // 过滤设置中被禁用的工具
        .filter(|s| !settings.disabled_tools.contains(&s.name.to_string()))
        .collect();
    // 标准 OpenAI tools 格式：function 必须含 name/description/parameters
    let schemas: Vec<Value> = specs
        .iter()
        .map(|s| {
            json!({
                "type": "function",
                "function": {
                    "name": s.name,
                    "description": s.description,
                    "parameters": s.schema
                }
            })
        })
        .collect();
    let max_steps = settings.max_steps.max(1) as usize;

    // 确保工作区记忆目录存在，并执行轻量记忆衰减清理
    if !session.workspace_path.is_empty() {
        let ws_path = std::path::Path::new(&session.workspace_path);
        let _ = crate::memory::ensure_memory_dir(ws_path);
        let _ = crate::memory::prune_memory(ws_path, 30, 20);
    }

    // 项目约束 + 关联项目说明 + 工作区知识记忆：按会话所属项目动态组装进 system prompt。
    // system 位于上下文第 0 位且永不截断 → 新对话注入一次、同对话多条消息不重复；
    // 每次运行重建，修改约束 / 关联或知识沉淀后同对话下一条消息就地生效。
    // 临时空间会话改用临时空间说明段（含临时路径映射与优先级 临时空间 > 关联项目 > 项目约束）
    let (project_section, project_plan_mode, negative_code_intent) = {
        let db = state.db.lock().unwrap();
        let plan_mode = if let Some(ref pid) = session.project_id {
            store::get_project(&db, pid)
                .ok()
                .flatten()
                .map(|p| p.plan_mode)
                .unwrap_or_else(|| "standard".into())
        } else if !session.workspace_path.is_empty() {
            store::find_project_by_path(&db, &session.workspace_path)
                .ok()
                .flatten()
                .map(|p| p.plan_mode)
                .unwrap_or_else(|| "standard".into())
        } else {
            "standard".into()
        };

        let neg_intent = if let Ok(msgs) = store::all_messages(&db, session_id) {
            msgs.iter()
                .filter(|m| m.role == "user")
                .last()
                .and_then(|m| m.content.as_deref())
                .map(contains_negative_code_intent)
                .unwrap_or(false)
        } else {
            false
        };

        let db_sec = if session.is_temp {
            match crate::temp::load_manifest(&db, session_id) {
                Ok(Some(m)) => Some(crate::temp::build_prompt_section(&db, &session, &m)),
                _ => None,
            }
        } else {
            match &session.project_id {
                Some(pid) => build_project_section(&db, pid).unwrap_or(None),
                None => None,
            }
        };
        let mem_sec = if !session.workspace_path.is_empty() {
            crate::memory::load_project_memory(std::path::Path::new(&session.workspace_path))
        } else {
            None
        };
        let collabs_sec = if !is_subagent {
            if let Ok(collabs) = store::list_collaborators(&db, session_id) {
                if !collabs.is_empty() {
                    let mut text = String::from("## 可用项目协作者名录 (Collaborators)\n你拥有以下常驻专业协作者团队。当用户任务命中某位协作者的【调度触发规则】且该协作者处于空闲状态时，你**必须无条件优先调用 `dispatch_collaborator` 委派**，随后紧接着调用 `wait_collaborators` 获取其成果报告，严禁自行直接执行！\n\n");
                    for c in collabs {
                        let is_busy = is_run_active(&state, &c.id);
                        let role_display = match c.subagent_role.as_deref() {
                            Some("image_gen") => "AI 绘画师 / 图像生成",
                            Some("vision") => "视觉感知 / 图像识别",
                            Some("frontend") => "前端开发",
                            Some("backend") => "后端开发",
                            Some("pm") => "产品经理",
                            Some("pmo") => "项目经理",
                            Some("testing") => "测试校验",
                            Some("review") => "代码审阅",
                            Some("fullstack") => "全栈开发",
                            Some(r) => r,
                            None => "专业协作者",
                        };
                        let dispatch_rule = c.dispatch_rule.as_deref()
                            .filter(|s| !s.trim().is_empty())
                            .unwrap_or_else(|| crate::models::default_dispatch_rule_for_role(c.subagent_role.as_deref().unwrap_or("")));
                        let model_info = if let Some(ref m) = c.model_id {
                            format!(" | 对话模型: `{m}`")
                        } else {
                            String::new()
                        };
                        let image_model_info = if let Some(ref im) = c.image_model_id {
                            format!(" | 生图模型: `{im}`")
                        } else {
                            String::new()
                        };
                        text.push_str(&format!(
                            "### 【{}】(ID: `{}` | 角色: {}{}{})\n- **当前状态**: {}\n- **主进程调度触发规则（命中即委派）**: {}\n- **专业职责设定**: {}\n\n",
                            c.title,
                            c.id,
                            role_display,
                            model_info,
                            image_model_info,
                            if is_busy { "🟡 运行中 (busy)" } else { "🟢 空闲 (idle，可立即委派)" },
                            dispatch_rule,
                            c.subagent_task.as_deref().unwrap_or("负责该领域专职工作")
                        ));
                    }
                    Some(text)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let plan_sec = if !session.workspace_path.is_empty() {
            crate::plan::load_active_plan_context(
                std::path::Path::new(&session.workspace_path),
                session_id,
                Some(&db),
            )
        } else {
            None
        };

        let mut parts = Vec::new();
        if let Some(d) = db_sec {
            parts.push(d);
        }
        if let Some(m) = mem_sec {
            parts.push(format!("## 本项目持久化认知与沉淀记忆（来自 .harness/memory/，可秒级召回）\n{}", m));
        }
        if let Some(p) = plan_sec {
            parts.push(p);
        }
        if let Some(c) = collabs_sec {
            parts.push(c);
        }
        let sec = if parts.is_empty() {
            None
        } else {
            Some(parts.join("\n\n"))
        };
        (sec, plan_mode, neg_intent)
    };
    let sys = system_prompt(
        &session,
        project_section.as_deref(),
        &settings.disabled_sops,
        &project_plan_mode,
    );
    let mut files_modified = false;
    let mut sop_retry_count = 0usize;
    let mut sop_verified = false;
    let mut tool_tracker = ToolErrorTracker::new(2);
    let mut has_checked_compaction = false;
    let mut created_plan_in_this_turn = false;
    for _step in 0..max_steps {
        // ---- 步骤 0：在首步或上下文逼近上限时检测是否触发自动压缩与确认 ----
        if !has_checked_compaction {
            let compacted = check_and_trigger_compaction(
                app,
                &state,
                session_id,
                &cfg,
                &sys,
                effective_ctx_limit,
            )
            .await;
            if compacted {
                let _ = app.emit("session:compacted", json!({ "sessionId": session_id }));
            }
            has_checked_compaction = true;
        }

        // ---- 步骤 1/2：组装上下文（含运行中被"引导"注入的新消息） ----
        let (messages, est_in, truncation_notice) = {
            let db = state.db.lock().unwrap();
            build_context(&db, session_id, &sys, effective_ctx_limit)
        };
        if let Some(ref notice) = truncation_notice {
            let _ = app.emit("context:truncated", notice);
        }
        let mut messages = match messages {
            Ok(m) => m,
            Err(e) => {
                emit_error(app, session_id, "context", e);
                return (RunOutcome::Failed, last_assistant_id, run_tokens);
            }
        };

        // 临界步数倒计时收敛提醒：当步数接近上限时注入高优先级系统收敛提示，防止达到 maxSteps 硬性强杀导致无输出
        let remaining_steps = max_steps.saturating_sub(_step);
        if remaining_steps <= 3 && max_steps > 3 {
            let warn_content = format!(
                "【⚠️ 临界步数紧急预警：当前仅剩最后 {remaining_steps} 步，即将达到最大步数上限（{max_steps}）！严禁继续调用任何探索、排查或读取类工具！必须立即根据目前已掌握的所有信息，按规定结构化格式输出最终总结与交付回复！】"
            );
            messages.push(json!({
                "role": "user",
                "content": warn_content,
            }));
        }

        // ---- 步骤 3：流式调用 LLM（先预占 assistant 消息位，供增量事件引用） ----
        let step_start = Instant::now();
        let assistant_id = {
            let db = state.db.lock().unwrap();
            match store::new_message_with_run(&db, session_id, "assistant", Some(String::new()), false, Some(run_id)) {
                Ok(m) => m.id,
                Err(e) => {
                    emit_error(app, session_id, "db", e);
                    return (RunOutcome::Failed, last_assistant_id, run_tokens);
                }
            }
        };
        last_assistant_id = Some(assistant_id.clone());
        let app2 = app.clone();
        let sid2 = session_id.to_string();
        let mid2 = assistant_id.clone();
        let buf2: std::sync::Arc<Mutex<String>> = std::sync::Arc::new(Mutex::new(String::new()));
        let rbuf2: std::sync::Arc<Mutex<String>> = std::sync::Arc::new(Mutex::new(String::new()));

        let mut retry_result: Option<llm::LlmResult> = None;
        let max_stream_retries = 3u32;
        for attempt in 1..=max_stream_retries {
            if attempt > 1 {
                // 回滚上一次尝试中刷入数据库与界面的半截内容
                buf2.lock().unwrap().clear();
                rbuf2.lock().unwrap().clear();
                {
                    let db = state.db.lock().unwrap();
                    let _ = store::update_message_content(&db, &assistant_id, "", None);
                    let _ = store::update_message_reasoning(&db, &assistant_id, "");
                }
                let _ = app.emit(
                    "message:reset",
                    json!({"sessionId": session_id, "messageId": assistant_id}),
                );
                let _ = app.emit(
                    "run:retry",
                    json!({
                        "sessionId": session_id,
                        "messageId": assistant_id,
                        "attempt": attempt,
                        "maxRetries": max_stream_retries,
                        "reason": "网络波动断开连接，正在自动重试...",
                    }),
                );
                let backoff_ms = 1000u64 * (1u64 << (attempt - 2));
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            }

            let mut last_flush = Instant::now();
            let app_text = app2.clone();
            let sid_text = sid2.clone();
            let mid_text = mid2.clone();
            let app_reason = app2.clone();
            let sid_reason = sid2.clone();
            let mid_reason = mid2.clone();

            let call = llm::chat_stream(
                &cfg,
                &messages,
                &schemas,
                {
                    let buf2 = buf2.clone();
                    move |delta| {
                        let mut b = buf2.lock().unwrap();
                        b.push_str(delta);
                        app_text.state::<crate::AppState>().emit(
                            "message:delta",
                            &json!({"sessionId": sid_text, "messageId": mid_text, "delta": delta}),
                        );
                        // 周期性落库，防止崩溃丢失全部内容
                        if last_flush.elapsed() > Duration::from_millis(800) {
                            let st = app_text.state::<crate::AppState>();
                            let db = st.db.lock().unwrap();
                            let _ = store::update_message_content(&db, &mid_text, &b.clone(), None);
                            last_flush = Instant::now();
                        }
                    }
                },
                {
                    let rbuf2 = rbuf2.clone();
                    let mut reasoning_flush = Instant::now();
                    move |delta| {
                        let mut b = rbuf2.lock().unwrap();
                        b.push_str(delta);
                        app_reason.state::<crate::AppState>().emit(
                            "message:reasoning:delta",
                            &json!({"sessionId": sid_reason, "messageId": mid_reason, "delta": delta}),
                        );
                        if reasoning_flush.elapsed() > Duration::from_millis(800) {
                            let st = app_reason.state::<crate::AppState>();
                            let db = st.db.lock().unwrap();
                            let _ = store::update_message_reasoning(&db, &mid_reason, &b.clone());
                            reasoning_flush = Instant::now();
                        }
                    }
                },
            )
            .await;

            match call {
                Ok(r) => {
                    retry_result = Some(r);
                    break;
                }
                Err(e) => {
                    let is_network_error = e.contains("流读取失败")
                        || e.contains("请求失败")
                        || e.contains("error decoding response body")
                        || e.contains("timed out")
                        || e.contains("connection reset")
                        || e.contains("429")
                        || e.contains("502")
                        || e.contains("503")
                        || e.contains("504");
                    if is_network_error && attempt < max_stream_retries {
                        eprintln!("[Agent LLM Retry] attempt {attempt}/{max_stream_retries} failed: {e}. Retrying...");
                        continue;
                    } else {
                        emit_error(app, session_id, "llm", e);
                        return (RunOutcome::Failed, last_assistant_id, run_tokens);
                    }
                }
            }
        }

        let mut result = match retry_result {
            Some(r) => r,
            None => {
                return (RunOutcome::Failed, last_assistant_id, run_tokens);
            }
        };

        // 空回复防护：模型不支持流式/工具或名称错误时给出明确提示
        if result.tool_calls.is_empty() && result.content.trim().is_empty() {
            if !result.reasoning.trim().is_empty() {
                // 推理思考模型（如 DeepSeek-R1、QwQ 等）可能将汇报输出在思考流中，兜底提取正文，避免误杀
                result.content = format!("【思考过程汇报】\n{}", result.reasoning.trim());
            } else {
                // 清理本次为该步骤预占位的空 assistant 消息，防止留在数据库中变成僵尸占位
                {
                    let db = state.db.lock().unwrap();
                    let _ = db.execute(
                        "DELETE FROM messages WHERE id = ?1 AND (content IS NULL OR content = '') AND tool_calls_json IS NULL",
                        rusqlite::params![assistant_id],
                    );
                }
                emit_error(
                    app,
                    session_id,
                    "empty_response",
                    "模型返回了空回复。请检查设置中的模型名称是否正确、该模型是否支持工具调用（function calling），或更换模型后重试。"
                        .into(),
                );
                return (RunOutcome::Failed, last_assistant_id, run_tokens);
            }
        }

        let step_duration_ms = step_start.elapsed().as_millis() as u64;
        let (prompt_tokens, completion_tokens, total_tokens) = match &result.usage {
            Some(u) => {
                let p = u
                    .get("prompt_tokens")
                    .or_else(|| u.get("promptTokens"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(est_in as u64);
                let c = u
                    .get("completion_tokens")
                    .or_else(|| u.get("completionTokens"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or_else(|| {
                        (estimate_tokens(&result.content) + estimate_tokens(&result.reasoning)) as u64
                    });
                let t = u
                    .get("total_tokens")
                    .or_else(|| u.get("totalTokens"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(p + c);
                (p, c, t)
            }
            None => {
                let est_out = estimate_tokens(&result.content) + estimate_tokens(&result.reasoning);
                (est_in as u64, est_out as u64, (est_in + est_out) as u64)
            }
        };

        run_tokens.prompt_tokens += prompt_tokens;
        run_tokens.completion_tokens += completion_tokens;
        run_tokens.total_tokens += total_tokens;

        let usage = json!({
            "promptTokens": prompt_tokens,
            "completionTokens": completion_tokens,
            "totalTokens": total_tokens,
            "inputEst": est_in,
            "outputEst": estimate_tokens(&result.content),
            "durationMs": step_duration_ms,
            "model": &cfg.model,
        });

        if result.tool_calls.is_empty() {
            // ---- 步骤 4：纯文本回复，交付前检查 SOP 自检 ----
            if files_modified && !sop_verified && sop_retry_count < 2 {
                let sop_opt = {
                    let db = state.db.lock().unwrap();
                    let proj = match &session.project_id {
                        Some(pid) => store::get_project(&db, pid).ok().flatten(),
                        None => if !session.workspace_path.is_empty() {
                            store::find_project_by_path(&db, &session.workspace_path).ok().flatten()
                        } else {
                            None
                        }
                    };
                    if let Some(p) = proj {
                        if p.sop_enabled {
                            let cmd = p.sop_verify_cmd.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| {
                                let (_, def_cmd) = crate::sop::detect_project_stack(&workspace);
                                def_cmd
                            });
                            if !cmd.trim().is_empty() {
                                Some(cmd)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else if !session.workspace_path.is_empty() {
                        let (_, def_cmd) = crate::sop::detect_project_stack(&workspace);
                        if !def_cmd.trim().is_empty() {
                            Some(def_cmd)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                if let Some(cmd) = sop_opt {
                    let _ = app.emit(
                        "sop:status",
                        json!({
                            "sessionId": session_id,
                            "status": "checking",
                            "command": cmd,
                        }),
                    );

                    let verify_res = crate::sop::run_verify_cmd(&workspace, &cmd, Duration::from_secs(60)).await;
                    match verify_res {
                        Ok((true, out)) => {
                            sop_verified = true;
                            let _ = app.emit(
                                "sop:status",
                                json!({
                                    "sessionId": session_id,
                                    "status": "passed",
                                    "command": cmd,
                                    "output": out,
                                }),
                            );
                        }
                        Ok((false, out)) => {
                            sop_retry_count += 1;
                            let _ = app.emit(
                                "sop:status",
                                json!({
                                    "sessionId": session_id,
                                    "status": "failed",
                                    "command": cmd,
                                    "output": out,
                                }),
                            );

                            {
                                let db = state.db.lock().unwrap();
                                let _ = store::update_message_content(&db, &assistant_id, &result.content, Some(&usage));
                                if !result.reasoning.is_empty() {
                                    let _ = store::update_message_reasoning(&db, &assistant_id, &result.reasoning);
                                }
                                let err_prompt = format!(
                                    "【🛡️ 交付前 SOP 自检未通过】\n自检命令：`{cmd}`\n执行输出：\n```\n{out}\n```\n检测到上述构建或测试报错。请分析原因并修改代码进行自愈修复，确保自检通过后再交付完成任务。"
                                );
                                let _ = store::new_message(&db, session_id, "user", Some(err_prompt), false);
                            }
                            continue;
                        }
                        Err(e) => {
                            let _ = app.emit(
                                "sop:status",
                                json!({
                                    "sessionId": session_id,
                                    "status": "error",
                                    "command": cmd,
                                    "output": e,
                                }),
                            );
                            sop_verified = true;
                        }
                    }
                }
            }

            // ---- 正常交付：文本消息落库并结束 run_once ----
            {
                let db = state.db.lock().unwrap();
                let _ = store::update_message_content(&db, &assistant_id, &result.content, Some(&usage));
                if !result.reasoning.is_empty() {
                    let _ = store::update_message_reasoning(&db, &assistant_id, &result.reasoning);
                }
            }
            let final_msg = {
                let db = state.db.lock().unwrap();
                store::get_message(&db, &assistant_id).ok().flatten()
            };
            if let Some(m) = final_msg {
                let _ = app.emit("message:final", &m);
            }
            {
                let db = state.db.lock().unwrap();
                let _ = store::touch_session(&db, session_id);
                if let Ok(Some(s)) = store::get_session(&db, session_id) {
                    let _ = app.emit("session:update", &s);
                }
            }
            // 对话正常完成交付时，自动将任务清单中仍处于 in_progress 的任务标记为 done，
            // 避免 Agent 执行完最后一步直接给出最终回复后遗留转圈状态
            auto_finish_session_todos(&state, app, session_id);
            let _ = sop_verified;
            return (RunOutcome::Done, last_assistant_id, run_tokens);
        }

        // ---- 步骤 5：工具调用 ----
        let tcs = result.tool_calls;
        let tc_json: Vec<Value> = tcs
            .iter()
            .map(|t| {
                json!({
                    "id": t.id,
                    "type": "function",
                    "function": {"name": t.name, "arguments": t.args}
                })
            })
            .collect();
        {
            let db = state.db.lock().unwrap();
            let _ = store::update_message_tool_calls(
                &db,
                &assistant_id,
                &Value::Array(tc_json),
                &result.content,
                Some(&usage),
            );
            if !result.reasoning.is_empty() {
                let _ = store::update_message_reasoning(&db, &assistant_id, &result.reasoning);
            }
        }
        let final_msg = {
            let db = state.db.lock().unwrap();
            store::get_message(&db, &assistant_id).ok().flatten()
        };
        if let Some(m) = final_msg {
            let _ = app.emit("message:final", &m);
        }
        {
            let db = state.db.lock().unwrap();
            let _ = store::touch_session(&db, session_id);
            if let Ok(Some(s)) = store::get_session(&db, session_id) {
                let _ = app.emit("session:update", &s);
            }
        }

        for tc in &tcs {
            let args: Value = serde_json::from_str(&tc.args).unwrap_or_else(|_| {
                json!({"__parse_error": "工具参数不是有效 JSON"})
            });

            let is_modifying = tc.name == "write_file" || tc.name == "edit_file";
            let guard_denied = if is_modifying {
                if project_plan_mode == "always_plan" && created_plan_in_this_turn {
                    Some("【模式守卫拦截】: 当前项目启用了「Always Plan 模式」，执行方案刚刚生成，严禁在同一轮次中未经用户确认直接修改代码。请停止调用写入工具，向用户汇报当前计划核心并请求确认。".to_string())
                } else if project_plan_mode == "always_proceed" && negative_code_intent {
                    Some("【模式守卫拦截】: 用户在当前任务中明确说明了先不改动代码（仅出方案/评估）。你已完成计划制定，严格禁止在当前轮次修改代码。请向用户输出方案说明并等待用户指示。".to_string())
                } else {
                    None
                }
            } else {
                None
            };

            let (status, mut result_text) = if let Some(reason) = guard_denied {
                let now = chrono::Utc::now().to_rfc3339();
                let ev = ToolEvent {
                    id: uuid::Uuid::new_v4().to_string(),
                    message_id: assistant_id.clone(),
                    tool_name: tc.name.clone(),
                    tool_call_id: Some(tc.id.clone()),
                    params: args.clone(),
                    result_text: Some(reason.clone()),
                    status: "denied".into(),
                    approval_scope: Some("guard_blocked".into()),
                    created_at: now,
                    subprocess_id: None,
                };
                {
                    let db = state.db.lock().unwrap();
                    let _ = store::insert_tool_event(&db, &ev);
                }
                emit_tool(app, session_id, &ev);
                ("denied".into(), reason)
            } else {
                handle_tool_call(
                    app,
                    session_id,
                    &assistant_id,
                    &tc.id,
                    &tc.name,
                    &args,
                    &specs,
                    &tool_ctx,
                    &settings.disabled_tools,
                )
                .await
            };

            // 工具自动自纠错机制：非用户审批拒绝的执行失败触发内部微反思引导
            if status == "failed" {
                let (can_retry, count) = tool_tracker.record_failure(&tc.name);
                if can_retry {
                    let _ = app.emit(
                        "tool:retry_guidance",
                        json!({
                            "sessionId": session_id,
                            "toolName": tc.name,
                            "status": "retrying",
                            "attempt": count,
                            "maxRetries": tool_tracker.max_retry_limit,
                            "error": result_text,
                        }),
                    );
                    result_text = build_tool_reflexion_prompt(
                        &tc.name,
                        &result_text,
                        count,
                        tool_tracker.max_retry_limit,
                    );
                } else {
                    let _ = app.emit(
                        "tool:retry_guidance",
                        json!({
                            "sessionId": session_id,
                            "toolName": tc.name,
                            "status": "failed",
                            "attempt": count,
                            "maxRetries": tool_tracker.max_retry_limit,
                            "error": result_text,
                            "message": format!("工具 `{}` 已连续失败 {} 次，达到自纠错上限", tc.name, tool_tracker.max_retry_limit),
                        }),
                    );
                    result_text = format!(
                        "{result_text}\n\n【提示】: 工具 `{}` 已连续失败 {} 次（达到自纠错上限），请停止重复尝试，向用户如实陈述原因或尝试其他方案。",
                        tc.name, tool_tracker.max_retry_limit
                    );
                }
            } else if status == "success" {
                if tool_tracker.record_success(&tc.name) {
                    let _ = app.emit(
                        "tool:retry_guidance",
                        json!({
                            "sessionId": session_id,
                            "toolName": tc.name,
                            "status": "success",
                            "attempt": 0,
                            "maxRetries": tool_tracker.max_retry_limit,
                            "message": format!("工具 `{}` 内部自纠修正成功，已恢复正常执行", tc.name),
                        }),
                    );
                }
            }
            // 工具结果作为 tool 消息进入上下文
            let tool_msg = {
                let db = state.db.lock().unwrap();
                let r = store::new_message(&db, session_id, "tool", Some(result_text.clone()), false);
                if let Ok(ref m) = r {
                    let _ = store::set_message_tool_call_id(&db, &m.id, &tc.id);
                }
                r
            };
            if let Ok(m) = tool_msg {
                let _ = app.emit("message:final", &m);
            }
            if (tc.name == "write_file" || tc.name == "edit_file") && status == "success" {
                files_modified = true;
                sop_verified = false;
                if let Some(rel) = args.get("path").and_then(|v| v.as_str()) {
                    let p = std::path::Path::new(rel);
                    let full_path = if p.is_absolute() {
                        p.to_path_buf()
                    } else {
                        std::path::Path::new(&session.workspace_path).join(p)
                    };
                    let _ = app.emit("file_viewer:file_changed", serde_json::json!({
                        "path": full_path.to_string_lossy().to_string()
                    }));
                }
            }
            if (tc.name == "create_plan" || tc.name == "update_plan") && status == "success" {
                created_plan_in_this_turn = true;
                let _ = app.emit("file_viewer:file_changed", serde_json::json!({
                    "path": format!("{}/.harness/plans", session.workspace_path)
                }));
            }
            let _ = status;
        }
        // 回到下一轮循环
    }

    emit_error(
        app,
        session_id,
        "max_steps",
        format!("已达到最大步数（{max_steps}），任务中止。可在设置中调整 maxSteps。"),
    );
    (RunOutcome::Done, last_assistant_id, run_tokens)
}

#[allow(clippy::too_many_arguments)]
async fn handle_tool_call(
    app: &AppHandle,
    session_id: &str,
    assistant_msg_id: &str,
    tool_call_id: &str,
    tool_name: &str,
    args: &Value,
    specs: &[ToolSpec],
    ctx: &ToolCtx,
    disabled_tools: &[String],
) -> (String, String) {
    let state = app.state::<crate::AppState>();
    let now = chrono::Utc::now().to_rfc3339();
    let mut ev = ToolEvent {
        id: uuid::Uuid::new_v4().to_string(),
        message_id: assistant_msg_id.to_string(),
        tool_name: tool_name.to_string(),
        tool_call_id: Some(tool_call_id.to_string()),
        params: args.clone(),
        result_text: None,
        status: "running".into(),
        approval_scope: Some("none".into()),
        created_at: now,
        subprocess_id: None,
    };

    let spec = specs.iter().find(|s| s.name == tool_name);
    let Some(spec) = spec else {
        ev.status = "failed".into();
        let reason = if disabled_tools.iter().any(|t| t == tool_name) {
            format!("工具已在设置中禁用: {tool_name}")
        } else {
            format!("未知工具: {tool_name}")
        };
        ev.result_text = Some(reason);
        {
            let db = state.db.lock().unwrap();
            let _ = store::insert_tool_event(&db, &ev);
        }
        emit_tool(app, session_id, &ev);
        return (ev.status.clone(), ev.result_text.clone().unwrap_or_default());
    };
    let risk = spec.risk;

    // 未绑定工作区的会话为纯对话模式：文件/命令工具不可用（todo、generate_image 除外）
    if ctx.workspace.as_os_str().is_empty() && tool_name != "todo" && tool_name != "generate_image" {
        let text = "当前会话未绑定工作区，文件与命令工具不可用。请直接以文字回答用户，并提示：如需读写文件或执行命令，可在顶栏选择工作区目录后重试。".to_string();
        ev.status = "failed".into();
        ev.result_text = Some(text.clone());
        {
            let db = state.db.lock().unwrap();
            let _ = store::insert_tool_event(&db, &ev);
        }
        emit_tool(app, session_id, &ev);
        return ("failed".into(), text);
    }

    // 路径越界检测（只读工具越界也需审批）
    let path_arg = args.get("path").and_then(|p| p.as_str());
    let outside = path_arg.map(|p| !tools::inside_workspace(ctx, p)).unwrap_or(false);
    let mut already_inserted = false;

    // ---- 权限判定：访问模式 → 会话规则 → 风险级 → 审批 ----
    // 访问模式与会话规则均在此实时读取：对话进行中在顶栏改模式、审批时选「本会话允许」，
    // 都能对本次运行内后续的工具调用立即生效，不必等下一轮运行。
    let (full_access, session_rules) = {
        let db = state.db.lock().unwrap();
        let mode = store::get_session(&db, session_id)
            .ok()
            .flatten()
            .and_then(|s| s.access_mode)
            .unwrap_or_else(|| "confirm".into());
        (
            mode == "full_access",
            store::list_session_rules(&db, session_id).unwrap_or_default(),
        )
    };
    let mut scope: Option<&'static str> = None;
    let mut need_ask = false;
    let mut ask_risk = "write";
    let mut force_once = false;

    // 检查是否命中 .harness 目录全量读写白名单（系统记忆、方案、配置、自省目录免审放行）
    let is_harness_operation = {
        let is_harness_tool = matches!(
            tool_name,
            "create_plan" | "update_plan" | "switch_plan" | "read_plan" | "record_memory"
        );
        let is_harness_path = path_arg.map(|p| {
            let p_norm = p.replace('\\', "/");
            p_norm.starts_with(".harness/")
                || p_norm == ".harness"
                || p_norm.contains("/.harness/")
                || p_norm.ends_with("/.harness")
        }).unwrap_or(false);
        is_harness_tool || is_harness_path
    };

    if full_access && tool_name != "temp_merge" {
        scope = Some("mode");
    } else if is_harness_operation {
        // .harness 目录全量读写放行白名单（方案/记忆/治理系统），免去底层写审批
        scope = Some("harness_whitelist");
    } else {
        let high_danger =
            tool_name == "run_command" && args.get("command").and_then(|c| c.as_str()).map(tools::is_high_danger).unwrap_or(false);
        if high_danger {
            // 高危命令：强制逐次审批，不可记忆放行
            need_ask = true;
            ask_risk = "execute";
            force_once = true;
        } else if tool_name == "temp_merge" {
            // 合并写回用户原始目录：不可逆且影响范围超出沙箱，
            // 任何访问模式下都强制逐次审批，且不可记忆放行
            need_ask = true;
            ask_risk = "write";
            force_once = true;
        } else if session_rules
            .iter()
            .any(|r| approval::rule_matches(r, tool_name, args, ctx))
        {
            scope = Some("session");
        } else if risk == Risk::ReadOnly && !outside {
            scope = Some("none");
        } else {
            need_ask = true;
            ask_risk = if risk == Risk::Execute {
                "execute"
            } else if outside {
                "path"
            } else {
                "write"
            };
        }
    }

    if need_ask {
        ev.status = "pending_approval".into();
        ev.approval_scope = None;
        {
            let db = state.db.lock().unwrap();
            let _ = store::insert_tool_event(&db, &ev);
        }
        already_inserted = true;
        emit_tool(app, session_id, &ev);

        let preview = build_preview(tool_name, args);
        let req = ApprovalRequest {
            event_id: ev.id.clone(),
            session_id: session_id.to_string(),
            tool_name: tool_name.to_string(),
            params: args.clone(),
            risk: ask_risk.into(),
            preview,
            force_once,
        };
        let decision = approval::request_approval(app, &state, req).await;
        match decision {
            Decision::Deny(reason) => {
                crate::growth::trigger_reflection_on_denial(
                    app.clone(),
                    session_id.to_string(),
                    tool_name.to_string(),
                    args.clone(),
                    reason.clone(),
                );
                let text = format!(
                    "用户拒绝了该操作。{}",
                    reason.map(|r| format!("原因：{r}")).unwrap_or_default()
                );
                ev.status = "denied".into();
                ev.result_text = Some(text.clone());
                {
                    let db = state.db.lock().unwrap();
                    let _ = store::update_tool_event(&db, &ev.id, "denied", Some(&text), Some("denied"));
                }
                emit_tool(app, session_id, &ev);
                return ("denied".into(), text);
            }
            d => {
                ev.approval_scope = Some(d.scope_str().into());
                // 记忆放行规则：「本会话允许」把规则写入当前对话并落库，
                // 仅对该对话生效、重启后仍保留
                if let Decision::AllowSession = d {
                    let (kind, pattern) = approval::rule_for(tool_name, args);
                    if !pattern.is_empty() {
                        let rules = {
                            let db = state.db.lock().unwrap();
                            let _ = store::add_session_rule(&db, session_id, &kind, &pattern);
                            store::list_session_rules(&db, session_id).unwrap_or_default()
                        };
                        // 通知前端刷新该对话的规则列表
                        emit_session_rules(app, session_id, &rules);
                    }
                }
            }
        }
    }

    // ---- 执行 ----
    ev.status = "running".into();
    if ev.approval_scope.is_none() {
        ev.approval_scope = scope.map(|s| s.to_string());
    }
    {
        let db = state.db.lock().unwrap();
        if already_inserted {
            let _ = store::update_tool_event(&db, &ev.id, "running", None, ev.approval_scope.as_deref());
        } else {
            let _ = store::insert_tool_event(&db, &ev);
        }
    }
    emit_tool(app, session_id, &ev);

    let ev_id = ev.id.clone();
    let app2 = app.clone();
    let sid2 = session_id.to_string();
    let on_partial = move |line: &str| {
        let _ = tauri::Emitter::emit(
            &app2,
            "tool:output",
            json!({"sessionId": sid2, "eventId": ev_id, "line": line}),
        );
    };

    let started = Instant::now();
    let mut call_ctx = ctx.clone();
    call_ctx.event_id = Some(ev.id.clone());
    let exec = tools::execute(tool_name, args, &call_ctx, &on_partial).await;
    let elapsed = started.elapsed().as_millis() as u64;

    let (status, text) = match exec {
        Ok(t) => ("success".to_string(), t),
        Err(e) => ("failed".to_string(), format!("工具执行失败：{e}")),
    };
    let mut detected_sub_id: Option<String> = None;
    if (tool_name == "spawn_subprocess" || tool_name == "spawn_subagent") && status == "success" {
        for line in text.lines() {
            if line.contains("ID:") {
                let trimmed = line.trim();
                let after = if let Some(idx) = trimmed.find("ID:") {
                    &trimmed[idx + 3..]
                } else {
                    ""
                };
                let id_val = after.trim().trim_matches('`').trim();
                if !id_val.is_empty() {
                    detected_sub_id = Some(id_val.to_string());
                    ev.subprocess_id = Some(id_val.to_string());
                    if let Some(obj) = ev.params.as_object_mut() {
                        obj.insert("subprocess_id".into(), json!(id_val));
                    }
                    break;
                }
            }
        }
    }
    if matches!(tool_name, "create_plan" | "update_plan" | "switch_plan" | "read_plan") && status == "success" {
        let normalized = text.replace('\\', "/");
        if let Some(pos) = normalized.find(".harness/plans/") {
            let sub = &normalized[pos..];
            let end = sub.find(|c: char| c.is_whitespace() || c == '`' || c == ')' || c == '"' || c == '\'').unwrap_or(sub.len());
            let path_str = sub[..end].trim_end_matches('.');
            if let Some(obj) = ev.params.as_object_mut() {
                obj.insert("plan_file_path".into(), json!(path_str));
            }
        }
    }
    ev.status = status.clone();
    ev.result_text = Some(text.clone());
    {
        let db = state.db.lock().unwrap();
        // todo 工具：保存会话任务清单快照
        if tool_name == "todo" && status == "success" {
            let _ = store::set_kv(&db, session_id, "todos", &args.to_string());
        }
        let params_str = serde_json::to_string(&ev.params).ok();
        let _ = store::update_tool_event_full(
            &db,
            &ev.id,
            &status,
            Some(&text),
            None,
            detected_sub_id.as_deref().or(ev.subprocess_id.as_deref()),
            params_str.as_deref(),
        );
    }
    emit_tool_with(app, session_id, &ev, json!({ "elapsedMs": elapsed }));
    (status, text)
}

/// 对话正常完成交付时调用：将任务清单中仍处于 in_progress 的任务标记为 done 并持久化，同时广播更新
pub fn auto_finish_session_todos(state: &crate::AppState, app: &AppHandle, session_id: &str) {
    let db = state.db.lock().unwrap();
    let raw: Option<String> = store::get_kv(&db, session_id, "todos").ok().flatten();
    let Some(raw_str) = raw else { return };
    let Ok(mut val) = serde_json::from_str::<Value>(&raw_str) else { return };
    let Some(todos_arr) = val.get_mut("todos").and_then(|v| v.as_array_mut()) else { return };

    let mut changed = false;
    for item in todos_arr.iter_mut() {
        if item.get("status").and_then(|s| s.as_str()) == Some("in_progress") {
            item["status"] = json!("done");
            changed = true;
        }
    }
    if changed {
        let new_raw = val.to_string();
        let _ = store::set_kv(&db, session_id, "todos", &new_raw);
        if let Some(todos) = val.get("todos") {
            let _ = app.emit(
                "session:todos",
                json!({
                    "sessionId": session_id,
                    "todos": todos
                }),
            );
        }
    }
}

/// 用户手动停止会话时调用：将仍处于 in_progress 的任务重置为 pending 并持久化，同时广播更新
pub fn stop_session_todos(state: &crate::AppState, app: &AppHandle, session_id: &str) {
    let db = state.db.lock().unwrap();
    let raw: Option<String> = store::get_kv(&db, session_id, "todos").ok().flatten();
    let Some(raw_str) = raw else { return };
    let Ok(mut val) = serde_json::from_str::<Value>(&raw_str) else { return };
    let Some(todos_arr) = val.get_mut("todos").and_then(|v| v.as_array_mut()) else { return };

    let mut changed = false;
    for item in todos_arr.iter_mut() {
        if item.get("status").and_then(|s| s.as_str()) == Some("in_progress") {
            item["status"] = json!("pending");
            changed = true;
        }
    }
    if changed {
        let new_raw = val.to_string();
        let _ = store::set_kv(&db, session_id, "todos", &new_raw);
        if let Some(todos) = val.get("todos") {
            let _ = app.emit(
                "session:todos",
                json!({
                    "sessionId": session_id,
                    "todos": todos
                }),
            );
        }
    }
}

fn build_preview(tool_name: &str, args: &Value) -> String {
    match tool_name {
        "run_command" => args
            .get("command")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string(),
        "temp_merge" => {
            "把临时空间的全部变更写回各项目原目录（冲突走 AI 智能合并）。\
             该操作影响你的原始目录且不可自动撤销；存在无法自动合并的冲突文件时将列出清单供人工处理。"
                .to_string()
        }
        "temp_snapshot" => {
            let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("manual");
            format!("为临时空间当前状态建立备份快照（恢复点）：{label}")
        }
        "temp_restore" => {
            let snap = args
                .get("snapshot")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            match snap {
                Some(s) => format!(
                    "把临时空间全部项目副本恢复到快照「{s}」：丢弃快照之后的全部修改（不影响原目录）。"
                ),
                None => "把临时空间全部项目副本恢复到基线：丢弃全部未提交修改（不影响原目录）。".to_string(),
            }
        }
        "write_file" => {
            let path = args.get("path").and_then(|p| p.as_str()).unwrap_or("");
            let content = args.get("content").and_then(|c| c.as_str()).unwrap_or("");
            let d = crate::diffutil::diff_lines("", content, 4000);
            format!("{path}（新增 {} 行）\n{}", d.added, d.text)
        }
        "edit_file" => {
            let path = args.get("path").and_then(|p| p.as_str()).unwrap_or("");
            let old = args.get("old_string").and_then(|c| c.as_str()).unwrap_or("");
            let new = args.get("new_string").and_then(|c| c.as_str()).unwrap_or("");
            format!("{path}\n{}", crate::diffutil::diff_lines(old, new, 4000).text)
        }
        "generate_image" => {
            let prompt = args.get("prompt").and_then(|p| p.as_str()).unwrap_or("");
            let size = args.get("size").and_then(|s| s.as_str()).unwrap_or("1024x1024");
            let filename = args.get("filename").and_then(|f| f.as_str()).unwrap_or("自动生成路径");
            format!("根据提示词生成图片：\n提示词: {prompt}\n分辨率: {size}\n目标文件: {filename}")
        }
        "create_plan" => {
            let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("未命名计划");
            let goals = args.get("goals").and_then(|v| v.as_str()).unwrap_or("");
            let arch = args.get("architecture").and_then(|v| v.as_str()).unwrap_or("");
            format!("创建任务执行计划【{}】\n目标：{}\n架构设计：{}", title, goals, arch)
        }
        "update_plan" => {
            let reason = args.get("reason").and_then(|v| v.as_str()).unwrap_or("");
            let note = args.get("revision_note").and_then(|v| v.as_str()).unwrap_or("");
            format!("更新任务计划\n调整原因：{}\n变更备注：{}", reason, note)
        }
        _ => serde_json::to_string_pretty(args).unwrap_or_default(),
    }
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

/// 检测用户 Prompt 中是否包含“先不改动代码”的明确否定语义
fn contains_negative_code_intent(s: &str) -> bool {
    let text = s.to_lowercase();
    let keywords = [
        "先不改代码", "先别改代码", "暂时别改代码", "不要改代码", "暂不改动代码",
        "先出方案", "只出方案", "先不要改代码", "只给方案", "仅出方案", "先给出方案",
        "方案我看下", "方案我看看", "只评估", "仅评估", "先评估", "仅设计", "只设计",
        "don't edit code", "don't modify code", "plan only", "only plan",
    ];
    keywords.iter().any(|k| text.contains(k))
}

async fn check_and_trigger_compaction(
    app: &AppHandle,
    state: &crate::AppState,
    session_id: &str,
    cfg: &crate::llm::LlmCfg,
    sys: &str,
    token_limit: usize,
) -> bool {
    let (need_compact, candidate_msgs, candidate_tokens) = {
        let db = state.db.lock().unwrap();
        let compactions = store::list_session_compactions(&db, session_id).unwrap_or_default();
        let max_compacted_seq = compactions.iter().map(|c| c.end_seq).max().unwrap_or(0);
        let msgs = store::all_messages(&db, session_id).unwrap_or_default();
        let uncompacted: Vec<_> = msgs
            .into_iter()
            .filter(|m| !m.queued && m.seq > max_compacted_seq)
            .collect();

        // 寻找最后一个 user 消息（当前活跃轮次起点）
        let last_user_idx = uncompacted.iter().rposition(|m| m.role == "user");
        if let Some(idx) = last_user_idx {
            // 至少有 2 条往期消息（即至少存在 1 轮前序已完成的对话）
            if idx >= 2 {
                let candidate = uncompacted[..idx].to_vec();
                let tokens: usize = candidate
                    .iter()
                    .map(|m| {
                        estimate_tokens(m.content.as_deref().unwrap_or(""))
                            + m.tool_calls.as_ref().map(crate::models::estimate_value_tokens).unwrap_or(0)
                    })
                    .sum();
                let est_total = estimate_tokens(sys) + tokens;
                // 当总消耗达到上限的 75% 或候选历史消耗较大时触发自动压缩
                let trigger = est_total >= token_limit * 75 / 100 || tokens >= token_limit / 2;
                (trigger, candidate, tokens)
            } else {
                (false, vec![], 0)
            }
        } else {
            (false, vec![], 0)
        }
    };

    if !need_compact || candidate_msgs.is_empty() {
        return false;
    }

    // 格式化待压缩的历史对话文本
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

    let summary_prompt = format!(
        r#"你是一个专业的编码项目备忘助手。请将以下对话历史进行结构化精炼总结，提炼为一份高质量 Markdown 格式的上下文备忘录。
该备忘录将在后续对话中替换这段历史，帮助你与用户保持关键记忆。

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
        history_str
    );

    // 调用 LLM 生成初版 Markdown 摘要
    let summary_messages = vec![
        json!({"role": "system", "content": "你是一个严谨客观的技术项目总结助手，擅长提取代码开发对话中的关键事实。"}),
        json!({"role": "user", "content": summary_prompt}),
    ];

    let summary_res = crate::llm::chat_stream(
        cfg,
        &summary_messages,
        &[],
        |_| {},
        |_| {},
    )
    .await;

    let initial_summary = match summary_res {
        Ok(r) if !r.content.trim().is_empty() => r.content,
        _ => return false,
    };

    let event_id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = tokio::sync::oneshot::channel();

    let start_seq = candidate_msgs.first().map(|m| m.seq).unwrap_or(0);
    let end_seq = candidate_msgs.last().map(|m| m.seq).unwrap_or(0);
    let start_preview = truncate_preview(candidate_msgs.first().and_then(|m| m.content.as_deref()).unwrap_or(""), 40);
    let end_preview = truncate_preview(candidate_msgs.last().and_then(|m| m.content.as_deref()).unwrap_or(""), 40);

    let timeout_secs = 30u64;

    let req = crate::models::CompactionRequest {
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

    let _ = app.emit("compaction:request", &req);

    // 等待用户在前端查看、补充并确认（默认阻塞等待 30 秒；若 30 秒无操作，则自动应用压缩并继续后续流程）
    let decision = match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), rx).await {
        Ok(Ok(d)) => d,
        _ => {
            // 超时 30 秒无操作：从待处理 map 中移除，触发超时事件以通知前端（通知保持展示供查阅，但不提供继续执行按钮）
            state.compactions.lock().unwrap().remove(&event_id);
            let _ = app.emit(
                "compaction:timeout",
                &serde_json::json!({
                    "eventId": event_id,
                    "sessionId": session_id,
                    "autoApplied": true,
                }),
            );
            crate::models::CompactionDecision {
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
        let compaction = crate::models::SessionCompaction {
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
        true
    } else {
        false
    }
}

fn build_context(
    conn: &rusqlite::Connection,
    session_id: &str,
    system: &str,
    token_limit: usize,
) -> (Result<Vec<Value>, String>, usize, Option<crate::models::TruncationNotice>) {
    let msgs = match store::all_messages(conn, session_id) {
        Ok(m) => m,
        Err(e) => return (Err(e), 0, None),
    };
    let compactions = store::list_session_compactions(conn, session_id).unwrap_or_default();
    let max_compacted_seq = compactions.iter().map(|c| c.end_seq).max().unwrap_or(0);

    // tool_call_id -> 结果文本
    let mut tool_results: HashMap<String, String> = HashMap::new();
    for m in &msgs {
        if m.role == "tool" {
            if let Some(id) = &m.tool_call_id {
                tool_results.insert(id.clone(), m.content.clone().unwrap_or_default());
            }
        }
    }

    let mut out = vec![json!({"role": "system", "content": system})];
    let mut est = estimate_tokens(system);

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

    // 处理未被压缩的消息
    for m in &msgs {
        if m.queued || m.seq <= max_compacted_seq {
            continue; // 待执行列表中的消息以及已被压缩历史不作为原始消息进入上下文
        }
        match m.role.as_str() {
            "user" => {
                let mut text_content = m.content.clone().unwrap_or_default();
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
                match &m.tool_calls {
                    Some(tcs) if !tcs.is_null() && tcs.as_array().map(|a| !a.is_empty()).unwrap_or(false) => {
                        if !content.is_empty() {
                            obj["content"] = Value::String(content.clone());
                        } else {
                            obj["content"] = Value::Null;
                        }
                        obj["tool_calls"] = tcs.clone();
                        est += estimate_tokens(&content) + crate::models::estimate_value_tokens(tcs);
                        out.push(obj);

                        // 紧随其后补齐每个调用的 tool 结果（缺失则合成，避免协议错误）
                        if let Some(arr) = tcs.as_array() {
                            for tc in arr {
                                let id = tc.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string();
                                let text = tool_results
                                    .remove(&id)
                                    .unwrap_or_else(|| "(执行被中断，无结果)".into());
                                est += estimate_tokens(&text);
                                out.push(json!({"role": "tool", "tool_call_id": id, "content": text}));
                            }
                        }
                    }
                    _ => {
                        if !content.is_empty() {
                            obj["content"] = Value::String(content.clone());
                            est += estimate_tokens(&content);
                            out.push(obj);
                        }
                    }
                }
            }
            _ => {} // tool 消息已在上面消费；queued 消息跳过
        }
    }

    // ---- 动态上下文感知工具输出瘦身 (Dynamic Context-Aware Tool Result Slimming) ----
    // 治理策略：
    // 1. 安全水位（est <= 80% token_limit）：完全不折叠任何工具输出，保障 Agent 跨步骤探索（Gather -> Synthesize）的工作记忆。
    // 2. 超出水位（est > 80% token_limit）：自远及近，优先瘦身较早历史中 > 2048 字符的工具输出，保护最近工作记忆。
    // 3. 活跃轮次绝对保护：当前正在等待 LLM 消费的工具结果严禁折叠。
    let safe_threshold = token_limit * 80 / 100;
    if est > safe_threshold {
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

        // 标记当前正处于活跃消费状态的最后一组工具调用
        if let Some(last_g) = groups.last_mut() {
            if last_g.assistant_idx + last_g.tool_indices.len() + 1 == out.len() {
                last_g.is_active = true;
            }
        }

        // 保护最近的 2 组工具调用作为短期工作记忆（避免上一步刚读完、下一步就看不见）
        let recent_cutoff = groups.len().saturating_sub(2);

        // 阶段 1：自远及近遍历远期历史工具组
        for g in groups.iter().take(recent_cutoff) {
            for &t_idx in &g.tool_indices {
                if est <= safe_threshold {
                    break;
                }
                let text = out[t_idx].get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if text.len() > 2048 {
                    let safe_len = text.char_indices().nth(2048).map(|(i, _)| i).unwrap_or(text.len());
                    let slimmed = format!(
                        "{}\n\n...[历史工具输出过长（共 {} 字符），已折叠前序详情以节省上下文空间]...",
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

        // 阶段 2：若折叠完远期历史后仍然超过 token_limit，允许对「近期但非活跃」工具输出折叠（绝对保护 is_active）
        if est > token_limit {
            for g in groups.iter().skip(recent_cutoff) {
                if g.is_active {
                    continue;
                }
                for &t_idx in &g.tool_indices {
                    if est <= token_limit {
                        break;
                    }
                    let text = out[t_idx].get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    if text.len() > 2048 {
                        let safe_len = text.char_indices().nth(2048).map(|(i, _)| i).unwrap_or(text.len());
                        let slimmed = format!(
                            "{}\n\n...[历史工具输出过长（共 {} 字符），已折叠前序详情以节省上下文空间]...",
                            &text[..safe_len],
                            text.len()
                        );
                        let old_tokens = estimate_tokens(&text);
                        let new_tokens = estimate_tokens(&slimmed);
                        est = est.saturating_sub(old_tokens).saturating_add(new_tokens);
                        out[t_idx]["content"] = Value::String(slimmed);
                    }
                }
                if est <= token_limit {
                    break;
                }
            }
        }
    }

    // 轮次级原子截断防护（Turn-based Atomic Pruning）：
    // 当即使折叠与压缩后仍超限时，按「完整轮次（Turn）」进行成对丢弃，
    // 杜绝单条推进破坏 assistant(tool_calls) 与 tool 结果的成对关系导致 400 报错。
    let mut notice: Option<crate::models::TruncationNotice> = None;
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
            // 计算被剔除轮次的 token 消耗
            let dropped_tokens: usize = out[turn_start..turn_end]
                .iter()
                .map(|m| {
                    let c = m.get("content").and_then(|v| v.as_str()).unwrap_or("");
                    let tc = m.get("tool_calls").map(crate::models::estimate_value_tokens).unwrap_or(0);
                    estimate_tokens(c) + tc
                })
                .sum();
            dropped_turns += 1;
            dropped_messages += turn_end - turn_start;
            total_dropped_tokens += dropped_tokens;

            est = est.saturating_sub(dropped_tokens);
            out.drain(turn_start..turn_end);

            // 重新计算 user 索引
            user_indices = out
                .iter()
                .enumerate()
                .filter(|(idx, m)| *idx > 0 && m.get("role").and_then(|r| r.as_str()) == Some("user"))
                .map(|(idx, _)| idx)
                .collect();
        }

        if dropped_turns > 0 {
            notice = Some(crate::models::TruncationNotice {
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

    // 协议防御：上下文末尾不能以 assistant 消息结束。
    // 如果末尾是 assistant 消息（例如异常退出残留或历史数据异常），
    // OpenAI / Claude / DeepSeek 等接口会认为助手已经交付答复，从而直接返回空回复或协议报错。
    while let Some(last) = out.last() {
        if last.get("role").and_then(|r| r.as_str()) == Some("assistant") {
            out.pop();
        } else {
            break;
        }
    }

    (Ok(out), est, notice)
}

/// 关联条目解析结果：说明必带；对方项目能按路径解析到且已设置自身约束时一并带上
struct ResolvedLink {
    name: String,
    path: String,
    description: String,
    constraints: Option<String>,
}

/// 解析项目关联条目（只读按路径查找，不创建项目实体）
fn resolve_links(conn: &rusqlite::Connection, project_id: &str) -> Result<Vec<ResolvedLink>, String> {
    let mut out = Vec::new();
    for l in store::list_project_links(conn, project_id)? {
        let constraints = store::find_project_by_path(conn, &l.path)?
            .filter(|p| !p.constraints.trim().is_empty())
            .map(|p| p.constraints);
        out.push(ResolvedLink {
            name: store::dir_name_of(&l.path),
            path: l.path,
            description: l.description,
            constraints,
        });
    }
    Ok(out)
}

/// 组装注入 system prompt 的「项目约束 + 演进经验 + 关联项目」段；全部为空时返回 None（不追加）
fn build_project_section(
    conn: &rusqlite::Connection,
    project_id: &str,
) -> Result<Option<String>, String> {
    let proj = store::get_project(conn, project_id)?;
    let constraints = proj.as_ref().map(|p| p.constraints.clone()).unwrap_or_default();
    let sop_cmd = proj.as_ref().and_then(|p| {
        if p.sop_enabled {
            if let Some(cmd) = p.sop_verify_cmd.as_ref().filter(|s| !s.trim().is_empty()) {
                Some(cmd.clone())
            } else if let Some(path) = p.path.as_deref().filter(|s| !s.trim().is_empty()) {
                let (_, def_cmd) = crate::sop::detect_project_stack(std::path::Path::new(path));
                if !def_cmd.trim().is_empty() {
                    Some(def_cmd)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    });
    let links = resolve_links(conn, project_id)?;
    let growths = store::list_accepted_growths_for_project(conn, project_id).unwrap_or_default();
    if !growths.is_empty() {
        let ids: Vec<String> = growths.iter().map(|g| g.id.clone()).collect();
        let _ = store::increment_growth_applied_count(conn, &ids);
    }
    if constraints.trim().is_empty() && links.is_empty() && growths.is_empty() && sop_cmd.is_none() {
        return Ok(None);
    }
    Ok(Some(project_section(&constraints, &links, &growths, sop_cmd.as_deref())))
}

/// 纯文本组装（无 IO，便于单测）
fn project_section(
    constraints: &str,
    links: &[ResolvedLink],
    growths: &[GrowthItem],
    sop_cmd: Option<&str>,
) -> String {
    let mut s = String::new();
    if !constraints.trim().is_empty() {
        s.push_str("## 项目约束（必须严格遵守）\n");
        s.push_str(constraints.trim());
    }
    if let Some(cmd) = sop_cmd {
        if !s.is_empty() {
            s.push_str("\n\n");
        }
        s.push_str("## 交付自检规范（SOP，必须严格遵守）：\n");
        s.push_str(&format!("- 本项目已开启交付前自检命令：`{cmd}`\n"));
        s.push_str("- 当你使用 write_file 或 edit_file 新增或修改了代码文件后，系统将在向用户交付前自动执行该自检命令。\n");
        s.push_str("- 若自检失败，请根据错误输出进行排查和修复，直到自检通过后再交付。\n");
    }
    if !growths.is_empty() {
        if !s.is_empty() {
            s.push_str("\n\n");
        }
        s.push_str("## 项目演进经验（基于往期会话沉淀，必须严格遵守）：\n");
        for (idx, g) in growths.iter().enumerate() {
            let cat = match g.category.as_str() {
                "command_rule" => "命令规范",
                "code_style" => "代码规范",
                "build_test" => "构建测试",
                "pitfall" => "避坑防雷",
                "workflow" => "工作流",
                _ => &g.category,
            };
            s.push_str(&format!("{}. [{}] {}\n", idx + 1, cat, g.rule_content.trim()));
        }
    }
    if !links.is_empty() {
        if !s.is_empty() {
            s.push_str("\n\n");
        }
        s.push_str("## 关联项目\n本项目与以下项目存在关联；跨目录引用其代码或需要遵循其目录约定时，按对应条目执行：\n");
        for l in links {
            s.push_str(&format!("\n### {}（{}）\n", l.name, l.path));
            s.push_str(l.description.trim());
            s.push('\n');
            if let Some(c) = &l.constraints {
                s.push_str("\n该项目自身约束（同样必须遵守）：\n");
                s.push_str(c.trim());
                s.push('\n');
            }
        }
        // 优先级声明仅在存在本项目约束时有意义，避免悬空引用
        if !constraints.trim().is_empty() {
            s.push_str("\n**优先级**：关联项目的目录约定、说明及其自身约束，与上方「项目约束」冲突时，以关联项目为准。");
        }
    }
    s
}

fn system_prompt(
    session: &Session,
    project_section: Option<&str>,
    disabled_sops: &[String],
    plan_mode: &str,
) -> String {
    let os = if cfg!(windows) { "Windows" } else { "Unix-like" };
    let is_collab = session.session_type == "collaborator";
    let is_sub = is_collab || session.session_type == "subprocess" || session.session_type == "subagent" || session.parent_session_id.is_some();
    let is_sop_enabled = |name: &str| !disabled_sops.iter().any(|s| s == name);

    let base = if session.workspace_path.is_empty() {
        // 未绑定工作区：纯对话模式
        format!(
            r#"你是 harness_mini，一个谨慎、专业的编码助手。当前会话未绑定工作区，处于纯对话模式。

操作系统：{os}

工作规则：
1. 当前会话没有绑定本地目录，文件与命令类工具（read_file / write_file / edit_file / glob / grep / list_dir / run_command）均不可用，不要尝试调用；相关的问题直接以文字回答或给出建议代码。
2. 如果用户需要你实际读写文件、执行命令，请提示：在顶栏点击工作区按钮选择目录后再继续。
3. 全程使用简体中文与用户交流，回答准确、简洁。"#,
            os = os
        )
    } else if is_collab {
        let role = session.subagent_role.as_deref().unwrap_or("常驻专业协作者");
        let task = session.subagent_task.as_deref().unwrap_or("");
        let mut rules = Vec::new();
        let mut rule_num = 1;

        rules.push(format!("{rule_num}. 专注于履行你的专业角色定位【{role}】，根据用户或主进程指派的目标深入工作。当前工作区根目录为【{}】，所有工具均以该根目录为基准。", session.workspace_path));
        rule_num += 1;

        if session.subagent_role.as_deref() == Some("image_gen") {
            rules.push(format!("{rule_num}. 【AI 绘画与图像生成核心职责】：根据用户或主进程的要求，深入理解视觉意图并提炼出画面细节丰富的高质量提示词（包括画质、风格、构图、主体、光影），直接调用 `generate_image` 工具生成图片。图片生成完成后在回复中展示成果并简明汇报。"));
            rule_num += 1;
        }

        if is_sop_enabled("plan_first") {
            rules.push(format!("{rule_num}. 【方案先行与克制改动】：开始实际修改前，优先梳理实现思路。如果包含代码修改，必须先阅读原文并做最小化精确修改。"));
            rule_num += 1;
        }

        if is_sop_enabled("memory_distill") {
            rules.push(format!("{rule_num}. 【边读边记与强制沉淀 SOP】：阅读核心配置文件或深入探索代码后，及时调用 `record_memory` 沉淀高价值技术知识。"));
            rule_num += 1;
        }

        if is_sop_enabled("safe_code_edit") {
            rules.push(format!("{rule_num}. 修改文件前必须先用 read_file 读取相关内容，用 edit_file 做精确修改；新文件用 write_file。"));
            rule_num += 1;
            rules.push(format!("{rule_num}. 动手前先用 glob / grep / list_dir / file_outline 探索并理解代码结构。"));
            rule_num += 1;
            rules.push(format!("{rule_num}. 修改完成后，尽量用 run_command 运行构建或测试验证改动。"));
            rule_num += 1;
        } else {
            rules.push(format!("{rule_num}. 动手前先用 glob / grep / list_dir / file_outline 探索并理解代码结构。"));
            rule_num += 1;
        }

        if is_sop_enabled("surgical_code_reading") {
            rules.push(format!("{rule_num}. 【精益代码研读与克制探索 SOP（必须严格遵守）】：\n   - 探索业务与代码时遵循漏斗式递进：先用 glob 查目录骨架，对源码文件优先用 file_outline 提取结构大纲与行号，再用 grep 定位关键词；\n   - 严禁对大型文件进行多轮循环分页切片（offset_line）式逐行深挖；\n   - 深入阅读时以函数/局部逻辑为靶标，通过 offset_line 与 max_lines 精读最小必要上下文（建议 50~100 行）；修改前必须局部 read_file 确认原文，严禁盲猜。"));
            rule_num += 1;
        }

        rules.push(format!("{rule_num}. 你是一个常驻专家角色，支持与用户或主进程进行持续的多轮交流与迭代修改。"));
        rule_num += 1;

        rules.push(format!("{rule_num}. 【施工工人准则与计划遵从】：若主进程在简报中提供了明确的目标文件、函数锚点或蓝图，你必须严格遵从，严禁重新设计或随意改动接口签名；若发现原计划有隐患，请在交付回复中列出《计划调整建议》。"));
        rule_num += 1;

        rules.push(format!(
            "{rule_num}. 全程使用简体中文交流；每一轮交付成果时，请简明总结完成状态、改动文件及核心技术逻辑（严格控制在 250 字以内），以便系统自动增量汇总汇报给主进程；长篇分析系统会自动落盘为工件，切勿堆砌冗长流水账。"
        ));

        let rules_text = rules.join("\n");
        format!(
            r#"你是 harness_mini 项目中由用户配置的专业常驻协作者 (Collaborator)。
角色定位：{role}
职责设定：{task}
工作区根目录：{path}
操作系统：{os}

工作规则：
{rules_text}"#,
            role = role,
            task = task,
            path = session.workspace_path,
            os = os,
            rules_text = rules_text
        )
    } else if is_sub {
        let role = session.subagent_role.as_deref().unwrap_or("专职子任务协作 Agent");
        let task = session.subagent_task.as_deref().unwrap_or("");
        let mut rules = Vec::new();
        let mut rule_num = 1;

        rules.push(format!("{rule_num}. 专注于完成上述分配给你的专属任务。当前合法工作区根目录与主会话严格保持完全一致【{}】，所有工具（read_file / write_file / edit_file / glob / grep / list_dir / file_outline / run_command / record_memory / read_memory）均以该根目录为基准。若任务描述中指明了重点关注的子目录或模块，请优先在该目录下展开工作。", session.workspace_path));
        rule_num += 1;

        rules.push(format!("{rule_num}. 【施工工人准则与架构遵从（必须严格遵守）】：\n   - 若主进程在简报或计划中已为你提供了《实施蓝图》、拟定函数签名或已知目标文件与行号锚点，你作为实施工人，【严禁重新设计接口签名、擅自改动方法名或推翻既定架构】！\n   - 你的核心职责是：使用 read_file 对目标局部进行 30~50 行手术级精读确认原文，严格按照既定蓝图进行精确修改并运行测试验证；\n   - 若在执行中发现计划或方案存在重大隐患，请在最终交付总结中提出《计划调整建议》，交由主进程统筹处理。"));
        rule_num += 1;

        rules.push(format!("{rule_num}. 【靶标定位与零盲搜守则（必须严格遵守）】：\n   - 主进程已在简报中为你梳理了已知重点文件与上下文图钉；【严禁在全工程范围无目的地使用 glob / grep / list_dir 盲目漫游】；\n   - 直接切入目标文件所在目录或行号展开工作，以最小的步骤和 Token 成本收敛交付。"));
        rule_num += 1;

        rules.push(format!("{rule_num}. 【快速收敛与步数预算原则（必须严格遵守）】：\n   - 探索与调研类任务建议在 10~15 步内完成收敛并给出总结；\n   - 代码探索阶段优先使用 glob、file_outline 和 grep 进行模式匹配与宏观架构定位，辅以少量关键核心文件的阅读；严禁对大型代码文件进行无休止的分页切片（offset_line）式逐行深挖；\n   - 一旦掌握关键代码位置或排查到核心结论，必须立即收敛，直接整理并输出最终报告，禁止拖延死磕细节。"));
        rule_num += 1;

        rules.push(format!("{rule_num}. 【严格职责边界原则】：严格局限于分配的角色定位【{role}】和任务目标开展工作，严禁跨界反查工作区内与当前任务职责无关的其他模块（例如前端调研专注前端组件与接口，严禁反查后端源码/数据库，该部分由主进程统筹）。"));
        rule_num += 1;

        if is_sop_enabled("plan_first") {
            rules.push(format!("{rule_num}. 【方案先行与克制改动】：在开始实际修改前，优先梳理与分析实现思路。若是分析调研类任务，重点产出结构清晰的分析报告与沉淀；若包含代码修改，必须先阅读原文并做最小化精确修改。"));
            rule_num += 1;
        }

        if is_sop_enabled("memory_distill") {
            rules.push(format!("{rule_num}. 【边读边记与强制沉淀 SOP（必须严格遵守）】：当你阅读分析了核心配置文件（如 pom.xml、package.json、Cargo.toml 等）或深入探索了具体业务模块代码后，在任务交付前**必须至少调用一次 `record_memory` 工具**将核心成果提炼落盘（技术框架体系沉淀至 category: \"profile\"，具体业务链路/功能模块/排错分析沉淀至 category: \"digest\"，工程避坑点沉淀至 category: \"convention\"），严禁完成多步探索后直接文字汇报而不留存持久化记忆。"));
            rule_num += 1;
        }

        if is_sop_enabled("safe_code_edit") {
            rules.push(format!("{rule_num}. 修改文件前必须先用 read_file 读取相关内容，用 edit_file 做基于精确原文的最小化修改；新文件才用 write_file。"));
            rule_num += 1;
            rules.push(format!("{rule_num}. 动手前先用 glob / grep / list_dir / file_outline 探索并理解代码结构。"));
            rule_num += 1;
            rules.push(format!("{rule_num}. 修改完成后，尽量用 run_command 运行构建或测试来验证改动。"));
            rule_num += 1;
        } else {
            rules.push(format!("{rule_num}. 动手前先用 glob / grep / list_dir / file_outline 探索并理解代码结构。"));
            rule_num += 1;
        }

        if is_sop_enabled("surgical_code_reading") {
            rules.push(format!("{rule_num}. 【精益代码研读与克制探索 SOP（必须严格遵守）】：\n   - 探索业务与代码时遵循漏斗式递进：先用 glob 查目录骨架，对源码文件优先用 file_outline 提取结构大纲与行号，再用 grep 定位关键词；\n   - 严禁对大型代码文件进行无休止的分页切片（offset_line）式逐行深挖；\n   - 阅读范围必须收敛在最小必要上下文（建议 50~100 行）；修改前必须局部 read_file 确认原文，严禁盲猜。"));
            rule_num += 1;
        }

        rules.push(format!("{rule_num}. 严禁再次创建子 Agent，直接使用现有工具高效完成分配的目标。"));
        rule_num += 1;

        rules.push(format!(
            "{rule_num}. 全程使用简体中文与用户交流；任务完成后，最终回复必须严格按以下结构化格式汇报成果（四要素总结，字数严格控制在 250 字以内，以便父 Agent 汇聚决策）：\n   - 🎯 任务完成状态（全部完成 / 部分完成 / 遇到阻碍）\n   - 📝 改动的文件清单（请列出准确相对路径）\n   - 💡 核心交付结论与关键技术说明\n   - 🧪 构建与测试验证结果\n⚠️ 提示：长篇技术分析与排查过程系统会自动落地为工件文件，严禁在最终文本回复中输出冗长的思考流水账！"
        ));

        let rules_text = rules.join("\n");
        format!(
            r#"你是 harness_mini 派生出的专业协作子 Agent 进程。
角色定位：{role}
主 Agent 分配的专属任务：{task}
工作区根目录：{path}
操作系统：{os}

工作规则：
{rules_text}"#,
            role = role,
            task = task,
            path = session.workspace_path,
            os = os,
            rules_text = rules_text
        )
    } else {
        let mut rules = Vec::new();
        let mut rule_num = 1;

        if plan_mode == "always_plan" {
            rules.push(format!(
                "{rule_num}. 【严格规划先行与确认门禁准则（Always Plan 模式 - 强安全风控）】：\n   \
                 - 适用范围：凡是对项目进行较大改动（预计修改 3 个以上文件、架构重构、引入新架构或新增功能特性）：\n   \
                 - 阶段划分（严禁越界）：\n     \
                   1. 【只读调研】：仅允许使用只读工具（glob / grep / list_dir / file_outline / read_file）摸清代码逻辑；\n     \
                   2. 【生成计划】：必须先调用 `create_plan` 工具在 `.harness/plans/` 目录下生成结构化计划 MD 文档；\n     \
                   3. 【必须停止并等待确认】：**生成计划后，当前轮次严禁调用任何写文件或编辑代码的工具（write_file / edit_file）**！你必须在回复中简明扼要汇报方案核心、涉及文件与分步清单，并明确请用户审阅确认。只有在用户确认计划后，下一轮对话才可开始修改代码。\n   \
                 - 微小改动（如修改单文件拼写错误、纯问答解释）无需生成计划，可直接处理。"
            ));
            rule_num += 1;
        } else if plan_mode == "always_proceed" {
            rules.push(format!(
                "{rule_num}. 【规划先行与自动推进准则（Always Proceed 模式 - 敏捷高效）】：\n   \
                 - 适用范围：凡是对项目进行较大改动（预计修改 3 个以上文件、跨模块重构、新增功能特性）：\n   \
                 - 核心流程：\n     \
                   1. 【必须先有计划】：同样必须调用 `create_plan` 工具生成结构化计划 MD 文档落盘，确立任务锚点与分步 Checklist；\n     \
                   2. 【自动推进与意图判断】：生成计划后：\n       \
                      * 若用户在指令中明确说明了“先不改动代码 / 仅出方案 / 暂勿修改 / 先评估”，则生成计划后停止，严禁改动代码；\n       \
                      * 若用户没有明确说明先不改代码，**生成计划后无需停下来等待用户确认，直接连贯执行计划、调用 edit_file / write_file 编写代码**，按清单逐步推进直至完成并自检！\n   \
                 - 微小改动（如单文件微调、纯解释）可直接处理。"
            ));
            rule_num += 1;
        } else if is_sop_enabled("plan_first") {
            rules.push(format!(
                "{rule_num}. 【方案先行与任务计划中枢治理规范（防需求失真，必须严格遵守）】：\n   - 大改动物理落盘先行：凡是涉及 3 个以上文件修改、架构重构、新增功能模块或预计多轮交互的复杂任务，**严禁在没有计划文档的情况下直接编写/修改代码**。必须先调用 `create_plan` 工具在 `.harness/plans/` 目录下生成结构化计划 MD 文档（明确目标、技术架构、受影响文件清单、分步 Checklist 与验证策略），并向用户呈现确认。\n   - 需求变更优先更新计划：在多轮对话中，一旦用户提出需求调整、增加/减少特性或纠偏，**在改动代码前，必须首先调用 `update_plan` 工具同步更新计划文档内容与变更历史 (Revision History)**，彻底杜绝“口头答应却遗留旧代码/旧逻辑”的需求失真。\n   - 按清单逐步推进与结案：执行阶段严格以计划 Checklist 为基准推进，每完成一个关键步骤，调用 `update_plan` 推进步骤状态；全部任务完成并通过测试后，调用 `update_plan(status: \"completed\")` 结案归档并解除活动挂载。\n   - 多需求隔离与计划流转：在同一会话中开启全新不相关的独立任务时，调用 `create_plan` 开启新的独立计划（旧计划自动挂起，杜绝多任务杂糅）；若需回溯前序任务，调用 `switch_plan` 重新激活。"
            ));
            rule_num += 1;
        }

        if is_sop_enabled("memory_distill") {
            rules.push(format!(
                "{rule_num}. 【边读边记与强制沉淀 SOP（必须严格遵守）】：\n   - 工作区内置了长期认知记忆系统（存储于 `.harness/memory/`）。下方已自动载入本项目沉淀的技术大盘（profile.md）、工程规范（conventions.md）与重要碎记（digests）。\n   - 秒级召回优先：当用户询问“当前项目采用的技术”、“技术栈是什么”、“工程规范”等常见问题时，**优先基于已载入的持久化记忆秒级直接回答**，无需反复调用工具重新扫描全量代码；若发现信息有缺失或过期，再针对性读取少量文件并用 record_memory 补全。\n   - 强制沉淀检查点：凡是在当前任务中阅读了 2 个以上文件、深入分析了某个功能模块/流程/配置文件后，在给出最终答复前，**必须至少调用一次 `record_memory` 工具**将核心事实沉淀到工作区，严禁在多步阅读探索后只输出文字答复而不落盘留存：\n     * 独立业务模块/流程链路/排错分析/架构设计 -> **必须调用 `record_memory(category: \"digest\", title: \"...\", content: \"...\")` 固化为主题碎记**；\n     * 项目技术大盘与框架体系 -> 调用 `record_memory(category: \"profile\", ...)` 沉淀至 profile.md；\n     * 常用规范与避坑要点 -> 调用 `record_memory(category: \"convention\", ...)` 沉淀至 conventions.md。"
            ));
            rule_num += 1;
        }

        if is_sop_enabled("safe_code_edit") {
            rules.push(format!("{rule_num}. 修改文件前必须先用 read_file 读取相关内容，用 edit_file 做基于精确原文的最小化修改；新文件才用 write_file。"));
            rule_num += 1;
            rules.push(format!("{rule_num}. 动手前先用 glob / grep / list_dir / file_outline 探索并理解代码结构。"));
            rule_num += 1;
            rules.push(format!("{rule_num}. 修改完成后，尽量用 run_command 运行构建或测试来验证改动。"));
            rule_num += 1;
        } else {
            rules.push(format!("{rule_num}. 动手前先用 glob / grep / list_dir / file_outline 探索并理解代码结构。"));
            rule_num += 1;
        }

        if is_sop_enabled("surgical_code_reading") {
            rules.push(format!(
                "{rule_num}. 【精益代码研读与克制探索 SOP（必须严格遵守）】：\n   - 漏斗式探索原则：探索项目架构与业务逻辑时，优先使用 glob / list_dir 查看目录与配置文件，对源码文件优先调用 `file_outline` 获取结构骨架（类/接口/结构体/函数签名及行号），结合 `grep` 定位关键线索；严禁在未摸清线索前盲目通读业务源码。\n   - 严禁连续切片漫游：严禁对超过 300 行的大型文件进行无休止、多轮递增的分页切片（offset_line）式逐行深挖；若需了解具体实现，必须以函数/模块为靶标，通过 offset_line 与 max_lines 精读最小必要上下文（建议 50~100 行）。\n   - 修改前的精确性保障：克制阅读不等于凭空臆断。在调用 edit_file 前，必须通过局部 read_file 确认要替换的原文及行号，严禁在未读取目标代码的情况下凭记忆或猜测修改。"
            ));
            rule_num += 1;
        }

        rules.push(format!(
            "{rule_num}. 路径基准说明：所有相对路径默认相对于工作区根目录解析。当前工作区为主要开发上下文；若任务明确需要读取、比对或操作工作区外部文件（如系统配置、全局依赖或关联工程），请使用规范的绝对路径，系统会根据权限策略处理。"
        ));
        rule_num += 1;

        rules.push(format!("{rule_num}. 不要执行破坏性命令（如递归删除、格式化磁盘等），它们会被强制要求用户确认。"));
        rule_num += 1;

        if is_sop_enabled("todo_lifecycle") {
            rules.push(format!("{rule_num}. 接到多步任务时，先用 todo 工具列出计划，并随进展更新各项状态；在执行完最后一步、给出最终回复前，务必调用 todo 工具将已完成任务的状态更新为 done（切勿遗留 in_progress 状态）。"));
            rule_num += 1;
        }

        if is_sop_enabled("subagent_orchestration") {
            rules.push(format!(
                "{rule_num}. 【团队协作者优先委派原则、反过度委派与自决准则】：\n   \
                 - 角色定位：你作为总架构师与统筹协调者，拥有专属的常驻专家团队（见下方【可用项目协作者名录】）。\n   \
                 - 强制委派机制：在接收到用户指令后，你必须首先对照各协作者的【主进程调度触发规则】；只要当前任务命中了某位空闲协作者的专属职责（例如涉及画图/生图/Logo制作命中【AI 绘画师】、涉及PRD/需求分析命中【产品经理】、涉及前端界面开发命中【前端开发】等），你【必须无条件优先且立即调用 `dispatch_collaborator` 工具】将任务委派给该协作者，并在调用后紧接着调用 `wait_collaborators` 等待成果汇报！\n   \
                 - 严禁越俎代庖：当名录中存在对应领域的专职协作者时，严禁自行直接执行！让专职角色做专职的事，你专注把控全局架构与成果汇总验收；\n   \
                 - 临时子进程协作：面对需要多步深度探索排查、跨模块重构、或需反复执行编译单测自检闭环的重型任务，可调用 `spawn_subprocess` 派生临时子进程处理。派发时务必使用 `relevant_files` / `pinned_context` 传入已知靶标文件与上下文图钉，让子进程跳过无意义的盲目搜索；\n   \
                 - ⚠️【反过度委派原则与自决准则 (Anti-Over-Delegation)】：若你在方案规划中或脑海中已经精确定位了目标文件、具体行数和拟追加的代码/方法（即微小明确的就地修改，如仅改动 1~2 个文件），【严禁派生子进程】！直接由你在主会话中调用 `read_file` 确认后用 `edit_file` 一步修改完成，杜绝因过度派发产生的额外上下文与等待开销。"
            ));
            rule_num += 1;
        }

        rules.push(format!("{rule_num}. 全程使用简体中文与用户交流；最终回复简洁总结：做了什么、改了哪些文件、验证结果如何。"));

        let rules_text = rules.join("\n");
        format!(
            r#"你是 harness_mini，一个谨慎、专业的编码 Agent，运行在用户的本地工作区中。

工作区根目录：{path}
操作系统：{os}

工作规则：
{rules_text}"#,
            path = session.workspace_path,
            os = os,
            rules_text = rules_text
        )
    };
    match project_section {
        Some(sec) => format!("{base}\n\n{sec}"),
        None => base,
    }
}

fn emit_tool(app: &AppHandle, session_id: &str, ev: &ToolEvent) {
    let _ = app.emit(
        "tool:update",
        json!({"sessionId": session_id, "event": ev}),
    );
}

/// 广播某对话的审批规则列表（新增/删除规则时调用，前端据此刷新会话设置）
fn emit_session_rules(app: &AppHandle, session_id: &str, rules: &[ApprovalRule]) {
    let _ = app.emit(
        "session:rules",
        json!({"sessionId": session_id, "rules": rules}),
    );
}

fn emit_tool_with(app: &AppHandle, session_id: &str, ev: &ToolEvent, extra: Value) {
    let _ = app.emit(
        "tool:update",
        json!({"sessionId": session_id, "event": ev, "extra": extra}),
    );
}

fn emit_error(app: &AppHandle, session_id: &str, kind: &str, message: String) {
    // 1) 清理本次运行遗留的空 assistant 占位消息（避免留白）
    {
        let state = app.state::<crate::AppState>();
        let db = state.db.lock().unwrap();
          let _ = db.execute(
            "DELETE FROM messages WHERE session_id = ?1 AND role = 'assistant'
                 AND (content IS NULL OR content = '')
                 AND (reasoning IS NULL OR reasoning = '')
                 AND tool_calls_json IS NULL",
            rusqlite::params![session_id],
        );
        // 2) 错误以 system 消息持久化进对话流（前端渲染为错误卡片）
        if let Ok(m) = store::new_message(&db, session_id, "system", Some(format!("⚠️ {message}")), false) {
            drop(db);
            let _ = app.emit("message:final", &m);
        }
    }
    // 3) 同时推送 toast 事件
    let _ = app.emit(
        "error",
        json!({"sessionId": session_id, "kind": kind, "message": message}),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens() {
        assert!(estimate_tokens("hello world") > 0);
        assert!(estimate_tokens("你好世界") > 0);
        let cjk = estimate_tokens("一二三四五六七八九十");
        let ascii = estimate_tokens("abcdefghij");
        assert!(cjk > ascii);
    }

    #[test]
    fn test_context_dangling_tool_calls() {
        // 构造内存验证：assistant 有 tool_calls 但缺 tool 结果 → 自动合成
        let sys = "sys";
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        store::init_schema(&conn).unwrap();
        let s = store::create_session(&conn, ".", None, "t", "confirm").unwrap();
        let u = store::new_message(&conn, &s.id, "user", Some("hi".into()), false).unwrap();
        let a = store::new_message(&conn, &s.id, "assistant", Some(String::new()), false).unwrap();
        store::update_message_tool_calls(
            &conn,
            &a.id,
            &json!([{"id": "call_1", "type": "function", "function": {"name": "read_file", "arguments": "{}"}}]),
            "",
            None,
        )
        .unwrap();
        let _ = u;
        let (msgs, _, _) = build_context(&conn, &s.id, sys, 28000);
        let msgs = msgs.unwrap();
        // system + user + assistant(tool_calls) + 合成 tool
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[3]["role"], "tool");
        assert_eq!(msgs[3]["tool_call_id"], "call_1");
    }

    #[test]
    fn test_build_context_with_compactions() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        store::init_schema(&conn).unwrap();
        let s = store::create_session(&conn, ".", None, "test", "confirm").unwrap();

        // 创建 3 条历史消息
        let _ = store::new_message(&conn, &s.id, "user", Some("轮次1问题".into()), false).unwrap();
        let _ = store::new_message(&conn, &s.id, "assistant", Some("轮次1回答".into()), false).unwrap();
        let _ = store::new_message(&conn, &s.id, "user", Some("轮次2问题".into()), false).unwrap();

        // 插入压缩记录（压缩了 seq 1 到 2）
        let comp = crate::models::SessionCompaction {
            id: "comp_1".into(),
            session_id: s.id.clone(),
            start_seq: 1,
            end_seq: 2,
            summary_markdown: "### 核心目标\n开发功能A".into(),
            tokens_before: 500,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        store::create_session_compaction(&conn, &comp).unwrap();

        let (msgs, _, _) = build_context(&conn, &s.id, "sys prompt", 10000);
        let msgs = msgs.unwrap();

        // 应包含：
        // 0: system
        // 1: user (备忘录)
        // 2: assistant (确认备忘)
        // 3: user (轮次2问题)
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[1]["role"], "user");
        assert!(msgs[1]["content"].as_str().unwrap().contains("开发功能A"));
        assert_eq!(msgs[3]["content"].as_str().unwrap(), "轮次2问题");
    }

    #[test]
    fn test_atomic_turn_drop_no_orphaned_tools() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        store::init_schema(&conn).unwrap();
        let s = store::create_session(&conn, ".", None, "test", "confirm").unwrap();

        // 第一轮：带工具调用的复杂轮次
        let _ = store::new_message(&conn, &s.id, "user", Some("第一轮指令".into()), false).unwrap();
        let a1 = store::new_message(&conn, &s.id, "assistant", Some(String::new()), false).unwrap();
        store::update_message_tool_calls(
            &conn,
            &a1.id,
            &json!([{"id": "call_1", "type": "function", "function": {"name": "read_file", "arguments": "{}"}}]),
            "",
            None,
        ).unwrap();
        let t1 = store::new_message(&conn, &s.id, "tool", Some("很多文件内容xxxxxxxxxxxxxxx".into()), false).unwrap();
        store::set_message_tool_call_id(&conn, &t1.id, "call_1").unwrap();
        let _ = store::new_message(&conn, &s.id, "assistant", Some("读完了".into()), false).unwrap();

        // 第二轮：当前轮
        let _ = store::new_message(&conn, &s.id, "user", Some("第二轮最新指令".into()), false).unwrap();

        // 设定非常小的 token_limit（强制触发截断淘汰第一轮）
        let (msgs, _, notice) = build_context(&conn, &s.id, "sys", 30);
        let msgs = msgs.unwrap();

        // 验证截断通知
        assert!(notice.is_some());
        let n = notice.unwrap();
        assert_eq!(n.dropped_turns, 1);
        assert!(n.dropped_messages >= 3);
        assert!(n.dropped_tokens > 0);

        // 验证淘汰后：第一轮被成对淘汰，绝对不能遗留孤立的 tool 消息！
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[1]["role"], "user");
        assert_eq!(msgs[1]["content"].as_str().unwrap(), "第二轮最新指令");
        // 确保没有孤立的 tool 消息
        for (i, m) in msgs.iter().enumerate() {
            if m["role"] == "tool" {
                assert!(i > 0 && msgs[i - 1]["role"] == "assistant");
            }
        }
    }

    #[test]
    fn test_active_turn_tool_results_not_folded() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        store::init_schema(&conn).unwrap();
        let s = store::create_session(&conn, ".", None, "test", "confirm").unwrap();

        // 轮次 1：用户提问 + assistant 执行工具（大文本读取，例如 5000 字符）
        let _ = store::new_message(&conn, &s.id, "user", Some("请读取文件".into()), false).unwrap();
        let a1 = store::new_message(&conn, &s.id, "assistant", Some(String::new()), false).unwrap();
        store::update_message_tool_calls(
            &conn,
            &a1.id,
            &json!([{"id": "call_read", "type": "function", "function": {"name": "read_file", "arguments": "{\"path\": \"pom.xml\"}"}}]),
            "",
            None,
        ).unwrap();
        let big_content = "X".repeat(5000);
        let t1 = store::new_message(&conn, &s.id, "tool", Some(big_content.clone()), false).unwrap();
        store::set_message_tool_call_id(&conn, &t1.id, "call_read").unwrap();

        // 当处于当前活跃轮次（正等待消费工具结果）时，大输出绝对不能被折叠！
        let (msgs, _, _) = build_context(&conn, &s.id, "sys", 64000);
        let msgs = msgs.unwrap();
        assert_eq!(msgs.len(), 4); // sys, user, assistant, tool
        assert_eq!(msgs[3]["role"], "tool");
        let active_tool_content = msgs[3]["content"].as_str().unwrap();
        assert_eq!(active_tool_content.len(), 5000);
        assert_eq!(active_tool_content, big_content);

        // 轮次 1 完成：assistant 给出解答
        let _ = store::new_message(&conn, &s.id, "assistant", Some("分析完毕".into()), false).unwrap();
        // 进入轮次 2：用户提出新问题
        let _ = store::new_message(&conn, &s.id, "user", Some("下一步".into()), false).unwrap();

        // 当上下文水位充裕时（如 64000 空间），即使进入后续轮次也保持完整工作记忆，绝不误折叠！
        let (msgs2, _, _) = build_context(&conn, &s.id, "sys", 64000);
        let msgs2 = msgs2.unwrap();
        assert_eq!(msgs2.len(), 6); // sys, user, assistant(tc), tool, assistant(reply), user
        assert_eq!(msgs2[3]["role"], "tool");
        assert_eq!(msgs2[3]["content"].as_str().unwrap().len(), 5000);

        // 仅当上下文面临严重压力（设置较小 token_limit 触发 safe_threshold）时，才按需对远期历史实施折叠瘦身
        let (msgs3, _, _) = build_context(&conn, &s.id, "sys", 1000);
        let msgs3 = msgs3.unwrap();
        assert_eq!(msgs3[3]["role"], "tool");
        let hist_tool_content = msgs3[3]["content"].as_str().unwrap();
        assert!(hist_tool_content.contains("已折叠前序详情以节省上下文空间"));
        assert!(hist_tool_content.len() < 5000);
    }

    fn link(name: &str, path: &str, description: &str, constraints: Option<&str>) -> ResolvedLink {
        ResolvedLink {
            name: name.into(),
            path: path.into(),
            description: description.into(),
            constraints: constraints.map(|c| c.into()),
        }
    }

    #[test]
    fn project_section_contains_links_and_precedence() {
        let links = vec![link("libx", "D:\\libx", "依赖其接口", Some("用 pnpm"))];
        let s = project_section("用 npm", &links, &[], None);
        assert!(s.contains("用 npm"));
        assert!(s.contains("libx"));
        assert!(s.contains("D:\\libx"));
        assert!(s.contains("用 pnpm"));
        assert!(s.contains("以关联项目为准"));
    }

    #[test]
    fn project_section_skips_empty_parts() {
        // 无自身约束、仅关联 → 不出现约束标题，也不出现悬空的优先级声明
        let links = vec![link("tools", "D:\\c", "工具库", None)];
        let s = project_section("", &links, &[], None);
        assert!(!s.contains("## 项目约束"));
        assert!(!s.contains("优先级"));
        assert!(s.contains("## 关联项目"));

        // 仅约束、无关联 → 不出现关联段
        let s2 = project_section("规范", &[], &[], None);
        assert!(s2.starts_with("## 项目约束"));
        assert!(!s2.contains("关联项目"));

        // 两者皆空的判定在 build_project_section，这里验证空串等价
        assert_eq!(project_section("  ", &[], &[], None), "");
    }

    #[test]
    fn project_section_includes_growths() {
        let growth = GrowthItem {
            id: "g1".into(),
            project_id: Some("p1".into()),
            session_id: None,
            session_title: None,
            message_id: None,
            run_id: None,
            trigger_type: "user_rejection".into(),
            trigger_context: "".into(),
            reflection_thought: "".into(),
            category: "command_rule".into(),
            title: "清理规范".into(),
            rule_content: "- 使用 cargo clean".into(),
            status: "accepted".into(),
            applied_count: 1,
            created_at: "".into(),
            updated_at: "".into(),
        };
        let s = project_section("约束", &[], &[growth], None);
        assert!(s.contains("## 项目演进经验"));
        assert!(s.contains("[命令规范] - 使用 cargo clean"));
    }

    #[test]
    fn project_section_includes_sop() {
        let s = project_section("约束", &[], &[], Some("cargo test"));
        assert!(s.contains("## 交付自检规范"));
        assert!(s.contains("cargo test"));
    }

    #[test]
    fn system_prompt_appends_project_section() {
        let session = Session {
            id: "s".into(),
            title: "t".into(),
            workspace_path: "D:\\ws".into(),
            access_mode: None,
            project_id: Some("p".into()),
            status: "active".into(),
            last_message_at: None,
            created_at: String::new(),
            updated_at: String::new(),
            is_temp: false,
            temp_code: None,
            temp_root: None,
            source_workspace: None,
            merged_seq: None,
            merged_pending: false,
            completion_tokens: Some(0),
            prompt_tokens: Some(0),
            total_tokens: Some(0),
            parent_session_id: None,
            session_type: "root".into(),
            subagent_role: None,
            subagent_task: None,
            context_token_limit: None,
            last_reported_msg_id: None,
            auto_report: None,
            ..Default::default()
        };
        let with = system_prompt(&session, Some("## 项目约束\nX"), &[], "standard");
        assert!(with.contains("工作区根目录：D:\\ws"));
        assert!(with.ends_with("## 项目约束\nX"));
        let without = system_prompt(&session, None, &[], "standard");
        assert!(!without.contains("项目约束"));
    }

    #[test]
    fn system_prompt_respects_disabled_sops() {
        let session = Session {
            id: "s1".into(),
            title: "T".into(),
            workspace_path: "D:\\ws".into(),
            access_mode: None,
            project_id: Some("p1".into()),
            status: "active".into(),
            last_message_at: None,
            created_at: String::new(),
            updated_at: String::new(),
            is_temp: false,
            temp_code: None,
            temp_root: None,
            source_workspace: None,
            merged_seq: None,
            merged_pending: false,
            completion_tokens: Some(0),
            prompt_tokens: Some(0),
            total_tokens: Some(0),
            parent_session_id: None,
            session_type: "root".into(),
            subagent_role: None,
            subagent_task: None,
            context_token_limit: None,
            last_reported_msg_id: None,
            auto_report: None,
            ..Default::default()
        };
        let all_enabled = system_prompt(&session, None, &[], "standard");
        assert!(all_enabled.contains("方案先行"));
        assert!(all_enabled.contains("边读边记与强制沉淀 SOP"));
        assert!(all_enabled.contains("团队协作者优先委派原则"));
        assert!(all_enabled.contains("修改文件前必须先用 read_file 读取相关内容"));
        assert!(all_enabled.contains("精益代码研读与克制探索 SOP"));
        assert!(all_enabled.contains("todo 工具列出计划"));

        let disabled = vec![
            "plan_first".to_string(),
            "memory_distill".to_string(),
            "todo_lifecycle".to_string(),
            "safe_code_edit".to_string(),
            "surgical_code_reading".to_string(),
        ];
        let filtered = system_prompt(&session, None, &disabled, "standard");
        assert!(!filtered.contains("方案先行"));
        assert!(!filtered.contains("边读边记与强制沉淀 SOP"));
        assert!(!filtered.contains("todo 工具列出计划"));
        assert!(!filtered.contains("修改文件前必须先用 read_file 读取相关内容"));
        assert!(!filtered.contains("精益代码研读与克制探索 SOP"));
        assert!(filtered.contains("团队协作者优先委派原则"));
    }

    #[test]
    fn test_system_prompt_plan_mode() {
        let session = Session {
            workspace_path: "D:\\ws".into(),
            ..Default::default()
        };
        let prompt_plan = system_prompt(&session, None, &[], "always_plan");
        assert!(prompt_plan.contains("Always Plan 模式"));
        assert!(prompt_plan.contains("必须停止并等待确认"));

        let prompt_proc = system_prompt(&session, None, &[], "always_proceed");
        assert!(prompt_proc.contains("Always Proceed 模式"));
        assert!(prompt_proc.contains("自动推进与意图判断"));

        let prompt_std = system_prompt(&session, None, &[], "standard");
        assert!(!prompt_std.contains("Always Plan 模式"));
        assert!(!prompt_std.contains("Always Proceed 模式"));
    }

    #[test]
    fn test_negative_code_intent() {
        assert!(contains_negative_code_intent("先分析一下，先不要改代码"));
        assert!(contains_negative_code_intent("先帮我出方案，方案我看下"));
        assert!(contains_negative_code_intent("请 plan only，先评估"));
        assert!(!contains_negative_code_intent("帮我实现用户鉴权模块"));
        assert!(!contains_negative_code_intent("重构数据导出功能"));
    }

    #[test]
    fn build_project_section_resolves_linked_constraints() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        store::init_schema(&conn).unwrap();
        let a = store::create_project(&conn, "A", Some("D:\\a")).unwrap();
        let b = store::create_project(&conn, "B", Some("D:\\b")).unwrap();
        store::set_project_constraints(&conn, &b.id, "B 的规范").unwrap();
        store::add_project_link(&conn, &a.id, "D:\\b", "依赖 B").unwrap();
        store::add_project_link(&conn, &a.id, "D:\\ghost", "无实体目录").unwrap();

        // A 无自身约束 + 两个关联 → 只有关联段；B 的自身约束按路径解析注入，ghost 无实体则仅说明；
        // 优先级声明依赖本项目约束存在，此处不应出现
        let sec = build_project_section(&conn, &a.id).unwrap().unwrap();
        assert!(!sec.contains("## 项目约束"));
        assert!(!sec.contains("优先级"));
        assert!(sec.contains("B 的规范"));
        assert!(sec.contains("无实体目录"));

        // A 设置自身约束后，约束段在前，优先级声明出现
        store::set_project_constraints(&conn, &a.id, "A 的规范").unwrap();
        let sec = build_project_section(&conn, &a.id).unwrap().unwrap();
        assert!(sec.starts_with("## 项目约束（必须严格遵守）\nA 的规范"));
        assert!(sec.contains("以关联项目为准"));

        // 全空 → 不注入
        let c = store::create_project(&conn, "C", None).unwrap();
        assert!(build_project_section(&conn, &c.id).unwrap().is_none());
    }

    #[test]
    fn test_tool_error_tracker() {
        let mut tracker = ToolErrorTracker::new(2);
        assert_eq!(tracker.record_failure("edit_file"), (true, 1));
        assert_eq!(tracker.record_failure("edit_file"), (true, 2));
        assert_eq!(tracker.record_failure("edit_file"), (false, 3));

        tracker.record_success("edit_file");
        assert_eq!(tracker.record_failure("edit_file"), (true, 1));
    }

    #[test]
    fn test_build_tool_reflexion_prompt() {
        let p = build_tool_reflexion_prompt("edit_file", "未找到匹配文本", 1, 2);
        assert!(p.contains("未找到匹配文本"));
        assert!(p.contains("第 1/2 次尝试"));
        assert!(p.contains("read_file"));

        let p_cmd = build_tool_reflexion_prompt("run_command", "exit code 1", 2, 2);
        assert!(p_cmd.contains("stderr"));
    }
}
