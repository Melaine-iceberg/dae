//! Verifies libghostty-vt's dirty-tracking behaviour when `clean()` is called
//! after each frame, which `dae_core::pty` relies on for incremental snapshots.
//!
//! Run with `cargo run -p dae-core --example dirty_probe`.

use dae_core::ghostty_vt::{GhosttyRowId, RenderState, Vt};

extern "C" fn write_pty(
    _t: dae_core::ghostty_vt::GhosttyTerminal,
    _u: *mut core::ffi::c_void,
    _d: *const u8,
    _l: usize,
) {
}

extern "C" fn noop_string_changed(
    _t: dae_core::ghostty_vt::GhosttyTerminal,
    _u: *mut core::ffi::c_void,
) {
}

extern "C" fn noop_render_hold(
    _t: dae_core::ghostty_vt::GhosttyTerminal,
    _u: *mut core::ffi::c_void,
    _held: bool,
) {
}

struct Noop;

fn vt() -> (Vt<Noop>, RenderState) {
    let vt = Vt::new(
        80,
        24,
        100,
        write_pty,
        noop_string_changed,
        noop_string_changed,
        noop_render_hold,
        Box::new(Noop),
    )
    .expect("vt");
    let render = RenderState::new().expect("render");
    (vt, render)
}

/// One two-phase update, then `(dirty level, ids per row, dirty flags per row)`.
fn frame(vt: &Vt<Noop>, render: &mut RenderState) -> (u8, Vec<GhosttyRowId>, Vec<bool>) {
    render.begin_update(vt);
    render.end_update();
    let level = render.dirty();
    let mut ids = Vec::new();
    let mut dirty = Vec::new();
    render.visit_rows(|_y, row| {
        ids.push(row.id());
        dirty.push(row.dirty());
    });
    render.clean();
    (level, ids, dirty)
}

fn main() {
    let (mut vt, mut render) = vt();

    let (level, _, dirty) = frame(&vt, &mut render);
    println!(
        "fresh terminal: level={level} dirty_rows={}",
        dirty.iter().filter(|d| **d).count()
    );

    vt.write(b"abc");
    let (level, _, dirty) = frame(&vt, &mut render);
    println!(
        "single-line write: level={level} dirty_rows={}",
        dirty.iter().filter(|d| **d).count()
    );

    let (level, _, _) = frame(&vt, &mut render);
    println!("second update, no write: level={level}");

    // A real scroll: fill the viewport, then one more line.
    vt.write(b"\x1b[H\x1b[2J");
    for line in 0..30 {
        vt.write(format!("row {line}\r\n").as_bytes());
    }
    let (_level, before, _) = frame(&vt, &mut render);
    vt.write(b"scroll line\r\n");
    let (level, after, _) = frame(&vt, &mut render);
    let rows = before.len();
    let shift = (1..rows)
        .filter(|&k| (0..rows - k).all(|i| after[i] == before[i + k]))
        .max();
    println!(
        "after a real scroll: level={level} shift={:?} (rows={rows})",
        shift
    );
}
