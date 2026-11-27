//! Keyboard submission in the compatibility chat composer.
use eframe::egui::{Event, ImeEvent, Key, KeyboardShortcut, Modifiers, TextEdit, Ui};

pub(crate) fn edit(ui: &mut Ui, instance: u64, draft: &mut String) -> bool {
    let id = ui.make_persistent_id(("native-chat-draft", instance));
    let ime_id = id.with("composing");
    let mut submit = false;
    if ui.is_enabled() && ui.memory(|m| m.has_focus(id)) {
        let mut composing = ui.data(|d| d.get_temp::<bool>(ime_id).unwrap_or(false));
        ui.input_mut(|input| {
            // Some backends deliver the confirmation Enter in the commit frame.
            let ime_frame = input.events.iter().any(|e| matches!(e, Event::Ime(_)));
            input.events.retain(|event| {
                match event {
                    Event::Ime(ImeEvent::Enabled | ImeEvent::Preedit(_)) => composing = true,
                    Event::Ime(ImeEvent::Disabled | ImeEvent::Commit(_)) => composing = false,
                    Event::Key {
                        key: Key::Enter,
                        pressed: true,
                        repeat,
                        modifiers,
                        ..
                    } if modifiers.is_none() && !composing && !ime_frame => {
                        submit |= !repeat;
                        return false;
                    }
                    _ => {}
                }
                true
            });
        });
        ui.data_mut(|d| d.insert_temp(ime_id, composing));
    } else {
        ui.data_mut(|d| d.remove::<bool>(ime_id));
    }
    ui.add(
        TextEdit::multiline(draft)
            .id(id)
            .desired_rows(3)
            .return_key(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter))
            .desired_width(f32::INFINITY),
    );
    submit && !draft.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{CentralPanel, Context, RawInput};

    fn key(modifiers: Modifiers, pressed: bool) -> Event {
        Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn only_a_focused_plain_enter_submits_the_current_draft() {
        let cx = Context::default();
        let mut draft = String::from("hello");
        let mut frame = |focus: bool, mut events: Vec<Event>| {
            events.insert(0, key(Modifiers::NONE, false));
            let mut sent = false;
            let _ = cx.run(
                RawInput {
                    events,
                    ..Default::default()
                },
                |cx| {
                    CentralPanel::default().show(cx, |ui| {
                        let id = ui.make_persistent_id(("native-chat-draft", 7_u64));
                        if focus {
                            ui.memory_mut(|m| m.request_focus(id));
                        } else {
                            ui.memory_mut(|m| m.surrender_focus(id));
                        }
                        sent |= edit(ui, 7, &mut draft);
                    });
                },
            );
            (sent, draft.clone())
        };
        assert_eq!(
            frame(false, vec![key(Modifiers::NONE, true)]),
            (false, "hello".into())
        );
        assert_eq!(
            frame(true, vec![key(Modifiers::SHIFT, true)]),
            (false, "hello\n".into())
        );
        for modifiers in [Modifiers::CTRL, Modifiers::ALT, Modifiers::COMMAND] {
            assert_eq!(
                frame(true, vec![key(modifiers, true)]),
                (false, "hello\n".into())
            );
        }
        assert!(!frame(true, vec![Event::Ime(ImeEvent::Enabled)]).0);
        assert!(!frame(true, vec![key(Modifiers::NONE, true)]).0);
        assert!(
            !frame(
                true,
                vec![
                    Event::Ime(ImeEvent::Commit("é".into())),
                    key(Modifiers::NONE, true)
                ]
            )
            .0
        );

        assert!(frame(true, vec![key(Modifiers::NONE, true)]).0);
    }
}
