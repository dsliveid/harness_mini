pub mod loop_runner;
pub mod memory;
pub mod planner;
pub mod verifier;

pub use loop_runner::run_task_loop;
pub use memory::{build_working_memory_block, safe_truncate};
pub use planner::{decompose_goal, replan_subtasks};
pub use verifier::{create_git_snapshot, execute_verify_command};

use crate::agent;
use crate::models::{LongTask, TaskCheckpoint, TaskSubItem};
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
    if let Some(idx) = task.subtasks.iter().position(|s| s.status != "completed" && s.status != "skipped") {
        return Some(idx);
    }
    None
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

/// 启动新的长任务（主会话单轨自治推进 + 后台守护进程控制）
pub fn start_long_task(
    app: AppHandle,
    session_id: String,
    goal: String,
    max_budget_tokens: Option<u64>,
) -> Result<LongTask, String> {
    let state = app.state::<crate::AppState>();

    // 若当前会话已有正在运行或规划中的长任务，自动覆盖并平滑归档旧任务
    let _ = cancel_long_task_by_session(&app, &session_id, Some("被新发起的长任务覆盖替代"));

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
        status: "running".to_string(),
        current_subtask_index: 0,
        subtasks: vec![],
        max_budget_tokens,
        total_tokens_used: 0,
        cached_tokens_used: 0,
        cache_hit_rate: 0.0,
        prompt_tokens_used: 0,
        completion_tokens_used: 0,
        current_step: 1,
        max_steps: subtask_max_steps.max(25),
        created_at: now.clone(),
        updated_at: now,
    };

    let user_msg_id = {
        let db = state.db.lock().unwrap();
        store::create_long_task(&db, &task)?;
        let prompt_text = format!(
            "🎯 **启动长任务自治推进模式**\n\n            **总目标**：{}\n\n            【长任务自主协同准则】：\n            1. 当前会话已进入后台守护进程驱动的长任务模式，主会话享有全量历史上下文与单轨决策权；\n            2. 请依据前文完整的工程全貌与盘点结论，直接思考并展开第一阶段工作；\n            3. 【严格防 Token 膨胀机制】：若涉及大范围文件探索、具体代码编写、测试调试等重度施工，**请务必调用 `spawn_subagent` 委派给独立子沙箱执行**，主会话只接收交付工件摘要；\n            4. 当全部目标已彻底完成并通过自检验收后，请在回复末尾明确输出 `🎯 **长任务全部目标达成**`，守护进程将自动结案。",
            goal
        );
        let user_msg = store::new_message(&db, &session_id, "user", Some(prompt_text), false)?;
        let _ = app.emit("message:final", &user_msg);
        user_msg.id
    };

    let _ = app.emit("task:update", &task);

    // 立即启动主会话 Agent 轮次推进！
    agent::spawn_session_task(app.clone(), session_id.clone(), Some(user_msg_id));

    let rx = register_control(&task_id);
    let app_clone = app.clone();
    let task_clone = task.clone();

    tauri::async_runtime::spawn(async move {
        run_task_loop(app_clone, task_clone, rx).await;
    });

    Ok(task)
}

/// 暂停长任务（微秒级非阻塞暂停）
pub fn pause_long_task(app: &AppHandle, task_id: &str) -> Result<(), String> {
    set_control(task_id, true);
    let state = app.state::<crate::AppState>();
    let session_id = {
        let db = state.db.lock().unwrap();
        let mut task = store::get_long_task(&db, task_id)?.ok_or("长任务不存在")?;
        task.status = "paused".to_string();
        task.updated_at = store::now();
        store::update_long_task(&db, &task)?;
        task.session_id
    };

    // 级联非阻塞停止主会话及其实时运行的子任务
    agent::stop_session_ext(app, &session_id, true);

    crate::emit_event(
        app,
        "run:status",
        &serde_json::json!({
            "sessionId": &session_id,
            "status": "idle"
        }),
    );
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
        t.status = "running".to_string();
        t.updated_at = store::now();
        store::update_long_task(&db, &t)?;
        t
    };

    let _ = app.emit("task:update", &task);
    let rx = register_control(task_id);
    let app_clone = app.clone();
    let task_clone = task.clone();

    // 重新拉起守护推进循环
    tauri::async_runtime::spawn(async move {
        run_task_loop(app_clone, task_clone, rx).await;
    });

    // 恢复主会话推进
    let _ = agent::continue_turn_autonomous(app, &task.session_id);

    Ok(task)
}

