use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::llm::{self, LlmCfg};
use crate::store;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

/// 记忆元数据（用于 digests 碎记的 Frontmatter）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DigestMeta {
    pub title: String,
    pub category: String,
    pub updated_at: String,
    pub access_count: u32,
    pub importance: String, // "high" | "normal" | "low"
}

impl Default for DigestMeta {
    fn default() -> Self {
        Self {
            title: String::new(),
            category: "digest".into(),
            updated_at: Utc::now().to_rfc3339(),
            access_count: 1,
            importance: "normal".into(),
        }
    }
}

/// 记忆根目录：<workspace>/.harness/memory
pub fn memory_dir(workspace: &Path) -> PathBuf {
    workspace.join(".harness").join("memory")
}

/// 主题碎记目录：<workspace>/.harness/memory/digests
pub fn digests_dir(workspace: &Path) -> PathBuf {
    memory_dir(workspace).join("digests")
}

/// 确保工作区记忆目录与基础文件模板存在
pub fn ensure_memory_dir(workspace: &Path) -> Result<(), String> {
    if workspace.as_os_str().is_empty() {
        return Ok(());
    }
    let mem = memory_dir(workspace);
    let dig = digests_dir(workspace);
    fs::create_dir_all(&dig).map_err(|e| format!("创建记忆目录失败: {e}"))?;

    let profile_path = mem.join("profile.md");
    if !profile_path.exists() {
        let initial_profile = r#"# 项目技术大盘与架构档案 (Project Profile)
> 本档案由 Agent 自动维护沉淀，记录当前项目的核心技术栈、架构分层与依赖约定。

## 核心框架与构建体系
- （待提炼沉淀：Agent 在执行技术栈分析或读取配置后将自动更新此档案）
"#;
        let _ = fs::write(&profile_path, initial_profile);
    }

    let conventions_path = mem.join("conventions.md");
    if !conventions_path.exists() {
        let initial_conventions = r#"# 项目工程规范与避坑约定 (Project Conventions)
> 本文档记录本项目的特定编码规范、中文编码要求(如 UTF-8)、环境与避坑注意事项。
"#;
        let _ = fs::write(&conventions_path, initial_conventions);
    }

    Ok(())
}

/// 解析 Frontmatter 与正文
pub fn parse_frontmatter(content: &str) -> (DigestMeta, String) {
    let mut meta = DigestMeta::default();
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return (meta, content.to_string());
    }

    let rest = &trimmed[3..];
    if let Some(end_idx) = rest.find("\n---") {
        let yaml_str = &rest[..end_idx];
        let body = rest[end_idx + 4..].trim_start_matches('\r').trim_start_matches('\n');

        for line in yaml_str.lines() {
            let line = line.trim();
            if let Some(v) = line.strip_prefix("title:") {
                meta.title = v.trim().trim_matches('"').trim_matches('\'').to_string();
            } else if let Some(v) = line.strip_prefix("category:") {
                meta.category = v.trim().trim_matches('"').trim_matches('\'').to_string();
            } else if let Some(v) = line.strip_prefix("updated_at:") {
                meta.updated_at = v.trim().trim_matches('"').trim_matches('\'').to_string();
            } else if let Some(v) = line.strip_prefix("access_count:") {
                if let Ok(c) = v.trim().parse::<u32>() {
                    meta.access_count = c;
                }
            } else if let Some(v) = line.strip_prefix("importance:") {
                meta.importance = v.trim().trim_matches('"').trim_matches('\'').to_string();
            }
        }
        (meta, body.to_string())
    } else {
        (meta, content.to_string())
    }
}

/// 带 Frontmatter 格式化
pub fn format_with_frontmatter(meta: &DigestMeta, body: &str) -> String {
    format!(
        "---\ntitle: \"{}\"\ncategory: \"{}\"\nupdated_at: \"{}\"\naccess_count: {}\nimportance: \"{}\"\n---\n\n{}",
        meta.title.replace('"', "\\\""),
        meta.category,
        meta.updated_at,
        meta.access_count,
        meta.importance,
        body.trim()
    )
}

/// 清洗文件名 Slug
pub fn sanitize_slug(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = s.trim_matches('-');
    if trimmed.is_empty() {
        "memory-item".into()
    } else {
        trimmed.to_string()
    }
}

