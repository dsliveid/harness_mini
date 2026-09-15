//! tempctl —— harness_mini「临时空间」的独立 CLI。
//!
//! 背景：harness_mini 是能修改自身源码的 AI Agent 工具（自举开发）。为避免改动影响
//! 正在运行的程序实例，把「临时空间」能力提取为本 CLI：在数据目录
//! `<data>\temp-project\<随机码>\` 建立项目副本，AI 的所有改动只落在副本内，
//! 由用户审查后手动执行「合并」写回原目录，随后用户自行重启程序生效。
//!
//! 与主程序（src-tauri/src/temp.rs）的关系：
//! - 语义对齐：相同的拷贝排除规则、相同的冲突判定矩阵；合并前全量预检，
//!   有冲突则一个文件都不写（比 app 更保守，app 无模型时也会跳过冲突文件）；
//! - 存储差异：app 用 git 基线提交，本 CLI 用并行快照树 `.temp-base`（零 git 依赖，
//!   副本目录保持普通目录形态，不产生 .git，不影响用户在副本内自行 git 操作）；
//! - 合并方向与 app 一致：以原目录版本为主体，叠加临时空间版本的修改；
//!   冲突（双方都改过 / 一方删除一方修改 / 双方各自新增）一律跳过并报告，绝不静默覆盖。
//!
//! 目录布局（副本内）：
//! ```text
//! <temp>/<项目名>/           工作区副本（可自由改动）
//!   ├─ .temp-meta.json       项目清单（key/name/source/description）
//!   └─ .temp-base/           基线快照树（与源文件同构，用于三方判定与回滚）
//! ```

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use similar::TextDiff;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

// ---------- 常量（与 src-tauri/src/temp.rs 对齐） ----------

/// 拷贝排除目录（任意层级）；.gitignore 规则由 ignore crate 在遍历时另行生效
const COPY_EXCLUDE_DIRS: &[&str] = &[".git", "node_modules", "target", "dist", ".temp-base"];
/// 基线快照树目录名
const BASE_DIR: &str = ".temp-base";
/// 项目清单文件名
const META_FILE: &str = ".temp-meta.json";
/// 单文件 diff 内容上限（与 app 的 DIFF_MAX_FILE_BYTES 一致），超过按 too_large 处理
const DIFF_MAX_FILE_BYTES: usize = 1024 * 1024;
/// 合并输出 JSON 行的最大字节数（超过则提示用 --file 分批查看）
const MERGE_JSON_MAX_BYTES: usize = 256 * 1024;

// ---------- 模型 ----------

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Entry {
    key: String,
    name: String,
    source: String,
    temp: String,
    #[serde(default)]
    description: String,
}

impl Entry {
    fn temp_path(&self) -> PathBuf {
        PathBuf::from(&self.temp)
    }
    fn source_path(&self) -> PathBuf {
        PathBuf::from(&self.source)
    }
    fn base_path(&self) -> PathBuf {
        self.temp_path().join(BASE_DIR)
    }
}

// ---------- 输出与错误 ----------

fn out_json(kind: &str, fields: serde_json::Map<String, Value>) {
    let mut o = Map::new();
    o.insert("kind".into(), json!(kind));
    for (k, v) in fields {
        o.insert(k, v);
    }
    println!("{}", Value::Object(o));
}

fn info(msg: impl std::fmt::Display) {
    eprintln!("[tempctl] {msg}");
}

type R<T> = Result<T, String>;

fn err(msg: impl std::fmt::Display) -> String {
    format!("[tempctl] {msg}")
}

// ---------- 通用工具 ----------

fn is_binary(buf: &[u8]) -> bool {
    let n = buf.len().min(8192);
    buf[..n].contains(&0)
}

/// child 是否位于 parent 目录之下（且不等于 parent）；Windows 下忽略大小写与分隔符差异
fn is_under(child: &Path, parent: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().replace('\\', "/");
    let (ck, pk) = (norm(child), norm(parent));
    let (ck, pk) = if cfg!(windows) {
        (ck.to_lowercase(), pk.to_lowercase())
    } else {
        (ck, pk)
    };
    let (ck, pk) = (ck.trim_end_matches('/'), pk.trim_end_matches('/'));
    ck != pk && ck.starts_with(&format!("{pk}/"))
}

/// 校验临时空间根目录必须位于 `<数据目录>\temp-project\` 之下（防误删/误写）
fn validate_root(root: &Path, data_dir: &Path) -> R<()> {
    let base = data_dir.join("temp-project");
    if !is_under(root, &base) {
        return Err(err(format!(
            "临时空间根目录非法：{}（必须位于 {} 之下）",
            root.display(),
            base.display()
        )));
    }
    Ok(())
}

/// 解析数据目录：显式指定优先；否则用当前目录下的 .dev-data（开发模式约定）。
/// require 为 true 时（状态变更类命令）解析不到则报错。
fn resolve_data_dir(explicit: Option<&Path>, require: bool) -> R<PathBuf> {
    if let Some(d) = explicit {
        return Ok(d.to_path_buf());
    }
    let cwd = std::env::current_dir().map_err(|e| err(format!("无法获取当前目录: {e}")))?;
    let dev = cwd.join(".dev-data");
    if dev.is_dir() {
        return Ok(dev);
    }
    if require {
        return Err(err(format!(
            "无法定位数据目录：当前目录 {} 下没有 .dev-data，请用 --data-dir 显式指定",
            cwd.display()
        )));
    }
    Ok(dev)
}

