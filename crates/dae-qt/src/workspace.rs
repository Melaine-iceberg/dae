//! The command surface QML calls.
//!
//! Every file operation is a `dae_core::fs::service` call. This module does
//! three shell-only things around them: it owns the Tokio runtime, it turns
//! the core's `Host` events into queued Qt property updates, and it holds the
//! bits of state that belong to a *shell* rather than to the file system —
//! the in-app clipboard and its system mirror, the last accent reading, and
//! the cached terminal typography.
//!
//! Calls return immediately. The outcome arrives as `operationFinished` or
//! `operationFailed`, which is what lets the QML side keep painting during a
//! large copy instead of blocking on an `invoke`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use qtbridge::{QmlElement, QmlObject, QThreadHandle, qobject};
use dae_core::error::FileSystemError;
use dae_core::fs::actions;
use dae_core::fs::archive::{self, ArchiveFormat};
use dae_core::fs::clipboard::{
    ClipboardEffect, SystemClipboardFiles, clipboard_sequence as system_clipboard_sequence,
};
use dae_core::fs::cloud::accounts as cloud_accounts;
use dae_core::fs::cloud::oauth::{self as cloud_oauth, AuthorizeCloudAccountInput};
use dae_core::fs::cloud::provider::CloudProviderKind;
use dae_core::fs::connections::{self, Protocol, SaveConnectionInput};
use dae_core::fs::directory_size;
use dae_core::fs::drives;
use dae_core::fs::hashing;
use dae_core::fs::jobs::{JobEvent, JobScheduler, TransferRequest};
use dae_core::fs::open_with;
use dae_core::fs::occupancy;
use dae_core::fs::recents::RecentSource;
use dae_core::fs::service;
use dae_core::fs::sidebar::{self, Favorite, PlaceKind};
use dae_core::fs::smb;
use dae_core::fs::templates;
use dae_core::fs::trash_view;
use dae_core::fs::types::{
    ConflictAction, EntryKind, NewEntryKind, TransferItem, display_name_from_path,
};
use dae_core::fs::vfs;
use dae_core::host::{Event, Host};
use serde::{Deserialize, Serialize};

use crate::default_manager::{self, DefaultManagerStatus};
use crate::errors::{default_manager_detail, friendly_error};
use crate::platform;
use crate::runtime;

#[cxx::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("drag.h");
        #[namespace = "dae_qt"]
        #[rust_name = "start_file_drag"]
        fn dae_qt_start_file_drag_rs(
            paths: &[String],
            actions: i32,
            signal: &str,
            count: i32,
            verdict: &str,
            ink: &str,
            canvas: &str,
        ) -> bool;

        #[namespace = "dae_qt"]
        #[rust_name = "local_drop_paths"]
        fn dae_qt_local_drop_paths(urls: &[String]) -> Vec<String>;

        include!("clipboard.h");
        #[namespace = "dae_qt"]
        #[rust_name = "set_clipboard_text"]
        fn dae_qt_set_clipboard_text_rs(text: &str);

        include!("platform_watch.h");
        #[namespace = "dae_qt"]
        #[rust_name = "install_theme_watcher"]
        fn dae_qt_install_theme_watcher();
    }

    extern "Rust" {
        /// Called by the Qt style-hint watcher on the GUI thread.
        #[namespace = "dae_qt"]
        fn dae_qt_theme_changed(dark: bool);
    }
}

/// The C++ side of `QStyleHints::colorSchemeChanged`.
fn dae_qt_theme_changed(dark: bool) {
    crate::system_watch::theme_changed(dark);
}

macro_rules! notifying_setters {
    ($($setter:ident($ty:ty) => $field:ident, $notify:ident;)*) => {
        $(#[allow(dead_code)]
        fn $setter(&mut self, value: $ty) {
            self.$field = value;
            self.$notify();
        })*
    };
}

#[qobject(NoQmlElement, ConvertToCamelCase)]
mod workspace_impl {
    use super::*;

    pub struct Workspace {
        status: String,
        last_error: String,
        busy: bool,
        can_undo: bool,
        can_redo: bool,
        progress_label: String,
        progress_done: i32,
        progress_total: i32,
        progress_kind: String,
        conflict_index: i32,
        conflict_total: i32,
        conflict_name: String,
        conflict_cut: bool,
        conflict_source_category: String,
        conflict_source_detail: String,
        conflict_target_category: String,
        conflict_target_detail: String,
        conflict_job_id: i64,
        conflict_waiting: i32,
        /// Each asking job's frozen conflict prompt, in the order their prompts
        /// arrived. The head is the question on screen; the rest wait untouched
        /// until their turn, so several jobs can ask at once without their
        /// collisions interleaving on one dialog.
        pending_conflicts: Vec<PendingPrompt>,
        queued_transfers: Vec<String>,
        queued_transfer_paths: Vec<String>,
        transfers_json: String,
        transfer_speed_limit: i32,
        transfer_speed: String,
        /// One row per accepted transfer — checking, answering conflicts or
        /// running. The scheduler addresses them by job id; an elevated retry
        /// owns a row too, under a local id the scheduler never mints.
        transfer_rows: Vec<TransferRow>,
        /// Ids for rows the shell creates itself (elevated retries). Starts far
        /// above the scheduler's counter so the two never collide.
        next_local_job: u64,
        /// Copies and moves are queued and answered by the core's job scheduler;
        /// this shell only submits requests and renders its events. Created on
        /// first use, when a Qt thread exists to route the events back through.
        transfer_scheduler: Option<Arc<JobScheduler>>,
        /// Cancellation flag of the running "calculate size" pass, if any.
        size_scan: Option<Arc<AtomicBool>>,
        /// Status-center history, oldest first.
        operations: Vec<OperationRecord>,
        drive_capabilities: String,
        external_archive_formats: String,
        custom_actions_json: String,
        operations_json: String,
        failure_count: i32,
        /// When each running operation began, with the label it showed. A stack
        /// rather than a map: operations legitimately overlap (a background size
        /// scan during a copy), and the newest start is the one finishing next.
        operation_starts: Vec<(u64, String)>,
        /// The retry payload for the operation about to start. `extractArchive`
        /// and a transfer set it immediately after `beginOperation`; finishing the
        /// operation consumes it.
        retry_hint: Option<String>,
        clipboard_mode: String,
        clipboard_paths: Vec<String>,
        /// The exact paths this shell last put on the system clipboard. A paste
        /// compares against it to tell its own list from files another app copied.
        system_clipboard_paths: Vec<String>,
        /// The system's clipboard counter when the mirror was last written or
        /// refreshed; a different counter means another app changed it since.
        system_clipboard_sequence: u32,
        /// Bumped whenever the clipboard changes, so a refresh that was already
        /// in flight cannot overwrite a newer copy.
        clipboard_generation: u64,
        accent: String,
        system_dark: bool,
        home_directory: String,
        terminal_font_size: i32,
        terminal_line_height: f64,
        terminal_font_family: String,
        shortcuts_json: String,
        log_directory: String,
        search_index_available: bool,
        open_with_picker_supported: bool,
        default_manager_supported: bool,
        default_manager_is_default: bool,
        default_manager_is_registered: bool,
        default_manager_detail: String,
        default_manager_error: String,
        default_manager_busy: bool,
        active_operations: i32,
    }

    impl Default for Workspace {
        fn default() -> Self {
            // The connection and cloud registries are file/keychain backed; the
            // sidebar and the connect dialogs read them through this same store.
            crate::init_stores();
            let settings = dae_core::settings::load_settings().unwrap_or_default();
            let home = service::get_home_directory().unwrap_or_default();
            let operations = load_operations();
            let operations_text = operations_json(&operations);

            // The limit is process-wide and lives in the core, so a transfer that
            // starts before the settings panel is ever opened still obeys it.
            let transfer_speed_limit = settings.transfer.speed_limit_kib.min(MAX_SPEED_LIMIT_KIB) as i32;
            service::set_transfer_speed_limit(
                (transfer_speed_limit > 0).then_some(transfer_speed_limit as u64 * 1024),
            );

            Self {
                status: String::new(),
                last_error: String::new(),
                busy: false,
                can_undo: false,
                can_redo: false,
                progress_label: String::new(),
                progress_done: 0,
                progress_total: 0,
                progress_kind: String::new(),
                conflict_index: 0,
                conflict_total: 0,
                conflict_name: String::new(),
                conflict_cut: false,
                conflict_source_category: String::new(),
                conflict_source_detail: String::new(),
                conflict_target_category: String::new(),
                conflict_target_detail: String::new(),
                conflict_job_id: 0,
                conflict_waiting: 0,
                pending_conflicts: Vec::new(),
                queued_transfers: Vec::new(),
                queued_transfer_paths: Vec::new(),
                transfers_json: "[]".to_owned(),
                transfer_speed_limit,
                transfer_speed: String::new(),
                transfer_rows: Vec::new(),
                next_local_job: 1_000_000_000,
                transfer_scheduler: None,
                custom_actions_json: custom_actions_json(),
                drive_capabilities: drive_capabilities_json(),
                external_archive_formats: external_archive_formats_json(),
                operations_json: operations_text,
                failure_count: 0,
                size_scan: None,
                operations,
                operation_starts: Vec::new(),
                retry_hint: None,
                clipboard_mode: String::new(),
                clipboard_paths: Vec::new(),
                system_clipboard_paths: Vec::new(),
                system_clipboard_sequence: 0,
                clipboard_generation: 0,
                accent: platform::accent_hex(),
                system_dark: crate::config::system_dark(),
                home_directory: home,
                terminal_font_size: settings.terminal.font_size as i32,
                terminal_line_height: settings.terminal.line_height as f64,
                terminal_font_family: settings.terminal.font_family.clone().unwrap_or_default(),
                shortcuts_json: shortcuts_json(&settings.shortcuts),
                log_directory: crate::logging::app_log_directory()
                    .to_string_lossy()
                    .into_owned(),
                search_index_available: dae_core::fs::search_index::backend().is_some(),
                open_with_picker_supported: cfg!(any(target_os = "macos", target_os = "linux")),
                default_manager_supported: cfg!(any(target_os = "macos", target_os = "linux", windows)),
                default_manager_is_default: false,
                default_manager_is_registered: false,
                default_manager_detail: String::new(),
                default_manager_error: String::new(),
                default_manager_busy: false,
                active_operations: 0,
            }
        }
    }


    impl Workspace {
        qproperty!("externalArchiveFormats", Member = external_archive_formats, Notify = external_archive_formats_changed);
        qproperty!("driveCapabilities", Member = drive_capabilities, Notify = drive_capabilities_changed);
        qproperty!("customActionsJson", Member = custom_actions_json, Notify = custom_actions_json_changed);
        qproperty!("operationsJson", Member = operations_json, Notify = operations_json_changed);
        qproperty!("failureCount", Member = failure_count, Notify = failure_count_changed);
        qproperty!("status", Member = status, Notify = status_changed);
        qproperty!("lastError", Member = last_error, Notify = last_error_changed);
        qproperty!("busy", Member = busy, Notify = busy_changed);
        qproperty!("canUndo", Member = can_undo, Notify = can_undo_changed);
        qproperty!("canRedo", Member = can_redo, Notify = can_redo_changed);
        qproperty!("progressLabel", Member = progress_label, Notify = progress_label_changed);
        qproperty!("progressDone", Member = progress_done, Notify = progress_done_changed);
        qproperty!("progressTotal", Member = progress_total, Notify = progress_total_changed);
        qproperty!("progressKind", Member = progress_kind, Notify = progress_kind_changed);
        qproperty!("conflictIndex", Member = conflict_index, Notify = conflict_index_changed);
        qproperty!("conflictTotal", Member = conflict_total, Notify = conflict_total_changed);
        qproperty!("conflictName", Member = conflict_name, Notify = conflict_name_changed);
        qproperty!("conflictCut", Member = conflict_cut, Notify = conflict_cut_changed);
        qproperty!("conflictSourceCategory", Member = conflict_source_category, Notify = conflict_source_category_changed);
        qproperty!("conflictSourceDetail", Member = conflict_source_detail, Notify = conflict_source_detail_changed);
        qproperty!("conflictTargetCategory", Member = conflict_target_category, Notify = conflict_target_category_changed);
        qproperty!("conflictTargetDetail", Member = conflict_target_detail, Notify = conflict_target_detail_changed);
        qproperty!("conflictJobId", Member = conflict_job_id, Notify = conflict_job_id_changed);
        qproperty!("conflictWaiting", Member = conflict_waiting, Notify = conflict_waiting_changed);
        qproperty!("queuedTransfers", Member = queued_transfers, Notify = queued_transfers_changed);
        qproperty!("queuedTransferPaths", Member = queued_transfer_paths, Notify = queued_transfer_paths_changed);
        qproperty!("transfersJson", Member = transfers_json, Notify = transfers_json_changed);
        qproperty!("transferSpeedLimit", Member = transfer_speed_limit, Notify = transfer_speed_limit_changed);
        qproperty!("transferSpeed", Member = transfer_speed, Notify = transfer_speed_changed);
        qproperty!("clipboardMode", Member = clipboard_mode, Notify = clipboard_mode_changed);
        qproperty!("clipboardPaths", Member = clipboard_paths, Notify = clipboard_paths_changed);
        qproperty!("accent", Member = accent, Notify = accent_changed);
        qproperty!("systemDark", Member = system_dark, Notify = system_dark_changed);
        qproperty!("homeDirectory", Member = home_directory, Notify = home_directory_changed);
        qproperty!("terminalFontSize", Member = terminal_font_size, Notify = terminal_font_size_changed);
        qproperty!("terminalLineHeight", Member = terminal_line_height, Notify = terminal_line_height_changed);
        qproperty!("terminalFontFamily", Member = terminal_font_family, Notify = terminal_font_family_changed);
        qproperty!("shortcutsJson", Member = shortcuts_json, Notify = shortcuts_json_changed);
        qproperty!("logDirectory", Member = log_directory, Notify = log_directory_changed);
        qproperty!("searchIndexAvailable", Member = search_index_available, Notify = search_index_available_changed);
        qproperty!("openWithPickerSupported", Member = open_with_picker_supported, Notify = open_with_picker_supported_changed);
        qproperty!("defaultManagerSupported", Member = default_manager_supported, Notify = default_manager_supported_changed);
        qproperty!("defaultManagerIsDefault", Member = default_manager_is_default, Notify = default_manager_is_default_changed);
        qproperty!("defaultManagerIsRegistered", Member = default_manager_is_registered, Notify = default_manager_is_registered_changed);
        qproperty!("defaultManagerDetail", Member = default_manager_detail, Notify = default_manager_detail_changed);
        qproperty!("defaultManagerError", Member = default_manager_error, Notify = default_manager_error_changed);
        qproperty!("defaultManagerBusy", Member = default_manager_busy, Notify = default_manager_busy_changed);

        #[qsignal]
        fn external_archive_formats_changed(&mut self);
        #[qsignal]
        fn drive_capabilities_changed(&mut self);
        #[qsignal]
        fn custom_actions_json_changed(&mut self);
        #[qsignal]
        fn operations_json_changed(&mut self);
        #[qsignal]
        fn failure_count_changed(&mut self);
        #[qsignal]
        fn status_changed(&mut self);
        #[qsignal]
        fn last_error_changed(&mut self);
        #[qsignal]
        fn busy_changed(&mut self);
        #[qsignal]
        fn can_undo_changed(&mut self);
        #[qsignal]
        fn can_redo_changed(&mut self);
        #[qsignal]
        fn progress_label_changed(&mut self);
        #[qsignal]
        fn progress_done_changed(&mut self);
        #[qsignal]
        fn progress_total_changed(&mut self);
        #[qsignal]
        fn progress_kind_changed(&mut self);
        #[qsignal]
        fn conflict_index_changed(&mut self);
        #[qsignal]
        fn conflict_total_changed(&mut self);
        #[qsignal]
        fn conflict_name_changed(&mut self);
        #[qsignal]
        fn conflict_cut_changed(&mut self);
        #[qsignal]
        fn conflict_source_category_changed(&mut self);
        #[qsignal]
        fn conflict_source_detail_changed(&mut self);
        #[qsignal]
        fn conflict_target_category_changed(&mut self);
        #[qsignal]
        fn conflict_target_detail_changed(&mut self);
        #[qsignal]
        fn conflict_job_id_changed(&mut self);
        #[qsignal]
        fn conflict_waiting_changed(&mut self);
        #[qsignal]
        fn queued_transfers_changed(&mut self);
        #[qsignal]
        fn queued_transfer_paths_changed(&mut self);
        #[qsignal]
        fn transfers_json_changed(&mut self);
        #[qsignal]
        fn transfer_speed_limit_changed(&mut self);
        #[qsignal]
        fn transfer_speed_changed(&mut self);
        #[qsignal]
        fn clipboard_mode_changed(&mut self);
        #[qsignal]
        fn clipboard_paths_changed(&mut self);
        #[qsignal]
        fn accent_changed(&mut self);
        #[qsignal]
        fn system_dark_changed(&mut self);
        #[qsignal]
        fn home_directory_changed(&mut self);
        #[qsignal]
        fn terminal_font_size_changed(&mut self);
        #[qsignal]
        fn terminal_line_height_changed(&mut self);
        #[qsignal]
        fn terminal_font_family_changed(&mut self);
        #[qsignal]
        fn shortcuts_json_changed(&mut self);
        #[qsignal]
        fn log_directory_changed(&mut self);
        #[qsignal]
        fn search_index_available_changed(&mut self);
        #[qsignal]
        fn open_with_picker_supported_changed(&mut self);
        #[qsignal]
        fn default_manager_supported_changed(&mut self);
        #[qsignal]
        fn default_manager_is_default_changed(&mut self);
        #[qsignal]
        fn default_manager_is_registered_changed(&mut self);
        #[qsignal]
        fn default_manager_detail_changed(&mut self);
        #[qsignal]
        fn default_manager_error_changed(&mut self);
        #[qsignal]
        fn default_manager_busy_changed(&mut self);

