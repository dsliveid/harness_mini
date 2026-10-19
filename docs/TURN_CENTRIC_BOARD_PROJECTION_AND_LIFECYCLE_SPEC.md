# 基于对话轮次纯函数状态投影与看板生命周期解耦治理规范
# Turn-Centric Board Projection & Dashboard Lifecycle Decoupling Governance Specification

| 属性 | 说明 |
| :--- | :--- |
| **文档代号** | `TURN_CENTRIC_BOARD_PROJECTION_AND_LIFECYCLE_SPEC` |
| **创建时间** | 2026-10-19 |
| **适用范围** | `harness_mini`（Tauri 2 + Rust + React 桌面 AI Agent 架构体系） |
| **关联核心模块** | [`src-tauri/src/plan.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/plan.rs), [`src-tauri/src/agent.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/agent.rs), [`src/components/FloatingTaskPanel.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/FloatingTaskPanel.tsx), [`src/store.ts`](file:///f:/WorkSpace/Other/harness_mini/src/store.ts), [`src/components/Composer.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/Composer.tsx) |
| **取代关系** | 取代 `docs/BOARD_SINGLE_PROJECTION_AND_PLAN_SOURCE_ATTRIBUTION_SPEC.md` 中关于“会话级最新方案兜底”与 `derive_current_board` 的全局回溯仲裁逻辑（规则 1 与规则 2） |
| **知识密级** | 任务管理、看板生命周期与领域模型解耦核心规范 |

---

## 目录