fn read_json_file(p: &Path) -> R<Value> {
    let s = std::fs::read_to_string(p)
        .map_err(|e| err(format!("读取失败 {}: {e}", p.display())))?;
    serde_json::from_str(&s).map_err(|e| err(format!("JSON 解析失败 {}: {e}", p.display())))
}

// ---------- 拷贝（对齐 app：排除目录 + .gitignore 生效 + 符号链接跳过） ----------

fn copy_project(src: &Path, dst: &Path) -> R<usize> {
    use ignore::WalkBuilder;
    std::fs::create_dir_all(dst).map_err(|e| err(format!("创建目录失败 {}: {e}", dst.display())))?;
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
    let mut count = 0usize;
    for entry in walker {
        let entry = entry.map_err(|e| err(format!("遍历失败: {e}")))?;
        if entry.depth() == 0 {
            continue;
        }
        let Some(ft) = entry.file_type() else { continue };
        let rel = entry.path().strip_prefix(src).unwrap_or(entry.path());
        let target = dst.join(rel);
        if ft.is_dir() {
            std::fs::create_dir_all(&target)
                .map_err(|e| err(format!("创建目录失败 {}: {e}", target.display())))?;
        } else if ft.is_symlink() {
            continue; // 不拷贝符号链接，避免越界引用
        } else if ft.is_file() {
            if let Some(parent) = target.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::copy(entry.path(), &target)
                .map_err(|e| err(format!("拷贝失败 {}: {e}", entry.path().display())))?;
            count += 1;
        }
    }
    Ok(count)
}

// ---------- 基线快照树（替代 app 的 git 基线提交） ----------

fn write_meta(entry: &Entry) -> R<()> {
    let v = json!({
        "key": entry.key,
        "name": entry.name,
        "source": entry.source,
        "description": entry.description,
    });
    let p = entry.temp_path().join(META_FILE);
    std::fs::write(&p, serde_json::to_string_pretty(&v).unwrap_or_default())
        .map_err(|e| err(format!("写入清单失败 {}: {e}", p.display())))
}

fn read_meta(temp: &Path) -> R<Value> {
    read_json_file(&temp.join(META_FILE))
}

/// 建立基线：把刚拷贝完成的工作区原样存入 .temp-base（快照树）
fn snapshot_to_base(temp: &Path) -> R<usize> {
    let base = temp.join(BASE_DIR);
    std::fs::create_dir_all(&base).map_err(|e| err(format!("创建目录失败 {}: {e}", base.display())))?;
    let mut count = 0usize;
    for entry in walkdir::WalkDir::new(temp)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || e.file_name().to_string_lossy() != BASE_DIR)
    {
        let entry = entry.map_err(|e| err(format!("遍历失败: {e}")))?;
        if entry.depth() == 0 {
            continue;
        }
        let rel = entry.path().strip_prefix(temp).unwrap_or(entry.path());
        let target = base.join(rel);
        let ft = entry.file_type();
        if ft.is_dir() {
            std::fs::create_dir_all(&target)
                .map_err(|e| err(format!("创建目录失败 {}: {e}", target.display())))?;
        } else if ft.is_file() {
            if let Some(parent) = target.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::copy(entry.path(), &target)
                .map_err(|e| err(format!("快照失败 {}: {e}", entry.path().display())))?;
            count += 1;
        }
    }
    Ok(count)
}

// ---------- 项目条目定位 ----------

/// 判断目录名是否为快照/基线产物，不应视为项目目录
/// （快照命名形如 `<项目名>.snapshot-<十六进制时间戳>`，所以用 contains 而非 ends_with）
fn is_snapshot_name(name: &str) -> bool {
    name == BASE_DIR || name.contains(".snapshot")
}

fn entry_from_json(v: &Value) -> R<Entry> {
    let get = |k: &str| -> R<String> {
        v.get(k)
            .and_then(|x| x.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| err(format!("计划条目缺少字段 {k}")))
    };
    Ok(Entry {
        key: v.get("key").and_then(|x| x.as_str()).unwrap_or("main").into(),
        name: v.get("name").and_then(|x| x.as_str()).unwrap_or("project").into(),
        source: get("source")?,
        temp: get("temp")?,
        description: v.get("description").and_then(|x| x.as_str()).unwrap_or("").into(),
    })
}

/// 在临时空间根目录下定位项目条目：扫描各子目录的 .temp-meta.json。
/// key 为 None 时：恰好只有一个项目则取之；否则报错并列出全部可用项目。
fn find_entry(root: &Path, key: Option<&str>) -> R<Entry> {
    let mut found: Vec<Entry> = Vec::new();
    let rd = std::fs::read_dir(root).map_err(|e| err(format!("读取目录失败 {}: {e}", root.display())))?;
    for d in rd {
        let d = d.map_err(|e| err(format!("读取目录失败 {}: {e}", root.display())))?;
        if !d.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = d.file_name().to_string_lossy().to_string();
        if is_snapshot_name(&name) {
            continue; // 快照树 / 快照产物不是项目
        }
        let meta = d.path().join(META_FILE);
        if !meta.exists() {
            continue;
        }
        let v = read_meta(&d.path())?;
        found.push(Entry {
            key: v.get("key").and_then(|x| x.as_str()).unwrap_or("main").into(),
            name: v.get("name").and_then(|x| x.as_str()).unwrap_or("?").into(),
            source: v.get("source").and_then(|x| x.as_str()).unwrap_or("").into(),
            description: v.get("description").and_then(|x| x.as_str()).unwrap_or("").into(),
            temp: d.path().to_string_lossy().to_string(),
        });
    }
    if found.is_empty() {
        return Err(err(format!(
            "未在 {} 下找到任何临时空间项目（缺少 {META_FILE}）",
            root.display()
        )));
    }
    if let Some(k) = key {
        let avail = found
            .iter()
            .map(|e| format!("{}({})", e.name, e.key))
            .collect::<Vec<_>>()
            .join(", ");
        return found
            .into_iter()
            .find(|e| e.key == k || e.name == k)
            .ok_or_else(|| err(format!("未找到项目「{k}」；可用项目: {avail}")));
    }
    if found.len() == 1 {
        return Ok(found.remove(0));
    }
    Err(err(format!(
        "存在多个项目，请用 --project 指定: {}",
        found.iter().map(|e| format!("{}({})", e.name, e.key)).collect::<Vec<_>>().join(", ")
    )))
}

