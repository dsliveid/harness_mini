//! 临时空间：会话级隔离工作区。
//!
//! 流程：alloc（生成随机码目录地址，不落盘）→ ensure（每次 Agent 运行前检查并按需
//! 拷贝主项目与关联项目 + git 基线提交）→ list_changes（工作区 vs 基线的变更清单）→
//! merge（写回各项目原目录，冲突走 AI 智能合并）/ clear（删除整个随机码目录）。
//!
//! 目录约定：`<数据目录>\temp-project\<随机码>\`，随机码与会话 1:1。

use crate::llm::{self, LlmCfg};
use crate::models::*;
use crate::store;
use serde_json::json;
use std::path::{Path, PathBuf};
use tauri::Emitter;

/// 拷贝排除目录（任意层级）；.gitignore 规则由 ignore crate 在遍历时另行生效
const COPY_EXCLUDE_DIRS: &[&str] = &[".git", "node_modules", "target", "dist"];
const MANIFEST_KEY: &str = "temp_manifest";
/// AI 合并的输入大小上限（基线 + 原文 + 临时文合计），超过则标记人工处理
const AI_MERGE_MAX_BYTES: usize = 512 * 1024;
const AI_MERGE_FAILED: &str = "<<MERGE_FAILED>>";

pub struct ChangedFile {
    pub path: String,
    /// 'A' 新增 | 'M' 修改 | 'D' 删除（相对基线提交）
    pub change: char,
}

// ---------- git ----------

pub fn git_available() -> bool {
    std::process::Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn run_git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("执行 git 失败: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        Err(format!(
            "git {} 失败: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// 在副本内建立基线：init（原项目 .git 不拷贝）→ 全量 add（含原项目未提交文件）→ 提交 → 返回 sha。
/// 显式注入 user.name/email，避免宿主机缺 git 全局配置导致提交失败。
pub fn git_baseline(dir: &Path) -> Result<String, String> {
    if !dir.join(".git").exists() {
        run_git(dir, &["init"])?;
    }
    run_git(dir, &["-c", "core.autocrlf=false", "add", "-A"])?;
    run_git(
        dir,
        &[
            "-c",
            "user.name=harness_mini",
            "-c",
            "user.email=harness-mini@local",
            "commit",
            "--allow-empty",
            "-m",
            "临时空间基线（harness_mini 自动提交）",
        ],
    )?;
    Ok(run_git(dir, &["rev-parse", "HEAD"])?.trim().to_string())
}

/// 基线提交中某文件的原始内容（基线中不存在时返回 None）
fn read_baseline_blob(dir: &Path, baseline: &str, rel: &str) -> Option<Vec<u8>> {
    let out = std::process::Command::new("git")
        .args(["cat-file", "-p", &format!("{baseline}:{rel}")])
        .current_dir(dir)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(out.stdout)
}

// ---------- 路径与拷贝 ----------

fn path_key(p: &Path) -> String {
    let t = p.to_string_lossy().replace('\\', "/");
    let t = t.trim_end_matches('/');
    if cfg!(windows) {
        t.to_lowercase()
    } else {
        t.to_string()
    }
}

/// child 是否位于 parent 目录之下（且不等于 parent）
pub fn is_under(child: &Path, parent: &Path) -> bool {
    let (ck, pk) = (path_key(child), path_key(parent));
    ck != pk && ck.starts_with(&format!("{pk}/"))
}

/// 校验临时空间根目录必须位于 `<数据目录>\temp-project\` 之下（防清单损坏误删/误写）
pub fn validate_root(root: &Path, data_dir: &Path) -> Result<(), String> {
    let base = data_dir.join("temp-project");
    if !is_under(root, &base) {
        return Err(format!("临时空间目录非法：{}", root.display()));
    }
    Ok(())
}

/// 递归拷贝项目目录到目标（目标须不存在）。
/// 排除 COPY_EXCLUDE_DIRS 与 .gitignore 规则命中的文件；require_git(false) 使
/// 非仓库目录也应用 .gitignore；符号链接不拷贝，避免越界引用。返回拷贝的文件数。
pub fn copy_project(src: &Path, dst: &Path) -> Result<usize, String> {
    use ignore::WalkBuilder;
    std::fs::create_dir_all(dst).map_err(|e| format!("创建目录失败: {e}"))?;
    let mut count = 0usize;
    let walker = WalkBuilder::new(src)
        .hidden(false) // 包含点开头文件（.gitignore 自身、.env 等）
        .require_git(false)
        .git_global(false)
        .parents(false) // 只应用拷贝树内的 ignore 规则，不受上级目录影响
        .filter_entry(|e| {
            if e.depth() == 0 {
                return true; // 根目录本身即使叫 target/dist 也不能排除
            }
            let is_excluded_dir = e
                .file_type()
                .map(|t| t.is_dir())
                .unwrap_or(false)
                && COPY_EXCLUDE_DIRS.iter().any(|x| e.file_name().to_string_lossy() == *x);
            !is_excluded_dir
        })
        .build();
    for entry in walker {
        let entry = entry.map_err(|e| format!("遍历失败: {e}"))?;
        if entry.depth() == 0 {
            continue;
        }
        let Some(ft) = entry.file_type() else { continue };
        let rel = entry.path().strip_prefix(src).unwrap_or(entry.path());
        let target = dst.join(rel);
        if ft.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| format!("创建目录失败: {e}"))?;
        } else if ft.is_symlink() {
            continue;
        } else if ft.is_file() {
            if let Some(parent) = target.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::copy(entry.path(), &target)
                .map_err(|e| format!("拷贝失败 {}: {e}", entry.path().display()))?;
            count += 1;
        }
    }
    Ok(count)
}

// ---------- alloc（生成计划，不落盘） ----------

/// 为项目生成临时空间计划：唯一随机码 + 主项目/关联项目的临时路径。
/// 仅计算地址不创建目录；git 不可用直接拒绝（临时空间强依赖 git 基线）。
pub fn alloc(data_dir: &Path, project: &Project, links: &[ProjectLink]) -> Result<TempAlloc, String> {
    let main_source = project
        .path
        .clone()
        .filter(|s| !s.trim().is_empty())
        .ok_or("项目未绑定目录，无法创建临时空间")?;
    if !git_available() {
        return Err("未检测到 Git（git 命令不可用），临时空间功能需要 Git 支持，请安装 Git 后重试".into());
    }
    let base = data_dir.join("temp-project");
    let code = loop {
        let u = uuid::Uuid::new_v4().simple().to_string();
        let c = u[..12].to_string();
        if !base.join(&c).exists() {
            break c;
        }
    };
    let root = base.join(&code);

    let mut used: Vec<String> = vec![];
    let mut entry = |key: String, source: &str, description: &str| -> TempProjectEntry {
        let base_name = store::dir_name_of(source);
        let mut dir = base_name.clone();
        let mut i = 2;
        while used.iter().any(|u| u.eq_ignore_ascii_case(&dir)) {
            dir = format!("{base_name}-{i}");
            i += 1;
        }
        used.push(dir.clone());
        TempProjectEntry {
            key,
            name: base_name.clone(),
            source: source.to_string(),
            temp: root.join(&dir).to_string_lossy().to_string(),
            description: description.to_string(),
            baseline: None,
        }
    };

    let main = entry("main".into(), &main_source, "主项目");
    let main_temp = main.temp.clone();
    let mut projects = vec![main];
    for l in links {
        projects.push(entry(format!("link:{}", l.id), &l.path, &l.description));
    }
    Ok(TempAlloc {
        code,
        root: root.to_string_lossy().to_string(),
        main_temp,
        source_workspace: main_source,
        projects,
    })
}

