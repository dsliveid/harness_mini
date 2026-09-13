use crate::diffutil;
use crate::models::truncate_result;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tokio::io::AsyncBufReadExt;

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

pub struct ToolCtx {
    pub workspace: PathBuf,
    /// 路径越界判定用的沙箱根：临时空间会话为临时空间根目录（含主项目与关联项目副本）；
    /// 普通会话为 None（等同 workspace）
    pub sandbox_root: Option<PathBuf>,
    pub command_timeout: std::time::Duration,
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
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

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
    loop {
        tokio::select! {
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

    let status = child.wait().await;
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
