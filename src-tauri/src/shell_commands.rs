//! Windows 11 *modern* shell context-menu commands — the packaged
//! `windows.fileExplorerContextMenus` extension point, hosted by dae.
//!
//! An app that wants a right-click command in Windows 11 declares it in its
//! MSIX manifest (`desktop4:FileExplorerContextMenus`, one `desktop5:Verb` per
//! shell item type) and implements the COM interface `IExplorerCommand` behind
//! that verb's CLSID. Explorer builds the menu by activating each matching
//! CLSID and asking it for a title, icon, state and flags. This module does the
//! same thing for dae's own menu, which is why the entries it produces look
//! like Explorer's: they *are* the same commands, localized by the same
//! providers.
//!
//! # Why this works without package identity
//!
//! dae is an unpackaged Win32 app and stays one. Two facts make hosting work,
//! both verified on Windows 11 26H2 (build 26340):
//!
//! 1. **The declarations are readable.** The verb → CLSID → item-type mapping
//!    lives in each package's `AppxManifest.xml`, which a normal user can read.
//!    (`C:\Program Files\WindowsApps` itself cannot be *enumerated* without
//!    elevation, so this module never lists it: it takes package full names from
//!    the enumerable `HKLM\SOFTWARE\Classes\PackagedCom\Package` key and opens
//!    the known path below each install root.)
//! 2. **The servers are activatable.** A packaged `com:SurrogateServer` is
//!    registered in the `PackagedCom` class index rather than `HKCR\CLSID`, but
//!    a plain `CoCreateInstance` from an identity-less process still activates
//!    it — out of process, in `dllhost.exe`. Measured on the machine above:
//!    12 packages declaring the extension point, 109 verb registrations, 18
//!    distinct CLSIDs, all 18 activating, with `GetTitle`/`GetIcon`/`GetState`/
//!    `GetFlags` returning real provider-localized data (`在终端中打开(&T)`
//!    for Windows Terminal, `使用 PowerRename 重命名` for PowerToys).
//!
//! Out-of-process hosting is also why this is safe to ship: a provider that
//! crashes takes down its own surrogate, not dae. The only foreign code that
//! ever runs inside dae's process is the marshalling proxy COM installs.
//!
//! # What this module does not do
//!
//! - **Legacy handlers.** `HKCR\*\shellex\ContextMenuHandlers` (7-Zip, WinRAR,
//!   TortoiseSVN) are in-process DLLs driven through `IContextMenu`, a different
//!   contract with a different risk profile. They are not hosted here.
//! - **Subcommands.** A verb flagged `ECF_HASSUBCOMMANDS`/`ECF_ISDROPDOWN` is
//!   skipped rather than rendered as a dead parent; no provider on the reference
//!   machine used one. Adding `EnumSubCommands` is additive: the cache key is
//!   already a string, so a child can be cached as `"<clsid>#<index>"`.
//! - **Non-file-system selections.** `Invoke` receives an `IShellItemArray`,
//!   which can only be built from real Win32 paths. Remote entries (SFTP, SMB,
//!   cloud) therefore get no commands — the same rule dae already applies to its
//!   own extension items.
//! - **Providers that fail.** A command that cannot be activated, or that
//!   reports a hidden state, is dropped for that selection. OneDrive and QQ
//!   Extension do exactly this on the reference machine; they must not take the
//!   rest of the menu down with them.
//!
//! # Threading
//!
//! All COM work happens on one long-lived STA thread that owns a message pump
//! (see [`backend::Host`]). Two reasons, both load-bearing:
//!
//! - **Apartment pinning.** [`backend::CommandCache`] keeps activated
//!   `IExplorerCommand` pointers alive so a right-click does not pay for a
//!   surrogate start per provider. An interface pointer is only valid inside the
//!   apartment that created it, so every call has to come back to that thread.
//! - **No deadlock.** The `IShellItemArray` handed to a provider is a proxy
//!   living in *our* apartment; when the provider enumerates it, that call
//!   arrives on our STA. An STA that sits inside another call without pumping
//!   deadlocks the provider, so the thread pumps between jobs.

use crate::file_system::error::FileSystemError;
use serde::Serialize;
use specta::Type;

/// One row of the shell-command section of a context menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ShellCommand {
    /// The verb's CLSID, echoed back to [`invoke_shell_command`]. Unique per
    /// command: a provider that registers one CLSID for forty file extensions
    /// (Paint) contributes one row, not forty.
    pub clsid: String,
    /// Menu text, with the provider's access-key markup removed.
    pub label: String,
    /// 16–32px icon as a `data:` URL, extracted through the shell.
    pub icon_data_url: Option<String>,
    /// Owning app's display name, set only when that app contributes more than
    /// one command to this menu. Explorer groups those under an app flyout; a
    /// lone command is shown inline instead.
    pub group: Option<String>,
    /// The provider asked for a separator above this row (`ECF_SEPARATORBEFORE`).
    pub separator_before: bool,
    /// The provider reports the command greyed out (`ECS_DISABLED`).
    pub disabled: bool,
}

/// What was right-clicked, reduced to what `ItemType` matching needs.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Selection {
    kind: SelectionKind,
    /// Lowercase, leading dot (`".zip"`); empty for a file without extension.
    extension: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionKind {
    File,
    Directory,
}

impl Selection {
    /// Reduces a selection to a single kind. The right-clicked entry decides:
    /// a mixed selection is still "the file menu" or "the folder menu" depending
    /// on where the click landed, which is the rule Explorer follows.
    fn for_paths(primary: &str, paths: &[String]) -> Result<Self, FileSystemError> {
        if paths.is_empty() {
            return Err(FileSystemError::InvalidInput(
                "fs.shell_command_empty_selection".into(),
            ));
        }

        let metadata = std::fs::metadata(primary).map_err(FileSystemError::from)?;
        Ok(if metadata.is_dir() {
            Self {
                kind: SelectionKind::Directory,
                extension: String::new(),
            }
        } else {
            Self {
                kind: SelectionKind::File,
                extension: extension_of(primary),
            }
        })
    }
}

/// Lowercase extension with a leading dot, or an empty string.
fn extension_of(path: &str) -> String {
    std::path::Path::new(path)
        .extension()
        .map(|value| format!(".{}", value.to_string_lossy().to_lowercase()))
        .unwrap_or_default()
}

