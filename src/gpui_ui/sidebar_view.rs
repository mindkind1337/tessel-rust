//! 1:1 `GPUI` Left Sidebar (`ProjectPanel.vue` parity):
//! - Collapsed `52px` workspace icon rail with initials, running/waiting status dot, `+`, and expand
//! - Expanded resizable sidebar with folder groups, project cards, session rows (`SessionTeamBadge`,
//!   Live Share guest pills, task lines, wake/sleep/restart/close), Git worktree rows, hidden
//!   worktree inbox banner, worktree cleanup button, orphan sessions, and `SidebarPreferences` chips.

use super::{
    DragTarget, TesselGpuiRoot,
    icons::GpuiIcon,
    theme::{GpuiThemeTokens, mono_font_family, rgba_with_alpha},
};
use crate::{
    app::gpui_bridge::{GpuiAction, GpuiProjectItem, GpuiSessionSidebarRow, GpuiWorkspaceSnapshot},
    project_panel::ProjectAction,
};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, div,
    prelude::FluentBuilder as _, px,
};

fn project_drop_target(
    row: gpui::Stateful<gpui::Div>,
    project: u64,
    accent: gpui::Rgba,
    cx: &mut Context<TesselGpuiRoot>,
) -> gpui::Stateful<gpui::Div> {
    use super::shell::SideBrowserDrag;
    row.on_drag_over::<SideBrowserDrag>(move |style, _, _, _| {
        style.bg(rgba_with_alpha(accent, 0.16))
    })
    .on_drop(cx.listener(move |root, drag: &SideBrowserDrag, _, cx| {
        root.dispatch_action(
            GpuiAction::UndockBrowserToProject {
                page: drag.0,
                project,
            },
            cx,
        );
    }))
}

pub(crate) fn render_left_sidebar_rail(
    snapshot: &GpuiWorkspaceSnapshot,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let mut rail = div()
        .id("gpui-left-sidebar-rail")
        .w(px(52.0))
        .h_full()
        .flex()
        .flex_col()
        .items_center()
        .py(px(8.0))
        .gap(px(6.0))
        .bg(tokens.panel)
        .border_r_1()
        .border_color(tokens.border);

    for proj in &snapshot.projects {
        let pid = proj.id;
        let is_active = proj.active;
        let status_dot = if proj.waiting_agents > 0 {
            Some(tokens.warn)
        } else if proj.running_agents > 0 {
            Some(tokens.ok_green)
        } else {
            None
        };

        rail = rail.child(
            div()
                .id(("gpui-rail-proj", pid))
                .map(|row| project_drop_target(row, pid, tokens.accent, cx))
                .debug_selector(move || format!("gpui-rail-proj-{pid}"))
                .relative()
                .w(px(36.0))
                .h(px(36.0))
                .rounded(px(tokens.radius_px + 1.0))
                .flex()
                .items_center()
                .justify_center()
                .bg(if is_active {
                    tokens.accent_soft
                } else {
                    tokens.surface
                })
                .border_1()
                .border_color(if is_active {
                    tokens.accent
                } else {
                    tokens.border
                })
                .text_size(px(12.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(if is_active {
                    tokens.text_strong
                } else {
                    tokens.text_dim
                })
                .hover(|s| s.bg(tokens.surface_2).text_color(tokens.text_strong))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.dispatch_action(GpuiAction::SelectProject(pid), cx);
                }))
                .child(SharedString::from(proj.initials.clone()))
                .when_some(status_dot, |el, dot_color| {
                    el.child(
                        div()
                            .absolute()
                            .right(px(-2.0))
                            .top(px(-2.0))
                            .w(px(8.0))
                            .h(px(8.0))
                            .rounded(px(4.0))
                            .bg(dot_color)
                            .border_1()
                            .border_color(tokens.panel),
                    )
                }),
        );
    }

    rail.child(
        div()
            .id("gpui-rail-add-project")
            .w(px(34.0))
            .h(px(34.0))
            .rounded(px(tokens.radius_px))
            .flex()
            .items_center()
            .justify_center()
            .bg(tokens.surface)
            .border_1()
            .border_color(tokens.border)
            .hover(|s| s.bg(tokens.surface_2))
            .on_click(cx.listener(|this, _, _, cx| {
                this.dispatch_action(GpuiAction::OpenAddProjectDialog, cx);
            }))
            .child(GpuiIcon::Plus.render(14.0, tokens.text_dim)),
    )
    .child(div().flex_1())
    .child(
        div()
            .id("gpui-rail-expand")
            .w(px(32.0))
            .h(px(28.0))
            .rounded(px(tokens.radius_px))
            .flex()
            .items_center()
            .justify_center()
            .hover(|s| s.bg(tokens.surface_2))
            .on_click(cx.listener(|this, _, _, cx| {
                this.dispatch_action(GpuiAction::ToggleLeftSidebar, cx);
            }))
            .child(GpuiIcon::SidebarLeft.render(14.0, tokens.text_dim)),
    )
    .into_any_element()
}

