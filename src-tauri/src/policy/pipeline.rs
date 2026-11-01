use super::classifier::CommandClassifier;
use super::types::{CommandRiskCategory, PolicyDecision, SecurityContext, SecurityZone, ToolInvocation};
use super::zones::ZoneResolver;
use crate::tools::Risk;
use serde_json::Value;

pub struct PolicyEngine;

impl PolicyEngine {
    /// 评估工具调用的安全合规性与执行决议
    pub fn evaluate(tool: &ToolInvocation, ctx: &SecurityContext) -> PolicyDecision {
        // 1. 未绑定工作区校验：除少数纯对话/媒体工具外，其余工具必须绑定工作区
        if ctx.workspace_root.as_os_str().is_empty()
            && tool.name != "todo"
            && tool.name != "generate_image"
            && tool.name != "recognize_image"
        {
            return PolicyDecision::Deny {
                reason: "当前会话未绑定工作区，文件与命令工具不可用。请直接以文字回答用户，并提示：如需读写文件或执行命令，可在顶栏选择工作区目录后重试。".into(),
            };
        }

        // 2. 方案/记忆/治理系统白名单（.harness 内部系统工具全量免审放行）
        if matches!(
            tool.name,
            "create_plan" | "update_plan" | "switch_plan" | "read_plan" | "record_memory"
        ) {
            return PolicyDecision::Allow {
                scope: "harness_whitelist",
            };
        }

        // 3. 临时空间写回主库（temp_merge）：不可逆且影响范围超出沙箱，任何模式均强制审批
        if tool.name == "temp_merge" {
            return PolicyDecision::RequireApproval {
                risk: "write",
                force_once: true,
                risk_source: None,
                preview: build_preview(tool.name, tool.args),
            };
        }

        // 4. 命令执行工具（run_command）决策流水线
        if tool.name == "run_command" {
            return Self::evaluate_run_command(tool.args, ctx);
        }

        // 5. 带有路径参数的工具（文件读写、目录查询、大纲解析等）
        if let Some(path_arg) = tool.args.get("path").and_then(|p| p.as_str()) {
            return Self::evaluate_path_tool(tool.name, tool.args, tool.risk, path_arg, ctx);
        }

        // 6. 其他常规工具（无路径参数的通用工具）
        Self::evaluate_generic_tool(tool.name, tool.args, tool.risk, ctx)
    }

