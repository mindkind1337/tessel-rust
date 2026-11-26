//! 1:1 `GPUI` views for all 7 `SidePanel` tabs:
//! - `SideTab::Dashboard` (`AgentDashboard.vue`)
//! - `SideTab::Files` (`ExplorerPanel.vue`)
//! - `SideTab::Changes` (`ScmPanel.vue` + docked `ScmHistoryPanel.vue` with `gpui::canvas` 5-color swimlanes)
//! - `SideTab::Tasks` (`TaskBoard.vue` stacked status sections)
//! - `SideTab::TaskHistory` (`TaskHistoryPanel.vue`)
//! - `SideTab::History` (`SessionHistoryPanel.vue`)
//! - `SideTab::Conversations` (`ConversationsPanel.vue`)

use super::{
    FocusInputTarget, TesselGpuiRoot,
    icons::GpuiIcon,
    theme::{GpuiThemeTokens, mono_font_family, rgba_with_alpha},
};
use crate::{
    app::gpui_bridge::{GpuiAction, GpuiSidePanelsSnapshot, GpuiWorkspaceSnapshot},
    scm_changes_panel::{ScmArea, ScmRemoteOp},
    side_panels_model::{
        ConversationWindow, DashboardGroupBy, DashboardState, SWIMLANE_PALETTE,
        SessionHistoryGroupBy, SessionHistoryScope, TaskHistoryFilter, TaskHistorySort,
        format_duration_compact, format_thread_as_text, history_csv,
    },
    side_panels_ui::SidePanelsAction,
    workspace::SideTab,
};
use gpui::{
    AnyElement, AppContext, Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    PathBuilder, Rgba, SharedString, StatefulInteractiveElement, Styled, canvas, div, point,
    prelude::FluentBuilder as _, px,
};

fn rgb_tuple((r, g, b): (u8, u8, u8)) -> Rgba {
    gpui::rgb((u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_side_panel_tab_body(
    snapshot: &GpuiWorkspaceSnapshot,
    scm_input: &super::scm_input::Input,
    explorer_input: &super::explorer_input::Input,
    panel_query_inputs: &super::panel_query_inputs::Inputs,
    task_rename_input: &super::task_rename_input::Input,
    task_card_input: &super::task_card_input::Inputs,
    task_agent_inputs: &super::task_agent_input::Inputs,
    task_decision_inputs: &super::task_decision_input::Inputs,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let sp = &snapshot.side_panels;
    let french = snapshot.french;
    match snapshot.side_panel.tab {
        SideTab::Dashboard => render_dashboard_tab(sp, panel_query_inputs, french, tokens, cx),
        SideTab::Files => {
            render_explorer_tab(sp, explorer_input, panel_query_inputs, french, tokens, cx)
        }
        SideTab::Changes => render_scm_changes_tab(sp, scm_input, french, tokens, cx),
        SideTab::Tasks => render_tasks_board_tab(
            sp,
            panel_query_inputs,
            task_rename_input,
            task_card_input,
            task_agent_inputs,
            task_decision_inputs,
            french,
            tokens,
            cx,
        ),
        SideTab::TaskHistory => render_task_history_tab(sp, panel_query_inputs, french, tokens, cx),
        // One native input entity cannot appear in two places in the same frame.
        SideTab::History if snapshot.modals.sessions_open => div().into_any_element(),
        SideTab::History => render_sessions_history_tab(sp, panel_query_inputs, french, tokens, cx),
        SideTab::Conversations => {
            render_conversations_tab(sp, panel_query_inputs, french, tokens, cx)
        }
    }
}

// ---------------------------------------------------------------------------
// 1. `SideTab::Dashboard` (`AgentDashboard.vue`)
// ---------------------------------------------------------------------------

fn render_dashboard_tab(
    sp: &GpuiSidePanelsSnapshot,
    panel_query_inputs: &super::panel_query_inputs::Inputs,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let vm = &sp.dashboard_vm;
    let sleep_ids = vm.sleepable_pane_ids.clone();

    let mut filter_bar = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(4.0))
        .child(render_chip(
            "gpui-dash-filter-all",
            format!("{} ({})", if french { "Tous" } else { "All" }, vm.total),
            sp.dashboard_filter.is_none(),
            tokens,
            GpuiAction::SetDashboardFilter(None),
            cx,
        ));

    for st in DashboardState::ALL {
        let count = vm.counts.get(&st).copied().unwrap_or(0);
        if count > 0 || sp.dashboard_filter == Some(st) {
            filter_bar = filter_bar.child(render_chip(
                format!("gpui-dash-filter-{st:?}"),
                format!("{} ({count})", st.label(french)),
                sp.dashboard_filter == Some(st),
                tokens,
                GpuiAction::SetDashboardFilter(Some(st)),
                cx,
            ));
        }
    }

    let mut controls = div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(4.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(3.0))
                .child(render_chip(
                    "gpui-dash-grp-status",
                    if french { "Par état" } else { "By status" }.to_owned(),
                    sp.dashboard_group == DashboardGroupBy::Status,
                    tokens,
                    GpuiAction::SetDashboardGroup(DashboardGroupBy::Status),
                    cx,
                ))
                .child(render_chip(
                    "gpui-dash-grp-project",
                    if french { "Par projet" } else { "By project" }.to_owned(),
                    sp.dashboard_group == DashboardGroupBy::Project,
                    tokens,
                    GpuiAction::SetDashboardGroup(DashboardGroupBy::Project),
                    cx,
                )),
        );

    if !sleep_ids.is_empty() {
        let n = sleep_ids.len();
        controls = controls.child(
            div()
                .id("gpui-dash-sleep-idle")
                .px(px(6.0))
                .py(px(2.5))
                .rounded(px(tokens.radius_px - 1.0))
                .bg(tokens.surface_2)
                .border_1()
                .border_color(tokens.border)
                .text_size(px(10.5))
                .text_color(tokens.text_strong)
                .hover(|s| s.border_color(tokens.accent))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.dispatch_action(
                        GpuiAction::SidePanels(SidePanelsAction::SleepPanes(sleep_ids.clone())),
                        cx,
                    );
                }))
                .child(SharedString::from(if french {
                    format!("Mettre en veille ({n})")
                } else {
                    format!("Sleep idle ({n})")
                })),
        );
    }

    let mut list = div()
        .id("gpui-dash-scroll")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(8.0));

    if vm.groups.is_empty() {
        list = list.child(render_empty_note(
            if french {
                "Aucun agent actif."
            } else {
                "No matching agent panes."
            },
            tokens,
        ));
    } else {
        for grp in &vm.groups {
            let mut grp_col = div().flex().flex_col().gap(px(4.0)).child(
                div()
                    .text_size(px(10.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(tokens.text_dim)
                    .child(SharedString::from(grp.heading.clone())),
            );
            for item in &grp.items {
                let pane_id = item.pane_id;
                let dot = rgb_tuple(item.state.rgb());
                grp_col = grp_col.child(
                    div()
                        .id(("gpui-dash-item", pane_id))
                        .p(px(7.0))
                        .rounded(px(tokens.radius_px))
                        .bg(if item.is_focused {
                            tokens.accent_soft
                        } else {
                            tokens.surface
                        })
                        .border_1()
                        .border_color(if item.is_focused {
                            tokens.accent
                        } else {
                            tokens.border
                        })
                        .flex()
                        .flex_col()
                        .gap(px(3.0))
                        .hover(|s| s.bg(tokens.surface_2))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.dispatch_action(
                                GpuiAction::SidePanels(SidePanelsAction::FocusPane(pane_id)),
                                cx,
                            );
                        }))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap(px(4.0))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .items_center()
                                        .gap(px(5.0))
                                        .child(div().w(px(7.0)).h(px(7.0)).rounded(px(3.5)).bg(dot))
                                        .child(
                                            div()
                                                .text_size(px(11.5))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(tokens.text_strong)
                                                .truncate()
                                                .child(SharedString::from(item.title.clone())),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(3.0))
                                        .child(
                                            div()
                                                .id(("gpui-dash-restart", pane_id))
                                                .w(px(18.0))
                                                .h(px(18.0))
                                                .rounded(px(3.0))
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .hover(|s| s.bg(tokens.panel))
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    cx.stop_propagation();
                                                    this.dispatch_action(
                                                        GpuiAction::SidePanels(
                                                            SidePanelsAction::RestartPane(pane_id),
                                                        ),
                                                        cx,
                                                    );
                                                }))
                                                .child(
                                                    GpuiIcon::RefreshCw
                                                        .render(10.5, tokens.text_dim),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .id(("gpui-dash-close", pane_id))
                                                .w(px(18.0))
                                                .h(px(18.0))
                                                .rounded(px(3.0))
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .hover(|s| s.bg(tokens.panel))
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    cx.stop_propagation();
                                                    this.dispatch_action(
                                                        GpuiAction::SidePanels(
                                                            SidePanelsAction::ClosePane(pane_id),
                                                        ),
                                                        cx,
                                                    );
                                                }))
                                                .child(
                                                    GpuiIcon::Close.render(10.5, tokens.text_dim),
                                                ),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(10.0))
                                .text_color(tokens.text_dim)
                                .truncate()
                                .child(SharedString::from(format!(
                                    "{} · {}",
                                    item.project_name, item.agent_kind
                                ))),
                        )
                        .when_some(item.task_line.clone(), |el, tline| {
                            el.child(
                                div()
                                    .text_size(px(10.5))
                                    .text_color(tokens.text)
                                    .truncate()
                                    .child(SharedString::from(tline)),
                            )
                        }),
                );
            }
            list = list.child(grp_col);
        }
    }

    div()
        .size_full()
        .p(px(8.0))
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(panel_query_inputs.render(FocusInputTarget::DashboardQuery, french, tokens, cx))
        .child(filter_bar)
        .child(controls)
        .child(list)
        .into_any_element()
}

