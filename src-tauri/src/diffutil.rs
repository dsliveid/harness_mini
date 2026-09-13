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
    let mut out = String::new();

    for change in d.iter_all_changes() {
        let line = change.value().trim_end_matches('\n');
        let line = line.trim_end_matches('\r');
        let prefix = match change.tag() {
            ChangeTag::Delete => "-",
            ChangeTag::Insert => "+",
            ChangeTag::Equal => " ",
        };
        match change.tag() {
            ChangeTag::Delete => removed += 1,
            ChangeTag::Insert => added += 1,
            ChangeTag::Equal => {}
        }
        use std::fmt::Write;
        let _ = writeln!(out, "{prefix}{line}");
        if out.len() > max_bytes {
            out.push_str("\n[diff 过长，已截断]");
            break;
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
    }

    #[test]
    fn test_diff_truncation() {
        let d = diff_lines("", "x\n".repeat(10000).as_str(), 100);
        assert!(d.text.contains("已截断"));
    }
}
