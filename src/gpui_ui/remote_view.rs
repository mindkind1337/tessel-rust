//! Native controls over the same SFTP state used by the compatibility renderer.
use super::{FocusInputTarget, TesselGpuiRoot, theme::GpuiThemeTokens};
use crate::{
    app::gpui_bridge::{GpuiAction, GpuiWorkspaceSnapshot},
    remote_panel::control::Command,
};
use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, img, prelude::FluentBuilder as _, px, uniform_list,
};

fn button(
    id: &'static str,
    label: &'static str,
    command: Command,
    enabled: bool,
    tokens: &GpuiThemeTokens,
    cx: &Context<TesselGpuiRoot>,
) -> impl IntoElement {
    div()
        .id(id)
        .px(px(9.))
        .py(px(5.))
        .rounded(px(4.))
        .bg(tokens.surface_2)
        .opacity(if enabled { 1. } else { 0.45 })
        .when(enabled, |el| {
            el.cursor_pointer()
                .on_click(cx.listener(move |root, _, _, cx| {
                    root.dispatch(GpuiAction::Remote(command.clone()), cx)
                }))
        })
        .child(label)
}

fn field(
    id: &'static str,
    inputs: &super::remote_inputs::Inputs,
    text: String,
    target: (FocusInputTarget, bool),
    enabled: bool,
    tokens: &GpuiThemeTokens,
    cx: &Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let (target, focused) = target;
    if let Some(state) = inputs.get(target) {
        return gpui_ce_elements::editable_text::editable_text(id)
            .state(state.downgrade())
            .multiline(target == FocusInputTarget::RemoteText)
            .accepts_input(enabled)
            .caret_blink_interval_500ms()
            .caret_color(gpui::rgb_to_hsla(tokens.text))
            .selection_color(gpui::rgb_to_hsla(tokens.accent_soft_bg))
            .min_w(px(180.))
            .w_full()
            .px(px(8.))
            .py(px(5.))
            .border_1()
            .border_color(tokens.border)
            .bg(tokens.surface)
            .into_any_element();
    }
    div()
        .id(id)
        .min_w(px(180.))
        .px(px(8.))
        .py(px(5.))
        .border_1()
        .border_color(if focused {
            tokens.accent
        } else {
            tokens.border
        })
        .bg(tokens.surface)
        .when(enabled, |el| {
            el.on_click(
                cx.listener(move |root, _, window, cx| root.set_focus_input(target, window, cx)),
            )
        })
        .child(SharedString::from(if text.is_empty() {
            " ".into()
        } else {
            text
        }))
        .into_any_element()
}

