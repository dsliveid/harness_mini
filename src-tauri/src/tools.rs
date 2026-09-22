use crate::diffutil;
use crate::models::truncate_result;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tauri::{Emitter, Manager};
use tokio::io::AsyncBufReadExt;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Risk {
    ReadOnly,
    Write,
    Execute,
}

impl Risk {
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            Risk::ReadOnly => "read",
            Risk::Write => "write",
            Risk::Execute => "execute",
        }
    }
}

pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: Value,
    pub risk: Risk,
}

#[derive(Clone)]
pub struct ToolCtx {
    pub workspace: PathBuf,
    /// 路径越界判定用的沙箱根：临时空间会话为临时空间根目录（含主项目与关联项目副本）；
    /// 普通会话为 None（等同 workspace）
    pub sandbox_root: Option<PathBuf>,
    pub command_timeout: std::time::Duration,
    /// 临时空间上下文：仅临时空间会话有值，temp_* 工具依赖它定位清单与项目
    pub temp: Option<crate::temp::TempAgentCtx>,
    /// 宿主上下文（AppState / AppHandle / 会话 id）：仅 run_once 运行期间存在
    pub host: Option<HostCtx>,
    /// 当前正在执行的工具事件 ID（用于控制台进程注册与手动关闭）
    pub event_id: Option<String>,
}

/// 把变更类型字符渲染为可读标记（工具输出用）
fn change_mark(c: char) -> &'static str {
    match c {
        'A' => "A",
        'D' => "D",
        _ => "M",
    }
}

/// 临时空间会话下发的 temp_* 工具集（普通会话不下发，见 agent.rs run_once 的 specs 过滤）
pub const TEMP_TOOL_NAMES: &[&str] = &[
    "temp_status",
    "temp_changes",
    "temp_diff",
    "temp_snapshot",
    "temp_restore",
    "temp_merge",
];

/// 子 Agent、协作者与子进程相关工具名称列表（子会话不下发）
pub const SUBAGENT_TOOL_NAMES: &[&str] = &[
    "spawn_subagent",
    "get_subagent_status",
    "wait_subagents",
    "stop_subagent",
    "spawn_subprocess",
    "wait_subprocesses",
    "stop_subprocess",
    "resume_subprocess",
    "resume_subagent",
    "dispatch_collaborator",
    "wait_collaborators",
    "get_collaborators",
];

/// temp_* 工具在非临时空间上下文中的报错文案
const TEMP_NO_CTX: &str = "临时空间上下文不可用（该对话可能不是临时空间对话）";

/// 工具对宿主的只读引用：供 temp_merge 等需要应用状态的工具使用。
/// AppState 由 tauri 托管，工具内部用 `app.state::<AppState>()` 借用即可。
#[derive(Clone)]
pub struct HostCtx {
    pub app: tauri::AppHandle,
    pub session_id: String,
}

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
            description: "当用户在多轮对话中调整需求、或任务推进完成某一阶段时，更新计划文档正文、推进分步清单状态，并记录变更历史。在改动代码前务必先同步计划！若任务全部完成并通过测试，请将 status 设为 completed 以结案解除挂载。",
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
                        "description": "可选：需局部更新的方案段落",
                        "properties": {
                            "goals": {"type": "string", "description": "更新后的需求背景与目标"},
                            "architecture": {"type": "string", "description": "更新后的技术设计方案"},
                            "files": {
                                "type": "array",
                                "items": {"type": "string"},
                                "description": "更新后的文件清单"
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
                        "description": "最大等待超时秒数（默认 60 秒，最大 300 秒）"
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
            description: "根据提示词生成图片。当用户要求画图、生成图片、插图、海报、图标等图像时调用。生成的图片将保存至本地并在界面展示。注意：若存在专属图像生成协作者，主进程必须优先委派协作者处理。",
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

pub fn is_high_danger(cmd: &str) -> bool {
    let patterns = [
        r"(?i)\brm\s+(-[a-z]+)*\s*-?[a-z]*[rf]",
        r"(?i)\bdel\s+/[sq]",
        r"(?i)\brd\s+/s",
        r"(?i)\bformat\s+[a-z]:",
        r"(?i)\bmkfs",
        r"(?i)\bdd\s+if=",
        r"(?i)\bshutdown\b",
        r"(?i)\breboot\b",
        r"(?i)\breg\s+delete\b",
        r"(?i)\bgit\s+push\s+.*--force\b",
        r"(?i)\bgit\s+reset\s+--hard\b",
    ];
    patterns.iter().any(|p| regex::Regex::new(p).map(|re| re.is_match(cmd)).unwrap_or(false))
}

pub fn get_u64_arg(args: &Value, key: &str, default: u64) -> u64 {
    match args.get(key) {
        Some(Value::Number(n)) => n.as_u64().unwrap_or(default),
        Some(Value::String(s)) => s.trim().parse::<u64>().unwrap_or(default),
        _ => default,
    }
}

pub fn get_bool_arg(args: &Value, key: &str, default: bool) -> bool {
    match args.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => true,
            "false" | "0" | "no" => false,
            _ => default,
        },
        Some(Value::Number(n)) => n.as_i64().map(|v| v != 0).unwrap_or(default),
        _ => default,
    }
}

fn find_match_line_numbers(text: &str, target: &str) -> Vec<usize> {
    let mut line_numbers = Vec::new();
    let mut cur_line = 1;
    let mut last_idx = 0;
    for (byte_idx, _) in text.match_indices(target) {
        cur_line += text[last_idx..byte_idx].chars().filter(|&c| c == '\n').count();
        line_numbers.push(cur_line);
        last_idx = byte_idx;
    }
    line_numbers
}

fn diagnose_edit_mismatch(text: &str, old_string: &str) -> String {
    let old_lines: Vec<&str> = old_string.lines().collect();
    if old_lines.is_empty() {
        return "old_string 不能为空".to_string();
    }
    let first_line_trimmed = old_lines[0].trim();
    if !first_line_trimmed.is_empty() {
        let mut candidate_lines = Vec::new();
        for (i, line) in text.lines().enumerate() {
            if line.trim() == first_line_trimmed {
                candidate_lines.push(i + 1);
            }
        }
        if !candidate_lines.is_empty() {
            let candidates_str = candidate_lines
                .iter()
                .take(5)
                .map(|l| format!("第 {l} 行"))
                .collect::<Vec<_>>()
                .join("、");
            return format!(
                "old_string 未在文件中找到完全匹配的内容。但第 1 行在文件中找到相似行（位于 {candidates_str}）。请检查缩进、空白字符或邻近行内容是否发生变动，或先用 read_file 确认最新内容。"
            );
        }
    }
    let old_collapsed = old_string.split_whitespace().collect::<Vec<_>>().join(" ");
    let text_collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text_collapsed.contains(&old_collapsed) {
        return "old_string 未完全匹配，但忽略缩进与空格差异后存在匹配。请通过 read_file 复制目标位置的精确缩进与空格。".to_string();
    }

    "old_string 未在文件中找到（提示：系统已自动统一 CRLF/LF 换行符，请使用 read_file 确认精确的内容与缩进）".to_string()
}

fn resolve(ctx: &ToolCtx, rel: &str) -> PathBuf {
    let p = Path::new(rel);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        let clean = rel.trim_start_matches(|c| c == '/' || c == '\\');
        ctx.workspace.join(clean)
    }
}

/// 路径是否位于沙箱内（用于审批判定；临时空间会话的沙箱为整个临时空间根目录）
pub fn inside_workspace(ctx: &ToolCtx, rel: &str) -> bool {
    let p = resolve(ctx, rel);
    let root = ctx.sandbox_root.as_ref().unwrap_or(&ctx.workspace);
    let Ok(root_canon) = root.canonicalize() else {
        return false;
    };

    // 若目标已存在，直接比对真实规范化路径
    if let Ok(p_canon) = p.canonicalize() {
        return p_canon.starts_with(&root_canon);
    }

    // 若目标尚不存在（如新建文件），向上追溯查找最近的已存在父目录进行比对
    let mut curr = p.as_path();
    while let Some(parent) = curr.parent() {
        if let Ok(parent_canon) = parent.canonicalize() {
            return parent_canon.starts_with(&root_canon);
        }
        curr = parent;
    }
    false
}

fn glob_to_regex(pattern: &str) -> String {
    #[cfg(windows)]
    let mut re = String::from("(?i)^");
    #[cfg(not(windows))]
    let mut re = String::from("^");

    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' => {
                if i + 1 < chars.len() && chars[i + 1] == '*' {
                    re.push_str(".*");
                    i += 2;
                    // 吞掉 ** 后跟的 /
                    if i < chars.len() && chars[i] == '/' {
                        i += 1;
                    }
                    continue;
                }
                re.push_str("[^/\\\\]*");
            }
            '?' => re.push_str("[^/\\\\]"),
            '{' => {
                if let Some(close_idx) = chars[i + 1..].iter().position(|&c| c == '}').map(|pos| i + 1 + pos) {
                    let inner: String = chars[i + 1..close_idx].iter().collect();
                    if inner.contains(',') {
                        let parts: Vec<&str> = inner.split(',').collect();
                        let regex_parts: Vec<String> = parts
                            .iter()
                            .map(|part| {
                                let mut sub = String::new();
                                for c in part.chars() {
                                    match c {
                                        '*' => sub.push_str("[^/\\\\]*"),
                                        '?' => sub.push_str("[^/\\\\]"),
                                        c if "\\.^$|+()[]{}-".contains(c) => {
                                            sub.push('\\');
                                            sub.push(c);
                                        }
                                        c => sub.push(c),
                                    }
                                }
                                sub
                            })
                            .collect();
                        re.push_str("(?:");
                        re.push_str(&regex_parts.join("|"));
                        re.push(')');
                        i = close_idx + 1;
                        continue;
                    }
                }
                re.push_str("\\{");
            }
            c if "\\.^$|+()[]}".contains(c) => {
                re.push('\\');
                re.push(c);
            }
            c => re.push(c),
        }
        i += 1;
    }
    re.push('$');
    re
}

fn is_binary(buf: &[u8]) -> bool {
    let n = buf.len().min(8192);
    buf[..n].contains(&0)
}

pub type PartialCb<'a> = &'a (dyn Fn(&str) + Send + Sync);

async fn pipe_lines<T: tokio::io::AsyncRead + Unpin>(
    stream: T,
    tx: tokio::sync::mpsc::UnboundedSender<String>,
) {
    let reader = tokio::io::BufReader::new(stream);
    let mut lines = reader.lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if tx.send(line).is_err() {
            break;
        }
    }
}

