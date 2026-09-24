//! Windows' accent: the value the Settings › Personalisation › Colours picker
//! writes into the DWM key. Reading the registry rather than asking the WinRT
//! `UISettings` API is deliberate — `windows` is already a dependency and the
//! key is what every non-UWP app reads.
//!
//! Change detection is `RegNotifyChangeKeyValue` on that key rather than the
//! `WM_DWMCOLORIZATIONCOLORCHANGED` broadcast, because the broadcast needs a
//! hidden window and a message pump to receive it, and it reports the
//! *colorization* colour (title bars) rather than the accent. Blocking on a
//! registry key costs one thread and one event.

use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, KEY_READ, REG_DWORD, REG_NOTIFY_CHANGE_LAST_SET,
    RegCloseKey, RegNotifyChangeKeyValue, RegOpenKeyExW, RegQueryValueExW,
};
use windows::core::PCWSTR;

const DWM_KEY: &str = r"Software\Microsoft\Windows\DWM";
const ACCENT_VALUE: &str = "AccentColor";
/// Fallback for the Windows 10 builds whose DWM key predates `AccentColor`.
/// This is the *colorization* colour — what the title bars use — rather than
/// the accent, which is why it is only ever asked second, and why it decodes
/// as ARGB where `AccentColor` decodes as ABGR.
const COLORIZATION_VALUE: &str = "ColorizationColor";

/// Encodes a Rust string as a null-terminated UTF-16 vector for `PCWSTR`,
/// matching `default_manager`'s helper.
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn open_key(sam: windows::Win32::System::Registry::REG_SAM_FLAGS) -> Option<HKEY> {
    let path = wide(DWM_KEY);
    let mut handle = HKEY::default();
    // SAFETY: `path` is null-terminated and outlives the call; `handle` is a
    // valid out-pointer. The registry functions are FFI, hence unsafe.
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(path.as_ptr()),
            Some(0),
            sam,
            &mut handle,
        )
    };
    opened.is_ok().then_some(handle)
}

pub(super) fn read() -> Option<String> {
    read_dword(ACCENT_VALUE)
        .and_then(super::from_abgr)
        .or_else(|| read_dword(COLORIZATION_VALUE).and_then(super::from_argb))
}

/// One REG_DWORD out of the DWM key, or `None` if the key, the value or the
/// read is missing. The key is opened per call: it is one `RegOpenKeyExW`, and
/// a handle held across the watcher's blocking wait would be another thing to
/// close on every exit path.
fn read_dword(name: &str) -> Option<u32> {
    let handle = open_key(KEY_READ)?;
    let name = wide(name);

    let read_value = (|| {
        let mut kind = REG_DWORD;
        let mut size: u32 = 0;
        // SAFETY: `name` outlives the call; `kind`/`size` are valid
        // out-pointers; `lpdata` is null on the size probe.
        let probe = unsafe {
            RegQueryValueExW(
                handle,
                PCWSTR(name.as_ptr()),
                None,
                Some(&mut kind),
                None,
                Some(&mut size),
            )
        };
        if probe.is_err() {
            return None;
        }

        let mut buffer: Vec<u8> = vec![0; size.max(4) as usize];
        // SAFETY: `buffer` has at least `size` bytes of capacity for the
        // out-write; the other pointers are as above.
        let read = unsafe {
            RegQueryValueExW(
                handle,
                PCWSTR(name.as_ptr()),
                None,
                Some(&mut kind),
                Some(buffer.as_mut_ptr()),
                Some(&mut size),
            )
        };
        if read.is_err() {
            return None;
        }
        // A REG_DWORD is 4 bytes of native-endian integer, whatever the size
        // probe reported.
        Some(u32::from_ne_bytes(buffer[..4].try_into().ok()?))
    })();

    // SAFETY: `handle` came from open_key and is closed exactly once.
    let _ = unsafe { RegCloseKey(handle) };
    read_value
}

