//! Shared abstractions for cloud storage providers. Each provider implements
//! its OAuth endpoints and file API primitives; `super::CloudBackend` wraps
//! one provider with token management and the explorer's path model.

use crate::file_system::error::FileSystemError;
use crate::file_system::types::EntryKind;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::Path;
use std::sync::Arc;

/// The cloud services the app can connect to. The serialized form doubles as
/// the provider segment of `cloud://` account ids, so it is a stable contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CloudProviderKind {
    GoogleDrive,
    Onedrive,
    Dropbox,
}

impl CloudProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CloudProviderKind::GoogleDrive => "google_drive",
            CloudProviderKind::Onedrive => "onedrive",
            CloudProviderKind::Dropbox => "dropbox",
        }
    }

    pub fn parse(value: &str) -> Result<Self, FileSystemError> {
        match value {
            "google_drive" => Ok(CloudProviderKind::GoogleDrive),
            "onedrive" => Ok(CloudProviderKind::Onedrive),
            "dropbox" => Ok(CloudProviderKind::Dropbox),
            other => Err(FileSystemError::InvalidInput(format!(
                "Unknown cloud provider: {other}"
            ))),
        }
    }
}

/// One OAuth token grant as returned by a provider's token endpoint.
#[derive(Debug, Clone)]
pub struct TokenSet {
    pub access_token: String,
    pub expires_in_secs: u64,
    /// Replacement refresh token when the provider rotates them (Dropbox).
    pub refresh_token: Option<String>,
}

/// Normalized metadata for one cloud entry, across providers.
#[derive(Debug, Clone)]
pub struct CloudMeta {
    pub id: String,
    pub name: String,
    pub kind: EntryKind,
    pub size: u64,
    /// Milliseconds since the Unix epoch, when the provider reports it.
    pub modified_at: Option<u64>,
    /// Parent folder id, when the provider reports it.
    pub parent_id: Option<String>,
    /// Provider-side path; Dropbox write endpoints need it.
    pub path: Option<String>,
}

#[async_trait]
pub trait CloudProvider: Send + Sync {
    // -- OAuth --

    /// The browser-facing authorization URL (PKCE challenge flow).
    fn authorize_url(
        &self,
        client_id: &str,
        redirect_uri: &str,
        state: &str,
        code_challenge: &str,
    ) -> String;

    async fn exchange_code(
        &self,
        client: &reqwest::Client,
        client_id: &str,
        client_secret: Option<&str>,
        code: &str,
        redirect_uri: &str,
        code_verifier: &str,
    ) -> Result<TokenSet, FileSystemError>;

    async fn refresh_token(
        &self,
        client: &reqwest::Client,
        client_id: &str,
        client_secret: Option<&str>,
        refresh_token: &str,
    ) -> Result<TokenSet, FileSystemError>;

    /// `(email, display name)` of the authorized account.
    async fn account_info(
        &self,
        client: &reqwest::Client,
        access_token: &str,
    ) -> Result<(String, String), FileSystemError>;

    // -- File API --

    /// Children of `folder_id`; an empty id means the drive root. Providers
    /// paginate internally and return the full listing.
    async fn list(
        &self,
        client: &reqwest::Client,
        token: &str,
        folder_id: &str,
    ) -> Result<Vec<CloudMeta>, FileSystemError>;

    async fn metadata(
        &self,
        client: &reqwest::Client,
        token: &str,
        id: &str,
    ) -> Result<CloudMeta, FileSystemError>;

    async fn create_folder(
        &self,
        client: &reqwest::Client,
        token: &str,
        parent_id: &str,
        name: &str,
    ) -> Result<CloudMeta, FileSystemError>;

    async fn rename(
        &self,
        client: &reqwest::Client,
        token: &str,
        id: &str,
        new_name: &str,
    ) -> Result<(), FileSystemError>;

    /// Move and/or rename in one call; `new_name` may equal the current name.
    async fn move_to(
        &self,
        client: &reqwest::Client,
        token: &str,
        id: &str,
        dest_parent_id: &str,
        new_name: &str,
    ) -> Result<(), FileSystemError>;

