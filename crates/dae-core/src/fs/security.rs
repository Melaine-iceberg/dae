//! File security facts the properties dialog shows read-only: an Authenticode
//! signature and, on Windows, the discretionary ACL.
//!
//! Neither is an editor. Verifying a signature is a read; changing one would
//! need a private key. An ACL editor is a separate design (inheritance,
//! propagation, deny ordering), and the existing POSIX/DOS editors stay the
//! only writable permission surfaces. What this module adds is the answer to
//! "is this file signed, by whom, and who can touch it" for the platform that
//! has a real answer.
//!
//! Windows is the only implementation. macOS's `SecStaticCodeCheckValidity`
//! and NFSv4 ACLs are not built here — no macOS artifact ships — and Linux has
//! no file-signature concept; both report [`FileSystemError::Unsupported`] and
//! the dialog hides the section by asking [`signature_supported`] /
//! [`acl_supported`] first.

use serde::Serialize;

use crate::error::FileSystemError;

/// One signature verdict. `status` is a stable token the QML layer maps to a
/// sentence: `valid`, `unsigned`, `untrusted` or `invalid`.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSignature {
    pub status: String,
    /// The signer's simple display name, when the chain exposed one.
    pub signer: Option<String>,
    pub issuer: Option<String>,
    /// The platform's own reason for a non-valid verdict.
    pub detail: Option<String>,
}

/// The owner, group and DACL entries of one file. Read-only by design.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileAcl {
    pub owner: Option<String>,
    pub group: Option<String>,
    /// One line per ACE, e.g. `Allow BUILTIN\Users: Read & execute`.
    pub entries: Vec<String>,
}

/// Whether this build can verify an Authenticode signature.
pub fn signature_supported() -> bool {
    cfg!(windows)
}

/// Whether this build can read a discretionary ACL.
pub fn acl_supported() -> bool {
    cfg!(windows)
}

/// Verifies `path`'s Authenticode signature. A missing signature is a verdict
/// (`unsigned`), not an error; an unreadable file is an error.
pub fn signature(path: &str) -> Result<FileSignature, FileSystemError> {
    signature_native(path)
}

/// Reads `path`'s owner, group and DACL. An entry with no DACL reports an
/// empty list rather than failing.
pub fn acl(path: &str) -> Result<FileAcl, FileSystemError> {
    acl_native(path)
}

#[cfg(not(windows))]
fn signature_native(_path: &str) -> Result<FileSignature, FileSystemError> {
    Err(FileSystemError::Unsupported(
        "File signatures are only available on Windows".into(),
    ))
}

#[cfg(not(windows))]
fn acl_native(_path: &str) -> Result<FileAcl, FileSystemError> {
    Err(FileSystemError::Unsupported(
        "ACL reading is only available on Windows".into(),
    ))
}

#[cfg(windows)]
fn signature_native(path: &str) -> Result<FileSignature, FileSystemError> {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();

    let mut signature = verify_embedded(&wide)?;
    // System binaries are usually catalog-signed: the Authenticode blob lives
    // in a .cat file, not in the executable, so the embedded check reports
    // "unsigned" for cmd.exe and most of System32. The catalog lookup is the
    // second half of the same question.
    if signature.status == "unsigned"
        && let Some(catalog) = verify_catalog(&wide)
    {
        signature = catalog;
    }
    Ok(signature)
}

/// Verifies the signature stored inside the file (or its security directory).
#[cfg(windows)]
fn verify_embedded(wide: &[u16]) -> Result<FileSignature, FileSystemError> {
    use windows::core::{GUID, PCWSTR};
    use windows::Win32::Foundation::{HANDLE, HWND};
    use windows::Win32::Security::WinTrust::{
        WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO,
        WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_SAFER_FLAG, WTD_STATEACTION_CLOSE,
        WTD_STATEACTION_VERIFY, WTD_UI_NONE, WinVerifyTrust,
    };

    let mut file_info = WINTRUST_FILE_INFO {
        cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: PCWSTR(wide.as_ptr()),
        hFile: HANDLE::default(),
        pgKnownSubject: std::ptr::null_mut(),
    };
    let mut data = WINTRUST_DATA {
        cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 {
            pFile: &mut file_info,
        },
        dwStateAction: WTD_STATEACTION_VERIFY,
        dwProvFlags: WTD_SAFER_FLAG,
        ..Default::default()
    };
    let mut action: GUID = WINTRUST_ACTION_GENERIC_VERIFY_V2;

    let status = unsafe {
        WinVerifyTrust(
            HWND(std::ptr::null_mut()),
            &mut action,
            &mut data as *mut _ as *mut _,
        )
    };

    let (signer, issuer) = if data.hWVTStateData.is_invalid() {
        (None, None)
    } else {
        signer_from_state(data.hWVTStateData)
    };

    data.dwStateAction = WTD_STATEACTION_CLOSE;
    unsafe {
        WinVerifyTrust(
            HWND(std::ptr::null_mut()),
            &mut action,
            &mut data as *mut _ as *mut _,
        );
    }

    Ok(FileSignature {
        status: signature_status(status).to_owned(),
        signer,
        issuer,
        detail: (status != 0).then(|| signature_detail(status)).flatten(),
    })
}