// ---------- manifest ----------

pub fn load_manifest(conn: &rusqlite::Connection, session_id: &str) -> Result<Option<TempManifest>, String> {
    Ok(store::get_kv(conn, session_id, MANIFEST_KEY)?
        .and_then(|s| serde_json::from_str(&s).ok()))
}

pub fn save_manifest(conn: &rusqlite::Connection, session_id: &str, m: &TempManifest) -> Result<(), String> {
    let s = serde_json::to_string(m).map_err(|e| e.to_string())?;
    store::set_kv(conn, session_id, MANIFEST_KEY, &s)
}

// ---------- ensure（每次运行前的检查与重建） ----------

/// 会话为临时会话时：检查根目录与各项目副本是否存在，缺失则重建（拷贝 + 基线提交）。
/// 幂等：已存在的项目副本跳过，不重复拷贝。
pub fn ensure_space(state: &crate::AppState, app: &tauri::AppHandle, session: &Session) -> Result<(), String> {
    if !session.is_temp {
        return Ok(());
    }
    let root = PathBuf::from(
        session
            .temp_root
            .as_deref()
            .ok_or("临时会话缺少 temp_root")?,
    );
    {
        let data_dir = state
            .data_dir
            .lock()
            .unwrap()
            .clone()
            .ok_or("数据目录不可用")?;
        validate_root(&root, &data_dir)?;
    }
    if !git_available() {
        return Err("未检测到 Git（git 命令不可用），无法准备临时空间".into());
    }
    let mut manifest = {
        let db = state.db.lock().unwrap();
        load_manifest(&db, &session.id)?.ok_or("临时空间清单缺失")?
    };
    if !root.exists() {
        std::fs::create_dir_all(&root).map_err(|e| format!("创建临时空间目录失败: {e}"))?;
    }
    let mut rebuilt = false;
    for p in manifest.projects.iter_mut() {
        if Path::new(&p.temp).exists() {
            continue;
        }
        let src = PathBuf::from(&p.source);
        if !src.exists() {
            return Err(format!("源目录不存在，无法拷贝到临时空间: {}", p.source));
        }
        copy_project(&src, Path::new(&p.temp))?;
        p.baseline = Some(git_baseline(Path::new(&p.temp))?);
        rebuilt = true;
    }
    if rebuilt {
        let db = state.db.lock().unwrap();
        save_manifest(&db, &session.id, &manifest)?;
    }
    emit_temp_update(state, app, &session.id);
    Ok(())
}

// ---------- 变更检测 ----------

/// 工作区相对基线提交的变更清单（把未跟踪文件全部暂存后 diff，与工作区内容一致）
pub fn list_changes(entry: &TempProjectEntry) -> Result<Vec<ChangedFile>, String> {
    let dir = Path::new(&entry.temp);
    if !dir.exists() {
        return Ok(vec![]);
    }
    let Some(baseline) = entry.baseline.as_deref() else {
        return Ok(vec![]);
    };
    run_git(dir, &["-c", "core.autocrlf=false", "add", "-A"])?;
    let out = run_git(
        dir,
        &[
            "-c",
            "diff.renames=false",
            "-c",
            "core.quotepath=false",
            "diff",
            "--cached",
            "--name-status",
            baseline,
        ],
    )?;
    let mut files = Vec::new();
    for line in out.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, '\t');
        let (Some(st), Some(p)) = (parts.next(), parts.next()) else {
            continue;
        };
        let ch = st.chars().next().unwrap_or('M');
        if matches!(ch, 'A' | 'M' | 'D') {
            files.push(ChangedFile {
                path: p.trim().to_string(),
                change: ch,
            });
        }
    }
    Ok(files)
}

fn is_binary(buf: &[u8]) -> bool {
    let n = buf.len().min(8192);
    buf[..n].contains(&0)
}

// ---------- 变更列表 / 文件 diff ----------

/// 单文件 diff 内容上限，超过则标记 too_large 不生成 hunks
const DIFF_MAX_FILE_BYTES: usize = 1024 * 1024;
/// 单文件 diff 总行数上限，超过则截断
const DIFF_MAX_LINES: usize = 4000;

fn change_label(c: char) -> &'static str {
    match c {
        'A' => "added",
        'D' => "deleted",
        _ => "modified",
    }
}

fn file_name_of(path: &str) -> String {
    let t = path.trim_end_matches('/');
    t.rsplit('/').next().unwrap_or(t).to_string()
}

/// 读取变更文件的两侧内容：old = 基线 blob（新增文件为 None），new = 工作区文件（删除为 None）
fn read_change_pair(entry: &TempProjectEntry, change: char, path: &str) -> (Option<Vec<u8>>, Option<Vec<u8>>, bool) {
    let temp_dir = Path::new(&entry.temp);
    let old = if change == 'A' {
        None
    } else {
        read_baseline_blob(temp_dir, entry.baseline.as_deref().unwrap_or(""), path)
    };
    let new = if change == 'D' {
        None
    } else {
        std::fs::read(temp_dir.join(path)).ok()
    };
    let too_large = old.as_ref().map(|b| b.len() > DIFF_MAX_FILE_BYTES).unwrap_or(false)
        || new.as_ref().map(|b| b.len() > DIFF_MAX_FILE_BYTES).unwrap_or(false);
    (old, new, too_large)
}

fn pair_is_binary(old: &Option<Vec<u8>>, new: &Option<Vec<u8>>) -> bool {
    [old, new].iter().any(|b| b.as_deref().map(is_binary).unwrap_or(false))
}

/// 单文件相对基线的增删行数（供 Agent 的 temp_changes 工具展示）
pub fn change_stat(entry: &TempProjectEntry, path: &str, change: char) -> (usize, usize) {
    let (old, new, too_large) = read_change_pair(entry, change, path);
    if too_large || pair_is_binary(&old, &new) {
        return (0, 0);
    }
    let old_str = String::from_utf8_lossy(old.as_deref().unwrap_or(b"")).to_string();
    let new_str = String::from_utf8_lossy(new.as_deref().unwrap_or(b"")).to_string();
    let d = similar::TextDiff::from_lines(old_str.as_str(), new_str.as_str());
    let (mut a, mut r) = (0usize, 0usize);
    for ch in d.iter_all_changes() {
        match ch.tag() {
            similar::ChangeTag::Insert => a += 1,
            similar::ChangeTag::Delete => r += 1,
            similar::ChangeTag::Equal => {}
        }
    }
    (a, r)
}

