# 扩展工具网关、子任务生命周期与反卡顿渲染治理架构设计

> **版本规范**：v1.0.0  
> **最后更新**：2026-11-03  
> **核心归属**：工具收敛架构（Tool Convergence）、有效工具语义解构（Effective Tool Unboxing）、子任务状态机防线、事件节流与反卡顿渲染（Anti-Lag Governance）

---

## 一、 背景与根因剖析 (Root Cause Analysis)

在长任务多轮推进与多子任务并发执行过程中，系统出现了若干严重的体验与稳定性问题：
1. **子任务卡片 UI 样式降级**：
   - 现象：子任务工具调用在前端被显示为普通扁平的 JSON 参数卡片（标签显示为 `call_extended_tool`），而不是专用的 `SubprocessBranchTree` 多级任务树分支卡片。
   - 根因：后端进行核心工具收敛（将子任务工具收敛入 `call_extended_tool` 网关）后，工具调用的顶层名称由 `spawn_subagent` 变成了 `call_extended_tool`，前端与后端生命周期 Hook 未对 `call_extended_tool` 进行解包提取有效工具（Effective Tool），导致 `tool_event.subprocess_id` 缺失，无法与子会话建立树形关联。
2. **子任务重复唤醒与自杀式死循环**：
   - 现象：子任务已成功调用 `deliver_to_parent` 结案并交付最终成果，但在父会话执行 `wait_subagents` 时，却被系统判定为“未产出最终回复”，强行触发 `restart_subagent` 唤醒进入 Run 2。
   - 根因：`wait_subagents_tool` 中的 `determine_subagent_status` 仅检测末尾消息是否为纯文本助手回复。由于子任务交付动作本身是一条工具调用（`role: "tool"`），导致已交付的子任务被误判为 `interrupted`。随后 `wait_subagents` 自动向其追加“请根据上述工具执行结果继续推进”的新提示词，引发子任务非预期的重复执行与 Token 浪费。
3. **主任务停止后子任务脱缰运行**：
   - 现象：用户在主会话点击停止按钮后，后台派生的子任务仍然在持续请求大模型并消耗 Token。
   - 根因：`stop_session_ext` 在级联停止子任务时，若子任务协程恰好处在两轮 LLM 调用的间隙或排队态，内存中未登记 `aborted_run`，导致未向前端补发 `run:status(idle)` 事件与 `subprocesses:changed` 广播，子任务生命周期状态滞留。
4. **长任务执行期间前端严重卡顿与掉帧**：
   - 现象：当 2~3 个子任务在后台并行生成时，主界面出现明显的打字卡顿、滚动掉帧、CPU 占用飙升。
   - 根因：
     - **无效全局重渲染**：子任务每生成一个 Token（`message:delta` 和 `message:reasoning_delta`）就会向前端广播，前端无论当前激活会话是谁，都会将全部增量写入全局 Store，触发整个组件树的频繁重渲染。
     - **无条件加载消息**：后端每次生成消息时向前端广播 `messages:changed`，前端无条件针对所有会话发起 `ipc.getMessages` 查询并序列化。
     - **Selector 引用不稳定**：`currentMessages` 选择器每次执行均调用 `sortMessages` 生成全新的数组实例，导致 React 认为消息引用每次均改变，彻底破坏了 `useMemo` 与纯组件缓存。
5. **扩展工具参数遗漏与 Schema 缺失**：
   - 现象：模型调用 `record_memory` 时常遗漏 `title` 或 `category` 参数导致执行报错。
   - 根因：模型只能通过简要目录了解扩展工具，未见完整 Schema；同时工具实现缺乏对常见别名（如 `architecture`）与省略标题的智能回退容错机制。

---

## 二、 核心架构原则与设计模式

为了杜绝“堆叠式补丁”，本次改造严格遵循以下系统级架构设计原则：

