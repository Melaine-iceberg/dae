//! The `fileicon://` image provider and the URLs the list models hand to it.
//!
//! QML asks for `image://fileicon/path/<path>?dir=1&size=...&v=...` for an
//! entry whose icon is its own (a folder with a custom icon, a shortcut, an
//! executable), and `image://fileicon/type/<ext>?size=...` for a file whose icon
//! is shared by every file with its extension. The second form is the
//! performance argument: a column of 800 `.rs` files asks for one URL, QML
//! caches one texture, and the desktop is asked once.
//!
//! Extraction itself is in `dae_core::fs::icons`; this module is the engine
//! seam. Requests arrive on Qt's image-loading thread, so they are cached here
//! with both answers remembered — an extension no shell or theme claims is the
//! ordinary case in a folder of unfamiliar files, and an unremembered miss
//! repeats the entire theme search for each row that arrives.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Icons kept before the oldest answer is dropped. A listing shows a few dozen
/// distinct types; the path-shaped entries (folders with custom icons,
/// shortcuts) are what make this larger than a screenful.
const CACHE_ENTRIES: usize = 512;

/// The default icon edge in physical pixels, used only when QML did not say.
const DEFAULT_EDGE: u32 = 36;

/// Extensions whose icon belongs to the individual file rather than its type.
/// A shortcut resolves its target's icon, and an executable carries its own, so
/// sharing one answer per extension would draw the wrong thing.
const PER_FILE_EXTENSIONS: &[&str] = &["exe", "lnk", "url", "ico", "appref-ms", "msi"];

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qqmlapplicationengine.h");
        type QQmlApplicationEngine = cxx_qt_lib::QQmlApplicationEngine;

        include!("image_provider.h");
        #[namespace = "dae_qt"]
        #[rust_name = "register_file_icon_provider"]
        unsafe fn dae_qt_register_file_icon_provider(engine: *mut QQmlApplicationEngine);
    }

    extern "Rust" {
        /// The OS icon for one path. `version` is the listing's `mtime-size`, so
        /// a folder whose custom icon changed lands on a new cache key.
        fn dae_qt_render_file_icon(path: &str, is_dir: bool, size: i32, version: &str) -> Vec<u8>;

        /// The OS icon shared by every file with `extension`.
        fn dae_qt_render_type_icon(extension: &str, size: i32) -> Vec<u8>;

        /// A launcher's icon name, as found in a `.desktop` entry. Linux only;
        /// the other shells key icons on a file, so this answers empty and the
        /// open-with row keeps its fallback glyph.
        fn dae_qt_render_named_icon(names: &str, size: i32) -> Vec<u8>;

        /// A desktop-produced thumbnail (PDF page, video frame, HEIC photo).
        /// `modified_ms` is the source's mtime as the provider's `QFileInfo`
        /// read it: Linux's shared thumbnail cache is invalidated by comparing
        /// the cached file with the source, and the provider already paid for
        /// that stat. The image provider's own cache version-tags these, so
        /// they are not kept a second time here.
        fn dae_qt_render_system_thumbnail(path: &str, size: i32, modified_ms: i64) -> Vec<u8>;
    }
}

/// Registers the provider before the QML root is loaded.
pub fn register_provider(engine: *mut cxx_qt_lib::QQmlApplicationEngine) {
    unsafe {
        qobject::register_file_icon_provider(engine);
    }
}

/// The `image://fileicon/...` URL for a listing row.
///
/// `version` only reaches the cache key of path-shaped icons. Type icons are
/// shared by construction, so versioning them would turn one request per
/// extension into one per file.
pub(crate) fn source_for(path: &str, is_dir: bool, version: &str) -> String {
    let extension = extension_of(path);
    if is_dir || PER_FILE_EXTENSIONS.contains(&extension.as_str()) {
        format!(
            "image://fileicon/path/{}?dir={}&size={DEFAULT_EDGE}&v={}",
            encode(path),
            u8::from(is_dir),
            encode(version)
        )
    } else {
        format!(
            "image://fileicon/type/{}?size={DEFAULT_EDGE}",
            encode(&extension)
        )
    }
}

/// The `image://thumbnail/...` URL for a listing row's first frame.
///
/// The provider caches by the revision it is given, so the listing's
/// `mtime-size` doubles as the cache key: a file edited on disk is not served
/// from the frame decoded before the edit.
pub(crate) fn thumbnail_source(path: &str, version: &str) -> String {
    format!(
        "image://thumbnail/{}?size=96&v={}",
        encode(path),
        encode(version)
    )
}

