//! 1:1 `GPUI` Modals, Dialogs, and Popovers:
//! - `CommandPalette`, `FileFinder`, `LaunchMenu` (`gpui::canvas` 16×16 split icons), `HelpDialog`
//! - `LiveShare` (Host/Join modal, SAS knock verification modal, toolbar pill popover)
//! - `AddProjectDialog` (Local + nested Git repos, Clone, `git init`, SSH), `GitHubDialog`
//! - `NotesPanel` modal, `ReviewPanel` modal, `WorktreeCleanupModal`
//! - `Teams` manager + worker spawn confirmation + inter-project relay approval modals
//! - Toolbar `Usage` & `Notifications` popovers + StatusBar `Resources` (`gpui::canvas` sparkline),
//!   `Ports`, and `SSH` popovers + generic `prompt_modal` and `confirm_modal`

use super::{
    FocusInputTarget, TesselGpuiRoot,
    icons::GpuiIcon,
    side_panels_view::render_focus_input_box,
    theme::{GpuiThemeTokens, mono_font_family, rgba_with_alpha},
};
use crate::{
    app::gpui_bridge::{GpuiAction, GpuiModalsSnapshot, GpuiWorkspaceSnapshot},
    dialogs_toolbar_model::format_fix_failing_checks_prompt,
    dialogs_toolbar_ui::{AddProjectTab, DialogsToolbarAction, GitHubTab},
    finder_model::{LaunchPlacement, LauncherLang},
    share_panel::{DialogTab, ShareGpuiAction},
    side_panels_model::format_notes_for_agent,
    side_panels_ui::SidePanelsAction,
    status_strip::StatusBarItemId,
    teams_relay::RELAY_HOURLY_CHOICES,
    teams_ui::TeamsUiAction,
};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, PathBuilder,
    Rgba, SharedString, StatefulInteractiveElement, Styled, canvas, div, point,
    prelude::FluentBuilder as _, px,
};

fn rgb_tuple((r, g, b): (u8, u8, u8)) -> Rgba {
    gpui::rgb((u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_active_modals_and_popovers(
    snapshot: &GpuiWorkspaceSnapshot,
    search_inputs: &super::search_inputs::Inputs,
    modal_query: &super::modal_query_input::Input,
    task_card_input: &super::task_card_input::Inputs,
    panel_query_inputs: &super::panel_query_inputs::Inputs,
    focus_input: FocusInputTarget,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> Vec<AnyElement> {
    let mut layers = Vec::new();
    let m = &snapshot.modals;
    let french = snapshot.french;

    if m.launch_menu_open {
        layers.push(render_launch_menu_popover(snapshot, tokens, cx));
    }
    if m.share.pill_popover_open {
        layers.push(render_share_pill_popover(m, french, tokens, cx));
    }
    if m.usage_popover_open {
        layers.push(render_usage_popover(m, french, tokens, cx));
    }
    if snapshot.toolbar.more_menu_open {
        layers.push(render_toolbar_more_menu(snapshot, tokens, cx));
    }
    if m.notifications_popover_open {
        layers.push(render_notifications_popover(m, french, tokens, cx));
    }
    if let Some(item_id) = m.status_popover {
        layers.push(render_status_popover(snapshot, item_id, tokens, cx));
    }
    if m.palette_open {
        layers.push(render_command_palette(m, search_inputs, french, tokens, cx));
    }
    if m.finder.is_some() {
        layers.push(render_file_finder(m, search_inputs, french, tokens, cx));
    }
    if m.help_open {
        layers.push(render_help_modal(m, modal_query, french, tokens, cx));
    }
    if m.share_modal_open {
        layers.push(render_share_modal(
            snapshot,
            modal_query,
            focus_input,
            tokens,
            cx,
        ));
    }
    if m.add_project_open {
        layers.push(render_add_project_modal(m, focus_input, french, tokens, cx));
    }
    if m.github_open {
        layers.push(render_github_modal(m, focus_input, french, tokens, cx));
    }
    if m.notes_open {
        layers.push(render_notes_modal(m, focus_input, french, tokens, cx));
    }
    if m.sessions_open {
        layers.push(render_sessions_modal(
            snapshot,
            panel_query_inputs,
            tokens,
            cx,
        ));
    }
    if m.review_open {
        layers.push(render_review_modal(m, french, tokens, cx));
    }
    if m.worktree_cleanup.is_some() {
        layers.push(render_worktree_cleanup_modal(m, french, tokens, cx));
    }
    if m.teams_manager_project.is_some() {
        layers.push(render_teams_manager_modal(
            m,
            focus_input,
            french,
            tokens,
            cx,
        ));
    }
    if let Some(worker) = m.teams_pending_workers.first() {
        layers.push(render_worker_approval_modal(worker, french, tokens, cx));
    }
    if let Some(prompt) = m.teams_relay_prompts.first() {
        layers.push(render_relay_approval_modal(prompt, french, tokens, cx));
    }
    if let Some(ask) = &m.share.pending_ask {
        layers.push(render_share_ask_modal(ask, french, tokens, cx));
    }
    if let Some(pm) = &m.prompt_modal {
        layers.push(render_prompt_modal(pm, focus_input, french, tokens, cx));
    }
    if let Some(cm) = &m.confirm_modal {
        layers.push(render_confirm_modal(
            cm,
            snapshot
                .side_panels
                .task_board
                .pending_delete
                .as_ref()
                .map(|_| task_card_input),
            french,
            tokens,
            cx,
        ));
    }
    if m.dev_update_open {
        layers.push(render_dev_update_modal(m, french, tokens, cx));
    }

    layers
}

// ---------------------------------------------------------------------------
// 1. Command Palette (`Ctrl+Shift+P`)
// ---------------------------------------------------------------------------

fn render_command_palette(
    m: &GpuiModalsSnapshot,
    inputs: &super::search_inputs::Inputs,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let mut list = div()
        .id("gpui-palette-scroll")
        .max_h(px(360.0))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(2.0));

    for (idx, row) in m.palette_rows.iter().enumerate() {
        if let Some(hdr) = &row.header {
            list = list.child(
                div()
                    .pt(px(4.0))
                    .px(px(8.0))
                    .text_size(px(10.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(tokens.text_dim)
                    .child(SharedString::from(hdr.to_uppercase())),
            );
        }
        let selected = idx == m.palette_selected;
        let cmd_id = row.command.id.clone();
        let scope = inputs.scope(crate::app::gpui_bridge::NativeSearchKind::Palette);
        let query = m.palette_query.clone();
        list = list.child(
            div()
                .id(("gpui-palette-row", idx as u64))
                .px(px(10.0))
                .py(px(6.0))
                .rounded(px(tokens.radius_px - 1.0))
                .bg(if selected {
                    tokens.accent_soft
                } else {
                    gpui::rgba(0x0000_0000)
                })
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .hover(|s| s.bg(tokens.surface_2))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(scope) = scope {
                        super::search_inputs::choose(this, scope, &query, &cmd_id, cx);
                    }
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(12.0))
                        .text_color(tokens.text_strong)
                        .truncate()
                        .child(SharedString::from(row.command.title.clone())),
                )
                .when_some(row.command.shortcut.clone(), |el, sc| {
                    el.child(
                        div()
                            .px(px(5.0))
                            .py(px(1.5))
                            .rounded(px(3.0))
                            .bg(tokens.surface_2)
                            .font_family(mono_font_family())
                            .text_size(px(10.0))
                            .text_color(tokens.text_dim)
                            .child(SharedString::from(sc)),
                    )
                }),
        );
    }

    modal_backdrop(
        "gpui-palette-backdrop",
        tokens,
        GpuiAction::ToggleCommandPalette,
        div()
            .id("gpui-palette-card")
            .w(px(560.0))
            .p(px(10.0))
            .rounded(px(tokens.radius_px + 2.0))
            .bg(tokens.panel)
            .border_1()
            .border_color(tokens.border_strong)
            .flex()
            .flex_col()
            .gap(px(8.0))
            .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(
                inputs.render(
                    super::search_inputs::SearchView {
                        kind: crate::app::gpui_bridge::NativeSearchKind::Palette,
                        value: &m.palette_query,
                        selected: m
                            .palette_rows
                            .get(m.palette_selected)
                            .map(|r| r.command.id.as_str()),
                        placeholder: if french {
                            "Rechercher un panneau, projet ou commande…"
                        } else {
                            "Type a command, pane, or workspace…"
                        },
                        french,
                    },
                    tokens,
                    cx,
                ),
            )
            .child(list)
            .into_any_element(),
        cx,
    )
}

// ---------------------------------------------------------------------------
// 2. Fuzzy File Finder (`Ctrl+Shift+J`)
// ---------------------------------------------------------------------------

fn render_file_finder(
    m: &GpuiModalsSnapshot,
    inputs: &super::search_inputs::Inputs,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let Some(f) = &m.finder else {
        return div().into_any_element();
    };

    let mut list = div()
        .id("gpui-finder-scroll")
        .max_h(px(360.0))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(2.0));

    for (idx, rel) in f.results.iter().enumerate() {
        let sel = idx == f.selected;
        let scope = inputs.scope(crate::app::gpui_bridge::NativeSearchKind::Finder);
        let query = f.query.clone();
        let relative = rel.clone();
        list = list.child(
            div()
                .id(("gpui-finder-row", idx as u64))
                .px(px(10.0))
                .py(px(5.5))
                .rounded(px(tokens.radius_px - 1.0))
                .bg(if sel {
                    tokens.accent_soft
                } else {
                    gpui::rgba(0x0000_0000)
                })
                .font_family(mono_font_family())
                .text_size(px(11.5))
                .text_color(tokens.text_strong)
                .flex()
                .items_center()
                .gap(px(6.))
                .hover(|s| s.bg(tokens.surface_2))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(scope) = scope {
                        super::search_inputs::choose(this, scope, &query, &relative, cx);
                    }
                }))
                .child(super::file_icons::render(rel, tokens, tokens.text_dim))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(SharedString::from(rel.clone())),
                ),
        );
    }

    modal_backdrop(
        "gpui-finder-backdrop",
        tokens,
        GpuiAction::CloseOverlays,
        div()
            .id("gpui-finder-card")
            .w(px(560.0))
            .p(px(10.0))
            .rounded(px(tokens.radius_px + 2.0))
            .bg(tokens.panel)
            .border_1()
            .border_color(tokens.border_strong)
            .flex()
            .flex_col()
            .gap(px(8.0))
            .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(inputs.render(
                super::search_inputs::SearchView {
                    kind: crate::app::gpui_bridge::NativeSearchKind::Finder,
                    value: &f.query,
                    selected: f.results.get(f.selected).map(String::as_str),
                    placeholder: if french {
                        "Aller à un fichier du projet…"
                    } else {
                        "Jump to a project file…"
                    },
                    french,
                },
                tokens,
                cx,
            ))
            .child(list)
            .into_any_element(),
        cx,
    )
}

// ---------------------------------------------------------------------------
// 3. LaunchMenu Popover (`Ctrl+Shift+Space` + `gpui::canvas` 16×16 Split Icons)
// ---------------------------------------------------------------------------

