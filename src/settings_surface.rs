//! Native settings surface matching the reference page below the 40 px toolbar.
use eframe::egui::{self, Color32, Id, Rect, Ui};

/// Returns (focus search, close page), leaving keys to nested dialogs and popups.
pub fn shortcuts(ui: &mut Ui, query: &mut String) -> (bool, bool) {
    if !ui.is_enabled() || egui::Popup::is_any_open(ui.ctx()) {
        return (false, false);
    }
    ui.input_mut(|input| {
        let focus = input.consume_key(egui::Modifiers::CTRL, egui::Key::F);
        let fresh_escape = input.events.iter().any(|event| {
            matches!(event,
            egui::Event::Key { key: egui::Key::Escape, pressed: true, repeat: false, modifiers, .. }
                if *modifiers == egui::Modifiers::NONE)
        });
        // Consume repeats too: holding Escape must not clear search and then close.
        input.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
        let close = fresh_escape && query.is_empty();
        if fresh_escape {
            query.clear();
        }
        (focus, close)
    })
}

pub fn show<R>(
    ctx: &egui::Context,
    content: impl FnOnce(&mut Ui, f32) -> R,
) -> egui::ModalResponse<R> {
    let viewport = ctx.content_rect();
    let bounds = Rect::from_min_max(
        egui::pos2(
            viewport.left(),
            (viewport.top() + 40.0).min(viewport.bottom()),
        ),
        viewport.max,
    );
    let id = Id::new("settings-page");
    let area = egui::Area::new(id)
        .kind(egui::UiKind::Modal)
        .sense(egui::Sense::hover())
        .fixed_pos(bounds.min)
        .order(egui::Order::Foreground)
        .interactable(true);
    egui::Modal::new(id)
        .area(area)
        .backdrop_color(Color32::TRANSPARENT)
        .frame(egui::Frame::NONE.fill(ctx.style().visuals.panel_fill))
        .show(ctx, |ui| {
            ui.set_min_size(bounds.size());
            ui.set_max_size(bounds.size());
            ui.set_clip_rect(bounds);
            content(ui, (viewport.width() * 0.24).clamp(200.0, 290.0))
        })
}

/// Bounded scrolling page body; the navigation remains in place while it scrolls.
pub fn body(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    let compact = ui.ctx().content_rect().width() <= 760.0;
    let horizontal = if compact { 20 } else { 40 };
    let top = if compact { 24 } else { 34 };
    let bottom = if compact { 40 } else { 60 };
    egui::ScrollArea::vertical()
        .id_salt("settings-content")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let width = ui.available_width().min(870.0);
            let inset = ((ui.available_width() - width) / 2.0).max(0.0);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(inset);
                ui.allocate_ui_with_layout(
                    egui::vec2(width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(width);
                        egui::Frame::NONE
                            .inner_margin(egui::Margin {
                                left: horizontal,
                                right: horizontal,
                                top,
                                bottom,
                            })
                            .show(ui, |ui| {
                                ui.set_width((width - 2.0 * f32::from(horizontal)).max(0.0));
                                content(ui);
                            });
                    },
                );
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_clears_search_then_closes_only_on_a_new_press_and_disabled_ui_keeps_keys() {
        let ctx = egui::Context::default();
        let mut query = "ssh".to_owned();
        let mut frame = |pressed, enabled| {
            let mut result = (false, false);
            let mut escape_left = false;
            let _ = ctx.run(
                egui::RawInput {
                    focused: true,
                    events: vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.add_enabled_ui(enabled, |ui| result = shortcuts(ui, &mut query));
                        escape_left = ui.input(|i| i.key_pressed(egui::Key::Escape));
                    });
                },
            );
            (result, escape_left, query.clone())
        };
        assert_eq!(frame(true, true), ((false, false), false, String::new()));
        assert_eq!(frame(true, true), ((false, false), false, String::new()));
        frame(false, true);
        assert_eq!(frame(true, true).0, (false, true));
        frame(false, true);
        assert_eq!(frame(true, false), ((false, false), true, String::new()));
    }

    #[test]
    fn page_stays_below_toolbar_and_long_content_is_clipped_at_both_window_sizes() {
        for size in [egui::vec2(1280.0, 800.0), egui::vec2(800.0, 520.0)] {
            let ctx = egui::Context::default();
            let mut page = Rect::NOTHING;
            let mut content_clip = Rect::NOTHING;
            for _ in 0..3 {
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ctx| {
                        let response = show(ctx, |ui, nav_width| {
                            let height = ui.available_height();
                            ui.horizontal_top(|ui| {
                                ui.allocate_space(egui::vec2(nav_width, height));
                                ui.allocate_ui_with_layout(
                                    egui::vec2(ui.available_width(), height),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        body(ui, |ui| {
                                            content_clip = ui.clip_rect();
                                            for _ in 0..150 {
                                                ui.label("Settings content");
                                            }
                                        })
                                    },
                                );
                            });
                        });
                        page = response.response.rect;
                    },
                );
            }
            assert!((page.top() - 40.0).abs() < 1.0, "{page:?}");
            assert!(page.bottom() <= size.y + 1.0, "{page:?}");
            assert!(page.right() <= size.x + 1.0, "{page:?}");
            assert!(content_clip.bottom() <= size.y + 1.0);
            assert!(content_clip.top() >= 40.0);
        }
    }
}
