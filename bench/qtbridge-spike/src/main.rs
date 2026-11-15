//! Feasibility gate for the CXX-Qt → Qt Bridge migration.
//!
//! The port touches three things Qt Bridge does not model directly, so this
//! binary proves each one against the Qt the shell is built with (6.12):
//!
//! 1. a `#[qobject]` type with properties, signals and slots, plus `QApp`;
//! 2. a `QAbstractItemModel` with named roles, which is what every listing in
//!    `crates/dae-qt` is;
//! 3. C++ that has to stay in C++ — a `QQuickImageProvider` reached through
//!    `QApp::qml_engine()`, and a `QQuickItem` registered as a QML type.
//!
//! Prints one line per mark and exits through QML after two seconds.

use qtbridge::qtbridge_type_lib::{
    QByteArray, QHash_i32_QByteArray, QModelIndex, QString, QVariant,
};
use qtbridge::{QAbstractItemModel, QAbstractItemModelBase, QApp, QmlElement, qobject};

const ROLE_FILE_NAME: i32 = 0x0100;
const ROLE_SIZE_TEXT: i32 = 0x0101;

// ---------------------------------------------------------------------------
// 1. Properties, signals, slots.
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct Backend {
    count: i32,
    marks: u32,
}

#[qobject(NoQmlElement)]
impl Backend {
    qproperty!("count", Member = count, Notify = count_changed);

    #[qsignal]
    fn count_changed(&mut self);

    /// Records one visible seam. Returns the mark count so QML can check that
    /// every seam landed, not just that the process survived.
    #[qslot]
    fn mark(&mut self, what: String) -> i32 {
        self.marks += 1;
        self.count = self.marks as i32;
        self.count_changed();
        println!("[spike] mark {}/6: {what}", self.marks);
        self.marks as i32
    }
}

impl QmlElement for Backend {
    // The port keeps the shell's existing module and element names, so none of
    // the 30 QML files change: `import dae 1.0` keeps resolving `Backend`.
    const URI: &str = "dae";
    const ELEMENT_NAME: &str = "Backend";
    const MINOR_VERSION: u8 = 0;
    const MAJOR_VERSION: u8 = 1;
    const IS_SINGLETON: bool = true;
}

// ---------------------------------------------------------------------------
// 2. Abstract item model with explicit roles.
// ---------------------------------------------------------------------------

#[qobject(Base = QAbstractItemModel, NoQmlElement)]
mod rows {
    use super::*;

    pub struct Rows {
        names: Vec<(String, u64)>,
    }

    impl Default for Rows {
        fn default() -> Self {
            Self {
                names: vec![
                    ("alpha.txt".to_owned(), 12),
                    ("beta.png".to_owned(), 4096),
                    ("gamma".to_owned(), 0),
                ],
            }
        }
    }

    impl QAbstractItemModel for Rows {
        fn index(&self, row: i32, column: i32, _parent: &QModelIndex) -> QModelIndex {
            if row < 0 || row as usize >= self.names.len() {
                return QModelIndex::default();
            }
            let ptr = &self.names[row as usize] as *const (String, u64) as usize;
            self.create_index(row, column, ptr)
        }

        fn parent(&self, _child: &QModelIndex) -> QModelIndex {
            QModelIndex::default()
        }

        fn row_count(&self, _parent: &QModelIndex) -> i32 {
            self.names.len() as i32
        }

        fn column_count(&self, _parent: &QModelIndex) -> i32 {
            1
        }

        fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
            let Some(row) = self.names.get(index.row() as usize) else {
                return QVariant::default();
            };
            match role {
                ROLE_FILE_NAME => QVariant::from(&QString::from(row.0.as_str())),
                ROLE_SIZE_TEXT => QVariant::from(&QString::from(row.1.to_string().as_str())),
                _ => QVariant::default(),
            }
        }

        fn role_names(&self) -> QHash_i32_QByteArray {
            let mut roles = QHash_i32_QByteArray::default();
            roles.insert(ROLE_FILE_NAME, QByteArray::from("fileName"));
            roles.insert(ROLE_SIZE_TEXT, QByteArray::from("sizeText"));
            roles
        }

        fn set_data(&mut self, _index: &QModelIndex, _value: &QVariant, _role: i32) -> bool {
            false
        }
    }
}

pub use rows::Rows;

impl QmlElement for Rows {
    const URI: &str = "dae";
    const ELEMENT_NAME: &str = "Rows";
    const MINOR_VERSION: u8 = 0;
    const MAJOR_VERSION: u8 = 1;
    const IS_SINGLETON: bool = false;
}

// ---------------------------------------------------------------------------
// 3. The C++ half.
// ---------------------------------------------------------------------------

unsafe extern "C" {
    fn spike_register_providers(engine: *mut core::ffi::c_void);
    fn spike_register_item();
}

const QML: &str = r##"
import QtQuick
import dae 1.0
import qtbridge_spike.cpp

Window {
    id: root
    visible: true
    width: 320
    height: 240
    color: "#101216"

    Image {
        id: icon
        width: 4
        height: 4
        source: "image://spike/test"
        onStatusChanged: if (status === Image.Ready)
            Backend.mark("image:" + sourceSize.width + "x" + sourceSize.height)
    }

    SpikeItem {
        id: item
        Component.onCompleted: Backend.mark("cpp-item:" + item.answer + ":" + item.label)
    }

    ListView {
        id: view
        width: 200
        height: 60
        model: Rows {}
        delegate: Text {
            required property string fileName
            required property string sizeText
            text: fileName + " " + sizeText
            color: "#f7f7f9"
        }
        onCountChanged: if (count > 0)
            Backend.mark("model-count:" + count)
    }

    Component.onCompleted: Backend.mark("qml-count:" + Backend.count)

    Timer {
        interval: 2000
        running: true
        onTriggered: {
            var first = view.itemAtIndex(0)
            Backend.mark("model-row:" + (first ? first.text : "missing"))
            Qt.quit()
        }
    }
}
"##;

fn main() {
    // No `linkme`: the registration order is explicit, and the same call can
    // sit in `dae_qt::init()` in the real port.
    Backend::register();
    Rows::register();

    let mut app = QApp::new();

    // SAFETY: the C++ side only installs an image provider and a QML type; the
    // engine lives for the whole call and outlives them.
    unsafe {
        let engine = app.qml_engine();
        let engine_ptr =
            std::pin::Pin::get_unchecked_mut(engine) as *mut _ as *mut core::ffi::c_void;
        spike_register_providers(engine_ptr);
        spike_register_item();
    }

    app.load_qml(QML.as_bytes()).run();
}
