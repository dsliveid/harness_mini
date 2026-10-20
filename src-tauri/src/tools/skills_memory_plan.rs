use super::file::record_file_snapshot;
use super::command::run_command;
use crate::models::truncate_result;
use std::path::PathBuf;
use tauri::{Emitter, Manager};
use super::helpers::*;
use super::types::*;
use serde_json::{json, Value};
use std::path::Path;

pub(crate) async fn list_skills_tool(ctx: &ToolCtx) -> Result<String, String> {
    let skills = crate::skills::list_skills(&ctx.workspace).await?;
    if skills.is_empty() {
        return Ok("当前工作区尚未定义任何技能。可通过 save_skill 工具将常用流程或脚本固化为技能。".into());
    }
    let mut out = format!("当前工作区共有 {} 个技能：\n", skills.len());
    for s in &skills {
        out.push_str(&format!("- **{}** ({}): {}\n  路径: {}\n", s.name, s.script_type, s.description, s.path));
    }
    Ok(truncate_result(&out))
}

pub(crate) async fn save_skill_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let name = args.get("name").and_then(|v| v.as_str()).ok_or("缺少 name")?;
    let description = args.get("description").and_then(|v| v.as_str()).ok_or("缺少 description")?;
    let script_type = args.get("script_type").and_then(|v| v.as_str()).ok_or("缺少 script_type")?;
    let script_content = args.get("script_content").and_then(|v| v.as_str()).ok_or("缺少 script_content")?;

    let saved = crate::skills::save_skill(&ctx.workspace, name, description, script_type, script_content).await?;
    Ok(format!("已成功将技能【{}】保存至 `{}`，后续可通过 `run_skill` 调用该技能。", saved.name, saved.path))
}

pub(crate) async fn run_skill_tool(
    args: &Value,
    ctx: &ToolCtx,
    on_partial: PartialCb<'_>,
) -> Result<String, String> {
    let name = args.get("name").and_then(|v| v.as_str()).ok_or("缺少 name")?;
    let skill_args = args.get("args").and_then(|v| v.as_str());

    let (cmd, cwd) = crate::skills::build_skill_command(&ctx.workspace, name, skill_args)?;
    let cmd_args = json!({
        "command": cmd,
        "cwd": cwd.to_string_lossy()
    });
    run_command(&cmd_args, ctx, on_partial).await
}

// ---------- 项目专属知识与记忆工具实现 ----------

pub(crate) async fn record_memory_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let category = args.get("category").and_then(|v| v.as_str()).ok_or("缺少 category 参数")?;
    let title = args.get("title").and_then(|v| v.as_str()).ok_or("缺少 title 参数")?;
    let content = args.get("content").and_then(|v| v.as_str()).ok_or("缺少 content 参数")?;


    let effect = crate::memory::record_memory_with_effect(&ctx.workspace, category, title, content)?;
    record_file_snapshot(ctx, &effect.rel_path, effect.old_bytes.as_deref(), &effect.new_bytes).await;
    Ok(effect.message)
}

pub(crate) async fn read_memory_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let topic = args.get("topic").and_then(|v| v.as_str());
    crate::memory::read_memory(&ctx.workspace, topic)
}

// ---------- 任务方案计划中枢工具实现 ----------

