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
        return "The server does not have the SFTP subsystem enabled".to_owned();
    }
    if let Some(detail) = message.strip_prefix("The file system operation failed: ") {
        if detail.starts_with("Could not reach the SMB server") {
            return "Could not connect to the server. Check the address, port, and network connection."
                .to_owned();
        }
        if detail.starts_with("SFTP authentication failed") {
            return "SFTP authentication failed. Check the username and password.".to_owned();
        }
        if let Some(text) = friendly_code(detail) {
            return text.to_owned();
        }
        return format!("Operation failed: {detail}");
    }
    if let Some(detail) = message.strip_prefix("The directory operation could not complete: ") {
        if let Some(text) = friendly_code(detail) {
            return text.to_owned();
        }
        return format!("Operation failed: {detail}");
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
            return "The server rejected the connection. Enter your username and password, then try again."
                .to_owned();
        }
        if message.contains("rejected the account or password") {
            return "The server rejected the username or password".to_owned();
        }
        return format!("Permission denied: {detail}");
    }
    if let Some(detail) = message.strip_prefix("The requested operation has invalid input: ") {
        if let Some(text) = friendly_code(detail) {
            return text.to_owned();
        }
        if detail.contains("SFTP account is required") {
            return "SFTP requires a username. Enter one and try again.".to_owned();
        }
        return format!("Invalid input: {detail}");
    }
    if let Some(detail) =
        message.strip_prefix("The storage backend does not support this operation: ")
    {
        if let Some(text) = friendly_code(detail) {
            return text.to_owned();
        }
        return format!("Not supported on this platform: {detail}");
    }
    if message.contains("No cloud account is connected for:") {
        return "No cloud account is connected. Add a cloud account in the sidebar and complete authorization first."
            .to_owned();
    }
    message.to_owned()
}

/// The stable codes emitted by `dae-core`, independent of the error variant
/// that wrapped them.
fn friendly_code(code: &str) -> Option<&'static str> {
    if code.starts_with("fs.undo_empty") {
        Some("Nothing to undo")
    } else if code.starts_with("fs.redo_empty") {
        Some("Nothing to redo")
    } else if code.starts_with("fs.trash_local_only") {
        Some("Trash is only available for local items")
    } else if code.starts_with("fs.terminal_local_dir_only") {
        Some("Terminal can only open local directories")
    } else if code.starts_with("fs.open_with_local_only") {
        Some("Open With is only available for local items")
    } else if code.starts_with("fs.open_with_not_found") {
        Some("This item no longer exists")
    } else if code.starts_with("fs.open_with_picker_unsupported") {
        Some("Open With is not supported on this system")
    } else if code.starts_with("fs.trash_entries_missing") {
        Some("The selected items are no longer in Trash")
    } else if code.starts_with("fs.trash_restore_no_origin") {
        Some("The item's original location could not be determined")
    } else if code.starts_with("git.not_a_worktree") {
        Some("This directory is not in a Git repository")
    } else if code.starts_with("git.branch_not_found") {
        Some("Branch not found")
    } else if code.starts_with("git.branch_exists") {
        Some("A branch with this name already exists")
    } else if code.starts_with("git.invalid_branch_name") {
        Some("Invalid branch name")
    } else if code.starts_with("git.empty_repo") {
        Some("Cannot create a branch because the repository has no commits")
    } else if code.starts_with("git.no_remote") {
        Some("No remote is configured for this repository")
    } else if code.starts_with("git.no_upstream") {
        Some("The current branch has no upstream branch")
    } else if code.starts_with("git.detached_head") {
        Some("HEAD is detached. Switch to a branch first.")
    } else if code.starts_with("git.checkout_failed") {
        Some("Could not switch branches. Commit or stash your local changes first.")
    } else if code.starts_with("git.create_branch_failed") {
        Some("Could not create branch")
    } else if code.starts_with("git.fetch_failed") {
        Some("Could not fetch remote updates. Check the network connection and remote URL.")
    } else if code.starts_with("git.pull_failed") {
        Some("Could not pull. Resolve local changes or conflicts first.")
    } else if code.starts_with("git.push_failed") {
        Some("Could not push. Pull remote updates first.")
    } else if code.starts_with("git.git_not_found") {
        Some("Git was not found. Install Git first.")
    } else if code.starts_with("default_manager.set_failed") {
        Some("Could not set the default file manager")
    } else if code.starts_with("default_manager.status_failed") {
        Some("Could not read the default file manager status")
    } else {
        None
    }
}

