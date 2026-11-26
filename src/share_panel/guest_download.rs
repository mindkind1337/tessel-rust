use super::*;
use crate::guest_file_editor::Key;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    io::Write,
    path::PathBuf,
    sync::mpsc::{Receiver, TryRecvError},
};
use tempfile::NamedTempFile;

enum Failure {
    Exists,
    Unavailable,
}

#[derive(Default)]
pub(super) struct Download {
    pending: Option<(PathBuf, Receiver<Option<NamedTempFile>>)>,
    notice: Option<Result<PathBuf, Failure>>,
    pub menu: Option<Key>,
}
impl Download {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn status(&self, fr: bool) -> Option<String> {
        if self.busy() {
            return Some(
                if fr {
                    "Téléchargement…"
                } else {
                    "Downloading…"
                }
                .into(),
            );
        }
        let result = self.notice.as_ref()?;
        let message = match result {
            Ok(path) => {
                return Some(format!(
                    "{} {}",
                    if fr {
                        "Fichier enregistré dans"
                    } else {
                        "File saved to"
                    },
                    path.display()
                ));
            }
            Err(Failure::Exists) if fr => "Ce fichier existe déjà. Choisissez un autre nom.",
            Err(Failure::Exists) => "A file with that name already exists. Choose another name.",
            Err(Failure::Unavailable) if fr => {
                "Téléchargement impossible. Vérifiez l’accès au fichier et la limite de 8 Mio."
            }
            Err(Failure::Unavailable) => {
                "Download failed. Check file access and the 8 MiB size limit."
            }
        };
        Some(message.into())
    }
    pub fn poll(&mut self) -> bool {
        let Some((_, rx)) = &self.pending else {
            return false;
        };
        let file = match rx.try_recv() {
            Ok(file) => file,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => None,
        };
        let (dest, _) = self.pending.take().unwrap();
        self.notice = Some(file.ok_or(Failure::Unavailable).and_then(|f| {
            f.persist_noclobber(&dest).map(|_| dest).map_err(|e| {
                if e.error.kind() == std::io::ErrorKind::AlreadyExists {
                    Failure::Exists
                } else {
                    Failure::Unavailable
                }
            })
        }));
        true
    }
}

pub(super) fn original_bytes(value: &serde_json::Value) -> Option<Vec<u8>> {
    let size = usize::try_from(value["size"].as_u64()?).ok()?;
    if size > crate::share::MAX_FILE {
        return None;
    }
    let data = value["base64"].as_str()?;
    if data.len() != size.div_ceil(3) * 4 {
        return None;
    }
    let bytes = STANDARD.decode(data).ok()?;
    if bytes.len() != size {
        return None;
    }
    Some(bytes)
}
fn prepare(value: serde_json::Value, dest: &Path) -> Option<NamedTempFile> {
    let bytes = original_bytes(&value)?;
    let mut file = NamedTempFile::new_in(dest.parent()?).ok()?;
    file.write_all(&bytes).ok()?;
    file.as_file().sync_all().ok()?;
    Some(file)
}

impl SharePanel {
    pub(super) fn downloadable(&self, key: &Key) -> bool {
        self.guest_files_scope == Some(key.scope)
            && (self.guest_image.key.as_ref() == Some(key)
                || self
                    .guest_file_listing
                    .entries
                    .iter()
                    .any(|e| !e.directory && e.path == key.path))
    }
    pub(crate) fn download_guest_file(
        &mut self,
        key: &Key,
        choose: impl FnOnce(&str) -> Option<PathBuf>,
    ) {
        self.poll_guest_scope();
        self.guest_download.menu = None;
        if self.guest_download.busy() || !self.downloadable(key) {
            return;
        }
        let Some(session) = &self.guest_session else {
            return;
        };
        let name = key.path.rsplit('/').next().unwrap_or("download.bin");
        let suggested = name.replace(['<', '>', ':', '"', '/', '\\', '|', '?', '*'], "_");
        let Some(dest) = choose(&suggested) else {
            return;
        };
        if dest.exists() {
            self.guest_download.notice = Some(Err(Failure::Exists));
            return;
        }
        let Ok(rx) = session.request(
            serde_json::json!({"op":"fs.download","projectId":key.scope.project,"path":key.path}),
        ) else {
            self.guest_download.notice = Some(Err(Failure::Unavailable));
            return;
        };
        let (tx, result) = std::sync::mpsc::channel();
        self.guest_download.pending = Some((dest.clone(), result));
        std::thread::spawn(move || {
            let file = rx
                .recv_timeout(Duration::from_secs(30))
                .ok()
                .and_then(Result::ok)
                .and_then(|v| prepare(v, &dest));
            let _ = tx.send(file);
        });
    }
    pub(super) fn choose_guest_download(&mut self, key: &Key) {
        self.download_guest_file(key, |name| {
            rfd::FileDialog::new().set_file_name(name).save_file()
        });
    }
}