pub(crate) async fn create_plan_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let title = args.get("title").and_then(|v| v.as_str()).ok_or("缺少 title 参数")?;
    let goals = args.get("goals").and_then(|v| v.as_str()).ok_or("缺少 goals 参数")?;
    let architecture = args.get("architecture").and_then(|v| v.as_str()).ok_or("缺少 architecture 参数")?;
    let files: Vec<String> = args
        .get("files")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    let steps: Vec<String> = args
        .get("steps")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    let verification = args.get("verification").and_then(|v| v.as_str());

    let session_id = ctx.host.as_ref().map(|h| h.session_id.as_str()).unwrap_or("");
    let app = ctx.host.as_ref().map(|h| &h.app);
    let state = app.map(|a| a.state::<crate::AppState>());
    let effect = {
        let db_guard = state.as_ref().map(|s| s.db.lock().unwrap());
        crate::plan::create_plan_with_effect(
            &ctx.workspace,
            session_id,
            title,
            goals,
            architecture,
            &files,
            &steps,
            verification,
            ctx.message_id.as_deref(),
            ctx.event_id.as_deref(),
            db_guard.as_deref(),
        )?
    };
    drop(state);

    if let (Some(rel), Some(nb)) = (&effect.suspended_plan_rel, &effect.suspended_new_bytes) {
        record_file_snapshot(ctx, rel, effect.suspended_old_bytes.as_deref(), nb).await;
    }
    record_file_snapshot(ctx, &effect.new_plan_rel, None, &effect.new_bytes).await;

    Ok(effect.message)
}

pub(crate) async fn update_plan_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let reason = args.get("reason").and_then(|v| v.as_str()).ok_or("缺少 reason 参数")?;
    let plan_id = args.get("plan_id").and_then(|v| v.as_str());
    let status = args.get("status").and_then(|v| v.as_str());
    let step_updates = args.get("step_updates").and_then(|v| v.as_array()).map(|v| v.as_slice());
    let modified_sections = args.get("modified_sections");
    let revision_note = args.get("revision_note").and_then(|v| v.as_str());

    let session_id = ctx.host.as_ref().map(|h| h.session_id.as_str()).unwrap_or("");
    let app = ctx.host.as_ref().map(|h| &h.app);
    let state = app.map(|a| a.state::<crate::AppState>());
    let effect = {
        let db_guard = state.as_ref().map(|s| s.db.lock().unwrap());
        crate::plan::update_plan_with_effect(
            &ctx.workspace,
            session_id,
            plan_id,
            reason,
            status,
            step_updates,
            modified_sections,
            revision_note,
            ctx.message_id.as_deref(),
            ctx.event_id.as_deref(),
            db_guard.as_deref(),
        )?
    };
    drop(state);

    record_file_snapshot(ctx, &effect.rel_path, Some(&effect.old_bytes), &effect.new_bytes).await;

    Ok(effect.message)
}

pub(crate) async fn switch_plan_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let plan_id = args.get("plan_id").and_then(|v| v.as_str()).ok_or("缺少 plan_id 参数")?;
    let session_id = ctx.host.as_ref().map(|h| h.session_id.as_str()).unwrap_or("");
    let app = ctx.host.as_ref().map(|h| &h.app);
    let state = app.map(|a| a.state::<crate::AppState>());
    let db_guard = state.as_ref().map(|s| s.db.lock().unwrap());
    let db_ref = db_guard.as_deref();

    crate::plan::switch_plan(
        &ctx.workspace,
        session_id,
        plan_id,
        ctx.message_id.as_deref(),
        ctx.event_id.as_deref(),
        db_ref,
    )
}

pub(crate) async fn read_plan_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let plan_id = args.get("plan_id").and_then(|v| v.as_str());
    let session_id = ctx.host.as_ref().map(|h| h.session_id.as_str()).unwrap_or("");
    let app = ctx.host.as_ref().map(|h| &h.app);
    let state = app.map(|a| a.state::<crate::AppState>());
    let db_guard = state.as_ref().map(|s| s.db.lock().unwrap());
    let db_ref = db_guard.as_deref();

    crate::plan::read_plan(&ctx.workspace, session_id, plan_id, db_ref)
}

pub(crate) async fn list_plans_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let include_archived = get_bool_arg(args, "include_archived", false);
    let session_id = ctx.host.as_ref().map(|h| h.session_id.as_str());
    let app = ctx.host.as_ref().map(|h| &h.app);
    let state = app.map(|a| a.state::<crate::AppState>());
    let db_guard = state.as_ref().map(|s| s.db.lock().unwrap());
    let db_ref = db_guard.as_deref();

    let list = crate::plan::list_plans(&ctx.workspace, session_id, include_archived, db_ref)?;
    serde_json::to_string_pretty(&list).map_err(|e| e.to_string())
}

