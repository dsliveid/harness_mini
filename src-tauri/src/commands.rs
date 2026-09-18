use crate::agent;
use crate::llm::{self, LlmCfg};
use crate::models::*;
use crate::store;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager, State};

fn is_run_active(state: &crate::AppState, session_id: &str) -> bool {
    agent::is_run_active(state, session_id)
}

fn emit_session(state: &crate::AppState, app: &AppHandle, session_id: &str) {
    let db = state.db.lock().unwrap();
    if let Ok(Some(s)) = store::get_session(&db, session_id) {
        let _ = app.emit("session:update", &s);
    }
}

fn emit_queue(state: &crate::AppState, app: &AppHandle, session_id: &str) {
    let items = {
        let db = state.db.lock().unwrap();
        store::list_queued(&db, session_id).unwrap_or_default()
    };
    let _ = app.emit(
        "queue:update",
        json!({"sessionId": session_id, "items": items
            .iter()
            .map(|m| json!({"id": m.id, "content": m.content.clone().unwrap_or_default(), "createdAt": m.created_at}))
            .collect::<Vec<_>>()}),
    );
}

// ---------- settings ----------

/// 广播设置变更（含审批规则），前端据此刷新缓存中的设置与规则列表
pub fn emit_settings_changed(state: &crate::AppState) {
    let db = state.db.lock().unwrap();
    let master = state.master_key.lock().unwrap();
    if let Ok(settings) = store::get_settings_with_secrets(&db, &master) {
        state.emit("settings:changed", &settings);
    }
}

#[tauri::command]
pub fn get_settings(state: State<'_, crate::AppState>) -> Result<SettingsData, String> {
    let db = state.db.lock().unwrap();
    let master = state.master_key.lock().unwrap();
    store::get_settings_with_secrets(&db, &master)
}

#[tauri::command]
pub fn set_settings(state: State<'_, crate::AppState>, settings: SettingsData) -> Result<(), String> {
    let db = state.db.lock().unwrap();
    let master = state.master_key.lock().unwrap();
    let old = store::get_settings(&db)?;
    let mut clean = settings.clone();
    clean.normalize();
    for p in clean.providers.iter_mut() {
        if !p.api_key.is_empty() {
            // AES-GCM 加密后入 secrets 表，JSON 不落明文
            store::secret_set(&db, &master, &p.id, &p.api_key)?;
            p.api_key = String::new();
        }
    }
    // 清理已删除 Provider 的凭据
    for old_p in &old.providers {
        if !clean.providers.iter().any(|p| p.id == old_p.id) {
            store::secret_delete(&db, &old_p.id)?;
        }
    }
    store::save_settings(&db, &clean)?;
    drop(master);
    drop(db);
    emit_settings_changed(&state);
    Ok(())
}

#[tauri::command]
pub fn list_tools() -> Result<Vec<ToolInfo>, String> {
    let specs = crate::tools::tool_specs();
    let list = specs
        .into_iter()
        .map(|s| ToolInfo {
            name: s.name.to_string(),
            description: s.description.to_string(),
            risk: s.risk.as_str().to_string(),
            is_temp: crate::tools::TEMP_TOOL_NAMES.contains(&s.name),
        })
        .collect();
    Ok(list)
}

#[tauri::command]
pub async fn test_provider(
    state: State<'_, crate::AppState>,
    provider: ProviderCfg,
) -> Result<String, String> {
    let mut key = provider.api_key.clone();
    if key.is_empty() {
        let db = state.db.lock().unwrap();
        let master = state.master_key.lock().unwrap();
        if let Ok(Some(k)) = store::secret_get(&db, &master, &provider.id) {
            key = k;
        }
    }
    let model = provider
        .models
        .first()
        .cloned()
        .filter(|m| !m.is_empty())
        .unwrap_or(provider.model.clone());
    if model.is_empty() {
        return Err("请先为该厂商添加模型".into());
    }
    let cfg = LlmCfg {
        base_url: provider.base_url.clone(),
        api_key: key,
        model,
    };
    llm::test_connection(&cfg).await
}

// ---------- projects ----------

#[tauri::command]
pub fn list_projects(state: State<'_, crate::AppState>) -> Result<Vec<Project>, String> {
    let db = state.db.lock().unwrap();
    store::list_projects(&db)
}

#[tauri::command]
pub fn create_project(state: State<'_, crate::AppState>, app: AppHandle, name: String, path: Option<String>) -> Result<Project, String> {
    let db = state.db.lock().unwrap();
    let path = path.as_deref().map(str::trim).filter(|s| !s.is_empty());
    // 未指定名称时取目录名
    let name = if name.trim().is_empty() {
        path.map(store::dir_name_of)
            .filter(|s| !s.is_empty())
            .ok_or("项目名称不能为空")?
    } else {
        name.trim().to_string()
    };
    let p = store::create_project(&db, &name, path)?;
    drop(db);
    let _ = app.emit("projects:changed", &p);
    Ok(p)
}

#[tauri::command]
pub fn remove_project(state: State<'_, crate::AppState>, app: AppHandle, id: String) -> Result<(), String> {
    let db = state.db.lock().unwrap();
    store::remove_project(&db, &id)?;
    drop(db);
    let _ = app.emit("projects:changed", &json!({"removed": id}));
    Ok(())
}

#[tauri::command]
pub fn set_project_pinned(
    state: State<'_, crate::AppState>,
    app: AppHandle,
    id: String,
    pinned: bool,
) -> Result<(), String> {
    let project = {
        let db = state.db.lock().unwrap();
        store::set_project_pinned(&db, &id, pinned)?;
        store::get_project(&db, &id)?.ok_or("项目不存在")?
    };
    let _ = app.emit("projects:changed", &project);
    Ok(())
}

// ---------- 项目设置（项目约束 / 关联项目） ----------

#[tauri::command]
pub fn set_project_constraints(
    state: State<'_, crate::AppState>,
    app: AppHandle,
    id: String,
    constraints: String,
) -> Result<(), String> {
    let project = {
        let db = state.db.lock().unwrap();
        store::set_project_constraints(&db, &id, &constraints)?;
        store::get_project(&db, &id)?.ok_or("项目不存在")?
    };
    let _ = app.emit("projects:changed", &project);
    Ok(())
}

