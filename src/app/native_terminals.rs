//! Explicit native-host pane routing. The runtime outlives every pane; no owner
//! store or network operation is performed by these UI methods.
use super::*;
use crate::host_opening_adapter::{HostConnection, OwnerStore};
use crate::terminal::{host_opening, host_runtime, host_terminal_list};
use std::sync::Arc;

#[derive(Default)]
pub(super) struct State {
    pub runtime: Option<host_runtime::Runtime>,
    pub drain: Option<host_runtime::Drain>,
    pub picker: Option<Picker>,
    pub panel: crate::host_existing_terminals_panel::HostExistingTerminalsPanel,
}

pub(super) struct Picker {
    pub alias: String,
    pub replaces: Option<u64>,
    pub loader: host_terminal_list::TerminalList,
    pub result: Option<host_terminal_list::Polled>,
    pub error: Option<crate::host_existing_terminals_panel::ExistingTerminalsError>,
}

impl Tessel {
    pub(super) fn wake_native_terminal(&mut self, id: u64, ctx: &egui::Context) {
        let Some(session) = self.sessions.iter().find(|s| s.id == id) else {
            return;
        };
        let Launch::Ssh(host) = &session.launch else {
            return;
        };
        let alias = host.target.alias.clone();
        let identity = session
            .native_host
            .as_ref()
            .and_then(|s| s.identity.clone());
        let Some(identity) = identity else {
            self.open_native_picker(&alias, ctx);
            self.native_terminals.picker.as_mut().unwrap().replaces = Some(id);
            return;
        };
        self.open_native_terminal_into(
            &alias,
            Some((identity.epoch, identity.terminal, true)),
            Some(id),
            ctx,
        );
    }

    pub(super) fn native_modal_open(&self) -> bool {
        self.native_terminals.picker.is_some()
            || self
                .native_terminals
                .runtime
                .as_ref()
                .is_some_and(host_runtime::Runtime::closing)
    }

    pub(super) fn open_native_picker(&mut self, alias: &str, ctx: &egui::Context) {
        self.command_palette.open = false;
        self.finder.close();
        self.native_terminals.picker = Some(Picker {
            alias: alias.into(),
            replaces: None,
            loader: host_terminal_list::TerminalList::new(alias.into()),
            result: None,
            error: None,
        });
        self.native_terminals.panel.reset_selection();
        self.refresh_native_picker(ctx);
    }

    fn refresh_native_picker(&mut self, ctx: &egui::Context) {
        use crate::host_existing_terminals_panel::ExistingTerminalsError as Error;
        let Some(picker) = &mut self.native_terminals.picker else {
            return;
        };
        picker.result = None;
        picker.error = None;
        let Some(connection) = self.native_hosts.connection(&picker.alias) else {
            picker.error = Some(Error::NotConnected);
            return;
        };
        let client = connection.client().clone();
        let host = Arc::new(HostConnection::new(
            client.clone(),
            client.options().request_timeout,
        ));
        let repaint = ctx.clone();
        if picker
            .loader
            .refresh(host, Arc::new(OwnerStore), move || {
                repaint.request_repaint()
            })
            .is_err()
        {
            picker.error = Some(Error::LocalBusy);
        }
    }

    pub(super) fn dispatch_native_actions(&mut self, ctx: &egui::Context) {
        use crate::host_terminal_panel::HostTerminalAction as Action;
        let actions: Vec<_> = self
            .sessions
            .iter_mut()
            .filter_map(|session| {
                session.native_host.as_ref()?;
                let Launch::Ssh(host) = &session.launch else {
                    return None;
                };
                let terminal = session.terminal.as_mut()?;
                let action = if terminal.native_cancelled() {
                    Some(Action::ClosePane)
                } else {
                    terminal.take_native_action()
                }?;
                Some((session.id, host.target.alias.clone(), action))
            })
            .collect();
        for (id, alias, action) in actions {
            match action {
                Action::ClosePane | Action::Detach => self.remove_session(id),
                Action::BrowseExistingTerminals => {
                    // Keep this recovery card: it may hold the only in-memory
                    // retry key or an access grant that has not been saved yet.
                    self.open_native_picker(&alias, ctx);
                }
                Action::RestartNew => self.open_native_terminal_into(&alias, None, Some(id), ctx),
                _ => {} // Backend-owned controls are dispatched inside Terminal.
            }
        }
    }

