# 目录完整性标签（Low IL）统一根因与根治规范

> 2026-11-25 定案。**本文是「目录被沙箱加固污染」这一类问题的统一根因文档。**
>
> 适用于以下**看似无关、实则同一根因**的全部现象：
> - 「打开目录」按钮点了没反应（explorer 窗口不出现）
> - 程序启动后界面白屏 / 完全没有窗口（WebView2 起不来）
> - 进程存活、日志正常、无 panic、无 WER 崩溃记录，但功能静默失效
>
> 关联文档：`docs/WEBVIEW2_BLANK_WINDOW_INTEGRITY_LEVEL_SPEC.md`（该文描述的是本文根因在
> WebView2 场景下的**表象与代码侧兜底**，其第四章的候选目录回退属**防御性兜底，不是根治**）。

---

## 一、核心结论（一句话）

> **exe 所在目录若带 `Low Mandatory Level` 完整性标签，从该目录启动的进程会被 Windows
> 强制降级为「低完整性进程（Low IL）」，此后它既写不了 `%LOCALAPPDATA%`，也拉不起
> 任何「中完整性」子进程（explorer、cmd、powershell、msedgewebview2 全部失效）。**

这不是代码 bug，是**目录 ACL 被污染**。改代码只能兜底，**根治必须修目录权限**。

## 二、两个表象症状，同一根因

> ✅ **2026-11-25 更新**：该根因现已**彻底修复**（目录标签已清除），进程恢复 Medium/High。
> 因此下游各处的「低完整性兜底代码」（如浏览器模块禁止用 `std::env::temp_dir()`）虽然
> 保留无妨，但**其存在本身不再代表诊断结论**——排查新问题时不要一上来就归因于 Low IL，
> 先用第三节的方法实测确认。

```
                    ┌─ 目录带 Low Mandatory Level 标签
                    │
                    ▼
        从该目录启动的 exe 被降级为 Low IL (RID=4096)
                    │
        ┌───────────┴────────────┐
        ▼                        ▼
  ① 写不了中完整性对象      ② 拉不起中完整性子进程
     %LOCALAPPDATA%            (UIPI 拦截)
     (连 %TEMP% 也拒)              │
        │                        ▼
        ├──► WebView2 无法创建   explorer.exe 秒退
        │    用户数据目录        (0xC0000142)
        │         │                  │
        │         ▼                  ▼
        │    CreateCoreWebView2   「打开目录」静默失败
        │    Environment 静默失败  （但 RPC 返回 ok:true）
        │         │
        │         ▼
        │    窗口壳存在但内容为空
        │    + transparent:true
        │    → 用户视角「界面打不开 / 白屏」
        │
        └──► Chromium 无法创建 profile
             锁文件 / Crashpad throttle_store
             → "Error code: 5" (ACCESS_DENIED)
             → 浏览器 ExitCode 21 / 渲染进程崩溃
```

**已知受此根因影响的下游症状（全部同源）**：

| 下游症状 | 具体表现 | 相关文档 |
|---|---|---|
| WebView2 白屏 | 进程活着但无窗口、`FailedToReceiveMessage` | `WEBVIEW2_BLANK_WINDOW_INTEGRITY_LEVEL_SPEC.md` |
| 打开目录失效 | explorer 秒退 `0xC0000142`，RPC 仍返回 `ok:true` | 本文 |
| 浏览器启动失败 | `Error code: 5` 锁文件失败、`ExitCode 21` | `BROWSER_BATCH_RELIABILITY_AND_SEMANTIC_PROBE_KNOWLEDGE.md` |
| 单元测试写临时目录失败 | `std::env::temp_dir()` 写入 `os error 5` | 同上（已改为禁止使用 `temp_dir()`） |

这些症状的**唯一共同变量就是目录标签**。这也是为什么「换终端、重启、退杀软、改
SmartScreen」全部无效——限制绑定在持久化的文件系统属性上。


## 三、判别方法（30 秒定性）

```powershell
# ① 看目录标签 —— 出现下面任意一行即为高危
icacls <项目根目录>
#   Mandatory Label\Low Mandatory Level:(OI)(CI)(NW)   ← 元凶
#   S-1-4-<一串数字>:(OI)(CI)(W,D,DC)                  ← 非标准沙箱 SID
#   Everyone:(CI)(DENY)(S,DC)                          ← 显式拒绝 Everyone

# ② 看进程完整性（RID：4096=Low / 8192=Medium / 12288=High）
#    注意：必须匹配 "Mandatory Label"，否则每一行都含 "Mandatory group" 会刷屏
whoami /groups | Select-String "Mandatory Label"
# 期望：Mandatory Label\Medium Mandatory Level  (S-1-16-8192)
#       Mandatory Label\High Mandatory Level    (S-1-16-12288)
# 异常：Mandatory Label\Low Mandatory Level     (S-1-16-4096)  ← 就是本问题
```

