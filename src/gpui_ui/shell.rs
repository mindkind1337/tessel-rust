//! 4-zone CSS Flexbox workspace shell (`Left Sidebar | Center ViewTabs + PaneGrid | Right SidePanel`)
//! matching `C:\Tessel\src\renderer\src\App.vue` & `style.css`.

use super::{
    DragTarget, TesselGpuiRoot,
    icons::GpuiIcon,
    theme::{
        GpuiThemeTokens, PANE_CHIP_HIDE_ACCOUNT_BELOW_PX, PANE_CHIP_HIDE_MODEL_BELOW_PX,
        PANE_NAV_HEIGHT_PX, SIDEBAR_RAIL_WIDTH_PX, VIEW_TABS_HEIGHT_PX, mono_font_family,
        rgba_with_alpha,
    },
};
use crate::{
    app::gpui_bridge::{
        GpuiAction, GpuiCenterMode, GpuiDividerSnapshot, GpuiPaneTileSnapshot, GpuiProjectItem,
        GpuiWorkspaceSnapshot,
    },
    pane_grid::{GridPreset, SplitDir},
    theme_palettes::THEMES,
    workspace::{RendererPreference, SideTab},
};
use gpui::{
    Context, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, div, prelude::FluentBuilder as _, px,
};

pub(crate) fn render_workspace_body(
    snapshot: &GpuiWorkspaceSnapshot,
    tokens: &GpuiThemeTokens,
    left_width_px: f32,
    right_width_px: f32,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let hide_center = snapshot.right_fullscreen && snapshot.toolbar.right_open;

    div()
        .id("gpui-workspace-body")
        .flex_1()
        .w_full()
        .min_h_0()
        .flex()
        .overflow_hidden()
        .bg(tokens.backdrop)
        .child(if snapshot.toolbar.left_open {
            render_left_sidebar_expanded(snapshot, tokens, left_width_px, cx).into_any_element()
        } else {
            render_left_sidebar_rail(snapshot, tokens, cx).into_any_element()
        })
        .when(!hide_center, |el| {
            el.child(render_center_column(snapshot, tokens, cx))
        })
        .when(snapshot.toolbar.right_open, |el| {
            el.child(render_right_panel(snapshot, tokens, right_width_px, cx))
        })
}

fn render_left_sidebar_rail(
    snapshot: &GpuiWorkspaceSnapshot,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let mut rail = div()
        .id("gpui-sidebar-rail")
        .w(px(SIDEBAR_RAIL_WIDTH_PX))
        .h_full()
        .py(px(8.0))
        .flex()
        .flex_col()
        .items_center()
        .justify_between()
        .bg(tokens.chrome)
        .border_r_1()
        .border_color(tokens.border);

    let mut top = div().flex().flex_col().items_center().gap(px(6.0));
    for proj in &snapshot.projects {
        let project_id = proj.id;
        let initials: SharedString = proj.initials.clone().into();
        let active = proj.active;
        let has_waiting = proj.waiting_agents > 0;
        let has_running = proj.running_agents > 0;

        top = top.child(
            div()
                .id(("rail-proj", project_id))
                .relative()
                .w(px(36.0))
                .h(px(36.0))
                .rounded(px(tokens.radius_px))
                .flex()
                .items_center()
                .justify_center()
                .bg(if active {
                    tokens.accent_soft_bg
                } else {
                    tokens.surface
                })
                .border_1()
                .border_color(if active { tokens.accent } else { tokens.border })
                .cursor_pointer()
                .hover(|s| s.bg(tokens.surface_2))
                .on_click(cx.listener(move |root, _, _, cx| {
                    root.dispatch(GpuiAction::SelectProject(project_id), cx);
                }))
                .child(
                    div()
                        .text_size(px(11.5))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(if active {
                            tokens.text_strong
                        } else {
                            tokens.text
                        })
                        .child(initials),
                )
                .when(has_waiting || has_running, |el| {
                    el.child(
                        div()
                            .absolute()
                            .top(px(3.0))
                            .right(px(3.0))
                            .w(px(6.5))
                            .h(px(6.5))
                            .rounded_full()
                            .bg(if has_waiting {
                                tokens.warn
                            } else {
                                tokens.ok_green
                            }),
                    )
                }),
        );
    }

    rail = rail.child(top).child(
        div()
            .id("rail-add-project")
            .w(px(34.0))
            .h(px(34.0))
            .rounded(px(tokens.radius_px))
            .flex()
            .items_center()
            .justify_center()
            .bg(tokens.surface)
            .border_1()
            .border_color(tokens.border)
            .cursor_pointer()
            .hover(|s| s.bg(tokens.surface_2).border_color(tokens.accent))
            .on_click(cx.listener(|root, _, _, cx| {
                root.dispatch(GpuiAction::OpenAddProjectDialog, cx);
            }))
            .child(GpuiIcon::Plus.render(14.0, tokens.text)),
    );
    rail
}