/// 变更列表（弹窗左侧）：按项目分组，带增删行数统计
pub fn list_temp_changes(manifest: &TempManifest) -> Result<TempChanges, String> {
    let mut projects = Vec::new();
    let mut total = 0usize;
    for entry in &manifest.projects {
        let mut files = Vec::new();
        for f in list_changes(entry)? {
            let (old, new, too_large) = read_change_pair(entry, f.change, &f.path);
            let binary = !too_large && pair_is_binary(&old, &new);
            let (added, removed) = if too_large || binary {
                (0, 0)
            } else {
                let old_str = String::from_utf8_lossy(old.as_deref().unwrap_or(b"")).to_string();
                let new_str = String::from_utf8_lossy(new.as_deref().unwrap_or(b"")).to_string();
                let d = similar::TextDiff::from_lines(old_str.as_str(), new_str.as_str());
                let (mut a, mut r) = (0usize, 0usize);
                for ch in d.iter_all_changes() {
                    match ch.tag() {
                        similar::ChangeTag::Insert => a += 1,
                        similar::ChangeTag::Delete => r += 1,
                        similar::ChangeTag::Equal => {}
                    }
                }
                (a, r)
            };
            files.push(TempChangeFile {
                path: f.path.clone(),
                name: file_name_of(&f.path),
                change: change_label(f.change).to_string(),
                added,
                removed,
                binary,
                too_large,
            });
        }
        if !files.is_empty() {
            total += files.len();
            projects.push(TempChangeProject {
                key: entry.key.clone(),
                name: entry.name.clone(),
                source: entry.source.clone(),
                temp: entry.temp.clone(),
                files,
            });
        }
    }
    Ok(TempChanges {
        total_files: total,
        projects,
    })
}

/// 单文件 diff：±3 行上下文分块，供统一视图与并排对比共用
pub fn file_diff(entry: &TempProjectEntry, path: &str) -> Result<TempFileDiff, String> {
    // 仅允许查看当前变更清单内的文件（同时起到路径白名单校验作用）
    let changes = list_changes(entry)?;
    let Some(change) = changes.iter().find(|f| f.path == path).map(|f| f.change) else {
        return Err(format!("该文件不在变更清单中: {path}"));
    };
    let (old, new, too_large) = read_change_pair(entry, change, path);
    let binary = !too_large && pair_is_binary(&old, &new);

    let mut hunks = Vec::new();
    let mut truncated = false;
    let (added, removed);
    if too_large || binary {
        added = 0;
        removed = 0;
    } else {
        let old_str = String::from_utf8_lossy(old.as_deref().unwrap_or(b"")).to_string();
        let new_str = String::from_utf8_lossy(new.as_deref().unwrap_or(b"")).to_string();
        let diff = similar::TextDiff::from_lines(&old_str, &new_str);
        let (mut a, mut r) = (0usize, 0usize);
        let mut total_lines = 0usize;
        'outer: for ops in diff.grouped_ops(3) {
            let mut lines: Vec<DiffLine> = Vec::new();
            for op in &ops {
                for ch in diff.iter_changes(op) {
                    let tag = match ch.tag() {
                        similar::ChangeTag::Delete => "del",
                        similar::ChangeTag::Insert => "add",
                        similar::ChangeTag::Equal => "same",
                    };
                    match ch.tag() {
                        similar::ChangeTag::Insert => a += 1,
                        similar::ChangeTag::Delete => r += 1,
                        similar::ChangeTag::Equal => {}
                    }
                    lines.push(DiffLine {
                        tag: tag.to_string(),
                        old_no: ch.old_index().map(|i| i + 1),
                        new_no: ch.new_index().map(|i| i + 1),
                        text: ch
                            .value()
                            .trim_end_matches('\n')
                            .trim_end_matches('\r')
                            .to_string(),
                    });
                }
            }
            if lines.is_empty() {
                continue;
            }
            if total_lines + lines.len() > DIFF_MAX_LINES {
                truncated = true;
                break 'outer;
            }
            let old_start = lines.iter().find_map(|l| l.old_no).unwrap_or(0);
            let new_start = lines.iter().find_map(|l| l.new_no).unwrap_or(0);
            let old_lines = lines.iter().filter(|l| l.old_no.is_some()).count();
            let new_lines = lines.iter().filter(|l| l.new_no.is_some()).count();
            total_lines += lines.len();
            hunks.push(DiffHunk {
                old_start,
                old_lines,
                new_start,
                new_lines,
                lines,
            });
        }
        added = a;
        removed = r;
    }
    Ok(TempFileDiff {
        project_key: entry.key.clone(),
        project_name: entry.name.clone(),
        path: path.to_string(),
        name: file_name_of(path),
        change: change_label(change).to_string(),
        binary,
        too_large,
        added,
        removed,
        truncated,
        hunks,
    })
}

// ---------- 合并 ----------