// ---------------------------------------------------------------------------
// 2. `SideTab::Files` (`ExplorerPanel.vue`)
// ---------------------------------------------------------------------------

fn render_explorer_tab(
    sp: &GpuiSidePanelsSnapshot,
    explorer_input: &super::explorer_input::Input,
    panel_query_inputs: &super::panel_query_inputs::Inputs,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let ex = &sp.explorer;
    if !ex.has_root {
        return div()
            .size_full()
            .p(px(12.0))
            .child(render_empty_note(
                if french {
                    "Ouvrez un projet pour parcourir ses fichiers."
                } else {
                    "Select a workspace project to browse files."
                },
                tokens,
            ))
            .into_any_element();
    }

    let toolbar = div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(4.0))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(11.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(tokens.text_strong)
                .truncate()
                .child(SharedString::from(ex.root_title.clone())),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(3.0))
                .child(render_chip(
                    "gpui-ex-mode",
                    if ex.content_search_mode {
                        if french { "Contenu" } else { "Content" }.to_owned()
                    } else {
                        if french { "Nom" } else { "Name" }.to_owned()
                    },
                    ex.content_search_mode,
                    tokens,
                    GpuiAction::ExplorerToggleSearchMode,
                    cx,
                ))
                .child(render_chip(
                    "gpui-ex-new-file",
                    "+F".to_owned(),
                    false,
                    tokens,
                    GpuiAction::ExplorerNewItem { dir: false },
                    cx,
                ))
                .child(render_chip(
                    "gpui-ex-new-dir",
                    "+D".to_owned(),
                    false,
                    tokens,
                    GpuiAction::ExplorerNewItem { dir: true },
                    cx,
                ))
                .child(render_chip(
                    "gpui-ex-dotfiles",
                    ".*".to_owned(),
                    ex.show_dotfiles,
                    tokens,
                    GpuiAction::ExplorerToggleDotfiles,
                    cx,
                ))
                .child(render_chip(
                    "gpui-ex-compact",
                    "a/b".to_owned(),
                    ex.compact_folders,
                    tokens,
                    GpuiAction::ExplorerToggleCompact,
                    cx,
                ))
                .child(
                    div()
                        .id("gpui-ex-refresh")
                        .w(px(22.0))
                        .h(px(22.0))
                        .rounded(px(tokens.radius_px - 1.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .hover(|s| s.bg(tokens.surface_2))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.dispatch_action(GpuiAction::ExplorerRefresh, cx);
                        }))
                        .child(GpuiIcon::RefreshCw.render(11.5, tokens.text_dim)),
                ),
        );

    let mut scroll = div()
        .id("gpui-explorer-scroll")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(1.5));

    if !ex.error.is_empty() {
        scroll = scroll.child(
            div()
                .p(px(6.0))
                .rounded(px(tokens.radius_px - 1.0))
                .bg(rgba_with_alpha(tokens.danger, 0.14))
                .text_size(px(11.0))
                .text_color(tokens.danger)
                .child(SharedString::from(ex.error.clone())),
        );
    }

    if ex.content_search_mode && !ex.filter_query.trim().is_empty() {
        for (idx, hit) in ex.content_hits.iter().enumerate() {
            let hit_path = hit.path.clone();
            let hit_line = hit.line;
            scroll = scroll.child(
                div()
                    .id(("gpui-ex-hit", idx as u64))
                    .px(px(6.0))
                    .py(px(4.0))
                    .rounded(px(tokens.radius_px - 1.0))
                    .bg(tokens.surface)
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .hover(|s| s.bg(tokens.surface_2))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.dispatch_action(
                            GpuiAction::ExplorerOpenContentHit {
                                path: hit_path.clone(),
                                line: hit_line,
                            },
                            cx,
                        );
                    }))
                    .child(
                        div()
                            .font_family(mono_font_family())
                            .text_size(px(10.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(tokens.accent)
                            .truncate()
                            .child(SharedString::from(format!(
                                "{}:{}",
                                hit.rel_display, hit.line
                            ))),
                    )
                    .child(
                        div()
                            .font_family(mono_font_family())
                            .text_size(px(10.5))
                            .text_color(tokens.text)
                            .truncate()
                            .child(SharedString::from(hit.preview.clone())),
                    ),
            );
        }
    } else {
        for (idx, row) in ex.rows.iter().enumerate() {
            let row_key = row.key.clone();
            let del_path = row.path.clone();
            let indent = (row.depth as f32) * 12.0;
            let icon = if row.dir {
                if row.open {
                    GpuiIcon::ChevronDown
                } else {
                    GpuiIcon::ChevronRight
                }
            } else {
                GpuiIcon::FileCode
            };

            scroll = scroll.child(
                div()
                    .id(("gpui-ex-row", idx as u64))
                    .pl(px(6.0 + indent))
                    .pr(px(6.0))
                    .py(px(3.0))
                    .rounded(px(tokens.radius_px - 1.0))
                    .bg(if row.active_doc || row.selected {
                        tokens.accent_soft
                    } else {
                        gpui::rgba(0x0000_0000)
                    })
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(4.0))
                    .hover(|s| s.bg(tokens.surface_2))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.dispatch_action(
                            GpuiAction::ExplorerClickRow {
                                key: row_key.clone(),
                                pinned: true,
                            },
                            cx,
                        );
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(5.0))
                            .child(icon.render(
                                11.5,
                                if row.dir {
                                    tokens.accent
                                } else {
                                    tokens.text_dim
                                },
                            ))
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(if row.active_doc {
                                        tokens.text_strong
                                    } else {
                                        tokens.text
                                    })
                                    .truncate()
                                    .child(SharedString::from(row.display_name.clone())),
                            ),
                    )
                    .child(
                        div()
                            .id(("gpui-ex-del", idx as u64))
                            .w(px(18.0))
                            .h(px(18.0))
                            .rounded(px(3.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(|s| s.bg(tokens.surface))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.dispatch_action(
                                    GpuiAction::ExplorerDeletePath {
                                        path: del_path.clone(),
                                        permanent: false,
                                    },
                                    cx,
                                );
                            }))
                            .child(GpuiIcon::Trash2.render(10.0, tokens.text_dim)),
                    ),
            );
        }
    }

    let searching = !ex.filter_query.trim().is_empty();
    let count = if ex.content_search_mode && searching {
        ex.content_hits.len()
    } else {
        ex.rows.len()
    };
    if ex.truncated {
        let message = if !searching {
            if french {
                "Certains fichiers ne sont pas affichés : limite atteinte.".to_owned()
            } else {
                "Some files are not shown: limit reached.".to_owned()
            }
        } else if count == 0 {
            if french {
                "La recherche s'est arrêtée avant la fin : limite atteinte.".to_owned()
            } else {
                "The search stopped early: limit reached.".to_owned()
            }
        } else if french {
            if count == 1 {
                "Seul le premier résultat est affiché. Précisez la recherche.".to_owned()
            } else {
                format!(
                    "Seuls les {count} premiers résultats sont affichés. Précisez la recherche."
                )
            }
        } else if count == 1 {
            "Only the first result is shown. Type more to narrow the search.".to_owned()
        } else {
            format!("Only the first {count} results are shown. Type more to narrow the search.")
        };
        scroll = scroll.child(
            div()
                .px(px(6.))
                .py(px(3.))
                .text_size(px(11.))
                .text_color(tokens.warn)
                .child(message),
        );
    }
    if ex.loading && count == 0 {
        scroll = scroll.child(
            div()
                .px(px(6.))
                .py(px(3.))
                .text_size(px(11.))
                .text_color(tokens.text_dim)
                .child(match (searching, french) {
                    (true, true) => "Recherche…",
                    (true, false) => "Searching…",
                    (false, true) => "Chargement…",
                    (false, false) => "Loading…",
                }),
        );
    }

    let mut heading = div().flex().items_center().child(render_chip(
        "explorer-outline-toggle",
        if french { "Plan" } else { "Outline" }.into(),
        sp.outline_open,
        tokens,
        GpuiAction::OutlineToggle,
        cx,
    ));
    if sp.outline_open
        && let Some((id, tree)) = &sp.outline
    {
        let expand = crate::outline_tree::all_collapsed(tree, &sp.outline_folds);
        heading = heading.child(render_chip(
            "explorer-outline-fold-all",
            crate::json_outline_panel::fold_all_label(expand, french).into(),
            false,
            tokens,
            GpuiAction::OutlineRow(
                *id,
                tree.clone(),
                crate::app::gpui_bridge::OutlineRowAction::SetAllFolds { expand },
            ),
            cx,
        ));
    }
    if sp.outline_open {
        let mut options = div().relative().child(render_chip(
            "outline-options",
            "⋯".into(),
            sp.outline_options_open,
            tokens,
            GpuiAction::OutlineOptions(!sp.outline_options_open),
            cx,
        ));
        if sp.outline_options_open {
            let mut menu = div()
                .id("outline-action-menu")
                .flex()
                .flex_col()
                .p(px(4.))
                .w(px(180.))
                .bg(tokens.surface)
                .border_1()
                .border_color(tokens.border)
                .rounded(px(4.))
                .shadow_lg()
                .occlude()
                .on_mouse_down_out(cx.listener(|root, _: &gpui::MouseDownEvent, _, cx| {
                    root.dispatch_action(GpuiAction::OutlineOptions(false), cx);
                }));
            menu = menu.child(render_chip(
                "outline-follow",
                if french {
                    "Suivre le curseur"
                } else {
                    "Follow Cursor"
                }
                .into(),
                sp.outline_follow_cursor,
                tokens,
                GpuiAction::OutlineFollow(!sp.outline_follow_cursor),
                cx,
            ));
            menu = menu.child(render_chip(
                "outline-filter-mode",
                if french {
                    "Filtrer à la frappe"
                } else {
                    "Filter on Type"
                }
                .into(),
                sp.outline_filter_on_type,
                tokens,
                GpuiAction::OutlineFilterMode(!sp.outline_filter_on_type),
                cx,
            ));
            for (sort, en, fr) in crate::json_outline_panel::SORT_OPTIONS {
                menu = menu.child(render_chip(
                    en,
                    if french { fr } else { en }.into(),
                    sp.outline_sort == sort,
                    tokens,
                    GpuiAction::OutlineSort(sort),
                    cx,
                ));
            }
            options = options.child(
                gpui::deferred(
                    gpui::anchored()
                        .offset(point(px(0.), px(20.)))
                        .snap_to_window_with_margin(px(4.))
                        .child(menu),
                )
                .priority(2),
            );
        }
        heading = heading.child(options);
    }
    let mut outline = div().flex().flex_col().child(heading);
    if sp.outline_open {
        let mut reveal_index = None;
        let mut body = div()
            .id("explorer-outline-scroll")
            .track_scroll(&explorer_input.outline_scroll)
            .max_h(px(160.0))
            .overflow_y_scroll()
            .flex()
            .flex_col();
        if let Some((id, tree)) = &sp.outline {
            let owner = *id;
            let focus = explorer_input
                .outline_focus
                .get_or_init(|| cx.focus_handle())
                .clone();
            body = body.track_focus(&focus).on_key_down(cx.listener(
                move |root, event: &gpui::KeyDownEvent, _, cx| {
                    let k = &event.keystroke;
                    if k.modifiers.control
                        || k.modifiers.platform
                        || k.modifiers.alt
                        || root.app.outline_document_id() != Some(owner)
                    {
                        return;
                    }
                    use eframe::egui::Key;
                    let key = match k.key.as_str() {
                        "down" => Some(Key::ArrowDown),
                        "up" => Some(Key::ArrowUp),
                        "left" => Some(Key::ArrowLeft),
                        "right" => Some(Key::ArrowRight),
                        "home" => Some(Key::Home),
                        "end" => Some(Key::End),
                        "enter" => Some(Key::Enter),
                        _ => None,
                    };
                    if let Some(key) = key {
                        root.dispatch_action(GpuiAction::OutlineNavigate(owner, key), cx);
                    } else if k.key == "escape" {
                        root.dispatch_action(GpuiAction::OutlineCloseQuery(owner), cx);
                    } else if let Some(seed) = k
                        .key_char
                        .as_deref()
                        .or(Some(k.key.as_str()))
                        .filter(|s| s.chars().count() == 1 && !s.trim().is_empty())
                    {
                        root.focus_input = FocusInputTarget::OutlineQuery;
                        root.dispatch_action(
                            GpuiAction::OutlineStartQuery(owner, seed.to_owned()),
                            cx,
                        );
                    } else {
                        return;
                    }
                    cx.stop_propagation();
                },
            ));
            let query = sp.outline_query.as_deref().unwrap_or_default().trim();
            let rows = crate::outline_tree::visible_rows(
                tree,
                &sp.outline_folds,
                query,
                !sp.outline_filter_on_type,
            );
            if sp.outline_query.is_some() {
                outline = outline.child(panel_query_inputs.render_with_hint(
                    FocusInputTarget::OutlineQuery,
                    french,
                    sp.outline_filter_on_type.then_some(if french {
                        "Taper pour filtrer"
                    } else {
                        "Type to filter"
                    }),
                    tokens,
                    cx,
                ));
                if !query.is_empty() && !rows.iter().any(|entry| !entry.row.matches.is_empty()) {
                    outline = outline.child(
                        div()
                            .id("outline-no-results")
                            .debug_selector(|| "outline-no-results".into())
                            .child(if french {
                                "Aucun résultat trouvé."
                            } else {
                                "No results found."
                            }),
                    );
                }
            }
            for (index, entry) in rows.into_iter().enumerate() {
                let symbol = entry.symbol;
                let depth = entry.row.depth;
                let selected = sp.outline_selected.as_ref() == Some(&symbol.key);
                if selected
                    && sp.outline_scroll_generation != 0
                    && explorer_input
                        .last_outline_scroll
                        .replace(Some((*id, sp.outline_scroll_generation)))
                        != Some((*id, sp.outline_scroll_generation))
                {
                    reveal_index = Some(index);
                }
                let range = symbol.selection_range.unwrap_or(symbol.range);
                use crate::app::gpui_bridge::OutlineRowAction;
                let open = entry.row.open;
                let mut row = div()
                    .when(selected, |row| row.bg(tokens.accent_soft))
                    .pl(px(8.0 + depth as f32 * 12.0))
                    .flex()
                    .items_center();
                if !symbol.children.is_empty() {
                    row = row.child(render_chip(
                        format!("outline-fold-{}", symbol.key),
                        if open { "▾" } else { "▸" }.into(),
                        false,
                        tokens,
                        GpuiAction::OutlineRow(
                            *id,
                            tree.clone(),
                            OutlineRowAction::Fold(symbol.key.clone()),
                        ),
                        cx,
                    ));
                }
                let row_key = symbol.key.clone();
                let selection_key = row_key.clone();
                let full_range = symbol.range;
                let id = *id;
                let snapshot = tree.clone();
                let row_focus = focus.clone();
                let hits = crate::outline_tree::highlight_ranges(&symbol.name, &entry.row.matches);
                let label = gpui::StyledText::new(SharedString::from(symbol.name.clone()))
                    .with_highlights(hits.into_iter().map(|span| {
                        (
                            span,
                            gpui::HighlightStyle {
                                color: Some(gpui::rgb_to_hsla(tokens.accent)),
                                ..Default::default()
                            },
                        )
                    }));
                row = row.child(
                    div()
                        .id(SharedString::from(format!("outline-name-{}", symbol.key)))
                        .debug_selector(move || format!("outline-name-{row_key}"))
                        .cursor_pointer()
                        .child(label)
                        .on_click(cx.listener(
                            move |this, event: &gpui::ClickEvent, window, cx| {
                                let select = event.click_count() == 2;
                                if !select {
                                    this.focus_input = FocusInputTarget::OutlineTree;
                                    window.focus(&row_focus, cx);
                                }
                                let target = if select { full_range } else { range };
                                this.dispatch_action(
                                    GpuiAction::OutlineRow(
                                        id,
                                        snapshot.clone(),
                                        OutlineRowAction::Reveal(
                                            selection_key.clone(),
                                            target,
                                            select,
                                        ),
                                    ),
                                    cx,
                                );
                            },
                        )),
                );
                body = body.child(row);
            }
            if tree.is_empty() {
                body = body.child(if french {
                    "Aucun symbole dans ce document."
                } else {
                    "No symbols found in this document."
                });
            }
        } else {
            body = body.child(if french {
                "L’éditeur actif ne fournit pas de plan."
            } else {
                "The active editor cannot provide outline information."
            });
        }
        outline = outline.relative().child(body);
        if let Some(index) = reveal_index {
            let scroll = explorer_input.outline_scroll.clone();
            // Children have their new bounds after prepaint, including newly unfolded rows.
            outline = outline.child(
                canvas(
                    |_, _, _| {},
                    move |_, (), window, _| {
                        let Some(row) = scroll.bounds_for_item(index) else {
                            return;
                        };
                        let viewport = scroll.bounds();
                        let offset = scroll.offset();
                        if row.top() + offset.y < viewport.top()
                            || row.bottom() + offset.y > viewport.bottom()
                        {
                            scroll
                                .set_offset(point(offset.x, viewport.center().y - row.center().y));
                            window.refresh();
                        }
                    },
                )
                .absolute()
                .size_full(),
            );
        }
    }

    div()
        .size_full()
        .p(px(8.0))
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(toolbar)
        .child(explorer_input.render(ex.content_search_mode, french, tokens, cx))
        .child(scroll)
        .child(outline)
        .child(render_timeline(&sp.timeline, french, tokens, cx))
        .into_any_element()
}

