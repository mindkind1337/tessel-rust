//! Complete native Status Bar (`StatusBar.vue` + `PortRow.vue` + `RemoteHostsStatus.vue` parity).
//!
//! Left: active input / broadcast target, pane summary (`N panes · W working · A waiting for you`),
//! and project path (hidden below `900 px`).
//! Right: background agent updates, Keep Computer Awake (`On` / `Agent` / `Off`), Resource Manager
//! (`Memory · Terminals` + `416 px` popover with sparklines & sortable tree), Listening Ports
//! (`Workspace count` + `384 px` popover with Open in Browser / Copy / Stop & External Ports), and
//! Remote Hosts (`288 px` SSH status popover). Right-click opens the indicator visibility menu.

use std::collections::HashSet;

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Id, Key, Order, Painter, Pos2, Rect, Response,
    Sense, Shape, Stroke, StrokeKind, Vec2,
};

use crate::icons::{self, Icon};
use crate::sidebar_model::{
    APP_HISTORY_KEY, KeepAwakeMode, ResourceSortBy, SidebarLang, StatusBarDensity, TerminalMeta,
    awake_copy, format_cpu, format_memory, ports_summary, resource_tree, sparkline_lead,
    sparkline_points, status_bar_density,
};
use crate::status_metrics::{DetectedPort, TelemetryReport};
use crate::theme_palettes::UiPalette;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target<'a> {
    None,
    Input(&'a str),
    Broadcast(usize),
}

#[must_use]
pub fn format_target(target: Target<'_>, french: bool) -> String {
    match target {
        Target::None | Target::Input("") => String::new(),
        Target::Input(name) => {
            if french {
                format!("Saisie → {name}")
            } else {
                format!("Input → {name}")
            }
        }
        Target::Broadcast(count) => {
            crate::layout_broadcast_ui::format_broadcast_status_target(french, count)
        }
    }
}