#[tauri::command]
pub fn list_project_links(
    state: State<'_, crate::AppState>,
    project_id: String,
) -> Result<Vec<ProjectLink>, String> {
    let db = state.db.lock().unwrap();
    store::list_project_links(&db, &project_id)
}

#[tauri::command]
pub fn add_project_link(
    state: State<'_, crate::AppState>,
    project_id: String,
    path: String,
    description: String,
) -> Result<ProjectLink, String> {
    let db = state.db.lock().unwrap();
    store::add_project_link(&db, &project_id, &path, &description)
}

#[tauri::command]
pub fn update_project_link(
    state: State<'_, crate::AppState>,
    id: String,
    path: String,
    description: String,
) -> Result<ProjectLink, String> {
    let db = state.db.lock().unwrap();
    store::update_project_link(&db, &id, &path, &description)
}

#[tauri::command]
pub fn delete_project_link(state: State<'_, crate::AppState>, id: String) -> Result<(), String> {
    let db = state.db.lock().unwrap();
    store::delete_project_link(&db, &id)
}

// ---------- sessions ----------

#[tauri::command]
pub fn list_sessions(state: State<'_, crate::AppState>) -> Result<Vec<Session>, String> {
    let db = state.db.lock().unwrap();
    store::list_sessions(&db, "active")
}

#[tauri::command]
pub fn list_archived(state: State<'_, crate::AppState>) -> Result<Vec<Session>, String> {
    let db = state.db.lock().unwrap();
    store::list_sessions(&db, "archived")
}

#[tauri::command]
pub fn rename_session(state: State<'_, crate::AppState>, app: AppHandle, id: String, title: String) -> Result<(), String> {
    let db = state.db.lock().unwrap();
    store::rename_session(&db, &id, title.trim())?;
    drop(db);
    emit_session(&state, &app, &id);
    Ok(())
}

#[tauri::command]
pub fn delete_session(state: State<'_, crate::AppState>, app: AppHandle, id: String) -> Result<(), String> {
    ensure_temp_cleared(&state, &id)?;
    if is_run_active(&state, &id) {
        agent::stop_session(&app, &id);
    }
    let db = state.db.lock().unwrap();
    store::delete_session(&db, &id)?;
    drop(db);
    let _ = app.emit("sessions:changed", json!({"deleted": id}));
    Ok(())
}

#[tauri::command]
pub fn archive_session(state: State<'_, crate::AppState>, app: AppHandle, id: String) -> Result<(), String> {
    // 与删除同一流程：临时空间仍在时禁止归档，须先清空
    ensure_temp_cleared(&state, &id)?;
    let db = state.db.lock().unwrap();
    store::set_session_status(&db, &id, "archived")?;
    drop(db);
    emit_session(&state, &app, &id);
    let _ = app.emit("sessions:changed", json!({"archived": id}));
    Ok(())
}

