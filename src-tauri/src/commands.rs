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
        reasoning_effort: None,
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
pub fn set_project_plan_mode(
    state: State<'_, crate::AppState>,
    app: AppHandle,
    id: String,
    mode: String,
) -> Result<(), String> {
    let project = {
        let db = state.db.lock().unwrap();
        store::set_project_plan_mode(&db, &id, &mode)?;
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
    let parent_id = {
        let db = state.db.lock().unwrap();
        let parent_id = store::get_session(&db, &id)?.and_then(|s| s.parent_session_id);
        store::delete_session(&db, &id)?;
        parent_id
    };
    let _ = app.emit("sessions:changed", json!({"deleted": id}));
    if let Some(pid) = parent_id {
        let _ = app.emit("subagents:changed", json!({"parentId": pid}));
        let _ = app.emit("collaborators:changed", json!({"parentId": pid}));
        let _ = app.emit("subprocesses:changed", json!({"parentId": pid}));
    }
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

#[tauri::command]
pub fn fork_session_at_message(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    session_id: String,
    message_id: String,
    new_title: Option<String>,
    include_target: Option<bool>,
) -> Result<Session, String> {
    let db = state.db.lock().unwrap();
    let s = store::fork_session_at_message(
        &db,
        &session_id,
        &message_id,
        new_title.as_deref(),
        include_target.unwrap_or(true),
    )?;
    let _ = app.emit("sessions:changed", json!({"created": s.id}));
    let _ = app.emit("session:update", &s);
    Ok(s)
}

#[tauri::command]
pub fn create_session(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    workspace_path: Option<String>,
    project_id: Option<String>,
    title: Option<String>,
    access_mode: Option<String>,
    context_token_limit: Option<usize>,
    temp: Option<TempAlloc>,
    image_provider_id: Option<String>,
    image_model_id: Option<String>,
    vision_provider_id: Option<String>,
    vision_model_id: Option<String>,
    reasoning_effort: Option<String>,
) -> Result<Session, String> {
    let access_mode = crate::models::normalize_access_mode(access_mode.as_deref().unwrap_or("confirm"));
    let title = title
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "主进程与统筹协调者".to_string());

    let db = state.db.lock().unwrap();
    if let Some(t) = temp {
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
        let s = store::create_session_with_models(
            &db,
            &t.main_temp,
            Some(&proj_id),
            &title,
            &access_mode,
            image_provider_id.as_deref(),
            image_model_id.as_deref(),
            vision_provider_id.as_deref(),
            vision_model_id.as_deref(),
            reasoning_effort.as_deref(),
        )?;
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
        let _ = app.emit("session:update", &s);
        return Ok(s);
    }

    let ws = workspace_path.clone().unwrap_or_default();
    let project_id = if ws.is_empty() {
        project_id
    } else {
        let p = store::find_or_create_project_by_path(&db, &ws)?;
        let pid = p.id.clone();
        let _ = app.emit("projects:changed", &p);
        Some(pid)
    };
    let s = store::create_session_with_models(
        &db,
        &ws,
        project_id.as_deref(),
        &title,
        &access_mode,
        image_provider_id.as_deref(),
        image_model_id.as_deref(),
        vision_provider_id.as_deref(),
        vision_model_id.as_deref(),
        reasoning_effort.as_deref(),
    )?;
    if let Some(lim) = context_token_limit {
        let _ = store::set_session_context_limit(&db, &s.id, Some(lim));
    }
    if !ws.is_empty() {
        let mut settings = store::get_settings(&db).unwrap_or_default();
        settings.last_workspace_path = Some(ws);
        if let Ok(json) = serde_json::to_string(&settings) {
            let _ = db.execute(
                "INSERT INTO settings(key, value) VALUES('app_settings', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                rusqlite::params![json],
            );
        }
    }
    let _ = app.emit("sessions:changed", json!({"created": s.id}));
    let _ = app.emit("session:update", &s);
    Ok(s)
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
    attachments: Option<Vec<crate::models::Attachment>>,
    image_provider_id: Option<String>,
    image_model_id: Option<String>,
    vision_provider_id: Option<String>,
    vision_model_id: Option<String>,
    reasoning_effort: Option<String>,
) -> Result<SendResult, String> {
    let text = text.trim().to_string();
    let has_attachments = attachments.as_ref().map(|a| !a.is_empty()).unwrap_or(false);
    if text.is_empty() && !has_attachments {
        return Err("消息内容与附件不能同时为空".into());
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
                let title: String = if !text.is_empty() {
                    text.chars().take(24).collect()
                } else if let Some(att) = attachments.as_ref().and_then(|a| a.first()) {
                    format!("[{}] {}", if att.is_image { "图片" } else { "文件" }, att.name)
                } else {
                    "新对话".into()
                };
                let s = store::create_session_with_models(
                    &db,
                    &t.main_temp,
                    Some(&proj_id),
                    &title,
                    &access_mode,
                    image_provider_id.as_deref(),
                    image_model_id.as_deref(),
                    vision_provider_id.as_deref(),
                    vision_model_id.as_deref(),
                    reasoning_effort.as_deref(),
                )?;
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
                let title: String = if !text.is_empty() {
                    text.chars().take(24).collect()
                } else if let Some(att) = attachments.as_ref().and_then(|a| a.first()) {
                    format!("[{}] {}", if att.is_image { "图片" } else { "文件" }, att.name)
                } else {
                    "新对话".into()
                };
                // 工作区即项目：落库时按工作区路径关联项目；尚无该项目则自动创建一条项目数据
                let project_id = if ws.is_empty() {
                    project_id
                } else {
                    let p = store::find_or_create_project_by_path(&db, &ws)?;
                    let pid = p.id.clone();
                    let _ = app.emit("projects:changed", &p);
                    Some(pid)
                };
                let s = store::create_session_with_models(
                    &db,
                    &ws,
                    project_id.as_deref(),
                    &title,
                    &access_mode,
                    image_provider_id.as_deref(),
                    image_model_id.as_deref(),
                    vision_provider_id.as_deref(),
                    vision_model_id.as_deref(),
                    reasoning_effort.as_deref(),
                )?;
                if let Some(lim) = context_token_limit {
                    let _ = store::set_session_context_limit(&db, &s.id, Some(lim));
                }
                let _ = app.emit("sessions:changed", json!({"created": s.id}));
                s.id
            }
        };
        let active = is_run_active(&state, &sid);
        let content_opt = if text.is_empty() { None } else { Some(text) };
        let msg = store::new_message_with_attachments(&db, &sid, "user", content_opt, attachments, active, None)?;
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
pub async fn save_attachment(
    state: State<'_, crate::AppState>,
    session_id: Option<String>,
    name: String,
    mime_type: String,
    base64_data: Option<String>,
    source_path: Option<String>,
) -> Result<crate::models::Attachment, String> {
    use base64::Engine;

    let data_dir = state
        .data_dir
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_else(|| state.default_data_dir.clone());
    let sid = session_id.unwrap_or_else(|| "temp".to_string());
    let attach_dir = data_dir.join("attachments").join(&sid);
    tokio::fs::create_dir_all(&attach_dir)
        .await
        .map_err(|e| format!("创建附件目录失败: {e}"))?;

    let id = uuid::Uuid::new_v4().to_string();
    let safe_base_name = name.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
    let target_filename = format!("{}_{}", &id[..8], safe_base_name);
    let target_path = attach_dir.join(&target_filename);

    let size = if let Some(b64) = base64_data {
        let clean_b64 = if let Some(idx) = b64.find(',') {
            &b64[idx + 1..]
        } else {
            &b64
        };
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(clean_b64.trim())
            .map_err(|e| format!("附件 Base64 解码失败: {e}"))?;
        let len = bytes.len() as u64;
        tokio::fs::write(&target_path, &bytes)
            .await
            .map_err(|e| format!("保存附件失败: {e}"))?;
        len
    } else if let Some(src) = source_path {
        let src_p = PathBuf::from(&src);
        if !src_p.exists() {
            return Err(format!("源文件不存在: {src}"));
        }
        let meta = tokio::fs::metadata(&src_p)
            .await
            .map_err(|e| format!("读取源文件信息失败: {e}"))?;
        tokio::fs::copy(&src_p, &target_path)
            .await
            .map_err(|e| format!("拷贝附件文件失败: {e}"))?;
        meta.len()
    } else {
        return Err("缺少 base64_data 或 source_path".into());
    };

    let is_img = mime_type.starts_with("image/")
        || ["png", "jpg", "jpeg", "webp", "gif", "bmp", "svg"]
            .iter()
            .any(|ext| name.to_lowercase().ends_with(ext));

    Ok(crate::models::Attachment {
        id,
        name,
        mime_type,
        size,
        path: target_path.to_string_lossy().to_string(),
        is_image: is_img,
    })
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
pub fn retry_turn(app: AppHandle, session_id: String) -> Result<(), String> {
    agent::retry_turn(&app, &session_id)
}

#[tauri::command]
pub fn continue_turn(app: AppHandle, session_id: String) -> Result<(), String> {
    agent::continue_turn(&app, &session_id)
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
    _subpath: Option<String>,
    _workspace_path: Option<String>,
) -> Result<Session, String> {
    let parent = {
        let db = state.db.lock().unwrap();
        store::get_session(&db, &parent_session_id)?.ok_or("父会话不存在")?
    };
    if parent.session_type == "subagent" || parent.parent_session_id.is_some() {
        return Err("子 Agent 不允许再创建子 Agent".into());
    }

    // 子 Agent 工作区严格继承父会话工作区根目录
    let sub_workspace = parent.workspace_path.clone();

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
            None,
        )?;
        let initial_prompt = format!(
            "【子 Agent 协作任务】\n角色定位：{role}\n任务标题：{title}\n工作区根目录：{sub_workspace}\n\n详细需求描述：\n{task}"
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
    let _ = app.emit("subprocess:created", json!({
        "parentId": parent_session_id,
        "subprocess": sub,
    }));
    let _ = app.emit("subprocesses:changed", json!({
        "parentId": parent_session_id,
    }));
    Ok(sub)
}

#[tauri::command]
pub fn stop_subagent(app: AppHandle, subagent_id: String) -> Result<(), String> {
    let state = app.state::<crate::AppState>();
    let parent_id = {
        let db = state.db.lock().unwrap();
        let pid = store::get_session(&db, &subagent_id)?.and_then(|s| s.parent_session_id);
        let _ = store::set_session_status(&db, &subagent_id, "cancelled");
        pid
    };
    agent::stop_session(&app, &subagent_id);
    if let Some(pid) = parent_id {
        let _ = app.emit("subagent:update", json!({
            "parentId": pid,
            "subagentId": subagent_id,
            "status": "cancelled",
        }));
        let _ = app.emit("subprocess:update", json!({
            "parentId": pid,
            "parentSessionId": pid,
            "subprocessId": subagent_id,
            "status": "cancelled",
        }));
        let _ = app.emit("subagents:changed", json!({"parentId": pid}));
        let _ = app.emit("subprocesses:changed", json!({"parentId": pid}));
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
        let _ = app.emit("collaborators:changed", json!({"parentId": pid}));
        let _ = app.emit("subprocesses:changed", json!({"parentId": pid}));
    }
    let _ = app.emit("sessions:changed", json!({"deleted": subagent_id}));
    Ok(())
}

#[tauri::command]
pub fn list_collaborators(state: State<'_, crate::AppState>, parent_session_id: String) -> Result<Vec<Session>, String> {
    let db = state.db.lock().unwrap();
    store::list_collaborators(&db, &parent_session_id)
}

#[tauri::command]
pub fn list_subprocesses(state: State<'_, crate::AppState>, parent_session_id: String) -> Result<Vec<Session>, String> {
    let db = state.db.lock().unwrap();
    store::list_subprocesses(&db, &parent_session_id)
}

#[tauri::command]
pub fn create_collaborator(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    parent_session_id: String,
    role: String,
    title: String,
    task_prompt: String,
    subpath: Option<String>,
    workspace_path: Option<String>,
    auto_report: Option<bool>,
    provider_id: Option<String>,
    model_id: Option<String>,
    dispatch_rule: Option<String>,
    image_provider_id: Option<String>,
    image_model_id: Option<String>,
    vision_provider_id: Option<String>,
    vision_model_id: Option<String>,
) -> Result<Session, String> {
    let parent = {
        let db = state.db.lock().unwrap();
        if parent_session_id.is_empty() || parent_session_id == "draft" {
            let ws = workspace_path.clone().unwrap_or_default();
            let project_id = if ws.is_empty() {
                None
            } else {
                let p = store::find_or_create_project_by_path(&db, &ws)?;
                let pid = p.id.clone();
                let _ = app.emit("projects:changed", &p);
                Some(pid)
            };
            let s = store::create_session(&db, &ws, project_id.as_deref(), "主进程与统筹协调者", "confirm")?;
            let _ = app.emit("sessions:changed", json!({"created": s.id}));
            let _ = app.emit("session:update", &s);
            s
        } else {
            store::get_session(&db, &parent_session_id)?.ok_or("父会话不存在")?
        }
    };
    if parent.session_type == "collaborator" || parent.session_type == "subprocess" || parent.parent_session_id.is_some() {
        return Err("当前会话不可再创建协作者".into());
    }
    let real_parent_id = parent.id.clone();

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

    let auto_report_val = auto_report.unwrap_or(true);
    let task_desc = if let Some(ref p) = subpath.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        format!("{task_prompt}\n（重点关注子目录：`{p}`）")
    } else {
        task_prompt
    };
    let collab = {
        let db = state.db.lock().unwrap();
        store::create_collaborator_session(
            &db,
            &real_parent_id,
            &role,
            &title,
            &task_desc,
            dispatch_rule.as_deref(),
            &sub_workspace,
            parent.access_mode.as_deref(),
            parent.project_id.as_deref(),
            auto_report_val,
            provider_id.as_deref(),
            model_id.as_deref(),
            image_provider_id.as_deref(),
            image_model_id.as_deref(),
            vision_provider_id.as_deref(),
            vision_model_id.as_deref(),
        )?
    };

    let _ = app.emit("collaborator:created", json!({
        "parentId": real_parent_id,
        "collaborator": collab,
    }));
    let _ = app.emit("collaborators:changed", json!({
        "parentId": real_parent_id,
    }));
    let _ = app.emit("subagent:created", json!({
        "parentId": real_parent_id,
        "subagent": collab,
    }));
    let _ = app.emit("subagents:changed", json!({
        "parentId": real_parent_id,
    }));

    Ok(collab)
}

#[tauri::command]
pub fn update_collaborator(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    collaborator_id: String,
    title: String,
    role: String,
    task_prompt: String,
    dispatch_rule: Option<String>,
    subpath: Option<String>,
    workspace_path: Option<String>,
    auto_report: Option<bool>,
    provider_id: Option<String>,
    model_id: Option<String>,
    image_provider_id: Option<String>,
    image_model_id: Option<String>,
    vision_provider_id: Option<String>,
    vision_model_id: Option<String>,
) -> Result<Session, String> {
    let parent_id = {
        let db = state.db.lock().unwrap();
        let s = store::get_session(&db, &collaborator_id)?.ok_or("协作者不存在")?;
        if s.session_type != "collaborator" {
            return Err("仅支持编辑协作者".into());
        }
        s.parent_session_id.ok_or("协作者未关联父会话")?
    };

    let parent_ws = {
        let db = state.db.lock().unwrap();
        store::get_session(&db, &parent_id)?
            .map(|p| p.workspace_path)
            .unwrap_or_default()
    };

    let sub_workspace = if let Some(ref ws) = workspace_path.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        let p = std::path::Path::new(ws);
        if p.is_absolute() {
            ws.to_string()
        } else {
            std::path::Path::new(&parent_ws).join(p).to_string_lossy().to_string()
        }
    } else {
        parent_ws
    };

    let auto_report_val = auto_report.unwrap_or(true);
    let task_desc = if let Some(ref p) = subpath.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        if !task_prompt.contains("（重点关注子目录：") {
            format!("{task_prompt}\n（重点关注子目录：`{p}`）")
        } else {
            task_prompt
        }
    } else {
        task_prompt
    };

    let collab = {
        let db = state.db.lock().unwrap();
        store::update_collaborator_session(
            &db,
            &collaborator_id,
            &title,
            &role,
            &task_desc,
            dispatch_rule.as_deref(),
            &sub_workspace,
            auto_report_val,
            provider_id.as_deref(),
            model_id.as_deref(),
            image_provider_id.as_deref(),
            image_model_id.as_deref(),
            vision_provider_id.as_deref(),
            vision_model_id.as_deref(),
        )?
    };

    let _ = app.emit("collaborator:updated", json!({
        "parentId": parent_id,
        "collaborator": collab,
    }));
    let _ = app.emit("collaborators:changed", json!({
        "parentId": parent_id,
    }));
    let _ = app.emit("subagents:changed", json!({
        "parentId": parent_id,
    }));
    let _ = app.emit("session:update", &collab);

    Ok(collab)
}

