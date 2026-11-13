use std::path::Path;
type Result<T> = std::result::Result<T, String>;
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
pub fn send_path_to_recycle_bin(path: &Path) -> Result<()> {
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
pub fn send_path_to_recycle_bin(_path: &Path) -> Result<()> {
    Err("Recycle Bin is currently supported on Windows only".into())
}