    /// run_command 专属安全评估逻辑
    fn evaluate_run_command(args: &Value, ctx: &SecurityContext) -> PolicyDecision {
        let cmd = args.get("command").and_then(|c| c.as_str()).unwrap_or("");
        let classification = CommandClassifier::classify(cmd, ctx.workspace_root, ctx.sandbox_root);

        match classification {
            // A. 无条件全局高危命令（format、git reset --hard 等）：强制单次审批
            CommandRiskCategory::UnconditionalHighDanger { pattern_name } => {
                PolicyDecision::RequireApproval {
                    risk: "execute",
                    force_once: true,
                    risk_source: Some("regex".to_string()),
                    preview: format!("[高危操作拦截: {pattern_name}]\n{cmd}"),
                }
            }

            // B. 路径相关递归删除命令（rm -rf 等）
            CommandRiskCategory::PathScopedDestructive { highest_zone, targets } => {
                match highest_zone {
                    // 1. 完全封闭在 Ephemeral 暂存区：自治无损
                    SecurityZone::Ephemeral => {
                        if ctx.access_mode == "full_access" {
                            // 全权模式下：静默放行，彻底消除误报
                            PolicyDecision::Allow {
                                scope: "ephemeral_exempt",
                            }
                        } else if Self::matches_session_rules("run_command", args, ctx.session_rules) {
                            PolicyDecision::Allow {
                                scope: "session",
                            }
                        } else {
                            // 确认模式下：普通审批（允许勾选「本会话允许」记住放行）
                            PolicyDecision::RequireApproval {
                                risk: "execute",
                                force_once: false,
                                risk_source: None,
                                preview: format!("[暂存区清理: {}]\n{cmd}", targets.join(", ")),
                            }
                        }
                    }

                    // 2. 影响到了 Workspace 用户项目代码：在 full_access 下按模式放行，否则审核
                    SecurityZone::Workspace => {
                        if ctx.access_mode == "full_access" {
                            PolicyDecision::Allow {
                                scope: "mode",
                            }
                        } else if Self::matches_session_rules("run_command", args, ctx.session_rules) {
                            PolicyDecision::Allow {
                                scope: "session",
                            }
                        } else {
                            PolicyDecision::RequireApproval {
                                risk: "execute",
                                force_once: false,
                                risk_source: None,
                                preview: format!("[工作区代码删除: {}]\n{cmd}", targets.join(", ")),
                            }
                        }
                    }

                    // 3. 触及 Critical 核心禁区（根目录、.git、工作区外或未知目标）：强制审批
                    SecurityZone::Critical => {
                        PolicyDecision::RequireApproval {
                            risk: "execute",
                            force_once: true,
                            risk_source: Some("destructive_critical".to_string()),
                            preview: format!("[高危/越界删除: {}]\n{cmd}", targets.join(", ")),
                        }
                    }
                }
            }

            // C. 常规执行命令（构建、测试、运行、查看等）
            CommandRiskCategory::GeneralExecute => {
                if ctx.access_mode == "full_access" {
                    PolicyDecision::Allow {
                        scope: "mode",
                    }
                } else if Self::matches_session_rules("run_command", args, ctx.session_rules) {
                    PolicyDecision::Allow {
                        scope: "session",
                    }
                } else {
                    PolicyDecision::RequireApproval {
                        risk: "execute",
                        force_once: false,
                        risk_source: None,
                        preview: build_preview("run_command", args),
                    }
                }
            }
        }
    }

    /// 针对包含文件路径的工具（read_file, write_file, edit_file 等）的评估逻辑
    fn evaluate_path_tool(
        tool_name: &str,
        args: &Value,
        risk: Risk,
        path_arg: &str,
        ctx: &SecurityContext,
    ) -> PolicyDecision {
        let zone = ZoneResolver::resolve_zone(path_arg, ctx.workspace_root, ctx.sandbox_root);

        // 1. 若目标处于 Critical 核心禁区（越界或 .git）：强制单次审批
        if zone == SecurityZone::Critical {
            return PolicyDecision::RequireApproval {
                risk: "path",
                force_once: true,
                risk_source: Some("outside_workspace".to_string()),
                preview: build_preview(tool_name, args),
            };
        }

        // 2. 若目标处于 Ephemeral 暂存区：系统缓存/临时文件静默放行
        if zone == SecurityZone::Ephemeral {
            return PolicyDecision::Allow {
                scope: "ephemeral_exempt",
            };
        }

        // 3. 目标处于常规 Workspace：根据访问模式与风险级别判定
        if ctx.access_mode == "full_access" {
            return PolicyDecision::Allow {
                scope: "mode",
            };
        }

        if Self::matches_session_rules(tool_name, args, ctx.session_rules) {
            return PolicyDecision::Allow {
                scope: "session",
            };
        }

        if risk == Risk::ReadOnly {
            return PolicyDecision::Allow {
                scope: "none",
            };
        }

        // 写操作且未被规则覆盖：请求审批（允许记住）
        PolicyDecision::RequireApproval {
            risk: "write",
            force_once: false,
            risk_source: None,
            preview: build_preview(tool_name, args),
        }
    }