```
+----------------------------------------------------------------------------------------------------+
|                                    大模型交互层 (LLM Context Boundary)                             |
|  1. 原生常驻核心工具集 (12 原生工具 + lookup_tool) 保持 100% 恒定 -> 保证 LLM Prompt 前缀缓存高命中率 |
|  2. 扩展工具目录 (TypeScript 函数签名目录) 注入系统提示词 -> 提供参数名与选填标注 (? 标识)        |
+----------------------------------------------------------------------------------------------------+
                                      |                                   |
                  原生调用: lookup_tool({ tool_name })    原生调用: call_extended_tool({ tool_name, args })
                                      |                                   |
                                      v                                   v
+----------------------------------------------------------------------------------------------------+
|                                  扩展工具代理网关 (Extended Tool Gateway)                          |
|  - 拦截 help 请求 -> 返回工具描述与完整 JSON Schema                                                |
|  - 代理转发目标工具执行 -> 支持静态工具与动态 MCP 工具                                             |
|  - 自动纠错保护 -> 执行失败时自动在报错后附加目标工具的完整 Schema 与修复指引                      |
+----------------------------------------------------------------------------------------------------+
                                      |
                                      v
+----------------------------------------------------------------------------------------------------+
|                               有效工具语义解构层 (Effective Tool Layer)                            |
|  - 后端 Hooks：自动提取 target_tool 关联的 subprocess_id、plan_file_path、memory_file_path          |
|  - 前端 Card：通过 resolveEffectiveTool 统一呈现 SubprocessBranchTree、方案树与记忆卡片            |
+----------------------------------------------------------------------------------------------------+
```

---

## 三、 详细实施机制

### 1. 两阶段工具检索与代理网关 (Two-Stage Tool Retrieval Gateway)

#### 1.1 原生常驻工具集严格收敛 (`CORE_TOOL_NAMES`)
系统下发给大模型的原生顶级工具严格保持为 13 个，无论主会话还是子会话结构高度统一：
- **文件与搜索**：`read_file`, `list_dir`, `file_outline`, `glob`, `grep`, `write_file`, `edit_file`
- **执行与任务**：`run_command`, `todo`, `create_plan`, `update_plan`
- **工具查询**：`lookup_tool`
- **扩展代理**：`call_extended_tool`

#### 1.2 TypeScript 紧凑签名目录与系统提示词规范
在 `static_system.rs` 中为所有扩展工具提供清晰的签名目录，消除参数臆测：
```typescript
// 子任务与协作者编排：
spawn_subagent(role: string, task: string, title?: string, subpath?: string)
wait_subagents(timeout_secs?: number)
get_subagent_status(subagent_id?: string)
stop_subagent(subagent_id: string)
dispatch_collaborator(collaborator_id: string, task: string, context?: object)
wait_collaborators(timeout_secs?: number)
get_collaborators()

// 知识沉淀与技能：
record_memory(content: string, title?: string, category?: string)
read_memory(query: string, category?: string)
list_skills()
run_skill(skill_name: string, args?: object)
```

#### 1.3 `lookup_tool` 独立原生工具
提供只读工具，允许大模型在入参不确定时主动查询目标工具的详细规范与 JSON Schema：
```json
{
  "name": "lookup_tool",
  "description": "查询工具的详细功能说明与完整 JSON 参数 Schema...",
  "parameters": {
    "type": "object",
    "properties": {
      "tool_name": { "type": "string" }
    },
    "required": ["tool_name"]
  }
}
```

#### 1.4 网关自动纠错与 Schema 智能回退 (Auto-Spec Recovery)
大模型若未经查询直接调用扩展工具导致参数校验失败（如缺少参数），`call_extended_tool` 捕获该错误并自动在错误信息末尾附加上该工具的完整 JSON Schema，提示模型在下一步立即自纠：
```
执行失败: 缺少 content 参数

【工具 `record_memory` 参数规范与纠错指引】
功能描述：在多步探索或代码阅读中，及时将关键技术栈...
参数 Schema：
```json
{ ... }
```
请核对必填参数并修正后重试。
```

---

### 2. 有效工具解包与生命周期统一治理 (Effective Tool Semantic Unboxing)

#### 2.1 后端生命周期钩子解包
在 `agent.rs` 中，对所有工具执行完毕后的生命周期 Hook 实施统一解包：
```rust
let (eval_name, eval_args) = if tool_name == "call_extended_tool" {
    let target = args.get("tool_name").and_then(|v| v.as_str()).unwrap_or(tool_name);
    let sub = args.get("arguments").unwrap_or(args);
    (target, sub)
} else {
    (tool_name, args)
};
```
无论子任务是通过直接调用还是通过 `call_extended_tool` 调起，均可正确写入 `tool_events.subprocess_id`，确保任务树持久化完全无缝。