/// The manual follow-up a successful platform call may leave the user with.
/// Returns an empty string for unknown codes so the panel hides that line.
pub(crate) fn default_manager_detail(code: &str) -> &'static str {
    match code {
        "default_manager.finish_in_system_settings" => {
            "Confirm dae on the Default apps page in Windows Settings."
        }
        "default_manager.cannot_unset_macos" => {
            "macOS does not allow apps to remove their default status. Choose another app in Finder settings."
        }
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::{default_manager_detail, friendly_error};

    #[test]
    fn known_core_codes_become_sentences_and_others_pass_through() {
        assert_eq!(friendly_error("fs.undo_empty"), "Nothing to undo");
        assert_eq!(
            friendly_error("The requested path was not found: C:/x"),
            "The requested path was not found: C:/x"
        );
    }

    #[test]
    fn wrapped_core_codes_are_translated() {
        assert_eq!(
            friendly_error("The requested operation has invalid input: fs.open_with_not_found"),
            "This item no longer exists"
        );
        assert_eq!(
            friendly_error(
                "The storage backend does not support this operation: fs.open_with_picker_unsupported"
            ),
            "Open With is not supported on this system"
        );
        assert_eq!(
            friendly_error("The requested operation has invalid input: fs.undo_empty"),
            "Nothing to undo"
        );
        assert_eq!(
            friendly_error("The requested operation has invalid input: fs.something_new"),
            "Invalid input: fs.something_new"
        );
    }

    #[test]
    fn git_codes_are_translated_in_every_wrapper() {
        assert_eq!(
            friendly_error("The requested path was not found: git.not_a_worktree"),
            "This directory is not in a Git repository"
        );
        assert_eq!(
            friendly_error("An item already exists at the destination: git.branch_exists"),
            "A branch with this name already exists"
        );
        assert_eq!(
            friendly_error("The requested operation has invalid input: git.no_upstream"),
            "The current branch has no upstream branch"
        );
        assert_eq!(
            friendly_error(
                "The directory operation could not complete: git.checkout_failed: local changes"
            ),
            "Could not switch branches. Commit or stash your local changes first."
        );
        assert_eq!(
            friendly_error("The directory operation could not complete: git.fetch_failed: timeout"),
            "Could not fetch remote updates. Check the network connection and remote URL."
        );
    }

    #[test]
    fn default_manager_codes_and_details_are_translated() {
        assert_eq!(
            friendly_error(
                "The directory operation could not complete: default_manager.set_failed: 5"
            ),
            "Could not set the default file manager"
        );
        assert_eq!(
            default_manager_detail("default_manager.finish_in_system_settings"),
            "Confirm dae on the Default apps page in Windows Settings."
        );
        assert_eq!(default_manager_detail("unknown"), "");
    }

    #[test]
    fn connection_failures_get_actionable_english_copy() {
        assert_eq!(
            friendly_error(
                "The file system operation failed: Could not reach the SMB server: refused"
            ),
            "Could not connect to the server. Check the address, port, and network connection."
        );
        assert_eq!(
            friendly_error(
                "Permission was denied: The server rejected the account or password. Check the credentials."
            ),
            "The server rejected the username or password"
        );
        assert_eq!(
            friendly_error(
                "The requested operation has invalid input: An SFTP account is required."
            ),
            "SFTP requires a username. Enter one and try again."
        );
        assert_eq!(
            friendly_error("The file system operation failed: The server has no SFTP subsystem"),
            "The server does not have the SFTP subsystem enabled"
        );
        assert_eq!(
            friendly_error(
                "The requested path was not found: No cloud account is connected for: cloud://google_drive:me@example.com"
            ),
            "No cloud account is connected. Add a cloud account in the sidebar and complete authorization first."
        );
        assert_eq!(
            friendly_error("The directory operation could not complete: disk full"),
            "Operation failed: disk full"
        );
    }
}