        notifying_setters! {
            set_external_archive_formats(String) => external_archive_formats, external_archive_formats_changed;
            set_drive_capabilities(String) => drive_capabilities, drive_capabilities_changed;
            set_custom_actions_json(String) => custom_actions_json, custom_actions_json_changed;
            set_operations_json(String) => operations_json, operations_json_changed;
            set_failure_count(i32) => failure_count, failure_count_changed;
            set_status(String) => status, status_changed;
            set_last_error(String) => last_error, last_error_changed;
            set_busy(bool) => busy, busy_changed;
            set_can_undo(bool) => can_undo, can_undo_changed;
            set_can_redo(bool) => can_redo, can_redo_changed;
            set_progress_label(String) => progress_label, progress_label_changed;
            set_progress_done(i32) => progress_done, progress_done_changed;
            set_progress_total(i32) => progress_total, progress_total_changed;
            set_progress_kind(String) => progress_kind, progress_kind_changed;
            set_conflict_index(i32) => conflict_index, conflict_index_changed;
            set_conflict_total(i32) => conflict_total, conflict_total_changed;
            set_conflict_name(String) => conflict_name, conflict_name_changed;
            set_conflict_cut(bool) => conflict_cut, conflict_cut_changed;
            set_conflict_source_category(String) => conflict_source_category, conflict_source_category_changed;
            set_conflict_source_detail(String) => conflict_source_detail, conflict_source_detail_changed;
            set_conflict_target_category(String) => conflict_target_category, conflict_target_category_changed;
            set_conflict_target_detail(String) => conflict_target_detail, conflict_target_detail_changed;
            set_conflict_job_id(i64) => conflict_job_id, conflict_job_id_changed;
            set_conflict_waiting(i32) => conflict_waiting, conflict_waiting_changed;
            set_queued_transfers(Vec<String>) => queued_transfers, queued_transfers_changed;
            set_queued_transfer_paths(Vec<String>) => queued_transfer_paths, queued_transfer_paths_changed;
            set_transfers_json(String) => transfers_json, transfers_json_changed;
            set_transfer_speed_limit(i32) => transfer_speed_limit, transfer_speed_limit_changed;
            set_transfer_speed(String) => transfer_speed, transfer_speed_changed;
            set_clipboard_mode(String) => clipboard_mode, clipboard_mode_changed;
            set_clipboard_paths(Vec<String>) => clipboard_paths, clipboard_paths_changed;
            set_accent(String) => accent, accent_changed;
            set_system_dark(bool) => system_dark, system_dark_changed;
            set_home_directory(String) => home_directory, home_directory_changed;
            set_terminal_font_size(i32) => terminal_font_size, terminal_font_size_changed;
            set_terminal_line_height(f64) => terminal_line_height, terminal_line_height_changed;
            set_terminal_font_family(String) => terminal_font_family, terminal_font_family_changed;
            set_shortcuts_json(String) => shortcuts_json, shortcuts_json_changed;
            set_log_directory(String) => log_directory, log_directory_changed;
            set_search_index_available(bool) => search_index_available, search_index_available_changed;
            set_open_with_picker_supported(bool) => open_with_picker_supported, open_with_picker_supported_changed;
            set_default_manager_supported(bool) => default_manager_supported, default_manager_supported_changed;
            set_default_manager_is_default(bool) => default_manager_is_default, default_manager_is_default_changed;
            set_default_manager_is_registered(bool) => default_manager_is_registered, default_manager_is_registered_changed;
            set_default_manager_detail(String) => default_manager_detail, default_manager_detail_changed;
            set_default_manager_error(String) => default_manager_error, default_manager_error_changed;
            set_default_manager_busy(bool) => default_manager_busy, default_manager_busy_changed;
        }

        #[qsignal]
        fn operation_finished(&mut self, kind: String, focus: String);
        #[qsignal]
        fn operation_failed(&mut self, kind: String, message: String);
        #[qsignal]
        fn compare_finished(&mut self, equal: bool);
        #[qsignal]
        fn folder_sizes_measured(&mut self, sizes: String);
        #[qsignal]
        fn file_holders_found(&mut self,

            request: String,

            report: String,

            error: String,);
        #[qsignal]
        fn process_ended(&mut self, pid: i32, ok: bool, message: String);
        #[qsignal]
        fn archive_password_required(&mut self,

            target: String,

            message: String,);
        #[qsignal]
        fn favorites_changed(&mut self);
        #[qsignal]
        fn places_changed(&mut self);
        #[qsignal]
        fn volumes_changed(&mut self);
        #[qsignal]
        fn connection_test_finished(&mut self, ok: bool, message: String);
        #[qsignal]
        fn connection_save_finished(&mut self,

            ok: bool,

            id: String,

            message: String,);
        #[qsignal]
        fn cloud_authorize_finished(&mut self, ok: bool, message: String);
        #[qsignal]
        fn connections_changed(&mut self);


        fn begin_operation(&mut self, label: &str) {
            self.operation_starts
                .push((now_millis(), label.to_owned()));
            self.retry_hint = None;
            let active = self.active_operations + 1;
            self.active_operations = active;
            self.set_busy(true);
            self.set_progress_done(0);
            self.set_progress_total(0);
            // Unknown until the operation reports: a rename sends no progress at
            // all, and a stale code from the previous command would make the strip
            // read the new one's numbers in the wrong unit.
            self.set_progress_kind(String::new());
            self.set_progress_label(label.to_owned());
            self.set_last_error(String::new());
        }

        pub(super) fn finish_operation(&mut self, kind: &str, focus: &str, error: Option<String>) {
            self.finish_operation_state(kind, focus, error, false);
        }

        /// A transfer the user stopped: no error toast, no failure badge, and a
        /// status-center row whose action is "continue" rather than "retry".
        pub(super) fn finish_cancelled_operation(&mut self, kind: &str) {
            self.finish_operation_state(kind, "", None, true);
        }

        fn finish_operation_state(
            &mut self,
            kind: &str,
            focus: &str,
            error: Option<String>,
            cancelled: bool,
        ) {
            self.settle_operation();
            self.record_operation_state(kind, error.as_deref(), cancelled);

            let kind_value = kind.to_owned();
            let focus_value = focus.to_owned();

            if cancelled {
                // Neither a success nor an error: the status bar says what
                // happened, and the status center's row offers to continue.
                let status = "Transfer cancelled";
                self.set_status(status.to_owned());
                self.operation_finished(kind_value, focus_value);
                return;
            }

            match error {
                Some(message) => {
                    self.set_last_error(message.to_owned());
                    let message_value = message.to_owned();
                    self.operation_failed(kind_value, message_value);
                }
                None => {
                    let status = success_message(kind);
                    self.set_status(status.to_owned());
                    self.operation_finished(kind_value, focus_value);
                }
            }
        }

        /// Turns a drive tool's outcome into the same toast and status-center row
        /// every other command uses.
        fn run_drive_action(
            &mut self,
            kind: &str,
            result: Result<(), FileSystemError>,
            success: &str,
        ) {
            match result {
                Ok(()) => {
                    self.begin_operation(success);
                    self.set_status(success.to_owned());
                    self.finish_operation(kind, "", None);
                }
                Err(error) => {
                    let message = error.to_string();
                    self.finish_operation(kind, "", Some(message));
                }
            }
        }

        /// One line for the status center. Runs before the toast, so a failed
        /// operation is already in the history when its message appears.
        fn record_operation(&mut self, kind: &str, error: Option<&str>) {
            self.record_operation_state(kind, error, false);
        }

        fn record_operation_state(
            &mut self,
            kind: &str,
            error: Option<&str>,
            cancelled: bool,
        ) {
            let (started, label) = self.operation_starts
                .pop()
                .unwrap_or_else(|| (now_millis(), String::new()));
            let retry = self.retry_hint
                .take()
                .unwrap_or_default();
            self.record_operation_row(kind, error, cancelled, started, label, retry);
        }

        /// Writes one status-center row from explicit provenance. Transfers call
        /// this with their own row's start, label and retry payload: several run
        /// at once, so the shared `operation_starts` stack — newest start finishes
        /// next — cannot be theirs.
        fn record_operation_row(
            &mut self,
            kind: &str,
            error: Option<&str>,
            cancelled: bool,
            started: u64,
            label: String,
            retry: String,
        ) {
            let retryable = !retry.is_empty() && matches!(kind, "copy" | "move" | "extract");

            // A cancelled *copy* is continued, not repeated: the retry payload
            // gains `resume`, which makes the job scheduler skip the conflict
            // check and merge into what already landed. A cancelled move cannot be
            // continued — the entries it finished are no longer at the source — so
            // it keeps the ordinary retry.
            let retry = if cancelled && kind == "copy" && retryable {
                match serde_json::from_str::<serde_json::Value>(&retry) {
                    Ok(mut payload) => {
                        payload["resume"] = serde_json::json!(true);
                        payload.to_string()
                    }
                    Err(_) => retry,
                }
            } else {
                retry
            };

            // Cancellation outranks the error slot: `finish_cancelled_operation`
            // passes no error, and a control that failed while being cancelled is
            // still a cancellation to the status center, not a failure to retry.
            let status = if cancelled {
                "cancelled"
            } else if error.is_some() {
                "failed"
            } else {
                "ok"
            };

            let record = OperationRecord {
                id: operation_id(),
                kind: kind.to_owned(),
                label: if label.is_empty() {
                    success_message(kind).to_owned()
                } else {
                    label
                },
                started,
                finished: now_millis(),
                status: status.to_owned(),
                detail: if cancelled {
                    "Cancelled".to_owned()
                } else {
                    error.unwrap_or_else(|| success_message(kind)).to_owned()
                },
                retryable,
                retry: if retryable { retry } else { String::new() },
                // Elevation is a *retry* on a path the user's token cannot write,
                // so it is only offered where a copy or a move actually failed and
                // the platform can raise a separate process.
                elevatable: status == "failed"
                    && matches!(kind, "copy" | "move")
                    && cfg!(any(windows, target_os = "linux", target_os = "macos")),
            };

            let failed = record.status == "failed";
            {
                let rust = &mut *self;
                rust.operations.push(record);
                let excess = rust
                    .operations
                    .len()
                    .saturating_sub(OPERATION_HISTORY_LIMIT);
                if excess > 0 {
                    rust.operations.drain(0..excess);
                }
                persist_operations(&rust.operations);
            }
            let json = operations_json(&self.operations);
            self.set_operations_json(json.to_owned());
            if failed {
                let count = self.failure_count + 1;
                self.set_failure_count(count);
            }
        }

        #[qslot]

        fn eject_drive(&mut self, path: String) {
            self.run_drive_action("drive", drives::eject(&path.to_string()), "Ejected");
        }

        #[qslot]

        fn unmount_drive(&mut self, path: String) {
            self.run_drive_action("drive", drives::unmount(&path.to_string()), "Unmounted");
        }

        #[qslot]

        fn mount_drive(&mut self, path: String) {
            self.run_drive_action("drive", drives::mount(&path.to_string()), "Mounted");
        }

        #[qslot]

        fn format_drive(&mut self, path: String) {
            self.run_drive_action(
                "drive",
                drives::open_format_tool(&path.to_string()),
                "Format dialog opened",
            );
        }

        #[qslot]

        fn open_storage_cleanup(&mut self) {
            self.run_drive_action(
                "drive",
                drives::open_cleanup_tool(),
                "Storage settings opened",
            );
        }

        #[qslot]

        fn drive_encryption_status(&mut self, path: String) -> String {
            match drives::encryption_status(&path.to_string()) {
                Ok(text) => text.to_owned(),
                Err(error) => error.to_string(),
            }
        }

        #[qslot]

        fn open_previous_versions(&mut self, path: String) {
            self.run_drive_action(
                "drive",
                drives::open_previous_versions(&path.to_string()),
                "Previous versions opened",
            );
        }

        #[qslot]

        fn custom_action_definitions(&self) -> String {
            let actions = dae_core::settings::load_settings()
                .map(|settings| settings.actions)
                .unwrap_or_default();
            let entries: Vec<serde_json::Value> = actions
                .into_iter()
                .map(|action| {
                    serde_json::json!({
                        "id": action.id,
                        "name": action.name,
                        "command": action.command,
                        "args": action.args.join("\n"),
                        "selection": action.selection,
                        "extensions": action.extensions.join(","),
                        "platforms": action.platforms.join(","),
                    })
                })
                .collect();
            let json = serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_owned());
            json.to_owned()
        }

        #[qslot]

