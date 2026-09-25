//! macOS: the icon the Finder would draw, through `NSWorkspace`.
//!
//! The cheapest of the three backends, because AppKit answers the question
//! directly — `iconForFile:` returns the composed icon, badges and custom
//! `.icon` resources and all, for a file or a folder alike. What is left is
//! turning an `NSImage` into bytes a webview can consume, and that has two rules
//! of its own worth stating, because both look skippable and neither is.
//!
//!   * **Pick a representation; do not encode the image.** `iconForFile:` hands
//!     back the whole multi-rep set an `.icns` carries — 16, 32, 48, 128, 256,
//!     512, 1024 and the `@2x` variants — and encoding the *image* means
//!     encoding its nominal size, which for a modern icon is 1024. At four
//!     megabytes of RGBA per icon, a directory of unfamiliar file types turns
//!     into a multi-second stall on the render pool.
//!   * **Transcode from TIFF; do not draw.** The direct route is
//!     `bitmapDataForType:`, and `objc2-app-kit` does not bind it, so the choice
//!     is between binding it by hand and using the one encoder that *is* bound.
//!     TIFF is lossless, `image` already has a TIFF decoder on for the thumbnail
//!     path, and the pixel format is AppKit's own. The alternative —
//!     `bitmapImageRepForCachingDisplayInRect:` and drawing into a bitmap —
//!     would put AppKit's graphics context on a non-main thread, which is the one
//!     thing here that could genuinely misbehave rather than merely be slow.

use super::FileIcon;
use image::DynamicImage;
use objc2::rc::Retained;
use objc2_app_kit::{NSBitmapImageRep, NSImage, NSWorkspace};
use objc2_foundation::NSString;
use std::io::Cursor;

/// How much worse an undersize representation is than an oversize one, per
/// pixel short. Multiplied rather than added so it can outvote any width the
/// theme actually ships.
const UPSCALE_PENALTY: i64 = 8;

/// The icon Finder would show for this path.
///
/// `is_dir` is unused: `iconForFile:` resolves a folder through the same call as
/// a file, which is one of the things that makes this the easy backend.
pub(super) fn extract(path: &str, size: u32, _is_dir: bool) -> Option<FileIcon> {
    let icon = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(path));
    let bitmap = best_representation(&icon, i64::from(size))?;
    let tiff = bitmap.TIFFRepresentation()?;

    let mut rendered = DynamicImage::from(image::load_from_memory(&tiff.to_vec()).ok()?.to_rgba8());

    // Only reachable for an icon that ships nothing but a giant representation.
    // Halving the bytes on the way out, at the 2x the frontend asked for so a
    // HiDPI display still gets real pixels rather than a resample of a resample.
    let target = size * 2;
    if rendered.width() > target {
        rendered = rendered.resize_exact(target, target, image::imageops::FilterType::Lanczos3);
    }

    let mut bytes: Vec<u8> = Vec::new();
    rendered
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .ok()?;
    if bytes.is_empty() {
        return None;
    }

    Some(FileIcon {
        mime: "image/png",
        bytes,
    })
}

/// The representation whose pixel width best fits `requested`.
///
/// `None` when the icon has no bitmap representation at all — a PDF-only icon,
/// which a hand-made `.icon` folder produces, carries `NSPDFImageRep`s, and
/// rendering one means drawing. The frontend keeps its own glyph for those, and
/// they are rare enough that trading them for a guaranteed-safe worker thread is
/// the right way round.
fn best_representation(icon: &NSImage, requested: i64) -> Option<Retained<NSBitmapImageRep>> {
    let mut best: Option<(Retained<NSBitmapImageRep>, i64)> = None;

    for rep in icon.representations().iter() {
        let Ok(bitmap) = rep.downcast::<NSBitmapImageRep>() else {
            continue;
        };
        let cost = cost_of(bitmap.pixelsWide() as i64, requested);
        if best.as_ref().is_none_or(|(_, best_cost)| cost < *best_cost) {
            best = Some((bitmap, cost));
        }
    }

    best.map(|(bitmap, _)| bitmap)
}

/// How wrong one representation width is for a request. Oversizing costs the
/// difference; undersizing costs it multiplied, because the result is soft.
fn cost_of(available: i64, requested: i64) -> i64 {
    if available >= requested {
        available - requested
    } else {
        (requested - available) * UPSCALE_PENALTY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ranking is the whole reason this file picks a representation instead
    /// of taking the image, so it is the thing to test — and it is arithmetic,
    /// so it can be tested on the machine it was written on rather than only on
    /// the one it runs on.
    #[test]
    fn prefers_the_tightest_fit_that_still_does_not_upscale() {
        // A 44px request (a 22px cell at 2x) against a stock .icns set.
        let widths = [16, 32, 48, 128, 256, 512, 1024];
        let best = widths
            .iter()
            .min_by_key(|&&width| cost_of(width, 44))
            .copied()
            .expect("the set is not empty");

        // 48 is the answer. 32 is *nearer* 44 in absolute terms and is the trap:
        // blowing a 32 up to 44 is the soft icon this exists to avoid.
        assert_eq!(best, 48);
        assert!(cost_of(32, 44) > cost_of(48, 44));
    }

    #[test]
    fn an_exact_match_beats_everything() {
        assert_eq!(cost_of(44, 44), 0);
        assert!(cost_of(44, 44) < cost_of(45, 44));
        assert!(cost_of(44, 44) < cost_of(43, 44));
    }

    #[test]
    fn asking_for_more_than_exists_takes_the_largest() {
        let widths = [16, 32, 48];
        let best = widths
            .iter()
            .max()
            .copied()
            .expect("the set is not empty");
        // Nothing can satisfy 128, so the ranking degrades monotonically toward
        // the widest option rather than picking an arbitrary member.
        assert_eq!(best, 48);
        assert!(cost_of(48, 128) < cost_of(32, 128));
        assert!(cost_of(32, 128) < cost_of(16, 128));
    }
}
