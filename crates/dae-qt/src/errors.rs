//! User-facing text for the core's error values.
//!
//! `dae-core` reports stable codes for some failures and English prose for
//! others. The Qt shell shows both, so this is the one place that decides which
//! sentences a user actually reads. Anything unrecognized passes through: an
//! unexpected failure is more useful with its own words than behind a generic
//! "something went wrong".

/// Translates the prefixes and codes a user can reach from the Qt shell.
pub(crate) fn friendly_error(message: &str) -> String {
    if message.contains("has no SFTP subsystem") {
        return "服务器未启用 SFTP 子系统".to_owned();
    }
    if let Some(detail) = message.strip_prefix("The file system operation failed: ") {
        if detail.starts_with("Could not reach the SMB server") {
            return "无法连接服务器，请检查地址、端口和网络".to_owned();
        }
        if detail.starts_with("SFTP authentication failed") {
            return "SFTP 认证失败，请检查账号和密码".to_owned();
        }
        return format!("操作失败：{detail}");
    }
    if let Some(detail) = message.strip_prefix("The directory operation could not complete: ") {
        return format!("操作失败：{detail}");
    }
    if let Some(detail) = message.strip_prefix("Permission was denied: ") {
        if message.contains("rejected the connection") {
            return "服务器拒绝了连接，请填写账号和密码后重试".to_owned();
        }
        if message.contains("rejected the account or password") {
            return "服务器拒绝了账号或密码".to_owned();
        }
        return format!("权限被拒绝：{detail}");
    }
    if let Some(detail) = message.strip_prefix("The requested operation has invalid input: ") {
        if detail.contains("SFTP account is required") {
            return "SFTP 需要账号，请填写后重试".to_owned();
        }
        return format!("输入无效：{detail}");
    }
    if message.starts_with("fs.undo_empty") {
        return "没有可撤销的操作".to_owned();
    }
    if message.starts_with("fs.redo_empty") {
        return "没有可重做的操作".to_owned();
    }
    if message.starts_with("fs.trash_local_only") {
        return "回收站只支持本地项目".to_owned();
    }
    if message.starts_with("fs.terminal_local_dir_only") {
        return "终端只能打开本地目录".to_owned();
    }
    if message.starts_with("fs.open_with_local_only") {
        return "只有本地项目可以选择打开方式".to_owned();
    }
    if message.starts_with("fs.trash_entries_missing") {
        return "所选项目已不在回收站".to_owned();
    }
    if message.starts_with("fs.trash_restore_no_origin") {
        return "无法确定该项目的原位置".to_owned();
    }
    message.to_owned()
}

#[cfg(test)]
mod tests {
    use super::friendly_error;

    #[test]
    fn known_core_codes_become_sentences_and_others_pass_through() {
        assert_eq!(friendly_error("fs.undo_empty"), "没有可撤销的操作");
        assert_eq!(
            friendly_error("The requested path was not found: C:/x"),
            "The requested path was not found: C:/x"
        );
    }

    #[test]
    fn connection_failures_get_actionable_chinese_copy() {
        assert_eq!(
            friendly_error(
                "The file system operation failed: Could not reach the SMB server: refused"
            ),
            "无法连接服务器，请检查地址、端口和网络"
        );
        assert_eq!(
            friendly_error(
                "Permission was denied: The server rejected the account or password. Check the credentials."
            ),
            "服务器拒绝了账号或密码"
        );
        assert_eq!(
            friendly_error(
                "The requested operation has invalid input: An SFTP account is required."
            ),
            "SFTP 需要账号，请填写后重试"
        );
        assert_eq!(
            friendly_error("The file system operation failed: The server has no SFTP subsystem"),
            "服务器未启用 SFTP 子系统"
        );
        assert_eq!(
            friendly_error("The directory operation could not complete: disk full"),
            "操作失败：disk full"
        );
    }
}
