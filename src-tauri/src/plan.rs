use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// 计划元数据（存储于 Markdown Frontmatter）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanMeta {
    pub id: String,
    pub title: String,
    pub status: String, // "drafting" | "in_progress" | "completed" | "suspended" | "archived"
    pub created_at: String,
    pub updated_at: String,
    pub version: u32,
    pub session_id: String,
}

impl Default for PlanMeta {
    fn default() -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            id: format!("plan-{}", Utc::now().format("%Y%m%d%H%M%S")),
            title: String::new(),
            status: "in_progress".into(),
            created_at: now.clone(),
            updated_at: now,
            version: 1,
            session_id: String::new(),
        }
    }
}

/// 计划执行清单步骤
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanStep {
    pub index: usize,
    pub content: String,
    pub status: String, // "pending" | "in_progress" | "done"
}

/// 计划列表概要（前端展示用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanSummary {
    pub id: String,
    pub title: String,
    pub status: String,
    pub version: u32,
    pub filename: String,
    pub created_at: String,
    pub updated_at: String,
    pub session_id: String,
    pub total_steps: usize,
    pub completed_steps: usize,
    pub is_active: bool,
}

/// 计划根目录：<workspace>/.harness/plans
pub fn plans_dir(workspace: &Path) -> PathBuf {
    workspace.join(".harness").join("plans")
}

/// 计划归档目录：<workspace>/.harness/plans/archive
pub fn archive_dir(workspace: &Path) -> PathBuf {
    plans_dir(workspace).join("archive")
}

/// 确保计划目录结构存在
pub fn ensure_plans_dir(workspace: &Path) -> Result<(), String> {
    if workspace.as_os_str().is_empty() {
        return Ok(());
    }
    let dir = plans_dir(workspace);
    let arch = archive_dir(workspace);
    fs::create_dir_all(&dir).map_err(|e| format!("创建计划目录失败: {e}"))?;
    fs::create_dir_all(&arch).map_err(|e| format!("创建计划归档目录失败: {e}"))?;
    Ok(())
}

/// 将标题清洗为合法的文件名 Slug
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
        "task-plan".into()
    } else {
        trimmed.to_string()
    }
}

/// 解析 Frontmatter 与 Markdown 正文
pub fn parse_frontmatter(content: &str) -> (PlanMeta, String) {
    let mut meta = PlanMeta::default();
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
            if let Some(v) = line.strip_prefix("id:") {
                meta.id = v.trim().trim_matches('"').trim_matches('\'').to_string();
            } else if let Some(v) = line.strip_prefix("title:") {
                meta.title = v.trim().trim_matches('"').trim_matches('\'').to_string();
            } else if let Some(v) = line.strip_prefix("status:") {
                meta.status = v.trim().trim_matches('"').trim_matches('\'').to_string();
            } else if let Some(v) = line.strip_prefix("created_at:") {
                meta.created_at = v.trim().trim_matches('"').trim_matches('\'').to_string();
            } else if let Some(v) = line.strip_prefix("updated_at:") {
                meta.updated_at = v.trim().trim_matches('"').trim_matches('\'').to_string();
            } else if let Some(v) = line.strip_prefix("version:") {
                if let Ok(num) = v.trim().parse::<u32>() {
                    meta.version = num;
                }
            } else if let Some(v) = line.strip_prefix("session_id:") {
                meta.session_id = v.trim().trim_matches('"').trim_matches('\'').to_string();
            }
        }
        (meta, body.to_string())
    } else {
        (meta, content.to_string())
    }
}

/// 格式化为带有 Frontmatter 的完整 Markdown
pub fn format_with_frontmatter(meta: &PlanMeta, body: &str) -> String {
    format!(
        "---\nid: \"{}\"\ntitle: \"{}\"\nstatus: \"{}\"\ncreated_at: \"{}\"\nupdated_at: \"{}\"\nversion: {}\nsession_id: \"{}\"\n---\n\n{}",
        meta.id.replace('"', "\\\""),
        meta.title.replace('"', "\\\""),
        meta.status,
        meta.created_at,
        meta.updated_at,
        meta.version,
        meta.session_id,
        body.trim()
    )
}

/// 从正文中提取步骤清单（- [x], - [/], - [ ]）
pub fn parse_steps_from_markdown(body: &str) -> Vec<PlanStep> {
    let mut steps = Vec::new();
    let mut idx = 1;

    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("- [x]") {
            steps.push(PlanStep {
                index: idx,
                content: rest.trim().to_string(),
                status: "done".into(),
            });
            idx += 1;
        } else if let Some(rest) = trimmed.strip_prefix("- [/]") {
            steps.push(PlanStep {
                index: idx,
                content: rest.trim().to_string(),
                status: "in_progress".into(),
            });
            idx += 1;
        } else if let Some(rest) = trimmed.strip_prefix("- [ ]") {
            steps.push(PlanStep {
                index: idx,
                content: rest.trim().to_string(),
                status: "pending".into(),
            });
            idx += 1;
        }
    }
    steps
}