// ---------------------------------------------------------------------------
// 3. `SideTab::Changes` (`ScmPanel.vue` + `ScmHistoryPanel.vue` Swimlanes)
// ---------------------------------------------------------------------------

fn render_scm_changes_tab(
    sp: &GpuiSidePanelsSnapshot,
    scm_input: &super::scm_input::Input,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let branch_lbl = sp.scm_branch.clone().unwrap_or_else(|| "HEAD".to_owned());

    let top_bar = div()
        .flex()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap(px(4.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(5.0))
                .child(GpuiIcon::GitBranch.render(12.5, tokens.accent))
                .child(
                    div()
                        .font_family(mono_font_family())
                        .text_size(px(11.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(tokens.text_strong)
                        .child(SharedString::from(branch_lbl)),
                )
                .when(sp.scm_ahead > 0 || sp.scm_behind > 0, |el| {
                    el.child(
                        div()
                            .font_family(mono_font_family())
                            .text_size(px(10.5))
                            .text_color(tokens.text_dim)
                            .child(SharedString::from(format!(
                                "↑{} ↓{}",
                                sp.scm_ahead, sp.scm_behind
                            ))),
                    )
                }),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(3.0))
                .child(render_chip(
                    "gpui-scm-fetch",
                    "Fetch".to_owned(),
                    false,
                    tokens,
                    GpuiAction::ScmRemoteOp(ScmRemoteOp::Fetch),
                    cx,
                ))
                .child(render_chip(
                    "gpui-scm-pull",
                    "Pull".to_owned(),
                    false,
                    tokens,
                    GpuiAction::ScmRemoteOp(ScmRemoteOp::Pull),
                    cx,
                ))
                .child(render_chip(
                    "gpui-scm-push",
                    "Push".to_owned(),
                    false,
                    tokens,
                    GpuiAction::ScmRemoteOp(ScmRemoteOp::Push),
                    cx,
                ))
                .child(render_chip(
                    "gpui-scm-notes",
                    "Notes".to_owned(),
                    false,
                    tokens,
                    GpuiAction::OpenNotesModal,
                    cx,
                ))
                .child(render_chip(
                    "gpui-scm-review",
                    "Review".to_owned(),
                    false,
                    tokens,
                    GpuiAction::OpenReviewModal,
                    cx,
                ))
                .child(
                    div()
                        .id("gpui-scm-refresh")
                        .w(px(22.0))
                        .h(px(22.0))
                        .rounded(px(tokens.radius_px - 1.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .hover(|s| s.bg(tokens.surface_2))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.dispatch_action(GpuiAction::ScmRefresh, cx);
                        }))
                        .child(GpuiIcon::RefreshCw.render(11.5, tokens.text_dim)),
                ),
        );

    let commit_row = scm_input.render(
        sp.scm_commit_busy,
        !sp.scm_busy
            && sp
                .scm_entries
                .iter()
                .any(|e| e.area == ScmArea::Staged && !e.is_conflict()),
        french,
        tokens,
        cx,
    );

    let mut changes_scroll = div()
        .id("gpui-scm-changes-scroll")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(6.0));

    if let Some(err) = &sp.scm_error {
        changes_scroll = changes_scroll.child(
            div()
                .p(px(6.0))
                .rounded(px(tokens.radius_px - 1.0))
                .bg(rgba_with_alpha(tokens.danger, 0.14))
                .text_size(px(11.0))
                .text_color(tokens.danger)
                .child(SharedString::from(err.clone())),
        );
    }

    for (area, heading_en, heading_fr) in [
        (ScmArea::Staged, "STAGED", "INDEXÉS"),
        (ScmArea::Unstaged, "CHANGES", "MODIFIÉS"),
        (ScmArea::Untracked, "UNTRACKED", "NON SUIVIS"),
    ] {
        let entries: Vec<_> = sp.scm_entries.iter().filter(|e| e.area == area).collect();
        if entries.is_empty() {
            continue;
        }
        let all_paths: Vec<String> = entries.iter().map(|e| e.path.clone()).collect();
        let bulk_action = if area == ScmArea::Staged {
            GpuiAction::ScmUnstage(all_paths)
        } else {
            GpuiAction::ScmStage(all_paths)
        };
        let bulk_lbl = if area == ScmArea::Staged {
            if french {
                "Tout désindexer"
            } else {
                "Unstage all"
            }
        } else if french {
            "Tout indexer"
        } else {
            "Stage all"
        };

        let mut sec = div().flex().flex_col().gap(px(2.0)).child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(px(10.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(tokens.text_dim)
                        .child(SharedString::from(format!(
                            "{} ({})",
                            if french { heading_fr } else { heading_en },
                            entries.len()
                        ))),
                )
                .child(render_chip(
                    format!("gpui-scm-bulk-{area:?}"),
                    bulk_lbl.to_owned(),
                    false,
                    tokens,
                    bulk_action,
                    cx,
                )),
        );

        for (e_idx, entry) in entries.iter().enumerate() {
            let path = entry.path.clone();
            let old_path = entry.old_path.clone();
            let stage_path = path.clone();
            let discard_path = path.clone();
            let entry_area = entry.area;
            let st_code = entry.status.code();

            sec = sec.child(
                div()
                    .id(SharedString::from(format!(
                        "gpui-scm-entry-{entry_area:?}-{e_idx}"
                    )))
                    .px(px(6.0))
                    .py(px(3.5))
                    .rounded(px(tokens.radius_px - 1.0))
                    .bg(tokens.surface)
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(4.0))
                    .hover(|s| s.bg(tokens.surface_2))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.dispatch_action(
                            GpuiAction::ScmOpenDiff {
                                area: entry_area,
                                path: path.clone(),
                                old_path: old_path.clone(),
                                pinned: true,
                            },
                            cx,
                        );
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .font_family(mono_font_family())
                                    .text_size(px(10.5))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(tokens.accent)
                                    .child(st_code),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .font_family(mono_font_family())
                                    .text_size(px(11.0))
                                    .text_color(tokens.text_strong)
                                    .truncate()
                                    .child(SharedString::from(entry.path.clone())),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(3.0))
                            .child(
                                div()
                                    .id(SharedString::from(format!(
                                        "gpui-scm-toggle-{entry_area:?}-{e_idx}"
                                    )))
                                    .px(px(5.0))
                                    .py(px(1.0))
                                    .rounded(px(3.0))
                                    .bg(tokens.surface_2)
                                    .text_size(px(10.5))
                                    .text_color(tokens.text_strong)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        let act = if entry_area == ScmArea::Staged {
                                            GpuiAction::ScmUnstage(vec![stage_path.clone()])
                                        } else {
                                            GpuiAction::ScmStage(vec![stage_path.clone()])
                                        };
                                        this.dispatch_action(act, cx);
                                    }))
                                    .child(if entry_area == ScmArea::Staged {
                                        "−"
                                    } else {
                                        "+"
                                    }),
                            )
                            .when(entry_area != ScmArea::Staged, |el| {
                                el.child(
                                    div()
                                        .id(SharedString::from(format!(
                                            "gpui-scm-discard-{entry_area:?}-{e_idx}"
                                        )))
                                        .px(px(5.0))
                                        .py(px(1.0))
                                        .rounded(px(3.0))
                                        .bg(tokens.surface_2)
                                        .text_size(px(10.5))
                                        .text_color(tokens.danger)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.dispatch_action(
                                                GpuiAction::ScmRequestDiscard {
                                                    area: entry_area,
                                                    paths: vec![discard_path.clone()],
                                                },
                                                cx,
                                            );
                                        }))
                                        .child("↺"),
                                )
                            }),
                    ),
            );
        }
        changes_scroll = changes_scroll.child(sec);
    }

    if sp.scm_entries.is_empty() {
        changes_scroll = changes_scroll.child(render_empty_note(
            if french {
                "Arbre de travail propre."
            } else {
                "Working tree clean."
            },
            tokens,
        ));
    }

    let commits_dock = render_scm_commits_dock(sp, french, tokens, cx);

    div()
        .size_full()
        .p(px(8.0))
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(top_bar)
        .child(commit_row)
        .child(changes_scroll)
        .child(commits_dock)
        .into_any_element()
}

