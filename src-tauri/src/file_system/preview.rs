use super::error::FileSystemError;
use base64::Engine as _;
use image::{DynamicImage, GenericImageView};
use serde::Serialize;
use specta::Type;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

/// Thumbnail payload as a ready-to-use `data:` URL (SKILL.md §17).
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Thumbnail {
    pub data_url: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TextPreview {
    pub content: String,
    pub truncated: bool,
}

/// Source images above this size are skipped so browsing a folder of huge
/// archives-as-images cannot stall the UI.
const THUMBNAIL_MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
/// Hard cap on decoded thumbnail pixels; larger images are downscaled in one
/// cheap `resize` step before the final smooth pass.
const THUMBNAIL_MAX_DECODED_PIXELS: u64 = 40 * 1024 * 1024;
const THUMBNAIL_CACHE_MAX_ENTRIES: usize = 256;

/// Capped in-memory cache keyed by path + mtime + size + target size. Entries
/// are cheap to regenerate, so eviction simply clears the whole map.
static THUMBNAIL_CACHE: Mutex<Option<HashMap<String, Thumbnail>>> = Mutex::new(None);

fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// Extensions the `image` crate can decode on every supported platform.
pub fn is_thumbnail_extension(path: &str) -> bool {
    matches!(
        extension_of(Path::new(path)).as_str(),
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "ico"
    )
}

#[tauri::command]
#[specta::specta]
pub async fn get_thumbnail(path: String, size: u16) -> Result<Option<Thumbnail>, FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || render_thumbnail(&path, size))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

fn render_thumbnail(path_string: &str, size: u16) -> Result<Option<Thumbnail>, FileSystemError> {
    let path = Path::new(path_string);
    if !is_thumbnail_extension(path_string) {
        return Ok(None);
    }

    let metadata = fs::metadata(path).map_err(FileSystemError::from)?;
    if !metadata.is_file() || metadata.len() > THUMBNAIL_MAX_SOURCE_BYTES {
        return Ok(None);
    }

    let modified_at = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64);
    let cache_key = format!(
        "{}|{}|{}|{size}",
        path_string,
        modified_at.unwrap_or(0),
        metadata.len()
    );

    if let Some(cached) = lookup_cache(&cache_key) {
        return Ok(Some(cached));
    }

    let Some(thumbnail) = decode_and_scale(path, size)? else {
        return Ok(None);
    };

    store_cache(cache_key, thumbnail.clone());
    Ok(Some(thumbnail))
}

fn lookup_cache(cache_key: &str) -> Option<Thumbnail> {
    THUMBNAIL_CACHE
        .lock()
        .ok()?
        .as_ref()?
        .get(cache_key)
        .cloned()
}

fn store_cache(cache_key: String, thumbnail: Thumbnail) {
    if let Ok(mut guard) = THUMBNAIL_CACHE.lock() {
        let cache = guard.get_or_insert_with(HashMap::new);
        if cache.len() >= THUMBNAIL_CACHE_MAX_ENTRIES {
            cache.clear();
        }
        cache.insert(cache_key, thumbnail);
    }
}

fn decode_and_scale(path: &Path, size: u16) -> Result<Option<Thumbnail>, FileSystemError> {
    let reader = image::ImageReader::open(path)
        .and_then(|reader| reader.with_guessed_format())
        .map_err(|error| FileSystemError::Io(error.to_string()))?;
    let decoded = reader
        .decode()
        .map_err(|error| FileSystemError::Io(error.to_string()))?;

    let (width, height) = decoded.dimensions();
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > THUMBNAIL_MAX_DECODED_PIXELS
    {
        return Ok(None);
    }

    // One fast nearest-neighbor step when the image is far larger than the
    // target keeps the final `thumbnail` pass cheap on huge photos.
    let target = u32::from(size.max(1));
    let working = if width > target * 4 && height > target * 4 {
        decoded.resize_exact(target * 4, target * 4, image::imageops::FilterType::Nearest)
    } else {
        decoded
    };
    let scaled = working.thumbnail(target, target);
    let (scaled_width, scaled_height) = scaled.dimensions();

    // Photos decode to opaque pixels; keep PNG only where alpha matters so
    // typical JPEGs stay small.
    let (mime, bytes) = if scaled.color().has_alpha() {
        let mut buffer = Vec::new();
        scaled
            .to_rgba8()
            .write_to(&mut std::io::Cursor::new(&mut buffer), image::ImageFormat::Png)
            .map_err(|error| FileSystemError::Io(error.to_string()))?;
        ("image/png", buffer)
    } else {
        let mut buffer = Vec::new();
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buffer, 82);
        DynamicImage::ImageRgb8(scaled.to_rgb8())
            .write_with_encoder(encoder)
            .map_err(|error| FileSystemError::Io(error.to_string()))?;
        ("image/jpeg", buffer)
    };

    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(Some(Thumbnail {
        data_url: format!("data:{mime};base64,{encoded}"),
        width: scaled_width,
        height: scaled_height,
    }))
}

#[tauri::command]
#[specta::specta]
pub async fn read_text_preview(
    path: String,
    max_bytes: u32,
) -> Result<TextPreview, FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || {
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
        use std::io::Read;
        file.read_exact(&mut buffer).map_err(FileSystemError::from)?;

        // Lossy decoding keeps multi-byte characters cut at the boundary from
        // failing the whole preview.
        Ok::<_, FileSystemError>(TextPreview {
            content: String::from_utf8_lossy(&buffer).into_owned(),
            truncated,
        })
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}
