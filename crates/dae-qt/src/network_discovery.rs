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
//!
//! Ported from CXX-Qt to Qt Bridge. The answer travels back through a
//! [`QThreadHandle`](qtbridge::QThreadHandle), which replaces
//! `cxx_qt::Threading::qt_thread()`; a result delivered after the singleton is
//! gone is dropped instead of reaching freed memory.

use std::time::Duration;

use dae_core::fs::discovery;
use qtbridge::{QmlElement, QmlObject, qobject};

use crate::runtime;

/// Long enough for a busy network to answer, short enough that a refresh is
/// not something the user waits on.
const SCAN_TIMEOUT: Duration = Duration::from_millis(1500);

/// QML usage: `NetworkDiscovery.servicesJson`, `.busy`, `.refresh()`.
///
/// `servicesJson` is a JSON array of
/// `{ protocol, name, host, port, addresses }`. Empty until a scan
/// finishes; the sidebar hides its section while it is empty.
pub struct NetworkDiscovery {
    services_json: String,
    busy: bool,
    scanned: bool,
}

impl Default for NetworkDiscovery {
    fn default() -> Self {
        Self {
            services_json: "[]".to_owned(),
            busy: false,
            scanned: false,
        }
    }
}

#[qobject(NoQmlElement)]
impl NetworkDiscovery {
    qproperty!(
        "servicesJson",
        Member = services_json,
        Notify = services_json_changed
    );
    qproperty!("busy", Member = busy, Notify = busy_changed);
    qproperty!("scanned", Member = scanned, Notify = scanned_changed);

    #[qsignal]
    fn services_json_changed(&mut self);
    #[qsignal]
    fn busy_changed(&mut self);
    #[qsignal]
    fn scanned_changed(&mut self);

    /// Browses the network for SMB/SFTP/WebDAV advertisements. A second
    /// call while a scan is running is ignored; the list replaces itself
    /// when the answer arrives.
    #[qslot]
    fn refresh(&mut self) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.busy_changed();
        self.scanned = true;
        self.scanned_changed();

        let handle = self.get_thread_handle();
        runtime::spawn(async move {
            let services = tokio::task::spawn_blocking(|| discovery::discover(SCAN_TIMEOUT))
                .await
                .unwrap_or_default();
            let json = serde_json::to_string(&services).unwrap_or_else(|_| "[]".to_owned());
            handle.invoke_mut(move |discovery| {
                discovery.services_json = json;
                discovery.services_json_changed();
                discovery.busy = false;
                discovery.busy_changed();
            });
        });
    }
}

impl QmlElement for NetworkDiscovery {
    const URI: &str = "dae";
    const ELEMENT_NAME: &str = "NetworkDiscovery";
    const MINOR_VERSION: u8 = 0;
    const MAJOR_VERSION: u8 = 1;
    const IS_SINGLETON: bool = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_discovery_has_nothing_to_show() {
        let state = NetworkDiscovery::default();
        assert_eq!(state.services_json, "[]");
        assert!(!state.busy);
        assert!(!state.scanned);
    }
}
