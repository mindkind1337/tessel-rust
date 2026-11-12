//! Native `egui` Live Share UI (`LiveShareDialog.vue`, `LiveShareAsk.vue`, `LiveSharePill.vue`,
//! `LiveSharePresence.vue`, and `liveshare/*` parity).

use crate::icons::{self, Icon};
use crate::share::{
    self, AskAnswer, ConnectedGuest, GuestConnectionState, GuestPhase, GuestSession, Host,
    HostCommand, HostSnapshot, Invitation, InviteLifetime, InviteSpec, InviteStatus, LiveShareAsk,
    MAX_INVITES, PaneGuestPresence, RightId, RightsPreset, ShareInvite, ShareRights, ShareSettings,
    SharedBrowserPage, SharedPaneSnapshot, SharedProject, TrustedPerson, clock_hhmm, initials,
    now_epoch_ms, short_code,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use eframe::egui::{
    self, Align, Color32, CornerRadius, FontId, Frame, Id, Key, Layout, Margin, Order, Pos2, Rect,
    RichText, ScrollArea, Sense, Stroke, StrokeKind, Ui, Vec2,
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    io::Write,
    net::{IpAddr, SocketAddr},
    path::Path,
    time::{Duration, Instant},
};

pub const DIALOG_MAX_WIDTH: f32 = 720.0_f32;
pub const ASK_MODAL_MAX_WIDTH: f32 = 560.0_f32;
pub const PILL_HEIGHT: f32 = crate::toolbar_fit::LIVE_SHARE_PILL_HEIGHT;
pub const PILL_POPOVER_WIDTH: f32 = 360.0_f32;
pub const ASK_ARM_DELAY: Duration = Duration::from_millis(900);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DialogTab {
    #[default]
    Share,
    Join,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HostSubTab {
    #[default]
    Overview,
    People,
    Invitations,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PeopleViewMode {
    #[default]
    List,
    Grid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PeopleFilter {
    #[default]
    All,
    InNow,
    Waiting,
    Offline,
    Trusted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GuestSubView {
    #[default]
    Panes,
    Files,
    Git,
    Tasks,
    Pages,
}

#[derive(Debug, Clone)]
pub struct InviteDraft {
    pub editing_id: Option<String>,
    pub open: bool,
    pub label: String,
    pub projects: Vec<u64>,
    pub include_later: bool,
    pub rights: ShareRights,
    pub lifetime: InviteLifetime,
    pub max_uses_text: String,
}

impl Default for InviteDraft {
    fn default() -> Self {
        Self {
            editing_id: None,
            open: false,
            label: String::new(),
            projects: Vec::new(),
            include_later: true,
            rights: ShareRights::view_only(),
            lifetime: InviteLifetime::OneDay,
            max_uses_text: String::new(),
        }
    }
}

impl InviteDraft {
    fn to_spec(&self, fallback_projects: &[u64]) -> InviteSpec {
        let projects = if self.projects.is_empty() {
            fallback_projects.to_vec()
        } else {
            self.projects.clone()
        };
        let max_uses = self
            .max_uses_text
            .trim()
            .parse::<u32>()
            .unwrap_or(0)
            .min(1000);
        InviteSpec {
            label: self.label.clone(),
            projects,
            include_later: self.include_later,
            rights: self.rights,
            lifetime: self.lifetime,
            max_uses,
        }
    }

    fn from_invite(inv: &ShareInvite) -> Self {
        Self {
            editing_id: Some(inv.id.clone()),
            open: true,
            label: inv.label.clone(),
            projects: inv.projects.clone(),
            include_later: inv.include_later,
            rights: inv.rights,
            lifetime: inv.lifetime,
            max_uses_text: if inv.max_uses > 0 {
                inv.max_uses.to_string()
            } else {
                String::new()
            },
        }
    }
}

#[derive(Debug, Clone)]
struct AskDraftState {
    key: String,
    projects_sig: Vec<u64>,
    opened_instant: Instant,
    opened_egui_time: f64,
    code_checked: bool,
    trust_next_time: bool,
    always_open: bool,
    trust_nudge: bool,
    selected_projects: Vec<u64>,
    rights: ShareRights,
}

/// Action returned to [`crate::app`] from toolbar/dialog interactions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShareUiAction {
    OpenDialog(DialogTab),
    CloseDialog,
}

pub struct SharePanel {
    host: Option<Host>,
    guest_session: Option<GuestSession>,
    guest_identity_hex: String,

    pub tab: DialogTab,
    pub host_sub_tab: HostSubTab,
    pub people_view: PeopleViewMode,
    pub people_filter: PeopleFilter,

    // Pre-share & host configuration
    pub host_name: String,
    pub bind_address: String,
    pub selected_projects: Vec<u64>,
    pub pre_share_invite: InviteDraft,
    pub settings: ShareSettings,
    pub trusted_people: Vec<TrustedPerson>,
    pub settings_learn_more_open: bool,
    pub confirm_stop_sharing: bool,

    // Invitations & People UI state
    pub invite_editor: Option<InviteDraft>,
    pub confirm_revoke_invite: Option<(String, bool)>,
    pub expanded_person_id: Option<String>,
    pub renaming_trust: Option<(String, String)>,
    pub copied_badge: Option<(String, Instant)>,

    // Guest Join UI state
    pub join_code: String,
    pub guest_name: String,
    pub follow_host: bool,
    pub guest_sub_view: GuestSubView,
    pub guest_selected_project: Option<u64>,
    pub guest_selected_pane: Option<u64>,
    pub guest_term_input: String,
    pub guest_editor_path: Option<String>,
    pub guest_editor_text: String,
    pub guest_new_file_name: String,
    pub guest_commit_msg: String,
    pub guest_git_status: String,
    pub guest_new_task_title: String,
    pub guest_tasks: Vec<Value>,
    guest_tasks_limited: bool,
    guest_tasks_error: String,

    // Toolbar pill popover & knock modal state
    pill_popover_open: bool,
    pill_popover_just_opened: bool,
    ask_draft: Option<AskDraftState>,

    pub error: String,
}

impl Default for SharePanel {
    fn default() -> Self {
        let mut id_bytes = [0u8; 32];
        let _ = getrandom::fill(&mut id_bytes);
        let guest_identity_hex: String = id_bytes.iter().map(|b| format!("{b:02x}")).collect();
        let default_user = std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "Host".into());
        Self {
            host: None,
            guest_session: None,
            guest_identity_hex,
            tab: DialogTab::Share,
            host_sub_tab: HostSubTab::Overview,
            people_view: PeopleViewMode::List,
            people_filter: PeopleFilter::All,
            host_name: default_user.clone(),
            bind_address: "127.0.0.1".into(),
            selected_projects: Vec::new(),
            pre_share_invite: InviteDraft::default(),
            settings: ShareSettings::default(),
            trusted_people: Vec::new(),
            settings_learn_more_open: false,
            confirm_stop_sharing: false,
            invite_editor: None,
            confirm_revoke_invite: None,
            expanded_person_id: None,
            renaming_trust: None,
            copied_badge: None,
            join_code: String::new(),
            guest_name: default_user,
            follow_host: true,
            guest_sub_view: GuestSubView::Panes,
            guest_selected_project: None,
            guest_selected_pane: None,
            guest_term_input: String::new(),
            guest_editor_path: None,
            guest_editor_text: String::new(),
            guest_new_file_name: String::new(),
            guest_commit_msg: String::new(),
            guest_git_status: String::new(),
            guest_new_task_title: String::new(),
            guest_tasks: Vec::new(),
            guest_tasks_limited: false,
            guest_tasks_error: String::new(),
            pill_popover_open: false,
            pill_popover_just_opened: false,
            ask_draft: None,
            error: String::new(),
        }
    }
}

impl SharePanel {
    #[must_use]
    pub fn hosting(&self) -> bool {
        self.host.is_some()
    }

    #[must_use]
    pub fn joined_or_waiting(&self) -> bool {
        self.guest_session.as_ref().is_some_and(|g| {
            matches!(
                g.snapshot().phase,
                GuestPhase::Connecting
                    | GuestPhase::WaitingForHost { .. }
                    | GuestPhase::Joined { .. }
            )
        })
    }

    #[must_use]
    pub fn is_pill_popover_open(&self) -> bool {
        self.pill_popover_open
    }

    /// What the compact toolbar pill shows (`None` = no pill: not hosting, not in a session).
    #[must_use]
    pub fn pill_state(&self) -> Option<crate::toolbar_fit::LiveSharePillState> {
        use crate::toolbar_fit::LiveSharePillState;
        if let Some(host) = &self.host {
            let snap = host.snapshot();
            let joined = snap
                .guests
                .iter()
                .filter(|g| g.state == GuestConnectionState::Joined)
                .count();
            return Some(LiveSharePillState::Host {
                joined,
                waiting: snap.asks.len(),
            });
        }
        let gs = self.guest_session.as_ref()?.snapshot();
        match gs.phase {
            GuestPhase::Connecting => Some(LiveSharePillState::Guest {
                host: String::new(),
                reconnecting: true,
            }),
            GuestPhase::WaitingForHost { host_name, .. } => Some(LiveSharePillState::Guest {
                host: host_name,
                reconnecting: true,
            }),
            GuestPhase::Joined { host_name, .. } => Some(LiveSharePillState::Guest {
                host: host_name,
                reconnecting: false,
            }),
            GuestPhase::Ended { .. } => None,
        }
    }

    pub fn close_pill_popover(&mut self) {
        self.pill_popover_open = false;
        self.pill_popover_just_opened = false;
    }

    #[must_use]
    pub fn has_pending_ask(&self) -> bool {
        self.host
            .as_ref()
            .is_some_and(|h| !h.snapshot().asks.is_empty())
    }

    /// Injects an active [`Host`] directly (used by headless UI tests and programmatic starts).
    pub fn set_host(&mut self, host: Host) {
        let snap = host.snapshot();
        self.selected_projects = snap.shared_projects.iter().map(|p| p.id).collect();
        self.settings = snap.settings;
        self.trusted_people = snap.trusted;
        self.confirm_stop_sharing = false;
        self.host = Some(host);
    }

    /// Injects an active [`GuestSession`] directly.
    pub fn set_guest_session(&mut self, session: GuestSession) {
        self.guest_session = Some(session);
        self.tab = DialogTab::Join;
    }

    pub fn stop_hosting(&mut self) {
        if let Some(host) = &self.host {
            let snap = host.snapshot();
            self.trusted_people = snap.trusted;
            self.settings = snap.settings;
        }
        self.host = None;
        self.confirm_stop_sharing = false;
        self.invite_editor = None;
        self.confirm_revoke_invite = None;
        self.ask_draft = None;
        self.pill_popover_open = false;
    }

    /// Synchronizes workspace state into the active [`Host`] and drains inbound guest commands.
    pub fn sync_host_workspace(
        &mut self,
        all_projects: &[SharedProject],
        panes: Vec<SharedPaneSnapshot>,
        pages: Vec<SharedBrowserPage>,
        active_project: Option<u64>,
        active_pane: Option<u64>,
    ) -> Vec<HostCommand> {
        if self.selected_projects.is_empty()
            && let Some(id) = active_project.or_else(|| all_projects.first().map(|p| p.id))
        {
            self.selected_projects.push(id);
        }
        self.selected_projects
            .retain(|id| all_projects.iter().any(|p| p.id == *id));
        if self.selected_projects.is_empty()
            && let Some(first) = all_projects.first()
        {
            self.selected_projects.push(first.id);
        }

        let Some(host) = &self.host else {
            return Vec::new();
        };
        let shared: Vec<SharedProject> = all_projects
            .iter()
            .filter(|p| self.selected_projects.contains(&p.id))
            .cloned()
            .collect();
        host.sync_workspace_state(shared, panes, pages, active_project, active_pane);
        host.drain_commands()
    }

    pub fn publish_tasks(&self, cards: Vec<crate::share::SharedTask>) {
        if let Some(host) = &self.host {
            host.publish_tasks(cards);
        }
    }
    pub fn task_scope(&self) -> Option<(Vec<u8>, Vec<u64>)> {
        self.host.as_ref().map(crate::share::Host::task_scope)
    }
    pub fn complete_task(
        &self,
        command: crate::share::GuestTaskCommand,
        commit: impl FnOnce(
            u64,
            &str,
            &crate::share::GuestTaskOp,
        ) -> Result<(bool, Vec<crate::share::SharedTask>), String>,
    ) {
        if let Some(host) = &self.host {
            host.complete_task(command, commit);
        } else {
            command.reject();
        }
    }

    /// Returns a map of `project_id -> Vec<guest_initials>` for the left sidebar `.osb-live` badge.
    #[must_use]
    pub fn shared_projects_sidebar_map(&self) -> HashMap<u64, Vec<String>> {
        let mut map = HashMap::new();
        if let Some(host) = &self.host {
            let snap = host.snapshot();
            for proj in &snap.shared_projects {
                let avatars: Vec<String> = snap
                    .guests
                    .iter()
                    .filter(|g| {
                        g.state == GuestConnectionState::Joined && g.projects.contains(&proj.id)
                    })
                    .map(|g| initials(&g.name))
                    .collect();
                map.insert(proj.id, avatars);
            }
        }
        map
    }

    #[must_use]
    pub fn pane_presence(&self, pane_id: u64) -> Vec<PaneGuestPresence> {
        self.host
            .as_ref()
            .map(|h| h.pane_presence(pane_id))
            .unwrap_or_default()
    }

    fn start_hosting_with(&mut self, all_projects: &[SharedProject]) {
        let chosen: Vec<SharedProject> = all_projects
            .iter()
            .filter(|p| self.selected_projects.contains(&p.id))
            .cloned()
            .collect();
        if chosen.is_empty() {
            self.error = "Tick at least one project.".into();
            return;
        }
        let ip: IpAddr = match self.bind_address.trim().parse() {
            Ok(ip) => ip,
            Err(_) => {
                self.error = "Enter a valid local IP address (for example 127.0.0.1).".into();
                return;
            }
        };
        let chosen_ids: Vec<u64> = chosen.iter().map(|p| p.id).collect();
        let spec = self.pre_share_invite.to_spec(&chosen_ids);
        match Host::start_multi(
            chosen,
            self.host_name.clone(),
            spec,
            self.settings.clone(),
            self.trusted_people.clone(),
            SocketAddr::new(ip, 0),
            ip,
        ) {
            Ok(host) => {
                self.host = Some(host);
                self.confirm_stop_sharing = false;
                self.error.clear();
            }
            Err(e) => self.error = e,
        }
    }

    fn mark_copied(&mut self, ctx: &egui::Context, key: impl Into<String>, text: String) {
        ctx.copy_text(text);
        self.copied_badge = Some((key.into(), Instant::now()));
    }

    fn is_copied(&self, key: &str) -> bool {
        self.copied_badge
            .as_ref()
            .is_some_and(|(k, at)| k == key && at.elapsed() <= Duration::from_millis(1600))
    }

    // -----------------------------------------------------------------------
    // 1. Toolbar Control (`LiveSharePill` + `360px` Quick-Manage Popover)
    // -----------------------------------------------------------------------

    pub fn show_toolbar_control(
        &mut self,
        ui: &mut Ui,
        all_projects: &[SharedProject],
        active_project: Option<u64>,
        french: bool,
    ) -> Option<ShareUiAction> {
        let mut ui_action = None;
        let host_snap = self.host.as_ref().map(Host::snapshot);
        let guest_snap = self.guest_session.as_ref().map(GuestSession::snapshot);
        let in_guest_session = guest_snap.as_ref().is_some_and(|g| {
            matches!(
                g.phase,
                GuestPhase::Connecting
                    | GuestPhase::WaitingForHost { .. }
                    | GuestPhase::Joined { .. }
            )
        });

        if host_snap.is_none() && !in_guest_session {
            self.pill_popover_open = false;
            let tip = if french {
                "Partager en direct"
            } else {
                "Live Share"
            };
            if share_icon_button(ui, Icon::ScreenShare, tip).clicked() {
                ui_action = Some(ShareUiAction::OpenDialog(DialogTab::Share));
            }
            return ui_action;
        }

        // Compact pill (`LiveSharePill.vue`, 20 px): the dot and the number of people (a guest:
        // the host's name); its words are in the tooltip and the popover.
        let Some(pill_state) = self.pill_state() else {
            return ui_action;
        };
        let pill_text = crate::toolbar_fit::live_share_pill_label(&pill_state, french);
        let pill_tip = crate::toolbar_fit::live_share_pill_tooltip(&pill_state, french);
        let waiting_count = match pill_state {
            crate::toolbar_fit::LiveSharePillState::Host { waiting, .. } => waiting,
            _ => 0,
        };
        let is_hosting_pill = host_snap.is_some();
        let show_follow_btn = matches!(
            guest_snap.as_ref().map(|g| &g.phase),
            Some(GuestPhase::Joined { .. })
        ) && !self.follow_host;

        let danger_red = Color32::from_rgb(0xef, 0x44, 0x44);
        let amber = Color32::from_rgb(0xf5, 0x9e, 0x0b);
        let accent = ui.visuals().selection.stroke.color;
        let dot_color = if is_hosting_pill {
            if crate::toolbar_fit::live_share_pill_dot_live(&pill_state) {
                danger_red
            } else {
                ui.visuals().weak_text_color()
            }
        } else {
            accent
        };

        let font = FontId::proportional(11.0_f32);
        let text_galley =
            ui.painter()
                .layout_no_wrap(pill_text.clone(), font.clone(), ui.visuals().text_color());
        let badge_extra = if waiting_count > 0 { 20.0_f32 } else { 0.0_f32 };
        // padding 0 7px 0 6px, dot 7px, gap 5px, max-width 160px
        let pill_w = (6.0_f32 + 7.0 + 5.0 + text_galley.size().x + badge_extra + 7.0)
            .ceil()
            .min(160.0);
        let (pill_rect, pill_resp) =
            ui.allocate_exact_size(Vec2::new(pill_w, PILL_HEIGHT), Sense::click());
        pill_resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &pill_tip)
        });
        let pill_resp = pill_resp.on_hover_text(&pill_tip);

        let bg = if self.pill_popover_open || pill_resp.hovered() {
            dot_color.gamma_multiply(0.18_f32)
        } else {
            dot_color.gamma_multiply(0.11_f32)
        };
        ui.painter().rect(
            pill_rect,
            CornerRadius::same(10),
            bg,
            Stroke::new(1.0_f32, dot_color.gamma_multiply(0.55_f32)),
            StrokeKind::Inside,
        );
        ui.painter().circle_filled(
            Pos2::new(pill_rect.min.x + 9.5_f32, pill_rect.center().y),
            3.5_f32,
            dot_color,
        );
        ui.painter().galley(
            Pos2::new(
                pill_rect.min.x + 18.0_f32,
                pill_rect.center().y - text_galley.size().y * 0.5_f32,
            ),
            text_galley,
            ui.visuals().strong_text_color(),
        );

        if waiting_count > 0 {
            let badge_rect = Rect::from_center_size(
                Pos2::new(pill_rect.max.x - 12.0_f32, pill_rect.center().y),
                Vec2::new(15.0_f32, 14.0_f32),
            );
            ui.painter()
                .rect_filled(badge_rect, CornerRadius::same(7), amber);
            ui.painter().text(
                badge_rect.center(),
                egui::Align2::CENTER_CENTER,
                waiting_count.to_string(),
                FontId::proportional(10.0_f32),
                Color32::BLACK,
            );
        }

        if pill_resp.clicked() {
            self.pill_popover_open = !self.pill_popover_open;
            self.pill_popover_just_opened = self.pill_popover_open;
        }

        if show_follow_btn {
            let follow_lbl = if french {
                "Suivre l'hôte"
            } else {
                "Follow host"
            };
            if ui
                .add(egui::Button::new(RichText::new(follow_lbl).size(11.0_f32)))
                .clicked()
            {
                self.follow_host = true;
            }
        }

        if self.pill_popover_open {
            let just_opened = std::mem::take(&mut self.pill_popover_just_opened);
            let anchor = Pos2::new(
                (pill_rect.right() - PILL_POPOVER_WIDTH).max(8.0_f32),
                pill_rect.bottom() + 6.0_f32,
            );
            let ctx = ui.ctx().clone();
            let area = egui::Area::new(Id::new("live-share-pill-popover"))
                .order(Order::Foreground)
                .fixed_pos(anchor)
                .constrain(true)
                .show(&ctx, |ui| {
                    Frame::popup(ui.style())
                        .inner_margin(Margin::same(12))
                        .corner_radius(CornerRadius::same(10))
                        .show(ui, |ui| {
                            ui.set_width(PILL_POPOVER_WIDTH - 24.0_f32);
                            if let Some(snap) = &host_snap {
                                self.render_host_pill_popover(
                                    ui,
                                    snap,
                                    all_projects,
                                    active_project,
                                    french,
                                    &mut ui_action,
                                );
                            } else if let Some(gs) = &guest_snap {
                                self.render_guest_pill_popover(ui, gs, french, &mut ui_action);
                            }
                        });
                });

            let esc = ctx.input(|i| i.key_pressed(Key::Escape));
            let outside = !just_opened
                && ctx.input(|i| {
                    i.pointer.any_pressed()
                        && i.pointer.interact_pos().is_some_and(|pos| {
                            !area.response.rect.contains(pos) && !pill_rect.contains(pos)
                        })
                });
            if esc || outside {
                self.pill_popover_open = false;
            }
        }

        ui_action
    }

    fn render_host_pill_popover(
        &mut self,
        ui: &mut Ui,
        snap: &HostSnapshot,
        all_projects: &[SharedProject],
        active_project: Option<u64>,
        french: bool,
        ui_action: &mut Option<ShareUiAction>,
    ) {
        ui.horizontal(|ui| {
            ui.strong("Live Share");
            let since = clock_hhmm(snap.since_ms);
            if !since.is_empty() {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.weak(if french {
                        format!("Depuis {since}")
                    } else {
                        format!("Since {since}")
                    });
                });
            }
        });
        ui.add_space(6.0_f32);

        if !snap.asks.is_empty() {
            ui.horizontal(|ui| {
                let n = snap.asks.len();
                let wait_txt = if french {
                    format!("{n} en attente de votre accord")
                } else {
                    format!("{n} waiting for your answer")
                };
                ui.colored_label(Color32::from_rgb(0xf5, 0x9e, 0x0b), wait_txt);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let let_in = if french {
                        "Faire entrer…"
                    } else {
                        "Let in…"
                    };
                    if ui.small_button(let_in).clicked() {
                        self.pill_popover_open = false;
                        *ui_action = Some(ShareUiAction::OpenDialog(DialogTab::Share));
                    }
                });
            });
            ui.separator();
        }

        if let Some(inv) = snap.invites.first() {
            ui.horizontal(|ui| {
                ui.monospace(short_code(&inv.code));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let lbl = if self.is_copied("pill-code") {
                        if french { "Copié" } else { "Copied" }
                    } else if french {
                        "Copier le code d'invitation"
                    } else {
                        "Copy invite code"
                    };
                    if ui.small_button(lbl).clicked() {
                        self.mark_copied(ui.ctx(), "pill-code", inv.code.clone());
                    }
                });
            });
            ui.add_space(4.0_f32);
        }

        ui.weak(if french {
            "Projets partagés"
        } else {
            "Shared projects"
        });
        ui.horizontal_wrapped(|ui| {
            for proj in &snap.shared_projects {
                ui.label(
                    RichText::new(&proj.name)
                        .size(11.5_f32)
                        .background_color(ui.visuals().faint_bg_color),
                );
            }
            if let Some(active_id) = active_project
                && !self.selected_projects.contains(&active_id)
                && all_projects.iter().any(|p| p.id == active_id)
            {
                let share_this = if french {
                    "Partager ce projet"
                } else {
                    "Share this project"
                };
                if ui.small_button(share_this).clicked() {
                    self.selected_projects.push(active_id);
                }
            }
        });

        let joined: Vec<&ConnectedGuest> = snap
            .guests
            .iter()
            .filter(|g| g.state == GuestConnectionState::Joined)
            .collect();
        ui.add_space(4.0_f32);
        if joined.is_empty() {
            ui.weak(if french {
                "Personne n'est connecté pour l'instant."
            } else {
                "Nobody is in right now."
            });
        } else {
            for g in joined {
                ui.horizontal(|ui| {
                    ui.strong(&g.name);
                    ui.weak("·");
                    ui.weak(g.rights.chip_text(french));
                });
            }
        }

        ui.separator();
        ui.horizontal(|ui| {
            let manage_lbl = if french { "Gérer…" } else { "Manage…" };
            if ui.button(manage_lbl).clicked() {
                self.pill_popover_open = false;
                *ui_action = Some(ShareUiAction::OpenDialog(DialogTab::Share));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let stop_lbl = if french {
                    "Arrêter le partage"
                } else {
                    "Stop Live Share"
                };
                if ui
                    .add(egui::Button::new(
                        RichText::new(stop_lbl).color(ui.visuals().error_fg_color),
                    ))
                    .clicked()
                {
                    self.stop_hosting();
                }
            });
        });
    }

    fn render_guest_pill_popover(
        &mut self,
        ui: &mut Ui,
        gs: &share::GuestSnapshot,
        french: bool,
        ui_action: &mut Option<ShareUiAction>,
    ) {
        ui.strong("Live Share");
        ui.add_space(4.0_f32);
        ui.label(gs.rights.guest_sentence(french));
        ui.add_space(4.0_f32);
        let follow_lbl = if french {
            "Suivre l'écran de l'hôte"
        } else {
            "Follow host's screen"
        };
        ui.checkbox(&mut self.follow_host, follow_lbl);
        ui.separator();
        ui.horizontal(|ui| {
            let open_lbl = if french { "Ouvrir…" } else { "Manage…" };
            if ui.button(open_lbl).clicked() {
                self.pill_popover_open = false;
                *ui_action = Some(ShareUiAction::OpenDialog(DialogTab::Join));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let leave_lbl = if french { "Quitter" } else { "Leave" };
                if ui
                    .add(egui::Button::new(
                        RichText::new(leave_lbl).color(ui.visuals().error_fg_color),
                    ))
                    .clicked()
                {
                    if let Some(mut sess) = self.guest_session.take() {
                        sess.leave();
                    }
                    self.pill_popover_open = false;
                }
            });
        });
    }

    // -----------------------------------------------------------------------
    // 2. Pane Header Presence (`LiveSharePresence.vue` parity)
    // -----------------------------------------------------------------------

    pub fn paint_pane_presence(ui: &mut Ui, presence: &[PaneGuestPresence], french: bool) {
        if presence.is_empty() {
            return;
        }
        let typing: Vec<&PaneGuestPresence> = presence.iter().filter(|p| p.typing).collect();
        let watching: Vec<&PaneGuestPresence> = presence
            .iter()
            .filter(|p| p.watching && !p.typing)
            .collect();

        if !typing.is_empty() {
            let names: Vec<&str> = typing.iter().map(|p| p.name.as_str()).collect();
            let tip = if typing.len() == 1 {
                if french {
                    format!("{} est en train d'écrire", names[0])
                } else {
                    format!("{} is typing", names[0])
                }
            } else if french {
                format!("{} sont en train d'écrire", names.join(", "))
            } else {
                format!("{} are typing", names.join(", "))
            };
            let badge = format!(
                "⌨ {}",
                typing
                    .iter()
                    .take(2)
                    .map(|p| p.initials.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            let resp = ui.add(
                egui::Label::new(
                    RichText::new(badge)
                        .size(10.5_f32)
                        .color(ui.visuals().selection.stroke.color)
                        .strong(),
                )
                .sense(Sense::hover()),
            );
            resp.on_hover_text(tip);
        }

        if !watching.is_empty() {
            let names: Vec<&str> = watching.iter().map(|p| p.name.as_str()).collect();
            let tip = if watching.len() == 1 {
                if french {
                    format!("{} regarde ce panneau", names[0])
                } else {
                    format!("{} is watching this pane", names[0])
                }
            } else if french {
                format!("{} regardent ce panneau", names.join(", "))
            } else {
                format!("{} are watching this pane", names.join(", "))
            };
            let mut badge = watching
                .iter()
                .take(3)
                .map(|p| p.initials.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            if watching.len() > 3 {
                badge.push_str(&format!(" +{}", watching.len() - 3));
            }
            let resp = ui.add(
                egui::Label::new(
                    RichText::new(badge)
                        .size(10.5_f32)
                        .color(ui.visuals().weak_text_color()),
                )
                .sense(Sense::hover()),
            );
            resp.on_hover_text(tip);
        }
    }

    // -----------------------------------------------------------------------
    // 3. `560px` Knock Approval Modal (`LiveShareAsk.vue` parity)
    // -----------------------------------------------------------------------

    /// Renders the non-dismissible `560px` `LiveShareAsk` knock modal when a guest is waiting.
    /// Blocks `Escape` and backdrop clicks, enforces the `900ms` arming delay, and gates
    /// `"Trust next time"` on `"I checked it with them: it is the same"`.
    pub fn show_ask_modal(
        &mut self,
        ctx: &egui::Context,
        all_projects: &[SharedProject],
        french: bool,
    ) -> bool {
        let Some(snap) = self.host.as_ref().map(Host::snapshot) else {
            self.ask_draft = None;
            return false;
        };
        let Some(first_ask) = snap.asks.first().cloned() else {
            self.ask_draft = None;
            return false;
        };
        let more_waiting = snap.asks.len().saturating_sub(1);

        // Consume Escape so underlying modals/views cannot close while a knock is on screen!
        ctx.input_mut(|i| {
            let _ = i.consume_key(egui::Modifiers::NONE, Key::Escape);
        });

        let current_egui_time = ctx.input(|i| i.time);
        let offered_sig: Vec<u64> = match &first_ask {
            LiveShareAsk::Join {
                offered_projects, ..
            } => offered_projects.clone(),
            LiveShareAsk::Open { project_id, .. } => vec![*project_id],
            LiveShareAsk::Close { pane_id, .. } => vec![*pane_id],
        };

        let reset_draft = self
            .ask_draft
            .as_ref()
            .is_none_or(|d| d.key != first_ask.key() || d.projects_sig != offered_sig);
        if reset_draft {
            let (selected_projects, rights) = match &first_ask {
                LiveShareAsk::Join {
                    offered_projects,
                    offered_rights,
                    ..
                } => {
                    let valid: Vec<u64> = offered_projects
                        .iter()
                        .copied()
                        .filter(|id| snap.shared_projects.iter().any(|p| p.id == *id))
                        .collect();
                    let fallback = if valid.is_empty() {
                        snap.shared_projects.iter().map(|p| p.id).collect()
                    } else {
                        valid
                    };
                    (fallback, *offered_rights)
                }
                _ => (Vec::new(), ShareRights::view_only()),
            };
            self.ask_draft = Some(AskDraftState {
                key: first_ask.key().to_owned(),
                projects_sig: offered_sig,
                opened_instant: Instant::now(),
                opened_egui_time: current_egui_time,
                code_checked: false,
                trust_next_time: false,
                always_open: false,
                trust_nudge: false,
                selected_projects,
                rights,
            });
        }

        let mut answer_to_send: Option<AskAnswer> = None;
        let viewport_w = ctx.content_rect().width();

        egui::Modal::new(Id::new("live-share-ask-modal")).show(ctx, |ui| {
            ui.ctx().move_to_top(ui.layer_id());
            ui.set_width((viewport_w - 40.0_f32).clamp(280.0_f32, ASK_MODAL_MAX_WIDTH));
            let Some(draft) = self.ask_draft.as_mut() else {
                return;
            };

            let egui_elapsed_ok = (current_egui_time - draft.opened_egui_time) >= 0.9_f64;
            let wall_elapsed_ok = draft.opened_instant.elapsed() >= ASK_ARM_DELAY;
            let armed = egui_elapsed_ok || wall_elapsed_ok;
            if !armed {
                ctx.request_repaint_after(Duration::from_millis(50));
            }

            match &first_ask {
                LiveShareAsk::Join {
                    key,
                    name,
                    code,
                    invite_label,
                    trustable,
                    ..
                } => {
                    ui.horizontal(|ui| {
                        icons::paint(
                            ui.painter(),
                            Rect::from_min_size(ui.cursor().min, Vec2::splat(18.0_f32)),
                            Icon::ScreenShare,
                            ui.visuals().selection.stroke.color,
                        );
                        ui.add_space(22.0_f32);
                        ui.heading(if french {
                            format!("{name} veut rejoindre votre session")
                        } else {
                            format!("{name} wants to join")
                        });
                    });
                    if !invite_label.is_empty() {
                        ui.weak(if french {
                            format!("Via l'invitation : {invite_label}")
                        } else {
                            format!("Through invitation: {invite_label}")
                        });
                    }
                    ui.add_space(8.0_f32);

                    // 6-digit SAS verification callout
                    Frame::new()
                        .fill(ui.visuals().faint_bg_color)
                        .stroke(Stroke::new(
                            1.0_f32,
                            if draft.trust_nudge && !draft.code_checked {
                                Color32::from_rgb(0xf5, 0x9e, 0x0b)
                            } else {
                                ui.visuals().widgets.noninteractive.bg_stroke.color
                            },
                        ))
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(Margin::same(10))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.strong(if french {
                                    "Code de vérification"
                                } else {
                                    "Verification code"
                                });
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(
                                        RichText::new(code)
                                            .monospace()
                                            .size(18.0_f32)
                                            .strong(),
                                    );
                                });
                            });
                            ui.weak(if french {
                                "Vérifiez avec cette personne que ses 6 chiffres sont identiques."
                            } else {
                                "Ask them to read their 6 digits aloud to make sure nobody is in between."
                            });
                            let check_lbl = if french {
                                "Je l'ai vérifié avec cette personne : c'est le même"
                            } else {
                                "I checked it with them: it is the same"
                            };
                            if ui.checkbox(&mut draft.code_checked, check_lbl).changed() {
                                if draft.code_checked {
                                    draft.trust_nudge = false;
                                } else {
                                    draft.trust_next_time = false;
                                }
                            }
                        });

                    ui.add_space(8.0_f32);
                    ui.strong(if french {
                        "Projets auxquels donner accès"
                    } else {
                        "Projects to let them into"
                    });
                    ui.horizontal_wrapped(|ui| {
                        for proj in &snap.shared_projects {
                            let proj_name = all_projects
                                .iter()
                                .find(|p| p.id == proj.id)
                                .map_or(proj.name.as_str(), |p| p.name.as_str());
                            let mut checked = draft.selected_projects.contains(&proj.id);
                            if ui.checkbox(&mut checked, proj_name).changed() {
                                if checked && !draft.selected_projects.contains(&proj.id) {
                                    draft.selected_projects.push(proj.id);
                                } else if !checked {
                                    draft.selected_projects.retain(|&id| id != proj.id);
                                }
                            }
                        }
                    });
                    if draft.selected_projects.is_empty() {
                        ui.colored_label(
                            ui.visuals().error_fg_color,
                            if french {
                                "Cochez au moins un projet."
                            } else {
                                "Tick at least one project."
                            },
                        );
                    }

                    ui.add_space(6.0_f32);
                    ScrollArea::vertical()
                        .id_salt("ask-rights-scroll")
                        .max_height(220.0_f32)
                        .show(ui, |ui| {
                            render_rights_grid(ui, &mut draft.rights, french);
                        });

                    if *trustable {
                        ui.add_space(6.0_f32);
                        let trust_lbl = if french {
                            "Faire confiance la prochaine fois (entrée sans frapper)"
                        } else {
                            "Trust next time (let them in without knocking)"
                        };
                        let mut want_trust = draft.trust_next_time;
                        let resp = ui.checkbox(&mut want_trust, trust_lbl);
                        if resp.changed() || resp.clicked() {
                            if draft.code_checked {
                                draft.trust_next_time = want_trust;
                                draft.trust_nudge = false;
                            } else {
                                draft.trust_next_time = false;
                                draft.trust_nudge = true;
                            }
                        }
                        if draft.trust_nudge && !draft.code_checked {
                            ui.colored_label(
                                Color32::from_rgb(0xf5, 0x9e, 0x0b),
                                if french {
                                    "Cochez d'abord « Je l'ai vérifié avec cette personne » ci-dessus."
                                } else {
                                    "Tick “I checked it with them” above first."
                                },
                            );
                        }
                    }

                    ui.separator();
                    ui.horizontal(|ui| {
                        if more_waiting > 0 {
                            ui.weak(if french {
                                format!("{more_waiting} autre(s) en attente après celle-ci")
                            } else {
                                format!("{more_waiting} more waiting after this one")
                            });
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let allow_lbl = if french { "Autoriser" } else { "Allow" };
                            let can_allow = armed && !draft.selected_projects.is_empty();
                            if ui
                                .add_enabled(can_allow, egui::Button::new(allow_lbl))
                                .clicked()
                            {
                                answer_to_send = Some(AskAnswer::Join {
                                    key: key.clone(),
                                    allow: true,
                                    projects: draft.selected_projects.clone(),
                                    rights: draft.rights,
                                    trust: draft.code_checked && draft.trust_next_time,
                                });
                            }
                            let deny_lbl = if french { "Refuser" } else { "Deny" };
                            if ui.button(deny_lbl).clicked() {
                                answer_to_send = Some(AskAnswer::Join {
                                    key: key.clone(),
                                    allow: false,
                                    projects: Vec::new(),
                                    rights: ShareRights::view_only(),
                                    trust: false,
                                });
                            }
                        });
                    });
                }
                LiveShareAsk::Open {
                    key,
                    name,
                    project_name,
                    agent_name,
                    ..
                } => {
                    let target = agent_name.as_deref().unwrap_or(if french {
                        "un terminal"
                    } else {
                        "a terminal"
                    });
                    ui.heading(if french {
                        format!("{name} veut ouvrir {target}")
                    } else {
                        format!("{name} wants to open {target}")
                    });
                    ui.label(if french {
                        format!("Dans le projet {project_name} sur votre ordinateur.")
                    } else {
                        format!("In project {project_name} on your computer.")
                    });
                    ui.add_space(6.0_f32);
                    let always_lbl = if french {
                        format!("Laisser {name} ouvrir des panneaux sans demander")
                    } else {
                        format!("Let {name} open panes without asking")
                    };
                    ui.checkbox(&mut draft.always_open, always_lbl);
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let allow_lbl = if french { "L'ouvrir" } else { "Open it" };
                            if ui
                                .add_enabled(armed, egui::Button::new(allow_lbl))
                                .clicked()
                            {
                                answer_to_send = Some(AskAnswer::Open {
                                    key: key.clone(),
                                    allow: true,
                                    always: draft.always_open,
                                });
                            }
                            let deny_lbl = if french { "Refuser" } else { "Deny" };
                            if ui.button(deny_lbl).clicked() {
                                answer_to_send = Some(AskAnswer::Open {
                                    key: key.clone(),
                                    allow: false,
                                    always: false,
                                });
                            }
                        });
                    });
                }
                LiveShareAsk::Close {
                    key,
                    name,
                    pane_name,
                    detail,
                    ..
                } => {
                    ui.heading(if french {
                        format!("{name} veut fermer {pane_name}")
                    } else {
                        format!("{name} wants to close {pane_name}")
                    });
                    ui.label(detail);
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let close_lbl = if french { "Le fermer" } else { "Close it" };
                            if ui
                                .add_enabled(
                                    armed,
                                    egui::Button::new(
                                        RichText::new(close_lbl)
                                            .color(ui.visuals().error_fg_color),
                                    ),
                                )
                                .clicked()
                            {
                                answer_to_send = Some(AskAnswer::Close {
                                    key: key.clone(),
                                    allow: true,
                                });
                            }
                            let keep_lbl = if french {
                                "Le garder ouvert"
                            } else {
                                "Keep it open"
                            };
                            if ui.button(keep_lbl).clicked() {
                                answer_to_send = Some(AskAnswer::Close {
                                    key: key.clone(),
                                    allow: false,
                                });
                            }
                        });
                    });
                }
            }
        });

        if let Some(ans) = answer_to_send {
            if let Some(host) = &self.host {
                host.answer_ask(ans);
            }
            self.ask_draft = None;
        }
        true
    }

    // -----------------------------------------------------------------------
    // 4. `720px` `LiveShareDialog` Modal (`LiveShareDialog.vue` parity)
    // -----------------------------------------------------------------------

    /// Renders the `720px` `LiveShareDialog` modal and returns `true` when it should close.
    pub fn show_dialog_modal(
        &mut self,
        ctx: &egui::Context,
        all_projects: &[SharedProject],
        french: bool,
    ) -> bool {
        let mut close_clicked = false;
        let viewport_w = ctx.content_rect().width();
        let max_scroll_h =
            (ctx.content_rect().height() * 0.86_f32 - 110.0_f32).clamp(220.0_f32, 620.0_f32);

        let pending_ask = self.has_pending_ask();
        let modal_resp = egui::Modal::new(Id::new("live-share-dialog")).show(ctx, |ui| {
            if pending_ask {
                ui.disable();
            }
            ui.set_width((viewport_w - 48.0_f32).clamp(300.0_f32, DIALOG_MAX_WIDTH));
            close_clicked = self.show_dialog_contents(ui, all_projects, french, max_scroll_h);
        });

        !pending_ask && (close_clicked || modal_resp.should_close())
    }

    pub fn show_dialog_contents(
        &mut self,
        ui: &mut Ui,
        all_projects: &[SharedProject],
        french: bool,
        max_scroll_h: f32,
    ) -> bool {
        if self.selected_projects.is_empty()
            && let Some(first) = all_projects.first()
        {
            self.selected_projects.push(first.id);
        }

        let mut close_requested = false;
        // Header: ScreenShare icon + "Live Share" + segmented Share | Join + Close ×
        ui.horizontal(|ui| {
            let icon_rect = Rect::from_min_size(ui.cursor().min, Vec2::splat(18.0_f32));
            icons::paint(
                ui.painter(),
                icon_rect,
                Icon::ScreenShare,
                ui.visuals().selection.stroke.color,
            );
            ui.add_space(22.0_f32);
            ui.heading("Live Share");
            ui.add_space(12.0_f32);

            let share_lbl = if self.hosting() {
                if french { "● Partager" } else { "● Share" }
            } else if french {
                "Partager"
            } else {
                "Share"
            };
            if ui
                .selectable_label(self.tab == DialogTab::Share, share_lbl)
                .clicked()
            {
                self.tab = DialogTab::Share;
            }

            let join_lbl = if self.joined_or_waiting() {
                if french { "● Rejoindre" } else { "● Join" }
            } else if french {
                "Rejoindre"
            } else {
                "Join"
            };
            if ui
                .selectable_label(self.tab == DialogTab::Join, join_lbl)
                .clicked()
            {
                self.tab = DialogTab::Join;
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let close_tip = if french { "Fermer" } else { "Close" };
                if share_icon_button(ui, Icon::Close, close_tip).clicked() {
                    close_requested = true;
                }
            });
        });
        ui.separator();

        ScrollArea::vertical()
            .id_salt("live-share-dialog-scroll")
            .max_height(max_scroll_h)
            .show(ui, |ui| {
                match self.tab {
                    DialogTab::Share => self.render_share_tab(ui, all_projects, french),
                    DialogTab::Join => self.render_join_tab(ui, french),
                }
                if !self.error.is_empty() {
                    ui.add_space(6.0_f32);
                    ui.colored_label(ui.visuals().error_fg_color, &self.error);
                }
            });

        close_requested
    }

    fn render_share_tab(&mut self, ui: &mut Ui, all_projects: &[SharedProject], french: bool) {
        let Some(snap) = self.host.as_ref().map(Host::snapshot) else {
            self.render_pre_share_view(ui, all_projects, french);
            return;
        };

        // `<LsStatus>` active banner
        self.render_status_banner(ui, &snap, french);
        ui.add_space(8.0_f32);

        // 4 Host Sub-Tabs (`Overview`, `People`, `Invitations`, `Settings`)
        let people_count = snap.guests.len() + snap.offline.len() + snap.trusted.len();
        let inv_count = snap.invites.len();
        ui.horizontal(|ui| {
            let ov_lbl = if french { "Aperçu" } else { "Overview" };
            if ui
                .selectable_label(self.host_sub_tab == HostSubTab::Overview, ov_lbl)
                .clicked()
            {
                self.host_sub_tab = HostSubTab::Overview;
            }

            let ppl_lbl = if french {
                format!("Personnes ({people_count})")
            } else {
                format!("People ({people_count})")
            };
            if ui
                .selectable_label(self.host_sub_tab == HostSubTab::People, ppl_lbl)
                .clicked()
            {
                self.host_sub_tab = HostSubTab::People;
            }

            let inv_lbl = format!("Invitations ({inv_count})");
            if ui
                .selectable_label(self.host_sub_tab == HostSubTab::Invitations, inv_lbl)
                .clicked()
            {
                self.host_sub_tab = HostSubTab::Invitations;
            }

            let set_lbl = if french { "Réglages" } else { "Settings" };
            if ui
                .selectable_label(self.host_sub_tab == HostSubTab::Settings, set_lbl)
                .clicked()
            {
                self.host_sub_tab = HostSubTab::Settings;
            }
        });
        ui.separator();

        match self.host_sub_tab {
            HostSubTab::Overview => self.render_host_overview(ui, &snap, all_projects, french),
            HostSubTab::People => self.render_host_people(ui, &snap, french),
            HostSubTab::Invitations => self.render_host_invitations(ui, &snap, french),
            HostSubTab::Settings => self.render_host_settings(ui, &snap, french),
        }
    }

    fn render_status_banner(&mut self, ui: &mut Ui, snap: &HostSnapshot, french: bool) {
        Frame::new()
            .fill(Color32::from_rgb(0xef, 0x44, 0x44).gamma_multiply(0.10_f32))
            .stroke(Stroke::new(
                1.0_f32,
                Color32::from_rgb(0xef, 0x44, 0x44).gamma_multiply(0.45_f32),
            ))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::same(10))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let n = snap.shared_projects.len();
                    let since = clock_hhmm(snap.since_ms);
                    let title = if french {
                        format!("Partage de {n} projet(s) depuis {since}")
                    } else {
                        format!("Sharing {n} project(s) since {since}")
                    };
                    ui.strong(title);
                    ui.weak("·");
                    ui.weak(if french {
                        "Reprend après redémarrage"
                    } else {
                        "Comes back after restarts"
                    });
                    if snap.settings.hide_ignored {
                        ui.weak("·");
                        ui.weak(if french {
                            "Fichiers secrets masqués"
                        } else {
                            "Secret files hidden"
                        });
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !self.confirm_stop_sharing {
                            let stop_lbl = if french {
                                "Arrêter le partage"
                            } else {
                                "Stop sharing"
                            };
                            if ui
                                .add(egui::Button::new(
                                    RichText::new(stop_lbl).color(ui.visuals().error_fg_color),
                                ))
                                .clicked()
                            {
                                self.confirm_stop_sharing = true;
                            }
                        }
                    });
                });

                if self.confirm_stop_sharing {
                    ui.separator();
                    ui.strong(if french {
                        "Arrêter Live Share ?"
                    } else {
                        "Stop Live Share?"
                    });
                    ui.weak(if french {
                        "Tous les invités seront déconnectés immédiatement et les codes actuels cesseront de fonctionner."
                    } else {
                        "Everyone in the session is disconnected right away and current invitation codes stop working."
                    });
                    ui.horizontal(|ui| {
                        let keep_lbl = if french {
                            "Continuer le partage"
                        } else {
                            "Keep sharing"
                        };
                        if ui.button(keep_lbl).clicked() {
                            self.confirm_stop_sharing = false;
                        }
                        let stop_lbl = if french {
                            "Arrêter le partage"
                        } else {
                            "Stop sharing"
                        };
                        if ui
                            .add(egui::Button::new(
                                RichText::new(stop_lbl).color(ui.visuals().error_fg_color),
                            ))
                            .clicked()
                        {
                            self.stop_hosting();
                        }
                    });
                }
            });
    }

    fn render_pre_share_view(&mut self, ui: &mut Ui, all_projects: &[SharedProject], french: bool) {
        ui.label(if french {
            "Partagez vos projets directement sur votre réseau local via TLS avec certificat épinglé et code de vérification à 6 chiffres."
        } else {
            "Share your projects directly over TLS with a pinned certificate and 6-digit verification code."
        });
        ui.add_space(8.0_f32);

        if all_projects.is_empty() {
            ui.weak(if french {
                "Ouvrez d'abord un dossier de projet pour démarrer un partage."
            } else {
                "Open a project folder first to start sharing."
            });
            return;
        }

        ui.strong(if french {
            "Projets à partager"
        } else {
            "Projects to share"
        });
        ui.horizontal_wrapped(|ui| {
            for proj in all_projects {
                let mut on = self.selected_projects.contains(&proj.id);
                if ui.checkbox(&mut on, &proj.name).changed() {
                    if on && !self.selected_projects.contains(&proj.id) {
                        self.selected_projects.push(proj.id);
                    } else if !on {
                        self.selected_projects.retain(|&id| id != proj.id);
                    }
                }
            }
        });

        ui.add_space(6.0_f32);
        ui.horizontal(|ui| {
            ui.label(if french {
                "Votre nom (visible par les invités)"
            } else {
                "Your name (guests see it)"
            });
            ui.add(
                egui::TextEdit::singleline(&mut self.host_name)
                    .desired_width(180.0_f32)
                    .char_limit(40),
            );
            ui.label(if french { "IP locale" } else { "LAN IP" });
            ui.add(
                egui::TextEdit::singleline(&mut self.bind_address)
                    .desired_width(130.0_f32)
                    .char_limit(45),
            );
        });

        ui.add_space(8.0_f32);
        Frame::new()
            .fill(ui.visuals().faint_bg_color)
            .stroke(Stroke::new(
                1.0_f32,
                ui.visuals().widgets.noninteractive.bg_stroke.color,
            ))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::same(10))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.strong(if french {
                        "Première invitation"
                    } else {
                        "First invitation"
                    });
                    render_rights_chip(ui, self.pre_share_invite.rights, french);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let toggle_lbl = if self.pre_share_invite.open {
                            if french { "Masquer" } else { "Hide" }
                        } else if french {
                            "Personnaliser…"
                        } else {
                            "Customize…"
                        };
                        if ui.small_button(toggle_lbl).clicked() {
                            self.pre_share_invite.open = !self.pre_share_invite.open;
                        }
                    });
                });

                if self.pre_share_invite.open {
                    ui.separator();
                    render_invite_editor_fields(
                        ui,
                        &mut self.pre_share_invite,
                        all_projects,
                        french,
                    );
                }
            });

        ui.add_space(10.0_f32);
        let start_lbl = if french {
            "Démarrer le partage"
        } else {
            "Start sharing"
        };
        if ui
            .add_enabled(
                !self.selected_projects.is_empty(),
                egui::Button::new(RichText::new(start_lbl).strong()),
            )
            .clicked()
        {
            self.start_hosting_with(all_projects);
        }
    }

    fn render_host_overview(
        &mut self,
        ui: &mut Ui,
        snap: &HostSnapshot,
        all_projects: &[SharedProject],
        french: bool,
    ) {
        if !snap.asks.is_empty() {
            Frame::new()
                .fill(Color32::from_rgb(0xf5, 0x9e, 0x0b).gamma_multiply(0.12_f32))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0xf5, 0x9e, 0x0b)))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.strong(if french {
                        "En attente de votre réponse"
                    } else {
                        "Waiting for your answer"
                    });
                    for ask in &snap.asks {
                        if let LiveShareAsk::Join {
                            key, name, code, ..
                        } = ask
                        {
                            ui.horizontal(|ui| {
                                ui.strong(name);
                                ui.monospace(code);
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let refuse = if french { "Refuser" } else { "Refuse" };
                                    if ui.small_button(refuse).clicked()
                                        && let Some(host) = &self.host
                                    {
                                        host.answer_ask(AskAnswer::Join {
                                            key: key.clone(),
                                            allow: false,
                                            projects: Vec::new(),
                                            rights: ShareRights::view_only(),
                                            trust: false,
                                        });
                                    }
                                });
                            });
                        }
                    }
                });
            ui.add_space(8.0_f32);
        }

        ui.strong(if french {
            "Projets que vous partagez"
        } else {
            "Projects you share"
        });
        ui.horizontal_wrapped(|ui| {
            let lock_last = self.selected_projects.len() <= 1;
            for proj in all_projects {
                let mut on = self.selected_projects.contains(&proj.id);
                let can_toggle = !on || !lock_last;
                if ui
                    .add_enabled(can_toggle, egui::Checkbox::new(&mut on, &proj.name))
                    .changed()
                {
                    if on && !self.selected_projects.contains(&proj.id) {
                        self.selected_projects.push(proj.id);
                    } else if !on && self.selected_projects.len() > 1 {
                        self.selected_projects.retain(|&id| id != proj.id);
                    }
                }
            }
        });

        ui.add_space(8.0_f32);
        ui.strong(if french {
            "Codes d'invitation"
        } else {
            "Invitation codes"
        });
        for inv in &snap.invites {
            ui.horizontal(|ui| {
                ui.label(inv.display_name(french));
                render_rights_chip(ui, inv.rights, french);
                ui.monospace(short_code(&inv.code));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let msg_key = format!("ov-msg-{}", inv.id);
                    let msg_lbl = if self.is_copied(&msg_key) {
                        if french { "Copié" } else { "Copied" }
                    } else if french {
                        "Copier le message"
                    } else {
                        "Copy message"
                    };
                    if ui.small_button(msg_lbl).clicked() {
                        let msg = format_invite_message(&snap.host_name, &inv.code, french);
                        self.mark_copied(ui.ctx(), msg_key, msg);
                    }

                    let code_key = format!("ov-code-{}", inv.id);
                    let code_lbl = if self.is_copied(&code_key) {
                        if french { "Copié" } else { "Copied" }
                    } else if french {
                        "Copier le code"
                    } else {
                        "Copy code"
                    };
                    if ui.small_button(code_lbl).clicked() {
                        self.mark_copied(ui.ctx(), code_key, inv.code.clone());
                    }
                });
            });
        }

        ui.add_space(8.0_f32);
        ui.strong(if french {
            "Connectés maintenant"
        } else {
            "In now"
        });
        let joined: Vec<&ConnectedGuest> = snap
            .guests
            .iter()
            .filter(|g| g.state == GuestConnectionState::Joined)
            .collect();
        if joined.is_empty() {
            ui.weak(if french {
                "Aucun invité connecté."
            } else {
                "No guests connected yet."
            });
        } else {
            for g in joined {
                ui.horizontal(|ui| {
                    ui.strong(&g.name);
                    render_rights_chip(ui, g.rights, french);
                    if g.trusted {
                        ui.weak(if french {
                            "· De confiance"
                        } else {
                            "· Trusted"
                        });
                    }
                });
            }
        }

        if !snap.recent_actions.is_empty() {
            ui.add_space(8.0_f32);
            ui.strong(if french {
                "Actions récentes des invités"
            } else {
                "Recent guest actions"
            });
            for act in snap.recent_actions.iter().take(8) {
                ui.horizontal_wrapped(|ui| {
                    ui.weak(clock_hhmm(act.at_ms));
                    ui.label(if french {
                        &act.summary_fr
                    } else {
                        &act.summary_en
                    });
                    if act.flagged {
                        ui.colored_label(
                            Color32::from_rgb(0xef, 0x44, 0x44),
                            if french {
                                "Instructions d'agent"
                            } else {
                                "Agent instructions"
                            },
                        );
                    }
                });
            }
        }
    }

    fn render_host_people(&mut self, ui: &mut Ui, snap: &HostSnapshot, french: bool) {
        let in_now_n = snap
            .guests
            .iter()
            .filter(|g| g.state == GuestConnectionState::Joined)
            .count();
        let waiting_n = snap
            .guests
            .iter()
            .filter(|g| g.state == GuestConnectionState::Waiting)
            .count();
        let offline_n = snap.offline.len();
        let trusted_n = snap.trusted.len();
        let all_n = in_now_n + waiting_n + offline_n + trusted_n;

        ui.horizontal_wrapped(|ui| {
            for (flt, lbl_en, lbl_fr, cnt) in [
                (PeopleFilter::All, "All", "Tous", all_n),
                (PeopleFilter::InNow, "In now", "En ligne", in_now_n),
                (PeopleFilter::Waiting, "Waiting", "En attente", waiting_n),
                (PeopleFilter::Offline, "Offline", "Hors ligne", offline_n),
                (PeopleFilter::Trusted, "Trusted", "De confiance", trusted_n),
            ] {
                let txt = format!("{} ({cnt})", if french { lbl_fr } else { lbl_en });
                if ui
                    .selectable_label(self.people_filter == flt, txt)
                    .clicked()
                {
                    self.people_filter = flt;
                }
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let grid_lbl = if french { "Grille" } else { "Grid" };
                if ui
                    .selectable_label(self.people_view == PeopleViewMode::Grid, grid_lbl)
                    .clicked()
                {
                    self.people_view = PeopleViewMode::Grid;
                }
                let list_lbl = if french { "Liste" } else { "List" };
                if ui
                    .selectable_label(self.people_view == PeopleViewMode::List, list_lbl)
                    .clicked()
                {
                    self.people_view = PeopleViewMode::List;
                }
            });
        });
        ui.add_space(6.0_f32);

        if self.people_view == PeopleViewMode::Grid {
            self.render_people_access_grid(ui, snap, french);
            return;
        }

        if all_n == 0 {
            ui.weak(if french {
                "Personne n'a encore rejoint ou frappé."
            } else {
                "Nobody has joined or knocked yet."
            });
            return;
        }

        // Connected & waiting guests
        for g in &snap.guests {
            let show = match self.people_filter {
                PeopleFilter::All => true,
                PeopleFilter::InNow => g.state == GuestConnectionState::Joined,
                PeopleFilter::Waiting => g.state == GuestConnectionState::Waiting,
                PeopleFilter::Trusted => g.trusted,
                PeopleFilter::Offline => false,
            };
            if !show {
                continue;
            }
            let expanded = self.expanded_person_id.as_deref() == Some(g.id.as_str());
            Frame::new()
                .fill(ui.visuals().faint_bg_color)
                .corner_radius(CornerRadius::same(6))
                .inner_margin(Margin::same(8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .selectable_label(
                                expanded,
                                format!("{} · {}", initials(&g.name), g.name),
                            )
                            .clicked()
                        {
                            self.expanded_person_id =
                                if expanded { None } else { Some(g.id.clone()) };
                        }
                        render_rights_chip(ui, g.rights, french);
                        ui.monospace(&g.code);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let rem_lbl = if french { "Retirer" } else { "Remove" };
                            if ui.small_button(rem_lbl).clicked()
                                && let Some(host) = &self.host
                            {
                                host.remove_guest(&g.id, false);
                            }
                            if !g.trusted && g.identity_hex.is_some() {
                                let tr_lbl = if french { "Faire confiance" } else { "Trust" };
                                if ui.small_button(tr_lbl).clicked()
                                    && let Some(host) = &self.host
                                    && let Err(e) = host.trust_guest(&g.id)
                                {
                                    self.error = e;
                                }
                            } else if let Some(tid) = &g.trust_id {
                                let untr_lbl = if french {
                                    "Ne plus faire confiance"
                                } else {
                                    "Stop trusting"
                                };
                                if ui.small_button(untr_lbl).clicked()
                                    && let Some(host) = &self.host
                                {
                                    host.untrust(tid);
                                }
                            }
                        });
                    });

                    if expanded {
                        ui.separator();
                        ui.horizontal_wrapped(|ui| {
                            for proj in &snap.shared_projects {
                                let mut has_proj = g.projects.contains(&proj.id);
                                let can_unchk = !has_proj || g.projects.len() > 1;
                                if ui
                                    .add_enabled(
                                        can_unchk,
                                        egui::Checkbox::new(&mut has_proj, &proj.name),
                                    )
                                    .changed()
                                    && let Some(host) = &self.host
                                {
                                    host.set_subject_project(&g.id, proj.id, has_proj);
                                }
                            }
                        });
                        let mut rts = g.rights;
                        if render_rights_grid(ui, &mut rts, french)
                            && let Some(host) = &self.host
                        {
                            host.set_subject_rights(&g.id, rts);
                        }
                    }
                });
            ui.add_space(4.0_f32);
        }

        // Trusted people not currently connected
        if matches!(
            self.people_filter,
            PeopleFilter::All | PeopleFilter::Trusted
        ) {
            for tp in &snap.trusted {
                if snap
                    .guests
                    .iter()
                    .any(|g| g.trust_id.as_deref() == Some(tp.id.as_str()))
                {
                    continue;
                }
                Frame::new()
                    .fill(ui.visuals().faint_bg_color)
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::same(8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.strong(&tp.name);
                            ui.weak(if french { "De confiance" } else { "Trusted" });
                            render_rights_chip(ui, tp.rights, french);
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let untr_lbl = if french {
                                    "Ne plus faire confiance"
                                } else {
                                    "Stop trusting"
                                };
                                if ui.small_button(untr_lbl).clicked()
                                    && let Some(host) = &self.host
                                {
                                    host.untrust(&tp.id);
                                }
                                let ren_lbl = if french { "Renommer" } else { "Rename" };
                                if ui.small_button(ren_lbl).clicked() {
                                    self.renaming_trust = Some((tp.id.clone(), tp.name.clone()));
                                }
                            });
                        });
                        if let Some((rid, draft_name)) = &mut self.renaming_trust
                            && rid == &tp.id
                        {
                            let mut commit = false;
                            let mut cancel = false;
                            ui.horizontal(|ui| {
                                ui.text_edit_singleline(draft_name);
                                if ui.small_button("OK").clicked() {
                                    commit = true;
                                }
                                if ui
                                    .small_button(if french { "Annuler" } else { "Cancel" })
                                    .clicked()
                                {
                                    cancel = true;
                                }
                            });
                            if commit {
                                if let Some(host) = &self.host {
                                    host.rename_trusted(&tp.id, draft_name);
                                }
                                self.renaming_trust = None;
                            } else if cancel {
                                self.renaming_trust = None;
                            }
                        }
                    });
                ui.add_space(4.0_f32);
            }
        }

        // Offline people
        if matches!(
            self.people_filter,
            PeopleFilter::All | PeopleFilter::Offline
        ) {
            for off in &snap.offline {
                ui.horizontal(|ui| {
                    ui.weak(&off.name);
                    ui.weak("·");
                    ui.weak(if french { "Hors ligne" } else { "Offline" });
                    render_rights_chip(ui, off.rights, french);
                });
            }
        }
    }

    fn render_people_access_grid(&mut self, ui: &mut Ui, snap: &HostSnapshot, french: bool) {
        egui::Grid::new("live-share-people-matrix")
            .striped(true)
            .spacing([8.0_f32, 6.0_f32])
            .show(ui, |ui| {
                ui.strong(if french { "Personne" } else { "Person" });
                for rid in RightId::ALL {
                    ui.strong(rid.short_label(french));
                }
                ui.end_row();

                for g in &snap.guests {
                    ui.label(&g.name);
                    for rid in RightId::ALL {
                        let mut on = g.rights.get(rid);
                        if ui.checkbox(&mut on, "").changed()
                            && let Some(host) = &self.host
                        {
                            let mut next = g.rights;
                            next.set(rid, on);
                            host.set_subject_rights(&g.id, next);
                        }
                    }
                    ui.end_row();
                }

                for off in &snap.offline {
                    ui.weak(&off.name);
                    for rid in RightId::ALL {
                        let mut on = off.rights.get(rid);
                        if ui.checkbox(&mut on, "").changed()
                            && let Some(host) = &self.host
                        {
                            let mut next = off.rights;
                            next.set(rid, on);
                            host.set_subject_rights(&off.id, next);
                        }
                    }
                    ui.end_row();
                }
            });
    }

    fn render_host_invitations(&mut self, ui: &mut Ui, snap: &HostSnapshot, french: bool) {
        ui.horizontal(|ui| {
            let new_lbl = if french {
                "Nouvelle invitation"
            } else {
                "New invitation"
            };
            if ui
                .add_enabled(
                    snap.invites.len() < MAX_INVITES && self.invite_editor.is_none(),
                    egui::Button::new(new_lbl),
                )
                .clicked()
            {
                self.invite_editor = Some(InviteDraft {
                    open: true,
                    projects: snap.shared_projects.iter().map(|p| p.id).collect(),
                    ..InviteDraft::default()
                });
            }
            ui.weak(format!("{}/{MAX_INVITES}", snap.invites.len()));
        });
        ui.add_space(6.0_f32);

        if let Some(mut editor) = self.invite_editor.take() {
            let mut keep_open = true;
            Frame::new()
                .fill(ui.visuals().faint_bg_color)
                .stroke(Stroke::new(1.0_f32, ui.visuals().selection.stroke.color))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    render_invite_editor_fields(ui, &mut editor, &snap.shared_projects, french);
                    ui.separator();
                    ui.horizontal(|ui| {
                        let save_lbl = if editor.editing_id.is_some() {
                            if french { "Enregistrer" } else { "Save" }
                        } else if french {
                            "Créer l'invitation"
                        } else {
                            "Create invitation"
                        };
                        if ui.button(save_lbl).clicked() {
                            let fallback: Vec<u64> =
                                snap.shared_projects.iter().map(|p| p.id).collect();
                            let spec = editor.to_spec(&fallback);
                            if let Some(host) = &self.host {
                                let res = if let Some(id) = &editor.editing_id {
                                    host.update_invite(id, spec)
                                } else {
                                    host.create_invite(spec).map(|_| ())
                                };
                                match res {
                                    Ok(()) => keep_open = false,
                                    Err(e) => self.error = e,
                                }
                            }
                        }
                        if ui
                            .button(if french { "Annuler" } else { "Cancel" })
                            .clicked()
                        {
                            keep_open = false;
                        }
                    });
                });
            if keep_open {
                self.invite_editor = Some(editor);
            }
            ui.add_space(8.0_f32);
        }

        let now_ms = now_epoch_ms();
        for inv in &snap.invites {
            let status = inv.status(now_ms);
            Frame::new()
                .fill(ui.visuals().faint_bg_color)
                .corner_radius(CornerRadius::same(8))
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.strong(inv.display_name(french));
                        let badge_color = match status {
                            InviteStatus::Live => Color32::from_rgb(0x10, 0xb9, 0x81),
                            InviteStatus::UsedUp | InviteStatus::Expired => {
                                ui.visuals().weak_text_color()
                            }
                            InviteStatus::Blocked => ui.visuals().error_fg_color,
                        };
                        ui.colored_label(badge_color, status.badge_label(french));
                        render_rights_chip(ui, inv.rights, french);
                        ui.weak("·");
                        ui.weak(inv.expiry_summary(now_ms, french));
                        ui.weak("·");
                        ui.weak(inv.uses_summary(french));
                    });

                    ui.horizontal_wrapped(|ui| {
                        ui.monospace(short_code(&inv.code));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let rev_lbl = if french { "Révoquer" } else { "Revoke" };
                            if ui
                                .add(egui::Button::new(
                                    RichText::new(rev_lbl)
                                        .size(11.5_f32)
                                        .color(ui.visuals().error_fg_color),
                                ))
                                .clicked()
                            {
                                self.confirm_revoke_invite = Some((inv.id.clone(), false));
                            }
                            let dup_lbl = if french { "Dupliquer" } else { "Duplicate" };
                            if ui.small_button(dup_lbl).clicked()
                                && let Some(host) = &self.host
                                && let Err(e) = host.duplicate_invite(&inv.id)
                            {
                                self.error = e;
                            }
                            let rot_lbl = if french { "Nouveau code" } else { "New code" };
                            if ui.small_button(rot_lbl).clicked()
                                && let Some(host) = &self.host
                                && let Err(e) = host.rotate_invite(&inv.id)
                            {
                                self.error = e;
                            }
                            let edit_lbl = if french { "Modifier" } else { "Edit" };
                            if ui.small_button(edit_lbl).clicked() {
                                self.invite_editor = Some(InviteDraft::from_invite(inv));
                            }
                            let msg_key = format!("inv-msg-{}", inv.id);
                            let msg_lbl = if self.is_copied(&msg_key) {
                                if french { "Copié" } else { "Copied" }
                            } else if french {
                                "Copier le message"
                            } else {
                                "Copy message"
                            };
                            if ui.small_button(msg_lbl).clicked() {
                                let msg = format_invite_message(&snap.host_name, &inv.code, french);
                                self.mark_copied(ui.ctx(), msg_key, msg);
                            }
                            let c_key = format!("inv-code-{}", inv.id);
                            let c_lbl = if self.is_copied(&c_key) {
                                if french { "Copié" } else { "Copied" }
                            } else if french {
                                "Copier le code"
                            } else {
                                "Copy code"
                            };
                            if ui.small_button(c_lbl).clicked() {
                                self.mark_copied(ui.ctx(), c_key, inv.code.clone());
                            }
                        });
                    });

                    if let Some((rev_id, remove_people)) = &mut self.confirm_revoke_invite
                        && rev_id == &inv.id
                    {
                        let mut do_revoke: Option<bool> = None;
                        let mut cancel_rev = false;
                        ui.separator();
                        let chk_lbl = if french {
                            "Déconnecter aussi les personnes entrées avec ce code"
                        } else {
                            "Also disconnect people who joined with this code"
                        };
                        ui.checkbox(remove_people, chk_lbl);
                        ui.horizontal(|ui| {
                            if ui
                                .add(egui::Button::new(
                                    RichText::new(if french {
                                        "Confirmer la révocation"
                                    } else {
                                        "Confirm revoke"
                                    })
                                    .color(ui.visuals().error_fg_color),
                                ))
                                .clicked()
                            {
                                do_revoke = Some(*remove_people);
                            }
                            if ui
                                .button(if french { "Annuler" } else { "Cancel" })
                                .clicked()
                            {
                                cancel_rev = true;
                            }
                        });
                        if let Some(rem) = do_revoke {
                            if let Some(host) = &self.host
                                && let Err(e) = host.revoke_invite(&inv.id, rem)
                            {
                                self.error = e;
                            }
                            self.confirm_revoke_invite = None;
                        } else if cancel_rev {
                            self.confirm_revoke_invite = None;
                        }
                    }
                });
            ui.add_space(6.0_f32);
        }
    }

    fn render_host_settings(&mut self, ui: &mut Ui, snap: &HostSnapshot, french: bool) {
        let mut hide_ignored = snap.settings.hide_ignored;
        let toggle_lbl = if french {
            "Masquer les fichiers ignorés et secrets aux invités"
        } else {
            "Hide ignored and secret files from guests"
        };
        if ui.checkbox(&mut hide_ignored, toggle_lbl).changed() {
            self.settings.hide_ignored = hide_ignored;
            if let Some(host) = &self.host {
                host.set_hide_ignored(hide_ignored);
            }
        }
        let learn_lbl = if french {
            "En savoir plus"
        } else {
            "Learn more"
        };
        if ui.small_button(learn_lbl).clicked() {
            self.settings_learn_more_open = !self.settings_learn_more_open;
        }
        if self.settings_learn_more_open {
            ui.weak(if french {
                "Les fichiers .env, clés privées (.pem, .key, id_rsa, id_ed25519), coffres de mots de passe et dossiers .git/.tessel sont protégés."
            } else {
                "Dotfiles, .env files, private keys (.pem, .key, id_rsa, id_ed25519), password vaults, and .git/.tessel directories stay protected."
            });
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(if french {
                "Un partage revient de lui-même"
            } else {
                "A share comes back by itself"
            });
            ui.weak("·");
            ui.strong(if french { "Toujours" } else { "Always" });
        });
        ui.weak(format!(
            "{}: {}",
            if french {
                "Écoute TLS"
            } else {
                "TLS endpoint"
            },
            snap.endpoint
        ));
    }

    // -----------------------------------------------------------------------
    // 5. Guest `Join` Tab (`LiveShareDialog.vue` Join view)
    // -----------------------------------------------------------------------

    fn render_join_tab(&mut self, ui: &mut Ui, french: bool) {
        let guest_snap = self.guest_session.as_ref().map(GuestSession::snapshot);

        match guest_snap.as_ref().map(|g| &g.phase) {
            None | Some(GuestPhase::Ended { .. }) => {
                if let Some(GuestSnapshotPhaseBanner { text }) = guest_snap
                    .as_ref()
                    .and_then(|g| ended_banner(&g.phase, french))
                {
                    ui.colored_label(Color32::from_rgb(0xf5, 0x9e, 0x0b), text);
                    ui.add_space(6.0_f32);
                }

                ui.label(if french {
                    "Collez un code d'invitation privé pour rejoindre une session en direct."
                } else {
                    "Paste a private invitation code to join a Live Share session."
                });
                ui.add_space(6.0_f32);
                ui.horizontal(|ui| {
                    ui.label(if french { "Votre nom" } else { "Your name" });
                    ui.add(
                        egui::TextEdit::singleline(&mut self.guest_name)
                            .desired_width(200.0_f32)
                            .char_limit(40),
                    );
                });
                ui.add_space(4.0_f32);
                ui.add(
                    egui::TextEdit::multiline(&mut self.join_code)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY)
                        .hint_text("tessel-rust-share:…"),
                );
                ui.add_space(6.0_f32);
                let join_lbl = if french { "Rejoindre" } else { "Join" };
                if ui
                    .add_enabled(
                        !self.join_code.trim().is_empty(),
                        egui::Button::new(RichText::new(join_lbl).strong()),
                    )
                    .clicked()
                {
                    match Invitation::decode(&self.join_code) {
                        Ok(inv) => {
                            let sess = GuestSession::connect(
                                inv,
                                self.guest_name.clone(),
                                self.guest_identity_hex.clone(),
                            );
                            self.guest_session = Some(sess);
                            self.join_code.clear();
                            self.error.clear();
                        }
                        Err(e) => self.error = e,
                    }
                }
            }
            Some(GuestPhase::Connecting) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(if french {
                        "Connexion TLS à l'hôte…"
                    } else {
                        "Connecting over TLS…"
                    });
                });
            }
            Some(GuestPhase::WaitingForHost {
                sas_code,
                host_name,
            }) => {
                Frame::new()
                    .fill(ui.visuals().faint_bg_color)
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.strong(if french {
                                format!("En attente de la réponse de {host_name}…")
                            } else {
                                format!("Waiting for {host_name}'s answer…")
                            });
                        });
                        ui.add_space(6.0_f32);
                        ui.label(if french {
                            format!(
                                "Votre code est {sas_code}. Vérifiez avec l'hôte qu'il voit les mêmes 6 chiffres."
                            )
                        } else {
                            format!(
                                "Your code is {sas_code}. Check with the host that they see the same 6 digits."
                            )
                        });
                        ui.add_space(6.0_f32);
                        if ui
                            .button(if french { "Annuler" } else { "Cancel" })
                            .clicked()
                            && let Some(mut sess) = self.guest_session.take()
                        {
                            sess.leave();
                        }
                    });
            }
            Some(GuestPhase::Joined {
                sas_code,
                host_name,
            }) => {
                let gs = guest_snap.as_ref().expect("joined snapshot");
                self.render_joined_guest_workspace(ui, gs, host_name, sas_code, french);
            }
        }
    }

    fn render_joined_guest_workspace(
        &mut self,
        ui: &mut Ui,
        gs: &share::GuestSnapshot,
        host_name: &str,
        sas_code: &str,
        french: bool,
    ) {
        if self.follow_host {
            if let Some(hp) = gs.host_active_project {
                self.guest_selected_project = Some(hp);
            }
            if let Some(hpane) = gs.host_active_pane {
                self.guest_selected_pane = Some(hpane);
            }
        }
        if self.guest_selected_project.is_none() {
            self.guest_selected_project = gs.projects.first().map(|(id, _)| *id);
        }

        ui.horizontal_wrapped(|ui| {
            ui.strong(if french {
                format!("Connecté avec {host_name}")
            } else {
                format!("Connected with {host_name}")
            });
            render_rights_chip(ui, gs.rights, french);
            ui.monospace(sas_code);
            let follow_lbl = if french {
                format!("Suivre l'écran de {host_name}")
            } else {
                format!("Follow {host_name}'s screen")
            };
            ui.checkbox(&mut self.follow_host, follow_lbl);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(egui::Button::new(
                        RichText::new(if french { "Quitter" } else { "Leave" })
                            .color(ui.visuals().error_fg_color),
                    ))
                    .clicked()
                    && let Some(mut sess) = self.guest_session.take()
                {
                    sess.leave();
                }
            });
        });
        ui.weak(gs.rights.guest_sentence(french));

        if !gs.last_error.is_empty() {
            ui.colored_label(ui.visuals().error_fg_color, &gs.last_error);
        }

        ui.separator();
        ui.horizontal(|ui| {
            for (sv, en, fr) in [
                (GuestSubView::Panes, "Panes", "Panneaux"),
                (GuestSubView::Files, "Files", "Fichiers"),
                (GuestSubView::Git, "Changes", "Modifications"),
                (GuestSubView::Tasks, "Tasks", "Tâches"),
                (GuestSubView::Pages, "Pages", "Pages"),
            ] {
                if sv == GuestSubView::Pages && !gs.rights.pages_view {
                    continue;
                }
                if ui
                    .selectable_label(self.guest_sub_view == sv, if french { fr } else { en })
                    .clicked()
                {
                    self.guest_sub_view = sv;
                }
            }
        });
        ui.add_space(4.0_f32);

        let Some(sess) = &self.guest_session else {
            return;
        };

        match self.guest_sub_view {
            GuestSubView::Panes => {
                ui.horizontal_wrapped(|ui| {
                    for (pane_id, _proj_id, title, _is_agent) in &gs.panes {
                        if ui
                            .selectable_label(self.guest_selected_pane == Some(*pane_id), title)
                            .clicked()
                        {
                            self.follow_host = false;
                            self.guest_selected_pane = Some(*pane_id);
                            sess.send_async(json!({"op": "term.output", "paneId": pane_id}));
                        }
                    }
                    let open_lbl = if gs.rights.open {
                        if french {
                            "+ Ouvrir un terminal"
                        } else {
                            "+ Open terminal"
                        }
                    } else if french {
                        "+ Demander un terminal"
                    } else {
                        "+ Ask to open terminal"
                    };
                    if ui.small_button(open_lbl).clicked() {
                        sess.send_async(json!({
                            "op": "term.open",
                            "projectId": self.guest_selected_project
                        }));
                    }
                });

                if let Some((pid, _title, output)) = &gs.last_pane_output {
                    ScrollArea::vertical()
                        .id_salt("guest-pane-output")
                        .max_height(180.0_f32)
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(RichText::new(output).monospace())
                                    .selectable(true),
                            );
                        });
                    if gs.rights.r#type {
                        ui.horizontal(|ui| {
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut self.guest_term_input)
                                    .desired_width(ui.available_width() - 80.0_f32)
                                    .hint_text(if french {
                                        "Envoyer une commande…"
                                    } else {
                                        "Send command…"
                                    }),
                            );
                            let enter =
                                resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                            if (enter
                                || ui.button(if french { "Envoyer" } else { "Send" }).clicked())
                                && !self.guest_term_input.is_empty()
                            {
                                let mut bytes = self.guest_term_input.as_bytes().to_vec();
                                bytes.push(b'\r');
                                sess.send_async(json!({
                                    "op": "term.write",
                                    "paneId": pid,
                                    "data": B64.encode(bytes),
                                }));
                                self.guest_term_input.clear();
                            }
                        });
                    }
                }
            }
            GuestSubView::Files => {
                ui.horizontal(|ui| {
                    if !gs.current_path.is_empty()
                        && ui
                            .small_button(if french {
                                ".. Dossier parent"
                            } else {
                                ".. Parent"
                            })
                            .clicked()
                    {
                        let parent = Path::new(&gs.current_path)
                            .parent()
                            .map(|p| p.to_string_lossy().replace('\\', "/"))
                            .unwrap_or_default();
                        sess.send_async(json!({
                            "op": "files.list",
                            "projectId": self.guest_selected_project,
                            "path": parent,
                        }));
                    }
                    if ui
                        .small_button(if french { "Actualiser" } else { "Refresh" })
                        .clicked()
                    {
                        sess.send_async(json!({
                            "op": "files.list",
                            "projectId": self.guest_selected_project,
                            "path": gs.current_path,
                        }));
                    }
                });

                for entry in &gs.entries {
                    let is_dir = entry["directory"].as_bool().unwrap_or(false);
                    let name = entry["name"].as_str().unwrap_or("");
                    let path = entry["path"].as_str().unwrap_or("");
                    if ui
                        .button(format!("{} {name}", if is_dir { "▸" } else { "·" }))
                        .clicked()
                    {
                        if is_dir {
                            sess.send_async(json!({
                                "op": "files.list",
                                "projectId": self.guest_selected_project,
                                "path": path,
                            }));
                        } else {
                            self.guest_editor_path = Some(path.to_owned());
                            sess.send_async(json!({
                                "op": "files.read",
                                "projectId": self.guest_selected_project,
                                "path": path,
                            }));
                        }
                    }
                }

                if let Some((name, bytes)) = &gs.downloaded_file
                    && let Ok(text) = std::str::from_utf8(bytes)
                {
                    ui.separator();
                    ui.strong(name);
                    if gs.rights.edit {
                        if self.guest_editor_text.is_empty() {
                            self.guest_editor_text = text.to_owned();
                        }
                        ui.add(
                            egui::TextEdit::multiline(&mut self.guest_editor_text)
                                .code_editor()
                                .desired_rows(6)
                                .desired_width(f32::INFINITY),
                        );
                        if let Some(rel_path) = &self.guest_editor_path
                            && ui
                                .button(if french { "Enregistrer" } else { "Save" })
                                .clicked()
                        {
                            sess.send_async(json!({
                                "op": "files.write",
                                "projectId": self.guest_selected_project,
                                "path": rel_path,
                                "data": B64.encode(self.guest_editor_text.as_bytes()),
                            }));
                        }
                    } else {
                        ui.add(egui::Label::new(RichText::new(text).monospace()).selectable(true));
                    }
                }
            }
            GuestSubView::Git => {
                if ui
                    .small_button(if french {
                        "Actualiser git status"
                    } else {
                        "Refresh git status"
                    })
                    .clicked()
                    && let Ok(val) = sess.call_sync(
                        json!({"op": "scm.status", "projectId": self.guest_selected_project}),
                        Duration::from_secs(3),
                    )
                {
                    self.guest_git_status = val["status"].as_str().unwrap_or_default().to_owned();
                }
                if !self.guest_git_status.is_empty() {
                    ui.monospace(&self.guest_git_status);
                }
                if gs.rights.git {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.guest_commit_msg)
                                .desired_width(240.0_f32)
                                .hint_text(if french {
                                    "Message de commit…"
                                } else {
                                    "Commit message…"
                                }),
                        );
                        if ui.button("Commit").clicked() && !self.guest_commit_msg.trim().is_empty()
                        {
                            sess.send_async(json!({
                                "op": "scm.commit",
                                "projectId": self.guest_selected_project,
                                "message": self.guest_commit_msg.trim(),
                            }));
                            self.guest_commit_msg.clear();
                        }
                    });
                }
            }
            GuestSubView::Tasks => {
                if ui
                    .small_button(if french {
                        "Actualiser les tâches"
                    } else {
                        "Refresh tasks"
                    })
                    .clicked()
                {
                    match sess.call_sync(
                        json!({"op":"tasks.list", "projectId":self.guest_selected_project}),
                        Duration::from_secs(3),
                    ) {
                        Ok(val) => {
                            self.guest_tasks = val["tasks"].as_array().cloned().unwrap_or_default();
                            self.guest_tasks_limited = val["limited"].as_bool().unwrap_or(false);
                            self.guest_tasks_error.clear();
                        }
                        Err(e) => {
                            self.guest_tasks.clear();
                            self.guest_tasks_error = e;
                        }
                    }
                }
                if self.guest_tasks_limited {
                    ui.label(if french {
                        "Affichage partiel : le tableau dépasse la limite de partage."
                    } else {
                        "Partial view: the board exceeds the sharing limit."
                    });
                }
                for t in &self.guest_tasks {
                    ui.label(format!(
                        "[{}] {}",
                        t["column"].as_str().unwrap_or("todo"),
                        t["title"].as_str().unwrap_or("")
                    ));
                }
                if gs.rights.tasks {
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.guest_new_task_title).char_limit(300)
                            .desired_width(220.0).hint_text(if french { "Nouvelle tâche…" } else { "New task…" }));
                        if ui.button(if french { "Ajouter" } else { "Add" }).clicked() && !self.guest_new_task_title.trim().is_empty() {
                            match sess.call_sync(json!({"op":"tasks.mutate", "projectId":self.guest_selected_project,
                                "action":"add", "title":self.guest_new_task_title.trim(), "column":"todo"}), Duration::from_secs(3)) {
                                Ok(val) => {
                                    self.guest_tasks = val["tasks"].as_array().cloned().unwrap_or_default();
                                    self.guest_tasks_limited = val["limited"].as_bool().unwrap_or(false);
                                    self.guest_new_task_title.clear();
                                    self.guest_tasks_error.clear();
                                }
                                Err(e) => { self.guest_tasks_error = e; }
                            }
                        }
                    });
                }
                if !self.guest_tasks_error.is_empty() {
                    ui.colored_label(egui::Color32::LIGHT_RED, &self.guest_tasks_error);
                }
            }
            GuestSubView::Pages => {
                for pg in &gs.pages {
                    ui.horizontal(|ui| {
                        ui.strong(&pg.title);
                        if ui.link(&pg.url).clicked() {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(&pg.url));
                        }
                    });
                }
            }
        }
    }

    /// Legacy compatibility shim for single-project `Ui` embedding.
    pub fn show(&mut self, ui: &mut Ui, project: Option<&Path>) {
        let projects: Vec<SharedProject> = project
            .map(|p| {
                vec![SharedProject {
                    id: 1,
                    name: p
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "Project".into()),
                    path: p.to_path_buf(),
                }]
            })
            .unwrap_or_default();
        let _ = self.show_dialog_contents(ui, &projects, false, 520.0_f32);
        let _ = Write::flush(&mut std::io::sink());
    }
}

