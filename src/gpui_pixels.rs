//! Convert bounded decoded pixels once; snapshots share the native image by Arc.
use std::sync::Arc;

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
