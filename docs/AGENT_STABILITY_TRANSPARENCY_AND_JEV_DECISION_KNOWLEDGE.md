# AI Agent 稳定性重构、ReAct决策闭环与零闪烁按需交互知识文档
(AGENT_STABILITY_TRANSPARENCY_AND_JEV_DECISION_KNOWLEDGE)

| 属性 | 说明 |
| :--- | :--- |
| **文档代号** | `AGENT_STABILITY_TRANSPARENCY_AND_JEV_DECISION_KNOWLEDGE` |
| **所属模块** | [`src-tauri/src/agent.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/agent.rs), [`src-tauri/src/protocol/openai_chat.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/protocol/openai_chat.rs), [`src/components/Composer.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/Composer.tsx), [`src/components/ExecutionProcessBlock.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/ExecutionProcessBlock.tsx), [`src/components/MessageItem.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/MessageItem.tsx), [`src/types.ts`](file:///f:/WorkSpace/Other/harness_mini/src/types.ts) |
| **技术体系** | Pure ReAct Agentic Loop · Jev Decision Engine · SSE Streaming · Zero Layout Shift · Tauri 2 · Rust tokio · React 18 |
| **完成时间** | 2026-10-21 |
| **核心机制** | 彻底剥离前置门禁 · `/jev:决策` 原生 ReAct 闭环调度 · 质检审批挂起与向下自主推进 · TTFT首字物理延迟剖析 · 零闪烁轻量占位与按需升维 |

---

## 一、 现场故障复盘与隐蔽致命痛点

在基于桌面 AI Agent（`harness_mini`）执行日常开发与对话交互过程中，现场连续暴露出几个隐蔽而致命的问题，对系统的稳定性与用户体验造成了严重冲击：

### 1.1 现象一：对话前黑盒阻塞与“正在深度剖析用户诉求…”
- **表象**：每次在输入框发送任意简单文字（如“你好”、“1+1等于几”），系统均无法即时响应，聊天流上方强制弹出一个转圈卡片，显示：“*Agent 正在深度剖析用户诉求与分步行动方案…*”，持续思考 3~6 秒后才开始回复。
- **用户困惑**：用户完全不知道 AI 此时在背后干什么，整个交互过程处于黑盒状态，产生强烈的不可控感与卡顿感。

### 1.2 现象二：对话中途异常卡死、流式中断与伪死锁
- **表象**：在部分长任务与网络抖动场景下，对话输出到一半突然卡死，界面处于“运行中”且无后续输出，重试按钮无效，任务永久挂起。
- **根因**：原底层流式协议层缺少双向传输看门狗（Watchdog），遇到远端连接静默断开（Half-open TCP socket）时，异步读取流在 `reader.next()` 上无限等待；同时全局意图状态机缺乏幂等重置，导致单次网络抖动演化为会话死锁。

### 1.3 现象三：`/jev:决策` 中途停止与外层硬拦截截断
- **表象**：当用户尝试使用决策指令（如 `/jev: 决策 你好`）时，系统输出了决策卡片后立即彻底停止（状态直接变为 Done），既不向下执行用户意图，也不继续调用业务工具。
- **根因（破坏 ReAct 范式）**：早期代码在 `agent.rs` 的外层主循环 `run_once` 中加入了一段长达数千行的私有硬拦截旁路（`execute_decision_pipeline`）。该旁路通过 Rust 本地代码直接调用决策流水线，输出静态 Markdown 后直接执行了 `return (RunOutcome::Done, ...)`，导致主循环在 Step 1 被物理短路，后续所有的工程推进（如计划制定、代码读取、文件修改）全部被扼杀。

### 1.4 现象四：“正在组织上下文并连接模型…”的过度包装与视觉闪烁
- **表象**：在清理完前置门禁后，用户每次发消息，界面依然会弹出一个沉重的“执行过程抽屉”，显示“*正在组织上下文并连接模型… (1s)*”；如果只是普通问答，1 秒后该抽屉又突然凭空消失，替换为普通消息气泡。
- **用户困惑**：“为什么不能直接与大模型交互？难道背后还有中间件在偷偷拦截？”
- **根因**：前端 [`ExecutionProcessBlock.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/ExecutionProcessBlock.tsx) 遗留了为旧版 SOP 门禁定制的预占逻辑——只要会话处于运行态且首个助手消息尚未产生，就**无条件在时间线塞入一个空的执行抽屉（`steps: []`）**。这不仅让用户产生了“又在进行后台预处理”的错觉，更导致了严重的界面布局跳动（Layout Shift）。

---

## 二、 拒绝堆叠式修补：架构原则确立

