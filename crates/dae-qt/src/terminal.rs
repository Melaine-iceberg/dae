//! The QML side of one terminal session.
//!
//! `dae_core::pty` owns the PTY and the libghostty-vt grid; this object only turns
//! its snapshots into a compact byte frame and coalesces output wakes before
//! they cross the Qt thread. The painter (`cpp/terminal_item.h`) reads the
//! frame; nothing here knows how a cell is drawn.

use core::pin::Pin;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QString};
use dae_core::pty::{
    SnapshotKind, TerminalEvent, TerminalSession as CoreSession, TerminalSnapshot,
};

/// Registers the C++ painter as the `dae.terminal/TerminalItem` QML type and
/// the two M3 Expressive indicators as `dae.m3`. Must run before the engine
/// loads a file that imports them.
pub fn register_item() {
    qobject::register_terminal_item();
    qobject::register_m3_items();
}

const FRAME_MAGIC: &[u8; 4] = b"DAET";
const FRAME_VERSION: u8 = 3;
/// magic | version | cols | rows | cursor col | cursor row | visible |
/// shape | display offset | history size | kind | dirty-row count.
const HEADER_LEN: usize = 4 + 1 + 2 + 2 + 2 + 2 + 1 + 1 + 4 + 4 + 1 + 2;
const CELL_LEN: usize = 4 + 4 + 4 + 1;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qbytearray.h");
        type QByteArray = cxx_qt_lib::QByteArray;

        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("terminal_item.h");
        #[namespace = "dae_qt"]
        #[rust_name = "register_terminal_item"]
        fn dae_qt_register_terminal_item();

        include!("m3_items.h");
        #[namespace = "dae_qt"]
        #[rust_name = "register_m3_items"]
        fn dae_qt_register_m3_items();
    }

    unsafe extern "RustQt" {
        /// One PTY session, exposed to QML as `TerminalSession`.
        ///
        /// `running` and `exitCode` are properties so the panel can show the
        /// shell's state without polling. Output is a signal rather than a
        /// property: the grid is too large to be a QML value, and the painter
        /// pulls it only when it is about to draw.
        #[qobject]
        #[qml_element]
        #[qproperty(bool, running)]
        #[qproperty(i32, exit_code, cxx_name = "exitCode")]
        #[qproperty(QString, title)]
        /// Find-bar state: the selected match's 1-based position and the
        /// total on the active screen, refreshed by `searchNext`/`searchPrev`.
        #[qproperty(i32, search_index, cxx_name = "searchIndex")]
        #[qproperty(i32, search_total, cxx_name = "searchTotal")]
        /// The window title the shell last set with OSC 0/2; empty until
        /// the shell reports one. Distinct from `title`, which carries the
        /// start error when spawning failed.
        #[qproperty(QString, window_title, cxx_name = "windowTitle")]
        /// The working directory the shell last reported with OSC 7/9/1337;
        /// empty until then, so the panel falls back to the spawn directory.
        #[qproperty(QString, cwd)]
        /// The executable this session was started with; empty means the
        /// platform default. The panel names the running shell from this, not
        /// from the setting, which may have changed since the spawn.
        #[qproperty(QString, shell)]
        type TerminalSession = super::TerminalSessionRust;

        /// `shell` is the executable the settings page configured; empty means
        /// "pick the platform default".
        #[qinvokable]
        fn start(
            self: Pin<&mut TerminalSession>,
            cwd: &QString,
            cols: i32,
            rows: i32,
            shell: &QString,
        ) -> bool;

        /// Writes a UTF-8 string. Enter and the control keys are encoded by
        /// the painter into the bytes the shell actually receives.
        #[qinvokable]
        fn write(self: Pin<&mut TerminalSession>, data: &QString) -> bool;

        /// Pastes text the way the running program asked to receive it —
        /// bracketed when it enabled mode 2004 — instead of raw keystrokes.
        #[qinvokable]
        #[cxx_name = "pasteText"]
        fn paste_text(self: Pin<&mut TerminalSession>, data: &QString) -> bool;

        /// Wheel scroll: arrow keys on the alternate screen, history
        /// scrolling on the primary one.
        #[qinvokable]
        #[cxx_name = "wheelScroll"]
        fn wheel_scroll(self: Pin<&mut TerminalSession>, lines: i32);

        #[qinvokable]
        #[rust_name = "write_bytes"]
        #[cxx_name = "writeBytes"]
        fn writeByteData(self: Pin<&mut TerminalSession>, data: &QByteArray) -> bool;

        #[qinvokable]
        fn resize(self: Pin<&mut TerminalSession>, cols: i32, rows: i32);

        #[qinvokable]
        fn scroll(self: Pin<&mut TerminalSession>, lines: i32);

        /// Scrolls to an absolute history offset, 0 being the live bottom.
        #[qinvokable]
        #[cxx_name = "scrollTo"]
        fn scroll_to(self: Pin<&mut TerminalSession>, offset: i32);

        #[qinvokable]
        fn kill(self: Pin<&mut TerminalSession>);

        /// Sets what the terminal search looks for; an empty string clears
        /// it (and with it every highlight).
        #[qinvokable]
        #[cxx_name = "setSearch"]
        fn set_search(self: Pin<&mut TerminalSession>, needle: &QString);

        /// Steps to the next (older) match and scrolls it into view. The
        /// find bar reads `searchIndex`/`searchTotal` afterwards.
        #[qinvokable]
        fn searchNext(self: Pin<&mut TerminalSession>) -> bool;

        /// Steps to the previous (newer) match.
        #[qinvokable]
        fn searchPrev(self: Pin<&mut TerminalSession>) -> bool;

        /// The viewport's highlight rectangles (see `encode_search_rects`),
        /// empty when no search is set.
        #[qinvokable]
        #[cxx_name = "searchRects"]
        fn search_rects(self: Pin<&mut TerminalSession>) -> QByteArray;

        /// The whole visible grid as a compact frame (see `encode_frame`),
        /// resolved against the light or dark palette.
        #[qinvokable]
        fn snapshot(&self, dark: bool) -> QByteArray;

        /// Forces a complete grid frame (e.g. when switching session tabs).
        #[qinvokable]
        #[cxx_name = "snapshotFull"]
        fn snapshot_full(&self, dark: bool) -> QByteArray;

        /// Encodes and writes a key event via libghostty-vt's key encoder.
        #[qinvokable]
        #[cxx_name = "sendKey"]
        fn send_key(
            self: Pin<&mut TerminalSession>,
            action: i32,
            key: i32,
            mods: i32,
            unshifted: i32,
            consumed_mods: i32,
            text: &QString,
            composing: bool,
        ) -> bool;

        /// Encodes and writes a mouse event when mouse tracking is active.
        #[qinvokable]
        #[cxx_name = "sendMouse"]
        fn send_mouse(
            self: Pin<&mut TerminalSession>,
            action: i32,
            button: i32,
            mods: i32,
            x: f64,
            y: f64,
            screen_w: i32,
            screen_h: i32,
            cell_w: i32,
            cell_h: i32,
            any_pressed: bool,
        ) -> bool;

        /// Whether the running program has enabled mouse tracking.
        #[qinvokable]
        #[cxx_name = "mouseTracking"]
        fn mouse_tracking(&self) -> bool;

        /// Sends a focus gained/lost report when mode 1004 is enabled.
        #[qinvokable]
        #[cxx_name = "sendFocus"]
        fn send_focus(self: Pin<&mut TerminalSession>, gained: bool);

        /// New grid content (or the shell's state) is ready to paint.
        #[qsignal]
        fn updated(self: Pin<&mut TerminalSession>);

        /// The shell has exited with `code`. Sent once per session.
        #[qsignal]
        fn exited(self: Pin<&mut TerminalSession>, code: i32);
    }

    impl cxx_qt::Threading for TerminalSession {}
}

