# AI Agent 会话生命周期、状态机单一事实源（SSOT）与执行异常治理知识文档
(SESSION_EXECUTION_LIFECYCLE_AND_SSOT_GOVERNANCE_KNOWLEDGE)

| 属性 | 说明 |
| :--- | :--- |
| **文档代号** | `SESSION_EXECUTION_LIFECYCLE_AND_SSOT_GOVERNANCE_KNOWLEDGE` |
| **所属模块** | [`src-tauri/src/store.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/store.rs), [`src-tauri/src/agent.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/agent.rs), [`src/components/ChatView.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/ChatView.tsx), [`src/store.ts`](file:///f:/WorkSpace/Other/harness_mini/src/store.ts) |
| **技术体系** | State Machine · SQLite Triggers · Single Source of Truth (SSOT) · Tauri 2 / Rust · React 18 / Zustand |
| **创建时间** | 2026-10-30 |
| **核心机制** | 数据库级状态级联触发器 · 正常退出与异常清理严格隔离 · 语义化状态收敛（cancelled vs interrupted） · 全库脏状态原子校准 |

---

## 一、 背景痛点与现场故障复盘

在桌面级 AI Agent（`harness_mini`）运行过程中，出现了一种隐蔽且反复偶发的严重体验问题：**用户发起对话后，Agent 已经完整执行并生成了回答（无报错、无崩溃、正常交付），但在用户切换会话再切回、或重新启动客户端后，该会话底部突然弹出了醒目的黄色异常恢复条：**

> **对话执行中断或遇到异常**  
> 您可以选择重试本轮，或让 Agent 基于当前已有上下文继续向下推进。

这种“明明已经执行完成了，却在事后被判为执行异常”的幽灵报错，给用户造成了严重的困惑与不信任感，且多次修复后仍有类似变种冒出。

---

### 1.1 现场数据事实排查

对本地 SQLite 数据库（`.dev-data/harness_mini.db`）中最新一次会话 `54575cb9-fe90-4612-9aad-bd47c34e85ce`（标题：“你好”）进行深入现场取证：

1. **消息层与 Token 消耗**：
   - 用户发送：“你好”；
   - 助理回复（seq=2）：完整生成了关于本项目认知档案的 480 tokens 内容，回答完好且正常落库；
2. **底层执行 Run 层（`runs` 表）**：
   - 对应 Run 记录（`63e4671c-849b-4576-b964-cfdf80e6ca15`）：
     - `status`: **`"done"`**（正常执行完毕）
     - `duration_ms`: 10129ms
     - `total_tokens`: 10604
     - `started_at` / `ended_at` 时间戳齐全完整；
3. **会话聚合层（`sessions` 表）**：
   - `last_run_status`: **`"interrupted"`**（异常中断！）。

不仅该会话，进一步排查发现，近期正常执行完毕并完成交付的 **6 个历史会话**（包括阶段分析报告、多任务编排等长会话），其在 `runs` 表中的实际终态全部为 `"done"`，但在 `sessions.last_run_status` 中**无一例外全被写成了 `"interrupted"`**。

---

## 二、 架构级根因与“堆叠式开发”反思

为什么这不是简单的某一行代码 Bug，而是系统架构设计存在结构性缺陷？

### 2.1 缺陷 1：缺乏单一事实源（Lack of SSOT），状态多头写入与“数据裂脑”

关于“会话当前的执行状态”，系统在设计演进中引入了多份冗余表示：
1. `runs.status`：记录物理层单次执行任务的真实生命周期（`running` / `done` / `failed` / `interrupted` / `cancelled`）；
2. `sessions.last_run_status`：为提升侧边栏列表查询性能而在 `sessions` 表中添加的反范式冗余字段；
3. `sessions.status`：会话业务态（`active` / `completed` / `cancelled` / `archived`）；
4. 前端 Zustand Store 中的 `runStatus`、`lastRunOutcome`、`activeTasks`。

**致命漏洞**：`sessions.last_run_status` 的更新逻辑散落在各个业务函数中（`finish_run`、`fail_open_runs`、`start_run`、`startup_reconcile` 等）。**一旦有任何一处遗漏条件或逻辑偏差，就会引发“数据裂脑”**——底层数据与聚合缓存完全对立。

---

### 2.2 缺陷 2：兜底逻辑越界侵入主流程，正常路径被反噬

排查 [`src-tauri/src/agent.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/agent.rs) 中的 `run_loop` 主循环：

