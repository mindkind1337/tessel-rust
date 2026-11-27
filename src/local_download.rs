//! Local file explorer downloads. The picker chooses a file or a parent directory.
use std::{
    collections::VecDeque,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;

const FILE_LIMIT: usize = 20_000;
const BYTE_LIMIT: u64 = 20 * 1024 * 1024 * 1024;
const FOLDER_LIMIT: usize = 5_000;

#[derive(Default)]
pub(crate) struct Report {
    path: PathBuf,
    files: usize,
    skipped: usize,
    cut: bool,
}

impl Report {
    pub fn message(&self, fr: bool) -> String {
        let path = dunce::simplified(&self.path).display();
        let mut s = if fr {
            format!("Copie enregistrée dans {path} · Fichiers : {}", self.files)
        } else {
            format!("Copy saved to {path} · Files: {}", self.files)
        };
        if self.skipped > 0 {
            s.push_str(&format!(
                " · {}: {}",
                if fr { "Éléments ignorés" } else { "Skipped" },
                self.skipped
            ));
        }
        if self.cut {
            s.push_str(if fr {
                " · Limite de copie atteinte."
            } else {
                " · Copy limit reached."
            });
        }
        s
    }
}

pub(crate) fn source(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let real = crate::files::confined(root, path)?;
    if real == fs::canonicalize(root).map_err(|e| e.to_string())? {
        return Err("Select a project file or subfolder.".into());
    }
    if real.is_dir() && crate::worktree::is_reparse_or_symlink(path) {
        return Err("This folder is a link to another location.".into());
    }
    if !real.is_file() && !real.is_dir() {
        return Err("Select a regular file or folder.".into());
    }
    Ok(real)
}

fn copy_file(src: &Path, dest: &Path, budget: u64, replace: bool) -> io::Result<Option<u64>> {
    let input = fs::File::open(src)?;
    if input.metadata()?.len() > budget {
        return Ok(None);
    }
    let mut temp = NamedTempFile::new_in(dest.parent().ok_or(io::ErrorKind::InvalidInput)?)?;
    let n = io::copy(&mut input.take(budget.saturating_add(1)), &mut temp)?;
    if n > budget {
        return Ok(None);
    }
    temp.as_file().sync_all()?;
    if replace {
        temp.persist(dest)?;
    } else {
        temp.persist_noclobber(dest)?;
    }
    Ok(Some(n))
}

pub(crate) fn copy(
    root: &Path,
    path: &Path,
    dest: &Path,
    folder: bool,
    mut progress: impl FnMut(usize),
) -> Result<Report, String> {
    let src = source(root, path)?;
    if src.is_dir() != folder {
        return Err("The selected item changed type.".into());
    }
    if !folder {
        if fs::canonicalize(dest).ok().as_ref() != Some(&src) {
            copy_file(&src, dest, u64::MAX, dest.exists()).map_err(|e| e.to_string())?;
        }
        return Ok(Report {
            path: dest.into(),
            files: 1,
            ..Default::default()
        });
    }
    let parent = fs::canonicalize(dest).map_err(|e| e.to_string())?;
    if parent.starts_with(&src) {
        return Err("A folder cannot be downloaded into itself.".into());
    }
    let name =
        crate::download_name::suggest(&path.file_name().unwrap_or_default().to_string_lossy());
    let local = crate::download_name::reserve_folder(&parent, &name).map_err(|e| e.to_string())?;
    let mut report = Report {
        path: local.clone(),
        ..Default::default()
    };
    let mut dirs = 1;
    let mut bytes = 0;
    let mut queue = VecDeque::from([(src, local)]);
    'tree: while let Some((from, to)) = queue.pop_front() {
        if fs::canonicalize(&from).ok().as_ref() != Some(&from)
            || fs::canonicalize(&to).ok().as_ref() != Some(&to)
        {
            report.skipped += 1;
            continue;
        }
        let Ok(entries) = from.read_dir() else {
            report.skipped += 1;
            continue;
        };
        for item in entries {
            let Ok(entry) = item else {
                report.skipped += 1;
                continue;
            };
            let child = entry.path();
            let target = to.join(entry.file_name());
            if crate::worktree::is_reparse_or_symlink(&child) {
                report.skipped += 1;
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                report.skipped += 1;
                continue;
            };
            if kind.is_dir() {
                if dirs >= FOLDER_LIMIT {
                    report.cut = true;
                    break 'tree;
                }
                if fs::create_dir(&target).is_err() {
                    report.skipped += 1;
                    continue;
                }
                dirs += 1;
                queue.push_back((child, target));
            } else if kind.is_file() {
                if report.files >= FILE_LIMIT {
                    report.cut = true;
                    break 'tree;
                }
                match copy_file(&child, &target, BYTE_LIMIT - bytes, false) {
                    Ok(Some(n)) => {
                        bytes += n;
                        report.files += 1;
                        progress(report.files);
                    }
                    Ok(None) => {
                        report.cut = true;
                        break 'tree;
                    }
                    Err(_) => report.skipped += 1,
                }
            } else {
                report.skipped += 1;
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn explorer_download_skips_junctions_and_refuses_a_linked_folder() {
        let project = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let tree = project.path().join("tree");
        fs::create_dir(&tree).unwrap();
        let link = tree.join("linked");
        let status = std::process::Command::new("cmd")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(&link)
            .arg(out.path())
            .output()
            .unwrap();
        assert!(status.status.success());
        assert!(source(project.path(), &link).is_err());
        let report = copy(project.path(), &tree, out.path(), true, |_| {}).unwrap();
        assert_eq!(report.skipped, 1);
        assert!(!report.path.join("linked").exists());
    }

    #[test]
    fn explorer_download_size_limit_and_collisions_leave_no_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source");
        let dest = dir.path().join("copy");
        fs::write(&src, b"four").unwrap();
        assert_eq!(copy_file(&src, &dest, 3, false).unwrap(), None);
        assert!(!dest.exists());
        fs::write(&dest, b"mine").unwrap();
        assert!(copy_file(&src, &dest, 4, false).is_err());
        assert_eq!(fs::read(&dest).unwrap(), b"mine");
        assert_eq!(dir.path().read_dir().unwrap().count(), 2);
    }

    #[test]
    fn explorer_download_copies_originals_and_prevents_recursive_destinations() {
        let project = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let dir = project.path().join("folder");
        fs::create_dir_all(dir.join("nested")).unwrap();
        let file = dir.join("nested/bytes");
        fs::write(&file, [0, 255, 4]).unwrap();
        fs::write(dir.join(".hidden"), b"h").unwrap();
        assert!(copy(project.path(), &dir, &dir.join("nested"), true, |_| {}).is_err());
        fs::create_dir(out.path().join("folder")).unwrap();
        let report = copy(project.path(), &dir, out.path(), true, |_| {}).unwrap();
        assert_eq!(report.path.file_name().unwrap(), "folder (2)");
        assert_eq!(report.files, 2);
        assert_eq!(
            fs::read(report.path.join("nested/bytes")).unwrap(),
            [0, 255, 4]
        );
        assert!(report.path.join(".hidden").is_file());
        let dest = out.path().join("chosen.bin");
        fs::write(&dest, b"old").unwrap();
        copy(project.path(), &file, &dest, false, |_| {}).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), [0, 255, 4]);
        copy(project.path(), &file, &file, false, |_| {}).unwrap();
        assert_eq!(fs::read(&file).unwrap(), [0, 255, 4]);
        assert!(copy(project.path(), &dest, &file, false, |_| {}).is_err());
    }
}