fn render_scm_commits_dock(
    sp: &GpuiSidePanelsSnapshot,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let header = div()
        .id("gpui-scm-commits-toggle")
        .w_full()
        .px(px(6.0))
        .py(px(4.0))
        .rounded(px(tokens.radius_px - 1.0))
        .bg(tokens.surface)
        .border_1()
        .border_color(tokens.border)
        .flex()
        .items_center()
        .justify_between()
        .hover(|s| s.bg(tokens.surface_2))
        .on_click(cx.listener(|this, _, _, cx| {
            this.dispatch_action(GpuiAction::ScmToggleCommitsOpen, cx);
        }))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(5.0))
                .child(
                    if sp.commits_open {
                        GpuiIcon::ChevronDown
                    } else {
                        GpuiIcon::ChevronRight
                    }
                    .render(11.5, tokens.text_dim),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(tokens.text_strong)
                        .child(if french { "Historique Git" } else { "Commits" }),
                ),
        )
        .child(
            div()
                .font_family(mono_font_family())
                .text_size(px(10.5))
                .text_color(tokens.text_dim)
                .child(SharedString::from(sp.commits.len().to_string())),
        );

    if !sp.commits_open {
        return header.into_any_element();
    }

    let mut list = div()
        .id("gpui-scm-commits-scroll")
        .h(px(210.0))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(2.0));

    for (c_idx, row) in sp.commits.iter().take(120).enumerate() {
        let hash = row.node.hash.clone();
        let expanded = sp.expanded_commits.contains(&hash);
        let lane = row.lane.min(5);
        let active_lanes = row.active_lanes.clamp(1, 6);
        let color_idx = row.color_index % SWIMLANE_PALETTE.len();
        let node_color = rgb_tuple(SWIMLANE_PALETTE[color_idx]);
        let graph_w = (active_lanes as f32 * 10.0 + 8.0).max(22.0);

        let swimlane_canvas = canvas(
            |_, _, _| (),
            move |bounds, (), window, _cx| {
                let ox = bounds.origin.x;
                let oy = bounds.origin.y;
                let h = bounds.size.height;
                for l in 0..active_lanes {
                    let x = ox + px(6.0 + l as f32 * 10.0);
                    let col = rgb_tuple(SWIMLANE_PALETTE[l % SWIMLANE_PALETTE.len()]);
                    let mut pb = PathBuilder::stroke(px(1.5));
                    pb.move_to(point(x, oy));
                    pb.line_to(point(x, oy + h));
                    if let Ok(path) = pb.build() {
                        window.paint_path(path, col);
                    }
                }
                let nx = ox + px(6.0 + lane as f32 * 10.0);
                let ny = oy + px(11.0);
                let mut dot_pb = PathBuilder::fill();
                dot_pb.move_to(point(nx - px(3.2), ny));
                dot_pb.line_to(point(nx, ny - px(3.2)));
                dot_pb.line_to(point(nx + px(3.2), ny));
                dot_pb.line_to(point(nx, ny + px(3.2)));
                dot_pb.close();
                if let Ok(path) = dot_pb.build() {
                    window.paint_path(path, node_color);
                }
            },
        )
        .w(px(graph_w))
        .h(px(22.0));

        let toggle_hash = hash.clone();
        list = list.child(
            div()
                .id(("gpui-scm-commit-row", c_idx as u64))
                .px(px(4.0))
                .rounded(px(tokens.radius_px - 1.0))
                .flex()
                .items_center()
                .gap(px(5.0))
                .hover(|s| s.bg(tokens.surface_2))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.dispatch_action(
                        GpuiAction::SidePanels(SidePanelsAction::ToggleExpandCommit(
                            toggle_hash.clone(),
                        )),
                        cx,
                    );
                }))
                .child(swimlane_canvas)
                .child(
                    div()
                        .font_family(mono_font_family())
                        .text_size(px(10.0))
                        .text_color(tokens.accent)
                        .child(SharedString::from(row.node.short_hash.clone())),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(11.0))
                        .text_color(tokens.text_strong)
                        .truncate()
                        .child(SharedString::from(row.node.subject.clone())),
                ),
        );

        if expanded && let Some(files) = sp.commit_files.get(&hash) {
            for (f_idx, cf) in files.iter().enumerate() {
                let c_hash = hash.clone();
                let c_path = cf.path.clone();
                list = list.child(
                    div()
                        .id(SharedString::from(format!(
                            "gpui-scm-cfile-{c_idx}-{f_idx}"
                        )))
                        .pl(px(graph_w + 14.0))
                        .pr(px(6.0))
                        .py(px(2.0))
                        .rounded(px(3.0))
                        .flex()
                        .items_center()
                        .gap(px(5.0))
                        .hover(|s| s.bg(tokens.surface))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.dispatch_action(
                                GpuiAction::SidePanels(SidePanelsAction::OpenCommitDiff {
                                    commit_hash: c_hash.clone(),
                                    path: c_path.clone(),
                                    pinned: true,
                                }),
                                cx,
                            );
                        }))
                        .child(
                            div()
                                .font_family(mono_font_family())
                                .text_size(px(10.0))
                                .text_color(tokens.accent)
                                .child(SharedString::from(cf.status.clone())),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .font_family(mono_font_family())
                                .text_size(px(10.5))
                                .text_color(tokens.text)
                                .truncate()
                                .child(SharedString::from(cf.path.clone())),
                        ),
                );
            }
        }
    }

    div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(header)
        .child(list)
        .into_any_element()
}

