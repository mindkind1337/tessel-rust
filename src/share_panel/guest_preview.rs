use crate::{guest_file_editor::Key, share::GuestSession};
use eframe::egui;
use std::{
    sync::{
        Arc, OnceLock,
        mpsc::{Receiver, TryRecvError},
    },
    time::Duration,
};

fn next_preview_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}
enum Original {
    Image(Arc<egui::ColorImage>),
    Pdf(Vec<u8>),
}

#[derive(Default)]
pub(super) struct FilePreview {
    pub key: Option<Key>,
    pending: Option<Receiver<Option<Original>>>,
    pub pdf: Option<(u64, std::cell::RefCell<crate::pdf::PdfView>)>,
    image: Option<Arc<egui::ColorImage>>,
    native: OnceLock<Arc<gpui::RenderImage>>,
    pub texture: Option<egui::TextureHandle>,
    pub menu: Option<(f32, f32, f32)>, // Pointer x/y and window height at opening.
    pub failed: bool,
}
impl FilePreview {
    pub fn supports(path: &str) -> bool {
        path.rsplit_once('.').is_some_and(|(_, ext)| {
            ext.eq_ignore_ascii_case("pdf")
                || crate::raster_preview::supported(&ext.to_ascii_lowercase())
        })
    }
    pub fn open(&mut self, key: Key, session: &GuestSession) {
        *self = Self::default();
        let request =
            serde_json::json!({"op":"fs.download","projectId":key.scope.project,"path":key.path});
        let extension = key
            .path
            .rsplit_once('.')
            .map(|(_, ext)| ext.to_ascii_lowercase())
            .unwrap_or_default();
        self.key = Some(key);
        let Ok(reply) = session.request(request) else {
            self.failed = true;
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        self.pending = Some(rx);
        std::thread::spawn(move || {
            let image = reply
                .recv_timeout(Duration::from_secs(25))
                .ok()
                .and_then(Result::ok)
                .and_then(|v| super::guest_download::original_bytes(&v))
                .and_then(|b| {
                    if extension == "pdf" {
                        Some(Original::Pdf(b))
                    } else {
                        crate::raster_preview::decode(&extension, &b)
                            .ok()
                            .map(|image| Original::Image(Arc::new(image)))
                    }
                });
            let _ = tx.send(image);
        });
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.pending else {
            return false;
        };
        let original = match rx.try_recv() {
            Ok(image) => image,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => None,
        };
        self.pending = None;
        match original {
            Some(Original::Image(image)) => self.image = Some(image),
            Some(Original::Pdf(bytes)) => {
                self.pdf = crate::pdf::PdfView::from_bytes(bytes)
                    .ok()
                    .map(|pdf| (next_preview_id(), pdf.into()));
            }
            None => {}
        }
        self.failed = self.image.is_none() && self.pdf.is_none();
        true
    }
    pub fn native(&self) -> Option<Arc<gpui::RenderImage>> {
        let image = self.image.as_ref()?;
        if self.native.get().is_none() {
            self.native
                .set(crate::gpui_pixels::from_color_image(image)?)
                .ok();
        }
        self.native.get().cloned()
    }
    pub fn texture(&mut self, ctx: &egui::Context) -> Option<&egui::TextureHandle> {
        let image = self.image.as_ref()?;
        Some(self.texture.get_or_insert_with(|| {
            ctx.load_texture("guest-preview", image.clone(), egui::TextureOptions::LINEAR)
        }))
    }
    pub fn error(&self, fr: bool) -> &'static str {
        if self
            .key
            .as_ref()
            .is_some_and(|key| key.path.to_ascii_lowercase().ends_with(".pdf"))
        {
            return if fr {
                "Aperçu PDF indisponible."
            } else {
                "PDF preview unavailable."
            };
        }
        if fr {
            "Aperçu de l’image indisponible."
        } else {
            "Image preview unavailable."
        }
    }
}
