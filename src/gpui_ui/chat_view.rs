//! 1:1 `GPUI` Native Codex & Claude chat views (`crate::chat::Chat` & `crate::claude::Chat`).
//!
//! Renders conversation turns, inline `Allow once` / `Deny` tool approvals, structured agent
//! question cards, model/effort selectors, `Switch to terminal`, and multi-line composer.

use super::{
    TesselGpuiRoot,
    icons::GpuiIcon,
    theme::{GpuiThemeTokens, mono_font_family, rgba_with_alpha},
};
use crate::{
    app::gpui_bridge::GpuiAction,
    chat::{NativeChatApprovalView, NativeChatViewSnapshot, QuestionView},
};
use gpui::{
    ClipboardItem, Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, div, prelude::FluentBuilder as _, px,
};

fn copy_message_button(
    key: (u64, usize),
    body: &str,
    copied: bool,
    french: bool,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let text = SharedString::from(body.to_owned());
    let copy = move |root: &mut TesselGpuiRoot, cx: &mut Context<TesselGpuiRoot>| {
        cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
        let marker = (key.0, key.1, std::time::Instant::now());
        root.chat_inputs.copied_message = Some(marker);
        cx.notify();
        cx.spawn(async move |root, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(1500))
                .await;
            let _ = root.update(cx, |root, cx| {
                if root.chat_inputs.copied_message == Some(marker) {
                    root.chat_inputs.copied_message = None;
                    cx.notify();
                }
            });
        })
        .detach();
    };
    let keyboard = copy.clone();
    let label = match (french, copied) {
        (true, true) => "Copié",
        (false, true) => "Copied",
        (true, false) => "Copier le message",
        (false, false) => "Copy message",
    };
    div()
        .id(("copy-message", key.1))
        .debug_selector(move || format!("copy-chat-message-{}", key.1))
        .focusable()
        .tab_stop(true)
        .role(gpui::Role::Button)
        .px(px(5.))
        .text_size(px(10.))
        .cursor_pointer()
        .on_click(cx.listener(move |root, _, _, cx| {
            cx.stop_propagation();
            copy(root, cx);
        }))
        .on_key_down(cx.listener(move |root, event: &gpui::KeyDownEvent, _, cx| {
            if !event.is_held && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                cx.stop_propagation();
                keyboard(root, cx);
            }
        }))
        .child(label)
}

fn scroll_reply_button(
    handle: gpui::ScrollHandle,
    index: usize,
    french: bool,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let keyboard = handle.clone();
    let label = if french {
        "Faire défiler ce message en haut"
    } else {
        "Scroll this message to top"
    };
    div()
        .id(("scroll-reply-top", index))
        .debug_selector(move || format!("scroll-reply-{index}"))
        .focusable()
        .tab_stop(true)
        .role(gpui::Role::Button)
        .aria_label(label)
        .self_end()
        .px(px(5.))
        .cursor_pointer()
        .on_click(cx.listener(move |_, _, _, cx| {
            cx.stop_propagation();
            handle.scroll_to_top_of_item(index);
            cx.notify();
        }))
        .on_key_down(cx.listener(move |_, event: &gpui::KeyDownEvent, _, cx| {
            if !event.is_held && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                cx.stop_propagation();
                keyboard.scroll_to_top_of_item(index);
                cx.notify();
            }
        }))
        .child("↑")
}

