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

use std::sync::{Mutex, OnceLock};


use super::recents::{self, RecentItem};
use super::sidebar::{Favorite, SystemPlace};
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

/// The process-wide prefetch slots. One app per process, so this is the
/// lifetime the managed state had.
pub(crate) fn state() -> &'static StartupPrefetch {
    static STATE: OnceLock<StartupPrefetch> = OnceLock::new();
    STATE.get_or_init(StartupPrefetch::default)
}

/// Answers the startup surface's queries on a background thread while the
/// webview loads.
///
/// The caller resolves the system places first — that needs the window
/// manager — and hands them over, so this module never sees a shell handle.
/// A plain thread, not the async runtime: the sidebar reads block on disk.
pub fn warm_startup_data(places: Option<Vec<SystemPlace>>) {
    std::thread::spawn(move || {
        let state = state();

        // The overview and sidebar first-frame queries. Fallible reads stay
        // uncached on failure so the real command surfaces the error.
        if let Some(places) = places {
            store(&state.system_places, places);
        }
        if let Ok(favorites) = super::sidebar::load_favorites() {
            store(&state.favorites, favorites);
        }
        if let Ok(recent_items) = recents::list_recents() {
            store(&state.recents, recent_items);
        }
        if let Ok(spaces_list) = spaces::list_spaces() {
            store(&state.spaces, spaces_list);
        }
    });
}
