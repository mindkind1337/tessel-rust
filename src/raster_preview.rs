pub(crate) fn supported(extension: &str) -> bool {
    [
        "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "svg", "avif",
    ]
    .contains(&extension)
}

pub(crate) fn decode(extension: &str, bytes: &[u8]) -> Result<eframe::egui::ColorImage, String> {
    if extension == "avif" {
        return avif(bytes);
    }
    if extension == "svg" {
        return svg(bytes);
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|e| e.to_string())?.to_rgba8();
    Ok(eframe::egui::ColorImage::from_rgba_unmultiplied(
        [decoded.width() as usize, decoded.height() as usize],
        decoded.as_raw(),
    ))
}

fn avif(bytes: &[u8]) -> Result<eframe::egui::ColorImage, String> {
    use imgalifier_avif::{Av1Backend, Decoder, Limits};
    const MAX_SIDE: u32 = 4096;
    let limits = Limits {
        max_pixels: u64::from(MAX_SIDE * MAX_SIDE),
        max_item_bytes: 16 * 1024 * 1024,
        max_threads: 1,
        ..Default::default()
    };
    let backend = Av1Backend {
        max_pixels: MAX_SIDE * MAX_SIDE,
        threads: 1,
    };
    let decoder = Decoder::new(bytes, backend, limits).map_err(|e| e.to_string())?;
    let info = decoder.info().map_err(|e| e.to_string())?;
    if info.width > MAX_SIDE || info.height > MAX_SIDE {
        return Err("Image dimensions exceed 4096 pixels".into());
    }
    let image = decoder.decode_primary().map_err(|e| e.to_string())?;
    if image.width > MAX_SIDE || image.height > MAX_SIDE {
        return Err("Image dimensions exceed 4096 pixels".into());
    }
    Ok(eframe::egui::ColorImage::from_rgba_unmultiplied(
        [image.width as usize, image.height as usize],
        &image.data,
    ))
}

fn svg(bytes: &[u8]) -> Result<eframe::egui::ColorImage, String> {
    use resvg::{tiny_skia, usvg};
    use std::sync::{Arc, OnceLock};
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();

    let source = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    let document = usvg::roxmltree::Document::parse_with_options(
        source,
        usvg::roxmltree::ParsingOptions {
            nodes_limit: 100_000,
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())?;
    let options = usvg::Options {
        fontdb: FONTS
            .get_or_init(|| {
                let mut fonts = usvg::fontdb::Database::new();
                fonts.load_system_fonts();
                Arc::new(fonts)
            })
            .clone(),
        image_href_resolver: usvg::ImageHrefResolver {
            // The image can originate on another machine. Only use bytes it supplied.
            resolve_string: Box::new(|_, _| None),
            resolve_data: Box::new(|_, data, _| {
                let reader = image::ImageReader::new(std::io::Cursor::new(data.as_slice()))
                    .with_guessed_format()
                    .ok()?;
                let format = reader.format()?;
                let (width, height) = reader.into_dimensions().ok()?;
                if width > 4096 || height > 4096 {
                    return None;
                }
                match format {
                    image::ImageFormat::Png => Some(usvg::ImageKind::PNG(data)),
                    image::ImageFormat::Jpeg => Some(usvg::ImageKind::JPEG(data)),
                    image::ImageFormat::Gif => Some(usvg::ImageKind::GIF(data)),
                    image::ImageFormat::WebP => Some(usvg::ImageKind::WEBP(data)),
                    _ => None,
                }
            }),
        },
        ..Default::default()
    };
    let tree = usvg::Tree::from_xmltree(&document, &options).map_err(|e| e.to_string())?;
    let size = tree.size();
    if size.width() > 4096.0 || size.height() > 4096.0 {
        return Err("Image dimensions exceed 4096 pixels".into());
    }
    let size = size.to_int_size();
    let mut pixels =
        tiny_skia::Pixmap::new(size.width(), size.height()).ok_or("Cannot allocate SVG preview")?;
    resvg::render(
        &tree,
        tiny_skia::Transform::identity(),
        &mut pixels.as_mut(),
    );
    Ok(eframe::egui::ColorImage::from_rgba_premultiplied(
        [size.width() as usize, size.height() as usize],
        pixels.data(),
    ))
}

#[cfg(test)]
mod tests {
    use super::decode;
    use base64::Engine as _;

    #[test]
    fn avif_preserves_color_and_alpha() {
        let image = decode("avif", include_bytes!("../tests/fixtures/alpha-bars.avif")).unwrap();
        assert_eq!(image.size, [64, 48]);
        for (index, expected) in [(8, [240u8, 32, 16, 255]), (56, [12, 40, 230, 128])] {
            let actual = image.pixels[index].to_srgba_unmultiplied();
            assert!(
                actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 4),
                "{actual:?}"
            );
        }
        assert!(decode("avif", b"invalid").is_err());
    }

    #[test]
    fn avif_rejects_large_declared_dimensions_before_pixel_decode() {
        let mut bytes = include_bytes!("../tests/fixtures/alpha-bars.avif").to_vec();
        let ispe = bytes.windows(4).position(|tag| tag == b"ispe").unwrap();
        bytes[ispe + 8..ispe + 12].copy_from_slice(&4097_u32.to_be_bytes());
        assert_eq!(
            decode("avif", &bytes).unwrap_err(),
            "Image dimensions exceed 4096 pixels"
        );
    }

    #[test]
    fn svg_uses_embedded_pixels_and_ignores_external_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("private.png");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 255, 0, 255]))
            .save(&path)
            .unwrap();
        let encoded =
            base64::engine::general_purpose::STANDARD.encode(std::fs::read(&path).unwrap());
        let source = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="2" height="1"><image width="1" height="1" xlink:href="data:image/png;base64,{encoded}"/><image x="1" width="1" height="1" xlink:href="{}"/></svg>"#,
            path.display()
        );
        let image = decode("svg", source.as_bytes()).unwrap();
        assert_eq!(image.pixels[0].to_array(), [0, 255, 0, 255]);
        assert_eq!(image.pixels[1].to_array(), [0; 4]);
    }

    #[test]
    fn svg_rejects_oversize_canvas_and_invalid_markup() {
        assert!(
            decode(
                "svg",
                br#"<svg xmlns="http://www.w3.org/2000/svg" width="8000" height="20"/>"#
            )
            .is_err()
        );
        assert!(decode("svg", b"<svg").is_err());
    }
}
