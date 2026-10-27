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
use std::sync::Arc;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QStringList, QUrl};
use dae_core::error::FileSystemError;
use dae_core::fs::archive::{self, ArchiveFormat};
use dae_core::fs::clipboard::{
    ClipboardEffect, SystemClipboardFiles, clipboard_sequence as system_clipboard_sequence,
};
use dae_core::fs::cloud::accounts as cloud_accounts;
use dae_core::fs::cloud::oauth::{self as cloud_oauth, AuthorizeCloudAccountInput};
use dae_core::fs::cloud::provider::CloudProviderKind;
use dae_core::fs::connections::{self, Protocol, SaveConnectionInput};
use dae_core::fs::open_with;
use dae_core::fs::recents::RecentSource;
use dae_core::fs::service;
use dae_core::fs::sidebar::{self, Favorite};
use dae_core::fs::smb;
use dae_core::fs::trash_view;
use dae_core::fs::types::{
    ConflictAction, EntryKind, NewEntryKind, TransferConflict, TransferItem,
    display_name_from_path,
};
use dae_core::fs::vfs;
use dae_core::host::{Event, Host};

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

        include!("drag.h");
        #[namespace = "dae_qt"]
        #[rust_name = "start_file_drag"]
        fn dae_qt_start_file_drag(paths: &QStringList, actions: i32) -> bool;

        include!("clipboard.h");
        #[namespace = "dae_qt"]
        #[rust_name = "set_clipboard_text"]
        fn dae_qt_set_clipboard_text(text: &QString);
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
        #[qproperty(QString, status)]
        #[qproperty(QString, last_error, cxx_name = "lastError")]
        #[qproperty(bool, busy)]
        #[qproperty(bool, can_undo, cxx_name = "canUndo")]
        #[qproperty(bool, can_redo, cxx_name = "canRedo")]
        #[qproperty(QString, progress_label, cxx_name = "progressLabel")]
        #[qproperty(i32, progress_done, cxx_name = "progressDone")]
        #[qproperty(i32, progress_total, cxx_name = "progressTotal")]
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
        /// Transfers waiting for the slot the running copy or move holds, one
        /// finished display line each (`复制 3 项 → dae`). The running one is
        /// not in the list: the progress row is where it shows.
        #[qproperty(QStringList, queued_transfers, cxx_name = "queuedTransfers")]
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
        /// True where the in-app picker is the only way to choose an
        /// application (macOS and Linux). Windows keeps the native
        /// `SHOpenWithDialog`, which the `openWith` command invokes.
        #[qproperty(bool, open_with_picker_supported, cxx_name = "openWithPickerSupported")]
        /// Live default-file-manager state for the settings panel. The panel
        /// reads fresh OS state on open and after every set/unset; `detail`
        /// names the manual step a platform still requires.
        #[qproperty(bool, default_manager_supported, cxx_name = "defaultManagerSupported")]
        #[qproperty(bool, default_manager_is_default, cxx_name = "defaultManagerIsDefault")]
        #[qproperty(bool, default_manager_is_registered, cxx_name = "defaultManagerIsRegistered")]
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

        /// Copies or moves `paths` into `destination`. `cut` selects move.
        #[qinvokable]
        #[cxx_name = "transferEntries"]
        fn transfer_entries(
            self: Pin<&mut Workspace>,
            paths: &QStringList,
            destination: &QString,
            cut: bool,
        );

        /// Answers the conflict the dialog is showing. `action` is `"replace"`,
        /// `"skip"` or `"keepBoth"`; `applyToAll` answers every conflict left in
        /// the batch the same way.
        #[qinvokable]
        #[cxx_name = "resolveConflict"]
        fn resolve_conflict(self: Pin<&mut Workspace>, action: &QString, apply_to_all: bool);

        /// Dismisses the conflict dialog and transfers nothing: not the item,
        /// not the rest of the batch.
        #[qinvokable]
        #[cxx_name = "cancelConflict"]
        fn cancel_conflict(self: Pin<&mut Workspace>);

        /// Drops the transfer waiting at `index` of `queuedTransfers`.
        #[qinvokable]
        #[cxx_name = "cancelQueuedTransfer"]
        fn cancel_queued_transfer(self: Pin<&mut Workspace>, index: i32);

        #[qinvokable]
        #[cxx_name = "cancelQueuedTransfers"]
        fn cancel_queued_transfers(self: Pin<&mut Workspace>);

        /// Starts a native drag carrying `paths` out of the window. `mode` is
        /// `"copy"`, `"move"`, `"link"` or `"any"`; `"any"` lets the drop
        /// target choose. Blocks until the user drops or cancels.
        #[qinvokable]
        #[cxx_name = "startDragOut"]
        fn start_drag_out(self: Pin<&mut Workspace>, paths: &QStringList, mode: &QString);

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
        fn invoke_shell_command(
            self: Pin<&mut Workspace>,
            id: &QString,
            paths: &QStringList,
        );

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

        #[qinvokable]
        #[cxx_name = "removeFavorite"]
        fn remove_favorite(self: Pin<&mut Workspace>, path: &QString);

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

