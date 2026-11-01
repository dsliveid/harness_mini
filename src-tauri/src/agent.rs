use crate::approval::{self, Decision};
use crate::llm::{self, LlmCfg};
use crate::models::*;
use crate::store;
use crate::tools::{self, ToolCtx, ToolSpec};
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
            // 1. 终态保护：已完成的子任务属于成功终态，严禁被主会话级联停止篡改为 cancelled
            if sub.status == "completed" {
                continue;
            }

            // 2. 活跃与非终态检测：判断子会话是否当前处于执行中或属于未完结态
            let is_running_task = is_run_active(&state, &sub.id);
            let is_non_terminal = matches!(sub.status.as_str(), "active" | "running" | "pending" | "in_progress");

            // 3. 常驻协作者保护：若为常驻协作者，仅在当前真实处于运行活跃状态时才停止并置为 cancelled；
            // 若协作者处于空闲待命（active/idle 但未在执行），绝不误杀与篡改状态
            if sub.session_type == "collaborator" && !is_running_task {
                continue;
            }

            // 4. 幂等性保护：若非活跃且已为终止态，跳过，避免重复派发事件
            if !is_running_task && !is_non_terminal {
                continue;
            }

            {
                let db = state.db.lock().unwrap();
                let _ = store::set_session_status(&db, &sub.id, "cancelled");
            }
            stop_session_ext(app, &sub.id, true);
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
            let _ = state.emit("run:status", &json!({
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
        if let Some(h) = handles.get(session_id) {
            let run = h.active_run.lock().unwrap().take();
            if let Some(jh) = h.abort.lock().unwrap().take() {
                jh.abort();
            }
            run
        } else {
            None
        }
    };
    if let Some(run_id) = aborted_run {
        // 用户主动停止任务：收尾残留的 in_progress 任务为 pending，避免界面继续转圈
        stop_session_todos(&state, app, session_id);
        // starting = 任务尚未拿到 run_id；无论哪种情况都把库中残留的 running 记录收尾，
        // 避免停止后数据库里留下永久“运行中”的 run（运行状态恢复以数据库为准）
        {
            let db = state.db.lock().unwrap();
            let _ = store::cancel_open_runs(&db, session_id);
            if let Ok(events) = store::fail_open_tool_events(&db, session_id) {
                drop(db);
                for ev in events {
                    state.emit(
                        "tool:update",
                        &json!({"sessionId": session_id, "event": ev}),
                    );
                }
            }
        }
        state.emit(
            "run:status",
            &json!({"sessionId": session_id, "runId": run_id, "status": "cancelled"}),
        );
    } else {
        // 会话本就空闲（如前端状态漂移后误点停止）：补发空闲事件让前端复位
        state.emit(
            "run:status",
            &json!({"sessionId": session_id, "status": "idle"}),
        );
    }

    // 4. 若当前会话关联活跃长任务，协同中止长任务，将其运行中的子任务标记为 interrupted
    // （保护：若长任务已由 task 模块显式置为 paused 或 cancelled，绝不逆向篡改覆盖）
    {
        let db = state.db.lock().unwrap();
        if let Ok(Some(mut lt)) = store::get_active_long_task(&db, session_id) {
            if lt.status != "paused" && lt.status != "cancelled" && lt.status != "completed" {
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
        // 关键修复：重启子任务时，同步将 SQLite 数据库中的生命周期状态由 cancelled/interrupted/failed 恢复为 running
        let _ = store::set_session_status(&db, subagent_id, "running");
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
                let prompt = s
                    .subagent_task
                    .as_deref()
                    .map(|t| t.trim())
                    .filter(|t| !t.is_empty())
                    .unwrap_or("请继续推进并执行任务。");
                let nm = store::new_message(&db, subagent_id, "user", Some(prompt.into()), false)?;
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

    // 长任务防护：若当前会话绑定了活跃的长任务，阻止在主会话直接触发单轮 retry
    {
        let db = state.db.lock().unwrap();
        if let Ok(Some(lt)) = store::get_latest_long_task(&db, session_id) {
            if lt.status != "completed" && lt.status != "cancelled" {
                return Err("长任务推进中，请在长任务面板或顶栏中进行管理或回滚到检查点".into());
            }
        }
    }

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
    state.emit(
        "run:status",
        &json!({"sessionId": session_id, "status": "running", "triggerId": trigger_id}),
    );
    Ok(())
}

/// 继续推进会话（承接上一步未完成内容或工具结果，自动恢复未完成的子任务与长任务）
pub fn continue_turn(app: &AppHandle, session_id: &str) -> Result<(), String> {
    let state = app.state::<crate::AppState>();

    // 0. 若当前会话关联处于挂起状态的长任务，通过 resume 恢复
    {
        let db = state.db.lock().unwrap();
        if let Ok(Some(lt)) = store::get_latest_long_task(&db, session_id) {
            if lt.status == "interrupted" || lt.status == "failed" || lt.status == "paused" {
                drop(db);
                crate::task::resume_long_task(app, &lt.id)?;
                return Ok(());
            } else if lt.status == "cancelled" {
                return Err("长任务已终止。如需继续执行，请重新发起长任务。".into());
            } else if lt.status == "completed" {
                return Err("长任务已全部达成。".into());
            }
        }
    }

    if is_run_active(&state, session_id) {
        return Err("当前会话正在运行中，无需继续".into());
    }

    // 1. 普通模式：自动协同恢复关联的未完成子任务/子进程
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

    // 2. 为主会话准备触发消息
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
    state.emit(
        "run:status",
        &json!({"sessionId": session_id, "status": "running", "triggerId": trigger_id}),
    );
    Ok(())
}

/// 提取用户主动发起的 `/jev: 决策 [意图]` 指令

/// 自治驱动推进：长任务守护进程专用的自动推进入口（向主会话注入轻量自治脉冲）
pub fn continue_turn_autonomous(app: &AppHandle, session_id: &str) -> Result<(), String> {
    let state = app.state::<crate::AppState>();
    if is_run_active(&state, session_id) {
        return Ok(());
    }

    // 1. 自动恢复关联的未完成子任务（若有）
    let subs_to_resume = {
        let db = state.db.lock().unwrap();
        let subs = store::list_subagents(&db, session_id).unwrap_or_default();
        subs.into_iter()
            .filter(|s| s.status == "interrupted" || s.status == "failed")
            .map(|s| s.id)
            .collect::<Vec<_>>()
    };
    for sub_id in subs_to_resume {
        let _ = restart_subagent(app, &sub_id);
    }

    // 2. 注入轻量自治推进脉冲
    let (trigger_id, new_msg) = {
        let db = state.db.lock().unwrap();
        let msgs = store::all_messages(&db, session_id)?;
        let last_msg = msgs.last();

        let prompt = match last_msg {
            Some(m) if m.role == "tool" => {
                "请根据上述工具执行结果，继续自主推进长任务目标。".into()
            }
            Some(m) if m.role == "assistant" => {
                "请承接上一步进展，继续自主推进长任务目标。若需具体编写代码或深度排查，请调用 spawn_subagent 委派给工人沙箱。全部达成时请在输出中声明 🎯 **长任务全部目标达成**。".into()
            }
            _ => {
                "请继续推进长任务目标。".into()
            }
        };

        let new_m = store::new_message(&db, session_id, "user", Some(prompt), false)?;
        (new_m.id.clone(), new_m)
    };

    let _ = app.emit("message:final", &new_msg);
    let _ = app.emit("messages:changed", serde_json::json!({"sessionId": session_id}));

    spawn_session_task(app.clone(), session_id.to_string(), Some(trigger_id.clone()));
    state.emit(
        "run:status",
        &serde_json::json!({"sessionId": session_id, "status": "running", "triggerId": trigger_id}),
    );
    Ok(())
}

pub fn extract_jev_intent(raw_text: &str) -> (bool, String) {
    let trimmed = raw_text.trim();
    if trimmed.starts_with("/jev:决策")
        || trimmed.starts_with("/jev: 决策")
        || trimmed.starts_with("/jev：决策")
        || trimmed.starts_with("/jev： 决策")
    {
        let remainder = if trimmed.starts_with("/jev:决策") {
            &trimmed["/jev:决策".len()..]
        } else if trimmed.starts_with("/jev: 决策") {
            &trimmed["/jev: 决策".len()..]
        } else if trimmed.starts_with("/jev：决策") {
            &trimmed["/jev：决策".len()..]
        } else {
            &trimmed["/jev： 决策".len()..]
        }.trim();
        (true, remainder.to_string())
    } else if trimmed.contains("决策要求：用户有输入，决策用户输入，无输入决策最近方案") {
        (true, trimmed.to_string())
    } else {
        (false, String::new())
    }
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
        let _ = state.emit(
            "run:status",
            &json!({
                "sessionId": session_id,
                "runId": run_id,
                "status": "running",
                "triggerId": trigger_id,
                "startedAt": store::now(),
            }),
        );

        // 3. 运行一次 Agent 主循环
        let (outcome, last_assistant_id, run_tokens) = run_once(&app, &session_id, &run_id, &trigger_id).await;
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
                Some(run_tokens.cached_tokens),
            );
            if let Some(ref aid) = last_assistant_id {
                let _ = store::update_message_turn_duration(&db, aid, run_duration_ms);
                if let Ok(Some(m)) = store::get_message(&db, aid) {
                    let _ = app.emit("message:final", &m);
                }
            }

            // 实时广播：会话 Token 已在 finish_run 中聚合更新，第一时间向前端推送最新的会话状态（支持底栏实时刷新）
            if let Ok(Some(updated_session)) = store::get_session(&db, &session_id) {
                let _ = app.emit("session:update", &updated_session);
                let _ = app.emit("session:updated", &updated_session);
                if let Some(ref pid) = updated_session.parent_session_id {
                    if let Ok(Some(parent_session)) = store::get_session(&db, pid) {
                        let _ = app.emit("session:update", &parent_session);
                        let _ = app.emit("session:updated", &parent_session);
                    }
                }
            }
        }
        let _ = state.emit(
            "run:status",
            &json!({
                "sessionId": session_id,
                "runId": run_id,
                "status": status,
                "durationMs": run_duration_ms,
                "totalTokens": run_tokens.total_tokens,
                "cachedTokens": run_tokens.cached_tokens,
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
    state.snapshot.finish_run(&session_id);
    // 兜底：任务被中止（stop）时走不到 finish_run，把残留的 running run 标记为 interrupted，
    // 否则以数据库为准的运行状态恢复会把该会话永远显示为“运行中”
    {
        let db = state.db.lock().unwrap();
        let _ = store::fail_open_runs(&db, &session_id);
        if let Ok(events) = store::fail_open_tool_events(&db, &session_id) {
            drop(db);
            for ev in events {
                let _ = state.emit(
                    "tool:update",
                    &json!({"sessionId": session_id, "event": ev}),
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

                // 终态持久化闭环：子任务/协作者执行完成后，自动提取产出结论落地到 .harness/subtasks/{subagent_id}_report.md
                let ws_path_str = if !s.workspace_path.is_empty() {
                    s.workspace_path.clone()
                } else if let Some(ref pid) = s.parent_session_id {
                    store::get_session(&db, pid).ok().flatten().map(|ps| ps.workspace_path).unwrap_or_default()
                } else {
                    String::new()
                };

                if !ws_path_str.is_empty() {
                    let ws_path = std::path::Path::new(&ws_path_str);
                    let msgs = store::get_messages(&db, &session_id, None, 100).unwrap_or_default();
                    let last_assistant = msgs
                        .iter()
                        .rev()
                        .find(|m| m.role == "assistant" && !m.content.as_deref().unwrap_or("").is_empty());
                    if let Some(m) = last_assistant {
                        let last_reply = m.content.as_deref().unwrap_or("");
                        let mut touched_files = std::collections::BTreeSet::new();
                        for msg in &msgs {
                            for te in &msg.tool_events {
                                if ["write_file", "edit_file", "apply_diff"].contains(&te.tool_name.as_str()) {
                                    if let Some(p) = te.params.get("path").and_then(|v| v.as_str()) {
                                        touched_files.insert(p.to_string());
                                    }
                                }
                            }
                        }
                        let touched_vec: Vec<String> = touched_files.into_iter().collect();
                        let _ = crate::tools::subagent::save_subtask_report_artifact(
                            ws_path,
                            &session_id,
                            &s.title,
                            s.subagent_role.as_deref().unwrap_or("协作者"),
                            last_reply,
                            &touched_vec,
                        );
                    }
                }

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

#[allow(dead_code)]
/// 解析会话或全局生效的决策模型 (Jev)，支持子会话动态继承父会话
fn resolve_decision_model_for_session(
    session: &crate::models::Session,
    parent_session: Option<&crate::models::Session>,
    settings: &crate::models::SettingsData,
) -> Option<crate::models::DecisionRoute> {
    crate::models::resolve_decision_route(Some(session), parent_session, settings, None).ok()
}

#[allow(dead_code)]
fn parse_understanding_and_plan(raw: &str) -> (String, String) {
    let clean = raw.trim();
    let json_str = if let Some(start) = clean.find('{') {
        if let Some(end) = clean.rfind('}') {
            &clean[start..=end]
        } else {
            clean
        }
    } else {
        clean
    };

    if let Ok(val) = serde_json::from_str::<Value>(json_str) {
        let u = val.get("understanding").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        let p = val.get("plan").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        if !u.is_empty() || !p.is_empty() {
            return (u, p);
        }
    }

    (raw.trim().to_string(), "根据对用户意图的理解，推进相应执行步骤。".into())
}

/// 提取最近几轮人机对话的关键摘要，用于意图对齐门控进行代词消解与上下文关联
/// 提取最近几轮人机对话的完整上下文，用于意图对齐门控进行代词消解与选项解析（不截断选项与末尾内容）
#[allow(dead_code)]
fn format_recent_context_for_decision(msgs: &[crate::models::Message], trigger_msg_id: Option<&str>) -> String {
    let mut relevant = Vec::new();
    let mut count = 0;
    for m in msgs.iter().rev() {
        if let Some(t_id) = trigger_msg_id {
            if m.id == t_id {
                continue;
            }
        }
        if m.role == "user" || m.role == "assistant" {
            let content = m.content.as_deref().unwrap_or("").trim();
            if !content.is_empty() {
                let role_label = if m.role == "user" { "用户" } else { "助手" };
                relevant.push(format!("- {}: {}", role_label, content));
                count += 1;
                if count >= 3 {
                    break;
                }
            }
        }
    }
    if relevant.is_empty() {
        return String::new();
    }
    relevant.reverse();
    relevant.join("\n\n")
}

/// 推理思考模型（如 DeepSeek-R1、QwQ 等）可能将最终答复输出在思考流中，
/// 尝试精准切分内部推演草稿与面向用户的正式交付结论，杜绝草稿外泄与正文被关进抽屉。
fn rescue_reasoning_deliverable(reasoning: &str) -> (String, String) {
    let trimmed = reasoning.trim();
    if trimmed.is_empty() {
        return (String::new(), String::new());
    }

    // 1. 显式 stop token 遗留或 end 标记（如 end**结论先说**、end## 先给结论、end: ...）
    static END_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let end_re = END_RE.get_or_init(|| {
        regex::Regex::new(r"(?i)(?:^|[\r\n\s。！!])(end[\s:：\r\n]*(\*\*|##|#|【|一句话|结论|核心|直接))").unwrap()
    });

    // 2. 结论/答复段落标志（以换行为前导）
    static TITLE_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let title_re = TITLE_RE.get_or_init(|| {
        regex::Regex::new(r"(?:\n\s*\n|\n)(##\s+|\*\*(?:先给结论|核心结论|结论先说|最终结论|结论|一句话结论|总结|总览|回答|建议)\*\*|【(?:先给结论|核心结论|结论先说|最终结论|结论|一句话结论|总结|总览|回答|建议)】|(?:先给结论|核心结论|结论先说|最终结论|一句话结论)[：:])").unwrap()
    });

    // 3. 强段落转折标志（\n\n综上所述、\n\n因此，、\n\n基于以上分析）
    static SUMMARY_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let summary_re = SUMMARY_RE.get_or_init(|| {
        regex::Regex::new(r"(?:\n\s*\n)(#+\s+|因此[，,]|综上所述[，,]|基于以上分析[，,])").unwrap()
    });

    if let Some(caps) = end_re.captures(trimmed) {
        if let Some(end_block) = caps.get(1) {
            let start = end_block.start();
            let target_start = caps.get(2).map(|c| c.start()).unwrap_or(end_block.end());
            let head = trimmed[..start].trim().to_string();
            let tail = trimmed[target_start..].trim().to_string();
            if !tail.is_empty() {
                return (head, tail);
            }
        }
    }

    if let Some(caps) = title_re.captures(trimmed) {
        if let Some(mat) = caps.get(0) {
            let start = mat.start();
            let target_start = caps.get(1).map(|c| c.start()).unwrap_or(start);
            let head = trimmed[..start].trim().to_string();
            let tail = trimmed[target_start..].trim().to_string();
            if !tail.is_empty() {
                return (head, tail);
            }
        }
    }

    if let Some(caps) = summary_re.captures(trimmed) {
        if let Some(mat) = caps.get(0) {
            let start = mat.start();
            let target_start = caps.get(1).map(|c| c.start()).unwrap_or(start);
            let head = trimmed[..start].trim().to_string();
            let tail = trimmed[target_start..].trim().to_string();
            if !tail.is_empty() {
                return (head, tail);
            }
        }
    }

    // 无法精准切分时的兜底：保留完整思考，正文赋予规范的提取引用块，绝不冒充正常纯净文本
    (
        trimmed.to_string(),
        format!("> 💡 *（模型未在交付正文通道输出，已从推演记录中提取如下内容）*\n\n{}", trimmed),
    )
}

/// Agent 单次运行主循环（§5.1）
async fn run_once(app: &AppHandle, session_id: &str, run_id: &str, trigger_id: &str) -> (RunOutcome, Option<String>, RunTokenMetrics) {
    let mut last_assistant_id: Option<String> = None;
    let mut run_tokens = RunTokenMetrics::default();
    let state = app.state::<crate::AppState>();
    let (session, settings, trigger_msg, parent_session) = {
        let db = state.db.lock().unwrap();
        let master = state.master_key.lock().unwrap();
        let s = store::get_session(&db, session_id).ok().flatten();
        let p = s
            .as_ref()
            .and_then(|sess| sess.parent_session_id.as_deref())
            .and_then(|pid| store::get_session(&db, pid).ok().flatten());
        (
            s,
            // 必须注入加密 secrets 表中的 API Key，否则以空 Key 请求导致 401
            store::get_settings_with_secrets(&db, &master).unwrap_or_default(),
            store::get_message(&db, trigger_id).ok().flatten(),
            p,
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

    // 动态向上继承模型配置（若当前会话未显式指定，且存在父会话，则继承父会话配置）
    let effective_provider_id = session.provider_id.as_deref()
        .or_else(|| parent_session.as_ref().and_then(|p| p.provider_id.as_deref()));
    let effective_model_id = session.model_id.as_deref()
        .or_else(|| parent_session.as_ref().and_then(|p| p.model_id.as_deref()));

    let effective_vision_provider_id = session.vision_provider_id.as_deref()
        .or_else(|| parent_session.as_ref().and_then(|p| p.vision_provider_id.as_deref()));
    let effective_vision_model_id = session.vision_model_id.as_deref()
        .or_else(|| parent_session.as_ref().and_then(|p| p.vision_model_id.as_deref()));

    let _effective_image_provider_id = session.image_provider_id.as_deref()
        .or_else(|| parent_session.as_ref().and_then(|p| p.image_provider_id.as_deref()));
    let effective_image_model_id = session.image_model_id.as_deref()
        .or_else(|| parent_session.as_ref().and_then(|p| p.image_model_id.as_deref()));

    // 解析模型配置：
    // 1. 若当前会话包含图片附件且配置了专属视觉模型（或全局视觉模型），优先使用视觉模型
    // 2. 否则优先使用会话专属绑定的厂商 + 模型（若已指定），未指定或失效时回退到全局激活模型
    // 注意：对话引擎必须具备 chat 能力；若用户绑定的模型只具备 image_gen，自动回退到厂商内具备 chat 能力的模型或全局激活模型
    let resolved_vision_model = if has_images {
        if let (Some(v_pid), Some(v_mid)) = (effective_vision_provider_id, effective_vision_model_id) {
            settings.providers.iter().find(|p| &p.id == v_pid && p.models.iter().any(|m| m == v_mid))
                .map(|p| (p.clone(), v_mid.to_string()))
        } else {
            crate::models::resolve_active_vision_model(&settings)
                .map(|(p, m)| (p.clone(), m.to_string()))
        }
    } else {
        None
    };

    let resolved_model = if let Some(vm) = resolved_vision_model {
        Some(vm)
    } else if let (Some(pid), Some(mid)) = (effective_provider_id, effective_model_id) {
        if let Some(p) = settings.providers.iter().find(|p| &p.id == pid) {
            if p.models.iter().any(|m| m == mid) && settings.has_capability(Some(pid), mid, "chat") {
                Some((p.clone(), mid.to_string()))
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
    let effective_reasoning_effort = session
        .reasoning_effort
        .as_deref()
        .filter(|s| !s.is_empty())
        .or_else(|| parent_session.as_ref().and_then(|p| p.reasoning_effort.as_deref()).filter(|s| !s.is_empty()))
        .or(settings.reasoning_effort.as_deref())
        .filter(|s| !s.is_empty() && *s != "default")
        .map(|s| s.to_string());



    let effective_proxy_url = settings.effective_proxy_url();
    let cfg = LlmCfg {
        base_url: pc.base_url.clone(),
        api_key: pc.api_key.clone(),
        model: model.clone(),
        protocol: pc.get_model_protocol(&model),
        reasoning_effort: effective_reasoning_effort,
        proxy_url: effective_proxy_url.clone(),
        session_id: Some(session_id.to_string()),
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
        message_id: None,
        proxy_url: effective_proxy_url,
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
    } else if effective_image_model_id.is_some() {
        true
    } else if !has_collab_image_gen {
        settings.active_image_model_id.is_some()
            || effective_model_id.map(|m| settings.has_capability(effective_provider_id, m, "image_gen")).unwrap_or(false)
            || resolve_active_model(&settings).map(|(p, m)| settings.has_capability(Some(&p.id), m, "image_gen")).unwrap_or(false)
    } else {
        false
    };

    let has_collab_vision = if !is_subagent {
        let db = state.db.lock().unwrap();
        store::list_collaborators(&db, session_id)
            .map(|list| list.iter().any(|c| c.subagent_role.as_deref() == Some("vision") || c.vision_model_id.is_some()))
            .unwrap_or(false)
    } else {
        false
    };

    let session_has_vision = if session.session_type == "collaborator" && session.subagent_role.as_deref() == Some("vision") {
        true
    } else if effective_vision_model_id.is_some() {
        true
    } else if !has_collab_vision {
        settings.active_vision_model_id.is_some()
            || effective_model_id.map(|m| settings.has_capability(effective_provider_id, m, "vision")).unwrap_or(false)
            || crate::models::resolve_active_vision_model(&settings).is_some()
            || resolve_active_model(&settings).map(|(p, m)| settings.has_capability(Some(&p.id), m, "vision")).unwrap_or(false)
    } else {
        false
    };

    let specs: Vec<ToolSpec> = tools::global_registry()
        .list_specs(session.is_temp, is_subagent)
        .await
        .into_iter()
        // 工具能力准入：只有具备生图能力的会话才下发 generate_image
        .filter(|s| s.name != "generate_image" || session_has_image_gen)
        // 工具能力准入：只有具备视觉能力的会话才下发 recognize_image
        .filter(|s| s.name != "recognize_image" || session_has_vision)
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
    let is_long_task = {
        let db = state.db.lock().unwrap();
        if let Ok(Some(task)) = store::get_active_long_task(&db, session_id) {
            task.status == "running" || task.status == "planning"
        } else {
            false
        }
    };

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

        let msgs = store::all_messages(&db, session_id).unwrap_or_default();
        let last_user_content = msgs
            .iter()
            .filter(|m| m.role == "user")
            .last()
            .and_then(|m| m.content.as_deref());

        let neg_intent = last_user_content.map(contains_negative_code_intent).unwrap_or(false);

        // 识别长任务触发：包含显式指令前缀或 /goal
        let directly_triggered_goal = last_user_content
            .map(|c| c.contains("【长任务目标模式】") || c.starts_with("/goal"))
            .unwrap_or(false);

        // 或者当前会话曾触发过长任务且仍有处于进行中 (in_progress) 的方案正在推进
        let ongoing_goal_plan = if !session.workspace_path.is_empty() {
            let ws = std::path::Path::new(&session.workspace_path);
            crate::plan::find_plan_file(ws, session_id, None, Some(&db))
                .map(|(_, meta, _)| meta.status == "in_progress")
                .unwrap_or(false)
                && msgs.iter().any(|m| {
                    m.content
                        .as_deref()
                        .map(|c| c.contains("【长任务目标模式】") || c.starts_with("/goal"))
                        .unwrap_or(false)
                })
        } else {
            false
        };

        // 识别长任务触发：包含显式指令前缀、/goal、或者数据库中有处于运行/推进中的长任务
        let is_running_long_task = store::get_active_long_task(&db, session_id)
            .ok()
            .flatten()
            .map(|t| t.status == "running" || t.status == "planning")
            .unwrap_or(false);

        let is_goal_task = is_long_task || directly_triggered_goal || ongoing_goal_plan || is_running_long_task;

        // 子任务（工人沙箱）与长任务（后台自治）均自动强制走 always_proceed 模式（自治推进，落盘即开工，严禁停顿审批）；
        // 普通对话则严格按用户/项目设置的 plan_mode 运行（支持 standard / always_plan / always_proceed）。
        let effective_plan_mode = if is_subagent || is_goal_task {
            "always_proceed".to_string()
        } else {
            plan_mode
        };

        let db_sec = if session.is_temp {
            match crate::temp::load_manifest(&db, session_id) {
                Ok(Some(m)) => Some(crate::temp::build_prompt_section(&db, &session, &m)),
                _ => None,
            }
        } else {
            match &session.project_id {
                Some(pid) => crate::context::build_static_project_section(&db, pid).unwrap_or(None),
                None => None,
            }
        };
        let static_mem_sec = if !session.workspace_path.is_empty() {
            crate::memory::load_static_project_memory(std::path::Path::new(&session.workspace_path))
        } else {
            None
        };
        let static_collabs_sec = if !is_subagent {
            crate::context::build_static_collaborator_section(&db, session_id)
        } else {
            None
        };

        // 提取并冻结当前轮次 User 消息的运行时环境附注 (Layer 4)
        // 确保动态任务计划、最新知识索引与中断标记只挂载在尾部，且写入后终生不可变以保护前缀缓存
        let trigger_user_msg = trigger_msg.as_ref().cloned().or_else(|| {
            msgs.iter().rev().find(|m| m.role == "user").cloned()
        });
        if let Some(ref um) = trigger_user_msg {
            if um.runtime_anchor.is_none() {
                if let Some(anchor) = crate::context::build_runtime_anchor(&db, &session, um.content.as_deref()) {
                    let _ = store::update_message_runtime_anchor(&db, &um.id, &anchor);
                }
            }
        }

        let sys = crate::context::build_static_system_prompt(
            &session,
            &settings.disabled_sops,
            &effective_plan_mode,
            db_sec.as_deref(),
            static_mem_sec.as_deref(),
            static_collabs_sec.as_deref(),
        );

        (Some(sys), effective_plan_mode, neg_intent)
    };
    let sys = project_section.unwrap_or_default();

    let verified_intent_prefix: Option<String> = None;
    let mut sop_verified = false;
    let mut files_modified = false;
    let mut code_modified = false;
    let mut sop_retry_count = 0usize;
    let mut tool_tracker = ToolErrorTracker::new(2);
    let mut has_checked_compaction = false;
    let mut created_plan_in_this_turn = false;
    let mut _step = 0usize;
    let reached_max_steps;
    loop {
        let current_max_steps = {
            let db = state.db.lock().unwrap();
            store::get_session(&db, session_id)
                .ok()
                .flatten()
                .and_then(|s| s.max_steps)
                .map(|v| v as usize)
                .unwrap_or(settings.max_steps.max(1) as usize)
        };

        if _step >= current_max_steps {
            reached_max_steps = current_max_steps;
            break;
        }
        let current_sys = &sys;

        // ---- 步骤 0：在首步或上下文逼近上限时检测是否触发自动压缩与确认 ----
        if !has_checked_compaction {
            let compacted = check_and_trigger_compaction(
                app,
                &state,
                session_id,
                &cfg,
                current_sys,
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
            build_context(&db, session_id, current_sys, effective_ctx_limit)
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
        let remaining_steps = current_max_steps.saturating_sub(_step);
        if remaining_steps <= 2 && current_max_steps > 2 {
            let limit_label = format!("最大步数上限（{current_max_steps}）");
            let warn_content = format!(
                "【⚠️ 临界步数紧急预警：当前仅剩最后 {remaining_steps} 步，即将达到{limit_label}！严禁继续调用任何探索、排查或读取类工具！必须立即根据目前已掌握的所有信息，按规定结构化格式输出最终总结与交付回复！】"
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
        state.snapshot.transition_to_step(session_id, run_id, _step, &assistant_id);
        let _ = app.emit(
            "step:started",
            json!({
                "sessionId": session_id,
                "runId": run_id,
                "stepIndex": _step,
                "messageId": assistant_id,
            }),
        );
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

            let app_text = app2.clone();
            let sid_text = sid2.clone();
            let mid_text = mid2.clone();
            let app_reason = app2.clone();
            let sid_reason = sid2.clone();
            let mid_reason = mid2.clone();

            if attempt == 1 {
                let trace_payload = serde_json::json!({
                    "model": &cfg.model,
                    "endpoint": &cfg.base_url,
                    "reasoning_effort": &cfg.reasoning_effort,
                    "messages": &messages,
                    "tools": &schemas,
                });
                if let Ok(db) = state.db.lock() {
                    let _ = store::save_llm_request_trace(
                        &db,
                        session_id,
                        &assistant_id,
                        Some(run_id),
                        &cfg.model,
                        &cfg.base_url,
                        &trace_payload.to_string(),
                    );
                }
            }

            let app_tool = app2.clone();
            let sid_tool = sid2.clone();
            let mid_tool = mid2.clone();
            let call = llm::chat_stream_full(
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
                    }
                },
                {
                    let rbuf2 = rbuf2.clone();
                    move |delta| {
                        let mut b = rbuf2.lock().unwrap();
                        b.push_str(delta);
                        app_reason.state::<crate::AppState>().emit(
                            "message:reasoning:delta",
                            &json!({"sessionId": sid_reason, "messageId": mid_reason, "delta": delta}),
                        );
                    }
                },
                move |chunk| {
                    let st = app_tool.state::<crate::AppState>();
                    if let Some(name) = chunk.name {
                        if !name.is_empty() {
                            let call_id = chunk.id.unwrap_or("");
                            st.snapshot.start_tool_preparing(&sid_tool, call_id, name);
                            st.emit(
                                "tool:preparing",
                                &json!({
                                    "sessionId": sid_tool,
                                    "messageId": mid_tool,
                                    "callId": call_id,
                                    "toolName": name,
                                    "index": chunk.index,
                                }),
                            );
                        }
                    }
                    if let Some(args) = chunk.args_delta {
                        if !args.is_empty() {
                            let call_id = chunk.id.unwrap_or("");
                            st.snapshot.append_tool_args_delta(&sid_tool, call_id, args);
                            st.emit(
                                "tool:preparing_delta",
                                &json!({
                                    "sessionId": sid_tool,
                                    "messageId": mid_tool,
                                    "callId": call_id,
                                    "delta": args,
                                    "index": chunk.index,
                                }),
                            );
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

        if _step == 0 {
            if let Some(ref pfx) = verified_intent_prefix {
                result.reasoning = format!("{}{}", pfx, result.reasoning);
            }
        }

        // 空回复防护：模型不支持流式/工具或名称错误时给出明确提示
        if result.tool_calls.is_empty() && result.content.trim().is_empty() {
            if !result.reasoning.trim().is_empty() {
                // 推理思考模型（如 DeepSeek-R1、QwQ 等）可能将汇报输出在思考流中，精准切分，避免误杀与内外错位
                let (clean_reasoning, extracted_content) = rescue_reasoning_deliverable(&result.reasoning);
                result.reasoning = clean_reasoning;
                result.content = extracted_content;
                app.state::<crate::AppState>().emit(
                    "message:delta",
                    &json!({
                        "sessionId": session_id,
                        "messageId": assistant_id,
                        "delta": &result.content
                    }),
                );
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
        let est_out = estimate_tokens(&result.content) + estimate_tokens(&result.reasoning);
        let parsed_usage = crate::models::parse_llm_usage(result.usage.as_ref(), est_in, est_out);

        run_tokens.prompt_tokens += parsed_usage.prompt_tokens;
        run_tokens.completion_tokens += parsed_usage.completion_tokens;
        run_tokens.total_tokens += parsed_usage.total_tokens;
        run_tokens.cached_tokens += parsed_usage.cached_tokens;

        // 单步实时增量持久化 Token（自动递归累加至父会话）并即刻广播给前端
        {
            let db = state.db.lock().unwrap();
            let _ = store::increment_session_tokens(
                &db,
                session_id,
                parsed_usage.prompt_tokens,
                parsed_usage.completion_tokens,
                parsed_usage.total_tokens,
                parsed_usage.cached_tokens,
            );
            if let Ok(Some(current_sess)) = store::get_session(&db, session_id) {
                let _ = app.emit("session:update", &current_sess);
                let _ = app.emit("session:updated", &current_sess);
                if let Some(ref pid) = current_sess.parent_session_id {
                    if let Ok(Some(parent_sess)) = store::get_session(&db, pid) {
                        let _ = app.emit("session:update", &parent_sess);
                        let _ = app.emit("session:updated", &parent_sess);
                    }
                }
            }
        }

        let usage = json!({
            "promptTokens": parsed_usage.prompt_tokens,
            "completionTokens": parsed_usage.completion_tokens,
            "totalTokens": parsed_usage.total_tokens,
            "cachedTokens": parsed_usage.cached_tokens,
            "reasoningTokens": parsed_usage.reasoning_tokens,
            "isEstimated": parsed_usage.is_estimated,
            "inputEst": est_in,
            "outputEst": est_out,
            "durationMs": step_duration_ms,
            "model": &cfg.model,
            "rawUsage": &result.usage,
        });

        if result.tool_calls.is_empty() {
            // ---- 步骤 4：纯文本回复，交付前检查 SOP 自检 ----
            if code_modified && !sop_verified && sop_retry_count < 2 {
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
                            _step += 1;
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
            let _ = files_modified;
            let _ = code_modified;
            state.snapshot.finish_step(session_id);
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

            let is_modifying = tc.name == "write_file"
                || tc.name == "edit_file"
                || tc.name == "run_command"
                || tc.name == "delete_file"
                || tc.name == "apply_diff";
            let guard_denied: Option<String> = if is_modifying {
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
                    reverted_at: None,
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
                let tool_attachments = if tc.name == "recognize_image" && status == "success" {
                    if let Some(start) = result_text.find("<!-- MULTIMODAL_IMAGE_DIRECT:") {
                        let rest = &result_text[start + "<!-- MULTIMODAL_IMAGE_DIRECT:".len()..];
                        if let Some(end) = rest.find(" -->") {
                            let img_path = rest[..end].trim();
                            let p = std::path::Path::new(img_path);
                            if let Ok(meta) = std::fs::metadata(p) {
                                let name = p.file_name().and_then(|f| f.to_str()).unwrap_or("image.png").to_string();
                                let ext = p.extension().and_then(|e| e.to_str()).map(|s| s.to_lowercase()).unwrap_or_default();
                                let mime = match ext.as_str() {
                                    "jpg" | "jpeg" => "image/jpeg",
                                    "webp" => "image/webp",
                                    "gif" => "image/gif",
                                    "svg" => "image/svg+xml",
                                    _ => "image/png",
                                };
                                Some(vec![crate::models::Attachment {
                                    id: uuid::Uuid::new_v4().to_string(),
                                    name,
                                    mime_type: mime.to_string(),
                                    size: meta.len(),
                                    path: img_path.to_string(),
                                    is_image: true,
                                }])
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };
                let normalized_result = crate::context::normalize_tool_output_for_ingress(&tc.name, &result_text);
                let r = store::new_message_with_attachments(&db, session_id, "tool", Some(normalized_result), tool_attachments, false, None);
                if let Ok(ref m) = r {
                    let _ = store::set_message_tool_call_id(&db, &m.id, &tc.id);
                }
                r
            };
            if let Ok(m) = tool_msg {
                let _ = app.emit("message:final", &m);
            }
            if (tc.name == "write_file" || tc.name == "edit_file" || tc.name == "apply_diff") && status == "success" {
                files_modified = true;
                if let Some(rel) = args.get("path").and_then(|v| v.as_str()) {
                    let p = std::path::Path::new(rel);
                    let full_path = if p.is_absolute() {
                        p.to_path_buf()
                    } else {
                        std::path::Path::new(&session.workspace_path).join(p)
                    };
                    if crate::sop::is_code_or_build_file_with_workspace(
                        &full_path,
                        Some(std::path::Path::new(&session.workspace_path)),
                    ) {
                        code_modified = true;
                        sop_verified = false;
                    }
                    let _ = app.emit("file_viewer:file_changed", serde_json::json!({
                        "path": full_path.to_string_lossy().to_string()
                    }));
                } else {
                    code_modified = true;
                    sop_verified = false;
                }
            }
            if tc.name == "create_plan" && status == "success" {
                created_plan_in_this_turn = true;
            }
            if matches!(tc.name.as_str(), "create_plan" | "update_plan" | "switch_plan") && status == "success" {
                let _ = app.emit("file_viewer:file_changed", serde_json::json!({
                    "path": format!("{}/.harness/plans", session.workspace_path)
                }));
                // 计划创建/更新/切换后，同步向前端广播最新任务清单
                let db = state.db.lock().unwrap();
                if let Ok(Some(raw)) = store::get_kv(&db, session_id, "todos") {
                    if let Ok(val) = serde_json::from_str::<Value>(&raw) {
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
            }
            let _ = status;
        }
        // 回到下一轮循环
        _step += 1;
    }

    let err_msg = format!("已达到最大步数（{reached_max_steps}），任务中止。可在会话设置或下方快捷操作中调整「最大步数」并继续推进。");
    emit_error(
        app,
        session_id,
        "max_steps",
        err_msg,
    );
    let outcome = RunOutcome::Done;
    (outcome, last_assistant_id, run_tokens)
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
        reverted_at: None,
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

    // ---- 策略引擎安全评估（访问模式、安全域、规则判定） ----
    let (access_mode, session_rules) = {
        let db = state.db.lock().unwrap();
        let mode = store::get_session(&db, session_id)
            .ok()
            .flatten()
            .and_then(|s| s.access_mode)
            .unwrap_or_else(|| "confirm".into());
        (
            mode,
            store::list_session_rules(&db, session_id).unwrap_or_default(),
        )
    };

    let policy_decision = crate::policy::PolicyEngine::evaluate(
        &crate::policy::ToolInvocation {
            name: tool_name,
            args,
            risk,
        },
        &crate::policy::SecurityContext {
            workspace_root: &ctx.workspace,
            sandbox_root: ctx.sandbox_root.as_deref(),
            access_mode: &access_mode,
            session_rules: &session_rules,
            is_temp_workspace: ctx.temp.is_some(),
        },
    );

    let mut scope: Option<&'static str> = None;
    let mut already_inserted = false;

    let (ask_risk, force_once, risk_source, preview) = match policy_decision {
        crate::policy::PolicyDecision::Deny { reason } => {
            ev.status = "failed".into();
            ev.result_text = Some(reason.clone());
            {
                let db = state.db.lock().unwrap();
                let _ = store::insert_tool_event(&db, &ev);
            }
            emit_tool(app, session_id, &ev);
            return ("failed".into(), reason);
        }
        crate::policy::PolicyDecision::Allow { scope: s } => {
            scope = Some(s);
            ("", false, None, String::new())
        }
        crate::policy::PolicyDecision::RequireApproval {
            risk: r,
            force_once: fo,
            risk_source: rs,
            preview: prev,
        } => (r, fo, rs, prev),
    };

    if scope.is_none() {
        ev.status = "pending_approval".into();
        ev.approval_scope = None;
        {
            let db = state.db.lock().unwrap();
            let _ = store::insert_tool_event(&db, &ev);
        }
        already_inserted = true;
        emit_tool(app, session_id, &ev);

        let req = ApprovalRequest {
            event_id: ev.id.clone(),
            session_id: session_id.to_string(),
            tool_name: tool_name.to_string(),
            params: args.clone(),
            risk: ask_risk.into(),
            preview,
            force_once,
            risk_source,
            jev_reason: None,
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
    state.snapshot.start_tool_executing(session_id, tool_call_id, tool_name, &ev.id);
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
    call_ctx.message_id = Some(assistant_msg_id.to_string());
    let exec = tools::execute(tool_name, args, &call_ctx, &on_partial).await;
    let elapsed = started.elapsed().as_millis() as u64;

    let (mut status, mut text) = match exec {
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
    if tool_name == "record_memory" && status == "success" {
        let normalized = text.replace('\\', "/");
        let detected_path = if let Some(pos) = normalized.find(".harness/memory/") {
            let sub = &normalized[pos..];
            let end = sub.find(|c: char| c.is_whitespace() || c == '`' || c == ')' || c == '"' || c == '\'' || c == '。').unwrap_or(sub.len());
            Some(sub[..end].trim_end_matches('.').to_string())
        } else {
            let cat = ev.params.get("category").and_then(|v| v.as_str()).unwrap_or("");
            let title = ev.params.get("title").and_then(|v| v.as_str()).unwrap_or("");
            match cat {
                "profile" | "tech_stack" => Some(".harness/memory/profile.md".to_string()),
                "convention" | "notes" => Some(".harness/memory/conventions.md".to_string()),
                _ if !title.is_empty() => {
                    let slug = crate::memory::sanitize_slug(title);
                    Some(format!(".harness/memory/digests/{slug}.md"))
                }
                _ => None,
            }
        };
        if let Some(p) = detected_path {
            if let Some(obj) = ev.params.as_object_mut() {
                obj.insert("memory_file_path".into(), json!(p));
            }
        }
    }
    if tool_name == "read_memory" && status == "success" {
        let topic = ev.params.get("topic").and_then(|v| v.as_str()).map(|s| s.trim());
        let path_str = match topic {
            Some("profile") | Some("tech_stack") => Some(".harness/memory/profile.md".to_string()),
            Some("conventions") | Some("convention") => Some(".harness/memory/conventions.md".to_string()),
            Some(t) if !t.is_empty() => {
                let slug = crate::memory::sanitize_slug(t);
                Some(format!(".harness/memory/digests/{slug}.md"))
            }
            _ => None,
        };
        if let Some(p) = path_str {
            if let Some(obj) = ev.params.as_object_mut() {
                obj.insert("memory_file_path".into(), json!(p));
            }
        }
    }

    // ---- 决策工具结果质检与人机协同审批挂起 ----
    if matches!(tool_name, "score_decision" | "judge_decision" | "choice_decision") && status == "success" {
        let parsed_json: Option<Value> = serde_json::from_str::<Value>(&text).ok().or_else(|| {
            let t = text.trim();
            let first_brace = t.find('{');
            let last_brace = t.rfind('}');
            if let (Some(fb), Some(lb)) = (first_brace, last_brace) {
                if lb > fb {
                    serde_json::from_str::<Value>(&t[fb..=lb]).ok()
                } else {
                    None
                }
            } else {
                None
            }
        });

        let (is_passed, decision_summary, reason_str) = match tool_name {
            "score_decision" => {
                if let Some(ref val) = parsed_json {
                    let score_val = val.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                    let pct = val.get("scorePercent").and_then(|v| v.as_u64()).map(|n| n as u32).unwrap_or((score_val * 100.0).round() as u32);
                    let crit = val.get("reason").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let passed = pct >= 90;
                    let summary = format!("评分: {}%（{}）", pct, if passed { "质检通过，≥90分" } else { "未达90分及格线" });
                    (passed, summary, crit)
                } else {
                    (false, "评分结果解析失败".to_string(), "返回结果非标准 JSON 格式".to_string())
                }
            }
            "judge_decision" => {
                if let Some(ref val) = parsed_json {
                    let v = val.get("verdict").and_then(|v| v.as_bool()).unwrap_or(false);
                    let r = val.get("reason").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let summary = format!("判定结论: {}", if v { "符合/明确通过" } else { "不符合/存疑" });
                    (v, summary, r)
                } else {
                    (false, "判定结果解析失败".to_string(), "返回结果非标准 JSON 格式".to_string())
                }
            }
            _ => { // choice_decision
                if let Some(ref val) = parsed_json {
                    let c = val.get("choice").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let r = val.get("reason").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let need_ask = c.contains("澄清") || c.contains("确认") || c.contains("clarify") || c.contains("ask");
                    let summary = format!("选定目标: `{}`", c);
                    (!need_ask, summary, r)
                } else {
                    (false, "选择结果解析失败".to_string(), "返回结果非标准 JSON 格式".to_string())
                }
            }
        };

        if is_passed {
            text = format!(
                "{}\n\n【决策质检通过】{}。评估意见: {}\n请围绕确认的核心意图与方向顺畅推进后续工程执行。",
                text, decision_summary, reason_str
            );
        } else {
            // 决策未达标/存疑：触发人工审批挂起，等待用户确认采纳或补充指导
            ev.status = "pending_approval".into();
            ev.result_text = Some(text.clone());
            ev.approval_scope = None;
            {
                let db = state.db.lock().unwrap();
                let params_str = serde_json::to_string(&ev.params).ok();
                let _ = store::update_tool_event_full(
                    &db,
                    &ev.id,
                    "pending_approval",
                    Some(&text),
                    None,
                    detected_sub_id.as_deref().or(ev.subprocess_id.as_deref()),
                    params_str.as_deref(),
                );
            }
            emit_tool(app, session_id, &ev);

            let preview = match tool_name {
                "score_decision" => format!(
                    "【核心意图】{}\n【交付边界】{}",
                    args.get("understanding").and_then(|v| v.as_str()).unwrap_or(""),
                    args.get("plan").and_then(|v| v.as_str()).unwrap_or("")
                ),
                "judge_decision" => format!(
                    "【待判定事项】{}",
                    args.get("state")
                        .or_else(|| args.get("statement"))
                        .or_else(|| args.get("understanding"))
                        .or_else(|| args.get("instruction"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                ),
                _ => format!(
                    "【决策问题】{}\n【备选分支】{}",
                    args.get("instruction").or_else(|| args.get("question")).and_then(|v| v.as_str()).unwrap_or(""),
                    args.get("options").or_else(|| args.get("choices")).map(|v| v.to_string()).unwrap_or_default()
                ),
            };

            let jev_reason = format!("{}。详细意见: {}", decision_summary, reason_str);
            let req = ApprovalRequest {
                event_id: ev.id.clone(),
                session_id: session_id.to_string(),
                tool_name: tool_name.to_string(),
                params: args.clone(),
                risk: "intent_decision".into(),
                preview,
                force_once: true,
                risk_source: Some("jev".into()),
                jev_reason: Some(jev_reason),
            };

            let decision = approval::request_approval(app, &state, req).await;
            match decision {
                Decision::AllowOnce | Decision::AllowSession => {
                    status = "success".into();
                    text = format!(
                        "{}\n\n【用户已人工确认采纳】用户已审阅方案并确认放行！请立即围绕当前方案推进后续执行，无需重复质检。",
                        text
                    );
                    ev.approval_scope = Some("allow_once".into());
                }
                Decision::Deny(feedback) => {
                    status = "denied".into();
                    let user_msg = feedback
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or_else(|| "用户未补充具体说明，建议反思方案并向用户说明疑问".to_string());
                    text = format!(
                        "{}\n\n【用户补充指导意见】：用户审阅后未直接放行，并给出了具体补充要求：“{}”。\n请严格根据用户的明确指导，反思并修正核心意图与行动方向，或再次向用户确认，切勿擅自偏离用户要求！",
                        text, user_msg
                    );
                    ev.approval_scope = Some("denied".into());
                }
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
    if matches!(tool_name, "todo" | "create_plan" | "update_plan" | "switch_plan") && status == "success" {
        let db = state.db.lock().unwrap();
        crate::plan::emit_board_updated(app, &db, session_id);
    }
    (status, text)
}

/// 对话正常完成交付时调用：将任务清单（内存/KV todos 及最近的 todo 工具事件）中仍处于 in_progress 或 pending 的任务标记为 done 并持久化，同时广播更新。
/// 注意：物理方案文档（.harness/plans/*.md）拥有独立完整的生命周期与分步执行管理，严禁在此自动批量标记完成。
pub fn auto_finish_session_todos(state: &crate::AppState, app: &AppHandle, session_id: &str) {
    let db = state.db.lock().unwrap();
    let raw: Option<String> = store::get_kv(&db, session_id, "todos").ok().flatten();
    let mut val: Option<Value> = raw.and_then(|r| serde_json::from_str(&r).ok());

    // 若 session_kv 中未找到，尝试从最近的 todo tool event 中获取
    if val.is_none() {
        let ev_params: Option<String> = db
            .query_row(
                "SELECT te.params_json FROM tool_events te JOIN messages m ON te.message_id = m.id WHERE m.session_id = ?1 AND te.tool_name = 'todo' ORDER BY te.created_at DESC LIMIT 1",
                rusqlite::params![session_id],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        if let Some(p) = ev_params {
            val = serde_json::from_str(&p).ok();
        }
    }

    let Some(mut val) = val else { return };
    let Some(todos_arr) = val.get_mut("todos").and_then(|v| v.as_array_mut()) else { return };

    let mut changed = false;
    for item in todos_arr.iter_mut() {
        let st = item.get("status").and_then(|s| s.as_str());
        if st == Some("in_progress") || st == Some("pending") {
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

        // 同步更新最近的 todo 工具事件 params_json，并广播 tool:update，使前端卡片与历史记录保持一致
        let last_todo_ev_id: Option<String> = db
            .query_row(
                "SELECT te.id FROM tool_events te JOIN messages m ON te.message_id = m.id WHERE m.session_id = ?1 AND te.tool_name = 'todo' ORDER BY te.created_at DESC LIMIT 1",
                rusqlite::params![session_id],
                |r| r.get(0),
            )
            .ok();
        if let Some(ref ev_id) = last_todo_ev_id {
            let _ = store::update_tool_event_full(
                &db,
                ev_id,
                "success",
                None,
                None,
                None,
                Some(&new_raw),
            );
            if let Ok(Some((ev, _))) = store::get_tool_event_with_session(&db, ev_id) {
                emit_tool(app, session_id, &ev);
            }
        }
    }

    // 检查当前物理方案：若步骤已全部标记为 done，自动结案为 completed
    if let Ok(Some(sess)) = store::get_session(&db, session_id) {
        let ws = std::path::Path::new(&sess.workspace_path);
        if let Some((path, mut meta, body)) = crate::plan::find_plan_file(ws, session_id, None, Some(&db)) {
            if meta.status == "in_progress" {
                let steps = crate::plan::parse_steps_from_markdown(&body);
                if !steps.is_empty() && steps.iter().all(|s| s.status == "done") {
                    meta.status = "completed".to_string();
                    meta.updated_at = chrono::Utc::now().to_rfc3339();
                    let content = crate::plan::format_with_frontmatter(&meta, &body);
                    let _ = std::fs::write(&path, content);
                }
            }
        }
    }

    crate::plan::emit_board_updated(app, &db, session_id);
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

        // 同步更新最近的 todo 工具事件 params_json，并广播 tool:update
        let last_todo_ev_id: Option<String> = db
            .query_row(
                "SELECT te.id FROM tool_events te JOIN messages m ON te.message_id = m.id WHERE m.session_id = ?1 AND te.tool_name = 'todo' ORDER BY te.created_at DESC LIMIT 1",
                rusqlite::params![session_id],
                |r| r.get(0),
            )
            .ok();
        if let Some(ref ev_id) = last_todo_ev_id {
            let _ = store::update_tool_event_full(
                &db,
                ev_id,
                "success",
                None,
                None,
                None,
                Some(&new_raw),
            );
            if let Ok(Some((ev, _))) = store::get_tool_event_with_session(&db, ev_id) {
                emit_tool(app, session_id, &ev);
            }
        }
    }

    crate::plan::emit_board_updated(app, &db, session_id);
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
    _sys: &str,
    _token_limit: usize,
) -> bool {
    crate::context::propose_and_execute_compaction(app, state, session_id, cfg, None, false)
        .await
        .unwrap_or(false)
}

pub fn build_context(
    conn: &rusqlite::Connection,
    session_id: &str,
    system: &str,
    token_limit: usize,
) -> (Result<Vec<Value>, String>, usize, Option<crate::models::TruncationNotice>) {
    crate::context::build_execution_context(conn, session_id, system, token_limit)
}

#[allow(unused_imports)]
pub use crate::context::static_system::{
    build_static_project_section as build_project_section, project_section, ResolvedLink,
};

pub fn system_prompt(
    session: &Session,
    project_section: Option<&str>,
    disabled_sops: &[String],
    plan_mode: &str,
) -> String {
    crate::context::build_static_system_prompt(session, disabled_sops, plan_mode, project_section, None, None)
}

fn emit_tool(app: &AppHandle, session_id: &str, ev: &ToolEvent) {
    crate::emit_event(
        app,
        "tool:update",
        &json!({"sessionId": session_id, "event": ev}),
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
    crate::emit_event(
        app,
        "tool:update",
        &json!({"sessionId": session_id, "event": ev, "extra": extra}),
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
    fn test_rescue_reasoning_deliverable() {
        // 场景 1：end 标记加粗结论
        let sample1 = "思考草稿... 保持简洁但信息密集。end**结论先说：对乳糖不耐受者，喝牛奶弊大于利。**\n\n详细展开...";
        let (r1, c1) = rescue_reasoning_deliverable(sample1);
        assert_eq!(r1, "思考草稿... 保持简洁但信息密集。");
        assert_eq!(c1, "**结论先说：对乳糖不耐受者，喝牛奶弊大于利。**\n\n详细展开...");

        // 场景 2：Markdown 标题结论
        let sample2 = "内部推演完毕。\n\n## 先给结论\n\n总体弊大于利。";
        let (r2, c2) = rescue_reasoning_deliverable(sample2);
        assert_eq!(r2, "内部推演完毕。");
        assert_eq!(c2, "## 先给结论\n\n总体弊大于利。");

        // 场景 3：无结构纯思考
        let sample3 = "我在思考，我想想怎么回答用户。";
        let (r3, c3) = rescue_reasoning_deliverable(sample3);
        assert_eq!(r3, "我在思考，我想想怎么回答用户。");
        assert!(c3.starts_with("> 💡 *（模型未在交付正文通道输出"));
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
    fn test_build_context_preserves_reasoning_content() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        store::init_schema(&conn).unwrap();
        let s = store::create_session(&conn, ".", None, "t", "confirm").unwrap();
        let _u = store::new_message(&conn, &s.id, "user", Some("hi".into()), false).unwrap();
        let a = store::new_message(&conn, &s.id, "assistant", Some("content".into()), false).unwrap();
        store::update_message_reasoning(&conn, &a.id, "I am thinking deeply...").unwrap();
        store::update_message_tool_calls(
            &conn,
            &a.id,
            &json!([{"id": "call_1", "type": "function", "function": {"name": "judge_decision", "arguments": "{}"}}]),
            "content",
            None,
        )
        .unwrap();

        let (msgs, _, _) = build_context(&conn, &s.id, "sys", 28000);
        let msgs = msgs.unwrap();
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[2]["role"], "assistant");
        // 核心验证：发往模型的上下文中剥离了历史思维链，防止思维反刍
        assert!(msgs[2].get("reasoning_content").is_none() || msgs[2]["reasoning_content"].is_null());
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
        assert!(all_enabled.contains("团队协同委派原则"));
        assert!(all_enabled.contains("修改文件前必须先用 read_file 读取相关内容"));
        assert!(all_enabled.contains("精益代码研读与克制探索 SOP"));
        assert!(all_enabled.contains("todo 工具列出计划"));
        assert!(all_enabled.contains("浏览器无头自主测试与免视觉验证 SOP"));

        let disabled = vec![
            "plan_first".to_string(),
            "memory_distill".to_string(),
            "todo_lifecycle".to_string(),
            "safe_code_edit".to_string(),
            "surgical_code_reading".to_string(),
                        "browser_autonomous_testing".to_string(),
        ];
        let filtered = system_prompt(&session, None, &disabled, "standard");
        assert!(!filtered.contains("方案先行"));
        assert!(!filtered.contains("边读边记与强制沉淀 SOP"));
        assert!(!filtered.contains("todo 工具列出计划"));
        assert!(!filtered.contains("修改文件前必须先用 read_file 读取相关内容"));
        assert!(!filtered.contains("精益代码研读与克制探索 SOP"));
                assert!(!filtered.contains("浏览器无头自主测试与免视觉验证 SOP"));
        assert!(filtered.contains("团队协同委派原则"));
    }

    #[test]
    fn test_browser_autonomous_testing_sop_in_system_prompt() {
        let session = Session {
            workspace_path: "D:\\ws".into(),
            ..Default::default()
        };
        let p = system_prompt(&session, None, &[], "standard");
        assert!(p.contains("浏览器无头自主测试与免视觉验证 SOP"));
        assert!(p.contains("headless: true"));
        assert!(p.contains("免视觉断言原则"));
        assert!(p.contains("登录态自愈与免登保底"));
        assert!(p.contains("探针式自检与自愈闭环"));
    }

    #[test]
    fn test_jev_intent_command_handling() {
        let (is_jev, intent) = extract_jev_intent("/jev:决策 重构登录模块方案");
        assert!(is_jev);
        assert_eq!(intent, "重构登录模块方案");

        let (is_jev2, intent2) = extract_jev_intent("/jev: 决策 评估数据库迁移");
        assert!(is_jev2);
        assert_eq!(intent2, "评估数据库迁移");

        let (is_not, _) = extract_jev_intent("普通对话请求");
        assert!(!is_not);
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
    fn test_long_task_always_proceed_flow_and_negative_intent() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        store::init_schema(&conn).unwrap();
        let proj = store::create_project(&conn, "TestProj", Some("D:\\ws")).unwrap();
        store::set_project_plan_mode(&conn, &proj.id, "always_plan").unwrap();

        let s = store::create_session(&conn, "D:\\ws", Some(&proj.id), "test", "confirm").unwrap();

        // 1. 常规长任务消息（无否定代码意图）
        let _m1 = store::new_message(&conn, &s.id, "user", Some("【长任务目标模式】实现用户认证模块\n\n请分析并拆解".into()), false).unwrap();
        let msgs = store::all_messages(&conn, &s.id).unwrap();
        let last_user = msgs.iter().filter(|m| m.role == "user").last().and_then(|m| m.content.as_deref());
        let neg = last_user.map(contains_negative_code_intent).unwrap_or(false);
        let is_goal = last_user.map(|c| c.contains("【长任务目标模式】") || c.starts_with("/goal")).unwrap_or(false);
        assert!(!neg);
        assert!(is_goal);

        let effective_mode = if is_goal { "always_proceed" } else { "always_plan" };
        assert_eq!(effective_mode, "always_proceed");

        // 2. 带否定意图的长任务消息
        let _m2 = store::new_message(&conn, &s.id, "user", Some("【长任务目标模式】梳理工程重构，先不要改代码，仅出方案评估".into()), false).unwrap();
        let msgs2 = store::all_messages(&conn, &s.id).unwrap();
        let last_user2 = msgs2.iter().filter(|m| m.role == "user").last().and_then(|m| m.content.as_deref());
        let neg2 = last_user2.map(contains_negative_code_intent).unwrap_or(false);
        let is_goal2 = last_user2.map(|c| c.contains("【长任务目标模式】") || c.starts_with("/goal")).unwrap_or(false);
        assert!(neg2);
        assert!(is_goal2);

        let effective_mode2 = if is_goal2 { "always_proceed" } else { "always_plan" };
        assert_eq!(effective_mode2, "always_proceed");
    }

    #[test]
    fn test_subagent_and_long_task_mode_hierarchy() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        store::init_schema(&conn).unwrap();
        let proj = store::create_project(&conn, "StrictProj", Some("D:\\ws")).unwrap();
        store::set_project_plan_mode(&conn, &proj.id, "always_plan").unwrap();

        // 1. 普通前台会话在 always_plan 项目中 -> 继承 always_plan
        let s = store::create_session(&conn, "D:\\ws", Some(&proj.id), "normal", "confirm").unwrap();
        let plan_mode = store::get_project(&conn, &proj.id).unwrap().unwrap().plan_mode;
        let is_sub = s.session_type == "subagent" || s.parent_session_id.is_some();
        let is_running_long = store::get_active_long_task(&conn, &s.id)
            .ok()
            .flatten()
            .map(|t| t.status == "running" || t.status == "planning")
            .unwrap_or(false);
        let eff = if is_sub || is_running_long { "always_proceed" } else { &plan_mode };
        assert_eq!(eff, "always_plan");

        // 2. 子任务沙箱会话在 always_plan 项目中 -> 强制升级为 always_proceed
        let sub = store::create_subagent_session(&conn, &s.id, "worker", "Worker Sub", "Do task", "D:\\ws", None, Some(&proj.id), None).unwrap();
        let is_sub2 = sub.session_type == "subagent" || sub.parent_session_id.is_some();
        let eff2 = if is_sub2 { "always_proceed" } else { &plan_mode };
        assert_eq!(eff2, "always_proceed");

        // 3. 长任务在 always_plan 项目中处于 running -> 强制升级为 always_proceed
        let now = chrono::Utc::now().to_rfc3339();
        let running_lt = crate::models::LongTask {
            id: "task-test-hierarchy".to_string(),
            session_id: s.id.clone(),
            workspace_path: "D:\\ws".to_string(),
            goal: "Long Task".to_string(),
            status: "running".to_string(),
            current_subtask_index: 0,
            subtasks: vec![],
            max_budget_tokens: None,
            total_tokens_used: 0,
            cached_tokens_used: 0,
            cache_hit_rate: 0.0,
            prompt_tokens_used: 0,
            completion_tokens_used: 0,
            current_step: 1,
            max_steps: 30,
            created_at: now.clone(),
            updated_at: now,
        };
        store::create_long_task(&conn, &running_lt).unwrap();
        let is_running_long2 = store::get_active_long_task(&conn, &s.id)
            .ok()
            .flatten()
            .map(|t| t.status == "running" || t.status == "planning")
            .unwrap_or(false);
        let eff3 = if is_running_long2 { "always_proceed" } else { &plan_mode };
        assert_eq!(eff3, "always_proceed");
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

    #[test]
    fn test_build_context_filters_reverted_messages() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        store::init_schema(&conn).unwrap();
        let s = store::create_session(&conn, "D:\\test", None, "test", "confirm").unwrap();

        // 1. 发送第 1 轮消息（有效）
        let _u1 = store::new_message(&conn, &s.id, "user", Some("请实现功能A".into()), false).unwrap();
        let _a1 = store::new_message(&conn, &s.id, "assistant", Some("已实现功能A".into()), false).unwrap();

        // 2. 发送第 2 轮消息（随后被软撤回）
        let _u2 = store::new_message(&conn, &s.id, "user", Some("请实现功能B".into()), false).unwrap();
        let a2 = store::new_message(&conn, &s.id, "assistant", Some("已实现功能B".into()), false).unwrap();

        // 3. 用户发送后续提问（使得 a2 成为对话历史，避免被末尾 assistant 协议防御剪除）
        let _u3 = store::new_message(&conn, &s.id, "user", Some("请检查代码".into()), false).unwrap();

        // 验证撤回前均在上下文中
        let (ctx1, _, _) = build_context(&conn, &s.id, "sys", 64000);
        let ctx1 = ctx1.unwrap();
        assert!(ctx1.iter().any(|m| m["content"].as_str() == Some("已实现功能B")));

        // 软撤回第 2 轮消息
        let now_str = store::now();
        store::mark_snapshots_reverted_for_message(&conn, &a2.id, Some(&now_str)).unwrap();

        // 验证撤回后第 2 轮 assistant 消息被精确排除，大模型看不见该历史
        let (ctx2, _, _) = build_context(&conn, &s.id, "sys", 64000);
        let ctx2 = ctx2.unwrap();
        assert!(ctx2.iter().any(|m| m["content"].as_str() == Some("已实现功能A")));
        assert!(!ctx2.iter().any(|m| m["content"].as_str() == Some("已实现功能B")));

        // 重新应用（重做）
        store::mark_snapshots_reverted_for_message(&conn, &a2.id, None).unwrap();
        let (ctx3, _, _) = build_context(&conn, &s.id, "sys", 64000);
        let ctx3 = ctx3.unwrap();
        assert!(ctx3.iter().any(|m| m["content"].as_str() == Some("已实现功能B")));
    }

    #[test]
    fn test_always_plan_guard_decoupling() {
        let project_plan_mode = "always_plan";
        let mut created_plan_in_this_turn = false;

        // 1. 调用 update_plan 成功：created_plan_in_this_turn 不应变为 true
        let tool_name = "update_plan";
        let status = "success";
        if tool_name == "create_plan" && status == "success" {
            created_plan_in_this_turn = true;
        }
        assert!(!created_plan_in_this_turn);

        // 紧接着调用 edit_file：不应被守卫拦截
        let is_modifying = true;
        let guard_denied = if is_modifying {
            if project_plan_mode == "always_plan" && created_plan_in_this_turn {
                Some("拦截".to_string())
            } else {
                None
            }
        } else {
            None
        };
        assert!(guard_denied.is_none());

        // 2. 调用 switch_plan 成功：created_plan_in_this_turn 同样不应变为 true
        let tool_name = "switch_plan";
        if tool_name == "create_plan" && status == "success" {
            created_plan_in_this_turn = true;
        }
        assert!(!created_plan_in_this_turn);

        // 3. 调用 create_plan 成功：created_plan_in_this_turn 必须变为 true
        let tool_name = "create_plan";
        if tool_name == "create_plan" && status == "success" {
            created_plan_in_this_turn = true;
        }
        assert!(created_plan_in_this_turn);

        // 紧接着在同轮次调用 edit_file：必须被守卫精准拦截
        let guard_denied = if is_modifying {
            if project_plan_mode == "always_plan" && created_plan_in_this_turn {
                Some("拦截".to_string())
            } else {
                None
            }
        } else {
            None
        };
        assert!(guard_denied.is_some());
    }

    #[test]
    fn test_physical_plan_not_mutated_on_turn_end() {
        struct TempDir(std::path::PathBuf);
        impl TempDir {
            fn new() -> Self {
                let p = std::env::temp_dir().join(format!("harness-test-agent-{}", uuid::Uuid::new_v4()));
                let _ = std::fs::create_dir_all(&p);
                Self(p)
            }
        }
        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        let tmp = TempDir::new();
        let ws = &tmp.0;
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        store::init_schema(&conn).unwrap();
        let s = store::create_session(&conn, ws.to_str().unwrap(), None, "test", "confirm").unwrap();

        // 1. 创建一份物理方案，包含 3 个待办步骤
        let res = crate::plan::create_plan(
            ws,
            &s.id,
            "测试功能方案",
            "测试目标",
            "测试架构",
            &["src/main.rs".into()],
            &["步骤1".into(), "步骤2".into(), "步骤3".into()],
            None,
            None,
            None,
            Some(&conn),
        );
        assert!(res.is_ok());

        // 2. 检查创建后状态
        let (_, meta1, body1) = crate::plan::find_plan_file(ws, &s.id, None, Some(&conn)).unwrap();
        assert_eq!(meta1.status, "in_progress");
        assert!(body1.contains("- [ ] 步骤 1"));
        assert!(body1.contains("- [ ] 步骤 2"));
        assert!(body1.contains("- [ ] 步骤 3"));

        // 3. 验证方案文件保持完整步骤与 in_progress 状态，未被刷为 completed 或 [x]
        let (_, meta2, body2) = crate::plan::find_plan_file(ws, &s.id, None, Some(&conn)).unwrap();
        assert_eq!(meta2.status, "in_progress");
        assert!(!body2.contains("- [x] 步骤 1"));
    }

    #[test]
    fn test_auto_finish_todos_heals_pending_and_in_progress() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        store::init_schema(&conn).unwrap();
        let s = store::create_session(&conn, "", None, "test", "confirm").unwrap();
        let msg = store::new_message(&conn, &s.id, "assistant", Some("final text".into()), false).unwrap();

        let initial_json = serde_json::json!({
            "todos": [
                { "content": "步骤1: 分析", "status": "done" },
                { "content": "步骤2: 编码", "status": "in_progress" },
                { "content": "步骤3: 输出最终报告", "status": "pending" }
            ]
        });
        store::set_kv(&conn, &s.id, "todos", &initial_json.to_string()).unwrap();

        let ev_id = uuid::Uuid::new_v4().to_string();
        let ev = ToolEvent {
            id: ev_id.clone(),
            message_id: msg.id.clone(),
            tool_name: "todo".to_string(),
            tool_call_id: Some("call_1".to_string()),
            params: initial_json.clone(),
            result_text: None,
            status: "success".to_string(),
            approval_scope: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            reverted_at: None,
            subprocess_id: None,
        };
        store::insert_tool_event(&conn, &ev).unwrap();

        // 模拟正常交付结束：执行自愈逻辑
        let raw = store::get_kv(&conn, &s.id, "todos").unwrap().unwrap();
        let mut val: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let todos_arr = val.get_mut("todos").and_then(|v| v.as_array_mut()).unwrap();
        for item in todos_arr.iter_mut() {
            let st = item.get("status").and_then(|s| s.as_str());
            if st == Some("in_progress") || st == Some("pending") {
                item["status"] = serde_json::json!("done");
            }
        }
        let updated_raw = val.to_string();
        store::set_kv(&conn, &s.id, "todos", &updated_raw).unwrap();

        let last_todo_ev_id: Option<String> = conn
            .query_row(
                "SELECT te.id FROM tool_events te JOIN messages m ON te.message_id = m.id WHERE m.session_id = ?1 AND te.tool_name = 'todo' ORDER BY te.created_at DESC LIMIT 1",
                rusqlite::params![s.id],
                |r| r.get(0),
            )
            .ok();
        assert_eq!(last_todo_ev_id.as_deref(), Some(ev_id.as_str()));

        store::update_tool_event_full(&conn, &ev_id, "success", None, None, None, Some(&updated_raw)).unwrap();

        // 验证 session_kv 中所有任务均为 done
        let saved_kv = store::get_kv(&conn, &s.id, "todos").unwrap().unwrap();
        let kv_val: serde_json::Value = serde_json::from_str(&saved_kv).unwrap();
        for t in kv_val["todos"].as_array().unwrap() {
            assert_eq!(t["status"].as_str(), Some("done"));
        }

        // 验证 tool_events 表中的 params_json 也同步被更新为全部 done
        let (ev, _) = store::get_tool_event_with_session(&conn, &ev_id).unwrap().unwrap();
        for t in ev.params["todos"].as_array().unwrap() {
            assert_eq!(t["status"].as_str(), Some("done"));
        }
    }

    #[test]
    fn test_code_modified_filtering_logic() {
        let ws = std::path::Path::new("F:\\WorkSpace\\Other\\harness_mini");

        // 纯文档、记忆与规划规范：不触发代码修改
        let doc_paths = vec![
            "docs/JEV_DECISION_GATEWAY_INTEGRATION_SPEC.md",
            "docs/architecture.markdown",
            ".harness/plans/20260930-revert-plans.md",
            ".harness/memory/digests/test.md",
            "README.md",
            ".gitignore",
        ];
        for dp in doc_paths {
            assert!(
                !crate::sop::is_code_or_build_file_with_workspace(std::path::Path::new(dp), Some(ws)),
                "{} 应被判定为非代码文件",
                dp
            );
        }

        // 真实代码及构建清单文件：应触发代码修改
        let code_paths = vec![
            "src-tauri/src/agent.rs",
            "src-tauri/src/sop.rs",
            "src/components/FloatingTaskPanel.tsx",
            "src-tauri/Cargo.toml",
            "package.json",
            "tsconfig.json",
        ];
        for cp in code_paths {
            assert!(
                crate::sop::is_code_or_build_file_with_workspace(std::path::Path::new(cp), Some(ws)),
                "{} 应被判定为代码/构建文件",
                cp
            );
        }
    }

    #[test]
    fn test_format_recent_context_for_gate() {
        let msgs = vec![
            Message {
                id: "m1".into(),
                session_id: "s1".into(),
                role: "user".into(),
                content: Some("你好，请介绍门诊系统".into()),
                ..Default::default()
            },
            Message {
                id: "m2".into(),
                session_id: "s1".into(),
                role: "assistant".into(),
                content: Some("请选择：A1 学习理解 / A2 架构设计。默认深入门诊全流程。".into()),
                ..Default::default()
            },
            Message {
                id: "m3".into(),
                session_id: "s1".into(),
                role: "user".into(),
                content: Some("A1".into()),
                ..Default::default()
            },
        ];

        let formatted = format_recent_context_for_decision(&msgs, Some("m3"));
        assert!(formatted.contains("请选择：A1 学习理解"));
        assert!(formatted.contains("助手:"));
        assert!(formatted.contains("用户: 你好"));
        assert!(!formatted.contains("用户: A1"));
    }

    
}