#[must_use]
pub fn format_pane_summary(
    pane_count: usize,
    working_count: usize,
    waiting_count: usize,
    french: bool,
) -> String {
    let mut parts = Vec::with_capacity(3);
    parts.push(match (french, pane_count) {
        (true, 1) => "1 panneau".to_owned(),
        (true, n) => format!("{n} panneaux"),
        (false, 1) => "1 pane".to_owned(),
        (false, n) => format!("{n} panes"),
    });
    if working_count > 0 {
        parts.push(if french {
            format!("{working_count} au travail")
        } else {
            format!("{working_count} working")
        });
    }
    if waiting_count > 0 {
        parts.push(if french {
            format!("{waiting_count} vous attendent")
        } else {
            format!("{waiting_count} waiting for you")
        });
    }
    parts.join(" · ")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StatusBarItemId {
    Ssh,
    Resources,
    Ports,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteHostStatusKind {
    Connected,
    Connecting,
    AuthFailed,
    Reconnecting,
    ReconnectionFailed,
    Error,
    Disconnected,
}

impl RemoteHostStatusKind {
    #[must_use]
    pub const fn is_connecting(self) -> bool {
        matches!(self, Self::Connecting | Self::Reconnecting)
    }

    #[must_use]
    pub const fn can_connect(self) -> bool {
        matches!(
            self,
            Self::Disconnected | Self::AuthFailed | Self::ReconnectionFailed | Self::Error
        )
    }

    #[must_use]
    pub const fn label(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::Connected, false) => "Connected",
            (Self::Connected, true) => "Connecté",
            (Self::Connecting, false) => "Connecting…",
            (Self::Connecting, true) => "Connexion…",
            (Self::AuthFailed, false) => "Auth failed",
            (Self::AuthFailed, true) => "Échec de l'authentification",
            (Self::Reconnecting, false) => "Reconnecting…",
            (Self::Reconnecting, true) => "Reconnexion…",
            (Self::ReconnectionFailed, false) => "Reconnection failed",
            (Self::ReconnectionFailed, true) => "Échec de la reconnexion",
            (Self::Error, false) => "Error",
            (Self::Error, true) => "Erreur",
            (Self::Disconnected, false) => "Disconnected",
            (Self::Disconnected, true) => "Déconnecté",
        }
    }

    #[must_use]
    pub const fn connect_verb(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::AuthFailed, false) => "Reconnect",
            (Self::AuthFailed, true) => "Reconnecter",
            (Self::Error | Self::ReconnectionFailed, false) => "Retry",
            (Self::Error | Self::ReconnectionFailed, true) => "Réessayer",
            (_, false) => "Connect",
            (_, true) => "Se connecter",
        }
    }

    fn dot_color(self, ui_colors: &UiPalette) -> Color32 {
        match self {
            Self::Connected => Color32::from_rgb(0x10, 0xb9, 0x81),
            Self::Connecting | Self::Reconnecting => Color32::from_rgb(0xea, 0xb3, 0x08),
            Self::AuthFailed | Self::ReconnectionFailed | Self::Error => {
                Color32::from_rgb(0xef, 0x44, 0x44)
            }
            Self::Disconnected => ui_colors.text_dim.gamma_multiply(0.45),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteHostRowInput {
    pub alias: String,
    pub label: String,
    pub status: RemoteHostStatusKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatusBarAction {
    SetKeepAwake(KeepAwakeMode),
    ToggleItem(StatusBarItemId),
    FocusSession(String),
    ActivateWorkspace(String),
    OpenPortInBrowser { url: String, port: u16 },
    CopyPortAddress { address: String, port: u16 },
    StopPort(DetectedPort),
    RefreshPorts,
    ConnectRemoteHost(String),
    DisconnectRemoteHost(String),
    ManageRemoteHosts,
    OpenAgentUpdates,
    DismissError,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum OpenPopover {
    KeepAwake,
    Resources,
    Ports,
    RemoteHosts,
    Visibility(Pos2),
}

#[derive(Clone, Debug, Default)]
pub struct StatusBarState {
    open: Option<OpenPopover>,
    pub sort_by: ResourceSortBy,
    pub app_expanded: bool,
    pub external_expanded: bool,
    copied_port: Option<(u16, f64)>,
    awake_anchor: Option<Rect>,
    resources_anchor: Option<Rect>,
    ports_anchor: Option<Rect>,
    remote_anchor: Option<Rect>,
    bar_rect: Option<Rect>,
}

impl StatusBarState {
    #[must_use]
    pub const fn is_resources_open(&self) -> bool {
        matches!(self.open, Some(OpenPopover::Resources))
    }

    #[must_use]
    pub const fn is_ports_open(&self) -> bool {
        matches!(self.open, Some(OpenPopover::Ports))
    }

    #[must_use]
    pub const fn has_open_popover(&self) -> bool {
        self.open.is_some()
    }

    pub fn close_popovers(&mut self) {
        self.open = None;
    }

    pub fn open_resources(&mut self) {
        self.open = Some(OpenPopover::Resources);
    }

    pub fn open_ports(&mut self) {
        self.open = Some(OpenPopover::Ports);
    }
}

pub struct StatusBarInput<'a> {
    pub ui_colors: &'a UiPalette,
    pub french: bool,
    pub target: Target<'a>,
    pub pane_count: usize,
    pub working_count: usize,
    pub waiting_count: usize,
    pub project_path: &'a str,
    pub keep_awake_mode: KeepAwakeMode,
    pub keep_awake_active: bool,
    pub show_ssh: bool,
    pub show_resources: bool,
    pub show_ports: bool,
    pub terminals: &'a [TerminalMeta],
    pub bound_terminals: &'a HashSet<String>,
    pub telemetry: &'a TelemetryReport,
    pub remote_hosts: &'a [RemoteHostRowInput],
    pub agent_updating: &'a [String],
    pub error_banner: &'a str,
    pub persistence_unavailable: bool,
}

fn lang_of(french: bool) -> SidebarLang {
    if french {
        SidebarLang::Fr
    } else {
        SidebarLang::En
    }
}

fn truncated_text(
    ui: &mut egui::Ui,
    rect: Rect,
    value: &str,
    color: Color32,
    hover: &str,
) -> Response {
    let response = ui.allocate_rect(rect, Sense::hover());
    if !value.is_empty() && rect.width() > 2.0 {
        let mut job = egui::text::LayoutJob::simple(
            value.to_owned(),
            FontId::proportional(11.0),
            color,
            rect.width().max(0.0),
        );
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        job.wrap.overflow_character = Some('…');
        let galley = ui.painter().layout_job(job);
        ui.painter().with_clip_rect(rect).galley(
            egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
            galley,
            color,
        );
    }
    if hover.is_empty() {
        response
    } else {
        response.on_hover_text(hover)
    }
}

fn measure_text_w(painter: &Painter, text: &str, font: FontId, color: Color32) -> f32 {
    if text.is_empty() {
        0.0
    } else {
        painter
            .layout_no_wrap(text.to_owned(), font, color)
            .size()
            .x
            .ceil()
    }
}

fn resource_tooltip_text(
    snap: Option<&crate::sidebar_model::ResourceSnapshot>,
    terminal_count: usize,
    french: bool,
) -> String {
    let lang = lang_of(french);
    let mem_desc = if let Some(s) = snap {
        let ws = format_memory(Some(s.total_memory_bytes), lang);
        let priv_mem = format_memory(Some(s.total_private_bytes), lang);
        if french {
            if s.total_private_bytes > 0.0 {
                format!("{ws} mémoire · {priv_mem} privée")
            } else {
                format!("{ws} mémoire")
            }
        } else if s.total_private_bytes > 0.0 {
            format!("{ws} memory · {priv_mem} private")
        } else {
            format!("{ws} memory")
        }
    } else if french {
        "mémoire indisponible".to_owned()
    } else {
        "memory unavailable".to_owned()
    };

    let line1 = match (french, terminal_count) {
        (true, 1) => format!("Gestionnaire de ressources - {mem_desc} - 1 session de terminal"),
        (true, n) => format!("Gestionnaire de ressources - {mem_desc} - {n} sessions de terminal"),
        (false, 1) => format!("Resource Manager - {mem_desc} - 1 terminal session"),
        (false, n) => format!("Resource Manager - {mem_desc} - {n} terminal sessions"),
    };
    let line2 = match (french, terminal_count > 0) {
        (true, true) => "Les sessions de terminal sont regroupées par espace de travail.",
        (true, false) => "Aucune session de terminal pour l'instant.",
        (false, true) => "Terminal sessions are grouped by workspace.",
        (false, false) => "No terminal sessions yet.",
    };
    format!("{line1}\n{line2}")
}

fn ports_tooltip_text(ws_count: usize, ext_count: usize, french: bool) -> String {
    let main = match (french, ws_count) {
        (true, 1) => "Ports — 1 port d'espace de travail".to_owned(),
        (true, n) => format!("Ports — {n} ports d'espace de travail"),
        (false, 1) => "Ports — 1 workspace port".to_owned(),
        (false, n) => format!("Ports — {n} workspace ports"),
    };
    match (french, ext_count) {
        (_, 0) => main,
        (true, 1) => format!("{main} · 1 externe"),
        (true, e) => format!("{main} · {e} externes"),
        (false, e) => format!("{main} · {e} external"),
    }
}

fn agent_updating_text(names: &[String], french: bool) -> String {
    match (french, names.len()) {
        (_, 0) => String::new(),
        (true, 1) => format!("Mise à jour de {}…", names[0]),
        (true, n) => format!("Mise à jour de {n} agents…"),
        (false, 1) => format!("Updating {}…", names[0]),
        (false, n) => format!("Updating {n} agents…"),
    }
}

fn trigger_bg_hover(ui_colors: &UiPalette) -> Color32 {
    Color32::from_rgba_unmultiplied(
        ui_colors.surface_3.r(),
        ui_colors.surface_3.g(),
        ui_colors.surface_3.b(),
        180,
    )
}

#[allow(clippy::too_many_lines)]
pub fn show_full(
    ui: &mut egui::Ui,
    state: &mut StatusBarState,
    input: &StatusBarInput<'_>,
) -> Option<StatusBarAction> {
    let mut action: Option<StatusBarAction> = None;
    let room = ui.available_rect_before_wrap();
    state.bar_rect = Some(room);

    let density = status_bar_density(room.width());
    let compact = density != StatusBarDensity::Full;
    let icon_only = density == StatusBarDensity::IconOnly;
    let lang = lang_of(input.french);
    let font_11 = FontId::proportional(11.0);
    let text_col = input.ui_colors.text;
    let dim_col = input.ui_colors.text_dim;
    let strong_col = input.ui_colors.text_strong;
    let sep_col = dim_col.gamma_multiply(0.5);
    let muted_icon_col = dim_col.gamma_multiply(0.85);

    if state.open.is_some() && ui.input(|i| i.key_pressed(Key::Escape)) {
        state.open = None;
    }
    let prev_open = state.open;

    let now_sec = ui.input(|i| i.time);
    let spin_angle = ((now_sec * std::f64::consts::TAU) % std::f64::consts::TAU) as f32;
    let mut needs_spin_repaint = false;

    // --- Measure & paint right-side segments (right-to-left geometry, left-to-right visual order)
    let mut trigger_rects: Vec<Rect> = Vec::with_capacity(5);
    let mut right_cursor = room.right();
    let seg_gap = 12.0_f32;
    let btn_h = 20.0_f32;
    let btn_y = room.center().y - btn_h * 0.5;

    // Segment 5 (rightmost): Remote Hosts (`v-if="shows('ssh')"`)
    if input.show_ssh {
        let connected_count = input
            .remote_hosts
            .iter()
            .filter(|h| h.status == RemoteHostStatusKind::Connected)
            .count();
        let any_connecting = input.remote_hosts.iter().any(|h| h.status.is_connecting());
        needs_spin_repaint |= any_connecting;
        let count_text = if any_connecting {
            if input.french {
                "Connexion…".to_owned()
            } else {
                "Connecting…".to_owned()
            }
        } else if input.french {
            if connected_count == 1 {
                "1 hôte".to_owned()
            } else {
                format!("{connected_count} hôtes")
            }
        } else if connected_count == 1 {
            "1 host".to_owned()
        } else {
            format!("{connected_count} hosts")
        };

        let w = if icon_only {
            // 6px dot + 6px gap + 12px icon + 8px pad
            8.0 + 6.0 + 6.0 + 12.0
        } else if compact {
            // 12px icon + 6px gap + 6px dot + 8px pad
            8.0 + 12.0 + 6.0 + 6.0
        } else {
            let tw = measure_text_w(ui.painter(), &count_text, font_11.clone(), dim_col);
            8.0 + 12.0 + 6.0 + tw + 6.0 + 6.0
        };
        let rect = Rect::from_min_size(
            Pos2::new((right_cursor - w).max(room.left()), btn_y),
            Vec2::new(w, btn_h),
        );
        right_cursor = rect.left() - seg_gap;
        trigger_rects.push(rect);
        state.remote_anchor = Some(rect);

        let is_open = matches!(state.open, Some(OpenPopover::RemoteHosts));
        let a11y = if input.french {
            "État de connexion de l'hôte distant"
        } else {
            "Remote host connection status"
        };
        let resp = ui.interact(rect, ui.id().with("sb-ssh"), Sense::click());
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), a11y)
        });
        if resp.hovered() || is_open {
            ui.painter().rect_filled(
                rect,
                CornerRadius::same(4),
                trigger_bg_hover(input.ui_colors),
            );
        }
        let dot_col = if any_connecting {
            Color32::from_rgb(0xea, 0xb3, 0x08)
        } else if connected_count > 0 {
            Color32::from_rgb(0x10, 0xb9, 0x81)
        } else {
            dim_col.gamma_multiply(0.45)
        };
        let mut x = rect.left() + 4.0;
        let cy = rect.center().y;
        if icon_only {
            ui.painter()
                .circle_filled(Pos2::new(x + 3.0, cy), 3.0, dot_col);
            x += 12.0;
            let ic_rect = Rect::from_min_size(Pos2::new(x, cy - 6.0), Vec2::splat(12.0));
            if any_connecting {
                icons::paint_spinner(ui.painter(), ic_rect, muted_icon_col, spin_angle);
            } else {
                icons::paint(
                    ui.painter(),
                    ic_rect,
                    Icon::MonitorSmartphone,
                    muted_icon_col,
                );
            }
        } else {
            let ic_rect = Rect::from_min_size(Pos2::new(x, cy - 6.0), Vec2::splat(12.0));
            if any_connecting {
                icons::paint_spinner(
                    ui.painter(),
                    ic_rect,
                    Color32::from_rgb(0xea, 0xb3, 0x08),
                    spin_angle,
                );
            } else if !input.remote_hosts.is_empty() && connected_count == input.remote_hosts.len()
            {
                icons::paint(
                    ui.painter(),
                    ic_rect,
                    Icon::Server,
                    Color32::from_rgb(0x10, 0xb9, 0x81),
                );
            } else if connected_count > 0 {
                icons::paint(ui.painter(), ic_rect, Icon::Server, muted_icon_col);
            } else {
                icons::paint(ui.painter(), ic_rect, Icon::ServerOff, muted_icon_col);
            }
            x += 18.0;
            if !compact {
                let col = if resp.hovered() { strong_col } else { dim_col };
                ui.painter().text(
                    Pos2::new(x, cy),
                    Align2::LEFT_CENTER,
                    &count_text,
                    font_11.clone(),
                    col,
                );
                x += measure_text_w(ui.painter(), &count_text, font_11.clone(), col) + 6.0;
            }
            ui.painter()
                .circle_filled(Pos2::new(x + 3.0, cy), 3.0, dot_col);
        }
        if resp.on_hover_text(a11y).clicked() {
            state.open = if is_open {
                None
            } else {
                Some(OpenPopover::RemoteHosts)
            };
        }
    }

    // Segment 4: Ports (`v-if="shows('ports')"`)
    if input.show_ports {
        let summary_entries: Vec<_> = input
            .telemetry
            .ports
            .iter()
            .map(DetectedPort::to_summary_entry)
            .collect();
        let psum = ports_summary(&summary_entries, lang);
        let total_ports = psum.workspace_count + psum.external_count;
        let show_num = !icon_only || total_ports > 0;
        let num_str = psum.workspace_count.to_string();
        let nw = if show_num {
            measure_text_w(ui.painter(), &num_str, font_11.clone(), dim_col)
        } else {
            0.0
        };
        let w = 8.0 + 12.0 + if show_num { 6.0 + nw } else { 0.0 };
        let rect = Rect::from_min_size(
            Pos2::new((right_cursor - w).max(room.left()), btn_y),
            Vec2::new(w, btn_h),
        );
        right_cursor = rect.left() - seg_gap;
        trigger_rects.push(rect);
        state.ports_anchor = Some(rect);

        let is_open = matches!(state.open, Some(OpenPopover::Ports));
        let tip = ports_tooltip_text(psum.workspace_count, psum.external_count, input.french);
        let a11y = match (input.french, psum.workspace_count) {
            (true, 1) => "Ports, 1 port d'espace de travail".to_owned(),
            (true, n) => format!("Ports, {n} ports d'espace de travail"),
            (false, 1) => "Ports, 1 workspace port".to_owned(),
            (false, n) => format!("Ports, {n} workspace ports"),
        };
        let resp = ui.interact(rect, ui.id().with("sb-ports"), Sense::click());
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &a11y)
        });
        if resp.hovered() || is_open {
            ui.painter().rect_filled(
                rect,
                CornerRadius::same(4),
                trigger_bg_hover(input.ui_colors),
            );
        }
        let cy = rect.center().y;
        let ic_rect =
            Rect::from_min_size(Pos2::new(rect.left() + 4.0, cy - 6.0), Vec2::splat(12.0));
        if input.telemetry.ports_refreshing {
            needs_spin_repaint = true;
            icons::paint_spinner(ui.painter(), ic_rect, muted_icon_col, spin_angle);
        } else {
            icons::paint(ui.painter(), ic_rect, Icon::Plug, muted_icon_col);
        }
        if show_num {
            let col = if resp.hovered() { strong_col } else { dim_col };
            ui.painter().text(
                Pos2::new(rect.left() + 22.0, cy),
                Align2::LEFT_CENTER,
                &num_str,
                font_11.clone(),
                col,
            );
        }
        if resp.on_hover_text(tip).clicked() {
            if is_open {
                state.open = None;
            } else {
                state.open = Some(OpenPopover::Ports);
                action = Some(StatusBarAction::RefreshPorts);
            }
        }
    }

    // Segment 3: Resource Manager (`v-if="shows('resource-usage')"`)
    if input.show_resources {
        let mem_badge = format_memory(
            input
                .telemetry
                .resources
                .as_ref()
                .map(|s| s.total_memory_bytes),
            lang,
        );
        let term_count = input.terminals.len();
        let count_str = term_count.to_string();
        let w = if icon_only {
            if term_count > 0 {
                8.0 + 12.0
                    + 6.0
                    + measure_text_w(ui.painter(), &count_str, font_11.clone(), dim_col)
            } else {
                8.0 + 12.0
            }
        } else {
            let mw = measure_text_w(ui.painter(), &mem_badge, font_11.clone(), dim_col);
            let dot_w = measure_text_w(ui.painter(), "·", font_11.clone(), sep_col);
            let cw = measure_text_w(ui.painter(), &count_str, font_11.clone(), dim_col);
            8.0 + 12.0 + 6.0 + mw + 6.0 + dot_w + 6.0 + 12.0 + 5.0 + cw
        };
        let rect = Rect::from_min_size(
            Pos2::new((right_cursor - w).max(room.left()), btn_y),
            Vec2::new(w, btn_h),
        );
        right_cursor = rect.left() - seg_gap;
        trigger_rects.push(rect);
        state.resources_anchor = Some(rect);

        let is_open = matches!(state.open, Some(OpenPopover::Resources));
        let tip =
            resource_tooltip_text(input.telemetry.resources.as_ref(), term_count, input.french);
        let a11y = match (input.french, term_count) {
            (true, 1) => "Gestionnaire de ressources, 1 session de terminal".to_owned(),
            (true, n) => format!("Gestionnaire de ressources, {n} sessions de terminal"),
            (false, 1) => "Resource Manager, 1 terminal session".to_owned(),
            (false, n) => format!("Resource Manager, {n} terminal sessions"),
        };
        let resp = ui.interact(rect, ui.id().with("sb-resources"), Sense::click());
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &a11y)
        });
        if resp.hovered() || is_open {
            ui.painter().rect_filled(
                rect,
                CornerRadius::same(4),
                trigger_bg_hover(input.ui_colors),
            );
        }
        let col = if resp.hovered() { strong_col } else { dim_col };
        let cy = rect.center().y;
        let mut x = rect.left() + 4.0;
        icons::paint(
            ui.painter(),
            Rect::from_min_size(Pos2::new(x, cy - 6.0), Vec2::splat(12.0)),
            Icon::MemoryStick,
            muted_icon_col,
        );
        x += 18.0;
        if icon_only {
            if term_count > 0 {
                ui.painter().text(
                    Pos2::new(x, cy),
                    Align2::LEFT_CENTER,
                    &count_str,
                    font_11.clone(),
                    col,
                );
            }
        } else {
            ui.painter().text(
                Pos2::new(x, cy),
                Align2::LEFT_CENTER,
                &mem_badge,
                font_11.clone(),
                col,
            );
            x += measure_text_w(ui.painter(), &mem_badge, font_11.clone(), col) + 6.0;
            ui.painter().text(
                Pos2::new(x, cy),
                Align2::LEFT_CENTER,
                "·",
                font_11.clone(),
                sep_col,
            );
            x += measure_text_w(ui.painter(), "·", font_11.clone(), sep_col) + 6.0;
            icons::paint(
                ui.painter(),
                Rect::from_min_size(Pos2::new(x, cy - 6.0), Vec2::splat(12.0)),
                Icon::Terminal,
                muted_icon_col,
            );
            x += 17.0;
            ui.painter().text(
                Pos2::new(x, cy),
                Align2::LEFT_CENTER,
                &count_str,
                font_11.clone(),
                col,
            );
        }
        if resp.on_hover_text(tip).clicked() {
            state.open = if is_open {
                None
            } else {
                Some(OpenPopover::Resources)
            };
        }
    }

    // Segment 2: Keep computer awake (always shown)
    {
        let copy = awake_copy(input.keep_awake_mode, input.keep_awake_active, lang);
        let title_word = if input.french {
            "Empêcher la mise en veille de l'ordinateur"
        } else {
            "Keep computer awake"
        };
        let aria = format!("{title_word}, {}", copy.status);
        let lw = if icon_only {
            0.0
        } else {
            measure_text_w(ui.painter(), copy.label, font_11.clone(), dim_col)
        };
        let w = if icon_only {
            8.0 + 12.0 + 4.0 + 6.0
        } else {
            8.0 + 12.0 + 4.0 + lw + 4.0 + 6.0
        };
        let rect = Rect::from_min_size(
            Pos2::new((right_cursor - w).max(room.left()), btn_y),
            Vec2::new(w, btn_h),
        );
        right_cursor = rect.left() - seg_gap;
        trigger_rects.push(rect);
        state.awake_anchor = Some(rect);

        let is_open = matches!(state.open, Some(OpenPopover::KeepAwake));
        let resp = ui.interact(rect, ui.id().with("sb-awake"), Sense::click());
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &aria)
        });
        if resp.hovered() || is_open {
            ui.painter().rect_filled(
                rect,
                CornerRadius::same(4),
                trigger_bg_hover(input.ui_colors),
            );
        }
        let cy = rect.center().y;
        let mut x = rect.left() + 4.0;
        let coffee_col = if input.keep_awake_active || resp.hovered() {
            strong_col
        } else {
            dim_col
        };
        icons::paint(
            ui.painter(),
            Rect::from_min_size(Pos2::new(x, cy - 6.0), Vec2::splat(12.0)),
            Icon::Coffee,
            coffee_col,
        );
        x += 16.0;
        if !icon_only {
            let label_col = if resp.hovered() { strong_col } else { dim_col };
            ui.painter().text(
                Pos2::new(x, cy),
                Align2::LEFT_CENTER,
                copy.label,
                font_11.clone(),
                label_col,
            );
            x += lw + 4.0;
        }
        let dot_col = if input.keep_awake_active {
            strong_col
        } else {
            dim_col.gamma_multiply(0.4)
        };
        ui.painter()
            .circle_filled(Pos2::new(x + 3.0, cy), 3.0, dot_col);
        if resp.on_hover_text(&aria).clicked() {
            state.open = if is_open {
                None
            } else {
                Some(OpenPopover::KeepAwake)
            };
        }
    }

    // Segment 1: Background Agent Update (`v-if="agentUpdatingText"`)
    let upd_text = agent_updating_text(input.agent_updating, input.french);
    if !upd_text.is_empty() {
        needs_spin_repaint = true;
        let uw = if icon_only {
            0.0
        } else {
            measure_text_w(ui.painter(), &upd_text, font_11.clone(), dim_col)
        };
        let w = 8.0 + 12.0 + if icon_only { 0.0 } else { 6.0 + uw };
        let rect = Rect::from_min_size(
            Pos2::new((right_cursor - w).max(room.left()), btn_y),
            Vec2::new(w, btn_h),
        );
        right_cursor = rect.left() - seg_gap;
        trigger_rects.push(rect);
        let resp = ui.interact(rect, ui.id().with("sb-agent-update"), Sense::click());
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &upd_text)
        });
        if resp.hovered() {
            ui.painter().rect_filled(
                rect,
                CornerRadius::same(4),
                trigger_bg_hover(input.ui_colors),
            );
        }
        let cy = rect.center().y;
        icons::paint_spinner(
            ui.painter(),
            Rect::from_min_size(Pos2::new(rect.left() + 4.0, cy - 6.0), Vec2::splat(12.0)),
            muted_icon_col,
            spin_angle,
        );
        if !icon_only {
            ui.painter().text(
                Pos2::new(rect.left() + 22.0, cy),
                Align2::LEFT_CENTER,
                &upd_text,
                font_11.clone(),
                if resp.hovered() { strong_col } else { dim_col },
            );
        }
        if resp.on_hover_text(&upd_text).clicked() {
            action = Some(StatusBarAction::OpenAgentUpdates);
        }
    }

    if needs_spin_repaint {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));
    }

    // --- Left side: Error/Persistence notice OR (Target + Summary + Path)
    let left_max_x = (right_cursor + seg_gap - 16.0).max(room.left());
    let left_room = Rect::from_min_max(room.min, Pos2::new(left_max_x, room.bottom()));
    if left_room.width() > 8.0 {
        if input.persistence_unavailable || !input.error_banner.is_empty() {
            ui.scope_builder(egui::UiBuilder::new().max_rect(left_room), |ui| {
                ui.horizontal_centered(|ui| {
                    if input.persistence_unavailable {
                        ui.colored_label(Color32::YELLOW, "Persistence unavailable");
                    }
                    if !input.error_banner.is_empty() {
                        ui.colored_label(Color32::LIGHT_RED, input.error_banner);
                        let dismiss_lbl = if input.french { "Fermer" } else { "Dismiss" };
                        if ui.small_button(dismiss_lbl).clicked() {
                            action = Some(StatusBarAction::DismissError);
                        }
                    }
                });
            });
        } else {
            let target_str = format_target(input.target, input.french);
            let summary_str = format_pane_summary(
                input.pane_count,
                input.working_count,
                input.waiting_count,
                input.french,
            );
            let mut x = left_room.left();
            if !target_str.is_empty() && x < left_room.right() {
                let tw = measure_text_w(ui.painter(), &target_str, font_11.clone(), text_col)
                    .min(left_room.width() * 0.45)
                    .min((left_room.right() - x).max(0.0));
                truncated_text(
                    ui,
                    Rect::from_min_size(
                        Pos2::new(x, left_room.top()),
                        Vec2::new(tw, left_room.height()),
                    ),
                    &target_str,
                    text_col,
                    &target_str,
                );
                x += tw + 12.0;
            }
            if x < left_room.right() {
                let sw = measure_text_w(ui.painter(), &summary_str, font_11, dim_col)
                    .min((left_room.right() - x).max(0.0));
                truncated_text(
                    ui,
                    Rect::from_min_size(
                        Pos2::new(x, left_room.top()),
                        Vec2::new(sw, left_room.height()),
                    ),
                    &summary_str,
                    dim_col,
                    &summary_str,
                );
                x += sw + 12.0;
            }
            if !compact && !input.project_path.is_empty() && x < left_room.right() {
                truncated_text(
                    ui,
                    Rect::from_min_max(Pos2::new(x, left_room.top()), left_room.max),
                    input.project_path,
                    dim_col,
                    input.project_path,
                );
            }
        }
    }

    // --- Right-click on status bar (outside exempt triggers) opens visibility menu
    let mut opened_visibility = false;
    if ui.input(|i| i.pointer.secondary_clicked())
        && let Some(pos) = ui.input(|i| i.pointer.interact_pos())
        && room.contains(pos)
        && !trigger_rects.iter().any(|r| r.contains(pos))
    {
        state.open = Some(OpenPopover::Visibility(pos));
        opened_visibility = true;
    }

    // --- Render Active Popover (`Order::Foreground`)
    if let Some(open_kind) = state.open {
        let pop_resp = match open_kind {
            OpenPopover::KeepAwake => show_keep_awake_menu(
                ui.ctx(),
                state.awake_anchor.unwrap_or(room),
                input,
                &mut action,
            ),
            OpenPopover::Visibility(anchor_pt) => {
                show_visibility_menu(ui.ctx(), anchor_pt, input, &mut action)
            }
            OpenPopover::Resources => show_resources_popover(
                ui.ctx(),
                state.resources_anchor.unwrap_or(room),
                state,
                input,
                &mut action,
            ),
            OpenPopover::Ports => show_ports_popover(
                ui.ctx(),
                state.ports_anchor.unwrap_or(room),
                state,
                input,
                now_sec,
                spin_angle,
                &mut action,
            ),
            OpenPopover::RemoteHosts => show_remote_hosts_popover(
                ui.ctx(),
                state.remote_anchor.unwrap_or(room),
                input,
                spin_angle,
                &mut action,
            ),
        };

        if matches!(
            action,
            Some(
                StatusBarAction::SetKeepAwake(_)
                    | StatusBarAction::FocusSession(_)
                    | StatusBarAction::ActivateWorkspace(_)
                    | StatusBarAction::ConnectRemoteHost(_)
                    | StatusBarAction::ManageRemoteHosts
                    | StatusBarAction::OpenPortInBrowser { .. }
            )
        ) {
            state.open = None;
        } else if state.open == prev_open
            && !opened_visibility
            && let Some(pop_rect) = pop_resp
            && ui.input(|i| i.pointer.any_pressed())
            && let Some(pos) = ui.input(|i| i.pointer.interact_pos())
            && !pop_rect.contains(pos)
            && !trigger_rects.iter().any(|r| r.contains(pos))
        {
            state.open = None;
        }
    }

    action
}