#[derive(Default)]
pub struct TerminalSessionRust {
    session: Option<Arc<CoreSession>>,
    running: bool,
    exit_code: i32,
    title: QString,
    search_index: i32,
    search_total: i32,
    window_title: QString,
    cwd: QString,
    shell: QString,
}

impl qobject::TerminalSession {
    /// Spawns the shell in `cwd`, or home when it is not a local directory.
    fn start(
        mut self: Pin<&mut Self>,
        cwd: &QString,
        cols: i32,
        rows: i32,
        shell: &QString,
    ) -> bool {
        if let Some(previous) = self.as_mut().rust_mut().session.take() {
            previous.kill();
        }

        let cols = cols.clamp(1, 1_000) as u16;
        let rows = rows.clamp(1, 1_000) as u16;
        let pending = Arc::new(AtomicBool::new(false));
        let thread = self.as_mut().qt_thread();
        let notify = {
            let pending = Arc::clone(&pending);
            Arc::new(move |event: TerminalEvent| match event {
                TerminalEvent::Output => {
                    // One queued update per burst: the grid is pulled, not
                    // pushed, so a second wake before the first is delivered
                    // would render the same frame twice.
                    if pending.swap(true, Ordering::AcqRel) {
                        return;
                    }
                    let pending = Arc::clone(&pending);
                    let _ = thread.queue(move |mut session| {
                        pending.store(false, Ordering::Release);
                        // The shell's OSC reports (title, pwd) may have
                        // changed with this burst; copy them before the
                        // painter repaints so the header follows at once.
                        session.as_mut().pull_reported_state();
                        session.as_mut().updated();
                    });
                }
                TerminalEvent::Exited(code) => {
                    let _ = thread.queue(move |mut session| {
                        session.as_mut().mark_exited(code);
                    });
                }
            })
        };

        let directory = PathBuf::from(cwd.to_string());
        let cwd = directory.is_dir().then_some(directory);
        let configured = shell.to_string().trim().to_owned();
        self.as_mut().set_shell(QString::from(configured.as_str()));
        let program = (!configured.is_empty()).then_some(configured);
        match CoreSession::spawn(cwd, cols, rows, notify, program) {
            Ok(session) => {
                // Publish the session before `running` flips: the painter
                // resizes when it sees `runningChanged`, and that resize must
                // find a live PTY or the shell keeps the placeholder 80x24.
                self.as_mut().rust_mut().session = Some(Arc::new(session));
                self.as_mut().set_running(true);
                self.as_mut().set_exit_code(-1);
                self.as_mut().set_title(QString::from("Terminal"));
                true
            }
            Err(error) => {
                self.as_mut().set_running(false);
                self.as_mut().set_exit_code(-1);
                self.as_mut().set_title(QString::from(error.as_str()));
                false
            }
        }
    }

