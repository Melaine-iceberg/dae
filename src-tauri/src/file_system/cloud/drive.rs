//! Google Drive provider: OAuth2 + Drive v3 REST. Folder ids are opaque
//! strings; the drive root is addressed as `root`.

use super::provider::{
    CloudMeta, CloudProvider, TokenSet, map_status, map_token_error, parse_timestamp,
    text_checked, url_encode,
};
use crate::file_system::error::FileSystemError;
use crate::file_system::types::EntryKind;
use async_trait::async_trait;
use serde::Deserialize;
use std::path::Path;
use std::sync::OnceLock;

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const API: &str = "https://www.googleapis.com/drive/v3";
const UPLOAD_API: &str = "https://www.googleapis.com/upload/drive/v3";
const SCOPE: &str = "https://www.googleapis.com/auth/drive";
const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
const FILE_FIELDS: &str = "id,name,mimeType,size,modifiedTime,parents";
/// Resumable uploads signal "send more" with a 308; a redirect-following
/// client would resend the chunk, so uploads use their own client.
fn upload_client() -> Result<reqwest::Client, FileSystemError> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    let built = CLIENT.get_or_init(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|error| error.to_string())
    });
    built
        .clone()
        .map_err(|error| FileSystemError::Internal(format!("Could not build the HTTP client: {error}")))
}

const UPLOAD_CHUNK_BYTES: usize = 8 * 1024 * 1024;

pub struct DriveProvider;

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
    #[serde(default)]
    refresh_token: Option<String>,
}

#[derive(Deserialize)]
struct FileList {
    #[serde(default)]
    next_page_token: Option<String>,
    #[serde(default)]
    files: Vec<DriveFile>,
}

#[derive(Deserialize)]
struct DriveFile {
    id: String,
    name: String,
    #[serde(rename = "mimeType", default)]
    mime_type: String,
    /// Drive reports sizes as strings.
    #[serde(default)]
    size: Option<String>,
    #[serde(rename = "modifiedTime", default)]
    modified_time: Option<String>,
    #[serde(default)]
    parents: Option<Vec<String>>,
}

impl DriveFile {
    fn to_meta(self) -> CloudMeta {
        let kind = if self.mime_type == FOLDER_MIME {
            EntryKind::Directory
        } else {
            EntryKind::File
        };
        CloudMeta {
            id: self.id,
            name: self.name,
            kind,
            size: self.size.and_then(|size| size.parse().ok()).unwrap_or(0),
            modified_at: self.modified_time.as_deref().and_then(parse_timestamp),
            parent_id: self.parents.and_then(|parents| parents.into_iter().next()),
            path: None,
        }
    }
}

#[derive(Deserialize)]
struct About {
    user: AboutUser,
}

#[derive(Deserialize)]
struct AboutUser {
    #[serde(rename = "emailAddress", default)]
    email_address: String,
    #[serde(rename = "displayName", default)]
    display_name: String,
}

async fn token_request(client: &reqwest::Client, form: Vec<(&str, &str)>) -> Result<TokenSet, FileSystemError> {
    let response = client
        .post(TOKEN_URL)
        .form(&form)
        .send()
        .await
        .map_err(|error| FileSystemError::Io(format!("Could not reach the token endpoint: {error}")))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(map_token_error(status, &text));
    }
    let parsed: TokenResponse = serde_json::from_str(&text)
        .map_err(|error| FileSystemError::Internal(format!("Unreadable token response: {error}")))?;
    Ok(TokenSet {
        access_token: parsed.access_token,
        expires_in_secs: parsed.expires_in,
        refresh_token: parsed.refresh_token,
    })
}

#[async_trait]
impl CloudProvider for DriveProvider {
    fn authorize_url(
        &self,
        client_id: &str,
        redirect_uri: &str,
        state: &str,
        code_challenge: &str,
    ) -> String {
        format!(
            "{AUTH_URL}?client_id={}&redirect_uri={}&response_type=code&scope={}\
             &access_type=offline&prompt=consent&state={}&code_challenge={}&code_challenge_method=S256",
            url_encode(client_id),
            url_encode(redirect_uri),
            url_encode(SCOPE),
            url_encode(state),
            url_encode(code_challenge),
        )
    }

    async fn exchange_code(
        &self,
        client: &reqwest::Client,
        client_id: &str,
        client_secret: Option<&str>,
        code: &str,
        redirect_uri: &str,
        code_verifier: &str,
    ) -> Result<TokenSet, FileSystemError> {
        let mut form = vec![
            ("code", code),
            ("client_id", client_id),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
            ("code_verifier", code_verifier),
        ];
        let secret_holder;
        if let Some(secret) = client_secret {
            secret_holder = secret.to_owned();
            form.push(("client_secret", &secret_holder));
        }
        token_request(client, form).await
    }