        fn custom_actions_for(
            &mut self,
            paths: Vec<String>,
            directory: String,
        ) -> String {
            let selection = paths;
            let directory = directory.to_string();
            let entries: Vec<serde_json::Value> = actions::available_actions()
                .into_iter()
                .filter(|action| actions::selection_allows(action, &selection, &directory))
                .map(|action| serde_json::json!({"id": action.id, "name": action.name}))
                .collect();
            let json = serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_owned());
            json.to_owned()
        }

        #[qslot]

        fn run_custom_action(
            &mut self,
            id: String,
            paths: Vec<String>,
            directory: String,
        ) {
            let id = id.to_string();
            let selection = paths;
            let directory = directory.to_string();
            let Some(action) = actions::action_by_id(&id) else {
                self.finish_operation("action", "", Some("Unknown action".to_owned()));
                return;
            };
            if !actions::selection_allows(&action, &selection, &directory) {
                self.finish_operation(
                    "action",
                    "",
                    Some("This action does not apply to the selection".to_owned()),
                );
                return;
            }
            match actions::run(&action, &selection, &directory) {
                Ok(program) => {
                    self.begin_operation("Running action…");
                    self.set_status(format!("Ran {program}"));
                    self.finish_operation("action", "", None);
                }
                Err(error) => {
                    self.finish_operation("action", "", Some(error.to_string()));
                }
            }
        }

        #[qslot]

        fn share_supported(&mut self) -> bool {
            crate::share::supported()
        }

        #[qslot]

        fn share_paths(&self, paths: Vec<String>, handle: f64) -> bool {
            crate::share::share_paths(&paths, handle)
        }

        #[qslot]

        fn save_custom_action(
            &mut self,
            id: String,
            name: String,
            command: String,
            args: String,
            selection: String,
        ) {
            let Ok(mut settings) = dae_core::settings::load_settings() else {
                return;
            };
            let id = id.to_string();
            let action = dae_core::settings::CustomAction {
                id: id.clone(),
                name: name.to_string(),
                command: command.to_string(),
                args: action_args(&args.to_string()),
                platforms: Vec::new(),
                selection: selection.to_string(),
                extensions: Vec::new(),
            };

            if id.trim().is_empty() {
                let mut candidate = settings.actions.len() + 1;
                loop {
                    let generated = format!("custom-{candidate}");
                    if !settings.actions.iter().any(|entry| entry.id == generated) {
                        settings.actions.push(dae_core::settings::CustomAction {
                            id: generated,
                            ..action
                        });
                        break;
                    }
                    candidate += 1;
                }
            } else if let Some(existing) = settings.actions.iter_mut().find(|entry| entry.id == id) {
                *existing = action;
            } else {
                settings.actions.push(action);
            }

            if dae_core::settings::save_settings(settings).is_ok() {
                let json = custom_actions_json();
                self.set_custom_actions_json(json.to_owned());
            }
        }

        #[qslot]

        fn remove_custom_action(&mut self, id: String) {
            let id = id.to_string();
            let Ok(mut settings) = dae_core::settings::load_settings() else {
                return;
            };
            settings.actions.retain(|action| action.id != id);
            if dae_core::settings::save_settings(settings).is_ok() {
                let json = custom_actions_json();
                self.set_custom_actions_json(json.to_owned());
            }
        }

        #[qslot]

        fn reset_custom_actions(&mut self) {
            let Ok(mut settings) = dae_core::settings::load_settings() else {
                return;
            };
            settings.actions = dae_core::settings::default_actions();
            if dae_core::settings::save_settings(settings).is_ok() {
                let json = custom_actions_json();
                self.set_custom_actions_json(json.to_owned());
            }
        }

        #[qslot]

        fn mark_operations_read(&mut self) {
            if self.failure_count != 0 {
                self.set_failure_count(0);
            }
        }

        #[qslot]

        fn clear_operation_history(&mut self) {
            self.operations.clear();
            persist_operations(&[]);
            self.set_operations_json("[]".to_owned());
            self.set_failure_count(0);
        }

        #[qslot]

        fn retry_operation(&mut self, id: String) {
            let id = id.to_string();
            let Some(record) = self
                .operations
                .iter()
                .find(|record| record.id == id)
                .cloned()
            else {
                return;
            };
            if !record.retryable {
                return;
            }
            let Ok(payload) = serde_json::from_str::<serde_json::Value>(&record.retry) else {
                return;
            };
            match payload["kind"].as_str() {
                Some("extract") => {
                    let archive = payload["archive"].as_str().unwrap_or_default();
                    let destination = payload["destination"].as_str().unwrap_or_default();
                    self.extract_archive(
                        String::from(archive),
                        String::from(destination),
                        String::default(),
                    );
                }
                Some("transfer") => {
                    let mut paths = Vec::new();
                    if let Some(items) = payload["paths"].as_array() {
                        for item in items {
                            if let Some(path) = item.as_str() {
                                paths.push(path.to_owned());
                            }
                        }
                    }
                    let destination = payload["destination"].as_str().unwrap_or_default();
                    let cut = payload["cut"].as_bool().unwrap_or(false);
                    let resume = payload["resume"].as_bool().unwrap_or(false);

                    if paths.len() > 0 && resume {
                        self.enqueue_resumed_transfer(
                            paths,
                            destination.to_owned(),
                            cut,
                        );
                    } else if paths.len() > 0 {
                        self.transfer_entries(paths, String::from(destination), cut);
                    }
                }
                _ => {}
            }
        }

        /// Clears the progress row without reporting an outcome. A protected
        /// archive uses this before asking for a password: nothing failed yet, and
        /// an `operationFinished` would wrongly say the extraction completed.
        fn settle_operation(&mut self) {
            let active = (self.active_operations - 1).max(0);
            self.active_operations = active;
            self.set_busy(active > 0);
            self.set_progress_done(0);
            self.set_progress_total(0);
            self.set_progress_kind(String::new());
            self.set_progress_label(String::new());
        }

        fn request_archive_password(&mut self, target: &str, message: &str) {
            let target = target.to_owned();
            let message = message.to_owned();
            self.set_last_error(message.clone());
            self.archive_password_required(target, message);
        }

        pub(super) fn apply_event(&mut self, event: Event) {
            match event {
                Event::FileOperationProgress {
                    operation_id,
                    operation,
                    phase,
                    completed,
                    total,
                    current_path,
                } => {
                    // A transfer's ticks belong to its row; everything else (a
                    // rename, a trash run, a size walk) keeps the shared progress
                    // row in the status strip.
                    let is_transfer = self
                        .transfer_rows
                        .iter()
                        .any(|row| row.operation_id.as_deref() == Some(operation_id.as_str()));
                    if is_transfer {
                        let rust = &mut *self;
                        let row = rust
                            .transfer_rows
                            .iter_mut()
                            .find(|row| row.operation_id.as_deref() == Some(operation_id.as_str()))
                            .expect("row just found");
                        row.done = completed;
                        if let Some(total) = total {
                            row.total = total;
                        }
                        row.current = current_path
                            .as_deref()
                            .map(display_name_from_path)
                            .unwrap_or_default();
                        // Only a copy or a move counts bytes: a delete reports
                        // entries, and calling that a rate would be a lie.
                        if matches!(
                            operation,
                            dae_core::progress::FileOperationKind::Copy
                                | dae_core::progress::FileOperationKind::Move
                        ) {
                            row.sample_speed(completed);
                        }
                        self.publish_transfers();
                        return;
                    }

                    let label = current_path
                        .as_deref()
                        .map(display_name_from_path)
                        .unwrap_or_else(|| operation_label(operation).to_owned());
                    self.set_progress_label(label.to_owned());
                    self.set_progress_kind(operation.code().to_owned());
                    self.set_progress_done(completed.min(i32::MAX as u64) as i32);
                    let total = total.unwrap_or(0).min(i32::MAX as u64) as i32;
                    self.set_progress_total(total);

                    if matches!(phase, dae_core::progress::FileOperationPhase::Preparing) {
                        self.set_busy(true);
                    }
                }
                Event::TransferJob(event) => self.apply_job_event(event),
                Event::UndoRedoChanged { can_undo, can_redo } => {
                    self.set_can_undo(can_undo);
                    self.set_can_redo(can_redo);
                }
                Event::DirectoryChanged { .. }
                | Event::DirectoryEntries { .. }
                | Event::SystemThemeChanged(_)
                | Event::SystemAccentChanged(_)
                | Event::DirectorySizeProgress { .. }
                | Event::FileHashProgress { .. } => {}
            }
        }

        #[qslot]

        fn create_folder(&mut self, directory: String, name: String) {
            self.create_entry(directory, name, NewEntryKind::Directory, "create");
        }

        #[qslot]

        fn create_file(&mut self, directory: String, name: String) {
            self.create_entry(directory, name, NewEntryKind::File, "create");
        }

        #[qslot]

        fn create_from_template(
            &mut self,
            directory: String,
            template_path: String,
            name: String,
        ) {
            let directory = directory.to_string();
            let template_path = template_path.to_string();
            let name = name.to_string();
            if name.trim().is_empty() {
                self.finish_operation("create", "", Some("Name cannot be empty".to_owned()));
                return;
            }
            if !vfs::is_local_path(&directory) || !vfs::is_local_path(&template_path) {
                self.finish_operation(
                    "create",
                    "",
                    Some("Templates can only be used in local folders".to_owned()),
                );
                return;
            }

            self.begin_operation("Creating…");
            let host = host_for(self.get_thread_handle());
            let thread = self.get_thread_handle();
            dispatch(thread, "create", async move {
                match service::create_from_template(directory, template_path, name, host).await {
                    Ok(created) => Outcome::ok(created),
                    Err(error) => Outcome::failed(error),
                }
            });
        }

        #[qslot]

        fn templates_json(&self) -> String {
            match templates::list() {
                Ok(entries) => {
                    let text = serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_owned());
                    text.to_owned()
                }
                Err(_) => "[]".to_owned(),
            }
        }

        #[qslot]

        fn open_templates_directory(&mut self) {
            let directory = match templates::ensure_directory() {
                Ok(directory) => directory,
                Err(error) => {
                    self.finish_operation("open", "", Some(error.to_string()));
                    return;
                }
            };
            let path = directory.to_string_lossy().into_owned();
            if let Err(message) = platform::open_path(&path) {
                self.finish_operation("open", "", Some(message));
                return;
            }
        }

        #[qslot]

        fn set_templates_directory(&mut self, path: String) {
            let directory = path.to_string().trim().to_owned();
            if directory.is_empty() || !Path::new(&directory).is_dir() {
                self.set_last_error("The templates folder does not exist".to_owned());
                return;
            }

            let Ok(mut settings) = dae_core::settings::load_settings() else {
                self.set_last_error("Could not read settings".to_owned());
                return;
            };
            settings.templates.directory = Some(directory);
            match dae_core::settings::save_settings(settings) {
                Ok(_) => self.set_last_error(String::new()),
                Err(error) => self.set_last_error(error.to_string()),
            }
        }

        #[qslot]

        fn file_holders(&mut self, paths: Vec<String>) {
            let requested: Vec<String> = paths
                .into_iter()
                .filter(|path| !path.trim().is_empty())
                .collect();
            let request =
                serde_json::to_string(&requested).unwrap_or_else(|_| "[]".to_owned());
            let local: Vec<String> = requested
                .iter()
                .filter(|path| vfs::is_local_path(path))
                .cloned()
                .collect();

            if local.is_empty() {
                let report = r#"{"holders":[],"scanned":0,"truncated":false}"#.to_owned();
                let request_value = request.to_owned();
                let error = "Locksmith only checks local paths".to_owned();
                self.file_holders_found(request_value, report, error);
                return;
            }

            let thread = self.get_thread_handle();
            runtime::spawn(async move {
                let outcome =
                    tokio::task::spawn_blocking(move || occupancy::holders_deep(&local)).await;

                thread.invoke_mut(move |workspace| match outcome {
                    Ok(report) => {
                        let report =
                            serde_json::to_string(&report).unwrap_or_else(|_| "{}".to_owned());
                        let request_value = request.to_owned();
                        let report_value = report.to_owned();
                        workspace.file_holders_found(
                            request_value,
                            report_value,
                            String::default(),
                        );
                    }
                    Err(error) => {
                        let request_value = request.to_owned();
                        let report_value =
                            r#"{"holders":[],"scanned":0,"truncated":false}"#.to_owned();
                        let message = error.to_string();
                        workspace.file_holders_found(
                            request_value,
                            report_value,
                            message,
                        );
                    }
                });
            });
        }

        #[qslot]

        fn end_process(&mut self, pid: i32) {
            let thread = self.get_thread_handle();
            runtime::spawn(async move {
                let outcome = tokio::task::spawn_blocking(move || {
                    if pid <= 0 {
                        return Err(FileSystemError::InvalidInput(
                            "No process was selected".to_owned(),
                        ));
                    }
                    occupancy::end_process(pid as u32)
                })
                .await;

                thread.invoke_mut(move |workspace| match outcome {
                    Ok(Ok(())) => workspace
                        .process_ended(pid, true, String::default()),
                    Ok(Err(error)) => {
                        let message = error.to_string();
                        workspace.process_ended(
                            pid,
                            false,
                            String::from(message.as_str()),
                        );
                    }
                    Err(error) => {
                        let message = error.to_string();
                        workspace.process_ended(
                            pid,
                            false,
                            String::from(message.as_str()),
                        );
                    }
                });
            });
        }

        fn create_entry(
            &mut self,
            directory: String,
            name: String,
            entry_kind: NewEntryKind,
            operation_kind: &'static str,
        ) {
            let directory = directory.to_string();
            let name = name.to_string();
            if name.trim().is_empty() {
                self.finish_operation(
                    operation_kind,
                    "",
                    Some("Name cannot be empty".to_owned()),
                );
                return;
            }

            self.begin_operation("Creating…");
            let host = host_for(self.get_thread_handle());
            let thread = self.get_thread_handle();
            dispatch(thread, operation_kind, async move {
                match service::create_entry(directory, name, entry_kind, host).await {
                    Ok(created) => Outcome::ok(created),
                    Err(error) => Outcome::failed(error),
                }
            });
        }

        #[qslot]

        fn rename_entry(&mut self, path: String, new_name: String) {
            let path = path.to_string();
            let new_name = new_name.to_string();
            if new_name.trim().is_empty() {
                self.finish_operation("rename", "", Some("Name cannot be empty".to_owned()));
                return;
            }

            self.begin_operation("Renaming…");
            let focus = dae_core::fs::undo::renamed_path(&path, &new_name).unwrap_or_default();
            let host = host_for(self.get_thread_handle());
            let thread = self.get_thread_handle();
            dispatch(thread, "rename", async move {
                match service::rename_entry(path, new_name, host).await {
                    Ok(()) => Outcome::ok(focus),
                    Err(error) => Outcome::failed(error),
                }
            });
        }

        #[qslot]

        fn compare_hashes(&mut self, paths: Vec<String>) {
            let paths = paths;
            if paths.len() != 2 {
                return;
            }
            if !paths.iter().all(|path| vfs::is_local_path(path)) {
                self.finish_operation(
                    "compare",
                    "",
                    Some("Hashes can only be compared for local files".to_owned()),
                );
                return;
            }

            self.begin_operation("Comparing hashes…");
            let thread = self.get_thread_handle();
            runtime::spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    let first = hashing::digests(Path::new(&paths[0]))?;
                    let second = hashing::digests(Path::new(&paths[1]))?;
                    Ok::<bool, FileSystemError>(first.sha256 == second.sha256)
                })
                .await;

                thread.invoke_mut(move |workspace| match result {
                    Ok(Ok(equal)) => {
                        workspace.settle_operation();
                        workspace.record_operation("compare", None);
                        workspace.compare_finished(equal);
                    }
                    Ok(Err(error)) => {
                        workspace
                            .finish_operation("compare", "", Some(error.to_string()))
                    }
                    Err(error) => {
                        workspace
                            .finish_operation("compare", "", Some(error.to_string()))
                    }
                });
            });
        }

        #[qslot]

        fn measure_folders(&mut self, paths: Vec<String>) {
            let directories: Vec<String> = paths
                .into_iter()
                .filter(|path| vfs::is_local_path(path) && Path::new(path).is_dir())
                .collect();
            if directories.is_empty() {
                return;
            }

            // A second pass replaces the first: the user asked again, and two
            // walks of the same tree would report the same numbers twice.
            let cancel = Arc::new(AtomicBool::new(false));
            if let Some(previous) = self.size_scan.replace(cancel.clone()) {
                previous.store(true, Ordering::Relaxed);
            }

            self.begin_operation("Calculating sizes…");
            let thread = self.get_thread_handle();
            runtime::spawn(async move {
                let measured = tokio::task::spawn_blocking(move || {
                    let mut measured: Vec<(String, u64)> = Vec::with_capacity(directories.len());
                    for path in directories {
                        if cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        let bytes = directory_size::measure_directory(Path::new(&path), &cancel);
                        measured.push((path, bytes));
                    }
                    measured
                })
                .await
                .unwrap_or_default();

                let mut payload = String::from("[");
                for (index, (path, bytes)) in measured.iter().enumerate() {
                    if index > 0 {
                        payload.push(',');
                    }
                    let escaped = serde_json::to_string(path).unwrap_or_else(|_| "\"\"".to_owned());
                    payload.push_str(&format!("{{\"path\":{escaped},\"size\":{bytes}}}"));
                }
                payload.push(']');

                thread.invoke_mut(move |workspace| {
                    workspace.finish_size_scan(payload);
                });
            });
        }

        #[qslot]

        fn cancel_measure_folders(&mut self) {
            let cancelled = self.size_scan.take();
            if let Some(cancel) = cancelled {
                cancel.store(true, Ordering::Relaxed);
                self.settle_operation();
                self.set_status("Size calculation cancelled".to_owned());
            }
        }

        fn finish_size_scan(&mut self, payload: String) {
            self.size_scan = None;
            let sizes = payload.to_owned();
            self.folder_sizes_measured(sizes);
            self.settle_operation();
            self.set_status("Sizes calculated".to_owned());
        }

        #[qslot]

        fn trash_entries(&mut self, paths: Vec<String>) {
            let paths = paths;
            if paths.is_empty() {
                return;
            }
            self.begin_operation("Moving to Trash…");
            let host = host_for(self.get_thread_handle());
            let thread = self.get_thread_handle();
            dispatch(thread, "trash", async move {
                match service::trash_entries(paths, operation_id(), host).await {
                    Ok(()) => Outcome::empty(),
                    Err(error) => Outcome::failed(error),
                }
            });
        }

        #[qslot]

        fn delete_entries(&mut self, paths: Vec<String>) {
            let paths = paths;
            if paths.is_empty() {
                return;
            }
            self.begin_operation("Deleting…");
            let host = host_for(self.get_thread_handle());
            let thread = self.get_thread_handle();
            dispatch(thread, "delete", async move {
                match service::delete_entries(paths, operation_id(), host).await {
                    Ok(()) => Outcome::empty(),
                    Err(error) => Outcome::failed(error),
                }
            });
        }

        #[qslot]

        fn duplicate_entries(&mut self, paths: Vec<String>) {
            let paths = paths;
            if paths.is_empty() {
                return;
            }
            self.begin_operation("Duplicating…");
            let host = host_for(self.get_thread_handle());
            let thread = self.get_thread_handle();
            dispatch(thread, "duplicate", async move {
                match service::duplicate_entries(paths, operation_id(), host).await {
                    Ok(created) => Outcome::ok(created.into_iter().next().unwrap_or_default()),
                    Err(error) => Outcome::failed(error),
                }
            });
        }

        #[qslot]

        fn compress_entries(
            &mut self,
            sources: Vec<String>,
            destination_dir: String,
            format: String,
            password: String,
        ) {
            let sources = sources;
            if sources.is_empty() {
                self.finish_operation(
                    "compress",
                    "",
                    Some("Select items to compress".to_owned()),
                );
                return;
            }

            let destination_dir = destination_dir.to_string();
            if destination_dir.is_empty() {
                self.finish_operation(
                    "compress",
                    "",
                    Some("Cannot determine archive destination".to_owned()),
                );
                return;
            }

            let Some(format) = archive_format(&format.to_string()) else {
                self.finish_operation(
                    "compress",
                    "",
                    Some("Unsupported archive format".to_owned()),
                );
                return;
            };

            let password = optional_string(&password);
            self.begin_operation("Compressing…");
            let host = host_for(self.get_thread_handle());
            let thread = self.get_thread_handle();
            dispatch(thread, "compress", async move {
                match archive::compress_entries(
                    sources,
                    destination_dir,
                    format,
                    password,
                    operation_id(),
                    host,
                )
                .await
                {
                    Ok(archive_path) => Outcome::ok(archive_path),
                    Err(error) => Outcome::failed(error),
                }
            });
        }

        #[qslot]

        fn extract_archive(
            &mut self,
            archive_path: String,
            destination_dir: String,
            password: String,
        ) {
            let archive_path = archive_path.to_string();
            if archive_path.is_empty() {
                return;
            }

            let destination_dir = optional_string(&destination_dir);
            let password = optional_string(&password);
            let password_supplied = password.is_some();
            self.begin_operation("Extracting…");
            let retry = serde_json::json!({
                "kind": "extract",
                "archive": archive_path,
                "destination": destination_dir.clone().unwrap_or_default(),
            });
            self.retry_hint = Some(retry.to_string());
            let host = host_for(self.get_thread_handle());
            let thread = self.get_thread_handle();
            let signal_target = archive_path.clone();
            runtime::spawn(async move {
                let result = archive::extract_archive(
                    archive_path,
                    destination_dir,
                    password,
                    operation_id(),
                    host,
                )
                .await;

                thread.invoke_mut(move |workspace| match result {
                    Ok(destination) => {
                        workspace
                            .finish_operation("extract", &destination, None)
                    }
                    Err(FileSystemError::WrongPassword(_)) => {
                        let message = if password_supplied {
                            "Incorrect password. Try again."
                        } else {
                            "This archive is encrypted. Enter a password."
                        };
                        workspace.settle_operation();
                        workspace
                            .request_archive_password(&signal_target, message);
                    }
                    Err(error) => workspace.finish_operation(
                        "extract",
                        "",
                        Some(friendly_error(&error.to_string())),
                    ),
                });
            });
        }

        #[qslot]

        fn transfer_entries(
            &mut self,
            paths: Vec<String>,
            destination: String,
            cut: bool,
        ) {
            let sources = paths;
            let destination = destination.to_string();
            if sources.is_empty() || destination.is_empty() {
                return;
            }
            // A cut is one-shot, but only for the list that was actually cut: a
            // move dragged in from Explorer must not discard an in-app cut
            // clipboard that happens to be unrelated.
            let clears_clipboard = cut
                && self.clipboard_mode.to_string() == "cut"
                && self.clipboard_paths == sources;
            let request = TransferRequest {
                sources,
                destination,
                cut,
                resume: false,
                spends_clipboard: clears_clipboard,
            };
            self.scheduler().submit(request);
            self.publish_queue();
        }

        /// The core's job scheduler, created on first use: it needs a Qt thread
        /// to route its events back through, which only exists once QML calls in.
        fn scheduler(&mut self) -> Arc<JobScheduler> {
            if let Some(scheduler) = &self.transfer_scheduler {
                return Arc::clone(scheduler);
            }
            let scheduler = JobScheduler::new(
                runtime::runtime().handle().clone(),
                host_for(self.get_thread_handle()),
            );
            self.transfer_scheduler = Some(Arc::clone(&scheduler));
            scheduler
        }

        /// Queues the "continue" of a cancelled copy: the same sources into the
        /// same destination, merged rather than checked for collisions.
        fn enqueue_resumed_transfer(
            &mut self,
            sources: Vec<String>,
            destination: String,
            cut: bool,
        ) {
            if sources.is_empty() || destination.is_empty() {
                return;
            }

            let request = TransferRequest {
                sources,
                destination,
                cut,
                resume: true,
                spends_clipboard: false,
            };
            self.scheduler().submit(request);
            self.publish_queue();
        }

        fn publish_queue(&mut self) {
            let queued = self
                .transfer_scheduler
                .as_ref()
                .map(|scheduler| scheduler.queued())
                .unwrap_or_default();
            let labels: Vec<String> = queued.iter().map(transfer_label).collect();
            let paths: Vec<String> = queued.iter().map(|job| job.destination.clone()).collect();
            self.set_queued_transfers(labels.to_vec());
            self.set_queued_transfer_paths(paths.to_vec());
        }

        /// The queue panel's ✕. Only a waiting job can be dropped: a running one
        /// is cancelled through its operation id, which keeps the half-copied
        /// tree and the "continue" offer honest.
        #[qslot]
        fn cancel_queued_transfer(&mut self, index: i32) {
            if index >= 0
                && let Some(scheduler) = &self.transfer_scheduler
            {
                scheduler.drop_queued(index as usize);
            }
            self.publish_queue();
        }

        #[qslot]

        fn cancel_queued_transfers(&mut self) {
            if let Some(scheduler) = &self.transfer_scheduler {
                scheduler.clear_queued();
            }
            self.publish_queue();
        }

        /// Publishes the rows as the JSON list the panel reads, plus the combined
        /// throughput the chip prints. Called whenever a row appears, moves or
        /// goes away.
        fn publish_transfers(&mut self) {
            let mut list = Vec::new();
            let mut combined_bps = 0.0;
            for row in &self.transfer_rows {
                if row.running {
                    combined_bps += row.speed_bps;
                }
                let percent = (row.done.min(row.total) * 100)
                    .checked_div(row.total)
                    .unwrap_or(0) as i32;
                let eta = if row.running {
                    transfer_eta(row.done, row.total, row.speed_bps)
                        .map(format_eta)
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                list.push(serde_json::json!({
                    "jobId": row.job_id,
                    "operationId": row.operation_id.clone().unwrap_or_default(),
                    "name": transfer_label(&row.request),
                    "target": row.request.destination,
                    "running": row.running,
                    "paused": row.paused,
                    "done": row.done,
                    "total": row.total,
                    "percent": percent,
                    "speed": row.speed,
                    "eta": eta,
                    "current": row.current,
                }));
            }
            let json = serde_json::Value::Array(list).to_string();
            let speed = if combined_bps > 0.0 {
                format_speed(combined_bps)
            } else {
                String::new()
            };
            self.set_transfers_json(json.to_owned());
            self.set_transfer_speed(speed.to_owned());
        }

        /// One running transfer reported back: write its status-center row from
        /// the row's own provenance, free the busy count and drop the row. The
        /// global progress row belongs to the non-transfer operations, so it is
        /// left alone here.
        fn finish_transfer_row(
            &mut self,
            job_id: u64,
            error: Option<String>,
            cancelled: bool,
        ) {
            let Some(position) = self
                .transfer_rows
                .iter()
                .position(|row| row.job_id == job_id)
            else {
                return;
            };
            let row = self.transfer_rows.remove(position);
            let kind = if row.request.cut { "move" } else { "copy" };

            let active = (self.active_operations - 1).max(0);
            self.active_operations = active;
            self.set_busy(active > 0);

            self.record_operation_row(
                kind,
                error.as_deref(),
                cancelled,
                row.started_at,
                row.label.clone(),
                row.retry.clone(),
            );

            let kind_value = kind.to_owned();
            if cancelled {
                self.set_status("Transfer cancelled".to_owned());
                self.operation_finished(kind_value, String::default());
            } else if let Some(message) = error {
                self.set_last_error(message.to_owned());
                let message_value = message.to_owned();
                self.operation_failed(kind_value, message_value);
            } else {
                self.set_status(success_message(kind).to_owned());
                self.operation_finished(kind_value, String::default());
            }
            self.publish_transfers();
        }

        #[qslot]

        fn pause_transfer(&mut self, job_id: i64) {
            let changed = {
                let rust = &mut *self;
                let Some(row) = rust
                    .transfer_rows
                    .iter_mut()
                    .find(|row| row.job_id == job_id as u64)
                else {
                    return;
                };
                let Some(operation) = row.operation_id.clone() else {
                    return;
                };
                // `false` means the transfer returned between the click and here;
                // its row is about to disappear with the Finished event anyway.
                if service::pause_transfer(&operation) {
                    row.paused = true;
                    row.speed = String::new();
                    row.speed_bps = 0.0;
                    row.speed_sample = None;
                    true
                } else {
                    false
                }
            };
            if changed {
                self.publish_transfers();
            }
        }

        #[qslot]

        fn resume_transfer(&mut self, job_id: i64) {
            let changed = {
                let rust = &mut *self;
                let Some(row) = rust
                    .transfer_rows
                    .iter_mut()
                    .find(|row| row.job_id == job_id as u64)
                else {
                    return;
                };
                let Some(operation) = row.operation_id.clone() else {
                    return;
                };
                if service::resume_transfer(&operation) {
                    row.paused = false;
                    true
                } else {
                    false
                }
            };
            if changed {
                self.publish_transfers();
            }
        }

        #[qslot]

        fn cancel_transfer(&mut self, job_id: i64) {
            let rust = &mut *self;
            let Some(row) = rust
                .transfer_rows
                .iter()
                .find(|row| row.job_id == job_id as u64)
            else {
                return;
            };
            let Some(operation) = row.operation_id.clone() else {
                return;
            };
            // The engine notices at its next checkpoint and returns `Cancelled`,
            // whose Finished event reports it as neither success nor failure.
            service::cancel_transfer(&operation);
        }

        #[qslot]

        fn apply_transfer_speed_limit(&mut self, kib_per_second: i32) {
            let kib = (kib_per_second.max(0) as u64).min(MAX_SPEED_LIMIT_KIB);

            // Applied first: the limit is in force for this session even when the
            // settings file cannot be written.
            service::set_transfer_speed_limit((kib > 0).then_some(kib * 1024));
            self.set_transfer_speed_limit(kib as i32);

            if let Ok(mut settings) = dae_core::settings::load_settings()
                && settings.transfer.speed_limit_kib != kib
            {
                settings.transfer.speed_limit_kib = kib;
                if let Err(error) = dae_core::settings::save_settings(settings) {
                    log::warn!(
                        target: "dae::workspace",
                        "could not persist the transfer limit: {error}"
                    );
                }
            }
        }

        /// Retries a failed copy or move in an elevated copy of this binary.
        ///
        /// The record's own retry payload is the job: the same sources, the same
        /// destination, and `Replace` because the failed attempt may have left part
        /// of the tree at the target.
        #[qslot]
        fn elevate_operation(&mut self, id: String) {
            let id = id.to_string();
            let Some(record) = self.operations.iter().find(|record| record.id == id) else {
                return;
            };

            if !record.elevatable {
                return;
            }

            let Ok(payload) = serde_json::from_str::<serde_json::Value>(&record.retry) else {
                return;
            };
            if payload["kind"].as_str() != Some("transfer") {
                return;
            }

            let paths: Vec<String> = payload["paths"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            let destination = payload["destination"].as_str().unwrap_or_default().to_owned();
            let cut = payload["cut"].as_bool().unwrap_or(false);
            let retry = record.retry.clone();

            if paths.is_empty() || destination.is_empty() {
                return;
            }

            let request = TransferRequest {
                sources: paths.clone(),
                destination: destination.clone(),
                cut,
                resume: false,
                spends_clipboard: false,
            };
            let job = dae_core::fs::elevated::ElevatedJob {
                kind: if cut { "move" } else { "copy" }.to_owned(),
                sources: paths
                    .into_iter()
                    .map(|path| TransferItem {
                        path,
                        on_conflict: ConflictAction::Replace,
                    })
                    .collect(),
                destination: Some(destination),
                rate_limit: service::transfer_speed_limit(),
            };

            // The elevated child reports progress and answers pause/cancel under
            // this id, exactly like an in-process transfer, so the same row and
            // its buttons work. `run_elevated` registers the control under it.
            let operation = operation_id();
            let job_id = {
                let rust = &mut *self;
                rust.next_local_job += 1;
                rust.next_local_job
            };
            {
                let rust = &mut *self;
                let mut row = TransferRow::new(job_id, request);
                row.running = true;
                row.operation_id = Some(operation.clone());
                row.label = if cut {
                    "Moving as administrator…"
                } else {
                    "Copying as administrator…"
                }
                .to_owned();
                // The same payload can be retried again if this attempt fails.
                row.retry = retry;
                rust.transfer_rows.push(row);
                rust.active_operations += 1;
            }
            self.set_busy(true);
            self.publish_transfers();

            let host = host_for(self.get_thread_handle());
            let thread = self.get_thread_handle();
            // Not `dispatch`: an elevated transfer owns its row directly (it never
            // went through the scheduler), so its completion settles the row too.
            runtime::spawn(async move {
                let outcome = match service::run_elevated(job, host, operation).await {
                    Ok(()) => Outcome::empty(),
                    Err(error) => Outcome::failed(error),
                };
                thread.invoke_mut(move |workspace| {
                    workspace
                        .finish_transfer_row(job_id, outcome.error, outcome.cancelled);
                });
            });
        }

        /// Renders one lifecycle event from the core's job scheduler. The rules
        /// (queue order, conflict answers, resume) live in `fs::jobs`; everything
        /// here is presentation plus the shell-owned clipboard and retry payload.
        /// Every event names its job, so several transfers can be on screen at
        /// once without their rows crossing.
        fn apply_job_event(&mut self, event: JobEvent) {
            match event {
                JobEvent::Accepted { job_id, request } => {
                    self.transfer_rows
                        .push(TransferRow::new(job_id, request));
                    self.publish_transfers();
                    self.publish_queue();
                }
                JobEvent::ConflictPrompt {
                    job_id,
                    conflict,
                    index,
                    total,
                    cut,
                } => {
                    // Render the payload once, on arrival; the prompt is frozen
                    // until the dialog reaches this job's turn.
                    let prompt = PendingPrompt {
                        job_id,
                        index,
                        total,
                        cut,
                        name: conflict.name.clone(),
                        source_category: glyph_category(conflict.source_kind).to_owned(),
                        source_detail: conflict_detail(
                            conflict.source_kind,
                            &conflict.name,
                            conflict.source_size,
                            conflict.source_modified_at,
                        ),
                        target_category: glyph_category(conflict.target_kind).to_owned(),
                        target_detail: conflict_detail(
                            conflict.target_kind,
                            &conflict.name,
                            conflict.target_size,
                            conflict.target_modified_at,
                        ),
                    };
                    {
                        let rust = &mut *self;
                        match rust
                            .pending_conflicts
                            .iter()
                            .position(|prompt| prompt.job_id == job_id)
                        {
                            // The job on screen advanced to its next collision.
                            Some(at) => rust.pending_conflicts[at] = prompt,
                            // A new job's prompt waits behind the one on screen.
                            None => rust.pending_conflicts.push(prompt),
                        }
                    }
                    self.show_conflict_head();
                }
                JobEvent::Started {
                    job_id,
                    operation_id,
                    request,
                } => {
                    self.forget_pending_conflict(job_id);
                    // A copy clipboard stays available so the same selection can
                    // be pasted into several directories; a cut is spent here,
                    // once, as the move it was waiting for finally starts.
                    if request.spends_clipboard {
                        self.clear_clipboard();
                    }
                    let retry = serde_json::json!({
                        "kind": "transfer",
                        "paths": &request.sources,
                        "destination": &request.destination,
                        "cut": request.cut,
                        "resume": request.resume,
                    })
                    .to_string();
                    {
                        let rust = &mut *self;
                        if !rust.transfer_rows.iter().any(|row| row.job_id == job_id) {
                            rust.transfer_rows
                                .push(TransferRow::new(job_id, request.clone()));
                        }
                        let row = rust
                            .transfer_rows
                            .iter_mut()
                            .find(|row| row.job_id == job_id)
                            .expect("row just ensured");
                        row.running = true;
                        // One id for the whole transfer: the core registers a
                        // control under it so pause, cancel and the limit all
                        // reach the running operation.
                        row.operation_id = Some(operation_id);
                        row.started_at = now_millis();
                        row.label = if request.cut { "Moving…" } else { "Copying…" }.to_owned();
                        row.retry = retry;
                    }
                    let active = self.active_operations + 1;
                    self.active_operations = active;
                    self.set_busy(true);
                    self.publish_transfers();
                }
                JobEvent::Finished {
                    job_id,
                    error,
                    cancelled,
                    ..
                } => {
                    self.finish_transfer_row(
                        job_id,
                        error.map(|error| friendly_error(&error)),
                        cancelled,
                    );
                    self.publish_queue();
                }
                JobEvent::CheckFailed { job_id, cut, error } => {
                    // Nothing was copied and no busy count was taken: drop the
                    // row and report the failure the way a copy would have, but
                    // without `settle_operation` — decrementing there would clear
                    // the progress row of whatever else is running.
                    {
                        let rust = &mut *self;
                        if let Some(position) = rust
                            .transfer_rows
                            .iter()
                            .position(|row| row.job_id == job_id)
                        {
                            rust.transfer_rows.remove(position);
                        }
                    }
                    let message = friendly_error(&error);
                    self.forget_pending_conflict(job_id);
                    self.set_last_error(message.to_owned());
                    let kind_value = if cut { "move" } else { "copy" }.to_owned();
                    let message_value = message.to_owned();
                    self.operation_failed(kind_value, message_value);
                    self.publish_transfers();
                    self.publish_queue();
                }
            }
        }

        fn clear_conflict(&mut self) {
            self.set_conflict_job_id(0);
            self.set_conflict_waiting(0);
            self.set_conflict_index(0);
            self.set_conflict_total(0);
            self.set_conflict_name(String::new());
            self.set_conflict_cut(false);
            self.set_conflict_source_category(String::new());
            self.set_conflict_source_detail(String::new());
            self.set_conflict_target_category(String::new());
            self.set_conflict_target_detail(String::new());
        }

        /// Drops the prompt `job_id` is waiting on and repaints the dialog from
        /// the front of the list — the next job's question when one waits, an
        /// empty dialog when none does.
        fn forget_pending_conflict(&mut self, job_id: u64) {
            self.pending_conflicts
                .retain(|prompt| prompt.job_id != job_id);
            self.show_conflict_head();
        }

        /// Paints the dialog from the front of the pending list. A prompt that
        /// waited behind another job is already rendered; this only copies it
        /// onto the properties.
        fn show_conflict_head(&mut self) {
            let (head, waiting) = {
                let rust = &mut *self;
                let waiting = rust.pending_conflicts.len().saturating_sub(1) as i32;
                (rust.pending_conflicts.first().cloned(), waiting)
            };
            let Some(head) = head else {
                self.clear_conflict();
                return;
            };
            self.set_conflict_waiting(waiting);
            self.set_conflict_job_id(head.job_id as i64);
            self.set_conflict_index(head.index as i32 + 1);
            self.set_conflict_total(head.total as i32);
            self.set_conflict_name(head.name.to_owned());
            self.set_conflict_cut(head.cut);
            self.set_conflict_source_category(head.source_category.to_owned());
            self.set_conflict_source_detail(head.source_detail.to_owned());
            self.set_conflict_target_category(head.target_category.to_owned());
            self.set_conflict_target_detail(head.target_detail.to_owned());
        }

        /// Records the answer for the job's conflict on screen; the core moves
        /// to its next one or runs the batch. Its reply — the next prompt or
        /// `Started` — repaints the dialog, so nothing is removed here.
        #[qslot]
        fn resolve_conflict(
            &mut self,
            job_id: i64,
            action: String,
            apply_to_all: bool,
        ) {
            let Some(action) = conflict_action(&action.to_string()) else {
                return;
            };
            if job_id <= 0 {
                return;
            }
            self.scheduler()
                .answer(job_id as u64, action, apply_to_all);
        }

        /// Closes the question for that job and transfers nothing: the
        /// destination stays untouched, the job's row goes with it, and the
        /// queue advances. A prompt waiting behind moves onto the screen;
        /// running transfers are untouched — their cancel goes through the
        /// operation's control.
        #[qslot]
        fn cancel_conflict(&mut self, job_id: i64) {
            if job_id <= 0 {
                return;
            }
            let job_id = job_id as u64;
            {
                let rust = &mut *self;
                // A job still in its pre-run phase — checking or asking — can
                // be dismissed; a running row is not this command's to touch.
                rust.transfer_rows
                    .retain(|row| row.running || row.job_id != job_id);
                rust.pending_conflicts
                    .retain(|prompt| prompt.job_id != job_id);
            }
            self.scheduler().dismiss(job_id);
            self.show_conflict_head();
            self.publish_transfers();
            self.publish_queue();
        }

        #[qslot]

        fn start_drag_out(&self, paths: Vec<String>, mode: String,
                          signal: String, count: i32, verdict: String,
                          ink: String, canvas: String) {
            if paths.is_empty() {
                return;
            }

            // Qt::CopyAction | Qt::MoveAction | Qt::LinkAction are 1, 2 and 4.
            let actions = match mode.to_string().as_str() {
                "copy" => 1,
                "move" => 2,
                "link" => 4,
                _ => 1 | 2 | 4,
            };
            if !ffi::start_file_drag(
                &paths,
                actions,
                &signal,
                count.max(1),
                &verdict,
                &ink,
                &canvas,
            ) {
                log::warn!(target: "dae::workspace", "could not start the file drag");
            }
        }

        #[qslot]

        fn create_shortcuts(&mut self, paths: Vec<String>, destination: String) {
            self.create_shortcuts_paths(paths, destination.to_string());
        }

        fn create_shortcuts_paths(&mut self, paths: Vec<String>, destination: String) {
            if destination.is_empty() {
                return;
            }
            if !vfs::is_local_path(&destination) {
                self.finish_operation(
                    "shortcut",
                    "",
                    Some("Shortcuts can only be created in local folders".to_owned()),
                );
                return;
            }

            // Links are a local-only operation; a link to a remote path would
            // not resolve outside dae.
            let sources: Vec<String> = paths
                .into_iter()
                .filter(|path| vfs::is_local_path(path))
                .collect();
            if sources.is_empty() {
                self.finish_operation(
                    "shortcut",
                    "",
                    Some("Shortcuts can only be created for local files".to_owned()),
                );
                return;
            }

            self.begin_operation("Creating shortcut…");
            let thread = self.get_thread_handle();
            dispatch(thread, "shortcut", async move {
                match service::create_shortcuts(sources, destination).await {
                    Ok(created) => Outcome::ok(created.into_iter().next().unwrap_or_default()),
                    Err(error) => Outcome::failed(error),
                }
            });
        }

        #[qslot]

        fn local_drop_paths(&self, urls: Vec<String>) -> Vec<String> {
            ffi::local_drop_paths(&urls)
        }

        #[qslot]

        fn copy_text(&self, text: String) -> bool {
            if text.is_empty() {
                return false;
            }
            // Deliberately quiet: "复制路径" and the checksum rows word their own
            // confirmation, and a toast from here would say the wrong one.
            ffi::set_clipboard_text(&text);
            true
        }

        #[qslot]

        fn rename_entries_batch(&mut self, paths: Vec<String>, names: Vec<String>) {
            let paths = paths;
            let names = names;
            if paths.is_empty() || paths.len() != names.len() {
                return;
            }
            let requests = paths
                .into_iter()
                .zip(names)
                .map(|(path, new_name)| dae_core::types::RenameRequest { path, new_name })
                .collect::<Vec<_>>();

            self.begin_operation("Renaming…");
            let host = host_for(self.get_thread_handle());
            let thread = self.get_thread_handle();
            dispatch(thread, "rename", async move {
                match service::rename_entries_batch(requests, operation_id(), host).await {
                    Ok(renamed) => Outcome::ok(renamed.into_iter().next().unwrap_or_default()),
                    Err(error) => Outcome::failed(error),
                }
            });
        }

        #[qslot]

        fn show_native_properties(&mut self, path: String) -> bool {
            let path = path.to_string();
            // A network path has no shell object to show, and opening the OS sheet
            // on a `smb://` string would only produce a shell error dialog.
            if path.is_empty() || !vfs::is_local_path(&path) {
                return false;
            }
            platform::show_properties(&path)
        }

        #[qslot]

        fn warm_shell_commands(&mut self) {
            runtime::spawn(async {
                if let Err(error) = dae_core::fs::shell_commands::warm_shell_commands().await {
                    log::warn!(target: "dae::workspace", "shell-command warm-up failed: {error}");
                }
            });
        }

        #[qslot]

        fn invoke_shell_command(&mut self, id: String, paths: Vec<String>) {
            let id = id.to_string();
            let paths = paths;
            if id.is_empty() {
                return;
            }
            let host = host_for(self.get_thread_handle());
            let thread = self.get_thread_handle();
            // Not a tracked operation: the launched app owns the result from here.
            // Only a failure to start comes back to dae, and it goes through the
            // same error toast every other command uses.
            runtime::spawn(async move {
                let result = dae_core::fs::shell_commands::invoke_shell_command(host, id, paths).await;
                if let Err(error) = result {
                    thread.invoke_mut(move |workspace| {
                        let message = friendly_error(&error.to_string());
                        let kind = "shellCommand".to_owned();
                        let message_value = message.to_owned();
                        workspace
                            .set_last_error(message.to_owned());
                        workspace.operation_failed(kind, message_value);
                    });
                }
            });
        }

        #[qslot]

        fn set_clipboard(&mut self, mode: String, paths: Vec<String>) {
            let mode = mode.to_string();
            if paths.is_empty() {
                self.clear_clipboard();
                return;
            }
            let paths = paths;
            self.set_clipboard_state(&mode, &paths);
            self.system_clipboard_sequence = system_clipboard_sequence();

            // Only local paths mean anything to Explorer; remote entries stay in
            // the in-app mirror and are pasted by this shell alone.
            let local: Vec<String> = paths
                .into_iter()
                .filter(|path| vfs::is_local_path(path))
                .collect();
            self.system_clipboard_paths = local.clone();
            if local.is_empty() {
                return;
            }

            let effect = if mode == "cut" {
                ClipboardEffect::Move
            } else {
                ClipboardEffect::Copy
            };
            runtime::spawn(async move {
                if let Err(error) = service::write_files_to_clipboard(local, effect).await {
                    log::warn!(target: "dae::workspace", "could not place the selection on the system clipboard: {error}");
                }
            });
        }

        #[qslot]

        fn clear_clipboard(&mut self) {
            self.clipboard_mode = String::new();
            self.clipboard_paths = Vec::new();
            self.clipboard_generation += 1;
        }

        /// Writes the mirror without touching the system clipboard.
        fn set_clipboard_state(&mut self, mode: &str, paths: &[String]) {
            self.set_clipboard_mode(mode.to_owned());
            self.set_clipboard_paths(paths.to_vec());
            self.clipboard_generation += 1;
        }

        #[qslot]

        fn paste_into(&mut self, destination: String) {
            let destination = destination.to_string();
            let own = self.system_clipboard_paths.clone();
            let sequence = self.system_clipboard_sequence;
            let mirror = self.clipboard_paths.clone();
            let mirror_mode = self.clipboard_mode.to_string();
            let thread = self.get_thread_handle();
            runtime::spawn(async move {
                let (system, current) = system_clipboard_snapshot().await;
                let contents =
                    clipboard_contents(system, &own, &mirror, &mirror_mode, current == sequence);
                let _ = thread
                    .invoke_mut(move |workspace| workspace.apply_paste(contents, destination));
            });
        }

        fn apply_paste(&mut self, contents: ClipboardContents, destination: String) {
            match contents {
                ClipboardContents::Empty => {
                    self.finish_operation("copy", "", Some("Clipboard is empty".to_owned()))
                }
                ClipboardContents::Files {
                    paths,
                    effect: ClipboardEffect::Link,
                } => self.create_shortcuts_paths(paths, destination),
                ClipboardContents::Files { paths, effect } => {
                    let list = paths.to_vec();
                    self.transfer_entries(
                        list,
                        destination,
                        effect == ClipboardEffect::Move,
                    );
                }
            }
        }

        #[qslot]

        fn paste_shortcut_into(&mut self, destination: String) {
            let destination = destination.to_string();
            if destination.is_empty() {
                return;
            }
            let own = self.system_clipboard_paths.clone();
            let sequence = self.system_clipboard_sequence;
            let mirror = self.clipboard_paths.clone();
            let mirror_mode = self.clipboard_mode.to_string();
            let thread = self.get_thread_handle();
            runtime::spawn(async move {
                let (system, current) = system_clipboard_snapshot().await;
                let contents =
                    clipboard_contents(system, &own, &mirror, &mirror_mode, current == sequence);
                thread.invoke_mut(move |workspace| match contents {
                    ClipboardContents::Empty => workspace.finish_operation(
                        "shortcut",
                        "",
                        Some("Clipboard is empty".to_owned()),
                    ),
                    ClipboardContents::Files { paths, .. } => workspace
                        .create_shortcuts_paths(paths, destination),
                });
            });
        }

        #[qslot]

        fn refresh_clipboard(&mut self) {
            let generation = self.clipboard_generation;
            let own = self.system_clipboard_paths.clone();
            let thread = self.get_thread_handle();
            runtime::spawn(async move {
                let (system, sequence) = match service::read_files_from_clipboard().await {
                    Ok(system) => (system, system_clipboard_sequence()),
                    Err(_) => return,
                };
                thread.invoke_mut(move |workspace| {
                    // An in-app copy that happened after the read wins.
                    if workspace.clipboard_generation != generation {
                        return;
                    }
                    workspace
                        .apply_clipboard_refresh(system, sequence, &own);
                });
            });
        }

        fn apply_clipboard_refresh(
            &mut self,
            system: Option<SystemClipboardFiles>,
            sequence: u32,
            own: &[String],
        ) {
            match system {
                Some(files) if !files.paths.is_empty() => {
                    // Our own list may include remote paths the system clipboard
                    // never saw, so only an outside list replaces the mirror.
                    if files.paths != own {
                        let mode = match files.effect {
                            ClipboardEffect::Move => "cut",
                            ClipboardEffect::Link => "link",
                            ClipboardEffect::Copy => "copy",
                        };
                        self.set_clipboard_state(mode, &files.paths);
                        self.system_clipboard_sequence = sequence;
                    }
                }
                _ => {
                    if !own.is_empty() {
                        // The file list this shell placed was replaced by
                        // something else (text, another app's data).
                        self.clear_clipboard();
                        self.system_clipboard_paths.clear();
                        self.system_clipboard_sequence = sequence;
                    }
                }
            }
        }

        #[qslot]

        fn undo(&mut self) {
            self.begin_operation("Undoing…");
            let host = host_for(self.get_thread_handle());
            let thread = self.get_thread_handle();
            dispatch(thread, "undo", async move {
                match service::undo_operation(operation_id(), host).await {
                    Ok(_) => Outcome::empty(),
                    Err(error) => Outcome::failed(error),
                }
            });
        }

        #[qslot]

        fn redo(&mut self) {
            self.begin_operation("Redoing…");
            let host = host_for(self.get_thread_handle());
            let thread = self.get_thread_handle();
            dispatch(thread, "redo", async move {
                match service::redo_operation(operation_id(), host).await {
                    Ok(_) => Outcome::empty(),
                    Err(error) => Outcome::failed(error),
                }
            });
        }

        #[qslot]

        fn start_system_watch(&mut self) {
            crate::system_watch::start(self.get_thread_handle());
        }

        /// Applies an accent reading from the native watcher. `None` means the
        /// platform published no colour, and the current default is kept.
        pub(crate) fn apply_system_accent(&mut self, accent: Option<String>) {
            let Some(accent) = accent else {
                return;
            };
            if self.accent.to_string() == accent {
                return;
            }
            self.set_accent(accent.to_owned());
        }

        /// Applies a light/dark reading from the native watcher. `None` means the
        /// platform published no preference, and the current reading is kept.
        pub(crate) fn apply_system_theme(&mut self, dark: Option<bool>) {
            let Some(dark) = dark else {
                return;
            };
            if self.system_dark == dark {
                return;
            }
            self.set_system_dark(dark);
        }

        pub(crate) fn notify_volumes_changed(&mut self) {
            self.volumes_changed();
        }

        #[qslot]

        fn restore_trash_entries(&mut self, ids: Vec<String>) {
            let ids = ids;
            if ids.is_empty() {
                return;
            }
            self.begin_operation("Restoring…");
            let host = host_for(self.get_thread_handle());
            let thread = self.get_thread_handle();
            dispatch(thread, "restore_trash", async move {
                match trash_view::restore_trash_entries(ids, operation_id(), host).await {
                    Ok(_) => Outcome::empty(),
                    Err(error) => Outcome::failed(error),
                }
            });
        }

        #[qslot]

        fn delete_trash_entries(&mut self, ids: Vec<String>) {
            let ids = ids;
            if ids.is_empty() {
                return;
            }
            self.begin_operation("Deleting permanently…");
            let host = host_for(self.get_thread_handle());
            let thread = self.get_thread_handle();
            dispatch(thread, "purge_trash", async move {
                match trash_view::delete_trash_entries(ids, operation_id(), host).await {
                    Ok(_) => Outcome::empty(),
                    Err(error) => Outcome::failed(error),
                }
            });
        }

        #[qslot]

        fn empty_trash(&mut self) {
            self.begin_operation("Emptying Trash…");
            let host = host_for(self.get_thread_handle());
            let thread = self.get_thread_handle();
            dispatch(thread, "empty_trash", async move {
                match trash_view::empty_trash(operation_id(), host).await {
                    Ok(_) => Outcome::empty(),
                    Err(error) => Outcome::failed(error),
                }
            });
        }

        #[qslot]

        fn open_path(&mut self, path: String) {
            let path = path.to_string();
            if !vfs::is_local_path(&path) {
                self.finish_operation(
                    "open",
                    "",
                    Some(
                        "Copy remote files to a local folder before opening them with a local app"
                            .to_owned(),
                    ),
                );
                return;
            }
            if let Err(message) = platform::open_path(&path) {
                self.finish_operation("open", "", Some(message));
                return;
            }
            self.record_recent(String::from(path.as_str()), false, true);
        }

        #[qslot]

        fn open_with(&mut self, path: String) {
            let path = path.to_string();
            self.begin_operation("Opening…");
            let thread = self.get_thread_handle();
            dispatch(thread, "openWith", async move {
                match service::open_with(path).await {
                    Ok(()) => Outcome::empty(),
                    Err(error) => Outcome::failed(error),
                }
            });
        }

        #[qslot]

        fn open_with_app(
            &mut self,
            path: String,
            app_id: String,
            set_default: bool,
        ) {
            let path = path.to_string();
            let app_id = app_id.to_string();
            if path.trim().is_empty() || app_id.trim().is_empty() {
                self.finish_operation(
                    "openWith",
                    "",
                    Some("Select an application".to_owned()),
                );
                return;
            }

            self.begin_operation(if set_default {
                "Setting as default…"
            } else {
                "Opening…"
            });
            let thread = self.get_thread_handle();
            dispatch(thread, "openWith", async move {
                match open_with::open_with_app(path, app_id, set_default).await {
                    Ok(()) => Outcome::empty(),
                    Err(error) => Outcome::failed(error),
                }
            });
        }

        #[qslot]

        fn open_terminal(&mut self, path: String) {
            let path = path.to_string();
            if !vfs::is_local_path(&path) {
                self.finish_operation(
                    "terminal",
                    "",
                    Some("Only local directories can be opened in Terminal".to_owned()),
                );
                return;
            }
            match service::open_terminal(path) {
                Ok(()) => self.set_status("Opened in system terminal".to_owned()),
                Err(error) => {
                    let message = friendly_error(&error.to_string());
                    self.finish_operation("terminal", "", Some(message));
                }
            }
        }

        #[qslot]

        fn selection_size_text(&self, paths: Vec<String>) -> String {
            let mut total: u64 = 0;
            let mut counted = false;
            for path in paths.iter().map(|path| path.to_string()) {
                if !dae_core::fs::vfs::is_local_path(&path) {
                    continue;
                }
                let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                    continue;
                };
                if metadata.is_file() {
                    total = total.saturating_add(metadata.len());
                    counted = true;
                }
            }
            if counted {
                crate::format::size(total)
            } else {
                String::new()
            }
        }

        #[qslot]

        fn subdirectories(&self, directory: String) -> Vec<String> {
            let directory = directory.to_string();
            let mut names: Vec<String> = Vec::new();
            if !directory.is_empty() && dae_core::fs::vfs::is_local_path(&directory) {
                if let Ok(entries) = std::fs::read_dir(&directory) {
                    for entry in entries.flatten() {
                        // `file_type` avoids a second stat per entry on a network
                        // drive, which is the whole cost of this call.
                        let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
                        if !is_dir {
                            continue;
                        }
                        if let Ok(name) = entry.file_name().into_string() {
                            names.push(name);
                        }
                    }
                }
            }
            // The menu and the completion list both read in name order, so the
            // order is decided once on the way out.
            names.sort_by(|left, right| {
                left.to_lowercase()
                    .cmp(&right.to_lowercase())
                    .then(left.cmp(right))
            });
            let mut out = Vec::new();
            for name in names {
                out.push(name);
            }
            out
        }

        #[qslot]

        fn recent_directories(&self) -> Vec<String> {
            let mut seen = std::collections::HashSet::new();
            let mut out = Vec::new();
            for recent in dae_core::fs::recents::list_recents().unwrap_or_default() {
                if recent.kind != dae_core::types::EntryKind::Directory {
                    continue;
                }
                if seen.insert(recent.path.clone()) {
                    out.push(recent.path);
                }
                if out.len() >= 32 {
                    break;
                }
            }
            out
        }

        #[qslot]

        fn expand_typed_path(&self, text: String) -> String {
            Self::expand_shorthand(&text)
        }

    /// Expands the typed-address shorthands. A leading `~` (alone or before a
    /// separator) becomes the user's home; `%VAR%` and `$VAR`/`${VAR}` become
    /// environment values. Anything that does not resolve stays as typed — the
    /// navigation then reports the path in its own words, which is what
    /// "keep what I typed" means.
    pub(super) fn expand_shorthand(text: &str) -> String {
        let expanded = if let Some(rest) = text.strip_prefix('~') {
            if rest.is_empty() || rest.starts_with(['/', '\\']) {
                std::env::var_os("USERPROFILE")
                    .or_else(|| std::env::var_os("HOME"))
                    .map(|home| {
                        let home = home.to_string_lossy().into_owned();
                        if rest.is_empty() { home } else { format!("{home}{rest}") }
                    })
                    .unwrap_or_else(|| text.to_owned())
            } else {
                text.to_owned()
            }
        } else {
            text.to_owned()
        };
        Self::expand_percent(&Self::expand_dollar(&expanded))
    }

    /// `%VAR%` to the environment value, char-wise so multibyte text survives
    /// untouched. An unresolvable pair is left exactly as typed.
    fn expand_percent(text: &str) -> String {
        let chars: Vec<char> = text.chars().collect();
        let mut out = String::with_capacity(text.len());
        let mut index = 0;
        while index < chars.len() {
            if chars[index] == '%' && index + 1 < chars.len()
                && let Some(close) = chars[index + 1..].iter().position(|item| *item == '%')
            {
                let name: String = chars[index + 1..index + 1 + close].iter().collect();
                if !name.is_empty()
                    && name.chars().all(|item| item.is_ascii_alphanumeric() || item == '_')
                    && let Some(value) = std::env::var_os(&name)
                {
                    out.push_str(&value.to_string_lossy());
                    index += close + 2;
                    continue;
                }
            }
            out.push(chars[index]);
            index += 1;
        }
        out
    }

    /// `$VAR` and `${VAR}` to the environment value. A `$` that names nothing
    /// stays — POSIX paths legally carry one.
    fn expand_dollar(text: &str) -> String {
        let chars: Vec<char> = text.chars().collect();
        let mut out = String::with_capacity(text.len());
        let mut index = 0;
        while index < chars.len() {
            if chars[index] == '$' && index + 1 < chars.len() {
                if chars[index + 1] == '{' {
                    if let Some(close) = chars[index + 2..].iter().position(|item| *item == '}') {
                        let name: String = chars[index + 2..index + 2 + close].iter().collect();
                        if let Some(value) = std::env::var_os(&name) {
                            out.push_str(&value.to_string_lossy());
                            index += close + 3;
                            continue;
                        }
                    }
                } else {
                    let mut end = index + 1;
                    while end < chars.len()
                        && (chars[end].is_ascii_alphanumeric() || chars[end] == '_')
                    {
                        end += 1;
                    }
                    if end > index + 1 {
                        let name: String = chars[index + 1..end].iter().collect();
                        if let Some(value) = std::env::var_os(&name) {
                            out.push_str(&value.to_string_lossy());
                            index = end;
                            continue;
                        }
                    }
                }
            }
            out.push(chars[index]);
            index += 1;
        }
        out
    }

        #[qslot]

        fn reveal(&mut self, path: String) {
            let path = path.to_string();
            if !vfs::is_local_path(&path) {
                self.finish_operation(
                    "reveal",
                    "",
                    Some("Remote items cannot be revealed in the system file manager".to_owned()),
                );
                return;
            }
            if let Err(message) = platform::reveal_path(&path) {
                self.finish_operation("reveal", "", Some(message));
            }
        }

        #[qslot]

        fn record_recent(&mut self, path: String, is_directory: bool, opened: bool) {
            let path = path.to_string();
            if path.is_empty() {
                return;
            }
            let kind = if is_directory {
                dae_core::types::EntryKind::Directory
            } else {
                dae_core::types::EntryKind::File
            };
            let source = if opened {
                RecentSource::Opened
            } else {
                RecentSource::Visited
            };
            runtime::spawn(async move {
                let _ = dae_core::fs::recents::record_recent(path, kind, source).await;
                // The taskbar's frequent group follows the folders the user browses,
                // not the files they open. The request comes after the visit is on
                // disk, or the rebuild would read a list without the folder that
                // triggered it.
                if is_directory {
                    crate::jump_list::refresh();
                }
            });
        }

        #[qslot]

        fn add_favorite(&mut self, path: String) {
            let path = path.to_string();
            if path.is_empty() {
                return;
            }
            let mut favorites = sidebar::load_favorites().unwrap_or_default();
            if favorites.iter().any(|favorite| favorite.path == path) {
                return;
            }
            favorites.push(Favorite {
                name: display_name_from_path(&path),
                path: path.clone(),
            });
            match sidebar::save_favorites(favorites) {
                Ok(()) => {
                    self.set_status("Added to favorites".to_owned());
                    self.favorites_changed();
                    // A folder the user favorited belongs in the taskbar menu, even
                    // if it was deleted from it earlier.
                    crate::jump_list::reinclude(&path);
                    crate::jump_list::refresh();
                }
                Err(error) => {
                    let message = friendly_error(&error.to_string());
                    self.set_last_error(message.to_owned());
                }
            }
        }

        #[qslot]

        fn add_favorite_paths(&mut self, paths: Vec<String>) {
            let mut favorites = sidebar::load_favorites().unwrap_or_default();
            let mut added: Vec<String> = Vec::new();
            for path in paths.iter() {
                let path = path.to_string();
                if path.is_empty() || !std::path::Path::new(&path).is_dir() {
                    continue;
                }
                if favorites.iter().any(|favorite| favorite.path == path) {
                    continue;
                }
                favorites.push(Favorite {
                    name: display_name_from_path(&path),
                    path: path.clone(),
                });
                added.push(path);
            }
            if added.is_empty() {
                return;
            }
            match sidebar::save_favorites(favorites) {
                Ok(()) => {
                    self.set_status("Added to favorites".to_owned());
                    self.favorites_changed();
                    for path in &added {
                        crate::jump_list::reinclude(path);
                    }
                    crate::jump_list::refresh();
                }
                Err(error) => {
                    let message = friendly_error(&error.to_string());
                    self.set_last_error(message.to_owned());
                }
            }
        }

        #[qslot]

        fn hide_place(&mut self, kind: String) {
            let Some(place) = parse_place_kind(&kind.to_string()) else {
                return;
            };
            let mut hidden = sidebar::load_hidden_places().unwrap_or_default();
            if hidden.contains(&place) {
                return;
            }
            hidden.push(place);
            match sidebar::save_hidden_places(hidden) {
                Ok(()) => {
                    self.set_status("Hidden from sidebar".to_owned());
                    self.places_changed();
                }
                Err(error) => {
                    let message = friendly_error(&error.to_string());
                    self.set_last_error(message.to_owned());
                }
            }
        }

        #[qslot]

        fn show_all_places(&mut self) {
            match sidebar::save_hidden_places(Vec::new()) {
                Ok(()) => {
                    self.set_status("Restored hidden places".to_owned());
                    self.places_changed();
                }
                Err(error) => {
                    let message = friendly_error(&error.to_string());
                    self.set_last_error(message.to_owned());
                }
            }
        }

        #[qslot]

        fn remove_favorite(&mut self, path: String) {
            let path = path.to_string();
            let favorites: Vec<Favorite> = sidebar::load_favorites()
                .unwrap_or_default()
                .into_iter()
                .filter(|favorite| favorite.path != path)
                .collect();
            match sidebar::save_favorites(favorites) {
                Ok(()) => {
                    self.set_status("Removed from favorites".to_owned());
                    self.favorites_changed();
                    crate::jump_list::refresh();
                }
                Err(error) => {
                    let message = friendly_error(&error.to_string());
                    self.set_last_error(message.to_owned());
                }
            }
        }

        #[qslot]

        fn is_favorite(&self, path: String) -> bool {
            let target = path.to_string();
            if target.is_empty() {
                return false;
            }
            sidebar::load_favorites()
                .unwrap_or_default()
                .iter()
                .any(|favorite| favorite.path.eq_ignore_ascii_case(&target) || favorite.path == target)
        }

        #[qslot]

        fn toggle_favorite(&mut self, path: String) {
            if self.is_favorite(path.clone()) {
                self.remove_favorite(path);
            } else {
                self.add_favorite(path);
            }
        }

        #[qslot]

        fn test_connection(
            &mut self,
            protocol: String,
            host: String,
            port: String,
            username: String,
            password: String,
        ) {
            let input = match connection_input(&protocol, &host, &port, &username, &password, false) {
                Ok(input) => input,
                Err(message) => {
                    self.connection_test_finished(false, String::from(message.as_str()));
                    return;
                }
            };

            let thread = self.get_thread_handle();
            runtime::spawn(async move {
                let result = smb::test_connection(input).await;
                thread.invoke_mut(move |workspace| match result {
                    Ok(()) => workspace
                        .connection_test_finished(true, String::from("Connection test succeeded")),
                    Err(error) => {
                        let message = friendly_error(&error.to_string());
                        workspace
                            .set_last_error(message.to_owned());
                        workspace
                            .connection_test_finished(false, String::from(message.as_str()));
                    }
                });
            });
        }

        #[qslot]

        fn save_connection(
            &mut self,
            protocol: String,
            host: String,
            port: String,
            username: String,
            password: String,
            remember_password: bool,
        ) {
            let input =
                match connection_input(&protocol, &host, &port, &username, &password, remember_password) {
                    Ok(input) => input,
                    Err(message) => {
                        self.connection_save_finished(
                            false,
                            String::default(),
                            String::from(message.as_str()),
                        );
                        return;
                    }
                };

            let thread = self.get_thread_handle();
            runtime::spawn(async move {
                let result = tokio::task::spawn_blocking(move || connections::save_connection(input))
                    .await
                    .map_err(|error| FileSystemError::Internal(error.to_string()))
                    .and_then(|result| result);
                thread.invoke_mut(move |workspace| match result {
                    Ok(connection) => {
                        workspace.connections_changed();
                        workspace.connection_save_finished(
                            true,
                            String::from(connection.id.as_str()),
                            String::default(),
                        );
                    }
                    Err(error) => {
                        let message = friendly_error(&error.to_string());
                        workspace
                            .set_last_error(message.to_owned());
                        workspace.connection_save_finished(
                            false,
                            String::default(),
                            String::from(message.as_str()),
                        );
                    }
                });
            });
        }

        #[qslot]

        fn delete_connection(&mut self, id: String) {
            let id = id.to_string();
            if id.trim().is_empty() {
                return;
            }
            match connections::delete_connection(id) {
                Ok(()) => {
                    self.set_status("Connection removed".to_owned());
                    self.connections_changed();
                    let kind = "connection_delete".to_owned();
                    self.operation_finished(kind, String::default());
                }
                Err(error) => {
                    let message = friendly_error(&error.to_string());
                    self.set_last_error(message.to_owned());
                    let kind = "connection_delete".to_owned();
                    self.operation_failed(kind, message);
                }
            }
        }

        #[qslot]

        fn authorize_cloud_account(
            &mut self,
            provider: String,
            client_id: String,
            client_secret: String,
        ) {
            let provider = match CloudProviderKind::parse(provider.to_string().trim()) {
                Ok(provider) => provider,
                Err(error) => {
                    let message = friendly_error(&error.to_string());
                    self.cloud_authorize_finished(false, String::from(message.as_str()));
                    return;
                }
            };
            let client_id = client_id.to_string().trim().to_owned();
            if client_id.is_empty() {
                self.cloud_authorize_finished(false, String::from("Enter a Client ID"));
                return;
            }

            let input = AuthorizeCloudAccountInput {
                provider,
                client_id,
                client_secret: optional_string(&client_secret),
            };
            let thread = self.get_thread_handle();
            runtime::spawn(async move {
                let result = cloud_oauth::authorize_cloud_account(input).await;
                thread.invoke_mut(move |workspace| match result {
                    Ok(account) => {
                        workspace.connections_changed();
                        workspace
                            .cloud_authorize_finished(true, String::from(account.id.as_str()));
                    }
                    Err(error) => {
                        let message = friendly_error(&error.to_string());
                        workspace
                            .set_last_error(message.to_owned());
                        workspace
                            .cloud_authorize_finished(false, String::from(message.as_str()));
                    }
                });
            });
        }

        #[qslot]

        fn delete_cloud_account(&mut self, id: String) {
            let id = id.to_string();
            if id.trim().is_empty() {
                return;
            }
            match cloud_accounts::delete_cloud_account(id) {
                Ok(()) => {
                    self.set_status("Cloud account removed".to_owned());
                    self.connections_changed();
                    let kind = "cloud_delete".to_owned();
                    self.operation_finished(kind, String::default());
                }
                Err(error) => {
                    let message = friendly_error(&error.to_string());
                    self.set_last_error(message.to_owned());
                    let kind = "cloud_delete".to_owned();
                    self.operation_failed(kind, message);
                }
            }
        }

        #[qslot]

        fn save_terminal_settings(
            &mut self,
            font_size: i32,
            line_height: f64,
            family: String,
        ) {
            let mut settings = dae_core::settings::load_settings().unwrap_or_default();
            settings.terminal.font_size = font_size.clamp(8, 24) as u8;
            settings.terminal.line_height = (line_height as f32).clamp(0.8, 3.0);
            let family = family.to_string();
            settings.terminal.font_family = (!family.trim().is_empty()).then_some(family);

            match dae_core::settings::save_settings(settings) {
                Ok(saved) => {
                    self.set_terminal_font_size(saved.terminal.font_size as i32);
                    self.set_terminal_line_height(saved.terminal.line_height as f64);
                    self.set_terminal_font_family(
                        saved.terminal.font_family.unwrap_or_default(),
                    );
                    self.set_status("Terminal settings saved".to_owned());
                }
                Err(error) => {
                    let message = friendly_error(&error.to_string());
                    self.set_last_error(message.to_owned());
                }
            }
        }

        #[qslot]

        fn refresh_default_manager(&mut self) {
            self.start_default_manager(None);
        }

        #[qslot]

        fn set_shortcut(&mut self, id: String, binding: String) {
            let id = id.to_string();
            if !dae_core::settings::default_shortcuts().contains_key(&id) {
                self.set_last_error("Unknown shortcut action".to_owned());
                return;
            }
            let binding = binding.to_string().trim().to_owned();
            if binding.len() > 64 || binding.chars().any(char::is_control) {
                self.set_last_error("Invalid shortcut format".to_owned());
                return;
            }
            self.apply_shortcut_change(move |shortcuts| {
                shortcuts.insert(id, binding);
            });
        }

        #[qslot]

        fn reset_shortcut(&mut self, id: String) {
            let id = id.to_string();
            let Some(binding) = dae_core::settings::default_shortcuts().get(&id).cloned() else {
                self.set_last_error("Unknown shortcut action".to_owned());
                return;
            };
            self.apply_shortcut_change(move |shortcuts| {
                shortcuts.insert(id, binding);
            });
        }

        #[qslot]

        fn reset_shortcuts(&mut self) {
            self.apply_shortcut_change(|shortcuts| {
                *shortcuts = dae_core::settings::default_shortcuts();
            });
        }

        /// Saves only the shortcut map. Loading first preserves the terminal
        /// typography and the cached file-manager flag that share the document.
        fn apply_shortcut_change(
            &mut self,
            change: impl FnOnce(&mut HashMap<String, String>),
        ) {
            let mut settings = dae_core::settings::load_settings().unwrap_or_default();
            change(&mut settings.shortcuts);
            match dae_core::settings::save_settings(settings) {
                Ok(saved) => {
                    self.set_shortcuts_json(shortcuts_json(&saved.shortcuts));
                    self.set_status("Shortcuts saved".to_owned());
                    self.set_last_error(String::new());
                }
                Err(error) => {
                    let message = friendly_error(&error.to_string());
                    self.set_last_error(message.to_owned());
                }
            }
        }

        #[qslot]

        fn open_log_directory(&mut self) {
            let directory = crate::logging::app_log_directory();
            if let Err(error) = std::fs::create_dir_all(&directory) {
                self.set_last_error(
                    format!("Could not create log directory: {error}"),
                );
                return;
            }
            match platform::open_path(&directory.to_string_lossy()) {
                Ok(()) => self.set_status("Opened log directory".to_owned()),
                Err(message) => self.set_last_error(message.to_owned()),
            }
        }

        #[qslot]

        fn set_default_file_manager(&mut self) {
            self.start_default_manager(Some(true));
        }

        #[qslot]

        fn unset_default_file_manager(&mut self) {
            self.start_default_manager(Some(false));
        }

        /// `None` re-reads the OS state; `Some(true/false)` registers or removes
        /// dae first. The blocking platform calls run off the Qt thread, and the
        /// busy flag drops overlapping requests the same way the Git operations do.
        fn start_default_manager(&mut self, change: Option<bool>) {
            if self.default_manager_busy {
                return;
            }
            self.set_default_manager_busy(true);
            self.set_default_manager_error(String::new());
            let thread = self.get_thread_handle();
            runtime::spawn(async move {
                let result = tokio::task::spawn_blocking(move || match change {
                    Some(true) => default_manager::set(),
                    Some(false) => default_manager::unset(),
                    None => default_manager::status(),
                })
                .await
                .map_err(|error| {
                    FileSystemError::Internal(format!("default_manager.status_failed: {error}"))
                })
                .and_then(|result| result);
                thread.invoke_mut(move |workspace| {
                    workspace.adopt_default_manager(change, result)
                });
            });
        }

        fn adopt_default_manager(
            &mut self,
            change: Option<bool>,
            result: Result<DefaultManagerStatus, FileSystemError>,
        ) {
            self.set_default_manager_busy(false);
            match result {
                Ok(status) => {
                    self.set_default_manager_supported(status.supported);
                    self.set_default_manager_is_default(status.is_default);
                    self.set_default_manager_is_registered(status.is_registered);
                    let detail = status
                        .detail
                        .as_deref()
                        .map(default_manager_detail)
                        .unwrap_or("");
                    self.set_default_manager_detail(detail.to_owned());
                    self.set_default_manager_error(String::new());
                    if change.is_some() {
                        let _ = dae_core::settings::set_default_file_manager_cache(status.is_default);
                    }
                }
                Err(error) => {
                    self.set_default_manager_detail(String::new());
                    let message = friendly_error(&error.to_string());
                    self.set_default_manager_error(message.to_owned());
                }
            }
        }
    }
}