pub async fn execute(
    name: &str,
    args: &Value,
    ctx: &ToolCtx,
    on_partial: PartialCb<'_>,
) -> Result<String, String> {
    match name {
        "read_file" => read_file(args, ctx).await,
        "list_dir" => list_dir(args, ctx).await,
        "file_outline" => file_outline(args, ctx).await,
        "glob" => glob(args, ctx).await,
        "grep" => grep(args, ctx).await,
        "write_file" => write_file(args, ctx).await,
        "edit_file" => edit_file(args, ctx).await,
        "run_command" => run_command(args, ctx, on_partial).await,
        "todo" => Ok("ok".to_string()),
        "list_skills" => list_skills_tool(ctx).await,
        "save_skill" => save_skill_tool(args, ctx).await,
        "run_skill" => run_skill_tool(args, ctx, on_partial).await,
        "record_memory" => record_memory_tool(args, ctx).await,
        "read_memory" => read_memory_tool(args, ctx).await,
        "create_plan" => create_plan_tool(args, ctx).await,
        "update_plan" => update_plan_tool(args, ctx).await,
        "switch_plan" => switch_plan_tool(args, ctx).await,
        "read_plan" => read_plan_tool(args, ctx).await,
        "list_plans" => list_plans_tool(args, ctx).await,
        "spawn_subagent" | "spawn_subprocess" => spawn_subagent_tool(args, ctx).await,
        "get_subagent_status" => get_subagent_status_tool(args, ctx).await,
        "wait_subagents" | "wait_subprocesses" => wait_subagents_tool(args, ctx).await,
        "stop_subagent" | "stop_subprocess" => stop_subagent_tool(args, ctx).await,
        "resume_subagent" | "resume_subprocess" => resume_subagent_tool(args, ctx).await,
        "dispatch_collaborator" => dispatch_collaborator_tool(args, ctx).await,
        "wait_collaborators" => wait_collaborators_tool(args, ctx).await,
        "get_collaborators" => get_collaborators_tool(ctx).await,
        "generate_image" => generate_image_tool(args, ctx).await,
        "temp_status" => temp_status(ctx).await,
        "temp_changes" => temp_changes(args, ctx).await,
        "temp_diff" => temp_diff(args, ctx).await,
        "temp_snapshot" => temp_snapshot(args, ctx).await,
        "temp_restore" => temp_restore(args, ctx).await,
        "temp_merge" => temp_merge(ctx).await,
        other => Err(format!("未知工具: {other}")),
    }
}

async fn read_file(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let rel = args.get("path").and_then(|v| v.as_str()).ok_or("缺少 path")?;
    let path = resolve(ctx, rel);
    let bytes = tokio::fs::read(&path).await.map_err(|e| format!("读取失败: {e}"))?;
    if is_binary(&bytes) {
        return Err("疑似二进制文件，无法以文本读取".into());
    }
    let text = String::from_utf8_lossy(&bytes);
    let offset = get_u64_arg(args, "offset_line", 1).max(1) as usize;
    let max_lines = get_u64_arg(args, "max_lines", 300) as usize;

    let mut out = String::new();
    let mut count = 0usize;
    for (i, line) in text.lines().enumerate() {
        let lineno = i + 1;
        if lineno < offset {
            continue;
        }
        if count >= max_lines {
            let end_line = offset + count - 1;
            out.push_str(&format!(
                "\n[已达到单次读取上限 max_lines={max_lines}（当前展示至第 {end_line} 行），文件尚未读完。若需了解代码结构，请优先使用 file_outline 提取大纲，或结合 grep 定位目标函数后用 offset_line 局部精读，严禁连续循环分页遍历]\n"
            ));
            break;
        }
        use std::fmt::Write;
        let _ = writeln!(out, "{lineno:>6}\t{line}");
        count += 1;
        if out.len() > crate::models::TOOL_RESULT_LIMIT {
            out.push_str("\n[文件过大，已截断至 32KB]\n");
            break;
        }
    }
    if count == 0 {
        out = "(空文件或范围为空)".into();
    }
    Ok(truncate_result(&out))
}

async fn file_outline(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let rel = args.get("path").and_then(|v| v.as_str()).ok_or("缺少 path")?;
    let path = resolve(ctx, rel);
    let bytes = tokio::fs::read(&path).await.map_err(|e| format!("读取失败: {e}"))?;
    if is_binary(&bytes) {
        return Err("疑似二进制文件，无法提取代码大纲".into());
    }
    let text = String::from_utf8_lossy(&bytes);
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let outline = extract_outline(&text, &ext);
    if outline.is_empty() {
        return Ok("(文件中未识别出类、结构体、接口、函数或标题等符号定义)".into());
    }
    Ok(truncate_result(&outline))
}

fn extract_outline(text: &str, ext: &str) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let mut total = 0usize;

    for (i, raw_line) in text.lines().enumerate() {
        let lineno = i + 1;
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let is_symbol = match ext {
            "rs" => {
                if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
                    false
                } else {
                    let words: Vec<&str> = trimmed.split_whitespace().collect();
                    is_rust_symbol(&words)
                }
            }
            "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => {
                if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
                    false
                } else {
                    is_ts_js_symbol(trimmed)
                }
            }
            "py" => {
                if trimmed.starts_with('#') {
                    false
                } else {
                    trimmed.starts_with("def ")
                        || trimmed.starts_with("async def ")
                        || trimmed.starts_with("class ")
                }
            }
            "go" => {
                if trimmed.starts_with("//") || trimmed.starts_with("/*") {
                    false
                } else {
                    trimmed.starts_with("func ")
                        || (trimmed.starts_with("type ")
                            && (trimmed.contains("struct") || trimmed.contains("interface")))
                }
            }
            "java" | "cs" | "cpp" | "c" | "h" | "hpp" => {
                if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
                    false
                } else {
                    is_c_like_symbol(trimmed)
                }
            }
            "md" | "markdown" => {
                trimmed.starts_with('#') && trimmed.chars().take_while(|&c| c == '#').count() <= 6
            }
            _ => {
                !raw_line.starts_with(' ')
                    && !raw_line.starts_with('\t')
                    && !trimmed.starts_with("//")
                    && !trimmed.starts_with('#')
                    && (trimmed.contains("fn ")
                        || trimmed.contains("func ")
                        || trimmed.contains("def ")
                        || trimmed.contains("class ")
                        || trimmed.contains("interface ")
                        || trimmed.contains("struct "))
            }
        };

        if is_symbol {
            let disp: String = raw_line.trim_end().chars().take(200).collect();
            let _ = writeln!(out, "{lineno:>6}\t{disp}");
            total += 1;
            if total >= 400 || out.len() > crate::models::TOOL_RESULT_LIMIT {
                out.push_str("\n[符号大纲过多，已截断前 400 个]\n");
                break;
            }
        }
    }

    out
}

fn is_rust_symbol(words: &[&str]) -> bool {
    if words.is_empty() {
        return false;
    }
    for (idx, &w) in words.iter().enumerate() {
        match w {
            "fn" | "struct" | "enum" | "trait" | "type" | "mod" | "macro_rules!" => {
                let valid_modifiers = words[..idx].iter().all(|&m| {
                    m == "pub"
                        || m.starts_with("pub(")
                        || m == "async"
                        || m == "const"
                        || m == "unsafe"
                        || m == "extern"
                        || m == "default"
                });
                if valid_modifiers {
                    return true;
                }
            }
            "impl" => {
                return idx == 0 || (idx == 1 && words[0] == "unsafe");
            }
            _ => {}
        }
    }
    false
}

fn is_ts_js_symbol(trimmed: &str) -> bool {
    let t = if trimmed.starts_with("export default ") {
        &trimmed["export default ".len()..]
    } else if trimmed.starts_with("export ") {
        &trimmed["export ".len()..]
    } else {
        trimmed
    };

    if t.starts_with("class ")
        || t.starts_with("interface ")
        || t.starts_with("type ")
        || t.starts_with("enum ")
        || t.starts_with("function ")
        || t.starts_with("async function ")
    {
        return true;
    }

    if trimmed.starts_with("export const ") || trimmed.starts_with("export let ") {
        return true;
    }

    if t.starts_with("const ") || t.starts_with("let ") {
        if t.contains("=>") || t.contains("function(") || t.contains("function (") {
            return true;
        }
    }

    false
}

fn is_c_like_symbol(trimmed: &str) -> bool {
    trimmed.starts_with("class ")
        || trimmed.starts_with("struct ")
        || trimmed.starts_with("enum ")
        || trimmed.starts_with("interface ")
        || trimmed.starts_with("public ")
        || trimmed.starts_with("private ")
        || trimmed.starts_with("protected ")
        || trimmed.starts_with("internal ")
        || trimmed.starts_with("void ")
        || (trimmed.contains('(') && trimmed.contains(')') && !trimmed.ends_with(';'))
}

async fn list_dir(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let rel = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
    let path = resolve(ctx, rel);
    let mut rd = tokio::fs::read_dir(&path).await.map_err(|e| format!("读取目录失败: {e}"))?;
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    while let Some(entry) = rd.next_entry().await.map_err(|e| e.to_string())? {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') && name != ".env" {
            continue;
        }
        let ft = entry.file_type().await.map_err(|e| e.to_string())?;
        if ft.is_dir() {
            dirs.push(format!("{name}/"));
        } else {
            let size = entry.metadata().await.map(|m| m.len()).unwrap_or(0);
            files.push(format!("{name}\t{size}B"));
        }
        if dirs.len() + files.len() > 2000 {
            break;
        }
    }
    dirs.sort();
    files.sort();
    let mut out = String::new();
    if !dirs.is_empty() {
        out.push_str(&dirs.join("\n"));
        out.push('\n');
    }
    if !files.is_empty() {
        out.push_str(&files.join("\n"));
    }
    if out.is_empty() {
        return Ok("(空目录)".to_string());
    }
    Ok(truncate_result(&out))
}

async fn glob(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let pattern = args.get("pattern").and_then(|v| v.as_str()).ok_or("缺少 pattern")?;
    let rel = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
    let base = resolve(ctx, rel);
    let re = regex::Regex::new(&glob_to_regex(pattern)).map_err(|e| format!("pattern 无效: {e}"))?;

    let mut matches = Vec::new();
    let walker = ignore::WalkBuilder::new(&base)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            name != ".git" && name != "node_modules" && name != "target"
        })
        .build();

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }
        let rel_path = entry
            .path()
            .strip_prefix(&base)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .replace('\\', "/");
        if re.is_match(&rel_path) {
            matches.push(rel_path);
            if matches.len() >= 500 {
                break;
            }
        }
    }
    if matches.is_empty() {
        return Ok("(无匹配文件)".into());
    }
    Ok(truncate_result(&matches.join("\n")))
}

