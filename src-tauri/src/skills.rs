use crate::models::SkillItem;
use std::path::{Path, PathBuf};

pub fn skills_dir(workspace: &Path) -> PathBuf {
    workspace.join(".harness").join("skills")
}

pub fn sanitize_skill_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("技能名称不能为空".into());
    }
    if !trimmed.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err("技能名称仅支持英文字母、数字、下划线及连字符（如 build-check）".into());
    }
    Ok(trimmed.to_string())
}

/// 解析 SKILL.md 中的描述信息
fn parse_skill_md(content: &str) -> (String, Option<String>) {
    let mut description = String::new();
    let mut script_type = None;

    let lines: Vec<&str> = content.lines().collect();
    if lines.first().map(|s| s.trim()) == Some("---") {
        let mut i = 1;
        while i < lines.len() {
            let line = lines[i].trim();
            if line == "---" {
                i += 1;
                break;
            }
            if let Some(desc) = line.strip_prefix("description:") {
                description = desc.trim().trim_matches('"').trim_matches('\'').to_string();
            } else if let Some(st) = line.strip_prefix("script_type:") {
                script_type = Some(st.trim().to_string());
            }
            i += 1;
        }
        if description.is_empty() {
            // 取正文首个非空行
            while i < lines.len() {
                let line = lines[i].trim();
                if !line.is_empty() && !line.starts_with('#') {
                    description = line.to_string();
                    break;
                }
                i += 1;
            }
        }
    } else {
        for line in lines {
            let line = line.trim();
            if !line.is_empty() && !line.starts_with('#') {
                description = line.to_string();
                break;
            }
        }
    }

    (description, script_type)
}

/// 列出工作区内所有已定义技能
pub async fn list_skills(workspace: &Path) -> Result<Vec<SkillItem>, String> {
    let base = skills_dir(workspace);
    if !base.exists() {
        return Ok(Vec::new());
    }

    let mut dir = match tokio::fs::read_dir(&base).await {
        Ok(d) => d,
        Err(_) => return Ok(Vec::new()),
    };

    let mut skills = Vec::new();
    while let Some(entry) = dir.next_entry().await.map_err(|e| e.to_string())? {
        let ft = entry.file_type().await.map_err(|e| e.to_string())?;
        if !ft.is_dir() {
            continue;
        }
        let skill_dir = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        let skill_md_path = skill_dir.join("SKILL.md");
        let (mut description, frontmatter_type) = if skill_md_path.exists() {
            let content = tokio::fs::read_to_string(&skill_md_path).await.unwrap_or_default();
            parse_skill_md(&content)
        } else {
            (String::new(), None)
        };

        // 查找脚本文件
        let candidates = ["run.bat", "run.ps1", "run.sh", "run.py", "run.js", "run.cmd"];
        let mut found_script_path = None;
        let mut script_type = frontmatter_type.unwrap_or_else(|| {
            if cfg!(windows) { "bat".into() } else { "sh".into() }
        });
        let mut script_content = String::new();

        for candidate in candidates {
            let p = skill_dir.join(candidate);
            if p.exists() {
                found_script_path = Some(p.clone());
                script_type = Path::new(candidate)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("bat")
                    .to_string();
                if let Ok(c) = tokio::fs::read_to_string(&p).await {
                    script_content = c;
                }
                break;
            }
        }

        if description.is_empty() {
            description = format!("项目技能 {name}");
        }

        let rel_path = found_script_path
            .as_ref()
            .and_then(|p| p.strip_prefix(workspace).ok())
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|| format!(".harness/skills/{name}"));

        let updated_at = entry
            .metadata()
            .await
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|t| {
                let dur = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
                chrono::DateTime::from_timestamp(dur.as_secs() as i64, 0)
                    .map(|dt| dt.to_rfc3339())
                    .unwrap_or_else(|| crate::store::now())
            })
            .unwrap_or_else(|| crate::store::now());

        skills.push(SkillItem {
            name,
            description,
            script_type,
            path: rel_path,
            content: script_content,
            updated_at,
        });
    }

    skills.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(skills)
}