/// 未显式给 root 时：在数据目录 temp-project 下找唯一已初始化的空间
fn find_sole_space(data_dir: &Path) -> R<PathBuf> {
    let base = data_dir.join("temp-project");
    let mut spaces: Vec<PathBuf> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&base) {
        for d in rd.flatten() {
            if !d.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let has_project = std::fs::read_dir(d.path())
                .map(|rd2| {
                    rd2.flatten().any(|s| {
                        s.file_type().map(|t| t.is_dir()).unwrap_or(false)
                            && !is_snapshot_name(&s.file_name().to_string_lossy())
                            && s.path().join(META_FILE).exists()
                    })
                })
                .unwrap_or(false);
            if has_project {
                spaces.push(d.path());
            }
        }
    }
    match spaces.len() {
        0 => Err(err("数据目录下没有已初始化的临时空间；请用 root 参数指定")),
        1 => Ok(spaces.remove(0)),
        _ => Err(err(format!(
            "数据目录下有多个临时空间，请用 root 参数指定: {}",
            spaces.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(" ; ")
        ))),
    }
}

fn resolve_entry(root: Option<&Path>, project: Option<&str>, data_dir: &Path) -> R<Entry> {
    let root = match root {
        Some(r) => r.to_path_buf(),
        None => find_sole_space(data_dir)?,
    };
    validate_root(&root, data_dir)?;
    find_entry(&root, project)
}

// ---------- 变更检测（工作区 vs 基线，等价于 app 的 add -A + diff --cached） ----------

/// 递归收集目录下全部文件（相对路径，'/'分隔，排序）
fn collect_files(dir: &Path, skip: &[&str]) -> R<Vec<String>> {
    let mut out = Vec::new();
    if !dir.exists() {
        return Ok(out);
    }
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !skip.iter().any(|s| e.file_name().to_string_lossy() == *s))
    {
        let entry = entry.map_err(|e| err(format!("遍历失败: {e}")))?;
        if entry.depth() == 0 {
            continue;
        }
        if entry.file_type().is_file() {
            let rel = entry.path().strip_prefix(dir).unwrap_or(entry.path());
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    out.sort();
    Ok(out)
}

#[derive(Clone, Debug)]
struct ChangedFile {
    path: String,
    change: char, // 'A' 新增 | 'M' 修改 | 'D' 删除（相对基线）
}

/// 全量变更清单：工作区现有 ∪ 基线现有，逐文件比对内容与存在性
fn list_changes(entry: &Entry) -> R<Vec<ChangedFile>> {
    let temp = entry.temp_path();
    if !temp.exists() {
        return Ok(vec![]);
    }
    let live = collect_files(&temp, &[BASE_DIR, META_FILE])?;
    let base_dir = entry.base_path();
    // 基线侧用与工作区相同的排除规则，避免 meta 在 base 有、live 无而被误判为删除
    let base = collect_files(&base_dir, &[BASE_DIR, META_FILE])?;
    let mut live_map = std::collections::BTreeMap::new();
    for f in &live {
        live_map.insert(f.clone(), std::fs::read(temp.join(f)).unwrap_or_default());
    }
    let mut base_map = std::collections::BTreeMap::new();
    for f in &base {
        base_map.insert(f.clone(), std::fs::read(base_dir.join(f)).unwrap_or_default());
    }
    let mut files = Vec::new();
    for (p, lv) in &live_map {
        match base_map.get(p) {
            None => files.push(ChangedFile { path: p.clone(), change: 'A' }),
            Some(bv) if bv != lv => files.push(ChangedFile { path: p.clone(), change: 'M' }),
            _ => {}
        }
    }
    for p in base_map.keys() {
        if !live_map.contains_key(p) {
            files.push(ChangedFile { path: p.clone(), change: 'D' });
        }
    }
    Ok(files)
}

// ---------- 三方内容（基线 / 临时 / 原目录） ----------

#[derive(Clone, Debug)]
struct Sides {
    base: Option<Vec<u8>>,
    temp: Option<Vec<u8>>,
    orig: Option<Vec<u8>>,
}

/// base：临时空间创建时的快照；temp：工作区当前内容；orig：原目录当前内容。
fn read_sides(entry: &Entry, rel: &str, change: char) -> R<Sides> {
    let temp = entry.temp_path();
    Ok(Sides {
        base: std::fs::read(entry.base_path().join(rel)).ok(),
        temp: if change == 'D' { None } else { std::fs::read(temp.join(rel)).ok() },
        orig: std::fs::read(entry.source_path().join(rel)).ok(),
    })
}

/// 冲突判定（与 src-tauri/src/temp.rs 的判定矩阵完全一致）：
///   (None, None)   纯新增，原目录也没有        → 不冲突
///   (None, Some)   双方各自新增了同名文件      → 冲突
///   (Some, None)   原文件被删而临时空间在改    → 冲突
///   (Some, Some)   base == orig 原目录未变    → 不冲突
///                  base != orig 原目录已变    → 冲突
fn is_conflict(s: &Sides) -> bool {
    match (&s.base, &s.orig) {
        (None, None) => false,
        (None, Some(_)) => true,
        (Some(_), None) => true,
        (Some(b), Some(o)) => b != o,
    }
}

// ---------- diff 统计 ----------

fn line_stats(old: Option<&[u8]>, new: Option<&[u8]>) -> (usize, usize, bool, bool) {
    let too_large = old.map(|b| b.len() > DIFF_MAX_FILE_BYTES).unwrap_or(false)
        || new.map(|b| b.len() > DIFF_MAX_FILE_BYTES).unwrap_or(false);
    let binary = !too_large
        && (old.map(is_binary).unwrap_or(false) || new.map(is_binary).unwrap_or(false));
    if too_large || binary {
        return (0, 0, binary, too_large);
    }
    let old_str = String::from_utf8_lossy(old.unwrap_or(b""));
    let new_str = String::from_utf8_lossy(new.unwrap_or(b""));
    let diff = TextDiff::from_lines(old_str.as_ref(), new_str.as_ref());
    let (mut a, mut r) = (0usize, 0usize);
    for ch in diff.iter_all_changes() {
        match ch.tag() {
            similar::ChangeTag::Insert => a += 1,
            similar::ChangeTag::Delete => r += 1,
            similar::ChangeTag::Equal => {}
        }
    }
    (a, r, false, false)
}

 // ---------- 子命令实现 ----------

fn cmd_alloc(source: &Path, data_dir: &Path) -> R<()> {
    let source = source
        .canonicalize()
        .map_err(|e| err(format!("源目录不存在 {}: {e}", source.display())))?;
    let base = data_dir.join("temp-project");
    let code = loop {
        let u = uuid::Uuid::new_v4().simple().to_string();
        let c = u[..12].to_string();
        if !base.join(&c).exists() {
            break c;
        }
    };
    let root = base.join(&code);
    let name = source
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "project".into());
    let temp = root.join(&name);
    out_json(
        "alloc",
        Map::from_iter([
            ("code".into(), json!(code)),
            ("root".into(), json!(root.to_string_lossy())),
            (
                "projects".into(),
                json!([{ "key": "main", "name": name, "source": source.to_string_lossy(), "temp": temp.to_string_lossy(), "description": "主项目" }]),
            ),
        ]),
    );
    Ok(())
}