/// 生命周期守卫：临时空间对话在其临时空间目录仍存在时不允许删除/归档
fn ensure_temp_cleared(state: &crate::AppState, id: &str) -> Result<(), String> {
    let db = state.db.lock().unwrap();
    if let Some(s) = store::get_session(&db, id)? {
        if s.is_temp {
            if let Some(r) = s.temp_root.as_deref() {
                let data_dir = state.data_dir.lock().unwrap().clone();
                let p = if let Some(d) = data_dir.as_ref() {
                    crate::temp::adapt_temp_path(Path::new(r), d)
                } else {
                    PathBuf::from(r)
                };
                if p.exists() {
                    return Err("该对话存在临时空间，请先清空临时空间后再删除/归档".into());
                }
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub fn unarchive_session(state: State<'_, crate::AppState>, app: AppHandle, id: String) -> Result<(), String> {
    let db = state.db.lock().unwrap();
    store::set_session_status(&db, &id, "active")?;
    drop(db);
    emit_session(&state, &app, &id);
    let _ = app.emit("sessions:changed", json!({"unarchived": id}));
    Ok(())
}

#[tauri::command]
pub fn set_session_mode(state: State<'_, crate::AppState>, app: AppHandle, id: String, mode: String) -> Result<(), String> {
    // 访问模式为会话级：仅接受具体的 confirm / full_access（规范化后落库）
    let mode = crate::models::normalize_access_mode(&mode);
    {
        let db = state.db.lock().unwrap();
        store::set_session_mode(&db, &id, &mode)?;
    }
    // 切到「完全访问」：立即放行该对话下已经弹出的审批条。
    // 运行中的 Agent 会在下一次工具调用判定时实时读到新模式；此处只处理已经挂起的
    // 审批请求，否则它们会一直挂到超时（5 分钟）才被拒绝。
    if mode == "full_access" {
        resolve_pending_approvals(&state, &app, &id);
    }
    emit_session(&state, &app, &id);
    Ok(())
}

#[tauri::command]
pub fn set_session_context_limit(
    state: State<'_, crate::AppState>,
    app: AppHandle,
    id: String,
    limit: Option<usize>,
) -> Result<(), String> {
    {
        let db = state.db.lock().unwrap();
        store::set_session_context_limit(&db, &id, limit)?;
    }
    emit_session(&state, &app, &id);
    Ok(())
}

/// 放行指定对话下所有挂起的审批请求（切换到「完全访问」时调用）
fn resolve_pending_approvals(state: &crate::AppState, app: &AppHandle, session_id: &str) {
    let pending: Vec<(String, tokio::sync::oneshot::Sender<crate::approval::Decision>)> = {
        let mut map = state.approvals.lock().unwrap();
        let ids: Vec<String> = map
            .iter()
            .filter(|(_, p)| p.session_id == session_id)
            .map(|(k, _)| k.clone())
            .collect();
        ids.into_iter()
            .filter_map(|k| map.remove(&k).map(|p| (k, p.tx)))
            .collect()
    };
    for (event_id, tx) in pending {
        let _ = tx.send(crate::approval::Decision::AllowOnce);
        let _ = app.emit(
            "approval:resolved",
            json!({"eventId": event_id, "decision": "allow_once"}),
        );
    }
}

/// 某对话的审批规则列表（会话设置面板使用）
#[tauri::command]
pub fn list_session_rules(
    state: State<'_, crate::AppState>,
    session_id: String,
) -> Result<Vec<ApprovalRule>, String> {
    let db = state.db.lock().unwrap();
    store::list_session_rules(&db, &session_id)
}

/// 删除某对话的一条审批规则；返回剩余规则并广播，保证运行中的对话也能即时刷新
#[tauri::command]
pub fn delete_session_rule(
    state: State<'_, crate::AppState>,
    app: AppHandle,
    session_id: String,
    id: String,
) -> Result<Vec<ApprovalRule>, String> {
    let rules = {
        let db = state.db.lock().unwrap();
        store::delete_session_rule(&db, &session_id, &id)?
    };
    let _ = app.emit(
        "session:rules",
        json!({"sessionId": session_id, "rules": &rules}),
    );
    Ok(rules)
}

#[tauri::command]
pub fn set_session_workspace(state: State<'_, crate::AppState>, _app: AppHandle, id: String, path: String) -> Result<(), String> {
    // 工作区只读：会话在首次发送转正时固定工作区，已保存的对话不允许切换工作区
    let _ = path;
    let db = state.db.lock().unwrap();
    if store::get_session(&db, &id)?.is_none() {
        return Err("会话不存在".into());
    }
    Err("对话已保存，工作空间为只读，不允许切换工作空间".into())
}

/// 仅调整会话所属项目（用于绑定未带目录的项目）
#[tauri::command]
pub fn set_session_project(state: State<'_, crate::AppState>, app: AppHandle, id: String, project_id: Option<String>) -> Result<(), String> {
    {
        let db = state.db.lock().unwrap();
        if let Some(pid) = project_id.as_deref() {
            store::get_project(&db, pid)?.ok_or("项目不存在")?;
        }
        store::set_session_project(&db, &id, project_id.as_deref())?;
    }
    emit_session(&state, &app, &id);
    Ok(())
}

#[tauri::command]
pub fn get_messages(
    state: State<'_, crate::AppState>,
    session_id: String,
    before_seq: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<Message>, String> {
    let db = state.db.lock().unwrap();
    store::get_messages(&db, &session_id, before_seq, limit.unwrap_or(200))
}

// ---------- 消息发送 / 队列 / 重发 ----------

#[tauri::command]
pub fn send_message(
    state: State<'_, crate::AppState>,
    app: AppHandle,
    session_id: Option<String>,
    text: String,
    workspace_path: Option<String>,
    project_id: Option<String>,
    temp: Option<TempAlloc>,
    access_mode: Option<String>,
    context_token_limit: Option<usize>,
) -> Result<SendResult, String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("消息不能为空".into());
    }
    // 访问模式为会话级：新对话落库时由前端传入（继承“上一条对话”或默认值）
    let access_mode = crate::models::normalize_access_mode(access_mode.as_deref().unwrap_or("confirm"));
    let (result, queued) = {
        let db = state.db.lock().unwrap();
        // 临时对话：首次发送时才真正落库
        let sid = match session_id {
            Some(id) => id,
            None if temp.is_some() => {
                // 临时空间对话：落库为临时会话。工作区 = 主项目临时副本；
                // 归属原项目；绝不按临时路径自动创建项目；不记住 last_workspace_path
                let t = temp.unwrap();
                let proj_id = project_id.ok_or("临时空间对话缺少归属项目")?;
                store::get_project(&db, &proj_id)?.ok_or("归属项目不存在")?;
                let data_dir = state
                    .data_dir
                    .lock()
                    .unwrap()
                    .clone()
                    .ok_or("数据目录不可用")?;
                let root = PathBuf::from(&t.root);
                crate::temp::validate_root(&root, &data_dir)?;
                for p in &t.projects {
                    if !crate::temp::is_under(Path::new(&p.temp), &root) {
                        return Err(format!("临时空间路径非法: {}", p.temp));
                    }
                }
                let title: String = text.chars().take(24).collect();
                let s = store::create_session(&db, &t.main_temp, Some(&proj_id), &title, &access_mode)?;
                if let Some(lim) = context_token_limit {
                    let _ = store::set_session_context_limit(&db, &s.id, Some(lim));
                }
                store::set_session_temp(&db, &s.id, &t.code, &t.root, &t.source_workspace)?;
                crate::temp::save_manifest(
                    &db,
                    &s.id,
                    &TempManifest {
                        code: t.code.clone(),
                        root: t.root.clone(),
                        projects: t.projects.clone(),
                    },
                )?;
                let _ = app.emit("sessions:changed", json!({"created": s.id}));
                s.id
            }
            None => {
                // 空字符串 = 未绑定工作区（纯对话模式），草稿的工作区由前端显式传入
                let ws = workspace_path.clone().unwrap_or_default();
                let title: String = text.chars().take(24).collect();
                // 工作区即项目：落库时按工作区路径关联项目；尚无该项目则自动创建一条项目数据
                let project_id = if ws.is_empty() {
                    project_id
                } else {
                    let p = store::find_or_create_project_by_path(&db, &ws)?;
                    let pid = p.id.clone();
                    let _ = app.emit("projects:changed", &p);
                    Some(pid)
                };
                let s = store::create_session(&db, &ws, project_id.as_deref(), &title, &access_mode)?;
                if let Some(lim) = context_token_limit {
                    let _ = store::set_session_context_limit(&db, &s.id, Some(lim));
                }
                let _ = app.emit("sessions:changed", json!({"created": s.id}));
                s.id
            }
        };
        let active = is_run_active(&state, &sid);
        let msg = store::new_message(&db, &sid, "user", Some(text), active)?;
        let msg_id = msg.id.clone();
        store::touch_session(&db, &sid)?;
        // 记住最近工作区（未绑定工作区的纯对话不覆盖；临时空间路径不记住）
        let mut session_row: Option<Session> = None;
        if let Ok(Some(s)) = store::get_session(&db, &sid) {
            let ws = s.workspace_path.clone();
            let is_temp = s.is_temp;
            let mut settings = store::get_settings(&db).unwrap_or_default();
            if !ws.is_empty() && !is_temp {
                settings.last_workspace_path = Some(ws);
            }
            if let Ok(json) = serde_json::to_string(&settings) {
                let _ = db.execute(
                    "INSERT INTO settings(key, value) VALUES('app_settings', ?1)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    rusqlite::params![json],
                );
            }
            let _ = app.emit("session:update", &s);
            session_row = Some(s);
        }
        let r = SendResult { session_id: sid, message_id: msg_id.clone(), queued: active, session: session_row };
        (r, (msg, active, msg_id))
    };
    let (msg, queued_flag, msg_id) = queued;
    let _ = app.emit("message:final", &msg);
    if queued_flag {
        emit_queue(&state, &app, &result.session_id);
    } else {
        agent::spawn_session_task(app.clone(), result.session_id.clone(), Some(msg_id));
    }
    Ok(result)
}

#[tauri::command]
pub fn list_queued(state: State<'_, crate::AppState>, session_id: String) -> Result<Vec<Message>, String> {
    let db = state.db.lock().unwrap();
    store::list_queued(&db, &session_id)
}

/// 引导：将待执行消息立即注入当前运行（下一轮次边界生效）
#[tauri::command]
pub fn guide_message(state: State<'_, crate::AppState>, app: AppHandle, session_id: String, message_id: String) -> Result<(), String> {
    let msg = {
        let db = state.db.lock().unwrap();
        store::dequeue_message(&db, &message_id)?
    };
    if let Some(m) = msg {
        let _ = app.emit("message:final", &m);
        emit_queue(&state, &app, &session_id);
    }
    // 会话空闲时（竞态）直接作为触发消息运行
    if !is_run_active(&state, &session_id) {
        agent::spawn_session_task(app, session_id, Some(message_id));
    }
    Ok(())
}

#[tauri::command]
pub fn delete_queued_message(state: State<'_, crate::AppState>, app: AppHandle, session_id: String, message_id: String) -> Result<(), String> {
    {
        let db = state.db.lock().unwrap();
        store::remove_queued(&db, &message_id)?;
    }
    emit_queue(&state, &app, &session_id);
    Ok(())
}

#[tauri::command]
pub fn stop_run(app: AppHandle, session_id: String) -> Result<(), String> {
    agent::stop_session(&app, &session_id);
    Ok(())
}

#[tauri::command]
pub fn list_subagents(state: State<'_, crate::AppState>, parent_session_id: String) -> Result<Vec<Session>, String> {
    let db = state.db.lock().unwrap();
    store::list_subagents(&db, &parent_session_id)
}

#[tauri::command]
pub fn spawn_subagent(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    parent_session_id: String,
    role: String,
    title: String,
    task: String,
    subpath: Option<String>,
    workspace_path: Option<String>,
) -> Result<Session, String> {
    let parent = {
        let db = state.db.lock().unwrap();
        store::get_session(&db, &parent_session_id)?.ok_or("父会话不存在")?
    };
    if parent.session_type == "subagent" || parent.parent_session_id.is_some() {
        return Err("子 Agent 不允许再创建子 Agent".into());
    }

    // 关键修复：确定子 Agent 的物理工作区根目录
    // 1. 若显式指定了 workspace_path，使用指定的独立根目录；
    // 2. 缺省时严格继承父会话的完整工作区根目录，绝不能将 subpath 拼接到根目录上！
    let sub_workspace = if let Some(ref ws) = workspace_path.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        let p = std::path::Path::new(ws);
        if p.is_absolute() {
            ws.to_string()
        } else {
            std::path::Path::new(&parent.workspace_path).join(p).to_string_lossy().to_string()
        }
    } else {
        parent.workspace_path.clone()
    };

    let (sub, user_msg) = {
        let db = state.db.lock().unwrap();
        let sub = store::create_subagent_session(
            &db,
            &parent_session_id,
            &role,
            &title,
            &task,
            &sub_workspace,
            parent.access_mode.as_deref(),
            parent.project_id.as_deref(),
        )?;
        let focus_info = subpath.as_ref().map(|p| {
            format!("\n\n重点关注子目录：`{p}`\n（提示：请优先深入该子目录开展探索与修改；当前工作区根目录依然为完整的 `{sub_workspace}`，根目录下的全局构建与配置文件如 pom.xml / package.json / README 等均在合法访问范围内，需要时可直接读取）")
        }).unwrap_or_default();
        let initial_prompt = format!(
            "【子 Agent 协作任务】\n角色定位：{role}\n任务标题：{title}\n工作区根目录：{sub_workspace}\n\n详细需求描述：\n{task}{focus_info}"
        );
        let user_msg = store::new_message(&db, &sub.id, "user", Some(initial_prompt), false)?;
        (sub, user_msg)
    };

    agent::spawn_session_task(app.clone(), sub.id.clone(), Some(user_msg.id));
    let _ = app.emit("subagent:created", json!({
        "parentId": parent_session_id,
        "subagent": sub,
    }));
    let _ = app.emit("subagents:changed", json!({
        "parentId": parent_session_id,
    }));
    Ok(sub)
}

#[tauri::command]
pub fn stop_subagent(app: AppHandle, subagent_id: String) -> Result<(), String> {
    let state = app.state::<crate::AppState>();
    let parent_id = {
        let db = state.db.lock().unwrap();
        store::get_session(&db, &subagent_id)?.and_then(|s| s.parent_session_id)
    };
    agent::stop_session(&app, &subagent_id);
    if let Some(pid) = parent_id {
        let _ = app.emit("subagent:update", json!({
            "parentId": pid,
            "subagentId": subagent_id,
            "status": "stopped",
        }));
        let _ = app.emit("subagents:changed", json!({"parentId": pid}));
    }
    Ok(())
}

#[tauri::command]
pub fn restart_subagent(app: AppHandle, subagent_id: String) -> Result<(), String> {
    agent::restart_subagent(&app, &subagent_id)
}

#[tauri::command]
pub fn restart_all_subagents(app: AppHandle, parent_session_id: String) -> Result<usize, String> {
    agent::restart_all_subagents(&app, &parent_session_id)
}

#[tauri::command]
pub fn delete_subagent(app: AppHandle, state: State<'_, crate::AppState>, subagent_id: String) -> Result<(), String> {
    agent::stop_session(&app, &subagent_id);
    let parent_id = {
        let db = state.db.lock().unwrap();
        let parent_id = store::get_session(&db, &subagent_id)?.and_then(|s| s.parent_session_id);
        store::delete_session(&db, &subagent_id)?;
        parent_id
    };
    if let Some(pid) = parent_id {
        let _ = app.emit("subagents:changed", json!({"parentId": pid}));
    }
    Ok(())
}

#[tauri::command]
pub fn report_subagent_to_parent(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    subagent_id: String,
) -> Result<String, String> {
    let (parent_id, report_prompt) = {
        let db = state.db.lock().unwrap();
        let sub = store::get_session(&db, &subagent_id)?.ok_or("子 Agent 不存在")?;
        let parent_id = sub.parent_session_id.ok_or("该会话不是子 Agent，没有父会话")?;
        let msgs = store::get_messages(&db, &subagent_id, None, 50)?;
        let last_reply = msgs
            .iter()
            .rev()
            .find(|m| m.role == "assistant" && !m.content.as_deref().unwrap_or("").is_empty())
            .and_then(|m| m.content.as_deref())
            .unwrap_or("(子 Agent 未产生文本产出)");

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
        let touched_str = if touched_files.is_empty() {
            String::new()
        } else {
            let list = touched_files.into_iter().map(|f| format!("`{f}`")).collect::<Vec<_>>().join(", ");
            format!("涉及改动文件：{}\n", list)
        };

        let role = sub.subagent_role.as_deref().unwrap_or("协作助手");
        let prompt = format!(
            "【子 Agent 成果汇报 - {} ({})】\n{}\n{}\n请主 Agent 审阅以上产出，进行集成检验并继续推进后续工作。",
            sub.title, role, touched_str, last_reply
        );
        (parent_id, prompt)
    };

    send_message(
        state,
        app,
        Some(parent_id),
        report_prompt,
        None,
        None,
        None,
        None,
        None,
    )?;

    Ok("已成功将成果汇报发送至主会话并唤醒主 Agent！".into())
}

/// 手动关闭正在执行的控制台命令进程
#[tauri::command]
pub fn kill_command(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    event_id: String,
) -> Result<(), String> {
    let rc_opt = {
        let mut cmds = state.running_commands.lock().unwrap();
        cmds.remove(&event_id)
    };
    if let Some(rc) = rc_opt {
        if let Some(pid) = rc.pid {
            crate::tools::kill_process_tree(pid);
        }
        let _ = rc.tx.send(());
    } else {
        // 未在活跃进程表（如历史崩溃/重启遗留、已退出的孤立事件）：
        // 兜底将数据库中的状态由 running 修正为 failed，并广播 tool:update 给前端移除卡片
        let db = state.db.lock().unwrap();
        if let Ok(Some((mut ev, session_id))) = store::get_tool_event_with_session(&db, &event_id) {
            if ev.status == "running" || ev.status == "pending_approval" {
                ev.status = "failed".to_string();
                let msg = "[控制台进程未在运行，已清理状态]".to_string();
                ev.result_text = Some(msg.clone());
                let _ = store::update_tool_event(&db, &ev.id, "failed", Some(&msg), None);
                drop(db);
                let _ = app.emit(
                    "tool:update",
                    json!({"sessionId": session_id, "event": ev}),
                );
            }
        }
    }
    Ok(())
}

/// 编辑并重新发送最后一条用户消息
#[tauri::command]
pub fn edit_and_resend(
    state: State<'_, crate::AppState>,
    app: AppHandle,
    session_id: String,
    message_id: String,
    new_text: String,
) -> Result<(), String> {
    if is_run_active(&state, &session_id) {
        return Err("Agent 正在运行，无法编辑重发".into());
    }
    let text = new_text.trim().to_string();
    if text.is_empty() {
        return Err("消息不能为空".into());
    }
    {
        let db = state.db.lock().unwrap();
        let m = store::get_message(&db, &message_id)?
            .filter(|m| m.session_id == session_id && m.role == "user" && !m.queued)
            .ok_or("仅支持编辑会话内的用户消息")?;
        // 临时空间对话：合并点（含）之前的消息永久不可编辑重发
        if let Some(s) = store::get_session(&db, &session_id)? {
            if let Some(boundary) = s.merged_seq {
                if m.seq <= boundary {
                    return Err("合并前的消息不可编辑".into());
                }
            }
        }
        store::update_message_content(&db, &message_id, &text, None)?;
        store::delete_messages_after(&db, &session_id, m.seq)?;
        store::touch_session(&db, &session_id)?;
    }
    let _ = app.emit("messages:changed", json!({"sessionId": session_id}));
    emit_session(&state, &app, &session_id);
    agent::spawn_session_task(app.clone(), session_id, Some(message_id));
    Ok(())
}

// ---------- 运行状态 ----------

/// 全局运行中的会话列表：界面刷新后前端据此恢复各会话的“运行中”状态显示
/// （以数据库 runs 表为准；任务中止/退出循环时均会兜底清理，不会残留）
#[tauri::command]
pub fn list_running_sessions(
    state: State<'_, crate::AppState>,
) -> Result<Vec<RunningSession>, String> {
    let db = state.db.lock().unwrap();
    let rows = store::all_running_runs(&db)?;
    Ok(rows
        .into_iter()
        .map(|(session_id, run_id)| RunningSession { session_id, run_id })
        .collect())
}


// ---------- 审批 ----------

#[tauri::command]
pub fn respond_approval(
    state: State<'_, crate::AppState>,
    event_id: String,
    decision: String,
    reason: Option<String>,
) -> Result<(), String> {
    let d = crate::approval::Decision::from_str(&decision, reason).ok_or("无效的审批决定")?;
    let tx = state.approvals.lock().unwrap().remove(&event_id).map(|p| p.tx);
    if let Some(tx) = tx {
        let _ = tx.send(d);
        Ok(())
    } else {
        Err("审批请求不存在或已处理".into())
    }
}

#[tauri::command]
pub fn respond_compaction(
    state: State<'_, crate::AppState>,
    app: AppHandle,
    event_id: String,
    approved: bool,
    final_summary: String,
) -> Result<(), String> {
    let tx = state.compactions.lock().unwrap().remove(&event_id).map(|p| p.tx);
    if let Some(tx) = tx {
        let _ = tx.send(crate::models::CompactionDecision {
            approved,
            final_summary,
        });
        let _ = app.emit("compaction:resolved", json!({ "eventId": event_id, "approved": approved }));
        Ok(())
    } else {
        Err("压缩请求不存在或已处理".into())
    }
}

#[tauri::command]
pub fn list_session_compactions(
    state: State<'_, crate::AppState>,
    session_id: String,
) -> Result<Vec<crate::models::SessionCompaction>, String> {
    let db = state.db.lock().unwrap();
    crate::store::list_session_compactions(&db, &session_id)
}

#[tauri::command]
pub fn get_session_todos(state: State<'_, crate::AppState>, session_id: String) -> Result<Value, String> {
    let running = is_run_active(&state, &session_id);
    let db = state.db.lock().unwrap();
    let raw: Option<String> = db
        .query_row(
            "SELECT value FROM session_kv WHERE session_id = ?1 AND key = 'todos'",
            rusqlite::params![session_id],
            |r| r.get(0),
        )
        .ok();
    let mut val: Value = raw.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);

    // 会话当前若未在执行，检查最近一次运行结果；若正常完成（或无记录），自愈修复遗留的 in_progress 任务为 done
    if !running {
        let last_run_status: Option<String> = db
            .query_row(
                "SELECT status FROM runs WHERE session_id = ?1 ORDER BY started_at DESC LIMIT 1",
                rusqlite::params![session_id],
                |r| r.get(0),
            )
            .ok();
        let should_mark_done = match last_run_status.as_deref() {
            Some("done") | None => true,
            _ => false,
        };
        if let Some(todos_arr) = val.get_mut("todos").and_then(|v| v.as_array_mut()) {
            let mut changed = false;
            for item in todos_arr.iter_mut() {
                if item.get("status").and_then(|s| s.as_str()) == Some("in_progress") {
                    item["status"] = if should_mark_done {
                        serde_json::json!("done")
                    } else {
                        serde_json::json!("pending")
                    };
                    changed = true;
                }
            }
            if changed {
                let _ = store::set_kv(&db, &session_id, "todos", &val.to_string());
            }
        }
    }

    Ok(val)
}

// ---------- 临时空间 ----------

/// 为项目生成临时空间计划（仅地址，不创建目录）。git 不可用时拒绝并给出安装提示。
#[tauri::command]
pub fn alloc_temp_code(state: State<'_, crate::AppState>, project_id: String) -> Result<TempAlloc, String> {
    let data_dir = state
        .data_dir
        .lock()
        .unwrap()
        .clone()
        .ok_or("数据目录不可用")?;
    let (project, links) = {
        let db = state.db.lock().unwrap();
        (
            store::get_project(&db, &project_id)?.ok_or("项目不存在")?,
            store::list_project_links(&db, &project_id)?,
        )
    };
    crate::temp::alloc(&data_dir, &project, &links)
}

/// 临时空间运行时状态：驱动临时目录 / 合并 / 清空按钮的禁用态
#[tauri::command]
pub fn get_temp_info(state: State<'_, crate::AppState>, session_id: String) -> Result<TempInfo, String> {
    let db = state.db.lock().unwrap();
    let s = store::get_session(&db, &session_id)?.ok_or("会话不存在")?;
    Ok(crate::temp::temp_info(&db, &s))
}

/// 变更列表（弹窗左侧）：按项目分组 + 增删行数统计
#[tauri::command]
pub fn list_temp_changes(state: State<'_, crate::AppState>, session_id: String) -> Result<TempChanges, String> {
    let manifest = {
        let db = state.db.lock().unwrap();
        let s = store::get_session(&db, &session_id)?.ok_or("会话不存在")?;
        if !s.is_temp {
            return Err("该对话不是临时空间对话".into());
        }
        crate::temp::load_manifest(&db, &session_id)?.ok_or("临时空间清单缺失")?
    };
    crate::temp::list_temp_changes(&manifest)
}

/// 单文件 diff（弹窗右侧；path 必须在变更清单内）
#[tauri::command]
pub fn get_temp_change_diff(
    state: State<'_, crate::AppState>,
    session_id: String,
    project_key: String,
    path: String,
) -> Result<TempFileDiff, String> {
    let manifest = {
        let db = state.db.lock().unwrap();
        let s = store::get_session(&db, &session_id)?.ok_or("会话不存在")?;
        if !s.is_temp {
            return Err("该对话不是临时空间对话".into());
        }
        crate::temp::load_manifest(&db, &session_id)?.ok_or("临时空间清单缺失")?
    };
    let entry = manifest
        .projects
        .iter()
        .find(|p| p.key == project_key)
        .ok_or("临时空间中不存在该项目")?;
    crate::temp::file_diff(entry, &path)
}

/// 合并：把临时空间的变更写回各项目原目录（冲突走 AI 智能合并）。
/// from_agent=true 时为 Agent 工具的透传调用（运行互斥检查跳过，审批已在工具层完成）
#[tauri::command]
pub async fn merge_temp_space(
    state: State<'_, crate::AppState>,
    app: AppHandle,
    session_id: String,
    from_agent: Option<bool>,
) -> Result<MergeSummary, String> {
    if from_agent.unwrap_or(false) {
        return crate::temp::merge_from_agent(&state, &app, &session_id).await;
    }
    crate::temp::merge_space(&state, &app, &session_id).await
}

/// 清空临时空间：删除整个随机码目录（含全部项目副本）。
/// 删除整棵目录（大量小文件）是长耗时纯阻塞 IO：同步命令会在主线程执行导致界面冻结闪现，
/// 直接在 async fn 里内联执行又会占死 tokio worker，故放入 spawn_blocking 的阻塞线程池执行
#[tauri::command]
pub async fn clear_temp_space(app: AppHandle, session_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<crate::AppState>();
        crate::temp::clear_space(&state, &app, &session_id)
    })
    .await
    .map_err(|e| format!("清空临时空间任务失败: {e}"))?
}

