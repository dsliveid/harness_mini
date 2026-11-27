//! 页面快照增量机制（snapshot diff）
//!
//! 解决的问题：
//! 1. `cdp.rs` 的 console/network 缓冲只做长度截断、永不清空，而 `to_markdown` 每次固定取
//!    「末 5 条 / 末 8 条」——导致无论本次操作发生了什么，都反复回吐同一批陈旧错误，
//!    既浪费 token 又误导 Agent 归因。
//! 2. 每次操作都全量 dump 整页元素树，一次登录流程 5 步就要吃 5 份重复快照。
//!
//! 机制：会话内维护 `PageSnapshot` 游标，每次取状态后只输出 added/removed/modified
//! 与「自上次以来新增的错误」。

use super::session::{BrowserDebugState, InteractiveElement, ViewportInfo};
use std::collections::{BTreeMap, BTreeSet};

/// 元素指纹（用于跨次比对，与交互 ref 同源）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementFingerprint {
    pub ref_id: String,
    pub tag: String,
    pub r#type: Option<String>,
    pub name: Option<String>,
    pub placeholder: Option<String>,
    pub text: Option<String>,
    pub value: Option<String>,
}

impl ElementFingerprint {
    fn from_element(el: &InteractiveElement) -> Self {
        Self {
            ref_id: el.ref_id.clone(),
            tag: el.tag.clone(),
            r#type: el.r#type.clone(),
            name: el.name.clone(),
            placeholder: el.placeholder.clone(),
            text: el.text.clone(),
            value: el.value.clone(),
        }
    }

