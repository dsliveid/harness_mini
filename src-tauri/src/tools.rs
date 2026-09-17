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

/// 子 Agent 协作工具名称列表
pub const SUBAGENT_TOOL_NAMES: &[&str] = &[
    "spawn_subagent",
    "get_subagent_status",
    "wait_subagents",
    "stop_subagent",
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
            description: "读取工作区内文本文件的内容（带行号）。修改文件前必须先用它确认精确原文。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "相对于工作区的文件路径"},
                    "offset_line": {"type": "integer", "description": "起始行（1 开始，默认 1）"},
                    "max_lines": {"type": "integer", "description": "最多读取行数（默认 2000）"}
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
            description: "在工作区文件内容中按正则搜索，返回 `路径:行号: 内容` 列表。",
            risk: Risk::ReadOnly,
            schema: json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string", "description": "正则表达式"},
                    "path": {"type": "string", "description": "搜索起始目录，默认工作区根目录"},
                    "include": {"type": "string", "description": "文件名过滤 glob，如 *.rs"}
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
        // ---------- 子 Agent 进程协作工具集 ----------
        ToolSpec {
            name: "spawn_subagent",
            description: "创建并启动一个独立的子 Agent 进程并行协作（如前端开发、后端开发、多模块开发等）。子 Agent 拥有独立上下文与工具环境，不污染主会话上下文。严禁子 Agent 递归嵌套调用本工具。",
            risk: Risk::Write,
            schema: json!({
                "type": "object",
                "properties": {
                    "role": {"type": "string", "description": "子 Agent 角色定位，例如：前端开发、后端开发、测试验证、文档编写"},
                    "title": {"type": "string", "description": "子任务简明标题，如 编写用户中心页面组件"},
                    "task": {"type": "string", "description": "分配给该子 Agent 的详细需求描述与任务要求"},
                    "subpath": {"type": "string", "description": "可选：该子 Agent 重点关注的工作区相对子目录（如 src/ 或 backend/）"}
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
            description: "等待一个或多个子 Agent 执行完毕并汇总获取它们的执行结论（包含所作改动与产出）。",
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

fn resolve(ctx: &ToolCtx, rel: &str) -> PathBuf {
    let p = Path::new(rel);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        ctx.workspace.join(p)
    }
}

/// 路径是否位于沙箱内（用于审批判定；临时空间会话的沙箱为整个临时空间根目录）
pub fn inside_workspace(ctx: &ToolCtx, rel: &str) -> bool {
    let p = resolve(ctx, rel);
    let root = ctx.sandbox_root.as_ref().unwrap_or(&ctx.workspace);
    match (p.canonicalize(), root.canonicalize()) {
        (Ok(p), Ok(w)) => p.starts_with(&w),
        _ => false,
    }
}

fn glob_to_regex(pattern: &str) -> String {
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
            c if "\\.^$|+()[]{}".contains(c) => {
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
        "glob" => glob(args, ctx).await,
        "grep" => grep(args, ctx).await,
        "write_file" => write_file(args, ctx).await,
        "edit_file" => edit_file(args, ctx).await,
        "run_command" => run_command(args, ctx, on_partial).await,
        "todo" => Ok("ok".to_string()),
        "list_skills" => list_skills_tool(ctx).await,
        "save_skill" => save_skill_tool(args, ctx).await,
        "run_skill" => run_skill_tool(args, ctx, on_partial).await,
        "spawn_subagent" => spawn_subagent_tool(args, ctx).await,
        "get_subagent_status" => get_subagent_status_tool(args, ctx).await,
        "wait_subagents" => wait_subagents_tool(args, ctx).await,
        "stop_subagent" => stop_subagent_tool(args, ctx).await,
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
    let offset = args.get("offset_line").and_then(|v| v.as_u64()).unwrap_or(1).max(1) as usize;
    let max_lines = args.get("max_lines").and_then(|v| v.as_u64()).unwrap_or(2000) as usize;

    let mut out = String::new();
    let mut count = 0usize;
    for (i, line) in text.lines().enumerate() {
        let lineno = i + 1;
        if lineno < offset {
            continue;
        }
        if count >= max_lines {
            out.push_str(&format!("\n[已达到 max_lines={max_lines}，从第 {offset} 行起未展示完]\n"));
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
    for entry in walkdir::WalkDir::new(&base)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            e.file_name().to_string_lossy() != ".git"
                && e.file_name().to_string_lossy() != "node_modules"
                && e.file_name().to_string_lossy() != "target"
        })
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_file() {
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
    let base = resolve(ctx, rel);
    let re = regex::Regex::new(pattern).map_err(|e| format!("正则无效: {e}"))?;
    let include_re = include
        .map(|g| regex::Regex::new(&glob_to_regex(g)).ok())
        .flatten();

    let mut out = String::new();
    let mut total = 0usize;
    for entry in walkdir::WalkDir::new(&base)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            e.file_name().to_string_lossy() != ".git"
                && e.file_name().to_string_lossy() != "node_modules"
                && e.file_name().to_string_lossy() != "target"
                && e.file_name().to_string_lossy() != "dist"
        })
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_file() {
            continue;
        }
        if entry.metadata().map(|m| m.len() > 1_000_000).unwrap_or(true) {
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
    }
    if total == 0 {
        return Ok("(无匹配)".into());
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
    tokio::fs::write(&path, content)
        .await
        .map_err(|e| format!("写入失败: {e}"))?;
    let old_str = old
        .map(|b| String::from_utf8_lossy(&b).to_string())
        .unwrap_or_default();
    let d = diffutil::diff_lines(&old_str, content, 24 * 1024);
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
    let replace_all = args.get("replace_all").and_then(|v| v.as_bool()).unwrap_or(false);
    let path = resolve(ctx, rel);

    let bytes = tokio::fs::read(&path).await.map_err(|e| format!("读取失败（请先 read_file 确认内容）: {e}"))?;
    let text = String::from_utf8_lossy(&bytes).to_string();

    let count = text.matches(old_string).count();
    if count == 0 {
        return Err("old_string 未在文件中找到（请先 read_file 确认精确内容，注意空白与换行）".into());
    }
    if !replace_all && count > 1 {
        return Err(format!("old_string 出现 {count} 次，需要更长的上下文使其唯一，或设置 replace_all=true"));
    }
    let new_text = if replace_all {
        text.replace(old_string, new_string)
    } else {
        text.replacen(old_string, new_string, 1)
    };
    tokio::fs::write(&path, &new_text)
        .await
        .map_err(|e| format!("写入失败: {e}"))?;
    let d = diffutil::diff_lines(&text, &new_text, 24 * 1024);
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
        let mut c = tokio::process::Command::new("cmd");
        c.arg("/C");
        c.raw_arg(format!("chcp 65001>nul 2>nul & {command}"));
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

// ---------- 子 Agent 协作工具具体实现 ----------

async fn spawn_subagent_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let host = ctx.host.as_ref().ok_or("内部错误：缺少宿主上下文")?;
    let role = args.get("role").and_then(|v| v.as_str()).ok_or("缺少 role 参数")?;
    let title = args.get("title").and_then(|v| v.as_str()).ok_or("缺少 title 参数")?;
    let task = args.get("task").and_then(|v| v.as_str()).ok_or("缺少 task 参数")?;
    let subpath = args.get("subpath").and_then(|v| v.as_str()).map(|s| s.to_string());

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

    let sub_workspace = if let Some(ref rel) = subpath {
        let p = Path::new(rel);
        if p.is_absolute() {
            rel.clone()
        } else {
            std::path::Path::new(&parent_session.workspace_path).join(p).to_string_lossy().to_string()
        }
    } else {
        parent_session.workspace_path.clone()
    };

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
        )?;
        let initial_prompt = format!(
            "【子 Agent 协作任务】\n角色定位：{role}\n任务标题：{title}\n\n详细需求描述：\n{task}{}",
            subpath.as_ref().map(|p| format!("\n重点目录：`{p}`")).unwrap_or_default()
        );
        let user_msg = crate::store::new_message(&db, &sub.id, "user", Some(initial_prompt), false)?;
        (sub, user_msg)
    };

    // 启动子 Agent 异步运行循环
    crate::agent::spawn_session_task(host.app.clone(), sub.id.clone(), Some(user_msg.id));

    // 广播事件通知前端刷新子 Agent 列表
    let _ = host.app.emit("subagent:created", json!({
        "parentId": parent_id,
        "subagent": sub,
    }));
    let _ = host.app.emit("subagents:changed", json!({
        "parentId": parent_id,
    }));

    Ok(format!(
        "已成功创建并启动子 Agent 进程！\n- ID: `{}`\n- 角色: {}\n- 标题: {}\n- 状态: 运行中 (running)\n\n子 Agent 正在独立上下文中执行，用户点击界面右侧可实时查看其完整对话流程与工具卡片。后续可通过 `wait_subagents` 或 `get_subagent_status` 协同跟进。",
        sub.id, role, title
    ))
}

async fn get_subagent_status_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let host = ctx.host.as_ref().ok_or("内部错误：缺少宿主上下文")?;
    let target_id = args.get("subagent_id").and_then(|v| v.as_str());
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
        let msgs = crate::store::get_messages(&db, &s.id, None, 10).unwrap_or_default();
        let last_reply = msgs
            .iter()
            .rev()
            .find(|m| m.role == "assistant" && !m.content.as_deref().unwrap_or("").is_empty())
            .and_then(|m| m.content.as_deref())
            .unwrap_or("(尚在准备或执行工具中)");
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
            if is_running { "🟡 运行中 (running)" } else { "🟢 已完成或空闲 (idle)" },
            s.total_tokens.unwrap_or(0),
            preview
        ));
    }

    Ok(out)
}

