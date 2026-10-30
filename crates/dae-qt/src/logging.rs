//! File-backed logging for the Qt shell.
//!
//! The Tauri shell got this from `tauri-plugin-log`: 4 MB rotation, five files
//! kept, local timestamps, and a panic hook that turns a vanished window into a
//! line in a file (`src-tauri/src/lib.rs`). The Qt shell shipped without any of
//! it: `config.rs` and `system_watch.rs` wrote their startup and failure lines
//! to stderr, and release builds are `windows_subsystem = "windows"`, so on the
//! machine that actually broke there was nothing to read. `dae-core` already
//! logs through `log`; this module is the sink those messages were missing.
//!
//! What lands in the file:
//!
//! * every `log::info!` and above from `dae-core`, `dae-qt` and this binary —
//!   the same crate, so one `set_boxed_logger` covers all of them;
//! * panics, through the hook installed by [`init`], at `error` level with the
//!   `dae::panic` target;
//! * one startup line naming the version, the platform and the level in force.
//!
//! `DAE_LOG` overrides the level (`off`/`error`/`warn`/`info`/`debug`/`trace`,
//! default `info`). Debug builds also mirror every line to stderr, because that
//! is where the end-to-end checks read them from; release builds write the file
//! only, since a GUI subsystem binary has no stderr to write to.
//!
//! Qt's own diagnostics (`qWarning`) are **not** captured: they go through
//! `qInstallMessageHandler`, which would mean doing this half in C++. The
//! directory and the format match the Tauri shell, so a support session still
//! has one place to look.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use log::{Level, LevelFilter, Log, Metadata, Record};

/// Size at which the live log file rotates.
///
/// The Tauri shell raised this from the plugin's 40 kB default for the same
/// reason: a couple of directory listings would otherwise push the failure that
/// mattered out of the file before anyone read it.
pub const LOG_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// Rotated files kept beside the live one. Five bounds the directory at five
/// times [`LOG_FILE_BYTES`].
pub const LOG_FILES_KEPT: usize = 5;

/// Base name of the live file and its rotations (`dae.log`, `dae.1.log`, …).
const FILE_STEM: &str = "dae";

/// Installs the logger and the panic hook. Safe to call more than once.
pub fn init() {
    static ONCE: OnceLock<()> = OnceLock::new();
    if ONCE.set(()).is_err() {
        return;
    }

    let directory = app_log_directory();
    let level = level_from_env();

    match Logger::open(directory.clone(), LOG_FILE_BYTES, level) {
        Ok(logger) => {
            if log::set_boxed_logger(Box::new(logger)).is_ok() {
                log::set_max_level(level);
            } else {
                eprintln!("dae: a logger is already installed; file logging disabled");
                return;
            }
        }
        Err(error) => {
            eprintln!(
                "dae: could not write {}: {error}",
                directory.join(format!("{FILE_STEM}.log")).display()
            );
            return;
        }
    }

    // After the logger, so the hook has somewhere to write.
    install_panic_hook();

    log::info!(
        target: "dae",
        "dae {} starting on {} ({}), logging at {level} to {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        app_log_directory().join(format!("{FILE_STEM}.log")).display()
    );
}

/// The directory both shells write to.
///
/// Identifier-scoped and named `logs`, matching the Tauri shell's `appLogDir`
/// convention: Windows `%LOCALAPPDATA%`, macOS `~/Library/Logs`, Linux
/// `$XDG_DATA_HOME` (or `~/.local/share`). A user upgrading from the Tauri
/// build finds the new lines in the directory they already know.
pub fn app_log_directory() -> PathBuf {
    #[cfg(target_os = "windows")]
    if let Some(base) = std::env::var_os("LOCALAPPDATA") {
        return std::path::PathBuf::from(base)
            .join(dae_core::paths::APP_IDENTIFIER)
            .join("logs");
    }

    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        return std::path::PathBuf::from(home)
            .join("Library")
            .join("Logs")
            .join(dae_core::paths::APP_IDENTIFIER);
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let base = std::env::var_os("XDG_DATA_HOME")
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|home| std::path::PathBuf::from(home).join(".local").join("share"))
            });
        if let Some(base) = base {
            return base.join(dae_core::paths::APP_IDENTIFIER).join("logs");
        }
    }

    dae_core::paths::config_dir()
        .map(|directory| directory.join("logs"))
        .unwrap_or_else(|_| std::env::temp_dir().join("dae-logs"))
}

