//! Signed, portable self-updates for the desktop shells.
//!
//! The release feed is a Tauri-shaped `latest-qt.json` next to the release
//! assets. Every artifact is a plain zip: its contents mirror the installed
//! directory on Windows/Linux, and the zip contains `dae.app` on macOS. That
//! keeps the installer engine-independent and lets both shells share the
//! format if the old one ever needs it.
//!
//! Installation is deliberately not done by this process. Qt DLLs, `.so`
//! files and the running executable are mapped, so replacing them from inside
//! the app is unsafe on every platform. [`download_and_install`] starts a small
//! PowerShell or `/bin/sh` script instead; it waits for this process to exit,
//! unpacks over the install directory, and starts the new build. The script is
//! generated with arguments, so paths with spaces or quotes survive.
//!
//! Two rules keep the trust boundary small:
//! - the manifest URL is fixed in the binary; `DAE_UPDATE_ENDPOINT` only
//!   overrides it for local testing,
//! - the artifact must carry a minisign signature from the release key before
//!   a single byte is unpacked.

mod install;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Once;
use std::time::Duration;

use base64::Engine as _;
use minisign_verify::{PublicKey, Signature};
use semver::Version;
use serde::Deserialize;
use tempfile::TempDir;
use thiserror::Error;
use url::Url;

use install::InstallLayout;

/// The Qt shell's feed. It intentionally differs from the Tauri shell's
/// `latest.json`: the two shells ship different artifacts during the
/// migration, and an update must never hand the other shell's installer to
/// this app.
pub const DEFAULT_FEED_URL: &str =
    "https://github.com/Melaine-iceberg/dae/releases/latest/download/latest-qt.json";

/// The key line from `minisign.pub`, the same release key the Tauri shell
/// trusts. Only the base64 key line is stored; the comment line is not part of
/// the cryptographic material.
pub const UPDATE_PUBLIC_KEY: &str = "RWRWkqFI/kqRq5NBsNGlgl5NMYox09HIoP5UbgD7sEkkemmj/tBe51eQ";

const USER_AGENT: &str = concat!("dae/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("the update feed could not be read: {0}")]
    Feed(String),
    #[error("the update feed is invalid: {0}")]
    InvalidFeed(String),
    #[error("this platform is not listed in the update feed: {0}")]
    PlatformMissing(String),
    #[error("the update could not be downloaded: {0}")]
    Download(String),
    #[error("the update signature is invalid: {0}")]
    Signature(String),
    #[error("the update archive is invalid: {0}")]
    Archive(String),
    #[error("the update could not be applied: {0}")]
    Install(String),
    #[error("this build has no writable installed layout")]
    NotInstalled,
}

#[derive(Debug, Clone, Deserialize)]
struct UpdateFeed {
    version: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    pub_date: Option<String>,
    platforms: BTreeMap<String, PlatformArtifact>,
}

#[derive(Debug, Clone, Deserialize)]
struct PlatformArtifact {
    url: String,
    signature: String,
}

/// A newer release from the feed, already matched to this platform.
#[derive(Debug, Clone)]
pub struct AvailableUpdate {
    pub version: Version,
    pub notes: String,
    pub pub_date: Option<String>,
    pub url: Url,
    pub signature: String,
}

impl AvailableUpdate {
    pub fn version_text(&self) -> String {
        self.version.to_string()
    }
}

