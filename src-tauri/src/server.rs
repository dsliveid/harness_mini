use crate::commands;
use crate::models::*;
use axum::{
    extract::{
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};
use tower_http::cors::{Any, CorsLayer};

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DaemonInfo {
    pub pid: u32,
    pub port: u16,
    pub token: String,
    pub data_dir: String,
    pub started_at: String,
}

pub fn daemon_info_path(data_dir: &Path) -> PathBuf {
    data_dir.join("daemon.json")
}

pub fn save_daemon_info(data_dir: &Path, info: &DaemonInfo) -> Result<(), String> {
    let p = daemon_info_path(data_dir);
    let s = serde_json::to_string_pretty(info).map_err(|e| e.to_string())?;
    std::fs::write(p, s).map_err(|e| e.to_string())
}

pub fn read_daemon_info(data_dir: &Path) -> Option<DaemonInfo> {
    let p = daemon_info_path(data_dir);
    let s = std::fs::read_to_string(p).ok()?;
    serde_json::from_str(&s).ok()
}

pub fn remove_daemon_info(data_dir: &Path) {
    let p = daemon_info_path(data_dir);
    let _ = std::fs::remove_file(p);
}

pub async fn check_daemon_alive(port: u16) -> bool {
    let url = format!("http://127.0.0.1:{}/health", port);
    if let Ok(resp) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(800))
        .build()
        .unwrap()
        .get(&url)
        .send()
        .await
    {
        resp.status().is_success()
    } else {
        false
    }
}

pub async fn start_server(
    app_handle: AppHandle,
    port_tx: tokio::sync::oneshot::Sender<u16>,
) -> Result<(), String> {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/api/rpc", post(rpc_handler))
        .route("/ws", get(ws_handler))
        .layer(cors)
        .with_state(app_handle);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let _ = port_tx.send(port);

    axum::serve(listener, app)
        .await
        .map_err(|e| e.to_string())
}

async fn health_handler(State(app): State<AppHandle>) -> Json<Value> {
    let state = app.state::<crate::AppState>();
    let pid = std::process::id();
    let db = state.db.lock().unwrap();
    let runs = crate::store::all_running_runs(&db).unwrap_or_default();
    Json(json!({
        "status": "ok",
        "pid": pid,
        "runningSessions": runs.into_iter().map(|(s, r)| json!({"sessionId": s, "runId": r})).collect::<Vec<_>>()
    }))
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(app): State<AppHandle>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, app))
}

