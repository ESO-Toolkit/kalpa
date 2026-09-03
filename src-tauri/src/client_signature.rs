//! Authenticode verification for user-supplied binaries.
//!
//! This is the only gate between "a DLL the user picked off their disk" and
//! "a DLL Kalpa writes next to `eso64.exe`". Kalpa never downloads NVIDIA
//! runtimes — they are not licensed for redistribution — so the DLSS and
//! Neural Rendering paths depend entirely on the user supplying a file, and
//! entirely on this module to decide whether it is what it claims to be.
//!
//! # Why a hash allowlist is not enough on its own
//!
//! DLSS Swapper distributed community-submitted DLLs vetted by hash alone, and
//! in 2026 people submitted malware into that manifest. A hash proves a file
//! matches something someone previously vouched for; it says nothing about who
//! authored it. Conversely a signature proves authorship but not desirability.
//! Callers that care about *which* NVIDIA build should check both.
//!
//! # What "verified" means here
//!
//! [`verify_authenticode`] reports whether the embedded signature chains to a
//! root the OS trusts, and who the leaf certificate names. It deliberately does
//! **not** decide policy — no allowlist, no "is this NVIDIA" opinion. Policy
//! belongs to the caller, so the same primitive can gate an NVIDIA runtime, a
//! `d3dcompiler` replacement, and `ReShade_Setup.exe` without this module
//! growing opinions about each.
//!
//! Non-Windows targets always report unverified: Authenticode is a Windows
//! construct, and the client-directory manager is Windows-only anyway.

use serde::Serialize;
use std::path::Path;

/// The result of checking one file's embedded signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SignatureInfo {
    /// True only when the signature is present, intact, and chains to a root
    /// the OS trusts. Any failure — absent, malformed, expired, self-signed,
    /// revoked — is `false`.
    pub trusted: bool,
    /// Common Name of the signing certificate's subject, e.g.
    /// `NVIDIA Corporation`. `None` when absent or unreadable.
    pub signer_common_name: Option<String>,
    /// Full subject string, for display when the CN alone is ambiguous.
    pub subject: Option<String>,
    /// Why verification failed, for the UI. `None` when `trusted`.
    pub failure_reason: Option<String>,
}

impl SignatureInfo {
    /// An untrusted result carrying a reason.
    pub fn untrusted(reason: impl Into<String>) -> Self {
        Self {
            trusted: false,
            signer_common_name: None,
            subject: None,
            failure_reason: Some(reason.into()),
        }
    }
}

/// Verify a file's embedded Authenticode signature.
///
/// Never panics and never returns an error: an unverifiable file is a normal
/// answer the UI renders, not an exceptional condition.
pub fn verify_authenticode(path: &Path) -> SignatureInfo {
    // Cross-platform sanity checks so both backends behave identically for
    // the boring cases, instead of every implementation re-deriving them.
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_dir() => {
            return SignatureInfo::untrusted("Path is a directory, not a file.");
        }
        Ok(meta) if !meta.is_file() => {
            return SignatureInfo::untrusted("Path is not a regular file.");
        }
        Ok(_) => {}
        Err(_) => {
            return SignatureInfo::untrusted("File does not exist or is not accessible.");
        }
    }

    imp::verify_authenticode(path)
}

/// True when the file is trusted **and** its signer common name matches
/// `expected_cn` exactly, case-insensitively.
///
/// Callers pass the CN they require rather than this module hardcoding one, so
/// the same check serves NVIDIA runtimes and any other vendor.
pub fn is_signed_by(path: &Path, expected_cn: &str) -> bool {
    let info = verify_authenticode(path);
    info.trusted
        && info
            .signer_common_name
            .as_deref()
            .is_some_and(|cn| cn.eq_ignore_ascii_case(expected_cn))
}

// ---------------------------------------------------------------------------
// Windows implementation: WinVerifyTrust + CryptoAPI certificate extraction.
// ---------------------------------------------------------------------------