pub(crate) fn render_left_sidebar_expanded(
    snapshot: &GpuiWorkspaceSnapshot,
    tokens: &GpuiThemeTokens,
    left_width_px: f32,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let french = snapshot.french;
    let prefs = snapshot.sidebar.prefs;
    // The expanded sidebar has no Search button of its own: the toolbar's command box
    // (Ctrl+Shift+P) finds worktrees and browser tabs too (b9061124).
    let workspaces_label = if french {
        "ESPACES DE TRAVAIL"
    } else {
        "WORKSPACES"
    };

    let mut scroll = div()
        .id("gpui-sidebar-scroll")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .px(px(8.0))
        .py(px(6.0))
        .flex()
        .flex_col()
        .gap(px(6.0));

    // Group projects by optional `group_name` while preserving ungrouped projects first.
    let mut ungrouped_indices: Vec<usize> = Vec::new();
    let mut grouped_names: Vec<String> = Vec::new();
    for (idx, p) in snapshot.projects.iter().enumerate() {
        if let Some(g) = p.group_name.as_ref().filter(|s| !s.trim().is_empty()) {
            if !grouped_names.contains(g) {
                grouped_names.push(g.clone());
            }
        } else {
            ungrouped_indices.push(idx);
        }
    }

    for idx in ungrouped_indices {
        let proj = &snapshot.projects[idx];
        scroll = scroll.child(render_project_card(
            proj,
            prefs.compact_rows,
            prefs.hide_agent_task_line,
            french,
            tokens,
            cx,
        ));
    }

    for group_name in grouped_names {
        let collapsed = snapshot.sidebar.collapsed_groups.contains(&group_name);
        let group_toggle_name = group_name.clone();
        let chevron = if collapsed {
            GpuiIcon::ChevronRight
        } else {
            GpuiIcon::ChevronDown
        };
        let count_in_group = snapshot
            .projects
            .iter()
            .filter(|p| p.group_name.as_deref() == Some(group_name.as_str()))
            .count();

        scroll = scroll.child(
            div()
                .id(SharedString::from(format!("gpui-proj-group-{group_name}")))
                .w_full()
                .px(px(6.0))
                .py(px(4.0))
                .rounded(px(tokens.radius_px - 1.0))
                .flex()
                .items_center()
                .justify_between()
                .hover(|s| s.bg(tokens.surface))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.dispatch_action(
                        GpuiAction::Project(ProjectAction::ToggleProjectGroup(
                            group_toggle_name.clone(),
                        )),
                        cx,
                    );
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.0))
                        .child(chevron.render(12.0, tokens.text_dim))
                        .child(GpuiIcon::Folder.render(12.5, tokens.accent))
                        .child(
                            div()
                                .text_size(px(11.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(tokens.text_dim)
                                .child(SharedString::from(group_name.clone())),
                        ),
                )
                .child(
                    div()
                        .text_size(px(10.5))
                        .font_family(mono_font_family())
                        .text_color(tokens.text_dim)
                        .child(SharedString::from(count_in_group.to_string())),
                ),
        );

        if !collapsed {
            for proj in snapshot
                .projects
                .iter()
                .filter(|p| p.group_name.as_deref() == Some(group_name.as_str()))
            {
                scroll = scroll.child(render_project_card(
                    proj,
                    prefs.compact_rows,
                    prefs.hide_agent_task_line,
                    french,
                    tokens,
                    cx,
                ));
            }
        }
    }

    if !snapshot.sidebar.orphan_sessions.is_empty() {
        scroll = scroll.child(
            div()
                .pt(px(6.0))
                .px(px(4.0))
                .text_size(px(10.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(tokens.text_dim)
                .child(if french {
                    "SESSIONS SANS PROJET"
                } else {
                    "UNASSIGNED SESSIONS"
                }),
        );
        for sess in &snapshot.sidebar.orphan_sessions {
            scroll = scroll.child(render_session_row(
                sess,
                prefs.compact_rows,
                prefs.hide_agent_task_line,
                french,
                tokens,
                cx,
            ));
        }
    }

    let footer = render_sidebar_prefs_footer(prefs, french, tokens, cx);

    div()
        .h_full()
        .flex()
        .flex_row()
        .child(
            div()
                .w(px(left_width_px))
                .h_full()
                .flex()
                .flex_col()
                .bg(tokens.panel)
                .border_r_1()
                .border_color(tokens.border)
                // Top header + Search + Add project
                .child(
                    div()
                        .px(px(10.0))
                        .py(px(8.0))
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .border_b_1()
                        .border_color(tokens.border)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .text_size(px(10.5))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(tokens.text_dim)
                                        .child(workspaces_label),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(4.0))
                                        .child(
                                            div()
                                                .id("gpui-sidebar-add-project")
                                                .px(px(6.0))
                                                .h(px(22.0))
                                                .rounded(px(tokens.radius_px - 1.0))
                                                .flex()
                                                .items_center()
                                                .gap(px(4.0))
                                                .bg(tokens.surface)
                                                .border_1()
                                                .border_color(tokens.border)
                                                .text_size(px(11.0))
                                                .text_color(tokens.text_strong)
                                                .hover(|s| s.bg(tokens.surface_2))
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.dispatch_action(
                                                        GpuiAction::OpenAddProjectDialog,
                                                        cx,
                                                    );
                                                }))
                                                .child(
                                                    GpuiIcon::Plus.render(11.5, tokens.text_strong),
                                                )
                                                .child(if french { "Projet" } else { "Project" }),
                                        )
                                        .child(
                                            div()
                                                .id("gpui-sidebar-collapse")
                                                .w(px(22.0))
                                                .h(px(22.0))
                                                .rounded(px(tokens.radius_px - 1.0))
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .hover(|s| s.bg(tokens.surface_2))
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.dispatch_action(
                                                        GpuiAction::ToggleLeftSidebar,
                                                        cx,
                                                    );
                                                }))
                                                .child(
                                                    GpuiIcon::SidebarLeft
                                                        .render(12.5, tokens.text_dim),
                                                ),
                                        ),
                                ),
                        ),
                )
                .child(scroll)
                .child(footer),
        )
        // 4px right-edge drag handle (`DragTarget::LeftSidebar`)
        .child(
            div()
                .id("gpui-left-sidebar-resize")
                .w(px(4.0))
                .h_full()
                .hover(|s| s.bg(tokens.accent))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                        this.begin_drag(
                            DragTarget::LeftSidebar {
                                start_x: f32::from(ev.position.x),
                                start_width: left_width_px,
                            },
                            cx,
                        );
                    }),
                ),
        )
        .into_any_element()
}

