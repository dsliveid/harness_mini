use crate::models::Session;
use serde::{Deserialize, Serialize};

/// 渲染上下文：一次性算好，供所有变体判定条件使用
pub struct SopContext<'a> {
    pub session: &'a Session,
    pub is_sub: bool,
    pub is_collab: bool,
    pub plan_mode: &'a str,
}

impl<'a> SopContext<'a> {
    pub fn for_session(session: &'a Session, plan_mode: &'a str) -> Self {
        let is_collab = session.session_type == "collaborator";
        let is_sub = is_collab
            || session.session_type == "subprocess"
            || session.session_type == "subagent"
            || session.parent_session_id.is_some();
        SopContext {
            session,
            is_sub,
            is_collab,
            plan_mode,
        }
    }
}

/// 一条规则的一个具体变体（= 提示词中真实可能出现的一种形态）
pub struct SopVariant {
    /// 「主进程」「子进程」「Always Plan」等，仅用于界面对比
    pub label: &'static str,
    /// 提示词里的真标题（【】内的文字）
    pub title: &'static str,
    /// 提示词正文（【】：之后的部分，保留原有换行与前缀）；`{report_file}` 为子任务报告文件名占位符
    pub body: &'static str,
    /// 该变体是否在本次渲染中生效
    pub active: fn(&SopContext) -> bool,
}

/// 内部声明用的元数据（不含 name：界面标题由 variants[0].title 派生，全项目唯一真源）
pub struct SopMeta {
    pub id: &'static str,
    pub phase: u8,
    pub phase_label: &'static str,
    pub category: &'static str,
    pub category_label: &'static str,
    pub scope: &'static str, // "universal" | "main_only" | "sub_only"
    pub scope_label: &'static str,
    pub is_hard_rule: bool,
    pub description: &'static str,
    pub disable_effect: &'static str,
}

pub struct SopDefinition {
    pub item: SopMeta,
    pub variants: Vec<SopVariant>,
}

/// 单个变体的对外视图（text 为提示词真实原文，不含序号）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SopVariantView {
    pub label: String,
    pub title: String,
    pub text: String,
}

/// 对外目录条目：元数据 + 由 variants 派生的 name 与变体视图（前端设置界面直接渲染）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SopItem {
    pub id: String,
    pub name: String,
    pub phase: u8,
    pub phase_label: String,
    pub category: String,
    pub category_label: String,
    pub scope: String,
    pub scope_label: String,
    pub is_hard_rule: bool,
    pub description: String,
    pub disable_effect: String,
    pub variants: Vec<SopVariantView>,
}

/// 获取全局 SOP 数据字典目录（作为前后端单一真源）
pub fn get_sop_catalog() -> Vec<SopItem> {
    get_all_sop_definitions()
        .into_iter()
        .map(|def| {
            let SopDefinition { item, variants } = def;
            SopItem {
                id: item.id.into(),
                name: variants[0].title.into(),
                phase: item.phase,
                phase_label: item.phase_label.into(),
                category: item.category.into(),
                category_label: item.category_label.into(),
                scope: item.scope.into(),
                scope_label: item.scope_label.into(),
                is_hard_rule: item.is_hard_rule,
                description: item.description.into(),
                disable_effect: item.disable_effect.into(),
                variants: variants
                    .iter()
                    .map(|v| SopVariantView {
                        label: v.label.into(),
                        title: v.title.into(),
                        text: v.body.into(),
                    })
                    .collect(),
            }
        })
        .collect()
}

/// 用「N. 【标题】：正文」格式渲染单个变体；正文占位符统一在此替换
fn render_variant(rule_num: usize, variant: &SopVariant, ctx: &SopContext) -> String {
    let body = variant
        .body
        .replace("{report_file}", &format!("{}_report.md", ctx.session.id));
    format!("{rule_num}. 【{}】：{}", variant.title, body)
}

/// 构建会话实际生效的 SOP 规则列表（作用域过滤 → 开关过滤 → 取第一个命中变体）
pub fn build_sop_rules(
    session: &Session,
    disabled_sops: &[String],
    plan_mode: &str,
    mut rule_num: usize,
) -> (Vec<String>, usize) {
    let ctx = SopContext::for_session(session, plan_mode);
    let mut rules = Vec::new();

    for def in get_all_sop_definitions() {
        // ① 作用域硬性过滤：确保主会话与子进程适用的状态绝不混淆
        if ctx.is_sub && def.item.scope == "main_only" {
            continue;
        }
        if !ctx.is_sub && def.item.scope == "sub_only" {
            continue;
        }

        // ② 开关判断：非硬规则且被禁用时跳过
        if !def.item.is_hard_rule && disabled_sops.iter().any(|d| d == &def.item.id) {
            continue;
        }

        // ③ 从 variants 中取第一个 active 命中项
        if let Some(v) = def.variants.iter().find(|v| (v.active)(&ctx)) {
            rules.push(render_variant(rule_num, v, &ctx));
            rule_num += 1;
        }
    }

    (rules, rule_num)
}