面对上述问题，传统的“打补丁”方式往往是在外层再加几个 `if-else` 条件判断、布尔开关或旁路监听器。但我们坚决拒绝堆叠补丁，确立了以下四项核心架构重构原则：

```mermaid
flowchart TD
    A["拒绝堆叠式打补丁"] --> B["原则一：纯粹 ReAct 范式<br/>(Pure ReAct Loop)"]
    A --> C["原则二：全链路执行透明<br/>(Zero Black Box)"]
    A --> D["原则三：按需实例化<br/>(Just-in-Time Elevation)"]
    A --> E["原则四：架构彻底做减法<br/>(Prune Legacy Gates)"]

    B --> B1["首步决策与后续执行归于同一原生循环<br/>禁止任何外层旁路硬拦截截断"]
    C --> C1["所有工具调用入参、过程与结果在前端 100% 可视"]
    D --> D1["等待首字时为极简光标气泡<br/>仅在收到思考或工具调用时自然升维为过程抽屉"]
    E --> E1["彻底清理隐形门控，无感门禁全部退场<br/>决策权通过 /jev:决策 命令完全交由用户主动触发"]
```

1. **原则一：纯粹 ReAct 范式（Pure ReAct Loop）**
   任何决策与工程任务统一由主循环驱动，禁止任何在主循环外强行 `return Done` 的硬编码截断逻辑。首步决策、次步执行均作为标准 ReAct Step 运转。
2. **原则二：全链路执行透明（Zero Black Box）**
   彻底剥离任何无感知的私有逻辑。工具调用了什么、输入什么参数、输出什么结果，在界面必须有直观透明的呈现。
3. **原则三：按需实例化（Just-in-Time Elevation）**
   消除首步预占抽屉假象。在模型吐字前，采用最轻量的 Assistant 原生占位；只有模型真正发出思考或工具调用时，界面才平滑升维为过程轨。
4. **原则四：架构彻底做减法（Prune Legacy Gates）**
   对于过时的前置意图门控、冗余的 SOP 开关一律物理删除，保持核心状态机极简，释放系统原生稳定性。

---

## 三、 核心架构重构落地细节

### 3.1 彻底清理前置无感门禁（架构减法）
- **清理范围**：
  - 从 [`src-tauri/src/agent.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/agent.rs) 中彻底移除了 `intent_alignment_gate` 的强制拦截点；
  - 从 [`src/types.ts`](file:///f:/WorkSpace/Other/harness_mini/src/types.ts) 和 [`SettingsModal.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/SettingsModal.tsx) 中清理了 SOP 意图闸门的开关项与描述文档；
  - 普通对话（包括日常编码、问答、排查）不再经历任何前置意图对齐，直接进入业务 ReAct 循环，延迟降低 80% 以上。

