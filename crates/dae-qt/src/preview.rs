//! The preview pane's state.
//!
//! One controller per window. `open()` reads the entry's metadata and, when the
//! extension calls for it, a bounded text peek and media tags. The work runs on
//! the shared Tokio runtime and is applied back on the Qt thread; a newer
//! `open()` bumps a generation so a slow file cannot overwrite the current one.
//!
//! `preload()` is the speculative half: the shell hands it the rows around the
//! selection, and it fills the same snapshot cache on a dedicated low-priority
//! runtime, so arrowing through a folder finds the next preview already read.
//! A newer selection bumps the preload epoch and the tasks that have not
//! started their content reads drop themselves.
//!
//! Thumbnails are not decoded here: `PreviewPanel.qml` asks the
//! `thumbnail://` image provider for the bytes, which keeps decoding off the
//! GUI thread and behind its own cache.

use std::collections::{HashMap, VecDeque};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, OnceLock};

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QStringList};
use dae_core::fs::media_meta;
use dae_core::fs::vfs;
use dae_core::fs::{office, preview, service};
use dae_core::types::{EntryKind, FileProperties, display_name_from_path};

use crate::format;
use crate::markdown_model;
use crate::office_markup;
use crate::preview_markup::{self, MarkdownBlock, SourceKind};
use crate::runtime;

const PREVIEW_READ_BYTES: u32 = 64 * 1024;
/// The whole OOXML container has to be read before any part can be opened, so
/// this is the pane's one large read. The core refuses anything above it.
const PREVIEW_OFFICE_BYTES: u32 = 48 * 1024 * 1024;
/// A `.doc`/`.xls`/`.ppt` is an OLE compound file, not a ZIP: reading it needs
/// a converter, so the pane says what to do instead of staying blank.
const LEGACY_OFFICE_MESSAGE: &str =
    "Legacy Office formats cannot be previewed. Save a .docx, .xlsx or .pptx copy first.";
/// OpenDocument and the Apple iWork bundles: known, not parsed.
const UNSUPPORTED_OFFICE_MESSAGE: &str = "This document format has no preview yet.";

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;

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
        /// How many entries a selected folder holds, and the first few of
        /// their names. Filled for directories only, from a bounded read that
        /// starts no watcher: the listing itself belongs to the tab.
        #[qproperty(i32, child_count, cxx_name = "childCount")]
        #[qproperty(QStringList, child_names, cxx_name = "childNames")]
        #[qproperty(QString, content)]
        /// The rendered HTML for Markdown and highlighted code, empty for a
        /// plain-text peek. `content` always keeps the raw text so the panel
        /// can fall back to it.
        #[qproperty(QString, content_html, cxx_name = "contentHtml")]
        #[qproperty(bool, is_markdown, cxx_name = "isMarkdown")]
        /// Non-zero revision identifying the current `MarkdownModel` block list
        /// published for this preview, or 0 when the entry is not Markdown.
        #[qproperty(i32, markdown_revision, cxx_name = "markdownRevision")]
        #[qproperty(bool, truncated)]
        #[qproperty(bool, has_text, cxx_name = "hasText")]
        #[qproperty(bool, has_thumbnail, cxx_name = "hasThumbnail")]
        #[qproperty(QString, thumbnail_revision, cxx_name = "thumbnailRevision")]
        #[qproperty(QString, media_text, cxx_name = "mediaText")]
        /// `audio` or `video` when this entry can be played in place; empty
        /// for everything else and for remote entries (their bytes live behind
        /// a VFS the media backend cannot open).
        #[qproperty(QString, media_kind, cxx_name = "mediaKind")]
        #[qproperty(QString, media_url, cxx_name = "mediaUrl")]
        /// The rendered body of an OOXML document (`.docx`, `.xlsx`, `.pptx`),
        /// empty for every other file. `officeFormat` names the family and
        /// `officeTruncated` says the pane is showing a sample.
        #[qproperty(bool, has_office, cxx_name = "hasOffice")]
        #[qproperty(QString, office_html, cxx_name = "officeHtml")]
        #[qproperty(QString, office_format, cxx_name = "officeFormat")]
        #[qproperty(bool, office_truncated, cxx_name = "officeTruncated")]
        /// Why an Office file has no preview: too large, encrypted, a legacy
        /// binary format, or a format dae does not parse. Shown in place of the
        /// body rather than replacing the metadata.
        #[qproperty(QString, office_error, cxx_name = "officeError")]
        #[qproperty(bool, loading)]
        #[qproperty(QString, error)]
        /// True for `smb://`, `sftp://` and `cloud://` entries. Their text is
        /// read through the VFS, but thumbnails and media tags are desktop
        /// services that only know local paths.
        #[qproperty(bool, remote)]
        type PreviewController = super::PreviewControllerRust;

        #[qinvokable]
        fn open(self: Pin<&mut PreviewController>, path: &QString);

        /// Fills the snapshot cache for the rows around the selection, in the
        /// order the shell expects to reach them. Low priority and cancellable:
        /// a newer `open()` or `preload()` drops whatever has not started.
        #[qinvokable]
        fn preload(self: Pin<&mut PreviewController>, paths: &QStringList);

        #[qinvokable]
        fn clear(self: Pin<&mut PreviewController>);

        /// The panel's light/dark choice. Markdown and highlighted code carry
        /// concrete colours, so a theme change re-renders the cached source.
        #[qinvokable]
        #[cxx_name = "setDark"]
        fn set_dark(self: Pin<&mut PreviewController>, dark: bool);
    }

    impl cxx_qt::Threading for PreviewController {}
}

