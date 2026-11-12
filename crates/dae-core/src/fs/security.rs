//! File security facts for the properties dialog: an Authenticode signature
//! (read-only) and a discretionary ACL the dialog can also edit.
//!
//! Verifying a signature is a read; changing one would need a private key, so
//! that half stays read-only. The ACL half is an editor: Windows writes the
//! DACL through `SetNamedSecurityInfoW` (explicit ACEs, deny-before-allow
//! ordering, an inheritance toggle), and Linux writes POSIX ACLs through the
//! `system.posix_acl_access` xattr. The POSIX mode and DOS attribute editors
//! remain the writable surface for the base permission bits.
//!
//! Signatures are Windows-only (`WinVerifyTrust`); macOS's
//! `SecStaticCodeCheckValidity` is not built here — no macOS artifact ships —
//! and Linux has no file-signature concept. ACLs are Windows and Linux; macOS
//! NFSv4 ACLs are not built. Unsupported platforms report
//! [`FileSystemError::Unsupported`] and the dialog hides the section by asking
//! [`signature_supported`] / [`acl_supported`] first.

use serde::{Deserialize, Serialize};

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

/// One ACE as the editor shows it. `id` addresses the entry on write (a Windows
/// SID string, or a POSIX `u:<uid>` / `g:<gid>`); `principal` is the display
/// name. `rights` is a preset token (`read`, `modify`, `full`) or `custom`, in
/// which case `rights_text` carries the platform's own summary.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AclEntry {
    pub principal: String,
    pub id: String,
    /// False for a deny ACE. POSIX entries are always allow.
    pub allow: bool,
    pub rights: String,
    pub rights_text: String,
    pub inherited: bool,
    /// Whether the editor may remove or replace this entry. Inherited Windows
    /// ACEs and the POSIX base entries are shown but not editable here.
    pub editable: bool,
}

/// The owner, group and DACL entries of one file, plus whether the platform
/// can write it back and (Windows) whether the DACL inherits from its parent.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileAcl {
    pub owner: Option<String>,
    pub group: Option<String>,
    pub entries: Vec<AclEntry>,
    pub inheritance: bool,
    pub editable: bool,
}

/// One entry the editor wants in the ACL. `principal` is either a name to
/// resolve or an `id` echoed back from a read (a Windows SID string, or a POSIX
/// `u:<uid>` / `g:<gid>` / `user:<name>` / `group:<name>`). `rights` is a preset
/// (`read`, `modify`, `full`) or `custom`, which keeps the mask the principal
/// already has so an untouched entry is never rewritten through a preset.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AclEntrySpec {
    pub principal: String,
    #[serde(default = "default_allow")]
    pub allow: bool,
    pub rights: String,
}

/// The full set of explicit entries the editor wants, plus the inheritance
/// switch (Windows only; ignored on POSIX).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AclChanges {
    pub entries: Vec<AclEntrySpec>,
    #[serde(default = "default_allow")]
    pub inheritance: bool,
}

fn default_allow() -> bool {
    true
}

/// Whether this build can verify an Authenticode signature.
pub fn signature_supported() -> bool {
    cfg!(windows)
}

/// Whether this build can read a discretionary ACL (Windows DACL, Linux POSIX
/// ACL).
pub fn acl_supported() -> bool {
    cfg!(any(windows, target_os = "linux"))
}

/// Verifies `path`'s Authenticode signature. A missing signature is a verdict
/// (`unsigned`), not an error; an unreadable file is an error.
pub fn signature(path: &str) -> Result<FileSignature, FileSystemError> {
    signature_native(path)
}

/// Reads `path`'s owner, group and ACL entries. An entry with no DACL reports
/// an empty list rather than failing.
pub fn acl(path: &str) -> Result<FileAcl, FileSystemError> {
    acl_native(path)
}

/// Writes the editor's explicit ACL entries back to `path`. See [`AclChanges`]
/// for the contract; the base POSIX bits stay with the mode editor.
pub fn set_acl(path: &str, changes: &AclChanges) -> Result<(), FileSystemError> {
    set_acl_native(path, changes)
}

