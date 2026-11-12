use crate::{files::confined, workspace::Result};
use std::{
    fs,
    path::{Path, PathBuf},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Folder,
    Rename,
    Delete,
    Trash,
}
#[derive(Clone)]
pub struct Action {
    pub kind: Kind,
    pub root: PathBuf,
    pub path: PathBuf,
    pub name: String,
}
pub struct Change {
    pub before: Option<PathBuf>,
    pub after: Option<PathBuf>,
}
fn name_ok(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|p| {
        stem.strip_prefix(p).is_some_and(|n| {
            matches!(
                n,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    });
    !name.is_empty()
        && name.len() <= 255
        && !matches!(name, "." | "..")
        && !name.ends_with([' ', '.'])
        && !name
            .chars()
            .any(|c| c.is_control() || "/\\:\"<>|?*".contains(c))
        && !reserved
}
#[cfg(windows)]
fn rename_new(from: &Path, to: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::Storage::FileSystem::{MOVE_FILE_FLAGS, MoveFileExW},
        core::PCWSTR,
    };
    let from: Vec<_> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<_> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    // Zero flags means the OS refuses an existing destination atomically.
    unsafe {
        MoveFileExW(
            PCWSTR(from.as_ptr()),
            PCWSTR(to.as_ptr()),
            MOVE_FILE_FLAGS(0),
        )
    }
    .map_err(|_| "Cannot rename; destination may already exist or be in use".into())
}
#[cfg(not(windows))]
fn rename_new(_from: &Path, _to: &Path) -> Result<()> {
    Err("Atomic no-overwrite rename is currently supported on Windows only".into())
}

#[cfg(windows)]
static RECYCLE_OWNER: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

#[cfg(windows)]
pub fn set_recycle_owner<W: wry::raw_window_handle::HasWindowHandle>(parent: &W) {
    if let Ok(handle) = parent.window_handle()
        && let wry::raw_window_handle::RawWindowHandle::Win32(raw) = handle.as_raw()
    {
        RECYCLE_OWNER.store(raw.hwnd.get(), std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(windows)]
fn send_path_to_recycle_bin(path: &Path) -> Result<()> {
    let path = path.to_owned();
    std::thread::Builder::new()
        .name("recycle-bin-sta".into())
        .spawn(move || recycle_on_sta(&path))
        .map_err(|_| "Cannot start the Recycle Bin operation.".to_owned())?
        .join()
        .map_err(|_| "The Recycle Bin operation stopped unexpectedly.".to_owned())?
}

#[cfg(windows)]
fn recycle_on_sta(path: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::{
            System::Com::{
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoUninitialize,
            },
            UI::Shell::{
                FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FOF_WANTNUKEWARNING,
                FOFX_EARLYFAILURE, FOFX_RECYCLEONDELETE, FileOperation, IFileOperation, IShellItem,
                SHCreateItemFromParsingName,
            },
        },
        core::PCWSTR,
    };
    let mut wide: Vec<u16> = dunce::simplified(path).as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err("Path contains an embedded NUL character".into());
    }
    wide.push(0);
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|_| "Cannot initialize the Recycle Bin operation.")?;
        let _apartment = Apartment;
        let operation: IFileOperation =
            CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER)
                .map_err(|_| "Cannot open the Recycle Bin operation.")?;
        let owner = windows::Win32::Foundation::HWND(
            RECYCLE_OWNER.load(std::sync::atomic::Ordering::Relaxed) as *mut std::ffi::c_void,
        );
        let mut process = 0;
        windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(
            owner,
            Some(&mut process),
        );
        if process == std::process::id()
            && windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(owner)).as_bool()
        {
            operation
                .SetOwnerWindow(owner)
                .map_err(|_| "Cannot attach the Recycle Bin dialog to Tessel.")?;
        }
        // https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileoperation-setoperationflags
        // Explicit recycling; a permanent-delete fallback must never be silent.
        operation
            .SetOperationFlags(
                FOFX_RECYCLEONDELETE
                    | FOFX_EARLYFAILURE
                    | FOF_NOERRORUI
                    | FOF_SILENT
                    | FOF_NOCONFIRMATION
                    | FOF_WANTNUKEWARNING,
            )
            .map_err(|_| "Cannot request safe recycling.")?;
        let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None)
            .map_err(|_| "Cannot locate the item to recycle.")?;
        operation
            .DeleteItem(&item, None)
            .map_err(|_| "Cannot queue the item for recycling.")?;
        operation
            .PerformOperations()
            .map_err(|_| "Could not move item to the Recycle Bin.")?;
        if operation
            .GetAnyOperationsAborted()
            .map_err(|_| "Cannot verify the Recycle Bin operation.")?
            .as_bool()
        {
            return Err("The Recycle Bin operation was cancelled or failed.".into());
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn send_path_to_recycle_bin(_path: &Path) -> Result<()> {
    Err("Recycle Bin is currently supported on Windows only".into())
}

pub fn move_to_trash(root: &Path, target: &Path) -> Result<PathBuf> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let path = confined(&root, target)?;
    if path == root
        || fs::symlink_metadata(target)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
    {
        return Err("Cannot change the project root or a symbolic link".into());
    }
    send_path_to_recycle_bin(&path)?;
    Ok(path)
}

