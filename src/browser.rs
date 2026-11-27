//! Native WebView2 child window. Websites receive no native IPC capability.
use crate::i18n::tr;
mod capture;
mod control;
mod cookie_import;
mod downloads;
pub(crate) mod editor_page;
#[cfg(windows)]
pub(crate) mod html_trust;
mod private_fill;
mod project_page;
pub use private_fill::Reply as FillReply;
pub mod smoke;
pub mod vault_smoke;
use eframe::egui;
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender},
    },
};
use wry::{WebContext, WebView, WebViewBuilder};

enum Event {
    Title(String),
    Popup(String),
    Error(String),
    Navigation(String),
}

pub type DownloadSummaryRow = (u64, String, u64, Option<u64>, String, Option<String>);

fn next_address_generation() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.try_update(
        std::sync::atomic::Ordering::Relaxed,
        std::sync::atomic::Ordering::Relaxed,
        |id| id.checked_add(1),
    )
    .expect("browser address identity exhausted")
}

pub struct Browser {
    pub(crate) tab_id: u64,
    title: String,
    project_pages: project_page::Pages,
    trusted_preview_folders: std::collections::HashSet<html_trust::Folder>,
    address_generation: u64,
    // Drop child OAuth popups first, then main view, then shared WebContext environment.
    popups: crate::browser_popup::win32::PopupManager,
    view: Option<WebView>,
    // Drop view before its environment.
    context: Option<WebContext>,
    address: String,
    committed_address: String,
    address_editing: bool,
    popup: Option<String>,
    error: String,
    cookie_import_status: String,
    downloads: VecDeque<downloads::Record>,
    download_tracker: downloads::Downloads,
    capture: capture::Capture,
    destination: Arc<Mutex<PathBuf>>,
    sender: Sender<Event>,
    receiver: Receiver<Event>,
    downloads_open: bool,
    agent_root: Option<PathBuf>,
    agent_profile: Option<tempfile::TempDir>,
    agent_controlled: Arc<std::sync::atomic::AtomicBool>,
    planned_download: Arc<Mutex<Option<(PathBuf, std::time::Instant)>>>,
    clip: crate::browser_clip::NativeClip,
    viewport: Option<(egui::Rect, f32)>,
    webview_had_focus: bool,
    auto_start_pending: bool,
}

impl Default for Browser {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        let destination = directories::UserDirs::new()
            .and_then(|d| d.download_dir().map(|p| p.join("Tessel Rust")))
            .unwrap_or_else(|| crate::launch::default_directory().join("Downloads/Tessel Rust"));
        Self {
            tab_id: next_address_generation(),
            title: String::new(),
            project_pages: Default::default(),
            trusted_preview_folders: Default::default(),
            address_generation: next_address_generation(),
            popups: Default::default(),
            view: None,
            context: None,
            address: "https://example.com".into(),
            committed_address: "https://example.com".into(),
            address_editing: false,
            popup: None,
            error: String::new(),
            cookie_import_status: String::new(),
            downloads: VecDeque::new(),
            download_tracker: Default::default(),
            capture: Default::default(),
            destination: Arc::new(Mutex::new(destination)),
            sender,
            receiver,
            downloads_open: false,
            agent_root: None,
            agent_profile: None,
            agent_controlled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            planned_download: Default::default(),
            clip: Default::default(),
            viewport: None,
            webview_had_focus: false,
            auto_start_pending: false,
        }
    }
}

impl Browser {
    pub(crate) fn tab_label(&self) -> (String, String) {
        let pending = (self.view.is_none() && self.auto_start_pending && !self.address_editing)
            .then(|| crate::view_tabs_browser_model::allowed_browser_url(&self.address, true))
            .flatten();
        let url = crate::view_tabs_browser_model::display_url(
            pending.as_deref().unwrap_or(&self.committed_address),
        );
        let title = if !self.title.is_empty() {
            self.title.clone()
        } else if url.is_empty() {
            crate::i18n::tr("Browser").to_owned()
        } else {
            crate::view_tabs_browser_model::host_of(&url)
        };
        let hint = if url.is_empty() { title.clone() } else { url };
        (title, hint)
    }

    pub(crate) fn downloading(&self) -> bool {
        self.downloads
            .iter()
            .any(|item| item.status == "Downloading")
    }

    pub fn close_popups(&mut self) {
        self.popups.close_all();
    }
    pub fn active_url(&self) -> Option<&str> {
        self.view
            .as_ref()
            .map(|_| self.committed_address.as_str())
            .filter(|u| !u.trim().is_empty())
    }

    pub fn is_started(&self) -> bool {
        self.view.is_some()
    }

    pub fn address_str(&self) -> &str {
        &self.address
    }

