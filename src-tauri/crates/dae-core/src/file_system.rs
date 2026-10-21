//! File system backends and the platform-neutral operations on top of them.

pub mod cloud;
pub mod connections;
pub mod entry_codec;
pub mod error;
pub mod fs_atomic;
pub mod git;
pub mod known_hosts;
pub mod local;
#[cfg(target_os = "macos")]
pub mod macos_trash;
pub mod progress;
pub mod sftp;
pub mod smb;
pub mod transfer;
pub mod types;
pub mod vfs;

#[cfg(test)]
mod test_support;
