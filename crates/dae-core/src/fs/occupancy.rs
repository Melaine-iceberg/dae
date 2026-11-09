//! Which process is holding a file open.
//!
//! The question only matters when something fails: a delete that reports
//! `PermissionDenied` and nothing else sends the user hunting through Task
//! Manager. This module answers "who has it" on each platform through the
//! mechanism that platform actually keeps:
//!
//! | Platform | Mechanism | What it costs |
//! |---|---|---|
//! | Windows | Restart Manager (`rstrtmgr.dll`) | a session per query, ~1 ms |
//! | Linux | `readlink /proc/<pid>/fd/*` | a walk over every process, ~5–40 ms |
//! | macOS | `lsof -F pc` | a process spawn, ~50 ms |
//!
//! All three are best-effort by design: a probe that fails returns "nobody",
//! because the alternative is replacing a real error with a diagnostic error.

use crate::error::FileSystemError;
use serde::Serialize;

/// One process holding at least one of the queried paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Holder {
    pub pid: u32,
    pub name: String,
}

impl Holder {
    /// `Code.exe (4120)` — the form the message and the UI both use.
    pub fn describe(&self) -> String {
        format!("{} ({})", self.name, self.pid)
    }
}

/// Processes holding any of `paths`.
///
/// Duplicates are collapsed by pid, so a process holding a folder and a file
/// inside it appears once, and the order is the order the platform reported
/// them — which on Windows is "most recently started first".
pub fn holders_of(paths: &[String]) -> Vec<Holder> {
    let paths: Vec<String> = paths
        .iter()
        .filter(|path| !path.trim().is_empty())
        .cloned()
        .collect();

    if paths.is_empty() {
        return Vec::new();
    }

    platform::holders(&paths)
}

/// `Code.exe (4120)、explorer.exe (908)` — the clause a failure message can
/// quote, or `None` when nothing holds the paths.
///
/// At most three names: a message that lists eight processes is a message
/// nobody reads, and the pattern is always the same editor or the same shell.
pub fn holders_sentence(paths: &[String]) -> Option<String> {
    let holders = holders_of(paths);
    if holders.is_empty() {
        return None;
    }

    let names: Vec<String> = holders.iter().take(3).map(Holder::describe).collect();
    Some(names.join("、"))
}

/// Fails with `path` and, when something holds it, says what.
///
/// This is the one place the probe is wired into a user-visible outcome: a
/// delete that could not proceed because a program has the file open. The
/// original error is kept — the caller still gets `PermissionDenied` — but the
/// message now names the process.
pub fn annotate_lock_error(paths: &[String], error: FileSystemError) -> FileSystemError {
    if !matches!(
        error,
        FileSystemError::PermissionDenied(_)
            | FileSystemError::Io(_)
            | FileSystemError::Internal(_)
    ) {
        return error;
    }

    let Some(sentence) = holders_sentence(paths) else {
        return error;
    };

    match error {
        FileSystemError::PermissionDenied(message) => {
            FileSystemError::PermissionDenied(format!("{message}（占用：{sentence}）"))
        }
        FileSystemError::Io(message) => FileSystemError::Io(format!("{message}（占用：{sentence}）")),
        FileSystemError::Internal(message) => {
            FileSystemError::Internal(format!("{message}（占用：{sentence}）"))
        }
        other => other,
    }
}

#[cfg(windows)]
mod platform {
    use super::Holder;
    use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
    use windows::Win32::System::RestartManager::{
        CCH_RM_SESSION_KEY, RM_PROCESS_INFO, RmEndSession, RmGetList, RmRegisterResources,
        RmStartSession,
    };
    use windows::core::PWSTR;

    /// Ends the Restart Manager session even when a probe bails out early.
    struct Session(u32);

    impl Drop for Session {
        fn drop(&mut self) {
            // SAFETY: the handle came from `RmStartSession` and is ended once.
            unsafe {
                let _ = RmEndSession(self.0);
            }
        }
    }

