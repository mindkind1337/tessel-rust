//! On-demand SFTP over OpenSSH's verified subsystem transport. No shell parsing.
use crate::workspace::Host;
use russh_sftp::{
    client::{Config, RawSftpSession, error::Error},
    protocol::{FileAttributes, OpenFlags, Packet, StatusCode},
};
use std::{
    pin::Pin,
    process::Stdio,
    task::{Context, Poll},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
const MAX_FILE: usize = 16 * 1024 * 1024;
#[derive(Clone)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub directory: bool,
    pub size: u64,
}
pub enum Action {
    List(String),
    Read(String),
    Save {
        path: String,
        original: Vec<u8>,
        replacement: Vec<u8>,
    },
}
pub enum Reply {
    Listing {
        root: String,
        path: String,
        entries: Vec<Entry>,
    },
    File {
        root: String,
        path: String,
        bytes: Vec<u8>,
    },
    Saved,
}
struct Transport {
    reader: tokio::process::ChildStdout,
    writer: tokio::process::ChildStdin,
}
impl AsyncRead for Transport {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().reader).poll_read(cx, buf)
    }
}
impl AsyncWrite for Transport {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().writer).poll_write(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().writer).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().writer).poll_shutdown(cx)
    }
}
fn clean(path: &str) -> bool {
    path.len() <= 8192 && !path.chars().any(char::is_control) && !path.split('/').any(|p| p == "..")
}
fn within(root: &str, path: &str) -> bool {
    path == root
        || (path.starts_with(root)
            && (root == "/" || path.as_bytes().get(root.len()) == Some(&b'/')))
}
fn ssh_args(host: &Host) -> Result<Vec<String>, String> {
    crate::workspace::validate_host(&host.alias)?;
    if !clean(&host.folder) {
        return Err("Invalid remote folder".into());
    }
    Ok([
        "-T",
        "-s",
        "-o",
        "BatchMode=yes",
        "-o",
        "StrictHostKeyChecking=yes",
        "-o",
        "ConnectTimeout=10",
        "-o",
        "ServerAliveInterval=15",
        "-o",
        "ServerAliveCountMax=2",
        "-o",
        "ControlMaster=no",
        "-o",
        "ControlPath=none",
        "--",
        &host.alias,
        "sftp",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect())
}
pub fn execute(host: Host, action: Action) -> Result<Reply, String> {
    let args = ssh_args(&host)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "Cannot start SFTP runtime")?;
    runtime.block_on(async move {
        let mut command=tokio::process::Command::new("ssh");
        command.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
        #[cfg(windows)] command.creation_flags(0x08000000);
        let mut child=command.spawn().map_err(|_|"OpenSSH was not found or could not start")?;
        #[cfg(windows)] let _job=ProcessJob::attach(child.raw_handle().ok_or("SSH process handle unavailable")?)?;
        let stream=Transport{reader:child.stdout.take().ok_or("No SSH output")?,writer:child.stdin.take().ok_or("No SSH input")?};
        let result=tokio::time::timeout(Duration::from_secs(30),async {
            let session=RawSftpSession::new_with_config(stream,Config{max_concurrent_reads:1,max_concurrent_writes:1,..Default::default()});
            let version=session.init().await.map_err(|_|"SSH/SFTP connection failed. Confirm the host key and key-based login in a terminal first.")?;
            let root=canonical(&session,if host.folder.is_empty(){"."}else{&host.folder}).await?;
            let result=operate(&session,&root,action,version.extensions.contains_key("posix-rename@openssh.com")).await;
            let _=session.close_session();result
        }).await.map_err(|_|"SFTP operation exceeded 30 seconds".to_string()).and_then(|r|r);
        let _=child.kill().await;let _=child.wait().await;
        result
    })
}
fn remote_error(error: Error) -> String {
    format!("SFTP request failed: {error}")
}
async fn canonical(session: &RawSftpSession, path: &str) -> Result<String, String> {
    if !clean(path) {
        return Err("Invalid remote path".into());
    }
    let name = session.realpath(path).await.map_err(remote_error)?;
    let resolved = name
        .files
        .first()
        .ok_or("No remote path returned")?
        .filename
        .trim_end_matches('/')
        .to_owned();
    let resolved = if resolved.is_empty() {
        "/".to_string()
    } else {
        resolved
    };
    if !resolved.starts_with('/') || !clean(&resolved) {
        return Err("Server returned an invalid absolute path".into());
    }
    Ok(resolved)
}
async fn scoped(session: &RawSftpSession, root: &str, path: &str) -> Result<String, String> {
    let joined = if path.is_empty() {
        root.to_string()
    } else if path.starts_with('/') {
        path.into()
    } else {
        format!("{}/{path}", root.trim_end_matches('/'))
    };
    let path = canonical(session, &joined).await?;
    if !within(root, &path) {
        return Err("Remote path leaves the selected root".into());
    }
    Ok(path)
}
async fn read(session: &RawSftpSession, path: &str) -> Result<(Vec<u8>, FileAttributes), String> {
    let handle = session
        .open(path, OpenFlags::READ, FileAttributes::default())
        .await
        .map_err(remote_error)?
        .handle;
    let result = async {
        let attrs = session.fstat(&handle).await.map_err(remote_error)?.attrs;
        if attrs.file_type() != russh_sftp::protocol::FileType::File {
            return Err("Only regular remote files can be opened".into());
        }
        if attrs.size.is_some_and(|n| n > MAX_FILE as u64) {
            return Err("Remote download limit is 16 MiB".into());
        }
        let mut bytes = Vec::new();
        loop {
            match session.read(&handle, bytes.len() as u64, 32768).await {
                Ok(data) => {
                    if data.data.is_empty() {
                        return Err("Server returned an empty non-EOF read".into());
                    }
                    if bytes.len() + data.data.len() > MAX_FILE {
                        return Err("Remote download limit is 16 MiB".into());
                    }
                    bytes.extend_from_slice(&data.data);
                }
                Err(Error::Status(s)) if s.status_code == StatusCode::Eof => break,
                Err(e) => return Err(remote_error(e)),
            }
        }
        Ok((bytes, attrs))
    }
    .await;
    let closed = session.close(handle).await.map_err(remote_error);
    let value = result?;
    closed?;
    Ok(value)
}
async fn operate(
    session: &RawSftpSession,
    root: &str,
    action: Action,
    posix_rename: bool,
) -> Result<Reply, String> {
    match action {
        Action::List(requested) => {
            let path = scoped(session, root, &requested).await?;
            let handle = session.opendir(&path).await.map_err(remote_error)?.handle;
            let result = async {
                let mut entries = Vec::new();
                let mut total = 0;
                loop {
                    match session.readdir(&handle).await {
                        Ok(batch) => {
                            if batch.files.is_empty() {
                                return Err("Server returned an empty directory batch".into());
                            }
                            total += batch.files.len();
                            if total > 10000 {
                                return Err(
                                    "Directory exceeds 10000 entries; select a smaller root".into(),
                                );
                            }
                            for entry in batch.files {
                                let name = entry.filename;
                                if name == "." || name == ".." {
                                    continue;
                                }
                                if !clean(&name) || name.contains('/') || entry.attrs.is_symlink() {
                                    continue;
                                }
                                let directory =
                                    entry.attrs.file_type() == russh_sftp::protocol::FileType::Dir;
                                if !directory
                                    && entry.attrs.file_type()
                                        != russh_sftp::protocol::FileType::File
                                {
                                    continue;
                                }
                                entries.push(Entry {
                                    path: format!("{}/{name}", path.trim_end_matches('/')),
                                    name,
                                    directory,
                                    size: entry.attrs.size.unwrap_or(0),
                                });
                            }
                        }
                        Err(Error::Status(s)) if s.status_code == StatusCode::Eof => break,
                        Err(e) => return Err(remote_error(e)),
                    }
                }
                entries.sort_by(|a, b| {
                    b.directory
                        .cmp(&a.directory)
                        .then_with(|| a.name.cmp(&b.name))
                });
                Ok(Reply::Listing {
                    root: root.into(),
                    path: path.clone(),
                    entries,
                })
            }
            .await;
            let closed = session.close(handle).await.map_err(remote_error);
            let value = result?;
            closed?;
            Ok(value)
        }
        Action::Read(requested) => {
            let path = scoped(session, root, &requested).await?;
            let (bytes, _) = read(session, &path).await?;
            Ok(Reply::File {
                root: root.into(),
                path,
                bytes,
            })
        }
        Action::Save {
            path,
            original,
            replacement,
        } => {
            if replacement.len() > 2 * 1024 * 1024 || original.len() > 2 * 1024 * 1024 {
                return Err("Remote editing limit is 2 MiB".into());
            }
            if !posix_rename {
                return Err("This server does not support atomic saves; original kept".into());
            }
            let resolved = scoped(session, root, &path).await?;
            if resolved != path {
                return Err("Remote file target changed; original kept".into());
            }
            let (current, attrs) = read(session, &path).await?;
            if current != original {
                return Err("Remote file changed since opening; save refused".into());
            }
            let parent = path.rsplit_once('/').ok_or("Invalid remote file")?.0;
            let mut random = [0u8; 16];
            getrandom::fill(&mut random).map_err(|_| "Cannot create unique save name")?;
            let token = random
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            let temp = format!("{parent}/.tessel-save-{token}");
            let handle = session
                .open(
                    &temp,
                    OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
                    FileAttributes {
                        permissions: Some(0o600),
                        ..Default::default()
                    },
                )
                .await
                .map_err(remote_error)?
                .handle;
            let result = async {
                for (index, chunk) in replacement.chunks(32768).enumerate() {
                    session
                        .write(&handle, (index * 32768) as u64, chunk.to_vec())
                        .await
                        .map_err(remote_error)?;
                }
                session
                    .fsetstat(
                        &handle,
                        FileAttributes {
                            uid: attrs.uid,
                            gid: attrs.gid,
                            permissions: attrs.permissions.map(|p| p & 0o777),
                            ..Default::default()
                        },
                    )
                    .await
                    .map_err(remote_error)?;
                session.close(&handle).await.map_err(remote_error)?;
                if scoped(session, root, &path).await? != path
                    || read(session, &path).await?.0 != original
                {
                    return Err("Remote file changed during save; original kept".into());
                }
                let mut data = Vec::new();
                for value in [&temp, &path] {
                    data.extend_from_slice(&(value.len() as u32).to_be_bytes());
                    data.extend_from_slice(value.as_bytes());
                }
                match session
                    .extended("posix-rename@openssh.com", data)
                    .await
                    .map_err(remote_error)?
                {
                    Packet::Status(s) if s.status_code == StatusCode::Ok => Ok(Reply::Saved),
                    _ => Err("Server did not confirm atomic save".into()),
                }
            }
            .await;
            if result.is_err() {
                let _ = session.close(handle).await;
                let _ = session.remove(temp).await;
            }
            result
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_paths_are_component_scoped() {
        assert!(within("/work", "/work/a"));
        assert!(!within("/work", "/workspace/private"));
        assert!(!clean("/work/../secret"));
        assert!(!clean("a\ncommand"));
        assert!(within("/", "/work"));
    }
    #[test]
    fn transport_never_accepts_options_as_a_host() {
        assert!(
            ssh_args(&Host {
                alias: "-oProxyCommand=bad".into(),
                folder: "".into()
            })
            .is_err()
        );
        let args = ssh_args(&Host {
            alias: "dev".into(),
            folder: "/work".into(),
        })
        .unwrap();
        assert!(args.contains(&"StrictHostKeyChecking=yes".into()));
        assert!(args.contains(&"BatchMode=yes".into()));
        assert_eq!(&args[args.len() - 2..], ["dev", "sftp"]);
    }
}

#[cfg(windows)]
struct ProcessJob(windows::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl ProcessJob {
    fn attach(process: std::os::windows::io::RawHandle) -> Result<Self, String> {
        use windows::{
            Win32::{Foundation::HANDLE, System::JobObjects::*},
            core::PCWSTR,
        };
        unsafe {
            let handle = CreateJobObjectW(None, PCWSTR::null())
                .map_err(|_| "Cannot create SSH process group")?;
            let job = Self(handle);
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
            .map_err(|_| "Cannot configure SSH cleanup")?;
            AssignProcessToJobObject(handle, HANDLE(process))
                .map_err(|_| "Cannot attach SSH process cleanup")?;
            Ok(job)
        }
    }
}
#[cfg(windows)]
impl Drop for ProcessJob {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod protocol_tests;
