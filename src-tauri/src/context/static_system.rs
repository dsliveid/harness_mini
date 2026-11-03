use crate::models::{GrowthItem, Session};
use crate::store;
use rusqlite::Connection;

#[derive(Debug, Clone)]
pub struct ResolvedLink {
    pub name: String,
    pub path: String,
    pub description: String,
    pub constraints: Option<String>,
}

/// 解析项目关联条目（只读按路径查找，不创建项目实体）
pub fn resolve_links(conn: &Connection, project_id: &str) -> Result<Vec<ResolvedLink>, String> {
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

/// 纯文本组装（无 IO，便于单测）
pub fn project_section(
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

/// 组装注入 system prompt 的「项目约束 + 演进经验 + 关联项目」段；全部为空时返回 None（不追加）
pub fn build_static_project_section(
    conn: &Connection,
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

/// 构建静态常驻协作者名录（剔除动态运行中/空闲状态，保持文本恒定）
pub fn build_static_collaborator_section(
    conn: &Connection,
    session_id: &str,
) -> Option<String> {
    let collabs = store::list_collaborators(conn, session_id).ok()?;
    if collabs.is_empty() {
        return None;
    }
    let mut text = String::from("## 可用项目协作者名录 (Collaborators)\n你拥有以下常驻专业协作者团队。主进程作为总架构师，默认优先就地直接处理简单任务与局部改动；仅在遇到明确的跨模块并发、专职领域或大体量独立子任务时，根据下述职责定位按需委派给对应协作者。\n\n");
    for c in collabs {
        let role_display = match c.subagent_role.as_deref() {
            Some("image_gen") => "AI 绘画师 / 图像生成",
            Some("vision") => "视觉感知 / 图像识别",
            Some("frontend") => "前端开发",
            Some("backend") => "后端开发",
            Some("pm") => "产品经理",
            Some("pmo") => "项目经理",
            Some("testing") => "测试校验",
            Some("review") => "代码审阅",
            Some("fullstack") => "全栈开发",
            Some(r) => r,
            None => "专业协作者",
        };
        let dispatch_rule = c
            .dispatch_rule
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| crate::models::default_dispatch_rule_for_role(c.subagent_role.as_deref().unwrap_or("")));
        let model_info = if let Some(ref m) = c.model_id {
            format!(" | 对话模型: `{m}`")
        } else {
            String::new()
        };
        let image_model_info = if let Some(ref im) = c.image_model_id {
            format!(" | 生图模型: `{im}`")
        } else {
            String::new()
        };
        text.push_str(&format!(
            "### 【{}】(ID: `{}` | 角色: {}{}{})\n- **主进程建议调度规则（按需委派）**: {}\n- **专业职责设定**: {}\n\n",
            c.title,
            c.id,
            role_display,
            model_info,
            image_model_info,
            dispatch_rule,
            c.subagent_task.as_deref().unwrap_or("负责该领域专职工作")
        ));
    }
    Some(text)
}

/// 构建纯静态 System Prompt (Layer 1 & Layer 2)
/// 
/// 核心保障：同一个 Session 生命周期内，只要静态配置未变，该函数输出的 System Prompt
/// 比特级完全恒等，确保成为 Radix Tree 永久稳固的树根！
pub fn build_static_system_prompt(
    session: &Session,
    disabled_sops: &[String],
    plan_mode: &str,
    static_project_section: Option<&str>,
    static_memory_section: Option<&str>,
    static_collab_section: Option<&str>,
) -> String {
    let os = if cfg!(windows) { "Windows" } else { "Unix-like" };
    let is_collab = session.session_type == "collaborator";
    let is_sub = is_collab
        || session.session_type == "subprocess"
        || session.session_type == "subagent"
        || session.parent_session_id.is_some();
    // 已统一由 sop_catalog::build_sop_rules 管理各 SOP 启用状态

    let base = if session.workspace_path.is_empty() {
        // 未绑定工作区：纯对话模式
        format!(
            r#"你是 harness_mini，一个谨慎、专业的编码助手。当前会话未绑定工作区，处于纯对话模式。

操作系统：{os}

工作规则：
1. 当前会话没有绑定本地目录，文件与命令类工具（read_file / write_file / edit_file / glob / grep / list_dir / run_command）均不可用，不要尝试调用；相关的问题直接以文字回答或给出建议代码。
2. 如果用户需要你实际读写文件、执行命令，请提示：在顶栏点击工作区按钮选择目录后再继续。
3. 全程使用简体中文与用户交流，回答准确、简洁。
4. 【思考与交付通道分离规范（严禁正文留空）】：若当前模型具备深度思考能力（Thinking/Reasoning），思考过程仅用于你内部的逻辑推导与推演；面向用户的最终回答与结论必须完整输出在正式正文（Content）中，严禁在思考结束后直接停止生成导致正文留空。"#,
            os = os
        )
    } else {
        // 统一架构：主对话、协作者、专职子 Agent 共享完全一致的规则骨架与运行门禁
        let header = if is_collab {
            let role = session.subagent_role.as_deref().unwrap_or("常驻专业协作者");
            let task = session.subagent_task.as_deref().unwrap_or("负责该领域专职工作");
            format!(
                r#"你是 harness_mini 项目中由用户配置的专业常驻协作者 (Collaborator)。
角色定位：{role}
职责设定：{task}
工作区根目录：{path}
操作系统：{os}"#,
                role = role,
                task = task,
                path = session.workspace_path,
                os = os
            )
        } else if is_sub {
            let role = session.subagent_role.as_deref().unwrap_or("专职子任务协作 Agent");
            format!(
                r#"你是 harness_mini 派生出的专业协作子 Agent 进程。
角色定位：{role}
专属任务目标、探查入口与交付验收标准已在下方【黑板交接书】中权威定义。请依此展开分析与推进。
工作区根目录：{path}
操作系统：{os}"#,
                role = role,
                path = session.workspace_path,
                os = os
            )
        } else {
            format!(
                r#"你是 harness_mini，一个谨慎、专业的编码 Agent，运行在用户的本地工作区中。
工作区根目录：{path}
操作系统：{os}"#,
                path = session.workspace_path,
                os = os
            )
        };

        let mut rules = Vec::new();
        let mut rule_num = 1;

        if is_sub {
            rules.push(format!("{rule_num}. 专注于完成分配给你的专职目标。当前合法工作区根目录与主会话严格保持完全一致【{}】，所有工具均以该根目录为基准。", session.workspace_path));
        } else {
            rules.push(format!("{rule_num}. 路径基准说明：所有相对路径默认相对于工作区根目录【{}】解析。当前工作区为主要开发上下文。", session.workspace_path));
        }
        rule_num += 1;

        if session.subagent_role.as_deref() == Some("image_gen") {
            rules.push(format!("{rule_num}. 【AI 绘画与图像生成核心职责（严禁编写 Python 脚本）】：根据用户或主进程的要求，深入理解视觉意图并提炼出画面细节丰富的高质量提示词（包括画质、风格、构图、主体、光影），直接调用 `generate_image` 工具生成图片，严禁编写 Python 脚本画图。图片生成完成后在回复中展示成果并简明汇报。"));
            rule_num += 1;
        }

        if session.subagent_role.as_deref() == Some("vision") {
            rules.push(format!("{rule_num}. 【视觉感知与图像识别核心职责】：根据用户或主进程指派的目标，调用 `recognize_image` 工具深入识别、提取与分析目标图片（支持文字提取、设计稿分析、界面元素审查等），并在回复中给出详尽专业的视觉分析报告。"));
            rule_num += 1;
        }

        // --- 统一 SOP 字典动态装配（单一真源） ---
        let (sop_rules, next_rule_num) = crate::sop_catalog::build_sop_rules(session, disabled_sops, plan_mode, rule_num);
        rules.extend(sop_rules);
        rule_num = next_rule_num;

        if is_sub {
            rules.push(format!(
                "{rule_num}. 【扩展工具目录（按需代理调用机制）】：\n                 当原生核心工具无法满足特定业务场景时，你可通过 `call_extended_tool(tool_name: \"...\", arguments: {{...}})` 按需调用以下扩展工具：\n                 - 知识沉淀与技能管理：\n                   * `record_memory(content: string, title?: string, category?: string)`: 沉淀技术大盘与主题事实至 .harness/memory/\n                   * `read_memory(query: string, category?: string)`: 检索 .harness/memory/ 中已沉淀的记忆知识\n                   * `list_skills()`: 查看已固化项目技能名录\n                   * `run_skill(skill_name: string, args?: object)`: 运行已固化项目技能\n                 - 多模态感知与生成：\n                   * `generate_image(prompt: string, aspect_ratio?: string)`: 绘制图片、图标、LOGO 或视觉图形\n                   * `recognize_image(path: string, prompt?: string)`: 深入识别与分析本地图片内容\n                 *(调用指引：若不确定完整参数规范，可先调用原生 `lookup_tool(tool_name: \"...\")` 查询其完整 Schema，或传 `arguments: {{\"help\": true}}`；若直接调用入参有误，系统也会自动返回完整参数指导)"
            ));
            rule_num += 1;
        } else {
            rules.push(format!(
                "{rule_num}. 【扩展工具目录（按需代理调用机制）】：\n                 当原生核心工具无法满足特定业务场景时，你可通过 `call_extended_tool(tool_name: \"...\", arguments: {{...}})` 按需调用以下扩展工具：\n                 - 子任务与协作者编排：\n                   * `spawn_subagent(role: string, task: string, title?: string, subpath?: string)`: 派生专职子 Agent 并行处理独立任务（按职责拆分，工作量尽量控制在 50 步内，无需提醒子任务步数）\n                   * `wait_subagents(timeout_secs?: number)`: 等待子 Agent 汇聚交付报告（默认 600 秒长等待）\n                   * `reject_subagent(subagent_id: string, feedback: string)`: 验收不通过时驳回子任务并指示返工（非必要情况禁止私自编码）\n                   * `get_subagent_status(subagent_id?: string)`: 查询子 Agent 运行状态\n                   * `stop_subagent(subagent_id: string)`: 终止指定子 Agent\n                   * `dispatch_collaborator(collaborator_id: string, task: string, context?: object)`: 按需委派任务给常驻专业协作者\n                   * `wait_collaborators(timeout_secs?: number)`: 等待协作者执行完成并获取成果汇报\n                   * `get_collaborators()`: 查询当前可用协作者名录与状态\n                 - 网页与浏览器交互：\n                   * `fetch_web_page(url: string)`: 通过 HTTP/HTTPS 获取网页并提取为 Markdown 文本\n                   * `browser_navigate(url: string)`: 打开后台 Chromium 访问网页并获取可交互元素清单 (@eX)\n                   * `browser_interact(action: string, element_id?: string, text?: string)`: 网页元素交互（点击/输入/提交）\n                   * `browser_scroll(direction: string)`: 滚动网页视口 (down/up/top/bottom)\n                   * `browser_get_debug_state()`: 获取当前页面的 Console 报错与 HTTP 异常列表\n                   * `browser_screenshot()`: 截取当前网页屏幕快照\n                   * `browser_close()`: 关闭当前浏览器会话并释放系统资源\n                 - 知识沉淀与技能管理：\n                   * `record_memory(content: string, title?: string, category?: string)`: 沉淀技术大盘与主题事实至 .harness/memory/\n                   * `read_memory(query: string, category?: string)`: 检索 .harness/memory/ 中已沉淀的记忆知识\n                   * `list_skills()`: 查看已固化项目技能名录\n                   * `run_skill(skill_name: string, args?: object)`: 运行已固化项目技能\n                 - 多模态感知与生成：\n                   * `generate_image(prompt: string, aspect_ratio?: string)`: 绘制图片、图标、LOGO 或视觉图形\n                   * `recognize_image(path: string, prompt?: string)`: 深入识别与分析本地图片内容\n                 *(调用指引：若不确定完整参数规范，可先调用原生 `lookup_tool(tool_name: \"...\")` 查询其完整 Schema，或传 `arguments: {{\"help\": true}}`；若直接调用入参有误，系统也会自动返回完整参数指导)"
            ));
            rule_num += 1;
        }



        rules.push(format!("{rule_num}. 不要执行破坏性命令（如递归删除、格式化磁盘等），它们会被强制要求用户确认。"));
        rule_num += 1;

        rules.push(format!("{rule_num}. 全程使用简体中文与用户交流；最终回复简洁总结：做了什么、改了哪些文件、验证结果如何。思考过程仅用于内部推导，面向用户的答复必须且只能输出在正文（Content）中。"));

        let rules_text = rules.join("\n");
        format!(
            r#"{header}

工作规则：
{rules_text}"#,
            header = header,
            rules_text = rules_text
        )
    };

    let mut full = base;
    if let Some(sec) = static_project_section {
        full.push_str("\n\n");
        full.push_str(sec);
    }
    if let Some(mem) = static_memory_section {
        full.push_str("\n\n## 本项目持久化认知与静态架构基线（来自 .harness/memory/）\n");
        full.push_str(mem);
    }
    if let Some(collabs) = static_collab_section {
        full.push_str("\n\n");
        full.push_str(collabs);
    }

    // 子任务黑板任务交接书权威挂载 (Layer 3 权威上下文)
    // 保证任务目标、交付标准与禁区在多轮交互中 100% 字节级恒定，100% 命中 KV 前缀缓存且永不被历史压缩抹除
    if is_sub && !session.workspace_path.is_empty() {
        let ws = std::path::Path::new(&session.workspace_path);
        let knowledge_filename = format!("{}_knowledge.md", session.id.replace('-', "_"));
        let knowledge_path = ws.join(".harness").join("subtasks").join(&knowledge_filename);
        if knowledge_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&knowledge_path) {
                if !content.trim().is_empty() {
                    full.push_str(&format!(
                        "\n\n## 📋 协同任务黑板交接书（最高权威基准，来自 .harness/subtasks/{knowledge_filename}）\n> ⚠️ 核心准则：此交接书定义了本任务的目标、验收标准与负向禁区。在多轮执行中始终保持最高效力。\n\n{content}\n"
                    ));
                }
            }
        }
    }

    full
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_static_system_prompt_identity() {
        let session = Session {
            workspace_path: "D:\\test_ws".into(),
            ..Default::default()
        };
        let p1 = build_static_system_prompt(&session, &[], "standard", Some("## 静态约束"), None, None);
        let p2 = build_static_system_prompt(&session, &[], "standard", Some("## 静态约束"), None, None);
        assert_eq!(p1, p2, "Static system prompt must be 100% bit-identical across runs");
    }

    #[test]
    fn test_static_system_prompt_never_contains_ephemeral_states() {
        let session = Session {
            workspace_path: "D:\\test_ws".into(),
            ..Default::default()
        };
        let p = build_static_system_prompt(&session, &[], "standard", None, None, None);
        assert!(!p.contains("<runtime_environment_anchor>"));
        assert!(!p.contains("断点恢复与进度重对齐提示"));
        assert!(!p.contains("💡 本项目近期边读边记重点沉淀"));
    }
}