fn render_launch_menu_popover(
    snapshot: &GpuiWorkspaceSnapshot,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let m = &snapshot.modals;
    let french = snapshot.french;
    let lang = if french {
        LauncherLang::Fr
    } else {
        LauncherLang::En
    };

    let mut placements_row = div().flex().items_center().gap(px(4.0));
    for pl in &m.launch_placements {
        let active = *pl == m.launch_placement;
        let p_copy = *pl;
        let stroke_col = if active {
            tokens.accent
        } else {
            tokens.text_dim
        };
        let split_icon = canvas(
            |_, _, _| (),
            move |bounds, (), window, _cx| {
                let ox = bounds.origin.x;
                let oy = bounds.origin.y;
                let mut border = PathBuilder::stroke(px(1.2));
                border.move_to(point(ox + px(2.0), oy + px(2.0)));
                border.line_to(point(ox + px(14.0), oy + px(2.0)));
                border.line_to(point(ox + px(14.0), oy + px(14.0)));
                border.line_to(point(ox + px(2.0), oy + px(14.0)));
                border.close();
                if let Ok(path) = border.build() {
                    window.paint_path(path, stroke_col);
                }
                let mut split_pb = PathBuilder::stroke(px(1.2));
                match p_copy {
                    LaunchPlacement::Auto => {
                        split_pb.move_to(point(ox + px(8.0), oy + px(2.0)));
                        split_pb.line_to(point(ox + px(8.0), oy + px(14.0)));
                        split_pb.move_to(point(ox + px(2.0), oy + px(8.0)));
                        split_pb.line_to(point(ox + px(14.0), oy + px(8.0)));
                    }
                    LaunchPlacement::Left | LaunchPlacement::Right => {
                        split_pb.move_to(point(ox + px(8.0), oy + px(2.0)));
                        split_pb.line_to(point(ox + px(8.0), oy + px(14.0)));
                    }
                    LaunchPlacement::Down => {
                        split_pb.move_to(point(ox + px(2.0), oy + px(8.0)));
                        split_pb.line_to(point(ox + px(14.0), oy + px(8.0)));
                    }
                }
                if let Ok(path) = split_pb.build() {
                    window.paint_path(path, stroke_col);
                }
            },
        )
        .w(px(16.0))
        .h(px(16.0));

        placements_row = placements_row.child(
            div()
                .id(SharedString::from(format!("gpui-lm-pl-{p_copy:?}")))
                .px(px(6.0))
                .py(px(3.0))
                .rounded(px(tokens.radius_px - 1.0))
                .bg(if active {
                    tokens.accent_soft
                } else {
                    tokens.surface
                })
                .border_1()
                .border_color(if active { tokens.accent } else { tokens.border })
                .flex()
                .items_center()
                .gap(px(4.0))
                .text_size(px(10.5))
                .text_color(tokens.text_strong)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.dispatch_action(GpuiAction::SetLaunchPlacement(p_copy), cx);
                }))
                .child(split_icon)
                .child(p_copy.label(lang)),
        );
    }

    let mut items = div().flex().flex_col().gap(px(3.0));
    items = items
        .child(launch_row_btn(
            "gpui-lm-shell",
            format!("Terminal ({})", snapshot.toolbar.preferred_shell_label),
            tokens,
            GpuiAction::LaunchFromMenu {
                agent_id: None,
                native_chat: None,
            },
            cx,
        ))
        .child(launch_row_btn(
            "gpui-lm-codex-chat",
            "Codex (Native Chat)".to_owned(),
            tokens,
            GpuiAction::LaunchFromMenu {
                agent_id: None,
                native_chat: Some("codex"),
            },
            cx,
        ))
        .child(launch_row_btn(
            "gpui-lm-claude-chat",
            "Claude (Native Chat)".to_owned(),
            tokens,
            GpuiAction::LaunchFromMenu {
                agent_id: None,
                native_chat: Some("claude"),
            },
            cx,
        ));

    for (aid, aname) in &snapshot.toolbar.installed_agents {
        let id_clone = aid.clone();
        items = items.child(launch_row_btn(
            format!("gpui-lm-agent-{aid}"),
            format!("{aname} (CLI)"),
            tokens,
            GpuiAction::LaunchFromMenu {
                agent_id: Some(id_clone),
                native_chat: None,
            },
            cx,
        ));
    }

    div()
        .id("gpui-launch-menu-backdrop")
        .absolute()
        .left(px(0.0))
        .top(px(0.0))
        .size_full()
        .on_click(cx.listener(|this, _, _, cx| {
            this.dispatch_action(GpuiAction::CloseOverlays, cx);
        }))
        .child(
            div()
                .id("gpui-launch-menu-card")
                .absolute()
                .left(px(280.0))
                .top(px(40.0))
                .w(px(320.0))
                .p(px(10.0))
                .rounded(px(tokens.radius_px + 2.0))
                .bg(tokens.panel)
                .border_1()
                .border_color(tokens.border_strong)
                .flex()
                .flex_col()
                .gap(px(6.0))
                .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
                .when(!m.launch_placements.is_empty(), |el| {
                    el.child(placements_row)
                })
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(tokens.text_dim)
                        .child(SharedString::from(m.launch_where_hint.clone())),
                )
                .child(modal_btn(
                    "gpui-lm-wt-toggle",
                    if m.launch_in_worktree {
                        "Isolate in new Git worktree: ON".to_owned()
                    } else {
                        "Isolate in new Git worktree: OFF".to_owned()
                    },
                    m.launch_in_worktree,
                    tokens,
                    GpuiAction::ToggleLaunchInWorktree,
                    cx,
                ))
                .child(items),
        )
        .into_any_element()
}

fn launch_row_btn(
    id: impl Into<SharedString>,
    label: String,
    tokens: &GpuiThemeTokens,
    action: GpuiAction,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let sid: SharedString = id.into();
    div()
        .id(sid)
        .px(px(8.0))
        .py(px(5.5))
        .rounded(px(tokens.radius_px - 1.0))
        .bg(tokens.surface)
        .text_size(px(11.5))
        .text_color(tokens.text_strong)
        .hover(|s| s.bg(tokens.accent_soft))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.dispatch_action(action.clone(), cx);
        }))
        .child(SharedString::from(label))
        .into_any_element()
}

// ---------------------------------------------------------------------------
// 4. Keyboard Shortcuts Help Dialog (`F1`)
// ---------------------------------------------------------------------------

fn render_help_modal(
    m: &GpuiModalsSnapshot,
    modal_query: &super::modal_query_input::Input,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let mut groups_col = div()
        .id("gpui-help-scroll")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(10.0));

    for grp in &m.help_groups {
        let mut g_box = div()
            .p(px(8.0))
            .rounded(px(tokens.radius_px))
            .bg(tokens.surface)
            .border_1()
            .border_color(tokens.border)
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .text_size(px(11.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(tokens.accent)
                    .child(grp.title),
            );
        for row in &grp.rows {
            let mut kbd_wrap = div().flex().items_center().gap(px(3.0));
            for chord in &row.chords {
                for &keycap in chord {
                    kbd_wrap = kbd_wrap.child(
                        div()
                            .px(px(5.0))
                            .py(px(1.5))
                            .rounded(px(3.0))
                            .bg(tokens.surface_2)
                            .border_1()
                            .border_color(tokens.border_strong)
                            .font_family(mono_font_family())
                            .text_size(px(10.0))
                            .text_color(tokens.text_strong)
                            .child(keycap),
                    );
                }
            }
            g_box = g_box.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(tokens.text)
                            .child(row.text),
                    )
                    .child(kbd_wrap),
            );
        }
        groups_col = groups_col.child(g_box);
    }

    modal_backdrop(
        "gpui-help-backdrop",
        tokens,
        GpuiAction::CloseOverlays,
        div()
            .id("gpui-help-card")
            .w(px(680.0))
            .h(px(520.0))
            .p(px(14.0))
            .rounded(px(tokens.radius_px + 3.0))
            .bg(tokens.panel)
            .border_1()
            .border_color(tokens.border_strong)
            .flex()
            .flex_col()
            .gap(px(10.0))
            .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(modal_header(
                if french {
                    "Raccourcis clavier"
                } else {
                    "Keyboard Shortcuts"
                },
                tokens,
                GpuiAction::CloseOverlays,
                cx,
            ))
            .child(modal_query.render(FocusInputTarget::HelpQuery, french, tokens, cx))
            .child(groups_col)
            .child(
                div()
                    .text_size(px(10.5))
                    .text_color(tokens.text_dim)
                    .child(SharedString::from(m.help_footer.clone())),
            )
            .into_any_element(),
        cx,
    )
}

// ---------------------------------------------------------------------------
// 5. Live Share Modal, SAS Knock Modal & Pill Popover
// ---------------------------------------------------------------------------

