# WebView2 白屏/窗口不显示：进程完整性级别与缓存目录治理规范

> 2026-11-20 定案。适用于「程序启动了、进程活着、日志正常，但界面完全不显示」这一类问题。
> 相关代码：`src-tauri/src/single_instance.rs`、`src-tauri/src/lib.rs`。

---

## 一、现象

典型表现（本项目的真实案例）：

1. 双击启动后**界面闪现一下就没有了**，或**完全没有窗口**；
2. 进程在任务管理器里**存活**、`Responding = True`；
3. 日志正常输出到「启动对账完成」后**再无输出**，没有 panic，`crash.log` 不存在；
4. Windows 事件日志里**没有** AppCrash / AppHang 记录；
5. 系统偶尔弹出 `Microsoft Defender SmartScreen 阻止了无法识别的应用启动`（**这条是次要的**，点「仍要运行」即放行）；
6. `tauri dev` 控制台偶见一条 `can't open file for write !`。

## 二、关键判别证据（不要凭感觉，按此表逐项确认）

| 检查项 | 正常 | 异常（本问题） |
|---|---|---|
| `msedgewebview2.exe` 子进程数 | 6 个左右 | **0 个** |
| 主窗口 `IsWindowVisible` | `True` | **`False`** |
| 主窗口矩形 | 1440x860 且已渲染 | 存在（如 1440x759），但**空壳** |
| `inner_size()` / `url()` | 正常返回 | **`Err(FailedToReceiveMessage)`** |
| 进程内存 | 数百 MB | **70~80 MB**（只起了 tokio 运行时 + 托盘） |
| 自己的 `ebwebview_<hash>` 目录 | 被持续写入 | **从未写入** |
| `crash.log` / WER 事件 | — | **均无记录**（说明不是崩溃） |

> `FailedToReceiveMessage` 是**决定性信号**：窗口对象存在，但无法与 WebView 通信 ——
> 即「窗口壳建好了，渲染引擎没起来」。

## 三、根因：Windows 强制完整性控制（MIC）

### 3.1 因果链

```
exe 从「带 Low Mandatory Level 标签的目录」启动
  → 进程被强制降级为「低完整性级别（Low IL）」
  → MIC 强制拒绝它写「中完整性」对象 %LOCALAPPDATA%（连 %TEMP% 也拒）
  → WebView2 无法创建用户数据目录
  → CreateCoreWebView2Environment 静默失败，原生层打印 "can't open file for write !"
  → 无浏览器进程、无渲染结果
  → 窗口壳存在但内容为空
  → 叠加 tauri.conf.json 的 "transparent": true + "decorations": false
  → 全透明空窗 = 用户视角「什么都没出现」
```

### 3.2 目录 ACL 的实际差异

```
# 异常目录（沙箱标记，例如 agent 工作区）
F:\WorkSpace\Other\harness_mini
    Everyone:(CI)(DENY)(S,DC)                          ← 显式拒绝 Everyone
    S-1-4-870895770-31734639:(OI)(CI)(W,D,DC)           ← 非标准沙箱 SID
    Mandatory Label\Low Mandatory Level:(OI)(CI)(NW)    ← 低完整性标签（元凶）

# 正常目录
F:\Software\HarnessMini
    （以上三项均不存在）
```

判定命令：

```powershell
icacls "F:\WorkSpace\Other\harness_mini"
# 出现 "Mandatory Label\Low Mandatory Level" 即为高危
```

### 3.3 为什么「换终端」「重启」「退杀软」都无效

- **换终端无效**：限制绑定在**目录 ACL 标签**上，与由哪个终端启动无关；
- **重启无效**：标签是持久化的文件系统属性；
- **退出杀软无效**：根因不是杀软（火绒 `HipsDaemon` / `sysdiag.sys` 等驱动确实常驻，
  但本问题与它无关）；
- **只有把 exe 放到无标签目录才会正常** —— 我们实测：同一个 exe 复制到 `F:\_probe_test`
  后，7 个写探针全部成功。

### 3.4 如何验证自己是否踩中

```powershell
# 1) 看进程完整性级别（Low / Medium / High）
whoami /groups | Select-String "Mandatory Level"

# 2) 看目录标签
icacls <项目根目录> | Select-String "Mandatory|DENY|S-1-4"
```

## 四、代码侧根治：实测可写性 + 逐级回退

**核心理念：不要对环境做任何假设，真写一次再决定。**

