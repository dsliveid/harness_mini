use similar::{ChangeTag, TextDiff};

/// 生成统一 diff 文本与增删行数统计
pub struct DiffOut {
    pub text: String,
    pub added: usize,
    pub removed: usize,
}

pub fn diff_lines(old: &str, new: &str, max_bytes: usize) -> DiffOut {
    let d = TextDiff::from_lines(old, new);
    let mut added = 0usize;
    let mut removed = 0usize;

    for change in d.iter_all_changes() {
        match change.tag() {
            ChangeTag::Delete => removed += 1,
            ChangeTag::Insert => added += 1,
            ChangeTag::Equal => {}
        }
    }

    let mut out = String::new();
    for hunk in d.unified_diff().context_radius(3).iter_hunks() {
        use std::fmt::Write;
        let _ = write!(out, "{hunk}");
        if out.len() > max_bytes {
            let mut end = max_bytes;
            while end > 0 && !out.is_char_boundary(end) {
                end -= 1;
            }
            out.truncate(end);
            out.push_str("\n[diff 过长，已截断]");
            break;
        }
    }

    if out.is_empty() && (added > 0 || removed > 0) {
        // 兜底：如果 hunk 迭代为空但统计有差异，直接调用统一格式化
        let full_udiff = d.unified_diff().context_radius(3).to_string();
        if full_udiff.len() > max_bytes {
            let mut end = max_bytes;
            while end > 0 && !full_udiff.is_char_boundary(end) {
                end -= 1;
            }
            out = format!("{}\n[diff 过长，已截断]", &full_udiff[..end]);
        } else {
            out = full_udiff;
        }
    }

    DiffOut { text: out, added, removed }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diff_basic() {
        let d = diff_lines("a\nb\nc\n", "a\nX\nc\n", 65536);
        assert_eq!(d.added, 1);
        assert_eq!(d.removed, 1);
        assert!(d.text.contains("-b"));
        assert!(d.text.contains("+X"));
        assert!(d.text.contains("@@"));
    }

    #[test]
    fn test_diff_truncation() {
        let d = diff_lines("", "x\n".repeat(10000).as_str(), 100);
        assert!(d.text.contains("已截断"));
    }

    #[test]
    fn test_diff_large_file_focus() {
        // 模拟 1000 行文件，只改动第 900 行
        let mut lines_old = Vec::new();
        let mut lines_new = Vec::new();
        for i in 1..=1000 {
            if i == 900 {
                lines_old.push("old_line_content");
                lines_new.push("new_line_content");
            } else {
                lines_old.push("constant_padding_line");
                lines_new.push("constant_padding_line");
            }
        }
        let old_text = lines_old.join("\n");
        let new_text = lines_new.join("\n");

        let d = diff_lines(&old_text, &new_text, 4096);
        assert_eq!(d.added, 1);
        assert_eq!(d.removed, 1);
        assert!(d.text.contains("-old_line_content"));
        assert!(d.text.contains("+new_line_content"));
        // 且不会因为包含了前面 800 行未修改内容而导致被截断！
        assert!(!d.text.contains("已截断"));
    }
}