/// 把临时空间的变更写回各项目原目录。
/// 冲突判定：原文件当前内容 ≠ 基线内容（原目录在拷贝后又被改动过）→ AI 智能合并；
/// AI 不可用/失败/二进制 → 记入 skipped，不动原文件。
async fn apply_merge(manifest: &TempManifest, llm_cfg: Option<&LlmCfg>) -> MergeSummary {
    let mut summary = MergeSummary::default();
    for entry in &manifest.projects {
        let mut ps = MergeProjectSummary {
            name: entry.name.clone(),
            source: entry.source.clone(),
            ..Default::default()
        };
        let changes = match list_changes(entry) {
            Ok(c) => c,
            Err(e) => {
                ps.skipped.push(format!("(全部) {e}"));
                summary.projects.push(ps);
                continue;
            }
        };
        if changes.is_empty() {
            summary.projects.push(ps);
            continue;
        }
        let src_dir = PathBuf::from(&entry.source);
        if !src_dir.exists() {
            ps.skipped
                .push(format!("(全部) 原目录不存在: {}", entry.source));
            summary.projects.push(ps);
            continue;
        }
        let baseline = entry.baseline.clone().unwrap_or_default();
        let temp_dir = PathBuf::from(&entry.temp);
        for f in &changes {
            let rel = &f.path;
            let temp_file = temp_dir.join(rel);
            let orig_file = src_dir.join(rel);
            let base = read_baseline_blob(&temp_dir, &baseline, rel);
            match f.change {
                'D' => {
                    if !orig_file.exists() {
                        ps.applied += 1; // 原目录本就没有，视为一致
                        continue;
                    }
                    match base {
                        Some(b) if b == std::fs::read(&orig_file).unwrap_or_default() => {
                            match std::fs::remove_file(&orig_file) {
                                Ok(_) => ps.applied += 1,
                                Err(e) => ps.skipped.push(format!("{rel}: 删除失败 {e}")),
                            }
                        }
                        Some(_) => ps.skipped.push(format!(
                            "{rel}: 原文件在基线后被修改过，而临时空间删除了它，需人工处理"
                        )),
                        None => ps.skipped.push(format!("{rel}: 基线中无此文件，状态异常，需人工处理")),
                    }
                }
                'A' | 'M' => {
                    let Ok(temp_bytes) = std::fs::read(&temp_file) else {
                        ps.skipped.push(format!("{rel}: 临时空间文件读取失败"));
                        continue;
                    };
                    if is_binary(&temp_bytes) {
                        ps.skipped.push(format!("{rel}: 二进制文件需人工处理"));
                        continue;
                    }
                    let orig_bytes = if orig_file.exists() {
                        std::fs::read(&orig_file).ok()
                    } else {
                        None
                    };
                    let conflict = match (&base, &orig_bytes) {
                        (None, None) => false,           // 新增文件，原目录也没有
                        (None, Some(_)) => true,         // 双方各自新增了同名文件
                        (Some(_), None) => true,         // 原文件被删除而临时空间在改它
                        (Some(b), Some(o)) => b != o,    // 原目录在基线后变过
                    };
                    if !conflict {
                        if let Some(parent) = orig_file.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        match std::fs::write(&orig_file, &temp_bytes) {
                            Ok(_) => ps.applied += 1,
                            Err(e) => ps.skipped.push(format!("{rel}: 写回失败 {e}")),
                        }
                    } else {
                        match ai_merge(
                            llm_cfg,
                            rel,
                            base.as_deref(),
                            orig_bytes.as_deref(),
                            &temp_bytes,
                        )
                        .await
                        {
                            Some(merged) => match std::fs::write(&orig_file, merged) {
                                Ok(_) => ps.ai_merged += 1,
                                Err(e) => ps.skipped.push(format!("{rel}: 写回失败 {e}")),
                            },
                            None => ps.skipped.push(format!("{rel}: 自动合并失败，需人工处理")),
                        }
                    }
                }
                _ => {}
            }
        }
        summary.projects.push(ps);
    }
    for ps in &summary.projects {
        summary.total_applied += ps.applied;
        summary.total_ai_merged += ps.ai_merged;
        summary.total_skipped += ps.skipped.len();
    }
    summary
}

/// AI 智能合并：以原目录版本为主体，叠加临时版本的修改意图；失败返回 None（原文件不动）
async fn ai_merge(
    cfg: Option<&LlmCfg>,
    rel: &str,
    base: Option<&[u8]>,
    orig: Option<&[u8]>,
    temp: &[u8],
) -> Option<Vec<u8>> {
    let cfg = cfg?;
    let total = temp.len() + orig.map(|b| b.len()).unwrap_or(0) + base.map(|b| b.len()).unwrap_or(0);
    if total > AI_MERGE_MAX_BYTES {
        return None;
    }
    let text = |b: Option<&[u8]>| -> String {
        match b {
            Some(b) => String::from_utf8_lossy(b).to_string(),
            None => "（基线中不存在，属新增文件）".into(),
        }
    };
    let orig_text = match orig {
        Some(_) => text(orig),
        None => "（原目录中已不存在）".into(),
    };
    let prompt = format!(
        r#"你是代码合并助手。同一个文件在临时空间和原目录各自发生了变化，请把「临时版本」的修改合并到「原目录版本」上（以原目录版本为主体，叠加临时版本的修改意图），输出合并后的完整文件内容。

文件路径：{rel}

基线版本（临时空间创建时的快照）：
{}

原目录版本（用户最新修改，以此为主体）：
{orig_text}

临时版本（临时空间中的修改）：
{}

要求：
1. 只输出合并后的完整文件内容，不要任何解释，不要 Markdown 代码块围栏。
2. 保留双方的修改意图，不丢失任何一方的有效变更。
3. 确实无法安全合并时，只输出 {AI_MERGE_FAILED}。"#,
        text(base),
        String::from_utf8_lossy(temp),
        rel = rel,
        orig_text = orig_text,
    );
    let messages = [json!({"role": "user", "content": prompt})];
    let result = llm::chat_stream(cfg, &messages, &[], |_| {}, |_| {}).await.ok()?;
    let content = result.content.trim();
    if content.is_empty() || content.contains(AI_MERGE_FAILED) {
        return None;
    }
    Some(strip_fence(content).into_bytes())
}

/// 剥掉模型可能带上的 Markdown 代码块围栏
fn strip_fence(s: &str) -> String {
    let t = s.trim();
    if let Some(rest) = t.strip_prefix("```") {
        let rest = rest.split_once('\n').map(|(_, r)| r).unwrap_or(rest);
        let rest = rest.trim_end();
        return rest.strip_suffix("```").unwrap_or(rest).trim_end().to_string();
    }
    t.to_string()
}

// ---------- 清空 ----------

/// 删除整个临时空间目录（含全部项目副本）。幂等：目录已不存在同样成功并解除禁止发送。
pub fn clear_space(state: &crate::AppState, app: &tauri::AppHandle, session_id: &str) -> Result<(), String> {
    let session = {
        let db = state.db.lock().unwrap();
        store::get_session(&db, session_id)?.ok_or("会话不存在")?
    };
    if !session.is_temp {
        return Err("该对话不是临时空间对话".into());
    }
    if crate::agent::is_run_active(state, session_id) {
        return Err("Agent 正在运行，请先停止后再清空临时空间".into());
    }
    let root = PathBuf::from(session.temp_root.clone().ok_or("临时空间缺失")?);
    {
        let data_dir = state.data_dir.lock().unwrap().clone().ok_or("数据目录不可用")?;
        validate_root(&root, &data_dir)?;
    }
    if root.exists() {
        std::fs::remove_dir_all(&root).map_err(|e| format!("删除临时空间失败: {e}"))?;
    }
    {
        let db = state.db.lock().unwrap();
        store::set_session_merge_state(&db, session_id, session.merged_seq, false)?;
    }
    emit_session_update(state, app, session_id);
    emit_temp_update(state, app, session_id);
    Ok(())
}

// ---------- 状态与事件 ----------

pub fn temp_info(db: &rusqlite::Connection, session: &Session) -> TempInfo {
    let manifest = load_manifest(db, &session.id).ok().flatten();
    let exists = session
        .temp_root
        .as_deref()
        .map(|r| Path::new(r).exists())
        .unwrap_or(false);
    let mut count = 0usize;
    if exists {
        if let Some(m) = &manifest {
            for p in &m.projects {
                if let Ok(files) = list_changes(p) {
                    count += files.len();
                }
            }
        }
    }
    TempInfo {
        is_temp: session.is_temp,
        exists,
        has_changes: count > 0,
        changed_count: count,
        merged: session.merged_seq.is_some(),
        merged_pending: session.merged_pending,
        merged_seq: session.merged_seq,
        temp_root: session.temp_root.clone(),
        source_workspace: session.source_workspace.clone(),
    }
}