#### 2.2 前端卡片多态解包 (`resolveEffectiveTool`)
在 `ToolCard.tsx` 中解构有效工具信息：
```typescript
export function resolveEffectiveTool(ev: ToolEvent): EffectiveToolInfo {
  if (ev.toolName === "call_extended_tool" && ev.params?.tool_name) {
    return {
      name: String(ev.params.tool_name),
      args: ev.params.arguments ?? {},
      isExtendedWrapper: true,
      rawToolName: ev.toolName,
    };
  }
  return {
    name: ev.toolName,
    args: ev.params ?? {},
    isExtendedWrapper: false,
    rawToolName: ev.toolName,
  };
}
```
界面卡片依据 `effective.name` 自动判定分支树、方案卡片或记忆卡片，完全恢复原有丰富的可视化控件与交互特性。

---

### 3. 子任务状态机防线与级联停止保证

#### 3.1 终态硬锁与防二次误唤醒 (Terminal Hard Lock)
- **`determine_subagent_status` 优先判定终态**：如果数据库中 `session_status == "completed"` 或已存在 `delivered_outcome` KV 记录，物理判定为完成，严禁因末尾是工具调用而误判为 `interrupted`。
- **`wait_subagents_tool` 拦截唤醒**：在决定是否调用 `restart_subagent` 前，强制校验是否已完结。
- **`restart_subagent` 入口防御**：若会话已为 `completed`，直接拒绝重启请求，返回错误。

#### 3.2 级联停止全链路同步 (Cascade Stop Guarantees)
在 `agent::stop_session_ext` 中：
- 级联停止子任务时，同时发射 `run:status(cancelled)` 与 `run:status(idle)` 事件，并向父会话发送 `subprocesses:changed` 广播。
- 当任一子任务被单独停止时，自动追溯其 `parent_session_id` 并通知父会话重新加载子任务清单，界面无残留转圈。

---

### 4. 反卡顿与前端渲染性能治理 (Anti-Lag Governance)

#### 4.1 非激活后台会话流式抑制与脏标记
- **`onMessageDelta` & `onMessageReasoningDelta` 窗口隔离**：
  若接收到的增量属于后台非激活会话（`sessionId !== currentId`），立即调用 `markSessionDirty(sessionId)` 并提前退出，不将成百上千个文本碎片压入 React 全局状态。
- **`messages:changed` 惰性加载**：
  仅在当前激活会话接收到变化通知时调用 `reloadMessages`；后台会话仅标记为脏，待用户切换至该会话时由 `selectSession` / `setCurrent` 按需单次拉取。

#### 4.2 稳定 Selector 引用
`currentMessages` 选择器返回当前会话已有消息列表引用，杜绝在选择器执行时无意义调用 `sortMessages` 生成新数组，从底层阻断全量重渲染。

#### 4.3 独立 Token 累加管道 (Token Rollup Independence)
Token 累加由后端 Rust 内核直接在 SQLite 数据表中通过递归更新父会话完成：
```rust
increment_session_tokens(conn, &pid, prompt_tokens, completion_tokens, total_tokens, cached_tokens);
```
随后直接广播 `session:update`。该管道完全脱离消息文本渲染逻辑，确保即便前端采用消息惰性加载，主会话顶部与看板上的 Token 统计依然 100% 实时精准。

---

## 四、 深度治理：安全策略误报消除、子任务等待收敛与流式节流反卡顿

在真实大模型复杂长对话测试中，深入发掘并彻底解决了以下 4 项核心缺陷：