#[tauri::command]
pub fn set_session_models(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    session_id: String,
    provider_id: Option<String>,
    model_id: Option<String>,
    image_provider_id: Option<String>,
    image_model_id: Option<String>,
    vision_provider_id: Option<String>,
    vision_model_id: Option<String>,
) -> Result<Session, String> {
    let db = state.db.lock().unwrap();
    let updated = store::update_session_models(
        &db,
        &session_id,
        provider_id.as_deref(),
        model_id.as_deref(),
        image_provider_id.as_deref(),
        image_model_id.as_deref(),
        vision_provider_id.as_deref(),
        vision_model_id.as_deref(),
    )?;
    drop(db);

    let _ = app.emit("session:update", &updated);
    let _ = app.emit("sessions:changed", json!({ "updated": session_id }));
    if updated.session_type == "collaborator" {
        let _ = app.emit("collaborator:updated", &updated);
        if let Some(ref pid) = updated.parent_session_id {
            let _ = app.emit("collaborators:changed", json!({ "parentId": pid }));
        }
    }
    Ok(updated)
}

#[tauri::command]
pub fn set_session_reasoning_effort(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    session_id: String,
    reasoning_effort: Option<String>,
) -> Result<Session, String> {
    let db = state.db.lock().unwrap();
    let updated = store::set_session_reasoning_effort(
        &db,
        &session_id,
        reasoning_effort.as_deref(),
    )?;
    drop(db);

    let _ = app.emit("session:update", &updated);
    let _ = app.emit("sessions:changed", json!({ "updated": session_id }));
    if updated.session_type == "collaborator" {
        let _ = app.emit("collaborator:updated", &updated);
        if let Some(ref pid) = updated.parent_session_id {
            let _ = app.emit("collaborators:changed", json!({ "parentId": pid }));
        }
    }
    Ok(updated)
}

