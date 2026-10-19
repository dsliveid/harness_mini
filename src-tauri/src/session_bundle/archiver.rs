use super::bundle::SessionBundle;
use std::path::{Path, PathBuf};

/// 会话 Bundle 物理冷热归档与解归档引擎
pub struct SessionArchiver;

impl SessionArchiver {
    /// 将活跃会话物理移入归档区（data/archives/{session_id}）
    pub fn archive_session_bundle(data_dir: &Path, session_id: &str) -> Result<PathBuf, String> {
        let bundle = SessionBundle::new(session_id, data_dir);
        let bundle_dir = bundle.resource().bundle_dir();
        let archive_dir = bundle.resource().archive_bundle_dir();

        // 确保 archives 父目录存在
        if let Some(parent) = archive_dir.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        // 如果热区 bundle 目录存在，物理移动至归档目录
        if bundle_dir.exists() {
            // 如果归档目录已有旧残留，先清理
            if archive_dir.exists() {
                let _ = std::fs::remove_dir_all(&archive_dir);
            }

            // 优先原子重命名移动；若跨卷或重命名失败则尝试递归拷贝后删除
            if std::fs::rename(&bundle_dir, &archive_dir).is_err() {
                copy_dir_all(&bundle_dir, &archive_dir)
                    .map_err(|e| format!("拷贝归档目录失败: {e}"))?;
                let _ = std::fs::remove_dir_all(&bundle_dir);
            }
        }

        // 如果存在未迁移的遗留附件目录，一并归入归档目录
        let legacy_att = bundle.resource().legacy_attachments_dir();
        if legacy_att.exists() {
            let target_att = archive_dir.join("attachments");
            let _ = std::fs::create_dir_all(&target_att);
            if let Ok(entries) = std::fs::read_dir(&legacy_att) {
                for entry in entries.flatten() {
                    let _ = std::fs::copy(entry.path(), target_att.join(entry.file_name()));
                }
            }
            let _ = std::fs::remove_dir_all(&legacy_att);
        }

        Ok(archive_dir)
    }

    /// 将已归档会话从冷区恢复回热区（data/sessions/{session_id}）
    pub fn unarchive_session_bundle(data_dir: &Path, session_id: &str) -> Result<PathBuf, String> {
        let bundle = SessionBundle::new(session_id, data_dir);
        let bundle_dir = bundle.resource().bundle_dir();
        let archive_dir = bundle.resource().archive_bundle_dir();

        if let Some(parent) = bundle_dir.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        if archive_dir.exists() {
            if bundle_dir.exists() {
                let _ = std::fs::remove_dir_all(&bundle_dir);
            }

            if std::fs::rename(&archive_dir, &bundle_dir).is_err() {
                copy_dir_all(&archive_dir, &bundle_dir)
                    .map_err(|e| format!("还原归档目录失败: {e}"))?;
                let _ = std::fs::remove_dir_all(&archive_dir);
            }
        }

        Ok(bundle_dir)
    }
}

/// 递归拷贝目录辅助工具
fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst.join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_archive_and_unarchive_flow() {
        let temp_dir = std::env::temp_dir().join(format!("hm_arc_test_{}", uuid::Uuid::new_v4()));
        let sid = "session_arch_test";
        let bundle = SessionBundle::new(sid, &temp_dir);

        // 创建测试会话文件与附件
        bundle.resource().ensure_bundle_dir().unwrap();
        std::fs::write(bundle.resource().bundle_db_path(), b"fake sqlite").unwrap();
        std::fs::write(bundle.resource().bundle_attachments_dir().join("pic.png"), b"image").unwrap();

        assert!(bundle.resource().bundle_dir().exists());
        assert!(!bundle.resource().archive_bundle_dir().exists());

        // 执行归档
        SessionArchiver::archive_session_bundle(&temp_dir, sid).unwrap();

        // 验证热区已清理，冷区已落盘
        assert!(!bundle.resource().bundle_dir().exists());
        assert!(bundle.resource().archive_bundle_dir().exists());
        assert!(bundle.resource().archive_bundle_dir().join("session.db").exists());
        assert!(bundle.resource().archive_bundle_dir().join("attachments").join("pic.png").exists());

        // 执行解归档
        SessionArchiver::unarchive_session_bundle(&temp_dir, sid).unwrap();

        // 验证已恢复回热区
        assert!(bundle.resource().bundle_dir().exists());
        assert!(!bundle.resource().archive_bundle_dir().exists());
        assert!(bundle.resource().bundle_db_path().exists());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