async fn grep(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let pattern = args.get("pattern").and_then(|v| v.as_str()).ok_or("缺少 pattern")?;
    let rel = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
    let include = args.get("include").and_then(|v| v.as_str());
    let context_lines = get_u64_arg(args, "context_lines", 0).min(5) as usize;
    let base = resolve(ctx, rel);
    let (re, is_escaped_fallback) = match regex::Regex::new(pattern) {
        Ok(r) => (r, false),
        Err(e) => {
            let escaped = regex::escape(pattern);
            match regex::Regex::new(&escaped) {
                Ok(r) => (r, true),
                Err(_) => return Err(format!("正则无效: {e}")),
            }
        }
    };
    let include_re = include
        .map(|g| regex::Regex::new(&glob_to_regex(g)).ok())
        .flatten();

    let mut out = String::new();
    let mut total = 0usize;
    let walker = ignore::WalkBuilder::new(&base)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            name != ".git" && name != "node_modules" && name != "target" && name != "dist"
        })
        .build();

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if meta.len() > 1_000_000 {
            continue;
        }
        let rel_path = entry
            .path()
            .strip_prefix(&base)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .replace('\\', "/");
        if let Some(ire) = &include_re {
            let fname = entry.file_name().to_string_lossy();
            if !ire.is_match(&fname) && !ire.is_match(&rel_path) {
                continue;
            }
        }
        let Ok(bytes) = tokio::fs::read(entry.path()).await else { continue };
        if is_binary(&bytes) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        use std::fmt::Write;

        if context_lines == 0 {
            for (i, line) in text.lines().enumerate() {
                if re.is_match(line) {
                    let disp: String = line.trim().chars().take(240).collect();
                    let _ = writeln!(out, "{rel_path}:{}: {disp}", i + 1);
                    total += 1;
                    if total >= 200 || out.len() > crate::models::TOOL_RESULT_LIMIT {
                        out.push_str("\n[匹配过多，结果已截断]");
                        return Ok(truncate_result(&out));
                    }
                }
            }
        } else {
            let lines: Vec<&str> = text.lines().collect();
            let mut match_indices = Vec::new();
            for (i, line) in lines.iter().enumerate() {
                if re.is_match(line) {
                    match_indices.push(i);
                }
            }

            if !match_indices.is_empty() {
                let mut ranges: Vec<(usize, usize)> = Vec::new();
                for &idx in &match_indices {
                    let start = idx.saturating_sub(context_lines);
                    let end = (idx + context_lines).min(lines.len().saturating_sub(1));
                    if let Some(last) = ranges.last_mut() {
                        if start <= last.1 + 1 {
                            last.1 = last.1.max(end);
                            continue;
                        }
                    }
                    ranges.push((start, end));
                }

                let match_set: std::collections::HashSet<usize> = match_indices.into_iter().collect();
                for (start, end) in ranges {
                    for line_idx in start..=end {
                        let is_m = match_set.contains(&line_idx);
                        let disp: String = lines[line_idx].trim_end().chars().take(240).collect();
                        let lineno = line_idx + 1;
                        if is_m {
                            let _ = writeln!(out, "{rel_path}:{lineno}: {disp}");
                            total += 1;
                        } else {
                            let _ = writeln!(out, "{rel_path}-{lineno}- {disp}");
                        }
                        if total >= 200 || out.len() > crate::models::TOOL_RESULT_LIMIT {
                            out.push_str("\n[匹配过多，结果已截断]");
                            return Ok(truncate_result(&out));
                        }
                    }
                    if end + 1 < lines.len() {
                        let _ = writeln!(out, "--");
                    }
                }
            }
        }
    }
    if total == 0 {
        if is_escaped_fallback {
            return Ok(format!("(无匹配。注: 原模式包含特殊符号且不是有效正则，已降级为字面量匹配: \"{pattern}\")"));
        }
        return Ok("(无匹配)".into());
    }
    if is_escaped_fallback {
        out.push_str(&format!("\n(注: 原 pattern 不是合法正则，已降级为字面量匹配: \"{pattern}\")"));
    }
    Ok(truncate_result(&out))
}

async fn write_file(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let rel = args.get("path").and_then(|v| v.as_str()).ok_or("缺少 path")?;
    let content = args.get("content").and_then(|v| v.as_str()).ok_or("缺少 content")?;
    let path = resolve(ctx, rel);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let old = tokio::fs::read(&path).await.ok();
    let existed = old.is_some();
    let old_str = old
        .map(|b| String::from_utf8_lossy(&b).to_string())
        .unwrap_or_default();
    let is_crlf = old_str.contains("\r\n");
    let content_to_write = if is_crlf && !content.contains("\r\n") {
        content.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        content.to_string()
    };
    tokio::fs::write(&path, &content_to_write)
        .await
        .map_err(|e| format!("写入失败: {e}"))?;
    let d = diffutil::diff_lines(&old_str, &content_to_write, 24 * 1024);
    let action = if existed { "已覆盖写入" } else { "已创建" };
    Ok(truncate_result(&format!(
        "{action} {rel}（+{} −{}）\n```diff\n{}```",
        d.added, d.removed, d.text
    )))
}

async fn edit_file(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let rel = args.get("path").and_then(|v| v.as_str()).ok_or("缺少 path")?;
    let old_string = args.get("old_string").and_then(|v| v.as_str()).ok_or("缺少 old_string")?;
    let new_string = args.get("new_string").and_then(|v| v.as_str()).ok_or("缺少 new_string")?;
    let replace_all = get_bool_arg(args, "replace_all", false);
    let path = resolve(ctx, rel);

    let bytes = tokio::fs::read(&path).await.map_err(|e| format!("读取失败（请先 read_file 确认内容）: {e}"))?;
    let raw_text = String::from_utf8_lossy(&bytes).to_string();

    let is_crlf = raw_text.contains("\r\n");
    let text_lf = raw_text.replace("\r\n", "\n");
    let old_lf = old_string.replace("\r\n", "\n");
    let new_lf = new_string.replace("\r\n", "\n");

    if old_lf.is_empty() {
        return Err("old_string 不能为空".into());
    }

    let count = text_lf.matches(&old_lf).count();
    if count == 0 {
        return Err(diagnose_edit_mismatch(&text_lf, &old_lf));
    }
    if !replace_all && count > 1 {
        let match_lines = find_match_line_numbers(&text_lf, &old_lf);
        let lines_str = match_lines
            .iter()
            .take(10)
            .map(|l| format!("第 {l} 行"))
            .collect::<Vec<_>>()
            .join("、");
        let suffix = if match_lines.len() > 10 { " 等" } else { "" };
        return Err(format!(
            "old_string 出现 {count} 次（位于 {lines_str}{suffix}），需要更多上下文行使其唯一，或设置 replace_all=true"
        ));
    }

    let replaced_lf = if replace_all {
        text_lf.replace(&old_lf, &new_lf)
    } else {
        text_lf.replacen(&old_lf, &new_lf, 1)
    };

    let final_content = if is_crlf {
        replaced_lf.replace('\n', "\r\n")
    } else {
        replaced_lf
    };

    tokio::fs::write(&path, &final_content)
        .await
        .map_err(|e| format!("写入失败: {e}"))?;
    let d = diffutil::diff_lines(&raw_text, &final_content, 24 * 1024);
    Ok(truncate_result(&format!(
        "已修改 {rel}（+{} −{}）\n```diff\n{}```",
        d.added, d.removed, d.text
    )))
}

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// 强制杀死指定进程及其所有子进程树（跨平台安全强杀）
pub fn kill_process_tree(pid: u32) {
    #[cfg(windows)]
    {
        let taskkill_bin = std::env::var("SystemRoot")
            .map(|r| format!("{r}\\System32\\taskkill.exe"))
            .unwrap_or_else(|_| "taskkill".into());
        let mut kill_cmd = std::process::Command::new(taskkill_bin);
        kill_cmd.args(["/F", "/T", "/PID", &pid.to_string()]);
        kill_cmd.creation_flags(CREATE_NO_WINDOW);
        let _ = kill_cmd.output();
    }
    #[cfg(not(windows))]
    {
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
            libc::kill(pid as i32, libc::SIGKILL);
        }
    }
}

/// 将子进程挂载到系统级 Job Object，确保主进程意外退出或崩溃时内核级联强杀孤儿进程
#[cfg(windows)]
pub fn assign_pid_to_job(pid: u32) {
    use std::sync::OnceLock;
    type HANDLE = *mut std::ffi::c_void;
    type BOOL = i32;
    type DWORD = u32;

    #[repr(C)]
    struct IO_COUNTERS {
        read_operation_count: u64,
        write_operation_count: u64,
        other_operation_count: u64,
        read_transfer_count: u64,
        write_transfer_count: u64,
        other_transfer_count: u64,
    }

    #[repr(C)]
    struct JOBOBJECT_BASIC_LIMIT_INFORMATION {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: DWORD,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: DWORD,
        affinity: usize,
        priority_class: DWORD,
        scheduling_class: DWORD,
    }

    #[repr(C)]
    struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
        basic_limit_information: JOBOBJECT_BASIC_LIMIT_INFORMATION,
        io_info: IO_COUNTERS,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_limit: usize,
        peak_job_memory_limit: usize,
    }

    extern "system" {
        fn CreateJobObjectW(lpJobAttributes: *mut std::ffi::c_void, lpName: *const u16) -> HANDLE;
        fn SetInformationJobObject(
            hJob: HANDLE,
            JobObjectInformationClass: i32,
            lpJobObjectInformation: *const std::ffi::c_void,
            cbJobObjectInformationLength: DWORD,
        ) -> BOOL;
        fn OpenProcess(dwDesiredAccess: DWORD, bInheritHandle: BOOL, dwProcessId: DWORD) -> HANDLE;
        fn AssignProcessToJobObject(hJob: HANDLE, hProcess: HANDLE) -> BOOL;
        fn CloseHandle(hObject: HANDLE) -> BOOL;
    }

    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: DWORD = 0x2000;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS: i32 = 9;
    const PROCESS_SET_QUOTA: DWORD = 0x0100;
    const PROCESS_TERMINATE: DWORD = 0x0001;

    static GLOBAL_JOB: OnceLock<usize> = OnceLock::new();
    let job_handle = *GLOBAL_JOB.get_or_init(|| unsafe {
        let job = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
        if !job.is_null() {
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let _ = SetInformationJobObject(
                job,
                JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
                &info as *const _ as *const std::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as DWORD,
            );
        }
        job as usize
    });

    if job_handle != 0 {
        unsafe {
            let h_proc = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
            if !h_proc.is_null() {
                let _ = AssignProcessToJobObject(job_handle as HANDLE, h_proc);
                let _ = CloseHandle(h_proc);
            }
        }
    }
}

/// 进程树生命周期守卫：协程被 abort / drop 时兜底强杀进程树
struct ProcessTreeGuard {
    pid: Option<u32>,
    active: bool,
}

impl ProcessTreeGuard {
    fn new(pid: Option<u32>) -> Self {
        Self { pid, active: true }
    }
    fn defuse(&mut self) {
        self.active = false;
    }
}

impl Drop for ProcessTreeGuard {
    fn drop(&mut self) {
        if self.active {
            if let Some(pid) = self.pid {
                kill_process_tree(pid);
            }
        }
    }
}

