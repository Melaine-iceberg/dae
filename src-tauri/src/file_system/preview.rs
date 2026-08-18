use super::error::FileSystemError;
use image::{DynamicImage, GenericImageView};
use serde::Serialize;
use specta::Type;
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TextPreview {
    pub content: String,
    pub truncated: bool,
}

/// Rendered thumbnail served as raw bytes over the custom `thumbnail://`
/// protocol. Raw bytes avoid the ~33% base64 inflation and JSON string
/// escaping that a `data:` URL over IPC would pay per image.
#[derive(Debug)]
pub struct RenderedThumbnail {
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

/// Source images above this size are skipped so browsing a folder of huge
/// archives-as-images cannot stall the UI.
const THUMBNAIL_MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
/// Hard cap on decoded thumbnail pixels; larger images are downscaled in one
/// cheap `resize` step before the final smooth pass.
const THUMBNAIL_MAX_DECODED_PIXELS: u64 = 40 * 1024 * 1024;
const THUMBNAIL_CACHE_MAX_ENTRIES: usize = 256;

/// Capped in-memory cache keyed by path + mtime + size + target size.
/// Entries are `Arc`'d so lookups never clone image bytes; eviction drops
/// the oldest entry instead of clearing the map so scrolling back through a
/// large folder stays cache-hot.
struct ThumbnailCache {
    entries: HashMap<String, Arc<RenderedThumbnail>>,
    insertion_order: VecDeque<String>,
}

static THUMBNAIL_CACHE: Mutex<Option<ThumbnailCache>> = Mutex::new(None);

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

/// Serves `thumbnail://localhost/?path=...&size=...` with raw image bytes.
/// The webview fetches these in parallel through its HTTP stack and caches
/// them per URL, which replaces one base64 `invoke` payload per image.
pub fn handle_thumbnail_protocol(
    _ctx: tauri::UriSchemeContext<'_, tauri::Wry>,
    request: tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<Vec<u8>> {
    match request
        .uri()
        .query()
        .and_then(thumbnail_request_params)
        .map(|(path, size)| render_thumbnail(&path, size))
    {
        Some(Ok(Some(thumbnail))) => tauri::http::Response::builder()
            .header("Content-Type", thumbnail.mime)
            // The URL embeds mtime + size, so a given URL is immutable.
            .header("Cache-Control", "public, max-age=86400, immutable")
            .body(thumbnail.bytes.clone())
            .unwrap_or_else(|_| empty_thumbnail_response(500)),
        Some(Ok(None)) => empty_thumbnail_response(404),
        // Unsupported files yield 404 so the frontend can fall back to the
        // file icon; genuinely broken reads surface as 500.
        Some(Err(_)) => empty_thumbnail_response(500),
        None => empty_thumbnail_response(400),
    }
}

fn empty_thumbnail_response(status: u16) -> tauri::http::Response<Vec<u8>> {
    tauri::http::Response::builder()
        .status(status)
        .body(Vec::new())
        .expect("static thumbnail response is always valid")
}

/// Extracts `path` and `size` from the request query string.
fn thumbnail_request_params(query: &str) -> Option<(String, u16)> {
    let mut path: Option<String> = None;
    let mut size: Option<u16> = None;

    for pair in query.split('&') {
        let (key, value) = pair.split_once('=')?;
        match key {
            "path" => path = Some(percent_decode(value)),
            "size" => size = value.parse().ok(),
            // `v` is a cache-busting version tag the backend can ignore.
            _ => {}
        }
    }

    path.map(|path| (path, size.unwrap_or(256)))
}

/// Percent-decoder for `encodeURIComponent`-encoded query values.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let high = hex_value(bytes[index + 1]);
                let low = hex_value(bytes[index + 2]);
                if let (Some(high), Some(low)) = (high, low) {
                    output.push((high << 4) | low);
                    index += 3;
                } else {
                    output.push(b'%');
                    index += 1;
                }
            }
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }

    String::from_utf8_lossy(&output).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn render_thumbnail(
    path_string: &str,
    size: u16,
) -> Result<Option<Arc<RenderedThumbnail>>, FileSystemError> {
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

    let thumbnail = Arc::new(thumbnail);
    store_cache(cache_key, Arc::clone(&thumbnail));
    Ok(Some(thumbnail))
}

fn lookup_cache(cache_key: &str) -> Option<Arc<RenderedThumbnail>> {
    THUMBNAIL_CACHE
        .lock()
        .ok()?
        .as_ref()?
        .entries
        .get(cache_key)
        .cloned()
}

fn store_cache(cache_key: String, thumbnail: Arc<RenderedThumbnail>) {
    if let Ok(mut guard) = THUMBNAIL_CACHE.lock() {
        let cache = guard.get_or_insert_with(|| ThumbnailCache {
            entries: HashMap::new(),
            insertion_order: VecDeque::new(),
        });

        // Refresh the recency marker when the key already exists.
        if cache.entries.contains_key(&cache_key) {
            if let Some(position) = cache
                .insertion_order
                .iter()
                .position(|key| key == &cache_key)
            {
                cache.insertion_order.remove(position);
            }
        }

        while cache.entries.len() >= THUMBNAIL_CACHE_MAX_ENTRIES {
            let Some(oldest) = cache.insertion_order.pop_front() else {
                break;
            };
            cache.entries.remove(&oldest);
        }

        cache.insertion_order.push_back(cache_key.clone());
        cache.entries.insert(cache_key, thumbnail);
    }
}

fn decode_and_scale(path: &Path, size: u16) -> Result<Option<RenderedThumbnail>, FileSystemError> {
    let reader = image::ImageReader::open(path)
        .and_then(|reader| reader.with_guessed_format())
        .map_err(|error| FileSystemError::Io(error.to_string()))?;
    let decoded = reader
        .decode()
        .map_err(|error| FileSystemError::Io(error.to_string()))?;

    let (width, height) = decoded.dimensions();
    if width == 0
        || height == 0
        || u64::from(width) * u64::from(height) > THUMBNAIL_MAX_DECODED_PIXELS
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

    // Photos decode to opaque pixels; keep PNG only where alpha matters so
    // typical JPEGs stay small.
    if scaled.color().has_alpha() {
        let mut buffer = Vec::new();
        scaled
            .to_rgba8()
            .write_to(
                &mut std::io::Cursor::new(&mut buffer),
                image::ImageFormat::Png,
            )
            .map_err(|error| FileSystemError::Io(error.to_string()))?;
        Ok(Some(RenderedThumbnail {
            mime: "image/png",
            bytes: buffer,
        }))
    } else {
        let mut buffer = Vec::new();
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buffer, 82);
        DynamicImage::ImageRgb8(scaled.to_rgb8())
            .write_with_encoder(encoder)
            .map_err(|error| FileSystemError::Io(error.to_string()))?;
        Ok(Some(RenderedThumbnail {
            mime: "image/jpeg",
            bytes: buffer,
        }))
    }
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
        file.read_exact(&mut buffer)
            .map_err(FileSystemError::from)?;

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
