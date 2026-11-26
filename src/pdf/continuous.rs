//! Continuous PDF pages for the Compatibility viewport.
use super::{MAX_PIXELS, MAX_ZOOM, MIN_ZOOM, PdfView, Rendered, dimensions, manual_zoom, tr};
use eframe::egui;
use std::{collections::BTreeMap, time::Duration};

#[derive(Default)]
pub(super) struct ContinuousPages {
    sizes: Vec<[f32; 2]>,
    textures: BTreeMap<usize, (f32, egui::TextureHandle)>,
    failures: BTreeMap<usize, (f32, String)>,
}

impl ContinuousPages {
    pub(super) fn store(&mut self, ctx: &egui::Context, index: usize, scale: f32, page: Rendered) {
        self.sizes = page.page_sizes;
        self.failures.remove(&index);
        self.textures.insert(
            index,
            (
                scale,
                ctx.load_texture("pdf-page", page.image, egui::TextureOptions::LINEAR),
            ),
        );
    }

    fn paint(
        &mut self,
        ui: &mut egui::Ui,
        viewport: egui::Rect,
        scale: f32,
        widest: f32,
    ) -> Option<usize> {
        let width = viewport.width().max(widest * scale + 48.);
        let origin = ui.min_rect().min;
        let render_scale = scale.clamp(MIN_ZOOM, MAX_ZOOM);
        let mut top = 16.;
        let mut nearby = Vec::new();
        let visible = viewport.expand2(egui::vec2(0., 600.));
        for (index, &[w, h]) in self.sizes.iter().enumerate() {
            let size = egui::vec2((w * scale).floor(), (h * scale).floor());
            let rect = egui::Rect::from_min_size(egui::pos2((width - size.x) / 2., top), size);
            top += size.y + 12.;
            if !visible.intersects(rect) {
                continue;
            }
            let distance = rect.distance_to_pos(viewport.center());
            nearby.push((index, distance));
            if viewport.intersects(rect) {
                let screen = rect.translate(origin.to_vec2());
                ui.painter().rect_filled(screen, 0., egui::Color32::WHITE);
                if let Some((_, texture)) = self.textures.get(&index) {
                    ui.painter().image(
                        texture.id(),
                        screen,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                        egui::Color32::WHITE,
                    );
                }
            }
        }
        ui.set_min_size(egui::vec2(width, top + 4.));
        nearby.sort_by(|a, b| a.1.total_cmp(&b.1));
        // Prefer visible pages; reserve at most two full-resolution pages' worth
        // of textures. Small pages share that budget.
        let mut remaining = MAX_PIXELS * 2;
        let mut wanted = Vec::new();
        for (index, _) in nearby {
            let [w, h] = self.sizes[index];
            let pixels = dimensions(w, h, render_scale).map_or(1, |s| {
                ((w * s).ceil() as usize)
                    .saturating_mul((h * s).ceil() as usize)
                    .min(MAX_PIXELS)
            });
            if pixels <= remaining {
                remaining -= pixels;
                wanted.push(index);
            }
        }
        self.textures.retain(|index, _| wanted.contains(index));
        wanted.into_iter().find(|index| {
            self.failures
                .get(index)
                .is_none_or(|(s, _)| *s != render_scale)
                && self
                    .textures
                    .get(index)
                    .is_none_or(|(s, _)| *s != render_scale)
        })
    }
}

