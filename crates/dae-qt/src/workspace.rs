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

use core::pin::Pin;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QColor, QString, QStringList, QUrl};
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

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;

        include!("cxx-qt-lib/qcolor.h");
        type QColor = cxx_qt_lib::QColor;

        include!("drag.h");
        #[namespace = "dae_qt"]
        #[rust_name = "start_file_drag"]
        fn dae_qt_start_file_drag(paths: &QStringList, actions: i32,
                                  signal: &QColor, count: i32,
                                  verdict: &QString, ink: &QColor,
                                  canvas: &QColor) -> bool;

        include!("clipboard.h");
        #[namespace = "dae_qt"]
        #[rust_name = "set_clipboard_text"]
        fn dae_qt_set_clipboard_text(text: &QString);

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

    unsafe extern "RustQt" {
        /// Process-wide file commands, reachable from QML as `Workspace`.
        ///
        /// `status` is a finished sentence for the toast; `lastError` is the
        /// same for a failure. `busy` and the progress properties are the
        /// lowest common denominator for long operations: the shell has one
        /// toast and one progress row, not one per transfer.
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        /// `rar`/`iso` when an external tool (7z, unrar, bsdtar) is installed;
        /// the extract row is gated on it. As JSON, e.g. `["rar","iso"]`.
        #[qproperty(QString, external_archive_formats, cxx_name = "externalArchiveFormats")]
        /// What the drive menu may offer on this platform, as JSON
        /// (`{eject, unmount, mount, format, cleanup, encryption,
        /// previousVersions}`). The menu hides the rows the platform cannot
        /// honour instead of showing ones that fail.
        #[qproperty(QString, drive_capabilities, cxx_name = "driveCapabilities")]
        /// The external actions that apply right now: platform, program on
        /// PATH, and — when the shell asks with a selection — the action's
        /// file/directory/multiple rule. Entries are `{id, name}`.
        #[qproperty(QString, custom_actions_json, cxx_name = "customActionsJson")]
        /// The status center's rows as JSON, newest first.
        #[qproperty(QString, operations_json, cxx_name = "operationsJson")]
        /// Failures the panel has not shown since they happened. The title bar
        /// badge reads it; opening the panel clears it.
        #[qproperty(i32, failure_count, cxx_name = "failureCount")]
        #[qproperty(QString, status)]
        #[qproperty(QString, last_error, cxx_name = "lastError")]
        #[qproperty(bool, busy)]
        #[qproperty(bool, can_undo, cxx_name = "canUndo")]
        #[qproperty(bool, can_redo, cxx_name = "canRedo")]
        #[qproperty(QString, progress_label, cxx_name = "progressLabel")]
        #[qproperty(i32, progress_done, cxx_name = "progressDone")]
        #[qproperty(i32, progress_total, cxx_name = "progressTotal")]
        /// What `progressDone` counts, as the core's code for the operation:
        /// `delete` and `properties` raise it by entries, `compress` and
        /// `extract` by bytes, and a move by either — bytes when it copies
        /// across volumes, entries when it only re-links. The strip prints a
        /// count where the unit is certain and a percentage everywhere else,
        /// so it has to know which it is looking at. Empty until an operation
        /// reports.
        #[qproperty(QString, progress_kind, cxx_name = "progressKind")]
        /// The conflict the dialog is showing, one at a time — the Tauri shell's
        /// dialog walked its list the same way, with one "apply to all" escape
        /// hatch, so QML never needs the whole list at once. `conflictIndex` is
        /// 1-based and 0 when no conflict is pending, which is also what keeps
        /// the dialog hidden.
        #[qproperty(i32, conflict_index, cxx_name = "conflictIndex")]
        #[qproperty(i32, conflict_total, cxx_name = "conflictTotal")]
        #[qproperty(QString, conflict_name, cxx_name = "conflictName")]
        /// True when the transfer behind the conflict is a move, so the dialog
        /// can say "move" where it would otherwise say "copy".
        #[qproperty(bool, conflict_cut, cxx_name = "conflictCut")]
        /// `folder`, `file`, `link` or `other` — the glyph set's names, so the
        /// dialog paints the same icon the listing does.
        #[qproperty(QString, conflict_source_category, cxx_name = "conflictSourceCategory")]
        /// The two caption lines of a side card (`kind · size` and `修改于 …`),
        /// formatted here because the model side already owns that vocabulary.
        #[qproperty(QString, conflict_source_detail, cxx_name = "conflictSourceDetail")]
        #[qproperty(QString, conflict_target_category, cxx_name = "conflictTargetCategory")]
        #[qproperty(QString, conflict_target_detail, cxx_name = "conflictTargetDetail")]
        /// The job the dialog is asking about, `0` when no dialog is up.
        /// Several jobs can be asking at once — the prompts queue, the front
        /// one is on screen — so answers and dismissals carry this id instead
        /// of assuming the one pending job.
        #[qproperty(i64, conflict_job_id, cxx_name = "conflictJobId")]
        /// How many further jobs are waiting for their own conflict answer
        /// behind the one on screen. The dialog prints it so a second
        /// prompt arriving is not a surprise.
        #[qproperty(i32, conflict_waiting, cxx_name = "conflictWaiting")]
        /// Transfers waiting for a free slot, one finished display line each
        /// (`Copy 3 items → dae`). Accepted jobs — checking, answering
        /// conflicts or running — are not in this list; they live in
        /// `transfersJson`.
        #[qproperty(QStringList, queued_transfers, cxx_name = "queuedTransfers")]
        /// Where each queued job is going, in the same order as
        /// `queuedTransfers`. The panel prints it under the name: the row's
        /// title says what is moving, this says where to.
        #[qproperty(QStringList, queued_transfer_paths, cxx_name = "queuedTransferPaths")]
        /// The accepted transfers as a JSON array, one object per row:
        /// `{jobId, operationId, name, target, running, paused, done, total,
        /// percent, speed, current}`. Several run at once, so the panel reads
        /// this list rather than a single progress row; `jobId` addresses a
        /// row's pause/cancel, `operationId` is empty until the job runs.
        /// A JSON string because cxx-qt cannot export a `QVariantList`
        /// property (the same seam `operationsJson` uses).
        #[qproperty(QString, transfers_json, cxx_name = "transfersJson")]
        /// The process-wide transfer limit in KiB per second; `0` is
        /// unlimited. Persisted under `[transfer] speedLimitKib`.
        #[qproperty(i32, transfer_speed_limit, cxx_name = "transferSpeedLimit")]
        /// The running transfers' combined throughput, already formatted
        /// (`"12.4 MB/s"`). Empty when nothing is moving.
        #[qproperty(QString, transfer_speed, cxx_name = "transferSpeed")]
        #[qproperty(QString, clipboard_mode, cxx_name = "clipboardMode")]
        #[qproperty(QStringList, clipboard_paths, cxx_name = "clipboardPaths")]
        #[qproperty(QString, accent)]
        /// The OS light/dark preference, kept current by the native watcher.
        /// Every window binds its `dark` property to this instead of reading
        /// the preference once at startup.
        #[qproperty(bool, system_dark, cxx_name = "systemDark")]
        #[qproperty(QString, home_directory, cxx_name = "homeDirectory")]
        #[qproperty(i32, terminal_font_size, cxx_name = "terminalFontSize")]
        #[qproperty(f64, terminal_line_height, cxx_name = "terminalLineHeight")]
        #[qproperty(QString, terminal_font_family, cxx_name = "terminalFontFamily")]
        /// The editable key bindings as a JSON object (`{"app.openSettings":
        /// "Mod+,", …}`). QML parses it once per change and turns each value
        /// into a `Shortcut.sequence`; an empty value means "unbound".
        #[qproperty(QString, shortcuts_json, cxx_name = "shortcutsJson")]
        /// The directory the log page opens. Shared with the Tauri shell's
        /// `appLogDir` convention so a user has one place to look.
        #[qproperty(QString, log_directory, cxx_name = "logDirectory")]
        /// Whether the platform's filename index answered on this machine.
        /// "所有位置" is a full walk without it, which the search bar says
        /// out loud rather than letting the wait look like a hang.
        #[qproperty(bool, search_index_available, cxx_name = "searchIndexAvailable")]
        /// True where the in-app picker is the only way to choose an
        /// application (macOS and Linux). Windows keeps the native
        /// `SHOpenWithDialog`, which the `openWith` command invokes.
        #[qproperty(bool, open_with_picker_supported, cxx_name = "openWithPickerSupported")]
        /// Live default-file-manager state for the settings panel. The panel
        /// reads fresh OS state on open and after every set/unset; `detail`
        /// names the manual step a platform still requires.
        #[qproperty(bool, default_manager_supported, cxx_name = "defaultManagerSupported")]
        #[qproperty(bool, default_manager_is_default, cxx_name = "defaultManagerIsDefault")]
        #[qproperty(
            bool,
            default_manager_is_registered,
            cxx_name = "defaultManagerIsRegistered"
        )]
        #[qproperty(QString, default_manager_detail, cxx_name = "defaultManagerDetail")]
        #[qproperty(QString, default_manager_error, cxx_name = "defaultManagerError")]
        #[qproperty(bool, default_manager_busy, cxx_name = "defaultManagerBusy")]
        type Workspace = super::WorkspaceRust;

        #[qinvokable]
        #[cxx_name = "createFolder"]
        fn create_folder(self: Pin<&mut Workspace>, directory: &QString, name: &QString);

        #[qinvokable]
        #[cxx_name = "createFile"]
        fn create_file(self: Pin<&mut Workspace>, directory: &QString, name: &QString);

        /// Instantiates the file or folder at `template` as `directory/name`.
        /// Only direct children of the templates folder are accepted; the core
        /// re-checks that rather than trusting the menu row.
        #[qinvokable]
        #[cxx_name = "createFromTemplate"]
        fn create_from_template(
            self: Pin<&mut Workspace>,
            directory: &QString,
            template_path: &QString,
            name: &QString,
        );

        /// The templates menu's rows as JSON (`[{path,name,isDir}]`).
        #[qinvokable]
        #[cxx_name = "templatesJson"]
        fn templates_json(&self) -> QString;

        /// Opens the templates folder in the OS, creating it if needed.
        #[qinvokable]
        #[cxx_name = "openTemplatesDirectory"]
        fn open_templates_directory(self: Pin<&mut Workspace>);

        /// Persists the folder the templates menu reads. `lastError` carries a
        /// failure; a successful call clears it.
        #[qinvokable]
        #[cxx_name = "setTemplatesDirectory"]
        fn set_templates_directory(self: Pin<&mut Workspace>, path: &QString);

        /// Asks which processes hold any of `paths` (directories are walked).
        /// The answer arrives on `fileHoldersFound`, tagged with the request so
        /// a dialog can ignore a stale result.
        #[qinvokable]
        #[cxx_name = "fileHolders"]
        fn file_holders(self: Pin<&mut Workspace>, paths: &QStringList);

        /// Ends one process a holder row named. The outcome arrives on
        /// `processEnded`; dae refuses to end itself and pid 0.
        #[qinvokable]
        #[cxx_name = "endProcess"]
        fn end_process(self: Pin<&mut Workspace>, pid: i32);

        #[qinvokable]
        #[cxx_name = "renameEntry"]
        fn rename_entry(self: Pin<&mut Workspace>, path: &QString, new_name: &QString);

        #[qinvokable]
        #[cxx_name = "trashEntries"]
        fn trash_entries(self: Pin<&mut Workspace>, paths: &QStringList);

        #[qinvokable]
        #[cxx_name = "deleteEntries"]
        fn delete_entries(self: Pin<&mut Workspace>, paths: &QStringList);

        #[qinvokable]
        #[cxx_name = "duplicateEntries"]
        fn duplicate_entries(self: Pin<&mut Workspace>, paths: &QStringList);

        /// Compresses `sources` into a new archive inside `destination_dir`.
        /// `format` is `"zip"`, `"tar"`, `"tar.gz"` or `"7z"`; an empty
        /// `password` means no encryption.
        #[qinvokable]
        #[cxx_name = "compressEntries"]
        fn compress_entries(
            self: Pin<&mut Workspace>,
            sources: &QStringList,
            destination_dir: &QString,
            format: &QString,
            password: &QString,
        );

        /// Extracts a local archive next to itself when `destination_dir` is
        /// empty. A protected archive answers with `archivePasswordRequired`.
        #[qinvokable]
        #[cxx_name = "extractArchive"]
        fn extract_archive(
            self: Pin<&mut Workspace>,
            archive_path: &QString,
            destination_dir: &QString,
            password: &QString,
        );

        #[qinvokable]
        #[cxx_name = "ejectDrive"]
        fn eject_drive(self: Pin<&mut Workspace>, path: &QString);

        #[qinvokable]
        #[cxx_name = "unmountDrive"]
        fn unmount_drive(self: Pin<&mut Workspace>, path: &QString);

        #[qinvokable]
        #[cxx_name = "mountDrive"]
        fn mount_drive(self: Pin<&mut Workspace>, path: &QString);

        /// Opens the platform's format dialog for `path`. Never formats here.
        #[qinvokable]
        #[cxx_name = "formatDrive"]
        fn format_drive(self: Pin<&mut Workspace>, path: &QString);

        /// Opens the platform's storage cleanup settings.
        #[qinvokable]
        #[cxx_name = "openStorageCleanup"]
        fn open_storage_cleanup(self: Pin<&mut Workspace>);

        /// A read-only encryption sentence, or the tool's failure. Empty
        /// `path` asks about the system volume.
        #[qinvokable]
        #[cxx_name = "driveEncryptionStatus"]
        fn drive_encryption_status(self: Pin<&mut Workspace>, path: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "openPreviousVersions"]
        fn open_previous_versions(self: Pin<&mut Workspace>, path: &QString);

        /// The actions that belong on the menu for this selection, as JSON.
        #[qinvokable]
        #[cxx_name = "customActionsFor"]
        fn custom_actions_for(
            self: Pin<&mut Workspace>,
            paths: &QStringList,
            directory: &QString,
        ) -> QString;

        /// Every stored action with its command and arguments, for the
        /// settings page. Unlike `customActionsFor` this list is not filtered:
        /// an action whose program is missing still has to be editable.
        #[qinvokable]
        #[cxx_name = "customActionDefinitions"]
        fn custom_action_definitions(&self) -> QString;

        /// Runs one `[[actions]]` entry without a shell. The failure path is
        /// the same `operationFailed` every other command uses, so the toast
        /// and the status center need no second channel.
        #[qinvokable]
        #[cxx_name = "runCustomAction"]
        fn run_custom_action(
            self: Pin<&mut Workspace>,
            id: &QString,
            paths: &QStringList,
            directory: &QString,
        );

        /// Whether this platform has a system share sheet. The context menu
        /// hides the entry when false instead of opening a sheet that cannot
        /// appear.
        #[qinvokable]
        #[cxx_name = "shareSupported"]
        fn share_supported(self: Pin<&mut Workspace>) -> bool;

        /// Opens the system share sheet for `paths` on the window `handle`.
        /// False means the sheet could not be shown; the caller reports it.
        #[qinvokable]
        #[cxx_name = "sharePaths"]
        fn share_paths(
            self: Pin<&mut Workspace>,
            paths: &QStringList,
            handle: f64,
        ) -> bool;

        /// Creates or updates an action. `id` empty creates one; `args` is one
        /// argument per line, which keeps a path with spaces a single argument.
        #[qinvokable]
        #[cxx_name = "saveCustomAction"]
        fn save_custom_action(
            self: Pin<&mut Workspace>,
            id: &QString,
            name: &QString,
            command: &QString,
            args: &QString,
            selection: &QString,
        );

        #[qinvokable]
        #[cxx_name = "removeCustomAction"]
        fn remove_custom_action(self: Pin<&mut Workspace>, id: &QString);

        /// Drops every user action and restores the built-in "open in IDE"
        /// entry (offered only while `code` is on PATH).
        #[qinvokable]
        #[cxx_name = "resetCustomActions"]
        fn reset_custom_actions(self: Pin<&mut Workspace>);

        /// Marks every failure as seen. Opening the panel calls this; the rows
        /// themselves stay until `clearOperationHistory`.
        #[qinvokable]
        #[cxx_name = "markOperationsRead"]
        fn mark_operations_read(self: Pin<&mut Workspace>);

        /// Drops every recorded operation, including the persisted half.
        #[qinvokable]
        #[cxx_name = "clearOperationHistory"]
        fn clear_operation_history(self: Pin<&mut Workspace>);

        /// Re-runs a failed transfer or extraction from the arguments it was
        /// recorded with. Other kinds cannot be retried: a delete that removed
        /// a file has nothing left to retry against.
        #[qinvokable]
        #[cxx_name = "retryOperation"]
        fn retry_operation(self: Pin<&mut Workspace>, id: &QString);

        /// Compares two local files by SHA-256, one read each, and answers
        /// with `compareFinished`. The dialog's checksum tab compares against a
        /// typed value instead; this covers "is this copy the same file".
        #[qinvokable]
        #[cxx_name = "compareHashes"]
        fn compare_hashes(self: Pin<&mut Workspace>, paths: &QStringList);

        /// Sums the folders in `paths` and answers with `folderSizesMeasured`.
        /// Enumeration never walks a directory, so this is the only path that
        /// turns a folder's em dash into a number. A second call cancels the
        /// first. `cancelMeasureFolders` stops the running pass.
        #[qinvokable]
        #[cxx_name = "measureFolders"]
        fn measure_folders(self: Pin<&mut Workspace>, paths: &QStringList);

        #[qinvokable]
        #[cxx_name = "cancelMeasureFolders"]
        fn cancel_measure_folders(self: Pin<&mut Workspace>);

        /// Copies or moves `paths` into `destination`. `cut` selects move.
        #[qinvokable]
        #[cxx_name = "transferEntries"]
        fn transfer_entries(
            self: Pin<&mut Workspace>,
            paths: &QStringList,
            destination: &QString,
            cut: bool,
        );

        /// Answers the conflict the dialog is showing for `jobId` (the
        /// `conflictJobId` on screen). `action` is `"replace"`, `"skip"` or
        /// `"keepBoth"`; `applyToAll` answers every conflict left in that
        /// batch the same way. Other jobs' prompts are untouched.
        #[qinvokable]
        #[cxx_name = "resolveConflict"]
        fn resolve_conflict(
            self: Pin<&mut Workspace>,
            job_id: i64,
            action: &QString,
            apply_to_all: bool,
        );

        /// Dismisses the question for `jobId` — the dialog's scrim, its Esc,
        /// or the row's ✕ — and transfers nothing of that job: not the
        /// item, not the rest of the batch. A prompt waiting behind moves
        /// onto the screen.
        #[qinvokable]
        #[cxx_name = "cancelConflict"]
        fn cancel_conflict(self: Pin<&mut Workspace>, job_id: i64);

        /// Drops the transfer waiting at `index` of `queuedTransfers`.
        #[qinvokable]
        #[cxx_name = "cancelQueuedTransfer"]
        fn cancel_queued_transfer(self: Pin<&mut Workspace>, index: i32);

        /// Holds the transfer named by `jobId` (a `transfersJson` row) at its
        /// next checkpoint. The bytes already written stay; nothing is undone.
        #[qinvokable]
        #[cxx_name = "pauseTransfer"]
        fn pause_transfer(self: Pin<&mut Workspace>, job_id: i64);

        /// Lets a paused transfer continue from where it stopped.
        #[qinvokable]
        #[cxx_name = "resumeTransfer"]
        fn resume_transfer(self: Pin<&mut Workspace>, job_id: i64);

        /// Stops the transfer named by `jobId`. The file it was writing is
        /// removed and the entries already copied stay; the status center then
        /// offers to continue, which skips exactly those entries.
        #[qinvokable]
        #[cxx_name = "cancelTransfer"]
        fn cancel_transfer(self: Pin<&mut Workspace>, job_id: i64);

        /// Sets the process-wide limit in KiB per second (`0` clears it) and
        /// persists it. Running transfers adopt it on their next chunk.
        ///
        /// A method rather than a property write: writing `transferSpeedLimit`
        /// from QML would only change the number the menu shows, while this
        /// reaches the core. The property exists for the menu to read.
        #[qinvokable]
        #[cxx_name = "applyTransferSpeedLimit"]
        fn apply_transfer_speed_limit(self: Pin<&mut Workspace>, kib_per_second: i32);

        /// Re-runs a failed copy or move as a separate elevated process, for
        /// the paths the user's own token cannot write. The window itself stays
        /// unprivileged.
        #[qinvokable]
        #[cxx_name = "elevateOperation"]
        fn elevate_operation(self: Pin<&mut Workspace>, id: &QString);

        #[qinvokable]
        #[cxx_name = "cancelQueuedTransfers"]
        fn cancel_queued_transfers(self: Pin<&mut Workspace>);

        /// Starts a native drag carrying `paths` out of the window. `mode` is
        /// `"copy"`, `"move"`, `"link"` or `"any"`; `"any"` lets the drop
        /// target choose. `signal` draws the dashed frame around the preview;
        /// `count` and `verdict` are the badge and the predicted action line
        /// the ghost carries (§10.3), and `ink`/`canvas` are the two colours
        /// those two marks are painted with. Blocks until the user drops or
        /// cancels.
        #[qinvokable]
        #[cxx_name = "startDragOut"]
        fn start_drag_out(self: Pin<&mut Workspace>, paths: &QStringList,
                          mode: &QString, signal: &QColor, count: i32,
                          verdict: &QString, ink: &QColor, canvas: &QColor);

        /// Creates a shortcut in `destination` for every path in `paths`.
        /// This is the Alt-drop side of an Explorer drag.
        #[qinvokable]
        #[cxx_name = "createShortcuts"]
        fn create_shortcuts(self: Pin<&mut Workspace>, paths: &QStringList, destination: &QString);

        /// Turns the file URLs an OS drop delivered into local paths. Anything
        /// that is not a `file:` URL (a browser URL, a remote scheme) is
        /// dropped: only local paths can enter the transfer pipeline.
        #[qinvokable]
        #[cxx_name = "localDropPaths"]
        fn local_drop_paths(self: Pin<&mut Workspace>, urls: &QStringList) -> QStringList;

        /// Remembers `paths` as the in-app clipboard and, for the local
        /// subset, places them on the system clipboard so Explorer can paste
        /// them. `mode` is `"copy"` or `"cut"`; an empty list clears the
        /// in-app half.
        #[qinvokable]
        #[cxx_name = "setClipboard"]
        fn set_clipboard(self: Pin<&mut Workspace>, mode: &QString, paths: &QStringList);

        #[qinvokable]
        #[cxx_name = "clearClipboard"]
        fn clear_clipboard(self: Pin<&mut Workspace>);

        /// Replaces the system clipboard's text. The context menu's "复制路径"
        /// and the checksum rows' copy buttons use it; the file-list clipboard
        /// above is a separate format. Returns false when the platform has no
        /// clipboard to write to, so the caller can say so.
        #[qinvokable]
        #[cxx_name = "copyText"]
        fn copy_text(self: Pin<&mut Workspace>, text: &QString) -> bool;

        /// Renames a whole batch in one operation. The dialog computes the new
        /// names and sends them paired with their paths, so the swap/staging
        /// rules stay in `dae_core::fs::service` where the undo record is.
        #[qinvokable]
        #[cxx_name = "renameEntriesBatch"]
        fn rename_entries_batch(
            self: Pin<&mut Workspace>,
            paths: &QStringList,
            names: &QStringList,
        );

        /// Runs one of the installed apps' context-menu commands against
        /// `paths`. The rows themselves come from `ShellCommandModel`.
        #[qinvokable]
        #[cxx_name = "invokeShellCommand"]
        fn invoke_shell_command(self: Pin<&mut Workspace>, id: &QString, paths: &QStringList);

        /// Primes the platform's discovery so the first right-click does not
        /// pay for it. Called once per process by the first window.
        #[qinvokable]
        #[cxx_name = "warmShellCommands"]
        fn warm_shell_commands(self: Pin<&mut Workspace>);

        /// Opens the platform's own properties sheet for `path`, if it has
        /// one. Returns false when it does not (every non-Windows platform)
        /// or when the path is remote, so QML can fall back to the in-app
        /// dialog.
        #[qinvokable]
        #[cxx_name = "showNativeProperties"]
        fn show_native_properties(self: Pin<&mut Self>, path: &QString) -> bool;

        #[qinvokable]
        #[cxx_name = "pasteInto"]
        fn paste_into(self: Pin<&mut Workspace>, destination: &QString);

        /// Creates shortcuts in `destination` for the clipboard's files,
        /// whatever effect the source asked for.
        #[qinvokable]
        #[cxx_name = "pasteShortcutInto"]
        fn paste_shortcut_into(self: Pin<&mut Workspace>, destination: &QString);

        /// Re-reads the system clipboard so the paste affordances follow
        /// files another app copied. Called when a window becomes active.
        #[qinvokable]
        #[cxx_name = "refreshClipboard"]
        fn refresh_clipboard(self: Pin<&mut Workspace>);

        #[qinvokable]
        fn undo(self: Pin<&mut Workspace>);

        #[qinvokable]
        fn redo(self: Pin<&mut Workspace>);

        /// Starts the native accent, theme and volume watchers. Called once by
        /// the first window; later calls are ignored.
        #[qinvokable]
        #[cxx_name = "startSystemWatch"]
        fn start_system_watch(self: Pin<&mut Workspace>);

        /// Restores the trash entries with `ids` to their original locations.
        #[qinvokable]
        #[cxx_name = "restoreTrashEntries"]
        fn restore_trash_entries(self: Pin<&mut Workspace>, ids: &QStringList);

        /// Permanently deletes the trash entries with `ids`.
        #[qinvokable]
        #[cxx_name = "deleteTrashEntries"]
        fn delete_trash_entries(self: Pin<&mut Workspace>, ids: &QStringList);

        /// Permanently deletes everything in the trash.
        #[qinvokable]
        #[cxx_name = "emptyTrash"]
        fn empty_trash(self: Pin<&mut Workspace>);

        #[qinvokable]
        #[cxx_name = "openPath"]
        fn open_path(self: Pin<&mut Workspace>, path: &QString);

        #[qinvokable]
        #[cxx_name = "openWith"]
        fn open_with(self: Pin<&mut Workspace>, path: &QString);

        /// Opens `path` with one application chosen in the in-app picker and,
        /// when `set_default` is true, registers it as the new handler.
        #[qinvokable]
        #[cxx_name = "openWithApp"]
        fn open_with_app(
            self: Pin<&mut Workspace>,
            path: &QString,
            app_id: &QString,
            set_default: bool,
        );

        #[qinvokable]
        #[cxx_name = "openTerminal"]
        fn open_terminal(self: Pin<&mut Workspace>, path: &QString);

        #[qinvokable]
        fn reveal(self: Pin<&mut Workspace>, path: &QString);

        /// The byte total of the selected files, formatted. Folders are left
        /// out: their total is a recursive walk, and the details pane says
        /// "files" rather than lying about a directory. Remote paths are
        /// skipped for the same reason `reveal` is local-only.
        #[qinvokable]
        #[cxx_name = "selectionSizeText"]
        fn selection_size_text(&self, paths: &QStringList) -> QString;

        /// The visible subdirectories of `directory`, sorted by name, for the
        /// breadcrumb's sibling menu and the address field's completion.
        /// Local only: one round trip per remote segment would turn a menu
        /// into a network stall, so a remote path answers empty.
        #[qinvokable]
        #[cxx_name = "subdirectories"]
        fn subdirectories(&self, directory: &QString) -> QStringList;

        /// The recently visited directories, newest first, deduplicated.
        /// The address field's completion turns to these when the typed
        /// text has no separator yet: a bare name is far more likely to be
        /// the start of a place the user has been than of a drive root.
        #[qinvokable]
        #[cxx_name = "recentDirectories"]
        fn recent_directories(&self) -> QStringList;

        /// Expands the typed-address shorthands: a leading `~` to the user's
        /// home, `%VAR%` (Windows style) and `$VAR`/`${VAR}` (POSIX style)
        /// to environment values. Anything that does not resolve stays as
        /// typed — the navigation then reports the path in its own words.
        #[qinvokable]
        #[cxx_name = "expandTypedPath"]
        fn expand_typed_path(&self, text: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "recordRecent"]
        fn record_recent(
            self: Pin<&mut Workspace>,
            path: &QString,
            is_directory: bool,
            opened: bool,
        );

        #[qinvokable]
        #[cxx_name = "addFavorite"]
        fn add_favorite(self: Pin<&mut Workspace>, path: &QString);

        /// Pins every local folder in `paths` that is not already pinned. The
        /// favorites drop target sends whatever the drag carried, which may
        /// include files; those are ignored rather than saved as broken rows.
        #[qinvokable]
        #[cxx_name = "addFavoritePaths"]
        fn add_favorite_paths(self: Pin<&mut Workspace>, paths: &QStringList);

        #[qinvokable]
        #[cxx_name = "removeFavorite"]
        fn remove_favorite(self: Pin<&mut Workspace>, path: &QString);

        /// Removes a system place (`home`, `desktop`, …) from the sidebar.
        #[qinvokable]
        #[cxx_name = "hidePlace"]
        fn hide_place(self: Pin<&mut Workspace>, kind: &QString);

        /// Brings every hidden system place back.
        #[qinvokable]
        #[cxx_name = "showAllPlaces"]
        fn show_all_places(self: Pin<&mut Workspace>);

        #[qinvokable]
        #[cxx_name = "saveTerminalSettings"]
        fn save_terminal_settings(
            self: Pin<&mut Workspace>,
            font_size: i32,
            line_height: f64,
            family: &QString,
        );

        /// Persists one binding for an id from `default_shortcuts()`. An empty
        /// string unbinds the action, which is how a user disables a key that
        /// collides with a shell or IME shortcut.
        #[qinvokable]
        #[cxx_name = "setShortcut"]
        fn set_shortcut(self: Pin<&mut Workspace>, id: &QString, binding: &QString);

        #[qinvokable]
        #[cxx_name = "resetShortcut"]
        fn reset_shortcut(self: Pin<&mut Workspace>, id: &QString);

        /// Restores every binding without touching the terminal or file-manager
        /// preferences that live in the same document.
        #[qinvokable]
        #[cxx_name = "resetShortcuts"]
        fn reset_shortcuts(self: Pin<&mut Workspace>);

        #[qinvokable]
        #[cxx_name = "openLogDirectory"]
        fn open_log_directory(self: Pin<&mut Workspace>);

        #[qinvokable]
        #[cxx_name = "refreshDefaultManager"]
        fn refresh_default_manager(self: Pin<&mut Workspace>);

        #[qinvokable]
        #[cxx_name = "setDefaultFileManager"]
        fn set_default_file_manager(self: Pin<&mut Workspace>);

        #[qinvokable]
        #[cxx_name = "unsetDefaultFileManager"]
        fn unset_default_file_manager(self: Pin<&mut Workspace>);

        /// The operation succeeded; `focus` is the path to select, empty when
        /// the operation created nothing addressable.
        #[qsignal]
        #[cxx_name = "operationFinished"]
        fn operation_finished(self: Pin<&mut Workspace>, kind: &QString, focus: &QString);

        /// The operation failed; `message` is already user-facing.
        #[qsignal]
        #[cxx_name = "operationFailed"]
        fn operation_failed(self: Pin<&mut Workspace>, kind: &QString, message: &QString);

        /// Two files were compared; `equal` is the SHA-256 verdict.
        #[qsignal]
        #[cxx_name = "compareFinished"]
        fn compare_finished(self: Pin<&mut Workspace>, equal: bool);

        /// Measured folder totals as a JSON array of `{"path": …, "size": …}`.
        /// One event per pass, not per folder: the model applies them in a
        /// batch and repaints each cell once.
        #[qsignal]
        #[cxx_name = "folderSizesMeasured"]
        fn folder_sizes_measured(self: Pin<&mut Workspace>, sizes: &QString);

        /// The Locksmith query for `request` finished. `report` is JSON
        /// (`{holders,scanned,truncated}`); `error` is a user-facing sentence
        /// when the query could not run, empty otherwise.
        #[qsignal]
        #[cxx_name = "fileHoldersFound"]
        fn file_holders_found(
            self: Pin<&mut Workspace>,
            request: &QString,
            report: &QString,
            error: &QString,
        );

        /// `endProcess` finished. `ok` is the platform verdict; on failure
        /// `message` is the OS error.
        #[qsignal]
        #[cxx_name = "processEnded"]
        fn process_ended(self: Pin<&mut Workspace>, pid: i32, ok: bool, message: &QString);

        /// An extraction needs a password. `target` is the archive path and
        /// `message` is a ready-to-display explanation (also a wrong-password
        /// retry). The caller answers by calling `extractArchive` again.
        #[qsignal]
        #[cxx_name = "archivePasswordRequired"]
        fn archive_password_required(
            self: Pin<&mut Workspace>,
            target: &QString,
            message: &QString,
        );

        #[qsignal]
        #[cxx_name = "favoritesChanged"]
        fn favorites_changed(self: Pin<&mut Workspace>);

        /// A system place was hidden or restored; the rows are re-resolved by
        /// `SidebarModel.refresh()` and the overview's list.
        #[qsignal]
        #[cxx_name = "placesChanged"]
        fn places_changed(self: Pin<&mut Workspace>);

        /// A drive or mount arrived or left. The sidebar answers by calling
        /// `SidebarModel.refresh()`, which already re-reads the disk list off
        /// the Qt thread.
        #[qsignal]
        #[cxx_name = "volumesChanged"]
        fn volumes_changed(self: Pin<&mut Workspace>);

        /// Verifies a server connection with the credentials typed into the
        /// dialog, without saving them. `port` is text so an empty field means
        /// the protocol default; the outcome arrives on `connectionTestFinished`.
        #[qinvokable]
        #[cxx_name = "testConnection"]
        fn test_connection(
            self: Pin<&mut Workspace>,
            protocol: &QString,
            host: &QString,
            port: &QString,
            username: &QString,
            password: &QString,
        );

        /// Persists a server connection and its password policy. The outcome
        /// arrives on `connectionSaveFinished`, carrying the connection id.
        #[qinvokable]
        #[cxx_name = "saveConnection"]
        fn save_connection(
            self: Pin<&mut Workspace>,
            protocol: &QString,
            host: &QString,
            port: &QString,
            username: &QString,
            password: &QString,
            remember_password: bool,
        );

        #[qinvokable]
        #[cxx_name = "deleteConnection"]
        fn delete_connection(self: Pin<&mut Workspace>, id: &QString);

        /// Runs the provider's loopback OAuth flow. The browser round trip can
        /// take minutes, so the result arrives on `cloudAuthorizeFinished`.
        #[qinvokable]
        #[cxx_name = "authorizeCloudAccount"]
        fn authorize_cloud_account(
            self: Pin<&mut Workspace>,
            provider: &QString,
            client_id: &QString,
            client_secret: &QString,
        );

        #[qinvokable]
        #[cxx_name = "deleteCloudAccount"]
        fn delete_cloud_account(self: Pin<&mut Workspace>, id: &QString);

        /// A saved connection was tested; `message` is a finished sentence.
        #[qsignal]
        #[cxx_name = "connectionTestFinished"]
        fn connection_test_finished(self: Pin<&mut Workspace>, ok: bool, message: &QString);

        /// A connection was saved. `id` is the explorer path on success.
        #[qsignal]
        #[cxx_name = "connectionSaveFinished"]
        fn connection_save_finished(
            self: Pin<&mut Workspace>,
            ok: bool,
            id: &QString,
            message: &QString,
        );

        /// The OAuth round trip finished. `message` is the account id
        /// (`cloud://…`) on success, a user-facing sentence otherwise.
        #[qsignal]
        #[cxx_name = "cloudAuthorizeFinished"]
        fn cloud_authorize_finished(self: Pin<&mut Workspace>, ok: bool, message: &QString);

        /// A connection or cloud account was added or removed. The sidebar
        /// answers by refreshing its model.
        #[qsignal]
        #[cxx_name = "connectionsChanged"]
        fn connections_changed(self: Pin<&mut Workspace>);
    }

    impl cxx_qt::Threading for Workspace {}
}