async fn handle_socket(mut socket: WebSocket, app: AppHandle) {
    let state = app.state::<crate::AppState>();
    let mut rx = state.event_bus.subscribe();

    loop {
        tokio::select! {
            ev = rx.recv() => {
                match ev {
                    Ok((event_name, payload)) => {
                        let msg = json!({
                            "event": event_name,
                            "payload": payload,
                        });
                        if socket.send(WsMessage::Text(msg.to_string())).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(WsMessage::Text(text))) => {
                        if let Ok(req) = serde_json::from_str::<Value>(&text) {
                            let id = req.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());
                            let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
                            let params = req.get("params").cloned().unwrap_or(Value::Null);

                            let res = dispatch_rpc(&app, method, params).await;
                            let resp = match res {
                                Ok(data) => json!({
                                    "id": id,
                                    "ok": true,
                                    "data": data,
                                }),
                                Err(err) => json!({
                                    "id": id,
                                    "ok": false,
                                    "error": err,
                                }),
                            };
                            if socket.send(WsMessage::Text(resp.to_string())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Some(Ok(WsMessage::Ping(p))) => {
                        if socket.send(WsMessage::Pong(p)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(WsMessage::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }
}

async fn rpc_handler(
    State(app): State<AppHandle>,
    Json(payload): Json<Value>,
) -> Json<Value> {
    let method = payload.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let params = payload.get("params").cloned().unwrap_or(Value::Null);
    match dispatch_rpc(&app, method, params).await {
        Ok(data) => Json(json!({ "ok": true, "data": data })),
        Err(err) => Json(json!({ "ok": false, "error": err })),
    }
}

pub async fn dispatch_rpc(
    app: &AppHandle,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let st = app.state::<crate::AppState>();
    match method {
        // ---- 运行时快照恢复 ----
        "get_session_active_state" => {
            let session_id = params
                .get("sessionId")
                .and_then(|v| v.as_str())
                .ok_or("缺少 sessionId")?;
            let res = commands::get_session_active_state(st, session_id.to_string())?;
            Ok(serde_json::to_value(res).map_err(|e| e.to_string())?)
        }

        // ---- 设置 ----
        "get_settings" => {
            let res = commands::get_settings(st)?;
            Ok(serde_json::to_value(res).map_err(|e| e.to_string())?)
        }
        "set_settings" => {
            let settings: SettingsData = serde_json::from_value(
                params.get("settings").cloned().unwrap_or(Value::Null),
            )
            .map_err(|e| format!("参数解析错误: {e}"))?;
            commands::set_settings(st, settings)?;
            Ok(Value::Null)
        }
        "list_tools" => {
            let res = commands::list_tools()?;
            Ok(serde_json::to_value(res).map_err(|e| e.to_string())?)
        }
        "test_provider" => {
            let provider: ProviderCfg = serde_json::from_value(
                params.get("provider").cloned().unwrap_or(Value::Null),
            )
            .map_err(|e| format!("参数解析错误: {e}"))?;
            let msg = commands::test_provider(st, provider).await?;
            Ok(json!(msg))
        }

        // ---- 项目 ----
        "list_projects" => {
            let res = commands::list_projects(st)?;
            Ok(serde_json::to_value(res).map_err(|e| e.to_string())?)
        }
        "create_project" => {
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let path = params.get("path").and_then(|v| v.as_str()).map(|s| s.to_string());
            let res = commands::create_project(st, app.clone(), name, path)?;
            Ok(serde_json::to_value(res).map_err(|e| e.to_string())?)
        }
        "remove_project" => {
            let id = params.get("id").and_then(|v| v.as_str()).ok_or("缺少 id")?.to_string();
            commands::remove_project(st, app.clone(), id)?;
            Ok(Value::Null)
        }
        "set_project_pinned" => {
            let id = params.get("id").and_then(|v| v.as_str()).ok_or("缺少 id")?.to_string();
            let pinned = params.get("pinned").and_then(|v| v.as_bool()).unwrap_or(false);
            commands::set_project_pinned(st, app.clone(), id, pinned)?;
            Ok(Value::Null)
        }
        "set_project_constraints" => {
            let id = params.get("id").and_then(|v| v.as_str()).ok_or("缺少 id")?.to_string();
            let constraints = params.get("constraints").and_then(|v| v.as_str()).unwrap_or("").to_string();
            commands::set_project_constraints(st, app.clone(), id, constraints)?;
            Ok(Value::Null)
        }
        "set_project_plan_mode" => {
            let id = params.get("id").and_then(|v| v.as_str()).ok_or("缺少 id")?.to_string();
            let mode = params.get("mode").and_then(|v| v.as_str()).unwrap_or("standard").to_string();
            commands::set_project_plan_mode(st, app.clone(), id, mode)?;
            Ok(Value::Null)
        }
        "list_project_links" => {
            let project_id = params.get("projectId").and_then(|v| v.as_str()).ok_or("缺少 projectId")?.to_string();
            let res = commands::list_project_links(st, project_id)?;
            Ok(serde_json::to_value(res).map_err(|e| e.to_string())?)
        }
        "add_project_link" => {
            let project_id = params.get("projectId").and_then(|v| v.as_str()).ok_or("缺少 projectId")?.to_string();
            let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let description = params.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let res = commands::add_project_link(st, project_id, path, description)?;
            Ok(serde_json::to_value(res).map_err(|e| e.to_string())?)
        }
        "update_project_link" => {
            let id = params.get("id").and_then(|v| v.as_str()).ok_or("缺少 id")?.to_string();
            let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let description = params.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let res = commands::update_project_link(st, id, path, description)?;
            Ok(serde_json::to_value(res).map_err(|e| e.to_string())?)
        }
        "delete_project_link" => {
            let id = params.get("id").and_then(|v| v.as_str()).ok_or("缺少 id")?.to_string();
            commands::delete_project_link(st, id)?;
            Ok(Value::Null)
        }

        // ---- 会话 ----
        "list_sessions" => {
            let res = commands::list_sessions(st)?;
            Ok(serde_json::to_value(res).map_err(|e| e.to_string())?)
        }
        "list_archived" => {
            let res = commands::list_archived(st)?;
            Ok(serde_json::to_value(res).map_err(|e| e.to_string())?)
        }
        "rename_session" => {
            let id = params.get("id").and_then(|v| v.as_str()).ok_or("缺少 id")?.to_string();
            let title = params.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
            commands::rename_session(st, app.clone(), id, title)?;
            Ok(Value::Null)
        }
        "delete_session" => {
            let id = params.get("id").and_then(|v| v.as_str()).ok_or("缺少 id")?.to_string();
            commands::delete_session(st, app.clone(), id)?;
            Ok(Value::Null)
        }
        "archive_session" => {
            let id = params.get("id").and_then(|v| v.as_str()).ok_or("缺少 id")?.to_string();
            commands::archive_session(st, app.clone(), id)?;
            Ok(Value::Null)
        }
        "unarchive_session" => {
            let id = params.get("id").and_then(|v| v.as_str()).ok_or("缺少 id")?.to_string();
            commands::unarchive_session(st, app.clone(), id)?;
            Ok(Value::Null)
        }
        "set_session_mode" => {
            let id = params.get("id").and_then(|v| v.as_str()).ok_or("缺少 id")?.to_string();
            let mode = params.get("mode").and_then(|v| v.as_str()).unwrap_or("confirm").to_string();
            commands::set_session_mode(st, app.clone(), id, mode)?;
            Ok(Value::Null)
        }
        "set_session_workspace" => {
            let id = params.get("id").and_then(|v| v.as_str()).ok_or("缺少 id")?.to_string();
            let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
            commands::set_session_workspace(st, app.clone(), id, path)?;
            Ok(Value::Null)
        }
        "set_session_project" => {
            let id = params.get("id").and_then(|v| v.as_str()).ok_or("缺少 id")?.to_string();
            let project_id = params.get("projectId").and_then(|v| v.as_str()).map(|s| s.to_string());
            commands::set_session_project(st, app.clone(), id, project_id)?;
            Ok(Value::Null)
        }

        // ---- 消息与执行 ----
        "get_messages" => {
            let session_id = params.get("sessionId").and_then(|v| v.as_str()).ok_or("缺少 sessionId")?.to_string();
            let before_seq = params.get("beforeSeq").and_then(|v| v.as_i64());
            let limit = params.get("limit").and_then(|v| v.as_i64());
            let res = commands::get_messages(st, session_id, before_seq, limit)?;
            Ok(serde_json::to_value(res).map_err(|e| e.to_string())?)
        }
        "fork_session_at_message" => {
            let session_id = params.get("sessionId").and_then(|v| v.as_str()).ok_or("缺少 sessionId")?.to_string();
            let message_id = params.get("messageId").and_then(|v| v.as_str()).ok_or("缺少 messageId")?.to_string();
            let new_title = params.get("newTitle").and_then(|v| v.as_str()).map(|s| s.to_string());
            let include_target = params.get("includeTarget").and_then(|v| v.as_bool());
            let res = commands::fork_session_at_message(app.clone(), st, session_id, message_id, new_title, include_target)?;
            Ok(serde_json::to_value(res).map_err(|e| e.to_string())?)
        }
        "send_message" => {
            let session_id = params.get("sessionId").and_then(|v| v.as_str()).map(|s| s.to_string());
            let text = params.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let workspace_path = params.get("workspacePath").and_then(|v| v.as_str()).map(|s| s.to_string());
            let project_id = params.get("projectId").and_then(|v| v.as_str()).map(|s| s.to_string());
            let temp: Option<TempAlloc> = params.get("temp").and_then(|v| serde_json::from_value(v.clone()).ok());
            let access_mode = params.get("accessMode").and_then(|v| v.as_str()).map(|s| s.to_string());
            let context_token_limit = params.get("contextTokenLimit").and_then(|v| v.as_u64()).map(|s| s as usize);
            let attachments: Option<Vec<crate::models::Attachment>> = params.get("attachments").and_then(|v| serde_json::from_value(v.clone()).ok());
            let image_provider_id = params.get("imageProviderId").and_then(|v| v.as_str()).map(|s| s.to_string());
            let image_model_id = params.get("imageModelId").and_then(|v| v.as_str()).map(|s| s.to_string());
            let vision_provider_id = params.get("visionProviderId").and_then(|v| v.as_str()).map(|s| s.to_string());
            let vision_model_id = params.get("visionModelId").and_then(|v| v.as_str()).map(|s| s.to_string());
            let res = commands::send_message(st, app.clone(), session_id, text, workspace_path, project_id, temp, access_mode, context_token_limit, attachments, image_provider_id, image_model_id, vision_provider_id, vision_model_id)?;
            Ok(serde_json::to_value(res).map_err(|e| e.to_string())?)
        }
        "list_queued" => {
            let session_id = params.get("sessionId").and_then(|v| v.as_str()).ok_or("缺少 sessionId")?.to_string();
            let res = commands::list_queued(st, session_id)?;
            Ok(serde_json::to_value(res).map_err(|e| e.to_string())?)
        }
        "guide_message" => {
            let session_id = params.get("sessionId").and_then(|v| v.as_str()).ok_or("缺少 sessionId")?.to_string();
            let message_id = params.get("messageId").and_then(|v| v.as_str()).ok_or("缺少 messageId")?.to_string();
            commands::guide_message(st, app.clone(), session_id, message_id)?;
            Ok(Value::Null)
        }
        "delete_queued_message" => {
            let session_id = params.get("sessionId").and_then(|v| v.as_str()).ok_or("缺少 sessionId")?.to_string();
            let message_id = params.get("messageId").and_then(|v| v.as_str()).ok_or("缺少 messageId")?.to_string();
            commands::delete_queued_message(st, app.clone(), session_id, message_id)?;
            Ok(Value::Null)
        }
        "stop_run" => {
            let session_id = params.get("sessionId").and_then(|v| v.as_str()).ok_or("缺少 sessionId")?.to_string();
            commands::stop_run(app.clone(), session_id)?;
            Ok(Value::Null)
        }
        "kill_command" => {
            let event_id = params.get("eventId").and_then(|v| v.as_str()).ok_or("缺少 eventId")?.to_string();
            commands::kill_command(app.clone(), st, event_id)?;
            Ok(Value::Null)
        }
        "list_running_sessions" => {
            let res = commands::list_running_sessions(st)?;
            Ok(serde_json::to_value(res).map_err(|e| e.to_string())?)
        }
        "edit_and_resend" => {
            let session_id = params.get("sessionId").and_then(|v| v.as_str()).ok_or("缺少 sessionId")?.to_string();
            let message_id = params.get("messageId").and_then(|v| v.as_str()).ok_or("缺少 messageId")?.to_string();
            let new_text = params.get("newText").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let attachments: Option<Vec<crate::models::Attachment>> = params.get("attachments").and_then(|v| serde_json::from_value(v.clone()).ok());
            commands::edit_and_resend(st, app.clone(), session_id, message_id, new_text, attachments)?;
            Ok(Value::Null)
        }

        // ---- 审批与交互 ----
        "respond_approval" => {
            let event_id = params.get("eventId").and_then(|v| v.as_str()).ok_or("缺少 eventId")?.to_string();
            let decision = params.get("decision").and_then(|v| v.as_str()).ok_or("缺少 decision")?.to_string();
            let reason = params.get("reason").and_then(|v| v.as_str()).map(|s| s.to_string());
            commands::respond_approval(st, event_id, decision, reason)?;
            Ok(Value::Null)
        }
        "respond_compaction" => {
            let event_id = params.get("eventId").and_then(|v| v.as_str()).ok_or("缺少 eventId")?.to_string();
            let approved = params.get("approved").and_then(|v| v.as_bool()).unwrap_or(true);
            let final_summary = params.get("finalSummary").and_then(|v| v.as_str()).unwrap_or("").to_string();
            commands::respond_compaction(st, app.clone(), event_id, approved, final_summary)?;
            Ok(Value::Null)
        }
        "list_session_compactions" => {
            let session_id = params.get("sessionId").and_then(|v| v.as_str()).ok_or("缺少 sessionId")?.to_string();
            let res = commands::list_session_compactions(st, session_id)?;
            Ok(serde_json::to_value(res).map_err(|e| e.to_string())?)
        }
        "list_session_rules" => {
            let session_id = params.get("sessionId").and_then(|v| v.as_str()).ok_or("缺少 sessionId")?.to_string();
            let res = commands::list_session_rules(st, session_id)?;
            Ok(serde_json::to_value(res).map_err(|e| e.to_string())?)
        }
        "delete_session_rule" => {
            let session_id = params.get("sessionId").and_then(|v| v.as_str()).ok_or("缺少 sessionId")?.to_string();
            let id = params.get("id").and_then(|v| v.as_str()).ok_or("缺少 id")?.to_string();
            let res = commands::delete_session_rule(st, app.clone(), session_id, id)?;
            Ok(serde_json::to_value(res).map_err(|e| e.to_string())?)
        }
        "get_session_todos" => {
            let session_id = params.get("sessionId").and_then(|v| v.as_str()).ok_or("缺少 sessionId")?.to_string();
            let res = commands::get_session_todos(st, session_id)?;
            Ok(serde_json::to_value(res).map_err(|e| e.to_string())?)
        }

        // ---- 临时空间与文件 ----
        "alloc_temp_code" => {
            let project_id = params.get("projectId").and_then(|v| v.as_str()).ok_or("缺少 projectId")?.to_string();
            let res = commands::alloc_temp_code(st, project_id)?;
            Ok(serde_json::to_value(res).map_err(|e| e.to_string())?)
        }
        "get_temp_info" => {
            let session_id = params.get("sessionId").and_then(|v| v.as_str()).ok_or("缺少 sessionId")?.to_string();
            let res = commands::get_temp_info(st, session_id)?;
            Ok(serde_json::to_value(res).map_err(|e| e.to_string())?)
        }
        "list_temp_changes" => {
            let session_id = params.get("sessionId").and_then(|v| v.as_str()).ok_or("缺少 sessionId")?.to_string();
            let res = commands::list_temp_changes(st, session_id)?;
            Ok(serde_json::to_value(res).map_err(|e| e.to_string())?)
        }
        "get_temp_change_diff" => {
            let session_id = params.get("sessionId").and_then(|v| v.as_str()).ok_or("缺少 sessionId")?.to_string();
            let project_key = params.get("projectKey").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let res = commands::get_temp_change_diff(st, session_id, project_key, path)?;
            Ok(serde_json::to_value(res).map_err(|e| e.to_string())?)
        }
        "merge_temp_space" => {
            let session_id = params.get("sessionId").and_then(|v| v.as_str()).ok_or("缺少 sessionId")?.to_string();
            let res = commands::merge_temp_space(st, app.clone(), session_id, None).await?;
            Ok(serde_json::to_value(res).map_err(|e| e.to_string())?)
        }
        "clear_temp_space" => {
            let session_id = params.get("sessionId").and_then(|v| v.as_str()).ok_or("缺少 sessionId")?.to_string();
            commands::clear_temp_space(app.clone(), session_id).await?;
            Ok(Value::Null)
        }

        // ---- 成长经验与 SOP ----
        "list_growths" => {
            let project_id = params.get("projectId").and_then(|v| v.as_str()).map(|s| s.to_string());
            let status = params.get("status").and_then(|v| v.as_str()).map(|s| s.to_string());
            let res = commands::list_growths(st, project_id, status)?;
            Ok(serde_json::to_value(res).map_err(|e| e.to_string())?)
        }
        "update_growth_status" => {
            let id = params.get("id").and_then(|v| v.as_str()).ok_or("缺少 id")?.to_string();
            let status = params.get("status").and_then(|v| v.as_str()).unwrap_or("active").to_string();
            commands::update_growth_status(st, app.clone(), id, status)?;
            Ok(Value::Null)
        }
        "delete_growth" => {
            let id = params.get("id").and_then(|v| v.as_str()).ok_or("缺少 id")?.to_string();
            commands::delete_growth(st, app.clone(), id)?;
            Ok(Value::Null)
        }
        "trigger_growth_reflection" => {
            let session_id = params.get("sessionId").and_then(|v| v.as_str()).ok_or("缺少 sessionId")?.to_string();
            let user_instruction = params.get("userInstruction").and_then(|v| v.as_str()).map(|s| s.to_string());
            commands::trigger_growth_reflection(app.clone(), session_id, user_instruction)?;
            Ok(Value::Null)
        }
        "get_token_stats" => {
            let project_id = params.get("projectId").and_then(|v| v.as_str()).map(|s| s.to_string());
            let days = params.get("days").and_then(|v| v.as_u64()).map(|d| d as u32);
            let res = commands::get_token_stats(st, project_id, days)?;
            Ok(serde_json::to_value(res).map_err(|e| e.to_string())?)
        }

        // ---- 数据目录状态 ----
        "get_data_status" => {
            let res = commands::get_data_status(st);
            Ok(serde_json::to_value(res).map_err(|e| e.to_string())?)
        }

        // ---- 打开本地目录与路径探测 ----
        "open_dir" => {
            let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let workspace_path = params
                .get("workspacePath")
                .or_else(|| params.get("workspace_path"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            commands::open_dir(st, path, workspace_path)?;
            Ok(Value::Null)
        }

        "inspect_path" => {
            let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let workspace_path = params
                .get("workspacePath")
                .or_else(|| params.get("workspace_path"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let res = commands::inspect_path(st, path, workspace_path);
            Ok(serde_json::to_value(res).map_err(|e| e.to_string())?)
        }

        _ => Err(format!("未知的 RPC 方法: {}", method)),
    }
}
