//! Owned Windows process-job cleanup shared by SSH and Git.
#[cfg(windows)]
pub(crate) struct ProcessJob(windows::Win32::Foundation::HANDLE);
// Windows job handles are kernel objects without thread affinity; ownership is unique.
#[cfg(windows)]
unsafe impl Send for ProcessJob {}
#[cfg(windows)]
impl ProcessJob {
    /// Let intentionally detached children survive a normal parent exit.
    #[allow(dead_code)] // SSH retains kill-on-close; Git releases successful background helpers.
    pub(crate) fn disarm(&mut self) -> Result<(), String> {
        use windows::Win32::System::JobObjects::*;
        let limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        unsafe {
            SetInformationJobObject(
                self.0,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
            .map_err(|_| "Cannot release process cleanup".to_string())
        }
    }
    pub(crate) fn attach(process: std::os::windows::io::RawHandle) -> Result<Self, String> {
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
