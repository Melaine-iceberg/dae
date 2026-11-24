//! The Windows taskbar jump list: the folder menu behind a right-click on
//! dae's taskbar button.
//!
//! Explorer's button carries two folder groups there — the ones the user
//! pinned and the ones they open often. dae gets the same shape from its own
//! two documents instead of from the shell's usage tracking, so both groups
//! already mean something inside the app:
//!
//! * **Pinned** — the sidebar's pinned section: user favorites first, then the
//!   visible system places, shown as "已固定".
//! * **Frequent** — `recent-items.json`, the folders the user browsed, newest
//!   first, shown as "常用".
//!
//! Both are handed to the shell as links that run dae again with the folder as
//! their argument, so the row opens in dae whatever the user has set as their
//! folder handler. The link carries two things the shell resolves for us: the
//! folder's own icon, asked of `SHGetFileInfoW` (`SHGFI_ICONLOCATION`) so a row
//! wears the blue Desktop and the green Downloads arrow rather than a generic
//! picture, and the folder's display name, asked of the shell item so it reads
//! "桌面" on a Chinese install exactly as Explorer's list does.
//!
//! # Why links and not folders
//!
//! The obvious row is an `IShellItem` for the folder itself. Windows refuses
//! that one: `AppendCategory` answers `0x80040F03`
//! (`DESTS_E_NO_MATCHING_ASSOC_HANDLER`) unless the application is a registered
//! handler for the item's type, and dae is not — a packaged install cannot
//! register itself for folders at all (`docs/decisions/engine-qt-quick.md`
//! §9.15), so the list would be empty for every MSIX user. A link to dae with
//! an argument list is the shape Windows documents for exactly this, and it has
//! the side benefit that pinning a row pins *dae opening that folder*.
//!
//! Windows keeps the pinned group itself; dae only has to honour the deletions
//! the user asked for, which is what [`reinclude`] undoes when the same folder
//! is favorited again.
//!
//! # Why there is no AppUserModelID here
//!
//! The list is keyed by the application's AppUserModelID, and the tempting
//! move is to declare one with `SetCurrentProcessExplicitAppUserModelID`. dae
//! does not: a declared ID has to match the one stamped on every shortcut that
//! launches the app (`System.AppUserModel.ID`), and dae ships no installer that
//! could write that property into a pin the user made by dragging `dae.exe` to
//! the taskbar. A process ID that disagrees with its shortcut is the documented
//! way to end up with two taskbar buttons. Both shapes dae is shipped in
//! already have a stable identity without one: the MSIX gets it from the
//! package manifest, and the portable zip keeps its executable path across the
//! in-place updates the updater performs.
//!
//! # Threading
//!
//! `ICustomDestinationList` is a shell COM object, and rewriting the list on
//! every directory visit would put a file write on the thread that paints.
//! Requests therefore land on one worker (created on demand), which waits for
//! the navigation to go quiet and writes once — [`refresh`] returns
//! immediately and never blocks the caller.

use std::path::{Path, PathBuf};
#[cfg(any(windows, test))]
use std::sync::Mutex;

use dae_core::fs::recents::{RecentItem, RecentSource};
use dae_core::fs::sidebar::Favorite;
use dae_core::types::EntryKind;
#[cfg(any(windows, test))]
use serde::{Deserialize, Serialize};

/// How many folders one group holds.
///
/// Explorer's groups are capped at ten by the shell; dae keeps eight so the
/// menu still fits above the taskbar's own rows on a short screen.
const GROUP_LIMIT: usize = 8;

/// How many deletions the document remembers before it stops growing. A user
/// who deletes more than this from a jump list is telling us the list is not
/// for them, and an unbounded document is a worse bug than that.
#[cfg(any(windows, test))]
const REMOVALS_LIMIT: usize = 128;

/// The document that remembers what the user deleted from the list.
#[cfg(any(windows, test))]
const REMOVALS_FILE: &str = "jump-list.json";

