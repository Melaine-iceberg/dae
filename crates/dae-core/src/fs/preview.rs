//! Engine-free preview helpers.
//!
//! Only the parts both shells can share live here: the bounded text peek and
//! the rules that decide whether a file has a text preview or an image
//! thumbnail. Image decoding itself stays in the shell that draws it.
//!
//! Whether a file is text is decided by its bytes, not by its name. An
//! allowlist of extensions would miss every source file whose suffix nobody
//! listed — and the same read already happens, so sniffing costs nothing.

use super::error::FileSystemError;
use super::types::EntryKind;
use super::vfs;
use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};
use encoding_rs::{Encoding, UTF_16BE, UTF_16LE};
use serde::Serialize;
use std::fs;
use std::io::Read;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextPreview {
    pub content: String,
    pub truncated: bool,
}

/// Extensions whose files are never text: containers another pane owns, media
/// the player handles, and the binary formats of programs, documents and
/// images. This is a fast reject, not the classifier — an extension outside it
/// is decided by its bytes in [`read_text_preview`].
///
/// Covers images (including the ones the thumbnail provider decodes), audio and
/// video, archives and packages, executables and build products, documents
/// another pane renders, fonts, and databases. `ts` is absent on purpose: a
/// TypeScript file outweighs an MPEG-TS stream in a file manager.
const BINARY_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff", "ico", "svg", "avif", "jxl", "heic",
    "heif", "psd", "ai", "eps", "raw", "cr2", "nef", "dng", "mp3", "flac", "wav", "m4a", "aac",
    "ogg", "opus", "wma", "ape", "mp4", "m4v", "mov", "mkv", "webm", "avi", "wmv", "flv", "mpg",
    "mpeg", "m2ts", "3gp", "zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz", "zst", "lz4",
    "lzma", "cab", "iso", "dmg", "msix", "appx", "deb", "rpm", "apk", "jar", "war", "pkg", "exe",
    "dll", "so", "dylib", "sys", "msi", "com", "scr", "obj", "lib", "pdb", "pyc", "pyo", "class",
    "wasm", "node", "pdf", "doc", "docx", "docm", "dot", "dotx", "xls", "xlsx", "xlsm", "xlsb",
    "xlt", "ppt", "pptx", "pptm", "pot", "pps", "odt", "ods", "odp", "rtf", "ttf", "otf", "woff",
    "woff2", "eot", "ttc", "sqlite", "sqlite3", "db", "mdb", "accdb", "parquet", "arrow",
    "feather", "hdf5", "h5", "npy", "npz", "pkl", "pickle",
];

/// Names without an extension that are still machine data.
const BINARY_FILE_NAMES: &[&str] = &[".ds_store", "thumbs.db"];

/// Image formats the Qt shell can decode in process. Desktop-produced
/// thumbnails (PDF first page, video frame) are a shell decision and are not
/// listed here.
const THUMBNAIL_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff", "ico", "svg", "avif", "jxl",
];

/// Control bytes that appear in text: tab, line breaks, form feed, backspace
/// and the ESC that starts an ANSI sequence in a log.
const TEXT_CONTROL_BYTES: &[u8] = b"\x08\t\n\x0b\x0c\r\x1b";