/// The release feed URL for this build. The environment override exists so a
/// local HTTP server can drive the whole flow without touching the binary.
pub fn feed_url() -> String {
    std::env::var("DAE_UPDATE_ENDPOINT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_FEED_URL.to_owned())
}

/// Whether this build should offer updates at all.
///
/// Debug builds are excluded (they usually run from `target/`, not from a
/// deployed bundle), and a deployed bundle must have the Qt platform directory
/// next to the executable. `DAE_UPDATE_DEV=1` lifts the first restriction when
/// the updater itself is being tested.
pub fn updates_enabled() -> bool {
    let dev = std::env::var("DAE_UPDATE_DEV").as_deref() == Ok("1");
    (cfg!(not(debug_assertions)) || dev) && InstallLayout::current().is_some()
}

/// Checks the feed for a version newer than `current_version`.
pub async fn check_for_update(
    endpoint: &str,
    current_version: &str,
) -> Result<Option<AvailableUpdate>, UpdateError> {
    let endpoint = Url::parse(endpoint)
        .map_err(|error| UpdateError::InvalidFeed(format!("invalid feed URL: {error}")))?;
    let current = parse_version(current_version)?;
    let feed = fetch_feed(&endpoint).await?;
    select_update(feed, &endpoint, &current, platform_key())
}

/// Downloads, verifies and installs `update`, then asks the caller to exit.
///
/// The function returns only after the detached installer script has started.
/// The caller must quit the UI so the script can replace the running files and
/// relaunch the app. `progress` is called with 0..=100 on a background thread.
pub async fn download_and_install<F>(
    update: &AvailableUpdate,
    progress: F,
) -> Result<(), UpdateError>
where
    F: Fn(u8) + Send + Sync,
{
    install::download_and_install(update, &progress).await
}

fn parse_version(text: &str) -> Result<Version, UpdateError> {
    let text = text.trim().trim_start_matches(['v', 'V']);
    Version::parse(text)
        .map_err(|error| UpdateError::InvalidFeed(format!("invalid version: {error}")))
}

fn platform_key() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "windows-x86_64",
        ("windows", "aarch64") => "windows-aarch64",
        ("macos", "x86_64") => "darwin-x86_64",
        ("macos", "aarch64") => "darwin-aarch64",
        ("linux", "x86_64") => "linux-x86_64",
        ("linux", "aarch64") => "linux-aarch64",
        (os, arch) => {
            // A compile-time table keeps the match exhaustive without leaking a
            // formatted allocation into the common path.
            let _ = (os, arch);
            "unsupported"
        }
    }
}

async fn fetch_feed(endpoint: &Url) -> Result<UpdateFeed, UpdateError> {
    let client = http_client()?;
    let response = client
        .get(endpoint.clone())
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .send()
        .await
        .map_err(|error| UpdateError::Feed(error.to_string()))?;

    let status = response.status();
    if !status.is_success() {
        return Err(UpdateError::Feed(format!("HTTP {status}")));
    }

    let body = response
        .bytes()
        .await
        .map_err(|error| UpdateError::Feed(error.to_string()))?;
    serde_json::from_slice(&body).map_err(|error| UpdateError::InvalidFeed(error.to_string()))
}

fn select_update(
    feed: UpdateFeed,
    endpoint: &Url,
    current: &Version,
    platform: &str,
) -> Result<Option<AvailableUpdate>, UpdateError> {
    let version = parse_version(&feed.version)?;

    // Compare first: an up-to-date user on a platform that the newest release
    // did not produce should see "up to date", not "not published".
    if version <= *current {
        return Ok(None);
    }

    let artifact = feed
        .platforms
        .get(platform)
        .ok_or_else(|| UpdateError::PlatformMissing(platform.to_owned()))?;

    let url = endpoint
        .join(&artifact.url)
        .map_err(|error| UpdateError::InvalidFeed(format!("invalid artifact URL: {error}")))?;
    if artifact.signature.trim().is_empty() {
        return Err(UpdateError::InvalidFeed(
            "the artifact has no signature".to_owned(),
        ));
    }

    Ok(Some(AvailableUpdate {
        version,
        notes: feed.notes,
        pub_date: feed.pub_date,
        url,
        signature: artifact.signature.clone(),
    }))
}

fn http_client() -> Result<reqwest::Client, UpdateError> {
    static PROVIDER: Once = Once::new();
    PROVIDER.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(180))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|error| UpdateError::Download(error.to_string()))
}

