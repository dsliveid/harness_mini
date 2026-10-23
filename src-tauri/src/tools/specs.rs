use super::types::*;
use serde_json::{json, Value};

pub fn tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "read_file",
            description: "读取工作区内文本文件的局部或全部内容（带行号）。修改文件前必须先用它确认精确原文。推荐配合 offset_line 与 max_lines 外科手术式精读局部，严禁对大型文件进行多轮循环分页遍历。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "相对于工作区的文件路径"},
                    "offset_line": {"type": "integer", "description": "起始行（1 开始，默认 1）"},
                    "max_lines": {"type": "integer", "description": "最多读取行数（默认 300，避免单次返回过多噪音与 Token 消耗）"}
                },
                "required": ["path"]
            }),
        },
        ToolSpec {
            name: "list_dir",
            description: "列出目录下的文件与子目录（含大小）。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "相对于工作区的目录路径，默认根目录"}
                }
            }),
        },
        ToolSpec {
            name: "file_outline",
            description: "提取代码文件的结构骨架与大纲（包含类、结构体、接口、枚举、函数签名及起始行号），过滤具体实现细节。在深入阅读代码前优先用它获取全局地图，以极低 Token 掌握全貌并精准定位目标行号。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "相对于工作区的文件路径"}
                },
                "required": ["path"]
            }),
        },
        ToolSpec {
            name: "glob",
            description: "按 glob 模式（支持 ** 与 *）查找文件，返回相对路径列表。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string", "description": "glob 模式，如 src/**/*.rs、*.json；支持 ** 与 *"},
                    "path": {"type": "string", "description": "搜索起始目录，默认工作区根目录"}
                },
                "required": ["pattern"]
            }),
        },
        ToolSpec {
            name: "grep",
            description: "在工作区文件内容中按正则搜索，返回 `路径:行号: 内容` 列表。支持返回匹配行周边的上下文行数（context_lines），便于快速看清代码逻辑而无需频繁调用 read_file。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string", "description": "正则表达式"},
                    "path": {"type": "string", "description": "搜索起始目录，默认工作区根目录"},
                    "include": {"type": "string", "description": "文件名过滤 glob，如 *.rs"},
                    "context_lines": {"type": "integer", "description": "可选：匹配行前后额外展示的上下文行数（0~5，默认 0）。若大于 0，将在结果中呈现匹配行及其前后的关联代码"}
                },
                "required": ["pattern"]
            }),
        },
        ToolSpec {
            name: "write_file",
            description: "新建文件或整体覆盖写入完整内容，返回 diff。仅用于新文件，修改已有文件请用 edit_file。",
            risk: Risk::Write,
            schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "相对于工作区的文件路径"},
                    "content": {"type": "string", "description": "完整文件内容"}
                },
                "required": ["path", "content"]
            }),
        },
        ToolSpec {
            name: "edit_file",
            description: "用精确原文替换的方式修改文件（old_string 必须与文件内容逐字符一致），返回 diff。",
            risk: Risk::Write,
            schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "相对于工作区的文件路径"},
                    "old_string": {"type": "string", "description": "要替换的精确原文（修改前必须先 read_file 确认）"},
                    "new_string": {"type": "string", "description": "替换后的文本"},
                    "replace_all": {"type": "boolean", "description": "替换全部出现（默认 false，false 时要求唯一匹配）"}
                },
                "required": ["path", "old_string", "new_string"]
            }),
        },
        ToolSpec {
            name: "run_command",
            description: "在工作区目录执行 shell 命令（构建、测试、git 等），实时返回输出。禁止破坏性命令。",
            risk: Risk::Execute,
            schema: json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "要在工作区执行的 shell 命令"},
                    "cwd": {"type": "string", "description": "相对工作区的子目录（默认工作区根目录）"}
                },
                "required": ["command"]
            }),
        },
        ToolSpec {
            name: "fetch_web_page",
            description: "通过 HTTP/HTTPS 请求获取指定网页的内容，并清洗转换为结构清晰的纯文本或 Markdown（自动过滤脚本、样式与无意义标签）。用于查阅外部技术文档、GitHub 内容、API 参考或网页资料。支持通过统一配置的代理访问网络。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {
                    "url": {"type": "string", "description": "目标网页的完整 URL 地址（必须以 http:// 或 https:// 开头）"},
                    "max_chars": {"type": "integer", "description": "最多返回的文本字符数（默认 8000，防止超大页面消耗过多 Token）"}
                },
                "required": ["url"]
            }),
        },
        ToolSpec {
            name: "browser_navigate",
            description: "在受控真实的 Chromium 浏览器中打开指定网页（默认无头模式后台静默运行，零桌面侵入，不干扰用户日常桌面操作；完整支持 Vue/React 等单页 SPA 渲染与动态 JS 执行）。返回页面标题、精简后的可交互元素清单（包含 @e1, @e2 等标签）、格式化数据表格、控制台错误与失败的网络请求。测试验证全面基于结构化文本断言，可用于本地服务 (如 127.0.0.1:8080) 的全链路界面调试与功能验证。",
            risk: Risk::Execute,
            schema: json!({
                "type": "object",
                "properties": {
                    "url": {"type": "string", "description": "目标网页的完整 URL 地址（如 http://127.0.0.1:8080）"},
                    "headless": {"type": "boolean", "description": "是否以无头模式运行（默认 true；需要人工观察界面调试时可设为 false 弹出浏览器实体窗口）"}
                },
                "required": ["url"]
            }),
        },
        ToolSpec {
            name: "browser_interact",
            description: "在当前受控网页上执行交互操作（如点击、打字输入、回车提交、等待元素加载等）。通过 CDP 虚拟事件通道直接注入内核，绝不抢占物理鼠标键盘。优先使用 @e1, @e2 等 ref 标识符（由 browser_navigate 提取返回）定位元素，也可使用 CSS 选择器。交互完成后自动返回更新后的页面状态、控制台错误及网络异常。",
            risk: Risk::Execute,
            schema: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["click", "type", "press_enter", "wait_for_selector", "scroll"],
                        "description": "交互动作类型：click (点击), type (输入文本), press_enter (回车), wait_for_selector (等待元素)"
                    },
                    "target": {"type": "string", "description": "目标元素 ref 标识符（如 @e1、@e2）或 CSS 选择器"},
                    "value": {"type": "string", "description": "输入文本（type 操作必填，如用户名密码）"}
                },
                "required": ["action", "target"]
            }),
        },
        ToolSpec {
            name: "browser_scroll",
            description: "滚动当前受控网页视口。支持按方向滚动(down/up/top/bottom)、按指定像素滚动，或直接滚动到指定交互元素(@eX)使其居中。长页面或表格溢出时，优先调用本工具向下滑动探索未展示内容。执行后自动刷新并返回最新视口内的元素与状态。",
            risk: Risk::Execute,
            schema: json!({
                "type": "object",
                "properties": {
                    "direction": {
                        "type": "string",
                        "enum": ["down", "up", "top", "bottom", "left", "right"],
                        "description": "滚动方向：down(向下翻一屏/默认)、up(向上翻一屏)、top(回到顶部)、bottom(直达底部)、left(向左滑)、right(向右滑)"
                    },
                    "distance": {
                        "type": "number",
                        "description": "滚动的像素距离(px)，默认约为当前视口高度的 80% (约 700px)"
                    },
                    "target": {
                        "type": "string",
                        "description": "目标元素标识符(如 @e35 或 CSS 选择器)，滚动使该元素在视口中居中显示"
                    }
                }
            }),
        },
        ToolSpec {
            name: "browser_get_debug_state",
            description: "获取当前受控网页的最新调试状态（包括可交互元素清单、最新的 Console 报错信息及 4xx/5xx 网络请求异常列表）。用于在异步数据加载完成或排查 Bug 时快速诊断前端问题。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolSpec {
            name: "browser_screenshot",
            description: "截取当前受控网页的实时视口屏幕截图并保存至本地工作区 .harness/tmp/browser_screenshot.png。注意：标准自动化测试应遵循免视觉准则（直接依据 DOM、数据表格与异常日志断言），本工具仅作为复杂排版或图表的人工协同查验辅助。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {
                    "full_page": {
                        "type": "boolean",
                        "description": "是否截取包含完整长页面的全屏截图（默认 false 为当前视口截图）"
                    }
                }
            }),
        },
        ToolSpec {
            name: "browser_close",
            description: "关闭当前的受控浏览器调试会话并释放系统资源。",
            risk: Risk::Execute,
            schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolSpec {
            name: "todo",
            description: "维护当前任务清单（全量覆盖）。多步任务开始时列出计划，随进展更新各项状态。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {
                    "todos": {
                        "type": "array",
                        "description": "当前任务清单（全量覆盖）",
                        "items": {
                            "type": "object",
                            "properties": {
                                "content": {"type": "string"},
                                "status": {"type": "string", "enum": ["pending", "in_progress", "done"]}
                            },
                            "required": ["content", "status"]
                        }
                    }
                },
                "required": ["todos"]
            }),
        },
        ToolSpec {
            name: "list_skills",
            description: "列出当前工作区已定义的所有项目技能（.harness/skills/）。执行复杂或高频任务前可先查询已有技能。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolSpec {
            name: "save_skill",
            description: "将高频或复杂的组合脚本固化为工作区技能（.harness/skills/<name>/），供后续重复调用。",
            risk: Risk::Write,
            schema: json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "技能英文唯一标识（字母、数字、下划线、连字符，如 build-check）"},
                    "description": {"type": "string", "description": "技能说明，明确其用途和调用时机"},
                    "script_type": {"type": "string", "enum": ["bat", "ps1", "sh", "py", "js"], "description": "脚本类型：bat | ps1 | sh | py | js"},
                    "script_content": {"type": "string", "description": "技能脚本源码"}
                },
                "required": ["name", "description", "script_type", "script_content"]
            }),
        },
        ToolSpec {
            name: "run_skill",
            description: "执行工作区已存在的项目技能（.harness/skills/<name>/），返回执行输出。",
            risk: Risk::Execute,
            schema: json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "要执行的技能名称"},
                    "args": {"type": "string", "description": "传递给技能脚本的参数（可选）"}
                },
                "required": ["name"]
            }),
        },
        // ---------- 项目专属知识与记忆工具集 ----------
        ToolSpec {
            name: "record_memory",
            description: "在多步探索或代码阅读中，及时将关键技术栈、架构约定或阶段性分析结论固化至项目知识库（.harness/memory/），避免大文件反复堆积与遗忘。",
            risk: Risk::Write,
            schema: json!({
                "type": "object",
                "properties": {
                    "category": {
                        "type": "string",
                        "enum": ["tech_stack", "profile", "convention", "digest"],
                        "description": "记忆类型：tech_stack/profile（项目技术大盘与框架体系，永久驻留）、convention（工程规范与避坑约定）、digest（阶段性探索碎记）"
                    },
                    "title": {
                        "type": "string",
                        "description": "简明标题，例如：Spring Boot与中间件架构、全局统一返回规范、JWT鉴权链路"
                    },
                    "content": {
                        "type": "string",
                        "description": "提炼总结的高价值结构化 Markdown 知识内容"
                    }
                },
                "required": ["category", "title", "content"]
            }),
        },
        ToolSpec {
            name: "read_memory",
            description: "查阅当前项目在 .harness/memory/ 中已积累的知识库档案（包括技术大盘、工程规范或特定主题碎记）。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {
                    "topic": {
                        "type": "string",
                        "description": "可选：profile（技术大盘）、conventions（规范约定）、或指定主题名称；缺省时列出所有记忆清单概览"
                    }
                }
            }),
        },
        // ---------- 任务方案计划中枢工具集 (.harness/plans/) ----------
        ToolSpec {
            name: "create_plan",
            description: "针对大改动任务（3个以上文件变动/重构）或多轮对话任务，在 .harness/plans/ 下建立规范的结构化计划 MD 文档，作为权威执行锚点。防止需求在多轮对话中失真或遗失。",
            risk: Risk::Write,
            schema: json!({
                "type": "object",
                "properties": {
                    "title": {"type": "string", "description": "计划简明标题，如 用户鉴权重构与 JWT 改造计划"},
                    "goals": {"type": "string", "description": "核心需求背景与业务目标（条理化清晰描述）"},
                    "architecture": {"type": "string", "description": "架构设计与技术实现方案"},
                    "files": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "预估涉及的文件清单及变更说明，如 [\"[NEW] src/auth/jwt.rs\", \"[MODIFY] src/middleware.rs\"]"
                    },
                    "steps": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "分步实施 Checklist 步骤描述列表，如 [\"编写 jwt.rs 核心编解码\", \"替换中间件并测试\"]"
                    },
                    "verification": {
                        "type": "string",
                        "description": "可选：验收与自检命令或测试策略，如 cargo test --package auth"
                    }
                },
                "required": ["title", "goals", "architecture", "files", "steps"]
            }),
        },
        ToolSpec {
            name: "update_plan",
            description: "当用户在多轮对话中调整需求、或任务推进完成某一阶段时，更新计划文档正文、推进分步清单状态，并记录变更历史。在改动代码前务必先同步计划！若计划调整导致步骤发生变动，必须通过 modified_sections.steps 传入更新后的完整执行清单；若任务全部完成并通过测试，请将 status 设为 completed 以结案解除挂载。",
            risk: Risk::Write,
            schema: json!({
                "type": "object",
                "properties": {
                    "reason": {"type": "string", "description": "调整原因或阶段性说明，如 用户要求增加 Redis 黑名单缓存 / 完成第一阶段编码"},
                    "plan_id": {"type": "string", "description": "可选：指定更新的计划 ID 或文件名 Slug；缺省时自动更新当前会话的活动计划"},
                    "status": {
                        "type": "string",
                        "enum": ["in_progress", "completed", "suspended"],
                        "description": "可选：更新计划状态。任务全部完成并通过测试后请设置为 completed"
                    },
                    "step_updates": {
                        "type": "array",
                        "description": "可选：更新特定步骤的状态",
                        "items": {
                            "type": "object",
                            "properties": {
                                "index": {"type": "integer", "description": "步骤序号（1 开始）"},
                                "content": {"type": "string", "description": "步骤匹配关键词（可选）"},
                                "status": {"type": "string", "enum": ["done", "in_progress", "pending"], "description": "目标状态"}
                            },
                            "required": ["status"]
                        }
                    },
                    "modified_sections": {
                        "type": "object",
                        "description": "可选：需局部更新的方案段落。若需求调整导致实施步骤变化，必须在此传入全量更新后的 steps 数组",
                        "properties": {
                            "goals": {"type": "string", "description": "更新后的需求背景与目标"},
                            "architecture": {"type": "string", "description": "更新后的技术设计方案"},
                            "files": {
                                "type": "array",
                                "items": {"type": "string"},
                                "description": "更新后的文件清单"
                            },
                            "steps": {
                                "type": "array",
                                "items": {"type": "string"},
                                "description": "更新后的完整分步实施 Checklist 描述列表（全量替换原步骤）"
                            },
                            "verification": {
                                "type": "string",
                                "description": "更新后的验证与验收策略"
                            }
                        }
                    },
                    "revision_note": {
                        "type": "string",
                        "description": "可选：追加至需求变更历史 (Revision History) 中的版本备注摘要"
                    }
                },
                "required": ["reason"]
            }),
        },
        ToolSpec {
            name: "switch_plan",
            description: "在当前会话的历史任务计划之间快速切换。例如当用户在多需求场景中提出“回到之前的鉴权任务改个参数”时，调用此工具将历史计划重新唤醒为当前活动计划。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {
                    "plan_id": {"type": "string", "description": "要激活的目标计划 ID 或文件名 Slug"}
                },
                "required": ["plan_id"]
            }),
        },
        ToolSpec {
            name: "read_plan",
            description: "查阅指定计划或当前正在执行的活动计划的完整 Markdown 方案文档。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {
                    "plan_id": {"type": "string", "description": "可选：目标计划 ID 或文件名 Slug；缺省时查阅当前活动计划"}
                }
            }),
        },
        ToolSpec {
            name: "list_plans",
            description: "列出当前工作区在 .harness/plans/ 中已建立的所有任务计划清单及其推进状态、版本与完成度。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {
                    "include_archived": {"type": "boolean", "description": "是否包含已归档至 archive/ 的历史计划，默认 false"}
                }
            }),
        },
        // ---------- 子 Agent 进程协作工具集 ----------
        ToolSpec {
            name: "spawn_subagent",
            description: "创建并启动一个独立的子 Agent 进程并行协作（如前端开发、后端开发、多模块开发等）。子 Agent 拥有独立上下文与工具环境，不污染主会话上下文。通常与 wait_subagents 配合使用：总架构师连续派生多个子任务后，应紧接着调用 wait_subagents 等待完成并汇总结果。严禁子 Agent 递归嵌套调用本工具。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {
                    "role": {"type": "string", "description": "子 Agent 角色定位，例如：前端开发、后端开发、测试验证、文档编写"},
                    "title": {"type": "string", "description": "子任务简明标题，如 编写用户中心页面组件"},
                    "task": {"type": "string", "description": "分配给该子 Agent 的详细需求描述与任务要求"},
                    "workspace": {"type": "string", "description": "可选：指定子 Agent 独立的工作区根目录绝对路径。缺省时默认继承当前主项目的完整工作区根目录。"},
                    "subpath": {"type": "string", "description": "可选：该子 Agent 重点关注的工作区相对子目录（如 src/ 或 backend/）。注意：此项仅作为任务重点指引，子 Agent 的工作区根目录依然为完整项目根目录，仍能访问根目录下的构建配置（如 pom.xml/package.json 等）。"}
                },
                "required": ["role", "title", "task"]
            }),
        },
        ToolSpec {
            name: "get_subagent_status",
            description: "查询子 Agent 进程的当前执行状态（running、done、failed、cancelled）与最新进展输出摘要。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {
                    "subagent_id": {"type": "string", "description": "可选：子 Agent ID；缺省时返回全部子 Agent 状态"}
                }
            }),
        },
        ToolSpec {
            name: "wait_subagents",
            description: "等待一个或多个子 Agent 协作进程执行完毕，并自动获取汇总它们的执行结论、产出总结与改动文件清单。在调用 spawn_subagent 派生子任务后，应紧接着调用本工具等待汇聚。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {
                    "subagent_ids": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "可选：要等待的子 Agent ID 列表；缺省时等待全部运行中的子 Agent"
                    },
                    "timeout_seconds": {
                        "type": "integer",
                        "description": "最大等待超时秒数（默认 60 秒，最大 900 秒）"
                    }
                }
            }),
        },
        ToolSpec {
            name: "stop_subagent",
            description: "停止指定的正在运行的子 Agent 协作进程。",
            risk: Risk::Write,
            schema: json!({
                "type": "object",
                "properties": {
                    "subagent_id": {"type": "string", "description": "要停止的子 Agent ID"}
                },
                "required": ["subagent_id"]
            }),
        },
        ToolSpec {
            name: "spawn_subprocess",
            description: "创建并启动一个独立的临时子进程并行执行特定任务（如探索、排查、独立测试等）。子进程拥有独立上下文与工具环境，物理工作区与主进程严格保持一致，执行完成后其成果直接内嵌在主对话流中呈现。外部用户无法干预子进程，任务完成后自动销毁/归档。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {
                    "role": {"type": "string", "description": "子任务角色定位，例如：技术调研、单元测试、独立排查"},
                    "title": {"type": "string", "description": "子任务简明标题，如 排查审批流程组件"},
                    "task": {"type": "string", "description": "分配给子进程的具体任务详细要求。如需处理特定子目录，请在此参数中明确说明目标目录相对路径与工作目标，建议任务简明聚焦"},
                    "relevant_files": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "可选：主进程已知重点关注的文件路径列表（如 [\"src/auth.rs\"]），帮助子进程跳过全局盲目搜索"
                    },
                    "pinned_context": {
                        "type": "array",
                        "items": {"type": "object"},
                        "description": "可选：上下文图钉与行号锚点列表，例如 [{\"path\": \"src/auth.rs\", \"focus_lines\": [120, 150], \"intent\": \"在此处追加方法\"}]"
                    },
                    "acceptance_criteria": {
                        "type": "string",
                        "description": "可选：交付验收标准或验证命令（如 cargo test test_auth）"
                    },
                    "constraints": {
                        "type": "string",
                        "description": "可选：严格禁止项与负向约束（如 仅修改该文件，严禁修改已有外部接口入参）"
                    }
                },
                "required": ["role", "title", "task"]
            }),
        },
        ToolSpec {
            name: "wait_subprocesses",
            description: "等待一个或多个由你派生的临时子进程执行完毕，并自动汇总它们的执行结论与改动文件清单。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {
                    "subprocess_ids": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "可选：要等待的子进程 ID列表；缺省时等待全部运行中的子进程"
                    },
                    "timeout_seconds": {
                        "type": "integer",
                        "description": "最大等待超时秒数（默认 60 秒，最大 300 秒）"
                    }
                }
            }),
        },
        ToolSpec {
            name: "stop_subprocess",
            description: "强制停止指定的正在运行的临时子进程。",
            risk: Risk::Write,
            schema: json!({
                "type": "object",
                "properties": {
                    "subprocess_id": {"type": "string", "description": "要停止的子进程 ID"}
                },
                "required": ["subprocess_id"]
            }),
        },
        ToolSpec {
            name: "resume_subprocess",
            description: "恢复推进指定处于中断、停止或失败状态的临时子进程，使其在当前已有断点处继续执行任务。",
            risk: Risk::Write,
            schema: json!({
                "type": "object",
                "properties": {
                    "subprocess_id": {"type": "string", "description": "要恢复推进的子进程 ID（如 sub-...）"}
                },
                "required": ["subprocess_id"]
            }),
        },
        ToolSpec {
            name: "dispatch_collaborator",
            description: "向项目专属的常驻【项目协作者】（如前端专家、测试专家等）委派工作任务。协作者将在独立会话中基于其长远角色设定工作，完成后自动增量汇报主会话。注意：委派前请先调用 get_collaborators 确认其处于空闲状态。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {
                    "collaborator_id": {"type": "string", "description": "目标协作者 ID"},
                    "task": {"type": "string", "description": "分配给该协作者的具体任务与要求"},
                    "relevant_files": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "可选：主进程已知重点关注的文件路径列表，帮助协作者跳过全局盲目搜索"
                    },
                    "pinned_context": {
                        "type": "array",
                        "items": {"type": "object"},
                        "description": "可选：上下文图钉与行号锚点列表"
                    },
                    "acceptance_criteria": {
                        "type": "string",
                        "description": "可选：交付验收标准或验证命令"
                    },
                    "constraints": {
                        "type": "string",
                        "description": "可选：严格禁止项与负向约束"
                    }
                },
                "required": ["collaborator_id", "task"]
            }),
        },
        ToolSpec {
            name: "wait_collaborators",
            description: "等待一个或多个协作者完成其当前轮次执行，并自动提取它们自上次汇报以来的增量成果与改动文件汇总回主会话。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {
                    "collaborator_ids": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "可选：要等待的协作者 ID 列表；缺省时等待所有运行中的协作者"
                    },
                    "timeout_seconds": {
                        "type": "integer",
                        "description": "最大等待超时秒数（默认 90 秒，最大 300 秒）"
                    }
                }
            }),
        },
        ToolSpec {
            name: "get_collaborators",
            description: "查询当前项目已配置的所有协作者名录、角色专长、当前运行状态（idle 空闲 / busy 运行中）及最新产出摘要。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolSpec {
            name: "generate_image",
            description: "根据提示词生成图像或图标。当用户需要绘制图片、插画、LOGO、应用图标、海报等视觉图像时调用。生成的图片将保存至本地并在界面直接展示。严禁编写 Python 脚本（如 PIL/matplotlib）绘制图标或图片，必须直接调用本工具。",
            risk: Risk::Write,
            schema: json!({
                "type": "object",
                "properties": {
                    "prompt": {
                        "type": "string",
                        "description": "用于生成图片的详细提示词 (Prompt)，应尽量丰富画面细节、风格、构图与光影"
                    },
                    "model": {
                        "type": "string",
                        "description": "可选：指定生图模型名称。缺省时系统自动匹配会话绑定的生图模型"
                    },
                    "size": {
                        "type": "string",
                        "description": "图片分辨率，如 1024x1024, 768x1344, 1344x768, 512x512 等，默认 1024x1024",
                        "enum": ["1024x1024", "768x1344", "1344x768", "512x512"]
                    },
                    "filename": {
                        "type": "string",
                        "description": "可选：指定生成的图片文件相对路径（如 assets/banner.png），缺省时自动保存在 generated_images 目录"
                    }
                },
                "required": ["prompt"]
            }),
        },
        ToolSpec {
            name: "recognize_image",
            description: "多模态视觉识别与图像理解工具。当需要查看、识别或分析本地图片文件（如 generated_images 目录中的生成图、项目图片资源、设计图、截图、相片等）的内容时调用。严禁使用 read_file 读取图片文件。系统将调用配置的视觉模型并返回深入的视觉理解分析结果。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "本地图片文件的路径（支持工作区相对路径或绝对路径，如 generated_images/happy-new-year.png 或 F:/.../image.png）"
                    },
                    "prompt": {
                        "type": "string",
                        "description": "可选：对图片的识别分析提示词或具体问题（例如：“请提取图片中的所有文字”、“画面中有哪些主体元素？”等）。缺省时系统将全面描述并深入分析图片内容。"
                    },
                    "model": {
                        "type": "string",
                        "description": "可选：指定用于识别的视觉模型名称。缺省时自动使用会话或全局配置的生效视觉模型。"
                    }
                },
                "required": ["path"]
            }),
        },
        ToolSpec {
            name: "judge_decision",
            description: "调用决策模型进行高精度二元判断。适用于评估条件是否满足、用户意图是否明确完整、方案是否可行、是否存在风险等对/错或真/假判断场景。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {
                    "state": {
                        "type": "string",
                        "description": "待判定或评估的业务命题、意图理解陈述、事实背景或方案说明"
                    },
                    "instruction": {
                        "type": "string",
                        "description": "可选：二元判断的核心命题或准则（例如：用户意图是否足够明确可直接执行交付？缺省时自动使用意图明确性与可行性准则）"
                    },
                    "model": {
                        "type": "string",
                        "description": "可选：指定决策模型。缺省时自动使用会话或全局绑定的决策模型"
                    }
                },
                "required": ["state"]
            }),
        },
        ToolSpec {
            name: "choice_decision",
            description: "调用决策模型在多个备选项中做出最优选择与分类。适用于方案多选一、分类归因、路由分发等离散选择场景。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {
                    "state": {
                        "type": "string",
                        "description": "进行选择所依据的上下文背景、需求目标或当前环境信息"
                    },
                    "instruction": {
                        "type": "string",
                        "description": "选择与分类的目标或准则"
                    },
                    "options": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "可供选择的离散选项文本列表"
                    },
                    "model": {
                        "type": "string",
                        "description": "可选：指定决策模型。缺省时自动使用会话或全局绑定的决策模型"
                    }
                },
                "required": ["state", "instruction", "options"]
            }),
        },
        ToolSpec {
            name: "score_decision",
            description: "调用决策模型对目标业务意图理解与交付边界进行量化百分制评分与多维度质检评估。用于第一层级意图理解与复杂需求方案准入审查。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {
                    "understanding": {
                        "type": "string",
                        "description": "对用户核心业务意向与诉求细节的简洁精准剖析（只描述用户要什么，严禁提及内部工具名、协作者或执行流水线）"
                    },
                    "plan": {
                        "type": "string",
                        "description": "业务交付目标与交付边界/合理默认假设（例如交付规格、视觉风格、默认尺寸等业务约定；严禁提及内部底层工具调用与协作者名录）"
                    },
                    "state": {
                        "type": "string",
                        "description": "可选：待评估的内容、方案或补充上下文"
                    },
                    "instruction": {
                        "type": "string",
                        "description": "可选：评分目标与评价维度说明（缺省时自动使用意图理解精准性与业务合理性审查准则）"
                    },
                    "rubric": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "可选：量化评分量表或打分标准参考项"
                    },
                    "model": {
                        "type": "string",
                        "description": "可选：指定决策模型。缺省时自动使用会话或全局绑定的决策模型"
                    }
                }
            }),
        },
        // ---------- 临时空间专用（普通会话不下发，见 agent.rs run_once 的 specs 过滤） ----------
        ToolSpec {
            name: "temp_status",
            description: "查看当前临时空间状态：项目清单（key/名称/临时路径）、各项目变更文件数、已保存的快照。",
            risk: Risk::ReadOnly,
            schema: json!({ "type": "object", "properties": {} }),
        },
        ToolSpec {
            name: "temp_changes",
            description: "列出临时空间中指定项目相对基线的全部变更文件（含增删行数）。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {
                    "project": {"type": "string", "description": "项目 key 或名称（temp_status 可查），默认主项目"}
                }
            }),
        },
        ToolSpec {
            name: "temp_diff",
            description: "查看临时空间中单个变更文件的 diff（基线 vs 当前副本，±3 行上下文分块）。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "temp_changes 返回的相对路径"},
                    "project": {"type": "string", "description": "项目 key 或名称，默认主项目"}
                },
                "required": ["path"]
            }),
        },
        ToolSpec {
            name: "temp_snapshot",
            description: "仅用于临时空间内部：为当前临时空间的修改状态保存备份快照（恢复点）。【注意：本工具不是创建临时空间，临时空间由用户在界面左侧栏发起】。在临时空间中做高风险或批量修改前，可用它留存备份快照，后续可通过 temp_restore 回滚到该状态。",
            risk: Risk::Write,
            schema: json!({
                "type": "object",
                "properties": {
                    "label": {"type": "string", "description": "快照/恢复点标签名（建议简短英文或数字，如 before-refactor）"}
                }
            }),
        },
        ToolSpec {
            name: "temp_restore",
            description: "把临时空间恢复到基线（丢弃全部未提交修改）或指定快照。只影响临时副本，不动原目录；已合并回原目录的内容不会被撤销。危险操作，需用户逐次审批。",
            risk: Risk::Write,
            schema: json!({
                "type": "object",
                "properties": {
                    "target": {"type": "string", "enum": ["baseline"], "description": "baseline = 恢复到基线"},
                    "snapshot": {"type": "string", "description": "快照名（temp_status 列出，可只写到时间戳前缀）；提供时优先生效"}
                }
            }),
        },
        ToolSpec {
            name: "temp_merge",
            description: "把临时空间的全部变更写回各项目原目录（冲突走 AI 智能合并）。不可逆且影响用户原始目录：务必在任务完成、验证通过并向用户说明后调用。",
            risk: Risk::Write,
            schema: json!({ "type": "object", "properties": {} }),
        },
    ]
}
