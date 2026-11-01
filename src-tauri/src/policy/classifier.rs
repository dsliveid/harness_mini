use super::types::{CommandRiskCategory, SecurityZone};
use super::zones::ZoneResolver;
use regex::Regex;
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

static UNCONDITIONAL_RE: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
static RM_RE: OnceLock<Regex> = OnceLock::new();
static DEL_RE: OnceLock<Regex> = OnceLock::new();
static RD_RE: OnceLock<Regex> = OnceLock::new();
static VAR_ASSIGN_RE: OnceLock<Regex> = OnceLock::new();
static PS_VAR_ASSIGN_RE: OnceLock<Regex> = OnceLock::new();
static CD_RE: OnceLock<Regex> = OnceLock::new();

fn init_regexes() {
    UNCONDITIONAL_RE.get_or_init(|| {
        vec![
            ("format", Regex::new(r"(?i)\bformat\s+[a-z]:").unwrap()),
            ("mkfs", Regex::new(r"(?i)\bmkfs\b").unwrap()),
            ("dd", Regex::new(r"(?i)\bdd\s+if=").unwrap()),
            ("shutdown", Regex::new(r"(?i)\bshutdown\b").unwrap()),
            ("reboot", Regex::new(r"(?i)\breboot\b").unwrap()),
            ("reg_delete", Regex::new(r"(?i)\breg\s+delete\b").unwrap()),
            ("git_force_push", Regex::new(r"(?i)\bgit\s+push\s+.*--force\b").unwrap()),
            ("git_reset_hard", Regex::new(r"(?i)\bgit\s+reset\s+--hard\b").unwrap()),
        ]
    });

    // 匹配 rm -r / rm -rf / rm -R 等
    RM_RE.get_or_init(|| Regex::new(r"(?i)\brm\s+(-[a-z]+)*\s*-[a-z]*r[a-z]*\b").unwrap());
    // 匹配 del /s
    DEL_RE.get_or_init(|| Regex::new(r"(?i)\bdel\b.*?(?:[/\\]s\b|[/\\]s:)").unwrap());
    // 匹配 rd /s / rmdir /s 等
    RD_RE.get_or_init(|| Regex::new(r"(?i)\b(?:rd|rmdir)\b.*?(?:[/\\]s\b|[/\\]s:)").unwrap());

    // 匹配 Shell 变量赋值：T=.harness/tmp/unz 或 export T="..."，支持包裹在引号内的子脚本
    VAR_ASSIGN_RE.get_or_init(|| {
        Regex::new(r#"(?:^|[;\s&|'"])(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)=['"]?([^'"\s;&|]+)['"]?"#).unwrap()
    });

    // 匹配 PowerShell 变量赋值：$T = ".harness/tmp/unz"
    PS_VAR_ASSIGN_RE.get_or_init(|| {
        Regex::new(r#"(?:^|[;\s&|'"])\$([A-Za-z_][A-Za-z0-9_]*)\s*=\s*['"]?([^'"\s;&|]+)['"]?"#).unwrap()
    });

    // 匹配 cd 切换工作目录
    CD_RE.get_or_init(|| {
        Regex::new(r#"(?:^|[;\s&|'"])cd(?:\s+/d)?\s+['"]?([^'"\s;&|]+)['"]?"#).unwrap()
    });
}

pub struct CommandClassifier;

impl CommandClassifier {
    /// 对完整 Shell 命令进行安全分类
    pub fn classify(
        cmd: &str,
        workspace_root: &Path,
        sandbox_root: Option<&Path>,
    ) -> CommandRiskCategory {
        init_regexes();

        // 1. 检查是否存在无条件全局高危
        for (name, re) in UNCONDITIONAL_RE.get().unwrap() {
            if re.is_match(cmd) {
                return CommandRiskCategory::UnconditionalHighDanger { pattern_name: name };
            }
        }

        // 2. 检查是否存在路径相关的递归删除指令
        let has_rm = RM_RE.get().unwrap().is_match(cmd);
        let has_del = DEL_RE.get().unwrap().is_match(cmd);
        let has_rd = RD_RE.get().unwrap().is_match(cmd);

        if !has_rm && !has_del && !has_rd {
            return CommandRiskCategory::GeneralExecute;
        }

        // 3. 提取变量映射表（如 T=.harness/tmp/unz）
        let mut var_map: HashMap<String, String> = HashMap::new();
        for cap in VAR_ASSIGN_RE.get().unwrap().captures_iter(cmd) {
            if let (Some(k), Some(v)) = (cap.get(1), cap.get(2)) {
                var_map.insert(k.as_str().to_string(), v.as_str().to_string());
            }
        }
        for cap in PS_VAR_ASSIGN_RE.get().unwrap().captures_iter(cmd) {
            if let (Some(k), Some(v)) = (cap.get(1), cap.get(2)) {
                var_map.insert(k.as_str().to_string(), v.as_str().to_string());
            }
        }

        // 4. 提取当前命令中可能 cd 进入的目录
        let mut active_cd: Option<String> = None;
        for cap in CD_RE.get().unwrap().captures_iter(cmd) {
            if let Some(target) = cap.get(1) {
                let resolved_target = Self::expand_vars(target.as_str(), &var_map);
                active_cd = Some(resolved_target);
            }
        }

        // 5. 提取所有被删除的目标参数（严格限定在删除指令所在子命令内，不越过 ';'、'&&'、'||'、'|'）
        let mut targets: Vec<String> = Vec::new();

        if has_rm {
            for cap in RM_RE.get().unwrap().captures_iter(cmd) {
                let match_end = cap.get(0).unwrap().end();
                let remaining = &cmd[match_end..];
                let sub_cmd = remaining.split([';', '&', '|', '\n']).next().unwrap_or("");
                for token in sub_cmd.split_whitespace() {
                    let t = token.trim_matches(|c| c == '\'' || c == '"');
                    if t.is_empty() || t.starts_with('-') {
                        continue;
                    }
                    let expanded = Self::expand_vars(t, &var_map);
                    targets.push(expanded);
                }
            }
        }

        if has_del {
            for cap in DEL_RE.get().unwrap().captures_iter(cmd) {
                let match_end = cap.get(0).unwrap().end();
                let remaining = &cmd[match_end..];
                let sub_cmd = remaining.split([';', '&', '|', '\n']).next().unwrap_or("");
                for token in sub_cmd.split_whitespace() {
                    let t = token.trim_matches(|c| c == '\'' || c == '"');
                    if t.is_empty() || t.starts_with('/') || t.starts_with('-') {
                        continue;
                    }
                    let lower = t.to_ascii_lowercase();
                    if lower == "cmd" || lower == "powershell" || lower == "bash" || lower == "del" {
                        continue;
                    }
                    let expanded = Self::expand_vars(t, &var_map);
                    targets.push(expanded);
                }
            }
        }

        if has_rd {
            for cap in RD_RE.get().unwrap().captures_iter(cmd) {
                let match_end = cap.get(0).unwrap().end();
                let remaining = &cmd[match_end..];
                let sub_cmd = remaining.split([';', '&', '|', '\n']).next().unwrap_or("");
                for token in sub_cmd.split_whitespace() {
                    let t = token.trim_matches(|c| c == '\'' || c == '"');
                    if t.is_empty() || t.starts_with('/') || t.starts_with('-') {
                        continue;
                    }
                    let lower = t.to_ascii_lowercase();
                    if lower == "cmd" || lower == "powershell" || lower == "bash" || lower == "rd" || lower == "rmdir" {
                        continue;
                    }
                    let expanded = Self::expand_vars(t, &var_map);
                    targets.push(expanded);
                }
            }
        }

        // 若子命令内未直接跟目标，但前面已明确 cd 到了某个目录
        if targets.is_empty() {
            if let Some(cd_dir) = active_cd {
                targets.push(cd_dir);
            }
        }

        // 若依然提取不到具体目标，出于安全审慎，保守归为 Critical
        if targets.is_empty() {
            return CommandRiskCategory::PathScopedDestructive {
                highest_zone: SecurityZone::Critical,
                targets: vec!["<unresolved_targets>".to_string()],
            };
        }

        // 6. 判定所有目标的最高安全域
        let mut highest = SecurityZone::Ephemeral;
        for t in &targets {
            // 未能展开的变量引用（如 $UNKNOWN）、根通配符或特殊相对路径一律保守视为 Critical
            if t.starts_with('$') || t == "/" || t == "/*" || t == "*" || t == "~" || t == "." || t == ".." || t == "C:\\" || t == "C:/" {
                highest = SecurityZone::Critical;
                break;
            }

            let zone = ZoneResolver::resolve_zone(t, workspace_root, sandbox_root);
            match zone {
                SecurityZone::Critical => {
                    highest = SecurityZone::Critical;
                    break;
                }
                SecurityZone::Workspace => {
                    if highest != SecurityZone::Critical {
                        highest = SecurityZone::Workspace;
                    }
                }
                SecurityZone::Ephemeral => {}
            }
        }

        CommandRiskCategory::PathScopedDestructive {
            highest_zone: highest,
            targets,
        }
    }

    /// 展开字符串中的 $VAR 或 ${VAR}
    fn expand_vars(raw: &str, var_map: &HashMap<String, String>) -> String {
        let mut result = raw.to_string();
        for (k, v) in var_map {
            let pat1 = format!("${k}");
            let pat2 = format!("${{{k}}}");
            result = result.replace(&pat1, v);
            result = result.replace(&pat2, v);
        }
        result
    }
}