    /// Permanently deletes a file or a (recursively handled by the engine)
    /// directory.
    async fn delete(
        &self,
        client: &reqwest::Client,
        token: &str,
        id: &str,
    ) -> Result<(), FileSystemError>;

    /// Starts an authenticated streaming download.
    async fn download(
        &self,
        client: &reqwest::Client,
        token: &str,
        id: &str,
    ) -> Result<reqwest::Response, FileSystemError>;

    /// Uploads the `size`-byte local file `source` as `name` inside
    /// `dest_parent_id`, replacing any existing entry of that name.
    async fn upload(
        &self,
        client: &reqwest::Client,
        token: &str,
        dest_parent_id: &str,
        name: &str,
        source: &Path,
        size: u64,
    ) -> Result<CloudMeta, FileSystemError>;
}

pub fn provider_for(kind: CloudProviderKind) -> Arc<dyn CloudProvider> {
    match kind {
        CloudProviderKind::GoogleDrive => Arc::new(super::drive::DriveProvider),
        CloudProviderKind::Onedrive => Arc::new(super::onedrive::OnedriveProvider),
        CloudProviderKind::Dropbox => Arc::new(super::dropbox::DropboxProvider),
    }
}

// -- Shared helpers ----------------------------------------------------------

/// Builds the HTTP client shared by all cloud traffic. Installs the
/// process-wide rustls provider (ring) on first use.
pub fn http_client() -> Result<reqwest::Client, FileSystemError> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|error| FileSystemError::Internal(format!("Could not build the HTTP client: {error}")))
}

/// Minimal percent-encoder for building authorization URLs.
pub fn url_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

/// Maps a non-success HTTP status onto a `FileSystemError`. 401 messages
/// carry a "401 " prefix that the token vault uses to spot token failures and
/// trigger one refresh + retry.
pub fn map_status(status: reqwest::StatusCode, detail: &str) -> FileSystemError {
    let detail = summarize(detail);
    match status.as_u16() {
        401 => FileSystemError::PermissionDenied(format!(
            "401 The cloud session expired: {detail}"
        )),
        403 => FileSystemError::PermissionDenied(detail),
        404 => FileSystemError::NotFound(detail),
        409 => FileSystemError::AlreadyExists(detail),
        429 => FileSystemError::Io(format!(
            "The cloud service rate-limited the request: {detail}"
        )),
        _ => FileSystemError::Io(format!("The cloud service returned {status}: {detail}")),
    }
}

/// Validates the response status and returns the body text.
pub async fn text_checked(response: reqwest::Response) -> Result<String, FileSystemError> {
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if status.is_success() {
        Ok(text)
    } else {
        Err(map_status(status, &text))
    }
}

/// Token-endpoint failures are credential/configuration problems, not file
/// errors; report them as authorization issues with the provider's detail.
pub fn map_token_error(status: reqwest::StatusCode, detail: &str) -> FileSystemError {
    match status.as_u16() {
        400 | 401 | 403 => FileSystemError::PermissionDenied(format!(
            "The provider rejected the OAuth credentials (check the client id/secret and the registered redirect URI): {}",
            summarize(detail)
        )),
        _ => FileSystemError::Io(format!("The token endpoint returned {status}: {}", summarize(detail))),
    }
}

/// Fills `chunk` from `file`, looping over partial reads. Returns the number
/// of bytes read; 0 means end of file.
pub fn read_chunk(file: &mut std::fs::File, chunk: &mut [u8]) -> Result<usize, FileSystemError> {
    use std::io::Read;

    let mut filled = 0;
    while filled < chunk.len() {
        match file.read(&mut chunk[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                return Err(FileSystemError::Io(format!(
                    "Could not read the staged upload file: {error}"
                )))
            }
        }
    }
    Ok(filled)
}

/// True when an error came from an expired or revoked access token.
pub fn is_token_error(error: &FileSystemError) -> bool {
    matches!(error, FileSystemError::PermissionDenied(message) if message.starts_with("401 "))
}