struct GuestSnapshotPhaseBanner {
    text: String,
}

fn ended_banner(phase: &GuestPhase, french: bool) -> Option<GuestSnapshotPhaseBanner> {
    let GuestPhase::Ended { reason, host_name } = phase else {
        return None;
    };
    let text = match (reason.as_str(), french) {
        ("denied", false) => "The host declined your request to join.".to_owned(),
        ("denied", true) => "L'hôte a refusé votre demande.".to_owned(),
        ("removed", false) => "The host removed you from the session.".to_owned(),
        ("removed", true) => "L'hôte vous a retiré de la session.".to_owned(),
        ("stopped", false) => format!("{host_name} stopped Live Share."),
        ("stopped", true) => format!("{host_name} a arrêté Live Share."),
        ("refused", false) => "This invitation code is invalid, expired, or revoked.".to_owned(),
        ("refused", true) => "Ce code d'invitation est invalide, expiré ou révoqué.".to_owned(),
        ("left", _) => return None,
        (_, false) => "Live Share connection ended.".to_owned(),
        (_, true) => "La connexion Live Share s'est terminée.".to_owned(),
    };
    Some(GuestSnapshotPhaseBanner { text })
}

fn format_invite_message(host_name: &str, code: &str, french: bool) -> String {
    if french {
        format!("Rejoignez la session Live Share de {host_name} dans Tessel :\n{code}")
    } else {
        format!("Join {host_name}'s Live Share session in Tessel:\n{code}")
    }
}

