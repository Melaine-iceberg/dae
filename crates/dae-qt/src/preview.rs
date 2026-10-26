//! The preview pane's state.
//!
//! One controller per window. `open()` reads the entry's metadata and, when the
//! extension calls for it, a bounded text peek and media tags. The work runs on
//! the shared Tokio runtime and is applied back on the Qt thread; a newer
//! `open()` bumps a generation so a slow file cannot overwrite the current one.
//!
//! Thumbnails are not decoded here: `PreviewPanel.qml` asks the
//! `thumbnail://` image provider for the bytes, which keeps decoding off the
//! GUI thread and behind its own cache.

use std::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use dae_core::fs::media_meta;
use dae_core::fs::{preview, service};
use dae_core::types::{EntryKind, FileProperties, display_name_from_path};

use crate::format;
use crate::runtime;

const PREVIEW_READ_BYTES: u32 = 64 * 1024;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("cxx-qt-lib/qqmlapplicationengine.h");
        type QQmlApplicationEngine = cxx_qt_lib::QQmlApplicationEngine;

        include!("image_provider.h");
        #[namespace = "dae_qt"]
        #[rust_name = "register_thumbnail_provider"]
        unsafe fn dae_qt_register_thumbnail_provider(engine: *mut QQmlApplicationEngine);
    }

    unsafe extern "RustQt" {
        /// QML usage: `PreviewController { id: preview }` then
        /// `preview.open(path)`. `thumbnailRevision` is part of the provider
        /// URL, so a file edited on disk is not served from a stale cache.
        #[qobject]
        #[qml_element]
        #[qproperty(QString, path)]
        #[qproperty(QString, name)]
        #[qproperty(bool, is_directory, cxx_name = "isDirectory")]
        #[qproperty(QString, size_text, cxx_name = "sizeText")]
        #[qproperty(QString, modified_text, cxx_name = "modifiedText")]
        #[qproperty(QString, kind_text, cxx_name = "kindText")]
        #[qproperty(QString, content)]
        #[qproperty(bool, truncated)]
        #[qproperty(bool, has_text, cxx_name = "hasText")]
        #[qproperty(bool, has_thumbnail, cxx_name = "hasThumbnail")]
        #[qproperty(QString, thumbnail_revision, cxx_name = "thumbnailRevision")]
        #[qproperty(QString, media_text, cxx_name = "mediaText")]
        #[qproperty(bool, loading)]
        #[qproperty(QString, error)]
        type PreviewController = super::PreviewControllerRust;

        #[qinvokable]
        fn open(self: Pin<&mut PreviewController>, path: &QString);

        #[qinvokable]
        fn clear(self: Pin<&mut PreviewController>);
    }

    impl cxx_qt::Threading for PreviewController {}
}

/// Registers the `thumbnail://` provider before the QML root is loaded.
pub fn register_image_providers(engine: *mut cxx_qt_lib::QQmlApplicationEngine) {
    unsafe {
        qobject::register_thumbnail_provider(engine);
    }
}

#[derive(Default)]
pub struct PreviewControllerRust {
    path: QString,
    name: QString,
    is_directory: bool,
    size_text: QString,
    modified_text: QString,
    kind_text: QString,
    content: QString,
    truncated: bool,
    has_text: bool,
    has_thumbnail: bool,
    thumbnail_revision: QString,
    media_text: QString,
    loading: bool,
    error: QString,
    generation: u64,
}

struct Snapshot {
    name: String,
    is_directory: bool,
    size_text: String,
    modified_text: String,
    kind_text: String,
    content: String,
    truncated: bool,
    has_text: bool,
    has_thumbnail: bool,
    thumbnail_revision: String,
    media_text: String,
}

impl qobject::PreviewController {
    fn open(mut self: Pin<&mut Self>, path: &QString) {
        let path = path.to_string();
        let generation = self.as_mut().rust_mut().generation.wrapping_add(1);
        self.as_mut().rust_mut().generation = generation;

        self.as_mut().set_path(QString::from(path.as_str()));
        self.as_mut()
            .set_name(QString::from(display_name_from_path(&path).as_str()));
        self.as_mut().set_is_directory(false);
        self.as_mut().set_size_text(QString::default());
        self.as_mut().set_modified_text(QString::default());
        self.as_mut().set_kind_text(QString::default());
        self.as_mut().set_content(QString::default());
        self.as_mut().set_truncated(false);
        self.as_mut().set_has_text(false);
        self.as_mut()
            .set_has_thumbnail(preview::is_thumbnail_path(&path));
        self.as_mut().set_thumbnail_revision(QString::default());
        self.as_mut().set_media_text(QString::default());
        self.as_mut().set_error(QString::default());
        self.as_mut().set_loading(true);

        let thread = self.as_mut().qt_thread();
        runtime::spawn(async move {
            let result = load_snapshot(&path).await;
            let _ =
                thread.queue(move |mut preview| preview.as_mut().apply(generation, path, result));
        });
    }

    fn clear(mut self: Pin<&mut Self>) {
        let generation = self.as_mut().rust_mut().generation.wrapping_add(1);
        self.as_mut().rust_mut().generation = generation;

        self.as_mut().set_path(QString::default());
        self.as_mut().set_name(QString::default());
        self.as_mut().set_is_directory(false);
        self.as_mut().set_size_text(QString::default());
        self.as_mut().set_modified_text(QString::default());
        self.as_mut().set_kind_text(QString::default());
        self.as_mut().set_content(QString::default());
        self.as_mut().set_truncated(false);
        self.as_mut().set_has_text(false);
        self.as_mut().set_has_thumbnail(false);
        self.as_mut().set_thumbnail_revision(QString::default());
        self.as_mut().set_media_text(QString::default());
        self.as_mut().set_loading(false);
        self.as_mut().set_error(QString::default());
    }

