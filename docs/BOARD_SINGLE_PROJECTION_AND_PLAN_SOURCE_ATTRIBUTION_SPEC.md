# 最新对话看板单一投影与方案归属固化治理规范
# Board Single Projection & Plan Source Attribution Governance Specification (Latest-Board-Only, Zero-Fallback)

本文档系统性总结 `harness_mini` 中 **任务与计划看板浮窗（`FloatingTaskPanel`）** 从「双真相源 + 启发式归属猜测 + 多兜底分支 + 分页翻页」演进为「**可见消息流单一投影 + 归属写入时固化 + 仅展示末项**」的完整重构过程，涵盖问题诊断、目标架构、前后端落地细节、取舍决策记录、行为契约与后续维护准则。

> **取代关系**：本文档**取代** `docs/MULTI_TASK_TIMELINE_AND_FLOATING_PANEL_GOVERNANCE_SPEC.md` 中关于「统一任务时间线 + 多方案共存翻页浏览 + 四维匹配正向联动」的实现约定（该文档第三章部分、第五章 3.1、第六章第 3 节）。旧的翻页器、选中态与四路模糊匹配已从代码中删除，后续如需回看历史看板，应以本文档第九章「后续维护准则」为准重新设计。

---

## 目录
- [一、问题诊断：复杂度是结构性的](#一问题诊断复杂度是结构性的)
  - [1. 根因一：归属事实在写入时丢失](#1-根因一归属事实在写入时丢失)
  - [2. 根因二：同一事实存多副本，靠补偿函数对齐](#2-根因二同一事实存多副本靠补偿函数对齐)
  - [3. 根因三：异质对象混合成一条时间线](#3-根因三异质对象混合成一条时间线)
  - [4. 结论：堆叠式修补 vs 收口式重构](#4-结论堆叠式修补-vs-收口式重构)
- [二、设计目标与硬约束](#二设计目标与硬约束)
- [三、目标架构：单一投影（Single Projection）](#三目标架构单一投影single-projection)
  - [1. 核心不变量与数据流](#1-核心不变量与数据流)
  - [2. 投影规则（唯一规则集）](#2-投影规则唯一规则集)
  - [3. 撤回、会话隔离为何"天然成立"](#3-撤回会话隔离为何天然成立)
- [四、后端治理：归属在写入时固化（Rust / Tauri）](#四后端治理归属在写入时固化rust--tauri)
  - [1. 数据结构扩展与兼容策略](#1-数据结构扩展与兼容策略)
  - [2. 写入点与"首次归因"语义](#2-写入点与首次归因语义)
  - [3. 工具上下文的归属注入链（关键前提）](#3-工具上下文的归属注入链关键前提)
  - [4. 撤回校准：只认事实，净减 3 处猜测](#4-撤回校准只认事实净减-3-处猜测)
  - [5. 历史方案一次性归属回填（backfill_plan_sources）](#5-历史方案一次性归属回填backfill_plan_sources)
- [五、前端重构：投影 + 末项渲染（React / TypeScript）](#五前端重构投影--末项渲染react--typescript)
  - [1. 唯一投影函数 deriveBoards](#1-唯一投影函数-deriveboards)
  - [2. 展示层收敛为"末项"](#2-展示层收敛为末项)
  - [3. 会话级 todos（sessionTodos）死代码链路清理](#3-会话级-todossessiontodos死代码链路清理)
- [六、取舍决策记录（Decision Log）](#六取舍决策记录decision-log)
  - [1. 撤回后：回退上一条 vs 留空](#1-撤回后回退上一条-vs-留空)
  - [2. 归属为空：不显示 vs 兜底显示](#2-归属为空不显示-vs-兜底显示)
  - [3. 撤回补偿逻辑：为什么不删](#3-撤回补偿逻辑为什么不删)
- [七、行为契约与边界场景](#七行为契约与边界场景)
- [八、工程验证](#八工程验证)
- [九、后续维护准则（How to Extend / What Not to Do）](#九后续维护准则how-to-extend--what-not-to-do)

---

## 一、问题诊断：复杂度是结构性的

改造前的看板实现并非"某处写得不够好"，而是**模型失真导致的系统性复杂度**：真相源分裂、归属事实丢失、异质对象混合。三者叠加，迫使开发者用一层又一层兜底去弥合矛盾。

### 1. 根因一：归属事实在写入时丢失

方案（Plan）的元数据 `PlanMeta`（`src-tauri/src/plan.rs`）原先只有 `id / title / status / created_at / updated_at / version / session_id`，**没有记录"本方案由哪条消息、哪个工具事件产生"**。

于是前端只能事后**猜**：用 `title`、`plan_id`、`filename`、`result_text` 四路模糊匹配反查归属，并额外挂了一条 `plans.length === 1` 的无条件兜底。猜测正确率随方案数量增加而下降，且在代码层面不可判定——这是所有兜底分支的源头。

### 2. 根因二：同一事实存多副本，靠补偿函数对齐

- 真相本是消息流中的工具事件，但会话级 `session_kv` 里另存了一份 `todos` 物化快照；撤回后用 `store::sync_session_todos_after_revert` 通过 SQL 反查"最近一个未撤回的 todo 事件"**重建**它。
- 方案状态存在磁盘 `.harness/plans/*.md` 的 Frontmatter 中，撤回后又用 `plan::sync_session_plans_after_revert` 反推校准 `active_plan_id`。

**每多一个副本，就要多写一段补偿逻辑**。这是"堆叠"的机制性来源，而非编码风格问题。

### 3. 根因三：异质对象混合成一条时间线

方案是"有生命周期、有文件、有版本的持久实体"，todo 是"每轮对话的快照"。二者被塞进同一条 `timeline` 并按 `createdAt` 字符串升序排序后，下游被迫衍生出：选中态（`selectedTaskId`）、翻页器（`currentIndex + 1 / timeline.length`）、聚焦匹配（3 次重试 + 主动 `loadPlans` 兜底）、失效选中校正 effect。

**下游复杂度全部来自上游建模失真。**

### 4. 结论：堆叠式修补 vs 收口式重构

| | 堆叠式（在旧 timeline 上加"只取末项"） | 收口式（单一投影）✅ |
|---|---|---|
| 改动面 | 前端 1 文件 | 前端 1 + 后端 2~3 |
| 分支数变化 | **+2~3 处判定**，与既有 5 处兜底相互作用 | 删除 5 处兜底 + 1 翻页器 + 1 选中 effect + 1 聚焦重试 |
| 撤回残留 Bug | 不解决，仅被"取末项"掩盖 | 结构性消失 |
| 未来要回看历史看板 | 又要重建下标与选择态 | 就是 `boards[index]`，天然具备 |
| 长期趋势 | 每个新需求再叠一层 | 新需求＝改一条投影规则 |

---

## 二、设计目标与硬约束

**业务目标**：任务与计划看板浮窗只展示**当前会话最后一条对话**产生的看板，取消分页模式；撤回最后一条对话后回退展示上一条有效看板，整个会话无有效看板时留空；新对话出现后展示新看板；**每个会话各自独立、互不影响**。

**硬约束（贯穿全部改动）**：

1. **净减分支**：每一次调整必须让某个兜底分支或启发式消失，严禁在既有兜底之上叠加新判定。
2. **单一真相源**：可见消息流是唯一事实来源，看板是其纯函数投影，不得引入第二份"独立实体集合"。
3. **事实前置**：能在写入时固化的信息（归属），绝不留给运行时反查猜测。
4. **缺失即缺失**：无法确定的事实保持为空，不做任何猜测式填充。

---

## 三、目标架构：单一投影（Single Projection）

### 1. 核心不变量与数据流

```
可见消息流（!revertedAt && !queued && role !== "tool"）
        │  每条消息携带 toolEvents（todo 快照 / create_plan / update_plan / switch_plan）
        │  方案清单（PlanSummary，含 source_message_id / source_tool_event_id）
        ▼
   deriveBoards(visibleMsgs, plans, ctx)        ← 唯一投影函数（纯函数，零兜底）
        ▼
   boards[]（按来源消息 seq 升序）
        ▼
   currentTask = boards[boards.length - 1]      ← 展示层唯一取值方式
```

**不变量**：看板不存在任何独立于消息流的状态；撤回、新增、切换会话都只是"改变投影输入"，输出自动随之变化。

### 2. 投影规则（唯一规则集）

| 看板类型 | 归属判定 | 状态映射 | 排序依据 |
|---|---|---|---|
| **todo** | 按 `role === "user"` 划分轮次，取该轮**最后一个** `params.todos` 非空的 `todo` 工具事件快照 | 运行中→保留原状态；异常停止且为末轮→`in_progress` 降为 `pending`；正常结束或历史轮次→未完成项视为 `done` | 该轮末尾消息的 `seq` |
| **plan** | **`source_message_id` 字段直取**；在该消息中按 `source_tool_event_id` 关联事件。**归属为空即丢弃**；来源消息已撤回/不可见也丢弃 | 直接取方案自身 `status` | 来源消息的 `seq` |

排序统一使用消息 `seq`（`Board.sourceSeq`），**彻底弃用 `createdAt` 字符串比较**，消除时区差异与同秒相等的抖动。同 `seq` 时 `plan` 优先于 `todo`。

### 3. 撤回、会话隔离为何"天然成立"

- **撤回**：消息被软标记 `revertedAt` → 从投影输入中过滤掉 → 该消息产生的看板自然消失 → 数组末项自动变成上一条有效看板。**无需任何补偿分支**（这正是"方案 B：回退上一条"零成本获得的原因）。
- **会话隔离**：投影输入本就是"当前会话的消息流 + 当前会话的方案清单"（`sessionPlans[currentId]`），切换会话即更换输入，天然互不影响；`PlanSummary.session_id` 仅用于父子会话/协作者的**关联边界**校验。

---

## 四、后端治理：归属在写入时固化（Rust / Tauri）

### 1. 数据结构扩展与兼容策略

`src-tauri/src/plan.rs`：

```rust
pub struct PlanMeta {
    // ...既有字段（id / title / status / created_at / updated_at / version / session_id）
    /// 归属事实：本方案的来源助手消息 ID（写入时固化，老文件缺失即为 None，不做任何猜测回填）
    #[serde(default)]
    pub source_message_id: Option<String>,
    /// 归属事实：本方案的来源工具事件 ID（写入时固化，老文件缺失即为 None，不做任何猜测回填）
    #[serde(default)]
    pub source_tool_event_id: Option<String>,
}
```

- `PlanSummary` 同步增加同名 `#[serde(default)]` 字段，供 `list_plans` 输出到前端。
- `normalize_source()`：**仅裁剪空白，空串一律视为 `None`**（"没有事实就不写"）。
- `format_with_frontmatter()`：**仅在 `Some` 时落盘该字段**，`None` 完全不写，保证老文件形态不被污染。
- `parse_frontmatter()` 兼容老文件（缺字段 → `None`），所有 `parse → format` 重写路径（挂起、步骤勾选、撤回校准）均保留已有归属。

### 2. 写入点与"首次归因"语义

| 函数 | 行为 |
|---|---|
| `create_plan_with_effect(..., source_message_id, source_tool_event_id, conn)` | 创建时直接把来源写入 Frontmatter |
| `update_plan_with_effect(..., source_message_id, source_tool_event_id, conn)` | **仅在原值为空时补写**本次来源 |
| `switch_plan(..., source_message_id, source_tool_event_id, conn)` | 同上，"首次归因不被覆盖" |

"首次归因"语义的价值：方案首次被创建的那条对话，才是它真正的业务归属；后续更新不应把归属改成最后修改者，否则"看板跟随对话轮次"会漂移。

### 3. 工具上下文的归属注入链（关键前提）

归属依赖 `ToolCtx` 的两个字段（`src-tauri/src/tools.rs`）：

```rust
pub struct ToolCtx {
    // ...
    /// 当前正在执行的工具事件 ID
    pub event_id: Option<String>,
    /// 当前正在执行的助手消息 ID
    pub message_id: Option<String>,
}
```

**注意**：`agent.rs` 首次构造 `ToolCtx` 时两者均为 `None`，而在**工具真正执行前**注入：

```rust
// agent.rs:2893-2897
let mut call_ctx = ctx.clone();
call_ctx.event_id = Some(ev.id.clone());
call_ctx.message_id = Some(assistant_msg_id.to_string());
let exec = tools::execute(tool_name, args, &call_ctx, &on_partial).await;
```

**结论**：正常对话执行流中的 `create_plan / update_plan / switch_plan` **必然携带归属** ✅。
**例外**：由中枢在意图对齐阶段直接调用的方案工具（无助手消息/工具事件上下文）归属为空 → 该方案不进入看板。此为已知边界，不是缺陷；如未来需要覆盖，应在此类调用路径上补齐上下文，而非在前端加兜底。

### 4. 撤回校准：只认事实，净减 3 处猜测

`sync_session_plans_after_revert`（撤回/重做后校准 `active_plan_id`）**只认两类事实**：

1. 方案已固化的 `source_tool_event_id` / `source_message_id`（唯一命中才采信）；
2. 工具显式入参 `plan_id`。

**无匹配即置空，无任何正则解析、slug 推导或磁盘模糊兜底。**

已删除的 3 处猜测逻辑：

| 删除项 | 原用途 | 危害 |
|---|---|---|
| `create_plan` 的 `result_text` 正则解析（`活动计划ID: \`plan-xxx\``） | 从工具返回文本里抠出 plan id | 依赖自然语言文案，文案一变即失效 |
| `title → sanitize_slug()` 推导 | 用标题猜文件名 | 标题重名/改动即误归属 |
| 磁盘方案目录"取最新一个"模糊兜底 | 找不到归属时随便挂靠 | **唯一的误归属源头**，会把看板挂到错误对话 |

### 5. 历史方案一次性归属回填（backfill_plan_sources）

```rust
plan::backfill_plan_sources(conn, workspace, session_id, include_archived, apply) -> PlanBackfillReport
```

| 规则 | 说明 |
|---|---|
| 处理范围 | **仅**两个归属字段均为空的方案文件 |
| 事实边界 | 严格限定同一 `session_id`、消息未撤回（`reverted_at IS NULL`）、事件 `status = success` |
| 强路径 | 事件日期 + `sanitize_slug(title)` 拼出的文件名与磁盘文件名**精确一致且唯一命中** |
| 弱路径 | `title` 全等且 slug 与文件名 slug 一致，且**唯一命中** |
| 多命中 | 一律置为 `ambiguous`，**保持为空不写**（绝不猜测） |
| 幂等 | "非空即跳过"，重复执行为 no-op；不触碰 `updated_at` |
| dry-run | `apply = false` 只产出报告，不改盘 |

报告字段：`scanned / already_attributed / written / unattributable / ambiguous`，逐条给出 `decision` 与 `matched_by`（`filename` / `title`）。

> 本项目工作区实测：`scanned=2, written=0, unattributable=2, ambiguous=0`（数据库内无对应 `create_plan` 事件，安全 no-op）。

---

## 五、前端重构：投影 + 末项渲染（React / TypeScript）

### 1. 唯一投影函数 deriveBoards

`src/components/boardProjection.ts`（约 165 行，含注释）：

```ts
export interface Board extends TimelineTask {
  /** 来源消息的 seq：投影排序的唯一依据（弃用 createdAt 字符串比较） */
  sourceSeq: number;
}

export function deriveBoards(msgs: Message[], plans: PlanSummary[], ctx?: ProjectionContext): Board[]
```

- `Board` 继承既有 `TimelineTask`，因此浮窗内部所有展示逻辑（步骤渲染、勾选、定位消息、折叠）**复用不变**。
- `ProjectionContext = { isRunning?, lastRunOutcome?, relatedSessionIds? }`：仅决定条目**展示状态**，不参与归属判定。
- plan 归属的全部逻辑收缩为 4 行：

```ts
// 归属为空（null）的方案不进入看板：老方案文件因缺少归属字段而不显示，
// 这是已知的一次性迁移代价（由后端另行回填），此处不加任何兜底或猜测。
if (!p.source_message_id) continue;
const source = visible.find((m) => m.id === p.source_message_id);
// 来源消息已被撤回/不可见 -> 该方案不显示（撤回后展示自动回退到上一条有效看板）
if (!source) continue;
```

**改造成效**：`FloatingTaskPanel.tsx` 中原先 **3 处 useMemo 共 264 行**（`todoTasks` + `planTasks` + `timeline`）收口为**单个 8 行投影 memo**。

### 2. 展示层收敛为"末项"

```ts
// 展示层只渲染投影末项：唯一看板，无选择态、无翻页
const currentTask: Board | null =
  timeline.length > 0 ? timeline[timeline.length - 1] : null;
```

**同时删除**：

| 删除项 | 原位置 | 说明 |
|---|---|---|
| 翻页器（◀ n/m ▶） | 面板操作栏 | 连带 `currentIndex` 与 `N 阶段` 徽标 |
| `selectedTaskId` 选中态 + 失效校正 effect | 组件状态与 effect | 不再存在"多项要选"的问题 |
| `focusFloatingTaskId` 的 3 次重试 + 主动 `loadPlans` + 超时兜底展开 | 聚焦 effect | 收敛为"命中当前显示项才高亮，否则忽略并清空指令" |
| `TimelineTask` 四路模糊匹配 | 原 `planTasks` | 归属改为字段直取 |

> 历史看板的"浮窗查看跳转"能力随之下线；信息不丢失——每条对话消息内的工具卡片仍内联渲染完整清单。

### 3. 会话级 todos（sessionTodos）死代码链路清理

投影改造后，前端对 `sessionTodos` 的唯一读取仅服务于"todo 步骤勾选写回"，而该写回**已无任何可见效果**（看板数据全部来自消息流的 todo 快照）。因此整条链路被判定为死代码并清理：

| 删除项 | 数量 |
|---|---|
| Zustand 状态字段与 `onSessionTodos` 事件监听 | 2 |
| store 中的拉取调用链 | 1 |
| `ipc.getSessionTodos` 包装 | 1 |
| `TodoItem` 类型与 `EMPTY_TODOS` 常量 | 1 |
| 写回/兜底分支 | 6 |
| todo 步骤勾选写回分支 | 1 |

**处理方式**：todo 看板的步骤勾选改为**只读展示**（点击态自动移除）；`plan` 的步骤勾选（`update_plan_step_status`）行为不变。清理后 `src` 下 `sessionTodos` / `TodoItem` 引用归零。

**后端未受影响**：`get_session_todos` 命令、`session:todos` 事件广播、`session_kv.todos`、`sync_session_todos_after_revert` **全部保留**（它们是 `auto_finish_session_todos` 的输入链，与前端是否拉取无关）。

---

## 六、取舍决策记录（Decision Log）

### 1. 撤回后：回退上一条 vs 留空

**采用：回退上一条。**

- todo 侧的"回退"是纯函数推导的**免费副产品**（撤回后剩余末项自动成为最新）；选择"留空"反而要额外删除/破坏多处兜底并新增否定式判定。
- plan 侧后端 `sync_session_plans_after_revert` 本就是"回退到上一个有效方案"的语义，前端"取末项"天然跟随。
- 稳定性本质："取末项"是**正向派生**（单调、幂等、无状态）；"撤回即留空"是**存在性判定**，判定链越长越易漏。
- 真正不可接受的是"显示**被撤回那条**对话的看板"——这一点由投影机制从根本上杜绝。

### 2. 归属为空：不显示 vs 兜底显示

**采用：不显示（用户确认）。**

- 保持零兜底；代价是无法回填的老方案在浮窗永久不可见（一次性迁移成本，方案文件与消息内联卡片不受影响）。
- 若未来改为"显示但以方案自身时间排序"，需明确其为**数据状态的真实表达**（nullable 来源），而非归属猜测，不能引入任何推断逻辑。

### 3. 撤回补偿逻辑：为什么不删

**采用：保留函数，净减其中的猜测分支。**

| 补偿项 | 消费者证据 | 结论 |
|---|---|---|
| `session_kv.todos` / `sync_session_todos_after_revert` | `agent.rs` 的 `auto_finish_session_todos`（其唯一输入）、`stop_session_todos`、`commands::get_session_todos` | **保留**（风险中低） |
| `active_plan_id` / `sync_session_plans_after_revert` | System Prompt 注入路径（`agent.rs` 注入 `load_active_plan_context` 结果）、`find_plan_file`、多个 commands | **必须保留**（风险高，不可删） |

即："净减"针对的是函数内部的**猜测分支**，而不是函数本身——避免以"简化"为名破坏撤回语义。

---

## 七、行为契约与边界场景

| 场景 | 契约行为 |
|---|---|
| 进入会话，末条对话产出过看板 | 展示该看板（todo 或 plan） |
| 末条对话被撤回 | 自动回退展示上一条有效看板；**绝不展示被撤回那条** |
| 整个会话无可展示看板 | 浮窗隐藏（保持 `!currentId \|\| (!hasTasks && !hasCmds)` 语义） |
| 撤回全部至会话开头 | 浮窗隐藏 |
| 出现新对话并产出看板 | 立即展示新看板 |
| 切换会话 | 展示目标会话**自己**的看板；目标会话无看板则隐藏 |
| 同一轮内既有 plan 又有 todo | 取 `sourceSeq` 更晚者；`seq` 相同时 plan 优先 |
| 方案归属为空（老文件 / 中枢直调创建） | 不进入看板（不显示） |
| 方案来源消息被单独撤回 | 该方案不显示 |
| todo 步骤勾选 | 只读（不再写回会话级状态） |
| plan 步骤勾选 | 可切换，且持久化（`update_plan_step_status`） |
| 历史看板"浮窗查看" | 命中的若是当前展示项则高亮；否则忽略（不跳转、不重试） |
| 只读工具卡片点击"浮窗查看" | 依赖 `focusFloatingTaskId`，行为同上 |

---

## 八、工程验证

| 验证项 | 命令 | 结果 |
|---|---|---|
| 前端类型检查 | `npx tsc --noEmit` | exit 0 |
| 前端完整构建 | `npm run build`（`tsc && vite build`） | exit 0 |
| 后端编译 | `cargo check --manifest-path src-tauri/Cargo.toml` | 通过（仅历史 `dead_code` 警告） |
| 后端单测 | `cargo test --lib --manifest-path src-tauri/Cargo.toml` | **161 passed; 0 failed**（`plan::tests` 15 个，新增 6 个） |

新增单测覆盖：Frontmatter 归属字段往返解析、老文件缺字段兼容、空串视为 `None`、首次归因不被覆盖、老文件在后续更新时补写、回填唯一命中写回 / 多命中不写 / 幂等 / 老文件无法归属、撤回校准无事实则置空。

**执行环境注意（Windows 工具环境）**：
- 不支持 `cd /d` 与管道 `|`；应直接在工作区根执行命令。
- `>` 重定向可能写出 UTF-16 文件导致不可读，需用 `powershell -NoProfile -Command "... | Out-File -Encoding utf8 x.txt"`。
- PowerShell 会把 cargo 的 stderr 警告视为 `NativeCommandError` 而返回 exit 1，**exit 1 不代表测试失败，必须查看 `test result:` 行**。

---

## 九、后续维护准则（How to Extend / What Not to Do）

### 新增一类看板（例如"运行中命令卡片"）时的正确做法
1. 在 `boardProjection.ts` 中新增一个 `xxxBoards(visible, ...)` 私有函数，返回 `Board[]`；
2. 在 `deriveBoards` 的合并与排序处接入（补 `sourceSeq`）；
3. 展示层无需改动——它只渲染末项。

### 新增需求时的自检清单
- [ ] 是否引入了一个**新的分支**而没有消除任何旧分支？若是，重新设计。
- [ ] 是否在"反查/推断"某个事实？若是，改为**在写入时固化**该事实。
- [ ] 是否想加"找不到就兜底用 XX"的逻辑？禁止——**缺失即缺失**。
- [ ] 是否绕过了 `deriveBoards` 直接读取会话级状态？禁止——那会重新引入第二真相源。

### 明确禁止
1. **不要**在前端恢复任何按 `title / filename / result_text` 的项目归属匹配。
2. **不要**让 `FloatingTaskPanel` 重新读取 `sessionTodos` 作为看板数据源。
3. **不要**在 `sync_session_plans_after_revert` 中恢复"取磁盘最新一个方案"的模糊兜底。
4. **不要**用 `createdAt` 字符串比较来做看板排序（统一用消息 `seq`）。
5. **不要**为了"让老方案显示"而放宽 `source_message_id` 为空即丢弃的判定；应通过 `backfill_plan_sources` 把事实补齐。

### 如需恢复"回看历史看板"
不要恢复旧的翻页器与选中态。正确路径是：`deriveBoards` 结果本身即有序数组，历史回看只是 `boards[index]` 的一次索引访问；若需要跨轮次浏览，应设计为一个**独立的只读历史视图**，与"当前看板 = 末项"的契约解耦。
