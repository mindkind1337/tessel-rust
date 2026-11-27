//! Native WebView2 child window. Websites receive no native IPC capability.
use crate::i18n::tr;
mod audio;
mod capture;
mod control;
mod cookie_import;
mod downloads;
pub(crate) mod editor_page;
#[cfg(windows)]
pub(crate) mod html_trust;
mod private_fill;
mod project_page;
mod zoom_keys;
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
    Zoom(i8),
    PlayingAudio(bool),
    Popup(String),
    Error(String),
    Navigation(String),
}

use crate::app::gpui_bridge::DownloadSummaryRow;

fn next_address_generation() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.try_update(
        std::sync::atomic::Ordering::Relaxed,
        std::sync::atomic::Ordering::Relaxed,
        |id| id.checked_add(1),
    )
    .expect("browser address identity exhausted")
}

struct FinishingView {
    downloads: Vec<u64>,
    // Drop the controller before the environment that created it.
    _view: WebView,
    _context: Option<WebContext>,
}

pub struct Browser {
    pub(crate) tab_id: u64,
    pub(crate) tab_order: u64,
    pub(crate) view_worktree: Option<PathBuf>,
    title: String,
    pub(crate) zoom_override: Option<i8>,
    default_zoom: i8,
    muted: bool,
    playing_audio: bool,
    project_pages: project_page::Pages,
    trusted_preview_folders: std::collections::HashSet<html_trust::Folder>,
    address_generation: u64,
    // Drop child OAuth popups first, then main view, then shared WebContext environment.
    popups: crate::browser_popup::win32::PopupManager,
    view: Option<WebView>,
    // Drop view before its environment.
    context: Option<WebContext>,
    finishing_views: Vec<FinishingView>,
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
        let tab_id = next_address_generation();
        Self {
            tab_id,
            tab_order: tab_id,
            view_worktree: None,
            title: String::new(),
            zoom_override: None,
            muted: false,
            playing_audio: false,
            default_zoom: crate::workspace::BrowserPreferences::default().zoom_half_steps,
            project_pages: Default::default(),
            trusted_preview_folders: Default::default(),
            address_generation: next_address_generation(),
            popups: Default::default(),
            view: None,
            context: None,
            finishing_views: Vec::new(),
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
    pub(crate) fn change_presentation(&mut self, to_side: bool) {
        self.poll();
        let address = self
            .view
            .as_ref()
            .and_then(|view| view.url().ok())
            .unwrap_or_else(|| self.current_location().to_owned());
        let address = if allowed(&address) {
            address
        } else {
            "about:blank".to_owned()
        };
        self.visible(false);
        self.close_popups();
        if let Some(view) = self.view.take() {
            let context = self.context.take();
            let downloads: Vec<_> = self
                .downloads
                .iter()
                .filter(|row| row.status == "Downloading")
                .filter(|row| {
                    !self
                        .finishing_views
                        .iter()
                        .any(|old| old.downloads.contains(&row.id))
                })
                .map(|row| row.id)
                .collect();
            if downloads.is_empty() {
                drop(view);
                drop(context);
            } else {
                // Unload the document while WebView2 finishes its original file operations.
                let _ = view.load_url("about:blank");
                self.finishing_views.push(FinishingView {
                    downloads,
                    _view: view,
                    _context: context,
                });
            }
        }
        (self.sender, self.receiver) = mpsc::channel();
        self.popup = None;
        self.playing_audio = false;
        self.address_editing = false;
        self.committed_address = address.clone();
        self.auto_start_pending = !to_side || address != "about:blank";
        self.address = if to_side && address == "about:blank" {
            String::new()
        } else {
            address
        };
        self.title = self.title.trim().chars().take(200).collect();
        self.error.clear();
        self.tab_id = next_address_generation();
        self.tab_order = self.tab_id;
        self.invalidate_address_input();
    }

    pub(crate) fn speaker_state(&self) -> Option<bool> {
        (self.playing_audio || self.muted).then_some(self.muted)
    }

    pub(crate) fn toggle_mute(&mut self) {
        if let Some(view) = &self.view
            && let Err(error) = audio::set_muted(view, !self.muted)
        {
            self.error = error.to_string();
            return;
        }
        self.muted = !self.muted;
    }

    pub(crate) fn zoom_steps(&self) -> i8 {
        self.zoom_override.unwrap_or(self.default_zoom)
    }

    fn apply_zoom(&mut self) {
        if let Some(view) = &self.view
            && let Err(error) = view.zoom(crate::workspace::BrowserPreferences::zoom_factor(
                self.zoom_steps(),
            ))
        {
            self.error = error.to_string();
        }
    }

    pub(crate) fn set_default_zoom(&mut self, value: i8) {
        let value = value.clamp(-6, 10);
        if value != self.default_zoom {
            self.default_zoom = value;
            if self.zoom_override.is_none() {
                self.apply_zoom();
            }
        }
    }

    pub(crate) fn change_zoom(&mut self, direction: i8) {
        self.zoom_override =
            (direction != 0).then(|| (self.zoom_steps() + direction.signum()).clamp(-6, 10));
        self.apply_zoom();
    }

    fn restorable_url(url: &str) -> Option<String> {
        if url.len() > 8192 {
            return None;
        }
        crate::view_tabs_browser_model::allowed_browser_url(url, false).filter(|url| {
            !url::Url::parse(url).ok().is_some_and(|url| {
                url.host_str().is_some_and(|host| {
                    host.starts_with("tesselpreview.") && host.ends_with(".localhost")
                })
            })
        })
    }

    fn current_location(&self) -> &str {
        if self.view.is_none() && self.auto_start_pending && !self.address_editing {
            &self.address
        } else {
            &self.committed_address
        }
    }

    pub(crate) fn restart_location(&self) -> (String, String) {
        match Self::restorable_url(self.current_location()) {
            Some(url) => (url, self.title.clone()),
            None => ("about:blank".into(), String::new()),
        }
    }

    pub(crate) fn from_restart_location(url: &str, title: &str) -> Self {
        let mut page = Self::default();
        let url = Self::restorable_url(url).unwrap_or_else(|| "about:blank".into());
        page.open_url(&url);
        page.committed_address = url.clone();
        if url != "about:blank" {
            page.title = title.chars().take(200).collect();
        }
        page
    }

    pub(crate) fn tab_label(&self) -> (String, String) {
        self.browser_label(false)
    }

    pub(crate) fn side_tab_title(&self) -> String {
        self.browser_label(true).0
    }

    fn browser_label(&self, side: bool) -> (String, String) {
        let pending = (self.view.is_none() && self.auto_start_pending && !self.address_editing)
            .then(|| crate::view_tabs_browser_model::allowed_browser_url(&self.address, true))
            .flatten();
        let url = crate::view_tabs_browser_model::display_url(
            pending.as_deref().unwrap_or(&self.committed_address),
        );
        let title = if !self.title.is_empty() {
            self.title.clone()
        } else if url.is_empty() {
            crate::i18n::tr(if side { "New page" } else { "Browser" }).to_owned()
        } else if side {
            url.clone()
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

    pub(crate) fn has_page_focus(&self) -> bool {
        self.webview_had_focus
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

    pub(crate) fn new_address_page() -> Self {
        Self {
            address: String::new(),
            committed_address: "about:blank".into(),
            ..Self::default()
        }
    }

    pub(crate) fn submit_address(&mut self) {
        self.navigate();
        if self.view.is_none() && self.error.is_empty() {
            self.auto_start_pending = true;
        }
    }

    pub(crate) fn history_directions(&self) -> (bool, bool) {
        self.view.as_ref().map_or((false, false), |view| {
            (
                view.can_go_back().unwrap_or(false),
                view.can_go_forward().unwrap_or(false),
            )
        })
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

    pub fn reveal_download(&self, id: u64) {
        let Some(row) = self
            .downloads
            .iter()
            .find(|row| row.id == id && row.status == "Completed")
        else {
            return;
        };
        let path = row.path.clone();
        let errors = self.sender.clone();
        std::thread::spawn(move || {
            if let Some(folder) = path.parent()
                && let Err(error) = crate::native_reveal::reveal(folder, &path)
            {
                let _ = errors.send(Event::Error(error));
            }
        });
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
            _ => return Err("A native Windows window is required to display this page.".into()),
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
        if !agent {
            zoom_keys::attach(&view, self.sender.clone())?;
            if let Err(error) = audio::attach(&view, self.muted, self.sender.clone()) {
                self.error = error.to_string();
            }
        }
        self.view = Some(view);
        if !agent {
            self.apply_zoom();
        }
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
        self.popups.prune_closed();
        let downloads = self.download_tracker.poll();
        let mut changed = self.downloads != downloads;
        self.downloads = downloads;
        self.finishing_views.retain(|view| {
            self.downloads
                .iter()
                .any(|row| row.status == "Downloading" && view.downloads.contains(&row.id))
        });
        while let Ok(event) = self.receiver.try_recv() {
            changed = true;
            match event {
                Event::PlayingAudio(playing) => self.playing_audio = playing,
                Event::Zoom(direction) => self.change_zoom(direction),
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
            self.playing_audio = false;
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
            if allowed(url) {
                self.committed_address = url.to_owned();
            }
            self.auto_start_pending = true;
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        self.show_with_address(ui, frame, false, true);
    }

    pub(crate) fn show_with_address(
        &mut self,
        ui: &mut egui::Ui,
        frame: &eframe::Frame,
        side: bool,
        only_browser: bool,
    ) {
        self.poll();
        if self.view.is_none() && self.auto_start_pending {
            self.auto_start_pending = false;
            if let Err(e) = self.create(frame) {
                self.error = e;
            }
        }
        if self.view.is_none() && !side {
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
        let address_id = egui::Id::new(("browser-address", self.tab_id));
        let shortcuts = ui.is_enabled()
            && (only_browser || self.has_page_focus() || ui.memory(|m| m.has_focus(address_id)));
        let focus_url =
            shortcuts && ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::L));
        let reload_shortcut = shortcuts
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
                    .id(address_id)
                    .desired_width((ui.available_width() - 140.0).max(100.0)),
            );
            if focus_url {
                response.request_focus();
            }
            if (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                || ui.button(tr("Go")).clicked()
            {
                self.submit_address();
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
            let mut reveal = None;
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
                            if status == "Completed"
                                && ui.small_button(tr("Show in folder")).clicked()
                            {
                                reveal = Some(download.id);
                            }
                            if ui.small_button(tr("Copy path")).clicked() {
                                ui.ctx().copy_text(path.display().to_string());
                            }
                        });
                    }
                });
            if let Some(id) = reveal {
                self.reveal_download(id);
            }
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
    fn tab_sound_targets_the_background_webview_without_switching_pages() {
        use crate::app::{Tessel, gpui_bridge::GpuiAction};
        use crate::webview_test_support::hidden_parent;
        use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_8;
        use windows::core::{BOOL, Interface};
        use wry::WebViewExtWindows;
        let temp = tempfile::tempdir().unwrap();
        let parent = hidden_parent();
        let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().join("state")), false, || {});
        let project = temp.path().join("project");
        std::fs::create_dir(&project).unwrap();
        app.apply_gpui_action(GpuiAction::AddProjectPath(project));
        let ctx = eframe::egui::Context::default();
        app.execute_palette_command(&ctx, "browser");
        app.execute_palette_command(&ctx, "browser:mute");
        let first = app.browser.tab_id;
        app.browser
            .create_with_profile(&parent, temp.path().join("profile"))
            .unwrap();
        let audio: ICoreWebView2_8 = unsafe {
            app.browser
                .view
                .as_ref()
                .unwrap()
                .controller()
                .CoreWebView2()
                .unwrap()
                .cast()
                .unwrap()
        };
        let muted = || {
            let mut value = BOOL::default();
            unsafe { audio.IsMuted(&mut value) }.unwrap();
            value.as_bool()
        };
        assert!(muted());
        // Exercise Chromium's playback notifications with the output muted throughout.
        let view = app.browser.view.as_ref().unwrap();
        crate::webview_test_support::wait_for(view, "document.readyState === 'complete'");
        let core = unsafe { view.controller().CoreWebView2().unwrap() };
        let params = serde_json::json!({
            "expression": "window.fixtureAudio = new AudioContext(); const osc = fixtureAudio.createOscillator(); osc.connect(fixtureAudio.destination); osc.start(); fixtureAudio.resume()",
            "userGesture": true
        });
        unsafe {
            core.CallDevToolsProtocolMethod(
                &windows::core::HSTRING::from("Runtime.evaluate"),
                &windows::core::HSTRING::from(params.to_string()),
                &webview2_com::CallDevToolsProtocolMethodCompletedHandler::create(Box::new(
                    |_, _| Ok(()),
                )),
            )
            .unwrap();
        }
        let wait_audio = |page: &mut Browser, playing| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(12);
            while page.playing_audio != playing {
                assert!(
                    std::time::Instant::now() < deadline,
                    "playback notification {playing} timed out"
                );
                crate::webview_test_support::pump();
                page.poll();
            }
        };
        wait_audio(&mut app.browser, true);
        app.browser
            .view
            .as_ref()
            .unwrap()
            .evaluate_script("fixtureAudio.close()")
            .unwrap();
        wait_audio(&mut app.browser, false);

        app.execute_palette_command(&ctx, "browser");
        let second = app.browser.tab_id;
        let first_speaker = |app: &mut Tessel| {
            app.gpui_snapshot(960., 640., 4.)
                .view_tabs
                .into_iter()
                .find(|tab| tab.browser_id == Some(first))
                .unwrap()
                .sound
        };
        assert_eq!(first_speaker(&mut app), Some(true));
        app.apply_gpui_action(GpuiAction::ToggleBrowserSound(first));
        assert!(!muted());
        assert_eq!(app.browser.tab_id, second);
        assert_eq!(app.browser.speaker_state(), None);
        assert_eq!(first_speaker(&mut app), None);
    }

