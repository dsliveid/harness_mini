# AI Agent 深度思考模式协议自愈、Reasoning Content 契约与 JIT 即时转译管道知识文档
(LLM_THINKING_MODE_REASONING_CONTENT_AND_JIT_PIPELINE_KNOWLEDGE)

| 属性 | 说明 |
| :--- | :--- |
| **文档代号** | `LLM_THINKING_MODE_REASONING_CONTENT_AND_JIT_PIPELINE_KNOWLEDGE` |
| **所属模块** | [`src-tauri/src/protocol/transpiler.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/protocol/transpiler.rs), [`src-tauri/src/protocol/openai_chat.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/protocol/openai_chat.rs), [`src-tauri/src/protocol/tests.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/protocol/tests.rs), [`src-tauri/src/protocol/mod.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/protocol/mod.rs), [`src-tauri/src/agent.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/agent.rs), [`src-tauri/src/llm.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/llm.rs) |
| **技术体系** | DeepSeek Thinking Protocol · OpenAI Chat Completions · JIT Transpiler · Zero-Distortion Protocol Adaptation · Rust · Multi-Turn Agent Loop |
| **创建时间** | 2026-10-21 |
| **核心机制** | Thinking 模式推理流保底自愈 · 非思考模式私有属性自动脱敏 · 孤立 Tool Call 保序闭合 · 协议管道下沉与核心领域层零侵入 |

---

## 一、 现场故障复盘与根因透视

### 1.1 现场故障现象与定位
在桌面级 AI Agent（`harness_mini`）执行复杂多步任务（如“检查医疗纠纷模块”）时，Agent 在前 4 轮工具调用顺利推进，却在第 5 轮发起下一阶段大模型推理时突然中断崩溃，界面抛出致命系统异常：

```text
⚠️ LLM 返回 400 Bad Request: {
  "error": {
    "param": null,
    "type": "invalid_request_error",
    "code": "invalid_request_error",
    "message": "Upstream request failed: [invalid_request_error] The `reasoning_content` in the thinking mode must be passed back to the API. (request_id: 58906076-9108-4ca4-a87a-bcf5237ab3fd)"
  }
}
```

- **目标会话**：`1df86a8b-baf3-4e5a-9a11-f8dfda2c8d7b`（“检查医疗纠纷模块：1、新增、修改、审核后，外部...”）；
- **激活服务商与模型**：`opencode` 网关（`https://opencode.ai/zen/go/v1`），模型 `deepseek-v4.1-flash`，开启思考模式（`reasoning_effort: "high"`）；
- **故障爆发轮次**：执行链第 5 步（`Request 5`，消息序号 Seq 14）。

---

### 1.2 全链路消息与 Token 审计追踪

通过底层 SQLite 审计记录（`messages` 与 `llm_requests` 表）回溯，整条执行链的推理与工具调用时序如下：

| 消息序号 | 角色 Role | 工具调用 / 内容摘要 | Reasoning 思考内容 | `rawUsage` 中的 `reasoning_tokens` | 后续回传状态 |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **Seq 2** | assistant | 调用 `glob` | 有（66 字符） | > 0 | `reasoning_content` 正常回传 |
| **Seq 5** | assistant | 调用 `read_memory`, `run_command` | 有（211 字符） | > 0 | `reasoning_content` 正常回传 |
| **Seq 8** | assistant | 调用 `run_command` | 有（72 字符） | > 0 | `reasoning_content` 正常回传 |
| **Seq 10** | assistant | 调用 `grep` | 有（157 字符） | > 0 | `reasoning_content` 正常回传 |
| **Seq 12** | assistant | 调用 `read_file`（Message ID: `45353e0b`） | **无（空字符串）** | **0**（`"reasoning_tokens": 0`） | **`reasoning_content` 字段缺失！** |
| **Seq 14** | system | 回传 `read_file` 结果发起下一轮推理 | —— | —— | **被上游网关 400 拦截** |

