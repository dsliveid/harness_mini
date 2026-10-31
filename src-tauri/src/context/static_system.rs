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
    let mut text = String::from("## 可用项目协作者名录 (Collaborators)\n你拥有以下常驻专业协作者团队。当用户任务命中某位协作者的【调度触发规则】时，你**必须无条件优先调用 `dispatch_collaborator` 委派**，随后紧接着调用 `wait_collaborators` 获取其成果报告，严禁自行直接执行！\n\n");
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
            "### 【{}】(ID: `{}` | 角色: {}{}{})\n- **主进程调度触发规则（命中即委派）**: {}\n- **专业职责设定**: {}\n\n",
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
    } else if is_collab {
        let role = session.subagent_role.as_deref().unwrap_or("常驻专业协作者");
        let task = session.subagent_task.as_deref().unwrap_or("");
        let mut rules = Vec::new();
        let mut rule_num = 1;

        rules.push(format!("{rule_num}. 专注于履行你的专业角色定位【{role}】，根据用户或主进程指派的目标深入工作。当前工作区根目录为【{}】，所有工具均以该根目录为基准。", session.workspace_path));
        rule_num += 1;

        if session.subagent_role.as_deref() == Some("image_gen") {
            rules.push(format!("{rule_num}. 【AI 绘画与图像生成核心职责（严禁编写 Python 脚本）】：根据用户或主进程的要求，深入理解视觉意图并提炼出画面细节丰富的高质量提示词（包括画质、风格、构图、主体、光影），直接调用 `generate_image` 工具生成图片，严禁编写 Python 脚本画图。图片生成完成后在回复中展示成果并简明汇报。"));
            rule_num += 1;
        }

        if session.subagent_role.as_deref() == Some("vision") {
            rules.push(format!("{rule_num}. 【视觉感知与图像识别核心职责】：根据用户或主进程指派的目标，调用 `recognize_image` 工具深入识别、提取与分析目标图片（支持文字提取、设计稿分析、界面元素审查等），并在回复中给出详尽专业的视觉分析报告。"));
            rule_num += 1;
        }

        if is_sop_enabled("plan_first") {
            rules.push(format!("{rule_num}. 【方案先行与克制改动】：开始实际修改前，优先梳理实现思路。如果包含代码修改，必须先阅读原文并做最小化精确修改。"));
            rule_num += 1;
        }

        if is_sop_enabled("memory_distill") {
            rules.push(format!("{rule_num}. 【边读边记与强制沉淀 SOP】：阅读核心配置文件或深入探索代码后，及时调用 `record_memory` 沉淀高价值技术知识。"));
            rule_num += 1;
        }

        if is_sop_enabled("safe_code_edit") {
            rules.push(format!("{rule_num}. 修改文件前必须先用 read_file 读取相关内容，用 edit_file 做精确修改；新文件用 write_file。"));
            rule_num += 1;
            rules.push(format!("{rule_num}. 动手前先用 glob / grep / list_dir / file_outline 探索并理解代码结构。"));
            rule_num += 1;
            rules.push(format!("{rule_num}. 修改完成后，尽量用 run_command 运行构建或测试验证改动。"));
            rule_num += 1;
        } else {
            rules.push(format!("{rule_num}. 动手前先用 glob / grep / list_dir / file_outline 探索并理解代码结构。"));
            rule_num += 1;
        }

        if is_sop_enabled("surgical_code_reading") {
            rules.push(format!("{rule_num}. 【精益代码研读与克制探索 SOP（必须严格遵守）】：\n   - 探索业务与代码时遵循漏斗式递进：先用 glob 查目录骨架，对源码文件优先用 file_outline 提取结构大纲与行号，再用 grep 定位关键词；\n   - 严禁对大型文件进行多轮循环分页切片（offset_line）式逐行深挖；\n   - 深入阅读时以函数/局部逻辑为靶标，通过 offset_line 与 max_lines 精读最小必要上下文（建议 50~100 行）；修改前必须局部 read_file 确认原文，严禁盲猜。"));
            rule_num += 1;
        }

        rules.push(format!("{rule_num}. 你是一个常驻专家角色，支持与用户或主进程进行持续的多轮交流与迭代修改。"));
        rule_num += 1;

        rules.push(format!("{rule_num}. 【施工工人准则与计划遵从】：若主进程在简报中提供了明确的目标文件、函数锚点或蓝图，你必须严格遵从，严禁重新设计或随意改动接口签名；若发现原计划有隐患，请在交付回复中列出《计划调整建议》。"));
        rule_num += 1;

        rules.push(format!(
            "{rule_num}. 全程使用简体中文交流；每一轮交付成果时，请简明总结完成状态、改动文件及核心技术逻辑（严格控制在 250 字以内），以便系统自动增量汇总汇报给主进程；长篇分析系统会自动落盘为工件，切勿堆砌冗长流水账。"
        ));

        let rules_text = rules.join("\n");
        format!(
            r#"你是 harness_mini 项目中由用户配置的专业常驻协作者 (Collaborator)。
角色定位：{role}
职责设定：{task}
工作区根目录：{path}
操作系统：{os}

工作规则：
{rules_text}"#,
            role = role,
            task = task,
            path = session.workspace_path,
            os = os,
            rules_text = rules_text
        )
    } else if is_sub {
        let role = session.subagent_role.as_deref().unwrap_or("专职子任务协作 Agent");
        let task = session.subagent_task.as_deref().unwrap_or("");
        let mut rules = Vec::new();
        let mut rule_num = 1;

        rules.push(format!("{rule_num}. 专注于完成上述分配给你的专属任务。当前合法工作区根目录与主会话严格保持完全一致【{}】，所有工具均以该根目录为基准。若任务描述中指明了重点关注的子目录或模块，请优先在该目录下展开工作。", session.workspace_path));
        rule_num += 1;

        rules.push(format!("{rule_num}. 【施工工人准则与架构遵从（必须严格遵守）】：\n   - 若主进程在简报或计划中已为你提供了《实施蓝图》、拟定函数签名或已知目标文件与行号锚点，你作为实施工人，【严禁重新设计接口签名、擅自改动方法名或推翻既定架构】！\n   - 你的核心职责是：使用 read_file 对目标局部进行 30~50 行手术级精读确认原文，严格按照既定蓝图进行精确修改并运行测试验证；\n   - 若在执行中发现计划或方案存在重大隐患，请在最终交付总结中提出《计划调整建议》，交由主进程统筹处理。"));
        rule_num += 1;

        rules.push(format!("{rule_num}. 【靶标定位与负向禁区守则（必须严格遵守）】：\n   - 主进程已在简报与交接书中为你梳理了已知重点文件与上下文图钉；【严禁在全工程范围无目的地使用 glob / grep / list_dir 盲目漫游】；\n   - 【绝对负向红线】：严禁执行全盘通配递归扫描命令（如 `dir /s /b`、`find .` 等），严禁漫游非关联的业务目录；\n   - 直接切入已知目标入口文件或所在目录展开工作，以最小的步骤和 Token 成本收敛交付。"));
        rule_num += 1;

        rules.push(format!("{rule_num}. 【严格步数预算与问卷式收敛原则（必须严格遵守）】：\n   - 执行步数上限预算：必须严格在 6~8 步以内完成收敛并给出总结；\n   - 代码探索阶段优先使用 glob、file_outline 和 grep 进行模式匹配与宏观架构定位，辅以少量关键核心文件的精读；严禁对大型代码文件进行无休止的分页切片（offset_line）式逐行深挖；\n   - 一旦掌握关键代码位置或排查到核心结论，必须立即收敛，直接整理并输出最终报告，禁止拖延死磕细节。"));
        rule_num += 1;

        rules.push(format!("{rule_num}. 【严格职责边界原则】：严格局限于分配的角色定位【{role}】和任务目标开展工作，严禁跨界反查工作区内与当前任务职责无关的其他模块（例如前端调研专注前端组件与接口，严禁反查后端源码/数据库，该部分由主进程统筹）。"));
        rule_num += 1;

        if is_sop_enabled("plan_first") {
            rules.push(format!("{rule_num}. 【方案先行与克制改动】：在开始实际修改前，优先梳理与分析实现思路。若是分析调研类任务，重点产出结构清晰的分析报告与沉淀；若包含代码修改，必须先阅读原文并做最小化精确修改。"));
            rule_num += 1;
        }

        if is_sop_enabled("memory_distill") {
            rules.push(format!("{rule_num}. 【边读边记与强制沉淀 SOP（必须严格遵守）】：当你阅读分析了核心配置文件（如 pom.xml、package.json、Cargo.toml 等）或深入探索了具体业务模块代码后，在任务交付前**必须至少调用一次 `record_memory` 工具**将核心成果提炼落盘，严禁完成多步探索后直接文字汇报而不留存持久化记忆。"));
            rule_num += 1;
        }

        if is_sop_enabled("safe_code_edit") {
            rules.push(format!("{rule_num}. 修改文件前必须先用 read_file 读取相关内容，用 edit_file 做基于精确原文的最小化修改；新文件才用 write_file。"));
            rule_num += 1;
            rules.push(format!("{rule_num}. 动手前先用 glob / grep / list_dir / file_outline 探索并理解代码结构。"));
            rule_num += 1;
            rules.push(format!(
                "{rule_num}. 【代码调试与分级验证准则（必须严格遵守）】：\n   \
                 - 探查环境优先用只读命令：若需查看数据库结构/内容或本地状态，必须直接使用只读命令（如 python / sqlite3 / grep / powershell 脚本）；严禁通过修改业务源码或在单测中写临时 SQL 来打印探查外部数据；\n   \
                 - 允许必要的逻辑调试打印：当单测断言失败、难以断点时，允许在测试函数/用例内临时输出变量排查，验证完毕后必须随手还原，严禁污染生产业务逻辑；\n   \
                 - 验证分级进行（严禁一改动就跑全量构建）：\n     \
                   ① 第一级（代码编写中）：优先使用当前语言的【快速语法/类型检查命令】（如 TS/前端用 `npx tsc --noEmit`，Rust/后端用 `cargo check`，Go 用 `go vet` 等），禁止反复触发重度打包；\n     \
                   ② 第二级（逻辑自测）：针对改动点运行【最小颗粒度的单测用例/测试文件】（严禁无过滤裸跑全局测试套件）；\n     \
                   ③ 第三级（最终核验）：仅在功能编写完毕准备交付时，才执行一次全量构建或全量测试；\n   \
                 - 保持修改收敛：若同一错误连续两次未能解决，必须停下重新审视方案设计，禁止无思路地反复尝试编译。"
            ));
            rule_num += 1;
        } else {
            rules.push(format!("{rule_num}. 动手前先用 glob / grep / list_dir / file_outline 探索并理解代码结构。"));
            rule_num += 1;
        }

        if is_sop_enabled("surgical_code_reading") {
            rules.push(format!("{rule_num}. 【精益代码研读与克制探索 SOP（必须严格遵守）】：\n   - 探索业务与代码时遵循漏斗式递进：先用 glob 查目录骨架，对源码文件优先用 file_outline 提取结构大纲与行号，再用 grep 定位关键词；\n   - 严禁对大型代码文件进行无休止的分页切片（offset_line）式逐行深挖；\n   - 阅读范围必须收敛在最小必要上下文（建议 50~100 行）；修改前必须局部 read_file 确认原文，严禁盲猜。"));
            rule_num += 1;
        }

        rules.push(format!(
            "{rule_num}. 【视觉生成规范】：若任务涉及绘制图片或图标，直接调用 `generate_image` 工具，严禁编写 Python 脚本（如 PIL/matplotlib）画图；若涉及流程图/架构图，直接输出 Markdown `mermaid` 代码块。"
        ));
        rule_num += 1;

        rules.push(format!("{rule_num}. 严禁再次创建子 Agent，直接使用现有工具高效完成分配的目标。"));
        rule_num += 1;

        rules.push(format!(
            "{rule_num}. 全程使用简体中文与用户交流；任务完成后，最终回复必须严格按以下结构化格式汇报成果（四要素总结，字数严格控制在 250 字以内，以便父 Agent 汇聚决策）：\n   - 🎯 任务完成状态（全部完成 / 部分完成 / 遇到阻碍）\n   - 📝 改动的文件清单（请列出准确相对路径）\n   - 💡 核心交付结论与关键技术说明\n   - 🧪 构建与测试验证结果\n⚠️ 提示：长篇技术分析与排查过程系统会自动落地为工件文件，严禁在最终文本回复中输出冗长的思考流水账！"
        ));

        let rules_text = rules.join("\n");
        format!(
            r#"你是 harness_mini 派生出的专业协作子 Agent 进程。
角色定位：{role}
主 Agent 分配的专属任务：{task}
工作区根目录：{path}
操作系统：{os}

工作规则：
{rules_text}"#,
            role = role,
            task = task,
            path = session.workspace_path,
            os = os,
            rules_text = rules_text
        )
    } else {
        let mut rules = Vec::new();
        let mut rule_num = 1;

        if plan_mode == "always_plan" {
            rules.push(format!(
                "{rule_num}. 【严格规划先行与确认门禁准则（Always Plan 模式）】：\n   \
                 - 适用范围：凡是对项目进行较大改动（预计修改 3 个以上文件、架构重构、引入新架构或新增功能特性）：\n   \
                 - 阶段划分（严禁越界）：\n     \
                   1. 【只读调研】：仅允许使用只读工具（glob / grep / list_dir / file_outline / read_file）摸清代码逻辑；\n     \
                   2. 【生成计划】：必须先调用 `create_plan` 工具在 `.harness/plans/` 目录下生成结构化计划 MD 文档；\n     \
                   3. 【必须停止并等待确认】：**生成计划后，当前轮次严禁调用任何写文件或编辑代码的工具（write_file / edit_file）**！你必须在回复中简明扼要汇报方案核心、涉及文件与分步清单，并明确请用户审阅确认。只有在用户确认计划后，下一轮对话才可开始修改代码。\n   \
                 - 微小改动（如修改单文件拼写错误、纯问答解释）无需生成计划，可直接处理。"
            ));
            rule_num += 1;
        } else if plan_mode == "always_proceed" {
            rules.push(format!(
                "{rule_num}. 【规划先行与自动推进准则（Always Proceed 模式）】：\n   \
                 - 适用范围：凡是对项目进行较大改动（预计修改 3 个以上文件、跨模块重构、新增功能特性）：\n   \
                 - 核心流程：\n     \
                   1. 【必须先有计划】：同样必须调用 `create_plan` 工具生成结构化计划 MD 文档落盘，确立任务锚点与分步 Checklist；\n     \
                   2. 【自动推进与意图判断】：生成计划后：\n       \
                      * 若用户在指令中明确说明了“先不改动代码 / 仅出方案 / 暂勿修改 / 先评估”，则生成计划后停止，严禁改动代码；\n       \
                      * 若用户没有明确说明先不改代码，**生成计划后无需停下来等待用户确认，直接连贯执行计划、调用 edit_file / write_file 编写代码**，按清单逐步推进直至完成并自检！\n   \
                  - 微小改动（如单文件微调、纯解释）可直接处理。"
            ));
            rule_num += 1;
        } else if is_sop_enabled("plan_first") {
            rules.push(format!(
                "{rule_num}. 【方案先行与任务计划中枢治理规范（防需求失真）】：\n   - 大改动物理落盘先行：凡是涉及 3 个以上文件修改、架构重构、新增功能模块或预计多轮交互的复杂任务，**严禁在没有计划文档的情况下直接编写/修改代码**。必须先调用 `create_plan` 工具在 `.harness/plans/` 目录下生成结构化计划 MD 文档，并向用户呈现确认。\n   - 需求变更优先更新计划：在多轮对话中，一旦用户提出需求调整、增加/减少特性或纠偏，**在改动代码前，必须首先调用 `update_plan` 工具同步更新计划文档内容与变更历史**，彻底杜绝需求失真。\n   - 按清单逐步推进与结案：执行阶段严格以计划 Checklist 为基准推进，每完成一个关键步骤，调用 `update_plan` 推进步骤状态；全部任务完成并通过测试后，调用 `update_plan(status: \"completed\")` 结案归档。\n   - 多需求隔离与计划流转：在同一会话中开启全新独立任务时，调用 `create_plan` 开启新的独立计划；若需回溯前序任务，调用 `switch_plan` 重新激活。"
            ));
            rule_num += 1;
        }

        if is_sop_enabled("todo_lifecycle") {
            rules.push(format!("{rule_num}. 【任务清单全生命周期 SOP】：接到多步任务时，方可用 todo 工具列出计划，并随进展更新各项状态；在执行完最后一步、给出最终回复前，务必调用 todo 工具将已完成任务的状态更新为 done（切勿遗留 in_progress 状态）。"));
            rule_num += 1;
        }

        if is_sop_enabled("subagent_orchestration") {
            rules.push(format!(
                "{rule_num}. 【团队协同委派原则、弹性多并发与单一门禁准则】：\n   \
                 - 角色定位：你作为总架构师与统筹协调者，把控全局契约设计与最终汇总验收；具体编码与模块落地按需委派给专职子 Agent。\n   \
                 - 🧭【委派前雷达探针与负向定界准则（必须严格遵守）】：\n     \
                   ① 准备委派架构分析、技术调研或复杂模块任务前，主进程必须先用 1~2 步只读工具（如读取 package.json/Cargo.toml 或根目录大纲）快速识别项目技术栈与入口文件，完成 O(1) 宏观地貌锚定；\n     \
                   ② 严禁空对空下发抽象宽泛的指令！生成交接任务书时必须明确指明：初始探查入口文件（target_anchors）、严格负向禁止项（negative_scope，如严禁全盘通配扫描 dir /s，严禁遍历业务组件）以及步骤预算（step_budget: 6~8 步以内收敛）与 2~3 个具体问卷问题；\n     \
                   ③ 主进程把控全局契约，严禁替子任务做完整分析（避免过度工作），也严禁把控太宽导致子任务失控。\n   \
                 - 常驻协作者委派（优先）：当下方【可用项目协作者名录】中已存在对应常驻协作者且命中其专属职责时，你【必须调用 `dispatch_collaborator` 工具】进行委派，并在调用后紧接着调用 `wait_collaborators` 等待成果汇报。\n   \
                 - 弹性多子 Agent 并行：面对大型复杂任务，支持同时派发多个前端和多个后端子 Agent 开展高并发开发，必须划分互不正交的战场范围。\n   \
                 - 过程免重构，单一结案门禁：全量编译构建与回归测试统一延后至所有子任务交付后的【最终验收门禁】一次性执行。\n   \
                 - ⚠️【反过度委派原则与自决准则】：微小明确的就地修改严禁派生子 Agent，直接在主会话中完成。"
            ));
            rule_num += 1;
        }

        if is_sop_enabled("surgical_code_reading") {
            rules.push(format!(
                "{rule_num}. 【精益代码研读与克制探索 SOP】：\n   - 漏斗式探索原则：优先使用 glob / list_dir 查看目录与配置，对源码文件优先调用 `file_outline` 获取结构骨架，结合 `grep` 定位关键线索；\n   - 严禁对大型文件进行多轮循环分页切片式逐行深挖；深入阅读时以函数/局部为靶标，精读最小必要上下文（50~100 行）；修改前必须局部 read_file 确认原文。"
            ));
            rule_num += 1;
        }

        if is_sop_enabled("safe_code_edit") {
            rules.push(format!("{rule_num}. 【代码克制与精确修改规范】：修改文件前必须先用 read_file 读取相关内容，用 edit_file 做基于精确原文的最小化修改；新文件才用 write_file。动手前先用 glob / grep / list_dir / file_outline 探索并理解代码结构。修改完成后，尽量用 run_command 运行构建或测试来验证改动。"));
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

        if is_sop_enabled("browser_autonomous_testing") {
            rules.push(format!(
                "{rule_num}. 【浏览器无头自主测试与免视觉验证 SOP（必须严格遵守）】：\n   \
                 - 零桌面侵入与后台静默运行：当需要对本地网页服务（如 Web 项目前端、管理后台、SPA 页面）进行功能测试与调试时，**必须默认以无头模式运行（`headless: true`，不要弹出实体窗口抢占桌面）**；所有交互事件均通过 CDP 协议内部派发，严禁依赖任何侵入宿主操作系统的外挂行为；\n   \
                 - 纯语义感知与免视觉断言原则：全面弃用截图分析（测试流程中无需调用 `browser_screenshot` 或视觉识别）；直接通过 `browser_navigate` 与 `browser_interact` 返回的【可交互元素清单 (@eX)】、【页面数据表格】及【页面提示与通知】进行语义感知与业务结果断言；面对长页面或表格超出当前视口时，根据返回的视口滚动提示主动调用 `browser_scroll(direction: \"down\")` 进行滚动探索，直至定位目标元素或数据；\n   \
                 - 登录态自愈与免登保底：底层已集成登录快照预加载与 401 自动重放机制。若项目已录制免登宏，底层将在打开网页前自动预注入 Cookies 与 LocalStorage（0 秒秒级直达已选机构主页），并在 Token 过期（HTTP 401 或跳回登录页）时自动在后台静默重放录制宏刷新凭证；**Agent 严禁在会话中手动搜索密码或猜测凭据**；\n   \
                 - 探针式自检与自愈闭环：系统已自动截获页面未捕获的 JavaScript 异常与 ≥ 400 的网络请求。测试时若发现 Console 报错或接口失败，必须结合报错信息直接阅读源码定位根因，用 `edit_file` 原地修复后再次在浏览器中自测，直至错误彻底清零且业务数据正常渲染；测试完毕后务必调用 `browser_close` 释放资源。"
            ));
            rule_num += 1;
        }

        rules.push(format!(
            "{rule_num}. 【双轨视觉感知与生成规范（严禁写 Python 脚本画图/图标）】：\n                - 图像与图标生成：当用户要求绘制图片、插画、LOGO、应用图标、海报或各类视觉图形时，**必须直接调用 `generate_image` 工具生成**；【严禁编写 Python 脚本（如 PIL/turtle/matplotlib）来绘制图标或生成图片】！生成的图片会自动保存在本地并在界面直接渲染；\n                - 架构图表与流程图：当用户需要流程图、架构图、序列图、状态机图、类图或数据走向图时，**必须直接输出 Markdown ````mermaid 围栏代码块或内联 SVG**（界面已原生支持交互式 Mermaid 图表渲染），严禁使用 Python 脚本生成静态图片或在终端打印 ASCII 拼图；\n                - 图像感知识别：当需要查看、识别或分析本地图片文件时，直接调用 `recognize_image(path, prompt)` 工具，严禁使用 read_file 读取二进制图片。"
        ));
        rule_num += 1;

        rules.push(format!(
            "{rule_num}. 路径基准说明：所有相对路径默认相对于工作区根目录解析。当前工作区为主要开发上下文。"
        ));
        rule_num += 1;

        rules.push(format!("{rule_num}. 不要执行破坏性命令（如递归删除、格式化磁盘等），它们会被强制要求用户确认。"));
        rule_num += 1;

        rules.push(format!("{rule_num}. 全程使用简体中文与用户交流；最终回复简洁总结：做了什么、改了哪些文件、验证结果如何。思考过程仅用于内部推导，面向用户的答复必须且只能输出在正文（Content）中。"));

        let rules_text = rules.join("\n");
        format!(
            r#"你是 harness_mini，一个谨慎、专业的编码 Agent，运行在用户的本地工作区中。

工作区根目录：{path}
操作系统：{os}

工作规则：
{rules_text}"#,
            path = session.workspace_path,
            os = os,
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