    #[test]
    #[ignore = "requires installed Windows WebView2 runtime"]
    fn owner_zoom_changes_native_controller_and_resets_to_current_default() {
        use wry::WebViewExtWindows;
        let temp = tempfile::tempdir().unwrap();
        let parent = crate::webview_test_support::hidden_parent();
        let mut page = Browser::default();
        page.create_with_profile(&parent, temp.path().to_owned())
            .unwrap();
        let controller = page.view.as_ref().unwrap().controller();
        let percent = || {
            let mut factor = 0.0;
            unsafe { controller.ZoomFactor(&mut factor) }.unwrap();
            (factor * 100.0).round() as i32
        };
        assert_eq!(percent(), 91);
        page.change_zoom(1);
        assert_eq!(percent(), 100);
        page.set_default_zoom(2);
        assert_eq!(percent(), 100);
        page.change_zoom(0);
        assert_eq!(percent(), 120);
        for _ in 0..20 {
            page.change_zoom(1);
        }
        assert_eq!(percent(), 249);
        for _ in 0..20 {
            page.change_zoom(-1);
        }
        assert_eq!(percent(), 58);
    }

    #[test]
    #[ignore = "requires installed Windows WebView2 runtime"]
    fn restart_loads_saved_owner_tab_in_webview() {
        use crate::app::{Tessel, gpui_bridge::GpuiAction};
        use crate::webview_test_support::{hidden_parent, wait_for};
        use std::io::{BufRead, BufReader, Write};
        let temp = tempfile::tempdir().unwrap();
        let parent = hidden_parent();
        let start = || Tessel::new_for_gpui_in_dir(Some(temp.path().join("state")), false, || {});
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/saved-tab", server.local_addr().unwrap());
        let mut app = start();
        let project = temp.path().join("project");
        std::fs::create_dir(&project).unwrap();
        app.apply_gpui_action(GpuiAction::AddProjectPath(project));
        app.execute_palette_command(&eframe::egui::Context::default(), "browser");
        app.browser.open_url(&url);
        drop(app);
        let mut app = start();
        assert!(!app.browser.is_started());
        assert_eq!(app.browser.address_str(), url);
        server.set_nonblocking(true).unwrap();
        let (stop, stopped) = std::sync::mpsc::channel::<()>();
        let request = std::thread::spawn(move || {
            let mut served = false;
            while matches!(
                stopped.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            ) {
                let (mut socket, _) = match server.accept() {
                    Ok(pair) => pair,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        continue;
                    }
                    Err(e) => panic!("fixture accept failed: {e}"),
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(std::time::Duration::from_millis(250)))
                    .unwrap();
                let mut reader = BufReader::new(&socket);
                let mut first = String::new();
                if reader.read_line(&mut first).is_err() {
                    continue;
                }
                let body = match first.split_whitespace().nth(1) {
                    Some("/saved-tab") => "<title>Restored owner tab</title>",
                    Some("/side") => "<title>Side browser page</title>",
                    _ => continue,
                };
                for line in reader.lines() {
                    if !matches!(line, Ok(header) if !header.is_empty()) {
                        break;
                    }
                }
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                served |= socket.write_all(response.as_bytes()).is_ok();
            }
            served
        });
        app.browser
            .create_with_profile(&parent, temp.path().join("profile"))
            .unwrap();
        app.browser.navigate_now();
        wait_for(
            app.browser.view.as_ref().unwrap(),
            "document.title === 'Restored owner tab'",
        );
        app.apply_gpui_action(GpuiAction::NewSideBrowser);
        let side = app.selected_side_browser.unwrap();
        let scope = app.browser_address_for(true).unwrap().0;
        app.side_browsers
            .get_mut(&side)
            .unwrap()
            .create_with_profile(&parent, temp.path().join("profile"))
            .unwrap();
        app.apply_gpui_action(GpuiAction::NativeBrowserAddress {
            scope,
            base: String::new(),
            text: url.replace("/saved-tab", "/side"),
            navigate: true,
        });
        wait_for(
            app.side_browsers[&side].view.as_ref().unwrap(),
            "document.title === 'Side browser page'",
        );
        wait_for(
            app.browser.view.as_ref().unwrap(),
            "document.title === 'Restored owner tab'",
        );
        drop(stop);
        assert!(request.join().unwrap());
    }

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
        check_owner_download_during_move(false);
    }

    #[test]
    #[ignore = "requires installed Windows WebView2 runtime"]
    fn moving_downloading_page_twice_keeps_original_operation_and_releases_old_controller() {
        check_owner_download_during_move(true);
    }

    fn check_owner_download_during_move(move_page: bool) {
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
        if move_page {
            app.apply_gpui_action(GpuiAction::DockBrowser(app.browser.tab_id));
            let side = app.selected_side_browser.unwrap();
            assert_eq!(app.side_browsers[&side].finishing_views.len(), 1);
            app.apply_gpui_action(GpuiAction::BrowserToggleDownloads(side));
            let snapshot = app.gpui_snapshot(960., 640., 4.);
            let drawer = snapshot.side_browser.unwrap();
            assert!(drawer.downloads_open);
            assert_eq!(drawer.tab_id, side);
            assert_eq!(drawer.downloads.len(), 1);
            assert_eq!(drawer.downloads[0].1, "Downloading");
            assert_eq!(PathBuf::from(&drawer.downloads[0].4), file);
            assert!(snapshot.browser.downloads.is_empty());
            assert!(!snapshot.browser.downloads_open);
            app.apply_gpui_action(GpuiAction::UndockBrowser(side));
            assert_eq!(app.browser.finishing_views.len(), 1);
            app.apply_gpui_action(GpuiAction::BrowserToggleDownloads(side));
            assert!(app.browser.downloads_open());
            assert!(app.gpui_snapshot(960., 640., 4.).side_browser.is_none());
        }
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
                assert!(app.browser.finishing_views.is_empty());
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
        app.browser.zoom_override = Some(2);
        app.browser.toggle_mute();
        wait_for(
            app.browser.view.as_ref().unwrap(),
            "(localStorage.setItem('move-fixture','session retained'),history.pushState({old:true},''),true)",
        );
        app.apply_gpui_action(GpuiAction::DockBrowser(ids[0]));
        assert_eq!(app.browser.tab_id, ids[1]);
        let side = app.selected_side_browser.unwrap();
        assert_ne!(side, ids[0]);
        let page = app.side_browsers.get_mut(&side).unwrap();
        assert!(!page.is_started());
        assert_eq!(page.tab_label().0, "first");
        assert_eq!(page.zoom_override, Some(2));
        assert!(page.muted);
        page.create_with_profile(&parent, dir.path().join("owner-profile"))
            .unwrap();
        page.navigate_now();
        wait_for(
            app.side_browsers[&side].view.as_ref().unwrap(),
            "document.title === 'first' && document.querySelector('#draft').value === '' && !history.state && localStorage.getItem('move-fixture') === 'session retained'",
        );
        wait_for(
            app.side_browsers[&side].view.as_ref().unwrap(),
            "(history.pushState({side:true}, '', '#side'), true)",
        );
        assert!(app.gpui_snapshot(960., 640., 4.).side_history.0);
        app.apply_gpui_action(GpuiAction::SideBrowserBack(side));
        wait_for(
            app.side_browsers[&side].view.as_ref().unwrap(),
            "!history.state",
        );
        assert!(app.gpui_snapshot(960., 640., 4.).side_history.1);
        app.apply_gpui_action(GpuiAction::SideBrowserForward(side));
        wait_for(
            app.side_browsers[&side].view.as_ref().unwrap(),
            "history.state?.side === true",
        );
        wait_for(
            app.side_browsers[&side].view.as_ref().unwrap(),
            "(document.querySelector('#draft').value = 'side draft') === 'side draft'",
        );
        app.apply_gpui_action(GpuiAction::SideBrowserReload(side));
        wait_for(
            app.side_browsers[&side].view.as_ref().unwrap(),
            "document.querySelector('#draft')?.value === ''",
        );
        wait_for(
            app.browser.view.as_ref().unwrap(),
            "document.querySelector('#draft').value === 'second!'",
        );

        app.apply_gpui_action(GpuiAction::UndockBrowser(side));
        assert!(app.side_browsers.is_empty());
        assert_ne!(app.browser.tab_id, side);
        assert!(!app.browser.is_started());
        app.browser
            .create_with_profile(&parent, dir.path().join("owner-profile"))
            .unwrap();
        app.browser.navigate_now();
        wait_for(
            app.browser.view.as_ref().unwrap(),
            "document.title === 'first' && document.querySelector('#draft').value === '' && localStorage.getItem('move-fixture') === 'session retained'",
        );
        app.apply_gpui_action(GpuiAction::CloseBrowserPage(app.browser.tab_id));
        assert_eq!(app.browser.tab_id, ids[1]);
        wait_for(
            app.browser.view.as_ref().unwrap(),
            "document.querySelector('#draft').value === 'second!'",
        );
    }

    #[test]
    fn moving_blank_and_unloaded_pages_discards_only_the_unsubmitted_address() {
        let mut page = Browser::new_address_page();
        page.set_native_address_draft("draft.example".into());
        page.change_presentation(true);
        assert_eq!(page.address_str(), "");
        assert!(!page.auto_start_pending);
        page.change_presentation(false);
        assert_eq!(page.address_str(), "about:blank");
        page.open_url("https://selected.example/");
        page.set_native_address_draft("https://draft.example/".into());
        page.change_presentation(true);
        assert_eq!(page.address_str(), "https://selected.example/");
        assert!(page.auto_start_pending);
        page.open_url("file:///outside");
        page.change_presentation(false);
        assert_eq!(page.address_str(), "about:blank");
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