### 1. 高危命令正则边界精细化（消除关键字误报）
- **现象**：子 Agent 在代码检索时执行 `Select-String -Path ... -Pattern '...Shutdown...'`，因包含 `Shutdown` 关键字，触发了“无条件高危命令（shutdown）”拦截，强制单次审批且全权模式无效。
- **根因**：原 `UNCONDITIONAL_RE` 中使用裸词正则 `(?i)\bshutdown\b`、`(?i)\breboot\b`，无命令位置感知，误伤了字符串字面量、正则模式参数与代码注释。
- **治理**：
  - 将高危正则严格限定在命令起始边界 `(?:^|[;&|`\n\(\)])\s*(?:sudo\s+)?`。
  - Windows `shutdown` 严格匹配参数动作开关（如 `/s`, `/r`, `/h`, `-s` 等）或整行执行；PowerShell 匹配 `Stop-Computer`。
  - `reboot` 匹配命令位置上的 `reboot` 或 `Restart-Computer`；`mkfs` 与 `format` 必须携带文件系统或盘符参数。
  - 增补单元测试 `test_benign_commands_with_high_danger_keywords_allowed`，杜绝一切模式匹配误拦截。

### 2. 跨工作区只读操作策略解绑（`full_access` 模式放行）
- **现象**：用户在 `full_access`（完全访问模式）下使用 `grep` / `read_file` 检索外部项目目录（如 `D:\WorkSpace\Vue\...`），弹窗提示“检测到高危命令，仅允许逐次确认，不可记忆放行：访问工作区之外的路径”。
- **根因**：`evaluate_path_tool` 只要检测到目标路径越出工作区（`ZoneResolver::resolve_zone == SecurityZone::Critical`），未先检查访问模式与 `Risk::ReadOnly`，便无条件强加 `force_once: true` 审批。
- **治理**：
  - **只读操作放行**：对于 `Risk::ReadOnly`（`grep`, `glob`, `read_file`, `list_dir`, `file_outline`），在 `full_access` 模式下直接 `Allow { scope: "mode" }` 免审执行。
  - **确认模式会话记忆**：在 `confirm` 模式下访问外部路径，标记 `force_once: false`，允许用户勾选“本会话允许”，避免逐个文件连续弹窗。
  - **破坏性隔离守底**：`.git` 核心元数据的写入/篡改与越界破坏性删除依然受严格保护。

### 3. 子任务等待生命周期与审批穿透治理（`wait_subagents`）
- **现象**：当子任务达到 60s 等待超时，主 Agent 误将子任务未完成时的初步思考（如“我先按指定范围阅读文档相关章节”）当做最终报告，直接产出结案答复，跳过了后续等待。
- **根因**：
  - `wait_subagents_tool` 未检测 `state.approvals`，当子任务被审批拦截时依然呈现为通用 running，超时后直接返回。
  - 未完成子任务的临时文本被直接格式化为 `- 交付核心结论：`，大模型判定已有结论并提前结案。
- **治理**：
  - **审批状态感知**：全面穿透 `state.approvals`，若子任务等待用户审批，状态显式标为 `pending_approval ("🟡 等待用户审批中")`，并呈现拦截原因。
  - **阶段进展与结论脱钩**：未完成（running / pending_approval / interrupted）的子任务，其摘要输出为 `- 阶段性进展（⚠️ 非最终交付结论，仍在推进中）`。
  - **硬性系统指令约束**：若存在未完成或超时子 Agent，工具输出顶层与底层强制注入硬约束指令，严禁直接结案，强制模型再次调用 `wait_subagents` 持续等待。

### 4. 激活窗口流式节流与事件广播去重（彻底消除界面卡顿）
- **现象**：长回复流式输出（如上万字方案、多段 KaTeX/代码）期间，浏览器主线程被占满，界面打字与滚动严重掉帧。
- **根因**：
  - 每秒到达数十个 `message:delta`，每次 delta 均直接触发 React Markdown 重新解析整篇上万字文档的 AST 树与 KaTeX，造成高频重排与卡顿。
  - 后端同时广播 `session:update` 与 `session:updated` 造成 4 倍重复重渲染。
- **治理**：
  - **流式 Delta 节流聚合**：在 `src/store.ts` 中实现 ~48ms（约 20 FPS）的增量缓冲区，高频到达的 Token 先在内存累加，定期合并推送至 Zustand 状态，渲染计算量降低 80%~90%。
  - **最终消息即时同步**：在 `onMessageFinal`、`setCurrent`（会话切换）与 `onStepFinished` 时立即刷新缓冲区，杜绝延迟或丢字。
  - **事件广播去重**：删除冗余的 `session:updated` 监听与广播，统一收敛至标准 `session:update`。

---

## 五、 验证与自测保障

本次改造涵盖了前端构建与全量 Rust 后端单元测试：
1. **TypeScript 编译与打包**：
   - `npm run build` 执行通过，4697 个模块构建无任何类型与语法错误。
2. **后端测试全量覆盖**：
   - 核心工具收敛与目录测试：`test_core_tools_contain_call_extended_tool_and_catalog_filtering` 通过。
   - `lookup_tool` 原生检索与代理纠错测试：`test_call_extended_tool_help` 通过。
   - 安全策略与安全域解析：`test_file_tools_zones`, `test_ephemeral_cache_cleanup_allowed_in_full_access`, `test_benign_commands_with_high_danger_keywords_allowed`, `test_kill_by_name_denied` 通过。
   - 全库 215 个 Rust 单元测试全部顺利通过（215 passed; 0 failed）。
