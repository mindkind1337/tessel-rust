#![allow(dead_code)]
//! Shared toolbar room model for both renderers (`toolbarFit.js`, `App.vue` `.tb-more`,
//! `LiveSharePill.vue` parity):
//! - [`toolbar_fit_level`]: how tight the toolbar is, decided from the room it measures (not from
//!   a fixed window width). Level 1 gathers the secondary buttons under one `⋯` button; level 2
//!   also hides the brand's version and shrinks the command box to its icon.
//! - [`more_menu_entries`]: the `⋯` menu (same actions as the hidden buttons, with shortcuts and a
//!   check on what is on).
//! - [`live_share_pill_label`] / [`live_share_pill_tooltip`]: the compact 20 px Live Share pill
//!   (a host: the number of people; a guest: the host's name), its words in the tooltip.

/// Least width the centred command box needs (its icon) with the centre's padding.
pub const TOOLBAR_CENTRE_MIN: f32 = 60.0;
/// Width of the `⋯` button that replaces the secondary buttons at level 1 (at least 28 px).
pub const TOOLBAR_MORE_WIDTH: f32 = 28.0;
/// Width of the icon-only command box at level 2.
pub const TOOLBAR_COMMAND_ICON_WIDTH: f32 = 30.0;
/// Height of the compact Live Share pill.
pub const LIVE_SHARE_PILL_HEIGHT: f32 = 20.0;
/// Diameter of the compact Live Share pill's dot.
pub const LIVE_SHARE_PILL_DOT: f32 = 7.0;

/// `toolbarFitLevel` (`toolbarFit.js`): `0` everything shown, `1` secondary buttons under `⋯`,
/// `2` also no version and an icon-only command box. Measurements are taken with everything shown.
#[must_use]
pub fn toolbar_fit_level(
    avail: f32,
    left: f32,
    right: f32,
    secondary: f32,
    more: f32,
    version: f32,
    centre: f32,
) -> u8 {
    let _ = version;
    if !avail.is_finite() || avail <= 0.0 {
        return 0;
    }
    if left + right + centre <= avail {
        return 0;
    }
    if left + (right - secondary + more) + centre <= avail {
        return 1;
    }
    2
}

/// Estimated room of a toolbar whose items have known widths (GPUI and egui lay out without a
/// DOM measurement, so each renderer sums its own fixed item widths).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ToolbarRoom {
    /// Width available to the toolbar's three groups (window width minus caption buttons).
    pub avail: f32,
    /// Left group with everything shown (brand, version, toggles, launch pills).
    pub left: f32,
    /// Right group with everything shown (secondary buttons included).
    pub right: f32,
    /// Sum of the secondary buttons (and their gaps) that move under `⋯` at level 1.
    pub secondary: f32,
    /// The brand's version text width.
    pub version: f32,
}

impl ToolbarRoom {
    #[must_use]
    pub fn level(&self) -> u8 {
        toolbar_fit_level(
            self.avail,
            self.left,
            self.right,
            self.secondary,
            TOOLBAR_MORE_WIDTH,
            self.version,
            TOOLBAR_CENTRE_MIN,
        )
    }
}

/// An action of the toolbar's `⋯` menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoreMenuAction {
    LiveShare,
    GitHub,
    Linear,
    Broadcast,
    FloatingTerminal,
    SplitRight,
    SplitDown,
    ArrangeGrid,
    ToggleLeftSidebar,
    ToggleRightSidebar,
    CloseActivePane,
}

/// One row of the `⋯` menu (`None` action = a separator).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoreMenuEntry {
    pub action: Option<MoreMenuAction>,
    pub label: &'static str,
    pub shortcut: Option<&'static str>,
    pub checked: bool,
    pub danger: bool,
}

impl MoreMenuEntry {
    const fn sep() -> Self {
        Self {
            action: None,
            label: "",
            shortcut: None,
            checked: false,
            danger: false,
        }
    }

    const fn item(
        action: MoreMenuAction,
        label: &'static str,
        shortcut: Option<&'static str>,
        checked: bool,
    ) -> Self {
        Self {
            action: Some(action),
            label,
            shortcut,
            checked,
            danger: false,
        }
    }

    #[must_use]
    pub const fn is_separator(&self) -> bool {
        self.action.is_none()
    }
}

/// State the `⋯` menu reflects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MoreMenuState {
    pub french: bool,
    pub broadcast: bool,
    pub floating_open: bool,
    pub left_open: bool,
    pub right_open: bool,
    pub can_arrange_panes: bool,
}

/// `⋯` button label and tooltip.
#[must_use]
pub const fn more_button_label(french: bool) -> &'static str {
    if french { "Plus" } else { "More" }
}

#[must_use]
pub const fn more_button_title(french: bool) -> &'static str {
    if french {
        "Plus : Live Share, tickets, diffusion, disposition, panneaux"
    } else {
        "More: Live Share, issues, broadcast, layout, panels"
    }
}