pub use workspace_impl::Workspace;

impl QmlElement for Workspace {
    const URI: &str = "dae";
    const ELEMENT_NAME: &str = "Workspace";
    const MINOR_VERSION: u8 = 0;
    const MAJOR_VERSION: u8 = 1;
    const IS_SINGLETON: bool = true;
}

/// Turns core events into queued Qt updates.
struct QtHost {
    thread: QThreadHandle<Workspace>,
}

impl Host for QtHost {
    fn emit(&self, event: Event) {
        self.thread.invoke_mut(move |workspace| workspace.apply_event(event));
    }
}

fn host_for(thread: QThreadHandle<Workspace>) -> Arc<dyn Host> {
    Arc::new(QtHost { thread })
}

/// The outcome an async command reports back on the Qt thread.
/// One accepted transfer's presentation state. The rules live in the core's
/// job scheduler; this is what the panel's row reads and what the status
/// center's record is written from when the job reports back.
struct TransferRow {
    job_id: u64,
    /// Minted by the scheduler at `Started`; `None` while the job is still
    /// checking or answering conflicts. Pause and cancel address the core
    /// by it.
    operation_id: Option<String>,
    request: TransferRequest,
    running: bool,
    done: u64,
    total: u64,
    /// Display name of the file on the wire, for the row's title.
    current: String,
    paused: bool,
    speed: String,
    /// Last progress sample for this row's throughput readout.
    speed_sample: Option<(Instant, u64)>,
    /// Smoothed bytes per second. Smoothed because a 60 ms event on a network
    /// share reports wildly different deltas one tick to the next.
    speed_bps: f64,
    started_at: u64,
    label: String,
    retry: String,
}

