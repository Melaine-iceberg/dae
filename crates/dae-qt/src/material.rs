//! The backdrop decision, carried over from the Tauri shell's
//! `window_material/windows.rs`.
//!
//! The Qt side writes the DWM attributes itself, so this module answers only
//! the two questions the Tauri module answered before touching a window:
//! which material this build can composite, and whether the user's
//! transparency setting lets it. The registry keys, the build thresholds and
//! the "an unread build gets nothing" rule are the same, so the two shells
//! cannot disagree about a machine.
//!
//! `window-vibrancy` reaches for `RtlGetVersion` and applies Acrylic through
//! `DWMWA_SYSTEMBACKDROP_TYPE` from build 22523. The Tauri module reads
//! `CurrentBuild` instead of `GetVersionExW` because the latter is
//! compatibility-faked for a process whose manifest does not claim Windows 10.
//! This port keeps the registry read rather than `QOperatingSystemVersion` for
//! the same reason: the Qt application's manifest is not something the shell
//! wants to depend on.

use std::sync::OnceLock;

/// Windows 11's first build. Below it neither DWM backdrop exists.
const MICA_MIN_BUILD: u32 = 22000;

/// The first build with `DWMWA_SYSTEMBACKDROP_TYPE`, which is the only way to
/// ask for Acrylic that actually composites a blur. Below it the early Windows
/// 11 builds keep Mica.
const ACRYLIC_MIN_BUILD: u32 = 22523;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Material {
    Mica,
    Acrylic,
    None,
}

impl Material {
    /// The spelling QML and the settings surface use.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mica => "mica",
            Self::Acrylic => "acrylic",
            Self::None => "none",
        }
    }

    /// The code the native backdrop call takes.
    pub fn code(self) -> i32 {
        match self {
            Self::Mica => 0,
            Self::Acrylic => 1,
            Self::None => 2,
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "mica" => Some(Self::Mica),
            "acrylic" => Some(Self::Acrylic),
            "none" => Some(Self::None),
            _ => None,
        }
    }
}

/// The whole decision: accessibility first, then the build band.
fn choose(build: Option<u32>, transparency_on: bool) -> Material {
    match build {
        Some(_) if !transparency_on => Material::None,
        Some(build) if build >= ACRYLIC_MIN_BUILD => Material::Acrylic,
        Some(build) if build >= MICA_MIN_BUILD => Material::Mica,
        _ => Material::None,
    }
}

/// The decision for this machine. Frozen on first read: a window's transparent
/// flag is fixed at creation, so the answer must not move under it.
pub fn startup() -> Material {
    static STARTUP: OnceLock<Material> = OnceLock::new();
    *STARTUP.get_or_init(|| {
        read()
            .map(|(build, on)| choose(build, on))
            .unwrap_or(Material::None)
    })
}

#[cfg(windows)]
mod registry {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, REG_DWORD, REG_SAM_FLAGS, REG_SZ,
        REG_VALUE_TYPE, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
    };
    use windows::core::PCWSTR;

    const WINDOWS_NT_KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    const CURRENT_BUILD_VALUE: &str = "CurrentBuild";
    const CURRENT_BUILD_NUMBER_VALUE: &str = "CurrentBuildNumber";
    const PERSONALIZE_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
    const TRANSPARENCY_VALUE: &str = "EnableTransparency";

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn open_key(root: HKEY, path: &str, sam: REG_SAM_FLAGS) -> Option<HKEY> {
        let path = wide(path);
        let mut handle = HKEY::default();
        // SAFETY: `path` is null-terminated and outlives the call; `handle` is
        // a valid out-pointer.
        let opened =
            unsafe { RegOpenKeyExW(root, PCWSTR(path.as_ptr()), Some(0), sam, &mut handle) };
        opened.is_ok().then_some(handle)
    }

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
            // SAFETY: `name` outlives the call; `kind`/`size` are valid
            // out-pointers; the data pointer is null on the size probe.
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
            // SAFETY: `buffer` has at least `size` bytes for the out-write;
            // the other pointers are as above.
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
            Some(u32::from_ne_bytes(buffer.get(..4)?.try_into().ok()?))
        })
    }

    fn read_string(root: HKEY, key: &str, name: &str) -> Option<String> {
        query(root, key, name, REG_SZ, |buffer| {
            let units: Vec<u16> = buffer
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_le_bytes(*pair))
                .collect();
            let text = String::from_utf16_lossy(&units);
            let text = text.trim_end_matches('\0').trim().to_owned();
            (!text.is_empty()).then_some(text)
        })
    }

    fn parse_build(raw: Option<String>) -> Option<u32> {
        raw?.trim().parse().ok()
    }

    fn transparency_enabled() -> bool {
        read_dword(PERSONALIZE_KEY, TRANSPARENCY_VALUE).is_none_or(|value| value != 0)
    }

    pub(super) fn read() -> Option<(Option<u32>, bool)> {
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
        Some((build, transparency_enabled()))
    }

    #[cfg(test)]
    pub(super) fn parse_build_for_test(raw: Option<String>) -> Option<u32> {
        parse_build(raw)
    }
}

