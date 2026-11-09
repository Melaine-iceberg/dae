//! PTY-backed terminal sessions with a libghostty-vt grid.
//!
//! The shell's terminal is not a string buffer: libghostty-vt parses the
//! escape sequences and owns the cell grid, scrollback, cursor and modes,
//! while `portable-pty` owns the platform PTY. This module is the engine-free
//! half — it produces a [`TerminalSnapshot`] of styled cells and knows nothing
//! about Qt or any other renderer. `crates/dae-qt` paints that snapshot.
//!
//! The VT engine is the vendored libghostty-vt static library (see
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

use crate::ghostty_vt::{self, GhosttyStyle, GhosttyStyleColor, Vt};

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSnapshot {
    pub cols: usize,
    pub rows: usize,
    pub cursor_col: usize,
    pub cursor_row: usize,
    pub cursor_visible: bool,
    /// 0 block, 1 underline, 2 beam, 3 hollow block.
    pub cursor_shape: u8,
    pub display_offset: usize,
    /// Lines kept above the viewport. Together with `rows` this is what the
    /// scrollbar needs to size its thumb; `display_offset` places it.
    pub history_size: usize,
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
}

/// Context for the library's write-pty callback. The callback fires inside
/// `Vt::write` while the session mutex is held, so the writer lock is only
/// ever taken in one order (session then writer).
struct WritePtyUserdata {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
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

struct TerminalState {
    vt: Vt<WritePtyUserdata>,
    cols: usize,
    rows: usize,
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

        let vt = Vt::new(
            cols,
            rows,
            SCROLLBACK_LINES,
            write_pty_callback,
            Box::new(WritePtyUserdata {
                writer: Arc::clone(&writer),
            }),
        )?;
        let inner = Arc::new(Mutex::new(TerminalState {
            vt,
            cols: usize::from(cols),
            rows: usize::from(rows),
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

        let mut cells = vec![CellSnapshot::blank(dark); cols.saturating_mul(rows)];
        state.vt.visit_rows(|y, mut row| {
            if y < 0 || y as usize >= rows {
                return;
            }
            while let Some((x, raw, style)) = row.next_cell() {
                if usize::from(x) >= cols {
                    break;
                }
                cells[y as usize * cols + usize::from(x)] = cell_snapshot(raw, &style, palette);
            }
        });

        let (cursor_col, cursor_row, cursor_shape, cursor_visible) = match state.vt.cursor() {
            Some((x, y, shape)) if usize::from(x) < cols && usize::from(y) < rows => {
                (usize::from(x), usize::from(y), shape, true)
            }
            _ => (0, 0, 0, false),
        };

        Some(TerminalSnapshot {
            cols,
            rows,
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
/// The dark table sits next to the listing's dark chrome rather than copying a
/// terminal's default theme. The light table is the matching daylight read:
/// slot 0 carries the ink and slots 7/15 are fills, because white-on-white is
/// the one thing a paper terminal cannot print. The neutral slots track
/// `Theme.canvas` / `Theme.ink` so the grid meets the panel without a seam.
fn palette(dark: bool) -> &'static [Rgb; 19] {
    const fn rgb(r: u8, g: u8, b: u8) -> Rgb {
        Rgb { r, g, b }
    }
    const DARK: [Rgb; 19] = [
        rgb(0x1c, 0x1f, 0x24), // black
        rgb(0xe0, 0x6c, 0x75), // red
        rgb(0x98, 0xc3, 0x79), // green
        rgb(0xe5, 0xc0, 0x7b), // yellow
        rgb(0x61, 0xaf, 0xef), // blue
        rgb(0xc6, 0x78, 0xdd), // magenta
        rgb(0x56, 0xb6, 0xc2), // cyan
        rgb(0xdc, 0xdf, 0xe4), // white
        rgb(0x5c, 0x63, 0x70), // bright black
        rgb(0xff, 0x7b, 0x86), // bright red
        rgb(0xb5, 0xe8, 0x90), // bright green
        rgb(0xff, 0xd6, 0x8a), // bright yellow
        rgb(0x82, 0xc4, 0xff), // bright blue
        rgb(0xe2, 0x9b, 0xf0), // bright magenta
        rgb(0x7f, 0xd6, 0xdf), // bright cyan
        rgb(0xff, 0xff, 0xff), // bright white
        rgb(0xd8, 0xde, 0xe9), // foreground
        rgb(0x0b, 0x0b, 0x0c), // background — the same near-black as `Theme.terminalCanvas`
        rgb(0x6e, 0xa8, 0xfe), // cursor
    ];
    const LIGHT: [Rgb; 19] = [
        rgb(0x10, 0x12, 0x16), // black — the ink
        rgb(0xc2, 0x2f, 0x2f), // red
        rgb(0x17, 0x80, 0x4a), // green
        rgb(0xc7, 0x86, 0x00), // yellow
        rgb(0x3f, 0x68, 0xc8), // blue
        rgb(0xb8, 0x43, 0x8a), // magenta
        rgb(0x0a, 0x86, 0xb4), // cyan
        rgb(0xd2, 0xd7, 0xde), // white — the paper step, a fill
        rgb(0x5b, 0x61, 0x69), // bright black — dim text
        rgb(0xc1, 0x34, 0x34), // bright red
        rgb(0x1f, 0x9a, 0x5c), // bright green
        rgb(0xe8, 0x9d, 0x01), // bright yellow
        rgb(0x5b, 0x7f, 0xd6), // bright blue
        rgb(0xcb, 0x5f, 0xa4), // bright magenta
        rgb(0x22, 0xa3, 0xc8), // bright cyan
        rgb(0xee, 0xf0, 0xf3), // bright white — the canvas below the card, a fill
        rgb(0x10, 0x12, 0x16), // foreground
        rgb(0xfb, 0xfb, 0xf9), // background — the same paper as `Theme.terminalCanvas`
        rgb(0x1f, 0x63, 0xcc), // cursor
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
    /// ink on paper, the dark one paints paper on ink.
    #[test]
    fn the_default_cell_follows_the_theme() {
        let light = CellSnapshot::blank(false);
        assert_eq!(light.fg, 0xff10_1216);
        assert_eq!(light.bg, 0xfffb_fbf9);
        let dark = CellSnapshot::blank(true);
        assert_eq!(dark.fg, 0xffd8_dee9);
        assert_eq!(dark.bg, 0xff0b_0b0c);
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