pub fn execute(action: Action) -> Result<Change> {
    let root = action.root.canonicalize().map_err(|e| e.to_string())?;
    let path = confined(&root, &action.path)?;
    if !matches!(action.kind, Kind::Delete | Kind::Trash) && !name_ok(&action.name) {
        return Err(
            "Enter a valid single filename; reserved names and path separators are refused".into(),
        );
    }
    match action.kind {
        Kind::File | Kind::Folder => {
            if !path.is_dir() {
                return Err("Choose a project folder".into());
            }
            let to = path.join(&action.name);
            if action.kind == Kind::File {
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&to)
                    .map_err(|e| e.to_string())?;
            } else {
                fs::create_dir(&to).map_err(|e| e.to_string())?;
            }
            Ok(Change {
                before: None,
                after: Some(to),
            })
        }
        Kind::Rename | Kind::Delete | Kind::Trash => {
            if path == root
                || fs::symlink_metadata(&action.path)
                    .map_err(|e| e.to_string())?
                    .file_type()
                    .is_symlink()
            {
                return Err("Cannot change the project root or a symbolic link".into());
            }
            if action.kind == Kind::Rename {
                let to = path.parent().ok_or("Missing parent")?.join(&action.name);
                rename_new(&path, &to)?;
                Ok(Change {
                    before: Some(path),
                    after: Some(to),
                })
            } else if action.kind == Kind::Trash {
                send_path_to_recycle_bin(&path)?;
                Ok(Change {
                    before: Some(path),
                    after: None,
                })
            } else {
                if path.is_dir() {
                    fs::remove_dir(&path).map_err(
                        |_| "Folder could not be removed; only empty folders can be deleted",
                    )?;
                } else if path.is_file() {
                    fs::remove_file(&path).map_err(|e| e.to_string())?;
                } else {
                    return Err("Only regular files and empty folders can be deleted".into());
                }
                Ok(Change {
                    before: Some(path),
                    after: None,
                })
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn confined_operations_preserve_collisions_and_refuse_recursive_delete() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let action = |kind, path: &Path, name: &str| Action {
            kind,
            root: root.clone(),
            path: path.into(),
            name: name.into(),
        };
        execute(action(Kind::File, &root, "original.txt")).unwrap();
        fs::write(root.join("original.txt"), b"original").unwrap();
        assert!(execute(action(Kind::File, &root, "original.txt")).is_err());
        execute(action(Kind::Folder, &root, "folder")).unwrap();
        execute(action(Kind::File, &root.join("folder"), "child.txt")).unwrap();
        assert!(execute(action(Kind::Delete, &root.join("folder"), "")).is_err());
        assert!(execute(action(Kind::Delete, &root, "")).is_err());
        let outside = tempfile::tempdir().unwrap();
        assert!(execute(action(Kind::File, outside.path(), "escape")).is_err());
        for name in [
            "../bad", "a/b", "NUL", "CON.txt", "a:stream", "a.", "a ", "COM¹",
        ] {
            assert!(execute(action(Kind::File, &root, name)).is_err());
        }
        #[cfg(windows)]
        {
            execute(action(Kind::File, &root, "existing.txt")).unwrap();
            assert!(
                execute(action(
                    Kind::Rename,
                    &root.join("original.txt"),
                    "existing.txt"
                ))
                .is_err()
            );
            execute(action(
                Kind::Rename,
                &root.join("original.txt"),
                "renamed.txt",
            ))
            .unwrap();
            assert_eq!(fs::read(root.join("renamed.txt")).unwrap(), b"original");
        }
        execute(action(Kind::Delete, &root.join("folder/child.txt"), "")).unwrap();
        execute(action(Kind::Delete, &root.join("folder"), "")).unwrap();
    }
}
