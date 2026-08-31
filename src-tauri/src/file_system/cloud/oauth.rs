//! Loopback OAuth for cloud providers: opens the system browser at the
//! provider's consent screen, receives the authorization code on a local
//! loopback listener, exchanges it for tokens, and saves the account.
//!
//! App credentials are supplied by the user per account (the app ships no
//! built-in client ids), so each provider's developer console is configured
//! with the fixed loopback redirect URI below.

use super::accounts::{self, StoredCloudAccount};
use super::provider::{self, CloudProviderKind};
use crate::file_system::error::FileSystemError;
use base64::Engine;
use rand::{Rng, distr::Alphanumeric};
use serde::Deserialize;
use specta::Type;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// The loopback redirect port. Fixed (not OS-assigned) because Dropbox
/// requires the exact redirect URI registered in its app console; Google and
/// Microsoft accept any loopback port, so a fixed one is the common
/// denominator. Only bound on 127.0.0.1.
pub const LOOPBACK_PORT: u16 = 51888;

pub fn redirect_uri() -> String {
    format!("http://127.0.0.1:{LOOPBACK_PORT}")
}

/// How long the loopback listener waits for the browser round trip.
const AUTH_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizeCloudAccountInput {
    pub provider: CloudProviderKind,
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
}

/// Runs the full browser authorization for one provider and saves the
/// resulting account. Re-authorizing an existing account replaces it.
#[tauri::command]
#[specta::specta]
pub async fn authorize_cloud_account(
    input: AuthorizeCloudAccountInput,
) -> Result<StoredCloudAccount, FileSystemError> {
    let client_id = input.client_id.trim().to_owned();
    if client_id.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "An OAuth client id is required".into(),
        ));
    }
    let client_secret = input
        .client_secret
        .map(|secret| secret.trim().to_owned())
        .filter(|secret| !secret.is_empty());

    let provider = provider::provider_for(input.provider);
    let verifier = random_token(64);
    let challenge = pkce_challenge(&verifier);
    let state = random_token(32);
    let redirect = redirect_uri();

    let url = provider.authorize_url(&client_id, &redirect, &state, &challenge);
    open_in_browser(&url)?;

    let code = wait_for_loopback_callback(&state).await?;

    let client = provider::http_client()?;
    let token_set = provider
        .exchange_code(
            &client,
            &client_id,
            client_secret.as_deref(),
            &code,
            &redirect,
            &verifier,
        )
        .await?;
    let (email, display_name) = provider
        .account_info(&client, &token_set.access_token)
        .await?;

    let email = email.trim().to_lowercase();
    if email.is_empty() {
        return Err(FileSystemError::Internal(
            "The provider did not report an account email".into(),
        ));
    }
    let refresh_token = token_set.refresh_token.ok_or_else(|| {
        FileSystemError::PermissionDenied(
            "The provider returned no refresh token. Remove the app's access in the provider's security settings and authorize again.".into(),
        )
    })?;

    let account = StoredCloudAccount {
        id: format!("cloud://{}:{email}", input.provider.as_str()),
        provider: input.provider,
        email,
        display_name,
    };
    accounts::upsert_account(
        account.clone(),
        &client_id,
        client_secret.as_deref(),
        &refresh_token,
    )?;
    Ok(account)
}

/// Binds the loopback listener, opens the browser, and waits for the
/// provider's redirect carrying `code` with a matching `state`.
async fn wait_for_loopback_callback(expected_state: &str) -> Result<String, FileSystemError> {
    let listener = bind_loopback().await?;

    let (mut socket, _) = tokio::time::timeout(AUTH_TIMEOUT, listener.accept())
        .await
        .map_err(|_| {
            FileSystemError::Io(
                "Timed out waiting for the browser authorization. Close this dialog and try again."
                    .into(),
            )
        })?
        .map_err(|error| {
            FileSystemError::Io(format!("The local authorization listener failed: {error}"))
        })?;

    // Only the request line matters; stop at the header/body boundary.
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let read = socket
            .read(&mut chunk)
            .await
            .map_err(|error| FileSystemError::Io(format!("Could not read the authorization callback: {error}")))?;
        if read == 0 || buffer.len() > 16 * 1024 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }

    let request = String::from_utf8_lossy(&buffer);
    let first_line = request.lines().next().unwrap_or_default();
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    if method != "GET" || target.is_empty() {
        let _ = respond(&mut socket, false, "Bad request.").await;
        return Err(FileSystemError::InvalidInput(
            "The authorization callback was malformed".into(),
        ));
    }

    let query = target.split_once('?').map(|(_, query)| query).unwrap_or_default();
    let params = parse_query(query);

    if let Some(error) = params.get("error") {
        let description = params.get("error_description").cloned().unwrap_or_default();
        let _ = respond(&mut socket, false, &format!("Authorization failed: {error}")).await;
        return Err(FileSystemError::PermissionDenied(format!(
            "The provider rejected the authorization: {error} {description}"
        )));
    }

    let code = params.get("code").cloned().ok_or_else(|| {
        FileSystemError::InvalidInput("The authorization callback carried no code".into())
    })?;
    let returned_state = params.get("state").cloned().unwrap_or_default();
    if returned_state != expected_state {
        let _ = respond(&mut socket, false, "State mismatch.").await;
        return Err(FileSystemError::InvalidInput(
            "The authorization callback state did not match; the response was ignored".into(),
        ));
    }

    let _ = respond(&mut socket, true, "").await;
    Ok(code)
}

