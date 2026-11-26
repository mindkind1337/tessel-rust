use crate::{guest_file_editor::Key, share::GuestSession};
use eframe::egui;
use std::{
    sync::{
        Arc, OnceLock,
        mpsc::{Receiver, TryRecvError},
    },
    time::Duration,
};

#[derive(Default)]
pub(super) struct ImagePreview {
    pub key: Option<Key>,
    pending: Option<Receiver<Option<Arc<egui::ColorImage>>>>,
    image: Option<Arc<egui::ColorImage>>,
    native: OnceLock<Arc<gpui::RenderImage>>,
    pub texture: Option<egui::TextureHandle>,
    pub menu: bool,
    pub failed: bool,
}
impl ImagePreview {
    pub fn supports(path: &str) -> bool {
        path.rsplit_once('.')
            .is_some_and(|(_, ext)| crate::raster_preview::supported(&ext.to_ascii_lowercase()))
    }
    pub fn open(&mut self, key: Key, session: &GuestSession) {
        *self = Self::default();
        let request =
            serde_json::json!({"op":"fs.download","projectId":key.scope.project,"path":key.path});
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
                .and_then(|b| crate::raster_preview::decode(&b).ok())
                .map(Arc::new);
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
        self.image = match rx.try_recv() {
            Ok(image) => image,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => None,
        };
        self.pending = None;
        self.failed = self.image.is_none();
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
    pub fn error(fr: bool) -> &'static str {
        if fr {
            "Aperçu de l’image indisponible."
        } else {
            "Image preview unavailable."
        }
    }
}