**最省时的定性实验**：把同一个 exe 复制到磁盘根下某个**无标签**的新目录再启动。
若行为恢复正常，即 100% 确认是目录标签问题。

> ⚠️ **注意**：`Get-Acl` **读不到**完整性标签（它在 SACL 里）。必须用 `icacls`
> 或 `GetNamedSecurityInfoW(..., LABEL_SECURITY_INFORMATION, ...)`。

## 四、实测证据矩阵（2026-11-25 全流程取证）

### 4.1 进程完整性对照

| 场景 | 令牌 RID | 结论 |
|---|---|---|
| dev 进程（从带标签目录启动） | **4096 = Low** | 被降级 |
| 同一 exe 移出该目录后启动 | 8192 / 12288 = Medium/High | 正常 |
| 安装版 `F:\Software\HarnessMini`（无标签） | 12288 = High | 一直正常 |

### 4.2 各类「打开目录」方案的实测结果（Low IL 进程下）

| 方案 | 结果 | 可靠性 |
|---|---|---|
| `ShellExecuteExW(path)` | 返回 `False`，`err=1223`（`ERROR_ELEVATION_REQUIRED`） | **可靠的失败信号** |
| `spawn("explorer.exe", path)` | 进程起来后秒退 `0xC0000142`（`STATUS_DLL_INIT_FAILED`） | ⚠️ **假成功**（`spawn` 返回 Ok） |
| `cmd /c start "" <path>` | 退出码 `1` | 失败 |
| `rundll32 url.dll,FileProtocolHandler` | 退出码 `0`，但**窗口不出现** | ☠️ **最危险：假成功** |
| `powershell Invoke-Item` | 退出码 `1` | 失败 |

> **关键教训**：`Command::spawn()` 只看「进程是否创建成功」，进程起来后立刻死掉它照样返回
> `Ok(())`。这就是故障能长期静默的原因。

### 4.3 修复后的实测结果

| 验证项 | 修复前 | 修复后 |
|---|---|---|
| dev 进程 RID | 4096（Low） | **12288（High）** |
| 目录 SACL | `ace_count=1`（Low 标签） | **`SACL=NULL`** |
| 根目录 DACL | 含 3 条异常 ACE | **与正常目录逐条一致（8 条继承 ACE）** |
| `open_dir` RPC 实测 | 返回 `ok:true`，窗口不出现 | **explorer 窗口真正弹出** ✓ |

## 五、根治方案（三步，只改根目录一处）

### 5.1 为什么不需要 `/T` 递归

异常 ACE 与完整性标签**只显式设置在根目录**，子项全部是 `(I)` 继承来的
（实测抽样 40 个目录，显式标签 0 个）。因此在根目录写入一条**可继承**的修正，
会自动级联覆盖全部子项，**无需遍历 15 万个子项**。

### 5.2 操作步骤

> 需要**管理员权限**。修改 SACL 还必须在**同一进程内**启用 `SeSecurityPrivilege`。

**步骤 1：移除 `Everyone:(CI)(DENY)(S,DC)`**

```powershell
icacls "<项目根目录>" /remove:d "*S-1-1-0"
```

> ⚠️ 该命令会尝试传播目录树，在数十万项的大目录上**容易超时（120s）**，
> 但超时后往往**仍已生效**。超时后务必重新 `icacls` 核对现场，不要盲目重试。

**步骤 2 & 3：用 Windows API 精确处理（不递归，立即生效）**

调用 `SetEntriesInAcl(REVOKE_ACCESS)` 移除沙箱 SID 的显式 ACE，再用
`SetNamedSecurityInfoW` 清空 SACL 标签：

```csharp
// 关键常量
const int    SE_FILE_OBJECT                      = 1;
const uint   DACL_SECURITY_INFORMATION           = 0x00000004;
const uint   LABEL_SECURITY_INFORMATION          = 0x00000010;
const uint   PROTECTED_SACL_SECURITY_INFORMATION = 0x10000000;

// 步骤 3：清空完整性标签，并置 protected（不再从父级继承标签）
SetNamedSecurityInfoW(path, SE_FILE_OBJECT,
    LABEL_SECURITY_INFORMATION | PROTECTED_SACL_SECURITY_INFORMATION,
    IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero);
```

> `PROTECTED_SACL_SECURITY_INFORMATION` 是**必须的**——不加它，清空后会立刻从父目录
> 重新继承，等于白做。

### 5.3 验证

```powershell
icacls "<项目根目录>"
# 期望输出与同盘正常目录完全一致，且不含 Mandatory / S-1-4 / DENY

icacls "<项目根目录>\src-tauri\target\debug"
# 子目录同样清洁（级联生效）
```

**期望的正常目录基线**（同盘任一未被污染的项目目录即可对照）：

