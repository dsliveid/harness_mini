use crate::agent;
use crate::llm::{self, LlmCfg};
use crate::models::{resolve_active_model, LongTask, LongTaskStatus, TaskCheckpoint, TaskSubItem};
use crate::store;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;
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
    if let Some(idx) = task.subtasks.iter().position(|s| s.status == "running" || s.status == "verifying") {
        return Some(idx);
    }
    if task.current_subtask_index < task.subtasks.len() {
        return Some(task.current_subtask_index);
    }
    task.subtasks.iter().position(|s| s.status == "pending" || s.status == "failed" || s.status == "interrupted")
}

fn register_control(task_id: &str) -> watch::Receiver<bool> {
    let mut map = TASK_CONTROLS.lock().unwrap();
    let controls = map.get_or_insert_with(HashMap::new);
    let (tx, rx) = watch::channel(false);
    controls.insert(task_id.to_string(), tx);
    rx
}

fn remove_control(task_id: &str) {
    let mut map = TASK_CONTROLS.lock().unwrap();
    if let Some(controls) = map.as_mut() {
        controls.remove(task_id);
    }
}

fn git_cmd() -> Command {
    let mut cmd = Command::new("git");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    cmd
}

/// 执行真实物理 Git 快照提交
fn create_git_snapshot(workspace: &Path, task_id: &str, step_num: usize) -> Option<String> {
    if !workspace.join(".git").exists() {
        return None;
    }
    // 1. git add -A
    let _ = git_cmd().current_dir(workspace).args(["add", "-A"]).output();
    // 2. git commit -m "..." --allow-empty
    let msg = format!("checkpoint: task_{task_id}_step_{step_num}");
    let _ = git_cmd()
        .current_dir(workspace)
        .args(["commit", "-m", &msg, "--allow-empty"])
        .output();
    // 3. git rev-parse HEAD
    let out = git_cmd()
        .current_dir(workspace)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if out.status.success() {
        let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !sha.is_empty() {
            return Some(sha);
        }
    }
    None
}

/// 执行自动化质量门禁命令（检测 Exit Code 与捕获输出）
async fn execute_verify_command(workspace: &Path, command_str: &str) -> (bool, String) {
    #[cfg(windows)]
    let mut c = tokio::process::Command::new("cmd");
    #[cfg(windows)]
    c.args(["/C", command_str]);

    #[cfg(not(windows))]
    let mut c = tokio::process::Command::new("sh");
    #[cfg(not(windows))]
    c.args(["-c", command_str]);

    c.current_dir(workspace);

    match c.output().await {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = if stderr.trim().is_empty() {
                stdout.to_string()
            } else if stdout.trim().is_empty() {
                stderr.to_string()
            } else {
                format!("stdout:\n{}\n\nstderr:\n{}", stdout.trim(), stderr.trim())
            };
            (output.status.success(), combined)
        }
        Err(e) => (false, format!("无法启动验证命令: {e}")),
    }
}

