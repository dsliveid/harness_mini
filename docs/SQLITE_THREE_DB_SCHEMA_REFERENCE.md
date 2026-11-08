# SQLite 三库分离架构 · 表结构权威速查手册

> **定位**：本文档是 harness_mini 存储层「三库分离」改造的唯一权威表结构基准，与代码同步维护。
> 任何 schema 变更必须先改本文档（或同 PR 内同步），再改代码。
> 关联文档：`docs/SESSION_BUNDLE_STORAGE_AND_PHYSICAL_LIFECYCLE_SPEC.md`（物理生命周期契约）。
> 实施计划：`.harness/plans/20261107-SQLite-三库分离架构实施计划-core---session-bundle---日志轮转.md`。

---

## 0. 实测硬结论（方案取舍依据，来自 scripts/sqlite_bench）

| 编号 | 实测结论 | 对 schema 的约束 |
|---|---|---|
| 1 | 大字段外置：库体积 191.7MB→0.5MB，模糊扫描 284.4ms→1.2ms（237 倍），写入吞吐 883→761 行/秒（-14%） | 大字段必须外置，接受写入微降 |
| 2 | 批量事务：窄行 16.4~31 倍加速；宽行仅 1.2 倍 | 日志写入必须窄行+批量，宽字段外置后才能吃到批量红利 |
| 3 | 混库时日志长事务阻塞核心写 549ms；分库后 0.2ms | 日志必须独立成库 |
| 4 | page_size 8192 比 4096 写入快 1.6 倍 | 新库统一 page_size=8192 |
| 5 | 老库执行 `PRAGMA auto_vacuum=INCREMENTAL` 实测仍返回 0(NONE) | 必须建库前设定；老库需先 VACUUM 一次再设定 |
| 6 | `incremental_vacuum(2000)` 一次仅回收 1 页，循环 300 轮 2044ms 仍未清空；全量 VACUUM 仅 9~10ms | 放弃 incremental_vacuum 循环，改用「分库+删文件+小库 VACUUM」 |
| 7 | VACUUM 阻塞：39MB 库 933ms，117MB 库 2254ms | VACUUM 仅对单库 >50MB 的小库在空闲时段执行 |
| 8 | 单会话历史查询走索引恒定 0.1~0.2ms | 分库目的不是读历史更快，而是首屏/磁盘/删除回收 |

---

## 1. 目录布局与分层职责

```
data/
├── harness_mini.db              # core.db（沿用现有文件名，避免迁移期双文件歧义）
├── harness_mini.db-wal
├── sessions/
│   └── {session_id}/
│       ├── session.db           # 会话专属库（消息/工具/任务/检查点）
│       ├── session.db-wal
│       └── attachments/         # 会话附件（既有 bundle_attachments_dir）
├── logs/
│   ├── logs-202511.db           # 按月轮转的日志库（YYYYMM 六位数字）
│   ├── logs-202511.db-wal
│   └── blobs/                   # 日志专属 blob 命名空间（见 §4.4 生命周期说明）
│       └── ab/cd/{sha256}
├── blobs/                       # 会话大字段外置（内容寻址，sha256 去重）
│   └── ab/cd/{sha256}           # 两级散列目录，避免单目录文件过多
└── archives/
    └── {session_id}.hmsession   # 归档包（既有 archiver）
```

| 库 | 内容 | 体积特征 | 生命周期 |
|---|---|---|---|
| `core.db` | 配置、项目、会话元数据、密钥、MCP、成长规则、日统计 | 恒定 1~10MB | 长期保留，需备份 |
| `sessions/<id>/session.db` | 该会话全部对话/工具/任务数据 | 单会话 MB 级 | 归档即整目录移走 |
| `logs/logs-YYYYMM.db` | LLM 请求审计、工具调用出入参明细 | 单月几十 MB | 按 TTL 删文件 |

---

## 2. core.db（元数据库）

> 建库 PRAGMA（**一次性、建表前设定**，顺序不可颠倒）：
> ```sql
> PRAGMA page_size = 8192;
> PRAGMA journal_mode = WAL;
> PRAGMA synchronous = NORMAL;
> PRAGMA busy_timeout = 5000;
> PRAGMA auto_vacuum = INCREMENTAL;   -- 必须建表前，老库需先 VACUUM 一次才能生效（实测结论 5）
> PRAGMA foreign_keys = ON;
> PRAGMA cache_size = -65536;
> ```