fn render_reply_prose(
    idx: usize,
    pane_id: u64,
    role: &str,
    body: &str,
    font_size: f32,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> gpui::AnyElement {
    let citations = crate::chat::citations::parse(role, body);
    div()
        .when(citations.is_some(), |el| {
            el.debug_selector(|| "chat-citation-body".into())
        })
        .text_size(px(font_size - 0.5))
        .line_height(px((font_size * 1.42).round()))
        .when(role.contains("Tool"), |el| {
            el.font_family(mono_font_family())
        })
        .text_color(tokens.text)
        .child(if let Some(cited) = citations {
            let (ranges, paths): (Vec<_>, Vec<_>) = cited.links.into_iter().unzip();
            let styled = gpui::StyledText::new(cited.text).with_highlights(
                ranges.iter().cloned().map(|range| {
                    (
                        range,
                        gpui::HighlightStyle {
                            color: Some(gpui::rgb_to_hsla(tokens.accent)),
                            ..Default::default()
                        },
                    )
                }),
            );
            let owner = cx.weak_entity();
            gpui::InteractiveText::new(("chat-citation", idx), styled)
                .on_click(ranges, move |index, _, cx| {
                    cx.stop_propagation();
                    let _ = owner.update(cx, |root, cx| {
                        root.dispatch(
                            GpuiAction::OpenChatCitation(pane_id, paths[index].clone()),
                            cx,
                        )
                    });
                })
                .into_any_element()
        } else {
            SharedString::from(body.to_owned()).into_any_element()
        })
        .into_any_element()
}

fn render_code_block(
    idx: usize,
    language: String,
    text: String,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> gpui::AnyElement {
    let copy = text.clone();
    let bar = div()
        .flex()
        .items_center()
        .justify_between()
        .child(SharedString::from(language))
        .when(!copy.is_empty(), |bar| {
            bar.child(
                div()
                    .id(("copy-reply-code", idx))
                    .debug_selector(|| "copy-reply-code".into())
                    .px(px(6.))
                    .cursor_pointer()
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.stop_propagation();
                        cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                    }))
                    .child(if french {
                        "Copier le code"
                    } else {
                        "Copy code"
                    }),
            )
        });
    div()
        .min_w_0()
        .p(px(8.))
        .bg(tokens.chrome)
        .rounded(px(6.))
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(bar)
        .child(
            div()
                .font_family(mono_font_family())
                .child(SharedString::from(text)),
        )
        .into_any_element()
}

