//! PTY-backed terminal sessions with a libghostty-vt grid.
//!
//! The shell's terminal is not a string buffer: libghostty-vt parses the
//! escape sequences and owns the cell grid, scrollback, cursor and modes,
//! while `portable-pty` owns the platform PTY. This module is the engine-free
//! half — it produces a [`TerminalSnapshot`] of styled cells and knows nothing
//! about Qt or any other renderer. `crates/dae-qt` paints that snapshot.
//!
//! The VT engine is the vendored libghostty-vt (see
//! `ghostty-vt/VERSION`); the raw C bindings live in `crate::ghostty_vt`.
//! Terminal queries (`CSI 6n`, device attributes, text-area size) are answered
//! by the library itself through the write-pty callback, which writes the
//! replies back to the same PTY the user's keystrokes go to.

use std::ffi::c_void;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::ghostty_vt::{
    self, GhosttyRowId, GhosttyStyle, GhosttyStyleColor, Vt,
};

const SCROLLBACK_LINES: usize = 10_000;

/// Cell attribute bits, mirrored by the Qt painter.
pub const FLAG_BOLD: u8 = 1 << 0;
pub const FLAG_ITALIC: u8 = 1 << 1;
pub const FLAG_UNDERLINE: u8 = 1 << 2;
pub const FLAG_STRIKEOUT: u8 = 1 << 3;
pub const FLAG_INVERSE: u8 = 1 << 4;
pub const FLAG_HIDDEN: u8 = 1 << 5;
pub const FLAG_WIDE: u8 = 1 << 6;
pub const FLAG_SPACER: u8 = 1 << 7;

/// One cell as the painter consumes it.
///
/// Colours are `0xAARRGGBB` (Qt's `QColor` byte order), already resolved for
/// inverse/dim/hidden. A wide character occupies its own cell; the following
/// spacer cell carries [`FLAG_SPACER`] and is not drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellSnapshot {
    pub ch: char,
    pub fg: u32,
    pub bg: u32,
    pub flags: u8,
}

impl Default for CellSnapshot {
    fn default() -> Self {
        Self::blank(true)
    }
}

impl CellSnapshot {
    /// A blank cell in the theme's own colours, for the grid slots the
    /// emulator never writes.
    fn blank(dark: bool) -> Self {
        let palette = palette(dark);
        Self {
            ch: ' ',
            fg: pack(palette[16]),
            bg: pack(palette[17]),
            flags: 0,
        }
    }
}

/// What a snapshot carries. The vendored libghostty-vt marks every update
/// fully dirty (a probe in `examples/dirty_probe.rs` showed `DIRTY_FULL`
/// and all rows dirty even for a bare cursor move), so per-row patching is
/// not available. What the library does expose is stable row identity: when
/// the viewport scrolled by `k` lines, every row keeps the id of the row
/// that moved there, so a shift frame sends only the `k` new bottom rows
/// and the painter moves the rest in its shadow grid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotKind {
    Full,
    /// The viewport scrolled up by `k`; `cells` holds the `k` new bottom
    /// rows, top to bottom.
    Shift(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSnapshot {
    pub cols: usize,
    pub rows: usize,
    pub kind: SnapshotKind,
    pub cursor_col: usize,
    pub cursor_row: usize,
    pub cursor_visible: bool,
    /// 0 block, 1 underline, 2 beam, 3 hollow block.
    pub cursor_shape: u8,
    pub display_offset: usize,
    /// Lines kept above the viewport. Together with `rows` this is what the
    /// scrollbar needs to size its thumb; `display_offset` places it.
    pub history_size: usize,
    /// For `Full`, the viewport top to bottom. For `Rows`, exactly the rows
    /// named in `kind`, each `cols` long, in ascending row order.
    pub cells: Vec<CellSnapshot>,
}

/// What the reader and waiter threads tell the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalEvent {
    /// New bytes were parsed into the grid.
    Output,
    /// The shell exited with this code.
    Exited(i32),
}

pub type TerminalNotifier = Arc<dyn Fn(TerminalEvent) + Send + Sync>;

