use crate::approval::{self, Decision};
use crate::llm::{self, LlmCfg};
use crate::models::*;
use crate::store;
use crate::tools::{self, Risk, ToolCtx, ToolSpec};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
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

#[derive(PartialEq)]
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

    pub fn record_success(&mut self, tool_name: &str) {
        self.consecutive_failures.remove(tool_name);
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
    let state = app.state::<crate::AppState>();
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
    // 队列未空则继续依次执行
    spawn_session_task(app.clone(), session_id.to_string(), None);
}

async fn run_loop(app: AppHandle, session_id: String, mut trigger: Option<String>) {
    let state = app.state::<crate::AppState>();
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
        let Some(_trigger_id) = trigger_id else { break };

        // 2. 创建 Run
        let run_id = {
            let db = state.db.lock().unwrap();
            store::create_run(&db, &session_id).ok()
        };
        let Some(run_id) = run_id else { break };
        if let Some(h) = state.handles.lock().unwrap().get(&session_id) {
            *h.active_run.lock().unwrap() = Some(run_id.clone());
        }
        let _ = app.emit(
            "run:status",
            json!({"sessionId": session_id, "runId": run_id, "status": "running"}),
        );

        // 3. 运行一次 Agent 主循环
        let outcome = run_once(&app, &session_id, &run_id).await;
        let status = match outcome {
            RunOutcome::Done => "done",
            RunOutcome::Failed => "failed",
        };
        {
            let db = state.db.lock().unwrap();
            let _ = store::finish_run(&db, &run_id, status);
        }
        let _ = app.emit(
            "run:status",
            json!({"sessionId": session_id, "runId": run_id, "status": status}),
        );
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
async fn run_once(app: &AppHandle, session_id: &str, _run_id: &str) -> RunOutcome {
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
    let Some(session) = session else { return RunOutcome::Failed };
    let workspace = PathBuf::from(&session.workspace_path);
    // 临时空间会话：每次运行前检查/重建临时空间（拷贝缺失的项目副本 + 基线提交）
    if session.is_temp {
        if let Err(e) = crate::temp::ensure_space(&state, app, &session) {
            emit_error(app, session_id, "temp", format!("临时空间准备失败: {e}"));
            return RunOutcome::Failed;
        }
    }
    // 未绑定工作区（空字符串）的会话为纯对话模式，跳过存在性检查
    if !session.workspace_path.is_empty() && !workspace.exists() {
        emit_error(app, session_id, "workspace", format!("工作区不存在: {}", session.workspace_path));
        return RunOutcome::Failed;
    }

    // 解析模型配置：全局激活的厂商 + 模型，失效时回落到第一个有模型的厂商
    let Some((pc, model)) = resolve_active_model(&settings) else {
        emit_error(
            app,
            session_id,
            "no_model",
            "尚未配置模型厂商，请先在设置中添加厂商与模型并选择。".into(),
        );
        return RunOutcome::Failed;
    };
    let cfg = LlmCfg {
        base_url: pc.base_url.clone(),
        api_key: pc.api_key.clone(),
        model: model.to_string(),
    };

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
            session.temp_root.as_ref().map(PathBuf::from)
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
    let specs: Vec<ToolSpec> = tools::tool_specs()
        .into_iter()
        // 临时空间专用工具仅对临时空间会话下发
        .filter(|s| session.is_temp || !tools::TEMP_TOOL_NAMES.contains(&s.name))
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

    // 项目约束 + 关联项目说明：按会话所属项目动态组装进 system prompt。
    // system 位于上下文第 0 位且永不截断 → 新对话注入一次、同对话多条消息不重复；
    // 每次运行重建，修改约束 / 关联后同对话下一条消息就地生效。
    // 临时空间会话改用临时空间说明段（含临时路径映射与优先级 临时空间 > 关联项目 > 项目约束）
    let project_section = {
        let db = state.db.lock().unwrap();
        if session.is_temp {
            match crate::temp::load_manifest(&db, session_id) {
                Ok(Some(m)) => Some(crate::temp::build_prompt_section(&db, &session, &m)),
                _ => None,
            }
        } else {
            match &session.project_id {
                Some(pid) => build_project_section(&db, pid).unwrap_or(None),
                None => None,
            }
        }
    };
    let sys = system_prompt(&session, project_section.as_deref());
    let mut files_modified = false;
    let mut sop_retry_count = 0usize;
    let mut sop_verified = false;
    let mut tool_tracker = ToolErrorTracker::new(2);
    for _step in 0..max_steps {
        // ---- 步骤 1/2：组装上下文（含运行中被"引导"注入的新消息） ----
        let (messages, est_in) = {
            let db = state.db.lock().unwrap();
            build_context(&db, session_id, &sys, settings.context_token_limit)
        };
        let messages = match messages {
            Ok(m) => m,
            Err(e) => {
                emit_error(app, session_id, "context", e);
                return RunOutcome::Failed;
            }
        };

        // ---- 步骤 3：流式调用 LLM（先预占 assistant 消息位，供增量事件引用） ----
        let assistant_id = {
            let db = state.db.lock().unwrap();
            match store::new_message(&db, session_id, "assistant", Some(String::new()), false) {
                Ok(m) => m.id,
                Err(e) => {
                    emit_error(app, session_id, "db", e);
                    return RunOutcome::Failed;
                }
            }
        };
        let app2 = app.clone();
        let sid2 = session_id.to_string();
        let mid2 = assistant_id.clone();
        let mut last_flush = Instant::now();
        let buf2: std::sync::Arc<Mutex<String>> = std::sync::Arc::new(Mutex::new(String::new()));
        let rbuf2: std::sync::Arc<Mutex<String>> = std::sync::Arc::new(Mutex::new(String::new()));
        // 正文与思考增量各用一组捕获（闭包互斥持有，不能共享同一组 move 变量）
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
                    let _ = tauri::Emitter::emit(
                        &app_text,
                        "message:delta",
                        json!({"sessionId": sid_text, "messageId": mid_text, "delta": delta}),
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
                    let _ = tauri::Emitter::emit(
                        &app_reason,
                        "message:reasoning:delta",
                        json!({"sessionId": sid_reason, "messageId": mid_reason, "delta": delta}),
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

        let result = match call {
            Ok(r) => r,
            Err(e) => {
                emit_error(app, session_id, "llm", e);
                return RunOutcome::Failed;
            }
        };

        // 空回复防护：模型不支持流式/工具或名称错误时给出明确提示
        if result.tool_calls.is_empty() && result.content.trim().is_empty() {
            emit_error(
                app,
                session_id,
                "empty_response",
                "模型返回了空回复。请检查设置中的模型名称是否正确、该模型是否支持工具调用（function calling），或更换模型后重试。"
                    .into(),
            );
            return RunOutcome::Done;
        }

        let est_out = estimate_tokens(&result.content);
        let usage = json!({"inputEst": est_in, "outputEst": est_out});

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
            let _ = sop_verified;
            return RunOutcome::Done;
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

        for tc in &tcs {
            let args: Value = serde_json::from_str(&tc.args).unwrap_or_else(|_| {
                json!({"__parse_error": "工具参数不是有效 JSON"})
            });
            let (status, mut result_text) = handle_tool_call(
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
            .await;

            // 工具自动自纠错机制：非用户审批拒绝的执行失败触发内部微反思引导
            if status == "failed" {
                let (can_retry, count) = tool_tracker.record_failure(&tc.name);
                if can_retry {
                    let _ = app.emit(
                        "tool:retry_guidance",
                        json!({
                            "sessionId": session_id,
                            "toolName": tc.name,
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
                    result_text = format!(
                        "{result_text}\n\n【提示】: 工具 `{}` 已连续失败 {} 次（达到自纠错上限），请停止重复尝试，向用户如实陈述原因或尝试其他方案。",
                        tc.name, tool_tracker.max_retry_limit
                    );
                }
            } else if status == "success" {
                tool_tracker.record_success(&tc.name);
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
    RunOutcome::Done
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

    // 未绑定工作区的会话为纯对话模式：文件/命令工具不可用（todo 除外，仅记录计划）
    if ctx.workspace.as_os_str().is_empty() && tool_name != "todo" {
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

    if full_access && tool_name != "temp_merge" {
        scope = Some("mode");
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
    ev.status = status.clone();
    ev.result_text = Some(text.clone());
    {
        let db = state.db.lock().unwrap();
        // todo 工具：保存会话任务清单快照
        if tool_name == "todo" && status == "success" {
            let _ = store::set_kv(&db, session_id, "todos", &args.to_string());
        }
        let _ = store::update_tool_event(&db, &ev.id, &status, Some(&text), None);
    }
    emit_tool_with(app, session_id, &ev, json!({ "elapsedMs": elapsed }));
    (status, text)
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
        _ => serde_json::to_string_pretty(args).unwrap_or_default(),
    }
}

fn build_context(
    conn: &rusqlite::Connection,
    session_id: &str,
    system: &str,
    token_limit: usize,
) -> (Result<Vec<Value>, String>, usize) {
    let msgs = match store::all_messages(conn, session_id) {
        Ok(m) => m,
        Err(e) => return (Err(e), 0),
    };
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
    for m in &msgs {
        if m.queued {
            continue; // 待执行列表中的消息不进入上下文
        }
        match m.role.as_str() {
            "user" => {
                out.push(json!({"role": "user", "content": m.content.clone().unwrap_or_default()}));
                est += estimate_tokens(m.content.as_deref().unwrap_or(""));
            }
            "assistant" => {
                let mut obj = json!({"role": "assistant"});
                let content = m.content.clone().unwrap_or_default();
                match &m.tool_calls {
                    Some(tcs) if !tcs.is_null() && tcs.as_array().map(|a| !a.is_empty()).unwrap_or(false) => {
                        if !content.is_empty() {
                            obj["content"] = Value::String(content.clone());
                        }
                        obj["tool_calls"] = tcs.clone();
                        est += estimate_tokens(&content);
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

    // 截断：超限时丢弃最早的普通消息，但保留最后一条用户消息及其后内容
    if est > token_limit {
        let last_user = out
            .iter()
            .rposition(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
            .unwrap_or(1);
        let mut drop_from = 1usize;
        while est > token_limit && drop_from < last_user {
            let dropped = estimate_tokens(
                out[drop_from].get("content").and_then(|c| c.as_str()).unwrap_or(""),
            );
            est -= dropped;
            drop_from += 1;
        }
        if drop_from > 1 {
            out.drain(1..drop_from);
        }
    }
    (Ok(out), est)
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

fn system_prompt(session: &Session, project_section: Option<&str>) -> String {
    let os = if cfg!(windows) { "Windows" } else { "Unix-like" };
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
    } else {
        format!(
            r#"你是 harness_mini，一个谨慎、专业的编码 Agent，运行在用户的本地工作区中。

工作区根目录：{path}
操作系统：{os}

工作规则：
1. 修改文件前必须先用 read_file 读取相关内容，用 edit_file 做基于精确原文的最小化修改；新文件才用 write_file。
2. 动手前先用 glob / grep / list_dir 探索并理解代码结构。
3. 修改完成后，尽量用 run_command 运行构建或测试来验证改动。
4. 所有路径相对于工作区根目录，不要访问工作区之外的路径。
5. 不要执行破坏性命令（如递归删除、格式化磁盘等），它们会被强制要求用户确认。
6. 接到多步任务时，先用 todo 工具列出计划，并随进展更新各项状态。
7. 全程使用简体中文与用户交流；最终回复简洁总结：做了什么、改了哪些文件、验证结果如何。"#,
            path = session.workspace_path,
            os = os
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
        )
        .unwrap();
        let _ = u;
        let (msgs, _) = build_context(&conn, &s.id, sys, 28000);
        let msgs = msgs.unwrap();
        // system + user + assistant(tool_calls) + 合成 tool
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[3]["role"], "tool");
        assert_eq!(msgs[3]["tool_call_id"], "call_1");
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
        };
        let with = system_prompt(&session, Some("## 项目约束\nX"));
        assert!(with.contains("工作区根目录：D:\\ws"));
        assert!(with.ends_with("## 项目约束\nX"));
        let without = system_prompt(&session, None);
        assert!(!without.contains("项目约束"));
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
