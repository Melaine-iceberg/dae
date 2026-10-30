//! The sidebar's rows, as one flat model with section names.
//!
//! Places, favourites, volumes and cloud accounts are different domain calls
//! with different lifetimes, but the sidebar draws them as one list. A
//! single `QAbstractListModel` keeps the QML side to one `ListView` and lets
//! `section.property` draw the headers, instead of several nested views that
//! each scroll independently.
//!
//! Section order is deliberate: the five primary places first, then the extra
//! places and favourites, then volumes and spaces (a disk is closer to "where
//! am I" than a saved space), then the network group — WSL distros are network
//! locations too — and finally cloud accounts. The trash is not a row here;
//! the sidebar parks it at the bottom itself.

use core::pin::Pin;
use std::path::Path;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use dae_core::fs::cloud::accounts::{StoredCloudAccount, list_cloud_accounts};
use dae_core::fs::cloud::provider::CloudProviderKind;
use dae_core::fs::connections::{self, Protocol, StoredConnection};
use dae_core::fs::sidebar::{self, PlaceKind, WslDistro};
use dae_core::fs::spaces::list_spaces;

use crate::format;
use crate::runtime;

const SECTION_PLACES: &str = "Places";
const SECTION_FAVORITES: &str = "Favorites";
const SECTION_SPACES: &str = "Spaces";
const SECTION_VOLUMES: &str = "Disks";
const SECTION_NETWORK: &str = "Network Storage";
const SECTION_CLOUD: &str = "Cloud Storage";

const ROLE_SECTION: i32 = 0x0200;
const ROLE_NAME: i32 = 0x0201;
const ROLE_PATH: i32 = 0x0202;
const ROLE_KIND: i32 = 0x0203;
const ROLE_DETAIL: i32 = 0x0204;
const ROLE_ICON: i32 = 0x0205;
const ROLE_IS_DIR: i32 = 0x0206;
const ROLE_FILE_SYSTEM: i32 = 0x0207;
const ROLE_FILL: i32 = 0x0106;

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
    /// How full a volume is, 0..1. Zero for everything that is not a volume:
    /// the bar is drawn only when there is a number behind it.
    fill: f64,
    /// NTFS, APFS, ext4… Empty for everything that is not a volume.
    file_system: String,
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
            ROLE_FILE_SYSTEM => QVariant::from(&QString::from(row.file_system.as_str())),
            ROLE_FILL => QVariant::from(&row.fill),
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
        roles.insert(ROLE_FILE_SYSTEM, QByteArray::from("fileSystem"));
        roles.insert(ROLE_FILL, QByteArray::from("fill"));
        roles
    }

    fn refresh(mut self: Pin<&mut Self>) {
        self.as_mut().set_status(QString::from("Loading…"));
        // A sidebar may be created before the command singleton; both stores
        // are process-wide and loading them here is idempotent.
        crate::init_stores();

        let mut base = Vec::new();
        let hidden = sidebar::load_hidden_places().unwrap_or_default();
        let mut places = sidebar::system_places();
        // `system_places` reports the OS's resolution order, which on Windows
        // puts Documents before Downloads. The sidebar's first five rows are
        // the destinations a user reaches for in the order they reach for
        // them, so they are ranked here instead of trusting the probe.
        places.sort_by_key(|place| place_rank(place.kind));
        for place in places {
            if hidden.contains(&place.kind) {
                continue;
            }
            base.push(place_row(place.kind, &place.path));
        }
        // Hiding is the one place action that removes its own row, so the way
        // back has to be a row of its own or it would not exist anywhere.
        if !hidden.is_empty() {
            base.push(SidebarRow {
                section: SECTION_PLACES.to_owned(),
                name: "Restore hidden places".to_owned(),
                path: String::new(),
                kind: "restorePlaces".to_owned(),
                detail: format!("{} hidden", hidden.len()),
                icon: "eye".to_owned(),
                is_dir: false,
                fill: 0.0,
                file_system: String::new(),
            });
        }
        let favorites = sidebar::load_favorites().unwrap_or_default();
        if favorites.is_empty() {
            // The hint is the favorites section's drop target: a folder
            // dragged onto it is pinned, and the section stays discoverable
            // before anything has been pinned.
            base.push(SidebarRow {
                section: SECTION_FAVORITES.to_owned(),
                name: "Drop folders here to favorite them".to_owned(),
                path: String::new(),
                kind: "favoriteHint".to_owned(),
                detail: String::new(),
                icon: "star".to_owned(),
                is_dir: false,
                fill: 0.0,
                file_system: String::new(),
            });
        } else {
            for favorite in favorites {
                base.push(SidebarRow {
                    section: SECTION_FAVORITES.to_owned(),
                    name: favorite.name,
                    path: favorite.path,
                    kind: "favorite".to_owned(),
                    detail: String::new(),
                    icon: "star".to_owned(),
                    is_dir: true,
                    fill: 0.0,
                    file_system: String::new(),
                });
            }
        }

        // Spaces are first-class destinations, not folders: the `path` field
        // carries the space id and the shell opens the space surface instead of
        // navigating. The action row stays below the list so the section is
        // still discoverable before the user has anything pinned. An empty
        // space carries no meta text: "空" on every row said nothing the place
        // itself did not.
        let mut spaces = Vec::new();
        for space in list_spaces().unwrap_or_default() {
            spaces.push(SidebarRow {
                section: SECTION_SPACES.to_owned(),
                name: space.name,
                path: space.id,
                kind: "space".to_owned(),
                detail: if space.items.is_empty() {
                    String::new()
                } else {
                    let count = space.items.len();
                    format!("{count} item{}", if count == 1 { "" } else { "s" })
                },
                icon: "space".to_owned(),
                is_dir: true,
                fill: 0.0,
                file_system: String::new(),
            });
        }
        spaces.push(action_row(SECTION_SPACES, "New space…", "addSpace"));

        let mut network = Vec::new();
        for connection in connections::list_connections().unwrap_or_default() {
            network.push(connection_row(connection));
        }
        // The add row keeps the group reachable while it is empty, and stays
        // at the end once there is something to manage. The WSL rows are
        // inserted ahead of this list when the async probe returns, because a
        // distro is also a network location.
        network.push(action_row(
            SECTION_NETWORK,
            "Connect network storage…",
            "addConnection",
        ));

        let mut cloud = Vec::new();
        for account in list_cloud_accounts().unwrap_or_default() {
            cloud.push(cloud_row(account));
        }
        cloud.push(action_row(SECTION_CLOUD, "Add cloud account…", "addCloud"));

        let thread = self.as_mut().qt_thread();
        runtime::spawn(async move {
            let volumes = sidebar::list_disks().await.unwrap_or_default();
            // Empty everywhere but Windows, which also makes the rows
            // disappear on the other platforms.
            let distros = sidebar::list_wsl_distros().await.unwrap_or_default();
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
                    fill: if volume.total_bytes == 0 {
                        0.0
                    } else {
                        ((volume.total_bytes - volume.available_bytes) as f64
                            / volume.total_bytes as f64)
                            .clamp(0.0, 1.0)
                    },
                    file_system: volume.file_system,
                });
            }
            rows.extend(spaces);
            for distro in distros {
                rows.push(distro_row(distro));
            }
            rows.extend(network);
            rows.extend(cloud);
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
        self.as_mut().set_status(QString::from(
            format!("{count} item{}", if count == 1 { "" } else { "s" }).as_str(),
        ));
        self.as_mut().set_loaded(true);
    }
}