    fn write(self: Pin<&mut Self>, data: &QString) -> bool {
        let Some(session) = self.session.as_ref() else {
            return false;
        };
        session.write(data.to_string().as_bytes()).is_ok()
    }

    fn paste_text(self: Pin<&mut Self>, data: &QString) -> bool {
        let Some(session) = self.session.as_ref() else {
            return false;
        };
        session.paste(&data.to_string()).is_ok()
    }

    fn wheel_scroll(mut self: Pin<&mut Self>, lines: i32) {
        if let Some(session) = self.session.as_ref() {
            // On the alternate screen the program took the wheel itself;
            // otherwise the grid moved and must be repainted.
            if !session.wheel_scroll(lines).unwrap_or(true) {
                self.as_mut().updated();
            }
        }
    }

    fn write_bytes(self: Pin<&mut Self>, data: &QByteArray) -> bool {
        let Some(session) = self.session.as_ref() else {
            return false;
        };
        session.write(data.as_slice()).is_ok()
    }

    fn resize(self: Pin<&mut Self>, cols: i32, rows: i32) {
        if let Some(session) = self.session.as_ref() {
            let _ = session.resize(cols.clamp(1, 1_000) as u16, rows.clamp(1, 1_000) as u16);
        }
    }

    fn scroll(mut self: Pin<&mut Self>, lines: i32) {
        if let Some(session) = self.session.as_ref() {
            session.scroll(lines);
        }
        // Scrolling changes the grid without new output, so the painter would
        // otherwise keep showing the frame it last pulled.
        self.as_mut().updated();
    }