pub(crate) fn render(
    snapshot: &GpuiWorkspaceSnapshot,
    inputs: &super::remote_inputs::Inputs,
    focus_input: FocusInputTarget,
    tokens: &GpuiThemeTokens,
    cx: &Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let remote = &snapshot.remote;
    let fr = snapshot.french;
    let free = !remote.busy && !remote.dirty && remote.rename.is_none() && remote.delete.is_none();
    let connected = remote.connected.is_some();
    let mut body = div()
        .id("remote-files")
        .size_full()
        .flex()
        .flex_col()
        .p(px(12.))
        .gap(px(8.))
        .text_color(tokens.text)
        .text_size(px(13.));
    body = body.child(if fr {
        "Fichiers distants · SFTP"
    } else {
        "Remote files · SFTP"
    });
    let mut hosts = div().flex().flex_wrap().gap(px(6.));
    for (index, alias) in remote.hosts.iter().enumerate() {
        hosts = hosts.child(
            div()
                .id(("remote-host", index))
                .px(px(9.))
                .py(px(5.))
                .bg(if remote.selected == index {
                    tokens.accent_soft_bg
                } else {
                    tokens.surface_2
                })
                .when(free, |el| {
                    el.cursor_pointer()
                        .on_click(cx.listener(move |root, _, _, cx| {
                            root.dispatch(GpuiAction::Remote(Command::Connect(index)), cx)
                        }))
                })
                .child(SharedString::from(alias.clone())),
        );
    }
    if remote.hosts.is_empty() {
        hosts = hosts.child(if fr {
            "Ajoutez un hôte SSH dans les paramètres."
        } else {
            "Add an SSH host in Settings first."
        });
    }
    body = body.child(hosts);
    if let Some(host) = &remote.connected {
        body = body.child(SharedString::from(format!("{host}:{}", remote.path)));
        if remote.live {
            body = body.child(if fr {
                "Mises à jour des fichiers en direct"
            } else {
                "Live file updates"
            });
        }
    }
    body = body.child(
        div()
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .child(button(
                "remote-refresh",
                if fr { "Actualiser" } else { "Refresh" },
                Command::Refresh,
                free && connected,
                tokens,
                cx,
            ))
            .child(button(
                "remote-parent",
                if fr {
                    "Dossier parent"
                } else {
                    "Parent folder"
                },
                Command::Parent,
                free && remote.can_parent,
                tokens,
                cx,
            ))
            .child(button(
                "remote-disconnect",
                if fr { "Déconnecter" } else { "Disconnect" },
                Command::Disconnect,
                free && connected,
                tokens,
                cx,
            )),
    );
    if remote.busy {
        body = body.child(if fr {
            "Requête SFTP en cours…"
        } else {
            "SFTP request in progress…"
        });
    }
    if !remote.error.is_empty() {
        body = body.child(SharedString::from(remote.error.clone()));
    }
    if remote.dirty {
        body = body.child(if fr {
            "Enregistrez ou annulez les modifications avant de changer de fichier."
        } else {
            "Save or discard edits before changing files."
        });
    }
    if connected {
        body = body.child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(6.))
                .child(field(
                    "remote-name",
                    inputs,
                    remote.new_name.clone(),
                    (
                        FocusInputTarget::RemoteName,
                        focus_input == FocusInputTarget::RemoteName,
                    ),
                    free,
                    tokens,
                    cx,
                ))
                .child(button(
                    "remote-new-file",
                    if fr {
                        "Créer un fichier"
                    } else {
                        "Create file"
                    },
                    Command::Create(false),
                    free && !remote.new_name.is_empty(),
                    tokens,
                    cx,
                ))
                .child(button(
                    "remote-new-dir",
                    if fr {
                        "Créer un dossier"
                    } else {
                        "Create folder"
                    },
                    Command::Create(true),
                    free && !remote.new_name.is_empty(),
                    tokens,
                    cx,
                )),
        );
    }
    let entries = remote.entries.clone();
    let palette = *tokens;
    body = body.child(
        uniform_list(
            "remote-file-list",
            entries.len(),
            cx.processor(move |_root, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|index| {
                        let (path, name, directory, size) = &entries[index];
                        let open = path.clone();
                        let rename = path.clone();
                        let delete = path.clone();
                        div()
                            .id(("remote-row", index))
                            .h(px(30.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .id(("remote-open", index))
                                    .flex_1()
                                    .px(px(5.))
                                    .when(free, |el| {
                                        el.cursor_pointer().on_click(cx.listener(
                                            move |root, _, _, cx| {
                                                root.dispatch(
                                                    GpuiAction::Remote(Command::Open(open.clone())),
                                                    cx,
                                                )
                                            },
                                        ))
                                    })
                                    .child(SharedString::from(if *directory {
                                        format!("▸ {name}")
                                    } else {
                                        format!("{name} · {size} B")
                                    })),
                            )
                            .child(
                                div()
                                    .id(("remote-rename", index))
                                    .px(px(5.))
                                    .bg(palette.surface_2)
                                    .when(free, |el| {
                                        el.cursor_pointer().on_click(cx.listener(
                                            move |root, _, _, cx| {
                                                root.dispatch(
                                                    GpuiAction::Remote(Command::BeginRename(
                                                        rename.clone(),
                                                    )),
                                                    cx,
                                                )
                                            },
                                        ))
                                    })
                                    .child(if fr { "Renommer…" } else { "Rename…" }),
                            )
                            .child(
                                div()
                                    .id(("remote-delete", index))
                                    .px(px(5.))
                                    .bg(palette.surface_2)
                                    .when(free, |el| {
                                        el.cursor_pointer().on_click(cx.listener(
                                            move |root, _, _, cx| {
                                                root.dispatch(
                                                    GpuiAction::Remote(Command::BeginDelete(
                                                        delete.clone(),
                                                    )),
                                                    cx,
                                                )
                                            },
                                        ))
                                    })
                                    .child(if fr { "Supprimer…" } else { "Delete…" }),
                            )
                    })
                    .collect()
            }),
        )
        .h(px(180.))
        .w_full(),
    );
    if let Some((path, name)) = &remote.rename {
        body = body.child(SharedString::from(path.clone())).child(
            div()
                .flex()
                .gap(px(6.))
                .child(field(
                    "remote-rename-draft",
                    inputs,
                    name.clone(),
                    (
                        FocusInputTarget::RemoteRename,
                        focus_input == FocusInputTarget::RemoteRename,
                    ),
                    !remote.busy,
                    tokens,
                    cx,
                ))
                .child(button(
                    "remote-rename-confirm",
                    if fr { "Renommer" } else { "Rename" },
                    Command::ConfirmRename,
                    !remote.busy && !name.is_empty(),
                    tokens,
                    cx,
                ))
                .child(button(
                    "remote-rename-cancel",
                    if fr { "Annuler" } else { "Cancel" },
                    Command::CancelDialog,
                    true,
                    tokens,
                    cx,
                )),
        );
    }
    if let Some(path) = &remote.delete {
        body = body
            .child(SharedString::from(format!(
                "{} {path}",
                if fr {
                    "Supprimer définitivement ?"
                } else {
                    "Delete permanently?"
                }
            )))
            .child(if fr {
                "Aucune corbeille. Seuls les dossiers vides peuvent être supprimés."
            } else {
                "No recycle bin. Only empty folders can be deleted."
            })
            .child(
                div()
                    .flex()
                    .gap(px(6.))
                    .child(button(
                        "remote-delete-confirm",
                        if fr {
                            "Supprimer définitivement"
                        } else {
                            "Delete permanently"
                        },
                        Command::ConfirmDelete,
                        !remote.busy,
                        tokens,
                        cx,
                    ))
                    .child(button(
                        "remote-delete-cancel",
                        if fr { "Annuler" } else { "Cancel" },
                        Command::CancelDialog,
                        true,
                        tokens,
                        cx,
                    )),
            );
    }
    if let Some((path, text, size)) = &remote.document {
        if !remote.versions.is_empty() {
            let mut list = div()
                .id("remote-local-versions")
                .max_h(px(95.))
                .overflow_y_scroll()
                .child(if fr {
                    "Historique local"
                } else {
                    "Local History"
                });
            for &timestamp in &remote.versions {
                list = list
                    .child(div().child(
                        crate::regional_date::long_local(timestamp, fr).unwrap_or_default(),
                    ));
            }
            body = body.child(list);
        }
        body = body.child(SharedString::from(path.clone())).child(
            div()
                .flex()
                .gap(px(6.))
                .child(button(
                    "remote-save",
                    if fr {
                        "Enregistrer sur l’hôte"
                    } else {
                        "Save to remote host"
                    },
                    Command::Save,
                    !remote.busy && remote.dirty,
                    tokens,
                    cx,
                ))
                .child(button(
                    "remote-discard",
                    if fr {
                        "Annuler les modifications"
                    } else {
                        "Discard edits"
                    },
                    Command::Discard,
                    !remote.busy && remote.dirty,
                    tokens,
                    cx,
                ))
                .child(button(
                    "remote-download",
                    if fr {
                        "Télécharger l’original…"
                    } else {
                        "Download original…"
                    },
                    Command::Download,
                    !remote.busy,
                    tokens,
                    cx,
                )),
        );
        if let Some(text) = text {
            body = body.child(
                div()
                    .id("remote-text-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(field(
                        "remote-text",
                        inputs,
                        text.clone(),
                        (
                            FocusInputTarget::RemoteText,
                            focus_input == FocusInputTarget::RemoteText,
                        ),
                        !remote.busy,
                        tokens,
                        cx,
                    )),
            );
        } else if let Some(office) = &remote.office {
            body = body.child(super::editor_view::render_office_summary(
                office, fr, tokens, cx, true,
            ));
        } else if remote.pdf.is_some() || remote.pixels.is_some() {
            if let Some((page, pages, scale, busy, _, error)) = &remote.pdf {
                body = body.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(button(
                            "remote-pdf-prev",
                            if fr { "Précédent" } else { "Previous" },
                            Command::PdfPage(-1),
                            !busy && *page > 0,
                            tokens,
                            cx,
                        ))
                        .when(*pages > 0, |el| {
                            el.child(SharedString::from(format!("Page {} / {}", page + 1, pages)))
                        })
                        .child(button(
                            "remote-pdf-next",
                            if fr { "Suivant" } else { "Next" },
                            Command::PdfPage(1),
                            !busy && page + 1 < *pages,
                            tokens,
                            cx,
                        ))
                        .child(button(
                            "remote-pdf-minus",
                            "−",
                            Command::PdfZoom(-0.25),
                            !busy && *scale > 0.5,
                            tokens,
                            cx,
                        ))
                        .child(SharedString::from(format!("{:.0}%", scale * 100.)))
                        .child(button(
                            "remote-pdf-plus",
                            "+",
                            Command::PdfZoom(0.25),
                            !busy && *scale < 3.,
                            tokens,
                            cx,
                        )),
                );
                if *busy {
                    body = body.child(if fr {
                        "Chargement de la page…"
                    } else {
                        "Loading page…"
                    });
                }
                if !error.is_empty() {
                    body = body.child(SharedString::from(error.clone()));
                }
            }
            if let Some(image) = &remote.pixels {
                let size = image.size(0);
                body = body.child(
                    div()
                        .id("remote-media-scroll")
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .overflow_scroll()
                        .child(
                            img(image.clone())
                                .w(px(size.width.0 as f32))
                                .h(px(size.height.0 as f32)),
                        ),
                );
            }
        } else {
            body = body.child(SharedString::from(format!(
                "{} · {size} B",
                if fr {
                    "Fichier binaire — télécharger l’original pour l’ouvrir"
                } else {
                    "Binary file — download the original to open it"
                }
            )));
        }
        if !remote.preview_error.is_empty() {
            body = body.child(SharedString::from(remote.preview_error.clone()));
        }
    }
    body
}