    fn apply(
        mut self: Pin<&mut Self>,
        generation: u64,
        path: String,
        result: Result<Snapshot, String>,
    ) {
        if self.generation != generation {
            return;
        }

        match result {
            Ok(snapshot) => {
                self.as_mut().set_path(QString::from(path.as_str()));
                self.as_mut()
                    .set_name(QString::from(snapshot.name.as_str()));
                self.as_mut().set_is_directory(snapshot.is_directory);
                self.as_mut()
                    .set_size_text(QString::from(snapshot.size_text.as_str()));
                self.as_mut()
                    .set_modified_text(QString::from(snapshot.modified_text.as_str()));
                self.as_mut()
                    .set_kind_text(QString::from(snapshot.kind_text.as_str()));
                self.as_mut()
                    .set_content(QString::from(snapshot.content.as_str()));
                self.as_mut().set_truncated(snapshot.truncated);
                self.as_mut().set_has_text(snapshot.has_text);
                self.as_mut().set_has_thumbnail(snapshot.has_thumbnail);
                self.as_mut()
                    .set_thumbnail_revision(QString::from(snapshot.thumbnail_revision.as_str()));
                self.as_mut()
                    .set_media_text(QString::from(snapshot.media_text.as_str()));
                self.as_mut().set_error(QString::default());
            }
            Err(message) => {
                self.as_mut().set_error(QString::from(message.as_str()));
            }
        }
        // Last: every binding that also reads `loading` then sees the complete
        // snapshot instead of an intermediate state.
        self.as_mut().set_loading(false);
    }
}

async fn load_snapshot(path: &str) -> Result<Snapshot, String> {
    let properties = service::get_file_properties(path.to_owned())
        .await
        .map_err(|error| friendly_preview_error(&error.to_string()))?;
    let is_directory = properties.kind == EntryKind::Directory;

    let (text, media) = if is_directory {
        (None, None)
    } else {
        let text = if preview::is_text_preview_path(path) {
            preview::read_text_preview(path.to_owned(), PREVIEW_READ_BYTES)
                .await
                .ok()
        } else {
            None
        };
        let media = if is_media_path(path) {
            media_meta::read_media_preview(path.to_owned()).await.ok()
        } else {
            None
        };
        (text, media)
    };

    Ok(snapshot_from(properties, text, media))
}

fn snapshot_from(
    properties: FileProperties,
    text: Option<preview::TextPreview>,
    media: Option<media_meta::MediaPreview>,
) -> Snapshot {
    let (content, truncated, has_text) = match text {
        Some(text) => (text.content, text.truncated, true),
        None => (String::new(), false, false),
    };
    Snapshot {
        name: properties.name,
        is_directory: properties.kind == EntryKind::Directory,
        size_text: properties
            .size
            .map(format::size)
            .unwrap_or_else(|| "—".to_owned()),
        modified_text: properties
            .modified_at
            .map(format::timestamp)
            .unwrap_or_else(|| "—".to_owned()),
        kind_text: kind_label(properties.kind).to_owned(),
        content,
        truncated,
        has_text,
        has_thumbnail: preview::is_thumbnail_path(&properties.path),
        thumbnail_revision: format!(
            "{}-{}",
            properties.modified_at.unwrap_or(0),
            properties.size.unwrap_or(0)
        ),
        media_text: media.map(media_lines).unwrap_or_default(),
    }
}

fn media_lines(media: media_meta::MediaPreview) -> String {
    let mut lines = Vec::new();
    if let Some(duration) = media.duration_ms {
        lines.push(format!("时长: {}", duration_label(duration)));
    }
    if let (Some(width), Some(height)) = (media.width, media.height) {
        lines.push(format!("尺寸: {width} × {height}"));
    }
    if let Some(bitrate) = media.bitrate_bps {
        lines.push(format!("码率: {}", bitrate_label(bitrate)));
    }
    for (label, value) in media.tags {
        lines.push(format!("{label}: {value}"));
    }
    lines.join("\n")
}

fn duration_label(milliseconds: u64) -> String {
    let total_seconds = milliseconds / 1000;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

fn bitrate_label(bits_per_second: u64) -> String {
    if bits_per_second >= 1_000_000 {
        format!("{:.1} Mbps", bits_per_second as f64 / 1_000_000.0)
    } else {
        format!("{} kbps", bits_per_second / 1000)
    }
}

fn kind_label(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Directory => "文件夹",
        EntryKind::File => "文件",
        EntryKind::Symlink => "符号链接",
        EntryKind::Other => "其他",
    }
}

fn is_media_path(path: &str) -> bool {
    let extension = std::path::Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    media_meta::is_audio_extension(&extension) || media_meta::is_video_extension(&extension)
}

fn friendly_preview_error(message: &str) -> String {
    if message.starts_with("The requested path was not found") {
        return "该项目已不存在".to_owned();
    }
    message.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_as_clock_time() {
        assert_eq!(duration_label(65_000), "1:05");
        assert_eq!(duration_label(3_725_000), "1:02:05");
    }

    #[test]
    fn media_extensions_are_not_text() {
        assert!(is_media_path("C:/clip.MP4"));
        assert!(is_media_path("C:/song.flac"));
        assert!(!is_media_path("C:/main.rs"));
    }
}