pub fn render_rights_chip(ui: &mut Ui, rights: ShareRights, french: bool) {
    let txt = rights.chip_text(french);
    let color = if rights.r#type {
        Color32::from_rgb(0xef, 0x44, 0x44)
    } else if rights.enabled_count() > 0 {
        ui.visuals().selection.stroke.color
    } else {
        ui.visuals().weak_text_color()
    };
    ui.label(
        RichText::new(txt)
            .size(11.0_f32)
            .color(color)
            .background_color(color.gamma_multiply(0.12_f32)),
    );
}

/// Renders the 4 presets (`View only`, `Collaborate`, `Full control`, `Custom`) + 8-tile
/// `<RightsGrid>` (`RightsGrid.vue` parity), including the red danger `Type` tile with its
/// `"Runs commands as you"` tag and typing warning callout. Returns `true` if `rights` changed.
pub fn render_rights_grid(ui: &mut Ui, rights: &mut ShareRights, french: bool) -> bool {
    let before = *rights;
    let current_preset = rights.preset();

    ui.horizontal_wrapped(|ui| {
        for preset in RightsPreset::ALL {
            let selected = current_preset == preset;
            let enabled = preset != RightsPreset::Custom || selected;
            if ui
                .add_enabled(
                    enabled,
                    egui::Button::selectable(selected, preset.label(french)),
                )
                .clicked()
                && preset != RightsPreset::Custom
            {
                *rights = ShareRights::from_preset(preset);
            }
        }
    });
    ui.weak(rights.preset().hint(*rights, french));
    let danger_red = Color32::from_rgb(0xef, 0x44, 0x44);
    if rights.r#type {
        ui.colored_label(
            danger_red,
            if french {
                "Attention : le droit Taper permet d'exécuter n'importe quelle commande sur ce PC en votre nom."
            } else {
                "Warning: Type permission lets guests run any command on this PC as you."
            },
        );
    }
    ui.add_space(4.0_f32);

    for rid in RightId::ALL {
        let mut on = rights.get(rid);
        let is_danger = rid.is_danger();
        let stroke = if is_danger && on {
            Stroke::new(1.0_f32, danger_red)
        } else if is_danger {
            Stroke::new(1.0_f32, danger_red.gamma_multiply(0.45_f32))
        } else {
            Stroke::new(1.0_f32, ui.visuals().widgets.noninteractive.bg_stroke.color)
        };
        let fill = if is_danger && on {
            danger_red.gamma_multiply(0.12_f32)
        } else {
            ui.visuals().faint_bg_color
        };

        Frame::new()
            .fill(fill)
            .stroke(stroke)
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin::symmetric(8, 6))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.checkbox(&mut on, rid.full_label(french)).changed() {
                        rights.set(rid, on);
                    }
                    if is_danger {
                        let tag = if french {
                            "Lance des commandes en votre nom"
                        } else {
                            "Runs commands as you"
                        };
                        ui.colored_label(danger_red, RichText::new(tag).size(10.5_f32).strong());
                    }
                });
                ui.weak(RichText::new(rid.hint(french)).size(11.0_f32));
            });
        ui.add_space(3.0_f32);
    }

    let _ = HashSet::<u64>::new();
    *rights != before
}

