//! Native center tabs, using the dimensions and vector controls of `ViewTabs.vue`.
use eframe::egui::{self, Color32, Rect, Stroke, Vec2};

use crate::i18n::tr;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Terminal,
    Editor,
    Diff,
    #[cfg(windows)]
    Browser,
}

#[cfg(windows)]
pub fn speaker(ui: &mut egui::Ui, muted: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(18.0), egui::Sense::click());
    let pen = Stroke::new(1.3_f32, ui.style().interact(&response).fg_stroke.color);
    let point = |x, y| rect.min + egui::vec2(x, y);
    let painter = ui.painter();
    painter.add(egui::Shape::closed_line(
        vec![
            point(2., 7.),
            point(5., 7.),
            point(8., 4.),
            point(8., 14.),
            point(5., 11.),
            point(2., 11.),
        ],
        pen,
    ));
    if muted {
        painter.line_segment([point(11., 7.), point(15., 11.)], pen);
        painter.line_segment([point(15., 7.), point(11., 11.)], pen);
    } else {
        for offset in [0., 3.] {
            painter.add(egui::Shape::line(
                vec![
                    point(10. + offset, 6. - offset / 2.),
                    point(12. + offset, 9.),
                    point(10. + offset, 12. + offset / 2.),
                ],
                pen,
            ));
        }
    }
    response.on_hover_text(tr(if muted { "Unmute tab" } else { "Mute tab" }))
}

pub struct Tab<'a> {
    pub title: &'a str,
    pub file_path: Option<&'a str>,
    pub hint: &'a str,
    pub kind: Kind,
    pub selected: bool,
    pub dirty: bool,
    pub closable: bool,
    pub preview: bool,
    pub deleted: bool,
    pub has_file_menu: bool,
}

pub struct Response {
    pub body: egui::Response,
    pub close: bool,
    pub keep_open: bool,
    pub close_others: bool,
    pub close_all: bool,
    pub copy_path: bool,
}

fn file_icon_tint(title: &str, selected: bool, accent: Color32, fallback: Color32) -> Color32 {
    let ext = title
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "rs" => Color32::from_rgb(0xde, 0xa5, 0x84),
        "ts" | "tsx" => Color32::from_rgb(0x51, 0x9a, 0xba),
        "js" | "jsx" | "mjs" | "cjs" => Color32::from_rgb(0xcb, 0xcb, 0x41),
        "vue" => Color32::from_rgb(0x41, 0xb8, 0x83),
        "json" | "toml" | "yaml" | "yml" => Color32::from_rgb(0xe5, 0xc0, 0x7b),
        "md" | "txt" => Color32::from_rgb(0x56, 0xb6, 0xc2),
        "css" | "scss" | "html" => Color32::from_rgb(0xc6, 0x78, 0xdd),
        _ if selected => accent,
        _ => fallback,
    }
}