// ---------- 打开目录 ----------

/// 在系统文件管理器中打开目录或定位文件
#[tauri::command]
pub fn open_dir(state: State<'_, crate::AppState>, path: String) -> Result<(), String> {
    let raw_trimmed = path.trim();
    let raw_path = Path::new(raw_trimmed);
    let resolved = if raw_path.is_relative() {
        if let Some(d) = state.data_dir.lock().unwrap().as_ref() {
            d.join(raw_path)
        } else {
            PathBuf::from(raw_trimmed)
        }
    } else if !raw_path.exists() && crate::temp::rel_temp_path(raw_path).is_some() {
        if let Some(d) = state.data_dir.lock().unwrap().as_ref() {
            crate::temp::adapt_temp_path(raw_path, d)
        } else {
            PathBuf::from(raw_trimmed)
        }
    } else {
        PathBuf::from(raw_trimmed)
    };
    let mut p = resolved.clone();
    // 若目标路径不存在，自动向上寻找最近存在的父目录
    while !p.exists() {
        match p.parent() {
            Some(parent) if parent != p => p = parent.to_path_buf(),
            _ => return Err(format!("目录不存在: {}", resolved.display())),
        }
    }

    let target_dir = if p.is_file() {
        p.parent().unwrap_or(&p)
    } else {
        &p
    };

    #[cfg(windows)]
    {
        let win_path = target_dir.to_string_lossy().replace('/', "\\");
        let res = std::process::Command::new("explorer")
            .arg(&win_path)
            .spawn();
        if let Err(e) = res {
            std::process::Command::new("cmd")
                .args(["/c", "start", "", &win_path])
                .spawn()
                .map_err(|e2| format!("打开目录失败: {e}; 备用启动亦失败: {e2}"))?;
        }
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(target_dir)
            .spawn()
            .map_err(|e| format!("打开目录失败: {e}"))?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(target_dir)
            .spawn()
            .map_err(|e| format!("打开目录失败: {e}"))?;
    }
    Ok(())
}