pub fn emit_temp_update(state: &crate::AppState, app: &tauri::AppHandle, session_id: &str) {
    let info = {
        let db = state.db.lock().unwrap();
        store::get_session(&db, session_id)
            .ok()
            .flatten()
            .map(|s| temp_info(&db, &s))
    };
    if let Some(info) = info {
        let _ = app.emit("temp:update", json!({"sessionId": session_id, "info": info}));
    }
}

fn emit_session_update(state: &crate::AppState, app: &tauri::AppHandle, session_id: &str) {
    let s = {
        let db = state.db.lock().unwrap();
        store::get_session(&db, session_id).ok().flatten()
    };
    if let Some(s) = s {
        let _ = app.emit("session:update", &s);
    }
}

/// 合并入口（命令层）：守卫 + 执行 + 落库合并状态
pub async fn merge_space(
    state: &crate::AppState,
    app: &tauri::AppHandle,
    session_id: &str,
) -> Result<MergeSummary, String> {
    let (session, settings) = {
        let db = state.db.lock().unwrap();
        let master = state.master_key.lock().unwrap();
        (
            store::get_session(&db, session_id)?.ok_or("会话不存在")?,
            store::get_settings_with_secrets(&db, &master).unwrap_or_default(),
        )
    };
    if !session.is_temp {
        return Err("该对话不是临时空间对话".into());
    }
    if crate::agent::is_run_active(state, session_id) {
        return Err("Agent 正在运行，请先停止后再合并".into());
    }
    if session.merged_pending {
        return Err("本次变更已合并，请先清空临时空间后再发起新的修改".into());
    }
    let root = PathBuf::from(session.temp_root.clone().ok_or("临时空间缺失")?);
    {
        let data_dir = state.data_dir.lock().unwrap().clone().ok_or("数据目录不可用")?;
        validate_root(&root, &data_dir)?;
    }
    if !root.exists() {
        return Err("临时空间目录不存在，无法合并".into());
    }
    let manifest = {
        let db = state.db.lock().unwrap();
        load_manifest(&db, session_id)?.ok_or("临时空间清单缺失")?
    };
    // 无可用模型时仍可干净应用变更，冲突文件标记需人工处理
    let llm_cfg = resolve_active_model(&settings).map(|(pc, m)| LlmCfg {
        base_url: pc.base_url.clone(),
        api_key: pc.api_key.clone(),
        model: m.to_string(),
    });

    let summary = apply_merge(&manifest, llm_cfg.as_ref()).await;

    {
        let db = state.db.lock().unwrap();
        let max_seq: i64 = db
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) FROM messages WHERE session_id = ?1",
                [session_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        store::set_session_merge_state(&db, session_id, Some(max_seq), true)?;
    }
    emit_session_update(state, app, session_id);
    emit_temp_update(state, app, session_id);
    Ok(summary)
}

// ---------- Agent 工具层（temp_status / temp_changes / temp_diff / temp_snapshot / temp_restore / temp_merge） ----------

/// Agent temp_* 工具的运行上下文：由 run_once 在 ensure_space 之后组装，
/// 非临时空间会话为 None（temp_* 工具不可用，specs 也不下发）
pub struct TempAgentCtx {
    pub manifest: TempManifest,
}

/// 从清单解析项目条目：key 精确匹配，其次按项目名称匹配（大小写不敏感）。
/// 不让 Agent 传绝对路径，从源头避免越界。
pub fn resolve_entry<'a>(
    manifest: &'a TempManifest,
    key_or_name: Option<&str>,
) -> Result<&'a TempProjectEntry, String> {
    let target = key_or_name.map(str::trim).filter(|s| !s.is_empty());
    let Some(t) = target else {
        return manifest
            .projects
            .iter()
            .find(|p| p.key == "main")
            .ok_or_else(|| "清单中缺少主项目（main）".to_string());
    };
    if let Some(p) = manifest.projects.iter().find(|p| p.key == t) {
        return Ok(p);
    }
    let hits: Vec<&TempProjectEntry> = manifest
        .projects
        .iter()
        .filter(|p| p.name.eq_ignore_ascii_case(t))
        .collect();
    match hits.len() {
        1 => Ok(hits[0]),
        0 => {
            let names: Vec<String> = manifest
                .projects
                .iter()
                .map(|p| format!("{}（key: {}）", p.name, p.key))
                .collect();
            Err(format!("未找到项目「{t}」。可用项目：{}", names.join("、")))
        }
        _ => Err(format!(
            "项目名称「{t}」对应多个项目，请改用项目 key 指定"
        )),
    }
}

