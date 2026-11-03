use super::*;
use russh_sftp::{
    protocol::{Attrs, Data, File, Handle, Name, Status, Version},
    server::Handler,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
#[derive(Clone)]
struct Record {
    bytes: Vec<u8>,
    attrs: FileAttributes,
}
struct Fixture {
    files: Arc<Mutex<HashMap<String, Record>>>,
    listed: bool,
    read_delay: Duration,
    write_delay: Duration,
    remove_delay: Duration,
}
fn status(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: String::new(),
        language_tag: String::new(),
    }
}
impl Handler for Fixture {
    type Error = StatusCode;
    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }
    async fn init(&mut self, _: u32, _: HashMap<String, String>) -> Result<Version, Self::Error> {
        let mut v = Version::new();
        v.extensions
            .insert("posix-rename@openssh.com".into(), "1".into());
        Ok(v)
    }
    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        Ok(Name {
            id,
            files: vec![File::dummy(match path.as_str() {
                "." => "/root",
                "/root/escape" => "/outside/private",
                _ => &path,
            })],
        })
    }
    async fn opendir(&mut self, id: u32, _: String) -> Result<Handle, Self::Error> {
        self.listed = false;
        Ok(Handle {
            id,
            handle: "directory".into(),
        })
    }
    async fn readdir(&mut self, id: u32, _: String) -> Result<Name, Self::Error> {
        if self.listed {
            return Err(StatusCode::Eof);
        }
        self.listed = true;
        let files = self.files.lock().unwrap();
        let mut names: Vec<_> = files
            .iter()
            .filter(|(p, _)| p.starts_with("/root/"))
            .map(|(p, r)| File::new(p.trim_start_matches("/root/"), r.attrs.clone()))
            .collect();
        names.push(File::new(
            "../escape",
            FileAttributes {
                permissions: Some(0o100600),
                ..Default::default()
            },
        ));
        Ok(Name { id, files: names })
    }
    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, Self::Error> {
        let mut files = self.files.lock().unwrap();
        if files.keys().any(|p| p.starts_with(&(path.clone() + "/"))) {
            return Err(StatusCode::Failure);
        }
        files.remove(&path).ok_or(StatusCode::NoSuchFile)?;
        Ok(status(id))
    }
    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let files = self.files.lock().unwrap();
        Ok(Attrs {
            id,
            attrs: files
                .get(&path)
                .ok_or(StatusCode::NoSuchFile)?
                .attrs
                .clone(),
        })
    }
    async fn rename(
        &mut self,
        id: u32,
        oldpath: String,
        newpath: String,
    ) -> Result<Status, Self::Error> {
        let mut files = self.files.lock().unwrap();
        if files.contains_key(&newpath) {
            return Err(StatusCode::Failure);
        }
        let file = files.remove(&oldpath).ok_or(StatusCode::NoSuchFile)?;
        files.insert(newpath, file);
        Ok(status(id))
    }
    async fn mkdir(
        &mut self,
        id: u32,
        path: String,
        attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        let mut files = self.files.lock().unwrap();
        if files.contains_key(&path) {
            return Err(StatusCode::Failure);
        }
        files.insert(
            path,
            Record {
                bytes: vec![],
                attrs: FileAttributes {
                    permissions: Some(0o40000 | attrs.permissions.unwrap_or(0o700)),
                    ..attrs
                },
            },
        );
        Ok(status(id))
    }
    async fn open(
        &mut self,
        id: u32,
        filename: String,
        flags: OpenFlags,
        attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        let mut files = self.files.lock().unwrap();
        if flags.contains(OpenFlags::CREATE) {
            if files.contains_key(&filename) && flags.contains(OpenFlags::EXCLUDE) {
                return Err(StatusCode::Failure);
            }
            files.insert(
                filename.clone(),
                Record {
                    bytes: vec![],
                    attrs: FileAttributes {
                        permissions: Some(0o100000 | attrs.permissions.unwrap_or(0o600)),
                        ..attrs
                    },
                },
            );
        }
        if !files.contains_key(&filename) {
            return Err(StatusCode::NoSuchFile);
        }
        Ok(Handle {
            id,
            handle: filename,
        })
    }
    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        let files = self.files.lock().unwrap();
        let file = files.get(&handle).ok_or(StatusCode::NoSuchFile)?;
        let mut attrs = file.attrs.clone();
        attrs.size = Some(file.bytes.len() as u64);
        Ok(Attrs { id, attrs })
    }
    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        tokio::time::sleep(self.read_delay).await;
        let files = self.files.lock().unwrap();
        let bytes = &files.get(&handle).ok_or(StatusCode::NoSuchFile)?.bytes;
        let offset = offset as usize;
        if offset >= bytes.len() {
            return Err(StatusCode::Eof);
        }
        Ok(Data {
            id,
            data: bytes[offset..(offset + len as usize).min(bytes.len())].to_vec(),
        })
    }
    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        tokio::time::sleep(self.write_delay).await;
        let mut files = self.files.lock().unwrap();
        let bytes = &mut files.get_mut(&handle).ok_or(StatusCode::NoSuchFile)?.bytes;
        let offset = offset as usize;
        bytes.resize(bytes.len().max(offset + data.len()), 0);
        bytes[offset..offset + data.len()].copy_from_slice(&data);
        Ok(status(id))
    }
    async fn fsetstat(
        &mut self,
        id: u32,
        handle: String,
        attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        self.files
            .lock()
            .unwrap()
            .get_mut(&handle)
            .ok_or(StatusCode::NoSuchFile)?
            .attrs = FileAttributes {
            permissions: Some(0o100000 | attrs.permissions.unwrap_or(0o600)),
            ..attrs
        };
        Ok(status(id))
    }
    async fn close(&mut self, id: u32, _: String) -> Result<Status, Self::Error> {
        Ok(status(id))
    }
    async fn remove(&mut self, id: u32, path: String) -> Result<Status, Self::Error> {
        tokio::time::sleep(self.remove_delay).await;
        self.files.lock().unwrap().remove(&path);
        Ok(status(id))
    }
    async fn extended(
        &mut self,
        id: u32,
        request: String,
        data: Vec<u8>,
    ) -> Result<Packet, Self::Error> {
        if request != "posix-rename@openssh.com" {
            return Err(StatusCode::OpUnsupported);
        }
        let first = u32::from_be_bytes(data[..4].try_into().unwrap()) as usize;
        let old = String::from_utf8(data[4..4 + first].to_vec()).unwrap();
        let next = 4 + first;
        let len = u32::from_be_bytes(data[next..next + 4].try_into().unwrap()) as usize;
        let new = String::from_utf8(data[next + 4..next + 4 + len].to_vec()).unwrap();
        let mut files = self.files.lock().unwrap();
        let file = files.remove(&old).ok_or(StatusCode::NoSuchFile)?;
        files.insert(new, file);
        Ok(Packet::Status(status(id)))
    }
}
#[test]
fn real_sftp_frames_preserve_originals_and_refuse_conflicts() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let files = Arc::new(Mutex::new(HashMap::from([(
                "/root/file.txt".into(),
                Record {
                    bytes: b"original".to_vec(),
                    attrs: FileAttributes {
                        permissions: Some(0o100640),
                        uid: Some(7),
                        gid: Some(8),
                        ..Default::default()
                    },
                },
            )])));
            let (client, server) = tokio::io::duplex(65536);
            let server = tokio::spawn(russh_sftp::server::run(
                server,
                Fixture {
                    files: files.clone(),
                    listed: false,
                    read_delay: Duration::ZERO,
                    write_delay: Duration::ZERO,
                    remove_delay: Duration::ZERO,
                },
            ));
            let session = RawSftpSession::new(client);
            session.init().await.unwrap();
            let root = canonical(&session, ".").await.unwrap();
            assert_eq!(root, "/root");
            let Reply::Listing { entries, .. } =
                operate(&session, &root, Action::List(String::new()), true)
                    .await
                    .unwrap()
            else {
                panic!("expected listing")
            };
            assert_eq!(entries.len(), 1);
            let Reply::File { bytes, .. } =
                operate(&session, &root, Action::Read("file.txt".into()), true)
                    .await
                    .unwrap()
            else {
                panic!("expected file")
            };
            assert_eq!(bytes, b"original");
            assert!(
                operate(&session, &root, Action::Read("escape".into()), true)
                    .await
                    .is_err()
            );
            let save = || Action::Save {
                path: "/root/file.txt".into(),
                original: b"original".to_vec(),
                replacement: b"updated".to_vec(),
            };
            assert!(operate(&session, &root, save(), false).await.is_err());
            operate(&session, &root, save(), true).await.unwrap();
            {
                let files = files.lock().unwrap();
                let file = &files["/root/file.txt"];
                assert_eq!(file.bytes, b"updated");
                assert_eq!(file.attrs.permissions, Some(0o100640));
                assert_eq!(file.attrs.uid, Some(7));
                assert_eq!(files.len(), 1);
            }
            assert!(operate(&session, &root, save(), true).await.is_err());
            assert_eq!(files.lock().unwrap()["/root/file.txt"].bytes, b"updated");
            for directory in [false, true] {
                let name = if directory { "new-folder" } else { "new.txt" };
                let create = || Action::Create {
                    parent: "/root".into(),
                    name: name.into(),
                    directory,
                };
                operate(&session, &root, create(), true).await.unwrap();
                assert!(operate(&session, &root, create(), true).await.is_err());
                let permissions = files.lock().unwrap()[&format!("/root/{name}")]
                    .attrs
                    .permissions
                    .unwrap();
                assert_eq!(permissions & 0o777, if directory { 0o700 } else { 0o600 });
            }
            for (parent, name) in [("/root", "../outside"), ("/root/escape", "private.txt")] {
                assert!(
                    operate(
                        &session,
                        &root,
                        Action::Create {
                            parent: parent.into(),
                            name: name.into(),
                            directory: false
                        },
                        true
                    )
                    .await
                    .is_err()
                );
            }
            assert_eq!(files.lock().unwrap()["/root/file.txt"].bytes, b"updated");
            assert!(
                operate(
                    &session,
                    &root,
                    Action::Rename {
                        path: "/root/file.txt".into(),
                        name: "new.txt".into()
                    },
                    true
                )
                .await
                .is_err()
            );
            for (path, name) in [
                ("/root", "moved"),
                ("/root/escape", "moved"),
                ("/root/file.txt", "../outside"),
            ] {
                assert!(
                    operate(
                        &session,
                        &root,
                        Action::Rename {
                            path: path.into(),
                            name: name.into()
                        },
                        true
                    )
                    .await
                    .is_err()
                );
            }
            operate(
                &session,
                &root,
                Action::Rename {
                    path: "/root/file.txt".into(),
                    name: "renamed.txt".into(),
                },
                true,
            )
            .await
            .unwrap();
            assert_eq!(files.lock().unwrap()["/root/renamed.txt"].bytes, b"updated");
            assert!(!files.lock().unwrap().contains_key("/root/file.txt"));
            for (path, directory) in [
                ("/root", true),
                ("/root/escape", false),
                ("/root/renamed.txt", true),
            ] {
                assert!(
                    operate(
                        &session,
                        &root,
                        Action::Delete {
                            path: path.into(),
                            directory
                        },
                        true
                    )
                    .await
                    .is_err()
                );
            }
            operate(
                &session,
                &root,
                Action::Create {
                    parent: "/root/new-folder".into(),
                    name: "child".into(),
                    directory: false,
                },
                true,
            )
            .await
            .unwrap();
            assert!(
                operate(
                    &session,
                    &root,
                    Action::Delete {
                        path: "/root/new-folder".into(),
                        directory: true
                    },
                    true
                )
                .await
                .is_err()
            );
            for (path, directory) in [
                ("/root/new-folder/child", false),
                ("/root/new-folder", true),
                ("/root/renamed.txt", false),
            ] {
                operate(
                    &session,
                    &root,
                    Action::Delete {
                        path: path.into(),
                        directory,
                    },
                    true,
                )
                .await
                .unwrap();
                assert!(!files.lock().unwrap().contains_key(path));
            }
            session.close_session().unwrap();
            server.abort();
        });
}

