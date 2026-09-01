use super::error::FileSystemError;
use serde::Serialize;
use specta::Type;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Lightweight metadata snapshot for the preview panel. Everything is parsed
/// from container headers only (a few KB of reads) — no audio/video decoding
/// happens, so probing gigabyte-sized media files stays instant.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MediaPreview {
    /// "audio" or "video".
    pub kind: String,
    pub duration_ms: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub bitrate_bps: Option<u64>,
    /// Ordered display tags (title, artist, album, ...).
    pub tags: Vec<(String, String)>,
}

/// Files above this size skip media probing; a 2GB cap covers any realistic
/// clip while keeping metadata reads bounded.
const MEDIA_PREVIEW_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "wav", "m4a", "aac", "ogg", "opus", "wma", "aiff", "aif",
];
// `.ts` is intentionally absent: TypeScript sources outweigh MPEG-TS streams
// in a file manager, and the extension drives the code preview instead.
const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "m4v", "mov", "mkv", "webm", "avi", "wmv", "flv",
];

pub fn is_audio_extension(extension: &str) -> bool {
    AUDIO_EXTENSIONS.contains(&extension)
}

pub fn is_video_extension(extension: &str) -> bool {
    VIDEO_EXTENSIONS.contains(&extension)
}

#[tauri::command]
#[specta::specta]
pub async fn read_media_preview(path: String) -> Result<MediaPreview, FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || read_media_preview_sync(&path))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

fn read_media_preview_sync(path_string: &str) -> Result<MediaPreview, FileSystemError> {
    let path = Path::new(path_string);
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let kind = if is_video_extension(&extension) {
        "video"
    } else if is_audio_extension(&extension) {
        "audio"
    } else {
        return Err(FileSystemError::Unsupported(path_string.to_owned()));
    };

    let metadata = fs::metadata(path).map_err(FileSystemError::from)?;
    if !metadata.is_file() {
        return Err(FileSystemError::InvalidInput(path_string.to_owned()));
    }
    if metadata.len() > MEDIA_PREVIEW_MAX_BYTES {
        return Err(FileSystemError::Unsupported(path_string.to_owned()));
    }

    let mut file = File::open(path).map_err(FileSystemError::from)?;
    let file_size = metadata.len();

    let mut preview = match extension.as_str() {
        "mp3" => parse_mp3(&mut file, file_size),
        "flac" => parse_flac(&mut file, file_size),
        "wav" => parse_wav(&mut file, file_size),
        "m4a" | "mp4" | "m4v" | "mov" => parse_mp4(&mut file, file_size),
        "mkv" | "webm" => parse_matroska(&mut file, file_size),
        "avi" => parse_avi(&mut file, file_size),
        // Containers without a cheap header parser (ts, wmv, ogg, ...) still
        // surface the type label; the panel then shows the file facts only.
        _ => Ok(MediaPreview {
            kind: kind.to_owned(),
            duration_ms: None,
            width: None,
            height: None,
            bitrate_bps: None,
            tags: Vec::new(),
        }),
    };

    if let Ok(preview) = preview.as_mut() {
        preview.kind = kind.to_owned();
    }
    preview
}

fn base_preview(kind: &str) -> MediaPreview {
    MediaPreview {
        kind: kind.to_owned(),
        duration_ms: None,
        width: None,
        height: None,
        bitrate_bps: None,
        tags: Vec::new(),
    }
}

fn io_error(error: std::io::Error) -> FileSystemError {
    FileSystemError::Io(error.to_string())
}

fn read_exact_at(
    file: &mut File,
    offset: u64,
    buffer: &mut [u8],
) -> Result<(), FileSystemError> {
    file.seek(SeekFrom::Start(offset)).map_err(io_error)?;
    file.read_exact(buffer).map_err(io_error)
}

fn u16be(bytes: &[u8]) -> u16 {
    u16::from_be_bytes(bytes[..2].try_into().unwrap_or([0, 0]))
}

fn u32be(bytes: &[u8]) -> u32 {
    u32::from_be_bytes(bytes[..4].try_into().unwrap_or([0, 0, 0, 0]))
}

fn u32le(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes[..4].try_into().unwrap_or([0, 0, 0, 0]))
}

/// ID3v2-style synchsafe integer (7 payload bits per byte).
fn syncsafe(bytes: &[u8]) -> usize {
    bytes.iter().fold(0usize, |value, byte| (value << 7) | usize::from(byte & 0x7F))
}

/// Pushes `(label, value)` unless the value is empty or the label repeats.
fn push_tag(tags: &mut Vec<(String, String)>, label: &str, value: String) {
    let value = value.trim_matches(char::from(0)).trim().to_owned();
    if value.is_empty() || tags.iter().any(|(existing, _)| existing == label) {
        return;
    }
    tags.push((label.to_owned(), value));
}