fn render_share_modal(
    snapshot: &GpuiWorkspaceSnapshot,
    modal_query: &super::modal_query_input::Input,
    focus_input: FocusInputTarget,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let sh = &snapshot.modals.share;
    let french = snapshot.french;

    let tabs = div()
        .flex()
        .gap(px(6.0))
        .child(modal_btn(
            "gpui-sh-tab-host",
            if french { "Partager" } else { "Host" }.to_owned(),
            sh.tab == DialogTab::Share,
            tokens,
            GpuiAction::Share(ShareGpuiAction::SetTab(DialogTab::Share)),
            cx,
        ))
        .child(modal_btn(
            "gpui-sh-tab-join",
            if french { "Rejoindre" } else { "Join" }.to_owned(),
            sh.tab == DialogTab::Join,
            tokens,
            GpuiAction::Share(ShareGpuiAction::SetTab(DialogTab::Join)),
            cx,
        ));

    let body = if sh.tab == DialogTab::Share {
        let mut col = div().flex().flex_col().gap(px(8.0)).child(
            div()
                .flex()
                .gap(px(6.0))
                .child(modal_btn(
                    "gpui-sh-toggle-host",
                    if sh.hosting {
                        "Stop Hosting".to_owned()
                    } else {
                        "Start Live Share Host".to_owned()
                    },
                    sh.hosting,
                    tokens,
                    GpuiAction::Share(if sh.hosting {
                        ShareGpuiAction::StopHosting
                    } else {
                        ShareGpuiAction::StartHosting
                    }),
                    cx,
                ))
                .when(sh.hosting, |el| {
                    el.child(modal_btn(
                        "gpui-sh-new-invite",
                        "+ Create Invite Link".to_owned(),
                        false,
                        tokens,
                        GpuiAction::Share(ShareGpuiAction::CreateInvite),
                        cx,
                    ))
                })
                .child(modal_btn(
                    "gpui-sh-knock",
                    if sh.require_knock {
                        "SAS Knock Approval: ON".to_owned()
                    } else {
                        "SAS Knock Approval: OFF".to_owned()
                    },
                    sh.require_knock,
                    tokens,
                    GpuiAction::Share(ShareGpuiAction::ToggleRequireKnock),
                    cx,
                )),
        );

        for inv in &sh.invites {
            let rev_id = inv.id.clone();
            let copy_url = inv.token_url.clone();
            col = col.child(
                div()
                    .p(px(7.0))
                    .rounded(px(tokens.radius_px))
                    .bg(tokens.surface)
                    .border_1()
                    .border_color(tokens.border)
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(6.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_family(mono_font_family())
                            .text_size(px(10.5))
                            .text_color(tokens.text_strong)
                            .truncate()
                            .child(SharedString::from(format!(
                                "{} ({}) — {}",
                                inv.label,
                                inv.status.badge_label(french),
                                inv.token_url
                            ))),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(4.0))
                            .child(modal_btn(
                                format!("gpui-sh-copy-{}", inv.id),
                                "Copy".to_owned(),
                                false,
                                tokens,
                                GpuiAction::SidePanels(SidePanelsAction::CopyText(copy_url)),
                                cx,
                            ))
                            .child(modal_btn(
                                format!("gpui-sh-rev-{}", inv.id),
                                "Revoke".to_owned(),
                                false,
                                tokens,
                                GpuiAction::Share(ShareGpuiAction::RevokeInvite(rev_id)),
                                cx,
                            )),
                    ),
            );
        }

        for g in &sh.guests {
            let gid = g.id.clone();
            col = col.child(
                div()
                    .p(px(6.0))
                    .rounded(px(tokens.radius_px))
                    .bg(tokens.surface)
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(tokens.text_strong)
                            .child(SharedString::from(format!("{} ({})", g.name, g.initials))),
                    )
                    .child(modal_btn(
                        format!("gpui-sh-kick-{}", g.id),
                        "Disconnect".to_owned(),
                        false,
                        tokens,
                        GpuiAction::Share(ShareGpuiAction::KickGuest(gid)),
                        cx,
                    )),
            );
        }
        col.into_any_element()
    } else {
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(render_focus_input_box(
                "gpui-sh-join-code",
                FocusInputTarget::LiveShareJoinCode,
                focus_input == FocusInputTarget::LiveShareJoinCode,
                &sh.join_code,
                "tessel://join/... or invite token",
                tokens,
                cx,
            ))
            .child(
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(modal_btn(
                        "gpui-sh-join-btn",
                        "Join Session".to_owned(),
                        true,
                        tokens,
                        GpuiAction::Share(ShareGpuiAction::JoinSession),
                        cx,
                    ))
                    .when(sh.guest_active, |el| {
                        el.child(modal_btn(
                            "gpui-sh-follow",
                            if sh.follow_host {
                                "Following Host".to_owned()
                            } else {
                                "Follow Host".to_owned()
                            },
                            sh.follow_host,
                            tokens,
                            GpuiAction::Share(ShareGpuiAction::ToggleFollowHost),
                            cx,
                        ))
                        .child(modal_btn(
                            "gpui-sh-leave",
                            "Leave".to_owned(),
                            false,
                            tokens,
                            GpuiAction::Share(ShareGpuiAction::DisconnectGuest),
                            cx,
                        ))
                    }),
            )
            .when_some(sh.guest_status.clone(), |el, st| {
                el.child(
                    div()
                        .text_size(px(11.5))
                        .text_color(tokens.accent)
                        .child(SharedString::from(st)),
                )
            })
            .when(
                sh.guest_board.scope.is_none() && !sh.guest_board.draft.is_empty(),
                |el| {
                    el.child(div().text_color(tokens.warn).child(if french {
                        "Brouillon conservé de la session précédente"
                    } else {
                        "Retained draft from the previous session"
                    }))
                    .child(
                        div()
                            .id("guest-recovery-draft")
                            .max_h(px(80.))
                            .overflow_y_scroll()
                            .child(sh.guest_board.draft.clone()),
                    )
                    .child(modal_btn(
                        "guest-copy-draft",
                        if french {
                            "Copier le brouillon"
                        } else {
                            "Copy draft"
                        }
                        .into(),
                        false,
                        tokens,
                        GpuiAction::SidePanels(SidePanelsAction::CopyText(
                            sh.guest_board.draft.clone(),
                        )),
                        cx,
                    ))
                    .when_some(
                        sh.guest_board.notice.as_ref(),
                        |el, notice| {
                            el.child(div().text_color(tokens.warn).child(notice.text(french)))
                        },
                    )
                },
            )
            .when_some(sh.guest_board.scope, |el, scope| {
                let board = &sh.guest_board;
                let mut content = div()
                    .id("gpui-guest-tasks")
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .max_h(px(340.))
                    .overflow_y_scroll();
                let mut projects = div().flex().flex_wrap().gap(px(4.));
                for (id, name) in &sh.guest_projects {
                    projects = projects.child(modal_btn(
                        format!("guest-project-{id}"),
                        name.clone(),
                        *id == scope.project,
                        tokens,
                        GpuiAction::Share(ShareGpuiAction::GuestProject {
                            session: scope.session,
                            project: *id,
                        }),
                        cx,
                    ));
                }
                for (files, id, label) in [
                    (true, "files", if french { "Fichiers" } else { "Files" }),
                    (false, "tasks", if french { "Tâches" } else { "Tasks" }),
                ] {
                    projects = projects.child(modal_btn(
                        format!("guest-tab-{id}"),
                        label.into(),
                        files == sh.guest_files.is_some(),
                        tokens,
                        GpuiAction::Share(ShareGpuiAction::GuestFilesTab(scope, files)),
                        cx,
                    ));
                }
                if let Some(files) = &sh.guest_files {
                    content = content.child(projects);
                    if !files.busy {
                        let dir = &files.directory;
                        if !dir.is_empty() {
                            content = content.child(modal_btn(
                                "guest-parent",
                                if french {
                                    "Dossier parent"
                                } else {
                                    "Parent folder"
                                }
                                .into(),
                                false,
                                tokens,
                                GpuiAction::Share(ShareGpuiAction::GuestDirectory(
                                    scope,
                                    dir.rsplit_once('/').map_or("", |(p, _)| p).into(),
                                )),
                                cx,
                            ));
                        }
                        content = content.child(modal_btn(
                            "guest-refresh-dir",
                            if french { "Actualiser" } else { "Refresh" }.into(),
                            false,
                            tokens,
                            GpuiAction::Share(ShareGpuiAction::GuestDirectory(scope, dir.clone())),
                            cx,
                        ));
                    } else {
                        content = content.child(if french {
                            "Chargement…"
                        } else {
                            "Loading…"
                        });
                    }
                    for (index, entry) in files.entries.iter().enumerate() {
                        let action = if entry.directory {
                            ShareGpuiAction::GuestDirectory(scope, entry.path.clone())
                        } else {
                            ShareGpuiAction::GuestOpenFile(crate::guest_file_editor::Key {
                                scope,
                                path: entry.path.clone(),
                            })
                        };
                        let label = div()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .when_else(
                                entry.directory,
                                |el| el.child(GpuiIcon::ChevronRight.render(12., tokens.text_dim)),
                                |el| {
                                    el.child(
                                        super::file_icons::render(
                                            &entry.name,
                                            tokens,
                                            tokens.text_dim,
                                        )
                                        .debug_selector(move || format!("guest-file-icon-{index}")),
                                    )
                                },
                            )
                            .child(div().min_w_0().truncate().child(entry.name.clone()));
                        let button = modal_btn_content(
                            format!("guest-entry-{index}"),
                            label,
                            false,
                            tokens,
                            GpuiAction::Share(action),
                            cx,
                        );
                        if entry.directory {
                            content = content.child(button);
                        } else {
                            let key = crate::guest_file_editor::Key {
                                scope,
                                path: entry.path.clone(),
                            };
                            let menu =
                                GpuiAction::Share(ShareGpuiAction::GuestFileMenu(key.clone()));
                            let right_click = menu.clone();
                            content = content.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .on_mouse_down(
                                        gpui::MouseButton::Right,
                                        cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.dispatch_action(right_click.clone(), cx);
                                        }),
                                    )
                                    .child(button)
                                    .child(modal_btn(
                                        format!("guest-file-menu-{index}"),
                                        "⋯".into(),
                                        false,
                                        tokens,
                                        menu,
                                        cx,
                                    )),
                            );
                            if files.download_menu.as_ref() == Some(&key) {
                                content = content.child(modal_btn(
                                    format!("guest-download-{index}"),
                                    if french {
                                        "Télécharger…"
                                    } else {
                                        "Download…"
                                    }
                                    .into(),
                                    false,
                                    tokens,
                                    GpuiAction::Share(ShareGpuiAction::GuestDownload(key)),
                                    cx,
                                ));
                            }
                        }
                    }
                    if let Some((key, pixels)) = &files.image {
                        let size = pixels.size(0);
                        let scale = (600.0f32 / size.width.0 as f32)
                            .min(270. / size.height.0 as f32)
                            .min(1.);
                        let action =
                            GpuiAction::Share(ShareGpuiAction::GuestImageMenu(key.clone()));
                        content = content.child(key.path.clone()).child(
                            div()
                                .id("guest-image-preview")
                                .debug_selector(|| "guest-image-preview".into())
                                .w(px(scale * size.width.0 as f32))
                                .h(px(scale * size.height.0 as f32))
                                .on_mouse_down(
                                    gpui::MouseButton::Right,
                                    cx.listener(move |root, _, _, cx| {
                                        cx.stop_propagation();
                                        root.dispatch_action(action.clone(), cx);
                                    }),
                                )
                                .child(gpui::img(pixels.clone()).size_full()),
                        );
                        if files.image_menu {
                            content = content.child(modal_btn(
                                "guest-image-download",
                                if french {
                                    "Télécharger l’original…"
                                } else {
                                    "Download original…"
                                }
                                .into(),
                                false,
                                tokens,
                                GpuiAction::Share(ShareGpuiAction::GuestDownload(key.clone())),
                                cx,
                            ));
                        }
                    }
                    if let Some(doc) = &files.document {
                        let (preview, partial) = doc.preview();
                        content = content
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(doc.key.path.clone()),
                            )
                            .child(
                                div()
                                    .id("guest-file-preview")
                                    .max_h(px(160.))
                                    .overflow_y_scroll()
                                    .child(preview.to_owned()),
                            );
                        if partial {
                            content = content.child(if french {
                                "Aperçu partiel (64 Kio)."
                            } else {
                                "Partial preview (64 KiB)."
                            });
                        }
                        if !files.busy {
                            content = content.child(modal_btn(
                                "guest-history-refresh",
                                if french { "Chronologie" } else { "Timeline" }.into(),
                                false,
                                tokens,
                                GpuiAction::Share(ShareGpuiAction::GuestHistory(
                                    doc.key.clone(),
                                    false,
                                )),
                                cx,
                            ));
                        }
                        if !files.busy {
                            content = content.child(modal_btn(
                                "guest-versions",
                                if french {
                                    "Historique local"
                                } else {
                                    "Local History"
                                }
                                .into(),
                                false,
                                tokens,
                                GpuiAction::Share(ShareGpuiAction::GuestVersion(
                                    doc.key.clone(),
                                    None,
                                )),
                                cx,
                            ));
                        }
                        if let Some(rows) = &files.versions {
                            let mut list = div()
                                .id("guest-version-list")
                                .max_h(px(140.))
                                .overflow_y_scroll();
                            if rows.is_empty() {
                                list = list.child(if french {
                                    "Aucune sauvegarde."
                                } else {
                                    "No saved versions."
                                });
                            }
                            for (index, row) in rows.iter().enumerate() {
                                list = list.child(modal_btn(
                                    format!("guest-version-{index}"),
                                    row.label(),
                                    false,
                                    tokens,
                                    GpuiAction::Share(ShareGpuiAction::GuestVersion(
                                        doc.key.clone(),
                                        Some(row.id.clone()),
                                    )),
                                    cx,
                                ));
                            }
                            content = content.child(list);
                        }
                        if let Some(patch) = &files.version_diff {
                            let mut section = div()
                                .debug_selector(|| "guest-saved-diff".into())
                                .child(crate::share_panel::guest_versions::comparison_title(
                                    french,
                                    patch.line_count() == 0,
                                ));
                            if patch.line_count() > 0 {
                                let patch = patch.clone();
                                section = section.child(
                                    gpui::uniform_list(
                                        "guest-saved-diff-lines",
                                        patch.line_count(),
                                        cx.processor(
                                            move |_, rows: std::ops::Range<usize>, _, _| {
                                                rows.map(|i| {
                                                    div()
                                                        .font_family(
                                                            super::theme::mono_font_family(),
                                                        )
                                                        .child(patch.line(i).to_owned())
                                                })
                                                .collect::<Vec<_>>()
                                            },
                                        ),
                                    )
                                    .h(px(180.)),
                                );
                            }
                            content = content.child(section);
                        }
                        if let Some(text) = &files.version_text {
                            content = content.child(
                                div()
                                    .id("guest-version-text")
                                    .debug_selector(|| "guest-version-text".into())
                                    .max_h(px(180.))
                                    .overflow_y_scroll()
                                    .child(text.as_ref().clone()),
                            );
                        }
                        if let Some(page) = &files.history {
                            if page.uncommitted {
                                content = content.child(if french {
                                    "Modifications non commitées"
                                } else {
                                    "Uncommitted Changes"
                                });
                            }
                            if !page.uncommitted && page.items.is_empty() {
                                content = content.child(if french {
                                    "Aucun historique."
                                } else {
                                    "No history."
                                });
                            }
                            for row in &page.items {
                                let date = crate::regional_date::long_local(row.time, french)
                                    .unwrap_or_default();
                                content = content
                                    .child(
                                        div()
                                            .debug_selector(|| "guest-history-commit".into())
                                            .child(format!(
                                                "{} · {} · {}",
                                                &row.id[..7],
                                                row.author,
                                                date
                                            )),
                                    )
                                    .child(div().child(row.message.clone()));
                            }
                            if files.more && !files.busy {
                                content = content.child(modal_btn(
                                    "guest-history-more",
                                    if french {
                                        "Charger la suite"
                                    } else {
                                        "Load More"
                                    }
                                    .into(),
                                    false,
                                    tokens,
                                    GpuiAction::Share(ShareGpuiAction::GuestHistory(
                                        doc.key.clone(),
                                        true,
                                    )),
                                    cx,
                                ));
                            }
                        }
                    }
                    for notice in &files.notices {
                        content =
                            content.child(div().text_color(tokens.warn).child(notice.clone()));
                    }
                    return el.child(content);
                }
                content = content.child(projects).child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(if french { "Tâches" } else { "Tasks" }),
                );
                if !board.pending {
                    content = content.child(modal_btn(
                        "guest-tasks-refresh",
                        if french { "Actualiser" } else { "Refresh" }.into(),
                        false,
                        tokens,
                        GpuiAction::Share(ShareGpuiAction::GuestTask {
                            scope,
                            action: crate::guest_task_board::Action::Refresh,
                        }),
                        cx,
                    ));
                } else {
                    content = content.child(div().child(if french {
                        "Requête en cours…"
                    } else {
                        "Request pending…"
                    }));
                }
                if board.limited {
                    content = content.child(div().text_color(tokens.warn).child(if french {
                        "Affichage partiel du tableau."
                    } else {
                        "Partial task board."
                    }));
                }
                let columns = if french {
                    ["À faire", "En cours", "À examiner", "Terminé"]
                } else {
                    ["To do", "Doing", "Review", "Done"]
                };
                for (column, label) in columns.iter().enumerate() {
                    content = content.child(
                        div()
                            .text_size(px(11.))
                            .text_color(tokens.text_dim)
                            .child(*label),
                    );
                    for card in board.cards.iter().filter(|c| c.column == column) {
                        content = content.child(
                            div()
                                .id(("guest-task", card.id))
                                .px(px(8.))
                                .py(px(5.))
                                .bg(tokens.surface_2)
                                .rounded(px(4.))
                                .child(SharedString::from(card.title.clone()))
                                .when_some(card.guest_by.clone(), |el, name| {
                                    el.child(
                                        div()
                                            .text_size(px(10.))
                                            .text_color(tokens.text_dim)
                                            .child(name),
                                    )
                                }),
                        );
                    }
                }
                if board.writable {
                    content = content.child(modal_query.render(
                        FocusInputTarget::GuestTaskDraft(scope),
                        french,
                        tokens,
                        cx,
                    ));
                    if !board.pending && !board.draft_over_limit && !board.draft.trim().is_empty() {
                        content = content.child(modal_btn(
                            "guest-task-add",
                            if french { "Ajouter" } else { "Add" }.into(),
                            true,
                            tokens,
                            GpuiAction::Share(ShareGpuiAction::GuestTask {
                                scope,
                                action: crate::guest_task_board::Action::Add,
                            }),
                            cx,
                        ));
                    }
                }
                if let Some(notice) = &board.notice {
                    content =
                        content.child(div().text_color(tokens.warn).child(notice.text(french)));
                }
                el.child(content)
            })
            .into_any_element()
    };

    modal_backdrop(
        "gpui-share-backdrop",
        tokens,
        GpuiAction::CloseOverlays,
        div()
            .id("gpui-share-card")
            .w(px(620.0))
            .p(px(14.0))
            .rounded(px(tokens.radius_px + 3.0))
            .bg(tokens.panel)
            .border_1()
            .border_color(tokens.border_strong)
            .flex()
            .flex_col()
            .gap(px(10.0))
            .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(modal_header(
                "Live Share (X25519 + ChaCha20-Poly1305 E2EE)",
                tokens,
                GpuiAction::CloseOverlays,
                cx,
            ))
            .child(tabs)
            .child(body)
            .when(!sh.error.is_empty(), |el| {
                el.child(
                    div()
                        .text_size(px(11.0))
                        .text_color(tokens.danger)
                        .child(SharedString::from(sh.error.clone())),
                )
            })
            .into_any_element(),
        cx,
    )
}

