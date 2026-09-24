//! CAS (Content-Addressed Storage) 文件级快照存储。
//!
//! 物理存放规范：`<DataDir>/snapshots/blobs/{hash[0..2]}/{hash[2..]}`
//! 以内容 SHA-256 哈希作为唯一键，全局内容去重。

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// 计算任意字节序列的 SHA-256 哈希十六进制字符串
pub fn compute_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// 快照 Blobs 根目录
pub fn blobs_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("snapshots").join("blobs")
}

/// 计算指定哈希值在磁盘上的物理存储路径
pub fn blob_path(data_dir: &Path, hash: &str) -> PathBuf {
    if hash.len() < 4 {
        return blobs_dir(data_dir).join(hash);
    }
    let (prefix, rest) = hash.split_at(2);
    blobs_dir(data_dir).join(prefix).join(rest)
}

/// 检查指定哈希的 Blob 是否已存在
pub fn blob_exists(data_dir: &Path, hash: &str) -> bool {
    blob_path(data_dir, hash).is_file()
}

/// 将字节序列写入 CAS 磁盘池，返回其 SHA-256 哈希
///
/// 若相同内容已存在，则跳过物理写入直接返回哈希（天然去重）。
/// 写入时采用临时文件 + 原地重命名，确保原子性，杜绝多线程写入冲突损坏。
pub fn save_blob(data_dir: &Path, bytes: &[u8]) -> Result<String, String> {
    let hash = compute_sha256(bytes);
    let target = blob_path(data_dir, &hash);
    if target.is_file() {
        return Ok(hash);
    }

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建快照目录失败: {e}"))?;
    }

    let temp_name = format!(".tmp_{}_{}", hash, uuid::Uuid::new_v4().simple());
    let temp_file = target.parent().unwrap().join(temp_name);

    std::fs::write(&temp_file, bytes).map_err(|e| format!("写入快照临时文件失败: {e}"))?;
    if let Err(e) = std::fs::rename(&temp_file, &target) {
        // 如果目标在重命名期间已由其他线程写入完成，且目标已存在，则安全忽略
        if target.is_file() {
            let _ = std::fs::remove_file(&temp_file);
        } else {
            let _ = std::fs::remove_file(&temp_file);
            return Err(format!("原子落盘快照文件失败: {e}"));
        }
    }

    Ok(hash)
}

/// 异步保存 Blob
pub async fn save_blob_async(data_dir: &Path, bytes: &[u8]) -> Result<String, String> {
    let hash = compute_sha256(bytes);
    let target = blob_path(data_dir, &hash);
    if tokio::fs::metadata(&target).await.is_ok() {
        return Ok(hash);
    }

    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("创建快照目录失败: {e}"))?;
    }

    let temp_name = format!(".tmp_{}_{}", hash, uuid::Uuid::new_v4().simple());
    let temp_file = target.parent().unwrap().join(temp_name);

    tokio::fs::write(&temp_file, bytes)
        .await
        .map_err(|e| format!("写入快照临时文件失败: {e}"))?;

    if let Err(e) = tokio::fs::rename(&temp_file, &target).await {
        if tokio::fs::metadata(&target).await.is_ok() {
            let _ = tokio::fs::remove_file(&temp_file).await;
        } else {
            let _ = tokio::fs::remove_file(&temp_file).await;
            return Err(format!("原子落盘快照文件失败: {e}"));
        }
    }

    Ok(hash)
}

/// 从 CAS 磁盘池读取指定哈希的内容
pub fn read_blob(data_dir: &Path, hash: &str) -> Result<Vec<u8>, String> {
    let target = blob_path(data_dir, hash);
    std::fs::read(&target).map_err(|e| format!("读取快照 Blob [{hash}] 失败: {e}"))
}

/// 异步读取 Blob
pub async fn read_blob_async(data_dir: &Path, hash: &str) -> Result<Vec<u8>, String> {
    let target = blob_path(data_dir, hash);
    tokio::fs::read(&target)
        .await
        .map_err(|e| format!("读取快照 Blob [{hash}] 失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_test_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hm_cas_test_{tag}_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_cas_write_and_read() {
        let dir = temp_test_dir("rw");
        let content = b"hello shadow snapshot cas storage!";
        let hash = save_blob(&dir, content).unwrap();
        assert_eq!(hash, compute_sha256(content));
        assert!(blob_exists(&dir, &hash));

        let read_back = read_blob(&dir, &hash).unwrap();
        assert_eq!(read_back, content);

        // 重复保存相同内容：天然幂等且去重
        let hash2 = save_blob(&dir, content).unwrap();
        assert_eq!(hash, hash2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_cas_async_write_and_read() {
        let dir = temp_test_dir("async_rw");
        let content = b"async test content \r\n 1234567890";
        let hash = save_blob_async(&dir, content).await.unwrap();
        assert_eq!(hash, compute_sha256(content));

        let read_back = read_blob_async(&dir, &hash).await.unwrap();
        assert_eq!(read_back, content);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