pub struct TerminalSession {
    inner: Arc<Mutex<TerminalState>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    notify: TerminalNotifier,
    exited: Arc<AtomicBool>,
    exit_code: Arc<AtomicI32>,
    /// What the shell reported through OSC 7/9/1337 (decoded path) and
    /// OSC 0/2, filled by the reader thread's callbacks.
    reported_pwd: Arc<Mutex<Option<String>>>,
    reported_title: Arc<Mutex<Option<String>>>,
}

/// Context for the library's write-pty callback. The callback fires inside
/// `Vt::write` while the session mutex is held, so the writer lock is only
/// ever taken in one order (session then writer).
struct WritePtyUserdata {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    reported_pwd: Arc<Mutex<Option<String>>>,
    reported_title: Arc<Mutex<Option<String>>>,
}

unsafe extern "C" fn write_pty_callback(
    _terminal: ghostty_vt::GhosttyTerminal,
    userdata: *mut c_void,
    data: *const u8,
    len: usize,
) {
    let userdata = unsafe { &*(userdata as *const WritePtyUserdata) };
    if let Ok(mut writer) = userdata.writer.lock() {
        let data = unsafe { std::slice::from_raw_parts(data, len) };
        let _ = writer.write_all(data);
        let _ = writer.flush();
    }
}
/// The shell's OSC 7/9/1337 report. The value is stored verbatim except for
/// OSC 7, where a `file://` URI is decoded into the path the panel shows.
unsafe extern "C" fn pwd_changed_callback(
    terminal: ghostty_vt::GhosttyTerminal,
    userdata: *mut c_void,
) {
    let userdata = unsafe { &*(userdata as *const WritePtyUserdata) };
    if let Some(raw) =
        unsafe { ghostty_vt::terminal_string(terminal, ghostty_vt::TERMINAL_DATA_PWD) }
        && let Ok(mut reported) = userdata.reported_pwd.lock()
    {
        *reported = Some(decode_pwd_report(&raw));
    }
}

/// The shell's OSC 0/2 title report.
unsafe extern "C" fn title_changed_callback(
    terminal: ghostty_vt::GhosttyTerminal,
    userdata: *mut c_void,
) {
    let userdata = unsafe { &*(userdata as *const WritePtyUserdata) };
    if let Some(raw) =
        unsafe { ghostty_vt::terminal_string(terminal, ghostty_vt::TERMINAL_DATA_TITLE) }
        && let Ok(mut reported) = userdata.reported_title.lock()
    {
        *reported = Some(raw);
    }
}

/// OSC 7 carries a `file://host/path` URI; OSC 9 (ConEmu) and OSC 1337
/// (iTerm2) carry bare paths. A URI is percent-decoded and its host dropped
/// (`file://localhost/K:/x` and `file:///K:/x` both mean `K:/x`); a bare
/// value is kept as the path it already is.
fn decode_pwd_report(raw: &str) -> String {
    let Some(rest) = raw.strip_prefix("file://") else {
        return raw.to_string();
    };
    // Skip the host component; the path is what follows it.
    let after_host = match rest.find('/') {
        Some(at) => &rest[at..],
        None => rest,
    };
    let decoded = percent_decode(after_host);
    // A Windows drive path arrives as `/K:/x`; a POSIX path as `/home/u`.
    // Only the former loses its leading separator.
    if let Some(path) = decoded.strip_prefix('/')
        && let (Some(drive), Some(colon)) = (path.chars().next(), path.chars().nth(1))
        && drive.is_ascii_alphabetic()
        && colon == ':'
    {
        return path.to_string();
    }
    decoded
}

/// Minimal percent-decoding for the path component of a `file:` URI.
/// A malformed escape is kept verbatim; the panel shows the bytes the
/// shell sent rather than guessing.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' && at + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[at + 1..at + 3]).unwrap_or("");
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                at += 3;
                continue;
            }
        }
        out.push(bytes[at]);
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

struct TerminalState {
    /// Owned by the state and freed with it, *before* the terminal: the
    /// search holds tracked state inside the terminal that must be
    /// released first. Declared before `vt` so field-drop order frees it
    /// first.
    search: Option<ghostty_vt::Search>,
    vt: Vt<WritePtyUserdata>,
    cols: usize,
    rows: usize,
    /// The row ids of the last frame the painter received, so the next
    /// snapshot can collapse a scroll into a shift.
    last_ids: Vec<GhosttyRowId>,
    last_dark: Option<bool>,
}

