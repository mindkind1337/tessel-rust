use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use eframe::egui;
use zeroize::Zeroize;

use crate::{
    account::{Api, Flow, Session, Store, User},
    i18n::tr,
    tessel_sync::{
        SCRYPT_LOG_N, SCRYPT_P, SCRYPT_R, ShrinkWarning, SyncFloorFile, SyncHttpClient,
        SyncRemoteMeta, SyncVersionEntry, build_rust_sync_envelope, check_shrink_guard,
        decrypt_sync_envelope, encrypt_sync_envelope, extract_rust_bundle_from_envelope,
        load_dpapi_sync_key, phrase_is_weak, sanitize_workspace_json_for_sync, save_dpapi_sync_key,
        scrypt_derive_key, suggest_phrase,
    },
    workspace::Workspace,
};

enum Reply {
    Login(Session),
    Refreshed(User),
    LoggedOut,
    SyncStatus {
        meta: Option<SyncRemoteMeta>,
        versions: Vec<SyncVersionEntry>,
    },
    SyncUploaded {
        seq: u64,
        projects_count: usize,
    },
    SyncDownloaded {
        seq: u64,
        workspace: Box<Workspace>,
    },
    SyncDeleted,
}

#[derive(Default)]
pub struct AccountPanel {
    store: Option<Store>,
    flow: Option<Flow>,
    pending: Option<Receiver<Result<Reply, String>>>,
    status: String,
    last_poll: Option<Instant>,
    sync_phrase: String,
    show_phrase: bool,
    sync_meta: Option<SyncRemoteMeta>,
    sync_versions: Vec<SyncVersionEntry>,
    last_uploaded_projects: Option<usize>,
    pending_shrink_confirm: Option<ShrinkWarning>,
    confirm_delete_cloud: bool,
    restored_workspace: Option<Workspace>,
}

impl AccountPanel {
    fn run(&mut self, work: impl FnOnce() -> Result<Reply, String> + Send + 'static) {
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        thread::spawn(move || {
            let result = work();
            if let Err(mpsc::SendError(Ok(Reply::Login(s)))) = tx.send(result)
                && let Ok(api) = Api::production()
            {
                let _ = api.logout(&s.bearer());
            }
        });
    }

    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub fn is_signed_in(&self) -> bool {
        self.store.as_ref().is_some_and(|s| s.session.is_some())
    }

