# AI Agent 浏览器交互与调试方案全景调查（行业实况版）

> 调研方式：实时抓取 GitHub 仓库 README、npm/PyPI registry 版本号、官方 benchmark 数据。所有版本号与 Star 数均为本次抓取实测值，非记忆推断。
> 结论定位：为「自研 CDP 常驻浏览器 daemon + 语义优先 + 视觉兜底」的桌面 Agent（本项目形态）提供选型与对标依据。

---

## 0. 三条最重要的行业变化（2026 年当下）

抓取官方 README 后确认，业内已经发生了三个结构性转向，与一年前的通行认知不同：

### 变化一：从「MCP 优先」转向「CLI + SKILLS 优先」

Playwright 官方 README（microsoft/playwright-mcp）原文明确写道：

> "Modern coding agents increasingly favor CLI–based workflows exposed as SKILLs over MCP because CLI invocations are more token-efficient: they avoid loading large tool schemas and verbose accessibility trees into the model context."
>
> "MCP remains relevant for specialized agentic loops that benefit from persistent state, rich introspection, and iterative reasoning over page structure, such as exploratory automation, self-healing tests, or long-running autonomous workflows."

并且官方直接分裂出了独立仓库 `microsoft/playwright-cli`（包名 `@playwright/cli`），核心卖点一句话：**"Token-efficient. Does not force page data into LLM."**

Chrome DevTools MCP 同样跟进：`chrome-devtools-mcp` 除了 MCP server，**同时提供 `chrome-devtools` CLI**，并附带 7 个 skills（`chrome-devtools` / `chrome-devtools-cli` / `a11y-debugging` / `cookie-debugging` / `debug-optimize-lcp` / `memory-leak-debugging` / `troubleshooting`）。

**含义**：MCP 的价值主张已从「给 LLM 一堆工具」退守为「给 LLM 一个持久的、可内省的会话」。高频、短交互的操作，CLI 是更优解。

### 变化二：从「DOM 序列化 + 元素索引」转向「模型直接写代码在持久 REPL 里操作 CDP」

browser-use 在 1.0/V4 做了架构级重写，官方 README 原文：

> "Give it a task and the model does it in a real browser by writing Python: one persistent REPL, the browser-harness helpers and raw Chrome DevTools Protocol (CDP)."
>
> "**The model, a REPL and CDP. That's the whole agent.**"

并且官方强烈警告版本漂移风险：**"pin it exactly (`browser-use==1.0.8`), never `>=`"**。

**含义**：把「选择器/索引」交给模型现场写代码，比预先定义工具集更灵活；代价是必须用精确版本锁定来对抗破坏性变更。

### 变化三：从「agent 外部驱动浏览器」转向「网站主动向 agent 暴露工具」（WebMCP）

`webmachinelearning/webmcp` 是 W3C WebML 工作组提案：网页用 JS 定义带自然语言描述和 schema 的「tools」，浏览器内置 agent / iframe 中的 agent / 扩展中的 agent 直接调用这些 tool，走页面已有的业务逻辑，而非 DOM 模拟点击。

官方给的三条支撑理由直击现有方案的痛点：

1. **UI 中介化与上下文丢失**（backend integration 绕过页面 UI）
2. **状态与认证复制**（web 开发者被迫在独立服务器重建用户状态）
3. **开发者负担**（暴露客户端能力要另写后端，无法复用客户端 JS）

并明确声明不冲突：**"If an agent ... finds that the task ... is not achievable through the WebMCP tools that the page provides, then it can fall back to general-purpose browser automation."**

**含义**：长期看，WebMCP 会成为「有合作意愿的站点」的最优通道；对存量非合作站点，DOM/视觉自动化仍是唯一解。本项目应把 WebMCP 当作**未来可插拔的快车道**预留接口。

---

## 1. 调查表 A —— 主流方案逐项实测对比
（版本号 = npm/PyPI latest；Star = shields 实时值）

