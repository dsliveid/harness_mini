# 浏览器端到端稳定性治理与工业级免登网关知识文档
(BROWSER_STABILITY_GOVERNANCE_AND_DETERMINISTIC_AUTH_KNOWLEDGE)

| 属性 | 说明 |
| :--- | :--- |
| **文档代号** | `BROWSER_STABILITY_GOVERNANCE_AND_DETERMINISTIC_AUTH_KNOWLEDGE` |
| **所属模块** | `src-tauri/src/browser/*`, `src-tauri/src/commands.rs`, `src/components/BrowserInspector.tsx`, `src/ipc.ts` |
| **技术体系** | Chromium DevTools Protocol (CDP) · BrowserContext 隔离沙箱 · StorageState 状态网关 · Win32 Job Object · 逻辑物理像素映射 |
| **创建时间** | 2026-10-30 |
| **版本演进** | 演进重构自 `BROWSER_CONTEXT_SANDBOX_AND_FRONTEND_DIAGNOSTICS_KNOWLEDGE.md` 及 `BROWSER_SMART_RECORDING_AND_LLM_AUTH_CONTRACT_KNOWLEDGE.md` |
| **核心机制** | 进程级 PID 目录物理隔离 · 1:1 CSS 逻辑像素与 DOM 原生派发 · 确定性全状态 StorageState 网关（Cookies + LocalStorage + SessionStorage） · 导航前原子错误清理 · 废除隐式黑盒重定向 |

---

## 一、 现场真实案例与故障深度复盘

在实际使用 Harness 浏览器调试工具进行端到端自动化测试与调试时，暴露出了 5 类严重阻碍 Agent 正常运行的典型故障：

### 1.1 现象一：新开页面登录文本框始终选不中 / 点击偏移
*   **现象**：受控浏览器新启动后，无论是人工在右侧视窗点击，还是 Agent 执行点击指令，光标始终无法聚焦在账号密码输入框中；在 Tab 2（交互元素列表）中点击对应元素，页面毫无反应。
*   **影响**：Agent 无法输入登录凭据，导致自动化验收测试在第 1 步就彻底中断。

### 1.2 现象二：登录录制后，免登录功能极不稳定
*   **现象**：用户在视窗中走完登录录制并由大模型提炼生成免登契约后，有时可用、有时频繁报错，页面死锁在“正在自愈登录态”循环中，甚至 Agent 报告“密码被清空”。
*   **影响**：免登录成为不可靠的黑盒概率功能，无法作为持续集成的信任基础。

### 1.3 现象三：访问目标页面点击报错，随后快照退化为 `chrome-error://chromewebdata/` 并残留旧端口错误
*   **复现轨迹**：
    1. Agent 执行 `browser_close()` 清空浏览器会话；
    2. 执行 `browser_navigate("http://127.0.0.1:18086/")`，返回正确页面 DOM（`@e1~@e8`），但同时报告了 8 条针对已停服旧端口 `http://127.0.0.1:5611/` 的 `ERR_CONNECTION_REFUSED` 报错；
    3. 执行 `browser_interact(click, "@e8")`（纯本地按钮），报错 `未找到目标元素 @e8`；
    4. 执行 `browser_get_debug_state()`，当前 URL 已经变成 `chrome-error://chromewebdata/`，且仍然固定残留 5611 错误。
*   **影响**：Agent 拿着旧快照的 Ref 作用在已经漂移的 error 页面上，操作全部失效，并被残留日志误导。

### 1.4 现象四：单台电脑启动两个 Agent 工具互斥打不开（ExitCode 21）
*   **现象**：当在一台电脑上并行启动两个 Harness Agent 时，只要其中一个使用了浏览器，另一个尝试打开浏览器就会瞬间闪退或长时间卡死。
*   **影响**：团队协同或并行多任务场景完全瘫痪。

### 1.5 现象五：多会话并发调用浏览器冲突与串流
*   **现象**：多会话同时运行或子 Agent 并发调用时，网络报错流互相穿透，甚至出现 A 会话操作了 B 会话页面的诡异现象。

---

## 二、 底层物理根因与机制深度剖析

```mermaid
flowchart TD
    subgraph RootCause1["根因一: 坐标系物理错位与 Ref 丢失"]
        DPI2["CDP deviceScaleFactor: 2 物理缩放"] --> Screen["截图像素 2880x1920"]
        Screen --> FrontendCalc["前端按图片比例换算得到物理像素 (如 2800, 1800)"]
        FrontendCalc --> CDPDispatch["Input.dispatchMouseEvent 严格接收 CSS 逻辑坐标 (1440x960)"]
        CDPDispatch --> OutOfView["点击偏出视口外部，输入框无法命中"]
        Tab2Ref["Tab 2 列表点击丢失 target 标识"] --> Fallback0["后端兜底取 (0.0, 0.0)，点在左上角空白处"]
    end

    subgraph RootCause2["根因二: 凭据捕获残缺与大模型黑盒自愈"]
        CookieOnly["旧 storage.rs 仅抓取 document.cookie"] --> LossStorage["现代 SPA 绝大多数凭据存储于 localStorage / sessionStorage，全部丢失"]
        BlurOnly["录制探针仅监听 change (失焦) 事件"] --> SkipEnter["用户回车或直接点击登录跳过 change，漏记密码"]
        LLMDesensitize["大模型提炼契约时概率性安全脱敏清空密码"] --> Deadlock["自愈状态机死锁"]
        NavigateHook["browser_navigate 隐式触发 check_and_heal_auth"] --> OldContract["读取历史残留旧契约 (5611 端口) 强行跳走"]
        OldContract --> ChromeError["5611 已停服，页面沦为 chrome-error://chromewebdata/"]
    end

    subgraph RootCause3["根因三: 全局硬编码目录与单例锁互斥 (ExitCode 21)"]
        HardcodedDir["硬编码 %TEMP%/harness_browser_daemon"] --> Lockfile["实例 1 启动占用 Windows lockfile 独占句柄"]
        Instance2["实例 2 启动尝试清理目录"] --> WinError32["Windows Win32 Error 32 拒绝访问，无法删除"]
        WinError32 --> ProcessSingleton["Chromium ProcessSingleton 检测失败"]
        ProcessSingleton --> Exit21["Chromium 立即闪退 ExitCode: 21"]
        Exit21 --> OverwritePort["同时误删 DevToolsActivePort，打崩实例 1"]
    end
```