/// Reads at most `max_bytes` of a file and decodes it as text.
///
/// `None` when the bytes are a binary container. The limit is clamped so a
/// caller cannot ask for an unbounded read, and it applies before decoding, so
/// a huge file costs one bounded read. Local files keep the symlink-following
/// `fs::metadata` size check; network files get the same bounded read through
/// their VFS backend, so a preview never downloads a file just to show its
/// first page.
pub async fn read_text_preview(
    path: String,
    max_bytes: u32,
) -> Result<Option<TextPreview>, FileSystemError> {
    tokio::task::spawn_blocking(move || {
        let limit = max_bytes.clamp(1, 256 * 1024) as u64;
        let backend = vfs::resolve(&path)?;

        let (file_size, is_directory) = if vfs::is_local_path(&path) {
            let metadata = fs::metadata(&path).map_err(FileSystemError::from)?;
            (metadata.len(), metadata.is_dir())
        } else {
            let stat = backend.stat(&path)?;
            (stat.size, stat.kind == EntryKind::Directory)
        };
        if is_directory {
            return Err(FileSystemError::InvalidInput(path));
        }

        let truncated = file_size > limit;
        let mut buffer = Vec::new();
        backend
            .open_read(&path)?
            .take(limit)
            .read_to_end(&mut buffer)
            .map_err(FileSystemError::from)?;

        Ok(decode_text(&buffer).map(|content| TextPreview { content, truncated }))
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// Whether the name is worth a read at all. A file outside the fast-reject
/// list is *probably* text; [`read_text_preview`] has the last word.
pub fn may_have_text_preview(path: &str) -> bool {
    let name = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if BINARY_FILE_NAMES.contains(&name.as_str()) {
        return false;
    }
    let extension = Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    !BINARY_EXTENSIONS.contains(&extension.as_str())
}

pub fn is_thumbnail_path(path: &str) -> bool {
    let extension = Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    THUMBNAIL_EXTENSIONS.contains(&extension.as_str())
}

/// Decodes a bounded read as text, or answers `None` for a binary container.
///
/// The checks run cheapest-first: a byte-order mark names the encoding, a NUL
/// byte means a container (except in the alternating lane a BOM-less UTF-16
/// file leaves), a control-byte density over 5 % means the same, and only then
/// is the encoding guessed. A failed guess still decodes instead of refusing:
/// a Windows `.txt` or a GBK source file is text whatever its charset.
fn decode_text(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return Some(String::new());
    }
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return Some(decode_utf8(rest));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return Some(decode_utf16(rest, UTF_16LE));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return Some(decode_utf16(rest, UTF_16BE));
    }
    if let Some(encoding) = utf16_without_bom(bytes) {
        return Some(decode_utf16(bytes, encoding));
    }
    if looks_binary(bytes) {
        return None;
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => return Some(text.to_owned()),
        // A limit that cut a multi-byte character is not a charset: the bytes
        // before the incomplete sequence are UTF-8, and they are the peek.
        Err(error) if error.error_len().is_none() => {
            return Some(String::from_utf8_lossy(&bytes[..error.valid_up_to()]).into_owned());
        }
        Err(_) => {}
    }
    let mut detector = EncodingDetector::new(Iso2022JpDetection::Allow);
    detector.feed(bytes, true);
    let encoding = detector.guess(None, Utf8Detection::Deny);
    let (text, _) = encoding.decode_without_bom_handling(bytes);
    Some(text.into_owned())
}

/// UTF-8 with a lossy fallback for the character the limit cut in half.
fn decode_utf8(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(error) if error.error_len().is_none() => {
            String::from_utf8_lossy(&bytes[..error.valid_up_to()]).into_owned()
        }
        Err(_) => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// A UTF-16 unit split by the byte limit decodes as a replacement character;
/// dropping the odd tail keeps the last line clean instead.
fn decode_utf16(bytes: &[u8], encoding: &'static Encoding) -> String {
    let even = bytes.len() - bytes.len() % 2;
    let (text, _) = encoding.decode_without_bom_handling(&bytes[..even]);
    text.into_owned()
}

/// Windows tools write UTF-16 without a BOM, where every ASCII character
/// leaves a zero in one lane. A file with zeros in one lane and almost none in
/// the other is that file, not a container.
fn utf16_without_bom(bytes: &[u8]) -> Option<&'static Encoding> {
    const LANE_SAMPLES: usize = 4096;
    if bytes.len() < 8 {
        return None;
    }
    let sample = &bytes[..bytes.len().min(LANE_SAMPLES * 2)];
    let pairs = sample.len() / 2;
    let mut even_zeros = 0usize;
    let mut odd_zeros = 0usize;
    for pair in sample.chunks_exact(2) {
        if pair[0] == 0 {
            even_zeros += 1;
        }
        if pair[1] == 0 {
            odd_zeros += 1;
        }
    }
    let mostly_in_lane = |zeros: usize| zeros * 10 >= pairs * 6;
    let rarely_in_lane = |zeros: usize| zeros * 10 <= pairs;
    if mostly_in_lane(odd_zeros) && rarely_in_lane(even_zeros) {
        return Some(UTF_16LE);
    }
    if mostly_in_lane(even_zeros) && rarely_in_lane(odd_zeros) {
        return Some(UTF_16BE);
    }
    None
}

fn looks_binary(bytes: &[u8]) -> bool {
    let mut control = 0usize;
    for &byte in bytes {
        if byte == 0 {
            return true;
        }
        if (byte < 0x20 && !TEXT_CONTROL_BYTES.contains(&byte)) || byte == 0x7F {
            control += 1;
        }
    }
    control * 20 > bytes.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(label: &str, name: &str) -> std::path::PathBuf {
        let directory =
            std::env::temp_dir().join(format!("dae-preview-test-{}-{}", std::process::id(), label));
        fs::create_dir_all(&directory).expect("create fixture directory");
        directory.join(name)
    }

    #[test]
    fn rejects_containers_by_extension_and_accepts_the_unknown_suffix() {
        assert!(may_have_text_preview("C:/tree/src/main.rs"));
        assert!(may_have_text_preview("C:/tree/README"));
        assert!(may_have_text_preview("C:/tree/notes.unknownext"));
        assert!(may_have_text_preview("C:/tree/types.ts"));
        assert!(!may_have_text_preview("C:/tree/photo.jpg"));
        assert!(!may_have_text_preview("C:/tree/archive.zip"));
        assert!(!may_have_text_preview("C:/tree/.DS_Store"));
    }

    #[test]
    fn classifies_thumbs_by_extension() {
        assert!(is_thumbnail_path("C:/tree/photo.JPG"));
        assert!(is_thumbnail_path("C:/tree/icon.svg"));
        assert!(!is_thumbnail_path("C:/tree/main.rs"));
        assert!(!is_thumbnail_path("C:/tree/report.pdf"));
    }

    #[tokio::test]
    async fn reads_a_bounded_prefix_and_flags_truncation() {
        let path = temp_path("bounded", "notes.txt");
        fs::write(&path, "hello preview").expect("write fixture");

        let preview = read_text_preview(path.to_string_lossy().into_owned(), 5)
            .await
            .expect("read preview")
            .expect("text preview");
        assert_eq!(preview.content, "hello");
        assert!(preview.truncated);

        let complete = read_text_preview(path.to_string_lossy().into_owned(), 1024)
            .await
            .expect("read complete preview")
            .expect("text preview");
        assert_eq!(complete.content, "hello preview");
        assert!(!complete.truncated);

        fs::remove_dir_all(path.parent().unwrap()).expect("remove fixture");
    }

    #[tokio::test]
    async fn a_cut_multibyte_character_does_not_fail_the_preview() {
        let path = temp_path("utf8", "notes.txt");
        fs::write(&path, "你好世界").expect("write fixture");

        let preview = read_text_preview(path.to_string_lossy().into_owned(), 4)
            .await
            .expect("read preview")
            .expect("text preview");
        assert!(preview.truncated);
        assert_eq!(preview.content, "你");

        fs::remove_dir_all(path.parent().unwrap()).expect("remove fixture");
    }

    #[tokio::test]
    async fn an_unlisted_extension_is_decided_by_its_bytes() {
        let text_path = temp_path("unknown-text", "MODULE.WEIRD");
        fs::write(&text_path, "fn main() {}\n").expect("write fixture");
        let preview = read_text_preview(text_path.to_string_lossy().into_owned(), 4096)
            .await
            .expect("read preview")
            .expect("plain text is previewed");
        assert_eq!(preview.content, "fn main() {}\n");

        let binary_path = temp_path("unknown-binary", "BLOB.WEIRD");
        fs::write(&binary_path, b"\x7fELF\x02\x01\x01\x00\x00\x00\x00\x00").expect("write fixture");
        let preview = read_text_preview(binary_path.to_string_lossy().into_owned(), 4096)
            .await
            .expect("read preview");
        assert!(preview.is_none(), "a container has no text preview");

        fs::remove_dir_all(text_path.parent().unwrap()).expect("remove fixture");
        fs::remove_dir_all(binary_path.parent().unwrap()).expect("remove fixture");
    }

    #[tokio::test]
    async fn decodes_utf16_and_legacy_encodings() {
        let utf16 = temp_path("utf16", "notes.txt");
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "你好 world".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        fs::write(&utf16, &bytes).expect("write fixture");
        let preview = read_text_preview(utf16.to_string_lossy().into_owned(), 4096)
            .await
            .expect("read preview")
            .expect("utf-16 is text");
        assert_eq!(preview.content, "你好 world");

        let gbk = temp_path("gbk", "notes.txt");
        let (encoded, _, _) = encoding_rs::GBK.encode("你好，世界");
        fs::write(&gbk, &encoded).expect("write fixture");
        let preview = read_text_preview(gbk.to_string_lossy().into_owned(), 4096)
            .await
            .expect("read preview")
            .expect("gbk is text");
        assert_eq!(preview.content, "你好，世界");

        fs::remove_dir_all(utf16.parent().unwrap()).expect("remove fixture");
        fs::remove_dir_all(gbk.parent().unwrap()).expect("remove fixture");
    }
}
