//! Documents the vendored libghostty-vt 1.0.1's dirty-tracking behaviour,
//! which `dae_core::pty` relies on when it picks a frame kind.
//!
//! Run with `cargo run -p dae-core --example dirty_probe`. Expected output:
//! the dirty level is always `FULL` (2) — even for a bare cursor move or a
//! second update with no write in between — and every row reports dirty.
//! The one usable incremental signal is row identity: after a real
//! viewport scroll, new row i carries the id old row i+k had.
//!
//! If a library upgrade changes this (partial dirty that actually fires),
//! `pty.rs` can go back to per-row patching instead of whole-grid reads.

use dae_core::ghostty_vt::{GhosttyRowId, Vt};

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

struct Noop;

fn vt() -> Vt<Noop> {
    Vt::new(
        80,
        24,
        100,
        write_pty,
        noop_string_changed,
        noop_string_changed,
        Box::new(Noop),
    )
    .expect("vt")
}

/// One update, then `(dirty level, ids per row, dirty flags per row)`.
fn frame(vt: &mut Vt<Noop>) -> (u8, Vec<GhosttyRowId>, Vec<bool>) {
    vt.update();
    let level = vt.dirty();
    let mut ids = Vec::new();
    let mut dirty = Vec::new();
    vt.visit_rows(|_y, row| {
        ids.push(row.id());
        dirty.push(row.dirty());
    });
    (level, ids, dirty)
}

fn main() {
    let mut vt = vt();

    let (level, _, dirty) = frame(&mut vt);
    println!(
        "fresh terminal: level={level} dirty_rows={}",
        dirty.iter().filter(|d| **d).count()
    );

    vt.write(b"abc");
    let (level, _, dirty) = frame(&mut vt);
    println!(
        "bare cursor move: level={level} dirty_rows={}",
        dirty.iter().filter(|d| **d).count()
    );

    let (level, _, _) = frame(&mut vt);
    println!("second update, no write: level={level}");

    // A real scroll: fill the viewport, then one more line.
    vt.write(b"\x1b[H\x1b[2J");
    for line in 0..30 {
        vt.write(format!("row {line}\r\n").as_bytes());
    }
    let (level, before, _) = frame(&mut vt);
    vt.write(b"scroll line\r\n");
    let (level, after, _) = frame(&mut vt);
    let rows = before.len();
    let shift = (1..rows)
        .filter(|&k| (0..rows - k).all(|i| after[i] == before[i + k]))
        .max();
    println!(
        "after a real scroll: level={level} shift={:?} (rows={rows})",
        shift
    );
}
