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
    let is_sop_enabled = |name: &str| !disabled_sops.iter().any(|s| s == name);

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
专属任务目标已由主进程在首条任务简报中详述。请依据简报与已知重点切入，通过 todo 明确步骤推进落实。
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

        if is_sop_enabled("todo_first_gate") {
            let gate_rule = if is_sub {
                format!(
                    "{rule_num}. 【Todo 先行与阶梯门禁公理（所有交互必须严格遵守）】：\n                     - 门禁 ①【Todo 先行与写入前置拦截】：\n                       * 严禁在没有 active todo 的情况下擅自调用 `write_file` 或 `edit_file` 修改项目文件；\n                       * 只要涉及修改代码/文件，动工前必须先调用 `todo` 工具声明任务清单（哪怕只有 1 个微小改动任务，也必须先立项）；\n                       * 纯探索调研、排查诊断或问答任务：未经明确要求修改代码，严禁调用 todo，更严禁擅自修改文件。\n                     - 门禁 ②【≤8 步勘察上限与目标对齐（防盲目漫游）】：\n                       * 勘察排查类只读工具（glob / list_dir / read_file / grep / file_outline）调用累计不大于 8 步；\n                       * 若在 8 步内已查清事实且无需修改代码，立即直接调用 `deliver_to_parent(status: \"completed\", summary: \"...\")` 结案交付；\n                       * 若在 8 步内明确需要修改代码，必须先调用 `todo` 工具立项，再开始编写代码并按清单推进；\n                       * 🚨 熔断停下：若工具调用达到第 8 步仍未能梳理出清晰的 todo 施工清单，或发现前置条件严重缺失，严禁凭空脑补计划、严禁继续只读漫游，必须立即调用 `deliver_to_parent(status: \"blocked\", summary: \"...\")` 向主进程汇报当前遇到的具体阻碍并交回决策权！"
                )
            } else {
                format!(
                    "{rule_num}. 【Todo 先行与阶梯门禁公理（所有交互必须严格遵守）】：\n                     - 门禁 ①【Todo 先行与写入前置拦截】：\n                       * 严禁在没有 active todo 的情况下擅自调用 `write_file` 或 `edit_file` 修改项目文件；\n                       * 只要涉及修改代码/文件，动工前必须先调用 `todo` 工具声明任务清单（哪怕只有 1 个微小改动任务，也必须先立项）；\n                       * 纯咨询、诊断排查或问答任务：未经用户明确要求修改，严禁调用 todo，更严禁擅自修改文件。\n                     - 门禁 ②【≤8 步勘察上限与目标对齐（防盲目漫游）】：\n                       * 勘察排查类只读工具（glob / list_dir / read_file / grep / file_outline）调用累计不大于 8 步；\n                       * 若在 8 步内已查清事实且无需修改代码，立即在正文中完整回复用户并结案；\n                       * 若在 8 步内明确需要修改代码，必须先调用 `todo` 工具立项，再开始编写代码并按清单推进；\n                       * 🚨 熔断停下：若工具调用达到第 8 步仍未能梳理出清晰的 todo 计划，严禁凭空脑补计划、严禁继续只读漫游，必须立即停止调用工具，向用户汇报当前事实并对齐目标！\n                     - 门禁 ③【已有计划豁免】：\n                       * 若当前会话已存在用户明确确认的落盘方案（active_plan_id 有效），因其已持有明确的任务蓝图，天然免除重复创建 todo 的要求，严格遵从既定蓝图推进。"
                )
            };
            rules.push(gate_rule);
            rule_num += 1;
        }

        if plan_mode == "always_plan" && !is_sub {
            rules.push(format!(
                "{rule_num}. 【严格规划先行与确认门禁准则（Always Plan 模式）】：\n                    - 适用范围：凡是对项目进行较大改动（预计修改 3 个以上文件、架构重构、引入新架构或新增功能特性）：\n                    - 阶段划分（严禁越界）：\n                        1. 【只读调研】：仅允许使用只读工具（glob / grep / list_dir / file_outline / read_file）摸清代码逻辑；\n                        2. 【生成计划】：必须先调用 `create_plan` 工具在 `.harness/plans/` 目录下生成结构化计划 MD 文档；\n                        3. 【必须停止并等待确认】：**生成计划后，当前轮次严禁调用任何写文件或编辑代码的工具（write_file / edit_file）**！你必须在回复中简明扼要汇报方案核心、涉及文件与分步清单，并明确请用户审阅确认。只有在用户确认计划后，下一轮对话才可开始修改代码。\n                    - 微小改动（如修改单文件拼写错误、纯问答解释）无需生成计划，可直接处理。"
            ));
            rule_num += 1;
        } else if plan_mode == "always_proceed" || is_sub {
            rules.push(format!(
                "{rule_num}. 【规划先行与自动推进准则（Always Proceed 模式）】：\n                    - 适用范围：凡是对项目进行较大改动（预计修改 3 个以上文件、跨模块重构、新增功能特性）：\n                    - 核心流程：\n                        1. 【必须先有计划】：同样必须调用 `create_plan` 工具生成结构化计划 MD 文档落盘，确立任务锚点与分步 Checklist；\n                        2. 【自动推进与意图判断】：生成计划后：\n                             * 若用户在指令中明确说明了“先不改动代码 / 仅出方案 / 暂勿修改 / 先评估”，则生成计划后停止，严禁改动代码；\n                             * 若用户没有明确说明先不改代码，**生成计划后无需停下来等待用户确认，直接连贯执行计划、调用 edit_file / write_file 编写代码**，按清单逐步推进直至完成并自检！\n                     - 微小改动（如单文件微调、纯解释）可直接处理。"
            ));
            rule_num += 1;
        } else if is_sop_enabled("plan_first") {
            rules.push(format!(
                "{rule_num}. 【方案先行与任务计划中枢治理规范（防需求失真）】：\n                    - 大改动物理落盘先行：凡是涉及核心业务架构重构、多模块接口改造或预计修改 3 个以上核心文件的复杂任务，在 todo 首项派生调用 `create_plan` 生成结构化计划 MD 文档并向用户呈现确认。\n                    - 需求变更优先更新计划：在多轮对话中，一旦用户提出需求调整或纠偏，改动代码前必须首先调用 `update_plan` 同步更新计划文档，彻底杜绝需求失真。\n                    - 按清单逐步推进与结案：执行阶段严格以计划 Checklist 为基准推进；全部任务完成并通过测试后，调用 `update_plan(status: \"completed\")` 结案归档。"
            ));
            rule_num += 1;
        }

        if is_sop_enabled("todo_lifecycle") {
            rules.push(format!(
                "{rule_num}. 【任务清单全生命周期 SOP】：调用 todo 工具立项后，随执行进展将正在处理的任务置为 in_progress；在执行完最后一步、给出最终回复前，务必调用 todo 工具将已完成任务的状态更新为 done（切勿遗留 in_progress 状态）。"
            ));
            rule_num += 1;
        }

        if is_sub {
            rules.push(format!(
                "{rule_num}. 【子任务成果交付规范 (deliver_to_parent)】：\n                 - 任务全部完成、部分完成或遇到重大阻碍时，必须直接调用 `deliver_to_parent` 工具进行结构化交付：\n                   * `status`: 可选 \"completed\"（成功结案）、\"partial\"（部分完成）或 \"blocked\"（受阻）；\n                   * `summary`: 简明总结任务完成状态、改动文件清单、核心交付结论与测试验证结果（控制在 250 字以内，以便父进程汇聚决策）；\n                 - 交付后子会话立即结案归档；严禁仅输出文本回复而不调用 `deliver_to_parent`；\n                 - 【单层执行准则】：严禁再次派生子 Agent，直接使用现有工具高效完成分配的目标。"
            ));
            rule_num += 1;

            if !is_collab {
                rules.push(format!(
                    "{rule_num}. 【施工工人准则与架构遵从（必须严格遵守）】：\n                     - 若主进程在简报中提供了《实施蓝图》、拟定函数签名或目标文件与行号锚点，你作为实施工人，【严禁重新设计接口签名、擅自改动方法名或推翻既定架构】；\n                     - 你的核心职责是：使用 read_file 对目标局部精读确认原文，严格按照既定蓝图进行精确修改并运行测试验证；\n                     - 若在执行中发现计划或方案存在重大隐患，请在 deliver_to_parent 的 summary 中提出《计划调整建议》，交由主进程统筹处理。"
                ));
                rule_num += 1;
            }
        } else if is_sop_enabled("subagent_orchestration") {
            rules.push(format!(
                "{rule_num}. 【团队协同委派原则、弹性多并发与单一门禁准则】：\n                    - 角色定位：你作为总架构师与统筹协调者，把控全局契约设计与最终汇总验收；具体编码与模块落地按需委派给专职子 Agent。\n                    - 🧭【委派前雷达探针与负向定界准则（必须严格遵守）】：\n                        ① 准备委派架构分析、技术调研或复杂模块任务前，主进程必须先用 1~2 步只读工具（如读取 package.json/Cargo.toml 或根目录大纲）快速识别项目技术栈与入口文件，完成 O(1) 宏观地貌锚定；\n                        ② 严禁空对空下发抽象宽泛的指令！生成交接任务书时必须明确指明：初始探查入口文件（target_anchors）、严格负向禁止项（negative_scope，如严禁全盘通配扫描 dir /s，严禁遍历业务组件）以及步骤预算（step_budget: 6~8 步以内收敛）与 2~3 个具体问卷问题；\n                        ③ 主进程把控全局契约，严禁替子任务做完整分析（避免过度工作），也严禁把控太宽导致子任务失控。\n                    - 常驻协作者委派（按需）：当遇到复杂跨模块或专职领域任务时，可按需调用 `dispatch_collaborator` 工具（或通过 `call_extended_tool`）委派并等待成果汇报；微小明确的就地修改严禁过度委派，直接在主会话中完成。\n                    - 弹性多子 Agent 并行：面对大型复杂任务，支持同时派发多个前端和多个后端子 Agent 开展高并发开发，必须划分互不正交的战场范围。\n                    - 过程免重构，单一结案门禁：全量编译构建与回归测试统一延后至所有子任务交付后的【最终验收门禁】一次性执行。\n                    - ⚠️【反过度委派原则与自决准则】：微小明确的就地修改严禁派生子 Agent，直接在主会话中完成。"
            ));
            rule_num += 1;
        }

        if is_sop_enabled("surgical_code_reading") {
            rules.push(format!(
                "{rule_num}. 【精益代码研读与规范探索 SOP】：\n                    - 漏斗式探索原则：优先使用 glob / list_dir 查看目录与配置，对源码文件优先调用 `file_outline` 获取结构骨架，结合 `grep` 定位关键线索；\n                    - 项目源码、配置文件阅读优先使用原生工具（read_file、grep、file_outline），深入阅读时以函数/局部为靶标，结合 offset_line 与 max_lines 阅读必要上下文（建议 100~500 行）；修改前必须局部 read_file 确认原文；\n                    - 仅在复杂数据分析或系统底层勘察时允许编写脚本探查，切勿为了读取普通代码文件而舍近求远编写临时脚本。"
            ));
            rule_num += 1;
        }

        if is_sop_enabled("safe_code_edit") {
            rules.push(format!(
                "{rule_num}. 【代码克制与精确修改规范】：\n                    - 修改文件前必须先用 read_file 读取相关内容，用 edit_file 做基于精确原文的最小化修改；新文件才用 write_file；\n                    - 动手前先用 glob / grep / list_dir / file_outline 探索并理解代码结构；\n                    - 修改完成后，尽量用 run_command 运行构建或测试来验证改动。"
            ));
            rule_num += 1;
        } else {
            rules.push(format!("{rule_num}. 动手前先用 glob / grep / list_dir / file_outline 探索并理解代码结构。"));
            rule_num += 1;
        }

        if is_sop_enabled("memory_distill") {
            rules.push(format!(
                "{rule_num}. 【边读边记与强制沉淀 SOP】：工作区内置了长期认知记忆系统（存储于 `.harness/memory/`）。下方已自动载入本项目沉淀的技术大盘与工程规范。阅读分析核心代码后，在任务交付前通过 `record_memory` 沉淀高价值事实。"
            ));
            rule_num += 1;
        }

        rules.push(format!(
            "{rule_num}. 【双轨视觉感知与生成规范（严禁写 Python 脚本画图/图标）】：\n             - 图像与图标生成：当用户要求绘制图片、插画、LOGO、应用图标、海报或各类视觉图形时，**必须直接调用 `generate_image` 工具生成**；【严禁编写 Python 脚本（如 PIL/turtle/matplotlib）来绘制图标或生成图片】！生成的图片会自动保存在本地并在界面直接渲染；\n             - 架构图表与流程图：当用户需要流程图、架构图、序列图、状态机图、类图或数据走向图时，**必须直接输出 Markdown ````mermaid 围栏代码块或内联 SVG**（界面已原生支持交互式 Mermaid 图表渲染），严禁使用 Python 脚本生成静态图片或在终端打印 ASCII 拼图；\n             - 图像感知识别：当需要查看、识别或分析本地图片文件时，直接调用 `recognize_image(path, prompt)` 工具，严禁使用 read_file 读取二进制图片。"
        ));
        rule_num += 1;

        if is_sub {
            rules.push(format!(
                "{rule_num}. 【扩展工具目录（按需代理调用机制）】：\n                 当原生核心工具无法满足特定业务场景时，你可通过 `call_extended_tool(tool_name: \"...\", arguments: {{...}})` 按需调用以下扩展工具：\n                 - 知识沉淀与技能管理：\n                   * `record_memory(content: string, title?: string, category?: string)`: 沉淀技术大盘与主题事实至 .harness/memory/\n                   * `read_memory(query: string, category?: string)`: 检索 .harness/memory/ 中已沉淀的记忆知识\n                   * `list_skills()`: 查看已固化项目技能名录\n                   * `run_skill(skill_name: string, args?: object)`: 运行已固化项目技能\n                 - 多模态感知与生成：\n                   * `generate_image(prompt: string, aspect_ratio?: string)`: 绘制图片、图标、LOGO 或视觉图形\n                   * `recognize_image(path: string, prompt?: string)`: 深入识别与分析本地图片内容\n                 *(调用指引：若不确定完整参数规范，可先调用原生 `lookup_tool(tool_name: \"...\")` 查询其完整 Schema，或传 `arguments: {{\"help\": true}}`；若直接调用入参有误，系统也会自动返回完整参数指导)"
            ));
            rule_num += 1;
        } else {
            rules.push(format!(
                "{rule_num}. 【扩展工具目录（按需代理调用机制）】：\n                 当原生核心工具无法满足特定业务场景时，你可通过 `call_extended_tool(tool_name: \"...\", arguments: {{...}})` 按需调用以下扩展工具：\n                 - 子任务与协作者编排：\n                   * `spawn_subagent(role: string, task: string, title?: string, subpath?: string)`: 派生专职子 Agent 并行处理大型独立任务\n                   * `wait_subagents(timeout_secs?: number)`: 等待子 Agent 汇聚\n                   * `get_subagent_status(subagent_id?: string)`: 查询子 Agent 运行状态\n                   * `stop_subagent(subagent_id: string)`: 终止指定子 Agent\n                   * `dispatch_collaborator(collaborator_id: string, task: string, context?: object)`: 按需委派任务给常驻专业协作者\n                   * `wait_collaborators(timeout_secs?: number)`: 等待协作者执行完成并获取成果汇报\n                   * `get_collaborators()`: 查询当前可用协作者名录与状态\n                 - 网页与浏览器交互：\n                   * `fetch_web_page(url: string)`: 通过 HTTP/HTTPS 获取网页并提取为 Markdown 文本\n                   * `browser_navigate(url: string)`: 打开后台 Chromium 访问网页并获取可交互元素清单 (@eX)\n                   * `browser_interact(action: string, element_id?: string, text?: string)`: 网页元素交互（点击/输入/提交）\n                   * `browser_scroll(direction: string)`: 滚动网页视口 (down/up/top/bottom)\n                   * `browser_get_debug_state()`: 获取当前页面的 Console 报错与 HTTP 异常列表\n                   * `browser_screenshot()`: 截取当前网页屏幕快照\n                   * `browser_close()`: 关闭当前浏览器会话并释放系统资源\n                 - 知识沉淀与技能管理：\n                   * `record_memory(content: string, title?: string, category?: string)`: 沉淀技术大盘与主题事实至 .harness/memory/\n                   * `read_memory(query: string, category?: string)`: 检索 .harness/memory/ 中已沉淀的记忆知识\n                   * `list_skills()`: 查看已固化项目技能名录\n                   * `run_skill(skill_name: string, args?: object)`: 运行已固化项目技能\n                 - 多模态感知与生成：\n                   * `generate_image(prompt: string, aspect_ratio?: string)`: 绘制图片、图标、LOGO 或视觉图形\n                   * `recognize_image(path: string, prompt?: string)`: 深入识别与分析本地图片内容\n                 *(调用指引：若不确定完整参数规范，可先调用原生 `lookup_tool(tool_name: \"...\")` 查询其完整 Schema，或传 `arguments: {{\"help\": true}}`；若直接调用入参有误，系统也会自动返回完整参数指导)"
            ));
            rule_num += 1;
        }

        rules.push(format!(
            "{rule_num}. 【进程生命周期安全红线（严防自杀与误杀）】：\n             - 严禁按进程名称全局通配强杀：绝对禁止执行任何 `Stop-Process -Name <name>`、`killall`、`pkill`、`taskkill /IM` 等命令；\n             - 进程管理必须精确到 PID：仅允许通过具体 PID（`Stop-Process -Id $PID` 或 `taskkill /PID $PID`）定向终止由当前会话主动启动的子进程；严禁操作任何非自身派生的宿主与系统公共进程。"
        ));
        rule_num += 1;

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
