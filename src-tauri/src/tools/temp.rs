use crate::models::truncate_result;
use tauri::Manager;
use super::helpers::*;
use super::types::*;
use serde_json::Value;

pub(crate) const TEMP_NO_CTX: &str = "临时空间上下文不可用（该对话可能不是临时空间对话）";

// ---------- 临时空间工具实现 ----------

/// temp_status：项目清单 + 各项目变更文件数 + 快照列表
pub(crate) async fn temp_status(ctx: &ToolCtx) -> Result<String, String> {
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
pub(crate) async fn temp_changes(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
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
pub(crate) async fn temp_diff(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
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
pub(crate) async fn temp_snapshot(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
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
pub(crate) async fn temp_restore(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
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
pub(crate) async fn temp_merge(ctx: &ToolCtx) -> Result<String, String> {
    let host = ctx.host.as_ref().ok_or("内部错误：缺少宿主上下文")?;
    let state = host.app.state::<crate::AppState>();
    let summary = crate::temp::merge_from_agent(&state, &host.app, &host.session_id).await?;
    Ok(truncate_result(&crate::temp::format_merge_summary(&summary)))
}

