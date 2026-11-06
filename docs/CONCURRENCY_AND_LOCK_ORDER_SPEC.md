# 并发模型与锁序规范（CONCURRENCY & LOCK ORDER SPEC）

> 建立背景：2026-11-06 排查「发送消息瞬间崩溃」。
> 当日发生 3 次 AppCrash（17:45:35 / 18:43:18 / 18:44:41）与 2 次 AppHang（13:17:44 / 13:18:14），
> 且崩溃发生在新构建版本之上。根因是并发地基缺陷，而非某处局部 bug。
> 本文档固化治理后的约束，**后续任何改动以此为准**，避免问题复现。

---

## 一、锁类型：统一使用 parking_lot，禁止 std::sync::Mutex

### 规定
`AppState` 及所有跨线程共享状态，**一律使用 `parking_lot::Mutex` / `parking_lot::RwLock`**。

### 原因（这是崩溃放大链的根源）
`std::sync::Mutex` 有 **poisoned** 语义：一旦某线程在持锁期间 panic，该锁被永久标记为中毒，
此后**每一次** `lock()` 都会返回 `Err(PoisonError)`。

历史代码在全项目致命路径上有 **353 处 `.lock().unwrap()`**。两者叠加形成不可恢复的连锁崩溃：

```
某处 panic（持锁期间）
  → 全局 state.db 被标记 poisoned
  → 后续每个 state.db.lock().unwrap() 立即 panic
  → 发任何消息都瞬间闪退（进程直接消失，无提示）
```

这精确解释了「18:43 崩溃 → 守护进程拉起 → 18:44 再次崩溃」的聚集特征：
不是偶发抖动，而是**一次性污染后的持续崩溃**。

`parking_lot::Mutex::lock()` 直接返回 `MutexGuard`，**没有 poisoned 概念**，
`lock().unwrap()` 从语法上不可能存在，连锁崩溃链被从类型层面根除。

### 现状（已完成）
- 迁移 353 处 `.lock().unwrap()`，全部清除；
- `std::sync::Mutex` / `std::sync::RwLock` 引用归零；
- `cargo test` 221 项全部通过。

---

## 二、阻塞隔离：command 不得在主线程做 IO

### 规定
1. **凡含磁盘 IO / 数据库读写 / 逐库扫描 / vacuum 的 `#[tauri::command]`，必须是 `async fn`**，
   且阻塞部分整体下沉到 `tauri::async_runtime::spawn_blocking`。
2. 纯内存操作（无 IO）的 command 可保持同步，避免无谓的调度开销。

### 原因
Tauri 的**同步 command 会在 UI 主线程执行**。任何耗时操作都会直接冻结 webview 事件循环，
Windows 看门狗判定 Not Responding → **AppHang**。

改造前全项目有 **153 个 command、其中 104 个是同步 `pub fn`，却只有 3 处 `spawn_blocking`**，
这是主线程冻结的基层原因。

### 标准范式（以 send_message 为准）
```rust
#[tauri::command]
pub async fn send_message(...) -> Result<T, String> {
    // 1. 主线程只做零 IO 的参数校验
    // 2. 整体下沉到阻塞池；state 必须在闭包内用 app.state::<AppState>() 重新获取
    let app_for_task = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<T, String> {
        let state = app_for_task.state::<crate::AppState>();
        // ... 全部落库逻辑 ...
        Ok(result)
    })
    .await
    .map_err(|e| format!("任务执行失败: {e}"))?
}
```

### 编译器强制保证
`MutexGuard` 不是 `Send`，**跨 `.await` 持有 guard 会直接编译失败**。
因此「锁内不做 IO」这条规则由类型系统强制执行，无需依赖人工 review。

---

## 三、锁序：严禁 db → data_dir 嵌套加锁

### 规定
- 全局锁序固定为：**`db` → `master_key`**（API Key 加密主密钥）。
- `data_dir` **不得出现在任何已持锁的临界区内**。
- `is_run_active` 等查询型状态判断应在**取 db 锁之前**完成。

### 原因
历史 `send_message` 在持有全局 `db` 锁的同时，又去取 `data_dir.lock()` 与
`handles`/`active_run` 双锁，形成 `db → data_dir → handles` 的嵌套结构。
交叉加锁既是死锁风险来源，也大幅放大了锁持有时间（单一热点锁）。

---

## 四、诊断能力：panic 必须落盘

### 规定
全局 panic hook 已于 `lib.rs::run()` **最早期**安装（`panic_guard::install()`）。

### 原因
`Cargo.toml` 的 release 配置是 `strip = true` + `lto = true`，
panic 在用户侧**完全不可见**（无控制台、无符号、无弹窗），表现为「静默闪退」。
这正是本次问题长期无法定位的直接原因。

### 产物
崩溃信息写入**生效数据目录**下的 `crash.log`：
```
[2026-11-06 18:44:41.123] PANIC thread=main at src/xxx.rs:123:45
  message: called `Option::unwrap()` on a `None` value
```
超过 2MB 自动轮转为 `crash.log.1`，保留最近两代。

**排查线上崩溃时，第一件事就是看 `data\crash.log`。**

---

## 五、关于 panic = "abort" 的决策

**当前不启用**。原因：
- abort 虽让崩溃更「干净」，但会丢失崩溃上下文，与后台任务恢复策略冲突；
- 先用 panic hook 换取可诊断性；
- 锁类型换代后 panic 概率已大幅下降，后续可再评估。

---

## 六、验收清单（改动并发相关代码时必查）

- [ ] 新增共享状态使用 `parking_lot`，未引入 `std::sync::Mutex`
- [ ] 新增含 IO 的 command 为 `async fn` + `spawn_blocking`
- [ ] 未出现 `db` 与 `data_dir` 的嵌套加锁
- [ ] `cargo check` 与 `cargo test` 全部通过
- [ ] 交付前用 `build-install.cmd` 重建并核对 exe 时间戳
