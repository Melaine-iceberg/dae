//! The sidebar's rows, as one flat model with section names.
//!
//! Places, favourites, volumes and recents are four different domain calls
//! with four different lifetimes, but the sidebar draws them as one list. A
//! single `QAbstractListModel` keeps the QML side to one `ListView` and lets
//! `section.property` draw the headers, instead of four nested views that each
//! scroll independently.

use core::pin::Pin;
use std::path::Path;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use dae_core::fs::recents::list_recents;
use dae_core::fs::sidebar::{self, PlaceKind};

use crate::format;
use crate::runtime;

const SECTION_PLACES: &str = "位置";
const SECTION_FAVORITES: &str = "收藏";
const SECTION_VOLUMES: &str = "磁盘";
const SECTION_RECENTS: &str = "最近";

const ROLE_SECTION: i32 = 0x0200;
const ROLE_NAME: i32 = 0x0201;
const ROLE_PATH: i32 = 0x0202;
const ROLE_KIND: i32 = 0x0203;
const ROLE_DETAIL: i32 = 0x0204;
const ROLE_ICON: i32 = 0x0205;
const ROLE_IS_DIR: i32 = 0x0206;

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
        /// The navigation column's content, reachable from QML as
        /// `SidebarModel`.
        #[qobject]
        #[qml_element]
        #[base = QAbstractListModel]
        #[qproperty(bool, loaded)]
        #[qproperty(QString, status)]
        type SidebarModel = super::SidebarModelRust;

        #[cxx_override]
        #[rust_name = "row_count"]
        fn rowCount(&self, parent: &QModelIndex) -> i32;

        #[cxx_override]
        fn data(&self, index: &QModelIndex, role: i32) -> QVariant;

        #[cxx_override]
        #[rust_name = "role_names"]
        fn roleNames(&self) -> QHash_i32_QByteArray;

        /// Re-resolves every section. Disk probing happens off the Qt thread;
        /// the file-backed sections are small enough to read here.
        #[qinvokable]
        fn refresh(self: Pin<&mut SidebarModel>);

        #[inherit]
        #[rust_name = "begin_reset_model"]
        unsafe fn beginResetModel(self: Pin<&mut SidebarModel>);

        #[inherit]
        #[rust_name = "end_reset_model"]
        unsafe fn endResetModel(self: Pin<&mut SidebarModel>);
    }

    impl cxx_qt::Threading for SidebarModel {}
}

#[derive(Clone)]
struct SidebarRow {
    section: String,
    name: String,
    path: String,
    kind: String,
    detail: String,
    icon: String,
    is_dir: bool,
}

#[derive(Default)]
pub struct SidebarModelRust {
    rows: Vec<SidebarRow>,
    loaded: bool,
    status: QString,
}

impl qobject::SidebarModel {
    fn row_count(&self, parent: &QModelIndex) -> i32 {
        if parent.is_valid() {
            0
        } else {
            self.rows.len() as i32
        }
    }

    fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        if index.column() != 0 || index.parent().is_valid() {
            return QVariant::default();
        }
        let Some(row) = self.rows.get(index.row() as usize) else {
            return QVariant::default();
        };
        match role {
            ROLE_SECTION => QVariant::from(&QString::from(row.section.as_str())),
            ROLE_NAME => QVariant::from(&QString::from(row.name.as_str())),
            ROLE_PATH => QVariant::from(&QString::from(row.path.as_str())),
            ROLE_KIND => QVariant::from(&QString::from(row.kind.as_str())),
            ROLE_DETAIL => QVariant::from(&QString::from(row.detail.as_str())),
            ROLE_ICON => QVariant::from(&QString::from(row.icon.as_str())),
            ROLE_IS_DIR => QVariant::from(&row.is_dir),
            _ => QVariant::default(),
        }
    }

    fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        roles.insert(ROLE_SECTION, QByteArray::from("section"));
        roles.insert(ROLE_NAME, QByteArray::from("name"));
        roles.insert(ROLE_PATH, QByteArray::from("path"));
        roles.insert(ROLE_KIND, QByteArray::from("kind"));
        roles.insert(ROLE_DETAIL, QByteArray::from("detail"));
        roles.insert(ROLE_ICON, QByteArray::from("icon"));
        roles.insert(ROLE_IS_DIR, QByteArray::from("isDir"));
        roles
    }

    fn refresh(mut self: Pin<&mut Self>) {
        self.as_mut().set_status(QString::from("读取中…"));

        let mut base = Vec::new();
        for place in sidebar::system_places() {
            base.push(place_row(place.kind, &place.path));
        }
        for favorite in sidebar::load_favorites().unwrap_or_default() {
            base.push(SidebarRow {
                section: SECTION_FAVORITES.to_owned(),
                name: favorite.name,
                path: favorite.path,
                kind: "favorite".to_owned(),
                detail: String::new(),
                icon: "star".to_owned(),
                is_dir: true,
            });
        }

        let mut recent = Vec::new();
        for item in list_recents().unwrap_or_default().into_iter().take(12) {
            recent.push(SidebarRow {
                section: SECTION_RECENTS.to_owned(),
                name: item.name,
                path: item.path,
                kind: "recent".to_owned(),
                detail: String::new(),
                icon: "recent".to_owned(),
                is_dir: item.kind == dae_core::types::EntryKind::Directory,
            });
        }

        let thread = self.as_mut().qt_thread();
        runtime::spawn(async move {
            let volumes = sidebar::list_disks().await.unwrap_or_default();
            let mut rows = base;
            for volume in volumes {
                let name = volume_name(&volume.name, &volume.mount_point);
                rows.push(SidebarRow {
                    section: SECTION_VOLUMES.to_owned(),
                    name,
                    path: volume.mount_point,
                    kind: "volume".to_owned(),
                    detail: format!(
                        "{} / {}",
                        format::size(volume.available_bytes),
                        format::size(volume.total_bytes)
                    ),
                    icon: "disk".to_owned(),
                    is_dir: true,
                });
            }
            rows.extend(recent);
            let _ = thread.queue(move |mut model| model.as_mut().replace_rows(rows));
        });
    }

    fn replace_rows(mut self: Pin<&mut Self>, rows: Vec<SidebarRow>) {
        unsafe {
            self.as_mut().begin_reset_model();
            self.as_mut().rust_mut().rows = rows;
            self.as_mut().end_reset_model();
        }
        let count = self.rows.len();
        self.as_mut()
            .set_status(QString::from(format!("{count} 项").as_str()));
        self.as_mut().set_loaded(true);
    }
}

fn place_row(kind: PlaceKind, path: &str) -> SidebarRow {
    let (name, icon) = match kind {
        PlaceKind::Home => ("主目录", "home"),
        PlaceKind::Desktop => ("桌面", "desktop"),
        PlaceKind::Documents => ("文档", "documents"),
        PlaceKind::Downloads => ("下载", "downloads"),
        PlaceKind::Pictures => ("图片", "pictures"),
        PlaceKind::Music => ("音乐", "music"),
        PlaceKind::Videos => ("视频", "videos"),
    };
    SidebarRow {
        section: SECTION_PLACES.to_owned(),
        name: name.to_owned(),
        path: path.to_owned(),
        kind: "place".to_owned(),
        detail: String::new(),
        icon: icon.to_owned(),
        is_dir: true,
    }
}

/// The volume name is the label the OS reports; a drive with no label falls
/// back to its mount point so the row is never blank.
fn volume_name(label: &str, mount_point: &str) -> String {
    let label = label.trim();
    if label.is_empty() {
        Path::new(mount_point)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| mount_point.to_owned())
    } else {
        label.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_system_place_has_a_name_and_an_icon() {
        for kind in [
            PlaceKind::Home,
            PlaceKind::Desktop,
            PlaceKind::Documents,
            PlaceKind::Downloads,
            PlaceKind::Pictures,
            PlaceKind::Music,
            PlaceKind::Videos,
        ] {
            let row = place_row(kind, "/tmp");
            assert!(!row.name.is_empty());
            assert!(!row.icon.is_empty());
            assert_eq!(row.section, SECTION_PLACES);
            assert!(row.is_dir);
        }
    }

    #[test]
    fn an_unlabelled_volume_falls_back_to_its_mount_point() {
        assert_eq!(volume_name("  ", "D:/"), "D:/");
        assert_eq!(volume_name("Data", "D:/"), "Data");
    }
}
