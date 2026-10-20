use serde::{Deserialize, Serialize};

/// 源代码位置坐标（堆栈帧）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceLocation {
    pub file: String,
    pub line: u32,
    pub column: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_name: Option<String>,
}

/// 结构化 JavaScript 运行时异常
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsRuntimeError {
    pub name: String,                    // e.g. "TypeError", "ReferenceError"
    pub message: String,                 // e.g. "Cannot read properties of undefined (reading 'xxx')"
    pub stack_trace: Vec<SourceLocation>,// 提取后的结构化堆栈
    pub raw_description: String,        // 原始调用堆栈描述
    pub timestamp: u64,                  // 毫秒时间戳
}

/// 结构化网络请求失败信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkErrorItem {
    pub url: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,             // e.g. 404, 500
    pub failure_reason: String,          // e.g. "net::ERR_CONNECTION_REFUSED", "HTTP 500"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_body_snippet: Option<String>,
}

/// 本地编译或打包构建错误（Webpack / Vite / ESLint / TSC）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompileErrorItem {
    pub stage: String,                  // "webpack" | "vite" | "eslint" | "tsc"
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

/// 前端多维诊断汇总报告
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DiagnosticReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compile_status: Option<Result<(), Vec<CompileErrorItem>>>,
    pub runtime_errors: Vec<JsRuntimeError>,
    pub network_errors: Vec<NetworkErrorItem>,
    pub dom_overlays: Vec<String>,       // 检测到的全屏报错遮罩文本 (Webpack / Vite Overlay)
    pub page_health: String,             // "Healthy" | "Degraded" | "Broken"
}

impl DiagnosticReport {
    pub fn has_errors(&self) -> bool {
        !self.runtime_errors.is_empty()
            || !self.network_errors.is_empty()
            || !self.dom_overlays.is_empty()
            || matches!(self.compile_status, Some(Err(ref errs)) if !errs.is_empty())
    }

    /// 转换为专供 LLM 消费的 Markdown 诊断报告
    pub fn to_markdown(&self) -> String {
        if !self.has_errors() {
            return String::new();
        }

        let mut md = String::new();
        md.push_str("### 🚨 前端异常多维诊断报告 (Diagnostics Report)\n\n");

        // 1. 致命级：全屏错误遮罩 (Webpack / Vite Overlay)
        if !self.dom_overlays.is_empty() {
            md.push_str("#### 💥 页面报错遮罩/编译弹层 (Error Overlays):\n```text\n");
            for overlay in &self.dom_overlays {
                md.push_str(overlay.trim());
                md.push('\n');
            }
            md.push_str("```\n\n");
        }

        // 2. 运行时未捕获 JavaScript 异常与调用堆栈
        if !self.runtime_errors.is_empty() {
            md.push_str("#### 🛑 JavaScript 运行时未捕获异常 (Runtime Exceptions):\n");
            for err in &self.runtime_errors {
                md.push_str(&format!("- **`{}`**: {}\n", err.name, err.message));
                if !err.stack_trace.is_empty() {
                    md.push_str("  **调用堆栈**:\n");
                    for frame in err.stack_trace.iter().take(5) {
                        let fn_name = frame.function_name.as_deref().unwrap_or("<anonymous>");
                        md.push_str(&format!("  - at `{fn_name}` (`{}:{}:{}`)\n", frame.file, frame.line, frame.column));
                    }
                } else if !err.raw_description.is_empty() {
                    md.push_str("```text\n");
                    for line in err.raw_description.lines().take(6) {
                        md.push_str("  ");
                        md.push_str(line);
                        md.push('\n');
                    }
                    md.push_str("```\n");
                }
            }
            md.push('\n');
        }

        // 3. 网络请求异常
        if !self.network_errors.is_empty() {
            md.push_str("#### 🌐 接口与网络请求失败 (Failed Network Requests):\n```text\n");
            for net in self.network_errors.iter().take(6) {
                if let Some(status) = net.status {
                    md.push_str(&format!("[HTTP {status}] {} {}\n", net.method, net.url));
                } else {
                    md.push_str(&format!("[Failed] {} {} -> {}\n", net.method, net.url, net.failure_reason));
                }
                if let Some(ref snippet) = net.response_body_snippet {
                    md.push_str(&format!("         响应内容: {snippet}\n"));
                }
            }
            md.push_str("```\n\n");
        }

        // 4. 构建编译级错误
        if let Some(Err(ref comp_errs)) = self.compile_status {
            if !comp_errs.is_empty() {
                md.push_str("#### 🛠️ 开发构建与编译失败 (Build Errors):\n");
                for c in comp_errs {
                    let loc = match (&c.file, c.line) {
                        (Some(f), Some(l)) => format!(" ({f}:{l})"),
                        (Some(f), None) => format!(" ({f})"),
                        _ => String::new(),
                    };
                    md.push_str(&format!("- **[{}]** {}{}\n", c.stage, c.message, loc));
                }
                md.push('\n');
            }
        }

        md
    }
}
