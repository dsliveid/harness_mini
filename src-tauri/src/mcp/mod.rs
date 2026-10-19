pub mod client;
pub mod transport;
pub mod types;

pub use client::{McpClient, McpToolWrapper};
pub use types::{McpContent, McpServerConfig, McpToolDefinition};

use crate::tools::global_registry;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tokio::sync::RwLock;

pub struct McpManager {
    clients: RwLock<HashMap<String, Arc<McpClient>>>,
}

impl Default for McpManager {
    fn default() -> Self {
        Self::new()
    }
}

static GLOBAL_MCP_MANAGER: OnceLock<McpManager> = OnceLock::new();

pub fn global_mcp_manager() -> &'static McpManager {
    GLOBAL_MCP_MANAGER.get_or_init(McpManager::new)
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            clients: RwLock::new(HashMap::new()),
        }
    }

    /// 启动指定的 MCP Server 并注册其工具至 ToolRegistry
    pub async fn start_server(&self, config: &McpServerConfig) -> Result<Vec<McpToolDefinition>, String> {
        // 先停掉旧实例（如果已存在）
        self.stop_server(&config.id).await;

        if !config.enabled {
            return Ok(vec![]);
        }

        let client = Arc::new(McpClient::spawn(config).await?);
        let tools = client.list_tools().await?;

        let registry = global_registry();
        for tool_def in &tools {
            let wrapper = McpToolWrapper::new(&config.name, tool_def.clone(), client.clone());
            registry.register(Arc::new(wrapper)).await;
        }

        let mut map = self.clients.write().await;
        map.insert(config.id.clone(), client);

        Ok(tools)
    }

    /// 停止指定的 MCP Server 并注销其注册的所有工具
    pub async fn stop_server(&self, server_id: &str) {
        let mut map = self.clients.write().await;
        if let Some(client) = map.remove(server_id) {
            client.terminate().await;
            // 从 registry 注销对应的工具
            if let Ok(tools) = client.list_tools().await {
                let registry = global_registry();
                for t in tools {
                    let qualified = format!("mcp__{}__{}", client.server_name, t.name);
                    registry.unregister(&qualified).await;
                }
            }
        }
    }

    /// 测试连接指定命令，并返回其暴露的工具清单（用于配置面板的联通性测试）
    pub async fn test_connection(
        &self,
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<Vec<McpToolDefinition>, String> {
        let temp_cfg = McpServerConfig {
            id: "temp_test".to_string(),
            name: "test".to_string(),
            command: command.to_string(),
            args: args.to_vec(),
            env: env.clone(),
            enabled: true,
            created_at: String::new(),
            updated_at: String::new(),
        };

        let client = McpClient::spawn(&temp_cfg).await?;
        let tools = client.list_tools().await?;
        client.terminate().await;
        Ok(tools)
    }

    /// 从数据库加载所有启用的 MCP Server 并初始化启动
    pub async fn reload_from_db(&self, configs: Vec<McpServerConfig>) {
        for cfg in configs {
            if cfg.enabled {
                if let Err(e) = self.start_server(&cfg).await {
                    eprintln!("启动 MCP 服务失败 [{}]: {e}", cfg.name);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::types::*;
    use serde_json::json;

    #[test]
    fn test_jsonrpc_request_serialization() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: json!(42),
            method: "tools/call".to_string(),
            params: Some(json!({
                "name": "eval",
                "arguments": {"code": "1 + 1"}
            })),
        };
        let s = serde_json::to_string(&req).unwrap();
        assert!(s.contains("\"jsonrpc\":\"2.0\""));
        assert!(s.contains("\"method\":\"tools/call\""));
        assert!(s.contains("\"id\":42"));
    }

    #[test]
    fn test_mcp_tool_definition_parsing() {
        let data = json!({
            "name": "browser_click",
            "description": "Click an element",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": {"type": "string"}
                },
                "required": ["selector"]
            }
        });
        let tool: McpToolDefinition = serde_json::from_value(data).unwrap();
        assert_eq!(tool.name, "browser_click");
        assert_eq!(tool.description.as_deref(), Some("Click an element"));
    }
}
