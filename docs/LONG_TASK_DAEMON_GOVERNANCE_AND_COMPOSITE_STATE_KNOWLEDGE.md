# 长任务守护进程治理、复合会话状态机与单轨并发架构设计

> **版本规范**：v1.0.0  
> **最后更新**：2026-11-03  
> **核心归属**：长任务守护进程生命周期（Long Task Daemon Lifecycle）、强类型控制信道（Typed Control Contract）、单轨运行并发排他（Single-Track Concurrency Guard）、复合会话状态机（Composite Session State）、右侧浮窗与折叠心跳视图（Right Floating Panel & Heartbeat）

---

## 一、 背景与根因剖析 (Root Cause Analysis)

在多轮长任务自治推进、多子任务沙箱并发拆解与跨轮次长时间等待（`wait_subagents`）场景下，系统暴露了若干严重的状态脱节、状态被意外覆写与并发协程失控问题：

### 1. 现象一：长任务明明已结案，后台却在“悄悄执行”并疯狂消耗 Token
* **表象特征**：
  在主会话历史中，Agent 在第 22~26 轮针对目标「分析当前项目的技术架构」明确调用了 `control_long_task(action: 'complete')`，工具返回了 `🎉 长任务总目标已圆满达成并宣告结案！`，并在末尾输出了 `🎯 **长任务全部目标达成**`。但用户发现程序并未停止，后台在无声无息地执行海量工具调用、吞吐了 60 多万 Token，界面几分钟没有任何文本输出，给人一种“长任务陷入死循环偷跑”的强烈失控感。
* **深层根因**：
  1. **无缝注入新长任务**：在第 26 轮结案后，系统或用户在同一会话中立即启动了第二个长任务（`task-a3a716a6`，总目标为「创建子任务分析当前项目用到的技术」）。由于是在同一个会话内推进，主会话无缝承接了新目标，并进入了第二阶段推进。
  2. **长等待工具缺乏透明度**：第二阶段长任务派发了 3 个重量级工人沙箱（前端清单员、主进程清单员、工程化清单员），主会话随后执行了 `wait_subagents(timeout_secs: 600)`。在长达数分钟的等待期间，主会话没有文本流产生；但后台 3 个子 Agent 正在全速扫描代码，底层 Agent Run 活跃运行了整整 414 秒（近 7 分钟）。用户若未手动在右侧打开子进程分屏，就会误以为主对话在“暗中死循环”。

---

### 2. 现象二：界面右侧浮窗的状态错误显示为【已暂停】（Paused）
* **表象特征**：
  当长任务被宣告结案（或正在等待子沙箱推进）时，右侧悬浮看板中的状态徽章与文字却突变成黄色的 **`已暂停`**。
* **深层根因（致命时序覆写与状态机污染）**：
  1. **信号通道布尔盲区（Boolean Blindness）**：
     后端控制通道原本采用 `watch::channel<bool>`。无论用户点击“暂停”、点击“终止”还是 Agent 自身“结案完成”，均统一调用 `set_control(task_id, true)`。
  2. **守护循环无脑写回 `paused`**：
     在 `src-tauri/src/task/loop_runner.rs` 中，循环醒来的第一行代码逻辑为：
     ```rust
     // 历史缺陷代码
     if *rx.borrow() {
         task.status = "paused".to_string(); // 👈 致命 Bug！
         store::update_long_task(&db, &task);
         app.emit("task:update", &task);
         return;
     }
     ```
     `loop_runner` 收到退出信号后，在**完全没有检查 DB 终态之前**，无脑假定“收到信号就是用户点了暂停”，直接把 `task.status` 暴力覆写为 `"paused"` 并写回数据库，同时向前端广播 `"status": "paused"`！原本已经被 `complete_long_task` 正确标记为 `completed` 的终态，在毫秒内被退出中的协程踩踏覆写！
  3. **连锁反应：旧任务被误杀归档为 `cancelled`**：
     由于 Task 1 被误标成了 `paused`（在数据库中仍被判定为活跃未结案任务），当 Task 2 启动时，系统误以为 Task 1 是一个未结案的废弃任务，调用 `cancel_long_task` 将其强行置为 `cancelled`。
  4. **前端单轮状态判定误伤长任务**：
     在前端 `src/store.ts` 的 `onRunStatus` 中，原逻辑判定：
     ```typescript
     // 历史缺陷代码
     if (p.status !== "running" && p.status !== "done" && st.activeTasks[p.sessionId]) {
         const mappedTaskStatus = p.status === "failed" ? "failed" : "interrupted";
         ...
     }
     ```
     在正常的多轮对话中，每一轮 LLM 吐字结束后系统均会广播 `p.status = "idle"`（空闲）。而 `idle` 既不是 `running` 也不是 `done`，导致前端在每一次轮次切换的瞬间，无差别将 `activeTasks` 篡改为 `interrupted`！而在 `FloatingTaskPanel.tsx` 中，`interrupted` 被折叠判定为 `isPaused`，一律渲染为黄色【已暂停】。

