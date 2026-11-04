//! Native WebView2 child window. Websites receive no native IPC capability.
use crate::i18n::tr;
mod capture;
mod control;
mod downloads;
mod private_fill;
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
    Popup(String),
    Error(String),
    Navigation(String),
}

pub struct Browser {
    view: Option<WebView>,
    // Drop view before its environment.
    context: Option<WebContext>,
    address: String,
    popup: Option<String>,
    error: String,
    downloads: VecDeque<downloads::Record>,
    download_tracker: downloads::Downloads,
    capture: capture::Capture,
    destination: Arc<Mutex<PathBuf>>,
    sender: Sender<Event>,
    receiver: Receiver<Event>,
    downloads_open: bool,
    agent_root: Option<PathBuf>,
    agent_profile: Option<tempfile::TempDir>,
    planned_download: Arc<Mutex<Option<(PathBuf, std::time::Instant)>>>,
    clip: crate::browser_clip::NativeClip,
    viewport: Option<(egui::Rect, f32)>,
    webview_had_focus: bool,
}

impl Default for Browser {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        let destination = directories::UserDirs::new()
            .and_then(|d| d.download_dir().map(|p| p.join("Tessel Rust")))
            .unwrap_or_else(|| crate::launch::default_directory().join("Downloads/Tessel Rust"));
        Self {
            view: None,
            context: None,
            address: "https://example.com".into(),
            popup: None,
            error: String::new(),
            downloads: VecDeque::new(),
            download_tracker: Default::default(),
            capture: Default::default(),
            destination: Arc::new(Mutex::new(destination)),
            sender,
            receiver,
            downloads_open: false,
            agent_root: None,
            agent_profile: None,
            planned_download: Default::default(),
            clip: Default::default(),
            viewport: None,
            webview_had_focus: false,
        }
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
    fn create(&mut self, frame: &eframe::Frame) -> Result<(), String> {
        let dirs = directories::ProjectDirs::from("com", "Tessel", "TesselRust")
            .ok_or("No app data directory")?;
        self.create_with_profile(frame, dirs.data_local_dir().join("browser-owner"))?;
        self.navigate();
        Ok(())
    }
    fn create_with_profile(
        &mut self,
        frame: &eframe::Frame,
        profile: PathBuf,
    ) -> Result<(), String> {
        self.context = Some(WebContext::new(Some(profile)));
        let start = self.sender.clone();
        let popup = self.sender.clone();
        let navigation = self.sender.clone();
        let destination = self.destination.clone();
        let agent_root = self.agent_root.clone();
        let agent = agent_root.is_some();
        let planned = self.planned_download.clone();
        let view = WebViewBuilder::new_with_web_context(
            self.context.as_mut().ok_or("Missing browser environment")?,
        )
        .with_url("about:blank")
        .with_visible(false)
        .with_devtools(false)
        .with_general_autofill_enabled(false)
        .with_navigation_handler(move |address| {
            let permit = allowed(&address);
            if permit {
                let _ = navigation.send(Event::Navigation(address));
            }
            permit
        })
        .with_new_window_req_handler(move |address, _| {
            if allowed(&address) {
                let _ = popup.send(Event::Popup(address));
            }
            wry::NewWindowResponse::Deny
        })
        .build_as_child(frame)
        .map_err(|e| e.to_string())?;
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
        self.address = address.clone();
        self.error.clear();
        if let Some(view) = &self.view {
            self.error = view
                .load_url(&address)
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default();
        }
    }
    pub fn poll(&mut self) {
        self.downloads = self.download_tracker.poll();
        while let Ok(event) = self.receiver.try_recv() {
            match event {
                Event::Popup(address) => self.popup = Some(address),
                Event::Error(e) => self.error = e,
                Event::Navigation(address) => self.address = address,
            }
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        self.poll();
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
        ui.horizontal(|ui| {
            if let Some(view) = &self.view {
                if ui.button("←").clicked() {
                    let _ = view.go_back();
                }
                if ui.button("→").clicked() {
                    let _ = view.go_forward();
                }
                if ui.button(tr("Reload")).clicked() {
                    let _ = view.reload();
                }
            }
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.address)
                    .desired_width((ui.available_width() - 140.0).max(100.0)),
            );
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
