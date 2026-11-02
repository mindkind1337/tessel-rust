use crate::vault::Vault;
use eframe::egui;
use std::sync::mpsc::{self, Receiver};
#[derive(Default)]
pub struct VaultPanel {
    vault: Option<Vault>,
    pending: Option<Receiver<Result<(Vault, String), String>>>,
    status: String,
    remove: Option<usize>,
}
impl VaultPanel {
    pub fn show(&mut self, ui: &mut egui::Ui, browser: &mut crate::browser::Browser) {
        if let Some(rx) = &self.pending
            && let Ok(result) = rx.try_recv()
        {
            self.pending = None;
            match result {
                Ok((vault, status)) => {
                    self.vault = Some(vault);
                    self.status = status;
                }
                Err(e) => self.status = e,
            }
        }
        ui.heading("Private saved passwords");
        ui.label("Encrypted by Windows for this user. Kept outside projects. Never exposed by guest or agent tools.");
        ui.label("Fill is manual, HTTPS and exact-origin only. It does not submit the form.");
        if self.pending.is_some() {
            ui.spinner();
            ui.label("Working…");
            return;
        }
        if self.vault.is_none() {
            if ui.button("Unlock local vault").clicked() {
                let (tx, rx) = mpsc::channel();
                self.pending = Some(rx);
                std::thread::spawn(move || {
                    let _ = tx.send(Vault::open_default().map(|v| (v, "Unlocked".into())));
                });
            }
        } else {
            ui.horizontal(|ui| {
                if ui.button("Import browser CSV…").clicked() {
                    let mut vault = self.vault.take().unwrap();
                    let (tx, rx) = mpsc::channel();
                    self.pending = Some(rx);
                    std::thread::spawn(move || {
                        let status = match rfd::FileDialog::new()
                            .add_filter("Password export", &["csv"])
                            .pick_file()
                        {
                            Some(path) => match vault.import_csv(&path) {
                                Ok(n) => format!("Imported {n} records"),
                                Err(e) => e,
                            },
                            None => "Import cancelled".into(),
                        };
                        let _ = tx.send(Ok((vault, status)));
                    });
                }
                if ui.button("Lock vault").clicked() {
                    self.vault = None;
                    self.remove = None;
                    self.status = "Locked".into();
                }
            });
            ui.collapsing("Export from your browser",|ui| {
                ui.label("Chrome: Password Manager → Settings → Export passwords.");
                ui.label("Edge: Settings → Passwords → saved-passwords menu → Export.");
                ui.label("Firefox: Passwords → menu → Export passwords.");
                ui.label("The CSV contains readable passwords. Remove it after importing when no longer needed. Passkeys and verification codes are not imported.");
            });
            if let Some(vault) = &self.vault {
                egui::ScrollArea::vertical().show(ui,|ui| {
                    for (index,account) in vault.accounts.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(&account.origin);ui.label(&account.username);
                            if ui.button("Fill in private browser").clicked() {
                                self.status=match browser.fill_account(account) {Ok(())=>"Fill requested for the matching page. Check the form in Browser; no login was submitted.".into(),Err(e)=>e};
                            }
                            if ui.button("Remove…").clicked() {self.remove=Some(index);}
                        });
                    }
                });
            }
            if let Some(index) = self.remove {
                ui.horizontal(|ui| {
                    ui.label("Remove this account from the local vault?");
                    if ui.button("Confirm removal").clicked() {
                        if let Some(vault) = &mut self.vault {
                            self.status = vault
                                .remove(index)
                                .map(|_| "Removed".into())
                                .unwrap_or_else(|e| e);
                        }
                        self.remove = None;
                    }
                    if ui.button("Keep").clicked() {
                        self.remove = None;
                    }
                });
            }
        }
        ui.label(&self.status);
    }
}
