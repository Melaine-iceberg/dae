//! Host-key verification for the SSH-backed storage backends (SFTP today).
//!
//! The server identity is checked against the user's own OpenSSH
//! `known_hosts` file — `~/.ssh/known_hosts`, the one `ssh`, `ssh-keyscan`
//! and `ssh-keygen -F/-R` already read and write. The policy is OpenSSH's
//! `accept-new`: an unknown host is recorded on first contact (trust on first
//! use) and every later contact has to present the same key. A key that
//! changed is a hard refusal, not a warning, because the two explanations for
//! it — the server was reinstalled, or someone is in the middle — cannot be
//! told apart from here. A host that is on record but never under the key type
//! the server just offered is refused too, rather than extended: switching
//! algorithms is a cheap way to sidestep a pinned key, and OpenSSH's
//! `accept-new` treats it the same way.
//!
//! Why the shared file rather than a store of our own:
//!
//! - a trust decision belongs to the machine, not to one app. A host the user
//!   already accepted with `ssh` (or pinned with `ssh-keyscan`) connects
//!   without a second, app-shaped opinion;
//! - it is auditable and repairable with the tools the user already has
//!   (`ssh-keygen -F host`, `ssh-keygen -R host`), which is also what the
//!   refusal below tells them to run;
//! - the matching rules OpenSSH uses — non-default ports as `[host]:port`,
//!   comma-separated host lists, `|1|` hashed entries — come with the file
//!   format, so a second implementation could only get them subtly wrong.
//!
//! What trust on first use does *not* buy: a first contact that is already
//! intercepted records the attacker's key. That is inherent to the policy
//! (it is what `accept-new` means), which is why every learned key is logged
//! with its SHA-256 fingerprint — an out-of-band comparison is the way to
//! catch it — and why the refusal path names both fingerprints.

use russh::keys::{self, HashAlg, PublicKey, known_hosts};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// What [`verify_host_key`] concluded, for the caller's log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostKeyVerdict {
    /// The presented key matches the one on record for this host.
    Known,
    /// The host had no record at all; the key was recorded just now.
    Learned,
}

/// Checks `key` against the user's known-hosts file for `host:port`,
/// recording it if the host is new.
///
/// The error is the user-facing sentence rather than a
/// [`super::error::FileSystemError`] because the caller is a `russh` handler:
/// it can only answer the handshake with yes/no, so it stashes the reason for
/// the connection code to surface instead of russh's generic "unknown key".
pub(crate) fn verify_host_key(
    host: &str,
    port: u16,
    key: &PublicKey,
) -> Result<HostKeyVerdict, String> {
    verify_at(&known_hosts_file()?, host, port, key)
}

/// Test seam: [`verify_host_key`] against a known-hosts file of the caller's
/// choosing, so nothing in the suite touches the real `~/.ssh/known_hosts`.
#[cfg(test)]
pub(crate) fn verify_host_key_at(
    path: &Path,
    host: &str,
    port: u16,
    key: &PublicKey,
) -> Result<HostKeyVerdict, String> {
    verify_at(path, host, port, key)
}

/// The file `ssh` itself uses. Absent is not an error: `learn` creates it,
/// exactly as `ssh` does on its own first connection.
fn known_hosts_file() -> Result<PathBuf, String> {
    std::env::home_dir()
        .map(|home| home.join(".ssh").join("known_hosts"))
        .ok_or_else(|| {
            "The SSH known-hosts file cannot be located because there is no home directory. \
             Set HOME (on Windows, USERPROFILE) and reconnect."
                .to_owned()
        })
}