fn popover_frame(ui_colors: &UiPalette) -> egui::Frame {
    egui::Frame::new()
        .fill(ui_colors.surface)
        .stroke(Stroke::new(1.0_f32, ui_colors.border))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::ZERO)
        .shadow(egui::Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(115),
        })
}

fn place_above_anchor(ctx: &egui::Context, anchor: Rect, width: f32, est_height: f32) -> Pos2 {
    let screen = ctx.content_rect();
    let left = (anchor.right() - width).clamp(8.0, (screen.right() - width - 8.0).max(8.0));
    let top = (anchor.top() - est_height - 8.0).max(8.0);
    Pos2::new(left, top)
}

fn show_keep_awake_menu(
    ctx: &egui::Context,
    anchor: Rect,
    input: &StatusBarInput<'_>,
    action: &mut Option<StatusBarAction>,
) -> Option<Rect> {
    let width = 256.0_f32;
    let pos = place_above_anchor(ctx, anchor, width, 182.0);
    let lang = lang_of(input.french);
    let current = awake_copy(input.keep_awake_mode, input.keep_awake_active, lang);
    let title = if input.french {
        "Empêcher la mise en veille de l'ordinateur"
    } else {
        "Keep computer awake"
    };

    let area = egui::Area::new(Id::new("sb-awake-popover"))
        .order(Order::Foreground)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            popover_frame(input.ui_colors).show(ui, |ui| {
                ui.set_width(width);
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new(title)
                            .size(11.0)
                            .strong()
                            .color(input.ui_colors.text),
                    );
                });
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new(&current.status)
                            .size(10.5)
                            .color(input.ui_colors.text_dim),
                    );
                });
                ui.add_space(4.0);
                let sep_y = ui.cursor().top();
                ui.painter().hline(
                    ui.max_rect().x_range(),
                    sep_y,
                    Stroke::new(1.0_f32, input.ui_colors.border),
                );
                ui.add_space(4.0);

                for mode in [KeepAwakeMode::On, KeepAwakeMode::Agents, KeepAwakeMode::Off] {
                    let item = awake_copy(mode, input.keep_awake_active, lang);
                    let checked = input.keep_awake_mode == mode;
                    let (row_rect, resp) =
                        ui.allocate_exact_size(Vec2::new(width - 8.0, 40.0), Sense::click());
                    let row_rect = row_rect.translate(Vec2::new(4.0, 0.0));
                    resp.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::RadioButton,
                            true,
                            checked,
                            item.label,
                        )
                    });
                    if resp.hovered() {
                        ui.painter().rect_filled(
                            row_rect,
                            CornerRadius::same(4),
                            input.ui_colors.surface_3,
                        );
                    }
                    ui.painter().text(
                        Pos2::new(row_rect.left() + 8.0, row_rect.top() + 12.0),
                        Align2::LEFT_CENTER,
                        item.label,
                        FontId::proportional(11.5),
                        input.ui_colors.text_strong,
                    );
                    let mut desc_job = egui::text::LayoutJob::simple(
                        item.hint.to_owned(),
                        FontId::proportional(10.5),
                        input.ui_colors.text_dim,
                        row_rect.width() - 30.0,
                    );
                    desc_job.wrap.max_rows = 1;
                    desc_job.wrap.overflow_character = Some('…');
                    let galley = ui.painter().layout_job(desc_job);
                    ui.painter().galley(
                        Pos2::new(row_rect.left() + 8.0, row_rect.top() + 22.0),
                        galley,
                        input.ui_colors.text_dim,
                    );
                    let radio_c = Pos2::new(row_rect.right() - 12.0, row_rect.center().y);
                    if checked {
                        ui.painter()
                            .circle_filled(radio_c, 3.5, input.ui_colors.accent);
                    } else {
                        ui.painter().circle_stroke(
                            radio_c,
                            3.5,
                            Stroke::new(1.0_f32, input.ui_colors.border),
                        );
                    }
                    if resp.clicked() {
                        *action = Some(StatusBarAction::SetKeepAwake(mode));
                    }
                }
                ui.add_space(4.0);
            });
        });
    Some(area.response.rect)
}