fn render_left_sidebar_expanded(
    snapshot: &GpuiWorkspaceSnapshot,
    tokens: &GpuiThemeTokens,
    left_width_px: f32,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let header_title = if snapshot.french {
        "ESPACES DE TRAVAIL"
    } else {
        "WORKSPACES"
    };
    let add_label = if snapshot.french {
        "Ajouter un projet"
    } else {
        "Add project"
    };

    let mut list = div()
        .id("gpui-sidebar-list")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .px(px(8.0))
        .py(px(6.0))
        .flex()
        .flex_col()
        .gap(px(5.0));

    let mut last_group: Option<String> = None;
    for proj in &snapshot.projects {
        if proj.group_name != last_group {
            last_group.clone_from(&proj.group_name);
            if let Some(group) = &proj.group_name {
                list = list.child(
                    div()
                        .pt(px(6.0))
                        .pb(px(2.0))
                        .px(px(4.0))
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .text_size(px(10.5))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(tokens.text_dim)
                        .child(GpuiIcon::Folder.render(11.0, tokens.text_dim))
                        .child(SharedString::from(group.clone())),
                );
            }
        }
        list = list.child(render_project_card(proj, tokens, cx));
    }

    if snapshot.projects.is_empty() {
        let empty_hint = if snapshot.french {
            "Aucun dossier ouvert."
        } else {
            "No project folders open yet."
        };
        list = list.child(
            div()
                .p(px(12.0))
                .rounded(px(tokens.radius_px))
                .bg(tokens.surface)
                .border_1()
                .border_color(tokens.border)
                .text_size(px(11.5))
                .text_color(tokens.text_dim)
                .child(empty_hint),
        );
    }

    div()
        .id("gpui-sidebar-expanded")
        .relative()
        .w(px(left_width_px))
        .h_full()
        .flex()
        .flex_col()
        .bg(tokens.chrome)
        .border_r_1()
        .border_color(tokens.border)
        .child(
            div()
                .h(px(VIEW_TABS_HEIGHT_PX))
                .px(px(10.0))
                .flex()
                .items_center()
                .justify_between()
                .border_b_1()
                .border_color(tokens.border)
                .child(
                    div()
                        .text_size(px(10.5))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(tokens.text_dim)
                        .child(header_title),
                )
                .child(
                    div()
                        .id("sidebar-header-add")
                        .p(px(3.0))
                        .rounded(px(4.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(tokens.surface_2))
                        .on_click(cx.listener(|root, _, _, cx| {
                            root.dispatch(GpuiAction::OpenAddProjectDialog, cx);
                        }))
                        .child(GpuiIcon::Plus.render(13.0, tokens.text)),
                ),
        )
        .child(list)
        .child(
            div()
                .p(px(8.0))
                .border_t_1()
                .border_color(tokens.border)
                .child(
                    div()
                        .id("sidebar-footer-add-project")
                        .h(px(28.0))
                        .w_full()
                        .px(px(8.0))
                        .rounded(px(tokens.radius_px))
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(6.0))
                        .bg(tokens.surface)
                        .border_1()
                        .border_color(tokens.border)
                        .cursor_pointer()
                        .hover(|s| s.bg(tokens.surface_2).border_color(tokens.accent))
                        .on_click(cx.listener(|root, _, _, cx| {
                            root.dispatch(GpuiAction::PromptPickProjectFolder, cx);
                        }))
                        .child(GpuiIcon::Plus.render(12.5, tokens.accent))
                        .child(
                            div()
                                .text_size(px(11.5))
                                .text_color(tokens.text_strong)
                                .child(add_label),
                        ),
                ),
        )
        // Interactive 4px right-edge resize handle
        .child(
            div()
                .id("resize-left-sidebar")
                .absolute()
                .top_0()
                .right_0()
                .w(px(4.0))
                .h_full()
                .cursor_col_resize()
                .hover(|s| s.bg(rgba_with_alpha(tokens.accent, 0.45)))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|root, _: &MouseDownEvent, _, cx| {
                        root.active_drag = Some(DragTarget::LeftSidebar);
                        cx.notify();
                    }),
                ),
        )
}

fn render_project_card(
    proj: &GpuiProjectItem,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let project_id = proj.id;
    let active = proj.active;

    let mut card = div()
        .id(("project-card", project_id))
        .w_full()
        .px(px(8.0))
        .py(px(6.0))
        .rounded(px(tokens.radius_px))
        .flex()
        .flex_col()
        .gap(px(3.0))
        .bg(if active {
            tokens.surface_2
        } else {
            tokens.surface
        })
        .border_1()
        .border_color(if active { tokens.accent } else { tokens.border })
        .cursor_pointer()
        .hover(|s| s.bg(tokens.surface_2).border_color(tokens.border_strong))
        .on_click(cx.listener(move |root, _, _, cx| {
            root.dispatch(GpuiAction::SelectProject(project_id), cx);
        }))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(6.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .overflow_hidden()
                        .child(GpuiIcon::Folder.render(
                            13.0,
                            if active {
                                tokens.accent
                            } else {
                                tokens.text_dim
                            },
                        ))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .font_weight(if active {
                                    gpui::FontWeight::SEMIBOLD
                                } else {
                                    gpui::FontWeight::NORMAL
                                })
                                .text_color(tokens.text_strong)
                                .truncate()
                                .child(SharedString::from(proj.name.clone())),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .when(proj.waiting_agents > 0, |el| {
                            el.child(div().w(px(7.0)).h(px(7.0)).rounded_full().bg(tokens.warn))
                        })
                        .when(proj.running_agents > 0, |el| {
                            el.child(
                                div()
                                    .w(px(7.0))
                                    .h(px(7.0))
                                    .rounded_full()
                                    .bg(tokens.ok_green),
                            )
                        })
                        .when(proj.session_count > 0, |el| {
                            el.child(
                                div()
                                    .px(px(5.0))
                                    .rounded(px(4.0))
                                    .bg(tokens.surface_3)
                                    .text_size(px(10.0))
                                    .text_color(tokens.text_dim)
                                    .child(SharedString::from(proj.session_count.to_string())),
                            )
                        }),
                ),
        )
        .when_some(proj.git_branch.as_ref(), |el, branch| {
            el.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .text_size(px(10.5))
                    .text_color(tokens.text_dim)
                    .child(GpuiIcon::GitBranch.render(10.5, tokens.accent))
                    .child(div().truncate().child(SharedString::from(branch.clone()))),
            )
        });

    for (wt_idx, (wt_branch, _wt_path)) in proj.secondary_worktrees.iter().take(3).enumerate() {
        card = card.child(
            div()
                .id(("proj-wt", wt_idx))
                .pl(px(14.0))
                .pt(px(2.0))
                .flex()
                .items_center()
                .gap(px(4.0))
                .text_size(px(10.5))
                .text_color(tokens.text_dim)
                .child(GpuiIcon::GitBranch.render(10.0, tokens.text_dim))
                .child(
                    div()
                        .truncate()
                        .child(SharedString::from(wt_branch.clone())),
                ),
        );
    }

    card
}