    fn scroll_to(mut self: Pin<&mut Self>, offset: i32) {
        if let Some(session) = self.session.as_ref() {
            session.scroll_to(offset.max(0) as usize);
        }
        self.as_mut().updated();
    }

    fn kill(self: Pin<&mut Self>) {
        if let Some(session) = self.session.as_ref() {
            session.kill();
        }
    }

    fn set_search(mut self: Pin<&mut Self>, needle: &QString) {
        let text = needle.to_string();
        let stripped = text.trim().to_owned();
        if let Some(session) = self.session.as_ref() {
            session.set_search_needle(if stripped.is_empty() { None } else { Some(&stripped) });
        }
        self.as_mut().set_search_index(0);
        self.as_mut().set_search_total(0);
        self.as_mut().updated();
    }

    // The QML-side names; the bridge expects them verbatim.
    #[allow(non_snake_case)]
    fn searchNext(self: Pin<&mut Self>) -> bool {
        self.search_step(true)
    }

    #[allow(non_snake_case)]
    fn searchPrev(self: Pin<&mut Self>) -> bool {
        self.search_step(false)
    }

    fn search_step(mut self: Pin<&mut Self>, forward: bool) -> bool {
        let Some(session) = self.session.as_ref() else {
            return false;
        };
        let Some((total, index)) = session.search_step(forward) else {
            self.as_mut().set_search_index(0);
            self.as_mut().set_search_total(0);
            self.as_mut().updated();
            return false;
        };
        self.as_mut().set_search_total(total.clamp(0, i32::MAX as usize) as i32);
        self.as_mut().set_search_index((index + 1).clamp(1, total.max(1)) as i32);
        self.as_mut().updated();
        true
    }

    /// `count` as `u16`, then per rectangle `row`/`column`/`columns` as
    /// `u16` each, little endian. The painter decodes exactly this.
    fn search_rects(self: Pin<&mut Self>) -> QByteArray {
        let Some(session) = self.session.as_ref() else {
            return QByteArray::default();
        };
        let rects = session.search_viewport();
        let mut out = Vec::with_capacity(2 + rects.len() * 6);
        out.extend_from_slice(&(rects.len() as u16).to_le_bytes());
        for rect in &rects {
            out.extend_from_slice(&rect.row.to_le_bytes());
            out.extend_from_slice(&rect.column.to_le_bytes());
            out.extend_from_slice(&rect.columns.to_le_bytes());
        }
        QByteArray::from(out.as_slice())
    }

    fn snapshot(&self, dark: bool) -> QByteArray {
        let Some(session) = self.session.as_ref() else {
            return QByteArray::default();
        };
        let Some(snapshot) = session.snapshot(dark) else {
            return QByteArray::default();
        };
        QByteArray::from(encode_frame(&snapshot).as_slice())
    }

    fn snapshot_full(&self, dark: bool) -> QByteArray {
        let Some(session) = self.session.as_ref() else {
            return QByteArray::default();
        };
        let Some(snapshot) = session.snapshot_full(dark) else {
            return QByteArray::default();
        };
        QByteArray::from(encode_frame(&snapshot).as_slice())
    }

    #[allow(clippy::too_many_arguments)]
    fn send_key(
        self: Pin<&mut Self>,
        action: i32,
        key: i32,
        mods: i32,
        unshifted: i32,
        consumed_mods: i32,
        text: &QString,
        composing: bool,
    ) -> bool {
        let Some(session) = self.session.as_ref() else {
            return false;
        };
        session
            .key_event(
                action,
                key,
                mods.clamp(0, u16::MAX as i32) as u16,
                consumed_mods.clamp(0, u16::MAX as i32) as u16,
                unshifted.max(0) as u32,
                &text.to_string(),
                composing,
            )
            .unwrap_or(false)
    }

