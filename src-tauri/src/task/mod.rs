pub mod loop_runner;
pub mod memory;
pub mod planner;
pub mod verifier;

pub use loop_runner::run_task_loop;
pub use memory::{build_working_memory_block, safe_truncate};
pub use planner::{decompose_goal, replan_subtasks};
pub use verifier::{create_git_snapshot, execute_verify_command};

use crate::agent;
use crate::models::{LongTask, LongTaskStatus, TaskCheckpoint, TaskSubItem};
use crate::store;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::watch;

static TASK_CONTROLS: Mutex<Option<HashMap<String, watch::Sender<bool>>>> = Mutex::new(None);

pub fn set_control(task_id: &str, stop_val: bool) {
    let mut map = TASK_CONTROLS.lock().unwrap();
    let controls = map.get_or_insert_with(HashMap::new);
    if let Some(tx) = controls.get(task_id) {
        let _ = tx.send(stop_val);
    }
}

pub fn get_next_subtask_index(task: &LongTask) -> Option<usize> {
    if let Some(idx) = task.subtasks.iter().position(|s| s.status == "running" || s.status == "verifying" || s.status == "in_progress") {
        return Some(idx);
    }
    if task.current_subtask_index < task.subtasks.len() {
        return Some(task.current_subtask_index);
    }
    task.subtasks.iter().position(|s| s.status == "pending" || s.status == "failed" || s.status == "interrupted" || s.status == "paused")
}

pub(crate) fn register_control(task_id: &str) -> watch::Receiver<bool> {
    let mut map = TASK_CONTROLS.lock().unwrap();
    let controls = map.get_or_insert_with(HashMap::new);
    let (tx, rx) = watch::channel(false);
    controls.insert(task_id.to_string(), tx);
    rx
}

pub(crate) fn remove_control(task_id: &str) {
    let mut map = TASK_CONTROLS.lock().unwrap();
    if let Some(controls) = map.as_mut() {
        controls.remove(task_id);
    }
}

/// 应用启动时收敛未正常退出的孤儿任务
pub fn recover_orphaned_tasks(conn: &rusqlite::Connection) -> Result<usize, String> {
    let now = store::now();
    let n = conn
        .execute(
            "UPDATE long_tasks SET status = 'interrupted', updated_at = ?1 WHERE status IN ('running', 'planning')",
            rusqlite::params![now],
        )
        .map_err(|e| format!("清理孤儿长任务失败: {e}"))?;
    Ok(n)
}

/// 启动新的长任务
pub fn start_long_task(
    app: AppHandle,
    session_id: String,
    goal: String,
    max_budget_tokens: Option<u64>,
) -> Result<LongTask, String> {
    let state = app.state::<crate::AppState>();

    // 检查该会话是否已有进行中或已暂停的长任务
    {
        let db = state.db.lock().unwrap();
        if let Ok(Some(existing)) = store::get_active_long_task(&db, &session_id) {
            if existing.status == "running" || existing.status == "planning" {
                return Err("当前会话已有正在运行的长任务，请等待完成或先暂停/终止它。".into());
            }
        }
    }

    let (workspace_path, task_id, subtask_max_steps) = {
        let db = state.db.lock().unwrap();
        let master = state.master_key.lock().unwrap();
        let s = store::get_session(&db, &session_id)?
            .ok_or_else(|| "会话不存在".to_string())?;
        let tid = format!("task-{}", uuid::Uuid::new_v4().simple());
        let settings = store::get_settings_with_secrets(&db, &master).unwrap_or_default();
        let limit = settings.task_subtask_max_steps.max(1) as usize;
        (s.workspace_path, tid, limit)
    };

    let now = store::now();
    let task = LongTask {
        id: task_id.clone(),
        session_id: session_id.clone(),
        workspace_path,
        goal: goal.clone(),
        status: LongTaskStatus::Planning.as_str().to_string(),
        current_subtask_index: 0,
        subtasks: vec![],
        max_budget_tokens,
        total_tokens_used: 0,
        cached_tokens_used: 0,
        cache_hit_rate: 0.0,
        prompt_tokens_used: 0,
        completion_tokens_used: 0,
        current_step: 0,
        max_steps: subtask_max_steps,
        created_at: now.clone(),
        updated_at: now,
    };

    {
        let db = state.db.lock().unwrap();
        store::create_long_task(&db, &task)?;
        let prompt_text = format!("🎯 **启动长任务目标模式**\n\n**总目标**：{}", goal);
        if let Ok(user_msg) = store::new_message(&db, &session_id, "user", Some(prompt_text), false) {
            let _ = app.emit("message:final", &user_msg);
        }
    }

    let _ = app.emit("task:update", &task);

    let rx = register_control(&task_id);
    let app_clone = app.clone();
    let task_clone = task.clone();

    tauri::async_runtime::spawn(async move {
        run_task_loop(app_clone, task_clone, rx).await;
    });

    Ok(task)
}

