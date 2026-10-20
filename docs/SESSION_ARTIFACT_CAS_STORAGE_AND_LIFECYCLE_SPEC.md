# 基于内容寻址(CAS)与不可变生命周期的会话多模态资产治理实施规范
# Session Artifact CAS Storage & Immutable Lifecycle Governance Specification

| 属性 | 说明 |
| :--- | :--- |
| **文档代号** | `SESSION_ARTIFACT_CAS_STORAGE_AND_LIFECYCLE_SPEC` |
| **创建时间** | 2026-10-20 |
| **适用范围** | `harness_mini`（Tauri 2 + Rust + React 桌面 AI Agent 架构体系） |
| **关联核心模块** | [`src-tauri/src/browser/mod.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/browser/mod.rs), [`src-tauri/src/agent.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/agent.rs), [`src-tauri/src/tools/skills_memory_plan.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/tools/skills_memory_plan.rs), [`src-tauri/src/session_bundle/`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/session_bundle/), [`src/components/SafeImage.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/SafeImage.tsx), [`src/components/ToolCard.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/ToolCard.tsx) |
| **知识密级** | 核心架构实施标准 / 资产子系统解耦与多模态显存治理规范 |

---

## 目录

- [一、背景与痛点深度复盘](#一背景与痛点深度复盘)
  - [1.1 现场缺陷：缩略图旧、详情新、历史全毁](#11-现场缺陷缩略图旧详情新历史全毁)
  - [1.2 数据链路复盘（以会话 e58e2876 为例）](#12-数据链路复盘以会话-e58e2876-为例)
  - [1.3 隐蔽致命缺陷：AI 多模态时序因果记忆被物理篡改](#13-隐蔽致命缺陷ai-多模态时序因果记忆被物理篡改)
- [二、架构演进哲学：拒绝堆叠式修补](#二架构演进哲学拒绝堆叠式修补)
  - [2.1 局部打补丁方案的结构性硬伤](#21-局部打补丁方案的结构性硬伤)
  - [2.2 AI Agent 资产第一性原理](#22-ai-agent-资产第一性原理)
  - [2.3 四大核心架构原则](#23-四大核心架构原则)
- [三、架构全景与拓扑规范](#三架构全景与拓扑规范)
  - [3.1 系统分层拓扑模型](#31-系统分层拓扑模型)
  - [3.2 物理目录拓扑规范（工作区零污染）](#32-物理目录拓扑规范工作区零污染)
  - [3.3 资产实体数据模型（Rust 端）](#33-资产实体数据模型rust-端)
- [四、实施方案与技术实现全解](#四实施方案与技术实现全解)
  - [4.1 阶段一：建立统一的 Session Artifact 管理子系统](#41-阶段一建立统一的-session-artifact-管理子系统)
  - [4.2 阶段二：工具层彻底解耦与调用收敛](#42-阶段二工具层彻底解耦与调用收敛)
  - [4.3 阶段三：修复模型上下文多模态因果链](#43-阶段三修复模型上下文多模态因果链)
  - [4.4 阶段四：前端分级渐进加载（LOD）与显存免疫](#44-阶段四前端分级渐进加载lod与显存免疫)
- [五、验收测试与质量守护基线](#五验收测试与质量守护基线)
- [六、收益对照矩阵](#六收益对照矩阵)

---

## 一、背景与痛点深度复盘

### 1.1 现场缺陷：缩略图旧、详情新、历史全毁
在 AI Agent 执行多模态自动化巡检（如浏览器操作、网页自愈、UI 校验）时，用户在聊天流界面中观察到以下严重反常现象：
1. **后续步骤缩略图显示旧图**：后发生的识图工具卡片，缩略图依然停留在先前的截图画面；
2. **点击详情变成最新图**：用户点击任意先前步骤的卡片缩略图试图放大排查，弹出的详情大图却全部变成了最新的第 6 步画面；
3. **历史诊断失效**：整个对话过程中的视觉演进历史被完全抹去，无法进行任何有效的视觉过程回溯。

### 1.2 数据链路复盘（以会话 e58e2876 为例）
以本地 SQLite 数据库中记录的真实对话（会话 `e58e2876-f710-4ed5-b180-47583c25c846`，医疗管理系统 UI 检查）为例：
Agent 连续执行了 6 轮 `browser_screenshot` 与 `recognize_image` 交互：

```
[步骤 1] browser_screenshot  ──> 覆写 .harness/tmp/browser_screenshot.png (详情页)
[步骤 2] recognize_image     ──> 读取 .harness/tmp/browser_screenshot.png
[步骤 3] browser_screenshot  ──> 覆写 .harness/tmp/browser_screenshot.png (列表页)
[步骤 4] recognize_image     ──> 读取 .harness/tmp/browser_screenshot.png
...
[步骤 11] browser_screenshot ──> 覆写 .harness/tmp/browser_screenshot.png (保存校验报错页)
[步骤 12] recognize_image    ──> 读取 .harness/tmp/browser_screenshot.png
```

由于底层 [`src-tauri/src/browser/mod.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/browser/mod.rs#L234-L248) 硬编码了固定单文件路径：
```rust
let rel_path = ".harness/tmp/browser_screenshot.png";
let target_file = ctx.workspace.join(rel_path);
let _ = tokio::fs::write(&target_file, bytes).await;
```
* **前端缩略图展示旧图**：`SafeImage` 接收的路径字符串完全相同，Tauri 协议 URL（`asset://localhost/...`）未发生变化，命中 WebView 内存解码强缓存；
* **点击详情变成最新图**：弹窗重新发起物理文件请求，而此时磁盘上的物理文件已被最后一步彻底覆盖。

### 1.3 隐蔽致命缺陷：AI 多模态时序因果记忆被物理篡改
在深入代码审查 [`src-tauri/src/agent.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/agent.rs#L3708-L3720) 时发现了一个更为严重的底层缺陷：
```rust
// agent.rs: 遍历消息历史构造多模态 LLM Prompt
if let Some(ref att_list) = atts {
    for att in att_list {
        if att.is_image {
            if let Ok(bytes) = std::fs::read(&att.path) {
                // 再次从磁盘读取该物理文件并编码为 Base64 传入上下文
```
当 Agent 推进到第 6 步向大模型发起后续提示词时，系统会重新读取历史上所有 tool 消息绑定的图片附件路径。
由于所有历史附件的 `att.path` 都指向 `.harness/tmp/browser_screenshot.png`，**大模型在第 6 轮思考时，读取到的步骤 1、步骤 3、步骤 5 的历史图片全部被物理篡改成了第 6 步的画面**！
模型的视觉时序因果记忆彻底紊乱，严重损害了 Agent 的自愈与逻辑自洽能力。

---

## 二、架构演进哲学：拒绝堆叠式修补

### 2.1 局部打补丁方案的结构性硬伤
若仅在 `browser/mod.rs` 内部增加时间戳、建立 15 张图的目录轮转、再拷贝一份 `latest.png`，属于典型的“堆叠式补丁”，具有三大架构缺陷：
1. **模块职责倒错**：浏览器驱动模块不仅负责 CDP 通信，还要维护文件滚动清理队列；
2. **工作区污染**：海量临时调试文件塞在用户工程的 `.harness/tmp/` 下，干扰代码监视与 Git 状态；
3. **不可复用**：后续新增任何多模态工具（桌面截图、录屏切片），均需将这套轮转逻辑重写一遍。

### 2.2 AI Agent 资产第一性原理
在现代 AI Agent 架构中，系统产物应清晰划分为三类：
1. **Workspace Files（工作区源代码与文档）**：用户代码资产，归属于 Git 追踪；
2. **Relational Meta（关系型时序元数据）**：保存在 SQLite 中的会话、消息、Token 消耗；
3. **Session Artifacts（会话二进制对象资产）**：截图快照、生成图片、上传附件等。它们具有**不可变（Immutable）**、**内容寻址（Content-Addressable）**、**生命周期受会话约束（Session-Scoped）**的天然属性。

### 2.3 四大核心架构原则
* **原则一：工作区零污染（Zero Workspace Pollution）**：所有临时视觉资产全部收敛入数据根目录，用户工程目录内不留任何临时图片；
* **原则二：内容寻址去重（Content-Addressable Storage, CAS）**：以内容哈希（SHA-256）作为物理存储与寻址凭证。连续截图若画面未变，物理写入为 0，磁盘与内存增量为 0；
* **原则三：不可变值语义（Immutable Value Semantics）**：生成后即刻不可变，从根源上消除缓存击穿与数据不一致；
* **原则四：分级渲染与显存免疫（LOD & Viewport Decoupling）**：前端卡片仅消费低分辨率轻量缩略流，详情按需加载，彻底解耦长列表与显存消耗。

---

## 三、架构全景与拓扑规范

### 3.1 系统分层拓扑模型

```
┌─────────────────────────────────────────────────────────────┐
│                    Tools & Perception Layer                 │
│   browser_screenshot   generate_image   desktop_shot(未来)  │
└──────────────────────────────┬──────────────────────────────┘
                               │ 提交原始 bytes / ext
                               ▼
┌─────────────────────────────────────────────────────────────┐
│              Unified Session Artifact Manager               │
│  - SHA-256 CAS 计算 & 去重索引                              │
│  - 会话配额与 FIFO/LRU 滚动淘汰机制                         │
│  - 缩略图渐进式下采样衍生管道 (Thumbnail Pipeline)          │
└──────────────────────────────┬──────────────────────────────┘
                               │
            ┌──────────────────┴──────────────────┐
            ▼                                     ▼
┌──────────────────────────────┐    ┌─────────────────────────┐
│     Physical Storage Pool    │    │   Consumers Subsystem   │
│ <DataDir>/sessions/{sid}/    │    │ 1. LLM Prompt Context   │
│ └── artifacts/               │    │    (精准时序不可变回放)  │
│     ├── blobs/               │    │ 2. Frontend Chat Stream │
│     └── thumbs/              │    │    (LOD 安全缩略图)     │
└──────────────────────────────┘    └─────────────────────────┘
```

### 3.2 物理目录拓扑规范（工作区零污染）
所有会话资产统一落盘于用户数据目录中，与项目工程代码绝对隔离：

```
<DataDir>/
├── harness_mini.db
└── sessions/
    └── {session_id}/
        └── artifacts/
            ├── blobs/
            │   ├── 0a/
            │   │   └── 0af81c9b4e2d...png     <-- 高清不可变快照 (CAS 去重)
            │   └── f3/
            │       └── f3e110b98a1c...png
            └── thumbs/
                ├── 0a/
                │   └── 0af81c9b4e2d.webp      <-- 200px 超轻缩略图 (供长列表消费)
                └── f3/
                    └── f3e110b98a1c.webp
```

### 3.3 资产实体数据模型（Rust 端）

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionArtifact {
    pub id: String,                 // 唯一标识 (UUID)
    pub session_id: String,         // 关联的会话 ID
    pub hash: String,               // SHA-256 全量指纹
    pub file_name: String,          // 语义化展示名 (如 shot_step2_0af81c9b.png)
    pub original_path: PathBuf,     // 物理原图绝对路径
    pub thumbnail_path: PathBuf,    // 物理缩略图绝对路径
    pub mime_type: String,          // image/png, image/webp 等
    pub byte_size: u64,             // 字节数
    pub is_ephemeral: bool,         // 是否为受滚动淘汰保护的临时快照
    pub created_at: i64,            // 创建时间戳 (毫秒)
}
```

---

## 四、实施方案与技术实现全解

### 4.1 阶段一：建立统一的 Session Artifact 管理子系统

在 `src-tauri/src/artifacts/` 下实现核心基础设施，对外暴露极简纯粹接口。

#### 1. 核心接口定义 (`src-tauri/src/artifacts/mod.rs`)

```rust
pub struct ArtifactManager;

impl ArtifactManager {
    /// 存储会话临时视觉快照 (CAS 校验 + 缩略图下采样 + 滚动缓冲区治理)
    pub async fn store_screenshot(
        data_dir: &Path,
        session_id: &str,
        bytes: &[u8],
        step_tag: Option<&str>,
    ) -> Result<SessionArtifact, String> {
        // 1. 计算 SHA-256 指纹
        use sha2::{Digest, Sha256};
        let hash_hex = format!("{:x}", Sha256::digest(bytes));
        let short_hash = &hash_hex[..12];

        // 2. 确定物理存储路径
        let artifacts_root = data_dir.join("sessions").join(session_id).join("artifacts");
        let blob_dir = artifacts_root.join("blobs").join(&hash_hex[..2]);
        let thumb_dir = artifacts_root.join("thumbs").join(&hash_hex[..2]);
        tokio::fs::create_dir_all(&blob_dir).await.map_err(|e| e.to_string())?;
        tokio::fs::create_dir_all(&thumb_dir).await.map_err(|e| e.to_string())?;

        let blob_path = blob_dir.join(format!("{hash_hex}.png"));
        let thumb_path = thumb_dir.join(format!("{hash_hex}.webp"));

        // 3. CAS 去重写入 (若物理文件已存在，跳过写磁盘，实现零损耗)
        if !blob_path.exists() {
            tokio::fs::write(&blob_path, bytes).await.map_err(|e| e.to_string())?;
            // 生成轻量 WebP 缩略图 (最大边长 240px，高画质低显存)
            Self::generate_thumbnail(bytes, &thumb_path).await.ok();
        }

        // 4. 执行会话级配额淘汰 (滚动清理：默认最多保留 20 份临时快照)
        Self::apply_retention_policy(&artifacts_root, 20).await.ok();

        let tag = step_tag.unwrap_or("shot");
        let display_name = format!("{tag}_{short_hash}.png");

        Ok(SessionArtifact {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            hash: hash_hex,
            file_name: display_name,
            original_path: blob_path,
            thumbnail_path: thumb_path,
            mime_type: "image/png".to_string(),
            byte_size: bytes.len() as u64,
            is_ephemeral: true,
            created_at: chrono::Utc::now().timestamp_millis(),
        })
    }

    /// 会话删除或归档时的联动物理清空
    pub async fn purge_session(data_dir: &Path, session_id: &str) -> Result<(), String> {
        let session_artifacts = data_dir.join("sessions").join(session_id).join("artifacts");
        if session_artifacts.exists() {
            let _ = tokio::fs::remove_dir_all(session_artifacts).await;
        }
        Ok(())
    }
}
```

#### 2. 滚动淘汰引擎实现（FIFO 配额约束）

```rust
impl ArtifactManager {
    async fn apply_retention_policy(artifacts_root: &Path, max_retain: usize) -> Result<(), String> {
        let blobs_base = artifacts_root.join("blobs");
        if !blobs_base.exists() { return Ok(()); }

        // 扫描该会话下所有 blob 快照
        let mut files = Vec::new();
        let mut subdirs = tokio::fs::read_dir(&blobs_base).await.map_err(|e| e.to_string())?;
        while let Ok(Some(subdir)) = subdirs.next_entry().await {
            if subdir.path().is_dir() {
                let mut entries = tokio::fs::read_dir(subdir.path()).await.map_err(|e| e.to_string())?;
                while let Ok(Some(entry)) = entries.next_entry().await {
                    if let Ok(meta) = entry.metadata().await {
                        if meta.is_file() {
                            files.push((entry.path(), meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH)));
                        }
                    }
                }
            }
        }

        // 超过配额时，淘汰最早创建的文件及其对应的缩略图
        if files.len() > max_retain {
            files.sort_by_key(|k| k.1); // 按时间正序
            let excess_count = files.len() - max_retain;
            for (old_blob, _) in files.iter().take(excess_count) {
                // 删除原始图
                let _ = tokio::fs::remove_file(old_blob).await;
                // 联动删除对应缩略图
                if let Some(stem) = old_blob.file_stem().and_then(|s| s.to_str()) {
                    if stem.len() >= 2 {
                        let thumb_file = artifacts_root.join("thumbs").join(&stem[..2]).join(format!("{stem}.webp"));
                        let _ = tokio::fs::remove_file(thumb_file).await;
                    }
                }
            }
        }
        Ok(())
    }
}
```

---

### 4.2 阶段二：工具层彻底解耦与调用收敛

#### 改造 [`src-tauri/src/browser/mod.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/browser/mod.rs) 中的 `browser_screenshot`
彻底删除硬编码的文件操作，模块代码量锐减，职责回归纯粹：

```rust
pub async fn browser_screenshot(ctx: &ToolCtx) -> Result<String, String> {
    let session_key = get_session_key(ctx);
    let manager = global_browser_manager();
    let session = manager
        .get(&session_key)
        .await
        .ok_or_else(|| "未找到活动的浏览器调试会话，请先调用 browser_navigate 打开网页".to_string())?;

    let b64 = session.capture_screenshot().await.map_err(|e| e.to_string())?;
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(&b64)
        .map_err(|e| format!("Base64 解码失败: {e}"))?;

    let host = ctx.host.as_ref().ok_or("内部错误：缺少宿主上下文")?;
    let state = host.app.state::<crate::AppState>();
    let data_dir = state.data_dir.lock().unwrap().clone().unwrap_or_else(|| state.default_data_dir.clone());

    // 一行委托给资产中台：写入 CAS 存储，完成去重与配额治理
    let artifact = crate::artifacts::ArtifactManager::store_screenshot(
        &data_dir,
        &session_key,
        &bytes,
        Some("browser"),
    ).await?;

    let path_str = artifact.original_path.to_string_lossy().replace('\\', "/");
    let thumb_str = artifact.thumbnail_path.to_string_lossy().replace('\\', "/");
    let approx_kb = bytes.len() / 1024;

    Ok(format!(
        "### 浏览器页面屏幕快照已成功捕获\n\
         - **资产快照路径**: `{path_str}`\n\
         - **缩略图路径**: `{thumb_str}`\n\
         - **指纹指纹**: `{}`\n\
         - **图像体积**: ~{approx_kb} KB\n\n\
         > **注意**：请优先依据 DOM 与语义断言；确需排查视觉效果时可调用 `recognize_image(path=\"{path_str}\")`。",
        artifact.hash
    ))
}
```

---

### 4.3 阶段三：修复模型上下文多模态因果链

修改 [`src-tauri/src/agent.rs`](file:///f:/WorkSpace/Other/harness_mini/src-tauri/src/agent.rs) 中的多模态注入逻辑：
由于每一步截图在 `recognize_image` 结果中输出的均为基于 CAS 内容指纹的独立不可变路径，写入 `tool_attachments` 的 `att.path` 自然也是唯一不可变文件。

```rust
// agent.rs: 注入 tool_attachments 时自动记录物理快照路径
let tool_attachments = if tc.name == "recognize_image" && status == "success" {
    // 自动捕获该快照当时的真实独立物理文件，不再存在覆盖篡改问题
```

在后续执行时，大模型在上下文回放历史消息时读取各历史附件，**100% 精确对应各阶段当时的页面状态**。时序因果链完全恢复。

---

### 4.4 阶段四：前端分级渐进加载（LOD）与显存免疫

#### 1. 工具卡片接入轻量缩略图（[`src/components/ToolCard.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/ToolCard.tsx)）
在 `VisionRecognizeView` 中：
* 缩略图优先渲染由资产中心预生成的 `thumbnail_path`（200px WebP，仅 5KB 左右）；
* 用户点击放大时，向 `ImageLightboxModal` 传递 `original_path` 渲染高清原图；
* 启用原生异步解码与惰性渲染：

```tsx
<div
  className="relative group cursor-zoom-in shrink-0 w-24 h-24 rounded-md overflow-hidden bg-black/50 border border-edge/60 flex items-center justify-center"
  onClick={() => setLightboxImage({ src: displayImagePath, title: prompt || "目标图片" })}
>
  <SafeImage
    src={thumbnailPath || displayImagePath}
    alt="目标图片"
    loading="lazy"
    decoding="async"
    className="w-full h-full object-cover group-hover:scale-105 transition-transform duration-200"
  />
  <div className="absolute inset-0 bg-black/40 opacity-0 group-hover:opacity-100 transition-opacity flex items-center justify-center">
    <ZoomIn size={16} className="text-white" />
  </div>
</div>
```

#### 2. WebView 显存与淘汰保护（[`src/components/SafeImage.tsx`](file:///f:/WorkSpace/Other/harness_mini/src/components/SafeImage.tsx)）
在 `SafeImage` 中为已淘汰的历史快照提供优雅回退，避免破图与控制台报错：

```tsx
if (hasError) {
  return (
    <div className="w-full h-full flex flex-col items-center justify-center bg-panel3/60 text-inkdim/60 text-[10px] p-2 text-center select-none border border-edge/40 rounded-md">
      <Clock size={16} className="opacity-60 mb-1" />
      <span>快照已按配额释放</span>
    </div>
  );
}
```

---

## 五、验收测试与质量守护基线

| 序号 | 验证场景 | 预期判定基线 |
| :---: | :--- | :--- |
| **TC-1** | 连续 10 轮截图并执行识图 | 对话流中 10 张卡片的缩略图清晰反映各步骤演进，**互不相同，永无串图**。 |
| **TC-2** | 点击历史卡片查看详情 | 点击第 1 张卡片，Lightbox 弹出的 100% 为第 1 步详情大图，**绝不显示后续步骤画面**。 |
| **TC-3** | 连续无变化截图（CAS 去重） | 页面处于相同状态连续截屏 5 次，底层 `blobs/` 物理文件数维持为 1，**零冗余写入**。 |
| **TC-4** | 会话滚动配额限制 | 连续执行 50 次截屏，底层快照物理文件总数始终维持在设定阈值（20 份），**磁盘与内存严格受控**。 |
| **TC-5** | 会话清理与工程隔离 | 检查用户工程工作区，`.harness/tmp/` 无任何游离临时图片；删除会话后，数据目录对应资产被彻底擦除。 |
| **TC-6** | 多模态模型上下文一致性 | 在第 10 步断点检查注入 LLM 的上下文 Payload，历史所有 steps 的图像 Base64 均与当时画面严格匹配。 |

---

## 六、收益对照矩阵

| 衡量维度 | 原状（单文件覆盖） | 局部堆叠补丁 | 推荐方案（统一 CAS 资产中心） |
| :--- | :--- | :--- | :--- |
| **缩略图与详情一致性** | ❌ 错乱（缩略图旧、详情新） | ⚠️ 依赖 URL 参数破缓存 | ✅ **数学级保证一致，不可变语义** |
| **模型多模态时序因果** | ❌ 严重破坏（历史被覆盖篡改） | ⚠️ 依赖手动维护路径 | ✅ **绝对精确还原时序上下文** |
| **磁盘占用增长** | 恒定 100KB（但数据全失） | ❌ 散落于工作区，无限膨胀 | ✅ **CAS 零冗余去重 + 滚动配额约束** |
| **前端 WebView 显存** | 易因缓存引起卡顿 | ❌ 1080p 大图常驻，易 OOM | ✅ **LOD 缩略图下采样 + 视口安全** |
| **工作区清洁度** | ⚠️ 临时图片污染工作区 | ❌ 堆叠在工作区 `.harness/tmp/` | ✅ **工作区零污染，集中于数据域** |
| **代码可维护性** | 耦合在 `browser/mod.rs` | ❌ 职责蔓延，难以横向复用 | ✅ **高内聚中台抽象，即插即用** |
