//! Native WebView2 child window. Websites receive no native IPC capability.
use crate::i18n::tr;
mod control;
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
    Started(PathBuf),
    Completed(Option<PathBuf>, bool),
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
    downloads: VecDeque<(PathBuf, String)>,
    destination: Arc<Mutex<PathBuf>>,
    sender: Sender<Event>,
    receiver: Receiver<Event>,
    downloads_open: bool,
    agent_root: Option<PathBuf>,
    agent_profile: Option<tempfile::TempDir>,
    planned_download: Arc<Mutex<Option<(PathBuf, std::time::Instant)>>>,
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
            destination: Arc::new(Mutex::new(destination)),
            sender,
            receiver,
            downloads_open: false,
            agent_root: None,
            agent_profile: None,
            planned_download: Default::default(),
        }
    }
}

fn allowed(address: &str) -> bool {
    url::Url::parse(address)
        .is_ok_and(|u| matches!(u.scheme(), "http" | "https" | "blob") || address == "about:blank")
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
    pub fn open_address(&mut self, address: String) {
        self.address = address;
        self.navigate();
    }

    pub fn fill_account(&self, account: &crate::vault::Account) -> Result<(), String> {
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
        let payload = zeroize::Zeroizing::new(
            serde_json::to_string(account).map_err(|_| "Cannot prepare fill")?,
        );
        let script = zeroize::Zeroizing::new(format!(
            r#"(() => {{
            const account = {};
            if (location.origin !== account.origin || window !== window.top) return;
            const visible = el => !el.disabled && !el.readOnly && el.getClientRects().length > 0;
            const fields = [...document.querySelectorAll('input[type=password]')].filter(visible);
            if (fields.length !== 1) return;
            const password = fields[0];
            const form = password.form;
            if (!form) return;
            const users = [...form.querySelectorAll('input[type=email],input[type=text],input:not([type])')].filter(visible);
            if (users.length > 1) return;
            const set = (el,value) => {{ Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(el,value); el.dispatchEvent(new Event('input',{{bubbles:true}})); el.dispatchEvent(new Event('change',{{bubbles:true}})); }};
            if (users.length === 1) set(users[0],account.username);
            set(password,account.password);
        }})()"#,
            payload.as_str()
        ));
        view.evaluate_script(&script)
            .map_err(|_| "Browser refused the fill request".into())
    }

    pub fn visible(&self, visible: bool) {
        if let Some(view) = &self.view {
            let _ = view.set_visible(visible);
        }
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
        let completed = self.sender.clone();
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
        .with_download_started_handler(move |_, path| {
            let result = if agent {
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
            };
            match result {
                Ok(target) => {
                    *path = target.clone();
                    let _ = start.send(Event::Started(target));
                    true
                }
                Err(e) => {
                    let _ = start.send(Event::Error(e));
                    false
                }
            }
        })
        .with_download_completed_handler(move |_, path, ok| {
            let _ = completed.send(Event::Completed(path, ok));
        })
        .build_as_child(frame)
        .map_err(|e| e.to_string())?;
        self.view = Some(view);
        Ok(())
    }
    fn navigate(&mut self) {
        let address = if self.address.contains(":") {
            self.address.clone()
        } else {
            format!("https://{}", self.address)
        };
        if !allowed(&address) {
            self.error =
                "Only HTTP(S), same-session blob URLs and about:blank are supported.".into();
            return;
        }
        if let Some(view) = &self.view {
            self.error = view
                .load_url(&address)
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default();
        }
    }
    pub fn poll(&mut self) {
        while let Ok(event) = self.receiver.try_recv() {
            match event {
                Event::Started(path) => {
                    let path = download_key(path);
                    self.downloads.push_front((path, "Downloading".into()));
                    self.downloads.truncate(100);
                }
                Event::Completed(path, ok) => {
                    if let Some(path) = path {
                        let path = download_key(path);
                        if let Some(d) = self.downloads.iter_mut().find(|d| d.0 == path) {
                            d.1 = if ok {
                                "Completed"
                            } else {
                                "Cancelled / failed"
                            }
                            .into();
                        } else {
                            self.downloads.push_front((
                                path,
                                if ok {
                                    "Completed"
                                } else {
                                    "Cancelled / failed"
                                }
                                .into(),
                            ));
                            self.downloads.truncate(100);
                        }
                    }
                }
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
                    for (path, status) in &self.downloads {
                        ui.horizontal(|ui| {
                            ui.label(tr(status));
                            ui.label(path.display().to_string());
                            if ui.small_button(tr("Copy path")).clicked() {
                                ui.ctx().copy_text(path.display().to_string());
                            }
                        });
                    }
                });
        }
        let rect = ui.available_rect_before_wrap();
        let scale = ui.ctx().pixels_per_point();
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
            let _ = view.set_visible(true);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
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
        for address in ["file:///C:/secret", "javascript:alert(1)", "tessel://tool"] {
            assert!(!allowed(address));
        }
        assert!(allowed("https://example.org"));
        assert_eq!(safe_name("CON.pdf"), "download.bin");
        assert!(!safe_name("../../evil.exe").contains('/'));
    }
}
