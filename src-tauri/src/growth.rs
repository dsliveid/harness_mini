use crate::llm::{self, LlmCfg};
use crate::models::*;
use crate::store;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReflectionOutput {
    pub category: String,
    pub title: String,
    pub reflection_thought: String,
    pub rule_content: String,
}

fn clean_json_text(text: &str) -> &str {
    let t = text.trim();
    if let Some(rest) = t.strip_prefix("```json") {
        if let Some(end) = rest.rfind("```") {
            return rest[..end].trim();
        }
    } else if let Some(rest) = t.strip_prefix("```") {
        if let Some(end) = rest.rfind("```") {
            return rest[..end].trim();
        }
    }
    t
}

pub fn trigger_reflection_on_denial(
    app: AppHandle,
    session_id: String,
    tool_name: String,
    params: Value,
    reason: Option<String>,
) {
    tauri::async_runtime::spawn(async move {
        let state = app.state::<crate::AppState>();
        let (session, settings) = {
            let db = state.db.lock().unwrap();
            let master = state.master_key.lock().unwrap();
            (
                store::get_session(&db, &session_id).ok().flatten(),
                store::get_settings_with_secrets(&db, &master).unwrap_or_default(),
            )
        };
        let Some(session) = session else { return };
        let Some((pc, model)) = crate::models::resolve_active_model(&settings) else { return };
        let cfg = LlmCfg {
            base_url: pc.base_url.clone(),
            api_key: pc.api_key.clone(),
            model: model.to_string(),
            reasoning_effort: None,
        };

        let _ = app.emit(
            "growth:status",
            json!({
                "sessionId": session_id,
                "status": "analyzing",
                "message": "检测到用户纠偏，AI 正在反思并提炼经验..."
            }),
        );

        let system_msg = json!({
            "role": "system",
            "content": r#"你是一个敏锐的 AI 经验提炼器。你的任务是分析当前编程 Agent 在执行操作时被用户拒绝的场景，反思原因，并提炼出 1 条通用的规则或避坑建议。

请输出严格的 JSON 格式（不要输出任何额外的解释或开场白）：
{
  "category": "command_rule" | "code_style" | "build_test" | "pitfall" | "workflow",
  "title": "简要经验标题（15字以内）",
  "reflectionThought": "分析为什么该操作被拒绝、为何用户期望的方案更好（100字以内）",
  "ruleContent": "- 提炼出的具体规则，将注入到系统提示词中指导后续行动（Markdown 无序列表格式，如：- 某某场景下，应当...）"
}"#
        });

        let trigger_ctx = format!(
            "工具名称: {}\n参数: {}\n用户拒绝原因: {}",
            tool_name,
            params,
            reason.as_deref().unwrap_or("(未填写具体原因)")
        );

        let user_msg = json!({
            "role": "user",
            "content": format!(
                "工作区路径: {}\n\n执行被拒事件:\n{}",
                session.workspace_path,
                trigger_ctx
            )
        });

        let call = llm::chat_stream(&cfg, &[system_msg, user_msg], &[], |_| {}, |_| {}).await;
        match call {
            Ok(res) => {
                let cleaned = clean_json_text(&res.content);
                if let Ok(parsed) = serde_json::from_str::<ReflectionOutput>(cleaned) {
                    let now = chrono::Utc::now().to_rfc3339();
                    let growth = GrowthItem {
                        id: uuid::Uuid::new_v4().to_string(),
                        project_id: session.project_id.clone(),
                        session_id: Some(session_id.clone()),
                        session_title: Some(session.title.clone()),
                        message_id: None,
                        run_id: None,
                        trigger_type: "user_rejection".into(),
                        trigger_context: trigger_ctx,
                        reflection_thought: parsed.reflection_thought,
                        category: parsed.category,
                        title: parsed.title,
                        rule_content: parsed.rule_content,
                        status: "proposed".into(),
                        applied_count: 0,
                        created_at: now.clone(),
                        updated_at: now,
                    };

                    {
                        let db = state.db.lock().unwrap();
                        let _ = store::insert_growth(&db, &growth);
                    }

                    let _ = app.emit("growth:proposed", &growth);
                    let _ = app.emit(
                        "growth:status",
                        json!({
                            "sessionId": session_id,
                            "status": "proposed",
                            "growthId": growth.id,
                            "message": "经验反思提炼完成，已生成成长提案。"
                        }),
                    );
                } else {
                    let _ = app.emit(
                        "growth:status",
                        json!({
                            "sessionId": session_id,
                            "status": "idle",
                            "message": ""
                        }),
                    );
                }
            }
            Err(_) => {
                let _ = app.emit(
                    "growth:status",
                    json!({
                        "sessionId": session_id,
                        "status": "idle",
                        "message": ""
                    }),
                );
            }
        }
    });
}

