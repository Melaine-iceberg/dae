//! Hand-rolled bindings to the vendored libghostty-vt C API.
//!
//! Only the surface `pty.rs` needs is declared here; the authoritative
//! headers sit next to the static library in `ghostty-vt/include`. The C API
//! is pre-1.0, so the vendored version is pinned in `ghostty-vt/VERSION`.
//!
//! Layout notes that matter for the `#[repr(C)]` mirrors below:
//! * every `GHOSTTY_ENUM_TYPED` enum is a C `int` (32 bit);
//! * `GhosttyStyle` is a fixed 16-byte struct on every platform;
//! * sized structs (`GhosttyRenderStateCursor`) start with a `size` field
//!   that must carry `sizeof` the struct the binding was built from.

use std::ffi::c_void;
use std::mem::size_of;
use std::os::raw::c_int;

pub type GhosttyResult = c_int;
pub const GHOSTTY_SUCCESS: GhosttyResult = 0;

pub type GhosttyTerminal = *mut c_void;
pub type GhosttyRenderState = *mut c_void;
pub type GhosttyRenderStateRowIterator = *mut c_void;
pub type GhosttyRenderStateRowCells = *mut c_void;
pub type GhosttyCell = u64;

// ghostty_terminal_set options
pub const TERMINAL_OPT_USERDATA: c_int = 1;
pub const TERMINAL_OPT_WRITE_PTY: c_int = 2;
pub const TERMINAL_OPT_SCROLLBACK_MAX_BYTES: c_int = 28;
pub const TERMINAL_OPT_SCROLLBACK_MAX_LINES: c_int = 29;

// ghostty_terminal_get data kinds
pub const TERMINAL_DATA_SCROLLBAR: c_int = 9;

// ghostty_render_state_get data kinds
pub const RENDER_STATE_DATA_ROW_ITERATOR: c_int = 4;
pub const RENDER_STATE_DATA_CURSOR: c_int = 18;

// ghostty_render_state_row_get data kinds
pub const ROW_DATA_CELLS: c_int = 3;
pub const ROW_DATA_VIEWPORT_Y: c_int = 6;

// ghostty_render_state_row_cells_get data kinds
pub const ROW_CELLS_DATA_RAW: c_int = 1;
pub const ROW_CELLS_DATA_STYLE: c_int = 2;

// ghostty_cell_get data kinds
pub const CELL_DATA_CODEPOINT: c_int = 1;
pub const CELL_DATA_WIDE: c_int = 3;

// GhosttyCellWide
pub const CELL_WIDE_NARROW: c_int = 0;
pub const CELL_WIDE_WIDE: c_int = 1;
pub const CELL_WIDE_SPACER_TAIL: c_int = 2;
pub const CELL_WIDE_SPACER_HEAD: c_int = 3;

// GhosttyCursorStyle
pub const CURSOR_STYLE_BAR: c_int = 0;
pub const CURSOR_STYLE_BLOCK: c_int = 1;
pub const CURSOR_STYLE_UNDERLINE: c_int = 2;
pub const CURSOR_STYLE_BLOCK_HOLLOW: c_int = 3;

// GhosttyStyleColorTag
pub const STYLE_COLOR_NONE: u8 = 0;
pub const STYLE_COLOR_PALETTE: u8 = 1;
pub const STYLE_COLOR_RGB: u8 = 2;

// GhosttyStyleFlags
pub const STYLE_FLAG_BOLD: u16 = 1 << 0;
pub const STYLE_FLAG_ITALIC: u16 = 1 << 1;
pub const STYLE_FLAG_FAINT: u16 = 1 << 2;
pub const STYLE_FLAG_INVERSE: u16 = 1 << 4;
pub const STYLE_FLAG_INVISIBLE: u16 = 1 << 5;
pub const STYLE_FLAG_STRIKETHROUGH: u16 = 1 << 6;
pub const STYLE_UNDERLINE_MASK: u16 = 0x7 << 8;

