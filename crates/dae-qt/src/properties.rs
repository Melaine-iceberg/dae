//! The properties dialog's state.
//!
//! One controller per window, created by `PropertiesDialog.qml`. It reads one
//! entry's metadata through `dae_core::fs::service`, exposes the platform's
//! editable half (POSIX mode and owner, DOS attributes), and owns the two
//! streaming runs the dialog can start: a folder-size scan and a checksum pass.
//!
//! Both runs report through `dae_core::host::Host`, implemented here so their
//! progress lands on this object rather than on `Workspace`: the dialog is the
//! only consumer, and an operation id is all it takes to keep a stale run from
//! writing over a newer one.
//!
//! Every finish path clears `loading` last, so a binding that reads both sees a
//! complete snapshot instead of an intermediate state — the same ordering
//! `PreviewController` uses.

use std::pin::Pin;
use std::sync::Arc;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use dae_core::fs::service;
use dae_core::fs::vfs;
use dae_core::fs::{directory_size, hashing};
use dae_core::host::{Event, Host};
use dae_core::types::{
    EntryKind, FileProperties, OwnerChange, PlatformProperties, PropertyChanges,
};

use crate::errors::friendly_error;
use crate::format;
use crate::runtime;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    unsafe extern "RustQt" {
        /// QML usage: `PropertiesController { id: controller }` then
        /// `controller.open(path)`. The dialog binds to the properties; the
        /// two `start*`/`cancel*` pairs drive work that outlives a frame.
        #[qobject]
        #[qml_element]
        #[qproperty(QString, path)]
        #[qproperty(QString, name)]
        /// The containing folder, as the dialog's "位置" row spells it.
        #[qproperty(QString, location)]
        #[qproperty(QString, kind_text, cxx_name = "kindText")]
        /// A symlink's destination, empty for anything else.
        #[qproperty(QString, target_text, cxx_name = "targetText")]
        #[qproperty(QString, size_text, cxx_name = "sizeText")]
        #[qproperty(QString, created_text, cxx_name = "createdText")]
        #[qproperty(QString, modified_text, cxx_name = "modifiedText")]
        #[qproperty(QString, accessed_text, cxx_name = "accessedText")]
        #[qproperty(bool, is_directory, cxx_name = "isDirectory")]
        #[qproperty(bool, is_symlink, cxx_name = "isSymlink")]
        /// True for `smb://`, `sftp://` and `cloud://` paths. Their metadata is
        /// readable but nothing here is editable: attributes are a local file
        /// system idea and the checksum reader only opens local files.
        #[qproperty(bool, remote)]
        #[qproperty(bool, editable)]
        /// `unix`, `windows` or `basic` — which editor the dialog shows.
        #[qproperty(QString, platform_kind, cxx_name = "platformKind")]
        /// POSIX permission bits, low 12 only (`0o7777`).
        #[qproperty(i32, mode)]
        #[qproperty(QString, user_name, cxx_name = "userName")]
        #[qproperty(QString, group_name, cxx_name = "groupName")]
        #[qproperty(bool, read_only, cxx_name = "readOnly")]
        #[qproperty(bool, hidden)]
        #[qproperty(bool, archive)]
        #[qproperty(bool, system)]
        #[qproperty(bool, loading)]
        #[qproperty(QString, error)]
        #[qproperty(bool, saving)]
        #[qproperty(QString, save_error, cxx_name = "saveError")]
        #[qproperty(bool, size_scanning, cxx_name = "sizeScanning")]
        #[qproperty(QString, size_scan_text, cxx_name = "sizeScanText")]
        #[qproperty(f64, size_scan_bytes, cxx_name = "sizeScanBytes")]
        #[qproperty(bool, size_scan_done, cxx_name = "sizeScanDone")]
        #[qproperty(bool, hashing)]
        #[qproperty(f64, hash_progress, cxx_name = "hashProgress")]
        #[qproperty(QString, hash_md5, cxx_name = "hashMd5")]
        #[qproperty(QString, hash_sha1, cxx_name = "hashSha1")]
        #[qproperty(QString, hash_sha256, cxx_name = "hashSha256")]
        #[qproperty(QString, hash_error, cxx_name = "hashError")]
        #[qproperty(bool, hash_done, cxx_name = "hashDone")]
        type PropertiesController = super::PropertiesControllerRust;

        #[qinvokable]
        fn open(self: Pin<&mut PropertiesController>, path: &QString);

        /// Stops every run and forgets the snapshot. The dialog calls it when
        /// it closes, so a scan cannot outlive the window that started it.
        #[qinvokable]
        fn close(self: Pin<&mut PropertiesController>);

        /// Writes the edits back. `mode` is the low 12 bits, `user`/`group`
        /// empty means "unchanged", and `recursive` applies the same change to
        /// every enclosed entry of a local folder.
        #[qinvokable]
        fn save(
            self: Pin<&mut PropertiesController>,
            mode: i32,
            user: &QString,
            group: &QString,
            read_only: bool,
            hidden: bool,
            archive: bool,
            system: bool,
            recursive: bool,
        );

        #[qinvokable]
        #[cxx_name = "startSizeScan"]
        fn start_size_scan(self: Pin<&mut PropertiesController>);

        #[qinvokable]
        #[cxx_name = "cancelSizeScan"]
        fn cancel_size_scan(self: Pin<&mut PropertiesController>);

        #[qinvokable]
        #[cxx_name = "startHashing"]
        fn start_hashing(self: Pin<&mut PropertiesController>);

        #[qinvokable]
        #[cxx_name = "cancelHashing"]
        fn cancel_hashing(self: Pin<&mut PropertiesController>);

        #[qsignal]
        fn saved(self: Pin<&mut PropertiesController>);

        #[qsignal]
        #[cxx_name = "saveFailed"]
        fn save_failed(self: Pin<&mut PropertiesController>, message: &QString);
    }

    impl cxx_qt::Threading for PropertiesController {}
}

