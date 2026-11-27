//! Select an existing local project item in Windows File Explorer.
use std::{os::windows::ffi::OsStrExt, path::Path};
use windows::{
    Win32::{
        System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize},
        UI::Shell::{Common::ITEMIDLIST, SHOpenFolderAndSelectItems, SHParseDisplayName},
    },
    core::PCWSTR,
};

pub(crate) fn reveal(root: &Path, path: &Path) -> Result<(), String> {
    with_item(root, path, |item| unsafe {
        SHOpenFolderAndSelectItems(item, None, 0)
    })
}

fn with_item(
    root: &Path,
    path: &Path,
    select: impl FnOnce(*const ITEMIDLIST) -> windows::core::Result<()>,
) -> Result<(), String> {
    let path = crate::files::confined(root, path)?;
    let wide: Vec<u16> = dunce::simplified(&path)
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();

    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }
    struct Item(*mut ITEMIDLIST);
    impl Drop for Item {
        fn drop(&mut self) {
            unsafe {
                CoTaskMemFree(Some(self.0.cast()));
            }
        }
    }

    // The caller uses a dedicated thread. Both the allocation and the apartment
    // outlive the shell call and are released on this same thread.
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|e| e.to_string())?;
        let _apartment = Apartment;
        let mut item = Item(std::ptr::null_mut());
        SHParseDisplayName(PCWSTR(wide.as_ptr()), None, &mut item.0, 0, None)
            .map_err(|e| e.to_string())?;
        select(item.0).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{ffi::OsString, os::windows::ffi::OsStringExt};
    use windows::Win32::UI::Shell::{GPFIDL_DEFAULT, SHGetPathFromIDListEx};

    #[test]
    fn explorer_reveal_resolves_the_exact_shell_item_and_confines_the_request() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("projet partagé");
        let folder = root.join("Dossier, [1]");
        let file = folder.join("résumé & notes.txt");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(&file, []).unwrap();
        for target in [&root, &folder, &file] {
            with_item(&root, target, |id| {
                let mut buf = vec![0u16; 32768];
                assert!(unsafe { SHGetPathFromIDListEx(id, &mut buf, GPFIDL_DEFAULT) }.as_bool());
                let end = buf.iter().position(|c| *c == 0).unwrap();
                let actual = OsString::from_wide(&buf[..end]);
                assert_eq!(
                    std::fs::canonicalize(actual).unwrap(),
                    target.canonicalize().unwrap()
                );
                Ok(())
            })
            .unwrap();
        }
        for refused in [tmp.path().to_owned(), root.join("missing")] {
            assert!(with_item(&root, &refused, |_| panic!("unavailable shell target")).is_err());
        }
    }
}
