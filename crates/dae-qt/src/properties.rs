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

use std::sync::Arc;

use qtbridge::{QmlElement, QmlObject, QThreadHandle, qobject};
use dae_core::fs::service;
use dae_core::fs::vfs;
use dae_core::fs::{directory_size, hashing, security};
use dae_core::host::{Event, Host};
use dae_core::types::{
    EntryKind, FileProperties, OwnerChange, PlatformProperties, PropertyChanges,
};

use crate::errors::friendly_error;
use crate::format;
use crate::model::category_of;
use crate::runtime;

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

macro_rules! notifying_setters {
    ($($setter:ident($ty:ty) => $field:ident, $notify:ident;)*) => {
        $(fn $setter(&mut self, value: $ty) {
            self.$field = value;
            self.$notify();
        })*
    };
}

#[qobject(NoQmlElement, ConvertToCamelCase)]
mod properties_impl {
    use super::*;

    #[derive(Default)]
    pub struct PropertiesController {
        path: String,
        name: String,
        location: String,
        kind_text: String,
        category: String,
        target_text: String,
        size_text: String,
        created_text: String,
        modified_text: String,
        accessed_text: String,
        is_directory: bool,
        is_symlink: bool,
        remote: bool,
        editable: bool,
        platform_kind: String,
        mode: i32,
        user_name: String,
        group_name: String,
        read_only: bool,
        hidden: bool,
        archive: bool,
        system: bool,
        loading: bool,
        error: String,
        saving: bool,
        save_error: String,
        size_scanning: bool,
        size_scan_text: String,
        size_scan_bytes: f64,
        size_scan_done: bool,
        hashing: bool,
        hash_progress: f64,
        hash_md5: String,
        hash_sha1: String,
        hash_sha256: String,
        hash_error: String,
        hash_done: bool,
        signature_status: String,
        signature_signer: String,
        signature_issuer: String,
        signature_detail: String,
        signature_supported: bool,
        acl_owner: String,
        acl_group: String,
        acl_entries_json: String,
        acl_inheritance: bool,
        acl_editable: bool,
        acl_supported: bool,
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