// ---------- 协同简报组装、双轨交付工件与摘要提炼辅助 ----------


pub(crate) async fn generate_image_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let host = ctx.host.as_ref().ok_or("内部错误：缺少宿主上下文")?;
    let state = host.app.state::<crate::AppState>();
    let session_id = &host.session_id;

    let (session, parent_session) = {
        let db = state.db.lock().unwrap();
        let s = crate::store::get_session(&db, session_id)?
            .ok_or_else(|| format!("会话不存在: {session_id}"))?;
        let p = s.parent_session_id.as_deref()
            .and_then(|pid| crate::store::get_session(&db, pid).ok().flatten());
        (s, p)
    };

    let settings = {
        let db = state.db.lock().unwrap();
        let master = state.master_key.lock().unwrap();
        crate::store::get_settings_with_secrets(&db, &master)?
    };

    let requested_model = args.get("model").and_then(|v| v.as_str()).map(|s| s.trim()).filter(|s| !s.is_empty());

    let effective_image_provider_id = session.image_provider_id.as_deref()
        .or_else(|| parent_session.as_ref().and_then(|p| p.image_provider_id.as_deref()));
    let effective_image_model_id = session.image_model_id.as_deref()
        .or_else(|| parent_session.as_ref().and_then(|p| p.image_model_id.as_deref()));
    let effective_provider_id = session.provider_id.as_deref()
        .or_else(|| parent_session.as_ref().and_then(|p| p.provider_id.as_deref()));
    let effective_model_id = session.model_id.as_deref()
        .or_else(|| parent_session.as_ref().and_then(|p| p.model_id.as_deref()));

    // 解析生图模型 (Provider, ModelName)：
    // 1. 若工具参数显式指定了 model，优先在所有厂商中匹配
    // 2. 检查会话专属（或继承父会话）生图模型 image_provider_id / image_model_id
    // 3. 优先使用全局配置的生图模型 active_image_provider_id / active_image_model_id（界面上会话未指定时为“跟随全局”）
    // 4. 回落：检查会话专属（或继承父会话）主模型 provider_id / model_id 是否具备 image_gen 能力
    // 5. 回落：检查会话所属 provider 是否有其他模型具备 image_gen 能力
    // 6. 回落：查找全局任意厂商中具备 image_gen 能力的模型
    let resolved = if let Some(req_m) = requested_model {
        settings.providers.iter().find(|p| p.models.iter().any(|m| m == req_m))
            .map(|p| (p.clone(), req_m.to_string()))
            .or_else(|| crate::models::resolve_active_model(&settings).map(|(p, _)| (p.clone(), req_m.to_string())))
    } else if let (Some(pid), Some(mid)) = (effective_image_provider_id, effective_image_model_id) {
        settings.providers.iter().find(|p| &p.id == pid)
            .map(|p| (p.clone(), mid.to_string()))
    } else if let (Some(pid), Some(mid)) = (&settings.active_image_provider_id, &settings.active_image_model_id) {
        settings.providers.iter().find(|p| &p.id == pid && p.models.iter().any(|m| m == mid))
            .map(|p| (p.clone(), mid.clone()))
    } else if let (Some(pid), Some(mid)) = (effective_provider_id, effective_model_id) {
        if settings.has_capability(Some(pid), mid, "image_gen") {
            settings.providers.iter().find(|p| &p.id == pid).map(|p| (p.clone(), mid.to_string()))
        } else if let Some(p) = settings.providers.iter().find(|p| &p.id == pid) {
            p.models.iter().find(|m| settings.has_capability(Some(pid), m, "image_gen"))
                .map(|m| (p.clone(), m.clone()))
        } else {
            None
        }
    } else {
        None
    }
    .or_else(|| {
        crate::models::resolve_active_image_model(&settings)
            .map(|(p, m)| (p.clone(), m.to_string()))
    });

    let (provider, model_name) = resolved.ok_or_else(|| {
        "生图失败：当前未配置具备【图像生成 (image_gen)】能力的可行模型。\n\n请在【设置 -> 厂商配置】中为对应厂商模型勾选「生图」能力，或为生图协作者配置指定的生图模型。".to_string()
    })?;

    let prompt = args.get("prompt").and_then(|v| v.as_str()).ok_or("缺少 prompt 参数")?;
    let size = args.get("size").and_then(|v| v.as_str());
    let filename_arg = args.get("filename").and_then(|v| v.as_str());

    let protocol = provider.get_model_protocol(&model_name);
    let cfg = crate::llm::LlmCfg {
        base_url: provider.base_url.clone(),
        api_key: provider.api_key.clone(),
        model: model_name.clone(),
        protocol,
        reasoning_effort: None,
        proxy_url: ctx.proxy_url.clone(),
        session_id: None,
    };

    let img_bytes = crate::llm::generate_image_api(&cfg, prompt, size).await
        .map_err(|e| format!("生图失败 [厂商: {} / 模型: {}]: {e}", provider.name, model_name))?;

    let target_path = if let Some(fname) = filename_arg.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        if !ctx.workspace.as_os_str().is_empty() {
            let p = Path::new(fname);
            if p.is_absolute() {
                PathBuf::from(p)
            } else {
                ctx.workspace.join(p)
            }
        } else {
            let data_dir = state.data_dir.lock().unwrap().clone().unwrap_or_else(|| state.default_data_dir.clone());
            data_dir.join("generated_images").join(session_id).join(fname)
        }
    } else {
        let data_dir = state.data_dir.lock().unwrap().clone().unwrap_or_else(|| state.default_data_dir.clone());
        let folder = data_dir.join("generated_images").join(session_id);
        let ext = "png";
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let rand_id = &uuid::Uuid::new_v4().to_string()[..6];
        folder.join(format!("img_{timestamp}_{rand_id}.{ext}"))
    };

    if let Some(parent) = target_path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| format!("创建图片存储目录失败: {e}"))?;
    }

    if img_bytes.is_empty() {
        return Err("生图失败：接口返回的图片二进制数据为空 (0 字节)".to_string());
    }

    tokio::fs::write(&target_path, &img_bytes).await.map_err(|e| format!("写入图片文件失败: {e}"))?;

    let raw_target_str = target_path.to_string_lossy().to_string();
    let target_str = raw_target_str.replace('\\', "/");
    let size_display = size.unwrap_or("1024x1024");
    let kb = img_bytes.len() / 1024;

    Ok(format!(
        "🎨 图片生成成功！\n- 保存路径: `{target_str}`\n- 厂商/模型: `{}/{model_name}`\n- 提示词: {prompt}\n- 分辨率: {size_display}\n- 大小: {kb} KB\n\n![{prompt}]({target_str})",
        provider.name
    ))
}