fn show_visibility_menu(
    ctx: &egui::Context,
    anchor_pt: Pos2,
    input: &StatusBarInput<'_>,
    action: &mut Option<StatusBarAction>,
) -> Option<Rect> {
    let width = 204.0_f32;
    let screen = ctx.content_rect();
    let pos = Pos2::new(
        anchor_pt
            .x
            .clamp(8.0, (screen.right() - width - 8.0).max(8.0)),
        (anchor_pt.y - 116.0).max(8.0),
    );
    let items = [
        (
            StatusBarItemId::Ssh,
            Icon::Server,
            if input.french {
                "Hôtes distants"
            } else {
                "Remote Hosts"
            },
            input.show_ssh,
        ),
        (
            StatusBarItemId::Resources,
            Icon::Activity,
            if input.french {
                "Gestionnaire de ressources"
            } else {
                "Resource Manager"
            },
            input.show_resources,
        ),
        (
            StatusBarItemId::Ports,
            Icon::Plug,
            "Ports",
            input.show_ports,
        ),
    ];

    let area = egui::Area::new(Id::new("sb-visibility-menu"))
        .order(Order::Foreground)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            popover_frame(input.ui_colors).show(ui, |ui| {
                ui.set_width(width);
                ui.add_space(4.0);
                for (id, icon, label, checked) in items {
                    let (r, resp) =
                        ui.allocate_exact_size(Vec2::new(width - 8.0, 28.0), Sense::click());
                    let r = r.translate(Vec2::new(4.0, 0.0));
                    resp.widget_info(|| {
                        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, checked, label)
                    });
                    if resp.hovered() {
                        ui.painter().rect_filled(
                            r,
                            CornerRadius::same(4),
                            input.ui_colors.surface_3,
                        );
                    }
                    icons::paint(
                        ui.painter(),
                        Rect::from_min_size(
                            Pos2::new(r.left() + 8.0, r.center().y - 6.5),
                            Vec2::splat(13.0),
                        ),
                        icon,
                        input.ui_colors.text_dim,
                    );
                    ui.painter().text(
                        Pos2::new(r.left() + 28.0, r.center().y),
                        Align2::LEFT_CENTER,
                        label,
                        FontId::proportional(11.5),
                        input.ui_colors.text,
                    );
                    if checked {
                        icons::paint(
                            ui.painter(),
                            Rect::from_min_size(
                                Pos2::new(r.right() - 20.0, r.center().y - 6.5),
                                Vec2::splat(13.0),
                            ),
                            Icon::Check,
                            input.ui_colors.accent,
                        );
                    }
                    if resp.clicked() {
                        *action = Some(StatusBarAction::ToggleItem(id));
                    }
                }
                ui.add_space(4.0);
            });
        });
    Some(area.response.rect)
}

fn paint_sparkline(painter: &Painter, rect: Rect, samples: &[f64], color: Color32) {
    let pts_str = sparkline_points(samples, rect.width() as f64, rect.height() as f64);
    if pts_str.is_empty() {
        return;
    }
    let pts: Vec<Pos2> = pts_str
        .split_whitespace()
        .filter_map(|pair| {
            let (xs, ys) = pair.split_once(',')?;
            let x = xs.parse::<f32>().ok()?;
            let y = ys.parse::<f32>().ok()?;
            Some(Pos2::new(rect.left() + x, rect.top() + y))
        })
        .collect();
    if pts.len() >= 2 {
        painter.add(Shape::Path(egui::epaint::PathShape::line(
            pts,
            Stroke::new(1.2_f32, color.gamma_multiply(0.75)),
        )));
    }
    if let Some((lx, ly)) = sparkline_lead(&pts_str) {
        painter.circle_filled(
            Pos2::new(rect.left() + lx as f32, rect.top() + ly as f32),
            1.6,
            color,
        );
    }
}