/// Registers the `thumbnail://` and `fileicon://` providers before the QML
/// root is loaded.
pub fn register_image_providers(engine: *mut cxx_qt_lib::QQmlApplicationEngine) {
    unsafe {
        qobject::register_thumbnail_provider(engine);
    }
    crate::icons::register_provider(engine);
}

/// Whether the pane should give this file an image slot: formats Qt decodes
/// itself, plus the ones the desktop can produce (PDF, video, HEIC) — the
/// desktop set only for local files, because a producer needs the file on
/// this disk. Remote rows still get the formats the in-process decoders
/// read, pulled bounded through `fs::thumbnails::fetch_remote`.
fn has_thumbnail(path: &str) -> bool {
    if vfs::is_local_path(path) {
        preview::is_thumbnail_path(path) || dae_core::fs::thumbnails::is_supported_path(path)
    } else {
        preview::is_thumbnail_path(path)
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
    child_count: i32,
    child_names: QStringList,
    content: QString,
    content_html: QString,
    is_markdown: bool,
    markdown_revision: i32,
    truncated: bool,
    has_text: bool,
    has_thumbnail: bool,
    thumbnail_revision: QString,
    media_text: QString,
    media_kind: QString,
    media_url: QString,
    has_office: bool,
    office_html: QString,
    office_format: QString,
    office_truncated: bool,
    office_error: QString,
    loading: bool,
    error: QString,
    remote: bool,
    generation: u64,
    /// Shared with the in-flight preload tasks: every `open()`, `clear()` and
    /// `preload()` bumps it, and a task whose epoch no longer matches drops
    /// itself instead of reading for a selection that has moved on.
    preload_epoch: Arc<AtomicU64>,
    source_text: String,
    source_kind: SourceKind,
    source_path: String,
    /// The extracted document behind `office_html`, kept so a theme switch can
    /// re-render without opening the container again.
    source_office: Option<office::OfficePreview>,
    dark: bool,
}

#[derive(Clone, Default)]
struct Snapshot {
    name: String,
    is_directory: bool,
    size_text: String,
    modified_text: String,
    kind_text: String,
    child_count: i32,
    child_names: Vec<String>,
    content: String,
    content_html: String,
    markdown_blocks: Arc<Vec<MarkdownBlock>>,
    source_kind: SourceKind,
    source_path: String,
    dark: bool,
    truncated: bool,
    has_text: bool,
    has_thumbnail: bool,
    thumbnail_revision: String,
    media_text: String,
    media_kind: String,
    media_url: String,
    office: Option<office::OfficePreview>,
    office_html: String,
    office_format: String,
    office_truncated: bool,
    office_error: String,
    remote: bool,
}

/// How many snapshots the cache keeps. Each holds at most a 64 KiB peek plus
/// its rendered HTML and an Office body, so the ceiling is a few tens of MB
/// even for a folder of large documents — and arrowing back and forth stays
/// inside it.
const SNAPSHOT_CACHE_ENTRIES: usize = 16;
/// How many neighbours one `preload()` call may queue. The shell sends the
/// next two rows and the previous one; the cap keeps a longer list from
/// turning speculation into a directory walk.
const PRELOAD_NEIGHBORS: usize = 3;

/// Snapshots by path, dated by the same mtime-size revision the thumbnail URL
/// carries. A hit means the file has not changed since the read, so the pane
/// can skip the content reads entirely; a file edited on disk misses and is
/// read again. FIFO eviction: the oldest speculation is the least likely to
/// be reached next.
struct SnapshotCache {
    entries: HashMap<String, (String, Snapshot)>,
    order: VecDeque<String>,
}

fn snapshot_cache() -> &'static Mutex<SnapshotCache> {
    static CACHE: OnceLock<Mutex<SnapshotCache>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(SnapshotCache {
            entries: HashMap::new(),
            order: VecDeque::new(),
        })
    })
}

fn cache_get(path: &str, revision: &str) -> Option<Snapshot> {
    let cache = snapshot_cache()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    cache
        .entries
        .get(path)
        .filter(|(cached, _)| cached == revision)
        .map(|(_, snapshot)| snapshot.clone())
}

fn cache_insert(path: &str, revision: &str, snapshot: &Snapshot) {
    let mut cache = snapshot_cache()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if !cache.entries.contains_key(path) {
        if cache.entries.len() >= SNAPSHOT_CACHE_ENTRIES
            && let Some(oldest) = cache.order.pop_front()
        {
            cache.entries.remove(&oldest);
        }
        cache.order.push_back(path.to_owned());
    }
    cache
        .entries
        .insert(path.to_owned(), (revision.to_owned(), snapshot.clone()));
}

/// The freshness stamp of a snapshot: the same pair the thumbnail provider
/// versions its URL with, so text and image agree on what "changed" means.
fn revision_of(properties: &FileProperties) -> String {
    format!(
        "{}-{}",
        properties.modified_at.unwrap_or(0),
        properties.size.unwrap_or(0)
    )
}