async fn bind_loopback() -> Result<TcpListener, FileSystemError> {
    let address = format!("127.0.0.1:{LOOPBACK_PORT}");
    let mut last_error = None;
    for _ in 0..3 {
        match TcpListener::bind(&address).await {
            Ok(listener) => return Ok(listener),
            Err(error) => {
                last_error = Some(error);
                tokio::time::sleep(Duration::from_millis(400)).await;
            }
        }
    }
    Err(FileSystemError::Io(format!(
        "Could not listen on {address} for the authorization callback (is another instance running?): {}",
        last_error.map(|error| error.to_string()).unwrap_or_default()
    )))
}

/// Minimal HTML reply so the browser shows a closing hint instead of an error.
async fn respond(socket: &mut tokio::net::TcpStream, ok: bool, detail: &str) -> std::io::Result<()> {
    let body = if ok {
        "<!doctype html><meta charset=\"utf-8\"><title>dae</title>\
         <body style=\"font-family:system-ui,sans-serif;text-align:center;padding-top:12vh\">\
         <h3>Authorization complete</h3>\
         <p>You can close this page and return to the app.</p>\
         <p>授权完成，可以关闭此页面返回应用。</p></body>"
            .to_owned()
    } else {
        format!(
            "<!doctype html><meta charset=\"utf-8\"><title>dae</title>\
             <body style=\"font-family:system-ui,sans-serif;text-align:center;padding-top:12vh\">\
             <h3>Authorization failed</h3><p>{detail}</p>\
             <p>授权失败：{detail}</p></body>"
        )
    };
    let status = if ok { "200 OK" } else { "400 Bad Request" };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    socket.write_all(response.as_bytes()).await?;
    socket.shutdown().await
}

fn parse_query(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            Some((percent_decode(key), percent_decode(value)))
        })
        .collect()
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex_pair = |byte: u8| match byte {
                    b'0'..=b'9' => Some(byte - b'0'),
                    b'a'..=b'f' => Some(byte - b'a' + 10),
                    b'A'..=b'F' => Some(byte - b'A' + 10),
                    _ => None,
                };
                match (hex_pair(bytes[index + 1]), hex_pair(bytes[index + 2])) {
                    (Some(high), Some(low)) => {
                        decoded.push(high * 16 + low);
                        index += 3;
                    }
                    _ => {
                        decoded.push(b'%');
                        index += 1;
                    }
                }
            }
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            other => {
                decoded.push(other);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn random_token(length: usize) -> String {
    rand::rng()
        .sample_iter(Alphanumeric)
        .take(length)
        .map(char::from)
        .collect()
}

/// S256 PKCE challenge for `verifier` (RFC 7636).
fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

fn open_in_browser(url: &str) -> Result<(), FileSystemError> {
    let result = {
        #[cfg(target_os = "windows")]
        {
            // Quoted so `&` separators survive cmd's parsing; the empty ""
            // argument is `start`'s window title slot.
            std::process::Command::new("cmd.exe")
                .args(["/c", "start", "", &format!("\"{url}\"")])
                .spawn()
        }
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("open").arg(url).spawn()
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            std::process::Command::new("xdg-open").arg(url).spawn()
        }
    };
    result.map(|_| ()).map_err(|error| {
        FileSystemError::Io(format!("Could not open the browser for authorization: {error}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_the_rfc7636_example_challenge() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn parses_callback_queries() {
        let params = parse_query("code=abc%20def&state=x%2By&plus=a+b&ignored&empty=");
        assert_eq!(params.get("code").map(String::as_str), Some("abc def"));
        // %2B is a literal '+'; only a bare '+' means space.
        assert_eq!(params.get("state").map(String::as_str), Some("x+y"));
        assert_eq!(params.get("plus").map(String::as_str), Some("a b"));
        assert_eq!(params.get("empty").map(String::as_str), Some(""));
        assert!(!params.contains_key("ignored"));
    }

    #[test]
    fn builds_the_documented_redirect_uri() {
        assert_eq!(redirect_uri(), "http://127.0.0.1:51888");
    }
}