    pub(super) fn show_native_picker(&mut self, ctx: &egui::Context) {
        use crate::host_existing_terminals_panel::*;
        use host_terminal_list::{IndexError, ListFailure, Polled, Saved};
        let Some(picker) = &self.native_terminals.picker else {
            return;
        };
        let alias = picker.alias.clone();
        let replaces = picker.replaces;
        let live_epoch = self
            .native_hosts
            .connection(&alias)
            .and_then(|c| c.client().epoch());
        let listing = match &picker.result {
            Some(Polled::Listed(list)) => Some(list),
            _ => None,
        };
        let entries: Vec<_> = listing
            .into_iter()
            .flat_map(|list| {
                list.entries.iter().map(|entry| ExistingTerminalEntry {
                    terminal_id: entry.terminal.as_str(),
                    pid: entry.pid,
                    size: entry.size,
                    attached: entry.attached,
                    leased_on_host: entry.leased_on_host,
                    retained_bytes: entry.retained_bytes,
                    exited: entry.exited.map(|e| ExistingTerminalExit {
                        exit_code: e.code,
                        signal: e.signal,
                    }),
                    saved_lease: match entry.saved {
                        Saved::Yes => SavedLease::Yes,
                        Saved::No => SavedLease::No,
                        Saved::Unknown => SavedLease::Unknown,
                    },
                    open_locally: self.sessions.iter().any(|session| {
                        matches!(&session.launch, Launch::Ssh(host) if host.target.alias == alias)
                            && session.terminal.is_some()
                            && session
                                .native_host
                                .as_ref()
                                .and_then(|h| h.identity.as_ref())
                                .is_some_and(|i| {
                                    i.epoch == list.epoch && i.terminal == entry.terminal
                                })
                    }),
                })
            })
            .collect();
        let pending: Vec<_> = listing
            .into_iter()
            .flat_map(|list| {
                list.pending.iter().map(|row| PendingCreationEntry {
                    id: &row.id,
                    epoch: row.epoch.as_str(),
                    current: row.current,
                })
            })
            .collect();
        let state = if let Some(reason) = picker.error {
            ExistingTerminalsState::Error { reason }
        } else {
            match &picker.result {
                None => ExistingTerminalsState::Loading,
                Some(Polled::Failed { failure, .. }) => ExistingTerminalsState::Error {
                    reason: match failure {
                        ListFailure::NotConnected => ExistingTerminalsError::NotConnected,
                        ListFailure::TimedOut => ExistingTerminalsError::TimedOut,
                        ListFailure::EpochChanged => ExistingTerminalsError::EpochChanged,
                        ListFailure::Unsupported => ExistingTerminalsError::Unsupported,
                        ListFailure::RequestFailed => ExistingTerminalsError::RequestFailed,
                    },
                },
                Some(Polled::Listed(list)) => ExistingTerminalsState::Loaded {
                    list_epoch: list.epoch.as_str(),
                    entries: &entries,
                    truncated: list.truncated,
                    store_error: list.store.err().map(|error| match error {
                        IndexError::Locked => LeaseStoreError::Locked,
                        IndexError::Corrupt => LeaseStoreError::Corrupt,
                        IndexError::Unavailable => LeaseStoreError::Unavailable,
                    }),
                    pending: &pending,
                    pending_truncated: list.pending_truncated,
                },
            }
        };
        let action = self.native_terminals.panel.show_modal(
            ctx,
            &ExistingTerminalsInput {
                id_salt: egui::Id::new("native-existing-terminals"),
                host_label: &alias,
                live_epoch: live_epoch.as_ref().map(|e| e.as_str()),
                state,
                can_attach: replaces.is_some() || self.sessions.len() < 100,
                can_open_new: replaces.is_some() || self.sessions.len() < 100,
                french: crate::i18n::is_french(),
            },
        );
        match action {
            Some(ExistingTerminalsAction::Close) => self.native_terminals.picker = None,
            Some(ExistingTerminalsAction::Refresh) => self.refresh_native_picker(ctx),
            Some(ExistingTerminalsAction::OpenNewTerminal) => {
                self.native_terminals.picker = None;
                self.open_native_terminal_into(&alias, None, replaces, ctx);
            }
            Some(
                action @ (ExistingTerminalsAction::ResumeSaved { .. }
                | ExistingTerminalsAction::AttachObserver { .. }),
            ) => {
                let resume = matches!(action, ExistingTerminalsAction::ResumeSaved { .. });
                let (ExistingTerminalsAction::ResumeSaved { epoch, terminal_id }
                | ExistingTerminalsAction::AttachObserver { epoch, terminal_id }) = action
                else {
                    unreachable!()
                };
                let (Some(epoch), Some(terminal)) = (
                    tessel_host_protocol::Epoch::parse(&epoch),
                    tessel_host_protocol::TerminalId::parse(&terminal_id),
                ) else {
                    return;
                };
                if live_epoch.as_ref() != Some(&epoch) {
                    self.refresh_native_picker(ctx);
                    return;
                }
                self.native_terminals.picker = None;
                self.open_native_terminal_into(
                    &alias,
                    Some((epoch, terminal, resume)),
                    replaces,
                    ctx,
                );
            }
            Some(ExistingTerminalsAction::FocusLocalTerminal { epoch, terminal_id }) => {
                if live_epoch.as_ref().is_none_or(|e| e.as_str() != epoch) {
                    return;
                }
                if let Some(session) = self.sessions.iter().find(|session| {
                    matches!(&session.launch, Launch::Ssh(host) if host.target.alias == alias)
                        && session.terminal.is_some()
                        && session
                            .native_host
                            .as_ref()
                            .and_then(|h| h.identity.as_ref())
                            .is_some_and(|i| {
                                i.epoch.as_str() == epoch && i.terminal.as_str() == terminal_id
                            })
                }) {
                    let id = session.id;
                    self.native_terminals.picker = None;
                    self.retire_native_placeholder(replaces, id);
                    self.focus_native_pane(id, ctx);
                }
            }
            None => {}
        }
    }