// Cargo.toml enables `Win32_Security_WinTrust` and `Win32_Security_Cryptography`
// on the `windows` crate, so this module uses the generated bindings for
// `wintrust.dll` / `crypt32.dll` instead of hand-declared `extern "system"`
// blocks and hand-transcribed `#[repr(C)]` structs. That matters here
// specifically: a single wrong field offset in a hand-rolled `CERT_INFO`
// would mean reading the wrong bytes while reporting a confident "trusted",
// in the one place that gates writing a user-supplied binary into the game
// folder.
#[cfg(target_os = "windows")]
mod imp {
    use super::SignatureInfo;
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Security::Cryptography::{
        CertCloseStore, CertFindCertificateInStore, CertFreeCertificateContext, CertGetNameStringW,
        CryptMsgClose, CryptMsgGetParam, CryptQueryObject, CERT_CONTEXT, CERT_FIND_SUBJECT_CERT,
        CERT_INFO, CERT_NAME_RDN_TYPE, CERT_NAME_SIMPLE_DISPLAY_TYPE,
        CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED_EMBED, CERT_QUERY_FORMAT_FLAG_BINARY,
        CERT_QUERY_OBJECT_FILE, CMSG_SIGNER_INFO, CMSG_SIGNER_INFO_PARAM, HCERTSTORE,
        PKCS_7_ASN_ENCODING, X509_ASN_ENCODING,
    };
    use windows::Win32::Security::WinTrust::{
        WinVerifyTrust, WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0,
        WINTRUST_FILE_INFO, WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE,
        WTD_STATEACTION_VERIFY, WTD_UI_NONE,
    };

    /// Well-known WinVerifyTrust / CryptoAPI result codes, mapped to a
    /// user-facing explanation instead of a bare hex code.
    fn describe_trust_result(code: i32) -> String {
        // These constants are HRESULT-shaped values that WinVerifyTrust
        // returns directly as its LONG result, not wrapped in a Result<>.
        const TRUST_E_NOSIGNATURE: u32 = 0x800B_0100;
        const TRUST_E_BAD_DIGEST: u32 = 0x8009_6010;
        const TRUST_E_EXPLICIT_DISTRUST: u32 = 0x800B_0111;
        const TRUST_E_SUBJECT_NOT_TRUSTED: u32 = 0x800B_0004;
        const CERT_E_UNTRUSTEDROOT: u32 = 0x800B_0109;
        const CERT_E_EXPIRED: u32 = 0x800B_0101;
        const CERT_E_CHAINING: u32 = 0x800B_010A;
        const CERT_E_REVOKED: u32 = 0x800B_010C;
        const CRYPT_E_SECURITY_SETTINGS: u32 = 0x8009_2026;

        match code as u32 {
            TRUST_E_NOSIGNATURE => "File is not signed.".to_string(),
            TRUST_E_BAD_DIGEST => "File was modified after it was signed.".to_string(),
            CERT_E_UNTRUSTEDROOT => {
                "Signature is self-signed or chains to a root the system does not trust."
                    .to_string()
            }
            CERT_E_EXPIRED => "Signing certificate has expired.".to_string(),
            CERT_E_REVOKED => "Signing certificate has been revoked.".to_string(),
            CERT_E_CHAINING => "Signature does not chain to a trusted root.".to_string(),
            TRUST_E_EXPLICIT_DISTRUST => {
                "Signature is explicitly distrusted by policy.".to_string()
            }
            TRUST_E_SUBJECT_NOT_TRUSTED => "Subject is explicitly not trusted.".to_string(),
            CRYPT_E_SECURITY_SETTINGS => {
                "Local security settings prohibit verifying this signature.".to_string()
            }
            other => format!("Signature verification failed (error 0x{other:08X})."),
        }
    }

