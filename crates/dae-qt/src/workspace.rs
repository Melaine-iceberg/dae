//! The command surface QML calls.
//!
//! Every file operation is a `dae_core::fs::service` call. This module does
//! three shell-only things around them: it owns the Tokio runtime, it turns
//! the core's `Host` events into queued Qt property updates, and it holds the
//! bits of state that belong to a *shell* rather than to the file system —
//! the in-app clipboard, the last accent reading, and the cached terminal
//! typography.
//!
//! Calls return immediately. The outcome arrives as `operationFinished` or
//! `operationFailed`, which is what lets the QML side keep painting during a
//! large copy instead of blocking on an `invoke`.

use core::pin::Pin;
use std::sync::Arc;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QStringList};
use dae_core::error::FileSystemError;
use dae_core::fs::recents::RecentSource;
use dae_core::fs::service;
use dae_core::fs::sidebar::{self, Favorite};
use dae_core::fs::types::{ConflictAction, NewEntryKind, TransferItem, display_name_from_path};
use dae_core::host::{Event, Host};

use crate::platform;
use crate::runtime;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
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
        #[qproperty(QString, clipboard_mode, cxx_name = "clipboardMode")]
        #[qproperty(QStringList, clipboard_paths, cxx_name = "clipboardPaths")]
        #[qproperty(QString, accent)]
        #[qproperty(QString, home_directory, cxx_name = "homeDirectory")]
        #[qproperty(i32, terminal_font_size, cxx_name = "terminalFontSize")]
        #[qproperty(f64, terminal_line_height, cxx_name = "terminalLineHeight")]
        #[qproperty(QString, terminal_font_family, cxx_name = "terminalFontFamily")]
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

        /// Copies or moves `paths` into `destination`. `cut` selects move.
        #[qinvokable]
        #[cxx_name = "transferEntries"]
        fn transfer_entries(
            self: Pin<&mut Workspace>,
            paths: &QStringList,
            destination: &QString,
            cut: bool,
        );

        /// Remembers `paths` as the in-app clipboard. `mode` is `"copy"` or
        /// `"cut"`; an empty list clears it.
        #[qinvokable]
        #[cxx_name = "setClipboard"]
        fn set_clipboard(self: Pin<&mut Workspace>, mode: &QString, paths: &QStringList);

        #[qinvokable]
        #[cxx_name = "clearClipboard"]
        fn clear_clipboard(self: Pin<&mut Workspace>);

        #[qinvokable]
        #[cxx_name = "pasteInto"]
        fn paste_into(self: Pin<&mut Workspace>, destination: &QString);

        #[qinvokable]
        fn undo(self: Pin<&mut Workspace>);

        #[qinvokable]
        fn redo(self: Pin<&mut Workspace>);

        #[qinvokable]
        #[cxx_name = "openPath"]
        fn open_path(self: Pin<&mut Workspace>, path: &QString);

        #[qinvokable]
        #[cxx_name = "openWith"]
        fn open_with(self: Pin<&mut Workspace>, path: &QString);

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

        /// The operation succeeded; `focus` is the path to select, empty when
        /// the operation created nothing addressable.
        #[qsignal]
        #[cxx_name = "operationFinished"]
        fn operation_finished(self: Pin<&mut Workspace>, kind: &QString, focus: &QString);

        /// The operation failed; `message` is already user-facing.
        #[qsignal]
        #[cxx_name = "operationFailed"]
        fn operation_failed(self: Pin<&mut Workspace>, kind: &QString, message: &QString);

        #[qsignal]
        #[cxx_name = "favoritesChanged"]
        fn favorites_changed(self: Pin<&mut Workspace>);
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
    clipboard_mode: QString,
    clipboard_paths: QStringList,
    accent: QString,
    home_directory: QString,
    terminal_font_size: i32,
    terminal_line_height: f64,
    terminal_font_family: QString,
    active_operations: i32,
}