fn get_all_sop_definitions() -> Vec<SopDefinition> {
    vec![
        // ------------------ 层级 1：前置意图与写入门禁 ------------------
        SopDefinition {
            item: SopMeta {
                id: "todo_first_gate",
                phase: 1,
                phase_label: "第一层级 · 前置意图与写入门禁",
                category: "thinking",
                category_label: "思考范式",
                scope: "universal",
                scope_label: "主/子对话可用",
                is_hard_rule: false,
                description: "动代码前必须先声明施工意图（小型任务调用 todo，复杂工程调用 create_plan，二选一即可）；只读勘察严格限制在 ≤8 步以内，查清直接答复；若达 8 步仍无法梳理意图必须立即停下向用户/父进程汇报事实并对齐阻碍；落盘计划与子进程施工统一遵守此项门禁。",
                disable_effect: "禁用后：Agent 免除动代码前的强制 todo 立项约束，且不触发 8 步勘察上限熔断，可自由读写探索。",
            },
            variants: vec![
                SopVariant {
                    label: "主进程",
                    title: "Todo 先行与阶梯门禁公理（所有交互必须严格遵守）",
                    body: "\n\
- 门禁 ①【Todo 先行与写入前置拦截】：\n\
* 严禁在未声明意图（无 active todo 且无 active plan）的情况下擅自调用 `write_file` 或 `edit_file` 修改项目文件；\n\
* 只要涉及修改代码/文件，动工前必须先声明施工意图（小型微调调用 `todo` 工具声明任务列表，复杂架构工程调用 `create_plan` 工具生成实施方案，二选一即可，严禁重复立项）；\n\
* 纯咨询、诊断排查或问答任务同样允许调用 `todo` 工具梳理勘察要点与任务清单（咨询往往需要深入了解项目），但未经用户明确要求修改，严禁擅自修改文件。\n\
- 门禁 ②【≤8 步勘察上限与目标对齐（防盲目漫游）】：\n\
* 勘察排查类只读工具（glob / list_dir / read_file / grep / file_outline）调用累计不大于 8 步；\n\
* 若在 8 步内已查清事实且无需修改代码，立即在正文中完整回复用户并结案；\n\
* 若在 8 步内明确需要修改代码，必须先声明意图（小型调整/探索调用 `todo` 工具声明任务列表，复杂跨模块工程调用 `create_plan` 工具生成实施方案，二选一即可，严禁重复创建），方可进行代码修改并按清单推进；\n\
* 🚨 熔断停下：若工具调用达到第 8 步仍未能梳理出清晰的意图声明（todo 列表或 plan 方案），严禁凭空脑补计划、严禁继续只读漫游，必须立即停止调用工具，向用户汇报当前事实并对齐目标！\n\
- 门禁 ③【已有计划豁免】：\n\
* 若当前会话已存在用户明确确认的落盘方案（active_plan_id 有效），因其已持有明确的任务蓝图，天然免除重复创建 todo 的要求，严格遵从既定蓝图推进。",
                    active: |ctx| !ctx.is_sub,
                },
                SopVariant {
                    label: "子进程",
                    title: "Todo 先行与阶梯门禁公理（所有交互必须严格遵守）",
                    body: "\n\
- 门禁 ①【Todo 先行与写入前置拦截】：\n\
* 严禁在未声明意图（无 active todo 且无 active plan）的情况下擅自调用 `write_file` 或 `edit_file` 修改项目代码；\n\
* 只要涉及修改代码，动工前必须先声明意图（小型任务调用 `todo` 工具声明步骤清单，复杂架构任务调用 `create_plan` 工具生成方案，二选一即可）；\n\
* 纯探索调研、排查诊断或问答任务：未经明确要求修改代码，严禁调用 todo，更严禁擅自修改文件；写入交付报告 `.harness/subtasks/{report_file}` 天然豁免 todo 门禁。\n\
- 门禁 ②【≤8 步勘察上限与目标对齐（防盲目漫游）】：\n\
* 勘察排查类只读工具（glob / list_dir / read_file / grep / file_outline）调用累计不大于 8 步；\n\
* 若在 8 步内已查清事实且无需修改代码，立即调用 `write_file` 写入交付报告 `.harness/subtasks/{report_file}` 并结案；\n\
* 若在 8 步内明确需要修改代码，必须先声明意图（小型任务调用 `todo` 工具立项，复杂架构任务调用 `create_plan` 工具生成方案，二选一即可，严禁重复创建），再开始编写代码并按清单推进；\n\
* 🚨 熔断停下：若工具调用达到第 8 步仍未能梳理出清晰的意图声明（todo 施工清单或 plan 方案），或发现前置条件严重缺失，严禁凭空脑补计划、严禁继续只读漫游，必须立即调用 `write_file` 写入交付报告向主进程汇报当前遇到的具体阻碍并交回决策权！",
                    active: |ctx| ctx.is_sub,
                },
            ],
        },

        // ------------------ 层级 2：工程规划与落地执行 ------------------
        SopDefinition {
            item: SopMeta {
                id: "plan_first",
                phase: 2,
                phase_label: "第二层级 · 工程规划与落地执行",
                category: "thinking",
                category_label: "思考范式",
                scope: "universal",
                scope_label: "主/子对话可用",
                is_hard_rule: false,
                description: "面对大改动任务（3个以上文件）或多轮需求，在 .harness/plans/ 自动建立结构化计划 MD 文档；需求变更时优先更新计划，后续严格根据计划分步执行，防止多轮需求失真与遗忘；若已有落盘计划，天然满足代码写入门禁。",
                disable_effect: "禁用后：Agent 接到任务后可直接编写代码，不强制在 .harness/plans/ 建立结构化方案文档。",
            },
            variants: vec![
                SopVariant {
                    label: "Always Plan",
                    title: "严格规划先行与确认门禁准则（Always Plan 模式）",
                    body: "\n\
- 适用范围：凡是对项目进行较大改动（预计修改 3 个以上文件、架构重构、引入新架构或新增功能特性）：\n\
- 阶段划分（严禁越界）：\n\
1. 【只读调研】：仅允许使用只读工具（glob / grep / list_dir / file_outline / read_file）摸清代码逻辑；\n\
2. 【生成计划】：必须先调用 `create_plan` 工具在 `.harness/plans/` 目录下生成结构化计划 MD 文档（已建方案无需重复创建 todo）；\n\
3. 【必须停止并等待确认】：**生成计划后，当前轮次严禁调用任何写文件或编辑代码的工具（write_file / edit_file）**！你必须在回复中简明扼要汇报方案核心、涉及文件与分步清单，并明确请用户审阅确认。只有在用户确认计划后，下一轮对话才可开始修改代码。\n\
- 微小改动（如修改单文件拼写错误、纯问答解释）无需生成计划，可直接处理。",
                    active: |ctx| !ctx.is_sub && ctx.plan_mode == "always_plan",
                },
                SopVariant {
                    label: "Always Proceed",
                    title: "规划先行与自动推进准则（Always Proceed 模式）",
                    body: "\n\
- 适用范围：凡是对项目进行较大改动（预计修改 3 个以上文件、跨模块重构、新增功能特性）：\n\
- 核心流程：\n\
1. 【必须先有计划】：同样必须调用 `create_plan` 工具生成结构化计划 MD 文档落盘，确立任务锚点与分步 Checklist（已建方案无需重复创建 todo）；\n\
2. 【自动推进与意图判断】：生成计划后：\n\
* 若用户在指令中明确说明了“先不改动代码 / 仅出方案 / 暂勿修改 / 先评估”，则生成计划后停止，严禁改动代码；\n\
* 若用户没有明确说明先不改代码，**生成计划后无需停下来等待用户确认，直接连贯执行计划、调用 edit_file / write_file 编写代码**，按清单逐步推进直至完成并自检！\n\
- 微小改动（如单文件微调、纯解释）可直接处理。",
                    active: |ctx| ctx.plan_mode == "always_proceed" || ctx.is_sub,
                },
                SopVariant {
                    label: "标准模式",
                    title: "方案先行与任务计划中枢治理规范（防需求失真）",
                    body: "\n\
- 大改动物理落盘先行：凡是涉及核心业务架构重构、多模块接口改造或预计修改 3 个以上核心文件的复杂任务，直接调用 `create_plan` 生成结构化方案文档；已有方案天然豁免 todo，执行阶段严格以方案 Checklist 为基准推进。\n\
- 需求变更优先更新计划：在多轮对话中，一旦用户提出需求调整或纠偏，改动代码前必须首先调用 `update_plan` 同步更新计划文档，彻底杜绝需求失真。\n\
- 按清单逐步推进与结案：执行阶段严格以计划 Checklist 为基准推进；全部任务完成并通过测试后，调用 `update_plan(status: \"completed\")` 结案归档。",
                    active: |ctx| !ctx.is_sub && ctx.plan_mode != "always_plan",
                },
            ],
        },

        SopDefinition {
            item: SopMeta {
                id: "todo_lifecycle",
                phase: 2,
                phase_label: "第二层级 · 工程规划与落地执行",
                category: "workflow",
                category_label: "任务规划",
                scope: "universal",
                scope_label: "主/子对话可用",
                is_hard_rule: false,
                description: "针对小的调整和探索，可以调用 todo 工具列出任务列表，并随进展更新各项任务状态；在执行完最后一步、给出最终回复前，务必调用 todo 工具将已完成任务的状态更新为 done，切勿遗留 in_progress 状态。",
                disable_effect: "禁用后：允许 Agent 自由多步推进，不强制调用 todo 工具维护状态清单。",
            },
            variants: vec![SopVariant {
                label: "通用",
                title: "任务清单全生命周期 SOP",
                body: "针对小的调整和探索，可以调用 todo 工具列出任务列表，并随进展更新各项任务状态；在执行完最后一步、给出最终回复前，务必调用 todo 工具将已完成任务的状态更新为 done，切勿遗留 in_progress 状态。",
                active: |_| true,
            }],
        },

        // 子进程协作编排（仅主进程）- 修复：去除总架构师绝对推诿设定，融入用户指定的打回纪律
        SopDefinition {
            item: SopMeta {
                id: "subagent_orchestration",
                phase: 2,
                phase_label: "第二层级 · 工程规划与落地执行",
                category: "orchestration",
                category_label: "协同体系",
                scope: "main_only",
                scope_label: "仅主对话",
                is_hard_rule: false,
                description: "面对复杂需求、多模块并发开发（如前后端分离、多独立模块并行）时，可按职责派生专职子 Agent 协同推进。",
                disable_effect: "禁用后：停用多子进程协作编排指引，Agent 倾向于在单主进程内直接就地处理全部任务。",
            },
            variants: vec![SopVariant {
                label: "主进程",
                title: "团队协同委派原则、弹性多并发与职责拆分准则",
                body: "\n\
- 面对跨模块、多领域（如前后端并行开发、独立脚本与界面改造等）大体量任务时，可按需调用 `spawn_subagent` 委派专职子 Agent 并行推进。\n\
- 🎯【按职责拆分与工作量控制原则（必须严格遵守）】：\n\
① 【按职责拆分，严禁按步数生硬切分】：子任务拆分必须严格按照系统架构、关注点分离与模块职责进行拆解（例如前端 UI 交互、后端接口逻辑、数据持久化、测试编写等），严禁按“第 1~50 步做什么、第 51~100 步做什么”这种步数阶段来切分任务；\n\
② 【工作量尽量控制在 50 步以内】：主任务在划分每个职责子任务的边界时，应合理评估工作量，将其尽量控制在 50 步工具调用以内能够独立闭环的原子体量；若某一职责体量过大，应进一步按子职责细分，以便派生更多并行子任务；\n\
③ ⚠️【无需主任务提醒子任务步数】：主任务在下发任务描述（task）时，切勿向子任务提及或限制 50 步的范围，子任务的记步器会自动把控；主任务只需专注于讲透业务职责、代码范围（subpath）、接口契约与验收交付标准；\n\
- 🧭【委派前雷达探针与定界规范】：准备委派前，主进程先用 1~2 步只读工具快速识别技术栈与入口文件，完成宏观地貌锚定；交接书需明确初始探查入口（target_anchors）与严格负向禁止项（negative_scope，如严禁全盘通配扫描 dir /s）；\n\
- 🚨【自检验收与打回纪律】：自检验收发现子任务交付的代码有缺陷时，主架构师调用 `reject_subagent` 打回返工，非必要情况禁止私自编码。\n\
- 弹性多子 Agent 并行：面对大型复杂任务，支持同时派发多个前端和多个后端子 Agent 开展高并发开发，必须划分互不正交的战场范围。",
                active: |_| true,
            }],
        },

        // 子任务交付规范（仅子任务）
        SopDefinition {
            item: SopMeta {
                id: "subagent_deliver",
                phase: 2,
                phase_label: "第二层级 · 工程规划与落地执行",
                category: "orchestration",
                category_label: "协同体系",
                scope: "sub_only",
                scope_label: "子对话可用",
                is_hard_rule: false,
                description: "专职子任务在完成施工、纯调研结案或遇到重大阻碍时，必须调用 write_file 工具向黑板文件 .harness/subtasks/{id}_report.md 正式交付结构化成果。落盘后子会话立即优雅结案。",
                disable_effect: "子对话专属交付规范，主对话天然跳过。",
            },
            variants: vec![SopVariant {
                label: "子任务",
                title: "纯黑板交付规范与结案硬性标准",
                body: "\n\
- 任务目标、验收标准与负向禁区已在下方【黑板交接书】中权威定义；\n\
- 无论任务是全部完成、部分完成或遇到重大阻碍，【唯一硬性结案标准】是调用 `write_file` 工具将结构化交付报告写入 `.harness/subtasks/{report_file}`：\n\
* 报告内容需详尽记录：执行结论、事实依据、架构/时序图（Mermaid）、涉及改动的文件列表及自检测试输出；\n\
* 若任务受阻，需在报告中明确陈述阻碍原因与前置缺失；\n\
- 写入交付报告后，在对话正文中简短总结交付结论即可自然结束；严禁仅输出文本回复而不落盘报告；\n\
- 【单层执行准则】：严禁再次派生子 Agent，直接使用现有工具高效完成分配的目标。",
                active: |_| true,
            }],
        },

        // 子任务实施工人准则（仅子任务，非常驻协作者）
        SopDefinition {
            item: SopMeta {
                id: "subagent_worker_blueprint",
                phase: 2,
                phase_label: "第二层级 · 工程规划与落地执行",
                category: "orchestration",
                category_label: "协同体系",
                scope: "sub_only",
                scope_label: "子对话可用",
                is_hard_rule: false,
                description: "子任务作为实施工人，严格遵从主进程蓝图进行精确修改与自检，严禁擅自改动方法签名、接口名或推翻既定全局设计。",
                disable_effect: "子对话专属准则，主对话天然跳过。",
            },
            variants: vec![SopVariant {
                label: "专职子任务",
                title: "施工工人准则与架构遵从（必须严格遵守）",
                body: "\n\
- 若主进程在简报或交接书中提供了《实施蓝图》、拟定函数签名或目标文件与行号锚点，你作为实施工人，【严禁重新设计接口签名、擅自改动方法名或推翻既定架构】；\n\
- 你的核心职责是：使用 read_file 对目标局部精读确认原文，严格按照既定蓝图进行精确修改并运行测试验证；\n\
- 若在执行中发现计划或方案存在重大隐患，请在交付报告 `.harness/subtasks/{report_file}` 中提出《计划调整建议》，交由主进程统筹处理。",
                active: |ctx| ctx.is_sub && !ctx.is_collab,
            }],
        },

        SopDefinition {
            item: SopMeta {
                id: "surgical_code_reading",
                phase: 2,
                phase_label: "第二层级 · 工程规划与落地执行",
                category: "quality",
                category_label: "工程质量",
                scope: "universal",
                scope_label: "主/子对话可用",
                is_hard_rule: false,
                description: "探索业务与代码时遵循漏斗式渐进探索（目录/清单 ➔ file_outline 骨架 ➔ grep 线索 ➔ 局部靶向精读）；严禁对大型文件进行多轮无休止的分页切片（offset_line）式逐行通读；修改前只精读最小必要上下文，严禁凭空盲猜。",
                disable_effect: "禁用后：允许 Agent 自由阅读大段源码，解除对连续分页通读与精益读取的强约束。",
            },
            variants: vec![SopVariant {
                label: "通用",
                title: "精益代码研读与规范探索 SOP",
                body: "\n\
- 漏斗式探索原则：优先使用 glob / list_dir 查看目录与配置，对源码文件优先调用 `file_outline` 获取结构骨架，结合 `grep` 定位关键线索；\n\
- 项目源码、配置文件阅读优先使用原生工具（read_file、grep、file_outline），深入阅读时以函数/局部为靶标，结合 offset_line 与 max_lines 阅读必要上下文（建议 100~500 行）；修改前必须局部 read_file 确认原文；\n\
- 仅在复杂数据分析或系统底层勘察时允许编写脚本探查，切勿为了读取普通代码文件而舍近求远编写临时脚本。",
                active: |_| true,
            }],
        },

        SopDefinition {
            item: SopMeta {
                id: "safe_code_edit",
                phase: 2,
                phase_label: "第二层级 · 工程规划与落地执行",
                category: "quality",
                category_label: "工程质量",
                scope: "universal",
                scope_label: "主/子对话可用",
                is_hard_rule: false,
                description: "修改文件前必须先用 read_file 读取相关内容，用 edit_file 做基于精确原文的最小化修改；新文件才用 write_file；动手前探索代码结构，修改后尽量用 run_command 运行构建或测试验证。",
                disable_effect: "禁用后：移除代码最小化精确修改与构建验证的强指引约束。",
            },
            variants: vec![SopVariant {
                label: "通用",
                title: "代码克制与精确修改规范",
                body: "\n\
- 修改文件前必须先用 read_file 读取相关内容，用 edit_file 做基于精确原文的最小化修改；新文件才用 write_file；\n\
- 动手前先用 glob / grep / list_dir / file_outline 探索并理解代码结构；\n\
- 修改完成后，尽量用 run_command 运行构建或测试来验证改动。",
                active: |_| true,
            }],
        },

        SopDefinition {
            item: SopMeta {
                id: "memory_distill",
                phase: 2,
                phase_label: "第二层级 · 工程规划与落地执行",
                category: "memory",
                category_label: "知识资产",
                scope: "universal",
                scope_label: "主/子对话可用",
                is_hard_rule: false,
                description: "内置工作区长期认知记忆（.harness/memory/）；常见技术栈提问优先秒级召回；深入探索或阅读多个文件后强制调用 record_memory 沉淀技术大盘与主题碎记；对话完成后台自动萃取 digests。",
                disable_effect: "禁用后：移除强制调用 record_memory 沉淀约束，且会话结束后不触发后台自动提炼。",
            },
            variants: vec![SopVariant {
                label: "通用",
                title: "边读边记与强制沉淀 SOP",
                body: "工作区内置了长期认知记忆系统（存储于 `.harness/memory/`）。下方已自动载入本项目沉淀的技术大盘与工程规范。阅读分析核心代码后，在任务交付前通过 `record_memory` 沉淀高价值事实。",
                active: |_| true,
            }],
        },

        // ------------------ 层级 3：系统交互与安全红线 ------------------
        SopDefinition {
            item: SopMeta {
                id: "visual_artifact",
                phase: 3,
                phase_label: "第三层级 · 系统交互与安全红线",
                category: "quality",
                category_label: "工程质量",
                scope: "universal",
                scope_label: "主/子对话可用",
                is_hard_rule: true,
                description: "图像与海报必须调用 generate_image 工具；架构/流程图必须直接输出 Markdown Mermaid 代码块，严禁写 Python 脚本绘制静态图。",
                disable_effect: "系统级质量规范，常驻生效。",
            },
            variants: vec![SopVariant {
                label: "通用",
                title: "双轨视觉感知与生成规范（严禁写 Python 脚本画图/图标）",
                body: "\n\
- 图像与图标生成：当用户要求绘制图片、插画、LOGO、应用图标、海报或各类视觉图形时，**必须直接调用 `generate_image` 工具生成**；【严禁编写 Python 脚本（如 PIL/turtle/matplotlib）来绘制图标或生成图片】！生成的图片会自动保存在本地并在界面直接渲染；\n\
- 架构图表与流程图：当用户需要流程图、架构图、序列图、状态机图、类图或数据走向图时，**必须直接输出 Markdown ````mermaid 围栏代码块或内联 SVG**（界面已原生支持交互式 Mermaid 图表渲染），严禁使用 Python 脚本生成静态图片或在终端打印 ASCII 拼图；\n\
- 图像感知识别：当需要查看、识别或分析本地图片文件时，直接调用 `recognize_image(path, prompt)` 工具，严禁使用 read_file 读取二进制图片。",
                active: |_| true,
            }],
        },

        SopDefinition {
            item: SopMeta {
                id: "process_safety_boundary",
                phase: 3,
                phase_label: "第三层级 · 系统交互与安全红线",
                category: "safety",
                category_label: "安全底线",
                scope: "universal",
                scope_label: "主/子对话可用",
                is_hard_rule: true,
                description: "严禁按进程名称全局通配强杀（如 killall/taskkill /IM），进程管理必须精确到具体 PID，严禁误杀宿主与外部系统进程。",
                disable_effect: "核心安全红线，常驻生效，不可关闭。",
            },
            variants: vec![SopVariant {
                label: "通用",
                title: "进程生命周期安全红线（严防自杀与误杀）",
                body: "\n\
- 严禁按进程名称全局通配强杀：绝对禁止执行任何 `Stop-Process -Name <name>`、`killall`、`pkill`、`taskkill /IM` 等命令；\n\
- 进程管理必须精确到 PID：仅允许通过具体 PID（`Stop-Process -Id $PID` 或 `taskkill /PID $PID`）定向终止由当前会话主动启动的子进程；严禁操作任何非自身派生的宿主与系统公共进程。",
                active: |_| true,
            }],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Session;

    fn mk_session(session_type: &str, parent: Option<&str>) -> Session {
        Session {
            id: "sess01".into(),
            title: "t".into(),
            workspace_path: "D:\\ws".into(),
            session_type: session_type.into(),
            parent_session_id: parent.map(|p| p.to_string()),
            ..Default::default()
        }
    }

    fn render(session: &Session, plan_mode: &str) -> String {
        build_sop_rules(session, &[], plan_mode, 1).0.join("\n")
    }

    /// 防膨胀护栏：基准字符数 = 重构前逐字快照的实测值；提示词任何增长都会在此失败
    #[test]
    fn prompt_snapshot_not_bloated() {
        let main = mk_session("root", None);
        let sub = mk_session("subagent", Some("p"));
        let collab = mk_session("collaborator", None);
        let cases = [
            ("主进程/standard", &main, "standard", 3300usize),
            ("主进程/always_plan", &main, "always_plan", 3450),
            ("主进程/always_proceed", &main, "always_proceed", 3350),
            ("子进程/standard", &sub, "standard", 3200),
            ("协作者/standard", &collab, "standard", 2950),
        ];
        for (name, session, mode, baseline) in cases {
            let text = render(session, mode);
            assert!(
                text.chars().count() <= baseline,
                "提示词[{name}]膨胀：{} 字符 > 基准 {baseline}",
                text.chars().count()
            );
        }
    }

    /// 提示词关键标题锚点 + 变体互斥 + 占位符替换
    #[test]
    fn prompt_key_anchors_present() {
        let main = mk_session("root", None);
        let sub = mk_session("subagent", Some("p"));

        let std_text = render(&main, "standard");
        for anchor in [
            "【Todo 先行与阶梯门禁公理（所有交互必须严格遵守）】",
            "【方案先行与任务计划中枢治理规范（防需求失真）】",
            "【任务清单全生命周期 SOP】",
            "【团队协同委派原则、弹性多并发与职责拆分准则】",
            "【边读边记与强制沉淀 SOP】",
            "【进程生命周期安全红线（严防自杀与误杀）】",
        ] {
            assert!(std_text.contains(anchor), "锚点缺失：{anchor}");
        }

        let sub_text = render(&sub, "standard");
        assert!(sub_text.contains("【纯黑板交付规范与结案硬性标准】"));
        assert!(sub_text.contains("【施工工人准则与架构遵从（必须严格遵守）】"));
        assert!(
            sub_text.contains(".harness/subtasks/sess01_report.md"),
            "报告文件名占位符未被正确替换"
        );
        assert!(!sub_text.contains("{report_file}"), "占位符残留原文");

        let plan_text = render(&main, "always_plan");
        assert!(plan_text.contains("【严格规划先行与确认门禁准则（Always Plan 模式）】"));
        assert!(!plan_text.contains("Always Proceed 模式"));

        let proceed_text = render(&main, "always_proceed");
        assert!(proceed_text.contains("【规划先行与自动推进准则（Always Proceed 模式）】"));
        assert!(!proceed_text.contains("Always Plan 模式"));
    }

    /// 名字同源：每条规则的 variants[0].title 必然出现在其生效场景的提示词中
    #[test]
    fn card_title_always_appears_in_prompt() {
        let main = mk_session("root", None);
        let sub = mk_session("subagent", Some("p"));
        let collab = mk_session("collaborator", None);
        let pairs = [
            (
                SopContext::for_session(&main, "standard"),
                render(&main, "standard"),
            ),
            (
                SopContext::for_session(&main, "always_plan"),
                render(&main, "always_plan"),
            ),
            (
                SopContext::for_session(&main, "always_proceed"),
                render(&main, "always_proceed"),
            ),
            (
                SopContext::for_session(&sub, "standard"),
                render(&sub, "standard"),
            ),
            (
                SopContext::for_session(&collab, "standard"),
                render(&collab, "standard"),
            ),
        ];
        for def in get_all_sop_definitions() {
            let v0 = &def.variants[0];
            // 名字同源：v0 的标题必须在全部上下文渲染结果的并集中出现
            // （配合 every_variant_is_reachable 保证 v0 至少在一个上下文中真实生效）
            let combined = pairs
                .iter()
                .map(|(_, text)| text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                combined.contains(v0.title),
                "{} 的界面标题「{}」未出现在任何提示词中（名字不同源）",
                def.item.id,
                v0.title
            );
        }
    }

    /// 无死项：每个变体都必须至少在一种上下文中生效
    #[test]
    fn every_variant_is_reachable() {
        let main = mk_session("root", None);
        let sub = mk_session("subagent", Some("p"));
        let collab = mk_session("collaborator", None);
        let ctxs = [
            SopContext::for_session(&main, "standard"),
            SopContext::for_session(&main, "always_plan"),
            SopContext::for_session(&main, "always_proceed"),
            SopContext::for_session(&sub, "standard"),
            SopContext::for_session(&collab, "standard"),
        ];
        for def in get_all_sop_definitions() {
            for (i, v) in def.variants.iter().enumerate() {
                assert!(
                    ctxs.iter().any(|c| (v.active)(c)),
                    "{} 的 variant#{}「{}」无任何生效场景（死项）",
                    def.item.id,
                    i,
                    v.label
                );
            }
        }
    }

    /// 开关语义：非硬规则禁用后规则条数正好 -1；硬规则禁用后不变
    #[test]
    fn disabling_non_hard_rule_removes_exactly_one() {
        let main = mk_session("root", None);
        let sub = mk_session("subagent", Some("p"));
        let defs = get_all_sop_definitions();
        for def in &defs {
            // sub_only 规则只能在子会话中验证开关效果
            let session = if def.item.scope == "sub_only" { &sub } else { &main };
            let (all, _) = build_sop_rules(session, &[], "standard", 1);
            let (filtered, _) = build_sop_rules(session, &[def.item.id.to_string()], "standard", 1);
            if def.item.is_hard_rule {
                assert_eq!(
                    filtered.len(),
                    all.len(),
                    "{} 为硬规则，禁用不应生效",
                    def.item.id
                );
            } else {
                assert_eq!(
                    filtered.len() + 1,
                    all.len(),
                    "{} 禁用后应恰好减少 1 条规则",
                    def.item.id
                );
            }
        }
    }

    /// 作用域隔离：main_only 不出现在子会话、sub_only 不出现在主会话
    #[test]
    fn scope_isolation_between_main_and_sub() {
        let main = mk_session("root", None);
        let sub = mk_session("subagent", Some("p"));
        let main_text = render(&main, "standard");
        let sub_text = render(&sub, "standard");
        for def in get_all_sop_definitions() {
            let title = def.variants[0].title;
            match def.item.scope {
                "main_only" => {
                    assert!(
                        main_text.contains(title),
                        "main_only 规则 {} 未出现在主进程提示词",
                        def.item.id
                    );
                    assert!(
                        !sub_text.contains(title),
                        "main_only 规则 {} 泄漏进子进程提示词",
                        def.item.id
                    );
                }
                "sub_only" => {
                    assert!(
                        sub_text.contains(title),
                        "sub_only 规则 {} 未出现在子进程提示词",
                        def.item.id
                    );
                    assert!(
                        !main_text.contains(title),
                        "sub_only 规则 {} 泄漏进主进程提示词",
                        def.item.id
                    );
                }
                _ => {}
            }
        }
    }

    /// 目录契约：id 唯一、name 必须由 variants[0].title 派生、变体视图不含序号
    #[test]
    fn catalog_ids_unique_and_name_derived() {
        let catalog = get_sop_catalog();
        assert_eq!(catalog.len(), 11);
        let mut ids: Vec<&str> = catalog.iter().map(|s| s.id.as_str()).collect();
        ids.sort_unstable();
        let total = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), total, "SOP id 存在重复");
        for s in &catalog {
            assert!(!s.variants.is_empty(), "{} 缺少 variants", s.id);
            assert_eq!(
                s.name, s.variants[0].title,
                "{} 的 name 未从 variants[0].title 派生",
                s.id
            );
            for v in &s.variants {
                assert!(
                    !v.text.starts_with(|c: char| c.is_ascii_digit()),
                    "{} 的变体 text 不应以序号开头",
                    s.id
                );
            }
        }
    }
}
