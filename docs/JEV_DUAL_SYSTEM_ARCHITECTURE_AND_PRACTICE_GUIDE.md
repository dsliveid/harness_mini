# Jev 双系统决策网关架构演进与实机排障知识指南 (JEV_DUAL_SYSTEM_ARCHITECTURE_AND_PRACTICE_GUIDE)

| 属性 | 说明 |
| :--- | :--- |
| **文档代号** | `JEV_DUAL_SYSTEM_ARCHITECTURE_AND_PRACTICE_GUIDE` |
| **创建时间** | 2026-09-30 |
| **适用范围** | `harness_mini`（Tauri 2 + Rust + React 桌面 AI Agent 编码工具） |
| **关联核心模块** | `src-tauri/src/jev.rs`, `src-tauri/src/agent.rs`, `src-tauri/src/tools.rs`, `src-tauri/src/memory.rs`, `src-tauri/src/commands.rs`, `src-tauri/src/secrets.rs`, `src-tauri/src/models.rs`, `src/components/SettingsModal.tsx`, `src/components/FrontmatterCard.tsx` |
| **知识密级** | 核心架构与实机排障知识沉淀 |

---

## 目录

1. [背景与双系统设计哲学](#一背景与双系统设计哲学)
2. [总体架构与三态门控契约](#二总体架构与三态门控契约)
3. [核心业务落地场景与代码实现](#三核心业务落地场景与代码实现)
   - 3.1 [场景一：记忆沉淀真实性与必要性门控](#31-场景一记忆沉淀真实性与必要性门控)
   - 3.2 [场景二与三：自适应思考深度与前置技术调研路由](#32-场景二与三自适应思考深度与前置技术调研路由)
   - 3.3 [场景四：高危命令破坏性语义双保险护栏](#33-场景四高危命令破坏性语义双保险护栏)
   - 3.4 [场景五：方案可行性结构化体检与诊断](#34-场景五方案可行性结构化体检与诊断)
4. [安全与配置持久化架构](#四安全与配置持久化架构)
5. [实机排障与连通性诊断知识库 (Troubleshooting)](#五实机排障与连通性诊断知识库-troubleshooting)
   - 5.1 [故障一：Base URL 路径重复拼接 (HTTP 404)](#51-故障一base-url-路径重复拼接-http-404)
   - 5.2 [故障二：密钥脱敏机制与前端表单交互冲突引发 401](#52-故障二密钥脱敏机制与前端表单交互冲突引发-401)
   - 5.3 [故障三：第三方中转站 (OpenCode / OneAPI) 协议兼容性](#53-故障三第三方中转站-opencode--oneapi-协议兼容性)
   - 5.4 [故障四：代理隧道与首包超时治理](#54-故障四代理隧道与首包超时治理)
6. [配置矩阵与最佳实践指引](#六配置矩阵与最佳实践指引)
7. [质量保障与回归验证基线](#七质量保障与回归验证基线)

---

## 一、背景与双系统设计哲学

在 `harness_mini` 的长期编码 Agent 使用中，传统的生成式大语言模型（LLM）具备极强的代码生成与深层推理能力，但在处理**离散、高频、敏感的“前置决策”**环节时存在结构性不足：
1. **记忆污染与脑补**：模型倾向于将未经验证的推测、瞬态调试信息无差别写入持久化记忆，导致后续会话上下文被误导；
2. **小任务过度思考 (Over-thinking)**：对单行代码修改或简单文件查阅，沿用会话级固定的深度思考参数，造成数倍的推理耗时与 Token 浪费；
3. **大需求调研不足 (Under-thinking)**：面对超出知识边界的新框架，缺乏前置识别与外部调研约束，容易凭空臆测 API；
4. **破坏性命令缺乏语义兜底**：依赖静态硬编码正则，易被参数变形、相对路径混淆或危险操作绕过。

基于 Daniel Kahneman 的双系统思维理论，本系统引入 **TypeSafe AI 的 Jev 模型作为 System One（快思考）**，与主编码 LLM（System Two 慢思考）协同分工：

| 对比维度 | System One (Jev) | System Two (主 LLM) |
| :--- | :--- | :--- |
| **核心职责** | 前置门控、分类、风险裁决、质量体检 | 方案推演、上下文理解、代码编写、重构 |
| **输出形式** | 强类型结构化原语（`Choice` / `Score` / `Noul`） | 自由文本流、Markdown、工具调用协议 |
| **延迟表现** | **70 ~ 500 ms** 极速响应 | 3 ~ 60+ 秒深度思考与流式吐字 |
| **成本计费** | 输入极低 ($0.042/MTok)，**输出 Token 免费** | 输入与推理输出双向全量计费 |
| **幻觉风险** | 基于 RLCD 校准概率，**无文本幻觉、无格式崩塌** | 存在概率性文本与格式幻觉 |

---

## 二、总体架构与三态门控契约

### 2.1 架构拓扑流图

```
                           [ 用户输入 / Agent 循环 ]
                                       │
                                       ▼
                         ┌───────────────────────────┐
                         │   Jev 决策网关 (JevClient) │
                         └─────────────┬─────────────┘
                                       │
             ┌─────────────────────────┴────────────────────────┐
             ▼                                                  ▼
     [ 开启且调用成功 ]                                   [ 未开启 / 超时 / 失败 / 低置信 ]
             │                                                  │
             ▼                                                  ▼
     Gate::Allow / Deny                                   Gate::Abstain
    (精细门控 / 自适应调节 / 拦截)                           (零副作用回退原有逻辑)
             │                                                  │
             ▼                                                  ▼
   [ 应用前置决策 ]                                      [ 原逻辑平滑流转 ]
```

### 2.2 三态决策契约 (`Gate`)

为了保证 Jev 网关的**完全插拔性与系统韧性**，后端所有决策接口均实现统一的 `Gate` 状态机：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "reason", rename_all = "lowercase")]
pub enum Gate {
    /// 明确允许 / 放行
    Allow,
    /// 明确拒绝，附带结构化拒绝理由（供 UI 提示或 Agent 自省）
    Deny(String),
    /// 弃权降级：未配置、开关关闭、超时、网络异常或置信度低于阈值
    Abstain,
}
```

> **全失败降级契约 (Abstain Guarantee)**：  
> 任何时候只要 Jev 发生网络中断、DNS 解析失败、HTTP 状态码异常、响应耗时超过阈值或模型置信度低于安全水位，网关**恒定返回 `Gate::Abstain`**。调用方零副作用回退到旧有处理流程，主业务循环不卡死、不报错、不崩溃。

---

## 三、核心业务落地场景与代码实现

### 3.1 场景一：记忆沉淀真实性与必要性门控
- **问题根因**：`record_memory_tool` 之前只要 Agent 调用即无条件落库，推测性内容与琐碎信息泛滥。
- **治理机制**：
  在 `src-tauri/src/tools.rs` 中，对传入的 `content` 与当前执行上下文前置构建原子化 Noul 命题：
  1. `verified_not_speculation`：该结论是否来自实地代码验证或实测，而非推测？
  2. `worth_long_term`：该知识点是否属于对后续项目开发具有长期复用价值的经验？
- **代码拦截点**：
  ```rust
  if let Some(client) = &jev_client {
      let (allow, reason) = client.judge_memory_quality(&content, &ctx_summary, min_conf).await;
      if !allow {
          return Ok(ToolResult::error(format!(
              "Jev 决策网关拦截记录：该内容未通过记忆质量门控（理由: {reason}）。请仅记录实地验证的确定性事实与关键规则。"
          )));
      }
  }
  ```
- **自动提炼过滤**：在 `src-tauri/src/memory.rs` 的会话收尾自动萃取流程中同样接入 Jev 质量检查，彻底封堵低质记忆。

### 3.2 场景二与三：自适应思考深度与前置技术调研路由
- **问题根因**：会话配置中的 `reasoning_effort` 过去是静态配置，简单单行修改与复杂架构推演一视同仁。
- **治理机制**：
  在 `src-tauri/src/agent.rs` 的 `run_once` 请求入口，通过 Jev 的 `Choice` 与 `Noul` 原语进行单次毫秒级请求前置评估：
  - **复杂度分类 (`TaskComplexity`)**：
    - `trivial` / `small`：动态将本轮调用的 `effective_reasoning_effort` 压制为 `"low"`，促使主模型快速收敛；
    - `deep_research`：将思考程度调高至 `"high"`；
  - **外部调研必要性 (`need_external_research`)**：
    若判定需求超出已知本地仓库认知域，且当前未查阅资料，则向当前轮次的 System Prompt 动态注入高优先级约束：
    ```
    【前置调研引导】：当前任务经评估涉及外部新规范或未知依赖，请优先调用 web_search 或 fetch_web_page 查阅权威文档，严禁凭既有旧印象臆造 API。
    ```

### 3.3 场景四：高危命令破坏性语义双保险护栏
- **问题根因**：静态 11 条危险命令正则无法识别复杂的脚本参数变形、管道混淆或间接破坏命令。
- **治理机制**：
  在 `src-tauri/src/agent.rs` 的命令审批判定中，形成 **静态正则 $\cup$ Jev 语义风险分析** 的双重防御网：
  - 正则命中：立即挂起审批；
  - 正则未命中：调用 Jev `check_command_safety`。若模型以高置信度判定存在破坏性（如删除生产配置、递归覆盖等），同样强制降级转入人工审批挂起态。

### 3.4 场景五：方案可行性结构化体检与诊断
- **交互创新**：
  在 `src/components/FrontmatterCard.tsx` 中为方案文档卡片增加 **「⚡ Jev 体检」** 交互。
- **多维量化诊断 (`PlanReviewReport`)**：
  点击后异步触发 Tauri 命令 `evaluate_plan`，Jev 并行对方案文本进行多原语评估：
  - 目标完备性概率 (`completeness_prob`)；
  - 步骤自洽度与依赖连续性 (`consistency_prob`)；
  - 验证遗漏风险 (`verification_gap_prob`)；
  - 上下文契合度打分 (`context_fit_score` 0~100)；
  - 风险级别 (`risk_level`: low / medium / high)；
  - 具体整改与加固建议列表。

---

## 四、安全与配置持久化架构

为了避免密钥泄漏及配置冲突，系统确立了严格的安全规范：

1. **AES-256-GCM 本地加密存储**：
   - 密钥使用系统本地生成的 32 字节主密钥（位于 `.dev-data/secret.key` 或 `data/secret.key`）；
   - Jev API Key 仅以密文 `v1:base64(nonce || ciphertext)` 形式持久化在 SQLite 的 `secrets` 表中；
   - 杜绝落入明文配置文件或被 Git 意外提交。
2. **前后端接口脱敏机制**：
   - 运行时 `store::get_settings` 读取设置发送给前端 UI 时，将 `jev.api_key` 显式脱敏为空字符串 `""`；
   - 前端保存配置时，若输入框为空，后端自动沿用数据库已有密文，绝不发生误抹除。
3. **独立特性开关矩阵**：
   - 支持总开关 `enabled`；
   - 细粒度控制 `features.memory_gate`、`features.thinking_depth`、`features.command_guard`、`features.plan_review_manual`。

---

## 五、实机排障与连通性诊断知识库 (Troubleshooting)

在首次集成与实机测试连通性时，曾暴露出一组典型的协议与工程交互故障。现将排查逻辑、根因与代码加固方案沉淀如下：

### 5.1 故障一：Base URL 路径重复拼接 (HTTP 404)
- **现象**：点击「测试连接」返回 `HTTP 404 Not Found`，返回网页 HTML。
- **根因分析**：
  - 用户配置 Base URL 为 `https://opencode.ai/zen/v1/systemone`（或官方 `https://api.typesafe.ai/v1`）；
  - 后端原实现采用硬拼接：`format!("{}/v1/systemone", self.base_url.trim_end_matches('/'))`；
  - 实际请求目标变为 `https://.../v1/systemone/v1/systemone`，触发 404。
- **治本修复**：
  在 `src-tauri/src/jev.rs` 引入智能端点解析函数 `resolve_systemone_endpoint`：
  ```rust
  pub fn resolve_systemone_endpoint(base_url: &str) -> String {
      let trimmed = base_url.trim().trim_end_matches('/');
      if trimmed.ends_with("/systemone") {
          trimmed.to_string()
      } else if trimmed.ends_with("/v1") {
          format!("{trimmed}/systemone")
      } else {
          format!("{trimmed}/v1/systemone")
      }
  }
  ```
  自动容错多级路径、尾部斜杠与大小写，杜绝重复叠加。

### 5.2 故障二：密钥脱敏机制与前端表单交互冲突引发 401
- **现象**：测试连接返回 `HTTP 401 Unauthorized: {"type":"error","error":{"type":"AuthError","message":"Invalid API key."}}`。
- **排障追踪**：
  从本地数据库解密 `secrets` 发现实际存储的字符串为 `"test"`（长度 4 位）。
- **根因剖析**：
  1. 后端安全脱敏将 Key 置空返回；
  2. 前端原有逻辑检查 `if (!targetCfg.apiKey?.trim()) pushToast("请先输入 Key")`；
  3. 用户重新打开设置想测连通性时，被该拦截阻断，遂在输入框随手输入 `"test"` 绕过校验；
  4. 导致 `"test"` 占位符被提交，覆盖了有效密钥，向远端请求时触发 401 认证失败。
- **治本修复**：
  - 前端移除阻断校验，占位符变更为 `••••••••（留空则沿用已保存密钥）`；
  - 后端 `test_jev` 优先读取前端传入的新 Key；若前端传入为空，自动无缝提取本地已保存的真实密钥发起测试；两者皆无时才给出明确指引。

### 5.3 故障三：第三方中转站 (OpenCode / OneAPI) 协议兼容性
- **现象**：使用某些中转站测试 Jev 模型时报 404 或 Unsupported Path。
- **核心认知**：
  - **TypeSafe AI 官方协议**：仅接受 `POST /v1/systemone`，参数为 `{ model, state, questions }` 原语，不走常规对话补全流；
  - **中转平台差异**：
    - 若中转站仅透传 OpenAI 标准格式（`POST /v1/chat/completions`），则无法直接承载 Jev 的 System One 结构化调用；
    - 接入前必须确认中转站或网关（如 Vercel AI Gateway、OpenRouter 等）是否已透传 TypeSafe 原生 `/v1/systemone` 路由。

### 5.4 故障四：代理隧道与首包超时治理
- **现象**：境内网络环境请求海外接口偶发 `operation timed out`。
- **治本修复**：
  - `JevClient` 自动感知全局代理设置（如 `http://127.0.0.1:7890`）；
  - 测试连接命令将网络超时时间从 2000ms 显式放宽至 5000ms，为首次 TLS 握手及代理转发提供充足余量；
  - 捕获 401/403/404 错误并在界面打印包含目标端点 URL 的中文建议，极大降低定位成本。

---

## 六、配置矩阵与最佳实践指引

在系统 **「设置」->「⚡ Jev 决策网关」** 中，按照如下方案进行配置：

### 推荐配置矩阵

| 参数项 | 方案 A：TypeSafe 官方接入（推荐） | 方案 B：OpenCode / 中转站接入 |
| :--- | :--- | :--- |
| **API 端点 (Base URL)** | `https://api.typesafe.ai` | `https://opencode.ai/zen` 或分配的中转基址 |
| **模型名称 (Model)** | `jev-latest` 或 `jev-1.13` | 中转站支持的 Jev 标识（如 `jev-1.13-free`） |
| **API Key** | 官方控制台申请的有效 Key (`ts-...`) | 中转平台颁发的真实调用 Token |
| **超时阈值 (Timeout)** | 800 ms（单次决策极速阈值） | 1200 ~ 1500 ms（视中转网络延迟适当放大） |
| **最低置信度 (Confidence)** | 0.60 | 0.60 |
| **网络代理** | 开启全局代理（推荐 `http://127.0.0.1:7890`） | 按需配置 |

---

## 七、质量保障与回归验证基线

为保障系统的长期稳定性，所有 Jev 模块代码均纳入全量自动化测试套件：

1. **Rust 后端自动化单测**：
   ```bash
   cargo test --manifest-path src-tauri/Cargo.toml
   ```
   - 验证项：
     - `jev::tests::test_resolve_systemone_endpoint`：涵盖 8 种 Base URL 输入格式的去重与拼接验证；
     - `jev::tests::test_gate_behavior`：三态门控契约降级验证；
     - `jev::tests::test_parse_systemone_flat_answers` / `nested_answers`：官方协议多格式反序列化兼容性；
     - `jev::tests::test_systemone_request_serialization`：Noul/Choice/Score 报文序列化；
     - `jev::tests::test_plan_review_report_serialization`：体检报告 CamelCase 结构体与前端对齐；
   - 结果基线：**133 项后端测试全绿通过 (0 failures)**。
2. **前端类型与工程构建**：
   ```bash
   npx tsc --noEmit
   npm run build
   ```
   - 结果基线：**0 类型报错，Vite 生产包构建成功**。

---

> **结语**：通过将 Jev 引入作为 System One 决策底座，`harness_mini` 在不改变原有主模型编码体验的前提下，以极低的时间和资金成本，成功解决了记忆质量失控、小任务过度消耗、大需求缺乏调研引导与高危命令防护单薄等核心痛点，形成了兼具安全性、经济性与高响应度的双系统协同编码助手。
