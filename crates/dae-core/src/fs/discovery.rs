//! mDNS discovery of the network file services the connect dialog can open.
//!
//! Browsing is a **one-shot, time-bounded probe**, not a daemon that lives for
//! the process lifetime: the sidebar asks for a list when it is built, and a
//! few seconds of multicast after that would only keep the radio awake for a
//! list nobody is looking at.
//!
//! Results fill host and port only. TXT records are deliberately ignored:
//! whatever they say is written by the advertiser, and a file manager must not
//! turn a remote string into credentials or into a path it opens without the
//! user typing it.
//!
//! Failure to start the daemon (no multicast, a sandbox without sockets) is not
//! an error: discovery is an extra, so the answer is an empty list and a debug
//! log line.

use mdns_sd::{Receiver, RecvTimeoutError, ServiceDaemon, ServiceEvent};
use serde::Serialize;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// The advertised service types mapped to the protocol name the connect dialog
/// uses (`smb`, `sftp`, `webdav`).
const SERVICES: [(&str, &str); 3] = [
    ("_smb._tcp.local.", "smb"),
    ("_sftp-ssh._tcp.local.", "sftp"),
    ("_webdav._tcp.local.", "webdav"),
];

/// One advertised service, reduced to what the connect dialog needs to fill
/// itself in.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredService {
    /// `smb`, `sftp` or `webdav`.
    pub protocol: String,
    /// The instance name the advertiser published ("Living Room NAS").
    pub name: String,
    /// The advertised host name without its trailing dot, or the first
    /// address when the advertiser published no name.
    pub host: String,
    pub port: u16,
    /// Every address the service resolved to, sorted.
    pub addresses: Vec<String>,
}

/// One resolved advertisement before de-duplication.
struct Raw {
    protocol: &'static str,
    fullname: String,
    hostname: String,
    port: u16,
    addresses: Vec<String>,
}

/// Browses the three service types for at most `timeout` and returns what
/// answered, de-duplicated by protocol + host + port. Blocking; the caller runs
/// it off the GUI thread.
pub fn discover(timeout: Duration) -> Vec<DiscoveredService> {
    let Ok(daemon) = ServiceDaemon::new() else {
        log::debug!(target: "dae::discovery", "the mDNS daemon could not start");
        return Vec::new();
    };

    let mut channels: Vec<(&'static str, Receiver<ServiceEvent>)> = Vec::new();
    for (service, protocol) in SERVICES {
        match daemon.browse(service) {
            Ok(receiver) => channels.push((protocol, receiver)),
            Err(error) => {
                log::debug!(target: "dae::discovery", "cannot browse {service}: {error}");
            }
        }
    }
    if channels.is_empty() {
        let _ = daemon.shutdown();
        return Vec::new();
    }

    let deadline = Instant::now() + timeout;
    let mut raw: Vec<Raw> = Vec::new();
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        let slice = remaining.min(Duration::from_millis(120));
        for (protocol, receiver) in &channels {
            match receiver.recv_timeout(slice) {
                Ok(ServiceEvent::ServiceResolved(service)) => raw.push(Raw {
                    protocol,
                    fullname: service.get_fullname().to_owned(),
                    hostname: service.get_hostname().to_owned(),
                    port: service.get_port(),
                    addresses: service
                        .get_addresses()
                        .iter()
                        .map(|address| address.to_string())
                        .collect(),
                }),
                // Found/Removed/SearchStarted/…: nothing to add, keep waiting.
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {}
            }
        }
    }
    let _ = daemon.shutdown();
    normalise(raw)
}