    impl PropertiesController {
        qproperty!("path", Member = path, Notify = path_changed);
        qproperty!("name", Member = name, Notify = name_changed);
        qproperty!("location", Member = location, Notify = location_changed);
        qproperty!("kindText", Member = kind_text, Notify = kind_text_changed);
        qproperty!("category", Member = category, Notify = category_changed);
        qproperty!("targetText", Member = target_text, Notify = target_text_changed);
        qproperty!("sizeText", Member = size_text, Notify = size_text_changed);
        qproperty!("createdText", Member = created_text, Notify = created_text_changed);
        qproperty!("modifiedText", Member = modified_text, Notify = modified_text_changed);
        qproperty!("accessedText", Member = accessed_text, Notify = accessed_text_changed);
        qproperty!("isDirectory", Member = is_directory, Notify = is_directory_changed);
        qproperty!("isSymlink", Member = is_symlink, Notify = is_symlink_changed);
        qproperty!("remote", Member = remote, Notify = remote_changed);
        qproperty!("editable", Member = editable, Notify = editable_changed);
        qproperty!("platformKind", Member = platform_kind, Notify = platform_kind_changed);
        qproperty!("mode", Member = mode, Notify = mode_changed);
        qproperty!("userName", Member = user_name, Notify = user_name_changed);
        qproperty!("groupName", Member = group_name, Notify = group_name_changed);
        qproperty!("readOnly", Member = read_only, Notify = read_only_changed);
        qproperty!("hidden", Member = hidden, Notify = hidden_changed);
        qproperty!("archive", Member = archive, Notify = archive_changed);
        qproperty!("system", Member = system, Notify = system_changed);
        qproperty!("loading", Member = loading, Notify = loading_changed);
        qproperty!("error", Member = error, Notify = error_changed);
        qproperty!("saving", Member = saving, Notify = saving_changed);
        qproperty!("saveError", Member = save_error, Notify = save_error_changed);
        qproperty!("sizeScanning", Member = size_scanning, Notify = size_scanning_changed);
        qproperty!("sizeScanText", Member = size_scan_text, Notify = size_scan_text_changed);
        qproperty!("sizeScanBytes", Member = size_scan_bytes, Notify = size_scan_bytes_changed);
        qproperty!("sizeScanDone", Member = size_scan_done, Notify = size_scan_done_changed);
        qproperty!("hashing", Member = hashing, Notify = hashing_changed);
        qproperty!("hashProgress", Member = hash_progress, Notify = hash_progress_changed);
        qproperty!("hashMd5", Member = hash_md5, Notify = hash_md5_changed);
        qproperty!("hashSha1", Member = hash_sha1, Notify = hash_sha1_changed);
        qproperty!("hashSha256", Member = hash_sha256, Notify = hash_sha256_changed);
        qproperty!("hashError", Member = hash_error, Notify = hash_error_changed);
        qproperty!("hashDone", Member = hash_done, Notify = hash_done_changed);
        qproperty!("signatureStatus", Member = signature_status, Notify = signature_status_changed);
        qproperty!("signatureSigner", Member = signature_signer, Notify = signature_signer_changed);
        qproperty!("signatureIssuer", Member = signature_issuer, Notify = signature_issuer_changed);
        qproperty!("signatureDetail", Member = signature_detail, Notify = signature_detail_changed);
        qproperty!("signatureSupported", Member = signature_supported, Notify = signature_supported_changed);
        qproperty!("aclOwner", Member = acl_owner, Notify = acl_owner_changed);
        qproperty!("aclGroup", Member = acl_group, Notify = acl_group_changed);
        qproperty!("aclEntriesJson", Member = acl_entries_json, Notify = acl_entries_json_changed);
        qproperty!("aclInheritance", Member = acl_inheritance, Notify = acl_inheritance_changed);
        qproperty!("aclEditable", Member = acl_editable, Notify = acl_editable_changed);
        qproperty!("aclSupported", Member = acl_supported, Notify = acl_supported_changed);

        #[qsignal]
        fn path_changed(&mut self);
        #[qsignal]
        fn name_changed(&mut self);
        #[qsignal]
        fn location_changed(&mut self);
        #[qsignal]
        fn kind_text_changed(&mut self);
        #[qsignal]
        fn category_changed(&mut self);
        #[qsignal]
        fn target_text_changed(&mut self);
        #[qsignal]
        fn size_text_changed(&mut self);
        #[qsignal]
        fn created_text_changed(&mut self);
        #[qsignal]
        fn modified_text_changed(&mut self);
        #[qsignal]
        fn accessed_text_changed(&mut self);
        #[qsignal]
        fn is_directory_changed(&mut self);
        #[qsignal]
        fn is_symlink_changed(&mut self);
        #[qsignal]
        fn remote_changed(&mut self);
        #[qsignal]
        fn editable_changed(&mut self);
        #[qsignal]
        fn platform_kind_changed(&mut self);
        #[qsignal]
        fn mode_changed(&mut self);
        #[qsignal]
        fn user_name_changed(&mut self);
        #[qsignal]
        fn group_name_changed(&mut self);
        #[qsignal]
        fn read_only_changed(&mut self);
        #[qsignal]
        fn hidden_changed(&mut self);
        #[qsignal]
        fn archive_changed(&mut self);
        #[qsignal]
        fn system_changed(&mut self);
        #[qsignal]
        fn loading_changed(&mut self);
        #[qsignal]
        fn error_changed(&mut self);
        #[qsignal]
        fn saving_changed(&mut self);
        #[qsignal]
        fn save_error_changed(&mut self);
        #[qsignal]
        fn size_scanning_changed(&mut self);
        #[qsignal]
        fn size_scan_text_changed(&mut self);
        #[qsignal]
        fn size_scan_bytes_changed(&mut self);
        #[qsignal]
        fn size_scan_done_changed(&mut self);
        #[qsignal]
        fn hashing_changed(&mut self);
        #[qsignal]
        fn hash_progress_changed(&mut self);
        #[qsignal]
        fn hash_md5_changed(&mut self);
        #[qsignal]
        fn hash_sha1_changed(&mut self);
        #[qsignal]
        fn hash_sha256_changed(&mut self);
        #[qsignal]
        fn hash_error_changed(&mut self);
        #[qsignal]
        fn hash_done_changed(&mut self);
        #[qsignal]
        fn signature_status_changed(&mut self);
        #[qsignal]
        fn signature_signer_changed(&mut self);
        #[qsignal]
        fn signature_issuer_changed(&mut self);
        #[qsignal]
        fn signature_detail_changed(&mut self);
        #[qsignal]
        fn signature_supported_changed(&mut self);
        #[qsignal]
        fn acl_owner_changed(&mut self);
        #[qsignal]
        fn acl_group_changed(&mut self);
        #[qsignal]
        fn acl_entries_json_changed(&mut self);
        #[qsignal]
        fn acl_inheritance_changed(&mut self);
        #[qsignal]
        fn acl_editable_changed(&mut self);
        #[qsignal]
        fn acl_supported_changed(&mut self);