/// 将项目记忆格式化输出，供注入 System Prompt（无 IO 错误，容错处理）
pub fn load_project_memory(workspace: &Path) -> Option<String> {
    if workspace.as_os_str().is_empty() {
        return None;
    }
    let mem = memory_dir(workspace);
    if !mem.exists() {
        return None;
    }

    let mut parts = Vec::new();

    // 1. 读取 profile.md
    let profile_path = mem.join("profile.md");
    if let Ok(c) = fs::read_to_string(&profile_path) {
        let trimmed = c.trim();
        // 排除仅有模板初始文字的情况
        if !trimmed.is_empty() && !trimmed.contains("（待提炼沉淀：") {
            parts.push(format!("### 项目技术大盘与架构档案 (profile.md)\n{trimmed}"));
        }
    }

    // 2. 读取 conventions.md
    let conv_path = mem.join("conventions.md");
    if let Ok(c) = fs::read_to_string(&conv_path) {
        let trimmed = c.trim();
        let lines: Vec<_> = trimmed.lines().filter(|l| !l.trim().is_empty()).collect();
        if lines.len() > 2 {
            parts.push(format!("### 常用工程规范与避坑约定 (conventions.md)\n{trimmed}"));
        }
    }

    // 3. 读取最近高价值的主题碎记 digests（最多提取前 3 篇）
    let dig = digests_dir(workspace);
    if let Ok(entries) = fs::read_dir(&dig) {
        let mut digests = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("md") {
                if let Ok(content) = fs::read_to_string(&path) {
                    let (meta, body) = parse_frontmatter(&content);
                    digests.push((meta, body));
                }
            }
        }
        // 按更新时间降序
        digests.sort_by(|a, b| b.0.updated_at.cmp(&a.0.updated_at));
        let top_digests: Vec<_> = digests.into_iter().take(3).collect();
        if !top_digests.is_empty() {
            let mut s = String::from("### 近期边读边记重点沉淀 (digests)\n");
            for (meta, body) in top_digests {
                let first_line = body.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
                s.push_str(&format!("- **{}** ({}): {}\n", meta.title, meta.category, first_line));
            }
            parts.push(s.trim().to_string());
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

/// 沉淀/记录记忆
pub fn record_memory(
    workspace: &Path,
    category: &str,
    title: &str,
    content: &str,
) -> Result<String, String> {
    ensure_memory_dir(workspace)?;
    let mem = memory_dir(workspace);
    let title = title.trim();
    let content = content.trim();

    match category {
        "profile" | "tech_stack" => {
            let path = mem.join("profile.md");
            let new_content = if content.starts_with('#') {
                content.to_string()
            } else {
                format!("# 项目技术大盘与架构档案 (Project Profile)\n\n{}\n", content)
            };
            fs::write(&path, new_content).map_err(|e| format!("写入 profile.md 失败: {e}"))?;
            Ok("已成功将最新技术栈与架构档案持久化至 `.harness/memory/profile.md`，后续所有新会话均会自动秒级召回。".into())
        }
        "convention" | "notes" => {
            let path = mem.join("conventions.md");
            let mut existing = fs::read_to_string(&path).unwrap_or_default();
            let timestamp = Utc::now().format("%Y-%m-%d").to_string();
            existing.push_str(&format!("\n\n### {} ({})\n{}\n", title, timestamp, content));
            fs::write(&path, existing).map_err(|e| format!("写入 conventions.md 失败: {e}"))?;
            Ok(format!("已成功将规范约定【{}】追加至 `.harness/memory/conventions.md`。", title))
        }
        _ => {
            // 默认存入 digests/*.md
            let slug = sanitize_slug(title);
            let filename = format!("{}.md", slug);
            let path = digests_dir(workspace).join(&filename);

            let mut meta = if path.exists() {
                let old_str = fs::read_to_string(&path).unwrap_or_default();
                let (old_meta, _) = parse_frontmatter(&old_str);
                DigestMeta {
                    title: title.to_string(),
                    category: category.to_string(),
                    updated_at: Utc::now().to_rfc3339(),
                    access_count: old_meta.access_count + 1,
                    importance: old_meta.importance,
                }
            } else {
                DigestMeta {
                    title: title.to_string(),
                    category: category.to_string(),
                    updated_at: Utc::now().to_rfc3339(),
                    access_count: 1,
                    importance: "normal".into(),
                }
            };

            // 如果内容中特别重要则提级
            if content.contains("CRITICAL") || content.contains("重要") || content.contains("必须") {
                meta.importance = "high".into();
            }

            let full = format_with_frontmatter(&meta, content);
            fs::write(&path, full).map_err(|e| format!("写入碎记失败: {e}"))?;
            Ok(format!("已成功将知识碎片【{}】固化至 `.harness/memory/digests/{}`。", title, filename))
        }
    }
}

/// 读取记忆内容
pub fn read_memory(workspace: &Path, topic: Option<&str>) -> Result<String, String> {
    ensure_memory_dir(workspace)?;
    let mem = memory_dir(workspace);

    match topic.map(|s| s.trim()) {
        Some("profile") | Some("tech_stack") => {
            let path = mem.join("profile.md");
            fs::read_to_string(&path).map_err(|e| format!("读取 profile.md 失败: {e}"))
        }
        Some("conventions") | Some("convention") => {
            let path = mem.join("conventions.md");
            fs::read_to_string(&path).map_err(|e| format!("读取 conventions.md 失败: {e}"))
        }
        Some(t) if !t.is_empty() => {
            let slug = sanitize_slug(t);
            let path = digests_dir(workspace).join(format!("{}.md", slug));
            if path.exists() {
                fs::read_to_string(&path).map_err(|e| format!("读取碎记失败: {e}"))
            } else {
                let mut existing = Vec::new();
                if let Ok(entries) = fs::read_dir(digests_dir(workspace)) {
                    for entry in entries.flatten() {
                        let ep = entry.path();
                        if ep.extension().and_then(|s| s.to_str()) == Some("md") {
                            if let Some(stem) = ep.file_stem().and_then(|s| s.to_str()) {
                                existing.push(stem.to_string());
                            }
                        }
                    }
                }
                let existing_hint = if existing.is_empty() {
                    "（当前暂无任何已保存的主题碎记，可用 topic: null 查看总览或 record_memory 沉淀新记忆）".to_string()
                } else {
                    format!("。当前已有的主题碎记包括: [{}]。可用 topic: null 查看总览", existing.join(", "))
                };
                Err(format!("未找到主题【{t}】对应的记忆碎记{existing_hint}"))
            }
        }
        _ => {
            // 列出全部概览清单
            let mut out = String::from("# 当前项目 Agent 专属知识记忆总览\n\n");
            let profile_path = mem.join("profile.md");
            if profile_path.exists() {
                out.push_str("- 📄 **profile.md** (技术大盘档案)\n");
            }
            let conv_path = mem.join("conventions.md");
            if conv_path.exists() {
                out.push_str("- 📋 **conventions.md** (工程规范与避坑约定)\n");
            }
            out.push_str("\n### 主题碎记列表 (digests):\n");
            let dig = digests_dir(workspace);
            let mut count = 0;
            if let Ok(entries) = fs::read_dir(&dig) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.extension().and_then(|s| s.to_str()) == Some("md") {
                        if let Ok(c) = fs::read_to_string(&p) {
                            let (meta, _) = parse_frontmatter(&c);
                            out.push_str(&format!(
                                "- `{}`: **{}** [类型: {}, 访问次数: {}, 重要度: {}]\n",
                                p.file_name().and_then(|f| f.to_str()).unwrap_or(""),
                                meta.title,
                                meta.category,
                                meta.access_count,
                                meta.importance
                            ));
                            count += 1;
                        }
                    }
                }
            }
            if count == 0 {
                out.push_str("(暂无主题碎记，可通过 `record_memory` 工具随时提炼沉淀)\n");
            }
            Ok(out)
        }
    }
}

