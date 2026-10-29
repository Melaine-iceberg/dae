//! The backdrop decision, carried over from the Tauri shell's
//! `window_material/windows.rs` and extended with the user's own choice.
//!
//! The Qt side writes the DWM attributes itself, so this module answers the
//! question the settings surface asks: given the background-effects switch and
//! the requested Windows material, what can this machine actually composite?
//! On Windows the answer is Mica (the default), Acrylic, or `None` when the
//! build, the OS transparency setting, or the user says no. On Wayland it is
//! `Blur` when the compositor advertises `ext-background-effect-v1`, and
//! `None` everywhere else. The registry keys, the build thresholds and the
//! "an unread build gets nothing" rule match the Tauri module, so the two
//! shells cannot disagree about a machine.
//!
//! `window-vibrancy` reaches for `RtlGetVersion` and applies Acrylic through
//! `DWMWA_SYSTEMBACKDROP_TYPE` from build 22523. The Tauri module reads
//! `CurrentBuild` instead of `GetVersionExW` because the latter is
//! compatibility-faked for a process whose manifest does not claim Windows 10.
//! This port keeps the registry read rather than `QOperatingSystemVersion` for
//! the same reason: the Qt application's manifest is not something the shell
//! wants to depend on.

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
    /// Wayland's `ext-background-effect-v1` blur. Windows has no equivalent
    /// name for it; the native call only draws a blur there when the platform
    /// reports the protocol, so the two shells never confuse the two.
    Blur,
    None,
}

impl Material {
    /// The spelling QML and the settings surface use. `blur` is the effective
    /// answer, not one of the Windows choices.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mica => "mica",
            Self::Acrylic => "acrylic",
            Self::Blur => "blur",
            Self::None => "none",
        }
    }

    /// The code the native backdrop call takes.
    pub fn code(self) -> i32 {
        match self {
            Self::Mica => 0,
            Self::Acrylic => 1,
            Self::Blur => 3,
            Self::None => 2,
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "mica" => Some(Self::Mica),
            "acrylic" => Some(Self::Acrylic),
            "blur" => Some(Self::Blur),
            "none" => Some(Self::None),
            _ => None,
        }
    }
}

/// The Windows decision: the accessibility setting wins, then the build band,
/// then the user's choice. Acrylic on a build without
/// `DWMWA_SYSTEMBACKDROP_TYPE` degrades to Mica rather than to no backdrop.
#[cfg_attr(not(windows), allow(dead_code))]
fn choose(build: Option<u32>, transparency_on: bool, requested: &str) -> Material {
    if !transparency_on {
        return Material::None;
    }
    let Some(build) = build else {
        return Material::None;
    };
    if build < MICA_MIN_BUILD {
        return Material::None;
    }
    if requested == "none" {
        return Material::None;
    }
    if requested == "acrylic" && build >= ACRYLIC_MIN_BUILD {
        return Material::Acrylic;
    }
    // `mica`, an old Acrylic request, and any unrecognised value all resolve
    // to the default material.
    Material::Mica
}

/// What the shell may actually draw for the user's settings on this machine.
///
/// Unlike the old startup answer this is not frozen: the settings panel applies
/// a new material to the live window, and an unsupported choice has to collapse
/// to `None` there rather than leave the chrome transparent over a black client.
pub fn resolve(blur_enabled: bool, requested: &str) -> Material {
    if !blur_enabled {
        return Material::None;
    }

    #[cfg(windows)]
    {
        // A/B override, same spelling as before: `mica`, `acrylic` or `none`.
        let requested =
            std::env::var("DAE_WINDOW_MATERIAL").unwrap_or_else(|_| requested.to_owned());
        // A Wayland-only name in the override is not a Windows material.
        let requested = if requested == "blur" {
            "mica".to_owned()
        } else {
            requested
        };
        return read()
            .map(|(build, on)| choose(build, on, &requested))
            .unwrap_or(Material::None);
    }

    #[cfg(target_os = "linux")]
    {
        let _ = requested;
        if crate::window::wayland_blur_supported() {
            return Material::Blur;
        }
        return Material::None;
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = requested;
        Material::None
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Acrylic starts where `DWMWA_SYSTEMBACKDROP_TYPE` starts.
    #[test]
    fn acrylic_starts_with_the_backdrop_attribute() {
        assert_eq!(
            choose(Some(ACRYLIC_MIN_BUILD), true, "acrylic"),
            Material::Acrylic
        );
        assert_eq!(choose(Some(22621), true, "acrylic"), Material::Acrylic);
        assert_eq!(choose(Some(26100), true, "acrylic"), Material::Acrylic);
    }

    /// 21996 is the last Windows 10 insider build in this numbering; treating
    /// it as Windows 11 would put a transparent window with no backdrop on it.
    #[test]
    fn mica_covers_the_builds_that_precede_it() {
        assert_eq!(choose(Some(21996), true, "mica"), Material::None);
        assert_eq!(choose(Some(MICA_MIN_BUILD), true, "mica"), Material::Mica);
        assert_eq!(
            choose(Some(ACRYLIC_MIN_BUILD - 1), true, "acrylic"),
            Material::Mica,
            "an early Windows 11 build falls back to Mica for Acrylic"
        );
    }

    /// The user's choice is honored inside the band the build supports.
    #[test]
    fn the_requested_material_wins_within_the_build_band() {
        assert_eq!(choose(Some(26100), true, "mica"), Material::Mica);
        assert_eq!(choose(Some(26100), true, "acrylic"), Material::Acrylic);
        assert_eq!(choose(Some(26100), true, "none"), Material::None);
    }

    /// The accessibility setting beats the user's choice.
    #[test]
    fn transparency_effects_off_wins() {
        assert_eq!(choose(Some(26100), false, "acrylic"), Material::None);
        assert_eq!(choose(Some(ACRYLIC_MIN_BUILD), false, "mica"), Material::None);
        assert_eq!(choose(Some(MICA_MIN_BUILD), false, "none"), Material::None);
    }

    /// A build that could not be read gets no backdrop: the other way round
    /// would put a transparent window on a machine this module failed to see.
    #[test]
    fn an_unread_build_gets_no_backdrop() {
        assert_eq!(choose(None, true, "mica"), Material::None);
        assert_eq!(choose(None, false, "acrylic"), Material::None);
    }

    /// Turning the setting off is decisive on every platform, before any
    /// capability query runs.
    #[test]
    fn a_disabled_setting_resolves_to_none() {
        assert_eq!(resolve(false, "mica"), Material::None);
        assert_eq!(resolve(false, "acrylic"), Material::None);
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

    #[test]
    fn names_and_codes_round_trip() {
        for material in [
            Material::Mica,
            Material::Acrylic,
            Material::Blur,
            Material::None,
        ] {
            assert_eq!(Material::from_name(material.as_str()), Some(material));
        }
        assert_eq!(Material::Mica.code(), 0);
        assert_eq!(Material::Acrylic.code(), 1);
        assert_eq!(Material::None.code(), 2);
        assert_eq!(Material::Blur.code(), 3);
        assert_eq!(Material::from_name("vibrancy"), None);
    }
}
