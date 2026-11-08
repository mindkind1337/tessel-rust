//! Native toolbar geometry and controls, following the renderer's toolbar v3.
use crate::{
    i18n::tr,
    icons::{self, Icon},
};
use eframe::egui::{self, Align2, FontId, Rect, Response, Sense, Stroke, StrokeKind, Vec2};

pub struct ToolbarRegions {
    pub brand: Rect,
    pub search: Rect,
    pub actions: Rect,
}

pub fn side_tab(ui: &mut egui::Ui, icon: Icon, label: &str, selected: bool) -> Response {
    side_tab_with_badge(ui, icon, label, selected, 0)
}

pub fn side_tab_with_badge(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    selected: bool,
    badge: usize,
) -> Response {
    let response = ui.add(
        egui::Button::new("")
            .frame(false)
            .min_size(Vec2::splat(36.0)),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            label,
        )
    });
    let color = if selected {
        ui.visuals().strong_text_color()
    } else if response.hovered() {
        ui.visuals().weak_text_color()
    } else {
        ui.visuals().weak_text_color().gamma_multiply(0.6)
    };
    icons::paint(
        ui.painter(),
        Rect::from_center_size(response.rect.center(), Vec2::splat(16.0)),
        icon,
        color,
    );
    if badge > 0 {
        let badge_text = if badge > 99 {
            "99+".to_owned()
        } else {
            badge.to_string()
        };
        let font = FontId::proportional(9.5);
        let galley = ui
            .painter()
            .layout_no_wrap(badge_text, font, egui::Color32::WHITE);
        let pill_w = (galley.size().x + 7.0_f32).max(15.0_f32);
        let pill_h = 14.0_f32;
        let pill_rect = Rect::from_min_size(
            egui::pos2(
                (response.rect.right() - pill_w - 2.0_f32).max(response.rect.left()),
                response.rect.top() + 3.0_f32,
            ),
            Vec2::new(pill_w, pill_h),
        );
        ui.painter()
            .rect_filled(pill_rect, 7.0_f32, ui.visuals().hyperlink_color);
        ui.painter().galley(
            egui::pos2(
                pill_rect.center().x - galley.size().x * 0.5_f32,
                pill_rect.center().y - galley.size().y * 0.5_f32,
            ),
            galley,
            egui::Color32::WHITE,
        );
    }
    if selected {
        let r = response.rect;
        ui.painter().rect_filled(
            Rect::from_min_max(
                egui::pos2(r.left() + r.width() * 0.25, r.bottom() - 2.0),
                egui::pos2(r.right() - r.width() * 0.25, r.bottom()),
            ),
            2.0,
            color,
        );
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.shrink(0.5),
            0.0,
            ui.visuals().selection.stroke,
            StrokeKind::Inside,
        );
    }
    response.on_hover_text(label)
}

#[cfg(test)]
pub fn regions(room: Rect) -> ToolbarRegions {
    let actions_width = 380.0_f32.min(room.width() * 0.62);
    regions_fit(room, actions_width, 0)
}

/// Toolbar regions for a measured right group (`actions_width`) and a fit level
/// (`toolbar_fit::toolbar_fit_level`): level 2 narrows the brand to its name and the command box
/// to its icon.
pub fn regions_fit(room: Rect, actions_width: f32, level: u8) -> ToolbarRegions {
    let brand_full = if level >= 2 { 80.0_f32 } else { 160.0_f32 };
    let actions_width = actions_width.min((room.width() - 40.0).max(0.0)).max(0.0);
    let brand_width = brand_full.min((room.width() - actions_width).max(0.0));
    let brand = Rect::from_min_size(room.min, Vec2::new(brand_width, room.height()));
    let actions = Rect::from_min_max(egui::pos2(room.max.x - actions_width, room.min.y), room.max);
    let available = (actions.min.x - brand.max.x - 32.0).max(0.0);
    let width = if level >= 2 {
        if available >= crate::toolbar_fit::TOOLBAR_COMMAND_ICON_WIDTH {
            crate::toolbar_fit::TOOLBAR_COMMAND_ICON_WIDTH
        } else {
            0.0
        }
    } else {
        available.min(440.0)
    };
    let search = Rect::from_center_size(
        egui::pos2((brand.max.x + actions.min.x) * 0.5, room.center().y),
        Vec2::new(width, 26.0),
    );
    ToolbarRegions {
        brand,
        search,
        actions,
    }
}

#[allow(dead_code)]
pub fn brand(ui: &egui::Ui, rect: Rect) {
    brand_with_build(ui, rect, env!("TESSEL_BUILD"));
}

pub fn brand_with_build(ui: &egui::Ui, rect: Rect, build_label: &str) {
    brand_with_build_fit(ui, rect, build_label, true);
}

