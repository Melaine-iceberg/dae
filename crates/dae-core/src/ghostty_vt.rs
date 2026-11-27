//! Hand-rolled bindings to the vendored libghostty-vt C API.
//!
//! Only the surface `pty.rs` needs is declared here; the authoritative
//! headers sit next to the binary in `ghostty-vt/include`. The values
//! and layouts below track the 1.0.x ABI pinned in `ghostty-vt/VERSION`; a
//! bump past a minor release means re-checking them against the new headers.
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
/// Operation failed because the provided buffer was too small.
pub const GHOSTTY_OUT_OF_SPACE: GhosttyResult = -3;
/// "The requested value has no value" — no search matches, or a grid
/// reference outside the requested coordinate system.
pub const GHOSTTY_NO_VALUE: GhosttyResult = -4;
pub type GhosttyTerminal = *mut c_void;
pub type GhosttyRenderState = *mut c_void;
pub type GhosttyRenderStateRowIterator = *mut c_void;
pub type GhosttyRenderStateRowCells = *mut c_void;
pub type GhosttyCell = u64;

// ghostty_terminal_set options
pub const TERMINAL_OPT_USERDATA: c_int = 1;
pub const TERMINAL_OPT_WRITE_PTY: c_int = 2;
/// `GhosttyTerminalTitleChangedFn` — read the new title inside the callback.
pub const TERMINAL_OPT_TITLE_CHANGED: c_int = 6;
/// `GhosttyTerminalPwdChangedFn` — read the new pwd inside the callback.
pub const TERMINAL_OPT_PWD_CHANGED: c_int = 26;
pub const TERMINAL_OPT_SCROLLBACK_MAX_BYTES: c_int = 28;
pub const TERMINAL_OPT_SCROLLBACK_MAX_LINES: c_int = 29;
/// Disable pulling scrollback rows back on resize when the host PTY (such as
/// Windows ConPTY) keeps its own screen buffer without scrollback.
pub const TERMINAL_OPT_RESIZE_PULL_SCROLLBACK: c_int = 41;
/// `GhosttyTerminalRenderHoldFn` — synchronized output (mode 2026) hold/release.
pub const TERMINAL_OPT_RENDER_HOLD: c_int = 42;

// ghostty_terminal_get data kinds
pub const TERMINAL_DATA_SCROLLBAR: c_int = 9;
/// The active screen (`GhosttyTerminalScreen`): 0 primary, 1 alternate.
pub const TERMINAL_DATA_ACTIVE_SCREEN: c_int = 6;
/// Whether any mouse tracking mode (X10, normal, button, any-event) is active.
pub const TERMINAL_DATA_MOUSE_TRACKING: c_int = 11;
/// The value of one mode (`GhosttyTerminalModeConfig`): initialize the
/// `mode` word, receive the boolean.
pub const TERMINAL_DATA_MODE: c_int = 37;

/// A DEC private mode number, as `ghostty_mode_new(value, false)` packs
/// it: the bare number.
pub const MODE_FOCUS_EVENT: u16 = 1004;
pub const MODE_BRACKETED_PASTE: u16 = 2004;
pub const MODE_ALT_SCREEN: u16 = 1047;

/// The window title the shell last set with OSC 0/2 (`GhosttyString`).
pub const TERMINAL_DATA_TITLE: c_int = 12;
/// The working directory the shell last set with OSC 7/9/1337
/// (`GhosttyString`). Empty when the shell never reported one.
pub const TERMINAL_DATA_PWD: c_int = 13;

// ghostty_search_set/get kinds and options
pub const SEARCH_OPT_NEEDLE: c_int = 1;
pub const SEARCH_DATA_TOTAL_MATCHES: c_int = 3;
pub const SEARCH_DATA_SELECTED_INDEX: c_int = 4;
pub const SEARCH_DATA_VIEWPORT_MATCHES: c_int = 7;

// ghostty_terminal_point_from_grid_ref tags
pub const POINT_TAG_VIEWPORT: c_int = 1;

pub const RENDER_STATE_DATA_COLS: c_int = 1;
pub const RENDER_STATE_DATA_ROWS: c_int = 2;
/// `GhosttyRenderStateDirty`: 0 clean, 1 partial (some rows), 2 full.
pub const RENDER_STATE_DATA_DIRTY: c_int = 3;
pub const RENDER_STATE_DATA_ROW_ITERATOR: c_int = 4;
pub const RENDER_STATE_DATA_CURSOR: c_int = 18;

pub const RENDER_STATE_DIRTY_FALSE: u8 = 0;
pub const RENDER_STATE_DIRTY_PARTIAL: u8 = 1;
pub const RENDER_STATE_DIRTY_FULL: u8 = 2;
// ghostty_render_state_row_get data kinds
pub const ROW_DATA_CELLS: c_int = 3;
pub const ROW_DATA_VIEWPORT_Y: c_int = 6;
pub const ROW_DATA_DIRTY: c_int = 1;
pub const ROW_DATA_ID: c_int = 7;
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

