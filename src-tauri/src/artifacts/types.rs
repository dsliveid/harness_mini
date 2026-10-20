use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 会话二进制资产元数据
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionArtifact {
    /// 唯一标识
    pub id: String,
    /// 归属的会话 ID
    pub session_id: String,
    /// 全量 SHA-256 内容寻址哈希
    pub hash: String,
    /// 语义化展示文件名 (如 shot_browser_0af81c9b.png)
    pub file_name: String,
    /// 物理高清原图绝对路径
    pub original_path: PathBuf,
    /// 物理轻量缩略图绝对路径 (用于前端长列表显存保护)
    pub thumbnail_path: Option<PathBuf>,
    /// MIME 类型 (如 image/png)
    pub mime_type: String,
    /// 原始文件字节大小
    pub byte_size: u64,
    /// 是否为受滚动配额治理的临时快照
    pub is_ephemeral: bool,
    /// 创建时间毫秒时间戳
    pub created_at: i64,
}