/// Maps the canonical tag ids used by ID3v2.3/2.4, MP4 `ilst` atoms, and
/// Vorbis comments onto stable display labels.
fn tag_label(frame_id: &str) -> Option<&'static str> {
    match frame_id {
        "TIT2" | "TITLE" | "\u{a9}nam" => Some("Title"),
        "TPE1" | "ARTIST" | "\u{a9}ART" => Some("Artist"),
        "TALB" | "ALBUM" | "\u{a9}alb" => Some("Album"),
        "TCON" | "GENRE" | "\u{a9}gen" => Some("Genre"),
        "TYER" | "TDRC" | "DATE" | "\u{a9}day" => Some("Year"),
        "TRCK" | "TRACKNUMBER" | "trkn" => Some("Track"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// MP3: ID3v2 tags + Xing/VBRI header for accurate VBR duration.
// ---------------------------------------------------------------------------

fn parse_mp3(file: &mut File, file_size: u64) -> Result<MediaPreview, FileSystemError> {
    let mut preview = base_preview("audio");

    // The tag and the first audio frames always fit in the first 512KB of a
    // sane file; reading only that prefix keeps huge MP3s cheap.
    let head_len = file_size.min(512 * 1024) as usize;
    let mut head = vec![0u8; head_len];
    read_exact_at(file, 0, &mut head)?;

    let mut audio_start = 0usize;
    if head_len >= 10 && &head[..3] == b"ID3" {
        let major_version = head[3];
        let flags = head[5];
        let tag_size = syncsafe(&head[6..10]);
        let tag_end = 10 + tag_size + if flags & 0x10 != 0 { 10 } else { 0 };
        audio_start = tag_end.min(head_len);
        if major_version == 2 {
            parse_id3v2_2(&head[10..audio_start], &mut preview.tags);
        } else {
            parse_id3v2_3(&head[10..audio_start], major_version, &mut preview.tags);
        }
    }

    // Locate the first MPEG frame to derive bitrate; Xing/Info refines VBR.
    if let Some((frame_offset, version, sample_rate, samples_per_frame, bitrate_bps, mono)) =
        find_mpeg_frame(&head, audio_start)
    {
        let xing = find_xing_header(&head, frame_offset, version, mono);
        if let Some((frames, xing_bitrate)) = xing {
            let duration = u64::from(frames) * u64::from(samples_per_frame) * 1000
                / u64::from(sample_rate);
            preview.duration_ms = Some(duration);
            preview.bitrate_bps = xing_bitrate.or_else(|| {
                duration
                    .checked_mul(1000)
                    .filter(|ms| *ms > 0)
                    .map(|ms| (file_size.saturating_sub(audio_start as u64)) * 8 * 1000 / ms)
            });
        } else {
            preview.bitrate_bps = Some(u64::from(bitrate_bps) * 1000);
            preview.duration_ms = u64::from(bitrate_bps)
                .checked_mul(1000)
                .filter(|bps| *bps > 0)
                .map(|bps| file_size.saturating_sub(frame_offset as u64) * 8 * 1000 / bps);
        }
    }

    Ok(preview)
}

fn parse_id3v2_2(frames: &[u8], tags: &mut Vec<(String, String)>) {
    let mut offset = 0;
    while offset + 6 <= frames.len() {
        let id = &frames[offset..offset + 3];
        if id[0] == 0 {
            break;
        }
        let size =
            (usize::from(frames[offset + 3]) << 16) | (usize::from(frames[offset + 4]) << 8) | usize::from(frames[offset + 5]);
        offset += 6;
        let data_end = offset + size;
        if data_end > frames.len() {
            break;
        }
        let id_text = String::from_utf8_lossy(id).into_owned();
        let mapped = match id_text.as_str() {
            "TT2" => "TIT2",
            "TP1" => "TPE1",
            "TAL" => "TALB",
            "TCO" => "TCON",
            "TYE" => "TYER",
            "TRK" => "TRCK",
            _ => id_text.as_str(),
        };
        if let Some(label) = tag_label(mapped) {
            push_tag(tags, label, decode_id3_text(&frames[offset..data_end]));
        }
        offset = data_end;
    }
}

fn parse_id3v2_3(frames: &[u8], major_version: u8, tags: &mut Vec<(String, String)>) {
    let mut offset = 0;
    while offset + 10 <= frames.len() {
        let id = &frames[offset..offset + 4];
        if id[0] == 0 {
            break;
        }
        // v2.4 sizes are synchsafe; v2.3 uses plain big-endian.
        let size = if major_version >= 4 {
            syncsafe(&frames[offset + 4..offset + 8])
        } else {
            usize::try_from(u32be(&frames[offset + 4..offset + 8])).unwrap_or(0)
        };
        offset += 10;
        let data_end = offset + size;
        if data_end > frames.len() {
            break;
        }
        let id_text = String::from_utf8_lossy(id).into_owned();
        if let Some(label) = tag_label(&id_text) {
            push_tag(tags, label, decode_id3_text(&frames[offset..data_end]));
        }
        offset = data_end;
    }
}

/// Decodes ID3v2 text frames honoring the leading encoding byte.
fn decode_id3_text(data: &[u8]) -> String {
    let Some((&encoding, payload)) = data.split_first() else {
        return String::new();
    };
    match encoding {
        0 => payload.iter().map(|&byte| char::from(byte)).collect(),
        1 => decode_utf16_payload(payload, true),
        2 => decode_utf16_payload(payload, false),
        _ => String::from_utf8_lossy(payload).into_owned(),
    }
}

fn decode_utf16_payload(payload: &[u8], respect_bom: bool) -> String {
    let mut units = payload
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>();
    if respect_bom
        && let Some(first) = units.first().copied()
    {
        if first == 0xFEFF {
            units.remove(0);
        } else if first == 0xFFFE {
            units.remove(0);
            for unit in &mut units {
                *unit = unit.swap_bytes();
            }
        }
    }
    if let Some(end) = units.iter().position(|unit| *unit == 0) {
        units.truncate(end);
    }
    String::from_utf16_lossy(&units)
}

/// Returns `(offset, mpeg_version, sample_rate, samples_per_frame, bitrate_kbps, mono)`
/// for the first valid MPEG-1/2/2.5 Layer III frame header.
fn find_mpeg_frame(head: &[u8], start: usize) -> Option<(usize, u8, u16, u16, u16, bool)> {
    let mut offset = start;
    while offset + 4 <= head.len() {
        let bytes = &head[offset..offset + 4];
        if bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0 {
            let version_bits = (bytes[1] >> 3) & 0x03;
            let layer_bits = (bytes[1] >> 1) & 0x03;
            let bitrate_index = usize::from(bytes[2] >> 4);
            let sample_index = usize::from((bytes[2] >> 2) & 0x03);
            let mono = bytes[3] >> 6 == 0x03;
            // Only Layer III (0b01) on real MPEG versions with valid indexes.
            if layer_bits == 0x01
                && version_bits != 0x01
                && bitrate_index > 0
                && bitrate_index < 15
                && sample_index < 3
            {
                let (version, samples_per_frame, bitrate_table, sample_rates) =
                    match version_bits {
                        0x03 => (
                            1u8,
                            1152u16,
                            [0u16, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320]
                                .as_slice(),
                            [44100u16, 48000, 32000].as_slice(),
                        ),
                        0x02 => (
                            2u8,
                            576u16,
                            [0u16, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160]
                                .as_slice(),
                            [22050u16, 24000, 16000].as_slice(),
                        ),
                        _ => (
                            3u8,
                            576u16,
                            [0u16, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160]
                                .as_slice(),
                            [11025u16, 12000, 8000].as_slice(),
                        ),
                    };
                return Some((
                    offset,
                    version,
                    sample_rates[sample_index],
                    samples_per_frame,
                    bitrate_table[bitrate_index],
                    mono,
                ));
            }
        }
        offset += 1;
    }
    None
}

/// Looks for a Xing/Info tag past the frame's side info; returns the frame
/// count and, when present, the stored average bitrate in kbps.
fn find_xing_header(head: &[u8], frame_offset: usize, version: u8, mono: bool) -> Option<(u32, Option<u64>)> {
    let side_info_len = match (version == 1, mono) {
        (true, false) => 32,
        (true, true) => 17,
        (false, false) => 17,
        (false, true) => 9,
    };
    let tag_offset = frame_offset + 4 + side_info_len;
    if tag_offset + 16 > head.len() {
        return None;
    }
    let tag = &head[tag_offset..tag_offset + 4];
    if tag != b"Xing" && tag != b"Info" {
        return None;
    }
    let flags = u32be(&head[tag_offset + 4..tag_offset + 8]);
    let mut cursor = tag_offset + 8;
    let mut frames = None;
    if flags & 0x1 != 0 && cursor + 4 <= head.len() {
        frames = Some(u32be(&head[cursor..cursor + 4]));
        cursor += 4;
    }
    let mut bitrate = None;
    if flags & 0x2 != 0 && cursor + 4 <= head.len() {
        let kbps = u64::from(u32be(&head[cursor..cursor + 4]));
        if kbps > 0 {
            bitrate = Some(kbps * 1000);
        }
    }
    frames.map(|frames| (frames, bitrate))
}

// ---------------------------------------------------------------------------
// FLAC: STREAMINFO for timing + VORBIS_COMMENT for tags.
// ---------------------------------------------------------------------------

fn parse_flac(file: &mut File, file_size: u64) -> Result<MediaPreview, FileSystemError> {
    let mut preview = base_preview("audio");

    let head_len = file_size.min(1024 * 1024) as usize;
    let mut head = vec![0u8; head_len];
    read_exact_at(file, 0, &mut head)?;
    if head_len < 4 || &head[..4] != b"fLaC" {
        return Ok(preview);
    }

    let mut offset = 4;
    while offset + 4 <= head_len {
        let header = head[offset];
        let is_last = header & 0x80 != 0;
        let block_type = header & 0x7F;
        let block_size =
            (usize::from(head[offset + 1]) << 16) | (usize::from(head[offset + 2]) << 8) | usize::from(head[offset + 3]);
        offset += 4;
        let block_end = offset + block_size;
        if block_end > head_len {
            break;
        }
        let block = &head[offset..block_end];

        if block_type == 0 && block.len() >= 18 {
            // Bytes 10..18 pack sample rate (20b), channels (3b), bits per
            // sample (5b), and total samples (36b).
            let packed = u64::from_be_bytes(block[10..18].try_into().unwrap_or([0; 8]));
            let sample_rate = packed >> 44;
            let total_samples = packed & 0x0F_FFFF_FFFF;
            if sample_rate > 0 && total_samples > 0 {
                preview.duration_ms = Some(total_samples * 1000 / sample_rate);
            }
        } else if block_type == 4 && block.len() >= 8 {
            parse_vorbis_comment(block, &mut preview.tags);
        }

        offset = block_end;
        if is_last {
            break;
        }
    }

    if preview.duration_ms.unwrap_or(0) > 0 {
        preview.bitrate_bps =
            Some(file_size * 8 * 1000 / preview.duration_ms.unwrap_or(1));
    }
    Ok(preview)
}

/// Shared Vorbis comment layout (FLAC block 4 / OGG): vendor string followed
/// by `KEY=value` pairs.
fn parse_vorbis_comment(block: &[u8], tags: &mut Vec<(String, String)>) {
    let vendor_len = usize::try_from(u32le(&block[0..4])).unwrap_or(usize::MAX);
    let mut cursor = 4 + vendor_len;
    if cursor + 4 > block.len() {
        return;
    }
    let count = usize::try_from(u32le(&block[cursor..cursor + 4])).unwrap_or(0).min(64);
    cursor += 4;
    for _ in 0..count {
        if cursor + 4 > block.len() {
            break;
        }
        let entry_len = usize::try_from(u32le(&block[cursor..cursor + 4])).unwrap_or(usize::MAX);
        cursor += 4;
        let entry_end = cursor + entry_len;
        if entry_end > block.len() {
            break;
        }
        let entry = String::from_utf8_lossy(&block[cursor..entry_end]);
        if let Some((key, value)) = entry.split_once('=')
            && let Some(label) = tag_label(&key.to_ascii_uppercase())
        {
            push_tag(tags, label, value.to_owned());
        }
        cursor = entry_end;
    }
}

// ---------------------------------------------------------------------------
// WAV: fmt + data chunks give duration directly.
// ---------------------------------------------------------------------------

fn parse_wav(file: &mut File, file_size: u64) -> Result<MediaPreview, FileSystemError> {
    let mut preview = base_preview("audio");

    let head_len = file_size.min(4096) as usize;
    let mut head = vec![0u8; head_len];
    read_exact_at(file, 0, &mut head)?;
    if head_len < 12 || &head[..4] != b"RIFF" || &head[8..12] != b"WAVE" {
        return Ok(preview);
    }

    let mut byte_rate = 0u32;
    let mut offset = 12;
    while offset + 8 <= head_len {
        let chunk_id = &head[offset..offset + 4];
        let chunk_size = usize::try_from(u32le(&head[offset + 4..offset + 8])).unwrap_or(0);
        let payload = offset + 8;
        if chunk_id == b"fmt " && payload + 16 <= head_len {
            byte_rate = u32le(&head[payload + 8..payload + 12]);
        } else if chunk_id == b"data" {
            if byte_rate > 0 {
                let data_size = file_size.min(payload as u64 + chunk_size as u64) - payload as u64;
                preview.duration_ms = Some(data_size * 1000 / u64::from(byte_rate));
                preview.bitrate_bps = Some(u64::from(byte_rate) * 8);
            }
            break;
        }
        // Chunks are word-aligned.
        offset = payload + chunk_size + (chunk_size & 1);
    }
    Ok(preview)
}

// ---------------------------------------------------------------------------
// MP4/M4A/MOV: box walk for mvhd timing, tkhd dimensions, ilst tags.
// ---------------------------------------------------------------------------

fn parse_mp4(file: &mut File, file_size: u64) -> Result<MediaPreview, FileSystemError> {
    let mut preview = base_preview("video");
    let mut timescale: Option<u32> = None;
    let mut duration_units: Option<u64> = None;
    let mut width: Option<u32> = None;
    let mut height: Option<u32> = None;

    let mut offset = 0u64;
    while offset + 8 <= file_size {
        let Some((kind, payload_start, box_end)) = read_box_header(file, offset, file_size)?
        else {
            break;
        };
        if &kind == b"moov" {
            walk_mp4_moov(file, payload_start, box_end, &mut timescale, &mut duration_units, &mut width, &mut height, &mut preview.tags)?;
            break;
        }
        if &kind == b"mdat" {
            break; // Metadata lives before mdat in fast-start files.
        }
        offset = box_end;
    }

    // Fragmented recordings (screen capture, live streams) append `moov`
    // after all `mdat` data; scan a bounded tail window for it instead of
    // walking gigabytes of media boxes.
    if timescale.is_none() {
        parse_mp4_tail(file, file_size, &mut timescale, &mut duration_units, &mut width, &mut height, &mut preview.tags)?;
    }

    if let (Some(timescale), Some(units)) = (timescale, duration_units)
        && timescale > 0
    {
        preview.duration_ms = Some(units * 1000 / u64::from(timescale));
    }
    if width.unwrap_or(0) > 0 {
        preview.width = width;
        preview.height = height;
    } else {
        preview.kind = "audio".to_owned();
    }
    if preview.duration_ms.unwrap_or(0) > 0 {
        preview.bitrate_bps = Some(file_size * 8 * 1000 / preview.duration_ms.unwrap_or(1));
    }
    Ok(preview)
}

/// Scans the last 32MB of the file for an appended `moov` box. The window
/// covers long fragmented recordings (the box grows with fragment count)
/// while keeping the probe bounded to one read regardless of file size.
#[allow(clippy::too_many_arguments)]
fn parse_mp4_tail(
    file: &mut File,
    file_size: u64,
    timescale: &mut Option<u32>,
    duration_units: &mut Option<u64>,
    width: &mut Option<u32>,
    height: &mut Option<u32>,
    tags: &mut Vec<(String, String)>,
) -> Result<(), FileSystemError> {
    const TAIL_WINDOW: u64 = 32 * 1024 * 1024;
    let window_start = file_size.saturating_sub(TAIL_WINDOW);
    let mut window = vec![0u8; (file_size - window_start) as usize];
    read_exact_at(file, window_start, &mut window)?;

    let mut search_from = 0usize;
    while search_from + 8 <= window.len() {
        let Some(pos) = window[search_from..].windows(4).position(|slice| slice == b"moov")
        else {
            break;
        };
        let type_pos = search_from + pos;
        // A real box header carries its size in the four bytes before the
        // type; candidate hits inside sample data fail this walk instead.
        if type_pos >= 4 {
            let candidate = type_pos - 4;
            if let Some((kind, payload_start, box_end)) =
                read_box_header(file, window_start + candidate as u64, file_size)?
                && &kind == b"moov"
            {
                walk_mp4_moov(file, payload_start, box_end, timescale, duration_units, width, height, tags)?;
                if timescale.is_some() {
                    return Ok(());
                }
            }
        }
        search_from = type_pos + 4;
    }
    Ok(())
}

/// Reads an ISO-BMFF box header at `offset`; returns `(type, payload_start,
/// box_end)`. Handles 64-bit largesize and open-ended size-0 boxes.
fn read_box_header(
    file: &mut File,
    offset: u64,
    limit: u64,
) -> Result<Option<([u8; 4], u64, u64)>, FileSystemError> {
    let mut header = [0u8; 8];
    if read_exact_at(file, offset, &mut header).is_err() {
        return Ok(None);
    }
    let mut size = u64::from(u32be(&header));
    let kind = [header[4], header[5], header[6], header[7]];
    let mut payload_start = offset + 8;
    if size == 1 {
        let mut large = [0u8; 8];
        if read_exact_at(file, offset + 8, &mut large).is_err() {
            return Ok(None);
        }
        size = u64::from_be_bytes(large);
        payload_start = offset + 16;
    }
    let box_end = if size == 0 { limit } else { offset + size };
    if box_end > limit || box_end <= offset {
        return Ok(None);
    }
    Ok(Some((kind, payload_start, box_end)))
}

#[allow(clippy::too_many_arguments)]
fn walk_mp4_moov(
    file: &mut File,
    start: u64,
    end: u64,
    timescale: &mut Option<u32>,
    duration_units: &mut Option<u64>,
    width: &mut Option<u32>,
    height: &mut Option<u32>,
    tags: &mut Vec<(String, String)>,
) -> Result<(), FileSystemError> {
    let mut offset = start;
    while offset + 8 <= end {
        let Some((kind, payload_start, box_end)) = read_box_header(file, offset, end)? else {
            break;
        };
        let payload_len = (box_end - payload_start) as usize;
        match &kind {
            b"mvhd" => {
                let mut payload = vec![0u8; payload_len.min(32)];
                read_exact_at(file, payload_start, &mut payload)?;
                if !payload.is_empty() && payload[0] == 1 && payload.len() >= 28 {
                    *timescale = Some(u32be(&payload[20..24]));
                    *duration_units = Some(u64::from_be_bytes(payload[24..32].try_into().unwrap_or([0; 8])));
                } else if payload.len() >= 20 {
                    *timescale = Some(u32be(&payload[12..16]));
                    *duration_units = Some(u64::from(u32be(&payload[16..20])));
                }
            }
            b"trak" => {
                walk_mp4_trak(file, payload_start, box_end, width, height)?;
            }
            b"udta" => {
                walk_mp4_udta(file, payload_start, box_end, tags)?;
            }
            _ => {}
        }
        offset = box_end;
    }
    Ok(())
}

fn walk_mp4_trak(
    file: &mut File,
    start: u64,
    end: u64,
    width: &mut Option<u32>,
    height: &mut Option<u32>,
) -> Result<(), FileSystemError> {
    let mut offset = start;
    while offset + 8 <= end {
        let Some((kind, payload_start, box_end)) = read_box_header(file, offset, end)? else {
            break;
        };
        if &kind == b"tkhd" {
            // Dimensions sit at the very end of tkhd as 16.16 fixed point;
            // audio tracks carry 0x0 which filters them out upstream.
            let tail_start = box_end.saturating_sub(8);
            if tail_start > payload_start {
                let mut tail = [0u8; 8];
                read_exact_at(file, tail_start, &mut tail)?;
                let track_width = u32be(&tail[0..4]) >> 16;
                let track_height = u32be(&tail[4..8]) >> 16;
                if track_width > width.unwrap_or(0) {
                    *width = Some(track_width);
                    *height = Some(track_height);
                }
            }
            return Ok(());
        }
        offset = box_end;
    }
    Ok(())
}

fn walk_mp4_udta(
    file: &mut File,
    start: u64,
    end: u64,
    tags: &mut Vec<(String, String)>,
) -> Result<(), FileSystemError> {
    let mut offset = start;
    while offset + 8 <= end {
        let Some((kind, payload_start, box_end)) = read_box_header(file, offset, end)? else {
            break;
        };
        if &kind == b"meta" {
            // `meta` is a full box: skip the version/flags byte quartet.
            walk_mp4_meta(file, payload_start + 4, box_end, tags)?;
            return Ok(());
        }
        offset = box_end;
    }
    Ok(())
}

fn walk_mp4_meta(
    file: &mut File,
    start: u64,
    end: u64,
    tags: &mut Vec<(String, String)>,
) -> Result<(), FileSystemError> {
    let mut offset = start;
    while offset + 8 <= end {
        let Some((kind, payload_start, box_end)) = read_box_header(file, offset, end)? else {
            break;
        };
        if &kind == b"ilst" {
            walk_mp4_ilst(file, payload_start, box_end, tags)?;
            return Ok(());
        }
        offset = box_end;
    }
    Ok(())
}

fn walk_mp4_ilst(
    file: &mut File,
    start: u64,
    end: u64,
    tags: &mut Vec<(String, String)>,
) -> Result<(), FileSystemError> {
    let mut offset = start;
    while offset + 8 <= end {
        let Some((kind, payload_start, box_end)) = read_box_header(file, offset, end)? else {
            break;
        };
        let key = String::from_utf8_lossy(&kind).into_owned();
        if let Some(label) = tag_label(&key) {
            // Each item holds a `data` box: 8 header bytes + 8 bytes of
            // type/locale before the UTF-8 payload (trkn is a binary u16 pair).
            let mut cursor = payload_start;
            while cursor + 8 <= box_end {
                let Some((child_kind, child_payload, child_end)) =
                    read_box_header(file, cursor, box_end)?
                else {
                    break;
                };
                if &child_kind == b"data" && child_end > child_payload + 8 {
                    if key == "trkn" {
                        let mut binary = [0u8; 8];
                        let read_len = (child_end - child_payload - 8).min(8) as usize;
                        read_exact_at(file, child_payload + 8, &mut binary[..read_len])?;
                        let track = u16be(&binary[2..4]);
                        let total = u16be(&binary[4..6]);
                        if track > 0 {
                            let value = if total > 0 {
                                format!("{track}/{total}")
                            } else {
                                track.to_string()
                            };
                            push_tag(tags, label, value);
                        }
                    } else {
                        let value_len = (child_end - child_payload - 8).min(1024) as usize;
                        let mut value_bytes = vec![0u8; value_len];
                        read_exact_at(file, child_payload + 8, &mut value_bytes)?;
                        push_tag(tags, label, String::from_utf8_lossy(&value_bytes).into_owned());
                    }
                    break;
                }
                cursor = child_end;
            }
        }
        offset = box_end;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Matroska/WebM: EBML walk for Segment/Info timing and the first video track.
// ---------------------------------------------------------------------------

const MKV_ID_EBML: u64 = 0x1A45_DFA3;
const MKV_ID_SEGMENT: u64 = 0x1853_8067;
const MKV_ID_INFO: u64 = 0x1549_A966;
const MKV_ID_TRACKS: u64 = 0x1654_AE6B;
const MKV_ID_TIMESTAMP_SCALE: u64 = 0x2AD7B1;
const MKV_ID_DURATION: u64 = 0x4489;
const MKV_ID_TRACK_ENTRY: u64 = 0xAE;
const MKV_ID_TRACK_TYPE: u64 = 0x83;
const MKV_ID_PIXEL_WIDTH: u64 = 0xB0;
const MKV_ID_PIXEL_HEIGHT: u64 = 0xBA;

/// Reads an EBML variable-length integer; `keep_marker` preserves the VINT
/// marker bit (element IDs) while data sizes strip it.
fn read_vint(bytes: &[u8], keep_marker: bool) -> Option<(u64, usize)> {
    let first = *bytes.first()?;
    if first == 0 {
        return None;
    }
    let length = first.leading_zeros() as usize + 1;
    if length > bytes.len() {
        return None;
    }
    let mut value = if keep_marker {
        u64::from(first)
    } else {
        u64::from(first & (0xFF >> length))
    };
    for byte in &bytes[1..length] {
        value = (value << 8) | u64::from(*byte);
    }
    Some((value, length))
}

fn parse_matroska(file: &mut File, file_size: u64) -> Result<MediaPreview, FileSystemError> {
    let mut preview = base_preview("video");

    // Info/Tracks practically always live in the first few MB; capping the
    // read keeps pathological files from stalling the preview panel.
    let head_len = file_size.min(4 * 1024 * 1024) as usize;
    let mut head = vec![0u8; head_len];
    read_exact_at(file, 0, &mut head)?;

    let mut timestamp_scale: Option<u64> = None;
    let mut duration: Option<f64> = None;
    let mut width: Option<u32> = None;
    let mut height: Option<u32> = None;

    let mut offset = 0usize;
    // Skip the EBML header, then enter the Segment.
    while offset < head_len {
        let Some((id, id_len)) = read_vint(&head[offset..], true) else {
            break;
        };
        let Some((size, size_len)) = read_vint(&head[offset + id_len..], false) else {
            break;
        };
        let payload_start = offset + id_len + size_len;
        let payload_end = payload_start + size as usize;
        if id == MKV_ID_EBML {
            offset = payload_end;
            continue;
        }
        if id == MKV_ID_SEGMENT {
            walk_mkv_container(
                &head,
                payload_start,
                payload_end.min(head_len),
                true,
                &mut timestamp_scale,
                &mut duration,
                &mut width,
                &mut height,
            );
            break;
        }
        offset = payload_end;
    }

    if let (Some(scale), Some(units)) = (timestamp_scale, duration) {
        preview.duration_ms = Some((units * scale as f64 / 1_000_000.0).round() as u64);
    }
    if width.unwrap_or(0) > 0 {
        preview.width = width;
        preview.height = height;
    } else {
        preview.kind = "audio".to_owned();
    }
    if preview.duration_ms.unwrap_or(0) > 0 {
        preview.bitrate_bps = Some(file_size * 8 * 1000 / preview.duration_ms.unwrap_or(1));
    }
    Ok(preview)
}

fn walk_mkv_container(
    head: &[u8],
    start: usize,
    end: usize,
    is_segment: bool,
    timestamp_scale: &mut Option<u64>,
    duration: &mut Option<f64>,
    width: &mut Option<u32>,
    height: &mut Option<u32>,
) {
    let mut offset = start;
    while offset < end {
        let Some((id, id_len)) = read_vint(&head[offset..], true) else {
            break;
        };
        let Some((size, size_len)) = read_vint(&head[offset + id_len..], false) else {
            break;
        };
        let payload_start = offset + id_len + size_len;
        let payload_end = payload_start + size as usize;
        if payload_end > end {
            break;
        }
        let payload = &head[payload_start..payload_end];

        match id {
            MKV_ID_INFO | MKV_ID_TRACKS if is_segment => {
                walk_mkv_container(
                    head,
                    payload_start,
                    payload_end,
                    false,
                    timestamp_scale,
                    duration,
                    width,
                    height,
                );
            }
            MKV_ID_TRACK_ENTRY => {
                walk_mkv_container(
                    head,
                    payload_start,
                    payload_end,
                    false,
                    timestamp_scale,
                    duration,
                    width,
                    height,
                );
            }
            MKV_ID_TIMESTAMP_SCALE => {
                *timestamp_scale = Some(mkv_uint(payload));
            }
            MKV_ID_DURATION => {
                *duration = Some(mkv_float(payload));
            }
            MKV_ID_TRACK_TYPE => {
                // Track type 1 marks video; tag scope is per-entry so a plain
                // container walk picks up whichever entry carries dimensions.
            }
            MKV_ID_PIXEL_WIDTH => {
                *width = Some(mkv_uint(payload) as u32);
            }
            MKV_ID_PIXEL_HEIGHT => {
                *height = Some(mkv_uint(payload) as u32);
            }
            _ => {}
        }
        offset = payload_end;
    }
}

fn mkv_uint(payload: &[u8]) -> u64 {
    payload.iter().fold(0u64, |value, byte| (value << 8) | u64::from(*byte))
}

fn mkv_float(payload: &[u8]) -> f64 {
    match payload.len() {
        4 => f64::from(f32::from_be_bytes(payload.try_into().unwrap_or([0; 4]))),
        8 => f64::from_be_bytes(payload.try_into().unwrap_or([0; 8])),
        _ => 0.0,
    }
}

// ---------------------------------------------------------------------------
// AVI: avih holds microseconds-per-frame; strf carries the video dimensions.
// ---------------------------------------------------------------------------

fn parse_avi(file: &mut File, file_size: u64) -> Result<MediaPreview, FileSystemError> {
    let mut preview = base_preview("video");

    let head_len = file_size.min(64 * 1024) as usize;
    let mut head = vec![0u8; head_len];
    read_exact_at(file, 0, &mut head)?;
    if head_len < 12 || &head[..4] != b"RIFF" || &head[8..12] != b"AVI " {
        return Ok(preview);
    }

    let mut microsec_per_frame: Option<u32> = None;
    let mut total_frames: Option<u32> = None;
    let mut width: Option<u32> = None;
    let mut height: Option<u32> = None;
    let mut in_video_stream = false;

    let mut offset = 12;
    while offset + 8 <= head_len {
        let chunk_id = &head[offset..offset + 4];
        let chunk_size = usize::try_from(u32le(&head[offset + 4..offset + 8])).unwrap_or(0);
        let payload = offset + 8;
        let payload_end = (payload + chunk_size).min(head_len);

        if chunk_id == b"avih" && payload + 20 <= payload_end {
            microsec_per_frame = Some(u32le(&head[payload..payload + 4]));
            total_frames = Some(u32le(&head[payload + 16..payload + 20]));
        } else if chunk_id == b"strh" && payload + 4 <= payload_end {
            in_video_stream = &head[payload..payload + 4] == b"vids";
        } else if chunk_id == b"strf" && in_video_stream && payload + 12 <= payload_end {
            // BITMAPINFOHEADER: width @4, signed height @8.
            let stream_width = u32le(&head[payload + 4..payload + 8]);
            let stream_height = i32::from_le_bytes(
                head[payload + 8..payload + 12].try_into().unwrap_or([0; 4]),
            );
            width = Some(stream_width);
            height = Some(stream_height.unsigned_abs());
        }

        // LIST chunks are walked into; others are skipped word-aligned.
        offset = if chunk_id == b"LIST" { payload } else { payload + chunk_size + (chunk_size & 1) };
    }

    if let (Some(us_per_frame), Some(frames)) = (microsec_per_frame, total_frames)
        && us_per_frame > 0
    {
        let duration_ms = u64::from(us_per_frame) * u64::from(frames) / 1000;
        preview.duration_ms = Some(duration_ms);
        if duration_ms > 0 {
            preview.bitrate_bps = Some(file_size * 8 * 1000 / duration_ms);
        }
    }
    if width.unwrap_or(0) > 0 {
        preview.width = width;
        preview.height = height;
    }
    Ok(preview)
}

#[cfg(test)]
mod tests {
    use super::{read_media_preview_sync, read_vint, syncsafe, tag_label};

    #[test]
    fn syncsafe_decodes_id3_sizes() {
        assert_eq!(syncsafe(&[0x00, 0x00, 0x02, 0x01]), 257);
        assert_eq!(syncsafe(&[0x7F, 0x7F, 0x7F, 0x7F]), 0x0FFF_FFFF);
    }

    #[test]
    fn vint_reads_ebml_ids_and_sizes() {
        // One-byte size 0x81 -> 1 with the marker stripped.
        assert_eq!(read_vint(&[0x81], false), Some((1, 1)));
        // Element id 0x1A45DFA3 keeps its marker bits.
        assert_eq!(read_vint(&[0x1A, 0x45, 0xDF, 0xA3], true), Some((0x1A45_DFA3, 4)));
    }

    #[test]
    fn tag_labels_map_across_containers() {
        assert_eq!(tag_label("TIT2"), Some("Title"));
        assert_eq!(tag_label("\u{a9}ART"), Some("Artist"));
        assert_eq!(tag_label("ALBUM"), Some("Album"));
        assert_eq!(tag_label("xxxx"), None);
    }

    #[test]
    fn parses_synthetic_wav_duration_and_bitrate() {
        // One second of silence: 8kHz mono 16-bit PCM.
        let sample_rate = 8000u32;
        let data_size = sample_rate * 2;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_size).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&1u16.to_le_bytes()); // mono
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
        wav.extend_from_slice(&2u16.to_le_bytes()); // block align
        wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_size.to_le_bytes());
        wav.extend(std::iter::repeat_n(0u8, data_size as usize));

        let path = std::env::temp_dir().join("dae-media-meta-test.wav");
        std::fs::write(&path, &wav).expect("temp dir is writable");

        let preview = read_media_preview_sync(path.to_str().unwrap())
            .expect("synthetic WAV parses");
        std::fs::remove_file(&path).ok();

        assert_eq!(preview.kind, "audio");
        assert_eq!(preview.duration_ms, Some(1000));
        assert_eq!(preview.bitrate_bps, Some(128_000));
        assert!(preview.tags.is_empty());
    }
}