impl qobject::PreviewController {
    fn open(mut self: Pin<&mut Self>, path: &QString) {
        let path = path.to_string();
        let generation = self.as_mut().rust_mut().generation.wrapping_add(1);
        self.as_mut().rust_mut().generation = generation;
        // The selection moved: speculation for the old neighbourhood is no
        // longer worth a thread. The shell's `preload()` for the new one
        // bumps the epoch again and queues under that.
        self.as_mut()
            .rust_mut()
            .preload_epoch
            .fetch_add(1, AtomicOrdering::AcqRel);

        // The previous snapshot stays on screen until the new one is ready.
        // Clearing it here collapsed the pane to a placeholder for every
        // selection, so moving through a folder flashed the whole preview.
        // `apply` swaps every field in one batch instead.
        self.as_mut().set_error(QString::default());
        self.as_mut().set_loading(true);

        let dark = self.dark;
        let thread = self.as_mut().qt_thread();
        runtime::spawn(async move {
            let result = load_snapshot(&path, dark).await;
            let _ =
                thread.queue(move |mut preview| preview.as_mut().apply(generation, path, result));
        });
    }

    fn preload(mut self: Pin<&mut Self>, paths: &QStringList) {
        let epoch = Arc::clone(&self.as_mut().rust_mut().preload_epoch);
        let generation = epoch.fetch_add(1, AtomicOrdering::AcqRel) + 1;
        let dark = self.dark;
        for path in paths.iter().take(PRELOAD_NEIGHBORS) {
            let path = path.to_string();
            // A remote read's latency is unpredictable, and one stuck SFTP
            // request would hold the single preload lane for every local
            // neighbour behind it. Speculation stays on local disk.
            if !vfs::is_local_path(&path) {
                continue;
            }
            let epoch = Arc::clone(&epoch);
            runtime::spawn_preload(async move {
                if epoch.load(AtomicOrdering::Acquire) != generation {
                    return;
                }
                let Ok(properties) = service::get_file_properties(path.clone()).await else {
                    return;
                };
                // The stat is the cheap half; the content reads are what a
                // moved-on selection must not pay for.
                if epoch.load(AtomicOrdering::Acquire) != generation {
                    return;
                }
                snapshot_cached(properties, &path, dark).await;
            });
        }
    }