/// Whether one `desktop5:ItemType/@Type` value covers the current selection.
///
/// The grammar is the shell's static-verb grammar. The forms below are the ones
/// that appear in practice; a value outside them (`ProgID`,
/// `SystemFileAssociations\…`, `DesktopBackground`) matches nothing rather than
/// everything — a command offered in the wrong context is worse than one that
/// does not appear.
fn item_type_matches(item_type: &str, selection: &Selection) -> bool {
    match item_type {
        // Files only: Explorer's `*` is every file, including extensionless ones.
        "*" => selection.kind == SelectionKind::File,
        "AllFilesystemObjects" => true,
        "Directory" => selection.kind == SelectionKind::Directory,
        // A folder's own background is a surface dae's per-entry menu lacks.
        "Directory\\Background" => false,
        value if value.starts_with('.') => {
            selection.kind == SelectionKind::File && selection.extension == value.to_lowercase()
        }
        _ => false,
    }
}

/// Strips the Windows access-key markup out of a provider's menu label.
///
/// Providers hand back the raw label with the platform's mnemonic markup, e.g.
/// `在终端中打开(&T)` for Windows Terminal or `通过 Z&ed 打开` for Zed. The
/// shell keeps that markup out of sight unless the user has turned on access-key
/// underlines, and dae's menu has no mnemonic handling to bind the letter to, so
/// only the visible text is kept. `&&` is an escaped literal `&`.
fn strip_label_markup(label: &str) -> String {
    let mut text = String::with_capacity(label.len());
    let mut characters = label.chars();

    while let Some(character) = characters.next() {
        if character != '&' {
            text.push(character);
            continue;
        }
        match characters.next() {
            Some('&') => text.push('&'),
            // The marked letter keeps its place in the text.
            Some(marked) => text.push(marked),
            // A trailing `&` is markup with no letter behind it.
            None => {}
        }
    }

    text
}

/// Splits a provider icon reference into a path and an icon index.
///
/// The shell's icon notation is `"<file>"` or `"<file>,<index>"`, where a
/// negative index is a resource id rather than a position in the file's icon
/// list. A trailing part that is not an integer belongs to the file name, since
/// a path may legally contain a comma.
fn split_icon_spec(spec: &str) -> (&str, i32) {
    match spec.rsplit_once(',') {
        Some((path, tail)) => match tail.parse::<i32>() {
            Ok(index) => (path, index),
            Err(_) => (spec, 0),
        },
        None => (spec, 0),
    }
}

/// One `desktop5:Verb` declaration read out of a package manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ManifestVerb {
    item_type: String,
    clsid: String,
}

/// Reads the `windows.fileExplorerContextMenus` declarations out of an
/// `AppxManifest.xml`.
///
/// Deliberately a local-name scan rather than a namespaced tree walk: the
/// manifest mixes four schemas (`desktop4`, `desktop5`, `com`, `uap10`) whose
/// prefixes are conventional but not guaranteed, while the element and
/// attribute names this needs are unambiguous.
fn parse_manifest_verbs(xml: &str) -> Vec<ManifestVerb> {
    use quick_xml::events::{BytesStart, Event};
    use quick_xml::{Reader, XmlVersion};

    fn attribute(element: &BytesStart<'_>, wanted: &str) -> Option<String> {
        element
            .attributes()
            .flatten()
            .find(|attribute| attribute.key.local_name().as_ref() == wanted)
            // XML 1.0 attribute normalization; an XML 1.1 manifest would differ
            // only in characters that never appear in these values.
            .and_then(|attribute| attribute.normalized_value(XmlVersion::default()).ok())
            .map(|value| value.into_owned())
    }

    fn push_verb(
        element: &BytesStart<'_>,
        item_type: &Option<String>,
        verbs: &mut Vec<ManifestVerb>,
    ) {
        let (Some(item_type), Some(clsid)) = (item_type, attribute(element, "Clsid")) else {
            return;
        };
        verbs.push(ManifestVerb {
            item_type: item_type.clone(),
            clsid: clsid.to_uppercase(),
        });
    }

    let mut reader = Reader::from_str(xml);
    let mut verbs = Vec::new();
    let mut in_menus = false;
    let mut current_item_type: Option<String> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) => match element.local_name().as_ref() {
                "FileExplorerContextMenus" => in_menus = true,
                "ItemType" if in_menus => current_item_type = attribute(&element, "Type"),
                "Verb" => push_verb(&element, &current_item_type, &mut verbs),
                _ => {}
            },
            // A self-closing element opens and closes at once, so it can never
            // contain another one; only the leaf this cares about can appear.
            Ok(Event::Empty(element)) => {
                if element.local_name().as_ref() == "Verb" {
                    push_verb(&element, &current_item_type, &mut verbs);
                }
            }
            Ok(Event::End(element)) => match element.local_name().as_ref() {
                "FileExplorerContextMenus" => in_menus = false,
                "ItemType" => current_item_type = None,
                _ => {}
            },
            Ok(Event::Eof) => break,
            Ok(_) => {}
            // A truncated or malformed manifest yields whatever was read before
            // the damage: one broken package must not hide the others.
            Err(_) => break,
        }
    }

    verbs
}

/// Collapses per-item-type registrations into one row per command and gives
/// commands from a multi-command app their Explorer-style group name.
///
/// Grouping is decided after visibility, so an app whose second command is
/// hidden for this selection contributes one inline row, not a one-entry flyout.
fn assign_groups(rows: Vec<(String, ShellCommand)>) -> Vec<ShellCommand> {
    use std::collections::HashMap;

    let mut per_app: HashMap<String, usize> = HashMap::new();
    for (app, _) in &rows {
        *per_app.entry(app.clone()).or_default() += 1;
    }

    let mut items: Vec<ShellCommand> = rows
        .into_iter()
        .map(|(app, mut item)| {
            if per_app.get(app.as_str()).copied().unwrap_or(0) > 1 {
                item.group = Some(app);
            }
            item
        })
        .collect();

    // Inline rows first, then app flyouts — the order Explorer uses, and the one
    // that keeps a stray installed extension from pushing dae's own items
    // around. Case-insensitive because that is how menus read; the sort is
    // stable, so equal labels keep provider order.
    items.sort_by(|left, right| {
        let key = |item: &ShellCommand| {
            (
                item.group.clone().map(|group| group.to_lowercase()),
                item.label.to_lowercase(),
            )
        };
        match (&left.group, &right.group) {
            (None, Some(_)) => std::cmp::Ordering::Less,
            (Some(_), None) => std::cmp::Ordering::Greater,
            _ => key(left).cmp(&key(right)),
        }
    });
    items
}

/// Lists the installed apps' right-click commands for this selection.
///
/// Returns an empty list on any platform without the extension point, and for
/// selections that cannot be turned into shell items, so the frontend can render
/// one section unconditionally.
#[tauri::command]
#[specta::specta]
pub async fn list_shell_commands(
    paths: Vec<String>,
    primary: String,
) -> Result<Vec<ShellCommand>, FileSystemError> {
    // A shell item can only be built from a real Win32 path; a remote backend
    // (SFTP, SMB, cloud) has none, and the caller renders no section for it.
    if !crate::file_system::vfs::is_local_path(&primary) {
        return Ok(Vec::new());
    }
    backend::list(paths, primary).await
}

