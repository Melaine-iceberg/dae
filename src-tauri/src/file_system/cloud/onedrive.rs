//! OneDrive provider: OAuth2 via the Microsoft identity platform + Microsoft
//! Graph Drive API. Item ids are opaque strings; the drive root is `root`.

use super::provider::{
    CloudMeta, CloudProvider, TokenSet, map_status, map_token_error, parse_timestamp, read_chunk,
    text_checked, url_encode,
};
use crate::file_system::error::FileSystemError;
use crate::file_system::types::EntryKind;
use async_trait::async_trait;
use serde::Deserialize;
use std::path::Path;

const AUTH_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/authorize";
const TOKEN_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/token";
const GRAPH: &str = "https://graph.microsoft.com/v1.0";
const SCOPE: &str = "offline_access Files.ReadWrite.All User.Read";
/// Upload chunks must be multiples of 320 KiB (327,680 bytes); 5 MiB divides
/// evenly (16 chunks) and is Microsoft's recommended size.
const UPLOAD_CHUNK_BYTES: usize = 5 * 1024 * 1024;

pub struct OnedriveProvider;

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
    /// OneDrive rotates refresh tokens on every use; a missing field means
    /// the previous token stays valid.
    #[serde(default)]
    refresh_token: Option<String>,
}

#[derive(Deserialize)]
struct GraphUser {
    #[serde(rename = "userPrincipalName", default)]
    user_principal_name: String,
    #[serde(default)]
    mail: Option<String>,
    #[serde(rename = "displayName", default)]
    display_name: String,
}

#[derive(Deserialize)]
struct GraphChildren {
    #[serde(default)]
    value: Vec<GraphItem>,
    #[serde(rename = "@odata.nextLink", default)]
    next_link: Option<String>,
}

#[derive(Deserialize)]
struct GraphItem {
    id: String,
    name: String,
    /// Present (as an empty object) when the item is a folder.
    #[serde(default)]
    folder: Option<serde_json::Value>,
    #[serde(default)]
    size: Option<u64>,
    #[serde(rename = "lastModifiedDateTime", default)]
    last_modified: Option<String>,
    #[serde(rename = "parentReference", default)]
    parent_reference: Option<ParentReference>,
}

#[derive(Deserialize)]
struct ParentReference {
    #[serde(default)]
    id: Option<String>,
}

impl GraphItem {
    fn to_meta(self) -> CloudMeta {
        CloudMeta {
            id: self.id,
            name: self.name,
            kind: if self.folder.is_some() {
                EntryKind::Directory
            } else {
                EntryKind::File
            },
            size: self.size.unwrap_or(0),
            modified_at: self.last_modified.as_deref().and_then(parse_timestamp),
            parent_id: self.parent_reference.and_then(|reference| reference.id),
            path: None,
        }
    }
}

#[derive(Deserialize)]
struct UploadSession {
    #[serde(rename = "uploadUrl")]
    upload_url: String,
}

fn rootish(id: &str) -> bool {
    id.is_empty() || id == "root"
}