    fn clear(mut self: Pin<&mut Self>) {
        let generation = self.as_mut().rust_mut().generation.wrapping_add(1);
        self.as_mut().rust_mut().generation = generation;
        self.as_mut()
            .rust_mut()
            .preload_epoch
            .fetch_add(1, AtomicOrdering::AcqRel);

        self.as_mut().set_path(QString::default());
        self.as_mut().set_name(QString::default());
        self.as_mut().set_is_directory(false);
        self.as_mut().set_size_text(QString::default());
        self.as_mut().set_modified_text(QString::default());
        self.as_mut().set_kind_text(QString::default());
        self.as_mut().set_child_count(0);
        self.as_mut().set_child_names(QStringList::default());
        self.as_mut().set_content(QString::default());
        self.as_mut().set_content_html(QString::default());
        self.as_mut().set_is_markdown(false);
        self.as_mut().set_markdown_revision(0);
        self.as_mut().set_truncated(false);
        self.as_mut().set_has_text(false);
        self.as_mut().set_has_thumbnail(false);
        self.as_mut().set_thumbnail_revision(QString::default());
        self.as_mut().set_media_text(QString::default());
        self.as_mut().set_media_kind(QString::default());
        self.as_mut().set_media_url(QString::default());
        self.as_mut().set_has_office(false);
        self.as_mut().set_office_html(QString::default());
        self.as_mut().set_office_format(QString::default());
        self.as_mut().set_office_truncated(false);
        self.as_mut().set_office_error(QString::default());
        self.as_mut().set_loading(false);
        self.as_mut().set_error(QString::default());
        self.as_mut().set_remote(false);
        self.as_mut().rust_mut().source_text.clear();
        self.as_mut().rust_mut().source_kind = SourceKind::Plain;
        self.as_mut().rust_mut().source_path.clear();
        self.as_mut().rust_mut().source_office = None;
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
                let is_markdown = snapshot.source_kind.is_markdown();
                let (html, markdown_revision) = if is_markdown {
                    let blocks = if snapshot.dark == self.dark {
                        Arc::clone(&snapshot.markdown_blocks)
                    } else {
                        Arc::new(preview_markup::render_markdown_blocks(
                            &snapshot.content,
                            &snapshot.source_path,
                            self.dark,
                        ))
                    };
                    (String::new(), markdown_model::publish(blocks))
                } else {
                    let html = if snapshot.dark == self.dark {
                        snapshot.content_html.clone()
                    } else {
                        preview_markup::render(
                            &snapshot.content,
                            &snapshot.source_path,
                            &snapshot.source_kind,
                            self.dark,
                        )
                    };
                    (html, 0)
                };
                let office_html = match (&snapshot.office, snapshot.dark == self.dark) {
                    (Some(preview), false) => office_markup::render(preview, self.dark),
                    _ => snapshot.office_html.clone(),
                };
                let has_office = snapshot.office.is_some();
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
                self.as_mut().set_child_count(snapshot.child_count);
                self.as_mut().set_child_names(
                    snapshot
                        .child_names
                        .iter()
                        .map(|name| QString::from(name.as_str()))
                        .collect::<QStringList>(),
                );
                self.as_mut()
                    .set_content(QString::from(snapshot.content.as_str()));
                self.as_mut().set_content_html(QString::from(html.as_str()));
                self.as_mut().set_is_markdown(is_markdown);
                self.as_mut().set_markdown_revision(markdown_revision);
                self.as_mut().set_truncated(snapshot.truncated);
                self.as_mut().set_has_text(snapshot.has_text);
                self.as_mut().set_has_thumbnail(snapshot.has_thumbnail);
                self.as_mut()
                    .set_thumbnail_revision(QString::from(snapshot.thumbnail_revision.as_str()));
                self.as_mut()
                    .set_media_text(QString::from(snapshot.media_text.as_str()));
                self.as_mut()
                    .set_media_kind(QString::from(snapshot.media_kind.as_str()));
                self.as_mut()
                    .set_media_url(QString::from(snapshot.media_url.as_str()));
                self.as_mut().set_has_office(has_office);
                self.as_mut()
                    .set_office_html(QString::from(office_html.as_str()));
                self.as_mut()
                    .set_office_format(QString::from(snapshot.office_format.as_str()));
                self.as_mut().set_office_truncated(snapshot.office_truncated);
                self.as_mut()
                    .set_office_error(QString::from(snapshot.office_error.as_str()));
                self.as_mut().set_remote(snapshot.remote);
                self.as_mut().set_error(QString::default());

                let mut rust = self.as_mut().rust_mut();
                rust.source_text = snapshot.content;
                rust.source_kind = snapshot.source_kind;
                rust.source_path = snapshot.source_path;
                rust.source_office = snapshot.office;
            }
            Err(message) => {
                // The retained snapshot belongs to the previous entry; drop it
                // rather than pair its content with this file's name and error.
                self.as_mut().set_path(QString::from(path.as_str()));
                self.as_mut()
                    .set_name(QString::from(display_name_from_path(&path).as_str()));
                self.as_mut().set_is_directory(false);
                self.as_mut().set_size_text(QString::default());
                self.as_mut().set_modified_text(QString::default());
                self.as_mut().set_kind_text(QString::default());
                self.as_mut().set_child_count(0);
                self.as_mut().set_child_names(QStringList::default());
                self.as_mut().set_content(QString::default());
                self.as_mut().set_content_html(QString::default());
                self.as_mut().set_is_markdown(false);
                self.as_mut().set_markdown_revision(0);
                self.as_mut().set_truncated(false);
                self.as_mut().set_has_text(false);
                self.as_mut().set_has_thumbnail(false);
                self.as_mut().set_thumbnail_revision(QString::default());
                self.as_mut().set_media_text(QString::default());
                self.as_mut().set_media_kind(QString::default());
                self.as_mut().set_media_url(QString::default());
                self.as_mut().set_has_office(false);
                self.as_mut().set_office_html(QString::default());
                self.as_mut().set_office_format(QString::default());
                self.as_mut().set_office_truncated(false);
                self.as_mut().set_office_error(QString::default());
                self.as_mut().set_remote(!vfs::is_local_path(&path));
                self.as_mut().set_error(QString::from(message.as_str()));

                let mut rust = self.as_mut().rust_mut();
                rust.source_text.clear();
                rust.source_kind = SourceKind::Plain;
                rust.source_path.clear();
                rust.source_office = None;
            }
        }
        // Last: every binding that also reads `loading` then sees the complete
        // snapshot instead of an intermediate state.
        self.as_mut().set_loading(false);
    }

    fn set_dark(mut self: Pin<&mut Self>, dark: bool) {
        if self.dark == dark {
            return;
        }
        self.as_mut().rust_mut().dark = dark;
        let (source, path, kind, office) = {
            let this = self.as_ref();
            let rust = this.rust();
            (
                rust.source_text.clone(),
                rust.source_path.clone(),
                rust.source_kind.clone(),
                rust.source_office.clone(),
            )
        };
        if kind.is_markdown() {
            let blocks = Arc::new(preview_markup::render_markdown_blocks(&source, &path, dark));
            let revision = markdown_model::publish(blocks);
            self.as_mut().set_markdown_revision(revision);
        } else if kind != SourceKind::Plain {
            let html = preview_markup::render(&source, &path, &kind, dark);
            if html != self.content_html.to_string() {
                self.as_mut().set_content_html(QString::from(html.as_str()));
            }
        }
        // An Office body carries concrete colours too, so switching the theme
        // has to re-render it from the cached extraction.
        if let Some(preview) = office {
            let html = office_markup::render(&preview, dark);
            if html != self.office_html.to_string() {
                self.as_mut().set_office_html(QString::from(html.as_str()));
            }
        }
    }
}

async fn load_snapshot(path: &str, dark: bool) -> Result<Snapshot, String> {
    let properties = service::get_file_properties(path.to_owned())
        .await
        .map_err(|error| friendly_preview_error(&error.to_string()))?;
    Ok(snapshot_cached(properties, path, dark).await)
}

/// The stat is done: answer from the cache when the file has not changed since
/// the cached read, otherwise read the body and cache it. Shared by the
/// interactive open and the preload lane, so a preload is exactly the read the
/// open would have done — the open then costs one stat.
async fn snapshot_cached(properties: FileProperties, path: &str, dark: bool) -> Snapshot {
    let revision = revision_of(&properties);
    if let Some(hit) = cache_get(&properties.path, &revision) {
        return hit;
    }
    let key = properties.path.clone();
    let snapshot = snapshot_body(properties, path, dark).await;
    cache_insert(&key, &revision, &snapshot);
    snapshot
}

