//! The typed explorer path. Every string the shell hands the core — a drive
//! letter, a POSIX path, a UNC share, a `\\wsl$` root, an
//! `smb://user@nas:445/share/folder` URL — parses here exactly once, and the
//! canonical text is written here exactly once. The backends each used to keep
//! their own parser and their own `format!` serializer; that is how
//! `user@nas` drifted into a host name, `ftps` folded differently per module,
//! and a `[::1]:2121` authority split at the wrong bracket. One parser and
//! one serializer is the whole point of the type.
//!
//! Local paths are kept verbatim, `file://` prefix included: Windows accepts
//! both separators, and the text the user typed is the best identity the
//! shell has. Remote segments are normalized (`\` → `/`, empty, `.` and `..`
//! dropped) because they are rebuilt constantly, and a canonical form that
//! drifts is a wrong path. Cloud segments are opaque ids and are kept as
//! given apart from empty-segment collapse.
//!
//! A username in the authority (`ftp://alice@nas/…`) is an input hint for
//! the connection store, never part of the canonical text: canonical paths
//! travel through breadcrumbs, recents and the undo journal, and none of
//! those should learn a credential.

use super::cloud::provider::CloudProviderKind;
use super::error::FileSystemError;
use std::fmt;
use std::path::Path;

/// The storage protocol a path addresses. The spellings that fold in the
/// wire world (`ftps`→FTP-with-TLS, `webdavs`→WebDAV-with-TLS) stay distinct
/// here so the canonical text can round-trip and the FTP backend can pick
/// its TLS mode without re-sniffing the string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    /// Drive letters, POSIX paths, UNC shares, `\\wsl$` roots, `file://`.
    Local,
    Smb,
    Sftp,
    Ftp,
    Ftps,
    WebDav,
    WebDavS,
    Cloud,
    /// A read-only view inside an archive: `archive:<target>!/<inner>`.
    /// The target is itself an [`EntryUrl`]; browsing inside it is backed by
    /// the archive's own index (see `archive_view.rs`).
    Archive,
}

impl Scheme {
    /// The lowercase prefix spelling; `Local` writes no prefix.
    pub fn as_str(self) -> &'static str {
        match self {
            Scheme::Local => "file",
            Scheme::Smb => "smb",
            Scheme::Sftp => "sftp",
            Scheme::Ftp => "ftp",
            Scheme::Ftps => "ftps",
            Scheme::WebDav => "webdav",
            Scheme::WebDavS => "webdavs",
            Scheme::Cloud => "cloud",
            Scheme::Archive => "archive",
        }
    }

    /// Both FTP spellings: one protocol, two TLS modes.
    pub fn is_ftp(self) -> bool {
        matches!(self, Scheme::Ftp | Scheme::Ftps)
    }

    /// Both WebDAV spellings: one protocol, two TLS modes.
    pub fn is_webdav(self) -> bool {
        matches!(self, Scheme::WebDav | Scheme::WebDavS)
    }

    fn parse_prefix(prefix: &str) -> Result<Scheme, FileSystemError> {
        match prefix.to_ascii_lowercase().as_str() {
            "file" => Ok(Scheme::Local),
            "smb" => Ok(Scheme::Smb),
            "sftp" => Ok(Scheme::Sftp),
            "ftp" => Ok(Scheme::Ftp),
            "ftps" => Ok(Scheme::Ftps),
            "webdav" => Ok(Scheme::WebDav),
            "webdavs" => Ok(Scheme::WebDavS),
            "archive" => Ok(Scheme::Archive),
            "cloud" => Ok(Scheme::Cloud),
            other => Err(FileSystemError::InvalidInput(format!(
                "Unsupported storage protocol: {other}"
            ))),
        }
    }
}

/// The `[user@]host[:port]` an `smb`/`sftp`/`ftp`/`webdav` path connects to.
/// The username is parse-only: it guides the connection store and never
/// appears in [`EntryUrl`]'s canonical text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    pub username: Option<String>,
    pub host: String,
    pub port: Option<u16>,
}

