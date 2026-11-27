//! Unified `40px` frameless toolbar, `24px` broadcast/error banners, and `24px` status bar
//! for the GPUI primary renderer (`1:1` layout with `C:\Tessel`).

use super::{
    TesselGpuiRoot,
    icons::{GpuiIcon, render_brand_logo},
    theme::{
        BROADCAST_BANNER_HEIGHT_PX, CAPTION_BTN_WIDTH_PX, GpuiThemeTokens, STATUSBAR_HEIGHT_PX,
        TOOLBAR_BTN_HEIGHT_PX, TOOLBAR_HEIGHT_PX, TOOLBAR_ICON_BTN_WIDTH_PX, rgba_with_alpha,
    },
};
use crate::{
    app::gpui_bridge::{GpuiAction, GpuiWorkspaceSnapshot},
    share_panel::ShareGpuiAction,
    status_strip::StatusBarItemId,
};
use gpui::{
    AppContext as _, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, WindowControlArea, div,
    prelude::FluentBuilder as _, px,
};

/// Copy the path shown by the file viewer. Guest callers supply the shared relative path.
pub(super) fn copy_path_button(
    path: String,
    french: bool,
    colors: GpuiThemeTokens,
) -> impl IntoElement {
    let label = if french {
        "Copier le chemin"
    } else {
        "Copy path"
    };
    div()
        .id("viewer-copy-path")
        .debug_selector(|| "viewer-copy-path".into())
        .role(gpui::Role::Button)
        .aria_label(label)
        .cursor_pointer()
        .px(px(5.))
        .py(px(2.))
        .child(GpuiIcon::CopyPath.render(14., colors.text_dim))
        .tooltip(move |_, cx| {
            cx.new(|_| ToolbarTooltip {
                text: label.into(),
                bg: colors.surface,
                fg: colors.text,
                border: colors.border,
            })
            .into()
        })
        .on_click(move |_, _, cx| {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(path.clone()));
        })
}

/// Plain text tooltip (the Live Share pill's full words, the `⋯` button's title).
pub(crate) struct ToolbarTooltip {
    pub text: SharedString,
    pub bg: gpui::Rgba,
    pub fg: gpui::Rgba,
    pub border: gpui::Rgba,
}

impl gpui::Render for ToolbarTooltip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(8.0))
            .py(px(4.0))
            .rounded(px(5.0))
            .bg(self.bg)
            .border_1()
            .border_color(self.border)
            .text_size(px(11.5))
            .text_color(self.fg)
            .child(self.text.clone())
    }
}

/// Estimated room of the GPUI toolbar (`toolbarFit.js`): the groups' widths with everything shown,
/// from the same fixed item sizes this file renders.
pub(crate) fn toolbar_fit_level(snapshot: &GpuiWorkspaceSnapshot, viewport_w: f32) -> u8 {
    let tb = &snapshot.toolbar;
    let french = snapshot.french;
    let text_w = |s: &str, per: f32| s.chars().count() as f32 * per;
    let icon = TOOLBAR_ICON_BTN_WIDTH_PX + 4.0;
    let version = text_w(&toolbar_version_text(&tb.build_label), 6.0);
    let mut left = 10.0 + 18.0 + 7.0 + 42.0 + 7.0 + version + 4.0 + 6.0;
    if tb.has_active_project {
        left += 8.0 + 12.0 + 5.0 + 13.0 + 5.0 + text_w(&tb.preferred_shell_label, 6.6) + 10.0 + 6.0;
        left += 24.0 + 6.0 + 70.0 + 6.0 + 72.0 + 6.0;
        for (_, label) in tb.installed_agents.iter().take(2) {
            left += text_w(label, 6.6) + 16.0 + 6.0;
        }
    }
    left += 24.0; // the two drag spacers' minimum
    let left_toggle = TOOLBAR_ICON_BTN_WIDTH_PX + 6.0;
    let broadcast_label = if french { "Diffusion" } else { "Broadcast" };
    let broadcast = text_w(broadcast_label, 6.6) + 13.0 + 5.0 + 18.0 + 4.0;
    let mut right = left_toggle;
    let mut secondary = left_toggle + broadcast + icon * 2.0 + icon; // + right toggle
    right += broadcast + icon * 2.0 + icon;
    if let Some(pill) = &tb.dev_update_pill {
        right += text_w(pill, 6.4) + 20.0;
    }
    if let Some(label) = &tb.share_pill_label {
        right += live_share_pill_width(label, tb.share_pill_badge.is_some()) + 4.0;
    } else {
        right += icon;
        secondary += icon;
    }
    right += icon; // Issues is also available without an active project.
    secondary += icon;
    if tb.waiting_agents_count > 0 {
        right += 130.0;
    }
    right += icon * 4.0; // usage, notifications, help, settings
    if tb.unread_notifications > 0 {
        right += 18.0;
    }
    right += 6.0 + 3.0 * CAPTION_BTN_WIDTH_PX;
    crate::toolbar_fit::ToolbarRoom {
        avail: viewport_w,
        left: left - left_toggle,
        right,
        secondary,
        version,
    }
    .level()
}

