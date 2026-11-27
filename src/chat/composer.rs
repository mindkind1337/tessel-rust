//! Keyboard submission in the compatibility chat composer.
use eframe::egui::{Event, ImeEvent, Key, KeyboardShortcut, Modifiers, TextEdit, Ui};

#[derive(Default)]
pub(crate) struct ComposerKeys {
    pub submit: bool,
    pub interrupt: bool,
}

pub(crate) fn edit(
    ui: &mut Ui,
    instance: u64,
    draft: &mut String,
    can_interrupt: bool,
) -> ComposerKeys {
    let id = ui.make_persistent_id(("native-chat-draft", instance));
    let ime_id = id.with("composing");
    let mut keys = ComposerKeys::default();
    let can_interrupt = can_interrupt && !eframe::egui::Popup::is_any_open(ui.ctx());
    // egui normally drops text focus at the beginning of the Escape frame,
    // before this widget receives its events. Reclaim only this composer's key.
    if can_interrupt
        && ui.is_enabled()
        && ui.input(|i| i.key_pressed(Key::Escape))
        && ui.memory(|m| m.focused().is_none() && m.had_focus_last_frame(id))
    {
        ui.memory_mut(|m| m.request_focus(id));
    }
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
                        keys.submit |= !repeat;
                        return false;
                    }
                    Event::Key {
                        key: Key::Escape,
                        pressed: true,
                        repeat,
                        ..
                    } if can_interrupt && !composing && !ime_frame => {
                        keys.interrupt |= !repeat;
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
    keys.submit &= !draft.trim().is_empty();
    keys
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
    fn escape_preserves_the_draft_and_only_interrupts_when_available() {
        let cx = Context::default();
        let mut draft = String::from("next question");
        let escape = |pressed| Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        let mut frame = |focus, working, events| {
            let mut interrupted = false;
            let _ = cx.run(
                RawInput {
                    events,
                    ..Default::default()
                },
                |cx| {
                    CentralPanel::default().show(cx, |ui| {
                        let id = ui.make_persistent_id(("native-chat-draft", 42_u64));
                        if focus == Some(true) {
                            ui.memory_mut(|m| m.request_focus(id));
                        } else if focus == Some(false) {
                            ui.memory_mut(|m| m.surrender_focus(id));
                        }
                        let keys = edit(ui, 42, &mut draft, working);
                        assert!(!keys.submit);
                        interrupted |= keys.interrupt;
                    });
                },
            );
            interrupted
        };
        assert!(!frame(Some(false), true, vec![escape(true), escape(false)]));
        assert!(!frame(Some(true), false, vec![escape(true), escape(false)]));
        assert!(!frame(
            Some(true),
            true,
            vec![Event::Ime(ImeEvent::Enabled), escape(true), escape(false)]
        ));
        assert!(!frame(
            Some(true),
            true,
            vec![Event::Ime(ImeEvent::Disabled)]
        ));
        assert!(!frame(Some(true), true, vec![]));
        assert!(frame(None, true, vec![escape(true)]));
        assert!(!frame(Some(true), true, vec![escape(true)]));
        assert_eq!(draft, "next question");
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
                        sent |= edit(ui, 7, &mut draft, false).submit;
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
