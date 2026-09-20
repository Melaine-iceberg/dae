// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Lives in the bin target rather than `lib.rs` on purpose: mobile builds link
// `dae_lib` as a staticlib/cdylib and must keep the platform allocator.
// Rationale and measurements for the swap: `Cargo.toml` (`mimalloc`).
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    dae_lib::run()
}
