use super::cdp::CdpClient;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieParam {
    pub name: String,
    pub value: String,
    pub domain: Option<String>,
    pub path: Option<String>,
    pub expires: Option<f64>,
    pub http_only: Option<bool>,
    pub secure: Option<bool>,
    pub same_site: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StorageState {
    pub cookies: Vec<Value>,
    pub updated_at: String,
}

pub fn get_storage_state_path(workspace: &Path) -> PathBuf {
    workspace.join(".harness").join("storage_state.json")
}

/// 保存会话的 Cookie 与 Storage 到工作区轻量 JSON (代替庞大的实体磁盘 profile 目录)
pub async fn save_storage_state(
    cdp: &Arc<CdpClient>,
    target_session_id: &str,
    workspace: &Path,
) -> Result<(), String> {
    if workspace.as_os_str().is_empty() {
        return Ok(());
    }

    let sid = Some(target_session_id);
    let res = cdp
        .send("Network.getCookies", None, sid, Duration::from_secs(5))
        .await?;

    let cookies = res
        .get("cookies")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();

    let state = StorageState {
        cookies,
        updated_at: chrono::Utc::now().to_rfc3339(),
    };

    let target_path = get_storage_state_path(workspace);
    if let Some(parent) = target_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    let json_bytes = serde_json::to_vec_pretty(&state)
        .map_err(|e| format!("序列化 storage_state.json 失败: {e}"))?;

    tokio::fs::write(&target_path, json_bytes)
        .await
        .map_err(|e| format!("写入 storage_state.json 失败: {e}"))?;

    Ok(())
}

/// 从工作区恢复 Cookie 与 Storage 到当前隔离的 BrowserContext
pub async fn restore_storage_state(
    cdp: &Arc<CdpClient>,
    target_session_id: &str,
    workspace: &Path,
) -> Result<bool, String> {
    if workspace.as_os_str().is_empty() {
        return Ok(false);
    }

    let target_path = get_storage_state_path(workspace);
    if !target_path.exists() {
        return Ok(false);
    }

    let content = match tokio::fs::read_to_string(&target_path).await {
        Ok(c) => c,
        Err(_) => return Ok(false),
    };

    let state: StorageState = match serde_json::from_str(&content) {
        Ok(s) => s,
        Err(_) => return Ok(false),
    };

    if state.cookies.is_empty() {
        return Ok(false);
    }

    let sid = Some(target_session_id);
    let _ = cdp
        .send(
            "Network.setCookies",
            Some(json!({ "cookies": state.cookies })),
            sid,
            Duration::from_secs(5),
        )
        .await;

    Ok(true)
}
