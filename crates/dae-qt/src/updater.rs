//! QML surface for `dae_core::update`.
//!
//! The core owns the feed, signature check, archive and installer script. This
//! object only turns the long-running flow into Qt properties: a small status
//! item in the status bar opens [`UpdateDialog`], and a completed install
//! emits `restartRequested` so QML can quit before the detached installer
//! replaces the mapped Qt libraries.
//!
//! Auto-checks happen once per launch after the first window settles. A found
//! update is advertised, not installed silently: restarting a file manager
//! under someone's hands without a click is worse than one extra click.

use core::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use dae_core::update::{self, AvailableUpdate, UpdateError};

use crate::runtime;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    unsafe extern "RustQt" {
        /// Process-wide update state, reachable from QML as `Updater`.
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(bool, supported)]
        #[qproperty(QString, current_version, cxx_name = "currentVersion")]
        #[qproperty(bool, checking)]
        #[qproperty(bool, checked)]
        #[qproperty(bool, available)]
        #[qproperty(QString, latest_version, cxx_name = "latestVersion")]
        #[qproperty(QString, notes)]
        #[qproperty(bool, downloading)]
        #[qproperty(i32, install_progress, cxx_name = "installProgress")]
        #[qproperty(bool, restarting)]
        #[qproperty(QString, error)]
        type Updater = super::UpdaterRust;

        /// Checks the signed feed. `manual` checks surface a failure in the
        /// dialog; the startup check only logs it, so being offline does not
        /// greet the user with an error.
        #[qinvokable]
        fn check(self: Pin<&mut Updater>, manual: bool);

        /// Downloads, verifies and starts the installer. The process must exit
        /// when `restartRequested` arrives.
        #[qinvokable]
        #[cxx_name = "downloadAndInstall"]
        fn download_and_install(self: Pin<&mut Updater>);

        #[qsignal]
        #[cxx_name = "restartRequested"]
        fn restart_requested(self: Pin<&mut Updater>);
    }

    impl cxx_qt::Threading for Updater {}
}

pub struct UpdaterRust {
    supported: bool,
    current_version: QString,
    checking: bool,
    checked: bool,
    available: bool,
    latest_version: QString,
    notes: QString,
    downloading: bool,
    install_progress: i32,
    restarting: bool,
    error: QString,
    update: Option<AvailableUpdate>,
}

impl Default for UpdaterRust {
    fn default() -> Self {
        Self {
            supported: update::updates_enabled(),
            current_version: QString::from(env!("CARGO_PKG_VERSION")),
            checking: false,
            checked: false,
            available: false,
            latest_version: QString::default(),
            notes: QString::default(),
            downloading: false,
            install_progress: 0,
            restarting: false,
            error: QString::default(),
            update: None,
        }
    }
}

impl qobject::Updater {
    fn check(mut self: Pin<&mut Self>, manual: bool) {
        if self.checking || self.downloading {
            return;
        }
        if !self.supported {
            if manual {
                self.as_mut().set_error(QString::from(
                    "This build does not support automatic updates",
                ));
                self.as_mut().set_checked(true);
            }
            return;
        }

        self.as_mut().set_checking(true);
        self.as_mut().set_checked(false);
        self.as_mut().set_error(QString::default());

        let thread = self.as_mut().qt_thread();
        runtime::spawn(async move {
            let result =
                update::check_for_update(&update::feed_url(), env!("CARGO_PKG_VERSION")).await;
            let _ = thread.queue(move |mut updater| {
                updater.as_mut().apply_check(result, manual);
            });
        });
    }

    fn download_and_install(mut self: Pin<&mut Self>) {
        if self.downloading || self.checking {
            return;
        }
        let Some(update) = self.update.clone() else {
            self.as_mut()
                .set_error(QString::from("No update is available to install"));
            return;
        };

        self.as_mut().set_downloading(true);
        self.as_mut().set_install_progress(0);
        self.as_mut().set_restarting(false);
        self.as_mut().set_error(QString::default());

        let thread = self.as_mut().qt_thread();
        let progress_thread = thread.clone();
        runtime::spawn(async move {
            let progress = move |percent: u8| {
                let _ = progress_thread.queue(move |mut updater| {
                    updater.as_mut().set_install_progress(percent as i32);
                });
            };
            let result = update::download_and_install(&update, progress).await;
            let _ = thread.queue(move |mut updater| updater.as_mut().apply_install(result));
        });
    }
}

