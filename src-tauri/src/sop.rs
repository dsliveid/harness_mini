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
            "cargo check --manifest-path src-tauri/Cargo.toml".into(),
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

/// 判断路径中某组件是否属于非代码/纯文档目录（相对工作区而言）
fn is_non_code_dir_component(comp: &str) -> bool {
    matches!(
        comp,
        "docs" | "doc" | "documentation" | ".harness" | ".vscode" | ".idea" | ".git" | ".github" | "scratch"
    )
}

/// 判断文件名或扩展名是否属于纯文档、图片、纯数据或 VCS 忽略配置等非代码资源
fn is_non_code_file(file_name_lower: &str, ext_lower: &str) -> bool {
    // 1. 忽略或元数据文件名（精确匹配或前缀匹配）
    if matches!(
        file_name_lower,
        ".gitignore"
            | ".gitattributes"
            | ".editorconfig"
            | ".prettierignore"
            | ".eslintignore"
            | ".npmignore"
            | "license"
            | "licence"
            | "notice"
            | "changelog"
            | "contributing"
    ) {
        return true;
    }
    if file_name_lower.starts_with("license.")
        || file_name_lower.starts_with("licence.")
        || file_name_lower.starts_with("notice.")
        || file_name_lower.starts_with("readme")
    {
        return true;
    }

    // 2. 纯文档、媒体、画图与数据文件扩展名
    matches!(
        ext_lower,
        "md" | "markdown"
            | "mdown"
            | "mkd"
            | "txt"
            | "rtf"
            | "pdf"
            | "doc"
            | "docx"
            | "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "svg"
            | "ico"
            | "webp"
            | "bmp"
            | "tiff"
            | "mp4"
            | "mp3"
            | "wav"
            | "drawio"
            | "excalidraw"
            | "fig"
            | "puml"
            | "csv"
            | "tsv"
            | "log"
    )
}

/// 判断文件名是否属于核心构建/清单/依赖配置文件
fn is_build_manifest_filename(file_name_lower: &str) -> bool {
    if matches!(
        file_name_lower,
        "cargo.toml"
            | "cargo.lock"
            | "package.json"
            | "package-lock.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
            | "bun.lockb"
            | "turbo.json"
            | "makefile"
            | "cmakelists.txt"
            | "dockerfile"
            | "docker-compose.yml"
            | "docker-compose.yaml"
            | "pyproject.toml"
            | "requirements.txt"
            | "setup.py"
            | "setup.cfg"
            | "pipfile"
            | "pipfile.lock"
            | "go.mod"
            | "go.sum"
            | "pom.xml"
            | "build.gradle"
            | "build.gradle.kts"
            | "settings.gradle"
    ) {
        return true;
    }

    // tsconfig*.json (如 tsconfig.json, tsconfig.node.json, tsconfig.app.json)
    if file_name_lower.starts_with("tsconfig") && file_name_lower.ends_with(".json") {
        return true;
    }

    // tauri*.conf.json (如 tauri.conf.json)
    if file_name_lower.starts_with("tauri") && file_name_lower.ends_with(".json") {
        return true;
    }

    // 前端常见构建配置: vite.config.*, webpack.config.*, rollup.config.*, tailwind.config.*
    if file_name_lower.starts_with("vite.config.")
        || file_name_lower.starts_with("webpack.config.")
        || file_name_lower.starts_with("rollup.config.")
        || file_name_lower.starts_with("postcss.config.")
        || file_name_lower.starts_with("tailwind.config.")
    {
        return true;
    }

    false
}

/// 判断文件扩展名是否属于源代码程序语言
fn is_source_code_extension(ext_lower: &str) -> bool {
    matches!(
        ext_lower,
        "rs" | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "mjs"
            | "cjs"
            | "vue"
            | "svelte"
            | "py"
            | "pyw"
            | "go"
            | "c"
            | "cpp"
            | "cc"
            | "cxx"
            | "h"
            | "hpp"
            | "hh"
            | "cs"
            | "java"
            | "kt"
            | "kts"
            | "scala"
            | "swift"
            | "rb"
            | "php"
            | "sql"
            | "sh"
            | "bash"
            | "zsh"
            | "bat"
            | "cmd"
            | "ps1"
            | "html"
            | "htm"
            | "css"
            | "scss"
            | "sass"
            | "less"
            | "wasm"
            | "proto"
    )
}