/// 记忆过期与自动蒸馏淘汰：淘汰超期/低价值碎记，维持精炼知识库
pub fn prune_memory(workspace: &Path, max_digests: usize, max_age_days: i64) -> Result<usize, String> {
    let dig = digests_dir(workspace);
    if !dig.exists() {
        return Ok(0);
    }

    let entries = fs::read_dir(&dig).map_err(|e| format!("读取目录失败: {e}"))?;
    struct Item {
        path: PathBuf,
        meta: DigestMeta,
        age_days: i64,
    }

    let now = Utc::now();
    let mut items = Vec::new();

    for entry in entries.flatten() {
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) == Some("md") {
            if let Ok(content) = fs::read_to_string(&p) {
                let (meta, _) = parse_frontmatter(&content);
                let age = DateTime::parse_from_rfc3339(&meta.updated_at)
                    .map(|dt| (now - dt.with_timezone(&Utc)).num_days())
                    .unwrap_or(0);
                items.push(Item {
                    path: p,
                    meta,
                    age_days: age,
                });
            }
        }
    }

    let mut pruned_count = 0usize;

    // 1. 淘汰超过 max_age_days 且重要度为 low 的过期条目
    items.retain(|item| {
        if item.meta.importance == "low" && item.age_days > max_age_days {
            let _ = fs::remove_file(&item.path);
            pruned_count += 1;
            false
        } else {
            true
        }
    });

    // 2. 如果剩余条目依然大于 max_digests，按综合得分排序淘汰多余条目
    // 得分规则：high=100分, normal=50分; 每次访问加 5 分; 随着天数扣分
    if items.len() > max_digests {
        items.sort_by(|a, b| {
            let score_a = match a.meta.importance.as_str() {
                "high" => 100,
                _ => 50,
            } + (a.meta.access_count as i64 * 5) - a.age_days;

            let score_b = match b.meta.importance.as_str() {
                "high" => 100,
                _ => 50,
            } + (b.meta.access_count as i64 * 5) - b.age_days;

            score_b.cmp(&score_a) // 降序，得分高的在前面
        });

        // 丢弃末尾得分低的条目
        for excess in items.iter().skip(max_digests) {
            let _ = fs::remove_file(&excess.path);
            pruned_count += 1;
        }
    }

    Ok(pruned_count)
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AutoDistillOutput {
    pub should_record: bool,
    #[serde(default)]
    pub title: String,
    #[serde(default = "default_distill_category")]
    pub category: String,
    #[serde(default = "default_distill_importance")]
    pub importance: String,
    #[serde(default)]
    pub content: String,
}