fn render_share_ask_modal(
    ask: &crate::share_panel::ShareGpuiAskModal,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let ask_key_close = ask.ask_key.clone();
    let ask_key_deny = ask.ask_key.clone();
    let ask_key_allow = ask.ask_key.clone();
    let checked = ask.code_checked;
    modal_backdrop(
        "gpui-share-ask-backdrop",
        tokens,
        GpuiAction::Share(ShareGpuiAction::AnswerAsk {
            ask_key: ask_key_close,
            allow: false,
        }),
        div()
            .id("gpui-share-ask-card")
            .w(px(480.0))
            .p(px(14.0))
            .rounded(px(tokens.radius_px + 3.0))
            .bg(tokens.panel)
            .border_1()
            .border_color(tokens.accent)
            .flex()
            .flex_col()
            .gap(px(10.0))
            .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(tokens.text_strong)
                    .child(SharedString::from(if french {
                        format!("{} souhaite rejoindre la session", ask.guest_name)
                    } else {
                        format!("{} wants to join your Live Share", ask.guest_name)
                    })),
            )
            .child(
                div()
                    .p(px(8.0))
                    .rounded(px(tokens.radius_px))
                    .bg(tokens.surface)
                    .font_family(mono_font_family())
                    .text_size(px(14.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(tokens.accent)
                    .child(SharedString::from(format!("SAS Code: {}", ask.sas_code))),
            )
            .child(modal_btn(
                "gpui-ask-check-sas",
                format!(
                    "[{}] Confirm SAS code matches guest's screen",
                    if checked { "✓" } else { " " }
                ),
                checked,
                tokens,
                GpuiAction::Share(ShareGpuiAction::ToggleAskCodeChecked),
                cx,
            ))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(6.0))
                    .child(modal_btn(
                        "gpui-ask-deny",
                        "Turn away".to_owned(),
                        false,
                        tokens,
                        GpuiAction::Share(ShareGpuiAction::AnswerAsk {
                            ask_key: ask_key_deny,
                            allow: false,
                        }),
                        cx,
                    ))
                    .when(checked, |el| {
                        el.child(modal_btn(
                            "gpui-ask-allow",
                            "Admit guest".to_owned(),
                            true,
                            tokens,
                            GpuiAction::Share(ShareGpuiAction::AnswerAsk {
                                ask_key: ask_key_allow,
                                allow: true,
                            }),
                            cx,
                        ))
                    }),
            )
            .into_any_element(),
        cx,
    )
}

fn render_share_pill_popover(
    m: &GpuiModalsSnapshot,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let sh = &m.share;
    popover_card(
        "gpui-share-pill-pop",
        160.0,
        40.0,
        300.0,
        tokens,
        GpuiAction::Share(ShareGpuiAction::ClosePillPopover),
        div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(tokens.text_strong)
                    .child(SharedString::from(
                        sh.pill_label
                            .clone()
                            .unwrap_or_else(|| "Live Share".to_owned()),
                    )),
            )
            .child(
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(modal_btn(
                        "gpui-sh-pop-open",
                        if french { "Ouvrir" } else { "Manage" }.to_owned(),
                        true,
                        tokens,
                        GpuiAction::OpenLiveShare,
                        cx,
                    ))
                    .when(sh.hosting, |el| {
                        el.child(modal_btn(
                            "gpui-sh-pop-stop",
                            "Stop".to_owned(),
                            false,
                            tokens,
                            GpuiAction::Share(ShareGpuiAction::StopHosting),
                            cx,
                        ))
                    }),
            )
            .into_any_element(),
        cx,
    )
}

// ---------------------------------------------------------------------------
// 6. AddProjectDialog (`Ctrl+Shift+N`)
// ---------------------------------------------------------------------------

fn render_add_project_modal(
    m: &GpuiModalsSnapshot,
    focus_input: FocusInputTarget,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let tabs = div()
        .flex()
        .flex_wrap()
        .gap(px(5.0))
        .child(modal_btn(
            "gpui-ap-tab-local",
            if french { "Dossier" } else { "Open Folder" }.to_owned(),
            m.add_project_tab == AddProjectTab::LocalFolder,
            tokens,
            GpuiAction::SetAddProjectTab(AddProjectTab::LocalFolder),
            cx,
        ))
        .child(modal_btn(
            "gpui-ap-tab-clone",
            "Git Clone".to_owned(),
            m.add_project_tab == AddProjectTab::CloneRepo,
            tokens,
            GpuiAction::SetAddProjectTab(AddProjectTab::CloneRepo),
            cx,
        ))
        .child(modal_btn(
            "gpui-ap-tab-init",
            "New Repo (git init)".to_owned(),
            m.add_project_tab == AddProjectTab::NewProject,
            tokens,
            GpuiAction::SetAddProjectTab(AddProjectTab::NewProject),
            cx,
        ))
        .child(modal_btn(
            "gpui-ap-tab-ssh",
            "SSH Remote".to_owned(),
            m.add_project_tab == AddProjectTab::SshRemote,
            tokens,
            GpuiAction::SetAddProjectTab(AddProjectTab::SshRemote),
            cx,
        ));

    let body = match m.add_project_tab {
        AddProjectTab::LocalFolder => {
            let mut col = div().flex().flex_col().gap(px(8.0)).child(modal_btn(
                "gpui-ap-pick-local",
                if let Some(p) = &m.add_picked_folder {
                    format!("Folder: {}", p.display())
                } else {
                    "Choose Folder…".to_owned()
                },
                false,
                tokens,
                GpuiAction::DialogsToolbar(DialogsToolbarAction::PickLocalProjectFolder),
                cx,
            ));
            for (idx, (repo_path, checked)) in m.add_nested_repos.iter().enumerate() {
                let rp = repo_path.clone();
                col = col.child(modal_btn(
                    format!("gpui-ap-nested-{idx}"),
                    format!(
                        "[{}] {}",
                        if *checked { "✓" } else { " " },
                        repo_path.display()
                    ),
                    *checked,
                    tokens,
                    GpuiAction::ToggleNestedRepo(rp),
                    cx,
                ));
            }
            if let Some(picked) = &m.add_picked_folder {
                let selected: Vec<_> = m
                    .add_nested_repos
                    .iter()
                    .filter_map(|(p, c)| (*c).then_some(p.clone()))
                    .collect();
                let paths = if selected.is_empty() {
                    vec![picked.clone()]
                } else {
                    selected
                };
                col = col.child(modal_btn(
                    "gpui-ap-add-submit",
                    "Add to Tessel".to_owned(),
                    true,
                    tokens,
                    GpuiAction::DialogsToolbar(DialogsToolbarAction::AddProjects(paths)),
                    cx,
                ));
            }
            col.into_any_element()
        }
        AddProjectTab::CloneRepo => {
            let url = m.add_clone_url.clone();
            let parent = m.add_clone_parent.clone();
            let fname = m.add_folder_name.clone();
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(render_focus_input_box(
                    "gpui-ap-clone-url",
                    FocusInputTarget::AddProjectCloneUrl,
                    focus_input == FocusInputTarget::AddProjectCloneUrl,
                    &m.add_clone_url,
                    "https://github.com/owner/repo.git",
                    tokens,
                    cx,
                ))
                .child(render_focus_input_box(
                    "gpui-ap-clone-fname",
                    FocusInputTarget::AddProjectFolderName,
                    focus_input == FocusInputTarget::AddProjectFolderName,
                    &m.add_folder_name,
                    "Target folder name…",
                    tokens,
                    cx,
                ))
                .child(modal_btn(
                    "gpui-ap-clone-submit",
                    "Clone & Open".to_owned(),
                    true,
                    tokens,
                    GpuiAction::DialogsToolbar(DialogsToolbarAction::CloneRepository {
                        url,
                        parent_dir: parent,
                        folder_name: fname,
                    }),
                    cx,
                ))
                .into_any_element()
        }
        AddProjectTab::NewProject => {
            let parent = m.add_clone_parent.clone();
            let fname = m.add_folder_name.clone();
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(render_focus_input_box(
                    "gpui-ap-init-fname",
                    FocusInputTarget::AddProjectFolderName,
                    focus_input == FocusInputTarget::AddProjectFolderName,
                    &m.add_folder_name,
                    "New project folder name…",
                    tokens,
                    cx,
                ))
                .child(modal_btn(
                    "gpui-ap-init-submit",
                    "Initialize Git Repository".to_owned(),
                    true,
                    tokens,
                    GpuiAction::DialogsToolbar(DialogsToolbarAction::InitNewProject {
                        parent_dir: parent,
                        folder_name: fname,
                    }),
                    cx,
                ))
                .into_any_element()
        }
        AddProjectTab::SshRemote => {
            let mut aliases = div().flex().flex_wrap().gap(px(4.0));
            for (idx, al) in m.add_ssh_aliases.iter().enumerate() {
                let a_clone = al.clone();
                aliases = aliases.child(modal_btn(
                    format!("gpui-ap-ssh-al-{idx}"),
                    al.clone(),
                    m.add_ssh_selected == *al,
                    tokens,
                    GpuiAction::SetAddProjectSshAlias(a_clone),
                    cx,
                ));
            }
            let alias = m.add_ssh_selected.clone();
            let rdir = m.add_ssh_remote_dir.clone();
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(aliases)
                .child(render_focus_input_box(
                    "gpui-ap-ssh-dir",
                    FocusInputTarget::AddProjectFolderName,
                    focus_input == FocusInputTarget::AddProjectFolderName,
                    &m.add_ssh_remote_dir,
                    "~/project…",
                    tokens,
                    cx,
                ))
                .child(modal_btn(
                    "gpui-ap-ssh-open",
                    "Open SSH Workspace".to_owned(),
                    true,
                    tokens,
                    GpuiAction::DialogsToolbar(DialogsToolbarAction::OpenSshWorkspace {
                        host_alias: alias,
                        remote_dir: rdir,
                    }),
                    cx,
                ))
                .into_any_element()
        }
    };

    modal_backdrop(
        "gpui-add-proj-backdrop",
        tokens,
        GpuiAction::DialogsToolbar(DialogsToolbarAction::CloseAddProjectModal),
        div()
            .id("gpui-add-proj-card")
            .w(px(512.0))
            .p(px(14.0))
            .rounded(px(tokens.radius_px + 3.0))
            .bg(tokens.panel)
            .border_1()
            .border_color(tokens.border_strong)
            .flex()
            .flex_col()
            .gap(px(10.0))
            .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(modal_header(
                if french {
                    "Ajouter un espace de travail"
                } else {
                    "Add Workspace Project"
                },
                tokens,
                GpuiAction::DialogsToolbar(DialogsToolbarAction::CloseAddProjectModal),
                cx,
            ))
            .child(tabs)
            .child(body)
            .when(!m.add_error.is_empty(), |el| {
                el.child(
                    div()
                        .text_size(px(11.0))
                        .text_color(tokens.danger)
                        .child(SharedString::from(m.add_error.clone())),
                )
            })
            .into_any_element(),
        cx,
    )
}

