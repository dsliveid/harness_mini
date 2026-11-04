# UUID 源头纯化与扩展工具参数拍平实施方案

- 文档 ID: `SPEC-20261104-UUID-SIMPLE-AND-TOOL-PARAM-FLATTENING`
- 创建时间: 2026-11-04
- 责任模块: 存储层 (`src-tauri/src/store.rs`)、工具调用网关 (`src-tauri/src/tools/mod.rs`, `src-tauri/src/tools/specs.rs`)、子任务生命周期 (`src-tauri/src/tools/subagent.rs`, `src-tauri/src/agent.rs`)、SOP 目录与系统提示词 (`src-tauri/src/sop_catalog.rs`, `src-tauri/src/context/static_system.rs`)
- 关联问题:
  1. `call_extended_tool` 嵌套调用 `spawn_subagent` 时，因同名命名空间碰撞发生 `arguments.arguments` 多层嵌套，导致 `缺少 role 参数` 报错。
  2. 报告文件读取时，大模型在经历多轮对话后将原生 UUID 短横线与文件名强制转换的下划线混淆（如 `41544c24_399b_493b-bf33-eca851a411e4_report.md`），引发 `os error 2`（系统找不到指定的文件）。

---

## 一、 背景与根因剖析

### 1. 工具调用参数“错层嵌套”根因
* **同名命名空间碰撞（Name Collision Shadowing）**：
  OpenAI / Claude 原生 Function Calling 协议规范的外层包装字段本就叫 `arguments`：
  ```json
  { "function": { "name": "call_extended_tool", "arguments": "{\"tool_name\": \"...\", \"arguments\": { ... }}" } }
  ```
  而系统的网关工具 `call_extended_tool` 在 `specs.rs` 中定义的子参数属性也命名为 `"arguments": { "type": "object" }`。
* **模型 Token 路径歧义**：
  在生成复杂的提示词与子任务配置时，大模型的注意力头在“工具外层 arguments”与“内层 arguments”之间产生语法混淆，偶发性地输出了多重包裹结构：
  ```json
  {
    "tool_name": "spawn_subagent",
    "arguments": {
      "arguments": {
        "role": "前端工程师",
        "task": "..."
      },
      "title": "B-2b 前端接线"
    }
  }
  ```
* **缺乏防御性拍平机制**：
  后端 `subagent.rs` 直接执行 `args.get("role")`，无法穿透内层的多余嵌套层级，导致直接抛错。

### 2. 工件文件名混淆与读取失败根因
* **多余的字符变换人为制造心智负担**：
  系统在 `store.rs` 生成会话 ID 时使用了标准的 `uuid::Uuid::new_v4().to_string()`（带 4 个连字符 `-`）。但在保存报告文件时，后端代码在 13 处位置强行执行了 `.replace('-', "_")`。
* **大模型长上下文下的记忆漂移**：
  标准 UUID 在分词器（Tokenizer）中会被切分成 15~20 个 Token。当模型在 20~30 步后凭记忆拼接文件名时，注意力稍微衰减就会本能地回落至 UUID 原生的连字符格式，拼出了“前半截下划线、后半截短横线”的畸形串，导致精确匹配的操作系统抛出找不到文件错误。

---

## 二、 架构设计原则（拒绝堆叠，做工程减法）

本次实施严格贯彻**奥卡姆剃刀原则与做减法思想**，不增设复杂的补丁层，从数据产生的源头直接消灭产生混淆的土壤：

1. **源头消除符号歧义**：
   从会话创建的根源直接生成无连字符的 32 位 Hex UUID（`Uuid::new_v4().simple()`）。在整个会话 ID 内部彻底消灭短横线 `-`。
2. **彻底废除人工字符变换**：
   全面移除全局 13 处 `.replace('-', "_")`。会话 ID 是什么，文件名就是什么，实现 100% 的“真源对齐（Single Source of Truth）”。
