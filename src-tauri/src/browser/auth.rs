use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
pub struct AuthMacro {
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub login_url: String,
    pub created_at: String,
    pub steps: Vec<AuthStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CookieData {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secure: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub same_site: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StorageState {
    pub updated_at: String,
    #[serde(default)]
    pub cookies: Vec<CookieData>,
    #[serde(default)]
    pub local_storage: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_storage: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthMacroStatus {
    pub has_macro: bool,
    pub has_storage_state: bool,
    pub step_count: usize,
    pub last_updated: Option<String>,
    pub login_url: Option<String>,
}

pub fn get_storage_state_path(workspace: &Path) -> PathBuf {
    workspace.join(".harness").join("storage_state.json")
}

pub fn get_auth_macro_path(workspace: &Path) -> PathBuf {
    workspace.join(".harness").join("auth_macro.json")
}

/// 确保工作区 .gitignore 包含登录宏与状态快照，防止密码或鉴权信息被误提交
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
    if !content.contains("storage_state.json") && !content.contains(".harness/") {
        to_append.push(".harness/storage_state.json");
    }
    if !content.contains("auth_macro.json") && !content.contains(".harness/") {
        to_append.push(".harness/auth_macro.json");
    }

    if !to_append.is_empty() {
        let mut new_content = content;
        if !new_content.is_empty() && !new_content.ends_with('\n') {
            new_content.push('\n');
        }
        new_content.push_str("\n# Harness AI Browser Auth & Storage State\n");
        for item in to_append {
            new_content.push_str(item);
            new_content.push('\n');
        }
        std::fs::write(&gitignore_path, new_content)
            .map_err(|e| format!("写入 .gitignore 失败: {e}"))?;
    }
    Ok(())
}

/// 存储状态快照 (Cookies + LocalStorage) 到 .harness/storage_state.json
pub fn save_storage_state(workspace: &Path, state: &StorageState) -> Result<(), String> {
    if workspace.as_os_str().is_empty() {
        return Err("工作区路径为空，无法保存 storage_state.json".to_string());
    }
    let file_path = get_storage_state_path(workspace);
    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let json = serde_json::to_string_pretty(state).map_err(|e| format!("序列化失败: {e}"))?;
    std::fs::write(&file_path, json).map_err(|e| format!("写入 storage_state.json 失败: {e}"))?;
    let _ = ensure_gitignore_entries(workspace);
    Ok(())
}

/// 读取状态快照
pub fn load_storage_state(workspace: &Path) -> Result<Option<StorageState>, String> {
    if workspace.as_os_str().is_empty() {
        return Ok(None);
    }
    let file_path = get_storage_state_path(workspace);
    if !file_path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&file_path)
        .map_err(|e| format!("读取 storage_state.json 失败: {e}"))?;
    let state = serde_json::from_str::<StorageState>(&content)
        .map_err(|e| format!("解析 storage_state.json 失败: {e}"))?;
    Ok(Some(state))
}

/// 保存登录宏到 .harness/auth_macro.json，对密码等敏感字段进行本地主密钥 AES-256-GCM 加密
pub fn save_auth_macro(
    workspace: &Path,
    macro_data: &AuthMacro,
    master_key: &[u8; 32],
) -> Result<(), String> {
    if workspace.as_os_str().is_empty() {
        return Err("工作区路径为空，无法保存 auth_macro.json".to_string());
    }
    let mut encrypted_macro = macro_data.clone();

    for step in &mut encrypted_macro.steps {
        if step.action == "type" {
            let is_pwd = step
                .selector
                .as_deref()
                .map(|s| s.contains("password"))
                .unwrap_or(false)
                || step
                    .desc
                    .as_deref()
                    .map(|d| d.contains("密码") || d.to_lowercase().contains("password"))
                    .unwrap_or(false)
                || step.is_encrypted == Some(true);

            if is_pwd {
                if let Some(ref plain) = step.value {
                    if !plain.starts_with("v1:") {
                        let enc = crate::secrets::encrypt(master_key, plain)?;
                        step.value = Some(enc);
                        step.is_encrypted = Some(true);
                    }
                }
            }
        }
    }

    let file_path = get_auth_macro_path(workspace);
    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let json = serde_json::to_string_pretty(&encrypted_macro)
        .map_err(|e| format!("序列化失败: {e}"))?;
    std::fs::write(&file_path, json).map_err(|e| format!("写入 auth_macro.json 失败: {e}"))?;
    let _ = ensure_gitignore_entries(workspace);
    Ok(())
}

/// 读取并解密登录宏，解密敏感字段供重放执行器调用
pub fn load_auth_macro(workspace: &Path, master_key: &[u8; 32]) -> Result<Option<AuthMacro>, String> {
    if workspace.as_os_str().is_empty() {
        return Ok(None);
    }
    let file_path = get_auth_macro_path(workspace);
    if !file_path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&file_path)
        .map_err(|e| format!("读取 auth_macro.json 失败: {e}"))?;
    let mut macro_data = serde_json::from_str::<AuthMacro>(&content)
        .map_err(|e| format!("解析 auth_macro.json 失败: {e}"))?;