/// Looks the file's hash up in the system catalogs and verifies that member.
/// `None` means "no catalog claims this file", which leaves the embedded
/// verdict (usually `unsigned`) in place.
#[cfg(windows)]
fn verify_catalog(wide: &[u16]) -> Option<FileSignature> {
    use windows::core::{GUID, PCWSTR};
    use windows::Win32::Foundation::{CloseHandle, HWND};
    use windows::Win32::Security::Cryptography::Catalog::{
        CATALOG_INFO, CryptCATAdminAcquireContext2, CryptCATAdminCalcHashFromFileHandle2,
        CryptCATAdminEnumCatalogFromHash, CryptCATAdminReleaseCatalogContext,
        CryptCATAdminReleaseContext, CryptCATCatalogInfoFromContext,
    };
    use windows::Win32::Security::WinTrust::{
        WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_CATALOG_INFO, WINTRUST_DATA,
        WINTRUST_DATA_0, WTD_CHOICE_CATALOG, WTD_REVOKE_NONE, WTD_SAFER_FLAG,
        WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE, WinVerifyTrust,
    };
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            0x8000_0000, // GENERIC_READ
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    }
    .ok()?;

    let result = (|| -> Option<FileSignature> {
        let algorithm: Vec<u16> = "SHA256\0".encode_utf16().collect();
        let mut admin = 0isize;
        unsafe {
            CryptCATAdminAcquireContext2(
                &mut admin,
                None,
                PCWSTR(algorithm.as_ptr()),
                None,
                None,
            )
        }
        .ok()?;

        let mut hash_length = 0u32;
        // The first call reports the length through an error, so its Result is
        // expected to fail; only the length matters.
        let _ = unsafe {
            CryptCATAdminCalcHashFromFileHandle2(admin, handle, &mut hash_length, None, None)
        };
        if hash_length == 0 {
            unsafe {
                let _ = CryptCATAdminReleaseContext(admin, 0);
            }
            return None;
        }

        let mut hash = vec![0u8; hash_length as usize];
        let hash_ok = unsafe {
            CryptCATAdminCalcHashFromFileHandle2(
                admin,
                handle,
                &mut hash_length,
                Some(hash.as_mut_ptr()),
                None,
            )
        }
        .is_ok();
        if !hash_ok {
            unsafe {
                let _ = CryptCATAdminReleaseContext(admin, 0);
            }
            return None;
        }
        hash.truncate(hash_length as usize);

        let catalog = unsafe { CryptCATAdminEnumCatalogFromHash(admin, &hash, None, None) };
        if catalog == 0 {
            unsafe {
                let _ = CryptCATAdminReleaseContext(admin, 0);
            }
            return None;
        }

        let mut info = CATALOG_INFO {
            cbStruct: std::mem::size_of::<CATALOG_INFO>() as u32,
            ..Default::default()
        };
        let info_ok = unsafe { CryptCATCatalogInfoFromContext(catalog, &mut info, 0) }.is_ok();
        if !info_ok {
            unsafe {
                let _ = CryptCATAdminReleaseCatalogContext(admin, catalog, 0);
                let _ = CryptCATAdminReleaseContext(admin, 0);
            }
            return None;
        }

        // The member tag is the file hash as uppercase hex.
        let tag: Vec<u16> = hash
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<String>()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let mut catalog_info = WINTRUST_CATALOG_INFO {
            cbStruct: std::mem::size_of::<WINTRUST_CATALOG_INFO>() as u32,
            pcwszCatalogFilePath: PCWSTR(info.wszCatalogFile.as_ptr()),
            pcwszMemberTag: PCWSTR(tag.as_ptr()),
            pcwszMemberFilePath: PCWSTR(wide.as_ptr()),
            hMemberFile: handle,
            pbCalculatedFileHash: hash.as_mut_ptr(),
            cbCalculatedFileHash: hash_length,
            hCatAdmin: admin,
            ..Default::default()
        };
        let mut data = WINTRUST_DATA {
            cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
            dwUIChoice: WTD_UI_NONE,
            fdwRevocationChecks: WTD_REVOKE_NONE,
            dwUnionChoice: WTD_CHOICE_CATALOG,
            Anonymous: WINTRUST_DATA_0 {
                pCatalog: &mut catalog_info,
            },
            dwStateAction: WTD_STATEACTION_VERIFY,
            dwProvFlags: WTD_SAFER_FLAG,
            ..Default::default()
        };
        let mut action: GUID = WINTRUST_ACTION_GENERIC_VERIFY_V2;
        let status = unsafe {
            WinVerifyTrust(
                HWND(std::ptr::null_mut()),
                &mut action,
                &mut data as *mut _ as *mut _,
            )
        };
        let (signer, issuer) = if data.hWVTStateData.is_invalid() {
            (None, None)
        } else {
            signer_from_state(data.hWVTStateData)
        };

        data.dwStateAction = WTD_STATEACTION_CLOSE;
        unsafe {
            WinVerifyTrust(
                HWND(std::ptr::null_mut()),
                &mut action,
                &mut data as *mut _ as *mut _,
            );
            let _ = CryptCATAdminReleaseCatalogContext(admin, catalog, 0);
            let _ = CryptCATAdminReleaseContext(admin, 0);
        }

        Some(FileSignature {
            status: signature_status(status).to_owned(),
            signer,
            issuer,
            detail: (status != 0).then(|| signature_detail(status)).flatten(),
        })
    })();

    unsafe {
        let _ = CloseHandle(handle);
    }
    result
}

