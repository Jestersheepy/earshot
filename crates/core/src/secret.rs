//! Small secrets (API keys) in Windows Credential Manager, so they never sit in config.json. Stored as
//! generic credentials for the current user, visible under Control Panel > Credential Manager.

use windows::Win32::Foundation::FILETIME;
use windows::Win32::Security::Credentials::{
    CRED_FLAGS, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW, CredWriteW,
};
use windows::core::{HSTRING, PWSTR};

fn target(name: &str) -> HSTRING {
    HSTRING::from(format!("Earshot/{name}"))
}

pub fn get(name: &str) -> Option<String> {
    unsafe {
        let mut cred: *mut CREDENTIALW = std::ptr::null_mut();
        CredReadW(&target(name), CRED_TYPE_GENERIC, None, &mut cred).ok()?;
        let c = &*cred;
        let bytes = std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize);
        let s = String::from_utf8(bytes.to_vec()).ok();
        CredFree(cred as *const _);
        s.filter(|s| !s.is_empty())
    }
}

/// Saves `value`, or removes the secret when it's empty.
pub fn set(name: &str, value: &str) -> windows::core::Result<()> {
    let t = target(name);
    if value.is_empty() {
        return match unsafe { CredDeleteW(&t, CRED_TYPE_GENERIC, None) } {
            Err(e) if e.code() != windows::Win32::Foundation::ERROR_NOT_FOUND.to_hresult() => Err(e),
            _ => Ok(()),
        };
    }
    let mut blob = value.as_bytes().to_vec();
    let mut target_w: Vec<u16> = t.to_string().encode_utf16().chain([0]).collect();
    let cred = CREDENTIALW {
        Flags: CRED_FLAGS(0),
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target_w.as_mut_ptr()),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        LastWritten: FILETIME::default(),
        ..Default::default()
    };
    unsafe { CredWriteW(&cred, 0) }
}

#[cfg(test)]
mod tests {
    #[test]
    fn round_trip() {
        let name = "test-secret";
        super::set(name, "sk-test-123").unwrap();
        assert_eq!(super::get(name).as_deref(), Some("sk-test-123"));
        super::set(name, "").unwrap();
        assert_eq!(super::get(name), None);
        super::set(name, "").unwrap(); // removing twice is fine
    }
}