fn render_center_column(
    snapshot: &GpuiWorkspaceSnapshot,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    div()
        .id("gpui-center-column")
        .relative()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_col()
        .overflow_hidden()
        .bg(tokens.term)
        .child(render_view_tabs_bar(snapshot, tokens, cx))
        .child(
            div()
                .id("gpui-center-stage")
                .relative()
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_hidden()
                .child(match snapshot.center_mode {
                    GpuiCenterMode::NoProjectsWelcome => {
                        render_no_projects_welcome(snapshot, tokens, cx).into_any_element()
                    }
                    GpuiCenterMode::EmptyProjectLauncher => {
                        render_empty_project_launcher(snapshot, tokens, cx).into_any_element()
                    }
                    GpuiCenterMode::WorkGrid => {
                        render_work_grid(snapshot, tokens, cx).into_any_element()
                    }
                    GpuiCenterMode::DocumentEditor => {
                        render_document_stage(snapshot, tokens, cx).into_any_element()
                    }
                    GpuiCenterMode::BrowserTab => {
                        render_browser_placeholder(snapshot, tokens, cx).into_any_element()
                    }
                    GpuiCenterMode::RemoteSftp => {
                        render_remote_sftp_stage(snapshot, tokens, cx).into_any_element()
                    }
                })
                .when_some(snapshot.overlay_title.as_ref(), |el, title| {
                    el.child(render_overlay_modal(title, snapshot, tokens, cx))
                }),
        )
}

fn render_view_tabs_bar(
    snapshot: &GpuiWorkspaceSnapshot,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let mut bar = div()
        .id("gpui-view-tabs")
        .h(px(VIEW_TABS_HEIGHT_PX))
        .w_full()
        .flex()
        .items_center()
        .overflow_x_scroll()
        .bg(tokens.chrome)
        .border_b_1()
        .border_color(tokens.border);

    for (idx, tab) in snapshot.view_tabs.iter().enumerate() {
        let selected = tab.selected;
        let doc_index = tab.doc_index;
        let kind = tab.kind;
        let closable = tab.closable;
        let icon = match kind {
            crate::view_tabs::Kind::Terminal => GpuiIcon::Terminal,
            #[cfg(windows)]
            crate::view_tabs::Kind::Browser => GpuiIcon::Globe,
            crate::view_tabs::Kind::Diff => GpuiIcon::FileDiff,
            crate::view_tabs::Kind::Editor => GpuiIcon::FileCode,
        };
        let title_str = if tab.dirty {
            format!("● {}", tab.title)
        } else {
            tab.title.clone()
        };

        bar = bar.child(
            div()
                .id(("view-tab", idx))
                .relative()
                .h_full()
                .px(px(11.0))
                .flex()
                .items_center()
                .gap(px(6.0))
                .bg(if selected { tokens.term } else { tokens.chrome })
                .border_r_1()
                .border_color(tokens.border)
                .cursor_pointer()
                .hover(|s| s.bg(tokens.surface))
                .on_click(cx.listener(move |root, _, _, cx| match kind {
                    crate::view_tabs::Kind::Terminal => {
                        root.dispatch(GpuiAction::SelectWorkTab, cx)
                    }
                    #[cfg(windows)]
                    crate::view_tabs::Kind::Browser => {
                        root.dispatch(GpuiAction::SelectBrowserTab, cx)
                    }
                    crate::view_tabs::Kind::Editor | crate::view_tabs::Kind::Diff => {
                        if let Some(doc_idx) = doc_index {
                            root.dispatch(GpuiAction::SelectDocumentTab(doc_idx), cx);
                        }
                    }
                }))
                .when(selected, |el| {
                    el.child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .right_0()
                            .h(px(2.0))
                            .bg(tokens.accent),
                    )
                })
                .child(icon.render(
                    12.5,
                    if selected {
                        tokens.accent
                    } else {
                        tokens.text_dim
                    },
                ))
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(if tab.deleted {
                            tokens.danger
                        } else if selected {
                            tokens.text_strong
                        } else {
                            tokens.text_dim
                        })
                        .child(SharedString::from(title_str)),
                )
                .when(closable, |el| {
                    el.child(
                        div()
                            .id(("close-view-tab", idx))
                            .p(px(2.0))
                            .rounded(px(3.0))
                            .hover(|s| s.bg(tokens.surface_2))
                            .on_click(cx.listener(move |root, _, _, cx| {
                                #[cfg(windows)]
                                if kind == crate::view_tabs::Kind::Browser {
                                    root.dispatch(GpuiAction::CloseBrowserTab, cx);
                                    return;
                                }
                                if let Some(doc_idx) = doc_index {
                                    root.dispatch(GpuiAction::CloseDocumentTab(doc_idx), cx);
                                }
                            }))
                            .child(GpuiIcon::Close.render(10.5, tokens.text_dim)),
                    )
                }),
        );
    }

    // Grid preset quick pills on right side of view-tabs bar when in WorkGrid
    if snapshot.center_mode == GpuiCenterMode::WorkGrid {
        bar = bar.child(
            div()
                .ml_auto()
                .pr(px(8.0))
                .flex()
                .items_center()
                .gap(px(4.0))
                .child(preset_btn(
                    "preset-1x2",
                    "1×2",
                    GridPreset::OneByTwo,
                    tokens,
                    cx,
                ))
                .child(preset_btn(
                    "preset-2x1",
                    "2×1",
                    GridPreset::TwoByOne,
                    tokens,
                    cx,
                ))
                .child(preset_btn(
                    "preset-2x2",
                    "2×2",
                    GridPreset::TwoByTwo,
                    tokens,
                    cx,
                ))
                .child(preset_btn(
                    "preset-3x2",
                    "3×2",
                    GridPreset::ThreeByTwo,
                    tokens,
                    cx,
                )),
        );
    }

    bar
}

