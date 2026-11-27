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

use qtbridge::{QmlElement, QmlObject, qobject};
use dae_core::update::{self, AvailableUpdate, UpdateError};

use crate::runtime;

macro_rules! notifying_setters {
    ($($setter:ident($ty:ty) => $field:ident, $notify:ident;)*) => {
        $(#[allow(dead_code)]
        fn $setter(&mut self, value: $ty) {
            self.$field = value;
            self.$notify();
        })*
    };
}

#[qobject(NoQmlElement, ConvertToCamelCase)]
mod updater_impl {
    use super::*;

    pub struct Updater {
        supported: bool,
        packaged: bool,
        current_version: String,
        checking: bool,
        checked: bool,
        available: bool,
        latest_version: String,
        notes: String,
        downloading: bool,
        install_progress: i32,
        restarting: bool,
        error: String,
        update: Option<AvailableUpdate>,
    }

    impl Default for Updater {
        fn default() -> Self {
            Self {
                supported: update::updates_enabled(),
                packaged: dae_core::package::is_packaged(),
                current_version: env!("CARGO_PKG_VERSION").to_owned(),
                checking: false,
                checked: false,
                available: false,
                latest_version: String::new(),
                notes: String::new(),
                downloading: false,
                install_progress: 0,
                restarting: false,
                error: String::new(),
                update: None,
            }
        }
    }


    impl Updater {
        qproperty!("supported", Member = supported, Notify = supported_changed);
        qproperty!("packaged", Member = packaged, Notify = packaged_changed);
        qproperty!("currentVersion", Member = current_version, Notify = current_version_changed);
        qproperty!("checking", Member = checking, Notify = checking_changed);
        qproperty!("checked", Member = checked, Notify = checked_changed);
        qproperty!("available", Member = available, Notify = available_changed);
        qproperty!("latestVersion", Member = latest_version, Notify = latest_version_changed);
        qproperty!("notes", Member = notes, Notify = notes_changed);
        qproperty!("downloading", Member = downloading, Notify = downloading_changed);
        qproperty!("installProgress", Member = install_progress, Notify = install_progress_changed);
        qproperty!("restarting", Member = restarting, Notify = restarting_changed);
        qproperty!("error", Member = error, Notify = error_changed);

        #[qsignal]
        fn supported_changed(&mut self);
        #[qsignal]
        fn packaged_changed(&mut self);
        #[qsignal]
        fn current_version_changed(&mut self);
        #[qsignal]
        fn checking_changed(&mut self);
        #[qsignal]
        fn checked_changed(&mut self);
        #[qsignal]
        fn available_changed(&mut self);
        #[qsignal]
        fn latest_version_changed(&mut self);
        #[qsignal]
        fn notes_changed(&mut self);
        #[qsignal]
        fn downloading_changed(&mut self);
        #[qsignal]
        fn install_progress_changed(&mut self);
        #[qsignal]
        fn restarting_changed(&mut self);
        #[qsignal]
        fn error_changed(&mut self);

        notifying_setters! {
            set_supported(bool) => supported, supported_changed;
            set_packaged(bool) => packaged, packaged_changed;
            set_current_version(String) => current_version, current_version_changed;
            set_checking(bool) => checking, checking_changed;
            set_checked(bool) => checked, checked_changed;
            set_available(bool) => available, available_changed;
            set_latest_version(String) => latest_version, latest_version_changed;
            set_notes(String) => notes, notes_changed;
            set_downloading(bool) => downloading, downloading_changed;
            set_install_progress(i32) => install_progress, install_progress_changed;
            set_restarting(bool) => restarting, restarting_changed;
            set_error(String) => error, error_changed;
        }

        #[qsignal]
        fn restart_requested(&mut self);


        #[qslot]

        fn check(&mut self, manual: bool) {
            if self.checking || self.downloading {
                return;
            }
            if !self.supported {
                // A packaged install is not a broken configuration: the tree it
                // runs from belongs to the system installer, so the dialog gets the
                // "updates arrive as a package" note instead of an error.
                if manual && !self.packaged {
                    self.set_error(
                        "This build does not support automatic updates".to_owned(),
                    );
                }
                self.set_checked(true);
                return;
            }

            self.set_checking(true);
            self.set_checked(false);
            self.set_error(String::new());

            let thread = self.get_thread_handle();
            runtime::spawn(async move {
                let result =
                    update::check_for_update(&update::feed_url(), env!("CARGO_PKG_VERSION")).await;
                thread.invoke_mut(move |updater| {
                    updater.apply_check(result, manual);
                });
            });
        }

        #[qslot]

        fn download_and_install(&mut self) {
            if self.downloading || self.checking {
                return;
            }
            let Some(update) = self.update.clone() else {
                self.set_error("No update is available to install".to_owned());
                return;
            };

            self.set_downloading(true);
            self.set_install_progress(0);
            self.set_restarting(false);
            self.set_error(String::new());

            let thread = self.get_thread_handle();
            let progress_thread = self.get_thread_handle();
            runtime::spawn(async move {
                let progress = move |percent: u8| {
                    progress_thread.invoke_mut(move |updater| {
                        updater.set_install_progress(percent as i32);
                    });
                };
                let result = update::download_and_install(&update, progress).await;
                thread.invoke_mut(move |updater| updater.apply_install(result));
            });
        }

        fn apply_check(
            &mut self,
            result: Result<Option<AvailableUpdate>, UpdateError>,
            manual: bool,
        ) {
            self.set_checking(false);
            self.set_checked(true);

            match result {
                Ok(Some(update)) => {
                    self.set_latest_version(update.version_text().to_owned());
                    self.set_notes(update.notes.to_owned());
                    self.update = Some(update);
                    self.set_available(true);
                    self.set_error(String::new());
                }
                Ok(None) => {
                    self.update = None;
                    self.set_available(false);
                    self.set_latest_version(String::new());
                    self.set_notes(String::new());
                    self.set_error(String::new());
                }
                Err(error) => {
                    if !manual {
                        log::warn!(target: "dae::update", "update check failed: {error}");
                    }
                    let message = friendly_update_error(&error);
                    self.set_error(message.to_owned());
                }
            }
        }

        fn apply_install(&mut self, result: Result<(), UpdateError>) {
            self.set_downloading(false);
            match result {
                Ok(()) => {
                    self.set_install_progress(100);
                    self.set_error(String::new());
                    self.set_restarting(true);
                    self.restart_requested();
                }
                Err(error) => {
                    log::warn!(target: "dae::update", "update install failed: {error}");
                    let message = friendly_update_error(&error);
                    self.set_error(message.to_owned());
                }
            }
        }
    }
}

pub use updater_impl::Updater;

impl QmlElement for Updater {
    const URI: &str = "dae";
    const ELEMENT_NAME: &str = "Updater";
    const MINOR_VERSION: u8 = 0;
    const MAJOR_VERSION: u8 = 1;
    const IS_SINGLETON: bool = true;
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