3. **消除命名碰撞，建立双保险**：
   - 将网关工具的参数名由 `arguments` 升级为语义极佳且不撞车的 `tool_params`；
   - 在网关执行入口加入确定性拍平（Defensive Flattening）函数，兼顾多种别名，确保底层对任何层级的入参彻底免疫。

---

## 三、 详细实施机制与步骤

### 阶段一：UUID 生成源头纯化与消除 `.replace`（彻底解决文件名问题）

#### 1. 会话 ID 生成端纯化（`src-tauri/src/store.rs`）
在 `store.rs` 中创建会话与子进程的核心入口，统一将 `Uuid::new_v4().to_string()` 替换为 `.simple().to_string()`：
* `create_session_with_models`（第 632 行附近）
* `create_subprocess_session`（第 744 行附近）
* `fork_session`、`fork_session_at_message` 等会话克隆入口

```rust
// 修改前：
id: uuid::Uuid::new_v4().to_string(), // 36 位带短横线：41544c24-399b-493b-bf33-eca851a411e4

// 修改后：
id: uuid::Uuid::new_v4().simple().to_string(), // 32 位纯十六进制：41544c24399b493bbf33eca851a411e4
```

#### 2. 全面清理工件命名的 `.replace('-', "_")`（全库瘦身）
将全库所有拼接报告和知识交接工件路径处的 `.replace('-', "_")` 彻底清理，直接使用原始 ID：
* **`src-tauri/src/tools/subagent.rs`**：
  - `save_subtask_knowledge_artifact`：`format!("{subagent_id}_knowledge.md")`
  - `save_subtask_report_artifact`：`format!("{subagent_id}_report.md")`
  - `spawn_subagent_tool`：交接文档与报告文件名格式化
  - `wait_subagents_tool`：检测报告与读取报告路径格式化
* **`src-tauri/src/agent.rs`**：
  - 执行完成检测、检查点快照与报告检测处的 `.replace('-', "_")`
* **`src-tauri/src/sop_catalog.rs`**：
  - SOP 数据字典中关联报告格式化的 `.replace('-', "_")`
* **`src-tauri/src/context/static_system.rs`**：
  - 系统提示词中构建知识文档路径处的 `.replace('-', "_")`

#### 3. 产生的文件名效果对比
| 状态 | 会话 ID | 知识工件文件名 | 交付报告工件文件名 |
| :--- | :--- | :--- | :--- |
| **修改前** | `41544c24-399b-493b-bf33-eca851a411e4` | `41544c24_399b_493b_bf33_eca851a411e4_knowledge.md` | `41544c24_399b_493b_bf33_eca851a411e4_report.md` |
| **修改后** | `41544c24399b493bbf33eca851a411e4` | `41544c24399b493bbf33eca851a411e4_knowledge.md` | `41544c24399b493bbf33eca851a411e4_report.md` |

> **关键收益**：在修改后的 32 位 ID 内部没有任何短横线 `-`，也没有任何内部下划线 `_`。模型在读取或复述时，**彻底丧失了混用 `-` 与 `_` 的物理条件**。

---

### 阶段二：工具参数重命名与引擎确定性拍平（彻底解决参数错层）

#### 1. 工具 Spec 契约升级（`src-tauri/src/tools/specs.rs`）
将 `call_extended_tool` 的入参属性由 `arguments` 正名为 `tool_params`，并在描述中引导模型：

```rust
ToolSpec {
    name: "call_extended_tool",
    description: "调用扩展目录中的专业工具。请将目标工具所需的参数放置在 `tool_params` 对象中传入。",
    risk: Risk::Execute,
    scope: ToolScope::Universal,
    schema: json!({
        "type": "object",
        "properties": {
            "tool_name": {
                "type": "string",
                "description": "要调用的扩展工具名称"
            },
            "tool_params": {
                "type": "object",
                "description": "传递给扩展工具的参数对象（支持传入 {\"help\": true} 查看该工具规范）"
            }
        },
        "required": ["tool_name", "tool_params"]
    }),
}
```