```
BUILTIN\Administrators:(I)(F)
BUILTIN\Administrators:(I)(OI)(CI)(IO)(F)
NT AUTHORITY\SYSTEM:(I)(F)
NT AUTHORITY\SYSTEM:(I)(OI)(CI)(IO)(F)
NT AUTHORITY\Authenticated Users:(I)(M)
NT AUTHORITY\Authenticated Users:(I)(OI)(CI)(IO)(M)
BUILTIN\Users:(I)(RX)
BUILTIN\Users:(I)(OI)(CI)(IO)(GR,GE)
```

修好后**必须重启应用**（已运行的进程无法提升自身完整性级别）。

## 六、代码侧兜底 vs 目录侧根治（重要区分）

| | 目录侧根治（本文第五节） | 代码侧兜底 |
|---|---|---|
| 做法 | 修复目录 ACL 标签 | `probe_writable` + 候选目录逐级回退 |
| 效果 | 进程恢复 Medium/High，**所有**子进程/写入能力恢复 | 仅让 WebView2 找到可写目录，**其他能力仍残缺** |
| 覆盖范围 | 全局 | 只管 WebView2 一个点 |
| 定位 | **首选，真正的根治** | 防御性设计，环境再被污染时不至于完全不可用 |

> `WEBVIEW2_BLANK_WINDOW_INTEGRITY_LEVEL_SPEC.md` 第四章的候选目录回退**不是**根治方案。
> 它能让白屏问题自愈，但**在 Low IL 下「打开目录」「命令执行」等功能依然全废**。
> 本次问题（打开目录失效）正是「只做了兜底、没做根治」暴露出来的缺口。

## 七、遗留代码缺陷（待修）

`src-tauri/src/commands.rs` 的 `open_dir` 当前实现：

```rust
let res = std::process::Command::new("explorer").arg(&win_path).spawn();
if let Err(e) = res { /* 只有启动失败才报错 */ }
```

**问题**：进程起来后秒退（`0xC0000142`）时 `spawn()` 仍返回 `Ok`，前端因此静默无提示。

**建议改法**（任选其一或组合）：

1. 优先用 `ShellExecuteExW` 并检查返回值 —— 实测在 Low IL 下会可靠返回
   `False + err=1223`，是**明确可判**的失败信号；
2. `spawn` 后短暂等待（如 300~500ms），若进程已退出且退出码非 0
   （尤其 `0xC0000142`）则回落并返回错误；
3. 前端侧对 `open_dir` 增加结果提示，避免「点了没反应」的空白体验。

## 八、排查清单（下次直接照做）

1. **先看目录标签**：`icacls <项目根> | Select-String "Mandatory|S-1-4|DENY"`
2. **再看进程 IL**：RID `4096` = Low，就是本问题；
3. **定性实验**：把 exe 复制到无标签新目录启动，行为是否恢复；
4. **不要浪费时间在**：换终端、重启（改之前）、退出杀软、改 SmartScreen、重装依赖；
5. 确认是本问题后，按第五节修目录 → **重启应用** → 复验。

## 九、避坑清单

- **`Get-Acl` 读不到完整性标签**（在 SACL 里）。必须用 `icacls` 或
  `GetNamedSecurityInfoW(LABEL_SECURITY_INFORMATION)`。
- **`icacls /reset` 对完整性标签无效** —— 它会重新从父级继承，标签原样回来。
- **`icacls /setintegritylevel` 必须指定级别**（`L`/`M`/`H`），无法用于「移除标签」。
  要彻底移除只能用 API 清空 SACL + `PROTECTED_SACL_SECURITY_INFORMATION`。
- **改 SACL 前必须启用 `SeSecurityPrivilege`**，否则返回 `1314`
  （`ERROR_PRIVILEGE_NOT_HELD`），哪怕当前 shell 已是管理员。
- **P/Invoke `EXPLICIT_ACCESS_W` 的 `TRUSTEE_W` 结构必须完整**：
  `pMultipleTrustee` + `MultipleTrusteeOperation` + `TrusteeForm` + `TrusteeType` +
  `ptstrName`。少写前两个字段会导致进程直接崩溃（`exit -1073740940`）。
- **`spawn()` 成功 ≠ 功能成功**：务必校验子进程的实际存活/退出码。
- **不要用 `rundll32` 之类「退出码 0 但没动作」的方案兜底** —— 它比直接失败更难排查。
- **`icacls /save` 才是能完整备份含 SACL 的方式**；`Get-Acl` 的 SDDL 不含完整性标签。

## 十、验证记录（2026-11-25）

```text
修复前：根目录 DACL 含 Everyone DENY + 沙箱 SID；SACL 含 Low 标签 ace_count=1
         dev 进程 RID = 4096 (Low)
         open_dir RPC → {"ok":true}，explorer 窗口不出现（静默失败）

修复后：根目录 DACL 与同盘正常目录逐条一致（8 条继承 ACE）
         src / src-tauri / target / debug / .git / .dev-data / .harness 抽样均无标签
         SACL = NULL
         dev 进程重启后 RID = 12288 (High)
         open_dir RPC → {"ok":true}，explorer 窗口真正弹出 ✓
```