    /// Wide (UTF-16, NUL-terminated) encoding of a path for Win32 wide APIs.
    fn to_wide(path: &Path) -> Vec<u16> {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    /// Runs WinVerifyTrust for the given file and returns the raw result
    /// code. Always issues the matching `WTD_STATEACTION_CLOSE` call so no
    /// state handle is leaked, regardless of the verify outcome.
    fn win_verify_trust(wide_path: &[u16]) -> i32 {
        let mut file_info = WINTRUST_FILE_INFO {
            cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
            pcwszFilePath: PCWSTR(wide_path.as_ptr()),
            hFile: Default::default(),
            pgKnownSubject: std::ptr::null_mut(),
        };

        let mut data = WINTRUST_DATA {
            cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
            pPolicyCallbackData: std::ptr::null_mut(),
            pSIPClientData: std::ptr::null_mut(),
            dwUIChoice: WTD_UI_NONE,
            fdwRevocationChecks: WTD_REVOKE_NONE,
            dwUnionChoice: WTD_CHOICE_FILE,
            Anonymous: WINTRUST_DATA_0 {
                pFile: &mut file_info,
            },
            dwStateAction: WTD_STATEACTION_VERIFY,
            hWVTStateData: Default::default(),
            pwszURLReference: Default::default(),
            dwProvFlags: Default::default(),
            dwUIContext: Default::default(),
            pSignatureSettings: std::ptr::null_mut(),
        };

        // WinVerifyTrust wants a *mut GUID, not the *const the action-id
        // constant is declared as, so a mutable local copy is needed.
        let mut action_id = WINTRUST_ACTION_GENERIC_VERIFY_V2;

        // SAFETY: `file_info` and `data` are valid, live for the duration of
        // the call, and `data.Anonymous.pFile` points at `file_info` which
        // outlives the call. `hwnd` is null (no UI, matches WTD_UI_NONE).
        let result = unsafe {
            WinVerifyTrust(
                HWND(std::ptr::null_mut()),
                &mut action_id,
                &mut data as *mut _ as *mut c_void,
            )
        };

        // Release the state handle opened by the VERIFY call above. Reuse the
        // same structs; only dwStateAction needs to change (hWVTStateData
        // was filled in by the VERIFY call itself).
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        // SAFETY: same struct, still valid; this call only tears down the
        // state WinVerifyTrust allocated above and must not be skipped.
        unsafe {
            let _ = WinVerifyTrust(
                HWND(std::ptr::null_mut()),
                &mut action_id,
                &mut data as *mut _ as *mut c_void,
            );
        }

        result
    }

    /// Best-effort extraction of the signer's subject / common name from the
    /// file's embedded PKCS#7 signature, independent of whether the chain is
    /// trusted. A user seeing "signed by X, but the root is untrusted" is far
    /// more actionable than a bare "unverified".
    ///
    /// Every handle opened here is closed on every return path, including
    /// early returns on failure.
    fn extract_signer(wide_path: &[u16]) -> Option<(Option<String>, Option<String>)> {
        let mut cert_store = HCERTSTORE::default();
        let mut crypt_msg: *mut c_void = std::ptr::null_mut();

        // SAFETY: all output pointers are valid for the duration of the call.
        let queried = unsafe {
            CryptQueryObject(
                CERT_QUERY_OBJECT_FILE,
                wide_path.as_ptr() as *const c_void,
                CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED_EMBED,
                CERT_QUERY_FORMAT_FLAG_BINARY,
                0,
                None,
                None,
                None,
                Some(&mut cert_store),
                Some(&mut crypt_msg),
                None,
            )
        };

        if queried.is_err() {
            return None;
        }

        // From here on, always close cert_store and crypt_msg before
        // returning, on every path (found, not-found, malformed).
        let result = (|| -> Option<(Option<String>, Option<String>)> {
            // First call: get the size of the CMSG_SIGNER_INFO blob.
            let mut needed: u32 = 0;
            // SAFETY: crypt_msg is a valid handle from the successful query above.
            let sized = unsafe {
                CryptMsgGetParam(crypt_msg, CMSG_SIGNER_INFO_PARAM, 0, None, &mut needed)
            };
            if sized.is_err() || needed == 0 {
                return None;
            }

            let mut buf: Vec<u8> = vec![0u8; needed as usize];
            // SAFETY: buf has capacity `needed`, matching the size query above.
            let filled = unsafe {
                CryptMsgGetParam(
                    crypt_msg,
                    CMSG_SIGNER_INFO_PARAM,
                    0,
                    Some(buf.as_mut_ptr() as *mut c_void),
                    &mut needed,
                )
            };
            if filled.is_err() || (needed as usize) < std::mem::size_of::<CMSG_SIGNER_INFO>() {
                return None;
            }

            // SAFETY: buf was sized and filled by CryptMsgGetParam above and
            // is at least as large as CMSG_SIGNER_INFO.
            let signer_info: &CMSG_SIGNER_INFO =
                unsafe { &*(buf.as_ptr() as *const CMSG_SIGNER_INFO) };

            let mut cert_info = CERT_INFO {
                Issuer: signer_info.Issuer,
                SerialNumber: signer_info.SerialNumber,
                ..Default::default()
            };

            // SAFETY: cert_store is the valid, open store from CryptQueryObject.
            // cert_info stays alive for the duration of this call.
            let cert_ctx = unsafe {
                CertFindCertificateInStore(
                    cert_store,
                    X509_ASN_ENCODING | PKCS_7_ASN_ENCODING,
                    0,
                    CERT_FIND_SUBJECT_CERT,
                    Some(&mut cert_info as *mut _ as *const c_void),
                    None,
                )
            };

            if cert_ctx.is_null() {
                return None;
            }

            // From here, always free cert_ctx before returning.
            let names = (
                get_cert_name(cert_ctx, CERT_NAME_SIMPLE_DISPLAY_TYPE),
                get_cert_name(cert_ctx, CERT_NAME_RDN_TYPE),
            );

            // SAFETY: cert_ctx came from a successful CertFindCertificateInStore
            // and has not been freed yet.
            unsafe {
                let _ = CertFreeCertificateContext(Some(cert_ctx));
            }

            Some(names)
        })();

        // SAFETY: crypt_msg and cert_store are valid handles obtained above;
        // this module only reaches this point once per call, so neither is
        // closed twice.
        unsafe {
            let _ = CryptMsgClose(Some(crypt_msg));
            let _ = CertCloseStore(Some(cert_store), 0);
        }

        result
    }

    /// Reads a display name string from a certificate context. Returns
    /// `None` on any failure (never panics).
    fn get_cert_name(cert_ctx: *const CERT_CONTEXT, name_type: u32) -> Option<String> {
        // SAFETY: cert_ctx is a live, valid certificate context for the
        // duration of this function.
        let len = unsafe { CertGetNameStringW(cert_ctx, name_type, 0, None, None) };
        if len <= 1 {
            return None;
        }

        let mut buf: Vec<u16> = vec![0u16; len as usize];
        // SAFETY: buf has capacity `len`, matching the size query above.
        let written = unsafe { CertGetNameStringW(cert_ctx, name_type, 0, None, Some(&mut buf)) };
        if written == 0 {
            return None;
        }

        // Trim the trailing NUL the API writes.
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let s = String::from_utf16_lossy(&buf[..end]);
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }

    pub(super) fn verify_authenticode(path: &Path) -> SignatureInfo {
        let wide_path = to_wide(path);

        let trust_code = win_verify_trust(&wide_path);
        let trusted = trust_code == 0;

        let signer = extract_signer(&wide_path);
        let (signer_common_name, subject) = signer.unwrap_or((None, None));

        let failure_reason = if trusted {
            None
        } else {
            Some(describe_trust_result(trust_code))
        };

        SignatureInfo {
            trusted,
            signer_common_name,
            subject,
            failure_reason,
        }
    }
}

// ---------------------------------------------------------------------------
// Non-Windows: Authenticode is a Windows-only construct.
// ---------------------------------------------------------------------------

#[cfg(not(target_os = "windows"))]
mod imp {
    use super::SignatureInfo;
    use std::path::Path;