/// One asking job's conflict prompt, frozen until the dialog reaches it.
/// The payload is rendered once, on arrival, so a prompt that waited
/// behind another job repaints byte-identically when its turn comes.
#[derive(Clone)]
struct PendingPrompt {
    job_id: u64,
    /// 0-based within the job's own batch; the dialog prints `index + 1`.
    index: usize,
    total: usize,
    cut: bool,
    name: String,
    source_category: String,
    source_detail: String,
    target_category: String,
    target_detail: String,
}

impl TransferRow {
    fn new(job_id: u64, request: TransferRequest) -> Self {
        let label = if request.cut { "Moving…" } else { "Copying…" }.to_owned();
        Self {
            job_id,
            operation_id: None,
            request,
            running: false,
            done: 0,
            total: 0,
            current: String::new(),
            paused: false,
            speed: String::new(),
            speed_sample: None,
            speed_bps: 0.0,
            started_at: now_millis(),
            label,
            retry: String::new(),
        }
    }

    /// Turns the byte deltas between progress events into a throughput figure.
    ///
    /// Nothing here reads a clock outside the events themselves: the reporter
    /// already throttles to one event per 60 ms, so the interval between two
    /// events is the measurement window. Smoothed with a 0.6/0.4 split because
    /// a single SMB round trip can halve one interval and double the next, and
    /// a readout that oscillates is worse than one that lags.
    fn sample_speed(&mut self, completed: u64) {
        let now = Instant::now();
        let previous = self.speed_sample.replace((now, completed));
        let Some((last_time, last_done)) = previous else {
            return;
        };
        let elapsed = now.duration_since(last_time).as_secs_f64();
        if elapsed < 0.05 || completed <= last_done {
            return;
        }
        let instant = (completed - last_done) as f64 / elapsed;
        self.speed_bps = if self.speed_bps <= 0.0 {
            instant
        } else {
            self.speed_bps * 0.6 + instant * 0.4
        };
        self.speed = format_speed(self.speed_bps);
    }
}