async fn snapshot_body(properties: FileProperties, path: &str, dark: bool) -> Snapshot {
    let is_directory = properties.kind == EntryKind::Directory;

    let (text, media, exif) = if is_directory {
        (None, None, None)
    } else {
        // Cloud paths end in opaque provider ids, so extension checks belong
        // on the resolved name rather than on the path text.
        let name = properties.name.as_str();
        let text = if preview::may_have_text_preview(name) {
            preview::read_text_preview(path.to_owned(), PREVIEW_READ_BYTES)
                .await
                .ok()
                .flatten()
        } else {
            None
        };
        // Media tags come from the platform's metadata readers, which only
        // open local files. A remote clip still gets its name and size.
        let local = vfs::is_local_path(path);
        let media = if is_media_path(name) && local {
            media_meta::read_media_preview(path.to_owned()).await.ok()
        } else {
            None
        };
        // EXIF is a header read like the media probe, and only local files
        // have a reader behind them. An EXIF-less JPEG still previews.
        let exif = if local && media_meta::is_image_path(name) {
            media_meta::read_image_exif(path.to_owned()).await.ok()
        } else {
            None
        };
        (text, media, exif)
    };


    let children = if is_directory {
        folder_children(path).await
    } else {
        (0, Vec::new())
    };

    // Office is a fourth kind of content, not a text peek: the body arrives as
    // sections, and the container has to be read whole. A failure becomes a
    // line in the pane instead of an error that hides the metadata.
    let (office, office_error) = if is_directory {
        (None, String::new())
    } else {
        let name = properties.name.as_str();
        if office::is_office_path(name) {
            match office::read_office_preview(path.to_owned(), PREVIEW_OFFICE_BYTES).await {
                Ok(preview) => (Some(preview), String::new()),
                Err(error) => (None, friendly_office_error(&error.to_string())),
            }
        } else if office::is_legacy_office_path(name) {
            (None, LEGACY_OFFICE_MESSAGE.to_owned())
        } else if office::is_unsupported_office_path(name) {
            (None, UNSUPPORTED_OFFICE_MESSAGE.to_owned())
        } else {
            (None, String::new())
        }
    };

    // The one honest preview of an unknown binary: its first bytes as a hex
    // view. Only when nothing else answered — an image keeps its picture,
    // media its tags, an Office container its sections — and only for a
    // local file. The code card renders it: mono, no line numbers,
    // horizontally scrollable, exactly what a byte listing wants.
    let hex = if is_directory || !vfs::is_local_path(path) {
        None
    } else {
        let name = properties.name.as_str();
        let claimed = text.is_some()
            || media.is_some()
            || exif.is_some()
            || office.is_some()
            || !office_error.is_empty()
            || preview::is_thumbnail_path(name)
            || is_media_path(name);
        if claimed {
            None
        } else {
            let path = path.to_owned();
            tokio::task::spawn_blocking(move || read_hex_preview(&path))
                .await
                .ok()
        }
    };

    let (text, forced_kind) = match (text, hex) {
        // A decodeable text always wins; the hex view is the fallback, not
        // a second opinion.
        (Some(text), _) => (Some(text), None),
        (None, Some(Ok(hex))) => (Some(hex), Some(SourceKind::Code(String::new()))),
        (None, None | Some(Err(_))) => (None, None),
    };

    snapshot_from(
        properties, text, media, exif, children, office, office_error, dark, forced_kind,
    )
}

/// The first [`HEX_PREVIEW_BYTES`] of a local file as a hex listing, or the
/// error the read failed with. `truncated` says the file had more bytes
/// than the view shows.
fn read_hex_preview(
    path: &str,
) -> Result<preview::TextPreview, dae_core::error::FileSystemError> {
    use std::io::Read;

    let total = std::fs::metadata(path).map(|metadata| metadata.len())?;
    let reader = std::fs::File::open(path)?;
    let mut bytes = Vec::with_capacity(HEX_PREVIEW_BYTES as usize);
    reader
        .take(HEX_PREVIEW_BYTES.into())
        .read_to_end(&mut bytes)?;
    Ok(preview::TextPreview {
        content: format_hex(&bytes),
        truncated: total > bytes.len() as u64,
    })
}

/// One row per 16 bytes: an 8-digit offset, the bytes, then the printable
/// ASCII column with everything else shown as a dot. The address column is
/// the row's own identity, so the plain card's line numbers would only
/// repeat it.
fn format_hex(bytes: &[u8]) -> String {
    const WIDTH: usize = 16;
    let mut out = String::new();
    for (row, chunk) in bytes.chunks(WIDTH).enumerate() {
        out.push_str(&format!("{:08x}  ", row * WIDTH));
        for index in 0..WIDTH {
            match chunk.get(index) {
                Some(byte) => out.push_str(&format!("{byte:02x} ")),
                None => out.push_str("   "),
            }
        }
        out.push(' ');
        for byte in chunk {
            out.push(if byte.is_ascii_graphic() || *byte == b' ' {
                *byte as char
            } else {
                '.'
            });
        }
        out.push('\n');
    }
    out
}