impl PdfView {
    pub fn show(&mut self, ui: &mut egui::Ui) {
        if let Some(result) = self.take_completed() {
            match result {
                Ok(page) => {
                    self.widest = page.widest;
                    self.pages = page.pages;
                    self.continuous.store(
                        ui.ctx(),
                        self.requested_page,
                        self.requested_scale,
                        page,
                    );
                }
                Err(error) if self.pages == 0 => self.error = error,
                Err(error) => {
                    self.continuous
                        .failures
                        .insert(self.requested_page, (self.requested_scale, error));
                }
            }
        }
        self.set_viewport(ui.available_width());
        let effective = self.effective_scale();
        ui.horizontal(|ui| {
            if self.pages > 0 {
                ui.label(format!(
                    "{} {}",
                    self.pages,
                    if self.pages == 1 { "page" } else { "pages" }
                ));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.selectable_label(self.fit, tr("Fit width")).clicked() {
                    self.fit_width();
                }
                if ui
                    .add_enabled(effective < MAX_ZOOM, egui::Button::new("+"))
                    .clicked()
                {
                    self.scale = manual_zoom(effective, 0.25);
                    self.fit = false;
                }
                ui.label(format!("{:.0} %", effective * 100.));
                if ui
                    .add_enabled(effective > MIN_ZOOM, egui::Button::new("−"))
                    .clicked()
                {
                    self.scale = manual_zoom(effective, -0.25);
                    self.fit = false;
                }
            });
        });
        if !self.error.is_empty() {
            ui.colored_label(egui::Color32::LIGHT_RED, &self.error);
            if ui.button(tr("Retry page")).clicked() {
                self.error.clear();
            }
        }
        if let Some((_, (_, error))) = self.continuous.failures.first_key_value() {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
            if ui.button(tr("Retry page")).clicked() {
                self.continuous.failures.clear();
            }
        }
        let scale = self.effective_scale();
        let next = if self.continuous.sizes.is_empty() {
            if self.error.is_empty() {
                ui.spinner();
            }
            self.error.is_empty().then_some(0)
        } else {
            egui::ScrollArea::both()
                .id_salt(("continuous-pdf", std::sync::Arc::as_ptr(&self.bytes)))
                .show_viewport(ui, |ui, viewport| {
                    self.continuous.paint(ui, viewport, scale, self.widest)
                })
                .inner
        };
        if self.pending.is_none()
            && let Some(page) = next
        {
            self.start_page_worker(page, scale.clamp(MIN_ZOOM, MAX_ZOOM));
        }
        if self.pending.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(40));
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scheduler_prioritizes_visible_pages_and_releases_distant_textures() {
        let ctx = egui::Context::default();
        let mut pages = ContinuousPages {
            sizes: vec![[300., 800.]; 40],
            ..Default::default()
        };
        let choose = |pages: &mut ContinuousPages, index: usize, scale: f32| {
            let mut selected = None;
            let viewport = egui::Rect::from_min_size(
                egui::pos2(0., 16. + index as f32 * (800. * scale + 12.)),
                egui::vec2(400., 300.),
            );
            let _ = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| {
                        selected = pages.paint(ui, viewport, scale, 300.);
                    })
                    .inner
            });
            selected
        };
        assert_eq!(choose(&mut pages, 22, 1.), Some(22));
        let texture = ctx.load_texture(
            "page",
            egui::ColorImage::filled([2, 2], egui::Color32::WHITE),
            Default::default(),
        );
        pages.textures.insert(22, (1., texture));
        assert_eq!(choose(&mut pages, 22, 1.), Some(21));
        assert_eq!(choose(&mut pages, 22, 2.), Some(22));
        assert_eq!(choose(&mut pages, 0, 1.), Some(0));
        assert!(pages.textures.is_empty());
    }

    #[test]
    fn scrolling_between_different_page_sizes_keeps_both_original_textures() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("original.pdf");
        let mut source = super::super::fixture();
        let at = source.windows(7).rposition(|v| v == b"300 180").unwrap();
        source[at..at + 7].copy_from_slice(b"600 320");
        std::fs::write(&path, &source).unwrap();
        let ctx = egui::Context::default();
        let mut view = PdfView::open(&path).unwrap();
        view.fit = false;
        view.scale = 1.;
        view.pages = 2;
        view.widest = 600.;
        for index in 0..2 {
            let raster = super::super::raster(source.clone(), index, 1.).unwrap();
            view.continuous.store(
                &ctx,
                index,
                1.,
                Rendered {
                    points: raster.page_sizes[index],
                    pages: 2,
                    widest: 600.,
                    page_sizes: raster.page_sizes,
                    image: egui::ColorImage::from_rgba_unmultiplied(raster.size, &raster.rgba),
                },
            );
        }
        let first = view.continuous.textures[&0].1.id();
        let second = view.continuous.textures[&1].1.id();
        let mut time = 0.;
        let mut frame = |events| {
            time += 0.025;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(690., 300.),
                    )),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| view.show(ui));
                },
            );
            ctx.tessellate(output.shapes, output.pixels_per_point)
                .into_iter()
                .filter_map(|shape| {
                    if let egui::epaint::Primitive::Mesh(mesh) = shape.primitive
                        && mesh.texture_id != egui::TextureId::default()
                    {
                        Some((mesh.texture_id, mesh.calc_bounds()))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        frame(vec![]);
        let before = frame(vec![]);
        let a = before.iter().find(|(id, _)| *id == first).unwrap().1;
        let b = before.iter().find(|(id, _)| *id == second).unwrap().1;
        assert_eq!(a.size(), egui::vec2(300., 180.));
        assert_eq!(b.size(), egui::vec2(600., 320.));
        assert_eq!(b.top() - a.bottom(), 12.);
        frame(vec![
            egui::Event::PointerMoved(egui::pos2(300., 150.)),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0., -400.),
                modifiers: Default::default(),
            },
        ]);
        for _ in 0..20 {
            frame(vec![]);
        }
        let after = frame(vec![]);
        assert!(after.iter().all(|(id, _)| *id != first));
        assert!(
            after
                .iter()
                .any(|(id, rect)| *id == second && rect.top() < b.top() - 150.)
        );
        assert!(view.pending.is_none());
    }
}