pub(crate) fn render_native_chat_pane_body(
    pane_id: u64,
    chat: &NativeChatViewSnapshot,
    chat_inputs: &super::chat_inputs::Inputs,
    font_size: f32,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let composer_id = chat.composer_id;
    let provider_title = if chat.provider == "claude" {
        if french {
            "Claude · chat natif"
        } else {
            "Claude · native chat"
        }
    } else if french {
        "Codex · chat natif"
    } else {
        "Codex · native chat"
    };

    let efforts: &[&str] = if chat.provider == "claude" {
        &["", "low", "medium", "high", "xhigh", "max"]
    } else {
        &["", "low", "medium", "high", "xhigh"]
    };

    let mut effort_pills = div().flex().items_center().gap(px(3.0));
    for (idx, eff) in efforts.iter().copied().enumerate() {
        let selected = chat.effort == eff;
        let label = if eff.is_empty() {
            if french { "Auto" } else { "Default" }
        } else {
            eff
        };
        let eff_owned = eff.to_owned();
        effort_pills = effort_pills.child(
            div()
                .id(("chat-effort", (pane_id as usize) * 16 + idx))
                .px(px(6.0))
                .py(px(1.5))
                .rounded(px(4.0))
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
                .text_size(px(10.0))
                .text_color(if selected {
                    tokens.accent
                } else {
                    tokens.text_dim
                })
                .cursor_pointer()
                .on_click(cx.listener(move |root, _, _, cx| {
                    root.dispatch(
                        GpuiAction::ChatSetEffort {
                            pane_id,
                            effort: eff_owned.clone(),
                        },
                        cx,
                    );
                }))
                .child(label),
        );
    }

    let mut transcript_list = div()
        .id(("chat-transcript", pane_id))
        .debug_selector(|| "chat-transcript".into())
        .when_some(chat_inputs.transcript(pane_id), |list, handle| {
            list.track_scroll(handle)
        })
        .flex_1()
        .min_h_0()
        .w_full()
        .p(px(10.0))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(8.0));

    if chat.can_load_older {
        transcript_list = transcript_list.child(
            div()
                .id(("chat-load-older", pane_id))
                .self_center()
                .px(px(10.0))
                .py(px(3.5))
                .rounded(px(tokens.radius_px))
                .bg(tokens.surface_2)
                .border_1()
                .border_color(tokens.border)
                .text_size(px(11.0))
                .text_color(tokens.text)
                .cursor_pointer()
                .on_click(cx.listener(move |root, _, _, cx| {
                    root.dispatch(GpuiAction::ChatLoadOlder(pane_id), cx);
                }))
                .child(if french {
                    "Charger les messages précédents"
                } else {
                    "Load older messages"
                }),
        );
    }

    if let Some(approval) = &chat.approval {
        transcript_list =
            transcript_list.child(render_approval_card(pane_id, approval, french, tokens, cx));
    }

    if !chat.questions.is_empty() {
        transcript_list = transcript_list.child(render_questions_card(
            pane_id,
            &chat.questions,
            &chat_inputs.questions,
            chat.questions_can_submit,
            french,
            tokens,
            cx,
        ));
    }

    if chat.messages.is_empty() && chat.approval.is_none() && chat.questions.is_empty() {
        let hint = if !chat.connected {
            if french {
                "Cliquez sur Connecter pour démarrer la session native sécurisée."
            } else {
                "Click Connect to start the sandboxed native chat session."
            }
        } else if french {
            "Prêt — saisissez votre message ci-dessous (Entrée pour envoyer)."
        } else {
            "Ready — type a message below (Enter to send, Shift+Enter for newline)."
        };
        transcript_list = transcript_list.child(
            div()
                .p(px(12.0))
                .rounded(px(tokens.radius_px))
                .bg(tokens.surface)
                .border_1()
                .border_color(tokens.border)
                .text_size(px(12.0))
                .text_color(tokens.text_dim)
                .child(hint),
        );
    } else {
        let first_message = usize::from(chat.can_load_older)
            + usize::from(chat.approval.is_some())
            + usize::from(!chat.questions.is_empty());
        for (msg_idx, (role, body)) in chat.messages.iter().enumerate() {
            let is_user = role == "You";
            let is_tool = role.contains("Tool");
            let bubble_bg = if is_user {
                tokens.accent_soft_bg
            } else if is_tool {
                tokens.chrome
            } else {
                tokens.surface
            };
            let bubble_border = if is_user {
                tokens.accent
            } else {
                tokens.border
            };
            transcript_list = transcript_list.child(
                div()
                    .id(("chat-msg", (pane_id as usize) * 2048 + msg_idx))
                    .debug_selector(move || format!("chat-row-{msg_idx}"))
                    .flex_none()
                    .w_full()
                    .p(px(9.0))
                    .rounded(px(tokens.radius_px))
                    .bg(bubble_bg)
                    .border_1()
                    .border_color(bubble_border)
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
                                    .text_size(px(10.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(if is_user {
                                        tokens.accent
                                    } else {
                                        tokens.text_strong
                                    })
                                    .child(SharedString::from(role.clone())),
                            )
                            .when(crate::chat::citations::copyable(role, body), |header| {
                                let key = (composer_id, msg_idx);
                                let copied = chat_inputs
                                    .copied_message
                                    .is_some_and(|(id, idx, _)| (id, idx) == key);
                                header.child(copy_message_button(key, body, copied, french, cx))
                            }),
                    )
                    .children(
                        crate::chat::blocks::message_blocks(role, body)
                            .into_iter()
                            .enumerate()
                            .map(|(index, block)| match block {
                                crate::chat::blocks::Block::Prose(text) => render_reply_prose(
                                    index, pane_id, role, text, font_size, tokens, cx,
                                ),
                                crate::chat::blocks::Block::Code { language, text } => {
                                    render_code_block(index, language, text, french, tokens, cx)
                                }
                            }),
                    )
                    .when(
                        crate::chat::citations::assistant_role(role) && !body.is_empty(),
                        |row| {
                            row.when_some(chat_inputs.transcript(pane_id), |row, handle| {
                                row.child(scroll_reply_button(
                                    handle.clone(),
                                    first_message + msg_idx,
                                    french,
                                    cx,
                                ))
                            })
                        },
                    ),
            );
        }
    }

    let composer_placeholder = if french {
        "Écrire un message à l'agent… (Entrée = envoyer, Maj+Entrée = nouvelle ligne)"
    } else {
        "Ask the agent… (Enter to send, Shift+Enter for newline)"
    };

    div()
        .id(("native-chat-root", pane_id))
        .flex_1()
        .min_h_0()
        .w_full()
        .flex()
        .flex_col()
        .overflow_hidden()
        .bg(tokens.term)
        // Top control & model bar
        .child(
            div()
                .px(px(10.0))
                .py(px(6.0))
                .bg(tokens.surface)
                .border_b_1()
                .border_color(tokens.border)
                .flex()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap(px(6.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(GpuiIcon::Sparkles.render(13.0, tokens.accent))
                        .child(
                            div()
                                .text_size(px(11.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(tokens.text_strong)
                                .child(provider_title),
                        )
                        .child(
                            div()
                                .px(px(6.0))
                                .py(px(1.5))
                                .rounded(px(4.0))
                                .bg(if chat.ready {
                                    rgba_with_alpha(tokens.ok_green, 0.16)
                                } else {
                                    tokens.surface_2
                                })
                                .text_size(px(10.0))
                                .text_color(if chat.ready {
                                    tokens.ok_green
                                } else {
                                    tokens.text_dim
                                })
                                .child(SharedString::from(chat.status.clone())),
                        )
                        .when_some(chat.thread_id.as_ref(), |el, tid| {
                            let tid_copy = tid.clone();
                            let short: String = tid.chars().take(12).collect();
                            el.child(
                                div()
                                    .id(("chat-copy-tid", pane_id))
                                    .px(px(5.0))
                                    .py(px(1.5))
                                    .rounded(px(4.0))
                                    .bg(tokens.surface_2)
                                    .font_family(mono_font_family())
                                    .text_size(px(10.0))
                                    .text_color(tokens.text_dim)
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            tid_copy.clone(),
                                        ));
                                    }))
                                    .child(SharedString::from(format!("{short}…"))),
                            )
                        }),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.0))
                        .child(effort_pills)
                        .when(!chat.connected, |el| {
                            el.child(
                                div()
                                    .id(("chat-connect", pane_id))
                                    .px(px(9.0))
                                    .py(px(3.0))
                                    .rounded(px(tokens.radius_px))
                                    .bg(tokens.accent)
                                    .text_size(px(11.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(gpui::rgb(0xFF_FF_FF))
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |root, _, _, cx| {
                                        root.dispatch(GpuiAction::ChatConnect(pane_id), cx);
                                    }))
                                    .child(if french { "Connecter" } else { "Connect" }),
                            )
                        })
                        .when(chat.connected, |el| {
                            el.child(
                                div()
                                    .id(("chat-stop", pane_id))
                                    .px(px(8.0))
                                    .py(px(3.0))
                                    .rounded(px(tokens.radius_px))
                                    .bg(tokens.surface_2)
                                    .border_1()
                                    .border_color(tokens.border)
                                    .text_size(px(11.0))
                                    .text_color(tokens.text)
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |root, _, _, cx| {
                                        root.dispatch(GpuiAction::ChatStop(pane_id), cx);
                                    }))
                                    .child(if french { "Arrêter" } else { "Stop" }),
                            )
                        })
                        .when(chat.working, |el| {
                            el.child(
                                div()
                                    .id(("chat-interrupt", pane_id))
                                    .px(px(8.0))
                                    .py(px(3.0))
                                    .rounded(px(tokens.radius_px))
                                    .bg(rgba_with_alpha(tokens.warn, 0.2))
                                    .border_1()
                                    .border_color(tokens.warn)
                                    .text_size(px(11.0))
                                    .text_color(tokens.warn)
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |root, _, _, cx| {
                                        root.dispatch(GpuiAction::ChatInterrupt(pane_id), cx);
                                    }))
                                    .child(if french { "Interrompre" } else { "Interrupt" }),
                            )
                        })
                        .child(
                            div()
                                .id(("chat-switch-term", pane_id))
                                .px(px(8.0))
                                .py(px(3.0))
                                .rounded(px(tokens.radius_px))
                                .bg(tokens.surface_2)
                                .border_1()
                                .border_color(tokens.border)
                                .text_size(px(11.0))
                                .text_color(tokens.text_dim)
                                .cursor_pointer()
                                .hover(|s| s.text_color(tokens.text_strong))
                                .on_click(cx.listener(move |root, _, _, cx| {
                                    root.dispatch(GpuiAction::SwitchSessionChatMode(pane_id), cx);
                                }))
                                .child(if french {
                                    "Terminal CLI"
                                } else {
                                    "Switch to CLI"
                                }),
                        ),
                ),
        )
        .when(!chat.error.is_empty(), |el| {
            el.child(
                div()
                    .px(px(10.0))
                    .py(px(4.0))
                    .bg(rgba_with_alpha(tokens.danger, 0.16))
                    .border_b_1()
                    .border_color(tokens.danger)
                    .text_size(px(11.0))
                    .text_color(tokens.text_strong)
                    .child(SharedString::from(chat.error.clone())),
            )
        })
        .child(transcript_list)
        // Bottom Composer
        .child(
            div()
                .p(px(8.0))
                .bg(tokens.surface)
                .border_t_1()
                .border_color(tokens.border)
                .flex()
                .items_end()
                .gap(px(8.0))
                .child(
                    div()
                        .id(("chat-composer-box", pane_id))
                        .flex_1()
                        .min_w_0()
                        .rounded(px(tokens.radius_px))
                        .bg(tokens.term)
                        .border_1()
                        .border_color(tokens.border_strong)
                        .child(chat_inputs.render(
                            pane_id,
                            composer_placeholder,
                            font_size,
                            french,
                            tokens,
                            cx,
                        )),
                )
                .child(
                    div()
                        .id(("chat-send-btn", pane_id))
                        .px(px(12.0))
                        .py(px(8.0))
                        .rounded(px(tokens.radius_px))
                        .bg(if chat.ready && !chat.composer.trim().is_empty() {
                            tokens.accent
                        } else {
                            tokens.surface_3
                        })
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if chat.ready && !chat.composer.trim().is_empty() {
                            gpui::rgb(0xFF_FF_FF)
                        } else {
                            tokens.text_dim
                        })
                        .cursor_pointer()
                        .on_click(cx.listener(move |root, _, window, cx| {
                            super::chat_inputs::submit(root, pane_id, composer_id, window, cx);
                        }))
                        .child(if french { "Envoyer" } else { "Send" }),
                ),
        )
}