/// `GhosttyRenderStateRowId`: two opaque words; two ids are the same row
/// contents when both words are equal; a zeroed id is never valid.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct GhosttyRowId {
    pub bits: [u64; 2],
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

/// A borrowed byte string: pointer valid until the next mutating call.
#[repr(C)]
pub struct GhosttyString {
    pub ptr: *const u8,
    pub len: usize,
}

/// `GhosttyTerminalTitleChangedFn` / `GhosttyTerminalPwdChangedFn`:
/// called on the thread that runs `ghostty_terminal_vt_write`, with the
/// `TERMINAL_OPT_USERDATA` pointer. Read the new value inside the callback
/// via [`terminal_string`].
pub type GhosttyTerminalStringChangedFn = unsafe extern "C" fn(GhosttyTerminal, *mut c_void);

/// `GhosttyTerminalRenderHoldFn`: called with `held = true` when mode 2026
/// (synchronized output) begins, and `held = false` when it ends.
pub type GhosttyTerminalRenderHoldFn = unsafe extern "C" fn(GhosttyTerminal, *mut c_void, bool);

/// Reads a `GhosttyString` data kind (title, pwd) into an owned String.
/// The library borrows the bytes, so this must run before the terminal is
/// mutated again — inside a change callback or under the caller's lock.
///
/// # Safety
/// `terminal` must be a live terminal handle.
pub unsafe fn terminal_string(terminal: GhosttyTerminal, data: c_int) -> Option<String> {
    let mut string = GhosttyString { ptr: std::ptr::null(), len: 0 };
    if unsafe { ghostty_terminal_get(terminal, data, &mut string as *mut _ as *mut c_void) }
        != GHOSTTY_SUCCESS
        || string.ptr.is_null()
    {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(string.ptr, string.len) };
    Some(String::from_utf8_lossy(bytes).into_owned())
}

/// A cell in the terminal's internal storage. The fields are opaque;
/// convert to screen coordinates with [`Search::viewport_matches`].
/// Frozen ABI: 16 bytes on 64-bit.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GhosttyGridRef {
    pub node: *mut c_void,
    pub x: u16,
    pub y: u16,
}

/// A selection between two cells, inclusive. Frozen ABI: 40 bytes on
/// 64-bit.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GhosttySelection {
    pub start: GhosttyGridRef,
    pub end: GhosttyGridRef,
    pub rectangle: bool,
}

/// A caller-provided buffer of selections: NULL/0 queries the required
/// capacity; `len` receives the count written or the capacity needed.
#[repr(C)]
pub struct GhosttySelectionBuffer {
    pub ptr: *mut GhosttySelection,
    pub cap: usize,
    pub len: usize,
}

/// A coordinate in the terminal grid (viewport or history).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct GhosttyPointCoordinate {
    pub x: u16,
    pub y: u32,
}

/// A mode and its boolean, for `GHOSTTY_TERMINAL_DATA_MODE`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GhosttyTerminalModeConfig {
    pub mode: u16,
    pub value: bool,
}

pub type GhosttySearch = *mut c_void;
pub type GhosttyKeyEncoder = *mut c_void;
pub type GhosttyKeyEvent = *mut c_void;
pub type GhosttyMouseEncoder = *mut c_void;
pub type GhosttyMouseEvent = *mut c_void;

// GhosttyFocusEvent
pub const FOCUS_GAINED: c_int = 0;
pub const FOCUS_LOST: c_int = 1;

// GhosttyKeyAction
pub const KEY_ACTION_RELEASE: c_int = 0;
pub const KEY_ACTION_PRESS: c_int = 1;
pub const KEY_ACTION_REPEAT: c_int = 2;

// GhosttyMods
pub const MODS_SHIFT: u16 = 1 << 0;
pub const MODS_CTRL: u16 = 1 << 1;
pub const MODS_ALT: u16 = 1 << 2;
pub const MODS_SUPER: u16 = 1 << 3;
pub const MODS_CAPS_LOCK: u16 = 1 << 4;
pub const MODS_NUM_LOCK: u16 = 1 << 5;

// GhosttyKey values used directly in Rust (remaining keys are mapped in dae-qt).
pub const KEY_UNIDENTIFIED: c_int = 0;
pub const KEY_ARROW_DOWN: c_int = 75;
pub const KEY_ARROW_UP: c_int = 78;

// GhosttyMouseAction
pub const MOUSE_ACTION_PRESS: c_int = 0;
pub const MOUSE_ACTION_RELEASE: c_int = 1;
pub const MOUSE_ACTION_MOTION: c_int = 2;

// GhosttyMouseButton
pub const MOUSE_BUTTON_UNKNOWN: c_int = 0;
pub const MOUSE_BUTTON_LEFT: c_int = 1;
pub const MOUSE_BUTTON_RIGHT: c_int = 2;
pub const MOUSE_BUTTON_MIDDLE: c_int = 3;
pub const MOUSE_BUTTON_FOUR: c_int = 4;
pub const MOUSE_BUTTON_FIVE: c_int = 5;

