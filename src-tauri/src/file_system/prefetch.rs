//! One-shot startup prefetch. While the webview is still loading its JS
//! bundle, a background thread answers the exact queries the sidebar and
//! overview surface will make on their first render, so those IPC calls
//! resolve from memory instead of hitting disk or spawning processes.
//!
//! Every slot is consumed on first hit, so a stale snapshot can never be
//! served twice — every later call goes back to the real source.

use std::collections::HashMap;
use std::sync::Mutex;

use tauri::Manager;

use super::recents::{self, RecentItem};
use super::sidebar::{self, DiskVolume, Favorite, SystemPlace, WslDistro};
use super::spaces::{self, Space};
use super::types::{DirectoryView, path_to_string};
use super::vfs;

#[derive(Default)]
pub struct StartupPrefetch {
    directories: Mutex<HashMap<String, DirectoryView>>,
    system_places: Mutex<Option<Vec<SystemPlace>>>,
    disks: Mutex<Option<Vec<DiskVolume>>>,
    wsl_distros: Mutex<Option<Vec<WslDistro>>>,
    favorites: Mutex<Option<Vec<Favorite>>>,
    recents: Mutex<Option<Vec<RecentItem>>>,
    spaces: Mutex<Option<Vec<Space>>>,
}

fn take<T>(slot: &Mutex<Option<T>>) -> Option<T> {
    slot.lock().ok()?.take()
}

fn store<T>(slot: &Mutex<Option<T>>, value: T) {
    if let Ok(mut slot) = slot.lock() {
        *slot = Some(value);
    }
}

impl StartupPrefetch {
    pub fn take_directory(&self, requested_path: &str) -> Option<DirectoryView> {
        self.directories.lock().ok()?.remove(requested_path)
    }

    pub fn take_system_places(&self) -> Option<Vec<SystemPlace>> {
        take(&self.system_places)
    }

    pub fn take_disks(&self) -> Option<Vec<DiskVolume>> {
        take(&self.disks)
    }

    pub fn take_wsl_distros(&self) -> Option<Vec<WslDistro>> {
        take(&self.wsl_distros)
    }

    pub fn take_favorites(&self) -> Option<Vec<Favorite>> {
        take(&self.favorites)
    }

    pub fn take_recents(&self) -> Option<Vec<RecentItem>> {
        take(&self.recents)
    }

    pub fn take_spaces(&self) -> Option<Vec<Space>> {
        take(&self.spaces)
    }
}

/// Answers the startup surface's queries on a background thread while the
/// webview loads. A plain thread, not the async runtime: `vfs::resolve` may
/// block on opening a network session, and the WSL probe spawns `wsl.exe`.
pub fn warm_startup_data(app: &tauri::AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        // Not on the startup path, but the first folder tab (e.g. via the
        // Home quick-access tile) calls `initialize()` which reads the home
        // directory. The cache key must match the exact string
        // `get_home_directory` returns, which is what the frontend requests.
        if let Ok(home) = app.path().home_dir() {
            let requested = path_to_string(&home);
            let view = vfs::resolve(&requested).and_then(|backend| backend.read_dir(&requested));
            if let (Ok(view), Ok(mut directories)) =
                (view, app.state::<StartupPrefetch>().directories.lock())
            {
                directories.insert(requested, view);
            }
        }

        let state = app.state::<StartupPrefetch>();

        // The overview and sidebar first-frame queries. Fallible reads stay
        // uncached on failure so the real command surfaces the error; the
        // disk and WSL probes already degrade to empty lists internally.
        if let Ok(places) = sidebar::get_system_places(app.clone()) {
            store(&state.system_places, places);
        }
        store(&state.disks, sidebar::list_disks_sync());
        store(&state.wsl_distros, sidebar::list_wsl_distros_sync());
        if let Ok(favorites) = sidebar::load_favorites(app.clone()) {
            store(&state.favorites, favorites);
        }
        if let Ok(recent_items) = recents::list_recents(app.clone()) {
            store(&state.recents, recent_items);
        }
        if let Ok(spaces_list) = spaces::list_spaces(app.clone()) {
            store(&state.spaces, spaces_list);
        }
    });
}