### 2.1 `settings` —— 全局键值配置（保留原样）
| 列 | 类型 | 约束 | 说明 |
|---|---|---|---|
| key | TEXT | PRIMARY KEY | 配置键名 |
| value | TEXT | NOT NULL | JSON 或纯文本值 |

### 2.2 `projects` —— 项目定义（保留原样）
| 列 | 类型 | 约束 |
|---|---|---|
| id | TEXT | PRIMARY KEY |
| name | TEXT | NOT NULL |
| path | TEXT | 工作区根路径 |
| pinned | INTEGER | NOT NULL DEFAULT 0 |
| created_at | TEXT | NOT NULL |
| constraints | TEXT | NOT NULL DEFAULT '' |
| sop_verify_cmd | TEXT | |
| sop_enabled | INTEGER | NOT NULL DEFAULT 1 |
| plan_mode | TEXT | NOT NULL DEFAULT 'standard' |

### 2.3 `project_links` —— 项目关联目录（保留原样）
| 列 | 类型 | 约束 |
|---|---|---|
| id | TEXT | PRIMARY KEY |
| project_id | TEXT | NOT NULL → projects.id |
| path | TEXT | NOT NULL |
| description | TEXT | NOT NULL DEFAULT '' |
| created_at | TEXT | NOT NULL |

索引：`idx_project_links_project(project_id)`

### 2.4 `sessions` —— 会话元数据（**首屏列表唯一数据源**）

初始列与既有 `ensure_column` 补齐列沿用现网 schema（id/title/workspace_path/access_mode/project_id/status/last_message_at/created_at/updated_at，及 is_temp、temp_code、temp_root、source_workspace、merged_seq、merged_pending、parent_session_id、session_type、subagent_role、subagent_task、context_token_limit、last_reported_msg_id、auto_report、trigger_tool_event_id、provider_id、model_id、dispatch_rule、image_provider_id、image_model_id、vision_provider_id、vision_model_id、decision_provider_id、decision_model_id、forked_from_session_id、forked_from_message_id、reasoning_effort、max_steps、total_tokens、prompt_tokens、completion_tokens、cached_tokens、last_run_status）。

**分库新增列**：

| 列 | 类型 | 约束 | 说明 |
|---|---|---|---|
| msg_count | INTEGER | NOT NULL DEFAULT 0 | 会话消息数（增量维护，替代 `COUNT(*)`） |
| tool_event_count | INTEGER | NOT NULL DEFAULT 0 | 工具调用数 |
| last_preview | TEXT | | 最近一条消息前 200 字符（列表预览，避免回查 session.db） |
| storage_state | TEXT | NOT NULL DEFAULT 'hot' | hot / archived / migrated |
| bundle_bytes | INTEGER | NOT NULL DEFAULT 0 | 该会话库占用字节（清理策略排序用） |
| schema_version | INTEGER | NOT NULL DEFAULT 1 | 会话库 schema 版本（升级用） |
| logical_deleted_at | TEXT | | 逻辑删除时间（物理清理由后台任务执行） |

**索引**（对齐首屏查询，走覆盖索引）：
```sql
CREATE INDEX idx_sessions_status_lastmsg ON sessions(status, last_message_at DESC, id DESC);
CREATE INDEX idx_sessions_project ON sessions(project_id, status);
CREATE INDEX idx_sessions_parent ON sessions(parent_session_id);
CREATE INDEX idx_sessions_storage_state ON sessions(storage_state);
```

首屏标准查询（目标 0.1ms 级）：
```sql
SELECT id, title, last_message_at, msg_count, last_preview, total_tokens, status
FROM sessions
WHERE status = 'active'
ORDER BY last_message_at DESC
LIMIT 50;
```

### 2.5 `secrets` —— 加密密钥（保留原样，必须留在 core）
| 列 | 类型 | 约束 |
|---|---|---|
| id | TEXT | PRIMARY KEY |
| enc | TEXT | NOT NULL（AES-GCM 密文） |

### 2.6 `mcp_servers` —— MCP 服务配置（保留原样）
| 列 | 类型 | 约束 |
|---|---|---|
| id | TEXT | PRIMARY KEY |
| name | TEXT | NOT NULL UNIQUE |
| command | TEXT | NOT NULL |
| args | TEXT | NOT NULL（JSON 数组） |
| env | TEXT | NOT NULL（JSON 对象） |
| enabled | INTEGER | NOT NULL DEFAULT 1 |
| created_at / updated_at | TEXT | NOT NULL |