/// Asks for the jump list to be rebuilt.
///
/// Cheap and safe to call on every navigation: the work is coalesced onto one
/// worker that waits for the list to stop changing. A no-op off Windows, where
/// the taskbar has no jump lists.
pub fn refresh() {
    #[cfg(windows)]
    imp::refresh();
}

/// Forgets that `path` was deleted from the jump list.
///
/// Windows expects an application to keep a destination out of its list once
/// the user removed it, so the deletion is written down here. Favoriting the
/// same folder again is a newer and stronger statement than the deletion, and
/// this is what lets it back in.
pub fn reinclude(path: &str) {
    #[cfg(windows)]
    imp::reinclude(path);

    #[cfg(not(windows))]
    {
        let _ = path;
    }
}

/// The folders the favorites group shows: local, still on disk, and not one the
/// user deleted from the jump list.
#[cfg_attr(not(windows), allow(dead_code))]
fn favorite_paths(favorites: &[Favorite], removed: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for favorite in favorites {
        push_candidate(&mut out, &favorite.path, removed);
    }
    out
}

/// The "Pinned" group mirrors the sidebar's pinned section: user favorites
/// first, then the system places that section always contains. The places are
/// also what keeps the heading present before the user has pinned anything.
#[cfg_attr(not(windows), allow(dead_code))]
fn pinned_paths(favorites: &[Favorite], places: &[String], removed: &[String]) -> Vec<PathBuf> {
    let mut out = favorite_paths(favorites, removed);
    for place in places {
        push_candidate(&mut out, place, removed);
    }
    out
}

/// The folders the frequent group shows.
///
/// `recents` is already newest first, which is the order the group is read in:
/// the store keeps a last-visited time but no per-folder count, so recency is
/// the ranking it can honestly offer.
#[cfg_attr(not(windows), allow(dead_code))]
fn frequent_paths(
    recents: &[RecentItem],
    favorites: &[PathBuf],
    removed: &[String],
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for item in recents {
        // A file that was opened is not a place to go back to.
        if item.source != RecentSource::Visited || item.kind != EntryKind::Directory {
            continue;
        }
        // A pinned folder already has a row; repeating it under a second
        // heading is what Windows' own lists avoid.
        if favorites
            .iter()
            .any(|path| fold(&path.to_string_lossy()) == fold(&item.path))
        {
            continue;
        }
        push_candidate(&mut out, &item.path, removed);
    }
    out
}

/// Adds `path` to `out` when the jump list can show it.
///
/// `path` is a stored string, not a `Path`: the favorites and the recents are
/// both user-editable documents and either may name something that has since
/// been renamed, unmounted or deleted, which the shell would answer with an
/// empty row.
#[cfg_attr(not(windows), allow(dead_code))]
fn push_candidate(out: &mut Vec<PathBuf>, path: &str, removed: &[String]) {
    if out.len() >= GROUP_LIMIT || path.is_empty() {
        return;
    }
    // Only the local disk: an `sftp://` or archive path is a backend's private
    // spelling of a location, not something the shell can open as a folder. A
    // relative one is not a place at all — the stored spelling is resolved
    // against whichever directory the launcher happens to start in.
    if !dae_core::fs::vfs::is_local_path(path) || !Path::new(path).is_absolute() {
        return;
    }
    if !Path::new(path).is_dir() {
        return;
    }
    let folded = fold(path);
    if removed.iter().any(|entry| fold(entry) == folded) {
        return;
    }
    if out
        .iter()
        .any(|kept| fold(&kept.to_string_lossy()) == folded)
    {
        return;
    }
    out.push(PathBuf::from(path));
}

/// Windows compares paths case-insensitively, and `C:\code\dae` and
/// `C:\code\dae\` name one folder, so every comparison here goes through this.
#[cfg_attr(not(windows), allow(dead_code))]
fn fold(path: &str) -> String {
    let trimmed = path.trim_end_matches(['\\', '/']);
    let trimmed = if trimmed.is_empty() { path } else { trimmed };
    trimmed.replace('/', "\\").to_lowercase()
}