// GhosttyTerminalScrollViewportTag
pub const SCROLL_VIEWPORT_DELTA: c_int = 2;
pub const SCROLL_VIEWPORT_ROW: c_int = 3;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct GhosttyColorRgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// Four bytes: a one-byte tag plus a three-byte union (palette index or RGB).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GhosttyStyleColor {
    pub tag: u8,
    pub value: [u8; 3],
}

impl GhosttyStyleColor {
    pub fn palette_index(self) -> Option<u8> {
        (self.tag == STYLE_COLOR_PALETTE).then_some(self.value[0])
    }

    pub fn rgb(self) -> Option<GhosttyColorRgb> {
        (self.tag == STYLE_COLOR_RGB).then_some(GhosttyColorRgb {
            r: self.value[0],
            g: self.value[1],
            b: self.value[2],
        })
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GhosttyStyle {
    pub fg_color: GhosttyStyleColor,
    pub bg_color: GhosttyStyleColor,
    pub underline_color: GhosttyStyleColor,
    pub flags: u16,
    pub reserved: u16,
}

const _: () = assert!(size_of::<GhosttyStyle>() == 16);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GhosttyTerminalScrollbar {
    pub total: u64,
    pub offset: u64,
    pub len: u64,
}

/// The scroll union is two `uint64_t` of padding; `delta` and `row` both fit
/// in the first word, so the wrapper stores the raw word and casts per tag.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GhosttyTerminalScrollViewport {
    pub tag: c_int,
    pub value: [u64; 2],
}

#[repr(C)]
pub struct GhosttyRenderStateCursor {
    pub size: usize,
    pub viewport_has_value: bool,
    pub viewport_x: u16,
    pub viewport_y: u16,
    pub wide_tail: bool,
    pub visible: bool,
    pub blinking: bool,
    pub password_input: bool,
    pub visual_style: c_int,
}

pub type GhosttyTerminalWritePtyFn =
    unsafe extern "C" fn(GhosttyTerminal, *mut c_void, *const u8, usize);

unsafe extern "C" {
    pub fn ghostty_terminal_new(
        allocator: *const c_void,
        terminal: *mut GhosttyTerminal,
        cols: u16,
        rows: u16,
    ) -> GhosttyResult;
    pub fn ghostty_terminal_free(terminal: GhosttyTerminal);
    pub fn ghostty_terminal_resize(
        terminal: GhosttyTerminal,
        cols: u16,
        rows: u16,
        cell_width_px: u32,
        cell_height_px: u32,
    ) -> GhosttyResult;
    pub fn ghostty_terminal_set(
        terminal: GhosttyTerminal,
        option: c_int,
        value: *const c_void,
    ) -> GhosttyResult;
    pub fn ghostty_terminal_get(
        terminal: GhosttyTerminal,
        data: c_int,
        out: *mut c_void,
    ) -> GhosttyResult;
    pub fn ghostty_terminal_vt_write(terminal: GhosttyTerminal, ptr: *const u8, len: usize);
    pub fn ghostty_terminal_scroll_viewport(
        terminal: GhosttyTerminal,
        behavior: GhosttyTerminalScrollViewport,
    ) -> GhosttyResult;

    pub fn ghostty_render_state_new(
        allocator: *const c_void,
        state: *mut GhosttyRenderState,
    ) -> GhosttyResult;
    pub fn ghostty_render_state_free(state: GhosttyRenderState);
    pub fn ghostty_render_state_update(
        state: GhosttyRenderState,
        terminal: GhosttyTerminal,
    ) -> GhosttyResult;
    pub fn ghostty_render_state_get(
        state: GhosttyRenderState,
        data: c_int,
        out: *mut c_void,
    ) -> GhosttyResult;

    pub fn ghostty_render_state_row_iterator_new(
        allocator: *const c_void,
        out: *mut GhosttyRenderStateRowIterator,
    ) -> GhosttyResult;
    pub fn ghostty_render_state_row_iterator_free(iterator: GhosttyRenderStateRowIterator);
    pub fn ghostty_render_state_row_iterator_next(iterator: GhosttyRenderStateRowIterator) -> bool;
    pub fn ghostty_render_state_row_get(
        iterator: GhosttyRenderStateRowIterator,
        data: c_int,
        out: *mut c_void,
    ) -> GhosttyResult;

    pub fn ghostty_render_state_row_cells_new(
        allocator: *const c_void,
        out: *mut GhosttyRenderStateRowCells,
    ) -> GhosttyResult;
    pub fn ghostty_render_state_row_cells_free(cells: GhosttyRenderStateRowCells);
    pub fn ghostty_render_state_row_cells_next(cells: GhosttyRenderStateRowCells) -> bool;
    pub fn ghostty_render_state_row_cells_get(
        cells: GhosttyRenderStateRowCells,
        data: c_int,
        out: *mut c_void,
    ) -> GhosttyResult;

    pub fn ghostty_cell_get(cell: GhosttyCell, data: c_int, out: *mut c_void) -> GhosttyResult;
}

/// One libghostty-vt terminal plus the render-state handles used to read it.
///
/// Every method takes `&mut self` because the C library mutates internal
/// caches even on reads; callers serialize access with their own lock.
/// `U` is the write-pty callback context; it is kept alive until after the
/// terminal handle is freed, which is what stops the callbacks.
pub struct Vt<U> {
    terminal: GhosttyTerminal,
    render: GhosttyRenderState,
    rows: GhosttyRenderStateRowIterator,
    cells: GhosttyRenderStateRowCells,
    _userdata: Box<U>,
}

// The handles never escape the lock that guards the session: the reader
// thread and the Qt thread both go through the same mutex before touching
// the Vt, so only one thread is inside the C library at a time.
unsafe impl<U> Send for Vt<U> {}

impl<U> Vt<U> {
    /// Creates a terminal whose escape-sequence replies (cursor position,
    /// device attributes, size reports) are written back through `write_pty`.
    /// The callback runs inside [`Self::write`] on the calling thread and
    /// receives a pointer to the boxed `userdata`.
    pub fn new(
        cols: u16,
        rows: u16,
        scrollback_lines: usize,
        write_pty: GhosttyTerminalWritePtyFn,
        mut userdata: Box<U>,
    ) -> Result<Self, String> {
        let mut terminal: GhosttyTerminal = std::ptr::null_mut();
        let mut render: GhosttyRenderState = std::ptr::null_mut();
        let mut row_iter: GhosttyRenderStateRowIterator = std::ptr::null_mut();
        let mut row_cells: GhosttyRenderStateRowCells = std::ptr::null_mut();
        let userdata_ptr = &mut *userdata as *mut U as *mut c_void;
        unsafe {
            if ghostty_terminal_new(std::ptr::null(), &mut terminal, cols, rows) != GHOSTTY_SUCCESS
            {
                return Err("term.vt_create_failed".to_string());
            }
            let ok = ghostty_render_state_new(std::ptr::null(), &mut render) == GHOSTTY_SUCCESS
                && ghostty_render_state_row_iterator_new(std::ptr::null(), &mut row_iter)
                    == GHOSTTY_SUCCESS
                && ghostty_render_state_row_cells_new(std::ptr::null(), &mut row_cells)
                    == GHOSTTY_SUCCESS;
            let vt = Self {
                terminal,
                render,
                rows: row_iter,
                cells: row_cells,
                _userdata: userdata,
            };
            if !ok {
                return Err("term.vt_render_state_failed".to_string());
            }
            ghostty_terminal_set(vt.terminal, TERMINAL_OPT_USERDATA, userdata_ptr);
            // The C API takes the function pointer *value* in the `const void*`
            // slot — the same thing `(const void*)on_write_pty` means in C.
            ghostty_terminal_set(
                vt.terminal,
                TERMINAL_OPT_WRITE_PTY,
                write_pty as *const c_void,
            );
            // The byte limit defaults to 10 KB, which would prune history
            // long before the line limit; let the line limit govern.
            let unlimited_bytes = usize::MAX;
            ghostty_terminal_set(
                vt.terminal,
                TERMINAL_OPT_SCROLLBACK_MAX_BYTES,
                &unlimited_bytes as *const _ as *const c_void,
            );
            ghostty_terminal_set(
                vt.terminal,
                TERMINAL_OPT_SCROLLBACK_MAX_LINES,
                &scrollback_lines as *const _ as *const c_void,
            );
            Ok(vt)
        }
    }