    pub(super) fn begin_native_exit(&mut self, ctx: &egui::Context) {
        self.command_palette.open = false;
        self.finder.close();
        self.save();
        self.pending_exit = false;
        self.native_terminals.picker = None;
        if let Some(runtime) = &mut self.native_terminals.runtime {
            runtime.begin_close();
            // Keep saved identities; dropping only the view detaches, never kills.
            for session in &mut self.sessions {
                if session.native_host.is_some() {
                    session.terminal = None;
                }
            }
            ctx.request_repaint();
        } else {
            self.allow_exit = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    pub(super) fn show_native_drain(&mut self, ctx: &egui::Context) {
        let Some(runtime) = &self.native_terminals.runtime else {
            return;
        };
        if !runtime.closing() {
            return;
        }
        egui::Modal::new(egui::Id::new("native-terminal-drain")).show(ctx, |ui| {
            ui.label(tr("Finishing terminal access storage before closing"));
            ui.label(tr(
                "Remote terminals remain on the host. Wait until their recovery access is saved.",
            ));
            ui.spinner();
            if let Some(drain) = &self.native_terminals.drain
                && !drain.unsettled.is_empty()
            {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    tr("Some terminal access changes could not be saved."),
                );
                if (drain.unsettled.not_saved > 0 || drain.unsettled.not_forgotten > 0)
                    && ui.button(tr("Retry saving")).clicked()
                {
                    runtime.desk().retry_failures();
                }
            }
            ui.label(tr(
                "Quitting now may leave a created terminal without saved recovery access.",
            ));
            if ui.button(tr("Quit anyway")).clicked() {
                self.allow_exit = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
    }

    fn ensure_native_runtime(&mut self) -> bool {
        if self.native_terminals.runtime.is_none() {
            match host_runtime::Runtime::start(Arc::new(OwnerStore)) {
                Ok(runtime) => self.native_terminals.runtime = Some(runtime),
                Err(_) => {
                    self.error = tr("Cannot start terminal access storage").into();
                    return false;
                }
            }
        }
        !self
            .native_terminals
            .runtime
            .as_ref()
            .is_some_and(host_runtime::Runtime::closing)
    }

    pub(super) fn open_native_terminal(
        &mut self,
        alias: &str,
        existing: Option<(
            tessel_host_protocol::Epoch,
            tessel_host_protocol::TerminalId,
            bool,
        )>,
        ctx: &egui::Context,
    ) {
        self.open_native_terminal_into(alias, existing, None, ctx);
    }

    fn retire_native_placeholder(&mut self, placeholder: Option<u64>, chosen: u64) {
        if let Some(id) = placeholder.filter(|id| *id != chosen)
            && self
                .sessions
                .iter()
                .any(|s| s.id == id && s.native_host.is_some() && s.terminal.is_none())
        {
            self.remove_session(id);
        }
    }

    fn focus_native_pane(&mut self, id: u64, ctx: &egui::Context) {
        let Some(session) = self.sessions.iter().find(|s| s.id == id) else {
            return;
        };
        let project = session.project;
        if let Some(terminal) = &session.terminal {
            terminal.request_focus(ctx);
        }
        self.active_session = Some(id);
        self.view = View::Work;
        self.overlay = None;
        if let Some(layout) = self
            .state
            .terminal_layouts
            .get_mut(&crate::terminal_layout::project_key(project))
            && layout.maximized.is_some()
        {
            layout.maximized = Some(id);
            self.grid_dirty = true;
        }
        if self.state.active_project != project {
            self.state.active_project = project;
            self.refresh_project();
        }
        self.save();
    }

    fn open_native_terminal_into(
        &mut self,
        alias: &str,
        existing: Option<(
            tessel_host_protocol::Epoch,
            tessel_host_protocol::TerminalId,
            bool,
        )>,
        replaces: Option<u64>,
        ctx: &egui::Context,
    ) {
        let mut destination = replaces.filter(|id| {
            self.sessions.iter().any(|s| {
                s.id == *id
                    && s.native_host.is_some()
                    && matches!(&s.launch, Launch::Ssh(host) if host.target.alias == alias)
            })
        });
        if let Some((epoch, terminal, _)) = &existing
            && let Some(session) = self
                .sessions
                .iter()
                .filter(|s| {
                    matches!(&s.launch, Launch::Ssh(host) if host.target.alias == alias)
                        && s.native_host
                            .as_ref()
                            .and_then(|s| s.identity.as_ref())
                            .is_some_and(|i| &i.epoch == epoch && &i.terminal == terminal)
                })
                .min_by_key(|s| s.terminal.is_none())
        {
            let id = session.id;
            if session.terminal.is_some() {
                self.retire_native_placeholder(replaces, id);
                self.focus_native_pane(id, ctx);
                return;
            }
            destination = Some(id);
        }
        if destination.is_none() && self.sessions.len() >= 100 {
            self.error = tr("Close a pane before opening another terminal").into();
            return;
        }
        if !self.ensure_native_runtime() {
            return;
        }
        let Some(connection) = self.native_hosts.connection(alias) else {
            self.error = tr("Connect the native host service first").into();
            return;
        };
        let client = connection.client().clone();
        let Some(hello) = client.hello() else {
            self.error = tr("The native host is disconnected").into();
            return;
        };
        if !hello
            .capabilities
            .contains(&tessel_host_protocol::Capability::Pty)
        {
            self.error = tr("This host does not grant terminal access").into();
            return;
        }
        let host = self
            .state
            .hosts
            .iter()
            .find(|host| host.alias == alias)
            .cloned()
            .unwrap_or_else(|| Host {
                alias: alias.to_owned(),
                folder: String::new(),
                ..Default::default()
            });
        let identity =
            existing.as_ref().map(
                |(epoch, terminal, _)| workspace::SavedHostTerminalIdentity {
                    epoch: epoch.clone(),
                    terminal: terminal.clone(),
                },
            );
        let target = match existing {
            Some((epoch, terminal, resume)) => host_opening::Target::Existing {
                alias: alias.into(),
                epoch,
                terminal,
                resume,
            },
            None => host_opening::Target::New(host_opening::Spec {
                alias: alias.into(),
                argv: Vec::new(),
                cwd: (!host.folder.is_empty()).then(|| host.folder.clone()),
                env: Default::default(),
                size: tessel_host_protocol::pty::Size {
                    rows: 30,
                    cols: 100,
                },
            }),
        };
        let id = destination.unwrap_or(self.next_session);
        let Some(next) = self
            .next_session
            .checked_add(u64::from(destination.is_none()))
        else {
            self.error = "Session identifiers exhausted".into();
            return;
        };
        let terminal = Terminal::spawn_native(
            HostConnection::new(client.clone(), client.options().request_timeout)
                .with_info(connection.info_cache()),
            target,
            self.native_terminals
                .runtime
                .as_ref()
                .expect("runtime prepared"),
            ctx.clone(),
        );
        if let Some(id) = destination {
            if let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) {
                session.terminal = Some(terminal);
                session.native_host = Some(workspace::SavedHostTerminal { identity });
                session.error.clear();
            }
            self.retire_native_placeholder(replaces, id);
            self.focus_native_pane(id, ctx);
            return;
        }
        self.next_session = next;
        self.sessions.push(Session {
            profile_data_dir: None,
            profile_root: None,
            profile_migration: None,
            resume_required: false,
            ssh_retry: Default::default(),
            ssh_tmux_id: None,
            native_host: Some(workspace::SavedHostTerminal { identity }),
            restarting: None,
            codex_discovery: None,
            codex_restore: None,
            model_picker: Default::default(),
            last_model: None,
            model_save_attempt: None,
            settings_changed: false,
            codex_seed: None,
            conversation_id: None,
            profile_binding: None,
            launch_fingerprint: None,
            session_options: None,
            worktree: None,
            pane_name: None,
            broadcast: true,
            id,
            project: self.state.active_project,
            title: format!("SSH · {alias}"),
            launch: Launch::ssh(host),
            cwd: self
                .state
                .active()
                .map(|p| p.path.clone())
                .unwrap_or_else(launch::default_directory),
            scrollback: self.state.terminal_prefs.scrollback,
            terminal: Some(terminal),
            loading: None,
            error: String::new(),
        });
        self.focus_native_pane(id, ctx);
    }

    pub(super) fn poll_native_terminals(&mut self, ctx: &egui::Context) {
        let mut changed = false;
        for session in &mut self.sessions {
            let Some(saved) = &mut session.native_host else {
                continue;
            };
            let Some(identity) = session
                .terminal
                .as_ref()
                .and_then(Terminal::native_identity)
            else {
                continue;
            };
            if saved
                .identity
                .as_ref()
                .is_none_or(|old| old.epoch != identity.epoch || old.terminal != identity.terminal)
            {
                saved.identity = Some(identity);
                changed = true;
            }
        }
        if changed {
            self.save();
        }
        if let Some(runtime) = &self.native_terminals.runtime {
            let drain = runtime.frame();
            if drain.ready {
                self.allow_exit = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else if runtime.closing() || drain.openings > 0 || drain.writing {
                ctx.request_repaint_after(Duration::from_millis(50));
            }
            self.native_terminals.drain = Some(drain);
        }
        if let Some(picker) = &mut self.native_terminals.picker
            && let Some(result) = picker.loader.poll()
        {
            picker.result = Some(result);
            picker.error = None;
        }
    }
}