pub fn show(ui: &mut egui::Ui, tab: Tab<'_>) -> Response {
    let font = egui::FontId::proportional(12.0);
    let danger = Color32::from_rgb(0xe0, 0x6c, 0x75);
    let base_color = if tab.deleted {
        danger
    } else if tab.selected {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    let extra = 10.0
        + 14.0
        + 6.0
        + if tab.closable { 28.0 } else { 12.0 }
        + if tab.dirty { 13.0 } else { 0.0 };
    let natural = ui
        .painter()
        .layout_no_wrap(tab.title.into(), font.clone(), base_color)
        .size()
        .x;
    let width = (natural + extra).clamp(72.0, 220.0);
    let body = ui.add_sized([width, 30.0], egui::Button::new("").frame(false));
    body.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            tab.selected,
            tab.title,
        )
    });
    let rect = body.rect;
    let fill = if tab.selected {
        ui.visuals().panel_fill
    } else if body.hovered() {
        ui.visuals().widgets.hovered.weak_bg_fill
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 0.0, fill);
    let accent = ui.visuals().selection.stroke.color;
    if tab.selected {
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min, Vec2::new(rect.width(), 2.0)),
            0.0,
            accent,
        );
    }
    if body.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            0.0,
            Stroke::new(1.0_f32, accent),
            egui::StrokeKind::Inside,
        );
    }
    ui.painter().line_segment(
        [rect.right_top(), rect.right_bottom()],
        ui.visuals().widgets.noninteractive.bg_stroke,
    );
    let icon_rect = Rect::from_center_size(
        egui::pos2(rect.left() + 17.0, rect.center().y),
        Vec2::splat(14.0),
    );
    let icon_color = if tab.selected { accent } else { base_color };
    match tab.kind {
        Kind::Terminal => crate::icons::paint(
            ui.painter(),
            icon_rect,
            crate::icons::Icon::Terminal,
            icon_color,
        ),
        Kind::Editor => {
            let file_col = file_icon_tint(
                tab.file_path.unwrap_or(tab.title),
                tab.selected,
                accent,
                base_color,
            );
            let p = |x: f32, y: f32| icon_rect.min + Vec2::new(x, y) * (14.0 / 16.0);
            for points in [
                [p(5.5, 4.5), p(2.0, 8.0), p(5.5, 11.5)],
                [p(10.5, 4.5), p(14.0, 8.0), p(10.5, 11.5)],
            ] {
                ui.painter().add(egui::Shape::line(
                    points.to_vec(),
                    Stroke::new(1.3_f32, file_col),
                ));
            }
        }
        Kind::Diff => crate::icons::paint(
            ui.painter(),
            icon_rect,
            crate::icons::Icon::GitBranch,
            Color32::from_rgb(0xe2, 0xc0, 0x8d),
        ),
        #[cfg(windows)]
        Kind::Browser => {
            let c = icon_rect.center();
            let stroke = Stroke::new(1.2_f32, icon_color);
            ui.painter().circle_stroke(c, 5.8, stroke);
            ui.painter()
                .line_segment([c - Vec2::new(5.8, 0.0), c + Vec2::new(5.8, 0.0)], stroke);
            let points: Vec<_> = (0..=32)
                .map(|i| {
                    let a = i as f32 * std::f32::consts::TAU / 32.0;
                    c + Vec2::new(a.cos() * 2.5, a.sin() * 5.8)
                })
                .collect();
            ui.painter().add(egui::Shape::line(points, stroke));
        }
    }

    let mut fmt = egui::TextFormat {
        font_id: font,
        color: base_color,
        italics: tab.preview,
        ..Default::default()
    };
    if tab.deleted {
        fmt.strikethrough = Stroke::new(1.0_f32, danger);
    }
    let mut job = egui::text::LayoutJob::default();
    job.append(tab.title, 0.0, fmt);
    job.wrap.max_width = (width - extra).max(1.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let text = ui.fonts_mut(|fonts| fonts.layout_job(job));
    ui.painter().galley(
        egui::pos2(rect.left() + 30.0, rect.center().y - text.size().y / 2.0),
        text,
        base_color,
    );

    if tab.dirty {
        let x = rect.right() - if tab.closable { 31.5 } else { 10.0 };
        ui.painter()
            .circle_filled(egui::pos2(x, rect.center().y), 3.5, base_color);
    }

    let mut close = false;
    let mut keep_open = tab.preview && ui.is_enabled() && body.double_clicked();
    let mut close_others = false;
    let mut close_all = false;
    let mut copy_path = false;

    if tab.closable {
        let close_rect = Rect::from_center_size(
            egui::pos2(rect.right() - 13.0, rect.center().y),
            Vec2::splat(18.0),
        );
        let button = ui.interact(close_rect, body.id.with("close"), egui::Sense::click());
        button.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                ui.is_enabled(),
                format!("{} {}", tr("Close"), tab.title),
            )
        });
        if tab.selected || body.hovered() || button.hovered() || button.has_focus() {
            if button.hovered() {
                ui.painter().rect_filled(
                    close_rect,
                    4.0,
                    ui.visuals().widgets.hovered.weak_bg_fill,
                );
            }
            crate::icons::paint(
                ui.painter(),
                close_rect.shrink(3.0),
                crate::icons::Icon::Close,
                base_color,
            );
        }
        close = button.clicked() || body.clicked_by(egui::PointerButton::Middle);
        button.on_hover_text(tr("Close"));
    }

    if ui.is_enabled() && (tab.closable || tab.has_file_menu) {
        body.context_menu(|ui| {
            ui.set_min_width(150.0);
            if tab.preview && ui.button(tr("Keep open")).clicked() {
                keep_open = true;
                ui.close();
            }
            if tab.closable && ui.button(tr("Close")).clicked() {
                close = true;
                ui.close();
            }
            if tab.has_file_menu {
                if ui.button(tr("Close others")).clicked() {
                    close_others = true;
                    ui.close();
                }
                if ui.button(tr("Close all")).clicked() {
                    close_all = true;
                    ui.close();
                }
                if !tab.hint.is_empty() {
                    ui.separator();
                    if ui.button(tr("Copy path")).clicked() {
                        copy_path = true;
                        ui.close();
                    }
                }
            }
        });
    }

    Response {
        body: body.on_hover_text(tab.hint),
        close,
        keep_open,
        close_others,
        close_all,
        copy_path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_button_and_middle_click_close_only_closable_enabled_tabs() {
        let ctx = egui::Context::default();
        let frame = |events, closable, enabled| {
            let mut result = None;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(500.0, 200.0),
                    )),
                    events,
                    focused: true,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.add_enabled_ui(enabled, |ui| {
                            result = Some(show(
                                ui,
                                Tab {
                                    title: "A very long modified document name.txt",
                                    file_path: Some("project/document.txt"),
                                    hint: "project/document.txt",
                                    kind: Kind::Editor,
                                    selected: true,
                                    dirty: true,
                                    closable,
                                    preview: false,
                                    deleted: false,
                                    has_file_menu: true,
                                },
                            ));
                        });
                    });
                },
            );
            result.unwrap()
        };
        let initial = frame(vec![], true, true);
        assert_eq!(initial.body.rect.height(), 30.0);
        assert!(initial.body.rect.width() <= 220.0);
        let click = |pos, button, closable, enabled| {
            let event = |pressed| egui::Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame(
                vec![egui::Event::PointerMoved(pos), event(true)],
                closable,
                enabled,
            );
            frame(vec![event(false)], closable, enabled)
        };
        let close_pos = egui::pos2(
            initial.body.rect.right() - 13.0,
            initial.body.rect.center().y,
        );
        assert!(click(close_pos, egui::PointerButton::Primary, true, true).close);
        assert!(
            click(
                initial.body.rect.center(),
                egui::PointerButton::Middle,
                true,
                true
            )
            .close
        );
        assert!(!click(close_pos, egui::PointerButton::Primary, true, false).close);
        assert!(
            !click(
                initial.body.rect.center(),
                egui::PointerButton::Middle,
                false,
                true
            )
            .close
        );
        let select = click(
            initial.body.rect.left_center() + Vec2::new(30.0, 0.0),
            egui::PointerButton::Primary,
            true,
            true,
        );
        assert!(select.body.clicked());
        assert!(!select.close);
    }
}
