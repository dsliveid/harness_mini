use tokio::io::AsyncBufReadExt;
use super::types::*;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub(crate) fn change_mark(c: char) -> &'static str {
    match c {
        'A' => "A",
        'D' => "D",
        _ => "M",
    }
}

#[allow(dead_code)]
pub fn is_high_danger(cmd: &str) -> bool {
    let patterns = [
        r"(?i)\brm\s+(-[a-z]+)*\s*-[a-z]*r[a-z]*\b",
        r"(?i)\bdel\b.*?(?:[/\\]s\b|[/\\]s:)",
        r"(?i)\b(?:rd|rmdir)\b.*?(?:[/\\]s\b|[/\\]s:)",
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

pub(crate) fn find_match_line_numbers(text: &str, target: &str) -> Vec<usize> {
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

pub(crate) fn diagnose_edit_mismatch(text: &str, old_string: &str) -> String {
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

pub(crate) fn resolve(ctx: &ToolCtx, rel: &str) -> PathBuf {
    let p = Path::new(rel);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        let clean = rel.trim_start_matches(|c| c == '/' || c == '\\');
        ctx.workspace.join(clean)
    }
}

/// 路径是否位于沙箱内（用于审批判定；临时空间会话的沙箱为整个临时空间根目录）
#[allow(dead_code)]
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

pub(crate) fn glob_to_regex(pattern: &str) -> String {
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

pub(crate) fn is_binary(buf: &[u8]) -> bool {
    let n = buf.len().min(8192);
    buf[..n].contains(&0)
}


pub(crate) async fn pipe_lines<T: tokio::io::AsyncRead + Unpin>(
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