/// Checks `key` against a known-hosts file at `path` rather than the user's
/// own.
///
/// The error is the user-facing sentence rather than a
/// [`super::error::FileSystemError`] because the caller is a `russh` handler:
/// it can only answer the handshake with yes/no, so it stashes the reason for
/// the connection code to surface instead of russh's generic "unknown key".
fn verify_at(
    path: &Path,
    host: &str,
    port: u16,
    key: &PublicKey,
) -> Result<HostKeyVerdict, String> {
    // OpenSSH lowercases host names before they reach the file, so `MyServer`
    // and `myserver` have to land on the same entry here too.
    let host = host.trim().to_ascii_lowercase();

    match known_hosts::check_known_hosts_path(&host, port, key, path) {
        Ok(true) => Ok(HostKeyVerdict::Known),
        Ok(false) => {
            // "No key matched" is two different situations. A host that is
            // absent from the file has never been accepted, so first use may
            // record it. A host that is *on record under another key type* is
            // not new at all: committing to an algorithm the file does not pin
            // yet is exactly what a man in the middle would do, and OpenSSH's
            // `accept-new` (since `known_hosts_contains_hostname`) refuses it
            // for the same reason.
            let recorded = recorded_fingerprints(path, &host, port);
            if recorded.is_empty() {
                learn(path, &host, port, key)
            } else {
                Err(different_key_type_message(
                    path, &host, port, key, &recorded,
                ))
            }
        }
        // The one case that is a decision rather than a failure: a key on
        // record disagrees with the server's.
        Err(keys::Error::KeyChanged { line }) => {
            Err(mismatch_message(path, &host, port, key, line))
        }
        Err(error) => Err(format!(
            "The host key of {} could not be checked against {}: {error}. \
             The connection was refused rather than skipped; make that file readable (it may be \
             owned by another account) and reconnect.",
            endpoint(&host, port),
            path.display()
        )),
    }
}

fn learn(path: &Path, host: &str, port: u16, key: &PublicKey) -> Result<HostKeyVerdict, String> {
    record(path, host, port, key).map_err(|error| {
        format!(
            "The host key of {} could not be recorded in {}: {error}. \
             The connection was refused rather than trusted without a record; make that folder \
             writable, or add the key yourself with `ssh-keyscan -p {port} {host} >> \"{}\"`, \
             and reconnect.",
            endpoint(host, port),
            path.display(),
            path.display()
        )
    })?;
    Ok(HostKeyVerdict::Learned)
}

/// Appends one entry to the known-hosts file, creating the file and its
/// `~/.ssh` folder when they are missing.
///
/// The format is OpenSSH's: `name SP key` per line, the name in `[host]:port`
/// form for non-default ports. `russh` has a writer for this, but it prefixes a
/// newline whenever the file does not already end with one — which plants a
/// blank first line in a brand-new `~/.ssh/known_hosts`. This is a user file
/// shared with `ssh`, so it is written in the shape the user's own tools would
/// leave behind.
fn record(path: &Path, host: &str, port: u16, key: &PublicKey) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let openssh = key
        .to_openssh()
        .map_err(|error| io::Error::other(error.to_string()))?;
    let line = format!("{} {openssh}\n", known_hosts_name(host, port));

    let created = !path.exists();
    let mut file = OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(path)?;

    // Only a file that already holds something without a final newline needs
    // one: an empty file starts at line 1, where the entry belongs.
    if file.metadata()?.len() > 0 {
        file.seek(SeekFrom::End(-1))?;
        let mut last = [0u8; 1];
        file.read_exact(&mut last)?;
        if last[0] != b'\n' {
            file.write_all(b"\n")?;
        }
    }
    file.write_all(line.as_bytes())?;
    file.flush()?;

    // `ssh` creates this file owner-only, and a host key list is not something
    // to leave world-readable. Only a file we just created is re-moded; an
    // existing one belongs to `ssh` and keeps whatever it has.
    #[cfg(unix)]
    if created {
        use std::os::unix::fs::PermissionsExt;
        let _ = file.set_permissions(fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = created;

    Ok(())
}

fn mismatch_message(
    path: &Path,
    host: &str,
    port: u16,
    presented: &PublicKey,
    line: usize,
) -> String {
    let recorded = recorded_fingerprints(path, host, port);
    let expected = if recorded.is_empty() {
        "a different key".to_owned()
    } else {
        recorded.join(", ")
    };

    format!(
        "The host key of {} does not match the one recorded in {} (line {line}): \
         recorded {expected}, offered {}. {}",
        endpoint(host, port),
        path.display(),
        presented.fingerprint(HashAlg::Sha256),
        repair_hint(path, host, port)
    )
}

/// The host is on record, but only under a key type the server did not offer.
/// A user with both an RSA and an ED25519 line for one host reaches this by
/// pinning a `HostKeyAlgorithms` the server cannot satisfy, so the message has
/// to explain the pin rather than accuse the server.
fn different_key_type_message(
    path: &Path,
    host: &str,
    port: u16,
    presented: &PublicKey,
    recorded: &[String],
) -> String {
    format!(
        "{} is already known in {}, but not with the {} key the server offered \
         (recorded {}, offered {}). If the server's key set changed, remove the stale lines with \
         `ssh-keygen -R \"{}\"` (in {}) and reconnect. Otherwise the connection may be \
         intercepted — do not continue.",
        endpoint(host, port),
        path.display(),
        presented.algorithm(),
        recorded.join(", "),
        presented.fingerprint(HashAlg::Sha256),
        known_hosts_name(host, port),
        path.display()
    )
}

/// The way out of any host-key refusal: the command that removes the entry the
/// client is objecting to, and the warning that goes with ignoring it.
fn repair_hint(path: &Path, host: &str, port: u16) -> String {
    format!(
        "If the server was reinstalled or replaced, remove the old line with \
         `ssh-keygen -R \"{}\"` (in {}) and reconnect. Otherwise the connection may be \
         intercepted — do not continue.",
        known_hosts_name(host, port),
        path.display()
    )
}

/// Fingerprints of the keys on record for this host, for the refusal message.
/// A file that cannot be re-read is not worth a second error here: the line
/// number and the offered fingerprint still identify the conflict.
fn recorded_fingerprints(path: &Path, host: &str, port: u16) -> Vec<String> {
    known_hosts::known_host_keys_path(host, port, path)
        .unwrap_or_default()
        .into_iter()
        .map(|(_, key)| key.fingerprint(HashAlg::Sha256).to_string())
        .collect()
}

/// `host:port` for prose.
fn endpoint(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_owned()
    } else {
        format!("{host}:{port}")
    }
}