/// 保存/更新一个项目技能
pub async fn save_skill(
    workspace: &Path,
    name: &str,
    description: &str,
    script_type: &str,
    content: &str,
) -> Result<SkillItem, String> {
    let safe_name = sanitize_skill_name(name)?;
    let skill_folder = skills_dir(workspace).join(&safe_name);
    tokio::fs::create_dir_all(&skill_folder)
        .await
        .map_err(|e| format!("创建技能目录失败: {e}"))?;

    let ext = match script_type.to_lowercase().as_str() {
        "ps1" => "ps1",
        "sh" => "sh",
        "py" => "py",
        "js" => "js",
        _ => "bat",
    };
    let script_filename = format!("run.{ext}");
    let script_path = skill_folder.join(&script_filename);

    tokio::fs::write(&script_path, content)
        .await
        .map_err(|e| format!("写入技能脚本失败: {e}"))?;

    let skill_md_content = format!(
        r#"---
name: {safe_name}
description: "{description}"
script_type: {ext}
---
# {safe_name}

{description}

## 执行脚本
```{ext}
{content}
```
"#
    );
    let md_path = skill_folder.join("SKILL.md");
    tokio::fs::write(&md_path, skill_md_content)
        .await
        .map_err(|e| format!("写入 SKILL.md 失败: {e}"))?;

    let rel_path = script_path
        .strip_prefix(workspace)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| format!(".harness/skills/{safe_name}/{script_filename}"));

    Ok(SkillItem {
        name: safe_name,
        description: description.to_string(),
        script_type: ext.to_string(),
        path: rel_path,
        content: content.to_string(),
        updated_at: crate::store::now(),
    })
}

/// 删除一个技能
pub async fn delete_skill(workspace: &Path, name: &str) -> Result<(), String> {
    let safe_name = sanitize_skill_name(name)?;
    let skill_folder = skills_dir(workspace).join(&safe_name);
    if skill_folder.exists() {
        tokio::fs::remove_dir_all(&skill_folder)
            .await
            .map_err(|e| format!("删除技能失败: {e}"))?;
    }
    Ok(())
}

/// 根据技能名称及参数构建执行命令与工作目录
pub fn build_skill_command(
    workspace: &Path,
    skill_name: &str,
    args: Option<&str>,
) -> Result<(String, PathBuf), String> {
    let safe_name = sanitize_skill_name(skill_name)?;
    let skill_folder = skills_dir(workspace).join(&safe_name);
    if !skill_folder.exists() {
        return Err(format!("未找到技能 {safe_name}"));
    }

    let candidates = ["run.bat", "run.ps1", "run.sh", "run.py", "run.js", "run.cmd"];
    let mut found = None;
    for c in candidates {
        let p = skill_folder.join(c);
        if p.exists() {
            found = Some((c, p));
            break;
        }
    }

    let (filename, script_path) = found.ok_or_else(|| format!("技能 {safe_name} 下未找到可执行脚本"))?;
    let path_str = script_path.to_string_lossy();
    let arg_str = args.unwrap_or("").trim();

    let cmd = if filename.ends_with(".bat") || filename.ends_with(".cmd") {
        if arg_str.is_empty() {
            format!("\"{}\"", path_str)
        } else {
            format!("\"{}\" {}", path_str, arg_str)
        }
    } else if filename.ends_with(".ps1") {
        if arg_str.is_empty() {
            format!("powershell -NoProfile -ExecutionPolicy Bypass -File \"{}\"", path_str)
        } else {
            format!("powershell -NoProfile -ExecutionPolicy Bypass -File \"{}\" {}", path_str, arg_str)
        }
    } else if filename.ends_with(".py") {
        if arg_str.is_empty() {
            format!("python \"{}\"", path_str)
        } else {
            format!("python \"{}\" {}", path_str, arg_str)
        }
    } else if filename.ends_with(".js") {
        if arg_str.is_empty() {
            format!("node \"{}\"", path_str)
        } else {
            format!("node \"{}\" {}", path_str, arg_str)
        }
    } else {
        if arg_str.is_empty() {
            format!("sh \"{}\"", path_str)
        } else {
            format!("sh \"{}\" {}", path_str, arg_str)
        }
    };

    Ok((cmd, workspace.to_path_buf()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_test_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hm_skill_test_{tag}_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[tokio::test]
    async fn test_skill_crud() {
        let dir = temp_test_dir("crud");
        let ws = dir.as_path();

        // 初始为空
        let list = list_skills(ws).await.unwrap();
        assert!(list.is_empty());

        // 保存技能
        let saved = save_skill(
            ws,
            "test-tool",
            "测试技能描述",
            "bat",
            "@echo hello from skill",
        )
        .await
        .unwrap();
        assert_eq!(saved.name, "test-tool");
        assert_eq!(saved.script_type, "bat");
        assert!(saved.content.contains("hello from skill"));

        // 列出技能
        let list = list_skills(ws).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "test-tool");
        assert_eq!(list[0].description, "测试技能描述");

        // 构建命令
        let (cmd, _) = build_skill_command(ws, "test-tool", Some("--arg1")).unwrap();
        assert!(cmd.contains("run.bat"));
        assert!(cmd.contains("--arg1"));

        // 删除技能
        delete_skill(ws, "test-tool").await.unwrap();
        let list_after = list_skills(ws).await.unwrap();
        assert!(list_after.is_empty());
    }

    #[test]
    fn test_sanitize_name() {
        assert!(sanitize_skill_name("valid_name-1").is_ok());
        assert!(sanitize_skill_name("invalid name with spaces").is_err());
        assert!(sanitize_skill_name("../traversal").is_err());
    }
}
