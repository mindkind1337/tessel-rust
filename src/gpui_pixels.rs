//! Convert bounded decoded pixels once; snapshots share the native image by Arc.
use std::sync::Arc;

/// Tracks images submitted by this window. Dropping an Arc alone does not evict
/// GPUI atlas textures; the caller must explicitly release returned images.
#[derive(Default)]
pub struct PresentedImages(Vec<Arc<gpui::RenderImage>>);

impl PresentedImages {
    pub fn replace(
        &mut self,
        images: impl IntoIterator<Item = Arc<gpui::RenderImage>>,
    ) -> Vec<Arc<gpui::RenderImage>> {
        let mut next = Vec::new();
        for image in images {
            if !next.iter().any(|current| Arc::ptr_eq(current, &image)) {
                next.push(image);
            }
        }
        std::mem::replace(&mut self.0, next)
            .into_iter()
            .filter(|old| !self.0.iter().any(|current| Arc::ptr_eq(current, old)))
            .collect()
    }
}

pub fn from_color_image(source: &eframe::egui::ColorImage) -> Option<Arc<gpui::RenderImage>> {
    let [width, height] = source.size;
    let count = width.checked_mul(height)?;
    if count == 0 || count > 16 * 1024 * 1024 || count != source.pixels.len() {
        return None;
    }
    let mut bytes = Vec::with_capacity(count.checked_mul(4)?);
    for pixel in &source.pixels {
        let [r, g, b, a] = pixel.to_srgba_unmultiplied();
        bytes.extend_from_slice(&[b, g, r, a]);
    }
    let image = image::RgbaImage::from_raw(width.try_into().ok()?, height.try_into().ok()?, bytes)?;
    Some(Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
        image,
    )])))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replaced_pages_and_closed_documents_release_presented_images() {
        let pixels = eframe::egui::ColorImage::from_rgba_unmultiplied([1, 1], &[0, 0, 0, 255]);
        let first = from_color_image(&pixels).unwrap();
        let second = from_color_image(&pixels).unwrap();
        let mut presented = PresentedImages::default();
        assert!(presented.replace([first.clone(), first.clone()]).is_empty());
        assert!(presented.replace([first.clone()]).is_empty());
        let retired = presented.replace([second.clone()]);
        assert_eq!(retired.len(), 1);
        assert!(Arc::ptr_eq(&retired[0], &first));
        let retired = presented.replace([]);
        assert_eq!(retired.len(), 1);
        assert!(Arc::ptr_eq(&retired[0], &second));
        assert!(presented.replace([]).is_empty());
        // A cached document can be presented again and uploaded on demand.
        assert!(presented.replace([first]).is_empty());
    }
    #[test]
    fn native_image_preserves_dimensions_and_uses_bgra_not_rgba() {
        let image = eframe::egui::ColorImage::from_rgba_unmultiplied(
            [2, 1],
            &[255, 0, 0, 255, 0, 0, 255, 255],
        );
        let rendered = from_color_image(&image).unwrap();
        assert_eq!(
            rendered.as_bytes(0).unwrap(),
            &[0, 0, 255, 255, 255, 0, 0, 255]
        );
        assert_eq!(rendered.size(0).width.0, 2);
        assert_eq!(rendered.size(0).height.0, 1);
    }
}