/// One command-line argument, quoted the way the Windows parser reads it back.
///
/// The trailing backslash is why this is not `format!("\"{path}\"")`: a drive
/// root is `K:\`, and `"K:\"` escapes the closing quote, so the row would start
/// dae with a folder nobody asked for — or, when the shell validates the link,
/// fail the whole group with `E_INVALIDARG`. Backslashes that precede the
/// closing quote are doubled, which is what turns `K:\` into `"K:\\"`.
#[cfg_attr(not(windows), allow(dead_code))]
fn quote(argument: &str) -> String {
    let mut quoted = String::with_capacity(argument.len() + 3);
    quoted.push('"');
    let mut backslashes = 0;
    for character in argument.chars() {
        match character {
            '\\' => {
                backslashes += 1;
                quoted.push('\\');
            }
            '"' => {
                // A literal quote keeps the backslashes that escape it.
                for _ in 0..=backslashes {
                    quoted.push('\\');
                }
                backslashes = 0;
                quoted.push('"');
            }
            other => {
                backslashes = 0;
                quoted.push(other);
            }
        }
    }
    for _ in 0..backslashes {
        quoted.push('\\');
    }
    quoted.push('"');
    quoted
}

/// The document behind [`reinclude`].
///
/// Everything from here to [`load_recents`] exists for the Windows list only;
/// it is compiled under `test` as well so the versioning rules can be checked
/// on the platforms `qt-check` builds.
#[cfg(any(windows, test))]
#[derive(Debug, Default, Serialize, Deserialize)]
struct Removals {
    #[serde(default)]
    removed: Vec<String>,
}

/// Serialises the read-modify-write of the removals document: `commit` writes it
/// from the worker while `reinclude` writes it from the thread that saved a
/// favorite, and a lost update there would either resurrect a deleted row or
/// hide a favorite for good.
#[cfg(any(windows, test))]
static REMOVALS_LOCK: Mutex<()> = Mutex::new(());

/// Applies `edit` to the deletions and writes the document back when it
/// reports a change, answering with the list as it now stands.
#[cfg(any(windows, test))]
fn update_removals(edit: impl FnOnce(&mut Vec<String>) -> bool) -> Vec<String> {
    let _guard = REMOVALS_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());

    let mut removed = read_removals();
    if edit(&mut removed) {
        write_removals(&removed);
    }
    removed
}

/// Failures are logged and dropped: a jump list that cannot remember a deletion
/// is still a working jump list.
#[cfg(any(windows, test))]
fn write_removals(removed: &[String]) {
    let Some(path) = removals_path() else {
        return;
    };
    let Ok(contents) = serde_json::to_vec_pretty(&Removals {
        removed: removed.to_vec(),
    }) else {
        return;
    };
    if let Some(parent) = path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            log::warn!(target: "dae::jump_list", "could not create {}: {error}", parent.display());
            return;
        }
    }
    if let Err(error) = dae_core::fs::atomic::write_atomic(&path, &contents) {
        log::warn!(target: "dae::jump_list", "could not record the jump list deletions: {error}");
    }
}

#[cfg(any(windows, test))]
fn read_removals() -> Vec<String> {
    let Some(path) = removals_path() else {
        return Vec::new();
    };
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str::<Removals>(&contents)
        .map(|document| document.removed)
        .unwrap_or_default()
}

/// The deletions live beside the favorites and the recents they apply to, so a
/// user who copies their profile gets the jump list they had.
#[cfg(any(windows, test))]
fn removals_path() -> Option<PathBuf> {
    dae_core::paths::config_dir()
        .ok()
        .map(|directory| directory.join(REMOVALS_FILE))
}

/// Adds the destinations the shell reported as deleted to `removed`.
///
/// Windows reports deletions once, in the session that follows them, and the
/// application is expected to carry them forward: a custom list is replaced
/// wholesale on every commit, so the memory has to live here.
#[cfg(any(windows, test))]
fn merge_removals(removed: &mut Vec<String>, reported: Vec<String>) -> bool {
    let mut changed = false;
    for path in reported {
        let folded = fold(&path);
        if removed.iter().any(|entry| fold(entry) == folded) {
            continue;
        }
        if removed.len() >= REMOVALS_LIMIT {
            break;
        }
        removed.push(path);
        changed = true;
    }
    changed
}

