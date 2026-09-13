use crate::models::{ApprovalRequest, ApprovalRule};
use crate::tools::ToolCtx;
use serde_json::Value;
use std::time::Duration;
use tauri::Emitter;
use tokio::sync::oneshot;

#[derive(Clone, Debug)]
pub enum Decision {
    AllowOnce,
    AllowSession,
    AllowAlways,
    Deny(Option<String>),
}

impl Decision {
    pub fn from_str(s: &str, reason: Option<String>) -> Option<Decision> {
        match s {
            "allow_once" => Some(Decision::AllowOnce),
            "allow_session" => Some(Decision::AllowSession),
            "allow_always" => Some(Decision::AllowAlways),
            "deny" => Some(Decision::Deny(reason)),
            _ => None,
        }
    }

    pub fn scope_str(&self) -> &'static str {
        match self {
            Decision::AllowOnce => "once",
            Decision::AllowSession => "session",
            Decision::AllowAlways => "rule",
            Decision::Deny(_) => "denied",
        }
    }
}

/// 向 UI 发起审批并等待结果；超时（5 分钟）自动拒绝（无人值守安全）
pub async fn request_approval(
    app: &tauri::AppHandle,
    state: &crate::AppState,
    req: ApprovalRequest,
) -> Decision {
    let (tx, rx) = oneshot::channel();
    state
        .approvals
        .lock()
        .unwrap()
        .insert(req.event_id.clone(), crate::PendingApproval { session_id: req.session_id.clone(), tx });
    let _ = app.emit("approval:request", &req);

    match tokio::time::timeout(Duration::from_secs(300), rx).await {
        Ok(Ok(d)) => d,
        Ok(Err(_)) => Decision::Deny(None),
        Err(_) => Decision::Deny(None),
    }
}

/// 审批规则匹配
pub fn rule_matches(rule: &ApprovalRule, tool_name: &str, args: &Value, ctx: &ToolCtx) -> bool {
    match rule.kind.as_str() {
        "tool" => rule.pattern == tool_name,
        "command_prefix" => args
            .get("command")
            .and_then(|c| c.as_str())
            .map(|c| c.trim_start().starts_with(rule.pattern.as_str()))
            .unwrap_or(false),
        "path_write" => args
            .get("path")
            .and_then(|p| p.as_str())
            .map(|p| {
                let norm = |s: &str| s.trim().replace('\\', "/").trim_start_matches("./").to_string();
                norm(p).starts_with(&norm(&rule.pattern))
            })
            .unwrap_or(false),
        _ => {
            let _ = ctx;
            false
        }
    }
}

/// 根据工具调用生成可记忆的规则
pub fn rule_for(tool_name: &str, args: &Value) -> (String, String) {
    if tool_name == "run_command" {
        let prefix = args
            .get("command")
            .and_then(|c| c.as_str())
            .and_then(|c| c.split_whitespace().next())
            .unwrap_or("")
            .to_string();
        ("command_prefix".into(), prefix)
    } else if args.get("path").is_some() {
        ("path_write".into(), args["path"].as_str().unwrap_or("").to_string())
    } else {
        ("tool".into(), tool_name.to_string())
    }
}