fn summarize(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= 300 {
        trimmed.to_owned()
    } else {
        trimmed.chars().take(300).collect::<String>()
    }
}

/// Parses provider timestamps ("2024-01-02T03:04:05.123Z", "+02:00" offsets,
/// Dropbox's " UTC" suffix) into milliseconds since the Unix epoch.
pub fn parse_timestamp(value: &str) -> Option<u64> {
    let bytes = value.as_bytes();
    if bytes.len() < 19 {
        return None;
    }

    let year: i64 = value.get(0..4)?.parse().ok()?;
    let month: i64 = value.get(5..7)?.parse().ok()?;
    let day: i64 = value.get(8..10)?.parse().ok()?;
    if !matches!(bytes.get(4), Some(b'-')) || !matches!(bytes.get(7), Some(b'-')) {
        return None;
    }
    if !matches!(bytes.get(10), Some(b'T') | Some(b' ')) {
        return None;
    }

    let hour: i64 = value.get(11..13)?.parse().ok()?;
    let minute: i64 = value.get(14..16)?.parse().ok()?;
    let second: i64 = value.get(17..19)?.parse().ok()?;
    if !matches!(bytes.get(13), Some(b':')) || !matches!(bytes.get(16), Some(b':')) {
        return None;
    }

    let mut millis = 0_u64;
    let mut index = 19;
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let mut factor = 100_u64;
        while index < bytes.len() && bytes[index].is_ascii_digit() && factor > 0 {
            millis += ((bytes[index] - b'0') as u64) * factor;
            factor /= 10;
            index += 1;
        }
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
    }

    let mut offset_seconds: i64 = 0;
    match bytes.get(index) {
        Some(b'Z') | Some(b'z') => {}
        Some(sign @ (b'+' | b'-')) => {
            let rest = &value[index + 1..];
            // take(5): offsets like "+02:00" embed a colon among the digits.
            let digits: String = rest.chars().take(5).filter(|c| c.is_ascii_digit()).collect();
            if digits.len() >= 4 {
                let offset_hours: i64 = digits[0..2].parse().ok()?;
                let offset_minutes: i64 = digits[2..4].parse().ok()?;
                offset_seconds = offset_hours * 3600 + offset_minutes * 60;
                if *sign == b'-' {
                    offset_seconds = -offset_seconds;
                }
            }
        }
        _ => {
            // No zone marker (Dropbox sends "... UTC" or a bare value); assume UTC.
        }
    }

    let days = days_from_civil(year, month, day);
    let seconds = days * 86400 + hour * 3600 + minute * 60 + second - offset_seconds;
    if seconds < 0 {
        return None;
    }
    Some((seconds as u64) * 1000 + millis)
}

/// Days between 1970-01-01 and the civil date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146097 + day_of_era - 719468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_reserved_characters_for_urls() {
        assert_eq!(
            url_encode("https://www.googleapis.com/auth/drive openid"),
            "https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fdrive%20openid"
        );
        assert_eq!(url_encode("a-b_c.~"), "a-b_c.~");
    }

    #[test]
    fn parses_provider_timestamps() {
        // RFC3339 with Z (Google Drive / Graph).
        assert_eq!(
            parse_timestamp("2024-01-02T03:04:05.678Z"),
            Some(1704164645678)
        );
        // No fraction, explicit offset.
        assert_eq!(parse_timestamp("2024-01-02T05:04:05+02:00"), Some(1704164645000));
        // Dropbox's space-separated UTC form.
        assert_eq!(parse_timestamp("2024-01-02 03:04:05"), Some(1704164645000));
        assert_eq!(parse_timestamp("not a date"), None);
    }

    #[test]
    fn token_errors_carry_the_401_prefix() {
        let error = map_status(reqwest::StatusCode::UNAUTHORIZED, "{\"err\":\"expired\"}");
        assert!(is_token_error(&error));
        let denied = FileSystemError::PermissionDenied("quota".into());
        assert!(!is_token_error(&denied));
    }
}