fn preset_btn(
    id: &'static str,
    label: &'static str,
    preset: GridPreset,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    div()
        .id(id)
        .px(px(6.0))
        .py(px(2.0))
        .rounded(px(4.0))
        .bg(tokens.surface)
        .border_1()
        .border_color(tokens.border)
        .text_size(px(10.5))
        .text_color(tokens.text_dim)
        .cursor_pointer()
        .hover(|s| s.bg(tokens.surface_2).text_color(tokens.text_strong))
        .on_click(cx.listener(move |root, _, _, cx| {
            root.dispatch(GpuiAction::ApplyGridPreset(preset), cx);
        }))
        .child(label)
}

fn render_no_projects_welcome(
    snapshot: &GpuiWorkspaceSnapshot,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let heading = if snapshot.french {
        "Bienvenue dans Tessel"
    } else {
        "Welcome to Tessel"
    };
    let subtitle = if snapshot.french {
        "Ouvrez un dossier Git ou créez un projet pour lancer vos terminaux et agents IA côte à côte."
    } else {
        "Open a Git repository or folder to orchestrate terminals, native AI chats, and code reviews side by side."
    };
    let open_label = if snapshot.french {
        "Ouvrir un dossier..."
    } else {
        "Open folder..."
    };
    let dialog_label = if snapshot.french {
        "Cloner / Initialiser Git..."
    } else {
        "Clone / Init repository..."
    };

    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .p(px(24.0))
        .child(
            div()
                .w(px(480.0))
                .p(px(24.0))
                .rounded(px(tokens.radius_px + 2.0))
                .bg(tokens.surface)
                .border_1()
                .border_color(tokens.border)
                .flex()
                .flex_col()
                .gap(px(12.0))
                .child(
                    div()
                        .text_size(px(18.0))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(tokens.text_strong)
                        .child(heading),
                )
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(tokens.text_dim)
                        .child(subtitle),
                )
                .child(
                    div()
                        .pt(px(6.0))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .id("welcome-open-folder")
                                .px(px(12.0))
                                .py(px(7.0))
                                .rounded(px(tokens.radius_px))
                                .bg(tokens.accent)
                                .text_size(px(12.0))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(gpui::white())
                                .cursor_pointer()
                                .on_click(cx.listener(|root, _, _, cx| {
                                    root.dispatch(GpuiAction::PromptPickProjectFolder, cx);
                                }))
                                .child(open_label),
                        )
                        .child(
                            div()
                                .id("welcome-add-dialog")
                                .px(px(12.0))
                                .py(px(7.0))
                                .rounded(px(tokens.radius_px))
                                .bg(tokens.surface_2)
                                .border_1()
                                .border_color(tokens.border)
                                .text_size(px(12.0))
                                .text_color(tokens.text_strong)
                                .cursor_pointer()
                                .hover(|s| s.bg(tokens.surface_3))
                                .on_click(cx.listener(|root, _, _, cx| {
                                    root.dispatch(GpuiAction::OpenAddProjectDialog, cx);
                                }))
                                .child(dialog_label),
                        ),
                ),
        )
}