pub struct WorkspaceRust {
    status: QString,
    last_error: QString,
    busy: bool,
    can_undo: bool,
    can_redo: bool,
    progress_label: QString,
    progress_done: i32,
    progress_total: i32,
    conflict_index: i32,
    conflict_total: i32,
    conflict_name: QString,
    conflict_cut: bool,
    conflict_source_category: QString,
    conflict_source_detail: QString,
    conflict_target_category: QString,
    conflict_target_detail: QString,
    queued_transfers: QStringList,
    /// Copies and moves waiting for the operation slot, oldest first. The one
    /// that owns the slot is not in here.
    queue: Vec<TransferJob>,
    /// The transfer holding the slot, from its conflict check until its
    /// copy/move reports back. `None` is what lets a new request start at once.
    active_transfer: Option<ActiveTransfer>,
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
        // Settings are a process-wide registry; initializing here means the
        // first QML window and every later one read the same document.
        let _ = dae_core::settings::init();
        // The connection and cloud registries are file/keychain backed; the
        // sidebar and the connect dialogs read them through this same store.
        crate::init_stores();
        let settings = dae_core::settings::load_settings().unwrap_or_default();
        let home = service::get_home_directory().unwrap_or_default();

        Self {
            status: QString::default(),
            last_error: QString::default(),
            busy: false,
            can_undo: false,
            can_redo: false,
            progress_label: QString::default(),
            progress_done: 0,
            progress_total: 0,
            conflict_index: 0,
            conflict_total: 0,
            conflict_name: QString::default(),
            conflict_cut: false,
            conflict_source_category: QString::default(),
            conflict_source_detail: QString::default(),
            conflict_target_category: QString::default(),
            conflict_target_detail: QString::default(),
            queued_transfers: QStringList::default(),
            queue: Vec::new(),
            active_transfer: None,
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
            log_directory: QString::from(app_log_directory().to_string_lossy().as_ref()),
            open_with_picker_supported: cfg!(any(target_os = "macos", target_os = "linux")),
            default_manager_supported: cfg!(any(
                target_os = "macos",
                target_os = "linux",
                windows
            )),
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
struct Outcome {
    focus: String,
    error: Option<String>,
}

impl Outcome {
    fn ok(focus: impl Into<String>) -> Self {
        Self {
            focus: focus.into(),
            error: None,
        }
    }

    fn empty() -> Self {
        Self::ok(String::new())
    }

    fn failed(error: FileSystemError) -> Self {
        Self {
            focus: String::new(),
            error: Some(friendly_error(&error.to_string())),
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
            workspace
                .as_mut()
                .finish_operation(kind, outcome.focus.as_str(), outcome.error);
        });
    });
}

/// [`dispatch`] for a copy or a move: the job holds the operation slot until it
/// reports back, so the queue is advanced here and nowhere else.
fn dispatch_transfer<F>(
    thread: cxx_qt::CxxQtThread<qobject::Workspace>,
    kind: &'static str,
    future: F,
) where
    F: std::future::Future<Output = Outcome> + Send + 'static,
{
    runtime::spawn(async move {
        let outcome = future.await;
        let _ = thread.queue(move |mut workspace| {
            workspace
                .as_mut()
                .finish_operation(kind, outcome.focus.as_str(), outcome.error);
            workspace.as_mut().end_transfer();
        });
    });
}

/// One copy or move, on its way through the queue.
struct TransferJob {
    sources: Vec<String>,
    destination: String,
    cut: bool,
    /// This job spends the in-app cut clipboard, so it is cleared when the
    /// transfer actually runs — not when it is requested. A conflict the user
    /// cancels must leave the cut on the clipboard for another try.
    clears_clipboard: bool,
    /// The line the queue panel prints, built once when the job is accepted so
    /// the panel formats nothing itself.
    label: String,
}

impl TransferJob {
    fn new(sources: Vec<String>, destination: String, cut: bool) -> Self {
        let label = format!(
            "{} {} 项 → {}",
            if cut { "移动" } else { "复制" },
            sources.len(),
            display_name_from_path(&destination)
        );
        Self {
            sources,
            destination,
            cut,
            clears_clipboard: false,
            label,
        }
    }

    /// Marks this job as the one that spends the cut clipboard.
    fn spending_clipboard(mut self) -> Self {
        self.clears_clipboard = true;
        self
    }
}

