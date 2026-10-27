//! User-facing text for the core's error values.
//!
//! `dae-core` reports stable codes for some failures and English prose for
//! others. The Qt shell shows both, so this is the one place that decides which
//! sentences a user actually reads. Anything unrecognized passes through: an
//! unexpected failure is more useful with its own words than behind a generic
//! "something went wrong".

/// Translates the prefixes and codes a user can reach from the Qt shell.
pub(crate) fn friendly_error(message: &str) -> String {
    // Some callers pass the code itself; most pass `FileSystemError::to_string`,
    // so the same lookup runs again on the detail of each known wrapper below.
    if let Some(text) = friendly_code(message) {
        return text.to_owned();
    }
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
        if let Some(text) = friendly_code(detail) {
            return text.to_owned();
        }
        return format!("操作失败：{detail}");
    }
    if let Some(detail) = message.strip_prefix("The directory operation could not complete: ") {
        if let Some(text) = friendly_code(detail) {
            return text.to_owned();
        }
        return format!("操作失败：{detail}");
    }
    // Git codes arrive wrapped in `NotFound`/`AlreadyExists` too. Only known
    // codes are translated; an unrecognized path stays in the core's words.
    if let Some(detail) = message.strip_prefix("The requested path was not found: ")
        && let Some(text) = friendly_code(detail)
    {
        return text.to_owned();
    }
    if let Some(detail) = message.strip_prefix("An item already exists at the destination: ")
        && let Some(text) = friendly_code(detail)
    {
        return text.to_owned();
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
        if let Some(text) = friendly_code(detail) {
            return text.to_owned();
        }
        if detail.contains("SFTP account is required") {
            return "SFTP 需要账号，请填写后重试".to_owned();
        }
        return format!("输入无效：{detail}");
    }
    if let Some(detail) =
        message.strip_prefix("The storage backend does not support this operation: ")
    {
        if let Some(text) = friendly_code(detail) {
            return text.to_owned();
        }
        return format!("当前平台不支持：{detail}");
    }
    if message.contains("No cloud account is connected for:") {
        return "尚未连接这个云盘账户，请先在侧边栏添加网盘账户并完成授权".to_owned();
    }
    message.to_owned()
}

/// The stable codes emitted by `dae-core`, independent of the error variant
/// that wrapped them.
fn friendly_code(code: &str) -> Option<&'static str> {
    if code.starts_with("fs.undo_empty") {
        Some("没有可撤销的操作")
    } else if code.starts_with("fs.redo_empty") {
        Some("没有可重做的操作")
    } else if code.starts_with("fs.trash_local_only") {
        Some("回收站只支持本地项目")
    } else if code.starts_with("fs.terminal_local_dir_only") {
        Some("终端只能打开本地目录")
    } else if code.starts_with("fs.open_with_local_only") {
        Some("只有本地项目可以选择打开方式")
    } else if code.starts_with("fs.open_with_not_found") {
        Some("这个项目已经不存在")
    } else if code.starts_with("fs.open_with_picker_unsupported") {
        Some("当前系统不支持选择打开方式")
    } else if code.starts_with("fs.trash_entries_missing") {
        Some("所选项目已不在回收站")
    } else if code.starts_with("fs.trash_restore_no_origin") {
        Some("无法确定该项目的原位置")
    } else if code.starts_with("git.not_a_worktree") {
        Some("这个目录不在 Git 仓库中")
    } else if code.starts_with("git.branch_not_found") {
        Some("找不到这个分支")
    } else if code.starts_with("git.branch_exists") {
        Some("同名分支已经存在")
    } else if code.starts_with("git.invalid_branch_name") {
        Some("分支名不合法")
    } else if code.starts_with("git.empty_repo") {
        Some("仓库还没有提交，无法创建分支")
    } else if code.starts_with("git.no_remote") {
        Some("仓库没有配置远程地址")
    } else if code.starts_with("git.no_upstream") {
        Some("当前分支还没有上游分支")
    } else if code.starts_with("git.detached_head") {
        Some("当前处于分离 HEAD 状态，请先切换到一个分支")
    } else if code.starts_with("git.checkout_failed") {
        Some("切换分支失败，请先提交或暂存本地改动")
    } else if code.starts_with("git.create_branch_failed") {
        Some("创建分支失败")
    } else if code.starts_with("git.fetch_failed") {
        Some("获取远程更新失败，请检查网络和远程地址")
    } else if code.starts_with("git.pull_failed") {
        Some("拉取失败，请先处理本地改动或冲突")
    } else if code.starts_with("git.push_failed") {
        Some("推送失败，请先拉取远程更新")
    } else if code.starts_with("git.git_not_found") {
        Some("未找到 git 命令，请先安装 Git")
    } else {
        None
    }
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
    fn wrapped_core_codes_are_translated() {
        assert_eq!(
            friendly_error("The requested operation has invalid input: fs.open_with_not_found"),
            "这个项目已经不存在"
        );
        assert_eq!(
            friendly_error(
                "The storage backend does not support this operation: fs.open_with_picker_unsupported"
            ),
            "当前系统不支持选择打开方式"
        );
        assert_eq!(
            friendly_error("The requested operation has invalid input: fs.undo_empty"),
            "没有可撤销的操作"
        );
        assert_eq!(
            friendly_error("The requested operation has invalid input: fs.something_new"),
            "输入无效：fs.something_new"
        );
    }

    #[test]
    fn git_codes_are_translated_in_every_wrapper() {
        assert_eq!(
            friendly_error("The requested path was not found: git.not_a_worktree"),
            "这个目录不在 Git 仓库中"
        );
        assert_eq!(
            friendly_error("An item already exists at the destination: git.branch_exists"),
            "同名分支已经存在"
        );
        assert_eq!(
            friendly_error("The requested operation has invalid input: git.no_upstream"),
            "当前分支还没有上游分支"
        );
        assert_eq!(
            friendly_error(
                "The directory operation could not complete: git.checkout_failed: local changes"
            ),
            "切换分支失败，请先提交或暂存本地改动"
        );
        assert_eq!(
            friendly_error("The directory operation could not complete: git.fetch_failed: timeout"),
            "获取远程更新失败，请检查网络和远程地址"
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
            friendly_error(
                "The requested path was not found: No cloud account is connected for: cloud://google_drive:me@example.com"
            ),
            "尚未连接这个云盘账户，请先在侧边栏添加网盘账户并完成授权"
        );
        assert_eq!(
            friendly_error("The directory operation could not complete: disk full"),
            "操作失败：disk full"
        );
    }
}
