use super::types::SecurityZone;
use std::path::{Path, PathBuf};

pub struct ZoneResolver;

impl ZoneResolver {
    /// 标准化路径字符串（统一斜杠为 '/'，去除首尾空白与包裹的引号，处理 Git Bash 盘符）
    pub fn normalize_path_str(p: &str) -> String {
        let trimmed = p.trim().trim_matches(|c| c == '\'' || c == '"');
        let norm = trimmed.replace('\\', "/");

        // 处理 Git Bash / MSYS 盘符语法：/d/path -> D:/path
        if norm.len() >= 3 && norm.starts_with('/') && norm.chars().nth(2) == Some('/') {
            let drive = norm.chars().nth(1).unwrap();
            if drive.is_ascii_alphabetic() {
                return format!("{}:/{}", drive.to_ascii_uppercase(), &norm[3..]);
            }
        }
        norm
    }

    /// 判定相对路径或绝对路径是否指向临时暂存区（Ephemeral Zone）
    pub fn is_ephemeral_path(path_str: &str) -> bool {
        let norm = Self::normalize_path_str(path_str);
        let clean = norm.trim_start_matches("./");

        // 禁止路径穿越
        if clean.contains("/../") || clean.ends_with("/..") || clean.starts_with("../") || clean == ".." {
            return false;
        }

        // 匹配项目级与会话级临时缓存规则
        if clean == ".harness"
            || clean.starts_with(".harness/")
            || clean == ".harness/tmp"
            || clean.starts_with(".harness/tmp/")
            || clean == ".harness/cache"
            || clean.starts_with(".harness/cache/")
            || clean.starts_with(".dev-data/temp-")
            || clean.starts_with("target/tmp/")
        {
            return true;
        }

        // 匹配操作系统临时目录
        if let Ok(sys_temp) = std::env::temp_dir().canonicalize() {
            let sys_temp_norm = Self::normalize_path_str(&sys_temp.to_string_lossy());
            if norm.starts_with(&sys_temp_norm) || norm.starts_with("/tmp/") || norm == "/tmp" {
                return true;
            }
        }

        false
    }

    /// 判定路径是否指向 .git 核心元数据目录
    pub fn is_git_dir(path_str: &str) -> bool {
        let norm = Self::normalize_path_str(path_str);
        let clean = norm.trim_start_matches("./");
        clean == ".git"
            || clean.starts_with(".git/")
            || clean.contains("/.git/")
            || clean.ends_with("/.git")
    }

    /// 将路径解析为绝对 PathBuf（不展开符号链接，支持 Git Bash 盘符）
    pub fn resolve_path(base: &Path, rel_or_abs: &str) -> PathBuf {
        let norm = Self::normalize_path_str(rel_or_abs);
        let p = Path::new(&norm);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            let clean = norm.trim_start_matches(|c| c == '/' || c == '\\');
            base.join(clean)
        }
    }

    /// 判断目标绝对路径是否位于根目录内（支持未创建的新路径向上查找）
    pub fn is_subpath_of(target: &Path, root: &Path) -> bool {
        let Ok(root_canon) = root.canonicalize() else {
            return false;
        };

        if let Ok(target_canon) = target.canonicalize() {
            return target_canon.starts_with(&root_canon);
        }

        let mut curr = target;
        while let Some(parent) = curr.parent() {
            if let Ok(parent_canon) = parent.canonicalize() {
                return parent_canon.starts_with(&root_canon);
            }
            curr = parent;
        }
        false
    }

    /// 综合评估任意路径字符串的安全域归属
    pub fn resolve_zone(
        raw_path: &str,
        workspace_root: &Path,
        sandbox_root: Option<&Path>,
    ) -> SecurityZone {
        let root = sandbox_root.unwrap_or(workspace_root);
        let resolved = Self::resolve_path(root, raw_path);
        let norm_str = Self::normalize_path_str(raw_path);

        // 1. 检查是否越出工作区/沙箱根
        if !Self::is_subpath_of(&resolved, root) {
            // 例外：若是系统标准临时目录，则依然允许归为 Ephemeral
            if Self::is_ephemeral_path(raw_path) {
                return SecurityZone::Ephemeral;
            }
            return SecurityZone::Critical;
        }

        // 2. 检查是否触碰 .git 核心元数据
        if Self::is_git_dir(&norm_str) {
            return SecurityZone::Critical;
        }

        // 3. 检查相对工作区根的路径是否属于 Ephemeral 暂存区
        if let Ok(rel) = resolved.strip_prefix(root) {
            let rel_str = Self::normalize_path_str(&rel.to_string_lossy());
            if Self::is_ephemeral_path(&rel_str) {
                return SecurityZone::Ephemeral;
            }
        }

        // 4. 直接判定字符串模式
        if Self::is_ephemeral_path(&norm_str) {
            return SecurityZone::Ephemeral;
        }

        // 5. 位于工作区内的其余文件均归为 Workspace
        SecurityZone::Workspace
    }
}
