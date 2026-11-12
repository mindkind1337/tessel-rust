//! Command palette UI. Only returns IDs from the supplied catalog.
use crate::palette::{CommandPaletteState, PaletteCommand, filter_palette_commands};
use eframe::egui::{self, Color32, FontId, Key, Modifiers, Rect, Vec2};

pub fn show(
    ctx: &egui::Context,
    state: &mut CommandPaletteState,
    focused: &mut bool,
    commands: &[PaletteCommand],
) -> Option<String> {
    if !state.open {
        *focused = false;
        return None;
    }
    let bounds = ctx.content_rect();
    let width = (bounds.width() - 32.0).clamp(200.0, 600.0);
    let height = (bounds.height() - 100.0).clamp(100.0, 520.0);
    let id = egui::Id::new("commands");
    let area = egui::Area::new(id)
        .kind(egui::UiKind::Modal)
        .sense(egui::Sense::hover())
        .fixed_pos(egui::pos2(
            bounds.center().x - width / 2.0,
            bounds.top() + 64.0,
        ))
        .order(egui::Order::Foreground);
    let mut selected = None;
    let response = egui::Modal::new(id).area(area)
        .backdrop_color(Color32::from_rgba_unmultiplied(5,6,8,115))
        .frame(egui::Frame::popup(&ctx.style()).fill(ctx.style().visuals.panel_fill).corner_radius(12).inner_margin(12))
        .show(ctx, |ui| {
            ui.set_width(width-24.0);
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(15.0), egui::Sense::hover());
                crate::icons::paint(ui.painter(), rect, crate::icons::Icon::Search, ui.visuals().weak_text_color());
                let edit = ui.add_sized([ui.available_width()-38.0, 32.0], egui::TextEdit::singleline(&mut state.query)
                    .char_limit(256).font(FontId::proportional(13.5)).frame(false)
                    .hint_text(crate::i18n::tr("Search panes, workspaces and commands")));
                if !*focused { edit.request_focus(); *focused=true; }
                if edit.changed() { state.index=0; }
                ui.weak("Esc");
            });
            ui.separator();
            let rows = filter_palette_commands(commands, &state.query);
            state.index = state.index.min(rows.len().saturating_sub(1));
            let (step, enter) = ui.input_mut(|input| {
                let down = input.consume_key(Modifiers::NONE, Key::ArrowDown);
                let up = input.consume_key(Modifiers::NONE, Key::ArrowUp);
                let enter = input.events.iter().any(|event| matches!(event,
                    egui::Event::Key { key: Key::Enter, pressed: true, repeat: false, modifiers, .. } if *modifiers == Modifiers::NONE));
                input.consume_key(Modifiers::NONE, Key::Enter);
                (if down { 1 } else if up { -1 } else { 0 }, enter)
            });
            if step != 0 {
                ui.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
                state.move_selection(step, rows.len());
            }
            if enter { selected = state.selected_id(&rows).map(str::to_owned); }
            egui::ScrollArea::vertical().id_salt("command-results").max_height(height-70.0).show(ui, |ui| {
                if rows.is_empty() {
                    let trimmed = state.query.trim();
                    if trimmed.is_empty() {
                        ui.weak(crate::i18n::tr("No matching commands"));
                    } else {
                        ui.weak(
                            crate::i18n::tr("Nothing matches \"{{query}}\"")
                                .replace("{{query}}", trimmed),
                        );
                    }
                }
                for (index, row) in rows.iter().enumerate() {
                    if let Some(group) = &row.header { ui.add_space(6.0); ui.weak(group); }
                    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(),32.0), egui::Sense::CLICK);
                    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, ui.is_enabled(), state.index==index, &row.command.title));
                    if state.index==index || response.hovered() {
                        ui.painter().rect_filled(rect,7.0,ui.visuals().widgets.active.weak_bg_fill);
                    }
                    let shortcut_width = row.command.shortcut.as_ref().map_or(0.0, |s| ui.painter().layout_no_wrap(s.clone(),FontId::proportional(11.0),Color32::WHITE).size().x + 16.0);
                    let usable = (rect.width()-20.0-shortcut_width).max(0.0);
                    let title_width = if row.command.hint.is_some() { usable*0.55 } else { usable };
                    text(ui, Rect::from_min_size(rect.min+egui::vec2(10.0,0.0),egui::vec2(title_width,32.0)), &row.command.title,12.5,ui.visuals().text_color());
                    if let Some(hint)=&row.command.hint {
                        text(ui,Rect::from_min_size(rect.min+egui::vec2(20.0+title_width,0.0),egui::vec2((usable-title_width-10.0).max(0.0),32.0)),hint,11.5,ui.visuals().weak_text_color());
                    }
                    if let Some(shortcut)=&row.command.shortcut {
                        ui.painter().text(rect.right_center()-egui::vec2(10.0,0.0),egui::Align2::RIGHT_CENTER,shortcut,FontId::proportional(11.0),ui.visuals().weak_text_color());
                    }
                    if response.clicked() { selected=Some(row.command.id.clone()); }
                    if response.hovered() && ui.input(|i| i.pointer.delta()!=Vec2::ZERO) { state.index=index; }
                    if step!=0 && state.index==index { response.scroll_to_me(Some(egui::Align::Center)); }
                    if let Some(hint)=&row.command.hint { response.on_hover_text(hint); }
                }
            });
        });
    if selected.is_some() || response.should_close() {
        state.close();
        *focused = false;
    }
    selected
}