fn render_project_card(
    proj: &GpuiProjectItem,
    compact: bool,
    hide_task_line: bool,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let pid = proj.id;
    let is_active = proj.active;

    let mut card = div()
        .w_full()
        .rounded(px(tokens.radius_px))
        .bg(if is_active {
            rgba_with_alpha(tokens.accent, 0.08)
        } else {
            tokens.surface
        })
        .border_1()
        .border_color(if is_active {
            tokens.accent
        } else {
            tokens.border
        })
        .flex()
        .flex_col()
        .overflow_hidden();

    // Project title row + quick actions
    card = card.child(
        div()
            .id(("gpui-proj-header", pid))
            .map(|row| project_drop_target(row, pid, tokens.accent, cx))
            .debug_selector(move || format!("gpui-proj-header-{pid}"))
            .px(px(8.0))
            .py(px(if compact { 5.0 } else { 7.0 }))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(6.0))
            .hover(|s| s.bg(tokens.surface_2))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.dispatch_action(GpuiAction::SelectProject(pid), cx);
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
                            .w(px(20.0))
                            .h(px(20.0))
                            .rounded(px(4.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(if is_active {
                                tokens.accent
                            } else {
                                tokens.surface_2
                            })
                            .text_size(px(10.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(if is_active {
                                tokens.accent_fg
                            } else {
                                tokens.text_strong
                            })
                            .child(SharedString::from(proj.initials.clone())),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(5.0))
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(tokens.text_strong)
                                            .truncate()
                                            .child(SharedString::from(proj.name.clone())),
                                    )
                                    .when_some(proj.git_branch.clone(), |el, branch| {
                                        el.child(
                                            div()
                                                .px(px(4.0))
                                                .py(px(1.0))
                                                .rounded(px(3.0))
                                                .bg(tokens.surface_2)
                                                .font_family(mono_font_family())
                                                .text_size(px(9.5))
                                                .text_color(tokens.text_dim)
                                                .child(SharedString::from(branch)),
                                        )
                                    }),
                            )
                            .when(!compact, |el| {
                                el.child(
                                    div()
                                        .text_size(px(10.0))
                                        .font_family(mono_font_family())
                                        .text_color(tokens.text_dim)
                                        .truncate()
                                        .child(SharedString::from(proj.path_display.clone())),
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.0))
                    .child(render_mini_icon_btn(
                        ("gpui-proj-new-term", pid),
                        GpuiIcon::Plus,
                        tokens,
                        GpuiAction::Project(ProjectAction::NewTerminal(Some(pid))),
                        cx,
                    ))
                    .child(render_mini_icon_btn(
                        ("gpui-proj-new-wt", pid),
                        GpuiIcon::GitBranch,
                        tokens,
                        GpuiAction::Project(ProjectAction::NewWorktree(pid)),
                        cx,
                    )),
            ),
    );

    // Secondary project management strip when active
    if is_active {
        card = card.child(
            div()
                .px(px(6.0))
                .py(px(3.0))
                .bg(rgba_with_alpha(tokens.surface_2, 0.65))
                .border_t_1()
                .border_color(tokens.border)
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(3.0))
                .child(render_tiny_text_btn(
                    ("gpui-proj-up", pid),
                    "↑",
                    tokens,
                    GpuiAction::Project(ProjectAction::MoveProject { id: pid, delta: -1 }),
                    cx,
                ))
                .child(render_tiny_text_btn(
                    ("gpui-proj-down", pid),
                    "↓",
                    tokens,
                    GpuiAction::Project(ProjectAction::MoveProject { id: pid, delta: 1 }),
                    cx,
                ))
                .child(render_tiny_text_btn(
                    ("gpui-proj-rename", pid),
                    if french { "Renommer" } else { "Rename" },
                    tokens,
                    GpuiAction::Project(ProjectAction::RenameProject(pid)),
                    cx,
                ))
                .child(render_tiny_text_btn(
                    ("gpui-proj-group-edit", pid),
                    if french { "Groupe" } else { "Group" },
                    tokens,
                    GpuiAction::Project(ProjectAction::EditProjectGroup(pid)),
                    cx,
                ))
                .child(render_tiny_text_btn(
                    ("gpui-proj-github", pid),
                    "GitHub",
                    tokens,
                    GpuiAction::Project(ProjectAction::OpenGitHub(pid)),
                    cx,
                ))
                .child(render_tiny_text_btn(
                    ("gpui-proj-teams", pid),
                    "Teams",
                    tokens,
                    GpuiAction::Project(ProjectAction::ManageTeams(pid)),
                    cx,
                ))
                .child(render_tiny_text_btn(
                    ("gpui-proj-sleep", pid),
                    if french { "Veille" } else { "Sleep" },
                    tokens,
                    GpuiAction::Project(ProjectAction::SleepProject(pid)),
                    cx,
                ))
                .child(render_tiny_text_btn(
                    ("gpui-proj-remove", pid),
                    "×",
                    tokens,
                    GpuiAction::Project(ProjectAction::RemoveProject(pid)),
                    cx,
                )),
        );
    }

    // Sessions list
    if !proj.sessions.is_empty() {
        let mut sess_box = div()
            .px(px(6.0))
            .py(px(4.0))
            .border_t_1()
            .border_color(tokens.border)
            .flex()
            .flex_col()
            .gap(px(2.0));
        for sess in &proj.sessions {
            sess_box = sess_box.child(render_session_row(
                sess,
                compact,
                hide_task_line,
                french,
                tokens,
                cx,
            ));
        }
        card = card.child(sess_box);
    }

    // Visible secondary Git worktrees + hidden worktree inbox + Clean up
    let show_inbox = !proj.inbox_dismissed && !proj.hidden_worktrees.is_empty();
    if !proj.visible_worktrees.is_empty() || show_inbox {
        let mut wt_section = div()
            .px(px(6.0))
            .py(px(4.0))
            .border_t_1()
            .border_color(tokens.border)
            .flex()
            .flex_col()
            .gap(px(3.0));

        wt_section = wt_section.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .px(px(2.0))
                .child(
                    div()
                        .text_size(px(10.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(tokens.text_dim)
                        .child("WORKTREES"),
                )
                .child(
                    div()
                        .id(("gpui-proj-wt-cleanup", pid))
                        .px(px(5.0))
                        .py(px(1.0))
                        .rounded(px(3.0))
                        .text_size(px(10.0))
                        .text_color(tokens.accent)
                        .hover(|s| s.bg(tokens.surface_2))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.dispatch_action(
                                GpuiAction::Project(ProjectAction::CleanUpWorktrees(pid)),
                                cx,
                            );
                        }))
                        .child(if french { "Nettoyer" } else { "Clean up" }),
                ),
        );

        for (wt_idx, wt) in proj.visible_worktrees.iter().enumerate() {
            let wt_path = wt.path.clone();
            let wt_branch = wt.branch.clone();
            let open_path = wt_path.clone();
            let open_branch = wt_branch.clone();
            let hide_path = wt_path.clone();
            let del_path = wt_path;
            let del_branch = wt_branch;

            wt_section = wt_section.child(
                div()
                    .id(SharedString::from(format!("gpui-wt-{pid}-{wt_idx}")))
                    .px(px(6.0))
                    .py(px(3.0))
                    .rounded(px(tokens.radius_px - 1.0))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(4.0))
                    .hover(|s| s.bg(tokens.surface_2))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.dispatch_action(
                            GpuiAction::Project(ProjectAction::OpenWorktreeTerminal {
                                project: pid,
                                path: open_path.clone(),
                                branch: open_branch.clone(),
                            }),
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
                            .child(GpuiIcon::GitBranch.render(11.0, tokens.accent))
                            .child(
                                div()
                                    .font_family(mono_font_family())
                                    .text_size(px(11.0))
                                    .text_color(tokens.text)
                                    .truncate()
                                    .child(SharedString::from(wt.branch.clone())),
                            )
                            .when(wt.dirty, |el| {
                                el.child(
                                    div().w(px(5.0)).h(px(5.0)).rounded(px(2.5)).bg(tokens.warn),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(3.0))
                            .child(
                                div()
                                    .id(SharedString::from(format!("gpui-wt-hide-{pid}-{wt_idx}")))
                                    .px(px(4.0))
                                    .rounded(px(3.0))
                                    .text_size(px(10.0))
                                    .text_color(tokens.text_dim)
                                    .hover(|s| s.bg(tokens.surface).text_color(tokens.text_strong))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.dispatch_action(
                                            GpuiAction::Project(ProjectAction::HideWorktree {
                                                project: pid,
                                                path: hide_path.clone(),
                                            }),
                                            cx,
                                        );
                                    }))
                                    .child(if french { "Masquer" } else { "Hide" }),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!("gpui-wt-del-{pid}-{wt_idx}")))
                                    .px(px(4.0))
                                    .rounded(px(3.0))
                                    .text_size(px(10.0))
                                    .text_color(tokens.danger)
                                    .hover(|s| s.bg(tokens.surface))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.dispatch_action(
                                            GpuiAction::Project(ProjectAction::DeleteWorktree {
                                                project: pid,
                                                path: del_path.clone(),
                                                branch: del_branch.clone(),
                                            }),
                                            cx,
                                        );
                                    }))
                                    .child("×"),
                            ),
                    ),
            );
        }

        if show_inbox {
            let first_hidden_path = proj.hidden_worktrees[0].path.clone();
            let hidden_count = proj.hidden_worktrees.len();
            wt_section = wt_section.child(
                div()
                    .px(px(6.0))
                    .py(px(4.0))
                    .rounded(px(tokens.radius_px - 1.0))
                    .bg(rgba_with_alpha(tokens.accent, 0.12))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(4.0))
                    .child(div().text_size(px(10.5)).text_color(tokens.text).child(
                        SharedString::from(if french {
                            format!("{hidden_count} worktree(s) détecté(s)")
                        } else {
                            format!("{hidden_count} hidden worktree(s)")
                        }),
                    ))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .child(
                                div()
                                    .id(("gpui-wt-inbox-show", pid))
                                    .px(px(5.0))
                                    .py(px(1.0))
                                    .rounded(px(3.0))
                                    .bg(tokens.accent)
                                    .text_size(px(10.0))
                                    .text_color(tokens.accent_fg)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.dispatch_action(
                                            GpuiAction::Project(
                                                ProjectAction::ShowHiddenWorktree {
                                                    project: pid,
                                                    path: first_hidden_path.clone(),
                                                },
                                            ),
                                            cx,
                                        );
                                    }))
                                    .child(if french { "Afficher" } else { "Show" }),
                            )
                            .child(
                                div()
                                    .id(("gpui-wt-inbox-dismiss", pid))
                                    .px(px(4.0))
                                    .text_size(px(10.5))
                                    .text_color(tokens.text_dim)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.dispatch_action(
                                            GpuiAction::Project(
                                                ProjectAction::DismissWorktreesInbox(pid),
                                            ),
                                            cx,
                                        );
                                    }))
                                    .child("×"),
                            ),
                    ),
            );
        }

        card = card.child(wt_section);
    }

    card.into_any_element()
}