- [一、背景与痛点深度复盘](#一背景与痛点深度复盘)
  - [1.1 核心问题表象](#11-核心问题表象)
  - [1.2 历史两次改造的局限与失真根因](#12-历史两次改造的局限与失真根因)
  - [1.3 核心矛盾：持久交付资产 vs 轮次交互监视器](#13-核心矛盾持久交付资产-vs-轮次交互监视器)
- [二、核心架构设计原则](#二核心架构设计原则)
  - [2.1 轮次视界收敛（Turn-Centric Scope）](#21-轮次视界收敛turn-centric-scope)
  - [2.2 资产与看板彻底解耦（Decoupling Artifact & Dashboard）](#22-资产与看板彻底解耦decoupling-artifact--dashboard)
  - [2.3 零回溯、零猜测、缺失即空白（Absence is Clean）](#23-零回溯零猜测缺失即空白absence-is-clean)
  - [2.4 状态机自动闭环（Auto Settlement Contract）](#24-状态机自动闭环auto-settlement-contract)
- [三、架构全景与数据流](#三架构全景与数据流)
  - [3.1 轮次生命周期与数据流模型](#31-轮次生命周期与数据流模型)
  - [3.2 后端权威仲裁判定算法](#32-后端权威仲裁判定算法)
- [四、具体工程实施细节](#四具体工程实施细节)
  - [4.1 后端：重构 derive_current_board 算法](#41-后端重构-derive_current_board-算法)
  - [4.2 后端：步骤手动勾选与结案状态机联动](#42-后端步骤手动勾选与结案状态机联动)
  - [4.3 后端：对话正常交付时方案状态的自愈闭环](#43-后端对话正常交付时方案状态的自愈闭环)
  - [4.4 前端：会话发送新消息时的状态重置与清洗](#44-前端会话发送新消息时的状态重置与清洗)
  - [4.5 前端：FloatingTaskPanel 的平滑收起与空态契约](#45-前端floatingtaskpanel-的平滑收起与空态契约)
- [五、行为契约与边界场景验证矩阵](#五行为契约与边界场景验证矩阵)
- [六、工程实施步骤与质量基线](#六工程实施步骤与质量基线)

---

## 一、背景与痛点深度复盘

### 1.1 核心问题表象
1. **方案跨轮次幽灵挂起**：上一轮对话中生成的任务方案，在开启了新的一轮对话（哪怕只是普通闲聊、问答或修改单处代码）后，依然顽固地悬挂在界面右侧浮窗；
2. **状态永久僵死为未完成**：即使任务在实际代码中已经完成，右侧看板状态依然长期保持为 `in_progress`（进行中）或执行中；
3. **改造未达预期**：前一次改造旨在消除多方案翻页并让浮窗在开启新对话后不显示旧任务，但实际上无论是未结案方案还是已结案方案，依然会被死死挂在右侧。

### 1.2 历史两次改造的局限与失真根因
1. **第一次改造（前端单一投影）**：
   - 提出了 `boards = deriveBoards(visibleMsgs); currentTask = boards[boards.length - 1]`；
   - **致命漏洞**：它把“所有产生过看板的历史消息中的最后一项”误当成了“最新一轮对话的看板”。如果当前轮次没有调用任何方案或待办工具，`boards.length - 1` 仍然指向上一轮乃至很久之前的旧方案，导致旧方案依然霸屏。
2. **第二次改造（后端 derive_current_board 仲裁，commit 9f55e1d）**：
   - 将逻辑迁移到后端，却走向了更严重的**“会话全局过度兜底”**：
   - **规则 1**：优先查找会话名下任何处于 `in_progress` 的方案，只要有就全局置顶；
   - **规则 2**：即使方案结案了，也要通过 `find_latest_session_plan` 挖出一个已结案方案兜底展示，绝不留白；
   - **结论**：后端的仲裁目标被扭曲为“无论如何都要在历史记录里挖出一个东西挂在右侧”，彻底摧毁了“开启新一轮对话后清空”的初衷。

### 1.3 核心矛盾：持久交付资产 vs 轮次交互监视器
- **方案（Plan MD 文件）** 是**持久交付资产（Artifact）**：跨越多个文件和多轮迭代，存储于 `.harness/plans/*.md`；
- **浮窗（FloatingTaskPanel）** 是**实时交互监视器（Turn Dashboard）**：仅用于监控**当前轮次（Active Turn）**中 AI Agent 的推进进度。
- 将二者生命周期强行绑定的直接后果：大模型只要在交付完代码后偷懒没调用 `update_plan(completed)`，或者用户输入了一个新的临时指令，系统就因为资产尚未标记完成而强制将其作为实时监控器霸占屏幕。

---

## 二、核心架构设计原则

### 2.1 轮次视界收敛（Turn-Centric Scope）
浮窗看板的唯一合法数据源，**必须且仅能来自当前会话最后一个未被撤回的对话轮次（Latest Active Turn）**。
- **轮次定义**：以当前会话中最后一条未撤回且 `role = 'user'` 的消息为起点，直至会话末尾（包含该用户消息触发的所有助手消息与工具事件）。
- **视界约束**：后端的仲裁视界强制受限于本轮消息范围。严禁跨过当前最新 User Message 向前穿透回溯更早历史中的方案或待办。

### 2.2 资产与看板彻底解耦（Decoupling Artifact & Dashboard）
- **持久资产按需引用**：方案即使未结案（`in_progress`），只要当前轮次没有调用 `create_plan` 或 `update_plan` 激活它，它就静默存放在磁盘上，右侧浮窗绝不主动将其挂起。
- **交互监控按需唤醒**：只有当 Agent 在当前轮次真正调用了计划或任务工具，浮窗才被唤醒；当前轮次若无任务工具调用，浮窗保持为空并自动隐去。

### 2.3 零回溯、零猜测、缺失即空白（Absence is Clean）
- 彻底废除规则 2 中“拿已结案方案兜底”的机制；
- 若当前最新轮次未产生任何看板（无成功的 `create_plan`、`update_plan`、`todo`），后端直接权威返回 `None`；
- 前端接收到 `None` 即自动收起或隐藏，绝不在前端自行做任何历史缓存回退。

### 2.4 状态机自动闭环（Auto Settlement Contract）
- **步骤全部完成时自动升级状态**：用户手动在界面上将步骤全部勾选为 `done` 时，后端 `update_plan_step_status` 不仅更新 Markdown 正文，必须同时自动将 Frontmatter 的 `meta.status` 置为 `completed` 并解除活动锚点绑定。
- **对话正常交付时自愈**：在 Agent 交付完毕（`auto_finish_session_todos`）时，若方案步骤均已完成，自动结案并立即广播 `board:updated`。

---

## 三、架构全景与数据流

### 3.1 轮次生命周期与数据流模型

```
用户发送新消息 (New User Message, seq = K)
         │
         ├───► 前端：清空本地当前会话 currentBoards[sessionId] = null
         │
         ▼
Agent 引擎启动运行 (Turn K 开始)
         │
   ┌─────┴────────────────────────────────────────────────┐
   │ 轮次执行中：模型是否调用看板类工具？                    │
   │ (create_plan / update_plan / todo)                   │
   └─────┬────────────────────────────────────────────────┘
         │
         ├───► 【是】：工具执行成功
         │            │
         │            ▼
         │       触发 derive_current_board (限定 source_msg.seq >= K)
         │            │
         │            ▼
         │       广播 board:updated ──► 前端实时渲染本轮活跃看板
         │
         └───► 【否】：纯文本回复、普通代码编辑、或非看板类工具
                      │
                      ▼
                 对话正常结束 (Turn K 交付)
                      │
                      ▼
                 触发 derive_current_board (限定 seq >= K)
                      │
                      ▼
                 由于本轮无看板工具 ──► 返回 None
                      │
                      ▼
                 广播 board:updated (board: null)
                      │
                      ▼
                 前端浮窗优雅隐去 (零历史残留)
```

### 3.2 后端权威仲裁判定算法

在 `derive_current_board` 中确立如下确定性仲裁规则：

1. **定位最新轮次基准点**：
   查询当前会话中未撤回（`reverted_at IS NULL`）的最后一条 `role = 'user'` 消息的 `seq`，记为 `last_user_seq`。
   - 若会话中根本不存在 User Message，直接返回 `None`。
2. **收集本轮有效看板事件（本轮视界）**：
   - **本轮 Todo**：查询当前会话中满足 `m.seq >= last_user_seq` 且 `m.reverted_at IS NULL` 的最新成功 `todo` 事件；
   - **本轮 Plan 激活**：查询当前会话中满足 `m.seq >= last_user_seq` 且 `m.reverted_at IS NULL` 的最新成功 `create_plan` 或 `update_plan` 事件。通过其绑定的方案文件提取当前最新状态；
3. **确定性仲裁输出**：
   - 若本轮 **既无 Todo 又无 Plan** $\longrightarrow$ **返回 `None`**（浮窗隐藏）；
   - 若本轮 **仅有 Plan** $\longrightarrow$ 返回该方案格式化看板；
   - 若本轮 **仅有 Todo** $\longrightarrow$ 返回该待办格式化看板；
   - 若本轮 **同时存在 Plan 与 Todo** $\longrightarrow$ 比较产生事件的助手消息 `seq`，取序号更大（更新者）优先；若 `seq` 相同，Plan 优先于 Todo。

---

## 四、具体工程实施细节

### 4.1 后端：重构 `derive_current_board` 算法
**目标文件**：[`src-tauri/src/plan.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/plan.rs)

#### 4.1.1 核心逻辑改造
彻底废除跨轮次回溯的“规则 1 全局置顶”与“规则 2 结案兜底”，实现精准的轮次视界判定：

```rust
pub fn derive_current_board(
    conn: &rusqlite::Connection,
    workspace: &Path,
    session_id: &str,
) -> Option<CurrentBoard> {
    if session_id.is_empty() || session_id == "draft" || session_id == "DRAFT_ID" {
        return None;
    }

    // 1. 获取当前会话最新一条未撤回的用户消息序号（当前轮次的起点）
    let last_user_seq: Option<i64> = conn
        .query_row(
            "SELECT seq FROM messages 
             WHERE session_id = ?1 AND role = 'user' AND reverted_at IS NULL 
             ORDER BY seq DESC LIMIT 1",
            rusqlite::params![session_id],
            |r| r.get(0),
        )
        .ok();

    let Some(min_seq) = last_user_seq else {
        return None;
    };

    // 2. 查询本轮（seq >= min_seq）内产生的最新有效 Todo
    let latest_turn_todo = query_turn_latest_todo(conn, session_id, min_seq);

    // 3. 查询本轮（seq >= min_seq）内被激活或更新的有效 Plan
    let latest_turn_plan = query_turn_active_plan(conn, workspace, session_id, min_seq);

    // 4. 纯函数仲裁：无回溯，无全局兜底，缺失即返回 None
    match (latest_turn_todo, latest_turn_plan) {
        (Some(todo_info), Some((path, meta, body, plan_seq))) => {
            if todo_info.seq > plan_seq {
                Some(format_todo_board(&todo_info))
            } else {
                Some(format_plan_board(&path, &meta, &body))
            }
        }
        (Some(todo_info), None) => Some(format_todo_board(&todo_info)),
        (None, Some((path, meta, body, _))) => Some(format_plan_board(&path, &meta, &body)),
        (None, None) => None, // 本轮未产生任务或方案，明确返回 None，浮窗隐去
    }
}
```

#### 4.1.2 辅助查询函数
- `query_turn_latest_todo(conn, session_id, min_seq)`：
  增加 `AND m.seq >= ?2` 过滤，确保待办必须属于本轮对话。
- `query_turn_active_plan(conn, workspace, session_id, min_seq)`：
  查询本轮内（`m.seq >= min_seq`）最后一次成功的 `create_plan`、`update_plan` 或 `switch_plan` 工具调用，并读取对应 `.harness/plans/*.md` 文件。未在本轮被工具触达的方案视为未激活，不参与本轮看板呈现。

---

### 4.2 后端：步骤手动勾选与结案状态机联动
**目标文件**：[`src-tauri/src/plan.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/plan.rs#L877-L940) 中的 `update_plan_step_status`

#### 改造点：
用户在右侧浮窗勾选步骤时，若所有步骤均已被标记为 `done`，自动将 Frontmatter 的 `status` 变更为 `completed` 并广播：

```rust
    // 同步步骤到 todo 队列
    let parsed_steps = parse_steps_from_markdown(&new_body);
    sync_steps_to_todos(conn, &meta.session_id, &parsed_steps);

    // 【新增】：若所有步骤均已完成，自动闭环方案为 completed 并解除 active_plan 绑定
    let all_steps_done = !parsed_steps.is_empty() && parsed_steps.iter().all(|s| s.status == "done");
    if all_steps_done && meta.status != "completed" {
        meta.status = "completed".to_string();
        meta.updated_at = Utc::now().to_rfc3339();
        let full_content = format_with_frontmatter(&meta, &new_body);
        let _ = fs::write(&file_path, full_content);
        let _ = set_active_plan_id(conn, &meta.session_id, None);
    }
```

---

### 4.3 后端：对话正常交付时方案状态的自愈闭环
**目标文件**：[`src-tauri/src/agent.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/agent.rs#L3173-L3190)

#### 改造点：
在 `auto_finish_session_todos` 中，如果大模型交付时步骤已经全为 `done`（或被自动闭环），不仅更新磁盘文件，并确保立即通过 `emit_board_updated` 将最新状态推送到前端。

---

### 4.4 前端：会话发送新消息时的状态重置与清洗
**目标文件**：[`src/components/Composer.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/Composer.tsx#L490-L525) 与 [`src/store.ts`](file:///f:/WorkSpace/Other/harness_mini/src/store.ts)

#### 改造点：
在用户点击“发送消息”的瞬间（发起新一轮 Turn），前端立即重置当前会话的 `currentBoards` 状态，避免在 Agent 产生新回复前界面仍显示前序轮次的陈旧方案：

```ts
// Composer.tsx: handleSend 发送前
if (currentId && currentId !== DRAFT_ID) {
  useStore.setState((st) => ({
    currentBoards: {
      ...st.currentBoards,
      [currentId]: null, // 清空本轮初始看板，实现无缝切轮
    },
  }));
}
```

---

### 4.5 前端：FloatingTaskPanel 的平滑收起与空态契约
**目标文件**：[`src/components/FloatingTaskPanel.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/FloatingTaskPanel.tsx#L280-L283)

#### 契约保持：
```tsx
const hasTasks = !!currentBoard && currentBoard.items.length > 0;
const hasCmds = runningCmds.length > 0;

if (!currentId || currentId === DRAFT_ID || (!hasTasks && !hasCmds)) return null;
```
当 `currentBoard` 为 `null`（新轮次无任务或上一轮任务已翻篇），且当前没有后台运行的命令进程时，面板安全返回 `null`，浮窗完全隐去，把主工作区完整留给用户的日常对话。

---

## 五、行为契约与边界场景验证矩阵

| 场景序号 | 场景描述 | 预期行为（重构后） | 旧实现对比（重构前缺陷） |
| :--- | :--- | :--- | :--- |
| **S-01** | 第一轮对话调用 `create_plan` 创建方案 | 浮窗展开展示第一轮方案及步骤进度 | 相同 |
| **S-02** | 开启第二轮对话，仅发送普通聊天或代码咨询（无任务工具） | **浮窗立即隐去，界面干干净净** ✅ | ❌ 第一轮方案始终僵死在右侧 |
| **S-03** | 开启第二轮对话，调用 `todo` 清单执行任务 | 浮窗仅展示第二轮的 `todo` 清单 | ❌ 第一轮的方案可能强行霸屏 |
| **S-04** | 开启第二轮对话，调用 `update_plan` 推进原方案 | 浮窗展示被重新激活的方案及更新后的进度 | 相同（但旧版会有多重回退抖动） |
| **S-05** | 在右侧浮窗中手动将方案所有步骤打钩 | 方案元数据与 Markdown **自动同步为 completed** ✅ | ❌ 元数据依然停留在 in_progress |
| **S-06** | 撤回最后一条普通对话 | 自动回退展示前一轮有效的看板（若前一轮有） | 相同 |
| **S-07** | 撤回产生方案的对话 | 方案看板自然随消息撤回而消失，浮窗隐去 | 相同 |
| **S-08** | 切换不同会话 | 各会话仅展示各自最后一轮的看板，无任务则无浮窗 | 相同 |

---

## 六、工程实施步骤与质量基线

### 6.1 实施顺序
1. **第一步（后端判定收敛）**：
   在 [`src-tauri/src/plan.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/plan.rs) 中实现 `query_turn_latest_todo` 与 `query_turn_active_plan`，并彻底重写 `derive_current_board`，废除跨轮次规则 2；
2. **第二步（结案状态机自愈）**：
   在 `update_plan_step_status` 中补齐全步骤 `done` 时自动升级为 `completed` 的逻辑；
3. **第三步（前端发送即重置）**：
   在 [`src/components/Composer.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/Composer.tsx) 发送动作中清空当前会话的 `currentBoard`；
4. **第四步（Rust 单元测试覆盖）**：
   在 `plan.rs` 的 `test_derive_current_board_scenarios` 中新增针对“新轮次无任务自动清空返回 None”的测试用例。

### 6.2 质量验证基线
- **Rust 编译与单测**：
  `cargo test --manifest-path src-tauri/Cargo.toml test_derive_current_board` 全部 100% 通过；
- **前端类型与打包**：
  `npm run build`（`tsc && vite build`）exit 0，零 TypeScript 类型报错。
- **回归验证**：
  遵循上述 8 项行为契约进行场景全量覆盖验证。
