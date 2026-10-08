//! 协议转译与端点自适应单元测试

use super::transpiler::*;
use crate::models::RequestProtocol;
use serde_json::json;

#[test]
fn test_protocol_inference() {
    assert_eq!(
        RequestProtocol::infer("https://api.openai.com/v1", "gpt-4o"),
        RequestProtocol::ChatCompletions
    );
    assert_eq!(
        RequestProtocol::infer("https://api.anthropic.com", "claude-3-5-sonnet"),
        RequestProtocol::Messages
    );
    assert_eq!(
        RequestProtocol::infer("https://api.typesafe.ai", "jev-1.13-free"),
        RequestProtocol::SystemOne
    );
    assert_eq!(
        RequestProtocol::infer("https://opencode.ai/zen/v1/systemone", "custom-model"),
        RequestProtocol::SystemOne
    );
    assert_eq!(
        RequestProtocol::infer("https://api.openai.com/v1/responses", "gpt-4o"),
        RequestProtocol::Response
    );
}

#[test]
fn test_serde_systemone_compatibility() {
    let de_direct: RequestProtocol = serde_json::from_str("\"systemone\"").unwrap();
    assert_eq!(de_direct, RequestProtocol::SystemOne);

    let de_alias: RequestProtocol = serde_json::from_str("\"system_one\"").unwrap();
    assert_eq!(de_alias, RequestProtocol::SystemOne);

    let ser = serde_json::to_string(&RequestProtocol::SystemOne).unwrap();
    assert_eq!(ser, "\"systemone\"");
}

#[test]
fn test_anthropic_transpile_basic_and_system_extraction() {
    let messages = vec![
        json!({"role": "system", "content": "You are an assistant."}),
        json!({"role": "user", "content": "Hello!"}),
        json!({"role": "assistant", "content": "Hi there!"}),
    ];
    let payload = transpile_to_anthropic(&messages, &[]);

    assert_eq!(payload.system, Some("You are an assistant.".to_string()));
    assert_eq!(payload.messages.len(), 2);
    assert_eq!(payload.messages[0]["role"], "user");
    assert_eq!(payload.messages[0]["content"][0]["text"], "Hello!");
    assert_eq!(payload.messages[1]["role"], "assistant");
    assert_eq!(payload.messages[1]["content"][0]["text"], "Hi there!");
}

#[test]
fn test_anthropic_transpile_tool_calls_and_results_squashing() {
    let messages = vec![
        json!({"role": "system", "content": "System instruction"}),
        json!({"role": "user", "content": "Read file foo.rs"}),
        json!({
            "role": "assistant",
            "content": "Reading...",
            "tool_calls": [
                {
                    "id": "call_123",
                    "type": "function",
                    "function": {
                        "name": "read_file",
                        "arguments": "{\"path\":\"foo.rs\"}"
                    }
                }
            ]
        }),
        // 工具执行结果在通用格式中为 role: "tool"
        json!({
            "role": "tool",
            "tool_call_id": "call_123",
            "content": "pub fn hello() {}"
        }),
    ];

    let tools = vec![json!({
        "type": "function",
        "function": {
            "name": "read_file",
            "description": "Read file contents",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": {"type": "string"}
                }
            }
        }
    })];

    let payload = transpile_to_anthropic(&messages, &tools);

    // 验证工具定义转译
    assert_eq!(payload.tools.len(), 1);
    assert_eq!(payload.tools[0]["name"], "read_file");
    assert_eq!(payload.tools[0]["input_schema"]["type"], "object");

    // 验证工具调用转译
    assert_eq!(payload.messages.len(), 3);
    assert_eq!(payload.messages[0]["role"], "user");
    assert_eq!(payload.messages[1]["role"], "assistant");
    assert_eq!(payload.messages[1]["content"][0]["type"], "text");
    assert_eq!(payload.messages[1]["content"][1]["type"], "tool_use");
    assert_eq!(payload.messages[1]["content"][1]["id"], "call_123");

    // 验证工具结果转译（必须作为 user 消息的 tool_result block）
    assert_eq!(payload.messages[2]["role"], "user");
    assert_eq!(payload.messages[2]["content"][0]["type"], "tool_result");
    assert_eq!(payload.messages[2]["content"][0]["tool_use_id"], "call_123");
    assert_eq!(payload.messages[2]["content"][0]["content"], "pub fn hello() {}");
}

