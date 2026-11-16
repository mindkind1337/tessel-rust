//! The two entries and inline 16px toolbar glyph from App.vue's Issues menu.
use crate::toolbar_fit::MoreMenuAction;
use eframe::egui::{self, Color32, Response, Sense, Stroke, Vec2, WidgetInfo, WidgetType};
pub const SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none" stroke="black"><circle cx="8" cy="8" r="5.7" stroke-width="1.3"/><path d="M8 4.5v4M8 11h.01" stroke-width="1.6" stroke-linecap="round"/></svg>"#;
pub fn entries(french: bool) -> [(MoreMenuAction, &'static str); 2] {
    [
        (
            MoreMenuAction::GitHub,
            if french {
                "Tickets et PR GitHub"
            } else {
                "GitHub issues and PRs"
            },
        ),
        (
            MoreMenuAction::Linear,
            if french {
                "Issues Linear"
            } else {
                "Linear issues"
            },
        ),
    ]
}
pub fn button(ui: &mut egui::Ui, open: bool, french: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(30., 28.), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), "Issues"));
    let visuals = ui.style().interact_selectable(&response, open);
    let color = visuals.fg_stroke.color;
    ui.painter().rect_filled(
        rect,
        4.,
        if open || response.hovered() {
            visuals.bg_fill
        } else {
            Color32::TRANSPARENT
        },
    );
    let center = rect.center();
    ui.painter()
        .circle_stroke(center, 5.7, Stroke::new(1.3_f32, color));
    ui.painter().line_segment(
        [center + Vec2::new(0., -3.5), center + Vec2::new(0., 0.5)],
        Stroke::new(1.6_f32, color),
    );
    ui.painter()
        .circle_filled(center + Vec2::new(0., 3.), 0.8, color);
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            4.,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Inside,
        );
    }
    response.on_hover_text(if french {
        "Issues GitHub et Linear"
    } else {
        "GitHub and Linear issues"
    })
}

pub fn contents(
    ui: &mut egui::Ui,
    palette: &crate::theme_palettes::UiPalette,
    french: bool,
) -> Option<MoreMenuAction> {
    ui.set_min_width(188.);
    ui.spacing_mut().item_spacing.y = 0.;
    let mut selected = None;
    for (action, label) in entries(french) {
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.), Sense::click());
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), label));
        if response.hovered() || response.has_focus() {
            ui.painter().rect_filled(rect, 5., palette.surface_3);
        }
        ui.painter().text(
            rect.left_center() + Vec2::new(8., 0.),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(12.),
            palette.text,
        );
        if response.clicked() {
            selected = Some(action);
        }
    }
    selected
}
