//! The QML side of one terminal session.
//!
//! `dae_core::pty` owns the PTY and the libghostty-vt grid; this object only turns
//! its snapshots into a compact byte frame and coalesces output wakes before
//! they cross the Qt thread. The painter (`cpp/terminal_item.h`) reads the
//! frame; nothing here knows how a cell is drawn.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use qtbridge::{QmlElement, QmlObject, qobject};
use dae_core::pty::{
    SnapshotKind, TerminalEvent, TerminalSession as CoreSession, TerminalSnapshot,
};

/// Registers the C++ painter as the `dae.terminal/TerminalItem` QML type and
/// the two M3 Expressive indicators as `dae.m3`. Must run before the engine
/// loads a file that imports them.
pub fn register_item() {
    ffi::register_terminal_item();
    ffi::register_m3_items();
}

const FRAME_MAGIC: &[u8; 4] = b"DAET";
const FRAME_VERSION: u8 = 3;
/// magic | version | cols | rows | cursor col | cursor row | visible |
/// shape | display offset | history size | kind | dirty-row count.
const HEADER_LEN: usize = 4 + 1 + 2 + 2 + 2 + 2 + 1 + 1 + 4 + 4 + 1 + 2;
const CELL_LEN: usize = 4 + 4 + 4 + 1;

#[cxx::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("terminal_item.h");
        #[namespace = "dae_qt"]
        #[rust_name = "register_terminal_item"]
        fn dae_qt_register_terminal_item();

        include!("m3_items.h");
        #[namespace = "dae_qt"]
        #[rust_name = "register_m3_items"]
        fn dae_qt_register_m3_items();
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
mod session_impl {
    use super::*;

    #[derive(Default)]
    pub struct TerminalSession {
        session: Option<Arc<CoreSession>>,
        running: bool,
        exit_code: i32,
        title: String,
        search_index: i32,
        search_total: i32,
        window_title: String,
        cwd: String,
        shell: String,
    }


    impl TerminalSession {
        qproperty!("running", Member = running, Notify = running_changed);
        qproperty!("exitCode", Member = exit_code, Notify = exit_code_changed);
        qproperty!("title", Member = title, Notify = title_changed);
        qproperty!("searchIndex", Member = search_index, Notify = search_index_changed);
        qproperty!("searchTotal", Member = search_total, Notify = search_total_changed);
        qproperty!("windowTitle", Member = window_title, Notify = window_title_changed);
        qproperty!("cwd", Member = cwd, Notify = cwd_changed);
        qproperty!("shell", Member = shell, Notify = shell_changed);

        #[qsignal]
        fn running_changed(&mut self);
        #[qsignal]
        fn exit_code_changed(&mut self);
        #[qsignal]
        fn title_changed(&mut self);
        #[qsignal]
        fn search_index_changed(&mut self);
        #[qsignal]
        fn search_total_changed(&mut self);
        #[qsignal]
        fn window_title_changed(&mut self);
        #[qsignal]
        fn cwd_changed(&mut self);
        #[qsignal]
        fn shell_changed(&mut self);

        notifying_setters! {
            set_running(bool) => running, running_changed;
            set_exit_code(i32) => exit_code, exit_code_changed;
            set_title(String) => title, title_changed;
            set_search_index(i32) => search_index, search_index_changed;
            set_search_total(i32) => search_total, search_total_changed;
            set_window_title(String) => window_title, window_title_changed;
            set_cwd(String) => cwd, cwd_changed;
            set_shell(String) => shell, shell_changed;
        }

        #[qsignal]
        fn updated(&mut self);
        #[qsignal]
        fn exited(&mut self, code: i32);


