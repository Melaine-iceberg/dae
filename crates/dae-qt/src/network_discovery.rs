//! QML surface for `dae_core::fs::discovery`.
//!
//! The core owns the mDNS probe; this object turns it into one JSON string the
//! sidebar parses, plus a `busy` flag for the refresh affordance. The probe is
//! time-bounded and runs on the shared runtime's blocking pool, so a network
//! that never answers cannot hold the UI thread.
//!
//! `scanned` exists so a second window (or a re-created sidebar) does not
//! restart the probe: the answer is process-wide and only changes when someone
//! asks for it again.

use std::pin::Pin;
use std::time::Duration;

use cxx_qt::Threading;
use cxx_qt_lib::QString;
use dae_core::fs::discovery;

use crate::runtime;

/// Long enough for a busy network to answer, short enough that a refresh is
/// not something the user waits on.
const SCAN_TIMEOUT: Duration = Duration::from_millis(1500);

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    unsafe extern "RustQt" {
        /// QML usage: `NetworkDiscovery.servicesJson`, `.busy`, `.refresh()`.
        ///
        /// `servicesJson` is a JSON array of
        /// `{ protocol, name, host, port, addresses }`. Empty until a scan
        /// finishes; the sidebar hides its section while it is empty.
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(QString, services_json, cxx_name = "servicesJson")]
        #[qproperty(bool, busy)]
        #[qproperty(bool, scanned)]
        type NetworkDiscovery = super::NetworkDiscoveryRust;

        /// Browses the network for SMB/SFTP/WebDAV advertisements. A second
        /// call while a scan is running is ignored; the list replaces itself
        /// when the answer arrives.
        #[qinvokable]
        fn refresh(self: Pin<&mut NetworkDiscovery>);
    }

    impl cxx_qt::Threading for NetworkDiscovery {}
}

pub struct NetworkDiscoveryRust {
    services_json: QString,
    busy: bool,
    scanned: bool,
}

impl Default for NetworkDiscoveryRust {
    fn default() -> Self {
        Self {
            services_json: QString::from("[]"),
            busy: false,
            scanned: false,
        }
    }
}

impl qobject::NetworkDiscovery {
    fn refresh(mut self: Pin<&mut Self>) {
        if self.busy {
            return;
        }
        self.as_mut().set_busy(true);
        self.as_mut().set_scanned(true);

        let thread = self.as_mut().qt_thread();
        runtime::spawn(async move {
            let services = tokio::task::spawn_blocking(|| discovery::discover(SCAN_TIMEOUT))
                .await
                .unwrap_or_default();
            let json = serde_json::to_string(&services).unwrap_or_else(|_| "[]".to_owned());
            let _ = thread.queue(move |mut discovery| {
                discovery
                    .as_mut()
                    .set_services_json(QString::from(json.as_str()));
                discovery.as_mut().set_busy(false);
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_discovery_has_nothing_to_show() {
        let state = NetworkDiscoveryRust::default();
        assert_eq!(state.services_json.to_string(), "[]");
        assert!(!state.busy);
        assert!(!state.scanned);
    }
}
