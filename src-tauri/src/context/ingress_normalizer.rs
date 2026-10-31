/// 工具输出入库前轻量化定型器 (Ingress Normalizer)
/// 
/// 核心准则：工具结果一旦进入数据库与消息流，终生不可变（Immutable）。
/// 针对重量级工具（run_command, grep, read_file, list_dir, glob, browser），
/// 在初次写入 messages 时即进行结构化收敛与首尾保留，防止巨型控制台日志或大文件全文
/// 永久污染历史上下文并引发梯形积分爆炸。

pub fn normalize_tool_output_for_ingress(tool_name: &str, raw_output: &str) -> String {
    match tool_name {
        "run_command" => {
            // 对命令输出进行入库保护：如果超过 4000 字符，保留头部 2000 字符与尾部 1200 字符，中间折叠
            if raw_output.len() > 4000 {
                let head_safe = raw_output
                    .char_indices()
                    .nth(2000)
                    .map(|(i, _)| i)
                    .unwrap_or(2000.min(raw_output.len()));
                let total_chars = raw_output.chars().count();
                let tail_start_char = total_chars.saturating_sub(1200);
                let tail_safe = raw_output
                    .char_indices()
                    .nth(tail_start_char)
                    .map(|(i, _)| i)
                    .unwrap_or(raw_output.len().saturating_sub(1200));

                format!(
                    "{}\n\n...[执行输出过长，已在入库时安全收敛（保留首尾核心，原始共 {} 字符）]...\n\n{}",
                    &raw_output[..head_safe],
                    raw_output.len(),
                    &raw_output[tail_safe..]
                )
            } else {
                raw_output.to_string()
            }
        }
        "grep" => {
            // 对搜索结果进行入库保护：行数超过 40 行或总字符超过 3500 时收敛
            let lines: Vec<&str> = raw_output.lines().collect();
            if lines.len() > 40 || raw_output.len() > 3500 {
                let kept = lines.iter().take(30).copied().collect::<Vec<_>>().join("\n");
                format!(
                    "{}\n\n...[grep 搜索命中项较多（共 {} 行，{} 字符），已收敛保留前 30 行核心命中]...",
                    kept,
                    lines.len(),
                    raw_output.len()
                )
            } else {
                raw_output.to_string()
            }
        }
        "read_file" => {
            // 读取文件若过大 (> 8000 字符)，收敛保留前 4500 字符与尾部 1800 字符
            if raw_output.len() > 8000 {
                let head_safe = raw_output
                    .char_indices()
                    .nth(4500)
                    .map(|(i, _)| i)
                    .unwrap_or(4500.min(raw_output.len()));
                let total_chars = raw_output.chars().count();
                let tail_start_char = total_chars.saturating_sub(1800);
                let tail_safe = raw_output
                    .char_indices()
                    .nth(tail_start_char)
                    .map(|(i, _)| i)
                    .unwrap_or(raw_output.len().saturating_sub(1800));

                format!(
                    "{}\n\n...[文件内容过长（共 {} 字符），已入库收敛首尾关键段落。需阅读局部请指定 start_line/end_line]...\n\n{}",
                    &raw_output[..head_safe],
                    raw_output.len(),
                    &raw_output[tail_safe..]
                )
            } else {
                raw_output.to_string()
            }
        }
        "list_dir" | "glob" => {
            // 目录扫描或通配符匹配若条目过多 (> 50 行或 > 3500 字符)，保留前 40 项
            let lines: Vec<&str> = raw_output.lines().collect();
            if lines.len() > 50 || raw_output.len() > 3500 {
                let kept = lines.iter().take(40).copied().collect::<Vec<_>>().join("\n");
                format!(
                    "{}\n\n...[文件清单项数较多（共 {} 项，{} 字符），已收敛保留前 40 项]...",
                    kept,
                    lines.len(),
                    raw_output.len()
                )
            } else {
                raw_output.to_string()
            }
        }
        "browser_interact" | "browser_open" => {
            // 浏览器页面渲染输出若过长 (> 8000 字符)，收敛保留前 5000 字符
            if raw_output.len() > 8000 {
                let safe_len = raw_output
                    .char_indices()
                    .nth(5000)
                    .map(|(i, _)| i)
                    .unwrap_or(5000.min(raw_output.len()));
                format!(
                    "{}\n\n...[浏览器渲染页面元素较多，已入库定型截断前 5000 字符]...",
                    &raw_output[..safe_len]
                )
            } else {
                raw_output.to_string()
            }
        }
        _ => raw_output.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalizer_run_command_short() {
        let short = "total 0\n-rw-r--r-- 1 user 0 test.txt";
        assert_eq!(normalize_tool_output_for_ingress("run_command", short), short);
    }

    #[test]
    fn test_normalizer_run_command_long() {
        let long = "A".repeat(8000);
        let res = normalize_tool_output_for_ingress("run_command", &long);
        assert!(res.contains("已在入库时安全收敛"));
        assert!(res.len() < 8000);
    }

    #[test]
    fn test_normalizer_grep_long() {
        let lines: Vec<String> = (0..80).map(|i| format!("file.rs:{}: match line {}", i, i)).collect();
        let raw = lines.join("\n");
        let res = normalize_tool_output_for_ingress("grep", &raw);
        assert!(res.contains("grep 搜索命中项较多"));
        assert!(res.lines().count() <= 35);
    }

    #[test]
    fn test_normalizer_read_file_long() {
        let long = "fn hello() { println!(\"world\"); }\n".repeat(300); // > 10,000 chars
        let res = normalize_tool_output_for_ingress("read_file", &long);
        assert!(res.contains("文件内容过长"));
        assert!(res.len() < long.len());
    }

    #[test]
    fn test_normalizer_list_dir_long() {
        let lines: Vec<String> = (0..100).map(|i| format!("item_{}.ts", i)).collect();
        let raw = lines.join("\n");
        let res = normalize_tool_output_for_ingress("list_dir", &raw);
        assert!(res.contains("文件清单项数较多"));
        assert!(res.lines().count() <= 45);
    }
}
