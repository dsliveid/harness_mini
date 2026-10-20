pub mod types;

pub use types::SessionArtifact;

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub struct ArtifactManager;

impl ArtifactManager {
    /// 默认单个会话内保留的最大瞬态快照数量（超额自动按 mtime 先进先出淘汰）
    pub const DEFAULT_MAX_SNAPSHOTS: usize = 20;

    /// 存储会话临时视觉快照 (CAS 内容寻址去重 + 缩略图下采样 + 滚动配额淘汰)
    pub async fn store_screenshot(
        data_dir: &Path,
        session_id: &str,
        bytes: &[u8],
        step_tag: Option<&str>,
    ) -> Result<SessionArtifact, String> {
        if bytes.is_empty() {
            return Err("无法存储空字节图像快照".to_string());
        }

        // 1. 计算 SHA-256 内容寻址哈希
        let hash_hex = format!("{:x}", Sha256::digest(bytes));
        let short_hash = if hash_hex.len() >= 12 { &hash_hex[..12] } else { &hash_hex };

        // 2. 规范物理存储拓扑 (会话隔离域，不污染工作区)
        let artifacts_root = data_dir.join("sessions").join(session_id).join("artifacts");
        let sub_dir_name = if hash_hex.len() >= 2 { &hash_hex[..2] } else { "00" };
        let blob_dir = artifacts_root.join("blobs").join(sub_dir_name);
        let thumb_dir = artifacts_root.join("thumbs").join(sub_dir_name);

        tokio::fs::create_dir_all(&blob_dir).await.map_err(|e| format!("创建快照目录失败: {e}"))?;
        tokio::fs::create_dir_all(&thumb_dir).await.map_err(|e| format!("创建缩略图目录失败: {e}"))?;

        let blob_path = blob_dir.join(format!("{hash_hex}.png"));
        let thumb_path = thumb_dir.join(format!("{hash_hex}.png"));

        // 3. CAS 去重写入 (若物理文件已存在，跳过写盘，实现物理零冗余)
        let mut thumb_created = thumb_path.exists();
        if !blob_path.exists() {
            tokio::fs::write(&blob_path, bytes).await.map_err(|e| format!("写入快照文件失败: {e}"))?;
            if let Ok(()) = Self::generate_thumbnail(bytes, &thumb_path).await {
                thumb_created = true;
            }
        } else if !thumb_created {
            // 补充生成缺失的缩略图
            if let Ok(()) = Self::generate_thumbnail(bytes, &thumb_path).await {
                thumb_created = true;
            }
        }

        // 4. 执行滚动配额淘汰治理
        let _ = Self::apply_retention_policy(&artifacts_root, Self::DEFAULT_MAX_SNAPSHOTS).await;

        let tag = step_tag.unwrap_or("shot");
        let display_name = format!("{tag}_{short_hash}.png");

        Ok(SessionArtifact {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            hash: hash_hex,
            file_name: display_name,
            original_path: blob_path,
            thumbnail_path: if thumb_created { Some(thumb_path) } else { None },
            mime_type: "image/png".to_string(),
            byte_size: bytes.len() as u64,
            is_ephemeral: true,
            created_at: chrono::Utc::now().timestamp_millis(),
        })
    }

    /// 生成 240px 宽度的轻量缩略图（供前端长列表消费，防止 WebView 内存/显存膨胀）
    async fn generate_thumbnail(bytes: &[u8], target_path: &Path) -> Result<(), String> {
        let bytes_vec = bytes.to_vec();
        let target = target_path.to_path_buf();

        tokio::task::spawn_blocking(move || -> Result<(), String> {
            let img = image::load_from_memory(&bytes_vec).map_err(|e| format!("解析图片格式失败: {e}"))?;
            let thumb = img.thumbnail(240, 240);
            thumb.save(&target).map_err(|e| format!("保存缩略图失败: {e}"))?;
            Ok(())
        })
        .await
        .map_err(|e| format!("生成缩略图任务中断: {e}"))?
    }

    /// 执行会话级配额淘汰 (滚动清理：默认最多保留 max_retain 份临时快照)
    pub async fn apply_retention_policy(artifacts_root: &Path, max_retain: usize) -> Result<(), String> {
        let blobs_base = artifacts_root.join("blobs");
        if !blobs_base.exists() {
            return Ok(());
        }

        let mut files: Vec<(PathBuf, SystemTime)> = Vec::new();
        let mut subdirs = match tokio::fs::read_dir(&blobs_base).await {
            Ok(s) => s,
            Err(_) => return Ok(()),
        };

        while let Ok(Some(subdir)) = subdirs.next_entry().await {
            if let Ok(file_type) = subdir.file_type().await {
                if file_type.is_dir() {
                    if let Ok(mut entries) = tokio::fs::read_dir(subdir.path()).await {
                        while let Ok(Some(entry)) = entries.next_entry().await {
                            if let Ok(meta) = entry.metadata().await {
                                if meta.is_file() {
                                    let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                                    files.push((entry.path(), modified));
                                }
                            }
                        }
                    }
                }
            }
        }

        if files.len() > max_retain {
            files.sort_by_key(|k| k.1); // 按修改时间正序，最早的在前
            let excess = files.len() - max_retain;
            for (old_blob, _) in files.into_iter().take(excess) {
                let _ = tokio::fs::remove_file(&old_blob).await;
                if let Some(stem) = old_blob.file_stem().and_then(|s| s.to_str()) {
                    if stem.len() >= 2 {
                        let thumb_file = artifacts_root
                            .join("thumbs")
                            .join(&stem[..2])
                            .join(format!("{stem}.png"));
                        let _ = tokio::fs::remove_file(thumb_file).await;
                    }
                }
            }
        }

        Ok(())
    }