// ---------------------------------------------------------------------------
// 7. GitHubDialog (`PRs`, `Issues`, CI Check Rollup & `Fix failing checks`)
// ---------------------------------------------------------------------------

fn render_github_modal(
    m: &GpuiModalsSnapshot,
    focus_input: FocusInputTarget,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let top = div()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .gap(px(5.0))
                .child(modal_btn(
                    "gpui-gh-tab-prs",
                    format!("Pull Requests ({})", m.github_prs.len()),
                    m.github_tab == GitHubTab::PullRequests,
                    tokens,
                    GpuiAction::SetGitHubTab(GitHubTab::PullRequests),
                    cx,
                ))
                .child(modal_btn(
                    "gpui-gh-tab-issues",
                    format!("Issues ({})", m.github_issues.len()),
                    m.github_tab == GitHubTab::Issues,
                    tokens,
                    GpuiAction::SetGitHubTab(GitHubTab::Issues),
                    cx,
                )),
        )
        .child(modal_btn(
            "gpui-gh-refresh",
            "Refresh".to_owned(),
            false,
            tokens,
            GpuiAction::DialogsToolbar(DialogsToolbarAction::RefreshGitHub),
            cx,
        ));

    let mut list = div()
        .id("gpui-gh-scroll")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(6.0));

    if m.github_tab == GitHubTab::PullRequests {
        for pr in &m.github_prs {
            let slug = format!("pr-{}", pr.number);
            let url = pr.url.clone();
            let fix_prompt =
                (!pr.failing_checks.is_empty()).then(|| format_fix_failing_checks_prompt(pr));
            let badge_col = rgb_tuple(pr.rollup.rgb());
            list = list.child(
                div()
                    .p(px(8.0))
                    .rounded(px(tokens.radius_px))
                    .bg(tokens.surface)
                    .border_1()
                    .border_color(tokens.border)
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(tokens.text_strong)
                                    .truncate()
                                    .child(SharedString::from(format!(
                                        "#{} {}",
                                        pr.number, pr.title
                                    ))),
                            )
                            .child(
                                div()
                                    .font_family(mono_font_family())
                                    .text_size(px(10.5))
                                    .text_color(badge_col)
                                    .child(SharedString::from(format!(
                                        "{} → {} · {}",
                                        pr.head_ref,
                                        pr.base_ref,
                                        pr.rollup.label(french)
                                    ))),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(4.0))
                            .when_some(fix_prompt, |el, prompt| {
                                el.child(modal_btn(
                                    format!("gpui-gh-fix-{}", pr.number),
                                    "Fix failing checks".to_owned(),
                                    false,
                                    tokens,
                                    GpuiAction::DialogsToolbar(
                                        DialogsToolbarAction::SendPromptToActiveAgent(prompt),
                                    ),
                                    cx,
                                ))
                            })
                            .child(modal_btn(
                                format!("gpui-gh-wt-{}", pr.number),
                                "Worktree".to_owned(),
                                false,
                                tokens,
                                GpuiAction::DialogsToolbar(
                                    DialogsToolbarAction::StartWorktreeFromSlug(slug),
                                ),
                                cx,
                            ))
                            .child(modal_btn(
                                format!("gpui-gh-open-{}", pr.number),
                                "Open".to_owned(),
                                false,
                                tokens,
                                GpuiAction::DialogsToolbar(DialogsToolbarAction::OpenExternalUrl(
                                    url,
                                )),
                                cx,
                            )),
                    ),
            );
        }
    } else {
        for iss in &m.github_issues {
            let slug = format!("issue-{}", iss.number);
            let url = iss.url.clone();
            list = list.child(
                div()
                    .p(px(8.0))
                    .rounded(px(tokens.radius_px))
                    .bg(tokens.surface)
                    .border_1()
                    .border_color(tokens.border)
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.0))
                            .text_color(tokens.text_strong)
                            .truncate()
                            .child(SharedString::from(format!("#{} {}", iss.number, iss.title))),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(4.0))
                            .child(modal_btn(
                                format!("gpui-gh-iss-wt-{}", iss.number),
                                "Start Worktree".to_owned(),
                                false,
                                tokens,
                                GpuiAction::DialogsToolbar(
                                    DialogsToolbarAction::StartWorktreeFromSlug(slug),
                                ),
                                cx,
                            ))
                            .child(modal_btn(
                                format!("gpui-gh-iss-open-{}", iss.number),
                                "Open".to_owned(),
                                false,
                                tokens,
                                GpuiAction::DialogsToolbar(DialogsToolbarAction::OpenExternalUrl(
                                    url,
                                )),
                                cx,
                            )),
                    ),
            );
        }
    }

    modal_backdrop(
        "gpui-gh-backdrop",
        tokens,
        GpuiAction::DialogsToolbar(DialogsToolbarAction::CloseGitHubModal),
        div()
            .id("gpui-gh-card")
            .w(px(740.0))
            .h(px(520.0))
            .p(px(14.0))
            .rounded(px(tokens.radius_px + 3.0))
            .bg(tokens.panel)
            .border_1()
            .border_color(tokens.border_strong)
            .flex()
            .flex_col()
            .gap(px(10.0))
            .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(modal_header(
                "GitHub Pull Requests & Issues",
                tokens,
                GpuiAction::DialogsToolbar(DialogsToolbarAction::CloseGitHubModal),
                cx,
            ))
            .child(top)
            .child(render_focus_input_box(
                "gpui-gh-query",
                FocusInputTarget::GitHubSearch,
                focus_input == FocusInputTarget::GitHubSearch,
                &m.github_query,
                "Filter PRs or issues…",
                tokens,
                cx,
            ))
            .when(!m.github_error.is_empty(), |el| {
                el.child(
                    div()
                        .text_size(px(11.0))
                        .text_color(tokens.danger)
                        .child(SharedString::from(m.github_error.clone())),
                )
            })
            .child(list)
            .into_any_element(),
        cx,
    )
}

// ---------------------------------------------------------------------------
// 8. NotesPanel Modal (`.tessel/notes.md`)
// ---------------------------------------------------------------------------

fn render_sessions_modal(
    snapshot: &GpuiWorkspaceSnapshot,
    inputs: &super::panel_query_inputs::Inputs,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let close = GpuiAction::SidePanels(SidePanelsAction::CloseSessionsModal);
    let body = super::side_panels_view::render_sessions_history_tab(
        &snapshot.side_panels,
        inputs,
        snapshot.french,
        tokens,
        cx,
    );
    modal_backdrop(
        "gpui-sessions-backdrop",
        tokens,
        close.clone(),
        div()
            .id("gpui-sessions-card")
            .w(px(720.0))
            .h(px(620.0))
            .max_w_full()
            .max_h_full()
            .p(px(14.0))
            .rounded(px(tokens.radius_px + 3.0))
            .bg(tokens.panel)
            .border_1()
            .border_color(tokens.border_strong)
            .flex()
            .flex_col()
            .gap(px(10.0))
            .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(modal_header(
                if snapshot.french {
                    "Conversations passées"
                } else {
                    "Past Conversations"
                },
                tokens,
                close,
                cx,
            ))
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element(),
        cx,
    )
}

fn render_notes_modal(
    m: &GpuiModalsSnapshot,
    focus_input: FocusInputTarget,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let prompt_payload = format_notes_for_agent(&m.notes_text);
    let focused = focus_input == FocusInputTarget::NotesEditor;

    modal_backdrop(
        "gpui-notes-backdrop",
        tokens,
        GpuiAction::SidePanels(SidePanelsAction::CloseNotesModal),
        div()
            .id("gpui-notes-card")
            .w(px(620.0))
            .h(px(460.0))
            .p(px(14.0))
            .rounded(px(tokens.radius_px + 3.0))
            .bg(tokens.panel)
            .border_1()
            .border_color(tokens.border_strong)
            .flex()
            .flex_col()
            .gap(px(10.0))
            .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(modal_header(
                if m.notes_dirty {
                    "Project Scratchpad (.tessel/notes.md •)"
                } else {
                    "Project Scratchpad (.tessel/notes.md)"
                },
                tokens,
                GpuiAction::SidePanels(SidePanelsAction::CloseNotesModal),
                cx,
            ))
            .child(
                div()
                    .id("gpui-notes-editor")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(10.0))
                    .rounded(px(tokens.radius_px))
                    .bg(tokens.surface)
                    .border_1()
                    .border_color(if focused {
                        tokens.accent
                    } else {
                        tokens.border
                    })
                    .font_family(mono_font_family())
                    .text_size(px(12.0))
                    .text_color(tokens.text_strong)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.set_focus_input(FocusInputTarget::NotesEditor, window, cx);
                    }))
                    .child(SharedString::from(if m.notes_text.is_empty() {
                        if french {
                            "Écrivez vos notes Markdown ou tâches ici…".to_owned()
                        } else {
                            "Write project scratchpad notes or checklists here…".to_owned()
                        }
                    } else if focused {
                        format!("{}▍", m.notes_text)
                    } else {
                        m.notes_text.clone()
                    })),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(6.0))
                    .child(modal_btn(
                        "gpui-notes-open-ed",
                        "Open in Editor".to_owned(),
                        false,
                        tokens,
                        GpuiAction::SidePanels(SidePanelsAction::OpenNotesInEditor),
                        cx,
                    ))
                    .child(modal_btn(
                        "gpui-notes-send",
                        "Send to Agent".to_owned(),
                        false,
                        tokens,
                        GpuiAction::SidePanels(SidePanelsAction::SendPromptToActiveAgent(
                            prompt_payload,
                        )),
                        cx,
                    ))
                    .child(modal_btn(
                        "gpui-notes-save",
                        "Save".to_owned(),
                        true,
                        tokens,
                        GpuiAction::SidePanels(SidePanelsAction::SaveNotes { force: true }),
                        cx,
                    )),
            )
            .into_any_element(),
        cx,
    )
}