#[derive(Default)]
pub struct PropertiesControllerRust {
    path: QString,
    name: QString,
    location: QString,
    kind_text: QString,
    target_text: QString,
    size_text: QString,
    created_text: QString,
    modified_text: QString,
    accessed_text: QString,
    is_directory: bool,
    is_symlink: bool,
    remote: bool,
    editable: bool,
    platform_kind: QString,
    mode: i32,
    user_name: QString,
    group_name: QString,
    read_only: bool,
    hidden: bool,
    archive: bool,
    system: bool,
    loading: bool,
    error: QString,
    saving: bool,
    save_error: QString,
    size_scanning: bool,
    size_scan_text: QString,
    size_scan_bytes: f64,
    size_scan_done: bool,
    hashing: bool,
    hash_progress: f64,
    hash_md5: QString,
    hash_sha1: QString,
    hash_sha256: QString,
    hash_error: QString,
    hash_done: bool,
    /// The snapshot the edits are compared against, so a field the user did
    /// not touch never reaches `chmod`/`chown`/`SetFileAttributesW`.
    loaded: Option<FileProperties>,
    /// Bumped by every `open()`, so a slow read cannot paint over a newer one.
    generation: u64,
    size_operation: String,
    hash_operation: String,
    next_operation: u64,
}

/// Turns core events into queued Qt updates for one controller.
struct PropertiesHost {
    thread: cxx_qt::CxxQtThread<qobject::PropertiesController>,
}

impl Host for PropertiesHost {
    fn emit(&self, event: Event) {
        let _ = self
            .thread
            .queue(move |mut controller| controller.as_mut().apply_event(event));
    }
}

/// The platform's permission editor is only meaningful for local paths.
fn platform_kind(properties: &FileProperties, remote: bool) -> &'static str {
    if remote {
        return "basic";
    }
    match properties.platform {
        PlatformProperties::Unix(_) => "unix",
        PlatformProperties::Windows(_) => "windows",
        PlatformProperties::Basic => "basic",
    }
}

fn parent_of(path: &str) -> String {
    let index = path.rfind(['/', '\\']).unwrap_or(0);
    if index == 0 {
        return path.to_owned();
    }
    path[..index].to_owned()
}

