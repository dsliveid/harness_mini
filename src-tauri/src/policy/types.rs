use crate::models::ApprovalRule;
use crate::tools::Risk;
use serde_json::Value;
use std::path::Path;

/// 资源安全域划分
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityZone {
    /// 核心禁区：系统盘根、操作系统关键目录、~/.ssh、.git/、工作区外部未授权路径
    Critical,
    /// 工程业务区：用户项目的源代码、资源、业务配置文件
    Workspace,
    /// 衍生暂存区：.harness/tmp/、.harness/cache/、.dev-data/temp-*、系统临时目录等可安全丢弃的沙箱空间
    Ephemeral,
}

/// Shell 命令的安全分类
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandRiskCategory {
    /// 无条件系统级高危命令（format, mkfs, dd if=, shutdown, reg delete, git reset --hard 等）
    UnconditionalHighDanger { pattern_name: &'static str },
    /// 路径相关破坏性操作（rm -rf, del /s, rd /s 等）
    PathScopedDestructive {
        /// 目标路径中最高级别的安全域
        highest_zone: SecurityZone,
        /// 解析出的目标路径描述（用于审计/预览）
        targets: Vec<String>,
    },
    /// 常规执行命令（cargo, go, npm, python, git status 等）
    GeneralExecute,
}

/// 策略引擎执行上下文
#[derive(Clone)]
pub struct SecurityContext<'a> {
    pub workspace_root: &'a Path,
    pub sandbox_root: Option<&'a Path>,
    pub access_mode: &'a str, // "full_access" | "confirm"
    pub session_rules: &'a [ApprovalRule],
    pub is_temp_workspace: bool,
}

/// 传入策略引擎的待评估工具调用
pub struct ToolInvocation<'a> {
    pub name: &'a str,
    pub args: &'a Value,
    pub risk: Risk,
}

/// 策略决策结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    /// 静默放行
    Allow {
        scope: &'static str, // "none" | "mode" | "session" | "ephemeral_exempt" | "harness_whitelist"
    },
    /// 需要人工审批
    RequireApproval {
        risk: &'static str,          // "execute" | "write" | "path"
        force_once: bool,            // 是否强制单次（禁止勾选本会话记住）
        risk_source: Option<String>, // "regex" | "destructive_critical" | "outside_workspace" 等
        preview: String,
    },
    /// 阻断拒绝（如未绑定工作区）
    Deny {
        reason: String,
    },
}