fn render_session_row(
    sess: &GpuiSessionSidebarRow,
    compact: bool,
    hide_task_line: bool,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let sid = sess.id;
    let pid = sess.project_id;
    let dot_color = if sess.waiting {
        tokens.warn
    } else if sess.working {
        tokens.ok_green
    } else if sess.sleeping || sess.exited {
        tokens.text_dim
    } else {
        tokens.accent
    };

    let mut row = div()
        .id(("gpui-sidebar-sess", sid))
        .w_full()
        .px(px(6.0))
        .py(px(if compact { 3.0 } else { 4.5 }))
        .rounded(px(tokens.radius_px - 1.0))
        .bg(if sess.active {
            tokens.accent_soft
        } else {
            gpui::rgba(0x0000_0000)
        })
        .flex()
        .flex_col()
        .gap(px(2.0))
        .hover(|s| s.bg(tokens.surface_2))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.dispatch_action(
                GpuiAction::Project(ProjectAction::SelectSession {
                    project: pid,
                    id: sid,
                }),
                cx,
            );
        }));

    let mut top = div()
        .w_full()
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
                .child(div().w(px(6.0)).h(px(6.0)).rounded(px(3.0)).bg(dot_color))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(11.5))
                        .font_weight(if sess.active {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(if sess.sleeping {
                            tokens.text_dim
                        } else {
                            tokens.text_strong
                        })
                        .truncate()
                        .child(SharedString::from(sess.title.clone())),
                ),
        );

    let mut badges_and_actions = div().flex().items_center().gap(px(3.0));

    if let Some(tb) = &sess.team_badge {
        if !tb.team_number.is_empty() {
            let lbl = if tb.is_lead {
                format!("#{}·lead", tb.team_number)
            } else {
                format!("#{}", tb.team_number)
            };
            badges_and_actions = badges_and_actions.child(
                div()
                    .px(px(4.0))
                    .py(px(1.0))
                    .rounded(px(3.0))
                    .bg(rgba_with_alpha(tokens.accent, 0.18))
                    .font_family(mono_font_family())
                    .text_size(px(9.5))
                    .text_color(tokens.accent)
                    .child(SharedString::from(lbl)),
            );
        }
        if tb.unread_count > 0 {
            badges_and_actions = badges_and_actions.child(
                div()
                    .px(px(4.0))
                    .py(px(1.0))
                    .rounded(px(3.0))
                    .bg(rgba_with_alpha(tokens.warn, 0.22))
                    .font_family(mono_font_family())
                    .text_size(px(9.5))
                    .text_color(tokens.warn)
                    .child(SharedString::from(format!("✉ {}", tb.unread_count))),
            );
        }
    }

    for initials in sess.share_guests.iter().take(2) {
        badges_and_actions = badges_and_actions.child(
            div()
                .px(px(4.0))
                .py(px(1.0))
                .rounded(px(8.0))
                .bg(rgba_with_alpha(tokens.ok_green, 0.20))
                .text_size(px(9.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(tokens.ok_green)
                .child(SharedString::from(initials.clone())),
        );
    }

    if sess.sleeping {
        badges_and_actions = badges_and_actions.child(
            div()
                .id(("gpui-sess-wake", sid))
                .px(px(4.0))
                .py(px(1.0))
                .rounded(px(3.0))
                .bg(tokens.surface_2)
                .text_size(px(9.5))
                .text_color(tokens.accent)
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.dispatch_action(GpuiAction::WakeSession(sid), cx);
                }))
                .child(if french { "Réveiller" } else { "Wake" }),
        );
    } else {
        badges_and_actions = badges_and_actions.child(
            div()
                .id(("gpui-sess-sleep", sid))
                .w(px(16.0))
                .h(px(16.0))
                .rounded(px(3.0))
                .flex()
                .items_center()
                .justify_center()
                .hover(|s| s.bg(tokens.surface))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.dispatch_action(
                        GpuiAction::Project(ProjectAction::SleepSession {
                            project: pid,
                            id: sid,
                        }),
                        cx,
                    );
                }))
                .child(GpuiIcon::Moon.render(10.0, tokens.text_dim)),
        );
    }

    badges_and_actions = badges_and_actions
        .child(
            div()
                .id(("gpui-sess-restart", sid))
                .w(px(16.0))
                .h(px(16.0))
                .rounded(px(3.0))
                .flex()
                .items_center()
                .justify_center()
                .hover(|s| s.bg(tokens.surface))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.dispatch_action(
                        GpuiAction::Project(ProjectAction::RestartSession {
                            project: pid,
                            id: sid,
                        }),
                        cx,
                    );
                }))
                .child(GpuiIcon::RefreshCw.render(10.0, tokens.text_dim)),
        )
        .child(
            div()
                .id(("gpui-sess-close", sid))
                .w(px(16.0))
                .h(px(16.0))
                .rounded(px(3.0))
                .flex()
                .items_center()
                .justify_center()
                .hover(|s| s.bg(tokens.surface))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.dispatch_action(
                        GpuiAction::Project(ProjectAction::CloseSession {
                            project: pid,
                            id: sid,
                        }),
                        cx,
                    );
                }))
                .child(GpuiIcon::Close.render(10.0, tokens.text_dim)),
        );

    top = top.child(badges_and_actions);
    row = row.child(top);

    if !hide_task_line && let Some(task) = sess.task_line.as_ref().filter(|t| !t.trim().is_empty())
    {
        row = row.child(
            div()
                .pl(px(11.0))
                .text_size(px(10.0))
                .text_color(tokens.text_dim)
                .truncate()
                .child(SharedString::from(task.clone())),
        );
    }

    row.into_any_element()
}