#[tauri::command]
pub fn set_collaborator_auto_report(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    collaborator_id: String,
    auto_report: bool,
) -> Result<(), String> {
    let parent_id = {
        let db = state.db.lock().unwrap();
        store::update_collaborator_auto_report(&db, &collaborator_id, auto_report)?;
        store::get_session(&db, &collaborator_id)?.and_then(|s| s.parent_session_id)
    };
    if let Some(pid) = parent_id {
        let _ = app.emit("collaborators:changed", json!({"parentId": pid}));
    }
    Ok(())
}

pub fn do_report_collaborator_increment(
    app: &AppHandle,
    collaborator_id: &str,
) -> Result<String, String> {
    let state = app.state::<crate::AppState>();
    let (parent_id, report_prompt, new_watermark_id) = {
        let db = state.db.lock().unwrap();
        let collab = store::get_session(&db, collaborator_id)?.ok_or("协作者不存在")?;
        let parent_id = collab.parent_session_id.ok_or("该会话没有关联的父会话")?;
        let msgs = store::get_messages(&db, collaborator_id, None, 100)?;

        let slice: Vec<&crate::models::Message> = if let Some(ref w_id) = collab.last_reported_msg_id {
            if let Some(pos) = msgs.iter().position(|m| &m.id == w_id) {
                msgs[pos + 1..].iter().collect()
            } else {
                msgs.iter().collect()
            }
        } else {
            msgs.iter().collect()
        };

        if slice.is_empty() {
            return Ok("自上次汇报以来暂无新增对话。".into());
        }

        let last_assistant = slice
            .iter()
            .rev()
            .find(|m| m.role == "assistant" && !m.content.as_deref().unwrap_or("").is_empty());

        let (last_reply, new_watermark_id) = match last_assistant {
            Some(m) => (m.content.as_deref().unwrap_or(""), m.id.clone()),
            None => return Ok("协作者尚未生成有效产出，暂无需汇报。".into()),
        };

        let mut touched_files = std::collections::BTreeSet::new();
        for m in &slice {
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

        let role = collab.subagent_role.as_deref().unwrap_or("协作者");
        let prompt = format!(
            "【协作者成果汇报 - {} ({})】\n{}\n{}\n请主 Agent 审阅以上增量产出，进行集成检验并继续推进后续工作。",
            collab.title, role, touched_str, last_reply
        );
        (parent_id, prompt, new_watermark_id)
    };

    send_message(
        state,
        app.clone(),
        Some(parent_id.clone()),
        report_prompt,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )?;

    {
        let state = app.state::<crate::AppState>();
        let db = state.db.lock().unwrap();
        let _ = store::update_collaborator_watermark(&db, collaborator_id, &new_watermark_id);
    }

    let _ = app.emit("collaborator:reported", json!({
        "collaboratorId": collaborator_id,
        "parentId": parent_id,
        "lastReportedMsgId": new_watermark_id,
    }));
    let _ = app.emit("collaborators:changed", json!({
        "parentId": parent_id,
    }));

    Ok("已成功将增量成果汇报发送至主会话！".into())
}

#[tauri::command]
pub fn report_collaborator_increment(
    app: AppHandle,
    collaborator_id: String,
) -> Result<String, String> {
    do_report_collaborator_increment(&app, &collaborator_id)
}

#[tauri::command]
pub fn report_subagent_to_parent(
    app: AppHandle,
    subagent_id: String,
) -> Result<String, String> {
    do_report_collaborator_increment(&app, &subagent_id)
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

/// 编辑并重新发送最后一条用户消息（自动回退本次提问产生的所有代码修改）
#[tauri::command]
pub async fn edit_and_resend(
    state: State<'_, crate::AppState>,
    app: AppHandle,
    session_id: String,
    message_id: String,
    new_text: String,
    attachments: Option<Vec<crate::models::Attachment>>,
) -> Result<(), String> {
    if is_run_active(&state, &session_id) {
        return Err("Agent 正在运行，无法编辑重发".into());
    }
    let text = new_text.trim().to_string();
    let has_attachments = attachments.as_ref().map(|a| !a.is_empty()).unwrap_or(false);
    if text.is_empty() && !has_attachments {
        return Err("消息内容与附件不能同时为空".into());
    }

    // 1. 检查权限并收集待回退的代码快照（在删除消息与快照元数据前执行）
    let (ws, data_dir, snaps_to_revert) = {
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
        let session = store::get_session(&db, &session_id)?
            .ok_or_else(|| format!("会话未找到: {session_id}"))?;
        let dir = {
            let lock = state.data_dir.lock().unwrap();
            lock.clone().unwrap_or_else(|| state.default_data_dir.clone())
        };
        let snaps = store::list_snapshots_after_seq(&db, &session_id, m.seq)?;
        (PathBuf::from(session.workspace_path), dir, snaps)
    };

    // 2. 自动无损还原该提问及后续所产生的所有代码修改（确定性回退到提问前状态）
    if !snaps_to_revert.is_empty() {
        let _ = crate::snapshot_revert::revert_snapshots(&ws, &data_dir, &snaps_to_revert, true).await;
    }

    // 3. 更新用户消息正文并安全作废后续消息与历史记录
    {
        let db = state.db.lock().unwrap();
        let m = store::get_message(&db, &message_id)?
            .filter(|m| m.session_id == session_id && m.role == "user" && !m.queued)
            .ok_or("仅支持编辑会话内的用户消息")?;
        store::update_message_content_and_attachments(&db, &message_id, &text, attachments.as_deref())?;
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
    let pending = state.compactions.lock().unwrap().remove(&event_id);
    if let Some(p) = pending {
        state.snapshot.set_pending_compaction(&p.session_id, None);
        let _ = p.tx.send(crate::models::CompactionDecision {
            approved,
            final_summary,
        });
        let _ = app.emit("compaction:resolved", json!({ "eventId": event_id, "approved": approved, "sessionId": p.session_id }));
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

    // 若当前会话关联了工作区中的活动计划，以计划文件中的步骤作为权威真理源优先同步（仅限进行中计划）
    if let Ok(Some(sess)) = crate::store::get_session(&db, &session_id) {
        if !sess.workspace_path.is_empty() {
            let ws = std::path::Path::new(&sess.workspace_path);
            if let Some((_, meta, body)) = crate::plan::find_plan_file(ws, &session_id, None, Some(&db)) {
                if meta.status == "in_progress" {
                    let steps = crate::plan::parse_steps_from_markdown(&body);
                    if !steps.is_empty() {
                        let todos_val: Vec<Value> = steps.iter().map(|s| {
                            serde_json::json!({
                                "content": s.content,
                                "status": s.status
                            })
                        }).collect();
                        val = serde_json::json!({ "todos": todos_val });
                        let _ = store::set_kv(&db, &session_id, "todos", &val.to_string());
                    }
                }
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

// ---------- 打开目录与路径探测 ----------

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PathInspectResult {
    pub exists: bool,
    pub is_dir: bool,
    pub is_file: bool,
    pub abs_path: String,
    pub file_name: String,
}

fn find_file_in_workspace(ws: &Path, file_name: &str) -> Option<PathBuf> {
    let mut dirs = vec![ws.to_path_buf()];
    let mut depth = 0;
    while !dirs.is_empty() && depth < 5 {
        let mut next_dirs = Vec::new();
        for d in dirs {
            let Ok(entries) = std::fs::read_dir(&d) else { continue };
            for entry in entries.flatten() {
                let Ok(ft) = entry.file_type() else { continue };
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if ft.is_dir() {
                    if !name_str.starts_with('.') && name_str != "node_modules" && name_str != "target" && name_str != "dist" {
                        next_dirs.push(entry.path());
                    }
                } else if ft.is_file() && name_str.eq_ignore_ascii_case(file_name) {
                    return Some(entry.path());
                }
            }
        }
        dirs = next_dirs;
        depth += 1;
    }
    None
}

/// 检查路径属性与存在性（支持工作区上下文与快速模糊检索）
#[tauri::command]
pub fn inspect_path(
    state: State<'_, crate::AppState>,
    path: String,
    workspace_path: Option<String>,
) -> PathInspectResult {
    let raw_trimmed = path.trim().trim_start_matches("file:///");
    let raw_path = Path::new(raw_trimmed);

    let mut resolved = if raw_path.is_absolute() {
        raw_path.to_path_buf()
    } else if let Some(ref ws) = workspace_path {
        if !ws.trim().is_empty() {
            Path::new(ws.trim()).join(raw_path)
        } else if let Some(d) = state.data_dir.lock().unwrap().as_ref() {
            d.join(raw_path)
        } else {
            PathBuf::from(raw_trimmed)
        }
    } else if let Some(d) = state.data_dir.lock().unwrap().as_ref() {
        d.join(raw_path)
    } else {
        PathBuf::from(raw_trimmed)
    };

    let mut exists = resolved.exists();

    // 如果未直接存在，且在工作区中，并且是一个纯文件名（如 App.tsx），尝试浅层模糊检索
    if !exists && !raw_trimmed.contains('/') && !raw_trimmed.contains('\\') {
        if let Some(ref ws) = workspace_path {
            let ws_p = Path::new(ws.trim());
            if ws_p.exists() {
                if let Some(found) = find_file_in_workspace(ws_p, raw_trimmed) {
                    resolved = found;
                    exists = true;
                }
            }
        }
    }

    let is_dir = resolved.is_dir();
    let is_file = resolved.is_file();
    let file_name = resolved
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| raw_trimmed.to_string());
    let abs_path = resolved.to_string_lossy().replace('\\', "/");

    PathInspectResult {
        exists,
        is_dir,
        is_file,
        abs_path,
        file_name,
    }
}

/// 在系统文件管理器中打开目录或定位文件
#[tauri::command]
pub fn open_dir(
    state: State<'_, crate::AppState>,
    path: String,
    workspace_path: Option<String>,
) -> Result<(), String> {
    let raw_trimmed = path.trim().trim_start_matches("file:///");
    let raw_path = Path::new(raw_trimmed);
    let resolved = if raw_path.is_absolute() {
        PathBuf::from(raw_trimmed)
    } else if let Some(ref ws) = workspace_path {
        if !ws.trim().is_empty() {
            Path::new(ws.trim()).join(raw_path)
        } else if let Some(d) = state.data_dir.lock().unwrap().as_ref() {
            d.join(raw_path)
        } else {
            PathBuf::from(raw_trimmed)
        }
    } else if let Some(d) = state.data_dir.lock().unwrap().as_ref() {
        d.join(raw_path)
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

    #[cfg(windows)]
    {
        if p.is_file() {
            let win_path = p.to_string_lossy().replace('/', "\\");
            let arg = format!("/select,{}", win_path);
            let res = std::process::Command::new("explorer")
                .arg(arg)
                .spawn();
            if res.is_ok() {
                return Ok(());
            }
        }
        let target_dir = if p.is_file() {
            p.parent().unwrap_or(&p)
        } else {
            &p
        };
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
        if p.is_file() {
            let res = std::process::Command::new("open")
                .args(["-R", &p.to_string_lossy()])
                .spawn();
            if res.is_ok() {
                return Ok(());
            }
        }
        let target_dir = if p.is_file() {
            p.parent().unwrap_or(&p)
        } else {
            &p
        };
        std::process::Command::new("open")
            .arg(target_dir)
            .spawn()
            .map_err(|e| format!("打开目录失败: {e}"))?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let target_dir = if p.is_file() {
            p.parent().unwrap_or(&p)
        } else {
            &p
        };
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

#[tauri::command]
pub async fn read_file_base64(state: State<'_, crate::AppState>, path: String) -> Result<String, String> {
    read_file_base64_inner(Some(&state), &path).await
}

pub async fn read_file_base64_inner(state: Option<&crate::AppState>, path: &str) -> Result<String, String> {
    let mut clean = path.trim().to_string();

    // 1. 剔除常见的网络/协议前缀
    if let Some(stripped) = clean.strip_prefix("asset://localhost/") {
        clean = stripped.to_string();
    } else if let Some(stripped) = clean.strip_prefix("asset://") {
        clean = stripped.to_string();
    } else if let Some(stripped) = clean.strip_prefix("http://asset.localhost/") {
        clean = stripped.to_string();
    } else if let Some(stripped) = clean.strip_prefix("https://asset.localhost/") {
        clean = stripped.to_string();
    } else if let Some(stripped) = clean.strip_prefix("file:///") {
        clean = stripped.to_string();
    } else if let Some(stripped) = clean.strip_prefix("file://") {
        clean = stripped.to_string();
    }

    // 2. URL 百分号解码（如 %20, %3A, %5C 等）
    if clean.contains('%') {
        let mut out = Vec::with_capacity(clean.len());
        let bytes = clean.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' && i + 2 < bytes.len() {
                if let Ok(val) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..=i + 2]).unwrap_or(""), 16) {
                    out.push(val);
                    i += 3;
                    continue;
                }
            }
            out.push(bytes[i]);
            i += 1;
        }
        if let Ok(decoded) = String::from_utf8(out) {
            clean = decoded;
        }
    }

    // 3. 针对 Windows 驱动器路径前有多余斜杠的处理，如 /D:/xxx -> D:/xxx
    if clean.starts_with('/') && clean.len() >= 3 && clean.chars().nth(2) == Some(':') {
        clean = clean[1..].to_string();
    }

    let file_path = PathBuf::from(&clean);
    let resolved_path = if file_path.exists() {
        Some(file_path)
    } else if let Some(st) = state {
        let db = st.db.lock().unwrap();
        let mut candidate = None;
        // 1. 尝试使用 settings.last_workspace_path 拼接相对路径
        if let Ok(settings) = store::get_settings(&db) {
            if let Some(ref ws) = settings.last_workspace_path {
                let p = Path::new(ws).join(&clean);
                if p.exists() {
                    candidate = Some(p);
                }
            }
        }
        // 2. 尝试从所有已保存项目中匹配工作区路径拼接
        if candidate.is_none() {
            if let Ok(projects) = store::list_projects(&db) {
                for p in projects {
                    if let Some(ref ws) = p.path {
                        let cand = Path::new(ws).join(&clean);
                        if cand.exists() {
                            candidate = Some(cand);
                            break;
                        }
                    }
                }
            }
        }
        // 3. 尝试从数据目录或 generated_images 目录查找
        if candidate.is_none() {
            let data_dir = st.data_dir.lock().unwrap().clone().unwrap_or_else(|| st.default_data_dir.clone());
            let cand1 = data_dir.join(&clean);
            if cand1.exists() {
                candidate = Some(cand1);
            } else {
                let cand2 = data_dir.join("generated_images").join(&clean);
                if cand2.exists() {
                    candidate = Some(cand2);
                }
            }
        }
        candidate
    } else {
        None
    };

    let target_file = resolved_path.ok_or_else(|| format!("文件不存在: {}", clean))?;

    let bytes = tokio::fs::read(&target_file)
        .await
        .map_err(|e| format!("读取图片文件失败: {e}"))?;

    let ext = target_file
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        _ => "image/png",
    };

    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:{mime};base64,{b64}"))
}

// ---------- 任务方案计划中枢 commands ----------

#[tauri::command]
pub fn get_active_plan(
    state: State<'_, crate::AppState>,
    session_id: String,
) -> Result<Option<Value>, String> {
    let db = state.db.lock().unwrap();
    let session = crate::store::get_session(&db, &session_id)?.ok_or("会话不存在")?;
    if session.workspace_path.is_empty() {
        return Ok(None);
    }
    let ws = std::path::Path::new(&session.workspace_path);
    if let Some((path, meta, body)) = crate::plan::find_plan_file(ws, &session_id, None, Some(&db)) {
        let filename = path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
        let steps = crate::plan::parse_steps_from_markdown(&body);
        Ok(Some(serde_json::json!({
            "meta": meta,
            "filename": filename,
            "body": body,
            "steps": steps,
        })))
    } else {
        // 容错兜底：若当前会话无进行中活动计划，回退查找该会话名下最近更新过的计划（如 completed 或 suspended），防止浮窗断崖式清空
        let pdir = crate::plan::plans_dir(ws);
        if pdir.exists() {
            if let Ok(entries) = std::fs::read_dir(&pdir) {
                let mut session_plans = Vec::new();
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("md") {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            let (meta, body) = crate::plan::parse_frontmatter(&content);
                            if meta.session_id == session_id {
                                session_plans.push((path, meta, body));
                            }
                        }
                    }
                }
                session_plans.sort_by(|a, b| b.1.updated_at.cmp(&a.1.updated_at));
                if let Some((path, meta, body)) = session_plans.into_iter().next() {
                    let filename = path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
                    let steps = crate::plan::parse_steps_from_markdown(&body);
                    return Ok(Some(serde_json::json!({
                        "meta": meta,
                        "filename": filename,
                        "body": body,
                        "steps": steps,
                    })));
                }
            }
        }
        Ok(None)
    }
}

#[tauri::command]
pub fn list_workspace_plans(
    state: State<'_, crate::AppState>,
    workspace_path: String,
    session_id: Option<String>,
    include_archived: Option<bool>,
) -> Result<Vec<crate::plan::PlanSummary>, String> {
    if workspace_path.is_empty() {
        return Ok(Vec::new());
    }
    let db = state.db.lock().unwrap();
    let ws = std::path::Path::new(&workspace_path);
    crate::plan::list_plans(ws, session_id.as_deref(), include_archived.unwrap_or(false), Some(&db))
}

#[tauri::command]
pub fn switch_plan(
    state: State<'_, crate::AppState>,
    workspace_path: String,
    session_id: String,
    plan_id: String,
) -> Result<String, String> {
    if workspace_path.is_empty() {
        return Err("工作区路径不能为空".into());
    }
    let db = state.db.lock().unwrap();
    let ws = std::path::Path::new(&workspace_path);
    crate::plan::switch_plan(ws, &session_id, &plan_id, Some(&db))
}

// ---------- 文件与变更查看器 (File Viewer) commands ----------

#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FileTextContent {
    pub path: String,
    pub name: String,
    pub content: String,
    pub size_bytes: u64,
    pub lines_count: usize,
    pub is_binary: bool,
    pub is_truncated: bool,
    pub language: String,
}

fn detect_language_by_path(path: &str) -> String {
    let p = Path::new(path);
    match p.extension().and_then(|s| s.to_str()).map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("rs") => "rust",
        Some("ts") => "typescript",
        Some("tsx") => "typescript",
        Some("js") | Some("mjs") | Some("cjs") => "javascript",
        Some("jsx") => "javascript",
        Some("json") => "json",
        Some("md") | Some("markdown") => "markdown",
        Some("py") => "python",
        Some("html") | Some("htm") => "html",
        Some("css") | Some("scss") | Some("less") => "css",
        Some("toml") => "toml",
        Some("yaml") | Some("yml") => "yaml",
        Some("sh") | Some("bash") | Some("zsh") => "bash",
        Some("cmd") | Some("bat") => "bat",
        Some("sql") => "sql",
        Some("xml") => "xml",
        Some("c") | Some("h") | Some("cpp") | Some("hpp") | Some("cc") => "cpp",
        Some("go") => "go",
        Some("java") => "java",
        Some("kt") => "kotlin",
        Some("php") => "php",
        Some("rb") => "ruby",
        Some("swift") => "swift",
        Some("vue") => "vue",
        Some("svelte") => "svelte",
        _ => "plaintext",
    }.to_string()
}