fn render_invite_editor_fields(
    ui: &mut Ui,
    draft: &mut InviteDraft,
    projects: &[SharedProject],
    french: bool,
) {
    ui.horizontal(|ui| {
        ui.label(if french { "Nom" } else { "Label" });
        ui.add(
            egui::TextEdit::singleline(&mut draft.label)
                .desired_width(200.0_f32)
                .char_limit(48)
                .hint_text(if french {
                    "ex. Équipe design"
                } else {
                    "e.g. Design review"
                }),
        );
    });

    if projects.len() > 1 {
        ui.horizontal_wrapped(|ui| {
            for proj in projects {
                let mut on = draft.projects.is_empty() || draft.projects.contains(&proj.id);
                if ui.checkbox(&mut on, &proj.name).changed() {
                    if draft.projects.is_empty() {
                        draft.projects = projects.iter().map(|p| p.id).collect();
                    }
                    if on && !draft.projects.contains(&proj.id) {
                        draft.projects.push(proj.id);
                    } else if !on && draft.projects.len() > 1 {
                        draft.projects.retain(|&id| id != proj.id);
                    }
                }
            }
        });
    }

    let later_lbl = if french {
        "Inclure les projets ajoutés plus tard"
    } else {
        "Include projects added later"
    };
    ui.checkbox(&mut draft.include_later, later_lbl);

    ui.horizontal_wrapped(|ui| {
        ui.label(if french { "Expiration :" } else { "Lifetime:" });
        for lt in InviteLifetime::ALL {
            if ui
                .selectable_label(draft.lifetime == lt, lt.label(french))
                .clicked()
            {
                draft.lifetime = lt;
            }
        }
        ui.label(if french {
            "Utilisations max :"
        } else {
            "Max uses:"
        });
        ui.add(
            egui::TextEdit::singleline(&mut draft.max_uses_text)
                .desired_width(56.0_f32)
                .hint_text("∞"),
        );
    });

    ui.add_space(4.0_f32);
    render_rights_grid(ui, &mut draft.rights, french);
}