fn render_sidebar_prefs_footer(
    prefs: crate::workspace::SidebarPreferences,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let mut p_act = prefs;
    p_act.sort_by_activity = !p_act.sort_by_activity;

    let mut p_cmp = prefs;
    p_cmp.compact_rows = !p_cmp.compact_rows;

    let mut p_slp = prefs;
    p_slp.hide_sleeping = !p_slp.hide_sleeping;

    let mut p_tsk = prefs;
    p_tsk.hide_agent_task_line = !p_tsk.hide_agent_task_line;

    div()
        .px(px(8.0))
        .py(px(6.0))
        .border_t_1()
        .border_color(tokens.border)
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(4.0))
        .child(render_pref_chip(
            "gpui-sb-pref-act",
            if french { "Activité" } else { "Activity" },
            prefs.sort_by_activity,
            tokens,
            GpuiAction::SetSidebarPrefs(p_act),
            cx,
        ))
        .child(render_pref_chip(
            "gpui-sb-pref-cmp",
            "Compact",
            prefs.compact_rows,
            tokens,
            GpuiAction::SetSidebarPrefs(p_cmp),
            cx,
        ))
        .child(render_pref_chip(
            "gpui-sb-pref-slp",
            if french {
                "Masquer veille"
            } else {
                "Hide sleep"
            },
            prefs.hide_sleeping,
            tokens,
            GpuiAction::SetSidebarPrefs(p_slp),
            cx,
        ))
        .child(render_pref_chip(
            "gpui-sb-pref-tsk",
            if french { "Sans tâche" } else { "Hide tasks" },
            prefs.hide_agent_task_line,
            tokens,
            GpuiAction::SetSidebarPrefs(p_tsk),
            cx,
        ))
        .into_any_element()
}