    pub(crate) fn begin_native_address_edit(&mut self) {
        self.address_editing = true;
    }
    pub(crate) fn cancel_native_address_edit(&mut self) {
        if self.address_editing {
            self.address = self.committed_address.clone();
            self.address_editing = false;
            self.invalidate_address_input();
        }
    }
    fn observe_navigation(&mut self, address: String) {
        self.committed_address = address.clone();
        if !self.address_editing {
            self.address_generation = next_address_generation();
            self.address = address;
        }
    }
    #[cfg(all(test, feature = "visual-test"))]
    pub(crate) fn navigation_fixture(&mut self, address: &str) {
        self.sender
            .send(Event::Navigation(address.to_owned()))
            .unwrap();
        self.poll();
    }
    pub(crate) fn invalidate_address_input(&mut self) {
        self.address_generation = next_address_generation();
    }
    pub(crate) fn address_generation(&self) -> u64 {
        self.address_generation
    }
    pub(crate) fn set_native_address_draft(&mut self, address: String) {
        self.address_editing = true;
        self.address = address;
    }
    pub fn set_address_str(&mut self, address: String) {
        self.address_editing = false;
        self.address_generation = next_address_generation();
        self.address = address;
    }

    pub fn navigate_now(&mut self) {
        self.navigate();
    }

    pub fn go_back_now(&self) {
        if let Some(view) = &self.view {
            let _ = view.go_back();
        }
    }

    pub fn go_forward_now(&self) {
        if let Some(view) = &self.view {
            let _ = view.go_forward();
        }
    }

    pub fn reload_now(&self) {
        if let Some(view) = &self.view {
            let _ = view.reload();
        }
    }

    pub fn popup_url(&self) -> Option<&str> {
        self.popup.as_deref()
    }

    pub fn accept_popup_now(&mut self) {
        if let Some(url) = self.popup.take() {
            self.address = url;
            self.navigate();
        }
    }

    pub fn dismiss_popup_now(&mut self) {
        self.popup = None;
    }

    pub fn error_str(&self) -> &str {
        &self.error
    }

    pub fn downloads_open(&self) -> bool {
        self.downloads_open
    }

    pub fn toggle_downloads_open(&mut self) {
        self.downloads_open = !self.downloads_open;
    }

    pub fn download_summaries(&self) -> Vec<DownloadSummaryRow> {
        self.downloads
            .iter()
            .map(|d| {
                (
                    d.id,
                    d.status.clone(),
                    d.received_bytes,
                    d.total_bytes,
                    d.path.display().to_string(),
                    d.error.clone(),
                )
            })
            .collect()
    }

    pub fn cancel_download_now(&mut self, id: u64) {
        if let Err(e) = self.download_tracker.cancel(id) {
            self.error = e;
        }
    }

    pub fn start_for_parent<W: wry::raw_window_handle::HasWindowHandle>(&mut self, parent: &W) {
        if self.view.is_none() {
            self.auto_start_pending = false;
            if let Err(e) = self.create(parent) {
                self.error = e;
            }
        }
    }