    /// 无路径的通用工具评估
    fn evaluate_generic_tool(
        tool_name: &str,
        args: &Value,
        risk: Risk,
        ctx: &SecurityContext,
    ) -> PolicyDecision {
        if risk == Risk::ReadOnly {
            return PolicyDecision::Allow {
                scope: "none",
            };
        }

        if ctx.access_mode == "full_access" {
            return PolicyDecision::Allow {
                scope: "mode",
            };
        }

        if Self::matches_session_rules(tool_name, args, ctx.session_rules) {
            return PolicyDecision::Allow {
                scope: "session",
            };
        }

        PolicyDecision::RequireApproval {
            risk: "write",
            force_once: false,
            risk_source: None,
            preview: build_preview(tool_name, args),
        }
    }

    /// 会话记忆放行规则匹配
    fn matches_session_rules(
        tool_name: &str,
        args: &Value,
        rules: &[crate::models::ApprovalRule],
    ) -> bool {
        rules.iter().any(|rule| match rule.kind.as_str() {
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
            _ => false,
        })
    }
}

/// 构造审批预览文本
pub fn build_preview(tool_name: &str, args: &Value) -> String {
    match tool_name {
        "run_command" => args
            .get("command")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string(),
        "temp_merge" => {
            "把临时空间的全部变更写回各项目原目录（冲突走 AI 智能合并）。\
             该操作影响你的原始目录且不可自动撤销；存在无法自动合并的冲突文件时将列出清单供人工处理。"
                .to_string()
        }
        "temp_snapshot" => {
            let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("manual");
            format!("为临时空间当前状态建立备份快照（恢复点）：{label}")
        }
        "temp_restore" => {
            let snap = args
                .get("snapshot")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            match snap {
                Some(s) => format!(
                    "把临时空间全部项目副本恢复到快照「{s}」：丢弃快照之后的全部修改（不影响原目录）。"
                ),
                None => "把临时空间全部项目副本恢复到基线：丢弃全部未提交修改（不影响原目录）。".to_string(),
            }
        }
        "write_file" => {
            let path = args.get("path").and_then(|p| p.as_str()).unwrap_or("");
            let content = args.get("content").and_then(|c| c.as_str()).unwrap_or("");
            let d = crate::diffutil::diff_lines("", content, 4000);
            format!("{path}（新增 {} 行）\n{}", d.added, d.text)
        }
        "edit_file" => {
            let path = args.get("path").and_then(|p| p.as_str()).unwrap_or("");
            let old = args.get("old_string").and_then(|c| c.as_str()).unwrap_or("");
            let new = args.get("new_string").and_then(|c| c.as_str()).unwrap_or("");
            format!("{path}\n{}", crate::diffutil::diff_lines(old, new, 4000).text)
        }
        "generate_image" => {
            let prompt = args.get("prompt").and_then(|p| p.as_str()).unwrap_or("");
            let size = args.get("size").and_then(|s| s.as_str()).unwrap_or("1024x1024");
            let filename = args.get("filename").and_then(|f| f.as_str()).unwrap_or("自动生成路径");
            format!("根据提示词生成图片：\n提示词: {prompt}\n分辨率: {size}\n目标文件: {filename}")
        }
        "recognize_image" => {
            let path = args.get("path").and_then(|p| p.as_str()).unwrap_or("");
            let prompt = args.get("prompt").and_then(|p| p.as_str()).unwrap_or("全面分析图片内容");
            format!("图像识别与分析：\n目标图片: {path}\n分析要求: {prompt}")
        }
        "create_plan" => {
            let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("未命名计划");
            let goals = args.get("goals").and_then(|v| v.as_str()).unwrap_or("");
            let arch = args.get("architecture").and_then(|v| v.as_str()).unwrap_or("");
            format!("创建任务执行计划【{}】\n目标：{}\n架构设计：{}", title, goals, arch)
        }
        "update_plan" => {
            let reason = args.get("reason").and_then(|v| v.as_str()).unwrap_or("");
            let note = args.get("revision_note").and_then(|v| v.as_str()).unwrap_or("");
            format!("更新任务计划\n调整原因：{}\n变更备注：{}", reason, note)
        }
        _ => serde_json::to_string_pretty(args).unwrap_or_default(),
    }
}
