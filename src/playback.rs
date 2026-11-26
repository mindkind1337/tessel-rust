//! Immutable, bounded original audio and video bytes for local and SSH editor previews.
use std::{io::Read, path::Path, sync::Arc};

pub const MAX_BYTES: usize = 30 * 1024 * 1024;

pub fn mime(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "flac" => "audio/flac",
        "weba" => "audio/webm",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "ogv" => "video/ogg",
        "mov" => "video/quicktime",
        _ => return None,
    })
}

#[derive(Clone)]
pub struct Source {
    mime: &'static str,
    bytes: Arc<Vec<u8>>,
    #[cfg(windows)]
    failed: Arc<std::sync::atomic::AtomicBool>,
}
impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Media")
            .field("mime", &self.mime)
            .field("length", &self.bytes.len())
            .finish()
    }
}
impl Source {
    pub fn open(path: &Path, mime: &'static str) -> Result<Self, String> {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        if file.metadata().map_err(|e| e.to_string())?.len() > MAX_BYTES as u64 {
            return Err("Media preview limit is 30 MiB".into());
        }
        let mut bytes = Vec::new();
        file.take((MAX_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > MAX_BYTES {
            return Err("Media preview limit is 30 MiB".into());
        }
        Ok(Self::new(mime, bytes))
    }

    pub(super) fn new(mime: &'static str, bytes: Vec<u8>) -> Self {
        Self {
            mime,
            bytes: Arc::new(bytes),
            #[cfg(windows)]
            failed: Default::default(),
        }
    }
    #[cfg(windows)]
    pub fn startup_failed(&self) -> bool {
        self.failed.load(std::sync::atomic::Ordering::Relaxed)
    }
    #[cfg(windows)]
    pub(crate) fn take_presented(
        ctx: &eframe::egui::Context,
    ) -> Option<(Self, eframe::egui::Rect)> {
        ctx.data_mut(|data| {
            data.remove_temp::<Option<(Self, eframe::egui::Rect)>>(eframe::egui::Id::new(
                "active-media-preview",
            ))
            .flatten()
        })
    }

    pub fn show(&self, ui: &mut eframe::egui::Ui) {
        #[cfg(windows)]
        {
            let bounds = ui.available_rect_before_wrap().intersect(ui.clip_rect());
            if self.startup_failed() {
                ui.painter().text(
                    bounds.center(),
                    eframe::egui::Align2::CENTER_CENTER,
                    startup_error(crate::i18n::is_french()),
                    eframe::egui::FontId::default(),
                    ui.visuals().text_color(),
                );
            }
            ui.allocate_rect(bounds, eframe::egui::Sense::hover());
            ui.ctx().data_mut(|data| {
                data.insert_temp(
                    eframe::egui::Id::new("active-media-preview"),
                    Some((self.clone(), bounds)),
                )
            });
        }
        #[cfg(not(windows))]
        ui.label(if crate::i18n::is_french() {
            "La lecture multimédia intégrée nécessite Windows."
        } else {
            "Embedded media playback requires Windows."
        });
    }
}

#[cfg(windows)]
pub fn startup_error(french: bool) -> &'static str {
    if french {
        "Le lecteur multimédia n’a pas pu démarrer."
    } else {
        "The media player could not start."
    }
}

#[cfg(windows)]
pub mod native {
    use super::*;
    use std::borrow::Cow;
    use wry::{
        WebView, WebViewBuilder,
        http::{Request, Response},
    };

    const PAGE: &str = "tesselmedia://localhost/player";
    const WINDOWS_PAGE: &str = "http://tesselmedia.localhost/player";