pub(super) fn watch(app: &tauri::AppHandle) {
    super::report(app, read());

    let Some(handle) = open_key(KEY_NOTIFY) else {
        // Without the key there is nothing to watch; the one reading above is
        // still correct and the shell simply will not follow later changes.
        return;
    };

    loop {
        // SAFETY: `handle` is an open key. With `fasynchronous` false and no
        // event this call blocks the thread until a value under the key is
        // written, which is exactly the wait wanted; `None` is a valid absent
        // `HANDLE` for the synchronous form.
        let changed = unsafe {
            RegNotifyChangeKeyValue(
                handle,
                false,
                REG_NOTIFY_CHANGE_LAST_SET,
                None,
                false,
            )
        };
        if changed.is_err() {
            break;
        }
        super::report(app, read());
    }

    // SAFETY: `handle` is closed exactly once, on the way out of the loop.
    let _ = unsafe { RegCloseKey(handle) };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads this machine's real DWM key.
    ///
    /// A smoke test of the FFI rather than of a value — nothing here asserts
    /// *which* accent this machine has. What it pins is that `open_key` and
    /// `read_dword` agree with how the registry actually stores a REG_DWORD: a
    /// wrong buffer size, a missing terminator or a mis-sized out-write reads
    /// as plausible-looking garbage and is caught nowhere else.
    #[test]
    fn reads_this_machines_accent_out_of_the_dwm_key() {
        let packed = read_dword(ACCENT_VALUE).expect("this machine has a DWM AccentColor");
        let decoded = super::super::from_abgr(packed).expect("a nonzero accent decodes to a hex");
        assert_eq!(decoded.len(), 7);
        assert!(decoded.starts_with('#'));

        // `read` is what the app actually calls, and it has to agree with the
        // direct read it is built from.
        assert_eq!(read().as_deref(), Some(decoded.as_str()));
    }

    /// Pins the one behaviour `watch` rests on that nothing else exercises:
    /// that `RegNotifyChangeKeyValue` *blocks* in its synchronous form and
    /// takes no event handle.
    ///
    /// If either were wrong the call would return immediately with an error,
    /// `watch` would fall out of its loop, and the accent would quietly stop
    /// following the system — a bug that only shows up on the first real
    /// change, long after this file is forgotten.
    ///
    /// Runs against a key created for the purpose and torn down afterwards, so
    /// it never disturbs the DWM key production watches.
    #[test]
    fn a_registry_write_unblocks_a_synchronous_watch() {
        use std::sync::mpsc;
        use windows::Win32::System::Registry::{
            KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW,
        };

        const TEST_KEY: &str = r"Software\dae-accent-watch-test";
        const TEST_VALUE: &str = "probe";

        let path = wide(TEST_KEY);
        // Drop anything a previous aborted run left behind, so the write below
        // is always a change and never a no-op.
        // SAFETY: `path` is null-terminated and outlives the call.
        let _ = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr())) };

        let mut key = HKEY::default();
        // SAFETY: `path` outlives the call; `key` is a valid out-pointer.
        let created = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(path.as_ptr()),
                Some(0),
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_NOTIFY | KEY_SET_VALUE,
                None,
                &mut key,
                None,
            )
        };
        assert!(created.is_ok(), "could not create the scratch key");

        let (sender, receiver) = mpsc::channel();
        // `HKEY` wraps a raw pointer and so opts out of `Send`, but a registry
        // key is process-wide and this is exactly the move `watch`'s own
        // thread does at startup. Carry it as its address and rebuild it there.
        let handle_address = key.0 as usize;
        std::thread::spawn(move || {
            let key = HKEY(handle_address as *mut core::ffi::c_void);
            // SAFETY: `key` is an open handle this thread now owns. This is
            // the exact call `watch` makes: synchronous, no event.
            let changed = unsafe {
                RegNotifyChangeKeyValue(key, false, REG_NOTIFY_CHANGE_LAST_SET, None, false)
            };
            // SAFETY: `key` is closed exactly once, here.
            let _ = unsafe { RegCloseKey(key) };
            let _ = sender.send(changed.is_ok());
        });

        // Let the watcher reach the blocking call before the write that is
        // meant to release it — a write that lands first is never observed and
        // the test would then hang on a wait nothing will end.
        std::thread::sleep(std::time::Duration::from_millis(300));

        let name = wide(TEST_VALUE);
        let payload = 1u32.to_ne_bytes();
        // SAFETY: `name` outlives the call; `payload` is a REG_DWORD's worth.
        let wrote = unsafe {
            RegSetValueExW(
                key,
                PCWSTR(name.as_ptr()),
                Some(0),
                REG_DWORD,
                Some(payload.as_slice()),
            )
        };
        assert!(wrote.is_ok(), "could not write the scratch value");

        let unblocked = receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap_or(false);
        // SAFETY: `path` is null-terminated; this removes the scratch subtree.
        let _ = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr())) };

        assert!(
            unblocked,
            "the synchronous watch never reported the write: it is either not \
             blocking (so `watch` spins) or it rejected the absent event handle \
             (so `watch` stops after one change)"
        );
    }
}