        /// Spawns the shell in `cwd`, or home when it is not a local directory.
        #[qslot]
        fn start(
            &mut self,
            cwd: String,
            cols: i32,
            rows: i32,
            shell: String,
        ) -> bool {
            if let Some(previous) = self.session.take() {
                previous.kill();
            }

            let cols = cols.clamp(1, 1_000) as u16;
            let rows = rows.clamp(1, 1_000) as u16;
            let pending = Arc::new(AtomicBool::new(false));
            let thread = Arc::new(self.get_thread_handle());
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
                        let thread = Arc::clone(&thread);
                        thread.invoke_mut(move |session| {
                            pending.store(false, Ordering::Release);
                            // The shell's OSC reports (title, pwd) may have
                            // changed with this burst; copy them before the
                            // painter repaints so the header follows at once.
                            session.pull_reported_state();
                            session.updated();
                        });
                    }
                    TerminalEvent::Exited(code) => {
                        let thread = Arc::clone(&thread);
                        thread.invoke_mut(move |session| {
                            session.mark_exited(code);
                        });
                    }
                })
            };

            let directory = PathBuf::from(cwd);
            let cwd = directory.is_dir().then_some(directory);
            let configured = shell.trim().to_owned();
            self.set_shell(configured.to_owned());
            let program = (!configured.is_empty()).then_some(configured);
            match CoreSession::spawn(cwd, cols, rows, notify, program) {
                Ok(session) => {
                    // Publish the session before `running` flips: the painter
                    // resizes when it sees `runningChanged`, and that resize must
                    // find a live PTY or the shell keeps the placeholder 80x24.
                    self.session = Some(Arc::new(session));
                    self.set_running(true);
                    self.set_exit_code(-1);
                    self.set_title("Terminal".to_owned());
                    true
                }
                Err(error) => {
                    self.set_running(false);
                    self.set_exit_code(-1);
                    self.set_title(error.to_owned());
                    false
                }
            }
        }

        #[qslot]

        fn write(&mut self, data: String) -> bool {
            let Some(session) = self.session.as_ref() else {
                return false;
            };
            session.write(data.as_bytes()).is_ok()
        }

        #[qslot]

        fn paste_text(&mut self, data: String) -> bool {
            let Some(session) = self.session.as_ref() else {
                return false;
            };
            session.paste(&data).is_ok()
        }

        #[qslot]

        fn wheel_scroll(&mut self, lines: i32) {
            if let Some(session) = self.session.as_ref() {
                // On the alternate screen the program took the wheel itself;
                // otherwise the grid moved and must be repainted.
                if !session.wheel_scroll(lines).unwrap_or(true) {
                    self.updated();
                }
            }
        }

        #[qslot]

        fn write_bytes(&mut self, data: Vec<u8>) -> bool {
            let Some(session) = self.session.as_ref() else {
                return false;
            };
            session.write(&data).is_ok()
        }

        #[qslot]

        fn resize(&mut self, cols: i32, rows: i32) {
            if let Some(session) = self.session.as_ref() {
                let _ = session.resize(cols.clamp(1, 1_000) as u16, rows.clamp(1, 1_000) as u16);
            }
        }

        #[qslot]

        fn scroll(&mut self, lines: i32) {
            if let Some(session) = self.session.as_ref() {
                session.scroll(lines);
            }
            // Scrolling changes the grid without new output, so the painter would
            // otherwise keep showing the frame it last pulled.
            self.updated();
        }

        #[qslot]

        fn scroll_to(&mut self, offset: i32) {
            if let Some(session) = self.session.as_ref() {
                session.scroll_to(offset.max(0) as usize);
            }
            self.updated();
        }

        #[qslot]

        fn kill(&mut self) {
            if let Some(session) = self.session.as_ref() {
                session.kill();
            }
        }

        #[qslot]

        fn set_search(&mut self, needle: String) {
            let text = needle;
            let stripped = text.trim().to_owned();
            if let Some(session) = self.session.as_ref() {
                session.set_search_needle(if stripped.is_empty() { None } else { Some(&stripped) });
            }
            self.set_search_index(0);
            self.set_search_total(0);
            self.updated();
        }

        // The QML-side names; the bridge expects them verbatim.
        #[qslot]
        fn search_next(&mut self) -> bool {
            self.search_step(true)
        }

        #[qslot]

        fn search_prev(&mut self) -> bool {
            self.search_step(false)
        }

        fn search_step(&mut self, forward: bool) -> bool {
            let Some(session) = self.session.as_ref() else {
                return false;
            };
            let Some((total, index)) = session.search_step(forward) else {
                self.set_search_index(0);
                self.set_search_total(0);
                self.updated();
                return false;
            };
            self.set_search_total(total.clamp(0, i32::MAX as usize) as i32);
            self.set_search_index((index + 1).clamp(1, total.max(1)) as i32);
            self.updated();
            true
        }

        /// `count` as `u16`, then per rectangle `row`/`column`/`columns` as
        /// `u16` each, little endian. The painter decodes exactly this.
        #[qslot]
        fn search_rects(&mut self) -> Vec<u8> {
            let Some(session) = self.session.as_ref() else {
                return Vec::new();
            };
            let rects = session.search_viewport();
            let mut out = Vec::with_capacity(2 + rects.len() * 6);
            out.extend_from_slice(&(rects.len() as u16).to_le_bytes());
            for rect in &rects {
                out.extend_from_slice(&rect.row.to_le_bytes());
                out.extend_from_slice(&rect.column.to_le_bytes());
                out.extend_from_slice(&rect.columns.to_le_bytes());
            }
            out
        }

        #[qslot]

        fn snapshot(&self, dark: bool) -> Vec<u8> {
            let Some(session) = self.session.as_ref() else {
                return Vec::new();
            };
            let Some(snapshot) = session.snapshot(dark) else {
                return Vec::new();
            };
            encode_frame(&snapshot)
        }

        #[qslot]

        fn snapshot_full(&self, dark: bool) -> Vec<u8> {
            let Some(session) = self.session.as_ref() else {
                return Vec::new();
            };
            let Some(snapshot) = session.snapshot_full(dark) else {
                return Vec::new();
            };
            encode_frame(&snapshot)
        }

        #[allow(clippy::too_many_arguments)]
        #[qslot]
        fn send_key(
            &mut self,
            action: i32,
            key: i32,
            mods: i32,
            unshifted: i32,
            consumed_mods: i32,
            text: String,
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
                    &text,
                    composing,
                )
                .unwrap_or(false)
        }

        #[allow(clippy::too_many_arguments)]
        #[qslot]
        fn send_mouse(
            &mut self,
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

        #[qslot]

        fn mouse_tracking(&self) -> bool {
            self.session
                .as_ref()
                .is_some_and(|session| session.mouse_tracking())
        }

        #[qslot]

        fn send_focus(&mut self, gained: bool) {
            if let Some(session) = self.session.as_ref() {
                let _ = session.focus_event(gained);
            }
        }

        /// Copies the OSC-reported title and working directory onto the QObject
        /// so QML bindings can follow them. Runs on the Qt thread with every
        /// output wake; the copies only allocate when a report actually changed.
        fn pull_reported_state(&mut self) {
            let title = self.session.as_ref().and_then(|s| s.window_title());
            if let Some(title) = title
                && title != self.window_title
            {
                self.set_window_title(title);
            }
            let cwd = self.session.as_ref().and_then(|s| s.working_directory());
            if let Some(cwd) = cwd
                && cwd != self.cwd
            {
                self.set_cwd(cwd);
            }
        }

        fn mark_exited(&mut self, code: i32) {
            if !self.running {
                return;
            }
            self.set_running(false);
            self.set_exit_code(code);
            self.exited(code);
            self.updated();
        }
    }
}

pub use session_impl::TerminalSession;

impl QmlElement for TerminalSession {
    const URI: &str = "dae";
    const ELEMENT_NAME: &str = "TerminalSession";
    const MINOR_VERSION: u8 = 0;
    const MAJOR_VERSION: u8 = 1;
    const IS_SINGLETON: bool = false;
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
