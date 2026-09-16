use std::path::Path;
use std::time::Duration;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// 自动探测项目技术栈及默认自检命令
/// 返回: (技术栈名称, 默认自检验证命令)
pub fn detect_project_stack(workspace: &Path) -> (String, String) {
    if !workspace.exists() || !workspace.is_dir() {
        return ("未知项目".into(), String::new());
    }

    // 0. Tauri (Rust + Frontend)
    if workspace.join("src-tauri").join("Cargo.toml").exists() && workspace.join("package.json").exists() {
        return (
            "Tauri (Rust + Frontend)".into(),
            "cargo test --manifest-path src-tauri/Cargo.toml".into(),
        );
    }

    // 1. Rust
    if workspace.join("Cargo.toml").exists() {
        return ("Rust".into(), "cargo check".into());
    }

    // 2. Node.js / TypeScript / Web
    let pkg_json = workspace.join("package.json");
    if pkg_json.exists() {
        let pm = if workspace.join("pnpm-lock.yaml").exists() {
            "pnpm"
        } else if workspace.join("yarn.lock").exists() {
            "yarn"
        } else if workspace.join("bun.lockb").exists() {
            "bun"
        } else {
            "npm"
        };

        // 读取 package.json 判断 scripts
        let mut default_cmd = format!("{pm} test");
        if let Ok(content) = std::fs::read_to_string(&pkg_json) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(scripts) = v.get("scripts").and_then(|s| s.as_object()) {
                    let has_valid_test = scripts.get("test").and_then(|t| t.as_str()).map(|s| {
                        !s.contains("no test specified") && !s.trim().is_empty()
                    }).unwrap_or(false);

                    if has_valid_test {
                        default_cmd = if pm == "npm" { "npm test".into() } else { format!("{pm} test") };
                    } else if scripts.contains_key("build") {
                        default_cmd = if pm == "npm" { "npm run build".into() } else { format!("{pm} run build") };
                    } else if scripts.contains_key("check") {
                        default_cmd = if pm == "npm" { "npm run check".into() } else { format!("{pm} run check") };
                    }
                }
            }
        }
        return ("Node.js / TypeScript".into(), default_cmd);
    }

    // 3. Go
    if workspace.join("go.mod").exists() {
        return ("Go".into(), "go test ./...".into());
    }

    // 4. Python
    if workspace.join("pyproject.toml").exists()
        || workspace.join("requirements.txt").exists()
        || workspace.join("Pipfile").exists()
        || workspace.join("setup.py").exists()
    {
        return ("Python".into(), "pytest".into());
    }

    ("通用项目".into(), String::new())
}

/// 执行交付自检命令
/// 返回: (是否通过, 命令输出内容)
pub async fn run_verify_cmd(
    workspace: &Path,
    cmd_str: &str,
    timeout: Duration,
) -> Result<(bool, String), String> {
    let cmd_trimmed = cmd_str.trim();
    if cmd_trimmed.is_empty() {
        return Ok((true, "未配置自检命令，已跳过".into()));
    }

    #[cfg(windows)]
    let mut cmd = {
        let mut c = tokio::process::Command::new("cmd");
        c.arg("/C");
        c.raw_arg(format!("chcp 65001>nul 2>nul & {cmd_trimmed}"));
        c.creation_flags(CREATE_NO_WINDOW);
        c
    };

    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = tokio::process::Command::new("sh");
        c.arg("-c").arg(cmd_trimmed);
        c
    };

    cmd.current_dir(workspace)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    let child = cmd.spawn().map_err(|e| format!("启动自检命令失败: {e}"))?;

    let output_res = tokio::time::timeout(timeout, child.wait_with_output()).await;
    match output_res {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let mut combined = String::new();
            if !stdout.trim().is_empty() {
                combined.push_str(&stdout);
            }
            if !stderr.trim().is_empty() {
                if !combined.is_empty() {
                    combined.push('\n');
                }
                combined.push_str(&stderr);
            }
            let success = output.status.success();
            let truncated = crate::models::truncate_result(&combined);
            Ok((success, truncated))
        }
        Ok(Err(e)) => Err(format!("自检命令执行错误: {e}")),
        Err(_) => Err(format!("自检命令执行超时（限制 {:?}）", timeout)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_test_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hm_sop_test_{tag}_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_detect_tauri_project() {
        let dir = temp_test_dir("tauri");
        std::fs::create_dir_all(dir.join("src-tauri")).unwrap();
        std::fs::write(dir.join("src-tauri/Cargo.toml"), "[package]\nname=\"app\"").unwrap();
        std::fs::write(dir.join("package.json"), r#"{"name": "app"}"#).unwrap();
        let (stack, cmd) = detect_project_stack(&dir);
        assert_eq!(stack, "Tauri (Rust + Frontend)");
        assert_eq!(cmd, "cargo test --manifest-path src-tauri/Cargo.toml");
    }

    #[test]
    fn test_detect_rust_project() {
        let dir = temp_test_dir("rust");
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname=\"foo\"").unwrap();
        let (stack, cmd) = detect_project_stack(&dir);
        assert_eq!(stack, "Rust");
        assert_eq!(cmd, "cargo check");
    }

    #[test]
    fn test_detect_node_project() {
        let dir = temp_test_dir("node");
        std::fs::write(
            dir.join("package.json"),
            r#"{"scripts": {"test": "jest", "build": "vite build"}}"#,
        )
        .unwrap();
        let (stack, cmd) = detect_project_stack(&dir);
        assert_eq!(stack, "Node.js / TypeScript");
        assert_eq!(cmd, "npm test");
    }

    #[test]
    fn test_detect_node_build_fallback() {
        let dir = temp_test_dir("node_fallback");
        std::fs::write(
            dir.join("package.json"),
            r#"{"scripts": {"test": "echo \"Error: no test specified\" && exit 1", "build": "tsc"}}"#,
        )
        .unwrap();
        let (stack, cmd) = detect_project_stack(&dir);
        assert_eq!(stack, "Node.js / TypeScript");
        assert_eq!(cmd, "npm run build");
    }

    #[test]
    fn test_detect_python_project() {
        let dir = temp_test_dir("py");
        std::fs::write(dir.join("requirements.txt"), "pytest").unwrap();
        let (stack, cmd) = detect_project_stack(&dir);
        assert_eq!(stack, "Python");
        assert_eq!(cmd, "pytest");
    }

    #[tokio::test]
    async fn test_run_verify_cmd_success() {
        let dir = temp_test_dir("verify");
        let cmd = "echo success";

        let (ok, out) = run_verify_cmd(&dir, cmd, Duration::from_secs(5))
            .await
            .unwrap();
        assert!(ok);
        assert!(out.contains("success"));
    }
}