    pub fn write(&mut self, bytes: &[u8]) {
        unsafe { ghostty_terminal_vt_write(self.terminal, bytes.as_ptr(), bytes.len()) }
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        unsafe {
            ghostty_terminal_resize(self.terminal, cols, rows, 0, 0);
        }
    }

    /// Scrolls by a delta; positive moves down, negative up.
    pub fn scroll_delta(&mut self, delta: isize) {
        let behavior = GhosttyTerminalScrollViewport {
            tag: SCROLL_VIEWPORT_DELTA,
            value: [delta as u64, 0],
        };
        unsafe {
            ghostty_terminal_scroll_viewport(self.terminal, behavior);
        }
    }

    /// Scrolls so `row` (counted from the top of the scrollback) becomes the
    /// first visible row.
    pub fn scroll_to_row(&mut self, row: usize) {
        let behavior = GhosttyTerminalScrollViewport {
            tag: SCROLL_VIEWPORT_ROW,
            value: [row as u64, 0],
        };
        unsafe {
            ghostty_terminal_scroll_viewport(self.terminal, behavior);
        }
    }

    /// `(total, offset, len)` of the scrollable area, all in rows.
    pub fn scrollbar(&mut self) -> Option<(u64, u64, u64)> {
        let mut scrollbar = GhosttyTerminalScrollbar {
            total: 0,
            offset: 0,
            len: 0,
        };
        let result = unsafe {
            ghostty_terminal_get(
                self.terminal,
                TERMINAL_DATA_SCROLLBAR,
                &mut scrollbar as *mut _ as *mut c_void,
            )
        };
        (result == GHOSTTY_SUCCESS).then_some((scrollbar.total, scrollbar.offset, scrollbar.len))
    }

