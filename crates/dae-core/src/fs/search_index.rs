//! The platform's own search index, used as an *additional* candidate source
//! for the name search.
//!
//! This module never replaces the recursive walk: the walk keeps cancellation,
//! tag filtering and remote-root rules, and the index only contributes paths
//! that already exist and whose file name matches the query. That split is
//! deliberate — index semantics differ per platform (Windows Search matches
//! content, tracker/baloo match full text), while dae's search box means "the
//! name contains this". The post-filter in `fs::search` enforces that meaning,
//! so a content hit whose name does not match is discarded rather than shown
//! as a result the walk would never have produced.
//!
//! Availability is probed once per process and cached:
//!
//! * **Windows** — `ISearchQueryHelper` + the `Search.CollatorDSO` OLE DB
//!   provider. Both are reachable from an unpackaged Win32 process; an MSIX
//!   build may see a different catalog view, which the probe reports as
//!   unavailable rather than as an empty result.
//! * **Linux** — `tracker3` or `baloosearch` on `PATH`. No daemon of our own:
//!   the desktop index already owns the crawler.
//! * **macOS** — not implemented (no artifact ships).
//!
//! Failure is never an error the user sees: the walk's results are the answer,
//! and the index only ever adds to them.

use std::sync::LazyLock;

use crate::error::FileSystemError;

/// Which index answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    WindowsSearch,
    Tracker3,
    Baloo,
}

impl Backend {
    /// Short name for logs and the "results came from" note.
    pub fn name(self) -> &'static str {
        match self {
            Backend::WindowsSearch => "windows-search",
            Backend::Tracker3 => "tracker3",
            Backend::Baloo => "baloo",
        }
    }
}

/// The index this machine can use, probed once.
pub fn backend() -> Option<Backend> {
    static BACKEND: LazyLock<Option<Backend>> = LazyLock::new(probe);
    *BACKEND
}

fn probe() -> Option<Backend> {
    #[cfg(windows)]
    {
        return windows_search::probe().then_some(Backend::WindowsSearch);
    }
    #[cfg(target_os = "linux")]
    {
        if command_available("tracker3") {
            return Some(Backend::Tracker3);
        }
        if command_available("baloosearch") {
            return Some(Backend::Baloo);
        }
        return None;
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        None
    }
}

/// Paths the index associates with `text`. Blocking; callers already run the
/// search on a blocking thread. An unavailable index or a failed query is an
/// empty contribution, not an error the search has to surface.
pub fn query(text: &str, limit: usize) -> Vec<String> {
    match try_query(text, limit) {
        Ok(paths) => paths,
        Err(error) => {
            log::debug!(target: "dae::search", "index query failed: {error}");
            Vec::new()
        }
    }
}

/// The same query with its failure kept, for diagnostics and tests. Production
/// callers use [`query`], which logs instead of failing the search.
pub fn try_query(text: &str, limit: usize) -> Result<Vec<String>, FileSystemError> {
    let Some(backend) = backend() else {
        return Ok(Vec::new());
    };
    let answer = match backend {
        #[cfg(windows)]
        Backend::WindowsSearch => windows_search::query(text, limit),
        #[cfg(target_os = "linux")]
        Backend::Tracker3 => linux_search::run(
            "tracker3",
            &["search", "--limit", &limit.to_string(), text],
            limit,
        ),
        #[cfg(target_os = "linux")]
        Backend::Baloo => {
            linux_search::run("baloosearch", &["-l", &limit.to_string(), text], limit)
        }
        #[allow(unreachable_patterns)]
        _ => Ok(Vec::new()),
    };
    answer
}