// ---------------------------------------------------------------------------
// 4. `SideTab::Tasks` (`TaskBoard.vue` stacked status sections)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn render_tasks_board_tab(
    sp: &GpuiSidePanelsSnapshot,
    panel_query_inputs: &super::panel_query_inputs::Inputs,
    task_rename_input: &super::task_rename_input::Input,
    task_card_input: &super::task_card_input::Inputs,
    task_agent_inputs: &super::task_agent_input::Inputs,
    task_decision_inputs: &super::task_decision_input::Inputs,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    use super::task_card_input::{Control, Drag, Ghost, control, head_control};
    use crate::native_task_board::Command;
    let Some(scope) = sp.task_board.scope else {
        return render_empty_note(
            if french {
                "Sélectionnez d’abord un projet."
            } else {
                "Select a project first."
            },
            tokens,
        );
    };
    let agents = std::sync::Arc::new(sp.task_agents.clone());
    let action = |command| GpuiAction::NativeTaskBoard { scope, command };
    let mut columns = div()
        .id("gpui-tasks-scroll")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .pt(px(6.))
        .px(px(12.))
        .pb(px(16.));
    for (column, en, fr, color) in [
        (0, "To do", "À faire", 0x8b93a1),
        (1, "Doing", "En cours", 0x6c9cff),
        (2, "Review", "À examiner", 0xf5a524),
        (3, "Done", "Terminé", 0x3ddc84),
    ] {
        let accent = gpui::rgb(color);
        let cards: Vec<_> = sp.tasks.iter().filter(|t| t.column == column).collect();
        let mut header = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .pt(px(4.))
            .px(px(2.))
            .pb(px(8.))
            .child(div().w(px(8.)).h(px(8.)).rounded(px(4.)).bg(accent))
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(tokens.text_strong)
                    .child(if french { fr } else { en }),
            )
            .child(
                div()
                    .min_w(px(18.))
                    .px(px(6.))
                    .rounded(px(9.))
                    .bg(tokens.surface_3)
                    .line_height(px(17.))
                    .text_center()
                    .text_size(px(10.5))
                    .text_color(tokens.text_dim)
                    .child(cards.len().to_string()),
            );
        let mut tools = div().flex().flex_wrap().gap(px(4.)).ml_auto();
        if column == 3 && !cards.is_empty() {
            if sp.task_board.selecting {
                let all = cards.iter().all(|t| sp.task_board.picked.contains(&t.id));
                tools = tools
                    .child(head_control(
                        "task-all",
                        if all {
                            if french { "Aucun" } else { "None" }
                        } else if french {
                            "Tout"
                        } else {
                            "All"
                        }
                        .into(),
                        Some(action(Command::ToggleAll)),
                        false,
                        tokens,
                        cx,
                    ))
                    .child(head_control(
                        "task-delete-picked",
                        format!(
                            "{} ({})",
                            if french { "Supprimer" } else { "Delete" },
                            sp.task_board.picked.len()
                        ),
                        (!sp.task_board.picked.is_empty()).then(|| action(Command::RequestDelete)),
                        true,
                        tokens,
                        cx,
                    ))
                    .child(head_control(
                        "task-selection",
                        if french { "Annuler" } else { "Cancel" }.into(),
                        Some(action(Command::ToggleSelection)),
                        false,
                        tokens,
                        cx,
                    ));
            } else {
                tools = tools.child(head_control(
                    "task-selection",
                    if french { "Sélectionner" } else { "Select" }.into(),
                    Some(action(Command::ToggleSelection)),
                    false,
                    tokens,
                    cx,
                ));
            }
        }
        header = header.child(tools);
        let col = div().pt(px(10.)).flex().flex_col().child(header);
        let mut body = div()
            .id(("task-drop", column))
            .role(gpui::Role::List)
            .debug_selector(move || format!("task-drop-{column}"))
            .flex()
            .flex_col()
            .gap(px(6.))
            .min_h(px(32.))
            .on_drag_over::<Drag>(move |style, drag, _, _| {
                if drag.scope == scope {
                    style.bg(rgba_with_alpha(accent, 0.12))
                } else {
                    style
                }
            })
            .on_drop(cx.listener(move |root, drag: &Drag, _, cx| {
                if drag.scope == scope {
                    root.dispatch_action(
                        GpuiAction::NativeTaskBoard {
                            scope,
                            command: Command::Move {
                                id: drag.id,
                                column,
                            },
                        },
                        cx,
                    );
                }
            }));
        if cards.is_empty() {
            body = body.child(
                div()
                    .p(px(10.))
                    .border_1()
                    .border_dashed()
                    .border_color(tokens.border_strong)
                    .rounded(px(tokens.radius_px))
                    .text_color(gpui::rgb(0x5d6572))
                    .text_size(px(11.5))
                    .text_center()
                    .child(if french { "Aucune tâche" } else { "No tasks" }),
            );
        }
        for task in cards {
            let id = task.id;
            let editing = sp.task_board.edit.as_ref().is_some_and(|e| e.id == id);
            let focus = task_card_input
                .handle(id)
                .expect("board focus synchronized")
                .clone();
            let click_focus = focus.clone();
            let row_focus = focus.clone();
            let selecting = column == 3 && sp.task_board.selecting;
            let selected = selecting && sp.task_board.picked.contains(&id);
            let drag = Drag {
                scope,
                id,
                title: task.title.clone(),
            };
            let ghost_tokens = *tokens;
            let dragging = cx
                .active_drag()
                .and_then(|d| d.value.downcast_ref::<Drag>())
                .is_some_and(|d| d.scope == scope && d.id == id);
            let title = if editing {
                task_rename_input.render(french, tokens, cx)
            } else {
                div()
                    .id(("task-title", id))
                    .debug_selector(move || format!("task-title-{id}"))
                    .flex_1()
                    .min_w_0()
                    .text_size(px(12.5))
                    .line_height(px(17.5))
                    .whitespace_normal()
                    .text_color(tokens.text_strong)
                    .on_click(cx.listener(move |root, event: &gpui::ClickEvent, w, cx| {
                        w.focus(&click_focus, cx);
                        if column == 0 && event.click_count() == 2 {
                            root.dispatch_action(
                                GpuiAction::NativeTaskBoard {
                                    scope,
                                    command: Command::BeginRename(id),
                                },
                                cx,
                            );
                        }
                    }))
                    .child(task.title.clone())
                    .into_any_element()
            };
            let mut row = div()
                .id(("task-card", id))
                .role(gpui::Role::ListItem)
                .aria_label(task.title.clone())
                .aria_keyshortcuts("Alt+ArrowLeft Alt+ArrowRight")
                .debug_selector(move || format!("task-card-{id}"))
                .track_focus(&focus)
                .tab_stop(!editing)
                .flex_shrink_0()
                .when(dragging, |r| r.opacity(0.45))
                .on_key_down(cx.listener(move |root, event: &gpui::KeyDownEvent, w, cx| {
                    if !focus.is_focused(w) {
                        return;
                    }
                    let k = &event.keystroke;
                    if k.modifiers.alt
                        && !k.modifiers.control
                        && !k.modifiers.platform
                        && !k.modifiers.shift
                        && matches!(k.key.as_str(), "left" | "right")
                    {
                        cx.stop_propagation();
                        if !event.is_held {
                            super::task_card_input::step(
                                root,
                                scope,
                                id,
                                if k.key == "left" { -1 } else { 1 },
                                cx,
                            );
                        }
                    }
                }))
                .p(px(10.))
                .rounded(px(tokens.radius_px))
                .bg(if selected {
                    tokens.accent_soft_bg
                } else {
                    tokens.surface_2
                })
                .border_1()
                .border_color(if selected {
                    tokens.accent
                } else {
                    tokens.border
                })
                .border_l(px(3.))
                .relative()
                .overflow_hidden()
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .w(px(3.))
                        .bg(accent),
                )
                .flex()
                .items_start()
                .gap(px(6.))
                .on_click(cx.listener(move |root, _, w, cx| {
                    if !editing {
                        w.focus(&row_focus, cx);
                        if selecting {
                            root.dispatch_action(
                                GpuiAction::NativeTaskBoard {
                                    scope,
                                    command: Command::TogglePick(id),
                                },
                                cx,
                            );
                        }
                    }
                }))
                .when(!editing, |row| {
                    row.cursor(gpui::CursorStyle::OpenHand)
                        .on_drag(drag, move |drag, _, _, cx| {
                            cx.new(|_| Ghost {
                                title: drag.title.clone(),
                                tokens: ghost_tokens,
                            })
                        })
                });

            if selecting {
                row = row.child(control(
                    format!("task-pick-{id}"),
                    if french {
                        "Sélectionner la tâche"
                    } else {
                        "Select task"
                    },
                    Control::Pick(selected),
                    action(Command::TogglePick(id)),
                    tokens,
                    cx,
                ));
            }
            let timing = crate::task_card_timing::cached(&task.timeline, column, french);
            let mut content = div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(title);
            if !editing {
                if task.report.is_some()
                    && let Some(handle) = task_card_input.report_handle(id)
                {
                    content = content.child(super::task_report_view::render(
                        task,
                        scope,
                        sp.task_board.open_reports.contains(&id),
                        handle,
                        french,
                        tokens,
                        cx,
                    ));
                }
                content = content.child(super::task_decision_input::render(
                    task_decision_inputs,
                    scope,
                    task,
                    french,
                    tokens,
                    cx,
                ));
                content = content.child(super::task_agent_input::render(
                    task_agent_inputs,
                    scope,
                    task,
                    agents.clone(),
                    french,
                    tokens,
                    cx,
                ));
                if matches!(column, 1 | 2)
                    && let Some(pane) = task.timeline.pane_id
                    && agents.iter().any(|p| p.id == pane)
                {
                    content = content.child(
                        div()
                            .id(("task-show-boundary", id))
                            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation()
                            })
                            .on_click(|_, _, cx| cx.stop_propagation())
                            .child(head_control(
                                "task-show-agent",
                                if french { "Voir l'agent" } else { "Show agent" }.into(),
                                Some(action(Command::ShowAgent { id, pane })),
                                false,
                                tokens,
                                cx,
                            )),
                    );
                }
            }
            if !timing.text.is_empty() {
                let tooltip = timing.tooltip;
                let theme = *tokens;
                content = content.child(
                    div()
                        .id(("task-timing", id))
                        .debug_selector(move || format!("task-timing-{id}"))
                        .text_size(px(10.5))
                        .text_color(tokens.text_dim)
                        .whitespace_normal()
                        .tooltip(move |_, cx| {
                            cx.new(|_| TaskTimingTooltip {
                                text: tooltip.clone(),
                                tokens: theme,
                            })
                            .into()
                        })
                        .child(timing.text),
                );
            }
            row = row.child(content);
            if column == 0 && !editing {
                row = row.child(control(
                    format!("task-rename-{id}"),
                    if french {
                        "Renommer la tâche"
                    } else {
                        "Rename task"
                    },
                    Control::Rename,
                    action(Command::BeginRename(id)),
                    tokens,
                    cx,
                ));
            }
            if !selecting {
                row = row.child(control(
                    format!("task-delete-{id}"),
                    if french {
                        "Supprimer la tâche"
                    } else {
                        "Delete task"
                    },
                    Control::Delete,
                    action(Command::RequestDeleteOne(id)),
                    tokens,
                    cx,
                ));
            }
            body = body.child(row);
        }
        columns = columns.child(col.child(body));
    }
    div()
        .id("native-task-board")
        .track_focus(
            task_card_input
                .board_handle()
                .expect("board focus synchronized"),
        )
        .tab_stop(false)
        .size_full()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(panel_query_inputs.render(FocusInputTarget::TaskNewTitle, french, tokens, cx))
        .child(columns)
        .into_any_element()
}