fn render_pref_chip(
    id: &'static str,
    label: &'static str,
    active: bool,
    tokens: &GpuiThemeTokens,
    action: GpuiAction,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    div()
        .id(id)
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
        .text_size(px(10.0))
        .text_color(if active {
            tokens.text_strong
        } else {
            tokens.text_dim
        })
        .hover(|s| s.bg(tokens.surface_2))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.dispatch_action(action.clone(), cx);
        }))
        .child(label)
        .into_any_element()
}

fn render_mini_icon_btn(
    id: (&'static str, u64),
    icon: GpuiIcon,
    tokens: &GpuiThemeTokens,
    action: GpuiAction,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    div()
        .id(id)
        .w(px(20.0))
        .h(px(20.0))
        .rounded(px(tokens.radius_px - 1.0))
        .flex()
        .items_center()
        .justify_center()
        .hover(|s| s.bg(tokens.surface))
        .on_click(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            this.dispatch_action(action.clone(), cx);
        }))
        .child(icon.render(11.5, tokens.text_dim))
        .into_any_element()
}

fn render_tiny_text_btn(
    id: (&'static str, u64),
    label: &'static str,
    tokens: &GpuiThemeTokens,
    action: GpuiAction,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    div()
        .id(id)
        .px(px(5.0))
        .py(px(2.0))
        .rounded(px(3.0))
        .bg(tokens.surface)
        .border_1()
        .border_color(tokens.border)
        .text_size(px(10.0))
        .text_color(tokens.text_dim)
        .hover(|s| s.bg(tokens.surface_2).text_color(tokens.text_strong))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.dispatch_action(action.clone(), cx);
        }))
        .child(label)
        .into_any_element()
}
