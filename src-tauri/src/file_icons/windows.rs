//! Windows: the shell's own icon, through `IShellItemImageFactory`.
//!
//! The implementation stays where it was written — `file_system::preview`'s COM
//! pipeline — because `shell_commands` reaches for the same function to turn the
//! icon paths an `IExplorerCommand` reports into bytes, and the two callers have
//! to agree on one apartment and one cache rather than maintaining two.
//!
//! What this module adds is the seam: the raw PNG the shell hands over is
//! labelled, and the folder case stops being a special one. `IShellItemImageFactory`
//! resolves a directory exactly as it resolves a file, so `is_dir` needs no
//! branch here — unlike on Linux, where a folder's icon lives in a different
//! context of the theme than a file's does.

use super::FileIcon;

pub(super) fn extract(path: &str, size: u32, _is_dir: bool) -> Option<FileIcon> {
    let bytes = crate::file_system::preview::extract_file_icon_png(path, size)?;
    Some(FileIcon {
        mime: "image/png",
        bytes,
    })
}