pub fn trigger_manual_reflection(
    app: AppHandle,
    session_id: String,
    user_instruction: Option<String>,
) {
    tauri::async_runtime::spawn(async move {
        let state = app.state::<crate::AppState>();
        let (session, settings) = {
            let db = state.db.lock().unwrap();
            let master = state.master_key.lock().unwrap();
            (
                store::get_session(&db, &session_id).ok().flatten(),
                store::get_settings_with_secrets(&db, &master).unwrap_or_default(),
            )
        };
        let Some(session) = session else { return };
        let Some((pc, model)) = crate::models::resolve_active_model(&settings) else { return };
        let cfg = LlmCfg {
            base_url: pc.base_url.clone(),
            api_key: pc.api_key.clone(),
            model: model.to_string(),
            reasoning_effort: None,
        };

        let _ = app.emit(
            "growth:status",
            json!({
                "sessionId": session_id,
                "status": "analyzing",
                "message": "AI 正在分析本轮对话并提炼成长经验..."
            }),
        );

        let recent_msgs = {
            let db = state.db.lock().unwrap();
            store::all_messages(&db, &session_id).unwrap_or_default()
        };

        let mut dialogue_summary = String::new();
        for m in recent_msgs.iter().rev().take(10).rev() {
            if m.role == "user" || m.role == "assistant" {
                if let Some(c) = &m.content {
                    dialogue_summary.push_str(&format!("{}: {}\n", m.role, crate::models::truncate_result(c)));
                }
            }
        }

        let system_msg = json!({
            "role": "system",
            "content": r#"你是一个经验丰富的 AI 架构师与复盘提炼器。请分析本轮对话中涉及的编码经验、踩坑防雷点、代码风格或工作流，提炼出 1 条长期有价值的项目规则。

请输出严格的 JSON 格式（不要输出任何额外的解释或开场白）：
{
  "category": "command_rule" | "code_style" | "build_test" | "pitfall" | "workflow",
  "title": "简要经验标题（15字以内）",
  "reflectionThought": "分析为什么需要这条规则、该规则能预防什么问题（100字以内）",
  "ruleContent": "- 提炼出的具体规则，将注入到系统提示词中指导后续行动（Markdown 无序列表格式，如：- 某某场景下，应当...）"
}"#
        });

        let prompt_text = format!(
            "用户提示/要求: {}\n\n最近对话内容:\n{}",
            user_instruction.as_deref().unwrap_or("总结本轮对话的经验与注意事项"),
            dialogue_summary
        );

        let user_msg = json!({
            "role": "user",
            "content": prompt_text
        });

        let call = llm::chat_stream(&cfg, &[system_msg, user_msg], &[], |_| {}, |_| {}).await;
        if let Ok(res) = call {
            let cleaned = clean_json_text(&res.content);
            if let Ok(parsed) = serde_json::from_str::<ReflectionOutput>(cleaned) {
                let now = chrono::Utc::now().to_rfc3339();
                let growth = GrowthItem {
                    id: uuid::Uuid::new_v4().to_string(),
                    project_id: session.project_id.clone(),
                    session_id: Some(session_id.clone()),
                    session_title: Some(session.title.clone()),
                    message_id: None,
                    run_id: None,
                    trigger_type: "manual".into(),
                    trigger_context: user_instruction.unwrap_or_else(|| "手动复盘对话".into()),
                    reflection_thought: parsed.reflection_thought,
                    category: parsed.category,
                    title: parsed.title,
                    rule_content: parsed.rule_content,
                    status: "proposed".into(),
                    applied_count: 0,
                    created_at: now.clone(),
                    updated_at: now,
                };

                {
                    let db = state.db.lock().unwrap();
                    let _ = store::insert_growth(&db, &growth);
                }

                let _ = app.emit("growth:proposed", &growth);
                let _ = app.emit(
                    "growth:status",
                    json!({
                        "sessionId": session_id,
                        "status": "proposed",
                        "growthId": growth.id,
                        "message": "经验提炼完成，已生成成长提案。"
                    }),
                );
                return;
            }
        }

        let _ = app.emit(
            "growth:status",
            json!({
                "sessionId": session_id,
                "status": "idle",
                "message": ""
            }),
        );
    });
}