/// Whether `program` resolves through `PATH`, with the Windows executable
/// extensions tried as well. Spawning it is the caller's decision; this only
/// answers "is it installed".
#[cfg(any(target_os = "linux", test))]
fn command_available(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    let extensions: &[&str] = if cfg!(windows) {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    std::env::split_paths(&path).any(|directory| {
        extensions.iter().any(|extension| {
            let candidate = directory.join(format!("{program}{extension}"));
            candidate.is_file()
        })
    })
}

/// Turns one line of index output into an absolute path.
///
/// tracker prints `file://` URIs, baloo prints paths; both are accepted, and a
/// line that is neither (banners, counts) is ignored. Percent escapes are
/// decoded because tracker escapes spaces and CJK.
#[cfg(any(target_os = "linux", test))]
use std::path::Path;

#[cfg(any(target_os = "linux", test))]
fn line_to_path(line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let decoded = match line.strip_prefix("file://") {
        Some(rest) => percent_decode(rest),
        None => line.to_owned(),
    };
    let path = if cfg!(windows) {
        // A `file:///C:/…` URI keeps a leading slash after the authority strip.
        decoded
            .strip_prefix('/')
            .filter(|rest| rest.as_bytes().get(1) == Some(&b':'))
            .unwrap_or(&decoded)
            .to_owned()
    } else {
        decoded
    };
    Path::new(&path).is_absolute().then_some(path)
}

/// `%XX` escapes, byte-wise, then UTF-8. Invalid escapes are kept verbatim:
/// the worst case is a path that does not exist, which the caller stats away.
#[cfg(any(windows, target_os = "linux", test))]
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
        {
            out.push(high * 16 + low);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(any(windows, target_os = "linux", test))]
fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// `file:` URLs from the Search provider to a Win32 path.
///
/// The provider answers `System.ItemUrl` as `file:C:/Users/...` (and sometimes
/// with a `//` authority); `System.ItemPathDisplay` is *not* used because it
/// localises well-known directories ("Users" becomes the display language's
/// word), which no longer names anything on disk.
#[cfg(any(windows, test))]
fn file_url_to_path(url: &str) -> Option<String> {
    let rest = url.strip_prefix("file:")?;
    let rest = rest
        .strip_prefix("///")
        .or_else(|| rest.strip_prefix("//"))
        .unwrap_or(rest);
    let path = percent_decode(rest);
    (!path.is_empty()).then_some(path)
}

#[cfg(windows)]
mod windows_search {
    use super::*;
    use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, CLSCTX_LOCAL_SERVER, COINIT_MULTITHREADED, CoCreateInstance,
        CoInitializeEx, CoTaskMemFree, CoUninitialize,
    };
    use windows::Win32::System::Search::{
        CSearchManager, DBACCESSOR_ROWDATA, DBBINDING, DBMEMOWNER_CLIENTOWNED, DBPARAMIO_NOTPARAM,
        DBPART_STATUS, DBPART_VALUE, DBSTATUS_S_OK, DBTYPE_VARIANT, HACCESSOR, IAccessor, ICommand,
        ICommandText, IDBCreateCommand, IDBCreateSession, IDBInitialize, IDataInitialize, IRowset,
        ISearchCatalogManager, ISearchManager, ISearchQueryHelper, MSDAINITIALIZE,
    };
    use windows::Win32::System::Variant::{VARIANT, VT_BSTR, VariantClear};
    use windows::core::{GUID, HSTRING, Interface, PCWSTR, PWSTR};

    /// `DBGUID_DEFAULT` is not exported by the `windows` crate; the value is
    /// the one `oledb.h` defines.
    const DBGUID_DEFAULT: GUID = GUID::from_u128(0xc8b521fb_5cf3_11ce_ade5_00aa0044773d);

    /// The connection string MSDN documents for the Search provider. The
    /// helper can produce one too, but the documented literal is the same
    /// string and does not need the catalog round trip.
    const CONNECTION: &str =
        "provider=Search.CollatorDSO;EXTENDED PROPERTIES=\"Application=Windows\"";

    /// Initialises COM for this thread and balances it on drop.
    struct Com(Option<()>);

    impl Com {
        fn ensure() -> Self {
            let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            // Already initialised with another model is fine: the caller owns
            // that lifetime and this guard must not uninitialise it.
            if hr == RPC_E_CHANGED_MODE {
                return Com(None);
            }
            Com(hr.is_ok().then_some(()))
        }
    }

    impl Drop for Com {
        fn drop(&mut self) {
            if self.0.is_some() {
                unsafe { CoUninitialize() };
            }
        }
    }

    pub(super) fn probe() -> bool {
        let _com = Com::ensure();
        query_helper().is_ok()
    }

    fn query_helper() -> windows::core::Result<ISearchQueryHelper> {
        unsafe {
            let manager: ISearchManager = CoCreateInstance(
                &CSearchManager,
                None::<&windows::core::IUnknown>,
                CLSCTX_LOCAL_SERVER,
            )?;
            let catalog: ISearchCatalogManager =
                manager.GetCatalog(&HSTRING::from("SystemIndex"))?;
            catalog.GetQueryHelper()
        }
    }

    pub(super) fn query(text: &str, limit: usize) -> Result<Vec<String>, FileSystemError> {
        let _com = Com::ensure();
        let helper = query_helper().map_err(step_error("query helper"))?;
        unsafe {
            helper
                .SetQueryMaxResults(limit as i32)
                .map_err(step_error("max results"))?;
            // The helper appends the select columns to its generated SQL; the
            // display path is the one field the merge needs.
            helper
                .SetQuerySelectColumns(&HSTRING::from("System.ItemUrl"))
                .map_err(step_error("select columns"))?;
            let sql = helper
                .GenerateSQLFromUserQuery(&HSTRING::from(text))
                .map_err(step_error("generate sql"))?;
            let sql = take_pwstr(sql);
            execute_sql(&sql, limit)
        }
    }

    /// Runs the generated SELECT through the provider and reads column 1 as a
    /// string until the rowset ends.
    unsafe fn execute_sql(sql: &str, limit: usize) -> Result<Vec<String>, FileSystemError> {
        unsafe {
            let data_init: IDataInitialize = CoCreateInstance(
                &MSDAINITIALIZE,
                None::<&windows::core::IUnknown>,
                CLSCTX_INPROC_SERVER,
            )
            .map_err(step_error("create data initialize"))?;

            let connection = HSTRING::from(CONNECTION);
            let mut source: Option<windows::core::IUnknown> = None;
            data_init
                .GetDataSource(
                    None::<&windows::core::IUnknown>,
                    CLSCTX_INPROC_SERVER.0,
                    PCWSTR(connection.as_ptr()),
                    &IDBInitialize::IID,
                    &mut source,
                )
                .map_err(step_error("get data source"))?;
            let source = source.ok_or_else(|| {
                FileSystemError::Internal("search: the provider returned no session".into())
            })?;

            let initialize: IDBInitialize =
                source.cast().map_err(step_error("cast data source"))?;
            initialize
                .Initialize()
                .map_err(step_error("initialize data source"))?;
            let sessions: IDBCreateSession = initialize
                .cast()
                .map_err(step_error("cast session factory"))?;
            let session = sessions
                .CreateSession(None::<&windows::core::IUnknown>, &IDBCreateCommand::IID)
                .map_err(step_error("create session"))?;
            let commands: IDBCreateCommand = session.cast().map_err(step_error("cast session"))?;
            let command = commands
                .CreateCommand(None::<&windows::core::IUnknown>, &ICommandText::IID)
                .map_err(step_error("create command"))?;
            let command: ICommandText = command.cast().map_err(step_error("cast command"))?;

            let statement = HSTRING::from(sql);
            command
                .SetCommandText(&DBGUID_DEFAULT, &statement)
                .map_err(step_error("set command text"))?;

            let mut rowset: Option<windows::core::IUnknown> = None;
            (&command as &ICommand)
                .Execute(
                    None::<&windows::core::IUnknown>,
                    &IRowset::IID,
                    None,
                    None,
                    Some(&mut rowset),
                )
                .map_err(step_error("execute"))?;
            let rowset: IRowset = rowset
                .ok_or_else(|| {
                    FileSystemError::Internal("search: the query returned no rowset".into())
                })?
                .cast()
                .map_err(step_error("cast rowset"))?;
            let accessor: IAccessor = rowset.cast().map_err(step_error("cast accessor"))?;

            // The provider hands this column back as a VARIANT (column type
            // 0xc), not as a string, so the binding asks for exactly that and
            // the reader unwraps BSTR/LPWSTR. Binding it as WSTR is rejected
            // with `DBBINDSTATUS_BADBINDINFO`.
            #[repr(C)]
            struct Row {
                status: u32,
                padding: u32,
                value: VARIANT,
            }

            let mut binding = DBBINDING::default();
            binding.iOrdinal = 1;
            binding.obStatus = 0;
            binding.obValue = 8;
            binding.dwPart = (DBPART_VALUE.0 | DBPART_STATUS.0) as u32;
            binding.dwMemOwner = DBMEMOWNER_CLIENTOWNED.0 as u32;
            binding.eParamIO = DBPARAMIO_NOTPARAM.0 as u32;
            binding.cbMaxLen = std::mem::size_of::<VARIANT>();
            binding.wType = DBTYPE_VARIANT.0 as u16;

            let row_bytes = std::mem::size_of::<Row>();
            let mut accessor_handle = HACCESSOR(0);
            let mut bind_status = [0u32; 1];
            if let Err(error) = accessor.CreateAccessor(
                DBACCESSOR_ROWDATA.0 as u32,
                1,
                &binding,
                row_bytes,
                &mut accessor_handle,
                Some(bind_status.as_mut_ptr()),
            ) {
                return Err(FileSystemError::Internal(format!(
                    "search index create accessor: {error} (binding status {})",
                    bind_status[0]
                )));
            }

            let mut paths = Vec::new();
            let mut row = Row {
                status: 0,
                padding: 0,
                value: VARIANT::default(),
            };
            while paths.len() < limit {
                let mut obtained = 0usize;
                let mut rows: [*mut usize; 1] = [std::ptr::null_mut()];
                rowset
                    .GetNextRows(0, 1, &mut obtained, &mut rows)
                    .map_err(step_error("next rows"))?;
                if obtained == 0 {
                    break;
                }
                // The provider allocates the handle array and hands back its
                // address; the HROW is the first element. The consumer frees
                // the array (CoTaskMemFree) after releasing the rows.
                let array = rows[0];
                if array.is_null() {
                    break;
                }
                let handle = *array;
                let data = rowset.GetData(handle, accessor_handle, (&mut row as *mut Row).cast());
                let _ = rowset.ReleaseRows(
                    1,
                    &handle,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                );
                CoTaskMemFree(Some(array.cast()));
                if let Err(error) = data {
                    log::debug!(target: "dae::search", "row read failed: {error}");
                    continue;
                }
                if row.status != DBSTATUS_S_OK.0 as u32 {
                    continue;
                }
                let variant = &row.value;
                // The provider publishes this column as a BSTR URL; anything
                // else is not a path and contributes nothing. `ItemPathDisplay`
                // is deliberately not used: it localises "Users" and friends,
                // which no longer names a file on disk.
                let url = match variant.Anonymous.Anonymous.vt {
                    VT_BSTR => (*variant.Anonymous.Anonymous.Anonymous.bstrVal).to_string(),
                    _ => String::new(),
                };
                let text = file_url_to_path(&url).unwrap_or_default();
                let _ = VariantClear(&mut row.value as *mut VARIANT);
                if !text.is_empty() {
                    paths.push(text);
                }
            }
            Ok(paths)
        }
    }

    /// Copies a `PWSTR` the provider allocated with `CoTaskMemAlloc` and frees
    /// it, so the caller never holds a COM-lifetime pointer.
    unsafe fn take_pwstr(value: PWSTR) -> String {
        unsafe {
            let text = value.to_string().unwrap_or_default();
            CoTaskMemFree(Some(value.0 as *const _));
            text
        }
    }

    fn step_error(step: &'static str) -> impl Fn(windows::core::Error) -> FileSystemError {
        move |error| FileSystemError::Internal(format!("search index {step}: {error}"))
    }
}