/// 获取当前会话绑定的活动计划 ID
pub fn get_active_plan_id(conn: Option<&rusqlite::Connection>, session_id: &str) -> Option<String> {
    if let Some(db) = conn {
        if let Ok(Some(pid)) = crate::store::get_kv(db, session_id, "active_plan_id") {
            let pid = pid.trim();
            if !pid.is_empty() {
                return Some(pid.to_string());
            }
        }
    }
    None
}

/// 设置当前会话绑定的活动计划 ID
pub fn set_active_plan_id(
    conn: Option<&rusqlite::Connection>,
    session_id: &str,
    plan_id: Option<&str>,
) -> Result<(), String> {
    if let Some(db) = conn {
        let val = plan_id.unwrap_or("").trim();
        crate::store::set_kv(db, session_id, "active_plan_id", val)?;
    }
    Ok(())
}

/// 定位指定 plan_id 或处于活动状态的计划文件
pub fn find_plan_file(
    workspace: &Path,
    session_id: &str,
    plan_id: Option<&str>,
    conn: Option<&rusqlite::Connection>,
) -> Option<(PathBuf, PlanMeta, String)> {
    let pdir = plans_dir(workspace);
    if !pdir.exists() {
        return None;
    }

    let target_id = plan_id
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| get_active_plan_id(conn, session_id));

    let entries = fs::read_dir(&pdir).ok()?;
    let mut files = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("md") {
            if let Ok(content) = fs::read_to_string(&path) {
                let (meta, body) = parse_frontmatter(&content);
                files.push((path, meta, body));
            }
        }
    }

    // 1. 若指定了目标 ID / slug
    if let Some(ref tid) = target_id {
        for (path, meta, body) in &files {
            if meta.id == *tid
                || path.file_stem().and_then(|s| s.to_str()) == Some(tid.as_str())
            {
                return Some((path.clone(), meta.clone(), body.clone()));
            }
        }
        // 如果调用方明确指定了 plan_id，但未匹配到，直接返回 None，不回退到其他计划
        if plan_id.map(|s| !s.trim().is_empty()).unwrap_or(false) {
            return None;
        }
    }

    // 2. 若未指定或没精确匹配，且指定了 session_id，则只查找当前会话下 status == "in_progress" 的最新计划
    if !session_id.is_empty() {
        let mut session_active = files
            .iter()
            .filter(|(_, meta, _)| meta.session_id == session_id && meta.status == "in_progress")
            .cloned()
            .collect::<Vec<_>>();

        session_active.sort_by(|a, b| b.1.updated_at.cmp(&a.1.updated_at));

        if let Some(active) = session_active.into_iter().next() {
            return Some(active);
        }

        // 2.1 若当前会话未找到进行中方案，且该会话拥有父会话（为子任务或协作者），穿透查找父会话绑定的进行中方案
        if let Some(c) = conn {
            if let Ok(Some(sess)) = crate::store::get_session(c, session_id) {
                if let Some(ref pid) = sess.parent_session_id {
                    let parent_target_id = get_active_plan_id(conn, pid);
                    if let Some(ref ptid) = parent_target_id {
                        for (path, meta, body) in &files {
                            if meta.id == *ptid
                                || path.file_stem().and_then(|s| s.to_str()) == Some(ptid.as_str())
                            {
                                return Some((path.clone(), meta.clone(), body.clone()));
                            }
                        }
                    }
                    let mut parent_active = files
                        .iter()
                        .filter(|(_, meta, _)| meta.session_id == *pid && meta.status == "in_progress")
                        .cloned()
                        .collect::<Vec<_>>();
                    parent_active.sort_by(|a, b| b.1.updated_at.cmp(&a.1.updated_at));
                    if let Some(active) = parent_active.into_iter().next() {
                        return Some(active);
                    }
                }
            }
        }

        // 当前会话指定了 session_id 但未找到属于该会话或其父会话的进行中方案，必须返回 None，严格隔离防止跨会话越权串扰！
        return None;
    }

    // 3. 兜底容错：仅当未提供 session_id（session_id 为空）且未提供 plan_id 时，才取该工作区下最新更新的进行中方案
    let mut latest_in_progress = files
        .into_iter()
        .filter(|(_, meta, _)| meta.status == "in_progress")
        .collect::<Vec<_>>();
    latest_in_progress.sort_by(|a, b| b.1.updated_at.cmp(&a.1.updated_at));
    latest_in_progress.into_iter().next()
}

fn available_plans_hint(
    workspace: &Path,
    session_id: Option<&str>,
    conn: Option<&rusqlite::Connection>,
) -> String {
    let available = list_plans(workspace, session_id, true, conn)
        .unwrap_or_default()
        .into_iter()
        .map(|p| format!("`{}` ({})", p.id, p.title))
        .collect::<Vec<_>>();
    if available.is_empty() {
        "（当前工作区暂无任何计划文档，可使用 create_plan 创建）".to_string()
    } else {
        format!("。当前已有的计划文档包括: [{}]。可用 list_plans 查看详情", available.join(", "))
    }
}