fn render_approval_card(
    pane_id: u64,
    approval: &NativeChatApprovalView,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let can_accept = approval.can_accept;
    let request_instance_id = approval.instance_id;
    div()
        .p(px(10.0))
        .rounded(px(tokens.radius_px))
        .bg(tokens.surface_2)
        .border_1()
        .border_color(tokens.warn)
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
                        .text_color(tokens.warn)
                        .child(if french {
                            "Approbation requise — aucune action sans votre accord"
                        } else {
                            "Approval requested — no action until you choose"
                        }),
                )
                .child(
                    div()
                        .font_family(mono_font_family())
                        .text_size(px(10.5))
                        .text_color(tokens.text_dim)
                        .child(SharedString::from(approval.title.clone())),
                ),
        )
        .child(
            div()
                .id(("chat-approval-json", pane_id))
                .max_h(px(150.0))
                .overflow_y_scroll()
                .p(px(8.0))
                .rounded(px(4.0))
                .bg(tokens.term)
                .font_family(mono_font_family())
                .text_size(px(11.0))
                .text_color(tokens.text)
                .child(SharedString::from(approval.details_json.clone())),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .id(("chat-approve-allow", pane_id))
                        .px(px(10.0))
                        .py(px(4.0))
                        .rounded(px(tokens.radius_px))
                        .bg(if can_accept {
                            tokens.ok_green
                        } else {
                            tokens.surface_3
                        })
                        .text_size(px(11.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(gpui::rgb(0xFF_FF_FF))
                        .cursor_pointer()
                        .on_click(cx.listener(move |root, _, _, cx| {
                            if can_accept {
                                root.dispatch(
                                    GpuiAction::ChatRespondApproval {
                                        pane_id,
                                        request_instance_id,
                                        allow: true,
                                    },
                                    cx,
                                );
                            }
                        }))
                        .child(if french {
                            "Autoriser une fois"
                        } else {
                            "Allow once"
                        }),
                )
                .child(
                    div()
                        .id(("chat-approve-deny", pane_id))
                        .px(px(10.0))
                        .py(px(4.0))
                        .rounded(px(tokens.radius_px))
                        .bg(rgba_with_alpha(tokens.danger, 0.22))
                        .border_1()
                        .border_color(tokens.danger)
                        .text_size(px(11.5))
                        .text_color(tokens.danger)
                        .cursor_pointer()
                        .on_click(cx.listener(move |root, _, _, cx| {
                            root.dispatch(
                                GpuiAction::ChatRespondApproval {
                                    pane_id,
                                    request_instance_id,
                                    allow: false,
                                },
                                cx,
                            );
                        }))
                        .child(if french { "Refuser" } else { "Deny" }),
                ),
        )
}