// ---------------------------------------------------------------------------
// 5. `SideTab::TaskHistory` (`TaskHistoryPanel.vue`)
// ---------------------------------------------------------------------------

fn render_task_history_tab(
    sp: &GpuiSidePanelsSnapshot,
    panel_query_inputs: &super::panel_query_inputs::Inputs,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let mut win_chips = div().flex().flex_wrap().items_center().gap(px(3.0));
    for w in TaskHistoryFilter::ALL {
        win_chips = win_chips.child(render_chip(
            format!("gpui-th-win-{w:?}"),
            w.label(french).to_owned(),
            sp.history_window == w,
            tokens,
            GpuiAction::SetTaskHistoryWindow(w),
            cx,
        ));
    }

    let mut sort_bar = div().flex().flex_wrap().items_center().gap(px(3.0));
    for s in TaskHistorySort::ALL {
        sort_bar = sort_bar.child(render_chip(
            format!("gpui-th-sort-{s:?}"),
            s.label(french).to_owned(),
            sp.history_sort == s,
            tokens,
            GpuiAction::SetTaskHistorySort(s),
            cx,
        ));
    }
    sort_bar = sort_bar.child(render_chip(
        "gpui-th-proj-only",
        if french {
            "Projet actif"
        } else {
            "Project only"
        }
        .to_owned(),
        sp.history_current_project_only,
        tokens,
        GpuiAction::ToggleTaskHistoryCurrentProject,
        cx,
    ));

    let csv_payload = history_csv(&sp.task_history_rows);
    let actions_row = div()
        .flex()
        .items_center()
        .gap(px(4.0))
        .child(render_chip(
            "gpui-th-export-csv",
            "Export CSV".to_owned(),
            false,
            tokens,
            GpuiAction::SidePanels(SidePanelsAction::ExportTaskHistoryCsv(csv_payload)),
            cx,
        ))
        .child(render_chip(
            "gpui-th-clear",
            if french { "Effacer" } else { "Clear" }.to_owned(),
            false,
            tokens,
            GpuiAction::SidePanels(SidePanelsAction::ClearTaskHistory),
            cx,
        ));

    let mut list = div()
        .id("gpui-th-scroll")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(4.0));

    if sp.task_history_rows.is_empty() {
        list = list.child(render_empty_note(
            if french {
                "Aucune exécution terminée."
            } else {
                "No completed task runs yet."
            },
            tokens,
        ));
    } else {
        for rec in &sp.task_history_rows {
            list = list.child(
                div()
                    .p(px(7.0))
                    .rounded(px(tokens.radius_px))
                    .bg(tokens.surface)
                    .border_1()
                    .border_color(tokens.border)
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(tokens.text_strong)
                            .child(SharedString::from(rec.title.clone())),
                    )
                    .child(
                        div()
                            .font_family(mono_font_family())
                            .text_size(px(10.0))
                            .text_color(tokens.text_dim)
                            .child(SharedString::from(format!(
                                "{} · {} · {} · ${:.3}",
                                rec.project_name,
                                rec.agent_name,
                                format_duration_compact(rec.duration_ms),
                                rec.cost_usd
                            ))),
                    ),
            );
        }
    }

    div()
        .size_full()
        .p(px(8.0))
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(panel_query_inputs.render(FocusInputTarget::TaskHistoryQuery, french, tokens, cx))
        .child(win_chips)
        .child(sort_bar)
        .child(actions_row)
        .child(list)
        .into_any_element()
}