    fn page(mime: &str, french: bool) -> Vec<u8> {
        let tag = if mime.starts_with("audio/") {
            "audio"
        } else {
            "video"
        };
        let error = if french {
            "Ce format multimédia ne peut pas être lu ici."
        } else {
            "This file cannot be played here (its format is not supported)."
        };
        format!(r#"<!doctype html>
<meta charset="utf-8">
<style>
  * {{ box-sizing: border-box }}
  html, body {{ margin: 0; height: 100% }}
  body {{ display: flex; align-items: center; justify-content: center; padding: 16px; font: 13px system-ui; color: GrayText }}
  video {{ max-width: 100%; max-height: 100% }}
  audio {{ width: min(560px, 100%) }}
</style>
<{tag} controls preload="metadata" src="/original"></{tag}>
<span role="alert" hidden>{error}</span>
<script>
  const media = document.querySelector('audio,video');
  function showError() {{
    media.hidden = true;
    document.querySelector('[role="alert"]').hidden = false;
  }}
  media.addEventListener('error', showError);
  if (media.error) showError();
</script>"#).into_bytes()
    }

    pub(crate) fn byte_range(header: &str, len: u64) -> Option<std::ops::Range<u64>> {
        let (start, end) = header.strip_prefix("bytes=")?.split_once('-')?;
        let number = |s: &str| {
            (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
                .then(|| s.parse::<u64>().ok())
                .flatten()
        };
        if start.is_empty() {
            let count = number(end)?.min(len);
            return (count > 0).then_some(len - count..len);
        }
        let start = number(start)?;
        let end = if end.is_empty() {
            len
        } else {
            number(end)?.saturating_add(1).min(len)
        };
        (start < end).then_some(start..end)
    }

    fn serve(
        source: &Source,
        french: bool,
        request: &Request<Vec<u8>>,
    ) -> Response<Cow<'static, [u8]>> {
        let mut reply = Response::builder()
            .header("Cache-Control", "no-store")
            .header("X-Content-Type-Options", "nosniff");
        let head = request.method() == "HEAD";
        let empty = Cow::Owned(Vec::new());
        if !head && request.method() != "GET" {
            return reply
                .status(405)
                .header("Allow", "GET, HEAD")
                .body(empty)
                .unwrap();
        }
        if request.uri().authority().map(|a| a.as_str()) != Some("localhost") {
            return reply.status(404).body(empty).unwrap();
        }
        match request.uri().path() {
            "/player" => {
                let html = page(source.mime, french);
                reply.header("Content-Security-Policy","default-src 'none'; media-src 'self'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'")
                    .header("Content-Type","text/html; charset=utf-8").header("Content-Length",html.len()).body(if head {empty} else {Cow::Owned(html)}).unwrap()
            }
            "/original" => {
                let len = source.bytes.len();
                reply = reply
                    .header("Accept-Ranges", "bytes")
                    .header("Content-Type", source.mime);
                let range = if let Some(header) = request.headers().get("Range") {
                    let Some(range) = header.to_str().ok().and_then(|s| byte_range(s, len as u64))
                    else {
                        return reply
                            .status(416)
                            .header("Content-Range", format!("bytes */{len}"))
                            .body(empty)
                            .unwrap();
                    };
                    reply = reply.status(206).header(
                        "Content-Range",
                        format!("bytes {}-{}/{len}", range.start, range.end - 1),
                    );
                    range.start as usize..range.end as usize
                } else {
                    0..len
                };
                reply
                    .header("Content-Length", range.len())
                    .body(if head {
                        empty
                    } else {
                        Cow::Owned(source.bytes[range].to_vec())
                    })
                    .unwrap()
            }
            _ => reply.status(404).body(empty).unwrap(),
        }
    }

    #[derive(Default)]
    pub struct Player {
        view: Option<WebView>,
        context: Option<wry::WebContext>,
        profile: Option<tempfile::TempDir>,
        source: Option<Source>,
        bounds: Option<wry::Rect>,
        hidden: bool,
    }
    impl Player {
        pub fn clear(&mut self) {
            self.view.take();
            self.source.take();
            self.bounds = None;
            self.hidden = false;
        }
        pub fn hide(&mut self) {
            if !self.hidden
                && let Some(view) = &self.view
                && view.set_visible(false).is_ok()
            {
                self.hidden = true;
            }
        }

        pub fn retain(&mut self, source: Option<&Source>) {
            if self
                .source
                .as_ref()
                .is_some_and(|old| !source.is_some_and(|new| Arc::ptr_eq(&old.bytes, &new.bytes)))
            {
                self.clear();
            }
        }
        pub fn sync(
            &mut self,
            parent: &impl wry::raw_window_handle::HasWindowHandle,
            source: &Source,
            bounds: eframe::egui::Rect,
            scale: f32,
            french: bool,
        ) {
            if !bounds.is_positive() {
                self.clear();
                return;
            }
            let rect = wry::Rect {
                position: wry::dpi::PhysicalPosition::new(
                    (bounds.min.x * scale).round() as i32,
                    (bounds.min.y * scale).round() as i32,
                )
                .into(),
                size: wry::dpi::PhysicalSize::new(
                    (bounds.width() * scale).round().max(1.) as u32,
                    (bounds.height() * scale).round().max(1.) as u32,
                )
                .into(),
            };
            if self
                .source
                .as_ref()
                .is_some_and(|old| Arc::ptr_eq(&old.bytes, &source.bytes))
            {
                if self.hidden
                    && let Some(view) = &self.view
                    && view.set_visible(true).is_ok()
                {
                    self.hidden = false;
                }
                if self.bounds != Some(rect)
                    && let Some(view) = &self.view
                    && view.set_bounds(rect).is_ok()
                {
                    self.bounds = Some(rect);
                }
                return;
            }
            self.clear();
            self.source = Some(source.clone());
            source
                .failed
                .store(true, std::sync::atomic::Ordering::Relaxed);
            // All documents in this window share one isolated runtime profile.
            // Each child view still serves only the bytes captured for its document.
            if self.context.is_none() {
                let Ok(profile) = tempfile::tempdir() else {
                    return;
                };
                self.context = Some(wry::WebContext::new(Some(profile.path().to_path_buf())));
                self.profile = Some(profile);
            }
            let Some(context) = &mut self.context else {
                return;
            };
            let src = source.clone();
            let result = WebViewBuilder::new_with_web_context(context)
                .with_incognito(true)
                .with_focused(false)
                .with_clipboard(false)
                .with_devtools(false)
                .with_transparent(true)
                .with_custom_protocol("tesselmedia".into(), move |_, req| {
                    serve(&src, french, &req)
                })
                .with_navigation_handler(|url| url == PAGE || url == WINDOWS_PAGE)
                .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
                .with_download_started_handler(|_, _| false)
                .with_drag_drop_handler(|_| true)
                .with_url(PAGE)
                .with_bounds(rect)
                .build_as_child(parent);
            if let Ok(view) = result {
                self.bounds = Some(rect);
                self.view = Some(view);
                source
                    .failed
                    .store(false, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
    #[cfg(test)]
    mod tests {
        include!("playback_tests.rs");
    }
}
