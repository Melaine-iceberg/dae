//! Windows' backdrop: Mica, which is Windows 11's own window background.
//!
//! Two facts decide it, and neither is readable from the effect call itself:
//! `DwmSetWindowAttribute` reports nothing when it refuses a backdrop it does
//! not know, and the `transparent` flag has to be chosen before the window
//! exists. So both are read here rather than inferred from a return value —
//! which is the difference between this module and a plain
//! `window.set_effects(Effect::Mica)` that would leave a Windows 10 machine with
//! a translucent tab strip over the user's desktop.
//!
//! * **The build.** Mica needs Windows 11, i.e. build 22000 up. Read from
//!   `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion!CurrentBuild` rather
//!   than `GetVersionExW`, because the latter is compatibility-faked to 6.2 for
//!   any process whose manifest does not claim Windows 10 — and a registry read
//!   is not shimmed. `window-vibrancy`'s own floor is the same 22000 (it reaches
//!   for `RtlGetVersion`), so the two probes cannot disagree about which
//!   machines get a backdrop. Above 22523 it uses the documented
//!   `DWMWA_SYSTEMBACKDROP_TYPE` and below that the undocumented
//!   `DWMWA_MICA_EFFECT`; that split is left to it.
//! * **Whether the user wants one.** Settings › Personalisation › Colours ›
//!   *Transparency effects* writes
//!   `HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Themes\Personalize!EnableTransparency`.
//!   An accessibility setting beating an aesthetic one is the rule, not a
//!   courtesy: with it off, the chrome falls back to the flat canvas the shell
//!   already draws, and nothing has to be re-tuned for it.

use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_NOTIFY, KEY_READ, REG_DWORD,
    REG_NOTIFY_CHANGE_LAST_SET, REG_SAM_FLAGS, REG_SZ, REG_VALUE_TYPE, RegCloseKey,
    RegNotifyChangeKeyValue, RegOpenKeyExW, RegQueryValueExW,
};
use windows::core::PCWSTR;

use tauri::window::{Effect, EffectsBuilder};
use tauri::{Runtime, WebviewWindow};

use super::Material;

/// Windows 11's first build. `window-vibrancy` refuses Mica below this, and the
/// DWM attribute that draws it does not exist above it either.
const MICA_MIN_BUILD: u32 = 22000;

const WINDOWS_NT_KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
/// The build number as text. Kept for compatibility by every Windows 10 and 11
/// release, which is what makes it readable on both.
const CURRENT_BUILD_VALUE: &str = "CurrentBuild";
/// The newer spelling, for a build whose older one is missing.
const CURRENT_BUILD_NUMBER_VALUE: &str = "CurrentBuildNumber";

const PERSONALIZE_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
/// The value the Settings switch writes. Verified against a real machine rather
/// than from documentation: `HKCU\…\Personalize` there carries
/// `EnableTransparency`, `ColorPrevalence`, `AppsUseLightTheme` and
/// `SystemUsesLightTheme`, and no `EnableTransparencyEffects` — a name that
/// reads nothing is read as "the user never turned effects off", so the wrong
/// spelling fails silently on every machine except the test below.
const TRANSPARENCY_VALUE: &str = "EnableTransparency";

/// Encodes a Rust string as a null-terminated UTF-16 vector for `PCWSTR`,
/// matching `default_manager`'s and `system_accent`'s helper.
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn open_key(root: HKEY, path: &str, sam: REG_SAM_FLAGS) -> Option<HKEY> {
    let path = wide(path);
    let mut handle = HKEY::default();
    // SAFETY: `path` is null-terminated and outlives the call; `handle` is a
    // valid out-pointer. The registry functions are FFI, hence unsafe.
    let opened = unsafe { RegOpenKeyExW(root, PCWSTR(path.as_ptr()), Some(0), sam, &mut handle) };
    opened.is_ok().then_some(handle)
}