fn render_questions_card(
    pane_id: u64,
    questions: &[QuestionView],
    inputs: &super::question_inputs::Inputs,
    can_submit: bool,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let request_instance_id = questions
        .first()
        .map(|q| q.request_instance_id)
        .unwrap_or(0);
    let mut card = div()
        .p(px(10.0))
        .rounded(px(tokens.radius_px))
        .bg(tokens.surface_2)
        .border_1()
        .border_color(tokens.accent)
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            div()
                .text_size(px(12.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(tokens.accent)
                .child(if french {
                    "Questions de l'agent"
                } else {
                    "Questions from the agent"
                }),
        );

    for (q_idx, q) in questions.iter().enumerate() {
        let mut q_box = div()
            .p(px(8.0))
            .rounded(px(4.0))
            .bg(tokens.surface)
            .border_1()
            .border_color(tokens.border)
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .text_size(px(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(tokens.text_strong)
                    .child(SharedString::from(q.header.clone())),
            )
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(tokens.text)
                    .child(SharedString::from(q.question.clone())),
            );

        for (opt_idx, (label, desc, selected)) in q.options.iter().enumerate() {
            let id_num = (pane_id as usize) * 256 + q_idx * 16 + opt_idx;
            let prefix = if q.multi_select {
                if *selected { "[✓] " } else { "[ ] " }
            } else if *selected {
                "(●) "
            } else {
                "( ) "
            };
            q_box = q_box.child(
                div()
                    .id(("chat-q-opt", id_num))
                    .px(px(7.0))
                    .py(px(3.5))
                    .rounded(px(4.0))
                    .bg(if *selected {
                        tokens.accent_soft_bg
                    } else {
                        tokens.surface_2
                    })
                    .border_1()
                    .border_color(if *selected {
                        tokens.accent
                    } else {
                        tokens.border
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(move |root, _, _, cx| {
                        root.dispatch(
                            GpuiAction::ChatSelectQuestionOption {
                                pane_id,
                                request_instance_id,
                                q_idx,
                                opt_idx,
                            },
                            cx,
                        );
                    }))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(tokens.text_strong)
                            .child(SharedString::from(format!("{prefix}{label}"))),
                    )
                    .when(!desc.is_empty(), |el| {
                        el.child(
                            div()
                                .text_size(px(10.5))
                                .text_color(tokens.text_dim)
                                .child(SharedString::from(desc.clone())),
                        )
                    }),
            );
        }

        if q.options.is_empty() || q.is_other {
            q_box = q_box.child(
                div()
                    .rounded(px(4.))
                    .bg(tokens.term)
                    .border_1()
                    .border_color(tokens.border_strong)
                    .child(inputs.render(pane_id, q_idx, french, tokens, cx)),
            );
        }

        card = card.child(q_box);
    }

    card.child(
        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(
                div()
                    .id(("chat-q-submit", pane_id))
                    .px(px(10.0))
                    .py(px(4.0))
                    .rounded(px(tokens.radius_px))
                    .bg(if can_submit {
                        tokens.accent
                    } else {
                        tokens.surface_3
                    })
                    .text_size(px(11.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if can_submit {
                        gpui::rgb(0xFF_FF_FF)
                    } else {
                        tokens.text_dim
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(move |root, _, window, cx| {
                        super::question_inputs::submit(
                            root,
                            pane_id,
                            request_instance_id,
                            false,
                            window,
                            cx,
                        );
                    }))
                    .child(if french {
                        "Envoyer les réponses"
                    } else {
                        "Send answers"
                    }),
            )
            .child(
                div()
                    .id(("chat-q-skip", pane_id))
                    .px(px(10.0))
                    .py(px(4.0))
                    .rounded(px(tokens.radius_px))
                    .bg(tokens.surface_3)
                    .text_size(px(11.5))
                    .text_color(tokens.text)
                    .cursor_pointer()
                    .on_click(cx.listener(move |root, _, window, cx| {
                        super::question_inputs::submit(
                            root,
                            pane_id,
                            request_instance_id,
                            true,
                            window,
                            cx,
                        );
                    }))
                    .child(if french { "Ignorer" } else { "Skip" }),
            ),
    )
}