#[cfg(not(windows))]
fn signature_native(_path: &str) -> Result<FileSignature, FileSystemError> {
    Err(FileSystemError::Unsupported(
        "File signatures are only available on Windows".into(),
    ))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn acl_native(_path: &str) -> Result<FileAcl, FileSystemError> {
    Err(FileSystemError::Unsupported(
        "ACL reading is only available on Windows and Linux".into(),
    ))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn set_acl_native(_path: &str, _changes: &AclChanges) -> Result<(), FileSystemError> {
    Err(FileSystemError::Unsupported(
        "ACL editing is only available on Windows and Linux".into(),
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
        ACL, DACL_SECURITY_INFORMATION, GetSecurityDescriptorControl, GROUP_SECURITY_INFORMATION,
        OWNER_SECURITY_INFORMATION, PSID, PSECURITY_DESCRIPTOR, SE_DACL_PROTECTED,
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

    // The protected bit is the inheritance switch: a protected DACL does not
    // inherit from its parent. `GetSecurityDescriptorControl` reads it out of
    // the self-relative descriptor the call above returned.
    let mut control = 0u16;
    let mut revision = 0u32;
    let inheritance = unsafe {
        GetSecurityDescriptorControl(descriptor, &mut control, &mut revision).is_ok()
    } && (control & SE_DACL_PROTECTED.0 == 0);

    let acl = FileAcl {
        owner: sid_name(owner),
        group: sid_name(group),
        entries: dacl_entries(dacl),
        inheritance,
        editable: true,
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
fn dacl_entries(dacl: *mut windows::Win32::Security::ACL) -> Vec<AclEntry> {
    use windows::Win32::Security::{
        ACL_SIZE_INFORMATION, AclSizeInformation, GetAce, GetAclInformation, INHERITED_ACE, PSID,
    };

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
        let ace_flags = unsafe { *bytes.add(1) };
        let mask = unsafe {
            u32::from_le_bytes([
                *bytes.add(4),
                *bytes.add(5),
                *bytes.add(6),
                *bytes.add(7),
            ])
        };
        let sid = unsafe { PSID(bytes.add(8) as *mut core::ffi::c_void) };
        let allow = match ace_type {
            0 => true,
            1 => false,
            // Audit and object ACEs are not part of the editable DACL view.
            _ => continue,
        };
        let inherited = ace_flags & INHERITED_ACE.0 as u8 != 0;
        let id = sid_string(sid).unwrap_or_default();
        let principal = sid_name(sid).unwrap_or_else(|| id.clone());
        entries.push(AclEntry {
            principal,
            id,
            allow,
            rights: rights_preset(mask).to_owned(),
            rights_text: access_mask_text(mask),
            inherited,
            editable: !inherited,
        });
    }
    entries
}

/// The SID's `S-1-…` string form, the stable identity the editor echoes back
/// when it writes an entry it read earlier.
#[cfg(windows)]
fn sid_string(sid: windows::Win32::Security::PSID) -> Option<String> {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::ConvertSidToStringSidW;

    if sid.is_invalid() {
        return None;
    }
    let mut text = PWSTR::null();
    unsafe { ConvertSidToStringSidW(sid, &mut text) }.ok()?;
    if text.is_null() {
        return None;
    }
    let result = unsafe { text.to_string() }.ok();
    unsafe {
        let _ = LocalFree(Some(HLOCAL(text.0 as *mut _)));
    }
    result
}

/// The preset token a mask matches, or `custom` for anything else.
#[cfg(windows)]
fn rights_preset(mask: u32) -> &'static str {
    match mask {
        FILE_GENERIC_READ => "read",
        FILE_MODIFY => "modify",
        FILE_ALL_ACCESS => "full",
        _ => "custom",
    }
}

#[cfg(windows)]
const FILE_ALL_ACCESS: u32 = 0x001F_01FF;
#[cfg(windows)]
const FILE_GENERIC_READ: u32 = 0x0012_0089;
#[cfg(windows)]
const FILE_GENERIC_WRITE: u32 = 0x0012_0116;
#[cfg(windows)]
const FILE_GENERIC_EXECUTE: u32 = 0x0012_00A0;
#[cfg(windows)]
const DELETE: u32 = 0x0001_0000;
/// Explorer's "Modify": read, write, execute and delete, but not the rights to
/// change permissions or take ownership.
#[cfg(windows)]
const FILE_MODIFY: u32 =
    FILE_GENERIC_READ | FILE_GENERIC_WRITE | FILE_GENERIC_EXECUTE | DELETE;

#[cfg(windows)]
fn access_mask_text(mask: u32) -> String {
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

#[cfg(windows)]
struct ResolvedAce {
    sid: Vec<u8>,
    mask: u32,
    allow: bool,
}

#[cfg(windows)]
fn set_acl_native(path: &str, changes: &AclChanges) -> Result<(), FileSystemError> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::{
        GetNamedSecurityInfoW, SetNamedSecurityInfoW, SE_FILE_OBJECT,
    };
    use windows::Win32::Security::{
        ACL, DACL_SECURITY_INFORMATION, GROUP_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION, PSID, PSECURITY_DESCRIPTOR,
        UNPROTECTED_DACL_SECURITY_INFORMATION,
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

    let mut resolved: Vec<ResolvedAce> = Vec::new();
    for spec in &changes.entries {
        let sid = resolve_sid(&spec.principal)?;
        // `custom` keeps the mask the principal already has, so an entry the
        // editor showed but the user never touched round-trips unchanged.
        let mask = if spec.rights == "custom" {
            current_explicit_mask(dacl, &sid, spec.allow)?
        } else {
            rights_mask(&spec.rights)?
        };
        // One ACE per (trustee, verb); a repeated entry replaces the earlier one.
        match resolved
            .iter()
            .position(|entry| entry.sid == sid && entry.allow == spec.allow)
        {
            Some(index) => resolved[index] = ResolvedAce { sid, mask, allow: spec.allow },
            None => resolved.push(ResolvedAce { sid, mask, allow: spec.allow }),
        }
    }

    // Protecting the DACL converts the inherited ACEs into explicit ones so the
    // entry keeps the access it had; leaving it unprotected lets the system
    // re-propagate them from the parent, so they are not copied here.
    let inherited = if changes.inheritance {
        Vec::new()
    } else {
        inherited_aces(dacl)
    };
    let is_directory = std::fs::metadata(path).map(|meta| meta.is_dir()).unwrap_or(false);
    let new_acl = build_dacl(&resolved, &inherited, is_directory)?;

    let information = DACL_SECURITY_INFORMATION
        | if changes.inheritance {
            UNPROTECTED_DACL_SECURITY_INFORMATION
        } else {
            PROTECTED_DACL_SECURITY_INFORMATION
        };
    let result = unsafe {
        SetNamedSecurityInfoW(
            PCWSTR(wide.as_ptr()),
            SE_FILE_OBJECT,
            information,
            None,
            None,
            Some(new_acl.as_ptr() as *const ACL),
            None,
        )
    };

    unsafe {
        if !descriptor.is_invalid() {
            LocalFree(Some(HLOCAL(descriptor.0)));
        }
    }

    if result.0 != 0 {
        return Err(FileSystemError::PermissionDenied(format!(
            "Could not write the security descriptor (error {})",
            result.0
        )));
    }
    Ok(())
}

/// Resolves a principal to owned SID bytes: a `S-1-…` string is parsed directly,
/// anything else is looked up as an account name.
#[cfg(windows)]
fn resolve_sid(principal: &str) -> Result<Vec<u8>, FileSystemError> {
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::ConvertStringSidToSidW;
    use windows::Win32::Security::{GetLengthSid, LookupAccountNameW, PSID, SID_NAME_USE};

    let principal = principal.trim();
    if principal.is_empty() {
        return Err(FileSystemError::InvalidInput(
            "An ACL entry needs a user or group name".into(),
        ));
    }

    if principal.starts_with("S-1-") {
        let wide: Vec<u16> = principal.encode_utf16().chain(std::iter::once(0)).collect();
        let mut sid = PSID::default();
        unsafe { ConvertStringSidToSidW(PCWSTR(wide.as_ptr()), &mut sid) }.map_err(|_| {
            FileSystemError::InvalidInput(format!("Unknown security identifier: {principal}"))
        })?;
        let length = unsafe { GetLengthSid(sid) } as usize;
        let bytes = unsafe { std::slice::from_raw_parts(sid.0 as *const u8, length) }.to_vec();
        unsafe {
            let _ = LocalFree(Some(HLOCAL(sid.0 as *mut _)));
        }
        return Ok(bytes);
    }

    let wide: Vec<u16> = principal.encode_utf16().chain(std::iter::once(0)).collect();
    let mut sid_length = 0u32;
    let mut domain_length = 0u32;
    let mut use_type = SID_NAME_USE::default();
    // The first call only sizes the buffers and is expected to fail.
    let _ = unsafe {
        LookupAccountNameW(
            None,
            PCWSTR(wide.as_ptr()),
            None,
            &mut sid_length,
            None,
            &mut domain_length,
            &mut use_type,
        )
    };
    if sid_length == 0 {
        return Err(FileSystemError::InvalidInput(format!(
            "Unknown user or group: {principal}"
        )));
    }
    let mut sid = vec![0u8; sid_length as usize];
    let mut domain = vec![0u16; domain_length as usize];
    let ok = unsafe {
        LookupAccountNameW(
            None,
            PCWSTR(wide.as_ptr()),
            Some(PSID(sid.as_mut_ptr() as *mut _)),
            &mut sid_length,
            Some(PWSTR(domain.as_mut_ptr())),
            &mut domain_length,
            &mut use_type,
        )
    };
    if ok.is_err() {
        return Err(FileSystemError::InvalidInput(format!(
            "Unknown user or group: {principal}"
        )));
    }
    sid.truncate(sid_length as usize);
    Ok(sid)
}

#[cfg(windows)]
fn rights_mask(rights: &str) -> Result<u32, FileSystemError> {
    match rights {
        "read" => Ok(FILE_GENERIC_READ),
        "modify" => Ok(FILE_MODIFY),
        "full" => Ok(FILE_ALL_ACCESS),
        _ => Err(FileSystemError::InvalidInput(format!(
            "Unknown permission preset: {rights}"
        ))),
    }
}

/// The mask of the principal's current explicit ACE with the same verb, the
/// value a `custom` spec keeps. Inherited ACEs do not count: the editor only
/// sends back entries it read as explicit.
#[cfg(windows)]
fn current_explicit_mask(
    dacl: *mut windows::Win32::Security::ACL,
    sid: &[u8],
    allow: bool,
) -> Result<u32, FileSystemError> {
    use windows::Win32::Security::{
        ACL_SIZE_INFORMATION, AclSizeInformation, GetAce, GetAclInformation, GetLengthSid,
        INHERITED_ACE, PSID,
    };

    let missing = || {
        FileSystemError::InvalidInput(
            "The ACL entry no longer exists on the file; reload and try again".into(),
        )
    };
    if dacl.is_null() {
        return Err(missing());
    }
    let mut info = ACL_SIZE_INFORMATION::default();
    if unsafe {
        GetAclInformation(
            dacl,
            &mut info as *mut _ as *mut _,
            std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    }
    .is_err()
    {
        return Err(missing());
    }
    for index in 0..info.AceCount {
        let mut ace = std::ptr::null_mut();
        if unsafe { GetAce(dacl, index, &mut ace) }.is_err() || ace.is_null() {
            continue;
        }
        let bytes = ace as *const u8;
        let ace_type = unsafe { *bytes };
        let ace_flags = unsafe { *bytes.add(1) };
        if ace_type != if allow { 0 } else { 1 } || ace_flags & INHERITED_ACE.0 as u8 != 0 {
            continue;
        }
        let ace_sid = unsafe { PSID(bytes.add(8) as *mut core::ffi::c_void) };
        let length = unsafe { GetLengthSid(ace_sid) } as usize;
        if length != sid.len() {
            continue;
        }
        let ace_bytes = unsafe { std::slice::from_raw_parts(ace_sid.0 as *const u8, length) };
        if ace_bytes != sid {
            continue;
        }
        return Ok(unsafe {
            u32::from_le_bytes([*bytes.add(4), *bytes.add(5), *bytes.add(6), *bytes.add(7)])
        });
    }
    Err(missing())
}

/// Copies the inherited ACEs out of a DACL with the inherited flag cleared, so
/// they can be re-added as explicit entries when the DACL is protected.
#[cfg(windows)]
fn inherited_aces(dacl: *mut windows::Win32::Security::ACL) -> Vec<Vec<u8>> {
    use windows::Win32::Security::{
        ACL_SIZE_INFORMATION, AclSizeInformation, GetAce, GetAclInformation, INHERITED_ACE,
    };

    let mut result = Vec::new();
    if dacl.is_null() {
        return result;
    }
    let mut info = ACL_SIZE_INFORMATION::default();
    if unsafe {
        GetAclInformation(
            dacl,
            &mut info as *mut _ as *mut _,
            std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    }
    .is_err()
    {
        return result;
    }
    for index in 0..info.AceCount {
        let mut ace = std::ptr::null_mut();
        if unsafe { GetAce(dacl, index, &mut ace) }.is_err() || ace.is_null() {
            continue;
        }
        let bytes = ace as *const u8;
        let flags = unsafe { *bytes.add(1) };
        if flags & INHERITED_ACE.0 as u8 == 0 {
            continue;
        }
        let size = unsafe { u16::from_le_bytes([*bytes.add(2), *bytes.add(3)]) } as usize;
        let mut copied = unsafe { std::slice::from_raw_parts(bytes, size) }.to_vec();
        copied[1] &= !(INHERITED_ACE.0 as u8);
        result.push(copied);
    }
    result
}

/// Builds a fresh DACL: explicit denies, then explicit allows, then any
/// converted inherited ACEs. New ACEs on a directory inherit to children.
#[cfg(windows)]
fn build_dacl(
    entries: &[ResolvedAce],
    inherited: &[Vec<u8>],
    inherit_to_children: bool,
) -> Result<Vec<u8>, FileSystemError> {
    use windows::Win32::Security::{
        ACL, ACL_REVISION, ACE_FLAGS, AddAccessAllowedAceEx, AddAccessDeniedAceEx, AddAce,
        CONTAINER_INHERIT_ACE, GetLengthSid, InitializeAcl, OBJECT_INHERIT_ACE, PSID,
    };

    let fail = |error: windows::core::Error| {
        FileSystemError::Internal(format!("Could not build the access control list: {error}"))
    };
    let flags = if inherit_to_children {
        OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE
    } else {
        ACE_FLAGS(0)
    };

    let mut size = std::mem::size_of::<ACL>() as u32;
    for entry in entries {
        size += 8 + unsafe { GetLengthSid(PSID(entry.sid.as_ptr() as *mut _)) };
    }
    for ace in inherited {
        size += ace.len() as u32;
    }

    let mut buffer = vec![0u8; size as usize];
    let acl = buffer.as_mut_ptr() as *mut ACL;
    unsafe { InitializeAcl(acl, size, ACL_REVISION) }.map_err(fail)?;

    let mut count = 0u32;
    for entry in entries.iter().filter(|entry| !entry.allow) {
        unsafe {
            AddAccessDeniedAceEx(
                acl,
                ACL_REVISION,
                flags,
                entry.mask,
                PSID(entry.sid.as_ptr() as *mut _),
            )
        }
        .map_err(fail)?;
        count += 1;
    }
    for entry in entries.iter().filter(|entry| entry.allow) {
        unsafe {
            AddAccessAllowedAceEx(
                acl,
                ACL_REVISION,
                flags,
                entry.mask,
                PSID(entry.sid.as_ptr() as *mut _),
            )
        }
        .map_err(fail)?;
        count += 1;
    }
    for ace in inherited {
        unsafe { AddAce(acl, ACL_REVISION, count, ace.as_ptr() as *const _, ace.len() as u32) }
            .map_err(fail)?;
        count += 1;
    }

    Ok(buffer)
}

// -- POSIX ACL (Linux) ----------------------------------------------------------
//
// POSIX ACLs live in the `system.posix_acl_access` xattr: a 4-byte version
// header then 8-byte entries (tag u16, perm u16, id u32). Reading and writing
// it directly is what `getfacl`/`setfacl` do, so no external tool is needed.
// The base bits (owner/group/other) stay owned by the mode editor; this editor
// only adds and removes named user/group entries and recomputes the mask.

#[cfg(target_os = "linux")]
mod posix {
    use super::{AclChanges, AclEntry, FileAcl};
    use crate::error::FileSystemError;
    use crate::fs::local::properties::accounts;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    use std::path::Path;

    const XATTR: &str = "system.posix_acl_access";

    const USER_OBJ: u16 = 0x01;
    const USER: u16 = 0x02;
    const GROUP_OBJ: u16 = 0x04;
    const GROUP: u16 = 0x08;
    const MASK: u16 = 0x10;
    const OTHER: u16 = 0x20;

    const READ: u16 = 0x04;
    const WRITE: u16 = 0x02;
    const EXECUTE: u16 = 0x01;

    const UNDEFINED_ID: u32 = 0xFFFF_FFFF;

    struct Ace {
        tag: u16,
        perm: u16,
        id: u32,
    }

    pub fn read(path: &str) -> Result<FileAcl, FileSystemError> {
        let meta = std::fs::metadata(path)?;
        let mode = meta.mode();
        let owner = accounts::user_name_by_uid(meta.uid())
            .unwrap_or_else(|| meta.uid().to_string());
        let group = accounts::group_name_by_gid(meta.gid())
            .unwrap_or_else(|| meta.gid().to_string());

        let aces = match read_xattr(Path::new(path))? {
            Some(blob) => parse(&blob)?,
            None => vec![
                Ace { tag: USER_OBJ, perm: (mode >> 6) as u16 & 7, id: UNDEFINED_ID },
                Ace { tag: GROUP_OBJ, perm: (mode >> 3) as u16 & 7, id: UNDEFINED_ID },
                Ace { tag: OTHER, perm: mode as u16 & 7, id: UNDEFINED_ID },
            ],
        };

        Ok(FileAcl {
            owner: Some(owner),
            group: Some(group),
            entries: aces.iter().map(entry).collect(),
            inheritance: false,
            editable: true,
        })
    }

    pub fn write(path: &str, changes: &AclChanges) -> Result<(), FileSystemError> {
        // The base entries come from the current ACL (or the mode), never from
        // the editor, so a save here cannot clobber an unsaved mode-tab edit.
        // A `custom` spec also keeps the perm the principal already has.
        let current = match read_xattr(Path::new(path))? {
            Some(blob) => parse(&blob)?,
            None => Vec::new(),
        };

        let mut named: Vec<Ace> = Vec::new();
        for spec in &changes.entries {
            let (tag, id) = resolve_principal(&spec.principal)?;
            let perm = if spec.rights == "custom" {
                current
                    .iter()
                    .find(|ace| ace.tag == tag && ace.id == id)
                    .map(|ace| ace.perm)
                    .ok_or_else(|| {
                        FileSystemError::InvalidInput(
                            "The ACL entry no longer exists on the file; reload and try again"
                                .into(),
                        )
                    })?
            } else {
                rights_mask(&spec.rights)?
            };
            match named.iter().position(|ace| ace.tag == tag && ace.id == id) {
                Some(index) => named[index] = Ace { tag, perm, id },
                None => named.push(Ace { tag, perm, id }),
            }
        }

        // No extended entries left: drop the ACL so the file is plain mode bits.
        if named.is_empty() {
            return remove_xattr(Path::new(path));
        }

        let mode = std::fs::metadata(path)?.mode();
        let base = |tag: u16, shift: u32| -> u16 {
            current
                .iter()
                .find(|ace| ace.tag == tag)
                .map(|ace| ace.perm)
                .unwrap_or((mode >> shift) as u16 & 7)
        };
        let user_obj = base(USER_OBJ, 6);
        let group_obj = base(GROUP_OBJ, 3);
        let other = base(OTHER, 0);

        let mut users: Vec<&Ace> = named.iter().filter(|ace| ace.tag == USER).collect();
        let mut groups: Vec<&Ace> = named.iter().filter(|ace| ace.tag == GROUP).collect();
        users.sort_by_key(|ace| ace.id);
        groups.sort_by_key(|ace| ace.id);

        // The mask is the union of the owning group and every named entry, the
        // effective ceiling the kernel applies to all of them.
        let mut mask = group_obj;
        for ace in &named {
            mask |= ace.perm;
        }

        let mut aces = vec![Ace { tag: USER_OBJ, perm: user_obj, id: UNDEFINED_ID }];
        aces.extend(users.iter().map(|ace| Ace { tag: USER, perm: ace.perm, id: ace.id }));
        aces.push(Ace { tag: GROUP_OBJ, perm: group_obj, id: UNDEFINED_ID });
        aces.extend(groups.iter().map(|ace| Ace { tag: GROUP, perm: ace.perm, id: ace.id }));
        aces.push(Ace { tag: MASK, perm: mask, id: UNDEFINED_ID });
        aces.push(Ace { tag: OTHER, perm: other, id: UNDEFINED_ID });

        write_xattr(Path::new(path), &encode(&aces))
    }

    fn entry(ace: &Ace) -> AclEntry {
        let (principal, id, editable) = match ace.tag {
            USER_OBJ => ("Owner".to_owned(), "owner".to_owned(), false),
            GROUP_OBJ => ("Owning group".to_owned(), "group".to_owned(), false),
            OTHER => ("Other".to_owned(), "other".to_owned(), false),
            MASK => ("Mask".to_owned(), "mask".to_owned(), false),
            USER => (
                accounts::user_name_by_uid(ace.id).unwrap_or_else(|| ace.id.to_string()),
                format!("u:{}", ace.id),
                true,
            ),
            GROUP => (
                accounts::group_name_by_gid(ace.id).unwrap_or_else(|| ace.id.to_string()),
                format!("g:{}", ace.id),
                true,
            ),
            _ => (format!("tag {}", ace.tag), String::new(), false),
        };
        AclEntry {
            principal,
            id,
            allow: true,
            rights: rights_preset(ace.perm).to_owned(),
            rights_text: perm_text(ace.perm),
            inherited: false,
            editable,
        }
    }

    fn resolve_principal(principal: &str) -> Result<(u16, u32), FileSystemError> {
        let principal = principal.trim();
        let (kind, name) = principal.split_once(':').ok_or_else(|| {
            FileSystemError::InvalidInput(format!(
                "Expected user:<name> or group:<name>, got {principal}"
            ))
        })?;
        let (tag, is_user) = match kind {
            "user" | "u" => (USER, true),
            "group" | "g" => (GROUP, false),
            _ => {
                return Err(FileSystemError::InvalidInput(format!(
                    "Unknown ACL principal kind: {kind}"
                )));
            }
        };
        if let Ok(id) = name.parse::<u32>() {
            return Ok((tag, id));
        }
        let id = if is_user {
            accounts::uid_by_name(name)
        } else {
            accounts::gid_by_name(name)
        };
        id.map(|id| (tag, id)).ok_or_else(|| {
            FileSystemError::InvalidInput(format!("Unknown user or group: {name}"))
        })
    }

    fn rights_mask(rights: &str) -> Result<u16, FileSystemError> {
        match rights {
            "read" => Ok(READ),
            "modify" => Ok(READ | WRITE),
            "full" => Ok(READ | WRITE | EXECUTE),
            _ => Err(FileSystemError::InvalidInput(format!(
                "Unknown permission preset: {rights}"
            ))),
        }
    }

    fn rights_preset(perm: u16) -> &'static str {
        match perm & 7 {
            4 => "read",
            6 => "modify",
            7 => "full",
            _ => "custom",
        }
    }

    fn perm_text(perm: u16) -> String {
        let mut text = String::new();
        text.push(if perm & READ != 0 { 'r' } else { '-' });
        text.push(if perm & WRITE != 0 { 'w' } else { '-' });
        text.push(if perm & EXECUTE != 0 { 'x' } else { '-' });
        text
    }

    fn parse(blob: &[u8]) -> Result<Vec<Ace>, FileSystemError> {
        if blob.len() < 4 || u32::from_le_bytes([blob[0], blob[1], blob[2], blob[3]]) != 2 {
            return Err(FileSystemError::InvalidInput(
                "Malformed POSIX ACL header".into(),
            ));
        }
        let body = &blob[4..];
        if body.len() % 8 != 0 {
            return Err(FileSystemError::InvalidInput(
                "Malformed POSIX ACL entries".into(),
            ));
        }
        Ok(body
            .chunks_exact(8)
            .map(|chunk| Ace {
                tag: u16::from_le_bytes([chunk[0], chunk[1]]),
                perm: u16::from_le_bytes([chunk[2], chunk[3]]),
                id: u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]),
            })
            .collect())
    }

    fn encode(aces: &[Ace]) -> Vec<u8> {
        let mut blob = Vec::with_capacity(4 + aces.len() * 8);
        blob.extend_from_slice(&2u32.to_le_bytes());
        for ace in aces {
            blob.extend_from_slice(&ace.tag.to_le_bytes());
            blob.extend_from_slice(&ace.perm.to_le_bytes());
            blob.extend_from_slice(&ace.id.to_le_bytes());
        }
        blob
    }

    fn names(path: &Path) -> Result<(CString, CString), FileSystemError> {
        let cpath = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            FileSystemError::InvalidInput("path contains an interior NUL byte".into())
        })?;
        let cname = CString::new(XATTR).expect("xattr name has no NUL");
        Ok((cpath, cname))
    }

    fn read_xattr(path: &Path) -> Result<Option<Vec<u8>>, FileSystemError> {
        let (cpath, cname) = names(path)?;
        loop {
            let size =
                unsafe { libc::getxattr(cpath.as_ptr(), cname.as_ptr(), std::ptr::null_mut(), 0) };
            if size < 0 {
                let error = std::io::Error::last_os_error();
                if absent(&error) {
                    return Ok(None);
                }
                return Err(error.into());
            }
            let mut buffer = vec![0u8; size as usize];
            let read = unsafe {
                libc::getxattr(
                    cpath.as_ptr(),
                    cname.as_ptr(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                )
            };
            if read < 0 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::ERANGE) {
                    continue;
                }
                if absent(&error) {
                    return Ok(None);
                }
                return Err(error.into());
            }
            buffer.truncate(read as usize);
            return Ok(Some(buffer));
        }
    }

    /// True when the error means "no ACL here" rather than a real failure:
    /// the attribute is absent, or the file system does not support ACLs.
    fn absent(error: &std::io::Error) -> bool {
        matches!(
            error.raw_os_error(),
            Some(libc::ENODATA) | Some(libc::EOPNOTSUPP)
        )
    }

    fn write_xattr(path: &Path, blob: &[u8]) -> Result<(), FileSystemError> {
        let (cpath, cname) = names(path)?;
        let result = unsafe {
            libc::setxattr(
                cpath.as_ptr(),
                cname.as_ptr(),
                blob.as_ptr().cast(),
                blob.len(),
                0,
            )
        };
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EOPNOTSUPP) {
                return Err(FileSystemError::Unsupported(
                    "This file system does not support POSIX ACLs".into(),
                ));
            }
            return Err(error.into());
        }
        Ok(())
    }

    fn remove_xattr(path: &Path) -> Result<(), FileSystemError> {
        let (cpath, cname) = names(path)?;
        let result = unsafe { libc::removexattr(cpath.as_ptr(), cname.as_ptr()) };
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if matches!(error.raw_os_error(), Some(libc::ENODATA) | Some(libc::EOPNOTSUPP)) {
                return Ok(());
            }
            return Err(error.into());
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn acl_native(path: &str) -> Result<FileAcl, FileSystemError> {
    posix::read(path)
}

#[cfg(target_os = "linux")]
fn set_acl_native(path: &str, changes: &AclChanges) -> Result<(), FileSystemError> {
    posix::write(path, changes)
}

#[cfg(all(test, windows))]
mod tests {
    use super::{AclChanges, AclEntrySpec, acl, set_acl, signature};
    use crate::error::FileSystemError;

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

    /// Protecting the DACL converts the inherited ACEs into explicit ones;
    /// re-enabling inheritance lets the parent's ACEs flow back in. `S-1-1-0`
    /// is Everyone on every locale, so the test never depends on a translated
    /// account name.
    #[test]
    fn writes_dacl_entries_and_toggles_inheritance() {
        let path = std::env::temp_dir().join(format!("dae-acl-write-{}.txt", std::process::id()));
        std::fs::write(&path, b"acl").expect("temp dir is writable");
        let path_str = path.to_str().unwrap();

        let protect = AclChanges {
            entries: vec![AclEntrySpec {
                principal: "S-1-1-0".to_owned(),
                allow: true,
                rights: "read".to_owned(),
            }],
            inheritance: false,
        };
        set_acl(path_str, &protect).expect("protected DACL write");

        let snapshot = acl(path_str).expect("read back protected DACL");
        assert!(!snapshot.inheritance, "{snapshot:?}");
        let everyone = snapshot
            .entries
            .iter()
            .find(|entry| entry.id == "S-1-1-0")
            .expect("Everyone entry");
        assert!(everyone.allow, "{everyone:?}");
        assert_eq!(everyone.rights, "read", "{everyone:?}");
        assert!(!everyone.inherited, "{everyone:?}");
        assert!(
            snapshot.entries.iter().all(|entry| !entry.inherited),
            "protecting converts inherited ACEs: {snapshot:?}"
        );

        // An untouched `custom` entry keeps its mask rather than being
        // rewritten through a preset.
        let keep = AclChanges {
            entries: vec![AclEntrySpec {
                principal: "S-1-1-0".to_owned(),
                allow: true,
                rights: "custom".to_owned(),
            }],
            inheritance: false,
        };
        set_acl(path_str, &keep).expect("custom mask preservation");
        let snapshot = acl(path_str).expect("read back preserved mask");
        let everyone = snapshot
            .entries
            .iter()
            .find(|entry| entry.id == "S-1-1-0")
            .expect("Everyone entry");
        assert_eq!(everyone.rights, "read", "{everyone:?}");

        let inherit = AclChanges {
            entries: vec![AclEntrySpec {
                principal: "S-1-1-0".to_owned(),
                allow: true,
                rights: "modify".to_owned(),
            }],
            inheritance: true,
        };
        set_acl(path_str, &inherit).expect("unprotected DACL write");

        let snapshot = acl(path_str).expect("read back inherited DACL");
        assert!(snapshot.inheritance, "{snapshot:?}");
        let everyone = snapshot
            .entries
            .iter()
            .find(|entry| entry.id == "S-1-1-0" && !entry.inherited)
            .expect("explicit Everyone entry");
        assert_eq!(everyone.rights, "modify", "{everyone:?}");
        assert!(
            snapshot.entries.iter().any(|entry| entry.inherited),
            "the temp folder propagates inheritable ACEs: {snapshot:?}"
        );

        let unknown = AclChanges {
            entries: vec![AclEntrySpec {
                principal: "dae-no-such-account-xyz".to_owned(),
                allow: true,
                rights: "read".to_owned(),
            }],
            inheritance: true,
        };
        assert!(
            matches!(set_acl(path_str, &unknown), Err(FileSystemError::InvalidInput(_))),
            "an unresolvable principal is invalid input"
        );

        std::fs::remove_file(&path).ok();
    }
}

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::{AclChanges, AclEntrySpec, acl, set_acl};
    use crate::error::FileSystemError;
    use std::os::unix::fs::MetadataExt;

    /// A named user and group entry round-trip through the xattr, the mask is
    /// recomputed, and removing every named entry drops back to plain mode
    /// bits. Skipped when the temp file system has no ACL support.
    #[test]
    fn writes_posix_acl_entries_and_removes_them() {
        let path = std::env::temp_dir().join(format!("dae-acl-{}.txt", std::process::id()));
        std::fs::write(&path, b"acl").expect("temp dir is writable");
        let path_str = path.to_str().unwrap();
        let uid = std::fs::metadata(&path).expect("metadata").uid();

        let changes = AclChanges {
            entries: vec![
                AclEntrySpec {
                    principal: format!("u:{uid}"),
                    allow: true,
                    rights: "modify".to_owned(),
                },
                AclEntrySpec {
                    principal: "g:0".to_owned(),
                    allow: true,
                    rights: "read".to_owned(),
                },
            ],
            inheritance: true,
        };
        match set_acl(path_str, &changes) {
            Ok(()) => {}
            Err(FileSystemError::Unsupported(_)) => {
                std::fs::remove_file(&path).ok();
                return;
            }
            Err(error) => panic!("write POSIX ACL: {error}"),
        }

        let snapshot = acl(path_str).expect("read back POSIX ACL");
        let user = snapshot
            .entries
            .iter()
            .find(|entry| entry.id == format!("u:{uid}"))
            .expect("named user entry");
        assert_eq!(user.rights, "modify", "{user:?}");
        assert!(user.editable, "{user:?}");
        let group = snapshot
            .entries
            .iter()
            .find(|entry| entry.id == "g:0")
            .expect("named group entry");
        assert_eq!(group.rights, "read", "{group:?}");
        let mask = snapshot
            .entries
            .iter()
            .find(|entry| entry.id == "mask")
            .expect("mask entry");
        assert_eq!(mask.rights_text, "rw-", "{mask:?}");
        assert!(
            snapshot.entries.iter().any(|entry| entry.id == "owner" && !entry.editable),
            "base entries stay read-only: {snapshot:?}"
        );

        // Untouched `custom` entries keep their perms.
        let keep = AclChanges {
            entries: vec![
                AclEntrySpec {
                    principal: format!("u:{uid}"),
                    allow: true,
                    rights: "custom".to_owned(),
                },
                AclEntrySpec {
                    principal: "g:0".to_owned(),
                    allow: true,
                    rights: "custom".to_owned(),
                },
            ],
            inheritance: true,
        };
        set_acl(path_str, &keep).expect("custom perm preservation");
        let snapshot = acl(path_str).expect("read back preserved perms");
        let user = snapshot
            .entries
            .iter()
            .find(|entry| entry.id == format!("u:{uid}"))
            .expect("named user entry");
        assert_eq!(user.rights, "modify", "{user:?}");

        let empty = AclChanges { entries: Vec::new(), inheritance: true };
        set_acl(path_str, &empty).expect("drop the extended ACL");
        let snapshot = acl(path_str).expect("read back plain mode");
        assert!(
            snapshot.entries.iter().all(|entry| !entry.editable),
            "no named entries remain: {snapshot:?}"
        );

        let unknown = AclChanges {
            entries: vec![AclEntrySpec {
                principal: "user:dae-no-such-user-xyz".to_owned(),
                allow: true,
                rights: "read".to_owned(),
            }],
            inheritance: true,
        };
        assert!(
            matches!(set_acl(path_str, &unknown), Err(FileSystemError::InvalidInput(_))),
            "an unresolvable principal is invalid input"
        );

        std::fs::remove_file(&path).ok();
    }
}