| # | 方案 | 包名/版本 | License | Stars | 底层驱动 | 元素表示 | 语言 | 定位 |
|---|---|---|---|---|---|---|---|---|
| 1 | **Playwright** | `playwright` 1.65.1 / PyPI 1.65.2 | Apache-2.0 | 101k | CDP(Chromium) / Juggler(Firefox) / WebKit | CSS/XPath/role/label/text/testid | TS/Py/Java/.NET | 自动化基座，不可替代 |
| 2 | **Playwright MCP** | `@playwright/mcp` 0.0.86 | Apache-2.0 | 40.1k | Playwright | **Accessibility Tree + ref** | TS | 通用 MCP 场景 |
| 3 | **Playwright CLI+Skills** | `@playwright/cli` | Apache-2.0 | (同 org) | Playwright | snapshot ref（`e21` 等） | TS | **coding agent 首选，token 最优** |
| 4 | **Chrome DevTools MCP** | `chrome-devtools-mcp` 1.14.0 | Apache-2.0 | 59.2k | Puppeteer + CDP | DOM/AX snapshot + uid | TS | 排障/性能分析独一档 |
| 5 | **Puppeteer** | `puppeteer` 25.18.1 | Apache-2.0 | 96k | 裸 CDP | CSS/XPath/ARIA | Node | 底层库，构建自有方案 |
| 6 | **browser-use** | PyPI `browser-use==1.0.8` | MIT | 122k | **REPL + 裸 CDP**（browser-harness 引擎） | 模型现场写 Python 取元素 | Python (Pydantic AI) | 通用任务型 agent |
| 7 | **browser-harness** | 引擎（browser-use 内部） | MIT | — | CDP 直连用户 Chrome | helpers + 可自写 helper | Python | 接管真实登录态浏览器 |
| 8 | **Stagehand** | `@browserbasehq/stagehand` 4.4.0 / PyPI 4.4.0 | MIT | 26k | Playwright | **observe() 返回真实 selector** | TS/Py/Go | AI 原生应用 SDK |
| 9 | **Skyvern** | PyPI `skyvern` 1.0.58 | AGPL-ish | 24k | Playwright + 视觉 LLM | 视觉 grounding + DOM 混合 | Python | 批量表单/workflow RPA |
| 10 | **agent-browser** | `agent-browser` 0.44.0 | Apache-2.0 | 54k | **Rust CLI + 常驻 daemon + CDP** | snapshot 文本 + `@e1` ref | Rust/TS | Vercel 出品，CLI+skill 派代表 |
| 11 | **Lightpanda** | nightly build | AGPL-ish | 42k | **Zig 自研无头浏览器**（非 Chromium fork） | CDP + WebDriver BiDi | Zig | 极端轻量并发（16x 内存/9x 速度） |
| 12 | **BrowserMCP** | `@browsermcp/mcp` | Apache-2.0 | 7.4k | `chrome.debugger` relay + Playwright | DOM | TS | 接管用户真实 Chrome tab |
| 13 | **Browserbase MCP** | hosted `mcp.browserbase.com` | — | 3.4k(归档) | Browserbase 云 + Stagehand | act/observe/extract | — | **自托管仓库已归档**，官方只用 hosted |
| 14 | **Steel Browser** | Docker `ghcr.io/steel-dev/steel-browser` | Apache-2.0 | 8k | Puppeteer + CDP | 透传，任选上层 | TS | 可自托管浏览器 API |
| 15 | **Browserless** | Docker | SSPL/commercial | 14k | CDP | 透传 | TS | 托管 Chromium 集群 |
| 16 | **OmniParser** | repo | MIT | 26k | 屏幕解析模型（视觉） | 截图 → 可交互框+语义标签 | Python | 视觉 grounding 组件 |
| 17 | **UI-TARS / Agent TARS** | `@agent-tars/cli` v0.3.0(2025-11-05) | Apache-2.0 | 40k / 12k | 端到端 GUI 模型 | 截图直出动作 | TS/Py | 桌面级 GUI Agent |
| 18 | **WebMCP** | `webmcp-types` / `@w3c/webmcp-polyfill` | W3C 提案 | 5k | 网站主动暴露 tool | 页面自定义 schema | JS | 长期最优通道（需站点配合） |

---

## 2. 调查表 B —— 唯一一份可比的横向实测数据

来源：Browser Use 官方用 **BU Bench V2.1** 公开 200 题中的前 100 题（bu2-001～100）做的横向评测，测定时间 2026-10-14，同一个 agent（Codex CLI 0.144.6 + GPT-6 Luna，medium reasoning，关闭 web search）、同一个 frozen judge、同一个云端浏览器（通过 CDP 接入）。

| 工具 | 平均评分 (0-100, 95% CI) | 完全解出率 | 中位输入 token | 中位模型成本/任务 | 中位耗时 |
|---|---|---|---|---|---|
| Browser Harness（browser-use 引擎） | 60.4 (55.2-65.4) | 11% | **2.1M** | **$0.042** | 9.0 min |
| Playwright MCP 0.0.83 | 60.0 (55.1-65.0) | 11% | 4.4M | $0.066 | **8.3 min** |
| Chrome DevTools MCP 1.10.1 | **61.0** (55.8-65.9) | 11% | 6.7M | $0.097 | 8.7 min |
| agent-browser 0.39.1 | 56.3 (51.1-61.5) | 9% | 3.3M | $0.056 | 11.2 min |

