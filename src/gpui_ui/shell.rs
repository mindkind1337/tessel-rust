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
    pane_grid::SplitDir,
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
    tabs_focus: &gpui::FocusHandle,
    remote_inputs: &super::remote_inputs::Inputs,
    editor_input: &super::editor_input::Input,
    chat_inputs: &super::chat_inputs::Inputs,
    terminal_inputs: &super::terminal_find_inputs::Inputs,
    browser_input: &super::browser_input::Input,
    side_browser_input: &super::browser_input::Input,
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
                tabs_focus,
                remote_inputs,
                editor_input,
                chat_inputs,
                terminal_inputs,
                browser_input,
                focus_input,
                tokens,
                (f32::from(window.viewport_size().width)
                    - if snapshot.toolbar.left_open {
                        left_width_px
                    } else {
                        super::theme::SIDEBAR_RAIL_WIDTH_PX
                    }
                    - if snapshot.toolbar.right_open {
                        right_width_px
                    } else {
                        0.
                    })
                .max(1.),
                stage_height_px,
                window,
                cx,
            ))
        })
        .when(snapshot.toolbar.right_open, |el| {
            el.child(render_right_panel(
                snapshot,
                side_browser_input,
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
    tabs_focus: &gpui::FocusHandle,
    remote_inputs: &super::remote_inputs::Inputs,
    editor_input: &super::editor_input::Input,
    chat_inputs: &super::chat_inputs::Inputs,
    terminal_inputs: &super::terminal_find_inputs::Inputs,
    browser_input: &super::browser_input::Input,
    focus_input: FocusInputTarget,
    tokens: &GpuiThemeTokens,
    stage_width_px: f32,
    stage_height_px: f32,
    window: &mut Window,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let stage_bounds = std::rc::Rc::new(std::cell::Cell::new(gpui::Bounds::default()));
    let painted = stage_bounds.clone();
    let pane_targets: Vec<_> = snapshot
        .tiles
        .iter()
        .filter(|_| snapshot.center_mode == GpuiCenterMode::WorkGrid)
        .map(|tile| (tile.session_id, tile.rect_px))
        .collect();
    #[cfg(windows)]
    let anchor = snapshot
        .view_tabs
        .iter()
        .find(|tab| tab.selected)
        .and_then(|tab| tab.browser_id);
    #[cfg(not(windows))]
    let anchor: Option<u64> = None;
    div()
        .id("gpui-center-column")
        .debug_selector(|| "browser-center-drop".into())
        .on_drop(
            cx.listener(move |root, page: &SideBrowserDrag, window, cx| {
                use crate::pane_grid::DropZone;
                let bounds = stage_bounds.get();
                let pos = window.mouse_position() - bounds.origin;
                let (x, y, w, h) = (
                    f32::from(pos.x),
                    f32::from(pos.y),
                    f32::from(bounds.size.width),
                    f32::from(bounds.size.height),
                );
                let edge = DropZone::workspace_edge(x, y, w, h);
                let zone = edge.or_else(|| anchor.and_then(|_| DropZone::at(x / w, y / h, false)));
                let action = if let Some(zone) = zone {
                    GpuiAction::UndockBrowserAt {
                        page: page.0,
                        anchor: if edge.is_some() { None } else { anchor },
                        before: matches!(zone, DropZone::Left | DropZone::Top),
                    }
                } else if let Some((pane, _)) = pane_targets.iter().find(|(_, r)| {
                    (r.left..r.right()).contains(&f64::from(x))
                        && (r.top..r.bottom()).contains(&f64::from(y))
                }) {
                    GpuiAction::UndockBrowserOnPane {
                        page: page.0,
                        pane: *pane,
                    }
                } else {
                    GpuiAction::UndockBrowser(page.0)
                };
                root.dispatch(action, cx)
            }),
        )
        .relative()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_col()
        .overflow_hidden()
        .bg(tokens.term)
        .child(render_view_tabs_bar(snapshot, tabs_focus, tokens, cx))
        .child(
            div()
                .id("gpui-center-stage")
                .on_prepaint(move |layout, _, _| painted.set(layout.bounds))
                .capture_key_down(cx.listener(|root, event, window, cx| {
                    if matches!(
                        root.chat_inputs.focused(window, cx),
                        Some(FocusInputTarget::ChatComposer(_))
                    ) && root.workspace_pane_navigation(event, window, cx)
                    {
                        return;
                    }
                    root.workspace_document_shortcut(event, window, cx);
                }))
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
                    GpuiCenterMode::GuestFiles => {
                        let body = div()
                            .id("guest-workspace")
                            .debug_selector(|| "guest-workspace".into())
                            .size_full()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .overflow_y_scroll();
                        if let Some(files) = &snapshot.modals.share.guest_files {
                            super::modals_view::render_guest_document(
                                body,
                                files,
                                snapshot.french,
                                tokens,
                                Some((stage_width_px, (stage_height_px - 80.).max(1.))),
                                editor_input.guest.state(),
                                cx,
                            )
                            .into_any_element()
                        } else {
                            body.into_any_element()
                        }
                    }
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
    tabs_focus: &gpui::FocusHandle,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let mut bar = div()
        .id("gpui-view-tabs")
        .track_focus(tabs_focus)
        .tab_stop(false)
        .role(gpui::Role::TabList)
        .aria_label(if snapshot.french {
            "Terminal, éditeurs et navigateurs"
        } else {
            "Terminal, editors and browsers"
        })
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
        let guest_key = tab.guest_key.clone();
        let kind = tab.kind;
        let closable = tab.closable;
        let tint = if selected {
            tokens.accent
        } else {
            tokens.text_dim
        };
        let icon = match kind {
            crate::view_tabs::Kind::Terminal => GpuiIcon::Terminal
                .render(14., tint)
                .flex_shrink_0()
                .into_any_element(),
            #[cfg(windows)]
            crate::view_tabs::Kind::Browser => GpuiIcon::Globe
                .render(14., tint)
                .flex_shrink_0()
                .into_any_element(),
            crate::view_tabs::Kind::Editor | crate::view_tabs::Kind::Diff => {
                let path = tab
                    .guest_key
                    .as_ref()
                    .map_or(tab.hint.as_str(), |key| &key.path);
                super::file_icons::render(path, tokens, tint).into_any_element()
            }
        };
        let select = match kind {
            crate::view_tabs::Kind::Terminal => Some(GpuiAction::SelectWorkTab),
            #[cfg(windows)]
            crate::view_tabs::Kind::Browser => tab.browser_id.map(GpuiAction::SelectBrowserPage),
            crate::view_tabs::Kind::Editor | crate::view_tabs::Kind::Diff => guest_key
                .clone()
                .map(GpuiAction::SelectGuestDocument)
                .or_else(|| doc_index.map(GpuiAction::SelectDocumentTab)),
        };
        let close = match kind {
            #[cfg(windows)]
            crate::view_tabs::Kind::Browser => tab.browser_id.map(GpuiAction::CloseBrowserPage),
            _ => guest_key
                .clone()
                .map(|key| {
                    GpuiAction::Share(crate::share_panel::ShareGpuiAction::GuestCloseTab(key))
                })
                .or_else(|| doc_index.map(GpuiAction::CloseDocumentTab)),
        }
        .filter(|_| closable);
        #[cfg(windows)]
        let speaker = tab.browser_id.zip(tab.sound);
        #[cfg(not(windows))]
        let speaker: Option<(u64, bool)> = None;
        #[cfg(windows)]
        let browser_drag = tab.browser_id.map(CenterBrowserDrag);
        #[cfg(not(windows))]
        let browser_drag: Option<CenterBrowserDrag> = None;
        let drag_title = tab.title.clone();
        let drag_tokens = *tokens;
        let lang = crate::view_tabs_browser_model::TabsLang::from(snapshot.french);
        let hint = if doc_index.is_some() {
            crate::view_tabs_browser_model::editor_hint(&tab.title, &tab.hint, tab.preview, lang)
        } else {
            tab.hint.clone()
        };
        let (bg, fg, border) = (tokens.surface, tokens.text, tokens.border);
        bar = bar.child(
            div()
                .id(("view-tab", idx))
                .when_some(browser_drag, |el, drag| {
                    el.on_drag(drag, move |_, _, _, cx| {
                        cx.new(|_| super::task_card_input::Ghost {
                            title: drag_title.clone(),
                            tokens: drag_tokens,
                        })
                    })
                })
                .debug_selector(move || format!("view-tab-{idx}"))
                .role(gpui::Role::Tab)
                .key_context("TesselViewTab")
                .aria_selected(selected)
                .aria_label(tab.title.clone())
                .focusable()
                .tab_stop(true)
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
                .focus_visible(|style| style.border_1().border_color(tokens.accent))
                .when_some(select, |el, select| {
                    let key_action = select.clone();
                    el.on_key_down(cx.listener(
                        move |root, event: &gpui::KeyDownEvent, window, cx| {
                            let key = &event.keystroke;
                            let mods = key.modifiers;
                            if root.app.gpui_has_open_modal()
                                || mods.control
                                || mods.alt
                                || mods.platform
                                || mods.function
                            {
                                return;
                            }
                            match key.key.as_str() {
                                "tab" if mods.shift => window.focus_prev(cx),
                                "tab" => window.focus_next(cx),
                                "enter" | "space" if !mods.shift => {
                                    if !event.is_held {
                                        root.dispatch(key_action.clone(), cx);
                                    }
                                }
                                _ => return,
                            }
                            window.prevent_default();
                            cx.stop_propagation();
                        },
                    ))
                    .on_click(cx.listener(
                        move |root, event: &gpui::ClickEvent, window, cx| {
                            root.dispatch(select.clone(), cx);
                            root.focus_handle.focus(window, cx);
                            if event.click_count() == 2
                                && let Some(index) = doc_index
                            {
                                root.dispatch(GpuiAction::KeepDocumentTab(index), cx);
                            }
                        },
                    ))
                })
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
                .child(icon)
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
                .when_some(speaker, |el, (id, muted)| {
                    let label = crate::i18n::tr(if muted { "Unmute tab" } else { "Mute tab" });
                    el.child(
                        div()
                            .id(("tab-sound", idx))
                            .debug_selector(move || format!("view-tab-sound-{idx}"))
                            .role(gpui::Role::Button)
                            .aria_label(label)
                            .cursor_pointer()
                            .flex_shrink_0()
                            .tooltip(move |_, cx| {
                                cx.new(|_| super::chrome::ToolbarTooltip {
                                    text: label.into(),
                                    bg,
                                    fg,
                                    border,
                                })
                                .into()
                            })
                            .on_click(cx.listener(move |root, _, _, cx| {
                                cx.stop_propagation();
                                root.dispatch(GpuiAction::ToggleBrowserSound(id), cx);
                            }))
                            .child(
                                (if muted {
                                    GpuiIcon::VolumeX
                                } else {
                                    GpuiIcon::Volume2
                                })
                                .render(13., if muted { tokens.text_dim } else { tokens.text }),
                            ),
                    )
                })
                .when_some(close, |el, action| {
                    let (hint, name) =
                        crate::view_tabs_browser_model::close_labels(&tab.title, tab.dirty, lang);
                    el.child(
                        div()
                            .id(("close-view-tab", idx))
                            .role(gpui::Role::Button)
                            .aria_label(name)
                            .tooltip(move |_, cx| {
                                cx.new(|_| super::chrome::ToolbarTooltip {
                                    text: hint.clone().into(),
                                    bg,
                                    fg,
                                    border,
                                })
                                .into()
                            })
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

    bar
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
    let grid_bounds = std::rc::Rc::new(std::cell::Cell::new(gpui::Bounds::default()));
    let painted = grid_bounds.clone();
    let tiles = snapshot
        .tiles
        .iter()
        .map(|tile| {
            let r = tile.rect_px;
            (
                tile.session_id,
                SharedString::from(tile.title.clone()),
                gpui::Bounds::new(
                    gpui::point(px(r.left as f32), px(r.top as f32)),
                    gpui::size(px(r.width.max(40.) as f32), px(r.height.max(40.) as f32)),
                ),
            )
        })
        .collect::<Vec<_>>();
    let target_at = std::rc::Rc::new(
        move |grid: gpui::Bounds<gpui::Pixels>, pointer: gpui::Point<gpui::Pixels>, from| {
            if tiles.len() < 2 || !grid.contains(&pointer) {
                return None;
            }
            let pos = pointer - grid.origin;
            use crate::pane_grid::DropZone;
            if let Some(zone) = DropZone::workspace_edge(
                pos.x.into(),
                pos.y.into(),
                grid.size.width.into(),
                grid.size.height.into(),
            ) {
                return Some((None, zone, grid, None));
            }
            let (id, title, local) = tiles
                .iter()
                .find(|(id, _, bounds)| *id != from && bounds.contains(&pos))?;
            let pt = pos - local.origin;
            let zone = DropZone::at(
                f32::from(pt.x) / f32::from(local.size.width),
                f32::from(pt.y) / f32::from(local.size.height),
                true,
            )?;
            Some((
                Some(*id),
                zone,
                gpui::Bounds::new(grid.origin + local.origin, local.size),
                Some(title.clone()),
            ))
        },
    );
    let drop_at = target_at.clone();
    let mut stage = div()
        .id("gpui-pane-grid")
        .debug_selector(|| "pane-grid-drop-bounds".into())
        .relative()
        .size_full()
        .on_prepaint(move |layout, _, _| painted.set(layout.bounds))
        .on_drop(cx.listener(move |root, drag: &DraggedPane, window, cx| {
            if let Some((to, zone, _, _)) =
                drop_at(grid_bounds.get(), window.mouse_position(), drag.id)
            {
                root.dispatch(
                    GpuiAction::MovePane {
                        from: drag.id,
                        to,
                        zone,
                    },
                    cx,
                );
            }
        }));
    for tile in &snapshot.tiles {
        stage = stage.child(render_pane_tile(
            tile,
            chat_inputs,
            terminal_inputs,
            snapshot,
            tokens,
            cx,
        ));
    }
    for (div_idx, divider) in snapshot.dividers.iter().enumerate() {
        stage = stage.child(render_split_divider(div_idx, divider, tokens, cx));
    }
    let accent = tokens.accent;
    let french = snapshot.french;
    let drop_preview = gpui::canvas(
        |_, _, _| (),
        move |grid, (), window, cx| {
            use crate::pane_grid::DropZone;
            let Some(drag) = cx
                .active_drag()
                .and_then(|drag| drag.value.downcast_ref::<DraggedPane>())
            else {
                return;
            };
            let Some((to, zone, mut bounds, title)) =
                target_at(grid, window.mouse_position(), drag.id)
            else {
                return;
            };
            let fraction = if to.is_none() { 1. / 3. } else { 0.5 };
            match zone {
                DropZone::Center => bounds = bounds.inset(px(8.)),
                DropZone::Left | DropZone::Right => {
                    let width = bounds.size.width;
                    bounds.size.width *= fraction;
                    if zone == DropZone::Right {
                        bounds.origin.x += width - bounds.size.width;
                    }
                }
                DropZone::Top | DropZone::Bottom => {
                    let height = bounds.size.height;
                    bounds.size.height *= fraction;
                    if zone == DropZone::Bottom {
                        bounds.origin.y += height - bounds.size.height;
                    }
                }
            }
            let label = pane_drop_label(zone, title.as_deref(), french);
            paint_drop_zone(bounds, label.into(), accent, window, cx);
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full();
    stage.child(drop_preview)
}

fn pane_drop_label(zone: crate::pane_grid::DropZone, title: Option<&str>, french: bool) -> String {
    use crate::pane_grid::DropZone::*;
    if let Some(title) = title {
        let prefix = match (zone, french) {
            (Center, false) => "Swap with",
            (Center, true) => "Échanger avec",
            (Left, false) => "Place left of",
            (Left, true) => "Placer à gauche de",
            (Right, false) => "Place right of",
            (Right, true) => "Placer à droite de",
            (Top, false) => "Place above",
            (Top, true) => "Placer au-dessus de",
            (Bottom, false) => "Place below",
            (Bottom, true) => "Placer sous",
        };
        return format!("{prefix} {title}");
    }
    match (zone, french) {
        (Top, false) => "Place across the top",
        (Top, true) => "Placer sur tout le haut",
        (Bottom, false) => "Place across the bottom",
        (Bottom, true) => "Placer sur tout le bas",
        (Left, false) => "Place down the left side",
        (Left, true) => "Placer sur tout le côté gauche",
        (Right, false) => "Place down the right side",
        (Right, true) => "Placer sur tout le côté droit",
        (Center, _) => "",
    }
    .into()
}

fn paint_drop_zone(
    bounds: gpui::Bounds<gpui::Pixels>,
    label: SharedString,
    accent: gpui::Rgba,
    window: &mut Window,
    cx: &mut gpui::App,
) {
    let mut font = gpui::font(super::theme::ui_font_family());
    font.weight = gpui::FontWeight::SEMIBOLD;
    let run = gpui::TextRun {
        len: label.len(),
        font,
        color: gpui::rgb_to_hsla(gpui::rgb(0x0b1530)),
        ..Default::default()
    };
    let line = window.text_system().shape_line(label, px(12.), &[run]);
    window.paint_quad(
        gpui::PaintQuad::new(bounds)
            .background(rgba_with_alpha(accent, 0.16))
            .border_widths(px(2.))
            .border_color(accent)
            .corner_radii(px(8.)),
    );
    let badge = gpui::Bounds::new(
        bounds.center() - gpui::point(line.width / 2. + px(12.), px(14.)),
        gpui::size(line.width + px(24.), px(28.)),
    );
    window.with_content_mask(
        Some(gpui::ContentMask {
            bounds,
            ..Default::default()
        }),
        |window| {
            window.paint_quad(
                gpui::PaintQuad::new(badge)
                    .background(accent)
                    .corner_radii(px(7.)),
            );
            let _ = line.paint(
                badge.origin + gpui::point(px(12.), px(6.)),
                px(16.),
                gpui::TextAlign::Left,
                None,
                window,
                cx,
            );
        },
    );
}

#[derive(Clone)]
pub(super) struct SideBrowserDrag(pub(super) u64);
#[derive(Clone)]
struct CenterBrowserDrag(u64);
#[derive(Clone)]
struct DraggedPane {
    id: u64,
    title: String,
}

fn render_pane_tile(
    tile: &GpuiPaneTileSnapshot,
    chat_inputs: &super::chat_inputs::Inputs,
    terminal_inputs: &super::terminal_find_inputs::Inputs,
    snapshot: &GpuiWorkspaceSnapshot,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let french = snapshot.french;
    let font_size = snapshot.font_size;
    let sid = tile.session_id;
    let width_px = tile.rect_px.width;
    let ghost_tokens = *tokens;
    let drag = DraggedPane {
        id: sid,
        title: tile.title.clone(),
    };
    let border_color = if snapshot.toolbar.broadcast_enabled
        && tile.active
        && tile.broadcast
        && !tile.is_native_chat
    {
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
        .debug_selector(move || format!("pane-drop-{sid}"))
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
                .id(("pane-drag-header", sid))
                .debug_selector(move || format!("pane-drag-header-{sid}"))
                .on_drag(drag, move |drag, _, _, cx| {
                    cx.new(|_| super::task_card_input::Ghost {
                        title: drag.title.clone(),
                        tokens: ghost_tokens,
                    })
                })
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
                        .when(
                            snapshot.toolbar.broadcast_enabled && !tile.is_native_chat,
                            |el| {
                                el.child(
                                    div()
                                        .id(("pane-broadcast", sid))
                                        .debug_selector(move || format!("pane-write-{sid}"))
                                        .role(gpui::Role::CheckBox)
                                        .aria_toggled(tile.broadcast.into())
                                        .focusable()
                                        .tab_stop(true)
                                        .aria_label(
                                            crate::layout_broadcast_ui::pane_broadcast_tooltip(
                                                french,
                                            ),
                                        )
                                        .flex()
                                        .items_center()
                                        .gap(px(4.))
                                        .mr(px(4.))
                                        .text_size(px(11.))
                                        .text_color(if tile.broadcast {
                                            tokens.broadcast_banner_fg
                                        } else {
                                            tokens.text_dim
                                        })
                                        .cursor_pointer()
                                        .tooltip({
                                            let (bg,fg,border) = (tokens.surface_2,tokens.text_strong,tokens.border);
                                            move |_, cx| cx.new(|_| super::chrome::ToolbarTooltip {text: crate::layout_broadcast_ui::pane_broadcast_tooltip(french).into(),bg,fg,border}).into()
                                        })
                                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                            cx.stop_propagation()
                                        })
                                        .on_click(cx.listener(move |root, _, window, cx| {
                                            cx.stop_propagation();
                                            root.dispatch(
                                                GpuiAction::ToggleSessionBroadcast(sid),
                                                cx,
                                            );
                                            root.focus_handle.focus(window, cx);
                                        }))
                                        .child(
                                            div()
                                                .size(px(12.))
                                                .rounded(px(2.))
                                                .border_1()
                                                .border_color(if tile.broadcast {
                                                    tokens.warn
                                                } else {
                                                    tokens.border_strong
                                                })
                                                .when(tile.broadcast, |el| {
                                                    el.bg(tokens.warn).child(
                                                        GpuiIcon::Check.render(10., tokens.chrome),
                                                    )
                                                }),
                                        )
                                        .child(crate::layout_broadcast_ui::pane_broadcast_label(
                                            french,
                                        )),
                                )
                            },
                        )
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

fn render_side_browser(
    id: u64,
    address: &super::browser_input::Input,
    snapshot: &GpuiWorkspaceSnapshot,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> gpui::AnyElement {
    let french = snapshot.french;
    let history = snapshot
        .side_browser
        .as_ref()
        .map(|page| page.history)
        .unwrap_or_default();
    let page = div()
        .size_full()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .children(
                    [
                        (
                            "side-browser-back",
                            "←",
                            history.0,
                            GpuiAction::SideBrowserBack(id),
                        ),
                        (
                            "side-browser-forward",
                            "→",
                            history.1,
                            GpuiAction::SideBrowserForward(id),
                        ),
                        (
                            "side-browser-reload",
                            "⟳",
                            true,
                            GpuiAction::SideBrowserReload(id),
                        ),
                    ]
                    .into_iter()
                    .map(|(key, label, enabled, action)| {
                        div()
                            .id(key)
                            .aria_disabled(!enabled)
                            .debug_selector(move || key.into())
                            .px(px(5.))
                            .flex_shrink_0()
                            .when(!enabled, |el| el.opacity(0.4))
                            .when(enabled, |el| {
                                el.cursor_pointer()
                                    .on_click(cx.listener(move |root, _, _, cx| {
                                        root.dispatch(action.clone(), cx);
                                    }))
                            })
                            .child(label)
                    }),
                )
                .child(address.render(french, tokens, cx))
                .when_some(
                    snapshot
                        .side_browser
                        .as_ref()
                        .filter(|page| !page.downloads.is_empty()),
                    |el, page| el.child(editor_view::download_toggle(page, tokens, cx)),
                )
                .child(
                    div()
                        .id("side-browser-return")
                        .debug_selector(|| "side-browser-return".into())
                        .cursor_pointer()
                        .p(px(6.))
                        .text_color(tokens.text)
                        .text_size(px(11.))
                        .on_click(cx.listener(move |root, _, _, cx| {
                            root.dispatch(GpuiAction::UndockBrowser(id), cx)
                        }))
                        .child(if french {
                            "Ramener au centre"
                        } else {
                            "Move to center"
                        }),
                ),
        )
        .when_some(
            snapshot
                .side_browser
                .as_ref()
                .filter(|page| page.downloads_open && !page.downloads.is_empty()),
            |el, page| el.child(editor_view::download_list(page, tokens, cx)),
        )
        .when_some(
            snapshot
                .side_browser
                .as_ref()
                .filter(|page| !page.error.is_empty()),
            |el, page| el.child(div().text_color(tokens.warn).child(page.error.clone())),
        );
    #[cfg(windows)]
    let page = {
        let entity = cx.weak_entity();
        page.child(
            gpui::canvas(
                |_, _, _| (),
                move |bounds, (), window, cx| {
                    let dragging = cx.has_active_drag();
                    let _ = entity.update(cx, |root, cx| {
                        let show = !dragging && root.app.side_browser_visible(id);
                        if let Some(page) = root.app.side_browsers.get_mut(&id)
                            && page.sync_gpui_bounds(
                                bounds.origin.x.into(),
                                bounds.origin.y.into(),
                                bounds.size.width.into(),
                                bounds.size.height.into(),
                                window.scale_factor(),
                                show,
                                None,
                                !show,
                            )
                        {
                            root.focus_input = FocusInputTarget::None;
                            window.blur(cx);
                        }
                    });
                },
            )
            .flex_1()
            .min_h_0()
            .w_full(),
        )
    };
    page.into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn render_right_panel(
    snapshot: &GpuiWorkspaceSnapshot,
    side_browser_input: &super::browser_input::Input,
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
        .border_color(tokens.border);

    let mut tabs_left = div()
        .id("side-scrollable-tabs")
        .flex_1()
        .min_w_0()
        .overflow_x_scroll()
        .flex()
        .items_center()
        .gap(px(2.0));
    for (idx, (tab, en_lbl, fr_lbl)) in tabs.iter().copied().enumerate() {
        let active = snapshot.selected_side_browser.is_none() && snapshot.right_tab == tab;
        let label = if snapshot.french { fr_lbl } else { en_lbl };
        tabs_left = tabs_left.child(
            div()
                .id(("right-side-tab", idx))
                .flex_shrink_0()
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

    for (id, title) in &snapshot.side_browser_tabs {
        let id = *id;
        let drag_title = title.clone();
        let drag_tokens = *tokens;
        tabs_left = tabs_left.child(
            div()
                .id(("side-browser-tab", id))
                .flex_shrink_0()
                .debug_selector(move || format!("side-browser-tab-{id}"))
                .flex()
                .items_center()
                .gap(px(3.))
                .px(px(7.))
                .py(px(3.5))
                .rounded(px(4.))
                .cursor_pointer()
                .when(snapshot.selected_side_browser == Some(id), |el| {
                    el.bg(tokens.accent_soft_bg)
                })
                .on_click(cx.listener(move |root, _, _, cx| {
                    root.dispatch(GpuiAction::SelectSideBrowser(id), cx)
                }))
                .on_aux_click(cx.listener(move |root, event: &gpui::ClickEvent, _, cx| {
                    if event.is_middle_click() {
                        cx.stop_propagation();
                        root.dispatch(GpuiAction::CloseSideBrowser(id), cx);
                    }
                }))
                .on_drag(SideBrowserDrag(id), move |_, _, _, cx| {
                    cx.new(|_| super::task_card_input::Ghost {
                        title: drag_title.clone(),
                        tokens: drag_tokens,
                    })
                })
                .child(div().max_w(px(160.)).truncate().child(title.clone()))
                .child(
                    div()
                        .id(("side-browser-close", id))
                        .debug_selector(move || format!("side-browser-close-{id}"))
                        .aria_label(if snapshot.french {
                            "Fermer l’onglet"
                        } else {
                            "Close tab"
                        })
                        .p(px(2.))
                        .rounded(px(3.))
                        .hover(|s| s.bg(tokens.surface_3))
                        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |root, _, _, cx| {
                            cx.stop_propagation();
                            root.dispatch(GpuiAction::CloseSideBrowser(id), cx);
                        }))
                        .child(GpuiIcon::Close.render(10.5, tokens.text_dim)),
                ),
        );
    }
    tab_strip = tab_strip.child(tabs_left);
    #[cfg(windows)]
    {
        tab_strip = tab_strip.child(
            div()
                .id("side-browser-new")
                .flex_shrink_0()
                .debug_selector(|| "side-browser-new".into())
                .aria_label(if snapshot.french {
                    "Nouvelle page Web"
                } else {
                    "New browser page"
                })
                .p(px(4.))
                .cursor_pointer()
                .on_click(
                    cx.listener(|root, _, _, cx| root.dispatch(GpuiAction::NewSideBrowser, cx)),
                )
                .child(GpuiIcon::Plus.render(14., tokens.text_dim)),
        );
    }
    tab_strip = tab_strip.child(
        div()
            .id("btn-right-fullscreen")
            .flex_shrink_0()
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
        .debug_selector(|| "browser-side-drop".into())
        .on_drop(cx.listener(|root, page: &CenterBrowserDrag, _, cx| {
            root.dispatch(GpuiAction::DockBrowser(page.0), cx)
        }))
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

    let accent = tokens.accent;
    let french = snapshot.french;
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
                .child(if let Some(id) = snapshot.selected_side_browser {
                    render_side_browser(id, side_browser_input, snapshot, tokens, cx)
                } else {
                    side_panels_view::render_side_panel_tab_body(
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
                    )
                    .into_any_element()
                }),
        )
        .when(
            cx.active_drag()
                .is_some_and(|drag| drag.value.is::<CenterBrowserDrag>()),
            |el| {
                el.child(
                    gpui::canvas(
                        |_, _, _| (),
                        move |bounds, (), window, cx| {
                            if bounds.contains(&window.mouse_position()) {
                                paint_drop_zone(
                                    bounds,
                                    if french {
                                        "Ouvrir dans le panneau latéral"
                                    } else {
                                        "Open in the side panel"
                                    }
                                    .into(),
                                    accent,
                                    window,
                                    cx,
                                );
                            }
                        },
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                )
            },
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
