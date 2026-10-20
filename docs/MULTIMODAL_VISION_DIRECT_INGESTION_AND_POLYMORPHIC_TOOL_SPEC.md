# 多模态视觉直通与单一自适应工具架构规范 (MULTIMODAL_VISION_DIRECT_INGESTION_AND_POLYMORPHIC_TOOL_SPEC)

| 属性 | 说明 |
| :--- | :--- |
| **文档代号** | `MULTIMODAL_VISION_DIRECT_INGESTION_AND_POLYMORPHIC_TOOL_SPEC` |
| **创建时间** | 2026-10-20 |
| **适用范围** | `harness_mini`（Tauri 2 + Rust + React 桌面级 AI Agent 架构体系） |
| **关联核心模块** | [`src-tauri/src/tools/skills_memory_plan.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/tools/skills_memory_plan.rs), [`src-tauri/src/agent.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/agent.rs), [`src-tauri/src/protocol/transpiler.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/protocol/transpiler.rs), [`src-tauri/src/tools/specs.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/tools/specs.rs), [`src-tauri/src/store.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/store.rs), [`src-tauri/src/models.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/models.rs) |
| **知识密级** | 核心智能体感知系统规范、多模态直通协议、单一自适应工具治理 |

---

## 目录

1. [背景与核心问题复盘](#一背景与核心问题复盘)
   - 1.1 [传统“工具代理识图模式”的局限性](#11-传统工具代理识图模式的局限性)
   - 1.2 [无成本约束下“工具识图”依然次优的四大理论根因](#12-无成本约束下工具识图依然次优的四大理论根因)
   - 1.3 [反模式警示：为何坚决反对“新增并行看图工具”](#13-反模式警示为何坚决反对新增并行看图工具)
2. [总体架构：单一入口自适应多态感知模型（Polymorphic Tool）](#二总体架构单一入口自适应多态感知模型polymorphic-tool)
   - 2.1 [设计哲学：面向意图（Intent）而非面向实现（Implementation）](#21-设计哲学面向意图intent而非面向实现implementation)
   - 2.2 [双轨感知流转架构图](#22-双轨感知流转架构图)
3. [系统级实施与端到端链路改造](#三系统级实施与端到端链路改造)
   - 3.1 [工具层：运行时模型能力探测与分流（skills_memory_plan.rs）](#31-工具层运行时模型能力探测与分流skills_memory_planrs)
   - 3.2 [会话与消息层：工具结果的多模态附件持久化（agent.rs & store.rs）](#32-会话与消息层工具结果的多模态附件持久化agentrs--storers)
   - 3.3 [上下文组装层：ToolResultItem 与图文混合 Content 组装（agent.rs）](#33-上下文组装层toolresultitem-与图文混合-content-组装agentrs)
   - 3.4 [即时协议转译层：Claude Messages 与 Responses 多模态适配（transpiler.rs）](#34-即时协议转译层claude-messages-与-responses-多模态适配transpilerrs)
4. [演进对比与质量基线验证](#四演进对比与质量基线验证)
   - 4.1 [架构演进前后全维度对比矩阵](#41-架构演进前后全维度对比矩阵)
   - 4.2 [自动化回归验证结论（50 项单测全绿）](#42-自动化回归验证结论50-项单测全绿)
5. [后续演进建议与长视觉上下文治理](#五后续演进建议与长视觉上下文治理)

---

## 一、背景与核心问题复盘

### 1.1 传统“工具代理识图模式”的局限性
在早期的 Agent 系统设计中，对话模型（LLM）多为纯文本模型。为了赋予 Agent 处理图像的能力，系统引入了 `recognize_image(path, prompt)` 工具：
* 主模型生成 Tool Call JSON；
* 工具执行函数读取本地图片，编码为 Base64；
* 工具函数**单独调用一次外部配置的视觉模型**，生成一段数百字的自然语言 Markdown 分析；
* 将此文本包装为 `tool` 角色的结果，返回给主模型。

这一模式对纯文本对话模型非常友好，但当主模型本身已升级为**原生多模态大模型**（如 Claude 3.5/3.7 Sonnet、GPT-4o、Gemini 1.5/2.0 等）时，该模式使得多模态模型只能通过纯文本与图像工具交互，造成了严重的能力阉割与性能浪费。

### 1.2 无成本约束下“工具识图”依然次优的四大理论根因
即便在**不考虑任何 Token 成本与 API 费用**的极端假设下，工具代理识图在智能上限和任务成功率上依然是显著劣化的次优解：

1. **目标驱动的上下文注意力断裂（Context-Blind Defect）**
   - 原生多模态模型的优势在于 Transformer 的跨模态全注意力机制（Cross-Attention）。主模型在思考具体的调试假设或业务逻辑时，其思维链（CoT）、历史代码与视觉 Token 是联合计算注意力的，能够“带着目的去看图”。
   - 二级识图模型是一个被孤立调用的子进程，完全脱离主模型的执行上下文，只能基于通用的 Prompt 泛泛扫描，必然漏掉主模型真正关心的微观特征（如某个边距偏移、局部阴影或闪烁 Toast）。
2. **信息论视角的不可逆有损压缩（Irreversible Lossy Compression）**
   - 图像具有极高的维度空间信息（绝对坐标、相对拓扑、色彩梯度、排版微调）。
   - 自然语言对连续视觉空间的表达带宽极低。降维成文字描述后，微观空间特征永久损耗，后续推理沦为主模型基于残缺信息的猜想。
3. **视觉物理定位（Visual Grounding）能力彻底剥夺**
   - 现代 Agent（如 Browser-Use、UI 自动化、设计稿审查）需要精确的物理坐标定位（Bounding Box / Click Coordinate）。
   - 文本识图工具只能输出“右上角有保存按钮”，无法提供像素级的坐标矩阵，导致基于视觉的精细化自动化操作无法稳定执行。
4. **误差级联与端到端延迟翻倍（Error Cascading & High Latency）**
   - 串行经过两次完整的 LLM 推理往返，网络超时、解析失败、幻觉扩散的系统级脆弱点翻倍。

### 1.3 反模式警示：为何坚决反对“新增并行看图工具”
在试图解决上述问题时，开发者极易陷入**“给多模态模型新增一个 `view_image` 工具，同时保留旧工具 `recognize_image`”**的堆叠式思维。此做法存在严重隐患：
* **模型语义重叠与意图混乱**：两工具功能高度同质化，顶级多模态模型面对“帮我识别这张图”时，因语义诱导极大概率误调 `recognize_image`，自动退化回旧模式；
* **纯文本模型崩溃**：纯文本模型无法识别自身生理缺陷，误调 `view_image` 会导致无法解析或协议报错；
* **Prompt Schema 污染**：必须在 System Prompt 中追加复杂的元指令规则，浪费 Token 且遵从率不可靠；
* **代码分叉**：物理路径校验、沙箱边界检查、20MB 限制、MIME 推断被迫维护两套重复逻辑。

---

## 二、总体架构：单一入口自适应多态感知模型（Polymorphic Tool）

### 2.1 设计哲学：面向意图（Intent）而非面向实现（Implementation）
大模型在执行任务时的核心意图极其纯粹：**“我想观察/审查指定路径的图像（path）”**。
至于具体是：
- **直接看原图**（原生多模态直通），还是
- **请翻译官转述**（纯文本模型代理降级），
这是系统运行时的**执行策略**，必须封装在工具与协议层内部，对大模型完全透明。

### 2.2 双轨感知流转架构图

```
                  Agent 决定审查图像: recognize_image(path, prompt?)
                                         │
                    ┌────────────────────┴────────────────────┐
                    ▼                                         ▼
            [基础校验与路径解析]                       [安全与大小校验]
            相对/绝对/generated_images 目录解析        <= 20MB / MIME 判定
                    │                                         │
                    └────────────────────┬────────────────────┘
                                         │
                        当前主会话模型是否原生支持 Vision？
                                       /   \
                       (支持: vision=true)   (不支持 / 纯文本)
                                     /       \
                                    /         \
    ┌──────────────────────────────▼───┐   ┌───▼──────────────────────────────┐
    │     【原生多模态直通分支】        │   │       【纯文本代理降级分支】      │
    │  1. 跳过外部视觉 LLM 二次调用    │   │  1. 解析生效的专属/全局视觉模型  │
    │  2. 返回轻量结构化直通标识       │   │  2. 构造 messages 发起子调用     │
    │  3. 将原图包装为 Attachment      │   │  3. 生成 Markdown 深入文本报告   │
    │  4. 绑定至当前 Tool 消息持久化   │   │  4. 纯文本回填至 Tool 消息       │
    └──────────────────┬───────────────┘   └───┬──────────────────────────────┘
                       │                       │
                       └───────────────┬───────┘
                                       ▼
                         【协议转译器 JIT Transpiler】
                                       │
            ┌──────────────────────────┴──────────────────────────┐
            ▼                                                     ▼
    [Claude Messages 协议]                               [OpenAI Chat 协议]
    将 Tool 结果自动转译为:                               将 Tool 结果自动转译为:
    tool_result content blocks                           role: "tool"
    ├─ {"type": "text", ...}                             ├─ content: [
    └─ {"type": "image", "source": base64}               │    {"type": "text", ...},
                                                         │    {"type": "image_url", ...}
                                                         │  ]
```

---

## 三、系统级实施与端到端链路改造

### 3.1 工具层：运行时模型能力探测与分流（`skills_memory_plan.rs`）
在 [`recognize_image_tool`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/tools/skills_memory_plan.rs#L380) 中，完成通用路径与 MIME 解析后，基于宿主上下文动态检查当前生效的主模型能力：

```rust
// 智能自适应判定：检查当前主会话模型自身是否原生具备视觉能力 (vision)
let requested_model = args.get("model").and_then(|v| v.as_str()).map(|s| s.trim()).filter(|s| !s.is_empty());
let has_native_vision = if requested_model.is_none() {
    let session = {
        let db = state.db.lock().unwrap();
        crate::store::get_session(&db, session_id)?
            .ok_or_else(|| format!("会话不存在: {session_id}"))?
    };
    let settings = {
        let db = state.db.lock().unwrap();
        let master = state.master_key.lock().unwrap();
        crate::store::get_settings_with_secrets(&db, &master)?
    };
    let (pid, mid) = if let (Some(pid), Some(mid)) = (&session.provider_id, &session.model_id) {
        (Some(pid.clone()), Some(mid.clone()))
    } else if let Some((p, m)) = crate::models::resolve_active_model(&settings) {
        (Some(p.id.clone()), Some(m.to_string()))
    } else {
        (None, None)
    };
    if let (Some(ref p), Some(ref m)) = (pid, mid) {
        settings.has_capability(Some(p), m, "vision")
    } else {
        false
    }
} else {
    false // 显式指定 model 参数时走委托子模型
};

if has_native_vision {
    return Ok(format!(
        "👁️ [图像已直接载入并注入多模态视觉上下文]\n- 目标文件: `{target_display_path}`\n- 图像类型: {mime} ({kb} KB)\n- 分析诉求: {prompt}\n\n<!-- MULTIMODAL_IMAGE_DIRECT:{target_display_path} -->\n系统已将该本地图片的高清原图直接注入到当前对话上下文中。作为原生多模态模型，你拥有对该图像的第一视觉直观感知，请直接结合任务要求进行精准分析与推理。"
    ));
}

// 纯文本模型自动降级：继续执行原有的二次视觉 LLM chat_stream 调用
let (provider, model_name) = resolve_vision_model_for_tool(args, ctx)?;
...
```

### 3.2 会话与消息层：工具结果的多模态附件持久化（`agent.rs` & `store.rs`）
当工具返回包含 `<!-- MULTIMODAL_IMAGE_DIRECT:... -->` 标识时，[`agent.rs:2538`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/agent.rs#L2538) 自动提取物理路径并构造 [`Attachment`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/models.rs#L1134)，利用已有的 [`new_message_with_attachments`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/store.rs#L3346) 将附件结构存入 SQLite 的 `attachments_json` 字段中：

```rust
let tool_attachments = if tc.name == "recognize_image" && status == "success" {
    if let Some(start) = result_text.find("<!-- MULTIMODAL_IMAGE_DIRECT:") {
        let rest = &result_text[start + "<!-- MULTIMODAL_IMAGE_DIRECT:".len()..];
        if let Some(end) = rest.find(" -->") {
            let img_path = rest[..end].trim();
            let p = std::path::Path::new(img_path);
            if let Ok(meta) = std::fs::metadata(p) {
                let name = p.file_name().and_then(|f| f.to_str()).unwrap_or("image.png").to_string();
                let ext = p.extension().and_then(|e| e.to_str()).map(|s| s.to_lowercase()).unwrap_or_default();
                let mime = match ext.as_str() {
                    "jpg" | "jpeg" => "image/jpeg",
                    "webp" => "image/webp",
                    "gif" => "image/gif",
                    "svg" => "image/svg+xml",
                    _ => "image/png",
                };
                Some(vec![crate::models::Attachment {
                    id: uuid::Uuid::new_v4().to_string(),
                    name,
                    mime_type: mime.to_string(),
                    size: meta.len(),
                    path: img_path.to_string(),
                    is_image: true,
                }])
            } else { None }
        } else { None }
    } else { None }
} else { None };

let tool_msg = {
    let db = state.db.lock().unwrap();
    let r = store::new_message_with_attachments(&db, session_id, "tool", Some(result_text.clone()), tool_attachments, false, None);
    if let Ok(ref m) = r {
        let _ = store::set_message_tool_call_id(&db, &m.id, &tc.id);
    }
    r
};
```

### 3.3 上下文组装层：ToolResultItem 与图文混合 Content 组装（`agent.rs`）
在 [`build_context`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/agent.rs#L3540) 中，工具结果索引由扁平的 `HashMap<String, String>` 升级为附带附件元数据的结构体：

```rust
let mut tool_results: HashMap<String, (String, Option<Vec<crate::models::Attachment>>)> = HashMap::new();
```

当为 `assistant` 的 `tool_calls` 匹配对应回填结果时，若包含图像附件，则将 `out` 数组中该 `tool` 消息的 `content` 组装为包含 `text` 与 `image_url` 的 Content 数组：

```rust
let (text, atts) = tool_results.remove(&id).unwrap_or_else(|| ("(执行被中断，无结果)".into(), None));
est += estimate_tokens(&text);

let mut image_parts: Vec<Value> = Vec::new();
if let Some(ref att_list) = atts {
    for att in att_list {
        if att.is_image {
            if let Ok(bytes) = std::fs::read(&att.path) {
                let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                let mime = if att.mime_type.is_empty() { "image/png" } else { &att.mime_type };
                let data_url = format!("data:{};base64,{}", mime, b64);
                image_parts.push(json!({
                    "type": "image_url",
                    "image_url": { "url": data_url }
                }));
                est += 1000;
            }
        }
    }
}

if !image_parts.is_empty() {
    let mut parts = vec![json!({"type": "text", "text": text})];
    parts.extend(image_parts);
    out.push(json!({"role": "tool", "tool_call_id": id, "content": parts}));
} else {
    out.push(json!({"role": "tool", "tool_call_id": id, "content": text}));
}
```

### 3.4 即时协议转译层：Claude Messages 与 Responses 多模态适配（`transpiler.rs`）
在 [`transpile_to_anthropic`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/protocol/transpiler.rs#L26) 中，当 `tool` 结果的 `content` 为结构化数组时，自动转译为符合 Anthropic 规范的多模态 Content Blocks：

```rust
"tool" => {
    let call_id = m.get("tool_call_id").and_then(|i| i.as_str()).unwrap_or("");
    if let Some(text) = m.get("content").and_then(|c| c.as_str()) {
        raw_converted.push(json!({
            "role": "user",
            "content": [{ "type": "tool_result", "tool_use_id": call_id, "content": text }]
        }));
    } else if let Some(parts) = m.get("content").and_then(|c| c.as_array()) {
        let mut tool_blocks = Vec::new();
        for part in parts {
            let p_type = part.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if p_type == "text" {
                if let Some(t) = part.get("text").and_then(|s| s.as_str()) {
                    tool_blocks.push(json!({"type": "text", "text": t}));
                }
            } else if p_type == "image_url" {
                if let Some(url) = part.get("image_url").and_then(|u| u.get("url")).and_then(|s| s.as_str()) {
                    if let Some((mime, b64)) = parse_data_url(url) {
                        tool_blocks.push(json!({
                            "type": "image",
                            "source": { "type": "base64", "media_type": mime, "data": b64 }
                        }));
                    }
                }
            }
        }
        if tool_blocks.is_empty() {
            tool_blocks.push(json!({"type": "text", "text": "..."}));
        }
        raw_converted.push(json!({
            "role": "user",
            "content": [{ "type": "tool_result", "tool_use_id": call_id, "content": tool_blocks }]
        }));
    }
}
```

在 [`transpile_to_responses`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/protocol/transpiler.rs#L396) 中，增加对图文混合数组的安全文本提取，保障 OpenAI Responses 协议中 `function_call_output` 的纯字符串契约。

---

## 四、演进对比与质量基线验证

### 4.1 架构演进前后全维度对比矩阵

| 评估维度 | 演进前：纯工具代理模式 | 演进后：单一入口自适应多态模式 |
| :--- | :--- | :--- |
| **多模态模型表现** | 强制降级为纯文本，失去细微排版、坐标感知 | **第一视角直连**，原生注意力和全上下文协同推理 |
| **纯文本模型表现** | 支持（调用辅助视觉模型） | **完全保持兼容**（自动平滑回退，逻辑零破坏） |
| **工具数量与 Schema** | 1 个工具（`recognize_image`） | **依然仅 1 个工具**（对外名称不变，零认知负担） |
| **Prompt 元指令税** | 无 | **零新增**（无需在 Prompt 中教模型如何根据能力选工具） |
| **端到端交互延迟** | 高（主模型与工具模型两次完整往返） | **低**（多模态直通单跳完成，省去二次网络 I/O） |
| **协议标准对齐** | 仅支持平铺 String 工具结果 | 完美契合 Anthropic Computer Use / Multimodal Tool Result 规范 |
| **用户切换模型体验** | 无感知 | **完全平滑自愈**（切换模型后下一轮调用自动走适配通道） |

### 4.2 自动化回归验证结论（50 项单测全绿）
所有相关核心单元测试已全数通过：
1. **协议转译测试**（`protocol::tests`，9 项）：
   - `test_anthropic_multimodal_tool_result_transpile` ✅ **通过**（新增：验证多模态工具结果转换为 Claude 原生 `image` block）
   - `test_anthropic_orphan_tool_call_self_healing` ✅ 通过
   - `test_anthropic_multimodal_image_url_transpile` ✅ 通过
   - `test_responses_transpile` ✅ 通过
2. **智能体运行核心测试**（`agent::tests`，28 项）：
   - 全部 28 项上下文构建、工具状态追踪、截断防护测试在 0.28s 内全部通过。
3. **工具执行与文件系统测试**（`tools::tests`，13 项）：
   - 全部 13 项参数校验、断言与执行测试全部通过。

---

## 五、后续演进建议与长视觉上下文治理

随着原生多模态直通的开启，Agent 连续观察多张高分辨率图片时，历史图像的 Token 消耗可能呈线性增长。建议在后续迭代中配合现有的 [`Dynamic Context-Aware Tool Result Slimming`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/agent.rs#L3689-L3730) 实施**视觉滑动保鲜窗（Visual Sliding Window）**：
1. **活跃步保护**：最近 1~2 轮的 `tool` 图片保留完整的高清 Base64 数据；
2. **历史图像下沉脱水**：当历史步数超过水位线（如超出安全阈值 80% Token Limit）时，协议转译层自动将较早轮次的 Base64 替换为 `"[历史图片已归档: path/to/image.png]"`，在保障主模型近期精准视觉注意力的同时，消除长期上下文溢出的风险。