/// 暂停长任务（单一权威控制中心：即时冻结各子任务与进程树，原子持久化 paused 状态）
pub fn pause_long_task(app: &AppHandle, task_id: &str) -> Result<(), String> {
    set_control(task_id, true);
    let state = app.state::<crate::AppState>();
    let (session_id, running_sub_ids) = {
        let db = state.db.lock().unwrap();
        let mut task = store::get_long_task(&db, task_id)?.ok_or("长任务不存在")?;
        task.status = "paused".to_string();
        let mut running_subs = Vec::new();
        for sub in &mut task.subtasks {
            if sub.status == "running" || sub.status == "verifying" || sub.status == "in_progress" {
                sub.status = "paused".to_string();
                if !sub.id.is_empty() && !sub.id.starts_with("sub-") {
                    running_subs.push(sub.id.clone());
                }
            }
        }
        task.updated_at = store::now();
        store::update_long_task(&db, &task)?;

        // 同步将进行中的子会话在 sessions 表中置为 paused，并关闭 open runs
        for sid in &running_subs {
            let _ = store::set_session_status(&db, sid, "paused");
            let _ = db.execute(
                "UPDATE runs SET status = 'cancelled', ended_at = ?1 WHERE session_id = ?2 AND status IN ('running', 'interrupted')",
                rusqlite::params![store::now(), sid],
            );
        }

        let _ = db.execute(
            "UPDATE runs SET status = 'cancelled', ended_at = ?1 WHERE session_id = ?2 AND status IN ('running', 'interrupted')",
            rusqlite::params![store::now(), &task.session_id],
        );
        (task.session_id, running_subs)
    };

    // 确定性即刻强杀：直接对所有运行中的子任务执行 AbortHandle 强杀与控制台进程树销毁（cascade=false，防止意外篡改）
    for sid in &running_sub_ids {
        agent::stop_session_ext(app, sid, false);
        let _ = app.emit("subprocess:update", serde_json::json!({
            "parentId": &session_id,
            "parentSessionId": &session_id,
            "subprocessId": sid,
            "status": "paused"
        }));
        let _ = app.emit("subagent:update", serde_json::json!({
            "parentId": &session_id,
            "subagentId": sid,
            "status": "paused"
        }));
        crate::emit_event(
            app,
            "run:status",
            &serde_json::json!({
                "sessionId": sid,
                "status": "idle"
            }),
        );
    }

    // 停止主会话挂起状态（cascade = false 保证不将子会话误刷成 cancelled）
    agent::stop_session_ext(app, &session_id, false);

    crate::emit_event(
        app,
        "run:status",
        &serde_json::json!({
            "sessionId": &session_id,
            "status": "idle"
        }),
    );
    let _ = app.emit("subprocesses:changed", serde_json::json!({ "parentId": &session_id }));
    let _ = app.emit("subagents:changed", serde_json::json!({ "parentId": &session_id }));

    if let Ok(Some(task)) = {
        let db = state.db.lock().unwrap();
        store::get_long_task(&db, task_id)
    } {
        let _ = app.emit("task:update", &task);
    }
    Ok(())
}

