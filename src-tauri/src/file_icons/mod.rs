//! The operating system's own icon for a file or folder.
//!
//! Same shape as [`crate::system_accent`]: one question asked of whichever
//! platform this was built for, and every answer handed to the frontend through
//! the `fileicon://` protocol that `file_system::preview` already serves. That
//! seam is deliberate — the protocol carries the mtime + size version tag, the
//! immutable cache headers, the render pool that keeps extraction off the UI
//! thread, and the 404 that lets a row fall back to a drawn glyph. A second
//! channel for the same bytes would have to re-derive all of it.
//!
//! Why this is a module and not three `#[cfg]` blocks in `preview.rs`: each
//! platform reaches its icons through an entirely different mechanism — a COM
//! shell item, an AppKit workspace call, a spec-defined search over the XDG
//! data roots — and two of the three need a resolver of their own that is
//! worth testing apart from the render path that calls it.
//!
//! The three backends answer one question, `extract`, and a `None` answer is a
//! normal outcome rather than a failure: it means this file has no icon the OS
//! can name, and the frontend keeps its own artwork.

/// One rendered icon. `mime` travels with the bytes because Linux answers with
/// SVG wherever the installed theme ships one, and a webview will only render
/// that as a vector if it is told it is one.
pub(crate) struct FileIcon {
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

#[cfg(any(target_os = "linux", test))]
#[cfg_attr(test, allow(dead_code))]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

/// The freedesktop theme search, re-exported for the context menu's `Icon=`
/// values. Same cfg as the module's own, which is the same one
/// `shell_commands::linux` carries — both are built on a Linux host and both are
/// built under `test`, so a Windows machine can type-check and run them.
#[cfg(any(target_os = "linux", test))]
#[cfg_attr(test, allow(dead_code))]
pub(crate) use linux::resolve_named_icon;

#[cfg(target_os = "linux")]
use linux as backend;
#[cfg(target_os = "macos")]
use macos as backend;
#[cfg(windows)]
use windows as backend;

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
mod backend {
    //! No icon reader is written for this platform. Saying so is the difference
    //! between an app that shows its own glyphs and one that fails to build.
    pub(super) fn extract(_path: &str, _size: u32, _is_dir: bool) -> Option<super::FileIcon> {
        None
    }
}

/// The icon the OS would show for `path` at `size` CSS pixels, or `None` when
/// it has none.
///
/// `is_dir` is a parameter rather than something read off the disk because the
/// caller (`render_file_icon`) has already paid for that `metadata` call, and
/// because on Linux a directory resolves through a different icon context
/// (`places`) than a file does (`mimetypes`) — the two are not interchangeable
/// and guessing from the path would be wrong for a symlink to either.
pub(crate) fn extract(path: &str, size: u32, is_dir: bool) -> Option<FileIcon> {
    backend::extract(path, size, is_dir)
}