fn render_empty_project_launcher(
    snapshot: &GpuiWorkspaceSnapshot,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let proj_name = snapshot
        .toolbar
        .active_project_name
        .clone()
        .unwrap_or_else(|| "Workspace".into());
    let subtitle = if snapshot.french {
        "Choisissez une session à démarrer dans ce projet"
    } else {
        "Choose a terminal or AI session to launch in this workspace"
    };

    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .p(px(24.0))
        .child(
            div()
                .w(px(520.0))
                .p(px(22.0))
                .rounded(px(tokens.radius_px + 2.0))
                .bg(tokens.surface)
                .border_1()
                .border_color(tokens.border)
                .flex()
                .flex_col()
                .gap(px(12.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(GpuiIcon::Folder.render(16.0, tokens.accent))
                        .child(
                            div()
                                .text_size(px(16.0))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(tokens.text_strong)
                                .child(SharedString::from(proj_name)),
                        ),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(tokens.text_dim)
                        .child(subtitle),
                )
                .child(
                    div()
                        .pt(px(4.0))
                        .flex()
                        .flex_wrap()
                        .gap(px(8.0))
                        .child(launcher_card_btn(
                            "empty-launch-shell",
                            GpuiIcon::Terminal,
                            snapshot.toolbar.preferred_shell_label.clone(),
                            tokens.accent,
                            tokens,
                            GpuiAction::NewShell,
                            cx,
                        ))
                        .child(launcher_card_btn(
                            "empty-launch-codex",
                            GpuiIcon::Sparkles,
                            "Codex Native Chat".into(),
                            tokens.accent,
                            tokens,
                            GpuiAction::LaunchCodexChat,
                            cx,
                        ))
                        .child(launcher_card_btn(
                            "empty-launch-claude",
                            GpuiIcon::Sparkles,
                            "Claude Native Chat".into(),
                            tokens.warn,
                            tokens,
                            GpuiAction::LaunchClaudeChat,
                            cx,
                        )),
                ),
        )
}

fn launcher_card_btn(
    id: &'static str,
    icon: GpuiIcon,
    label: String,
    icon_color: gpui::Rgba,
    tokens: &GpuiThemeTokens,
    action: GpuiAction,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    div()
        .id(id)
        .px(px(12.0))
        .py(px(9.0))
        .rounded(px(tokens.radius_px))
        .bg(tokens.surface_2)
        .border_1()
        .border_color(tokens.border)
        .flex()
        .items_center()
        .gap(px(7.0))
        .cursor_pointer()
        .hover(|s| s.bg(tokens.surface_3).border_color(tokens.accent))
        .on_click(cx.listener(move |root, _, _, cx| {
            root.dispatch(action.clone(), cx);
        }))
        .child(icon.render(14.0, icon_color))
        .child(
            div()
                .text_size(px(12.0))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(tokens.text_strong)
                .child(SharedString::from(label)),
        )
}

fn render_work_grid(
    snapshot: &GpuiWorkspaceSnapshot,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let mut stage = div().id("gpui-pane-grid").relative().size_full();

    for tile in &snapshot.tiles {
        stage = stage.child(render_pane_tile(tile, snapshot.font_size, tokens, cx));
    }
    for (div_idx, divider) in snapshot.dividers.iter().enumerate() {
        stage = stage.child(render_split_divider(div_idx, divider, tokens, cx));
    }
    stage
}

fn render_pane_tile(
    tile: &GpuiPaneTileSnapshot,
    font_size: f32,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let sid = tile.session_id;
    let width_px = tile.rect_px.width;
    let border_color = if tile.broadcast {
        tokens.broadcast_pane_ring
    } else if tile.active {
        tokens.active_pane_ring
    } else {
        tokens.border
    };

    let show_model_chip = width_px >= PANE_CHIP_HIDE_MODEL_BELOW_PX;
    let show_account_chip = width_px >= PANE_CHIP_HIDE_ACCOUNT_BELOW_PX;

    let mut body = div()
        .id(("pane-body", sid))
        .flex_1()
        .min_h_0()
        .w_full()
        .p(px(8.0))
        .overflow_y_scroll()
        .font_family(mono_font_family())
        .text_size(px(font_size))
        .line_height(px((font_size * 1.38).round()))
        .bg(tokens.term)
        .text_color(tokens.text);

    if !tile.error.is_empty() {
        body = body.child(
            div()
                .mb(px(6.0))
                .px(px(8.0))
                .py(px(4.0))
                .rounded(px(4.0))
                .bg(rgba_with_alpha(tokens.danger, 0.16))
                .border_1()
                .border_color(tokens.danger)
                .text_color(tokens.text_strong)
                .child(SharedString::from(tile.error.clone())),
        );
    }

    if tile.preview_lines.is_empty() {
        let placeholder = if tile.loading {
            "Starting session..."
        } else if tile.is_native_chat {
            "Ready — send a prompt to start the conversation."
        } else {
            "Terminal session ready."
        };
        body = body.child(div().text_color(tokens.text_dim).child(placeholder));
    } else {
        for line in &tile.preview_lines {
            body = body.child(
                div()
                    .w_full()
                    .whitespace_nowrap()
                    .child(SharedString::from(line.clone())),
            );
        }
    }

    div()
        .id(("pane-tile", sid))
        .absolute()
        .left(px(tile.rect_px.left as f32))
        .top(px(tile.rect_px.top as f32))
        .w(px(tile.rect_px.width.max(40.0) as f32))
        .h(px(tile.rect_px.height.max(40.0) as f32))
        .flex()
        .flex_col()
        .overflow_hidden()
        .bg(tokens.term)
        .border_1()
        .border_color(border_color)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |root, _: &MouseDownEvent, _, cx| {
                root.dispatch(GpuiAction::FocusSession(sid), cx);
            }),
        )
        // 30px `.pane-nav` header
        .child(
            div()
                .h(px(PANE_NAV_HEIGHT_PX))
                .w_full()
                .px(px(8.0))
                .flex()
                .items_center()
                .justify_between()
                .bg(if tile.active {
                    tokens.surface_2
                } else {
                    tokens.chrome
                })
                .border_b_1()
                .border_color(tokens.border)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .overflow_hidden()
                        .child(
                            if tile.is_agent {
                                GpuiIcon::Sparkles
                            } else {
                                GpuiIcon::Terminal
                            }
                            .render(
                                12.5,
                                if tile.active {
                                    tokens.accent
                                } else {
                                    tokens.text_dim
                                },
                            ),
                        )
                        .child(
                            div()
                                .text_size(px(11.5))
                                .font_weight(if tile.active {
                                    gpui::FontWeight::SEMIBOLD
                                } else {
                                    gpui::FontWeight::NORMAL
                                })
                                .text_color(if tile.active {
                                    tokens.text_strong
                                } else {
                                    tokens.text
                                })
                                .truncate()
                                .child(SharedString::from(tile.title.clone())),
                        )
                        .when_some(
                            tile.model_chip.as_ref().filter(|_| show_model_chip),
                            |el, model| {
                                el.child(
                                    div()
                                        .px(px(5.0))
                                        .py(px(1.0))
                                        .rounded(px(4.0))
                                        .bg(tokens.surface_3)
                                        .text_size(px(10.0))
                                        .text_color(tokens.text_dim)
                                        .child(SharedString::from(model.clone())),
                                )
                            },
                        )
                        .when_some(
                            tile.account_chip.as_ref().filter(|_| show_account_chip),
                            |el, account| {
                                el.child(
                                    div()
                                        .px(px(5.0))
                                        .py(px(1.0))
                                        .rounded(px(4.0))
                                        .bg(tokens.accent_soft_bg)
                                        .text_size(px(10.0))
                                        .text_color(tokens.accent)
                                        .child(SharedString::from(account.clone())),
                                )
                            },
                        )
                        .when_some(tile.worktree_branch.as_ref(), |el, wt| {
                            el.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(3.0))
                                    .text_size(px(10.0))
                                    .text_color(tokens.text_dim)
                                    .child(GpuiIcon::GitBranch.render(10.0, tokens.accent))
                                    .child(SharedString::from(wt.clone())),
                            )
                        }),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(2.0))
                        .child(pane_header_icon_btn(
                            ("pane-broadcast", sid),
                            GpuiIcon::Radio,
                            if tile.broadcast {
                                tokens.warn
                            } else {
                                tokens.text_dim
                            },
                            tokens,
                            GpuiAction::ToggleSessionBroadcast(sid),
                            cx,
                        ))
                        .child(pane_header_icon_btn(
                            ("pane-split-v", sid),
                            GpuiIcon::Columns,
                            tokens.text_dim,
                            tokens,
                            GpuiAction::NewShellSplit(sid, SplitDir::Row),
                            cx,
                        ))
                        .child(pane_header_icon_btn(
                            ("pane-split-h", sid),
                            GpuiIcon::Rows,
                            tokens.text_dim,
                            tokens,
                            GpuiAction::NewShellSplit(sid, SplitDir::Col),
                            cx,
                        ))
                        .child(pane_header_icon_btn(
                            ("pane-max", sid),
                            if tile.maximized {
                                GpuiIcon::Minimize2
                            } else {
                                GpuiIcon::Maximize2
                            },
                            if tile.maximized {
                                tokens.accent
                            } else {
                                tokens.text_dim
                            },
                            tokens,
                            GpuiAction::ToggleMaximizeSession(sid),
                            cx,
                        ))
                        .child(pane_header_icon_btn(
                            ("pane-close", sid),
                            GpuiIcon::Close,
                            tokens.text_dim,
                            tokens,
                            GpuiAction::CloseSession(sid),
                            cx,
                        )),
                ),
        )
        .child(body)
}

