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
const OPERATION_TIMEOUT: Duration = Duration::from_secs(300);
#[cfg(windows)]
pub(crate) const MAX_PREVIEW_BYTES: usize = 50 * 1024 * 1024;
const MAX_FILE: usize = 16 * 1024 * 1024;
#[derive(Clone)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub directory: bool,
    pub size: u64,
}
pub enum Action {
    Delete {
        path: String,
        directory: bool,
    },
    Rename {
        path: String,
        name: String,
    },
    Create {
        parent: String,
        name: String,
        directory: bool,
    },
    List(String),
    Read(String),
    #[cfg(windows)]
    PreviewAsset {
        expected_root: String,
        path: String,
    },
    Save {
        path: String,
        original: Vec<u8>,
        replacement: Vec<u8>,
    },
}
pub enum Reply {
    Deleted {
        path: String,
        parent: String,
    },
    Renamed {
        old: String,
        new: String,
        parent: String,
    },
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
    #[cfg(windows)]
    PreviewTooLarge,
    Saved,
    Created {
        parent: String,
    },
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
fn child_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_control)
}
fn within(root: &str, path: &str) -> bool {
    path == root
        || (path.starts_with(root)
            && (root == "/" || path.as_bytes().get(root.len()) == Some(&b'/')))
}
fn ssh_args(host: &Host) -> Result<Vec<String>, String> {
    transport_args(host, true)
}
fn transport_args(host: &Host, sftp: bool) -> Result<Vec<String>, String> {
    crate::workspace::validate_host(&host.alias)?;
    if !clean(&host.folder) {
        return Err("Invalid remote folder".into());
    }
    let mut args = vec!["-T"];
    if sftp {
        args.push("-s");
    }
    args.extend([
        "-o",
        "BatchMode=yes",
        "-o",
        "StrictHostKeyChecking=yes",
        "-o",
        crate::ssh_config::CONNECT_TIMEOUT,
        "-o",
        crate::ssh_config::KEEPALIVE_INTERVAL,
        "-o",
        crate::ssh_config::KEEPALIVE_COUNT,
        "-o",
        "ControlMaster=no",
        "-o",
        "ControlPath=none",
        "--",
        &host.alias,
        if sftp { "sftp" } else { "sh -s" },
    ]);
    Ok(args.into_iter().map(str::to_owned).collect())
}
#[path = "remote/file_history.rs"]
pub(crate) mod file_history;
pub fn execute(host: Host, action: Action) -> Result<Reply, String> {
    let args = ssh_args(&host)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "Cannot start SFTP runtime")?;
    runtime.block_on(async move {
        let mut command=tokio::process::Command::new("ssh");
        crate::agent_env::clean_command(command.as_std_mut());
        command.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
        #[cfg(windows)] command.creation_flags(0x08000000);
        let mut child=command.spawn().map_err(|_|"OpenSSH was not found or could not start")?;
        #[cfg(windows)] let _job=ProcessJob::attach(child.raw_handle().ok_or("SSH process handle unavailable")?)?;
        let stream=Transport{reader:child.stdout.take().ok_or("No SSH output")?,writer:child.stdin.take().ok_or("No SSH input")?};
        let session=RawSftpSession::new_with_config(stream,Config{max_concurrent_reads:1,max_concurrent_writes:1,..Default::default()});
        let setup=tokio::time::timeout(Duration::from_secs(30),async {
            let version=session.init().await.map_err(|_|"SSH/SFTP connection failed. Confirm the host key and key-based login in a terminal first.")?;
            let root=canonical_project_root(&session,&host.folder).await?;
            Ok::<_,String>((root,version.extensions.contains_key("posix-rename@openssh.com")))
        }).await.map_err(|_|"SFTP connection setup exceeded 30 seconds".to_string()).and_then(|r|r);
        let result=match setup {
            Ok((root,rename))=>operate(&session,&root,action,rename).await,
            Err(error)=>Err(error),
        };
        let _=session.close_session();
        let _=child.kill().await;let _=child.wait().await;
        result
    })
}
fn remote_error(error: Error) -> String {
    format!("SFTP request failed: {error}")
}
// SFTP REALPATH takes a filesystem path. The initial directory corresponds
// to the SSH login directory and the server does not expand shell syntax.
async fn canonical_project_root(session: &RawSftpSession, folder: &str) -> Result<String, String> {
    match folder {
        "" | "~" | "~/" => canonical(session, ".").await,
        _ => match folder.strip_prefix("~/") {
            Some(relative) => canonical(session, &format!("./{relative}")).await,
            None => canonical(session, folder).await,
        },
    }
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
#[cfg(windows)]
pub(crate) fn preview_relative<'a>(root: &str, path: &'a str) -> Option<&'a str> {
    let rel = path
        .strip_prefix(root.trim_end_matches('/'))?
        .strip_prefix('/')?;
    (root.starts_with('/')
        && rel.split('/').all(|s| {
            !s.is_empty()
                && !s.starts_with('.')
                && !s.contains('\\')
                && !s.chars().any(char::is_control)
        }))
    .then_some(rel)
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
    let extension = std::path::Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let limit = if crate::files::playback::mime(&extension).is_some() {
        crate::files::playback::MAX_BYTES
    } else {
        MAX_FILE
    };
    read_bounded(session, path, limit)
        .await?
        .ok_or_else(|| format!("Remote download limit is {} MiB", limit / (1024 * 1024)))
}
async fn read_bounded(
    session: &RawSftpSession,
    path: &str,
    limit: usize,
) -> Result<Option<(Vec<u8>, FileAttributes)>, String> {
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
        if attrs.size.is_some_and(|n| n > limit as u64) {
            return Ok(None);
        }
        let mut bytes = Vec::new();
        loop {
            match session.read(&handle, bytes.len() as u64, 32768).await {
                Ok(data) => {
                    if data.data.is_empty() {
                        return Err("Server returned an empty non-EOF read".into());
                    }
                    if bytes.len() + data.data.len() > limit {
                        return Ok(None);
                    }
                    bytes.extend_from_slice(&data.data);
                }
                Err(Error::Status(s)) if s.status_code == StatusCode::Eof => break,
                Err(e) => return Err(remote_error(e)),
            }
        }
        Ok(Some((bytes, attrs)))
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
    operate_with_timeout(session, root, action, posix_rename, OPERATION_TIMEOUT).await
}

