//! Hardened OAuth sign-in popup window logic and Win32 WebView2 popup host (`a93791f3`).
//!
//! Websites receive zero native IPC capability. Only user-initiated (`<= 1500 ms`, single-use
//! per click), explicitly sized (`width=` / `height=` / `innerWidth=` / `innerHeight=`),
//! `http://` or `https://` requests from the owner's private browser (never agent-controlled or
//! agent-profile browsers) open an owned Win32 popup sharing the opener's WebView2 environment
//! so `window.opener` and `window.close()` complete OAuth sign-in flows cleanly.

#![allow(dead_code)]

/// Maximum age in milliseconds for a user click/keypress gesture to authorize one popup.
pub const POPUP_GESTURE_MS: u64 = 1_500;
/// Default popup client width when only height is specified.
pub const POPUP_DEFAULT_WIDTH: u32 = 600;
/// Default popup client height when only width is specified.
pub const POPUP_DEFAULT_HEIGHT: u32 = 700;
/// Minimum popup client dimension (width and height).
pub const POPUP_MIN_SIZE: u32 = 300;
/// Maximum popup client width.
pub const POPUP_MAX_WIDTH: u32 = 1_400;
/// Maximum popup client height.
pub const POPUP_MAX_HEIGHT: u32 = 1_000;

/// Outer screen rectangle of the parent Tessel window used to center OAuth popups.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParentRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Clamped and centered popup target rectangle (client width/height + centered top-left).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PopupRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Tracks a user input gesture and guarantees at most **one** popup opens per click within `1500 ms`.
#[derive(Clone, Debug, Default)]
pub struct PopupGestureTracker {
    gesture_at_ms: Option<u64>,
    last_consumed_at_ms: Option<u64>,
}

impl PopupGestureTracker {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            gesture_at_ms: None,
            last_consumed_at_ms: None,
        }
    }

    pub fn record_gesture(&mut self, now_ms: u64) {
        // Prevent a single click handler calling `window.open` twice in the same burst from
        // re-arming immediately after the first popup consumed the click gesture.
        if self
            .last_consumed_at_ms
            .is_some_and(|prev| now_ms.saturating_sub(prev) < 250)
        {
            return;
        }
        self.gesture_at_ms = Some(now_ms);
    }

    pub fn clear(&mut self) {
        self.gesture_at_ms = None;
    }

    /// Consumes the recorded gesture if it occurred within [`POPUP_GESTURE_MS`] of `now_ms`.
    pub fn consume_if_fresh(&mut self, now_ms: u64) -> bool {
        let Some(at) = self.gesture_at_ms.take() else {
            return false;
        };
        if now_ms.saturating_sub(at) <= POPUP_GESTURE_MS {
            self.last_consumed_at_ms = Some(now_ms);
            true
        } else {
            false
        }
    }
}

