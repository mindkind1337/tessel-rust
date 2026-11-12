//! Hardware-accelerated `GPUI` DirectWrite terminal & floating-terminal views
//! backed by `crate::terminal::Terminal` (`vt100::Parser`, scrollback, find, OAuth banner).

use super::{
    DragTarget, FocusInputTarget, TesselGpuiRoot,
    icons::GpuiIcon,
    theme::{GpuiThemeTokens, mono_font_family, rgba_with_alpha},
};
use crate::{
    app::gpui_bridge::{GpuiAction, GpuiFloatingTerminalSnapshot, GpuiPaneTileSnapshot},
    terminal::{TerminalRenderSnapshot, TerminalStyledSpan},
    workspace::TerminalCursorStyle,
};
use gpui::{
    ClipboardItem, Context, FontWeight, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, Rgba, ScrollWheelEvent, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder as _, px,
};

fn rgb_triplet(rgb: [u8; 3]) -> Rgba {
    gpui::rgb((u32::from(rgb[0]) << 16) | (u32::from(rgb[1]) << 8) | u32::from(rgb[2]))
}

pub(crate) fn render_terminal_pane_body(
    tile: &GpuiPaneTileSnapshot,
    inputs: &super::terminal_find_inputs::Inputs,
    font_size: f32,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let sid = tile.session_id;

    if tile.dormant {
        let wake_lbl = if french {
            "Réveiller le terminal"
        } else {
            "Wake terminal"
        };
        let switch_lbl = if french {
            "Passer en chat natif"
        } else {
            "Switch to native chat"
        };
        let dormant_desc = if french {
            "Session restaurée en veille — aucun processus actif tant que vous ne la réveillez pas."
        } else {
            "Restored session is sleeping — no background process runs until you wake it."
        };
        return div()
            .id(("pane-dormant", sid))
            .flex_1()
            .min_h_0()
            .w_full()
            .p(px(16.0))
            .flex()
            .flex_col()
            .items_start()
            .justify_center()
            .gap(px(10.0))
            .bg(tokens.term)
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(tokens.text_dim)
                    .child(dormant_desc),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .id(("btn-wake-session", sid))
                            .px(px(11.0))
                            .py(px(5.0))
                            .rounded(px(tokens.radius_px))
                            .bg(tokens.accent)
                            .text_size(px(11.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(gpui::rgb(0xFF_FF_FF))
                            .cursor_pointer()
                            .on_click(cx.listener(move |root, _, _, cx| {
                                root.focus_input = FocusInputTarget::Terminal(Some(sid));
                                root.dispatch(GpuiAction::WakeSession(sid), cx);
                            }))
                            .child(wake_lbl),
                    )
                    .when(tile.can_switch_chat_mode, |el| {
                        el.child(
                            div()
                                .id(("btn-dormant-switch-chat", sid))
                                .px(px(10.0))
                                .py(px(5.0))
                                .rounded(px(tokens.radius_px))
                                .bg(tokens.surface_2)
                                .border_1()
                                .border_color(tokens.border)
                                .text_size(px(11.5))
                                .text_color(tokens.text_strong)
                                .cursor_pointer()
                                .hover(|s| s.bg(tokens.surface_3))
                                .on_click(cx.listener(move |root, _, _, cx| {
                                    root.dispatch(GpuiAction::SwitchSessionChatMode(sid), cx);
                                }))
                                .child(switch_lbl),
                        )
                    }),
            )
            .into_any_element();
    }

    let Some(render_snap) = &tile.terminal_render else {
        let has_startup_err = !tile.error.is_empty();
        let msg = if has_startup_err {
            tile.error.clone()
        } else if tile.loading {
            if french {
                "Démarrage du terminal…".into()
            } else {
                "Starting terminal…".into()
            }
        } else {
            "Terminal ready.".into()
        };
        return div()
            .id(("pane-term-loading", sid))
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .flex_col()
            .justify_between()
            .bg(tokens.term)
            .child(
                div()
                    .p(px(12.0))
                    .font_family(mono_font_family())
                    .text_size(px(font_size))
                    .text_color(if tile.error.is_empty() {
                        tokens.text_dim
                    } else {
                        tokens.danger
                    })
                    .child(SharedString::from(msg)),
            )
            .when(has_startup_err, |el| {
                el.child(render_exit_overlay_pill(
                    sid, true, None, french, tokens, cx,
                ))
            })
            .into_any_element();
    };

    render_vt_surface(
        Some(sid),
        render_snap,
        inputs,
        tile.cursor_style,
        tile.active,
        &tile.error,
        font_size,
        french,
        tokens,
        cx,
    )
    .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn render_vt_surface(
    session_id: Option<u64>,
    snap: &TerminalRenderSnapshot,
    inputs: &super::terminal_find_inputs::Inputs,
    cursor_style: TerminalCursorStyle,
    focused_pane: bool,
    extra_error: &str,
    font_size: f32,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let salt = session_id.unwrap_or(u64::MAX);
    let row_height_px = (font_size * 1.34).round().max(13.0);

    let mut wrapper = div()
        .id(("vt-wrapper", salt))
        .flex_1()
        .min_h_0()
        .w_full()
        .flex()
        .flex_col()
        .overflow_hidden()
        .bg(tokens.term)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |root, _: &MouseDownEvent, _, cx| {
                root.focus_input = FocusInputTarget::Terminal(session_id);
                if let Some(sid) = session_id {
                    root.dispatch(GpuiAction::FocusSession(sid), cx);
                } else {
                    cx.notify();
                }
            }),
        )
        .on_scroll_wheel(cx.listener(move |root, ev: &ScrollWheelEvent, _, cx| {
            let dy = ev.delta.pixel_delta(px(row_height_px)).y;
            let lines = (f32::from(dy) / row_height_px).round() as i64;
            if lines != 0 {
                root.dispatch(
                    GpuiAction::TerminalScroll {
                        session_id,
                        delta_lines: lines,
                    },
                    cx,
                );
            }
        }));

    if !extra_error.is_empty() || (!snap.exited && !snap.status.is_empty()) {
        let banner_txt = if !extra_error.is_empty() {
            extra_error.to_owned()
        } else {
            snap.status.clone()
        };
        wrapper = wrapper.child(
            div()
                .px(px(8.0))
                .py(px(3.0))
                .bg(rgba_with_alpha(tokens.danger, 0.16))
                .border_b_1()
                .border_color(tokens.danger)
                .flex()
                .items_center()
                .justify_between()
                .text_size(px(11.0))
                .text_color(tokens.text_strong)
                .child(SharedString::from(banner_txt)),
        );
    }

    if let Some(lines_n) = snap.pending_multiline_lines {
        let msg = if french {
            format!("Coller {lines_n} lignes dans ce terminal ?")
        } else {
            format!("Paste {lines_n} lines into this terminal?")
        };
        wrapper = wrapper.child(
            div()
                .px(px(8.0))
                .py(px(4.0))
                .bg(tokens.surface_2)
                .border_b_1()
                .border_color(tokens.warn)
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .text_size(px(11.5))
                .child(div().text_color(tokens.warn).child(SharedString::from(msg)))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            div()
                                .id(("btn-confirm-paste", salt))
                                .px(px(8.0))
                                .py(px(2.0))
                                .rounded(px(4.0))
                                .bg(tokens.accent)
                                .text_color(gpui::rgb(0xFF_FF_FF))
                                .cursor_pointer()
                                .on_click(cx.listener(move |root, _, _, cx| {
                                    root.dispatch(
                                        GpuiAction::TerminalConfirmMultilinePaste {
                                            session_id,
                                            confirm: true,
                                        },
                                        cx,
                                    );
                                }))
                                .child(if french {
                                    "Coller quand même"
                                } else {
                                    "Paste anyway"
                                }),
                        )
                        .child(
                            div()
                                .id(("btn-cancel-paste", salt))
                                .px(px(8.0))
                                .py(px(2.0))
                                .rounded(px(4.0))
                                .bg(tokens.surface_3)
                                .text_color(tokens.text)
                                .cursor_pointer()
                                .on_click(cx.listener(move |root, _, _, cx| {
                                    root.dispatch(
                                        GpuiAction::TerminalConfirmMultilinePaste {
                                            session_id,
                                            confirm: false,
                                        },
                                        cx,
                                    );
                                }))
                                .child(if french { "Annuler" } else { "Cancel" }),
                        ),
                ),
        );
    }

    if let Some(prompt) = &snap.device_login {
        let url_open = prompt.url.clone();
        let url_copy = prompt.url.clone();
        let code_copy = prompt.device_code.clone();
        let host_badge = if prompt.known_host {
            prompt.host.clone()
        } else if french {
            format!("Hôte externe : {}", prompt.host)
        } else {
            format!("External host: {}", prompt.host)
        };
        wrapper = wrapper.child(
            div()
                .px(px(8.0))
                .py(px(5.0))
                .bg(tokens.surface_2)
                .border_b_1()
                .border_color(if prompt.known_host {
                    tokens.accent
                } else {
                    tokens.warn
                })
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(6.0))
                .text_size(px(11.0))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(tokens.text_strong)
                        .child(if french {
                            "Connexion détectée"
                        } else {
                            "Sign-in link detected"
                        }),
                )
                .child(
                    div()
                        .text_color(if prompt.known_host {
                            tokens.text_dim
                        } else {
                            tokens.warn
                        })
                        .child(SharedString::from(host_badge)),
                )
                .when_some(prompt.device_code.as_ref(), |el, code| {
                    el.child(
                        div()
                            .px(px(5.0))
                            .py(px(1.0))
                            .rounded(px(4.0))
                            .bg(tokens.accent_soft_bg)
                            .font_family(mono_font_family())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(tokens.accent)
                            .child(SharedString::from(format!("Code: {code}"))),
                    )
                })
                .child(
                    div()
                        .id(("oauth-open-tessel", salt))
                        .px(px(7.0))
                        .py(px(2.0))
                        .rounded(px(4.0))
                        .bg(tokens.accent)
                        .text_color(gpui::rgb(0xFF_FF_FF))
                        .cursor_pointer()
                        .on_click(cx.listener(move |root, _, _, cx| {
                            root.dispatch(GpuiAction::TerminalDismissDeviceLogin(session_id), cx);
                            root.dispatch(GpuiAction::TerminalOpenTesselUrl(url_open.clone()), cx);
                        }))
                        .child(if french {
                            "Ouvrir dans Tessel"
                        } else {
                            "Open in Tessel"
                        }),
                )
                .child(
                    div()
                        .id(("oauth-copy-url", salt))
                        .px(px(7.0))
                        .py(px(2.0))
                        .rounded(px(4.0))
                        .bg(tokens.surface_3)
                        .text_color(tokens.text_strong)
                        .cursor_pointer()
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(url_copy.clone()));
                        }))
                        .child(if french {
                            "Copier le lien"
                        } else {
                            "Copy link"
                        }),
                )
                .when_some(code_copy, |el, code| {
                    el.child(
                        div()
                            .id(("oauth-copy-code", salt))
                            .px(px(7.0))
                            .py(px(2.0))
                            .rounded(px(4.0))
                            .bg(tokens.surface_3)
                            .text_color(tokens.text_strong)
                            .cursor_pointer()
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(code.clone()));
                            }))
                            .child(if french {
                                "Copier le code"
                            } else {
                                "Copy code"
                            }),
                    )
                })
                .child(
                    div()
                        .id(("oauth-dismiss", salt))
                        .p(px(2.0))
                        .cursor_pointer()
                        .on_click(cx.listener(move |root, _, _, cx| {
                            root.dispatch(GpuiAction::TerminalDismissDeviceLogin(session_id), cx);
                        }))
                        .child(GpuiIcon::Close.render(11.0, tokens.text_dim)),
                ),
        );
    }

    if snap.find_open && inputs.visible(session_id, snap.find_input_id) {
        let find_id = snap.find_input_id;
        wrapper = wrapper.child(
            div()
                .px(px(8.0))
                .py(px(4.0))
                .bg(tokens.surface)
                .border_b_1()
                .border_color(tokens.border)
                .flex()
                .items_center()
                .gap(px(6.0))
                .text_size(px(11.5))
                .child(GpuiIcon::Search.render(12.0, tokens.accent))
                .child(inputs.render(session_id, snap, french, tokens, cx))
                .when(!snap.find_badge.is_empty(), |el| {
                    el.child(
                        div()
                            .text_size(px(10.5))
                            .text_color(tokens.text_dim)
                            .child(SharedString::from(snap.find_badge.clone())),
                    )
                })
                .child(
                    div()
                        .id(("term-find-prev", salt))
                        .px(px(6.0))
                        .py(px(1.5))
                        .rounded(px(4.0))
                        .bg(tokens.surface_2)
                        .cursor_pointer()
                        .on_click(cx.listener(move |root, _, window, cx| {
                            super::terminal_find_inputs::run(
                                root,
                                session_id,
                                find_id,
                                Some(-1),
                                window,
                                cx,
                            )
                        }))
                        .child("↑"),
                )
                .child(
                    div()
                        .id(("term-find-next", salt))
                        .px(px(6.0))
                        .py(px(1.5))
                        .rounded(px(4.0))
                        .bg(tokens.surface_2)
                        .cursor_pointer()
                        .on_click(cx.listener(move |root, _, window, cx| {
                            super::terminal_find_inputs::run(
                                root,
                                session_id,
                                find_id,
                                Some(1),
                                window,
                                cx,
                            )
                        }))
                        .child("↓"),
                )
                .child(
                    div()
                        .id(("term-find-close", salt))
                        .p(px(2.5))
                        .rounded(px(4.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(tokens.surface_2))
                        .on_click(cx.listener(move |root, _, window, cx| {
                            super::terminal_find_inputs::run(
                                root, session_id, find_id, None, window, cx,
                            )
                        }))
                        .child(GpuiIcon::Close.render(11.0, tokens.text_dim)),
                ),
        );
    }

    if snap.viewing_scrollback {
        let sb_txt = if french {
            format!("Historique (+{} lignes)", snap.scrollback_offset)
        } else {
            format!("Viewing scrollback (+{} lines)", snap.scrollback_offset)
        };
        wrapper = wrapper.child(
            div()
                .px(px(8.0))
                .py(px(3.0))
                .bg(tokens.accent_soft_bg)
                .border_b_1()
                .border_color(tokens.accent)
                .flex()
                .items_center()
                .justify_between()
                .text_size(px(11.0))
                .child(
                    div()
                        .text_color(tokens.accent)
                        .child(SharedString::from(sb_txt)),
                )
                .child(
                    div()
                        .id(("term-back-live", salt))
                        .px(px(7.0))
                        .py(px(1.5))
                        .rounded(px(4.0))
                        .bg(tokens.accent)
                        .text_color(gpui::rgb(0xFF_FF_FF))
                        .cursor_pointer()
                        .on_click(cx.listener(move |root, _, _, cx| {
                            root.dispatch(GpuiAction::TerminalBackToLive(session_id), cx);
                        }))
                        .child(if french {
                            "Retour au direct"
                        } else {
                            "Return to live"
                        }),
                ),
        );
    }

    let mut grid = div()
        .id(("vt-grid", salt))
        .relative()
        .flex_1()
        .min_h_0()
        .w_full()
        .px(px(8.0))
        .py(px(6.0))
        .overflow_hidden()
        .font_family(mono_font_family())
        .text_size(px(font_size))
        .line_height(px(row_height_px));

    for (row_idx, spans) in snap.lines.iter().enumerate() {
        grid = grid.child(render_terminal_row(
            row_idx as u16,
            spans,
            snap.cursor.filter(|_| focused_pane),
            cursor_style,
            row_height_px,
            font_size,
            tokens,
        ));
    }

    let mut out = wrapper.child(grid);
    if snap.exited
        && let Some(sid) = session_id
    {
        out = out.child(render_exit_overlay_pill(
            sid,
            false,
            snap.exit_code,
            french,
            tokens,
            cx,
        ));
    }
    out
}