impl Server {
    /// The canonical authority text: `host` or `host:port`, no user.
    pub fn authority_text(&self) -> String {
        match self.port {
            Some(port) => format!("{}:{}", self.host, port),
            None => self.host.clone(),
        }
    }
}

/// Where a path points, after one parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    /// The local disk, verbatim: the shell hands the core whatever shape the
    /// user typed, and Windows accepts them all.
    Local { path: String },
    /// `scheme://[user@]host[:port]/segments` beneath one server.
    Server {
        scheme: Scheme,
        server: Server,
        segments: Vec<String>,
    },
    /// `cloud://provider:email/id/…`. The segments are opaque entry ids (the
    /// last may be a display name), so unlike [`Location::Server`] they are
    /// kept as given — a `.` or `..` in an id stays.
    Cloud {
        provider: CloudProviderKind,
        email: String,
        segments: Vec<String>,
    },

    /// `archive:<target>!/<inner segments>`. The target is a fully parsed
    /// path — local first; a remote target parses but the backend rejects
    /// it, so the error stays one layer down where the reason is known.
    Archive {
        target: Box<EntryUrl>,
        segments: Vec<String>,
    },
}

/// A parsed explorer path. Parsing validates; the accessors cannot hand back
/// a host-less server or a malformed cloud account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryUrl {
    location: Location,
}

impl EntryUrl {
    /// Parses one path of any kind. An `archive:` form has no `://` of its
    /// own (`archive:/K:/a.zip!/x`), so it is recognized before the generic
    /// split; anything else with a `://` separator before it is a regular
    /// path that merely contains the bytes — `C:/notes on smb://`, the file —
    /// and stays local.
    pub fn parse(text: &str) -> Result<Self, FileSystemError> {
        if text.len() >= 8 && text[..8].eq_ignore_ascii_case("archive:") {
            return Self::parse_archive(&text[8..]);
        }
        let Some((prefix, rest)) = text.split_once("://") else {
            return Ok(Self {
                location: Location::Local {
                    path: text.to_owned(),
                },
            });
        };

        // A separator inside the prefix means the "://" was part of a name.
        if prefix.is_empty() || prefix.contains(['/', '\\']) {
            return Ok(Self {
                location: Location::Local {
                    path: text.to_owned(),
                },
            });
        }

        let scheme = Scheme::parse_prefix(prefix)?;
        if matches!(scheme, Scheme::Local) {
            // `file://` is a local path in URI clothing; keep the original
            // text so the round trip is exact.
            return Ok(Self {
                location: Location::Local {
                    path: text.to_owned(),
                },
            });
        }

        if scheme == Scheme::Cloud {
            return Self::parse_cloud(scheme, rest);
        }
        Self::parse_server(scheme, rest)
    }

    fn parse_server(scheme: Scheme, rest: &str) -> Result<Self, FileSystemError> {
        let (authority, remainder) = split_authority(rest);
        let (username, host_port) = match authority.rsplit_once('@') {
            Some((user, host)) if !user.is_empty() => (Some(user.to_owned()), host),
            _ => (None, authority),
        };
        let (host, port) = parse_authority(host_port)?;
        let segments = normalize_segments(remainder.unwrap_or(""));

        Ok(Self {
            location: Location::Server {
                scheme,
                server: Server {
                    username,
                    host,
                    port,
                },
                segments,
            },
        })
    }

    /// Parses `provider:email[/id/…]`. The split is on the FIRST colon:
    /// emails never contain colons, while an rsplit would mistake a domain
    /// suffix for a port.
    fn parse_cloud(scheme: Scheme, rest: &str) -> Result<Self, FileSystemError> {
        if scheme != Scheme::Cloud {
            return Err(FileSystemError::InvalidInput(format!(
                "Not a cloud path: {rest}"
            )));
        }
        let (authority, tail) = match rest.split_once('/') {
            Some((authority, tail)) => (authority, tail),
            None => (rest, ""),
        };
        let (provider, email) = authority.split_once(':').ok_or_else(|| {
            FileSystemError::InvalidInput(format!("Malformed cloud account path: cloud://{rest}"))
        })?;
        let provider = CloudProviderKind::parse(provider)?;
        if email.is_empty() {
            return Err(FileSystemError::InvalidInput(format!(
                "The cloud account has no email: {authority}"
            )));
        }

        Ok(Self {
            location: Location::Cloud {
                provider,
                email: email.to_owned(),
                segments: tail
                    .split('/')
                    .filter(|segment| !segment.is_empty())
                    .map(str::to_owned)
                    .collect(),
            },
        })
    }