fn share_icon_button(ui: &mut Ui, icon: Icon, tooltip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(26.0_f32), Sense::click());
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tooltip)
    });
    if resp.hovered() && ui.is_enabled() {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(6),
            ui.visuals().widgets.hovered.bg_fill,
        );
    }
    let color = if resp.hovered() && ui.is_enabled() {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    icons::paint(
        ui.painter(),
        Rect::from_center_size(rect.center(), Vec2::splat(15.0_f32)),
        icon,
        color,
    );
    resp.on_hover_text(tooltip)
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct ShareGpuiInviteRow {
    pub id: String,
    pub label: String,
    pub token_url: String,
    pub uses: u32,
    pub max_uses: u32,
    pub status: InviteStatus,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct ShareGpuiGuestRow {
    pub id: String,
    pub name: String,
    pub initials: String,
    pub joined: bool,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct ShareGpuiAskModal {
    pub ask_key: String,
    pub guest_name: String,
    pub sas_code: String,
    pub code_checked: bool,
    pub trust_next_time: bool,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct ShareGpuiSnapshot {
    pub tab: DialogTab,
    pub host_sub_tab: HostSubTab,
    pub hosting: bool,
    pub host_name: String,
    pub bind_address: String,
    pub selected_projects: Vec<u64>,
    pub pill_popover_open: bool,
    pub pill_label: Option<String>,
    pub pill_tooltip: Option<String>,
    pub pill_badge: Option<String>,
    pub pill_dot_live: bool,
    pub waiting_asks_count: usize,
    pub join_code: String,
    pub guest_name: String,
    pub guest_status: Option<String>,
    pub guest_active: bool,
    pub follow_host: bool,
    pub require_knock: bool,
    pub invites: Vec<ShareGpuiInviteRow>,
    pub guests: Vec<ShareGpuiGuestRow>,
    pub pending_ask: Option<ShareGpuiAskModal>,
    pub error: String,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) enum ShareGpuiAction {
    SetTab(DialogTab),
    SetHostSubTab(HostSubTab),
    ToggleProject(u64),
    SetHostName(String),
    SetBindAddress(String),
    ToggleRequireKnock,
    StartHosting,
    StopHosting,
    CreateInvite,
    RevokeInvite(String),
    KickGuest(String),
    SetJoinCode(String),
    SetGuestName(String),
    JoinSession,
    DisconnectGuest,
    ToggleFollowHost,
    TogglePillPopover,
    ClosePillPopover,
    ToggleAskCodeChecked,
    ToggleAskTrustNext,
    AnswerAsk { ask_key: String, allow: bool },
}

#[allow(dead_code)]
impl SharePanel {
    pub(crate) fn gpui_snapshot(&self, french: bool) -> ShareGpuiSnapshot {
        let host_snap = self.host.as_ref().map(Host::snapshot);
        let guest_snap = self.guest_session.as_ref().map(GuestSession::snapshot);
        let mut invites = Vec::new();
        let mut guests = Vec::new();
        let mut pending_ask = None;
        let mut waiting_asks_count = 0usize;
        let pill_state = self.pill_state();
        let pill_label = pill_state
            .as_ref()
            .map(|s| crate::toolbar_fit::live_share_pill_label(s, french));
        let pill_tooltip = pill_state
            .as_ref()
            .map(|s| crate::toolbar_fit::live_share_pill_tooltip(s, french));
        let pill_badge = pill_state
            .as_ref()
            .and_then(crate::toolbar_fit::live_share_pill_badge);
        let pill_dot_live = pill_state
            .as_ref()
            .is_some_and(crate::toolbar_fit::live_share_pill_dot_live);

        if let Some(snap) = &host_snap {
            let now_ms = now_epoch_ms();
            waiting_asks_count = snap.asks.len();
            for inv in &snap.invites {
                let token_url = inv.code.clone();
                invites.push(ShareGpuiInviteRow {
                    id: inv.id.clone(),
                    label: if inv.label.trim().is_empty() {
                        if french {
                            "Invitation".into()
                        } else {
                            "Invite link".into()
                        }
                    } else {
                        inv.label.clone()
                    },
                    token_url,
                    uses: inv.uses,
                    max_uses: inv.max_uses,
                    status: inv.status(now_ms),
                });
            }
            for g in &snap.guests {
                guests.push(ShareGpuiGuestRow {
                    id: g.id.clone(),
                    name: g.name.clone(),
                    initials: initials(&g.name),
                    joined: g.state == GuestConnectionState::Joined,
                });
            }
            if let Some(first_ask) = snap.asks.first() {
                let code_checked = self.ask_draft.as_ref().is_some_and(|d| d.code_checked);
                let trust_next_time = self.ask_draft.as_ref().is_some_and(|d| d.trust_next_time);
                let (ask_key, guest_name, sas_code) = match first_ask {
                    LiveShareAsk::Join {
                        key, name, code, ..
                    } => (key.clone(), name.clone(), code.clone()),
                    LiveShareAsk::Open { key, name, .. } => {
                        (key.clone(), name.clone(), String::new())
                    }
                    LiveShareAsk::Close { key, name, .. } => {
                        (key.clone(), name.clone(), String::new())
                    }
                };
                pending_ask = Some(ShareGpuiAskModal {
                    ask_key,
                    guest_name,
                    sas_code,
                    code_checked,
                    trust_next_time,
                });
            }
        }

        let mut guest_status = None;
        let mut guest_active = false;
        if let Some(gs) = &guest_snap {
            match &gs.phase {
                GuestPhase::Connecting => {
                    guest_active = true;
                    let s = if french {
                        "Connexion…".to_owned()
                    } else {
                        "Connecting…".to_owned()
                    };
                    guest_status = Some(s);
                }
                GuestPhase::WaitingForHost { sas_code, .. } => {
                    guest_active = true;
                    let s = if french {
                        format!("En attente de l'hôte · Code SAS : {sas_code}")
                    } else {
                        format!("Waiting for host · SAS code: {sas_code}")
                    };
                    guest_status = Some(s);
                }
                GuestPhase::Joined { host_name, .. } => {
                    guest_active = true;
                    let s = if french {
                        format!("En direct avec {host_name}")
                    } else {
                        format!("Live with {host_name}")
                    };
                    guest_status = Some(s);
                }
                GuestPhase::Ended { .. } => {
                    guest_status = ended_banner(&gs.phase, french).map(|b| b.text);
                }
            }
        }

        ShareGpuiSnapshot {
            tab: self.tab,
            host_sub_tab: self.host_sub_tab,
            hosting: self.hosting(),
            host_name: self.host_name.clone(),
            bind_address: self.bind_address.clone(),
            selected_projects: self.selected_projects.clone(),
            pill_popover_open: self.pill_popover_open,
            pill_label,
            pill_tooltip,
            pill_badge,
            pill_dot_live,
            waiting_asks_count,
            join_code: self.join_code.clone(),
            guest_name: self.guest_name.clone(),
            guest_status,
            guest_active,
            follow_host: self.follow_host,
            require_knock: self.settings.hide_ignored,
            invites,
            guests,
            pending_ask,
            error: self.error.clone(),
        }
    }

    pub(crate) fn apply_gpui_share_action(
        &mut self,
        action: ShareGpuiAction,
        all_projects: &[SharedProject],
    ) {
        match action {
            ShareGpuiAction::SetTab(t) => self.tab = t,
            ShareGpuiAction::SetHostSubTab(st) => self.host_sub_tab = st,
            ShareGpuiAction::ToggleProject(pid) => {
                if self.selected_projects.contains(&pid) {
                    if self.selected_projects.len() > 1 {
                        self.selected_projects.retain(|&id| id != pid);
                    }
                } else {
                    self.selected_projects.push(pid);
                }
            }
            ShareGpuiAction::SetHostName(n) => self.host_name = n,
            ShareGpuiAction::SetBindAddress(b) => self.bind_address = b,
            ShareGpuiAction::ToggleRequireKnock => {
                self.settings.hide_ignored = !self.settings.hide_ignored;
                if let Some(host) = &self.host {
                    host.set_hide_ignored(self.settings.hide_ignored);
                }
            }
            ShareGpuiAction::StartHosting => {
                if self.selected_projects.is_empty() {
                    self.selected_projects = all_projects.iter().map(|p| p.id).collect();
                }
                self.start_hosting_with(all_projects);
            }
            ShareGpuiAction::StopHosting => self.stop_hosting(),
            ShareGpuiAction::CreateInvite => {
                if let Some(host) = &self.host {
                    let ids: Vec<u64> = if self.selected_projects.is_empty() {
                        all_projects.iter().map(|p| p.id).collect()
                    } else {
                        self.selected_projects.clone()
                    };
                    let spec = self.pre_share_invite.to_spec(&ids);
                    if let Err(e) = host.create_invite(spec) {
                        self.error = e;
                    }
                }
            }
            ShareGpuiAction::RevokeInvite(inv_id) => {
                if let Some(host) = &self.host
                    && let Err(e) = host.revoke_invite(&inv_id, false)
                {
                    self.error = e;
                }
            }
            ShareGpuiAction::KickGuest(gid) => {
                if let Some(host) = &self.host {
                    host.remove_guest(&gid, false);
                }
            }
            ShareGpuiAction::SetJoinCode(code) => self.join_code = code,
            ShareGpuiAction::SetGuestName(name) => self.guest_name = name,
            ShareGpuiAction::JoinSession => match Invitation::decode(self.join_code.trim()) {
                Ok(inv) => {
                    let gs = GuestSession::connect(
                        inv,
                        self.guest_name.clone(),
                        self.guest_identity_hex.clone(),
                    );
                    self.guest_session = Some(gs);
                    self.join_code.clear();
                    self.error.clear();
                }
                Err(e) => self.error = e,
            },
            ShareGpuiAction::DisconnectGuest => {
                self.guest_session = None;
            }
            ShareGpuiAction::ToggleFollowHost => {
                self.follow_host = !self.follow_host;
            }
            ShareGpuiAction::TogglePillPopover => {
                self.pill_popover_open = !self.pill_popover_open;
            }
            ShareGpuiAction::ClosePillPopover => {
                self.close_pill_popover();
            }
            ShareGpuiAction::ToggleAskCodeChecked => {
                let draft = self.ask_draft.get_or_insert_with(|| AskDraftState {
                    key: String::new(),
                    projects_sig: Vec::new(),
                    opened_instant: Instant::now()
                        .checked_sub(ASK_ARM_DELAY)
                        .unwrap_or_else(Instant::now),
                    opened_egui_time: 0.0,
                    code_checked: false,
                    trust_next_time: false,
                    always_open: false,
                    trust_nudge: false,
                    selected_projects: self.selected_projects.clone(),
                    rights: ShareRights::view_only(),
                });
                draft.code_checked = !draft.code_checked;
            }
            ShareGpuiAction::ToggleAskTrustNext => {
                let draft = self.ask_draft.get_or_insert_with(|| AskDraftState {
                    key: String::new(),
                    projects_sig: Vec::new(),
                    opened_instant: Instant::now()
                        .checked_sub(ASK_ARM_DELAY)
                        .unwrap_or_else(Instant::now),
                    opened_egui_time: 0.0,
                    code_checked: false,
                    trust_next_time: false,
                    always_open: false,
                    trust_nudge: false,
                    selected_projects: self.selected_projects.clone(),
                    rights: ShareRights::view_only(),
                });
                draft.trust_next_time = !draft.trust_next_time;
            }
            ShareGpuiAction::AnswerAsk { ask_key, allow } => {
                if let Some(host) = &self.host {
                    let trust = self
                        .ask_draft
                        .as_ref()
                        .is_some_and(|d| d.code_checked && d.trust_next_time);
                    let projects = if self.selected_projects.is_empty() {
                        all_projects.iter().map(|p| p.id).collect()
                    } else {
                        self.selected_projects.clone()
                    };
                    let snap = host.snapshot();
                    let answer = match snap.asks.iter().find(|a| match a {
                        LiveShareAsk::Join { key, .. }
                        | LiveShareAsk::Open { key, .. }
                        | LiveShareAsk::Close { key, .. } => key == &ask_key,
                    }) {
                        Some(LiveShareAsk::Open { .. }) => AskAnswer::Open {
                            key: ask_key,
                            allow,
                            always: false,
                        },
                        Some(LiveShareAsk::Close { .. }) => AskAnswer::Close {
                            key: ask_key,
                            allow,
                        },
                        Some(LiveShareAsk::Join { .. }) | None => AskAnswer::Join {
                            key: ask_key,
                            allow,
                            projects: if allow { projects } else { Vec::new() },
                            rights: ShareRights::view_only(),
                            trust: allow && trust,
                        },
                    };
                    host.answer_ask(answer);
                    self.ask_draft = None;
                }
            }
        }
    }
}