#[tauri::command]
pub async fn open_file_viewer(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    payload: Option<serde_json::Value>,
) -> Result<(), String> {
    if let Some(ref p) = payload {
        *state.file_viewer_init_tab.lock().unwrap() = Some(p.clone());
    }

    if let Some(win) = app.get_webview_window("file_viewer") {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
        if let Some(ref p) = payload {
            let _ = win.emit("file_viewer:open_tab", p);
        }
        return Ok(());
    }

    let win = tauri::WebviewWindowBuilder::new(
        &app,
        "file_viewer",
        tauri::WebviewUrl::App("index.html?window=file_viewer".into()),
    )
    .title("文件与变更查看器")
    .inner_size(1220.0, 800.0)
    .min_inner_size(800.0, 500.0)
    .center()
    .decorations(false)
    .transparent(true)
    .shadow(true)
    .build()
    .map_err(|e| format!("创建文件查看器窗口失败: {e}"))?;

    let _ = win.show();
    let _ = win.set_focus();
    Ok(())
}

#[tauri::command]
pub fn get_file_viewer_init_tab(
    state: State<'_, crate::AppState>,
) -> Result<Option<serde_json::Value>, String> {
    let val = state.file_viewer_init_tab.lock().unwrap().take();
    Ok(val)
}

#[tauri::command]
pub fn read_text_file(
    path: String,
    max_bytes: Option<usize>,
) -> Result<FileTextContent, String> {
    let raw_trimmed = path.trim().trim_start_matches("file:///");
    let p = Path::new(raw_trimmed);
    if !p.exists() {
        return Err(format!("文件不存在: {}", p.display()));
    }
    if !p.is_file() {
        return Err(format!("指定路径不是常规文件: {}", p.display()));
    }

    let meta = std::fs::metadata(p).map_err(|e| format!("获取文件信息失败: {e}"))?;
    let size_bytes = meta.len();
    let limit = max_bytes.unwrap_or(2 * 1024 * 1024);

    let mut file = std::fs::File::open(p).map_err(|e| format!("打开文件失败: {e}"))?;
    use std::io::Read;

    let mut check_buf = [0u8; 8192];
    let n = file.read(&mut check_buf).unwrap_or(0);
    let is_binary = check_buf[..n].contains(&0);

    if is_binary {
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
        let language = detect_language_by_path(raw_trimmed);
        return Ok(FileTextContent {
            path: p.to_string_lossy().to_string(),
            name,
            content: String::new(),
            size_bytes,
            lines_count: 0,
            is_binary: true,
            is_truncated: false,
            language,
        });
    }

    use std::io::Seek;
    let _ = file.seek(std::io::SeekFrom::Start(0));

    let to_read = std::cmp::min(size_bytes, limit as u64) as usize;
    let mut buf = vec![0u8; to_read];
    let actual_read = file.read(&mut buf).map_err(|e| format!("读取内容失败: {e}"))?;
    buf.truncate(actual_read);

    let is_truncated = size_bytes > (actual_read as u64);
    let content = String::from_utf8_lossy(&buf).to_string();
    let lines_count = content.lines().count();
    let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
    let language = detect_language_by_path(raw_trimmed);

    Ok(FileTextContent {
        path: p.to_string_lossy().to_string(),
        name,
        content,
        size_bytes,
        lines_count,
        is_binary: false,
        is_truncated,
        language,
    })
}