fn text(ui: &egui::Ui, rect: Rect, text: &str, size: f32, color: Color32) {
    if rect.width() <= 0.0 {
        return;
    }
    let mut job =
        egui::text::LayoutJob::simple_singleline(text.into(), FontId::proportional(size), color);
    job.wrap.max_width = rect.width();
    job.wrap.max_rows = 1;
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    ui.painter().with_clip_rect(rect).galley(
        egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filtered_catalog_selection_survives_shrinking_and_held_enter_cannot_run_again() {
        let ctx = egui::Context::default();
        let mut state = CommandPaletteState {
            generation: 0,
            open: true,
            query: "backend".into(),
            index: 99,
        };
        let mut focused = false;
        let commands = vec![
            PaletteCommand::new("project:1", "Projects", "Frontend"),
            PaletteCommand::new("session:2", "Sessions", "Backend terminal"),
        ];
        let frame = |state: &mut CommandPaletteState,
                     focused: &mut bool,
                     events,
                     commands: &[PaletteCommand]| {
            let mut action = None;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(800.0, 600.0),
                    )),
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ctx| action = show(ctx, state, focused, commands),
            );
            action
        };
        let enter = |pressed| {
            vec![egui::Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: Modifiers::NONE,
            }]
        };
        frame(&mut state, &mut focused, vec![], &commands);
        assert_eq!(
            frame(&mut state, &mut focused, enter(true), &commands).as_deref(),
            Some("session:2")
        );
        assert!(!state.open);
        state.open = true;
        assert!(frame(&mut state, &mut focused, enter(true), &commands).is_none());
        assert!(state.open);
        frame(&mut state, &mut focused, enter(false), &commands);
        state.index = 99;
        assert!(frame(&mut state, &mut focused, enter(true), &[]).is_none());
        assert!(state.open);
        assert_eq!(state.index, 0);
    }

    #[test]
    fn unmatched_query_renders_localized_nothing_matches_empty_state() {
        fn has_text(shapes: &[egui::epaint::ClippedShape], exact: &str) -> bool {
            fn walk(shape: &egui::epaint::Shape, exact: &str) -> bool {
                match shape {
                    egui::epaint::Shape::Text(t) => t.galley.job.text == exact,
                    egui::epaint::Shape::Vec(list) => list.iter().any(|s| walk(s, exact)),
                    _ => false,
                }
            }
            shapes.iter().any(|cs| walk(&cs.shape, exact))
        }

        let ctx = egui::Context::default();
        let mut state = CommandPaletteState {
            generation: 0,
            open: true,
            query: "  xyz-unmatched  ".into(),
            index: 0,
        };
        let mut focused = false;
        let commands = vec![PaletteCommand::new(
            "tessel:help",
            "Tessel",
            "Keyboard shortcuts",
        )];
        crate::i18n::set_language(crate::i18n::Language::English);
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(800.0, 600.0),
                )),
                focused: true,
                ..Default::default()
            },
            |ctx| {
                let _ = show(ctx, &mut state, &mut focused, &commands);
            },
        );
        let out_en = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(800.0, 600.0),
                )),
                focused: true,
                ..Default::default()
            },
            |ctx| {
                let _ = show(ctx, &mut state, &mut focused, &commands);
            },
        );
        assert!(has_text(
            &out_en.shapes,
            "Nothing matches \"xyz-unmatched\""
        ));

        crate::i18n::set_language(crate::i18n::Language::French);
        let out_fr = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(800.0, 600.0),
                )),
                focused: true,
                ..Default::default()
            },
            |ctx| {
                let _ = show(ctx, &mut state, &mut focused, &commands);
            },
        );
        assert!(has_text(
            &out_fr.shapes,
            "Aucun résultat pour « xyz-unmatched »"
        ));
        crate::i18n::set_language(crate::i18n::Language::English);
    }
}
