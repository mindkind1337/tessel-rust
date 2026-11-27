use super::*;
use crate::guest_file_editor::Key as FileKey;
#[derive(Clone, Debug)]
pub(crate) struct GuestFilesView {
    pub image: Option<(FileKey, std::sync::Arc<gpui::RenderImage>)>,
    pub image_menu: bool,
    pub download_menu: Option<FileKey>,
    pub directory: String,
    pub tabs: Vec<FileKey>,
    pub confirm_close: bool,
    pub can_save_then_close: bool,
    pub entries: Vec<crate::guest_file_listing::Entry>,
    pub document: Option<crate::guest_file_editor::Document>,
    pub notices: Vec<String>,
    pub busy: bool,
    pub history: Option<std::sync::Arc<super::guest_history::Page>>,
    pub versions: Option<std::sync::Arc<Vec<super::guest_versions::Version>>>,
    pub version_diff: Option<std::sync::Arc<crate::saved_version_diff::Patch>>,
    pub version_text: Option<std::sync::Arc<String>>,
    pub more: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct RetainedFiles {
    pub document: crate::guest_file_editor::Document,
    pub other: Vec<FileKey>,
    pub confirm_discard: bool,
    pub notice: Option<crate::guest_file_editor::Notice>,
}

pub(crate) fn retained_label(key: &FileKey, french: bool) -> String {
    let project = if french { "projet" } else { "project" };
    format!(
        "{} · {project} {} · session {}",
        key.path, key.scope.project, key.scope.session
    )
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum RecoveryAction {
    Select,
    Copy,
    AskDiscard,
    Keep,
    Discard,
}

impl SharePanel {
    pub(super) fn retained_files(&self) -> Option<RetainedFiles> {
        if self.tab != DialogTab::Join
            || self.guest_files_scope.is_some()
            || !self.guest_file_editor.has_draft()
        {
            return None;
        }
        Some(RetainedFiles {
            document: self.guest_file_document()?,
            other: self.guest_file_editor.other_drafts().cloned().collect(),
            confirm_discard: self.confirm_guest_file_discard,
            notice: self.guest_file_notice(),
        })
    }

    pub(crate) fn recover_file(&mut self, key: &FileKey, action: RecoveryAction) -> Option<String> {
        self.poll_guest_scope();
        if self.tab != DialogTab::Join
            || self.guest_files_scope.is_some()
            || self
                .guest_session
                .as_ref()
                .is_some_and(|s| !matches!(s.snapshot().phase, GuestPhase::Ended { .. }))
            || !self.guest_file_editor.has_draft()
        {
            return None;
        }
        if matches!(action, RecoveryAction::Select) {
            if self.guest_file_editor.select_retained(key) {
                self.confirm_guest_file_discard = false;
            }
            return None;
        }
        let doc = self
            .guest_file_editor
            .document
            .as_ref()
            .filter(|d| &d.key == key)?;
        match action {
            RecoveryAction::Copy => return Some(doc.text.as_ref().clone()),
            RecoveryAction::AskDiscard => self.confirm_guest_file_discard = true,
            RecoveryAction::Keep => self.confirm_guest_file_discard = false,
            RecoveryAction::Discard if self.confirm_guest_file_discard => {
                self.confirm_guest_file_discard = false;
                self.guest_file_editor.discard();
            }
            _ => {}
        }
        None
    }

    pub(super) fn guest_files_view(&self, french: bool) -> Option<GuestFilesView> {
        let scope = self.guest_files_scope?;
        if self.guest_sub_view != GuestSubView::Files {
            return None;
        }
        let doc = self
            .guest_file_editor
            .document
            .as_ref()
            .filter(|d| d.key.scope == scope)
            .cloned();
        let mut notices = Vec::new();
        if let Some(n) = &self.guest_file_listing.notice {
            notices.push(n.text(french));
        }
        if let Some(n) = &self.guest_file_editor.notice {
            notices.push(n.text(french));
        }
        if let Some(error) = &self.guest_history.error {
            notices.push(if error.is_empty() {
                if french {
                    "Historique indisponible."
                } else {
                    "History unavailable."
                }
                .into()
            } else {
                error.clone()
            });
        }
        if self.guest_versions.diff_failed {
            notices.push(super::guest_versions::comparison_error(french).into());
        }
        if self.guest_versions.failed {
            notices.push(
                if french {
                    "Historique local indisponible."
                } else {
                    "Local history unavailable."
                }
                .into(),
            );
        }
        notices.extend(self.guest_download.status(french));
        if self.guest_image.failed {
            notices.push(super::guest_image::ImagePreview::error(french).into());
        }
        Some(GuestFilesView {
            image: self.guest_image.key.clone().zip(self.guest_image.native()),
            image_menu: self.guest_image.menu,
            download_menu: self.guest_download.menu.clone(),
            versions: self.guest_versions.rows.clone(),
            version_diff: self.guest_versions.diff.clone(),
            version_text: self.guest_versions.text.clone(),
            directory: self.guest_file_listing.path.clone(),
            tabs: self.guest_file_editor.tabs.clone(),
            confirm_close: self.confirm_guest_file_discard,
            can_save_then_close: self.confirm_guest_file_discard
                && self.guest_file_editor.editable()
                && !self.guest_file_editor.busy(),
            entries: self.guest_file_listing.entries.clone(),
            document: doc,
            notices,
            busy: self.guest_image.busy()
                || self.guest_file_listing.busy()
                || self.guest_file_editor.busy()
                || self.guest_versions.busy()
                || self.guest_history.busy(),
            history: self.guest_history.page.clone(),
            more: self.guest_history.page.as_ref().is_some_and(|p| p.has_more)
                && self.guest_history.next_limit(false) < 200,
        })
    }
    pub(crate) fn current_guest_file_text(&mut self, key: &FileKey) -> Option<String> {
        self.poll_guest_scope();
        if self.guest_files_scope != Some(key.scope) || self.guest_sub_view != GuestSubView::Files {
            return None;
        }
        self.guest_file_editor
            .document
            .as_ref()
            .filter(|doc| &doc.key == key)
            .map(|doc| doc.text.as_ref().clone())
    }

    pub(crate) fn editable_workspace_document(&self) -> Option<crate::guest_file_editor::Document> {
        if self.guest_sub_view != GuestSubView::Files || !self.guest_file_editor.editable() {
            return None;
        }
        self.guest_file_document()
            .filter(|d| self.active_workspace_file() == Some(&d.key))
    }

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
        let loading = self.guest_file_editor.reading().or_else(|| {
            if self.guest_image.busy() {
                self.guest_image.key.as_ref()
            } else {
                None
            }
        });
        if let Some(loading) = loading {
            if self.guest_files_scope != Some(key.scope)
                || !crate::guest_file_editor::valid_path(&key.path)
            {
                return false;
            }
            // Finish the current request, then read only the most recently selected file.
            self.next_guest_file = (loading != &key).then_some(key);
            return true;
        }
        self.confirm_guest_file_discard = false;
        if self.guest_file_editor.may_switch(&key) {
            if super::guest_image::ImagePreview::supports(&key.path) {
                self.guest_file_editor.leave_document();
                self.guest_image.open(key, session);
                return true;
            }
            self.guest_image = Default::default();
        }
        self.guest_file_editor
            .open(key, Instant::now(), |payload| session.request(payload))
    }
    pub(crate) fn workspace_tab_context(&self) -> Option<(String, String)> {
        let key = self.workspace_tabs().first()?;
        let session = self.guest_session.as_ref()?;
        if session.generation() != key.scope.session {
            return None;
        }
        session.tab_context(key.scope.project)
    }

    pub(crate) fn workspace_tabs(&self) -> &[FileKey] {
        &self.guest_file_editor.tabs
    }

    pub(crate) fn select_workspace_file(&mut self, key: FileKey) -> bool {
        self.poll_guest_scope();
        if self.guest_files_scope != Some(key.scope) || !self.workspace_tabs().contains(&key) {
            return false;
        }
        self.guest_sub_view = GuestSubView::Files;
        self.select_guest_tab(key);
        true
    }

    pub(crate) fn active_workspace_file(&self) -> Option<&FileKey> {
        self.workspace_tabs()
            .iter()
            .find(|key| self.guest_tab_active(key))
    }

    pub(crate) fn guest_tab_dirty(&self, key: &FileKey) -> bool {
        self.guest_file_editor
            .document
            .as_ref()
            .is_some_and(|doc| &doc.key == key && doc.dirty())
            || self.guest_file_editor.parked_draft(key)
    }

    pub(crate) fn guest_tab_active(&self, key: &FileKey) -> bool {
        self.guest_file_editor
            .document
            .as_ref()
            .is_some_and(|doc| &doc.key == key)
            || (!self.guest_image.failed && self.guest_image.key.as_ref() == Some(key))
    }

    pub(super) fn select_guest_tab(&mut self, key: FileKey) {
        if self.guest_file_editor.tabs.contains(&key)
            && (!self.guest_tab_active(&key) || self.next_guest_file.is_some())
        {
            self.open_guest_file(key);
        }
    }

    pub(super) fn start_guest_save_then_close(&mut self, key: &FileKey) {
        // Match saveDocs in the Vue editor: at most three writes per close request.
        self.save_guest_close_revision(key, 2);
    }

    fn save_guest_close_revision(&mut self, key: &FileKey, retries: u8) {
        self.poll_guest_scope();
        if self.confirm_guest_file_discard
            && self.guest_files_scope == Some(key.scope)
            && self.guest_file_editor.editable()
            && self.active_workspace_file() == Some(key)
            && self.save_guest_file(key)
            && self.guest_file_editor.busy()
        {
            self.confirm_guest_file_discard = false;
            self.save_then_close = Some((key.clone(), retries));
        }
    }

    pub(super) fn finish_guest_save_then_close(&mut self) {
        let Some((key, retries)) = self.save_then_close.take() else {
            return;
        };
        if self.guest_files_scope != Some(key.scope)
            || self.active_workspace_file() != Some(&key)
            || self.guest_file_notice() != Some(crate::guest_file_editor::Notice::Saved)
        {
            return;
        }
        if self.guest_file_editor.has_draft() {
            // An edit arrived while the host was acknowledging the previous revision.
            self.confirm_guest_file_discard = true;
            if retries > 0 {
                self.save_guest_close_revision(&key, retries - 1);
            }
        } else {
            self.discard_guest_tab(&key);
        }
    }

    pub(super) fn confirm_guest_close(&mut self, key: &FileKey, discard: bool) {
        if self.confirm_guest_file_discard
            && self
                .guest_file_editor
                .document
                .as_ref()
                .is_some_and(|doc| &doc.key == key)
        {
            self.confirm_guest_file_discard = false;
            if discard && !self.guest_file_editor.busy() {
                self.discard_guest_tab(key);
            }
        }
    }

    pub(crate) fn close_guest_tab(&mut self, key: &FileKey) {
        self.poll_guest_scope();
        if self.guest_files_scope != Some(key.scope)
            || self.guest_file_editor.busy()
            || self.guest_image.busy()
        {
            return;
        }
        let Some(index) = self
            .guest_file_editor
            .tabs
            .iter()
            .position(|tab| tab == key)
        else {
            return;
        };
        if self.guest_file_editor.parked_draft(key) {
            if self.open_guest_file(key.clone()) && self.guest_tab_active(key) {
                self.confirm_guest_file_discard = true;
            }
            return;
        }
        if self.guest_image.key.as_ref() == Some(key) {
            self.guest_image = Default::default();
            self.guest_file_editor.tabs.remove(index);
            self.open_neighbor_tab(index);
            return;
        }
        if self
            .guest_file_editor
            .document
            .as_ref()
            .is_none_or(|doc| &doc.key != key)
        {
            self.guest_file_editor.tabs.remove(index);
            return;
        }
        if self.guest_file_editor.has_draft() {
            self.confirm_guest_file_discard = true;
            return;
        }
        self.discard_guest_tab(key);
    }

    fn discard_guest_tab(&mut self, key: &FileKey) {
        let index = self
            .guest_file_editor
            .tabs
            .iter()
            .position(|tab| tab == key)
            .unwrap_or(0);
        self.guest_file_editor.discard();
        self.open_neighbor_tab(index);
    }

    fn open_neighbor_tab(&mut self, index: usize) {
        if let Some(next) = self
            .guest_file_editor
            .tabs
            .get(index)
            .or_else(|| self.guest_file_editor.tabs.last())
            .cloned()
        {
            self.open_guest_file(next);
        }
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
    pub(super) fn list_guest_files(&mut self, path: String) -> bool {
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
        if let Some(status) = self.guest_download.status(french) {
            ui.label(status);
        }
        let mut download = None;
        let mut open = None;
        for entry in &self.guest_file_listing.entries {
            let row = ui.add_enabled(
                !self.guest_file_listing.busy()
                    && (!self.guest_file_editor.busy()
                        || self.guest_file_editor.reading().is_some()),
                egui::Button::new(format!(
                    "{} {}",
                    if entry.directory { "▸" } else { "·" },
                    entry.name
                )),
            );
            if !entry.directory
                && let Some(scope) = self.guest_files_scope
            {
                let key = FileKey {
                    scope,
                    path: entry.path.clone(),
                };
                if row.secondary_clicked() {
                    self.guest_download.menu = Some(key.clone());
                }
                if self.guest_download.menu.as_ref() == Some(&key)
                    && ui
                        .add_enabled(
                            !self.guest_download.busy(),
                            egui::Button::new(if french {
                                "Télécharger…"
                            } else {
                                "Download…"
                            }),
                        )
                        .clicked()
                {
                    download = Some(entry.path.clone());
                }
            }
            if row.clicked() {
                self.guest_download.menu = None;
                if entry.directory {
                    list = Some(entry.path.clone());
                } else {
                    open = Some(entry.path.clone());
                }
            }
        }
        if let Some(path) = download
            && let Some(scope) = self.guest_files_scope
        {
            self.choose_guest_download(&FileKey { scope, path });
        }
        if let Some(path) = list {
            self.list_guest_files(path);
        }
        if let Some(path) = open
            && let Some(scope) = self.guest_files_scope
        {
            self.open_guest_file(FileKey { scope, path });
        }
        let mut selected = None;
        let mut closed = None;
        ui.horizontal_wrapped(|ui| {
            for key in &self.guest_file_editor.tabs {
                let active = self.guest_tab_active(key);
                if ui.selectable_label(active, &key.path).clicked() {
                    selected = Some(key.clone());
                }
                if ui
                    .small_button("×")
                    .on_hover_text(if french {
                        "Fermer le fichier"
                    } else {
                        "Close file"
                    })
                    .clicked()
                {
                    closed = Some(key.clone());
                }
            }
        });
        if let Some(key) = closed {
            self.close_guest_tab(&key);
        }
        if let Some(key) = selected {
            self.select_guest_tab(key);
        }
        if let Some(key) = self.active_workspace_file()
            && ui
                .button(if french {
                    "Ouvrir dans l’espace de travail"
                } else {
                    "Open in workspace"
                })
                .clicked()
        {
            self.workspace_file_request = Some(key.clone());
        }
        self.render_guest_document(ui, french, false);
    }

    pub(crate) fn render_guest_document(&mut self, ui: &mut Ui, french: bool, workspace: bool) {
        if self.guest_image.busy() {
            ui.spinner();
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        if self.guest_image.failed {
            ui.label(super::guest_image::ImagePreview::error(french));
        }
        if let Some(key) = self.guest_image.key.clone()
            && let Some(texture) = self.guest_image.texture(ui.ctx())
        {
            guest_path_heading(ui, &key.path, french);
            let mut save = false;
            let height = if workspace {
                ui.available_height().max(180.)
            } else {
                270.
            };
            ui.add(
                egui::Image::new(texture)
                    .max_size(egui::vec2(ui.available_width(), height))
                    .sense(egui::Sense::click()),
            )
            .context_menu(|ui| {
                if ui
                    .button(if french {
                        "Télécharger l’original…"
                    } else {
                        "Download original…"
                    })
                    .clicked()
                {
                    save = true;
                    ui.close();
                }
            });
            if save {
                self.choose_guest_download(&key);
            }
        }
        if !workspace
            || self
                .guest_file_editor
                .document
                .as_ref()
                .is_none_or(|d| Some(d.key.scope) == self.guest_files_scope)
        {
            self.render_guest_file_editor(ui, french);
        }
    }
    pub(super) fn render_guest_file_editor(&mut self, ui: &mut Ui, french: bool) {
        if self.guest_file_editor.busy() {
            ui.spinner();
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        if let Some(document) = self.guest_file_document() {
            ui.separator();
            guest_path_heading(ui, &document.key.path, french);
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
                        self.discard_guest_tab(&document.key);
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
                self.guest_versions.bind(Some(&document.key));
                let versions = &mut self.guest_versions;
                if ui
                    .add_enabled(
                        !versions.busy(),
                        egui::Button::new(if french {
                            "Historique local"
                        } else {
                            "Local History"
                        }),
                    )
                    .clicked()
                {
                    versions.request(session, None);
                }
                let mut selected = None;
                if let Some(rows) = &versions.rows {
                    if rows.is_empty() {
                        ui.label(if french {
                            "Aucune sauvegarde."
                        } else {
                            "No saved versions."
                        });
                    }
                    ScrollArea::vertical()
                        .id_salt("guest-version-list")
                        .max_height(120.)
                        .show(ui, |ui| {
                            for row in rows.iter() {
                                if ui
                                    .add_enabled(!versions.busy(), egui::Button::new(row.label()))
                                    .clicked()
                                {
                                    selected = Some(row.id.clone());
                                }
                            }
                        });
                }
                if let Some(id) = selected {
                    versions.request(session, Some(&id));
                }
                if versions.failed {
                    ui.label(if french {
                        "Historique local indisponible."
                    } else {
                        "Local history unavailable."
                    });
                }
                if versions.diff_failed {
                    ui.label(super::guest_versions::comparison_error(french));
                }
                if let Some(patch) = &versions.diff {
                    ui.label(super::guest_versions::comparison_title(
                        french,
                        patch.line_count() == 0,
                    ));
                    if patch.line_count() > 0 {
                        let height = ui.text_style_height(&egui::TextStyle::Monospace);
                        ScrollArea::both()
                            .id_salt("guest-saved-diff")
                            .max_height(180.)
                            .show_rows(ui, height, patch.line_count(), |ui, rows| {
                                for i in rows {
                                    ui.monospace(patch.line(i));
                                }
                            });
                    }
                }
                if let Some(text) = &versions.text {
                    ScrollArea::both()
                        .id_salt("guest-version-text")
                        .max_height(180.)
                        .show(ui, |ui| {
                            ui.monospace(text.as_str());
                        });
                }
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
                if self.guest_file_editor.editable() {
                    ui.label(close_draft_question(&document.key, french));
                } else {
                    ui.label(if french {"Abandonner ce brouillon non enregistré ? Copiez-le d’abord si vous voulez le conserver."}else{"Discard this unsaved draft? Copy it first if you want to keep it."});
                }
                ui.horizontal(|ui| {
                    if self.guest_file_editor.editable()
                        && ui
                            .add_enabled(
                                !busy,
                                egui::Button::new(if french {
                                    "Enregistrer et fermer"
                                } else {
                                    "Save and close"
                                }),
                            )
                            .clicked()
                    {
                        self.start_guest_save_then_close(&document.key);
                    }
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
                        self.confirm_guest_close(&document.key, true);
                    }
                });
            }
        }
        if let Some(notice) = self.guest_file_notice() {
            ui.colored_label(Color32::LIGHT_RED, notice.text(french));
        }
    }
}

fn guest_path_heading(ui: &mut Ui, path: &str, french: bool) {
    ui.horizontal(|ui| {
        ui.strong(path);
        if ui
            .small_button(if french {
                "Copier le chemin"
            } else {
                "Copy path"
            })
            .clicked()
        {
            ui.ctx().copy_text(path.to_owned());
        }
    });
}

pub(crate) fn close_draft_question(key: &FileKey, french: bool) -> String {
    if french {
        format!(
            "Enregistrer les modifications de « {} » avant de fermer l’onglet ?",
            key.path
        )
    } else {
        format!("Save changes to “{}” before closing the tab?", key.path)
    }
}