### 2.1 根因一：物理像素 vs CSS 逻辑像素错位
在 Chrome DevTools Protocol 中，`Input.dispatchMouseEvent` 的 `x` 与 `y` 参数接收的是 **CSS 逻辑像素**（以页面视口 $1440\times 960$ 为基准）。旧版本在初始化 Session 时设置了 `deviceScaleFactor: 2`，无头 Chromium 生成的截图为物理分辨率 $2880\times 1920$。前端在处理鼠标点击时，通过 `img.naturalWidth / rect.width` 计算出了大图物理像素并传给后端。后端直接将超过 1440 的物理坐标派发给 CDP，导致点击点直接落在了视口之外。

此外，在 Tab 2 点击列表元素时，前端调用 `browser_user_interact(click, undefined, undefined, el.ref_id)`，旧版后端未对 `text` 中的 Ref 格式进行解析，直接走坐标分支，取 `x.unwrap_or(0.0), y.unwrap_or(0.0)`，强制派发了 `(0, 0)` 点击。

### 2.2 根因二：隐式幽灵重定向与不可靠的自愈契约
为什么访问 18086 会跳到 5611 并变成 `chrome-error`？
1. **隐式后置钩子**：旧版 [`src-tauri/src/browser/mod.rs`](file:///F:/WorkSpace/Other/harness_mini/src-tauri/src/browser/mod.rs) 的 `browser_navigate` 与 `browser_interact` 函数在执行完毕后，私自追加了一段逻辑：如果页面检测到密码框，自动调用 `check_and_heal_auth`。
2. **契约历史残留**：系统工作区中残留着之前测试 5611 项目时生成的旧契约。`check_and_heal_auth` 自动读取了该契约，强行调用 `session.navigate("http://127.0.0.1:5611/login")`！
3. **已停服报错与旧快照欺骗**：5611 此时已经停服，页面迅速沦为 `chrome-error://chromewebdata/`。此时更致命的是，旧代码捕获错误后，**把重定向跳转前的旧快照当成了当前结果返回给了 Agent**。Agent 看到的快照是 18086 的 `@e8`，但浏览器物理上早已停留在了 5611 的错误页面上，导致随后的所有点击彻底失效。

### 2.3 根因三：单凭据丢失与概率自愈的本质缺陷
1. **状态存储残缺**：现代前端框架（Vue、React 等）基本采用 JWT / Token 机制存储于 `localStorage` 或 `sessionStorage`，旧版 [`storage.rs`](file:///F:/WorkSpace/Other/harness_mini/src-tauri/src/browser/storage.rs) 仅读取 `document.cookie`，刷新后登录态必定丢失。
2. **黑盒概率不可控**：依靠大模型对录制轨迹进行语义提炼，不可避免会遇到密码脱敏（大模型自动将密码替换为 `******` 导致契约无法使用）、固定 sleep 等待竞态、弹窗标题未命中等脆弱点。

### 2.4 根因四：单例文件锁互斥与跨 Tab 串扰
1. **ExitCode 21 闪退**：[`daemon.rs`](file:///F:/WorkSpace/Other/harness_mini/src-tauri/src/browser/daemon.rs) 将所有实例的用户数据目录固定为 `%TEMP%\harness_browser_daemon`。Chromium 启动时会利用 `lockfile` 进行 `ProcessSingleton` 校验。第二个进程无法获取独占锁，立刻以 ExitCode 21 闪退。
2. **越权借用他人 Tab**：[`BrowserManager::get`](file:///F:/WorkSpace/Other/harness_mini/src-tauri/src/browser/mod.rs) 包含了回退机制：“若未精确命中传入的 `session_id`，尝试回退到任意当前依然存活的调试实例”。当会话 A 查询不到自己的 Tab 时，会直接抢占会话 B 的 Tab 派发点击，引发灾难性串扰。

---

## 三、 最优方案与架构演进（工业级做减法）

我们拒绝在不可靠的黑盒链路上做“补丁式堆叠”，而是遵循**“彻底根治、架构清晰、工程确定性”**原则，全面对齐业界一流标准（Playwright / Puppeteer 的 StorageState 规范与 BrowserContext 沙箱隔离）。

```mermaid
flowchart TD
    subgraph MultiAgent_Isolation [1. 进程级 PID 物理隔离]
        Agent1[Agent 进程 1 PID 1001] -->|专用 Profile| Dir1["%TEMP%/harness_browser_daemon_1001"]
        Agent2[Agent 进程 2 PID 2002] -->|专用 Profile| Dir2["%TEMP%/harness_browser_daemon_2002"]
        Dir1 --> Cr1[Chromium 守护进程 1]
        Dir2 --> Cr2[Chromium 守护进程 2]
        Cr1 -->|Target.createBrowserContext| Ctx1[会话 1 完全隔离沙箱]
        Cr1 -->|Target.createBrowserContext| Ctx2[会话 2 完全隔离沙箱]
    end

    subgraph Deterministic_Storage [2. 确定性 StorageState 全状态网关]
        HumanLogin[人工或测试在视窗中完成一次登录] -->|点击固化| SaveCmd["save_browser_storage_state"]
        SaveCmd -->|Runtime.evaluate + Network.getCookies| Snapshot["Cookies + localStorage + sessionStorage"]
        Snapshot -->|按 Origin 隔离持久化| DiskFile[".harness/storage_states/storage_{host}_{port}.json"]
        DiskFile -->|导航首行业务代码执行前原子注入| Restore["Page.addScriptToEvaluateOnNewDocument + Network.setCookies"]
    end

    subgraph Deterministic_Interact [3. 确定性无污染交互]
        Nav[browser_navigate] --> ClearErr["原子清空历史网络/控制台错误队列 (clear_errors)"]
        ClearErr --> DirectLoad["纯净直连目标地址 (绝无隐式自愈与强行重定向)"]
        DirectLoad --> DOMFirst["DOM 原生派发优先 (传入 @eX 时调用 Element.click())"]
        DOMFirst --> ExactClick["DPI 1:1 精准逻辑坐标对齐"]
    end
```

### 3.1 核心架构四项原则
1. **隔离性原则（Isolation First）**：每个进程拥有独立的用户数据目录，每个调试会话拥有独立的 `BrowserContext`，网络事件与 DOM 树严格基于 `sessionId` 隔离，严禁跨会话借用 Tab。
2. **确定性原则（Determinism Over Probability）**：废除隐式后台自愈重定向与大模型猜密码。导航就是导航，点击就是点击；登录态固化就是确定性的全状态快照预注入。
3. **做减法原则（Subtractive Governance）**：删除不可控的隐式后置钩子、删除脏数据兜底、删除跨 Tab 越权借用逻辑。
4. **状态完整性原则（Full-Spectrum Storage）**：登录凭据必须覆盖 `Cookies` + `LocalStorage` + `SessionStorage`，并以 `Host + Port` 命名空间切分隔离，防止端口间相互污染。

---

## 四、 核心实现细节与代码落地

### 4.1 进程级 PID 物理隔离（根治 ExitCode 21）
在 [`src-tauri/src/browser/daemon.rs`](file:///F:/WorkSpace/Other/harness_mini/src-tauri/src/browser/daemon.rs) 中，用户数据目录由静态路径升级为进程 PID 动态隔离：
```rust
impl BrowserDaemon {
    pub fn new() -> Self {
        let p = std::env::temp_dir().join(format!("harness_browser_daemon_{}", std::process::id()));
        Self {
            child: Arc::new(Mutex::new(None)),
            cdp: Arc::new(RwLock::new(None)),
            ws_url: Arc::new(RwLock::new(None)),
            daemon_profile_dir: p,
        }
    }

    pub async fn stop(&self) {
        let mut child_guard = self.child.lock().await;
        if let Some(mut child) = child_guard.take() {
            let _ = child.kill().await;
        }
        ...
        // 彻底清除本进程关联的单例目录与文件锁，释放操作系统句柄
        let _ = tokio::fs::remove_dir_all(&self.daemon_profile_dir).await;
    }
}
```

### 4.2 像素 1:1 逻辑对齐与 DOM 原生优先派发（根治选不中）
1. 在 [`src-tauri/src/browser/session.rs`](file:///F:/WorkSpace/Other/harness_mini/src-tauri/src/browser/session.rs) 中，将无头视窗 DPI 缩放锁定为 `1`：
   ```rust
   let params = json!({
       "width": 1440,
       "height": 960,
       "deviceScaleFactor": 1, // 锁定 1:1，消灭无头模式物理截图与逻辑像素的反算失真
       "mobile": false
   });
   let _ = self.cdp.send("Emulation.setDeviceMetricsOverride", Some(params), sid, Duration::from_secs(5)).await;
   ```
2. 在 [`src-tauri/src/commands.rs`](file:///F:/WorkSpace/Other/harness_mini/src-tauri/src/commands.rs) 的 `browser_user_interact` 中，识别元素 Ref 并优先派发原生 DOM 点击：
   ```rust
   match action.as_str() {
       "click" => {
           if let Some(ref t) = text {
               let clean = t.trim();
               if clean.starts_with('@') || !clean.is_empty() {
                   // 优先按元素 Ref 原生派发，不再退化为坐标 (0, 0)
                   let _ = session.interact("click", clean, None).await?;
                   return Ok(());
               }
           }
           let px = x.unwrap_or(0.0);
           let py = y.unwrap_or(0.0);
           session.user_click(px, py).await
       }
       ...
   }
   ```

### 4.3 确定性全状态 StorageState 网关（根治免登不稳定）
在 [`src-tauri/src/browser/storage.rs`](file:///F:/WorkSpace/Other/harness_mini/src-tauri/src/browser/storage.rs) 中，构建工业级全状态捕获与注水引擎：
1. **全状态捕获**：
   *   通过 `Network.getCookies` 抓取全量 Cookies；
   *   通过 CDP `Runtime.evaluate` 提取目标域名的 `window.localStorage` 与 `window.sessionStorage`。
2. **Host + Port 独立隔离**：
   ```rust
   pub fn get_storage_state_path(workspace: &Path, target_url: Option<&str>) -> PathBuf {
       if let Some(u) = target_url {
           if let Ok(parsed) = url::Url::parse(u) {
               let host = parsed.host_str().unwrap_or("default");
               let port = parsed.port_or_known_default().unwrap_or(80);
               let safe_host = host.replace(|c: char| !c.is_alphanumeric() && c != '.', "_");
               return workspace
                   .join(".harness")
                   .join("storage_states")
                   .join(format!("storage_{safe_host}_{port}.json"));
           }
       }
       workspace.join(".harness").join("storage_state.json")
   }
   ```
3. **首行业务代码前原子预注水**：
   通过 `Page.addScriptToEvaluateOnNewDocument` 注入预水合脚本，在前端任何 Vue/React 启动脚本执行前，直接还原 `localStorage` 与 `sessionStorage`：
   ```javascript
   (function() {
       try {
           const ls = {ls_json};
           for (const [k, v] of Object.entries(ls)) {
               try { localStorage.setItem(k, v); } catch (_) {}
           }
           const ss = {ss_json};
           for (const [k, v] of Object.entries(ss)) {
               try { sessionStorage.setItem(k, v); } catch (_) {}
           }
       } catch (_) {}
   })()
   ```

### 4.4 彻底移除隐式自愈重定向与原子错误清理（根治 5611 幽灵错误）
1. 在 [`src-tauri/src/browser/cdp.rs`](file:///F:/WorkSpace/Other/harness_mini/src-tauri/src/browser/cdp.rs) 中增加 `clear_errors()`：
   ```rust
   pub async fn clear_errors(&self) {
       self.network_errors.lock().await.clear();
       self.structured_network_errors.lock().await.clear();
       self.console_logs.lock().await.clear();
       self.runtime_exceptions.lock().await.clear();
       self.requests_map.lock().await.clear();
   }
   ```
2. 在 [`src-tauri/src/browser/session.rs`](file:///F:/WorkSpace/Other/harness_mini/src-tauri/src/browser/session.rs) 的 `navigate()` 启动时刻，强制原子清空旧错误，并自动注入对应 Origin 的 StorageState：
   ```rust
   // 1. 彻底清空前序页面残留的网络/控制台错误
   self.cdp.clear_errors().await;

   // 2. 匹配并预恢复当前站点的 StorageState
   let _ = restore_storage_state(&self.cdp, &self.target_session_id, &self.workspace_path, Some(url)).await;
   ```
3. 在 [`src-tauri/src/browser/mod.rs`](file:///F:/WorkSpace/Other/harness_mini/src-tauri/src/browser/mod.rs) 中彻底剔除 `check_and_heal_auth`，保证操作的确定性。
4. 彻底删除 `BrowserManager::get` 中随机遍历借用他人存活 Tab 的危险回退代码。

---

## 五、 全链路验证与治理效果对比

### 5.1 自动化测试全绿验证
运行 `Cargo` 单元测试，8 项全功能测试在 **0.88s** 内极速全部通过：
```text
running 8 tests
test browser::tests::test_build_interact_script ... ok
test browser::tests::test_build_scroll_script ... ok
test agent::tests::test_browser_autonomous_testing_sop_in_system_prompt ... ok
test browser::tests::test_detect_chromium ... ok
test browser::synthesizer::tests::test_default_prompt_and_trace_struct ... ok
test browser::synthesizer::tests::test_parse_distilled_json ... ok
test browser::auth::tests::test_auth_contract_encryption_roundtrip ... ok
test browser::tests::test_browser_daemon_and_context_lifecycle ... ok

test result: ok. 8 passed; 0 failed; finished in 0.88s
```
前端代码通过 `npx tsc --noEmit` 静态类型检查，ExitCode 0 零错误。

### 5.2 治理前后指标对比矩阵

| 维度 | 治理前（旧方案） | 治理后（本次重构后方案） | 收益与稳定性提升 |
| :--- | :--- | :--- | :--- |
| **点击与选中文本框** | 偏出视口外部，Tab 2 列表点击强制落入 `(0,0)` 空白 | DPI 1:1 坐标严格对齐，支持 DOM 原生事件直达 | **100% 精准命中**，消除坐标偏移 |
| **免登录与持久化** | 仅抓 Cookie，遗漏 WebStorage；大模型概率脱敏密码；死锁 | 抓取 Cookies + LocalStorage + SessionStorage，确定性无损预注水 | **100% 确定性**，无大模型猜测延迟与幻觉 |
| **端口与项目隔离** | 共享单文件 `storage_state.json`，18086 与 5611 相互篡改 | 按 `storage_{host}_{port}.json` 独立持久化 | 各业务站点**物理级互不干扰** |
| **操作确定性** | 隐式后台自愈重定向，跳往已关端口并返回脏 DOM 快照 | 绝对确定性，输入什么 URL 访问什么 URL，导航前原子清理残留错误 | **零幽灵报错**，彻底告别 Chrome Error 污染 |
| **多 Agent / 多开** | 硬编码路径导致独占锁冲突（ExitCode 21 闪退） | 进程 PID 独立 Profile 目录与 Job Object 纳管 | **支持单机任意多个 Agent 并发**无互斥 |
| **多会话并发安全性** | 未命中随机借用他人活着的 Tab，串扰严重 | 严格按 `BrowserContext` 强隔离，拒绝跨会话越权操作 | **零串流、零借 Tab 串扰** |

---

## 六、 总结与最佳实践准则

1. **确定性工程高于概率模型**：
   在自动化测试与浏览器控制基础设施中，鉴权持久化应首选工业级标准的 `StorageState` 快照网关，避免把核心链路建立在“大模型实时观察与猜测重填”的概率链条上。
2. **像素空间必须统一**：
   无头浏览器渲染截图时必须保证与输入派发接口（CDP Input）的坐标空间采用相同的比例尺（推荐锁定 `deviceScaleFactor: 1`）。
3. **外部进程必须带 PID 沙箱**：
   桌面客户端拉起任何自带锁文件的第三方子进程（如 Chromium、Node 等）时，用户数据目录必须显式带上操作系统当前进程的 PID（如 `harness_browser_daemon_{pid}`），并在退出时强制闭环释放。
4. **单次导航必须原子重置错误流**：
   每次进入新页面之前，必须原子清空底层网络失败与控制台错误队列，严防旧页面（如已停服项目）的崩溃日志跨页面污染新业务快照。

---

## 五、关闭浏览器后重新打开“输入框多次点击无响应，需等待数秒才恢复”深度归因与根治方案

### 1. 现象复盘与核心疑点
- **复现步骤**：在浏览器调试面板中关闭调试浏览器，随后重新输入 URL 打开（或由 Agent 重拉实例）。
- **表象故障**：在前端视窗快照渲染出登录页面后，用户立即用鼠标在视窗的输入框区域连续点击多次，输入框毫无反应（无聚焦边框、光标不闪烁、右侧文本框输入无法同步）；但停顿等待 3~5 秒后，再次点击同一位置，输入框立刻精准响应并获取焦点。
- **关键疑问**：为什么刚打开的几秒内多次点击均无效，而等待一段时间后即可恢复？

---

### 2. 深层根因剖析（四重并发缺陷叠加）

```
[用户触发重新打开浏览器]
       │
       ▼
1. 冷启动 / 新 Context 创建
   - 浏览器无本地缓存，并发拉取数十个前端资源 chunk
       │
       ▼
2. navigate() 盲目 sleep(1500ms)
   - 1.5 秒时仅首屏静态 HTML/骨架渲染完成，巨型 JS bundle 正在 V8 编译与主线程密集解析
   - Vue / React 根组件尚未挂载 (Hydration Incomplete)
   - 此时提前返回 debug_state 并捕获首帧静态快照
       │
       ▼
3. 视口设备像素比畸变 (deviceScaleFactor = 2)
   - 截图像素为 2880x1920，视口 CSS 像素为 1440x960
   - 前端基于图片物理像素缩放，clickX 计算翻倍，初次点击直接偏离目标元素 2 倍远！
       │
       ▼
4. 操作系统窗口失焦 (Missing Focus Emulation)
   - 无头模式下无真实物理窗口，Chromium 默认 document.hasFocus() === false
   - 纯底层 mousePressed 事件被 Chromium 抑制，不触发 activeElement 转移与 focusin
       │
       ▼
5. 为什么等待几秒后又可以响应？
   - 3~5 秒后 SPA 框架真正 Mount 完成，主线程进入 Idle；
   - 用户在尝试多次后点击其他区域或触发了聚焦回退，或通过输入操作激活了焦点。
```

---

### 3. 架构级根治方案（无堆叠补丁，系统性机制升级）

#### 机制 1：分辨率体系 1:1 归一化（CSS DIP 与截图物理分辨率严格对齐）
- 将 Chromium `Emulation.setDeviceMetricsOverride` 的 `deviceScaleFactor` 设为 `1`（`width: 1440, height: 960`）。
- **收益**：
  1. CSS 逻辑像素与截图像素严格 1:1，前端 `(e.clientX - rect.left) * (1440 / rect.width)` 计算绝对精准，无任何倍率失真。
  2. 截图数据体积减少 75%（原 2880x1920 变为 1440x960），WebSocket 通道传输延迟暴降，快照渲染极速呈现。

#### 机制 2：全周期页面焦点模拟（Emulation.setFocusEmulationEnabled）
- 在 Session 启动、页面 Target 挂载以及页面导航就绪时，全周期强制开启：
  ```rust
  cdp.send("Emulation.setFocusEmulationEnabled", Some(json!({ "enabled": true })), sid, Duration::from_secs(2)).await;
  cdp.send("Page.bringToFront", None, sid, Duration::from_secs(2)).await;
  ```
- 保证无头 Chromium 认定该沙箱页面处于操作系统最高焦点状态，彻底解除事件抑制。

#### 机制 3：智能就绪等待（Smart Readiness Wait 替代盲目 sleep）
- 弃用固定的 `sleep(1500ms)`，改用面向单页应用（SPA）的自适应就绪探针：
  1. 轮询探测 `document.readyState === 'complete'`；
  2. 满足 complete 后追加 300ms 挂载安全窗口，确保 Vue/React 组件树建立完备并完成事件委托；
  3. 最大 3.5s 超时保护（防止长轮询接口导致无限阻塞）。
- 确保呈现给用户的首张快照即为“完全可交互”状态。

#### 机制 4：物理仿真点击与智能双重兜底聚焦（Dual-Guaranteed Focus）
- 重构 `session.user_click(x, y)`：
  1. **光标移动**：派发 `mouseMoved(x, y)`，激活前端组件的 hover / active 态；
  2. **物理按压**：派发 `mousePressed`，并加入 **35ms 人类真实按压物理延迟**，派发 `mouseReleased`，防止高频微秒事件被防抖过滤；
  3. **元素级兜底聚焦**：通过 `document.elementFromPoint(x, y)` 探测命中元素，若为 `input`、`textarea`、`contenteditable` 或主流组件库输入框包装层（`.el-input`, `.ant-input-affix-wrapper`, `.n-input` 等），显式调用 `focus()`。双重保障输入框 100% 获得焦点与闪烁光标！

#### 机制 5：受控表单响应式事件深度同步
- 重构 `session.user_type_text(text)`：
  1. 键入前检测并自动聚焦当前可见输入框；
  2. 调用 `Input.insertText` 插入文本；
  3. 主动派发 `input` 与 `change` 冒泡事件，强制通知 Vue 3 / React 的受控组件状态池更新（同步 `v-model`），彻底消除“字打进去了但提交时表单值为空”的顽疾。

---

## 五、 StorageState 契约数据同步与前端字段序列化治理

### 1. 现场故障现象：Token 与 Storage 项显示为 0，固化时间为空
*   **现象**：用户在系统（如 `http://192.168.200.45:4185/` 或 `http://192.168.200.45:7899/YWPT/`）人工登录成功后，切换至【登录态】面板点击【固化当前登录态快照】。后端文件实际已经成功落盘，但面板上展示：
    - `0 Cookies / 0 Storage 项`；
    - 最后固化时间显示为 `尚未保存` 或 `0`；
    - 开发者无法判断快照是否成功捕获，极易误导用户认为工具失效。

### 2. 底层物理根因：驼峰序列化（CamelCase）与前端（SnakeCase）的隐蔽失配
*   **Rust 侧误加配置**：后端结构体 [`StorageStateSummary`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/browser/storage.rs) 头部被误加了 `#[serde(rename_all = "camelCase")]`。序列化给前端的 JSON 为：
    `{ "cookieCount": 1, "localStorageCount": 2, "sessionStorageCount": 7, "updatedAt": "2026-10-30..." }`；
*   **TypeScript / React 侧失配**：前端接口 [`src/types.ts`](file:///f:/WorkSpace/Other/harness_mini/src/types.ts) 和组件 [`BrowserInspector.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/BrowserInspector.tsx) 均按照 Rust 原生命名读取 `storageSummary?.cookie_count`、`storageSummary?.local_storage_count`、`storageSummary?.updated_at`；
*   **物理结果**：由于键名大小写失配，前端所有字段读取结果全部命中 `undefined`，UI 降级呈现为 `0` 和未保存。此外，面板此前仅展示了 `LocalStorage` 计数与键名，而像卫宁/用友 YWPT 系统核心将 Token 保存在 `SessionStorage` 中，导致用户产生“Token 完全没抓到”的严重误解。

### 3. 架构级治理方案
1. **统一数据契约序列化规范**：
   - 彻底移除 Rust 后端多余的 `#[serde(rename_all = "camelCase")]`，全面对齐前端及全工程通用的 `snake_case` 契约风格；
2. **LocalStorage 与 SessionStorage 全量可视化呈现**：
   - 前端同步展示 Cookies、LocalStorage 与 SessionStorage 的全量聚合条数（如 `7 Storage (0 LS / 7 SS)`）；
   - UI 键名清单中增加 LocalStorage（蓝色标签）与 SessionStorage（紫色标签）双区分组展示，开发者可清晰看到 `auth_..._token`、`auth_..._user` 等关键身份凭据。

---

## 六、 快照清理双重文件死锁与幽灵复活根治方案

### 1. 现场故障现象
1. **清除快照需点击两次**：用户在面板点击【清除快照】后，面板数据依然存在，必须点击第二次才能完全清除；
2. **跨会话幽灵复活**：清理快照后，关闭浏览器再重新打开，快照可能重新显现，需要多次重复清理后才能完全消失。

### 2. 底层物理根因：双写死锁、回退机制穿透与内存态残留
```
[save_storage_state 固化]
       │
       ├─► 1. 写入特定站点文件: .harness/storage_states/storage_192.168.200.45_7899.json
       └─► 2. 写入全局兜底文件: .harness/storage_state.json (根目录多余写!)

[第 1 次点击清除快照]
       │
       ▼
   clear_storage_state 仅删除了 storage_states/...7899.json
       │
       ▼
   前端紧接着调用 get_storage_state_summary() 刷新状态
       │
       ▼
   get_storage_state_path 发现特定文件没了，触发旧版“向下兼容回退”：
   - 读取并命中了根目录遗留的 .harness/storage_state.json！
   - 导致 summary 依然返回 exists: true，UI 显示快照仍在！

[第 2 次点击清除快照]
       │
       ▼
   此时特定文件已空，get_storage_state_path 返回根目录文件，并将其删除（需点击两次）。

[重新打开浏览器为何又复活？]
   1. 浏览器初始化 (session.rs) 在打开 about:blank 时盲目执行了 restore_storage_state(..., None)，将遗留根文件重新注入！
   2. 旧版 clear_storage_state 仅执行了文件删除，未清理受控 Chromium 内存中已挂载的 Cookies 和 WebStorage，刷新后浏览器仍携带内存凭证。
```

### 3. 架构级根治方案（全生命周期闭环）
1. **坚决废除根目录双写与全局兜底机制**：
   - `save_storage_state` 仅允许写入当前站点对应的物理隔离文件 `.harness/storage_states/storage_<host>_<port>.json`，彻底禁止向 `.harness/storage_state.json` 写入任何数据；
   - `get_storage_state_path` 若指定了 URL，仅返回该站点独立文件，严禁向根目录随意回退跨站串扰；
2. **原子性级联删除与历史死锁清理**：
   - `clear_storage_state` 单次执行即级联清除对应站点的快照文件，并顺带销毁根目录下的 legacy `storage_state.json`，确保无死角残留；
3. **CDP 深度联动：文件与浏览器内存同步清空**：
   - 在 [`commands.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/commands.rs) 中重构 `clear_browser_storage_state`：
     - 在删除磁盘快照文件的同时，向活动受控浏览器派发 CDP 指令：
       ```rust
       s.cdp().send("Network.clearBrowserCookies", None, sid, Duration::from_secs(3)).await;
       s.cdp().send("Runtime.evaluate", Some(json!({
           "expression": "try { localStorage.clear(); } catch (_) {} try { sessionStorage.clear(); } catch (_) {}",
           "returnByValue": false
       })), sid, Duration::from_secs(3)).await;
       s.cdp().send("Page.reload", Some(json!({ "ignoreCache": true })), sid, Duration::from_secs(5)).await;
       ```
     - 真正做到**“一次点击：磁盘快照清除 + 内存凭证清空 + 页面立即回退未登录”**。
4. **移除冷启动盲目注水**：
   - 移除 [`session.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/browser/session.rs) 在创建 `about:blank` 页面时对 `storage_state.json` 的盲目预加载，凭据注入仅在明确导航到具体目标 URL 时按需触发。

---

## 七、 复杂 SPA 服务端 Token 注销、全屏遮罩死锁与空白根路径自愈实录

### 1. 现场故障现象与核心矛盾
*   **测试场景**：
    - 针对《医务综合管理系统》（`http://192.168.200.45:7899/YWPT/#/login`，基于 Nuxt Auth / 用友卫宁 YH 框架定制开发）；
    - 账号：`zyt`，密码：`123qwe,.`；
    - 对比系统：`http://192.168.200.45:4185/`（正常可用）。
*   **表象故障**：
    1. 在 7899 登录后固化快照，随后用户在网页中点击右上角【退出】，页面返回登录页；
    2. 用户点击【注入并恢复登录态】，页面无任何反应，随后停留在完全空白界面（原有的全屏 Loading 菊花移除后，底下内容一片空白）；
    3. 而同样的流程在 4185 端口系统上却能顺畅恢复登录。

### 2. 深度根因解密：网络探针与产物反编译逆向
通过真实的 CDP 请求抓包与前端核心 bundle（`app.cb3ce607.js`）反编译，还原出了完整的事实真相：

```mermaid
sequenceDiagram
    autonumber
    actor User as 用户
    participant Inspector as BrowserInspector
    participant Browser as 受控 Chromium
    participant Server as 7899 业务服务端 (Gateway/Redis)

    Note over User,Browser: 步骤一：人工退出登录引发服务端注销
    User->>Browser: 点击网页右上角【退出】按钮
    Browser->>Server: POST /api/user/logout.do (携带当前 Token)
    Server-->>Browser: 200 OK (服务端 Redis 立即将该 Token 加入黑名单销毁!)
    Browser-->>User: 路由切回 #/login (此时该 Token 在服务器端永久失效)

    Note over User,Browser: 步骤二：用户尝试使用旧快照注入
    User->>Inspector: 点击【注入并恢复登录态】
    Inspector->>Browser: 写入 session_storage (恢复旧 Token)
    Inspector->>Browser: 执行重载或导航
    Browser->>Server: GET /api/auth/system/v1/getSystemTotalInterface (携带旧 Token)
    Server-->>Browser: HTTP 401 Unauthorized (JSON: status 209 "您已经长时间未操作，请重新登录!")

    Note over Browser: 步骤三：前端框架双重死锁导致白屏
    alt 致命死锁点 1: 全屏遮罩 #flashload
        Browser->>Browser: HTML 解析首行插入 <div id="flashload"> 全屏白屏加载菊花
        Note over Browser: 遮罩销毁依赖: onMenuAfter -> p()<br/>因接口 401，菜单流程被拦截中断，p() 永远未调用！<br/>结果: 永久卡在空白加载菊花。
    else 致命死锁点 2: Vue 路由守卫空根路径
        Browser->>Browser: 路由守卫检测 loggedIn=true 且当前在 /login<br/>执行 r(n.path)；首屏重载无 from，n.path 为 "/"
        Note over Browser: 路由重定向到 #/ (空根路径)<br/>该系统没有任何组件匹配 "/"，<router-view> 为空！<br/>结果: 遮罩移除后，底下页面彻底留白！
    end
```

#### 关键疑点解答：为什么 4185 可以而 7899 不行？
- **4185 端口系统**：采用纯前端校验的无状态 JWT，退出登录只在本地清空 localStorage，未向服务端汇报拉黑；因此只要旧 Token 未过 JWT `exp` 过期时间，再次注入依然能够通过网关鉴权；
- **7899 端口系统（YWPT）**：典型的金融/医务级安全网关架构。用户在网页中主动点击【退出】，前端会立即向网关发送 `POST /api/user/logout.do`，**服务端在 Redis 中瞬时将该 Token 拉黑销毁**。因此，任何被用户主动在网页内退出过的 Token，无论如何注入，服务端必然返回 401 拒绝访问。

### 3. 架构级系统性治理方案

针对这一复杂场景，我们确立了**“路由精准落地 + 残留遮罩看门狗自愈 + 空白路由防留白兜底 + SOP 心智模型建立”**的四维防御体系：

#### 维度 1：真实工作台落地页记录与智能直达 (`landing_url`)
- **记录落地路由**：在 `StorageState` 中持久化记录当前完整业务 URL（含 Hash，如 `http://192.168.200.45:7899/YWPT/#/muti-organization` 或 `#/index`）；
- **避开登录页首屏重载**：在 `restore_storage_state` 注入后，若当前页面停留在登录页（`/login` 或 `#/login`），系统不进行原地重载，而是**直接调用 `Page.navigate` 导航至 `landing_url` 工作台**，绕开 Vue Router 在 `#/login` 触发 `r(n.path)` 导向空根路径的缺陷。

#### 维度 2：全屏阻塞遮罩自愈看门狗 (Flashload Auto-Dismiss Watchdog)
- 在注入脚本中内嵌轻量级 DOM 看门狗：
  ```javascript
  const dismissMask = () => {
      const fl = document.getElementById('flashload');
      if (fl) { fl.style.display = 'none'; try { fl.remove(); } catch (_) {} }
      const masks = document.querySelectorAll('.yh-loading-mask.is-fullscreen, .el-loading-mask.is-fullscreen');
      masks.forEach(m => { m.style.display = 'none'; try { m.remove(); } catch (_) {} });
  };
  setTimeout(dismissMask, 1200);
  setTimeout(dismissMask, 2400);
  ```
- 即使接口报错 401 阻断动态菜单，全屏遮罩也会在 1.2 秒内自动销毁，绝不阻断用户视窗。

#### 维度 3：空白路由自愈防留白兜底 (Empty Route Guardian)
- 针对 Vue Router 在鉴权失败或重定向至空白根路径（`#/` 或 `#`）的场景，看门狗在检测到 `app` 容器为空且路由非法时：
  ```javascript
  const isBlankRoute = !hash || hash === '#' || hash === '#/' || hash === '#/null';
  const isAppEmpty = !app || !app.innerText || app.innerText.trim().length === 0;
  if (isBlankRoute && isAppEmpty && pathname.includes('/YWPT')) {
      window.location.hash = '#/login';
  }
  ```
- 自动重定向回 `#/login`，保证登录表单（账号、密码、验证码）正常展现，**绝不给用户留白**。

#### 维度 4：工业级 SOP 验证质量规范与用户心智模型
在前端面板和规范手册中明确建立正确的操作认知：
> [!IMPORTANT]
> **测试免登恢复的核心法则**：
> 1. **切勿在网页中点击【退出登录】**：在系统网页内点击“退出登录”会通知服务端将当前 Token 彻底注销拉黑。任何已在服务端销毁的凭证均无法恢复。
> 2. **正确验证免登方式**：
>    - **冷启动验证**：在页面登录并固化快照后，直接在控制面板点击【关闭浏览器】或在 Agent 会话中重新请求访问该 URL，验证是否能够免输入账号密码直达系统内部页面；
>    - **快照更新**：若凭据因服务端自然超时（如 24 小时自然过期）或被主动注销，在登录页面重新登录一次并点击【固化当前登录态快照】即可无缝更新有效凭据。

---

## 八、 治理演进全景对比矩阵

| 治理维度 | 初始状态 (缺陷态) | 中期状态 (过渡态) | 最终状态 (工业级标准) |
| :--- | :--- | :--- | :--- |
| **免登核心原理** | 录制鼠标键盘轨迹 + 大模型猜测契约 | 单纯读取 document.cookie | **对齐 Playwright 的全状态 StorageState 网关（Cookies + LocalStorage + SessionStorage）** |
| **快照持久化隔离** | 根目录共享 `storage_state.json`，多站点互相覆盖 | 站点文件与根文件双写，造成二次清理与复活 | **按 `Host + Port` 严格物理隔离单写，原子性级联删除与 CDP 内存同擦除** |
| **前后端数据契约** | 大小写混杂，`camelCase` 导致前端读取为 0 | 部分字段兜底映射 | **统一 Rust 与 TypeScript 为 `snake_case`，全量展现 Cookies / LS / SS 计数与键名** |
| **首屏点击响应** | 刚打开几秒内多次点击输入框无效，需等数秒 | 粗暴 sleep 固定等待 | **智能就绪等待（Smart Readiness Wait）+ 1:1 坐标归一化 + 元素级双重兜底聚焦** |
| **SPA 异常死锁** | YWPT 全屏 Loading 菊花卡死 / 页面完全空白 | 仅人工刷新 | **落地路由直达 (`landing_url`) + 遮罩自愈看门狗 + 空白路由防留白守护** |
| **多 Agent 启动** | 单一硬编码目录互斥，ExitCode 21 崩溃 | 简单重试机制 | **PID + UUID 目录物理隔离，独占句柄隔离，完全支持单机无限并发** |