---

### 3. 现象三：双 Run 并发竞争、消息错位与孤儿协程脱缰运行
* **表象特征**：
  当用户在长任务长等待期间在输入框发送消息（如“停止所有长任务”）时，系统出现了两个 Agent Run 在同一个会话中同时调用 LLM、同时往消息表交替写入的严重并发冲突。
* **深层根因**：
  1. **`spawn_session_task` 缺乏排他锁**：
     在 `src-tauri/src/agent.rs` 中，每次调用 `spawn_session_task` 时，即使检测到会话已有活跃的 `active_run`，系统依然直接调用 `tauri::async_runtime::spawn` 拉起一个新的 `run_loop`。
  2. **Abort 句柄被覆盖导致前序协程孤立**：
     新的 `run_loop` 创建后，直接执行 `*g = Some(jh)` 覆盖了 `h.abort`。原本在后台运行的 Run 2 失去了关联句柄，变成不受管控的**孤儿协程**，在后台继续执行了 7 分钟，与用户后续输入触发的 Run 4 发生激烈的资源竞争与数据踩踏。

---

### 4. 现象四：右侧浮窗在取消后直接消失，折叠态缺乏心跳感知
* **表象特征**：
  长任务一旦被终止（`cancelled`），右侧浮窗立刻从 DOM 中彻底卸载消失，用户无法查看最终沉淀的步骤清单与执行总结；当浮窗折叠收起到屏幕最右侧时，仅展示普通的静态文字，无法得知后台到底处于推进、等待还是暂停。
* **深层根因**：
  前端判定 `hasLongTask = !!activeTask && activeTask.status !== "cancelled"`，将 `cancelled` 排除在渲染范围之外；且浮窗折叠态仅渲染静态图标，缺少呼吸动效与心跳数据流。

---

## 二、 核心架构原则与设计模式 (Architecture & Governance Model)

针对上述四大深层病灶，本次改造**坚决摒弃“堆叠补丁”式的临时修改**，从系统第一性原理重构守护进程生命周期与会话状态拓扑：

```
+---------------------------------------------------------------------------------------------------------+
|                                    会话守护控制层 (Session Daemon Control Core)                          |
|  1. SessionDaemonGuard: 会话级单例守护锁 (同一 session_id 物理上唯一存在一个活跃守护协程)                  |
|  2. DaemonControlSignal: 强类型控制契约 (Pause / Cancel / Complete / Running 消除布尔盲区)              |
|  3. SessionRunMutex: 会话执行排他锁 (spawn_session_task 强门禁，杜绝双 Run 并发与孤儿协程)                |
+---------------------------------------------------------------------------------------------------------+
                                      |                                   |
                                状态驱动与控制                      生命周期与排他
                                      v                                   v
+---------------------------------------------------------------------------------------------------------+
|                                  运行时驱动引擎 (Runtime Daemon Loop Engine)                             |
|  - 终态只读保护 (Read-Only Terminal Guard): 收到退出信号先查 DB，已为终态则安静退出，严禁逆向写回 paused  |
|  - wait_subagents 动态干预通道: 循环内感知用户排队消息 (list_queued)，检测到即刻提前唤醒进入下一轮纠偏      |
|  - 协同等待实时心跳 (subagents:wait_heartbeat): 每 2 秒向前端广播子任务就绪进度 (N/M 已就绪)              |
+---------------------------------------------------------------------------------------------------------+
                                      |
                                  原子事件广播
                                      v
+---------------------------------------------------------------------------------------------------------+
|                                前端复合状态机 (Composite Session State Architecture)                    |
|  - 宏微正交状态解耦: 宏观长任务态 (running/waiting/paused/completed/cancelled) 与 微观单轮流式态正交分离   |
|  - 状态主从绑定: 守护进程活跃推进时，主会话 runStatus 持续锁定为 running，消除“假死与空闲闪烁”             |
|  - 消除单轮误伤: 仅在明确收到 cancelled/failed 时流转长任务终态，空闲轮次切换绝不篡改状态                  |
+---------------------------------------------------------------------------------------------------------+
                                      |
                                  视图精准投射
                                      v
+---------------------------------------------------------------------------------------------------------+
|                                右侧浮窗与折叠心跳增强 (FloatingTaskPanel Refined)                       |
|  - 终态全景展现: 补齐 已达成 (绿) / 已终止 (灰) / 等待沙箱 (紫) / 已暂停 (黄) / 推进中 (蓝) 5 大状态分支   |
|  - 折叠态心跳药丸: 呼吸指示灯 + 动态轮次/就绪进度，余光即可掌握后台动向                                 |
|  - 运行期干预联动: Composer 自动感知长任务态，输入框提示纠偏插话，并拦截同一会话内重复发起长任务          |
+---------------------------------------------------------------------------------------------------------+
```