/// The `⋯` menu's rows in `App.vue` order.
#[must_use]
pub fn more_menu_entries(state: MoreMenuState) -> Vec<MoreMenuEntry> {
    let fr = state.french;
    let pick = |en: &'static str, f: &'static str| if fr { f } else { en };
    let mut out = vec![
        MoreMenuEntry::item(MoreMenuAction::LiveShare, "Live Share", None, false),
        MoreMenuEntry::item(
            MoreMenuAction::GitHub,
            pick("GitHub issues and PRs", "Tickets et PR GitHub"),
            None,
            false,
        ),
        MoreMenuEntry::item(
            MoreMenuAction::Linear,
            pick("Linear issues", "Issues Linear"),
            None,
            false,
        ),
        MoreMenuEntry::sep(),
        MoreMenuEntry::item(
            MoreMenuAction::Broadcast,
            pick("Broadcast", "Diffusion"),
            Some("Ctrl+Shift+B"),
            state.broadcast,
        ),
        MoreMenuEntry::item(
            MoreMenuAction::FloatingTerminal,
            pick("Floating terminal", "Terminal flottant"),
            Some("Ctrl+`"),
            state.floating_open,
        ),
        MoreMenuEntry::sep(),
        MoreMenuEntry::item(
            MoreMenuAction::SplitRight,
            pick("Split right", "Diviser à droite"),
            Some("Ctrl+Shift+E"),
            false,
        ),
        MoreMenuEntry::item(
            MoreMenuAction::SplitDown,
            pick("Split down", "Diviser vers le bas"),
            Some("Ctrl+Shift+O"),
            false,
        ),
    ];
    if state.can_arrange_panes {
        out.push(MoreMenuEntry::item(
            MoreMenuAction::ArrangeGrid,
            pick(
                "Arrange panes in a grid",
                "Organiser les panneaux en grille",
            ),
            None,
            false,
        ));
    }
    out.push(MoreMenuEntry::sep());
    out.push(MoreMenuEntry::item(
        MoreMenuAction::ToggleLeftSidebar,
        pick(
            "Toggle the left sidebar",
            "Afficher ou masquer la barre latérale gauche",
        ),
        None,
        state.left_open,
    ));
    out.push(MoreMenuEntry::item(
        MoreMenuAction::ToggleRightSidebar,
        pick(
            "Toggle the right sidebar",
            "Afficher ou masquer la barre latérale droite",
        ),
        None,
        state.right_open,
    ));
    out.push(MoreMenuEntry::sep());
    out.push(MoreMenuEntry {
        action: Some(MoreMenuAction::CloseActivePane),
        label: pick("Close active pane", "Fermer le panneau actif"),
        shortcut: Some("Ctrl+Shift+W"),
        checked: false,
        danger: true,
    });
    out
}

/// What the compact Live Share pill shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LiveSharePillState {
    /// Hosting: people connected and people waiting for an answer.
    Host { joined: usize, waiting: usize },
    /// Hosting, still starting (`retry_in_s` when a retry is scheduled).
    HostStarting {
        restarting: bool,
        retry_in_s: Option<u64>,
    },
    /// In someone's session (`reconnecting` while connecting or waiting for the host).
    Guest { host: String, reconnecting: bool },
}

fn host_or_question(host: &str) -> &str {
    let h = host.trim();
    if h.is_empty() { "?" } else { h }
}

/// Short pill text (`LiveSharePill.vue` `label`).
#[must_use]
pub fn live_share_pill_label(state: &LiveSharePillState, french: bool) -> String {
    match state {
        LiveSharePillState::Host { joined, .. } => joined.to_string(),
        LiveSharePillState::HostStarting {
            restarting,
            retry_in_s,
        } => match (retry_in_s, restarting, french) {
            (Some(s), _, false) if *s > 0 => format!("Retrying in {s} s"),
            (Some(s), _, true) if *s > 0 => format!("Nouvel essai dans {s} s"),
            (_, true, false) => "Restarting…".to_owned(),
            (_, true, true) => "Redémarrage…".to_owned(),
            (_, false, false) => "Starting…".to_owned(),
            (_, false, true) => "Démarrage…".to_owned(),
        },
        LiveSharePillState::Guest { host, reconnecting } => {
            let h = host_or_question(host);
            if *reconnecting {
                format!("{h}…")
            } else {
                h.to_owned()
            }
        }
    }
}

/// Amber waiting badge text (`waiting > 0` only).
#[must_use]
pub fn live_share_pill_badge(state: &LiveSharePillState) -> Option<String> {
    match state {
        LiveSharePillState::Host { waiting, .. } if *waiting > 0 => Some(waiting.to_string()),
        _ => None,
    }
}

/// `true` when the dot is red (hosting with someone connected).
#[must_use]
pub const fn live_share_pill_dot_live(state: &LiveSharePillState) -> bool {
    matches!(state, LiveSharePillState::Host { joined, .. } if *joined > 0)
}

/// Full words of the pill (tooltip and accessible label, `LiveSharePill.vue` `ariaLabel`).
#[must_use]
pub fn live_share_pill_tooltip(state: &LiveSharePillState, french: bool) -> String {
    match state {
        LiveSharePillState::Guest { host, .. } => {
            let h = host_or_question(host);
            if french {
                format!("Live Share : dans la session de {h}. Ouvrir les détails")
            } else {
                format!("Live Share: in {h}’s session. Open the details")
            }
        }
        LiveSharePillState::HostStarting { .. } => if french {
            "Live Share démarre. Ouvrir les détails"
        } else {
            "Live Share is starting. Open the details"
        }
        .to_owned(),
        LiveSharePillState::Host { joined, waiting } => {
            let base = match (french, *joined == 1) {
                (false, true) => {
                    "Live Share: sharing, 1 person connected. Open the details".to_owned()
                }
                (false, false) => {
                    format!("Live Share: sharing, {joined} people connected. Open the details")
                }
                (true, true) => {
                    "Live Share : partage en cours, 1 personne connectée. Ouvrir les détails"
                        .to_owned()
                }
                (true, false) => format!(
                    "Live Share : partage en cours, {joined} personnes connectées. Ouvrir les détails"
                ),
            };
            if *waiting > 0 {
                let badge = if french {
                    format!("{waiting} en attente de votre réponse")
                } else {
                    format!("{waiting} waiting for your answer")
                };
                format!("{base} ({badge})")
            } else {
                base
            }
        }
    }
}
