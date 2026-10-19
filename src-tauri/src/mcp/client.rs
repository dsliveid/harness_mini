use super::transport::StdioTransport;
use super::types::{CallToolResult, McpContent, McpServerConfig, McpToolDefinition};
use crate::tools::{PartialCb, Risk, Tool, ToolCtx};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

pub struct McpClient {
    pub server_id: String,
    pub server_name: String,
    transport: StdioTransport,
}

impl McpClient {
    pub async fn spawn(config: &McpServerConfig) -> Result<Self, String> {
        let transport = StdioTransport::spawn(&config.command, &config.args, &config.env).await?;
        let client = Self {
            server_id: config.id.clone(),
            server_name: config.name.clone(),
            transport,
        };

        // 执行握手
        client.initialize().await?;
        Ok(client)
    }

    pub async fn initialize(&self) -> Result<Value, String> {
        let init_params = json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {}
            },
            "clientInfo": {
                "name": "harness_mini",
                "version": "0.1.0"
            }
        });

        let res = self
            .transport
            .send_request("initialize", Some(init_params), Duration::from_secs(20))
            .await?;

        // 发送 initialized 通知
        let _ = self
            .transport
            .send_notification("notifications/initialized", None)
            .await;

        Ok(res)
    }

    pub async fn list_tools(&self) -> Result<Vec<McpToolDefinition>, String> {
        let res = self
            .transport
            .send_request("tools/list", None, Duration::from_secs(20))
            .await?;

        let tools_val = res
            .get("tools")
            .cloned()
            .unwrap_or_else(|| json!([]));

        serde_json::from_value(tools_val).map_err(|e| format!("解析 tools/list 失败: {e}"))
    }

    pub async fn call_tool(&self, name: &str, arguments: &Value) -> Result<String, String> {
        let params = json!({
            "name": name,
            "arguments": arguments
        });

        let res = self
            .transport
            .send_request("tools/call", Some(params), Duration::from_secs(120))
            .await?;

        let call_res: CallToolResult =
            serde_json::from_value(res).map_err(|e| format!("解析 tools/call 响应失败: {e}"))?;

        let mut output = String::new();
        for content in call_res.content {
            match content {
                McpContent::Text { text } => {
                    if !output.is_empty() {
                        output.push('\n');
                    }
                    output.push_str(&text);
                }
                McpContent::Image { mime_type, .. } => {
                    let mime = mime_type.unwrap_or_else(|| "image/png".to_string());
                    output.push_str(&format!("\n[MCP 图像数据 ({mime})]\n"));
                }
                _ => {}
            }
        }

        if call_res.is_error.unwrap_or(false) {
            Err(if output.is_empty() {
                "MCP 工具返回未知执行错误".to_string()
            } else {
                output
            })
        } else {
            Ok(output)
        }
    }

    pub async fn terminate(&self) {
        self.transport.terminate().await;
    }
}

/// 适配 crate::tools::Tool 的 MCP 工具包装器
pub struct McpToolWrapper {
    qualified_name: String,
    raw_tool_name: String,
    description: String,
    schema: Value,
    client: Arc<McpClient>,
}

impl McpToolWrapper {
    pub fn new(server_name: &str, def: McpToolDefinition, client: Arc<McpClient>) -> Self {
        let qualified_name = format!("mcp__{}__{}", sanitize_name(server_name), def.name);
        let desc = format!(
            "[MCP 服务: {}] {}",
            server_name,
            def.description.unwrap_or_default()
        );
        Self {
            qualified_name,
            raw_tool_name: def.name,
            description: desc,
            schema: def.input_schema,
            client,
        }
    }
}

fn sanitize_name(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
        .collect()
}

#[async_trait::async_trait]
impl Tool for McpToolWrapper {
    fn name(&self) -> String {
        self.qualified_name.clone()
    }

    fn description(&self) -> String {
        self.description.clone()
    }

    fn schema(&self) -> Value {
        self.schema.clone()
    }

    fn risk(&self) -> Risk {
        Risk::Execute
    }

    async fn execute(
        &self,
        args: &Value,
        _ctx: &ToolCtx,
        _on_partial: Option<PartialCb<'_>>,
    ) -> Result<String, String> {
        self.client.call_tool(&self.raw_tool_name, args).await
    }
}