fn cmd_ensure(root: &Path, data_dir: &Path, projects_file: Option<&Path>, only: Option<&str>) -> R<()> {
    validate_root(root, data_dir)?;
    let Some(pf) = projects_file else {
        return Err(err("缺少 --projects 计划文件（alloc 输出的 projects JSON 行）。请先运行 alloc。"));
    };
    let content =
        std::fs::read_to_string(pf).map_err(|e| err(format!("读取计划失败 {}: {e}", pf.display())))?;
    let mut entries: Vec<Entry> = Vec::new();
    for line in content.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line)
            .map_err(|e| err(format!("计划行 JSON 解析失败: {e}")))?;
        // 兼容两种行格式：单个项目条目，或 alloc 的完整输出（含 projects 数组的外壳对象）
        match v.get("projects") {
            Some(Value::Array(arr)) => {
                for item in arr {
                    entries.push(entry_from_json(item)?);
                }
            }
            _ => entries.push(entry_from_json(&v)?),
        }
    }
    if let Some(k) = only {
        entries.retain(|e| e.key == k || e.name == k);
        if entries.is_empty() {
            return Err(err(format!("--project {k} 未命中任何计划条目")));
        }
    }
    for e in &entries {
        if !is_under(Path::new(&e.temp), root) {
            return Err(err(format!(
                "计划中的临时目录不在根目录之下: {} (root={})",
                e.temp,
                root.display()
            )));
        }
        if is_under(Path::new(&e.source), root) {
            return Err(err(format!("源目录不允许位于临时空间内部: {}", e.source)));
        }
    }
    let mut results = Vec::new();
    for e in &entries {
        let temp = e.temp_path();
        if temp.join(BASE_DIR).exists() {
            info(format!("跳过（已初始化）: {} → {}", e.source, e.temp));
            results.push(json!({"name": e.name, "temp": e.temp, "copied": 0, "skipped": true}));
            continue;
        }
        let src = e.source_path();
        if !src.exists() {
            return Err(err(format!("源目录不存在: {}", e.source)));
        }
        let copied = copy_project(&src, &temp)?;
        write_meta(e)?;
        let baselined = snapshot_to_base(&temp)?;
        info(format!(
            "已拷贝 {copied} 个文件并建立基线快照 {baselined} 个文件: {} → {}",
            e.source,
            e.temp
        ));
        results.push(json!({"name": e.name, "temp": e.temp, "copied": copied, "skipped": false}));
    }
    out_json("ensure", Map::from_iter([("projects".into(), json!(results))]));
    Ok(())
}

