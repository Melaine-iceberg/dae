//! PTY-backed terminal sessions with an alacritty grid.
//!
//! The shell's terminal is not a string buffer: `alacritty_terminal` parses the
//! escape sequences and owns the cell grid, scrollback, cursor and modes, while
//! `portable-pty` owns the platform PTY. This module is the engine-free half —
//! it produces a [`TerminalSnapshot`] of styled cells and knows nothing about
//! Qt or any other renderer. `crates/dae-qt` paints that snapshot.
//!
//! Why not alacritty's own event loop: it is built around one terminal per
//! process and a window-id event proxy. A file manager needs several sessions
//! whose lifetime follows tabs and windows, so the reader thread here feeds
//! `vte::ansi::Processor` directly. That is the same parser alacritty uses.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Processor, Rgb};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

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
        Self {
            ch: ' ',
            fg: 0xffd8dee9,
            bg: 0xff14161a,
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
    size: Arc<Mutex<(u16, u16)>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    notify: TerminalNotifier,
    exited: Arc<AtomicBool>,
    exit_code: Arc<AtomicI32>,
}

/// The terminal's side of the conversation.
///
/// `alacritty_terminal` answers terminal queries (`CSI 6n`, device attributes,
/// text-area size) by sending `Event::PtyWrite` to this listener. Dropping
/// those answers leaves a shell such as PowerShell waiting forever for a cursor
/// position before it draws its first prompt, so the listener writes them back
/// to the same PTY the user's keystrokes go to.
struct PtyListener {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    size: Arc<Mutex<(u16, u16)>>,
}

impl EventListener for PtyListener {
    fn send_event(&self, event: Event) {
        let response = match event {
            Event::PtyWrite(text) => text,
            Event::TextAreaSizeRequest(format) => {
                let (rows, cols) = *self.size.lock().unwrap_or_else(|error| error.into_inner());
                format(WindowSize {
                    num_lines: rows,
                    num_cols: cols,
                    cell_width: 0,
                    cell_height: 0,
                })
            }
            _ => return,
        };
        if let Ok(mut writer) = self.writer.lock() {
            let _ = writer.write_all(response.as_bytes());
            let _ = writer.flush();
        }
    }
}

struct TerminalState {
    term: Term<PtyListener>,
    processor: Processor,
    cols: usize,
    rows: usize,
}