    /// 返回与另一指纹的字段级差异 (字段名, 旧值, 新值)
    fn diff_fields(&self, other: &Self) -> Vec<(String, String, String)> {
        let mut out = Vec::new();
        let pairs: [(&str, &Option<String>, &Option<String>); 5] = [
            ("type", &self.r#type, &other.r#type),
            ("name", &self.name, &other.name),
            ("placeholder", &self.placeholder, &other.placeholder),
            ("text", &self.text, &other.text),
            ("value", &self.value, &other.value),
        ];
        for (field, old, new) in pairs {
            if old != new {
                out.push((
                    field.to_string(),
                    old.clone().unwrap_or_default(),
                    new.clone().unwrap_or_default(),
                ));
            }
        }
        out
    }
}

/// 单次页面快照
#[derive(Debug, Clone, Default)]
pub struct PageSnapshot {
    pub url: String,
    pub title: String,
    pub elements: BTreeMap<String, ElementFingerprint>,
    pub notices: BTreeSet<String>,
    pub overlays: BTreeSet<String>,
    pub tables_hash: u64,
    pub tables: Vec<String>,
    pub viewport: ViewportInfo,
    pub console_cursor: usize,
    pub network_cursor: usize,
}

impl PageSnapshot {
    pub fn from_state(state: &BrowserDebugState) -> Self {
        let mut elements = BTreeMap::new();
        for el in &state.elements {
            elements.insert(el.ref_id.clone(), ElementFingerprint::from_element(el));
        }
        Self {
            url: state.url.clone(),
            title: state.title.clone(),
            elements,
            notices: state.notices.iter().cloned().collect(),
            overlays: state.dom_overlays.iter().cloned().collect(),
            tables_hash: hash_str(&state.tables.join("\u{1}")),
            tables: state.tables.clone(),
            viewport: state.viewport.clone(),
            console_cursor: 0,
            network_cursor: 0,
        }
    }
}

/// 两次快照的差异
#[derive(Debug, Clone, Default)]
pub struct SnapshotDiff {
    pub url_changed: Option<(String, String)>,
    pub title_changed: Option<(String, String)>,
    pub added: Vec<ElementFingerprint>,
    pub removed: Vec<String>,
    pub modified: Vec<(String, Vec<(String, String, String)>)>,
    pub notices_added: Vec<String>,
    pub overlays_added: Vec<String>,
    pub tables_changed: bool,
    pub console_new: Vec<String>,
    pub network_new: Vec<String>,
    pub viewport_changed: bool,
}

impl SnapshotDiff {
    pub fn is_empty(&self) -> bool {
        self.url_changed.is_none()
            && self.title_changed.is_none()
            && self.added.is_empty()
            && self.removed.is_empty()
            && self.modified.is_empty()
            && self.notices_added.is_empty()
            && self.overlays_added.is_empty()
            && !self.tables_changed
            && self.console_new.is_empty()
            && self.network_new.is_empty()
            && !self.viewport_changed
    }
}

/// 计算两次快照之间的差异
pub fn compute_diff(prev: &PageSnapshot, curr: &PageSnapshot) -> SnapshotDiff {
    let mut diff = SnapshotDiff::default();

    if prev.url != curr.url {
        diff.url_changed = Some((prev.url.clone(), curr.url.clone()));
    }
    if prev.title != curr.title {
        diff.title_changed = Some((prev.title.clone(), curr.title.clone()));
    }

    // 元素三分类：added / removed / modified
    for (ref_id, curr_el) in &curr.elements {
        match prev.elements.get(ref_id) {
            None => diff.added.push(curr_el.clone()),
            Some(prev_el) => {
                let fields = prev_el.diff_fields(curr_el);
                if !fields.is_empty() {
                    diff.modified.push((ref_id.clone(), fields));
                }
            }
        }
    }
    for ref_id in prev.elements.keys() {
        if !curr.elements.contains_key(ref_id) {
            diff.removed.push(ref_id.clone());
        }
    }

    // 通知与遮罩：只回新增
    for n in &curr.notices {
        if !prev.notices.contains(n) {
            diff.notices_added.push(n.clone());
        }
    }
    for o in &curr.overlays {
        if !prev.overlays.contains(o) {
            diff.overlays_added.push(o.clone());
        }
    }

    diff.tables_changed = prev.tables_hash != curr.tables_hash;

    diff.viewport_changed = prev.viewport.scroll_y != curr.viewport.scroll_y
        || prev.viewport.scroll_x != curr.viewport.scroll_x
        || prev.viewport.current_page != curr.viewport.current_page;

    diff
}

/// 游标切片：返回自上次以来新增的日志
///
/// 缓冲因容量上限截断导致 `len < cursor` 时（旧条目已被挤掉），游标重置并返回全部现存条目。
pub fn slice_since(logs: &[String], cursor: usize) -> (Vec<String>, usize, bool) {
    if cursor > logs.len() {
        // 缓冲已滚动，旧游标失效
        return (logs.to_vec(), logs.len(), true);
    }
    (logs[cursor..].to_vec(), logs.len(), false)
}

/// FNV-1a 64 位哈希（用于表格内容比对）
fn hash_str(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

// ---------------- 渲染层：按 detail 分级输出 ----------------

/// 极简摘要（summary）：只给 URL/标题/变化计数/新增错误
pub fn render_summary(snap: &PageSnapshot, diff: &SnapshotDiff) -> String {
    let mut md = String::new();
    md.push_str(&format!("- **URL**: {}\n- **标题**: {}\n", snap.url, snap.title));
    md.push_str(&format!(
        "- **页面变化**: 新增 {} / 消失 {} / 变更 {} | 表格{}变化\n",
        diff.added.len(),
        diff.removed.len(),
        diff.modified.len(),
        if diff.tables_changed { "" } else { "未" }
    ));
    if let Some((old, new)) = &diff.url_changed {
        md.push_str(&format!("- **URL 跳转**: `{old}` → `{new}`\n"));
    }
    append_new_errors(&mut md, diff);
    md
}

/// 增量视图（diff）：只列变化项
pub fn render_diff(snap: &PageSnapshot, diff: &SnapshotDiff) -> String {
    let mut md = String::new();
    md.push_str(&format!("- **URL**: {}\n- **标题**: {}\n", snap.url, snap.title));

    if let Some((old, new)) = &diff.url_changed {
        md.push_str(&format!("- **URL 跳转**: `{old}` → `{new}`\n"));
    }
    if let Some((old, new)) = &diff.title_changed {
        md.push_str(&format!("- **标题变更**: `{old}` → `{new}`\n"));
    }

    if diff.is_empty() {
        md.push_str("\n> ℹ️ 页面与上次快照完全一致，无任何变化。\n");
        return md;
    }

    md.push('\n');

    if !diff.added.is_empty() {
        md.push_str(&format!("#### ➕ 新增可交互元素 ({})\n", diff.added.len()));
        for el in diff.added.iter().take(20) {
            md.push_str(&format!("- {}\n", fmt_element(el)));
        }
        if diff.added.len() > 20 {
            md.push_str(&format!("- ...（另有 {} 个新增元素已省略）\n", diff.added.len() - 20));
        }
        md.push('\n');
    }

    if !diff.modified.is_empty() {
        md.push_str(&format!("#### ✏️ 元素状态变更 ({})\n", diff.modified.len()));
        for (ref_id, fields) in diff.modified.iter().take(20) {
            let changes: Vec<String> = fields
                .iter()
                .map(|(f, o, n)| format!("{f}: `{o}`→`{n}`"))
                .collect();
            md.push_str(&format!("- `{ref_id}` {}\n", changes.join(", ")));
        }
        if diff.modified.len() > 20 {
            md.push_str(&format!("- ...（另有 {} 个元素变更已省略）\n", diff.modified.len() - 20));
        }
        md.push('\n');
    }

    if !diff.removed.is_empty() {
        let shown: Vec<String> = diff.removed.iter().take(20).map(|r| format!("`{r}`")).collect();
        md.push_str(&format!(
            "#### ➖ 消失元素 ({})\n- {}\n\n",
            diff.removed.len(),
            shown.join(", ")
        ));
    }

    if diff.tables_changed {
        md.push_str("#### 📊 表格数据已变化\n");
        for tbl in snap.tables.iter().take(3) {
            md.push_str(tbl);
            md.push_str("\n\n");
        }
    }

    append_new_errors(&mut md, diff);
    md
}

fn append_new_errors(md: &mut String, diff: &SnapshotDiff) {
    if !diff.overlays_added.is_empty() {
        md.push_str("#### 💥 新增页面报错遮罩\n```text\n");
        for o in &diff.overlays_added {
            md.push_str(o);
            md.push('\n');
        }
        md.push_str("```\n");
    }
    if !diff.notices_added.is_empty() {
        md.push_str("#### 🔔 新增页面提示\n");
        for n in diff.notices_added.iter().take(8) {
            md.push_str(&format!("- {n}\n"));
        }
    }
    // 关键：只回「自上次以来的新增错误」，不再反复回吐陈旧日志
    if !diff.console_new.is_empty() {
        md.push_str("#### 🆕 新增控制台错误\n```\n");
        for e in diff.console_new.iter().take(8) {
            md.push_str(e);
            md.push('\n');
        }
        md.push_str("```\n");
    }
    if !diff.network_new.is_empty() {
        md.push_str("#### 🆕 新增网络异常\n```\n");
        for e in diff.network_new.iter().take(8) {
            md.push_str(e);
            md.push('\n');
        }
        md.push_str("```\n");
    }
}

fn fmt_element(el: &ElementFingerprint) -> String {
    let mut s = format!("`{}` <{}", el.ref_id, el.tag);
    if let Some(t) = &el.r#type {
        s.push_str(&format!(" type=\"{t}\""));
    }
    if let Some(n) = &el.name {
        s.push_str(&format!(" name=\"{n}\""));
    }
    if let Some(p) = &el.placeholder {
        s.push_str(&format!(" placeholder=\"{p}\""));
    }
    s.push('>');
    if let Some(t) = &el.text {
        if !t.is_empty() {
            s.push_str(&format!(" \"{t}\""));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(ref_id: &str, text: &str) -> InteractiveElement {
        InteractiveElement {
            ref_id: ref_id.to_string(),
            tag: "button".to_string(),
            r#type: None,
            name: None,
            placeholder: None,
            text: Some(text.to_string()),
            value: None,
            href: None,
            in_dialog: false,
        }
    }

    fn state_with(elements: Vec<InteractiveElement>) -> BrowserDebugState {
        BrowserDebugState {
            url: "http://a".to_string(),
            title: "T".to_string(),
            viewport: ViewportInfo::default(),
            elements,
            hidden_elements_count: 0,
            console_errors: Vec::new(),
            network_errors: Vec::new(),
            notices: Vec::new(),
            dom_overlays: Vec::new(),
            tables: Vec::new(),
            page_state: Default::default(),
        }
    }

    #[test]
    fn test_diff_detects_added_removed_modified() {
        let prev = PageSnapshot::from_state(&state_with(vec![el("@eaaa", "登录"), el("@ebbb", "取消")]));
        let curr = PageSnapshot::from_state(&state_with(vec![el("@eaaa", "登 录"), el("@eccc", "确定")]));

        let diff = compute_diff(&prev, &curr);
        // @eaaa 文本变化 → modified
        assert_eq!(diff.modified.len(), 1);
        assert_eq!(diff.modified[0].0, "@eaaa");
        // @eccc 新增 → added
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.added[0].ref_id, "@eccc");
        // @ebbb 消失 → removed
        assert_eq!(diff.removed, vec!["@ebbb".to_string()]);
    }

    #[test]
    fn test_diff_identical_is_empty() {
        let s = state_with(vec![el("@eaaa", "登录")]);
        let prev = PageSnapshot::from_state(&s);
        let curr = PageSnapshot::from_state(&s);
        assert!(compute_diff(&prev, &curr).is_empty());
    }

    #[test]
    fn test_url_and_title_change_detected() {
        let mut s2 = state_with(vec![]);
        s2.url = "http://b".to_string();
        s2.title = "T2".to_string();
        let prev = PageSnapshot::from_state(&state_with(vec![]));
        let curr = PageSnapshot::from_state(&s2);
        let diff = compute_diff(&prev, &curr);
        assert!(diff.url_changed.is_some());
        assert!(diff.title_changed.is_some());
    }

    #[test]
    fn test_slice_since_only_new_items() {
        let logs = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        // 游标在 1 → 只返回 b, c
        let (new_items, new_cursor, rolled) = slice_since(&logs, 1);
        assert_eq!(new_items, vec!["b".to_string(), "c".to_string()]);
        assert_eq!(new_cursor, 3);
        assert!(!rolled);
    }

    #[test]
    fn test_slice_since_handles_rolled_buffer() {
        // 游标 10 但缓冲只剩 2 条（发生过截断）→ 全量返回并标记已滚动
        let logs = vec!["x".to_string(), "y".to_string()];
        let (new_items, new_cursor, rolled) = slice_since(&logs, 10);
        assert_eq!(new_items.len(), 2);
        assert_eq!(new_cursor, 2);
        assert!(rolled);
    }

    #[test]
    fn test_tables_hash_change() {
        let mut s2 = state_with(vec![]);
        s2.tables = vec!["| a | b |".to_string()];
        let prev = PageSnapshot::from_state(&state_with(vec![]));
        let curr = PageSnapshot::from_state(&s2);
        assert!(compute_diff(&prev, &curr).tables_changed);
    }
}