fn toolbar_version_text(build_label: &str) -> String {
    let count = build_label
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_start_matches('b');
    format!("{}.{}", env!("CARGO_PKG_VERSION"), count)
}

fn live_share_pill_width(label: &str, badge: bool) -> f32 {
    (6.0 + crate::toolbar_fit::LIVE_SHARE_PILL_DOT
        + 5.0
        + label.chars().count() as f32 * 6.0
        + if badge { 20.0 } else { 0.0 }
        + 7.0)
        .min(160.0)
}

pub(crate) fn render_toolbar(
    snapshot: &GpuiWorkspaceSnapshot,
    issues_menu: &super::issues_menu::Menu,
    tokens: &GpuiThemeTokens,
    is_maximized: bool,
    viewport_w: f32,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let tb = &snapshot.toolbar;
    let french = snapshot.french;
    let level = toolbar_fit_level(snapshot, viewport_w);
    let full = level == 0;

    let search_placeholder = if french {
        "Rechercher ou lancer une commande..."
    } else {
        "Search or run a command..."
    };
    let broadcast_label = if french { "Diffusion" } else { "Broadcast" };

    div()
        .id("gpui-toolbar")
        .h(px(TOOLBAR_HEIGHT_PX))
        .w_full()
        .flex()
        .items_center()
        .bg(tokens.titlebar_bg)
        .border_b_1()
        .border_color(tokens.border)
        .text_color(tokens.text)
        .text_size(px(12.0))
        // Left cluster: Brand mosaic + Sidebar toggle + Quick launch pills
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .pl(px(10.0))
                .pr(px(6.0))
                .h_full()
                .child(
                    div()
                        .id("toolbar-brand-drag")
                        .flex()
                        .items_center()
                        .gap(px(7.0))
                        .pr(px(4.0))
                        .h_full()
                        .window_control_area(WindowControlArea::Drag)
                        .child(render_brand_logo(tokens))
                        .child(
                            div()
                                .text_size(px(12.5))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(tokens.text_strong)
                                .child("Tessel"),
                        )
                        .when(level < 2, |el| {
                            el.child(
                                div().text_size(px(11.0)).text_color(tokens.text_dim).child(
                                    SharedString::from(toolbar_version_text(&tb.build_label)),
                                ),
                            )
                        }),
                )
                .when(full, |el| {
                    el.child(toolbar_icon_btn(
                        "btn-toggle-left-sidebar",
                        GpuiIcon::SidebarLeft,
                        tb.left_open,
                        tokens,
                        GpuiAction::ToggleLeftSidebar,
                        cx,
                    ))
                })
                .when(tb.has_active_project, |el| {
                    let shell_title: SharedString = tb.preferred_shell_label.clone().into();
                    let mut row = el
                        .child(
                            div()
                                .id("btn-new-shell")
                                .h(px(TOOLBAR_BTN_HEIGHT_PX))
                                .px(px(8.0))
                                .rounded(px(tokens.radius_px))
                                .flex()
                                .items_center()
                                .gap(px(5.0))
                                .bg(tokens.surface_2)
                                .border_1()
                                .border_color(tokens.border)
                                .cursor_pointer()
                                .hover(|s| {
                                    s.bg(tokens.surface_3).border_color(tokens.border_strong)
                                })
                                .on_click(cx.listener(|root, _, _, cx| {
                                    root.dispatch(GpuiAction::NewShell, cx);
                                }))
                                .child(GpuiIcon::Plus.render(12.0, tokens.text_strong))
                                .child(GpuiIcon::Terminal.render(13.0, tokens.accent))
                                .child(div().text_color(tokens.text_strong).child(shell_title)),
                        )
                        .child(
                            div()
                                .id("btn-launch-menu-chevron")
                                .h(px(TOOLBAR_BTN_HEIGHT_PX))
                                .px(px(5.0))
                                .rounded(px(tokens.radius_px))
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(if snapshot.modals.launch_menu_open {
                                    tokens.accent_soft_bg
                                } else {
                                    tokens.surface_2
                                })
                                .border_1()
                                .border_color(if snapshot.modals.launch_menu_open {
                                    tokens.accent
                                } else {
                                    tokens.border
                                })
                                .cursor_pointer()
                                .hover(|s| {
                                    s.bg(tokens.surface_3).border_color(tokens.border_strong)
                                })
                                .on_click(cx.listener(|root, _, _, cx| {
                                    root.dispatch(GpuiAction::ToggleLaunchMenu, cx);
                                }))
                                .child(GpuiIcon::ChevronDown.render(12.0, tokens.text)),
                        )
                        .child(
                            div()
                                .id("btn-launch-codex-chat")
                                .h(px(TOOLBAR_BTN_HEIGHT_PX))
                                .px(px(8.0))
                                .rounded(px(tokens.radius_px))
                                .flex()
                                .items_center()
                                .gap(px(5.0))
                                .bg(tokens.surface)
                                .border_1()
                                .border_color(tokens.border)
                                .cursor_pointer()
                                .hover(|s| s.bg(tokens.surface_2).border_color(tokens.accent))
                                .on_click(cx.listener(|root, _, _, cx| {
                                    root.dispatch(GpuiAction::LaunchCodexChat, cx);
                                }))
                                .child(GpuiIcon::Sparkles.render(12.5, tokens.accent))
                                .child(div().text_color(tokens.text).child("Codex")),
                        )
                        .child(
                            div()
                                .id("btn-launch-claude-chat")
                                .h(px(TOOLBAR_BTN_HEIGHT_PX))
                                .px(px(8.0))
                                .rounded(px(tokens.radius_px))
                                .flex()
                                .items_center()
                                .gap(px(5.0))
                                .bg(tokens.surface)
                                .border_1()
                                .border_color(tokens.border)
                                .cursor_pointer()
                                .hover(|s| s.bg(tokens.surface_2).border_color(tokens.accent))
                                .on_click(cx.listener(|root, _, _, cx| {
                                    root.dispatch(GpuiAction::LaunchClaudeChat, cx);
                                }))
                                .child(GpuiIcon::Sparkles.render(12.5, tokens.warn))
                                .child(div().text_color(tokens.text).child("Claude")),
                        );

                    for (idx, (id, label)) in tb.installed_agents.iter().take(2).enumerate() {
                        let agent_id = id.clone();
                        let agent_label: SharedString = label.clone().into();
                        row = row.child(
                            div()
                                .id(("btn-toolbar-agent", idx))
                                .h(px(TOOLBAR_BTN_HEIGHT_PX))
                                .px(px(7.0))
                                .rounded(px(tokens.radius_px))
                                .flex()
                                .items_center()
                                .gap(px(4.0))
                                .bg(tokens.surface)
                                .border_1()
                                .border_color(tokens.border)
                                .cursor_pointer()
                                .hover(|s| s.bg(tokens.surface_2))
                                .on_click(cx.listener(move |root, _, _, cx| {
                                    root.dispatch(GpuiAction::LaunchAgent(agent_id.clone()), cx);
                                }))
                                .child(div().text_color(tokens.text_dim).child(agent_label)),
                        );
                    }
                    row
                }),
        )
        // Left drag spacer
        .child(
            div()
                .id("toolbar-drag-left")
                .flex_1()
                .h_full()
                .min_w(px(12.0))
                .window_control_area(WindowControlArea::Drag),
        )
        // Centered Command Palette trigger (`Ctrl+K`); level 2: its icon only (30 px)
        .child(
            div()
                .id("btn-command-palette")
                .h(px(TOOLBAR_BTN_HEIGHT_PX))
                .when(level >= 2, |el| {
                    el.w(px(crate::toolbar_fit::TOOLBAR_COMMAND_ICON_WIDTH))
                        .flex_shrink_0()
                        .justify_center()
                })
                .when(level < 2, |el| {
                    el.w(px(280.0))
                        .flex_shrink(1.0)
                        .min_w(px(crate::toolbar_fit::TOOLBAR_CENTRE_MIN))
                        .px(px(10.0))
                        .justify_between()
                })
                .rounded(px(tokens.radius_px))
                .flex()
                .items_center()
                .bg(if tb.command_palette_open {
                    tokens.surface_3
                } else {
                    tokens.surface
                })
                .border_1()
                .border_color(if tb.command_palette_open {
                    tokens.accent
                } else {
                    tokens.border
                })
                .cursor_pointer()
                .hover(|s| s.bg(tokens.surface_2).border_color(tokens.border_strong))
                .on_click(cx.listener(|root, _, _, cx| {
                    root.dispatch(GpuiAction::ToggleCommandPalette, cx);
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .overflow_hidden()
                        .child(GpuiIcon::Search.render(13.0, tokens.text_dim))
                        .when(level < 2, |el| {
                            el.child(
                                div()
                                    .text_color(tokens.text_dim)
                                    .truncate()
                                    .child(search_placeholder),
                            )
                        }),
                )
                .when(level < 2, |el| {
                    el.child(
                        div()
                            .px(px(5.0))
                            .py(px(1.0))
                            .rounded(px(4.0))
                            .bg(tokens.surface_2)
                            .border_1()
                            .border_color(tokens.border)
                            .text_size(px(10.5))
                            .text_color(tokens.text_dim)
                            .child("Ctrl+K"),
                    )
                }),
        )
        // Right drag spacer
        .child(
            div()
                .id("toolbar-drag-right")
                .flex_1()
                .h_full()
                .min_w(px(12.0))
                .window_control_area(WindowControlArea::Drag),
        )
        // Right controls + Window caption buttons
        .child(
            div()
                .flex()
                .items_center()
                .h_full()
                .gap(px(4.0))
                .when_some(tb.dev_update_pill.clone(), |el, pill_txt| {
                    el.child(
                        div()
                            .id("btn-dev-update-pill")
                            .h(px(TOOLBAR_BTN_HEIGHT_PX))
                            .px(px(8.0))
                            .rounded(px(tokens.radius_px))
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .bg(rgba_with_alpha(tokens.warn, 0.20))
                            .border_1()
                            .border_color(tokens.warn)
                            .text_size(px(11.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(tokens.warn)
                            .cursor_pointer()
                            .hover(|s| s.bg(rgba_with_alpha(tokens.warn, 0.30)))
                            .on_click(cx.listener(|root, _, _, cx| {
                                root.dispatch(GpuiAction::ToggleDevUpdateDialog, cx);
                            }))
                            .child(SharedString::from(pill_txt)),
                    )
                })
                .when(full, |el| {
                    el.child(
                        div()
                            .id("btn-broadcast-toggle")
                            .h(px(TOOLBAR_BTN_HEIGHT_PX))
                            .px(px(8.0))
                            .rounded(px(tokens.radius_px))
                            .flex()
                            .items_center()
                            .gap(px(5.0))
                            .bg(if tb.broadcast_enabled {
                                rgba_with_alpha(tokens.warn, 0.20)
                            } else {
                                tokens.surface
                            })
                            .border_1()
                            .border_color(if tb.broadcast_enabled {
                                tokens.warn
                            } else {
                                tokens.border
                            })
                            .cursor_pointer()
                            .hover(|s| s.bg(tokens.surface_2))
                            .on_click(cx.listener(|root, _, _, cx| {
                                root.dispatch(GpuiAction::ToggleBroadcastMode, cx);
                            }))
                            .child(GpuiIcon::Radio.render(
                                13.0,
                                if tb.broadcast_enabled {
                                    tokens.warn
                                } else {
                                    tokens.text_dim
                                },
                            ))
                            .child(
                                div()
                                    .text_color(if tb.broadcast_enabled {
                                        tokens.broadcast_banner_fg
                                    } else {
                                        tokens.text_dim
                                    })
                                    .child(if tb.broadcast_count > 0 {
                                        SharedString::from(format!(
                                            "{broadcast_label} ({})",
                                            tb.broadcast_count
                                        ))
                                    } else {
                                        SharedString::new_static(broadcast_label)
                                    }),
                            ),
                    )
                    .child(toolbar_icon_btn(
                        "btn-autotile-grid",
                        GpuiIcon::LayoutGrid,
                        false,
                        tokens,
                        GpuiAction::AutoTilePanes,
                        cx,
                    ))
                    .child(toolbar_icon_btn(
                        "btn-floating-terminal",
                        GpuiIcon::PanelBottom,
                        tb.floating_open,
                        tokens,
                        GpuiAction::ToggleFloatingTerminal,
                        cx,
                    ))
                })
                // Live Share: the compact 20 px pill while sharing or in a session (always
                // shown), else the icon (a secondary button).
                .when_some(tb.share_pill_label.clone(), |el, label| {
                    let dot = if tb.share_pill_dot_live {
                        gpui::rgb(0x00ef_4444)
                    } else if tb.share_pill_badge.is_none() && label == "0" {
                        tokens.text_dim
                    } else {
                        tokens.accent
                    };
                    let tip: SharedString =
                        tb.share_pill_tooltip.clone().unwrap_or_default().into();
                    let (tip_bg, tip_fg, tip_border) =
                        (tokens.surface_2, tokens.text_strong, tokens.border);
                    el.child(
                        div()
                            .id("btn-live-share-pill")
                            .h(px(crate::toolbar_fit::LIVE_SHARE_PILL_HEIGHT))
                            .max_w(px(160.0))
                            .pl(px(6.0))
                            .pr(px(7.0))
                            .rounded(px(10.0))
                            .flex()
                            .items_center()
                            .gap(px(5.0))
                            .bg(rgba_with_alpha(dot, 0.12))
                            .border_1()
                            .border_color(rgba_with_alpha(dot, 0.55))
                            .text_size(px(11.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(tokens.surface_2))
                            .tooltip(move |_window, cx| {
                                let text = tip.clone();
                                cx.new(|_| ToolbarTooltip {
                                    text,
                                    bg: tip_bg,
                                    fg: tip_fg,
                                    border: tip_border,
                                })
                                .into()
                            })
                            .on_click(cx.listener(|root, _, _, cx| {
                                root.dispatch(
                                    GpuiAction::Share(ShareGpuiAction::TogglePillPopover),
                                    cx,
                                );
                            }))
                            .child(
                                div()
                                    .w(px(crate::toolbar_fit::LIVE_SHARE_PILL_DOT))
                                    .h(px(crate::toolbar_fit::LIVE_SHARE_PILL_DOT))
                                    .rounded_full()
                                    .bg(dot),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_color(tokens.text_strong)
                                    .child(SharedString::from(label)),
                            )
                            .when_some(tb.share_pill_badge.clone(), |el, badge| {
                                el.child(
                                    div()
                                        .px(px(4.0))
                                        .rounded(px(7.0))
                                        .bg(gpui::rgb(0x00f5_9e0b))
                                        .text_size(px(10.0))
                                        .text_color(gpui::rgb(0x0000_0000))
                                        .child(SharedString::from(badge)),
                                )
                            }),
                    )
                })
                .when(full && tb.share_pill_label.is_none(), |el| {
                    el.child(toolbar_icon_btn(
                        "btn-live-share",
                        GpuiIcon::Share2,
                        tb.share_active,
                        tokens,
                        GpuiAction::OpenLiveShare,
                        cx,
                    ))
                })
                .when(full, |el| {
                    el.child(super::issues_menu::button(
                        issues_menu,
                        snapshot.toolbar.issues_menu_open,
                        tokens,
                        cx,
                    ))
                })
                .when(tb.waiting_agents_count > 0, |el| {
                    let first_waiting = tb.first_waiting_pane;
                    let req_label = if french {
                        format!("Demande agent ({})", tb.waiting_agents_count)
                    } else {
                        format!("Agent request ({})", tb.waiting_agents_count)
                    };
                    el.child(
                        div()
                            .id("pill-waiting-agents")
                            .h(px(TOOLBAR_BTN_HEIGHT_PX))
                            .px(px(8.0))
                            .rounded(px(tokens.radius_px))
                            .flex()
                            .items_center()
                            .gap(px(5.0))
                            .bg(rgba_with_alpha(tokens.warn, 0.18))
                            .border_1()
                            .border_color(tokens.warn)
                            .cursor_pointer()
                            .hover(|s| s.bg(rgba_with_alpha(tokens.warn, 0.28)))
                            .on_click(cx.listener(move |root, _, _, cx| {
                                if let Some(pid) = first_waiting {
                                    root.dispatch(GpuiAction::FocusSession(pid), cx);
                                }
                            }))
                            .child(GpuiIcon::Activity.render(12.5, tokens.warn))
                            .child(
                                div()
                                    .text_color(tokens.warn)
                                    .child(SharedString::from(req_label)),
                            ),
                    )
                })
                .child(toolbar_icon_btn(
                    "btn-usage-popover",
                    GpuiIcon::Gauge,
                    snapshot.modals.usage_popover_open,
                    tokens,
                    GpuiAction::ToggleUsagePopover,
                    cx,
                ))
                .child(
                    div()
                        .id("pill-notifications")
                        .relative()
                        .h(px(TOOLBAR_BTN_HEIGHT_PX))
                        .min_w(px(TOOLBAR_ICON_BTN_WIDTH_PX))
                        .px(px(if tb.unread_notifications > 0 {
                            7.0
                        } else {
                            0.0
                        }))
                        .rounded(px(tokens.radius_px))
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(4.0))
                        .bg(
                            if snapshot.modals.notifications_popover_open
                                || tb.unread_notifications > 0
                            {
                                tokens.accent_soft_bg
                            } else {
                                gpui::rgba(0x0000_0000)
                            },
                        )
                        .when(
                            snapshot.modals.notifications_popover_open
                                || tb.unread_notifications > 0,
                            |el| el.border_1().border_color(tokens.accent),
                        )
                        .cursor_pointer()
                        .hover(|s| s.bg(tokens.surface_2))
                        .on_click(cx.listener(|root, _, _, cx| {
                            root.dispatch(GpuiAction::ToggleNotificationsPopover, cx);
                        }))
                        .child(GpuiIcon::Bell.render(
                            14.0,
                            if snapshot.modals.notifications_popover_open
                                || tb.unread_notifications > 0
                            {
                                tokens.accent
                            } else {
                                tokens.text_dim
                            },
                        ))
                        .when(tb.unread_notifications > 0, |el| {
                            el.child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(tokens.text_strong)
                                    .child(SharedString::from(tb.unread_notifications.to_string())),
                            )
                        }),
                )
                .child(toolbar_icon_btn(
                    "btn-help",
                    GpuiIcon::HelpCircle,
                    snapshot.modals.help_open,
                    tokens,
                    GpuiAction::OpenHelp,
                    cx,
                ))
                // `⋯`: the secondary buttons' actions when the toolbar is tight (level >= 1)
                .when(!full, |el| {
                    let open = tb.more_menu_open;
                    let dot_color = if open { tokens.accent } else { tokens.text_dim };
                    let dot = move || div().w(px(3.0)).h(px(3.0)).rounded_full().bg(dot_color);
                    let title: SharedString = crate::toolbar_fit::more_button_title(french).into();
                    let (tip_bg, tip_fg, tip_border) =
                        (tokens.surface_2, tokens.text_strong, tokens.border);
                    el.child(
                        div()
                            .id("btn-toolbar-more")
                            .w(px(TOOLBAR_ICON_BTN_WIDTH_PX
                                .max(crate::toolbar_fit::TOOLBAR_MORE_WIDTH)))
                            .h(px(TOOLBAR_BTN_HEIGHT_PX))
                            .rounded(px(tokens.radius_px))
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap(px(1.8))
                            .when(open, |el| {
                                el.bg(tokens.accent_soft_bg)
                                    .border_1()
                                    .border_color(tokens.accent)
                            })
                            .cursor_pointer()
                            .hover(|s| s.bg(tokens.surface_2))
                            .tooltip(move |_window, cx| {
                                let text = title.clone();
                                cx.new(|_| ToolbarTooltip {
                                    text,
                                    bg: tip_bg,
                                    fg: tip_fg,
                                    border: tip_border,
                                })
                                .into()
                            })
                            .on_click(cx.listener(|root, _, _, cx| {
                                root.dispatch(GpuiAction::ToggleToolbarMore, cx);
                            }))
                            .child(dot())
                            .child(dot())
                            .child(dot()),
                    )
                })
                .child(toolbar_icon_btn(
                    "btn-settings",
                    GpuiIcon::Settings,
                    snapshot.settings.is_some(),
                    tokens,
                    GpuiAction::OpenSettings,
                    cx,
                ))
                .when(full, |el| {
                    el.child(toolbar_icon_btn(
                        "btn-toggle-right-sidebar",
                        GpuiIcon::SidebarRight,
                        tb.right_open,
                        tokens,
                        GpuiAction::ToggleRightSidebar,
                        cx,
                    ))
                })
                // Native Win32/Client caption buttons (`3 × 46×40px`)
                .child(
                    div()
                        .ml(px(6.0))
                        .flex()
                        .items_center()
                        .h_full()
                        .child(
                            div()
                                .id("win-btn-minimize")
                                .w(px(CAPTION_BTN_WIDTH_PX))
                                .h_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .window_control_area(WindowControlArea::Min)
                                .hover(|s| s.bg(tokens.surface_2))
                                .on_click(|_, window: &mut Window, _| {
                                    window.minimize_window();
                                })
                                .child(GpuiIcon::WinMinimize.render(10.0, tokens.titlebar_symbol)),
                        )
                        .child(
                            div()
                                .id("win-btn-maximize")
                                .w(px(CAPTION_BTN_WIDTH_PX))
                                .h_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .window_control_area(WindowControlArea::Max)
                                .hover(|s| s.bg(tokens.surface_2))
                                .on_click(|_, window: &mut Window, _| {
                                    window.zoom_window();
                                })
                                .child(if is_maximized {
                                    GpuiIcon::WinRestore.render(10.0, tokens.titlebar_symbol)
                                } else {
                                    GpuiIcon::WinMaximize.render(10.0, tokens.titlebar_symbol)
                                }),
                        )
                        .child(
                            div()
                                .id("win-btn-close")
                                .w(px(CAPTION_BTN_WIDTH_PX))
                                .h_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .window_control_area(WindowControlArea::Close)
                                .hover(|s| s.bg(tokens.caption_close_hover_bg))
                                .on_click(|_, window: &mut Window, _| {
                                    window.remove_window();
                                })
                                .child(GpuiIcon::WinClose.render(10.0, tokens.titlebar_symbol)),
                        ),
                ),
        )
}

fn toolbar_icon_btn(
    id: &'static str,
    icon: GpuiIcon,
    active: bool,
    tokens: &GpuiThemeTokens,
    action: GpuiAction,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    div()
        .id(id)
        .w(px(TOOLBAR_ICON_BTN_WIDTH_PX))
        .h(px(TOOLBAR_BTN_HEIGHT_PX))
        .rounded(px(tokens.radius_px))
        .flex()
        .items_center()
        .justify_center()
        .bg(if active {
            tokens.accent_soft_bg
        } else {
            gpui::rgba(0x0000_0000)
        })
        .when(active, |el| el.border_1().border_color(tokens.accent))
        .cursor_pointer()
        .hover(|s| s.bg(tokens.surface_2))
        .on_click(cx.listener(move |root, _, _, cx| {
            root.dispatch(action.clone(), cx);
        }))
        .child(icon.render(
            14.5,
            if active {
                tokens.accent
            } else {
                tokens.text_dim
            },
        ))
}

pub(crate) fn render_broadcast_banner(
    snapshot: &GpuiWorkspaceSnapshot,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> Option<impl IntoElement> {
    if !snapshot.toolbar.broadcast_enabled {
        return None;
    }
    let msg = if snapshot.french {
        format!(
            "Mode diffusion actif — la saisie clavier est envoyée à {} terminal(aux).",
            snapshot.toolbar.broadcast_count
        )
    } else {
        format!(
            "Broadcast mode active — keystrokes are sent to {} terminal pane(s).",
            snapshot.toolbar.broadcast_count
        )
    };
    let stop_label = if snapshot.french {
        "Arrêter"
    } else {
        "Turn off"
    };

    Some(
        div()
            .id("gpui-broadcast-banner")
            .h(px(BROADCAST_BANNER_HEIGHT_PX))
            .w_full()
            .px(px(12.0))
            .flex()
            .items_center()
            .justify_between()
            .bg(tokens.broadcast_banner_bg)
            .border_b_1()
            .border_color(rgba_with_alpha(tokens.warn, 0.45))
            .text_size(px(11.5))
            .text_color(tokens.broadcast_banner_fg)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(GpuiIcon::Radio.render(12.5, tokens.warn))
                    .child(SharedString::from(msg)),
            )
            .child(
                div()
                    .id("btn-stop-broadcast")
                    .px(px(7.0))
                    .py(px(1.5))
                    .rounded(px(4.0))
                    .bg(rgba_with_alpha(tokens.warn, 0.22))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgba_with_alpha(tokens.warn, 0.35)))
                    .on_click(cx.listener(|root, _, _, cx| {
                        root.dispatch(GpuiAction::ToggleBroadcastMode, cx);
                    }))
                    .child(stop_label),
            ),
    )
}

