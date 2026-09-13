//! API Key 本地加密存储（完全便携方案）。
//!
//! 主密钥保存在数据目录的 `secret.key`（首次使用时随机生成，32 字节），
//! 密文以 `v1:` + base64(nonce ‖ ciphertext) 存入 SQLite 的 secrets 表。
//! 数据目录整体拷贝（含 secret.key）即可在其它机器上完整还原 API Key。

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use rand::rngs::OsRng;
use rand::RngCore;
use std::path::Path;

const KEY_FILE: &str = "secret.key";
const NONCE_LEN: usize = 12;

/// 加载（或首次生成）数据目录主密钥
pub fn load_or_create_master_key(dir: &Path) -> Result<[u8; 32], String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("创建数据目录失败: {e}"))?;
    let path = dir.join(KEY_FILE);
    if let Ok(bytes) = std::fs::read(&path) {
        if bytes.len() == 32 {
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            return Ok(key);
        }
    }
    let key = ephemeral_master_key();
    std::fs::write(&path, key).map_err(|e| format!("写入主密钥文件失败: {e}"))?;
    Ok(key)
}

/// 一次性临时密钥：数据目录不可写时的内存哨兵模式使用，进程退出即失效
pub fn ephemeral_master_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    key
}

/// 迁移主密钥文件（更换数据目录时调用，保证 secrets 表中的密文在新目录仍可解密）
pub fn copy_key_file(from_dir: &Path, to_dir: &Path) -> Result<bool, String> {
    let src = from_dir.join(KEY_FILE);
    if !src.exists() {
        return Ok(false);
    }
    std::fs::create_dir_all(to_dir).map_err(|e| format!("创建数据目录失败: {e}"))?;
    std::fs::copy(&src, to_dir.join(KEY_FILE)).map_err(|e| format!("复制主密钥文件失败: {e}"))?;
    Ok(true)
}

pub fn encrypt(master: &[u8; 32], plain: &str) -> Result<String, String> {
    let cipher = Aes256Gcm::new_from_slice(master).map_err(|e| e.to_string())?;
    let mut nonce = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce);
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), plain.as_bytes())
        .map_err(|_| "加密失败".to_string())?;
    let mut buf = Vec::with_capacity(NONCE_LEN + ct.len());
    buf.extend_from_slice(&nonce);
    buf.extend_from_slice(&ct);
    Ok(format!("v1:{}", B64.encode(buf)))
}

pub fn decrypt(master: &[u8; 32], enc: &str) -> Result<String, String> {
    let rest = enc.strip_prefix("v1:").ok_or("密文格式无效")?;
    let buf = B64.decode(rest).map_err(|e| format!("密文解码失败: {e}"))?;
    if buf.len() <= NONCE_LEN {
        return Err("密文数据不完整".into());
    }
    let (nonce, ct) = buf.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new_from_slice(master).map_err(|e| e.to_string())?;
    let pt = cipher
        .decrypt(Nonce::from_slice(nonce), ct)
        .map_err(|_| "解密失败（主密钥不匹配或数据损坏）".to_string())?;
    String::from_utf8(pt).map_err(|_| "解密结果不是有效文本".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let master = ephemeral_master_key();
        let enc = encrypt(&master, "sk-test-12345").unwrap();
        assert!(enc.starts_with("v1:"));
        assert!(!enc.contains("sk-test"));
        assert_eq!(decrypt(&master, &enc).unwrap(), "sk-test-12345");
        // 同一明文两次加密产生不同密文（随机 nonce）
        let enc2 = encrypt(&master, "sk-test-12345").unwrap();
        assert_ne!(enc, enc2);
    }

    #[test]
    fn decrypt_rejects_wrong_key_and_garbage() {
        let master = ephemeral_master_key();
        let other = ephemeral_master_key();
        let enc = encrypt(&master, "secret").unwrap();
        assert!(decrypt(&other, &enc).is_err());
        assert!(decrypt(&master, "v1:!!!not-base64!!!").is_err());
        assert!(decrypt(&master, "no-prefix").is_err());
    }

    #[test]
    fn master_key_persists_in_dir() {
        let dir = std::env::temp_dir().join(format!("hm_secrets_test_{}", uuid::Uuid::new_v4()));
        let k1 = load_or_create_master_key(&dir).unwrap();
        let k2 = load_or_create_master_key(&dir).unwrap();
        assert_eq!(k1, k2);
        std::fs::remove_dir_all(&dir).ok();
    }
}