pub(crate) fn resolve_vision_model_for_tool(
    args: &Value,
    ctx: &ToolCtx,
) -> Result<(crate::models::ProviderCfg, String), String> {
    let host = ctx.host.as_ref().ok_or("内部错误：缺少宿主上下文")?;
    let state = host.app.state::<crate::AppState>();
    let session_id = &host.session_id;

    let session = {
        let db = state.db.lock().unwrap();
        crate::store::get_session(&db, session_id)?
            .ok_or_else(|| format!("会话不存在: {session_id}"))?
    };

    let settings = {
        let db = state.db.lock().unwrap();
        let master = state.master_key.lock().unwrap();
        crate::store::get_settings_with_secrets(&db, &master)?
    };

    let requested_model = args.get("model").and_then(|v| v.as_str()).map(|s| s.trim()).filter(|s| !s.is_empty());

    let resolved = if let Some(req_m) = requested_model {
        settings.providers.iter().find(|p| p.models.iter().any(|m| m == req_m))
            .map(|p| (p.clone(), req_m.to_string()))
            .or_else(|| crate::models::resolve_active_model(&settings).map(|(p, _)| (p.clone(), req_m.to_string())))
    } else if let (Some(pid), Some(mid)) = (&session.vision_provider_id, &session.vision_model_id) {
        settings.providers.iter().find(|p| &p.id == pid)
            .map(|p| (p.clone(), mid.clone()))
    } else if let (Some(pid), Some(mid)) = (&settings.active_vision_provider_id, &settings.active_vision_model_id) {
        settings.providers.iter().find(|p| &p.id == pid && p.models.iter().any(|m| m == mid))
            .map(|p| (p.clone(), mid.clone()))
    } else if let (Some(pid), Some(mid)) = (&session.provider_id, &session.model_id) {
        if settings.has_capability(Some(pid), mid, "vision") {
            settings.providers.iter().find(|p| &p.id == pid).map(|p| (p.clone(), mid.clone()))
        } else if let Some(p) = settings.providers.iter().find(|p| &p.id == pid) {
            p.models.iter().find(|m| settings.has_capability(Some(pid), m, "vision"))
                .map(|m| (p.clone(), m.clone()))
        } else {
            None
        }
    } else {
        None
    }
    .or_else(|| {
        crate::models::resolve_active_vision_model(&settings)
            .map(|(p, m)| (p.clone(), m.to_string()))
    })
    .or_else(|| {
        crate::models::resolve_active_model(&settings)
            .filter(|(p, m)| settings.has_capability(Some(&p.id), m, "vision"))
            .map(|(p, m)| (p.clone(), m.to_string()))
    });

    resolved.ok_or_else(|| {
        "识图失败：当前未配置具备【视觉感知 (vision)】能力的可行模型。\n\n请在【设置 -> 厂商配置】中为对应厂商模型勾选「视觉」能力，或在会话设置中配置视觉专属模型。".to_string()
    })
}