async fn run_command(
    args: &Value,
    ctx: &ToolCtx,
    on_partial: PartialCb<'_>,
) -> Result<String, String> {
    let command = args.get("command").and_then(|v| v.as_str()).ok_or("缺少 command")?;
    let cwd = args
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(|c| resolve(ctx, c))
        .unwrap_or_else(|| ctx.workspace.clone());

    #[cfg(windows)]
    let mut cmd = {
        let mut c = tokio::process::Command::new("powershell");
        c.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command"]);
        c.arg(format!(
            "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; $OutputEncoding = [System.Text.Encoding]::UTF8; {command}"
        ));
        c.creation_flags(CREATE_NO_WINDOW);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = tokio::process::Command::new("sh");
        c.arg("-c").arg(command);
        c
    };
    cmd.current_dir(&cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| format!("启动命令失败: {e}"))?;
    let pid = child.id();
    #[cfg(windows)]
    if let Some(p) = pid {
        assign_pid_to_job(p);
    }
    let mut tree_guard = ProcessTreeGuard::new(pid);
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let (kill_tx, mut kill_rx) = tokio::sync::oneshot::channel::<()>();
    let event_id_opt = ctx.event_id.clone();
    if let (Some(host), Some(eid)) = (&ctx.host, &event_id_opt) {
        let state = host.app.state::<crate::AppState>();
        state.running_commands.lock().unwrap().insert(
            eid.clone(),
            crate::RunningCommand {
                session_id: host.session_id.clone(),
                pid,
                tx: kill_tx,
            },
        );
    }

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let tx_err = tx.clone();
    tokio::spawn(async move {
        pipe_lines(stdout, tx).await;
    });
    tokio::spawn(async move {
        pipe_lines(stderr, tx_err).await;
    });

    let mut output = String::new();
    let deadline = tokio::time::Instant::now() + ctx.command_timeout;
    let mut timed_out = false;
    let mut killed_by_user = false;
    loop {
        tokio::select! {
            _ = &mut kill_rx => {
                killed_by_user = true;
                if let Some(p) = pid {
                    kill_process_tree(p);
                }
                let _ = child.start_kill();
                break;
            }
            line = rx.recv() => {
                match line {
                    Some(l) => {
                        use std::fmt::Write;
                        let _ = writeln!(output, "{l}");
                        if output.len() < 64 * 1024 {
                            on_partial(&l);
                        }
                        if output.len() > 512 * 1024 {
                            output.push_str("\n[输出过长，已停止收集]");
                            let _ = child.start_kill();
                            break;
                        }
                    }
                    None => break,
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                timed_out = true;
                let _ = child.start_kill();
                break;
            }
        }
    }

    if let (Some(host), Some(eid)) = (&ctx.host, &event_id_opt) {
        let state = host.app.state::<crate::AppState>();
        state.running_commands.lock().unwrap().remove(eid);
    }

    let wait_res = tokio::time::timeout(std::time::Duration::from_secs(3), child.wait()).await;
    let status = match wait_res {
        Ok(s) => s,
        Err(_) => {
            let _ = child.start_kill();
            Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "等待子进程退出超时"))
        }
    };
    tree_guard.defuse();
    if killed_by_user {
        return Err(format!("{output}\n[控制台进程已由用户手动关闭]"));
    }
    if timed_out {
        return Ok(truncate_result(&format!(
            "{output}\n[命令超时（{}s），已强制终止]",
            ctx.command_timeout.as_secs()
        )));
    }
    let code = match status {
        Ok(s) => s.code().unwrap_or(-1),
        Err(_) => -1,
    };
    Ok(truncate_result(&format!("{output}\n[exit code: {code}]")))
}

// ---------- 临时空间工具实现 ----------

/// temp_status：项目清单 + 各项目变更文件数 + 快照列表
async fn temp_status(ctx: &ToolCtx) -> Result<String, String> {
    let t = ctx.temp.as_ref().ok_or(TEMP_NO_CTX)?;
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "临时空间根目录: {}", t.manifest.root);
    let _ = writeln!(out, "快照目录: {}", crate::temp::list_snapshots(&t.manifest.root).join("、"));
    for p in &t.manifest.projects {
        let n = crate::temp::list_changes(p).map(|c| c.len()).unwrap_or(0);
        let _ = writeln!(
            out,
            "- 「{}」 key={} 变更文件 {} 个\n  临时目录: {}\n  原目录: {}",
            p.name, p.key, n, p.temp, p.source
        );
    }
    Ok(truncate_result(&out))
}

/// temp_changes：指定项目相对基线的变更文件清单（含增删行数）
async fn temp_changes(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let t = ctx.temp.as_ref().ok_or(TEMP_NO_CTX)?;
    let project = args.get("project").and_then(|v| v.as_str());
    let entry = crate::temp::resolve_entry(&t.manifest, project)?;
    let changes = crate::temp::list_changes(entry)?;
    if changes.is_empty() {
        return Ok(format!("「{}」暂无变更", entry.name));
    }
    use std::fmt::Write;
    let mut out = String::new();
    for f in &changes {
        let (a, r) = crate::temp::change_stat(entry, &f.path, f.change);
        let _ = writeln!(out, "{}\t{}\t+{} −{}", change_mark(f.change), f.path, a, r);
    }
    Ok(truncate_result(&format!(
        "「{}」共 {} 个变更文件：\n{out}",
        entry.name,
        changes.len()
    )))
}

/// temp_diff：单文件 diff（复用 UI 弹窗同源的数据，仅渲染为文本）
async fn temp_diff(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let t = ctx.temp.as_ref().ok_or(TEMP_NO_CTX)?;
    let path = args.get("path").and_then(|v| v.as_str()).ok_or("缺少 path")?;
    let project = args.get("project").and_then(|v| v.as_str());
    let entry = crate::temp::resolve_entry(&t.manifest, project)?;
    let d = crate::temp::file_diff(entry, path)?;
    if d.binary {
        return Ok(format!("{}（二进制文件，不显示 diff）", d.path));
    }
    if d.too_large {
        return Ok(format!("{}（文件过大，不显示 diff）", d.path));
    }
    if d.hunks.is_empty() {
        return Ok(format!("{}（无差异）", d.path));
    }
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "「{}」{}（+{} −{}）", d.project_name, d.path, d.added, d.removed);
    if d.truncated {
        out.push_str("[diff 过长已截断]\n");
    }
    for h in &d.hunks {
        let _ = writeln!(out, "@@ -{},{} +{},{} @@", h.old_start, h.old_lines, h.new_start, h.new_lines);
        for l in &h.lines {
            let mark = match l.tag.as_str() {
                "add" => "+",
                "del" => "-",
                _ => " ",
            };
            let _ = writeln!(out, "{mark}{}", l.text);
        }
    }
    Ok(truncate_result(&out))
}

/// temp_snapshot：快照当前临时空间
async fn temp_snapshot(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let t = ctx.temp.as_ref().ok_or(TEMP_NO_CTX)?;
    let label = args
        .get("label")
        .and_then(|v| v.as_str())
        .unwrap_or("manual");
    let out = crate::temp::snapshot_workspace(&t.manifest, label)?;
    let mut text = format!(
        "快照「{}」已保存至 {}（拷贝 {} 个文件）",
        out.label, out.dir, out.copied
    );
    for f in &out.failed {
        text.push_str(&format!("\n- 失败: {f}"));
    }
    if !out.failed.is_empty() {
        text.push_str("\n快照不完整，谨慎使用该恢复点。");
    }
    Ok(truncate_result(&text))
}

/// temp_restore：恢复到基线或快照（仅动临时副本）
async fn temp_restore(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let t = ctx.temp.as_ref().ok_or(TEMP_NO_CTX)?;
    let snapshot = args.get("snapshot").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty());
    let target = match snapshot {
        Some(name) => crate::temp::RestoreTarget::Snapshot(name.to_string()),
        None => crate::temp::RestoreTarget::Baseline,
    };
    let out = crate::temp::restore_workspace(&t.manifest, &target)?;
    let mut text = match target {
        crate::temp::RestoreTarget::Baseline => "已恢复到基线（丢弃全部未提交修改）".to_string(),
        crate::temp::RestoreTarget::Snapshot(_) => "已恢复到快照".to_string(),
    };
    for r in &out.restored {
        text.push_str(&format!("\n✅ {r}"));
    }
    for f in &out.failed {
        text.push_str(&format!("\n❌ {f}"));
    }
    if out.failed.is_empty() {
        text.push_str("\n注意：仅重置了临时副本，已合并回原目录的内容不受影响。");
    }
    Ok(truncate_result(&text))
}

/// temp_merge：把临时空间变更合并回原目录（Agent 发起，跳过运行互斥）
async fn temp_merge(ctx: &ToolCtx) -> Result<String, String> {
    let host = ctx.host.as_ref().ok_or("内部错误：缺少宿主上下文")?;
    let state = host.app.state::<crate::AppState>();
    let summary = crate::temp::merge_from_agent(&state, &host.app, &host.session_id).await?;
    Ok(truncate_result(&crate::temp::format_merge_summary(&summary)))
}

async fn list_skills_tool(ctx: &ToolCtx) -> Result<String, String> {
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

async fn save_skill_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let name = args.get("name").and_then(|v| v.as_str()).ok_or("缺少 name")?;
    let description = args.get("description").and_then(|v| v.as_str()).ok_or("缺少 description")?;
    let script_type = args.get("script_type").and_then(|v| v.as_str()).ok_or("缺少 script_type")?;
    let script_content = args.get("script_content").and_then(|v| v.as_str()).ok_or("缺少 script_content")?;

    let saved = crate::skills::save_skill(&ctx.workspace, name, description, script_type, script_content).await?;
    Ok(format!("已成功将技能【{}】保存至 `{}`，后续可通过 `run_skill` 调用该技能。", saved.name, saved.path))
}

async fn run_skill_tool(
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

async fn record_memory_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let category = args.get("category").and_then(|v| v.as_str()).ok_or("缺少 category 参数")?;
    let title = args.get("title").and_then(|v| v.as_str()).ok_or("缺少 title 参数")?;
    let content = args.get("content").and_then(|v| v.as_str()).ok_or("缺少 content 参数")?;

    crate::memory::record_memory(&ctx.workspace, category, title, content)
}

async fn read_memory_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let topic = args.get("topic").and_then(|v| v.as_str());
    crate::memory::read_memory(&ctx.workspace, topic)
}

// ---------- 任务方案计划中枢工具实现 ----------

async fn create_plan_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
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
    let db_guard = state.as_ref().map(|s| s.db.lock().unwrap());
    let db_ref = db_guard.as_deref();

    crate::plan::create_plan(
        &ctx.workspace,
        session_id,
        title,
        goals,
        architecture,
        &files,
        &steps,
        verification,
        db_ref,
    )
}

async fn update_plan_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let reason = args.get("reason").and_then(|v| v.as_str()).ok_or("缺少 reason 参数")?;
    let plan_id = args.get("plan_id").and_then(|v| v.as_str());
    let status = args.get("status").and_then(|v| v.as_str());
    let step_updates = args.get("step_updates").and_then(|v| v.as_array()).map(|v| v.as_slice());
    let modified_sections = args.get("modified_sections");
    let revision_note = args.get("revision_note").and_then(|v| v.as_str());

    let session_id = ctx.host.as_ref().map(|h| h.session_id.as_str()).unwrap_or("");
    let app = ctx.host.as_ref().map(|h| &h.app);
    let state = app.map(|a| a.state::<crate::AppState>());
    let db_guard = state.as_ref().map(|s| s.db.lock().unwrap());
    let db_ref = db_guard.as_deref();

    crate::plan::update_plan(
        &ctx.workspace,
        session_id,
        plan_id,
        reason,
        status,
        step_updates,
        modified_sections,
        revision_note,
        db_ref,
    )
}

async fn switch_plan_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let plan_id = args.get("plan_id").and_then(|v| v.as_str()).ok_or("缺少 plan_id 参数")?;
    let session_id = ctx.host.as_ref().map(|h| h.session_id.as_str()).unwrap_or("");
    let app = ctx.host.as_ref().map(|h| &h.app);
    let state = app.map(|a| a.state::<crate::AppState>());
    let db_guard = state.as_ref().map(|s| s.db.lock().unwrap());
    let db_ref = db_guard.as_deref();

    crate::plan::switch_plan(&ctx.workspace, session_id, plan_id, db_ref)
}

async fn read_plan_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let plan_id = args.get("plan_id").and_then(|v| v.as_str());
    let session_id = ctx.host.as_ref().map(|h| h.session_id.as_str()).unwrap_or("");
    let app = ctx.host.as_ref().map(|h| &h.app);
    let state = app.map(|a| a.state::<crate::AppState>());
    let db_guard = state.as_ref().map(|s| s.db.lock().unwrap());
    let db_ref = db_guard.as_deref();

    crate::plan::read_plan(&ctx.workspace, session_id, plan_id, db_ref)
}

