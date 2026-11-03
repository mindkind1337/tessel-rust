//! Workspace status chrome; only caller-supplied, observed state is displayed.
use eframe::egui::{self, Color32, FontId, Rect, Response, Sense, Vec2};

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

pub fn show(ui: &mut egui::Ui, target: &str, count: usize, path: &str, french: bool) {
    let room = ui.available_rect_before_wrap();
    let full = ui.ctx().content_rect().width() >= crate::sidebar_model::STATUS_BAR_COMPACT_BELOW;
    let summary = match (french, count) {
        (true, 1) => "1 panneau".to_owned(),
        (true, _) => format!("{count} panneaux"),
        (false, 1) => "1 pane".to_owned(),
        (false, _) => format!("{count} panes"),
    };
    let target = if target.is_empty() {
        String::new()
    } else if french {
        format!("Saisie → {target}")
    } else {
        format!("Input → {target}")
    };
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