/// The C++ side of `QStyleHints::colorSchemeChanged`.
fn dae_qt_theme_changed(dark: bool) {
    crate::system_watch::theme_changed(dark);
}

pub struct WorkspaceRust {
    status: QString,
    last_error: QString,
    busy: bool,
    can_undo: bool,
    can_redo: bool,
    progress_label: QString,
    progress_done: i32,
    progress_total: i32,
    progress_kind: QString,
    conflict_index: i32,
    conflict_total: i32,
    conflict_name: QString,
    conflict_cut: bool,
    conflict_source_category: QString,
    conflict_source_detail: QString,
    conflict_target_category: QString,
    conflict_target_detail: QString,
    conflict_job_id: i64,
    conflict_waiting: i32,
    /// Each asking job's frozen conflict prompt, in the order their prompts
    /// arrived. The head is the question on screen; the rest wait untouched
    /// until their turn, so several jobs can ask at once without their
    /// collisions interleaving on one dialog.
    pending_conflicts: Vec<PendingPrompt>,
    queued_transfers: QStringList,
    queued_transfer_paths: QStringList,
    transfers_json: QString,
    transfer_speed_limit: i32,
    transfer_speed: QString,
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
    drive_capabilities: QString,
    external_archive_formats: QString,
    custom_actions_json: QString,
    operations_json: QString,
    failure_count: i32,
    /// When each running operation began, with the label it showed. A stack
    /// rather than a map: operations legitimately overlap (a background size
    /// scan during a copy), and the newest start is the one finishing next.
    operation_starts: Vec<(u64, String)>,
    /// The retry payload for the operation about to start. `extractArchive`
    /// and a transfer set it immediately after `beginOperation`; finishing the
    /// operation consumes it.
    retry_hint: Option<String>,
    clipboard_mode: QString,
    clipboard_paths: QStringList,
    /// The exact paths this shell last put on the system clipboard. A paste
    /// compares against it to tell its own list from files another app copied.
    system_clipboard_paths: Vec<String>,
    /// The system's clipboard counter when the mirror was last written or
    /// refreshed; a different counter means another app changed it since.
    system_clipboard_sequence: u32,
    /// Bumped whenever the clipboard changes, so a refresh that was already
    /// in flight cannot overwrite a newer copy.
    clipboard_generation: u64,
    accent: QString,
    system_dark: bool,
    home_directory: QString,
    terminal_font_size: i32,
    terminal_line_height: f64,
    terminal_font_family: QString,
    shortcuts_json: QString,
    log_directory: QString,
    search_index_available: bool,
    open_with_picker_supported: bool,
    default_manager_supported: bool,
    default_manager_is_default: bool,
    default_manager_is_registered: bool,
    default_manager_detail: QString,
    default_manager_error: QString,
    default_manager_busy: bool,
    active_operations: i32,
}