#[test]
fn slow_progress_and_cancelled_save_cleanup() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::pause();
            let original = vec![42; 8 * 32768];
            let files = Arc::new(Mutex::new(HashMap::from([(
                "/root/file.txt".into(),
                Record {
                    bytes: original.clone(),
                    attrs: FileAttributes {
                        permissions: Some(0o100600),
                        ..Default::default()
                    },
                },
            )])));
            let (client, server) = tokio::io::duplex(65536);
            let server = tokio::spawn(russh_sftp::server::run(
                server,
                Fixture {
                    files: files.clone(),
                    listed: false,
                    read_delay: Duration::from_secs(4),
                    write_delay: Duration::from_secs(2),
                    remove_delay: Duration::ZERO,
                },
            ));
            let session = RawSftpSession::new(client);
            session.init().await.unwrap();
            assert!(
                operate_with_timeout(
                    &session,
                    "/root",
                    Action::Read("file.txt".into()),
                    true,
                    Duration::from_secs(30)
                )
                .await
                .is_err(),
                "the former deadline must reproduce the failure"
            );
            let start = tokio::time::Instant::now();
            let Reply::File { bytes, .. } =
                operate(&session, "/root", Action::Read("file.txt".into()), true)
                    .await
                    .unwrap()
            else {
                panic!("expected file")
            };
            assert_eq!(bytes, original);
            assert!(start.elapsed() > Duration::from_secs(30));
            // Nine read replies take 36 seconds; expire during the first temp-file write.
            let error = operate_with_timeout(
                &session,
                "/root",
                Action::Save {
                    path: "/root/file.txt".into(),
                    original: original.clone(),
                    replacement: vec![9; 65536],
                },
                true,
                Duration::from_secs(37),
            )
            .await
            .err()
            .expect("deadline must cancel save");
            assert!(error.contains("deadline exceeded"), "{error}");
            assert!(!error.contains("cleanup could not"), "{error}");
            {
                let files = files.lock().unwrap();
                assert_eq!(
                    files.len(),
                    1,
                    "temporary file must be removed after cancellation"
                );
                assert_eq!(files["/root/file.txt"].bytes, original);
            }
            session.close_session().unwrap();
            server.abort();
        });
}