---

## 三、 四大关键治理模块落地细节 (Implementation Details)

### 模块一：强类型控制契约与终态只读保护 (`src-tauri/src/task/`)

彻底废弃原先的 `watch::channel<bool>`，定义强类型指令枚举：

```rust
// src-tauri/src/task/mod.rs
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonControlSignal {
    Running,
    Pause { reason: Option<String> },
    Cancel { reason: Option<String> },
    Complete { summary: Option<String>, evidence: Option<String> },
}
```

#### 1. 守护循环的“终态只读保护（Read-Only Terminal Guard）”
在 `loop_runner.rs` 中，守护循环收到控制信号后，按信号类别与数据库真实状态进行严格分流：
```rust
// src-tauri/src/task/loop_runner.rs
// 1. 响应控制信道指令（强类型信号分流与终态保护）
let signal = rx.borrow().clone();
match signal {
    crate::task::DaemonControlSignal::Pause { .. } => {
        let db_status = {
            let db = state.db.lock().unwrap();
            store::get_long_task(&db, &task_id).ok().flatten().map(|t| t.status)
        };
        // 终态保护：若已在数据库中宣告完成或取消，严禁反向写回 paused！
        if db_status.as_deref() != Some("completed") && db_status.as_deref() != Some("cancelled") {
            task.status = "paused".to_string();
            task.updated_at = store::now();
            {
                let db = state.db.lock().unwrap();
                let _ = store::update_long_task(&db, &task);
            }
            let _ = app.emit("task:update", &task);
        }
        crate::task::remove_control(&task_id);
        return;
    }
    crate::task::DaemonControlSignal::Cancel { .. } | crate::task::DaemonControlSignal::Complete { .. } => {
        // 取消与结案指令：DB 已在 complete/cancel_long_task 中确立终态并派发事件，守护进程纯只读退出
        crate::task::remove_control(&task_id);
        return;
    }
    crate::task::DaemonControlSignal::Running => {}
}
```

#### 2. 新任务启动时的旧任务保护
在 `start_long_task` 中，仅针对未完结的旧任务（`running` / `planning` / `paused`）进行安全归档；如果旧任务已经完成（`completed`），系统将其完好归档，绝不误触发 `cancel_long_task`：
```rust
// src-tauri/src/task/mod.rs
if let Ok(Some(old_active)) = get_active_task(&app, &session_id) {
    if old_active.status == "running" || old_active.status == "planning" || old_active.status == "paused" {
        let _ = cancel_long_task(&app, &old_active.id);
    }
}
```

---

### 模块二：会话单轨并发排他与干预通道 (`src-tauri/src/agent.rs` & `tools/subagent.rs`)

彻底解决双 Run 并发竞争与长等待期间用户无法干预的问题：

