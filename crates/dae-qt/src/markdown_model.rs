//! Virtualized block model for the Markdown preview.
//!
//! Markdown is parsed into blocks on the background preview lane (`preview_markup`)
//! and published here as an `Arc<Vec<MarkdownBlock>>` keyed by an integer
//! revision, so switching files or scrolling never copies the document through
//! QML or shapes the entire file on the GUI thread at once.

use core::pin::Pin;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};

use crate::preview_markup::{MarkdownBlock, MarkdownBlockKind};

const ROLE_KIND: i32 = 0x0400;
const ROLE_HTML: i32 = 0x0401;
const ROLE_LEVEL: i32 = 0x0402;
const ROLE_MARKER: i32 = 0x0403;
const ROLE_LANGUAGE: i32 = 0x0404;
const ROLE_RAW_TEXT: i32 = 0x0405;
const ROLE_IMAGE_URL: i32 = 0x0406;
const ROLE_ALT: i32 = 0x0407;

const PUBLISHED_CAP: usize = 16;

fn published_store() -> &'static Mutex<VecDeque<(i32, Arc<Vec<MarkdownBlock>>)>> {
    static STORE: OnceLock<Mutex<VecDeque<(i32, Arc<Vec<MarkdownBlock>>)>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(VecDeque::with_capacity(PUBLISHED_CAP)))
}

fn next_revision() -> i32 {
    static NEXT: AtomicI32 = AtomicI32::new(1);
    loop {
        let current = NEXT.load(Ordering::Relaxed);
        let next = if current == i32::MAX { 1 } else { current + 1 };
        if NEXT
            .compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return current;
        }
    }
}

/// Registers a parsed Markdown block list and returns a non-zero revision id
/// that `MarkdownModel.loadRevision` resolves in O(1).
pub(crate) fn publish(blocks: Arc<Vec<MarkdownBlock>>) -> i32 {
    let revision = next_revision();
    let mut store = published_store()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if store.len() >= PUBLISHED_CAP {
        store.pop_front();
    }
    store.push_back((revision, blocks));
    revision
}

fn fetch_published(revision: i32) -> Arc<Vec<MarkdownBlock>> {
    if revision <= 0 {
        return Arc::default();
    }
    let store = published_store()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    store
        .iter()
        .rev()
        .find(|(id, _)| *id == revision)
        .map(|(_, blocks)| Arc::clone(blocks))
        .unwrap_or_default()
}

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!(<QAbstractListModel>);
        type QAbstractListModel;

        include!("cxx-qt-lib/qmodelindex.h");
        type QModelIndex = cxx_qt_lib::QModelIndex;

        include!("cxx-qt-lib/qvariant.h");
        type QVariant = cxx_qt_lib::QVariant;

        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("cxx-qt-lib/qhash.h");
        type QHash_i32_QByteArray = cxx_qt_lib::QHash<cxx_qt_lib::QHashPair_i32_QByteArray>;
    }

    unsafe extern "RustQt" {
        /// Virtualized Markdown blocks for `Inspector`'s `ListView`.
        #[qobject]
        #[qml_element]
        #[base = QAbstractListModel]
        #[qproperty(i32, count)]
        type MarkdownModel = super::MarkdownModelRust;

        #[cxx_override]
        #[rust_name = "row_count"]
        fn rowCount(&self, parent: &QModelIndex) -> i32;

        #[cxx_override]
        fn data(&self, index: &QModelIndex, role: i32) -> QVariant;

        #[cxx_override]
        #[rust_name = "role_names"]
        fn roleNames(&self) -> QHash_i32_QByteArray;

        #[qinvokable]
        #[cxx_name = "loadRevision"]
        fn load_revision(self: Pin<&mut MarkdownModel>, revision: i32);

        #[qinvokable]
        fn clear(self: Pin<&mut MarkdownModel>);

        #[inherit]
        #[rust_name = "begin_reset_model"]
        unsafe fn beginResetModel(self: Pin<&mut MarkdownModel>);

        #[inherit]
        #[rust_name = "end_reset_model"]
        unsafe fn endResetModel(self: Pin<&mut MarkdownModel>);
    }
}

#[derive(Default)]
pub struct MarkdownModelRust {
    blocks: Arc<Vec<MarkdownBlock>>,
    count: i32,
}

impl qobject::MarkdownModel {
    fn row_count(&self, parent: &QModelIndex) -> i32 {
        if parent.is_valid() {
            0
        } else {
            self.blocks.len() as i32
        }
    }

    fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        if index.column() != 0 || index.parent().is_valid() {
            return QVariant::default();
        }
        let Some(block) = self.blocks.get(index.row() as usize) else {
            return QVariant::default();
        };
        match role {
            ROLE_KIND => QVariant::from(&QString::from(match block.kind {
                MarkdownBlockKind::Heading => "heading",
                MarkdownBlockKind::Paragraph => "paragraph",
                MarkdownBlockKind::Quote => "quote",
                MarkdownBlockKind::ListItem => "listItem",
                MarkdownBlockKind::Code => "code",
                MarkdownBlockKind::Table => "table",
                MarkdownBlockKind::Image => "image",
                MarkdownBlockKind::Rule => "rule",
            })),
            ROLE_HTML => QVariant::from(&QString::from(block.html.as_str())),
            ROLE_LEVEL => QVariant::from(&block.level),
            ROLE_MARKER => QVariant::from(&QString::from(block.marker.as_str())),
            ROLE_LANGUAGE => QVariant::from(&QString::from(block.language.as_str())),
            ROLE_RAW_TEXT => QVariant::from(&QString::from(block.raw_text.as_str())),
            ROLE_IMAGE_URL => QVariant::from(&QString::from(block.image_url.as_str())),
            ROLE_ALT => QVariant::from(&QString::from(block.alt.as_str())),
            _ => QVariant::default(),
        }
    }

    fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        roles.insert(ROLE_KIND, QByteArray::from("kind"));
        roles.insert(ROLE_HTML, QByteArray::from("html"));
        roles.insert(ROLE_LEVEL, QByteArray::from("level"));
        roles.insert(ROLE_MARKER, QByteArray::from("marker"));
        roles.insert(ROLE_LANGUAGE, QByteArray::from("language"));
        roles.insert(ROLE_RAW_TEXT, QByteArray::from("rawText"));
        roles.insert(ROLE_IMAGE_URL, QByteArray::from("imageUrl"));
        roles.insert(ROLE_ALT, QByteArray::from("alt"));
        roles
    }

    fn load_revision(mut self: Pin<&mut Self>, revision: i32) {
        let next = fetch_published(revision);
        self.as_mut().replace_blocks(next);
    }

    fn clear(mut self: Pin<&mut Self>) {
        if self.blocks.is_empty() {
            return;
        }
        self.as_mut().replace_blocks(Arc::default());
    }

    fn replace_blocks(mut self: Pin<&mut Self>, next: Arc<Vec<MarkdownBlock>>) {
        let count = next.len().min(i32::MAX as usize) as i32;
        unsafe {
            self.as_mut().begin_reset_model();
        }
        self.as_mut().rust_mut().blocks = next;
        unsafe {
            self.as_mut().end_reset_model();
        }
        self.as_mut().set_count(count);
    }
}