// ---------------------------------------------------------------------------
// 6. `SideTab::History` (`SessionHistoryPanel.vue`)
// ---------------------------------------------------------------------------

pub(crate) fn render_sessions_history_tab(
    sp: &GpuiSidePanelsSnapshot,
    panel_query_inputs: &super::panel_query_inputs::Inputs,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let mut scope_bar = div().flex().flex_wrap().items_center().gap(px(3.0));
    for sc in SessionHistoryScope::ALL {
        scope_bar = scope_bar.child(render_chip(
            format!("gpui-sh-scope-{sc:?}"),
            sc.label(french).to_owned(),
            sp.sessions_scope == sc,
            tokens,
            GpuiAction::SetSessionsScope(sc),
            cx,
        ));
    }

    let group_bar = div()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(3.0))
                .child(render_chip(
                    "gpui-sh-grp-folder",
                    SessionHistoryGroupBy::Folder.label(french).to_owned(),
                    sp.sessions_group == SessionHistoryGroupBy::Folder,
                    tokens,
                    GpuiAction::SetSessionsGroup(SessionHistoryGroupBy::Folder),
                    cx,
                ))
                .child(render_chip(
                    "gpui-sh-grp-agent",
                    SessionHistoryGroupBy::Agent.label(french).to_owned(),
                    sp.sessions_group == SessionHistoryGroupBy::Agent,
                    tokens,
                    GpuiAction::SetSessionsGroup(SessionHistoryGroupBy::Agent),
                    cx,
                )),
        )
        .child(
            div()
                .id("gpui-sh-refresh")
                .w(px(22.0))
                .h(px(22.0))
                .rounded(px(tokens.radius_px - 1.0))
                .flex()
                .items_center()
                .justify_center()
                .hover(|s| s.bg(tokens.surface_2))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.dispatch_action(
                        GpuiAction::SidePanels(SidePanelsAction::RefreshSessions),
                        cx,
                    );
                }))
                .child(GpuiIcon::RefreshCw.render(11.5, tokens.text_dim)),
        );

    let mut list = div()
        .id("gpui-sh-scroll")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(4.0));

    if sp.past_sessions.is_empty() && !sp.sessions_loading && !sp.sessions_limited {
        list = list.child(render_empty_note(
            if french {
                "Aucune session passée trouvée."
            } else {
                "No past transcripts found."
            },
            tokens,
        ));
    } else {
        for (idx, entry) in sp.past_sessions.iter().enumerate() {
            let resume_act = if let Some(pid) = entry.open_pane_id {
                GpuiAction::SidePanels(SidePanelsAction::FocusPane(pid))
            } else {
                GpuiAction::SidePanels(SidePanelsAction::ResumeSession {
                    profile_binding: entry.profile_binding.clone(),
                    agent_id: entry.agent_id.clone(),
                    session_id: entry.id.clone(),
                    folder: entry.folder.clone(),
                })
            };
            let del_entry = entry.clone();

            list = list.child(
                div()
                    .id(("gpui-sh-row", idx as u64))
                    .p(px(7.0))
                    .rounded(px(tokens.radius_px))
                    .bg(tokens.surface)
                    .border_1()
                    .border_color(tokens.border)
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(6.0))
                    .hover(|s| s.bg(tokens.surface_2))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.dispatch_action(resume_act.clone(), cx);
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(tokens.text_strong)
                                    .truncate()
                                    .child(SharedString::from(entry.title.clone())),
                            )
                            .child(
                                div()
                                    .font_family(mono_font_family())
                                    .text_size(px(10.0))
                                    .text_color(tokens.text_dim)
                                    .truncate()
                                    .child(SharedString::from(format!(
                                        "{} · {} · {}",
                                        entry.agent_id,
                                        crate::side_panels_model::session_profile_label(
                                            entry, french
                                        ),
                                        entry.folder.display()
                                    ))),
                            ),
                    )
                    .child(
                        div()
                            .id(("gpui-sh-del", idx as u64))
                            .w(px(20.0))
                            .h(px(20.0))
                            .rounded(px(3.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(|s| s.bg(tokens.panel))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.dispatch_action(
                                    GpuiAction::SidePanels(SidePanelsAction::DeleteSession(
                                        del_entry.clone(),
                                    )),
                                    cx,
                                );
                            }))
                            .child(GpuiIcon::Trash2.render(10.5, tokens.text_dim)),
                    ),
            );
        }
    }

    if sp.sessions_loading {
        list = list.child(render_empty_note(
            if french {
                "Recherche dans les conversations…"
            } else {
                "Searching conversations…"
            },
            tokens,
        ));
    }
    if sp.sessions_limited {
        list = list.child(render_empty_note(if french { "Recherche partielle : certains transcripts n’ont pas pu être lus ou la limite a été atteinte." } else { "Partial search: some transcripts could not be read or the search limit was reached." }, tokens));
    }
    div()
        .size_full()
        .p(px(8.0))
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(panel_query_inputs.render(FocusInputTarget::SessionsQuery, french, tokens, cx))
        .child(render_chip(
            "gpui-sh-transcripts",
            if french {
                "Chercher aussi dans les transcripts"
            } else {
                "Search transcript contents too"
            }
            .to_owned(),
            sp.sessions_include_transcripts,
            tokens,
            GpuiAction::ToggleSessionsTranscripts,
            cx,
        ))
        .child(scope_bar)
        .child(group_bar)
        .child(list)
        .into_any_element()
}

// ---------------------------------------------------------------------------
// 7. `SideTab::Conversations` (`ConversationsPanel.vue`)
// ---------------------------------------------------------------------------

fn render_conversations_tab(
    sp: &GpuiSidePanelsSnapshot,
    panel_query_inputs: &super::panel_query_inputs::Inputs,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let mut win_bar = div().flex().flex_wrap().items_center().gap(px(3.0));
    for w in ConversationWindow::ALL {
        win_bar = win_bar.child(render_chip(
            format!("gpui-conv-win-{w:?}"),
            w.label(french).to_owned(),
            sp.conv_window == w,
            tokens,
            GpuiAction::SetConversationsWindow(w),
            cx,
        ));
    }

    let selected = sp
        .selected_thread_key
        .as_ref()
        .and_then(|k| sp.threads.iter().find(|t| &t.key == k))
        .or_else(|| sp.threads.first());

    let mut thread_list = div()
        .id("gpui-conv-threads")
        .max_h(px(160.0))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(3.0));

    for (idx, th) in sp.threads.iter().enumerate() {
        let is_sel = selected.is_some_and(|s| s.key == th.key);
        let key = th.key.clone();
        thread_list = thread_list.child(
            div()
                .id(("gpui-conv-th", idx as u64))
                .px(px(7.0))
                .py(px(4.5))
                .rounded(px(tokens.radius_px - 1.0))
                .bg(if is_sel {
                    tokens.accent_soft
                } else {
                    tokens.surface
                })
                .border_1()
                .border_color(if is_sel { tokens.accent } else { tokens.border })
                .flex()
                .items_center()
                .justify_between()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.dispatch_action(
                        GpuiAction::SelectConversationThread(Some(key.clone())),
                        cx,
                    );
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(11.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(tokens.text_strong)
                        .truncate()
                        .child(SharedString::from(th.title.clone())),
                )
                .child(
                    div()
                        .font_family(mono_font_family())
                        .text_size(px(10.0))
                        .text_color(tokens.text_dim)
                        .child(SharedString::from(th.messages.len().to_string())),
                ),
        );
    }

    let detail = if let Some(th) = selected {
        let copy_txt = format_thread_as_text(th);
        let mut msgs = div()
            .id("gpui-conv-messages")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(5.0));
        for msg in &th.messages {
            msgs = msgs.child(
                div()
                    .p(px(6.0))
                    .rounded(px(tokens.radius_px - 1.0))
                    .bg(tokens.surface)
                    .border_1()
                    .border_color(tokens.border)
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .font_family(mono_font_family())
                            .text_size(px(10.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(tokens.accent)
                            .child(SharedString::from(format!(
                                "{} → {}",
                                msg.sender, msg.recipient
                            ))),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(tokens.text)
                            .child(SharedString::from(msg.body.clone())),
                    ),
            );
        }
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(tokens.text_strong)
                            .truncate()
                            .child(SharedString::from(th.title.clone())),
                    )
                    .child(render_chip(
                        "gpui-conv-copy",
                        if french { "Copier" } else { "Copy" }.to_owned(),
                        false,
                        tokens,
                        GpuiAction::SidePanels(SidePanelsAction::CopyText(copy_txt)),
                        cx,
                    )),
            )
            .child(msgs)
            .into_any_element()
    } else {
        render_empty_note(
            if sp.conversations_loading {
                "…"
            } else if french {
                "Aucun fil de conversation."
            } else {
                "No conversation threads yet."
            },
            tokens,
        )
    };

    div()
        .size_full()
        .p(px(8.0))
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(panel_query_inputs.render(FocusInputTarget::ConversationsQuery, french, tokens, cx))
        .child(win_bar)
        .child(thread_list)
        .child(detail)
        .when(sp.conversations_loading, |view| {
            view.child(render_empty_note(
                if french {
                    "Chargement des messages récents…"
                } else {
                    "Loading recent messages…"
                },
                tokens,
            ))
        })
        .when(sp.conversations_limited, |view| {
            view.child(render_empty_note(
                if french {
                    "Aperçu partiel : limite atteinte ou fichiers indisponibles."
                } else {
                    "Partial preview: limit reached or unavailable files."
                },
                tokens,
            ))
        })
        .into_any_element()
}

