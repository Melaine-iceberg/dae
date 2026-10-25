//! Process memory for gate 4.
//!
//! Same metric as gate 1 (`bench/qt-list-spike/src/memory.rs`): Linux PSS is
//! what the WebView baseline used, so the Windows reading charges every
//! resident page once and a shared page `1 / share_count` of its size via
//! `QueryWorkingSetEx`. Working set and private commit are recorded alongside
//! so a delta can be read as "new pages" or "new commit".

#[derive(Clone, Copy, Debug, Default)]
pub struct Snapshot {
    pub pss: u64,
    pub working_set: u64,
    pub private: u64,
    pub peak_working_set: u64,
}

pub fn sample() -> Snapshot {
    #[cfg(windows)]
    {
        windows_sample()
    }
    #[cfg(not(windows))]
    {
        proc_sample()
    }
}

#[cfg(windows)]
fn windows_sample() -> Snapshot {
    use std::mem::size_of;

    use windows::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
    use windows::Win32::System::Threading::GetCurrentProcess;

    unsafe {
        let handle = GetCurrentProcess();
        let mut counters = PROCESS_MEMORY_COUNTERS_EX::default();
        counters.cb = size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        let _ = GetProcessMemoryInfo(
            handle,
            &mut counters as *mut _ as *mut PROCESS_MEMORY_COUNTERS,
            counters.cb,
        );

        let mut info = SYSTEM_INFO::default();
        GetSystemInfo(&mut info);
        let page = info.dwPageSize as usize;

        let pss = working_set_pss(handle, page, counters.WorkingSetSize).unwrap_or(0);
        Snapshot {
            pss,
            working_set: counters.WorkingSetSize as u64,
            private: counters.PrivateUsage as u64,
            peak_working_set: counters.PeakWorkingSetSize as u64,
        }
    }
}

#[cfg(windows)]
unsafe fn working_set_pss(
    handle: windows::Win32::Foundation::HANDLE,
    page: usize,
    working_set: usize,
) -> Option<u64> {
    use std::ffi::c_void;
    use std::mem::size_of;

    use windows::Win32::System::ProcessStatus::{
        QueryWorkingSet, QueryWorkingSetEx, PSAPI_WORKING_SET_EX_BLOCK,
        PSAPI_WORKING_SET_EX_INFORMATION,
    };

    // The first word is `NumberOfEntries`, the rest are one flags word per
    // resident page. Start from the working-set size and grow until the query
    // fits; a page can be swapped between the two calls.
    let mut capacity = (working_set / page).saturating_add(4096).max(4096);
    let mut words: Vec<usize> = Vec::new();
    loop {
        let bytes = (1 + capacity) * size_of::<usize>();
        words.clear();
        words.resize(1 + capacity, 0);
        if QueryWorkingSet(handle, words.as_mut_ptr().cast(), bytes as u32).is_ok() {
            break;
        }
        capacity = capacity.checked_mul(2)?;
        if capacity > 64 * 1024 * 1024 {
            return None;
        }
    }

    let count = words[0].min(capacity);
    let mut pages: Vec<PSAPI_WORKING_SET_EX_INFORMATION> = Vec::with_capacity(count);
    for flags in &words[1..=count] {
        // Protection 5 + ShareCount 3 + Shared 1 + Node 3 = 12 low bits.
        let virtual_page = flags >> 12;
        pages.push(PSAPI_WORKING_SET_EX_INFORMATION {
            VirtualAddress: (virtual_page << 12) as *mut c_void,
            VirtualAttributes: PSAPI_WORKING_SET_EX_BLOCK::default(),
        });
    }
    if pages.is_empty() {
        return Some(0);
    }

    let bytes = pages.len() * size_of::<PSAPI_WORKING_SET_EX_INFORMATION>();
    QueryWorkingSetEx(handle, pages.as_mut_ptr().cast(), bytes as u32).ok()?;

    let mut pss = 0u64;
    for entry in &pages {
        let flags = entry.VirtualAttributes.Flags;
        if flags & 1 == 0 {
            continue;
        }
        let shared = (flags >> 15) & 1 == 1;
        let share_count = ((flags >> 1) & 0x7).max(1) as u64;
        pss += if shared {
            page as u64 / share_count
        } else {
            page as u64
        };
    }
    Some(pss)
}

#[cfg(not(windows))]
fn proc_sample() -> Snapshot {
    use std::fs;

    let status = fs::read_to_string("/proc/self/status").unwrap_or_default();
    let kilobytes = |key: &str| -> u64 {
        status
            .lines()
            .find_map(|line| line.strip_prefix(key))
            .and_then(|value| value.split_whitespace().next()?.parse::<u64>().ok())
            .unwrap_or(0)
            * 1024
    };
    let pss = fs::read_to_string("/proc/self/smaps_rollup")
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|line| line.strip_prefix("Pss:"))
                .and_then(|value| value.split_whitespace().next()?.parse::<u64>().ok())
        })
        .map(|value| value * 1024)
        .unwrap_or(0);
    let working_set = kilobytes("VmRSS:");
    Snapshot {
        pss,
        working_set,
        private: 0,
        peak_working_set: kilobytes("VmHWM:"),
    }
}