### 2.7 `agent_growths` —— 成长规则沉淀（跨会话资产，必须留在 core）
| 列 | 类型 | 约束 | 说明 |
|---|---|---|---|
| id | TEXT | PRIMARY KEY | |
| project_id / session_id / message_id / run_id | TEXT | | 溯源锚点（**弱引用**，会话删除后置 NULL，不做外键） |
| trigger_type / trigger_context | TEXT | NOT NULL | |
| reflection_thought | TEXT | NOT NULL | |
| category / title / rule_content | TEXT | NOT NULL | |
| status | TEXT | NOT NULL DEFAULT 'proposed' | proposed/applied/rejected |
| applied_count | INTEGER | NOT NULL DEFAULT 0 | |
| created_at / updated_at | TEXT | NOT NULL | |

索引：`idx_growths_project(project_id,status)`、`idx_growths_session(session_id)`

### 2.8 `daily_token_stats` —— 日 token 统计（预聚合表，写消息同事务增量更新）
| 列 | 类型 | 约束 |
|---|---|---|
| date | TEXT | NOT NULL（YYYY-MM-DD） |
| session_id | TEXT | NOT NULL |
| project_id | TEXT | |
| prompt_tokens / completion_tokens / total_tokens / cached_tokens | INTEGER | NOT NULL DEFAULT 0 |

主键 `(date, session_id)`；索引：`idx_daily_tokens_date(date)`、`idx_daily_tokens_project(project_id)`

### 2.9 `schema_migrations` —— 迁移版本管理（新增）
| 列 | 类型 | 约束 |
|---|---|---|
| version | INTEGER | PRIMARY KEY |
| name | TEXT | NOT NULL |
| applied_at | TEXT | NOT NULL |

用途：记录「三库拆分」各阶段完成状态，支持断点续迁。

---

## 3. sessions/{id}/session.db（会话库）

> 建库 PRAGMA：`page_size=8192` / `journal_mode=WAL` / `synchronous=NORMAL` / `foreign_keys=ON` / `auto_vacuum=INCREMENTAL`
> 既有 schema 定义于 `src-tauri/src/session_bundle/bundle.rs::init_session_bundle_schema`。

### 3.1 `messages` —— 对话消息

既有列沿用（id, session_id, run_id, seq, role, content, reasoning, tool_calls_json, tool_call_id, queued, usage_json, created_at, prompt_tokens, completion_tokens, total_tokens, cached_tokens, is_estimated, duration_ms, turn_duration_ms, attachments_json, reverted_at, runtime_anchor）。

**分库新增列**：

| 列 | 类型 | 说明 |
|---|---|---|
| content_blob | TEXT | content 外置后的 blob 引用（相对路径） |
| content_bytes | INTEGER DEFAULT 0 | 原始字节数（判定是否外置） |
| content_sha256 | TEXT | 去重哈希 |
| is_truncated | INTEGER DEFAULT 0 | 是否被截断保护 |

索引：`idx_bundle_messages_session(session_id, seq)`、`idx_bundle_messages_run(run_id)`、`idx_messages_created(created_at)`

### 3.2 `tool_events` —— 工具调用事件（**业务态**：审批、回滚必需，只留摘要）

既有列沿用（id, message_id REFERENCES messages ON DELETE CASCADE, tool_name, tool_call_id, params_json, result_text, status, approval_scope, created_at, subprocess_id）。

**分库新增列**：

| 列 | 类型 | 说明 |
|---|---|---|
| result_blob | TEXT | result_text 外置引用 |
| result_bytes | INTEGER DEFAULT 0 | 原始字节数 |
| result_sha256 | TEXT | 去重哈希 |
| result_preview | TEXT | 前 200 字符摘要（列表直读该列） |
| truncated | INTEGER DEFAULT 0 | 是否截断 |

索引：`idx_bundle_tool_events_msg(message_id)`

> 实测依据：仅此表的字段外置即可把库体积缩到 1/380、模糊扫描 284.4ms→1.2ms。