/// Asks one key for one value, handing the open handle to `read` and closing it
/// on every exit path. Each key is opened per call, as in `system_accent`: it is
/// one `RegOpenKeyExW`, and a handle held across the watcher's blocking wait
/// would be another thing to close on every exit path.
fn query<T>(
    root: HKEY,
    key: &str,
    name: &str,
    kind: REG_VALUE_TYPE,
    read: impl Fn(&[u8]) -> Option<T>,
) -> Option<T> {
    let handle = open_key(root, key, KEY_READ)?;
    let name = wide(name);

    let value = (|| {
        let mut kind = kind;
        let mut size: u32 = 0;
        // SAFETY: `name` outlives the call; `kind`/`size` are valid out-pointers;
        // the data pointer is null on the size probe.
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
        let read_result = unsafe {
            RegQueryValueExW(
                handle,
                PCWSTR(name.as_ptr()),
                None,
                Some(&mut kind),
                Some(buffer.as_mut_ptr()),
                Some(&mut size),
            )
        };
        if read_result.is_err() {
            return None;
        }
        read(&buffer)
    })();

    // SAFETY: `handle` came from open_key and is closed exactly once.
    let _ = unsafe { RegCloseKey(handle) };
    value
}

fn read_dword(key: &str, name: &str) -> Option<u32> {
    query(HKEY_CURRENT_USER, key, name, REG_DWORD, |buffer| {
        // A REG_DWORD is 4 bytes of native-endian integer, whatever the size
        // probe reported.
        Some(u32::from_ne_bytes(buffer.get(..4)?.try_into().ok()?))
    })
}

fn read_string(root: HKEY, key: &str, name: &str) -> Option<String> {
    query(root, key, name, REG_SZ, |buffer| {
        // REG_SZ is UTF-16LE; a trailing NUL may or may not be counted in the
        // reported size, so decode the whole buffer and trim it after.
        let units: Vec<u16> = buffer
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        let text = String::from_utf16_lossy(&units);
        let text = text.trim_end_matches('\0').trim().to_owned();
        (!text.is_empty()).then_some(text)
    })
}

/// Parses `CurrentBuild` — a decimal string, no manifest shim involved.
fn parse_build(raw: Option<String>) -> Option<u32> {
    raw?.trim().parse().ok()
}

/// Whether the user's transparency effects are on. An absent value counts as
/// on: it is the state of a machine that has never turned them off, and group
/// policy and server SKUs both leave the value unwritten rather than writing
/// zero.
fn transparency_enabled() -> bool {
    read_dword(PERSONALIZE_KEY, TRANSPARENCY_VALUE).is_none_or(|value| value != 0)
}

/// The whole decision, separated from the two registry reads so it is testable
/// on a machine that is whatever it happens to be.
fn choose(build: Option<u32>, transparency_on: bool) -> Material {
    match build {
        Some(build) if build >= MICA_MIN_BUILD && transparency_on => Material::Mica,
        _ => Material::None,
    }
}

pub(super) fn read() -> Material {
    let build = parse_build(read_string(
        HKEY_LOCAL_MACHINE,
        WINDOWS_NT_KEY,
        CURRENT_BUILD_VALUE,
    ))
    .or_else(|| {
        parse_build(read_string(
            HKEY_LOCAL_MACHINE,
            WINDOWS_NT_KEY,
            CURRENT_BUILD_NUMBER_VALUE,
        ))
    });
    choose(build, transparency_enabled())
}

/// Hands one window its backdrop.
///
/// The dark variant is chosen rather than plain `Effect::Mica` whenever the
/// shell has said which theme it is drawing in: Mica tints itself from the
/// window's immersive-dark attribute, so asking for the system's answer would
/// put light Mica behind a dark tab strip for anyone who pins an appearance
/// instead of following the OS. The attribute is also what colours the 1px DWM
/// border, which is why this holds even for a shell that never shows a backdrop.
pub(super) fn apply<R: Runtime>(window: &WebviewWindow<R>, material: Material, dark: Option<bool>) {
    let effects = match material {
        Material::Mica => Some(match dark {
            Some(true) => Effect::MicaDark,
            Some(false) => Effect::MicaLight,
            None => Effect::Mica,
        }),
        Material::Vibrancy | Material::None => None,
    };

    match effects {
        Some(effect) => {
            let _ = window.set_effects(EffectsBuilder::new().effect(effect).build());
        }
        None => {
            let _ = window.set_effects(None::<tauri::utils::config::WindowEffectsConfig>);
        }
    }
}