/// The hex view's read window. Not paginated on purpose: a full hex editor
/// is its own surface, this is a peek.
const HEX_PREVIEW_BYTES: u32 = 4096;

/// The pane names the first few children of a selected folder. This is a
/// bounded snapshot rather than a listing: no watcher, no sorting options, no
/// git scan. A folder the backend cannot enumerate keeps a zero count.
async fn folder_children(path: &str) -> (i32, Vec<String>) {
    if !vfs::is_local_path(path) {
        return (0, Vec::new());
    }
    let path = std::path::PathBuf::from(path);
    tokio::task::spawn_blocking(move || read_children(&path))
        .await
        .unwrap_or((0, Vec::new()))
}

fn read_children(path: &std::path::Path) -> (i32, Vec<String>) {
    const PREVIEW_NAMES: usize = 4;
    let Ok(entries) = std::fs::read_dir(path) else {
        return (0, Vec::new());
    };
    let mut directories = Vec::new();
    let mut files = Vec::new();
    let mut count = 0usize;
    for entry in entries.flatten() {
        count += 1;
        if directories.len() + files.len() >= PREVIEW_NAMES {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            directories.push(name);
        } else {
            files.push(name);
        }
    }
    directories.sort();
    files.sort();
    directories.extend(files);
    directories.truncate(PREVIEW_NAMES);
    (count.min(i32::MAX as usize) as i32, directories)
}

fn snapshot_from(
    properties: FileProperties,
    text: Option<preview::TextPreview>,
    media: Option<media_meta::MediaPreview>,
    exif: Option<media_meta::ImageExif>,
    children: (i32, Vec<String>),
    office: Option<office::OfficePreview>,
    office_error: String,
    dark: bool,
    forced_kind: Option<SourceKind>,
) -> Snapshot {
    let is_directory = properties.kind == EntryKind::Directory;
    let revision = revision_of(&properties);
    let (content, truncated, has_text, source_kind, content_html, markdown_blocks) = match text {
        Some(text) => {
            let kind =
                forced_kind.unwrap_or_else(|| preview_markup::classify(&properties.name));
            let (html, blocks) = if kind.is_markdown() {
                (
                    String::new(),
                    Arc::new(preview_markup::render_markdown_blocks(
                        &text.content,
                        &properties.path,
                        dark,
                    )),
                )
            } else {
                (
                    preview_markup::render(&text.content, &properties.path, &kind, dark),
                    Arc::default(),
                )
            };
            (text.content, text.truncated, true, kind, html, blocks)
        }
        None => (
            String::new(),
            false,
            false,
            SourceKind::Plain,
            String::new(),
            Arc::default(),
        ),
    };
    // The media player still uses Qt's file backend, so a remote entry keeps
    // its metadata slot but gets no in-pane player.
    let media_kind = if !is_directory && vfs::is_local_path(&properties.path) {
        media_kind_of(&properties.name).to_owned()
    } else {
        String::new()
    };
    let media_url = if media_kind.is_empty() {
        String::new()
    } else {
        preview_markup::file_url(&properties.path).unwrap_or_default()
    };
    let (office_html, office_format, office_truncated) = match &office {
        Some(document) => (
            office_markup::render(document, dark),
            document.kind.label().to_owned(),
            document.truncated,
        ),
        None => (String::new(), String::new(), false),
    };
    Snapshot {
        name: properties.name,
        is_directory,
        size_text: properties
            .size
            .map(format::size)
            .unwrap_or_else(|| "—".to_owned()),
        modified_text: properties
            .modified_at
            .map(format::timestamp)
            .unwrap_or_else(|| "—".to_owned()),
        kind_text: kind_label(properties.kind).to_owned(),
        child_count: children.0,
        child_names: children.1,
        content,
        content_html,
        markdown_blocks,
        source_kind,
        source_path: properties.path.clone(),
        dark,
        truncated,
        has_text,
        has_thumbnail: has_thumbnail(&properties.path),
        thumbnail_revision: revision,
        media_text: metadata_block(media, exif),
        media_kind,
        media_url,
        office,
        office_html,
        office_format,
        office_truncated,
        office_error,
        remote: !vfs::is_local_path(&properties.path),
    }
}

fn media_kind_of(name: &str) -> &'static str {
    let extension = std::path::Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if media_meta::is_video_extension(&extension) {
        "video"
    } else if media_meta::is_audio_extension(&extension) {
        "audio"
    } else {
        ""
    }
}

fn media_lines(media: media_meta::MediaPreview) -> String {
    let mut lines = Vec::new();
    if let Some(duration) = media.duration_ms {
        lines.push(format!("Duration: {}", duration_label(duration)));
    }
    if let (Some(width), Some(height)) = (media.width, media.height) {
        lines.push(format!("Dimensions: {width} × {height}"));
    }
    if let Some(bitrate) = media.bitrate_bps {
        lines.push(format!("Bitrate: {}", bitrate_label(bitrate)));
    }
    for (label, value) in media.tags {
        lines.push(format!("{label}: {value}"));
    }
    lines.join("\n")
}