    #[allow(clippy::too_many_arguments)]
    fn send_mouse(
        self: Pin<&mut Self>,
        action: i32,
        button: i32,
        mods: i32,
        x: f64,
        y: f64,
        screen_w: i32,
        screen_h: i32,
        cell_w: i32,
        cell_h: i32,
        any_pressed: bool,
    ) -> bool {
        let Some(session) = self.session.as_ref() else {
            return false;
        };
        let button = (button >= 0).then_some(button);
        session
            .mouse_event(
                action,
                button,
                mods.clamp(0, u16::MAX as i32) as u16,
                x as f32,
                y as f32,
                screen_w.max(1) as u32,
                screen_h.max(1) as u32,
                cell_w.max(1) as u32,
                cell_h.max(1) as u32,
                any_pressed,
            )
            .unwrap_or(false)
    }

    fn mouse_tracking(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|session| session.mouse_tracking())
    }

    fn send_focus(self: Pin<&mut Self>, gained: bool) {
        if let Some(session) = self.session.as_ref() {
            let _ = session.focus_event(gained);
        }
    }

    /// Copies the OSC-reported title and working directory onto the QObject
    /// so QML bindings can follow them. Runs on the Qt thread with every
    /// output wake; the copies only allocate when a report actually changed.
    fn pull_reported_state(mut self: Pin<&mut Self>) {
        let title = self.session.as_ref().and_then(|s| s.window_title());
        if let Some(title) = title.map(QString::from)
            && title != self.window_title
        {
            self.as_mut().set_window_title(title);
        }
        let cwd = self.session.as_ref().and_then(|s| s.working_directory());
        if let Some(cwd) = cwd.map(QString::from)
            && cwd != self.cwd
        {
            self.as_mut().set_cwd(cwd);
        }
    }

    fn mark_exited(mut self: Pin<&mut Self>, code: i32) {
        if !self.running {
            return;
        }
        self.as_mut().set_running(false);
        self.as_mut().set_exit_code(code);
        self.as_mut().exited(code);
        self.as_mut().updated();
    }
}

/// `DAET` | version 3 | cols | rows | cursor col | cursor row | visible |
/// shape | display offset | history size | kind | count.
///
/// Little endian, one 13-byte record per cell: Unicode scalar, fg ARGB, bg
/// ARGB, attribute bits.
/// - kind 0 (full, count 0): every viewport cell.
/// - kind 1 (shift, count k): the k new bottom rows, top to bottom.
/// - kind 2 (rows, count n): n dirty rows, each prefixed with `row` (`u16` LE)
///   followed by `cols` cells (n == 0 when the frame is clean).
fn encode_frame(snapshot: &TerminalSnapshot) -> Vec<u8> {
    let (kind_byte, count, payload_cells) = match &snapshot.kind {
        SnapshotKind::Full => (0u8, 0usize, snapshot.cells.len()),
        SnapshotKind::Shift(lines) => (1u8, *lines, snapshot.cells.len()),
        SnapshotKind::Rows(rows) => (
            2u8,
            rows.len(),
            rows.iter().map(|r| r.cells.len()).sum::<usize>(),
        ),
    };
    let row_headers = if kind_byte == 2 { count * 2 } else { 0 };
    let mut frame = Vec::with_capacity(HEADER_LEN + row_headers + payload_cells * CELL_LEN);
    frame.extend_from_slice(FRAME_MAGIC);
    frame.push(FRAME_VERSION);
    frame.extend_from_slice(&(snapshot.cols as u16).to_le_bytes());
    frame.extend_from_slice(&(snapshot.rows as u16).to_le_bytes());
    frame.extend_from_slice(&(snapshot.cursor_col as u16).to_le_bytes());
    frame.extend_from_slice(&(snapshot.cursor_row as u16).to_le_bytes());
    frame.push(u8::from(snapshot.cursor_visible));
    frame.push(snapshot.cursor_shape);
    frame.extend_from_slice(&(snapshot.display_offset as u32).to_le_bytes());
    frame.extend_from_slice(&(snapshot.history_size as u32).to_le_bytes());
    frame.push(kind_byte);
    frame.extend_from_slice(&(count as u16).to_le_bytes());
    match &snapshot.kind {
        SnapshotKind::Full | SnapshotKind::Shift(_) => {
            for cell in &snapshot.cells {
                frame.extend_from_slice(&(cell.ch as u32).to_le_bytes());
                frame.extend_from_slice(&cell.fg.to_le_bytes());
                frame.extend_from_slice(&cell.bg.to_le_bytes());
                frame.push(cell.flags);
            }
        }
        SnapshotKind::Rows(rows) => {
            for row in rows {
                frame.extend_from_slice(&row.row.to_le_bytes());
                for cell in &row.cells {
                    frame.extend_from_slice(&(cell.ch as u32).to_le_bytes());
                    frame.extend_from_slice(&cell.fg.to_le_bytes());
                    frame.extend_from_slice(&cell.bg.to_le_bytes());
                    frame.push(cell.flags);
                }
            }
        }
    }
    frame
}