#[cfg(windows)]
fn read() -> Option<(Option<u32>, bool)> {
    registry::read()
}

#[cfg(not(windows))]
fn read() -> Option<(Option<u32>, bool)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Acrylic starts where `DWMWA_SYSTEMBACKDROP_TYPE` starts.
    #[test]
    fn acrylic_starts_with_the_backdrop_attribute() {
        assert_eq!(choose(Some(ACRYLIC_MIN_BUILD), true), Material::Acrylic);
        assert_eq!(choose(Some(22621), true), Material::Acrylic);
        assert_eq!(choose(Some(26100), true), Material::Acrylic);
    }

    /// 21996 is the last Windows 10 insider build in this numbering; treating
    /// it as Windows 11 would put a transparent window with no backdrop on it.
    #[test]
    fn mica_covers_the_builds_that_precede_it() {
        assert_eq!(choose(Some(21996), true), Material::None);
        assert_eq!(choose(Some(MICA_MIN_BUILD), true), Material::Mica);
        assert_eq!(choose(Some(ACRYLIC_MIN_BUILD - 1), true), Material::Mica);
    }

    /// The accessibility setting beats either material.
    #[test]
    fn transparency_effects_off_wins() {
        assert_eq!(choose(Some(26100), false), Material::None);
        assert_eq!(choose(Some(ACRYLIC_MIN_BUILD), false), Material::None);
        assert_eq!(choose(Some(MICA_MIN_BUILD), false), Material::None);
    }

    /// A build that could not be read gets no backdrop: the other way round
    /// would put a transparent window on a machine this module failed to see.
    #[test]
    fn an_unread_build_gets_no_backdrop() {
        assert_eq!(choose(None, true), Material::None);
        assert_eq!(choose(None, false), Material::None);
    }

    #[cfg(windows)]
    #[test]
    fn reads_the_build_number_as_text() {
        assert_eq!(
            registry::parse_build_for_test(Some("26100".to_owned())),
            Some(26100)
        );
        assert_eq!(
            registry::parse_build_for_test(Some(" 26100 ".to_owned())),
            Some(26100)
        );
        assert_eq!(
            registry::parse_build_for_test(Some("26H2".to_owned())),
            None
        );
        assert_eq!(registry::parse_build_for_test(None), None);
    }

    /// The startup answer is what the window's transparency was built from, so
    /// a second read that disagreed would silently repaint it.
    #[test]
    fn the_startup_answer_never_moves() {
        assert_eq!(startup(), startup());
    }

    #[test]
    fn names_and_codes_round_trip() {
        for material in [Material::Mica, Material::Acrylic, Material::None] {
            assert_eq!(Material::from_name(material.as_str()), Some(material));
        }
        assert_eq!(Material::Mica.code(), 0);
        assert_eq!(Material::Acrylic.code(), 1);
        assert_eq!(Material::None.code(), 2);
        assert_eq!(Material::from_name("vibrancy"), None);
    }
}
