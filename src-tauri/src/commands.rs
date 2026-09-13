use crate::agent;
use crate::llm::{self, LlmCfg};
use crate::models::*;
use crate::store;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, State};

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
    Ok(())
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
pub fn set_project_pinned(state: State<'_, crate::AppState>, id: String, pinned: bool) -> Result<(), String> {
    let db = state.db.lock().unwrap();
    store::set_project_pinned(&db, &id, pinned)
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
    let db = state.db.lock().unwrap();
    store::set_session_status(&db, &id, "archived")?;
    drop(db);
    emit_session(&state, &app, &id);
    let _ = app.emit("sessions:changed", json!({"archived": id}));
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
    let mode = if mode == "full_access" { Some("full_access") } else if mode == "confirm" { Some("confirm") } else { None };
    let db = state.db.lock().unwrap();
    store::set_session_mode(&db, &id, mode)?;
    drop(db);
    emit_session(&state, &app, &id);
    Ok(())
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
) -> Result<SendResult, String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("消息不能为空".into());
    }
    let (result, queued) = {
        let db = state.db.lock().unwrap();
        // 临时对话：首次发送时才真正落库
        let sid = match session_id {
            Some(id) => id,
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
                let s = store::create_session(&db, &ws, project_id.as_deref(), &title)?;
                let _ = app.emit("sessions:changed", json!({"created": s.id}));
                s.id
            }
        };
        let active = is_run_active(&state, &sid);
        let msg = store::new_message(&db, &sid, "user", Some(text), active)?;
        let msg_id = msg.id.clone();
        store::touch_session(&db, &sid)?;
        // 记住最近工作区（未绑定工作区的纯对话不覆盖）
        if let Ok(Some(s)) = store::get_session(&db, &sid) {
            let ws = s.workspace_path.clone();
            let mut settings = store::get_settings(&db).unwrap_or_default();
            if !ws.is_empty() {
                settings.last_workspace_path = Some(ws);
            }
            let mut clean = settings.clone();
            clean.approval_rules = vec![];
            if let Ok(json) = serde_json::to_string(&clean) {
                let _ = db.execute(
                    "INSERT INTO settings(key, value) VALUES('app_settings', ?1)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    rusqlite::params![json],
                );
            }
            let _ = app.emit("session:update", &s);
        }
        let r = SendResult { session_id: sid, message_id: msg_id.clone(), queued: active };
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
        store::update_message_content(&db, &message_id, &text, None)?;
        store::delete_messages_after(&db, &session_id, m.seq)?;
        store::touch_session(&db, &session_id)?;
    }
    let _ = app.emit("messages:changed", json!({"sessionId": session_id}));
    emit_session(&state, &app, &session_id);
    agent::spawn_session_task(app.clone(), session_id, Some(message_id));
    Ok(())
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
pub fn get_session_todos(state: State<'_, crate::AppState>, session_id: String) -> Result<Value, String> {
    let db = state.db.lock().unwrap();
    let raw: Option<String> = db
        .query_row(
            "SELECT value FROM session_kv WHERE session_id = ?1 AND key = 'todos'",
            rusqlite::params![session_id],
            |r| r.get(0),
        )
        .ok();
    Ok(raw.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null))
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