    /// Refreshes the render state from the terminal. Must run before
    /// [`Self::visit_rows`] or [`Self::cursor`].
    pub fn update(&mut self) -> bool {
        unsafe { ghostty_render_state_update(self.render, self.terminal) == GHOSTTY_SUCCESS }
    }

    /// Visits every viewport row top to bottom. For each row, `visit_row`
    /// receives the viewport y and a reader positioned before the first
    /// cell; call [`Row::next_cell`] until it returns `None`.
    pub fn visit_rows(&mut self, mut visit_row: impl FnMut(i32, Row<'_>)) {
        unsafe {
            if ghostty_render_state_get(
                self.render,
                RENDER_STATE_DATA_ROW_ITERATOR,
                &mut self.rows as *mut _ as *mut c_void,
            ) != GHOSTTY_SUCCESS
            {
                return;
            }
            while ghostty_render_state_row_iterator_next(self.rows) {
                let mut y: i32 = 0;
                if ghostty_render_state_row_get(
                    self.rows,
                    ROW_DATA_VIEWPORT_Y,
                    &mut y as *mut _ as *mut c_void,
                ) != GHOSTTY_SUCCESS
                {
                    continue;
                }
                if ghostty_render_state_row_get(
                    self.rows,
                    ROW_DATA_CELLS,
                    &mut self.cells as *mut _ as *mut c_void,
                ) != GHOSTTY_SUCCESS
                {
                    continue;
                }
                visit_row(
                    y,
                    Row {
                        cells: self.cells,
                        column: 0,
                        _marker: std::marker::PhantomData,
                    },
                );
            }
        }
    }

    /// The cursor as the viewport sees it: `(x, y, shape)` when visible,
    /// where shape is 0 block, 1 underline, 2 beam, 3 hollow block.
    pub fn cursor(&mut self) -> Option<(u16, u16, u8)> {
        let mut cursor = GhosttyRenderStateCursor {
            size: size_of::<GhosttyRenderStateCursor>(),
            viewport_has_value: false,
            viewport_x: 0,
            viewport_y: 0,
            wide_tail: false,
            visible: false,
            blinking: false,
            password_input: false,
            visual_style: CURSOR_STYLE_BLOCK,
        };
        let result = unsafe {
            ghostty_render_state_get(
                self.render,
                RENDER_STATE_DATA_CURSOR,
                &mut cursor as *mut _ as *mut c_void,
            )
        };
        if result != GHOSTTY_SUCCESS || !cursor.visible || !cursor.viewport_has_value {
            return None;
        }
        let shape = match cursor.visual_style {
            CURSOR_STYLE_BAR => 2,
            CURSOR_STYLE_UNDERLINE => 1,
            CURSOR_STYLE_BLOCK_HOLLOW => 3,
            _ => 0,
        };
        Some((cursor.viewport_x, cursor.viewport_y, shape))
    }
}

impl<U> Drop for Vt<U> {
    fn drop(&mut self) {
        unsafe {
            ghostty_render_state_row_cells_free(self.cells);
            ghostty_render_state_row_iterator_free(self.rows);
            ghostty_render_state_free(self.render);
            // Freeing the terminal stops any write_pty callback; the
            // userdata Box is dropped with the struct, after this call.
            ghostty_terminal_free(self.terminal);
        }
    }
}

/// A reader over one row's cells, borrowed from [`Vt::visit_rows`].
pub struct Row<'a> {
    cells: GhosttyRenderStateRowCells,
    column: u16,
    _marker: std::marker::PhantomData<&'a mut ()>,
}

impl Row<'_> {
    /// Advances to the next cell, returning its column, packed value and
    /// style, or `None` past the last column.
    pub fn next_cell(&mut self) -> Option<(u16, GhosttyCell, GhosttyStyle)> {
        unsafe {
            if !ghostty_render_state_row_cells_next(self.cells) {
                return None;
            }
            let mut raw: GhosttyCell = 0;
            let mut style = GhosttyStyle {
                fg_color: GhosttyStyleColor {
                    tag: STYLE_COLOR_NONE,
                    value: [0; 3],
                },
                bg_color: GhosttyStyleColor {
                    tag: STYLE_COLOR_NONE,
                    value: [0; 3],
                },
                underline_color: GhosttyStyleColor {
                    tag: STYLE_COLOR_NONE,
                    value: [0; 3],
                },
                flags: 0,
                reserved: 0,
            };
            if ghostty_render_state_row_cells_get(
                self.cells,
                ROW_CELLS_DATA_RAW,
                &mut raw as *mut _ as *mut c_void,
            ) != GHOSTTY_SUCCESS
                || ghostty_render_state_row_cells_get(
                    self.cells,
                    ROW_CELLS_DATA_STYLE,
                    &mut style as *mut _ as *mut c_void,
                ) != GHOSTTY_SUCCESS
            {
                return None;
            }
            let column = self.column;
            self.column += 1;
            Some((column, raw, style))
        }
    }
}

/// Reads a cell's base codepoint; 0 when the cell carries no text.
pub fn cell_codepoint(cell: GhosttyCell) -> u32 {
    let mut codepoint: u32 = 0;
    unsafe {
        ghostty_cell_get(
            cell,
            CELL_DATA_CODEPOINT,
            &mut codepoint as *mut _ as *mut c_void,
        );
    }
    codepoint
}

/// Reads a cell's width behavior: one of the `CELL_WIDE_*` values.
pub fn cell_wide(cell: GhosttyCell) -> c_int {
    let mut wide: c_int = CELL_WIDE_NARROW;
    unsafe {
        ghostty_cell_get(cell, CELL_DATA_WIDE, &mut wide as *mut _ as *mut c_void);
    }
    wide
}
