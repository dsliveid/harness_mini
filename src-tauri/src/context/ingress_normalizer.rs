/// 工具输出入库前轻量化定型器 (Ingress Normalizer)
/// 
/// 核心准则：工具结果一旦进入数据库与消息流，终生不可变（Immutable）。
/// 若工具输出过于庞大（如 run_command 输出数万字符），在初次写入时就应进行结构化精炼与前后保留，
/// 严禁后续轮次中通过 `is_historical_turn` 进行事后重写篡改（那会导致下游 KV Cache 全盘报废）。

pub fn normalize_tool_output_for_ingress(tool_name: &str, raw_output: &str) -> String {
    match tool_name {
        "run_command" => {
            // 对命令输出进行入库保护：如果超过 6000 字符，保留头部 3500 字符与尾部 1500 字符，中间折叠
            if raw_output.len() > 6000 {
                let head_safe = raw_output
                    .char_indices()
                    .nth(3500)
                    .map(|(i, _)| i)
                    .unwrap_or(3500.min(raw_output.len()));
                let total_chars = raw_output.chars().count();
                let tail_start_char = total_chars.saturating_sub(1500);
                let tail_safe = raw_output
                    .char_indices()
                    .nth(tail_start_char)
                    .map(|(i, _)| i)
                    .unwrap_or(raw_output.len().saturating_sub(1500));

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
        "browser_interact" | "browser_open" => {
            // 浏览器页面渲染输出若过长 (> 12000 字符)，收敛保留前 8000 字符
            if raw_output.len() > 12000 {
                let safe_len = raw_output
                    .char_indices()
                    .nth(8000)
                    .map(|(i, _)| i)
                    .unwrap_or(8000.min(raw_output.len()));
                format!(
                    "{}\n\n...[浏览器渲染页面元素较多，已入库定型截断前 8000 字符]...",
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
}