struct Outcome {
    focus: String,
    error: Option<String>,
    /// True when the user stopped it. A cancellation is not a failure: it gets
    /// a neutral status-center row and a "continue" action, not a diagnosis.
    cancelled: bool,
}

impl Outcome {
    fn ok(focus: impl Into<String>) -> Self {
        Self {
            focus: focus.into(),
            error: None,
            cancelled: false,
        }
    }

    fn empty() -> Self {
        Self::ok(String::new())
    }

    fn failed(error: FileSystemError) -> Self {
        Self {
            focus: String::new(),
            error: Some(friendly_error(&error.to_string())),
            cancelled: matches!(error, FileSystemError::Cancelled(_)),
        }
    }
}

/// Runs one core future and reports on the Qt thread.
fn dispatch<F>(thread: QThreadHandle<Workspace>, kind: &'static str, future: F)
where
    F: std::future::Future<Output = Outcome> + Send + 'static,
{
    runtime::spawn(async move {
        let outcome = future.await;
        thread.invoke_mut(move |workspace| {
            if outcome.cancelled {
                workspace.finish_cancelled_operation(kind);
            } else {
                workspace.finish_operation(
                    kind,
                    outcome.focus.as_str(),
                    outcome.error,
                );
            }
        });
    });
}

/// The system clipboard's files (`None` when it holds no file list) and the
/// change counter read alongside them.
async fn system_clipboard_snapshot() -> (Option<SystemClipboardFiles>, u32) {
    let files = service::read_files_from_clipboard().await.ok().flatten();
    (files, system_clipboard_sequence())
}

/// Where a paste's files come from, once the system clipboard and the in-app
/// mirror have been reconciled.
enum ClipboardContents {
    Empty,
    Files {
        paths: Vec<String>,
        effect: ClipboardEffect,
    },
}

/// The system clipboard wins when it holds a file list other than the one this
/// shell placed there and another app has changed it since; otherwise the
/// in-app mirror keeps remote entries (and failed system writes) alive.
fn clipboard_contents(
    system: Option<SystemClipboardFiles>,
    own: &[String],
    mirror: &[String],
    mirror_mode: &str,
    system_unchanged: bool,
) -> ClipboardContents {
    if let Some(files) = system {
        if !files.paths.is_empty() && files.paths != own && !system_unchanged {
            return ClipboardContents::Files {
                paths: files.paths,
                effect: files.effect,
            };
        }
    }
    if mirror.is_empty() {
        return ClipboardContents::Empty;
    }
    let effect = match mirror_mode {
        "cut" => ClipboardEffect::Move,
        "link" => ClipboardEffect::Link,
        _ => ClipboardEffect::Copy,
    };
    ClipboardContents::Files {
        paths: mirror.to_vec(),
        effect,
    }
}

/// QML passes empty strings where the bridge means `None`.
fn optional_string(value: &str) -> Option<String> {
    let value = value.to_string();
    (!value.is_empty()).then_some(value)
}

/// Validates the typed fields and turns them into the core's save contract.
/// Both Test and Connect go through here so the two paths cannot disagree
/// about what a blank port or a padded host means.
fn connection_input(
    protocol: &str,
    host: &str,
    port: &str,
    username: &str,
    password: &str,
    remember_password: bool,
) -> Result<SaveConnectionInput, String> {
    let protocol = protocol_from(&protocol.to_string())?;
    let host = host.to_string().trim().to_owned();
    if host.is_empty() {
        return Err("Enter a server address".into());
    }
    let username = username.to_string().trim().to_owned();

    Ok(SaveConnectionInput {
        protocol,
        host,
        port: parse_port(&port.to_string())?,
        username: (!username.is_empty()).then_some(username),
        password: optional_string(password),
        remember_password,
    })
}

fn protocol_from(value: &str) -> Result<Protocol, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "smb" => Ok(Protocol::Smb),
        "sftp" => Ok(Protocol::Sftp),
        "ftp" => Ok(Protocol::Ftp),
        "webdav" => Ok(Protocol::WebDav),
        other => Err(format!("Unsupported connection protocol: {other}")),
    }
}

