use super::helpers::*;
use super::types::*;
use crate::models::truncate_result;
use serde_json::Value;
use std::path::Path;

pub(crate) async fn glob(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
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

pub(crate) async fn grep(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
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