/// Reads the signer and issuer out of WinVerifyTrust's provider state, which
/// is valid only between the VERIFY and CLOSE state actions.
#[cfg(windows)]
fn signer_from_state(
    state: windows::Win32::Foundation::HANDLE,
) -> (Option<String>, Option<String>) {
    use windows::Win32::Security::WinTrust::{
        WTHelperGetProvSignerFromChain, WTHelperProvDataFromStateData,
    };

    let provider = unsafe { WTHelperProvDataFromStateData(state) };
    if provider.is_null() {
        return (None, None);
    }
    let signer = unsafe { WTHelperGetProvSignerFromChain(provider, 0, false, 0) };
    if signer.is_null() {
        return (None, None);
    }
    let chain = unsafe { (*signer).pasCertChain };
    if chain.is_null() {
        return (None, None);
    }
    let certificate = unsafe { (*chain).pCert };
    if certificate.is_null() {
        return (None, None);
    }
    (certificate_name(certificate), certificate_issuer(certificate))
}

#[cfg(windows)]
fn signature_status(status: i32) -> &'static str {
    const TRUST_E_NOSIGNATURE: i32 = 0x800B_0100u32 as i32;
    const TRUST_E_SUBJECT_NOT_TRUSTED: i32 = 0x800B_0004u32 as i32;
    const TRUST_E_EXPLICIT_DISTRUST: i32 = 0x800B_0111u32 as i32;

    match status {
        0 => "valid",
        TRUST_E_NOSIGNATURE => "unsigned",
        TRUST_E_SUBJECT_NOT_TRUSTED | TRUST_E_EXPLICIT_DISTRUST => "untrusted",
        _ => "invalid",
    }
}

#[cfg(windows)]
fn signature_detail(status: i32) -> Option<String> {
    const TRUST_E_NOSIGNATURE: i32 = 0x800B_0100u32 as i32;
    const TRUST_E_BAD_DIGEST: i32 = 0x8009_6010u32 as i32;
    const CERT_E_EXPIRED: i32 = 0x800B_0101u32 as i32;
    const CERT_E_UNTRUSTEDROOT: i32 = 0x800B_0109u32 as i32;
    const TRUST_E_SUBJECT_NOT_TRUSTED: i32 = 0x800B_0004u32 as i32;
    const TRUST_E_EXPLICIT_DISTRUST: i32 = 0x800B_0111u32 as i32;

    let text = match status {
        TRUST_E_NOSIGNATURE => "The file has no Authenticode signature",
        TRUST_E_BAD_DIGEST => "The file changed after it was signed",
        CERT_E_EXPIRED => "The signing certificate has expired",
        CERT_E_UNTRUSTEDROOT => "The certificate chain ends in an untrusted root",
        TRUST_E_SUBJECT_NOT_TRUSTED => "The signature is not trusted",
        TRUST_E_EXPLICIT_DISTRUST => "The publisher is explicitly distrusted",
        _ => return Some(format!("WinVerifyTrust returned 0x{status:08X}")),
    };
    Some(text.to_owned())
}

