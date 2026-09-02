//! `dae://` deep links: browsers and other apps open directories in the
//! running explorer by URL. Combined with the single-instance plugin, a
//! second launch forwards its CLI args (including the URL) to the running
//! instance; on macOS the deep-link plugin delivers URLs via its open-url
//! event instead.

use serde::Serialize;
use specta::Type;
use tauri::Manager;
use tauri_specta::Event;

use crate::file_system::error::FileSystemError;

/// Emitted when a `dae://` URL asks to show a directory; the frontend
/// navigates the active pane to the payload path.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[tauri_specta(event_name = "deep-link-open-directory")]
pub struct OpenDirectoryRequested(pub String);

/// Buffers the latest requested directory until the webview picks it up:
/// a deep link that lands before the frontend mounts its listener would
/// otherwise be lost, since Tauri events are fire-and-forget.
#[derive(Default)]
pub struct PendingDeepLink(std::sync::Mutex<Option<String>>);

/// Returns the buffered deep-link directory (if any) and clears the buffer.
/// Called once on startup, after the listener is registered.
#[tauri::command]
#[specta::specta]
pub fn take_pending_open_directory(
    state: tauri::State<'_, PendingDeepLink>,
) -> Result<Option<String>, FileSystemError> {
    Ok(state.take())
}

impl PendingDeepLink {
    fn set(&self, directory: String) {
        *self.0.lock().expect("deep-link lock poisoned") = Some(directory);
    }

    fn take(&self) -> Option<String> {
        self.0.lock().expect("deep-link lock poisoned").take()
    }
}

/// Supported spellings, all percent-encoded where needed:
///   dae://open?path=C:\Users\me\Downloads
///   dae://open/C%3A%5CUsers%5Cme%5CDownloads
///   dae://open/home/me/Downloads
pub fn parse_open_directory(raw: &str) -> Option<String> {
    let url = url::Url::parse(raw).ok()?;
    if url.scheme() != "dae" {
        return None;
    }

    // The query form arrives already percent-decoded; the path form keeps
    // its encoding, so it needs an explicit decode (percent-decoding, not
    // `form_urlencoded`, which would wrongly turn `+` into spaces).
    let path = if let Some((_, value)) = url.query_pairs().find(|(key, _)| key == "path") {
        value.into_owned()
    } else {
        let encoded = url.path().trim_start_matches('/');
        if encoded.is_empty() {
            return None;
        }
        percent_encoding::percent_decode_str(encoded)
            .decode_utf8()
            .ok()?
            .into_owned()
    };

    resolve_existing_directory(&path)
}

/// Maps a path onto a directory that actually exists, stepping up to the
/// parent for file targets so a "show this file" link still lands somewhere
/// browsable.
fn resolve_existing_directory(path: &str) -> Option<String> {
    let path = std::path::Path::new(path);
    let metadata = std::fs::metadata(path).ok()?;

    let directory = if metadata.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };

    directory.to_str().map(str::to_owned)
}

/// Handles one activation whose `args` may carry `dae://` URLs — the
/// single-instance forward from a duplicate launch, or the initial launch's
/// own CLI on Windows/Linux. A bare directory path is also honored: that is
/// what the OS passes when dae is launched as the default folder handler.
/// Buffers and emits one navigation event per resolvable target and brings
/// the running window to the front either way.
pub fn handle_activation(app: &tauri::AppHandle, args: &[String]) {
    for arg in args {
        let directory = parse_open_directory(arg).or_else(|| raw_directory_arg(arg));
        if let Some(directory) = directory {
            // Buffer first so a pre-mount deep link survives until the
            // frontend pulls it through `take_pending_open_directory`.
            if let Some(state) = app.try_state::<PendingDeepLink>() {
                state.set(directory.clone());
            }
            let _ = OpenDirectoryRequested(directory).emit(app);
        }
    }

    focus_main_window(app);
}

/// A bare directory path argument, as the OS supplies when dae is the default
/// folder handler. Only existing directories qualify, so the program's own
/// executable path (argv[0], a file), installer flags, and other non-directory
/// arguments are ignored rather than navigating somewhere unintended.
fn raw_directory_arg(arg: &str) -> Option<String> {
    let path = std::path::Path::new(arg);
    if !path.is_dir() {
        return None;
    }
    path.to_str().map(str::to_owned)
}

/// Restores and focuses the main window — the single-instance UX expects
/// the running app to come forward when a second launch is rejected.
pub fn focus_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_open_directory, raw_directory_arg};

    fn encode(path: &str) -> String {
        url::form_urlencoded::byte_serialize(path.as_bytes()).collect()
    }

    #[test]
    fn rejects_foreign_schemes_and_garbage() {
        assert_eq!(parse_open_directory("https://example.com"), None);
        assert_eq!(parse_open_directory("not a url at all"), None);
        assert_eq!(parse_open_directory("dae://open"), None);
    }

    #[test]
    fn query_form_resolves_existing_directory() {
        let dir = std::env::temp_dir();
        let url = format!("dae://open?path={}", encode(dir.to_str().unwrap()));

        assert_eq!(parse_open_directory(&url), Some(dir.to_string_lossy().into_owned()));
    }

    #[test]
    fn path_form_resolves_existing_directory() {
        let dir = std::env::temp_dir();
        let url = format!("dae://open/{}", encode(dir.to_str().unwrap()));

        assert_eq!(parse_open_directory(&url), Some(dir.to_string_lossy().into_owned()));
    }

    #[test]
    fn file_target_resolves_to_parent() {
        let root = std::env::temp_dir().join(format!("dae-deep-link-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("create test root");
        let file = root.join("target.txt");
        std::fs::write(&file, b"x").expect("write test file");

        let url = format!("dae://open?path={}", encode(file.to_str().unwrap()));

        assert_eq!(parse_open_directory(&url), Some(root.to_string_lossy().into_owned()));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_target_is_rejected() {
        let url = "dae://open?path=%2Fno%2Fsuch%2Fdirectory%2Fanywhere";

        assert_eq!(parse_open_directory(url), None);
    }

    #[test]
    fn raw_directory_arg_accepts_existing_directory() {
        let dir = std::env::temp_dir();

        assert_eq!(
            raw_directory_arg(dir.to_str().unwrap()),
            Some(dir.to_string_lossy().into_owned())
        );
    }

    #[test]
    fn raw_directory_arg_rejects_file() {
        // Mirrors argv[0] on an "open with dae" launch: the exe is a file and
        // must be ignored rather than navigating to its containing folder.
        let root = std::env::temp_dir().join(format!("dae-raw-arg-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("create test root");
        let file = root.join("not-a-dir.txt");
        std::fs::write(&file, b"x").expect("write test file");

        assert_eq!(raw_directory_arg(file.to_str().unwrap()), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn raw_directory_arg_rejects_flags_and_garbage() {
        assert_eq!(raw_directory_arg("--flag"), None);
        assert_eq!(raw_directory_arg("dae://open"), None);
        assert_eq!(raw_directory_arg("/no/such/directory/anywhere"), None);
    }
}
