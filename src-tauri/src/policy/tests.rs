use super::*;
use crate::models::ApprovalRule;
use crate::tools::Risk;
use serde_json::json;
use std::path::PathBuf;

fn test_ctx<'a>(
    ws: &'a PathBuf,
    mode: &'a str,
    rules: &'a [ApprovalRule],
) -> SecurityContext<'a> {
    SecurityContext {
        workspace_root: ws,
        sandbox_root: None,
        access_mode: mode,
        session_rules: rules,
        is_temp_workspace: false,
    }
}

#[test]
fn test_unconditional_high_danger_blocked() {
    let ws = PathBuf::from(r"D:\WorkSpace\Project\harness_go");
    let ctx = test_ctx(&ws, "full_access", &[]);

    let dangerous_cmds = [
        "git reset --hard HEAD~1",
        "git push origin main --force",
        "format C:",
        "shutdown /s /t 0",
        "mkfs.ext4 /dev/sda1",
    ];

    for cmd in dangerous_cmds {
        let args = json!({ "command": cmd });
        let tool = ToolInvocation {
            name: "run_command",
            args: &args,
            risk: Risk::Execute,
        };
        let decision = PolicyEngine::evaluate(&tool, &ctx);
        match decision {
            PolicyDecision::RequireApproval { force_once, risk, .. } => {
                assert!(force_once, "命令 {cmd} 必须强制单次审批");
                assert_eq!(risk, "execute");
            }
            other => panic!("高危命令 {cmd} 未被拦截，实际返回: {:?}", other),
        }
    }
}

#[test]
fn test_benign_commands_with_high_danger_keywords_allowed() {
    let ws = PathBuf::from(r"D:\WorkSpace\Project\harness_go");
    let ctx = test_ctx(&ws, "full_access", &[]);

    let benign_cmds = [
        r"Select-String -Path 'cmd/harness-go/main.go' -Pattern 'MaxSteps|MaxConcurrent|agent.New\(|Shutdown|Stop\('",
        r#"git commit -m "feat: graceful shutdown and reboot support""#,
        r#"echo "reboot system after update""#,
        r#"grep -rn "shutdown" ./src"#,
        r#"node -e "console.log('format c:')""#,
    ];

    for cmd in benign_cmds {
        let args = json!({ "command": cmd });
        let tool = ToolInvocation {
            name: "run_command",
            args: &args,
            risk: Risk::Execute,
        };
        let decision = PolicyEngine::evaluate(&tool, &ctx);
        match decision {
            PolicyDecision::Allow { scope } => {
                assert_eq!(scope, "mode", "安全命令【{cmd}】应在 full_access 下直接按模式放行");
            }
            other => panic!("安全命令【{cmd}】被误判拦截: {:?}", other),
        }
    }
}

#[test]
fn test_ephemeral_cache_cleanup_allowed_in_full_access() {
    let ws = PathBuf::from(r"D:\WorkSpace\Project\harness_go");
    let ctx = test_ctx(&ws, "full_access", &[]);

    // 用户在真实历史会话中触发审批的实际命令 1
    let cmd1 = "bash -lc 'T=.harness/tmp/unz; rm -rf $T; mkdir -p $T; tar -xzf dist/harness-go-0.1.0-linux-amd64.tar.gz -C $T; ls -l $T/harness-go-0.1.0-linux-amd64/'";
    let args1 = json!({ "command": cmd1 });
    let tool1 = ToolInvocation {
        name: "run_command",
        args: &args1,
        risk: Risk::Execute,
    };
    let decision1 = PolicyEngine::evaluate(&tool1, &ctx);
    assert_eq!(
        decision1,
        PolicyDecision::Allow {
            scope: "ephemeral_exempt"
        },
        "在 full_access 模式下，针对 .harness/tmp 的清理必须静默放行"
    );

    // 用户在真实历史会话中触发审批的实际命令 2
    let cmd2 = "bash -lc \"rm -rf /d/WorkSpace/Project/harness_go/.harness/tmp/mt; mkdir -p /d/WorkSpace/Project/harness_go/.harness/tmp/mt\"";
    let args2 = json!({ "command": cmd2 });
    let tool2 = ToolInvocation {
        name: "run_command",
        args: &args2,
        risk: Risk::Execute,
    };
    let decision2 = PolicyEngine::evaluate(&tool2, &ctx);
    assert_eq!(
        decision2,
        PolicyDecision::Allow {
            scope: "ephemeral_exempt"
        }
    );

    // Windows rd /s 命令
    let cmd3 = r"cmd /c rd /s /q .harness\tmp\build";
    let args3 = json!({ "command": cmd3 });
    let tool3 = ToolInvocation {
        name: "run_command",
        args: &args3,
        risk: Risk::Execute,
    };
    let decision3 = PolicyEngine::evaluate(&tool3, &ctx);
    assert_eq!(
        decision3,
        PolicyDecision::Allow {
            scope: "ephemeral_exempt"
        }
    );
}