/// 组装工作记忆块（Working Memory Block），包含历史检查点总结
fn build_working_memory_block(checkpoints: &[TaskCheckpoint], goal: &str) -> String {
    if checkpoints.is_empty() {
        return format!("【总目标锚点】: {}\n尚未有已归档的历史阶段检查点，当前为首个启动阶段。", goal);
    }
    let mut text = format!("【总目标锚点】: {}\n\n【已完成历史阶段检查点与沉淀事实 (Working Memory)】:\n", goal);
    for cp in checkpoints {
        text.push_str(&format!(
            "- 阶段 #{}: 结论摘要 -> {}\n",
            cp.step_number,
            if cp.summary.len() > 300 {
                format!("{}…", &cp.summary[..300])
            } else {
                cp.summary.clone()
            }
        ));
    }
    text.push_str("\n⚠️ 请基于上述已完成的成果继续推进，严禁无故推翻或重复已完成的修改！");
    text
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

    let (workspace_path, task_id) = {
        let db = state.db.lock().unwrap();
        let s = store::get_session(&db, &session_id)?
            .ok_or_else(|| "会话不存在".to_string())?;
        let tid = format!("task-{}", uuid::Uuid::new_v4().simple());
        (s.workspace_path, tid)
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
        current_step: 0,
        max_steps: 50,
        created_at: now.clone(),
        updated_at: now,
    };

    {
        let db = state.db.lock().unwrap();
        store::create_long_task(&db, &task)?;
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

/// 暂停长任务
pub fn pause_long_task(app: &AppHandle, task_id: &str) -> Result<(), String> {
    set_control(task_id, true);
    let state = app.state::<crate::AppState>();
    let session_id = {
        let db = state.db.lock().unwrap();
        let mut task = store::get_long_task(&db, task_id)?.ok_or("长任务不存在")?;
        task.status = "paused".to_string();
        if let Some(cur_idx) = get_next_subtask_index(&task) {
            if task.subtasks[cur_idx].status == "running"
                || task.subtasks[cur_idx].status == "verifying"
                || task.subtasks[cur_idx].status == "in_progress"
            {
                task.subtasks[cur_idx].status = "paused".to_string();
            }
        }
        task.updated_at = store::now();
        store::update_long_task(&db, &task)?;
        task.session_id
    };
    agent::stop_session(app, &session_id);
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
        // 如果当前子任务处于 failed/verifying/interrupted/in_progress，重置为 pending 以便重新执行该子任务
        if let Some(sub) = t.subtasks.get_mut(t.current_subtask_index) {
            if sub.status == "failed" || sub.status == "in_progress" || sub.status == "verifying" || sub.status == "interrupted" {
                sub.status = "pending".to_string();
                sub.error = None;
            }
        }
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

/// 终止/取消长任务
pub fn cancel_long_task(app: &AppHandle, task_id: &str) -> Result<(), String> {
    set_control(task_id, true);
    let state = app.state::<crate::AppState>();
    let session_id = {
        let db = state.db.lock().unwrap();
        let mut task = store::get_long_task(&db, task_id)?.ok_or("长任务不存在")?;
        task.status = "cancelled".to_string();
        if let Some(cur_idx) = get_next_subtask_index(&task) {
            if task.subtasks[cur_idx].status == "running"
                || task.subtasks[cur_idx].status == "verifying"
                || task.subtasks[cur_idx].status == "in_progress"
            {
                task.subtasks[cur_idx].status = "cancelled".to_string();
            }
        }
        task.updated_at = store::now();
        store::update_long_task(&db, &task)?;
        task.session_id
    };
    agent::stop_session(app, &session_id);
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

    // 2. 如果包含 Git 物理提交且存在 .git，执行 hard reset
    if let Some(ref sha) = cp.git_commit_hash {
        let ws = Path::new(&task.workspace_path);
        if ws.join(".git").exists() {
            let _ = git_cmd().current_dir(ws).args(["reset", "--hard", sha]).output();
        }
    }

    // 3. 将任务回退到该检查点步数
    task.current_subtask_index = cp.step_number.saturating_sub(1);
    task.status = "paused".to_string();
    task.updated_at = store::now();

    // 4. 重置后续子任务状态为 pending
    for i in task.current_subtask_index..task.subtasks.len() {
        task.subtasks[i].status = "pending".to_string();
        task.subtasks[i].summary = None;
        task.subtasks[i].error = None;
        task.subtasks[i].verify_output = None;
    }

    {
        let db = state.db.lock().unwrap();
        // 清理当前检查点之后已作废的快照
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

/// 动态重规划（遇到严重阻碍或新情况时自适应重整剩余路线图）
pub async fn replan_subtasks(
    app: &AppHandle,
    task_id: &str,
    cur_idx: usize,
    reason: &str,
) -> Result<bool, String> {
    let state = app.state::<crate::AppState>();
    let (mut task, pc, model) = {
        let db = state.db.lock().unwrap();
        let master = state.master_key.lock().unwrap();
        let task = store::get_long_task(&db, task_id)?.ok_or("长任务不存在")?;
        let settings = store::get_settings_with_secrets(&db, &master).unwrap_or_default();
        let s = store::get_session(&db, &task.session_id)?.ok_or("会话不存在")?;
        let resolved = if let (Some(pid), Some(mid)) = (&s.provider_id, &s.model_id) {
            settings.providers.iter().find(|p| &p.id == pid && p.models.contains(mid))
                .map(|p| (p.clone(), mid.clone()))
        } else {
            resolve_active_model(&settings).map(|(p, m)| (p.clone(), m.to_string()))
        };
        let (pc, model) = resolved.ok_or("未配置可用模型")?;
        (task, pc, model)
    };

    let cfg = LlmCfg {
        base_url: pc.base_url,
        api_key: pc.api_key,
        model,
    };

    let cps = {
        let db = state.db.lock().unwrap();
        store::list_task_checkpoints(&db, task_id).unwrap_or_default()
    };
    let memory_block = build_working_memory_block(&cps, &task.goal);

    let cur_title = task.subtasks.get(cur_idx).map(|s| s.title.as_str()).unwrap_or("");
    let prompt = format!(
        "【长任务自适应动态重规划申请】\n总目标：{}\n\n{}\n\n当前推进至阶段 {}/{}：【{}】。\n执行过程中遇到以下阻碍或新情况：\n{}\n\n请重新审视当前工作区状态，对剩余未完成的后续子任务进行动态重规划（调整、细化或替换当前及后续子任务）。\n输出必须严格为 JSON 数组，严禁包含任何 Markdown 格式包裹或解释文本：\n[{{\"title\": \"调整后的阶段任务名称\", \"description\": \"具体任务内容与可验收成果\", \"verifyCommand\": \"可选的自动化验证命令，如 cargo check 或 npm test\"}}]",
        task.goal,
        memory_block,
        cur_idx + 1,
        task.subtasks.len(),
        cur_title,
        reason
    );

    let messages = vec![
        json!({
            "role": "system",
            "content": "你是一个严谨的架构师。你需要根据实际开发阻碍，动态重构后续执行路线图。请严格输出合法 JSON 数组。"
        }),
        json!({
            "role": "user",
            "content": prompt
        }),
    ];

    let mut buf = String::new();
    if let Ok(res) = llm::chat_stream(&cfg, &messages, &[], |d| buf.push_str(d), |_| {}).await {
        let clean = res.content.trim();
        let json_str = clean.strip_prefix("```json")
            .or_else(|| clean.strip_prefix("```"))
            .unwrap_or(clean);
        let json_str = json_str.strip_suffix("```").unwrap_or(json_str).trim();

        if let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(json_str) {
            let mut new_remains = Vec::new();
            for (offset, v) in arr.into_iter().enumerate() {
                let idx = cur_idx + offset + 1;
                let title = v.get("title").and_then(|x| x.as_str()).unwrap_or("调整任务").to_string();
                let desc = v.get("description").and_then(|x| x.as_str()).map(|s| s.to_string());
                let verify_cmd = v.get("verifyCommand").and_then(|x| x.as_str()).map(|s| s.to_string());
                new_remains.push(TaskSubItem {
                    id: format!("sub-replan-{}", idx),
                    index: idx,
                    title,
                    description: desc,
                    status: "pending".to_string(),
                    summary: None,
                    error: None,
                    verify_command: verify_cmd,
                    verify_output: None,
                });
            }
            if !new_remains.is_empty() {
                task.subtasks.truncate(cur_idx);
                task.subtasks.extend(new_remains);
                task.updated_at = store::now();
                {
                    let db = state.db.lock().unwrap();
                    store::update_long_task(&db, &task)?;
                }
                let _ = app.emit("task:update", &task);
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// 目标拆解逻辑（支持推断自动化验证门禁命令）
async fn decompose_goal(app: &AppHandle, session_id: &str, goal: &str) -> Vec<TaskSubItem> {
    let state = app.state::<crate::AppState>();
    let (session, settings) = {
        let db = state.db.lock().unwrap();
        let master = state.master_key.lock().unwrap();
        (
            store::get_session(&db, session_id).ok().flatten(),
            store::get_settings_with_secrets(&db, &master).unwrap_or_default(),
        )
    };

    let resolved_model = if let Some(ref s) = session {
        if let (Some(pid), Some(mid)) = (&s.provider_id, &s.model_id) {
            settings.providers.iter().find(|p| &p.id == pid && p.models.contains(mid))
                .map(|p| (p.clone(), mid.clone()))
        } else {
            resolve_active_model(&settings).map(|(p, m)| (p.clone(), m.to_string()))
        }
    } else {
        resolve_active_model(&settings).map(|(p, m)| (p.clone(), m.to_string()))
    };

    if let Some((pc, model)) = resolved_model {
        let cfg = LlmCfg {
            base_url: pc.base_url,
            api_key: pc.api_key,
            model,
        };

        let sys_prompt = "你是一个专家级 AI 架构师与长任务规划专家。用户将提供一个复杂的开发/编码任务总目标。请你将该目标拆解为 3 至 5 个按逻辑依赖顺序推进、目标明确且可独立执行验收的子任务步骤。\n请为每个适用的子任务推断最恰当的自动化验证命令（verifyCommand，例如 npm test, cargo check, pytest 等）。\n严禁输出任何 Markdown 格式包裹（如不要带 ```json 标记），仅输出纯 JSON 数组。\n格式必须严格符合：\n[{\"title\": \"子任务名称\", \"description\": \"具体任务内容与可验收成果\", \"verifyCommand\": \"可选的自动化验证命令\"}]";

        let messages = vec![
            json!({ "role": "system", "content": sys_prompt }),
            json!({ "role": "user", "content": goal }),
        ];

        let mut buf = String::new();
        if let Ok(res) = llm::chat_stream(&cfg, &messages, &[], |d| buf.push_str(d), |_| {}).await {
            let clean = res.content.trim();
            let json_str = clean.strip_prefix("```json")
                .or_else(|| clean.strip_prefix("```"))
                .unwrap_or(clean);
            let json_str = json_str.strip_suffix("```").unwrap_or(json_str).trim();

            if let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(json_str) {
                let mut items = Vec::new();
                for (i, v) in arr.into_iter().enumerate() {
                    let title = v.get("title").and_then(|x| x.as_str()).unwrap_or("未命名任务").to_string();
                    let desc = v.get("description").and_then(|x| x.as_str()).map(|s| s.to_string());
                    let verify_cmd = v.get("verifyCommand").and_then(|x| x.as_str()).map(|s| s.to_string());
                    items.push(TaskSubItem {
                        id: format!("sub-{}", i + 1),
                        index: i + 1,
                        title,
                        description: desc,
                        status: "pending".to_string(),
                        summary: None,
                        error: None,
                        verify_command: verify_cmd,
                        verify_output: None,
                    });
                }
                if !items.is_empty() {
                    return items;
                }
            }
        }
    }

    // 默认兜底拆解
    vec![
        TaskSubItem {
            id: "sub-1".to_string(),
            index: 1,
            title: "梳理工作区现状与代码依赖".to_string(),
            description: Some("探索相关文件结构与当前逻辑，明确修改范围与实现思路".to_string()),
            status: "pending".to_string(),
            summary: None,
            error: None,
            verify_command: None,
            verify_output: None,
        },
        TaskSubItem {
            id: "sub-2".to_string(),
            index: 2,
            title: "执行核心功能编码与修改".to_string(),
            description: Some("根据梳理结果修改或创建核心代码与配置".to_string()),
            status: "pending".to_string(),
            summary: None,
            error: None,
            verify_command: None,
            verify_output: None,
        },
        TaskSubItem {
            id: "sub-3".to_string(),
            index: 3,
            title: "执行检查与结果验收".to_string(),
            description: Some("验证修改后的代码与功能是否正常，产出交付总结".to_string()),
            status: "pending".to_string(),
            summary: None,
            error: None,
            verify_command: None,
            verify_output: None,
        },
    ]
}

/// 长任务主调度执行循环（含工作记忆注入、质量门禁校验、物理快照与自适应自纠）
async fn run_task_loop(app: AppHandle, mut task: LongTask, rx: watch::Receiver<bool>) {
    let state = app.state::<crate::AppState>();

    // 1. Planning 阶段：若子任务尚未拆解，先拆解
    if task.subtasks.is_empty() {
        let subtasks = decompose_goal(&app, &task.session_id, &task.goal).await;
        task.subtasks = subtasks;
        task.status = "running".to_string();
        task.updated_at = store::now();

        {
            let db = state.db.lock().unwrap();
            let _ = store::update_long_task(&db, &task);
        }
        let _ = app.emit("task:update", &task);

        // 在会话中注入一条助手消息，展示已拆解的计划
        let mut plan_msg = format!("📋 **长任务目标已确立并完成任务拆解**\n\n**总目标**：{}\n\n**执行路线图**：\n", task.goal);
        for sub in &task.subtasks {
            let verify_info = if let Some(ref v) = sub.verify_command {
                format!(" `[验证: {}]`", v)
            } else {
                String::new()
            };
            plan_msg.push_str(&format!("{}. **{}**{}：{}\n", sub.index, sub.title, verify_info, sub.description.as_deref().unwrap_or("")));
        }
        plan_msg.push_str("\n🚀 开始自动推进第 1 阶段...");
        {
            let db = state.db.lock().unwrap();
            let _ = store::new_message(&db, &task.session_id, "assistant", Some(plan_msg), false);
        }
    }

    // 2. 推进各子任务
    while task.current_subtask_index < task.subtasks.len() {
        if *rx.borrow() {
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

        let cur_idx = task.current_subtask_index;
        task.subtasks[cur_idx].status = "in_progress".to_string();
        task.current_step = cur_idx + 1;
        task.status = "running".to_string();
        task.updated_at = store::now();
        {
            let db = state.db.lock().unwrap();
            let _ = store::update_long_task(&db, &task);
        }
        let _ = app.emit("task:update", &task);

        let subtask = task.subtasks[cur_idx].clone();

        // 提取历史检查点并生成工作记忆（Working Memory）
        let working_memory = {
            let db = state.db.lock().unwrap();
            let cps = store::list_task_checkpoints(&db, &task.id).unwrap_or_default();
            build_working_memory_block(&cps, &task.goal)
        };

        let mut verify_instruction = String::new();
        if let Some(ref vc) = subtask.verify_command {
            verify_instruction = format!("\n4. 本阶段挂载了自动化验证门禁：`{}`，请务必保证相关代码在执行该命令时语法与测试通过。", vc);
        }

        let prompt = format!(
            "【长任务自主推进阶段 {}/{}】: {}\n\n{}\n\n目标详情: {}\n\n【自主执行指令】:\n1. 请独立调用相关工具完成本阶段任务；\n2. 产出具体改动或验证结论，严禁停留在空泛设想；\n3. 完成后在最终答复中明确总结本次子任务所完成的具体成果。{}",
            task.current_step,
            task.subtasks.len(),
            subtask.title,
            working_memory,
            subtask.description.as_deref().unwrap_or(""),
            verify_instruction
        );

        // 向会话插入触发消息
        let user_msg = {
            let db = state.db.lock().unwrap();
            match store::new_message(&db, &task.session_id, "user", Some(prompt), false) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("长任务插入触发消息失败: {e}");
                    task.subtasks[cur_idx].status = "failed".to_string();
                    task.subtasks[cur_idx].error = Some(e);
                    task.status = "failed".to_string();
                    let _ = store::update_long_task(&db, &task);
                    let _ = app.emit("task:update", &task);
                    return;
                }
            }
        };
        let _ = app.emit("message:final", &user_msg);

        // 启动 Agent 执行
        agent::spawn_session_task(app.clone(), task.session_id.clone(), Some(user_msg.id));

        // 等待轮次完成
        tokio::time::sleep(Duration::from_millis(500)).await;
        while agent::is_run_active(&state, &task.session_id) {
            if *rx.borrow() {
                agent::stop_session(&app, &task.session_id);
                task.subtasks[cur_idx].status = "interrupted".to_string();
                task.status = "interrupted".to_string();
                task.updated_at = store::now();
                {
                    let db = state.db.lock().unwrap();
                    let _ = store::update_long_task(&db, &task);
                }
                let _ = app.emit("task:update", &task);
                return;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }

        if *rx.borrow() {
            task.subtasks[cur_idx].status = "interrupted".to_string();
            task.status = "interrupted".to_string();
            task.updated_at = store::now();
            {
                let db = state.db.lock().unwrap();
                let _ = store::update_long_task(&db, &task);
            }
            let _ = app.emit("task:update", &task);
            return;
        }

        // 检查会话最新运行状态或长任务是否被中断：若非正常完成（done），绝不能把该子任务推进为 completed！
        let (is_interrupted, last_status) = {
            let db = state.db.lock().unwrap();
            let lr = store::get_last_run_status(&db, &task.session_id);
            let tdb = store::get_long_task(&db, &task.id).ok().flatten();
            let is_paused_or_cancelled = tdb
                .as_ref()
                .map(|t| t.status == "interrupted" || t.status == "cancelled" || t.status == "paused")
                .unwrap_or(false);
            (is_paused_or_cancelled, lr)
        };

        if is_interrupted || last_status.as_deref() != Some("done") {
            let is_failed = last_status.as_deref() == Some("failed");
            let new_st = if is_failed { "failed" } else { "interrupted" };
            task.subtasks[cur_idx].status = new_st.to_string();
            task.status = new_st.to_string();
            task.updated_at = store::now();
            {
                let db = state.db.lock().unwrap();
                let _ = store::update_long_task(&db, &task);
            }
            let _ = app.emit("task:update", &task);
            return;
        }

        // 阶段三：硬性自动化质量门禁执行与反思自愈闭环
        let ws_path = PathBuf::from(&task.workspace_path);
        let mut verification_passed = true;
        let mut verify_res_output = String::new();

        if let Some(ref v_cmd) = subtask.verify_command {
            task.subtasks[cur_idx].status = "verifying".to_string();
            let _ = app.emit("task:update", &task);

            let (passed, out) = execute_verify_command(&ws_path, v_cmd).await;
            verification_passed = passed;
            verify_res_output = out.clone();
            task.subtasks[cur_idx].verify_output = Some(out);

            // 若验证未通过，尝试触发一次定向修复
            if !verification_passed {
                let fix_prompt = format!(
                    "【🚨 自动化质量门禁校验未通过】\n验证命令 `{}` 执行失败，报错如下：\n```\n{}\n```\n请立即根据上述报错信息自主调用工具排查并修复代码缺陷，确保验证命令通过！",
                    v_cmd,
                    if verify_res_output.len() > 1500 { format!("{}…[截断]", &verify_res_output[..1500]) } else { verify_res_output.clone() }
                );
                let fix_msg = {
                    let db = state.db.lock().unwrap();
                    store::new_message(&db, &task.session_id, "user", Some(fix_prompt), false).ok()
                };
                if let Some(fm) = fix_msg {
                    let _ = app.emit("message:final", &fm);
                    agent::spawn_session_task(app.clone(), task.session_id.clone(), Some(fm.id));

                    tokio::time::sleep(Duration::from_millis(500)).await;
                    while agent::is_run_active(&state, &task.session_id) {
                        if *rx.borrow() {
                            agent::stop_session(&app, &task.session_id);
                            return;
                        }
                        tokio::time::sleep(Duration::from_millis(300)).await;
                    }
                    // 二次复测
                    let (re_passed, re_out) = execute_verify_command(&ws_path, v_cmd).await;
                    verification_passed = re_passed;
                    task.subtasks[cur_idx].verify_output = Some(re_out);
                }
            }
        }

        if !verification_passed {
            // 验证仍失败，触发动态重规划或暂停提醒
            task.subtasks[cur_idx].status = "failed".to_string();
            task.subtasks[cur_idx].error = Some("自动化质量门禁校验未通过".to_string());
            let replanned = replan_subtasks(&app, &task.id, cur_idx, &verify_res_output).await.unwrap_or(false);
            if !replanned {
                task.status = "paused".to_string();
                let _ = app.emit("task:update", &task);
                return;
            }
            // 若成功重规划，跳过本步直接继续推进调整后的任务
            continue;
        }

        // 轮次结束，读取助手回复
        let (summary, total_tokens) = {
            let db = state.db.lock().unwrap();
            let msgs = store::all_messages(&db, &task.session_id).unwrap_or_default();
            let last_asst = msgs.iter().rev().find(|m| m.role == "assistant");
            let sum = last_asst.and_then(|m| m.content.clone()).unwrap_or_else(|| "本阶段已顺利执行完毕。".to_string());
            let tok_sum: u64 = msgs.iter().filter_map(|m| m.total_tokens).sum();
            (sum, tok_sum)
        };

        // 阶段四：物理 Git 检查点生成
        let git_hash = create_git_snapshot(&ws_path, &task.id, cur_idx + 1);

        // 创建 Checkpoint 快照
        let cp = TaskCheckpoint {
            id: format!("cp-{}", uuid::Uuid::new_v4().simple()),
            task_id: task.id.clone(),
            step_number: cur_idx + 1,
            subtask_id: Some(subtask.id.clone()),
            status: "completed".to_string(),
            summary: summary.clone(),
            working_memory: format!("阶段 [{}] {} 已完成。产出摘要：{}", cur_idx + 1, subtask.title, summary),
            git_commit_hash: git_hash,
            created_at: store::now(),
        };
        {
            let db = state.db.lock().unwrap();
            let _ = store::save_task_checkpoint(&db, &cp);
        }
        let _ = app.emit("task:checkpoint", &cp);

        // 标记该子任务完成
        task.subtasks[cur_idx].status = "completed".to_string();
        task.subtasks[cur_idx].summary = Some(summary);
        task.total_tokens_used = total_tokens;
        task.current_subtask_index += 1;
        task.updated_at = store::now();

        {
            let db = state.db.lock().unwrap();
            let _ = store::update_long_task(&db, &task);
        }
        let _ = app.emit("task:update", &task);

        tokio::time::sleep(Duration::from_millis(800)).await;
    }

    // 3. 全部完成
    task.status = "completed".to_string();
    task.updated_at = store::now();
    {
        let db = state.db.lock().unwrap();
        let _ = store::update_long_task(&db, &task);
        let final_text = format!(
            "🎉 **长任务全部子任务已顺利达成！**\n\n**总目标**：{}\n已通过全部 {} 个阶段性验证检查点，代码与成果已完成归档。",
            task.goal,
            task.subtasks.len()
        );
        let _ = store::new_message(&db, &task.session_id, "assistant", Some(final_text), false);
    }
    let _ = app.emit("task:update", &task);
    let _ = app.emit("task:finished", json!({ "taskId": task.id, "status": "completed", "goal": task.goal }));
    let _ = app.emit("task:notification", json!({ "title": "长任务达成", "body": format!("目标【{}】已顺利完成！", task.goal) }));
    remove_control(&task.id);
}