#[cfg(windows)]
fn certificate_name(certificate: *const windows::Win32::Security::Cryptography::CERT_CONTEXT) -> Option<String> {
    use windows::Win32::Security::Cryptography::{
        CERT_NAME_SIMPLE_DISPLAY_TYPE, CertGetNameStringW,
    };

    unsafe {
        let length = CertGetNameStringW(certificate, CERT_NAME_SIMPLE_DISPLAY_TYPE, 0, None, None);
        if length <= 1 {
            return None;
        }
        let mut buffer = vec![0u16; length as usize];
        let written = CertGetNameStringW(
            certificate,
            CERT_NAME_SIMPLE_DISPLAY_TYPE,
            0,
            None,
            Some(&mut buffer),
        );
        if written <= 1 {
            return None;
        }
        Some(String::from_utf16_lossy(&buffer[..written as usize - 1]))
    }
}

#[cfg(windows)]
fn certificate_issuer(certificate: *const windows::Win32::Security::Cryptography::CERT_CONTEXT) -> Option<String> {
    use windows::Win32::Security::Cryptography::{
        CERT_NAME_SIMPLE_DISPLAY_TYPE, CertGetNameStringW, CERT_NAME_ISSUER_FLAG,
    };

    unsafe {
        let length = CertGetNameStringW(
            certificate,
            CERT_NAME_SIMPLE_DISPLAY_TYPE,
            CERT_NAME_ISSUER_FLAG,
            None,
            None,
        );
        if length <= 1 {
            return None;
        }
        let mut buffer = vec![0u16; length as usize];
        let written = CertGetNameStringW(
            certificate,
            CERT_NAME_SIMPLE_DISPLAY_TYPE,
            CERT_NAME_ISSUER_FLAG,
            None,
            Some(&mut buffer),
        );
        if written <= 1 {
            return None;
        }
        Some(String::from_utf16_lossy(&buffer[..written as usize - 1]))
    }
}

#[cfg(windows)]
fn acl_native(path: &str) -> Result<FileAcl, FileSystemError> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::{
        GetNamedSecurityInfoW, SE_FILE_OBJECT,
    };
    use windows::Win32::Security::{
        ACL, DACL_SECURITY_INFORMATION, GROUP_SECURITY_INFORMATION,
        OWNER_SECURITY_INFORMATION, PSID, PSECURITY_DESCRIPTOR,
    };

    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut owner = PSID::default();
    let mut group = PSID::default();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();

    let result = unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(wide.as_ptr()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            Some(&mut owner),
            Some(&mut group),
            Some(&mut dacl),
            None,
            &mut descriptor,
        )
    };
    if result.0 != 0 {
        return Err(FileSystemError::PermissionDenied(format!(
            "Could not read the security descriptor (error {})",
            result.0
        )));
    }

    let acl = FileAcl {
        owner: sid_name(owner),
        group: sid_name(group),
        entries: dacl_entries(dacl),
    };

    unsafe {
        if !descriptor.is_invalid() {
            LocalFree(Some(HLOCAL(descriptor.0)));
        }
    }

    Ok(acl)
}

#[cfg(windows)]
fn sid_name(sid: windows::Win32::Security::PSID) -> Option<String> {
    use windows::core::PWSTR;
    use windows::Win32::Security::{LookupAccountSidW, SID_NAME_USE};

    if sid.is_invalid() {
        return None;
    }

    let mut name_length = 0u32;
    let mut domain_length = 0u32;
    let mut use_type = SID_NAME_USE::default();
    unsafe {
        let _ = LookupAccountSidW(
            None,
            sid,
            None,
            &mut name_length,
            None,
            &mut domain_length,
            &mut use_type,
        );
    }
    if name_length == 0 {
        return None;
    }

    let mut name = vec![0u16; name_length as usize];
    let mut domain = vec![0u16; domain_length as usize];
    let ok = unsafe {
        LookupAccountSidW(
            None,
            sid,
            Some(PWSTR(name.as_mut_ptr())),
            &mut name_length,
            Some(PWSTR(domain.as_mut_ptr())),
            &mut domain_length,
            &mut use_type,
        )
    };
    if ok.is_err() {
        return None;
    }

    let name = String::from_utf16_lossy(&name[..name_length as usize]);
    let domain = String::from_utf16_lossy(&domain[..domain_length as usize]);
    if domain.is_empty() {
        Some(name)
    } else {
        Some(format!("{domain}\\{name}"))
    }
}

