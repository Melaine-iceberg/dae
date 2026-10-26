//! Engine-free preview helpers.
//!
//! Only the parts both shells can share live here: the bounded UTF-8 text peek
//! and the extension rules that decide whether a file has a text preview or an
//! image thumbnail. Image decoding itself stays in the shell that draws it.

use super::error::FileSystemError;
use serde::Serialize;
use specta::Type;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TextPreview {
    pub content: String,
    pub truncated: bool,
}

/// Files with a plain-text peek.
const TEXT_EXTENSIONS: &[&str] = &[
    "txt",
    "log",
    "csv",
    "tsv",
    "bat",
    "cmd",
    "env",
    "properties",
    "diff",
    "patch",
    "lock",
    "md",
    "markdown",
    "mdx",
    "rst",
    "adoc",
    "org",
    "json",
    "jsonc",
    "json5",
    "yaml",
    "yml",
    "toml",
    "ini",
    "conf",
    "cfg",
    "cnf",
    "xml",
    "html",
    "htm",
    "xhtml",
    "css",
    "scss",
    "sass",
    "less",
    "styl",
    "js",
    "jsx",
    "mjs",
    "cjs",
    "ts",
    "tsx",
    "mts",
    "cts",
    "vue",
    "svelte",
    "astro",
    "py",
    "pyi",
    "rs",
    "go",
    "java",
    "kt",
    "kts",
    "scala",
    "groovy",
    "gradle",
    "c",
    "h",
    "cc",
    "cpp",
    "cxx",
    "hpp",
    "hxx",
    "m",
    "mm",
    "cs",
    "fs",
    "vb",
    "php",
    "rb",
    "swift",
    "dart",
    "lua",
    "r",
    "jl",
    "ex",
    "exs",
    "erl",
    "hrl",
    "hs",
    "ml",
    "mli",
    "clj",
    "cljs",
    "sh",
    "bash",
    "zsh",
    "fish",
    "ps1",
    "psm1",
    "sql",
    "graphql",
    "gql",
    "proto",
    "thrift",
    "tf",
    "hcl",
    "nix",
    "cmake",
    "mk",
    "tex",
    "bib",
    "sty",
    "ipynb",
    "srt",
    "vtt",
    "gitignore",
    "gitattributes",
    "editorconfig",
    "dockerignore",
    "npmrc",
    "prettierrc",
    "eslintrc",
];

/// Extensionless files that are still worth showing as text.
const TEXT_FILE_NAMES: &[&str] = &[
    "dockerfile",
    "makefile",
    "rakefile",
    "gemfile",
    "procfile",
    "license",
    "licence",
    "readme",
    "changelog",
    "authors",
    "notice",
    "cargo.lock",
    "cmakelists.txt",
];

/// Image formats the Qt shell can decode in process. Desktop-produced
/// thumbnails (PDF first page, video frame) are a shell decision and are not
/// listed here.
const THUMBNAIL_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff", "ico", "svg", "avif", "jxl",
];

/// Reads at most `max_bytes` of a file as lossy UTF-8.
///
/// Lossy decoding is deliberate: a multi-byte character cut by the byte limit
/// should not fail the whole preview. The limit is clamped so a caller cannot
/// ask for an unbounded read.
pub async fn read_text_preview(
    path: String,
    max_bytes: u32,
) -> Result<TextPreview, FileSystemError> {
    tokio::task::spawn_blocking(move || {
        let metadata = fs::metadata(&path).map_err(FileSystemError::from)?;
        if !metadata.is_file() {
            return Err(FileSystemError::InvalidInput(path));
        }

        let limit = max_bytes.clamp(1, 256 * 1024) as usize;
        let file_size = metadata.len();
        let truncated = file_size > limit as u64;
        let bytes_to_read = file_size.min(limit as u64) as usize;

        let mut buffer = vec![0u8; bytes_to_read];
        let mut file = fs::File::open(&path).map_err(FileSystemError::from)?;
        use std::io::Read;
        file.read_exact(&mut buffer)
            .map_err(FileSystemError::from)?;

        Ok(TextPreview {
            content: String::from_utf8_lossy(&buffer).into_owned(),
            truncated,
        })
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

pub fn is_text_preview_path(path: &str) -> bool {
    let name = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if TEXT_FILE_NAMES.contains(&name.as_str()) {
        return true;
    }
    // `Path::extension` answers `None` for dotfiles, so `.gitignore` and
    // `.editorconfig` need the bare name checked after the leading dot.
    if let Some(dot_name) = name.strip_prefix('.') {
        if TEXT_EXTENSIONS.contains(&dot_name) {
            return true;
        }
    }
    let extension = Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    TEXT_EXTENSIONS.contains(&extension.as_str())
}

pub fn is_thumbnail_path(path: &str) -> bool {
    let extension = Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    THUMBNAIL_EXTENSIONS.contains(&extension.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_text_by_extension_and_well_known_name() {
        assert!(is_text_preview_path("C:/tree/src/main.rs"));
        assert!(is_text_preview_path("C:/tree/README"));
        assert!(is_text_preview_path("C:/tree/Dockerfile"));
        assert!(is_text_preview_path("C:/tree/.gitignore"));
        assert!(!is_text_preview_path("C:/tree/photo.jpg"));
        assert!(!is_text_preview_path("C:/tree/archive.zip"));
    }

    #[test]
    fn classifies_thumbnails_by_extension() {
        assert!(is_thumbnail_path("C:/tree/photo.JPG"));
        assert!(is_thumbnail_path("C:/tree/icon.svg"));
        assert!(!is_thumbnail_path("C:/tree/main.rs"));
        assert!(!is_thumbnail_path("C:/tree/report.pdf"));
    }

    #[tokio::test]
    async fn reads_a_bounded_prefix_and_flags_truncation() {
        let path = std::env::temp_dir().join(format!(
            "dae-preview-test-{}-{}.txt",
            std::process::id(),
            line!()
        ));
        fs::write(&path, "hello preview").expect("write fixture");

        let preview = read_text_preview(path.to_string_lossy().into_owned(), 5)
            .await
            .expect("read preview");
        assert_eq!(preview.content, "hello");
        assert!(preview.truncated);

        let complete = read_text_preview(path.to_string_lossy().into_owned(), 1024)
            .await
            .expect("read complete preview");
        assert_eq!(complete.content, "hello preview");
        assert!(!complete.truncated);

        fs::remove_file(path).expect("remove fixture");
    }

    #[tokio::test]
    async fn a_cut_multibyte_character_does_not_fail_the_preview() {
        let path = std::env::temp_dir().join(format!(
            "dae-preview-utf8-test-{}-{}.txt",
            std::process::id(),
            line!()
        ));
        fs::write(&path, "你好世界").expect("write fixture");

        let preview = read_text_preview(path.to_string_lossy().into_owned(), 4)
            .await
            .expect("read preview");
        assert!(preview.truncated);
        assert!(preview.content.starts_with('你'));

        fs::remove_file(path).expect("remove fixture");
    }
}