/// Drops `path` from `removed`, which is what re-favoriting a folder does.
#[cfg(any(windows, test))]
fn forget_removal(removed: &mut Vec<String>, path: &str) -> bool {
    let folded = fold(path);
    let before = removed.len();
    removed.retain(|entry| fold(entry) != folded);
    removed.len() != before
}

#[cfg(any(windows, test))]
fn load_favorites() -> Vec<Favorite> {
    dae_core::fs::sidebar::load_favorites().unwrap_or_else(|error| {
        log::warn!(target: "dae::jump_list", "could not read the favorites: {error}");
        Vec::new()
    })
}

/// The visible system places, in the order the sidebar's pinned section shows
/// them. Hidden places are left out, so the taskbar menu and the sidebar do
/// not disagree about what belongs in that section.
#[cfg(any(windows, test))]
fn load_place_paths() -> Vec<String> {
    let hidden = dae_core::fs::sidebar::load_hidden_places().unwrap_or_default();
    dae_core::fs::sidebar::system_places()
        .into_iter()
        .filter(|place| !hidden.contains(&place.kind))
        .map(|place| place.path)
        .collect()
}

#[cfg(any(windows, test))]
fn load_recents() -> Vec<RecentItem> {
    dae_core::fs::recents::list_recents().unwrap_or_else(|error| {
        log::warn!(target: "dae::jump_list", "could not read the recent items: {error}");
        Vec::new()
    })
}