    pub(super) fn verify_authenticode(_path: &Path) -> SignatureInfo {
        SignatureInfo::untrusted("Authenticode verification is only available on Windows.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonexistent_path_is_untrusted() {
        let path = Path::new("Z:/definitely/does/not/exist/kalpa-signature-test.dll");
        let info = verify_authenticode(path);
        assert!(!info.trusted);
        assert!(info.failure_reason.is_some());
    }

    #[test]
    fn directory_is_untrusted() {
        let dir = std::env::temp_dir();
        let info = verify_authenticode(&dir);
        assert!(!info.trusted);
        assert!(info.failure_reason.is_some());
    }

    #[cfg(windows)]
    #[test]
    fn trusted_microsoft_binary_is_verified() {
        let path = Path::new(r"C:\Windows\System32\kernel32.dll");
        if !path.exists() {
            eprintln!("skipping: {} not present on this machine", path.display());
            return;
        }

        let info = verify_authenticode(path);
        assert!(
            info.trusted,
            "expected kernel32.dll to be trusted, got {info:?}"
        );
        assert!(
            info.signer_common_name.is_some(),
            "expected a signer common name, got {info:?}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn unsigned_file_is_untrusted_with_no_signature_reason() {
        // Arbitrary bytes are not even a well-formed PE image, so WinTrust
        // reports a file-format error rather than "no signature" — that
        // codepath is legitimate but distinct. To exercise the actual
        // "valid file, no signature" case, take a real signed system DLL and
        // zero out its Attribute Certificate Table directory entry (the PE
        // optional header's Security data directory), which is exactly the
        // difference between a signed and unsigned build of the same binary.
        let source = Path::new(r"C:\Windows\System32\kernel32.dll");
        if !source.exists() {
            eprintln!("skipping: {} not present on this machine", source.display());
            return;
        }

        let mut bytes = std::fs::read(source).expect("read kernel32.dll");
        if !strip_security_directory(&mut bytes) {
            eprintln!("skipping: could not locate PE security directory to strip");
            return;
        }

        let dir = tempfile::tempdir().expect("create tempdir");
        let dest = dir.path().join("kernel32_unsigned.dll");
        std::fs::write(&dest, &bytes).expect("write stripped copy");

        let info = verify_authenticode(&dest);
        assert!(!info.trusted, "expected stripped copy to be untrusted");
        assert_eq!(info.failure_reason.as_deref(), Some("File is not signed."));
    }

    /// Zeroes the PE optional header's Security (Attribute Certificate
    /// Table) data directory entry in place, producing a structurally valid
    /// but unsigned PE image. Returns `false` (rather than panicking) if the
    /// bytes don't look like a PE this parser understands.
    #[cfg(windows)]
    fn strip_security_directory(bytes: &mut [u8]) -> bool {
        const IMAGE_DIRECTORY_ENTRY_SECURITY: usize = 4;
        const OPTIONAL_HEADER32_MAGIC: u16 = 0x10B;
        const OPTIONAL_HEADER64_MAGIC: u16 = 0x20B;
        // Bytes preceding the DataDirectory array within the optional
        // header, for each header variant.
        const DATA_DIRECTORY_OFFSET_32: usize = 96;
        const DATA_DIRECTORY_OFFSET_64: usize = 112;

        fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
            bytes
                .get(offset..offset + 2)
                .map(|s| u16::from_le_bytes([s[0], s[1]]))
        }
        fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
            bytes
                .get(offset..offset + 4)
                .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        }

        if bytes.len() < 0x40 || &bytes[0..2] != b"MZ" {
            return false;
        }
        let e_lfanew = match read_u32(bytes, 0x3C) {
            Some(v) => v as usize,
            None => return false,
        };
        if bytes.len() < e_lfanew + 24 || &bytes[e_lfanew..e_lfanew + 4] != b"PE\0\0" {
            return false;
        }

        let optional_header_offset = e_lfanew + 24;
        let magic = match read_u16(bytes, optional_header_offset) {
            Some(v) => v,
            None => return false,
        };
        let data_dir_offset_within_optional = if magic == OPTIONAL_HEADER32_MAGIC {
            DATA_DIRECTORY_OFFSET_32
        } else if magic == OPTIONAL_HEADER64_MAGIC {
            DATA_DIRECTORY_OFFSET_64
        } else {
            return false;
        };

        let security_entry_offset = optional_header_offset
            + data_dir_offset_within_optional
            + IMAGE_DIRECTORY_ENTRY_SECURITY * 8;

        if bytes.len() < security_entry_offset + 8 {
            return false;
        }

        bytes[security_entry_offset..security_entry_offset + 8].fill(0);
        true
    }

