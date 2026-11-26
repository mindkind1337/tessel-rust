pub(crate) fn supported(extension: &str) -> bool {
    ["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "svg"].contains(&extension)
}

pub(crate) fn decode(extension: &str, bytes: &[u8]) -> Result<eframe::egui::ColorImage, String> {
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
