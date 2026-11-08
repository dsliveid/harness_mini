# SQLite 三库分离与日志轮转实施知识沉淀
# SQLite Three-DB Separation & Log Rotation — Implementation Knowledge

> **定位**：记录 2026-11-07 存储层三库分离改造（计划 `.harness/plans/20261107-SQLite-三库分离架构实施计划-core---session-bundle---日志轮转.md`，18 步全部完成）中**解决的问题、实施方案、关键工程决策与踩坑教训**。
> 权威表结构：`docs/SQLITE_THREE_DB_SCHEMA_REFERENCE.md`；物理生命周期契约：`docs/SESSION_BUNDLE_STORAGE_AND_PHYSICAL_LIFECYCLE_SPEC.md`（§7 为本次增补）。

---

## 一、解决的问题（症状 → 根因 → 对策）

| # | 症状 | 根因 | 对策 | 兑现结果 |
|---|---|---|---|---|
| 1 | 库文件持续膨胀（`harness_mini.db` 191.7MB 且删数据不缩小） | `tool_events.result_text` / `llm_requests.payload_json` 全量内联，单行数十 KB~数 MB，行极宽导致页填充率崩塌 | 大字段内容寻址外置（sha256 + zstd + 两级散列目录），行内只留 200 字符 preview 与引用 | 2000 行×48KB 宽表：95.72MB → **0.01MB**；模糊扫描 472ms → **<1ms** |
| 2 | 首屏会话列表随总量线性退化（10 万行 95.2ms） | 列表查询跨表聚合 messages（GROUP BY），无法走索引 | sessions 表增量维护 `msg_count/last_preview/last_message_at`，列表只读元数据 + 覆盖索引 `idx_sessions_status_lastmsg(status, last_message_at DESC, id DESC)` | 目标 0.1ms 级（覆盖索引物理就绪，读路径已切换） |
| 3 | 删数据后空间不回收（incremental_vacuum 每次仅回收 1 页） | auto_vacuum=NONE 老库上执行 `PRAGMA auto_vacuum=INCREMENTAL` 实测无效；循环 incremental_vacuum 300 轮仍未清空 | 三级回收：删文件（主要）→ 删整月日志 → >50MB 小库全量 VACUUM；老库启动时「先设 PRAGMA → VACUUM 一次」自动修复 | 归档/删会话 = 删目录，回收率 100%、O(1) |
| 4 | 日志长事务阻塞核心写（实测 549ms，引发界面卡顿/闪退史） | 日志与核心数据同库共享一把写锁 | 日志独立成月轮转库 + 单写线程批量提交，调用方非阻塞入队 | 日志长事务期间核心写阻塞 113ms → **0ms** |
| 5 | 日志写入吞吐低（逐条 0.15 万行/秒） | 每条一个事务；宽行吃不到批量红利 | 窄行（大字段先外置）+ 200 条/200ms 批量事务 | **33.3 万行/秒**（约 220 倍） |
| 6 | 长任务数据在重启后"失明"（迁移引擎搬入分库、任务模块仍读主库） | 任务模块 20+ 处直连 catalog，与分库路由不一致 | `with_task_db` 全链路路由（task_id 定位会话 → session.db，回落 catalog） | 步骤11 一并修复的存量缺陷 |
| 7 | runs 跨库触发器失效风险 | 分库后触发器无法跨库同步 `last_run_status` | 摘除触发器（DROP IF EXISTS），应用层在 run 终态变更的同一操作内回写 catalog | 完成态/取消态/中断兜底三路径全覆盖 |

---

## 二、实施方案（三库架构 + 六阶段落地）

### 2.1 目标拓扑

```
data/
├── harness_mini.db              # core.db（元数据，恒定 MB 级；沿用原文件名避免迁移期双文件歧义）
├── sessions/{id}/session.db     # 会话库（消息/工具/任务/检查点 + blobs 外置索引）
├── logs/
│   ├── logs-YYYYMM.db           # 按月轮转日志库（llm_requests 审计 + tool_call_logs 出入参明细）
│   └── blobs/ab/cd/{sha256}     # 日志专属 blob 命名空间（随 TTL 整月回收）
├── blobs/ab/cd/{sha256}         # 会话大字段外置（内容寻址 + zstd，压缩无收益则 identity）
└── archives/{id}.hmsession      # 冷归档
```

### 2.2 六阶段实施内容与关键文件