impl TerminalSession {
    /// Spawns a shell in a new PTY.
    ///
    /// `cwd` is only honoured when it is a local directory; virtual paths
    /// (`\\wsl$`, SFTP) fall back to home so the shell never opens somewhere it
    /// cannot chdir. `notify` runs on the reader thread after every read.
    pub fn spawn(
        cwd: Option<PathBuf>,
        cols: u16,
        rows: u16,
        notify: TerminalNotifier,
        shell: Option<String>,
    ) -> Result<Self, String> {
        let cols = cols.max(1);
        let rows = rows.max(1);

        let mut command = shell_command(shell);
        if let Some(directory) = cwd.filter(|path| path.is_dir()) {
            command.cwd(directory);
        } else if let Some(home) = home_directory() {
            command.cwd(home);
        }
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        // Lets shell integration snippets (Assets/terminal-integration)
        // detect that they are running under dae, the way Zed's ZED_TERM
        // does; nothing else reads it.
        command.env("DAE_TERM", "1");

        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| format!("term.pty_create_failed: {error}"))?;
        let mut child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| format!("term.shell_spawn_failed: {error}"))?;
        let killer = child.clone_killer();
        let writer: Arc<Mutex<Box<dyn Write + Send>>> = Arc::new(Mutex::new(
            pair.master
                .take_writer()
                .map_err(|error| format!("term.pty_input_failed: {error}"))?,
        ));
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| format!("term.pty_output_failed: {error}"))?;
        // Dropping the slave keeps reads from hanging once the shell exits.
        drop(pair.slave);

        let reported_pwd = Arc::new(Mutex::new(None));
        let reported_title = Arc::new(Mutex::new(None));
        let vt = Vt::new(
            cols,
            rows,
            SCROLLBACK_LINES,
            write_pty_callback,
            title_changed_callback,
            pwd_changed_callback,
            Box::new(WritePtyUserdata {
                writer: Arc::clone(&writer),
                reported_pwd: Arc::clone(&reported_pwd),
                reported_title: Arc::clone(&reported_title),
            }),
        )?;
        let inner = Arc::new(Mutex::new(TerminalState {
            search: None,
            vt,
            cols: usize::from(cols),
            rows: usize::from(rows),
            last_ids: Vec::new(),
            last_dark: None,
        }));
        let exited = Arc::new(AtomicBool::new(false));
        let exit_code = Arc::new(AtomicI32::new(-1));

        thread::spawn({
            let inner = Arc::clone(&inner);
            let notify = Arc::clone(&notify);
            move || {
                let mut buffer = [0u8; 16 * 1024];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(read) => {
                            if let Ok(mut state) = inner.lock() {
                                state.vt.write(&buffer[..read]);
                            }
                            notify(TerminalEvent::Output);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
            }
        });

        thread::spawn({
            let exited = Arc::clone(&exited);
            let exit_code = Arc::clone(&exit_code);
            let notify = Arc::clone(&notify);
            move || {
                let code = match child.wait() {
                    Ok(status) => status.exit_code() as i32,
                    Err(_) => -1,
                };
                exit_code.store(code, Ordering::Release);
                exited.store(true, Ordering::Release);
                notify(TerminalEvent::Exited(code));
            }
        });

        Ok(Self {
            inner,
            writer,
            master: Mutex::new(pair.master),
            killer: Mutex::new(killer),
            notify,
            exited,
            exit_code,
            reported_pwd,
            reported_title,
        })
    }

    /// Writes raw bytes to the shell. Encoded keys and pasted text both land
    /// here; the renderer is responsible for turning Enter into `\r`.
    pub fn write(&self, bytes: &[u8]) -> Result<(), String> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| "term.input_unavailable".to_string())?;
        writer
            .write_all(bytes)
            .and_then(|()| writer.flush())
            .map_err(|error| format!("term.write_failed: {error}"))
    }

    /// Pastes `text` the way the running program asked to receive it: when
    /// it enabled bracketed paste (mode 2004) the bytes are wrapped in the
    /// 200/201 markers, so a vim or tmux sees one paste instead of a run
    /// of Enters; without it the text is sent as keystrokes, newlines as
    /// carriage returns. Unsafe control bytes are neutralised either way.
    pub fn paste(&self, text: &str) -> Result<(), String> {
        let bracketed = self
            .inner
            .lock()
            .map(|mut state| state.vt.mode(ghostty_vt::MODE_BRACKETED_PASTE))
            .unwrap_or(false);

        // `ghostty_paste_encode` modifies the buffer in place; keep a copy.
        let mut data = text.as_bytes().to_vec();
        let mut needed: usize = 0;
        let query = unsafe {
            ghostty_vt::ghostty_paste_encode(
                data.as_mut_ptr(),
                data.len(),
                bracketed,
                std::ptr::null_mut(),
                0,
                &mut needed,
            )
        };
        if query != ghostty_vt::GHOSTTY_OUT_OF_SPACE {
            return Err("term.paste_encode_failed".to_string());
        }
        let mut encoded = vec![0u8; needed.max(1)];
        let mut written: usize = 0;
        let result = unsafe {
            ghostty_vt::ghostty_paste_encode(
                data.as_mut_ptr(),
                data.len(),
                bracketed,
                encoded.as_mut_ptr(),
                encoded.len(),
                &mut written,
            )
        };
        if result != ghostty_vt::GHOSTTY_SUCCESS {
            return Err("term.paste_encode_failed".to_string());
        }
        self.write(&encoded[..written])
    }

    /// What the wheel does with `steps` (positive scrolls up): on the
    /// alternate screen the program owns the viewport, so the wheel
    /// becomes arrow keys; on the primary screen it walks the history.
    /// Returns `true` when the alternate screen took the wheel.
    pub fn wheel_scroll(&self, steps: i32) -> Result<bool, String> {
        let alternate = self
            .inner
            .lock()
            .map(|mut state| state.vt.alternate_screen())
            .unwrap_or(false);
        if !alternate {
            self.scroll(steps);
            return Ok(false);
        }
        let sequence: &[u8] = if steps >= 0 { b"\x1b[A" } else { b"\x1b[B" };
        for _ in 0..steps.unsigned_abs() {
            self.write(sequence)?;
        }
        Ok(true)
    }

    /// Resizes both the emulator grid and the PTY. The shell learns the new
    /// size through `TIOCSWINSZ` / `ResizePseudoConsole`.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        let cols = cols.max(1);
        let rows = rows.max(1);
        {
            let mut state = self
                .inner
                .lock()
                .map_err(|_| "term.state_unavailable".to_string())?;
            state.vt.resize(cols, rows);
            state.cols = usize::from(cols);
            state.rows = usize::from(rows);
            // The next frame has new dimensions; the painter's shadow grid
            // and the remembered row ids cannot carry over.
            state.last_ids.clear();
        }
        let master = self
            .master
            .lock()
            .map_err(|_| "term.pty_unavailable".to_string())?;
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| format!("term.resize_failed: {error}"))
    }

    /// Moves the viewport through scrollback. Positive scrolls up.
    pub fn scroll(&self, lines: i32) {
        if let Ok(mut state) = self.inner.lock() {
            state.vt.scroll_delta(-(lines as isize));
        }
    }

    /// Moves the viewport to an absolute scrollback offset, 0 being the live
    /// bottom. A scrollbar drag cannot be expressed as a fixed delta because
    /// output may land between two pointer events and shift the history.
    pub fn scroll_to(&self, offset: usize) {
        if let Ok(mut state) = self.inner.lock()
            && let Some((total, _current, len)) = state.vt.scrollbar()
        {
            let history = total.saturating_sub(len);
            state
                .vt
                .scroll_to_row(history.saturating_sub(offset as u64) as usize);
        }
    }

    /// The grid as the painter consumes it. `dark` picks the palette the
    /// renderer is drawing against; the cells carry resolved colours, so a
    /// theme flip is a fresh snapshot rather than a repaint of stale ones.
    ///
    /// The first frame after a spawn or a resize, and every frame after a
    /// theme flip, is full. A frame whose rows are all the previous frame's
    /// rows moved up by `k`, plus `k` new ones, is a shift: only the new
    /// rows cross the bridge.
    pub fn snapshot(&self, dark: bool) -> Option<TerminalSnapshot> {
        let mut state = self.inner.lock().ok()?;
        if !state.vt.update() {
            return None;
        }
        let cols = state.cols;
        let rows = state.rows;
        let palette = palette(dark);

        let (history_size, display_offset) = state
            .vt
            .scrollbar()
            .map(|(total, offset, len)| {
                let history = total.saturating_sub(len);
                (history as usize, history.saturating_sub(offset) as usize)
            })
            .unwrap_or((0, 0));

        let (cursor_col, cursor_row, cursor_shape, cursor_visible) = match state.vt.cursor() {
            Some((x, y, shape)) if usize::from(x) < cols && usize::from(y) < rows => {
                (usize::from(x), usize::from(y), shape, true)
            }
            _ => (0, 0, 0, false),
        };

        let mut ids = vec![GhosttyRowId { bits: [0; 2] }; rows];
        let mut cells = vec![CellSnapshot::blank(dark); cols.saturating_mul(rows)];
        state.vt.visit_rows(|y, mut row| {
            if y < 0 || y as usize >= rows {
                return;
            }
            ids[y as usize] = row.id();
            while let Some((x, raw, style)) = row.next_cell() {
                if usize::from(x) >= cols {
                    break;
                }
                cells[y as usize * cols + usize::from(x)] = cell_snapshot(raw, &style, palette);
            }
        });

        // A theme flip recolours every cell, and a shadow grid from a
        // different palette cannot be patched; those, and anything whose
        // rows are not a pure scroll of the last frame, go out whole.
        let pure_scroll = state.last_ids.len() == rows && state.last_dark == Some(dark);
        let shift = pure_scroll
            .then(|| {
                (1..rows)
                    .filter(|&k| (0..rows - k).all(|i| ids[i] == state.last_ids[i + k]))
                    .max()
            })
            .flatten();

        let (kind, cells) = match shift {
            Some(k) => (SnapshotKind::Shift(k), cells.split_off((rows - k) * cols)),
            None => (SnapshotKind::Full, cells),
        };

        state.last_ids = ids;
        state.last_dark = Some(dark);

        Some(TerminalSnapshot {
            cols,
            rows,
            kind,
            cursor_col,
            cursor_row,
            cursor_visible,
            cursor_shape,
            display_offset,
            history_size,
            cells,
        })
    }

    pub fn is_running(&self) -> bool {
        !self.exited.load(Ordering::Acquire)
    }

    /// The working directory the shell last reported (OSC 7/9/1337), as a
    /// decoded path. `None` until the shell's integration sends its first
    /// report; the UI falls back to the directory the shell spawned in.
    pub fn working_directory(&self) -> Option<String> {
        self.reported_pwd.lock().ok().and_then(|guard| guard.clone())
    }

    /// The window title the shell last set with OSC 0/2. `None` before the
    /// first report.
    pub fn window_title(&self) -> Option<String> {
        self.reported_title.lock().ok().and_then(|guard| guard.clone())
    }

    /// Sets the search needle, or clears it with `None`. A cleared needle
    /// drops the search handle so repaints stop drawing highlights.
    pub fn set_search_needle(&self, needle: Option<&str>) {
        if let Ok(mut state) = self.inner.lock() {
            match (needle, state.search.as_mut()) {
                (Some(needle), Some(search)) => {
                    search.set_needle(Some(needle));
                    search.run();
                }
                (Some(needle), None) => {
                    if let Ok(mut search) = state.vt.create_search() {
                        search.set_needle(Some(needle));
                        search.run();
                        state.search = Some(search);
                    }
                }
                (None, _) => state.search = None,
            }
        }
    }

    /// Selects the next match toward older content (`forward`) or the
    /// previous one, scrolling it into view, and reports `(total,
    /// selected index)` for the find bar's "k of n".
    pub fn search_step(&self, forward: bool) -> Option<(usize, usize)> {
        self.inner.lock().ok().and_then(|mut state| {
            let search = state.search.as_mut()?;
            search.run();
            search.step(forward)
        })
    }

    /// The matches on the visible pages in viewport coordinates, for the
    /// painter's highlight rectangles.
    pub fn search_viewport(&self) -> Vec<ghostty_vt::SearchRect> {
        if let Ok(mut state) = self.inner.lock() {
            let (cols, rows) = (state.cols, state.rows);
            if let Some(search) = state.search.as_mut() {
                return search.viewport_matches(cols as u16, rows as u16);
            }
        }
        Vec::new()
    }

    pub fn exit_code(&self) -> i32 {
        self.exit_code.load(Ordering::Acquire)
    }

    pub fn kill(&self) {
        if let Ok(mut killer) = self.killer.lock() {
            let _ = killer.kill();
        }
        self.exited.store(true, Ordering::Release);
        (self.notify)(TerminalEvent::Exited(-1));
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        if self.is_running()
            && let Ok(mut killer) = self.killer.lock()
        {
            let _ = killer.kill();
        }
    }
}