    for step in &mut macro_data.steps {
        if step.is_encrypted == Some(true) {
            if let Some(ref enc) = step.value {
                if enc.starts_with("v1:") {
                    let plain = crate::secrets::decrypt(master_key, enc)?;
                    step.value = Some(plain);
                }
            }
        }
    }

    Ok(Some(macro_data))
}

/// 获取当前登录宏配置状态（无需解密，供前端 UI 快速读取）
pub fn get_auth_macro_status(workspace: &Path) -> AuthMacroStatus {
    let macro_path = get_auth_macro_path(workspace);
    let state_path = get_storage_state_path(workspace);

    let (has_macro, step_count, login_url) = if macro_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&macro_path) {
            if let Ok(m) = serde_json::from_str::<AuthMacro>(&content) {
                (true, m.steps.len(), Some(m.login_url))
            } else {
                (false, 0, None)
            }
        } else {
            (false, 0, None)
        }
    } else {
        (false, 0, None)
    };

    let (has_storage_state, last_updated) = if state_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&state_path) {
            if let Ok(s) = serde_json::from_str::<StorageState>(&content) {
                (true, Some(s.updated_at))
            } else {
                (true, None)
            }
        } else {
            (false, None)
        }
    } else {
        (false, None)
    };

    AuthMacroStatus {
        has_macro,
        has_storage_state,
        step_count,
        last_updated,
        login_url,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_macro_encryption_roundtrip() {
        let temp_dir = std::env::temp_dir().join(format!("harness_test_auth_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&temp_dir);

        let master_key = crate::secrets::ephemeral_master_key();

        let macro_data = AuthMacro {
            version: "1.0".to_string(),
            project: Some("test-project".to_string()),
            login_url: "http://127.0.0.1:8080/#/login".to_string(),
            created_at: Utc::now().to_rfc3339(),
            steps: vec![
                AuthStep {
                    order: 1,
                    action: "type".to_string(),
                    selector: Some("input[name='username']".to_string()),
                    value: Some("admin".to_string()),
                    is_encrypted: None,
                    desc: Some("输入用户名".to_string()),
                    ms: None,
                    exclude: None,
                },
                AuthStep {
                    order: 2,
                    action: "type".to_string(),
                    selector: Some("input[type='password']".to_string()),
                    value: Some("SuperSecret123!".to_string()),
                    is_encrypted: None,
                    desc: Some("输入密码".to_string()),
                    ms: None,
                    exclude: None,
                },
                AuthStep {
                    order: 3,
                    action: "click".to_string(),
                    selector: Some("button[type='submit']".to_string()),
                    value: None,
                    is_encrypted: None,
                    desc: Some("点击登录".to_string()),
                    ms: None,
                    exclude: None,
                },
            ],
        };

        // 1. 保存并加密
        save_auth_macro(&temp_dir, &macro_data, &master_key).unwrap();

        // 验证物理文件中密码已被密文替代
        let raw_content = std::fs::read_to_string(get_auth_macro_path(&temp_dir)).unwrap();
        assert!(!raw_content.contains("SuperSecret123!"), "密码严禁以明文形式落在磁盘上！");
        assert!(raw_content.contains("v1:"), "密文字段应带有 v1: 标识");

        // 2. 加载并解密
        let loaded = load_auth_macro(&temp_dir, &master_key).unwrap().expect("应成功加载宏");
        assert_eq!(loaded.steps.len(), 3);
        assert_eq!(loaded.steps[1].value.as_deref(), Some("SuperSecret123!"));

        // 3. 验证状态读取
        let status = get_auth_macro_status(&temp_dir);
        assert!(status.has_macro);
        assert_eq!(status.step_count, 3);
        assert_eq!(status.login_url.as_deref(), Some("http://127.0.0.1:8080/#/login"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_storage_state_roundtrip() {
        let temp_dir = std::env::temp_dir().join(format!("harness_test_state_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&temp_dir);

        let mut ls = HashMap::new();
        ls.insert("token".to_string(), "Bearer test-jwt-token".to_string());
        ls.insert("activeOrgId".to_string(), "1893893166902304769".to_string());

        let state = StorageState {
            updated_at: Utc::now().to_rfc3339(),
            cookies: vec![CookieData {
                name: "JSESSIONID".to_string(),
                value: "abc-123".to_string(),
                domain: "127.0.0.1".to_string(),
                path: "/".to_string(),
                expires: None,
                http_only: Some(true),
                secure: Some(false),
                same_site: None,
            }],
            local_storage: ls,
            session_storage: None,
        };

        save_storage_state(&temp_dir, &state).unwrap();
        let loaded = load_storage_state(&temp_dir).unwrap().expect("应成功加载状态快照");
        assert_eq!(loaded.cookies.len(), 1);
        assert_eq!(loaded.cookies[0].name, "JSESSIONID");
        assert_eq!(loaded.local_storage.get("activeOrgId").map(|s| s.as_str()), Some("1893893166902304769"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