#### 关键事实发现：大模型连续调用工具时推理为 0 Token
检查 Seq 12 对应的底层 `rawUsage` 记录：
```json
{
  "completion_tokens": 105,
  "completion_tokens_details": {
    "reasoning_tokens": 0
  },
  "prompt_tokens": 15743,
  "total_tokens": 15848
}
```
大模型在接收到 Seq 11 的 `grep` 检索结果后，由于意图明确，**直接生成了针对目标代码文件的 `read_file` 工具调用，跳过了思考阶段**。在该次 SSE 流中，大模型输出了 0 个思考 Token，未下发任何 `reasoning_content` 增量分片。

---

### 1.3 `build_context` 旧逻辑的致命盲区

在 [`src-tauri/src/agent.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/agent.rs) 的历史实现中，存在如下逻辑：
```rust
"assistant" => {
    let mut obj = json!({"role": "assistant"});
    let content = m.content.clone().unwrap_or_default();
    
    // ⚠️ 盲区：只有当数据库中 m.reasoning 存在且非空白时，才写入 reasoning_content
    if let Some(ref r) = m.reasoning {
        if !r.trim().is_empty() {
            obj["reasoning_content"] = Value::String(r.clone());
            est += estimate_tokens(r);
        }
    }
    match &m.tool_calls {
        Some(tcs) if !tcs.is_null() => {
            obj["content"] = if !content.is_empty() { Value::String(content) } else { Value::Null };
            obj["tool_calls"] = tcs.clone();
            out.push(obj);
            // 补齐 tool 结果...
        }
        ...
```
**致命连锁反应**：
1. Seq 12 没有产生思考内容，数据库记录中 `m.reasoning == NULL`；
2. `build_context` 组装上下文时，因为 `m.reasoning` 为空，生成的 Assistant 消息中**完全省略了 `reasoning_content` 键**；
3. 进入下一轮请求（Request 5）时，发往上游 API 的报文里，Seq 12 变成了 `{ "role": "assistant", "content": null, "tool_calls": [...] }`；
4. **DeepSeek / OpenCode 的 Thinking 模式协议强校验**：当客户端启用了思考模式或请求中携带了 `reasoning_effort`，上游服务端对历史消息执行刚性 Schema 校验——**所有发起工具调用的 Assistant 历史消息，必须回传 `reasoning_content` 字段**。若缺失该字段，网关直接判定推理状态机上下文断裂，硬性抛出 400 Bad Request。

---

## 二、 架构设计反思：堆叠式打补丁 vs 管道化治理

在定位到根因后，有两种截然不同的改造思路：

### 2.1 反模式：在 `build_context` 中“堆叠式打补丁”
在 [`src-tauri/src/agent.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/agent.rs) 的 `build_context` 中直接加 `if` 判断：
```rust
// ❌ 典型的堆叠式打补丁（Ad-hoc Patch / 反模式）
if is_thinking_mode {
    obj["reasoning_content"] = Value::String(m.reasoning.unwrap_or_default());
}
```
这种修改具有严重的架构危害：
1. **抽象泄露（Leaky Abstraction）**：`build_context` 属于 Agent 核心领域层，单一职责是从数据库读取事实并在 Token 预算内进行历史滑动窗口与瘦身。它必须保持厂商中立，绝不能引入 DeepSeek 的私有怪癖；
2. **对标准模型的“毒丸效应”（Poison Pill Side Effects）**：如果用户中途将模型切换为官方 `gpt-4o` 或 Azure OpenAI，原生接口会严格检查参数 Schema。向其发送非标准的 `reasoning_content: ""` 会直接引发新异常：`400 Bad Request: Additional properties are not allowed ('reasoning_content' was unexpected)`；
3. **千行巨石代码加速劣化**：[`src-tauri/src/agent.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/agent.rs) 已经有 4700+ 行代码，继续在此堆叠厂商特化判断会导致复杂度爆炸、回归风险不可控。

### 2.2 业界标杆实践：Canonical Domain Model + JIT Transpiler Pipeline
参考 Cline、Roo Code、LiteLLM 等业内顶级 Agent 的架构演进方案：
- **存储与领域层保持中立（Storage Neutrality）**：数据库与 `build_context` 忠实呈现业务数据事实，产出标准的内部规范消息；
- **即时转译与自愈管道下沉（JIT Protocol Sanitizer）**：将所有异构厂商的协议怪癖、参数格式互转、Schema 校验兜底与孤立调用自愈，全部下沉到 [`src-tauri/src/protocol/`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/protocol/) 适配器管道中，在发起 HTTP 请求前的最后一毫秒进行纯函数转译。

---

## 三、 全链路系统拓扑与数据流转

```mermaid
flowchart TD
    subgraph Domain ["1. 统一领域与上下文层 (Neutral Context)"]
        DB[("SQLite: messages 表<br/>role, content, reasoning, tool_calls")] --> BC["agent::build_context<br/>• Token 预算评估与工具结果瘦身<br/>• 输出纯净中立消息序列 Vec&lt;Value&gt;"]
    end

    subgraph Pipeline ["2. JIT 协议自愈与转译层 (protocol::transpiler)"]
        BC --> Router{"协议路由器<br/>dispatch_chat_stream"}
        
        Router -->|Messages| TranspileClaude["transpile_to_anthropic<br/>• 角色严格交替合并 (Squashing)<br/>• tool 转 user.tool_result 块"]
        Router -->|Response| TranspileResp["transpile_to_responses<br/>• instructions 顶层剥离<br/>• input 原语转换"]
        Router -->|ChatCompletions| TranspileChat["transpile_to_openai_chat (本次核心落地)<br/>▼ 依据目标模型能力动态双向自愈：<br/>1. Thinking 模式 (DeepSeek / effort开启):<br/>   确保 assistant 存在 reasoning_content (缺则保底 \"\")<br/>2. Non-Thinking 模式 (GPT-4o / Qwen):<br/>   彻底剥离 reasoning_content 与 reasoning<br/>3. 孤立 Tool Call 自动保序补齐中断占位"]
    end

    subgraph Transport ["3. 纯净传输层 (Transport Adapters)"]
        TranspileClaude --> NetClaude["anthropic::chat_stream<br/>POST /v1/messages"]
        TranspileResp --> NetResp["openai_responses::chat_stream<br/>POST /v1/responses"]
        TranspileChat --> NetChat["openai_chat::chat_stream<br/>POST /chat/completions"]
    end
```

---

## 四、 核心协议自愈规则与实现细节

### 4.1 思考/推理模式自适应启发式判定 (`is_thinking_mode`)
在 [`src-tauri/src/protocol/transpiler.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/protocol/transpiler.rs) 中实现统一的思考模式判定逻辑：

```rust
pub fn is_thinking_mode(model: &str, reasoning_effort: Option<&str>) -> bool {
    // 1. 显式配置了非 default / none 的思考级别
    if let Some(effort) = reasoning_effort {
        let trimmed = effort.trim();
        if !trimmed.is_empty() && trimmed != "default" && trimmed != "none" {
            return true;
        }
    }
    // 2. 识别原生深度思考/推理模型族
    let lower = model.to_ascii_lowercase();
    lower.contains("reasoner")
        || lower.contains("deepseek-r1")
        || lower.contains("-r1")
        || lower.contains("qwq")
        || lower.contains("o1-")
        || lower.contains("o3-")
        || lower == "o1"
        || lower == "o3"
}
```

---

### 4.2 规则一：Thinking 模式下的保底自愈机制
当 `is_thinking_mode` 为 `true` 时，管道遍历所有消息：
- 若消息角色为 `assistant`：
  - 提取现存的 `reasoning_content`，若无则尝试迁移 legacy 的 `reasoning` 字段；
  - 若两者均不存在（如模型 0 Token 推理直接调用工具），**强制写入保底空字符串 `""`**：
    ```rust
    obj.insert("reasoning_content".to_string(), json!(current_reasoning));
    obj.remove("reasoning");
    ```
- **收益**：既忠实保留了模型前序轮次真实的思考流，又完全满足了 DeepSeek 服务端对“回传上下文必须具备 `reasoning_content`”的格式完整性约束。

---

### 4.3 规则二：非 Thinking 模式下的严格脱敏机制
当用户在会话中途将模型切换至原生 `gpt-4o`、`gpt-3.5` 或无思考能力的模型时，`is_thinking_mode` 为 `false`：
- 管道自动将前序由思考模型写入的私有字段完全清除：
  ```rust
  obj.remove("reasoning_content");
  obj.remove("reasoning");
  ```
- **收益**：避免向标准 OpenAI 接口传输未定义的属性字段，杜绝 `400 unexpected property` 报错，实现无感平滑切模型。

---

### 4.4 规则三：孤立 Tool Call 保序自愈机制
在 Agent 执行过程中，用户随时可能点击“停止生成”或中断执行。若历史中残留了 Assistant 的 `tool_calls`，但缺少对应的 `tool` 结果：
- 管道维护保序队列 `pending_tool_calls: Vec<String>`；
- 当遇到后续非 `tool` 角色消息或遍历至末尾时，自动在断点处插入占位结果：
  ```json
  {
    "role": "tool",
    "tool_call_id": orphan_id,
    "content": "(执行被中断，无结果)"
  }
  ```
- **收益**：维护了 OpenAI Chat Completions 协议对 `assistant(tool_calls) -> tool(result)` 调用的状态机完整性。

---

### 4.5 管道转译完整实现与接入

#### 转译器主逻辑 ([`src-tauri/src/protocol/transpiler.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/protocol/transpiler.rs))
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiChatPayload {
    pub messages: Vec<Value>,
    pub tools: Vec<Value>,
}

pub fn transpile_to_openai_chat(
    messages: &[Value],
    tools: &[Value],
    model: &str,
    reasoning_effort: Option<&str>,
) -> OpenAiChatPayload {
    let thinking = is_thinking_mode(model, reasoning_effort);
    let mut sanitized_messages = Vec::with_capacity(messages.len());
    let mut pending_tool_calls: Vec<String> = Vec::new();

    for msg in messages {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");

        if role != "tool" && !pending_tool_calls.is_empty() {
            for orphan_id in pending_tool_calls.drain(..) {
                sanitized_messages.push(json!({
                    "role": "tool",
                    "tool_call_id": orphan_id,
                    "content": "(执行被中断，无结果)"
                }));
            }
        }

        let mut m = msg.clone();
        match role {
            "assistant" => {
                if let Some(obj) = m.as_object_mut() {
                    if thinking {
                        let current_reasoning = obj
                            .get("reasoning_content")
                            .or_else(|| obj.get("reasoning"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        obj.insert("reasoning_content".to_string(), json!(current_reasoning));
                        obj.remove("reasoning");
                    } else {
                        obj.remove("reasoning_content");
                        obj.remove("reasoning");
                    }
                }

                if let Some(tcs) = m.get("tool_calls").and_then(|t| t.as_array()) {
                    for tc in tcs {
                        if let Some(id) = tc.get("id").and_then(|i| i.as_str()) {
                            if !id.is_empty() && !pending_tool_calls.contains(&id.to_string()) {
                                pending_tool_calls.push(id.to_string());
                            }
                        }
                    }
                }
                sanitized_messages.push(m);
            }
            "tool" => {
                if let Some(id) = m.get("tool_call_id").and_then(|i| i.as_str()) {
                    pending_tool_calls.retain(|x| x != id);
                }
                sanitized_messages.push(m);
            }
            _ => {
                sanitized_messages.push(m);
            }
        }
    }

    for orphan_id in pending_tool_calls {
        sanitized_messages.push(json!({
            "role": "tool",
            "tool_call_id": orphan_id,
            "content": "(执行被中断，无结果)"
        }));
    }

    OpenAiChatPayload {
        messages: sanitized_messages,
        tools: tools.to_vec(),
    }
}
```

#### 适配器接入 ([`src-tauri/src/protocol/openai_chat.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/protocol/openai_chat.rs))
```rust
pub async fn chat_stream(
    cfg: &LlmCfg,
    messages: &[Value],
    tools: &[Value],
    mut on_text: impl FnMut(&str) + Send,
    mut on_reasoning: impl FnMut(&str) + Send,
) -> Result<LlmResult, String> {
    let url = endpoint(&cfg.base_url);
    
    // 发起网络请求前即时经过自愈管道规整
    let payload = crate::protocol::transpiler::transpile_to_openai_chat(
        messages,
        tools,
        &cfg.model,
        cfg.reasoning_effort.as_deref(),
    );
    
    let mut body = json!({
        "model": cfg.model,
        "messages": payload.messages,
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    if !payload.tools.is_empty() {
        body["tools"] = Value::Array(payload.tools);
    }
    ...
```

---

## 五、 质量基线与回归验证结论

### 5.1 协议单元测试集验证（12 项测试全部通过）
在 [`src-tauri/src/protocol/tests.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/protocol/tests.rs) 中增补了 3 项针对性单元测试，覆盖故障场景与边界条件：

1. **`test_openai_chat_thinking_mode_auto_heals_missing_reasoning_content`**：
   - 验证在思考模式下，面对轮次 1（带正常思考）与轮次 2（0 Token 思考直接调工具）时，轮次 1 的 `reasoning_content` 保持原样，轮次 2 自动注入 `""`，顺利满足上游约束；
2. **`test_openai_chat_non_thinking_mode_strips_reasoning_content`**：
   - 验证当切入标准 `gpt-4o` 时，历史消息中的 `reasoning_content` 与 `reasoning` 均被彻底剥离；
3. **`test_openai_chat_orphan_tool_call_self_healing`**：
   - 验证在孤立工具调用场景下，管道在紧随其后的消息前自动注入 `(执行被中断，无结果)` 占位。

执行测试结果输出：
```text
running 12 tests
test protocol::tests::test_llm_cfg_effective_session_id ... ok
test protocol::tests::test_protocol_inference ... ok
test protocol::tests::test_openai_chat_non_thinking_mode_strips_reasoning_content ... ok
test protocol::tests::test_anthropic_multimodal_image_url_transpile ... ok
test protocol::tests::test_openai_chat_thinking_mode_auto_heals_missing_reasoning_content ... ok
test protocol::tests::test_anthropic_transpile_basic_and_system_extraction ... ok
test protocol::tests::test_responses_transpile ... ok
test protocol::tests::test_openai_chat_orphan_tool_call_self_healing ... ok
test protocol::tests::test_serde_systemone_compatibility ... ok
test protocol::tests::test_anthropic_orphan_tool_call_self_healing ... ok
test protocol::tests::test_anthropic_multimodal_tool_result_transpile ... ok
test protocol::tests::test_anthropic_transpile_tool_calls_and_results_squashing ... ok

test result: ok. 12 passed; 0 failed; 0 ignored; finished in 0.00s
```

### 5.2 核心业务层测试回归验证
执行核心业务测试 `test_build_context_preserves_reasoning_content`：
```text
running 1 test
test agent::tests::test_build_context_preserves_reasoning_content ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; finished in 0.03s
```
证实核心领域层未发生任何非预期行为改变。

### 5.3 跨平台与前端编译基线
- `cargo check --manifest-path src-tauri/Cargo.toml`：**0 错误**；
- `npm run build`（Vite + TypeScript）：**0 错误构建通过**。

---

## 六、 架构治理启示与工程金律

1. **协议怪癖必须下沉，严禁污染领域核心**：
   面对异构模型网关（OpenAI、DeepSeek、Claude、Responses、Ollama）各异的私有约束与参数怪癖，坚决不向核心业务层（如 `build_context`）引入厂商特化逻辑。必须通过传输层 JIT 转译管道进行边界隔离与兜底自愈；
2. **能力适配必须具备“双向防御性”**：
   解决一个模型的校验问题（如 DeepSeek 需要 `reasoning_content`），必须同时评估对其他模型（如 GPT-4o 严禁出现该字段）的反向影响。自愈管道必须基于目标模型能力（Thinking / Non-Thinking）进行动态双向自适应；
3. **拒绝局部堆叠打补丁，追求每一次改动都使架构更清晰**：
   通过建立统一的协议转译规范与纯函数管道，不仅彻底解决了当前报错，还一并消除了跨模型切换时的隐性雷区，使整体工程架构具备更强的鲁棒性与演进能力。