impl Default for WorkspaceRust {
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
            status: QString::default(),
            last_error: QString::default(),
            busy: false,
            can_undo: false,
            can_redo: false,
            progress_label: QString::default(),
            progress_done: 0,
            progress_total: 0,
            progress_kind: QString::default(),
            conflict_index: 0,
            conflict_total: 0,
            conflict_name: QString::default(),
            conflict_cut: false,
            conflict_source_category: QString::default(),
            conflict_source_detail: QString::default(),
            conflict_target_category: QString::default(),
            conflict_target_detail: QString::default(),
            conflict_job_id: 0,
            conflict_waiting: 0,
            pending_conflicts: Vec::new(),
            queued_transfers: QStringList::default(),
            queued_transfer_paths: QStringList::default(),
            transfers_json: QString::from("[]"),
            transfer_speed_limit,
            transfer_speed: QString::default(),
            transfer_rows: Vec::new(),
            next_local_job: 1_000_000_000,
            transfer_scheduler: None,
            custom_actions_json: QString::from(custom_actions_json().as_str()),
            drive_capabilities: QString::from(drive_capabilities_json().as_str()),
            external_archive_formats: QString::from(external_archive_formats_json().as_str()),
            operations_json: QString::from(operations_text.as_str()),
            failure_count: 0,
            size_scan: None,
            operations,
            operation_starts: Vec::new(),
            retry_hint: None,
            clipboard_mode: QString::default(),
            clipboard_paths: QStringList::default(),
            system_clipboard_paths: Vec::new(),
            system_clipboard_sequence: 0,
            clipboard_generation: 0,
            accent: QString::from(platform::accent_hex().as_str()),
            system_dark: crate::config::system_dark(),
            home_directory: QString::from(home.as_str()),
            terminal_font_size: settings.terminal.font_size as i32,
            terminal_line_height: settings.terminal.line_height as f64,
            terminal_font_family: QString::from(
                settings.terminal.font_family.as_deref().unwrap_or(""),
            ),
            shortcuts_json: QString::from(shortcuts_json(&settings.shortcuts).as_str()),
            log_directory: QString::from(
                crate::logging::app_log_directory()
                    .to_string_lossy()
                    .as_ref(),
            ),
            search_index_available: dae_core::fs::search_index::backend().is_some(),
            open_with_picker_supported: cfg!(any(target_os = "macos", target_os = "linux")),
            default_manager_supported: cfg!(any(target_os = "macos", target_os = "linux", windows)),
            default_manager_is_default: false,
            default_manager_is_registered: false,
            default_manager_detail: QString::default(),
            default_manager_error: QString::default(),
            default_manager_busy: false,
            active_operations: 0,
        }
    }
}

/// Turns core events into queued Qt updates.
struct QtHost {
    thread: cxx_qt::CxxQtThread<qobject::Workspace>,
}

impl Host for QtHost {
    fn emit(&self, event: Event) {
        let _ = self
            .thread
            .queue(move |mut workspace| workspace.as_mut().apply_event(event));
    }
}