fn kind_label(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Directory => "Folder",
        EntryKind::File => "File",
        EntryKind::Symlink => "Symbolic link",
        EntryKind::Other => "Other",
    }
}

impl PropertiesControllerRust {
    /// A fresh id per run: the dialog may start a scan, close it and start
    /// another before the first one's final event arrives.
    fn next_operation_id(&mut self, prefix: &str) -> String {
        self.next_operation = self.next_operation.wrapping_add(1);
        format!("qt-{prefix}-{}", self.next_operation)
    }
}

impl qobject::PropertiesController {
    fn open(mut self: Pin<&mut Self>, path: &QString) {
        let path = path.to_string();
        let generation = self.as_mut().rust_mut().generation.wrapping_add(1);
        self.as_mut().rust_mut().generation = generation;
        self.as_mut().rust_mut().loaded = None;

        self.as_mut().stop_runs();
        self.as_mut().set_path(QString::from(path.as_str()));
        self.as_mut().set_error(QString::default());
        self.as_mut().set_save_error(QString::default());
        self.as_mut().set_saving(false);
        self.as_mut().set_loading(true);
        self.as_mut().set_remote(!vfs::is_local_path(&path));
        self.as_mut().reset_size_scan();
        self.as_mut().reset_hash();

        let thread = self.as_mut().qt_thread();
        runtime::spawn(async move {
            let result = service::get_file_properties(path.clone()).await;
            let _ = thread.queue(move |mut controller| {
                controller.as_mut().apply_properties(generation, result)
            });
        });
    }

    fn close(mut self: Pin<&mut Self>) {
        let generation = self.as_mut().rust_mut().generation.wrapping_add(1);
        self.as_mut().rust_mut().generation = generation;
        self.as_mut().stop_runs();
        self.as_mut().rust_mut().loaded = None;
    }

    fn apply_properties(
        mut self: Pin<&mut Self>,
        generation: u64,
        result: Result<FileProperties, dae_core::error::FileSystemError>,
    ) {
        if self.generation != generation {
            return;
        }

        match result {
            Ok(properties) => {
                let remote = !vfs::is_local_path(&properties.path);
                self.as_mut()
                    .set_path(QString::from(properties.path.as_str()));
                self.as_mut()
                    .set_name(QString::from(properties.name.as_str()));
                self.as_mut()
                    .set_location(QString::from(parent_of(&properties.path).as_str()));
                self.as_mut()
                    .set_kind_text(QString::from(kind_label(properties.kind)));
                self.as_mut()
                    .set_target_text(QString::from(properties.target.as_deref().unwrap_or("")));
                self.as_mut().set_size_text(QString::from(
                    properties
                        .size
                        .map(format::size)
                        .unwrap_or_else(|| "—".to_owned())
                        .as_str(),
                ));
                self.as_mut().set_created_text(QString::from(
                    properties
                        .created_at
                        .map(format::timestamp)
                        .unwrap_or_else(|| "—".to_owned())
                        .as_str(),
                ));
                self.as_mut().set_modified_text(QString::from(
                    properties
                        .modified_at
                        .map(format::timestamp)
                        .unwrap_or_else(|| "—".to_owned())
                        .as_str(),
                ));
                self.as_mut().set_accessed_text(QString::from(
                    properties
                        .accessed_at
                        .map(format::timestamp)
                        .unwrap_or_else(|| "—".to_owned())
                        .as_str(),
                ));
                self.as_mut()
                    .set_is_directory(properties.kind == EntryKind::Directory);
                self.as_mut()
                    .set_is_symlink(properties.kind == EntryKind::Symlink);
                self.as_mut().set_remote(remote);

                let kind = platform_kind(&properties, remote);
                self.as_mut().set_platform_kind(QString::from(kind));
                self.as_mut().set_editable(!remote && kind != "basic");

                match &properties.platform {
                    PlatformProperties::Unix(unix) => {
                        self.as_mut().set_mode((unix.mode & 0o7777) as i32);
                        self.as_mut().set_user_name(QString::from(
                            unix.user_name
                                .clone()
                                .unwrap_or_else(|| unix.uid.to_string())
                                .as_str(),
                        ));
                        self.as_mut().set_group_name(QString::from(
                            unix.group_name
                                .clone()
                                .unwrap_or_else(|| unix.gid.to_string())
                                .as_str(),
                        ));
                    }
                    PlatformProperties::Windows(windows) => {
                        self.as_mut().set_read_only(windows.read_only);
                        self.as_mut().set_hidden(windows.hidden);
                        self.as_mut().set_archive(windows.archive);
                        self.as_mut().set_system(windows.system);
                    }
                    PlatformProperties::Basic => {}
                }

                self.as_mut().rust_mut().loaded = Some(properties);
                self.as_mut().set_error(QString::default());
            }
            Err(error) => {
                self.as_mut()
                    .set_error(QString::from(friendly_error(&error.to_string()).as_str()));
            }
        }
        self.as_mut().set_loading(false);
    }