/// Matches `(?:^|[\s,;])(?:inner)?(?:width|height)\s*=` case-insensitively on a raw `window.open`
/// features string (`a93791f3`).
#[must_use]
pub fn has_popup_size_feature(features: &str) -> bool {
    let bytes = features.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let at_delimiter = i == 0
            || matches!(
                bytes[i - 1],
                b' ' | b'\t' | b'\r' | b'\n' | b'\x0b' | b'\x0c' | b',' | b';'
            );
        if at_delimiter {
            let rest = &features[i..];
            let after_inner = strip_ascii_prefix_ci(rest, "inner").unwrap_or(rest);
            if let Some(after_dim) = strip_ascii_prefix_ci(after_inner, "width")
                .or_else(|| strip_ascii_prefix_ci(after_inner, "height"))
            {
                let trimmed = after_dim.trim_start_matches(|c: char| c.is_ascii_whitespace());
                if trimmed.starts_with('=') {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

/// Extracts optional `(width, height)` numbers from a raw `window.open` features string.
#[must_use]
pub fn parse_popup_feature_dimensions(features: &str) -> (Option<f64>, Option<f64>) {
    let mut width = None;
    let mut height = None;
    for raw_token in features.split([',', ';']) {
        let token = raw_token.trim();
        let Some((raw_key, raw_val)) = token.split_once('=') else {
            continue;
        };
        let key = raw_key.trim().to_ascii_lowercase();
        let val_prefix: String = raw_val
            .trim()
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '+')
            .collect();
        let Ok(parsed) = val_prefix.parse::<f64>() else {
            continue;
        };
        match key.as_str() {
            "width" | "innerwidth" => width = Some(parsed),
            "height" | "innerheight" => height = Some(parsed),
            _ => {}
        }
    }
    (width, height)
}

fn clamp_dimension(raw: Option<f64>, default_px: u32, max_px: u32) -> u32 {
    match raw {
        Some(v) if v.is_finite() && v > 0.0 => {
            v.round().clamp(POPUP_MIN_SIZE as f64, max_px as f64) as u32
        }
        _ => default_px,
    }
}

/// Computes the clamped (`300..=1400` × `300..=1000`, default `600 × 700`) and parent-centered
/// popup geometry (`a93791f3`).
#[must_use]
pub fn popup_window_rect(
    explicit_size: Option<(f64, f64)>,
    features_str: Option<&str>,
    parent: Option<ParentRect>,
) -> PopupRect {
    let parsed_dims = features_str
        .map(parse_popup_feature_dimensions)
        .unwrap_or((None, None));
    let req_w = explicit_size.map(|(w, _)| w).or(parsed_dims.0);
    let req_h = explicit_size.map(|(_, h)| h).or(parsed_dims.1);
    let width = clamp_dimension(req_w, POPUP_DEFAULT_WIDTH, POPUP_MAX_WIDTH);
    let height = clamp_dimension(req_h, POPUP_DEFAULT_HEIGHT, POPUP_MAX_HEIGHT);
    let (x, y) = if let Some(p) = parent {
        let cx = (p.x as f64 + (p.width as f64 - width as f64) / 2.0).round() as i32;
        let cy = (p.y as f64 + (p.height as f64 - height as f64) / 2.0).round() as i32;
        (cx, cy)
    } else {
        (100, 100)
    };
    PopupRect {
        x,
        y,
        width,
        height,
    }
}

/// Returns `true` only for `http://` and `https://` URLs (`<= 8192` bytes) without embedded
/// credentials (never `about:blank`, `javascript:`, `data:`, `file:`, `blob:`, or custom schemes).
#[must_use]
pub fn is_popup_url_allowed(address: &str) -> bool {
    if address.is_empty() || address.len() > 8192 {
        return false;
    }
    let Ok(parsed) = url::Url::parse(address) else {
        return false;
    };
    matches!(parsed.scheme(), "http" | "https")
        && parsed.host_str().is_some()
        && parsed.username().is_empty()
        && parsed.password().is_none()
}

/// Gates whether a `window.open` request opens a real OAuth child window vs staying in-pane (`a93791f3`).
#[must_use]
pub fn should_open_oauth_popup(
    url: &str,
    user_gesture: bool,
    has_size: bool,
    agent_controlled: bool,
    is_agent_browser: bool,
) -> bool {
    user_gesture && has_size && !agent_controlled && !is_agent_browser && is_popup_url_allowed(url)
}

fn strip_ascii_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() < prefix.len() || !s.is_char_boundary(prefix.len()) {
        return None;
    }
    if s[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

#[cfg(windows)]
pub mod win32 {
    use super::{ParentRect, is_popup_url_allowed, popup_window_rect};
    use std::{cell::RefCell, num::NonZeroIsize, rc::Rc, sync::OnceLock};
    use webview2_com::{
        DownloadStartingEventHandler, Microsoft::Web::WebView2::Win32::*,
        NavigationStartingEventHandler, take_pwstr,
    };
    use windows::{
        Win32::{
            Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
            UI::WindowsAndMessaging::{
                AdjustWindowRectEx, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow,
                GA_ROOT, GetAncestor, GetWindowRect, IDC_ARROW, IsWindow, LoadCursorW,
                RegisterClassExW, SW_SHOWNORMAL, SetWindowTextW, ShowWindow, WINDOW_EX_STYLE,
                WM_CLOSE, WNDCLASSEXW, WS_CLIPCHILDREN, WS_OVERLAPPEDWINDOW,
            },
        },
        core::{HSTRING, Interface, PCWSTR, PWSTR, w},
    };
    use wry::{
        NewWindowOpener, NewWindowResponse, WebView, WebViewBuilder, WebViewBuilderExtWindows,
        WebViewExtWindows,
        raw_window_handle::{
            HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle,
        },
    };

    struct PopupWindowHandle(HWND);

    impl HasWindowHandle for PopupWindowHandle {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            let nz = NonZeroIsize::new(self.0.0 as isize).ok_or(HandleError::Unavailable)?;
            let raw = Win32WindowHandle::new(nz);
            Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(raw)) })
        }
    }

    struct ActivePopup {
        hwnd: HWND,
        _webview: WebView,
    }

    /// Owns all live child OAuth popup windows spawned by a [`crate::browser::Browser`] instance.
    /// Dropping [`PopupManager`] destroys every child popup `HWND` immediately (`DestroyWindow`),
    /// ensuring `beforeunload` handlers in untrusted pages cannot keep popups alive.
    #[derive(Clone, Default)]
    pub struct PopupManager {
        entries: Rc<RefCell<Vec<ActivePopup>>>,
    }

    impl Drop for PopupManager {
        fn drop(&mut self) {
            if Rc::strong_count(&self.entries) == 1 {
                self.close_all();
            }
        }
    }

    impl PopupManager {
        pub fn prune_closed(&self) {
            if let Ok(mut list) = self.entries.try_borrow_mut() {
                list.retain(|entry| unsafe { IsWindow(Some(entry.hwnd)).as_bool() });
            }
        }

        pub fn close_all(&self) {
            if let Ok(mut list) = self.entries.try_borrow_mut() {
                for entry in list.drain(..) {
                    if unsafe { IsWindow(Some(entry.hwnd)).as_bool() } {
                        let _ = unsafe { DestroyWindow(entry.hwnd) };
                    }
                }
            }
        }

        pub fn open_oauth_popup(
            &self,
            owner_or_child_hwnd: HWND,
            url: &str,
            explicit_size: Option<(f64, f64)>,
            opener: NewWindowOpener,
        ) -> Result<ICoreWebView2, String> {
            if !is_popup_url_allowed(url) {
                return Err("Disallowed OAuth popup URL".into());
            }
            self.prune_closed();
            let class_name = ensure_popup_window_class()?;
            let owner_hwnd = resolve_root_hwnd(owner_or_child_hwnd);
            let parent_bounds = read_window_rect(owner_hwnd);
            let target = popup_window_rect(explicit_size, None, parent_bounds);

            let style = WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN;
            let ex_style = WINDOW_EX_STYLE(0);
            let mut outer = RECT {
                left: target.x,
                top: target.y,
                right: target.x.saturating_add(target.width as i32),
                bottom: target.y.saturating_add(target.height as i32),
            };
            let _ = unsafe { AdjustWindowRectEx(&mut outer, style, false, ex_style) };
            let outer_w = (outer.right - outer.left).max(super::POPUP_MIN_SIZE as i32);
            let outer_h = (outer.bottom - outer.top).max(super::POPUP_MIN_SIZE as i32);
            let (win_x, win_y) = if let Some(p) = parent_bounds {
                (
                    (p.x as f64 + (p.width as f64 - outer_w as f64) / 2.0).round() as i32,
                    (p.y as f64 + (p.height as f64 - outer_h as f64) / 2.0).round() as i32,
                )
            } else {
                (CW_USEDEFAULT, CW_USEDEFAULT)
            };

            let initial_title = popup_title(url, "");
            let popup_hwnd = unsafe {
                CreateWindowExW(
                    ex_style,
                    class_name,
                    &HSTRING::from(initial_title),
                    style,
                    win_x,
                    win_y,
                    outer_w,
                    outer_h,
                    (!owner_hwnd.0.is_null()).then_some(owner_hwnd),
                    None,
                    None,
                    None,
                )
            }
            .map_err(|e| format!("Cannot create OAuth popup window: {e}"))?;

            let title_hwnd_raw = popup_hwnd.0 as isize;
            let initial_url_for_title = url.to_owned();
            let build_result = WebViewBuilder::new()
                .with_environment(opener.environment)
                .with_visible(true)
                .with_devtools(false)
                .with_general_autofill_enabled(false)
                .with_document_title_changed_handler(move |doc_title| {
                    let hwnd = HWND(title_hwnd_raw as *mut core::ffi::c_void);
                    if unsafe { IsWindow(Some(hwnd)).as_bool() } {
                        let caption = popup_title(&initial_url_for_title, &doc_title);
                        let _ = unsafe { SetWindowTextW(hwnd, &HSTRING::from(caption)) };
                    }
                })
                .with_navigation_handler(move |next_url| {
                    let ok = is_popup_url_allowed(&next_url);
                    if ok {
                        let hwnd = HWND(title_hwnd_raw as *mut core::ffi::c_void);
                        if unsafe { IsWindow(Some(hwnd)).as_bool() } {
                            let caption = popup_title(&next_url, "");
                            let _ = unsafe { SetWindowTextW(hwnd, &HSTRING::from(caption)) };
                        }
                    }
                    ok
                })
                .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
                .build(&PopupWindowHandle(popup_hwnd));

            let popup_view = match build_result {
                Ok(v) => v,
                Err(err) => {
                    let _ = unsafe { DestroyWindow(popup_hwnd) };
                    return Err(format!("Cannot initialize OAuth popup WebView: {err}"));
                }
            };

            if let Err(err) = harden_popup_webview(&popup_view) {
                drop(popup_view);
                let _ = unsafe { DestroyWindow(popup_hwnd) };
                return Err(err);
            }

            let _ = unsafe { ShowWindow(popup_hwnd, SW_SHOWNORMAL) };
            let core_webview = popup_view.webview();
            self.entries.borrow_mut().push(ActivePopup {
                hwnd: popup_hwnd,
                _webview: popup_view,
            });
            Ok(core_webview)
        }
    }

    fn harden_popup_webview(view: &WebView) -> Result<(), String> {
        unsafe {
            let core = view.webview();
            core.add_FrameNavigationStarting(
                &NavigationStartingEventHandler::create(Box::new(|_, args| {
                    let Some(args) = args else {
                        return Ok(());
                    };
                    let mut raw = PWSTR::null();
                    args.Uri(&mut raw)?;
                    let uri = take_pwstr(raw);
                    if !uri.eq_ignore_ascii_case("about:blank") && !is_popup_url_allowed(&uri) {
                        args.SetCancel(true)?;
                    }
                    Ok(())
                })),
                &mut 0,
            )
            .map_err(|_| "Cannot harden OAuth popup subframe navigation".to_string())?;

            let webview4: ICoreWebView2_4 = core
                .cast()
                .map_err(|_| "Cannot access OAuth popup download guard".to_string())?;
            webview4
                .add_DownloadStarting(
                    &DownloadStartingEventHandler::create(Box::new(|_, args| {
                        if let Some(args) = args {
                            args.SetCancel(true)?;
                            args.SetHandled(true)?;
                        }
                        Ok(())
                    })),
                    &mut 0,
                )
                .map_err(|_| "Cannot attach OAuth popup download guard".to_string())?;
        }
        Ok(())
    }

    fn popup_title(url: &str, doc_title: &str) -> String {
        let origin = url::Url::parse(url)
            .ok()
            .map(|u| u.origin().ascii_serialization())
            .filter(|o| o != "null")
            .unwrap_or_else(|| "Sign in".to_owned());
        let clean_title: String = doc_title
            .chars()
            .filter(|c| !c.is_control())
            .take(120)
            .collect();
        let trimmed = clean_title.trim();
        if trimmed.is_empty() {
            format!("{origin} — Tessel")
        } else {
            format!("{origin} — {trimmed}")
        }
    }

    fn resolve_root_hwnd(hwnd: HWND) -> HWND {
        if hwnd.0.is_null() {
            return hwnd;
        }
        let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
        if root.0.is_null() { hwnd } else { root }
    }

    fn read_window_rect(hwnd: HWND) -> Option<ParentRect> {
        if hwnd.0.is_null() {
            return None;
        }
        let mut rect = RECT::default();
        unsafe { GetWindowRect(hwnd, &mut rect).ok()? };
        let w = (rect.right - rect.left).max(1) as u32;
        let h = (rect.bottom - rect.top).max(1) as u32;
        Some(ParentRect {
            x: rect.left,
            y: rect.top,
            width: w,
            height: h,
        })
    }

    unsafe extern "system" fn popup_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if msg == WM_CLOSE {
            // Destroy directly so an untrusted page's `beforeunload` handler cannot trap closing.
            let _ = unsafe { DestroyWindow(hwnd) };
            return LRESULT(0);
        }
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    fn ensure_popup_window_class() -> Result<PCWSTR, String> {
        static REGISTERED: OnceLock<Result<(), String>> = OnceLock::new();
        const CLASS_NAME: PCWSTR = w!("TesselOAuthPopupWindow");
        let res = REGISTERED.get_or_init(|| unsafe {
            let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
            let wc = WNDCLASSEXW {
                cbSize: core::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(popup_wnd_proc),
                hCursor: cursor,
                lpszClassName: CLASS_NAME,
                ..Default::default()
            };
            if RegisterClassExW(&wc) == 0 {
                Err("Failed to register OAuth popup window class".into())
            } else {
                Ok(())
            }
        });
        res.clone().map(|()| CLASS_NAME)
    }
}