#[derive(Clone, Copy)]
struct Size {
    cols: usize,
    rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.cols
    }
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
    ) -> Result<Self, String> {
        let cols = cols.max(1);
        let rows = rows.max(1);

        let mut command = shell_command();
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

        let size = Arc::new(Mutex::new((rows, cols)));
        let term = Term::new(
            Config {
                scrolling_history: SCROLLBACK_LINES,
                ..Config::default()
            },
            &Size {
                cols: usize::from(cols),
                rows: usize::from(rows),
            },
            PtyListener {
                writer: Arc::clone(&writer),
                size: Arc::clone(&size),
            },
        );
        let inner = Arc::new(Mutex::new(TerminalState {
            term,
            processor: Processor::default(),
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
                                let TerminalState {
                                    term, processor, ..
                                } = &mut *state;
                                processor.advance(term, &buffer[..read]);
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
            size,
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
            state.term.resize(Size {
                cols: usize::from(cols),
                rows: usize::from(rows),
            });
            state.cols = usize::from(cols);
            state.rows = usize::from(rows);
        }
        if let Ok(mut size) = self.size.lock() {
            *size = (rows, cols);
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
            state.term.scroll_display(Scroll::Delta(lines));
        }
    }

    pub fn snapshot(&self) -> Option<TerminalSnapshot> {
        let state = self.inner.lock().ok()?;
        let content = state.term.renderable_content();
        let cols = state.cols;
        let rows = state.rows;
        let mut cells = vec![CellSnapshot::default(); cols.saturating_mul(rows)];

        for indexed in content.display_iter {
            let row = indexed.point.line.0 + content.display_offset as i32;
            let column = indexed.point.column.0;
            if row < 0 || row as usize >= rows || column >= cols {
                continue;
            }
            cells[row as usize * cols + column] = cell_snapshot(indexed.cell);
        }

        let cursor = content.cursor;
        let cursor_row = cursor.point.line.0 + content.display_offset as i32;
        let cursor_col = cursor.point.column.0;
        let in_view = cursor_row >= 0
            && (cursor_row as usize) < rows
            && cursor_col < cols
            && cursor.shape != CursorShape::Hidden;

        Some(TerminalSnapshot {
            cols,
            rows,
            cursor_col,
            cursor_row: cursor_row.max(0) as usize,
            cursor_visible: in_view,
            cursor_shape: cursor_shape_code(cursor.shape),
            display_offset: content.display_offset,
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

fn cell_snapshot(cell: &alacritty_terminal::term::cell::Cell) -> CellSnapshot {
    use alacritty_terminal::term::cell::Flags;

    let mut fg = resolve_color(cell.fg);
    let mut bg = resolve_color(cell.bg);
    let mut flags = 0u8;

    if cell.flags.contains(Flags::BOLD) {
        flags |= FLAG_BOLD;
    }
    if cell.flags.intersects(Flags::ITALIC | Flags::BOLD_ITALIC) {
        flags |= FLAG_ITALIC;
    }
    if cell.flags.intersects(Flags::ALL_UNDERLINES) {
        flags |= FLAG_UNDERLINE;
    }
    if cell.flags.contains(Flags::STRIKEOUT) {
        flags |= FLAG_STRIKEOUT;
    }
    if cell.flags.contains(Flags::INVERSE) {
        flags |= FLAG_INVERSE;
        std::mem::swap(&mut fg, &mut bg);
    }
    if cell.flags.contains(Flags::HIDDEN) {
        flags |= FLAG_HIDDEN;
        fg = bg;
    }
    if cell.flags.contains(Flags::WIDE_CHAR) {
        flags |= FLAG_WIDE;
    }
    if cell
        .flags
        .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
    {
        flags |= FLAG_SPACER;
    }
    if cell.flags.contains(Flags::DIM | Flags::DIM_BOLD) {
        fg = dim(fg);
    }

    CellSnapshot {
        ch: if flags & FLAG_HIDDEN != 0 {
            ' '
        } else {
            cell.c
        },
        fg,
        bg,
        flags,
    }
}

fn cursor_shape_code(shape: CursorShape) -> u8 {
    match shape {
        CursorShape::Block => 0,
        CursorShape::Underline => 1,
        CursorShape::Beam => 2,
        CursorShape::HollowBlock => 3,
        CursorShape::Hidden => 0,
    }
}

fn resolve_color(color: Color) -> u32 {
    match color {
        Color::Spec(rgb) => pack(rgb),
        Color::Indexed(index) => pack(indexed(index)),
        Color::Named(named) => pack(named_color(named)),
    }
}

fn named_color(color: NamedColor) -> Rgb {
    let palette = palette();
    match color {
        NamedColor::Black => palette[0],
        NamedColor::Red => palette[1],
        NamedColor::Green => palette[2],
        NamedColor::Yellow => palette[3],
        NamedColor::Blue => palette[4],
        NamedColor::Magenta => palette[5],
        NamedColor::Cyan => palette[6],
        NamedColor::White => palette[7],
        NamedColor::BrightBlack => palette[8],
        NamedColor::BrightRed => palette[9],
        NamedColor::BrightGreen => palette[10],
        NamedColor::BrightYellow => palette[11],
        NamedColor::BrightBlue => palette[12],
        NamedColor::BrightMagenta => palette[13],
        NamedColor::BrightCyan => palette[14],
        NamedColor::BrightWhite => palette[15],
        NamedColor::Foreground | NamedColor::BrightForeground => palette[16],
        NamedColor::DimForeground => dim_rgb(palette[16]),
        NamedColor::Background => palette[17],
        NamedColor::Cursor => palette[18],
        NamedColor::DimBlack => dim_rgb(palette[0]),
        NamedColor::DimRed => dim_rgb(palette[1]),
        NamedColor::DimGreen => dim_rgb(palette[2]),
        NamedColor::DimYellow => dim_rgb(palette[3]),
        NamedColor::DimBlue => dim_rgb(palette[4]),
        NamedColor::DimMagenta => dim_rgb(palette[5]),
        NamedColor::DimCyan => dim_rgb(palette[6]),
        NamedColor::DimWhite => dim_rgb(palette[7]),
    }
}

fn indexed(index: u8) -> Rgb {
    if index < 16 {
        return palette()[usize::from(index)];
    }
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

/// The shell palette. Chosen to sit next to the listing's dark chrome rather
/// than to be a literal copy of any terminal's default theme.
fn palette() -> [Rgb; 19] {
    const fn rgb(r: u8, g: u8, b: u8) -> Rgb {
        Rgb { r, g, b }
    }
    [
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
        rgb(0x14, 0x16, 0x1a), // background
        rgb(0x6e, 0xa8, 0xfe), // cursor
    ]
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

fn dim_rgb(rgb: Rgb) -> Rgb {
    Rgb {
        r: rgb.r * 2 / 3,
        g: rgb.g * 2 / 3,
        b: rgb.b * 2 / 3,
    }
}

fn home_directory() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

#[cfg(target_os = "windows")]
fn shell_command() -> CommandBuilder {
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
fn shell_command() -> CommandBuilder {
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
        assert_eq!(indexed(1), palette()[1]);
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