/// Merges duplicate advertisements into one row per protocol + host + port.
fn normalise(raw: Vec<Raw>) -> Vec<DiscoveredService> {
    let mut merged: BTreeMap<(u8, String, u16), DiscoveredService> = BTreeMap::new();
    for item in raw {
        if item.port == 0 {
            continue;
        }
        let mut addresses = item.addresses;
        addresses.sort();
        addresses.dedup();
        let hostname = item.hostname.trim_end_matches('.');
        let host = if hostname.is_empty() {
            match addresses.first() {
                Some(address) => address.clone(),
                None => continue,
            }
        } else {
            hostname.to_owned()
        };
        let service = DiscoveredService {
            protocol: item.protocol.to_owned(),
            name: instance_name(&item.fullname),
            host,
            port: item.port,
            addresses,
        };
        merged
            .entry(service.key())
            .and_modify(|existing| {
                for address in &service.addresses {
                    if !existing.addresses.contains(address) {
                        existing.addresses.push(address.clone());
                    }
                }
                existing.addresses.sort();
                if existing.name.is_empty() {
                    existing.name = service.name.clone();
                }
            })
            .or_insert(service);
    }
    merged.into_values().collect()
}

impl DiscoveredService {
    /// The identity of one advertised endpoint. The name is not part of it:
    /// the same service can answer twice under different instance names.
    fn key(&self) -> (u8, String, u16) {
        (
            protocol_rank(&self.protocol),
            self.host.to_lowercase(),
            self.port,
        )
    }
}

/// SMB, then SFTP, then WebDAV — the order people look for them in the
/// dialog's protocol row.
fn protocol_rank(protocol: &str) -> u8 {
    match protocol {
        "smb" => 0,
        "sftp" => 1,
        _ => 2,
    }
}

/// The instance label of a full name ("Name._smb._tcp.local." → "Name"). An
/// instance name may itself contain dots, so the split is on the first `._`.
fn instance_name(fullname: &str) -> String {
    let trimmed = fullname.trim_end_matches('.');
    match trimmed.split_once("._") {
        Some((name, _)) => name.to_owned(),
        None => trimmed.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(protocol: &'static str, name: &str, host: &str, port: u16) -> Raw {
        Raw {
            protocol,
            fullname: format!("{name}._{protocol}._tcp.local."),
            hostname: host.to_owned(),
            port,
            addresses: Vec::new(),
        }
    }

    #[test]
    fn duplicate_advertisements_merge_into_one_row() {
        let mut first = raw("smb", "NAS", "nas.local", 445);
        first.addresses = vec!["192.168.1.10".to_owned()];
        let mut second = raw("smb", "NAS._smb", "NAS.local.", 445);
        second.addresses = vec!["192.168.1.11".to_owned()];
        let rows = normalise(vec![first, second]);
        assert_eq!(rows.len(), 1);
        // The first advertisement wins the spelling; the key is folded.
        assert_eq!(rows[0].host, "nas.local");
        assert_eq!(
            rows[0].addresses,
            vec!["192.168.1.10".to_owned(), "192.168.1.11".to_owned()]
        );
    }

    #[test]
    fn the_same_host_on_two_protocols_stays_two_rows() {
        let rows = normalise(vec![
            raw("smb", "NAS", "nas.local", 445),
            raw("sftp", "NAS", "nas.local", 22),
            raw("webdav", "NAS", "nas.local", 80),
        ]);
        let protocols: Vec<&str> = rows.iter().map(|row| row.protocol.as_str()).collect();
        assert_eq!(protocols, vec!["smb", "sftp", "webdav"]);
    }

    #[test]
    fn a_missing_hostname_falls_back_to_an_address() {
        let mut item = raw("sftp", "Box", "", 22);
        item.addresses = vec!["fe80::1".to_owned()];
        let rows = normalise(vec![item]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].host, "fe80::1");
    }

    #[test]
    fn entries_without_a_host_or_port_are_dropped() {
        let mut nameless = raw("smb", "NAS", "", 445);
        nameless.addresses.clear();
        let rows = normalise(vec![nameless, raw("smb", "NAS", "nas.local", 0)]);
        assert!(rows.is_empty());
    }

    #[test]
    fn the_instance_name_stops_at_the_service_type() {
        assert_eq!(
            instance_name("My.NAS._smb._tcp.local."),
            "My.NAS".to_owned()
        );
        assert_eq!(instance_name("plain"), "plain".to_owned());
    }
}