    #[allow(dead_code)]
    pub(crate) fn gpui_snapshot(&self) -> AccountGpuiSnapshot {
        let store_open = self.store.is_some();
        let (signed_in, user_name, user_email) =
            match self.store.as_ref().and_then(|s| s.session.as_ref()) {
                Some(sess) => (
                    true,
                    sess.user.name.clone().unwrap_or_default(),
                    sess.user.email.clone(),
                ),
                None => (false, String::new(), String::new()),
            };
        AccountGpuiSnapshot {
            store_open,
            signed_in,
            signing_in_browser: self.flow.is_some(),
            busy: self.pending.is_some(),
            user_name,
            user_email,
            sync_phrase_len: self.sync_phrase.len(),
            sync_seq: self.sync_meta.as_ref().map(|m| m.seq),
            sync_versions_count: self.sync_versions.len(),
            status: self.status.clone(),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn gpui_open_store(&mut self) {
        if self.store.is_none() {
            match Store::open_default() {
                Ok(s) => {
                    self.store = Some(s);
                    self.status.clear();
                }
                Err(e) => self.status = e,
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) fn gpui_begin_browser_sign_in(&mut self) {
        self.gpui_open_store();
        let Some(store) = &self.store else {
            return;
        };
        let result = crate::account::register_protocol().and_then(|_| store.begin());
        match result {
            Ok(flow) => {
                if webbrowser::open(&flow.login_url()).is_ok() {
                    self.flow = Some(flow);
                    self.status.clear();
                } else {
                    store.cancel(&flow);
                    self.status = "Could not open the sign-in browser".into();
                }
            }
            Err(e) => self.status = e,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn gpui_sign_out(&mut self) {
        if let Some(store) = &mut self.store {
            match store.clear() {
                Ok(Some(s)) => self.run(move || {
                    Api::production()?.logout(&s.bearer())?;
                    Ok(Reply::LoggedOut)
                }),
                Ok(None) => {}
                Err(e) => self.status = e,
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) fn gpui_suggest_sync_phrase(&mut self) {
        match suggest_phrase() {
            Ok(phrase) => {
                self.sync_phrase = phrase;
                self.show_phrase = true;
            }
            Err(e) => self.status = e,
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub(crate) struct AccountGpuiSnapshot {
    pub store_open: bool,
    pub signed_in: bool,
    pub signing_in_browser: bool,
    pub busy: bool,
    pub user_name: String,
    pub user_email: String,
    pub sync_phrase_len: usize,
    pub sync_seq: Option<u64>,
    pub sync_versions_count: usize,
    pub status: String,
}

impl AccountPanel {
    pub fn take_restored_workspace(&mut self) -> Option<Workspace> {
        self.restored_workspace.take()
    }

    pub fn poll(&mut self) {
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(result) => {
                    self.pending = None;
                    self.status = match result {
                        Ok(Reply::Login(s)) => {
                            let token = s.bearer();
                            match self
                                .store
                                .as_mut()
                                .ok_or("Account is locked".to_string())
                                .and_then(|store| store.set(s))
                            {
                                Ok(()) => "Signed in".into(),
                                Err(e) => {
                                    self.run(move || {
                                        Api::production()?.logout(&token)?;
                                        Ok(Reply::LoggedOut)
                                    });
                                    e
                                }
                            }
                        }
                        Ok(Reply::Refreshed(user)) => match self
                            .store
                            .as_mut()
                            .ok_or("Account is locked".to_string())
                            .and_then(|store| store.update_user(user))
                        {
                            Ok(()) => "Account verified".into(),
                            Err(e) => e,
                        },
                        Ok(Reply::LoggedOut) => {
                            self.sync_meta = None;
                            self.sync_versions.clear();
                            "Signed out".into()
                        }
                        Ok(Reply::SyncStatus { meta, versions }) => {
                            let msg = match &meta {
                                Some(m) => format!(
                                    "Cloud sync active · revision #{} ({} bytes)",
                                    m.seq, m.size
                                ),
                                None => "No encrypted snapshot stored on server yet.".into(),
                            };
                            self.sync_meta = meta;
                            self.sync_versions = versions;
                            msg
                        }
                        Ok(Reply::SyncUploaded {
                            seq,
                            projects_count,
                        }) => {
                            self.last_uploaded_projects = Some(projects_count);
                            self.pending_shrink_confirm = None;
                            if let Some(meta) = &mut self.sync_meta {
                                meta.seq = seq;
                            } else {
                                self.sync_meta = Some(SyncRemoteMeta {
                                    seq,
                                    updated_at: None,
                                    salt: None,
                                    size: 0,
                                });
                            }
                            format!("Encrypted snapshot uploaded (revision #{seq}).")
                        }
                        Ok(Reply::SyncDownloaded { seq, workspace }) => {
                            self.last_uploaded_projects = Some(workspace.projects.len());
                            self.restored_workspace = Some(*workspace);
                            format!("Decrypted snapshot #{seq} ready for restore preview.")
                        }
                        Ok(Reply::SyncDeleted) => {
                            self.sync_meta = None;
                            self.sync_versions.clear();
                            self.confirm_delete_cloud = false;
                            "Cloud sync data deleted.".into()
                        }
                        Err(e) => e,
                    };
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending = None;
                    self.status = "Account worker stopped".into();
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self
            .last_poll
            .is_some_and(|t| t.elapsed() < Duration::from_millis(250))
        {
            return;
        }
        self.last_poll = Some(Instant::now());
        if let (Some(store), Some(flow)) = (&self.store, &self.flow) {
            match store.complete(flow) {
                Ok(Some(code)) => {
                    let verifier = flow.verifier();
                    self.flow = None;
                    self.status = "Completing sign-in".into();
                    self.run(move || {
                        Api::production()?
                            .exchange(&code, &verifier)
                            .map(Reply::Login)
                    });
                }
                Err(e) => {
                    self.flow = None;
                    self.status = e;
                }
                Ok(None) => {}
            }
        }
    }

    #[allow(dead_code)]
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let empty = Workspace::default();
        self.show_with_workspace(ui, &empty);
    }

    pub fn show_with_workspace(&mut self, ui: &mut egui::Ui, workspace: &Workspace) {
        let french = crate::i18n::is_french();
        ui.heading(tr("Tessel account"));
        ui.label(tr("Your account session stays encrypted on this PC, outside projects and Live Share. Agents cannot access it."));
        ui.separator();
        if self.store.is_none() {
            if ui.button(tr("Open account")).clicked() {
                match Store::open_default() {
                    Ok(s) => {
                        self.store = Some(s);
                        self.status.clear();
                    }
                    Err(e) => self.status = e,
                }
            }
        } else if self.pending.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(if french {
                    "Opération chiffrée en cours…"
                } else {
                    "Encrypted account operation in progress…"
                });
            });
        } else if self.flow.is_some() {
            ui.label(tr("Finish signing in in your usual browser."));
            if ui.button(tr("Cancel sign-in")).clicked() {
                if let (Some(store), Some(flow)) = (&self.store, self.flow.take()) {
                    store.cancel(&flow);
                }
                self.status = "Sign-in cancelled".into();
            }
        } else if self.store.as_ref().is_some_and(|s| s.session.is_some()) {
            let (user_id, user_name, user_email, token, account_dir): (
                String,
                String,
                String,
                _,
                PathBuf,
            ) = {
                let store = self.store.as_ref().expect("opened");
                let session = store.session.as_ref().expect("session checked");
                (
                    session.user.id.clone(),
                    session.user.name.clone().unwrap_or_default(),
                    session.user.email.clone(),
                    session.bearer(),
                    store.directory().to_path_buf(),
                )
            };
            if !user_name.is_empty() {
                ui.strong(&user_name);
            }
            ui.label(&user_email);
            ui.horizontal(|ui| {
                let refresh_tok = token.clone();
                if ui.button(tr("Refresh account")).clicked() {
                    self.run(move || {
                        Api::production()?
                            .refresh(&refresh_tok)
                            .map(Reply::Refreshed)
                    });
                }
                if ui.button(tr("Sign out")).clicked() {
                    match self.store.as_mut().expect("opened").clear() {
                        Ok(Some(s)) => self.run(move || {
                            Api::production()?.logout(&s.bearer())?;
                            Ok(Reply::LoggedOut)
                        }),
                        Ok(None) => {}
                        Err(e) => self.status = e,
                    }
                }
            });

            ui.add_space(12.0_f32);
            ui.separator();
            ui.add_space(6.0_f32);
            self.show_encrypted_sync_section(ui, workspace, &user_id, &token, &account_dir, french);
        } else if ui.button(tr("Sign in with your browser")).clicked() {
            let result = crate::account::register_protocol()
                .and_then(|_| self.store.as_ref().expect("opened").begin());
            match result {
                Ok(flow) => {
                    if webbrowser::open(&flow.login_url()).is_ok() {
                        self.flow = Some(flow);
                        self.status.clear();
                    } else {
                        self.store.as_ref().expect("opened").cancel(&flow);
                        self.status = "Could not open the sign-in browser".into();
                    }
                }
                Err(e) => self.status = e,
            }
        }
        if !self.status.is_empty() {
            ui.add_space(6.0_f32);
            ui.label(tr(&self.status));
        }
        if self.pending.is_some() || self.flow.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
    }

    fn show_encrypted_sync_section(
        &mut self,
        ui: &mut egui::Ui,
        workspace: &Workspace,
        user_id: &str,
        bearer: &str,
        account_dir: &std::path::Path,
        french: bool,
    ) {
        let palette = crate::theme::active(ui.ctx()).spec().ui;
        ui.heading(if french {
            "Synchronisation chiffrée de bout en bout"
        } else {
            "End-to-end encrypted cloud sync"
        });
        ui.small(if french {
            "scrypt (N=131072, r=8, p=1) + AES-256-GCM · La phrase secrète ne quitte jamais ce PC. Jetons et conversations locales sont exclus."
        } else {
            "scrypt (N=131072, r=8, p=1) + AES-256-GCM · Your passphrase never leaves this PC. Tokens and local conversation IDs are stripped."
        });

        let has_cached_key = load_dpapi_sync_key(account_dir, user_id)
            .ok()
            .flatten()
            .is_some();
        if has_cached_key {
            ui.small(
                egui::RichText::new(if french {
                    "Clé dérivée mise en cache localement (Windows DPAPI)."
                } else {
                    "Derived sync key cached locally (Windows DPAPI)."
                })
                .color(palette.accent),
            );
        }

        ui.add_space(6.0_f32);
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.sync_phrase)
                    .password(!self.show_phrase)
                    .hint_text(if french {
                        "Phrase secrète de synchronisation (≥ 12 caractères)…"
                    } else {
                        "Sync passphrase (≥ 12 characters)…"
                    })
                    .desired_width(280.0_f32),
            );
            if ui
                .button(if self.show_phrase {
                    if french { "Masquer" } else { "Hide" }
                } else if french {
                    "Afficher"
                } else {
                    "Show"
                })
                .clicked()
            {
                self.show_phrase = !self.show_phrase;
            }
            if ui
                .button(if french {
                    "Suggérer une phrase"
                } else {
                    "Suggest phrase"
                })
                .clicked()
            {
                match suggest_phrase() {
                    Ok(p) => {
                        self.sync_phrase = p;
                        self.show_phrase = true;
                    }
                    Err(e) => self.status = e,
                }
            }
        });

        let creating_new = self.sync_meta.is_none();
        if creating_new
            && !self.sync_phrase.trim().is_empty()
            && let Some(weak_reason) = phrase_is_weak(&self.sync_phrase)
        {
            ui.colored_label(palette.warn, weak_reason);
        }

        if let Some(warn) = self.pending_shrink_confirm.clone() {
            ui.add_space(4.0_f32);
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(
                    palette.warn,
                    format!(
                        "Project count shrunk from {} to {}. Upload anyway?",
                        warn.shrunk_from, warn.shrunk_to
                    ),
                );
                if ui.button("Confirm upload").clicked() {
                    self.pending_shrink_confirm = None;
                    self.start_sync_upload(workspace, user_id, bearer, account_dir, true);
                }
                if ui.button(tr("Cancel")).clicked() {
                    self.pending_shrink_confirm = None;
                }
            });
        }

        ui.add_space(6.0_f32);
        ui.horizontal_wrapped(|ui| {
            if ui
                .button(if french {
                    "Vérifier l'état cloud"
                } else {
                    "Check cloud status"
                })
                .clicked()
            {
                let tok = bearer.to_owned();
                self.run(move || {
                    let client = SyncHttpClient::production()?;
                    let meta = client.fetch_meta(&tok)?;
                    let versions = if meta.is_some() {
                        client.list_versions(&tok).unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    Ok(Reply::SyncStatus { meta, versions })
                });
            }

            let can_use_key = !self.sync_phrase.trim().is_empty() || has_cached_key;
            if ui
                .add_enabled(
                    can_use_key,
                    egui::Button::new(if french {
                        "Téléverser un instantané"
                    } else {
                        "Upload snapshot"
                    }),
                )
                .clicked()
            {
                self.start_sync_upload(workspace, user_id, bearer, account_dir, false);
            }

            if ui
                .add_enabled(
                    can_use_key,
                    egui::Button::new(if french {
                        "Télécharger et prévisualiser"
                    } else {
                        "Download & preview restore"
                    }),
                )
                .clicked()
            {
                let phrase = self.sync_phrase.clone();
                let uid = user_id.to_owned();
                let tok = bearer.to_owned();
                let dir = account_dir.to_path_buf();
                self.run(move || {
                    let client = SyncHttpClient::production()?;
                    let Some((meta, ciphertext)) = client.fetch_snapshot(&tok)? else {
                        return Err("No cloud sync snapshot found on server.".into());
                    };
                    let key =
                        resolve_or_derive_sync_key(&dir, &uid, &phrase, meta.salt.as_deref())?;
                    let envelope = decrypt_sync_envelope(&key, &uid, &ciphertext)?;
                    let mut floor = SyncFloorFile::load(&dir);
                    let min_seq = floor.floors.get(&uid).copied().unwrap_or(0);
                    let (seq, bundle) =
                        extract_rust_bundle_from_envelope(&envelope, min_seq, false)?;
                    let ws: Workspace = serde_json::from_value(bundle)
                        .map_err(|e| format!("Invalid workspace snapshot: {e}"))?;
                    ws.validate()?;
                    let _ = floor.record_seq(&dir, &uid, seq);
                    Ok(Reply::SyncDownloaded {
                        seq,
                        workspace: Box::new(ws),
                    })
                });
            }

            if !self.confirm_delete_cloud {
                if ui
                    .button(if french {
                        "Supprimer les données cloud…"
                    } else {
                        "Delete cloud data…"
                    })
                    .clicked()
                {
                    self.confirm_delete_cloud = true;
                }
            } else {
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(if french {
                                "Confirmer la suppression"
                            } else {
                                "Confirm cloud delete"
                            })
                            .color(egui::Color32::WHITE),
                        )
                        .fill(palette.danger),
                    )
                    .clicked()
                {
                    let tok = bearer.to_owned();
                    self.run(move || {
                        SyncHttpClient::production()?.delete_cloud_sync(&tok)?;
                        Ok(Reply::SyncDeleted)
                    });
                }
                if ui.button(tr("Cancel")).clicked() {
                    self.confirm_delete_cloud = false;
                }
            }
        });

        if !self.sync_versions.is_empty() {
            ui.add_space(6.0_f32);
            ui.small(
                egui::RichText::new(format!(
                    "Recent cloud revisions ({}):",
                    self.sync_versions.len()
                ))
                .color(palette.text_dim),
            );
            for v in &self.sync_versions {
                ui.small(format!("• #{} · {} bytes", v.seq, v.size));
            }
        }
    }

    fn start_sync_upload(
        &mut self,
        workspace: &Workspace,
        user_id: &str,
        bearer: &str,
        account_dir: &std::path::Path,
        bypass_shrink: bool,
    ) {
        if !bypass_shrink
            && let Some(prev) = self.last_uploaded_projects
            && let Some(warn) = check_shrink_guard(prev, workspace.projects.len())
        {
            self.pending_shrink_confirm = Some(warn);
            return;
        }
        let creating_new = self.sync_meta.is_none();
        if creating_new
            && !self.sync_phrase.trim().is_empty()
            && let Some(weak) = phrase_is_weak(&self.sync_phrase)
        {
            self.status = weak.to_owned();
            return;
        }
        let Ok(raw_ws) = serde_json::to_value(workspace) else {
            self.status = "Cannot serialize workspace".into();
            return;
        };
        let projects_count = workspace.projects.len();
        let phrase = self.sync_phrase.clone();
        let uid = user_id.to_owned();
        let tok = bearer.to_owned();
        let dir = account_dir.to_path_buf();

        self.run(move || {
            let client = SyncHttpClient::production()?;
            let remote_meta = client.fetch_meta(&tok)?;
            let expected_seq = remote_meta.as_ref().map_or(0, |m| m.seq);
            let next_seq = expected_seq.saturating_add(1);

            let (salt, key) = if !phrase.trim().is_empty() {
                let mut salt_bytes = [0u8; 16];
                if let Some(existing_salt_b64) =
                    remote_meta.as_ref().and_then(|m| m.salt.as_deref())
                    && let Ok(decoded) = URL_SAFE_NO_PAD.decode(existing_salt_b64.trim())
                    && decoded.len() == 16
                {
                    salt_bytes.copy_from_slice(&decoded);
                } else {
                    getrandom::fill(&mut salt_bytes)
                        .map_err(|_| "Random generator unavailable".to_owned())?;
                }
                let derived =
                    scrypt_derive_key(&phrase, &salt_bytes, SCRYPT_LOG_N, SCRYPT_R, SCRYPT_P)?;
                let _ = save_dpapi_sync_key(&dir, &uid, &salt_bytes, &derived);
                (salt_bytes, derived)
            } else {
                load_dpapi_sync_key(&dir, &uid)?
                    .ok_or_else(|| "Enter your sync passphrase first.".to_owned())?
            };

            let sanitized = sanitize_workspace_json_for_sync(raw_ws);
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let device = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Tessel-PC".into());
            let envelope = build_rust_sync_envelope(next_seq, &device, now_ms, sanitized);
            let ciphertext = encrypt_sync_envelope(&key, &uid, &envelope)?;
            let salt_b64 = URL_SAFE_NO_PAD.encode(salt);
            let uploaded_seq =
                client.upload_snapshot(&tok, expected_seq, &salt_b64, &ciphertext)?;
            let mut floor = SyncFloorFile::load(&dir);
            let _ = floor.record_seq(&dir, &uid, uploaded_seq);
            Ok(Reply::SyncUploaded {
                seq: uploaded_seq,
                projects_count,
            })
        });
    }
}

fn resolve_or_derive_sync_key(
    dir: &std::path::Path,
    user_id: &str,
    phrase: &str,
    remote_salt_b64: Option<&str>,
) -> Result<zeroize::Zeroizing<[u8; 32]>, String> {
    if !phrase.trim().is_empty() {
        let salt_b64 = remote_salt_b64.ok_or("Cloud sync snapshot is missing salt metadata.")?;
        let decoded = URL_SAFE_NO_PAD
            .decode(salt_b64.trim())
            .map_err(|_| "Invalid cloud sync salt.")?;
        if decoded.len() != 16 {
            return Err("Invalid cloud sync salt length.".into());
        }
        let mut salt = [0u8; 16];
        salt.copy_from_slice(&decoded);
        let key = scrypt_derive_key(phrase, &salt, SCRYPT_LOG_N, SCRYPT_R, SCRYPT_P)?;
        let _ = save_dpapi_sync_key(dir, user_id, &salt, &key);
        return Ok(key);
    }
    if let Some((_salt, cached)) = load_dpapi_sync_key(dir, user_id)? {
        return Ok(cached);
    }
    Err("Enter your sync passphrase to decrypt this cloud snapshot.".into())
}

impl Drop for AccountPanel {
    fn drop(&mut self) {
        self.sync_phrase.zeroize();
        if let (Some(store), Some(flow)) = (&self.store, &self.flow) {
            store.cancel(flow);
        }
    }
}