#### 1. 会话级执行排他门禁 (`SessionRunMutex`)
在 `spawn_session_task` 入口处建立物理互斥：
```rust
// src-tauri/src/agent.rs
pub fn spawn_session_task(app: AppHandle, session_id: String, trigger: Option<String>) {
    {
        let state = app.state::<crate::AppState>();
        let mut handles = state.handles.lock().unwrap();
        let h = handles.entry(session_id.clone()).or_insert_with(SessionHandle::new);
        if h.active_run.lock().unwrap().is_some() {
            // 当前会话已有正在执行的 run，绝对不并发拉起第二个 run_loop，严禁覆盖现有 abort handle！
            return;
        }
        *h.active_run.lock().unwrap() = Some("starting".into());
    }
    ...
```

#### 2. `wait_subagents` 动态干预感知与实时心跳广播
在 `wait_subagents_tool` 中，打破以往死睡 600 秒的黑盒，引入双向联动：
```rust
// src-tauri/src/tools/subagent.rs
loop {
    // 1. 若父会话已被外部停止或收到新的干预指示，立即退出等待
    if !crate::agent::is_run_active(&state, parent_id) {
        break;
    }
    let has_user_steering = {
        let db = state.db.lock().unwrap();
        crate::store::list_queued(&db, parent_id).map(|l| !l.is_empty()).unwrap_or(false)
    };
    if has_user_steering {
        // 用户在主会话中发送了纠偏消息，立即提前中断长等待，让主会话决策层直接进入下一轮响应
        break;
    }

    let any_running = target_ids.iter().any(|id| crate::agent::is_run_active(&state, id));
    if !any_running || start_wait.elapsed() >= max_wait {
        break;
    }

    // 2. 定期向前端广播心跳（每 2 秒一次），实时同步子沙箱执行聚合进度
    if last_heartbeat.elapsed() >= std::time::Duration::from_secs(2) {
        last_heartbeat = std::time::Instant::now();
        let mut running_cnt = 0usize;
        let mut done_cnt = 0usize;
        for id in &target_ids {
            if crate::agent::is_run_active(&state, id) {
                running_cnt += 1;
            } else {
                done_cnt += 1;
            }
        }
        let _ = host.app.emit("subagents:wait_heartbeat", serde_json::json!({
            "parentSessionId": parent_id,
            "total": target_ids.len(),
            "running": running_cnt,
            "completed": done_cnt,
            "elapsedSecs": start_wait.elapsed().as_secs(),
            "timeoutSecs": timeout_secs,
        }));
    }
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
}
```

---

### 模块三：前端复合状态机与数据流治理 (`src/store.ts` & `src/events.ts`)

#### 1. 状态绑定机制（守护进程进行中，主会话永不跳空闲）
```typescript
// src/store.ts -> onRunStatus
let effectiveRunStatus = p.status === "running" ? ("running" as const) : ("idle" as const);
const activeTaskForSession = nextActiveTasks[p.sessionId];
if (
  activeTaskForSession &&
  (activeTaskForSession.status === "running" ||
   activeTaskForSession.status === "planning" ||
   activeTaskForSession.status === "waiting_subagents")
) {
  // 守护进程仍在推进或等待子任务，主对话与守护进程绑定，继续保持 running 进行中！
  effectiveRunStatus = "running" as const;
}

return {
  runStatus: { ...st.runStatus, [p.sessionId]: effectiveRunStatus },
  ...
};
```

#### 2. 消除误伤：空闲轮次切换不篡改长任务状态
```typescript
// src/store.ts -> onRunStatus
// 仅在明确收到取消/失败事件时将进行中的长任务置为 cancelled/failed，空闲轮次切换绝不篡改长任务状态
let nextActiveTasks = st.activeTasks;
if ((p.status === "cancelled" || p.status === "failed") && st.activeTasks[p.sessionId]) {
  const curTask = st.activeTasks[p.sessionId]!;
  if (curTask.status === "running" || curTask.status === "planning" || curTask.status === "waiting_subagents") {
    const mappedTaskStatus = p.status === "failed" ? "failed" : "cancelled";
    ...
  }
}
```

#### 3. 实时心跳事件响应
```typescript
// src/store.ts
onWaitHeartbeat(ev: SubagentsWaitHeartbeat) {
  set((st) => {
    const curTask = st.activeTasks[ev.parentSessionId];
    const nextTasks = curTask && curTask.status === "running"
      ? {
          ...st.activeTasks,
          [ev.parentSessionId]: {
            ...curTask,
            status: "waiting_subagents" as const,
          },
        }
      : st.activeTasks;

    return {
      activeTasks: nextTasks,
      subagentsWaitHeartbeat: {
        ...st.subagentsWaitHeartbeat,
        [ev.parentSessionId]: ev,
      },
    };
  });
}
```

