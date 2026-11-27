//! Download an SFTP directory into a newly reserved local directory.
use super::{
    MAX_PREVIEW_BYTES, RawSftpSession, canonical, child_name, read_bounded, remote_error, scoped,
};
use russh_sftp::{
    client::error::Error,
    protocol::{FileType, StatusCode},
};
use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

pub(crate) type Progress = Arc<AtomicUsize>;
const FILE_LIMIT: usize = 5000;
const DIRECTORY_LIMIT: usize = 2000;
const BYTE_LIMIT: usize = 1024 * 1024 * 1024;

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub path: PathBuf,
    files: usize,
    skipped: usize,
    too_large: usize,
    cut: bool,
}

impl Report {
    pub fn message(&self, fr: bool) -> String {
        let path = self.path.display();
        let mut message = if fr {
            format!(
                "Dossier téléchargé dans {path} · Fichiers enregistrés : {}",
                self.files
            )
        } else {
            format!("Folder downloaded to {path} · Files saved: {}", self.files)
        };
        if self.skipped > 0 {
            message.push_str(&format!(
                " · {} {}",
                self.skipped,
                if fr {
                    "élément(s) ignoré(s)"
                } else {
                    "skipped"
                }
            ));
        }
        if self.too_large > 0 {
            message.push_str(&format!(
                " · {} {}",
                self.too_large,
                if fr {
                    "fichier(s) de plus de 50 Mio"
                } else {
                    "over 50 MiB"
                }
            ));
        }
        if self.cut {
            message.push_str(if fr {
                " · Transfert incomplet : limite atteinte."
            } else {
                " · Incomplete transfer: limit reached."
            });
        }
        message
    }
}

async fn list(
    session: &RawSftpSession,
    path: &str,
    report: &mut Report,
) -> Result<Vec<(String, FileType)>, String> {
    if canonical(session, path).await? != path {
        return Err("Remote directory changed".into());
    }
    let handle = session.opendir(path).await.map_err(remote_error)?.handle;
    let result = async {
        let mut entries = Vec::new();
        let mut seen = 0;
        loop {
            let batch = match session.readdir(&handle).await {
                Ok(batch) if !batch.files.is_empty() => batch,
                Ok(_) => return Err("Empty SFTP directory response".into()),
                Err(Error::Status(s)) if s.status_code == StatusCode::Eof => break,
                Err(e) => return Err(remote_error(e)),
            };
            for entry in batch.files {
                if entry.filename == "." || entry.filename == ".." {
                    continue;
                }
                seen += 1;
                if seen > 10_000 {
                    report.cut = true;
                    return Ok(entries);
                }
                let kind = entry.attrs.file_type();
                if child_name(&entry.filename) && matches!(kind, FileType::Dir | FileType::File) {
                    entries.push((entry.filename, kind));
                } else {
                    report.skipped += 1;
                }
            }
        }
        Ok(entries)
    }
    .await;
    session.close(handle).await.map_err(remote_error)?;
    result
}

pub(super) async fn download(
    session: &RawSftpSession,
    root: &str,
    path: &str,
    parent: &Path,
    progress: &Progress,
    deadline: Duration,
) -> Result<Report, String> {
    let source = tokio::time::timeout(deadline, async {
        let source = scoped(session, root, path).await?;
        if source != path
            || source == root
            || !session
                .lstat(&source)
                .await
                .map_err(remote_error)?
                .attrs
                .is_dir()
        {
            return Err("Select a listed remote directory".to_string());
        }
        Ok(source)
    })
    .await
    .map_err(|_| "Remote directory lookup timed out")??;
    let local_parent = fs::canonicalize(parent).map_err(|e| e.to_string())?;
    let local = crate::download_name::reserve_folder(
        &local_parent,
        &crate::download_name::suggest(&source),
    )
    .map_err(|e| e.to_string())?;
    let mut report = Report {
        path: dunce::simplified(&local).to_owned(),
        ..Report::default()
    };
    let mut queue = VecDeque::from([(source, local)]);
    let mut folders = 0;
    let mut bytes = 0;
    'tree: while let Some((directory, dest)) = queue.pop_front() {
        if fs::canonicalize(&dest).ok().as_ref() != Some(&dest) {
            report.skipped += 1;
            continue;
        }
        let entries =
            match tokio::time::timeout(deadline, list(session, &directory, &mut report)).await {
                Ok(Ok(entries)) => entries,
                Ok(Err(error)) if directory == path => {
                    let _ = fs::remove_dir(&dest);
                    return Err(error);
                }
                Ok(Err(_)) => {
                    report.skipped += 1;
                    continue;
                }
                Err(_) => {
                    report.cut = true;
                    break;
                }
            };
        for (name, kind) in entries {
            let remote_path = format!("{directory}/{name}");
            let target = dest.join(crate::download_name::suggest(&name));
            if kind == FileType::Dir {
                if folders >= DIRECTORY_LIMIT {
                    report.cut = true;
                    break 'tree;
                }
                if fs::canonicalize(&dest).ok().as_ref() != Some(&dest)
                    || fs::create_dir(&target).is_err()
                {
                    report.skipped += 1;
                    continue;
                }
                folders += 1;
                queue.push_back((remote_path, target));
                continue;
            }
            if report.files >= FILE_LIMIT || bytes >= BYTE_LIMIT {
                report.cut = true;
                break 'tree;
            }
            let fetch = async {
                if canonical(session, &remote_path).await? != remote_path {
                    return Err("Remote file changed".into());
                }
                read_bounded(session, &remote_path, MAX_PREVIEW_BYTES).await
            };
            match tokio::time::timeout(deadline, fetch).await {
                Ok(Ok(Some((data, _)))) => {
                    if data.len() > BYTE_LIMIT - bytes {
                        report.cut = true;
                        break 'tree;
                    }
                    if fs::canonicalize(&dest).ok().as_ref() != Some(&dest)
                        || crate::files::save_download(&target, &data).is_err()
                    {
                        report.skipped += 1;
                        continue;
                    }
                    bytes += data.len();
                    report.files += 1;
                    progress.store(report.files, Ordering::Relaxed);
                }
                Ok(Ok(None)) => report.too_large += 1,
                Ok(Err(_)) => report.skipped += 1,
                Err(_) => {
                    report.cut = true;
                    break 'tree;
                }
            }
        }
        if report.cut {
            break;
        }
    }
    Ok(report)
}