/// 快照/项目子目录名清洗：仅保留字母数字与 - _，其余替换为下划线
fn sanitize_component(s: &str) -> String {
    let mut out = String::new();
    for c in s.trim().chars() {
        if c.is_alphanumeric() || matches!(c, '-' | '_') {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    let t = out.trim_matches('_').to_string();
    if t.is_empty() { "_".into() } else { t.chars().take(60).collect() }
}

#[derive(Debug, Default)]
pub struct SnapshotOutcome {
    pub label: String,
    pub dir: String,
    pub copied: usize,
    pub failed: Vec<String>,
}

/// 快照当前临时空间：把每个项目副本完整拷贝到 `<临时根>\<label>.snapshot-<时间戳>\`
/// （按项目 key 分子目录，排除 .git 与被忽略文件）。目标目录已存在时报错，绝不覆盖历史快照。
pub fn snapshot_workspace(manifest: &TempManifest, label: &str) -> Result<SnapshotOutcome, String> {
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let label = sanitize_component(label);
    let dir = PathBuf::from(&manifest.root).join(format!("{label}.snapshot-{ts}"));
    if dir.exists() {
        return Err(format!("快照目录已存在，请更换 label 后重试: {}", dir.display()));
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建快照目录失败: {e}"))?;
    let mut out = SnapshotOutcome {
        label,
        dir: dir.to_string_lossy().to_string(),
        ..Default::default()
    };
    for entry in &manifest.projects {
        let src = PathBuf::from(&entry.temp);
        let dst = dir.join(sanitize_component(&entry.key));
        match copy_project(&src, &dst) {
            Ok(n) => out.copied += n,
            Err(e) => out.failed.push(format!("「{}」: {e}", entry.name)),
        }
    }
    Ok(out)
}

#[derive(Debug, Clone)]
pub enum RestoreTarget {
    /// 恢复到基线提交：丢弃全部未提交修改
    Baseline,
    /// 恢复到快照（temp_status 输出的快照目录名，允许只写时间戳前缀）
    Snapshot(String),
}

#[derive(Debug, Default)]
pub struct RestoreOutcome {
    pub restored: Vec<String>,
    pub failed: Vec<String>,
}

/// 把临时空间恢复到基线或指定快照。只动临时副本，不触碰任何原始目录。
/// 注意：不会撤销已合并到原始目录的变更（合并发生在源目录，恢复只重置副本）。
pub fn restore_workspace(manifest: &TempManifest, target: &RestoreTarget) -> Result<RestoreOutcome, String> {
    let mut out = RestoreOutcome::default();
    match target {
        RestoreTarget::Baseline => {
            for entry in &manifest.projects {
                match restore_entry_baseline(entry) {
                    Ok(()) => out.restored.push(entry.name.clone()),
                    Err(e) => out.failed.push(format!("「{}」: {e}", entry.name)),
                }
            }
        }
        RestoreTarget::Snapshot(name) => {
            let dir = resolve_snapshot_dir(&manifest.root, name)?;
            for entry in &manifest.projects {
                match restore_entry_from_snapshot(entry, &dir) {
                    Ok(()) => out.restored.push(entry.name.clone()),
                    Err(e) => out.failed.push(format!("「{}」: {e}", entry.name)),
                }
            }
        }
    }
    Ok(out)
}

/// 恢复单个项目副本到基线提交：受控文件还原为基线，删除未跟踪的非忽略文件
/// （保留 node_modules/.env 等被忽略文件，避免破坏可重新生成的本地产物与环境配置）
fn restore_entry_baseline(entry: &TempProjectEntry) -> Result<(), String> {
    let dir = PathBuf::from(&entry.temp);
    if !dir.join(".git").exists() {
        return Err(format!("「{}」缺少 git 仓库，无法恢复基线", entry.name));
    }
    let baseline = entry.baseline.clone().unwrap_or_else(|| "HEAD".into());
    run_git(&dir, &["reset", "--hard", &baseline])?;
    run_git(&dir, &["clean", "-fd"])?;
    Ok(())
}

/// 从快照恢复单个项目副本：
/// 1) 受控文件回到基线、清掉未跟踪非忽略文件；2) 基线有而快照没有的受控文件删除；
/// 3) 快照内容覆盖回工作区。恢复后工作区与基线的差异即快照相对基线的修改，diff 照常可用。
fn restore_entry_from_snapshot(entry: &TempProjectEntry, snap_root: &Path) -> Result<(), String> {
    let dir = PathBuf::from(&entry.temp);
    if !dir.exists() {
        return Err(format!("项目副本不存在: {}", entry.temp));
    }
    if !dir.join(".git").exists() {
        return Err(format!("「{}」缺少 git 仓库，无法恢复", entry.name));
    }
    let snap = snap_root.join(sanitize_component(&entry.key));
    if !snap.is_dir() {
        return Err(format!("快照中缺少项目「{}」的数据", entry.name));
    }
    run_git(&dir, &["reset", "--hard"])?;
    run_git(&dir, &["clean", "-fd"])?;
    let tracked = run_git(&dir, &["ls-files"])?;
    for f in tracked.lines() {
        let f = f.trim();
        if f.is_empty() || snap.join(f).is_file() {
            continue;
        }
        let p = dir.join(f);
        if p.is_file() {
            let _ = std::fs::remove_file(&p);
        }
    }
    copy_over(&snap, &dir)?;
    Ok(())
}

/// 把 src 目录的文件覆盖拷贝到已存在的 dst（补齐缺失的父目录；不删除 dst 独有文件）
fn copy_over(src: &Path, dst: &Path) -> Result<usize, String> {
    use ignore::WalkBuilder;
    let mut count = 0usize;
    let walker = WalkBuilder::new(src)
        .hidden(false)
        .require_git(false)
        .git_global(false)
        .parents(false)
        .build();
    for entry in walker {
        let entry = entry.map_err(|e| format!("遍历快照失败: {e}"))?;
        if entry.depth() == 0 {
            continue;
        }
        let Some(ft) = entry.file_type() else { continue };
        if !ft.is_file() {
            continue;
        }
        let rel = entry.path().strip_prefix(src).unwrap_or(entry.path());
        let target = dst.join(rel);
        if let Some(parent) = target.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::copy(entry.path(), &target)
            .map_err(|e| format!("恢复失败 {}: {e}", entry.path().display()))?;
        count += 1;
    }
    Ok(count)
}

/// 解析快照目录：精确名称优先，否则按 `<name>.snapshot-*` 前缀匹配取最新（时间戳字典序最大）
pub fn resolve_snapshot_dir(root: &str, name: &str) -> Result<PathBuf, String> {
    let base = PathBuf::from(root);
    let rd = std::fs::read_dir(&base).map_err(|e| format!("读取临时空间根目录失败: {e}"))?;
    let want = name.trim();
    let mut exact: Option<PathBuf> = None;
    let mut prefixed: Vec<PathBuf> = Vec::new();
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        if !n.contains(".snapshot-") {
            continue;
        }
        if n == want {
            exact = Some(e.path());
        } else if n.starts_with(&format!("{want}.snapshot-")) {
            prefixed.push(e.path());
        }
    }
    if let Some(p) = exact {
        return Ok(p);
    }
    prefixed.sort();
    prefixed
        .pop()
        .ok_or_else(|| format!("未找到快照「{want}」，可用快照见 temp_status 输出"))
}

/// 列出临时根目录下的快照目录名（供 temp_status 展示）
pub fn list_snapshots(root: &str) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(root) else { return Vec::new() };
    let mut v: Vec<String> = rd
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.contains(".snapshot-"))
        .collect();
    v.sort();
    v
}

/// Agent 发起的合并：与用户手动合并相同，但跳过「Agent 正在运行」互斥检查
/// （调用方正是 Agent 运行本身）。合并成功后同样落库状态并刷新 UI。
pub async fn merge_from_agent(
    state: &crate::AppState,
    app: &tauri::AppHandle,
    session_id: &str,
) -> Result<MergeSummary, String> {
    let session = {
        let db = state.db.lock().unwrap();
        store::get_session(&db, session_id)?.ok_or("会话不存在")?
    };
    if !session.is_temp {
        return Err("该对话不是临时空间对话".into());
    }
    if session.merged_pending {
        return Err("本次变更已合并，请等待用户清空临时空间后再发起新的修改".into());
    }
    let root = PathBuf::from(session.temp_root.clone().ok_or("临时空间缺失")?);
    {
        let data_dir = state.data_dir.lock().unwrap().clone().ok_or("数据目录不可用")?;
        validate_root(&root, &data_dir)?;
    }
    if !root.exists() {
        return Err("临时空间目录不存在，无法合并".into());
    }
    let (manifest, settings) = {
        let db = state.db.lock().unwrap();
        let master = state.master_key.lock().unwrap();
        (
            load_manifest(&db, session_id)?.ok_or("临时空间清单缺失")?,
            store::get_settings_with_secrets(&db, &master).unwrap_or_default(),
        )
    };
    let llm_cfg = resolve_active_model(&settings).map(|(pc, m)| LlmCfg {
        base_url: pc.base_url.clone(),
        api_key: pc.api_key.clone(),
        model: m.to_string(),
    });

    let summary = apply_merge(&manifest, llm_cfg.as_ref()).await;

    {
        let db = state.db.lock().unwrap();
        let max_seq: i64 = db
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) FROM messages WHERE session_id = ?1",
                [session_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        store::set_session_merge_state(&db, session_id, Some(max_seq), true)?;
    }
    emit_session_update(state, app, session_id);
    emit_temp_update(state, app, session_id);
    Ok(summary)
}