async fn list_plans_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
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

fn build_briefing_packet(
    db: &rusqlite::Connection,
    workspace: &Path,
    parent_id: &str,
    role: &str,
    title: &str,
    task: &str,
    args: &Value,
) -> String {
    let mut packet = format!("【任务协同委派简报】\n- 角色定位：{role}\n- 任务标题：{title}\n- 详细需求描述：\n{task}\n");

    // 1. 全局权威计划与进度切片
    if let Some((plan_path, meta, body)) = crate::plan::find_plan_file(workspace, parent_id, None, Some(db)) {
        let rel_path = plan_path.strip_prefix(workspace).map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| format!(".harness/plans/{}", plan_path.file_name().and_then(|s| s.to_str()).unwrap_or("plan.md")));
        let steps = crate::plan::parse_steps_from_markdown(&body);
        let done_count = steps.iter().filter(|s| s.status == "done").count();
        let in_progress_step = steps.iter().find(|s| s.status == "in_progress").map(|s| format!("第 {} 步: {}", s.index, s.content)).unwrap_or_else(|| "（推进中）".into());

        packet.push_str(&format!(
            "\n【项目权威计划与全局约束】\n- 计划文档路径：`{rel_path}` (版本: v{})\n- 计划全局总目标：{}\n- 全局推进状态：执行中 ({}/{})\n- 当前主线阶段：{}\n- 协同准则：本任务为上述权威计划的支撑环节。如需了解全局架构，使用 `read_file` 查阅该计划文件；严禁做出违背该计划的改动或推翻全局设计！\n",
            meta.version, meta.title, done_count, steps.len(), in_progress_step
        ));
    }

    // 2. 重点已知文件（显式传入 或 自动从父会话前序最近读写工具中提取）
    let mut known_files: Vec<String> = args.get("relevant_files")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|x| x.as_str().map(|s| s.trim().to_string())).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();

    if known_files.is_empty() {
        if let Ok(msgs) = crate::store::get_messages(db, parent_id, None, 10) {
            let mut seen = std::collections::BTreeSet::new();
            for m in msgs.iter().rev() {
                for te in &m.tool_events {
                    if ["read_file", "edit_file", "write_file", "file_outline"].contains(&te.tool_name.as_str()) {
                        if let Some(p) = te.params.get("path").and_then(|v| v.as_str()) {
                            if !seen.contains(p) {
                                seen.insert(p.to_string());
                                known_files.push(p.to_string());
                                if known_files.len() >= 5 {
                                    break;
                                }
                            }
                        }
                    }
                }
                if known_files.len() >= 5 {
                    break;
                }
            }
        }
    }

    if !known_files.is_empty() {
        packet.push_str("\n【主进程已知重点文件（请直接定位切入，免去盲目探索）】\n");
        for f in &known_files {
            packet.push_str(&format!("- `{f}`\n"));
        }
    }

    // 3. 上下文图钉 / 行号锚点 (pinned_context)
    if let Some(pinned) = args.get("pinned_context").and_then(|v| v.as_array()) {
        if !pinned.is_empty() {
            packet.push_str("\n【精确上下文图钉与行号锚点】\n");
            for item in pinned {
                if let Some(obj) = item.as_object() {
                    let p = obj.get("path").and_then(|v| v.as_str()).unwrap_or("");
                    let lines = obj.get("focus_lines").map(|v| v.to_string()).unwrap_or_default();
                    let intent = obj.get("intent").and_then(|v| v.as_str()).unwrap_or("");
                    packet.push_str(&format!("- 文件 `{p}` (关注行: {lines}): {intent}\n"));
                } else if let Some(s) = item.as_str() {
                    packet.push_str(&format!("- {s}\n"));
                }
            }
        }
    }

    // 4. 交付验收标准与严格约束
    if let Some(ac) = args.get("acceptance_criteria").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()) {
        packet.push_str(&format!("\n【交付验收标准与验证命令】\n{ac}\n"));
    }
    if let Some(c) = args.get("constraints").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()) {
        packet.push_str(&format!("\n【严格禁止项与负向约束】\n{c}\n"));
    }

    packet.push_str("\n【施工工人准则】：你作为实施工人，请优先对已知目标文件进行手术级精读（read_file 局部 30~50 行）并完成代码修改或验证；严禁脱离蓝图全库漫游！");
    packet
}

fn save_subtask_report_artifact(
    workspace: &Path,
    subagent_id: &str,
    title: &str,
    role: &str,
    full_content: &str,
    touched_files: &[String],
) -> Result<String, String> {
    if workspace.as_os_str().is_empty() {
        return Err("工作区为空".into());
    }
    let subtasks_dir = workspace.join(".harness").join("subtasks");
    let _ = std::fs::create_dir_all(&subtasks_dir);
    let filename = format!("{}_report.md", subagent_id.replace('-', "_"));
    let file_path = subtasks_dir.join(&filename);

    let files_section = if touched_files.is_empty() {
        "无".to_string()
    } else {
        touched_files.iter().map(|f| format!("- `{f}`")).collect::<Vec<_>>().join("\n")
    };

    let report_md = format!(
        "# 协同任务交付报告: {title}\n\n- **子任务 ID**: `{subagent_id}`\n- **角色定位**: {role}\n- **生成时间**: {}\n- **涉及改动文件**:\n{files_section}\n\n---\n\n## 详细报告与产出内容\n\n{full_content}\n",
        chrono::Utc::now().to_rfc3339()
    );

    let _ = std::fs::write(&file_path, report_md).map_err(|e| format!("保存工件失败: {e}"))?;
    Ok(format!(".harness/subtasks/{}", filename))
}

fn extract_compact_summary(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut count = 0;
    let mut end_idx = trimmed.len();
    for (i, _) in trimmed.char_indices() {
        if count >= max_chars {
            end_idx = i;
            break;
        }
        count += 1;
    }
    format!("{}...\n\n*(注：内容已自动压缩提炼为决策摘要，完整报告细节请查阅上方工件文件)*", &trimmed[..end_idx])
}

// ---------- 子 Agent 协作工具具体实现 ----------

async fn spawn_subagent_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let host = ctx.host.as_ref().ok_or("内部错误：缺少宿主上下文")?;
    let role = args.get("role").and_then(|v| v.as_str()).ok_or("缺少 role 参数")?;
    let title = args.get("title").and_then(|v| v.as_str()).ok_or("缺少 title 参数")?;
    let task = args.get("task").and_then(|v| v.as_str()).ok_or("缺少 task 参数")?;

    let state = host.app.state::<crate::AppState>();
    let parent_id = &host.session_id;

    let (parent_session, current_subs_count) = {
        let db = state.db.lock().unwrap();
        let p = crate::store::get_session(&db, parent_id)?.ok_or("父会话不存在")?;
        // 校验递归深度：子 Agent 会话禁止再次创建子 Agent
        if p.session_type == "subagent" || p.parent_session_id.is_some() {
            return Err("子 Agent 不允许递归创建新的子 Agent，请直接由当前子 Agent 完成指定任务。".into());
        }
        let subs = crate::store::list_subagents(&db, parent_id)?;
        (p, subs.len())
    };

    if current_subs_count >= 10 {
        return Err("子 Agent 数量已达上限 (10)，请等待部分子任务完成或停止后再创建。".into());
    }

    // 子 Agent 工作区严格与父会话保持完全一致
    let sub_workspace = parent_session.workspace_path.clone();

    let (sub, user_msg) = {
        let db = state.db.lock().unwrap();
        let sub = crate::store::create_subagent_session(
            &db,
            parent_id,
            role,
            title,
            task,
            &sub_workspace,
            parent_session.access_mode.as_deref(),
            parent_session.project_id.as_deref(),
            ctx.event_id.as_deref(),
        )?;
        // 关键强绑定：立即将生成的真实 sub.id 回填并持久化至当前 tool_event
        if let Some(ref ev_id) = ctx.event_id {
            let mut params_with_id = args.clone();
            if let Some(obj) = params_with_id.as_object_mut() {
                obj.insert("subprocess_id".into(), json!(sub.id));
            }
            let _ = crate::store::set_tool_event_subprocess_id(
                &db,
                ev_id,
                &sub.id,
                Some(&params_with_id.to_string()),
            );
        }
        let initial_prompt = build_briefing_packet(&db, &ctx.workspace, parent_id, role, title, task, args);
        let user_msg = crate::store::new_message(&db, &sub.id, "user", Some(initial_prompt), false)?;
        (sub, user_msg)
    };

    // 启动子 Agent 异步运行循环
    crate::agent::spawn_session_task(host.app.clone(), sub.id.clone(), Some(user_msg.id));

    // 广播事件通知前端刷新子 Agent / 子进程列表（附带 toolEventId 与 subprocess 强关联）
    let _ = host.app.emit("subprocess:created", json!({
        "parentId": parent_id,
        "subprocess": sub,
        "toolEventId": ctx.event_id,
    }));
    let _ = host.app.emit("subprocesses:changed", json!({
        "parentId": parent_id,
    }));
    let _ = host.app.emit("subagent:created", json!({
        "parentId": parent_id,
        "subagent": sub,
        "toolEventId": ctx.event_id,
    }));
    let _ = host.app.emit("subagents:changed", json!({
        "parentId": parent_id,
    }));

    Ok(format!(
        "已成功创建并启动子 Agent 进程！\n- ID: `{}`\n- 角色: {}\n- 标题: {}\n- 状态: 运行中 (running)\n\n子 Agent 正在独立上下文中执行，用户点击界面右侧可实时查看其完整对话流程与工具卡片。后续可通过 `wait_subagents` 或 `get_subagent_status` 协同跟进。",
        sub.id, role, title
    ))
}

fn determine_subagent_status(
    is_running: bool,
    session_status: &str,
    has_terminal_reply: bool,
    has_max_steps_msg: bool,
) -> (&'static str, &'static str) {
    if is_running {
        ("running", "🟡 仍在运行 (running)")
    } else if session_status == "cancelled" {
        ("cancelled", "⚪ 已取消/停止 (cancelled)")
    } else if session_status == "failed" {
        ("failed", "🔴 运行失败出错 (failed - 可调用 resume_subprocess 恢复)")
    } else if session_status == "interrupted" {
        ("interrupted", "🟠 执行中断未完成 (interrupted - 可调用 resume_subprocess 恢复)")
    } else if has_max_steps_msg {
        ("interrupted", "🟠 已达最大步数上限中止 (max_steps_reached - 可调用 resume_subprocess 继续推进)")
    } else if !has_terminal_reply {
        ("interrupted", "🟠 执行中断未完全收敛 (interrupted - 可调用 resume_subprocess 恢复)")
    } else {
        ("completed", "🟢 已完成交付 (completed)")
    }
}

