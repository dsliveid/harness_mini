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

pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: Value,
    pub risk: Risk,
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

/// 抽象 Tool trait，为动态工具、MCP 客户端与浏览器自动化提供统一契约
#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> String;
    fn description(&self) -> String;
    fn schema(&self) -> Value;
    fn risk(&self) -> Risk;
    async fn execute(&self, args: &Value, ctx: &ToolCtx, on_partial: Option<PartialCb<'_>>) -> Result<String, String>;
}
