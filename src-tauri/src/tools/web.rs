use super::types::*;
use serde_json::Value;

/// fetch_web_page：通过 HTTP/HTTPS 请求获取网页内容并清洗为 Markdown 文本
pub(crate) async fn fetch_web_page(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let url = args.get("url").and_then(|v| v.as_str()).ok_or("缺少 url 参数")?.trim();
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("URL 格式无效，必须以 http:// 或 https:// 开头".into());
    }
    let max_chars = args.get("max_chars").and_then(|v| v.as_u64()).unwrap_or(8000) as usize;

    let client = crate::llm::get_client(ctx.proxy_url.as_deref());
    let resp = client
        .get(url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
        .timeout(std::time::Duration::from_secs(25))
        .send()
        .await
        .map_err(|e| format!("网页请求失败: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(format!("网页请求返回错误状态码: {status}"));
    }

    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_lowercase();

    let raw_text = resp.text().await.map_err(|e| format!("读取网页内容失败: {e}"))?;

    let clean = if content_type.contains("html") || raw_text.contains("<html") || raw_text.contains("<!DOCTYPE") {
        html_to_markdown(&raw_text)
    } else {
        raw_text
    };

    let total_len = clean.chars().count();
    let truncated: String = clean.chars().take(max_chars).collect();
    if total_len > max_chars {
        Ok(format!("{truncated}\n\n[内容已截断，共 {total_len} 字符，已展示前 {max_chars} 字符]"))
    } else {
        Ok(truncated)
    }
}

/// 快速清洗 HTML 并转换为结构化 Markdown 文本
pub(crate) fn html_to_markdown(html: &str) -> String {
    use regex::Regex;
    // 1. 移除 script, style, noscript, svg, head
    let re_script = Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap();
    let re_style = Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap();
    let re_noscript = Regex::new(r"(?is)<noscript[^>]*>.*?</noscript>").unwrap();
    let re_svg = Regex::new(r"(?is)<svg[^>]*>.*?</svg>").unwrap();
    let re_head = Regex::new(r"(?is)<head[^>]*>.*?</head>").unwrap();

    let s = re_script.replace_all(html, "");
    let s = re_style.replace_all(&s, "");
    let s = re_noscript.replace_all(&s, "");
    let s = re_svg.replace_all(&s, "");
    let s = re_head.replace_all(&s, "");

    // 2. 标题转换 <h1>..<h6> -> # .. ######
    let re_h = Regex::new(r"(?i)<h([1-6])[^>]*>(.*?)</h[1-6]>").unwrap();
    let s = re_h.replace_all(&s, |caps: &regex::Captures| {
        let level: usize = caps[1].parse().unwrap_or(1);
        let prefix = "#".repeat(level);
        format!("\n\n{prefix} {}\n", &caps[2])
    });

    // 3. 换行与段落
    let re_br = Regex::new(r"(?i)<(br|hr)\s*/?>").unwrap();
    let s = re_br.replace_all(&s, "\n");
    let re_p = Regex::new(r"(?i)</(p|div|li|tr|pre|blockquote)>").unwrap();
    let s = re_p.replace_all(&s, "\n");

    // 4. 链接转换 <a href="...">text</a> -> [text](href)
    let re_a = Regex::new(r#"(?i)<a[^>]+href=["']([^"']+)["'][^>]*>(.*?)</a>"#).unwrap();
    let s = re_a.replace_all(&s, |caps: &regex::Captures| {
        let href = &caps[1];
        let text = &caps[2].trim();
        if text.is_empty() {
            String::new()
        } else {
            format!("[{text}]({href})")
        }
    });

    // 5. 剥离所有剩余标签
    let re_tags = Regex::new(r"<[^>]+>").unwrap();
    let s = re_tags.replace_all(&s, " ");

    // 6. 还原基础 HTML 实体
    let s = s
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&mdash;", "—")
        .replace("&ndash;", "–");

    // 7. 规范化空白行与空格
    let mut out = String::new();
    let mut empty_line_count = 0;
    for line in s.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            empty_line_count += 1;
            if empty_line_count <= 2 {
                out.push('\n');
            }
        } else {
            empty_line_count = 0;
            out.push_str(trimmed);
            out.push('\n');
        }
    }
    out.trim().to_string()
}

