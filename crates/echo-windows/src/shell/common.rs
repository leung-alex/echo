use sha2::{Digest, Sha256};
use std::{path::Path, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::*,
    Security::Authorization::*,
    Security::*,
    System::{RemoteDesktop::ProcessIdToSessionId, Threading::*},
};
pub(super) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
pub(super) fn error() -> String {
    std::io::Error::last_os_error().to_string()
}
pub(super) struct Handle(pub HANDLE);
impl Handle {
    pub unsafe fn new(h: HANDLE) -> Result<Self, String> {
        if h.is_null() || h == INVALID_HANDLE_VALUE {
            Err(error())
        } else {
            Ok(Self(h))
        }
    }
}
// Kernel handles can cross threads; ownership remains unique and Drop closes once.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub(super) fn process_session(pid: u32) -> Result<u32, String> {
    let mut session = 0;
    unsafe {
        if ProcessIdToSessionId(pid, &mut session) == 0 {
            return Err(error());
        }
    }
    Ok(session)
}

pub(super) fn same_session(pid: u32) -> bool {
    sessions_match(
        process_session(pid),
        process_session(unsafe { GetCurrentProcessId() }),
    )
}

fn sessions_match(peer: Result<u32, String>, own: Result<u32, String>) -> bool {
    match (peer, own) {
        (Ok(peer), Ok(own)) => peer == own,
        _ => false,
    }
}

pub(super) fn current_sid() -> Result<String, String> {
    unsafe { sid_for_process(GetCurrentProcess()) }
}
pub(super) fn process_sid(pid: u32) -> Result<String, String> {
    unsafe {
        let p = Handle::new(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid))?;
        sid_for_process(p.0)
    }
}
unsafe fn sid_for_process(process: HANDLE) -> Result<String, String> {
    let mut token = null_mut();
    if OpenProcessToken(process, TOKEN_QUERY, &mut token) == 0 {
        return Err(error());
    }
    let token = Handle(token);
    let mut length = 0;
    GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut length);
    if length == 0 || length > 65536 {
        return Err("invalid token size".into());
    }
    // usize storage provides the alignment required by TOKEN_USER.
    let mut buffer = vec![0_usize; (length as usize).div_ceil(std::mem::size_of::<usize>())];
    if GetTokenInformation(
        token.0,
        TokenUser,
        buffer.as_mut_ptr().cast(),
        length,
        &mut length,
    ) == 0
    {
        return Err(error());
    }
    let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
    let mut text = null_mut();
    if ConvertSidToStringSidW(user.User.Sid, &mut text) == 0 {
        return Err(error());
    }
    let mut len = 0;
    while *text.add(len) != 0 {
        len += 1;
    }
    let sid = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
    LocalFree(text.cast());
    Ok(sid)
}
pub(super) struct Security {
    descriptor: PSECURITY_DESCRIPTOR,
}
impl Security {
    pub fn for_user(sid: &str) -> Result<Self, String> {
        unsafe {
            let mut descriptor = null_mut();
            let sddl = wide(&format!("D:P(A;;GA;;;{sid})"));
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                null_mut(),
            ) == 0
            {
                return Err(error());
            }
            Ok(Self { descriptor })
        }
    }
    pub fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.descriptor,
            bInheritHandle: 0,
        }
    }
}
impl Drop for Security {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.descriptor);
        }
    }
}
pub(super) fn namespace(path: &Path, sid: &str) -> Result<String, String> {
    let path = std::path::absolute(path).map_err(|e| e.to_string())?;
    let text = path
        .to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase();
    let mut session = 0;
    unsafe {
        if ProcessIdToSessionId(GetCurrentProcessId(), &mut session) == 0 {
            return Err(error());
        }
    }
    Ok(format!(
        "{:x}",
        Sha256::digest(format!("{sid}|{session}|{text}"))
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn current_user_is_a_sid() {
        assert!(current_sid().unwrap().starts_with("S-1-"));
    }
    #[test]
    fn data_roots_have_distinct_namespaces() {
        let sid = current_sid().unwrap();
        assert_ne!(
            namespace(Path::new("C:/echo-one"), &sid).unwrap(),
            namespace(Path::new("C:/echo-two"), &sid).unwrap()
        );
    }
    #[test]
    fn namespace_normalizes_case_and_slashes() {
        let sid = current_sid().unwrap();
        assert_eq!(
            namespace(Path::new("C:/Echo/"), &sid).unwrap(),
            namespace(Path::new("c:\\echo"), &sid).unwrap()
        );
    }

    #[test]
    fn own_process_matches_its_interactive_session() {
        assert!(same_session(std::process::id()));
        assert!(process_session(std::process::id()).is_ok());
    }

    #[test]
    fn an_unqueryable_peer_is_rejected_instead_of_assuming_the_same_session() {
        assert!(!same_session(u32::MAX));
    }

    #[test]
    fn different_interactive_sessions_are_rejected() {
        assert!(!sessions_match(Ok(7), Ok(8)));
    }
}