#[tauri::command]
pub fn save_text_file(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    path: String,
    content: String,
    session_id: Option<String>,
) -> Result<(), String> {
    let raw_trimmed = path.trim().trim_start_matches("file:///");
    let p = Path::new(raw_trimmed);
    if let Some(parent) = p.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建父目录失败: {e}"))?;
        }
    }
    std::fs::write(p, &content).map_err(|e| format!("保存文件失败: {e}"))?;

    let _ = app.emit("file_viewer:file_changed", serde_json::json!({
        "path": p.to_string_lossy().to_string()
    }));

    // 如果保存的是计划 Markdown 文件，重新解析步骤并同步 todos
    let norm_path = raw_trimmed.replace('\\', "/");
    if norm_path.contains("/.harness/plans/") || norm_path.starts_with(".harness/plans/") {
        let steps = crate::plan::parse_steps_from_markdown(&content);
        let db = state.db.lock().unwrap();
        if let Some(ref sid) = session_id {
            if !sid.is_empty() {
                crate::plan::sync_steps_to_todos(Some(&db), sid, &steps);
                let payload: Vec<serde_json::Value> = steps
                    .iter()
                    .map(|s| {
                        serde_json::json!({
                            "content": s.content,
                            "status": s.status
                        })
                    })
                    .collect();
                let _ = app.emit("session:todos", serde_json::json!({
                    "sessionId": sid,
                    "todos": payload,
                }));
            }
        }
    }

    Ok(())
}