### 3.3 `blobs` —— 外置大字段索引表（新增）
| 列 | 类型 | 约束 | 说明 |
|---|---|---|---|
| sha256 | TEXT | PRIMARY KEY | 内容哈希（天然去重） |
| rel_path | TEXT | NOT NULL | 相对 blob 根目录的路径 |
| bytes | INTEGER | NOT NULL | 原始大小 |
| encoding | TEXT | NOT NULL DEFAULT 'identity' | identity / zstd |
| ref_count | INTEGER | NOT NULL DEFAULT 1 | 引用计数（0 时可清理） |
| created_at | TEXT | NOT NULL | |

物理文件按 `blobs/ab/cd/{sha256}` 两级散列存放。

### 3.4 `runs` —— 单次运行记录（既有列沿用）
| 列 | 类型 | 约束 |
|---|---|---|
| id | TEXT | PRIMARY KEY |
| session_id | TEXT | NOT NULL |
| trigger_type | TEXT | NOT NULL DEFAULT 'manual' |
| status | TEXT | NOT NULL（running/done/cancelled/interrupted） |
| started_at | TEXT | NOT NULL |
| ended_at | TEXT | |
| trigger_message_id | TEXT | |
| duration_ms | INTEGER | DEFAULT 0 |
| total_tokens / prompt_tokens / completion_tokens / cached_tokens | INTEGER | DEFAULT 0 |

索引：`idx_runs_session(session_id, started_at DESC, id DESC)`

> **重要变更**：主库 `runs` 上同步 `sessions.last_run_status` 的触发器分库后**跨库失效**。替代：应用层在 `runs` 状态变更的同一事务内回写 `core.db.sessions.last_run_status`（单一事实源语义不变）。

### 3.5 `session_kv` —— 会话级键值（保留原样）
主键 `(session_id, key)`；列：session_id / key / value（均 TEXT NOT NULL）。

### 3.6 `session_rules_t` —— 会话级规则（保留原样）
id PK；session_id / kind / pattern / created_at；索引 `idx_bundle_session_rules(session_id)`。

### 3.7 `session_compactions` —— 上下文压缩摘要（保留原样）
id PK；session_id / start_seq / end_seq / summary_markdown / tokens_before / created_at。
> 长对话瘦身抓手：保留最近 N 轮原文，更早的压成摘要。

### 3.8 `long_tasks` —— 长任务（保留原样）
id PK；session_id / workspace_path / goal / status / current_subtask_index / subtasks_json('[]') / max_budget_tokens / total_tokens_used / current_step / max_steps(50) / created_at / updated_at。

### 3.9 `task_checkpoints` —— 任务检查点（保留原样）
id PK；task_id REFERENCES long_tasks ON DELETE CASCADE / step_number / subtask_id / status / summary / working_memory / git_commit_hash / created_at。
索引：`idx_task_checkpoints_task(task_id, step_number)`

### 3.10 `tool_file_snapshots` —— 文件改动快照（保留原样）
id PK；session_id / message_id / tool_event_id / file_path / before_hash / after_hash / is_new_file / reverted_at / created_at。
索引：`idx_bundle_snapshots_session(session_id, created_at)`
> 快照正文走 blobs 内容寻址去重，回滚按 hash 还原。

---

## 4. logs/logs-YYYYMM.db（日志库）

> 建库 PRAGMA（每月新文件创建时设定）：
> ```sql
> PRAGMA page_size = 8192;
> PRAGMA journal_mode = WAL;
> PRAGMA synchronous = NORMAL;
> PRAGMA busy_timeout = 5000;
> PRAGMA auto_vacuum = NONE;   -- 日志库不回收空间，整月文件直接删（实测结论 6）
> ```
> 特性：**只追加、不改写**（ttft/tps 回填除外），独立连接 + 单写线程 + 批量提交。

**设计决策（v2，已评审）**：不建 `llm_stream_chunks` 流式分片表。流式数据经 `chat_stream_full` 的 `on_text` 回调实时推前端，完整文本已落 `messages.content`，帧级分片无业务消费方；单次 2000 token 回复约产生 2000 行，会使日行数从万级膨胀至百万级。性能排查需求由 `llm_requests.ttft_ms` / `tokens_per_sec` 两个标量列满足。

