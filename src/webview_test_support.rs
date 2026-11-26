use std::{
    num::NonZeroIsize,
    sync::mpsc,
    time::{Duration, Instant},
};
use windows::{
    Win32::{Foundation::HWND, UI::WindowsAndMessaging::*},
    core::w,
};
use wry::raw_window_handle::{
    HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle,
};

pub(crate) struct Parent(HWND);
impl Drop for Parent {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}
impl HasWindowHandle for Parent {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let raw = Win32WindowHandle::new(NonZeroIsize::new(self.0.0 as isize).unwrap());
        // The HWND belongs to Parent and outlives the borrowed handle.
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(raw)) })
    }
}

pub(crate) fn pump() {
    let mut msg = MSG::default();
    unsafe {
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    std::thread::sleep(Duration::from_millis(5));
}

pub(crate) fn wait_for(view: &wry::WebView, expression: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut last = String::new();
    while Instant::now() < deadline {
        let (tx, rx) = mpsc::channel();
        view.evaluate_script_with_callback(expression, move |s| {
            let _ = tx.send(s);
        })
        .unwrap();
        while Instant::now() < deadline {
            pump();
            if let Ok(value) = rx.try_recv() {
                if value == "true" {
                    return;
                }
                last = value;
                break;
            }
        }
    }
    panic!("WebView2 probe timed out: {expression}; last result {last}");
}

pub(crate) fn hidden_parent() -> Parent {
    Parent(
        unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("Tessel preview fixture"),
                WS_POPUP | WS_CLIPCHILDREN,
                0,
                0,
                400,
                300,
                None,
                None,
                None,
                None,
            )
        }
        .unwrap(),
    )
}