fn parse_port(value: &str) -> Result<Option<u16>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    match value.parse::<u16>() {
        Ok(port) if port > 0 => Ok(Some(port)),
        _ => Err("Port must be between 1 and 65535".into()),
    }
}

fn archive_format(value: &str) -> Option<ArchiveFormat> {
    match value {
        "zip" => Some(ArchiveFormat::Zip),
        "tar" => Some(ArchiveFormat::Tar),
        "tar.gz" => Some(ArchiveFormat::TarGz),
        "7z" => Some(ArchiveFormat::SevenZip),
        _ => None,
    }
}

/// The QML side's `"replace" | "skip" | "keepBoth"` → the core's enum. An
/// unknown value is ignored rather than mapped to a default: the dialog is the
/// only caller, and a typo there must not silently turn into "replace".
fn conflict_action(value: &str) -> Option<ConflictAction> {
    match value {
        "replace" => Some(ConflictAction::Replace),
        "skip" => Some(ConflictAction::Skip),
        "keepBoth" => Some(ConflictAction::KeepBoth),
        _ => None,
    }
}

/// A place row carries its icon name; the menu sends that back when hiding.
fn parse_place_kind(value: &str) -> Option<PlaceKind> {
    match value {
        "home" => Some(PlaceKind::Home),
        "desktop" => Some(PlaceKind::Desktop),
        "documents" => Some(PlaceKind::Documents),
        "downloads" => Some(PlaceKind::Downloads),
        "pictures" => Some(PlaceKind::Pictures),
        "music" => Some(PlaceKind::Music),
        "videos" => Some(PlaceKind::Videos),
        _ => None,
    }
}

/// The glyph set's category name, so a conflict card draws the same icon the
/// listing would.
fn glyph_category(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Directory => "folder",
        EntryKind::Symlink => "link",
        EntryKind::Other => "other",
        EntryKind::File => "file",
    }
}

/// The type word the listing's 类型 column shows for this entry.
fn kind_label(kind: EntryKind, name: &str) -> String {
    match kind {
        EntryKind::Directory => "Folder".to_owned(),
        EntryKind::Symlink => "Symlink".to_owned(),
        EntryKind::Other => "Other".to_owned(),
        EntryKind::File => {
            let extension = name
                .rsplit_once('.')
                .map(|(_, extension)| extension)
                .unwrap_or("");
            if extension.is_empty() {
                "File".to_owned()
            } else {
                extension.to_uppercase()
            }
        }
    }
}

/// A side card's two caption lines: `kind · size`, then the date. Both sides of
/// a conflict use the same two lines, so they are one string rather than five
/// properties the dialog would have to glue back together.
fn conflict_detail(
    kind: EntryKind,
    name: &str,
    size: Option<u64>,
    modified_at: Option<u64>,
) -> String {
    let size = size
        .map(crate::format::size)
        .unwrap_or_else(|| "—".to_owned());
    let modified = modified_at
        .map(crate::format::timestamp)
        .unwrap_or_else(|| "—".to_owned());
    format!(
        "{} · {}\nModified {}",
        kind_label(kind, name),
        size,
        modified
    )
}

/// The status center's file: the last operations this shell ran, newest last
/// in memory, newest first in the document.
const OPERATIONS_FILE_NAME: &str = "operations-qt.json";
/// A session can issue thousands of operations; the panel only has to answer
/// "what happened lately", so older entries fall off the front.
const OPERATION_HISTORY_LIMIT: usize = 200;
/// What survives a restart. Smaller than the in-memory ring on purpose: the
/// bad case is a session that failed a hundred transfers, and a hundred rows
/// after a relaunch are noise rather than history.
const OPERATION_PERSISTED_LIMIT: usize = 50;

/// Upper bound on a hand-edited `[transfer] speedLimitKib`. 10 GiB/s is past
/// any link a desktop has, so a larger number is a typo rather than an intent,
/// and clamping it keeps the value inside `i32` on the QML side.
const MAX_SPEED_LIMIT_KIB: u64 = 10 * 1024 * 1024;

/// One line of the status center.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OperationRecord {
    id: String,
    kind: String,
    label: String,
    started: u64,
    finished: u64,
    /// `ok`, `failed` or `cancelled`; a running operation is only ever on the
    /// progress row.
    status: String,
    detail: String,
    retryable: bool,
    /// The call arguments a retry needs, as JSON. Empty when not retryable.
    retry: String,
    /// Whether "retry with administrator rights" applies to this row.
    elevatable: bool,
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

fn operations_path() -> Option<std::path::PathBuf> {
    dae_core::paths::config_dir()
        .ok()
        .map(|dir| dir.join(OPERATIONS_FILE_NAME))
}

fn load_operations() -> Vec<OperationRecord> {
    let Some(path) = operations_path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut records: Vec<OperationRecord> = serde_json::from_str(&text).unwrap_or_default();
    // Older builds restarted the counter every launch, so a loaded row could
    // share its id with a newer one and the newer silently answered both.
    // Suffix the collisions in place: the panel and `retry_operation` read the
    // same list, so the row that was clicked is the row that is retried.
    let mut seen = std::collections::HashSet::new();
    for (index, record) in records.iter_mut().enumerate() {
        if !seen.insert(record.id.clone()) {
            record.id = format!("{}-{index}", record.id);
        }
    }
    // The file is oldest first; keep the tail, not the head.
    let excess = records.len().saturating_sub(OPERATION_HISTORY_LIMIT);
    records.drain(0..excess);
    records
}

