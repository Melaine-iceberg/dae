//! `dae://` deep links and the single-instance boundary.
//!
//! The native half (`cpp/activation.cpp`) owns a per-user lock file and local
//! socket. The first process keeps its windows and tabs; a later launch sends
//! its command line, plus the directory it was started in, and exits. This
//! module parses those arguments before QML sees them, so the shell is handed
//! only directories that exist. It also registers the `dae` URL scheme in the
//! current user's registry after winning the slot.
//!
//! Supported spellings, matching the Tauri shell:
//!
//! * `dae://open?path=<percent-encoded path>`
//! * `dae://open/<percent-encoded path>`
//! * `dae:///<percent-encoded path>`
//!
//! A file target resolves to its directory; a target that no longer exists is
//! dropped. A second launch with no valid target still brings the running
//! window forward, because the native side reports every activation.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use cxx_qt_lib::{QString, QStringList};

/// The most targets one activation may open. The native side applies the same
/// limit to forwarded arguments.
const MAX_TARGETS: usize = 32;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;

        include!("activation.h");
        #[namespace = "dae_qt"]
        #[rust_name = "activation_claim"]
        fn dae_qt_activation_claim() -> bool;

        include!("macos_menu.h");
        #[namespace = "dae_qt"]
        fn dae_qt_install_macos_menu();
    }

    extern "Rust" {
        /// Turns a forwarded command line into openable directories. Called by
        /// the native server in `cpp/activation.cpp` before it wakes QML.
        fn dae_qt_activation_resolve(
            args: &QStringList,
            working_directory: &QString,
        ) -> QStringList;
    }
}

/// Claims the process-wide single-instance slot.
///
/// Returns `true` for the first process, which should create the shell. A
/// later launch forwards its arguments to the owner and returns `false`, so
/// `main` can exit without creating an engine or a window. Only the owner
/// registers the URL scheme; registering from a forwarded launch would rewrite
/// the registry on every open.
pub fn claim() -> bool {
    if !qobject::activation_claim() {
        return false;
    }

    #[cfg(windows)]
    if let Err(error) = register_url_scheme() {
        log::warn!(target: "dae::activation", "could not register dae:// URL scheme: {error}");
    }

    // A portable Linux build has no installer to put dae.desktop in place; the
    // scheme handler is registered from the running executable instead, the
    // same way Windows rewrites its registry command on every launch.
    #[cfg(target_os = "linux")]
    crate::linux_desktop::ensure_registered();

    // QGuiApplication has no menu bar; macOS needs one for Cmd+Q, About,
    // Services and the Window menu. Qt's own NSView answers the edit
    // selectors through the responder chain, so this is the whole menu.
    #[cfg(target_os = "macos")]
    qobject::dae_qt_install_macos_menu();

    true
}

/// Where a launch target came from. The status line names the source so a
/// deep link is distinguishable from a plain command-line directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetSource {
    DeepLink,
    CommandLine,
}

/// One resolved thing to open: a directory, plus the form that named it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchTarget {
    pub path: PathBuf,
    pub source: TargetSource,
}

/// Resolves one command-line argument.
///
/// `working_directory` is the directory the launching process started in; it
/// is needed because the owner may be running somewhere else when a second
/// launch forwards a relative path. Absolute arguments ignore it.
pub fn target_for_arg(arg: &OsStr, working_directory: &Path) -> Option<LaunchTarget> {
    if let Some(text) = arg.to_str() {
        if looks_like_deep_link(text) {
            return parse_open_directory(text, working_directory).map(|path| LaunchTarget {
                path,
                source: TargetSource::DeepLink,
            });
        }
    }

    resolve_directory(&PathBuf::from(arg), working_directory).map(|path| LaunchTarget {
        path,
        source: TargetSource::CommandLine,
    })
}

/// The Rust side of the native server: resolve every forwarded argument, drop
/// duplicates, and return the paths QML should open.
fn dae_qt_activation_resolve(args: &QStringList, working_directory: &QString) -> QStringList {
    let base = {
        let text = working_directory.to_string();
        if text.is_empty() {
            std::env::current_dir().unwrap_or_default()
        } else {
            PathBuf::from(text)
        }
    };

    let mut seen = Vec::new();
    let mut resolved = QStringList::default();
    for arg in args.iter() {
        let text = arg.to_string();
        let Some(target) = target_for_arg(OsStr::new(&text), &base) else {
            continue;
        };
        if seen.contains(&target.path) {
            continue;
        }
        seen.push(target.path.clone());
        resolved.append(QString::from(target.path.to_string_lossy().as_ref()));
        if resolved.len() >= MAX_TARGETS as isize {
            break;
        }
    }
    resolved
}

fn looks_like_deep_link(text: &str) -> bool {
    text.get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("dae:"))
}

