//! The QML side of one terminal session.
//!
//! `dae_core::pty` owns the PTY and the alacritty grid; this object only turns
//! its snapshots into a compact byte frame and coalesces output wakes before
//! they cross the Qt thread. The painter (`cpp/terminal_item.h`) reads the
//! frame; nothing here knows how a cell is drawn.

use core::pin::Pin;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QString};
use dae_core::pty::{TerminalEvent, TerminalSession as CoreSession, TerminalSnapshot};

/// Registers the C++ painter as the `dae.terminal/TerminalItem` QML type.
/// Must run before the engine loads a file that imports it.
pub fn register_item() {
    qobject::register_terminal_item();
}

const FRAME_MAGIC: &[u8; 4] = b"DAET";
const FRAME_VERSION: u8 = 2;
const HEADER_LEN: usize = 4 + 1 + 2 + 2 + 2 + 2 + 1 + 1 + 4 + 4;
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

        /// The whole visible grid as a compact frame (see `encode_frame`),
        /// resolved against the light or dark palette.
        #[qinvokable]
        fn snapshot(&self, dark: bool) -> QByteArray;

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
    shell: QString,
}

impl qobject::TerminalSession {
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

    fn snapshot(&self, dark: bool) -> QByteArray {
        let Some(session) = self.session.as_ref() else {
            return QByteArray::default();
        };
        let Some(snapshot) = session.snapshot(dark) else {
            return QByteArray::default();
        };
        QByteArray::from(encode_frame(&snapshot).as_slice())
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

/// `DAET` | version | cols | rows | cursor col | cursor row | visible | shape |
/// display offset | history size | cells.
///
/// Little endian, one 13-byte record per cell: Unicode scalar, fg ARGB, bg
/// ARGB, attribute bits. The painter decodes exactly this; keeping the format
/// next to the encoder is why it has no serde dependency.
fn encode_frame(snapshot: &TerminalSnapshot) -> Vec<u8> {
    let mut frame = Vec::with_capacity(HEADER_LEN + snapshot.cells.len() * CELL_LEN);
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
    for cell in &snapshot.cells {
        frame.extend_from_slice(&(cell.ch as u32).to_le_bytes());
        frame.extend_from_slice(&cell.fg.to_le_bytes());
        frame.extend_from_slice(&cell.bg.to_le_bytes());
        frame.push(cell.flags);
    }
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_starts_with_the_header_the_painter_expects() {
        let snapshot = TerminalSnapshot {
            cols: 2,
            rows: 1,
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
        assert_eq!(frame.len(), HEADER_LEN + 2 * CELL_LEN);
    }
}
