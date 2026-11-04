//! UI controller: metadata only; the worker owns the decrypted vault.
use crate::browser_passwords_panel::{
    BrowserPasswordsAction, BrowserPasswordsInput, BrowserPasswordsPanel, StatusBanner,
};
use crate::{
    i18n::tr,
    vault_worker::{Command, Effect, Snapshot, Worker},
};
use eframe::egui;
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct VaultPanel {
    dialog: Option<rfd::FileDialog>,
    worker: Option<Worker>,
    snapshot: Option<Snapshot>,
    pending: Option<u64>,
    picker: Option<mpsc::Receiver<Option<std::path::PathBuf>>>,
    picker_cancelled: bool,
    escape_held: bool,
    panel: BrowserPasswordsPanel,
    status: String,
    warning: bool,
    fill: Option<(Instant, crate::browser::FillReply)>,
}
impl VaultPanel {
    /// Run before Settings navigation, which otherwise consumes Escape first.
    pub fn handle_escape(&mut self, ctx: &egui::Context, navigation_focused: bool) -> bool {
        let down = ctx.input(|i| i.key_down(egui::Key::Escape));
        if !down {
            self.escape_held = false;
        }
        if self.escape_held {
            return ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        }
        if self.snapshot.is_none() {
            return false;
        }
        if egui::Popup::is_any_open(ctx)
            || (navigation_focused
                && !self.panel.has_pending_confirmation()
                && !self.panel.is_profile_popup_open())
        {
            // The first press belongs to that control; retain ownership of its
            // repeats so closing it cannot turn the same physical press into Lock.
            self.escape_held = down;
            return false;
        }
        let pressed = ctx.input(|i| {
            i.focused
                && i.events.iter().any(|event| {
                    matches!(event,
            egui::Event::Key { key: egui::Key::Escape, pressed: true, repeat: false, modifiers, .. }
            if modifiers.matches_logically(egui::Modifiers::NONE))
                })
        });
        if matches!(
            self.panel.handle_escape(ctx),
            Some(BrowserPasswordsAction::Close)
        ) {
            self.suspend();
        }
        self.escape_held = pressed && down;
        pressed
    }
    #[cfg(feature = "visual-test")]
    pub fn load_fixture(&mut self) {
        self.worker = Some(Worker::fixture().expect("synthetic vault fixture"));
        self.pending = Some(0);
        self.panel.reset_on_open();
        if std::env::args().any(|arg| arg == "--vault-export-fixture") {
            self.panel.set_export_help_open(true);
        }
    }
    pub fn with_parent(parent: &eframe::CreationContext<'_>) -> Self {
        Self {
            dialog: Some(rfd::FileDialog::new().set_parent(parent)),
            ..Default::default()
        }
    }
    pub fn suspend(&mut self) {
        if self.worker.is_some()
            || self.fill.is_some()
            || self.snapshot.is_some()
            || self.picker.is_some()
        {
            self.worker = None;
            self.snapshot = None;
            self.pending = None;
            // Keep the secret-free dialog slot until it closes, so repeated
            // lock/unlock cannot spawn an unbounded number of native dialogs.
            self.picker_cancelled = self.picker.is_some();
            self.fill = None;
            self.panel.reset_on_open();
            self.status = tr("Locked").into();
            self.warning = false;
        }
    }
    fn submit(&mut self, command: Command) {
        let Some(revision) = self.snapshot.as_ref().map(|snapshot| snapshot.revision) else {
            return;
        };
        let Some(worker) = &mut self.worker else {
            return;
        };
        match worker.submit(revision, command) {
            Ok(id) => {
                self.pending = Some(id);
                self.status.clear();
                self.warning = false;
            }
            Err(error) => {
                self.status = tr(&error).into();
                self.warning = true;
            }
        }
    }
    fn poll(&mut self, browser: &crate::browser::Browser) {
        let picked = self
            .picker
            .as_ref()
            .and_then(|picker| match picker.try_recv() {
                Ok(path) => Some(path),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some(None),
            });
        if let Some(path) = picked {
            self.picker = None;
            let cancelled = std::mem::take(&mut self.picker_cancelled);
            match path.filter(|_| !cancelled) {
                Some(path) => self.submit(Command::ImportCsv(path)),
                None => {
                    self.status = tr("Import cancelled").into();
                    self.warning = false;
                }
            }
        }
        if let Some((started, reply)) = &self.fill {
            let result = match reply.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(
                    "Private fill could not be confirmed; check the login fields.".into(),
                )),
                Err(mpsc::TryRecvError::Empty) if started.elapsed() >= Duration::from_secs(10) => {
                    Some(Err(
                        "Private fill timed out; fields may have been filled.".into()
                    ))
                }
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                self.fill = None;
                self.warning = result.is_err();
                self.status = tr(&result
                    .map(|_| "Login fields filled; no form was submitted.".into())
                    .unwrap_or_else(|e| e))
                .into();
            }
        }
        let Some(result) = self.worker.as_ref().map(Worker::poll) else {
            return;
        };
        let reply = match result {
            Ok(Some(reply)) => reply,
            Ok(None) => return,
            Err(error) => {
                self.suspend();
                self.status = tr(&error).into();
                self.warning = true;
                return;
            }
        };
        if self.pending != Some(reply.request) {
            return;
        }
        self.pending = None;
        if reply.snapshot.is_none() {
            self.worker = None;
        }
        if let Some(snapshot) = reply.snapshot {
            if self
                .snapshot
                .as_ref()
                .is_none_or(|previous| previous.revision != snapshot.revision)
            {
                self.panel.invalidate_pending_confirmation();
            }
            self.snapshot = Some(snapshot);
        }
        self.warning = reply.result.is_err();
        self.status = match reply.result {
            Err(error) => tr(&error).into(),
            Ok(Effect::Opened) => tr("Unlocked").into(),
            Ok(Effect::Imported(count)) => format!("{} {count}", tr("Imported records:")),
            Ok(Effect::Removed) => tr("Removed").into(),
            Ok(Effect::Cancelled) => tr("Import cancelled").into(),
            Ok(Effect::Fill(account)) => {
                // The private bridge checks the live page again; account is wiped
                // here, never retained by this controller or handed to the widget.
                match browser.fill_account(&account) {
                    Ok(reply) => {
                        self.fill = Some((Instant::now(), reply));
                        tr("Checking the private login form…").into()
                    }
                    Err(error) => {
                        self.warning = true;
                        tr(&error).into()
                    }
                }
            }
        };
    }
    pub fn show(&mut self, ui: &mut egui::Ui, browser: &mut crate::browser::Browser, french: bool) {
        self.poll(browser);
        ui.label(tr("Encrypted by Windows for this user. Kept outside projects. Never exposed by guest or agent tools."));
        if self.worker.is_none() {
            if ui.button(tr("Unlock local vault")).clicked() {
                match Worker::open_default() {
                    Ok(worker) => {
                        self.worker = Some(worker);
                        self.pending = Some(0);
                        self.panel.reset_on_open();
                        self.status.clear();
                    }
                    Err(error) => {
                        self.status = tr(&error).into();
                        self.warning = true;
                    }
                }
            }
            ui.label(&self.status);
            return;
        }
        if ui.button(tr("Lock vault")).clicked() {
            self.suspend();
            return;
        }
        let busy = self.pending.is_some() || self.fill.is_some() || self.picker.is_some();
        if busy {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        if self.snapshot.is_none() {
            ui.spinner();
            ui.label(tr("Working…"));
            return;
        }
        ui.label(tr(
            "Agent access is not available in this version; saved passwords remain owner-only.",
        ));
        let action = self.panel.show(
            ui,
            BrowserPasswordsInput {
                entries: &self.snapshot.as_ref().unwrap().entries,
                vault_revision: self.snapshot.as_ref().unwrap().revision,
                sources: &[],
                can_fill_current_page: browser.private_login_origin().is_ok(),
                can_manage_agent_access: false,
                browser_import_available: false,
                embedded: true,
                busy,
                progress: None,
                status: Some(StatusBanner {
                    text: &self.status,
                    warn: self.warning,
                }),
                batch_results: None,
                french,
            },
        );
        if let Some(action) = action {
            match action {
                BrowserPasswordsAction::Close => self.suspend(),
                BrowserPasswordsAction::ImportCsv if !busy => {
                    let (sender, receiver) = mpsc::sync_channel(1);
                    let dialog = self
                        .dialog
                        .clone()
                        .unwrap_or_default()
                        .add_filter("Password export", &["csv"]);
                    // A native dialog must never hold the decrypted vault or its lock.
                    match std::thread::Builder::new()
                        .name("vault-csv-picker".into())
                        .spawn(move || {
                            let path = dialog.pick_file();
                            let _ = sender.send(path);
                        }) {
                        Ok(_) => {
                            self.picker = Some(receiver);
                            self.picker_cancelled = false;
                        }
                        Err(_) => {
                            self.status = tr("Cannot open CSV picker").into();
                            self.warning = true;
                        }
                    }
                }
                BrowserPasswordsAction::Delete { id, vault_revision }
                    if !busy
                        && self
                            .snapshot
                            .as_ref()
                            .is_some_and(|s| s.revision == vault_revision) =>
                {
                    self.submit(Command::Delete(id))
                }
                BrowserPasswordsAction::Fill { id, vault_revision }
                    if !busy
                        && self
                            .snapshot
                            .as_ref()
                            .is_some_and(|s| s.revision == vault_revision) =>
                {
                    match browser.private_login_origin() {
                        Ok(origin) => self.submit(Command::Fill { id, origin }),
                        Err(error) => {
                            self.status = tr(&error).into();
                            self.warning = true;
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn escape_clears_private_search_before_locking_and_parent_navigation() {
        let ctx = egui::Context::default();
        let mut panel = VaultPanel {
            snapshot: Some(Snapshot {
                revision: 1,
                entries: vec![],
            }),
            ..Default::default()
        };
        panel.panel.set_query("synthetic search");
        for phase in 0..3 {
            let pressed = phase != 1;
            let _ = ctx.run(
                egui::RawInput {
                    focused: true,
                    events: vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    ..Default::default()
                },
                |ctx| {
                    assert_eq!(panel.handle_escape(ctx, false), pressed);
                    assert!(
                        !ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
                    );
                },
            );
            assert!(panel.panel.query().is_empty());
            assert_eq!(panel.snapshot.is_some(), phase != 2);
        }
    }
    #[test]
    fn held_escape_does_not_escape_to_parent_after_lock() {
        let ctx = egui::Context::default();
        let mut panel = VaultPanel {
            snapshot: Some(Snapshot {
                revision: 1,
                entries: vec![],
            }),
            ..Default::default()
        };
        for repeat in [false, true, true] {
            let _ = ctx.run(
                egui::RawInput {
                    focused: true,
                    events: vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    ..Default::default()
                },
                |ctx| {
                    assert!(panel.handle_escape(ctx, false));
                    assert!(
                        !ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
                    );
                },
            );
            assert!(panel.snapshot.is_none());
        }
    }
    #[test]
    fn navigation_focus_and_native_popup_keep_escape() {
        for popup in [false, true] {
            let ctx = egui::Context::default();
            let mut panel = VaultPanel {
                snapshot: Some(Snapshot {
                    revision: 1,
                    entries: vec![],
                }),
                ..Default::default()
            };
            panel.panel.set_query("retained");
            let _ = ctx.run(
                egui::RawInput {
                    focused: true,
                    events: vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    ..Default::default()
                },
                |ctx| {
                    if popup {
                        egui::Popup::open_id(ctx, egui::Id::new("export-help-menu"));
                    }
                    assert!(!panel.handle_escape(ctx, !popup));
                    assert!(
                        ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
                    );
                },
            );
            assert!(panel.snapshot.is_some());
            assert_eq!(panel.panel.query(), "retained");
            egui::Popup::close_all(&ctx);
            let _ = ctx.run(
                egui::RawInput {
                    focused: true,
                    events: vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: true,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    ..Default::default()
                },
                |ctx| {
                    assert!(panel.handle_escape(ctx, false));
                    assert!(
                        !ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
                    );
                },
            );
            assert!(panel.snapshot.is_some());
            assert_eq!(panel.panel.query(), "retained");
        }
    }
}