/// Downloads the artifact into a temporary directory and verifies its
/// signature. The temporary directory must stay alive until extraction ends.
async fn download_verified<F>(
    update: &AvailableUpdate,
    progress: &F,
) -> Result<(TempDir, PathBuf), UpdateError>
where
    F: Fn(u8) + Send + Sync,
{
    let client = http_client()?;
    let response = client
        .get(update.url.clone())
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .send()
        .await
        .map_err(|error| UpdateError::Download(error.to_string()))?;

    let status = response.status();
    if !status.is_success() {
        return Err(UpdateError::Download(format!("HTTP {status}")));
    }

    let total = response.content_length().filter(|size| *size > 0);
    let temp = tempfile::Builder::new()
        .prefix("dae-update-")
        .tempdir()
        .map_err(|error| UpdateError::Download(error.to_string()))?;
    let zip_path = temp.path().join("update.zip");
    let mut file = tokio::fs::File::create(&zip_path)
        .await
        .map_err(|error| UpdateError::Download(error.to_string()))?;

    use tokio::io::AsyncWriteExt;
    let mut response = response;
    let mut downloaded = 0_u64;
    let mut last_percent = 0_u8;
    progress(0);

    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| UpdateError::Download(error.to_string()))?
    {
        file.write_all(&chunk)
            .await
            .map_err(|error| UpdateError::Download(error.to_string()))?;
        downloaded += chunk.len() as u64;
        if let Some(total) = total {
            let percent = (downloaded.min(total) * 90 / total) as u8;
            if percent != last_percent {
                last_percent = percent;
                progress(percent);
            }
        }
    }
    file.flush()
        .await
        .map_err(|error| UpdateError::Download(error.to_string()))?;
    drop(file);
    progress(90);

    let bytes =
        std::fs::read(&zip_path).map_err(|error| UpdateError::Download(error.to_string()))?;
    verify_with_key(&bytes, &update.signature, UPDATE_PUBLIC_KEY)?;

    Ok((temp, zip_path))
}

fn verify_with_key(bytes: &[u8], signature: &str, key: &str) -> Result<(), UpdateError> {
    let public_key =
        PublicKey::from_base64(key).map_err(|error| UpdateError::Signature(error.to_string()))?;
    let signature_text = minisign_signature_text(signature);
    let signature = Signature::decode(&signature_text)
        .map_err(|error| UpdateError::Signature(error.to_string()))?;
    public_key
        // `true` keeps older non-prehashed builds installable; the key is not
        // shared with another signing purpose.
        .verify(bytes, &signature, true)
        .map_err(|error| UpdateError::Signature(error.to_string()))
}