**关键读法（务必注意利益相关）**：
- 分数**没有统计显著差异**——四者两两配对比较的 95% CI 全部包含 0（agent-browser −4.0，CI −10.0~+1.8；Playwright MCP −0.4，−6.6~+5.7；Chrome DevTools MCP +0.6，−5.6~+6.6）。**没有任何工具在能力上胜出。**
- 真正拉开差距的是 **token 成本**：2.1M vs 6.7M，相差 3.2 倍；成本 $0.042 vs $0.097，相差 2.3 倍。
- 结论：**选型不应比「谁更聪明」，而应比「谁的上下文经济学更优」**。这与本项目 digest 中「视觉调用泛滥 → 上下文爆炸」的根因完全同构。

---

## 3. 调查表 C —— 交互范式五派横向对比

| 维度 | ① 视觉/坐标 | ② DOM 选择器 | ③ Accessibility Tree | ④ 语义动作 API | ⑤ 裸 CDP / 写代码 |
|---|---|---|---|---|---|
| 代表 | OmniParser、UI-TARS、Computer Use | Puppeteer、Selenium | **Playwright MCP/CLI、agent-browser** | Stagehand、Skyvern | **browser-use、browser-harness、本项目** |
| 单步 token | 极高（1-2k/张） | 中 | 低~中（结构已压缩） | 低 | **最低（可精确裁剪）** |
| 泛化性 | 最强（不看 DOM） | 弱 | 中强 | 强 | 极高（模型现场写码） |
| 稳定性 | 低（坐标漂移/缩放） | 高 | 高 | 中高 | 极高（可内建命中校验） |
| Canvas/图像 UI | 唯一可行 | 失效 | 失效 | 失效 | 可见（结合坐标） |
| iframe / Shadow DOM | 免疫 | 需显式处理 | 部分自动穿透 | 视实现 | 自行实现递归穿透 |
| 反自动化 | 最强 | 弱 | 弱 | 中 | 中（可伪造启动参数） |
| 可观测性 | 差（只有图） | 中 | 强 | 中 | **最强（console/network/trace 全在手）** |
| 落地难度 | 低 | 低 | 低 | 低 | **高（需自建语义层）** |

**行业共识**：①与②③不是替代，而是**漏斗**——语义层先试，失败/空态再降级视觉，且**必须做命中校验**。

---

## 4. 调查表 D —— 工程暗坑 × 各派应对（含官方原话证据）

| 暗坑 | 表现 | Playwright 系 | 视觉系 | 扩展系 | 自研 CDP（本项目） |
|---|---|---|---|---|---|
| **点击静默成功（最致命）** | 目标被遮挡/不可见，点击落到别处却上报成功 | actionability：visible/stable/enabled/receives-events | 不适用 | DOM 校验 | **agent-browser 官方明确设计：目标 hidden / disabled / 视口外 / 被其他元素（如同意横幅）覆盖时，click 必须失败而不是报成功**；本项目已按此修（probe_page + 禁合成点击） |
| **空态盲区** | 页面未渲染完 → 判定「无此元素」 | auto-wait + expect.poll | 截图同样空 | 同左 | 本项目：wait_for_selector 分级 + 空态类名配置 |
| 元素定位漂移 | 改版后选择器全失效 | `getByRole/getByLabel` 语义定位 | 免疫 | 注入脚本 | 多级回退 + 候选打分 |
| iframe / Shadow DOM | 选择器穿不透 | frameLocator / 自动穿透 | 免疫 | 可穿透 | DOM 脚本递归穿透 |
| 虚拟列表 | 滚动后元素被回收 | 手动滚动 + 重查 | 免疫 | 同左 | snapshot diff + 光标 |
| 登录 / 2FA / SSO | 无法自动输入 | storageState / persistent context | 不可控 | **最强（真人已登录）** | credential + login_sop 固化 |
| 反自动化检测 | `navigator.webdriver`、CDP 指纹 | 需 stealth | 免疫 | 免疫 | 启动参数伪造 + 真实输入事件 |
| **上下文爆炸** | 全量 DOM 塞模型 | AX 树裁剪（`snapshot --depth=N`） | 单图固定成本 | DOM 抽样 | **snapshot 增量 + 语义层窄化提取** |
| 版本漂移破坏 API | 升级后代码崩 | 语义化版本稳定 | — | — | **browser-use 官方要求 `==1.0.8` 精确锁死**；本项目应对 CDP 域做兼容探测 |
| 原生对话框/上传 | file chooser 卡死 | `page.on('filechooser')` / `upload` | 部分可行 | 受限 | CDP `Page.setInterceptFileChooserDialog` |
| 多任务隔离 | 会话互相污染 | BrowserContext | 多实例 | 多 tab | daemon + `create_context/dispose_context` + target 绑定 |
| 僵尸进程 | 进程泄漏 | 内置清理 | 沙箱托管 | 不涉及 | `cleanup_stale_profiles` + `is_pid_alive` 探活 |
| **授权与安全** | agent 越权操作 | — | — | **BrowserMCP：chrome.debugger relay 仅在 127.0.0.1，扩展自身强制规则（禁隐身页/禁 chrome:// 页/Stop 后全禁），首次需 6 位配对码比对 + Allow** | 本项目：`approval.rs` |