    /// 会话删除或归档时的联动物理清空
    pub async fn purge_session(data_dir: &Path, session_id: &str) -> Result<(), String> {
        let session_dir = data_dir.join("sessions").join(session_id);
        let session_artifacts = session_dir.join("artifacts");
        if session_artifacts.exists() {
            let _ = tokio::fs::remove_dir_all(&session_artifacts).await;
        }
        // 若会话目录下已无其他子目录或文件，一并将空目录移除
        if session_dir.exists() {
            if let Ok(mut rd) = tokio::fs::read_dir(&session_dir).await {
                if rd.next_entry().await.ok().flatten().is_none() {
                    let _ = tokio::fs::remove_dir(&session_dir).await;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 辅助构造有效的小尺寸 2x2 PNG 图像
    fn create_dummy_png(color: [u8; 4]) -> Vec<u8> {
        let mut img = image::RgbaImage::new(2, 2);
        for pixel in img.pixels_mut() {
            *pixel = image::Rgba(color);
        }
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        buf.into_inner()
    }

    #[tokio::test]
    async fn test_cas_deduplication_and_thumbnail() {
        let temp_dir = std::env::temp_dir().join(format!("harness_art_test_{}", uuid::Uuid::new_v4()));
        let session_id = "test-session-cas";

        let png_bytes = create_dummy_png([255, 0, 0, 255]);

        // 第一次写入
        let art1 = ArtifactManager::store_screenshot(&temp_dir, session_id, &png_bytes, Some("test"))
            .await
            .expect("第一次应成功写入");

        assert!(art1.original_path.exists(), "原图文件必须存在");
        assert!(art1.thumbnail_path.as_ref().unwrap().exists(), "缩略图文件必须存在");
        assert!(art1.file_name.starts_with("test_"));

        let meta1_time = tokio::fs::metadata(&art1.original_path).await.unwrap().modified().unwrap();

        // 稍微等待 10ms 确保时间戳可辨
        tokio::time::sleep(tokio::time::Duration::from_millis(15)).await;

        // 第二次写入相同字节 (CAS 去重验证)
        let art2 = ArtifactManager::store_screenshot(&temp_dir, session_id, &png_bytes, Some("test"))
            .await
            .expect("第二次应成功返回");

        assert_eq!(art1.hash, art2.hash, "哈希必须完全相同");
        assert_eq!(art1.original_path, art2.original_path, "物理路径必须完全相同");
        let meta2_time = tokio::fs::metadata(&art2.original_path).await.unwrap().modified().unwrap();
        assert_eq!(meta1_time, meta2_time, "第二次写入不应触碰物理磁盘 (CAS 零写入)");

        // 清理测试环境
        let _ = ArtifactManager::purge_session(&temp_dir, session_id).await;
        assert!(!temp_dir.join("sessions").join(session_id).exists());
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }

    #[tokio::test]
    async fn test_retention_policy_rolling_buffer() {
        let temp_dir = std::env::temp_dir().join(format!("harness_art_ret_{}", uuid::Uuid::new_v4()));
        let session_id = "test-session-retention";
        let artifacts_root = temp_dir.join("sessions").join(session_id).join("artifacts");

        // 写入 5 个不同快照
        for i in 0..5 {
            let png_bytes = create_dummy_png([i as u8, 10, 20, 255]);
            ArtifactManager::store_screenshot(&temp_dir, session_id, &png_bytes, None)
                .await
                .unwrap();
            tokio::time::sleep(tokio::time::Duration::from_millis(15)).await;
        }

        // 应用配额保留 3 份
        ArtifactManager::apply_retention_policy(&artifacts_root, 3)
            .await
            .unwrap();

        // 统计当前剩余文件数
        let mut count = 0;
        let mut subdirs = tokio::fs::read_dir(artifacts_root.join("blobs")).await.unwrap();
        while let Ok(Some(sub)) = subdirs.next_entry().await {
            if sub.file_type().await.unwrap().is_dir() {
                let mut entries = tokio::fs::read_dir(sub.path()).await.unwrap();
                while let Ok(Some(_)) = entries.next_entry().await {
                    count += 1;
                }
            }
        }

        assert_eq!(count, 3, "滚动缓冲区应将快照文件数量严格限制在 3 份");

        let _ = ArtifactManager::purge_session(&temp_dir, session_id).await;
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }
}