// ---------------------------------------------------------------------------
// 9. Branch Review & Merge Modal (`ReviewPanel.vue`)
// ---------------------------------------------------------------------------

fn render_review_modal(
    m: &GpuiModalsSnapshot,
    _french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let mut branches = div().flex().flex_wrap().gap(px(4.0));
    for (idx, (br, wt_path)) in m.review_candidates.iter().enumerate() {
        let b_clone = br.clone();
        let p_clone = wt_path.clone();
        branches = branches.child(modal_btn(
            format!("gpui-rev-br-{idx}"),
            br.clone(),
            m.review_head_branch == *br,
            tokens,
            GpuiAction::SidePanels(SidePanelsAction::SelectReviewBranch {
                head_branch: b_clone,
                worktree_path: p_clone,
            }),
            cx,
        ));
    }

    let body = if let Some(rs) = &m.review_snapshot {
        let mut files_col = div()
            .id("gpui-rev-files")
            .w(px(230.0))
            .h_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(3.0));
        for (idx, fe) in rs.files.iter().enumerate() {
            let fpath = fe.path.clone();
            let sel = m.review_selected_file.as_deref() == Some(fe.path.as_str());
            files_col = files_col.child(
                div()
                    .id(("gpui-rev-file", idx as u64))
                    .px(px(6.0))
                    .py(px(4.0))
                    .rounded(px(tokens.radius_px - 1.0))
                    .bg(if sel {
                        tokens.accent_soft
                    } else {
                        tokens.surface
                    })
                    .font_family(mono_font_family())
                    .text_size(px(11.0))
                    .text_color(tokens.text_strong)
                    .truncate()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.dispatch_action(
                            GpuiAction::SidePanels(SidePanelsAction::SelectReviewFile(
                                fpath.clone(),
                            )),
                            cx,
                        );
                    }))
                    .child(SharedString::from(format!("{} {}", fe.status, fe.path))),
            );
        }

        let diff_col = div()
            .id("gpui-rev-diff")
            .flex_1()
            .h_full()
            .overflow_y_scroll()
            .p(px(8.0))
            .rounded(px(tokens.radius_px))
            .bg(tokens.term)
            .font_family(mono_font_family())
            .text_size(px(11.0))
            .text_color(tokens.text)
            .child(SharedString::from(m.review_selected_diff.clone()));

        let can_merge = rs.can_clean_merge();
        let base_br = rs.base_branch.clone();
        let head_br = rs.head_branch.clone();
        let head_oid = rs.head_oid.clone();
        let push_br = rs.head_branch.clone();

        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .font_family(mono_font_family())
                            .text_size(px(11.5))
                            .text_color(tokens.text_strong)
                            .child(SharedString::from(format!(
                                "{} ← {} (↑{} ↓{} · {} conflicts)",
                                rs.base_branch,
                                rs.head_branch,
                                rs.ahead,
                                rs.behind,
                                rs.conflicts.len()
                            ))),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(5.0))
                            .child(modal_btn(
                                "gpui-rev-push",
                                "Push Branch".to_owned(),
                                false,
                                tokens,
                                GpuiAction::SidePanels(SidePanelsAction::PushReviewBranch(push_br)),
                                cx,
                            ))
                            .when(can_merge, |el| {
                                el.child(modal_btn(
                                    "gpui-rev-merge",
                                    "Merge (--no-ff)".to_owned(),
                                    true,
                                    tokens,
                                    GpuiAction::SidePanels(SidePanelsAction::MergeReviewBranch {
                                        base_branch: base_br,
                                        head_branch: head_br,
                                        expect_head_oid: head_oid,
                                    }),
                                    cx,
                                ))
                            }),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .gap(px(8.0))
                    .child(files_col)
                    .child(diff_col),
            )
            .into_any_element()
    } else {
        div()
            .text_size(px(12.0))
            .text_color(tokens.text_dim)
            .child(
                "Select a branch or worktree above to inspect its changes against the base branch.",
            )
            .into_any_element()
    };

    modal_backdrop(
        "gpui-review-backdrop",
        tokens,
        GpuiAction::SidePanels(SidePanelsAction::CloseReviewModal),
        div()
            .id("gpui-review-card")
            .w(px(780.0))
            .h(px(540.0))
            .p(px(14.0))
            .rounded(px(tokens.radius_px + 3.0))
            .bg(tokens.panel)
            .border_1()
            .border_color(tokens.border_strong)
            .flex()
            .flex_col()
            .gap(px(10.0))
            .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(modal_header(
                "Branch Review & Guarded Merge",
                tokens,
                GpuiAction::SidePanels(SidePanelsAction::CloseReviewModal),
                cx,
            ))
            .child(branches)
            .when_some(m.review_error.clone(), |el, err| {
                el.child(
                    div()
                        .text_size(px(11.0))
                        .text_color(tokens.danger)
                        .child(SharedString::from(err)),
                )
            })
            .child(body)
            .into_any_element(),
        cx,
    )
}

// ---------------------------------------------------------------------------
// 10. Worktree Cleanup Modal
// ---------------------------------------------------------------------------

fn render_worktree_cleanup_modal(
    m: &GpuiModalsSnapshot,
    _french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let Some(wc) = &m.worktree_cleanup else {
        return div().into_any_element();
    };

    let mut list = div()
        .id("gpui-wc-scroll")
        .max_h(px(300.0))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(4.0));

    for (idx, item) in wc.items.iter().enumerate() {
        let checked = wc.selected.contains(&item.path);
        let p = item.path.clone();
        list = list.child(modal_btn(
            format!("gpui-wc-item-{idx}"),
            format!(
                "[{}] {} ({})",
                if checked { "✓" } else { " " },
                item.branch,
                if item.merged { "merged" } else { "unmerged" }
            ),
            checked,
            tokens,
            GpuiAction::WorktreeCleanupTogglePath(p),
            cx,
        ));
    }

    modal_backdrop(
        "gpui-wc-backdrop",
        tokens,
        GpuiAction::WorktreeCleanupClose,
        div()
            .id("gpui-wc-card")
            .w(px(540.0))
            .p(px(14.0))
            .rounded(px(tokens.radius_px + 3.0))
            .bg(tokens.panel)
            .border_1()
            .border_color(tokens.border_strong)
            .flex()
            .flex_col()
            .gap(px(10.0))
            .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(modal_header(
                &format!("Clean Up Git Worktrees — {}", wc.project_name),
                tokens,
                GpuiAction::WorktreeCleanupClose,
                cx,
            ))
            .child(
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(modal_btn(
                        "gpui-wc-safe",
                        "Select Merged & Clean".to_owned(),
                        false,
                        tokens,
                        GpuiAction::WorktreeCleanupSelectSafe,
                        cx,
                    ))
                    .child(modal_btn(
                        "gpui-wc-none",
                        "Clear Selection".to_owned(),
                        false,
                        tokens,
                        GpuiAction::WorktreeCleanupSelectNone,
                        cx,
                    )),
            )
            .child(list)
            .child(div().flex().justify_end().gap(px(6.0)).child(modal_btn(
                "gpui-wc-remove",
                format!("Remove Selected ({})", wc.selected.len()),
                !wc.selected.is_empty(),
                tokens,
                GpuiAction::WorktreeCleanupConfirmRemove,
                cx,
            )))
            .into_any_element(),
        cx,
    )
}

// ---------------------------------------------------------------------------
// 11. Teams Manager + Worker Approval + Relay Pair Approval Modals
// ---------------------------------------------------------------------------

fn render_teams_manager_modal(
    m: &GpuiModalsSnapshot,
    focus_input: FocusInputTarget,
    _french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let pid = m.teams_manager_project.unwrap_or(0);
    let draft_name = m.teams_new_name.trim().to_owned();

    let mut list = div()
        .id("gpui-teams-scroll")
        .max_h(px(340.0))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(8.0));

    for team in &m.teams_list {
        let del_id = team.id.clone();
        let mut t_card = div()
            .p(px(8.0))
            .rounded(px(tokens.radius_px))
            .bg(tokens.surface)
            .border_1()
            .border_color(tokens.border)
            .flex()
            .flex_col()
            .gap(px(5.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(12.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(tokens.text_strong)
                            .child(SharedString::from(team.name.clone())),
                    )
                    .child(modal_btn(
                        format!("gpui-team-del-{}", team.id),
                        "Delete Team".to_owned(),
                        false,
                        tokens,
                        GpuiAction::Teams(TeamsUiAction::DeleteTeam(del_id)),
                        cx,
                    )),
            );

        let mut panes_wrap = div().flex().flex_wrap().gap(px(4.0));
        for (pane_id, pane_lbl) in &m.teams_project_panes {
            let in_team = team.members.contains(pane_id);
            let is_lead = team.lead_pane == Some(*pane_id);
            let tid1 = team.id.clone();
            let tid2 = team.id.clone();
            let p_id = *pane_id;
            panes_wrap = panes_wrap
                .child(modal_btn(
                    format!("gpui-team-mem-{}-{p_id}", team.id),
                    format!("[{}] {pane_lbl}", if in_team { "✓" } else { "+" }),
                    in_team,
                    tokens,
                    GpuiAction::Teams(TeamsUiAction::ToggleMember {
                        team_id: tid1,
                        pane_id: p_id,
                    }),
                    cx,
                ))
                .when(in_team, |el| {
                    el.child(modal_btn(
                        format!("gpui-team-lead-{}-{p_id}", team.id),
                        if is_lead { "★ Lead" } else { "Make Lead" }.to_owned(),
                        is_lead,
                        tokens,
                        GpuiAction::Teams(TeamsUiAction::ToggleLead {
                            team_id: tid2,
                            pane_id: p_id,
                        }),
                        cx,
                    ))
                });
        }
        t_card = t_card.child(panes_wrap);
        list = list.child(t_card);
    }

    modal_backdrop(
        "gpui-teams-backdrop",
        tokens,
        GpuiAction::CloseTeamsManager,
        div()
            .id("gpui-teams-card")
            .w(px(580.0))
            .p(px(14.0))
            .rounded(px(tokens.radius_px + 3.0))
            .bg(tokens.panel)
            .border_1()
            .border_color(tokens.border_strong)
            .flex()
            .flex_col()
            .gap(px(10.0))
            .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(modal_header(
                "Agent Teams & Orchestration",
                tokens,
                GpuiAction::CloseTeamsManager,
                cx,
            ))
            .child(
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(div().flex_1().child(render_focus_input_box(
                        "gpui-team-new-name",
                        FocusInputTarget::NewTeamName,
                        focus_input == FocusInputTarget::NewTeamName,
                        &m.teams_new_name,
                        "Team name (e.g. Backend Squad)…",
                        tokens,
                        cx,
                    )))
                    .child(modal_btn(
                        "gpui-team-create-btn",
                        "Create Team".to_owned(),
                        true,
                        tokens,
                        GpuiAction::Teams(TeamsUiAction::CreateTeam {
                            project: pid,
                            name: draft_name,
                        }),
                        cx,
                    )),
            )
            .child(list)
            .into_any_element(),
        cx,
    )
}

fn render_worker_approval_modal(
    w: &crate::teams_relay::WorkerRecord,
    _french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let wid1 = w.id.clone();
    let wid2 = w.id.clone();
    modal_backdrop(
        "gpui-worker-ask-backdrop",
        tokens,
        GpuiAction::Teams(TeamsUiAction::RefuseWorker(wid1)),
        div()
            .id("gpui-worker-ask-card")
            .w(px(480.0))
            .p(px(14.0))
            .rounded(px(tokens.radius_px + 3.0))
            .bg(tokens.panel)
            .border_1()
            .border_color(tokens.accent)
            .flex()
            .flex_col()
            .gap(px(10.0))
            .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(tokens.text_strong)
                    .child(SharedString::from(format!(
                        "{} wants to spawn worker '{}' ({})",
                        w.coordinator_name, w.worker_name, w.agent
                    ))),
            )
            .child(
                div()
                    .p(px(8.0))
                    .rounded(px(tokens.radius_px))
                    .bg(tokens.surface)
                    .text_size(px(11.5))
                    .text_color(tokens.text)
                    .child(SharedString::from(w.task_goal.clone())),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(6.0))
                    .child(modal_btn(
                        "gpui-worker-refuse",
                        "Refuse".to_owned(),
                        false,
                        tokens,
                        GpuiAction::Teams(TeamsUiAction::RefuseWorker(w.id.clone())),
                        cx,
                    ))
                    .child(modal_btn(
                        "gpui-worker-approve",
                        "Approve Worker".to_owned(),
                        true,
                        tokens,
                        GpuiAction::Teams(TeamsUiAction::ApproveWorker(wid2)),
                        cx,
                    )),
            )
            .into_any_element(),
        cx,
    )
}