/// The transfer that owns the operation slot.
struct ActiveTransfer {
    job: TransferJob,
    /// Collisions found before transferring. Empty while the copy runs.
    conflicts: Vec<TransferConflict>,
    /// The conflict the dialog is showing; `conflicts.len()` means every
    /// decision is in.
    index: usize,
    /// Source path → resolution. A source the check found nothing for keeps
    /// `Fail`, which is the Tauri shell's rule as well: the dialog answers
    /// collisions that were on disk when the batch was checked, and one that
    /// appears between the check and the copy is a race, not a choice.
    decisions: HashMap<String, ConflictAction>,
}

impl ActiveTransfer {
    /// Records `action` for the conflict on screen and steps to the next one.
    /// Returns true when every conflict has an answer, which is the moment the
    /// batch can finally run.
    ///
    /// Kept apart from the Qt object so the decision rules — one at a time,
    /// "apply to all", the last answer — are testable without a QObject.
    fn answer(&mut self, action: ConflictAction, apply_to_all: bool) -> bool {
        let Some(conflict) = self.conflicts.get(self.index) else {
            return true;
        };
        self.decisions.insert(conflict.source_path.clone(), action);
        if apply_to_all {
            for rest in &self.conflicts[self.index + 1..] {
                self.decisions.insert(rest.source_path.clone(), action);
            }
            self.index = self.conflicts.len();
        } else {
            self.index += 1;
        }
        self.index >= self.conflicts.len()
    }
}

/// Drops the job waiting at `index`. Out-of-range is a no-op: the panel can be
/// one frame behind a queue that already moved on.
fn drop_queued(queue: &mut Vec<TransferJob>, index: usize) {
    if index < queue.len() {
        queue.remove(index);
    }
}

/// Removes and returns the oldest waiting job.
fn take_next_queued(queue: &mut Vec<TransferJob>) -> Option<TransferJob> {
    if queue.is_empty() {
        None
    } else {
        Some(queue.remove(0))
    }
}

impl qobject::Workspace {
    fn begin_operation(mut self: Pin<&mut Self>, label: &str) {
        let active = self.active_operations + 1;
        self.as_mut().rust_mut().active_operations = active;
        self.as_mut().set_busy(true);
        self.as_mut().set_progress_done(0);
        self.as_mut().set_progress_total(0);
        self.as_mut().set_progress_label(QString::from(label));
        self.as_mut().set_last_error(QString::default());
    }

    fn finish_operation(mut self: Pin<&mut Self>, kind: &str, focus: &str, error: Option<String>) {
        self.as_mut().settle_operation();

        let kind_value = QString::from(kind);
        let focus_value = QString::from(focus);
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

    /// Clears the progress row without reporting an outcome. A protected
    /// archive uses this before asking for a password: nothing failed yet, and
    /// an `operationFinished` would wrongly say the extraction completed.
    fn settle_operation(mut self: Pin<&mut Self>) {
        let active = (self.active_operations - 1).max(0);
        self.as_mut().rust_mut().active_operations = active;
        self.as_mut().set_busy(active > 0);
        self.as_mut().set_progress_done(0);
        self.as_mut().set_progress_total(0);
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
                operation,
                phase,
                completed,
                total,
                current_path,
                ..
            } => {
                let label = current_path
                    .as_deref()
                    .map(display_name_from_path)
                    .unwrap_or_else(|| operation_label(operation).to_owned());
                self.as_mut()
                    .set_progress_label(QString::from(label.as_str()));
                self.as_mut()
                    .set_progress_done(completed.min(i32::MAX as u64) as i32);
                let total = total.unwrap_or(0).min(i32::MAX as u64) as i32;
                self.as_mut().set_progress_total(total);
                if matches!(phase, dae_core::progress::FileOperationPhase::Preparing) {
                    self.as_mut().set_busy(true);
                }
            }
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
            self.as_mut()
                .finish_operation(operation_kind, "", Some("名称不能为空".to_owned()));
            return;
        }