    /// `open` on a new entry or `close` on the old one: both runs are about to
    /// become meaningless, and their ids are what cancels them.
    fn stop_runs(mut self: Pin<&mut Self>) {
        self.as_mut().cancel_size_scan();
        self.as_mut().cancel_hashing();
    }

    /// Builds the change set from the dialog's draft and the loaded snapshot.
    /// Empty owner fields are "leave alone" rather than an attempt to clear
    /// the account name.
    fn build_changes(
        &self,
        mode: i32,
        user: &str,
        group: &str,
        read_only: bool,
        hidden: bool,
        archive: bool,
        system: bool,
    ) -> PropertyChanges {
        let mut changes = PropertyChanges::default();
        let Some(properties) = self.loaded.as_ref() else {
            return changes;
        };

        match &properties.platform {
            PlatformProperties::Unix(unix) => {
                let wanted = (mode as u32) & 0o7777;
                if wanted != (unix.mode & 0o7777) {
                    changes.mode = Some(wanted);
                }

                let initial_user = unix
                    .user_name
                    .clone()
                    .unwrap_or_else(|| unix.uid.to_string());
                let initial_group = unix
                    .group_name
                    .clone()
                    .unwrap_or_else(|| unix.gid.to_string());
                let wanted_user = user.trim();
                let wanted_group = group.trim();
                let mut owner = OwnerChange {
                    user: None,
                    group: None,
                };
                if !wanted_user.is_empty() && wanted_user != initial_user {
                    owner.user = Some(wanted_user.to_owned());
                }
                if !wanted_group.is_empty() && wanted_group != initial_group {
                    owner.group = Some(wanted_group.to_owned());
                }
                if owner.user.is_some() || owner.group.is_some() {
                    changes.owner = Some(owner);
                }
            }
            PlatformProperties::Windows(windows) => {
                if read_only != windows.read_only {
                    changes.read_only = Some(read_only);
                }
                if hidden != windows.hidden {
                    changes.hidden = Some(hidden);
                }
                if archive != windows.archive {
                    changes.archive = Some(archive);
                }
                if system != windows.system {
                    changes.system = Some(system);
                }
            }
            PlatformProperties::Basic => {}
        }

        changes
    }

    #[allow(clippy::too_many_arguments)]
    fn save(
        mut self: Pin<&mut Self>,
        mode: i32,
        user: &QString,
        group: &QString,
        read_only: bool,
        hidden: bool,
        archive: bool,
        system: bool,
        recursive: bool,
    ) {
        if self.loading || self.saving {
            return;
        }
        let path = self.path.to_string();
        if path.is_empty() {
            return;
        }

        let changes = self.build_changes(
            mode,
            &user.to_string(),
            &group.to_string(),
            read_only,
            hidden,
            archive,
            system,
        );
        // A dialog that changed nothing closes quietly instead of writing the
        // same attributes back.
        let has_changes = changes.mode.is_some()
            || changes.owner.is_some()
            || changes.read_only.is_some()
            || changes.hidden.is_some()
            || changes.archive.is_some()
            || changes.system.is_some();

        if !has_changes {
            self.as_mut().saved();
            return;
        }

        self.as_mut().set_saving(true);
        self.as_mut().set_save_error(QString::default());

        let operation = self.as_mut().rust_mut().next_operation_id("props");
        let generation = self.generation;
        let thread = self.as_mut().qt_thread();
        let host = Arc::new(PropertiesHost {
            thread: thread.clone(),
        }) as Arc<dyn Host>;
        runtime::spawn(async move {
            let result = if recursive {
                service::update_file_properties_recursive(operation, path.clone(), changes, host)
                    .await
                    .map(|_| ())
            } else {
                service::update_file_properties(path.clone(), changes).await
            };
            // Re-read on success: the platform may normalize what it stored
            // (a `chmod` through a umask, an archive bit Windows adds on its
            // own), and the dialog should show what is there, not what was
            // asked for.
            let refreshed = match &result {
                Ok(()) => service::get_file_properties(path).await.ok(),
                Err(_) => None,
            };
            let _ = thread.queue(move |mut controller| {
                controller
                    .as_mut()
                    .apply_save(generation, result, refreshed)
            });
        });
    }