#[cfg(target_os = "linux")]
mod linux_search {
    use super::*;
    use std::process::Command;

    /// Runs the desktop index's CLI and keeps its path-like lines. A missing
    /// binary or a non-zero exit yields an empty contribution.
    pub(super) fn run(
        program: &str,
        args: &[&str],
        limit: usize,
    ) -> Result<Vec<String>, FileSystemError> {
        let output = Command::new(program).args(args).output()?;
        if !output.status.success() {
            return Err(FileSystemError::Io(format!(
                "{program} exited with {}",
                output.status
            )));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut paths: Vec<String> = stdout
            .lines()
            .filter_map(line_to_path)
            .take(limit)
            .collect();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn a_file_uri_becomes_a_path() {
        let path = line_to_path("file:///home/me/a%20note.txt").expect("a URI is a hit");
        assert!(path.ends_with("home/me/a note.txt"), "{path}");
    }

    #[test]
    fn a_plain_path_is_accepted() {
        let path = if cfg!(windows) {
            "C:\\Users\\me\\a note.txt"
        } else {
            "/home/me/a note.txt"
        };
        assert_eq!(line_to_path(path).as_deref(), Some(path));
    }

    #[test]
    fn a_windows_uri_loses_its_uri_slash() {
        if !cfg!(windows) {
            return;
        }
        let path = line_to_path("file:///C:/Users/me/a.txt").expect("a URI is a hit");
        assert_eq!(path, "C:/Users/me/a.txt");
    }

    #[test]
    fn search_urls_become_windows_paths() {
        assert_eq!(
            file_url_to_path("file:C:/Users/me/a.txt").as_deref(),
            Some("C:/Users/me/a.txt")
        );
        assert_eq!(
            file_url_to_path("file:///C:/Users/me/a%20b.txt").as_deref(),
            Some("C:/Users/me/a b.txt")
        );
        assert_eq!(file_url_to_path("https://example.com"), None);
    }

    #[test]
    fn percent_escapes_decode_cjk() {
        let decoded = percent_decode("%E9%87%8D%E8%A6%81.txt");
        assert_eq!(decoded, "重要.txt");
    }

    #[test]
    fn banners_and_relative_lines_are_ignored() {
        assert_eq!(line_to_path("Results 3:"), None);
        assert_eq!(line_to_path("relative/path.txt"), None);
        assert_eq!(line_to_path(""), None);
    }
}
