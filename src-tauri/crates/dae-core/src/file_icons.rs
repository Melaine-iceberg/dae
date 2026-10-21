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
//! The three backends answer two questions, `extract` for one entry and
//! `extract_type` for a whole type, and a `None` answer is a normal outcome
//! rather than a failure: it means this file has no icon the OS can name, and the
//! frontend keeps its own artwork.

/// One rendered icon. `mime` travels with the bytes because the producers
/// disagree about format: Linux rasterizes a themed SVG to PNG at the requested
/// size (see `file_icons::linux` for why the vector must not reach the webview),
/// while the other two platforms hand back whatever their shell extracted.
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

/// The freedesktop theme search wrapped for the rows that name an icon rather
/// than a file and travel as one message — a `.desktop`'s `Icon=`, which is
/// where the context menu gets its glyphs. Same cfg as the module's own, which
/// is the same one `shell_commands::linux` carries — both are built on a Linux
/// host and both are built under `test`, so a Windows machine can type-check and
/// run them.
#[cfg(any(target_os = "linux", test))]
#[cfg_attr(test, allow(dead_code))]
pub(crate) use linux::named_icon_data_url;

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

    pub(super) fn extract_type(_extension: &str, _size: u32) -> Option<super::FileIcon> {
        None
    }
}

/// The icon the OS would show for `path` at `size` CSS pixels, or `None` when
/// it has none.
///
/// `is_dir` is a parameter rather than something read off the disk because the
/// caller has it already — the listing that made the row reported the entry's
/// kind, and a `stat` per icon request is the blocking read this whole pipeline
/// was moved off the UI thread to avoid. It matters on Linux, where a directory
/// resolves through a different icon context (`places`) than a file does
/// (`mimetypes`) — the two are not interchangeable and guessing from the path
/// would be wrong for a symlink to either. It matters on Windows and macOS for
/// the opposite reason: nothing is guessed there, because a folder's own icon is
/// whatever the user dropped on it, and only the path can name it.
pub(crate) fn extract(path: &str, size: u32, is_dir: bool) -> Option<FileIcon> {
    backend::extract(path, size, is_dir)
}

/// The icon the OS would show for a whole *type* at `size`, which is the same
/// answer for every file sharing `extension`.
///
/// No path reaches this function, and that is the point: a request that names a
/// type is asking to share an answer with every other row of that type, so the
/// URL carries the extension alone, the webview caches one response for the whole
/// column, and the desktop is asked once. Each backend resolves the type through
/// whatever mechanism it can drive from a name — the theme's mimetype context,
/// the registered class icon, LaunchServices — none of which needs the file to
/// exist.
pub(crate) fn extract_type(extension: &str, size: u32) -> Option<FileIcon> {
    backend::extract_type(extension, size)
}

/// The theme's own icon for a name — a `.desktop`'s `Icon=` value — at `size`.
///
/// Linux only. The other two platforms key their icons on a file, and a name
/// that is not one means nothing to their shells, so the request goes unanswered
/// and the row keeps the glyph the frontend draws.
#[cfg(target_os = "linux")]
pub(crate) fn named(name: &str, size: u32) -> Option<FileIcon> {
    linux::resolve_named_icon(name, size)
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn named(_name: &str, _size: u32) -> Option<FileIcon> {
    None
}