pub(crate) fn render_error_banner(
    snapshot: &GpuiWorkspaceSnapshot,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> Option<impl IntoElement> {
    let err = snapshot.toolbar.error_banner.as_ref()?;
    Some(
        div()
            .id("gpui-error-banner")
            .min_h(px(24.0))
            .w_full()
            .px(px(12.0))
            .py(px(3.0))
            .flex()
            .items_center()
            .justify_between()
            .bg(rgba_with_alpha(tokens.danger, 0.18))
            .border_b_1()
            .border_color(tokens.danger)
            .text_size(px(11.5))
            .text_color(tokens.text_strong)
            .child(SharedString::from(err.clone()))
            .child(
                div()
                    .id("btn-dismiss-error")
                    .p(px(2.0))
                    .rounded(px(4.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(tokens.surface_2))
                    .on_click(cx.listener(|root, _, _, cx| {
                        root.dispatch(GpuiAction::DismissError, cx);
                    }))
                    .child(GpuiIcon::Close.render(12.0, tokens.text_strong)),
            ),
    )
}

pub(crate) fn render_status_bar(
    snapshot: &GpuiWorkspaceSnapshot,
    tokens: &GpuiThemeTokens,
    viewport_width_px: f32,
    cx: &mut Context<TesselGpuiRoot>,
) -> Option<impl IntoElement> {
    let sb = &snapshot.status_bar;
    if !sb.visible {
        return None;
    }

    let awake_text = match sb.keep_awake {
        crate::workspace::KeepAwakeSetting::Off => {
            if snapshot.french {
                "Veille auto"
            } else {
                "Sleep normal"
            }
        }
        crate::workspace::KeepAwakeSetting::Agents => {
            if snapshot.french {
                "Éveil (agents)"
            } else {
                "Awake (auto)"
            }
        }
        crate::workspace::KeepAwakeSetting::On => {
            if snapshot.french {
                "Toujours éveillé"
            } else {
                "Always awake"
            }
        }
    };

    let sessions_label = if sb.working_agents > 0 {
        format!("{} panes · {} active", sb.session_count, sb.working_agents)
    } else {
        format!("{} panes", sb.session_count)
    };

    Some(
        div()
            .id("gpui-statusbar")
            .h(px(STATUSBAR_HEIGHT_PX))
            .w_full()
            .px(px(10.0))
            .flex()
            .items_center()
            .justify_between()
            .bg(tokens.chrome)
            .border_t_1()
            .border_color(tokens.border)
            .text_size(px(11.0))
            .text_color(tokens.text_dim)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .overflow_hidden()
                    .child(
                        div()
                            .id("status-keep-awake")
                            .px(px(6.0))
                            .py(px(1.5))
                            .rounded(px(4.0))
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .bg(if sb.keep_awake_active {
                                tokens.accent_soft_bg
                            } else {
                                gpui::rgba(0x0000_0000)
                            })
                            .cursor_pointer()
                            .hover(|s| s.bg(tokens.surface_2))
                            .on_click(cx.listener(|root, _, _, cx| {
                                root.dispatch(GpuiAction::CycleKeepAwake, cx);
                            }))
                            .child(GpuiIcon::Coffee.render(
                                11.5,
                                if sb.keep_awake_active {
                                    tokens.accent
                                } else {
                                    tokens.text_dim
                                },
                            ))
                            .child(
                                div()
                                    .text_color(if sb.keep_awake_active {
                                        tokens.text_strong
                                    } else {
                                        tokens.text_dim
                                    })
                                    .child(awake_text),
                            ),
                    )
                    .when_some(
                        sb.workspace_path
                            .as_ref()
                            .filter(|_| viewport_width_px >= 900.0),
                        |el, ws_path| {
                            el.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(4.0))
                                    .max_w(px(340.0))
                                    .overflow_hidden()
                                    .child(GpuiIcon::Folder.render(11.5, tokens.text_dim))
                                    .child(
                                        div().truncate().child(SharedString::from(ws_path.clone())),
                                    ),
                            )
                        },
                    )
                    .when_some(sb.git_branch.as_ref(), |el, branch| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.0))
                                .child(GpuiIcon::GitBranch.render(11.5, tokens.accent))
                                .child(
                                    div()
                                        .text_color(tokens.text)
                                        .child(SharedString::from(branch.clone())),
                                ),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .child(GpuiIcon::Terminal.render(11.5, tokens.text_dim))
                            .child(SharedString::from(sessions_label)),
                    )
                    .child(
                        div()
                            .id("status-ssh-pill")
                            .px(px(5.0))
                            .py(px(1.0))
                            .rounded(px(4.0))
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .bg(
                                if snapshot.modals.status_popover == Some(StatusBarItemId::Ssh) {
                                    tokens.accent_soft_bg
                                } else {
                                    gpui::rgba(0x0000_0000)
                                },
                            )
                            .cursor_pointer()
                            .hover(|s| s.bg(tokens.surface_2))
                            .on_click(cx.listener(|root, _, _, cx| {
                                root.dispatch(
                                    GpuiAction::ToggleStatusPopover(StatusBarItemId::Ssh),
                                    cx,
                                );
                            }))
                            .child(GpuiIcon::Server.render(11.5, tokens.text_dim))
                            .child(SharedString::from(format!("SSH ({})", sb.ssh_host_count))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .id("status-ports-pill")
                            .px(px(5.0))
                            .py(px(1.0))
                            .rounded(px(4.0))
                            .bg(
                                if snapshot.modals.status_popover == Some(StatusBarItemId::Ports) {
                                    tokens.accent_soft_bg
                                } else {
                                    gpui::rgba(0x0000_0000)
                                },
                            )
                            .cursor_pointer()
                            .hover(|s| s.bg(tokens.surface_2))
                            .on_click(cx.listener(|root, _, _, cx| {
                                root.dispatch(
                                    GpuiAction::ToggleStatusPopover(StatusBarItemId::Ports),
                                    cx,
                                );
                            }))
                            .child(SharedString::from(format!(
                                "Ports: {}",
                                sb.listening_ports_count
                            ))),
                    )
                    .child(
                        div()
                            .id("status-resources-pill")
                            .px(px(5.0))
                            .py(px(1.0))
                            .rounded(px(4.0))
                            .bg(
                                if snapshot.modals.status_popover
                                    == Some(StatusBarItemId::Resources)
                                {
                                    tokens.accent_soft_bg
                                } else {
                                    gpui::rgba(0x0000_0000)
                                },
                            )
                            .cursor_pointer()
                            .hover(|s| s.bg(tokens.surface_2))
                            .on_click(cx.listener(|root, _, _, cx| {
                                root.dispatch(
                                    GpuiAction::ToggleStatusPopover(StatusBarItemId::Resources),
                                    cx,
                                );
                            }))
                            .child(SharedString::from(format!("{} MB", sb.app_rss_mb))),
                    )
                    .child(
                        div()
                            .id("status-renderer-pill")
                            .px(px(6.0))
                            .py(px(1.0))
                            .rounded(px(4.0))
                            .bg(tokens.accent_soft_bg)
                            .text_color(tokens.accent)
                            .cursor_pointer()
                            .on_click(cx.listener(|root, _, _, cx| {
                                root.dispatch(
                                    GpuiAction::OpenSettingsPage("appearance".into()),
                                    cx,
                                );
                            }))
                            .child("GPUI"),
                    )
                    .child(SharedString::from(sb.version_build.clone())),
            ),
    )
}