fn render_relay_approval_modal(
    p: &crate::teams_relay::RelayApprovalPrompt,
    _french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let pair = p.pair;
    let mut choices = div().flex().gap(px(6.0));
    for limit in RELAY_HOURLY_CHOICES {
        choices = choices.child(modal_btn(
            format!("gpui-relay-allow-{limit}"),
            format!("Allow ({limit}/hr)"),
            limit == p.selected_hourly_limit,
            tokens,
            GpuiAction::Teams(TeamsUiAction::ApproveRelayPair {
                pair,
                hourly_limit: limit,
            }),
            cx,
        ));
    }

    modal_backdrop(
        "gpui-relay-ask-backdrop",
        tokens,
        GpuiAction::Teams(TeamsUiAction::DeclineRelayPair(pair)),
        div()
            .id("gpui-relay-ask-card")
            .w(px(500.0))
            .p(px(14.0))
            .rounded(px(tokens.radius_px + 3.0))
            .bg(tokens.panel)
            .border_1()
            .border_color(tokens.accent)
            .flex()
            .flex_col()
            .gap(px(10.0))
            .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(tokens.text_strong)
                    .child(SharedString::from(format!(
                        "Cross-Project Relay: {} ({}) → {} ({})",
                        p.from_agent_name, p.from_project_name, p.to_agent_name, p.to_project_name
                    ))),
            )
            .child(
                div()
                    .p(px(8.0))
                    .rounded(px(tokens.radius_px))
                    .bg(tokens.surface)
                    .text_size(px(11.5))
                    .text_color(tokens.text)
                    .child(SharedString::from(p.preview_text.clone())),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .child(modal_btn(
                        "gpui-relay-decline",
                        "Decline (10 min)".to_owned(),
                        false,
                        tokens,
                        GpuiAction::Teams(TeamsUiAction::DeclineRelayPair(pair)),
                        cx,
                    ))
                    .child(choices),
            )
            .into_any_element(),
        cx,
    )
}

// ---------------------------------------------------------------------------
// 12. Toolbar Popovers (`Usage` & `Notifications`) + StatusBar Popovers
// ---------------------------------------------------------------------------

fn render_usage_popover(
    m: &GpuiModalsSnapshot,
    _french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let u = &m.usage_summary;
    let mut col = div().flex().flex_col().gap(px(6.0)).child(
        div()
            .text_size(px(12.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(tokens.text_strong)
            .child(SharedString::from(format!(
                "Today: ${:.2} / ${:.2} · 30d: ${:.2}",
                u.today_cost_usd, u.daily_budget_usd, u.month_cost_usd
            ))),
    );
    for bar in &u.providers {
        col = col.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .font_family(mono_font_family())
                .text_size(px(11.0))
                .text_color(tokens.text)
                .child(SharedString::from(bar.provider.clone()))
                .child(SharedString::from(format!("${:.2}", bar.cost_today_usd))),
        );
    }
    col = col.child(modal_btn(
        "gpui-usage-open-stats",
        "Open Insights Settings".to_owned(),
        false,
        tokens,
        GpuiAction::DialogsToolbar(DialogsToolbarAction::OpenStatsSettings),
        cx,
    ));

    popover_card(
        "gpui-usage-pop",
        520.0,
        40.0,
        320.0,
        tokens,
        GpuiAction::ToggleUsagePopover,
        col.into_any_element(),
        cx,
    )
}

fn render_notifications_popover(
    m: &GpuiModalsSnapshot,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let mut list = div()
        .id("gpui-notif-scroll")
        .max_h(px(260.0))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(4.0));

    if m.notifications_items.is_empty() {
        list = list.child(
            div()
                .text_size(px(11.5))
                .text_color(tokens.text_dim)
                .child(if french {
                    "Aucune notification."
                } else {
                    "All caught up — no notifications."
                }),
        );
    } else {
        for item in &m.notifications_items {
            let nid = item.id;
            let focus_pane = item.pane_id;
            list = list.child(
                div()
                    .id(("gpui-notif-item", nid))
                    .p(px(6.0))
                    .rounded(px(tokens.radius_px - 1.0))
                    .bg(tokens.surface)
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(pid) = focus_pane {
                            this.dispatch_action(
                                GpuiAction::DialogsToolbar(DialogsToolbarAction::FocusPane(pid)),
                                cx,
                            );
                        }
                    }))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(tokens.text_strong)
                            .child(SharedString::from(item.title.clone())),
                    )
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(tokens.text_dim)
                            .child(SharedString::from(item.detail.clone())),
                    ),
            );
        }
    }

    popover_card(
        "gpui-notif-pop",
        560.0,
        40.0,
        340.0,
        tokens,
        GpuiAction::ToggleNotificationsPopover,
        div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(12.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(tokens.text_strong)
                            .child("Notifications"),
                    )
                    .child(modal_btn(
                        "gpui-notif-read-all",
                        "Mark read".to_owned(),
                        false,
                        tokens,
                        GpuiAction::DialogsToolbar(DialogsToolbarAction::MarkAllNotificationsRead),
                        cx,
                    )),
            )
            .child(list)
            .into_any_element(),
        cx,
    )
}

fn render_status_popover(
    snapshot: &GpuiWorkspaceSnapshot,
    item_id: StatusBarItemId,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let m = &snapshot.modals;
    let body = match item_id {
        StatusBarItemId::Resources => {
            let samples = m.telemetry_app_sparkline.clone();
            let accent = tokens.accent;
            let sparkline = canvas(
                |_, _, _| (),
                move |bounds, (), window, _cx| {
                    if samples.len() < 2 {
                        return;
                    }
                    let max_v = samples.iter().copied().fold(1.0_f64, f64::max);
                    let w = f32::from(bounds.size.width);
                    let h = f32::from(bounds.size.height);
                    let ox = bounds.origin.x;
                    let oy = bounds.origin.y;
                    let mut pb = PathBuilder::stroke(px(1.6));
                    for (i, &val) in samples.iter().enumerate() {
                        let x = (i as f32 / (samples.len() - 1) as f32) * w;
                        let y = h - ((val / max_v) as f32 * (h - 6.0) + 3.0);
                        if i == 0 {
                            pb.move_to(point(ox + px(x), oy + px(y)));
                        } else {
                            pb.line_to(point(ox + px(x), oy + px(y)));
                        }
                    }
                    if let Ok(path) = pb.build() {
                        window.paint_path(path, accent);
                    }
                },
            )
            .w_full()
            .h(px(44.0));

            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(
                    div()
                        .font_family(mono_font_family())
                        .text_size(px(11.5))
                        .text_color(tokens.text_strong)
                        .child(SharedString::from(format!(
                            "App RSS: {} MB · Panes RSS: {} MB · CPU: {:.1}%",
                            m.telemetry_app_rss_bytes / (1024 * 1024),
                            m.telemetry_panes_rss_bytes / (1024 * 1024),
                            m.telemetry_panes_cpu_pct
                        ))),
                )
                .child(
                    div()
                        .rounded(px(tokens.radius_px - 1.0))
                        .bg(tokens.surface)
                        .border_1()
                        .border_color(tokens.border)
                        .p(px(4.0))
                        .child(sparkline),
                )
                .into_any_element()
        }
        StatusBarItemId::Ports => {
            let mut col = div().flex().flex_col().gap(px(4.0)).child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(12.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(tokens.text_strong)
                            .child("Listening Dev Ports"),
                    )
                    .child(modal_btn(
                        "gpui-ports-refresh",
                        "Rescan".to_owned(),
                        false,
                        tokens,
                        GpuiAction::RefreshPortsNow,
                        cx,
                    )),
            );
            for (idx, p) in m.telemetry_ports.iter().take(10).enumerate() {
                let url = p.url.clone();
                col = col.child(modal_btn(
                    format!("gpui-port-open-{idx}"),
                    format!(":{} — {} (PID {})", p.port, p.process_name, p.pid),
                    false,
                    tokens,
                    GpuiAction::TerminalOpenTesselUrl(url),
                    cx,
                ));
            }
            col.into_any_element()
        }
        StatusBarItemId::Ssh => div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(tokens.text_strong)
                    .child(SharedString::from(format!(
                        "Configured SSH Hosts: {}",
                        snapshot.status_bar.ssh_host_count
                    ))),
            )
            .child(modal_btn(
                "gpui-status-open-ssh",
                "Manage SSH Hosts".to_owned(),
                true,
                tokens,
                GpuiAction::OpenSettingsPage("ssh".into()),
                cx,
            ))
            .into_any_element(),
    };

    div()
        .id("gpui-status-pop-backdrop")
        .absolute()
        .left(px(0.0))
        .top(px(0.0))
        .size_full()
        .on_click(cx.listener(move |this, _, _, cx| {
            this.dispatch_action(GpuiAction::ToggleStatusPopover(item_id), cx);
        }))
        .child(
            div()
                .id("gpui-status-pop-card")
                .absolute()
                .right(px(24.0))
                .bottom(px(30.0))
                .w(px(340.0))
                .p(px(10.0))
                .rounded(px(tokens.radius_px + 2.0))
                .bg(tokens.panel)
                .border_1()
                .border_color(tokens.border_strong)
                .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
                .child(body),
        )
        .into_any_element()
}

// ---------------------------------------------------------------------------
// 13. Generic Prompt & Confirm Modals
// ---------------------------------------------------------------------------

fn render_prompt_modal(
    pm: &crate::app::gpui_bridge::GpuiPromptModalSnapshot,
    focus_input: FocusInputTarget,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    modal_backdrop(
        "gpui-prompt-backdrop",
        tokens,
        GpuiAction::CancelPromptModal,
        div()
            .id("gpui-prompt-card")
            .w(px(440.0))
            .p(px(14.0))
            .rounded(px(tokens.radius_px + 3.0))
            .bg(tokens.panel)
            .border_1()
            .border_color(tokens.border_strong)
            .flex()
            .flex_col()
            .gap(px(10.0))
            .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(tokens.text_strong)
                    .child(SharedString::from(pm.title.clone())),
            )
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(tokens.text_dim)
                    .child(SharedString::from(pm.subtitle.clone())),
            )
            .child(render_focus_input_box(
                "gpui-prompt-input",
                FocusInputTarget::PromptDraft,
                focus_input == FocusInputTarget::PromptDraft,
                &pm.draft,
                "",
                tokens,
                cx,
            ))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(6.0))
                    .child(modal_btn(
                        "gpui-prompt-cancel",
                        if french { "Annuler" } else { "Cancel" }.to_owned(),
                        false,
                        tokens,
                        GpuiAction::CancelPromptModal,
                        cx,
                    ))
                    .child(modal_btn(
                        "gpui-prompt-submit",
                        pm.confirm_label.clone(),
                        true,
                        tokens,
                        GpuiAction::SubmitPromptModal,
                        cx,
                    )),
            )
            .into_any_element(),
        cx,
    )
}