#[allow(clippy::too_many_lines)]
fn show_resources_popover(
    ctx: &egui::Context,
    anchor: Rect,
    state: &mut StatusBarState,
    input: &StatusBarInput<'_>,
    action: &mut Option<StatusBarAction>,
) -> Option<Rect> {
    let width = 416.0_f32;
    let lang = lang_of(input.french);
    let tree = resource_tree(
        input.telemetry.resources.as_ref(),
        input.terminals,
        state.sort_by,
        lang,
    );
    let est_h = 240.0 + (input.terminals.len().min(6) as f32) * 24.0;
    let pos = place_above_anchor(ctx, anchor, width, est_h);

    let title = if input.french {
        "Gestionnaire de ressources"
    } else {
        "Resource Manager"
    };
    let cpu_hint = if input.french {
        "Charge CPU cumulée. Une valeur supérieure à 100 % signifie que plusieurs cœurs travaillent simultanément."
    } else {
        "Combined CPU load. Values above 100% mean more than one core is working at once."
    };
    let ws_hint = if input.french {
        "Somme de la plage de travail (WS) : pages présentes en RAM en ce moment."
    } else {
        "Summed working set (WS): pages resident in RAM right now."
    };
    let priv_hint = if input.french {
        "Somme des octets privés : mémoire engagée par ces processus."
    } else {
        "Summed private bytes: memory these processes have committed."
    };

    let area = egui::Area::new(Id::new("sb-resources-popover"))
        .order(Order::Foreground)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            popover_frame(input.ui_colors).show(ui, |ui| {
                ui.set_width(width);
                // Header
                let (head_r, _) = ui.allocate_exact_size(Vec2::new(width, 32.0), Sense::hover());
                icons::paint(
                    ui.painter(),
                    Rect::from_min_size(
                        Pos2::new(head_r.left() + 12.0, head_r.center().y - 6.0),
                        Vec2::splat(12.0),
                    ),
                    Icon::MemoryStick,
                    input.ui_colors.text_dim,
                );
                ui.painter().text(
                    Pos2::new(head_r.left() + 30.0, head_r.center().y),
                    Align2::LEFT_CENTER,
                    title,
                    FontId::proportional(12.0),
                    input.ui_colors.text_strong,
                );
                ui.painter().hline(
                    head_r.x_range(),
                    head_r.bottom(),
                    Stroke::new(1.0_f32, input.ui_colors.border),
                );

                // Summary strip
                if let Some(snap) = &input.telemetry.resources {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        ui.label(
                            egui::RichText::new(format_cpu(Some(snap.total_cpu_percent), lang))
                                .size(11.0)
                                .color(input.ui_colors.text),
                        )
                        .on_hover_text(cpu_hint);
                        ui.label(
                            egui::RichText::new("·")
                                .size(11.0)
                                .color(input.ui_colors.text_dim),
                        );
                        let mem_word = if input.french { "mémoire" } else { "memory" };
                        ui.label(
                            egui::RichText::new(format!(
                                "{} {mem_word}",
                                format_memory(Some(snap.total_memory_bytes), lang)
                            ))
                            .size(11.0)
                            .color(input.ui_colors.text),
                        )
                        .on_hover_text(ws_hint);
                        if snap.total_private_bytes > 0.0 {
                            ui.label(
                                egui::RichText::new("·")
                                    .size(11.0)
                                    .color(input.ui_colors.text_dim),
                            );
                            let priv_word = if input.french { "privée" } else { "private" };
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} {priv_word}",
                                    format_memory(Some(snap.total_private_bytes), lang)
                                ))
                                .size(11.0)
                                .color(input.ui_colors.text_dim),
                            )
                            .on_hover_text(priv_hint);
                        }
                    });
                    ui.add_space(6.0);
                    let y = ui.cursor().top();
                    ui.painter().hline(
                        head_r.x_range(),
                        y,
                        Stroke::new(1.0_f32, input.ui_colors.border),
                    );
                }

                // Sort bar
                let (sort_r, _) = ui.allocate_exact_size(Vec2::new(width, 24.0), Sense::hover());
                let name_lbl = if input.french { "NOM" } else { "NAME" };
                let mem_lbl = if input.french { "MÉMOIRE" } else { "MEMORY" };
                let name_btn = Rect::from_min_max(
                    Pos2::new(sort_r.left() + 12.0, sort_r.top()),
                    Pos2::new(sort_r.right() - 140.0, sort_r.bottom()),
                );
                let cpu_btn = Rect::from_min_max(
                    Pos2::new(sort_r.right() - 136.0, sort_r.top()),
                    Pos2::new(sort_r.right() - 84.0, sort_r.bottom()),
                );
                let mem_btn = Rect::from_min_max(
                    Pos2::new(sort_r.right() - 80.0, sort_r.top()),
                    Pos2::new(sort_r.right() - 12.0, sort_r.bottom()),
                );
                for (b_rect, col_kind, lbl, align) in [
                    (
                        name_btn,
                        ResourceSortBy::Name,
                        name_lbl,
                        Align2::LEFT_CENTER,
                    ),
                    (cpu_btn, ResourceSortBy::Cpu, "CPU", Align2::RIGHT_CENTER),
                    (
                        mem_btn,
                        ResourceSortBy::Memory,
                        mem_lbl,
                        Align2::RIGHT_CENTER,
                    ),
                ] {
                    let r = ui.interact(b_rect, ui.id().with(("res-sort", lbl)), Sense::click());
                    let active = state.sort_by == col_kind;
                    let c = if active || r.hovered() {
                        input.ui_colors.text_strong
                    } else {
                        input.ui_colors.text_dim
                    };
                    let anchor_x = if align == Align2::LEFT_CENTER {
                        b_rect.left()
                    } else {
                        b_rect.right()
                    };
                    ui.painter().text(
                        Pos2::new(anchor_x, b_rect.center().y),
                        align,
                        lbl,
                        FontId::proportional(10.0),
                        c,
                    );
                    if r.clicked() {
                        state.sort_by = col_kind;
                    }
                }
                ui.painter().hline(
                    sort_r.x_range(),
                    sort_r.bottom(),
                    Stroke::new(1.0_f32, input.ui_colors.border),
                );

                // Scrollable body
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        ui.set_width(width);
                        if let Some(tree) = &tree {
                            if tree.groups.is_empty() && input.terminals.is_empty() {
                                ui.add_space(12.0);
                                ui.vertical_centered(|ui| {
                                    ui.label(
                                        egui::RichText::new(if input.french {
                                            "Rien en cours d'exécution pour le moment"
                                        } else {
                                            "Nothing running right now"
                                        })
                                        .size(11.0)
                                        .color(input.ui_colors.text_dim),
                                    );
                                });
                                ui.add_space(12.0);
                            }
                            for g in &tree.groups {
                                let (gr, _) =
                                    ui.allocate_exact_size(Vec2::new(width, 26.0), Sense::hover());
                                ui.painter().text(
                                    Pos2::new(gr.left() + 12.0, gr.center().y),
                                    Align2::LEFT_CENTER,
                                    &g.name,
                                    FontId::proportional(11.5),
                                    input.ui_colors.text_strong,
                                );
                                let spark_r = Rect::from_min_size(
                                    Pos2::new(gr.right() - 192.0, gr.center().y - 7.0),
                                    Vec2::new(48.0, 14.0),
                                );
                                paint_sparkline(
                                    ui.painter(),
                                    spark_r,
                                    input.telemetry.sparklines.samples(&g.id),
                                    input.ui_colors.accent,
                                );
                                ui.painter().text(
                                    Pos2::new(gr.right() - 84.0, gr.center().y),
                                    Align2::RIGHT_CENTER,
                                    format_cpu(Some(g.cpu_percent), lang),
                                    FontId::proportional(11.0),
                                    input.ui_colors.text_dim,
                                );
                                ui.painter().text(
                                    Pos2::new(gr.right() - 12.0, gr.center().y),
                                    Align2::RIGHT_CENTER,
                                    format_memory(Some(g.memory_bytes), lang),
                                    FontId::proportional(11.0),
                                    input.ui_colors.text,
                                );

                                for row in &g.rows {
                                    let (sr, s_resp) = ui.allocate_exact_size(
                                        Vec2::new(width, 24.0),
                                        Sense::click(),
                                    );
                                    if s_resp.hovered() {
                                        ui.painter().rect_filled(
                                            sr,
                                            CornerRadius::ZERO,
                                            input.ui_colors.surface_3,
                                        );
                                    }
                                    let bound = input.bound_terminals.contains(&row.id);
                                    let dot_c = if bound {
                                        Color32::from_rgb(0x10, 0xb9, 0x81)
                                    } else {
                                        input.ui_colors.text_dim.gamma_multiply(0.4)
                                    };
                                    ui.painter().circle_filled(
                                        Pos2::new(sr.left() + 24.0, sr.center().y),
                                        3.0,
                                        dot_c,
                                    );
                                    let procs = input
                                        .telemetry
                                        .process_counts
                                        .get(&row.id)
                                        .copied()
                                        .unwrap_or(0);
                                    let disp_name = if procs > 1 {
                                        if input.french {
                                            format!("{} ({procs} proc.)", row.name)
                                        } else {
                                            format!("{} ({procs} procs)", row.name)
                                        }
                                    } else {
                                        row.name.clone()
                                    };
                                    ui.painter().text(
                                        Pos2::new(sr.left() + 34.0, sr.center().y),
                                        Align2::LEFT_CENTER,
                                        &disp_name,
                                        FontId::proportional(11.0),
                                        input.ui_colors.text,
                                    );
                                    let cpu_txt = if bound {
                                        format_cpu(Some(row.cpu_percent), lang)
                                    } else {
                                        "—".to_owned()
                                    };
                                    let mem_txt = if bound {
                                        format_memory(Some(row.memory_bytes), lang)
                                    } else {
                                        "—".to_owned()
                                    };
                                    ui.painter().text(
                                        Pos2::new(sr.right() - 84.0, sr.center().y),
                                        Align2::RIGHT_CENTER,
                                        cpu_txt,
                                        FontId::proportional(11.0),
                                        input.ui_colors.text_dim,
                                    );
                                    ui.painter().text(
                                        Pos2::new(sr.right() - 12.0, sr.center().y),
                                        Align2::RIGHT_CENTER,
                                        mem_txt,
                                        FontId::proportional(11.0),
                                        input.ui_colors.text,
                                    );
                                    let go_tip = if input.french {
                                        format!("Aller à {}", row.name)
                                    } else {
                                        format!("Go to {}", row.name)
                                    };
                                    if s_resp.on_hover_text(go_tip).clicked() {
                                        *action =
                                            Some(StatusBarAction::FocusSession(row.id.clone()));
                                    }
                                }
                            }

                            // Unbound terminals not yet in snapshot:
                            for meta in input.terminals {
                                if input
                                    .telemetry
                                    .resources
                                    .as_ref()
                                    .is_some_and(|s| s.terminals.contains_key(&meta.id))
                                {
                                    continue;
                                }
                                let (sr, s_resp) =
                                    ui.allocate_exact_size(Vec2::new(width, 24.0), Sense::click());
                                if s_resp.hovered() {
                                    ui.painter().rect_filled(
                                        sr,
                                        CornerRadius::ZERO,
                                        input.ui_colors.surface_3,
                                    );
                                }
                                ui.painter().circle_filled(
                                    Pos2::new(sr.left() + 24.0, sr.center().y),
                                    3.0,
                                    input.ui_colors.text_dim.gamma_multiply(0.4),
                                );
                                ui.painter().text(
                                    Pos2::new(sr.left() + 34.0, sr.center().y),
                                    Align2::LEFT_CENTER,
                                    &meta.name,
                                    FontId::proportional(11.0),
                                    input.ui_colors.text_dim,
                                );
                                ui.painter().text(
                                    Pos2::new(sr.right() - 84.0, sr.center().y),
                                    Align2::RIGHT_CENTER,
                                    "—",
                                    FontId::proportional(11.0),
                                    input.ui_colors.text_dim,
                                );
                                ui.painter().text(
                                    Pos2::new(sr.right() - 12.0, sr.center().y),
                                    Align2::RIGHT_CENTER,
                                    "—",
                                    FontId::proportional(11.0),
                                    input.ui_colors.text_dim,
                                );
                                if s_resp.clicked() {
                                    *action = Some(StatusBarAction::FocusSession(meta.id.clone()));
                                }
                            }

                            // Collapsible TESSEL row
                            let sep_y = ui.cursor().top();
                            ui.painter().hline(
                                sort_r.x_range(),
                                sep_y,
                                Stroke::new(1.0_f32, input.ui_colors.border.gamma_multiply(0.6)),
                            );
                            let (ar, a_resp) =
                                ui.allocate_exact_size(Vec2::new(width, 28.0), Sense::click());
                            if a_resp.hovered() {
                                ui.painter().rect_filled(
                                    ar,
                                    CornerRadius::ZERO,
                                    input.ui_colors.surface_3,
                                );
                            }
                            let chev_icon = if state.app_expanded {
                                Icon::ChevronDown
                            } else {
                                Icon::ChevronRight
                            };
                            icons::paint(
                                ui.painter(),
                                Rect::from_min_size(
                                    Pos2::new(ar.left() + 10.0, ar.center().y - 6.0),
                                    Vec2::splat(12.0),
                                ),
                                chev_icon,
                                input.ui_colors.text_dim,
                            );
                            ui.painter().text(
                                Pos2::new(ar.left() + 26.0, ar.center().y),
                                Align2::LEFT_CENTER,
                                "TESSEL",
                                FontId::proportional(10.5),
                                input.ui_colors.text_strong,
                            );
                            let app_spark_r = Rect::from_min_size(
                                Pos2::new(ar.right() - 192.0, ar.center().y - 7.0),
                                Vec2::new(48.0, 14.0),
                            );
                            paint_sparkline(
                                ui.painter(),
                                app_spark_r,
                                input.telemetry.sparklines.samples(APP_HISTORY_KEY),
                                input.ui_colors.accent,
                            );
                            ui.painter().text(
                                Pos2::new(ar.right() - 84.0, ar.center().y),
                                Align2::RIGHT_CENTER,
                                format_cpu(Some(tree.app_cpu_percent), lang),
                                FontId::proportional(11.0),
                                input.ui_colors.text_dim,
                            );
                            ui.painter().text(
                                Pos2::new(ar.right() - 12.0, ar.center().y),
                                Align2::RIGHT_CENTER,
                                format_memory(Some(tree.app_memory_bytes), lang),
                                FontId::proportional(11.0),
                                input.ui_colors.text,
                            );
                            if a_resp.clicked() {
                                state.app_expanded = !state.app_expanded;
                            }
                            if state.app_expanded {
                                for sub in &tree.app_rows {
                                    let (sr, _) = ui.allocate_exact_size(
                                        Vec2::new(width, 22.0),
                                        Sense::hover(),
                                    );
                                    ui.painter().circle_filled(
                                        Pos2::new(sr.left() + 24.0, sr.center().y),
                                        3.0,
                                        Color32::from_rgb(0x10, 0xb9, 0x81),
                                    );
                                    ui.painter().text(
                                        Pos2::new(sr.left() + 34.0, sr.center().y),
                                        Align2::LEFT_CENTER,
                                        &sub.label,
                                        FontId::proportional(11.0),
                                        input.ui_colors.text_dim,
                                    );
                                    ui.painter().text(
                                        Pos2::new(sr.right() - 84.0, sr.center().y),
                                        Align2::RIGHT_CENTER,
                                        format_cpu(Some(sub.cpu_percent), lang),
                                        FontId::proportional(11.0),
                                        input.ui_colors.text_dim,
                                    );
                                    ui.painter().text(
                                        Pos2::new(sr.right() - 12.0, sr.center().y),
                                        Align2::RIGHT_CENTER,
                                        format_memory(Some(sub.memory_bytes), lang),
                                        FontId::proportional(11.0),
                                        input.ui_colors.text,
                                    );
                                }
                            }
                        } else {
                            ui.add_space(16.0);
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    egui::RichText::new(if input.french {
                                        "Chargement…"
                                    } else {
                                        "Loading…"
                                    })
                                    .size(11.0)
                                    .color(input.ui_colors.text_dim),
                                );
                            });
                            ui.add_space(16.0);
                        }
                    });
            });
        });
    Some(area.response.rect)
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn show_ports_popover(
    ctx: &egui::Context,
    anchor: Rect,
    state: &mut StatusBarState,
    input: &StatusBarInput<'_>,
    now_sec: f64,
    spin_angle: f32,
    action: &mut Option<StatusBarAction>,
) -> Option<Rect> {
    let width = 384.0_f32;
    let lang = lang_of(input.french);
    let summary_entries: Vec<_> = input
        .telemetry
        .ports
        .iter()
        .map(DetectedPort::to_summary_entry)
        .collect();
    let psum = ports_summary(&summary_entries, lang);

    // Group workspace ports preserving discovery order:
    let mut ws_groups: Vec<(String, String, Vec<&DetectedPort>)> = Vec::new();
    let mut external_ports: Vec<&DetectedPort> = Vec::new();
    for p in &input.telemetry.ports {
        if let Some(ws_key) = &p.workspace_key {
            let ws_name = p.workspace_name.clone().unwrap_or_else(|| ws_key.clone());
            if let Some(g) = ws_groups.iter_mut().find(|g| g.0 == *ws_key) {
                g.2.push(p);
            } else {
                ws_groups.push((ws_key.clone(), ws_name, vec![p]));
            }
        } else {
            external_ports.push(p);
        }
    }

    let est_rows = psum.workspace_count
        + if state.external_expanded {
            external_ports.len().min(6)
        } else {
            0
        };
    let est_h = 150.0 + (est_rows as f32) * 36.0;
    let pos = place_above_anchor(ctx, anchor, width, est_h);

    let counts_txt = if input.french {
        let ws = if psum.workspace_count == 1 {
            "1 port d'espace de travail".to_owned()
        } else {
            format!("{} ports d'espace de travail", psum.workspace_count)
        };
        let ext = if psum.external_count <= 1 {
            format!("{} externe", psum.external_count)
        } else {
            format!("{} externes", psum.external_count)
        };
        format!("{ws} · {ext}")
    } else {
        let ws = if psum.workspace_count == 1 {
            "1 workspace port".to_owned()
        } else {
            format!("{} workspace ports", psum.workspace_count)
        };
        format!("{ws} · {} external", psum.external_count)
    };

    let area = egui::Area::new(Id::new("sb-ports-popover"))
        .order(Order::Foreground)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            popover_frame(input.ui_colors).show(ui, |ui| {
                ui.set_width(width);
                // Header
                let (hr, _) = ui.allocate_exact_size(Vec2::new(width, 34.0), Sense::hover());
                icons::paint(
                    ui.painter(),
                    Rect::from_min_size(
                        Pos2::new(hr.left() + 12.0, hr.center().y - 6.0),
                        Vec2::splat(12.0),
                    ),
                    Icon::Plug,
                    input.ui_colors.text_dim,
                );
                ui.painter().text(
                    Pos2::new(hr.left() + 28.0, hr.center().y),
                    Align2::LEFT_CENTER,
                    "Ports",
                    FontId::proportional(12.0),
                    input.ui_colors.text_strong,
                );
                ui.painter().text(
                    Pos2::new(hr.right() - 38.0, hr.center().y),
                    Align2::RIGHT_CENTER,
                    &counts_txt,
                    FontId::proportional(10.5),
                    input.ui_colors.text_dim,
                );
                let ref_rect = Rect::from_min_size(
                    Pos2::new(hr.right() - 30.0, hr.center().y - 11.0),
                    Vec2::splat(22.0),
                );
                let ref_lbl = if input.french {
                    "Chercher de nouveau"
                } else {
                    "Scan again"
                };
                let ref_resp = ui.interact(
                    ref_rect,
                    ui.id().with("ports-refresh-btn"),
                    Sense::click(),
                );
                ref_resp.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        !input.telemetry.ports_refreshing,
                        ref_lbl,
                    )
                });
                if ref_resp.hovered() {
                    ui.painter().rect_filled(
                        ref_rect,
                        CornerRadius::same(4),
                        input.ui_colors.surface_3,
                    );
                }
                let ref_ic = Rect::from_center_size(ref_rect.center(), Vec2::splat(12.0));
                if input.telemetry.ports_refreshing {
                    icons::paint_spinner(
                        ui.painter(),
                        ref_ic,
                        input.ui_colors.text_dim,
                        spin_angle,
                    );
                } else {
                    icons::paint(
                        ui.painter(),
                        ref_ic,
                        Icon::RefreshCw,
                        input.ui_colors.text_dim,
                    );
                }
                if ref_resp.on_hover_text(ref_lbl).clicked() && !input.telemetry.ports_refreshing {
                    *action = Some(StatusBarAction::RefreshPorts);
                }
                ui.painter().hline(
                    hr.x_range(),
                    hr.bottom(),
                    Stroke::new(1.0_f32, input.ui_colors.border),
                );

                if !input.telemetry.ports_error.is_empty() {
                    ui.add_space(4.0);
                    let err_msg = if input.french {
                        format!(
                            "Scan des ports indisponible sur cet ordinateur : {}",
                            input.telemetry.ports_error
                        )
                    } else {
                        format!(
                            "Port scan unavailable on this computer: {}",
                            input.telemetry.ports_error
                        )
                    };
                    ui.label(
                        egui::RichText::new(err_msg)
                            .size(11.0)
                            .color(input.ui_colors.danger),
                    );
                }

                egui::ScrollArea::vertical()
                    .max_height(380.0)
                    .show(ui, |ui| {
                        ui.set_width(width);
                        if ws_groups.is_empty() {
                            ui.add_space(14.0);
                            if input.telemetry.ports_refreshing {
                                ui.vertical_centered(|ui| {
                                    ui.label(
                                        egui::RichText::new(if input.french {
                                            "Recherche des ports de l'espace de travail..."
                                        } else {
                                            "Scanning for workspace ports..."
                                        })
                                        .size(11.0)
                                        .color(input.ui_colors.text_dim),
                                    );
                                });
                            } else {
                                ui.vertical_centered(|ui| {
                                    ui.label(
                                        egui::RichText::new(if input.french {
                                            "Aucun port d'espace de travail détecté"
                                        } else {
                                            "No workspace ports detected"
                                        })
                                        .size(11.5)
                                        .color(input.ui_colors.text),
                                    );
                                    ui.add_space(4.0);
                                    let hint = if input.french {
                                        "Un programme lancé dans l'un de vos terminaux qui écoute sur un port (un serveur de développement, par exemple) s'affiche ici."
                                    } else {
                                        "A program started in one of your terminals that listens on a port (a dev server, for example) shows up here."
                                    };
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(hint)
                                                .size(10.5)
                                                .color(input.ui_colors.text_dim),
                                        )
                                        .wrap(),
                                    );
                                });
                            }
                            ui.add_space(14.0);
                        } else {
                            for (ws_key, ws_name, rows) in &ws_groups {
                                let (gh, _) = ui.allocate_exact_size(
                                    Vec2::new(width, 26.0),
                                    Sense::hover(),
                                );
                                ui.painter().text(
                                    Pos2::new(gh.left() + 12.0, gh.center().y),
                                    Align2::LEFT_CENTER,
                                    ws_name,
                                    FontId::proportional(11.0),
                                    input.ui_colors.text_strong,
                                );
                                let go_rect = Rect::from_min_size(
                                    Pos2::new(gh.right() - 52.0, gh.center().y - 10.0),
                                    Vec2::splat(20.0),
                                );
                                let go_lbl = if input.french {
                                    "Aller au worktree"
                                } else {
                                    "Go to Worktree"
                                };
                                let go_resp = ui.interact(
                                    go_rect,
                                    ui.id().with(("go-ws", ws_key)),
                                    Sense::click(),
                                );
                                if go_resp.hovered() {
                                    ui.painter().rect_filled(
                                        go_rect,
                                        CornerRadius::same(4),
                                        input.ui_colors.surface_3,
                                    );
                                }
                                icons::paint(
                                    ui.painter(),
                                    Rect::from_center_size(go_rect.center(), Vec2::splat(12.0)),
                                    Icon::FolderOpen,
                                    input.ui_colors.text_dim,
                                );
                                if go_resp.on_hover_text(go_lbl).clicked() {
                                    *action =
                                        Some(StatusBarAction::ActivateWorkspace(ws_key.clone()));
                                }
                                ui.painter().text(
                                    Pos2::new(gh.right() - 12.0, gh.center().y),
                                    Align2::RIGHT_CENTER,
                                    rows.len().to_string(),
                                    FontId::monospace(10.5),
                                    input.ui_colors.text_dim,
                                );

                                for port in rows {
                                    paint_port_row(
                                        ui,
                                        width,
                                        port,
                                        state,
                                        input,
                                        now_sec,
                                        action,
                                    );
                                }
                            }
                        }

                        // Collapsible External Ports section
                        let sep_y = ui.cursor().top();
                        ui.painter().hline(
                            hr.x_range(),
                            sep_y,
                            Stroke::new(1.0_f32, input.ui_colors.border),
                        );
                        let (er, e_resp) =
                            ui.allocate_exact_size(Vec2::new(width, 28.0), Sense::click());
                        if e_resp.hovered() {
                            ui.painter()
                                .rect_filled(er, CornerRadius::ZERO, input.ui_colors.surface_3);
                        }
                        icons::paint(
                            ui.painter(),
                            Rect::from_min_size(
                                Pos2::new(er.left() + 10.0, er.center().y - 6.0),
                                Vec2::splat(12.0),
                            ),
                            if state.external_expanded {
                                Icon::ChevronDown
                            } else {
                                Icon::ChevronRight
                            },
                            input.ui_colors.text_dim,
                        );
                        let ext_title = if input.french {
                            "Ports externes"
                        } else {
                            "External Ports"
                        };
                        ui.painter().text(
                            Pos2::new(er.left() + 26.0, er.center().y),
                            Align2::LEFT_CENTER,
                            ext_title,
                            FontId::proportional(11.0),
                            input.ui_colors.text_dim,
                        );
                        ui.painter().text(
                            Pos2::new(er.right() - 12.0, er.center().y),
                            Align2::RIGHT_CENTER,
                            psum.external_count.to_string(),
                            FontId::monospace(10.5),
                            input.ui_colors.text_dim,
                        );
                        if e_resp.clicked() {
                            state.external_expanded = !state.external_expanded;
                        }
                        if state.external_expanded {
                            if external_ports.is_empty() {
                                ui.add_space(8.0);
                                ui.vertical_centered(|ui| {
                                    ui.label(
                                        egui::RichText::new(if input.french {
                                            "Aucun port externe détecté"
                                        } else {
                                            "No external ports detected"
                                        })
                                        .size(11.0)
                                        .color(input.ui_colors.text_dim),
                                    );
                                });
                                ui.add_space(8.0);
                            } else {
                                for port in &external_ports {
                                    paint_port_row(
                                        ui,
                                        width,
                                        port,
                                        state,
                                        input,
                                        now_sec,
                                        action,
                                    );
                                }
                            }
                        }
                    });
            });
        });
    Some(area.response.rect)
}