fn pane_header_icon_btn(
    id: (&'static str, u64),
    icon: GpuiIcon,
    color: gpui::Rgba,
    tokens: &GpuiThemeTokens,
    action: GpuiAction,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    div()
        .id(id)
        .w(px(22.0))
        .h(px(22.0))
        .rounded(px(4.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|s| s.bg(tokens.surface_3))
        .on_click(cx.listener(move |root, _, _, cx| {
            root.dispatch(action.clone(), cx);
        }))
        .child(icon.render(11.5, color))
}

fn render_split_divider(
    div_idx: usize,
    divider: &GpuiDividerSnapshot,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let split_id = divider.split_id.clone();
    let dir = divider.dir;
    let index = divider.index;
    let split_side_px = divider.split_side_px;

    let mut el = div()
        .id(("split-divider", div_idx))
        .absolute()
        .left(px(divider.rect_px.left as f32))
        .top(px(divider.rect_px.top as f32))
        .w(px(divider.rect_px.width.max(4.0) as f32))
        .h(px(divider.rect_px.height.max(4.0) as f32))
        .bg(tokens.border)
        .hover(|s| s.bg(tokens.accent))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |root, ev: &MouseDownEvent, _, cx| {
                let start_pos = match dir {
                    SplitDir::Row => f64::from(ev.position.x),
                    SplitDir::Col => f64::from(ev.position.y),
                };
                root.active_drag = Some(DragTarget::SplitDivider {
                    split_id: split_id.clone(),
                    dir,
                    index,
                    split_side_px,
                    last_pos_px: start_pos,
                });
                cx.notify();
            }),
        );

    el = match dir {
        SplitDir::Row => el.cursor_col_resize(),
        SplitDir::Col => el.cursor_row_resize(),
    };
    el
}

fn render_document_stage(
    snapshot: &GpuiWorkspaceSnapshot,
    tokens: &GpuiThemeTokens,
    _cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let Some(doc) = &snapshot.active_document else {
        return div().size_full().into_any_element();
    };

    let mut code = div()
        .id("gpui-doc-stage-code")
        .flex_1()
        .min_h_0()
        .w_full()
        .p(px(12.0))
        .overflow_y_scroll()
        .font_family(mono_font_family())
        .text_size(px(snapshot.font_size))
        .line_height(px((snapshot.font_size * 1.42).round()))
        .bg(tokens.term);

    for (line_no, line) in doc.preview_lines.iter().enumerate() {
        code = code.child(
            div()
                .flex()
                .gap(px(12.0))
                .child(
                    div()
                        .w(px(40.0))
                        .text_right()
                        .text_color(tokens.text_dim)
                        .child(SharedString::from((line_no + 1).to_string())),
                )
                .child(
                    div()
                        .flex_1()
                        .text_color(tokens.text)
                        .whitespace_nowrap()
                        .child(SharedString::from(line.clone())),
                ),
        );
    }

    div()
        .size_full()
        .flex()
        .flex_col()
        .child(
            div()
                .h(px(28.0))
                .px(px(12.0))
                .flex()
                .items_center()
                .justify_between()
                .bg(tokens.surface)
                .border_b_1()
                .border_color(tokens.border)
                .text_size(px(11.5))
                .text_color(tokens.text_dim)
                .child(SharedString::from(doc.path_display.clone()))
                .child(SharedString::from(format!("{} lines", doc.line_count))),
        )
        .child(code)
        .into_any_element()
}

