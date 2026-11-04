# 子进程协作鲁棒性加固与工具卡片有效语义解构修复实施方案

- 文档 ID: `SPEC-20261104-SUBAGENT-AND-TOOLCARD-REPAIR`
- 创建时间: 2026-11-04
- 责任模块: 前端 (`src/components/ToolCard.tsx`)、后端工具层 (`src-tauri/src/tools/subagent.rs`)、上下文系统提示词 (`src-tauri/src/context/static_system.rs`)
- 关联问题:
  1. `spawn_subagent` 频繁报错 `缺少 title 参数`
  2. `wait_subagents` 工具卡片内原本置顶的交付报告工件卡片未显示，且卡片展示排版错位
  3. 扩展工具网关解构不彻底导致的图标退化、交接文档失效、浏览器拦截失效等隐患

---

## 一、背景与问题根因分析

### 1. 创建子进程频繁报错 `缺少 title 参数`
- **契约定义冲突**：
  在 `src-tauri/src/context/static_system.rs`（第 285 行）中，给模型的扩展工具目录声明为：
  ```typescript
  spawn_subagent(role: string, task: string, title?: string, subpath?: string)
  ```
  模型遵循提示词规范，将 `title` 视为可选参数（带 `?` 号），在调用时常常只传 `role` 与 `task`。
- **后端硬编码拦截**：
  在 `src-tauri/src/tools/subagent.rs`（第 195 行）中：
  ```rust
  let title = args.get("title").and_then(|v| v.as_str()).ok_or("缺少 title 参数")?;
  ```
  后端执行硬性校验，缺少任何容错与缺省兜底策略，导致一旦模型未显式提供 `title` 便直接打断并报错。

### 2. 等待子进程工具卡片内报告卡片消失及排版错位
- **报告卡片消失根本原因**：
  在 11 月 3 日提交的 `ca7fbb61`（扩展工具网关收敛）中，所有扩展工具调用的顶层名称均变为 `ev.toolName === "call_extended_tool"`，真实工具名称解包至 `effective.name`。
  前端在 `ToolCard.tsx` 中遗漏了子任务交付报告的有效工具判断：
  - **路径提取逻辑（第 1907 行）**：
    ```tsx
    if (!["wait_subprocesses", "wait_subagents", "wait_collaborators"].includes(ev.toolName)) {
      return [];
    }
    ```
    此处使用原始 `ev.toolName`（值为 `"call_extended_tool"`），导致 `resolvedSubtaskReports` 恒为空数组。
  - **卡片渲染守卫（第 2381 行）**：
    ```tsx
    ["wait_subprocesses", "wait_subagents", "wait_collaborators"].includes(ev.toolName) &&
    resolvedSubtaskReports.length > 0
    ```
    双重拦截导致置顶的绿色【协同任务交付报告】卡片彻底不渲染。
- **“位置不对”的心理预期落差**：
  - **卡片内部**：原本卡片展开后顶部展示的是结构化交付报告卡片，下方折叠长文本；报告丢失后顶部直接裸露底层的巨幅原始文本，视觉重心失衡。
  - **时间线宏观排布**：主 Agent 在派生子进程（Seq 37）后并未立即等待，而是穿插执行了多达 33 步只读勘察与命令，导致 `wait_subagents` 出现在离派生点极远的下方，造成视觉割裂。

### 3. 其他扩展工具未解包隐患
- **交接文档卡片失效**（第 1971、2347 行）：`resolvedSubtaskKnowledgePath` 也判断了 `ev.toolName`，导致任务交接书卡片无法渲染。
- **图标与分类角标退化**（第 2126、2129 行）：`<ToolIcon name={ev.toolName} />` 传入 `call_extended_tool`，导致专用工具图标全部降级为通用扩展工具图标。
- **浏览器鉴权拦截失效**（第 2331 行）：`ev.toolName.startsWith("browser_")` 未解包，导致浏览器自动化工具被网关包装后，遇到未登录等鉴权拦截时无法弹出 `<AuthCard>` 供人工干预。
- **决策卡片过滤失效**（第 2326、2438 行）：同样存在使用 `ev.toolName` 的硬编码。

---

## 二、架构改造与实施设计