```rust
// agent.rs: agent_loop 退出主循环处
    while let Some(...) = { ... } {
        let (outcome, ...) = run_once(...).await;
        // 1. 正常执行完毕，调用 finish_run，将 runs.status 设为 done，sessions.last_run_status 设为 done
        store::finish_run(&db, &run_id, "done", ...);
    } // 正常跑完，无待处理消息，跳出循环

    // 2. 开发者在循环外添加了“防御性兜底”：
    {
        let db = state.db.lock().unwrap();
        let _ = store::fail_open_runs(&db, &session_id); // <-- 致命调用！
    }
```

再查看 [`src-tauri/src/store.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/store.rs) 原先的 `fail_open_runs` 实现：

```rust
pub fn fail_open_runs(conn: &Connection, session_id: &str) -> Result<(), String> {
    // 1. 只有 status 为 running 的 run 会被置为 interrupted（正常跑完后这里更新 0 行）
    conn.execute(
        "UPDATE runs SET status = 'interrupted', ended_at = ?2 WHERE session_id = ?1 AND status = 'running'",
        params![session_id, now()],
    )?;
    // 2. 致命 Bug：无条件将 sessions.last_run_status 改为 interrupted！
    let _ = conn.execute(
        "UPDATE sessions SET last_run_status = 'interrupted' WHERE id = ?1",
        params![session_id],
    );
    Ok(())
}
```

**问题本质**：
- 在成熟的架构中，**正常退出路径（Happy Path）绝对严禁执行异常清理逻辑（Error Cleanup Path）**；
- 开发者此前为了防范“任务中断或崩溃时可能悬挂为 running”，在主循环正常退出后盲目塞入了 `fail_open_runs`；
- 而 `fail_open_runs` 的第二条 SQL 没有限制 `WHERE last_run_status = 'running'`，也未校验第一条 SQL 是否真实影响了行数（`affected > 0`），导致**每一次正常执行完成的会话，在退出瞬间都会被这句无条件 SQL 强行逆转为 `interrupted`**！

---

### 2.3 缺陷 3：瞬态事件流（Event）与持久化快照（Snapshot）相互踩踏

为什么在对话刚完成时用户没看到报错，切回来或刷新后报错才浮现？
1. **完成瞬间**：`run_loop` 结束前向前端派发了 WebSocket 事件 `run:status = "done"`，前端内存中的 `lastRunOutcome[sid]` 暂时为 `"done"`，此时界面不显示异常条；
2. **毫秒之后**：后端的 `fail_open_runs` 异步覆写了 SQLite，将 `sessions.last_run_status` 改为 `"interrupted"`；
3. **切换/刷新**：用户点击其他会话再切回时，前端 [`selectSession`](file:///f:/WorkSpace/Other/harness_mini/src/store.ts#L1093) 触发，从 `curSession.lastRunStatus` 中恢复状态。此时内存直接被 SQLite 脏数据冲刷为 `"interrupted"`，前端随即渲染错误条。

---

### 2.4 缺陷 4：堆叠式开发（Band-aid Accumulation）的恶性循环

```
[原始问题] 某些任务崩溃导致状态卡在 running
    │
    ▼ (打补丁 1)
在多个退出路径随处调用 fail_open_runs
    │
    ▼ (打补丁 2)
性能需要：在 sessions 表增加 last_run_status 冗余列
    │
    ▼ (引入隐患)
fail_open_runs 粗暴写入 UPDATE sessions SET last_run_status = 'interrupted'
    │
    ▼ (引发故障)
正常完成的会话被误杀为 interrupted，前端频繁报错
    │
    ▼ (打补丁 3)
前端 ChatView.tsx 写 6 层启发式规则去“猜”真实状态：
检查 activeTasks、检查长任务完成文本、检查 ⚠️、检查 user 角色、检查工具失败...
    │
    ▼ (恶果)
逻辑极度脆弱，规则相互打架，维护成本指数级飙升
```

---

## 三、 核心架构重构方案

要彻底消除此类问题，必须抛弃“头痛医头”的堆叠式打补丁，从架构底层确立**单一事实源**与**生命周期边界**。

### 3.1 方案架构图

```
┌────────────────────────────────────────────────────────┐
│                   权威单一事实源 (SSOT)                  │
│               数据库核心表：`runs` (status)              │
└────────────────────────────────────────────────────────┘
                           │
       ┌───────────────────┴───────────────────┐
       ▼ (SQLite Trigger 自动级联同步)           ▼ (应用层 Rust API)
┌───────────────────────────────┐     ┌───────────────────────────────────┐
│       `sessions` 表缓存       │     │     严格生命周期收敛函数          │
│ `last_run_status` 始终同步对齐 │     │ - finish_run: 正常交付 ('done')   │
└───────────────────────────────┘     │ - cancel_open_runs: 取消 ('cancelled')│
                                      │ - fail_open_runs: 异常 ('interrupted')│
                                      │   (均带 affected > 0 边界保护)    │
                                      └───────────────────────────────────┘
```

---

## 四、 关键技术实现详析

### 4.1 数据库触发器自动同步（消除应用层漏改）

在 [`src-tauri/src/store.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/store.rs#L125) 数据库表初始化脚本中，为 `runs` 表建立底层触发器：

```sql
-- 权威单一事实源约束：runs 状态变更时自动对齐 sessions.last_run_status，消灭应用层漏改/误改
CREATE TRIGGER IF NOT EXISTS trg_runs_status_sync_insert
AFTER INSERT ON runs
BEGIN
    UPDATE sessions SET last_run_status = NEW.status WHERE id = NEW.session_id;
END;

CREATE TRIGGER IF NOT EXISTS trg_runs_status_sync_update
AFTER UPDATE OF status ON runs
BEGIN
    UPDATE sessions SET last_run_status = NEW.status WHERE id = NEW.session_id;
END;
```

**成效**：无论应用层是调用完成、执行取消还是通过迁移收敛，只要 `runs` 表状态变动，SQLite 引擎自动级联保证 `sessions.last_run_status` 100% 强一致，彻底根除双状态裂脑。

---

### 4.2 增加受影响行校验与语义细化

重构 [`src-tauri/src/store.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/store.rs#L4131)：

1. **`fail_open_runs` 增加 `affected > 0` 保护**：
   ```rust
   pub fn fail_open_runs(conn: &Connection, session_id: &str) -> Result<(), String> {
       let affected = conn.execute(
           "UPDATE runs SET status = 'interrupted', ended_at = ?2 WHERE session_id = ?1 AND status = 'running'",
           params![session_id, now()],
       )
       .map_err(|e| e.to_string())?;

       // 仅在确实收敛了处于 'running' 状态的孤立 Run 时，才更新 session 状态
       if affected > 0 {
           let _ = conn.execute(
               "UPDATE sessions SET last_run_status = 'interrupted' WHERE id = ?1",
               params![session_id],
           );
       }
       Ok(())
   }
   ```
2. **拆分出语义明确的 `cancel_open_runs`**：
   ```rust
   /// 用户主动中止会话时收尾：把该会话残留的 running 状态 run 标记为 cancelled
   pub fn cancel_open_runs(conn: &Connection, session_id: &str) -> Result<(), String> {
       let affected = conn.execute(
           "UPDATE runs SET status = 'cancelled', ended_at = ?2 WHERE session_id = ?1 AND status = 'running'",
           params![session_id, now()],
       )
       .map_err(|e| e.to_string())?;

       if affected > 0 {
           let _ = conn.execute(
               "UPDATE sessions SET last_run_status = 'cancelled' WHERE id = ?1",
               params![session_id],
           );
       }
       Ok(())
   }
   ```

并在 [`src-tauri/src/agent.rs:245`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/agent.rs#L245) 的 `stop_session_ext` 中将原先的 `fail_open_runs` 替换为 `cancel_open_runs`，确保主动停止时数据库与前端广播事件同为 `"cancelled"`，界面准确展示“对话执行已手动中止”。

---

### 4.3 修正启动自愈 SQL 隐性报错

在 [`src-tauri/src/store.rs:5242`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/store.rs#L5242) 的 `startup_reconcile` 中，发现原代码误写了不存在的列名：
```rust
// ❌ 错误原代码：finished_at 列在 runs 表中并不存在，导致每次启动报错静默失效
conn.execute("UPDATE runs SET status = 'interrupted', finished_at = ?1 WHERE status = 'running'", ...);

// ✅ 修正后代码：准确更新 ended_at
conn.execute("UPDATE runs SET status = 'interrupted', ended_at = ?1 WHERE status = 'running'", ...);
```

---

### 4.4 历史脏数据原子级回填对齐

对数据库中因旧逻辑累积被误标为 `interrupted` 的历史记录执行原子级全库校准：

```sql
UPDATE sessions
SET last_run_status = (
    SELECT status FROM runs
    WHERE runs.session_id = sessions.id
    ORDER BY started_at DESC, id DESC LIMIT 1
)
WHERE EXISTS (
    SELECT 1 FROM runs WHERE runs.session_id = sessions.id
);
```

**校准结果**：全量 49 个会话的偏差从 100% 降为 **0 偏差**，包括最新“你好”会话在内的所有正常交付会话均恢复为真实的 `"done"`。

---

## 五、 验证与质量保障

| 验证项 | 验证手段 | 验证结果 |
| :--- | :--- | :--- |
| **后端 Rust 编译** | `cargo check --manifest-path src-tauri/Cargo.toml` | **通过（0 错误）** |
| **前端 TypeScript 类型检查** | `npx tsc --noEmit` | **通过（0 错误）** |
| **数据库触发器验证** | Python SQLite 模拟写入与级联检查 | **Trigger 正常触发，自动同步完成** |
| **会话状态校验** | 重新加载最新会话与历史交付会话 | **异常恢复条彻底消失，恢复为正常交付视图** |
| **主动停止校验** | 点击停止后切回 | **展示明确的“对话执行已手动中止”，状态一致** |

---

## 六、 架构设计长效准则（防退化建议）

为防止未来开发中再次退化为“堆叠式开发”，团队应严格遵守以下开发准则：

1. **坚持单一事实源原则（SSOT）**：业务实体的核心状态只能有一个权威表拥有写权限，任何反范式派生字段必须具备数据库级约束（Foreign Key / Trigger）或单一受限通道，禁止随处手写 `UPDATE`。
2. **正常流程与异常处理物理隔离**：正常执行的出口（Happy Path）只处理正常交付，不得掺杂兜底收尾代码；清理逻辑应限定在显式捕获的异常处理块或特定管理命令中。
3. **推崇 RAII Guard 模式**：对于容易因 panic、提前 return、协作中断而悬挂的异步任务，在 Rust 侧应封装成带有 `Drop` 实现的 `RunGuard`，由编译器保证异常时收敛，杜绝人工在退出点漏加或滥加兜底。
4. **前端消费强类型契约**：前端展示逻辑必须消费后端提供的确定性结构体（如 `turn.status`、`turn.error_code`），严禁依靠正则匹配消息内容、检查 Unicode 符号（如 ⚠️）或层层猜测来反推执行成败。