        notifying_setters! {
            set_path(String) => path, path_changed;
            set_name(String) => name, name_changed;
            set_location(String) => location, location_changed;
            set_kind_text(String) => kind_text, kind_text_changed;
            set_category(String) => category, category_changed;
            set_target_text(String) => target_text, target_text_changed;
            set_size_text(String) => size_text, size_text_changed;
            set_created_text(String) => created_text, created_text_changed;
            set_modified_text(String) => modified_text, modified_text_changed;
            set_accessed_text(String) => accessed_text, accessed_text_changed;
            set_is_directory(bool) => is_directory, is_directory_changed;
            set_is_symlink(bool) => is_symlink, is_symlink_changed;
            set_remote(bool) => remote, remote_changed;
            set_editable(bool) => editable, editable_changed;
            set_platform_kind(String) => platform_kind, platform_kind_changed;
            set_mode(i32) => mode, mode_changed;
            set_user_name(String) => user_name, user_name_changed;
            set_group_name(String) => group_name, group_name_changed;
            set_read_only(bool) => read_only, read_only_changed;
            set_hidden(bool) => hidden, hidden_changed;
            set_archive(bool) => archive, archive_changed;
            set_system(bool) => system, system_changed;
            set_loading(bool) => loading, loading_changed;
            set_error(String) => error, error_changed;
            set_saving(bool) => saving, saving_changed;
            set_save_error(String) => save_error, save_error_changed;
            set_size_scanning(bool) => size_scanning, size_scanning_changed;
            set_size_scan_text(String) => size_scan_text, size_scan_text_changed;
            set_size_scan_bytes(f64) => size_scan_bytes, size_scan_bytes_changed;
            set_size_scan_done(bool) => size_scan_done, size_scan_done_changed;
            set_hashing(bool) => hashing, hashing_changed;
            set_hash_progress(f64) => hash_progress, hash_progress_changed;
            set_hash_md5(String) => hash_md5, hash_md5_changed;
            set_hash_sha1(String) => hash_sha1, hash_sha1_changed;
            set_hash_sha256(String) => hash_sha256, hash_sha256_changed;
            set_hash_error(String) => hash_error, hash_error_changed;
            set_hash_done(bool) => hash_done, hash_done_changed;
            set_signature_status(String) => signature_status, signature_status_changed;
            set_signature_signer(String) => signature_signer, signature_signer_changed;
            set_signature_issuer(String) => signature_issuer, signature_issuer_changed;
            set_signature_detail(String) => signature_detail, signature_detail_changed;
            set_signature_supported(bool) => signature_supported, signature_supported_changed;
            set_acl_owner(String) => acl_owner, acl_owner_changed;
            set_acl_group(String) => acl_group, acl_group_changed;
            set_acl_entries_json(String) => acl_entries_json, acl_entries_json_changed;
            set_acl_inheritance(bool) => acl_inheritance, acl_inheritance_changed;
            set_acl_editable(bool) => acl_editable, acl_editable_changed;
            set_acl_supported(bool) => acl_supported, acl_supported_changed;
        }