/// `host` / `[host]:port` — the name a key is filed under, and the argument
/// `ssh-keygen -R` expects.
fn known_hosts_name(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_owned()
    } else {
        format!("[{host}]:{port}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Two real ed25519 public keys: the fixtures pin the encoding paths
    // (`from_openssh`, `to_openssh`) that the file is actually written with.
    const KEY_A: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJdD7y3aLq454yWBdwLWbieU1ebz9/cu7/QEXn9OIeZJ";
    const KEY_B: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIA6rWI3G2sz07DnfFlrouTcysQlj2P+jpNSOEWD9OJ3X";
    // A second *algorithm*, for the case where the host is on record but the
    // server offered a key type the file does not pin.
    const KEY_ECDSA: &str = "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBNW77R7Pqg3dau75pCw8pE5t3ndaoGgTFtvRbJXzxJVHzicQo1swyuya4vdUh72I1jqAgiW/Rk9oY2iDkFN/Q3I=";

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("dae-known-hosts-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("create the test directory");
            Self(path)
        }

        fn known_hosts(&self) -> PathBuf {
            self.0.join("known_hosts")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn key(openssh: &str) -> PublicKey {
        PublicKey::from_openssh(openssh).expect("parse the fixture key")
    }

    fn contents(dir: &TempDir) -> String {
        std::fs::read_to_string(dir.known_hosts()).unwrap_or_default()
    }

    #[test]
    fn trusts_a_host_on_first_use_and_records_it() {
        let dir = TempDir::new("first-use");
        let path = dir.known_hosts();

        let verdict = verify_host_key_at(&path, "example.com", 22, &key(KEY_A))
            .expect("first contact is accepted");
        assert_eq!(verdict, HostKeyVerdict::Learned);

        let recorded = contents(&dir);
        assert!(
            recorded.starts_with("example.com ssh-ed25519 AAAA"),
            "a default-port host is filed under its bare name: {recorded}"
        );
        assert_eq!(recorded.lines().count(), 1);
    }

    #[test]
    fn a_non_default_port_is_filed_as_a_bracketed_host() {
        let dir = TempDir::new("port");
        let path = dir.known_hosts();

        verify_host_key_at(&path, "example.com", 2222, &key(KEY_A)).expect("first contact");

        let recorded = contents(&dir);
        assert!(
            recorded.starts_with("[example.com]:2222 ssh-ed25519 AAAA"),
            "{recorded}"
        );
    }

    #[test]
    fn a_recorded_host_is_known_and_not_recorded_twice() {
        let dir = TempDir::new("known");
        let path = dir.known_hosts();

        verify_host_key_at(&path, "example.com", 22, &key(KEY_A)).expect("first contact");
        let after_first = contents(&dir);

        let verdict = verify_host_key_at(&path, "example.com", 22, &key(KEY_A))
            .expect("second contact matches");
        assert_eq!(verdict, HostKeyVerdict::Known);
        assert_eq!(contents(&dir), after_first, "the file must not grow");
    }

    #[test]
    fn the_host_name_is_folded_to_lowercase() {
        let dir = TempDir::new("case");
        let path = dir.known_hosts();

        verify_host_key_at(&path, "MyServer.Local", 22, &key(KEY_A)).expect("first contact");
        assert!(contents(&dir).starts_with("myserver.local "));

        let verdict = verify_host_key_at(&path, "myserver.local", 22, &key(KEY_A))
            .expect("the folded name matches");
        assert_eq!(verdict, HostKeyVerdict::Known);
    }

    #[test]
    fn a_changed_key_is_refused_with_both_fingerprints_and_the_way_out() {
        let dir = TempDir::new("changed");
        let path = dir.known_hosts();

        verify_host_key_at(&path, "example.com", 2222, &key(KEY_A)).expect("first contact");

        let error = verify_host_key_at(&path, "example.com", 2222, &key(KEY_B))
            .expect_err("a changed key must be refused");

        let expected = key(KEY_A).fingerprint(HashAlg::Sha256).to_string();
        let offered = key(KEY_B).fingerprint(HashAlg::Sha256).to_string();
        assert!(error.contains(&expected), "records the old key: {error}");
        assert!(error.contains(&offered), "names the offered key: {error}");
        assert!(error.contains("line 1"), "points at the line: {error}");
        assert!(
            error.contains(&path.display().to_string()),
            "names the file: {error}"
        );
        assert!(
            error.contains("ssh-keygen -R \"[example.com]:2222\""),
            "carries the repair command: {error}"
        );
    }

    #[test]
    fn hosts_do_not_share_their_records() {
        let dir = TempDir::new("isolation");
        let path = dir.known_hosts();

        verify_host_key_at(&path, "a.example.com", 22, &key(KEY_A)).expect("first host");
        let verdict = verify_host_key_at(&path, "b.example.com", 22, &key(KEY_B))
            .expect("the second host is new, not a mismatch");
        assert_eq!(verdict, HostKeyVerdict::Learned);
        assert_eq!(contents(&dir).lines().count(), 2);
    }

    #[test]
    fn an_entry_written_by_ssh_is_recognised() {
        let dir = TempDir::new("ssh-entry");
        let path = dir.known_hosts();
        // `ssh` adds a trailing comment to the line it writes.
        fs::write(&path, format!("example.com {KEY_A} alice@laptop\n"))
            .expect("seed an ssh-shaped file");

        let verdict = verify_host_key_at(&path, "example.com", 22, &key(KEY_A))
            .expect("a key ssh already trusted is known here too");
        assert_eq!(verdict, HostKeyVerdict::Known);
        assert_eq!(contents(&dir).lines().count(), 1, "nothing was appended");
    }

    #[test]
    fn an_existing_file_without_a_final_newline_is_continued_not_corrupted() {
        let dir = TempDir::new("no-newline");
        let path = dir.known_hosts();
        fs::write(&path, "other.example.com ").expect("seed a half-written file");

        verify_host_key_at(&path, "example.com", 22, &key(KEY_A)).expect("first contact");

        let recorded = contents(&dir);
        assert_eq!(
            recorded.lines().count(),
            2,
            "the entry starts on its own line: {recorded}"
        );
        assert_eq!(
            recorded.lines().nth(1),
            Some(format!("example.com {KEY_A}").as_str())
        );
    }

    #[test]
    fn a_known_host_offering_another_key_type_is_refused_not_recorded() {
        let dir = TempDir::new("other-type");
        let path = dir.known_hosts();

        verify_host_key_at(&path, "example.com", 22, &key(KEY_A)).expect("first contact");

        let error = verify_host_key_at(&path, "example.com", 22, &key(KEY_ECDSA))
            .expect_err("a second key type is not a new host");
        assert!(
            error.contains("already known") && error.contains("ecdsa-sha2-nistp256"),
            "explains the mismatch instead of the host being new: {error}"
        );
        assert_eq!(
            contents(&dir).lines().count(),
            1,
            "nothing was appended for the offered type"
        );
    }

    #[test]
    fn a_key_that_cannot_be_recorded_is_refused_rather_than_trusted() {
        let dir = TempDir::new("unwritable");
        // A *file* where the folder should be: the check reads it as "no such
        // host" and the write then fails, which is the must-not-trust case.
        let blocker = dir.0.join("blocker");
        std::fs::write(&blocker, b"").expect("create the blocking file");
        let path = blocker.join("known_hosts");

        let error = verify_host_key_at(&path, "example.com", 22, &key(KEY_A))
            .expect_err("a key that cannot be recorded is not trusted");
        assert!(
            error.contains("could not be recorded"),
            "explains the refusal: {error}"
        );
        assert!(
            error.contains("ssh-keyscan"),
            "names the manual way out: {error}"
        );
    }
}