// ---------- 数据目录 ----------

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataStatus {
    /// 数据目录不可写，等待用户在界面中选择
    pub pending: bool,
    /// 当前生效的数据目录（pending 时为空）
    pub data_dir: Option<String>,
    /// 默认存放位置（程序同级 data\；开发构建为项目根目录 .dev-data\）
    pub default_dir: String,
    pub default_writable: bool,
    /// 是否使用了自定义目录（指针文件生效）
    pub is_custom: bool,
    /// pending 时不可写目录的提示信息
    pub unwritable_path: Option<String>,
}

#[tauri::command]
pub fn get_data_status(state: State<'_, crate::AppState>) -> DataStatus {
    DataStatus {
        pending: state.data_pending.load(Ordering::Relaxed),
        data_dir: state
            .data_dir
            .lock()
            .unwrap()
            .as_ref()
            .map(|p| p.display().to_string()),
        default_dir: state.default_data_dir.display().to_string(),
        default_writable: crate::paths::ensure_writable(&state.default_data_dir),
        is_custom: state.is_custom_dir.load(Ordering::Relaxed),
        unwritable_path: state.unwritable_path.clone(),
    }
}

/// 切换数据目录核心逻辑：迁移数据 → 打开新库 → 原地替换连接与主密钥（热切换，进程不退出）。
/// 全程持有 db 锁（锁序统一 db → master / data_dir），切换对外原子生效；
/// 调用方负责引导指针文件与前端刷新。
fn switch_data_dir(
    state: &crate::AppState,
    target: &std::path::Path,
    custom: bool,
) -> Result<(), String> {
    if let Some(cur) = state.data_dir.lock().unwrap().clone() {
        if crate::paths::same_dir(&cur, target) {
            return Ok(()); // 与当前目录一致，无需迁移
        }
    }
    let mut db = state.db.lock().unwrap();
    let mut master = state.master_key.lock().unwrap();
    let target_db = target.join("harness_mini.db");
    let reuse_existing = target_db.exists();
    if !reuse_existing {
        // 把当前数据迁往目标：目标密钥必须与被迁移数据一致，
        // 清除目标残留密钥文件并随迁当前密钥（内存哨兵库无密钥文件，稍后生成新钥重加密）
        let _ = std::fs::remove_file(target.join("secret.key"));
        if let Some(cur) = state.data_dir.lock().unwrap().clone() {
            let _ = crate::secrets::copy_key_file(&cur, target);
            let _ = crate::temp::copy_temp_projects_dir(&cur, target);
        }
    }
    let new_master = crate::secrets::load_or_create_master_key(target)?;
    let new_conn = if reuse_existing {
        // 目标已有库：视为复用该目录既有数据（其密文以目标自身密钥解密）
        store::open_db(&target_db)?
    } else if state.data_dir.lock().unwrap().is_some() {
        // 真实库：VACUUM INTO 一致性快照迁往新目录后打开
        let sql = format!("VACUUM INTO '{}'", target_db.to_string_lossy().replace('\'', "''"));
        db.execute(&sql, []).map_err(|e| format!("迁移数据失败: {e}"))?;
        store::open_db(&target_db)?
    } else {
        // 内存哨兵库（启动时目录不可写）：新建目标库，搬运设置与 API Key（重加密）
        let new_db = store::open_db(&target_db)?;
        store::copy_settings_and_secrets(&db, &new_db, &master, &new_master)?;
        new_db
    };
    let _ = crate::temp::rebase_temp_storage(&new_conn, target);
    *db = new_conn; // 旧连接随之关闭，旧目录数据保留为备份
    *master = new_master;
    drop(db);
    drop(master);
    *state.data_dir.lock().unwrap() = Some(target.to_path_buf());
    state.data_pending.store(false, Ordering::Relaxed);
    state.is_custom_dir.store(custom, Ordering::Relaxed);
    Ok(())
}

