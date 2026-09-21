# 多步执行过程聚合收敛与子任务单轨工作区治理规范 (EXECUTION_PROCESS_AND_SUBTASK_GOVERNANCE_SPEC)

| 项目 | 内容 |
| --- | --- |
| 文档名称 | 多步执行过程聚合收敛与子任务单轨工作区治理规范 |
| 版本 | v1.0 |
| 状态 | 生产已落地已发布 |
| 关联模块 | `src/components` (React / Tailwind / UI), `src/types.ts`, `src-tauri` (Rust / Agent / Tools) |
| 运行位置 | `F:\Software\HarnessMini\harness-mini.exe` |

---

## 1. 背景与核心价值

在 `harness_mini` 面向复杂工程场景的实际使用中，AI Agent 在处理多步任务（如代码全局分析、方案制定、跨模块排查）时暴露了两大体验瓶颈：

1. **子任务臃肿化与路径迷失**：
   - 原系统为子任务提供了独立 `workspace` 和 `subpath`（重点关注目录）设置，导致主 Agent 派发时产生路径歧义；
   - 子 Agent 缺乏步数预算与收敛意识，在大型文件中陷入多轮分页微切片（`offset_line`）式逐行深挖，超过 50 步上限强制中止，耗费海量 Token 却无法生成总结。
2. **对话完成态的“结果污染”与视觉混乱**：
   - 在多步执行中（例如一轮对话包含 23 个 assistant 步骤），大模型在每调用一个工具前都会输出过渡性意图描述（如“*收到需求，我先分析...*”、“*先建立任务清单：*”、“*表结构已明确，继续看后端...*”）；
   - 前端无差别地将每一步渲染为一级 14px 白色高亮正文气泡，并在每个步骤底部都悬挂“复制”按钮；
   - **核心矛盾**：用户在**等待过程中需要即时看到反馈**以确认 AI 正在思考；但在**对话完成后**，界面上散落着 20 多个过程碎语，真正重要的最终交付结果被严重稀释，用户翻找和复制结果极度困难。

本规范全面总结了对上述两类问题的彻底治理方案：**子任务单轨工作区与步数强制收敛**，以及**多步执行过程与交付结果聚合分离规范（方案 A）**。

---

## 2. 总体架构与数据流图

```mermaid
flowchart TD
    User([用户指令输入]) --> MainAgent[主 Agent 会话 (ChatView)]

    subgraph 子任务单轨工作区治理
        MainAgent -->|派发子任务 (spawn_subprocess)| SubAgent[子 Agent / 临时子进程]
        MainAgent -.->|继承物理工作区根目录| SubAgent
        SubAgent -->|检测剩余可用步数| StepCheck{可用步数 <= 3 ?}
        StepCheck -- 是 --> ForceWarn[注入临界预警: 严禁继续调工具, 强制总结交付]
        StepCheck -- 否 --> ToolExec[执行 grep / glob / outline 宏观定位]
    end

    subgraph 方案 A: 过程与结果聚合分离架构
        MainAgent --> TurnMsgStream[轮次消息流: 连续 N 步 Assistant]
        TurnMsgStream --> Aggregator[groupTimelineItems 轮次聚合引擎]
        Aggregator --> ProcessSteps[前 N-1 步: 中间意图说明 + 工具卡片]
        Aggregator --> FinalStep[最后 1 步: 无工具调用的最终成果报告]

        ProcessSteps --> ProcessDrawer[ExecutionProcessBlock 执行过程抽屉]
        FinalStep --> FinalMessage[MessageItem 一级主回复气泡]
    end

    ProcessDrawer -.->|运行中| StateOpen[默认展开: 实时可见思考与工具]
    ProcessDrawer -.->|完成后| StateClosed[自动收起: 已完成 N 步摘要条]
    ProcessDrawer --> NoCopy[去除明细内多余复制按钮<br/>100% 保留工具自身复制功能]
    FinalMessage --> FinalCopy[唯一挂载: 复制回复正文 + 轮次耗时/Token]
```

---

## 3. 子任务工作区单轨统一与轻量收敛规范

### 3.1 工作区单轨严格继承（取消分歧设置）
- **根目录一致性**：子任务的合法物理工作区必须与主进程保持 100% 完全一致，任何子任务都不再允许配置独立 `workspace` 或 `subpath`。
- **工具定义精简**：从 `spawn_subprocess` 工具参数定义中移除 `workspace` 与 `subpath`，仅保留 `role`（角色）、`title`（标题）、`task`（具体要求）。
- **任务目标定位规范**：若需要子任务聚焦某个模块，主 Agent 直接在 `task` 描述中指明目标相对路径（如“*负责分析 src/components/signaturePad 目录下的...*”）。
- **界面精简**：`CreateSubagentModal` 移除工作区输入框，杜绝用户配置心智负担。

