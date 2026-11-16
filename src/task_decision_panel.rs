//! Compatibility renderer for the same explicit decision-answer action as GPUI.
use crate::{tasks_board_panel::TasksBoardAction, workspace::Task};
use eframe::egui;
pub fn show(
    ui: &mut egui::Ui,
    tasks: &[Task],
    project: u64,
    french: bool,
) -> Option<TasksBoardAction> {
    let mut action = None;
    let cards: Vec<_> = tasks
        .iter()
        .filter(|task| task.project == project && task.decision.is_some())
        .collect();
    if cards.is_empty() {
        return None;
    }
    egui::CollapsingHeader::new(if french { "Décisions" } else { "Decisions" })
        .id_salt(("task-decisions", project))
        .default_open(
            cards
                .iter()
                .any(|task| task.decision.as_ref().unwrap().pending()),
        )
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .max_height(200.)
                .show(ui, |ui| {
                    for task in cards {
                        let decision = task.decision.as_ref().unwrap();
                        ui.push_id((project, task.id, decision.request), |ui| {
                            ui.group(|ui| {
                                ui.label(&task.title);
                                ui.label(&decision.question);
                                if let Some(answer) = &decision.answer {
                                    ui.label(&answer.text);
                                    return;
                                }
                                let answer_action =
                                    |text: String| TasksBoardAction::AnswerDecision {
                                        id: task.id,
                                        request: decision.request,
                                        answer: text,
                                    };
                                ui.horizontal_wrapped(|ui| {
                                    for option in &decision.options {
                                        if ui.button(option).clicked() {
                                            action = Some(answer_action(option.clone()));
                                        }
                                    }
                                });
                                let draft_id = ui.make_persistent_id("answer-draft");
                                let mut draft = ui.data_mut(|data| {
                                    data.get_temp::<String>(draft_id).unwrap_or_default()
                                });
                                ui.add(egui::TextEdit::singleline(&mut draft).hint_text(
                                    if french {
                                        "Votre réponse"
                                    } else {
                                        "Your answer"
                                    },
                                ));
                                let valid = !crate::agent_names::js_trim(&draft).is_empty()
                                    && draft.encode_utf16().count() <= 500;
                                if ui
                                    .add_enabled(
                                        valid,
                                        egui::Button::new(if french {
                                            "Répondre"
                                        } else {
                                            "Answer"
                                        }),
                                    )
                                    .clicked()
                                {
                                    action = Some(answer_action(draft.clone()));
                                }
                                ui.data_mut(|data| data.insert_temp(draft_id, draft));
                            })
                        });
                    }
                });
        });
    action
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compatibility_choice_emits_the_exact_question_identity_and_respects_disabled_ui() {
        let task:Task=serde_json::from_value(serde_json::json!({"id":2,"project":1,"title":"Synthetic","column":0,
            "decision":{"request":3,"question":"Continue?","options":["Continue"],"by":null,"asked_at_ms":1,"answer":null}})).unwrap();
        for enabled in [true, false] {
            let ctx = egui::Context::default();
            let tasks = vec![task.clone()];
            let frame = |events| {
                let mut action = None;
                let out = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(600., 500.),
                        )),
                        events,
                        focused: true,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            ui.add_enabled_ui(enabled, |ui| action = show(ui, &tasks, 1, false))
                        });
                    },
                );
                (out, action)
            };
            let _ = frame(vec![]);
            let (out, _) = frame(vec![]);
            fn text_pos(shape: &egui::Shape) -> Option<egui::Pos2> {
                match shape {
                    egui::Shape::Text(t) if t.galley.job.text == "Continue" => {
                        Some(t.pos + t.galley.size() * 0.5)
                    }
                    egui::Shape::Vec(shapes) => shapes.iter().find_map(text_pos),
                    _ => None,
                }
            }
            let pos = out
                .shapes
                .iter()
                .find_map(|shape| text_pos(&shape.shape))
                .expect("choice is rendered");
            let (_, action) = frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ]);
            if enabled {
                assert_eq!(
                    action,
                    Some(TasksBoardAction::AnswerDecision {
                        id: 2,
                        request: 3,
                        answer: "Continue".into()
                    })
                );
            } else {
                assert!(action.is_none());
            }
        }
    }
}
