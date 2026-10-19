use std::path::PathBuf;

/// 会话外部资源域与物理生命周期抽象
#[derive(Debug, Clone)]
pub struct SessionResource {
    pub session_id: String,
    pub data_dir: PathBuf,
}

impl SessionResource {
    pub fn new(session_id: impl Into<String>, data_dir: impl Into<PathBuf>) -> Self {
        Self {
            session_id: session_id.into(),
            data_dir: data_dir.into(),
        }
    }

    /// 旧版散落附件目录：data/attachments/{session_id}
    pub fn legacy_attachments_dir(&self) -> PathBuf {
        self.data_dir.join("attachments").join(&self.session_id)
    }

    /// 会话单元化 Bundle 目录：data/sessions/{session_id}
    pub fn bundle_dir(&self) -> PathBuf {
        self.data_dir.join("sessions").join(&self.session_id)
    }

    /// Bundle 内部专属附件目录：data/sessions/{session_id}/attachments
    pub fn bundle_attachments_dir(&self) -> PathBuf {
        self.bundle_dir().join("attachments")
    }

    /// Bundle 内部专属 SQLite 库文件：data/sessions/{session_id}/session.db
    pub fn bundle_db_path(&self) -> PathBuf {
        self.bundle_dir().join("session.db")
    }

    /// 归档目录：data/archives/{session_id}.tar.zst 或 data/archives/{session_id}
    pub fn archive_bundle_dir(&self) -> PathBuf {
        self.data_dir.join("archives").join(&self.session_id)
    }

    /// 归档压缩包路径：data/archives/{session_id}.hmsession
    pub fn archive_package_path(&self) -> PathBuf {
        self.data_dir
            .join("archives")
            .join(format!("{}.hmsession", self.session_id))
    }

    /// 确保会话 Bundle 目录存在
    pub fn ensure_bundle_dir(&self) -> Result<PathBuf, String> {
        let bdir = self.bundle_dir();
        std::fs::create_dir_all(&bdir).map_err(|e| format!("创建会话Bundle目录失败: {e}"))?;
        let att_dir = self.bundle_attachments_dir();
        std::fs::create_dir_all(&att_dir).map_err(|e| format!("创建会话附件目录失败: {e}"))?;
        Ok(bdir)
    }

    /// 彻底清理该会话关联的所有物理文件与目录（包括遗留附件与新版 Bundle 目录）
    pub fn cleanup_all_external_resources(&self) -> Result<(), String> {
        let legacy = self.legacy_attachments_dir();
        if legacy.exists() {
            let _ = std::fs::remove_dir_all(&legacy);
        }

        let bundle = self.bundle_dir();
        if bundle.exists() {
            let _ = std::fs::remove_dir_all(&bundle);
        }

        let archive_pkg = self.archive_package_path();
        if archive_pkg.exists() {
            let _ = std::fs::remove_file(&archive_pkg);
        }

        let archive_dir = self.archive_bundle_dir();
        if archive_dir.exists() {
            let _ = std::fs::remove_dir_all(&archive_dir);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_session_resource_paths() {
        let res = SessionResource::new("test-sess-1", Path::new("C:/test_data"));
        assert_eq!(
            res.legacy_attachments_dir(),
            PathBuf::from("C:/test_data/attachments/test-sess-1")
        );
        assert_eq!(
            res.bundle_dir(),
            PathBuf::from("C:/test_data/sessions/test-sess-1")
        );
        assert_eq!(
            res.bundle_attachments_dir(),
            PathBuf::from("C:/test_data/sessions/test-sess-1/attachments")
        );
        assert_eq!(
            res.bundle_db_path(),
            PathBuf::from("C:/test_data/sessions/test-sess-1/session.db")
        );
    }

    #[test]
    fn test_cleanup_removes_created_dirs() {
        let temp_dir = std::env::temp_dir().join(format!("hm_res_test_{}", uuid::Uuid::new_v4()));
        let res = SessionResource::new("sess_abc", &temp_dir);

        // 创建临时遗留附件目录和 bundle 目录
        std::fs::create_dir_all(res.legacy_attachments_dir()).unwrap();
        std::fs::write(res.legacy_attachments_dir().join("img.png"), b"test").unwrap();
        res.ensure_bundle_dir().unwrap();
        std::fs::write(res.bundle_attachments_dir().join("photo.jpg"), b"photo").unwrap();

        assert!(res.legacy_attachments_dir().exists());
        assert!(res.bundle_dir().exists());

        res.cleanup_all_external_resources().unwrap();

        assert!(!res.legacy_attachments_dir().exists());
        assert!(!res.bundle_dir().exists());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