#### 2. 系统提示词目录对齐（`src-tauri/src/context/static_system.rs`）
更新静态系统提示词中对 `call_extended_tool` 的调用示例：
```markdown
call_extended_tool({ tool_name: "spawn_subagent", tool_params: { role: "...", task: "..." } })
```

#### 3. 网关执行层确定性拍平（`src-tauri/src/tools/mod.rs`）
在 `mod.rs` 的 `call_extended_tool` 处理分支中，实现通用、健壮的“别名合并 + 递归拍平”机制：

```rust
"call_extended_tool" => {
    let target_name = args
        .get("tool_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "缺少 `tool_name`".to_string())?;

    // 1. 兼容多种参数键名（tool_params / arguments / payload / target_args）
    let mut sub_args = args.get("tool_params")
        .or_else(|| args.get("arguments"))
        .or_else(|| args.get("payload"))
        .or_else(|| args.get("target_args"))
        .cloned()
        .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));

    // 2. 确定性拍平（Defensive Flattening）：
    // 若 sub_args 内部由于大模型幻觉依然包裹了嵌套的 arguments/tool_params，递归提取并合并至顶层
    if let Some(map) = sub_args.as_object_mut() {
        for nested_key in ["tool_params", "arguments", "payload"] {
            if let Some(nested_val) = map.remove(nested_key) {
                if let Some(nested_map) = nested_val.as_object() {
                    for (k, v) in nested_map {
                        map.entry(k).or_insert(v.clone());
                    }
                }
            }
        }
    }

    // 后续正常执行 target_name，传入已拍平且无歧义的 sub_args
    execute(target_name, &sub_args, ctx, on_partial).await
}
```

---

## 四、 涉及文件变动清单

| 文件路径 | 变动性质 | 改造重点 |
| :--- | :--- | :--- |
| `src-tauri/src/store.rs` | 核心逻辑修改 | 将会话与子进程 ID 生成由 `.to_string()` 改为 `.simple().to_string()` |
| `src-tauri/src/tools/subagent.rs` | 核心逻辑修改 | 彻底删除 6 处工件路径拼接的 `.replace('-', "_")`，保持 UUID 直出 |
| `src-tauri/src/agent.rs` | 适配清理 | 删除 3 处报告检测与工件匹配的 `.replace('-', "_")` |
| `src-tauri/src/sop_catalog.rs` | 适配清理 | 删除 3 处报告规则路径格式化的 `.replace('-', "_")` |
| `src-tauri/src/context/static_system.rs` | 提示词校准 | 删除 1 处 `.replace`，提示词更新为 `tool_params` 签名 |
| `src-tauri/src/tools/specs.rs` | Schema 调整 | 将 `call_extended_tool` 的入参字段正名为 `tool_params` |
| `src-tauri/src/tools/mod.rs` | 防御加固 | 增加 `tool_params` 别名读取与内层嵌套自动拍平逻辑 |
| `src-tauri/src/tools/tests.rs` | 单测同步 | 更新单测断言以对齐无短横线的报告工件命名规则 |

---

## 五、 验收与验证方案

1. **单元测试回归**：
   运行 `cargo test --bin harness-mini` 或模块测试，验证 `subagent` 与 `tools` 单测全部 PASS。
2. **多层嵌套容错验证**：
   在单测中模拟传入多重嵌套 JSON：
   ```json
   { "tool_name": "spawn_subagent", "arguments": { "arguments": { "role": "测试", "task": "验证" }, "title": "测试任务" } }
   ```
   断言网关能够自动将其拍平为带有 `role`、`task` 和 `title` 的单层对象，零报错成功解析。
3. **文件名一致性验证**：
   启动子进程任务，验证磁盘物理落盘文件为：
   `.harness/subtasks/[32位纯字符]_knowledge.md` 与 `_report.md`。
   调用 `wait_subagents` 并由主 Agent 调用 `read_file`，确认 100% 命中，无任何 `os error 2`。
4. **前端卡片解包回归**：
   检查前端 `ToolCard` 渲染，确认任务交接书卡片与协同交付报告卡片正常展示并可点击打开。
