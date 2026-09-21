//! One-shot startup prefetch. While the webview is still loading its JS
//! bundle, a background thread answers the exact queries the sidebar and
//! overview surface will make on their first render, so those IPC calls
//! resolve from memory instead of hitting disk or spawning processes.
//!
//! Every slot is consumed on first hit, so a stale snapshot can never be
//! served twice — every later call goes back to the real source.
//!
//! Disks and WSL distros are deliberately NOT prefetched: their sidebar
//! sections are collapsed by default and query lazily on first expand, so
//! warming them here would spend startup time (the WSL probe spawns
//! `wsl.exe`) on data that is usually never shown.

use std::sync::Mutex;

use tauri::Manager;

use super::recents::{self, RecentItem};
use super::sidebar::{self, Favorite, SystemPlace};
use super::spaces::{self, Space};

#[derive(Default)]
pub struct StartupPrefetch {
    system_places: Mutex<Option<Vec<SystemPlace>>>,
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
    pub fn take_system_places(&self) -> Option<Vec<SystemPlace>> {
        take(&self.system_places)
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
/// webview loads. A plain thread, not the async runtime: `get_system_places`
/// asks the OS for its volumes, which blocks on a dead mapped drive.
pub fn warm_startup_data(app: &tauri::AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let state = app.state::<StartupPrefetch>();

        // The overview and sidebar first-frame queries. Fallible reads stay
        // uncached on failure so the real command surfaces the error.
        if let Ok(places) = sidebar::get_system_places(app.clone()) {
            store(&state.system_places, places);
        }
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