---

## 5. 调查表 E —— MCP / CLI 生态清单（含归档与停更标记）

| 名称 | 形态 | 版本/状态 | 能力 | 风险 |
|---|---|---|---|---|
| `@playwright/mcp` | MCP (stdio) | 0.0.86 | AX snapshot + ref，导航/点击/填写/网络/截图 | 复杂页面 snapshot 仍爆上下文 |
| `@playwright/cli` | **CLI + Skills** | latest | 同能力，token 显著更省；`snapshot --depth=N`、`find --regex`、`--filename` 落盘 | 需 agent 会跑 shell |
| `chrome-devtools-mcp` | MCP + **CLI** + 7 skills | 1.14.0 | Performance trace、Network、Console（带 sourcemap 栈）、a11y/cookie/LCP/内存泄漏排障 | 默认上报使用统计（`--no-usage-statistics` 关闭）；仅官方支持 Chrome / Chrome for Testing |
| `@playwright/cli --skills` | skill 安装器 | — | `install --skills[=agents|--global]` | skills 与版本强绑定，升级需重装 |
| `agent-browser` | CLI + Skills + MCP + WebMCP | 0.44.0 | 全栈命令组（含 `trace`/`profiler`/`vitals`/`react`/`a11y`/`record`/`diff`/`stream`/`dashboard`） | session 一小时后自动关（可配） |
| `browser-use` | Python lib / CLI / MCP / skill | 1.0.8 | 模型写 Python 驱动 CDP；`browser-use --cli-mcp` | **必须 `==` 精确锁版本** |
| `@browserbasehq/mcp`（自托管） | MCP | **已归档** | 仅历史参考 | 官方改用 hosted `https://mcp.browserbase.com/mcp` |
| `@browsermcp/mcp` | MCP + daemon + 扩展 | latest | 接管用户已登录 Chrome tab | 仅 Chromium 系（Firefox 无 debugger API）；商店权限审核趋严 |
| `steel-browser` | Docker 服务 | public beta | 会话/代理/扩展/stealth/请求日志/live view/replay | 3000/9223 端口**无认证**，不可暴露公网 |
| `browserless` | Docker | 商业+SSPL | 托管 Chromium 集群 | 许可限制 |
| `@modelcontextprotocol/server-puppeteer` | MCP | 2025.5.12（旧） | 基础操作 | 社区早期参考实现，已实质被 Playwright 版替代 |
| `webmcp-types` / `@w3c/webmcp-polyfill` | W3C 提案 + polyfill | — | 网站主动暴露 tool | 需站点配合，存量站点无效 |

---

## 6. 选型决策矩阵

| 场景 | 首选 | 备选 | 依据 |
|---|---|---|---|
| 快速 PoC / 内部工具 | `@playwright/cli` + skills | `@playwright/mcp` | token 最省，官方主推 |
| 需要持久会话 + 深度内省 | `@playwright/mcp` | `chrome-devtools-mcp` | 官方原文：MCP 适用于 long-running autonomous workflows |
| 前端性能/报错排障 | `chrome-devtools-mcp` | 自研 CDP + Performance 域 | Performance/Network/Console 官方面板语义 |
| 批量表单 / RPA workflow | Skyvern | Stagehand + 自建 workflow | workflow 可复用、视觉抗改版 |
| AI 原生应用（要可控成本） | Stagehand | Playwright + 自研语义层 | `observe()` 返回真实 selector，**凭据不进模型** |
| 桌面 Agent 产品（**本项目**） | 自研 CDP daemon + 语义优先 + 视觉兜底 | 嵌 Playwright / agent-browser daemon | 控制力与可观测性最优；参考 agent-browser 的 daemon+ref+skill 架构 |
| 必须复用用户真实登录态 | BrowserMCP（扩展 relay） | browser-harness（CDP 接已开 Chrome） | 2FA/SSO 唯一解 |
| 极致并发/成本敏感爬取 | Lightpanda | Steel / Browserless | 16x 内存、9x 速度，CDP 兼容 |
| 泛化到陌生站点 | browser-use | Skyvern（视觉混合） | 写不动语义层的地方只能靠视觉/写码 |
| 长期押注 | 预留 WebMCP 通道 | — | 合作站点可直接调 tool，跳过 UI 模拟 |