#[cfg(windows)]
fn dacl_entries(dacl: *mut windows::Win32::Security::ACL) -> Vec<String> {
    use windows::Win32::Security::{ACL_SIZE_INFORMATION, AclSizeInformation, GetAclInformation, GetAce, PSID};

    if dacl.is_null() {
        return Vec::new();
    }

    let mut info = ACL_SIZE_INFORMATION::default();
    let ok = unsafe {
        GetAclInformation(
            dacl,
            &mut info as *mut _ as *mut _,
            std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    };
    if ok.is_err() {
        return Vec::new();
    }

    let mut entries = Vec::new();
    for index in 0..info.AceCount {
        let mut ace = std::ptr::null_mut();
        if unsafe { GetAce(dacl, index, &mut ace) }.is_err() || ace.is_null() {
            continue;
        }
        // ACE_HEADER { AceType: u8, AceFlags: u8, AceSize: u16 } then
        // ACCESS_ALLOWED_ACE { Mask: u32, SidStart: SID }. The binding does
        // not expose the concrete ACE structs, so the offsets are read here;
        // every ACE this lists has that layout.
        let bytes = ace as *const u8;
        let ace_type = unsafe { *bytes };
        let mask = unsafe {
            u32::from_le_bytes([
                *bytes.add(4),
                *bytes.add(5),
                *bytes.add(6),
                *bytes.add(7),
            ])
        };
        let sid = unsafe { PSID(bytes.add(8) as *mut core::ffi::c_void) };
        let (verb, rights) = match ace_type {
            0 => ("Allow", access_mask_text(mask)),
            1 => ("Deny", access_mask_text(mask)),
            2 => ("Audit", access_mask_text(mask)),
            _ => continue,
        };
        let who = sid_name(sid).unwrap_or_else(|| "Everyone".to_owned());
        entries.push(format!("{verb} {who}: {rights}"));
    }
    entries
}

#[cfg(windows)]
fn access_mask_text(mask: u32) -> String {
    const FILE_ALL_ACCESS: u32 = 0x001F_01FF;
    const FILE_GENERIC_READ: u32 = 0x0012_0089;
    const FILE_GENERIC_WRITE: u32 = 0x0012_0116;
    const FILE_GENERIC_EXECUTE: u32 = 0x0012_00A0;
    const DELETE: u32 = 0x0001_0000;
    const WRITE_DAC: u32 = 0x0004_0000;
    const WRITE_OWNER: u32 = 0x0008_0000;

    if mask == FILE_ALL_ACCESS {
        return "Full control".to_owned();
    }

    let mut parts = Vec::new();
    if mask & FILE_GENERIC_READ == FILE_GENERIC_READ {
        parts.push("Read");
    }
    if mask & FILE_GENERIC_WRITE == FILE_GENERIC_WRITE {
        parts.push("Write");
    }
    if mask & FILE_GENERIC_EXECUTE == FILE_GENERIC_EXECUTE {
        parts.push("Execute");
    }
    if mask & DELETE != 0 {
        parts.push("Delete");
    }
    if mask & WRITE_DAC != 0 {
        parts.push("Change permissions");
    }
    if mask & WRITE_OWNER != 0 {
        parts.push("Take ownership");
    }
    if parts.is_empty() {
        format!("0x{mask:08X}")
    } else {
        parts.join(" & ")
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::{acl, signature};

    /// `notepad.exe` is a Microsoft-signed system binary on every supported
    /// Windows build; a freshly written temp file has no signature. Both
    /// verdicts go through the same `WinVerifyTrust` path the dialog uses.
    #[test]
    fn verifies_a_signed_binary_and_an_unsigned_file() {
        let signed = r"C:\Windows\System32\notepad.exe";
        if std::path::Path::new(signed).exists() {
            let result = signature(signed).expect("signed binary");
            assert_eq!(result.status, "valid", "{result:?}");
            assert!(result.signer.is_some(), "{result:?}");
        }

        let unsigned =
            std::env::temp_dir().join(format!("dae-unsigned-{}.txt", std::process::id()));
        std::fs::write(&unsigned, b"unsigned").expect("temp dir is writable");
        let result = signature(unsigned.to_str().unwrap()).expect("unsigned file");
        assert_eq!(result.status, "unsigned", "{result:?}");
        std::fs::remove_file(&unsigned).ok();
    }

    #[test]
    fn reads_owner_group_and_acl_entries() {
        let path = std::env::temp_dir().join(format!("dae-acl-{}.txt", std::process::id()));
        std::fs::write(&path, b"acl").expect("temp dir is writable");

        let acl = acl(path.to_str().unwrap()).expect("security descriptor");
        assert!(acl.owner.is_some(), "{acl:?}");
        assert!(!acl.entries.is_empty(), "{acl:?}");

        std::fs::remove_file(&path).ok();
    }
}