### 1. 后端参数解析鲁棒性加固（`src-tauri/src/tools/subagent.rs`）
- 对 `title` 进行智能兜底提取：
  1. 优先读取 `args["title"]`；
  2. 若缺省，尝试从 `args["task"]` 智能截取首行（或去除 Markdown 标题符号后的前 20 字符）；
  3. 若仍为空，使用 `format!("{role}任务")` 作为兜底标题。
- 对参数名幻觉做别名兼容：
  - 若 `args["role"]` 缺省，尝试读取 `args["subagent_type"]` 或 `args["subagent_role"]`。

### 2. 系统提示词与工具 Spec 契约校准
- 在 `src-tauri/src/context/static_system.rs` 中，对 `spawn_subagent` 的声明进行优化说明：明确推荐指定 `title`，并与 `specs.rs` 保持语义对齐。

### 3. 前端 `ToolCard.tsx` 全面有效语义解构
将 `ToolCard.tsx` 中所有依赖 `ev.toolName` 的业务判定，彻底替换为有效工具名 `effective.name` 及规整后的 `normalizedEv`：
1. **交付报告提取**：`["wait_subprocesses", "wait_subagents", "wait_collaborators"].includes(effective.name)`
2. **交接书提取**：`["spawn_subprocess", "spawn_subagent", "dispatch_collaborator"].includes(effective.name)`
3. **交付报告卡片渲染**：使用 `effective.name`
4. **交接书卡片渲染**：使用 `effective.name`
5. **卡片图标与分类徽标**：`<ToolIcon name={effective.name} />`、`toolCategoryBadge(effective.name)`
6. **浏览器鉴权拦截**：`effective.name.startsWith("browser_")`
7. **决策工具判定**：统一使用 `effective.name`

---

## 三、代码级修改清单与具体实现

### 修改 1：`src-tauri/src/tools/subagent.rs`
#### 变更点：增加参数别名映射与智能标题生成兜底
```rust
pub(crate) async fn spawn_subagent_tool(args: &Value, ctx: &ToolCtx) -> Result<String, String> {
    let host = ctx.host.as_ref().ok_or("内部错误：缺少宿主上下文")?;
    
    // 1. 角色提取（兼容 subagent_type / subagent_role 别名）
    let role = args.get("role")
        .or_else(|| args.get("subagent_type"))
        .or_else(|| args.get("subagent_role"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or("缺少 role 参数（子 Agent 角色定位）")?;

    // 2. 任务提取
    let task = args.get("task")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or("缺少 task 参数（详细职责与交付验收要求）")?;

    // 3. 标题智能兜底生成：优先取 title，若无则自动从 task 或 role 提炼
    let fallback_title;
    let title = if let Some(t) = args.get("title").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()) {
        t
    } else {
        // 从 task 中截取前 25 个有效字符作为简要标题
        let clean_task = task.lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or(task)
            .trim_start_matches(|c: char| c == '#' || c == '*' || c == '-' || c == ' ' || c == '【' || c == '】');
        let truncated: String = clean_task.chars().take(25).collect();
        fallback_title = if truncated.is_empty() {
            format!("{role}协同任务")
        } else {
            truncated
        };
        &fallback_title
    };
    // ...
```

---

### 修改 2：`src/components/ToolCard.tsx`
#### 变更点：有效语义全面渗透替换