#[cfg(test)]
mod tests {
    use dae_core::pty::DirtyRow;

    use super::*;

    #[test]
    fn a_frame_starts_with_the_header_the_painter_expects() {
        let snapshot = TerminalSnapshot {
            cols: 2,
            rows: 1,
            kind: SnapshotKind::Full,
            cursor_col: 1,
            cursor_row: 0,
            cursor_visible: true,
            cursor_shape: 0,
            display_offset: 0,
            history_size: 0,
            cells: vec![Default::default(); 2],
        };
        let frame = encode_frame(&snapshot);
        assert_eq!(&frame[..4], FRAME_MAGIC);
        assert_eq!(frame[4], FRAME_VERSION);
        // Header: 25 bytes, kind 0 (full), shifted-line count 0.
        assert_eq!(frame[23], 0);
        assert_eq!(read_u16(&frame, 24), 0);
        assert_eq!(frame.len(), HEADER_LEN + 2 * CELL_LEN);
    }

    #[test]
    fn a_shift_frame_carries_only_its_new_bottom_rows() {
        let snapshot = TerminalSnapshot {
            cols: 2,
            rows: 3,
            kind: SnapshotKind::Shift(1),
            cursor_col: 0,
            cursor_row: 2,
            cursor_visible: true,
            cursor_shape: 0,
            display_offset: 0,
            history_size: 0,
            cells: vec![Default::default(); 2],
        };
        let frame = encode_frame(&snapshot);
        assert_eq!(frame[23], 1);
        assert_eq!(read_u16(&frame, 24), 1);
        // One new row of two cells, and nothing else.
        assert_eq!(frame.len(), HEADER_LEN + 2 * CELL_LEN);
    }

    #[test]
    fn a_rows_frame_carries_only_dirty_rows() {
        let snapshot = TerminalSnapshot {
            cols: 2,
            rows: 3,
            kind: SnapshotKind::Rows(vec![DirtyRow {
                row: 1,
                cells: vec![Default::default(); 2],
            }]),
            cursor_col: 0,
            cursor_row: 1,
            cursor_visible: true,
            cursor_shape: 0,
            display_offset: 0,
            history_size: 0,
            cells: Vec::new(),
        };
        let frame = encode_frame(&snapshot);
        assert_eq!(frame[23], 2);
        assert_eq!(read_u16(&frame, 24), 1);
        assert_eq!(read_u16(&frame, HEADER_LEN), 1);
        assert_eq!(frame.len(), HEADER_LEN + 2 + 2 * CELL_LEN);
    }

    fn read_u16(frame: &[u8], at: usize) -> u16 {
        u16::from_le_bytes([frame[at], frame[at + 1]])
    }
}