#[test]
fn test_anthropic_orphan_tool_call_self_healing() {
    // 场景：Assistant 发起了 tool_calls，但用户点击了停止或未收到结果，紧接着发了新消息或切换模型
    let messages = vec![
        json!({"role": "user", "content": "Run command"}),
        json!({
            "role": "assistant",
            "content": "Running command...",
            "tool_calls": [
                {
                    "id": "call_orphan_999",
                    "type": "function",
                    "function": {
                        "name": "run_command",
                        "arguments": "{\"cmd\":\"ls\"}"
                    }
                }
            ]
        }),
        // 没有 tool 消息，用户直接发送了新消息
        json!({"role": "user", "content": "Never mind, do something else"}),
    ];

    let payload = transpile_to_anthropic(&messages, &[]);

    // 验证自愈机制：在紧随其后的 user 消息中自动注入了 missing tool_result
    assert_eq!(payload.messages.len(), 3);
    let next_user_blocks = payload.messages[2]["content"].as_array().unwrap();
    let has_healed_tool_result = next_user_blocks.iter().any(|b| {
        b["type"] == "tool_result" && b["tool_use_id"] == "call_orphan_999"
    });
    assert!(has_healed_tool_result, "未匹配的孤立 tool_use 必须被自动注入虚拟结果以满足 Claude 校验");
}

#[test]
fn test_anthropic_multimodal_image_url_transpile() {
    let data_url = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
    let messages = vec![
        json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "What is in this image?"},
                {"type": "image_url", "image_url": {"url": data_url}}
            ]
        })
    ];

    let payload = transpile_to_anthropic(&messages, &[]);
    assert_eq!(payload.messages.len(), 1);
    let blocks = payload.messages[0]["content"].as_array().unwrap();
    assert_eq!(blocks[0]["type"], "text");
    assert_eq!(blocks[1]["type"], "image");
    assert_eq!(blocks[1]["source"]["type"], "base64");
    assert_eq!(blocks[1]["source"]["media_type"], "image/png");
}

#[test]
fn test_responses_transpile() {
    let messages = vec![
        json!({"role": "system", "content": "Instructions here"}),
        json!({"role": "user", "content": "Hello"}),
        json!({
            "role": "assistant",
            "content": "Working",
            "tool_calls": [{
                "id": "c1",
                "function": {
                    "name": "test_fn",
                    "arguments": "{}"
                }
            }]
        }),
        json!({
            "role": "tool",
            "tool_call_id": "c1",
            "content": "result data"
        }),
    ];

    let payload = transpile_to_responses(&messages, &[]);
    assert_eq!(payload.instructions, Some("Instructions here".to_string()));
    assert_eq!(payload.input.len(), 4);
    assert_eq!(payload.input[0]["type"], "message");
    assert_eq!(payload.input[0]["role"], "user");
    assert_eq!(payload.input[1]["type"], "message");
    assert_eq!(payload.input[1]["role"], "assistant");
    assert_eq!(payload.input[2]["type"], "function_call");
    assert_eq!(payload.input[2]["call_id"], "c1");
    assert_eq!(payload.input[3]["type"], "function_call_output");
    assert_eq!(payload.input[3]["call_id"], "c1");
}

#[test]
fn test_llm_cfg_effective_session_id() {
    let cfg1 = crate::llm::LlmCfg {
        base_url: "https://opencode.ai/zen/go/v1".into(),
        api_key: "sk-test".into(),
        model: "claude-haiku-5-5".into(),
        protocol: RequestProtocol::Messages,
        reasoning_effort: None,
        proxy_url: None,
        session_id: Some("session-custom-123".into()),
    };
    assert_eq!(cfg1.effective_session_id(), "session-custom-123");

    let cfg2 = crate::llm::LlmCfg {
        base_url: "https://opencode.ai/zen/go/v1".into(),
        api_key: "sk-test".into(),
        model: "claude-haiku-5-5".into(),
        protocol: RequestProtocol::Messages,
        reasoning_effort: None,
        proxy_url: None,
        session_id: None,
    };
    let s2 = cfg2.effective_session_id();
    assert!(s2.starts_with("sess-"), "Auto-generated session id should start with sess-");
    assert!(s2.len() > 10);

    let cfg3 = crate::llm::LlmCfg {
        session_id: Some("   ".into()),
        ..cfg2
    };
    let s3 = cfg3.effective_session_id();
    assert!(s3.starts_with("sess-"), "Whitespace session id should fallback to auto-generated session id");
}

