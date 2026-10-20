use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuthStep {
    pub order: usize,
    pub action: String, // "type" | "click" | "wait" | "press_enter" | "wait_for_url"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_encrypted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuthAction {
    pub step: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desc: Option<String>,
    pub action_type: String, // "select_modal_item" | "click" | "dismiss_notice" | "wait"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dialog_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirm_btn: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuthContractCredentials {
    pub username: String,
    pub password_cipher: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuthContractPostLogin {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<AuthAction>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success_assert_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuthContract {
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_name: Option<String>,
    pub login_url: String,
    pub credentials: AuthContractCredentials,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_login: Option<AuthContractPostLogin>,
    pub updated_at: String,
}

pub fn get_browser_profile_path(workspace: &Path) -> PathBuf {
    workspace.join(".harness").join("browser_profile")
}

pub fn get_auth_contract_path(workspace: &Path) -> PathBuf {
    workspace.join(".harness").join("auth_contract.json")
}

/// 确保工作区 .gitignore 包含登录凭据与持久化 Profile，防止密码或鉴权信息被误提交
pub fn ensure_gitignore_entries(workspace: &Path) -> Result<(), String> {
    if workspace.as_os_str().is_empty() {
        return Ok(());
    }
    let gitignore_path = workspace.join(".gitignore");
    let content = if gitignore_path.exists() {
        std::fs::read_to_string(&gitignore_path).map_err(|e| format!("读取 .gitignore 失败: {e}"))?
    } else {
        String::new()
    };

    let mut to_append = Vec::new();
    if !content.contains("browser_profile") && !content.contains(".harness/") {
        to_append.push(".harness/browser_profile/");
    }
    if !content.contains("auth_contract.json") && !content.contains(".harness/") {
        to_append.push(".harness/auth_contract.json");
    }

    if !to_append.is_empty() {
        let mut new_content = content;
        if !new_content.is_empty() && !new_content.ends_with('\n') {
            new_content.push('\n');
        }
        new_content.push_str("\n# Harness AI Browser Auth & Profile\n");
        for item in to_append {
            new_content.push_str(item);
            new_content.push('\n');
        }
        std::fs::write(&gitignore_path, new_content)
            .map_err(|e| format!("写入 .gitignore 失败: {e}"))?;
    }
    Ok(())
}

pub fn save_auth_contract(
    workspace: &Path,
    contract: &AuthContract,
    master_key: &[u8; 32],
    raw_password: Option<&str>,
) -> Result<(), String> {
    if workspace.as_os_str().is_empty() {
        return Err("工作区路径为空，无法保存 auth_contract.json".to_string());
    }
    let mut enc_contract = contract.clone();
    if let Some(plain) = raw_password {
        if !plain.starts_with("v1:") {
            let enc = crate::secrets::encrypt(master_key, plain)?;
            enc_contract.credentials.password_cipher = enc;
        }
    } else if !enc_contract.credentials.password_cipher.starts_with("v1:") {
        let enc = crate::secrets::encrypt(master_key, &enc_contract.credentials.password_cipher)?;
        enc_contract.credentials.password_cipher = enc;
    }

    let file_path = get_auth_contract_path(workspace);
    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let json = serde_json::to_string_pretty(&enc_contract).map_err(|e| format!("序列化失败: {e}"))?;
    std::fs::write(&file_path, json).map_err(|e| format!("写入 auth_contract.json 失败: {e}"))?;
    let _ = ensure_gitignore_entries(workspace);
    Ok(())
}

pub fn load_auth_contract(workspace: &Path, master_key: &[u8; 32]) -> Result<Option<AuthContract>, String> {
    if workspace.as_os_str().is_empty() {
        return Ok(None);
    }
    let file_path = get_auth_contract_path(workspace);
    if !file_path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&file_path)
        .map_err(|e| format!("读取 auth_contract.json 失败: {e}"))?;
    let mut contract = serde_json::from_str::<AuthContract>(&content)
        .map_err(|e| format!("解析 auth_contract.json 失败: {e}"))?;

    if contract.credentials.password_cipher.starts_with("v1:") {
        let plain = crate::secrets::decrypt(master_key, &contract.credentials.password_cipher)?;
        contract.credentials.password_cipher = plain;
    }

    Ok(Some(contract))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_auth_contract_encryption_roundtrip() {
        let temp_dir = std::env::temp_dir().join(format!("harness_test_contract_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let master_key = crate::secrets::ephemeral_master_key();

        let contract = AuthContract {
            version: "3.0".to_string(),
            system_name: Some("测试医疗系统".to_string()),
            login_url: "http://127.0.0.1:8080/#/login".to_string(),
            credentials: AuthContractCredentials {
                username: "admin".to_string(),
                password_cipher: "MySecretPassword123!".to_string(),
            },
            post_login: Some(AuthContractPostLogin {
                org_name: Some("测试分院".to_string()),
                actions: Some(vec![AuthAction {
                    step: 1,
                    desc: Some("选择呼吸内科".to_string()),
                    action_type: "select_modal_item".to_string(),
                    dialog_title: Some("科室选择".to_string()),
                    match_text: Some("呼吸内科".to_string()),
                    selector: None,
                    confirm_btn: Some("确定".to_string()),
                }]),
                success_assert_url: Some("#/index".to_string()),
            }),
            updated_at: Utc::now().to_rfc3339(),
        };

        // 1. 保存
        save_auth_contract(&temp_dir, &contract, &master_key, None).unwrap();

        // 验证物理文件中密码已被密文替代
        let raw = std::fs::read_to_string(get_auth_contract_path(&temp_dir)).unwrap();
        assert!(!raw.contains("MySecretPassword123!"), "密码绝不可明文写盘！");
        assert!(raw.contains("v1:"), "密文须带 v1: 前缀");

        // 2. 加载与解密
        let loaded = load_auth_contract(&temp_dir, &master_key).unwrap().expect("应成功加载契约");
        assert_eq!(loaded.credentials.username, "admin");
        assert_eq!(loaded.credentials.password_cipher, "MySecretPassword123!");
        assert_eq!(loaded.login_url, "http://127.0.0.1:8080/#/login");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