// GhosttyMouseEncoderOption
pub const MOUSE_ENCODER_OPT_SIZE: c_int = 3;
pub const MOUSE_ENCODER_OPT_ANY_BUTTON_PRESSED: c_int = 4;
pub const MOUSE_ENCODER_OPT_TRACK_LAST_CELL: c_int = 5;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GhosttyMousePosition {
    pub x: f32,
    pub y: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GhosttyMouseEncoderSize {
    pub size: usize,
    pub screen_width: u32,
    pub screen_height: u32,
    pub cell_width: u32,
    pub cell_height: u32,
    pub padding_top: u32,
    pub padding_bottom: u32,
    pub padding_right: u32,
    pub padding_left: u32,
}

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
    pub fn ghostty_render_state_begin_update(
        state: GhosttyRenderState,
        terminal: GhosttyTerminal,
    ) -> GhosttyResult;
    pub fn ghostty_render_state_end_update(state: GhosttyRenderState) -> GhosttyResult;
    pub fn ghostty_render_state_clean(state: GhosttyRenderState) -> GhosttyResult;
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
    pub fn ghostty_render_state_row_iterator_next_dirty(
        iterator: GhosttyRenderStateRowIterator,
        out_y: *mut u16,
    ) -> bool;
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

    pub fn ghostty_search_new(
        allocator: *const c_void,
        out: *mut GhosttySearch,
        terminal: GhosttyTerminal,
    ) -> GhosttyResult;
    pub fn ghostty_paste_encode(
        data: *mut u8,
        data_len: usize,
        bracketed: bool,
        buffer: *mut u8,
        buffer_len: usize,
        out_written: *mut usize,
    ) -> GhosttyResult;
    pub fn ghostty_focus_encode(
        event: c_int,
        buf: *mut u8,
        buf_len: usize,
        out_written: *mut usize,
    ) -> GhosttyResult;
    pub fn ghostty_key_encoder_new(
        allocator: *const c_void,
        encoder: *mut GhosttyKeyEncoder,
    ) -> GhosttyResult;
    pub fn ghostty_key_encoder_free(encoder: GhosttyKeyEncoder);
    pub fn ghostty_key_encoder_set_from_terminal(
        encoder: GhosttyKeyEncoder,
        terminal: GhosttyTerminal,
    ) -> GhosttyResult;
    pub fn ghostty_key_encoder_encode(
        encoder: GhosttyKeyEncoder,
        event: GhosttyKeyEvent,
        buf: *mut u8,
        buf_len: usize,
        out_written: *mut usize,
    ) -> GhosttyResult;
    pub fn ghostty_key_event_new(
        allocator: *const c_void,
        event: *mut GhosttyKeyEvent,
    ) -> GhosttyResult;
    pub fn ghostty_key_event_free(event: GhosttyKeyEvent);
    pub fn ghostty_key_event_set_action(event: GhosttyKeyEvent, action: c_int) -> GhosttyResult;
    pub fn ghostty_key_event_set_key(event: GhosttyKeyEvent, key: c_int) -> GhosttyResult;
    pub fn ghostty_key_event_set_mods(event: GhosttyKeyEvent, mods: u16) -> GhosttyResult;
    pub fn ghostty_key_event_set_consumed_mods(
        event: GhosttyKeyEvent,
        consumed_mods: u16,
    ) -> GhosttyResult;
    pub fn ghostty_key_event_set_composing(
        event: GhosttyKeyEvent,
        composing: bool,
    ) -> GhosttyResult;
    pub fn ghostty_key_event_set_utf8(
        event: GhosttyKeyEvent,
        utf8: GhosttyString,
    ) -> GhosttyResult;
    pub fn ghostty_key_event_set_unshifted_codepoint(
        event: GhosttyKeyEvent,
        codepoint: u32,
    ) -> GhosttyResult;
    pub fn ghostty_mouse_encoder_new(
        allocator: *const c_void,
        encoder: *mut GhosttyMouseEncoder,
    ) -> GhosttyResult;
    pub fn ghostty_mouse_encoder_free(encoder: GhosttyMouseEncoder);
    pub fn ghostty_mouse_encoder_set(
        encoder: GhosttyMouseEncoder,
        option: c_int,
        value: *const c_void,
    ) -> GhosttyResult;
    pub fn ghostty_mouse_encoder_set_from_terminal(
        encoder: GhosttyMouseEncoder,
        terminal: GhosttyTerminal,
    ) -> GhosttyResult;
    pub fn ghostty_mouse_encoder_encode(
        encoder: GhosttyMouseEncoder,
        event: GhosttyMouseEvent,
        buf: *mut u8,
        buf_len: usize,
        out_written: *mut usize,
    ) -> GhosttyResult;
    pub fn ghostty_mouse_event_new(
        allocator: *const c_void,
        event: *mut GhosttyMouseEvent,
    ) -> GhosttyResult;
    pub fn ghostty_mouse_event_free(event: GhosttyMouseEvent);
    pub fn ghostty_mouse_event_set_action(
        event: GhosttyMouseEvent,
        action: c_int,
    ) -> GhosttyResult;
    pub fn ghostty_mouse_event_set_button(
        event: GhosttyMouseEvent,
        button: c_int,
    ) -> GhosttyResult;
    pub fn ghostty_mouse_event_clear_button(event: GhosttyMouseEvent) -> GhosttyResult;
    pub fn ghostty_mouse_event_set_mods(event: GhosttyMouseEvent, mods: u16) -> GhosttyResult;
    pub fn ghostty_mouse_event_set_position(
        event: GhosttyMouseEvent,
        position: GhosttyMousePosition,
    ) -> GhosttyResult;

    pub fn ghostty_search_free(search: GhosttySearch);
    pub fn ghostty_search_set(
        search: GhosttySearch,
        option: c_int,
        value: *const c_void,
    ) -> GhosttyResult;
    pub fn ghostty_search_get(
        search: GhosttySearch,
        data: c_int,
        out: *mut c_void,
    ) -> GhosttyResult;
    pub fn ghostty_search_run(search: GhosttySearch) -> GhosttyResult;
    pub fn ghostty_search_select_next(search: GhosttySearch) -> GhosttyResult;
    pub fn ghostty_search_select_prev(search: GhosttySearch) -> GhosttyResult;

    pub fn ghostty_terminal_point_from_grid_ref(
        terminal: GhosttyTerminal,
        reference: *const GhosttyGridRef,
        tag: c_int,
        out: *mut GhosttyPointCoordinate,
    ) -> GhosttyResult;
}