pub(super) fn watch(app: &tauri::AppHandle) {
    super::report(app, read());

    let Some(handle) = open_key(HKEY_CURRENT_USER, PERSONALIZE_KEY, KEY_NOTIFY) else {
        // Without the key there is nothing to watch; the reading above is still
        // correct and the shell simply will not follow later changes — which on
        // this key means a user who turns transparency back on gets it at the
        // next launch rather than a half-drawn window now.
        return;
    };

    loop {
        // SAFETY: `handle` is an open key. With `fasynchronous` false and no
        // event this call blocks the thread until a value under the key is
        // written, which is exactly the wait wanted; `None` is a valid absent
        // `HANDLE` for the synchronous form. Same call `system_accent::windows`
        // makes on the DWM key.
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
        // Every value under `Personalize` wakes this, including the accent and
        // `AppsUseLightTheme`, so the report is what it now reads rather than a
        // flipped local copy. `report` drops it when nothing moved.
        super::report(app, read());
    }

    // SAFETY: `handle` is closed exactly once, on the way out of the loop.
    let _ = unsafe { RegCloseKey(handle) };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The boundary is Windows 11's first build and not one either side of it:
    /// 21996 is the last Windows 10 insider build in that numbering, and
    /// confusing the two is a machine whose window shows its desktop through the
    /// tab strip.
    #[test]
    fn mica_starts_at_windows_11() {
        assert_eq!(choose(Some(21996), true), Material::None);
        assert_eq!(choose(Some(MICA_MIN_BUILD), true), Material::Mica);
        assert_eq!(choose(Some(26100), true), Material::Mica);
    }

    /// The accessibility setting beats the effect.
    #[test]
    fn transparency_effects_off_wins() {
        assert_eq!(choose(Some(26100), false), Material::None);
        assert_eq!(choose(Some(MICA_MIN_BUILD), false), Material::None);
    }

    /// A build that could not be read is treated as one that cannot have Mica.
    /// The other way round would put a transparent window on a machine whose
    /// registry this module failed to read at all.
    #[test]
    fn an_unread_build_gets_no_backdrop() {
        assert_eq!(choose(None, true), Material::None);
    }

    #[test]
    fn reads_the_build_number_as_text() {
        assert_eq!(parse_build(Some("26100".to_owned())), Some(26100));
        assert_eq!(parse_build(Some(" 26100 ".to_owned())), Some(26100));
        assert_eq!(parse_build(Some("26H2".to_owned())), None);
        assert_eq!(parse_build(None), None);
    }

    /// This one is about the machine the tests run on rather than about the
    /// parsing: it is the only check that the two keys above are spelled
    /// correctly and readable without elevation, and `choose` is what turns
    /// their values into a backdrop.
    #[test]
    fn this_machine_answers_both_questions() {
        let build = parse_build(read_string(
            HKEY_LOCAL_MACHINE,
            WINDOWS_NT_KEY,
            CURRENT_BUILD_VALUE,
        ));
        assert!(
            build.is_some_and(|build| build > 10000),
            "CurrentBuild should read as a five-digit build number, got {build:?}"
        );
        // The failure this guards against is a probe pointed at a value Windows
        // never writes: an absent one reads as "effects are on", so a wrong name
        // gives every machine a backdrop and no test but this one would notice.
        let transparency = read_dword(PERSONALIZE_KEY, TRANSPARENCY_VALUE);
        assert!(
            matches!(transparency, Some(0) | Some(1)),
            "{TRANSPARENCY_VALUE} should read as 0 or 1, got {transparency:?}"
        );
    }

    /// A key that is missing entirely has to leave the watcher without a handle
    /// rather than blocking on an invalid one — `watch` documents this path, and
    /// nothing else exercises it.
    #[test]
    fn an_unopenable_key_yields_no_handle() {
        assert!(open_key(HKEY_CURRENT_USER, "Software\\Does\\Not\\Exist", KEY_READ).is_none());
    }
}