---

## 7. 对本项目的三条最高杠杆改进（按 ROI 排序）

> 对标事实：本项目 `browser/session.rs` 已有 `real_click_by_selector`（真实鼠标）、`user_click`（坐标）、`start_screencast`、`snapshot`、`evaluator`；`daemon.rs` 已有 context 隔离与僵尸清理；`credential`+`login_sop` 固化登录。架构方向与业界收敛形态一致。

1. **把 agent-browser 的「click 必须失败」语义完整落地**
   官方原话：hidden / disabled / 视口外 / 被其他元素覆盖 → 失败而非成功。本项目已修 probe_page，建议进一步把 `visible + stable + enabled + receives-events` 四态断言做成**统一前置断言函数**，任何交互入口强制先过。

2. **上下文经济学对齐（token 是第一成本）**
   横向实测显示 token 差 3.2 倍而能力无显著差异。落地动作：
   - snapshot 支持 `--depth=N` 式裁剪与正则 `find`（Playwright CLI 已证明有效）
   - 大结果落盘（`--filename`）而非塞进对话
   - 视觉调用前先做**空态判定**，消灭「假空态 → 盲调视觉」链路

3. **预留双通道演进位**
   - 短期：AX 树兜底通道（DOM 提取失败时降级 a11y 树，成本远低于视觉）
   - 长期：WebMCP 适配层（`webmcp-types` schema → 本项目的 tool 注册表）

---

## 8. 风险与合规清单

- **Browserbase 自托管 MCP 仓库已归档**——不要按旧文档接入 `@browserbasehq/mcp`，官方仅维护 hosted endpoint。
- **Steel 的 3000/9223 端口无认证**，官方 SECURITY.md 明示不可暴露公网。
- **chrome-devtools-mcp 默认上报使用统计**（tool 成功率/延迟/环境），需 `--no-usage-statistics` 或 `CHROME_DEVTOOLS_MCP_NO_USAGE_STATISTICS` 关闭；Performance 工具还会把 trace URL 发给 Google CrUX（`--no-performance-crux` 关闭）。
- **browser-use 不能用 `>=` 依赖**，官方要求精确 pin。
- **Lightpanda 无原生 Windows 二进制**，Windows 需走 WSL2 或 Docker；Linux 版需 glibc 2.35(x64)/2.38(arm64)。
- **扩展派权限风险**：Chrome Web Store 对 `<all_urls>` + `debugger` 审核趋严，商业产品慎选。
- **视觉派不可审计**：坐标点错无日志可溯，生产必须落每步截图 + 决策依据。
- **CDP 版本漂移**：`Page.screencast`、`Target` 等域在大版本间会变，自研客户端须锁版本 + 兼容探测。

---

## 9. 数据来源

| 类型 | 来源 |
|---|---|
| README 原文 | `raw.githubusercontent.com/{microsoft/playwright-mcp, microsoft/playwright-cli, ChromeDevTools/chrome-devtools-mcp, browser-use/browser-use, browser-use/browser-harness, browserbase/stagehand, Skyvern-AI/skyvern, vercel-labs/agent-browser, lightpanda-io/browser, BrowserMCP/mcp, steel-dev/steel-browser, browserbase/mcp-server-browserbase, bytedance/UI-TARS-desktop, webmachinelearning/webmcp}/main/README.md` |
| 版本号 | `registry.npmjs.org/{pkg}/latest`、`pypi.org/pypi/{pkg}/json` |
| Star / 活跃度 | `img.shields.io/github/stars/{repo}.json`、`img.shields.io/github/last-commit/{repo}.json` |
| 横向实测 | Browser Use 官方 `browser-harness` README 内嵌 BU Bench V2.1 结果（2026-10-14），原始 JSON：`browser-use.com/benchmarks/coding-agent-browser-tools.json` |
| 视觉基准 | Skyvern 2.0 WebVoyager 85.8%（`skyvern.com/blog/skyvern-2-0-state-of-the-art-web-navigation-with-85-8-on-webvoyager-eval/`） |