### 3.2 临界步数预警与防强杀机制
在 Rust 引擎执行循环（`agent.rs` 的 `run_loop`）中引入步数倒计时：
```rust
// 当剩余可用步数 <= 3 时，动态向上下文尾部注入强制收敛警报
if _step >= max_steps.saturating_sub(3) {
    let warn_prompt = format!(
        "【⚠️ 临界步数紧急预警：当前已达第 {_step} 步，仅剩最后 {} 步！严禁继续调用任何探索、排查或读取类工具！必须立即根据目前已掌握的所有信息，按规定结构化格式输出最终总结与交付回复！】",
        max_steps.saturating_sub(_step)
    );
    // 注入系统临时消息驱动模型迅速产出报告
}
```

### 3.3 探索方式精益化守则
在系统提示词中对子任务施加硬约束：
- **预算感知**：调研类任务明确要求在 10~15 步内收敛交付；
- **宏观定位优先**：优先使用 `glob` 查看文件树，使用 `file_outline` 查看大纲结构，使用 `grep` 定位关键符号；
- **严禁切片漫游**：严禁对大文件进行连续、多轮递增的 `offset_line` 逐行深挖；
- **严格职责边界**：严禁跨界反查工作区内与分配职责无关的其他模块（如前端任务禁止反查后端 Java/SQL）。

---

## 4. 多步执行过程与交付结果聚合分离规范（方案 A）

### 4.1 核心设计理念
- **等待中（Running）全透明**：AI 的“收到需求，我先分析..”以及每一步工具调用的过程文本必须实时呈现，满足用户等待时的确定性与反馈感；
- **完成后（Done）全收敛**：一旦本轮最终交付报告生成，所有中间 20 多个过渡步骤自动折叠，仅保留一行优雅的状态摘要栏，将视觉中心彻底让位于最终结果；
- **功能完备性**：
  - 去除中间过程明细内无意义的消息级“复制正文”按钮；
  - 100% 完好保留工具卡片（`ToolCard`）自带的命令行复制、提示词复制、文件路径复制、代码 diff 复制等能力。

---

### 4.2 轮次聚合引擎 (`groupTimelineItems`)

前端时间线数据流平铺结构通过 `groupTimelineItems` 算法重组为结构化层级：

```typescript
export type GroupedTimelineItem =
  | { type: "compaction"; compaction: SessionCompaction }
  | { type: "message"; msg: Message }
  | {
      type: "process";
      id: string;
      steps: Message[];
      turnMetrics?: TurnMetrics;
    };
```

#### 聚合判定逻辑：
1. **单步纯文本**：若一轮对话仅有 1 条 assistant 消息且不含工具调用，直接输出为普通 `message`，不渲染折叠栏；
2. **多步任务分离**：
   - 提取轮次中最后一条 assistant 消息（`lastMsg`）；
   - 若 `lastMsg` **不包含工具调用**（即最终文本交付结果）：
     - 将前面的第 `1` 至 `N-1` 步打包为 `type: "process"`（作为执行过程明细）；
     - 将 `lastMsg` 单独输出为 `type: "message"`（作为最终交付报告）；
   - 若 `lastMsg` **包含工具调用**（正在多步流式执行中，或中断在工具步）：
     - 将所有步骤打包为 `type: "process"`。

---

### 4.3 执行过程折叠抽屉 (`ExecutionProcessBlock`)

#### 组件视觉与交互规范：
```text
┌── 🛠️ 执行过程 (已完成 22 个步骤 · 38 次工具调用) ───────────── 用时 18.2s · 24,192 tokens  [展开详情 ▾] ──┐
│                                                                                                    │
│  （当展开时，内部按垂直时间线平铺展示每个步骤）                                                   │
│  ├─ 步骤 1 / 22 (用时 1.2s)                                                                       │
│  │  💬 [意图文本] 收到需求。这是一个涉及审批流程配置的功能调整，我先建立任务清单：                 │
│  │  └─ [✓ 任务清单 (todo)]                                                                         │
│  │                                                                                                │
│  ├─ 步骤 2 / 22 (用时 0.8s)                                                                       │
│  │  💬 [意图文本] 先召回已沉淀的审批流记忆，并行探索关键目录：                                     │
│  │  ├─ [✓ 读取记忆 (read_memory)]                                                                 │
│  │  └─ [✓ 查找文件 (glob)]                                                                        │
│  └─ ...                                                                                           │
└───────────────────────────────────────────────────────────────────────────────────────────────────┘

🎯 分析完成。以下是完整的调整方案与实施计划：
# 审批流程手写签名功能调整方案
[一、现状分析结论与实施步骤...]

[📋 复制回复正文]  [⏱️ 用时: 18.2s]  [⚡ Tokens: 24,192]
```

#### 动态展开机制：
- `userToggled` 状态锁：用户未主动点击时，`isOpen = isRunning`；
- 运行中默认展开，打字机实时跳动；
- 运行结束瞬间，自动平滑收起为单行摘要条；
- 用户点击可随时手动复盘展开或再次收起。

---

### 4.4 消息组件渲染层重构 (`MessageItem.tsx`)