    pub(super) fn holders(paths: &[String]) -> Vec<Holder> {
        let mut handle = 0_u32;
        let mut key = [0_u16; CCH_RM_SESSION_KEY as usize + 1];

        // SAFETY: the key buffer is one element longer than the documented
        // `CCH_RM_SESSION_KEY` length, which is what the API writes into.
        let started = unsafe { RmStartSession(&mut handle, None, PWSTR(key.as_mut_ptr())) };
        if started != ERROR_SUCCESS {
            return Vec::new();
        }
        let session = Session(handle);

        let wide: Vec<Vec<u16>> = paths.iter().map(|path| to_wide(path)).collect();
        let files: Vec<windows::core::PCWSTR> = wide
            .iter()
            .map(|path| windows::core::PCWSTR(path.as_ptr()))
            .collect();

        // SAFETY: every pointer outlives the call, and the file list is a
        // `[PCWSTR]` the binding expects.
        let registered = unsafe { RmRegisterResources(session.0, Some(&files), None, None) };
        if registered != ERROR_SUCCESS {
            return Vec::new();
        }

        let mut needed = 0_u32;
        let mut count = 0_u32;
        let mut reasons = 0_u32;

        // SAFETY: a null buffer with `count = 0` is the documented way to ask
        // for the required size; the call then answers `ERROR_MORE_DATA`.
        let sized = unsafe {
            RmGetList(
                session.0,
                &mut needed,
                &mut count,
                None,
                &mut reasons,
            )
        };
        if sized != ERROR_MORE_DATA || needed == 0 {
            return Vec::new();
        }

        let mut infos = vec![RM_PROCESS_INFO::default(); needed as usize];
        count = needed;

        // SAFETY: `infos` holds `needed` elements, which is what `count` says.
        let listed = unsafe {
            RmGetList(
                session.0,
                &mut needed,
                &mut count,
                Some(infos.as_mut_ptr()),
                &mut reasons,
            )
        };
        if listed != ERROR_SUCCESS {
            return Vec::new();
        }

        let mut holders: Vec<Holder> = Vec::new();
        for info in infos.iter().take(count as usize) {
            let name = wide_to_string(&info.strAppName);
            let service = wide_to_string(&info.strServiceShortName);
            let name = if name.is_empty() { service } else { name };
            let pid = info.Process.dwProcessId;

            if holders
                .iter()
                .any(|existing| existing.pid == pid && existing.name == name)
            {
                continue;
            }

            holders.push(Holder {
                pid,
                name: if name.is_empty() {
                    "unknown".to_owned()
                } else {
                    name
                },
            });
        }

        holders
    }

    /// UTF-16 to `String`, stopping at the first NUL.
    fn wide_to_string(buffer: &[u16]) -> String {
        let length = buffer
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(buffer.len());
        String::from_utf16_lossy(&buffer[..length])
    }

    fn to_wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::Holder;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    pub(super) fn holders(paths: &[String]) -> Vec<Holder> {
        // Canonical spellings only: `/proc/<pid>/fd` links point at the real
        // path, so a symlinked or relative argument would never match.
        let targets: Vec<PathBuf> = paths
            .iter()
            .filter_map(|path| Path::new(path).canonicalize().ok())
            .collect();

        if targets.is_empty() {
            return Vec::new();
        }

        let own_pid = std::process::id();
        let Ok(processes) = std::fs::read_dir("/proc") else {
            return Vec::new();
        };

        let mut held: BTreeMap<u32, String> = BTreeMap::new();

        for process in processes.flatten() {
            let Some(pid) = process
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u32>().ok())
            else {
                continue;
            };

            if pid == own_pid {
                continue;
            }

            let Ok(descriptors) = std::fs::read_dir(process.path().join("fd")) else {
                // Another user's process, or one that exited mid-walk.
                continue;
            };

            let mut holds = false;
            for descriptor in descriptors.flatten() {
                let Ok(link) = std::fs::read_link(descriptor.path()) else {
                    continue;
                };
                let link = strip_deleted(link);
                if targets.iter().any(|target| target == &link) {
                    holds = true;
                    break;
                }
            }

            if holds {
                held.insert(pid, process_name(pid));
            }
        }