        #[qsignal]
        fn saved(&mut self);
        #[qsignal]
        fn save_failed(&mut self, message: String);


        /// A fresh id per run: the dialog may start a scan, close it and start
        /// another before the first one's final event arrives.
        fn next_operation_id(&mut self, prefix: &str) -> String {
            self.next_operation = self.next_operation.wrapping_add(1);
            format!("qt-{prefix}-{}", self.next_operation)
        }

        #[qslot]

        fn open(&mut self, path: String) {
                    let generation = self.generation.wrapping_add(1);
            self.generation = generation;
            self.loaded = None;

            self.stop_runs();
            self.set_path(path.to_owned());
            self.set_error(String::new());
            self.set_save_error(String::new());
            self.set_saving(false);
            self.set_loading(true);
            self.set_remote(!vfs::is_local_path(&path));
            self.reset_size_scan();
            self.reset_hash();
            self.reset_security();

            let thread = self.get_thread_handle();
            let security_path = path.clone();
            runtime::spawn(async move {
                let result = service::get_file_properties(path).await;
                // Signature and ACL are blocking platform reads; they run beside
                // the metadata read rather than after it, so a slow WinVerifyTrust
                // cannot hold the whole dialog.
                let security = if vfs::is_local_path(&security_path) {
                    tokio::task::spawn_blocking(move || {
                        (
                            security::signature(&security_path),
                            security::acl(&security_path),
                        )
                    })
                    .await
                    .ok()
                } else {
                    None
                };
                thread.invoke_mut(move |controller| {
                    controller.apply_properties(generation, result);
                    controller.apply_security(generation, security);
                });
            });
        }

        #[qslot]

        fn close(&mut self) {
            let generation = self.generation.wrapping_add(1);
            self.generation = generation;
            self.stop_runs();
            self.loaded = None;
        }

        fn apply_properties(
            &mut self,
            generation: u64,
            result: Result<FileProperties, dae_core::error::FileSystemError>,
        ) {
            if self.generation != generation {
                return;
            }

            match result {
                Ok(properties) => {
                    let remote = !vfs::is_local_path(&properties.path);
                    self.set_path(properties.path.to_owned());
                    self.set_name(properties.name.to_owned());
                    self.set_location(parent_of(&properties.path).to_owned());
                    self.set_kind_text(kind_label(properties.kind).to_owned());
                    self.set_category(
                        category_of(properties.kind, &properties.name).to_owned(),
                    );
                    self.set_target_text(properties.target.as_deref().unwrap_or("").to_owned());
                    self.set_size_text(
                        properties
                            .size
                            .map(format::size)
                            .unwrap_or_else(|| "—".to_owned()),
                    );
                    self.set_created_text(
                        properties
                            .created_at
                            .map(format::timestamp)
                            .unwrap_or_else(|| "—".to_owned()),
                    );
                    self.set_modified_text(
                        properties
                            .modified_at
                            .map(format::timestamp)
                            .unwrap_or_else(|| "—".to_owned()),
                    );
                    self.set_accessed_text(
                        properties
                            .accessed_at
                            .map(format::timestamp)
                            .unwrap_or_else(|| "—".to_owned()),
                    );
                    self.set_is_directory(properties.kind == EntryKind::Directory);
                    self.set_is_symlink(properties.kind == EntryKind::Symlink);
                    self.set_remote(remote);

                    let kind = platform_kind(&properties, remote);
                    self.set_platform_kind(kind.to_owned());
                    self.set_editable(!remote && kind != "basic");

                    match &properties.platform {
                        PlatformProperties::Unix(unix) => {
                            self.set_mode((unix.mode & 0o7777) as i32);
                            self.set_user_name(
                                unix.user_name
                                    .clone()
                                    .unwrap_or_else(|| unix.uid.to_string()),
                            );
                            self.set_group_name(
                                unix.group_name
                                    .clone()
                                    .unwrap_or_else(|| unix.gid.to_string()),
                            );
                        }
                        PlatformProperties::Windows(windows) => {
                            self.set_read_only(windows.read_only);
                            self.set_hidden(windows.hidden);
                            self.set_archive(windows.archive);
                            self.set_system(windows.system);
                        }
                        PlatformProperties::Basic => {}
                    }

                    self.loaded = Some(properties);
                    self.set_error(String::new());
                }
                Err(error) => {
                    self.set_error(friendly_error(&error.to_string()).to_owned());
                }
            }
            self.set_loading(false);
        }