fn host_for(thread: &cxx_qt::CxxQtThread<qobject::Workspace>) -> Arc<dyn Host> {
    Arc::new(QtHost {
        thread: thread.clone(),
    })
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
fn dispatch<F>(thread: cxx_qt::CxxQtThread<qobject::Workspace>, kind: &'static str, future: F)
where
    F: std::future::Future<Output = Outcome> + Send + 'static,
{
    runtime::spawn(async move {
        let outcome = future.await;
        let _ = thread.queue(move |mut workspace| {
            if outcome.cancelled {
                workspace.as_mut().finish_cancelled_operation(kind);
            } else {
                workspace.as_mut().finish_operation(
                    kind,
                    outcome.focus.as_str(),
                    outcome.error,
                );
            }
        });
    });
}

impl qobject::Workspace {
    fn begin_operation(mut self: Pin<&mut Self>, label: &str) {
        self.as_mut()
            .rust_mut()
            .operation_starts
            .push((now_millis(), label.to_owned()));
        self.as_mut().rust_mut().retry_hint = None;
        let active = self.active_operations + 1;
        self.as_mut().rust_mut().active_operations = active;
        self.as_mut().set_busy(true);
        self.as_mut().set_progress_done(0);
        self.as_mut().set_progress_total(0);
        // Unknown until the operation reports: a rename sends no progress at
        // all, and a stale code from the previous command would make the strip
        // read the new one's numbers in the wrong unit.
        self.as_mut().set_progress_kind(QString::default());
        self.as_mut().set_progress_label(QString::from(label));
        self.as_mut().set_last_error(QString::default());
    }

    fn finish_operation(mut self: Pin<&mut Self>, kind: &str, focus: &str, error: Option<String>) {
        self.as_mut().finish_operation_state(kind, focus, error, false);
    }

    /// A transfer the user stopped: no error toast, no failure badge, and a
    /// status-center row whose action is "continue" rather than "retry".
    fn finish_cancelled_operation(mut self: Pin<&mut Self>, kind: &str) {
        self.as_mut()
            .finish_operation_state(kind, "", None, true);
    }

    fn finish_operation_state(
        mut self: Pin<&mut Self>,
        kind: &str,
        focus: &str,
        error: Option<String>,
        cancelled: bool,
    ) {
        self.as_mut().settle_operation();
        self.as_mut()
            .record_operation_state(kind, error.as_deref(), cancelled);

        let kind_value = QString::from(kind);
        let focus_value = QString::from(focus);

        if cancelled {
            // Neither a success nor an error: the status bar says what
            // happened, and the status center's row offers to continue.
            let status = "Transfer cancelled";
            self.as_mut().set_status(QString::from(status));
            self.as_mut().operation_finished(&kind_value, &focus_value);
            return;
        }

        match error {
            Some(message) => {
                self.as_mut()
                    .set_last_error(QString::from(message.as_str()));
                let message_value = QString::from(message.as_str());
                self.as_mut().operation_failed(&kind_value, &message_value);
            }
            None => {
                let status = success_message(kind);
                self.as_mut().set_status(QString::from(status));
                self.as_mut().operation_finished(&kind_value, &focus_value);
            }
        }
    }

    /// Turns a drive tool's outcome into the same toast and status-center row
    /// every other command uses.
    fn run_drive_action(
        mut self: Pin<&mut Self>,
        kind: &str,
        result: Result<(), FileSystemError>,
        success: &str,
    ) {
        match result {
            Ok(()) => {
                self.as_mut().begin_operation(success);
                self.as_mut().set_status(QString::from(success));
                self.as_mut().finish_operation(kind, "", None);
            }
            Err(error) => {
                let message = error.to_string();
                self.as_mut().finish_operation(kind, "", Some(message));
            }
        }
    }

    /// One line for the status center. Runs before the toast, so a failed
    /// operation is already in the history when its message appears.
    fn record_operation(mut self: Pin<&mut Self>, kind: &str, error: Option<&str>) {
        self.as_mut().record_operation_state(kind, error, false);
    }

    fn record_operation_state(
        mut self: Pin<&mut Self>,
        kind: &str,
        error: Option<&str>,
        cancelled: bool,
    ) {
        let (started, label) = self
            .as_mut()
            .rust_mut()
            .operation_starts
            .pop()
            .unwrap_or_else(|| (now_millis(), String::new()));
        let retry = self
            .as_mut()
            .rust_mut()
            .retry_hint
            .take()
            .unwrap_or_default();
        self.as_mut()
            .record_operation_row(kind, error, cancelled, started, label, retry);
    }

    /// Writes one status-center row from explicit provenance. Transfers call
    /// this with their own row's start, label and retry payload: several run
    /// at once, so the shared `operation_starts` stack — newest start finishes
    /// next — cannot be theirs.
    fn record_operation_row(
        mut self: Pin<&mut Self>,
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
            let mut rust = self.as_mut().rust_mut();
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
        self.as_mut()
            .set_operations_json(QString::from(json.as_str()));
        if failed {
            let count = self.failure_count + 1;
            self.as_mut().set_failure_count(count);
        }
    }

    fn eject_drive(mut self: Pin<&mut Self>, path: &QString) {
        self.as_mut()
            .run_drive_action("drive", drives::eject(&path.to_string()), "Ejected");
    }

    fn unmount_drive(mut self: Pin<&mut Self>, path: &QString) {
        self.as_mut()
            .run_drive_action("drive", drives::unmount(&path.to_string()), "Unmounted");
    }

    fn mount_drive(mut self: Pin<&mut Self>, path: &QString) {
        self.as_mut()
            .run_drive_action("drive", drives::mount(&path.to_string()), "Mounted");
    }

    fn format_drive(mut self: Pin<&mut Self>, path: &QString) {
        self.as_mut().run_drive_action(
            "drive",
            drives::open_format_tool(&path.to_string()),
            "Format dialog opened",
        );
    }

    fn open_storage_cleanup(mut self: Pin<&mut Self>) {
        self.as_mut().run_drive_action(
            "drive",
            drives::open_cleanup_tool(),
            "Storage settings opened",
        );
    }

    fn drive_encryption_status(self: Pin<&mut Self>, path: &QString) -> QString {
        match drives::encryption_status(&path.to_string()) {
            Ok(text) => QString::from(text.as_str()),
            Err(error) => QString::from(error.to_string().as_str()),
        }
    }

    fn open_previous_versions(mut self: Pin<&mut Self>, path: &QString) {
        self.as_mut().run_drive_action(
            "drive",
            drives::open_previous_versions(&path.to_string()),
            "Previous versions opened",
        );
    }

    fn custom_action_definitions(&self) -> QString {
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
        QString::from(json.as_str())
    }

    fn custom_actions_for(
        self: Pin<&mut Self>,
        paths: &QStringList,
        directory: &QString,
    ) -> QString {
        let selection = strings(paths);
        let directory = directory.to_string();
        let entries: Vec<serde_json::Value> = actions::available_actions()
            .into_iter()
            .filter(|action| actions::selection_allows(action, &selection, &directory))
            .map(|action| serde_json::json!({"id": action.id, "name": action.name}))
            .collect();
        let json = serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_owned());
        QString::from(json.as_str())
    }

    fn run_custom_action(
        mut self: Pin<&mut Self>,
        id: &QString,
        paths: &QStringList,
        directory: &QString,
    ) {
        let id = id.to_string();
        let selection = strings(paths);
        let directory = directory.to_string();
        let Some(action) = actions::action_by_id(&id) else {
            self.as_mut()
                .finish_operation("action", "", Some("Unknown action".to_owned()));
            return;
        };
        if !actions::selection_allows(&action, &selection, &directory) {
            self.as_mut().finish_operation(
                "action",
                "",
                Some("This action does not apply to the selection".to_owned()),
            );
            return;
        }
        match actions::run(&action, &selection, &directory) {
            Ok(program) => {
                self.as_mut().begin_operation("Running action…");
                self.as_mut()
                    .set_status(QString::from(format!("Ran {program}").as_str()));
                self.as_mut().finish_operation("action", "", None);
            }
            Err(error) => {
                self.as_mut()
                    .finish_operation("action", "", Some(error.to_string()));
            }
        }
    }

    fn share_supported(self: Pin<&mut Self>) -> bool {
        crate::share::supported()
    }

    fn share_paths(self: Pin<&mut Self>, paths: &QStringList, handle: f64) -> bool {
        crate::share::share_paths(paths, handle)
    }

    fn save_custom_action(
        mut self: Pin<&mut Self>,
        id: &QString,
        name: &QString,
        command: &QString,
        args: &QString,
        selection: &QString,
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
            self.as_mut()
                .set_custom_actions_json(QString::from(json.as_str()));
        }
    }

    fn remove_custom_action(mut self: Pin<&mut Self>, id: &QString) {
        let id = id.to_string();
        let Ok(mut settings) = dae_core::settings::load_settings() else {
            return;
        };
        settings.actions.retain(|action| action.id != id);
        if dae_core::settings::save_settings(settings).is_ok() {
            let json = custom_actions_json();
            self.as_mut()
                .set_custom_actions_json(QString::from(json.as_str()));
        }
    }

    fn reset_custom_actions(mut self: Pin<&mut Self>) {
        let Ok(mut settings) = dae_core::settings::load_settings() else {
            return;
        };
        settings.actions = dae_core::settings::default_actions();
        if dae_core::settings::save_settings(settings).is_ok() {
            let json = custom_actions_json();
            self.as_mut()
                .set_custom_actions_json(QString::from(json.as_str()));
        }
    }

    fn mark_operations_read(mut self: Pin<&mut Self>) {
        if self.failure_count != 0 {
            self.as_mut().set_failure_count(0);
        }
    }

    fn clear_operation_history(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().operations.clear();
        persist_operations(&[]);
        self.as_mut().set_operations_json(QString::from("[]"));
        self.as_mut().set_failure_count(0);
    }

    fn retry_operation(mut self: Pin<&mut Self>, id: &QString) {
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
                self.as_mut().extract_archive(
                    &QString::from(archive),
                    &QString::from(destination),
                    &QString::default(),
                );
            }
            Some("transfer") => {
                let mut paths = QStringList::default();
                if let Some(items) = payload["paths"].as_array() {
                    for item in items {
                        if let Some(path) = item.as_str() {
                            paths.append(QString::from(path));
                        }
                    }
                }
                let destination = payload["destination"].as_str().unwrap_or_default();
                let cut = payload["cut"].as_bool().unwrap_or(false);
                let resume = payload["resume"].as_bool().unwrap_or(false);

                if paths.len() > 0 && resume {
                    self.as_mut().enqueue_resumed_transfer(
                        strings(&paths),
                        destination.to_owned(),
                        cut,
                    );
                } else if paths.len() > 0 {
                    self.as_mut()
                        .transfer_entries(&paths, &QString::from(destination), cut);
                }
            }
            _ => {}
        }
    }

    /// Clears the progress row without reporting an outcome. A protected
    /// archive uses this before asking for a password: nothing failed yet, and
    /// an `operationFinished` would wrongly say the extraction completed.
    fn settle_operation(mut self: Pin<&mut Self>) {
        let active = (self.active_operations - 1).max(0);
        self.as_mut().rust_mut().active_operations = active;
        self.as_mut().set_busy(active > 0);
        self.as_mut().set_progress_done(0);
        self.as_mut().set_progress_total(0);
        self.as_mut().set_progress_kind(QString::default());
        self.as_mut().set_progress_label(QString::default());
    }

    fn request_archive_password(mut self: Pin<&mut Self>, target: &str, message: &str) {
        let target = QString::from(target);
        let message = QString::from(message);
        self.as_mut().set_last_error(message.clone());
        self.as_mut().archive_password_required(&target, &message);
    }

    fn apply_event(mut self: Pin<&mut Self>, event: Event) {
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
                    let mut rust = self.as_mut().rust_mut();
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
                    self.as_mut().publish_transfers();
                    return;
                }

                let label = current_path
                    .as_deref()
                    .map(display_name_from_path)
                    .unwrap_or_else(|| operation_label(operation).to_owned());
                self.as_mut()
                    .set_progress_label(QString::from(label.as_str()));
                self.as_mut()
                    .set_progress_kind(QString::from(operation.code()));
                self.as_mut()
                    .set_progress_done(completed.min(i32::MAX as u64) as i32);
                let total = total.unwrap_or(0).min(i32::MAX as u64) as i32;
                self.as_mut().set_progress_total(total);

                if matches!(phase, dae_core::progress::FileOperationPhase::Preparing) {
                    self.as_mut().set_busy(true);
                }
            }
            Event::TransferJob(event) => self.as_mut().apply_job_event(event),
            Event::UndoRedoChanged { can_undo, can_redo } => {
                self.as_mut().set_can_undo(can_undo);
                self.as_mut().set_can_redo(can_redo);
            }
            Event::DirectoryChanged { .. }
            | Event::DirectoryEntries { .. }
            | Event::SystemThemeChanged(_)
            | Event::SystemAccentChanged(_)
            | Event::DirectorySizeProgress { .. }
            | Event::FileHashProgress { .. } => {}
        }
    }

    fn create_folder(mut self: Pin<&mut Self>, directory: &QString, name: &QString) {
        self.as_mut()
            .create_entry(directory, name, NewEntryKind::Directory, "create");
    }

    fn create_file(mut self: Pin<&mut Self>, directory: &QString, name: &QString) {
        self.as_mut()
            .create_entry(directory, name, NewEntryKind::File, "create");
    }

    fn create_from_template(
        mut self: Pin<&mut Self>,
        directory: &QString,
        template_path: &QString,
        name: &QString,
    ) {
        let directory = directory.to_string();
        let template_path = template_path.to_string();
        let name = name.to_string();
        if name.trim().is_empty() {
            self.as_mut()
                .finish_operation("create", "", Some("Name cannot be empty".to_owned()));
            return;
        }
        if !vfs::is_local_path(&directory) || !vfs::is_local_path(&template_path) {
            self.as_mut().finish_operation(
                "create",
                "",
                Some("Templates can only be used in local folders".to_owned()),
            );
            return;
        }

        self.as_mut().begin_operation("Creating…");
        let thread = self.as_mut().qt_thread();
        let host = host_for(&thread);
        dispatch(thread, "create", async move {
            match service::create_from_template(directory, template_path, name, host).await {
                Ok(created) => Outcome::ok(created),
                Err(error) => Outcome::failed(error),
            }
        });
    }

    fn templates_json(&self) -> QString {
        match templates::list() {
            Ok(entries) => {
                let text = serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_owned());
                QString::from(text.as_str())
            }
            Err(_) => QString::from("[]"),
        }
    }

    fn open_templates_directory(mut self: Pin<&mut Self>) {
        let directory = match templates::ensure_directory() {
            Ok(directory) => directory,
            Err(error) => {
                self.as_mut()
                    .finish_operation("open", "", Some(error.to_string()));
                return;
            }
        };
        let path = directory.to_string_lossy().into_owned();
        if let Err(message) = platform::open_path(&path) {
            self.as_mut().finish_operation("open", "", Some(message));
            return;
        }
    }

    fn set_templates_directory(mut self: Pin<&mut Self>, path: &QString) {
        let directory = path.to_string().trim().to_owned();
        if directory.is_empty() || !Path::new(&directory).is_dir() {
            self.as_mut()
                .set_last_error(QString::from("The templates folder does not exist"));
            return;
        }

        let Ok(mut settings) = dae_core::settings::load_settings() else {
            self.as_mut()
                .set_last_error(QString::from("Could not read settings"));
            return;
        };
        settings.templates.directory = Some(directory);
        match dae_core::settings::save_settings(settings) {
            Ok(_) => self.as_mut().set_last_error(QString::default()),
            Err(error) => self
                .as_mut()
                .set_last_error(QString::from(error.to_string().as_str())),
        }
    }

    fn file_holders(mut self: Pin<&mut Self>, paths: &QStringList) {
        let requested: Vec<String> = strings(paths)
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
            let report = QString::from(r#"{"holders":[],"scanned":0,"truncated":false}"#);
            let request_value = QString::from(request.as_str());
            let error = QString::from("Locksmith only checks local paths");
            self.as_mut()
                .file_holders_found(&request_value, &report, &error);
            return;
        }

        let thread = self.as_mut().qt_thread();
        runtime::spawn(async move {
            let outcome =
                tokio::task::spawn_blocking(move || occupancy::holders_deep(&local)).await;

            let _ = thread.queue(move |mut workspace| match outcome {
                Ok(report) => {
                    let report =
                        serde_json::to_string(&report).unwrap_or_else(|_| "{}".to_owned());
                    let request_value = QString::from(request.as_str());
                    let report_value = QString::from(report.as_str());
                    workspace.as_mut().file_holders_found(
                        &request_value,
                        &report_value,
                        &QString::default(),
                    );
                }
                Err(error) => {
                    let request_value = QString::from(request.as_str());
                    let report_value =
                        QString::from(r#"{"holders":[],"scanned":0,"truncated":false}"#);
                    let message = QString::from(error.to_string().as_str());
                    workspace.as_mut().file_holders_found(
                        &request_value,
                        &report_value,
                        &message,
                    );
                }
            });
        });
    }

    fn end_process(mut self: Pin<&mut Self>, pid: i32) {
        let thread = self.as_mut().qt_thread();
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

            let _ = thread.queue(move |mut workspace| match outcome {
                Ok(Ok(())) => workspace
                    .as_mut()
                    .process_ended(pid, true, &QString::default()),
                Ok(Err(error)) => {
                    let message = error.to_string();
                    workspace.as_mut().process_ended(
                        pid,
                        false,
                        &QString::from(message.as_str()),
                    );
                }
                Err(error) => {
                    let message = error.to_string();
                    workspace.as_mut().process_ended(
                        pid,
                        false,
                        &QString::from(message.as_str()),
                    );
                }
            });
        });
    }

    fn create_entry(
        mut self: Pin<&mut Self>,
        directory: &QString,
        name: &QString,
        entry_kind: NewEntryKind,
        operation_kind: &'static str,
    ) {
        let directory = directory.to_string();
        let name = name.to_string();
        if name.trim().is_empty() {
            self.as_mut().finish_operation(
                operation_kind,
                "",
                Some("Name cannot be empty".to_owned()),
            );
            return;
        }

        self.as_mut().begin_operation("Creating…");
        let thread = self.as_mut().qt_thread();
        let host = host_for(&thread);
        dispatch(thread, operation_kind, async move {
            match service::create_entry(directory, name, entry_kind, host).await {
                Ok(created) => Outcome::ok(created),
                Err(error) => Outcome::failed(error),
            }
        });
    }

    fn rename_entry(mut self: Pin<&mut Self>, path: &QString, new_name: &QString) {
        let path = path.to_string();
        let new_name = new_name.to_string();
        if new_name.trim().is_empty() {
            self.as_mut()
                .finish_operation("rename", "", Some("Name cannot be empty".to_owned()));
            return;
        }

        self.as_mut().begin_operation("Renaming…");
        let focus = dae_core::fs::undo::renamed_path(&path, &new_name).unwrap_or_default();
        let thread = self.as_mut().qt_thread();
        let host = host_for(&thread);
        dispatch(thread, "rename", async move {
            match service::rename_entry(path, new_name, host).await {
                Ok(()) => Outcome::ok(focus),
                Err(error) => Outcome::failed(error),
            }
        });
    }

    fn compare_hashes(mut self: Pin<&mut Self>, paths: &QStringList) {
        let paths = strings(paths);
        if paths.len() != 2 {
            return;
        }
        if !paths.iter().all(|path| vfs::is_local_path(path)) {
            self.as_mut().finish_operation(
                "compare",
                "",
                Some("Hashes can only be compared for local files".to_owned()),
            );
            return;
        }

        self.as_mut().begin_operation("Comparing hashes…");
        let thread = self.as_mut().qt_thread();
        runtime::spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                let first = hashing::digests(Path::new(&paths[0]))?;
                let second = hashing::digests(Path::new(&paths[1]))?;
                Ok::<bool, FileSystemError>(first.sha256 == second.sha256)
            })
            .await;

            let _ = thread.queue(move |mut workspace| match result {
                Ok(Ok(equal)) => {
                    workspace.as_mut().settle_operation();
                    workspace.as_mut().record_operation("compare", None);
                    workspace.as_mut().compare_finished(equal);
                }
                Ok(Err(error)) => {
                    workspace
                        .as_mut()
                        .finish_operation("compare", "", Some(error.to_string()))
                }
                Err(error) => {
                    workspace
                        .as_mut()
                        .finish_operation("compare", "", Some(error.to_string()))
                }
            });
        });
    }

    fn measure_folders(mut self: Pin<&mut Self>, paths: &QStringList) {
        let directories: Vec<String> = strings(paths)
            .into_iter()
            .filter(|path| vfs::is_local_path(path) && Path::new(path).is_dir())
            .collect();
        if directories.is_empty() {
            return;
        }

        // A second pass replaces the first: the user asked again, and two
        // walks of the same tree would report the same numbers twice.
        let cancel = Arc::new(AtomicBool::new(false));
        if let Some(previous) = self.as_mut().rust_mut().size_scan.replace(cancel.clone()) {
            previous.store(true, Ordering::Relaxed);
        }

        self.as_mut().begin_operation("Calculating sizes…");
        let thread = self.as_mut().qt_thread();
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

            let _ = thread.queue(move |mut workspace| {
                workspace.as_mut().finish_size_scan(payload);
            });
        });
    }

    fn cancel_measure_folders(mut self: Pin<&mut Self>) {
        let cancelled = self.as_mut().rust_mut().size_scan.take();
        if let Some(cancel) = cancelled {
            cancel.store(true, Ordering::Relaxed);
            self.as_mut().settle_operation();
            self.as_mut()
                .set_status(QString::from("Size calculation cancelled"));
        }
    }

    fn finish_size_scan(mut self: Pin<&mut Self>, payload: String) {
        self.as_mut().rust_mut().size_scan = None;
        let sizes = QString::from(payload.as_str());
        self.as_mut().folder_sizes_measured(&sizes);
        self.as_mut().settle_operation();
        self.as_mut().set_status(QString::from("Sizes calculated"));
    }

    fn trash_entries(mut self: Pin<&mut Self>, paths: &QStringList) {
        let paths = strings(paths);
        if paths.is_empty() {
            return;
        }
        self.as_mut().begin_operation("Moving to Trash…");
        let thread = self.as_mut().qt_thread();
        let host = host_for(&thread);
        dispatch(thread, "trash", async move {
            match service::trash_entries(paths, operation_id(), host).await {
                Ok(()) => Outcome::empty(),
                Err(error) => Outcome::failed(error),
            }
        });
    }

    fn delete_entries(mut self: Pin<&mut Self>, paths: &QStringList) {
        let paths = strings(paths);
        if paths.is_empty() {
            return;
        }
        self.as_mut().begin_operation("Deleting…");
        let thread = self.as_mut().qt_thread();
        let host = host_for(&thread);
        dispatch(thread, "delete", async move {
            match service::delete_entries(paths, operation_id(), host).await {
                Ok(()) => Outcome::empty(),
                Err(error) => Outcome::failed(error),
            }
        });
    }

    fn duplicate_entries(mut self: Pin<&mut Self>, paths: &QStringList) {
        let paths = strings(paths);
        if paths.is_empty() {
            return;
        }
        self.as_mut().begin_operation("Duplicating…");
        let thread = self.as_mut().qt_thread();
        let host = host_for(&thread);
        dispatch(thread, "duplicate", async move {
            match service::duplicate_entries(paths, operation_id(), host).await {
                Ok(created) => Outcome::ok(created.into_iter().next().unwrap_or_default()),
                Err(error) => Outcome::failed(error),
            }
        });
    }

    fn compress_entries(
        mut self: Pin<&mut Self>,
        sources: &QStringList,
        destination_dir: &QString,
        format: &QString,
        password: &QString,
    ) {
        let sources = strings(sources);
        if sources.is_empty() {
            self.as_mut().finish_operation(
                "compress",
                "",
                Some("Select items to compress".to_owned()),
            );
            return;
        }

        let destination_dir = destination_dir.to_string();
        if destination_dir.is_empty() {
            self.as_mut().finish_operation(
                "compress",
                "",
                Some("Cannot determine archive destination".to_owned()),
            );
            return;
        }

        let Some(format) = archive_format(&format.to_string()) else {
            self.as_mut().finish_operation(
                "compress",
                "",
                Some("Unsupported archive format".to_owned()),
            );
            return;
        };

        let password = optional_string(password);
        self.as_mut().begin_operation("Compressing…");
        let thread = self.as_mut().qt_thread();
        let host = host_for(&thread);
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

    fn extract_archive(
        mut self: Pin<&mut Self>,
        archive_path: &QString,
        destination_dir: &QString,
        password: &QString,
    ) {
        let archive_path = archive_path.to_string();
        if archive_path.is_empty() {
            return;
        }

        let destination_dir = optional_string(destination_dir);
        let password = optional_string(password);
        let password_supplied = password.is_some();
        self.as_mut().begin_operation("Extracting…");
        let retry = serde_json::json!({
            "kind": "extract",
            "archive": archive_path,
            "destination": destination_dir.clone().unwrap_or_default(),
        });
        self.as_mut().rust_mut().retry_hint = Some(retry.to_string());
        let thread = self.as_mut().qt_thread();
        let host = host_for(&thread);
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

            let _ = thread.queue(move |mut workspace| match result {
                Ok(destination) => {
                    workspace
                        .as_mut()
                        .finish_operation("extract", &destination, None)
                }
                Err(FileSystemError::WrongPassword(_)) => {
                    let message = if password_supplied {
                        "Incorrect password. Try again."
                    } else {
                        "This archive is encrypted. Enter a password."
                    };
                    workspace.as_mut().settle_operation();
                    workspace
                        .as_mut()
                        .request_archive_password(&signal_target, message);
                }
                Err(error) => workspace.as_mut().finish_operation(
                    "extract",
                    "",
                    Some(friendly_error(&error.to_string())),
                ),
            });
        });
    }

    fn transfer_entries(
        mut self: Pin<&mut Self>,
        paths: &QStringList,
        destination: &QString,
        cut: bool,
    ) {
        let sources = strings(paths);
        let destination = destination.to_string();
        if sources.is_empty() || destination.is_empty() {
            return;
        }
        // A cut is one-shot, but only for the list that was actually cut: a
        // move dragged in from Explorer must not discard an in-app cut
        // clipboard that happens to be unrelated.
        let clears_clipboard = cut
            && self.clipboard_mode.to_string() == "cut"
            && strings(&self.clipboard_paths) == sources;
        let request = TransferRequest {
            sources,
            destination,
            cut,
            resume: false,
            spends_clipboard: clears_clipboard,
        };
        self.as_mut().scheduler().submit(request);
        self.as_mut().publish_queue();
    }

    /// The core's job scheduler, created on first use: it needs a Qt thread
    /// to route its events back through, which only exists once QML calls in.
    fn scheduler(mut self: Pin<&mut Self>) -> Arc<JobScheduler> {
        if let Some(scheduler) = &self.transfer_scheduler {
            return Arc::clone(scheduler);
        }
        let thread = self.as_mut().qt_thread();
        let scheduler = JobScheduler::new(
            runtime::runtime().handle().clone(),
            host_for(&thread),
        );
        self.as_mut().rust_mut().transfer_scheduler = Some(Arc::clone(&scheduler));
        scheduler
    }

    /// Queues the "continue" of a cancelled copy: the same sources into the
    /// same destination, merged rather than checked for collisions.
    fn enqueue_resumed_transfer(
        mut self: Pin<&mut Self>,
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
        self.as_mut().scheduler().submit(request);
        self.as_mut().publish_queue();
    }

    fn publish_queue(mut self: Pin<&mut Self>) {
        let queued = self
            .transfer_scheduler
            .as_ref()
            .map(|scheduler| scheduler.queued())
            .unwrap_or_default();
        let labels: Vec<String> = queued.iter().map(transfer_label).collect();
        let paths: Vec<String> = queued.iter().map(|job| job.destination.clone()).collect();
        self.as_mut().set_queued_transfers(string_list(&labels));
        self.as_mut().set_queued_transfer_paths(string_list(&paths));
    }

    /// The queue panel's ✕. Only a waiting job can be dropped: a running one
    /// is cancelled through its operation id, which keeps the half-copied
    /// tree and the "continue" offer honest.
    fn cancel_queued_transfer(mut self: Pin<&mut Self>, index: i32) {
        if index >= 0
            && let Some(scheduler) = &self.transfer_scheduler
        {
            scheduler.drop_queued(index as usize);
        }
        self.as_mut().publish_queue();
    }

    fn cancel_queued_transfers(mut self: Pin<&mut Self>) {
        if let Some(scheduler) = &self.transfer_scheduler {
            scheduler.clear_queued();
        }
        self.as_mut().publish_queue();
    }

    /// Publishes the rows as the JSON list the panel reads, plus the combined
    /// throughput the chip prints. Called whenever a row appears, moves or
    /// goes away.
    fn publish_transfers(mut self: Pin<&mut Self>) {
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
        self.as_mut()
            .set_transfers_json(QString::from(json.as_str()));
        self.as_mut()
            .set_transfer_speed(QString::from(speed.as_str()));
    }

    /// One running transfer reported back: write its status-center row from
    /// the row's own provenance, free the busy count and drop the row. The
    /// global progress row belongs to the non-transfer operations, so it is
    /// left alone here.
    fn finish_transfer_row(
        mut self: Pin<&mut Self>,
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
        let row = self.as_mut().rust_mut().transfer_rows.remove(position);
        let kind = if row.request.cut { "move" } else { "copy" };

        let active = (self.active_operations - 1).max(0);
        self.as_mut().rust_mut().active_operations = active;
        self.as_mut().set_busy(active > 0);

        self.as_mut().record_operation_row(
            kind,
            error.as_deref(),
            cancelled,
            row.started_at,
            row.label.clone(),
            row.retry.clone(),
        );

        let kind_value = QString::from(kind);
        if cancelled {
            self.as_mut()
                .set_status(QString::from("Transfer cancelled"));
            self.as_mut()
                .operation_finished(&kind_value, &QString::default());
        } else if let Some(message) = error {
            self.as_mut()
                .set_last_error(QString::from(message.as_str()));
            let message_value = QString::from(message.as_str());
            self.as_mut().operation_failed(&kind_value, &message_value);
        } else {
            self.as_mut()
                .set_status(QString::from(success_message(kind)));
            self.as_mut()
                .operation_finished(&kind_value, &QString::default());
        }
        self.as_mut().publish_transfers();
    }

    fn pause_transfer(mut self: Pin<&mut Self>, job_id: i64) {
        let changed = {
            let mut rust = self.as_mut().rust_mut();
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
            self.as_mut().publish_transfers();
        }
    }

    fn resume_transfer(mut self: Pin<&mut Self>, job_id: i64) {
        let changed = {
            let mut rust = self.as_mut().rust_mut();
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
            self.as_mut().publish_transfers();
        }
    }

    fn cancel_transfer(mut self: Pin<&mut Self>, job_id: i64) {
        let rust = self.as_mut().rust_mut();
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

    fn apply_transfer_speed_limit(mut self: Pin<&mut Self>, kib_per_second: i32) {
        let kib = (kib_per_second.max(0) as u64).min(MAX_SPEED_LIMIT_KIB);

        // Applied first: the limit is in force for this session even when the
        // settings file cannot be written.
        service::set_transfer_speed_limit((kib > 0).then_some(kib * 1024));
        self.as_mut().set_transfer_speed_limit(kib as i32);

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
    fn elevate_operation(mut self: Pin<&mut Self>, id: &QString) {
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
            let mut rust = self.as_mut().rust_mut();
            rust.next_local_job += 1;
            rust.next_local_job
        };
        {
            let mut rust = self.as_mut().rust_mut();
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
        self.as_mut().set_busy(true);
        self.as_mut().publish_transfers();

        let thread = self.as_mut().qt_thread();
        let host = host_for(&thread);
        // Not `dispatch`: an elevated transfer owns its row directly (it never
        // went through the scheduler), so its completion settles the row too.
        runtime::spawn(async move {
            let outcome = match service::run_elevated(job, host, operation).await {
                Ok(()) => Outcome::empty(),
                Err(error) => Outcome::failed(error),
            };
            let _ = thread.queue(move |mut workspace| {
                workspace
                    .as_mut()
                    .finish_transfer_row(job_id, outcome.error, outcome.cancelled);
            });
        });
    }

    /// Renders one lifecycle event from the core's job scheduler. The rules
    /// (queue order, conflict answers, resume) live in `fs::jobs`; everything
    /// here is presentation plus the shell-owned clipboard and retry payload.
    /// Every event names its job, so several transfers can be on screen at
    /// once without their rows crossing.
    fn apply_job_event(mut self: Pin<&mut Self>, event: JobEvent) {
        match event {
            JobEvent::Accepted { job_id, request } => {
                self.as_mut()
                    .rust_mut()
                    .transfer_rows
                    .push(TransferRow::new(job_id, request));
                self.as_mut().publish_transfers();
                self.as_mut().publish_queue();
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
                    let mut rust = self.as_mut().rust_mut();
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
                self.as_mut().show_conflict_head();
            }
            JobEvent::Started {
                job_id,
                operation_id,
                request,
            } => {
                self.as_mut().forget_pending_conflict(job_id);
                // A copy clipboard stays available so the same selection can
                // be pasted into several directories; a cut is spent here,
                // once, as the move it was waiting for finally starts.
                if request.spends_clipboard {
                    self.as_mut().clear_clipboard();
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
                    let mut rust = self.as_mut().rust_mut();
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
                self.as_mut().rust_mut().active_operations = active;
                self.as_mut().set_busy(true);
                self.as_mut().publish_transfers();
            }
            JobEvent::Finished {
                job_id,
                error,
                cancelled,
                ..
            } => {
                self.as_mut().finish_transfer_row(
                    job_id,
                    error.map(|error| friendly_error(&error)),
                    cancelled,
                );
                self.as_mut().publish_queue();
            }
            JobEvent::CheckFailed { job_id, cut, error } => {
                // Nothing was copied and no busy count was taken: drop the
                // row and report the failure the way a copy would have, but
                // without `settle_operation` — decrementing there would clear
                // the progress row of whatever else is running.
                {
                    let mut rust = self.as_mut().rust_mut();
                    if let Some(position) = rust
                        .transfer_rows
                        .iter()
                        .position(|row| row.job_id == job_id)
                    {
                        rust.transfer_rows.remove(position);
                    }
                }
                let message = friendly_error(&error);
                self.as_mut().forget_pending_conflict(job_id);
                self.as_mut()
                    .set_last_error(QString::from(message.as_str()));
                let kind_value = QString::from(if cut { "move" } else { "copy" });
                let message_value = QString::from(message.as_str());
                self.as_mut().operation_failed(&kind_value, &message_value);
                self.as_mut().publish_transfers();
                self.as_mut().publish_queue();
            }
        }
    }

    fn clear_conflict(mut self: Pin<&mut Self>) {
        self.as_mut().set_conflict_job_id(0);
        self.as_mut().set_conflict_waiting(0);
        self.as_mut().set_conflict_index(0);
        self.as_mut().set_conflict_total(0);
        self.as_mut().set_conflict_name(QString::default());
        self.as_mut().set_conflict_cut(false);
        self.as_mut()
            .set_conflict_source_category(QString::default());
        self.as_mut().set_conflict_source_detail(QString::default());
        self.as_mut()
            .set_conflict_target_category(QString::default());
        self.as_mut().set_conflict_target_detail(QString::default());
    }

    /// Drops the prompt `job_id` is waiting on and repaints the dialog from
    /// the front of the list — the next job's question when one waits, an
    /// empty dialog when none does.
    fn forget_pending_conflict(mut self: Pin<&mut Self>, job_id: u64) {
        self.as_mut()
            .rust_mut()
            .pending_conflicts
            .retain(|prompt| prompt.job_id != job_id);
        self.as_mut().show_conflict_head();
    }

    /// Paints the dialog from the front of the pending list. A prompt that
    /// waited behind another job is already rendered; this only copies it
    /// onto the properties.
    fn show_conflict_head(mut self: Pin<&mut Self>) {
        let (head, waiting) = {
            let rust = self.as_mut().rust_mut();
            let waiting = rust.pending_conflicts.len().saturating_sub(1) as i32;
            (rust.pending_conflicts.first().cloned(), waiting)
        };
        let Some(head) = head else {
            self.as_mut().clear_conflict();
            return;
        };
        self.as_mut().set_conflict_waiting(waiting);
        self.as_mut().set_conflict_job_id(head.job_id as i64);
        self.as_mut().set_conflict_index(head.index as i32 + 1);
        self.as_mut().set_conflict_total(head.total as i32);
        self.as_mut()
            .set_conflict_name(QString::from(head.name.as_str()));
        self.as_mut().set_conflict_cut(head.cut);
        self.as_mut()
            .set_conflict_source_category(QString::from(head.source_category.as_str()));
        self.as_mut()
            .set_conflict_source_detail(QString::from(head.source_detail.as_str()));
        self.as_mut()
            .set_conflict_target_category(QString::from(head.target_category.as_str()));
        self.as_mut()
            .set_conflict_target_detail(QString::from(head.target_detail.as_str()));
    }

    /// Records the answer for the job's conflict on screen; the core moves
    /// to its next one or runs the batch. Its reply — the next prompt or
    /// `Started` — repaints the dialog, so nothing is removed here.
    fn resolve_conflict(
        mut self: Pin<&mut Self>,
        job_id: i64,
        action: &QString,
        apply_to_all: bool,
    ) {
        let Some(action) = conflict_action(&action.to_string()) else {
            return;
        };
        if job_id <= 0 {
            return;
        }
        self.as_mut()
            .scheduler()
            .answer(job_id as u64, action, apply_to_all);
    }

    /// Closes the question for that job and transfers nothing: the
    /// destination stays untouched, the job's row goes with it, and the
    /// queue advances. A prompt waiting behind moves onto the screen;
    /// running transfers are untouched — their cancel goes through the
    /// operation's control.
    fn cancel_conflict(mut self: Pin<&mut Self>, job_id: i64) {
        if job_id <= 0 {
            return;
        }
        let job_id = job_id as u64;
        {
            let mut rust = self.as_mut().rust_mut();
            // A job still in its pre-run phase — checking or asking — can
            // be dismissed; a running row is not this command's to touch.
            rust.transfer_rows
                .retain(|row| row.running || row.job_id != job_id);
            rust.pending_conflicts
                .retain(|prompt| prompt.job_id != job_id);
        }
        self.as_mut().scheduler().dismiss(job_id);
        self.as_mut().show_conflict_head();
        self.as_mut().publish_transfers();
        self.as_mut().publish_queue();
    }

    fn start_drag_out(self: Pin<&mut Self>, paths: &QStringList, mode: &QString,
                      signal: &QColor, count: i32, verdict: &QString,
                      ink: &QColor, canvas: &QColor) {
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
        if !qobject::start_file_drag(paths, actions, signal, count.max(1), verdict, ink, canvas) {
            log::warn!(target: "dae::workspace", "could not start the file drag");
        }
    }

    fn create_shortcuts(mut self: Pin<&mut Self>, paths: &QStringList, destination: &QString) {
        self.as_mut()
            .create_shortcuts_paths(strings(paths), destination.to_string());
    }

    fn create_shortcuts_paths(mut self: Pin<&mut Self>, paths: Vec<String>, destination: String) {
        if destination.is_empty() {
            return;
        }
        if !vfs::is_local_path(&destination) {
            self.as_mut().finish_operation(
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
            self.as_mut().finish_operation(
                "shortcut",
                "",
                Some("Shortcuts can only be created for local files".to_owned()),
            );
            return;
        }

        self.as_mut().begin_operation("Creating shortcut…");
        let thread = self.as_mut().qt_thread();
        dispatch(thread, "shortcut", async move {
            match service::create_shortcuts(sources, destination).await {
                Ok(created) => Outcome::ok(created.into_iter().next().unwrap_or_default()),
                Err(error) => Outcome::failed(error),
            }
        });
    }

    fn local_drop_paths(self: Pin<&mut Self>, urls: &QStringList) -> QStringList {
        let mut paths = QStringList::default();
        for raw in urls.iter() {
            let url = QUrl::from_user_input(&raw, &QString::default());
            if !url.is_local_file() {
                continue;
            }
            let path = url.to_local_file_or_default();
            if !path.as_slice().is_empty() {
                paths.append(path);
            }
        }
        paths
    }

    fn copy_text(self: Pin<&mut Self>, text: &QString) -> bool {
        let text = text.to_string();
        if text.is_empty() {
            return false;
        }
        // Deliberately quiet: "复制路径" and the checksum rows word their own
        // confirmation, and a toast from here would say the wrong one.
        qobject::set_clipboard_text(&QString::from(text.as_str()));
        true
    }

    fn rename_entries_batch(mut self: Pin<&mut Self>, paths: &QStringList, names: &QStringList) {
        let paths = strings(paths);
        let names = strings(names);
        if paths.is_empty() || paths.len() != names.len() {
            return;
        }
        let requests = paths
            .into_iter()
            .zip(names)
            .map(|(path, new_name)| dae_core::types::RenameRequest { path, new_name })
            .collect::<Vec<_>>();

        self.as_mut().begin_operation("Renaming…");
        let thread = self.as_mut().qt_thread();
        let host = host_for(&thread);
        dispatch(thread, "rename", async move {
            match service::rename_entries_batch(requests, operation_id(), host).await {
                Ok(renamed) => Outcome::ok(renamed.into_iter().next().unwrap_or_default()),
                Err(error) => Outcome::failed(error),
            }
        });
    }

    fn show_native_properties(self: Pin<&mut Self>, path: &QString) -> bool {
        let path = path.to_string();
        // A network path has no shell object to show, and opening the OS sheet
        // on a `smb://` string would only produce a shell error dialog.
        if path.is_empty() || !vfs::is_local_path(&path) {
            return false;
        }
        platform::show_properties(&path)
    }

    fn warm_shell_commands(self: Pin<&mut Self>) {
        runtime::spawn(async {
            if let Err(error) = dae_core::fs::shell_commands::warm_shell_commands().await {
                log::warn!(target: "dae::workspace", "shell-command warm-up failed: {error}");
            }
        });
    }

    fn invoke_shell_command(mut self: Pin<&mut Self>, id: &QString, paths: &QStringList) {
        let id = id.to_string();
        let paths = strings(paths);
        if id.is_empty() {
            return;
        }
        let thread = self.as_mut().qt_thread();
        let host = host_for(&thread);
        // Not a tracked operation: the launched app owns the result from here.
        // Only a failure to start comes back to dae, and it goes through the
        // same error toast every other command uses.
        runtime::spawn(async move {
            let result = dae_core::fs::shell_commands::invoke_shell_command(host, id, paths).await;
            if let Err(error) = result {
                let _ = thread.queue(move |mut workspace| {
                    let message = friendly_error(&error.to_string());
                    let kind = QString::from("shellCommand");
                    let message_value = QString::from(message.as_str());
                    workspace
                        .as_mut()
                        .set_last_error(QString::from(message.as_str()));
                    workspace.as_mut().operation_failed(&kind, &message_value);
                });
            }
        });
    }

    fn set_clipboard(mut self: Pin<&mut Self>, mode: &QString, paths: &QStringList) {
        let mode = mode.to_string();
        if paths.is_empty() {
            self.as_mut().clear_clipboard();
            return;
        }
        let paths = strings(paths);
        self.as_mut().set_clipboard_state(&mode, &paths);
        self.as_mut().rust_mut().system_clipboard_sequence = system_clipboard_sequence();

        // Only local paths mean anything to Explorer; remote entries stay in
        // the in-app mirror and are pasted by this shell alone.
        let local: Vec<String> = paths
            .into_iter()
            .filter(|path| vfs::is_local_path(path))
            .collect();
        self.as_mut().rust_mut().system_clipboard_paths = local.clone();
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

    fn clear_clipboard(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().clipboard_mode = QString::default();
        self.as_mut().rust_mut().clipboard_paths = QStringList::default();
        self.as_mut().rust_mut().clipboard_generation += 1;
    }

    /// Writes the mirror without touching the system clipboard.
    fn set_clipboard_state(mut self: Pin<&mut Self>, mode: &str, paths: &[String]) {
        self.as_mut().rust_mut().clipboard_mode = QString::from(mode);
        self.as_mut().rust_mut().clipboard_paths = string_list(paths);
        self.as_mut().rust_mut().clipboard_generation += 1;
    }

    fn paste_into(mut self: Pin<&mut Self>, destination: &QString) {
        let destination = destination.to_string();
        let own = self.system_clipboard_paths.clone();
        let sequence = self.system_clipboard_sequence;
        let mirror = strings(&self.clipboard_paths);
        let mirror_mode = self.clipboard_mode.to_string();
        let thread = self.as_mut().qt_thread();
        runtime::spawn(async move {
            let (system, current) = system_clipboard_snapshot().await;
            let contents =
                clipboard_contents(system, &own, &mirror, &mirror_mode, current == sequence);
            let _ = thread
                .queue(move |mut workspace| workspace.as_mut().apply_paste(contents, destination));
        });
    }

    fn apply_paste(mut self: Pin<&mut Self>, contents: ClipboardContents, destination: String) {
        match contents {
            ClipboardContents::Empty => {
                self.as_mut()
                    .finish_operation("copy", "", Some("Clipboard is empty".to_owned()))
            }
            ClipboardContents::Files {
                paths,
                effect: ClipboardEffect::Link,
            } => self.as_mut().create_shortcuts_paths(paths, destination),
            ClipboardContents::Files { paths, effect } => {
                let list = string_list(&paths);
                self.as_mut().transfer_entries(
                    &list,
                    &QString::from(destination),
                    effect == ClipboardEffect::Move,
                );
            }
        }
    }

    fn paste_shortcut_into(mut self: Pin<&mut Self>, destination: &QString) {
        let destination = destination.to_string();
        if destination.is_empty() {
            return;
        }
        let own = self.system_clipboard_paths.clone();
        let sequence = self.system_clipboard_sequence;
        let mirror = strings(&self.clipboard_paths);
        let mirror_mode = self.clipboard_mode.to_string();
        let thread = self.as_mut().qt_thread();
        runtime::spawn(async move {
            let (system, current) = system_clipboard_snapshot().await;
            let contents =
                clipboard_contents(system, &own, &mirror, &mirror_mode, current == sequence);
            let _ = thread.queue(move |mut workspace| match contents {
                ClipboardContents::Empty => workspace.as_mut().finish_operation(
                    "shortcut",
                    "",
                    Some("Clipboard is empty".to_owned()),
                ),
                ClipboardContents::Files { paths, .. } => workspace
                    .as_mut()
                    .create_shortcuts_paths(paths, destination),
            });
        });
    }

    fn refresh_clipboard(mut self: Pin<&mut Self>) {
        let generation = self.clipboard_generation;
        let own = self.system_clipboard_paths.clone();
        let thread = self.as_mut().qt_thread();
        runtime::spawn(async move {
            let (system, sequence) = match service::read_files_from_clipboard().await {
                Ok(system) => (system, system_clipboard_sequence()),
                Err(_) => return,
            };
            let _ = thread.queue(move |mut workspace| {
                // An in-app copy that happened after the read wins.
                if workspace.clipboard_generation != generation {
                    return;
                }
                workspace
                    .as_mut()
                    .apply_clipboard_refresh(system, sequence, &own);
            });
        });
    }

    fn apply_clipboard_refresh(
        mut self: Pin<&mut Self>,
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
                    self.as_mut().set_clipboard_state(mode, &files.paths);
                    self.as_mut().rust_mut().system_clipboard_sequence = sequence;
                }
            }
            _ => {
                if !own.is_empty() {
                    // The file list this shell placed was replaced by
                    // something else (text, another app's data).
                    self.as_mut().clear_clipboard();
                    self.as_mut().rust_mut().system_clipboard_paths.clear();
                    self.as_mut().rust_mut().system_clipboard_sequence = sequence;
                }
            }
        }
    }

    fn undo(mut self: Pin<&mut Self>) {
        self.as_mut().begin_operation("Undoing…");
        let thread = self.as_mut().qt_thread();
        let host = host_for(&thread);
        dispatch(thread, "undo", async move {
            match service::undo_operation(operation_id(), host).await {
                Ok(_) => Outcome::empty(),
                Err(error) => Outcome::failed(error),
            }
        });
    }

    fn redo(mut self: Pin<&mut Self>) {
        self.as_mut().begin_operation("Redoing…");
        let thread = self.as_mut().qt_thread();
        let host = host_for(&thread);
        dispatch(thread, "redo", async move {
            match service::redo_operation(operation_id(), host).await {
                Ok(_) => Outcome::empty(),
                Err(error) => Outcome::failed(error),
            }
        });
    }

    fn start_system_watch(self: Pin<&mut Self>) {
        crate::system_watch::start(self.qt_thread());
    }

    /// Applies an accent reading from the native watcher. `None` means the
    /// platform published no colour, and the current default is kept.
    pub(crate) fn apply_system_accent(mut self: Pin<&mut Self>, accent: Option<String>) {
        let Some(accent) = accent else {
            return;
        };
        if self.accent.to_string() == accent {
            return;
        }
        self.as_mut().set_accent(QString::from(accent.as_str()));
    }

    /// Applies a light/dark reading from the native watcher. `None` means the
    /// platform published no preference, and the current reading is kept.
    pub(crate) fn apply_system_theme(mut self: Pin<&mut Self>, dark: Option<bool>) {
        let Some(dark) = dark else {
            return;
        };
        if self.system_dark == dark {
            return;
        }
        self.as_mut().set_system_dark(dark);
    }

    pub(crate) fn notify_volumes_changed(mut self: Pin<&mut Self>) {
        self.as_mut().volumes_changed();
    }

    fn restore_trash_entries(mut self: Pin<&mut Self>, ids: &QStringList) {
        let ids = strings(ids);
        if ids.is_empty() {
            return;
        }
        self.as_mut().begin_operation("Restoring…");
        let thread = self.as_mut().qt_thread();
        let host = host_for(&thread);
        dispatch(thread, "restore_trash", async move {
            match trash_view::restore_trash_entries(ids, operation_id(), host).await {
                Ok(_) => Outcome::empty(),
                Err(error) => Outcome::failed(error),
            }
        });
    }

    fn delete_trash_entries(mut self: Pin<&mut Self>, ids: &QStringList) {
        let ids = strings(ids);
        if ids.is_empty() {
            return;
        }
        self.as_mut().begin_operation("Deleting permanently…");
        let thread = self.as_mut().qt_thread();
        let host = host_for(&thread);
        dispatch(thread, "purge_trash", async move {
            match trash_view::delete_trash_entries(ids, operation_id(), host).await {
                Ok(_) => Outcome::empty(),
                Err(error) => Outcome::failed(error),
            }
        });
    }

    fn empty_trash(mut self: Pin<&mut Self>) {
        self.as_mut().begin_operation("Emptying Trash…");
        let thread = self.as_mut().qt_thread();
        let host = host_for(&thread);
        dispatch(thread, "empty_trash", async move {
            match trash_view::empty_trash(operation_id(), host).await {
                Ok(_) => Outcome::empty(),
                Err(error) => Outcome::failed(error),
            }
        });
    }

    fn open_path(mut self: Pin<&mut Self>, path: &QString) {
        let path = path.to_string();
        if !vfs::is_local_path(&path) {
            self.as_mut().finish_operation(
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
            self.as_mut().finish_operation("open", "", Some(message));
            return;
        }
        self.as_mut()
            .record_recent(&QString::from(path.as_str()), false, true);
    }

    fn open_with(mut self: Pin<&mut Self>, path: &QString) {
        let path = path.to_string();
        self.as_mut().begin_operation("Opening…");
        let thread = self.as_mut().qt_thread();
        dispatch(thread, "openWith", async move {
            match service::open_with(path).await {
                Ok(()) => Outcome::empty(),
                Err(error) => Outcome::failed(error),
            }
        });
    }

    fn open_with_app(
        mut self: Pin<&mut Self>,
        path: &QString,
        app_id: &QString,
        set_default: bool,
    ) {
        let path = path.to_string();
        let app_id = app_id.to_string();
        if path.trim().is_empty() || app_id.trim().is_empty() {
            self.as_mut().finish_operation(
                "openWith",
                "",
                Some("Select an application".to_owned()),
            );
            return;
        }

        self.as_mut().begin_operation(if set_default {
            "Setting as default…"
        } else {
            "Opening…"
        });
        let thread = self.as_mut().qt_thread();
        dispatch(thread, "openWith", async move {
            match open_with::open_with_app(path, app_id, set_default).await {
                Ok(()) => Outcome::empty(),
                Err(error) => Outcome::failed(error),
            }
        });
    }

    fn open_terminal(mut self: Pin<&mut Self>, path: &QString) {
        let path = path.to_string();
        if !vfs::is_local_path(&path) {
            self.as_mut().finish_operation(
                "terminal",
                "",
                Some("Only local directories can be opened in Terminal".to_owned()),
            );
            return;
        }
        match service::open_terminal(path) {
            Ok(()) => self
                .as_mut()
                .set_status(QString::from("Opened in system terminal")),
            Err(error) => {
                let message = friendly_error(&error.to_string());
                self.as_mut()
                    .finish_operation("terminal", "", Some(message));
            }
        }
    }

    fn selection_size_text(&self, paths: &QStringList) -> QString {
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
            QString::from(crate::format::size(total).as_str())
        } else {
            QString::default()
        }
    }

    fn subdirectories(&self, directory: &QString) -> QStringList {
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
        let mut out = QStringList::default();
        for name in names {
            out.append(QString::from(name.as_str()));
        }
        out
    }

    fn recent_directories(&self) -> QStringList {
        let mut seen = std::collections::HashSet::new();
        let mut out = QStringList::default();
        for recent in dae_core::fs::recents::list_recents().unwrap_or_default() {
            if recent.kind != dae_core::types::EntryKind::Directory {
                continue;
            }
            if seen.insert(recent.path.clone()) {
                out.append(QString::from(recent.path.as_str()));
            }
            if out.len() >= 32 {
                break;
            }
        }
        out
    }

    fn expand_typed_path(&self, text: &QString) -> QString {
        QString::from(Self::expand_shorthand(&text.to_string()).as_str())
    }

/// Expands the typed-address shorthands. A leading `~` (alone or before a
/// separator) becomes the user's home; `%VAR%` and `$VAR`/`${VAR}` become
/// environment values. Anything that does not resolve stays as typed — the
/// navigation then reports the path in its own words, which is what
/// "keep what I typed" means.
fn expand_shorthand(text: &str) -> String {
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

    fn reveal(mut self: Pin<&mut Self>, path: &QString) {
        let path = path.to_string();
        if !vfs::is_local_path(&path) {
            self.as_mut().finish_operation(
                "reveal",
                "",
                Some("Remote items cannot be revealed in the system file manager".to_owned()),
            );
            return;
        }
        if let Err(message) = platform::reveal_path(&path) {
            self.as_mut().finish_operation("reveal", "", Some(message));
        }
    }

    fn record_recent(self: Pin<&mut Self>, path: &QString, is_directory: bool, opened: bool) {
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

    fn add_favorite(mut self: Pin<&mut Self>, path: &QString) {
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
                self.as_mut()
                    .set_status(QString::from("Added to favorites"));
                self.as_mut().favorites_changed();
                // A folder the user favorited belongs in the taskbar menu, even
                // if it was deleted from it earlier.
                crate::jump_list::reinclude(&path);
                crate::jump_list::refresh();
            }
            Err(error) => {
                let message = friendly_error(&error.to_string());
                self.as_mut()
                    .set_last_error(QString::from(message.as_str()));
            }
        }
    }

    fn add_favorite_paths(mut self: Pin<&mut Self>, paths: &QStringList) {
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
                self.as_mut()
                    .set_status(QString::from("Added to favorites"));
                self.as_mut().favorites_changed();
                for path in &added {
                    crate::jump_list::reinclude(path);
                }
                crate::jump_list::refresh();
            }
            Err(error) => {
                let message = friendly_error(&error.to_string());
                self.as_mut()
                    .set_last_error(QString::from(message.as_str()));
            }
        }
    }

    fn hide_place(mut self: Pin<&mut Self>, kind: &QString) {
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
                self.as_mut()
                    .set_status(QString::from("Hidden from sidebar"));
                self.as_mut().places_changed();
            }
            Err(error) => {
                let message = friendly_error(&error.to_string());
                self.as_mut()
                    .set_last_error(QString::from(message.as_str()));
            }
        }
    }

    fn show_all_places(mut self: Pin<&mut Self>) {
        match sidebar::save_hidden_places(Vec::new()) {
            Ok(()) => {
                self.as_mut()
                    .set_status(QString::from("Restored hidden places"));
                self.as_mut().places_changed();
            }
            Err(error) => {
                let message = friendly_error(&error.to_string());
                self.as_mut()
                    .set_last_error(QString::from(message.as_str()));
            }
        }
    }

    fn remove_favorite(mut self: Pin<&mut Self>, path: &QString) {
        let path = path.to_string();
        let favorites: Vec<Favorite> = sidebar::load_favorites()
            .unwrap_or_default()
            .into_iter()
            .filter(|favorite| favorite.path != path)
            .collect();
        match sidebar::save_favorites(favorites) {
            Ok(()) => {
                self.as_mut()
                    .set_status(QString::from("Removed from favorites"));
                self.as_mut().favorites_changed();
                crate::jump_list::refresh();
            }
            Err(error) => {
                let message = friendly_error(&error.to_string());
                self.as_mut()
                    .set_last_error(QString::from(message.as_str()));
            }
        }
    }

    fn test_connection(
        mut self: Pin<&mut Self>,
        protocol: &QString,
        host: &QString,
        port: &QString,
        username: &QString,
        password: &QString,
    ) {
        let input = match connection_input(protocol, host, port, username, password, false) {
            Ok(input) => input,
            Err(message) => {
                self.as_mut()
                    .connection_test_finished(false, &QString::from(message.as_str()));
                return;
            }
        };

        let thread = self.as_mut().qt_thread();
        runtime::spawn(async move {
            let result = smb::test_connection(input).await;
            let _ = thread.queue(move |mut workspace| match result {
                Ok(()) => workspace
                    .as_mut()
                    .connection_test_finished(true, &QString::from("Connection test succeeded")),
                Err(error) => {
                    let message = friendly_error(&error.to_string());
                    workspace
                        .as_mut()
                        .set_last_error(QString::from(message.as_str()));
                    workspace
                        .as_mut()
                        .connection_test_finished(false, &QString::from(message.as_str()));
                }
            });
        });
    }

    fn save_connection(
        mut self: Pin<&mut Self>,
        protocol: &QString,
        host: &QString,
        port: &QString,
        username: &QString,
        password: &QString,
        remember_password: bool,
    ) {
        let input =
            match connection_input(protocol, host, port, username, password, remember_password) {
                Ok(input) => input,
                Err(message) => {
                    self.as_mut().connection_save_finished(
                        false,
                        &QString::default(),
                        &QString::from(message.as_str()),
                    );
                    return;
                }
            };

        let thread = self.as_mut().qt_thread();
        runtime::spawn(async move {
            let result = tokio::task::spawn_blocking(move || connections::save_connection(input))
                .await
                .map_err(|error| FileSystemError::Internal(error.to_string()))
                .and_then(|result| result);
            let _ = thread.queue(move |mut workspace| match result {
                Ok(connection) => {
                    workspace.as_mut().connections_changed();
                    workspace.as_mut().connection_save_finished(
                        true,
                        &QString::from(connection.id.as_str()),
                        &QString::default(),
                    );
                }
                Err(error) => {
                    let message = friendly_error(&error.to_string());
                    workspace
                        .as_mut()
                        .set_last_error(QString::from(message.as_str()));
                    workspace.as_mut().connection_save_finished(
                        false,
                        &QString::default(),
                        &QString::from(message.as_str()),
                    );
                }
            });
        });
    }

    fn delete_connection(mut self: Pin<&mut Self>, id: &QString) {
        let id = id.to_string();
        if id.trim().is_empty() {
            return;
        }
        match connections::delete_connection(id) {
            Ok(()) => {
                self.as_mut()
                    .set_status(QString::from("Connection removed"));
                self.as_mut().connections_changed();
                let kind = QString::from("connection_delete");
                self.as_mut().operation_finished(&kind, &QString::default());
            }
            Err(error) => {
                let message = friendly_error(&error.to_string());
                self.as_mut()
                    .set_last_error(QString::from(message.as_str()));
                let kind = QString::from("connection_delete");
                self.as_mut()
                    .operation_failed(&kind, &QString::from(message.as_str()));
            }
        }
    }

    fn authorize_cloud_account(
        mut self: Pin<&mut Self>,
        provider: &QString,
        client_id: &QString,
        client_secret: &QString,
    ) {
        let provider = match CloudProviderKind::parse(provider.to_string().trim()) {
            Ok(provider) => provider,
            Err(error) => {
                let message = friendly_error(&error.to_string());
                self.as_mut()
                    .cloud_authorize_finished(false, &QString::from(message.as_str()));
                return;
            }
        };
        let client_id = client_id.to_string().trim().to_owned();
        if client_id.is_empty() {
            self.as_mut()
                .cloud_authorize_finished(false, &QString::from("Enter a Client ID"));
            return;
        }

        let input = AuthorizeCloudAccountInput {
            provider,
            client_id,
            client_secret: optional_string(client_secret),
        };
        let thread = self.as_mut().qt_thread();
        runtime::spawn(async move {
            let result = cloud_oauth::authorize_cloud_account(input).await;
            let _ = thread.queue(move |mut workspace| match result {
                Ok(account) => {
                    workspace.as_mut().connections_changed();
                    workspace
                        .as_mut()
                        .cloud_authorize_finished(true, &QString::from(account.id.as_str()));
                }
                Err(error) => {
                    let message = friendly_error(&error.to_string());
                    workspace
                        .as_mut()
                        .set_last_error(QString::from(message.as_str()));
                    workspace
                        .as_mut()
                        .cloud_authorize_finished(false, &QString::from(message.as_str()));
                }
            });
        });
    }

    fn delete_cloud_account(mut self: Pin<&mut Self>, id: &QString) {
        let id = id.to_string();
        if id.trim().is_empty() {
            return;
        }
        match cloud_accounts::delete_cloud_account(id) {
            Ok(()) => {
                self.as_mut()
                    .set_status(QString::from("Cloud account removed"));
                self.as_mut().connections_changed();
                let kind = QString::from("cloud_delete");
                self.as_mut().operation_finished(&kind, &QString::default());
            }
            Err(error) => {
                let message = friendly_error(&error.to_string());
                self.as_mut()
                    .set_last_error(QString::from(message.as_str()));
                let kind = QString::from("cloud_delete");
                self.as_mut()
                    .operation_failed(&kind, &QString::from(message.as_str()));
            }
        }
    }

    fn save_terminal_settings(
        mut self: Pin<&mut Self>,
        font_size: i32,
        line_height: f64,
        family: &QString,
    ) {
        let mut settings = dae_core::settings::load_settings().unwrap_or_default();
        settings.terminal.font_size = font_size.clamp(8, 24) as u8;
        settings.terminal.line_height = (line_height as f32).clamp(0.8, 3.0);
        let family = family.to_string();
        settings.terminal.font_family = (!family.trim().is_empty()).then_some(family);

        match dae_core::settings::save_settings(settings) {
            Ok(saved) => {
                self.as_mut()
                    .set_terminal_font_size(saved.terminal.font_size as i32);
                self.as_mut()
                    .set_terminal_line_height(saved.terminal.line_height as f64);
                self.as_mut().set_terminal_font_family(QString::from(
                    saved.terminal.font_family.as_deref().unwrap_or(""),
                ));
                self.as_mut()
                    .set_status(QString::from("Terminal settings saved"));
            }
            Err(error) => {
                let message = friendly_error(&error.to_string());
                self.as_mut()
                    .set_last_error(QString::from(message.as_str()));
            }
        }
    }

    fn refresh_default_manager(mut self: Pin<&mut Self>) {
        self.as_mut().start_default_manager(None);
    }

    fn set_shortcut(mut self: Pin<&mut Self>, id: &QString, binding: &QString) {
        let id = id.to_string();
        if !dae_core::settings::default_shortcuts().contains_key(&id) {
            self.as_mut()
                .set_last_error(QString::from("Unknown shortcut action"));
            return;
        }
        let binding = binding.to_string().trim().to_owned();
        if binding.len() > 64 || binding.chars().any(char::is_control) {
            self.as_mut()
                .set_last_error(QString::from("Invalid shortcut format"));
            return;
        }
        self.as_mut().apply_shortcut_change(move |shortcuts| {
            shortcuts.insert(id, binding);
        });
    }

    fn reset_shortcut(mut self: Pin<&mut Self>, id: &QString) {
        let id = id.to_string();
        let Some(binding) = dae_core::settings::default_shortcuts().get(&id).cloned() else {
            self.as_mut()
                .set_last_error(QString::from("Unknown shortcut action"));
            return;
        };
        self.as_mut().apply_shortcut_change(move |shortcuts| {
            shortcuts.insert(id, binding);
        });
    }

    fn reset_shortcuts(mut self: Pin<&mut Self>) {
        self.as_mut().apply_shortcut_change(|shortcuts| {
            *shortcuts = dae_core::settings::default_shortcuts();
        });
    }

    /// Saves only the shortcut map. Loading first preserves the terminal
    /// typography and the cached file-manager flag that share the document.
    fn apply_shortcut_change(
        mut self: Pin<&mut Self>,
        change: impl FnOnce(&mut HashMap<String, String>),
    ) {
        let mut settings = dae_core::settings::load_settings().unwrap_or_default();
        change(&mut settings.shortcuts);
        match dae_core::settings::save_settings(settings) {
            Ok(saved) => {
                self.as_mut()
                    .set_shortcuts_json(QString::from(shortcuts_json(&saved.shortcuts).as_str()));
                self.as_mut().set_status(QString::from("Shortcuts saved"));
                self.as_mut().set_last_error(QString::default());
            }
            Err(error) => {
                let message = friendly_error(&error.to_string());
                self.as_mut()
                    .set_last_error(QString::from(message.as_str()));
            }
        }
    }

    fn open_log_directory(mut self: Pin<&mut Self>) {
        let directory = crate::logging::app_log_directory();
        if let Err(error) = std::fs::create_dir_all(&directory) {
            self.as_mut().set_last_error(QString::from(
                format!("Could not create log directory: {error}").as_str(),
            ));
            return;
        }
        match platform::open_path(&directory.to_string_lossy()) {
            Ok(()) => self
                .as_mut()
                .set_status(QString::from("Opened log directory")),
            Err(message) => self
                .as_mut()
                .set_last_error(QString::from(message.as_str())),
        }
    }

    fn set_default_file_manager(mut self: Pin<&mut Self>) {
        self.as_mut().start_default_manager(Some(true));
    }

    fn unset_default_file_manager(mut self: Pin<&mut Self>) {
        self.as_mut().start_default_manager(Some(false));
    }

    /// `None` re-reads the OS state; `Some(true/false)` registers or removes
    /// dae first. The blocking platform calls run off the Qt thread, and the
    /// busy flag drops overlapping requests the same way the Git operations do.
    fn start_default_manager(mut self: Pin<&mut Self>, change: Option<bool>) {
        if self.default_manager_busy {
            return;
        }
        self.as_mut().set_default_manager_busy(true);
        self.as_mut().set_default_manager_error(QString::default());
        let thread = self.as_mut().qt_thread();
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
            let _ = thread.queue(move |mut workspace| {
                workspace.as_mut().adopt_default_manager(change, result)
            });
        });
    }

    fn adopt_default_manager(
        mut self: Pin<&mut Self>,
        change: Option<bool>,
        result: Result<DefaultManagerStatus, FileSystemError>,
    ) {
        self.as_mut().set_default_manager_busy(false);
        match result {
            Ok(status) => {
                self.as_mut()
                    .set_default_manager_supported(status.supported);
                self.as_mut()
                    .set_default_manager_is_default(status.is_default);
                self.as_mut()
                    .set_default_manager_is_registered(status.is_registered);
                let detail = status
                    .detail
                    .as_deref()
                    .map(default_manager_detail)
                    .unwrap_or("");
                self.as_mut()
                    .set_default_manager_detail(QString::from(detail));
                self.as_mut().set_default_manager_error(QString::default());
                if change.is_some() {
                    let _ = dae_core::settings::set_default_file_manager_cache(status.is_default);
                }
            }
            Err(error) => {
                self.as_mut().set_default_manager_detail(QString::default());
                let message = friendly_error(&error.to_string());
                self.as_mut()
                    .set_default_manager_error(QString::from(message.as_str()));
            }
        }
    }
}