impl Default for WorkspaceRust {
    fn default() -> Self {
        // Settings are a process-wide registry; initializing here means the
        // first QML window and every later one read the same document.
        let _ = dae_core::settings::init();
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
            clipboard_mode: QString::default(),
            clipboard_paths: QStringList::default(),
            accent: QString::from(platform::accent_hex().as_str()),
            home_directory: QString::from(home.as_str()),
            terminal_font_size: settings.terminal.font_size as i32,
            terminal_line_height: settings.terminal.line_height as f64,
            terminal_font_family: QString::from(
                settings.terminal.font_family.as_deref().unwrap_or(""),
            ),
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
        let active = (self.active_operations - 1).max(0);
        self.as_mut().rust_mut().active_operations = active;
        self.as_mut().set_busy(active > 0);
        self.as_mut().set_progress_done(0);
        self.as_mut().set_progress_total(0);
        self.as_mut().set_progress_label(QString::default());

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

    fn transfer_entries(
        mut self: Pin<&mut Self>,
        paths: &QStringList,
        destination: &QString,
        cut: bool,
    ) {
        let items = transfer_items(paths);
        if items.is_empty() {
            return;
        }
        let destination = destination.to_string();
        let kind = if cut { "move" } else { "copy" };
        self.as_mut()
            .begin_operation(if cut { "移动中…" } else { "复制中…" });
        let thread = self.as_mut().qt_thread();
        let host = host_for(&thread);
        dispatch(thread, kind, async move {
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
        // The cut clipboard is one-shot; a copy clipboard stays available so
        // the same selection can be pasted into several directories.
        if cut {
            self.as_mut().clear_clipboard();
        }
    }

    fn set_clipboard(mut self: Pin<&mut Self>, mode: &QString, paths: &QStringList) {
        let mode = mode.to_string();
        if paths.is_empty() {
            self.as_mut().clear_clipboard();
            return;
        }
        self.as_mut().rust_mut().clipboard_mode = QString::from(mode.as_str());
        self.as_mut().rust_mut().clipboard_paths = paths.clone();
    }

    fn clear_clipboard(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().clipboard_mode = QString::default();
        self.as_mut().rust_mut().clipboard_paths = QStringList::default();
    }

    fn paste_into(mut self: Pin<&mut Self>, destination: &QString) {
        let mode = self.clipboard_mode.to_string();
        if mode.is_empty() {
            self.as_mut()
                .finish_operation("copy", "", Some("剪贴板是空的".to_owned()));
            return;
        }
        let paths = self.clipboard_paths.clone();
        self.as_mut()
            .transfer_entries(&paths, destination, mode == "cut");
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

    fn open_path(mut self: Pin<&mut Self>, path: &QString) {
        let path = path.to_string();
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

    fn open_terminal(mut self: Pin<&mut Self>, path: &QString) {
        let path = path.to_string();
        match service::open_terminal(path) {
            Ok(()) => self
                .as_mut()
                .set_status(QString::from("已在系统终端中打开")),
            Err(error) => {
                let message = friendly_error(&error.to_string());
                self.as_mut()
                    .set_last_error(QString::from(message.as_str()));
            }
        }
    }

    fn reveal(mut self: Pin<&mut Self>, path: &QString) {
        let path = path.to_string();
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
}

fn strings(list: &QStringList) -> Vec<String> {
    list.iter().map(ToString::to_string).collect()
}

fn transfer_items(list: &QStringList) -> Vec<TransferItem> {
    list.iter()
        .map(|path| TransferItem {
            path: path.to_string(),
            on_conflict: ConflictAction::KeepBoth,
        })
        .collect()
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
        "copy" => "已复制",
        "move" => "已移动",
        "undo" => "已撤销",
        "redo" => "已重做",
        "openWith" => "已打开",
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

/// Core errors are stable codes in some paths and prose in others. Translate
/// the codes a user can actually reach here, and leave the rest intact rather
/// than hiding an unexpected failure behind a generic sentence.
fn friendly_error(message: &str) -> String {
    if message.starts_with("fs.undo_empty") {
        return "没有可撤销的操作".to_owned();
    }
    if message.starts_with("fs.redo_empty") {
        return "没有可重做的操作".to_owned();
    }
    if message.starts_with("fs.trash_local_only") {
        return "回收站只支持本地项目".to_owned();
    }
    if message.starts_with("fs.terminal_local_dir_only") {
        return "终端只能打开本地目录".to_owned();
    }
    if message.starts_with("fs.open_with_local_only") {
        return "只有本地项目可以选择打开方式".to_owned();
    }
    message.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_items_keep_both_rather_than_overwrite() {
        let mut list = QStringList::default();
        list.append(QString::from("C:/a.txt"));
        list.append(QString::from("C:/b.txt"));

        let items = transfer_items(&list);
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|item| item.path.starts_with("C:/")));
    }

    #[test]
    fn known_core_codes_become_sentences_and_others_pass_through() {
        assert_eq!(friendly_error("fs.undo_empty"), "没有可撤销的操作");
        assert_eq!(
            friendly_error("The requested path was not found: C:/x"),
            "The requested path was not found: C:/x"
        );
    }
}