    async fn refresh_token(
        &self,
        client: &reqwest::Client,
        client_id: &str,
        client_secret: Option<&str>,
        refresh_token: &str,
    ) -> Result<TokenSet, FileSystemError> {
        let mut form = vec![
            ("refresh_token", refresh_token),
            ("client_id", client_id),
            ("grant_type", "refresh_token"),
        ];
        let secret_holder;
        if let Some(secret) = client_secret {
            secret_holder = secret.to_owned();
            form.push(("client_secret", &secret_holder));
        }
        token_request(client, form).await
    }

    async fn account_info(
        &self,
        client: &reqwest::Client,
        access_token: &str,
    ) -> Result<(String, String), FileSystemError> {
        let text = text_checked(
            client
                .get(format!("{API}/about"))
                .query(&[("fields", "user(emailAddress,displayName)")])
                .bearer_auth(access_token)
                .send()
                .await
                .map_err(|error| FileSystemError::Io(format!("Could not reach Google Drive: {error}")))?,
        )
        .await?;
        let about: About = serde_json::from_str(&text)
            .map_err(|error| FileSystemError::Internal(format!("Unreadable account info: {error}")))?;
        let display = if about.user.display_name.is_empty() {
            about.user.email_address.clone()
        } else {
            about.user.display_name
        };
        Ok((about.user.email_address, display))
    }