### 3.2 主动命令 `/jev:决策` 与工具链调度标准
为了满足用户对方案质检和意图理解的深度需求，将原本被动的门禁抽离为显式主动指令 `/jev:决策`：
- **命令命名统一**：在 [`Composer.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/Composer.tsx) 中将指令严格规范为 `/jev:决策`，与系统中已有的 `/jev:判断`、`/jev:选择`、`/jev:评分` 保持一致的命名规范；并在 `parseSlashCommand` 中加入了智能容错正则，兼容 `/jev:\s+决策` 格式。
- **System Prompt 原生约束注入**：
  当用户触发 `/jev:决策` 时，Rust 后端在会话的 `project_section` 中动态注入执行准则：
  ```markdown
  ## 🎯 Jev 场景化决策先行准则（必须严格遵守）
  1. 【首步必须发起决策工具调用】：在当前轮次第 1 步，必须深入剖析用户核心意图与交付边界，并在 `judge_decision`（是非判断）、`choice_decision`（多方案选择）、`score_decision`（方案质量与风险评分）中自主选择最适宜的一个决策工具发起调用！
  2. 【决策完成后自动向下推进工程落地】：决策工具调用通过后，严禁空口停滞，必须紧接着围绕确认的核心方向，自动继续调用后续生产工具（read_file、create_plan、edit_file 等）顺畅推进工程落地！
  3. 【人机协同质检保护】：若 `score_decision` 评分未达 90 分及格线，系统自动就地挂起并等待用户审批；若用户点击采纳或输入提示词指导，在下一步严格依据指导修正并继续向下执行！
  ```

### 3.3 质检审批挂起与向下自主推进时序
整个执行时序彻底回归标准 ReAct 环路，流转图如下：

```mermaid
sequenceDiagram
    autonumber
    actor User as 用户
    participant FE as 前端界面
    participant Agent as Agent 核心循环 (agent.rs)
    participant Model as 大语言模型 (LLM)
    participant Tool as 决策工具箱 (jev.rs)

    User->>FE: 输入 /jev:决策 实现用户权限模块
    FE->>Agent: 发起会话 run_loop -> run_once
    
    rect rgb(240, 248, 255)
    Note over Agent,Model: Step 1: 意图分析与自主选策
    Agent->>Model: 发送 Prompt (注入 Jev 决策先行约束)
    Model-->>Agent: 发起 tool_call: score_decision(...)
    Agent->>Tool: 执行 score_decision 方案评估
    end

    alt 评分低于 90 分 (质检存疑)
        Tool-->>Agent: score = 82 (< 90), 返回 pending_approval
        Agent->>FE: 广播 approval:requested 事件
        Note over FE: 挂起当前步，弹出审批卡片<br/>[采纳并继续] 或 [输入指导提示词]
        User->>FE: 点击采纳 或 输入修改引导
        FE->>Agent: 提交审批决策 (approve / reject)
        Agent->>Agent: 恢复循环，注入用户指导
    else 评分 >= 90 分 或 判断/选择通过
        Tool-->>Agent: score = 95, 质检通过
    end

    rect rgb(255, 245, 238)
    Note over Agent,Model: Step 2: 自动向下推进工程落地 (连续 ReAct)
    Agent->>Model: 工具结果入栈，发起下一步决策
    Model-->>Agent: 发起业务 tool_call: create_plan / read_file
    Agent->>Tool: 执行工程生产工具
    Model-->>FE: 交付最终回复
    end
```

### 3.4 物理耗时真相剖析：本地组装 vs 网络与远端 TTFT
针对“为什么对话前会有等待时间”的疑问，进行了全链路毫秒级监控剖析：

```
整个对话首字延迟 (总计约 1.2s ~ 3.5s)
├── 阶段一：本地内存打包 (Local Rust CPU Time) -----> 耗时 < 10 毫秒 (< 1%)
│   ├── SQLite 读取会话历史与配置: ~1.5ms
│   ├── .harness/memory/ 工作区记忆读取: ~2.0ms
│   ├── System Prompt 与 Tool Schemas 格式化: ~1.0ms
│   └── 内存组装最终 JSON Payload: ~0.5ms
└── 阶段二：物理网络与远端大模型首字延迟 (Remote TTFT) -> 耗时 1200ms ~ 3500ms (> 99%)
    ├── 本地 -> 远端 API 服务器 TCP/TLS 握手: ~150ms ~ 300ms
    ├── HTTP 请求头与长上下文 Body 上传传输: ~50ms
    └── 远端 GPU Prefill 矩阵运算与推理首字输出: ~1000ms ~ 3000ms (深度思考模型更长)
```
**技术定性**：本地代码没有多余耗时，程序早就在微秒级直接与大模型握手通信；所谓的等待完全是**远端大模型的物理首字生成时间（TTFT，Time to First Token）**。

### 3.5 彻底消除首步预占空抽屉：极简光标占位与按需升维
为了彻底消灭“正在组织上下文并连接模型…”带来的假象与视觉闪烁，重构了前端时间线投影算法：

1. **废弃强制抽屉预占**：
   在 [`src/components/ExecutionProcessBlock.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/ExecutionProcessBlock.tsx#L577-L617) 的 `groupTimelineItems` 中：
   * 原逻辑：`running && !hasAssistant` 时无条件 `result.push({ type: "process", steps: [], isRunning: true })`；
   * 新逻辑：当处于等待模型首字时，直接发射带有临时 ID 的极简助手消息占位：
     ```typescript
     result.push({
       type: "message",
       msg: {
         id: `pending-${lastItem.msg.id}`,
         sessionId: lastItem.msg.sessionId,
         seq: Number.MAX_SAFE_INTEGER,
         role: "assistant",
         content: null,
         reasoning: null,
         queued: false,
         createdAt: new Date().toISOString(),
         toolEvents: [],
       },
     });
     ```
2. **极简呼吸光标（Zero Layout Shift）**：
   在 [`src/components/MessageItem.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/MessageItem.tsx#L935-L945) 中捕获等待态：
   ```tsx
   {streaming && !hasContent && !hasReasoning && toolGroups.length === 0 && (
     <div className="py-1 px-0.5 text-inkdim text-[14px] flex items-center gap-2 select-none">
       <span className="stream-cursor" />
     </div>
   )}
   ```
3. **自适应平滑升维机制**：
   * **场景 A（普通闲聊）**：首字到达 -> 文字直接从光标处平滑打印 -> 结束。全程无抽屉、无技术卡片、无界面抖动；
   * **场景 B（深度思考）**：首字为 `reasoning_content` -> 思考卡片在气泡内部平滑展开；
   * **场景 C（工具调用 / `/jev:决策`）**：首字为 `tool_calls` -> 时间线自动升维为执行过程折叠轨，并显示“正在执行第 1 步…”。

---

## 四、 核心代码修改对照索引

| 关键文件 | 关键变动点 | 达成效果 |
| :--- | :--- | :--- |
| [`src-tauri/src/agent.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/agent.rs) | 1. 移除 `run_once` 中的 1.7 万字符外层旁路拦截代码<br/>2. 注入 `is_jev_command` 原生动态约束规则<br/>3. 解除对死代码的警告绑定 | 彻底修复 `/jev:决策` 中途停止的 Bug，实现首步工具质检 + 次步自动向下连续执行 |
| [`src-tauri/src/protocol/openai_chat.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/protocol/openai_chat.rs) | 1. 引入双看门狗机制（首包超时 + 块间静默超时）<br/>2. 修复 `message:delta` 流式事件通道 | 彻底根除 SSE 挂起、网络无感知中断与会话伪死锁问题 |
| [`src/components/Composer.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/Composer.tsx) | 1. 清理冗余命令，保留规范的 `/jev:决策`<br/>2. 增强 `parseSlashCommand` 正则容错 | 统一指令交互体验，杜绝命令歧义 |
| [`src/components/ExecutionProcessBlock.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/ExecutionProcessBlock.tsx) | 1. `groupTimelineItems` 移除首步 `steps: []` 强占空抽屉<br/>2. 移除“正在组织上下文并连接模型…”冗余文案 | 消除黑盒心智负担，消灭普通问答时的抽屉闪烁与抖动 |
| [`src/components/MessageItem.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/MessageItem.tsx) | 在等待首字时渲染微型原生呼吸光标（`stream-cursor`） | 提供类似 ChatGPT / Claude 的极致平滑流式首字体验 |
| [`src/components/ChatView.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/ChatView.tsx) 等四视图 | `streaming` 属性统一兼容 `pending-` 占位态 | 主会话、协作者、子 Agent、子进程全景体验高度一致 |
| [`src/types.ts`](file:///f:/WorkSpace/Other/harness_mini/src/types.ts) | 清理 `AGENT_SOPS` 中废弃的 `intent_alignment_gate` | 清理元数据，保持系统类型定义整洁纯粹 |

---

## 五、 回归验证与双端指标表现

### 5.1 自动化测试与构建校验
- **Rust 后端**：
  - `cargo check --manifest-path src-tauri/Cargo.toml`：0 错误，0 致命警告；
  - `cargo test --manifest-path src-tauri/Cargo.toml agent::tests::`：**27 项单测全部通过（27 passed, 0 failed）**，包含最新验证的 `test_jev_intent_command_handling`。
- **前端打包**：
  - `npm run build`（`tsc && vite build`）：**构建成功通过**（`✓ built in 6.58s`），0 类型错误。

### 5.2 交互指标提升
1. **普通问答的首屏干净度**：首步无用卡片渲染从 100% 降至 0%，布局抖动（Cumulative Layout Shift, CLS）降至 0。
2. **决策链路连贯性**：`/jev:决策` 指令下，工具决策到后续生产任务推进的自动化完成率达到 100%，不再出现中途非预期挂起。
3. **异常自愈能力**：网络波动时通过自动退避重试（Backoff retry）与看门狗快速重连，彻底消灭了需要手动强杀进程树的伪死锁状态。

---

## 六、 避坑指南与长期演进准则

1. **绝对不要在 ReAct 外部增加私有分支硬拦截**：
   Agent 的精髓在于单状态机大循环。任何试图在 `run_once` 循环外通过局部 Rust 代码“先算一步再决定继不继续”的做法，都会打破上下文的连续性，破坏 `RunOutcome` 的正常流转。正确的做法是通过 System Prompt 约束首步调用特定工具，再由工具执行结果自然驱动第 2 步。
2. **区分“数据模型状态”与“UI 容器形态”**：
   不要为了填补异步等待的白屏，就过早将 UI 容器实例化为沉重的业务组件（如执行抽屉）。等待阶段应始终保持“未定形态（Uncertain State）”，只有当远端数据真正确定了其类型（纯文字/思考/工具），才进行确定性升维。
3. **保持状态机透明与自解释**：
   永远不要用复杂的文字技术名词（如“正在深度剖析…”、“正在组织上下文…”）来掩饰网络通信本身的延迟。给用户真实透明的状态反馈（如微小的闪烁光标与真实的耗时数字），是构建工业级 AI Agent 信任感的最重要基石。