    #[cfg(windows)]
    #[test]
    fn tampered_signed_file_is_untrusted() {
        let source = Path::new(r"C:\Windows\System32\kernel32.dll");
        if !source.exists() {
            eprintln!("skipping: {} not present on this machine", source.display());
            return;
        }

        let dir = tempfile::tempdir().expect("create tempdir");
        let dest = dir.path().join("kernel32_tampered.dll");
        std::fs::copy(source, &dest).expect("copy kernel32.dll");

        // Confirm the untouched copy verifies first, so a failure below is
        // attributable to the tamper, not an unrelated environment issue.
        let baseline = verify_authenticode(&dest);
        if !baseline.trusted {
            eprintln!("skipping: untouched copy did not verify as trusted: {baseline:?}");
            return;
        }

        // Flip a byte roughly in the middle of the file — inside the signed
        // content, past the PE header and well before the certificate table
        // that trails most signed PE files.
        let len = std::fs::metadata(&dest).expect("stat copy").len();
        {
            use std::io::{Seek, SeekFrom, Write};
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .open(&dest)
                .expect("open copy for writing");
            let offset = len / 2;
            f.seek(SeekFrom::Start(offset)).expect("seek");
            let mut byte = [0u8; 1];
            std::io::Read::read_exact(
                &mut std::fs::File::open(&dest).expect("reopen for read"),
                &mut byte,
            )
            .ok();
            f.seek(SeekFrom::Start(offset)).expect("seek again");
            f.write_all(&[byte[0] ^ 0xFF]).expect("flip byte");
        }

        let info = verify_authenticode(&dest);
        assert!(
            !info.trusted,
            "expected tampered copy to be untrusted, got {info:?}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn is_signed_by_matches_case_insensitively_and_rejects_wrong_cn() {
        let path = Path::new(r"C:\Windows\System32\kernel32.dll");
        if !path.exists() {
            eprintln!("skipping: {} not present on this machine", path.display());
            return;
        }

        let info = verify_authenticode(path);
        if !info.trusted {
            eprintln!("skipping: kernel32.dll did not verify as trusted: {info:?}");
            return;
        }
        let cn = match info.signer_common_name {
            Some(cn) => cn,
            None => {
                eprintln!("skipping: no signer common name reported");
                return;
            }
        };

        assert!(is_signed_by(path, &cn));
        assert!(is_signed_by(path, &cn.to_uppercase()));
        assert!(is_signed_by(path, &cn.to_lowercase()));
        assert!(!is_signed_by(path, "Definitely Not The Right Vendor"));
    }
}
