//! Whether this process was started from an installed app package.
//!
//! Windows is the only platform with the concept today (MSIX). The answer
//! changes two behaviours, and both of them are file-system facts rather than
//! preferences:
//!
//! * An installed package lives under `C:\Program Files\WindowsApps\<name>_
//!   <version>_<arch>__<publisher>`, which is read-only and gets a **new
//!   directory name on every update**. The portable updater in
//!   [`crate::update`] swaps files beside the executable, so it cannot work
//!   there: it could neither write the tree nor keep a stable path for the
//!   shortcuts and registry entries pointing at it. Updates have to arrive as a
//!   new package, so the updater reports itself unavailable.
//! * `dae://` is declared by the package manifest, which registers it for the
//!   application rather than for one executable path. Writing the per-user
//!   `HKCU\Software\Classes\dae` command on top of that would pin the scheme to
//!   the version directory that happens to be running, and the next update
//!   would leave a stale path behind.
//!
//! Nothing else changes: a packaged process still reads and writes
//! `%LOCALAPPDATA%` through the same [`crate::paths`] helpers, because
//! full-trust MSIX apps are not redirected.

/// True when the current process belongs to an installed Windows app package.
///
/// The check is a probe, not a path test: `GetCurrentPackageFullName` answers
/// for the running process, so a portable copy that happens to sit inside a
/// package's directory would still report `false`.
#[cfg(windows)]
pub fn is_packaged() -> bool {
    use windows::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER;
    use windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;

    // A zero-length buffer with no pointer is the documented size probe: the
    // call writes the required length and reports `ERROR_INSUFFICIENT_BUFFER`
    // when the process has an identity, and `APPMODEL_ERROR_NO_PACKAGE`
    // (15700) when it does not. Only the former counts — any other failure
    // leaves us saying "not packaged", which keeps the portable path (the one
    // that works everywhere) in charge.
    let mut length = 0u32;
    // SAFETY: the out-pointer is a live local; a null buffer with a zero length
    // means the call must not write through it, which is what the API documents
    // for this probe.
    let status = unsafe { GetCurrentPackageFullName(&mut length, None) };
    status == ERROR_INSUFFICIENT_BUFFER
}

#[cfg(not(windows))]
pub fn is_packaged() -> bool {
    false
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// The test binary is never deployed inside a package, so this both covers
    /// the call and pins the "portable by default" answer the updater relies on
    /// during `cargo test`.
    #[test]
    fn a_test_binary_is_not_packaged() {
        assert!(!is_packaged());
    }
}