    /// Parses the text after `archive:`: one `!` separates the target path
    /// from the inner segments. The first `!` wins — a target whose own name
    /// contains `!` is the one documented ambiguity of the form, and the
    /// canonical text always round-trips.
    fn parse_archive(rest: &str) -> Result<Self, FileSystemError> {
        if rest.starts_with("//") {
            return Err(FileSystemError::InvalidInput(format!(
                "fs.archive_scheme_form: {rest}"
            )));
        }
        let (target_text, inner) = rest.split_once('!').ok_or_else(|| {
            FileSystemError::InvalidInput(format!("fs.archive_missing_separator: {rest}"))
        })?;
        let target = Self::parse(target_text)?;
        if matches!(target.location, Location::Archive { .. }) {
            return Err(FileSystemError::InvalidInput(format!(
                "fs.archive_nested_unsupported: {rest}"
            )));
        }
        // `normalize_segments` clamps `..` at the archive root, so a member
        // can never address outside its own archive.
        Ok(Self {
            location: Location::Archive {
                target: Box::new(target),
                segments: normalize_segments(inner),
            },
        })
    }

    pub fn location(&self) -> &Location {
        &self.location
    }

    pub fn scheme(&self) -> Scheme {
        match &self.location {
            Location::Local { .. } => Scheme::Local,
            Location::Server { scheme, .. } => *scheme,
            Location::Cloud { .. } => Scheme::Cloud,
            Location::Archive { .. } => Scheme::Archive,
        }
    }

    pub fn is_local(&self) -> bool {
        matches!(self.location, Location::Local { .. })
    }

    /// The local path text, verbatim as given.
    pub fn local_path(&self) -> Option<&str> {
        match &self.location {
            Location::Local { path } => Some(path),
            _ => None,
        }
    }

    /// The server the path connects to, with its username hint.
    pub fn server(&self) -> Option<&Server> {
        match &self.location {
            Location::Server { server, .. } => Some(server),
            _ => None,
        }
    }

    /// The cloud account the path lives in.
    pub fn cloud_account(&self) -> Option<(CloudProviderKind, &str)> {
        match &self.location {
            Location::Cloud {
                provider, email, ..
            } => Some((*provider, email.as_str())),
            _ => None,
        }
    }

    /// The segments beneath the server root, the account root, or the archive
    /// root. Empty for a local path, which has no segments — its shape
    /// belongs to the OS.
    pub fn segments(&self) -> &[String] {
        match &self.location {
            Location::Local { .. } => &[],
            Location::Server { segments, .. } => segments,
            Location::Cloud { segments, .. } => segments,
            Location::Archive { segments, .. } => segments,
        }
    }

    /// The same location with `segments` beneath its root.
    pub fn with_segments(&self, segments: Vec<String>) -> Self {
        let location = match &self.location {
            Location::Local { .. } => return self.clone(),
            Location::Server { scheme, server, .. } => Location::Server {
                scheme: *scheme,
                server: server.clone(),
                segments,
            },
            Location::Cloud {
                provider, email, ..
            } => Location::Cloud {
                provider: *provider,
                email: email.clone(),
                segments,
            },
            Location::Archive { target, .. } => Location::Archive {
                target: target.clone(),
                segments,
            },
        };
        Self { location }
    }

    /// Appends `name` as one segment (or joins it onto a local path). The
    /// caller validates the name — the backends already reject separators
    /// before calling, and a name pushed here is never re-split.
    pub fn join(&self, name: &str) -> Self {
        match &self.location {
            Location::Local { path } => Self {
                location: Location::Local {
                    path: Path::new(path).join(name).to_string_lossy().into_owned(),
                },
            },
            Location::Server { .. } | Location::Cloud { .. } | Location::Archive { .. } => {
                let mut segments = self.segments().to_vec();
                segments.push(name.to_owned());
                self.with_segments(segments)
            }
        }
    }