/// 恢复长任务执行
pub fn resume_long_task(app: &AppHandle, task_id: &str) -> Result<LongTask, String> {
    let state = app.state::<crate::AppState>();
    let task = {
        let db = state.db.lock().unwrap();
        let mut t = store::get_long_task(&db, task_id)?.ok_or("长任务不存在")?;
        if t.status != "paused" && t.status != "failed" && t.status != "interrupted" {
            return Err(format!("任务当前状态为 {}，无法恢复执行", t.status));
        }
        t.status = "running".to_string();
        t.updated_at = store::now();
        for sub in &mut t.subtasks {
            if sub.status == "failed"
                || sub.status == "in_progress"
                || sub.status == "verifying"
                || sub.status == "interrupted"
                || sub.status == "paused"
            {
                sub.status = "pending".to_string();
                sub.error = None;
            }
        }
        let next_idx = get_next_subtask_index(&t).unwrap_or(t.current_subtask_index);
        t.current_subtask_index = next_idx;
        store::update_long_task(&db, &t)?;
        t
    };

    let _ = app.emit("task:update", &task);
    let rx = register_control(task_id);
    let app_clone = app.clone();
    let task_clone = task.clone();

    tauri::async_runtime::spawn(async move {
        run_task_loop(app_clone, task_clone, rx).await;
    });

    Ok(task)
}

/// 终止/取消长任务（单一权威控制中心：即时冻结各子任务与进程树，原子持久化 cancelled 状态）
pub fn cancel_long_task(app: &AppHandle, task_id: &str) -> Result<(), String> {
    set_control(task_id, true);
    let state = app.state::<crate::AppState>();
    let (session_id, running_sub_ids) = {
        let db = state.db.lock().unwrap();
        let mut task = store::get_long_task(&db, task_id)?.ok_or("长任务不存在")?;
        task.status = "cancelled".to_string();
        let mut running_subs = Vec::new();
        for sub in &mut task.subtasks {
            if sub.status == "running" || sub.status == "verifying" || sub.status == "in_progress" {
                sub.status = "cancelled".to_string();
                if !sub.id.is_empty() && !sub.id.starts_with("sub-") {
                    running_subs.push(sub.id.clone());
                }
            }
        }
        task.updated_at = store::now();
        store::update_long_task(&db, &task)?;

        for sid in &running_subs {
            let _ = store::set_session_status(&db, sid, "cancelled");
            let _ = db.execute(
                "UPDATE runs SET status = 'cancelled', ended_at = ?1 WHERE session_id = ?2 AND status IN ('running', 'interrupted')",
                rusqlite::params![store::now(), sid],
            );
        }

        let _ = db.execute(
            "UPDATE runs SET status = 'cancelled', ended_at = ?1 WHERE session_id = ?2 AND status IN ('running', 'interrupted')",
            rusqlite::params![store::now(), &task.session_id],
        );
        (task.session_id, running_subs)
    };

    for sid in &running_sub_ids {
        agent::stop_session_ext(app, sid, false);
        let _ = app.emit("subprocess:update", serde_json::json!({
            "parentId": &session_id,
            "parentSessionId": &session_id,
            "subprocessId": sid,
            "status": "cancelled"
        }));
        let _ = app.emit("subagent:update", serde_json::json!({
            "parentId": &session_id,
            "subagentId": sid,
            "status": "cancelled"
        }));
        crate::emit_event(
            app,
            "run:status",
            &serde_json::json!({
                "sessionId": sid,
                "status": "idle"
            }),
        );
    }

    agent::stop_session_ext(app, &session_id, false);

    crate::emit_event(
        app,
        "run:status",
        &serde_json::json!({
            "sessionId": &session_id,
            "status": "idle"
        }),
    );
    let _ = app.emit("subprocesses:changed", serde_json::json!({ "parentId": &session_id }));
    let _ = app.emit("subagents:changed", serde_json::json!({ "parentId": &session_id }));

    if let Ok(Some(task)) = {
        let db = state.db.lock().unwrap();
        store::get_long_task(&db, task_id)
    } {
        let _ = app.emit("task:update", &task);
    }
    Ok(())
}