async fn wait_subagents_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let host = ctx.host.as_ref().ok_or("内部错误：缺少宿主上下文")?;
    let timeout_secs = args
        .get("timeout_seconds")
        .and_then(|v| v.as_u64())
        .unwrap_or(60)
        .clamp(5, 300);
    let specified_ids: Option<Vec<String>> = args.get("subagent_ids").and_then(|v| {
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
        out.push_str(&format!("⚠️ 等待达到超时上限（{timeout_secs}s），部分子 Agent 可能仍在后台继续运行。\n\n"));
    } else {
        out.push_str("✅ 所有目标子 Agent 执行完毕！汇总结果如下：\n\n");
    }

    let db = state.db.lock().unwrap();
    for id in &target_ids {
        let is_running = crate::agent::is_run_active(&state, id);
        let s = crate::store::get_session(&db, id)?.unwrap_or_else(|| {
            crate::models::Session {
                id: id.clone(),
                title: "未知子Agent".into(),
                workspace_path: "".into(),
                access_mode: None,
                project_id: None,
                status: "active".into(),
                last_message_at: None,
                created_at: "".into(),
                updated_at: "".into(),
                is_temp: false,
                temp_code: None,
                temp_root: None,
                source_workspace: None,
                merged_seq: None,
                merged_pending: false,
                total_tokens: Some(0),
                prompt_tokens: Some(0),
                completion_tokens: Some(0),
                parent_session_id: Some(parent_id.clone()),
                session_type: "subagent".into(),
                subagent_role: None,
                subagent_task: None,
            }
        });
        let msgs = crate::store::get_messages(&db, id, None, 10).unwrap_or_default();
        let last_reply = msgs
            .iter()
            .rev()
            .find(|m| m.role == "assistant" && !m.content.as_deref().unwrap_or("").is_empty())
            .and_then(|m| m.content.as_deref())
            .unwrap_or("(未产生文本回复)");

        out.push_str(&format!(
            "### 子 Agent: {} ({})\n- 状态: {}\n- Token: {}\n- 最终答复/成果：\n```markdown\n{}\n```\n\n",
            s.title,
            s.subagent_role.as_deref().unwrap_or(""),
            if is_running { "🟡 仍在运行" } else { "🟢 已完成" },
            s.total_tokens.unwrap_or(0),
            last_reply
        ));
    }

    Ok(out)
}

async fn stop_subagent_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let host = ctx.host.as_ref().ok_or("内部错误：缺少宿主上下文")?;
    let subagent_id = args.get("subagent_id").and_then(|v| v.as_str()).ok_or("缺少 subagent_id 参数")?;
    crate::agent::stop_session(&host.app, subagent_id);
    let _ = host.app.emit("subagent:update", json!({
        "parentId": host.session_id,
        "subagentId": subagent_id,
        "status": "stopped"
    }));
    Ok(format!("已成功停止子 Agent【{subagent_id}】。"))
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
}
