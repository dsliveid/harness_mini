use super::helpers::*;
use super::types::*;
use crate::diffutil;
use crate::models::truncate_result;
use serde_json::{json, Value};
use std::path::Path;
use tauri::{Emitter, Manager};

pub(crate) async fn read_file(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let rel = args.get("path").and_then(|v| v.as_str()).ok_or("缺少 path")?;
    let path = resolve(ctx, rel);
    let bytes = tokio::fs::read(&path).await.map_err(|e| format!("读取失败: {e}"))?;
    if is_binary(&bytes) {
        return Err("疑似二进制文件，无法以文本读取。若该文件为图片，请直接调用 recognize_image 工具进行视觉多模态识别与分析。".into());
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

pub(crate) async fn file_outline(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
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

pub(crate) fn extract_outline(text: &str, ext: &str) -> String {
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

pub(crate) fn is_rust_symbol(words: &[&str]) -> bool {
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

pub(crate) fn is_ts_js_symbol(trimmed: &str) -> bool {
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

pub(crate) fn is_c_like_symbol(trimmed: &str) -> bool {
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

pub(crate) async fn list_dir(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
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


pub(crate) async fn write_file(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let rel = args.get("path").and_then(|v| v.as_str()).ok_or("缺少 path")?;
    let content = args.get("content").and_then(|v| v.as_str()).ok_or("缺少 content")?;
    let path = resolve(ctx, rel);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let old = tokio::fs::read(&path).await.ok();
    let existed = old.is_some();
    let old_str = old
        .as_ref()
        .map(|b| String::from_utf8_lossy(b).to_string())
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
    record_file_snapshot(ctx, rel, old.as_deref(), content_to_write.as_bytes()).await;
    let d = diffutil::diff_lines(&old_str, &content_to_write, 24 * 1024);
    let action = if existed { "已覆盖写入" } else { "已创建" };
    Ok(truncate_result(&format!(
        "{action} {rel}（+{} −{}）\n```diff\n{}```",
        d.added, d.removed, d.text
    )))
}

pub async fn record_file_snapshot_raw(
    app: &tauri::AppHandle,
    session_id: &str,
    message_id: &str,
    tool_event_id: &str,
    rel_path: &str,
    old_bytes: Option<&[u8]>,
    new_bytes: &[u8],
) {
    let state = app.state::<crate::AppState>();
    let data_dir = {
        let lock = state.data_dir.lock().unwrap();
        lock.clone().unwrap_or_else(|| state.default_data_dir.clone())
    };

    let before_hash = match old_bytes {
        Some(b) => match crate::snapshot_fs::save_blob_async(&data_dir, b).await {
            Ok(h) => Some(h),
            Err(e) => {
                eprintln!("[Snapshot] 保存旧文件快照失败: {e}");
                None
            }
        },
        None => None,
    };

    let after_hash = match crate::snapshot_fs::save_blob_async(&data_dir, new_bytes).await {
        Ok(h) => h,
        Err(e) => {
            eprintln!("[Snapshot] 保存新文件快照失败: {e}");
            return;
        }
    };

    let snap = crate::models::ToolFileSnapshot {
        id: uuid::Uuid::new_v4().to_string(),
        session_id: session_id.to_string(),
        message_id: message_id.to_string(),
        tool_event_id: tool_event_id.to_string(),
        file_path: rel_path.replace('\\', "/"),
        before_hash,
        after_hash,
        is_new_file: old_bytes.is_none(),
        reverted_at: None,
        created_at: crate::store::now(),
    };

    let db = state.db.lock().unwrap();
    if let Err(e) = crate::store::insert_tool_file_snapshot(&db, &snap) {
        eprintln!("[Snapshot] 插入快照元数据失败: {e}");
    }
}

pub(crate) async fn record_file_snapshot(
    ctx: &ToolCtx,
    rel_path: &str,
    old_bytes: Option<&[u8]>,
    new_bytes: &[u8],
) {
    let (Some(host), Some(event_id), Some(message_id)) = (&ctx.host, &ctx.event_id, &ctx.message_id) else {
        return;
    };
    record_file_snapshot_raw(
        &host.app,
        &host.session_id,
        message_id,
        event_id,
        rel_path,
        old_bytes,
        new_bytes,
    ).await;
}

pub(crate) async fn edit_file(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
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
    record_file_snapshot(ctx, rel, Some(&bytes), final_content.as_bytes()).await;
    let d = diffutil::diff_lines(&raw_text, &final_content, 24 * 1024);
    Ok(truncate_result(&format!(
        "已修改 {rel}（+{} −{}）\n```diff\n{}```",
        d.added, d.removed, d.text
    )))
}