pub fn compute_unified_split_diff(
    old_text: &str,
    new_text: &str,
    project_key: &str,
    project_name: &str,
    path: &str,
    name: &str,
) -> TempFileDiff {
    let diff = similar::TextDiff::from_lines(old_text, new_text);
    let mut added = 0usize;
    let mut removed = 0usize;
    let mut total_lines = 0usize;
    let mut truncated = false;
    let mut hunks = Vec::new();

    const DIFF_MAX_LINES: usize = 4000;

    'outer: for ops in diff.grouped_ops(3) {
        let mut lines: Vec<DiffLine> = Vec::new();
        for op in &ops {
            for ch in diff.iter_changes(op) {
                let tag = match ch.tag() {
                    similar::ChangeTag::Delete => "del",
                    similar::ChangeTag::Insert => "add",
                    similar::ChangeTag::Equal => "same",
                };
                match ch.tag() {
                    similar::ChangeTag::Insert => added += 1,
                    similar::ChangeTag::Delete => removed += 1,
                    similar::ChangeTag::Equal => {}
                }
                lines.push(DiffLine {
                    tag: tag.to_string(),
                    old_no: ch.old_index().map(|i| i + 1),
                    new_no: ch.new_index().map(|i| i + 1),
                    text: ch.value().trim_end_matches('\n').trim_end_matches('\r').to_string(),
                });
            }
        }
        if lines.is_empty() {
            continue;
        }
        if total_lines + lines.len() > DIFF_MAX_LINES {
            truncated = true;
            break 'outer;
        }
        let old_start = lines.iter().find_map(|l| l.old_no).unwrap_or(0);
        let new_start = lines.iter().find_map(|l| l.new_no).unwrap_or(0);
        let old_lines = lines.iter().filter(|l| l.old_no.is_some()).count();
        let new_lines = lines.iter().filter(|l| l.new_no.is_some()).count();
        total_lines += lines.len();

        hunks.push(DiffHunk {
            old_start,
            old_lines,
            new_start,
            new_lines,
            lines,
        });
    }

    let change = if added > 0 && removed == 0 {
        "added"
    } else if removed > 0 && added == 0 {
        "deleted"
    } else {
        "modified"
    };

    TempFileDiff {
        project_key: project_key.to_string(),
        project_name: project_name.to_string(),
        path: path.to_string(),
        name: name.to_string(),
        change: change.to_string(),
        binary: false,
        too_large: false,
        added,
        removed,
        truncated,
        hunks,
    }
}

#[tauri::command]
pub fn get_file_diff(
    path: String,
    old_content: Option<String>,
    new_content: Option<String>,
    workspace_path: Option<String>,
) -> Result<TempFileDiff, String> {
    let raw_trimmed = path.trim().trim_start_matches("file:///");
    let p = Path::new(raw_trimmed);
    let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();

    if let (Some(ref old_str), Some(ref new_str)) = (&old_content, &new_content) {
        return Ok(compute_unified_split_diff(
            old_str,
            new_str,
            "custom",
            "变更比对",
            raw_trimmed,
            &name,
        ));
    }

    let disk_content = if p.is_file() {
        std::fs::read_to_string(p).unwrap_or_default()
    } else if let Some(ref ws) = workspace_path {
        let abs_p = Path::new(ws).join(raw_trimmed);
        if abs_p.is_file() {
            std::fs::read_to_string(&abs_p).unwrap_or_default()
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    let actual_new = new_content.unwrap_or(disk_content);

    let actual_old = if let Some(o) = old_content {
        o
    } else {
        let ws_dir = workspace_path
            .as_deref()
            .map(Path::new)
            .or_else(|| p.parent())
            .unwrap_or_else(|| Path::new("."));

        let rel_path = if let Ok(rel) = p.strip_prefix(ws_dir) {
            rel.to_string_lossy().replace('\\', "/")
        } else {
            raw_trimmed.replace('\\', "/")
        };

        let output = std::process::Command::new("git")
            .args(["show", &format!("HEAD:{}", rel_path)])
            .current_dir(ws_dir)
            .output();

        match output {
            Ok(out) if out.status.success() => {
                String::from_utf8_lossy(&out.stdout).to_string()
            }
            _ => String::new(),
        }
    };

    Ok(compute_unified_split_diff(
        &actual_old,
        &actual_new,
        "workspace",
        "工作区",
        raw_trimmed,
        &name,
    ))
}

#[tauri::command]
pub fn get_plan_detail(
    state: State<'_, crate::AppState>,
    workspace_path: String,
    session_id: Option<String>,
    plan_id: Option<String>,
) -> Result<Option<Value>, String> {
    if workspace_path.is_empty() {
        return Ok(None);
    }
    let db = state.db.lock().unwrap();
    let ws = Path::new(&workspace_path);
    let sid = session_id.unwrap_or_default();
    if let Some((path, meta, body)) = crate::plan::find_plan_file(ws, &sid, plan_id.as_deref(), Some(&db)) {
        let filename = path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
        let steps = crate::plan::parse_steps_from_markdown(&body);
        Ok(Some(serde_json::json!({
            "meta": meta,
            "filename": filename,
            "body": body,
            "steps": steps,
        })))
    } else {
        Ok(None)
    }
}

#[tauri::command]
pub fn open_in_external_editor(path: String, line: Option<usize>) -> Result<(), String> {
    let raw_trimmed = path.trim().trim_start_matches("file:///");
    let p = Path::new(raw_trimmed);
    if !p.exists() {
        return Err(format!("文件不存在: {}", p.display()));
    }

    let line_num = line.unwrap_or(1);
    let target_arg = format!("{}:{}", raw_trimmed, line_num);

    // 1. 尝试使用 VS Code 打开并定位行
    if std::process::Command::new("code")
        .args(["--goto", &target_arg])
        .spawn()
        .is_ok()
    {
        return Ok(());
    }

    // 2. 尝试使用 Cursor 打开并定位行
    if std::process::Command::new("cursor")
        .args(["--goto", &target_arg])
        .spawn()
        .is_ok()
    {
        return Ok(());
    }

    // 3. 回退为操作系统默认程序打开
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer")
            .arg(raw_trimmed)
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg(raw_trimmed)
            .spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open")
            .arg(raw_trimmed)
            .spawn();
    }

    Ok(())
}

#[tauri::command]
pub fn update_plan_step_status(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    workspace_path: String,
    session_id: Option<String>,
    plan_id: Option<String>,
    step_index: usize,
    status: String,
) -> Result<(), String> {
    if workspace_path.is_empty() {
        return Err("工作区路径不能为空".into());
    }
    let db = state.db.lock().unwrap();
    let ws = Path::new(&workspace_path);
    crate::plan::update_plan_step_status(
        ws,
        session_id.as_deref(),
        plan_id.as_deref(),
        step_index,
        &status,
        Some(&db),
    )?;

    if let Some(ref sid) = session_id {
        if let Ok(Some(raw)) = crate::store::get_kv(&db, sid, "todos") {
            if let Ok(val) = serde_json::from_str::<Value>(&raw) {
                if let Some(todos) = val.get("todos") {
                    let _ = app.emit(
                        "session:todos",
                        serde_json::json!({
                            "sessionId": sid,
                            "todos": todos
                        }),
                    );
                }
            }
        }
    }

    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileOutlineItem {
    pub line: usize,
    pub symbol: String,
    pub kind: String,
}

#[tauri::command]
pub fn get_file_outline(path: String) -> Result<Vec<FileOutlineItem>, String> {
    let raw_trimmed = path.trim().trim_start_matches("file:///");
    let p = Path::new(raw_trimmed);
    if !p.is_file() {
        return Err(format!("文件不存在: {}", p.display()));
    }

    let text = std::fs::read_to_string(p).map_err(|e| format!("读取文件失败: {e}"))?;
    let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();

    let mut items = Vec::new();

    for (i, raw_line) in text.lines().enumerate() {
        let lineno = i + 1;
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let (is_match, kind) = match ext.as_str() {
            "rs" => {
                if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
                    (false, "")
                } else {
                    let words: Vec<&str> = trimmed.split_whitespace().collect();
                    if words.iter().any(|&w| w == "fn") {
                        (true, "fn")
                    } else if words.iter().any(|&w| w == "struct") {
                        (true, "struct")
                    } else if words.iter().any(|&w| w == "enum") {
                        (true, "enum")
                    } else if words.iter().any(|&w| w == "trait") {
                        (true, "interface")
                    } else if words.iter().any(|&w| w == "impl") {
                        (true, "impl")
                    } else if words.iter().any(|&w| w == "type") {
                        (true, "type")
                    } else {
                        (false, "")
                    }
                }
            }
            "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => {
                if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
                    (false, "")
                } else {
                    let t = if trimmed.starts_with("export default ") {
                        &trimmed["export default ".len()..]
                    } else if trimmed.starts_with("export ") {
                        &trimmed["export ".len()..]
                    } else {
                        trimmed
                    };

                    if t.starts_with("class ") {
                        (true, "class")
                    } else if t.starts_with("interface ") {
                        (true, "interface")
                    } else if t.starts_with("type ") {
                        (true, "type")
                    } else if t.starts_with("enum ") {
                        (true, "enum")
                    } else if t.starts_with("function ") || t.starts_with("async function ") {
                        (true, "fn")
                    } else if (t.starts_with("const ") || t.starts_with("let "))
                        && (t.contains("=>") || t.contains("function(") || t.contains("function ("))
                    {
                        (true, "fn")
                    } else {
                        (false, "")
                    }
                }
            }
            "py" => {
                if trimmed.starts_with('#') {
                    (false, "")
                } else if trimmed.starts_with("class ") {
                    (true, "class")
                } else if trimmed.starts_with("def ") || trimmed.starts_with("async def ") {
                    (true, "fn")
                } else {
                    (false, "")
                }
            }
            "go" => {
                if trimmed.starts_with("//") || trimmed.starts_with("/*") {
                    (false, "")
                } else if trimmed.starts_with("func ") {
                    (true, "fn")
                } else if trimmed.starts_with("type ") && trimmed.contains("struct") {
                    (true, "struct")
                } else if trimmed.starts_with("type ") && trimmed.contains("interface") {
                    (true, "interface")
                } else if trimmed.starts_with("type ") {
                    (true, "type")
                } else {
                    (false, "")
                }
            }
            "java" | "cs" | "cpp" | "c" | "h" | "hpp" => {
                if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
                    (false, "")
                } else if trimmed.starts_with("class ") || trimmed.contains(" class ") {
                    (true, "class")
                } else if trimmed.starts_with("interface ") || trimmed.contains(" interface ") {
                    (true, "interface")
                } else if trimmed.starts_with("enum ") || trimmed.contains(" enum ") {
                    (true, "enum")
                } else if trimmed.starts_with("struct ") || trimmed.contains(" struct ") {
                    (true, "struct")
                } else if trimmed.contains('(') && trimmed.contains(')') && !trimmed.ends_with(';') {
                    (true, "fn")
                } else {
                    (false, "")
                }
            }
            "md" | "markdown" => {
                if trimmed.starts_with('#') && trimmed.chars().take_while(|&c| c == '#').count() <= 6 {
                    (true, "heading")
                } else {
                    (false, "")
                }
            }
            _ => {
                if !raw_line.starts_with(' ')
                    && !raw_line.starts_with('\t')
                    && !trimmed.starts_with("//")
                    && !trimmed.starts_with('#')
                {
                    if trimmed.contains("fn ") || trimmed.contains("func ") || trimmed.contains("def ") {
                        (true, "fn")
                    } else if trimmed.contains("class ") {
                        (true, "class")
                    } else if trimmed.contains("interface ") {
                        (true, "interface")
                    } else if trimmed.contains("struct ") {
                        (true, "struct")
                    } else {
                        (false, "")
                    }
                } else {
                    (false, "")
                }
            }
        };

        if is_match {
            let clean_symbol = trimmed
                .trim_end_matches('{')
                .trim_end_matches(';')
                .trim()
                .chars()
                .take(120)
                .collect::<String>();

            items.push(FileOutlineItem {
                line: lineno,
                symbol: clean_symbol,
                kind: kind.to_string(),
            });

            if items.len() >= 300 {
                break;
            }
        }
    }

    Ok(items)
}