fn dae_qt_render_file_icon(path: &str, is_dir: bool, size: i32, version: &str) -> Vec<u8> {
    let size = clamp_edge(size);
    let key = format!("path|{path}|{is_dir}|{version}|{size}");
    cached(&key, || {
        dae_core::fs::icons::extract(path, size, is_dir).map(|icon| icon.bytes)
    })
    .unwrap_or_default()
}

fn dae_qt_render_type_icon(extension: &str, size: i32) -> Vec<u8> {
    let size = clamp_edge(size);
    let extension = extension.trim_start_matches('.').to_ascii_lowercase();
    let key = format!("type|{extension}|{size}");
    cached(&key, || {
        dae_core::fs::icons::extract_type(&extension, size).map(|icon| icon.bytes)
    })
    .unwrap_or_default()
}

fn dae_qt_render_named_icon(names: &str, size: i32) -> Vec<u8> {
    let size = clamp_edge(size);
    let key = format!("name|{names}|{size}");
    cached(&key, || {
        dae_core::fs::icons::named(names, size).map(|icon| icon.bytes)
    })
    .unwrap_or_default()
}

fn dae_qt_render_system_thumbnail(path: &str, size: i32, modified_ms: i64) -> Vec<u8> {
    dae_core::fs::thumbnails::extract(path, clamp_edge(size), modified_from_ms(modified_ms))
        .map(|thumbnail| thumbnail.bytes)
        .unwrap_or_default()
}

/// The mtime `QFileInfo` reports, as the `SystemTime` the freshness rule
/// compares against. A negative stamp cannot name a time on this side of the
/// epoch, and a value no clock can represent is treated as absent rather than
/// guessed at — a corrupt timestamp must not panic the image-loading thread.
fn modified_from_ms(milliseconds: i64) -> Option<SystemTime> {
    u64::try_from(milliseconds)
        .ok()
        .and_then(|milliseconds| UNIX_EPOCH.checked_add(Duration::from_millis(milliseconds)))
}

fn clamp_edge(size: i32) -> u32 {
    u32::try_from(size).unwrap_or(DEFAULT_EDGE).clamp(16, 256)
}

fn extension_of(path: &str) -> String {
    std::path::Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// Percent-encodes everything outside the unreserved set plus `/`, so a path
/// survives QML's URL handling and `QUrl::fromPercentEncoding` restores it
/// byte for byte. `+` is encoded too: the decoder does not read it as a space.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for &byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

struct Cache {
    entries: HashMap<String, Option<Vec<u8>>>,
    order: VecDeque<String>,
}

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(Cache {
            entries: HashMap::new(),
            order: VecDeque::new(),
        })
    })
}

fn cached(key: &str, produce: impl FnOnce() -> Option<Vec<u8>>) -> Option<Vec<u8>> {
    let mut cache = cache().lock().unwrap_or_else(|error| error.into_inner());
    if let Some(hit) = cache.entries.get(key) {
        return hit.clone();
    }

    let value = produce();
    if cache.entries.len() >= CACHE_ENTRIES
        && let Some(oldest) = cache.order.pop_front()
    {
        cache.entries.remove(&oldest);
    }
    cache.order.push_back(key.to_owned());
    cache.entries.insert(key.to_owned(), value.clone());
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_urls_share_one_request_per_extension() {
        let first = source_for("C:/tree/src/main.rs", false, "1-2");
        let second = source_for("C:/tree/src/lib.rs", false, "3-4");
        assert_eq!(first, second);
        assert_eq!(first, "image://fileicon/type/rs?size=36");
    }

    #[test]
    fn per_file_urls_carry_a_version() {
        let url = source_for("C:/tree/run.exe", false, "11-22");
        assert_eq!(
            url,
            "image://fileicon/path/C%3A/tree/run.exe?dir=0&size=36&v=11-22"
        );
    }

    #[test]
    fn directories_and_shortcuts_stay_per_file() {
        assert!(source_for("C:/tree/docs", true, "1-0").contains("/path/"));
        assert!(source_for("C:/tree/app.lnk", false, "1-0").contains("/path/"));
    }

    #[test]
    fn encoding_round_trips_every_byte_the_decoder_cares_about() {
        assert_eq!(
            encode("C:/a b/你好?.txt"),
            "C%3A/a%20b/%E4%BD%A0%E5%A5%BD%3F.txt"
        );
        assert_eq!(encode("plain/path_1.2~3"), "plain/path_1.2~3");
    }

    #[test]
    fn misses_are_remembered_too() {
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let key = "test|remembered-miss";
        assert_eq!(
            cached(key, || {
                calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                None
            }),
            None
        );
        assert_eq!(
            cached(key, || {
                calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                None
            }),
            None
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
    }
}