fn cell_snapshot(raw: u64, style: &GhosttyStyle, palette: &[Rgb; 19]) -> CellSnapshot {
    let mut fg = resolve_color(style.fg_color, palette[16], palette);
    let mut bg = resolve_color(style.bg_color, palette[17], palette);
    let mut flags = 0u8;

    if style.flags & ghostty_vt::STYLE_FLAG_BOLD != 0 {
        flags |= FLAG_BOLD;
    }
    if style.flags & ghostty_vt::STYLE_FLAG_ITALIC != 0 {
        flags |= FLAG_ITALIC;
    }
    if style.flags & ghostty_vt::STYLE_UNDERLINE_MASK != 0 {
        flags |= FLAG_UNDERLINE;
    }
    if style.flags & ghostty_vt::STYLE_FLAG_STRIKETHROUGH != 0 {
        flags |= FLAG_STRIKEOUT;
    }
    if style.flags & ghostty_vt::STYLE_FLAG_INVERSE != 0 {
        flags |= FLAG_INVERSE;
        std::mem::swap(&mut fg, &mut bg);
    }
    if style.flags & ghostty_vt::STYLE_FLAG_INVISIBLE != 0 {
        flags |= FLAG_HIDDEN;
        fg = bg;
    }
    match ghostty_vt::cell_wide(raw) {
        ghostty_vt::CELL_WIDE_WIDE => flags |= FLAG_WIDE,
        ghostty_vt::CELL_WIDE_SPACER_TAIL | ghostty_vt::CELL_WIDE_SPACER_HEAD => {
            flags |= FLAG_SPACER;
        }
        _ => {}
    }
    if style.flags & ghostty_vt::STYLE_FLAG_FAINT != 0 {
        fg = dim(fg);
    }

    let ch = if flags & FLAG_HIDDEN != 0 {
        ' '
    } else {
        // Empty cells report codepoint 0; the grid stores blanks as spaces.
        match char::from_u32(ghostty_vt::cell_codepoint(raw)) {
            Some('\0') | None => ' ',
            Some(ch) => ch,
        }
    };

    CellSnapshot { ch, fg, bg, flags }
}

