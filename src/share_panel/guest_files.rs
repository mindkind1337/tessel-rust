use super::*;
use crate::guest_file_editor::Key as FileKey;
impl SharePanel {
    pub fn guest_file_document(&self) -> Option<crate::guest_file_editor::Document> {
        self.guest_file_editor.document.clone()
    }
    pub fn guest_file_notice(&self) -> Option<crate::guest_file_editor::Notice> {
        self.guest_file_editor.notice.clone()
    }
    pub fn open_guest_file(&mut self, key: FileKey) -> bool {
        self.poll_guest_scope();
        let Some(session) = &self.guest_session else {
            return false;
        };
        self.confirm_guest_file_discard = false;
        self.guest_file_editor
            .open(key, Instant::now(), |payload| session.request(payload))
    }
    pub fn edit_guest_file(&mut self, key: &FileKey, text: String) -> bool {
        self.poll_guest_scope();
        self.guest_file_editor.edit(key, text)
    }
    pub fn save_guest_file(&mut self, key: &FileKey) -> bool {
        self.poll_guest_scope();
        let Some(session) = &self.guest_session else {
            return false;
        };
        self.guest_file_editor
            .save(key, Instant::now(), |payload| session.request(payload))
    }
    fn list_guest_files(&mut self, path: String) -> bool {
        self.poll_guest_scope();
        let Some(scope) = self.guest_files_scope else {
            return false;
        };
        let Some(session) = &self.guest_session else {
            return false;
        };
        self.guest_file_listing
            .start(scope, path, Instant::now(), |payload| {
                session.request(payload)
            })
    }
    pub(super) fn render_guest_files(&mut self, ui: &mut Ui, french: bool) {
        self.poll_guest_scope();
        if self.guest_file_listing.take_initial() {
            self.list_guest_files(String::new());
        }
        let mut list = None;
        ui.horizontal(|ui| {
            if !self.guest_file_listing.path.is_empty()
                && ui
                    .add_enabled(
                        !self.guest_file_listing.busy(),
                        egui::Button::new(if french {
                            ".. Dossier parent"
                        } else {
                            ".. Parent"
                        }),
                    )
                    .clicked()
            {
                list = Some(
                    self.guest_file_listing
                        .path
                        .rsplit_once('/')
                        .map_or("", |(parent, _)| parent)
                        .to_owned(),
                );
            }
            if ui
                .add_enabled(
                    !self.guest_file_listing.busy(),
                    egui::Button::new(if french { "Actualiser" } else { "Refresh" }),
                )
                .clicked()
            {
                list = Some(self.guest_file_listing.path.clone());
            }
            if self.guest_file_listing.busy() {
                ui.spinner();
                ui.ctx().request_repaint_after(Duration::from_millis(50));
            }
        });
        if let Some(notice) = &self.guest_file_listing.notice {
            ui.colored_label(Color32::LIGHT_RED, notice.text(french));
        }
        let mut open = None;
        for entry in &self.guest_file_listing.entries {
            if ui
                .add_enabled(
                    !self.guest_file_listing.busy() && !self.guest_file_editor.busy(),
                    egui::Button::new(format!(
                        "{} {}",
                        if entry.directory { "▸" } else { "·" },
                        entry.name
                    )),
                )
                .clicked()
            {
                if entry.directory {
                    list = Some(entry.path.clone());
                } else {
                    open = Some(entry.path.clone());
                }
            }
        }
        if let Some(path) = list {
            self.list_guest_files(path);
        }
        if let Some(path) = open
            && let Some(scope) = self.guest_files_scope
        {
            self.open_guest_file(FileKey { scope, path });
        }
        self.render_guest_file_editor(ui, french);
    }
    pub(super) fn render_guest_file_editor(&mut self, ui: &mut Ui, french: bool) {
        if self.guest_file_editor.busy() {
            ui.spinner();
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        if let Some(document) = self.guest_file_document() {
            ui.separator();
            ui.strong(&document.key.path);
            let busy = self.guest_file_editor.busy();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !busy
                            && self.guest_file_editor.editable()
                            && self.guest_file_editor.has_draft(),
                        egui::Button::new(if french { "Enregistrer" } else { "Save" }),
                    )
                    .clicked()
                {
                    self.save_guest_file(&document.key);
                }
                if ui
                    .button(if french {
                        "Copier le texte"
                    } else {
                        "Copy text"
                    })
                    .clicked()
                {
                    // Use the latest text, including an edit in this frame.
                    if let Some(current) = &self.guest_file_editor.document {
                        ui.ctx().copy_text(current.text.as_ref().clone());
                    }
                }
                if ui
                    .add_enabled(
                        !busy,
                        egui::Button::new(if french {
                            "Fermer le fichier"
                        } else {
                            "Close file"
                        }),
                    )
                    .clicked()
                {
                    if self.guest_file_editor.has_draft() {
                        self.confirm_guest_file_discard = true;
                    } else {
                        self.guest_file_editor.discard();
                    }
                }
            });
            if self.guest_files_scope == Some(document.key.scope)
                && self
                    .guest_file_editor
                    .document
                    .as_ref()
                    .is_some_and(|doc| doc.key == document.key)
                && let Some(session) = &self.guest_session
            {
                self.guest_history.bind(Some(&document.key));
                self.guest_history.show(ui, french, session);
            }
            ScrollArea::vertical()
                .id_salt("guest-file-content")
                .max_height(180.)
                .show(ui, |ui| {
                    if self.guest_file_editor.editable() {
                        let mut text = document.text.as_ref().clone();
                        if ui
                            .add(
                                egui::TextEdit::multiline(&mut text)
                                    .code_editor()
                                    .desired_rows(6)
                                    .desired_width(f32::INFINITY),
                            )
                            .changed()
                        {
                            self.edit_guest_file(&document.key, text);
                        }
                    } else {
                        let (preview, partial) = document.preview();
                        if partial {
                            ui.label(if french { "Aperçu partiel (64 Kio). Copier le texte conserve le fichier complet." } else { "Partial preview (64 KiB). Copy text keeps the complete file." });
                        }
                        ui.add(egui::Label::new(RichText::new(preview).monospace()).selectable(true));
                    }
                });
            if !self.guest_file_editor.has_draft() {
                self.confirm_guest_file_discard = false;
            }
            if self.confirm_guest_file_discard {
                ui.label(if french {"Abandonner ce brouillon non enregistré ? Copiez-le d’abord si vous voulez le conserver."}else{"Discard this unsaved draft? Copy it first if you want to keep it."});
                ui.horizontal(|ui| {
                    if ui
                        .button(if french {
                            "Conserver le brouillon"
                        } else {
                            "Keep draft"
                        })
                        .clicked()
                    {
                        self.confirm_guest_file_discard = false;
                    }
                    if ui
                        .add_enabled(
                            !busy,
                            egui::Button::new(if french {
                                "Abandonner le brouillon"
                            } else {
                                "Discard draft"
                            }),
                        )
                        .clicked()
                    {
                        self.guest_file_editor.discard();
                        self.confirm_guest_file_discard = false;
                    }
                });
            }
        }
        if let Some(notice) = self.guest_file_notice() {
            ui.colored_label(Color32::LIGHT_RED, notice.text(french));
        }
    }
}