#[test]
fn unresponsive_cleanup_is_bounded_and_reported() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::pause();
            let files = Arc::new(Mutex::new(HashMap::from([(
                "/root/file.txt".into(),
                Record {
                    bytes: b"original".to_vec(),
                    attrs: FileAttributes {
                        permissions: Some(0o100600),
                        ..Default::default()
                    },
                },
            )])));
            let (client, server) = tokio::io::duplex(65536);
            let server = tokio::spawn(russh_sftp::server::run(
                server,
                Fixture {
                    files: files.clone(),
                    listed: false,
                    read_delay: Duration::ZERO,
                    write_delay: Duration::from_secs(2),
                    remove_delay: Duration::from_secs(20),
                },
            ));
            let session = RawSftpSession::new(client);
            session.init().await.unwrap();
            let start = tokio::time::Instant::now();
            let error = operate_with_timeout(
                &session,
                "/root",
                Action::Save {
                    path: "/root/file.txt".into(),
                    original: b"original".to_vec(),
                    replacement: b"updated".to_vec(),
                },
                true,
                Duration::from_secs(1),
            )
            .await
            .err()
            .expect("deadline");
            assert!(error.contains("cleanup could not be confirmed"), "{error}");
            assert!(start.elapsed() < Duration::from_secs(7));
            {
                let files = files.lock().unwrap();
                assert_eq!(files["/root/file.txt"].bytes, b"original");
                assert_eq!(
                    files.len(),
                    2,
                    "unacknowledged cleanup must not be reported as success"
                );
            }
            session.close_session().unwrap();
            server.abort();
        });
}