#[test]
fn test_ephemeral_cleanup_in_confirm_mode_allows_session_memory() {
    let ws = PathBuf::from(r"D:\WorkSpace\Project\harness_go");
    let ctx = test_ctx(&ws, "confirm", &[]);

    let cmd = "bash -lc 'T=.harness/tmp/unz; rm -rf $T; mkdir -p $T'";
    let args = json!({ "command": cmd });
    let tool = ToolInvocation {
        name: "run_command",
        args: &args,
        risk: Risk::Execute,
    };
    let decision = PolicyEngine::evaluate(&tool, &ctx);

    match decision {
        PolicyDecision::RequireApproval { force_once, .. } => {
            assert!(!force_once, "针对暂存区的清理在 confirm 模式下应允许勾选「本会话允许」，不得 force_once");
        }
        other => panic!("confirm 模式下未被提示审批，实际返回: {:?}", other),
    }
}

#[test]
fn test_root_and_wildcard_deletions_are_critical() {
    let ws = PathBuf::from(r"D:\WorkSpace\Project\harness_go");
    let ctx = test_ctx(&ws, "full_access", &[]);

    let root_cmds = [
        "rm -rf /",
        "rm -rf /*",
        "rm -rf *",
        "rm -rf .",
        "rm -rf .git",
        "rm -rf .git/objects",
    ];

    for cmd in root_cmds {
        let args = json!({ "command": cmd });
        let tool = ToolInvocation {
            name: "run_command",
            args: &args,
            risk: Risk::Execute,
        };
        let decision = PolicyEngine::evaluate(&tool, &ctx);
        match decision {
            PolicyDecision::RequireApproval { force_once, risk_source, .. } => {
                assert!(force_once, "删除根目录或 .git 必须强制单次审批: {cmd}");
                assert!(risk_source.is_some());
            }
            other => panic!("根目录删除 {cmd} 居然被放行，实际返回: {:?}", other),
        }
    }
}