/// 判断给定文件路径是否属于代码源文件或关键构建配置文件。
/// 可传入工作区根路径（可选），以进行精准的相对路径目录过滤。
pub fn is_code_or_build_file_with_workspace(path: &Path, workspace: Option<&Path>) -> bool {
    // 1. 获取相对工作区路径（如果是绝对路径且位于工作区内）
    let rel_path = if let Some(ws) = workspace {
        path.strip_prefix(ws).unwrap_or(path)
    } else {
        path
    };

    // 2. 检查相对路径各级目录：若位于文档/非代码目录，直接判定为非代码文件
    for comp in rel_path.components() {
        if let std::path::Component::Normal(os_str) = comp {
            let s = os_str.to_string_lossy().to_lowercase();
            if is_non_code_dir_component(&s) {
                return false;
            }
        }
    }

    // 3. 提取文件名与扩展名
    let file_name_lower = rel_path
        .file_name()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    let ext_lower = rel_path
        .extension()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    // 4. 显式非代码/文档文件排除
    if is_non_code_file(&file_name_lower, &ext_lower) {
        return false;
    }

    // 5. 核心构建/清单配置文件
    if is_build_manifest_filename(&file_name_lower) {
        return true;
    }

    // 6. 源码程序语言扩展名
    if is_source_code_extension(&ext_lower) {
        return true;
    }

    // 7. 若在 src/ 或 src-tauri/ 等源码核心目录下，且非排除项（如某些能力配置 json）
    for comp in rel_path.components() {
        if let std::path::Component::Normal(os_str) = comp {
            let s = os_str.to_string_lossy().to_lowercase();
            if matches!(s.as_str(), "src" | "src-tauri" | "lib" | "app" | "pkg") {
                // 如果是 json/toml/yaml 等配置文件，在源码目录下也视为构建相关
                if matches!(ext_lower.as_str(), "json" | "toml" | "yaml" | "yml" | "xml") {
                    return true;
                }
            }
        }
    }

    false
}

/// 快捷方法：无工作区上下文时的判断
#[allow(dead_code)]
pub fn is_code_or_build_file(path: &Path) -> bool {
    is_code_or_build_file_with_workspace(path, None)
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
        assert_eq!(cmd, "cargo check --manifest-path src-tauri/Cargo.toml");
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

    #[test]
    fn test_is_code_or_build_file() {
        let ws = Path::new("F:\\WorkSpace\\Other\\harness_mini");

        // 1. 文档目录及扩展名：应返回 false
        assert!(!is_code_or_build_file_with_workspace(
            Path::new("docs/JEV_DECISION_GATEWAY_INTEGRATION_SPEC.md"),
            Some(ws)
        ));
        assert!(!is_code_or_build_file_with_workspace(
            &ws.join("docs\\specification.markdown"),
            Some(ws)
        ));
        assert!(!is_code_or_build_file_with_workspace(
            Path::new(".harness/plans/20260930-plan.md"),
            Some(ws)
        ));
        assert!(!is_code_or_build_file_with_workspace(
            Path::new(".harness/memory/digests/test.md"),
            Some(ws)
        ));
        assert!(!is_code_or_build_file_with_workspace(
            Path::new("README.md"),
            Some(ws)
        ));
        assert!(!is_code_or_build_file_with_workspace(
            Path::new("LICENSE"),
            Some(ws)
        ));
        assert!(!is_code_or_build_file_with_workspace(
            Path::new(".gitignore"),
            Some(ws)
        ));
        assert!(!is_code_or_build_file_with_workspace(
            Path::new("design.drawio"),
            Some(ws)
        ));
        assert!(!is_code_or_build_file_with_workspace(
            Path::new("public/icon.png"),
            Some(ws)
        ));

        // 2. 源码文件：应返回 true
        assert!(is_code_or_build_file_with_workspace(
            Path::new("src-tauri/src/agent.rs"),
            Some(ws)
        ));
        assert!(is_code_or_build_file_with_workspace(
            &ws.join("src\\components\\FloatingTaskPanel.tsx"),
            Some(ws)
        ));
        assert!(is_code_or_build_file_with_workspace(
            Path::new("src/main.ts"),
            Some(ws)
        ));
        assert!(is_code_or_build_file_with_workspace(
            Path::new("scripts/test.py"),
            Some(ws)
        ));

        // 3. 构建/清单文件：应返回 true
        assert!(is_code_or_build_file_with_workspace(
            Path::new("package.json"),
            Some(ws)
        ));
        assert!(is_code_or_build_file_with_workspace(
            Path::new("src-tauri/Cargo.toml"),
            Some(ws)
        ));
        assert!(is_code_or_build_file_with_workspace(
            Path::new("tsconfig.json"),
            Some(ws)
        ));
        assert!(is_code_or_build_file_with_workspace(
            Path::new("src-tauri/capabilities/default.json"),
            Some(ws)
        ));
    }
}
