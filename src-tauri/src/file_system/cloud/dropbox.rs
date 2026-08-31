//! Dropbox provider: OAuth2 + API v2 RPC + content endpoints. Entry ids
//! (`id:...`) are stable; write endpoints need display paths, resolved on
//! demand via `get_metadata`.

use super::provider::{
    CloudMeta, CloudProvider, TokenSet, map_status, map_token_error, parse_timestamp, read_chunk,
    url_encode,
};
use crate::file_system::error::FileSystemError;
use crate::file_system::types::EntryKind;
use async_trait::async_trait;
use serde::Deserialize;
use std::path::Path;

const AUTH_URL: &str = "https://www.dropbox.com/oauth2/authorize";
const TOKEN_URL: &str = "https://api.dropboxapi.com/oauth2/token";
const API: &str = "https://api.dropboxapi.com/2";
const CONTENT_API: &str = "https://content.dropboxapi.com/2";
const SCOPE: &str = "account_info.read files.content.read files.content.write \
                     files.metadata.read files.metadata.write files.sharing.write";
/// Well under the 150 MiB per-request limit of the session endpoints.
const UPLOAD_CHUNK_BYTES: usize = 8 * 1024 * 1024;

pub struct DropboxProvider;

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
    /// Returned when token rotation is enabled for the app; the stored
    /// refresh token must be replaced by it.
    #[serde(default)]
    refresh_token: Option<String>,
}

#[derive(Deserialize)]
struct ListFolderResult {
    #[serde(default)]
    entries: Vec<DropboxEntry>,
    cursor: String,
    #[serde(default)]
    has_more: bool,
}

#[derive(Deserialize)]
struct DropboxEntry {
    /// "file", "folder", or "deleted".
    #[serde(rename = ".tag", default)]
    tag: String,
    /// Missing for entries in some shared/mounted folders; the display path
    /// doubles as a stable address then.
    #[serde(default)]
    id: String,
    name: String,
    #[serde(default)]
    path_display: Option<String>,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    server_modified: Option<String>,
}

impl DropboxEntry {
    fn to_meta(self, parent_id: &str) -> Option<CloudMeta> {
        let kind = match self.tag.as_str() {
            "folder" => EntryKind::Directory,
            "file" => EntryKind::File,
            _ => return None,
        };
        let id = if self.id.is_empty() {
            self.path_display.clone().unwrap_or_default()
        } else {
            self.id
        };
        Some(CloudMeta {
            id,
            name: self.name,
            kind,
            size: self.size.unwrap_or(0),
            modified_at: self.server_modified.as_deref().and_then(parse_timestamp),
            parent_id: if parent_id.is_empty() {
                None
            } else {
                Some(parent_id.to_owned())
            },
            path: self.path_display,
        })
    }
}

#[derive(Deserialize)]
struct CreateFolderResult {
    metadata: DropboxEntry,
}

#[derive(Deserialize)]
struct CurrentAccount {
    #[serde(default)]
    email: String,
    name: AccountName,
}

#[derive(Deserialize)]
struct AccountName {
    #[serde(default)]
    display_name: String,
}

#[derive(Deserialize)]
struct UploadSessionStart {
    session_id: String,
}

/// Maps a Dropbox error response. Dropbox reports `path/not_found` with HTTP
/// 409 (the same code it uses for real conflicts), so the error summary has
/// to disambiguate before the generic status mapping runs.
fn map_dropbox_error(status: reqwest::StatusCode, text: &str) -> FileSystemError {
    if status.as_u16() == 401 {
        // Keeps the "401 " prefix the token vault uses to trigger refreshes.
        return map_status(status, text);
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
        let summary = value
            .get("error_summary")
            .and_then(|summary| summary.as_str())
            .unwrap_or("");

        if summary.contains("path/not_found") || summary.contains("lookup/not_found") {
            return FileSystemError::NotFound(summarize(summary));
        }
        if summary.contains("conflict") {
            return FileSystemError::AlreadyExists(summarize(summary));
        }
        if summary.contains("insufficient_space") {
            return FileSystemError::Io(format!("Dropbox is out of space: {}", summarize(summary)));
        }
    }
    map_status(status, text)
}

/// Validates a Dropbox RPC/content response and returns its body text.
async fn dropbox_checked(response: reqwest::Response) -> Result<String, FileSystemError> {
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if status.is_success() {
        Ok(text)
    } else {
        Err(map_dropbox_error(status, &text))
    }
}

fn summarize(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= 300 {
        trimmed.to_owned()
    } else {
        trimmed.chars().take(300).collect::<String>()
    }
}

