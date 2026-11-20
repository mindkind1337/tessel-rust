//! Owner-only cookie-file import. No exported values or paths enter diagnostics.
use super::Browser;
use crate::cookie_import_model::{MAX_COOKIE_FILE, SameSitePolicy, parse_cookie_export};
use std::io::Read;
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use windows::core::{HSTRING, Interface};
use wry::WebViewExtWindows;

impl Browser {
    pub fn cookie_import_status(&self) -> &str {
        &self.cookie_import_status
    }

    /// Only a local owner toolbar action calls this method; never a browser tool.
    pub fn import_cookie_file_now(&mut self) {
        if self.agent_root.is_some() || self.view.is_none() {
            self.error = "Cookie import requires the owner's running browser.".into();
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Cookie export (JSON / Netscape)", &["json", "txt"])
            .pick_file()
        else {
            return;
        };
        let result = (|| {
            let file = std::fs::File::open(path).map_err(|_| "Cannot read cookie export.")?;
            let mut text = String::new();
            file.take(MAX_COOKIE_FILE as u64 + 1)
                .read_to_string(&mut text)
                .map_err(|_| "Cookie export must be readable UTF-8 text.")?;
            if text.len() > MAX_COOKIE_FILE {
                return Err("Cookie export exceeds 32 MiB.");
            }
            self.import_cookie_text(&text)
        })();
        match result {
            Ok((saved, skipped)) => {
                self.error.clear();
                self.cookie_import_status = if crate::i18n::is_french() {
                    format!("{saved} cookies importés; {skipped} ignorés.")
                } else {
                    format!("{saved} cookies imported; {skipped} skipped.")
                };
            }
            Err(error) => {
                self.cookie_import_status.clear();
                self.error = error.into();
            }
        }
    }

    pub(super) fn import_cookie_text(&self, text: &str) -> Result<(usize, usize), &'static str> {
        // Check authority before parsing or acquiring the browser's cookie manager.
        if self.agent_root.is_some() {
            return Err("Cookie import is unavailable in agent browsers.");
        }
        let view = self.view.as_ref().ok_or("Start the owner browser first.")?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "System clock is unavailable.")?
            .as_secs();
        let (cookies, summary) = parse_cookie_export(text, &[], now);
        if cookies.is_empty() {
            return Err("No eligible cookies in this export (Google cookies are excluded).");
        }
        let mut saved = 0;
        let mut skipped = summary.skipped;
        unsafe {
            let core: ICoreWebView2_2 = view
                .webview()
                .cast()
                .map_err(|_| "Cookie import is unavailable in this browser runtime.")?;
            let manager = core
                .CookieManager()
                .map_err(|_| "Cookie manager is unavailable.")?;
            for cookie in cookies {
                let result = (|| -> windows::core::Result<()> {
                    let host = cookie.host.trim_start_matches('.');
                    let domain = if cookie.host_only {
                        host.to_owned()
                    } else {
                        format!(".{host}")
                    };
                    // Use the native API so the explicit domain/host-only distinction survives.
                    let native = manager.CreateCookie(
                        &HSTRING::from(&cookie.name),
                        &HSTRING::from(&cookie.value),
                        &HSTRING::from(domain),
                        &HSTRING::from(&cookie.path),
                    )?;
                    native.SetIsSecure(cookie.secure)?;
                    native.SetIsHttpOnly(cookie.http_only)?;
                    if cookie.expires > 0 {
                        native.SetExpires(cookie.expires as f64)?;
                    }
                    match cookie.same_site {
                        SameSitePolicy::Unspecified => {}
                        SameSitePolicy::NoRestriction => {
                            native.SetSameSite(COREWEBVIEW2_COOKIE_SAME_SITE_KIND_NONE)?
                        }
                        SameSitePolicy::Lax => {
                            native.SetSameSite(COREWEBVIEW2_COOKIE_SAME_SITE_KIND_LAX)?
                        }
                        SameSitePolicy::Strict => {
                            native.SetSameSite(COREWEBVIEW2_COOKIE_SAME_SITE_KIND_STRICT)?
                        }
                    }
                    manager.AddOrUpdateCookie(&native)
                })();
                if result.is_ok() {
                    saved += 1;
                } else {
                    skipped += 1;
                }
            }
        }
        Ok((saved, skipped))
    }
}