fn cmd_changes(root: Option<&Path>, project: Option<&str>, data_dir: &Path, as_json: bool) -> R<()> {
    let entry = resolve_entry(root, project, data_dir)?;
    let changes = list_changes(&entry)?;
    if as_json {
        out_json(
            "changes",
            Map::from_iter([
                ("project".into(), json!(entry.name)),
                ("key".into(), json!(entry.key)),
                ("temp".into(), json!(entry.temp)),
                ("source".into(), json!(entry.source)),
                ("count".into(), json!(changes.len())),
                (
                    "files".into(),
                    json!(changes
                        .iter()
                        .map(|f| json!({"path": f.path, "change": f.change.to_string()}))
                        .collect::<Vec<_>>()),
                ),
            ]),
        );
        return Ok(());
    }
    if changes.is_empty() {
        println!("[tempctl] 无变更（工作区与基线一致）: {}", entry.temp);
        return Ok(());
    }
    println!(
        "[tempctl] 项目「{}」变更 {} 个文件（{} ← {}）",
        entry.name,
        changes.len(),
        entry.temp,
        entry.source
    );
    for f in &changes {
        let sides = read_sides(&entry, &f.path, f.change)?;
        let (a, r, binary, too_large) = line_stats(sides.base.as_deref(), sides.temp.as_deref());
        let stat = if too_large {
            "(过大)".to_string()
        } else if binary {
            "(二进制)".to_string()
        } else {
            format!("(+{a}/-{r})")
        };
        println!("  [{}] {} {}", f.change, f.path, stat);
    }
    Ok(())
}

fn cmd_diff(root: &Path, project: Option<&str>, path: Option<&str>, data_dir: &Path) -> R<()> {
    validate_root(root, data_dir)?;
    let entry = find_entry(root, project)?;
    let changes = list_changes(&entry)?;
    let selected: Vec<ChangedFile> = match path {
        Some(p) => vec![changes
            .into_iter()
            .find(|f| f.path == p)
            .ok_or_else(|| err(format!("该文件不在变更清单中: {p}")))?],
        None => changes,
    };
    for f in &selected {
        let sides = read_sides(&entry, &f.path, f.change)?;
        let (a, r, binary, too_large) = line_stats(sides.base.as_deref(), sides.temp.as_deref());
        let mark = if too_large {
            "（过大，跳过内容）"
        } else if binary {
            "（二进制，跳过内容）"
        } else {
            ""
        };
        println!("### [{}] {} (+{a}/-{r}){mark}", f.change, f.path);
        if too_large || binary {
            continue;
        }
        let old_str = String::from_utf8_lossy(sides.base.as_deref().unwrap_or(b""));
        let new_str = String::from_utf8_lossy(sides.temp.as_deref().unwrap_or(b""));
        let diff = TextDiff::from_lines(old_str.as_ref(), new_str.as_ref());
        for ops in diff.grouped_ops(3) {
            for op in &ops {
                for ch in diff.iter_changes(op) {
                    let tag = match ch.tag() {
                        similar::ChangeTag::Delete => "-",
                        similar::ChangeTag::Insert => "+",
                        similar::ChangeTag::Equal => " ",
                    };
                    println!(
                        "{tag} {:>4} {:>4} | {}",
                        ch.old_index().map(|i| i + 1).unwrap_or(0),
                        ch.new_index().map(|i| i + 1).unwrap_or(0),
                        ch.value().trim_end_matches('\n').trim_end_matches('\r')
                    );
                }
            }
            println!();
        }
    }
    Ok(())
}

/// 合并前预检：逐文件判定是否可安全写回
struct Verdict {
    file: ChangedFile,
    ok: bool,
    reason: String,
}

fn plan_merge(entry: &Entry, only_files: &[String]) -> R<(Vec<Verdict>, bool)> {
    let changes = list_changes(entry)?;
    let changes: Vec<ChangedFile> = if only_files.is_empty() {
        changes
    } else {
        let hit: Vec<String> = only_files
            .iter()
            .filter(|p| !changes.iter().any(|f| &f.path == *p))
            .cloned()
            .collect();
        if !hit.is_empty() {
            return Err(err(format!(
                "以下文件不在变更清单中，请检查路径: {}",
                hit.join(", ")
            )));
        }
        changes
            .into_iter()
            .filter(|f| only_files.iter().any(|p| p == &f.path))
            .collect()
    };
    let mut plan = Vec::new();
    let mut all_ok = true;
    for f in changes {
        let sides = read_sides(entry, &f.path, f.change)?;
        let (ok, reason) = if f.change != 'D' && sides.temp.is_none() {
            (false, "临时空间文件读取失败".into())
        } else if f.change != 'D' && is_binary(sides.temp.as_deref().unwrap_or(b"")) {
            (false, "二进制文件需人工处理".into())
        } else if is_conflict(&sides) {
            let reason = match (&sides.base, &sides.orig) {
                (Some(_), None) => "原文件已不存在，而临时空间在修改/新增它，需人工处理".to_string(),
                (None, Some(_)) => "双方各自新增了同名文件，需人工处理".to_string(),
                _ => "原目录版本与基线不一致（双方都改过），需人工处理".to_string(),
            };
            (false, reason)
        } else {
            (true, String::new())
        };
        if !ok {
            all_ok = false;
        }
        plan.push(Verdict { file: f, ok, reason });
    }
    Ok((plan, all_ok))
}