/// 把合并结果格式化为给 Agent 的文本（含需人工处理的冲突清单与后续指引）
pub fn format_merge_summary(s: &MergeSummary) -> String {
    let mut t = format!(
        "合并完成：直接写回 {} 个文件，AI 智能合并 {} 个，需人工处理 {} 个。",
        s.total_applied, s.total_ai_merged, s.total_skipped
    );
    for p in &s.projects {
        t.push_str(&format!(
            "\n- 「{}」：直接写回 {}，AI 合并 {}，人工处理 {}",
            p.name,
            p.applied,
            p.ai_merged,
            p.skipped.len()
        ));
        for x in &p.skipped {
            t.push_str(&format!("\n    - {x}"));
        }
    }
    if s.total_skipped > 0 {
        t.push_str(
            "\n存在需人工处理的冲突文件（原始目录与临时空间都改动过且无法自动合并）。\n\
             请把上面的文件清单明确告知用户，在编辑器中人工处理；本次合并已记录，不要再次调用 temp_merge。",
        );
    }
    t
}

// ---------- system prompt 临时空间段 ----------

/// 组装临时空间会话注入 system prompt 的说明段（含 优先级：临时空间 > 关联项目 > 项目约束，
/// 以及"只做静态检查，不编译不运行"的验证要求）。
pub fn build_prompt_section(conn: &rusqlite::Connection, session: &Session, manifest: &TempManifest) -> String {
    let project = session
        .project_id
        .as_deref()
        .and_then(|pid| store::get_project(conn, pid).ok().flatten());
    let main = manifest.projects.iter().find(|p| p.key == "main");
    let links: Vec<&TempProjectEntry> = manifest
        .projects
        .iter()
        .filter(|p| p.key != "main")
        .collect();
    let constraints = project
        .as_ref()
        .map(|p| p.constraints.trim())
        .filter(|c| !c.is_empty());

    let mut s = String::from("## 临时空间（本说明优先级最高）\n");
    if let Some(m) = main {
        s.push_str(&format!(
            "- 当前工作区是主项目「{}」的临时副本：{}；原始目录：{}（不会被修改）。\n",
            m.name, m.temp, m.source
        ));
    }
    if !links.is_empty() {
        s.push_str("- 关联项目已一并拷贝到临时空间：\n");
        for l in &links {
            let desc = if l.description.trim().is_empty() {
                String::new()
            } else {
                format!("；说明：{}", l.description.trim())
            };
            s.push_str(&format!(
                "  - 「{}」：{} → {}{}\n",
                l.name, l.source, l.temp, desc
            ));
        }
    }
    s.push_str("- 你的所有修改只能落在上述临时副本内；仅当用户执行「合并」后，变更才会写回原始目录。\n");
    s.push_str("- 在本空间内修改代码后，不需要编译和运行项目；完成代码修改后，只需完成静态检查（语法检查 / lint / 类型检查）即可。\n");
    if constraints.is_some() || !links.is_empty() {
        s.push_str("\n**优先级**：临时空间说明 > 关联项目（目录约定、说明及其自身约束）> 项目约束；冲突时以靠前者为准。\n");
    }

    if let Some(c) = constraints {
        s.push_str("\n## 项目约束（必须严格遵守）\n");
        s.push_str(c);
        s.push('\n');
    }

    if !links.is_empty() {
        s.push_str("\n## 关联项目\n本项目与以下项目存在关联；以下路径为临时空间中的副本（跨目录引用其代码时使用临时路径）：\n");
        for l in &links {
            s.push_str(&format!(
                "\n### {}（临时副本：{}；原始目录：{}）\n",
                l.name, l.temp, l.source
            ));
            s.push_str(l.description.trim());
            s.push('\n');
            // 对方项目实体存在且已设自身约束时一并注入（按原始路径解析）
            if let Ok(Some(p)) = store::find_project_by_path(conn, &l.source) {
                if !p.constraints.trim().is_empty() {
                    s.push_str("\n该项目自身约束（同样必须遵守）：\n");
                    s.push_str(p.constraints.trim());
                    s.push('\n');
                }
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("hm_temp_{tag}_{}", uuid::Uuid::new_v4()))
    }

    fn write_file(p: &Path, content: &str) {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, content).unwrap();
    }

    #[test]
    fn copy_project_respects_gitignore_and_excludes() {
        let src = temp_dir("src");
        let dst = temp_dir("dst");
        write_file(&src.join("a.txt"), "a");
        write_file(&src.join("sub/b.txt"), "b");
        write_file(&src.join("node_modules/x.js"), "x");
        write_file(&src.join("target/y.bin"), "y");
        write_file(&src.join("dist/z.js"), "z");
        write_file(&src.join("debug.log"), "log");
        write_file(&src.join(".gitignore"), "debug.log\n");
        write_file(&src.join(".env.example"), "K=v");

        let n = copy_project(&src, &dst).unwrap();
        assert!(n >= 3);
        assert!(dst.join("a.txt").exists());
        assert!(dst.join("sub/b.txt").exists());
        assert!(dst.join(".gitignore").exists());
        assert!(dst.join(".env.example").exists());
        assert!(!dst.join("node_modules").exists());
        assert!(!dst.join("target").exists());
        assert!(!dst.join("dist").exists());
        assert!(!dst.join("debug.log").exists());
        std::fs::remove_dir_all(&src).ok();
        std::fs::remove_dir_all(&dst).ok();
    }

    #[test]
    fn is_under_rejects_outside_paths() {
        let data = temp_dir("data");
        let base = data.join("temp-project");
        assert!(is_under(&base.join("abc"), &base));
        assert!(!is_under(&base, &base));
        assert!(!is_under(&data.join("other"), &base));
        // Windows 下分隔符与大小写差异
        if cfg!(windows) {
            assert!(is_under(
                &PathBuf::from("d:/DATA/temp-project/ABC/x"),
                &PathBuf::from("D:\\data\\temp-project")
            ));
        }
        std::fs::remove_dir_all(&data).ok();
    }

    #[test]
    fn alloc_generates_unique_code_and_entries() {
        let data = temp_dir("data");
        let project = Project {
            id: "p1".into(),
            name: "app".into(),
            path: Some("D:\\ws\\app".into()),
            pinned: false,
            created_at: String::new(),
            last_activity_at: None,
            constraints: String::new(),
        };
        let links = vec![
            ProjectLink { id: "l1".into(), project_id: "p1".into(), path: "D:\\libs\\core".into(), description: "工具库".into(), created_at: String::new() },
            ProjectLink { id: "l2".into(), project_id: "p1".into(), path: "D:\\other\\app".into(), description: String::new(), created_at: String::new() },
        ];
        let a1 = alloc(&data, &project, &links).unwrap();
        let a2 = alloc(&data, &project, &links).unwrap();
        assert_ne!(a1.code, a2.code);
        assert_eq!(a1.projects.len(), 3); // 主项目 + 2 关联
        assert_eq!(a1.projects[0].key, "main");
        assert_eq!(PathBuf::from(&a1.projects[0].temp), PathBuf::from(&a1.root).join("app"));
        // 重名目录自动追加序号
        assert_eq!(PathBuf::from(&a1.projects[2].temp), PathBuf::from(&a1.root).join("app-2"));
        std::fs::remove_dir_all(&data).ok();
    }

    #[test]
    fn baseline_and_change_detection_flow() {
        if !git_available() {
            return; // 环境无 git 时跳过
        }
        let dir = temp_dir("flow");
        write_file(&dir.join("a.txt"), "v1\n");
        write_file(&dir.join("sub/b.txt"), "b\n");
        let baseline = git_baseline(&dir).unwrap();
        assert!(!baseline.is_empty());

        let entry = TempProjectEntry {
            key: "main".into(),
            name: "t".into(),
            source: dir.to_string_lossy().to_string(),
            temp: dir.to_string_lossy().to_string(),
            description: String::new(),
            baseline: Some(baseline),
        };
        assert!(list_changes(&entry).unwrap().is_empty());

        write_file(&dir.join("a.txt"), "v2\n"); // M
        write_file(&dir.join("new.txt"), "n\n"); // A
        std::fs::remove_file(dir.join("sub/b.txt")).unwrap(); // D
        let mut changes = list_changes(&entry).unwrap();
        changes.sort_by(|x, y| x.path.cmp(&y.path));
        assert_eq!(changes.len(), 3);
        assert_eq!((changes[0].path.as_str(), changes[0].change), ("a.txt", 'M'));
        assert_eq!((changes[1].path.as_str(), changes[1].change), ("new.txt", 'A'));
        assert_eq!((changes[2].path.as_str(), changes[2].change), ("sub/b.txt", 'D'));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn apply_merge_writes_back_and_flags_conflicts() {
        if !git_available() {
            return;
        }
        let src = temp_dir("msrc");
        let tmp = temp_dir("mtmp");
        write_file(&src.join("a.txt"), "v1\n");
        write_file(&src.join("gone.txt"), "g\n");
        copy_project(&src, &tmp).unwrap();
        let baseline = git_baseline(&tmp).unwrap();

        // 临时空间内：修改 a.txt、新增 new.txt、删除 gone.txt
        write_file(&tmp.join("a.txt"), "v2\n");
        write_file(&tmp.join("new.txt"), "n\n");
        std::fs::remove_file(tmp.join("gone.txt")).unwrap();

        // 基线后在原目录改动 a.txt → 冲突（无模型 → 跳过，原文件不动）
        write_file(&src.join("a.txt"), "orig-change\n");

        let manifest = TempManifest {
            code: "x".into(),
            root: tmp.parent().unwrap().to_string_lossy().to_string(),
            projects: vec![TempProjectEntry {
                key: "main".into(),
                name: "t".into(),
                source: src.to_string_lossy().to_string(),
                temp: tmp.to_string_lossy().to_string(),
                description: String::new(),
                baseline: Some(baseline),
            }],
        };
        let summary = apply_merge(&manifest, None).await;
        assert_eq!(summary.total_applied, 2); // new.txt 写回 + gone.txt 删除
        assert_eq!(summary.total_ai_merged, 0);
        assert_eq!(summary.total_skipped, 1); // a.txt 冲突
        assert_eq!(std::fs::read_to_string(src.join("a.txt")).unwrap(), "orig-change\n");
        assert_eq!(std::fs::read_to_string(src.join("new.txt")).unwrap(), "n\n");
        assert!(!src.join("gone.txt").exists());
        std::fs::remove_dir_all(&src).ok();
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn strip_fence_removes_markdown_wrapper() {
        assert_eq!(strip_fence("```\nabc\n```"), "abc");
        assert_eq!(strip_fence("```rust\nabc\n```"), "abc");
        assert_eq!(strip_fence("abc"), "abc");
    }

    #[tokio::test]
    async fn changes_list_and_file_diff_flow() {
        if !git_available() {
            return;
        }
        let src = temp_dir("csrc");
        let tmp = temp_dir("ctmp");
        write_file(&src.join("a.txt"), "l1\nl2\nl3\n");
        write_file(&src.join("gone.txt"), "g\n");
        copy_project(&src, &tmp).unwrap();
        let baseline = git_baseline(&tmp).unwrap();

        // 修改 a.txt、新增 new.txt、删除 gone.txt
        write_file(&tmp.join("a.txt"), "l1\nl2-changed\nl3\nl4\n");
        write_file(&tmp.join("new.txt"), "n1\n");
        std::fs::remove_file(tmp.join("gone.txt")).unwrap();

        let entry = TempProjectEntry {
            key: "main".into(),
            name: "app".into(),
            source: src.to_string_lossy().to_string(),
            temp: tmp.to_string_lossy().to_string(),
            description: String::new(),
            baseline: Some(baseline),
        };

        let changes = list_temp_changes(&TempManifest {
            code: "x".into(),
            root: tmp.parent().unwrap().to_string_lossy().to_string(),
            projects: vec![entry.clone()],
        })
        .unwrap();
        assert_eq!(changes.total_files, 3);
        assert_eq!(changes.projects.len(), 1);
        let by_path = |p: &str| changes.projects[0].files.iter().find(|f| f.path == p).unwrap();
        assert_eq!(by_path("a.txt").change, "modified");
        assert_eq!((by_path("a.txt").added, by_path("a.txt").removed), (2, 1));
        assert_eq!(by_path("new.txt").change, "added");
        assert_eq!(by_path("new.txt").added, 1);
        assert_eq!(by_path("gone.txt").change, "deleted");
        assert_eq!(by_path("gone.txt").removed, 1);

        // 单文件 diff：分块结构 + 行号 + 前后内容
        let d = file_diff(&entry, "a.txt").unwrap();
        assert_eq!(d.change, "modified");
        assert!(!d.binary && !d.too_large && !d.truncated);
        assert_eq!(d.hunks.len(), 1);
        // similar 的 Replace op 先输出删除行再输出新增行
        let tags: Vec<&str> = d.hunks[0].lines.iter().map(|l| l.tag.as_str()).collect();
        assert_eq!(tags, vec!["same", "del", "add", "same", "add"]);
        let del = d.hunks[0].lines.iter().find(|l| l.tag == "del").unwrap();
        let add = d.hunks[0].lines.iter().find(|l| l.tag == "add" && l.text == "l2-changed").unwrap();
        assert_eq!((del.old_no, del.new_no, del.text.as_str()), (Some(2), None, "l2"));
        assert_eq!((add.old_no, add.new_no, add.text.as_str()), (None, Some(2), "l2-changed"));

        // 不在变更清单中的文件拒绝查看
        assert!(file_diff(&entry, "nope.txt").is_err());
        std::fs::remove_dir_all(&src).ok();
        std::fs::remove_dir_all(&tmp).ok();
    }
}