/// 同步计划步骤至 session_kv 中的 "todos"
pub fn sync_steps_to_todos(
    conn: Option<&rusqlite::Connection>,
    session_id: &str,
    steps: &[PlanStep],
) {
    if let Some(db) = conn {
        if steps.is_empty() {
            return;
        }
        let todos_val: Vec<Value> = steps
            .iter()
            .map(|s| {
                json!({
                    "content": s.content,
                    "status": s.status
                })
            })
            .collect();
        let payload = json!({ "todos": todos_val });
        let _ = crate::store::set_kv(db, session_id, "todos", &payload.to_string());
    }
}

/// 创建新计划
pub fn create_plan(
    workspace: &Path,
    session_id: &str,
    title: &str,
    goals: &str,
    architecture: &str,
    files: &[String],
    steps: &[String],
    verification: Option<&str>,
    conn: Option<&rusqlite::Connection>,
) -> Result<String, String> {
    ensure_plans_dir(workspace)?;
    let title = title.trim();
    if title.is_empty() {
        return Err("计划标题不能为空".into());
    }

    // 若当前会话已有处于 in_progress 的计划，先将其置为 suspended（挂起），实现多任务隔离
    if let Some((old_path, mut old_meta, old_body)) =
        find_plan_file(workspace, session_id, None, conn)
    {
        if old_meta.status == "in_progress" {
            old_meta.status = "suspended".into();
            old_meta.updated_at = Utc::now().to_rfc3339();
            let updated = format_with_frontmatter(&old_meta, &old_body);
            let _ = fs::write(&old_path, updated);
        }
    }

    let date_str = Utc::now().format("%Y%m%d").to_string();
    let time_str = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let slug = sanitize_slug(title);
    let filename = format!("{}-{}.md", date_str, slug);
    let file_path = plans_dir(workspace).join(&filename);

    let plan_id = format!("plan-{}-{}", date_str, slug);
    let meta = PlanMeta {
        id: plan_id.clone(),
        title: title.to_string(),
        status: "in_progress".into(),
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
        version: 1,
        session_id: session_id.to_string(),
    };

    let files_list = if files.is_empty() {
        "- （待进一步探索确定）".to_string()
    } else {
        files
            .iter()
            .map(|f| format!("- {}", f.trim()))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let steps_list = if steps.is_empty() {
        "- [ ] 步骤 1：详细梳理并推进核心任务".to_string()
    } else {
        steps
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let s_trimmed = s.trim().trim_start_matches("- [ ]").trim_start_matches("- [x]").trim_start_matches("- [/]").trim();
                format!("- [ ] 步骤 {}：{}", i + 1, s_trimmed)
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let verify_text = verification
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .unwrap_or("运行项目自检测试与自动化验证脚本");

    let body = format!(
        r#"# 任务方案：{title}

## 一、需求背景与目标 (Requirements & Goals)
{goals}

## 二、架构设计与技术方案 (Architecture & Design)
{architecture}

## 三、涉及文件与影响范围 (Scope & Affected Files)
{files_list}

## 四、分步执行清单 (Execution Checklist)
{steps_list}

## 五、验证与验收策略 (Verification Strategy)
- {verify_text}

## 六、需求变更历史 (Revision History)
- **v1 ({time_str})**：初始创建方案，明确核心目标与任务分解。
"#,
        title = title,
        goals = goals.trim(),
        architecture = architecture.trim(),
        files_list = files_list,
        steps_list = steps_list,
        verify_text = verify_text,
        time_str = time_str
    );

    let full_content = format_with_frontmatter(&meta, &body);
    fs::write(&file_path, full_content)
        .map_err(|e| format!("写入计划文件失败 ({filename}): {e}"))?;

    // 绑定当前会话的 active_plan_id
    set_active_plan_id(conn, session_id, Some(&plan_id))?;

    // 同步步骤到 todo 队列
    let parsed_steps = parse_steps_from_markdown(&body);
    sync_steps_to_todos(conn, session_id, &parsed_steps);

    Ok(format!(
        "已成功在 `.harness/plans/{filename}` 创建专属任务计划【{title}】（版本 v1）。\n- 方案文件物理路径: `.harness/plans/{filename}`\n- 活动计划ID: `{plan_id}`\n\n【⚠️ 重要阶段指令 - 方案先行门禁】: 方案文档现已物理落盘！根据规划先行原则，当前轮次严禁继续调用任何代码写入/编辑工具（write_file/edit_file）！你必须立即停止调用工具，向用户输出结构化总结回复（汇报核心目标、设计方案、影响文件清单及实施步骤），并明确请用户在对话中审阅确认方案！用户在对话中回复确认或给出调整指示后方可开始编写代码实施。"
    ))
}

/// 更新计划内容、推进步骤或调整状态
pub fn update_plan(
    workspace: &Path,
    session_id: &str,
    plan_id: Option<&str>,
    reason: &str,
    status: Option<&str>,
    step_updates: Option<&[Value]>,
    modified_sections: Option<&Value>,
    revision_note: Option<&str>,
    conn: Option<&rusqlite::Connection>,
) -> Result<String, String> {
    ensure_plans_dir(workspace)?;
    let Some((file_path, mut meta, mut body)) =
        find_plan_file(workspace, session_id, plan_id, conn)
    else {
        let hint = available_plans_hint(workspace, Some(session_id), conn);
        return Err(format!("未找到指定或当前活动的计划文档，无法更新{hint}"));
    };

    let time_str = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    meta.version += 1;
    meta.updated_at = Utc::now().to_rfc3339();

    if let Some(st) = status {
        let st_clean = st.trim();
        if !st_clean.is_empty() {
            meta.status = st_clean.to_string();
            // 如果标记为 completed，解除活动锚点绑定
            if st_clean == "completed" {
                let _ = set_active_plan_id(conn, session_id, None);
            }
        }
    }

    // 1. 处理步骤状态更新 (step_updates: [{ index: 1, status: "done" / "in_progress" / "pending" }])
    if let Some(updates) = step_updates {
        let mut lines: Vec<String> = body.lines().map(|s| s.to_string()).collect();
        let mut step_count = 0;

        for line in &mut lines {
            let original = line.clone();
            let trimmed = original.trim();
            let is_step = trimmed.starts_with("- [ ]")
                || trimmed.starts_with("- [x]")
                || trimmed.starts_with("- [/]");
            if is_step {
                step_count += 1;
                for up in updates {
                    let match_idx = up
                        .get("index")
                        .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
                        .map(|v| v as usize)
                        == Some(step_count);
                    let match_content = up.get("content").and_then(|v| v.as_str()).map(|c| {
                        let c_trim = c.trim();
                        !c_trim.is_empty() && trimmed.contains(c_trim)
                    }).unwrap_or(false);

                    if match_idx || match_content {
                        if let Some(st) = up.get("status").and_then(|v| v.as_str()) {
                            let prefix = match st {
                                "done" => "- [x]",
                                "in_progress" => "- [/]",
                                _ => "- [ ]",
                            };
                            let content_part = if let Some(r) = trimmed.strip_prefix("- [x]") {
                                r
                            } else if let Some(r) = trimmed.strip_prefix("- [/]") {
                                r
                            } else if let Some(r) = trimmed.strip_prefix("- [ ]") {
                                r
                            } else {
                                ""
                            };
                            *line = format!("{} {}", prefix, content_part.trim());
                        }
                    }
                }
            }
        }
        body = lines.join("\n");
    }

    // 2. 处理各段落的局部更新 (modified_sections: { goals, architecture, files, steps, verification })
    if let Some(mods) = modified_sections {
        if let Some(g) = mods.get("goals").and_then(|v| v.as_str()) {
            body = replace_markdown_section(&body, "## 一、需求背景与目标", g);
        }
        if let Some(a) = mods.get("architecture").and_then(|v| v.as_str()) {
            body = replace_markdown_section(&body, "## 二、架构设计与技术方案", a);
        }
        if let Some(f) = mods.get("files").and_then(|v| v.as_array()) {
            let files_text = f
                .iter()
                .filter_map(|v| v.as_str())
                .map(|s| format!("- {}", s.trim()))
                .collect::<Vec<_>>()
                .join("\n");
            body = replace_markdown_section(&body, "## 三、涉及文件与影响范围", &files_text);
        }
        if let Some(s) = mods.get("steps").and_then(|v| v.as_array()) {
            let steps_text = if s.is_empty() {
                "- [ ] 步骤 1：详细梳理并推进核心任务".to_string()
            } else {
                s.iter()
                    .enumerate()
                    .map(|(i, val)| {
                        let raw = val.as_str().unwrap_or("").trim();
                        let text = raw
                            .trim_start_matches("- [ ]")
                            .trim_start_matches("- [x]")
                            .trim_start_matches("- [/]")
                            .trim();
                        let mark = if raw.starts_with("- [x]") {
                            "- [x]"
                        } else if raw.starts_with("- [/]") {
                            "- [/]"
                        } else {
                            "- [ ]"
                        };
                        format!("{} 步骤 {}：{}", mark, i + 1, text)
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            body = replace_markdown_section(&body, "## 四、分步执行清单", &steps_text);
        }
        if let Some(v) = mods.get("verification").and_then(|v| v.as_str()) {
            let v_trimmed = v.trim();
            let v_text = if v_trimmed.is_empty() {
                "- 运行项目自检测试与自动化验证脚本".to_string()
            } else if v_trimmed.starts_with('-') {
                v_trimmed.to_string()
            } else {
                format!("- {}", v_trimmed)
            };
            body = replace_markdown_section(&body, "## 五、验证与验收策略", &v_text);
        }
    }

    // 若计划标记为 completed，将所有步骤自动闭环为 - [x] 已完成
    if meta.status == "completed" {
        let mut completed_lines: Vec<String> = Vec::new();
        for line in body.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("- [ ]") {
                let rest = trimmed.strip_prefix("- [ ]").unwrap();
                completed_lines.push(format!("- [x] {}", rest.trim()));
            } else if trimmed.starts_with("- [/]") {
                let rest = trimmed.strip_prefix("- [/]").unwrap();
                completed_lines.push(format!("- [x] {}", rest.trim()));
            } else {
                completed_lines.push(line.to_string());
            }
        }
        body = completed_lines.join("\n");
    }

    // 3. 追加变更历史到 ## 六、需求变更历史
    let rev_text = revision_note
        .map(|r| r.trim())
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| reason.trim());

    let history_entry = format!("- **v{} ({})**：{}\n", meta.version, time_str, rev_text);
    if let Some(pos) = body.find("## 六、需求变更历史") {
        let rest = &body[pos..];
        if let Some(newline_pos) = rest.find('\n') {
            let insert_idx = pos + newline_pos + 1;
            body.insert_str(insert_idx, &history_entry);
        } else {
            body.push_str(&format!("\n{}", history_entry));
        }
    } else {
        body.push_str(&format!("\n## 六、需求变更历史\n{}", history_entry));
    }

    let full_content = format_with_frontmatter(&meta, &body);
    fs::write(&file_path, full_content)
        .map_err(|e| format!("更新计划文件失败: {e}"))?;

    // 同步更新后的步骤到 todo
    let steps = parse_steps_from_markdown(&body);
    sync_steps_to_todos(conn, session_id, &steps);

    let status_desc = match meta.status.as_str() {
        "completed" => "已完成验收并解除活动挂载",
        "suspended" => "已挂起",
        _ => "活动执行中",
    };

    let filename = file_path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    Ok(format!(
        "已成功将计划【{}】更新至版本 v{}（状态：{}）。\n- 方案文件物理路径: `.harness/plans/{}`\n变更说明：{}\n步骤清单与持久化文档已实时同步更新。",
        meta.title, meta.version, status_desc, filename, rev_text
    ))
}

/// 单独更新计划某一执行步骤的状态（供前端交互勾选并持久化）
pub fn update_plan_step_status(
    workspace: &Path,
    session_id: Option<&str>,
    plan_id: Option<&str>,
    step_index: usize,
    status: &str, // "pending" | "in_progress" | "done"
    conn: Option<&rusqlite::Connection>,
) -> Result<(), String> {
    ensure_plans_dir(workspace)?;
    let sid = session_id.unwrap_or_default();
    let Some((file_path, mut meta, body)) = find_plan_file(workspace, sid, plan_id, conn) else {
        let hint = available_plans_hint(workspace, session_id, conn);
        return Err(format!("未找到指定任务方案文档，无法更新步骤{hint}"));
    };

    meta.updated_at = Utc::now().to_rfc3339();

    let prefix = match status {
        "done" => "- [x]",
        "in_progress" => "- [/]",
        _ => "- [ ]",
    };

    let mut lines: Vec<String> = body.lines().map(|s| s.to_string()).collect();
    let mut current_idx = 0;
    let mut found = false;

    for line in &mut lines {
        let trimmed = line.trim();
        let is_step = trimmed.starts_with("- [ ]")
            || trimmed.starts_with("- [x]")
            || trimmed.starts_with("- [/]");
        if is_step {
            current_idx += 1;
            if current_idx == step_index {
                let content_part = if let Some(r) = trimmed.strip_prefix("- [x]") {
                    r
                } else if let Some(r) = trimmed.strip_prefix("- [/]") {
                    r
                } else if let Some(r) = trimmed.strip_prefix("- [ ]") {
                    r
                } else {
                    ""
                };
                *line = format!("{} {}", prefix, content_part.trim());
                found = true;
                break;
            }
        }
    }

    if !found {
        return Err(format!("未找到第 {} 步任务", step_index));
    }

    let new_body = lines.join("\n");
    let full_content = format_with_frontmatter(&meta, &new_body);
    fs::write(&file_path, full_content).map_err(|e| format!("写入任务方案文件失败: {e}"))?;

    // 同步步骤到 todo 队列
    let parsed_steps = parse_steps_from_markdown(&new_body);
    sync_steps_to_todos(conn, &meta.session_id, &parsed_steps);

    Ok(())
}


/// 辅助函数：替换 Markdown 某个二级标题下的正文段落
fn replace_markdown_section(content: &str, header_prefix: &str, new_body: &str) -> String {
    let Some(start_idx) = content.find(header_prefix) else {
        return content.to_string();
    };

    let rest = &content[start_idx..];
    let header_end = rest.find('\n').unwrap_or(0);
    let after_header = start_idx + header_end;

    // 查找下一个以 ## 开头的标题
    let next_header_idx = content[after_header..]
        .find("\n## ")
        .map(|idx| after_header + idx);

    let end_idx = next_header_idx.unwrap_or(content.len());

    let before = &content[..after_header];
    let after = &content[end_idx..];

    format!("{}\n\n{}\n\n{}", before.trim_end(), new_body.trim(), after.trim_start())
}

/// 切换当前活动的计划（多任务流转唤醒）
pub fn switch_plan(
    workspace: &Path,
    session_id: &str,
    plan_id: &str,
    conn: Option<&rusqlite::Connection>,
) -> Result<String, String> {
    ensure_plans_dir(workspace)?;
    let Some((target_path, mut target_meta, target_body)) =
        find_plan_file(workspace, session_id, Some(plan_id), conn)
    else {
        let hint = available_plans_hint(workspace, Some(session_id), conn);
        return Err(format!("未找到 ID 或名称为【{plan_id}】的计划文档{hint}"));
    };

    // 如果原先有其他正在执行的计划，将其置为 suspended
    if let Some((old_path, mut old_meta, old_body)) =
        find_plan_file(workspace, session_id, None, conn)
    {
        if old_meta.id != target_meta.id && old_meta.status == "in_progress" {
            old_meta.status = "suspended".into();
            old_meta.updated_at = Utc::now().to_rfc3339();
            let _ = fs::write(&old_path, format_with_frontmatter(&old_meta, &old_body));
        }
    }

    target_meta.status = "in_progress".into();
    target_meta.updated_at = Utc::now().to_rfc3339();
    let updated = format_with_frontmatter(&target_meta, &target_body);
    fs::write(&target_path, updated).map_err(|e| format!("保存切换后的计划失败: {e}"))?;

    // 绑定当前 active_plan_id
    set_active_plan_id(conn, session_id, Some(&target_meta.id))?;

    // 同步步骤到 todo 队列
    let steps = parse_steps_from_markdown(&target_body);
    sync_steps_to_todos(conn, session_id, &steps);

    let filename = target_path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    Ok(format!(
        "已成功切换当前活动计划为【{}】(版本 v{})，并恢复作为 System Prompt 顶层锚点。\n- 方案文件物理路径: `.harness/plans/{}`\n前序任务已妥善挂起，步骤清单已切换同步。",
        target_meta.title, target_meta.version, filename
    ))
}

/// 查阅计划完整 Markdown 内容
pub fn read_plan(
    workspace: &Path,
    session_id: &str,
    plan_id: Option<&str>,
    conn: Option<&rusqlite::Connection>,
) -> Result<String, String> {
    let Some((_, meta, body)) = find_plan_file(workspace, session_id, plan_id, conn) else {
        let hint = available_plans_hint(workspace, Some(session_id), conn);
        return match plan_id {
            Some(pid) => Err(format!("未找到 ID 或名称为【{pid}】的计划文档{hint}")),
            None => Err(format!("当前会话未绑定活动计划，且未找到默认计划{hint}")),
        };
    };
    Ok(format_with_frontmatter(&meta, &body))
}

/// 列出工作区内所有任务计划清单
pub fn list_plans(
    workspace: &Path,
    session_id: Option<&str>,
    include_archived: bool,
    conn: Option<&rusqlite::Connection>,
) -> Result<Vec<PlanSummary>, String> {
    let pdir = plans_dir(workspace);
    if !pdir.exists() {
        return Ok(Vec::new());
    }

    let active_id = session_id.and_then(|sid| get_active_plan_id(conn, sid));
    let mut dirs = vec![pdir.clone()];
    if include_archived {
        let adir = archive_dir(workspace);
        if adir.exists() {
            dirs.push(adir);
        }
    }

    let mut result = Vec::new();
    for d in dirs {
        if let Ok(entries) = fs::read_dir(&d) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("md") {
                    if let Ok(content) = fs::read_to_string(&path) {
                        let (meta, body) = parse_frontmatter(&content);
                        let steps = parse_steps_from_markdown(&body);
                        let completed_count =
                            steps.iter().filter(|s| s.status == "done").count();
                        let is_active = active_id
                            .as_ref()
                            .map(|aid| *aid == meta.id)
                            .unwrap_or(false);

                        result.push(PlanSummary {
                            id: meta.id,
                            title: meta.title,
                            status: meta.status,
                            version: meta.version,
                            filename: path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string(),
                            created_at: meta.created_at,
                            updated_at: meta.updated_at,
                            session_id: meta.session_id,
                            total_steps: steps.len(),
                            completed_steps: completed_count,
                            is_active,
                        });
                    }
                }
            }
        }
    }

    // 按更新时间降序排列
    result.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(result)
}

/// 运行时防失真装配：提炼当前活动计划摘要注入 System Prompt
pub fn load_active_plan_context(
    workspace: &Path,
    session_id: &str,
    conn: Option<&rusqlite::Connection>,
) -> Option<String> {
    if workspace.as_os_str().is_empty() {
        return None;
    }

    let (path, meta, body) = find_plan_file(workspace, session_id, None, conn)?;
    if meta.status != "in_progress" {
        return None;
    }

    let filename = path.file_name().and_then(|s| s.to_str()).unwrap_or("plan.md");
    let steps = parse_steps_from_markdown(&body);
    let done_steps = steps.iter().filter(|s| s.status == "done").count();
    let total_steps = steps.len();

    let mut step_lines = Vec::new();
    for s in &steps {
        let mark = match s.status.as_str() {
            "done" => "[x]",
            "in_progress" => "[/]",
            _ => "[ ]",
        };
        step_lines.push(format!("  - {} 步骤 {}：{}", mark, s.index, s.content));
    }
    let steps_text = if step_lines.is_empty() {
        "  - （暂无步骤细化）".to_string()
    } else {
        step_lines.join("\n")
    };

    let is_child = if let Some(c) = conn {
        crate::store::get_session(c, session_id)
            .ok()
            .flatten()
            .map(|s| s.parent_session_id.is_some())
            .unwrap_or(false)
    } else {
        false
    };

    if is_child {
        Some(format!(
            r#"## 当前项目权威执行计划（继承自父会话，来自 .harness/plans/{filename}，版本 v{version}）
> ⚠️ 核心准则：此计划是防止多轮需求失真的最高权威基准。你作为协同子任务/协作者，所有工作均是为了推进该计划，必须严格遵从此清单的分步与架构约束。
- **任务总目标**：{title}
- **全局推进状态**：执行中 (in_progress) · 完成度 ({done_steps}/{total_steps})
- **当前执行步骤清单**：
{steps_text}
- **子任务协同约束**：严禁擅自修改计划文档或推翻全局既定方案！你只需对照你的具体分配目标实施代码或验证。若在执行中发现原计划存在缺陷，请在最终交付回复中列出《计划调整建议》，由主进程统筹决策。"#,
            filename = filename,
            version = meta.version,
            title = meta.title,
            done_steps = done_steps,
            total_steps = total_steps,
            steps_text = steps_text
        ))
    } else {
        Some(format!(
            r#"## 当前任务权威执行计划（来自 .harness/plans/{filename}，版本 v{version}）
> ⚠️ 核心准则：此计划是防止多轮需求失真的最高权威基准。后续所有代码变更必须严格以分步清单为准逐步推进。
- **任务目标**：{title}
- **推进状态**：执行中 (in_progress) · 完成度 ({done_steps}/{total_steps})
- **当前执行步骤清单**：
{steps_text}
- **约束声明**：若用户提出需求调整或增删，**必须优先调用 `update_plan` 工具更新计划文档与变更历史，再开始改动代码**；全部执行完成并通过测试后，调用 `update_plan(status="completed")` 结案归档。"#,
            filename = filename,
            version = meta.version,
            title = meta.title,
            done_steps = done_steps,
            total_steps = total_steps,
            steps_text = steps_text
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("harness-test-{}", uuid::Uuid::new_v4()));
            let _ = fs::create_dir_all(&p);
            Self(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn test_frontmatter_parse_and_format() {
        let meta = PlanMeta {
            id: "plan-test-1".into(),
            title: "测试计划".into(),
            status: "in_progress".into(),
            created_at: "2026-09-22T10:00:00Z".into(),
            updated_at: "2026-09-22T10:00:00Z".into(),
            version: 1,
            session_id: "sess-123".into(),
        };
        let body = "## 一、需求背景\n这是测试正文内容";
        let formatted = format_with_frontmatter(&meta, body);

        let (parsed_meta, parsed_body) = parse_frontmatter(&formatted);
        assert_eq!(parsed_meta.id, "plan-test-1");
        assert_eq!(parsed_meta.title, "测试计划");
        assert_eq!(parsed_meta.version, 1);
        assert!(parsed_body.contains("这是测试正文内容"));
    }

    #[test]
    fn test_plan_crud_and_steps() {
        let tmp = TempDir::new();
        let ws = tmp.path();
        let sid = "session-test";

        // 1. 创建计划
        let res = create_plan(
            ws,
            sid,
            "鉴权重构",
            "替换 Session 为 JWT",
            "使用 jsonwebtoken HS256",
            &["src/auth.rs".into()],
            &["实现编解码".into(), "接入中间件".into()],
            Some("cargo test"),
            None,
        );
        assert!(res.is_ok());

        // 2. 验证文件存在
        let list = list_plans(ws, Some(sid), false, None).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title, "鉴权重构");
        assert_eq!(list[0].total_steps, 2);
        assert_eq!(list[0].completed_steps, 0);

        // 3. 更新步骤状态与版本
        let updates = json!([
            { "index": 1, "status": "done" },
            { "index": 2, "status": "in_progress" }
        ]);
        let up_res = update_plan(
            ws,
            sid,
            None,
            "完成步骤 1 编解码",
            None,
            updates.as_array().map(|v| v.as_slice()),
            None,
            Some("完成 JWT 编解码实现"),
            None,
        );
        assert!(up_res.is_ok());

        let list2 = list_plans(ws, Some(sid), false, None).unwrap();
        assert_eq!(list2[0].version, 2);
        assert_eq!(list2[0].completed_steps, 1);

        // 4. 验证 Prompt 注入内容
        let ctx = load_active_plan_context(ws, sid, None).unwrap();
        assert!(ctx.contains("当前任务权威执行计划"));
        assert!(ctx.contains("[x] 步骤 1"));
        assert!(ctx.contains("[/] 步骤 2"));

        // 5. 验证独立更新步骤状态 update_plan_step_status
        let step_res = update_plan_step_status(ws, Some(sid), None, 2, "done", None);
        assert!(step_res.is_ok());
        let list3 = list_plans(ws, Some(sid), false, None).unwrap();
        assert_eq!(list3[0].completed_steps, 2);

        // 6. 验证 modified_sections.steps 全量替换分步清单
        let new_steps_mod = json!({
            "steps": ["重新梳理的步骤1", "新增的步骤2", "新增的步骤3"],
            "verification": "cargo test --all"
        });
        let mod_res = update_plan(
            ws,
            sid,
            None,
            "需求调整，更新执行步骤",
            None,
            None,
            Some(&new_steps_mod),
            None,
            None,
        );
        assert!(mod_res.is_ok());
        let list4 = list_plans(ws, Some(sid), false, None).unwrap();
        assert_eq!(list4[0].total_steps, 3);
        assert_eq!(list4[0].completed_steps, 0); // 重新梳理的新步骤默认为 pending 0

        // 7. 验证标记为 completed 时自动闭环所有步骤为 done [x]
        let comp_res = update_plan(
            ws,
            sid,
            None,
            "全部完成交付",
            Some("completed"),
            None,
            None,
            None,
            None,
        );
        assert!(comp_res.is_ok());
        let list5 = list_plans(ws, Some(sid), false, None).unwrap();
        assert_eq!(list5[0].completed_steps, 3);
        assert_eq!(list5[0].status, "completed");
    }

    #[test]
    fn test_plan_session_isolation() {
        let tmp = TempDir::new();
        let ws = tmp.path();
        let sid_a = "session-a";
        let sid_b = "session-b";

        // 1. 会话 A 创建专属计划
        create_plan(
            ws,
            sid_a,
            "会话A专属计划",
            "目标A",
            "架构A",
            &[],
            &["步骤A1".into()],
            None,
            None,
        )
        .unwrap();

        // 2. 会话 A 查询其活动计划：应当能成功找到
        let plan_a = find_plan_file(ws, sid_a, None, None);
        assert!(plan_a.is_some());
        assert_eq!(plan_a.unwrap().1.title, "会话A专属计划");

        // 3. 会话 B 尚未创建计划，查询其活动计划：必须严格隔离返回 None，绝不可泄露会话 A 的计划
        let plan_b = find_plan_file(ws, sid_b, None, None);
        assert!(plan_b.is_none());

        // 4. 会话 B 加载 Prompt 计划上下文：也必须返回 None
        let ctx_b = load_active_plan_context(ws, sid_b, None);
        assert!(ctx_b.is_none());
    }

    #[test]
    fn test_plan_not_found_hints() {
        let temp = std::env::temp_dir().join(format!("harness_test_plan_hint_{}", uuid::Uuid::new_v4()));
        let ws = temp.as_path();
        let sid = "session-test-hint";

        // 没有计划时查询
        let err1 = read_plan(ws, sid, Some("non_existent_plan"), None).unwrap_err();
        assert!(err1.contains("暂无任何计划文档"));

        // 创建一个计划
        create_plan(
            ws,
            sid,
            "用户认证系统重构",
            "重构认证模块",
            "JWT 认证架构",
            &[],
            &["实现登录接口".into()],
            None,
            None,
        ).unwrap();

        // 再次查询不存在的 plan_id，验证提示包含了现有的计划 ID 和标题
        let err2 = read_plan(ws, sid, Some("wrong_id"), None).unwrap_err();
        assert!(err2.contains("用户认证系统重构"));
        assert!(err2.contains("已有的计划文档包括"));

        // switch_plan 也应包含已有计划提示
        let err3 = switch_plan(ws, sid, "wrong_id", None).unwrap_err();
        assert!(err3.contains("用户认证系统重构"));
        assert!(err3.contains("已有的计划文档包括"));

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_child_session_inherits_parent_plan() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::store::init_schema(&conn).unwrap();
        let tmp = TempDir::new();
        let ws = tmp.path();

        let parent = crate::store::create_session(&conn, &ws.to_string_lossy(), None, "主会话", "standard").unwrap();
        let child = crate::store::create_subagent_session(
            &conn,
            &parent.id,
            "后端开发",
            "实现JWT刷新",
            "任务详情",
            &ws.to_string_lossy(),
            None,
            None,
            None,
        ).unwrap();

        // 1. 父会话创建计划
        create_plan(
            ws,
            &parent.id,
            "主项目重构方案",
            "重构认证与授权",
            "JWT 架构",
            &[],
            &["实现登录".into(), "实现刷新".into()],
            None,
            Some(&conn),
        ).unwrap();

        // 2. 子会话查询计划：应该能够穿透查询到父会话的活动计划
        let plan_opt = find_plan_file(ws, &child.id, None, Some(&conn));
        assert!(plan_opt.is_some(), "子会话应能穿透查询到父会话的计划");
        assert_eq!(plan_opt.unwrap().1.title, "主项目重构方案");

        // 3. 子会话加载 Prompt：应该包含继承自父会话及子任务协同约束
        let ctx = load_active_plan_context(ws, &child.id, Some(&conn)).unwrap();
        assert!(ctx.contains("继承自父会话"));
        assert!(ctx.contains("子任务协同约束"));
        assert!(ctx.contains("严禁擅自修改计划文档"));
    }
}