fn resolve_color(color: GhosttyStyleColor, default: Rgb, palette: &[Rgb; 19]) -> u32 {
    if let Some(index) = color.palette_index() {
        let rgb = if index < 16 {
            palette[usize::from(index)]
        } else {
            indexed(index)
        };
        return pack(rgb);
    }
    if let Some(rgb) = color.rgb() {
        return pack(Rgb {
            r: rgb.r,
            g: rgb.g,
            b: rgb.b,
        });
    }
    pack(default)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rgb {
    r: u8,
    g: u8,
    b: u8,
}

fn indexed(index: u8) -> Rgb {
    if index >= 232 {
        let level = 8 + (usize::from(index) - 232) * 10;
        let level = level as u8;
        return Rgb {
            r: level,
            g: level,
            b: level,
        };
    }
    let index = usize::from(index) - 16;
    let steps = [0u8, 95, 135, 175, 215, 255];
    Rgb {
        r: steps[index / 36],
        g: steps[(index / 6) % 6],
        b: steps[index % 6],
    }
}

/// The shell palette, chosen for the theme the terminal is drawn against.
///
/// The neutral slots track `Theme.canvas` / `Theme.ink`, so the grid meets the
/// panel without a seam. The semantic slots use the same hues as the rest of
/// the shell: danger red, success green, warning yellow, code blue, font
/// magenta and accent teal. The light table keeps the ink dark and the fills
/// light because white-on-white is the one thing it cannot print.
fn palette(dark: bool) -> &'static [Rgb; 19] {
    const fn rgb(r: u8, g: u8, b: u8) -> Rgb {
        Rgb { r, g, b }
    }
    const DARK: [Rgb; 19] = [
        rgb(0x29, 0x2b, 0x30), // black
        rgb(0xf4, 0x7b, 0x74), // red
        rgb(0x7b, 0xc4, 0x95), // green
        rgb(0xe6, 0xb5, 0x5d), // yellow
        rgb(0x95, 0xc1, 0xdc), // blue
        rgb(0xbb, 0xb4, 0xdf), // magenta
        rgb(0x6b, 0xca, 0xc9), // cyan
        rgb(0xbc, 0xbe, 0xc2), // white
        rgb(0x6b, 0x6d, 0x72), // bright black
        rgb(0xf7, 0x9a, 0x94), // bright red
        rgb(0xa2, 0xd6, 0xb2), // bright green
        rgb(0xf0, 0xc8, 0x77), // bright yellow
        rgb(0xb3, 0xd5, 0xe8), // bright blue
        rgb(0xd0, 0xc8, 0xec), // bright magenta
        rgb(0x8f, 0xdc, 0xdb), // bright cyan
        rgb(0xf7, 0xf7, 0xf9), // bright white
        rgb(0xf7, 0xf7, 0xf9), // foreground
        rgb(0x10, 0x12, 0x16), // background — `Theme.canvas`
        rgb(0x6b, 0xca, 0xc9), // cursor — `Theme.accent`
    ];
    const LIGHT: [Rgb; 19] = [
        rgb(0x1c, 0x1e, 0x23), // black — the ink
        rgb(0xba, 0x2b, 0x2e), // red
        rgb(0x22, 0x78, 0x49), // green
        rgb(0x8d, 0x5a, 0x00), // yellow
        rgb(0x33, 0x6a, 0x88), // blue
        rgb(0x64, 0x5b, 0x8b), // magenta
        rgb(0x00, 0x7a, 0x7b), // cyan
        rgb(0xd7, 0xd9, 0xdc), // white — a fill
        rgb(0x6b, 0x6d, 0x72), // bright black — dim text
        rgb(0xa3, 0x27, 0x2a), // bright red
        rgb(0x1a, 0x5f, 0x3a), // bright green
        rgb(0x7a, 0x4e, 0x00), // bright yellow
        rgb(0x2b, 0x5b, 0x76), // bright blue
        rgb(0x55, 0x4b, 0x7a), // bright magenta
        rgb(0x00, 0x69, 0x6a), // bright cyan
        rgb(0xf7, 0xf7, 0xf9), // bright white
        rgb(0x1c, 0x1e, 0x23), // foreground
        rgb(0xfd, 0xfd, 0xfd), // background — `Theme.canvas`
        rgb(0x00, 0x7a, 0x7b), // cursor — `Theme.accent`
    ];
    if dark { &DARK } else { &LIGHT }
}