| 阶段 | 内容 | 关键文件 |
|---|---|---|
| 阶段0 | 表结构权威基准 + 内容寻址外置（>8192 字节触发，zstd-3，压缩无收益回退 identity，ref_count 去重） | `docs/SQLITE_THREE_DB_SCHEMA_REFERENCE.md`、`store/blob.rs` |
| 阶段1 | 日志剥离：月轮转 + 单写线程 + 200条/200ms 批量 + TTL 清理与孤儿 blob GC；llm_requests（含 `ttft_ms/tokens_per_sec` 标量）与新增 tool_call_logs 切写；读路径日志库优先、旧库回退 | `store/logs.rs`、`agent.rs`、`commands.rs` |
| 阶段2 | core 收窄：sessions 七列增量元数据 + 三覆盖索引 + 排序键归一化 + 老库 auto_vacuum 修复 + list_sessions 重写 | `store/core.rs`、`store.rs` |
| 阶段3 | 会话分库补全：bundle schema 扩展（blobs 表 + 外置引用列幂等补列）、迁移引擎补强（schema_migrations 断点续迁 + tool_events 双向对账）、长任务全链路路由、触发器摘除改应用层、写消息增量维护 + 启动对账 | `session_bundle/bundle.rs`、`session_bundle/migrate.rs`、`lib.rs`、`task/*` |
| 阶段4 | 治理与回滚：storage.mode 开关（single/hybrid/split）、optimize_database 分库化（>50MB 会话库 VACUUM）、归档闭环（storage_state 同步） | `commands.rs`、`store/core.rs` |
| 阶段5 | 前端适配（Session 轻量字段、Sidebar 预览、存储模式设置入口）+ 文档回写 + 全量验收 | `src/types.ts`、`Sidebar.tsx`、`DataDirSection.tsx`、SPEC §7 |

### 2.3 核心模块设计

**日志写线程（store/logs.rs）**
- 有界队列（256）+ `try_send` 非阻塞投递：队列满丢弃审计记录并告警，**绝不阻塞业务写路径**（硬性设计约束）。
- 单写线程批量提交：首批到达后最多等 200ms 或攒满 200 条，按月分组各开一个 `BEGIN IMMEDIATE ... COMMIT`。
- 大字段在**写线程内落库时**外置（调用方零文件 IO）；4MB 审计硬上限（字符边界安全截断）。
- TTL 清理走同一队列串行执行（Windows 上打开中的 DB 文件无法删除，避免句柄争抢）。
- `ttft_ms`：首个可见增量（正文或思维链）到达的毫秒数；`tokens_per_sec`：usage completion tokens / 首增量后的生成时长。每次流式尝试（含重试）独立留痕，标量回填精确对应各自请求行。

**增量维护 + 启动对账（store/core.rs）**
- 跨库无法共享事务，采用「同一操作内先后提交两库 + 启动对账兜底」：
  - 写消息路径 `apply_message_metadata`（流式占位只计数不覆盖预览、终态正文 `refresh_message_preview`）；
  - 启动 10s 后台 `reconcile_all_session_metadata` 以 session.db 为事实源修正 `msg_count/tool_event_count/last_preview/bundle_bytes` 偏差（幂等，单会话失败不阻断）。

**迁移引擎（session_bundle/migrate.rs 补强）**
- 显式枚举列名（严禁 SELECT *）；messages + tool_events 双向行数对账；对账失败事务 drop 即回滚、绝不删源数据；每会话完成记录 `schema_migrations(split:<id>)` 支持断点续迁。

---

## 三、关键工程决策与理由

1. **日志 blob 独立命名空间（`logs/blobs/`，不与全局 `blobs/` 混用）**
   TTL 删整月库文件时该月注册表（含 ref_count）随之消失；若混用全局命名空间，将无法区分「日志遗留孤儿」与「会话在用 blob」，要么误删会话数据、要么永久累积孤儿。隔离后删月文件 + 孤儿 GC 可安全执行，实现日志空间 100% 回收。代价：跨月重复内容不去重（重复率低，可接受）。

2. **core.db 沿用 `harness_mini.db` 文件名**
   避免迁移期出现两个"主库"文件的歧义；三库是逻辑划分，物理上 catalog 即 core。

3. **`migrate_split.rs` 落位为 `session_bundle/migrate.rs` 补强**
   计划硬性约束「必须复用并扩展 session_bundle/，不得另起炉灶」优先于文件清单的字面新增项；两条迁移路径（存量自动迁移 + 断点续迁记录）共用同一 schema_migrations。

4. **storage.mode 用 AtomicU8 缓存而非每次查 settings**
   `with_session_db` 是热路径，逐调用回查 settings 会给 catalog 锁增加无谓争用；原子量随启动加载、`set_storage_mode` 命令与数据目录热切换时同步。

5. **锁纪律：绝不在持有会话连接时获取 catalog 锁（反之亦然）**
   所有「session.db 写 + catalog 元数据同步」的组合均为**先后串行**获取两把锁（with_session_db 返回后再 `state.db.lock()`），规避 ABBA 死锁。本次改造中多处借机拆掉了原有的嵌套锁块。