        self.as_mut().begin_operation("新建中…");
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
                .finish_operation("rename", "", Some("名称不能为空".to_owned()));
            return;
        }

        self.as_mut().begin_operation("重命名中…");
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

    fn trash_entries(mut self: Pin<&mut Self>, paths: &QStringList) {
        let paths = strings(paths);
        if paths.is_empty() {
            return;
        }
        self.as_mut().begin_operation("移到回收站…");
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
        self.as_mut().begin_operation("删除中…");
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
        self.as_mut().begin_operation("创建副本…");
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
            self.as_mut()
                .finish_operation("compress", "", Some("请选择要压缩的项目".to_owned()));
            return;
        }

        let destination_dir = destination_dir.to_string();
        if destination_dir.is_empty() {
            self.as_mut().finish_operation(
                "compress",
                "",
                Some("无法确定压缩包保存位置".to_owned()),
            );
            return;
        }

        let Some(format) = archive_format(&format.to_string()) else {
            self.as_mut()
                .finish_operation("compress", "", Some("不支持的压缩格式".to_owned()));
            return;
        };

        let password = optional_string(password);
        self.as_mut().begin_operation("压缩中…");
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
        self.as_mut().begin_operation("解压中…");
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
                        "密码不正确，请重新输入"
                    } else {
                        "压缩包已加密，请输入密码"
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
        let mut job = TransferJob::new(sources, destination, cut);
        if clears_clipboard {
            job = job.spending_clipboard();
        }
        self.as_mut().enqueue_transfer(job);
    }

    /// Takes a transfer into the queue, or starts it when the slot is free.
    ///
    /// The queue is serial because the shell reports one operation at a time on
    /// one progress row: two copies running at once would make that row show
    /// whichever event arrived last and neither job's total.
    fn enqueue_transfer(mut self: Pin<&mut Self>, job: TransferJob) {
        if self.active_transfer.is_some() {
            self.as_mut().rust_mut().queue.push(job);
            self.as_mut().publish_queue();
            return;
        }
        self.as_mut().start_transfer(job);
    }

    fn publish_queue(mut self: Pin<&mut Self>) {
        let labels: Vec<String> = self.queue.iter().map(|job| job.label.clone()).collect();
        self.as_mut().set_queued_transfers(string_list(&labels));
    }

    /// The queue panel's ✕. Only a waiting job can be dropped: the core has no
    /// cancellation seam for one already running, and pretending otherwise
    /// would leave a half-copied folder behind.
    fn cancel_queued_transfer(mut self: Pin<&mut Self>, index: i32) {
        let mut queue = std::mem::take(&mut self.as_mut().rust_mut().queue);
        if index >= 0 {
            drop_queued(&mut queue, index as usize);
        }
        self.as_mut().rust_mut().queue = queue;
        self.as_mut().publish_queue();
    }

    fn cancel_queued_transfers(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().queue.clear();
        self.as_mut().publish_queue();
    }

    /// Gives `job` the operation slot and asks the core what would collide.
    fn start_transfer(mut self: Pin<&mut Self>, job: TransferJob) {
        let sources = job.sources.clone();
        let destination = job.destination.clone();
        self.as_mut().rust_mut().active_transfer = Some(ActiveTransfer {
            job,
            conflicts: Vec::new(),
            index: 0,
            decisions: HashMap::new(),
        });
        let thread = self.as_mut().qt_thread();
        runtime::spawn(async move {
            let result = service::check_transfer_conflicts(sources, destination).await;
            let _ = thread.queue(move |mut workspace| workspace.as_mut().adopt_conflicts(result));
        });
    }

    /// Decides what the check found: copy straight away, or hand the collisions
    /// to the dialog.
    fn adopt_conflicts(
        mut self: Pin<&mut Self>,
        result: Result<Vec<TransferConflict>, FileSystemError>,
    ) {
        if self.active_transfer.is_none() {
            return;
        }
        match result {
            Ok(conflicts) if conflicts.is_empty() => self.as_mut().run_transfer(),
            Ok(conflicts) => {
                if let Some(active) = self.as_mut().rust_mut().active_transfer.as_mut() {
                    active.conflicts = conflicts;
                    active.index = 0;
                }
                self.as_mut().publish_conflict();
            }
            Err(error) => {
                // Nothing was copied: report the failure the way a copy would
                // have, but without `settle_operation` — this transfer never
                // began one, and decrementing there would clear the progress
                // row of whatever else is running.
                let kind = self.as_mut().transfer_kind();
                let message = friendly_error(&error.to_string());
                self.as_mut().rust_mut().active_transfer = None;
                self.as_mut().clear_conflict();
                self.as_mut().set_last_error(QString::from(message.as_str()));
                let kind_value = QString::from(kind);
                let message_value = QString::from(message.as_str());
                self.as_mut().operation_failed(&kind_value, &message_value);
                self.as_mut().start_next_queued();
            }
        }
    }

    fn transfer_kind(&self) -> &'static str {
        match self.active_transfer.as_ref() {
            Some(active) if active.job.cut => "move",
            _ => "copy",
        }
    }

    /// Publishes the conflict the dialog is on. Called whenever the index moves
    /// and by nothing else, so the dialog's eight properties always describe
    /// one conflict.
    fn publish_conflict(mut self: Pin<&mut Self>) {
        let Some(active) = self.active_transfer.as_ref() else {
            self.as_mut().clear_conflict();
            return;
        };
        let total = active.conflicts.len();
        let Some(conflict) = active.conflicts.get(active.index) else {
            self.as_mut().clear_conflict();
            return;
        };
        // Copied out before the setters run: they borrow `self` mutably.
        let index = active.index as i32 + 1;
        let cut = active.job.cut;
        let name = conflict.name.clone();
        let source_category = glyph_category(conflict.source_kind).to_owned();
        let source_detail = conflict_detail(
            conflict.source_kind,
            &conflict.name,
            conflict.source_size,
            conflict.source_modified_at,
        );
        let target_category = glyph_category(conflict.target_kind).to_owned();
        let target_detail = conflict_detail(
            conflict.target_kind,
            &conflict.name,
            conflict.target_size,
            conflict.target_modified_at,
        );

        self.as_mut().set_conflict_total(total as i32);
        self.as_mut().set_conflict_index(index);
        self.as_mut().set_conflict_name(QString::from(name.as_str()));
        self.as_mut().set_conflict_cut(cut);
        self.as_mut()
            .set_conflict_source_category(QString::from(source_category.as_str()));
        self.as_mut()
            .set_conflict_source_detail(QString::from(source_detail.as_str()));
        self.as_mut()
            .set_conflict_target_category(QString::from(target_category.as_str()));
        self.as_mut()
            .set_conflict_target_detail(QString::from(target_detail.as_str()));
    }

    fn clear_conflict(mut self: Pin<&mut Self>) {
        self.as_mut().set_conflict_index(0);
        self.as_mut().set_conflict_total(0);
        self.as_mut().set_conflict_name(QString::default());
        self.as_mut().set_conflict_cut(false);
        self.as_mut()
            .set_conflict_source_category(QString::default());
        self.as_mut()
            .set_conflict_source_detail(QString::default());
        self.as_mut()
            .set_conflict_target_category(QString::default());
        self.as_mut()
            .set_conflict_target_detail(QString::default());
    }

    /// Records the answer for the conflict on screen and either moves to the
    /// next one or runs the batch.
    fn resolve_conflict(mut self: Pin<&mut Self>, action: &QString, apply_to_all: bool) {
        let Some(action) = conflict_action(&action.to_string()) else {
            return;
        };
        let Some(mut active) = self.as_mut().rust_mut().active_transfer.take() else {
            return;
        };
        let answered = active.answer(action, apply_to_all);
        self.as_mut().rust_mut().active_transfer = Some(active);
        if answered {
            self.as_mut().run_transfer();
        } else {
            self.as_mut().publish_conflict();
        }
    }

    /// Closes the dialog and transfers nothing: the Tauri shell's cancel did
    /// the same, leaving the destination untouched.
    fn cancel_conflict(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().active_transfer = None;
        self.as_mut().clear_conflict();
        self.as_mut().start_next_queued();
    }

    /// Runs the batch that holds the slot with the decisions collected so far.
    fn run_transfer(mut self: Pin<&mut Self>) {
        let Some(active) = self.as_mut().rust_mut().active_transfer.take() else {
            return;
        };
        self.as_mut().clear_conflict();

        let items = active
            .job
            .sources
            .iter()
            .map(|path| TransferItem {
                path: path.clone(),
                on_conflict: active
                    .decisions
                    .get(path)
                    .copied()
                    .unwrap_or(ConflictAction::Fail),
            })
            .collect::<Vec<_>>();
        let cut = active.job.cut;
        let destination = active.job.destination.clone();
        let kind = if cut { "move" } else { "copy" };

        // A copy clipboard stays available so the same selection can be pasted
        // into several directories; a cut is spent here, once, as the move it
        // was waiting for finally starts.
        if active.job.clears_clipboard {
            self.as_mut().clear_clipboard();
        }

        // The job keeps the slot while the copy runs, so a transfer asked for
        // now queues behind it instead of sharing the progress row.
        self.as_mut().rust_mut().active_transfer = Some(ActiveTransfer {
            job: active.job,
            conflicts: Vec::new(),
            index: 0,
            decisions: HashMap::new(),
        });
        self.as_mut()
            .begin_operation(if cut { "移动中…" } else { "复制中…" });

        let thread = self.as_mut().qt_thread();
        let host = host_for(&thread);
        dispatch_transfer(thread, kind, async move {
            let result = if cut {
                service::move_entries(items, destination, operation_id(), host).await
            } else {
                service::copy_entries(items, destination, operation_id(), host).await
            };
            match result {
                Ok(()) => Outcome::empty(),
                Err(error) => Outcome::failed(error),
            }
        });
    }

    /// Frees the slot and starts whatever was waiting behind it.
    fn end_transfer(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().active_transfer = None;
        self.as_mut().start_next_queued();
    }

    fn start_next_queued(mut self: Pin<&mut Self>) {
        let mut queue = std::mem::take(&mut self.as_mut().rust_mut().queue);
        let next = take_next_queued(&mut queue);
        self.as_mut().rust_mut().queue = queue;
        self.as_mut().publish_queue();
        if let Some(job) = next {
            self.as_mut().start_transfer(job);
        }
    }

    fn start_drag_out(self: Pin<&mut Self>, paths: &QStringList, mode: &QString) {
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
        if !qobject::start_file_drag(paths, actions) {
            eprintln!("dae: could not start the file drag");
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
                Some("只能在本地文件夹创建快捷方式".to_owned()),
            );
            return;
        }

        // Shortcuts are a local-only operation; a remote source would be
        // written into a .lnk that cannot resolve.
        let sources: Vec<String> = paths
            .into_iter()
            .filter(|path| vfs::is_local_path(path))
            .collect();
        if sources.is_empty() {
            self.as_mut().finish_operation(
                "shortcut",
                "",
                Some("只能为本地文件创建快捷方式".to_owned()),
            );
            return;
        }

        self.as_mut().begin_operation("创建快捷方式…");
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

    fn rename_entries_batch(
        mut self: Pin<&mut Self>,
        paths: &QStringList,
        names: &QStringList,
    ) {
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

        self.as_mut().begin_operation("重命名中…");
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
                eprintln!("dae: shell-command warm-up failed: {error}");
            }
        });
    }

    fn invoke_shell_command(
        mut self: Pin<&mut Self>,
        id: &QString,
        paths: &QStringList,
    ) {
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
                    workspace
                        .as_mut()
                        .operation_failed(&kind, &message_value);
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
                eprintln!("dae: could not place the selection on the system clipboard: {error}");
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
                    .finish_operation("copy", "", Some("剪贴板是空的".to_owned()))
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
                    Some("剪贴板是空的".to_owned()),
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
        self.as_mut().begin_operation("撤销中…");
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
        self.as_mut().begin_operation("重做中…");
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
        self.as_mut().begin_operation("恢复中…");
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
        self.as_mut().begin_operation("永久删除中…");
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
        self.as_mut().begin_operation("清空回收站…");
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
                Some("远程文件需要用本地应用打开前，请先复制到本地文件夹".to_owned()),
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
        self.as_mut().begin_operation("正在打开…");
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
            self.as_mut()
                .finish_operation("openWith", "", Some("请选择一个应用".to_owned()));
            return;
        }

        self.as_mut().begin_operation(if set_default {
            "正在设为默认打开方式…"
        } else {
            "正在打开…"
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
            self.as_mut()
                .finish_operation("terminal", "", Some("终端只能打开本地目录".to_owned()));
            return;
        }
        match service::open_terminal(path) {
            Ok(()) => self
                .as_mut()
                .set_status(QString::from("已在系统终端中打开")),
            Err(error) => {
                let message = friendly_error(&error.to_string());
                self.as_mut()
                    .finish_operation("terminal", "", Some(message));
            }
        }
    }

    fn reveal(mut self: Pin<&mut Self>, path: &QString) {
        let path = path.to_string();
        if !vfs::is_local_path(&path) {
            self.as_mut().finish_operation(
                "reveal",
                "",
                Some("远程项目无法在系统资源管理器中定位".to_owned()),
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
            path,
        });
        match sidebar::save_favorites(favorites) {
            Ok(()) => {
                self.as_mut().set_status(QString::from("已加入收藏"));
                self.as_mut().favorites_changed();
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
                self.as_mut().set_status(QString::from("已从收藏移除"));
                self.as_mut().favorites_changed();
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
                    .connection_test_finished(true, &QString::from("连接测试成功")),
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
                self.as_mut().set_status(QString::from("已移除连接"));
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
                .cloud_authorize_finished(false, &QString::from("请输入 Client ID"));
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
                self.as_mut().set_status(QString::from("已移除云盘账户"));
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
                self.as_mut().set_status(QString::from("终端设置已保存"));
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
                .set_last_error(QString::from("未知的快捷键操作"));
            return;
        }
        let binding = binding.to_string().trim().to_owned();
        if binding.len() > 64 || binding.chars().any(char::is_control) {
            self.as_mut()
                .set_last_error(QString::from("快捷键格式无效"));
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
                .set_last_error(QString::from("未知的快捷键操作"));
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
                self.as_mut().set_shortcuts_json(QString::from(
                    shortcuts_json(&saved.shortcuts).as_str(),
                ));
                self.as_mut()
                    .set_status(QString::from("快捷键已保存"));
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
        let directory = app_log_directory();
        if let Err(error) = std::fs::create_dir_all(&directory) {
            self.as_mut().set_last_error(QString::from(
                format!("无法创建日志目录：{error}").as_str(),
            ));
            return;
        }
        match platform::open_path(&directory.to_string_lossy()) {
            Ok(()) => self.as_mut().set_status(QString::from("已打开日志目录")),
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
                FileSystemError::Internal(format!(
                    "default_manager.status_failed: {error}"
                ))
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
                self.as_mut()
                    .set_default_manager_error(QString::default());
                if change.is_some() {
                    let _ = dae_core::settings::set_default_file_manager_cache(
                        status.is_default,
                    );
                }
            }
            Err(error) => {
                self.as_mut()
                    .set_default_manager_detail(QString::default());
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
        return Err("请输入服务器地址".into());
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
        other => Err(format!("不支持的连接协议：{other}")),
    }
}

fn parse_port(value: &str) -> Result<Option<u16>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    match value.parse::<u16>() {
        Ok(port) if port > 0 => Ok(Some(port)),
        _ => Err("端口必须是 1-65535".into()),
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
        EntryKind::Directory => "文件夹".to_owned(),
        EntryKind::Symlink => "符号链接".to_owned(),
        EntryKind::Other => "其他".to_owned(),
        EntryKind::File => {
            let extension = name
                .rsplit_once('.')
                .map(|(_, extension)| extension)
                .unwrap_or("");
            if extension.is_empty() {
                "文件".to_owned()
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
    format!("{} · {}\n修改于 {}", kind_label(kind, name), size, modified)
}

fn operation_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!("qt-op-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

fn success_message(kind: &str) -> &'static str {
    match kind {
        "create" => "已创建",
        "rename" => "已重命名",
        "trash" => "已移到回收站",
        "delete" => "已永久删除",
        "duplicate" => "已创建副本",
        "shortcut" => "已创建快捷方式",
        "copy" => "已复制",
        "move" => "已移动",
        "compress" => "已压缩",
        "extract" => "已解压",
        "undo" => "已撤销",
        "redo" => "已重做",
        "openWith" => "已打开",
        "restore_trash" => "已恢复",
        "purge_trash" => "已永久删除",
        "empty_trash" => "已清空回收站",
        _ => "完成",
    }
}

fn operation_label(operation: dae_core::progress::FileOperationKind) -> &'static str {
    use dae_core::progress::FileOperationKind;
    match operation {
        FileOperationKind::Copy => "复制中…",
        FileOperationKind::Move => "移动中…",
        FileOperationKind::Delete => "删除中…",
        FileOperationKind::Compress => "压缩中…",
        FileOperationKind::Extract => "解压中…",
        FileOperationKind::Properties => "计算中…",
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
        push_json_string(&mut json, shortcuts.get(id).map(String::as_str).unwrap_or(""));
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

/// The directory the log page opens. It follows the Tauri shell's
/// `appLogDir` convention (identifier-scoped, one `logs` folder) so both
/// shells point at the same place.
fn app_log_directory() -> std::path::PathBuf {
    #[cfg(target_os = "windows")]
    if let Some(base) = std::env::var_os("LOCALAPPDATA") {
        return std::path::PathBuf::from(base)
            .join(dae_core::paths::APP_IDENTIFIER)
            .join("logs");
    }

    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        return std::path::PathBuf::from(home)
            .join("Library")
            .join("Logs")
            .join(dae_core::paths::APP_IDENTIFIER);
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let base = std::env::var_os("XDG_DATA_HOME")
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|home| std::path::PathBuf::from(home).join(".local").join("share"))
            });
        if let Some(base) = base {
            return base
                .join(dae_core::paths::APP_IDENTIFIER)
                .join("logs");
        }
    }

    dae_core::paths::config_dir()
        .map(|directory| directory.join("logs"))
        .unwrap_or_else(|_| std::env::temp_dir().join("dae-logs"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A transfer conflict for `path`, with both sides looking like plain
    /// files: the decision rules do not read the metadata.
    fn conflict(path: &str) -> TransferConflict {
        TransferConflict {
            source_path: path.to_owned(),
            target_path: path.to_owned(),
            name: display_name_from_path(path),
            source_kind: EntryKind::File,
            source_size: Some(1),
            source_modified_at: Some(1_000),
            target_kind: EntryKind::File,
            target_size: Some(2),
            target_modified_at: Some(2_000),
        }
    }

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
    fn a_conflict_batch_waits_for_every_answer_unless_applied_to_all() {
        let mut active = ActiveTransfer {
            job: TransferJob::new(vec!["C:/a".into(), "C:/b".into(), "C:/c".into()], "C:/dst".into(), false),
            conflicts: vec![conflict("C:/a"), conflict("C:/b"), conflict("C:/c")],
            index: 0,
            decisions: HashMap::new(),
        };

        // One answer at a time: the batch runs only after the third.
        assert!(!active.answer(ConflictAction::Skip, false));
        assert_eq!(active.index, 1);
        assert_eq!(active.decisions.get("C:/a"), Some(&ConflictAction::Skip));
        assert!(!active.answer(ConflictAction::KeepBoth, false));
        assert!(active.answer(ConflictAction::Replace, false));
        assert_eq!(active.decisions.len(), 3);
        assert_eq!(active.decisions.get("C:/c"), Some(&ConflictAction::Replace));
    }

    #[test]
    fn apply_to_all_answers_the_rest_with_the_same_action() {
        let mut active = ActiveTransfer {
            job: TransferJob::new(vec!["C:/a".into(), "C:/b".into()], "C:/dst".into(), false),
            conflicts: vec![conflict("C:/a"), conflict("C:/b"), conflict("C:/c")],
            index: 1,
            decisions: HashMap::new(),
        };
        active
            .decisions
            .insert("C:/a".into(), ConflictAction::Skip);

        assert!(active.answer(ConflictAction::Replace, true));
        assert_eq!(active.decisions.get("C:/b"), Some(&ConflictAction::Replace));
        assert_eq!(active.decisions.get("C:/c"), Some(&ConflictAction::Replace));
        // The one already answered keeps its own answer.
        assert_eq!(active.decisions.get("C:/a"), Some(&ConflictAction::Skip));
    }

    #[test]
    fn a_queued_transfer_waits_its_turn_in_order() {
        let mut queue = Vec::new();
        queue.push(TransferJob::new(vec!["C:/a".into()], "C:/one".into(), false));
        queue.push(TransferJob::new(vec!["C:/b".into()], "C:/two".into(), true));

        assert_eq!(take_next_queued(&mut queue).map(|job| job.destination), Some("C:/one".to_string()));
        assert_eq!(take_next_queued(&mut queue).map(|job| job.destination), Some("C:/two".to_string()));
        assert!(take_next_queued(&mut queue).is_none());
    }

    #[test]
    fn dropping_a_queued_transfer_leaves_the_rest_and_ignores_a_stale_index() {
        let mut queue = Vec::new();
        queue.push(TransferJob::new(vec!["C:/a".into()], "C:/one".into(), false));
        queue.push(TransferJob::new(vec!["C:/b".into()], "C:/two".into(), false));

        drop_queued(&mut queue, 7);
        assert_eq!(queue.len(), 2);
        drop_queued(&mut queue, 0);
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].destination, "C:/two");
    }

    #[test]
    fn a_queued_transfer_names_its_operation_and_target() {
        let copy = TransferJob::new(vec!["C:/a".into(), "C:/b".into()], "C:/dst".into(), false);
        assert_eq!(copy.label, "复制 2 项 → dst");
        let cut = TransferJob::new(vec!["C:/a".into()], "C:/dst".into(), true);
        assert_eq!(cut.label, "移动 1 项 → dst");
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
        assert_eq!(kind_label(EntryKind::Directory, "docs"), "文件夹");
        assert_eq!(kind_label(EntryKind::File, "notes.txt"), "TXT");
        assert_eq!(kind_label(EntryKind::File, "README"), "文件");
    }

    #[test]
    fn a_conflict_card_carries_both_caption_lines() {
        let detail = conflict_detail(EntryKind::File, "a.bin", Some(2048), Some(1_792_886_400_000));
        assert_eq!(detail, "BIN · 2.0 KB\n修改于 2026-10-25 00:00");

        // A folder has no size and a remote entry may have no timestamp; both
        // read as an em dash rather than as a zero.
        let detail = conflict_detail(EntryKind::Directory, "docs", None, None);
        assert_eq!(detail, "文件夹 · —\n修改于 —");
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
        assert_eq!(missing_host, "请输入服务器地址");

        let bad_port = connection_input(
            &QString::from("smb"),
            &QString::from("nas.local"),
            &QString::from("70000"),
            &QString::default(),
            &QString::default(),
            false,
        )
        .expect_err("port is out of range");
        assert_eq!(bad_port, "端口必须是 1-65535");

        let unknown = connection_input(
            &QString::from("s3"),
            &QString::from("bucket"),
            &QString::default(),
            &QString::default(),
            &QString::default(),
            false,
        )
        .expect_err("unknown protocol");
        assert!(unknown.starts_with("不支持的连接协议"));
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
        assert!(json.contains("\"app.pathJump\":\"Mod+P\""));
    }
}
