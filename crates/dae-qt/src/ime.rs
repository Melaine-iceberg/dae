//! The focus-change half of gate 2.
//!
//! Qt 6.12's `QWindowsInputContext::reset()` commits the stored composition
//! string to the *old* focus object when the focus moves. Clicking from a
//! composing `TextInput` into another field therefore leaves the raw pinyin
//! (`shi'jie`) in the first field instead of cancelling it. The platform input
//! context cannot be replaced from QML, but cancelling at the IMM level before
//! Qt handles the press removes the composition first, so `reset()` has
//! nothing to commit.
//!
//! The native guard lives in `cpp/ime.h`; this module is the type-safe seam
//! the window controller calls after it has attached an HWND.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("ime.h");
        #[namespace = "dae_qt"]
        #[rust_name = "install_focus_cancel_guard"]
        fn dae_qt_install_focus_cancel_guard(token: &QString) -> bool;
    }
}

pub use qobject::install_focus_cancel_guard;