/// One libghostty-vt terminal plus its reusable input encoders.
///
/// Every method takes `&mut self` because the C library mutates internal
/// caches even on reads; callers serialize access with their own lock.
/// `U` is the write-pty/effect callback context; it is kept alive until after
/// the terminal handle is freed, which is what stops the callbacks.
pub struct Vt<U> {
    terminal: GhosttyTerminal,
    key_encoder: GhosttyKeyEncoder,
    key_event: GhosttyKeyEvent,
    mouse_encoder: GhosttyMouseEncoder,
    mouse_event: GhosttyMouseEvent,
    _userdata: Box<U>,
}

// The handles never escape the lock that guards the session: the reader
// thread and the Qt thread both go through the same mutex before touching
// the Vt, so only one thread is inside the C library at a time.
unsafe impl<U> Send for Vt<U> {}

/// Decoupled render state for lock-minimized two-phase updates.
///
/// Lock the terminal only for [`Self::begin_update`], then release the
/// terminal lock and run [`Self::end_update`], row reads, and [`Self::clean`].
pub struct RenderState {
    render: GhosttyRenderState,
    rows: GhosttyRenderStateRowIterator,
    cells: GhosttyRenderStateRowCells,
}

unsafe impl Send for RenderState {}

impl RenderState {
    pub fn new() -> Result<Self, String> {
        let mut render: GhosttyRenderState = std::ptr::null_mut();
        let mut row_iter: GhosttyRenderStateRowIterator = std::ptr::null_mut();
        let mut row_cells: GhosttyRenderStateRowCells = std::ptr::null_mut();
        unsafe {
            let ok = ghostty_render_state_new(std::ptr::null(), &mut render) == GHOSTTY_SUCCESS
                && ghostty_render_state_row_iterator_new(std::ptr::null(), &mut row_iter)
                    == GHOSTTY_SUCCESS
                && ghostty_render_state_row_cells_new(std::ptr::null(), &mut row_cells)
                    == GHOSTTY_SUCCESS;
            let state = Self {
                render,
                rows: row_iter,
                cells: row_cells,
            };
            if !ok {
                return Err("term.vt_render_state_failed".to_string());
            }
            Ok(state)
        }
    }

    /// Captures dirty terminal state into this render state. Must be paired
    /// with [`Self::end_update`] after releasing the terminal lock.
    pub fn begin_update<U>(&mut self, vt: &Vt<U>) -> bool {
        unsafe { ghostty_render_state_begin_update(self.render, vt.terminal) == GHOSTTY_SUCCESS }
    }

    /// Completes deferred work on render-state-owned memory without touching
    /// the terminal.
    pub fn end_update(&mut self) -> bool {
        unsafe { ghostty_render_state_end_update(self.render) == GHOSTTY_SUCCESS }
    }

    /// Marks all global and per-row dirty flags as consumed so the next update
    /// can report `DIRTY_FALSE` or `DIRTY_PARTIAL`.
    pub fn clean(&mut self) {
        unsafe {
            ghostty_render_state_clean(self.render);
        }
    }