fn cmd_merge(
    root: &Path,
    project: Option<&str>,
    files: &[String],
    dry_run: bool,
    data_dir: &Path,
) -> R<()> {
    validate_root(root, data_dir)?;
    let entry = find_entry(root, project)?;
    let (plan, all_ok) = plan_merge(&entry, files)?;

    let mut applied = 0usize;
    let mut skipped: Vec<String> = Vec::new();
    for v in &plan {
        if v.ok {
            info(format!("[{}] {} → 写入", v.file.change, v.file.path));
            applied += 1;
        } else {
            info(format!("[{}] {} → 跳过（{}）", v.file.change, v.file.path, v.reason));
            skipped.push(format!("{}: {}", v.file.path, v.reason));
        }
    }

    if dry_run {
        out_json(
            "merge-dry-run",
            Map::from_iter([
                ("project".into(), json!(entry.name)),
                ("would_apply".into(), json!(applied)),
                ("skipped".into(), json!(skipped)),
                ("all_ok".into(), json!(all_ok)),
            ]),
        );
        if !all_ok {
            return Err(err("存在冲突/异常文件：本次未写回任何文件（dry-run）"));
        }
        return Ok(());
    }

    // 全量预检失败：一个文件都不写（比 app 更保守，避免部分应用导致状态混乱）
    if !all_ok {
        out_json(
            "merge-blocked",
            Map::from_iter([
                ("project".into(), json!(entry.name)),
                ("applied".into(), json!(0)),
                ("skipped".into(), json!(skipped)),
            ]),
        );
        return Err(err(format!(
            "存在 {} 个冲突/异常文件，本次合并已整体中止（原目录未被修改）。\
             请人工处理后重试，或用 --file 指定无冲突子集。",
            skipped.len()
        )));
    }

    // 执行写回（此时全部通过预检）
    for v in &plan {
        let rel = &v.file.path;
        let orig_file = entry.source_path().join(rel);
        match v.file.change {
            'D' => {
                // plan_merge 已确认 base == orig
                let _ = std::fs::remove_file(&orig_file);
            }
            _ => {
                let temp_file = entry.temp_path().join(rel);
                if let Some(parent) = orig_file.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                std::fs::copy(&temp_file, &orig_file)
                    .map_err(|e| err(format!("写回失败 {}: {e}", orig_file.display())))?;
            }
        }
    }
    let mut fields = Map::from_iter([
        ("kind".into(), json!("merge")),
        ("project".into(), json!(entry.name)),
        ("source".into(), json!(entry.source)),
        ("applied".into(), json!(applied)),
        ("skipped".into(), json!(skipped)),
    ]);
    // 变更文件较多时不在 stdout 列出全部，避免刷屏（可用 changes --json 查全量）
    let full_len = Value::Object(fields.clone()).to_string().len();
    if full_len > MERGE_JSON_MAX_BYTES {
        fields.insert("truncated".into(), json!(true));
    }
    println!("{}", Value::Object(fields));
    Ok(())
}

/// 快照当前工作区（合并/恢复前的保险，产物落在临时空间根目录下，命名带 .snapshot 后缀）
fn cmd_snapshot(root: &Path, project: Option<&str>, data_dir: &Path) -> R<()> {
    validate_root(root, data_dir)?;
    let entry = find_entry(root, project)?;
    let temp = entry.temp_path();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let snap = temp
        .parent()
        .unwrap_or(&temp)
        .join(format!("{}.snapshot-{stamp:x}", temp.file_name().unwrap_or_default().to_string_lossy()));
    let copied = copy_project(&temp, &snap)?;
    // 快照不包含基线树（保持轻量）
    let base_in_snap = snap.join(BASE_DIR);
    if base_in_snap.exists() {
        std::fs::remove_dir_all(&base_in_snap).map_err(|e| err(format!("清理快照基线失败: {e}")))?;
    }
    info(format!("已快照 {copied} 个文件 → {}", snap.display()));
    out_json(
        "snapshot",
        Map::from_iter([
            ("snapshot_dir".into(), json!(snap.to_string_lossy())),
            ("files".into(), json!(copied)),
        ]),
    );
    Ok(())
}

/// 把工作区恢复到基线（丢弃未合并改动；合并前审查不通过时用）。基线树与清单保持不变。
fn cmd_restore(root: &Path, project: Option<&str>, data_dir: &Path) -> R<()> {
    validate_root(root, data_dir)?;
    let entry = find_entry(root, project)?;
    let temp = entry.temp_path();
    let base = entry.base_path();
    if !base.exists() {
        return Err(err(format!("基线快照不存在: {}", base.display())));
    }
    let files = collect_files(&temp, &[BASE_DIR, META_FILE])?;
    for p in &files {
        std::fs::remove_file(temp.join(p)).map_err(|e| err(format!("清理工作区失败 {p}: {e}")))?;
    }
    let mut restored = 0usize;
    for p in collect_files(&base, &[])? {
        let to = temp.join(&p);
        if let Some(parent) = to.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::copy(base.join(&p), &to)
            .map_err(|e| err(format!("恢复失败 {p}: {e}")))?;
        restored += 1;
    }
    info(format!("已恢复到基线，共 {restored} 个文件: {}", temp.display()));
    out_json("restore", Map::from_iter([("restored".into(), json!(restored))]));
    Ok(())
}

fn cmd_clear(root: &Path, data_dir: &Path) -> R<()> {
    validate_root(root, data_dir)?;
    if !root.exists() {
        info(format!("目录不存在，视为已清空: {}", root.display()));
        out_json("clear", Map::from_iter([("removed".into(), json!(false))]));
        return Ok(());
    }
    std::fs::remove_dir_all(root).map_err(|e| err(format!("删除失败 {}: {e}", root.display())))?;
    info(format!("已删除临时空间: {}", root.display()));
    out_json("clear", Map::from_iter([("removed".into(), json!(true))]));
    Ok(())
}