    /// Splits off the last segment (or file name), returning the parent and
    /// the name. `None` at a root — a server root, an account root, or the
    /// top of a local path — where there is nothing to split.
    pub fn pop(&self) -> Option<(Self, String)> {
        match &self.location {
            Location::Local { path } => {
                let path = Path::new(path);
                let name = path.file_name()?.to_string_lossy().into_owned();
                let parent = path.parent()?.to_string_lossy().into_owned();
                Some((
                    Self {
                        location: Location::Local { path: parent },
                    },
                    name,
                ))
            }
            Location::Server { .. } | Location::Cloud { .. } | Location::Archive { .. } => {
                let mut segments = self.segments().to_vec();
                let name = segments.pop()?;
                Some((self.with_segments(segments), name))
            }
        }
    }

    /// The last segment, or a local path's file name. An archive root names
    /// the archive file itself, so the root is not nameless.
    pub fn file_name(&self) -> Option<&str> {
        match &self.location {
            Location::Local { path } => Path::new(path)
                .file_name()
                .and_then(std::ffi::OsStr::to_str),
            Location::Server { segments, .. } | Location::Cloud { segments, .. } => {
                segments.last().map(String::as_str)
            }
            Location::Archive { target, segments } => {
                segments.last().map(String::as_str).or_else(|| target.file_name())
            }
        }
    }
}

impl fmt::Display for EntryUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.location {
            Location::Local { path } => formatter.write_str(path),
            Location::Server {
                scheme,
                server,
                segments,
            } => {
                write!(
                    formatter,
                    "{}://{}",
                    scheme.as_str(),
                    server.authority_text()
                )?;
                if !segments.is_empty() {
                    write!(formatter, "/{}", segments.join("/"))?;
                }
                Ok(())
            }
            Location::Cloud {
                provider,
                email,
                segments,
            } => {
                write!(formatter, "cloud://{}:{email}", provider.as_str())?;
                if !segments.is_empty() {
                    write!(formatter, "/{}", segments.join("/"))?;
                }
                Ok(())
            }
            Location::Archive { target, segments } => {
                write!(formatter, "archive:{target}!")?;
                if !segments.is_empty() {
                    write!(formatter, "/{}", segments.join("/"))?;
                }
                Ok(())
            }
        }
    }
}

/// Splits `host[:port]/rest` into the authority and the rest, bracket-aware:
/// in `[::1]:2121/pub` the port belongs to the authority, so the split is at
/// the first `/` after the closing bracket, never at the bracket itself.
fn split_authority(rest: &str) -> (&str, Option<&str>) {
    if let Some(stripped) = rest.strip_prefix('[')
        && let Some(end) = stripped.find(']')
    {
        let after = &rest[end + 2..];
        return match after.find('/') {
            Some(slash) => (&rest[..end + 2 + slash], Some(&rest[end + 3 + slash..])),
            None => (rest, None),
        };
    }

    match rest.split_once('/') {
        Some((authority, remainder)) => (authority, Some(remainder)),
        None => (rest, None),
    }
}

/// Parses the `[host]:port` authority, accepting bracketed IPv6 addresses.
/// An empty port (`host:/path`) means "the protocol default".
fn parse_authority(authority: &str) -> Result<(String, Option<u16>), FileSystemError> {
    let (host, port) = if authority.starts_with('[') {
        let end = authority
            .find(']')
            .ok_or_else(|| FileSystemError::InvalidInput("Unbalanced IPv6 address".into()))?;
        let host = &authority[..=end];
        let port = authority[end + 1..].strip_prefix(':');
        (host, port)
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        }
    };

    if host.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "A server host is required".into(),
        ));
    }

    let port = match port {
        None | Some("") => None,
        Some(port) => Some(
            port.parse::<u16>()
                .map_err(|_| FileSystemError::InvalidInput(format!("Invalid port: {port}")))?,
        ),
    };

    Ok((host.to_owned(), port))
}