/// The pane's metadata block: media tags when the file has a player, EXIF
/// fields when it is a photo, both when it is a motion-photo container.
fn metadata_block(
    media: Option<media_meta::MediaPreview>,
    exif: Option<media_meta::ImageExif>,
) -> String {
    let mut blocks = Vec::new();
    if let Some(media) = media {
        let text = media_lines(media);
        if !text.is_empty() {
            blocks.push(text);
        }
    }
    if let Some(exif) = exif {
        let text = exif_lines(exif);
        if !text.is_empty() {
            blocks.push(text);
        }
    }
    blocks.join("\n")
}

fn exif_lines(exif: media_meta::ImageExif) -> String {
    let mut lines = Vec::new();
    if let Some((width, height)) = exif.dimensions {
        lines.push(format!("Dimensions: {width} × {height}"));
    }
    if let Some(camera) = exif.camera {
        lines.push(format!("Camera: {camera}"));
    }
    if let Some(lens) = exif.lens {
        lines.push(format!("Lens: {lens}"));
    }
    if let Some(taken) = exif.taken {
        lines.push(format!("Taken: {taken}"));
    }
    if let Some(exposure) = exif.exposure {
        lines.push(format!("Exposure: {exposure}"));
    }
    if let Some(aperture) = exif.aperture {
        lines.push(format!("Aperture: {aperture}"));
    }
    if let Some(iso) = exif.iso {
        lines.push(format!("ISO: {iso}"));
    }
    if let Some(focal_length) = exif.focal_length {
        lines.push(format!("Focal length: {focal_length}"));
    }
    if let Some(gps) = exif.gps {
        lines.push(format!("GPS: {gps}"));
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
        EntryKind::Directory => "Folder",
        EntryKind::File => "File",
        EntryKind::Symlink => "Symbolic link",
        EntryKind::Other => "Other",
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
        return "This item no longer exists".to_owned();
    }
    message.to_owned()
}

