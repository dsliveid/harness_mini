use crate::llm::{self, LlmCfg};
use crate::models::{resolve_active_model, TaskSubItem};
use crate::store;
use crate::task::memory::build_working_memory_block;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

/// 目标拆解逻辑（支持推断自动化验证门禁命令、多模块波次规划，并向会话实时流式传输思考与剖析过程）
pub async fn decompose_goal(
    app: &AppHandle,
    session_id: &str,
    plan_msg_id: &str,
    goal: &str,
) -> (Vec<TaskSubItem>, String, crate::models::ParsedTokenUsage) {
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
        let protocol = pc.get_model_protocol(&model);
        let reasoning_effort = session.as_ref()
            .and_then(|s| s.reasoning_effort.clone())
            .or_else(|| settings.reasoning_effort.clone())
            .or_else(|| Some("medium".to_string()));

        let cfg = LlmCfg {
            base_url: pc.base_url,
            api_key: pc.api_key,
            model,
            protocol,
            reasoning_effort,
            proxy_url: settings.effective_proxy_url(),
            session_id: Some(session_id.to_string()),
        };

        let os_platform = if cfg!(windows) {
            "Windows (终端命令需兼容 cmd/PowerShell，严禁在 Windows 环境输出 bash 等非原生命令)"
        } else {
            "Unix/Linux (支持标准 sh/bash 脚本)"
        };
        let sys_prompt = format!(
            "你是一个专家级 AI 架构师与长任务规划专家。当前系统运行环境为：{}。\n\
            用户将提供一个复杂的开发/编码任务总目标。请你将该目标拆解为 3 至 5 个目标明确且可独立执行验收的子任务步骤。\n\n\
            【规划与波次 (Wave) 编排要求】\n\
            1. 请在思考过程（或 <think> 标签）中深入剖析任务总目标、现有工程依赖、实现路线与各阶段验证门禁策略；\n\
            2. 【波次 (Wave) 编排与并行规划核心原则】：\n\
               - 编号相同的子任务将受控并行同时启动（系统底层并发池支持多个子任务同时推进并展示在顶栏与对话卡片中）；\n\
               - 若任务初始阶段的各个子任务彼此独立（例如前后端分别调研分析、或者互不冲突的独立模块开发），【直接从 wave: 1 就赋予相同的波次编号（如全部为 wave: 1），使系统一开始就同时并行推进多个子任务】；\n\
               - 仅当确实存在强前置接口契约（如必须先产出公共数据协议/接口类型才能写具体业务）时，才将前置依赖设为单独串行波次（如 wave: 1），后续模块设为 wave: 2；\n\
               - 最后的全局集成联调、报告汇总或整机回归验收，设为后续收敛波次；\n\
               - 若属于常规严格先后依赖任务，请顺序递增 wave 编号（1, 2, 3...）；\n\
            3. 请为每个适用的子任务推断最恰当的自动化验证命令（verifyCommand，例如 npm test, cargo check 等；严禁生成任何需要人工按键交互输入的命令）；\n\
            4. 若子任务负责特定模块子目录，请指明 scopePath（例如 crates/auth/ 或 src/modules/user/，可选）；\n\
            5. 思考完毕后，严格输出合法 JSON 数组（可包含在 ```json 代码块中）：\n\
            [{{\"title\": \"子任务名称\", \"description\": \"具体任务内容与可验收成果\", \"wave\": 1, \"scopePath\": \"可选模块目录\", \"verifyCommand\": \"可选自动化验证命令\"}}]",
            os_platform
        );

        let messages = vec![
            json!({ "role": "system", "content": sys_prompt }),
            json!({ "role": "user", "content": goal }),
        ];

        let reasoning_buf = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let text_buf = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let in_think_tag = std::sync::Arc::new(std::sync::Mutex::new(false));

        let r_acc = reasoning_buf.clone();
        let app_r = app.clone();
        let sid_r = session_id.to_string();
        let mid_r = plan_msg_id.to_string();
        let on_reasoning = move |delta: &str| {
            if !delta.is_empty() {
                r_acc.lock().unwrap().push_str(delta);
                let _ = app_r.emit(
                    "message:reasoning:delta",
                    json!({
                        "sessionId": &sid_r,
                        "messageId": &mid_r,
                        "delta": delta,
                    }),
                );
            }
        };

        let t_acc = text_buf.clone();
        let r_acc_text = reasoning_buf.clone();
        let in_think = in_think_tag.clone();
        let app_t = app.clone();
        let sid_t = session_id.to_string();
        let mid_t = plan_msg_id.to_string();
        let on_text = move |delta: &str| {
            t_acc.lock().unwrap().push_str(delta);
            let mut think_flag = in_think.lock().unwrap();
            if delta.contains("<think>") {
                *think_flag = true;
            }
            if *think_flag {
                let clean = delta.replace("<think>", "").replace("</think>", "");
                if !clean.is_empty() {
                    r_acc_text.lock().unwrap().push_str(&clean);
                    let _ = app_t.emit(
                        "message:reasoning:delta",
                        json!({
                            "sessionId": &sid_t,
                            "messageId": &mid_t,
                            "delta": clean,
                        }),
                    );
                }
                if delta.contains("</think>") {
                    *think_flag = false;
                }
            }
        };

        if let Ok(res) = llm::chat_stream(&cfg, &messages, &[], on_text, on_reasoning).await {
            let est_in = crate::models::estimate_tokens(&sys_prompt) + crate::models::estimate_tokens(goal);
            let est_out = crate::models::estimate_tokens(&res.content) + crate::models::estimate_tokens(&res.reasoning);
            let parsed_usage = crate::models::parse_llm_usage(res.usage.as_ref(), est_in, est_out);

            let state = app.state::<crate::AppState>();
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
            }

            let captured_reasoning = reasoning_buf.lock().unwrap().clone();
            let final_r = if captured_reasoning.is_empty() {
                res.reasoning.clone()
            } else {
                captured_reasoning
            };

            let full_text = text_buf.lock().unwrap().clone();
            let content_str = if full_text.trim().is_empty() {
                res.content.clone()
            } else {
                full_text
            };

            let clean = if let Some(end_idx) = content_str.find("</think>") {
                content_str[end_idx + 8..].trim()
            } else {
                content_str.trim()
            };

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
                    let wave = v.get("wave").and_then(|x| x.as_u64()).map(|w| w as usize).unwrap_or(i + 1);
                    let scope_path = v.get("scopePath").and_then(|x| x.as_str()).map(|s| s.to_string());
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
                        wave,
                        scope_path,
                    });
                }
                if !items.is_empty() {
                    return (items, final_r, parsed_usage);
                }
            }

            return (
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
                        wave: 1,
                        scope_path: None,
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
                        wave: 2,
                        scope_path: None,
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
                        wave: 3,
                        scope_path: None,
                    },
                ],
                final_r,
                parsed_usage,
            );
        }
    }

    // 默认兜底拆解
    (
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
                wave: 1,
                scope_path: None,
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
                wave: 2,
                scope_path: None,
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
                wave: 3,
                scope_path: None,
            },
        ],
        String::new(),
        crate::models::ParsedTokenUsage::default(),
    )
}