/// Normalizes the part after the authority into canonical segments:
/// backslashes fold, separators collapse, and `.`/`..` drop — the engines
/// build child and parent paths by slicing segments, so a segment that
/// pretends to be a directory is a path the wrong round trip.
pub(crate) fn normalize_segments(rest: &str) -> Vec<String> {
    rest.replace('\\', "/")
        .trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != "." && *segment != "..")
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> EntryUrl {
        EntryUrl::parse(text).expect("parses")
    }

    #[test]
    fn local_paths_round_trip_verbatim() {
        for text in [
            "C:/Users/ada/notes.txt",
            r"\\wsl$\Ubuntu\home\ada",
            r"\\NAS\share\docs",
            "/home/ada",
            "C:\\Program Files\\dae",
        ] {
            assert_eq!(parse(text).to_string(), text, "verbatim: {text}");
            assert!(parse(text).is_local());
        }
    }

    #[test]
    fn a_separator_before_the_scheme_colon_keeps_the_path_local() {
        // A regular path that merely contains the bytes "://": not a scheme.
        let text = r"C:\notes on smb://, the file.txt";
        assert_eq!(parse(text).to_string(), text);
        assert!(parse(text).is_local());
        assert!(EntryUrl::parse("://nas/share").is_ok_and(|url| url.is_local()));
    }

    #[test]
    fn file_uris_stay_local_and_round_trip() {
        for text in ["file:///C:/Users/ada", "file://"] {
            assert_eq!(parse(text).to_string(), text);
            assert!(parse(text).is_local());
        }
    }

    #[test]
    fn scheme_spellings_parse_case_insensitively_and_round_trip_lowercase() {
        assert_eq!(parse("SMB://nas/share").scheme(), Scheme::Smb);
        assert_eq!(parse("Sftp://nas/home").scheme(), Scheme::Sftp);
        assert_eq!(parse("WEBDAVS://nas/dav").scheme(), Scheme::WebDavS);
        assert_eq!(parse("webdavs://nas/dav").to_string(), "webdavs://nas/dav");
        assert_eq!(parse("FTPS://nas/pub").to_string(), "ftps://nas/pub");
        assert!(Scheme::Ftps.is_ftp() && Scheme::Ftp.is_ftp());
        assert!(Scheme::WebDavS.is_webdav() && Scheme::WebDav.is_webdav());
    }

    #[test]
    fn an_unknown_scheme_reports_the_spelling() {
        // `FileSystemError`'s Display adds its own lead-in; the parse error
        // carries the spelling after it.
        assert!(
            EntryUrl::parse("s3://bucket/key")
                .unwrap_err()
                .to_string()
                .ends_with("Unsupported storage protocol: s3")
        );
    }

    #[test]
    fn server_paths_parse_the_authority_and_normalize_segments() {
        let url = parse("smb://nas:445/share\\docs/./x/../y//z/");
        assert_eq!(url.scheme(), Scheme::Smb);
        let server = url.server().expect("server");
        assert_eq!(server.host, "nas");
        assert_eq!(server.port, Some(445));
        assert_eq!(server.username, None);
        // `.` and `..` drop as segments (the engines slice by index); they do
        // not resolve against their neighbours.
        assert_eq!(url.segments(), ["share", "docs", "x", "y", "z"]);
        assert_eq!(url.to_string(), "smb://nas:445/share/docs/x/y/z");

        // No path: the server root.
        assert_eq!(parse("smb://nas").to_string(), "smb://nas");
    }

    #[test]
    fn a_username_is_an_input_hint_and_never_prints() {
        let url = parse("sftp://ada@nas:2222/home/ada");
        let server = url.server().expect("server");
        assert_eq!(server.username.as_deref(), Some("ada"));
        assert_eq!(server.host, "nas");
        assert_eq!(server.port, Some(2222));
        assert_eq!(url.to_string(), "sftp://nas:2222/home/ada");
        assert_eq!(server.authority_text(), "nas:2222");

        // `@host` with an empty user is a host, not a blank credential.
        assert_eq!(
            parse("ftp://@nas/pub").server().expect("server").username,
            None
        );
    }

    #[test]
    fn ipv6_authorities_split_at_the_slash_not_the_bracket() {
        let url = parse("ftp://[2001:db8::1]:2121/pub/x");
        let server = url.server().expect("server");
        assert_eq!(server.host, "[2001:db8::1]");
        assert_eq!(server.port, Some(2121));
        assert_eq!(url.segments(), ["pub", "x"]);
        assert_eq!(url.to_string(), "ftp://[2001:db8::1]:2121/pub/x");

        // No port, no path: the whole bracketed address is the authority.
        let url = parse("webdav://[::1]");
        assert_eq!(url.server().expect("server").host, "[::1]");
        assert_eq!(url.server().expect("server").port, None);
        assert_eq!(url.to_string(), "webdav://[::1]");
    }

    #[test]
    fn an_empty_port_means_the_default_and_a_bad_port_is_rejected() {
        assert_eq!(parse("ftp://nas:/pub").server().expect("server").port, None);
        assert!(
            EntryUrl::parse("ftp://nas:70000/pub")
                .unwrap_err()
                .to_string()
                .ends_with("Invalid port: 70000")
        );
        assert!(
            EntryUrl::parse("ftp://nas:abc/pub")
                .unwrap_err()
                .to_string()
                .ends_with("Invalid port: abc")
        );
        // Unbalanced bracket, missing host: no server to talk to.
        assert!(EntryUrl::parse("ftp://[::1/pub").is_err());
        assert!(
            EntryUrl::parse("smb:///share")
                .unwrap_err()
                .to_string()
                .ends_with("A server host is required")
        );
    }

    #[test]
    fn cloud_authorities_parse_and_ids_stay_opaque() {
        let url = parse("cloud://google_drive:user@example.com");
        let (provider, email) = url.cloud_account().expect("cloud");
        assert_eq!(provider.as_str(), "google_drive");
        assert_eq!(email, "user@example.com");
        assert_eq!(url.segments(), [] as [String; 0]);
        assert_eq!(url.to_string(), "cloud://google_drive:user@example.com");

        // Entry ids are opaque: `.` and `..` are valid id text and survive.
        let url = parse("cloud://dropbox:ada@example.org/folder-id/..");
        assert_eq!(url.segments(), ["folder-id", ".."]);
        assert_eq!(
            url.to_string(),
            "cloud://dropbox:ada@example.org/folder-id/.."
        );
    }

    #[test]
    fn malformed_cloud_authorities_report_the_text() {
        assert!(
            EntryUrl::parse("cloud://google_drive")
                .unwrap_err()
                .to_string()
                .contains("Malformed cloud account path")
        );
        assert!(
            EntryUrl::parse("cloud://google_drive:/x")
                .unwrap_err()
                .to_string()
                .contains("The cloud account has no email: google_drive:")
        );
    }

    #[test]
    fn joining_and_popping_are_inverse() {
        let url = parse("smb://nas:445/share/docs");
        let child = url.join("readme.md");
        assert_eq!(child.to_string(), "smb://nas:445/share/docs/readme.md");
        let (parent, name) = child.pop().expect("splits");
        assert_eq!(name, "readme.md");
        assert_eq!(parent.to_string(), "smb://nas:445/share/docs");

        // At the server root there is nothing left to split.
        assert!(parse("smb://nas").pop().is_none());
        assert_eq!(parse("smb://nas").file_name(), None);

        // Local paths join and pop through the OS path rules, so the joined
        // text is whatever this platform's separator is; the name and the
        // parent are separator-stable.
        let local = parse("/home/ada");
        let child = local.join("notes.txt");
        let (parent, name) = child.pop().expect("splits");
        assert_eq!(name, "notes.txt");
        assert_eq!(parent.local_path(), Some("/home/ada"));
    }

    #[test]
    fn segments_of_a_local_path_read_empty_rather_than_guessing() {
        assert!(parse(r"C:\Users\ada").segments().is_empty());
    }
}