/// The two group titles, in the language the shell is being read in.
///
/// A jump list is drawn by the taskbar, outside QML, so the titles cannot come
/// from `qsTr()`. The stored language decides: English gets English, and both
/// Chinese scripts get the source wording the rest of the interface uses.
#[cfg_attr(not(windows), allow(dead_code))]
fn titles() -> (&'static str, &'static str) {
    if uses_english() {
        ("Pinned", "Frequent")
    } else {
        ("已固定", "常用")
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
fn uses_english() -> bool {
    let stored = crate::appearance::stored_language();
    if stored.eq_ignore_ascii_case("en") {
        return true;
    }
    if stored.eq_ignore_ascii_case("system") {
        let system = crate::i18n::system_language().to_ascii_lowercase();
        return !system.starts_with("zh");
    }
    false
}

#[cfg(windows)]
mod imp {
    use std::path::Path;
    use std::sync::OnceLock;
    use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
    use std::time::Duration;

    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoTaskMemFree, IBindCtx,
    };
    use windows::Win32::UI::Shell::Common::{IObjectArray, IObjectCollection};
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    use windows::Win32::UI::Shell::{
        DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellItem,
        IShellLinkW, SHCreateItemFromParsingName, SHFILEINFOW, SHGFI_ICONLOCATION, SHGetFileInfoW,
        SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_NORMALDISPLAY, ShellLink,
    };
    use windows::core::{GUID, HSTRING, Interface};

    use super::{
        forget_removal, frequent_paths, load_favorites, load_place_paths, load_recents,
        merge_removals, pinned_paths, quote, titles, update_removals,
    };

    /// `System.Title` — `FMTID_SummaryInformation`, property 2. The jump list
    /// reads a row's text from this property, not from the link's description,
    /// and rejects a link that does not carry it.
    ///
    /// Spelled out here because `PKEY_Title` is generated into the
    /// `EnhancedStorage` module, which nothing else in this crate needs.
    const TITLE_KEY: PROPERTYKEY = PROPERTYKEY {
        fmtid: GUID::from_u128(0xf29f85e0_4ff9_1068_ab91_08002b27b3d9),
        pid: 2,
    };

    /// How long the list has to stand still before it is written. Navigating
    /// with the arrow keys is one rebuild per pause, not one per directory.
    const QUIET: Duration = Duration::from_millis(600);

    /// The worker's mailbox. Capacity one: a request that arrives while one is
    /// still queued is the same request.
    static REQUESTS: OnceLock<SyncSender<()>> = OnceLock::new();

    pub fn refresh() {
        let sender = REQUESTS.get_or_init(|| {
            let (sender, receiver) = sync_channel(1);
            if let Err(error) = std::thread::Builder::new()
                .name("dae-jump-list".to_owned())
                .spawn(move || work(receiver))
            {
                // Nothing else can be done: the list simply stops following the
                // session. The log line is the only trace of it.
                log::warn!(target: "dae::jump_list", "could not start the jump list thread: {error}");
            }
            sender
        });

        match sender.try_send(()) {
            // A queued request covers this one; a dead worker is already logged.
            Ok(()) | Err(TrySendError::Full(())) | Err(TrySendError::Disconnected(())) => {}
        }
    }

    pub fn reinclude(path: &str) {
        let _ = update_removals(|removed| forget_removal(removed, path));
    }

    fn work(receiver: Receiver<()>) {
        while receiver.recv().is_ok() {
            loop {
                match receiver.recv_timeout(QUIET) {
                    Ok(()) => continue,
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }

            if let Err(error) = commit() {
                log::warn!(target: "dae::jump_list", "{error}");
            }
        }
    }

    /// Replaces the whole list: the shell has no "update one row", only a
    /// begin / append / commit session.
    fn commit() -> Result<(), String> {
        // The shell objects need an apartment on this thread. The worker owns
        // this one, so the only other answer is `RPC_E_CHANGED_MODE`, which
        // means it was already initialised and is harmless.
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };

        let list = create_list()?;

        // `pcMinSlots` is an out-parameter we have no use for; zero asks the
        // shell for its own answer.
        let mut slots = 0u32;
        let reported: IObjectArray = unsafe { list.BeginList(&mut slots) }
            .map_err(|error| format!("the jump list session did not open: {error}"))?;

        // A custom list is replaced wholesale on commit, so the deletions the
        // user made are the application's to carry forward.
        let removed = update_removals(|removed| merge_removals(removed, reported_paths(&reported)));

        let pinned = pinned_paths(&load_favorites(), &load_place_paths(), &removed);
        let frequent = frequent_paths(&load_recents(), &pinned, &removed);

        let written = append_groups(&list, &pinned, &frequent);
        if let Err(error) = written {
            let _ = unsafe { list.AbortList() };
            return Err(error);
        }

        unsafe { list.CommitList() }
            .map_err(|error| format!("the jump list was not committed: {error}"))?;
        log::debug!(
            target: "dae::jump_list",
            "committed {} pinned and {} frequent folders",
            pinned.len(),
            frequent.len()
        );
        Ok(())
    }

    fn create_list() -> Result<ICustomDestinationList, String> {
        // No `SetAppID`: the list belongs to whatever identity the process was
        // given, which is the one the taskbar button is already grouped under.
        unsafe { CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER) }
            .map_err(|error| format!("the jump list object is unavailable: {error}"))
    }

    fn append_groups(
        list: &ICustomDestinationList,
        pinned: &[std::path::PathBuf],
        frequent: &[std::path::PathBuf],
    ) -> Result<(), String> {
        let (pinned_title, frequent_title) = titles();
        if !pinned.is_empty() {
            append(list, pinned_title, pinned)?;
        }
        if !frequent.is_empty() {
            append(list, frequent_title, frequent)?;
        }
        Ok(())
    }

    /// One category, as a collection of links that reopen dae on the folder.
    ///
    /// The executable is resolved the same way the folder handler registers it,
    /// so a packaged install points at the package's execution alias and
    /// survives the next update rather than pinning the versioned directory.
    fn append(
        list: &ICustomDestinationList,
        title: &str,
        paths: &[std::path::PathBuf],
    ) -> Result<(), String> {
        let executable =
            crate::default_manager::launch_path().map_err(|error| error.to_string())?;

        let collection: IObjectCollection =
            unsafe { CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER) }
                .map_err(|error| format!("could not start the {title} group: {error}"))?;

        for path in paths {
            let link = link_for(path, &executable)?;
            unsafe { collection.AddObject(&link) }
                .map_err(|error| format!("could not add {}: {error}", path.display()))?;
        }

        let items: &IObjectArray = &collection;
        unsafe { list.AppendCategory(&HSTRING::from(title), items) }
            .map_err(|error| format!("could not append the {title} group: {error}"))
    }

    /// The link behind one row: dae, started on `folder`.
    ///
    /// The four properties are the set Windows documents as required for a
    /// link in a jump list, and it validates every one of them: without
    /// `SetArguments` the row could not be pinned, without `SetIconLocation` it
    /// has no picture, and the row's text must come from `System.Title` — a
    /// `SetDescription` alone fails the whole group with `E_INVALIDARG`.
    fn link_for(folder: &Path, executable: &Path) -> Result<IShellLinkW, String> {
        let link: IShellLinkW = unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
            .map_err(|error| format!("could not build a row for {}: {error}", folder.display()))?;

        // One quoted argument: the activation parser in `crate::activation`
        // reads it as a directory to open, and a second launch forwards it to
        // the window already running.
        let arguments = HSTRING::from(quote(&folder.to_string_lossy()));
        let name = display_name(folder);
        let (icon, index) = icon_location(folder);

        unsafe {
            link.SetPath(&HSTRING::from(executable))
                .and_then(|()| link.SetArguments(&arguments))
                .and_then(|()| link.SetIconLocation(&HSTRING::from(icon.as_str()), index))
                // The tooltip; the row's text is the title set below.
                .and_then(|()| {
                    link.SetDescription(&HSTRING::from(folder.to_string_lossy().as_ref()))
                })
        }
        .map_err(|error| format!("could not describe {}: {error}", folder.display()))?;

        set_title(&link, &name)
            .map_err(|error| format!("could not name {}: {error}", folder.display()))?;

        Ok(link)
    }

    /// Sets the row's text, which the jump list reads from `System.Title`.
    fn set_title(link: &IShellLinkW, title: &str) -> windows::core::Result<()> {
        let store: IPropertyStore = link.cast()?;
        let value = PROPVARIANT::from(title);
        unsafe { store.SetValue(&TITLE_KEY, &value) }
    }

    /// What the shell calls `folder` — "桌面" for the Desktop on a Chinese
    /// install, "dae" for an ordinary directory.
    fn display_name(folder: &Path) -> String {
        let shell_name = || -> Option<String> {
            let item: IShellItem =
                unsafe { SHCreateItemFromParsingName(&HSTRING::from(folder), None::<&IBindCtx>) }
                    .ok()?;
            let name = unsafe { item.GetDisplayName(SIGDN_NORMALDISPLAY) }.ok()?;
            // SAFETY: `GetDisplayName` hands back a CoTaskMem allocation holding
            // a NUL-terminated string; it is copied out and released here, at
            // the only owner.
            let text = unsafe { name.to_string() }.unwrap_or_default();
            unsafe { CoTaskMemFree(Some(name.0 as *const core::ffi::c_void)) };
            (!text.is_empty()).then_some(text)
        };

        shell_name()
            .or_else(|| {
                folder
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| folder.display().to_string())
    }

    /// Where the shell keeps `folder`'s icon: a module and a resource index.
    ///
    /// This is what makes a row wear the folder's real picture — the shell
    /// answers `imageres.dll,-184` for Downloads and `-183` for the Desktop.
    /// A directory is not itself an icon source, so pointing the link at the
    /// folder would draw a blank page instead.
    fn icon_location(folder: &Path) -> (String, i32) {
        let mut info = SHFILEINFOW::default();
        let asked = unsafe {
            SHGetFileInfoW(
                &HSTRING::from(folder),
                FILE_FLAGS_AND_ATTRIBUTES::default(),
                Some(&mut info),
                std::mem::size_of::<SHFILEINFOW>() as u32,
                SHGFI_ICONLOCATION,
            )
        };
        if asked == 0 {
            return plain_folder_icon();
        }

        let end = info
            .szDisplayName
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(0);
        let path = String::from_utf16_lossy(&info.szDisplayName[..end]);
        // The buffer is MAX_PATH: a location cut off there names no module, and
        // the row would draw nothing at all.
        if path.is_empty() || !Path::new(&path).is_file() {
            return plain_folder_icon();
        }
        (path, info.iIcon)
    }

    /// The ordinary folder icon, for a path the shell has no picture for.
    fn plain_folder_icon() -> (String, i32) {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_owned());
        (format!("{root}\\System32\\shell32.dll"), 3)
    }

    /// The folders the shell says the user deleted from the list.
    ///
    /// The array holds whatever the previous list was built from — a shell item
    /// for a row Windows pinned itself, a link for the rows dae added — so both
    /// shapes are read. A link is identified by the folder in its arguments,
    /// which is the same string the deletion has to match later.
    fn reported_paths(reported: &IObjectArray) -> Vec<String> {
        let mut paths = Vec::new();
        let Ok(count) = (unsafe { reported.GetCount() }) else {
            return paths;
        };

        for index in 0..count {
            let Ok(item) = (unsafe { reported.GetAt::<IShellItem>(index) }) else {
                if let Some(path) = reported_link_path(reported, index) {
                    paths.push(path);
                }
                continue;
            };

            let Ok(name) = (unsafe { item.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING) }) else {
                continue;
            };
            // SAFETY: `GetDisplayName` hands back a CoTaskMem allocation holding
            // a NUL-terminated string; it is copied out and released here, at
            // the only owner.
            let text = unsafe { name.to_string() }.unwrap_or_default();
            unsafe { CoTaskMemFree(Some(name.0 as *const core::ffi::c_void)) };
            if !text.is_empty() {
                paths.push(text);
            }
        }

        paths
    }

    /// The folder named in a deleted row's argument list, if that row is a link
    /// dae built (`"<folder>"`, as [`link_for`] writes it).
    fn reported_link_path(reported: &IObjectArray, index: u32) -> Option<String> {
        let link = unsafe { reported.GetAt::<IShellLinkW>(index) }.ok()?;
        let mut buffer = [0u16; 1024];
        unsafe { link.GetArguments(&mut buffer) }.ok()?;
        let end = buffer.iter().position(|unit| *unit == 0).unwrap_or(0);
        let arguments = String::from_utf16_lossy(&buffer[..end]);

        let folder = arguments.trim();
        let folder = folder
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
            .unwrap_or(folder);
        (!folder.is_empty()).then(|| folder.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "dae-jump-list-{}-{}-{label}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).expect("create temp directory");
            Self(path)
        }

        fn path(&self) -> String {
            self.0.to_string_lossy().into_owned()
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn favorite(path: &str) -> Favorite {
        Favorite {
            name: path.to_owned(),
            path: path.to_owned(),
        }
    }

    fn visited(path: &str) -> RecentItem {
        RecentItem {
            path: path.to_owned(),
            name: path.to_owned(),
            kind: EntryKind::Directory,
            source: RecentSource::Visited,
            accessed_at: 1,
        }
    }

    /// The list is a menu of places; a folder that is gone, or one that only
    /// exists inside an archive, would draw as an empty row.
    #[test]
    fn a_missing_or_remote_folder_is_not_a_destination() {
        let temp = TempDir::new("missing");
        let gone = format!("{}\\gone", temp.path());

        assert_eq!(favorite_paths(&[favorite(&temp.path())], &[]).len(), 1);
        assert!(favorite_paths(&[favorite(&gone)], &[]).is_empty());
        assert!(favorite_paths(&[favorite("sftp://host/dir")], &[]).is_empty());
        assert!(favorite_paths(&[favorite("")], &[]).is_empty());
    }

    /// A stored path that is not absolute would be resolved against whichever
    /// directory the launcher happens to start in, so it is not a destination.
    #[test]
    fn a_relative_stored_path_is_not_a_destination() {
        assert!(favorite_paths(&[favorite("dae/bench")], &[]).is_empty());
        assert!(favorite_paths(&[favorite("./dae")], &[]).is_empty());
    }

    /// The row is started as `dae "<folder>"`, and a drive root is the case
    /// where naive quoting hands the parser an escaped closing quote.
    #[test]
    fn a_folder_is_quoted_for_the_command_line() {
        assert_eq!(quote(r"K:\code\dae"), r#""K:\code\dae""#);
        assert_eq!(quote(r"K:\"), r#""K:\\""#);
        assert_eq!(quote(r"C:\a b\"), r#""C:\a b\\""#);
        // A quote inside the path keeps the backslashes that escape it.
        assert_eq!(quote(r#"a\"b"#), r#""a\\\"b""#);
    }

    /// A pinned folder is already in the list; the frequent group must not
    /// repeat it, and neither group may repeat itself.
    #[test]
    fn the_groups_do_not_repeat_a_folder() {
        let temp = TempDir::new("repeats");
        let path = temp.path();

        let pinned = pinned_paths(&[favorite(&path), favorite(&path)], &[], &[]);
        assert_eq!(pinned.len(), 1);

        let frequent = frequent_paths(&[visited(&path), visited(&path)], &pinned, &[]);
        assert!(
            frequent.is_empty(),
            "a pinned folder is not also a frequent row"
        );
    }

    /// With no user pins, the system places still give the "Pinned" category
    /// rows, so both it and "Frequent" remain in the taskbar menu.
    #[test]
    fn system_places_keep_the_pinned_group_visible() {
        let temp = TempDir::new("places");
        let path = temp.path();

        let pinned = pinned_paths(&[], &[path.clone()], &[]);
        assert_eq!(pinned, [PathBuf::from(path)]);
    }

    /// A folder the user deleted from the list stays out, even when the stored
    /// spelling differs in case or in a trailing separator.
    #[test]
    fn a_deleted_folder_stays_out_however_it_is_spelled() {
        let temp = TempDir::new("removed");
        let path = temp.path();
        let shouty = path.to_uppercase();
        let trailing = format!("{path}\\");

        assert!(favorite_paths(&[favorite(&path)], &[shouty]).is_empty());
        assert!(favorite_paths(&[favorite(&path)], &[trailing]).is_empty());
        assert_eq!(
            favorite_paths(&[favorite(&path)], &["C:\\elsewhere".to_owned()]).len(),
            1
        );
    }

    /// Windows reports a deletion once; carrying it forward has to be
    /// idempotent, or the document grows on every commit.
    #[test]
    fn merging_reported_deletions_is_idempotent() {
        let mut removed = Vec::new();
        assert!(merge_removals(
            &mut removed,
            vec![r"C:\code\dae".to_owned()]
        ));
        assert_eq!(removed.len(), 1);

        // The same folder again — other case, other trailing separator — is not
        // a second deletion.
        assert!(!merge_removals(
            &mut removed,
            vec![r"c:\CODE\dae\".to_owned()]
        ));
        assert_eq!(removed, [r"C:\code\dae"]);

        assert!(merge_removals(&mut removed, vec![r"C:\other".to_owned()]));
        assert_eq!(removed, [r"C:\code\dae", r"C:\other"]);
    }

    /// Favoriting a folder again outranks the earlier deletion.
    #[test]
    fn re_adding_a_folder_forgets_its_deletion() {
        let mut removed = vec!["C:\\code\\dae".to_owned(), "C:\\other".to_owned()];

        assert!(forget_removal(&mut removed, r"c:\code\DAE\"));
        assert_eq!(removed, ["C:\\other"]);
        assert!(!forget_removal(&mut removed, r"c:\code\DAE\"));
    }
}