fn render_exit_overlay_pill(
    sid: u64,
    failed_to_start: bool,
    exit_code: Option<i32>,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let (msg, primary_lbl, close_lbl) =
        crate::terminal::terminal_exit_banner_copy(failed_to_start, exit_code, french);
    div()
        .id(("exit-overlay-wrap", sid))
        .w_full()
        .px(px(12.0))
        .pb(px(10.0))
        .pt(px(4.0))
        .flex()
        .justify_center()
        .child(
            div()
                .id(("exit-overlay-pill", sid))
                .max_w(px(560.0))
                .w_full()
                .px(px(14.0))
                .py(px(8.0))
                .rounded(px(10.0))
                .bg(tokens.surface_2)
                .border_1()
                .border_color(tokens.border_strong)
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(10.0))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(120.0))
                        .text_size(px(12.0))
                        .text_color(tokens.text_strong)
                        .child(SharedString::from(msg)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .flex_shrink_0()
                        .ml_auto()
                        .child(
                            div()
                                .id(("btn-restart-exited", sid))
                                .px(px(10.0))
                                .py(px(4.0))
                                .rounded(px(tokens.radius_px))
                                .bg(tokens.accent)
                                .text_size(px(11.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(gpui::rgb(0xFF_FF_FF))
                                .cursor_pointer()
                                .on_click(cx.listener(move |root, _, _, cx| {
                                    root.dispatch(GpuiAction::RestartSession(sid), cx);
                                }))
                                .child(primary_lbl),
                        )
                        .child(
                            div()
                                .id(("btn-close-exited", sid))
                                .px(px(10.0))
                                .py(px(4.0))
                                .rounded(px(tokens.radius_px))
                                .bg(tokens.surface)
                                .border_1()
                                .border_color(tokens.border)
                                .text_size(px(11.5))
                                .text_color(tokens.text_strong)
                                .cursor_pointer()
                                .hover(|s| s.bg(tokens.surface_3))
                                .on_click(cx.listener(move |root, _, _, cx| {
                                    root.dispatch(GpuiAction::CloseSession(sid), cx);
                                }))
                                .child(close_lbl),
                        ),
                ),
        )
}

