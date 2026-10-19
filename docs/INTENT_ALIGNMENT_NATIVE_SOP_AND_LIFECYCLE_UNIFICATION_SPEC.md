# 内生型意图对齐 SOP 与执行生命周期统一治理规范 (INTENT_ALIGNMENT_NATIVE_SOP_AND_LIFECYCLE_UNIFICATION_SPEC)

| 属性 | 说明 |
| :--- | :--- |
| **文档代号** | `INTENT_ALIGNMENT_NATIVE_SOP_AND_LIFECYCLE_UNIFICATION_SPEC` |
| **创建时间** | 2026-10-19 |
| **适用范围** | `harness_mini`（Tauri 2 + Rust + React 桌面 AI Agent 架构体系） |
| **关联核心模块** | [`src-tauri/src/agent.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/agent.rs), [`src-tauri/src/tools.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/tools.rs), [`src-tauri/src/jev.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/jev.rs), [`src/types.ts`](file:///f:/WorkSpace/Other/harness_mini/src/types.ts), [`src/components/ToolCard.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/ToolCard.tsx) |
| **知识密级** | 核心架构演进与智能体调度生命周期治理规范 |

---

## 目录

1. [演进背景与痛点深度复盘](#一演进背景与痛点深度复盘)
   - 1.1 [历史沿革：从“外置旁路门禁”到“伪 SOP 硬门禁”](#11-历史沿革从外置旁路门禁到伪-sop-硬门禁)
   - 1.2 [典型线上故障：大模型口头预告导致“假死早退”](#12-典型线上故障大模型口头预告导致假死早退)
   - 1.3 [深层架构成因：物理隔离与 ReAct 单退出门冲突](#13-深层架构成因物理隔离与-react-单退出门冲突)
2. [内生型 SOP 架构设计原则](#二内生型-sop-架构设计原则)
   - 2.1 [去物理门禁，回归统一生命周期（Single Lifecycle）](#21-去物理门禁回归统一生命周期single-lifecycle)
   - 2.2 [工具全量透传与零盲搜守则（Full Tool Availability）](#22-工具全量透传与零盲搜守则full-tool-availability)
   - 2.3 [软性规范引导与人机协同审批闭环（Soft Guidance + Human in the loop）](#23-软性规范引导与人机协同审批闭环soft-guidance--human-in-the-loop)
3. [核心改造与代码实施细节](#三核心改造与代码实施细节)
   - 3.1 [彻底消除 Phase 1 临时状态锁与提示词替换](#31-彻底消除-phase-1-临时状态锁与提示词替换)
   - 3.2 [抹除工具下发物理过滤（统一全量 Schemas）](#32-抹除工具下发物理过滤统一全量-schemas)
   - 3.3 [重构 `intent_alignment_gate` 原生 SOP 规则](#33-重构-intent_alignment_gate-原生-sop-规则)
   - 3.4 [清理各 SOP 标题中残留的层级硬隔离标记](#34-清理各-sop-标题中残留的层级硬隔离标记)
4. [端到端执行流对比](#四端到端执行流对比)
   - 4.1 [重构前（物理硬门禁流）](#41-重构前物理硬门禁流)
   - 4.2 [重构后（内生型统一 SOP 流）](#42-重构后内生型统一-sop-流)
5. [质量基线与回归验证结论](#五质量基线与回归验证结论)

---

## 一、演进背景与痛点深度复盘

### 1.1 历史沿革：从“外置旁路门禁”到“伪 SOP 硬门禁”
1. **阶段一（外置旁路门禁）**：早期通过独立的 `run_intent_alignment_gate` 函数进行前置准入判定。由于其脱离会话主线程，造成了严重的**“上下文脱节”**（模型看不到完整对话历史）以及**“过程黑盒不可视”**（用户端无法实时看到判定过程）。
2. **阶段二（SOP 化重构）**：为了让决策流程内生于会话，引入了 `intent_alignment_gate` SOP，希望模型在主对话中通过调用决策工具（`judge_decision` / `choice_decision` / `score_decision`）进行自检，并通过前端 `ToolCard` 渲染透明卡片。
3. **阶段三（物理两阶段回潮困境）**：在随后的代码迭代中，为了防止所谓“提示词前摄干扰”，系统在 `agent.rs` 引入了 **Phase 1 物理隔离机制**：
   - 在 Step 0 强行将统一提示词替换为极简的 `phase1_intent_system_prompt`；
   - 物理收缴了所有代码读写与探索工具，强行仅下发 3 个决策工具；
   - 设置了 `intent_alignment_done` 布尔锁，要求必须触发决策工具成功才放行进入 Phase 2。
   - **后果**：名义上是 SOP，底层实现上却退化为了一个**披着 SOP 外皮的硬闸机门禁**。

### 1.2 典型线上故障：大模型口头预告导致“假死早退”
在处理复杂工程指令（例如：“1、保留现有弹窗形式... 2、优先用现有的 vxe-table... 3、详细说明 list 需改造为按身份证号聚合...”）时，出现了严重的突发停止故障：
- **故障现象**：助手输出了 *“我先验证 vxe-table 树形能力，再详细展开 list 聚合改造流程并更新计划。”* 后，**没有调用任何工具，也没有任何后续输出，任务直接结束**。
- **用户困惑**：AI 明明说“我先去验证”，为什么说了半截话就直接断了？

### 1.3 深层架构成因：物理隔离与 ReAct 单退出门冲突
查看当时的调度核心逻辑：
```rust
// 缺陷代码：在 ReAct 单循环中杂糅门禁与生产执行
if result.tool_calls.is_empty() {
    return (RunOutcome::Done, last_assistant_id, run_tokens);
}
```
- 模型在 Phase 1 受到提示词约束（*“正文文本必须保持极致极简，只输出两句话意向，严禁展开具体实现”*），精准生成了两句行动预告；
- 由于 Phase 1 物理屏蔽了 `grep`、`read_file` 等所有探索工具，模型此时无法发起代码探索；
- 若模型在当前步骤未触发决策工具（`decision_schemas`），`result.tool_calls` 即为空；
- **调度引擎缺乏阶段契约校验，将这句口头预告误判为“已完成全部任务最终交付”，当场退出了 `run_once` 循环**。

---

## 二、内生型 SOP 架构设计原则

针对上述架构痛点，本次治理遵循**工业级 Agent（如 Claude Code、Cline、LangGraph）的去门禁化设计哲学**：

### 2.1 去物理门禁，回归统一生命周期（Single Lifecycle）
- 彻底废除人工制造的 `Phase 1 vs Phase 2` 物理断层；
- 不再在主循环中做 System Prompt 的临时偷换，自始至终下发统一、完整的全局系统提示词；
- 不再维护 `intent_alignment_done` 等脆弱的状态锁。

### 2.2 工具全量透传与零盲搜守则（Full Tool Availability）
- 从第一步（Step 0）起，全生命周期统一下发全量工具集 `schemas`；
- 意图对齐与摸排阶段，完全允许且推荐模型使用 `glob`、`grep`、`read_file` 查阅最小必要上下文，杜绝“既要模型搞清业务意图，又不给模型看工程代码”的矛盾设计。

### 2.3 软性规范引导与人机协同审批闭环（Soft Guidance + Human in the loop）
- 意图对齐作为一条正规 SOP 规则写入系统提示词，引导模型面对新任务时优先评估明确性；
- 若模型调用决策工具且质检存疑（得分 < 90 或 judge = false），底层依然无缝触发 `pending_approval` 挂起审批流，弹出前端交互卡片；
- 强化执行连续性约束：严禁口头声明“我先看下...”却不调用工具，有动作必须随同发起工具调用。

---

## 三、核心改造与代码实施细节

修改完全收敛在 [`src-tauri/src/agent.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/agent.rs)。

### 3.1 彻底消除 Phase 1 临时状态锁与提示词替换
移除 `run_once` 中初始化的临时状态变量与死锁判断：
```rust
// 移除前：脆弱的双阶段切换状态
let is_sub = is_subagent || ...;
let enable_phase1 = !is_sub && !settings.disabled_sops.iter().any(|s| s == "intent_alignment_gate") && !decision_schemas.is_empty();
let mut intent_alignment_done = !enable_phase1;
let phase1_sys = phase1_intent_system_prompt(&session);

// 移除后：统一上下文提示词
let current_sys = &sys;
```
彻底删除独立函数 `phase1_intent_system_prompt`。

### 3.2 抹除工具下发物理过滤（统一全量 Schemas）
移除 `decision_schemas` 过滤与 `step_schemas` 分支下发：
```rust
// 移除前：在 Step 0 剥夺生产工具
let step_schemas = if is_in_phase1 { decision_schemas.clone() } else { schemas.clone() };
let call = llm::chat_stream(&cfg, &messages, &step_schemas, ...).await;

// 移除后：自始至终下发完整工具链
let call = llm::chat_stream(&cfg, &messages, &schemas, ...).await;
```

### 3.3 重构 `intent_alignment_gate` 原生 SOP 规则
在 `system_prompt` 中，将原先充满物理禁令（“严禁调用除决策工具外的任何工具”、“必须只输出两句话”）的硬门禁提示词，重构为符合现代 Agent 协同准则的内生 SOP：
```rust
if is_sop_enabled("intent_alignment_gate") {
    rules.push(format!(
        "{rule_num}. 【首步意图定性与场景化决策质检 SOP】：\n   \
         - 触发时机：当你接收到用户发送的新任务、功能需求、代码修改、生图或技术问题时，在开始实际工程改动前，优先进行意图对齐与决策质检；日常问候或纯文本简答直接回复；后续分步执行步骤无需重复质检。\n   \
         - 场景化决策工具选型（按需调用）：\n       \
           * **简单即时任务**（如单点问答、简单生图、单处微调）：优先调用 `judge_decision`，判断“该需求是否目标完全明确且无歧义，可直接执行？（True/False）”；\n       \
           * **方案存在分支路线**（如直接做 vs 委派协作者 vs 多个实现方案）：优先调用 `choice_decision`，从备选策略中裁决最优项；\n       \
           * **复杂多步业务改造 / 系统重构**：调用 `score_decision`，对核心意图理解与关键计划进行百分制评分（≥90分放行）。\n   \
         - 人机协同审批机制：\n     \
           * 若质检存疑（得分<90分 或 judge=false）：系统将就地挂起并弹出确认卡片由用户亲自定夺。若用户点击【采纳方案】，工具将放行通知你继续；若用户点击【补充指导】，你必须在下一步严格依据用户的补充指导修正方案！\n   \
         - 调研探索与执行连贯性：\n     \
           * 探索与调研阶段可正常调用只读工具（glob / grep / list_dir / file_outline / read_file）摸排代码；\n     \
           * 【严禁空口预告后中断】：严禁仅在回复文本中输出“我先看下/我来验证/稍后更新”等行动预告却不发起任何工具调用！若当前步骤需要探索、修改或更新计划，必须直接发起对应工具调用，保持连贯执行直至阶段交付。"
    ));
    rule_num += 1;
}
```

### 3.4 清理各 SOP 标题中残留的层级硬隔离标记
将 `system_prompt` 中残留的历史层级标签归一化：
- `【第二层级 · 严格规划先行与确认门禁准则（Always Plan 模式，意图质检放行后生效）】`  
  👉 `【严格规划先行与确认门禁准则（Always Plan 模式）】`
- `【第二层级 · 方案先行与任务计划中枢治理规范（防需求失真，意图质检放行后生效）】`  
  👉 `【方案先行与任务计划中枢治理规范（防需求失真）】`
- `【第二层级 · 任务清单全生命周期 SOP（意图质检放行后生效）】`  
  👉 `【任务清单全生命周期 SOP】`
- `【第二层级 · 精益代码研读与克制探索 SOP（意图质检放行后生效）】`  
  👉 `【精益代码研读与克制探索 SOP】`

---

## 四、端到端执行流对比

### 4.1 重构前（物理硬门禁流）
```
用户输入: "帮我改造医疗纠纷模块..."
   │
   ▼
[Step 0]: 进入 Phase 1 物理黑盒
   ├── 提示词被强行截断为 phase1_intent_system_prompt
   ├── 工具集被物理削减为仅 3 个决策工具 (只剩 judge/choice/score)
   ├── 模型生成: "我先验证 vxe-table 树形能力..." (遵守双句规则)
   ├── 模型未调用决策工具 (tool_calls 为空)
   └── 引擎检测: tool_calls.is_empty() == true ──► 误判交付完成 ──► ❌ 突然停止假死！
```

### 4.2 重构后（内生型统一 SOP 流）
```
用户输入: "帮我改造医疗纠纷模块..."
   │
   ▼
[Step 0]: 原生主循环 (统一提示词 + 全量生产工具)
   ├── 模型拥有完整工程规则与全量工具 (grep / read_file / judge_decision / create_plan)
   ├── 模型遵循 SOP 规范，边思考边发起 grep("vxe-table") 或 judge_decision
   ├── 工具调用原生落库并广播，前端 ToolCard 实时可视渲染
   │     ├── 若决策存疑 ──► 挂起并等待用户审批卡片定夺
   │     └── 若决策通过 / 调研执行 ──► 自动进入下一循环步
   └── 后续步骤顺畅推进：生成改造计划 ──► 编写代码 ──► 构建测试 ──► 最终交付完成！
```

---

## 五、质量基线与回归验证结论

修改完成后，执行 Rust 核心库全量单元测试套件：
```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib
```
**测试输出**：
```
test result: ok. 162 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.85s
```
**覆盖验证点**：
- `test_intent_alignment_sop_in_system_prompt`：验证 SOP 开启时准则正确注入，包含决策工具选型及防空口中断规则；
- `system_prompt_respects_disabled_sops`：验证在设置中禁用 `intent_alignment_gate` 时规范能精准剥离；
- 长任务生命周期、单向快照还原（`snapshot_revert`）、计划源文件溯源（`plan_source_attribution`）等全部 162 项核心测试 100% 通过。
- 架构回归单一生命周期，代码复杂度显著降低，彻底清除了因物理隔离导致任务突然早退的系统性风险。
