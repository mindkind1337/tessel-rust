//! 4-zone CSS Flexbox workspace shell (`Left Sidebar | Center ViewTabs + PaneGrid | Right SidePanel`)
//! matching `C:\Tessel\src\renderer\src\App.vue` & `style.css`.

use super::{
    DragTarget, FocusInputTarget, TesselGpuiRoot, chat_view, editor_view,
    icons::GpuiIcon,
    side_panels_view, sidebar_view, terminal_view,
    theme::{
        GpuiThemeTokens, PANE_CHIP_HIDE_ACCOUNT_BELOW_PX, PANE_CHIP_HIDE_MODEL_BELOW_PX,
        PANE_NAV_HEIGHT_PX, VIEW_TABS_HEIGHT_PX, rgba_with_alpha,
    },
};
use crate::{
    app::gpui_bridge::{
        GpuiAction, GpuiCenterMode, GpuiDividerSnapshot, GpuiPaneTileSnapshot,
        GpuiWorkspaceSnapshot,
    },
    pane_grid::{GridPreset, SplitDir},
    teams_relay::WakeChannel,
    teams_ui::TeamsUiAction,
    workspace::SideTab,
};
use gpui::{
    AppContext, Context, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder as _, px,
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_workspace_body(
    snapshot: &GpuiWorkspaceSnapshot,
    remote_inputs: &super::remote_inputs::Inputs,
    editor_input: &super::editor_input::Input,
    chat_inputs: &super::chat_inputs::Inputs,
    terminal_inputs: &super::terminal_find_inputs::Inputs,
    browser_input: &super::browser_input::Input,
    scm_input: &super::scm_input::Input,
    explorer_input: &super::explorer_input::Input,
    panel_query_inputs: &super::panel_query_inputs::Inputs,
    task_rename_input: &super::task_rename_input::Input,
    task_card_input: &super::task_card_input::Inputs,
    task_agent_inputs: &super::task_agent_input::Inputs,
    task_decision_inputs: &super::task_decision_input::Inputs,
    focus_input: FocusInputTarget,
    tokens: &GpuiThemeTokens,
    left_width_px: f32,
    right_width_px: f32,
    stage_height_px: f32,
    window: &mut Window,
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
            sidebar_view::render_left_sidebar_expanded(snapshot, tokens, left_width_px, cx)
        } else {
            sidebar_view::render_left_sidebar_rail(snapshot, tokens, cx)
        })
        .when(!hide_center, |el| {
            el.child(render_center_column(
                snapshot,
                remote_inputs,
                editor_input,
                chat_inputs,
                terminal_inputs,
                browser_input,
                focus_input,
                tokens,
                stage_height_px,
                window,
                cx,
            ))
        })
        .when(snapshot.toolbar.right_open, |el| {
            el.child(render_right_panel(
                snapshot,
                scm_input,
                explorer_input,
                panel_query_inputs,
                task_rename_input,
                task_card_input,
                task_agent_inputs,
                task_decision_inputs,
                tokens,
                right_width_px,
                cx,
            ))
        })
}

