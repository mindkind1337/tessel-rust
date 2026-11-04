//! Workspace status chrome; only caller-supplied, observed state is displayed.
use eframe::egui::{self, Color32, FontId, Rect, Response, Sense, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target<'a> {
    None,
    Input(&'a str),
    Broadcast(usize),
}

#[must_use]
pub fn format_target(target: Target<'_>, french: bool) -> String {
    match target {
        Target::None | Target::Input("") => String::new(),
        Target::Input(name) => {
            if french {
                format!("Saisie → {name}")
            } else {
                format!("Input → {name}")
            }
        }
        Target::Broadcast(count) => {
            crate::layout_broadcast_ui::format_broadcast_status_target(french, count)
        }
    }
}

fn text(ui: &mut egui::Ui, rect: Rect, value: &str, color: Color32) -> Response {
    let response = ui.allocate_rect(rect, Sense::hover());
    let mut job = egui::text::LayoutJob::simple(
        value.to_owned(),
        FontId::proportional(11.0),
        color,
        rect.width().max(0.0),
    );
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let galley = ui.painter().layout_job(job);
    ui.painter().with_clip_rect(rect).galley(
        egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    response.on_hover_text(value)
}

pub fn show(ui: &mut egui::Ui, target: Target<'_>, count: usize, path: &str, french: bool) {
    let room = ui.available_rect_before_wrap();
    let full = ui.ctx().content_rect().width() >= crate::sidebar_model::STATUS_BAR_COMPACT_BELOW;
    let summary = match (french, count) {
        (true, 1) => "1 panneau".to_owned(),
        (true, _) => format!("{count} panneaux"),
        (false, 1) => "1 pane".to_owned(),
        (false, _) => format!("{count} panes"),
    };
    let target = format_target(target, french);
    let color = ui.visuals().text_color();
    let weak = ui.visuals().weak_text_color();
    let mut x = room.left();
    if !target.is_empty() {
        let width = ui
            .painter()
            .layout_no_wrap(target.clone(), FontId::proportional(11.0), color)
            .size()
            .x
            .min(room.width() * 0.45);
        text(
            ui,
            Rect::from_min_size(egui::pos2(x, room.top()), Vec2::new(width, room.height())),
            &target,
            color,
        );
        x += width + 12.0;
    }
    let width = ui
        .painter()
        .layout_no_wrap(summary.clone(), FontId::proportional(11.0), weak)
        .size()
        .x;
    text(
        ui,
        Rect::from_min_size(egui::pos2(x, room.top()), Vec2::new(width, room.height())),
        &summary,
        weak,
    );
    x += width + 12.0;
    if full && x < room.right() {
        text(
            ui,
            Rect::from_min_max(egui::pos2(x, room.top()), room.max),
            path,
            weak,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_strip_target_distinguishes_single_input_and_broadcast_en_fr() {
        assert_eq!(format_target(Target::None, false), "");
        assert_eq!(format_target(Target::Input(""), true), "");
        assert_eq!(format_target(Target::Input("Ada"), false), "Input → Ada");
        assert_eq!(format_target(Target::Input("Ada"), true), "Saisie → Ada");
        assert_eq!(
            format_target(Target::Broadcast(1), false),
            "Broadcast → 1 pane"
        );
        assert_eq!(
            format_target(Target::Broadcast(3), false),
            "Broadcast → 3 panes"
        );
        assert_eq!(
            format_target(Target::Broadcast(1), true),
            "Diffusion → 1 panneau"
        );
        assert_eq!(
            format_target(Target::Broadcast(3), true),
            "Diffusion → 3 panneaux"
        );
    }
}