/// Tauri's updater stores a base64-encoded copy of the minisign signature text
/// in `latest.json`; `minisign_verify` wants the four text lines. Accept both
/// shapes so the Qt feed can stay byte-for-byte in Tauri's format, while the
/// tests and hand-written feeds can keep using the raw signature.
fn minisign_signature_text(signature: &str) -> String {
    let signature = signature.trim();
    if signature.starts_with("untrusted comment:") {
        return signature.to_owned();
    }

    base64::engine::general_purpose::STANDARD
        .decode(signature)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .filter(|text| text.starts_with("untrusted comment:"))
        .unwrap_or_else(|| signature.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(version: &str, platform: &str) -> UpdateFeed {
        let mut platforms = BTreeMap::new();
        platforms.insert(
            platform.to_owned(),
            PlatformArtifact {
                url: "dae-0.6.0.zip".to_owned(),
                signature: "trusted comment: signed".to_owned(),
            },
        );
        UpdateFeed {
            version: version.to_owned(),
            notes: "A steady release.".to_owned(),
            pub_date: Some("2026-10-26T00:00:00Z".to_owned()),
            platforms,
        }
    }

    #[test]
    fn a_newer_feed_selects_the_platform_artifact() {
        let endpoint = Url::parse("https://example.com/latest-qt.json").unwrap();
        let selected = select_update(
            feed("0.6.0", "windows-x86_64"),
            &endpoint,
            &Version::parse("0.5.0").unwrap(),
            "windows-x86_64",
        )
        .unwrap()
        .unwrap();

        assert_eq!(selected.version, Version::parse("0.6.0").unwrap());
        assert_eq!(selected.url.as_str(), "https://example.com/dae-0.6.0.zip");
    }

    #[test]
    fn the_same_or_older_version_is_not_an_update() {
        let endpoint = Url::parse("https://example.com/latest-qt.json").unwrap();
        let current = Version::parse("0.6.0").unwrap();
        assert!(
            select_update(
                feed("0.6.0", "linux-x86_64"),
                &endpoint,
                &current,
                "linux-x86_64"
            )
            .unwrap()
            .is_none()
        );
        assert!(
            select_update(
                feed("0.5.0", "linux-x86_64"),
                &endpoint,
                &current,
                "linux-x86_64"
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn version_prefixes_are_tolerated() {
        let endpoint = Url::parse("https://example.com/latest-qt.json").unwrap();
        let selected = select_update(
            feed("v0.6.0", "darwin-aarch64"),
            &endpoint,
            &Version::parse("0.5.0").unwrap(),
            "darwin-aarch64",
        )
        .unwrap();
        assert!(selected.is_some());
    }

    #[test]
    fn a_platform_missing_from_the_feed_is_reported() {
        let endpoint = Url::parse("https://example.com/latest-qt.json").unwrap();
        let error = select_update(
            feed("0.6.0", "windows-x86_64"),
            &endpoint,
            &Version::parse("0.5.0").unwrap(),
            "linux-aarch64",
        )
        .unwrap_err();
        assert!(matches!(error, UpdateError::PlatformMissing(name) if name == "linux-aarch64"));
    }

    #[test]
    fn an_up_to_date_build_does_not_care_about_missing_platforms() {
        let endpoint = Url::parse("https://example.com/latest-qt.json").unwrap();
        assert!(
            select_update(
                feed("0.5.0", "windows-x86_64"),
                &endpoint,
                &Version::parse("0.5.0").unwrap(),
                "linux-aarch64",
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn feed_parsing_accepts_relative_urls_and_optional_fields() {
        let json = r#"{
            "version": "0.6.0",
            "platforms": {
                "linux-x86_64": {
                    "url": "https://example.com/a.zip",
                    "signature": "signed"
                }
            }
        }"#;
        let parsed: UpdateFeed = serde_json::from_str(json).unwrap();
        assert!(parsed.notes.is_empty());
        assert!(parsed.pub_date.is_none());
        assert!(parsed.platforms.contains_key("linux-x86_64"));
    }

    #[test]
    fn minisign_signatures_are_checked_against_the_given_key() {
        let key = "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
        let signature = "untrusted comment: signature from minisign secret key
RWQf6LRCGA9i59SLOFxz6NxvASXDJeRtuZykwQepbDEGt87ig1BNpWaVWuNrm73YiIiJbq71Wi+dP9eKL8OC351vwIasSSbXxwA=
trusted comment: timestamp:1555779966\tfile:test
QtKMXWyYcwdpZAlPF7tE2ENJkRd1ujvKjlj1m9RtHTBnZPa5WKU5uWRs5GoP5M/VqE81QFuMKI5k/SfNQUaOAA==";
        verify_with_key(b"test", signature, key).unwrap();
        assert!(matches!(
            verify_with_key(b"Test", signature, key),
            Err(UpdateError::Signature(_))
        ));

        // Tauri writes `.sig` files as base64 of this same text. The release
        // feed embeds that file verbatim, so both shapes must verify.
        let encoded = base64::engine::general_purpose::STANDARD.encode(signature.as_bytes());
        assert!(!encoded.contains('\n'));
        verify_with_key(b"test", &encoded, key).unwrap();
    }

    #[test]
    fn the_production_key_is_a_valid_minisign_key() {
        PublicKey::from_base64(UPDATE_PUBLIC_KEY).expect("release key must decode");
    }
}