impl qobject::Updater {
    fn apply_check(
        mut self: Pin<&mut Self>,
        result: Result<Option<AvailableUpdate>, UpdateError>,
        manual: bool,
    ) {
        self.as_mut().set_checking(false);
        self.as_mut().set_checked(true);

        match result {
            Ok(Some(update)) => {
                self.as_mut()
                    .set_latest_version(QString::from(update.version_text().as_str()));
                self.as_mut()
                    .set_notes(QString::from(update.notes.as_str()));
                self.as_mut().rust_mut().update = Some(update);
                self.as_mut().set_available(true);
                self.as_mut().set_error(QString::default());
            }
            Ok(None) => {
                self.as_mut().rust_mut().update = None;
                self.as_mut().set_available(false);
                self.as_mut().set_latest_version(QString::default());
                self.as_mut().set_notes(QString::default());
                self.as_mut().set_error(QString::default());
            }
            Err(error) => {
                if !manual {
                    log::warn!(target: "dae::update", "update check failed: {error}");
                }
                let message = friendly_update_error(&error);
                self.as_mut().set_error(QString::from(message.as_str()));
            }
        }
    }

    fn apply_install(mut self: Pin<&mut Self>, result: Result<(), UpdateError>) {
        self.as_mut().set_downloading(false);
        match result {
            Ok(()) => {
                self.as_mut().set_install_progress(100);
                self.as_mut().set_error(QString::default());
                self.as_mut().set_restarting(true);
                self.as_mut().restart_requested();
            }
            Err(error) => {
                log::warn!(target: "dae::update", "update install failed: {error}");
                let message = friendly_update_error(&error);
                self.as_mut().set_error(QString::from(message.as_str()));
            }
        }
    }
}

fn friendly_update_error(error: &UpdateError) -> String {
    match error {
        UpdateError::Feed(detail) => {
            format!("Update check failed: {}", short_detail(detail))
        }
        UpdateError::Download(detail) => {
            format!("Update download failed: {}", short_detail(detail))
        }
        UpdateError::InvalidFeed(detail) => {
            format!("Invalid update feed: {}", short_detail(detail))
        }
        UpdateError::PlatformMissing(_) => {
            "No update has been released for this platform".to_owned()
        }
        UpdateError::Signature(_) => {
            "Update package signature verification failed. Installation was blocked.".to_owned()
        }
        UpdateError::Archive(_) => {
            "The update package is incomplete. The current version was kept.".to_owned()
        }
        UpdateError::Install(detail) => {
            format!(
                "Could not start the update installer: {}",
                short_detail(detail)
            )
        }
        UpdateError::NotInstalled => "This build has no updatable installation location".to_owned(),
    }
}

fn short_detail(detail: &str) -> String {
    let first_line = detail.lines().next().unwrap_or("").trim();
    if first_line.chars().count() <= 140 {
        return first_line.to_owned();
    }
    let mut shortened: String = first_line.chars().take(139).collect();
    shortened.push('…');
    shortened
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_errors_keep_the_first_line_and_stay_short() {
        let error = UpdateError::Download("timed out\nwhile reading".to_owned());
        assert_eq!(
            friendly_update_error(&error),
            "Update download failed: timed out"
        );
        let error = UpdateError::Feed("connection reset".to_owned());
        assert_eq!(
            friendly_update_error(&error),
            "Update check failed: connection reset"
        );

        let long = "x".repeat(300);
        assert_eq!(short_detail(&long).chars().count(), 140);
    }

    #[test]
    fn signature_failures_do_not_echo_the_parser_error() {
        let error = UpdateError::Signature("invalid base64".to_owned());
        assert_eq!(
            friendly_update_error(&error),
            "Update package signature verification failed. Installation was blocked."
        );
    }
}