fn pack(rgb: Rgb) -> u32 {
    0xff00_0000 | (u32::from(rgb.r) << 16) | (u32::from(rgb.g) << 8) | u32::from(rgb.b)
}

fn dim(argb: u32) -> u32 {
    let r = ((argb >> 16) & 0xff) * 2 / 3;
    let g = ((argb >> 8) & 0xff) * 2 / 3;
    let b = (argb & 0xff) * 2 / 3;
    (argb & 0xff00_0000) | (r << 16) | (g << 8) | b
}

fn home_directory() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

/// A configured shell wins over the platform guess. It is handed to the PTY as
/// the program itself — no guessed `-NoLogo` and no argument splitting — so a
/// path with spaces stays one path.
fn shell_command(shell: Option<String>) -> CommandBuilder {
    if let Some(program) = shell {
        return CommandBuilder::new(program);
    }
    default_shell_command()
}

#[cfg(target_os = "windows")]
fn default_shell_command() -> CommandBuilder {
    let (program, args): (&str, &[&str]) = if is_in_path("pwsh.exe") {
        ("pwsh.exe", &["-NoLogo"])
    } else if is_in_path("powershell.exe") {
        ("powershell.exe", &["-NoLogo"])
    } else {
        ("cmd.exe", &[])
    };
    let mut command = CommandBuilder::new(program);
    command.args(args);
    command
}

