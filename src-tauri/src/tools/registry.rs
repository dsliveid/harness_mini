use super::types::{PartialCb, Tool, ToolCtx, ToolScope, ToolSpec, CORE_TOOL_NAMES, TEMP_TOOL_NAMES};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct ToolRegistry {
    dynamic_tools: RwLock<HashMap<String, Arc<dyn Tool>>>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            dynamic_tools: RwLock::new(HashMap::new()),
        }
    }

    pub async fn register(&self, tool: Arc<dyn Tool>) {
        let mut map = self.dynamic_tools.write().await;
        map.insert(tool.name(), tool);
    }

    pub async fn unregister(&self, name: &str) {
        let mut map = self.dynamic_tools.write().await;
        map.remove(name);
    }

    pub async fn list_specs(&self, is_temp: bool, is_subagent: bool) -> Vec<ToolSpec> {
        let mut specs = super::specs::tool_specs();
        // 基于 ToolScope 进行严格作用域过滤：
        // 1. 主会话只下发 Universal 与 MainOnly 工具（不下发 SubOnly 如 deliver_to_parent）
        // 2. 子任务会话只下发 Universal 与 SubOnly 工具（物理屏蔽 MainOnly 如子进程派生、浏览器等）
        // 3. 核心收敛：常规会话仅下发 CORE_TOOL_NAMES 及 deliver_to_parent（临时空间含 temp_*），其余扩展工具统一通过 call_extended_tool 目录按需调用
        specs.retain(|s| {
            let scope_allowed = if is_subagent {
                s.scope == ToolScope::Universal || s.scope == ToolScope::SubOnly
            } else {
                s.scope == ToolScope::Universal || s.scope == ToolScope::MainOnly
            };
            if !scope_allowed {
                return false;
            }
            if !is_temp && TEMP_TOOL_NAMES.contains(&s.name) {
                return false;
            }
            CORE_TOOL_NAMES.contains(&s.name) || s.name == "deliver_to_parent" || (is_temp && TEMP_TOOL_NAMES.contains(&s.name))
        });

        let dyn_tools = self.dynamic_tools.read().await;
        for tool in dyn_tools.values() {
            let name_str: &'static str = Box::leak(tool.name().into_boxed_str());
            let desc_str: &'static str = Box::leak(tool.description().into_boxed_str());
            specs.push(ToolSpec {
                name: name_str,
                description: desc_str,
                schema: tool.schema(),
                risk: tool.risk(),
                scope: ToolScope::Universal,
            });
        }
        specs
    }

    pub async fn execute(
        &self,
        name: &str,
        args: &Value,
        ctx: &ToolCtx,
        on_partial: PartialCb<'_>,
    ) -> Result<String, String> {
        // 1. Check dynamic tools
        {
            let dyn_tools = self.dynamic_tools.read().await;
            if let Some(tool) = dyn_tools.get(name) {
                return tool.execute(args, ctx, Some(on_partial)).await;
            }
        }

        // 2. Static tools dispatch
        super::dispatch_static(name, args, ctx, on_partial).await
    }
}