/// The order of the rows inside the Places section. The first five are the
/// primary destinations; Music and Videos keep their rows but sit after them,
/// which is also what keeps the first screen to ten rows on a fresh profile.
fn place_rank(kind: PlaceKind) -> u8 {
    match kind {
        PlaceKind::Home => 0,
        PlaceKind::Desktop => 1,
        PlaceKind::Downloads => 2,
        PlaceKind::Documents => 3,
        PlaceKind::Pictures => 4,
        PlaceKind::Music => 5,
        PlaceKind::Videos => 6,
    }
}

fn place_row(kind: PlaceKind, path: &str) -> SidebarRow {
    let (name, icon) = match kind {
        PlaceKind::Home => ("Home", "home"),
        PlaceKind::Desktop => ("Desktop", "desktop"),
        PlaceKind::Documents => ("Documents", "documents"),
        PlaceKind::Downloads => ("Downloads", "downloads"),
        PlaceKind::Pictures => ("Pictures", "pictures"),
        PlaceKind::Music => ("Music", "music"),
        PlaceKind::Videos => ("Videos", "videos"),
    };
    SidebarRow {
        section: SECTION_PLACES.to_owned(),
        name: name.to_owned(),
        path: path.to_owned(),
        kind: "place".to_owned(),
        detail: String::new(),
        icon: icon.to_owned(),
        is_dir: true,
        fill: 0.0,
        file_system: String::new(),
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

/// Each distro is reachable at its `\\wsl$\<name>` share. Accessing that path
/// starts a stopped distro, so the row needs no separate "start" action. It
/// lives in the network section: a distro is reached over the same WSL network
/// share as an SMB mount is reached over TCP.
fn distro_row(distro: WslDistro) -> SidebarRow {
    SidebarRow {
        section: SECTION_NETWORK.to_owned(),
        name: distro.name,
        path: distro.path,
        kind: "wsl".to_owned(),
        detail: String::new(),
        icon: "wsl".to_owned(),
        is_dir: true,
        fill: 0.0,
        file_system: String::new(),
    }
}

fn connection_row(connection: StoredConnection) -> SidebarRow {
    let protocol = protocol_label(connection.protocol);
    let detail = match connection.port {
        Some(port) => format!("{protocol} · {port}"),
        None => protocol.to_owned(),
    };
    SidebarRow {
        section: SECTION_NETWORK.to_owned(),
        name: connection.host,
        path: connection.id,
        kind: "connection".to_owned(),
        detail,
        icon: "server".to_owned(),
        is_dir: true,
        fill: 0.0,
        file_system: String::new(),
    }
}

fn cloud_row(account: StoredCloudAccount) -> SidebarRow {
    SidebarRow {
        section: SECTION_CLOUD.to_owned(),
        name: account.email,
        path: account.id,
        kind: "cloud".to_owned(),
        detail: cloud_provider_label(account.provider).to_owned(),
        icon: "cloud".to_owned(),
        is_dir: true,
        fill: 0.0,
        file_system: String::new(),
    }
}

fn action_row(section: &str, name: &str, kind: &str) -> SidebarRow {
    SidebarRow {
        section: section.to_owned(),
        name: name.to_owned(),
        path: String::new(),
        kind: kind.to_owned(),
        detail: String::new(),
        icon: "add".to_owned(),
        is_dir: true,
        fill: 0.0,
        file_system: String::new(),
    }
}

fn protocol_label(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::Smb => "SMB",
        Protocol::Sftp => "SFTP",
        Protocol::Ftp => "FTP",
        Protocol::WebDav => "WebDAV",
    }
}

fn cloud_provider_label(provider: CloudProviderKind) -> &'static str {
    match provider {
        CloudProviderKind::GoogleDrive => "Google Drive",
        CloudProviderKind::Onedrive => "OneDrive",
        CloudProviderKind::Dropbox => "Dropbox",
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
        assert_eq!(volume_name("  ", "/media/Data"), "Data");
        // A root has no file name, so the mount point itself is the name.
        assert_eq!(volume_name("  ", "/"), "/");
        // On a Windows drive root the prefix is not a file name either; only
        // there does the raw mount point keep its spelling.
        #[cfg(windows)]
        assert_eq!(volume_name("  ", "D:/"), "D:/");
        assert_eq!(volume_name("Data", "D:/"), "Data");
    }

    #[test]
    fn the_primary_places_keep_their_reach_order() {
        let mut kinds = [
            PlaceKind::Videos,
            PlaceKind::Documents,
            PlaceKind::Home,
            PlaceKind::Pictures,
            PlaceKind::Downloads,
            PlaceKind::Music,
            PlaceKind::Desktop,
        ];
        kinds.sort_by_key(|kind| place_rank(*kind));

        assert_eq!(
            kinds,
            [
                PlaceKind::Home,
                PlaceKind::Desktop,
                PlaceKind::Downloads,
                PlaceKind::Documents,
                PlaceKind::Pictures,
                PlaceKind::Music,
                PlaceKind::Videos,
            ]
        );
    }

    #[test]
    fn a_wsl_distro_becomes_a_navigable_row() {
        let row = distro_row(WslDistro {
            name: "Ubuntu".into(),
            path: r"\\wsl$\Ubuntu".into(),
        });

        assert_eq!(row.section, SECTION_NETWORK);
        assert_eq!(row.name, "Ubuntu");
        assert_eq!(row.path, r"\\wsl$\Ubuntu");
        assert_eq!(row.kind, "wsl");
        assert_eq!(row.icon, "wsl");
        assert!(row.is_dir);
    }

    #[test]
    fn a_saved_connection_becomes_a_network_row() {
        let row = connection_row(StoredConnection {
            id: "sftp://nas.local:2222".into(),
            protocol: Protocol::Sftp,
            host: "nas.local".into(),
            port: Some(2222),
            username: Some("alice".into()),
        });

        assert_eq!(row.section, SECTION_NETWORK);
        assert_eq!(row.name, "nas.local");
        assert_eq!(row.path, "sftp://nas.local:2222");
        assert_eq!(row.kind, "connection");
        assert_eq!(row.detail, "SFTP · 2222");
        assert!(row.is_dir);
    }

    #[test]
    fn a_cloud_account_becomes_a_cloud_row() {
        let row = cloud_row(StoredCloudAccount {
            id: "cloud://google_drive:user@example.com".into(),
            provider: CloudProviderKind::GoogleDrive,
            email: "user@example.com".into(),
            display_name: "User".into(),
        });

        assert_eq!(row.section, SECTION_CLOUD);
        assert_eq!(row.name, "user@example.com");
        assert_eq!(row.detail, "Google Drive");
        assert_eq!(row.kind, "cloud");
    }

    #[test]
    fn add_rows_are_not_paths() {
        let row = action_row(SECTION_NETWORK, "Connect network storage…", "addConnection");
        assert_eq!(row.path, "");
        assert_eq!(row.icon, "add");
    }
}