```diff
@@ -1905,9 +1905,9 @@ export function ToolCard({ ev }: { ev: ToolEvent }) {
   // 提取子任务/协作者交付报告工件的物理路径列表（.harness/subtasks/*_report.md），支持单任务与多任务并行返回
   const resolvedSubtaskReports = useMemo(() => {
-    if (!["wait_subprocesses", "wait_subagents", "wait_collaborators"].includes(ev.toolName)) {
+    if (!["wait_subprocesses", "wait_subagents", "wait_collaborators"].includes(effective.name)) {
       return [];
     }
     if (!ev.resultText) return [];
@@ -1969,9 +1969,9 @@ export function ToolCard({ ev }: { ev: ToolEvent }) {
   // 提取子任务/协作者任务知识交接书工件的物理路径（.harness/subtasks/*_knowledge.md）
   const resolvedSubtaskKnowledgePath = useMemo(() => {
-    if (!["spawn_subprocess", "spawn_subagent", "dispatch_collaborator"].includes(ev.toolName)) {
+    if (!["spawn_subprocess", "spawn_subagent", "dispatch_collaborator"].includes(effective.name)) {
       return null;
     }
@@ -2124,9 +2124,9 @@ export function ToolCard({ ev }: { ev: ToolEvent }) {
       <div className="...">
         <div className="...">
-          <ToolIcon name={ev.toolName} />
-          {toolCategoryBadge(ev.toolName)}
+          <ToolIcon name={effective.name} />
+          {toolCategoryBadge(effective.name)}
         </div>
@@ -2324,9 +2324,9 @@ export function ToolCard({ ev }: { ev: ToolEvent }) {
       {/* 展开时：若是决策工具，展示专用决策卡片 */}
-      {expanded && ["judge_decision", "choice_decision", "score_decision"].includes(ev.toolName) && (
+      {expanded && ["judge_decision", "choice_decision", "score_decision"].includes(effective.name) && (
         <DecisionCardView ev={normalizedEv} modelInfo={toolModelInfo} />
       )}
 
       {/* 浏览器鉴权拦截 HITL 交互卡片 */}
-      {ev.toolName.startsWith("browser_") && ev.resultText && (
+      {effective.name.startsWith("browser_") && ev.resultText && (
@@ -2345,9 +2345,9 @@ export function ToolCard({ ev }: { ev: ToolEvent }) {
       {/* 展开时：若是派发协作者，展示专用任务交接文档单行卡片 */}
       {expanded &&
-        ev.toolName === "dispatch_collaborator" &&
+        effective.name === "dispatch_collaborator" &&
         resolvedSubtaskKnowledgePath && (
@@ -2378,9 +2378,9 @@ export function ToolCard({ ev }: { ev: ToolEvent }) {
       {/* 展开时：若是等待子任务/协作者汇聚且已产出报告，展示专用交付报告单行卡片（仅在完成后统一展示） */}
       {expanded &&
-        ["wait_subprocesses", "wait_subagents", "wait_collaborators"].includes(ev.toolName) &&
+        ["wait_subprocesses", "wait_subagents", "wait_collaborators"].includes(effective.name) &&
         ev.status !== "running" &&
         ev.status !== "pending_approval" &&
         resolvedSubtaskReports.length > 0 && (
@@ -2436,9 +2436,9 @@ export function ToolCard({ ev }: { ev: ToolEvent }) {
-      {expanded && showResult && !["judge_decision", "choice_decision", "score_decision"].includes(ev.toolName) && (
+      {expanded && showResult && !["judge_decision", "choice_decision", "score_decision"].includes(effective.name) && (
         <CollapsibleResult text={ev.resultText!} />
       )}
```

---

## 四、验证与验收方案

### 1. 自动化门禁验证
1. 前端门禁：
   ```bash
   cd web || npm run build
   npx tsc -b
   ```
   要求：TSC 编译与 Vite 打包通过，零语法与类型错误。
2. 后端门禁：
   ```bash
   cargo check
   cargo test --lib tools::tests
   ```
   要求：Rust 编译检查通过，工具单元测试全绿。

### 2. 业务功能验收
1. **测试 `spawn_subagent` 容错**：
   - 构造仅传 `role` 和 `task`（缺省 `title`）的调用，验证后端自动根据 `task` 提取标题并成功派生子 Agent，不再报错。
   - 构造传 `subagent_type` 代替 `role` 的调用，验证正常兼容。
2. **测试 `wait_subagents` 报告卡片**：
   - 查看最后一条会话中的 `wait_subagents` 卡片；
   - 验证展开卡片顶部已能够正确渲染绿色的【协同任务交付报告】单行条目，且包含子 Agent 角色徽标与【查看报告】按钮；
   - 点击【查看报告】按钮，验证可在右侧无缝打开 `.harness/subtasks/*_report.md` Markdown 文档。
3. **测试图标与角标**：
   - 验证 `wait_subagents`、`record_memory` 等工具卡片均显示对应的专用图标和类型徽标，不再退化为通用扩展图标。
