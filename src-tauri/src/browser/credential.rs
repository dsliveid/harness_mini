//! 浏览器凭据保管箱
//!
//! 解决的问题：登录流程若把密码明文写进提示词或 SOP 文档，密码会进入 LLM 上下文、
//! 落盘到会话消息表、并可能被 Git 提交。本模块提供 `${secret:KEY}` 占位符背后的
//! 加密存储，明文只在进程内存中短暂存在。
//!
//! 存储位置：`<工作区>/.harness/browser_secrets.json`
//! 加密方案：复用 `crate::secrets`（AES-256-GCM，密文带 `v1:` 前缀）

use crate::secrets;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SecretStore {
    /// key → 密文（`v1:` 前缀）
    #[serde(default)]
    pub entries: BTreeMap<String, String>,
}

pub fn secrets_path(workspace: &Path) -> PathBuf {
    workspace.join(".harness").join("browser_secrets.json")
}

fn load_store(workspace: &Path) -> Result<SecretStore, String> {
    let path = secrets_path(workspace);
    if !path.exists() {
        return Ok(SecretStore::default());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("读取 browser_secrets.json 失败: {e}"))?;
    if content.trim().is_empty() {
        return Ok(SecretStore::default());
    }
    serde_json::from_str::<SecretStore>(&content)
        .map_err(|e| format!("解析 browser_secrets.json 失败: {e}"))
}

fn save_store(workspace: &Path, store: &SecretStore) -> Result<(), String> {
    let path = secrets_path(workspace);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let json = serde_json::to_string_pretty(store)
        .map_err(|e| format!("序列化 browser_secrets.json 失败: {e}"))?;
    std::fs::write(&path, json).map_err(|e| format!("写入 browser_secrets.json 失败: {e}"))?;
    ensure_gitignore(workspace);
    Ok(())
}

/// 确保凭据文件不会被误提交
fn ensure_gitignore(workspace: &Path) {
    let gitignore = workspace.join(".gitignore");
    let existing = std::fs::read_to_string(&gitignore).unwrap_or_default();
    if existing.contains("browser_secrets.json") || existing.contains(".harness/") {
        return;
    }
    let mut content = existing;
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str("\n# Harness AI Browser Credentials\n.harness/browser_secrets.json\n");
    let _ = std::fs::write(&gitignore, content);
}

/// 按 key 读取明文密码（仅在内存中使用，调用方严禁回显）
pub fn load_secret(workspace: &Path, master_key: &[u8; 32], key: &str) -> Result<Option<String>, String> {
    let store = load_store(workspace)?;
    match store.entries.get(key) {
        Some(cipher) => {
            let plain = secrets::decrypt(master_key, cipher)?;
            Ok(Some(plain))
        }
        None => Ok(None),
    }
}

/// 写入或更新一个凭据（加密后落盘）
pub fn save_secret(
    workspace: &Path,
    master_key: &[u8; 32],
    key: &str,
    plain: &str,
) -> Result<(), String> {
    if key.trim().is_empty() {
        return Err("凭据 key 不能为空".to_string());
    }
    let mut store = load_store(workspace)?;
    let cipher = if plain.starts_with("v1:") {
        plain.to_string()
    } else {
        secrets::encrypt(master_key, plain)?
    };
    store.entries.insert(key.trim().to_string(), cipher);
    save_store(workspace, &store)
}

/// 列出已配置的凭据 key（不返回值，仅 key 名，供提示用户）
pub fn list_secret_keys(workspace: &Path) -> Vec<String> {
    load_store(workspace)
        .map(|s| s.entries.keys().cloned().collect())
        .unwrap_or_default()
}

/// 删除一个凭据
pub fn delete_secret(workspace: &Path, key: &str) -> Result<bool, String> {
    let mut store = load_store(workspace)?;
    let removed = store.entries.remove(key).is_some();
    if removed {
        save_store(workspace, &store)?;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_ws(tag: &str) -> PathBuf {
        // 注意：不能用 std::env::temp_dir()。本项目在低完整性级别下运行时
        // 无法写 TEMP / LOCALAPPDATA（os error 5 拒绝访问），故改用 crate 内相对目录。
        let base = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("target")
            .join("cred_test_tmp");
        let dir = base.join(format!("{}_{}", tag, uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_save_and_load_secret_roundtrip() {
        let ws = temp_ws("roundtrip");
        let key = [7u8; 32];

        save_secret(&ws, &key, "login_pwd", "S3cret!").unwrap();
        let got = load_secret(&ws, &key, "login_pwd").unwrap();
        assert_eq!(got, Some("S3cret!".to_string()));

        // 密文落盘，不应包含明文
        let raw = std::fs::read_to_string(secrets_path(&ws)).unwrap();
        assert!(!raw.contains("S3cret!"));
        assert!(raw.contains("v1:"));

        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn test_load_missing_returns_none() {
        let ws = temp_ws("missing");
        let key = [1u8; 32];
        assert_eq!(load_secret(&ws, &key, "nope").unwrap(), None);
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn test_list_keys_and_delete() {
        let ws = temp_ws("listkeys");
        let key = [3u8; 32];
        save_secret(&ws, &key, "a", "1").unwrap();
        save_secret(&ws, &key, "b", "2").unwrap();

        let mut keys = list_secret_keys(&ws);
        keys.sort();
        assert_eq!(keys, vec!["a".to_string(), "b".to_string()]);

        assert!(delete_secret(&ws, "a").unwrap());
        assert_eq!(list_secret_keys(&ws), vec!["b".to_string()]);

        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn test_gitignore_written() {
        let ws = temp_ws("gitignore");
        let key = [5u8; 32];
        save_secret(&ws, &key, "x", "y").unwrap();
        let gi = std::fs::read_to_string(ws.join(".gitignore")).unwrap();
        assert!(gi.contains("browser_secrets.json"));
        let _ = std::fs::remove_dir_all(&ws);
    }
}