#[cfg(target_os = "windows")]
fn is_in_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|directory| directory.join(program).is_file())
        })
        .unwrap_or(false)
}

#[cfg(not(target_os = "windows"))]
fn default_shell_command() -> CommandBuilder {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|path| !path.is_empty() && PathBuf::from(path).exists())
        .or_else(|| {
            [
                "/bin/zsh",
                "/usr/bin/zsh",
                "/bin/bash",
                "/usr/bin/bash",
                "/bin/sh",
            ]
            .into_iter()
            .find(|path| PathBuf::from(path).exists())
            .map(str::to_string)
        });
    CommandBuilder::new(shell.unwrap_or_else(|| "/bin/sh".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc7_uris_decode_to_plain_paths() {
        // Windows drive paths arrive with an empty host, so the path is
        // `/K:/code` and the leading separator must go.
        assert_eq!(decode_pwd_report("file:///K:/code"), "K:/code");
        assert_eq!(decode_pwd_report("file://localhost/K:/code"), "K:/code");
        // POSIX paths keep their leading separator.
        assert_eq!(decode_pwd_report("file://localhost/home/u"), "/home/u");
        assert_eq!(decode_pwd_report("file:///home/u"), "/home/u");
        // Percent-encoded UTF-8 and spaces come back as the bytes the shell
        // had; a malformed escape is kept verbatim rather than dropped.
        assert_eq!(decode_pwd_report("file:///a%20b"), "/a b");
        assert_eq!(decode_pwd_report("file:///%E4%B8%AD"), "/中");
        // `%2b` is a complete escape (+); a truncated `%2` at the end of
        // the value is kept verbatim rather than dropped.
        assert_eq!(decode_pwd_report("file:///a%2b"), "/a+");
        assert_eq!(decode_pwd_report("file:///a%2"), "/a%2");
        // OSC 9 (ConEmu) and OSC 1337 (iTerm2) carry bare paths already.
        assert_eq!(decode_pwd_report("K:\\code"), "K:\\code");
        assert_eq!(decode_pwd_report(""), "");
    }

    #[test]
    fn ansi_palette_covers_the_three_ranges() {
        assert_eq!(indexed(16), Rgb { r: 0, g: 0, b: 0 });
        assert_eq!(indexed(196), Rgb { r: 255, g: 0, b: 0 });
        assert_eq!(indexed(232), Rgb { r: 8, g: 8, b: 8 });
        assert_eq!(
            indexed(255),
            Rgb {
                r: 238,
                g: 238,
                b: 238
            }
        );
    }

    /// The two default cells have to meet their panel: the light grid paints
    /// ink on canvas, the dark one canvas ink on near-black.
    #[test]
    fn the_default_cell_follows_the_theme() {
        let light = CellSnapshot::blank(false);
        assert_eq!(light.fg, 0xff1c_1e23);
        assert_eq!(light.bg, 0xfffd_fdfd);
        let dark = CellSnapshot::blank(true);
        assert_eq!(dark.fg, 0xfff7_f7f9);
        assert_eq!(dark.bg, 0xff10_1216);
    }

    #[test]
    fn packed_colours_are_opaque_argb() {
        assert_eq!(
            pack(Rgb {
                r: 0x12,
                g: 0x34,
                b: 0x56
            }),
            0xff12_3456
        );
        assert_eq!(dim(0xff30_3030) >> 24, 0xff);
    }
}