    pub fn auto_start_if_pending<W: wry::raw_window_handle::HasWindowHandle>(
        &mut self,
        parent: &W,
    ) {
        if self.view.is_none() && self.auto_start_pending {
            self.auto_start_pending = false;
            if let Err(e) = self.create(parent) {
                self.error = e;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn sync_gpui_bounds(
        &mut self,
        x_logical: f32,
        y_logical: f32,
        w_logical: f32,
        h_logical: f32,
        scale: f32,
        can_show: bool,
        overlay_logical: Option<(f32, f32, f32, f32)>,
        reclaim_parent_focus: bool,
    ) -> bool {
        let rect = egui::Rect::from_min_size(
            egui::pos2(x_logical, y_logical),
            egui::vec2(w_logical.max(1.0), h_logical.max(1.0)),
        );
        self.viewport = Some((rect, scale));
        if let Some(view) = &self.view {
            let _ = view.set_bounds(wry::Rect {
                position: wry::dpi::PhysicalPosition::new(
                    (rect.min.x * scale) as i32,
                    (rect.min.y * scale) as i32,
                )
                .into(),
                size: wry::dpi::PhysicalSize::new(
                    (rect.width() * scale).max(1.0) as u32,
                    (rect.height() * scale).max(1.0) as u32,
                )
                .into(),
            });
        }
        let overlay = overlay_logical.map(|(ox, oy, ow, oh)| {
            egui::Rect::from_min_size(egui::pos2(ox, oy), egui::vec2(ow, oh))
        });
        self.sync_presentation(can_show, overlay, reclaim_parent_focus)
    }
}

fn allowed(address: &str) -> bool {
    address.len() <= 8192
        && crate::view_tabs_browser_model::allowed_browser_url(address, true).is_some()
}
fn safe_name(name: &str) -> String {
    let name: String = name
        .chars()
        .filter(|c| !c.is_control() && !"<>:\"/\\|?*".contains(*c))
        .take(160)
        .collect();
    let name = name.trim_matches(['.', ' ']);
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if name.is_empty()
        || [
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ]
        .contains(&stem.as_str())
    {
        "download.bin".into()
    } else {
        name.to_string()
    }
}
fn download_target(
    folder: &std::path::Path,
    suggested: &std::path::Path,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    // Each download gets a fresh directory; no existing user file is overwritten.
    let dir = tempfile::Builder::new()
        .prefix("download-")
        .tempdir_in(folder)
        .map_err(|e| e.to_string())?
        .keep();
    Ok(dir.join(safe_name(
        &suggested.file_name().unwrap_or_default().to_string_lossy(),
    )))
}

fn download_key(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| {
        path.parent()
            .and_then(|p| p.canonicalize().ok())
            .zip(path.file_name())
            .map(|(p, n)| p.join(n))
            .unwrap_or_else(|| path.clone())
    })
}
impl Browser {
    pub fn private_login_origin(&self) -> Result<String, String> {
        if self.agent_root.is_some() {
            return Err("The owner vault is unavailable to agent browsers".into());
        }
        let view = self
            .view
            .as_ref()
            .ok_or("Start the private browser and open the login page first")?;
        crate::vault::origin(&view.url().map_err(|_| "Cannot read browser location")?)
    }
    pub fn open_address(&mut self, address: String) {
        self.address = address;
        self.navigate();
    }

    pub fn fill_account(&self, account: &crate::vault::Account) -> Result<FillReply, String> {
        if account.origin.len() > 8192
            || account.username.len() > 1024
            || account.password.len() > 8192
        {
            return Err("Credential exceeds private fill limits".into());
        }
        if self.agent_root.is_some() {
            return Err("The owner vault is unavailable to agent browsers".into());
        }
        let view = self
            .view
            .as_ref()
            .ok_or("Start the private browser and open the login page first")?;
        let current = view.url().map_err(|_| "Cannot read browser location")?;
        if crate::vault::origin(&current)? != account.origin {
            return Err("The browser must be on this account's exact HTTPS site".into());
        }
        let capacity =
            (account.origin.len() + account.username.len() + account.password.len()) * 6 + 128;
        let mut payload = zeroize::Zeroizing::new(Vec::with_capacity(capacity));
        serde_json::to_writer(&mut *payload, account).map_err(|_| "Cannot prepare fill")?;
        let payload = std::str::from_utf8(&payload).map_err(|_| "Cannot encode fill")?;
        const CODE: &str = r#"(() => {
            'use strict';
            const account = __ACCOUNT__;
            if (location.origin !== account.origin || window !== window.top) return 'wrong-site';
            const visible = el => el.isConnected && !el.disabled && !el.readOnly && el.getClientRects().length > 0 && getComputedStyle(el).visibility === 'visible';
            const fields = [...document.querySelectorAll('input[type=password]')].filter(el => visible(el) && el.autocomplete !== 'new-password');
            if (fields.length !== 1) return fields.length ? 'ambiguous-form' : 'no-login-form';
            const password = fields[0];
            const form = password.form;
            const sameOrigin = raw => { try { return new URL(raw || location.href, document.baseURI).origin === account.origin; } catch { return false; } };
            const destinationsOk = f => {
                if (!f || !f.isConnected || f.ownerDocument !== document || !sameOrigin(f.getAttribute('action'))) return false;
                return [...document.querySelectorAll('button,input')].filter(el => el.form === f && ['submit','image'].includes(el.type) && el.hasAttribute('formaction')).every(el => sameOrigin(el.getAttribute('formaction')));
            };
            if (!destinationsOk(form)) return 'unsafe-form';
            const users = [...form.querySelectorAll('input[type=email],input[type=text],input:not([type])')].filter(el => visible(el) && el.form === form);
            if (users.length > 1) return 'ambiguous-form';
            const set = (el,value) => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(el,value); el.dispatchEvent(new Event('input',{bubbles:true})); el.dispatchEvent(new Event('change',{bubbles:true})); };
            if (users.length === 1) set(users[0],account.username);
            if (location.origin !== account.origin || !visible(password) || password.type !== 'password' || password.autocomplete === 'new-password' || password.form !== form || !destinationsOk(form)) return 'unsafe-form';
            set(password,account.password);
            return 'ok';
        })()"#;
        let (prefix, suffix) = CODE.split_once("__ACCOUNT__").expect("credential slot");
        let mut script = zeroize::Zeroizing::new(String::with_capacity(
            prefix.len() + payload.len() + suffix.len(),
        ));
        script.push_str(prefix);
        script.push_str(payload);
        script.push_str(suffix);
        private_fill::start(view, account.origin.clone(), script)
    }

    pub fn begin_frame(&mut self) {
        self.viewport = None;
    }

    pub fn visible(&mut self, visible: bool) {
        if !visible {
            self.clip.reset();
            self.viewport = None;
            if let Some(view) = &self.view {
                if webview_owns_focus(wry::WebViewExtWindows::hwnd(view)) {
                    let _ = view.focus_parent();
                }
                let _ = view.set_visible(false);
            }
            self.webview_had_focus = false;
        } else if let Some(view) = &self.view {
            let _ = view.set_visible(true);
        }
    }

    /// Synchronizes the child WebView's Win32 clipping region, visibility, and parent focus
    /// after all overlays, popups, and dialogs for the current frame have been evaluated.
    ///
    /// Returns `true` when the WebView newly acquired Win32 keyboard focus on a frame where
    /// `reclaim_parent_focus` was `false` (the user clicked inside the live webpage).
    pub fn sync_presentation(
        &mut self,
        can_show: bool,
        overlay: Option<egui::Rect>,
        reclaim_parent_focus: bool,
    ) -> bool {
        use wry::WebViewExtWindows;
        let Some((browser_rect, scale)) = self.viewport.filter(|_| can_show) else {
            self.visible(false);
            return false;
        };
        let Some(view) = &self.view else {
            self.clip.reset();
            self.webview_had_focus = false;
            return false;
        };
        let hwnd = view.hwnd();
        let cutout = match crate::browser_clip::cutout(browser_rect, overlay, scale) {
            Ok(cutout) => cutout,
            Err(_) => {
                self.visible(false);
                return false;
            }
        };
        if self.clip.apply(hwnd, cutout).is_err() {
            self.visible(false);
            return false;
        }
        let mut has_focus = webview_owns_focus(hwnd);
        let mut webview_took_focus = false;
        if reclaim_parent_focus && has_focus {
            let _ = view.focus_parent();
            has_focus = false;
        } else if has_focus && !self.webview_had_focus {
            webview_took_focus = true;
        }
        self.webview_had_focus = has_focus;
        let _ = view.set_visible(true);
        webview_took_focus
    }
    fn create<W: wry::raw_window_handle::HasWindowHandle>(
        &mut self,
        parent: &W,
    ) -> Result<(), String> {
        let dirs = directories::ProjectDirs::from("com", "Tessel", "TesselRust")
            .ok_or("No app data directory")?;
        self.create_with_profile(parent, dirs.data_local_dir().join("browser-owner"))?;
        self.navigate();
        Ok(())
    }
    fn create_with_profile<W: wry::raw_window_handle::HasWindowHandle>(
        &mut self,
        parent: &W,
        profile: PathBuf,
    ) -> Result<(), String> {
        self.popups.close_all();
        let parent_hwnd_raw = match parent.window_handle().map(|h| h.as_raw()) {
            Ok(wry::raw_window_handle::RawWindowHandle::Win32(raw)) => raw.hwnd.get(),
            _ => 0,
        };
        self.context = Some(WebContext::new(Some(profile)));
        let start = self.sender.clone();
        let popup = self.sender.clone();
        let navigation = self.sender.clone();
        let titles = self.sender.clone();
        let destination = self.destination.clone();
        let agent_root = self.agent_root.clone();
        let agent = agent_root.is_some();
        let agent_controlled = self.agent_controlled.clone();
        let popups_mgr = self.popups.clone();
        let gesture_tracker = std::rc::Rc::new(std::cell::RefCell::new(
            crate::browser_popup::PopupGestureTracker::new(),
        ));
        let gesture_for_handler = gesture_tracker.clone();
        let planned = self.planned_download.clone();
        let view = WebViewBuilder::new_with_web_context(
            self.context.as_mut().ok_or("Missing browser environment")?,
        );
        let view = if agent {
            view
        } else {
            self.project_pages.attach(view)?
        };
        let view = view
            .with_url("about:blank")
            .with_visible(false)
            .with_document_title_changed_handler(move |title| {
                let _ = titles.send(Event::Title(title));
            })
            .with_devtools(false)
            .with_general_autofill_enabled(false)
            .with_navigation_handler(move |address| {
                let permit = allowed(&address);
                if permit {
                    let _ = navigation.send(Event::Navigation(address));
                }
                permit
            })
            .with_new_window_req_handler(move |address, features| {
                let now_ms = crate::side_panels_model::now_unix_ms();
                let has_size = features.size.is_some();
                let is_controlled = agent_controlled.load(std::sync::atomic::Ordering::Acquire);
                let gesture_ok = gesture_for_handler.borrow_mut().consume_if_fresh(now_ms);
                if crate::browser_popup::should_open_oauth_popup(
                    &address,
                    gesture_ok,
                    has_size,
                    is_controlled,
                    agent,
                ) {
                    let owner_hwnd =
                        windows::Win32::Foundation::HWND(parent_hwnd_raw as *mut core::ffi::c_void);
                    let explicit_size = features.size.map(|s| (s.width, s.height));
                    return match popups_mgr.open_oauth_popup(
                        owner_hwnd,
                        &address,
                        explicit_size,
                        features.opener,
                    ) {
                        Ok(webview) => wry::NewWindowResponse::Create { webview },
                        Err(e) => {
                            let _ = popup.send(Event::Error(e));
                            wry::NewWindowResponse::Deny
                        }
                    };
                }
                if allowed(&address) {
                    let _ = popup.send(Event::Popup(address));
                }
                wry::NewWindowResponse::Deny
            })
            .build_as_child(parent)
            .map_err(|e| e.to_string())?;
        // Observe WebView2's native `IsUserInitiated` flag synchronously before `wry`'s deferred
        // `dispatch_handler` invokes `with_new_window_req_handler` on the next message-loop tick.
        unsafe {
            use wry::WebViewExtWindows as _;
            let _ = view.webview().add_NewWindowRequested(
                &webview2_com::NewWindowRequestedEventHandler::create(Box::new(move |_, args| {
                    if let Some(args) = args {
                        let mut user_initiated = windows::core::BOOL::from(false);
                        if args.IsUserInitiated(&mut user_initiated).is_ok()
                            && user_initiated.as_bool()
                        {
                            gesture_tracker
                                .borrow_mut()
                                .record_gesture(crate::side_panels_model::now_unix_ms());
                        }
                    }
                    Ok(())
                })),
                &mut 0,
            );
        }
        self.download_tracker.attach(
            &view,
            move |suggested| {
                let path = &suggested;
                if agent {
                    planned
                        .lock()
                        .map_err(|_| "Download settings unavailable".to_string())
                        .and_then(|mut plan| {
                            let (path, deadline) = plan
                                .take()
                                .ok_or("Prepare a download with the agent tool first")?;
                            if std::time::Instant::now() > deadline || path.exists() {
                                return Err(
                                    "Download plan expired or destination already exists".into()
                                );
                            }
                            let parent = path
                                .parent()
                                .ok_or("Download folder unavailable")?
                                .canonicalize()
                                .map_err(|_| "Download folder unavailable")?;
                            if agent_root
                                .as_ref()
                                .is_none_or(|root| !parent.starts_with(root))
                            {
                                return Err("Download folder left this project".into());
                            }
                            Ok(path)
                        })
                } else {
                    destination
                        .lock()
                        .map_err(|_| "Download settings unavailable".to_string())
                        .and_then(|folder| download_target(&folder, path))
                }
            },
            start,
        )?;
        self.clip.reset();
        self.viewport = None;
        self.webview_had_focus = false;
        self.view = Some(view);
        Ok(())
    }
    fn navigate(&mut self) {
        use crate::view_tabs_browser_model::{
            SearchEngine, allowed_browser_url, normalize_browser_input,
        };
        let address = (self.address.len() <= 8192)
            .then(|| {
                allowed_browser_url(&self.address, true)
                    .or_else(|| normalize_browser_input(&self.address, true, SearchEngine::Google))
            })
            .flatten();
        let Some(address) = address.filter(|address| allowed(address)) else {
            self.error = "Unsupported or oversized browser address.".into();
            return;
        };
        self.address_generation = next_address_generation();
        self.address = address.clone();
        self.committed_address = address.clone();
        self.address_editing = false;
        self.error.clear();
        if let Some(view) = &self.view {
            self.error = view
                .load_url(&address)
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default();
        }
    }
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        self.popups.prune_closed();
        self.downloads = self.download_tracker.poll();
        while let Ok(event) = self.receiver.try_recv() {
            changed = true;
            match event {
                Event::Title(title) => self.title = title.trim().chars().take(200).collect(),
                Event::Popup(address) => self.popup = Some(address),
                Event::Error(e) => self.error = e,
                Event::Navigation(address) => self.observe_navigation(address),
            }
        }
        changed
    }
    pub(crate) fn open_remote_page(
        &mut self,
        host: crate::workspace::Host,
        root: &str,
        file: &str,
        text: &str,
        scripts: bool,
    ) -> Result<(), String> {
        let folder = html_trust::Folder::Ssh {
            host: host.alias.clone(),
            root: root.into(),
        };
        let url = self
            .project_pages
            .for_open(scripts)
            .open_remote(host, root, file, text)?;
        if scripts {
            self.trusted_preview_folders.insert(folder);
        }
        self.open_url(&url);
        Ok(())
    }
    pub(crate) fn open_project_page(
        &mut self,
        doc: &crate::files::Document,
        scripts: bool,
    ) -> Result<(), String> {
        let root = doc.tools.html_root.as_ref().unwrap_or(&doc.root);
        let url = self
            .project_pages
            .for_open(scripts)
            .open(root, &doc.path, &doc.text)?;
        if scripts {
            self.trusted_preview_folders
                .insert(html_trust::Folder::Local(root.clone()));
        }
        self.open_url(&url);
        Ok(())
    }
    pub(crate) fn revoke_preview_scripts(&mut self, folder: &html_trust::Folder) {
        self.project_pages.restrict(folder);
        if self.trusted_preview_folders.remove(folder)
            && let Some(view) = self.view.take()
        {
            // A suspended history entry can retain its previous script environment.
            // Recreate the controller to discard live and cached documents together.
            let current = view
                .url()
                .ok()
                .filter(|url| allowed(url))
                .unwrap_or_else(|| self.committed_address.clone());
            self.popups.close_all();
            self.popup = None;
            (self.sender, self.receiver) = mpsc::channel();
            drop(view);
            self.context.take();
            self.clip.reset();
            self.viewport = None;
            self.webview_had_focus = false;
            self.open_url(&current);
        }
    }
    pub fn open_url(&mut self, url: &str) {
        self.address_editing = false;
        self.address_generation = next_address_generation();
        self.address = url.to_owned();

        self.title.clear();
        if self.view.is_some() {
            self.navigate();
        } else {
            self.auto_start_pending = true;
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        self.poll();
        if self.view.is_none() && self.auto_start_pending {
            self.auto_start_pending = false;
            if let Err(e) = self.create(frame) {
                self.error = e;
            }
        }
        if self.view.is_none() {
            ui.label(tr("Private browser profile for this Rust app. No cookies or passwords are imported from Tessel."));
            if ui.button(tr("Start browser")).clicked()
                && let Err(e) = self.create(frame)
            {
                self.error = e;
            }
            if !self.error.is_empty() {
                ui.colored_label(egui::Color32::LIGHT_RED, &self.error);
            }
            return;
        }
        let focus_url =
            ui.is_enabled() && ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::L));
        let reload_shortcut = ui.is_enabled()
            && ui.input_mut(|i| {
                i.consume_key(egui::Modifiers::CTRL, egui::Key::R)
                    || i.consume_key(egui::Modifiers::NONE, egui::Key::F5)
            });
        ui.horizontal(|ui| {
            if let Some(view) = &self.view {
                if ui.button("←").clicked() {
                    let _ = view.go_back();
                }
                if ui.button("→").clicked() {
                    let _ = view.go_forward();
                }
                if ui.button(tr("Reload")).clicked() || reload_shortcut {
                    let _ = view.reload();
                }
            }
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.address)
                    .desired_width((ui.available_width() - 140.0).max(100.0)),
            );
            if focus_url {
                response.request_focus();
            }
            if (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                || ui.button(tr("Go")).clicked()
            {
                self.navigate();
            }
            ui.toggle_value(
                &mut self.downloads_open,
                format!("{} ({})", tr("Downloads"), self.downloads.len()),
            );
        });
        if let Some(popup) = self.popup.clone() {
            ui.horizontal(|ui| {
                ui.label(tr("Page requested a new window."));
                if ui.button(tr("Open in this tab")).clicked() {
                    self.address = popup;
                    self.navigate();
                    self.popup = None;
                }
                if ui.button(tr("Dismiss")).clicked() {
                    self.popup = None;
                }
            });
        }
        if !self.error.is_empty() {
            ui.colored_label(egui::Color32::LIGHT_RED, &self.error);
        }
        if self.agent_root.is_none() {
            ui.horizontal(|ui| {
                if ui
                    .button(if crate::i18n::is_french() {
                        "Importer un fichier de cookies…"
                    } else {
                        "Import cookies from file…"
                    })
                    .clicked()
                {
                    self.import_cookie_file_now();
                }
                ui.label(&self.cookie_import_status);
            });
        }
        if self.downloads_open {
            if let Some(root) = &self.agent_root {
                ui.label(format!(
                    "{}: {}",
                    tr("Prepare agent downloads within project"),
                    root.display()
                ));
            } else if let Ok(folder) = self.destination.lock() {
                ui.label(format!(
                    "{}: {}",
                    tr("Save original files to"),
                    folder.display()
                ));
            }
            if self.agent_root.is_none() && ui.button(tr("Choose download folder")).clicked() {
                // File dialog is deliberately user initiated; no page can invoke it.
                if let Some(path) = rfd::FileDialog::new().pick_folder()
                    && let Ok(mut folder) = self.destination.lock()
                {
                    *folder = path;
                }
            }
            egui::ScrollArea::vertical()
                .max_height(160.0)
                .show(ui, |ui| {
                    for download in &self.downloads {
                        let path = &download.path;
                        let status = &download.status;
                        ui.horizontal(|ui| {
                            ui.label(tr(status));
                            ui.label(match download.total_bytes {
                                Some(total) => format!("{} / {} B", download.received_bytes, total),
                                None => format!("{} B", download.received_bytes),
                            });
                            if status == "Downloading"
                                && ui.small_button(tr("Cancel")).clicked()
                                && let Err(e) = self.download_tracker.cancel(download.id)
                            {
                                self.error = e;
                            }
                            if let Some(error) = &download.error {
                                ui.colored_label(egui::Color32::LIGHT_RED, error);
                            }
                            ui.label(path.display().to_string());
                            if ui.small_button(tr("Copy path")).clicked() {
                                ui.ctx().copy_text(path.display().to_string());
                            }
                        });
                    }
                });
        }
        if self.downloads.iter().any(|d| d.status == "Downloading") {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(200));
        }
        let rect = ui.available_rect_before_wrap();
        let scale = ui.ctx().pixels_per_point();
        self.viewport = Some((rect, scale));
        if let Some(view) = &self.view {
            let _ = view.set_bounds(wry::Rect {
                position: wry::dpi::PhysicalPosition::new(
                    (rect.min.x * scale) as i32,
                    (rect.min.y * scale) as i32,
                )
                .into(),
                size: wry::dpi::PhysicalSize::new(
                    (rect.width() * scale).max(1.0) as u32,
                    (rect.height() * scale).max(1.0) as u32,
                )
                .into(),
            });
        }
    }
}