#[test]
fn test_file_tools_zones() {
    let ws = PathBuf::from(r"D:\WorkSpace\Project\harness_go");
    let ctx = test_ctx(&ws, "confirm", &[]);

    // 读内部代码文件 -> Allow none
    let read_args = json!({ "path": "src/main.rs" });
    let tool_read = ToolInvocation {
        name: "read_file",
        args: &read_args,
        risk: Risk::ReadOnly,
    };
    assert_eq!(
        PolicyEngine::evaluate(&tool_read, &ctx),
        PolicyDecision::Allow { scope: "none" }
    );

    // 写暂存区文件 -> Allow ephemeral_exempt
    let write_tmp_args = json!({ "path": ".harness/tmp/output.json", "content": "{}" });
    let tool_write_tmp = ToolInvocation {
        name: "write_file",
        args: &write_tmp_args,
        risk: Risk::Write,
    };
    assert_eq!(
        PolicyEngine::evaluate(&tool_write_tmp, &ctx),
        PolicyDecision::Allow { scope: "ephemeral_exempt" }
    );

    // 写业务代码文件（confirm 模式）-> RequireApproval (force_once = false)
    let write_src_args = json!({ "path": "src/main.rs", "content": "fn main() {}" });
    let tool_write_src = ToolInvocation {
        name: "write_file",
        args: &write_src_args,
        risk: Risk::Write,
    };
    match PolicyEngine::evaluate(&tool_write_src, &ctx) {
        PolicyDecision::RequireApproval { force_once, risk, .. } => {
            assert!(!force_once);
            assert_eq!(risk, "write");
        }
        other => panic!("写业务代码应在 confirm 模式下弹窗，实际返回: {:?}", other),
    }

    // 触碰 .git 目录 -> RequireApproval (force_once = true)
    let git_args = json!({ "path": ".git/config" });
    let tool_git = ToolInvocation {
        name: "read_file",
        args: &git_args,
        risk: Risk::ReadOnly,
    };
    match PolicyEngine::evaluate(&tool_git, &ctx) {
        PolicyDecision::RequireApproval { force_once, risk, .. } => {
            assert!(force_once);
            assert_eq!(risk, "path");
        }
        other => panic!(".git 目录操作必须强制审批，实际返回: {:?}", other),
    }

    // 工作区外读/搜索在 confirm 模式下 -> RequireApproval (force_once = false 允许会话记忆)
    let outside_search_args = json!({ "path": r"D:\WorkSpace\Vue\yx-medical-manager-web\src", "pattern": "login" });
    let tool_outside_search = ToolInvocation {
        name: "grep",
        args: &outside_search_args,
        risk: Risk::ReadOnly,
    };
    match PolicyEngine::evaluate(&tool_outside_search, &ctx) {
        PolicyDecision::RequireApproval { force_once, risk, risk_source, .. } => {
            assert!(!force_once, "工作区外只读查询在 confirm 模式下应允许记住放行");
            assert_eq!(risk, "read");
            assert_eq!(risk_source.as_deref(), Some("outside_workspace"));
        }
        other => panic!("工作区外检索在 confirm 模式下应提示审批: {:?}", other),
    }

    // 工作区外读/搜索在 full_access 模式下 -> 直接 Allow mode 放行，无需审批
    let full_ctx = test_ctx(&ws, "full_access", &[]);
    assert_eq!(
        PolicyEngine::evaluate(&tool_outside_search, &full_ctx),
        PolicyDecision::Allow { scope: "mode" },
        "工作区外检索在 full_access 模式下应直接放行"
    );
}

#[test]
fn test_empty_workspace_denied() {
    let empty_ws = PathBuf::new();
    let ctx = test_ctx(&empty_ws, "confirm", &[]);

    let args = json!({ "command": "dir" });
    let tool = ToolInvocation {
        name: "run_command",
        args: &args,
        risk: Risk::Execute,
    };
    match PolicyEngine::evaluate(&tool, &ctx) {
        PolicyDecision::Deny { reason } => {
            assert!(reason.contains("未绑定工作区"));
        }
        other => panic!("未绑定工作区必须 Deny，实际返回: {:?}", other),
    }
}

#[test]
fn test_kill_by_name_denied() {
    let ws = PathBuf::from(r"D:\WorkSpace\Project\harness_go");
    let ctx = test_ctx(&ws, "full_access", &[]);

    let kill_cmds = [
        "Stop-Process -Name node -Force",
        "powershell -NoProfile -Command \"Get-Process -Name harness-go -ErrorAction SilentlyContinue | Stop-Process -Force\"",
        "Get-Process -Name node | Stop-Process -Force",
        "killall -9 node",
        "pkill -f vite",
        "taskkill /IM node.exe /F",
        "taskkill -im python.exe",
        "Stop-Process -ProcessName node",
    ];

    for cmd in kill_cmds {
        let args = json!({ "command": cmd });
        let tool = ToolInvocation {
            name: "run_command",
            args: &args,
            risk: Risk::Execute,
        };
        let decision = PolicyEngine::evaluate(&tool, &ctx);
        match decision {
            PolicyDecision::Deny { reason } => {
                assert!(reason.contains("严禁使用进程名称全局通配强杀命令"), "命令 {cmd} 拦截提示不符: {reason}");
            }
            other => panic!("高危杀进程命令 {cmd} 未被 Deny，实际返回: {:?}", other),
        }
    }

    // 允许通过 PID 精准杀自身子进程
    let safe_cmds = [
        "Stop-Process -Id 1234",
        "taskkill /PID 1234 /F",
    ];
    for cmd in safe_cmds {
        let args = json!({ "command": cmd });
        let tool = ToolInvocation {
            name: "run_command",
            args: &args,
            risk: Risk::Execute,
        };
        let decision = PolicyEngine::evaluate(&tool, &ctx);
        match decision {
            PolicyDecision::Allow { .. } => {}
            other => panic!("安全 PID 命令 {cmd} 不应被阻断，实际返回: {:?}", other),
        }
    }
}