/// The live log file, for the settings page and for support instructions.
pub fn log_file_path() -> PathBuf {
    app_log_directory().join(format!("{FILE_STEM}.log"))
}

/// `DAE_LOG` wins when it names a level; anything else falls back to `info`.
fn level_from_env() -> LevelFilter {
    std::env::var("DAE_LOG")
        .ok()
        .and_then(|value| parse_level(&value))
        .unwrap_or(LevelFilter::Info)
}

fn parse_level(value: &str) -> Option<LevelFilter> {
    match value.trim().to_ascii_lowercase().as_str() {
        "off" => Some(LevelFilter::Off),
        "error" => Some(LevelFilter::Error),
        "warn" => Some(LevelFilter::Warn),
        "info" => Some(LevelFilter::Info),
        "debug" => Some(LevelFilter::Debug),
        "trace" => Some(LevelFilter::Trace),
        _ => None,
    }
}

/// Routes panics into the log file.
///
/// The release profile sets `panic = "abort"`, so a panic ends the process
/// without unwinding, and the GUI subsystem build has no console to fall back
/// on. Without this hook a crash is a window that vanishes and nothing else.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("a panic payload that is not a string");
        let location = info.location().map_or_else(
            || "an unknown location".to_owned(),
            |location| format!("{}:{}", location.file(), location.line()),
        );

        log::error!(target: "dae::panic", "Panicked at {location}: {message}");

        // Keeps the default handler's stderr report and, under `RUST_BACKTRACE`,
        // its backtrace — free in release, where nothing is attached to stderr.
        previous(info);
    }));
}

struct State {
    file: Option<File>,
    len: u64,
}

struct Logger {
    directory: PathBuf,
    path: PathBuf,
    max_bytes: u64,
    level: LevelFilter,
    state: Mutex<State>,
}

impl Logger {
    /// Opens the live file, creating the directory if the shell has never run.
    fn open(directory: PathBuf, max_bytes: u64, level: LevelFilter) -> std::io::Result<Self> {
        fs::create_dir_all(&directory)?;
        let path = directory.join(format!("{FILE_STEM}.log"));
        let (file, len) = open_append(&path)?;
        Ok(Self {
            directory,
            path,
            max_bytes,
            level,
            state: Mutex::new(State {
                file: Some(file),
                len,
            }),
        })
    }

    /// Rotates when the next line would cross the limit.
    ///
    /// The handle has to be dropped first: Windows refuses the rename while the
    /// file is open, and `File` keeps no sharing flag for it.
    fn rotate(&self, state: &mut State) {
        state.file.take();
        if rotate_files(&self.directory, &self.path).is_ok()
            && let Ok((file, len)) = open_append(&self.path)
        {
            state.file = Some(file);
            state.len = len;
        }
    }
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let line = format_line(record);

        // Debug builds keep the console output the end-to-end checks read.
        // Tests write the file only: `stderr` here is a direct write, which the
        // test harness does not capture, so mirroring would bury the summary.
        if cfg!(debug_assertions) && !cfg!(test) {
            let _ = std::io::stderr().write_all(line.as_bytes());
        }

        let Ok(mut state) = self.state.lock() else {
            return;
        };

        if state.file.is_none() || state.len + line.len() as u64 > self.max_bytes {
            self.rotate(&mut state);
        }

        if let Some(file) = state.file.as_mut()
            && file.write_all(line.as_bytes()).is_ok()
        {
            state.len += line.len() as u64;
        }
    }

    fn flush(&self) {
        if let Ok(mut state) = self.state.lock()
            && let Some(file) = state.file.as_mut()
        {
            let _ = file.flush();
        }
    }
}

fn format_line(record: &Record) -> String {
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let level = match record.level() {
        Level::Error => "ERROR",
        Level::Warn => "WARN",
        Level::Info => "INFO",
        Level::Debug => "DEBUG",
        Level::Trace => "TRACE",
    };
    format!(
        "[{timestamp}][{level}][{}] {}\n",
        record.target(),
        record.args()
    )
}