    fn apply_save(
        mut self: Pin<&mut Self>,
        generation: u64,
        result: Result<(), dae_core::error::FileSystemError>,
        refreshed: Option<FileProperties>,
    ) {
        if self.generation != generation {
            return;
        }
        self.as_mut().set_saving(false);
        match result {
            Ok(()) => {
                if let Some(properties) = refreshed {
                    self.as_mut().apply_refreshed(properties);
                }
                self.as_mut().saved();
            }
            Err(error) => {
                let message = friendly_error(&error.to_string());
                self.as_mut()
                    .set_save_error(QString::from(message.as_str()));
                self.as_mut().save_failed(&QString::from(message.as_str()));
            }
        }
    }

    /// Applies a post-save re-read without touching the run state: a size scan
    /// or checksum pass may be in flight and must not be reset by a save.
    fn apply_refreshed(mut self: Pin<&mut Self>, properties: FileProperties) {
        let remote = !vfs::is_local_path(&properties.path);
        self.as_mut().set_remote(remote);
        match &properties.platform {
            PlatformProperties::Unix(unix) => {
                self.as_mut().set_mode((unix.mode & 0o7777) as i32);
                self.as_mut().set_user_name(QString::from(
                    unix.user_name
                        .clone()
                        .unwrap_or_else(|| unix.uid.to_string())
                        .as_str(),
                ));
                self.as_mut().set_group_name(QString::from(
                    unix.group_name
                        .clone()
                        .unwrap_or_else(|| unix.gid.to_string())
                        .as_str(),
                ));
            }
            PlatformProperties::Windows(windows) => {
                self.as_mut().set_read_only(windows.read_only);
                self.as_mut().set_hidden(windows.hidden);
                self.as_mut().set_archive(windows.archive);
                self.as_mut().set_system(windows.system);
            }
            PlatformProperties::Basic => {}
        }
        self.as_mut().rust_mut().loaded = Some(properties);
    }

    fn reset_size_scan(mut self: Pin<&mut Self>) {
        self.as_mut().set_size_scanning(false);
        self.as_mut().set_size_scan_text(QString::default());
        self.as_mut().set_size_scan_bytes(0.0);
        self.as_mut().set_size_scan_done(false);
    }

    fn start_size_scan(mut self: Pin<&mut Self>) {
        if !self.is_directory || self.remote || self.size_scanning {
            return;
        }
        let path = self.path.to_string();
        if path.is_empty() {
            return;
        }

        self.as_mut().reset_size_scan();
        self.as_mut().set_size_scanning(true);
        let operation = self.as_mut().rust_mut().next_operation_id("size");
        self.as_mut().rust_mut().size_operation = operation.clone();

        let host = Arc::new(PropertiesHost {
            thread: self.as_mut().qt_thread(),
        }) as Arc<dyn Host>;
        match directory_size::start_directory_size_calculation(operation.clone(), vec![path], host)
        {
            Ok(()) => {}
            Err(error) => {
                self.as_mut().rust_mut().size_operation.clear();
                self.as_mut().set_size_scanning(false);
                self.as_mut()
                    .set_size_scan_text(QString::from(friendly_error(&error.to_string()).as_str()));
            }
        }
    }

