//! 模型请求协议抽象与分发网关
//!
//! 支持四类协议：
//! - ChatCompletions: OpenAI 标准接口 (/chat/completions)
//! - Messages: Anthropic Claude 原生接口 (/v1/messages)
//! - Response: OpenAI Responses 新版统一接口 (/v1/responses)
//! - SystemOne: TypeSafe Jev 专用极速决策门控接口 (/v1/systemone)

pub mod anthropic;
pub mod openai_chat;
pub mod openai_responses;
pub mod transpiler;

#[cfg(test)]
pub mod tests;

use crate::llm::{LlmCfg, LlmResult};
use crate::models::RequestProtocol;
use serde_json::Value;

/// 统一流式入口：根据 cfg.protocol 自动路由至对应协议适配器
pub async fn dispatch_chat_stream(
    cfg: &LlmCfg,
    messages: &[Value],
    tools: &[Value],
    on_text: impl FnMut(&str) + Send,
    on_reasoning: impl FnMut(&str) + Send,
) -> Result<LlmResult, String> {
    match cfg.protocol {
        RequestProtocol::ChatCompletions => {
            openai_chat::chat_stream(cfg, messages, tools, on_text, on_reasoning).await
        }
        RequestProtocol::Messages => {
            anthropic::chat_stream(cfg, messages, tools, on_text, on_reasoning).await
        }
        RequestProtocol::Response => {
            openai_responses::chat_stream(cfg, messages, tools, on_text, on_reasoning).await
        }
        RequestProtocol::SystemOne => Err(
            "SystemOne 协议为极速结构化决策门控协议（POST /v1/systemone），不支持通用对话补全。\n\
            请在设置中为主对话选择 OpenAI Chat、Claude Messages 或 OpenAI Response 协议，将 SystemOne 分配为决策模型。"
                .into(),
        ),
    }
}