async fn operate_with_timeout(
    session: &RawSftpSession,
    root: &str,
    action: Action,
    posix_rename: bool,
    deadline: Duration,
) -> Result<Reply, String> {
    // Kept outside the cancellable future: a deadline must not skip save cleanup.
    let pending = std::cell::RefCell::new(None);
    let mut result = tokio::time::timeout(
        deadline,
        operate_inner(session, root, action, posix_rename, &pending),
    )
    .await
    .unwrap_or_else(|_| {
        Err("SFTP operation deadline exceeded; check the remote file before retrying a save".into())
    });
    if let Some((path, handle)) = pending.into_inner()
        && result.is_err()
    {
        let cleaned = tokio::time::timeout(Duration::from_secs(5), async {
            let _ = session.close(handle).await;
            session.remove(path).await
        })
        .await;
        if !matches!(cleaned, Ok(Ok(_)))
            && let Err(error) = &mut result
        {
            error.push_str("; remote temporary-file cleanup could not be confirmed");
        }
    }
    result
}

async fn operate_inner(
    session: &RawSftpSession,
    root: &str,
    action: Action,
    posix_rename: bool,
    pending: &std::cell::RefCell<Option<(String, String)>>,
) -> Result<Reply, String> {
    match action {
        Action::Delete { path, directory } => {
            let resolved = scoped(session, root, &path).await?;
            if resolved != path || resolved == root {
                return Err("Cannot delete the remote root or a changed target".into());
            }
            let attrs = session.lstat(&path).await.map_err(remote_error)?.attrs;
            let expected = if directory {
                russh_sftp::protocol::FileType::Dir
            } else {
                russh_sftp::protocol::FileType::File
            };
            if attrs.file_type() != expected {
                return Err("Remote item type changed; deletion refused".into());
            }
            if directory {
                session.rmdir(&path).await.map_err(remote_error)?;
            } else {
                session.remove(&path).await.map_err(remote_error)?;
            }
            let parent = path.rsplit_once('/').ok_or("Invalid remote path")?.0;
            Ok(Reply::Deleted {
                path: path.clone(),
                parent: if parent.is_empty() {
                    "/".into()
                } else {
                    parent.into()
                },
            })
        }
        Action::Rename { path, name } => {
            if !child_name(&name) {
                return Err("Enter one file or folder name, without path separators".into());
            }
            let resolved = scoped(session, root, &path).await?;
            if resolved != path || resolved == root {
                return Err("Cannot rename the remote root or a changed target".into());
            }
            let parent = path.rsplit_once('/').ok_or("Invalid remote path")?.0;
            let parent = if parent.is_empty() { "/" } else { parent };
            let new = format!("{}/{name}", parent.trim_end_matches('/'));
            // SFTP v3 RENAME refuses an existing destination; never use posix-rename here.
            match session.lstat(&new).await {
                Err(Error::Status(s)) if s.status_code == StatusCode::NoSuchFile => {}
                Ok(_) => return Err("Destination already exists; rename refused".into()),
                Err(e) => return Err(remote_error(e)),
            }
            session.rename(&path, &new).await.map_err(remote_error)?;
            Ok(Reply::Renamed {
                old: path.clone(),
                new,
                parent: parent.into(),
            })
        }
        Action::Create {
            parent,
            name,
            directory,
        } => {
            if !child_name(&name) {
                return Err("Enter one file or folder name, without path separators".into());
            }
            let parent = scoped(session, root, &parent).await?;
            let path = format!("{}/{name}", parent.trim_end_matches('/'));
            if directory {
                session
                    .mkdir(
                        &path,
                        FileAttributes {
                            permissions: Some(0o700),
                            ..Default::default()
                        },
                    )
                    .await
                    .map_err(remote_error)?;
            } else {
                let handle = session
                    .open(
                        &path,
                        OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
                        FileAttributes {
                            permissions: Some(0o600),
                            ..Default::default()
                        },
                    )
                    .await
                    .map_err(remote_error)?
                    .handle;
                session.close(handle).await.map_err(remote_error)?;
            }
            Ok(Reply::Created { parent })
        }
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
        #[cfg(windows)]
        Action::PreviewAsset {
            expected_root,
            path: requested,
        } => {
            if expected_root != root || preview_relative(root, &requested).is_none() {
                return Err("Invalid preview path".into());
            }
            let mut path = scoped(session, root, &requested).await?;
            if preview_relative(root, &path).is_none() {
                return Err("Invalid preview target".into());
            }
            if session
                .stat(&path)
                .await
                .map_err(remote_error)?
                .attrs
                .file_type()
                == russh_sftp::protocol::FileType::Dir
            {
                path = scoped(session, root, &format!("{path}/index.html")).await?;
                if preview_relative(root, &path).is_none() {
                    return Err("Invalid preview index".into());
                }
            }
            let Some((bytes, _)) = read_bounded(session, &path, MAX_PREVIEW_BYTES).await? else {
                return Ok(Reply::PreviewTooLarge);
            };
            Ok(Reply::File {
                root: root.into(),
                path,
                bytes,
            })
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
            *pending.borrow_mut() = Some((temp.clone(), handle.clone()));
            async {
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
            .await
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    #[ignore = "requires TESSEL_TEST_SFTP_SERVER pointing to an OpenSSH sftp-server binary"]
    fn real_sftp_subsystem_preserves_files_conflicts_and_preview_scope() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("pages")).unwrap();
        std::fs::write(dir.path().join("pages/index.html"), "page").unwrap();
        std::fs::write(dir.path().join("site.css"), "body{}").unwrap();
        std::fs::write(dir.path().join(".hidden"), "hidden").unwrap();
        for (name, size) in [
            ("asset.bin", MAX_FILE + 1),
            ("large.bin", MAX_PREVIEW_BYTES + 1),
        ] {
            std::fs::File::create(dir.path().join(name))
                .unwrap()
                .set_len(size as u64)
                .unwrap();
        }
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let mut child = tokio::process::Command::new(
                    std::env::var_os("TESSEL_TEST_SFTP_SERVER").unwrap(),
                )
                .current_dir(dir.path())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .creation_flags(0x08000000)
                .kill_on_drop(true)
                .spawn()
                .unwrap();
                let stream = Transport {
                    reader: child.stdout.take().unwrap(),
                    writer: child.stdin.take().unwrap(),
                };
                let session = RawSftpSession::new(stream);
                let version = session.init().await.unwrap();
                let root = canonical(&session, ".").await.unwrap();
                for folder in ["", "~", "~/"] {
                    assert_eq!(
                        canonical_project_root(&session, folder).await.unwrap(),
                        root
                    );
                }
                let nested = canonical_project_root(&session, "~/pages").await.unwrap();
                assert_eq!(nested, format!("{root}/pages"));
                let Reply::File { bytes, .. } =
                    operate(&session, &nested, Action::Read("index.html".into()), false)
                        .await
                        .unwrap()
                else {
                    panic!("expected the file in the home-relative project");
                };
                assert_eq!(bytes, b"page");
                assert!(
                    canonical_project_root(&session, "~/../outside")
                        .await
                        .is_err()
                );
                let atomic = version.extensions.contains_key("posix-rename@openssh.com");
                let run = |action| operate(&session, &root, action, atomic);
                let path = format!("{root}/notes.txt");
                let create = || Action::Create {
                    parent: root.clone(),
                    name: "notes.txt".into(),
                    directory: false,
                };
                assert!(matches!(
                    run(create()).await.unwrap(),
                    Reply::Created { .. }
                ));
                let payload = vec![b'x'; 70001];
                let save = |original: &[u8], replacement: &[u8]| Action::Save {
                    path: path.clone(),
                    original: original.to_vec(),
                    replacement: replacement.to_vec(),
                };
                assert!(atomic, "OpenSSH fixture must advertise atomic rename");
                assert!(matches!(
                    run(save(b"", &payload)).await.unwrap(),
                    Reply::Saved
                ));
                assert!(run(create()).await.is_err());
                assert!(run(save(b"", b"stale edit")).await.is_err());
                let Reply::File { bytes, .. } = run(Action::Read(path.clone())).await.unwrap()
                else {
                    panic!("remote read did not return bytes");
                };
                assert_eq!(bytes, payload);
                let rename = |name: &str| Action::Rename {
                    path: path.clone(),
                    name: name.into(),
                };
                assert!(run(rename("site.css")).await.is_err());
                let Reply::Renamed { new, .. } = run(rename("final.txt")).await.unwrap() else {
                    panic!("remote rename did not report its destination");
                };
                assert_eq!(
                    std::fs::read(dir.path().join("final.txt")).unwrap(),
                    payload
                );
                assert_eq!(
                    std::fs::read(dir.path().join("site.css")).unwrap(),
                    b"body{}"
                );
                assert!(!dir.path().join("notes.txt").exists());
                let Reply::Listing { entries, .. } = run(Action::List(root.clone())).await.unwrap()
                else {
                    panic!("remote list did not return entries");
                };
                assert!(entries.iter().any(|entry| entry.path == new));
                assert!(
                    !entries
                        .iter()
                        .any(|entry| entry.name.starts_with(".tessel-save-"))
                );
                assert!(matches!(
                    run(Action::Delete {
                        path: new,
                        directory: false
                    })
                    .await
                    .unwrap(),
                    Reply::Deleted { .. }
                ));
                assert!(!dir.path().join("final.txt").exists());
                for (path, expected) in [
                    ("site.css", Some("body{}")),
                    ("pages", Some("page")),
                    (".hidden", None),
                    ("../outside", None),
                ] {
                    let result = operate(
                        &session,
                        &root,
                        Action::PreviewAsset {
                            expected_root: root.clone(),
                            path: format!("{root}/{path}"),
                        },
                        false,
                    )
                    .await;
                    if let Some(text) = expected {
                        let Reply::File { bytes, .. } = result.unwrap() else {
                            panic!("expected file")
                        };
                        assert_eq!(bytes, text.as_bytes());
                    } else {
                        assert!(result.is_err());
                    }
                }
                assert!(
                    operate(
                        &session,
                        &root,
                        Action::PreviewAsset {
                            expected_root: "/other".into(),
                            path: format!("{root}/site.css")
                        },
                        false
                    )
                    .await
                    .is_err()
                );
                for (name, large) in [("asset.bin", false), ("large.bin", true)] {
                    let path = format!("{root}/{name}");
                    assert!(read(&session, &path).await.is_err());
                    let result = operate(
                        &session,
                        &root,
                        Action::PreviewAsset {
                            expected_root: root.clone(),
                            path,
                        },
                        false,
                    )
                    .await
                    .unwrap();
                    match result {
                        Reply::PreviewTooLarge => assert!(large),
                        Reply::File { bytes, .. } => {
                            assert!(!large);
                            assert_eq!(bytes.len(), MAX_FILE + 1);
                        }
                        _ => panic!("unexpected preview response"),
                    }
                }
                let _ = session.close_session();
                let _ = child.kill().await;
                let _ = child.wait().await;
            });
    }
    #[test]
    fn remote_paths_are_component_scoped() {
        assert!(child_name("résumé.txt"));
        for bad in ["", ".", "..", "../outside", "a/b", "a\\b", "a\n"] {
            assert!(!child_name(bad));
        }
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
                folder: "".into(),
                ..Default::default()
            })
            .is_err()
        );
        let args = ssh_args(&Host {
            alias: "dev".into(),
            folder: "/work".into(),
            ..Default::default()
        })
        .unwrap();
        assert!(args.contains(&"ConnectTimeout=30".into()));
        assert!(args.contains(&"ServerAliveInterval=15".into()));
        assert!(args.contains(&"ServerAliveCountMax=12".into()));
        assert!(args.contains(&"StrictHostKeyChecking=yes".into()));
        assert!(args.contains(&"BatchMode=yes".into()));
        assert_eq!(&args[args.len() - 2..], ["dev", "sftp"]);
    }
}

#[cfg(windows)]
pub(crate) use crate::process_job::ProcessJob;

#[cfg(test)]
mod protocol_tests;