        /// `open` on a new entry or `close` on the old one: both runs are about to
        /// become meaningless, and their ids are what cancels them.
        fn stop_runs(&mut self) {
            self.cancel_size_scan();
            self.cancel_hashing();
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
        #[qslot]
        fn save(
            &mut self,
            mode: i32,
            user: String,
            group: String,
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
                &user,
                &group,
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
                self.saved();
                return;
            }

            self.set_saving(true);
            self.set_save_error(String::new());

            let operation = self.next_operation_id("props");
            let generation = self.generation;
            let thread = self.get_thread_handle();
            let host = Arc::new(PropertiesHost {
                thread: self.get_thread_handle(),
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
                thread.invoke_mut(move |controller| {
                    controller
                        .apply_save(generation, result, refreshed)
                });
            });
        }

        fn apply_save(
            &mut self,
            generation: u64,
            result: Result<(), dae_core::error::FileSystemError>,
            refreshed: Option<FileProperties>,
        ) {
            if self.generation != generation {
                return;
            }
            self.set_saving(false);
            match result {
                Ok(()) => {
                    if let Some(properties) = refreshed {
                        self.apply_refreshed(properties);
                    }
                    self.saved();
                }
                Err(error) => {
                    let message = friendly_error(&error.to_string());
                    self.set_save_error(message.clone());
                    self.save_failed(String::from(message.as_str()));
                }
            }
        }

        /// Applies a post-save re-read without touching the run state: a size scan
        /// or checksum pass may be in flight and must not be reset by a save.
        fn apply_refreshed(&mut self, properties: FileProperties) {
            let remote = !vfs::is_local_path(&properties.path);
            self.set_remote(remote);
            match &properties.platform {
                PlatformProperties::Unix(unix) => {
                    self.set_mode((unix.mode & 0o7777) as i32);
                    self.set_user_name(
                        unix.user_name
                            .clone()
                            .unwrap_or_else(|| unix.uid.to_string()),
                    );
                    self.set_group_name(
                        unix.group_name
                            .clone()
                            .unwrap_or_else(|| unix.gid.to_string()),
                    );
                }
                PlatformProperties::Windows(windows) => {
                    self.set_read_only(windows.read_only);
                    self.set_hidden(windows.hidden);
                    self.set_archive(windows.archive);
                    self.set_system(windows.system);
                }
                PlatformProperties::Basic => {}
            }
            self.loaded = Some(properties);
        }

        fn reset_size_scan(&mut self) {
            self.set_size_scanning(false);
            self.set_size_scan_text(String::new());
            self.set_size_scan_bytes(0.0);
            self.set_size_scan_done(false);
        }

        #[qslot]