#[tauri::command]
pub fn revert_file_hunk(path: String, hunk: DiffHunk) -> Result<(), String> {
    let raw_trimmed = path.trim().trim_start_matches("file:///");
    let p = Path::new(raw_trimmed);
    if !p.is_file() {
        return Err(format!("目标文件不存在或无法写入: {}", p.display()));
    }

    let content = std::fs::read_to_string(p).map_err(|e| format!("读取文件失败: {e}"))?;
    let is_crlf = content.contains("\r\n");

    let expected_lines: Vec<String> = hunk
        .lines
        .iter()
        .filter(|l| l.tag != "del")
        .map(|l| l.text.clone())
        .collect();

    let replacement_lines: Vec<String> = hunk
        .lines
        .iter()
        .filter(|l| l.tag != "add")
        .map(|l| l.text.clone())
        .collect();

    let file_lines: Vec<String> = content
        .lines()
        .map(|s| s.to_string())
        .collect();

    if expected_lines.is_empty() {
        return Err("差异块内容为空，无法撤销".into());
    }

    let exp_len = expected_lines.len();
    let mut match_idx: Option<usize> = None;

    let target_hint = hunk.new_start.saturating_sub(1);
    if target_hint + exp_len <= file_lines.len()
        && file_lines[target_hint..target_hint + exp_len] == expected_lines[..]
    {
        match_idx = Some(target_hint);
    } else {
        let search_start = target_hint.saturating_sub(150);
        let search_end = (target_hint + 150).min(file_lines.len().saturating_sub(exp_len));
        for i in search_start..=search_end {
            if i + exp_len <= file_lines.len() && file_lines[i..i + exp_len] == expected_lines[..] {
                match_idx = Some(i);
                break;
            }
        }
    }

    let Some(start) = match_idx else {
        return Err("在当前文件中未匹配到该差异块内容，可能文件已被改动，请刷新后再试".into());
    };

    let mut new_file_lines = Vec::with_capacity(file_lines.len() - exp_len + replacement_lines.len());
    new_file_lines.extend_from_slice(&file_lines[..start]);
    new_file_lines.extend(replacement_lines);
    new_file_lines.extend_from_slice(&file_lines[start + exp_len..]);

    let sep = if is_crlf { "\r\n" } else { "\n" };
    let mut result_text = new_file_lines.join(sep);
    if content.ends_with('\n') && !result_text.ends_with('\n') {
        result_text.push_str(sep);
    }

    std::fs::write(p, result_text).map_err(|e| format!("写入撤销修改失败: {e}"))?;
    Ok(())
}

// ==================== 长任务（Long-Running Task）Commands ====================

#[tauri::command]
pub fn start_long_task(
    app: AppHandle,
    session_id: String,
    goal: String,
    max_budget_tokens: Option<u64>,
) -> Result<LongTask, String> {
    crate::task::start_long_task(app, session_id, goal, max_budget_tokens)
}

#[tauri::command]
pub fn pause_long_task(app: AppHandle, task_id: String) -> Result<(), String> {
    crate::task::pause_long_task(&app, &task_id)
}

#[tauri::command]
pub fn resume_long_task(app: AppHandle, task_id: String) -> Result<LongTask, String> {
    crate::task::resume_long_task(&app, &task_id)
}

#[tauri::command]
pub fn cancel_long_task(app: AppHandle, task_id: String) -> Result<(), String> {
    crate::task::cancel_long_task(&app, &task_id)
}

#[tauri::command]
pub fn get_active_task(app: AppHandle, session_id: String) -> Result<Option<LongTask>, String> {
    crate::task::get_active_task(&app, &session_id)
}

#[tauri::command]
pub fn list_task_checkpoints(app: AppHandle, task_id: String) -> Result<Vec<TaskCheckpoint>, String> {
    crate::task::list_task_checkpoints(&app, &task_id)
}

#[tauri::command]
pub fn rollback_to_checkpoint(app: AppHandle, checkpoint_id: String) -> Result<LongTask, String> {
    crate::task::rollback_to_checkpoint(&app, &checkpoint_id)
}

#[tauri::command]
pub fn update_task_subtasks(
    app: AppHandle,
    task_id: String,
    subtasks: Vec<TaskSubItem>,
) -> Result<LongTask, String> {
    crate::task::update_task_subtasks(&app, &task_id, subtasks)
}

// ---------- 影子快照与时光机（多轮撤回/重做/Diff审查） ----------

#[tauri::command]
pub async fn revert_message_turn(
    state: State<'_, crate::AppState>,
    message_id: String,
    force: Option<bool>,
) -> Result<RevertResult, String> {
    let (ws, data_dir, snapshots, session_id) = {
        let db = state.db.lock().unwrap();
        let msg = store::get_message(&db, &message_id)?
            .ok_or_else(|| format!("消息未找到: {message_id}"))?;
        let session = store::get_session(&db, &msg.session_id)?
            .ok_or_else(|| format!("会话未找到: {}", msg.session_id))?;
        let dir = {
            let lock = state.data_dir.lock().unwrap();
            lock.clone().unwrap_or_else(|| state.default_data_dir.clone())
        };
        let snaps = store::list_snapshots_for_turn(&db, &message_id)?;
        (PathBuf::from(session.workspace_path), dir, snaps, msg.session_id)
    };

    let force = force.unwrap_or(false);
    let res = crate::snapshot_revert::revert_snapshots(&ws, &data_dir, &snapshots, force).await?;

    if res.success {
        let db = state.db.lock().unwrap();
        let now = store::now();
        let affected_ids = store::mark_snapshots_reverted_for_turn(&db, &message_id, Some(&now))?;
        for mid in affected_ids {
            if let Ok(Some(msg)) = store::get_message(&db, &mid) {
                state.emit("message:update", &msg);
            }
        }
        state.emit("messages:changed", &serde_json::json!({ "sessionId": session_id }));
    }

    Ok(res)
}