async fn get_subagent_status_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let host = ctx.host.as_ref().ok_or("内部错误：缺少宿主上下文")?;
    let target_id = args.get("subagent_id")
        .or_else(|| args.get("subprocess_id"))
        .and_then(|v| v.as_str());
    let state = host.app.state::<crate::AppState>();
    let parent_id = &host.session_id;

    let subs = {
        let db = state.db.lock().unwrap();
        crate::store::list_subagents(&db, parent_id)?
    };

    if subs.is_empty() {
        return Ok("当前会话尚未创建任何子 Agent 进程。".into());
    }

    let filtered: Vec<&crate::models::Session> = if let Some(tid) = target_id {
        subs.iter().filter(|s| s.id == tid).collect()
    } else {
        subs.iter().collect()
    };

    if filtered.is_empty() {
        return Ok(format!("未找到指定 ID 的子 Agent: {}", target_id.unwrap_or_default()));
    }

    let mut out = format!("共查询到 {} 个子 Agent 状态：\n\n", filtered.len());
    let db = state.db.lock().unwrap();
    for s in filtered {
        let is_running = crate::agent::is_run_active(&state, &s.id);
        let msgs = crate::store::get_messages(&db, &s.id, None, 50).unwrap_or_default();
        let last_reply = msgs
            .iter()
            .rev()
            .find(|m| m.role == "assistant" && !m.content.as_deref().unwrap_or("").is_empty())
            .and_then(|m| m.content.as_deref())
            .unwrap_or("(尚在准备或执行工具中)");
        let last_asst = msgs.iter().rev().find(|m| m.role == "assistant");
        let has_terminal_reply = last_asst.map(|m| {
            m.tool_calls.is_none()
                || m.tool_calls.as_ref().map(|t| t.is_null() || t.as_array().map(|a| a.is_empty()).unwrap_or(false)).unwrap_or(false)
        }).unwrap_or(false);
        let has_max_steps_msg = msgs.iter().rev().any(|m| m.role == "system" && m.content.as_deref().unwrap_or("").contains("已达到最大步数"));

        let (_code, status_display) = determine_subagent_status(
            is_running,
            &s.status,
            has_terminal_reply,
            has_max_steps_msg,
        );

        let preview = if last_reply.len() > 300 {
            format!("{}...", &last_reply[..last_reply.char_indices().nth(300).map(|(i,_)| i).unwrap_or(last_reply.len())])
        } else {
            last_reply.to_string()
        };

        out.push_str(&format!(
            "- **【{}】** (ID: `{}`)\n  角色: {}\n  状态: {}\n  Token消耗: {}\n  最新输出摘要: {}\n\n",
            s.title,
            s.id,
            s.subagent_role.as_deref().unwrap_or("协作助手"),
            status_display,
            s.total_tokens.unwrap_or(0),
            preview
        ));
    }

    Ok(out)
}

async fn wait_subagents_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let host = ctx.host.as_ref().ok_or("内部错误：缺少宿主上下文")?;
    let timeout_secs = get_u64_arg(args, "timeout_seconds", 60).clamp(5, 300);
    let specified_ids: Option<Vec<String>> = args.get("subagent_ids")
        .or_else(|| args.get("subprocess_ids"))
        .and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()
            })
        });

    let state = host.app.state::<crate::AppState>();
    let parent_id = &host.session_id;

    let target_ids: Vec<String> = {
        let db = state.db.lock().unwrap();
        let subs = crate::store::list_subagents(&db, parent_id)?;
        if let Some(ids) = specified_ids {
            subs.into_iter().filter(|s| ids.contains(&s.id)).map(|s| s.id).collect()
        } else {
            subs.into_iter().map(|s| s.id).collect()
        }
    };

    if target_ids.is_empty() {
        return Ok("没有找到需要等待的子 Agent。".into());
    }

    let start_wait = std::time::Instant::now();
    let max_wait = std::time::Duration::from_secs(timeout_secs);
    let mut auto_recovered = std::collections::HashSet::new();

    loop {
        let mut any_running = false;
        for id in &target_ids {
            let is_running = crate::agent::is_run_active(&state, id);
            if is_running {
                any_running = true;
            } else if !auto_recovered.contains(id) {
                // 检查是否是非正常中断停止（未交付且未显式取消），若发生意外中断且尚有等待时间，自动触发一次自愈恢复
                let should_recover = {
                    let db = state.db.lock().unwrap();
                    if let Ok(Some(s)) = crate::store::get_session(&db, id) {
                        if s.status == "interrupted" || s.status == "failed" {
                            true
                        } else if s.status != "cancelled" {
                            let msgs = crate::store::get_messages(&db, id, None, 10).unwrap_or_default();
                            let last_asst = msgs.iter().rev().find(|m| m.role == "assistant");
                            let has_terminal_reply = last_asst.map(|m| {
                                m.tool_calls.is_none()
                                    || m.tool_calls.as_ref().map(|t| t.is_null() || t.as_array().map(|a| a.is_empty()).unwrap_or(false)).unwrap_or(false)
                            }).unwrap_or(false);
                            !has_terminal_reply
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                };

                if should_recover && start_wait.elapsed() < max_wait {
                    auto_recovered.insert(id.clone());
                    let _ = crate::agent::restart_subagent(&host.app, id);
                    any_running = true;
                }
            }
        }

        if !any_running || start_wait.elapsed() >= max_wait {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    let mut out = String::new();
    let is_timed_out = start_wait.elapsed() >= max_wait;

    let db = state.db.lock().unwrap();
    let mut any_interrupted_or_failed = false;
    let mut any_still_running = false;
    let mut sub_results = Vec::new();

    for id in &target_ids {
        let is_running = crate::agent::is_run_active(&state, id);
        if is_running {
            any_still_running = true;
        }
        let s = crate::store::get_session(&db, id)?.unwrap_or_else(|| {
            crate::models::Session {
                id: id.clone(),
                title: "未知子Agent".into(),
                parent_session_id: Some(parent_id.clone()),
                session_type: "subagent".into(),
                ..Default::default()
            }
        });
        let msgs = crate::store::get_messages(&db, id, None, 50).unwrap_or_default();
        let last_reply = msgs
            .iter()
            .rev()
            .find(|m| m.role == "assistant" && !m.content.as_deref().unwrap_or("").is_empty())
            .and_then(|m| m.content.as_deref())
            .unwrap_or("(未产生文本回复)");

        let last_asst = msgs.iter().rev().find(|m| m.role == "assistant");
        let has_terminal_reply = last_asst.map(|m| {
            m.tool_calls.is_none()
                || m.tool_calls.as_ref().map(|t| t.is_null() || t.as_array().map(|a| a.is_empty()).unwrap_or(false)).unwrap_or(false)
        }).unwrap_or(false);
        let has_max_steps_msg = msgs.iter().rev().any(|m| m.role == "system" && m.content.as_deref().unwrap_or("").contains("已达到最大步数"));

        let (status_code, status_display) = determine_subagent_status(
            is_running,
            &s.status,
            has_terminal_reply,
            has_max_steps_msg,
        );
        if status_code == "interrupted" || status_code == "failed" {
            any_interrupted_or_failed = true;
        }

        let display_reply = if has_max_steps_msg {
            format!("⚠️ 该子任务已达到最大步数上限中止，未生成最终交付报告。最后思考或动作：\n{}", last_reply)
        } else if status_code == "interrupted" && last_reply != "(未产生文本回复)" {
            format!("⚠️ 该子任务执行中断或未完全收敛交付。最后思考或动作：\n{}", last_reply)
        } else {
            last_reply.to_string()
        };

        let mut touched_files = std::collections::BTreeSet::new();
        for m in &msgs {
            for te in &m.tool_events {
                if ["write_file", "edit_file", "apply_diff"].contains(&te.tool_name.as_str()) {
                    if let Some(p) = te.params.get("path").and_then(|v| v.as_str()) {
                        touched_files.insert(p.to_string());
                    }
                }
            }
        }
        let touched_vec: Vec<String> = touched_files.into_iter().collect();
        let touched_line = if touched_vec.is_empty() {
            String::new()
        } else {
            let list = touched_vec.iter().map(|f| format!("`{f}`")).collect::<Vec<_>>().join(", ");
            format!("- 涉及改动文件: {}\n", list)
        };

        // 自动沉淀技术分析类子任务成果至项目技术大盘 (profile.md)
        let is_tech_analysis = {
            let role_str = s.subagent_role.as_deref().unwrap_or("");
            let title_str = s.title.as_str();
            let task_str = s.subagent_task.as_deref().unwrap_or("");
            role_str.contains("技术栈") || role_str.contains("架构")
                || title_str.contains("技术栈") || title_str.contains("架构")
                || task_str.contains("技术栈") || task_str.contains("依赖分析")
        };
        if is_tech_analysis && !is_running && has_terminal_reply && last_reply.len() > 50 && last_reply != "(未产生文本回复)" {
            let already_recorded = msgs.iter().any(|m| {
                m.tool_events.iter().any(|te| te.tool_name == "record_memory")
            });
            if !already_recorded && !ctx.workspace.as_os_str().is_empty() {
                let _ = crate::memory::record_memory(
                    &ctx.workspace,
                    "profile",
                    &s.title,
                    last_reply,
                );
            }
        }

        // 双轨交付：将长文本落地到工件文件 .harness/subtasks/{subagent_id}_report.md
        let artifact_path_opt = if !ctx.workspace.as_os_str().is_empty() && (display_reply.len() > 250 || display_reply.contains("```") || !touched_vec.is_empty()) {
            save_subtask_report_artifact(
                &ctx.workspace,
                id,
                &s.title,
                s.subagent_role.as_deref().unwrap_or("协作助手"),
                &display_reply,
                &touched_vec,
            ).ok()
        } else {
            None
        };

        let compact_summary = extract_compact_summary(&display_reply, 350);
        let artifact_line = if let Some(ref ap) = artifact_path_opt {
            format!("- 📄 完整技术报告工件: [查看完整详细报告](file:///{})\n", ap.replace('\\', "/"))
        } else {
            String::new()
        };

        sub_results.push(format!(
            "### 协同子 Agent: {} ({})\n- 状态: {}\n- Token 消耗: {}\n{}{}- 交付核心结论：\n```markdown\n{}\n```\n\n",
            s.title,
            s.subagent_role.as_deref().unwrap_or("协作助手"),
            status_display,
            s.total_tokens.unwrap_or(0),
            touched_line,
            artifact_line,
            compact_summary
        ));
    }

    if is_timed_out || any_still_running {
        out.push_str(&format!("⚠️ 等待达到超时上限（{timeout_secs}s），部分子 Agent 可能仍在后台继续运行。\n\n"));
    } else if any_interrupted_or_failed {
        out.push_str("⚠️ 部分子 Agent 执行中断或未完成交付（可在下一步调用 resume_subprocess 继续恢复推进）：\n\n");
    } else {
        out.push_str("✅ 所有目标子 Agent 执行完毕！汇总结果如下：\n\n");
    }

    for sr in sub_results {
        out.push_str(&sr);
    }

    Ok(out)
}

async fn stop_subagent_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let host = ctx.host.as_ref().ok_or("内部错误：缺少宿主上下文")?;
    let subagent_id = args.get("subagent_id").and_then(|v| v.as_str()).ok_or("缺少 subagent_id 参数")?;
    {
        let state = host.app.state::<crate::AppState>();
        let db = state.db.lock().unwrap();
        let _ = crate::store::set_session_status(&db, subagent_id, "cancelled");
    }
    crate::agent::stop_session(&host.app, subagent_id);
    let _ = host.app.emit("subagent:update", json!({
        "parentId": host.session_id,
        "subagentId": subagent_id,
        "status": "cancelled"
    }));
    let _ = host.app.emit("subprocess:update", json!({
        "parentId": host.session_id,
        "parentSessionId": host.session_id,
        "subprocessId": subagent_id,
        "status": "cancelled"
    }));
    let _ = host.app.emit("subagents:changed", json!({
        "parentId": host.session_id
    }));
    let _ = host.app.emit("subprocesses:changed", json!({
        "parentId": host.session_id
    }));
    Ok(format!("已成功停止子 Agent【{subagent_id}】。"))
}

async fn resume_subagent_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let host = ctx.host.as_ref().ok_or("内部错误：缺少宿主上下文")?;
    let subagent_id = args.get("subagent_id")
        .or_else(|| args.get("subprocess_id"))
        .and_then(|v| v.as_str())
        .ok_or("缺少 subagent_id 或 subprocess_id 参数")?;

    let state = host.app.state::<crate::AppState>();
    let parent_id = &host.session_id;

    // 校验子 Agent 是否存在且属于当前主会话
    {
        let db = state.db.lock().unwrap();
        let s = crate::store::get_session(&db, subagent_id)?.ok_or("未找到指定的子 Agent")?;
        if s.parent_session_id.as_deref() != Some(parent_id) {
            return Err("指定的子 Agent 不属于当前主会话".into());
        }
    }

    crate::agent::restart_subagent(&host.app, subagent_id)?;

    Ok(format!(
        "已成功恢复并重新启动子 Agent【{}】。它正在后台继续推进执行，你可以调用 wait_subagents 工具等待其最新执行成果。",
        subagent_id
    ))
}

async fn dispatch_collaborator_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let host = ctx.host.as_ref().ok_or("内部错误：缺少宿主上下文")?;
    let collaborator_id = args.get("collaborator_id").and_then(|v| v.as_str()).ok_or("缺少 collaborator_id 参数")?;
    let task = args.get("task").and_then(|v| v.as_str()).ok_or("缺少 task 参数")?;

    let state = host.app.state::<crate::AppState>();
    let parent_id = &host.session_id;

    let collab = {
        let db = state.db.lock().unwrap();
        let s = crate::store::get_session(&db, collaborator_id)?.ok_or("未找到指定的协作者")?;
        if s.parent_session_id.as_deref() != Some(parent_id) {
            return Err("指定的协作者不属于当前主会话".into());
        }
        s
    };

    if crate::agent::is_run_active(&state, collaborator_id) {
        return Err(format!("协作者【{}】当前正在运行中，请先调用 wait_collaborators 等待其完成，再指派新任务。", collab.title));
    }

    let user_prompt = {
        let db = state.db.lock().unwrap();
        build_briefing_packet(
            &db,
            &ctx.workspace,
            parent_id,
            collab.subagent_role.as_deref().unwrap_or("协作者"),
            &collab.title,
            task,
            args,
        )
    };
    let user_msg = {
        let db = state.db.lock().unwrap();
        let _ = crate::store::set_kv(&db, collaborator_id, "dispatched_by_parent", "true");
        crate::store::new_message(&db, collaborator_id, "user", Some(user_prompt), false)?
    };

    crate::agent::spawn_session_task(host.app.clone(), collaborator_id.to_string(), Some(user_msg.id));

    Ok(format!(
        "已成功向协作者【{}】(角色: {}) 委派任务！\n- 任务要求: {}\n- 状态: 运行中 (running)\n\n协作者已开始独立执行。你可以紧接着调用 wait_collaborators 等待并获取其增量产出报告。",
        collab.title,
        collab.subagent_role.as_deref().unwrap_or("协作者"),
        task
    ))
}