async fn token_request(
    client: &reqwest::Client,
    form: Vec<(&str, &str)>,
) -> Result<TokenSet, FileSystemError> {
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
impl CloudProvider for DropboxProvider {
    fn authorize_url(
        &self,
        client_id: &str,
        redirect_uri: &str,
        state: &str,
        code_challenge: &str,
    ) -> String {
        format!(
            "{AUTH_URL}?client_id={}&redirect_uri={}&response_type=code&token_access_type=offline\
             &scope={}&state={}&code_challenge={}&code_challenge_method=S256",
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
        // Access tokens only live ~4 hours, so this path runs constantly;
        // with rotation enabled the response also carries the next refresh
        // token, which the account registry must persist.
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
        let response = client
            .post(format!("{API}/users/get_current_account"))
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|error| FileSystemError::Io(format!("Could not reach Dropbox: {error}")))?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(map_dropbox_error(status, &text));
        }
        let account: CurrentAccount = serde_json::from_str(&text)
            .map_err(|error| FileSystemError::Internal(format!("Unreadable account info: {error}")))?;
        let display = if account.name.display_name.is_empty() {
            account.email.clone()
        } else {
            account.name.display_name
        };
        Ok((account.email, display))
    }

    async fn list(
        &self,
        client: &reqwest::Client,
        token: &str,
        folder_id: &str,
    ) -> Result<Vec<CloudMeta>, FileSystemError> {
        let folder_path = self.display_path(client, token, folder_id).await?;

        let first_text = dropbox_checked(
            client
                .post(format!("{API}/files/list_folder"))
                .bearer_auth(token)
                .json(&serde_json::json!({ "path": folder_path, "limit": 1000 }))
                .send()
                .await
                .map_err(|error| FileSystemError::Io(format!("Could not reach Dropbox: {error}")))?,
        )
        .await?;
        let mut page: ListFolderResult = serde_json::from_str(&first_text)
            .map_err(|error| FileSystemError::Internal(format!("Unreadable listing: {error}")))?;

        let mut entries: Vec<CloudMeta> = page
            .entries
            .into_iter()
            .filter_map(|entry| entry.to_meta(folder_id))
            .collect();

        while page.has_more {
            let text = dropbox_checked(
                client
                    .post(format!("{API}/files/list_folder/continue"))
                    .bearer_auth(token)
                    .json(&serde_json::json!({ "cursor": page.cursor }))
                    .send()
                    .await
                    .map_err(|error| FileSystemError::Io(format!("Could not reach Dropbox: {error}")))?,
            )
            .await?;
            page = serde_json::from_str(&text)
                .map_err(|error| FileSystemError::Internal(format!("Unreadable listing: {error}")))?;
            entries.extend(
                page.entries
                    .into_iter()
                    .filter_map(|entry| entry.to_meta(folder_id)),
            );
        }

        Ok(entries)
    }

    async fn metadata(
        &self,
        client: &reqwest::Client,
        token: &str,
        id: &str,
    ) -> Result<CloudMeta, FileSystemError> {
        if id.is_empty() {
            // The root has no metadata record; synthesize it.
            return Ok(CloudMeta {
                id: String::new(),
                name: String::new(),
                kind: EntryKind::Directory,
                size: 0,
                modified_at: None,
                parent_id: None,
                path: Some(String::new()),
            });
        }
        let text = dropbox_checked(
            client
                .post(format!("{API}/files/get_metadata"))
                .bearer_auth(token)
                .json(&serde_json::json!({ "path": id }))
                .send()
                .await
                .map_err(|error| FileSystemError::Io(format!("Could not reach Dropbox: {error}")))?,
        )
        .await?;
        let entry: DropboxEntry = serde_json::from_str(&text)
            .map_err(|error| FileSystemError::Internal(format!("Unreadable metadata: {error}")))?;
        entry.to_meta("").ok_or_else(|| {
            FileSystemError::NotFound(format!("The Dropbox entry is gone: {id}"))
        })
    }

    async fn create_folder(
        &self,
        client: &reqwest::Client,
        token: &str,
        parent_id: &str,
        name: &str,
    ) -> Result<CloudMeta, FileSystemError> {
        let parent_path = self.display_path(client, token, parent_id).await?;
        let text = dropbox_checked(
            client
                .post(format!("{API}/files/create_folder_v2"))
                .bearer_auth(token)
                .json(&serde_json::json!({
                    "path": child_path(&parent_path, name),
                }))
                .send()
                .await
                .map_err(|error| FileSystemError::Io(format!("Could not reach Dropbox: {error}")))?,
        )
        .await?;
        let wrapped: CreateFolderResult = serde_json::from_str(&text)
            .map_err(|error| FileSystemError::Internal(format!("Unreadable metadata: {error}")))?;
        wrapped.metadata.to_meta(parent_id).ok_or_else(|| {
            FileSystemError::Internal("Dropbox returned no metadata for the created folder".into())
        })
    }

    async fn rename(
        &self,
        client: &reqwest::Client,
        token: &str,
        id: &str,
        new_name: &str,
    ) -> Result<(), FileSystemError> {
        let current = self.metadata(client, token, id).await?;
        let from = current.path.ok_or_else(|| {
            FileSystemError::Internal(format!("The Dropbox entry has no path: {id}"))
        })?;
        let to = child_path(&parent_path(&from)?, new_name);

        dropbox_checked(
            client
                .post(format!("{API}/files/move_v2"))
                .bearer_auth(token)
                .json(&serde_json::json!({ "from_path": from, "to_path": to }))
                .send()
                .await
                .map_err(|error| FileSystemError::Io(format!("Could not reach Dropbox: {error}")))?,
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
        let from = current.path.ok_or_else(|| {
            FileSystemError::Internal(format!("The Dropbox entry has no path: {id}"))
        })?;
        let dest_parent = self.display_path(client, token, dest_parent_id).await?;
        let to = child_path(&dest_parent, new_name);

        dropbox_checked(
            client
                .post(format!("{API}/files/move_v2"))
                .bearer_auth(token)
                .json(&serde_json::json!({ "from_path": from, "to_path": to }))
                .send()
                .await
                .map_err(|error| FileSystemError::Io(format!("Could not reach Dropbox: {error}")))?,
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
        dropbox_checked(
            client
                .post(format!("{API}/files/delete_v2"))
                .bearer_auth(token)
                .json(&serde_json::json!({ "path": id }))
                .send()
                .await
                .map_err(|error| FileSystemError::Io(format!("Could not reach Dropbox: {error}")))?,
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
            .post(format!("{CONTENT_API}/files/download"))
            .bearer_auth(token)
            .header(
                "Dropbox-API-Arg",
                serde_json::json!({ "path": id }).to_string(),
            )
            .send()
            .await
            .map_err(|error| FileSystemError::Io(format!("Could not reach Dropbox: {error}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(map_dropbox_error(status, &text));
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
        let parent_path = self.display_path(client, token, dest_parent_id).await?;
        let commit_path = child_path(&parent_path, name);

        let start_text = dropbox_checked(
            client
                .post(format!("{CONTENT_API}/files/upload_session/start"))
                .bearer_auth(token)
                .body(Vec::new())
                .send()
                .await
                .map_err(|error| FileSystemError::Io(format!("Upload failed: {error}")))?,
        )
        .await?;
        let session: UploadSessionStart = serde_json::from_str(&start_text)
            .map_err(|error| FileSystemError::Internal(format!("Unreadable upload session: {error}")))?;

        let mut file = std::fs::File::open(source).map_err(|error| {
            FileSystemError::Io(format!("Could not read the staged upload file: {error}"))
        })?;
        let mut offset: u64 = 0;
        let mut chunk = vec![0_u8; UPLOAD_CHUNK_BYTES];
        loop {
            let read = if offset < size {
                let read = read_chunk(&mut file, &mut chunk)?;
                if read == 0 {
                    return Err(FileSystemError::Io(
                        "The staged upload file ended before its announced size".into(),
                    ));
                }
                read
            } else {
                0
            };
            let is_last = offset + read as u64 == size;

            if !is_last {
                let arg = serde_json::json!({
                    "cursor": { "session_id": session.session_id, "offset": offset },
                    "close": false,
                });
                let response = client
                    .post(format!("{CONTENT_API}/files/upload_session/append_v2"))
                    .bearer_auth(token)
                    .header("Dropbox-API-Arg", arg.to_string())
                    .body(chunk[..read].to_vec())
                    .send()
                    .await
                    .map_err(|error| FileSystemError::Io(format!("Upload failed: {error}")))?;
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                if !status.is_success() {
                    return Err(map_dropbox_error(status, &text));
                }
                offset += read as u64;
                continue;
            }

            let arg = serde_json::json!({
                "cursor": { "session_id": session.session_id, "offset": offset },
                "commit": {
                    "path": commit_path,
                    "mode": "overwrite",
                    "autorename": false,
                },
                "close": true,
            });
            let response = client
                .post(format!("{CONTENT_API}/files/upload_session/finish"))
                .bearer_auth(token)
                .header("Dropbox-API-Arg", arg.to_string())
                .body(chunk[..read].to_vec())
                .send()
                .await
                .map_err(|error| FileSystemError::Io(format!("Upload failed: {error}")))?;
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            if !status.is_success() {
                return Err(map_dropbox_error(status, &text));
            }
            let entry: DropboxEntry = serde_json::from_str(&text)
                .map_err(|error| FileSystemError::Internal(format!("Unreadable upload result: {error}")))?;
            return entry.to_meta(dest_parent_id).ok_or_else(|| {
                FileSystemError::Internal("Dropbox returned no metadata for the uploaded file".into())
            });
        }
    }
}

impl DropboxProvider {
    /// Resolves an entry id to its display path ("" for the root), which the
    /// write endpoints require.
    async fn display_path(
        &self,
        client: &reqwest::Client,
        token: &str,
        id: &str,
    ) -> Result<String, FileSystemError> {
        if id.is_empty() {
            return Ok(String::new());
        }
        let meta = self.metadata(client, token, id).await?;
        meta.path.ok_or_else(|| {
            FileSystemError::Internal(format!("The Dropbox entry has no path: {id}"))
        })
    }
}

/// Joins a parent display path and a child name ("/" becomes "/name").
fn child_path(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    }
}

/// The parent of a display path, or `None`-ish "" for top-level entries.
fn parent_path(path: &str) -> Result<String, FileSystemError> {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) => Ok(String::new()),
        Some(index) => Ok(trimmed[..index].to_owned()),
        None => Err(FileSystemError::InvalidInput(format!(
            "The Dropbox root cannot be renamed: {path}"
        ))),
    }
}