fn paint_port_row(
    ui: &mut egui::Ui,
    width: f32,
    port: &DetectedPort,
    state: &mut StatusBarState,
    input: &StatusBarInput<'_>,
    now_sec: f64,
    action: &mut Option<StatusBarAction>,
) {
    let (r, row_resp) = ui.allocate_exact_size(Vec2::new(width, 36.0), Sense::click());
    let btn_open = Rect::from_min_size(
        Pos2::new(r.right() - 84.0, r.center().y - 11.0),
        Vec2::splat(22.0),
    );
    let btn_copy = Rect::from_min_size(
        Pos2::new(r.right() - 58.0, r.center().y - 11.0),
        Vec2::splat(22.0),
    );
    let btn_stop = Rect::from_min_size(
        Pos2::new(r.right() - 32.0, r.center().y - 11.0),
        Vec2::splat(22.0),
    );

    let open_resp = ui.interact(
        btn_open,
        ui.id().with(("port-open", port.port, port.pid)),
        Sense::click(),
    );
    let copy_resp = ui.interact(
        btn_copy,
        ui.id().with(("port-copy", port.port, port.pid)),
        Sense::click(),
    );
    let stop_resp = ui.interact(
        btn_stop,
        ui.id().with(("port-stop", port.port, port.pid)),
        Sense::click(),
    );

    let any_hover =
        row_resp.hovered() || open_resp.hovered() || copy_resp.hovered() || stop_resp.hovered();
    if any_hover {
        ui.painter()
            .rect_filled(r, CornerRadius::ZERO, input.ui_colors.surface_3);
    }

    // Port number (monospace bold, no leading colon per PortRow.vue:37)
    ui.painter().text(
        Pos2::new(r.left() + 12.0, r.center().y),
        Align2::LEFT_CENTER,
        port.port.to_string(),
        FontId::monospace(12.0),
        input.ui_colors.text_strong,
    );

    let proc_lbl = if !port.process_name.is_empty() {
        port.process_name.clone()
    } else if port.pid != 0 {
        format!("PID {}", port.pid)
    } else if input.french {
        "Processus inconnu".to_owned()
    } else {
        "Unknown process".to_owned()
    };

    ui.painter().text(
        Pos2::new(r.left() + 64.0, r.top() + 11.0),
        Align2::LEFT_CENTER,
        &proc_lbl,
        FontId::proportional(11.5),
        input.ui_colors.text,
    );
    ui.painter().text(
        Pos2::new(r.left() + 64.0, r.top() + 25.0),
        Align2::LEFT_CENTER,
        &port.address,
        FontId::monospace(10.0),
        input.ui_colors.text_dim,
    );

    // 1. Open in Browser button (and clicking the row itself also opens in embedded Browser)
    let open_lbl = if input.french {
        "Ouvrir dans le navigateur"
    } else {
        "Open in Browser"
    };
    open_resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, open_lbl));
    if open_resp.hovered() {
        ui.painter()
            .rect_filled(btn_open, CornerRadius::same(4), input.ui_colors.border);
    }
    icons::paint(
        ui.painter(),
        Rect::from_center_size(btn_open.center(), Vec2::splat(12.0)),
        Icon::ExternalLink,
        if open_resp.hovered() {
            input.ui_colors.text_strong
        } else {
            input.ui_colors.text_dim
        },
    );

    // 2. Copy address button (1500ms Check feedback)
    let is_copied = state
        .copied_port
        .is_some_and(|(p, until)| p == port.port && now_sec < until);
    if is_copied {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(200));
    }
    let copy_lbl = if input.french {
        format!("Copier {}", port.address)
    } else {
        format!("Copy {}", port.address)
    };
    copy_resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &copy_lbl));
    if copy_resp.hovered() {
        ui.painter()
            .rect_filled(btn_copy, CornerRadius::same(4), input.ui_colors.border);
    }
    icons::paint(
        ui.painter(),
        Rect::from_center_size(btn_copy.center(), Vec2::splat(12.0)),
        if is_copied { Icon::Check } else { Icon::Copy },
        if is_copied {
            Color32::from_rgb(0x10, 0xb9, 0x81)
        } else if copy_resp.hovered() {
            input.ui_colors.text_strong
        } else {
            input.ui_colors.text_dim
        },
    );

    // 3. Stop process button
    let stop_lbl = if !port.can_stop {
        if input.french {
            "Indisponible pour les ports externes"
        } else {
            "Unavailable for external ports"
        }
    } else if input.french {
        "Arrêter le processus"
    } else {
        "Stop Process"
    };
    stop_resp.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, port.can_stop, stop_lbl)
    });
    if port.can_stop && stop_resp.hovered() {
        ui.painter()
            .rect_filled(btn_stop, CornerRadius::same(4), input.ui_colors.border);
    }
    let stop_col = if !port.can_stop {
        input.ui_colors.text_dim.gamma_multiply(0.3)
    } else if stop_resp.hovered() {
        Color32::from_rgb(0xf8, 0x71, 0x71)
    } else {
        input.ui_colors.text_dim
    };
    icons::paint(
        ui.painter(),
        Rect::from_center_size(btn_stop.center(), Vec2::splat(12.0)),
        Icon::Trash2,
        stop_col,
    );

    let copy_hovered = copy_resp.hovered();
    let stop_hovered = stop_resp.hovered();
    if copy_resp.on_hover_text(&copy_lbl).clicked() {
        state.copied_port = Some((port.port, now_sec + 1.5));
        *action = Some(StatusBarAction::CopyPortAddress {
            address: port.address.clone(),
            port: port.port,
        });
    } else if stop_resp.on_hover_text(stop_lbl).clicked() && port.can_stop {
        *action = Some(StatusBarAction::StopPort(port.clone()));
    } else if open_resp.on_hover_text(open_lbl).clicked()
        || (row_resp.on_hover_text(open_lbl).clicked() && !copy_hovered && !stop_hovered)
    {
        *action = Some(StatusBarAction::OpenPortInBrowser {
            url: port.url.clone(),
            port: port.port,
        });
    }
}