fn webview_owns_focus(hwnd: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GUITHREADINFO, GetGUIThreadInfo, IsChild};
    if hwnd.0.is_null() {
        return false;
    }
    let mut info = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    if unsafe { GetGUIThreadInfo(0, &mut info) }.is_err() || info.hwndFocus.0.is_null() {
        return false;
    }
    info.hwndFocus == hwnd || unsafe { IsChild(hwnd, info.hwndFocus) }.as_bool()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires installed Windows WebView2 runtime"]
    fn project_switch_keeps_both_native_browser_documents_alive() {
        use crate::app::{Tessel, gpui_bridge::GpuiAction};
        use crate::webview_test_support::{hidden_parent, wait_for};

        let dir = tempfile::tempdir().unwrap();
        let parent = hidden_parent();
        let profile = dir.path().join("test-owner");
        let mut app = Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
        let mut projects = Vec::new();
        for name in ["one", "two"] {
            let site = dir.path().join(name);
            std::fs::create_dir(&site).unwrap();
            let path = site.join("index.html");
            std::fs::write(&path, format!("<title>{name}</title><input id='draft'>")).unwrap();
            let doc = crate::files::Document::open(&site, &path).unwrap();
            app.apply_gpui_action(GpuiAction::AddProjectPath(site));
            projects.push(app.state.active_project.unwrap());
            app.apply_gpui_action(GpuiAction::SelectBrowserTab);
            app.browser
                .create_with_profile(&parent, profile.clone())
                .unwrap();
            app.browser.open_project_page(&doc, false).unwrap();
            let view = app.browser.view.as_ref().unwrap();
            wait_for(
                view,
                &format!("document.title === '{name}' && !!document.querySelector('#draft')"),
            );
            wait_for(
                view,
                &format!(
                    "(document.querySelector('#draft').value = '{name} draft') === '{name} draft'"
                ),
            );
        }
        for (id, name) in projects.into_iter().zip(["one", "two"]) {
            app.apply_gpui_action(GpuiAction::SelectProject(id));
            app.apply_gpui_action(GpuiAction::SelectBrowserTab);
            wait_for(
                app.browser.view.as_ref().unwrap(),
                &format!(
                    "document.title === '{name}' && document.querySelector('#draft').value === '{name} draft'"
                ),
            );
        }
    }

    #[test]
    #[ignore = "requires installed Windows WebView2 runtime"]
    fn project_switch_finishes_in_flight_download_in_original_folder() {
        use crate::app::{Tessel, gpui_bridge::GpuiAction};
        use crate::webview_test_support::{hidden_parent, pump};
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;
        use std::time::{Duration, Instant};
        let temp = tempfile::tempdir().unwrap();
        let parent = hidden_parent();
        let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().join("state")), false, || {});
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        app.apply_gpui_action(GpuiAction::AddProjectPath(first.clone()));
        let owner = app.state.active_project.unwrap();
        app.apply_gpui_action(GpuiAction::SelectBrowserTab);
        app.browser
            .create_with_profile(&parent, temp.path().join("profile"))
            .unwrap();
        *app.browser.destination.lock().unwrap() = first.clone();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/transfer.bin", listener.local_addr().unwrap());
        let (release, wait) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            let deadline = Instant::now() + Duration::from_secs(25);
            let mut stream = loop {
                assert!(Instant::now() < deadline, "no attachment request received");
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(e) => panic!("fixture accept failed: {e}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_millis(500)))
                    .unwrap();
                let mut request = String::new();
                if BufReader::new(&stream).read_line(&mut request).is_err() || request.is_empty() {
                    continue;
                }
                if request.starts_with("GET ")
                    && request
                        .split_whitespace()
                        .nth(1)
                        .is_some_and(|path| path.ends_with("/transfer.bin"))
                {
                    break stream;
                }
                let _ = stream.write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            };
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=transfer.bin\r\nContent-Length: 32768\r\nConnection: close\r\n\r\n").unwrap();
            stream.write_all(&[7; 16384]).unwrap();
            wait.recv_timeout(Duration::from_secs(30)).unwrap();
            stream.write_all(&[8; 16384]).unwrap();
        });
        app.browser.open_url(&url);
        let deadline = Instant::now() + Duration::from_secs(20);
        while !app
            .browser
            .download_summaries()
            .iter()
            .any(|row| row.1 == "Downloading")
        {
            assert!(
                Instant::now() < deadline,
                "WebView2 did not start the attachment"
            );
            pump();
            app.poll_for_gpui();
        }
        let file = PathBuf::from(&app.browser.download_summaries()[0].4);
        assert!(file.starts_with(first.canonicalize().unwrap()));
        assert_eq!(file.file_name().unwrap(), "transfer.bin");
        app.apply_gpui_action(GpuiAction::AddProjectPath(second));
        assert!(app.browser.download_summaries().is_empty());
        release.send(()).unwrap();
        server.join().unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        let expected = [&[7; 16384][..], &[8; 16384][..]].concat();
        while !std::fs::read(&file).is_ok_and(|bytes| bytes == expected) {
            assert!(
                Instant::now() < deadline,
                "background transfer did not finish with the expected bytes"
            );
            pump();
            app.poll_for_gpui();
        }
        app.apply_gpui_action(GpuiAction::SelectProject(owner));
        loop {
            pump();
            app.poll_for_gpui();
            if app
                .browser
                .download_summaries()
                .iter()
                .any(|row| row.1 == "Completed" && row.2 == 32768)
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "owner did not receive the completed record"
            );
        }
    }

    #[test]
    #[ignore = "requires installed Windows WebView2 runtime"]
    fn browser_tabs_in_one_project_keep_live_pages_and_titles() {
        use crate::app::{Tessel, gpui_bridge::GpuiAction};
        use crate::webview_test_support::{hidden_parent, pump, wait_for};
        let dir = tempfile::tempdir().unwrap();
        let parent = hidden_parent();
        let site = dir.path().join("site");
        std::fs::create_dir(&site).unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
        app.apply_gpui_action(GpuiAction::AddProjectPath(site.clone()));
        let mut ids = Vec::new();
        for name in ["first", "second", "third"] {
            app.execute_palette_command(&eframe::egui::Context::default(), "browser");
            ids.push(app.browser.tab_id);
            assert_eq!(app.browser.tab_label().0, crate::i18n::tr("Browser"));
            let path = site.join(format!("{name}.html"));
            std::fs::write(&path, format!("<title>{name}</title><input id='draft'>")).unwrap();
            let doc = crate::files::Document::open(&site, &path).unwrap();
            app.browser
                .create_with_profile(&parent, dir.path().join("owner-profile"))
                .unwrap();
            app.browser.open_project_page(&doc, false).unwrap();
            wait_for(
                app.browser.view.as_ref().unwrap(),
                &format!(
                    "document.title === '{name}' && (document.querySelector('#draft').value = '{name}!') === '{name}!'"
                ),
            );
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while app.browser.tab_label().0 != name {
                assert!(std::time::Instant::now() < deadline);
                pump();
                app.poll_for_gpui();
            }
        }
        let tabs = app.gpui_snapshot(960., 640., 4.).view_tabs;
        assert_eq!(
            tabs.iter()
                .filter_map(|tab| tab.browser_id.map(|id| (id, tab.title.as_str())))
                .collect::<Vec<_>>(),
            [(ids[0], "first"), (ids[1], "second"), (ids[2], "third")]
        );
        app.apply_gpui_action(GpuiAction::SelectBrowserPage(ids[0]));
        wait_for(
            app.browser.view.as_ref().unwrap(),
            "document.querySelector('#draft').value === 'first!'",
        );
        app.apply_gpui_action(GpuiAction::CloseBrowserPage(ids[2]));
        assert_eq!(app.browser.tab_id, ids[0]);
        app.apply_gpui_action(GpuiAction::CloseBrowserPage(ids[0]));
        assert_eq!(app.browser.tab_id, ids[1]);
        wait_for(
            app.browser.view.as_ref().unwrap(),
            "document.querySelector('#draft').value === 'second!'",
        );
    }

    #[test]
    fn address_bar_normalizes_without_opening_a_browser_or_leaking_credentials() {
        for (input, expected) in [
            ("localhost:5173/test", "http://localhost:5173/test"),
            ("example.org", "https://example.org/"),
            ("", "about:blank"),
            (
                "rust native ui",
                "https://www.google.com/search?q=rust%20native%20ui",
            ),
            ("blob:https://example.org/id", "blob:https://example.org/id"),
        ] {
            let mut browser = Browser {
                address: input.into(),
                ..Default::default()
            };
            browser.navigate();
            assert_eq!(browser.address, expected);
            assert!(browser.error.is_empty());
            assert!(browser.view.is_none());
        }
        for input in [
            "https://user:secret@example.org",
            "javascript:alert(1)",
            "file:///C:/private",
            "data:text/plain,secret",
        ] {
            let mut browser = Browser {
                address: input.into(),
                ..Default::default()
            };
            browser.navigate();
            assert!(!browser.error.is_empty());
            assert!(!browser.error.contains("secret"));
            assert!(browser.view.is_none());
        }
    }
    #[test]
    fn downloads_do_not_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let first = download_target(dir.path(), std::path::Path::new("report.pdf")).unwrap();
        std::fs::write(&first, "original").unwrap();
        let second = download_target(dir.path(), std::path::Path::new("report.pdf")).unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read_to_string(first).unwrap(), "original");
    }
    #[test]
    fn denies_native_schemes() {
        for address in [
            "file:///C:/secret",
            "javascript:alert(1)",
            "tessel://tool",
            "https://user:secret@example.org",
            "https://user@example.org",
        ] {
            assert!(!allowed(address));
        }
        assert!(allowed("https://example.org"));
        assert_eq!(safe_name("CON.pdf"), "download.bin");
        assert!(!safe_name("../../evil.exe").contains('/'));
    }
}