fn open_append(path: &Path) -> std::io::Result<(File, u64)> {
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    let len = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    Ok((file, len))
}

/// `dae.log` → `dae.1.log` → … → `dae.5.log`, dropping the oldest.
fn rotate_files(directory: &Path, path: &Path) -> std::io::Result<()> {
    let _ = fs::remove_file(directory.join(format!("{FILE_STEM}.{LOG_FILES_KEPT}.log")));

    for index in (1..LOG_FILES_KEPT).rev() {
        let from = directory.join(format!("{FILE_STEM}.{index}.log"));
        if from.exists() {
            let to = directory.join(format!("{FILE_STEM}.{}.log", index + 1));
            let _ = fs::rename(&from, &to);
        }
    }

    if path.exists() {
        fs::rename(path, directory.join(format!("{FILE_STEM}.1.log")))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("dae-logging-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        path
    }

    fn logger(directory: &Path, max_bytes: u64) -> Logger {
        Logger::open(directory.to_path_buf(), max_bytes, LevelFilter::Info).expect("logger")
    }

    fn write(logger: &Logger, level: Level, message: &str) {
        logger.log(
            &Record::builder()
                .args(format_args!("{message}"))
                .level(level)
                .target("dae::test")
                .build(),
        );
    }

    #[test]
    fn a_line_carries_its_level_target_and_message() {
        let directory = scratch("format");
        let logger = logger(&directory, LOG_FILE_BYTES);
        write(&logger, Level::Info, "profile read");

        let text = fs::read_to_string(directory.join("dae.log")).expect("log file");
        assert!(text.ends_with("profile read\n"), "{text}");
        assert!(text.contains("[INFO][dae::test]"), "{text}");
        assert!(text.starts_with('['), "{text}");

        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn rotation_keeps_the_newest_lines_and_drops_the_oldest_file() {
        let directory = scratch("rotate");
        let logger = logger(&directory, 200);

        for index in 0..40 {
            write(
                &logger,
                Level::Info,
                &format!("line {index:02} with some padding"),
            );
        }
        logger.flush();
        drop(logger);

        let live = fs::read_to_string(directory.join("dae.log")).expect("live file");
        assert!(live.len() as u64 <= 200, "live file exceeded the limit");
        assert!(directory.join("dae.1.log").exists(), "no rotation happened");
        assert!(
            !directory.join("dae.6.log").exists(),
            "more files kept than configured"
        );

        let kept: Vec<u64> = (1..=LOG_FILES_KEPT)
            .filter(|index| directory.join(format!("dae.{index}.log")).exists())
            .map(|index| index as u64)
            .collect();
        assert!(!kept.is_empty(), "rotation left no history");

        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn below_info_is_dropped() {
        let directory = scratch("level");
        let logger = logger(&directory, LOG_FILE_BYTES);
        write(&logger, Level::Debug, "noise");
        write(&logger, Level::Info, "kept");

        let text = fs::read_to_string(directory.join("dae.log")).expect("log file");
        assert!(!text.contains("noise"), "{text}");
        assert!(text.contains("kept"), "{text}");

        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn dae_log_names_a_level_and_garbage_falls_back() {
        assert_eq!(parse_level("DEBUG"), Some(LevelFilter::Debug));
        assert_eq!(parse_level(" warn "), Some(LevelFilter::Warn));
        assert_eq!(parse_level("off"), Some(LevelFilter::Off));
        assert_eq!(parse_level("verbose"), None);
    }

    #[test]
    fn the_log_directory_keeps_the_tauri_convention() {
        let directory = app_log_directory();
        assert!(directory.ends_with("logs"), "{}", directory.display());
        assert!(
            directory
                .components()
                .any(|part| part.as_os_str() == dae_core::paths::APP_IDENTIFIER),
            "{} is not identifier-scoped",
            directory.display()
        );

        #[cfg(target_os = "macos")]
        assert!(
            directory.to_string_lossy().contains("Library/Logs"),
            "{}",
            directory.display()
        );
    }
}