fn cmd_list(data_dir: &Path) -> R<()> {
    let base = data_dir.join("temp-project");
    let mut spaces = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&base) {
        for d in rd.flatten() {
            if !d.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let mut projects = Vec::new();
            if let Ok(rd2) = std::fs::read_dir(d.path()) {
                for s in rd2.flatten() {
                    let name = s.file_name().to_string_lossy().to_string();
                    if is_snapshot_name(&name) {
                        continue;
                    }
                    let meta = s.path().join(META_FILE);
                    if !meta.exists() {
                        continue;
                    }
                    let Ok(v) = read_meta(s.path().as_path()) else { continue };
                    let entry = Entry {
                        key: v.get("key").and_then(|x| x.as_str()).unwrap_or("main").into(),
                        name: v.get("name").and_then(|x| x.as_str()).unwrap_or("?").into(),
                        source: v.get("source").and_then(|x| x.as_str()).unwrap_or("").into(),
                        description: v.get("description").and_then(|x| x.as_str()).unwrap_or("").into(),
                        temp: s.path().to_string_lossy().to_string(),
                    };
                    let count = list_changes(&entry).map(|c| c.len()).unwrap_or(0);
                    projects.push(json!({
                        "key": entry.key, "name": entry.name, "source": entry.source,
                        "temp": entry.temp, "changed_files": count,
                    }));
                }
            }
            if !projects.is_empty() {
                spaces.push(json!({"root": d.path().to_string_lossy(), "projects": projects}));
            }
        }
    }
    out_json(
        "list",
        Map::from_iter([
            ("data_dir".into(), json!(data_dir.to_string_lossy())),
            ("spaces".into(), json!(spaces)),
        ]),
    );
    Ok(())
}

// ---------- 参数解析（手写，避免引入 clap；离线可编译） ----------

struct Args {
    data_dir: Option<PathBuf>,
    cmd: Cmd,
}

enum Cmd {
    Alloc { source: PathBuf },
    Ensure { root: PathBuf, projects: Option<PathBuf>, project: Option<String> },
    Changes { root: Option<PathBuf>, project: Option<String>, json: bool },
    Diff { root: PathBuf, project: Option<String>, path: Option<String> },
    Merge { root: PathBuf, project: Option<String>, files: Vec<String>, dry_run: bool },
    Snapshot { root: PathBuf, project: Option<String> },
    Restore { root: PathBuf, project: Option<String> },
    Clear { root: PathBuf },
    List,
}

const USAGE: &str = "用法:
  tempctl [--data-dir <目录>] <子命令> [参数]

子命令:
  alloc <源目录>                          生成临时空间计划（JSON，不落盘）
  ensure --root <根目录> --projects <计划文件> [--project <key|名称>]
                                          拷贝项目副本并建立基线快照（幂等）
  changes [root] [--project <k>] [--json] 变更清单（root 缺省时自动发现唯一空间）
  diff --root <根目录> [--project <k>] [--path <文件>]
                                          变更详情（统一 diff 格式）
  merge --root <根目录> [--project <k>] [--file <路径>]... [--dry-run]
                                          合并写回原目录（先全量预检，有冲突则整体中止）
  snapshot --root <根目录> [--project <k>]  快照当前工作区
  restore --root <根目录> [--project <k>]   恢复工作区到基线（丢弃未合并改动）
  clear --root <根目录>                    删除整个临时空间
  list                                    列出数据目录下全部临时空间及变更数

数据目录: --data-dir 显式指定；缺省时取当前目录下的 .dev-data（开发模式约定）。
临时空间根目录必须位于 <数据目录>\\temp-project\\ 之下（安全校验）。";