fn render_confirm_modal(
    cm: &crate::app::gpui_bridge::GpuiConfirmModalSnapshot,
    task_card_input: Option<&super::task_card_input::Inputs>,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let button = |id, label, primary, action, cx: &mut Context<TesselGpuiRoot>| {
        if let Some(handle) = task_card_input.and_then(|i| i.confirm_handle(usize::from(primary))) {
            super::task_card_input::confirm_control(id, label, primary, handle, action, tokens, cx)
        } else {
            modal_btn(id, label, primary, tokens, action, cx)
        }
    };
    modal_backdrop(
        "gpui-confirm-backdrop",
        tokens,
        GpuiAction::CancelConfirmModal,
        div()
            .id("gpui-confirm-card")
            .when(task_card_input.is_some(), |d| {
                d.role(gpui::Role::AlertDialog)
                    .aria_modal(true)
                    .aria_label(cm.title.clone())
                    .aria_description(cm.body.clone())
            })
            .w(px(440.0))
            .p(px(14.0))
            .rounded(px(tokens.radius_px + 3.0))
            .bg(tokens.panel)
            .border_1()
            .border_color(if cm.danger {
                tokens.danger
            } else {
                tokens.border_strong
            })
            .flex()
            .flex_col()
            .gap(px(10.0))
            .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(tokens.text_strong)
                    .child(SharedString::from(cm.title.clone())),
            )
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(tokens.text)
                    .child(SharedString::from(cm.body.clone())),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(6.0))
                    .child(button(
                        "gpui-confirm-cancel",
                        if french { "Annuler" } else { "Cancel" }.to_owned(),
                        false,
                        GpuiAction::CancelConfirmModal,
                        cx,
                    ))
                    .when_some(cm.secondary_label.clone(), |el, sec| {
                        el.child(button(
                            "gpui-confirm-secondary",
                            sec,
                            false,
                            GpuiAction::ConfirmModalSecondary,
                            cx,
                        ))
                    })
                    .child(button(
                        "gpui-confirm-primary",
                        cm.confirm_label.clone(),
                        true,
                        GpuiAction::ConfirmModalPrimary,
                        cx,
                    )),
            )
            .into_any_element(),
        cx,
    )
}

// ---------------------------------------------------------------------------
// Shared Modal Helpers
// ---------------------------------------------------------------------------

fn modal_backdrop(
    id: &'static str,
    tokens: &GpuiThemeTokens,
    close_action: GpuiAction,
    card: AnyElement,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    div()
        .id(id)
        .absolute()
        .left(px(0.0))
        .top(px(0.0))
        .size_full()
        .bg(rgba_with_alpha(tokens.bg, 0.68))
        .flex()
        .items_center()
        .justify_center()
        .on_click(cx.listener(move |this, _, _, cx| {
            this.dispatch_action(close_action.clone(), cx);
        }))
        .child(card)
        .into_any_element()
}

/// Toolbar `⋯` menu (`App.vue` `.toolbar-menu.align-right`): the hidden secondary buttons'
/// actions, with their shortcuts and a check on what is on.
fn render_toolbar_more_menu(
    snapshot: &GpuiWorkspaceSnapshot,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let tb = &snapshot.toolbar;
    let entries = crate::toolbar_fit::more_menu_entries(crate::toolbar_fit::MoreMenuState {
        french: snapshot.french,
        broadcast: tb.broadcast_enabled,
        floating_open: tb.floating_open,
        left_open: tb.left_open,
        right_open: tb.right_open,
        can_arrange_panes: tb.can_arrange_panes,
    });
    let mut col = div().flex().flex_col().gap(px(1.0));
    for (idx, entry) in entries.into_iter().enumerate() {
        let Some(action) = entry.action else {
            col = col.child(div().my(px(3.0)).h(px(1.0)).bg(tokens.border));
            continue;
        };
        let color = if entry.danger {
            tokens.danger
        } else {
            tokens.text
        };
        col = col.child(
            div()
                .id(("gpui-toolbar-more-item", idx))
                .h(px(28.0))
                .px(px(9.0))
                .rounded(px(tokens.radius_px - 1.0))
                .flex()
                .items_center()
                .justify_between()
                .gap(px(14.0))
                .text_size(px(12.0))
                .text_color(color)
                .cursor_pointer()
                .hover(|s| s.bg(tokens.surface_2))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.dispatch_action(GpuiAction::ToolbarMore(action), cx);
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .child(SharedString::from(entry.label))
                        .when(entry.checked, |el| {
                            el.child(div().text_color(tokens.accent).child(" ✓"))
                        }),
                )
                .when_some(entry.shortcut, |el, sc| {
                    el.child(
                        div()
                            .text_size(px(10.5))
                            .font_family(mono_font_family())
                            .text_color(tokens.text_dim)
                            .child(sc),
                    )
                }),
        );
    }
    div()
        .id("gpui-toolbar-more-backdrop")
        .absolute()
        .left(px(0.0))
        .top(px(0.0))
        .size_full()
        .on_click(cx.listener(|this, _, _, cx| {
            this.dispatch_action(GpuiAction::ToggleToolbarMore, cx);
        }))
        .child(
            div()
                .id("gpui-toolbar-more-menu")
                .absolute()
                .right(px(3.0 * super::theme::CAPTION_BTN_WIDTH_PX + 40.0))
                .top(px(super::theme::TOOLBAR_HEIGHT_PX))
                .w(px(290.0))
                .p(px(5.0))
                .rounded(px(tokens.radius_px + 2.0))
                .bg(tokens.panel)
                .border_1()
                .border_color(tokens.border_strong)
                .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
                .child(col),
        )
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn popover_card(
    id: &'static str,
    left_px: f32,
    top_px: f32,
    width_px: f32,
    tokens: &GpuiThemeTokens,
    close_action: GpuiAction,
    body: AnyElement,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    div()
        .id(id)
        .absolute()
        .left(px(0.0))
        .top(px(0.0))
        .size_full()
        .on_click(cx.listener(move |this, _, _, cx| {
            this.dispatch_action(close_action.clone(), cx);
        }))
        .child(
            div()
                .id(SharedString::from(format!("{id}-inner")))
                .absolute()
                .left(px(left_px))
                .top(px(top_px))
                .w(px(width_px))
                .p(px(10.0))
                .rounded(px(tokens.radius_px + 2.0))
                .bg(tokens.panel)
                .border_1()
                .border_color(tokens.border_strong)
                .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
                .child(body),
        )
        .into_any_element()
}

fn modal_header(
    title: &str,
    tokens: &GpuiThemeTokens,
    close_action: GpuiAction,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(tokens.text_strong)
                .child(SharedString::from(title.to_owned())),
        )
        .child(
            div()
                .id(SharedString::from(format!("gpui-hdr-close-{title}")))
                .w(px(24.0))
                .h(px(24.0))
                .rounded(px(tokens.radius_px - 1.0))
                .flex()
                .items_center()
                .justify_center()
                .hover(|s| s.bg(tokens.surface_2))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.dispatch_action(close_action.clone(), cx);
                }))
                .child(GpuiIcon::Close.render(13.0, tokens.text_dim)),
        )
        .into_any_element()
}

fn modal_btn(
    id: impl Into<SharedString>,
    label: String,
    active: bool,
    tokens: &GpuiThemeTokens,
    action: GpuiAction,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    modal_btn_content(id, SharedString::from(label), active, tokens, action, cx)
}

fn modal_btn_content(
    id: impl Into<SharedString>,
    label: impl IntoElement,
    active: bool,
    tokens: &GpuiThemeTokens,
    action: GpuiAction,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let sid: SharedString = id.into();
    div()
        .id(sid.clone())
        .debug_selector(move || sid.to_string())
        .px(px(8.0))
        .py(px(4.5))
        .rounded(px(tokens.radius_px - 1.0))
        .bg(if active {
            tokens.accent_soft
        } else {
            tokens.surface
        })
        .border_1()
        .border_color(if active { tokens.accent } else { tokens.border })
        .text_size(px(11.0))
        .font_weight(if active {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::NORMAL
        })
        .text_color(if active {
            tokens.text_strong
        } else {
            tokens.text_dim
        })
        .hover(|s| s.bg(tokens.surface_2).text_color(tokens.text_strong))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.dispatch_action(action.clone(), cx);
        }))
        .child(label)
        .into_any_element()
}

fn render_dev_update_modal(
    m: &GpuiModalsSnapshot,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let title = if french {
        "Mise à jour Git disponible"
    } else {
        "Git checkout update"
    };
    let mut body = div().flex().flex_col().gap(px(8.0));

    if let Some(st) = &m.dev_update_state {
        let summary = if french {
            format!(
                "Actuel : b{} ({})  →  Distant : b{} ({} commit(s) entrant(s))",
                st.local_build, st.local_commit, st.remote_build, st.behind
            )
        } else {
            format!(
                "Current: b{} ({})  →  Upstream: b{} ({} incoming commit(s))",
                st.local_build, st.local_commit, st.remote_build, st.behind
            )
        };
        body = body.child(
            div()
                .text_size(px(11.5))
                .text_color(tokens.text_strong)
                .child(SharedString::from(summary)),
        );

        if !st.incoming_log.is_empty() {
            let mut log_box = div()
                .p(px(8.0))
                .rounded(px(tokens.radius_px))
                .bg(tokens.surface_2)
                .border_1()
                .border_color(tokens.border)
                .flex()
                .flex_col()
                .gap(px(3.0))
                .font_family(mono_font_family())
                .text_size(px(10.5))
                .text_color(tokens.text);
            for line in &st.incoming_log {
                log_box = log_box.child(SharedString::from(line.clone()));
            }
            body = body.child(log_box);
        }

        match &st.blocked {
            Some(crate::dev_update::DevUpdateBlocked::Dirty { overlap }) => {
                let msg = crate::dev_update::format_overlap_message(overlap, french);
                body = body.child(
                    div()
                        .p(px(8.0))
                        .rounded(px(tokens.radius_px))
                        .bg(rgba_with_alpha(tokens.warn, 0.16))
                        .border_1()
                        .border_color(tokens.warn)
                        .text_size(px(11.0))
                        .text_color(tokens.text_strong)
                        .child(SharedString::from(msg)),
                );
            }
            Some(crate::dev_update::DevUpdateBlocked::Ahead { count }) => {
                let msg = if french {
                    format!(
                        "Votre branche a {count} commit(s) local/locaux d'avance. Lancez git pull --rebase ou fusionnez manuellement."
                    )
                } else {
                    format!(
                        "Your branch is {count} commit(s) ahead of upstream. Rebase or merge manually."
                    )
                };
                body = body.child(
                    div()
                        .p(px(8.0))
                        .rounded(px(tokens.radius_px))
                        .bg(rgba_with_alpha(tokens.warn, 0.16))
                        .border_1()
                        .border_color(tokens.warn)
                        .text_size(px(11.0))
                        .text_color(tokens.text_strong)
                        .child(SharedString::from(msg)),
                );
            }
            None => {}
        }
    }

    if let Some(notice) = &m.dev_update_notice {
        body = body.child(
            div()
                .text_size(px(11.0))
                .text_color(tokens.text_dim)
                .child(SharedString::from(notice.clone())),
        );
    }

    let can_ff = m
        .dev_update_state
        .as_ref()
        .is_some_and(|st| st.behind > 0 && st.blocked.is_none())
        && !m.dev_update_busy;

    let mut actions = div().flex().items_center().justify_end().gap(px(6.0));
    actions = actions.child(modal_btn(
        "gpui-dev-upd-copy",
        if french {
            "Copier git pull".into()
        } else {
            "Copy git pull".into()
        },
        false,
        tokens,
        GpuiAction::CopyGitPullCommand,
        cx,
    ));
    actions = actions.child(modal_btn(
        "gpui-dev-upd-check",
        if french {
            "Revérifier".into()
        } else {
            "Check again".into()
        },
        false,
        tokens,
        GpuiAction::RefreshDevUpdate,
        cx,
    ));
    if can_ff {
        actions = actions.child(modal_btn(
            "gpui-dev-upd-ff",
            if french {
                "Avance rapide (fast-forward)".into()
            } else {
                "Fast-forward now".into()
            },
            true,
            tokens,
            GpuiAction::ApplyDevUpdate,
            cx,
        ));
    }

    modal_backdrop(
        "gpui-dev-update-backdrop",
        tokens,
        GpuiAction::ToggleDevUpdateDialog,
        div()
            .id("gpui-dev-update-card")
            .w(px(520.0))
            .p(px(14.0))
            .rounded(px(tokens.radius_px + 2.0))
            .bg(tokens.panel)
            .border_1()
            .border_color(tokens.border_strong)
            .flex()
            .flex_col()
            .gap(px(10.0))
            .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .child(modal_header(
                title,
                tokens,
                GpuiAction::ToggleDevUpdateDialog,
                cx,
            ))
            .child(body)
            .child(actions)
            .into_any_element(),
        cx,
    )
}