pub(crate) async fn recognize_image_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let host = ctx.host.as_ref().ok_or("内部错误：缺少宿主上下文")?;
    let state = host.app.state::<crate::AppState>();
    let session_id = &host.session_id;

    let raw_path = args.get("path").and_then(|v| v.as_str()).map(|s| s.trim()).filter(|s| !s.is_empty())
        .ok_or("缺少 path 参数：请提供要识别的本地图片路径")?;
    let prompt_arg = args.get("prompt").and_then(|v| v.as_str()).map(|s| s.trim()).filter(|s| !s.is_empty());
    let prompt = prompt_arg.unwrap_or("请详细描述并分析这张图片的内容，包括画面主体、文字内容、艺术风格、色彩与关键细节特征。");

    // 1. 寻找图片文件的真实物理路径（支持绝对路径、工作区相对路径、generated_images 相对路径）
    let file_path = Path::new(raw_path);
    let target_path = if file_path.is_absolute() && file_path.exists() {
        file_path.to_path_buf()
    } else {
        let in_workspace = if !ctx.workspace.as_os_str().is_empty() {
            let p = ctx.workspace.join(file_path);
            if p.exists() { Some(p) } else { None }
        } else {
            None
        };

        if let Some(p) = in_workspace {
            p
        } else {
            let data_dir = state.data_dir.lock().unwrap().clone().unwrap_or_else(|| state.default_data_dir.clone());
            let in_session_artifacts = data_dir.join("sessions").join(session_id).join("artifacts").join(file_path);
            let in_session_gen = data_dir.join("generated_images").join(session_id).join(file_path);
            let in_gen_root = data_dir.join(file_path);
            if in_session_artifacts.exists() {
                in_session_artifacts
            } else if in_session_gen.exists() {
                in_session_gen
            } else if in_gen_root.exists() {
                in_gen_root
            } else if file_path.exists() {
                file_path.to_path_buf()
            } else {
                return Err(format!("图片文件不存在: `{raw_path}`。请检查路径是否正确（支持绝对路径或工作区相对路径）。"));
            }
        }
    };

    // 2. 检查文件大小（限制 20MB）
    let meta = tokio::fs::metadata(&target_path).await
        .map_err(|e| format!("获取图片信息失败: {e}"))?;
    if meta.len() > 20 * 1024 * 1024 {
        return Err("图片文件体积超过 20MB 上限，暂不支持处理过大文件".to_string());
    }

    let bytes = tokio::fs::read(&target_path).await
        .map_err(|e| format!("读取图片文件失败: {e}"))?;
    if bytes.is_empty() {
        return Err("图片文件内容为空 (0 字节)".to_string());
    }

    // 3. 智能推断 MIME 类型
    let ext = target_path.extension().and_then(|e| e.to_str()).map(|s| s.to_lowercase()).unwrap_or_default();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        _ => {
            if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
                "image/png"
            } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
                "image/jpeg"
            } else if bytes.starts_with(b"GIF8") {
                "image/gif"
            } else if bytes.starts_with(b"RIFF") && bytes.len() > 12 && &bytes[8..12] == b"WEBP" {
                "image/webp"
            } else {
                "image/png"
            }
        }
    };

    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let data_url = format!("data:{mime};base64,{b64}");
    let target_display_path = target_path.to_string_lossy().replace('\\', "/");
    let kb = bytes.len() / 1024;



    // 4. 智能自适应判定：若当前主会话模型自身已原生具备视觉能力 (vision)，且未显式指定委托子模型，
    // 则直接将高清原图注入多模态上下文，免去冗余的外部二次调用与信息折损。
    let requested_model = args.get("model").and_then(|v| v.as_str()).map(|s| s.trim()).filter(|s| !s.is_empty());
    let has_native_vision = if requested_model.is_none() {
        let session = {
            let db = state.db.lock().unwrap();
            crate::store::get_session(&db, session_id)?
                .ok_or_else(|| format!("会话不存在: {session_id}"))?
        };
        let settings = {
            let db = state.db.lock().unwrap();
            let master = state.master_key.lock().unwrap();
            crate::store::get_settings_with_secrets(&db, &master)?
        };
        let (pid, mid) = if let (Some(pid), Some(mid)) = (&session.provider_id, &session.model_id) {
            (Some(pid.clone()), Some(mid.clone()))
        } else if let Some((p, m)) = crate::models::resolve_active_model(&settings) {
            (Some(p.id.clone()), Some(m.to_string()))
        } else {
            (None, None)
        };
        if let (Some(ref p), Some(ref m)) = (pid, mid) {
            settings.has_capability(Some(p), m, "vision")
        } else {
            false
        }
    } else {
        false
    };

    if has_native_vision {
        return Ok(format!(
            "👁️ [图像已直接载入并注入多模态视觉上下文]\n- 目标文件: `{target_display_path}`\n- 图像类型: {mime} ({kb} KB)\n- 分析诉求: {prompt}\n\n<!-- MULTIMODAL_IMAGE_DIRECT:{target_display_path} -->\n系统已将该本地图片的高清原图直接注入到当前对话上下文中。作为原生多模态模型，你拥有对该图像的第一视觉直观感知，请直接结合任务要求进行精准分析与推理。"
        ));
    }

    // 5. 纯文本模型或指定子模型降级分支：解析生效视觉模型并执行二次调用
    let (provider, model_name) = resolve_vision_model_for_tool(args, ctx)?;

    // 5. 构建多模态请求并调用 LLM
    let protocol = provider.get_model_protocol(&model_name);
    let cfg = crate::llm::LlmCfg {
        base_url: provider.base_url.clone(),
        api_key: provider.api_key.clone(),
        model: model_name.clone(),
        protocol,
        reasoning_effort: None,
        proxy_url: ctx.proxy_url.clone(),
        session_id: Some(session_id.to_string()),
    };

    let messages = vec![
        json!({
            "role": "user",
            "content": [
                {
                    "type": "text",
                    "text": prompt
                },
                {
                    "type": "image_url",
                    "image_url": {
                        "url": data_url
                    }
                }
            ]
        })
    ];

    let res = crate::llm::chat_stream(&cfg, &messages, &[], |_| {}, |_| {}).await
        .map_err(|e| format!("识图调用失败 [厂商: {} / 模型: {}]: {e}", provider.name, model_name))?;

    let target_display_path = target_path.to_string_lossy().replace('\\', "/");
    let kb = bytes.len() / 1024;

    Ok(format!(
        "👁️ 图像识别完成！\n- 目标图片: `{target_display_path}`\n- 厂商/模型: `{}/{model_name}`\n- 大小: {kb} KB\n- 分析要求: {prompt}\n\n### 识别与分析结果：\n{}",
        provider.name,
        res.content.trim()
    ))
}

