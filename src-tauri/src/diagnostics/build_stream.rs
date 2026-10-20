use super::types::CompileErrorItem;
use std::time::Duration;

#[derive(Debug, Clone)]
pub enum DevServerStatus {
    NotRunning,
    RunningHealthy { status_code: u16, title: Option<String> },
    CompilationFailed { stage: String, error_snippet: String },
}

/// 快速探针检测本地前端 Dev Server（如 127.0.0.1:8080, localhost:5173 等）
pub async fn probe_dev_server(url: &str) -> DevServerStatus {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_millis(1500))
        .danger_accept_invalid_certs(true)
        .build()
    {
        Ok(c) => c,
        Err(_) => return DevServerStatus::NotRunning,
    };

    let resp = match client.get(url).send().await {
        Ok(r) => r,
        Err(_) => return DevServerStatus::NotRunning,
    };

    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();

    // 检查响应内容中是否包含 Webpack / Vite 明显的编译错误页面特征
    if text.contains("Failed to compile") || text.contains("Module build failed") || text.contains("webpack-dev-server-client-overlay") {
        return DevServerStatus::CompilationFailed {
            stage: "webpack".to_string(),
            error_snippet: extract_compile_snippet(&text),
        };
    }

    if text.contains("Internal server error") && text.contains("vite") {
        return DevServerStatus::CompilationFailed {
            stage: "vite".to_string(),
            error_snippet: extract_compile_snippet(&text),
        };
    }

    // 提取简单标题
    let title = extract_title(&text);

    DevServerStatus::RunningHealthy {
        status_code: status,
        title,
    }
}

/// 辅助：从 HTML 中截取编译报错简述
fn extract_compile_snippet(html: &str) -> String {
    let lower = html.to_lowercase();
    if let Some(idx) = lower.find("failed to compile") {
        let snippet = &html[idx..std::cmp::min(idx + 500, html.len())];
        return strip_html_tags(snippet);
    }
    if let Some(idx) = lower.find("module build failed") {
        let snippet = &html[idx..std::cmp::min(idx + 500, html.len())];
        return strip_html_tags(snippet);
    }
    strip_html_tags(&html[..std::cmp::min(300, html.len())])
}

fn strip_html_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        if c == '<' {
            in_tag = true;
        } else if c == '>' {
            in_tag = false;
        } else if !in_tag {
            out.push(c);
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn extract_title(html: &str) -> Option<String> {
    let start = html.find("<title>")?;
    let end = html.find("</title>")?;
    if end > start + 7 {
        Some(html[start + 7..end].trim().to_string())
    } else {
        None
    }
}

pub fn parse_raw_compile_errors(raw_log: &str) -> Vec<CompileErrorItem> {
    let mut errors = Vec::new();
    for line in raw_log.lines() {
        if line.contains("ERROR in") || line.contains("Failed to compile") || line.contains("[vite] Internal server error") {
            errors.push(CompileErrorItem {
                stage: if line.contains("vite") { "vite".to_string() } else { "webpack".to_string() },
                message: line.trim().to_string(),
                file: None,
                line: None,
            });
        }
    }
    errors
}