    /// The `(cols, rows)` captured by the last update.
    pub fn dimensions(&self) -> (usize, usize) {
        let mut cols: u16 = 0;
        let mut rows: u16 = 0;
        unsafe {
            ghostty_render_state_get(
                self.render,
                RENDER_STATE_DATA_COLS,
                &mut cols as *mut _ as *mut c_void,
            );
            ghostty_render_state_get(
                self.render,
                RENDER_STATE_DATA_ROWS,
                &mut rows as *mut _ as *mut c_void,
            );
        }
        (usize::from(cols), usize::from(rows))
    }

    /// Visits every viewport row top to bottom.
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
                        row: self.rows,
                        cells: self.cells,
                        column: 0,
                        _marker: std::marker::PhantomData,
                    },
                );
            }
        }
    }

    /// Visits only the rows requiring a redraw according to the dirty state.
    pub fn visit_dirty_rows(&mut self, mut visit_row: impl FnMut(u16, Row<'_>)) {
        unsafe {
            if ghostty_render_state_get(
                self.render,
                RENDER_STATE_DATA_ROW_ITERATOR,
                &mut self.rows as *mut _ as *mut c_void,
            ) != GHOSTTY_SUCCESS
            {
                return;
            }
            let mut y: u16 = 0;
            while ghostty_render_state_row_iterator_next_dirty(self.rows, &mut y) {
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
                        row: self.rows,
                        cells: self.cells,
                        column: 0,
                        _marker: std::marker::PhantomData,
                    },
                );
            }
        }
    }

    /// The dirty level of the last update: 0 nothing changed,
    /// 1 some rows changed, 2 redraw all.
    pub fn dirty(&self) -> u8 {
        let mut dirty: c_int = 0;
        unsafe {
            ghostty_render_state_get(
                self.render,
                RENDER_STATE_DATA_DIRTY,
                &mut dirty as *mut _ as *mut c_void,
            );
        }
        dirty.clamp(0, 2) as u8
    }

    /// The cursor as the viewport sees it: `(x, y, shape)` when visible,
    /// where shape is 0 block, 1 underline, 2 beam, 3 hollow block.
    pub fn cursor(&self) -> Option<(u16, u16, u8)> {
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

impl Drop for RenderState {
    fn drop(&mut self) {
        unsafe {
            ghostty_render_state_row_cells_free(self.cells);
            ghostty_render_state_row_iterator_free(self.rows);
            ghostty_render_state_free(self.render);
        }
    }
}

impl<U> Vt<U> {
    /// Creates a terminal whose escape-sequence replies (cursor position,
    /// device attributes, size reports) are written back through `write_pty`,
    /// title (OSC 0/2) and working directory (OSC 7/9/1337) changes are
    /// reported through `title_changed` / `pwd_changed`, and synchronized
    /// output holds (mode 2026) are reported through `render_hold`.
    pub fn new(
        cols: u16,
        rows: u16,
        scrollback_lines: usize,
        write_pty: GhosttyTerminalWritePtyFn,
        title_changed: GhosttyTerminalStringChangedFn,
        pwd_changed: GhosttyTerminalStringChangedFn,
        render_hold: GhosttyTerminalRenderHoldFn,
        mut userdata: Box<U>,
    ) -> Result<Self, String> {
        let mut terminal: GhosttyTerminal = std::ptr::null_mut();
        let mut key_encoder: GhosttyKeyEncoder = std::ptr::null_mut();
        let mut key_event: GhosttyKeyEvent = std::ptr::null_mut();
        let mut mouse_encoder: GhosttyMouseEncoder = std::ptr::null_mut();
        let mut mouse_event: GhosttyMouseEvent = std::ptr::null_mut();
        let userdata_ptr = &mut *userdata as *mut U as *mut c_void;
        unsafe {
            if ghostty_terminal_new(std::ptr::null(), &mut terminal, cols, rows) != GHOSTTY_SUCCESS
            {
                return Err("term.vt_create_failed".to_string());
            }
            let ok = ghostty_key_encoder_new(std::ptr::null(), &mut key_encoder)
                == GHOSTTY_SUCCESS
                && ghostty_key_event_new(std::ptr::null(), &mut key_event) == GHOSTTY_SUCCESS
                && ghostty_mouse_encoder_new(std::ptr::null(), &mut mouse_encoder)
                    == GHOSTTY_SUCCESS
                && ghostty_mouse_event_new(std::ptr::null(), &mut mouse_event) == GHOSTTY_SUCCESS;
            let vt = Self {
                terminal,
                key_encoder,
                key_event,
                mouse_encoder,
                mouse_event,
                _userdata: userdata,
            };
            if !ok {
                return Err("term.vt_encoder_failed".to_string());
            }
            ghostty_terminal_set(vt.terminal, TERMINAL_OPT_USERDATA, userdata_ptr);
            // The C API takes the function pointer *value* in the `const void*`
            // slot — the same thing `(const void*)on_write_pty` means in C.
            ghostty_terminal_set(
                vt.terminal,
                TERMINAL_OPT_WRITE_PTY,
                write_pty as *const c_void,
            );
            ghostty_terminal_set(
                vt.terminal,
                TERMINAL_OPT_TITLE_CHANGED,
                title_changed as *const c_void,
            );
            ghostty_terminal_set(
                vt.terminal,
                TERMINAL_OPT_PWD_CHANGED,
                pwd_changed as *const c_void,
            );
            ghostty_terminal_set(
                vt.terminal,
                TERMINAL_OPT_RENDER_HOLD,
                render_hold as *const c_void,
            );
            // Windows ConPTY keeps its own screen buffer without scrollback,
            // so pulling scrollback lines down on resize makes the local grid
            // disagree with ConPTY.
            #[cfg(target_os = "windows")]
            {
                let pull_scrollback = false;
                ghostty_terminal_set(
                    vt.terminal,
                    TERMINAL_OPT_RESIZE_PULL_SCROLLBACK,
                    &pull_scrollback as *const _ as *const c_void,
                );
            }
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
            let track_last_cell = true;
            ghostty_mouse_encoder_set(
                vt.mouse_encoder,
                MOUSE_ENCODER_OPT_TRACK_LAST_CELL,
                &track_last_cell as *const _ as *const c_void,
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

    /// Encodes a key event using the terminal's active modes (cursor/keypad
    /// application, alt-esc prefix, backarrow, modifyOtherKeys, Kitty protocol).
    pub fn encode_key(
        &mut self,
        action: c_int,
        key: c_int,
        mods: u16,
        consumed_mods: u16,
        unshifted_codepoint: u32,
        utf8: &str,
        composing: bool,
    ) -> Vec<u8> {
        unsafe {
            ghostty_key_encoder_set_from_terminal(self.key_encoder, self.terminal);
            ghostty_key_event_set_action(self.key_event, action);
            if ghostty_key_event_set_key(self.key_event, key) != GHOSTTY_SUCCESS {
                ghostty_key_event_set_key(self.key_event, KEY_UNIDENTIFIED);
            }
            ghostty_key_event_set_mods(self.key_event, mods);
            ghostty_key_event_set_consumed_mods(self.key_event, consumed_mods);
            ghostty_key_event_set_composing(self.key_event, composing);
            ghostty_key_event_set_unshifted_codepoint(self.key_event, unshifted_codepoint);
            let text = GhosttyString {
                ptr: if utf8.is_empty() {
                    std::ptr::null()
                } else {
                    utf8.as_ptr()
                },
                len: utf8.len(),
            };
            ghostty_key_event_set_utf8(self.key_event, text);

            let mut buf = [0u8; 128];
            let mut written = 0usize;
            let res = ghostty_key_encoder_encode(
                self.key_encoder,
                self.key_event,
                buf.as_mut_ptr(),
                buf.len(),
                &mut written,
            );
            let out = if res == GHOSTTY_SUCCESS {
                buf[..written].to_vec()
            } else if res == GHOSTTY_OUT_OF_SPACE && written > 0 {
                let mut dyn_buf = vec![0u8; written];
                let mut dyn_written = 0usize;
                if ghostty_key_encoder_encode(
                    self.key_encoder,
                    self.key_event,
                    dyn_buf.as_mut_ptr(),
                    dyn_buf.len(),
                    &mut dyn_written,
                ) == GHOSTTY_SUCCESS
                {
                    dyn_buf.truncate(dyn_written);
                    dyn_buf
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };
            ghostty_key_event_set_utf8(
                self.key_event,
                GhosttyString {
                    ptr: std::ptr::null(),
                    len: 0,
                },
            );
            out
        }
    }

    /// Encodes a mouse event using the terminal's active mouse tracking mode
    /// and format (X10, UTF-8, SGR, URxvt, SGR-Pixels).
    pub fn encode_mouse(
        &mut self,
        action: c_int,
        button: Option<c_int>,
        mods: u16,
        x: f32,
        y: f32,
        size: GhosttyMouseEncoderSize,
        any_button_pressed: bool,
    ) -> Vec<u8> {
        unsafe {
            ghostty_mouse_encoder_set_from_terminal(self.mouse_encoder, self.terminal);
            ghostty_mouse_encoder_set(
                self.mouse_encoder,
                MOUSE_ENCODER_OPT_SIZE,
                &size as *const _ as *const c_void,
            );
            ghostty_mouse_encoder_set(
                self.mouse_encoder,
                MOUSE_ENCODER_OPT_ANY_BUTTON_PRESSED,
                &any_button_pressed as *const _ as *const c_void,
            );
            ghostty_mouse_event_set_action(self.mouse_event, action);
            match button {
                Some(btn) => {
                    ghostty_mouse_event_set_button(self.mouse_event, btn);
                }
                None => {
                    ghostty_mouse_event_clear_button(self.mouse_event);
                }
            }
            ghostty_mouse_event_set_mods(self.mouse_event, mods);
            ghostty_mouse_event_set_position(self.mouse_event, GhosttyMousePosition { x, y });

            let mut buf = [0u8; 128];
            let mut written = 0usize;
            if ghostty_mouse_encoder_encode(
                self.mouse_encoder,
                self.mouse_event,
                buf.as_mut_ptr(),
                buf.len(),
                &mut written,
            ) == GHOSTTY_SUCCESS
            {
                buf[..written].to_vec()
            } else {
                Vec::new()
            }
        }
    }

    /// Encodes focus gained/lost if the running program enabled mode 1004.
    pub fn encode_focus(&mut self, gained: bool) -> Option<Vec<u8>> {
        if !self.mode(MODE_FOCUS_EVENT) {
            return None;
        }
        let event = if gained { FOCUS_GAINED } else { FOCUS_LOST };
        let mut buf = [0u8; 16];
        let mut written = 0usize;
        let res = unsafe { ghostty_focus_encode(event, buf.as_mut_ptr(), buf.len(), &mut written) };
        (res == GHOSTTY_SUCCESS && written > 0).then(|| buf[..written].to_vec())
    }

    /// Whether any mouse tracking mode (1000/1002/1003/9) is active.
    pub fn mouse_tracking(&mut self) -> bool {
        let mut tracking = false;
        unsafe {
            ghostty_terminal_get(
                self.terminal,
                TERMINAL_DATA_MOUSE_TRACKING,
                &mut tracking as *mut _ as *mut c_void,
            );
        }
        tracking
    }

    /// Creates a search bound to this terminal. The handle is kept by the
    /// caller (in the same lock) and freed before the terminal.
    pub fn create_search(&mut self) -> Result<Search, String> {
        let mut search: GhosttySearch = std::ptr::null_mut();
        unsafe {
            if ghostty_search_new(std::ptr::null(), &mut search, self.terminal)
                != GHOSTTY_SUCCESS
            {
                return Err("term.search_create_failed".to_string());
            }
        }
        Ok(Search {
            handle: search,
            terminal: self.terminal,
        })
    }

    /// The current value of a DEC private mode (the `ghostty_mode_new`
    /// word, e.g. [`MODE_BRACKETED_PASTE`]).
    pub fn mode(&mut self, mode: u16) -> bool {
        let mut config = GhosttyTerminalModeConfig { mode, value: false };
        unsafe {
            ghostty_terminal_get(
                self.terminal,
                TERMINAL_DATA_MODE,
                &mut config as *mut _ as *mut c_void,
            );
        }
        config.value
    }

    /// Whether the alternate screen is showing — vim, less, htop. The
    /// wheel turns into arrow keys there instead of scrolling history.
    pub fn alternate_screen(&mut self) -> bool {
        let mut screen: c_int = 0;
        unsafe {
            ghostty_terminal_get(
                self.terminal,
                TERMINAL_DATA_ACTIVE_SCREEN,
                &mut screen as *mut _ as *mut c_void,
            );
        }
        screen == 1
    }
}

/// A scrollback search over one terminal, newest matches first.
///
/// Every method reads the terminal, so callers serialize access with the
/// same lock that guards the [`Vt`]; the handle must be freed before the
/// terminal it is bound to.
pub struct Search {
    handle: GhosttySearch,
    terminal: GhosttyTerminal,
}

// The handle never escapes the lock that guards the session, exactly like
// Vt.
unsafe impl Send for Search {}

/// One highlighted stretch of cells in viewport coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchRect {
    pub row: u16,
    pub column: u16,
    pub columns: u16,
}

impl Search {
    /// Sets what to look for; `None` clears the needle and returns the
    /// search to idle. ASCII letters match case-insensitively.
    pub fn set_needle(&mut self, needle: Option<&str>) {
        let value = needle.map(|text| GhosttyString {
            ptr: text.as_bytes().as_ptr(),
            len: text.len(),
        });
        unsafe {
            ghostty_search_set(
                self.handle,
                SEARCH_OPT_NEEDLE,
                value
                    .as_ref()
                    .map_or(std::ptr::null(), |string| string as *const _ as *const c_void)
                    .cast_mut(),
            );
        }
    }

    /// Brings the search up to date. Blocking; interactive callers accept
    /// this because the alternative is a feed/tick state machine for a
    /// 10 000-line scrollback cap.
    pub fn run(&mut self) -> bool {
        unsafe { ghostty_search_run(self.handle) == GHOSTTY_SUCCESS }
    }

    /// Selects the next (older) match, scrolling it into view, and returns
    /// `(total, selected index)` — the "k of n" the find bar shows.
    /// Wraps after the oldest match.
    pub fn step(&mut self, forward: bool) -> Option<(usize, usize)> {
        let moved = unsafe {
            if forward {
                ghostty_search_select_next(self.handle)
            } else {
                ghostty_search_select_prev(self.handle)
            }
        };
        if moved != GHOSTTY_SUCCESS {
            return None;
        }
        let mut total: usize = 0;
        let mut index: usize = 0;
        unsafe {
            ghostty_search_get(
                self.handle,
                SEARCH_DATA_TOTAL_MATCHES,
                &mut total as *mut _ as *mut c_void,
            );
            ghostty_search_get(
                self.handle,
                SEARCH_DATA_SELECTED_INDEX,
                &mut index as *mut _ as *mut c_void,
            );
        }
        Some((total, index))
    }

    /// The matches on the visible pages, in viewport coordinates, for
    /// drawing highlight rectangles. Matches slightly outside the
    /// viewport (same page) are clipped away.
    pub fn viewport_matches(&mut self, cols: u16, rows: u16) -> Vec<SearchRect> {
        let mut buffer = GhosttySelectionBuffer {
            ptr: std::ptr::null_mut(),
            cap: 0,
            len: 0,
        };
        unsafe {
            if ghostty_search_get(
                self.handle,
                SEARCH_DATA_VIEWPORT_MATCHES,
                &mut buffer as *mut _ as *mut c_void,
            ) != GHOSTTY_OUT_OF_SPACE
                || buffer.len == 0
            {
                return Vec::new();
            }
        }
        let mut selections = vec![GhosttySelection {
            start: GhosttyGridRef {
                node: std::ptr::null_mut(),
                x: 0,
                y: 0,
            },
            end: GhosttyGridRef {
                node: std::ptr::null_mut(),
                x: 0,
                y: 0,
            },
            rectangle: false,
        }; buffer.len];
        buffer.ptr = selections.as_mut_ptr();
        buffer.cap = selections.len();
        let mut rects = Vec::new();
        unsafe {
            if ghostty_search_get(
                self.handle,
                SEARCH_DATA_VIEWPORT_MATCHES,
                &mut buffer as *mut _ as *mut c_void,
            ) != GHOSTTY_SUCCESS
            {
                return rects;
            }
            for selection in &selections[..buffer.len.min(selections.len())] {
                rects.extend(selection_rects(self.terminal, selection, cols, rows));
            }
        }
        rects
    }
}

/// Turns one selection into per-row viewport rectangles.
fn selection_rects(
    terminal: GhosttyTerminal,
    selection: &GhosttySelection,
    cols: u16,
    rows: u16,
) -> Vec<SearchRect> {
    let mut start = GhosttyPointCoordinate::default();
    let mut end = GhosttyPointCoordinate::default();
    unsafe {
        if ghostty_terminal_point_from_grid_ref(
            terminal,
            &selection.start,
            POINT_TAG_VIEWPORT,
            &mut start,
        ) != GHOSTTY_SUCCESS
            || ghostty_terminal_point_from_grid_ref(
                terminal,
                &selection.end,
                POINT_TAG_VIEWPORT,
                &mut end,
            ) != GHOSTTY_SUCCESS
        {
            return Vec::new();
        }
    }
    if start.y >= u32::from(rows) || end.y >= u32::from(rows) {
        return Vec::new();
    }
    let (top, right, bottom) = if (start.y, start.x) <= (end.y, end.x) {
        (start, end.x, end.y)
    } else {
        (end, start.x, start.y)
    };
    (top.y..=bottom)
        .filter_map(|row| {
            let from = if row == top.y { top.x } else { 0 };
            let to = if row == bottom { right } else { cols.saturating_sub(1) };
            (from <= to).then(|| SearchRect {
                row: row as u16,
                column: from,
                columns: to - from + 1,
            })
        })
        .collect()
}

impl Drop for Search {
    fn drop(&mut self) {
        unsafe { ghostty_search_free(self.handle) };
    }
}

impl<U> Drop for Vt<U> {
    fn drop(&mut self) {
        unsafe {
            ghostty_mouse_event_free(self.mouse_event);
            ghostty_mouse_encoder_free(self.mouse_encoder);
            ghostty_key_event_free(self.key_event);
            ghostty_key_encoder_free(self.key_encoder);
            // Freeing the terminal stops any write_pty callback; the
            // userdata Box is dropped with the struct, after this call.
            ghostty_terminal_free(self.terminal);
        }
    }
}

/// A reader over one row's cells, borrowed from [`Vt::visit_rows`].
pub struct Row<'a> {
    row: GhosttyRenderStateRowIterator,
    cells: GhosttyRenderStateRowCells,
    column: u16,
    _marker: std::marker::PhantomData<&'a mut ()>,
}

impl Row<'_> {
    /// Whether the render state's last update changed this row. Only
    /// meaningful while the dirty level is partial; a full update marks
    /// every row dirty anyway.
    pub fn dirty(&self) -> bool {
        let mut dirty = false;
        unsafe {
            ghostty_render_state_row_get(
                self.row,
                ROW_DATA_DIRTY,
                &mut dirty as *mut _ as *mut c_void,
            );
        }
        dirty
    }

    /// The row's identity across render-state updates. Two ids are the same
    /// row contents when both words are equal; a zeroed id is never valid.
    pub fn id(&self) -> GhosttyRowId {
        let mut id = GhosttyRowId { bits: [0; 2] };
        unsafe {
            ghostty_render_state_row_get(
                self.row,
                ROW_DATA_ID,
                &mut id as *mut _ as *mut c_void,
            );
        }
        id
    }
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
