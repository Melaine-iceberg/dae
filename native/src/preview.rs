//! Text preview for the explorer's preview panel.
//!
//! The renderer only needs the first few kilobytes of a file to paint its
//! preview, so this reads a bounded prefix instead of the whole file. Decoding
//! is lossy: a multi-byte character cut at the read boundary must not fail the
//! preview.

use dae_core::file_system::error::FileSystemError;
use serde::Serialize;
use std::fs;
use std::io::Read;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextPreview {
    pub content: String,
    pub truncated: bool,
}

/// Reads at most `max_bytes` from the start of `path`. Callers run it on the
/// blocking pool.
pub fn read_text_preview(path: String, max_bytes: u32) -> Result<TextPreview, FileSystemError> {
    let metadata = fs::metadata(&path).map_err(FileSystemError::from)?;
    if !metadata.is_file() {
        return Err(FileSystemError::InvalidInput(path));
    }

    let limit = max_bytes.clamp(1, 256 * 1024) as usize;
    let file_size = metadata.len() as usize;
    let truncated = file_size > limit;
    let bytes_to_read = file_size.min(limit);

    let mut buffer = vec![0u8; bytes_to_read];
    let mut file = fs::File::open(&path).map_err(FileSystemError::from)?;
    file.read_exact(&mut buffer)
        .map_err(FileSystemError::from)?;

    Ok(TextPreview {
        content: String::from_utf8_lossy(&buffer).into_owned(),
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_at_the_requested_byte_limit() {
        let path = std::env::temp_dir().join(format!("dae-text-preview-{}.txt", std::process::id()));
        fs::write(&path, b"abcdefghij").expect("write test file");

        let preview = read_text_preview(path.to_string_lossy().into_owned(), 4).expect("preview");
        assert_eq!(preview.content, "abcd");
        assert!(preview.truncated);

        let whole = read_text_preview(path.to_string_lossy().into_owned(), 64).expect("preview");
        assert_eq!(whole.content, "abcdefghij");
        assert!(!whole.truncated);

        fs::remove_file(path).ok();
    }
}