fn render_browser_placeholder(
    snapshot: &GpuiWorkspaceSnapshot,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let label = if snapshot.french {
        "Navigateur intégré — retour aux terminaux"
    } else {
        "Embedded Browser Tab — return to Terminal Grid"
    };
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .id("browser-return-work")
                .px(px(14.0))
                .py(px(8.0))
                .rounded(px(tokens.radius_px))
                .bg(tokens.surface)
                .border_1()
                .border_color(tokens.border)
                .cursor_pointer()
                .on_click(cx.listener(|root, _, _, cx| {
                    root.dispatch(GpuiAction::SelectWorkTab, cx);
                }))
                .child(label),
        )
}

fn render_remote_sftp_stage(
    _snapshot: &GpuiWorkspaceSnapshot,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .id("sftp-return-work")
                .px(px(14.0))
                .py(px(8.0))
                .rounded(px(tokens.radius_px))
                .bg(tokens.surface)
                .border_1()
                .border_color(tokens.border)
                .cursor_pointer()
                .on_click(cx.listener(|root, _, _, cx| {
                    root.dispatch(GpuiAction::SelectWorkTab, cx);
                }))
                .child("Remote SFTP Browser"),
        )
}

fn render_overlay_modal(
    title: &str,
    snapshot: &GpuiWorkspaceSnapshot,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let title_str = SharedString::from(title.to_owned());
    let is_settings = title == "Settings";

    div()
        .id("gpui-overlay-backdrop")
        .absolute()
        .inset_0()
        .bg(gpui::rgba(0x0000_008f))
        .flex()
        .items_center()
        .justify_center()
        .occlude()
        .child(
            div()
                .w(px(if is_settings { 680.0 } else { 520.0 }))
                .max_h(px(520.0))
                .p(px(18.0))
                .rounded(px(tokens.radius_px + 2.0))
                .bg(tokens.surface)
                .border_1()
                .border_color(tokens.border_strong)
                .flex()
                .flex_col()
                .gap(px(12.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .pb(px(8.0))
                        .border_b_1()
                        .border_color(tokens.border)
                        .child(
                            div()
                                .text_size(px(15.0))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(tokens.text_strong)
                                .child(title_str),
                        )
                        .child(
                            div()
                                .id("btn-close-overlay")
                                .p(px(4.0))
                                .rounded(px(4.0))
                                .cursor_pointer()
                                .hover(|s| s.bg(tokens.surface_2))
                                .on_click(cx.listener(|root, _, _, cx| {
                                    root.dispatch(GpuiAction::CloseOverlays, cx);
                                }))
                                .child(GpuiIcon::Close.render(13.0, tokens.text)),
                        ),
                )
                .when(is_settings, |el| {
                    el.child(render_quick_appearance_controls(snapshot, tokens, cx))
                })
                .when(!is_settings, |el| {
                    el.child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(8.0))
                            .child(launcher_card_btn(
                                "modal-new-shell",
                                GpuiIcon::Terminal,
                                snapshot.toolbar.preferred_shell_label.clone(),
                                tokens.accent,
                                tokens,
                                GpuiAction::NewShell,
                                cx,
                            ))
                            .child(launcher_card_btn(
                                "modal-open-folder",
                                GpuiIcon::Folder,
                                "Open Folder...".into(),
                                tokens.accent,
                                tokens,
                                GpuiAction::PromptPickProjectFolder,
                                cx,
                            ))
                            .child(launcher_card_btn(
                                "modal-open-settings",
                                GpuiIcon::Settings,
                                "Settings".into(),
                                tokens.text,
                                tokens,
                                GpuiAction::OpenSettings,
                                cx,
                            )),
                    )
                }),
        )
}

fn render_quick_appearance_controls(
    snapshot: &GpuiWorkspaceSnapshot,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let mut theme_row = div().flex().flex_wrap().gap(px(6.0));
    for (idx, theme_id) in THEMES.iter().copied().enumerate() {
        let active = snapshot.theme == theme_id;
        theme_row = theme_row.child(
            div()
                .id(("settings-theme-chip", idx))
                .px(px(9.0))
                .py(px(5.0))
                .rounded(px(tokens.radius_px))
                .bg(if active {
                    tokens.accent_soft_bg
                } else {
                    tokens.surface_2
                })
                .border_1()
                .border_color(if active { tokens.accent } else { tokens.border })
                .text_size(px(11.5))
                .text_color(tokens.text_strong)
                .cursor_pointer()
                .on_click(cx.listener(move |root, _, _, cx| {
                    root.dispatch(GpuiAction::SetTheme(theme_id), cx);
                }))
                .child(theme_id.label(snapshot.french)),
        );
    }

    let mut renderer_row = div().flex().flex_col().gap(px(6.0));
    for (idx, pref) in [RendererPreference::AutoGpui, RendererPreference::CompatEgui]
        .into_iter()
        .enumerate()
    {
        let selected = snapshot.renderer == pref;
        renderer_row = renderer_row.child(
            div()
                .id(("settings-renderer-opt", idx))
                .p(px(8.0))
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
                .cursor_pointer()
                .on_click(cx.listener(move |root, _, _, cx| {
                    root.dispatch(GpuiAction::SetRenderer(pref), cx);
                }))
                .child(
                    div()
                        .text_size(px(12.0))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(tokens.text_strong)
                        .child(pref.label(snapshot.french)),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(tokens.text_dim)
                        .child(pref.description(snapshot.french)),
                ),
        );
    }

    div()
        .flex()
        .flex_col()
        .gap(px(12.0))
        .child(
            div()
                .text_size(px(11.5))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(tokens.text_dim)
                .child(if snapshot.french { "THÈME" } else { "THEME" }),
        )
        .child(theme_row)
        .child(
            div()
                .text_size(px(11.5))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(tokens.text_dim)
                .child(if snapshot.french {
                    "MOTEUR DE RENDU UI"
                } else {
                    "UI RENDERER"
                }),
        )
        .child(renderer_row)
}