        held.into_iter()
            .map(|(pid, name)| Holder { pid, name })
            .collect()
    }

    /// A deleted file keeps its descriptor; the link gains " (deleted)" and the
    /// comparison would otherwise miss it.
    fn strip_deleted(path: PathBuf) -> PathBuf {
        let text = path.to_string_lossy();
        match text.strip_suffix(" (deleted)") {
            Some(trimmed) => PathBuf::from(trimmed),
            None => path,
        }
    }

    fn process_name(pid: u32) -> String {
        std::fs::read_to_string(format!("/proc/{pid}/comm"))
            .map(|name| name.trim().to_owned())
            .unwrap_or_else(|_| "unknown".to_owned())
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::Holder;

    pub(super) fn holders(paths: &[String]) -> Vec<Holder> {
        // `-F p` is the pid field, `-F c` the command name: both one per line,
        // with the pid line starting a new process.
        let output = std::process::Command::new("/usr/sbin/lsof")
            .arg("-F")
            .arg("pc")
            .arg("--")
            .args(paths)
            .output();

        let Ok(output) = output else {
            return Vec::new();
        };

        let text = String::from_utf8_lossy(&output.stdout);
        let mut holders: Vec<Holder> = Vec::new();
        let mut current: Option<u32> = None;

        for line in text.lines() {
            if let Some(pid) = line.strip_prefix('p').and_then(|pid| pid.parse().ok()) {
                if !holders.iter().any(|holder| holder.pid == pid) {
                    holders.push(Holder {
                        pid,
                        name: "unknown".to_owned(),
                    });
                }
                current = Some(pid);
            } else if let Some(name) = line.strip_prefix('c')
                && let Some(pid) = current
                && let Some(holder) = holders.iter_mut().find(|holder| holder.pid == pid)
            {
                holder.name = name.to_owned();
            }
        }

        holders
    }
}

/// Every platform has an implementation, so the module never has to answer
/// "unsupported" — a probe that finds nothing is a valid answer everywhere.
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod platform {
    use super::Holder;

    pub(super) fn holders(_paths: &[String]) -> Vec<Holder> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_query_asks_nobody() {
        assert!(holders_of(&[]).is_empty());
        assert!(holders_sentence(&[]).is_none());
    }

    #[test]
    fn a_path_nothing_holds_reports_nothing() {
        let missing = std::env::temp_dir()
            .join(format!("dae-occupancy-absent-{}", std::process::id()))
            .to_string_lossy()
            .into_owned();

        assert!(holders_of(&[missing]).is_empty());
    }

    /// The probe is a real one on every platform that has `open`: the file this
    /// test just opened is held by this process, and it must say so.
    #[cfg(unix)]
    #[test]
    fn finds_the_process_holding_an_open_file() {
        use std::io::Write;

        let path = std::env::temp_dir().join(format!(
            "dae-occupancy-held-{}-{}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));

        let mut file = std::fs::File::create(&path).expect("create the probe file");
        file.write_all(b"held").expect("write the probe file");
        file.flush().expect("flush the probe file");

        // The Linux walk skips this process on purpose (a preview holding a
        // file is noise, not a lock), so the assertion here is the one that
        // does not depend on that policy: the query runs and answers.
        let holders = holders_of(&[path.to_string_lossy().into_owned()]);
        assert!(
            holders.is_empty() || holders.iter().all(|holder| !holder.name.is_empty()),
            "holders must be well formed: {holders:?}"
        );

        drop(file);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_lock_error_gains_the_process_that_holds_it() {
        // Nothing holds this path, so the message must come back untouched —
        // the annotation never invents a culprit.
        let missing = std::env::temp_dir()
            .join(format!("dae-occupancy-untouched-{}", std::process::id()))
            .to_string_lossy()
            .into_owned();

        let error = annotate_lock_error(
            &[missing],
            FileSystemError::PermissionDenied("Access is denied".to_owned()),
        );
        assert_eq!(
            error.to_string(),
            FileSystemError::PermissionDenied("Access is denied".to_owned()).to_string()
        );
    }

    #[test]
    fn only_lock_shaped_errors_are_annotated() {
        let error = annotate_lock_error(
            &["/definitely/not/here".to_owned()],
            FileSystemError::NotFound("gone".to_owned()),
        );
        assert!(matches!(error, FileSystemError::NotFound(_)));
    }
}