### 4.1 `llm_requests` —— LLM 请求审计
| 列 | 类型 | 约束 | 说明 |
|---|---|---|---|
| id | TEXT | PRIMARY KEY | 入队时由调用方生成（perf 回填按 id 定位） |
| session_id | TEXT | NOT NULL | **弱引用**（不设外键） |
| message_id | TEXT | NOT NULL | 弱引用 |
| run_id | TEXT | | |
| model | TEXT | NOT NULL | |
| endpoint | TEXT | NOT NULL | |
| payload_json | TEXT | NOT NULL | ≤8KB 内联全文；>8KB 时存前 200 字符 preview |
| payload_bytes | INTEGER | NOT NULL DEFAULT 0 | 原始字节数 |
| payload_sha256 | TEXT | | 外置时非空，指向 blobs 表 |
| ttft_ms | INTEGER | | 首 token 延迟毫秒（流式首个增量到达时测定） |
| tokens_per_sec | REAL | | 生成速度 token/秒 |
| created_at | TEXT | NOT NULL | |

索引：`idx_logs_llm_session(session_id, created_at)`、`idx_logs_llm_created(created_at)`、`idx_logs_llm_message(message_id)`

> **外置替换瘦身**：旧路径的「lean payload 砍中间消息」逻辑废弃——全量 payload 进 blob（zstd 压缩后通常仅几百 KB），行内只留 preview，审计保真度不降反升。

### 4.2 `tool_call_logs` —— 工具调用出入参明细（审计态，从主库剥离）
| 列 | 类型 | 约束 | 说明 |
|---|---|---|---|
| id | TEXT | PRIMARY KEY | |
| session_id | TEXT | NOT NULL | 弱引用 |
| message_id | TEXT | NOT NULL | 弱引用 |
| tool_event_id | TEXT | NOT NULL | 弱引用 → session.db tool_events.id |
| tool_name | TEXT | NOT NULL | |
| params_preview | TEXT | | 入参前 200 字符（≤8KB 时存全文） |
| params_blob | TEXT | | 入参外置引用（sha256） |
| result_preview | TEXT | | 出参前 200 字符（≤8KB 时存全文） |
| result_blob | TEXT | | 出参外置引用（sha256） |
| bytes | INTEGER | NOT NULL DEFAULT 0 | params+result 原始字节合计 |
| status | TEXT | NOT NULL | success/failed/denied/... |
| duration_ms | INTEGER | NOT NULL DEFAULT 0 | |
| created_at | TEXT | NOT NULL | |

索引：`idx_tool_logs_session(session_id, created_at)`、`idx_tool_logs_created(created_at)`、`idx_tool_logs_name(tool_name)`

> **关键分工**：`tool_events`（会话库）= 业务态（审批、回滚必需，result_text 内联 ≤64KB）；`tool_call_logs`（日志库）= 审计态（>8KB 全量外置，硬上限 4MB，可整月丢弃）。两者解耦，删除日志不影响业务回滚。

### 4.3 `blobs` —— 日志库 blob 注册表
与 §3.3 同构（`store/blob.rs::ensure_blob_schema` 通用建表）。每月库各自维护注册表与引用计数。

### 4.4 日志 blob 生命周期（**实现决策，与全局 blobs 隔离**）

- 日志外置文件存放于 **`data/logs/blobs/ab/cd/{sha256}`**，**不与**会话库共享 `data/blobs/` 根目录。
- **理由**：TTL 删整月库文件时，该月注册表（含 ref_count）随之消失；若 blob 文件混入全局命名空间，将无法区分「日志遗留孤儿」与「会话仍在引用的 blob」，要么误删会话数据、要么永久累积孤儿。日志独立命名空间后，TTL 清理可安全执行：
  1. 删除 `logs-YYYYMM.db`（含 -wal/-shm）；
  2. GC `logs/blobs/` 下未被任何**剩余**月份库 blobs 表引用的物理文件（剩余月份数量有限，全量收集 sha 集合代价可控）。
- 跨月重复内容不去重（每月库各自内容寻址去重）；重复率低，牺牲极小，换取删月文件 100% 回收。

### 4.5 写入策略（对应实测结论 2/3）
- 内存有界队列（容量 256）+ 单写线程（std::thread，不依赖 tokio 运行时）。
- **每 200 条或 200ms 触发一次 `BEGIN IMMEDIATE ... COMMIT`**（实测 31 倍加速）。
- 所有 >8KB 字段在**落库时**（写线程内）外置到 `logs/blobs`，行内只留 preview/引用，保证每行窄。
- 调用方仅入队（带 100ms 超时），永不阻塞在文件 IO / DB IO 上；队列满则丢弃该条审计记录并告警（业务态数据不受影响）。
- 跨月自动切换新文件（按记录 `created_at` 的 `YYYYMM` 路由）。
- TTL 清理任务走同一队列串行执行，避免与写线程在 Windows 上争抢文件句柄。

