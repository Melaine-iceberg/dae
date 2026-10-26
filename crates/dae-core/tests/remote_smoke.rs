//! Opt-in smoke tests against a real SMB, SFTP or cloud backend.
//!
//! They are ignored by default because they need a server (or an account)
//! that CI does not have. Each test connects through the same `vfs` entry
//! point the shell uses, then browses, writes, reads, name-searches, copies
//! and deletes a uniquely named fixture. Run one with:
//!
//! ```text
//! DAE_SMB_HOST=localhost DAE_SMB_PORT=445 \
//! DAE_SMB_USER=... DAE_SMB_PASSWORD=... \
//! DAE_SMB_ROOT='smb://localhost:445/DevelopmentFiles' \
//! cargo test -p dae-core --test remote_smoke smb -- --ignored --nocapture
//! ```
//!
//! SFTP uses `DAE_SFTP_HOST`, `DAE_SFTP_PORT`, `DAE_SFTP_USER`,
//! `DAE_SFTP_PASSWORD` and `DAE_SFTP_ROOT`; cloud uses `DAE_CLOUD_ROOT` with
//! an account already authorized in this profile.

use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use dae_core::fs::connections::{self, Protocol};
use dae_core::fs::types::{ConflictAction, NewEntryKind, TransferItem};
use dae_core::fs::vfs::{self, SharedBackend};
use dae_core::fs::{preview, service, sftp, smb};
use dae_core::host::{Event, Host};

struct NoopHost;

impl Host for NoopHost {
    fn emit(&self, _event: Event) {}
}

fn operation_id() -> String {
    format!("remote-smoke-{}", unique_suffix())
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{}-{nanos}", std::process::id())
}

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is required for this smoke test"))
}

fn optional(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn port(name: &str) -> Option<u16> {
    optional(name).map(|value| value.parse().expect("port must be a number"))
}

fn smb_session() -> (SharedBackend, String) {
    let host = required("DAE_SMB_HOST");
    let port = port("DAE_SMB_PORT");
    let user = optional("DAE_SMB_USER");
    let password = optional("DAE_SMB_PASSWORD");
    let root = required("DAE_SMB_ROOT");

    let backend: SharedBackend = Arc::new(
        smb::SmbBackend::connect_with(&host, port, user.as_deref(), password.as_deref())
            .expect("SMB connection"),
    );
    connections::store_session(Protocol::Smb, &host, port, Arc::clone(&backend));
    (backend, root)
}

fn sftp_session() -> (SharedBackend, String) {
    let host = required("DAE_SFTP_HOST");
    let port = port("DAE_SFTP_PORT");
    let user = required("DAE_SFTP_USER");
    let password = optional("DAE_SFTP_PASSWORD").unwrap_or_default();
    let root = required("DAE_SFTP_ROOT");

    let backend: SharedBackend = Arc::new(
        sftp::SftpBackend::connect_with(&host, port, Some(&user), Some(&password))
            .expect("SFTP connection"),
    );
    connections::store_session(Protocol::Sftp, &host, port, Arc::clone(&backend));
    (backend, root)
}

fn cloud_session() -> (SharedBackend, String) {
    let root = required("DAE_CLOUD_ROOT");
    dae_core::fs::cloud::accounts::init().expect("cloud account store");
    let backend = vfs::resolve(&root).expect("cloud connection");
    (backend, root)
}

enum Backend {
    Smb,
    Sftp,
    Cloud,
}

impl Backend {
    fn label(&self) -> &'static str {
        match self {
            Backend::Smb => "smb",
            Backend::Sftp => "sftp",
            Backend::Cloud => "cloud",
        }
    }

    fn session(&self) -> (SharedBackend, String) {
        match self {
            Backend::Smb => smb_session(),
            Backend::Sftp => sftp_session(),
            Backend::Cloud => cloud_session(),
        }
    }
}

fn exercise(backend_kind: Backend) {
    let (backend, root) = backend_kind.session();
    let label = backend_kind.label();
    let host: Arc<dyn Host> = Arc::new(NoopHost);
    let name = format!("dae-remote-smoke-{}", unique_suffix());

    // The cached session above is what `vfs::resolve` must hand back, so the
    // rest of the flow goes through the same paths the shell uses.
    let resolved = vfs::resolve(&root).expect("vfs resolves the session");

    let root_view = resolved.read_dir(&root).expect("browse the root");
    println!(
        "{label}: browsed {} entries at {}",
        root_view.entries.len(),
        root
    );

    let source_dir = resolved
        .create_entry(&root, &name, NewEntryKind::Directory)
        .expect("create fixture directory");
    let destination_dir = resolved
        .create_entry(&root, &format!("{name}-copy"), NewEntryKind::Directory)
        .expect("create destination directory");

    let fixture = format!("{source_dir}/hello.txt");
    {
        let mut writer = backend
            .open_write(&fixture)
            .expect("open fixture for write");
        writer
            .write_all(b"hello from the remote smoke test")
            .expect("write fixture");
        writer.flush().expect("flush fixture");
    }

    let stat = backend.stat(&fixture).expect("stat fixture");
    assert_eq!(stat.size, 32, "{label}: fixture size");

    // The asynchronous halves create a runtime of their own; the backends
    // above already own one, and nesting their blocking calls inside a
    // second runtime would panic.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("test runtime");

    let text = runtime
        .block_on(preview::read_text_preview(fixture.clone(), 4096))
        .expect("text preview over the vfs");
    assert_eq!(
        text.content, "hello from the remote smoke test",
        "{label}: preview"
    );

    let search = resolved
        .search(&source_dir, "hello", &|| true)
        .expect("name search");
    assert!(
        search.entries.iter().any(|entry| entry.name == "hello.txt"),
        "{label}: search found the fixture"
    );

    runtime
        .block_on(service::copy_entries(
            vec![TransferItem {
                path: fixture.clone(),
                on_conflict: ConflictAction::Fail,
            }],
            destination_dir.clone(),
            operation_id(),
            Arc::clone(&host),
        ))
        .expect("copy within the backend");

    let copied_dir = backend
        .read_dir(&destination_dir)
        .expect("read the copy destination");
    let copied = copied_dir
        .entries
        .iter()
        .find(|entry| entry.name == "hello.txt")
        .unwrap_or_else(|| panic!("{label}: copied file is listed"));
    let mut content = String::new();
    backend
        .open_read(&copied.path)
        .expect("open the copy")
        .read_to_string(&mut content)
        .expect("read the copy");
    assert_eq!(
        content, "hello from the remote smoke test",
        "{label}: copy content"
    );

    let renamed = format!("{source_dir}/renamed.txt");
    backend
        .rename_to(&fixture, &renamed)
        .expect("rename the fixture");
    assert!(
        backend.stat(&renamed).is_ok(),
        "{label}: renamed file exists"
    );

    runtime
        .block_on(service::delete_entries(
            vec![source_dir, destination_dir],
            operation_id(),
            host,
        ))
        .expect("delete the fixtures");

    println!("{label}: connect, browse, preview, search, copy, rename and delete passed");
}

#[test]
#[ignore = "requires DAE_SMB_* and a reachable SMB server"]
fn smb_connects_browses_and_copies() {
    exercise(Backend::Smb);
}

#[test]
#[ignore = "requires DAE_SFTP_* and a reachable SFTP server"]
fn sftp_connects_browses_and_copies() {
    exercise(Backend::Sftp);
}

#[test]
#[ignore = "requires DAE_CLOUD_ROOT and an authorized cloud account"]
fn cloud_connects_browses_and_copies() {
    exercise(Backend::Cloud);
}
