# AI Agent 会话消息时序完整性、分页截断与执行过程抽屉聚合治理技术规范
# AI Agent Message Sequence Integrity, Pagination Invariant & Execution Timeline Governance Specification

本文档详细记录 `harness_mini` 在处理复杂 AI Agent 对话、多步 ReAct 循环长任务及会话切换时，所遭遇的 **消息时序颠倒、执行过程串轮、最终交付答复倒置置顶** 等严重乱序故障的排查全过程、底层机制根因剖析、时序推导证明、业内标杆架构对比，以及最终落地的 **时序单调递增不变量（Monotonic Ordering Invariant）、双向去重合并、分页粒度优化与 `run_id` 物理隔离** 等全链路治理方案。

---

## 目录
- [一、故障背景与问题全景](#一故障背景与问题全景)
- [二、底层真实数据拓扑还原](#二底层真实数据拓扑还原)
  - [2.1 倒数第二轮对话（Turn 3）拓扑](#21-倒数第二轮对话turn-3拓扑)
  - [2.2 最后一轮对话（Turn 4）拓扑](#22-最后一轮对话turn-4拓扑)
- [三、底层根因深度剖析](#三底层根因深度剖析)
  - [3.1 首屏小分页与单轮超长执行之间的尺寸倒挂](#31-首屏小分页与单轮超长执行之间的尺寸倒挂)
  - [3.2 `mergeSessionMessages` 数组合并的无序追加漏洞](#32-mergesessionmessages-数组合并的无序追加漏洞)
  - [3.3 现象一：“最后一轮对话显示到最前面”的数学推导](#33-现象一最后一轮对话显示到最前面的数学推导)
  - [3.4 现象二：“前几轮回复混入最后一轮执行过程”的推导](#34-现象二前几轮回复混入最后一轮执行过程的推导)
  - [3.5 全链路缺乏 `seq` 单调递增不变量守护](#35-全链路缺乏-seq-单调递增不变量守护)
  - [3.6 抽屉聚合状态机对物理顺序的脆弱依赖](#36-抽屉聚合状态机对物理顺序的脆弱依赖)
- [四、业内标杆实践对比与架构启示](#四业内标杆实践对比与架构启示)
  - [4.1 Claude Code / Cursor：Turn/Run 作为一等公民](#41-claude-code--cursorturnrun-作为一等公民)
  - [4.2 VS Code Chat：单向单调 Sequence 不变量](#42-vs-code-chat单向单调-sequence-不变量)
  - [4.3 为什么 Agent 场景不能按固定消息条数硬切分页](#43-为什么-agent-场景不能按固定消息条数硬切分页)
- [五、完整治理与工程落地实施方案](#五完整治理与工程落地实施方案)
  - [5.1 核心排序工具函数 `sortMessages`](#51-核心排序工具函数-sortmessages)
  - [5.2 重构 `mergeSessionMessages`：双向去重与属性取精](#52-重构-mergesessionmessages双向去重与属性取精)
  - [5.3 数据入队全链路时序守护（Upsert / Delta / Reset）](#53-数据入队全链路时序守护upsert--delta--reset)
  - [5.4 选择器防御性排序（Defensive Selector）](#54-选择器防御性排序defensive-selector)
  - [5.5 首屏分页扩容与 `loadEarlier` 归位](#55-首屏分页扩容与-loadearlier-归位)
  - [5.6 引入 `run_id` 双重隔离守护抽屉边界](#56-引入-run_id-双重隔离守护抽屉边界)
- [六、仿真验证与修复效果对比](#六仿真验证与修复效果对比)
- [七、代码防劣化规范与工程化约束](#七代码防劣化规范与工程化约束)

---

## 一、故障背景与问题全景

在 `harness_mini` 的持续使用中，用户报告会话区频繁出现严重的消息显示混乱，主要表现为以下典型症状：

1. **现象 A：最后一轮的执行过程与最终交付答复倒置，被直接显示到了会话的最顶部**
   - 用户发送确认指令后，Agent 经过多步工具调用给出了最终交付回复；但在切换会话或页面刷新后，该回复及后半段执行过程直接跳到了整个聊天窗口的最上方（甚至排在第 1 轮提问之前）。
2. **现象 B：前几轮的回复被显示到了最后一轮的执行过程抽屉、或者最后一轮的消息回复里**
   - 最后一轮正在执行或完成后，折叠卡片中展开后发现竟然包含了前几轮的工具调用甚至更早对话的文本说明。
3. **现象 C：单轮对话被腰斩，用户提问与后续执行步骤脱节**
   - 某一轮的用户提问孤立停留在中间，后续执行步骤消失或被拆分到不同区域。

这些问题并非偶发性的网络丢失，而是**系统性、确定性发生的逻辑缺陷**。

---

## 二、底层真实数据拓扑还原

直接提取本地 SQLite 数据库（`harness_mini.db`）中的真实数据，分析故障发生时的真实物理拓扑：

会话 ID：`1be981dc-4525-487b-9f78-876c46f33f96`，累计消息数：88 条。

### 2.1 倒数第二轮对话（Turn 3）拓扑
* `seq = 25` (`role: user`, `run_id: 264d392e...`)：
  > `我需要可视化的配置和可视化的功能测试界面`
* `seq = 26` (`role: assistant`, `run_id: 264d392e...`)：
  > `收到，需求升级：从命令行工具升级为可视化配置...按规范先更新计划文档...`（工具调用：`update_plan`）
* `seq = 27` (`role: tool`)：`update_plan` 成功输出。
* `seq = 28` (`role: assistant`, `run_id: 264d392e...`)：
  > `## 📋 计划已升级至 v3：可视化配置 + 可视化测试界面...`（无工具调用，最终交付回复）

### 2.2 最后一轮对话（Turn 4）拓扑
* `seq = 29` (`role: user`, `run_id: e1bb336a...`)：
  > `确认`
* `seq = 30 ~ 87`（多步 assistant 与 tool 交叉循环，`run_id: e1bb336a...`）：
  - 模型执行了多达 **25 次连续 ReAct 循环**：
    - `seq 30`：方案确认，开始编码（工具 `todo`）
    - `seq 32`：编写 requirements 与配置文件（工具 `write_file` × 3）
    - `seq 37`：编写核心 client 与 logger（工具 `write_file` × 2）
    - `seq 43`：编写 5 个独立探针脚本（工具 `write_file` × 3）
    - `seq 53`：编写 Streamlit 主界面 `app.py`（工具 `write_file`）
    - `seq 55`：修复 `app.py` 残缺表达式（工具 `edit_file`）
    - `seq 59`：编写 README 并冒烟验证（工具 `write_file`）
    - `seq 61 ~ 79`：执行 10 次语法与模块编译检查、安装依赖、启动 Streamlit、HTTP 200 验证、清理进程
    - `seq 81 ~ 87`：沉淀知识碎片、更新计划文档状态
  - 期间产生大量带有阶段汇报正文的 assistant 消息。
* `seq = 88` (`role: assistant`, `run_id: e1bb336a...`)：
  > `## ✅ 开发完成：jev-1.13-free 可视化测试工具\n\n### 交付内容...`（最终无工具答复）

> **核心矛盾**：最后一轮对话跨越了 **seq 29 到 seq 88，足足包含了 60 条物理消息**！

---

## 三、底层根因深度剖析

### 3.1 首屏小分页与单轮超长执行之间的尺寸倒挂

此前代码中，为了加快首屏打开速度，在 [`src/store.ts`](file:///f:/WorkSpace/Other/harness_mini/src/store.ts) 中配置了小分页：
```ts
export const INITIAL_MESSAGES_LIMIT = 30;
```
后端 Rust [`src-tauri/src/store.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/store.rs) 的 SQL 实现为：
```sql
SELECT ... FROM messages WHERE session_id = ?1 ORDER BY seq DESC LIMIT 30
```
读取后再执行 `msgs.reverse()`。

**这意味着**：当总消息数有 88 条时，首屏只拉取了最新的 30 条消息——即 **`seq 59` 到 `seq 88`**。
* `seq 29`（最后一轮的触发问题“确认”）根本不在首屏的 30 条之内！
* 导致最后一轮对话在刚打开时被**直接腰斩成两半**：前一半（`seq 29 ~ 58`）在数据库里未被拉取，后一半（`seq 59 ~ 88`）在界面上变成了**没有对应 User 消息的孤岛**。

---

### 3.2 `mergeSessionMessages` 数组合并的无序追加漏洞

当用户在会话之间切换，或者热更新重载时，本地内存中往往保留了旧的完整消息（`seq 1 ~ 88`）。切换会话触发 `selectSession`，拉取最新的 30 条（`seq 59 ~ 88`），然后调用 `mergeSessionMessages(local, fetched)`。

查看崩溃前的原逻辑：
```ts
// ❌ 存在严重时序漏洞的原实现
function mergeSessionMessages(local: Message[] | undefined, fetched: Message[]): Message[] {
  if (!local || local.length === 0) return fetched;
  const byId = new Map(local.map((m) => [m.id, m]));
  // 1. 先把 fetched (最新 30 条，seq 59 ~ 88) 塞入 out
  const out = fetched.map((m) => {
    const l = byId.get(m.id);
    if (!l) return m;
    return { ...m, ...l };
  });
  const fetchedIds = new Set(fetched.map((m) => m.id));
  // 2. 然后遍历 local，把不在 fetched 中的消息逐一 push 到末尾
  for (const l of local) {
    if (!fetchedIds.has(l.id)) out.push(l); // ⚠️ 致命追加
  }
  return out; // ⚠️ 完全未按 seq 重新排序！
}
```

原作者的假设是：“`local` 中比 `fetched` 多的消息，必然是前端正在流式输出、尚未落库的新消息，所以 push 到末尾”。
**但在分页场景下，这个假设完全颠倒**：
* `local` 中不在 `fetched` 里的消息，是**历史更早的旧消息**（`seq 1 ~ 58`）；
* 结果合并出的数组顺序变成了：
  $$\mathbf{[seq\ 59,\ seq\ 60,\ \dots,\ seq\ 88,\ \ seq\ 1,\ seq\ 2,\ \dots,\ seq\ 58]}$$
* **最新的 30 条消息被整体搬移到了数组最前端，前面的历史消息被扔到了末尾！**

---

### 3.3 现象一：“最后一轮对话显示到最前面”的数学推导

前端渲染组件 [`src/components/ExecutionProcessBlock.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/ExecutionProcessBlock.tsx) 的 `groupTimelineItems` 采用的是线性扫描状态机：
1. 遇到 `assistant` 消息：放入 `currentAssistantSteps` 暂存数组；
2. 遇到 `user` 消息：触发 `flushAssistantSteps`，将暂存的 assistant 步骤折叠打包为一个 `<ExecutionProcessBlock>`（执行过程卡片），最后一步如果是纯文本则提出来作为最终回复，随后输出 user 消息。

当数组顺序为 `[seq 59..88, seq 1..58]` 时：
1. 遍历第 0~29 项（`seq 59 ~ 88`）：全部是 `assistant` 消息；
2. 数组开头没有任何 `user` 消息，`currentAssistantSteps` 一路收集了 `seq 59 ~ 86` 以及 `seq 88`（最终回复）；
3. 遇到第 30 项 `seq 1`（第 1 轮的 user 提问）时，立即触发 `flushAssistantSteps`：
   - `seq 59 ~ 86` 被打包成了第 0 项的执行抽屉！
   - `seq 88`（最后一轮的“开发完成交付总结”）被提取出来作为第 1 项的助理回复！
4. 紧接着第 2 项才渲染 `seq 1`（第 1 轮提问）！
**数学证明与界面现象完全契合：最后一轮的成果直接被画在了全会话的最顶端！**

---

### 3.4 现象二：“前几轮回复混入最后一轮执行过程”的推导

当最后一轮执行完毕后，用户发送了第 5 轮新提问（例如 `seq 89`）：
* 内存中当前列表为 `[seq 59 ~ 88, seq 89]`；
* 触发会话切换或事件重新同步，后端拉取最新 30 条，返回 `[seq 60 ~ 89]`；
* `mergeSessionMessages` 运行：
  - `fetched` 为 `[seq 60 ~ 89]`；
  - `local` 中的 `seq 59`（上一轮带有正文“现在编写 README 并进行冒烟验证：”的旧步骤）不在 `fetched` 中，**被 `out.push` 到了末尾**；
  - 数组变为：`[seq 60 ~ 89, seq 59]`；
* 渲染层扫描时：
  - 先渲染了最新提问 `seq 89`；
  - 紧接着在 `seq 89` 后面遇到了 `seq 59`（旧 assistant 步骤）；
  - 状态机误以为 `seq 59` 是紧随提问 `seq 89` 之后到达的执行步骤，**直接将前序轮次的汇报塞进了当前最新轮次的执行过程或回复中**！

---

### 3.5 全链路缺乏 `seq` 单调递增不变量守护

排查发现，在整个前端状态层中：
* `mergeSessionMessages`：未排序，直接追加；
* `upsertMessage`：未排序，未命中时直接 `[...list, m]` 尾部追加；
* `loadEarlier`：未排序，直接 `[...earlier, ...msgs]`；
* `onMessageDelta`：流式占位符赋予 `Number.MAX_SAFE_INTEGER`，但转正后未重新按真实 `seq` 归位；
* `ChatView.tsx`：直接信任 `msgs`，直接映射渲染；
* `currentMessages` 选择器：透传数组，无兜底防护。

**系统对“时序单调递增”没有任何防御性契约，任何一个细小分支的异步乱序，都会导致永久性的状态污染。**

---

### 3.6 抽屉聚合状态机对物理顺序的脆弱依赖

`groupTimelineItems` 仅仅依赖 `item.msg.role === 'user'` 来划分轮次抽屉。它缺乏业务维度的强隔离标识（如 `run_id`）：
* 一旦因网络截断或分页丢失了中间某条 `user` 消息，两个不同轮次的 assistant 消息就会因为角色连续而被**合并到同一个抽屉里**。

---

## 四、业内标杆实践对比与架构启示

### 4.1 Claude Code / Cursor：Turn/Run 作为一等公民
在先进的 Agent 系统设计中（如 Cursor Composer、Claude Code）：
* **Turn（轮次）/ Run（执行周期）是业务实体的一等公民（First-Class Entity）**；
* 消息从来不是零散平铺存储的，而是归属于特定的 `RunID` 或 `TurnID`；
* 状态机在聚合步骤时，以 `RunID` 为绝对隔离边界，而非单纯依赖 `role === "user"`。

### 4.2 VS Code Chat：单向单调 Sequence 不变量
在 VS Code Interactive Editor 及 Chat 架构中：
* 任何进入 Session 树的数据结构，在 Reducer/Store 层必须强制经过序列重排算法（Sequence Reconciliation）；
* 无论后端是批量返回、增量推流还是分片历史拉取，进入视图树前必须经过 $O(N \log N)$ 或有序插入维护单调不变量。

### 4.3 为什么 Agent 场景不能按固定消息条数硬切分页
* 传统 IM 中，一条消息即一个完整语义块；
* 在 Agent 场景中，一个简单的操作可能产生数十条工具调用与结果。**一整轮执行（Run）才是不可分割的语义单元**。
* 首屏限制若小于单轮执行跨度，必然导致“无头执行”或“截断残卷”。首屏拉取应足够覆盖常见大轮次（如 100 条），并在后端支持按轮次对齐（Turn-Aligned）。

---

## 五、完整治理与工程落地实施方案

基于上述根因，系统实施了纵深防御的四层治理架构：

```
┌────────────────────────────────────────────────────────┐
│  Layer 1: 时序不变量核心 (sortMessages Invariant)       │
│  - 严格以 seq 升序排列                                 │
│  - 流式占位符 (MAX_SAFE_INTEGER) 稳定置于最末端         │
└───────────────────────────┬────────────────────────────┘
                            │ 约束
┌───────────────────────────▼────────────────────────────┐
│  Layer 2: 状态存储与合并层 (Store Reconciliation)       │
│  - mergeSessionMessages: 双向 Map 去重合并 + 强制排序   │
│  - upsertMessage / loadEarlier: 杜绝乱序追加           │
│  - INITIAL_MESSAGES_LIMIT: 从 30 扩容至 100 条          │
└───────────────────────────┬────────────────────────────┘
                            │ 约束
┌───────────────────────────▼────────────────────────────┐
│  Layer 3: 视图选择器防御层 (Defensive Selector)         │
│  - currentMessages(s): 统一经过 sortMessages 输出       │
└───────────────────────────┬────────────────────────────┘
                            │ 约束
┌───────────────────────────▼────────────────────────────┐
│  Layer 4: 抽屉聚合与指标隔离 (Timeline & Run Isolation) │
│  - computeTurnMetrics: 引入 runId 跨轮隔离              │
│  - groupTimelineItems: 发现不同 runId 强制结算前序抽屉 │
└────────────────────────────────────────────────────────┘
```

### 5.1 核心排序工具函数 `sortMessages`
在 [`src/types.ts`](file:///f:/WorkSpace/Other/harness_mini/src/types.ts#L555-L565) 建立基础数学约束：
```ts
/**
 * 确保消息列表严格单调递增排序（按 seq 升序；占位符按创建时间稳定置于末尾）。
 */
export function sortMessages(msgs: Message[]): Message[] {
  return [...msgs].sort((a, b) => {
    if (a.seq !== b.seq) return a.seq - b.seq;
    const tA = a.createdAt ? new Date(a.createdAt).getTime() : 0;
    const tB = b.createdAt ? new Date(b.createdAt).getTime() : 0;
    return tA - tB;
  });
}
```

### 5.2 重构 `mergeSessionMessages`：双向去重与属性取精
在 [`src/store.ts`](file:///f:/WorkSpace/Other/harness_mini/src/store.ts#L459-L485) 彻底推翻原先的错误尾部 push：
```ts
function mergeSessionMessages(local: Message[] | undefined, fetched: Message[]): Message[] {
  if (!local || local.length === 0) return sortMessages(fetched);
  const byId = new Map(local.map((m) => [m.id, m]));
  const mergedMap = new Map<string, Message>();

  // 1. 先合入 fetched，同 ID 消息取两端最新属性
  for (const m of fetched) {
    const l = byId.get(m.id);
    if (!l) {
      mergedMap.set(m.id, m);
    } else {
      mergedMap.set(m.id, {
        ...m,
        content: (l.content?.length ?? 0) >= (m.content?.length ?? 0) ? l.content : m.content,
        reasoning: (l.reasoning?.length ?? 0) >= (m.reasoning?.length ?? 0) ? l.reasoning : m.reasoning,
        toolEvents: mergeToolEvents(l.toolEvents, m.toolEvents),
        toolCalls: (m.toolCalls?.length ?? 0) > 0 ? m.toolCalls : l.toolCalls,
        revertedAt: m.revertedAt ?? l.revertedAt,
      });
    }
  }

  // 2. 将 local 中独有的消息合入（无论是历史旧消息还是正在流式的新消息）
  for (const l of local) {
    if (!mergedMap.has(l.id)) {
      mergedMap.set(l.id, l);
    }
  }

  // 3. 强制按 seq 严格升序归位输出，杜绝任何物理数组错位
  return sortMessages(Array.from(mergedMap.values()));
}
```

### 5.3 数据入队全链路时序守护（Upsert / Delta / Reset）
* **`upsertMessage`**：更新或追加后通过 `sortMessages` 返回；
* **`upsertToolEvent`**：创建占位符时通过 `sortMessages` 返回；
* **`onMessageDelta` / `onMessageReasoningDelta`**：创建流式占位符时通过 `sortMessages` 插入；
* **`loadEarlier`**：改用 `mergeSessionMessages(st.messages[id], earlier)` 替代简单的数组解构拼接。

### 5.4 选择器防御性排序（Defensive Selector）
在 [`src/store.ts`](file:///f:/WorkSpace/Other/harness_mini/src/store.ts#L2850-L2856) 中：
```ts
export function currentMessages(s: Store): Message[] {
  if (!s.currentId) return [];
  const list = s.messages[s.currentId];
  if (!list || list.length === 0) return [];
  return sortMessages(list); // 渲染前终极防御
}
```

### 5.5 首屏分页扩容与 `loadEarlier` 归位
* 将 `INITIAL_MESSAGES_LIMIT` 和 `LOAD_MORE_LIMIT` 从 30 调整为 **100**。
* 此前 Rust 后端已经在 [`src-tauri/src/store.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/store.rs) 实现了 `tool_events_for_batch`（批量 IN 查询），拉取 100 条消息仅需 1~2 次批量查询，耗时仅几毫秒，不仅完全消除了性能顾虑，而且保证首屏能够完整容纳包含 60 步的长任务轮次。

### 5.6 引入 `run_id` 双重隔离守护抽屉边界
在 [`src/components/ExecutionProcessBlock.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/ExecutionProcessBlock.tsx#L210-L225) 与 [`src/types.ts`](file:///f:/WorkSpace/Other/harness_mini/src/types.ts#L640-L655) 中：
```ts
} else if (item.msg.role === "assistant") {
  // 检查 runId 边界：若新步骤与当前累积步骤的 runId 明确不同，强制切断并结算前序抽屉，杜绝跨轮次混合
  if (
    currentAssistantSteps.length > 0 &&
    currentAssistantSteps[0].runId &&
    item.msg.runId &&
    currentAssistantSteps[0].runId !== item.msg.runId
  ) {
    flushAssistantSteps(false);
  }
  currentAssistantSteps.push(item.msg);
}
```

---

## 六、仿真验证与修复效果对比

使用当前出现严重错乱的真实数据库会话（88 条消息）进行前后仿真对比：

### 修复前（错误发生时）
```text
[ 0] PROCESS BLOCK (steps seqs=[59, 61, 63, 65, 67, 69, 71, 73, 75, 77, 79, 81, 84, 86]) ❌ 最后一轮步骤跳到顶部
[ 1] ASSISTANT (seq=88): ## ✅ 开发完成：jev-1.13-free 可视化测试工具                      ❌ 最后一轮交付回复跳到顶部
[ 2] USER (seq=1): 我需要实现一个简单的jev对话测试工具...
[ 3] PROCESS BLOCK (steps seqs=[2, 5])
[ 4] ASSISTANT (seq=7): ## 📋 需求分析与实现方案...
[ 5] USER (seq=8): 先了解jev的能力再和opencode相关调用方法，再来完善计划文档
[ 6] PROCESS BLOCK (steps seqs=[9, 12, 15, 18, 20, 22])
[ 7] ASSISTANT (seq=24): ## 📋 调研完成，计划文档已按事实纠偏重设计（v2）...
[ 8] USER (seq=25): 我需要可视化的配置和可视化的功能测试界面
[ 9] PROCESS BLOCK (steps seqs=[26])
[10] ASSISTANT (seq=28): ## 📋 计划已升级至 v3：可视化配置 + 可视化测试界面...
[11] USER (seq=29): 确认                                                                 ❌ 用户提问
[12] PROCESS BLOCK (steps seqs=[30, 32, 37, 40, 43, 47, 51, 53, 55, 57])                ❌ 最后一轮被腰斩，且无最终答复
```

### 修复后（严格单调递增与时序自愈）
```text
[ 0] USER (seq=1): 我需要实现一个简单的jev对话测试工具...                                 ✅ Turn 1
[ 1] PROCESS BLOCK (steps=2 steps, seq range [2..5])
[ 2] ASSISTANT (seq=7): ## 📋 需求分析与实现方案...
[ 3] USER (seq=8): 先了解jev的能力再和opencode相关调用方法，再来完善计划文档              ✅ Turn 2
[ 4] PROCESS BLOCK (steps=6 steps, seq range [9..22])
[ 5] ASSISTANT (seq=24): ## 📋 调研完成，计划文档已按事实纠偏重设计（v2）...
[ 6] USER (seq=25): 我需要可视化的配置和可视化的功能测试界面                              ✅ Turn 3
[ 7] PROCESS BLOCK (steps=1 steps, seq range [26..26])
[ 8] ASSISTANT (seq=28): ## 📋 计划已升级至 v3：可视化配置 + 可视化测试界面...
[ 9] USER (seq=29): 确认                                                                 ✅ Turn 4
[10] PROCESS BLOCK (steps=24 steps, seq range [30..86])                                  ✅ 24 步执行完整聚合在提问之后
[11] ASSISTANT (seq=88): ## ✅ 开发完成：jev-1.13-free 可视化测试工具                    ✅ 最终交付成果完美收尾
```

* **编译验证**：运行 `npm run build` (`tsc && vite build`)，**0 错误通过**。

---

## 七、代码防劣化规范与工程化约束

为防止后续业务迭代中再次引入时序混乱，制定如下防劣化开发红线：

1. **红线 1：禁止直接操作 `messages[sessionId]` 进行无序追加**
   - 严禁书写 `[...list, m]` 或 `[...earlier, ...msgs]` 后直接存入 store；
   - 任何消息集合变动必须通过 `sortMessages` 或 `mergeSessionMessages` 进行规整。
2. **红线 2：分页大小不得低于单轮长任务预期步数**
   - 严禁随意将首屏分页下调至低于 50 条；若需极低内存优化，必须引入虚拟滚动列表（Virtual List），而非物理截断。
3. **红线 3：抽屉与指标计算必须双重检查 `run_id`**
   - 严禁单纯依靠 `role === 'user'` 作为不可动摇的分界线；必须时刻以 `run_id` 物理边界作为状态机的第二重保护。
4. **红线 4：占位符生命周期闭环**
   - 流式占位符必须保持 `seq = Number.MAX_SAFE_INTEGER`；
   - 收到 `message:final` 后必须原位替换或重排，严禁残留未销毁的占位符。
