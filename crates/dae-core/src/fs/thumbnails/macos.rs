//! macOS: QuickLook's generator, reached through `qlmanage`.
//!
//! `qlmanage -t` is the command-line face of the same QuickLook generator
//! Finder uses for PDF first pages, video posters and HEIC/AVIF photos the
//! `image` crate cannot decode. It writes one PNG per input into an output
//! directory; this backend gives it a private directory, waits with a bounded
//! deadline, and returns the first PNG.
//!
//! The command is intentionally run out of process: QuickLook may load a
//! third-party generator for a file type, and a generator that hangs must not
//! hold the Qt image-loading thread or the preview pipeline. A miss is normal
//! (nothing claims the type, or the TCC prompt denied the folder) and the view
//! keeps its type glyph.

use super::Thumbnail;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How long one QuickLook request may take before the child is killed. A PDF
/// first page is usually well under 150 ms; five seconds is long enough for a
/// cold generator load and short enough to not stall a preview.
const TIMEOUT: Duration = Duration::from_secs(5);

/// A private output directory removed when the request finishes, including on
/// early returns.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(super) fn extract(path: &str, size: u32, _modified: Option<SystemTime>) -> Option<Thumbnail> {
    let scratch = Scratch::new()?;
    let output = scratch.0.as_path();
    let pixel_size = size.clamp(32, 1024).to_string();

    let mut child = Command::new("/usr/bin/qlmanage")
        .args(["-t", "-s", &pixel_size, "-o"])
        .arg(output)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let deadline = Instant::now() + TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }

    let png = first_png(output)?;
    let bytes = fs::read(png).ok()?;
    // A generator that writes an error placeholder still produces a file; only
    // a real PNG is a thumbnail.
    bytes
        .starts_with(b"\x89PNG\r\n\x1a\n")
        .then_some(Thumbnail {
            mime: "image/png",
            bytes,
        })
}

impl Scratch {
    fn new() -> Option<Self> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let path =
            std::env::temp_dir().join(format!("dae-quicklook-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&path).ok()?;
        Some(Self(path))
    }
}

/// The first PNG `qlmanage` wrote into `directory`.
fn first_png(directory: &Path) -> Option<PathBuf> {
    fs::read_dir(directory)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
        })
}