fn render_terminal_row(
    row_idx: u16,
    spans: &[TerminalStyledSpan],
    cursor: Option<(u16, u16)>,
    cursor_style: TerminalCursorStyle,
    row_height_px: f32,
    font_size: f32,
    tokens: &GpuiThemeTokens,
) -> impl IntoElement {
    let mut row_div = div()
        .relative()
        .h(px(row_height_px))
        .w_full()
        .flex()
        .items_center()
        .whitespace_nowrap();

    for span in spans {
        let mut s = div()
            .text_color(rgb_triplet(span.fg))
            .whitespace_nowrap()
            .child(SharedString::from(span.text.clone()));
        if span.selected {
            s = s.bg(rgba_with_alpha(tokens.accent, 0.36));
        } else if let Some(bg) = span.bg {
            s = s.bg(rgb_triplet(bg));
        }
        if span.bold {
            s = s.font_weight(FontWeight::BOLD);
        }
        if span.italic {
            s = s.italic();
        }
        if span.underline {
            s = s.underline();
        }
        row_div = row_div.child(s);
    }

    if let Some((c_row, c_col)) = cursor
        && c_row == row_idx
    {
        let cell_w = (font_size * 0.60).max(5.5);
        let x_left = f32::from(c_col) * cell_w;
        let cursor_el = match cursor_style {
            TerminalCursorStyle::Block => div()
                .absolute()
                .left(px(x_left))
                .top(px(1.0))
                .w(px(cell_w))
                .h(px((row_height_px - 2.0).max(8.0)))
                .bg(rgba_with_alpha(tokens.accent, 0.45))
                .border_1()
                .border_color(tokens.accent),
            TerminalCursorStyle::Bar => div()
                .absolute()
                .left(px(x_left))
                .top(px(1.0))
                .w(px(2.0))
                .h(px((row_height_px - 2.0).max(8.0)))
                .bg(tokens.accent),
            TerminalCursorStyle::Underline => div()
                .absolute()
                .left(px(x_left))
                .bottom(px(1.0))
                .w(px(cell_w))
                .h(px(2.0))
                .bg(tokens.accent),
        };
        row_div = row_div.child(cursor_el);
    }

    row_div
}