fn persist_operations(records: &[OperationRecord]) {
    let Some(path) = operations_path() else {
        return;
    };
    let start = records.len().saturating_sub(OPERATION_PERSISTED_LIMIT);
    let Ok(contents) = serde_json::to_vec_pretty(&records[start..]) else {
        return;
    };
    if let Err(error) = dae_core::fs::atomic::write_atomic(&path, &contents) {
        log::warn!(target: "dae::workspace", "could not save the operation log: {error}");
    }
}

/// Newest first, as the panel draws it. Written on every change rather than
/// read through a model: the list is at most 200 rows and the panel is closed
/// most of the time.
fn operations_json(records: &[OperationRecord]) -> String {
    let mut sorted: Vec<&OperationRecord> = records.iter().collect();
    sorted.sort_by_key(|record| std::cmp::Reverse(record.finished));
    serde_json::to_string(&sorted).unwrap_or_else(|_| "[]".to_owned())
}

fn external_archive_formats_json() -> String {
    serde_json::to_string(&archive::external_formats()).unwrap_or_else(|_| "[]".to_owned())
}

fn drive_capabilities_json() -> String {
    let capabilities = drives::capabilities();
    serde_json::json!({
        "eject": capabilities.eject,
        "unmount": capabilities.unmount,
        "mount": capabilities.mount,
        "format": capabilities.format,
        "cleanup": capabilities.cleanup,
        "encryption": capabilities.encryption,
        "previousVersions": capabilities.previous_versions,
    })
    .to_string()
}

/// The menu-ready action list: id and name only, because the menu shows
/// nothing else and the command may be long.
fn custom_actions_json() -> String {
    let entries: Vec<serde_json::Value> = actions::available_actions()
        .into_iter()
        .map(|action| serde_json::json!({"id": action.id, "name": action.name}))
        .collect();
    serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_owned())
}

/// One argument per line; blank lines are dropped rather than passed as empty
/// arguments, which almost every program reads as a missing value.
fn action_args(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

fn operation_id() -> String {
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicU64, Ordering};
    // The counter alone restarts at 1 on every launch, and the status center
    // loads yesterday's rows: `retry_operation` then found the oldest row with
    // today's id and did nothing. The launch time keeps ids unique across
    // sessions; the counter keeps them unique inside one.
    static LAUNCH: OnceLock<u64> = OnceLock::new();
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let launch = *LAUNCH.get_or_init(now_millis);
    format!("qt-op-{launch}-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

/// The line the queue panel prints for a waiting job.
fn transfer_label(request: &TransferRequest) -> String {
    format!(
        "{} {} {} → {}",
        if request.cut { "Move" } else { "Copy" },
        request.sources.len(),
        if request.sources.len() == 1 { "item" } else { "items" },
        display_name_from_path(&request.destination)
    )
}

/// `12.4 MB/s`, `812 KB/s`, `940 B/s`.
///
/// One decimal above a megabyte and none below: a transfer rate is read for its
/// order of magnitude ("is this 12 or 120?"), and the second digit on a KB/s
/// figure is noise the reporter's 60 ms window cannot justify.
fn format_speed(bytes_per_second: f64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;

    if bytes_per_second >= GIB {
        format!("{:.1} GB/s", bytes_per_second / GIB)
    } else if bytes_per_second >= MIB {
        format!("{:.1} MB/s", bytes_per_second / MIB)
    } else if bytes_per_second >= KIB {
        format!("{:.0} KB/s", bytes_per_second / KIB)
    } else {
        format!("{:.0} B/s", bytes_per_second.max(0.0))
    }
}

/// Remaining seconds for a running row, `None` when the readout cannot answer
/// yet: no smoothed rate (the first sample only seeds the next one), a total
/// the job never learned, or nothing left to copy.
fn transfer_eta(done: u64, total: u64, speed_bps: f64) -> Option<f64> {
    if speed_bps <= 0.0 {
        return None;
    }
    let remaining = total.checked_sub(done)?;
    if remaining == 0 {
        return None;
    }
    Some(remaining as f64 / speed_bps)
}

/// `1:42`, `12:07`, `1:04:22`. A clock rather than a sentence, so the readout
/// never needs a translator — same reason `format_speed` prints raw units.
fn format_eta(seconds: f64) -> String {
    if !seconds.is_finite() || seconds < 0.0 {
        return String::new();
    }
    let whole = (seconds + 0.5).min(359_999.0) as u64;
    let hours = whole / 3600;
    let minutes = (whole % 3600) / 60;
    let secs = whole % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes}:{secs:02}")
    }
}

fn success_message(kind: &str) -> &'static str {
    match kind {
        "create" => "Created",
        "rename" => "Renamed",
        "trash" => "Moved to Trash",
        "delete" => "Permanently deleted",
        "duplicate" => "Duplicated",
        "shortcut" => "Shortcut created",
        "copy" => "Copied",
        "move" => "Moved",
        "compress" => "Compressed",
        "extract" => "Extracted",
        "undo" => "Undone",
        "redo" => "Redone",
        "openWith" => "Opened",
        "compare" => "Compared",
        "restore_trash" => "Restored",
        "purge_trash" => "Permanently deleted",
        "empty_trash" => "Trash emptied",
        _ => "Done",
    }
}

fn operation_label(operation: dae_core::progress::FileOperationKind) -> &'static str {
    use dae_core::progress::FileOperationKind;
    match operation {
        FileOperationKind::Copy => "Copying…",
        FileOperationKind::Move => "Moving…",
        FileOperationKind::Delete => "Deleting…",
        FileOperationKind::Compress => "Compressing…",
        FileOperationKind::Extract => "Extracting…",
        FileOperationKind::Properties => "Calculating…",
    }
}

/// Serializes the bindings for QML. Ids are sorted so the text is stable, and
/// empty bindings are kept: they tell the panel that an action is deliberately
/// unbound rather than missing from the document.
fn shortcuts_json(shortcuts: &HashMap<String, String>) -> String {
    let mut ids: Vec<&str> = shortcuts.keys().map(String::as_str).collect();
    ids.sort_unstable();

    let mut json = String::from("{");
    for (index, id) in ids.into_iter().enumerate() {
        if index > 0 {
            json.push(',');
        }
        push_json_string(&mut json, id);
        json.push(':');
        push_json_string(
            &mut json,
            shortcuts.get(id).map(String::as_str).unwrap_or(""),
        );
    }
    json.push('}');
    json
}

fn push_json_string(out: &mut String, value: &str) {
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            character if character.is_control() => {
                out.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => out.push(character),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_paths_expand_home_and_environment_shorthands() {
        use crate::workspace::Workspace;

        // The name is unique to this test; nothing else reads it while
        // tests run in parallel.
        unsafe { std::env::set_var("DAE_QT_TEST_VAR", "K:/expanded") };
        assert_eq!(
            Workspace::expand_shorthand("%DAE_QT_TEST_VAR%\\sub"),
            "K:/expanded\\sub"
        );
        assert_eq!(
            Workspace::expand_shorthand("start $DAE_QT_TEST_VAR"),
            "start K:/expanded"
        );
        assert_eq!(
            Workspace::expand_shorthand("${DAE_QT_TEST_VAR}/x"),
            "K:/expanded/x"
        );
        // Unresolvable shorthands stay exactly as typed.
        assert_eq!(
            Workspace::expand_shorthand("%DAE_QT_MISSING%"),
            "%DAE_QT_MISSING%"
        );
        assert_eq!(Workspace::expand_shorthand("$DAE_QT_MISSING"), "$DAE_QT_MISSING");
        assert_eq!(Workspace::expand_shorthand("50% done"), "50% done");
        assert_eq!(Workspace::expand_shorthand("~notes"), "~notes");
        // `~` alone or before a separator is the home.
        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .expect("a test machine has a home");
        let home = home.to_string_lossy().into_owned();
        assert_eq!(Workspace::expand_shorthand("~"), home);
        assert_eq!(Workspace::expand_shorthand("~\\sub"), format!("{home}\\sub"));
    }

    /// The readout is scanned, not read: one decimal above a megabyte, none
    /// below, and never a unit the number does not need.
    #[test]
    fn a_transfer_rate_prints_in_the_unit_that_scans_best() {
        assert_eq!(format_speed(0.0), "0 B/s");
        assert_eq!(format_speed(940.0), "940 B/s");
        assert_eq!(format_speed(812.0 * 1024.0), "812 KB/s");
        assert_eq!(format_speed(12.44 * 1024.0 * 1024.0), "12.4 MB/s");
        assert_eq!(format_speed(1.5 * 1024.0 * 1024.0 * 1024.0), "1.5 GB/s");
        // A negative sample can only come from a clock that moved backwards;
        // it must not print "-0 B/s".
        assert_eq!(format_speed(-12.0), "0 B/s");
    }

    /// TeraCopy's other readout: the estimate rides the smoothed rate, so it
    /// answers only once two progress events have been seen, and it never
    /// divides by a job that has not started.
    #[test]
    fn an_eta_answers_only_from_two_samples() {
        assert_eq!(transfer_eta(10, 100, 0.0), None, "no rate yet");
        assert_eq!(transfer_eta(100, 100, 2.0), None, "nothing left");
        assert_eq!(transfer_eta(10, 0, 2.0), None, "no total learned");
        // 90 bytes left at 3 B/s is half a minute away.
        assert_eq!(transfer_eta(10, 100, 3.0), Some(30.0));
    }

    #[test]
    fn an_eta_prints_as_a_clock() {
        assert_eq!(format_eta(42.4), "0:42");
        assert_eq!(format_eta(727.0), "12:07");
        assert_eq!(format_eta(3762.0), "1:02:42");
        // A clock that moved backwards, or a rate that went negative, prints
        // nothing rather than a negative time.
        assert_eq!(format_eta(f64::NAN), "");
        assert_eq!(format_eta(-1.0), "");
        // Past a hundred hours the estimate is noise; it saturates the
        // clock rather than growing without bound.
        assert_eq!(format_eta(400_000.0), "99:59:59");
    }

    /// A transfer conflict for `path`, with both sides looking like plain
    /// files: the decision rules do not read the metadata.
    #[test]
    fn paste_prefers_files_another_app_copied() {
        let external = SystemClipboardFiles {
            paths: vec!["C:/x".to_string()],
            effect: ClipboardEffect::Move,
        };
        match clipboard_contents(
            Some(external),
            &["C:/own".to_string()],
            &["C:/own".to_string()],
            "copy",
            false,
        ) {
            ClipboardContents::Files { paths, effect } => {
                assert_eq!(paths, vec!["C:/x".to_string()]);
                assert_eq!(effect, ClipboardEffect::Move);
            }
            ClipboardContents::Empty => panic!("expected the system files"),
        }
    }

    #[test]
    fn paste_keeps_the_mirror_when_the_system_list_is_ours() {
        let own = vec!["C:/local".to_string()];
        let mirror = vec!["C:/local".to_string(), "smb://host/share/file".to_string()];
        let system = SystemClipboardFiles {
            paths: own.clone(),
            effect: ClipboardEffect::Copy,
        };
        match clipboard_contents(Some(system), &own, &mirror, "copy", false) {
            ClipboardContents::Files { paths, effect } => {
                assert_eq!(paths, mirror);
                assert_eq!(effect, ClipboardEffect::Copy);
            }
            ClipboardContents::Empty => panic!("expected the mirrored list"),
        }
    }

    #[test]
    fn paste_without_any_clipboard_is_empty() {
        assert!(matches!(
            clipboard_contents(None, &[], &[], "", true),
            ClipboardContents::Empty
        ));
    }

    #[test]
    fn an_unchanged_system_list_does_not_override_a_newer_internal_copy() {
        // Copying remote files cannot reach the system clipboard, so it still
        // holds whatever was there before even though the user copied since.
        let stale = SystemClipboardFiles {
            paths: vec!["C:/older".to_string()],
            effect: ClipboardEffect::Copy,
        };
        match clipboard_contents(
            Some(stale),
            &[],
            &["smb://host/share/file".to_string()],
            "copy",
            true,
        ) {
            ClipboardContents::Files { paths, .. } => {
                assert_eq!(paths, vec!["smb://host/share/file".to_string()]);
            }
            ClipboardContents::Empty => panic!("expected the internal list"),
        }
    }

    #[test]
    fn a_queued_transfer_names_its_operation_and_target() {
        let copy = TransferRequest {
            sources: vec!["C:/a".into(), "C:/b".into()],
            destination: "C:/dst".into(),
            cut: false,
            resume: false,
            spends_clipboard: false,
        };
        assert_eq!(transfer_label(&copy), "Copy 2 items → dst");
        let cut = TransferRequest {
            sources: vec!["C:/a".into()],
            cut: true,
            ..copy
        };
        assert_eq!(transfer_label(&cut), "Move 1 item → dst");
    }

    #[test]
    fn the_dialog_actions_are_the_three_the_core_understands() {
        assert_eq!(conflict_action("replace"), Some(ConflictAction::Replace));
        assert_eq!(conflict_action("skip"), Some(ConflictAction::Skip));
        assert_eq!(conflict_action("keepBoth"), Some(ConflictAction::KeepBoth));
        // A string the dialog cannot produce must not quietly become a policy:
        // a typo that turned into `replace` would delete the user's file.
        assert_eq!(conflict_action(""), None);
        assert_eq!(conflict_action("keep_both"), None);
    }

    #[test]
    fn the_conflict_card_names_the_kind_the_listing_would() {
        assert_eq!(kind_label(EntryKind::Directory, "docs"), "Folder");
        assert_eq!(kind_label(EntryKind::File, "notes.txt"), "TXT");
        assert_eq!(kind_label(EntryKind::File, "README"), "File");
    }

    #[test]
    fn a_conflict_card_carries_both_caption_lines() {
        let detail = conflict_detail(
            EntryKind::File,
            "a.bin",
            Some(2048),
            Some(1_792_886_400_000),
        );
        assert_eq!(detail, "BIN · 2.0 KiB\nModified 2026-10-25 00:00");

        // A folder has no size and a remote entry may have no timestamp; both
        // read as an em dash rather than as a zero.
        let detail = conflict_detail(EntryKind::Directory, "docs", None, None);
        assert_eq!(detail, "Folder · —\nModified —");
    }

    #[test]
    fn qml_archive_ids_map_to_core_formats() {
        assert_eq!(archive_format("zip"), Some(ArchiveFormat::Zip));
        assert_eq!(archive_format("tar"), Some(ArchiveFormat::Tar));
        assert_eq!(archive_format("tar.gz"), Some(ArchiveFormat::TarGz));
        assert_eq!(archive_format("7z"), Some(ArchiveFormat::SevenZip));
        assert_eq!(archive_format("rar"), None);
    }

    #[test]
    fn connection_fields_are_trimmed_and_blank_extras_dropped() {
        let input = connection_input(
            "SMB",
            "  nas.local ",
            "",
            " alice ",
            "",
            true,
        )
        .expect("valid connection input");

        assert_eq!(input.protocol, Protocol::Smb);
        assert_eq!(input.host, "nas.local");
        assert_eq!(input.port, None);
        assert_eq!(input.username.as_deref(), Some("alice"));
        assert_eq!(input.password, None);
        assert!(input.remember_password);

        let sftp = connection_input(
            "sftp",
            "nas.local",
            "2222",
            "",
            "secret",
            false,
        )
        .expect("valid sftp input");
        assert_eq!(sftp.protocol, Protocol::Sftp);
        assert_eq!(sftp.port, Some(2222));
        assert_eq!(sftp.password.as_deref(), Some("secret"));
    }

    #[test]
    fn connection_validation_reports_the_offending_field() {
        let missing_host = connection_input(
            "smb",
            "   ",
            "",
            "",
            "",
            false,
        )
        .expect_err("host is required");
        assert_eq!(missing_host, "Enter a server address");

        let bad_port = connection_input(
            "smb",
            "nas.local",
            "70000",
            "",
            "",
            false,
        )
        .expect_err("port is out of range");
        assert_eq!(bad_port, "Port must be between 1 and 65535");

        let unknown = connection_input(
            "s3",
            "bucket",
            "",
            "",
            "",
            false,
        )
        .expect_err("unknown protocol");
        assert!(unknown.starts_with("Unsupported connection protocol"));
    }

    #[test]
    fn shortcut_json_is_sorted_and_keeps_empty_bindings() {
        let mut shortcuts = HashMap::new();
        shortcuts.insert("explorer.copy".to_string(), String::new());
        shortcuts.insert("app.openSettings".to_string(), "Mod+,".to_string());
        shortcuts.insert("a.key".to_string(), "Mod+\"x".to_string());

        // Sorted so the QML text is stable, escaped so a recorded key cannot
        // break the JSON, and empty is kept rather than treated as missing.
        assert_eq!(
            shortcuts_json(&shortcuts),
            r#"{"a.key":"Mod+\"x","app.openSettings":"Mod+,","explorer.copy":""}"#
        );
    }

    #[test]
    fn the_default_shortcut_document_round_trips_through_the_qml_json() {
        let defaults = dae_core::settings::default_shortcuts();
        let json = shortcuts_json(&defaults);
        assert!(json.starts_with('{') && json.ends_with('}'));
        for id in defaults.keys() {
            assert!(
                json.contains(&format!("\"{id}\":")),
                "missing {id} in {json}"
            );
        }
        assert!(json.contains("\"app.pathJump\":\"Mod+L\""));
    }
}
