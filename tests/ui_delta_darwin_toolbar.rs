//! Darwin's 2026-11-08 toolbar & sidebar fixes (`b9061124`, `d5f3231e`, `cc1ec459`):
//! measured-room toolbar fit (`toolbarFit.js`), the `⋯` menu, and the compact Live Share pill.

#[path = "../src/toolbar_fit.rs"]
mod toolbar_fit;

use toolbar_fit::*;

#[test]
fn toolbar_fit_level_matches_toolbar_fit_spec() {
    // toolbarFit.spec.js cases: room for everything, room once the secondary buttons go under
    // "more", neither, unknown room, and the default centre.
    assert_eq!(
        toolbar_fit_level(1200.0, 400.0, 500.0, 200.0, 28.0, 60.0, 60.0),
        0
    );
    assert_eq!(
        toolbar_fit_level(900.0, 400.0, 500.0, 200.0, 28.0, 60.0, 60.0),
        1
    );
    assert_eq!(
        toolbar_fit_level(700.0, 400.0, 500.0, 200.0, 28.0, 60.0, 60.0),
        2
    );
    assert_eq!(
        toolbar_fit_level(0.0, 400.0, 500.0, 200.0, 28.0, 60.0, 60.0),
        0
    );
    assert_eq!(
        toolbar_fit_level(f32::NAN, 400.0, 500.0, 200.0, 28.0, 60.0, 60.0),
        0
    );
    // Exactly at the edge stays at the looser level.
    assert_eq!(
        toolbar_fit_level(960.0, 400.0, 500.0, 200.0, 28.0, 60.0, 60.0),
        0
    );
    assert_eq!(
        toolbar_fit_level(788.0, 400.0, 500.0, 200.0, 28.0, 60.0, 60.0),
        1
    );

    // A chip that comes (dev update pill, agent request) tightens the same window.
    let mut room = ToolbarRoom {
        avail: 1180.0,
        left: 420.0,
        right: 560.0,
        secondary: 220.0,
        version: 60.0,
    };
    assert_eq!(room.level(), 0);
    room.right += 160.0;
    assert_eq!(room.level(), 1);
    room.avail = 700.0;
    assert_eq!(room.level(), 2);
}

#[test]
fn more_menu_holds_the_hidden_actions_with_shortcuts_and_checks() {
    let entries = more_menu_entries(MoreMenuState {
        french: false,
        broadcast: true,
        floating_open: false,
        left_open: true,
        right_open: false,
        can_arrange_panes: true,
    });
    let actions: Vec<_> = entries.iter().filter_map(|e| e.action).collect();
    assert_eq!(
        actions,
        [
            MoreMenuAction::LiveShare,
            MoreMenuAction::GitHub,
            MoreMenuAction::Broadcast,
            MoreMenuAction::FloatingTerminal,
            MoreMenuAction::SplitRight,
            MoreMenuAction::SplitDown,
            MoreMenuAction::ArrangeGrid,
            MoreMenuAction::ToggleLeftSidebar,
            MoreMenuAction::ToggleRightSidebar,
            MoreMenuAction::CloseActivePane,
        ]
    );
    assert_eq!(entries.iter().filter(|e| e.is_separator()).count(), 4);
    let find = |a| entries.iter().find(|e| e.action == Some(a)).unwrap();
    assert!(find(MoreMenuAction::Broadcast).checked);
    assert_eq!(
        find(MoreMenuAction::Broadcast).shortcut,
        Some("Ctrl+Shift+B")
    );
    assert!(!find(MoreMenuAction::FloatingTerminal).checked);
    assert_eq!(
        find(MoreMenuAction::FloatingTerminal).shortcut,
        Some("Ctrl+`")
    );
    assert_eq!(
        find(MoreMenuAction::SplitRight).shortcut,
        Some("Ctrl+Shift+E")
    );
    assert_eq!(
        find(MoreMenuAction::SplitDown).shortcut,
        Some("Ctrl+Shift+O")
    );
    assert!(find(MoreMenuAction::ToggleLeftSidebar).checked);
    assert!(!find(MoreMenuAction::ToggleRightSidebar).checked);
    let close = find(MoreMenuAction::CloseActivePane);
    assert!(close.danger);
    assert_eq!(close.shortcut, Some("Ctrl+Shift+W"));

    let single = more_menu_entries(MoreMenuState {
        french: true,
        ..Default::default()
    });
    assert!(
        !single
            .iter()
            .any(|e| e.action == Some(MoreMenuAction::ArrangeGrid))
    );
    assert!(single.iter().any(|e| e.label == "Tickets et PR GitHub"));
    assert!(single.iter().any(|e| e.label == "Fermer le panneau actif"));
    assert_eq!(more_button_label(false), "More");
    assert_eq!(more_button_label(true), "Plus");
}

#[test]
fn live_share_pill_is_short_and_keeps_its_words_in_the_tooltip() {
    let host0 = LiveSharePillState::Host {
        joined: 0,
        waiting: 0,
    };
    assert_eq!(live_share_pill_label(&host0, false), "0");
    assert!(!live_share_pill_dot_live(&host0));
    assert_eq!(live_share_pill_badge(&host0), None);
    assert_eq!(
        live_share_pill_tooltip(&host0, false),
        "Live Share: sharing, 0 people connected. Open the details"
    );

    let host1 = LiveSharePillState::Host {
        joined: 1,
        waiting: 2,
    };
    assert_eq!(live_share_pill_label(&host1, false), "1");
    assert!(live_share_pill_dot_live(&host1));
    assert_eq!(live_share_pill_badge(&host1).as_deref(), Some("2"));
    assert_eq!(
        live_share_pill_tooltip(&host1, false),
        "Live Share: sharing, 1 person connected. Open the details (2 waiting for your answer)"
    );
    assert_eq!(
        live_share_pill_tooltip(&host1, true),
        "Live Share : partage en cours, 1 personne connectée. Ouvrir les détails (2 en attente de votre réponse)"
    );

    let starting = LiveSharePillState::HostStarting {
        restarting: false,
        retry_in_s: None,
    };
    assert_eq!(live_share_pill_label(&starting, false), "Starting…");
    let restarting = LiveSharePillState::HostStarting {
        restarting: true,
        retry_in_s: None,
    };
    assert_eq!(live_share_pill_label(&restarting, true), "Redémarrage…");
    let retry = LiveSharePillState::HostStarting {
        restarting: true,
        retry_in_s: Some(7),
    };
    assert_eq!(live_share_pill_label(&retry, false), "Retrying in 7 s");

    let guest = LiveSharePillState::Guest {
        host: "jeanc".into(),
        reconnecting: false,
    };
    assert_eq!(live_share_pill_label(&guest, false), "jeanc");
    assert_eq!(
        live_share_pill_tooltip(&guest, false),
        "Live Share: in jeanc’s session. Open the details"
    );
    let reconnecting = LiveSharePillState::Guest {
        host: "jeanc".into(),
        reconnecting: true,
    };
    assert_eq!(live_share_pill_label(&reconnecting, false), "jeanc…");
    let unknown = LiveSharePillState::Guest {
        host: "  ".into(),
        reconnecting: false,
    };
    assert_eq!(live_share_pill_label(&unknown, false), "?");
    assert_eq!(LIVE_SHARE_PILL_HEIGHT, 20.0);
    assert_eq!(LIVE_SHARE_PILL_DOT, 7.0);
}