fn render_right_panel(
    snapshot: &GpuiWorkspaceSnapshot,
    tokens: &GpuiThemeTokens,
    right_width_px: f32,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let tabs: &[(SideTab, &str, &str)] = &[
        (SideTab::Dashboard, "Overview", "Aperçu"),
        (SideTab::Files, "Files", "Fichiers"),
        (SideTab::Changes, "Changes", "Modifs"),
        (SideTab::Tasks, "Tasks", "Tâches"),
        (SideTab::TaskHistory, "Runs", "Exécutions"),
        (SideTab::History, "History", "Historique"),
        (SideTab::Conversations, "Threads", "Fils"),
    ];

    let mut tab_strip = div()
        .id("gpui-right-tab-strip")
        .h(px(VIEW_TABS_HEIGHT_PX))
        .w_full()
        .px(px(6.0))
        .flex()
        .items_center()
        .justify_between()
        .border_b_1()
        .border_color(tokens.border)
        .overflow_x_scroll();

    let mut tabs_left = div().flex().items_center().gap(px(2.0));
    for (idx, (tab, en_lbl, fr_lbl)) in tabs.iter().copied().enumerate() {
        let active = snapshot.right_tab == tab;
        let label = if snapshot.french { fr_lbl } else { en_lbl };
        tabs_left = tabs_left.child(
            div()
                .id(("right-side-tab", idx))
                .px(px(7.0))
                .py(px(3.5))
                .rounded(px(4.0))
                .bg(if active {
                    tokens.accent_soft_bg
                } else {
                    gpui::rgba(0x0000_0000)
                })
                .text_size(px(11.0))
                .text_color(if active {
                    tokens.text_strong
                } else {
                    tokens.text_dim
                })
                .cursor_pointer()
                .hover(|s| s.bg(tokens.surface_2))
                .on_click(cx.listener(move |root, _, _, cx| {
                    root.dispatch(GpuiAction::SelectSideTab(tab), cx);
                }))
                .child(label),
        );
    }

    tab_strip = tab_strip.child(tabs_left).child(
        div()
            .id("btn-right-fullscreen")
            .p(px(4.0))
            .rounded(px(4.0))
            .cursor_pointer()
            .hover(|s| s.bg(tokens.surface_2))
            .on_click(cx.listener(|root, _, _, cx| {
                root.dispatch(GpuiAction::ToggleRightFullscreen, cx);
            }))
            .child(
                if snapshot.right_fullscreen {
                    GpuiIcon::Minimize2
                } else {
                    GpuiIcon::Maximize2
                }
                .render(12.0, tokens.text_dim),
            ),
    );

    let sp = &snapshot.side_panel;
    let summary_text = match sp.tab {
        SideTab::Dashboard => format!(
            "Changed files: {} ({} staged) · Tasks: {} todo / {} doing / {} done",
            sp.changed_files_count, sp.staged_count, sp.tasks_todo, sp.tasks_doing, sp.tasks_done
        ),
        SideTab::Files => sp.explorer_root_name.as_deref().map_or_else(
            || "No workspace folder selected.".into(),
            |n| format!("Explorer: {n}"),
        ),
        SideTab::Changes => format!(
            "{} modified file(s), {} staged",
            sp.changed_files_count, sp.staged_count
        ),
        SideTab::Tasks => format!(
            "Kanban — {} todo · {} doing · {} done",
            sp.tasks_todo, sp.tasks_doing, sp.tasks_done
        ),
        SideTab::TaskHistory => format!("{} recorded task run(s)", sp.history_count),
        SideTab::History => "Past CLI & chat sessions".into(),
        SideTab::Conversations => format!("{} team / project thread(s)", sp.conversations_count),
    };

    let mut panel = div()
        .id("gpui-right-panel")
        .relative()
        .h_full()
        .flex()
        .flex_col()
        .bg(tokens.chrome)
        .border_l_1()
        .border_color(tokens.border);

    panel = if snapshot.right_fullscreen {
        panel.flex_1()
    } else {
        panel.w(px(right_width_px))
    };

    panel
        .child(tab_strip)
        .child(
            div()
                .id("gpui-right-panel-body")
                .flex_1()
                .p(px(12.0))
                .overflow_y_scroll()
                .child(
                    div()
                        .p(px(10.0))
                        .rounded(px(tokens.radius_px))
                        .bg(tokens.surface)
                        .border_1()
                        .border_color(tokens.border)
                        .text_size(px(12.0))
                        .text_color(tokens.text)
                        .child(SharedString::from(summary_text)),
                ),
        )
        .when(!snapshot.right_fullscreen, |el| {
            el.child(
                div()
                    .id("resize-right-panel")
                    .absolute()
                    .top_0()
                    .left_0()
                    .w(px(4.0))
                    .h_full()
                    .cursor_col_resize()
                    .hover(|s| s.bg(rgba_with_alpha(tokens.accent, 0.45)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|root, _: &MouseDownEvent, _, cx| {
                            root.active_drag = Some(DragTarget::RightPanel);
                            cx.notify();
                        }),
                    ),
            )
        })
}
