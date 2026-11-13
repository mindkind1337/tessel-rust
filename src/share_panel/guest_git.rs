use super::*;
use crate::guest_git::{Action, Snapshot};
impl SharePanel {
    pub fn guest_git_snapshot(&self) -> Snapshot {
        self.guest_git.snapshot()
    }
    pub fn edit_guest_commit(
        &mut self,
        scope: crate::guest_task_board::Scope,
        text: String,
    ) -> bool {
        self.poll_guest_scope();
        self.guest_git.edit(scope, text)
    }
    pub fn request_guest_git(
        &mut self,
        scope: crate::guest_task_board::Scope,
        action: Action,
    ) -> bool {
        self.poll_guest_scope();
        let Some(session) = &self.guest_session else {
            return false;
        };
        self.guest_git
            .start(scope, action, Instant::now(), |payload| {
                session.request(payload)
            })
    }
    pub(super) fn render_guest_git(&mut self, ui: &mut Ui, french: bool) {
        let view = self.guest_git_snapshot();
        if ui
            .add_enabled(
                view.scope.is_some() && !view.pending,
                egui::Button::new(if french {
                    "Actualiser git status"
                } else {
                    "Refresh git status"
                }),
            )
            .clicked()
            && let Some(scope) = view.scope
        {
            self.request_guest_git(scope, Action::Status);
        }
        if view.pending {
            ui.spinner();
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        if view.editable() {
            let mut draft = view.draft.clone();
            if ui
                .add(
                    egui::TextEdit::singleline(&mut draft)
                        .desired_width(360.)
                        .hint_text(if french {
                            "Message de commit…"
                        } else {
                            "Commit message…"
                        }),
                )
                .changed()
                && let Some(scope) = view.scope
            {
                self.edit_guest_commit(scope, draft);
            }
        } else if !view.draft.is_empty() {
            ui.label(if french {
                "Message conservé (lecture seule)"
            } else {
                "Retained message (read-only)"
            });
            ui.add(egui::Label::new(&view.draft).selectable(true));
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    view.editable() && !view.pending,
                    egui::Button::new("Commit"),
                )
                .clicked()
                && let Some(scope) = view.scope
            {
                self.request_guest_git(scope, Action::Commit);
            }
            if view.draft_scope.is_some() {
                if ui
                    .button(if french {
                        "Copier le message"
                    } else {
                        "Copy message"
                    })
                    .clicked()
                {
                    ui.ctx().copy_text(self.guest_git_snapshot().draft);
                }
                if ui
                    .add_enabled(
                        !view.pending,
                        egui::Button::new(if french {
                            "Abandonner le message"
                        } else {
                            "Discard message"
                        }),
                    )
                    .clicked()
                {
                    self.confirm_guest_git_discard = true;
                }
            }
        });
        if self.guest_git_snapshot().draft_scope.is_none() {
            self.confirm_guest_git_discard = false;
        }
        if self.confirm_guest_git_discard {
            ui.label(if french {
                "Abandonner ce message de commit ?"
            } else {
                "Discard this commit message?"
            });
            ui.horizontal(|ui| {
                if ui
                    .button(if french { "Conserver" } else { "Keep" })
                    .clicked()
                {
                    self.confirm_guest_git_discard = false;
                }
                if ui
                    .add_enabled(
                        !self.guest_git.busy(),
                        egui::Button::new(if french {
                            "Confirmer l’abandon"
                        } else {
                            "Confirm discard"
                        }),
                    )
                    .clicked()
                {
                    self.guest_git.discard();
                    self.confirm_guest_git_discard = false;
                }
            });
        }
        let latest = self.guest_git_snapshot();
        if latest.uncertain && latest.notice != Some(crate::guest_git::Notice::Unknown) {
            ui.colored_label(
                Color32::LIGHT_RED,
                crate::guest_git::Notice::Unknown.text(french),
            );
        }
        if let Some(notice) = latest.notice {
            ui.colored_label(Color32::LIGHT_RED, notice.text(french));
        }
        if let Some(status) = &view.status {
            if view.hidden > 0 {
                ui.label(if french {
                    format!("Fichiers masqués : {}", view.hidden)
                } else {
                    format!("Hidden files: {}", view.hidden)
                });
            }
            if view.limited {
                ui.label(if french {
                    "Résultat Git partiel."
                } else {
                    "Partial Git result."
                });
            }
            if status.is_empty() && !view.limited && view.hidden == 0 {
                ui.label(if french {
                    "Aucune modification Git."
                } else {
                    "No Git changes."
                });
            }
            let mut end = status.len().min(64 * 1024);
            while !status.is_char_boundary(end) {
                end -= 1;
            }
            if end < status.len() {
                ui.label(if french {
                    "Aperçu Git partiel."
                } else {
                    "Partial Git preview."
                });
            }
            ScrollArea::vertical()
                .id_salt("guest-git-status")
                .max_height(180.)
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(&status[..end]).monospace())
                            .selectable(true),
                    );
                });
        }
    }
}