/// The brand; `show_version == false` (fit level 2) keeps the logo and the name only.
pub fn brand_with_build_fit(ui: &egui::Ui, rect: Rect, build_label: &str, show_version: bool) {
    let logo = Rect::from_center_size(
        egui::pos2(rect.min.x + 8.0, rect.center().y),
        Vec2::splat(16.0),
    );
    icons::paint_brand(ui.painter(), logo, cfg!(debug_assertions));
    let painter = ui.painter().with_clip_rect(rect);
    painter.text(
        egui::pos2(rect.min.x + 24.0, rect.center().y),
        Align2::LEFT_CENTER,
        "Tessel",
        FontId::proportional(12.5),
        ui.visuals().text_color(),
    );
    let count = build_label
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_start_matches('b');
    if show_version {
        painter.text(
            egui::pos2(rect.min.x + 66.0, rect.center().y),
            Align2::LEFT_CENTER,
            format!("{}.{}", env!("CARGO_PKG_VERSION"), count),
            FontId::proportional(11.5),
            ui.visuals().weak_text_color(),
        );
    }
    ui.interact(rect, ui.id().with("brand"), Sense::hover())
        .on_hover_text(build_label);
}

pub fn command_button(ui: &mut egui::Ui, rect: Rect) -> Response {
    let response = ui.interact(rect, ui.id().with("command-button"), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            ui.is_enabled(),
            tr("Search panes, run a command"),
        )
    });
    let visuals = ui.style().interact(&response);
    let fill = if response.hovered() {
        visuals.weak_bg_fill
    } else {
        ui.visuals().widgets.noninteractive.bg_fill
    };
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect(
        rect,
        7.0,
        fill,
        Stroke::new(1.0_f32, visuals.bg_stroke.color),
        StrokeKind::Inside,
    );
    let color = if response.hovered() {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    // Fit level 2: the box is its icon only (30 px).
    if rect.width() < crate::toolbar_fit::TOOLBAR_CENTRE_MIN {
        icons::paint(
            &painter,
            Rect::from_center_size(rect.center(), Vec2::splat(13.0)),
            Icon::Search,
            color,
        );
        return response.on_hover_text(tr("Search panes, run a command"));
    }
    icons::paint(
        &painter,
        Rect::from_center_size(
            egui::pos2(rect.min.x + 16.5, rect.center().y),
            Vec2::splat(13.0),
        ),
        Icon::Search,
        color,
    );
    let show_chord = rect.width() >= 280.0;
    let text_right = if show_chord {
        rect.max.x - 91.0
    } else {
        rect.max.x - 6.0
    };
    let text_rect = Rect::from_min_max(
        egui::pos2(rect.min.x + 31.0, rect.min.y),
        egui::pos2(text_right.max(rect.min.x + 31.0), rect.max.y),
    );
    let mut text = egui::text::LayoutJob::simple(
        tr("Search panes, run a command").to_owned(),
        FontId::proportional(12.0),
        color,
        text_rect.width(),
    );
    text.wrap.max_rows = 1;
    text.wrap.break_anywhere = true;
    text.wrap.overflow_character = Some('…');
    let galley = painter.layout_job(text);
    let pos = egui::pos2(text_rect.min.x, rect.center().y - galley.size().y * 0.5);
    painter
        .with_clip_rect(text_rect.intersect(rect))
        .galley(pos, galley, color);
    if show_chord {
        let cap = Rect::from_center_size(
            egui::pos2(rect.max.x - 45.0, rect.center().y),
            Vec2::new(78.0, 18.0),
        );
        painter.rect_stroke(
            cap,
            4.0,
            Stroke::new(1.0_f32, ui.visuals().widgets.noninteractive.bg_stroke.color),
            StrokeKind::Inside,
        );
        painter.text(
            cap.center(),
            Align2::CENTER_CENTER,
            "Ctrl+Shift+P",
            FontId::proportional(10.5),
            color,
        );
    }
    response.on_hover_text(tr("Search panes, run a command"))
}

pub fn icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    selected: bool,
    narrow: bool,
) -> Response {
    let size = Vec2::new(if narrow { 24.0 } else { 30.0 }, 28.0);
    let response = ui.add(
        egui::Button::new("")
            .min_size(size)
            .frame(false)
            .selected(selected),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    let color = if !ui.is_enabled() {
        ui.visuals().weak_text_color().gamma_multiply(0.5)
    } else if response.hovered() || selected {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            response.rect.center(),
            Vec2::splat(if narrow { 10.0 } else { 16.0 }),
        ),
        icon,
        color,
    );
    response.on_hover_text(label)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn toolbar_regions_do_not_overlap_and_search_is_capped() {
        for width in [320.0, 520.0, 800.0, 1280.0, 1920.0] {
            let room = Rect::from_min_size(egui::pos2(10.0, 6.0), Vec2::new(width, 28.0));
            let layout = regions(room);
            assert!(room.contains_rect(layout.brand));
            assert!(room.contains_rect(layout.actions));
            assert!(layout.brand.right() <= layout.actions.left());
            assert!(layout.search.width() <= 440.0);
            if layout.search.width() > 0.0 {
                assert!(layout.search.left() >= layout.brand.right());
                assert!(layout.search.right() <= layout.actions.left());
            }
        }
    }
}