---

## 5. 大字段外置与截断策略

**阈值**：单字段 > **8192 字节**（`store/blob.rs::INLINE_THRESHOLD`）触发外置。

处理顺序：
1. 计算 sha256；若 blobs 表已存在同哈希 → `ref_count += 1`，直接复用（去重）。
2. 否则 zstd（level 3）压缩；压缩无收益则 identity 原样存储。
3. 表内字段写 rel_path、bytes、sha256，并冗余 `*_preview`（前 200 字符，字符级截断保证 UTF-8 合法）。
4. 读取时先看 blob 引用 → 按需懒加载；**列表查询绝不加载 blob**，只在详情视图读取。

**截断保护**：外置后表内不再有超大字段；内联字段沿用 64KB 硬截断（`sanitize_overflow_text`）。

---

## 6. 空间回收策略（基于实测结论 5/6/7）

**放弃** `incremental_vacuum` 循环方案。

三级策略：
1. **一级：删文件**（主要手段）——归档/删除会话直接删目录，回收率 100%，O(1)。
2. **二级：删整月日志**——按 TTL 直接删除 `logs-YYYYMM.db` + 该月孤儿 blob（见 §4.4）。
3. **三级：小库 VACUUM**——仅对单库 >50MB 的会话库，在空闲时段执行一次全量 VACUUM（39MB 阻塞 933ms，需 UI 提示）。

**auto_vacuum 正确姿势**：新建库在建表前设定 `auto_vacuum=INCREMENTAL`；老库升级脚本**先执行一次 VACUUM 再设定**（实测只有这样模式才能从 0 变 2）。

---

## 7. 迁移安全性设计（对齐既有 SPEC）

1. **显式列名映射**：迁移脚本手写全列名列表，禁止 `SELECT *`（历史库列顺序与新建表不一致会字段错位）。
2. **只增不删**：先写入目标库并校验 `dst_count == src_count`，失败回滚且**绝不删除源数据**。
3. **断点续迁**：每会话迁移完成即在 `core.db.schema_migrations` 记录，可中断续跑（幂等）。
4. **启动不自作主张**：启动只做对账（`reconcile_orphan_bundles` 已有）；破坏性迁移需用户显式确认。
5. **回滚开关**：`settings` 保留 `storage.mode = single|hybrid|split`；`with_session_db` 已支持「无 bundle 时回落主库」的混合读路径。

---

## 8. 分阶段实施对照（当前进度）

| 阶段 | 内容 | 状态 |
|---|---|---|
| 阶段0 | 本文档 + `store/blob.rs`（内容寻址外置） | ✅ 完成 |
| 阶段1 | `store/logs.rs` 日志剥离（月轮转+批量+TTL）、llm_requests/tool_call_logs 切写 | ✅ 完成 |
| 阶段2 | core 收窄：`store/core.rs` sessions 增量列 + 首屏覆盖索引 + list_sessions 重写 + 启动对账 | ✅ 完成 |
| 阶段3 | 会话分库：bundle schema 扩展（blobs 表 + 外置引用列）+ 迁移引擎补强（schema_migrations 断点续迁 + 双向对账）+ 长任务路由（with_task_db）+ 触发器改应用层 | ✅ 完成 |
| 阶段4 | storage.mode 开关（single\|hybrid\|split）+ optimize_database 分库化（>50MB 会话库 VACUUM）+ 归档闭环（storage_state） | ✅ 完成 |
| 阶段5 | 前端适配（types.ts 轻量字段 + Sidebar 预览 + 存储模式设置入口）+ SPEC 文档回写 + 全量验收 | ✅ 完成 |

> 实施偏差说明：计划文件清单中的 `store/migrate_split.rs` 依硬性约束「必须复用并扩展 session_bundle/，不得另起炉灶」落位为
> `session_bundle/migrate.rs` 的补强（schema_migrations 断点记录 + tool_events 双向对账）；`migrate.rs` 的旧库迁移路径
> 仍服务于「主库残留数据 → 分库」的一次性自动迁移，两者共用同一 schema_migrations 记录。