fn default_distill_category() -> String {
    "digest".into()
}

fn default_distill_importance() -> String {
    "normal".into()
}

/// 去除模型可能包裹的 markdown 代码块标记
pub fn clean_json_text(text: &str) -> &str {
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

/// 对话结束后的自动主题提炼 (Auto-Distillation on Run Finish)
/// 异步非阻塞执行：
/// 1. 检查本轮会话是否包含有效的工作区与探索事实；
/// 2. 若本轮已主动调用过 record_memory(category="digest")，则跳过避免重复；
/// 3. 若为简单问答或无代码/文件探索，跳过避免垃圾碎记；
/// 4. 否则异步调用 LLM 提炼 1 篇针对性主题碎记，自动沉淀至 .harness/memory/digests/<slug>.md。
pub fn trigger_auto_distillation(
    app: AppHandle,
    session_id: String,
    run_id: String,
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
        if settings.disabled_sops.contains(&"memory_distill".to_string()) {
            return;
        }
        if session.workspace_path.is_empty() {
            return;
        }
        let ws_path = PathBuf::from(&session.workspace_path);
        if !ws_path.exists() {
            return;
        }

        let Some((pc, model)) = crate::models::resolve_active_model(&settings) else { return };
        let cfg = LlmCfg {
            base_url: pc.base_url.clone(),
            api_key: pc.api_key.clone(),
            model: model.to_string(),
            reasoning_effort: None,
        };

        let all_msgs = {
            let db = state.db.lock().unwrap();
            store::all_messages(&db, &session_id).unwrap_or_default()
        };

        let run_messages: Vec<_> = all_msgs
            .iter()
            .filter(|m| m.run_id.as_deref() == Some(&run_id))
            .cloned()
            .collect();
        let messages = if run_messages.is_empty() {
            all_msgs.into_iter().rev().take(10).rev().collect()
        } else {
            run_messages
        };

        if messages.is_empty() {
            return;
        }

        // 检查当前轮次是否已经主动调用过 record_memory(category="digest")
        let already_recorded_digest = messages.iter().any(|m| {
            m.tool_events.iter().any(|te| {
                if te.tool_name == "record_memory" {
                    let cat = te.params.get("category").and_then(|v| v.as_str()).unwrap_or("");
                    cat == "digest"
                } else {
                    false
                }
            })
        });
        if already_recorded_digest {
            return;
        }

        let mut touched_files = Vec::new();
        let mut tool_actions = Vec::new();
        let mut assistant_reply = String::new();
        let mut user_prompt = String::new();

        for m in &messages {
            if m.role == "user" && user_prompt.is_empty() {
                if let Some(c) = &m.content {
                    user_prompt = c.clone();
                }
            }
            if m.role == "assistant" {
                if let Some(c) = &m.content {
                    if !c.trim().is_empty() {
                        assistant_reply = c.clone();
                    }
                }
            }
            for te in &m.tool_events {
                if let Some(p) = te.params.get("path").and_then(|v| v.as_str()) {
                    touched_files.push(p.to_string());
                }
                tool_actions.push(format!("{}: {}", te.tool_name, te.status));
            }
        }

        // 判定是否有值得提炼的实体内容
        let has_substantial_action = !touched_files.is_empty() || tool_actions.len() >= 2 || assistant_reply.len() >= 120;
        if !has_substantial_action {
            return;
        }

        if user_prompt.trim().len() < 4 && touched_files.is_empty() {
            return;
        }

        let system_msg = json!({
            "role": "system",
            "content": r#"你是一个敏锐的项目认知与架构知识提炼器。你的任务是分析当前编程 Agent 刚刚完成的技术对话、代码探索与交付成果，提炼出 1 篇高价值的「主题碎记 (Digest)」。

提炼规范：
1. 聚焦于对话中涉及的具体功能模块、架构设计、业务链路、代码文件职责、配置要点或特定排错结论。
2. 如果本次对话是关于“项目技术栈全貌/大盘”，或者属于极简问答/纯闲聊无技术细节，请直接输出 {"shouldRecord": false}。
3. 若有值得保存的技术主题，请输出严格的 JSON 格式（不要输出任何额外的说明文字或 markdown 代码块）：
{
  "shouldRecord": true,
  "title": "简明主题标题（15字以内，如：用户鉴权拦截与Token流转、批量导出内存优化方案）",
  "category": "digest",
  "importance": "normal",
  "content": "结构清晰的 Markdown 知识正文，必须包含：\n### 1. 核心概述与背景\n### 2. 关键代码文件与入口\n### 3. 核心流转逻辑与实现细节\n### 4. 注意事项与避坑指引"
}"#
        });

        let truncated_reply = if assistant_reply.len() > 3000 {
            let mut end = 3000;
            while end > 0 && !assistant_reply.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}...\n(后续内容省略)", &assistant_reply[..end])
        } else {
            assistant_reply
        };

        let file_summary = if touched_files.is_empty() {
            "无".into()
        } else {
            touched_files.into_iter().take(10).collect::<Vec<_>>().join(", ")
        };

        let user_msg = json!({
            "role": "user",
            "content": format!(
                "工作区路径：{}\n用户提问需求：\n{}\n\n涉及关键文件：{}\n\nAgent 最终交付内容：\n{}",
                session.workspace_path,
                user_prompt,
                file_summary,
                truncated_reply
            )
        });

        let call = llm::chat_stream(&cfg, &[system_msg, user_msg], &[], |_| {}, |_| {}).await;
        if let Ok(res) = call {
            let cleaned = clean_json_text(&res.content);
            if let Ok(parsed) = serde_json::from_str::<AutoDistillOutput>(cleaned) {
                if parsed.should_record && !parsed.title.trim().is_empty() && !parsed.content.trim().is_empty() {
                    let _ = record_memory(&ws_path, "digest", &parsed.title, &parsed.content);
                    let _ = app.emit(
                        "memory:updated",
                        json!({
                            "sessionId": session_id,
                            "category": "digest",
                            "title": parsed.title,
                            "action": "auto_distilled"
                        }),
                    );

                    // 如果本次产出是技术栈大盘分析，且 profile.md 仍处于空模板状态，同步更新 profile.md
                    let is_tech_stack_topic = parsed.title.contains("技术栈")
                        || parsed.title.contains("架构")
                        || user_prompt.contains("技术栈")
                        || user_prompt.contains("架构");
                    if is_tech_stack_topic {
                        let prof_path = memory_dir(&ws_path).join("profile.md");
                        if let Ok(c) = fs::read_to_string(&prof_path) {
                            if c.contains("（待提炼沉淀：") {
                                let _ = record_memory(&ws_path, "profile", &parsed.title, &parsed.content);
                            }
                        }
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ensure_memory_dir_creates_structure() {
        let temp_dir = std::env::temp_dir().join(format!("harness_test_mem_{}", uuid::Uuid::new_v4()));
        let _ = fs::create_dir_all(&temp_dir);

        ensure_memory_dir(&temp_dir).unwrap();

        assert!(memory_dir(&temp_dir).exists());
        assert!(digests_dir(&temp_dir).exists());
        assert!(memory_dir(&temp_dir).join("profile.md").exists());
        assert!(memory_dir(&temp_dir).join("conventions.md").exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_record_and_load_memory() {
        let temp_dir = std::env::temp_dir().join(format!("harness_test_mem_{}", uuid::Uuid::new_v4()));
        let _ = fs::create_dir_all(&temp_dir);

        // 记录技术栈大盘
        let res1 = record_memory(
            &temp_dir,
            "tech_stack",
            "Spring Boot架构",
            "## 核心技术栈\n- Spring Boot 2.7.5\n- MySQL 8.0\n- Redis",
        )
        .unwrap();
        assert!(res1.contains("profile.md"));

        // 记录工程规范
        let res2 = record_memory(
            &temp_dir,
            "convention",
            "编码格式",
            "全项目源码与注释必须采用 UTF-8 编码。",
        )
        .unwrap();
        assert!(res2.contains("conventions.md"));

        // 记录主题碎记
        let res3 = record_memory(
            &temp_dir,
            "auth",
            "JWT鉴权拦截",
            "鉴权通过 JwtAuthInterceptor 实现，Token 放置在 Header Authorization 中。",
        )
        .unwrap();
        assert!(res3.contains("digests"));

        // 验证 load_project_memory 能成功提取并组合
        let loaded = load_project_memory(&temp_dir).unwrap();
        assert!(loaded.contains("Spring Boot 2.7.5"));
        assert!(loaded.contains("UTF-8"));
        assert!(loaded.contains("JWT鉴权拦截"));

        // 验证 read_memory 能读取 profile
        let read_prof = read_memory(&temp_dir, Some("profile")).unwrap();
        assert!(read_prof.contains("Spring Boot 2.7.5"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_memory_decay_and_pruning() {
        let temp_dir = std::env::temp_dir().join(format!("harness_test_mem_{}", uuid::Uuid::new_v4()));
        let _ = fs::create_dir_all(&temp_dir);

        // 写入 5 条碎记
        for i in 1..=5 {
            record_memory(
                &temp_dir,
                "digest",
                &format!("temp-item-{}", i),
                &format!("这是第 {} 个临时探索细节", i),
            )
            .unwrap();
        }

        // max_digests 设定为 3，应淘汰多余的 2 条
        let pruned = prune_memory(&temp_dir, 3, 30).unwrap();
        assert_eq!(pruned, 2);

        let entries = fs::read_dir(digests_dir(&temp_dir)).unwrap().count();
        assert_eq!(entries, 3);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_clean_json_text_and_parse_auto_distill() {
        let raw = r#"```json
{
  "shouldRecord": true,
  "title": "JWT鉴权与Token流转",
  "category": "digest",
  "importance": "high",
  "content": "核心概述：基于拦截器与ThreadLocal上下文实现。"
}
```"#;
        let cleaned = clean_json_text(raw);
        let parsed: AutoDistillOutput = serde_json::from_str(cleaned).unwrap();
        assert!(parsed.should_record);
        assert_eq!(parsed.title, "JWT鉴权与Token流转");
        assert_eq!(parsed.category, "digest");
        assert_eq!(parsed.importance, "high");
        assert!(parsed.content.contains("ThreadLocal"));

        let skip_raw = r#"{"shouldRecord": false}"#;
        let parsed_skip: AutoDistillOutput = serde_json::from_str(skip_raw).unwrap();
        assert!(!parsed_skip.should_record);
    }

    #[test]
    fn test_read_memory_hints() {
        let temp_dir = std::env::temp_dir().join(format!("harness_test_mem_hint_{}", uuid::Uuid::new_v4()));
        let _ = fs::create_dir_all(&temp_dir);

        // 尚未写入任何碎记时读取不存在的主题
        let err1 = read_memory(&temp_dir, Some("non_existent")).unwrap_err();
        assert!(err1.contains("暂无任何已保存的主题碎记"));

        // 写入一条碎记
        record_memory(&temp_dir, "database", "数据库连接池配置", "采用 HikariCP 配置").unwrap();

        // 再次读取不存在的主题，验证提示中包含已有的碎记
        let err2 = read_memory(&temp_dir, Some("non_existent")).unwrap_err();
        assert!(err2.contains("数据库连接池配置"));
        assert!(err2.contains("已有的主题碎记包括"));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