#[allow(clippy::too_many_arguments)]
fn render_center_column(
    snapshot: &GpuiWorkspaceSnapshot,
    remote_inputs: &super::remote_inputs::Inputs,
    editor_input: &super::editor_input::Input,
    chat_inputs: &super::chat_inputs::Inputs,
    terminal_inputs: &super::terminal_find_inputs::Inputs,
    browser_input: &super::browser_input::Input,
    focus_input: FocusInputTarget,
    tokens: &GpuiThemeTokens,
    stage_height_px: f32,
    window: &mut Window,
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
                        render_work_grid(snapshot, chat_inputs, terminal_inputs, tokens, cx)
                            .into_any_element()
                    }
                    GpuiCenterMode::DocumentEditor => editor_view::render_document_editor_stage(
                        snapshot,
                        editor_input,
                        tokens,
                        cx,
                    )
                    .into_any_element(),
                    GpuiCenterMode::BrowserTab => editor_view::render_browser_stage(
                        snapshot,
                        browser_input,
                        tokens,
                        window,
                        cx,
                    )
                    .into_any_element(),
                    GpuiCenterMode::RemoteSftp => super::remote_view::render(
                        snapshot,
                        remote_inputs,
                        editor_input,
                        focus_input,
                        tokens,
                        cx,
                    )
                    .into_any_element(),
                })
                .when(snapshot.floating.open, |el| {
                    el.child(terminal_view::render_floating_terminal_drawer(
                        &snapshot.floating,
                        terminal_inputs,
                        stage_height_px,
                        snapshot.font_size,
                        snapshot.french,
                        tokens,
                        cx,
                    ))
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
        let close = match kind {
            #[cfg(windows)]
            crate::view_tabs::Kind::Browser => Some(GpuiAction::CloseBrowserTab),
            _ => doc_index.map(GpuiAction::CloseDocumentTab),
        }
        .filter(|_| closable);
        let mut hint = tab.hint.clone();
        if tab.preview {
            hint.push_str(if snapshot.french {
                "\nAperçu : double-cliquez pour le garder ouvert"
            } else {
                "\nPreview: double-click to keep it open"
            });
        }
        let (bg, fg, border) = (tokens.surface, tokens.text, tokens.border);
        bar = bar.child(
            div()
                .id(("view-tab", idx))
                .debug_selector(move || format!("view-tab-{idx}"))
                .group("view-tab")
                .relative()
                .h_full()
                .min_w_0()
                .max_w(px(220.))
                .pl(px(10.))
                .pr(px(if closable { 4. } else { 12. }))
                .when(!closable, |el| el.flex_shrink_0())
                .flex()
                .items_center()
                .gap(px(6.0))
                .bg(if selected {
                    tokens.surface
                } else {
                    tokens.chrome
                })
                .border_r_1()
                .border_color(tokens.border)
                .cursor_pointer()
                .when(!selected, |el| el.hover(|s| s.bg(tokens.surface_2)))
                .tooltip(move |_, cx| {
                    cx.new(|_| super::chrome::ToolbarTooltip {
                        text: hint.clone().into(),
                        bg,
                        fg,
                        border,
                    })
                    .into()
                })
                .when_some(close.clone(), |el, action| {
                    el.on_aux_click(cx.listener(move |root, event: &gpui::ClickEvent, _, cx| {
                        if !event.is_middle_click() {
                            return;
                        }
                        cx.stop_propagation();
                        root.dispatch(action.clone(), cx);
                    }))
                })
                .on_click(
                    cx.listener(move |root, event: &gpui::ClickEvent, _, cx| match kind {
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
                                if event.click_count() == 2 {
                                    root.dispatch(GpuiAction::KeepDocumentTab(doc_idx), cx);
                                }
                            }
                        }
                    }),
                )
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
                .child(
                    icon.render(
                        14.,
                        if selected {
                            tokens.accent
                        } else {
                            tokens.text_dim
                        },
                    )
                    .flex_shrink_0(),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .debug_selector(move || format!("view-tab-label-{idx}"))
                        .text_size(px(12.))
                        .text_color(if selected {
                            tokens.text_strong
                        } else {
                            tokens.text_dim
                        })
                        .when(tab.preview, |el| el.italic())
                        .when(tab.deleted, |el| el.line_through())
                        .child(SharedString::from(tab.title.clone())),
                )
                .when(tab.dirty, |el| {
                    el.child(
                        div()
                            .debug_selector(move || format!("view-tab-dirty-{idx}"))
                            .size(px(7.))
                            .flex_shrink_0()
                            .rounded_full()
                            .opacity(0.85)
                            .bg(if selected {
                                tokens.text_strong
                            } else {
                                tokens.text_dim
                            }),
                    )
                })
                .when_some(close, |el, action| {
                    el.child(
                        div()
                            .id(("close-view-tab", idx))
                            .debug_selector(move || format!("view-tab-close-{idx}"))
                            .size(px(18.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .when(!selected, |el| {
                                el.invisible().group_hover("view-tab", |s| s.visible())
                            })
                            .hover(|s| s.bg(tokens.surface_3))
                            .on_click(cx.listener(move |root, _, _, cx| {
                                cx.stop_propagation();
                                root.dispatch(action.clone(), cx);
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
    chat_inputs: &super::chat_inputs::Inputs,
    terminal_inputs: &super::terminal_find_inputs::Inputs,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let mut stage = div().id("gpui-pane-grid").relative().size_full();

    for tile in &snapshot.tiles {
        stage = stage.child(render_pane_tile(
            tile,
            chat_inputs,
            terminal_inputs,
            snapshot.font_size,
            snapshot.french,
            tokens,
            cx,
        ));
    }
    for (div_idx, divider) in snapshot.dividers.iter().enumerate() {
        stage = stage.child(render_split_divider(div_idx, divider, tokens, cx));
    }
    stage
}

fn render_pane_tile(
    tile: &GpuiPaneTileSnapshot,
    chat_inputs: &super::chat_inputs::Inputs,
    terminal_inputs: &super::terminal_find_inputs::Inputs,
    font_size: f32,
    french: bool,
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

    let body = if let Some(chat) = &tile.native_chat {
        chat_view::render_native_chat_pane_body(
            sid,
            chat,
            chat_inputs,
            font_size,
            french,
            tokens,
            cx,
        )
        .into_any_element()
    } else {
        terminal_view::render_terminal_pane_body(
            tile,
            terminal_inputs,
            font_size,
            french,
            tokens,
            cx,
        )
        .into_any_element()
    };

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
                        })
                        .when_some(tile.team_badge.as_ref(), |el, badge| {
                            let mut row = el;
                            if !badge.team_number.is_empty() {
                                let label = if badge.is_lead {
                                    if french {
                                        format!("#{} · chef", badge.team_number)
                                    } else {
                                        format!("#{} · lead", badge.team_number)
                                    }
                                } else {
                                    format!("#{}", badge.team_number)
                                };
                                row = row.child(
                                    div()
                                        .px(px(5.0))
                                        .py(px(1.0))
                                        .rounded(px(4.0))
                                        .bg(if badge.is_lead {
                                            tokens.accent_soft_bg
                                        } else {
                                            tokens.surface_3
                                        })
                                        .text_size(px(10.0))
                                        .text_color(if badge.is_lead {
                                            tokens.accent
                                        } else {
                                            tokens.text_dim
                                        })
                                        .child(SharedString::from(label)),
                                );
                            }
                            if let Some(coord) = &badge.worker_of {
                                let tag = if french {
                                    format!("exécutant de {coord}")
                                } else {
                                    format!("worker of {coord}")
                                };
                                row = row.child(
                                    div()
                                        .px(px(5.0))
                                        .py(px(1.0))
                                        .rounded(px(4.0))
                                        .bg(tokens.surface_3)
                                        .text_size(px(10.0))
                                        .text_color(tokens.text_dim)
                                        .child(SharedString::from(tag)),
                                );
                            }
                            if badge.unread_count > 0 {
                                row = row.child(
                                    div()
                                        .px(px(5.0))
                                        .py(px(1.0))
                                        .rounded(px(4.0))
                                        .bg(tokens.accent_soft_bg)
                                        .text_size(px(10.0))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(tokens.accent)
                                        .child(SharedString::from(format!(
                                            "✉ {}",
                                            badge.unread_count
                                        ))),
                                );
                            }
                            row
                        })
                        .children(tile.share_guests.iter().take(3).map(|guest| {
                            div()
                                .px(px(5.0))
                                .py(px(1.0))
                                .rounded(px(99.0))
                                .bg(rgba_with_alpha(tokens.ok_green, 0.18))
                                .text_size(px(9.5))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(tokens.ok_green)
                                .child(SharedString::from(guest.clone()))
                        })),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(2.0))
                        .when(tile.can_switch_chat_mode, |el| {
                            let mode_lbl = if tile.is_native_chat { "CLI" } else { "Chat" };
                            el.child(
                                div()
                                    .id(("pane-switch-mode", sid))
                                    .px(px(5.0))
                                    .py(px(1.5))
                                    .mr(px(2.0))
                                    .rounded(px(4.0))
                                    .bg(tokens.surface_3)
                                    .text_size(px(9.5))
                                    .text_color(tokens.text)
                                    .cursor_pointer()
                                    .hover(|s| s.text_color(tokens.accent))
                                    .on_click(cx.listener(move |root, _, _, cx| {
                                        root.dispatch(GpuiAction::SwitchSessionChatMode(sid), cx);
                                    }))
                                    .child(mode_lbl),
                            )
                        })
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
        .when_some(tile.wake_banner.as_ref(), |el, banner| {
            let wake_msg = match (banner.channel, french) {
                (WakeChannel::Team, false) => {
                    "Unread team message waiting (idle state not confirmed)"
                }
                (WakeChannel::Team, true) => {
                    "Message d'équipe en attente (inactivité non confirmée)"
                }
                (WakeChannel::Relay, false) => {
                    "Unread inter-project message waiting (idle state not confirmed)"
                }
                (WakeChannel::Relay, true) => {
                    "Message inter-projets en attente (inactivité non confirmée)"
                }
            };
            let send_lbl = if french {
                "Lui envoyer le rappel"
            } else {
                "Send reminder"
            };
            let dismiss_lbl = if french { "Ignorer" } else { "Dismiss" };
            let target_pane = banner.pane_id;
            el.child(
                div()
                    .w_full()
                    .px(px(8.0))
                    .py(px(4.0))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(6.0))
                    .bg(tokens.surface_2)
                    .border_b_1()
                    .border_color(tokens.border)
                    .text_size(px(11.0))
                    .child(div().text_color(tokens.text).truncate().child(wake_msg))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .id(("pane-wake-send", sid))
                                    .px(px(6.0))
                                    .py(px(1.5))
                                    .rounded(px(4.0))
                                    .bg(tokens.accent_soft_bg)
                                    .text_color(tokens.accent)
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |root, _, _, cx| {
                                        root.dispatch(
                                            GpuiAction::Teams(TeamsUiAction::SendWakeReminder(
                                                target_pane,
                                            )),
                                            cx,
                                        );
                                    }))
                                    .child(send_lbl),
                            )
                            .child(
                                div()
                                    .id(("pane-wake-dismiss", sid))
                                    .px(px(5.0))
                                    .py(px(1.5))
                                    .rounded(px(4.0))
                                    .text_color(tokens.text_dim)
                                    .cursor_pointer()
                                    .hover(|s| s.bg(tokens.surface_3))
                                    .on_click(cx.listener(move |root, _, _, cx| {
                                        root.dispatch(
                                            GpuiAction::Teams(TeamsUiAction::DismissWakeBanner(
                                                target_pane,
                                            )),
                                            cx,
                                        );
                                    }))
                                    .child(dismiss_lbl),
                            ),
                    ),
            )
        })
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

#[allow(clippy::too_many_arguments)]
fn render_right_panel(
    snapshot: &GpuiWorkspaceSnapshot,
    scm_input: &super::scm_input::Input,
    explorer_input: &super::explorer_input::Input,
    panel_query_inputs: &super::panel_query_inputs::Inputs,
    task_rename_input: &super::task_rename_input::Input,
    task_card_input: &super::task_card_input::Inputs,
    task_agent_inputs: &super::task_agent_input::Inputs,
    task_decision_inputs: &super::task_decision_input::Inputs,
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
                .min_h_0()
                .overflow_hidden()
                .flex()
                .flex_col()
                .child(side_panels_view::render_side_panel_tab_body(
                    snapshot,
                    scm_input,
                    explorer_input,
                    panel_query_inputs,
                    task_rename_input,
                    task_card_input,
                    task_agent_inputs,
                    task_decision_inputs,
                    tokens,
                    cx,
                )),
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
