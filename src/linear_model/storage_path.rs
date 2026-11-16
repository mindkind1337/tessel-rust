//! Reject redirected credential paths. Checks do not replace descriptor-relative
//! traversal: ancestor replacement by another process remains a separate race.
use std::{
    fs::{self, File, Metadata, OpenOptions},
    path::{Component, Path},
};
const ERROR: &str = "Linear storage must be a regular private location";
fn redirected(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}
fn local_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(ERROR.into());
    }
    Ok(())
}
/// Walk every existing ancestor before allowing missing descendants.
pub(super) fn inspect(path: &Path) -> Result<Option<Metadata>, String> {
    local_path(path)?;
    let mut found = None;
    for current in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
        let metadata = match fs::symlink_metadata(current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(ERROR.into()),
        };
        if redirected(&metadata) || (current != path && !metadata.is_dir()) {
            return Err(ERROR.into());
        }
        found = Some(metadata);
    }
    Ok(found)
}
pub(super) fn directory(path: &Path) -> Result<(), String> {
    match inspect(path)? {
        Some(metadata) if metadata.is_dir() => Ok(()),
        Some(_) => Err(ERROR.into()),
        None => {
            directory(path.parent().ok_or(ERROR)?)?;
            match fs::create_dir(path) {
                Ok(()) => (),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
                Err(_) => return Err(ERROR.into()),
            }
            if !inspect(path)?.is_some_and(|metadata| metadata.is_dir()) {
                return Err(ERROR.into());
            }
            Ok(())
        }
    }
}
fn single_file(file: &File) -> Result<(), String> {
    let metadata = file.metadata().map_err(|_| ERROR)?;
    if !metadata.is_file() || redirected(&metadata) {
        return Err(ERROR.into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle},
        };
        let mut information = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: borrowed live File handle; output is valid writable storage.
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut information) }
            .map_err(|_| ERROR)?;
        if information.nNumberOfLinks != 1 {
            return Err(ERROR.into());
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(ERROR.into());
        }
    }
    #[cfg(not(any(windows, unix)))]
    {
        return Err(ERROR.into());
    }
    Ok(())
}
pub(super) fn open(path: &Path, create: bool) -> Result<Option<File>, String> {
    let metadata = inspect(path)?;
    if metadata
        .as_ref()
        .is_some_and(|metadata| !metadata.is_file())
    {
        return Err(ERROR.into());
    }
    if metadata.is_none() && !create {
        return Ok(None);
    }
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(create)
        .create(create)
        .truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options.open(path).map_err(|_| ERROR)?;
    single_file(&file)?;
    inspect(path)?;
    Ok(Some(file))
}
pub(super) fn check_file(path: &Path) -> Result<(), String> {
    open(path, false).map(|_| ())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hardlinked_credentials_and_locks_refuse_all_storage_operations() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("untouched");
        fs::write(&target, b"synthetic fixture").unwrap();
        for name in ["linear.dat", "linear.lock"] {
            let folder = root.path().join(name.replace('.', "-"));
            fs::create_dir(&folder).unwrap();
            fs::hard_link(&target, folder.join(name)).unwrap();
            let path = folder.join("linear.dat");
            assert!(super::super::load_key_at(&path).is_err());
            assert!(super::super::delete_key_at(&path).is_err());
            assert!(super::super::save_key_at(&path, "synthetic-key-123").is_err());
            assert_eq!(fs::read(&target).unwrap(), b"synthetic fixture");
        }
    }
    #[cfg(windows)]
    #[test]
    fn ancestor_junction_refuses_reads_writes_and_directory_creation() {
        use std::os::windows::process::CommandExt;
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        fs::create_dir(&target).unwrap();
        let alias = root.path().join("alias");
        let output = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(&alias)
            .arg(&target)
            .creation_flags(0x08000000)
            .output()
            .unwrap();
        assert!(output.status.success());
        for path in [alias.join("linear.dat"), alias.join("nested/linear.dat")] {
            assert!(super::super::load_key_at(&path).is_err());
            assert!(super::super::delete_key_at(&path).is_err());
            assert!(super::super::save_key_at(&path, "synthetic-key-123").is_err());
        }
        assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
        fs::remove_dir(&alias).unwrap();
    }
}
