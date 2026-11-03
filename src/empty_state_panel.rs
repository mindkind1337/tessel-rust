//! Empty-project launcher. Receives available actions; never starts processes itself.
use eframe::egui::{self, Key, Modifiers};

pub struct Entry {
    pub id: String,
    pub label: String,
    pub group: String,
    pub enabled: bool,
}

#[derive(Default)]
pub struct EmptyState {
    selected: usize,
}

impl EmptyState {
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        title: &str,
        subtitle: &str,
        entries: &[Entry],
    ) -> Option<String> {
        let room = ui.available_rect_before_wrap();
        let root = ui.interact(
            room,
            ui.id().with("empty-project-launcher"),
            egui::Sense::click(),
        );
        if ui.is_enabled() && (root.clicked() || ui.memory(|m| m.focused().is_none())) {
            root.request_focus();
        }
        let available: Vec<_> = entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| e.enabled.then_some(i))
            .collect();
        if !available.contains(&self.selected) {
            self.selected = available.first().copied().unwrap_or(0);
        }
        let mut action = None;
        let mut scroll_selected = false;
        if ui.is_enabled() && root.has_focus() && !egui::Popup::is_any_open(ui.ctx()) {
            let (step, activate, escape, number) = ui.input_mut(|input| {
                let up = input.consume_key(Modifiers::NONE, Key::ArrowUp);
                let down = input.consume_key(Modifiers::NONE, Key::ArrowDown);
                let fresh = |key| {
                    input.events.iter().any(|event| {
                        matches!(event,
                    egui::Event::Key { key: actual, pressed: true, repeat: false, modifiers, .. }
                        if *actual == key && *modifiers == Modifiers::NONE)
                    })
                };
                let activate = fresh(Key::Enter);
                let number = [
                    Key::Num1,
                    Key::Num2,
                    Key::Num3,
                    Key::Num4,
                    Key::Num5,
                    Key::Num6,
                    Key::Num7,
                    Key::Num8,
                    Key::Num9,
                ]
                .iter()
                .position(|key| fresh(*key));
                input.consume_key(Modifiers::NONE, Key::Enter);
                let escape = input.consume_key(Modifiers::NONE, Key::Escape);
                (
                    if down {
                        1
                    } else if up {
                        -1
                    } else {
                        0
                    },
                    activate,
                    escape,
                    number,
                )
            });
            if step != 0 && !available.is_empty() {
                scroll_selected = true;
                ui.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
                let pos = available
                    .iter()
                    .position(|&i| i == self.selected)
                    .unwrap_or(0);
                self.selected =
                    available[(pos as isize + step).rem_euclid(available.len() as isize) as usize];
            }
            if escape {
                root.surrender_focus();
            } else if let Some(index) = number.filter(|&i| i < entries.len() && entries[i].enabled)
            {
                action = Some(entries[index].id.clone());
            } else if activate && let Some(entry) = entries.get(self.selected).filter(|e| e.enabled)
            {
                action = Some(entry.id.clone());
            }
        }
        let width = (room.width() - 48.0).clamp(0.0, 520.0);
        egui::ScrollArea::vertical()
            .id_salt("empty-project-scroll")
            .show(ui, |ui| {
                ui.add_space(48.0);
                ui.horizontal_top(|ui| {
                    ui.add_space(
                        ((room.width() - width) / 2.0 - ui.spacing().item_spacing.x).max(0.0),
                    );
                    ui.allocate_ui_with_layout(
                        egui::vec2(width, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(width);
                            ui.add(
                                egui::Label::new(egui::RichText::new(title).size(15.0).strong())
                                    .truncate(),
                            )
                            .on_hover_text(title);
                            if !subtitle.is_empty() {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(subtitle).size(12.0).weak(),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(subtitle);
                            }
                            ui.add_space(14.0);
                            let mut last_group = "";
                            for (index, entry) in entries.iter().enumerate() {
                                if entry.group != last_group {
                                    ui.add_space(10.0);
                                    ui.weak(&entry.group);
                                    last_group = &entry.group;
                                }
                                let row = ui.add_enabled(
                                    entry.enabled,
                                    egui::Button::selectable(
                                        root.has_focus() && self.selected == index,
                                        "",
                                    )
                                    .min_size(egui::vec2(width, 32.0))
                                    .truncate(),
                                );
                                row.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::WidgetType::Button,
                                        ui.is_enabled() && entry.enabled,
                                        &entry.label,
                                    )
                                });
                                let number = egui::Rect::from_center_size(
                                    egui::pos2(row.rect.left() + 17.0, row.rect.center().y),
                                    egui::Vec2::splat(18.0),
                                );
                                ui.painter().rect_stroke(
                                    number,
                                    4.0,
                                    ui.visuals().widgets.noninteractive.bg_stroke,
                                    egui::StrokeKind::Inside,
                                );
                                let color = if entry.enabled {
                                    ui.visuals().text_color()
                                } else {
                                    ui.visuals().weak_text_color()
                                };
                                ui.painter().text(
                                    number.center(),
                                    egui::Align2::CENTER_CENTER,
                                    (index + 1).to_string(),
                                    egui::FontId::proportional(11.0),
                                    color,
                                );
                                let mut label = egui::text::LayoutJob::simple_singleline(
                                    entry.label.clone(),
                                    egui::FontId::proportional(13.0),
                                    color,
                                );
                                label.wrap.max_width = (row.rect.width() - 44.0).max(1.0);
                                label.wrap.max_rows = 1;
                                let galley = ui.fonts_mut(|f| f.layout_job(label));
                                ui.painter().galley(
                                    egui::pos2(
                                        row.rect.left() + 36.0,
                                        row.rect.center().y - galley.size().y / 2.0,
                                    ),
                                    galley,
                                    color,
                                );
                                if row.clicked() {
                                    action = Some(entry.id.clone());
                                }
                                if scroll_selected && index == self.selected {
                                    row.scroll_to_me(Some(egui::Align::Center));
                                }
                                if row.hovered()
                                    && entry.enabled
                                    && ui.input(|input| input.pointer.delta() != egui::Vec2::ZERO)
                                {
                                    self.selected = index;
                                }
                                row.on_hover_text(&entry.label);
                            }
                            ui.add_space(14.0);
                            ui.weak(crate::i18n::tr(
                                "1–9 or arrows and Enter to start · Esc to stay empty",
                            ));
                        },
                    );
                });
            });
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stationary_mouse_does_not_override_arrow_selection() {
        let ctx = egui::Context::default();
        let mut state = EmptyState::default();
        let entries: Vec<_> = ["shell", "folder"]
            .into_iter()
            .map(|id| Entry {
                id: id.into(),
                label: id.into(),
                group: "Actions".into(),
                enabled: true,
            })
            .collect();
        let mut frame = |events| {
            let mut action = None;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 600.0),
                    )),
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        action = state.show(ui, "Project", "", &entries);
                    });
                },
            );
            let shell = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text == "shell" => {
                        Some(text.pos + text.galley.size() / 2.0)
                    }
                    _ => None,
                })
                .expect("shell label is painted");
            (action, shell)
        };
        frame(vec![]);
        let (_, shell) = frame(vec![]);
        frame(vec![egui::Event::PointerMoved(shell)]);
        let key = |key, pressed| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        frame(vec![key(Key::ArrowDown, true)]);
        frame(vec![key(Key::ArrowDown, false)]);
        assert_eq!(
            frame(vec![key(Key::Enter, true)]).0.as_deref(),
            Some("folder")
        );
    }

    #[test]
    fn keyboard_skips_disabled_actions_never_repeats_launch_and_respects_other_focus() {
        let ctx = egui::Context::default();
        let mut state = EmptyState::default();
        let entries: Vec<_> = [("shell", true), ("missing", false), ("folder", true)]
            .into_iter()
            .map(|(id, enabled)| Entry {
                id: id.into(),
                label: id.into(),
                group: "Actions".into(),
                enabled,
            })
            .collect();
        let outside = egui::Id::new("outside-input");
        let mut frame = |events, enabled, external| {
            let mut action = None;
            let mut enter_left = false;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 600.0),
                    )),
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        if external {
                            ui.memory_mut(|m| m.request_focus(outside));
                        }
                        ui.interact(
                            egui::Rect::from_min_size(ui.cursor().min, egui::vec2(10.0, 10.0)),
                            outside,
                            egui::Sense::focusable_noninteractive(),
                        );
                        ui.add_enabled_ui(enabled, |ui| {
                            action = state.show(ui, "Synthetic project", "", &entries)
                        });
                        enter_left = ui.input(|i| i.key_pressed(Key::Enter));
                    });
                },
            );
            (action, enter_left)
        };
        let key = |key, pressed| {
            vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: Modifiers::NONE,
            }]
        };
        frame(vec![], true, false);
        frame(vec![], true, false);
        frame(key(Key::ArrowDown, true), true, false);
        frame(key(Key::ArrowDown, false), true, false);
        assert_eq!(
            frame(key(Key::Enter, true), true, false).0.as_deref(),
            Some("folder")
        );
        assert!(frame(key(Key::Enter, true), true, false).0.is_none());
        frame(key(Key::Enter, false), true, false);
        assert_eq!(frame(key(Key::Enter, true), false, false), (None, true));
        frame(key(Key::Enter, false), false, false);
        assert_eq!(frame(key(Key::Enter, true), true, true), (None, true));
    }
}
