//! UI-thread WebView2 download tracking; no URLs, cookies or headers are retained.
use super::{Event, download_key};
use serde::Serialize;
use std::time::{Duration, Instant};
use std::{cell::RefCell, collections::VecDeque, path::PathBuf, rc::Rc, sync::mpsc::Sender};
use webview2_com::{
    DownloadStartingEventHandler, Microsoft::Web::WebView2::Win32::*, StateChangedEventHandler,
    take_pwstr,
};
use windows::core::{HSTRING, Interface, PWSTR};
use wry::WebViewExtWindows;
#[derive(Clone, Serialize, PartialEq, Eq)]
pub(super) struct Record {
    pub id: u64,
    pub path: PathBuf,
    pub status: String,
    pub received_bytes: u64,
    pub total_bytes: Option<u64>,
    pub error: Option<String>,
}
struct Download {
    record: Record,
    operation: Option<ICoreWebView2DownloadOperation>,
    last_progress: Instant,
}
impl Drop for Download {
    fn drop(&mut self) {
        if let Some(op) = self.operation.take() {
            let _ = unsafe { op.Cancel() };
        }
    }
}
struct State {
    stall_timeout: Duration,
    next_id: u64,
    items: VecDeque<Download>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            stall_timeout: Duration::from_secs(120),
            next_id: 0,
            items: VecDeque::new(),
        }
    }
}
#[derive(Default)]
pub(super) struct Downloads {
    state: Rc<RefCell<State>>,
}
impl Downloads {
    pub fn set_stall_timeout(&self, timeout: Duration) {
        self.state.borrow_mut().stall_timeout = timeout;
    }
    pub fn attach(
        &self,
        view: &wry::WebView,
        choose: impl Fn(PathBuf) -> Result<PathBuf, String> + 'static,
        errors: Sender<Event>,
    ) -> Result<(), String> {
        let state = self.state.clone();
        unsafe {
            let webview: ICoreWebView2_4 = view
                .controller()
                .CoreWebView2()
                .and_then(|v| v.cast())
                .map_err(|_| "Download tracking unavailable")?;
            webview
                .add_DownloadStarting(
                    &DownloadStartingEventHandler::create(Box::new(move |_, args| {
                        let Some(args) = args else {
                            return Ok(());
                        };
                        args.SetCancel(true)?;
                        args.SetHandled(true)?;
                        let state_ref = state.clone();
                        let mut state = state_ref.borrow_mut();
                        let operation = args.DownloadOperation()?;
                        if let Some(existing) = state.items.iter().find(|d| {
                            d.operation
                                .as_ref()
                                .is_some_and(|previous| previous.as_raw() == operation.as_raw())
                        }) {
                            args.SetResultFilePath(&HSTRING::from(
                                dunce::simplified(&existing.record.path).as_os_str(),
                            ))?;
                            args.SetCancel(false)?;
                            return Ok(());
                        }
                        if state.items.iter().filter(|d| d.operation.is_some()).count() >= 8 {
                            args.SetCancel(true)?;
                            let _ = errors
                                .send(Event::Error("At most 8 downloads may run at once".into()));
                            return Ok(());
                        }
                        let mut suggested = PWSTR::null();
                        args.ResultFilePath(&mut suggested)?;
                        let target = match choose(PathBuf::from(take_pwstr(suggested))) {
                            Ok(path) => path,
                            Err(e) => {
                                args.SetCancel(true)?;
                                let _ = errors.send(Event::Error(e));
                                return Ok(());
                            }
                        };
                        let Some(id) = state.next_id.checked_add(1) else {
                            args.SetCancel(true)?;
                            return Ok(());
                        };
                        state.next_id = id;
                        // WebView2 receives the exact original destination in its authenticated session.
                        args.SetResultFilePath(&HSTRING::from(
                            dunce::simplified(&target).as_os_str(),
                        ))?;
                        let operation_events = operation.clone();
                        while state.items.len() >= 100 {
                            if let Some(i) = state.items.iter().rposition(|d| d.operation.is_none())
                            {
                                state.items.remove(i);
                            } else {
                                break;
                            }
                        }
                        state.items.push_front(Download {
                            record: Record {
                                id,
                                path: download_key(target),
                                status: "Downloading".into(),
                                received_bytes: 0,
                                total_bytes: None,
                                error: None,
                            },
                            operation: Some(operation),
                            last_progress: Instant::now(),
                        });
                        let weak = Rc::downgrade(&state_ref);
                        drop(state);
                        operation_events.add_StateChanged(
                            &StateChangedEventHandler::create(Box::new(move |op, _| {
                                let (Some(state), Some(op)) = (weak.upgrade(), op) else {
                                    return Ok(());
                                };
                                let mut status = COREWEBVIEW2_DOWNLOAD_STATE::default();
                                op.State(&mut status)?;
                                if status == COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS {
                                    return Ok(());
                                }
                                let mut reason = COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON::default();
                                if status == COREWEBVIEW2_DOWNLOAD_STATE_INTERRUPTED {
                                    op.InterruptReason(&mut reason)?;
                                }
                                if let Ok(mut state) = state.try_borrow_mut()
                                    && let Some(download) =
                                        state.items.iter_mut().find(|d| d.record.id == id)
                                {
                                    if download.record.status == "Failed"
                                        && download.operation.is_none()
                                    {
                                        return Ok(());
                                    }
                                    download.record.status =
                                        if status == COREWEBVIEW2_DOWNLOAD_STATE_COMPLETED {
                                            "Completed"
                                        } else if reason
                                            == COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_USER_CANCELED
                                        {
                                            "Cancelled"
                                        } else {
                                            "Failed"
                                        }
                                        .into();
                                    if download.record.status == "Failed" {
                                        download.record.error = Some(interruption_message(reason));
                                    }
                                    let mut bytes = 0;
                                    let mut total = 0;
                                    if op.BytesReceived(&mut bytes).is_ok() {
                                        download.record.received_bytes = bytes.max(0) as u64;
                                    }
                                    if op.TotalBytesToReceive(&mut total).is_ok() {
                                        download.record.total_bytes =
                                            (total >= 0).then_some(total as u64);
                                    }
                                    download.operation = None;
                                }
                                Ok(())
                            })),
                            &mut 0,
                        )?;
                        args.SetCancel(false)?;
                        Ok(())
                    })),
                    &mut 0,
                )
                .map_err(|_| "Cannot attach download tracking")?;
        }
        Ok(())
    }
    pub fn poll(&self) -> VecDeque<Record> {
        let mut state = self.state.borrow_mut();
        let stall_timeout = state.stall_timeout;
        for download in &mut state.items {
            let Some(op) = &download.operation else {
                continue;
            };
            let result = unsafe {
                (|| -> windows::core::Result<_> {
                    let mut received = 0;
                    let mut total = 0;
                    let mut status = COREWEBVIEW2_DOWNLOAD_STATE::default();
                    op.BytesReceived(&mut received)?;
                    op.TotalBytesToReceive(&mut total)?;
                    op.State(&mut status)?;
                    let mut reason = COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON::default();
                    if status == COREWEBVIEW2_DOWNLOAD_STATE_INTERRUPTED {
                        op.InterruptReason(&mut reason)?;
                    }
                    Ok((received, total, status, reason))
                })()
            };
            match result {
                Ok((received, total, status, reason)) => {
                    if received.max(0) as u64 > download.record.received_bytes {
                        download.last_progress = Instant::now();
                    }
                    download.record.received_bytes = received.max(0) as u64;
                    download.record.total_bytes =
                        if total >= 0 { Some(total as u64) } else { None };
                    if status == COREWEBVIEW2_DOWNLOAD_STATE_COMPLETED {
                        download.record.status = "Completed".into();
                        download.operation = None;
                    } else if status == COREWEBVIEW2_DOWNLOAD_STATE_INTERRUPTED {
                        download.record.status =
                            if reason == COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_USER_CANCELED {
                                "Cancelled"
                            } else {
                                "Failed"
                            }
                            .into();
                        if download.record.status == "Failed" {
                            download.record.error = Some(interruption_message(reason));
                        }
                        download.operation = None;
                    }
                }
                Err(_) => {
                    download.record.status = "Failed".into();
                    download.record.error = Some("Cannot read download state".into());
                    if let Some(op) = download.operation.take() {
                        let _ = unsafe { op.Cancel() };
                    }
                }
            }
        }
        for download in &mut state.items {
            if download.operation.is_some() && download.last_progress.elapsed() >= stall_timeout {
                download.record.status = "Failed".into();
                download.record.error = Some(format!(
                    "Transfer stopped after {} seconds without data progress",
                    stall_timeout.as_secs()
                ));
                if let Some(op) = download.operation.take() {
                    let _ = unsafe { op.Cancel() };
                }
            }
        }
        state.items.iter().map(|d| d.record.clone()).collect()
    }
    pub fn cancel(&self, id: u64) -> Result<(), String> {
        let state = self.state.borrow();
        let download = state
            .items
            .iter()
            .find(|d| d.record.id == id)
            .ok_or("Unknown download ID")?;
        let operation = download
            .operation
            .as_ref()
            .ok_or("Download is no longer active")?;
        unsafe { operation.Cancel() }.map_err(|_| "Cannot cancel download".into())
    }
}

fn interruption_message(reason: COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON) -> String {
    let message = match reason {
        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_NO_SPACE => "Not enough disk space",
        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_ACCESS_DENIED => "Destination access denied",
        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_TOO_LARGE => {
            "File exceeds the destination limit"
        }
        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_TOO_SHORT
        | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_SERVER_CONTENT_LENGTH_MISMATCH => {
            "Incomplete response from the server"
        }
        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_NETWORK_DISCONNECTED => "Network disconnected",
        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_NETWORK_TIMEOUT => "Download timed out",
        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_NETWORK_FAILED => "Network transfer failed",
        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_SERVER_UNAUTHORIZED
        | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_SERVER_FORBIDDEN => {
            "Server refused the authenticated download"
        }
        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_BLOCKED_BY_POLICY
        | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_MALICIOUS
        | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_SECURITY_CHECK_FAILED => {
            "Browser security blocked this download"
        }
        _ => "Download interrupted",
    };
    format!("{message} (WebView2 {})", reason.0)
}