`single_instance.rs` 提供两个函数：

```rust
/// 真写探针：创建目录 + 写文件 + 删除，返回是否可写
pub fn probe_writable(dir: &Path) -> bool;

/// 按候选链挑选 WebView2 用户数据目录，并把结果写入环境变量
pub fn setup_webview_isolation(dir_hash: &str) -> PathBuf;
```

回退顺序（第一个可写者胜出）：

| 顺序 | 候选 | 说明 |
|---|---|---|
| 1 | 进程已有 `WEBVIEW2_USER_DATA_FOLDER` | 尊重外部显式配置，**但先验证可写**（不可写则丢弃，不再盲从） |
| 2 | `%LOCALAPPDATA%\com.harness.mini\instances\ebwebview_<hash>` | 常规路径，多开隔离；release 走这条 |
| 3 | `<exe 同级>\webview_cache_<hash>` | **受限环境主用**（exe 所在目录常带可写标签） |
| 4 | `<Temp>\com.harness.mini\ebwebview_<hash>` | 最后兜底 |

行为约定：

- 每级失败打印 `[single_instance][WARN] WebView2 候选目录不可写: <路径>`；
- 发生回退时明确提示原因「进程完整性级别被降级，无法写 %LOCALAPPDATA%」；
- 全部不可写时仍设最后一个候选并打印 `[ERROR]`，**不让调用方拿到 None**
  （避免陷入无任何日志的空白窗口）。

## 五、精简后的诊断日志（线上保留）

```text
[Panic Guard] 已安装全局 panic 捕获钩子
[Startup] exe = ... | dir_hash = ... | 继承的 WEBVIEW2_USER_DATA_FOLDER = ...
[Startup] WebView2 用户数据目录（实际生效）= ...
[Startup] 生效 data_dir = ... | 内存库 = false | 不可写路径 = None
[Core] 启动对账完成: N 会话已核对, 0 跳过
# 仅在异常时才额外出现：
[Startup][WARN] 5s 后主窗口仍不可见：检查上方 WebView2 用户数据目录是否可写（低完整性级别进程无法写 %LOCALAPPDATA%）
```

判断口诀：**看「WebView2 用户数据目录」落在哪**。

- 落在 `...\instances\ebwebview_<hash>` → 常规路径，正常；
- 落在 `...\target\debug\webview_cache_<hash>` → 已触发回退，说明环境受限但**已自愈**。

## 六、排查清单（下次遇到直接照做）

1. **数一数 `msedgewebview2.exe` 子进程** —— 0 个就锁定本问题类型；
2. `icacls <项目根>` 看有无 `Mandatory Label\Low Mandatory Level`；
3. `whoami /groups | Select-String "Mandatory Level"` 看进程 IL；
4. 把 exe 复制到 `F:\` 根下某个新目录再启动，对比是否正常（**最省时的定性实验**）；
5. 查 `crash.log` 与 WER —— 两者皆空才说明是「静默失败」而非崩溃；
6. 不要浪费时间在：换终端、重启、退出杀软、改 SmartScreen 设置。

## 七、不要踩的坑

- **`tauri dev` 的 exe 与 `npm run build` 的 dist 都不是用户所见**：前端在编译期嵌入 exe，
  改动必须重建才生效（详见 `conventions.md`）。
- **`src-tauri\debug.log` 不是我们的日志**：里面是搜狗输入法注入 Chromium 的 WARNING，
  `[<pid>:...]` 前缀是该实例的浏览器子进程，**不含任何我们的错误**。
- **单元测试也不能假设 `%TEMP%` 可写**：受限环境下测试进程同样被拒。测试应断言
  「探针返回值与实际写结果一致」，而不是「一定可写」。
- **不要用 `WEBVIEW2_USER_DATA_FOLDER` 的「已存在就不覆盖」写法**：继承来的脏值优先级
  高于隔离逻辑，是静默失败的常见来源。当前实现改为「先验证可写再采用」。

## 八、验证记录（2026-11-20）

```text
cargo check                                  → Finished，0 error
cargo test single_instance
  test_dir_to_hash_case_and_trailing_slashes ... ok
  test_setup_webview_isolation_env_var       ... ok   # 含「选中目录必须真的可写」断言
  test_probe_writable_detects_writable_and_unwritable ... ok  # 探针与实际写结果一致性
```

修复后用户实测：主界面正常显示。