fn show_remote_hosts_popover(
    ctx: &egui::Context,
    anchor: Rect,
    input: &StatusBarInput<'_>,
    spin_angle: f32,
    action: &mut Option<StatusBarAction>,
) -> Option<Rect> {
    let width = 288.0_f32;
    let est_h = 90.0 + (input.remote_hosts.len().min(6) as f32) * 40.0;
    let pos = place_above_anchor(ctx, anchor, width, est_h);
    let title = if input.french {
        "HÔTES DISTANTS"
    } else {
        "REMOTE HOSTS"
    };

    // Connected hosts first, matching RemoteHostsStatus.vue:32
    let mut ordered: Vec<&RemoteHostRowInput> = input
        .remote_hosts
        .iter()
        .filter(|r| r.status == RemoteHostStatusKind::Connected)
        .chain(
            input
                .remote_hosts
                .iter()
                .filter(|r| r.status != RemoteHostStatusKind::Connected),
        )
        .collect();
    ordered.truncate(32);

    let area = egui::Area::new(Id::new("sb-remote-popover"))
        .order(Order::Foreground)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            popover_frame(input.ui_colors).show(ui, |ui| {
                ui.set_width(width);
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new(title)
                            .size(10.0)
                            .strong()
                            .color(input.ui_colors.text_dim),
                    );
                });
                ui.add_space(4.0);

                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .show(ui, |ui| {
                        ui.set_width(width);
                        for row in &ordered {
                            let (rr, _) =
                                ui.allocate_exact_size(Vec2::new(width, 38.0), Sense::hover());
                            ui.painter().circle_filled(
                                Pos2::new(rr.left() + 14.0, rr.center().y),
                                3.0,
                                row.status.dot_color(input.ui_colors),
                            );
                            ui.painter().text(
                                Pos2::new(rr.left() + 24.0, rr.top() + 12.0),
                                Align2::LEFT_CENTER,
                                &row.label,
                                FontId::proportional(11.5),
                                input.ui_colors.text_strong,
                            );
                            let sub = format!(
                                "{} · {}",
                                if input.french {
                                    "Hôte SSH"
                                } else {
                                    "SSH Host"
                                },
                                row.status.label(input.french)
                            );
                            ui.painter().text(
                                Pos2::new(rr.left() + 24.0, rr.top() + 26.0),
                                Align2::LEFT_CENTER,
                                sub,
                                FontId::proportional(10.0),
                                input.ui_colors.text_dim,
                            );

                            if row.status.is_connecting() {
                                icons::paint_spinner(
                                    ui.painter(),
                                    Rect::from_min_size(
                                        Pos2::new(rr.right() - 24.0, rr.center().y - 6.0),
                                        Vec2::splat(12.0),
                                    ),
                                    input.ui_colors.text_dim,
                                    spin_angle,
                                );
                            } else if row.status.can_connect() {
                                let verb = row.status.connect_verb(input.french);
                                let bw = measure_text_w(
                                    ui.painter(),
                                    verb,
                                    FontId::proportional(11.0),
                                    input.ui_colors.text,
                                ) + 14.0;
                                let br = Rect::from_min_size(
                                    Pos2::new(rr.right() - bw - 10.0, rr.center().y - 11.0),
                                    Vec2::new(bw, 22.0),
                                );
                                let b_resp = ui.interact(
                                    br,
                                    ui.id().with(("rh-conn", &row.alias)),
                                    Sense::click(),
                                );
                                ui.painter().rect(
                                    br,
                                    CornerRadius::same(4),
                                    if b_resp.hovered() {
                                        input.ui_colors.surface_3
                                    } else {
                                        Color32::TRANSPARENT
                                    },
                                    Stroke::new(1.0_f32, input.ui_colors.border),
                                    StrokeKind::Inside,
                                );
                                ui.painter().text(
                                    br.center(),
                                    Align2::CENTER_CENTER,
                                    verb,
                                    FontId::proportional(11.0),
                                    input.ui_colors.text_strong,
                                );
                                if b_resp.clicked() {
                                    *action =
                                        Some(StatusBarAction::ConnectRemoteHost(row.alias.clone()));
                                }
                            } else if row.status == RemoteHostStatusKind::Connected {
                                let verb = if input.french {
                                    "Se déconnecter"
                                } else {
                                    "Disconnect"
                                };
                                let bw = measure_text_w(
                                    ui.painter(),
                                    verb,
                                    FontId::proportional(11.0),
                                    input.ui_colors.text_dim,
                                ) + 14.0;
                                let br = Rect::from_min_size(
                                    Pos2::new(rr.right() - bw - 10.0, rr.center().y - 11.0),
                                    Vec2::new(bw, 22.0),
                                );
                                let b_resp = ui.interact(
                                    br,
                                    ui.id().with(("rh-disc", &row.alias)),
                                    Sense::click(),
                                );
                                if b_resp.hovered() {
                                    ui.painter().rect_filled(
                                        br,
                                        CornerRadius::same(4),
                                        input.ui_colors.surface_3,
                                    );
                                }
                                ui.painter().text(
                                    br.center(),
                                    Align2::CENTER_CENTER,
                                    verb,
                                    FontId::proportional(11.0),
                                    input.ui_colors.text_dim,
                                );
                                if b_resp.clicked() {
                                    *action = Some(StatusBarAction::DisconnectRemoteHost(
                                        row.alias.clone(),
                                    ));
                                }
                            }
                        }
                    });

                let sep_y = ui.cursor().top();
                ui.painter().hline(
                    ui.max_rect().x_range(),
                    sep_y,
                    Stroke::new(1.0_f32, input.ui_colors.border),
                );
                ui.add_space(4.0);
                let (mr, m_resp) =
                    ui.allocate_exact_size(Vec2::new(width - 8.0, 28.0), Sense::click());
                let mr = mr.translate(Vec2::new(4.0, 0.0));
                if m_resp.hovered() {
                    ui.painter()
                        .rect_filled(mr, CornerRadius::same(4), input.ui_colors.surface_3);
                }
                let manage_lbl = if input.french {
                    "Gérer les hôtes distants…"
                } else {
                    "Manage Remote Hosts…"
                };
                ui.painter().text(
                    Pos2::new(mr.left() + 8.0, mr.center().y),
                    Align2::LEFT_CENTER,
                    manage_lbl,
                    FontId::proportional(11.5),
                    input.ui_colors.text,
                );
                if m_resp.clicked() {
                    *action = Some(StatusBarAction::ManageRemoteHosts);
                }
                ui.add_space(4.0);
            });
        });
    Some(area.response.rect)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_strip_target_distinguishes_single_input_and_broadcast_en_fr() {
        assert_eq!(format_target(Target::None, false), "");
        assert_eq!(format_target(Target::Input(""), true), "");
        assert_eq!(format_target(Target::Input("Ada"), false), "Input → Ada");
        assert_eq!(format_target(Target::Input("Ada"), true), "Saisie → Ada");
        assert_eq!(
            format_target(Target::Broadcast(1), false),
            "Broadcast → 1 pane"
        );
        assert_eq!(
            format_target(Target::Broadcast(3), false),
            "Broadcast → 3 panes"
        );
        assert_eq!(
            format_target(Target::Broadcast(1), true),
            "Diffusion → 1 panneau"
        );
        assert_eq!(
            format_target(Target::Broadcast(3), true),
            "Diffusion → 3 panneaux"
        );
    }

    #[test]
    fn pane_summary_formats_working_and_waiting_counts_en_fr() {
        assert_eq!(format_pane_summary(1, 0, 0, false), "1 pane");
        assert_eq!(format_pane_summary(1, 0, 0, true), "1 panneau");
        assert_eq!(
            format_pane_summary(3, 2, 1, false),
            "3 panes · 2 working · 1 waiting for you"
        );
        assert_eq!(
            format_pane_summary(3, 2, 1, true),
            "3 panneaux · 2 au travail · 1 vous attendent"
        );
    }
}