        fn start_size_scan(&mut self) {
            if !self.is_directory || self.remote || self.size_scanning {
                return;
            }
            let path = self.path.to_string();
            if path.is_empty() {
                return;
            }

            self.reset_size_scan();
            self.set_size_scanning(true);
            let operation = self.next_operation_id("size");
            self.size_operation = operation.clone();

            let host = Arc::new(PropertiesHost {
                thread: self.get_thread_handle(),
            }) as Arc<dyn Host>;
            match directory_size::start_directory_size_calculation(operation.clone(), vec![path], host)
            {
                Ok(()) => {}
                Err(error) => {
                    self.size_operation.clear();
                    self.set_size_scanning(false);
                    self.set_size_scan_text(friendly_error(&error.to_string()).to_owned());
                }
            }
        }

        #[qslot]

        fn cancel_size_scan(&mut self) {
            let operation = std::mem::take(&mut self.size_operation);
            if operation.is_empty() {
                return;
            }
            directory_size::cancel_directory_size_calculation(operation);
            self.set_size_scanning(false);
        }

        fn reset_hash(&mut self) {
            self.set_hashing(false);
            self.set_hash_progress(0.0);
            self.set_hash_md5(String::new());
            self.set_hash_sha1(String::new());
            self.set_hash_sha256(String::new());
            self.set_hash_error(String::new());
            self.set_hash_done(false);
        }

        fn reset_security(&mut self) {
            self.set_signature_supported(security::signature_supported());
            self.set_acl_supported(security::acl_supported());
            self.set_signature_status(String::new());
            self.set_signature_signer(String::new());
            self.set_signature_issuer(String::new());
            self.set_signature_detail(String::new());
            self.set_acl_owner(String::new());
            self.set_acl_group(String::new());
            self.set_acl_entries_json(String::new());
            self.set_acl_inheritance(false);
            self.set_acl_editable(false);
        }

        fn apply_security(
            &mut self,
            generation: u64,
            result: Option<(
                Result<security::FileSignature, dae_core::error::FileSystemError>,
                Result<security::FileAcl, dae_core::error::FileSystemError>,
            )>,
        ) {
            if self.generation != generation {
                return;
            }
            let Some((signature, acl)) = result else {
                return;
            };

            if let Ok(signature) = signature {
                self.set_signature_status(signature.status.to_owned());
                self.set_signature_signer(signature.signer.as_deref().unwrap_or("").to_owned());
                self.set_signature_issuer(signature.issuer.as_deref().unwrap_or("").to_owned());
                self.set_signature_detail(signature.detail.as_deref().unwrap_or("").to_owned());
            }

            if let Ok(acl) = acl {
                self.apply_acl(acl);
            }
        }

        /// Publishes one ACL snapshot. The JSON goes last so the dialog's
        /// `onAclEntriesJsonChanged` handler sees the final inheritance and
        /// editable flags when it re-syncs its draft.
        fn apply_acl(&mut self, acl: security::FileAcl) {
            self.set_acl_owner(acl.owner.as_deref().unwrap_or("").to_owned());
            self.set_acl_group(acl.group.as_deref().unwrap_or("").to_owned());
            self.set_acl_inheritance(acl.inheritance);
            self.set_acl_editable(acl.editable);
            let json = serde_json::to_string(&acl.entries).unwrap_or_else(|_| "[]".to_owned());
            self.set_acl_entries_json(json.to_owned());
        }

        #[qslot]

        fn save_acl(&mut self, entries_json: String, inheritance: bool) {
            if self.loading || self.saving {
                return;
            }
            let path = self.path.to_string();
            if path.is_empty() {
                return;
            }

            let entries: Vec<security::AclEntrySpec> =
                match serde_json::from_str(&entries_json) {
                    Ok(entries) => entries,
                    Err(error) => {
                        let message = friendly_error(&format!("Invalid ACL entries: {error}"));
                        self.set_save_error(message.clone());
                        self.save_failed(String::from(message.as_str()));
                        return;
                    }
                };
            let changes = security::AclChanges { entries, inheritance };

            self.set_saving(true);
            self.set_save_error(String::new());

            let generation = self.generation;
            let thread = self.get_thread_handle();
            runtime::spawn(async move {
                let write_path = path.clone();
                let result = tokio::task::spawn_blocking(move || security::set_acl(&write_path, &changes))
                    .await
                    .unwrap_or_else(|_| {
                        Err(dae_core::error::FileSystemError::Internal(
                            "The ACL write was interrupted".into(),
                        ))
                    });
                // Re-read on success: the platform normalizes what it stored (the
                // propagated inherited ACEs, the recomputed POSIX mask), and the
                // dialog should show what is there, not what was asked for.
                let refreshed = if result.is_ok() {
                    tokio::task::spawn_blocking(move || security::acl(&path))
                        .await
                        .ok()
                        .and_then(|result| result.ok())
                } else {
                    None
                };
                thread.invoke_mut(move |controller| {
                    controller
                        .apply_acl_save(generation, result, refreshed)
                });
            });
        }