/// 切换数据目录：校验可写性与运行状态 → 迁移并热切换 → 写入引导指针。
/// 前端收到成功返回后整页刷新即生效，无需重启进程。
#[tauri::command]
pub fn set_data_dir(state: State<'_, crate::AppState>, app: AppHandle, path: String) -> Result<(), String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("目录不能为空".into());
    }
    let new_dir = std::path::PathBuf::from(path);
    if !crate::paths::ensure_writable(&new_dir) {
        return Err(format!("所选目录不可写：{path}"));
    }
    if crate::agent::any_run_active(&state) {
        return Err("有会话任务正在运行，请先停止后再切换数据目录".into());
    }
    switch_data_dir(&state, &new_dir, true)?;
    crate::paths::write_pointer(&app, &new_dir)?;
    Ok(())
}

/// 恢复默认数据目录：校验默认位置可写 → 清除引导指针 → 热切换回默认目录
#[tauri::command]
pub fn reset_data_dir(state: State<'_, crate::AppState>, app: AppHandle) -> Result<(), String> {
    let default_dir = state.default_data_dir.clone();
    if !crate::paths::ensure_writable(&default_dir) {
        return Err(format!("默认位置不可写：{}", default_dir.display()));
    }
    if crate::agent::any_run_active(&state) {
        return Err("有会话任务正在运行，请先停止后再切换数据目录".into());
    }
    crate::paths::clear_pointer(&app)?;
    switch_data_dir(&state, &default_dir, false)
}