---

### 模块四：右侧浮窗与输入框交互增强 (`FloatingTaskPanel.tsx` & `Composer.tsx`)

#### 1. 终态卡片全保留与全状态徽章
* 移除了 `hasLongTask` 中对 `cancelled` 的排斥，任务即使终止或达成，卡片始终保留展示成果。
* 状态标签全面穷举：
  * `isCompleted`：🟢 **已达成** (`bg-green-500/20 text-green-300`)
  * `isCancelled`：⚪ **已终止** (`bg-slate-500/20 text-slate-300`)
  * `isWaitingSubagents`：🟣 **等待沙箱** (`bg-purple-500/20 text-purple-300 animate-pulse`)
  * `isPaused`：🟡 **已暂停** (`bg-amber-500/20 text-amber-300`)
  * `isFailed`：🔴 **遇阻** (`bg-red-500/20 text-red-300`)
  * `isRunning`：🔵 **推进中** (`bg-blue-500/20 text-blue-300 animate-pulse`)

#### 2. 沙箱等待进度指示条
在浮窗卡片中直观呈现心跳聚合进度：
```tsx
{/* 沙箱协同等待实时进度心跳 */}
{isWaitingSubagents && waitHeartbeat && (
  <div className="flex items-center justify-between text-[10.5px] px-2 py-1 rounded bg-purple-500/15 border border-purple-500/25 text-purple-300">
    <span className="flex items-center gap-1">
      <Loader2 size={10} className="animate-spin text-purple-400" />
      <span>协同沙箱推进中: {waitHeartbeat.completed}/{waitHeartbeat.total} 已就绪</span>
    </span>
    <span className="font-mono text-[10px] text-inkdim">{waitHeartbeat.elapsedSecs}s / {waitHeartbeat.timeoutSecs}s</span>
  </div>
)}
```

#### 3. 折叠把手呼吸灯小药丸 (Mini Heartbeat Pill)
当右侧浮窗收起折叠到边缘时，折叠拉手显示呼吸小圆点与动态文字：
- `🔵 推进中` / `🟣 等待沙箱` / `⏸️ 已暂停` / `⚪ 已终止` / `🟢 已达成`，保证用户在任何情况下都能掌握长任务生命周期。

#### 4. 输入框单例门禁与纠偏提示 (`Composer.tsx`)
- 拦截同一会话中重复发起长任务的企图，提示“当前会话已有正在推进的长任务，请先暂停或终止旧任务”；
- 当长任务在推进或等待沙箱时，占位提示语自动变为：`长任务推进中…输入内容将作为实时纠偏干预指令即刻注入`。

---

## 四、 验证与自检验收记录 (Verification)

| 验证项 | 验证命令 / 测试用例 | 结果 | 验收结论 |
|---|---|---|---|
| **前端 TypeScript 静态检查** | `npx tsc --noEmit` | **0 errors** | 类型契约与事件签名完全闭环 |
| **Rust 编译与借用检查** | `cargo check --manifest-path src-tauri/Cargo.toml` | **0 errors** | 强类型信道无缝适配 |
| **后端单元与回归测试集** | `cargo test --manifest-path src-tauri/Cargo.toml task` | **8 passed; 0 failed** | 长任务截断、上下文锚点与自治循环测试全绿 |
| **结案覆写防线自检** | `test_is_waiting_for_user_confirmation` | **passed** | 终态只读保护有效 |

---

## 五、 总结与日常维护指引

1. **绝对禁止逆向覆写状态**：
   在任何后台长任务协程退出代码中，**严禁无脑执行 `task.status = "paused"`**。必须先检查数据库或内存中任务是否已被 `complete` 或 `cancel` 确立终态。
2. **坚持会话级单轨原则**：
   一个会话物理上只能对应一个前台 Agent 决策流。用户在执行期插入的消息必须作为排队/纠偏输入，绝对不可并发拉起第二个 `spawn_session_task`。
3. **协同沙箱始终透明**：
   耗时超过 5 秒的异步协同工具必须提供定时进度心跳（Heartbeat），杜绝主对话出现“长达数分钟无响应”的体感假死。