        fn apply_acl_save(
            &mut self,
            generation: u64,
            result: Result<(), dae_core::error::FileSystemError>,
            refreshed: Option<security::FileAcl>,
        ) {
            if self.generation != generation {
                return;
            }
            self.set_saving(false);
            match result {
                Ok(()) => {
                    if let Some(acl) = refreshed {
                        self.apply_acl(acl);
                    }
                    self.saved();
                }
                Err(error) => {
                    let message = friendly_error(&error.to_string());
                    self.set_save_error(message.clone());
                    self.save_failed(String::from(message.as_str()));
                }
            }
        }

        #[qslot]

        fn start_hashing(&mut self) {
            if self.is_directory || self.remote || self.hashing {
                return;
            }
            let path = self.path.to_string();
            if path.is_empty() {
                return;
            }

            self.reset_hash();
            self.set_hashing(true);
            let operation = self.next_operation_id("hash");
            self.hash_operation = operation.clone();

            let host = Arc::new(PropertiesHost {
                thread: self.get_thread_handle(),
            }) as Arc<dyn Host>;
            match hashing::start_file_hash_calculation(operation.clone(), path, host) {
                Ok(()) => {}
                Err(error) => {
                    self.hash_operation.clear();
                    self.set_hashing(false);
                    let message = friendly_error(&error.to_string());
                    self.set_hash_error(message.to_owned());
                    self.set_hash_done(true);
                }
            }
        }

        #[qslot]

        fn cancel_hashing(&mut self) {
            let operation = std::mem::take(&mut self.hash_operation);
            if operation.is_empty() {
                return;
            }
            hashing::cancel_file_hash_calculation(operation);
            self.set_hashing(false);
        }

        pub(super) fn apply_event(&mut self, event: Event) {
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
                    self.set_size_scan_bytes(size as f64);
                    self.set_size_scan_text(format::size(size).to_owned());
                    if completed {
                        self.size_operation.clear();
                        self.set_size_scanning(false);
                        self.set_size_scan_done(true);
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
                    self.set_hash_progress(fraction);
                    if completed {
                        self.hash_operation.clear();
                        self.set_hashing(false);
                        self.set_hash_done(true);
                        if let Some(digests) = digests {
                            self.set_hash_md5(digests.md5.to_owned());
                            self.set_hash_sha1(digests.sha1.to_owned());
                            self.set_hash_sha256(digests.sha256.to_owned());
                        }
                        if let Some(error) = error {
                            let message = friendly_error(&error);
                            self.set_hash_error(message.to_owned());
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

pub use properties_impl::PropertiesController;

impl QmlElement for PropertiesController {
    const URI: &str = "dae";
    const ELEMENT_NAME: &str = "PropertiesController";
    const MINOR_VERSION: u8 = 0;
    const MAJOR_VERSION: u8 = 1;
    const IS_SINGLETON: bool = false;
}

struct PropertiesHost {
    thread: QThreadHandle<PropertiesController>,
}

impl Host for PropertiesHost {
    fn emit(&self, event: Event) {
        let _ = self
            .thread
            .invoke_mut(move |controller| controller.apply_event(event));
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