pub(crate) fn render_floating_terminal_drawer(
    floating: &GpuiFloatingTerminalSnapshot,
    inputs: &super::terminal_find_inputs::Inputs,
    stage_height_px: f32,
    font_size: f32,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let drawer_h = (stage_height_px * floating.height_ratio).clamp(140.0, stage_height_px * 0.85);
    let err_str = floating.error.as_deref().unwrap_or("");

    let body = if let Some(snap) = &floating.terminal_render {
        render_vt_surface(
            None,
            snap,
            inputs,
            floating.cursor_style,
            true,
            err_str,
            font_size,
            french,
            tokens,
            cx,
        )
        .into_any_element()
    } else {
        let msg = if let Some(e) = &floating.error {
            e.clone()
        } else if floating.starting {
            if french {
                "Démarrage du terminal flottant…".into()
            } else {
                "Starting floating terminal…".into()
            }
        } else if french {
            "Terminal flottant prêt.".into()
        } else {
            "Floating terminal ready.".into()
        };
        div()
            .flex_1()
            .p(px(12.0))
            .font_family(mono_font_family())
            .text_size(px(font_size))
            .text_color(tokens.text_dim)
            .child(SharedString::from(msg))
            .into_any_element()
    };

    div()
        .id("gpui-floating-terminal-drawer")
        .absolute()
        .left_0()
        .right_0()
        .bottom_0()
        .h(px(drawer_h))
        .flex()
        .flex_col()
        .bg(tokens.term)
        .border_t_1()
        .border_color(tokens.border_strong)
        .occlude()
        // Top 4px vertical drag handle
        .child(
            div()
                .id("resize-floating-terminal")
                .h(px(4.0))
                .w_full()
                .cursor_row_resize()
                .hover(|s| s.bg(tokens.accent))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |root, _: &MouseDownEvent, _, cx| {
                        root.active_drag = Some(DragTarget::FloatingTerminal { stage_height_px });
                        cx.notify();
                    }),
                ),
        )
        // Header bar
        .child(
            div()
                .h(px(28.0))
                .px(px(10.0))
                .flex()
                .items_center()
                .justify_between()
                .bg(tokens.surface_2)
                .border_b_1()
                .border_color(tokens.border)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(GpuiIcon::Terminal.render(12.5, tokens.accent))
                        .child(
                            div()
                                .text_size(px(11.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(tokens.text_strong)
                                .child(SharedString::from(floating.title.clone())),
                        )
                        .child(
                            div()
                                .px(px(5.0))
                                .rounded(px(4.0))
                                .bg(tokens.surface_3)
                                .text_size(px(10.0))
                                .text_color(tokens.text_dim)
                                .child("Ctrl+`"),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .child(
                            div()
                                .id("floating-restart")
                                .px(px(6.0))
                                .py(px(2.0))
                                .rounded(px(4.0))
                                .bg(tokens.surface_3)
                                .text_size(px(10.5))
                                .text_color(tokens.text)
                                .cursor_pointer()
                                .on_click(cx.listener(|root, _, _, cx| {
                                    root.dispatch(GpuiAction::RestartFloatingTerminal, cx);
                                }))
                                .child(if french { "Relancer" } else { "Restart" }),
                        )
                        .child(
                            div()
                                .id("floating-hide")
                                .px(px(6.0))
                                .py(px(2.0))
                                .rounded(px(4.0))
                                .bg(tokens.surface_3)
                                .text_size(px(10.5))
                                .text_color(tokens.text)
                                .cursor_pointer()
                                .on_click(cx.listener(|root, _, _, cx| {
                                    root.dispatch(GpuiAction::ToggleFloatingTerminal, cx);
                                }))
                                .child(if french { "Masquer" } else { "Hide" }),
                        )
                        .child(
                            div()
                                .id("floating-close")
                                .p(px(3.0))
                                .rounded(px(4.0))
                                .cursor_pointer()
                                .hover(|s| s.bg(tokens.surface_3))
                                .on_click(cx.listener(|root, _, _, cx| {
                                    root.dispatch(GpuiAction::CloseFloatingTerminal, cx);
                                }))
                                .child(GpuiIcon::Close.render(11.5, tokens.text_dim)),
                        ),
                ),
        )
        .child(body)
}