// ---------------------------------------------------------------------------
// Shared Side-Panel Primitives
// ---------------------------------------------------------------------------

pub(crate) fn render_focus_input_box(
    id: impl Into<gpui::ElementId>,
    target: FocusInputTarget,
    focused: bool,
    value: &str,
    placeholder: &str,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let empty = value.is_empty();
    let shown = if empty {
        placeholder.to_owned()
    } else if focused {
        format!("{value}▍")
    } else {
        value.to_owned()
    };

    div()
        .id(id)
        .w_full()
        .h(px(28.0))
        .px(px(8.0))
        .rounded(px(tokens.radius_px))
        .bg(tokens.surface)
        .border_1()
        .border_color(if focused {
            tokens.accent
        } else {
            tokens.border
        })
        .flex()
        .items_center()
        .text_size(px(11.5))
        .text_color(if empty && !focused {
            tokens.text_dim
        } else {
            tokens.text_strong
        })
        .on_click(cx.listener(move |this, _, window, cx| {
            this.set_focus_input(target, window, cx);
        }))
        .child(SharedString::from(shown))
        .into_any_element()
}

fn render_chip(
    id: impl Into<SharedString>,
    label: String,
    active: bool,
    tokens: &GpuiThemeTokens,
    action: GpuiAction,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let id_shared: SharedString = id.into();
    div()
        .id(id_shared.clone())
        .debug_selector(move || id_shared.to_string())
        .px(px(6.0))
        .py(px(2.0))
        .rounded(px(tokens.radius_px - 1.0))
        .bg(if active {
            tokens.accent_soft
        } else {
            tokens.surface
        })
        .border_1()
        .border_color(if active { tokens.accent } else { tokens.border })
        .text_size(px(10.5))
        .text_color(if active {
            tokens.text_strong
        } else {
            tokens.text_dim
        })
        .hover(|s| s.bg(tokens.surface_2))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.dispatch_action(action.clone(), cx);
        }))
        .child(SharedString::from(label))
        .into_any_element()
}

fn render_timeline(
    snap: &crate::file_timeline::Snapshot,
    fr: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    use crate::file_timeline::Action;
    let mut section = div().flex().flex_col();
    section = section.child(render_chip(
        "timeline-toggle",
        if fr { "Chronologie" } else { "Timeline" }.into(),
        snap.shown,
        tokens,
        GpuiAction::Timeline(Action::Toggle),
        cx,
    ));
    if !snap.shown {
        return section.into_any_element();
    }
    let mut filter = div().relative().child(render_chip(
        "timeline-filter",
        if fr {
            "Filtrer la chronologie"
        } else {
            "Filter Timeline"
        }
        .into(),
        snap.filters_open,
        tokens,
        GpuiAction::Timeline(Action::Filters(!snap.filters_open)),
        cx,
    ));
    if snap.filters_open {
        use crate::file_timeline::Source;
        let mut menu = div()
            .id("timeline-source-menu")
            .flex()
            .flex_col()
            .w(px(176.))
            .p(px(4.))
            .bg(tokens.surface)
            .border_1()
            .border_color(tokens.border)
            .rounded(px(4.))
            .shadow_lg()
            .occlude()
            .on_mouse_down_out(cx.listener(|root, _: &gpui::MouseDownEvent, _, cx| {
                root.dispatch_action(GpuiAction::Timeline(Action::Filters(false)), cx);
            }));
        for (key, source) in [
            ("timeline-filter-git", Source::Git),
            ("timeline-filter-local", Source::Local),
        ] {
            menu = menu.child(render_chip(
                key,
                source.label(fr).into(),
                source.enabled(snap.sources),
                tokens,
                GpuiAction::Timeline(Action::ToggleSource(source)),
                cx,
            ));
        }
        filter = filter.child(
            gpui::deferred(
                gpui::anchored()
                    .offset(point(px(0.), px(20.)))
                    .snap_to_window_with_margin(px(4.))
                    .child(menu),
            )
            .priority(2),
        );
    }
    section = section.child(filter);
    if let Some(file) = &snap.file {
        section = section.child(
            div()
                .flex()
                .items_center()
                .child(snap.file_label(file, fr))
                .child(render_chip(
                    "timeline-pin",
                    snap.pin_label(fr).into(),
                    snap.pinned,
                    tokens,
                    GpuiAction::Timeline(Action::TogglePin(file.clone())),
                    cx,
                ))
                .child(render_chip(
                    "timeline-refresh",
                    if fr { "Actualiser" } else { "Refresh" }.into(),
                    false,
                    tokens,
                    GpuiAction::Timeline(Action::Refresh(file.clone(), false)),
                    cx,
                )),
        );
    }
    if let Some(status) = snap.status(fr) {
        section = section.child(render_empty_note(&status, tokens));
    }
    if let (Some(file), Some(Ok(history))) = (&snap.file, &snap.data) {
        let mut list = div()
            .id("timeline-entries")
            .max_h(px(160.))
            .overflow_y_scroll()
            .flex()
            .flex_col();
        for (index, row) in history
            .rows(chrono::Utc::now().timestamp(), fr)
            .into_iter()
            .enumerate()
        {
            let commit = row.key();
            let hover_key = commit.clone();
            let hover_history = history.clone();
            let theme = *tokens;
            let key = format!("timeline-{}", commit.as_deref().unwrap_or("working"));
            let path = file.clone();
            list = list.child(
                div()
                    .id(SharedString::from(key))
                    .debug_selector(move || format!("timeline-entry-{index}"))
                    .tooltip(move |_, cx| {
                        cx.new(|_| TimelineTooltip {
                            details: hover_history.details(
                                hover_key.as_deref(),
                                fr,
                                chrono::Utc::now().timestamp(),
                            ),
                            tokens: theme,
                        })
                        .into()
                    })
                    .cursor_pointer()
                    .text_size(px(12.))
                    .flex()
                    .gap_1()
                    .items_center()
                    .child(div().flex_1().overflow_hidden().child(row.label.to_owned()))
                    .children(
                        row.commit
                            .map(|c| div().overflow_hidden().child(c.author.clone())),
                    )
                    .children(row.age)
                    .on_click(cx.listener(move |root, event: &gpui::ClickEvent, _, cx| {
                        root.dispatch_action(
                            GpuiAction::Timeline(Action::Open(
                                path.clone(),
                                commit.clone(),
                                event.click_count() == 2,
                            )),
                            cx,
                        );
                    })),
            );
        }
        if history.more {
            list = list.child(render_chip(
                "timeline-more",
                if fr { "Charger la suite" } else { "Load More" }.into(),
                false,
                tokens,
                GpuiAction::Timeline(Action::Refresh(file.clone(), true)),
                cx,
            ));
        }
        section = section.child(list);
    }
    section.into_any_element()
}

fn render_empty_note(text: &str, tokens: &GpuiThemeTokens) -> AnyElement {
    div()
        .p(px(12.0))
        .rounded(px(tokens.radius_px))
        .bg(tokens.surface)
        .border_1()
        .border_color(tokens.border)
        .text_size(px(11.5))
        .text_color(tokens.text_dim)
        .child(SharedString::from(text.to_owned()))
        .into_any_element()
}

struct TimelineTooltip {
    details: Option<crate::file_timeline::Details>,
    tokens: GpuiThemeTokens,
}
impl gpui::Render for TimelineTooltip {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut panel = div()
            .debug_selector(|| "timeline-hover".into())
            .flex()
            .flex_col()
            .gap_1()
            .max_w(px(480.))
            .p(px(8.))
            .rounded(px(5.))
            .bg(self.tokens.surface_3)
            .text_color(self.tokens.text)
            .text_size(px(12.))
            .whitespace_normal();
        if let Some(d) = &self.details {
            let mut head = div().flex().items_center().gap_1();
            if let Some(icon) = &d.avatar {
                let mut grid = div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .p(px(2.))
                    .bg(gpui::rgb(0xf0f0f0));
                let color = rgb_tuple((icon.rgb[0], icon.rgb[1], icon.rgb[2]));
                for row in icon.cells.as_chunks::<5>().0 {
                    grid = grid.child(
                        div().flex().children(
                            row.iter()
                                .map(|on| div().size(px(4.)).when(*on, |cell| cell.bg(color))),
                        ),
                    );
                }
                head = head.child(grid);
            }
            panel = panel
                .child(head.child(d.heading.clone()))
                .child(d.text.clone())
                .children(d.hash.clone());
        }
        panel
    }
}

struct TaskTimingTooltip {
    text: String,
    tokens: GpuiThemeTokens,
}
impl gpui::Render for TaskTimingTooltip {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .p(px(8.))
            .rounded(px(5.))
            .bg(self.tokens.surface_3)
            .text_color(self.tokens.text)
            .text_size(px(11.))
            .whitespace_normal()
            .child(self.text.clone())
    }
}