    fn cancel_size_scan(mut self: Pin<&mut Self>) {
        let operation = std::mem::take(&mut self.as_mut().rust_mut().size_operation);
        if operation.is_empty() {
            return;
        }
        directory_size::cancel_directory_size_calculation(operation);
        self.as_mut().set_size_scanning(false);
    }

    fn reset_hash(mut self: Pin<&mut Self>) {
        self.as_mut().set_hashing(false);
        self.as_mut().set_hash_progress(0.0);
        self.as_mut().set_hash_md5(QString::default());
        self.as_mut().set_hash_sha1(QString::default());
        self.as_mut().set_hash_sha256(QString::default());
        self.as_mut().set_hash_error(QString::default());
        self.as_mut().set_hash_done(false);
    }

    fn start_hashing(mut self: Pin<&mut Self>) {
        if self.is_directory || self.remote || self.hashing {
            return;
        }
        let path = self.path.to_string();
        if path.is_empty() {
            return;
        }

        self.as_mut().reset_hash();
        self.as_mut().set_hashing(true);
        let operation = self.as_mut().rust_mut().next_operation_id("hash");
        self.as_mut().rust_mut().hash_operation = operation.clone();

        let host = Arc::new(PropertiesHost {
            thread: self.as_mut().qt_thread(),
        }) as Arc<dyn Host>;
        match hashing::start_file_hash_calculation(operation.clone(), path, host) {
            Ok(()) => {}
            Err(error) => {
                self.as_mut().rust_mut().hash_operation.clear();
                self.as_mut().set_hashing(false);
                let message = friendly_error(&error.to_string());
                self.as_mut()
                    .set_hash_error(QString::from(message.as_str()));
                self.as_mut().set_hash_done(true);
            }
        }
    }

    fn cancel_hashing(mut self: Pin<&mut Self>) {
        let operation = std::mem::take(&mut self.as_mut().rust_mut().hash_operation);
        if operation.is_empty() {
            return;
        }
        hashing::cancel_file_hash_calculation(operation);
        self.as_mut().set_hashing(false);
    }

    fn apply_event(mut self: Pin<&mut Self>, event: Event) {
        match event {
            Event::DirectorySizeProgress {
                operation_id,
                size,
                completed,
                ..
            } => {
                if operation_id.is_empty() || operation_id != self.size_operation {
                    return;
                }
                self.as_mut().set_size_scan_bytes(size as f64);
                self.as_mut()
                    .set_size_scan_text(QString::from(format::size(size).as_str()));
                if completed {
                    self.as_mut().rust_mut().size_operation.clear();
                    self.as_mut().set_size_scanning(false);
                    self.as_mut().set_size_scan_done(true);
                }
            }
            Event::FileHashProgress {
                operation_id,
                bytes_read,
                total_bytes,
                completed,
                digests,
                error,
                ..
            } => {
                if operation_id.is_empty() || operation_id != self.hash_operation {
                    return;
                }
                let fraction = if total_bytes == 0 {
                    0.0
                } else {
                    (bytes_read as f64 / total_bytes as f64).clamp(0.0, 1.0)
                };
                self.as_mut().set_hash_progress(fraction);
                if completed {
                    self.as_mut().rust_mut().hash_operation.clear();
                    self.as_mut().set_hashing(false);
                    self.as_mut().set_hash_done(true);
                    if let Some(digests) = digests {
                        self.as_mut()
                            .set_hash_md5(QString::from(digests.md5.as_str()));
                        self.as_mut()
                            .set_hash_sha1(QString::from(digests.sha1.as_str()));
                        self.as_mut()
                            .set_hash_sha256(QString::from(digests.sha256.as_str()));
                    }
                    if let Some(error) = error {
                        let message = friendly_error(&error);
                        self.as_mut()
                            .set_hash_error(QString::from(message.as_str()));
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parent_keeps_the_separator_style_of_the_path() {
        assert_eq!(parent_of("C:\\Users\\me\\file.txt"), "C:\\Users\\me");
        assert_eq!(parent_of("/home/me/file.txt"), "/home/me");
        assert_eq!(parent_of("file.txt"), "file.txt");
    }
}