/// 获取会话当前活跃长任务
pub fn get_active_task(app: &AppHandle, session_id: &str) -> Result<Option<LongTask>, String> {
    let state = app.state::<crate::AppState>();
    let db = state.db.lock().unwrap();
    store::get_active_long_task(&db, session_id)
}

/// 列出任务的所有检查点
pub fn list_task_checkpoints(app: &AppHandle, task_id: &str) -> Result<Vec<TaskCheckpoint>, String> {
    let state = app.state::<crate::AppState>();
    let db = state.db.lock().unwrap();
    store::list_task_checkpoints(&db, task_id)
}

/// 回退到指定检查点（时光机与代码级 Reset）
pub fn rollback_to_checkpoint(app: &AppHandle, checkpoint_id: &str) -> Result<LongTask, String> {
    let state = app.state::<crate::AppState>();
    let (mut task, cp) = {
        let db = state.db.lock().unwrap();
        let mut stmt = db
            .prepare(
                "SELECT id, task_id, step_number, subtask_id, status, summary, working_memory, git_commit_hash, created_at
                 FROM task_checkpoints WHERE id = ?1",
            )
            .map_err(|e| e.to_string())?;

        let cp = stmt
            .query_row(rusqlite::params![checkpoint_id], |r| {
                let step_num: i64 = r.get(2)?;
                Ok(TaskCheckpoint {
                    id: r.get(0)?,
                    task_id: r.get(1)?,
                    step_number: step_num as usize,
                    subtask_id: r.get(3)?,
                    status: r.get(4)?,
                    summary: r.get(5)?,
                    working_memory: r.get(6)?,
                    git_commit_hash: r.get(7)?,
                    created_at: r.get(8)?,
                })
            })
            .map_err(|e| format!("未找到指定的检查点: {e}"))?;

        let task = store::get_long_task(&db, &cp.task_id)?.ok_or("关联的长任务不存在")?;
        (task, cp)
    };

    // 1. 暂停当前可能在运行的任务
    let _ = pause_long_task(app, &task.id);

    // 2. 将任务回退到该检查点步数
    task.current_subtask_index = cp.step_number.saturating_sub(1);
    task.status = "paused".to_string();
    task.updated_at = store::now();

    // 3. 重置后续子任务状态为 pending
    for i in task.current_subtask_index..task.subtasks.len() {
        task.subtasks[i].status = "pending".to_string();
        task.subtasks[i].summary = None;
        task.subtasks[i].error = None;
        task.subtasks[i].verify_output = None;
    }

    {
        let db = state.db.lock().unwrap();
        let _ = db.execute(
            "DELETE FROM task_checkpoints WHERE task_id = ?1 AND step_number > ?2",
            rusqlite::params![task.id, cp.step_number as i64],
        );
        store::update_long_task(&db, &task)?;
    }

    let _ = app.emit("task:update", &task);
    Ok(task)
}

/// 更新待执行的子任务清单（支持用户在线手动编辑/增删调序）
pub fn update_task_subtasks(
    app: &AppHandle,
    task_id: &str,
    subtasks: Vec<TaskSubItem>,
) -> Result<LongTask, String> {
    let state = app.state::<crate::AppState>();
    let task = {
        let db = state.db.lock().unwrap();
        let mut t = store::get_long_task(&db, task_id)?.ok_or("长任务不存在")?;
        t.subtasks = subtasks;
        t.updated_at = store::now();
        store::update_long_task(&db, &t)?;
        t
    };
    let _ = app.emit("task:update", &task);
    Ok(task)
}

#[cfg(test)]
pub mod tests {
    pub use super::memory::tests::*;
}