#[tauri::command]
pub fn exit_app(app: AppHandle) {
    app.exit(0);
}

// ---------- 成长演进 (agent_growths) ----------

#[tauri::command]
pub fn list_growths(
    state: State<'_, crate::AppState>,
    project_id: Option<String>,
    status: Option<String>,
) -> Result<Vec<GrowthItem>, String> {
    let db = state.db.lock().unwrap();
    store::list_growths(&db, project_id.as_deref(), status.as_deref())
}

#[tauri::command]
pub fn update_growth_status(
    state: State<'_, crate::AppState>,
    app: AppHandle,
    id: String,
    status: String,
) -> Result<(), String> {
    let item = {
        let db = state.db.lock().unwrap();
        store::update_growth_status(&db, &id, &status)?;
        store::get_growth(&db, &id)?
    };
    if let Some(item) = item {
        let _ = app.emit("growth:updated", &item);
    }
    Ok(())
}

#[tauri::command]
pub fn update_growth_rule(
    state: State<'_, crate::AppState>,
    app: AppHandle,
    id: String,
    title: String,
    rule_content: String,
    category: String,
) -> Result<(), String> {
    let item = {
        let db = state.db.lock().unwrap();
        store::update_growth_rule(&db, &id, &title, &rule_content, &category)?;
        store::get_growth(&db, &id)?
    };
    if let Some(item) = item {
        let _ = app.emit("growth:updated", &item);
    }
    Ok(())
}