fn parse_open_directory(raw: &str, base: &Path) -> Option<PathBuf> {
    let url = url::Url::parse(raw).ok()?;
    if !url.scheme().eq_ignore_ascii_case("dae") {
        return None;
    }

    // `?path=` is the documented form. The query is parsed by hand rather than
    // with `form_urlencoded` so a literal `+` in a folder name is not turned
    // into a space; only `%XX` sequences are decoded.
    let encoded = url
        .query()
        .and_then(|query| query_path_value(query))
        .or_else(|| {
            let path = url.path().trim_start_matches('/');
            (!path.is_empty()).then(|| path.to_owned())
        })?;
    if encoded.is_empty() {
        return None;
    }
    let decoded = percent_encoding::percent_decode_str(&encoded)
        .decode_utf8()
        .ok()?;
    resolve_directory(Path::new(decoded.as_ref()), base)
}

/// The raw `path` value from a query string, still percent-encoded.
fn query_path_value(query: &str) -> Option<String> {
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let Ok(decoded_key) = percent_encoding::percent_decode_str(key).decode_utf8() else {
            continue;
        };
        if decoded_key == "path" {
            return Some(value.to_owned());
        }
    }
    None
}

/// A target becomes its own directory when it is one, or its parent when it is
/// a file. Relative targets are resolved against the launching directory.
fn resolve_directory(path: &Path, base: &Path) -> Option<PathBuf> {
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };

    if candidate.is_dir() {
        return Some(candidate);
    }
    if candidate.is_file() {
        return candidate
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(Path::to_path_buf);
    }
    None
}

/// The command Windows stores under `shell\open\command`. `%1` is replaced by
/// the launching shell with the `dae://` URL.
#[cfg_attr(not(windows), allow(dead_code))]
fn url_open_command(executable: &Path) -> String {
    format!("\"{}\" \"%1\"", executable.display())
}

#[cfg_attr(not(windows), allow(dead_code))]
fn default_icon_value(executable: &Path) -> String {
    format!("\"{}\",0", executable.display())
}