fn strings(list: &QStringList) -> Vec<String> {
    list.iter().map(ToString::to_string).collect()
}

fn string_list(paths: &[String]) -> QStringList {
    let mut list = QStringList::default();
    for path in paths {
        list.append(QString::from(path.as_str()));
    }
    list
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
fn optional_string(value: &QString) -> Option<String> {
    let value = value.to_string();
    (!value.is_empty()).then_some(value)
}

/// Validates the typed fields and turns them into the core's save contract.
/// Both Test and Connect go through here so the two paths cannot disagree
/// about what a blank port or a padded host means.
fn connection_input(
    protocol: &QString,
    host: &QString,
    port: &QString,
    username: &QString,
    password: &QString,
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
        use qobject::Workspace;

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
            &QString::from("SMB"),
            &QString::from("  nas.local "),
            &QString::from(""),
            &QString::from(" alice "),
            &QString::from(""),
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
            &QString::from("sftp"),
            &QString::from("nas.local"),
            &QString::from("2222"),
            &QString::default(),
            &QString::from("secret"),
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
            &QString::from("smb"),
            &QString::from("   "),
            &QString::default(),
            &QString::default(),
            &QString::default(),
            false,
        )
        .expect_err("host is required");
        assert_eq!(missing_host, "Enter a server address");

        let bad_port = connection_input(
            &QString::from("smb"),
            &QString::from("nas.local"),
            &QString::from("70000"),
            &QString::default(),
            &QString::default(),
            false,
        )
        .expect_err("port is out of range");
        assert_eq!(bad_port, "Port must be between 1 and 65535");

        let unknown = connection_input(
            &QString::from("s3"),
            &QString::from("bucket"),
            &QString::default(),
            &QString::default(),
            &QString::default(),
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