#### 1. 时序因果纠正
原有时序为 `ToolCard` 在前、`msg.content` 在后，导致用户误认为文本是工具的执行输出。重构后调整为：
1. `ReasoningBlock`（内部思考通道）
2. `msg.content`（步骤意图说明）
3. `toolGroups`（工具卡片）

#### 2. `isProcessStep` 模式分流
在 `MessageItemProps` 中扩展 `isProcessStep?: boolean` 标志位：
- **处于过程抽屉内部时 (`isProcessStep === true`)**：
  - 意图文本以浅灰色卡片样式呈现（`text-[13px] text-ink/90 bg-panel2/40 border border-edge/40`），避免与最终结果混淆；
  - **隐藏底栏消息级“复制回复正文”按钮**；
  - **工具卡片原生复制按钮完全保留**（命令行复制、提示词复制、路径复制、diff 复制）。
- **处于最终答复时 (`isProcessStep === false`)**：
  - 恢复全尺寸 14px 一级 Markdown 渲染；
  - 保留唯一的“复制回复正文”按钮、整轮耗时与 Token 汇总。

---

## 5. 模块适配与修改文件对照表

| 文件路径 | 变更范畴 | 核心变动说明 |
| --- | --- | --- |
| [`src/components/ExecutionProcessBlock.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/ExecutionProcessBlock.tsx) | **[新建]** UI 组件 | 封装多步执行过程抽屉、状态计算与 `groupTimelineItems` 聚合算法 |
| [`src/components/MessageItem.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/MessageItem.tsx) | **[修改]** 消息呈现 | 支持 `isProcessStep`；调整时序为“意图在前、工具在后”；过程中隐藏消息复制按钮 |
| [`src/components/ChatView.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/ChatView.tsx) | **[修改]** 主会话视图 | 接入 `groupTimelineItems` 聚合时间线与 `ExecutionProcessBlock` 卡片 |
| [`src/components/CollaboratorView.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/CollaboratorView.tsx) | **[修改]** 协作者视图 | 全面接入执行过程聚合抽屉与中间步骤降噪 |
| [`src/components/SubagentView.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/SubagentView.tsx) | **[修改]** 子 Agent 视图 | 全面接入执行过程聚合抽屉 |
| [`src/components/SubprocessView.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/SubprocessView.tsx) | **[修改]** 临时子进程 | 全面接入执行过程聚合抽屉 |
| [`src-tauri/src/tools.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/tools.rs) | **[修改]** 后端工具 | `spawn_subprocess` 移除 `workspace` 与 `subpath` 参数；工作区统一继承 |
| [`src-tauri/src/agent.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/agent.rs) | **[修改]** 执行引擎 | `run_loop` 引入剩余步数紧急预警；优化子任务精益探索提示词 |
| [`src-tauri/src/commands.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/commands.rs) | **[修改]** 命令接口 | 子任务派发严格单轨继承父会话工作区 |
| [`src/components/CreateSubagentModal.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/CreateSubagentModal.tsx) | **[修改]** 弹窗交互 | 移除工作区与重点关注目录输入项，提示在 task 描述中指定目录 |

---

## 6. 验证与生产部署指南

### 6.1 自动化质量验证
1. **前端类型校验与打包**：
   ```bash
   npm run build
   # 输出：✓ built in 5.05s, 0 errors
   ```
2. **后端 Rust 单元测试**：
   ```bash
   cd src-tauri
   cargo test
   # 输出：test result: ok. 79 passed; 0 failed
   ```

### 6.2 二进制发布与热更新
本工程实际生产环境部署于 `F:\Software\HarnessMini`。在 Windows 环境下更新正被运行占用的可执行文件时，采用 NTFS 重命名热替换方案：
```powershell
# 1. 编译 Release 产物
cd f:\WorkSpace\Other\harness_mini\src-tauri
cargo build --release

# 2. 对已锁定的当前 exe 进行更名规避文件占用
Rename-Item -Path 'F:\Software\HarnessMini\harness-mini.exe' -NewName 'harness-mini.exe.old'

# 3. 将新产物拷入就位
Copy-Item 'f:\WorkSpace\Other\harness_mini\src-tauri\target\release\harness-mini.exe' 'F:\Software\HarnessMini\harness-mini.exe'
```
用户仅需重启应用即可完全载入最新版本。

---

## 7. 总结与后续演进建议

通过本次“**子任务单轨工作区统一**”与“**方案 A：过程与结果聚合分离**”的落地，系统实现了以下重大体验提升：
1. **即时性与整洁性并存**：执行时有打字反馈，完成后只留交付成果；
2. **信息层级分明**：20 多步复杂任务的阅读负担由原来需滚动 5~10 屏骤降为一屏直达结论；
3. **操作体验精准**：复制按钮唯独服务于最终交付内容，工具层复制能力 100% 完整保留；
4. **架构清晰单轨**：彻底消除子任务多工作区路径歧义与步数超限崩溃。
