//! Virtualized block model for the Markdown preview.
//!
//! Markdown is parsed into blocks on the background preview lane (`preview_markup`)
//! and published here as an `Arc<Vec<MarkdownBlock>>` keyed by an integer
//! revision, so switching files or scrolling never copies the document through
//! QML or shapes the entire file on the GUI thread at once.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use qtbridge::qtbridge_type_lib::{
    QByteArray, QHash_i32_QByteArray, QModelIndex, QString, QVariant,
};
use qtbridge::{QAbstractItemModel, QAbstractItemModelBase, QmlElement, qobject};

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

#[qobject(Base = QAbstractItemModel, NoQmlElement, ConvertToCamelCase)]
mod rows {
    use super::*;

    /// Virtualized Markdown blocks for `Inspector`'s `ListView`.
    #[derive(Default)]
    pub struct MarkdownModel {
        blocks: Arc<Vec<MarkdownBlock>>,
        count: i32,
    }

    impl QAbstractItemModel for MarkdownModel {
        fn index(&self, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex {
            if parent.is_valid() || column != 0 || row < 0 || row as usize >= self.blocks.len() {
                return QModelIndex::default();
            }
            self.create_index(row, column, 0)
        }

        fn parent(&self, _child: &QModelIndex) -> QModelIndex {
            QModelIndex::default()
        }

        fn row_count(&self, parent: &QModelIndex) -> i32 {
            if parent.is_valid() {
                0
            } else {
                self.blocks.len() as i32
            }
        }

        fn column_count(&self, parent: &QModelIndex) -> i32 {
            if parent.is_valid() { 0 } else { 1 }
        }

        fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
            if !index.is_valid() || index.column() != 0 || index.row() < 0 {
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

        fn role_names(&self) -> QHash_i32_QByteArray {
            let mut roles = QHash_i32_QByteArray::default();
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

        fn set_data(&mut self, _index: &QModelIndex, _value: &QVariant, _role: i32) -> bool {
            false
        }
    }

    impl MarkdownModel {
        qproperty!("count", Member = count, Notify = count_changed);

        #[qsignal]
        fn count_changed(&mut self);

        #[qslot]
        fn load_revision(&mut self, revision: i32) {
            let next = fetch_published(revision);
            self.replace_blocks(next);
        }

        #[qslot]
        fn clear(&mut self) {
            if self.blocks.is_empty() {
                return;
            }
            self.replace_blocks(Arc::default());
        }

        fn replace_blocks(&mut self, next: Arc<Vec<MarkdownBlock>>) {
            let count = next.len().min(i32::MAX as usize) as i32;
            self.begin_reset_model();
            self.blocks = next;
            self.end_reset_model();
            self.count = count;
            self.count_changed();
        }
    }
}

pub use rows::MarkdownModel;

impl QmlElement for MarkdownModel {
    const URI: &str = "dae";
    const ELEMENT_NAME: &str = "MarkdownModel";
    const MINOR_VERSION: u8 = 0;
    const MAJOR_VERSION: u8 = 1;
    const IS_SINGLETON: bool = false;
}