async fn wait_collaborators_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let host = ctx.host.as_ref().ok_or("内部错误：缺少宿主上下文")?;
    let timeout_secs = get_u64_arg(args, "timeout_seconds", 90).clamp(5, 300);
    let specified_ids: Option<Vec<String>> = args.get("collaborator_ids").and_then(|v| {
        v.as_array().map(|arr| {
            arr.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()
        })
    });

    let state = host.app.state::<crate::AppState>();
    let parent_id = &host.session_id;

    let target_ids: Vec<String> = {
        let db = state.db.lock().unwrap();
        let collabs = crate::store::list_collaborators(&db, parent_id)?;
        if let Some(ids) = specified_ids {
            collabs.into_iter().filter(|s| ids.contains(&s.id)).map(|s| s.id).collect()
        } else {
            collabs.into_iter().map(|s| s.id).collect()
        }
    };

    if target_ids.is_empty() {
        return Ok("当前项目暂无需要等待的协作者。".into());
    }

    let start_wait = std::time::Instant::now();
    let max_wait = std::time::Duration::from_secs(timeout_secs);

    loop {
        let any_running = target_ids.iter().any(|id| crate::agent::is_run_active(&state, id));
        if !any_running || start_wait.elapsed() >= max_wait {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    let mut out = String::new();
    let is_timed_out = start_wait.elapsed() >= max_wait;
    if is_timed_out {
        out.push_str(&format!("⚠️ 等待达到超时上限（{timeout_secs}s），部分协作者可能仍在后台运行。\n\n"));
    } else {
        out.push_str("✅ 目标协作者本轮执行完毕！汇总增量成果如下：\n\n");
    }

    let db = state.db.lock().unwrap();
    for id in &target_ids {
        let is_running = crate::agent::is_run_active(&state, id);
        let s = match crate::store::get_session(&db, id)? {
            Some(s) => s,
            None => continue,
        };
        let msgs = crate::store::get_messages(&db, id, None, 50).unwrap_or_default();

        let slice: Vec<&crate::models::Message> = if let Some(ref w_id) = s.last_reported_msg_id {
            if let Some(pos) = msgs.iter().position(|m| &m.id == w_id) {
                msgs[pos + 1..].iter().collect()
            } else {
                msgs.iter().collect()
            }
        } else {
            msgs.iter().collect()
        };

        let last_assistant = slice
            .iter()
            .rev()
            .find(|m| m.role == "assistant" && !m.content.as_deref().unwrap_or("").is_empty())
            .copied()
            .or_else(|| {
                // 兜底保护：若水位线之后无增量匹配，取该协作者全量消息中最新一条有效回复
                msgs.iter().rev().find(|m| m.role == "assistant" && !m.content.as_deref().unwrap_or("").is_empty())
            });

        let last_reply = match last_assistant {
            Some(m) => m.content.as_deref().unwrap_or("(未产生文本回复)"),
            None => "(本轮暂无新增文本产出)",
        };

        let mut touched_files = std::collections::BTreeSet::new();
        let files_source: Vec<&crate::models::Message> = if !slice.is_empty() {
            slice
        } else {
            msgs.iter().collect()
        };
        for m in &files_source {
            for te in &m.tool_events {
                if ["write_file", "edit_file", "apply_diff"].contains(&te.tool_name.as_str()) {
                    if let Some(p) = te.params.get("path").and_then(|v| v.as_str()) {
                        touched_files.insert(p.to_string());
                    }
                }
            }
        }
        let touched_vec: Vec<String> = touched_files.into_iter().collect();
        let touched_line = if touched_vec.is_empty() {
            String::new()
        } else {
            let list = touched_vec.iter().map(|f| format!("`{f}`")).collect::<Vec<_>>().join(", ");
            format!("- 涉及改动文件: {}\n", list)
        };

        // 双轨交付：将长文本落地到工件文件 .harness/subtasks/{collaborator_id}_report.md
        let artifact_path_opt = if !ctx.workspace.as_os_str().is_empty() && (last_reply.len() > 250 || last_reply.contains("```") || !touched_vec.is_empty()) {
            save_subtask_report_artifact(
                &ctx.workspace,
                id,
                &s.title,
                s.subagent_role.as_deref().unwrap_or("协作者"),
                last_reply,
                &touched_vec,
            ).ok()
        } else {
            None
        };

        let compact_summary = extract_compact_summary(last_reply, 350);
        let artifact_line = if let Some(ref ap) = artifact_path_opt {
            format!("- 📄 完整技术报告工件: [查看完整详细报告](file:///{})\n", ap.replace('\\', "/"))
        } else {
            String::new()
        };

        let last_asst = msgs.iter().rev().find(|m| m.role == "assistant");
        let has_terminal_reply = last_asst.map(|m| {
            m.tool_calls.is_none()
                || m.tool_calls.as_ref().map(|t| t.is_null() || t.as_array().map(|a| a.is_empty()).unwrap_or(false)).unwrap_or(false)
        }).unwrap_or(false);
        let has_max_steps_msg = msgs.iter().rev().any(|m| m.role == "system" && m.content.as_deref().unwrap_or("").contains("已达到最大步数"));
        let (_code, status_display) = determine_subagent_status(
            is_running,
            &s.status,
            has_terminal_reply,
            has_max_steps_msg,
        );

        if let Some(m) = last_assistant {
            let _ = crate::store::update_collaborator_watermark(&db, id, &m.id);
        }

        out.push_str(&format!(
            "### 协作者: {} ({})\n- 状态: {}\n- Token 消耗: {}\n{}{}- 增量交付结论：\n```markdown\n{}\n```\n\n",
            s.title,
            s.subagent_role.as_deref().unwrap_or("协作者"),
            status_display,
            s.total_tokens.unwrap_or(0),
            touched_line,
            artifact_line,
            compact_summary
        ));
    }

    Ok(out)
}

async fn get_collaborators_tool(ctx: &ToolCtx) -> Result<String, String> {
    let host = ctx.host.as_ref().ok_or("内部错误：缺少宿主上下文")?;
    let state = host.app.state::<crate::AppState>();
    let parent_id = &host.session_id;

    let collabs = {
        let db = state.db.lock().unwrap();
        crate::store::list_collaborators(&db, parent_id)?
    };

    if collabs.is_empty() {
        return Ok("当前主会话尚未配置任何常驻协作者。如需并行处理任务，可由用户在界面顶部创建协作者，或由你调用 spawn_subprocess 派生临时子进程。".into());
    }

    let mut out = format!("当前已配置 {} 位项目协作者：\n\n", collabs.len());
    let db = state.db.lock().unwrap();
    for c in collabs {
        let is_running = crate::agent::is_run_active(&state, &c.id);
        let msgs = crate::store::get_messages(&db, &c.id, None, 10).unwrap_or_default();
        let last_reply = msgs
            .iter()
            .rev()
            .find(|m| m.role == "assistant" && !m.content.as_deref().unwrap_or("").is_empty())
            .and_then(|m| m.content.as_deref())
            .unwrap_or("(就绪待命)");
        let preview = if last_reply.len() > 200 {
            format!("{}...", &last_reply[..last_reply.char_indices().nth(200).map(|(i,_)| i).unwrap_or(last_reply.len())])
        } else {
            last_reply.to_string()
        };

        out.push_str(&format!(
            "- **【{}】** (ID: `{}`)\n  角色: {}\n  状态: {}\n  初始职责: {}\n  最新进展: {}\n\n",
            c.title,
            c.id,
            c.subagent_role.as_deref().unwrap_or("协作者"),
            if is_running { "🟡 运行中 (busy)" } else { "🟢 空闲中 (idle)" },
            c.subagent_task.as_deref().unwrap_or("负责该领域工作"),
            preview
        ));
    }

    Ok(out)
}

async fn generate_image_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
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

    // 解析生图模型 (Provider, ModelName)：
    // 1. 若工具参数显式指定了 model，优先在所有厂商中匹配
    // 2. 检查会话专属生图模型 image_provider_id / image_model_id
    // 3. 检查会话专属模型 provider_id / model_id 是否具备 image_gen 能力
    // 4. 检查会话所属 provider 是否有其他模型具备 image_gen 能力
    // 5. 检查全局 active_image_provider_id / active_image_model_id
    // 6. 查找全局任意厂商中具备 image_gen 能力的模型
    let resolved = if let Some(req_m) = requested_model {
        settings.providers.iter().find(|p| p.models.iter().any(|m| m == req_m))
            .map(|p| (p.clone(), req_m.to_string()))
            .or_else(|| crate::models::resolve_active_model(&settings).map(|(p, _)| (p.clone(), req_m.to_string())))
    } else if let (Some(pid), Some(mid)) = (&session.image_provider_id, &session.image_model_id) {
        settings.providers.iter().find(|p| &p.id == pid)
            .map(|p| (p.clone(), mid.clone()))
    } else if let (Some(pid), Some(mid)) = (&session.provider_id, &session.model_id) {
        if settings.has_capability(Some(pid), mid, "image_gen") {
            settings.providers.iter().find(|p| &p.id == pid).map(|p| (p.clone(), mid.clone()))
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

    let cfg = crate::llm::LlmCfg {
        base_url: provider.base_url.clone(),
        api_key: provider.api_key.clone(),
        model: model_name.clone(),
    };

    let img_bytes = crate::llm::generate_image_api(&cfg, prompt, size).await?;

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

    tokio::fs::write(&target_path, &img_bytes).await.map_err(|e| format!("写入图片文件失败: {e}"))?;

    let raw_target_str = target_path.to_string_lossy().to_string();
    let target_str = raw_target_str.replace('\\', "/");
    let size_display = size.unwrap_or("1024x1024");
    let kb = img_bytes.len() / 1024;

    Ok(format!(
        "🎨 图片生成成功！\n- 保存路径: `{target_str}`\n- 使用模型: `{model_name}`\n- 提示词: {prompt}\n- 分辨率: {size_display}\n- 大小: {kb} KB\n\n![{prompt}]({target_str})"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_glob_regex() {
        let re = regex::Regex::new(&glob_to_regex("src/**/*.rs")).unwrap();
        assert!(re.is_match("src/a/b.rs"));
        assert!(re.is_match("src/main.rs"));
        assert!(!re.is_match("other/main.rs"));

        let re2 = regex::Regex::new(&glob_to_regex("*.json")).unwrap();
        assert!(re2.is_match("package.json"));
        assert!(!re2.is_match("a/package.json"));
    }

    #[test]
    fn test_high_danger() {
        assert!(is_high_danger("rm -rf /"));
        assert!(is_high_danger("git reset --hard HEAD~1"));
        assert!(is_high_danger("del /s /q *.tmp"));
        assert!(!is_high_danger("npm run build"));
        assert!(!is_high_danger("cargo test"));
    }

    #[test]
    fn test_extract_outline_rust() {
        let code = r#"
// Some comment
pub struct User {
    pub name: String,
}

impl User {
    pub fn new(name: &str) -> Self {
        Self { name: name.to_string() }
    }
}

async fn fetch_data() -> Result<(), ()> {
    Ok(())
}
"#;
        let outline = extract_outline(code, "rs");
        assert!(outline.contains("pub struct User"));
        assert!(outline.contains("impl User"));
        assert!(outline.contains("pub fn new"));
        assert!(outline.contains("async fn fetch_data"));
        assert!(!outline.contains("Some comment"));
        assert!(!outline.contains("pub name: String"));
    }

    #[test]
    fn test_extract_outline_ts() {
        let code = r#"
export interface Item {
  id: string;
}

export const LIST = [1, 2];

export function getItem(): Item {
  return { id: "1" };
}

const handleClick = async () => {
  console.log("clicked");
};
"#;
        let outline = extract_outline(code, "ts");
        assert!(outline.contains("export interface Item"));
        assert!(outline.contains("export const LIST"));
        assert!(outline.contains("export function getItem"));
        assert!(outline.contains("const handleClick"));
    }

    #[test]
    fn test_inside_workspace() {
        let temp = std::env::temp_dir();
        let ctx = ToolCtx {
            workspace: temp.clone(),
            sandbox_root: None,
            command_timeout: std::time::Duration::from_secs(10),
            temp: None,
            host: None,
            event_id: None,
        };

        // 工作区内已存在目录
        assert!(inside_workspace(&ctx, "."));
        // 工作区内新建文件（不存在）
        assert!(inside_workspace(&ctx, "non_existent_file_abc123.txt"));
        // 工作区内多层不存在子路径
        assert!(inside_workspace(&ctx, "a/b/c/new_file.txt"));
        // 尝试越界
        assert!(!inside_workspace(&ctx, "../../outside_something_abc123"));
    }

    #[test]
    fn test_args_helpers() {
        let json_data = json!({
            "num": 123,
            "str_num": "456",
            "invalid_num": "not_a_num",
            "bool_val": true,
            "str_bool_true": "true",
            "str_bool_false": "FALSE",
            "num_bool_1": 1,
            "num_bool_0": 0
        });

        assert_eq!(get_u64_arg(&json_data, "num", 0), 123);
        assert_eq!(get_u64_arg(&json_data, "str_num", 0), 456);
        assert_eq!(get_u64_arg(&json_data, "invalid_num", 99), 99);
        assert_eq!(get_u64_arg(&json_data, "non_existent", 42), 42);

        assert_eq!(get_bool_arg(&json_data, "bool_val", false), true);
        assert_eq!(get_bool_arg(&json_data, "str_bool_true", false), true);
        assert_eq!(get_bool_arg(&json_data, "str_bool_false", true), false);
        assert_eq!(get_bool_arg(&json_data, "num_bool_1", false), true);
        assert_eq!(get_bool_arg(&json_data, "num_bool_0", true), false);
        assert_eq!(get_bool_arg(&json_data, "non_existent", true), true);
    }

    #[test]
    fn test_resolve_slash_trim() {
        let temp = std::env::temp_dir();
        let ctx = ToolCtx {
            workspace: temp.clone(),
            sandbox_root: None,
            command_timeout: std::time::Duration::from_secs(10),
            temp: None,
            host: None,
            event_id: None,
        };

        let resolved_normal = resolve(&ctx, "src/main.rs");
        let resolved_slash = resolve(&ctx, "/src/main.rs");
        let resolved_backslash = resolve(&ctx, "\\src\\main.rs");

        assert_eq!(resolved_normal, temp.join("src/main.rs"));
        assert_eq!(resolved_slash, temp.join("src/main.rs"));
        assert_eq!(resolved_backslash, temp.join("src\\main.rs"));
    }

    #[test]
    fn test_glob_to_regex_braces() {
        let re_str = glob_to_regex("*.{js,ts}");
        let re = regex::Regex::new(&re_str).unwrap();
        assert!(re.is_match("index.js"));
        assert!(re.is_match("index.ts"));
        assert!(!re.is_match("index.rs"));

        let re_nested = glob_to_regex("src/**/*.{java,kt}");
        let re2 = regex::Regex::new(&re_nested).unwrap();
        assert!(re2.is_match("src/models/User.java"));
        assert!(re2.is_match("src/User.kt"));
        assert!(!re2.is_match("src/User.cpp"));
    }

    #[test]
    fn test_find_match_line_numbers() {
        let text = "alpha\nbeta\ngamma\nbeta\nomega";
        let lines = find_match_line_numbers(text, "beta");
        assert_eq!(lines, vec![2, 4]);
    }

    #[test]
    fn test_diagnose_edit_mismatch() {
        let text = "public class Hello {\n    void run() {\n        System.out.println(\"ok\");\n    }\n}";
        // 缩进不匹配
        let old = "  void run() {\n      System.out.println(\"ok\");\n  }";
        let diag = diagnose_edit_mismatch(text, old);
        assert!(diag.contains("相似行") || diag.contains("忽略缩进"));

        // 完全没有的
        let not_found = "class NonExistentFooBarBaz {}";
        let diag2 = diagnose_edit_mismatch(text, not_found);
        assert!(diag2.contains("系统已自动统一 CRLF/LF"));
    }

    #[tokio::test]
    async fn test_edit_file_crlf_preservation() {
        let temp_dir = std::env::temp_dir().join(format!("harness_test_edit_{}", uuid::Uuid::new_v4()));
        let _ = tokio::fs::create_dir_all(&temp_dir).await;
        let test_file = temp_dir.join("Test.java");

        // 写入 CRLF 格式原始文件
        let original_crlf = "public class Test {\r\n    private int a;\r\n}\r\n";
        tokio::fs::write(&test_file, original_crlf).await.unwrap();

        let ctx = ToolCtx {
            workspace: temp_dir.clone(),
            sandbox_root: None,
            command_timeout: std::time::Duration::from_secs(10),
            temp: None,
            host: None,
            event_id: None,
        };

        // LLM 发送 LF 换行的 old_string 与 new_string
        let args = json!({
            "path": "Test.java",
            "old_string": "    private int a;\n",
            "new_string": "    private int a;\n    private int b;\n"
        });

        let res = edit_file(&args, &ctx).await;
        assert!(res.is_ok(), "edit_file 应该成功匹配 LF 与 CRLF: {:?}", res.err());

        // 读取磁盘，验证仍为 CRLF
        let modified = tokio::fs::read_to_string(&test_file).await.unwrap();
        assert!(modified.contains("\r\n"), "应当保留文件原有的 CRLF 换行格式");
        assert!(modified.contains("private int b;"));

        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }

    #[test]
    fn test_briefing_packet_and_artifact_summary() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::store::init_schema(&conn).unwrap();
        let ws = std::env::temp_dir().join(format!("harness_test_briefing_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&ws).unwrap();

        let parent_sess = crate::store::create_session(&conn, &ws.to_string_lossy(), None, "主会话", "standard").unwrap();

        // 1. 测试简报生成
        let args = json!({
            "relevant_files": ["src/auth.rs", "src/token.rs"],
            "acceptance_criteria": "cargo test test_auth",
            "constraints": "严禁修改外部接口"
        });
        let packet = build_briefing_packet(&conn, &ws, &parent_sess.id, "后端工程师", "重构JWT", "实现Token无感刷新", &args);
        assert!(packet.contains("【任务协同委派简报】"));
        assert!(packet.contains("src/auth.rs"));
        assert!(packet.contains("src/token.rs"));
        assert!(packet.contains("cargo test test_auth"));
        assert!(packet.contains("严禁修改外部接口"));
        assert!(packet.contains("【施工工人准则】"));

        // 2. 测试工件保存与紧凑摘要截断
        let touched = vec!["src/auth.rs".to_string()];
        let full_text = "A".repeat(800);
        let artifact_path = save_subtask_report_artifact(&ws, "sub_123", "测试任务", "测试角色", &full_text, &touched).unwrap();
        assert!(artifact_path.contains(".harness/subtasks/sub_123_report.md"));
        assert!(ws.join(&artifact_path).exists());

        let summary = extract_compact_summary(&full_text, 100);
        assert!(summary.len() < 250);
        assert!(summary.contains("内容已自动压缩提炼"));

        let _ = std::fs::remove_dir_all(&ws);
    }
}