/// Registers `dae://` for the current user. Writing `HKCU\Software\Classes`
/// needs no elevation and works for a portable copy without an installer. A
/// later launch rewrites the command if the executable moved.
#[cfg(windows)]
fn register_url_scheme() -> Result<(), String> {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
        RegCreateKeyExW, RegSetValueExW,
    };
    use windows::core::PCWSTR;

    const PROTOCOL_KEY: &str = r"Software\Classes\dae";
    const COMMAND_KEY: &str = r"Software\Classes\dae\shell\open\command";

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn create_key(path: &str) -> Result<HKEY, String> {
        let wide_path = wide(path);
        let mut key = HKEY::default();
        // SAFETY: `wide_path` is null-terminated and outlives the call; `key` is
        // a valid out-pointer and no security attributes or class are requested.
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(wide_path.as_ptr()),
                Some(0),
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                &mut key,
                None,
            )
        };
        if status.is_err() {
            return Err(format!("{path} (Windows error {})", status.0));
        }
        Ok(key)
    }

    fn set_string(key: HKEY, name: Option<&str>, value: &str) -> Result<(), String> {
        let name = name.map(wide);
        // REG_SZ bytes include the trailing NUL.
        let value = wide(value);
        // SAFETY: the value buffer is a live UTF-16 vector; announcing exactly
        // its byte length lets Windows copy it, and `name` lives until return.
        let bytes =
            unsafe { std::slice::from_raw_parts(value.as_ptr().cast::<u8>(), value.len() * 2) };
        let status = unsafe {
            RegSetValueExW(
                key,
                name.as_ref()
                    .map_or(PCWSTR::null(), |name| PCWSTR(name.as_ptr())),
                None,
                REG_SZ,
                Some(bytes),
            )
        };
        if status.is_err() {
            return Err(format!("Windows error {}", status.0));
        }
        Ok(())
    }

    let executable = std::env::current_exe()
        .map_err(|error| format!("current executable is unknown: {error}"))?;

    let protocol = create_key(PROTOCOL_KEY)?;
    let command = create_key(COMMAND_KEY)?;

    let result = (|| {
        set_string(protocol, None, "URL:dae")?;
        set_string(protocol, Some("URL Protocol"), "")?;
        set_string(
            protocol,
            Some("DefaultIcon"),
            &default_icon_value(&executable),
        )?;
        set_string(command, None, &url_open_command(&executable))
    })();

    let _ = unsafe { RegCloseKey(command) };
    let _ = unsafe { RegCloseKey(protocol) };
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "dae-qt-activation-{}-{}-{label}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).expect("create temp directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn encoded(path: &Path) -> String {
        url::form_urlencoded::byte_serialize(path.to_string_lossy().as_bytes()).collect()
    }

    fn resolve(raw: &str, base: &Path) -> Option<PathBuf> {
        target_for_arg(OsStr::new(raw), base).map(|target| target.path)
    }

    #[test]
    fn query_and_path_forms_resolve_to_the_same_directory() {
        let temp = TempDir::new("forms");
        let directory = temp.path();

        assert_eq!(
            resolve(
                &format!("dae://open?path={}", encoded(directory)),
                directory
            )
            .as_deref(),
            Some(directory)
        );
        assert_eq!(
            resolve(&format!("dae://open/{}", encoded(directory)), directory).as_deref(),
            Some(directory)
        );
        assert_eq!(
            resolve(&format!("dae:///{}", encoded(directory)), directory).as_deref(),
            Some(directory)
        );
    }

    #[test]
    fn a_file_target_opens_its_directory() {
        let temp = TempDir::new("file");
        let file = temp.path().join("notes.txt");
        std::fs::write(&file, b"x").expect("write temp file");

        assert_eq!(
            resolve(file.to_string_lossy().as_ref(), temp.path()).as_deref(),
            Some(temp.path())
        );
        assert_eq!(
            resolve(&format!("dae://open?path={}", encoded(&file)), temp.path()).as_deref(),
            Some(temp.path())
        );
    }

    #[test]
    fn a_relative_target_uses_the_launching_directory() {
        let temp = TempDir::new("relative");
        let child = temp.path().join("child");
        std::fs::create_dir_all(&child).expect("create child directory");

        assert_eq!(
            resolve("child", temp.path()).as_deref(),
            Some(child.as_path())
        );
        assert_eq!(
            resolve("dae://open?path=child", temp.path()).as_deref(),
            Some(child.as_path())
        );
    }

    #[test]
    fn foreign_schemes_flags_and_missing_paths_are_not_targets() {
        let temp = TempDir::new("reject");
        let missing = temp.path().join("missing");

        assert_eq!(resolve("https://example.com", temp.path()), None);
        assert_eq!(resolve("dae://open?path=not-there", temp.path()), None);
        assert_eq!(resolve("--some-flag", temp.path()), None);
        assert_eq!(
            resolve(missing.to_string_lossy().as_ref(), temp.path()),
            None
        );
        assert_eq!(target_for_arg(OsStr::new("dae://open"), temp.path()), None);
    }

    #[test]
    fn the_target_source_names_a_deep_link() {
        let temp = TempDir::new("source");
        let target = target_for_arg(
            OsStr::new(&format!("dae://open?path={}", encoded(temp.path()))),
            temp.path(),
        )
        .expect("deep link target");

        assert_eq!(target.source, TargetSource::DeepLink);
        assert_eq!(target.path, temp.path());
    }

    #[test]
    fn a_literal_plus_in_a_query_path_is_not_a_space() {
        let temp = TempDir::new("plus");
        let directory = temp.path().join("a+b");
        std::fs::create_dir_all(&directory).expect("create plus directory");

        assert_eq!(
            resolve(
                &format!("dae://open?path={}", encoded(&directory)),
                temp.path()
            )
            .as_deref(),
            Some(directory.as_path())
        );
        assert_eq!(
            resolve(
                &format!(
                    "dae://open?path={}",
                    directory.to_string_lossy().replace('+', "%2B")
                ),
                temp.path()
            )
            .as_deref(),
            Some(directory.as_path())
        );
        // An unencoded plus must stay a plus; only percent escapes decode.
        assert_eq!(
            resolve(
                &format!("dae://open?path={}", directory.to_string_lossy()),
                temp.path()
            )
            .as_deref(),
            Some(directory.as_path())
        );
    }

    #[test]
    fn forwarded_arguments_are_resolved_once() {
        let temp = TempDir::new("forwarded");
        let child = temp.path().join("child");
        std::fs::create_dir_all(&child).expect("create child directory");

        let mut args = QStringList::default();
        args.append(QString::from("--flag"));
        args.append(QString::from("child"));
        args.append(QString::from(child.to_string_lossy().as_ref()));

        let resolved = dae_qt_activation_resolve(
            &args,
            &QString::from(temp.path().to_string_lossy().as_ref()),
        );
        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved.get(0).unwrap().to_string(),
            child.to_string_lossy()
        );
    }

    #[test]
    fn the_registry_command_quotes_the_executable() {
        assert_eq!(
            url_open_command(Path::new(r"C:\Program Files\Dae\dae.exe")),
            r#""C:\Program Files\Dae\dae.exe" "%1""#
        );
        assert_eq!(
            default_icon_value(Path::new(r"C:\Dae\dae.exe")),
            r#""C:\Dae\dae.exe",0"#
        );
    }
}