6. **runs 兜底块回写读取真实最新终态而非硬编码**
   run 循环结束的兜底块在正常完成后也会执行，若硬编码写 `interrupted` 会覆盖 `done`；改为读 session.db 中最新 run 的实际 status 回写，天然幂等。

---

## 四、踩坑记录（避免重蹈）

| 坑 | 现象 | 教训 |
|---|---|---|
| SQL 批处理混入 `//` 注释 | `execute_batch` 报 `near "/": syntax error`，81 个测试瞬间全红（init_schema 失败连锁） | rusqlite `execute_batch` 只认 `--` 与 `/* */`；在 r#""# SQL 字符串里写 Rust 风格注释编译器不报错，靠测试兜底 |
| `SyncSender::send_timeout` 在 rustc 1.98 报 E0658（std_internals） | 编译失败 | 有界队列场景直接用 `try_send` + 丢弃策略，不依赖阻塞投递；还规避了 parking_lot Guard 上的方法解析歧义 |
| `if let Some(ref x) = *guard` 临时守卫生命周期 | E0597 `st does not live long enough` | 先 `let guard = ...lock()` 显式绑定再解引用 |
| 老库 `PRAGMA auto_vacuum=INCREMENTAL` 设定后读回仍为 0 | 空间回收模式"看起来设了其实没生效" | 必须「先设定 → VACUUM 一次」模式才持久化；新库则必须建表**前**设定 |
| Content-addressed 去重让"每月一份"测试假象 | 同内容跨月写入物理上只有一份文件，按文件数断言必错 | 构造测试数据时用**不同内容**才能验证"孤儿 GC"路径 |
| 长任务模块与迁移引擎的数据源漂移 | 新会话任务写 catalog，重启后被迁入 session.db，任务模块永久读空 | 新增写路由时必须全局检索同类读写点（grep 表名而非函数名），否则路由不一致成为隐性数据丢失 |

---

## 五、验收数据

- **正确性**：`cargo check --all-targets` 0 错误；`cargo test --lib` 245 全绿（blob 4 / logs 8 / core 4 + 既有迁移/任务/并发测试）；`npx tsc --noEmit`、`npm run build` 通过。
- **性能（scripts/sqlite_bench/bench5，release 模式）**：

| 指标 | 改造前 | 改造后 |
|---|---|---|
| 日志长事务期间核心写阻塞 | 113ms（同库） | **0ms**（分库） |
| 日志写吞吐（窄行 200B） | 0.15 万行/秒（逐条） | **33.3 万行/秒**（200 条/事务） |
| 大字段模糊扫描（2000 行 × 48KB） | 472ms / 95.72MB | **<1ms / 0.01MB** |
| 删除会话/过期日志磁盘回收 | incremental_vacuum 无法清空 | 删目录/删月文件，100% |

- **基准复现**：`cd scripts/sqlite_bench && cargo run --release --bin bench5`。

---

## 六、遗留与运维注意事项

1. **次要路径计数偏差**：引导消息、子 Agent 提示词等旁路消息未做计数增量，偏差由启动对账自动修正（日志 `[Core] 启动对账完成: N 会话已核对, M 跳过`）。
2. **回滚通路**：设置页「存储模式 → 单库」一键切回 `single`（读写一律 catalog），未迁移数据保持可用；有任务运行时切换被拒绝。
3. **TTL 语义**：日志保留「最近 N 个整月 + 当月」，默认 N=6，可经 settings 行 `logs.retention_months` 覆盖；过期月库删除后其专属 blob 由 GC 兜底。
4. **旧写路径保留**：`save_llm_request_trace`（含 lean payload 瘦身）标注 `#[allow(dead_code)]` 保留为 single 模式回滚写路径。
5. **收益兑现检查点**：首屏 0.1ms 级依赖覆盖索引 + `last_message_at` 非空（`ensure_core_evolution` 已做归一化）；若手工清空过 sessions 表元数据，重启一次即自动回填。

---

## 七、可复用的通用经验

1. **时序大数据与关系元数据必须物理分库**：写放大特征（宽行日志）与读放大特征（列表聚合）在同一把写锁下互为灾难；分库是消除互相阻塞的唯一手段，索引只是治标。
2. **跨库一致性放弃幻想，选择「操作内先后提交 + 幂等对账」**：SQLite 无跨库事务，把对账做成启动期幂等任务，比发明分布式协议便宜一个数量级。
3. **内容寻址外置是 SQLite 宽表的标准解**：sha256 去重 + zstd + preview 冗余三件套，读路径"列表绝不触碰 blob、详情懒加载"。
4. **有界队列 + 非阻塞投递 + 宁丢不堵**是旁路数据（审计/日志）写入的铁律；业务路径的延迟预算永远优先于旁路数据的完整性。
5. **每次 schema 演进必须同时给出老库幂等升级路径**（ensure_column / DROP IF EXISTS / 一次性修复任务），否则灰度期用户数据必然踩坑。