    async fn list(
        &self,
        client: &reqwest::Client,
        token: &str,
        folder_id: &str,
    ) -> Result<Vec<CloudMeta>, FileSystemError> {
        let parent = if folder_id.is_empty() { "root" } else { folder_id };
        let query_filter = format!("'{parent}' in parents and trashed = false");

        let mut entries = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let mut query: Vec<(&str, String)> = vec![
                ("q", query_filter.clone()),
                ("pageSize", "1000".to_owned()),
                ("fields", format!("nextPageToken,files({FILE_FIELDS})")),
                ("supportsAllDrives", "true".to_owned()),
            ];
            if let Some(token) = &page_token {
                query.push(("pageToken", token.clone()));
            }

            let text = text_checked(
                client
                    .get(format!("{API}/files"))
                    .query(&query)
                    .bearer_auth(token)
                    .send()
                    .await
                    .map_err(|error| FileSystemError::Io(format!("Could not reach Google Drive: {error}")))?,
            )
            .await?;
            let page: FileList = serde_json::from_str(&text)
                .map_err(|error| FileSystemError::Internal(format!("Unreadable listing: {error}")))?;

            entries.extend(page.files.into_iter().map(DriveFile::to_meta));
            match page.next_page_token {
                Some(next) => page_token = Some(next),
                None => return Ok(entries),
            }
        }
    }

    async fn metadata(
        &self,
        client: &reqwest::Client,
        token: &str,
        id: &str,
    ) -> Result<CloudMeta, FileSystemError> {
        let text = text_checked(
            client
                .get(format!("{API}/files/{id}"))
                .query(&[
                    ("fields", FILE_FIELDS.to_owned()),
                    ("supportsAllDrives", "true".to_owned()),
                ])
                .bearer_auth(token)
                .send()
                .await
                .map_err(|error| FileSystemError::Io(format!("Could not reach Google Drive: {error}")))?,
        )
        .await?;
        let file: DriveFile = serde_json::from_str(&text)
            .map_err(|error| FileSystemError::Internal(format!("Unreadable metadata: {error}")))?;
        Ok(file.to_meta())
    }

    async fn create_folder(
        &self,
        client: &reqwest::Client,
        token: &str,
        parent_id: &str,
        name: &str,
    ) -> Result<CloudMeta, FileSystemError> {
        let parent = if parent_id.is_empty() { "root" } else { parent_id };
        let text = text_checked(
            client
                .post(format!("{API}/files"))
                .query(&[("fields", FILE_FIELDS.to_owned())])
                .bearer_auth(token)
                .json(&serde_json::json!({
                    "name": name,
                    "parents": [parent],
                    "mimeType": FOLDER_MIME,
                }))
                .send()
                .await
                .map_err(|error| FileSystemError::Io(format!("Could not reach Google Drive: {error}")))?,
        )
        .await?;
        let file: DriveFile = serde_json::from_str(&text)
            .map_err(|error| FileSystemError::Internal(format!("Unreadable metadata: {error}")))?;
        Ok(file.to_meta())
    }

    async fn rename(
        &self,
        client: &reqwest::Client,
        token: &str,
        id: &str,
        new_name: &str,
    ) -> Result<(), FileSystemError> {
        text_checked(
            client
                .patch(format!("{API}/files/{id}"))
                .query(&[("fields", "id")])
                .bearer_auth(token)
                .json(&serde_json::json!({ "name": new_name }))
                .send()
                .await
                .map_err(|error| FileSystemError::Io(format!("Could not reach Google Drive: {error}")))?,
        )
        .await?;
        Ok(())
    }

    async fn move_to(
        &self,
        client: &reqwest::Client,
        token: &str,
        id: &str,
        dest_parent_id: &str,
        new_name: &str,
    ) -> Result<(), FileSystemError> {
        let current = self.metadata(client, token, id).await?;
        let dest_parent = if dest_parent_id.is_empty() { "root".to_owned() } else { dest_parent_id.to_owned() };
        let mut query = vec![
            ("fields", "id".to_owned()),
            ("addParents", dest_parent),
            ("supportsAllDrives", "true".to_owned()),
        ];
        if let Some(old_parent) = current.parent_id {
            query.push(("removeParents", old_parent));
        }

        text_checked(
            client
                .patch(format!("{API}/files/{id}"))
                .query(&query)
                .bearer_auth(token)
                .json(&serde_json::json!({ "name": new_name }))
                .send()
                .await
                .map_err(|error| FileSystemError::Io(format!("Could not reach Google Drive: {error}")))?,
        )
        .await?;
        Ok(())
    }

    async fn delete(
        &self,
        client: &reqwest::Client,
        token: &str,
        id: &str,
    ) -> Result<(), FileSystemError> {
        text_checked(
            client
                .delete(format!("{API}/files/{id}"))
                .query(&[("supportsAllDrives", "true")])
                .bearer_auth(token)
                .send()
                .await
                .map_err(|error| FileSystemError::Io(format!("Could not reach Google Drive: {error}")))?,
        )
        .await?;
        Ok(())
    }

    async fn download(
        &self,
        client: &reqwest::Client,
        token: &str,
        id: &str,
    ) -> Result<reqwest::Response, FileSystemError> {
        let response = client
            .get(format!("{API}/files/{id}"))
            .query(&[("alt", "media"), ("supportsAllDrives", "true")])
            .bearer_auth(token)
            .send()
            .await
            .map_err(|error| FileSystemError::Io(format!("Could not reach Google Drive: {error}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(map_status(status, &text));
        }
        Ok(response)
    }

    async fn upload(
        &self,
        client: &reqwest::Client,
        token: &str,
        dest_parent_id: &str,
        name: &str,
        source: &Path,
        size: u64,
    ) -> Result<CloudMeta, FileSystemError> {
        // Drive tolerates duplicate names, so an existing same-name entry
        // must be removed explicitly to keep "create or truncate" semantics.
        let parent = if dest_parent_id.is_empty() { "root" } else { dest_parent_id };
        for sibling in self.list(client, token, dest_parent_id).await? {
            if sibling.name == name {
                self.delete(client, token, &sibling.id).await?;
            }
        }

        // Start a resumable session.
        let start = client
            .post(format!("{UPLOAD_API}/files"))
            .query(&[("uploadType", "resumable"), ("fields", FILE_FIELDS)])
            .bearer_auth(token)
            .json(&serde_json::json!({ "name": name, "parents": [parent] }))
            .send()
            .await
            .map_err(|error| FileSystemError::Io(format!("Could not reach Google Drive: {error}")))?;
        if !start.status().is_success() {
            let status = start.status();
            let text = start.text().await.unwrap_or_default();
            return Err(map_status(status, &text));
        }
        let session_url = start
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| FileSystemError::Internal("Google Drive returned no upload session URL".into()))?
            .to_owned();

        let upload_client = upload_client()?;
        let mut final_text = String::new();

        if size == 0 {
            let response = upload_client
                .put(&session_url)
                .header(reqwest::header::CONTENT_RANGE, "bytes */0")
                .header(reqwest::header::CONTENT_LENGTH, "0")
                .bearer_auth(token)
                .send()
                .await
                .map_err(|error| FileSystemError::Io(format!("Upload failed: {error}")))?;
            final_text = text_checked(response).await?;
        } else {
            let mut file = std::fs::File::open(source).map_err(|error| {
                FileSystemError::Io(format!("Could not read the staged upload file: {error}"))
            })?;
            let mut offset: u64 = 0;
            let mut chunk = vec![0_u8; UPLOAD_CHUNK_BYTES];
            loop {
                let read = super::provider::read_chunk(&mut file, &mut chunk)?;
                if read == 0 {
                    break;
                }
                let end = offset + read as u64 - 1;
                let is_last = offset + read as u64 == size;
                let range = if is_last {
                    format!("bytes {offset}-{end}/{size}")
                } else {
                    format!("bytes {offset}-{end}/*")
                };

                let response = upload_client
                    .put(&session_url)
                    .header(reqwest::header::CONTENT_RANGE, range)
                    .bearer_auth(token)
                    .body(chunk[..read].to_vec())
                    .send()
                    .await
                    .map_err(|error| FileSystemError::Io(format!("Upload failed: {error}")))?;

                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                if status.is_success() {
                    final_text = text;
                    break;
                }
                if status.as_u16() != 308 {
                    return Err(map_status(status, &text));
                }
                offset += read as u64;
            }
        }

        let file: DriveFile = serde_json::from_str(&final_text)
            .map_err(|error| FileSystemError::Internal(format!("Unreadable upload result: {error}")))?;
        Ok(file.to_meta())
    }
}