#[tauri::command]
pub fn delete_growth(
    state: State<'_, crate::AppState>,
    app: AppHandle,
    id: String,
) -> Result<(), String> {
    {
        let db = state.db.lock().unwrap();
        store::delete_growth(&db, &id)?;
    }
    let _ = app.emit("growth:deleted", json!({ "id": id }));
    Ok(())
}

#[tauri::command]
pub fn trigger_growth_reflection(
    app: AppHandle,
    session_id: String,
    user_instruction: Option<String>,
) -> Result<(), String> {
    crate::growth::trigger_manual_reflection(app, session_id, user_instruction);
    Ok(())
}

// ==================== 技能工具 (Skills) ====================

#[tauri::command]
pub async fn list_project_skills(workspace_path: String) -> Result<Vec<SkillItem>, String> {
    if workspace_path.is_empty() {
        return Ok(Vec::new());
    }
    crate::skills::list_skills(Path::new(&workspace_path)).await
}

#[tauri::command]
pub async fn save_project_skill(
    workspace_path: String,
    name: String,
    description: String,
    script_type: String,
    content: String,
) -> Result<SkillItem, String> {
    if workspace_path.is_empty() {
        return Err("缺少工作区路径".into());
    }
    crate::skills::save_skill(
        Path::new(&workspace_path),
        &name,
        &description,
        &script_type,
        &content,
    )
    .await
}

#[tauri::command]
pub async fn delete_project_skill(
    workspace_path: String,
    skill_name: String,
) -> Result<(), String> {
    if workspace_path.is_empty() {
        return Err("缺少工作区路径".into());
    }
    crate::skills::delete_skill(Path::new(&workspace_path), &skill_name).await
}

// ==================== 交付自检 SOP ====================

#[tauri::command]
pub fn get_project_sop(
    state: State<'_, crate::AppState>,
    workspace_path: String,
    project_id: Option<String>,
) -> Result<ProjectSopInfo, String> {
    let ws = Path::new(&workspace_path);
    let (detected_stack, detected_default_cmd) = crate::sop::detect_project_stack(ws);

    let db = state.db.lock().unwrap();
    let project = if let Some(pid) = project_id.as_deref().filter(|s| !s.is_empty()) {
        store::get_project(&db, pid)?
    } else if !workspace_path.is_empty() {
        store::find_project_by_path(&db, &workspace_path)?
    } else {
        None
    };

    if let Some(p) = project {
        let verify_cmd = p.sop_verify_cmd.unwrap_or_default();
        Ok(ProjectSopInfo {
            project_id: p.id,
            project_name: p.name,
            sop_verify_cmd: verify_cmd,
            sop_enabled: p.sop_enabled,
            detected_stack,
            detected_default_cmd,
        })
    } else {
        Ok(ProjectSopInfo {
            project_id: String::new(),
            project_name: if workspace_path.is_empty() {
                "未关联项目".into()
            } else {
                store::dir_name_of(&workspace_path)
            },
            sop_verify_cmd: String::new(),
            sop_enabled: true,
            detected_stack,
            detected_default_cmd,
        })
    }
}

#[tauri::command]
pub fn set_project_sop(
    state: State<'_, crate::AppState>,
    project_id: String,
    verify_cmd: Option<String>,
    enabled: bool,
) -> Result<(), String> {
    let db = state.db.lock().unwrap();
    store::set_project_sop(&db, &project_id, verify_cmd.as_deref(), enabled)
}

#[tauri::command]
pub async fn run_workspace_sop(
    workspace_path: String,
    cmd: String,
) -> Result<String, String> {
    let ws = Path::new(&workspace_path);
    if !ws.exists() {
        return Err("工作区目录不存在".into());
    }
    let (ok, out) = crate::sop::run_verify_cmd(ws, &cmd, std::time::Duration::from_secs(60)).await?;
    if ok {
        Ok(format!("✅ 自检通过：\n{}", out))
    } else {
        Err(format!("❌ 自检失败：\n{}", out))
    }
}

#[tauri::command]
pub fn get_token_stats(
    state: State<'_, crate::AppState>,
    project_id: Option<String>,
    days: Option<u32>,
) -> Result<TokenStatsReport, String> {
    let db = state.db.lock().unwrap();
    store::get_token_stats(&db, project_id.as_deref(), days)
}

#[tauri::command]
pub fn get_session_active_state(
    state: State<'_, crate::AppState>,
    session_id: String,
) -> Result<crate::snapshot::SessionActiveState, String> {
    Ok(state.snapshot.get(&session_id))
}



