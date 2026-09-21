# 上下文 Token 智能管理与可视化压缩技术规范
# Context Token Management & Visual Compaction Specification

本文档详细阐述 `harness_mini` 在面对长对话、复杂工具调用及海量代码交互时，针对 **上下文 Token 限制、智能摘要压缩、模型窗口差异化配置与兜底原子轮次截断** 的完整系统设计、实现架构与技术规范。

---

## 目录
- [一、背景与核心挑战](#一背景与核心挑战)
- [二、整体架构与处理管道](#二整体架构与处理管道)
- [三、精准 Token 计量与预估模型](#三精准-token-计量与预估模型)
- [四、多模型上下文规格矩阵与动态配置](#四多模型上下文规格矩阵与动态配置)
- [五、可视化自动压缩与人机协同工作流](#五可视化自动压缩与人机协同工作流)
- [六、兜底机制：原子轮次截断（Atomic Turn-based Pruning）](#六兜底机制原子轮次截断atomic-turn-based-pruning)
- [七、数据持久化、IPC 契约与事件总线规范](#七数据持久化ipc-契约与事件总线规范)
- [八、测试用例与验证标准](#八测试用例与验证标准)

---

## 一、背景与核心挑战

在以自主编程、多步工具执行（如连续调用 `read_file`、`edit_file`、`run_command`）为核心场景的代码助手系统中，上下文管理面临以下四大核心矛盾：

1. **有限窗口与暴增信息量的矛盾**：
   - 频繁的代码读取（单次可能输出上千行）与终端命令日志，会迅速耗尽模型的上下文窗口（Context Window），导致模型丢失初始需求目标或直接报错崩溃。
2. **机械硬截断导致的 API 协议损毁**：
   - OpenAI 及众多主流模型服务（DeepSeek、Claude、Qwen 等）的 Chat Completions 规范要求：`role="tool"` 的返回消息必须紧随包含该 `tool_call_id` 的 `role="assistant"` 之后。
   - 传统的“按单条消息从前往后丢弃”策略极易切断 Assistant 与 Tool 之间的父子关联，导致遗留下孤立的 `tool` 消息，直接触发服务端 `400 Bad Request: An assistant message with 'tool_calls' must be followed by tool messages` 致命错误。
3. **黑盒式自动压缩与人类掌控感的冲突**：
   - 传统的无感隐式压缩让开发者无法了解哪些历史信息被丢失或概括；若提炼出的摘要存在技术偏差，可能误导后续所有步骤。
   - 但若完全依赖人工阻塞确认，在长耗时任务中用户一旦离席，Agent 就会无限期停滞挂起。
4. **不同提供商与模型规格千差万别**：
   - DeepSeek 常用 64K 上下文、Claude/Kimi 具备 200K、GPT-4o/Qwen 支持 128K、Gemini 可达 1M+、本地 Ollama 通常只有 8K/32K。固定写死上限无法发挥各模型的最大效能。

针对上述问题，本系统构建了贯穿 **精准计量 ➔ 历史瘦身 ➔ 阈值提炼 ➔ 30秒弹性确认 ➔ 原子成对兜底** 的全链路治理方案。

---

## 二、整体架构与处理管道

```mermaid
flowchart TD
    subgraph Trigger [执行前上下文检测 run_once]
        A[新一轮对话发起 / 工具回调完毕] --> B[收集 Session 内全量历史消息]
        B --> C[精准预估总 Token 消耗 est_tokens]
    end

    subgraph Compaction [智能语义压缩 check_and_trigger_compaction]
        C --> D{est_tokens >= 上限阈值 75%?}
        D -->|否| J[进入上下文组装阶段]
        D -->|是| E[圈定活跃轮次前的往期历史 seq: start..end]
        E --> F[调用 LLM 提炼 4 段式 Markdown 技术备忘录]
        F --> G[派发 compaction:request 事件通知前端]
        G --> H{前端响应等待: 30 秒超时器}
        H -->|30秒内 用户点击确认/补充| I1[应用人工编辑/确认后的 Markdown]
        H -->|30秒内 用户点击跳过| I2[放弃压缩，直接继续后续流程]
        H -->|超过30秒无操作 超时自动应用| I3[自动采用初版 Markdown 存库<br/>派发 compaction:timeout 事件<br/>前端通知保持展示供查阅，移除继续按钮]
        I1 & I3 --> I4[持久化至 session_compactions 表<br/>派发 compaction:applied]
    end

    subgraph ContextBuild [上下文安全构建 build_context]
        I4 & I2 & J --> K[注入 System Prompt 与项目 SOP / 演进经验]
        K --> L[注入已生效的最新压缩备忘录<br/>过滤已归档的历史消息 seq <= end_seq]
        L --> M[历史工具大输出折叠瘦身 >2048 字符]
        M --> N{此时总 Tokens 依然 > context_token_limit?}
        N -->|否| P[组装完成: 交付流式 LLM 生成]
        N -->|是 极端情况/单轮超大| Q[触发: 原子轮次成对淘汰 Atomic Turn Pruning]
        Q --> R[成对移出最早的一个完整交互轮次<br/>user -> assistant(tools) -> tool* -> assistant]
        R --> S[派发非阻塞 context:truncated 通知事件]
        S --> N
    end
```

---

## 三、精准 Token 计量与预估模型

精确的 Token 计数是预防超限和精准触发压缩的前提。

### 1. 计量模型与经验常数
对于未分词的原始字符串，系统采用字符加权启发式计量法：
- **ASCII / 英文字符**：按 `len * 0.3` 估算；
- **CJK / 中文字符**：按 `len * 0.7` 估算；
- **基础开销**：单条消息元数据（role、name、边界）固定计入 4 tokens。

### 2. `tool_calls` 参数级递归计量（`estimate_value_tokens`）
旧有系统常忽略助手消息中 `tool_calls` 内传递的大段 JSON 代码或参数，导致严重漏算。本系统在 [`src-tauri/src/models.rs`](file:///d:/WorkSpace/Other/harness_mini/src-tauri/src/models.rs) 中实现了递归参数 Token 解析：
```rust
pub fn estimate_value_tokens(val: &serde_json::Value) -> usize {
    match val {
        serde_json::Value::Null => 1,
        serde_json::Value::Bool(_) => 1,
        serde_json::Value::Number(_) => 2,
        serde_json::Value::String(s) => estimate_tokens(s),
        serde_json::Value::Array(arr) => {
            arr.iter().map(estimate_value_tokens).sum::<usize>() + 2
        }
        serde_json::Value::Object(map) => {
            map.iter()
                .map(|(k, v)| estimate_tokens(k) + estimate_value_tokens(v) + 2)
                .sum::<usize>() + 2
        }
    }
}
```

### 3. 历史工具超大输出折叠瘦身（Tool Result Slimming）
- **痛点**：在多轮对话中，先前步骤中读取的数千行文件（`read_file`）或大量检索结果，在当前轮次通常只需保留其“已被读取”的事实，全量保留会造成严重的 Token 浪费。
- **机制**：在 [`build_context`](file:///d:/WorkSpace/Other/harness_mini/src-tauri/src/agent.rs) 中，非当前轮次的往期 `role="tool"` 输出若超过 **2048 字符**，自动实施上下文瘦身：
  ```rust
  let slimmed = format!(
      "{}\n\n...[历史工具输出过长（共 {} 字符），已折叠前序详情以节省上下文空间]...",
      &text[..2048],
      text.len()
  );
  ```
- **注意**：只瘦身传递给模型的瞬时上下文，数据库中始终完整保存原始内容，保证审计与前端查阅不受损。

---

## 四、多模型上下文规格矩阵与动态配置

在设置中心（`SettingsModal.tsx`）中，系统内置了主流大模型的标准规格与推荐上限，并支持自定义微调。

### 1. 预设规格矩阵表

| 模型规格标识 (Preset ID) | 代表厂商 / 模型 | 物理窗口总量 | 推荐上下文阈值 (Recommended Limit) | 预留补全/输出空间 |
| :--- | :--- | :--- | :--- | :--- |
| **`deepseek-64k`** | DeepSeek V3 / R1 (常用配置) | 64,000 | **52,000** | 12,000 |
| **`claude-200k`** | Anthropic Claude 3.5 Sonnet | 200,000 | **160,000** | 40,000 |
| **`gpt4o-128k`** | OpenAI GPT-4o / GPT-4o-mini | 128,000 | **100,000** | 28,000 |
| **`qwen-128k`** | 阿里通义千问 Qwen 2.5 / Plus | 128,000 | **100,000** | 28,000 |
| **`glm4-128k`** | 智谱清言 GLM-4 / GLM-4-Plus | 128,000 | **100,000** | 28,000 |
| **`kimi-200k`** | 月之暗面 Moonshot Kimi Chat | 200,000 | **160,000** | 40,000 |
| **`gemini-1m`** | Google Gemini 1.5 Pro / 2.0 | 1,000,000 | **500,000** | 500,000 |
| **`local-8k`** | 本地小显存模型 (Ollama 8K) | 8,192 | **6,000** | 2,192 |
| **`local-32k`** | 本地常规模型 (Ollama 32K) | 32,768 | **26,000** | 6,768 |

### 2. 界面交互与输入约束
- **下拉与快捷标签**：按厂商分类展示下拉菜单与常用 Tag，点击即可一键填入推荐阈值。
- **开放式手动微调**：数字输入框放宽至 `2,000` 到 `2,000,000`，满足开发者针对私有微调模型或混合推理窗口的个性化需求。
- **前后端默认值基准**：前后端统一默认值为 **`64,000`** Tokens。

---

## 五、可视化自动压缩与人机协同工作流

### 1. 触发时机与历史切片
- **触发阈值**：当历史消息累计预估 Token 达到设定的 `context_token_limit * 75%` 时，触发压缩判定。
- **保护当前轮次**：压缩仅针对当前活跃轮次之前的“已完成轮次”（候选区间 `start_seq..=end_seq`），绝不压缩当前用户刚刚输入的最新指令。

### 2. 结构化技术备忘录提炼 Prompt 设计
系统调用 LLM，将候选区间的对话与工具调用提炼为 4 个固定板块的 Markdown：
```markdown
### 🎯 任务背景与核心目标
（用 2-3 句话总结本次对话最初的目标和用户意图）
### 🔑 关键技术决策与约定
（架构决策、依赖选型、代码规范、用户中途明确提出的硬性要求）
### 📁 涉及文件与修改记录
（已读取、创建或编辑的文件列表及主要变更点）
### 📌 历史遗留与注意事项
（之前步骤中发现的坑、未决问题或后续需要注意的事项）
```

### 3. 30 秒弹性阻塞与非阻塞保持工作流（核心特性）

```
[Agent 产生压缩备忘录] 
         │
         ├─── 发送 compaction:request ───► [前端弹出 CompactionBanner]
         │                                       │
         ▼                                       ├── 倒计时 30s 开始显示 (剩余 Xs)
[后端异步等待 30 秒]                              ├── 用户可切换 Markdown 预览
         │                                       └── 用户可切到编辑框修改内容
         ├───────────────────────────────────────┤
  (情景 A: 30秒内人工确认)                (情景 B: 30秒无操作超时)
         │                                       │
   用户点击【确认并应用】                   30 秒超时到达，后端自动唤醒
         │                                       │
   后端收到人工修改后的 Markdown            后端以初始备忘录持久化入库
   写入数据库并继续执行                    后端派发 compaction:timeout
   前端关闭卡片，进入下一步                后端继续执行后续流程 (不卡死)
                                                 │
                                                 ▼
                                          [前端卡片平滑切换状态]
                                          • 状态标识: "30秒无操作已自动应用"
                                          • 移除【确认并应用（继续执行）】按钮
                                          • 移除【跳过】按钮
                                          • 保持展示 Markdown 备忘录供查看
                                          • 提供【我知道了】/【关闭】按键
```

- **设计目标**：
  - 避免“无人值守时任务无限停滞”；
  - 避免“超时后弹窗突兀消失导致用户不知道压缩了什么”；
  - 杜绝“后续任务已经继续执行，前端却依然能点击‘继续下一步’导致逻辑重入冲突”。

### 4. 历史归档与会话折叠展示
- 一旦某段历史被成功压缩，聊天界面（[`ChatView.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/ChatView.tsx)）将**完整保留所有历史对话记录**，确保用户可随时向上回溯查阅历史发言与工具执行详情。
- 压缩卡片 [`CompactedHistoryCard`](file:///f:/WorkSpace/Other/harness_mini/src/components/CompactionBanner.tsx) 会作为阶段性里程碑内联插入在对应轮次之间（`seq = end_seq` 之后），醒目标注该阶段历史已被压缩提炼为技术备忘录并注入后续上下文。
- 用户可随时点击卡片展开查阅已被持久化的技术备忘录与压缩生成时间。

---

## 六、兜底机制：原子轮次截断（Atomic Turn-based Pruning）

### 1. 触发场景
在以下两种极限情况下，单靠语义压缩可能仍不足以使上下文降低至安全限制以下：
1. **单轮对话超大**：用户在单轮内粘贴了上万行日志，或某个工具在单轮执行中产生了超大返回值；
2. **极小窗口配置**：用户手动将上下文上限调至非常小的数值（例如 4,000 Tokens）。

此时，系统进入物理裁剪阶段。

### 2. 传统截断致命漏洞与“原子轮次”解决方案
- **OpenAI / 兼容协议强制约束**：
  ```json
  [
    {"role": "user", "content": "帮我看看这个文件"},
    {"role": "assistant", "tool_calls": [{"id": "call_123", ...}]},
    {"role": "tool", "tool_call_id": "call_123", "content": "文件内容..."},
    {"role": "assistant", "content": "文件看完了，存在以下问题..."}
  ]
  ```
  - 如果按消息单条丢弃，若丢弃了前面的 `assistant(tool_calls)`，留下单独的 `role="tool"` 消息排在最前，调用 API 时将直接遭遇 **`400 Bad Request: Invalid 'messages': orphaned tool response`**。
- **原子轮次（Atomic Turn）定义**：
  - 一个最小不可分割轮次定义为：从 `role="user"` 发起，中间包含伴随的 `assistant` 及若干 `tool`，直到下一轮 `user` 之前的整个消息集合。
- **安全裁剪算法**：
  在 [`src-tauri/src/agent.rs`](file:///d:/WorkSpace/Other/harness_mini/src-tauri/src/agent.rs) 的 `build_context` 中：
  ```rust
  // 按原子轮次成对淘汰最早的完整历史轮次，直至预估总量 <= token_limit
  while est_total > token_limit && turns.len() > 1 {
      let dropped_turn = turns.remove(0); // 移除最早的一整轮
      dropped_turns_count += 1;
      dropped_messages_count += dropped_turn.len();
      // 动态更新 Token 预估并记录截断提醒
  }
  ```
  该算法保证：**只要工具消息被移出，其对应的助手调用也必被一同移出；留在上下文中的消息永远具备完整的父子协议调用链**。

### 3. 非阻塞截断提醒（`TruncationNotice`）
- 发生截断并不中断正在运行的模型生成，而是作为可观察性指标异步推送至前端。
- 前端展示卡片包含：
  - 移出轮数与消息数统计；
  - 截断前 Tokens、移出释放 Tokens、截断后当前 Tokens 对比；
  - 最早被淘汰消息文本预览折叠展开；
  - 手动关闭与批量关闭功能。

---

## 七、数据持久化、IPC 契约与事件总线规范

### 1. SQLite 数据库结构
在 `session_compactions` 表中存储已应用的压缩记录：
```sql
CREATE TABLE IF NOT EXISTS session_compactions (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    start_seq INTEGER NOT NULL,
    end_seq INTEGER NOT NULL,
    summary_markdown TEXT NOT NULL,
    tokens_before INTEGER NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_session_compactions_session ON session_compactions(session_id);
```

### 2. Rust 数据结构
```rust
/// 上下文压缩请求（推送给前端展示并等待确认）
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CompactionRequest {
    pub event_id: String,
    pub session_id: String,
    pub start_seq: i64,
    pub end_seq: i64,
    pub start_preview: String,
    pub end_preview: String,
    pub message_count: usize,
    pub tokens_before: usize,
    pub summary: String,
    pub timeout_seconds: u64, // 默认 30
}

/// 上下文硬截断通知（推送给前端提示卡片）
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TruncationNotice {
    pub id: String,
    pub session_id: String,
    pub dropped_turns: usize,
    pub dropped_messages: usize,
    pub dropped_tokens: usize,
    pub token_limit: usize,
    pub est_tokens_before: usize,
    pub est_tokens_after: usize,
    pub first_preview: String,
    pub created_at: String,
}
```

### 3. IPC 命令与事件通道

| 通信通道 (Channel / Command) | 方向 | 载荷内容 | 业务语义与行为 |
| :--- | :--- | :--- | :--- |
| **`compaction:request`** | 后端 ➔ 前端 | `CompactionRequest` | 触发前端弹出确认横幅，启动 30 秒倒计时 |
| **`compaction:timeout`** | 后端 ➔ 前端 | `{ eventId, sessionId, autoApplied: true }` | 通知前端 30 秒已到已自动推进，卡片切换为只读等待查看模式 |
| **`compaction:resolved`** | 后端 ➔ 前端 | `{ eventId }` | 用户在 30 秒内手动点击了确认或跳过，前端关闭横幅 |
| **`compaction:applied`** | 后端 ➔ 前端 | `SessionCompaction` | 压缩备忘录已正式落库生效，前端更新归档卡片列表 |
| **`context:truncated`** | 后端 ➔ 前端 | `TruncationNotice` | 发生原子轮次截断，前端非阻塞挂出警示卡片 |
| **`respond_compaction`** (IPC) | 前端 ➔ 后端 | `(eventId, approved, finalSummary)` | 前端将用户的确认或编辑后的备忘录发回后端唤醒通道 |
| **`list_session_compactions`** (IPC) | 前端 ➔ 后端 | `(sessionId)` | 切换会话时拉取该会话的所有历史压缩记录 |

---

## 八、测试用例与验证标准

系统通过了完备的自动化单元测试与静态构建检验。

### 1. 核心单元测试用例
位于 [`src-tauri/src/agent.rs`](file:///d:/WorkSpace/Other/harness_mini/src-tauri/src/agent.rs)：

- **`test_build_context_with_compactions`**：
  - 验证当数据库存在 `session_compactions` 记录时，`build_context` 能正确将已压缩范围（`seq <= end_seq`）的历史消息过滤，并将格式化的 Markdown 备忘录作为 `user` 与 `assistant` 的对答注入到上下文第 1、2 位，验证后置活跃消息依然正常挂载。
- **`test_atomic_turn_drop_no_orphaned_tools`**：
  - 构造包含复杂 `tool_calls` 和 `tool` 结果的早期交互轮次；
  - 故意将 `context_token_limit` 设为极低数值（如 30 tokens）；
  - 执行 `build_context`，断言早期轮次被整轮淘汰；
  - 遍历输出消息，断言**绝不存在孤立的 `tool` 消息**（所有 `role="tool"` 的前一条消息必然是 `role="assistant"`）；
  - 验证正确生成并返回 `TruncationNotice` 统计结构体。
- **`test_estimate_tokens` 与参数计量测试**：
  - 验证英文字符、中文字符、复杂嵌套 JSON 对象在 `estimate_value_tokens` 下的估算准确性。

### 2. 自动化构建标准
- **后端**：`cargo test --lib` 63 个测试用例全部 Pass，0 失败。
- **前端**：`npm run build`（`tsc && vite build`）TypeScript 类型检查 0 错误，静态打包顺利通过。

---

## 总结

通过上述方案，`harness_mini` 建立了一套**精准预估、智能总结、人机协同弹性确认、协议级原子安全兜底**的完整上下文治理体系。不仅大幅提升了长任务编程对话的成功率与记忆持久度，而且通过 30 秒无操作自动继续与无阻塞通知机制，达成了自动化执行与人工透明可控性的最佳平衡。