/// Address of `{parent}'s child named {name}` in Graph's item-path syntax,
/// used by the content upload and upload-session endpoints.
fn child_address(parent_id: &str, name: &str, operation: &str) -> String {
    let encoded = url_encode(name);
    if rootish(parent_id) {
        format!("{GRAPH}/me/drive/root:/{encoded}:/{operation}")
    } else {
        format!("{GRAPH}/me/drive/items/{parent_id}:/{encoded}:/{operation}")
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
impl CloudProvider for OnedriveProvider {
    fn authorize_url(
        &self,
        client_id: &str,
        redirect_uri: &str,
        state: &str,
        code_challenge: &str,
    ) -> String {
        format!(
            "{AUTH_URL}?client_id={}&redirect_uri={}&response_type=code&scope={}\
             &state={}&code_challenge={}&code_challenge_method=S256",
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
                .get(format!("{GRAPH}/me"))
                .bearer_auth(access_token)
                .send()
                .await
                .map_err(|error| FileSystemError::Io(format!("Could not reach OneDrive: {error}")))?,
        )
        .await?;
        let user: GraphUser = serde_json::from_str(&text)
            .map_err(|error| FileSystemError::Internal(format!("Unreadable account info: {error}")))?;
        let email = user
            .mail
            .filter(|mail| !mail.is_empty())
            .unwrap_or(user.user_principal_name);
        let display = if user.display_name.is_empty() {
            email.clone()
        } else {
            user.display_name
        };
        Ok((email, display))
    }

    async fn list(
        &self,
        client: &reqwest::Client,
        token: &str,
        folder_id: &str,
    ) -> Result<Vec<CloudMeta>, FileSystemError> {
        let mut url = if rootish(folder_id) {
            format!("{GRAPH}/me/drive/root/children?$top=200")
        } else {
            format!("{GRAPH}/me/drive/items/{folder_id}/children?$top=200")
        };

        let mut entries = Vec::new();
        loop {
            let text = text_checked(
                client
                    .get(&url)
                    .bearer_auth(token)
                    .send()
                    .await
                    .map_err(|error| FileSystemError::Io(format!("Could not reach OneDrive: {error}")))?,
            )
            .await?;
            let page: GraphChildren = serde_json::from_str(&text)
                .map_err(|error| FileSystemError::Internal(format!("Unreadable listing: {error}")))?;

            entries.extend(page.value.into_iter().map(GraphItem::to_meta));
            match page.next_link {
                Some(next) => url = next,
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
        let url = if rootish(id) {
            format!("{GRAPH}/me/drive/root")
        } else {
            format!("{GRAPH}/me/drive/items/{id}")
        };
        let text = text_checked(
            client
                .get(&url)
                .bearer_auth(token)
                .send()
                .await
                .map_err(|error| FileSystemError::Io(format!("Could not reach OneDrive: {error}")))?,
        )
        .await?;
        let item: GraphItem = serde_json::from_str(&text)
            .map_err(|error| FileSystemError::Internal(format!("Unreadable metadata: {error}")))?;
        Ok(item.to_meta())
    }

    async fn create_folder(
        &self,
        client: &reqwest::Client,
        token: &str,
        parent_id: &str,
        name: &str,
    ) -> Result<CloudMeta, FileSystemError> {
        let url = if rootish(parent_id) {
            format!("{GRAPH}/me/drive/root/children")
        } else {
            format!("{GRAPH}/me/drive/items/{parent_id}/children")
        };
        let text = text_checked(
            client
                .post(&url)
                .bearer_auth(token)
                .json(&serde_json::json!({
                    "name": name,
                    "folder": {},
                    "@microsoft.graph.conflictBehavior": "fail",
                }))
                .send()
                .await
                .map_err(|error| FileSystemError::Io(format!("Could not reach OneDrive: {error}")))?,
        )
        .await?;
        let item: GraphItem = serde_json::from_str(&text)
            .map_err(|error| FileSystemError::Internal(format!("Unreadable metadata: {error}")))?;
        Ok(item.to_meta())
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
                .patch(format!("{GRAPH}/me/drive/items/{id}"))
                .bearer_auth(token)
                .json(&serde_json::json!({ "name": new_name }))
                .send()
                .await
                .map_err(|error| FileSystemError::Io(format!("Could not reach OneDrive: {error}")))?,
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
        // PATCH wants the destination folder's real id; "root" needs resolving.
        let dest_parent = if rootish(dest_parent_id) {
            self.metadata(client, token, "").await?.id
        } else {
            dest_parent_id.to_owned()
        };

        text_checked(
            client
                .patch(format!("{GRAPH}/me/drive/items/{id}"))
                .bearer_auth(token)
                .json(&serde_json::json!({
                    "name": new_name,
                    "parentReference": { "id": dest_parent },
                }))
                .send()
                .await
                .map_err(|error| FileSystemError::Io(format!("Could not reach OneDrive: {error}")))?,
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
        // The item lands in OneDrive's own recycle bin, where the user can
        // restore it from the web UI.
        let response = client
            .delete(format!("{GRAPH}/me/drive/items/{id}"))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|error| FileSystemError::Io(format!("Could not reach OneDrive: {error}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(map_status(status, &text));
        }
        Ok(())
    }

    async fn download(
        &self,
        client: &reqwest::Client,
        token: &str,
        id: &str,
    ) -> Result<reqwest::Response, FileSystemError> {
        // Responds with a redirect to a temporary pre-authenticated URL; the
        // shared client follows it.
        let response = client
            .get(format!("{GRAPH}/me/drive/items/{id}/content"))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|error| FileSystemError::Io(format!("Could not reach OneDrive: {error}")))?;
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
        if size == 0 {
            // A session cannot carry a zero-length range; the simple content
            // PUT creates the empty file in one request.
            let text = text_checked(
                client
                    .put(child_address(dest_parent_id, name, "content"))
                    .query(&[("@microsoft.graph.conflictBehavior", "replace")])
                    .bearer_auth(token)
                    .body(Vec::new())
                    .send()
                    .await
                    .map_err(|error| FileSystemError::Io(format!("Upload failed: {error}")))?,
            )
            .await?;
            let item: GraphItem = serde_json::from_str(&text)
                .map_err(|error| FileSystemError::Internal(format!("Unreadable upload result: {error}")))?;
            return Ok(item.to_meta());
        }

        let session_text = text_checked(
            client
                .post(child_address(dest_parent_id, name, "createUploadSession"))
                .bearer_auth(token)
                .json(&serde_json::json!({
                    "item": { "@microsoft.graph.conflictBehavior": "replace" },
                }))
                .send()
                .await
                .map_err(|error| FileSystemError::Io(format!("Upload failed: {error}")))?,
        )
        .await?;
        let session: UploadSession = serde_json::from_str(&session_text)
            .map_err(|error| FileSystemError::Internal(format!("Unreadable upload session: {error}")))?;

        let mut file = std::fs::File::open(source).map_err(|error| {
            FileSystemError::Io(format!("Could not read the staged upload file: {error}"))
        })?;
        let mut offset: u64 = 0;
        let mut chunk = vec![0_u8; UPLOAD_CHUNK_BYTES];
        loop {
            let read = read_chunk(&mut file, &mut chunk)?;
            if read == 0 {
                return Err(FileSystemError::Io(
                    "The staged upload file ended before its announced size".into(),
                ));
            }
            let end = offset + read as u64 - 1;

            // The upload URL is pre-authenticated; no bearer header needed.
            let response = client
                .put(&session.upload_url)
                .header(reqwest::header::CONTENT_RANGE, format!("bytes {offset}-{end}/{size}"))
                .body(chunk[..read].to_vec())
                .send()
                .await
                .map_err(|error| FileSystemError::Io(format!("Upload failed: {error}")))?;

            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            if status.as_u16() == 200 || status.as_u16() == 201 {
                let item: GraphItem = serde_json::from_str(&text)
                    .map_err(|error| FileSystemError::Internal(format!("Unreadable upload result: {error}")))?;
                return Ok(item.to_meta());
            }
            if status.as_u16() != 202 {
                return Err(map_status(status, &text));
            }
            offset += read as u64;
        }
    }
}