#[tauri::command]
pub async fn reapply_message_turn(
    state: State<'_, crate::AppState>,
    message_id: String,
    force: Option<bool>,
) -> Result<ReapplyResult, String> {
    let (ws, data_dir, snapshots, session_id) = {
        let db = state.db.lock().unwrap();
        let msg = store::get_message(&db, &message_id)?
            .ok_or_else(|| format!("消息未找到: {message_id}"))?;
        let session = store::get_session(&db, &msg.session_id)?
            .ok_or_else(|| format!("会话未找到: {}", msg.session_id))?;
        let dir = {
            let lock = state.data_dir.lock().unwrap();
            lock.clone().unwrap_or_else(|| state.default_data_dir.clone())
        };
        let snaps = store::list_snapshots_for_turn(&db, &message_id)?;
        (PathBuf::from(session.workspace_path), dir, snaps, msg.session_id)
    };

    let force = force.unwrap_or(false);
    let res = crate::snapshot_revert::reapply_snapshots(&ws, &data_dir, &snapshots, force).await?;

    if res.success {
        let db = state.db.lock().unwrap();
        let affected_ids = store::mark_snapshots_reverted_for_turn(&db, &message_id, None)?;
        for mid in affected_ids {
            if let Ok(Some(msg)) = store::get_message(&db, &mid) {
                state.emit("message:update", &msg);
            }
        }
        state.emit("messages:changed", &serde_json::json!({ "sessionId": session_id }));
    }

    Ok(res)
}

#[tauri::command]
pub async fn revert_tool_event(
    state: State<'_, crate::AppState>,
    tool_event_id: String,
    force: Option<bool>,
) -> Result<RevertResult, String> {
    let (ws, data_dir, snapshots, message_id, session_id) = {
        let db = state.db.lock().unwrap();
        let snaps = store::list_snapshots_for_event(&db, &tool_event_id)?;
        if snaps.is_empty() {
            return Ok(RevertResult {
                success: true,
                reverted_files: vec![],
                has_conflict: false,
                conflicted_files: vec![],
                message: "该工具调用无文件修改快照".into(),
            });
        }
        let session_id = snaps[0].session_id.clone();
        let message_id = snaps[0].message_id.clone();
        let session = store::get_session(&db, &session_id)?
            .ok_or_else(|| format!("会话未找到: {session_id}"))?;
        let dir = {
            let lock = state.data_dir.lock().unwrap();
            lock.clone().unwrap_or_else(|| state.default_data_dir.clone())
        };
        (PathBuf::from(session.workspace_path), dir, snaps, message_id, session_id)
    };

    let force = force.unwrap_or(false);
    let res = crate::snapshot_revert::revert_snapshots(&ws, &data_dir, &snapshots, force).await?;

    if res.success && !snapshots.is_empty() {
        let db = state.db.lock().unwrap();
        let now = store::now();
        store::mark_snapshot_reverted_for_event(&db, &tool_event_id, Some(&now))?;
        if let Ok(Some(msg)) = store::get_message(&db, &message_id) {
            state.emit("message:update", &msg);
        }
        state.emit("messages:changed", &serde_json::json!({ "sessionId": session_id }));
    }

    Ok(res)
}

#[tauri::command]
pub async fn reapply_tool_event(
    state: State<'_, crate::AppState>,
    tool_event_id: String,
    force: Option<bool>,
) -> Result<ReapplyResult, String> {
    let (ws, data_dir, snapshots, message_id, session_id) = {
        let db = state.db.lock().unwrap();
        let snaps = store::list_snapshots_for_event(&db, &tool_event_id)?;
        if snaps.is_empty() {
            return Ok(ReapplyResult {
                success: true,
                reapplied_files: vec![],
                has_conflict: false,
                conflicted_files: vec![],
                message: "该工具调用无文件修改快照".into(),
            });
        }
        let session_id = snaps[0].session_id.clone();
        let message_id = snaps[0].message_id.clone();
        let session = store::get_session(&db, &session_id)?
            .ok_or_else(|| format!("会话未找到: {session_id}"))?;
        let dir = {
            let lock = state.data_dir.lock().unwrap();
            lock.clone().unwrap_or_else(|| state.default_data_dir.clone())
        };
        (PathBuf::from(session.workspace_path), dir, snaps, message_id, session_id)
    };

    let force = force.unwrap_or(false);
    let res = crate::snapshot_revert::reapply_snapshots(&ws, &data_dir, &snapshots, force).await?;

    if res.success && !snapshots.is_empty() {
        let db = state.db.lock().unwrap();
        store::mark_snapshot_reverted_for_event(&db, &tool_event_id, None)?;
        if let Ok(Some(msg)) = store::get_message(&db, &message_id) {
            state.emit("message:update", &msg);
        }
        state.emit("messages:changed", &serde_json::json!({ "sessionId": session_id }));
    }

    Ok(res)
}

#[tauri::command]
pub async fn get_turn_diff(
    state: State<'_, crate::AppState>,
    message_id: String,
) -> Result<Vec<SnapshotFileDiff>, String> {
    let (data_dir, snapshots) = {
        let db = state.db.lock().unwrap();
        let dir = {
            let lock = state.data_dir.lock().unwrap();
            lock.clone().unwrap_or_else(|| state.default_data_dir.clone())
        };
        let snaps = store::list_snapshots_for_turn(&db, &message_id)?;
        (dir, snaps)
    };

    crate::snapshot_revert::get_snapshots_diff_details(&data_dir, &snapshots).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_read_file_base64_logic() {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("test_read_base64.png");
        let sample_bytes = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
        tokio::fs::write(&test_file, sample_bytes).await.unwrap();

        let path_str = test_file.to_string_lossy().to_string();

        // 1. 原生路径读取
        let res = read_file_base64_inner(None, &path_str).await.unwrap();
        assert!(res.starts_with("data:image/png;base64,"));

        // 2. 带 file:/// 前缀读取
        let file_url = format!("file:///{}", path_str.replace('\\', "/"));
        let res2 = read_file_base64_inner(None, &file_url).await.unwrap();
        assert_eq!(res, res2);

        // 3. 不存在的文件测试
        let bad = read_file_base64_inner(None, "D:/non_existent_file_xyz_123.png").await;
        assert!(bad.is_err());

        // 4. 真实混合分隔符路径测试（若存在）
        let real_path = "D:\\WorkSpace\\IDEA\\yx-medical-manager\\generated_images/tech_dark_background.png";
        if std::path::Path::new(real_path).exists() {
            let res_real = read_file_base64_inner(None, real_path).await.unwrap();
            assert!(res_real.starts_with("data:image/png;base64,"));
        }

        let _ = tokio::fs::remove_file(test_file).await;
    }

    #[test]
    fn test_get_file_outline_and_revert_hunk() {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join(format!("test_hunk_revert_{}.rs", uuid::Uuid::new_v4()));
        let initial_code = "pub struct Account {\n    pub id: String,\n}\n\nimpl Account {\n    pub fn new() -> Self {\n        Self { id: \"1\".into() }\n    }\n}\n";
        std::fs::write(&test_file, initial_code).unwrap();
        let path_str = test_file.to_string_lossy().to_string();

        // 1. 测试大纲提取
        let outline = get_file_outline(path_str.clone()).unwrap();
        assert!(outline.iter().any(|item| item.symbol.contains("struct Account")));
        assert!(outline.iter().any(|item| item.symbol.contains("impl Account")));
        assert!(outline.iter().any(|item| item.symbol.contains("fn new")));

        // 2. 修改代码模拟局部变动
        let modified_code = "pub struct Account {\n    pub id: String,\n}\n\nimpl Account {\n    pub fn new() -> Self {\n        Self { id: \"2_modified\".into() }\n    }\n}\n";
        std::fs::write(&test_file, modified_code).unwrap();

        // 3. 计算 diff 并获取 hunk
        let diff = get_file_diff(path_str.clone(), Some(initial_code.into()), Some(modified_code.into()), None).unwrap();
        assert_eq!(diff.hunks.len(), 1);

        // 4. 执行撤销 revert_file_hunk
        let res = revert_file_hunk(path_str.clone(), diff.hunks[0].clone());
        assert!(res.is_ok());

        // 5. 验证文件恢复回 initial_code
        let reverted_code = std::fs::read_to_string(&test_file).unwrap();
        assert!(reverted_code.contains("id: \"1\".into()"));
        assert!(!reverted_code.contains("id: \"2_modified\".into()"));

        let _ = std::fs::remove_file(test_file);
    }
}






