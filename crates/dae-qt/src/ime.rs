//! The focus-change half of gate 2, on all three platforms.
//!
//! Rule: a composition is cancelled, never committed, when the focus leaves the
//! item that owns it. Where the cancel happens differs by platform, because the
//! input contexts disagree about what `reset()` means:
//!
//! - **Windows.** Qt 6.12's `QWindowsInputContext::reset()` *commits* the stored
//!   composition string to the old focus object, so clicking from a composing
//!   `TextInput` into another field leaves the raw pinyin (`shi'jie`) behind.
//!   The guard cancels at the IMM level before Qt handles the press, so
//!   `reset()` has nothing to commit.
//! - **macOS and Linux.** `QCocoaInputContext::reset()` discards the marked
//!   text and `QIBusPlatformInputContext::reset()` drops the preedit, so the
//!   same interception point calls `QInputMethod::reset()` and gets a cancel.
//!   Both already cancel on a focus change, which is why the Windows defect has
//!   no counterpart there; the guard keeps the shell's behaviour independent of
//!   that policy.
//!
//! The native guard lives in `cpp/ime.h`; this module is the type-safe seam the
//! window controller calls after it has attached the window. It answers whether
//! the token named a live window on every platform, so a caller that ignores
//! the result still gets the same wiring.

#[cxx::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("ime.h");
        #[namespace = "dae_qt"]
        #[rust_name = "install_focus_cancel_guard"]
        fn dae_qt_install_focus_cancel_guard_rs(token: &str) -> bool;
    }
}

pub use ffi::install_focus_cancel_guard;
