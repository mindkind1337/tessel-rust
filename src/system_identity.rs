//! Local account/hostname metadata used by OpenSSH Include tokens. No credentials.
#[derive(Default)]
pub(crate) struct Identity {
    pub username: String,
    pub hostname: String,
    pub uid: Option<String>,
}

impl Identity {
    pub fn detect() -> Self {
        let mut identity = platform_identity();
        if identity.username.is_empty() {
            identity.username = std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .unwrap_or_default();
        }
        identity
    }
}

#[cfg(windows)]
fn platform_identity() -> Identity {
    use windows::{
        Win32::{
            Networking::WinSock::{GetHostNameW, WSACleanup, WSADATA, WSAStartup},
            System::WindowsProgramming::GetUserNameW,
        },
        core::PWSTR,
    };
    fn text(buffer: &[u16]) -> Option<String> {
        let end = buffer.iter().position(|c| *c == 0)?;
        String::from_utf16(&buffer[..end]).ok()
    }
    let mut identity = Identity::default();
    let mut username = [0_u16; 257];
    let mut size = username.len() as u32;
    // SAFETY: The writable buffer and supplied length agree and outlive the call.
    if unsafe { GetUserNameW(Some(PWSTR(username.as_mut_ptr())), &mut size) }.is_ok() {
        identity.username = text(&username).unwrap_or_default();
    }
    let mut data = WSADATA::default();
    // SAFETY: Valid out pointer; each successful startup is balanced by cleanup.
    if unsafe { WSAStartup(0x0202, &mut data) } == 0 {
        let mut hostname = [0_u16; 256];
        // SAFETY: Winsock is initialized and the API receives a bounded writable slice.
        let result = unsafe { GetHostNameW(&mut hostname) };
        // SAFETY: Releases only this function's WSAStartup reference, not other users'.
        unsafe { WSACleanup() };
        if result == 0 {
            identity.hostname = text(&hostname).unwrap_or_default();
        }
    }
    // Node's Windows os.userInfo reports uid -1; it is unavailable to Include %i.
    identity
}

#[cfg(unix)]
fn platform_identity() -> Identity {
    use std::{ffi::CStr, ptr};
    let mut identity = Identity::default();
    let mut hostname = [0_u8; 1024];
    // SAFETY: The buffer is writable for its advertised size.
    if unsafe { libc::gethostname(hostname.as_mut_ptr().cast(), hostname.len()) } == 0 {
        hostname[hostname.len() - 1] = 0;
        let end = hostname.iter().position(|b| *b == 0).unwrap();
        identity.hostname = String::from_utf8_lossy(&hostname[..end]).into_owned();
    }
    // SAFETY: geteuid has no pointer arguments or preconditions.
    let uid = unsafe { libc::geteuid() };
    let mut capacity = 4096;
    while capacity <= 1024 * 1024 {
        let mut buffer = vec![0_u8; capacity];
        // SAFETY: passwd is a C record of scalar/pointer fields, initially all null/zero.
        let mut record: libc::passwd = unsafe { std::mem::zeroed() };
        let mut result = ptr::null_mut();
        // SAFETY: All writable pointers and the backing buffer stay valid through the call.
        let status = unsafe {
            libc::getpwuid_r(
                uid,
                &mut record,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };
        if status == libc::ERANGE {
            capacity *= 2;
            continue;
        }
        if status == 0 && !result.is_null() && !record.pw_name.is_null() {
            identity.uid = Some(record.pw_uid.to_string());
            // SAFETY: Successful getpwuid_r supplies a NUL-terminated name in the live buffer.
            identity.username = unsafe { CStr::from_ptr(record.pw_name) }
                .to_string_lossy()
                .into_owned();
        }
        break;
    }
    identity
}

#[cfg(not(any(windows, unix)))]
fn platform_identity() -> Identity {
    Identity::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "Requires the Node reference runtime; executed explicitly during native acceptance"]
    fn native_identity_matches_node_os_with_spoofed_environment() {
        let mut command = std::process::Command::new("node");
        command.args(["-e", "const os=require('node:os');const u=os.userInfo();process.stdout.write(JSON.stringify({username:u.username,hostname:os.hostname(),uid:u.uid}));"])
            .env("USERNAME", "synthetic-wrong-user")
            .env("USER", "synthetic-wrong-user")
            .env("COMPUTERNAME", "synthetic-wrong-host")
            .env("HOSTNAME", "synthetic-wrong-host");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let output = command
            .output()
            .expect("Node reference runtime must be installed");
        assert!(output.status.success(), "Node OS reference failed");
        let reference: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("Invalid reference metadata");
        let identity = Identity::detect();
        assert!(
            Some(identity.username.as_str()) == reference["username"].as_str(),
            "Native username differs from reference"
        );
        assert!(
            Some(identity.hostname.as_str()) == reference["hostname"].as_str(),
            "Native hostname differs from reference"
        );
        let uid = reference["uid"]
            .as_i64()
            .filter(|value| *value >= 0)
            .map(|value| value.to_string());
        assert!(identity.uid == uid, "Native UID differs from reference");
    }
}
