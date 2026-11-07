//! Terminal OAuth / Device-Code Sign-In Banner (`deviceLogin.js`, `TerminalPane.vue`):
//! - `400 ms` debounce on local PTY screen output.
//! - 14 OAuth trigger phrases, soft-wrapped `https://` URL reconstruction across full-width rows,
//!   strict single-distinct-URL invariant (`<= 4096` chars, `https` only, no embedded credentials),
//!   optional `XXXX-XXXX` device-code extraction, and known-host allowlist vs hostname caution.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use eframe::egui;

pub const DEVICE_LOGIN_DEBOUNCE: Duration = Duration::from_millis(400);
pub const MAX_OAUTH_URL_LEN: usize = 4096;

pub const OAUTH_TRIGGER_PHRASES: [&str; 14] = [
    "sign in",
    "log in",
    "login",
    "authenticate",
    "authorization",
    "authorize",
    "device code",
    "user code",
    "one-time code",
    "verification uri",
    "enter code",
    "open this link",
    "open browser",
    "oauth",
];

pub const KNOWN_OAUTH_HOSTS: [&str; 15] = [
    "github.com",
    "login.microsoftonline.com",
    "microsoft.com",
    "accounts.google.com",
    "console.anthropic.com",
    "claude.ai",
    "auth.openai.com",
    "chatgpt.com",
    "platform.openai.com",
    "gitlab.com",
    "bitbucket.org",
    "vercel.com",
    "cloudflare.com",
    "dash.cloudflare.com",
    "tesselcode.com",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceLoginPrompt {
    pub url: String,
    pub host: String,
    pub known_host: bool,
    pub device_code: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeviceLoginAction {
    OpenInTessel(String),
    OpenInSystemBrowser(String),
    CopyLink(String),
    CopyCode(String),
    Dismiss,
}

#[must_use]
pub fn is_known_oauth_host(host: &str) -> bool {
    let lower = host.trim().to_ascii_lowercase();
    KNOWN_OAUTH_HOSTS
        .iter()
        .any(|&allowed| lower == allowed || lower.ends_with(&format!(".{allowed}")))
}

/// Joins soft-wrapped terminal rows when a line reaches the terminal column width inside an `https://` token.
#[must_use]
pub fn unwrap_soft_wrapped_lines(screen: &str, cols: u16) -> String {
    let min_wrap_cols = (cols as usize).saturating_sub(1).max(40);
    let raw_lines: Vec<&str> = screen.lines().collect();
    let mut out = String::with_capacity(screen.len());
    let mut in_url_wrap = false;

    for (idx, &line) in raw_lines.iter().enumerate() {
        let trimmed_end = line.trim_end_matches('\r');
        if in_url_wrap {
            let cont: String = trimmed_end
                .chars()
                .take_while(|c| !c.is_whitespace())
                .collect();
            out.push_str(&cont);
            let rest = &trimmed_end[cont.len()..];
            out.push_str(rest);
            in_url_wrap = rest.is_empty()
                && trimmed_end.chars().count() >= min_wrap_cols
                && idx + 1 < raw_lines.len()
                && raw_lines[idx + 1]
                    .chars()
                    .next()
                    .is_some_and(is_url_continuation_char);
            if !in_url_wrap {
                out.push('\n');
            }
            continue;
        }

        out.push_str(trimmed_end);
        let last_token = trimmed_end
            .split_whitespace()
            .next_back()
            .unwrap_or_default();
        let wraps_url = last_token.starts_with("https://")
            && trimmed_end.chars().count() >= min_wrap_cols
            && idx + 1 < raw_lines.len()
            && raw_lines[idx + 1]
                .chars()
                .next()
                .is_some_and(is_url_continuation_char);
        if wraps_url {
            in_url_wrap = true;
        } else {
            out.push('\n');
        }
    }
    out
}

fn is_url_continuation_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(
            c,
            '-' | '.' | '_' | '~' | '/' | '?' | '#' | '&' | '=' | '%' | '+' | ':' | '@'
        )
}

fn trim_trailing_url_punct(raw: &str) -> &str {
    raw.trim_end_matches(['.', ',', ';', ':', '!', '?', ')', '>', ']', '\'', '"'])
}

#[must_use]
pub fn extract_device_code(text: &str) -> Option<String> {
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        let mentions_code =
            lower.contains("code") || lower.contains("enter ") || lower.contains("verification");
        if !mentions_code {
            continue;
        }
        for raw_tok in line.split_whitespace() {
            let tok = raw_tok.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-');
            if (7..=14).contains(&tok.len()) && tok.contains('-') {
                let groups: Vec<&str> = tok.split('-').collect();
                if (2..=3).contains(&groups.len())
                    && groups.iter().all(|g| {
                        (3..=6).contains(&g.len())
                            && g.chars()
                                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
                    })
                    && tok
                        .chars()
                        .any(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
                {
                    return Some(tok.to_owned());
                }
            }
        }
    }
    None
}

#[must_use]
pub fn detect_device_login_prompt(screen_text: &str, cols: u16) -> Option<DeviceLoginPrompt> {
    if screen_text.trim().is_empty() {
        return None;
    }
    let unwrapped = unwrap_soft_wrapped_lines(screen_text, cols);
    let lower = unwrapped.to_ascii_lowercase();
    if !OAUTH_TRIGGER_PHRASES.iter().any(|p| lower.contains(p)) {
        return None;
    }

    let mut distinct_urls: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for token in unwrapped.split_whitespace() {
        let idx = match token.find("https://") {
            Some(i) => i,
            None => continue,
        };
        let candidate = trim_trailing_url_punct(&token[idx..]);
        if candidate.len() > MAX_OAUTH_URL_LEN {
            return None;
        }
        if let Ok(parsed) = url::Url::parse(candidate) {
            if parsed.scheme() != "https"
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.host_str().is_none()
            {
                return None;
            }
            let normalized = candidate.to_owned();
            if seen.insert(normalized.clone()) {
                distinct_urls.push(normalized);
                if distinct_urls.len() > 1 {
                    // Safety invariant: never guess when multiple distinct HTTPS URLs appear.
                    return None;
                }
            }
        }
    }

    let url = distinct_urls.into_iter().next()?;
    let parsed = url::Url::parse(&url).ok()?;
    let host = parsed.host_str()?.to_owned();
    let known_host = is_known_oauth_host(&host);
    let device_code = extract_device_code(&unwrapped);

    Some(DeviceLoginPrompt {
        url,
        host,
        known_host,
        device_code,
    })
}

#[derive(Default)]
pub struct DeviceLoginDetector {
    last_revision: u64,
    pending_since: Option<Instant>,
    dismissed_url: Option<String>,
    active_prompt: Option<DeviceLoginPrompt>,
}

impl DeviceLoginDetector {
    pub fn tick(
        &mut self,
        ctx: &egui::Context,
        output_revision: u64,
        cols: u16,
        read_screen: impl FnOnce() -> String,
    ) {
        if output_revision != self.last_revision {
            self.last_revision = output_revision;
            self.pending_since = Some(Instant::now());
        }
        if let Some(since) = self.pending_since {
            let elapsed = since.elapsed();
            if elapsed < DEVICE_LOGIN_DEBOUNCE {
                ctx.request_repaint_after(DEVICE_LOGIN_DEBOUNCE - elapsed);
                return;
            }
            self.pending_since = None;
            let screen = read_screen();
            match detect_device_login_prompt(&screen, cols) {
                Some(prompt) => {
                    if self.dismissed_url.as_deref() != Some(prompt.url.as_str()) {
                        self.active_prompt = Some(prompt);
                    }
                }
                None => {
                    self.active_prompt = None;
                }
            }
        }
    }

    #[must_use]
    pub fn active_prompt(&self) -> Option<&DeviceLoginPrompt> {
        self.active_prompt.as_ref()
    }

    pub fn dismiss_current(&mut self) {
        if let Some(p) = self.active_prompt.take() {
            self.dismissed_url = Some(p.url);
        }
    }
}

pub fn show_device_login_banner(
    ui: &mut egui::Ui,
    prompt: &DeviceLoginPrompt,
    french: bool,
) -> Option<DeviceLoginAction> {
    let palette = crate::theme::active(ui.ctx()).spec().ui;
    let mut action = None;

    egui::Frame::NONE
        .fill(palette.surface_2)
        .stroke(egui::Stroke::new(
            1.0_f32,
            if prompt.known_host {
                palette.accent
            } else {
                palette.warn
            },
        ))
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new(if french {
                        "Connexion détectée"
                    } else {
                        "Sign-in link detected"
                    })
                    .strong()
                    .color(palette.text_strong),
                );
                if prompt.known_host {
                    ui.small(egui::RichText::new(&prompt.host).color(palette.text_dim));
                } else {
                    ui.colored_label(
                        palette.warn,
                        if french {
                            format!("Hôte externe : {}", prompt.host)
                        } else {
                            format!("External host: {}", prompt.host)
                        },
                    );
                }
                if let Some(code) = &prompt.device_code {
                    ui.label(
                        egui::RichText::new(format!("Code: {code}"))
                            .monospace()
                            .strong()
                            .color(palette.accent),
                    );
                }
                if ui
                    .button(if french {
                        "Ouvrir dans Tessel"
                    } else {
                        "Open in Tessel"
                    })
                    .on_hover_text(&prompt.url)
                    .clicked()
                {
                    action = Some(DeviceLoginAction::OpenInTessel(prompt.url.clone()));
                }
                if ui
                    .button(if french {
                        "Ouvrir dans mon navigateur"
                    } else {
                        "Open in my browser"
                    })
                    .on_hover_text(&prompt.url)
                    .clicked()
                {
                    action = Some(DeviceLoginAction::OpenInSystemBrowser(prompt.url.clone()));
                }
                if ui
                    .button(if french {
                        "Copier le lien"
                    } else {
                        "Copy the link"
                    })
                    .clicked()
                {
                    action = Some(DeviceLoginAction::CopyLink(prompt.url.clone()));
                }
                if let Some(code) = &prompt.device_code
                    && ui
                        .button(if french {
                            "Copier le code"
                        } else {
                            "Copy the code"
                        })
                        .clicked()
                {
                    action = Some(DeviceLoginAction::CopyCode(code.clone()));
                }
                if ui.button(if french { "Fermer" } else { "Close" }).clicked() {
                    action = Some(DeviceLoginAction::Dismiss);
                }
            });
        });
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconstructs_soft_wrapped_oauth_url_and_extracts_device_code() {
        let line1 =
            "To sign in, open https://github.com/login/device?client_id=Iv1.1234567890abcdef";
        let line2 = "&scope=repo%20read:org in your browser and enter code ABCD-9876.";
        let screen = format!("{line1}\n{line2}\n");
        let prompt = detect_device_login_prompt(&screen, 80).expect("should detect OAuth prompt");
        assert_eq!(
            prompt.url,
            "https://github.com/login/device?client_id=Iv1.1234567890abcdef&scope=repo%20read:org"
        );
        assert!(prompt.known_host);
        assert_eq!(prompt.device_code.as_deref(), Some("ABCD-9876"));
    }

    #[test]
    fn rejects_multiple_distinct_urls_or_credentials_and_flags_unknown_host() {
        let two_urls = "Sign in at https://github.com/login/device or https://gitlab.com/oauth";
        assert!(detect_device_login_prompt(two_urls, 100).is_none());

        let embedded_creds = "Sign in at https://user:secret@example.com/oauth";
        assert!(detect_device_login_prompt(embedded_creds, 100).is_none());

        let unknown = "Please authenticate via https://sso.internal-corp.example/device";
        let prompt = detect_device_login_prompt(unknown, 100).expect("single https URL");
        assert_eq!(prompt.host, "sso.internal-corp.example");
        assert!(!prompt.known_host);
    }
}
