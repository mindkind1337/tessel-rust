//! Embedded public brand artwork, shared by both task-card renderers.
//! See THIRD_PARTY_NOTICES.md for MIT Lovecast Inc. and LobeHub attribution.
//! No runtime paths, network requests, scripts, fonts or user profiles are used.
use eframe::egui::{self, Color32, Pos2, Rect, Stroke};
use gpui::{AssetRegistry, ParsedSvg, RenderImage, SvgRenderer};
use std::{
    cell::RefCell,
    collections::HashMap,
    sync::{Arc, OnceLock},
};

pub const FALLBACK_SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none"><rect x="2" y="3.5" width="20" height="17" rx="3" stroke="black" stroke-width="1.5"/><path d="M6.5 9.5l3.2 2.5-3.2 2.5M11.5 15h6" stroke="black" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>"#;

fn catalog() -> &'static HashMap<String, String> {
    static CATALOG: OnceLock<HashMap<String, String>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        #[derive(serde::Deserialize)]
        struct Catalog {
            icons: HashMap<String, String>,
        }
        serde_json::from_str::<Catalog>(include_str!("agent_brand_assets.json"))
            .map(|c| c.icons)
            .unwrap_or_default()
    })
}

pub fn canonical(kind: &str) -> Option<&'static str> {
    let key = kind.to_ascii_lowercase();
    catalog().get_key_value(&key).map(|(key, _)| key.as_str())
}

/// Thirteen quarter-DPI buckets, bounded even for invalid input or extreme zoom.
fn bucket(scale: f32) -> u8 {
    if !scale.is_finite() {
        return 4;
    }
    (scale.clamp(1., 4.) * 4.).ceil() as u8
}