/// 动态重规划（遇到严重阻碍或新情况时自适应重整剩余路线图）
pub async fn replan_subtasks(
    app: &AppHandle,
    task_id: &str,
    cur_idx: usize,
    reason: &str,
) -> Result<bool, String> {
    let state = app.state::<crate::AppState>();
    let (mut task, pc, model, proxy_url) = {
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
        let proxy_url = settings.effective_proxy_url();
        (task, pc, model, proxy_url)
    };

    let protocol = pc.get_model_protocol(&model);
    let cfg = LlmCfg {
        base_url: pc.base_url,
        api_key: pc.api_key,
        model,
        protocol,
        reasoning_effort: None,
        proxy_url,
        session_id: Some(task_id.to_string()),
    };

    let cps = {
        let db = state.db.lock().unwrap();
        store::list_task_checkpoints(&db, task_id).unwrap_or_default()
    };
    let memory_block = build_working_memory_block(&cps, &task.goal);

    let cur_title = task.subtasks.get(cur_idx).map(|s| s.title.as_str()).unwrap_or("");
    let prompt = format!(
        "【长任务自适应动态重规划申请】\n总目标：{}\n\n{}\n\n当前推进至阶段 {}/{}：【{}】。\n执行过程中遇到以下阻碍或新情况：\n{}\n\n请重新审视当前工作区状态，对剩余未完成的后续子任务进行动态重规划（调整、细化或替换当前及后续子任务）。\n输出必须严格为 JSON 数组，严禁包含任何 Markdown 格式包裹或解释文本：\n[{{\"title\": \"调整后的阶段任务名称\", \"description\": \"具体任务内容与可验收成果\", \"wave\": 1, \"scopePath\": \"可选模块目录\", \"verifyCommand\": \"可选的自动化验证命令，如 cargo check 或 npm test\"}}]",
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
        let est_in = crate::models::estimate_tokens(&prompt);
        let est_out = crate::models::estimate_tokens(&res.content);
        let parsed_usage = crate::models::parse_llm_usage(res.usage.as_ref(), est_in, est_out);
        {
            let db = state.db.lock().unwrap();
            let _ = store::increment_session_tokens(
                &db,
                &task.session_id,
                parsed_usage.prompt_tokens,
                parsed_usage.completion_tokens,
                parsed_usage.total_tokens,
                parsed_usage.cached_tokens,
            );
        }
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
                let wave = v.get("wave").and_then(|x| x.as_u64()).map(|w| w as usize).unwrap_or(idx);
                let scope_path = v.get("scopePath").and_then(|x| x.as_str()).map(|s| s.to_string());
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
                    wave,
                    scope_path,
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