/// 终止/取消长任务
pub fn cancel_long_task(app: &AppHandle, task_id: &str) -> Result<(), String> {
    set_control(task_id, true);
    let state = app.state::<crate::AppState>();
    let session_id = {
        let db = state.db.lock().unwrap();
        let mut task = store::get_long_task(&db, task_id)?.ok_or("长任务不存在")?;
        task.status = "cancelled".to_string();
        task.updated_at = store::now();
        store::update_long_task(&db, &task)?;
        task.session_id
    };

    agent::stop_session_ext(app, &session_id, true);

    crate::emit_event(
        app,
        "run:status",
        &serde_json::json!({
            "sessionId": &session_id,
            "status": "idle"
        }),
    );
    if let Ok(Some(task)) = {
        let db = state.db.lock().unwrap();
        store::get_long_task(&db, task_id)
    } {
        let _ = app.emit("task:update", &task);
    }
    Ok(())
}

/// 宣告完成长任务（支持 Agent 自主裁决或系统结案）
pub fn complete_long_task(
    app: &AppHandle,
    task_id: &str,
    summary: Option<&str>,
    evidence: Option<&str>,
) -> Result<LongTask, String> {
    set_control(task_id, true);
    let state = app.state::<crate::AppState>();
    let (task, cp_opt) = {
        let db = state.db.lock().unwrap();
        let mut t = store::get_long_task(&db, task_id)?.ok_or("长任务不存在")?;
        t.status = "completed".to_string();
        t.updated_at = store::now();

        for sub in &mut t.subtasks {
            if sub.status != "completed" && sub.status != "skipped" {
                sub.status = "completed".to_string();
                if sub.summary.is_none() {
                    sub.summary = summary.map(|s| s.to_string());
                }
                sub.error = None;
            }
        }
        store::update_long_task(&db, &t)?;

        let summary_text = summary.unwrap_or("长任务全部目标达成并由智能体通过实测验收");
        let evidence_text = evidence.unwrap_or("已在现场通过测试与自检验证");
        let cp = TaskCheckpoint {
            id: format!("cp-{}", uuid::Uuid::new_v4().simple()),
            task_id: t.id.clone(),
            step_number: t.current_step.max(1),
            subtask_id: None,
            status: "completed".to_string(),
            summary: summary_text.to_string(),
            working_memory: format!("【长任务结案报告】\n总目标：{}\n结论摘要：{}\n验收证据：{}", t.goal, summary_text, evidence_text),
            git_commit_hash: None,
            created_at: store::now(),
        };
        let _ = store::save_task_checkpoint(&db, &cp);

        let ws = std::path::PathBuf::from(&t.workspace_path);
        if let Some((_, meta, _)) = crate::plan::find_plan_file(&ws, &t.session_id, None, Some(&db)) {
            let _ = crate::plan::update_plan(
                &ws,
                &t.session_id,
                Some(&meta.id),
                summary_text,
                Some("completed"),
                None,
                None,
                None,
                None,
                None,
                Some(&db),
            );
        }

        (t, Some(cp))
    };

    if let Some(cp) = cp_opt {
        let _ = app.emit("task:checkpoint", &cp);
    }
    let _ = app.emit("task:update", &task);
    {
        let db = state.db.lock().unwrap();
        crate::plan::emit_board_updated(app, &db, &task.session_id);
    }
    Ok(task)
}

/// 由会话 ID 宣告完成当前活跃的长任务
pub fn complete_long_task_by_session(
    app: &AppHandle,
    session_id: &str,
    summary: Option<&str>,
    evidence: Option<&str>,
) -> Result<LongTask, String> {
    let task = get_active_task(app, session_id)?
        .ok_or_else(|| "当前会话未找到活跃的长任务守护进程".to_string())?;
    complete_long_task(app, &task.id, summary, evidence)
}

/// 由会话 ID 暂停当前活跃的长任务
pub fn pause_long_task_by_session(
    app: &AppHandle,
    session_id: &str,
    _reason: Option<&str>,
) -> Result<(), String> {
    let task = get_active_task(app, session_id)?
        .ok_or_else(|| "当前会话未找到活跃的长任务守护进程".to_string())?;
    pause_long_task(app, &task.id)
}

/// 由会话 ID 终止当前活跃的长任务
pub fn cancel_long_task_by_session(
    app: &AppHandle,
    session_id: &str,
    _reason: Option<&str>,
) -> Result<(), String> {
    let task = get_active_task(app, session_id)?
        .ok_or_else(|| "当前会话未找到活跃的长任务守护进程".to_string())?;
    cancel_long_task(app, &task.id)
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

/// 回退到指定检查点
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

    let _ = pause_long_task(app, &task.id);

    task.current_subtask_index = cp.step_number.saturating_sub(1);
    task.status = "paused".to_string();
    task.updated_at = store::now();

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

/// 更新待执行的子任务清单
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