struct Cache {
    renderer: SvgRenderer,
    parsed: HashMap<&'static str, Option<ParsedSvg>>,
    images: HashMap<(&'static str, u8), Option<Arc<RenderImage>>>,
}
impl Default for Cache {
    fn default() -> Self {
        Self {
            renderer: SvgRenderer::new(Arc::new(AssetRegistry::default())),
            parsed: HashMap::new(),
            images: HashMap::new(),
        }
    }
}
thread_local! { static CACHE: RefCell<Cache> = RefCell::new(Cache::default()); }

pub fn raster(kind: &str, scale: f32) -> Option<Arc<RenderImage>> {
    let kind = canonical(kind)?;
    let dpi = bucket(scale);
    CACHE.with(|cell| {
        let mut cache = cell.borrow_mut();
        if let Some(image) = cache.images.get(&(kind, dpi)) {
            return image.clone();
        }
        if !cache.parsed.contains_key(kind) {
            let parsed = cache
                .renderer
                .parse_svg(catalog().get(kind)?.as_bytes())
                .ok();
            cache.parsed.insert(kind, parsed);
        }
        // GPUI adds its normal 2x SVG supersampling; this is not a fixed large bitmap.
        let image = cache
            .parsed
            .get(kind)?
            .as_ref()
            .and_then(|svg| cache.renderer.render_parsed(svg, f32::from(dpi) / 4.).ok());
        cache.images.insert((kind, dpi), image.clone());
        image
    })
}

pub fn paint(ui: &egui::Ui, rect: Rect, kind: &str, color: Color32) {
    if let Some(kind) = canonical(kind) {
        let dpi = bucket(ui.ctx().pixels_per_point());
        let id = egui::Id::new(("task-brand-texture", kind, dpi));
        let cached = ui
            .ctx()
            .data_mut(|data| data.get_temp::<egui::TextureHandle>(id));
        let texture = cached.or_else(|| {
            let image = raster(kind, f32::from(dpi) / 4.)?;
            let size = image.size(0);
            let mut rgba = image.as_bytes(0)?.to_vec();
            for pixel in rgba.as_chunks_mut::<4>().0 {
                pixel.swap(0, 2);
            }
            let pixels = egui::ColorImage::from_rgba_unmultiplied(
                [size.width.0 as usize, size.height.0 as usize],
                &rgba,
            );
            let texture = ui.ctx().load_texture(
                format!("task-brand-{kind}-{dpi}"),
                pixels,
                egui::TextureOptions::LINEAR,
            );
            ui.ctx()
                .data_mut(|data| data.insert_temp(id, texture.clone()));
            Some(texture)
        });
        if let Some(texture) = texture {
            ui.painter().image(
                texture.id(),
                rect,
                Rect::from_min_max(Pos2::ZERO, egui::pos2(1., 1.)),
                Color32::WHITE,
            );
            return;
        }
    }
    // Unknown kinds have no label/accent in TaskCard.vue: use currentColor, no initial badge.
    let s = rect.width().min(rect.height()) / 24.;
    let p = |x, y| rect.min + egui::vec2(x * s, y * s);
    ui.painter().rect_stroke(
        Rect::from_min_max(p(2., 3.5), p(22., 20.5)),
        (3. * s).round() as u8,
        Stroke::new(1.5 * s, color),
        egui::StrokeKind::Middle,
    );
    ui.painter().add(egui::Shape::line(
        vec![p(6.5, 9.5), p(9.7, 12.), p(6.5, 14.5)],
        Stroke::new(1.6 * s, color),
    ));
    ui.painter()
        .line_segment([p(11.5, 15.), p(17.5, 15.)], Stroke::new(1.6 * s, color));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn embedded_brands_decode_with_stable_identity_and_preserve_color() {
        assert_eq!(catalog().len(), 50);
        for kind in catalog().keys() {
            let image = raster(kind, 1.).unwrap_or_else(|| panic!("invalid artwork: {kind}"));
            assert_eq!(image.frame_count(), 1);
            assert!(
                image
                    .as_bytes(0)
                    .unwrap()
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|p| p[3] > 0),
                "{kind}"
            );
            assert!(Arc::ptr_eq(&image, &raster(kind, 1.).unwrap()));
        }
        for kind in ["gemini", "opencode", "omp", "kilo", "antigravity"] {
            let image = raster(kind, 1.25).unwrap();
            let colors: std::collections::HashSet<_> = image
                .as_bytes(0)
                .unwrap()
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[3] > 200)
                .map(|p| [p[0], p[1], p[2]])
                .collect();
            assert!(colors.len() > 1, "lost brand colors: {kind}");
        }
        assert!(raster("unknown-custom-agent", 1.).is_none());
        assert_eq!(canonical("CODEX"), Some("codex"));
        assert_eq!(bucket(f32::NAN), 4);
        assert_eq!(bucket(1000.), 16);
        assert_eq!(bucket(-1.), 4);
    }
    #[test]
    fn brand_alpha_and_texture_cache_preserve_edges_and_stable_uploads() {
        let image = raster("codex", 1.).unwrap();
        let edge = image
            .as_bytes(0)
            .unwrap()
            .as_chunks::<4>()
            .0
            .iter()
            .find(|p| (80..180).contains(&p[3]))
            .expect("antialiased edge");
        assert!(
            edge[..3].iter().all(|c| (230..=240).contains(c)),
            "expected straight-alpha BGRA, got {edge:?}"
        );
        assert_eq!(image.size(0).width.0, 26);
        assert_eq!(raster("codex", 4.).unwrap().size(0).width.0, 104);
        let ctx = egui::Context::default();
        let render = || {
            ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    paint(
                        ui,
                        Rect::from_min_size(egui::pos2(8., 8.), egui::vec2(13., 13.)),
                        "codex",
                        Color32::WHITE,
                    );
                    paint(
                        ui,
                        Rect::from_min_size(egui::pos2(28., 8.), egui::vec2(13., 13.)),
                        "unknown",
                        Color32::RED,
                    );
                });
            })
        };
        let first = render();
        assert!(!first.textures_delta.set.is_empty());
        let second = render();
        assert!(
            second.textures_delta.set.is_empty(),
            "unchanged icon must not upload again"
        );
        assert!(
            second.shapes.iter().any(
                |s| matches!(&s.shape, egui::Shape::Rect(r) if r.stroke.color == Color32::RED)
            )
        );
    }
}