pub(crate) fn resolve_decision_route_for_tool(
    args: &Value,
    ctx: &ToolCtx,
) -> Result<crate::models::DecisionRoute, String> {
    let host = ctx.host.as_ref().ok_or("内部错误：缺少宿主上下文")?;
    let state = host.app.state::<crate::AppState>();
    let session_id = &host.session_id;

    let session = {
        let db = state.db.lock().unwrap();
        crate::store::get_session(&db, session_id)?
            .ok_or_else(|| format!("会话不存在: {session_id}"))?
    };

    let parent_session = if let Some(ref pid) = session.parent_session_id {
        let db = state.db.lock().unwrap();
        crate::store::get_session(&db, pid).ok().flatten()
    } else {
        None
    };

    let settings = {
        let db = state.db.lock().unwrap();
        let master = state.master_key.lock().unwrap();
        crate::store::get_settings_with_secrets(&db, &master)?
    };

    let requested_model = args.get("model").and_then(|v| v.as_str()).map(|s| s.trim()).filter(|s| !s.is_empty());

    crate::models::resolve_decision_route(Some(&session), parent_session.as_ref(), &settings, requested_model)
}

pub(crate) async fn judge_decision_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let route = resolve_decision_route_for_tool(args, ctx)?;
    let state = args.get("state")
        .or_else(|| args.get("statement"))
        .or_else(|| args.get("understanding"))
        .or_else(|| args.get("content"))
        .and_then(|v| v.as_str())
        .ok_or("缺少 state 参数（待判定的命题或上下文）")?;
    let instruction_raw = args.get("instruction")
        .or_else(|| args.get("question"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let default_instruction = "请对所给命题或业务意图陈述进行严格的二元明确性与可行性判定。若目标清晰、合理可行且无歧义，判定为 True；若存在重大缺陷或歧义，判定为 False。";
    let instruction = if !instruction_raw.is_empty() {
        instruction_raw
    } else {
        default_instruction
    };

    let res = crate::jev::execute_decision_pipeline(
        &route,
        "judge",
        state,
        instruction,
        None,
        None,
        ctx.proxy_url.clone(),
    )
    .await
    .map_err(|e| format!("决策执行失败 [厂商: {} / 模型: {}]: {e}", route.primary_provider.name, route.primary_model))?;

    let mut val = serde_json::to_value(&res).map_err(|e| e.to_string())?;
    if let Value::Object(ref mut map) = val {
        map.insert("provider".into(), Value::String(route.primary_provider.name.clone()));
        map.insert("providerId".into(), Value::String(route.primary_provider.id.clone()));
        map.insert("model".into(), Value::String(res.model.clone()));
        map.insert("state".into(), Value::String(state.to_string()));
        map.insert("instruction".into(), Value::String(instruction.to_string()));
    }

    Ok(serde_json::to_string_pretty(&val).unwrap_or_else(|_| "{}".to_string()))
}

pub(crate) async fn choice_decision_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let route = resolve_decision_route_for_tool(args, ctx)?;
    let state = args.get("state")
        .or_else(|| args.get("understanding"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let instruction = args.get("instruction")
        .or_else(|| args.get("question"))
        .and_then(|v| v.as_str())
        .unwrap_or("请根据业务上下文与候选选项，裁决选定最符合要求的目标分支。")
        .trim();
    let options = args.get("options")
        .or_else(|| args.get("choices"))
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect::<Vec<_>>())
        .ok_or("缺少 options 参数或选项格式不正确")?;

    let res = crate::jev::execute_decision_pipeline(
        &route,
        "choice",
        state,
        instruction,
        Some(options.clone()),
        None,
        ctx.proxy_url.clone(),
    )
    .await
    .map_err(|e| format!("决策执行失败 [厂商: {} / 模型: {}]: {e}", route.primary_provider.name, route.primary_model))?;

    let mut val = serde_json::to_value(&res).map_err(|e| e.to_string())?;
    if let Value::Object(ref mut map) = val {
        map.insert("provider".into(), Value::String(route.primary_provider.name.clone()));
        map.insert("providerId".into(), Value::String(route.primary_provider.id.clone()));
        map.insert("model".into(), Value::String(res.model.clone()));
        map.insert("options".into(), serde_json::json!(options));
        map.insert("state".into(), Value::String(state.to_string()));
        map.insert("instruction".into(), Value::String(instruction.to_string()));
    }

    Ok(serde_json::to_string_pretty(&val).unwrap_or_else(|_| "{}".to_string()))
}

pub(crate) async fn score_decision_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let route = resolve_decision_route_for_tool(args, ctx)?;
    let understanding = args.get("understanding")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let plan = args.get("plan")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let state_raw = args.get("state")
        .or_else(|| args.get("context"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();

    let combined_state = if !understanding.is_empty() || !plan.is_empty() {
        let mut s = String::new();
        if !understanding.is_empty() {
            s.push_str(&format!("【核心意图理解】:
{}

", understanding));
        }
        if !plan.is_empty() {
            s.push_str(&format!("【执行动作规划】:
{}

", plan));
        }
        if !state_raw.is_empty() {
            s.push_str(&format!("【上下文背景】:
{}", state_raw));
        }
        s
    } else if !state_raw.is_empty() {
        state_raw.to_string()
    } else {
        return Err("缺少待评分内容，请提供 plan / understanding 或 state 参数".to_string());
    };

    let instruction_raw = args.get("instruction")
        .or_else(|| args.get("question"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let default_instruction = "请对所提供的任务理解与执行规划进行全面严密的综合评分。打分区间 0 到 100（以 0.0 到 1.0 表示）。要求针对意图准确性、步骤可行性与风险完备性综合评估，并在 reason 中阐述评分理由。";
    let instruction = if !instruction_raw.is_empty() {
        instruction_raw
    } else {
        default_instruction
    };

    let rubric = args.get("rubric")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect::<Vec<_>>());

    let res = crate::jev::execute_decision_pipeline(
        &route,
        "score",
        &combined_state,
        instruction,
        None,
        rubric,
        ctx.proxy_url.clone(),
    )
    .await
    .map_err(|e| format!("决策执行失败 [厂商: {} / 模型: {}]: {e}", route.primary_provider.name, route.primary_model))?;

    let score_val = res.score.unwrap_or(0.0);
    let score_pct = (score_val * 100.0).round() as u32;

    let mut val = serde_json::to_value(&res).map_err(|e| e.to_string())?;
    if let Value::Object(ref mut map) = val {
        map.insert("provider".into(), Value::String(route.primary_provider.name.clone()));
        map.insert("providerId".into(), Value::String(route.primary_provider.id.clone()));
        map.insert("model".into(), Value::String(res.model.clone()));
        if !understanding.is_empty() {
            map.insert("understanding".into(), Value::String(understanding.to_string()));
        }
        if !plan.is_empty() {
            map.insert("plan".into(), Value::String(plan.to_string()));
        }
        map.insert("scorePercent".into(), serde_json::json!(score_pct));
        map.insert("instruction".into(), Value::String(instruction.to_string()));
    }

    Ok(serde_json::to_string_pretty(&val).unwrap_or_else(|_| "{}".to_string()))
}