fn parse_args() -> R<Args> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() || argv.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        std::process::exit(0);
    }
    if argv.iter().any(|a| a == "-V" || a == "--version") {
        println!("tempctl 0.1.0");
        std::process::exit(0);
    }

    let mut data_dir: Option<PathBuf> = None;
    let mut rest: Vec<String> = Vec::new();
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--data-dir" => {
                i += 1;
                let v = argv.get(i).ok_or_else(|| err("--data-dir 需要一个值"))?;
                data_dir = Some(PathBuf::from(v));
            }
            other => rest.push(other.to_string()),
        }
        i += 1;
    }
    let Some(cmd) = rest.first().cloned() else {
        return Err(err(format!("缺少子命令\n\n{USAGE}")));
    };
    let args = &rest[1..];

    // 从扁平参数中抽取通用选项
    let mut root: Option<PathBuf> = None;
    let mut project: Option<String> = None;
    let mut json = false;
    let mut dry_run = false;
    let mut files: Vec<String> = Vec::new();
    let mut path_opt: Option<String> = None;
    let mut projects_file: Option<PathBuf> = None;
    let mut positionals: Vec<String> = Vec::new();
    let mut j = 0;
    while j < args.len() {
        let a = args[j].as_str();
        match a {
            "--root" => {
                j += 1;
                root = Some(PathBuf::from(args.get(j).ok_or_else(|| err("--root 需要一个值"))?));
            }
            "--project" => {
                j += 1;
                project = Some(args.get(j).ok_or_else(|| err("--project 需要一个值"))?.clone());
            }
            "--path" => {
                j += 1;
                path_opt = Some(args.get(j).ok_or_else(|| err("--path 需要一个值"))?.clone());
            }
            "--projects" => {
                j += 1;
                projects_file = Some(PathBuf::from(args.get(j).ok_or_else(|| err("--projects 需要一个值"))?));
            }
            "--file" => {
                j += 1;
                files.push(args.get(j).ok_or_else(|| err("--file 需要一个值"))?.clone());
            }
            "--json" => json = true,
            "--dry-run" => dry_run = true,
            other => positionals.push(other.to_string()),
        }
        j += 1;
    }

    let need_root = |what: &str| -> R<PathBuf> {
        root.clone().ok_or_else(|| err(format!("{what} 需要 --root 参数")))
    };

    let cmd = match cmd.as_str() {
        "alloc" => Cmd::Alloc {
            source: PathBuf::from(
                positionals
                    .first()
                    .ok_or_else(|| err("alloc 需要一个源目录参数"))?,
            ),
        },
        "ensure" => Cmd::Ensure {
            root: need_root("ensure")?,
            projects: projects_file,
            project,
        },
        // changes 允许用位置参数携带 root（tempctl changes <root>）
        "changes" => {
            let root = root.or_else(|| positionals.first().map(PathBuf::from));
            if root.is_none() && positionals.len() > 1 {
                return Err(err("changes 最多接受一个位置参数（root）"));
            }
            Cmd::Changes { root, project, json }
        }
        "diff" => {
            if !positionals.is_empty() {
                return Err(err(format!("diff 不接受位置参数: {}", positionals.join(" "))));
            }
            Cmd::Diff { root: need_root("diff")?, project, path: path_opt }
        }
        "merge" => {
            if !positionals.is_empty() {
                return Err(err(format!("merge 不接受位置参数: {}", positionals.join(" "))));
            }
            Cmd::Merge { root: need_root("merge")?, project, files, dry_run }
        }
        "snapshot" => {
            if !positionals.is_empty() {
                return Err(err(format!("snapshot 不接受位置参数: {}", positionals.join(" "))));
            }
            Cmd::Snapshot { root: need_root("snapshot")?, project }
        }
        "restore" => {
            if !positionals.is_empty() {
                return Err(err(format!("restore 不接受位置参数: {}", positionals.join(" "))));
            }
            Cmd::Restore { root: need_root("restore")?, project }
        }
        "clear" => {
            if !positionals.is_empty() {
                return Err(err(format!("clear 不接受位置参数: {}", positionals.join(" "))));
            }
            Cmd::Clear { root: need_root("clear")? }
        }
        "list" => Cmd::List,
        other => return Err(err(format!("未知子命令「{other}」\n\n{USAGE}"))),
    };
    Ok(Args { data_dir, cmd })
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let require = !matches!(args.cmd, Cmd::Changes { .. } | Cmd::List);
    let data_dir = match resolve_data_dir(args.data_dir.as_deref(), require) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let r = match args.cmd {
        Cmd::Alloc { source } => cmd_alloc(&source, &data_dir),
        Cmd::Ensure { root, projects, project } => cmd_ensure(&root, &data_dir, projects.as_deref(), project.as_deref()),
        Cmd::Changes { root, project, json } => cmd_changes(root.as_deref(), project.as_deref(), &data_dir, json),
        Cmd::Diff { root, project, path } => cmd_diff(&root, project.as_deref(), path.as_deref(), &data_dir),
        Cmd::Merge { root, project, files, dry_run } => cmd_merge(&root, project.as_deref(), &files, dry_run, &data_dir),
        Cmd::Snapshot { root, project } => cmd_snapshot(&root, project.as_deref(), &data_dir),
        Cmd::Restore { root, project } => cmd_restore(&root, project.as_deref(), &data_dir),
        Cmd::Clear { root } => cmd_clear(&root, &data_dir),
        Cmd::List => cmd_list(&data_dir),
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

// ---------- 测试 ----------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_binary_detects_nul() {
        assert!(is_binary(&[0x68, 0x00, 0x69]));
        assert!(!is_binary(b"hello world"));
    }

    #[test]
    fn is_under_checks_prefix() {
        assert!(is_under(Path::new("D:\\a\\b\\c"), Path::new("D:\\a\\b")));
        assert!(!is_under(Path::new("D:\\a\\b"), Path::new("D:\\a\\b")));
        assert!(!is_under(Path::new("D:\\a\\bc"), Path::new("D:\\a\\b")));
        assert!(is_under(Path::new("d:\\A\\B\\c"), Path::new("D:\\a\\b")));
    }

    #[test]
    fn conflict_matrix_matches_app() {
        let mk = |base: Option<&[u8]>, temp: Option<&[u8]>, orig: Option<&[u8]>| Sides {
            base: base.map(|b| b.to_vec()),
            temp: temp.map(|b| b.to_vec()),
            orig: orig.map(|b| b.to_vec()),
        };
        // 纯新增：双方都没有 → 不冲突
        assert!(!is_conflict(&mk(None, Some(b"t"), None)));
        // 双方各自新增同名文件 → 冲突
        assert!(is_conflict(&mk(None, Some(b"t"), Some(b"o"))));
        // 原文件被删而临时空间在改 → 冲突
        assert!(is_conflict(&mk(Some(b"b"), Some(b"t"), None)));
        // 原目录未变 → 不冲突
        assert!(!is_conflict(&mk(Some(b"same"), Some(b"t"), Some(b"same"))));
        // 原目录在基线后变过 → 冲突
        assert!(is_conflict(&mk(Some(b"base"), Some(b"t"), Some(b"orig-modified"))));
    }
}