/// Office failures are three user-visible outcomes, not one: too large, no
/// text, unreadable. The core reports codes so the wording stays here.
fn friendly_office_error(message: &str) -> String {
    if message.contains("fs.office_too_large") {
        return "Office document is too large to preview.".to_owned();
    }
    if message.contains("fs.office_empty") {
        return "This Office document has no readable text.".to_owned();
    }
    "This Office file cannot be read. It may be encrypted or damaged.".to_owned()
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

    #[test]
    fn desktop_rendered_types_get_a_thumbnail_slot() {
        assert!(has_thumbnail("C:/tree/photo.jpg"));
        assert!(has_thumbnail("C:/tree/report.pdf"));
        assert!(has_thumbnail("C:/tree/clip.mp4"));
        assert!(!has_thumbnail("C:/tree/main.rs"));
    }

    #[test]
    fn remote_entries_get_the_formats_qt_decodes_but_not_the_desktop_set() {
        // Over the wire the pane still previews images...
        assert!(has_thumbnail("smb://nas/share/photo.jpg"));
        assert!(has_thumbnail("archive:/K:/shots.zip!/pixel.png"));
        // ...but a PDF or a video would need a local file for the desktop
        // producer, so it keeps the type glyph.
        assert!(!has_thumbnail("smb://nas/share/report.pdf"));
        assert!(!has_thumbnail("webdav://nas/clip.mp4"));
        assert!(!has_thumbnail("archive:/K:/shots.zip!/main.rs"));
    }

    #[test]
    fn network_entries_keep_the_desktop_produced_set_off() {
        // Images over the wire are covered above; what stayed local-only is
        // the desktop-produced set, whose producers read a local file.
        assert!(!has_thumbnail("smb://nas/media/report.pdf"));
        assert!(!has_thumbnail("sftp://nas/home/clip.mp4"));
        assert!(!has_thumbnail(
            "cloud://google_drive:me@example.com/photo.heic"
        ));
    }

    #[test]
    fn the_hex_view_lays_out_offset_bytes_and_ascii() {
        // `HI!` then a control byte, then padding to make the row's tail
        // visibly short.
        let bytes = [b'H', b'I', b'!', 0x00, b' '];
        let hex = format_hex(&bytes);
        let rows: Vec<&str> = hex.trim_end_matches('\n').split('\n').collect();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].contains("HI!. "), "ASCII column with a dot for control");
        assert_eq!(
            rows[0].find("HI!. "),
            Some(8 + 2 + 16 * 3 + 1),
            "short rows pad so the ASCII column stays aligned"
        );

        // Row two gets its own address.
        let many: Vec<u8> = (0..20u8).collect();
        let hex = format_hex(&many);
        let rows: Vec<&str> = hex.trim_end_matches('\n').split('\n').collect();
        assert_eq!(rows.len(), 2);
        assert!(rows[1].starts_with("00000010"));
    }

    #[test]
    fn the_hex_view_reads_a_bounded_window() {
        let root = std::env::temp_dir().join("dae-preview-hex");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let small = root.join("small.bin");
        std::fs::write(&small, b"abcd").unwrap();
        let preview = read_hex_preview(&small.to_string_lossy()).unwrap();
        assert!(!preview.truncated, "the whole file fits the window");
        assert!(preview.content.contains("61 62 63 64"), "the bytes are listed");
        assert!(preview.content.contains("abcd"), "and readable in the ASCII column");

        let big = root.join("big.bin");
        std::fs::write(&big, vec![0u8; HEX_PREVIEW_BYTES as usize + 1]).unwrap();
        let preview = read_hex_preview(&big.to_string_lossy()).unwrap();
        assert!(preview.truncated, "one byte past the window says so");
        let lines = preview.content.trim_end_matches('\n').split('\n').count();
        assert_eq!(lines, (HEX_PREVIEW_BYTES / 16) as usize, "exactly the window's rows");
    }

    #[test]
    fn a_folder_preview_counts_every_child_but_names_the_first_few() {
        let root = std::env::temp_dir().join("dae-preview-folder-children");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("b.txt"), b"x").unwrap();
        std::fs::write(root.join("a.txt"), b"x").unwrap();

        let (count, names) = read_children(&root);
        assert_eq!(count, 3);
        // Folders first, then files, each alphabetically.
        assert_eq!(names, vec!["sub", "a.txt", "b.txt"]);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn office_failures_read_as_three_different_outcomes() {
        assert_eq!(
            friendly_office_error("The requested operation has invalid input: fs.office_too_large"),
            "Office document is too large to preview."
        );
        assert_eq!(
            friendly_office_error("fs.office_empty"),
            "This Office document has no readable text."
        );
        assert_eq!(
            friendly_office_error("fs.office_unreadable"),
            "This Office file cannot be read. It may be encrypted or damaged."
        );
    }

    #[test]
    fn a_preloaded_snapshot_is_the_read_the_open_would_do() {
        let root = std::env::temp_dir().join("dae-preview-preload-test");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("notes.txt");
        std::fs::write(&file, b"the quick brown fox").unwrap();
        let path = file.to_string_lossy().into_owned();

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let snapshot = runtime.block_on(async {
            let properties = service::get_file_properties(path.clone()).await.unwrap();
            snapshot_cached(properties, &path, true).await
        });
        assert_eq!(snapshot.content, "the quick brown fox");
        assert!(snapshot.has_text);

        // The preload filled the cache under the file's own revision, so the
        // interactive open finds it without a second content read.
        let hit = cache_get(&path, &snapshot.thumbnail_revision)
            .expect("preload cached the snapshot");
        assert_eq!(hit.content, "the quick brown fox");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_snapshot_cache_dates_hits_and_evicts_the_oldest() {
        let base = "C:/dae-preview-cache-test/";
        let first = Snapshot {
            name: "first".to_owned(),
            ..Snapshot::default()
        };
        cache_insert(&format!("{base}a.txt"), "10-20", &first);
        assert_eq!(
            cache_get(&format!("{base}a.txt"), "10-20").unwrap().name,
            "first"
        );
        // A file edited on disk carries a new revision and misses.
        assert!(cache_get(&format!("{base}a.txt"), "11-20").is_none());
        assert!(cache_get(&format!("{base}missing.txt"), "10-20").is_none());

        // Re-reading the same path refreshes its entry in place.
        cache_insert(&format!("{base}a.txt"), "11-20", &first);
        assert!(cache_get(&format!("{base}a.txt"), "11-20").is_some());

        // Past the ceiling the oldest entry goes; the newest stays.
        for index in 0..SNAPSHOT_CACHE_ENTRIES {
            cache_insert(
                &format!("{base}fill{index}.txt"),
                "1-1",
                &Snapshot::default(),
            );
        }
        cache_insert(&format!("{base}overflow.txt"), "1-1", &Snapshot::default());
        assert!(cache_get(&format!("{base}overflow.txt"), "1-1").is_some());
        assert!(cache_get(&format!("{base}a.txt"), "11-20").is_none());
    }

    #[test]
    fn local_code_and_markdown_keep_their_markup_instead_of_falling_back_to_hex() {
        let root = std::env::temp_dir().join(format!("dae-preview-markup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let code = root.join("main.rs");
        std::fs::write(&code, b"fn main() { let x = 1; }\n").unwrap();
        let md = root.join("README.md");
        std::fs::write(&md, b"# Title\n\nhello **world**\n").unwrap();
        let bin = root.join("blob.bin");
        std::fs::write(&bin, [0x00, 0x01, 0x02, 0x7f]).unwrap();

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let code_path = code.to_string_lossy().into_owned();
            let props = service::get_file_properties(code_path.clone()).await.unwrap();
            let snap = snapshot_body(props, &code_path, true).await;
            assert_eq!(snap.source_kind, SourceKind::Code("rs".to_owned()));
            assert!(
                snap.content_html.contains("<span style=\"color:"),
                "expected syntax highlight spans, got: {}",
                snap.content_html
            );

            let md_path = md.to_string_lossy().into_owned();
            let props = service::get_file_properties(md_path.clone()).await.unwrap();
            let snap = snapshot_body(props, &md_path, true).await;
            assert_eq!(snap.source_kind, SourceKind::Markdown);
            assert!(
                snap.markdown_blocks
                    .iter()
                    .any(|block| block.html.contains("<b>world</b>")),
                "expected virtualized markdown blocks, got: {:?}",
                snap.markdown_blocks
            );

            let bin_path = bin.to_string_lossy().into_owned();
            let props = service::get_file_properties(bin_path.clone()).await.unwrap();
            let snap = snapshot_body(props, &bin_path, true).await;
            assert_eq!(snap.source_kind, SourceKind::Code(String::new()));
            assert!(snap.content.starts_with("00000000  00 01 02 7f"));
        });

        let _ = std::fs::remove_dir_all(&root);
    }
}
