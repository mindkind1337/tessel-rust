//! Pure layout & localization helpers for the terminal `.exit-overlay` pill (`8982166c` / `a587e1b9`).

#![allow(dead_code)]

/// Returns `(message, primary_action_label, close_pane_label)` for the terminal exit / startup-error banner.
#[must_use]
pub fn terminal_exit_banner_copy(
    failed_to_start: bool,
    exit_code: Option<i32>,
    french: bool,
) -> (String, &'static str, &'static str) {
    if failed_to_start {
        if french {
            (
                "Ce terminal n'a pas pu démarrer.".into(),
                "Réessayer",
                "Fermer le panneau",
            )
        } else {
            (
                "This terminal couldn't start.".into(),
                "Retry",
                "Close pane",
            )
        }
    } else if french {
        let msg = match exit_code {
            Some(code) => format!("Le processus s'est terminé avec le code {code}."),
            None => "Le processus s'est terminé.".into(),
        };
        (msg, "Redémarrer", "Fermer le panneau")
    } else {
        let msg = match exit_code {
            Some(code) => format!("Process exited with code {code}."),
            None => "Process exited.".into(),
        };
        (msg, "Restart", "Close pane")
    }
}

/// Returns `true` when the `.exit-overlay` pill wraps its grouped `.exit-actions` container
/// (`[Restart/Retry]` + `[Close pane]`) together onto a second row in narrow panes.
#[must_use]
pub fn exit_overlay_actions_wrap_together(
    banner_inner_width_px: f32,
    msg_min_width_px: f32,
    actions_width_px: f32,
    gap_px: f32,
) -> bool {
    banner_inner_width_px < msg_min_width_px + gap_px + actions_width_px
}
