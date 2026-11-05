use serde_json::Value;
use std::path::PathBuf;

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToolScope {
    /// 主子会话通用核心工具（文件读写、检索、命令、Todo、计划、记忆等）
    Universal,
    /// 仅主会话可用（子任务派发、协作者编排、浏览器自动化等）
    MainOnly,
    /// 仅子任务会话可用（向父会话正式交付）
    SubOnly,
}

impl ToolScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            ToolScope::Universal => "universal",
            ToolScope::MainOnly => "main_only",
            ToolScope::SubOnly => "sub_only",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ToolScope::Universal => "主/子对话可用",
            ToolScope::MainOnly => "仅主对话",
            ToolScope::SubOnly => "子对话可用",
        }
    }
}

pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: Value,
    pub risk: Risk,
    pub scope: ToolScope,
}

#[derive(Clone)]
pub struct HostCtx {
    pub app: tauri::AppHandle,
    pub session_id: String,
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
    /// 当前正在执行的助手消息 ID（用于影子快照归属）
    pub message_id: Option<String>,
    /// 当前生效的代理地址（若未开启代理则为 None）
    pub proxy_url: Option<String>,
}

pub type PartialCb<'a> = &'a (dyn Fn(&str) + Send + Sync);

/// 常驻核心工具集（直接以原生顶级 Function 下发给大模型，保持前缀缓存 100% 恒定）
pub const CORE_TOOL_NAMES: &[&str] = &[
    "read_file",
    "list_dir",
    "file_outline",
    "glob",
    "grep",
    "write_file",
    "edit_file",
    "run_command",
    "todo",
    "create_plan",
    "update_plan",
    "lookup_tool",
    "call_extended_tool",
];

/// 临时空间会话下发的 temp_* 工具集（普通会话不下发，见 agent.rs run_once 的 specs 过滤）
pub const TEMP_TOOL_NAMES: &[&str] = &[
    "temp_status",
    "temp_changes",
    "temp_diff",
    "temp_snapshot",
    "temp_restore",
    "temp_merge",
];

/// 子 Agent、协作者与子进程相关工具名称列表（已由 ToolScope::MainOnly 替代，保留常用于兼容与断言）
#[allow(dead_code)]
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
    "reject_subprocess",
    "reject_subagent",
    "dispatch_collaborator",
    "wait_collaborators",
    "get_collaborators",
];

/// 抽象 Tool trait，为动态工具、MCP 客户端与浏览器自动化提供统一契约
#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> String;
    fn description(&self) -> String;
    fn schema(&self) -> Value;
    fn risk(&self) -> Risk;
    async fn execute(&self, args: &Value, ctx: &ToolCtx, on_partial: Option<PartialCb<'_>>) -> Result<String, String>;
}


/// 传输层工具调用信封归一化模型（Canonical Tool Call）
///
/// 在工具收敛架构下，大模型可能通过直接调用原生顶级工具（如 read_file），
/// 或通过收敛信封协议（如 call_extended_tool(tool_name: "recognize_image", tool_params: {...})）发起调用。
/// `CanonicalToolCall` 在系统入口处完成确定性解包与参数拍平，
/// 使得整个系统的后续环节（权限治理、审批流、事件存储、执行调度、多模态附件、自纠错状态跟踪、方案看板）
/// 均面向真实规范的工具名称与参数，彻底消除对收敛信封名的补丁式拆包。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CanonicalToolCall {
    /// 原始调用 ID（例如 LLM 下发的 call_123）
    pub call_id: String,
    /// 传输层协议工具名（例如 "call_extended_tool" 或 "read_file"）
    pub protocol_name: String,
    /// 规范化后的真实工具名（例如 "recognize_image" 或 "read_file"）
    pub name: String,
    /// 规范化并确定性拍平后的业务参数
    pub params: Value,
    /// 是否为通过扩展工具信封发起的调用
    pub is_extended: bool,
}

impl CanonicalToolCall {
    pub fn from_raw(call_id: &str, raw_name: &str, raw_args: &str) -> Self {
        let raw_val: Value = serde_json::from_str(raw_args).unwrap_or_else(|_| {
            serde_json::json!({"__parse_error": "工具参数不是有效 JSON"})
        });
        Self::from_value(call_id, raw_name, raw_val)
    }

    pub fn from_value(call_id: &str, raw_name: &str, args: Value) -> Self {
        if raw_name == "call_extended_tool" {
            let target_name = args
                .get("tool_name")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string());

            if let Some(target_name) = target_name {
                if !target_name.is_empty() {
                    let mut sub_args = args
                        .get("tool_params")
                        .or_else(|| args.get("arguments"))
                        .or_else(|| args.get("payload"))
                        .or_else(|| args.get("target_args"))
                        .cloned()
                        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));

                    // 确定性拍平（Defensive Flattening）：
                    // 1. 若 sub_args 内部由于模型幻觉继续嵌套 arguments/tool_params/payload，递归提取并合并至顶层
                    if let Some(map) = sub_args.as_object_mut() {
                        for nested_key in ["tool_params", "arguments", "payload", "target_args"] {
                            if let Some(nested_val) = map.remove(nested_key) {
                                if let Some(nested_map) = nested_val.as_object() {
                                    for (k, v) in nested_map {
                                        map.entry(k).or_insert(v.clone());
                                    }
                                }
                            }
                        }
                    } else if !sub_args.is_object() {
                        sub_args = Value::Object(serde_json::Map::new());
                    }

                    // 2. 若模型将部分业务参数（如 path/prompt/title/task 等）直接放在顶层与 tool_name 平级，
                    // 将其补充合并进 sub_args（以 sub_args 内部已有参数为先）
                    if let Some(args_map) = args.as_object() {
                        if let Some(sub_map) = sub_args.as_object_mut() {
                            for (k, v) in args_map {
                                if k != "tool_name"
                                    && k != "tool_params"
                                    && k != "arguments"
                                    && k != "payload"
                                    && k != "target_args"
                                {
                                    sub_map.entry(k.clone()).or_insert(v.clone());
                                }
                            }
                        }
                    }

                    return Self {
                        call_id: call_id.to_string(),
                        protocol_name: raw_name.to_string(),
                        name: target_name,
                        params: sub_args,
                        is_extended: true,
                    };
                }
            }
        }

        Self {
            call_id: call_id.to_string(),
            protocol_name: raw_name.to_string(),
            name: raw_name.to_string(),
            params: args,
            is_extended: false,
        }
    }
}