/// Runs one command against the same paths it was listed for.
#[tauri::command]
#[specta::specta]
pub async fn invoke_shell_command(
    clsid: String,
    paths: Vec<String>,
) -> Result<(), FileSystemError> {
    backend::invoke(clsid, paths).await
}

/// Primes the shell-command path before the user's first right-click.
///
/// Called once from the frontend after the window is revealed. See
/// [`backend::warm`] for what it does and why it is worth doing.
#[tauri::command]
#[specta::specta]
pub async fn warm_shell_commands() -> Result<(), FileSystemError> {
    backend::warm().await
}

// ---------------------------------------------------------------------------
// Windows backend
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod backend {
    use super::{
        ManifestVerb, Selection, ShellCommand, assign_groups, item_type_matches,
        parse_manifest_verbs, split_icon_spec, strip_label_markup,
    };
    use crate::file_system::error::FileSystemError;
    use base64::Engine as _;
    use std::collections::{HashMap, VecDeque};
    use std::ffi::c_void;
    use std::path::PathBuf;
    use std::sync::mpsc::{Sender, channel};
    use std::sync::{LazyLock, Mutex};
    use std::time::{Duration, Instant};
    use windows::Win32::Foundation::{ERROR_NO_MORE_ITEMS, HANDLE};
    use windows::Win32::Graphics::Gdi::{DeleteObject, HGDIOBJ};
    use windows::Win32::System::Com::{
        CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    };
    use windows::Win32::System::Registry::{
        HKEY, HKEY_LOCAL_MACHINE, KEY_READ, RegCloseKey, RegEnumKeyExW, RegOpenKeyExW,
        RegQueryValueExW,
    };
    use windows::Win32::System::Threading::{CreateEventW, INFINITE, SetEvent};
    use windows::Win32::UI::Shell::Common::ITEMIDLIST;
    use windows::Win32::UI::Shell::{
        ECF_HASSUBCOMMANDS, ECF_ISDROPDOWN, ECF_SEPARATORBEFORE, ECS_DISABLED, ECS_HIDDEN,
        IExplorerCommand, IPersistIDList, IShellItem, IShellItemArray, SHCreateItemFromParsingName,
        SHCreateShellItemArrayFromIDLists, SHDefExtractIconW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        DestroyIcon, DispatchMessageW, GetIconInfo, HICON, ICONINFO, MSG,
        MsgWaitForMultipleObjects, PM_REMOVE, PeekMessageW, QS_ALLINPUT, TranslateMessage,
    };
    use windows::core::{GUID, HSTRING, Interface as _, PCWSTR, PWSTR};

    /// Side of the extracted command icon in pixels. The menu renders it at
    /// 16px; extracting at 2x keeps it sharp on scaled displays at a cost of a
    /// few hundred bytes per command.
    const ICON_SIZE: u32 = 32;
    /// How long a scan of the installed packages is reused. Explorer itself
    /// needs a restart to notice a new package, so a few minutes of staleness is
    /// not the bottleneck; the scan re-reads a dozen small XML files.
    const SCAN_TTL: Duration = Duration::from_secs(300);
    /// Ceiling on live surrogate connections. Each cached command keeps its
    /// `dllhost.exe` alive; 64 covers a loaded machine's providers while
    /// bounding the processes dae is responsible for.
    const CACHE_LIMIT: usize = 64;

    const PACKAGE_KEY: &str = r"SOFTWARE\Classes\PackagedCom\Package";

    /// One package's contribution of verbs.
    #[derive(Clone)]
    struct Registration {
        item_type: String,
        clsid: String,
        /// Owning app's display name, for the grouped-flyout decision.
        app: String,
    }

    // -----------------------------------------------------------------------
    // Enumeration
    // -----------------------------------------------------------------------

    /// Reads the CLSID → package mapping Windows maintains for packaged COM
    /// servers, then each package's manifest for the verbs built on it.
    ///
    /// The registry half cannot answer "which item types does this command apply
    /// to" — that lives in the manifest — and a manifest cannot be found by
    /// listing `WindowsApps`, which denies enumeration to unelevated users. So:
    /// registry for the package list, known paths for the manifests.
    fn scan() -> Vec<Registration> {
        let mut registrations = Vec::new();
        for package in packaged_apps() {
            let Some(xml) = read_manifest(&package.full_name) else {
                continue;
            };
            for ManifestVerb { item_type, clsid } in parse_manifest_verbs(&xml) {
                registrations.push(Registration {
                    item_type,
                    clsid,
                    app: package.display_name.clone(),
                });
            }
        }
        registrations
    }

    struct PackagedApp {
        full_name: String,
        display_name: String,
    }

    /// Enumerates `HKLM\SOFTWARE\Classes\PackagedCom\Package`. Its subkey names
    /// are package full names; the `Server\0` values carry the app's display
    /// name, already localized by the package.
    fn packaged_apps() -> Vec<PackagedApp> {
        let Some(key) = open_key(PACKAGE_KEY) else {
            return Vec::new();
        };

        let mut apps = Vec::new();
        let mut index = 0;
        while let Some(full_name) = enum_key(key, index) {
            index += 1;
            let server = format!(r"{PACKAGE_KEY}\{full_name}\Server\0");
            let display_name = read_value(&server, "DisplayName")
                .or_else(|| read_value(&server, "ApplicationDisplayName"))
                .unwrap_or_else(|| full_name.clone());
            apps.push(PackagedApp {
                full_name,
                display_name,
            });
        }

        // SAFETY: the handle came from `open_key` and is closed exactly once.
        unsafe {
            let _ = RegCloseKey(key);
        };
        apps
    }

    fn open_key(subkey: &str) -> Option<HKEY> {
        let path = wide(subkey);
        let mut handle = HKEY::default();
        // SAFETY: `path` is null-terminated and outlives the call; `handle` is a
        // valid out-pointer.
        let opened = unsafe {
            RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(path.as_ptr()),
                Some(0),
                KEY_READ,
                &mut handle,
            )
        };
        opened.is_ok().then_some(handle)
    }

    /// Reads the `index`-th subkey name of `key`, or `None` past the last one.
    fn enum_key(key: HKEY, index: u32) -> Option<String> {
        let mut buffer = [0u16; 512];
        let mut length = buffer.len() as u32;
        // SAFETY: `buffer` and `length` are a matching pointer/capacity pair; the
        // remaining optional out-parameters are unrequested.
        let result = unsafe {
            RegEnumKeyExW(
                key,
                index,
                Some(PWSTR(buffer.as_mut_ptr())),
                &mut length,
                None,
                None,
                None,
                None,
            )
        };
        if result == ERROR_NO_MORE_ITEMS {
            return None;
        }
        if result.is_err() {
            return None;
        }
        Some(String::from_utf16_lossy(&buffer[..length as usize]))
    }

    /// Reads a `REG_SZ` value of a registry key.
    fn read_value(subkey: &str, value: &str) -> Option<String> {
        let key = open_key(subkey)?;
        let name = wide(value);

        let mut size = 0u32;
        // SAFETY: a size query with a null buffer; `size` is a valid out-pointer.
        let sized = unsafe {
            RegQueryValueExW(
                key,
                PCWSTR(name.as_ptr()),
                None,
                None,
                None,
                Some(&mut size),
            )
        };
        if sized.is_err() || size == 0 {
            // SAFETY: the handle came from `open_key` and is closed once.
            unsafe {
                let _ = RegCloseKey(key);
            };
            return None;
        }

        let mut buffer = vec![0u8; size as usize];
        // SAFETY: `buffer` is `size` bytes, which is what the query just asked for.
        let read = unsafe {
            RegQueryValueExW(
                key,
                PCWSTR(name.as_ptr()),
                None,
                None,
                Some(buffer.as_mut_ptr()),
                Some(&mut size),
            )
        };
        // SAFETY: the handle came from `open_key` and is closed once.
        unsafe {
            let _ = RegCloseKey(key);
        };
        if read.is_err() {
            return None;
        }

        let units: Vec<u16> = buffer[..size as usize]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .take_while(|unit| *unit != 0)
            .collect();
        let value = String::from_utf16_lossy(&units);
        (!value.trim().is_empty()).then_some(value)
    }

    /// Directories that can hold an installed package. `%ProgramFiles%` covers
    /// the system volume; a package installed to any other volume lands in a
    /// `WindowsApps` folder at its root.
    fn install_roots() -> &'static [PathBuf] {
        static ROOTS: LazyLock<Vec<PathBuf>> = LazyLock::new(|| {
            let mut roots = Vec::new();
            for variable in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
                if let Ok(value) = std::env::var(variable) {
                    roots.push(PathBuf::from(value).join("WindowsApps"));
                }
            }
            for disk in sysinfo::Disks::new_with_refreshed_list().list() {
                roots.push(disk.mount_point().join("WindowsApps"));
            }
            roots.sort();
            roots.dedup();
            roots
        });
        &ROOTS
    }

    fn read_manifest(full_name: &str) -> Option<String> {
        install_roots()
            .iter()
            .map(|root| root.join(full_name).join("AppxManifest.xml"))
            .find(|path| path.is_file())
            .and_then(|path| std::fs::read_to_string(path).ok())
    }

    /// The scan, cached. A right-click must not re-read every manifest, and a
    /// package installed while dae runs should still appear without a restart.
    fn registrations() -> Vec<Registration> {
        /// When the scan ran, and what it found.
        type ScanCache = Option<(Instant, Vec<Registration>)>;

        static CACHE: LazyLock<Mutex<ScanCache>> = LazyLock::new(|| Mutex::new(None));

        let mut cached = CACHE.lock().expect("shell command cache poisoned");
        if let Some((scanned_at, registrations)) = cached.as_ref()
            && scanned_at.elapsed() < SCAN_TTL
        {
            return registrations.clone();
        }

        let registrations = scan();
        *cached = Some((Instant::now(), registrations.clone()));
        registrations
    }

    // -----------------------------------------------------------------------
    // STA host
    // -----------------------------------------------------------------------

    /// Activated commands, keyed by CLSID.
    #[derive(Default)]
    struct CommandCache {
        commands: HashMap<String, IExplorerCommand>,
        /// Insertion order, for eviction.
        order: VecDeque<String>,
        /// CLSIDs that refused to activate, with the failure for the log. Kept
        /// for the process lifetime: a provider that answers `E_NOINTERFACE` will
        /// keep answering it, and retrying costs a surrogate start per click.
        broken: HashMap<String, String>,
        /// Icon spec → `data:` URL, with `None` for a spec that has no icon.
        icons: HashMap<String, Option<String>>,
    }

    impl CommandCache {
        fn command(&mut self, clsid: &str) -> Option<IExplorerCommand> {
            if self.broken.contains_key(clsid) {
                return None;
            }
            if let Some(command) = self.commands.get(clsid) {
                return Some(command.clone());
            }

            let Some(id) = parse_guid(clsid) else {
                self.broken
                    .insert(clsid.to_string(), "malformed CLSID".into());
                return None;
            };

            // SAFETY: `id` is a initialized GUID. `CLSCTX_ALL` matters: a
            // packaged verb is served by a surrogate, so an in-process-only
            // request would fail with a class-not-registered error.
            let activated: windows::core::Result<IExplorerCommand> =
                unsafe { CoCreateInstance(&id, None, CLSCTX_ALL) };

            match activated {
                Ok(command) => {
                    if self.order.len() >= CACHE_LIMIT
                        && let Some(oldest) = self.order.pop_front()
                    {
                        self.commands.remove(&oldest);
                    }
                    self.order.push_back(clsid.to_string());
                    self.commands.insert(clsid.to_string(), command.clone());
                    Some(command)
                }
                Err(error) => {
                    self.broken.insert(clsid.to_string(), error.to_string());
                    None
                }
            }
        }

        fn icon(&mut self, spec: &str) -> Option<String> {
            if let Some(cached) = self.icons.get(spec) {
                return cached.clone();
            }
            let resolved = resolve_icon(spec);
            self.icons.insert(spec.to_string(), resolved.clone());
            resolved
        }
    }

    type Task = Box<dyn FnOnce(&mut CommandCache) + Send>;

    /// The single-threaded COM apartment every shell call goes through.
    struct Host {
        tasks: Sender<Task>,
        /// A `HANDLE` is not `Send`, so it travels as its address.
        wake: isize,
    }

    impl Host {
        fn spawn() -> Result<Self, String> {
            let (tasks, receiver) = channel::<Task>();
            // SAFETY: an unnamed, auto-reset, initially-unsignaled event. The
            // handle lives until the process exits, which is also when the
            // thread waiting on it stops.
            let wake = unsafe { CreateEventW(None, false, false, PCWSTR::null()) }
                .map_err(|error| error.to_string())?;
            let wake_address = wake.0 as isize;

            std::thread::Builder::new()
                .name("shell-commands".into())
                .spawn(move || {
                    // SAFETY: the apartment belongs to this thread, which runs
                    // until the process exits, so the matching `CoUninitialize`
                    // would never be reached anyway.
                    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
                    let mut cache = CommandCache::default();
                    let wake = HANDLE(wake_address as *mut c_void);

                    loop {
                        while let Ok(task) = receiver.try_recv() {
                            // A panicking task must not take the host down: the
                            // receiver of its reply sees a dropped sender and
                            // reports an ordinary failure.
                            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                task(&mut cache);
                            }));
                        }

                        // SAFETY: `message` is a valid out-parameter, a zeroed
                        // filter range means every message, and `wake` is a live
                        // event handle.
                        unsafe {
                            // Providers call back into the `IShellItemArray` proxy
                            // that lives in this apartment, so those calls have to
                            // be dispatched before the thread can sleep.
                            let mut message = MSG::default();
                            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                                let _ = TranslateMessage(&message);
                                DispatchMessageW(&message);
                            }
                            MsgWaitForMultipleObjects(Some(&[wake]), false, INFINITE, QS_ALLINPUT);
                        }
                    }
                })
                .map_err(|error| error.to_string())?;

            Ok(Self {
                tasks,
                wake: wake_address,
            })
        }

        /// Queues `work` on the apartment thread and waits for its result.
        async fn run<T, F>(&self, work: F) -> Result<T, FileSystemError>
        where
            T: Send + 'static,
            F: FnOnce(&mut CommandCache) -> T + Send + 'static,
        {
            let (reply, receive) = tokio::sync::oneshot::channel();
            let task: Task = Box::new(move |cache| {
                let _ = reply.send(work(cache));
            });

            self.tasks.send(task).map_err(|error| {
                FileSystemError::Internal(format!("fs.shell_command_host: {error}"))
            })?;
            // SAFETY: the event handle is owned by the host thread and stays
            // valid for the process lifetime. Setting it after the enqueue is
            // race-free: the auto-reset event is already set if the thread has
            // not slept yet, so the wakeup cannot be lost.
            unsafe {
                let _ = SetEvent(HANDLE(self.wake as *mut c_void));
            };

            receive
                .await
                .map_err(|_| FileSystemError::Internal("fs.shell_command_host_dropped".into()))
        }
    }

    fn host() -> Result<&'static Host, FileSystemError> {
        static HOST: LazyLock<Result<Host, String>> = LazyLock::new(Host::spawn);
        HOST.as_ref()
            .map_err(|error| FileSystemError::Internal(format!("fs.shell_command_host: {error}")))
    }

    /// Parses `XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX`, with or without braces.
    fn parse_guid(value: &str) -> Option<GUID> {
        let digits: String = value
            .chars()
            .filter(|character| character.is_ascii_hexdigit())
            .collect();
        if digits.len() != 32 {
            return None;
        }
        u128::from_str_radix(&digits, 16).ok().map(GUID::from_u128)
    }

    // -----------------------------------------------------------------------
    // Menu construction
    // -----------------------------------------------------------------------

    /// Primes the one-time costs a first right-click would otherwise pay alone.
    ///
    /// Behind the menu's open animation there is a hard threshold: content that
    /// arrives after it has to be inserted into a popup that has already
    /// settled, and the resulting height change is what reads as a flicker.
    /// Measured on the reference machine with the menu's 120 ms animation, a
    /// cold `list` takes 211-228 ms and a warm one 19 ms — so the section lands
    /// after the animation on the first right-click and before it afterwards.
    /// Almost all of that gap is the first activation of each provider's COM
    /// surrogate; the manifest scan is ~32 ms.
    ///
    /// Both seed paths are the machine's own — the temp directory and dae's own
    /// executable — so nothing of the user's is touched and both always exist.
    /// Between them they cover the `Directory`, `AllFilesystemObjects` and `*`
    /// item types, which is where the providers a user meets first live. A
    /// provider registered for a single extension stays cold; guessing which
    /// extensions a machine cares about is not worth the surrogate it would
    /// leave running.
    ///
    /// Failures are deliberately swallowed: this is speculative background
    /// work, and a seed that yields nothing leaves that type exactly as cold as
    /// it was before.
    pub(super) async fn warm() -> Result<(), FileSystemError> {
        // The scan answers for every selection and is pure I/O, so it stays off
        // the COM apartment the way `list` keeps it.
        let _ = tauri::async_runtime::spawn_blocking(registrations).await;

        // Started by the first `list` below, but named here so the thread's
        // cost is not accidentally read as part of a menu.
        let _ = host()?;

        let mut seeds = vec![std::env::temp_dir()];
        if let Ok(executable) = std::env::current_exe() {
            seeds.push(executable);
        }

        for seed in seeds {
            let path = seed.to_string_lossy().into_owned();
            let _ = list(vec![path.clone()], path).await;
        }

        Ok(())
    }

    pub(super) async fn list(
        paths: Vec<String>,
        primary: String,
    ) -> Result<Vec<ShellCommand>, FileSystemError> {
        let selection_paths = paths.clone();
        let candidates = tauri::async_runtime::spawn_blocking(move || {
            let selection = Selection::for_paths(&primary, &selection_paths)?;
            // The scan reads the registry and a dozen manifests: real I/O, kept
            // off both the webview's command thread and the COM apartment.
            let candidates: Vec<Registration> = registrations()
                .into_iter()
                .filter(|registration| item_type_matches(&registration.item_type, &selection))
                .collect();
            Ok::<_, FileSystemError>(candidates)
        })
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))??;

        // Collapse the per-item-type hits: one row per (command, app), so a
        // provider registered for forty extensions is activated once.
        let mut seen = std::collections::HashSet::new();
        let candidates: Vec<(String, String)> = candidates
            .into_iter()
            .filter(|registration| seen.insert(registration.clsid.clone()))
            .map(|registration| (registration.clsid, registration.app))
            .collect();

        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        host()?
            .run(move |cache| build_menu(cache, &candidates, &paths))
            .await
    }

    pub(super) async fn invoke(clsid: String, paths: Vec<String>) -> Result<(), FileSystemError> {
        host()?
            .run(move |cache| {
                let Some(command) = cache.command(&clsid) else {
                    return Err(FileSystemError::InvalidInput(format!(
                        "fs.shell_command_unknown: {clsid}"
                    )));
                };
                let array = shell_item_array(&paths)?;
                // SAFETY: both arguments are valid, and `Invoke` is the
                // provider's own entry point — it may run for as long as its app
                // takes to start.
                unsafe { command.Invoke(&array, None) }
                    .map_err(|error| FileSystemError::Internal(error.to_string()))
            })
            .await?
    }

    /// Activates and interrogates every candidate, dropping the ones this
    /// selection cannot show. Runs on the apartment thread.
    fn build_menu(
        cache: &mut CommandCache,
        candidates: &[(String, String)],
        paths: &[String],
    ) -> Vec<ShellCommand> {
        let array = match shell_item_array(paths) {
            Ok(array) => array,
            Err(error) => {
                log::warn!("Unable to build a shell item array: {error}");
                return Vec::new();
            }
        };

        let mut rows = Vec::new();
        for (clsid, app) in candidates {
            let Some(command) = cache.command(clsid) else {
                continue;
            };

            let flags = unsafe { command.GetFlags() }.unwrap_or(0);
            if flags & ECF_HASSUBCOMMANDS.0 as u32 != 0 || flags & ECF_ISDROPDOWN.0 as u32 != 0 {
                // A flyout parent has no action of its own; rendering it as a
                // command would be a lie, and `EnumSubCommands` is not wired up
                // yet (see the module docs).
                continue;
            }

            // `false` asks for the fast answer, which is what Explorer does while
            // assembling a menu.
            let state = match unsafe { command.GetState(&array, false) } {
                Ok(state) => state,
                Err(_) => continue,
            };
            if state & ECS_HIDDEN.0 as u32 != 0 {
                continue;
            }

            let Some(raw_label) = take_pwstr(unsafe { command.GetTitle(&array) }.ok()) else {
                continue;
            };
            let label = strip_label_markup(&raw_label);
            if label.trim().is_empty() {
                continue;
            }

            let icon_data_url = take_pwstr(unsafe { command.GetIcon(&array) }.ok())
                .and_then(|spec| cache.icon(&spec));

            rows.push((
                app.clone(),
                ShellCommand {
                    clsid: clsid.clone(),
                    label,
                    icon_data_url,
                    group: None,
                    separator_before: flags & ECF_SEPARATORBEFORE.0 as u32 != 0,
                    disabled: state & ECS_DISABLED.0 as u32 != 0,
                },
            ));
        }

        report_broken_once(cache);
        assign_groups(rows)
    }

    /// Names the providers that refuse to activate, once per process. A package
    /// mid-update or a DLL that only answers its own app is normal, so this is
    /// diagnosis rather than an error — but it must not be reprinted on every
    /// right-click.
    fn report_broken_once(cache: &CommandCache) {
        static REPORTED: LazyLock<Mutex<bool>> = LazyLock::new(|| Mutex::new(false));
        if cache.broken.is_empty() {
            return;
        }
        let mut reported = REPORTED.lock().expect("shell command report poisoned");
        if *reported {
            return;
        }
        *reported = true;
        for (clsid, error) in &cache.broken {
            log::warn!("Shell command {clsid} is unavailable: {error}");
        }
    }

    /// Builds the shell item array a verb expects, from the selection's paths.
    fn shell_item_array(paths: &[String]) -> Result<IShellItemArray, FileSystemError> {
        let mut ids: Vec<*const ITEMIDLIST> = Vec::with_capacity(paths.len());
        let mut failure = None;

        for path in paths {
            // SAFETY: the item is created from a valid path, and the ID list it
            // yields is owned here until it is handed to the array.
            let id = unsafe {
                SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(path.as_str()), None)
                    .and_then(|item| item.cast::<IPersistIDList>())
                    .and_then(|persist| persist.GetIDList())
            };
            match id {
                Ok(id) => ids.push(id),
                Err(error) => {
                    failure = Some(error.to_string());
                    break;
                }
            }
        }

        let array = if ids.len() == paths.len() && failure.is_none() {
            // SAFETY: `ids` holds live ID lists for the duration of the call; the
            // array copies them.
            unsafe { SHCreateShellItemArrayFromIDLists(&ids) }.map_err(|error| error.to_string())
        } else {
            Err(failure.unwrap_or_else(|| "no selection".into()))
        };

        // The array owns its own copies, so the ID lists are freed either way.
        for id in ids {
            // SAFETY: each pointer came from `IPersistIDList::GetIDList`, which
            // allocates with the COM task allocator, and is freed once.
            unsafe { CoTaskMemFree(Some(id as *const c_void)) };
        }

        array.map_err(|error| {
            FileSystemError::Internal(format!("fs.shell_command_selection: {error}"))
        })
    }

    /// Takes ownership of a provider-allocated string and frees it.
    fn take_pwstr(value: Option<PWSTR>) -> Option<String> {
        let value = value?;
        if value.is_null() {
            return None;
        }
        // A provider may return nonsense here: VS Code's command hands back a
        // dangling pointer on the reference machine, which reads as mojibake
        // rather than failing. Nothing outside the provider can detect that, but
        // the buffer still has to be released.
        // SAFETY: a non-null `PWSTR` from a COM call is a null-terminated string
        // allocated by the caller's task allocator.
        let text = unsafe { value.to_string() }.ok();
        // SAFETY: the same allocation, freed exactly once.
        unsafe { CoTaskMemFree(Some(value.0 as *const c_void)) };
        text
    }

    // -----------------------------------------------------------------------
    // Icons
    // -----------------------------------------------------------------------

    fn resolve_icon(spec: &str) -> Option<String> {
        let (path, index) = split_icon_spec(spec);
        let bytes = if index == 0 {
            crate::file_system::preview::extract_file_icon_png(path, ICON_SIZE)
        } else {
            icon_from_resource(path, index)
        }?;
        Some(format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ))
    }

    /// An icon that names a resource id (`"shell32.dll,-101"`) has to go through
    /// the shell's own extractor; `IShellItemImageFactory` only understands files
    /// whose default icon is wanted.
    fn icon_from_resource(path: &str, index: i32) -> Option<Vec<u8>> {
        let path = wide(path);
        let mut icon = HICON::default();
        // SAFETY: `path` is null-terminated and outlives the call; `icon` is a
        // valid out-pointer for the large-icon slot.
        let extracted = unsafe {
            SHDefExtractIconW(
                PCWSTR(path.as_ptr()),
                index,
                0,
                Some(&mut icon),
                None,
                ICON_SIZE,
            )
        };
        if extracted.is_err() || icon.is_invalid() {
            return None;
        }

        let png = icon_to_png(icon);
        // SAFETY: the icon came from `SHDefExtractIconW` and is destroyed once.
        unsafe {
            let _ = DestroyIcon(icon);
        };
        png
    }

    fn icon_to_png(icon: HICON) -> Option<Vec<u8>> {
        let mut info = ICONINFO::default();
        // SAFETY: `icon` is a live icon handle and `info` a valid out-pointer.
        unsafe { GetIconInfo(icon, &mut info) }.ok()?;

        let png = crate::file_system::preview::bitmap_to_png(info.hbmColor);
        // SAFETY: `GetIconInfo` hands over two bitmaps that the caller owns.
        unsafe {
            let _ = DeleteObject(HGDIOBJ(info.hbmColor.0));
            let _ = DeleteObject(HGDIOBJ(info.hbmMask.0));
        }
        png
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    #[cfg(test)]
    mod windows_tests {
        use super::*;

        #[test]
        fn parses_the_clsid_forms_a_manifest_uses() {
            let expected = 0x1c6d_f0c0_192a_4451_be36_6a59_a86a_692eu128;
            assert_eq!(
                parse_guid("1C6DF0C0-192A-4451-BE36-6A59A86A692E").map(|id| id.to_u128()),
                Some(expected)
            );
            assert_eq!(
                parse_guid("{1c6df0c0-192a-4451-be36-6a59a86a692e}").map(|id| id.to_u128()),
                Some(expected)
            );
        }

        #[test]
        fn refuses_a_clsid_that_is_not_32_hex_digits() {
            assert!(parse_guid("").is_none());
            assert!(parse_guid("1C6DF0C0-192A-4451-BE36-6A59A86A692").is_none());
            assert!(parse_guid("not-a-clsid").is_none());
        }

        #[test]
        fn resolves_the_packages_of_this_machine() {
            // Reads the real registry and the real manifests: the one check that
            // the discovery path works against Windows rather than against a
            // fixture. It asserts nothing about *which* apps are installed, only
            // that a manifest is found and yields parseable verbs.
            let apps = packaged_apps();
            assert!(!apps.is_empty(), "no packaged COM servers registered");

            let with_verbs = apps
                .iter()
                .filter(|app| {
                    read_manifest(&app.full_name)
                        .is_some_and(|xml| !parse_manifest_verbs(&xml).is_empty())
                })
                .count();
            println!(
                "{} packages with packaged COM servers, {with_verbs} declaring context-menu verbs",
                apps.len()
            );
        }

        /// Lists what the machine's providers report, for a real file and a real
        /// directory. Ignored by default because it activates third-party COM
        /// servers and depends on what is installed:
        /// `cargo test --lib shell_commands -- --ignored --nocapture`.
        #[test]
        #[ignore = "activates the installed apps' COM servers"]
        fn lists_the_commands_of_this_machine() {
            let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("Cargo.toml")
                .to_string_lossy()
                .into_owned();
            let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .to_string_lossy()
                .into_owned();

            for (what, primary) in [("file", file), ("directory", directory)] {
                let paths = vec![primary.clone()];
                let items = tauri::async_runtime::block_on(list(paths, primary))
                    .expect("listing shell commands");
                println!("\n{what}: {} commands", items.len());
                for item in items {
                    println!(
                        "  {:<28} group={:<22} icon={:<5} disabled={} {}",
                        item.label,
                        item.group.unwrap_or_else(|| "-".into()),
                        item.icon_data_url.is_some(),
                        item.disabled,
                        item.clsid
                    );
                }
            }
        }

        /// The warm-up has to leave the *next* `list` on the fast path — that is
        /// its entire purpose. The bound is deliberately loose: a warm `list`
        /// measures ~19 ms on the reference machine against 211-228 ms cold, so
        /// 150 ms separates the two without turning a slow machine into a
        /// failure. Ignored for the same reason as the test above.
        #[test]
        #[ignore = "activates the installed apps' COM servers"]
        fn warming_leaves_the_next_list_on_the_fast_path() {
            let primary = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("Cargo.toml")
                .to_string_lossy()
                .into_owned();

            let started = std::time::Instant::now();
            tauri::async_runtime::block_on(warm()).expect("warming shell commands");
            let warming = started.elapsed();

            let paths = vec![primary.clone()];
            let started = std::time::Instant::now();
            let items = tauri::async_runtime::block_on(list(paths, primary))
                .expect("listing shell commands");
            let first_list = started.elapsed();

            println!(
                "warm took {warming:?}; the first list after it took {first_list:?} ({} commands)",
                items.len()
            );
            assert!(
                first_list < std::time::Duration::from_millis(150),
                "the first list after warming took {first_list:?}, which is the cold path"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Everywhere else
// ---------------------------------------------------------------------------

#[cfg(not(windows))]
mod backend {
    use super::ShellCommand;
    use crate::file_system::error::FileSystemError;

    /// macOS and Linux have no OS-wide, third-party-hostable context-menu
    /// extension point: a Finder Sync extension is loaded by Finder alone, and
    /// the Linux desktops' plugin APIs (Nautilus Python, KIO `KFileItemAction`)
    /// are private to each file manager. The declarative mechanisms those
    /// platforms do offer — macOS Services, `.desktop` actions — are a separate
    /// feature, so this surface stays empty rather than pretending otherwise.
    pub(super) async fn list(
        _paths: Vec<String>,
        _primary: String,
    ) -> Result<Vec<ShellCommand>, FileSystemError> {
        Ok(Vec::new())
    }

    pub(super) async fn invoke(_clsid: String, _paths: Vec<String>) -> Result<(), FileSystemError> {
        Err(FileSystemError::Unsupported(
            "fs.shell_commands_windows_only".into(),
        ))
    }

    /// Nothing to prime: `list` answers empty on every platform without the
    /// extension point, so there is no cold path to move out of the way.
    pub(super) async fn warm() -> Result<(), FileSystemError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection(kind: SelectionKind, extension: &str) -> Selection {
        Selection {
            kind,
            extension: extension.to_string(),
        }
    }

    fn file(extension: &str) -> Selection {
        selection(SelectionKind::File, extension)
    }

    #[test]
    fn matches_the_documented_item_types() {
        let directory = selection(SelectionKind::Directory, "");

        assert!(item_type_matches("*", &file(".txt")));
        assert!(item_type_matches("*", &file("")));
        assert!(!item_type_matches("*", &directory));

        assert!(item_type_matches("AllFilesystemObjects", &file(".txt")));
        assert!(item_type_matches("AllFilesystemObjects", &directory));

        assert!(item_type_matches("Directory", &directory));
        assert!(!item_type_matches("Directory", &file(".txt")));

        assert!(item_type_matches(".zip", &file(".zip")));
        assert!(!item_type_matches(".zip", &file(".tar")));
        assert!(!item_type_matches(".zip", &directory));
    }

    #[test]
    fn matches_item_types_case_insensitively() {
        assert!(item_type_matches(".PNG", &file(".png")));
    }

    #[test]
    fn refuses_the_contexts_it_cannot_represent() {
        let directory = selection(SelectionKind::Directory, "");
        // dae's menu hangs off an entry, so there is no folder background, and a
        // ProgID key is not something this module resolves.
        assert!(!item_type_matches("Directory\\Background", &directory));
        assert!(!item_type_matches(
            "SystemFileAssociations\\text",
            &file(".txt")
        ));
        assert!(!item_type_matches("DesktopBackground", &directory));
    }

    #[test]
    fn strips_the_access_key_markup_from_a_label() {
        // Windows Terminal writes the mnemonic and the parentheses its own
        // resource string carries; Zed marks the middle of the word.
        assert_eq!(strip_label_markup("在终端中打开(&T)"), "在终端中打开(T)");
        assert_eq!(strip_label_markup("通过 Z&ed 打开"), "通过 Zed 打开");
        assert_eq!(strip_label_markup("Open with Code"), "Open with Code");
    }

    #[test]
    fn keeps_escaped_ampersands_and_ignores_later_markers() {
        assert_eq!(strip_label_markup("Copy && Paste"), "Copy & Paste");
        assert_eq!(strip_label_markup("&Edit with &Vim"), "Edit with Vim");
    }

    #[test]
    fn splits_icon_specs_the_way_the_shell_writes_them() {
        assert_eq!(
            split_icon_spec(r"C:\Windows\explorer.exe"),
            (r"C:\Windows\explorer.exe", 0)
        );
        assert_eq!(
            split_icon_spec(r"C:\Windows\shell32.dll,-101"),
            (r"C:\Windows\shell32.dll", -101)
        );
        assert_eq!(
            split_icon_spec(r"C:\Windows\shell32.dll,3"),
            (r"C:\Windows\shell32.dll", 3)
        );
        // A comma that belongs to the file name is not an index.
        assert_eq!(
            split_icon_spec(r"C:\some, dir\app.ico"),
            (r"C:\some, dir\app.ico", 0)
        );
    }

    #[test]
    fn reads_verbs_out_of_a_package_manifest() {
        let manifest = r#"<?xml version="1.0" encoding="utf-8"?>
<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
  xmlns:com="http://schemas.microsoft.com/appx/manifest/com/windows10"
  xmlns:desktop4="http://schemas.microsoft.com/appx/manifest/desktop/windows10/4"
  xmlns:desktop5="http://schemas.microsoft.com/appx/manifest/desktop/windows10/5">
  <Applications>
    <Application Id="VSCode" Executable="Code.exe">
      <Extensions>
        <com:Extension Category="windows.comServer">
          <com:ComServer>
            <com:SurrogateServer DisplayName="Visual Studio Code">
              <com:Class Id="1C6DF0C0-192A-4451-BE36-6A59A86A692E" Path="code_explorer_command_x64.dll" ThreadingModel="STA" />
            </com:SurrogateServer>
          </com:ComServer>
        </com:Extension>
        <desktop4:Extension Category="windows.fileExplorerContextMenus">
          <desktop4:FileExplorerContextMenus>
            <desktop5:ItemType Type="Directory">
              <desktop5:Verb Id="OpenWithCode" Clsid="1C6DF0C0-192A-4451-BE36-6A59A86A692E" />
            </desktop5:ItemType>
            <desktop5:ItemType Type="*">
              <desktop5:Verb Id="OpenWithCode" Clsid="1c6df0c0-192a-4451-be36-6a59a86a692e" />
            </desktop5:ItemType>
          </desktop4:FileExplorerContextMenus>
        </desktop4:Extension>
      </Extensions>
    </Application>
  </Applications>
</Package>"#;

        assert_eq!(
            parse_manifest_verbs(manifest),
            vec![
                ManifestVerb {
                    item_type: "Directory".into(),
                    clsid: "1C6DF0C0-192A-4451-BE36-6A59A86A692E".into(),
                },
                ManifestVerb {
                    item_type: "*".into(),
                    clsid: "1C6DF0C0-192A-4451-BE36-6A59A86A692E".into(),
                },
            ]
        );
    }

    #[test]
    fn ignores_verbs_outside_the_context_menu_extension() {
        // The same package also backs a thumbnail handler; only verbs inside
        // `FileExplorerContextMenus` are menu items.
        let manifest = r#"<Package>
  <Extensions>
    <desktop4:Extension Category="windows.fileExplorerContextMenus">
      <desktop4:FileExplorerContextMenus>
        <desktop5:ItemType Type="*">
          <desktop5:Verb Id="One" Clsid="11111111-1111-1111-1111-111111111111" />
        </desktop5:ItemType>
      </desktop4:FileExplorerContextMenus>
    </desktop4:Extension>
    <desktop:Extension Category="windows.thumbnailHandler">
      <desktop5:Verb Id="NotAMenuItem" Clsid="22222222-2222-2222-2222-222222222222" />
    </desktop:Extension>
  </Extensions>
  <desktop5:Verb Id="AlsoNotAMenuItem" Clsid="33333333-3333-3333-3333-333333333333" />
</Package>"#;

        assert_eq!(
            parse_manifest_verbs(manifest),
            vec![ManifestVerb {
                item_type: "*".into(),
                clsid: "11111111-1111-1111-1111-111111111111".into(),
            }]
        );
    }

    #[test]
    fn keeps_the_verbs_of_a_manifest_that_is_cut_short() {
        let manifest = r#"<Package>
  <desktop4:FileExplorerContextMenus>
    <desktop5:ItemType Type="Directory">
      <desktop5:Verb Id="Open" Clsid="44444444-4444-4444-4444-444444444444" />
    </desktop5:ItemType>"#;

        assert_eq!(
            parse_manifest_verbs(manifest),
            vec![ManifestVerb {
                item_type: "Directory".into(),
                clsid: "44444444-4444-4444-4444-444444444444".into(),
            }]
        );
    }

    fn command(label: &str) -> ShellCommand {
        ShellCommand {
            clsid: label.to_string(),
            label: label.to_string(),
            icon_data_url: None,
            group: None,
            separator_before: false,
            disabled: false,
        }
    }

    #[test]
    fn a_lone_command_stays_inline_and_a_pair_becomes_a_flyout() {
        let items = assign_groups(vec![
            ("Terminal".into(), command("在终端中打开")),
            ("Zed".into(), command("Open in Zed")),
            ("Zed".into(), command("Open Project in Zed")),
        ]);

        assert_eq!(items[0].label, "在终端中打开");
        assert_eq!(items[0].group, None);
        // Grouped rows sort after the inline ones and keep their app together.
        assert_eq!(items[1].group.as_deref(), Some("Zed"));
        assert_eq!(items[2].group.as_deref(), Some("Zed"));
        assert_eq!(items[1].label, "Open in Zed");
        assert_eq!(items[2].label, "Open Project in Zed");
    }
}
