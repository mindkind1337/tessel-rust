//! Native controls over the same SFTP state used by the compatibility renderer.
use super::{FocusInputTarget, TesselGpuiRoot, theme::GpuiThemeTokens};
use crate::{
    app::gpui_bridge::{GpuiAction, GpuiWorkspaceSnapshot},
    remote_panel::control::Command,
};
use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder as _, px, uniform_list,
};

fn button(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    command: Command,
    enabled: bool,
    tokens: &GpuiThemeTokens,
    cx: &Context<TesselGpuiRoot>,
) -> impl IntoElement {
    div()
        .id(id.into())
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
        .child(label.into())
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
    editor_input: &super::editor_input::Input,
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
                "remote-terminal-here",
                if fr {
                    "Ouvrir un terminal ici"
                } else {
                    "Open a terminal here"
                },
                Command::TerminalHere {
                    host: remote.connected.clone().unwrap_or_default(),
                    path: remote.path.clone(),
                },
                free && connected && !remote.path.is_empty(),
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
    if let Some(message) = &remote.folder_status {
        body = body.child(message.clone());
    }
    if let Some(path) = &remote.downloaded {
        body = body.child(format!(
            "{} {}",
            if fr {
                "Fichier téléchargé."
            } else {
                "File downloaded."
            },
            path.display()
        ));
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
    let connected_alias = remote.connected.clone().unwrap_or_default();
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
                                    .min_w_0()
                                    .flex()
                                    .items_center()
                                    .gap(px(5.))
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
                                    .when_else(
                                        *directory,
                                        |el| {
                                            el.child(
                                                super::icons::GpuiIcon::ChevronRight
                                                    .render(12., palette.text_dim),
                                            )
                                        },
                                        |el| {
                                            el.child(
                                                super::file_icons::render(
                                                    name,
                                                    &palette,
                                                    palette.text_dim,
                                                )
                                                .debug_selector(move || {
                                                    format!("remote-file-icon-{index}")
                                                }),
                                            )
                                        },
                                    )
                                    .child(div().min_w_0().truncate().child(name.clone()))
                                    .when(!*directory, |el| {
                                        el.child(div().flex_shrink_0().child(format!("· {size} B")))
                                    }),
                            )
                            .when(free, |row| {
                                let path = path.clone();
                                let host = connected_alias.clone();
                                row.on_mouse_down(
                                    gpui::MouseButton::Right,
                                    cx.listener(
                                        move |root, event: &gpui::MouseDownEvent, window, cx| {
                                            window.prevent_default();
                                            cx.stop_propagation();
                                            root.focus_handle.focus(window, cx);
                                            root.dispatch(
                                                GpuiAction::Remote(Command::FileMenu {
                                                    host: host.clone(),
                                                    path: path.clone(),
                                                    position: (
                                                        f32::from(event.position.x),
                                                        f32::from(event.position.y),
                                                    ),
                                                }),
                                                cx,
                                            );
                                        },
                                    ),
                                )
                            })
                            .child(button(
                                format!("remote-terminal-{index}"),
                                if fr { "Terminal ici" } else { "Terminal here" },
                                Command::TerminalHere {
                                    host: connected_alias.clone(),
                                    path: path.clone(),
                                },
                                free,
                                &palette,
                                cx,
                            ))
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
        body = body.child(button(
            "remote-git-history",
            crate::file_timeline::Source::Git.label(fr),
            Command::GitHistory(path.clone(), false),
            !remote.busy && !remote.history_loading,
            tokens,
            cx,
        ));
        if remote.history_loading {
            body = body.child(if fr { "Chargement…" } else { "Loading…" });
        }
        if let Some(page) = &remote.git_history {
            let mut list = div()
                .id("ssh-commit-history")
                .max_h(px(180.))
                .overflow_y_scroll();
            if page.uncommitted {
                list = list.child(button(
                    "ssh-uncommitted",
                    if fr {
                        "Modifications non commitées"
                    } else {
                        "Uncommitted Changes"
                    },
                    Command::GitDiff(path.clone(), None),
                    !remote.busy,
                    tokens,
                    cx,
                ));
            }
            if !page.uncommitted && page.commits.is_empty() {
                list = list.child(if fr { "Aucun commit." } else { "No commits." });
            }
            for row in &page.commits {
                list = list.child(button(
                    format!("ssh-diff-{}", row.id),
                    row.details(fr),
                    Command::GitDiff(path.clone(), Some(row.id.clone())),
                    !remote.busy,
                    tokens,
                    cx,
                ));
            }
            body = body.child(list);
            if page.more {
                body = body.child(button(
                    "ssh-history-more",
                    if fr { "Charger la suite" } else { "Load More" },
                    Command::GitHistory(path.clone(), true),
                    !remote.busy && !remote.history_loading,
                    tokens,
                    cx,
                ));
            }
        }
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
            for (id, timestamp) in &remote.versions {
                list = list.child(div().child(button(
                    format!("remote-version-{id}"),
                    crate::regional_date::long_local(*timestamp, fr).unwrap_or_default(),
                    Command::Compare(path.clone(), id.clone()),
                    !remote.busy,
                    tokens,
                    cx,
                )));
            }
            body = body.child(list);
        }
        if let Some(lines) = &remote.comparison {
            body = body.child(button(
                "remote-close-comparison",
                if fr {
                    "Fermer la comparaison"
                } else {
                    "Close comparison"
                },
                Command::CloseComparison,
                true,
                tokens,
                cx,
            ));
            if lines.is_empty() {
                body = body.child(if fr {
                    "Aucune différence."
                } else {
                    "No differences."
                });
            } else {
                let lines = lines.clone();
                body = body.child(
                    uniform_list(
                        "remote-history-diff",
                        lines.len(),
                        cx.processor(move |_, range: std::ops::Range<usize>, _, _| {
                            range
                                .map(|i| {
                                    div()
                                        .font_family(super::theme::mono_font_family())
                                        .child(lines[i].clone())
                                })
                                .collect::<Vec<_>>()
                        }),
                    )
                    .h(px(180.)),
                );
            }
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
                    Command::Download(path.clone()),
                    !remote.busy,
                    tokens,
                    cx,
                )),
        );
        if text.is_some()
            && crate::remote_panel::control::is_html(path)
            && let Some(host) = &remote.connected
        {
            let enabled = remote.html_plain.is_none();
            let toolbar = div().flex().flex_wrap().gap(px(5.));
            let toolbar = toolbar.child(button(
                "remote-html-plain-toggle",
                if enabled {
                    if fr { "Texte brut" } else { "Plain text" }
                } else {
                    "Source"
                },
                Command::HtmlPlain {
                    host: host.clone(),
                    path: path.clone(),
                    enabled,
                },
                !remote.busy,
                tokens,
                cx,
            ));
            #[cfg(windows)]
            let toolbar = {
                let viewing = remote.html_page.is_some();
                let mut toolbar = toolbar.when(!viewing, |bar| {
                    bar.child(button(
                        "remote-html-preview",
                        if fr { "Aperçu" } else { "Preview" },
                        Command::HtmlPage {
                            host: host.clone(),
                            path: path.clone(),
                        },
                        !remote.busy,
                        tokens,
                        cx,
                    ))
                });
                if viewing {
                    toolbar = toolbar.child(button(
                        "remote-html-source",
                        "Source",
                        Command::HtmlPlain {
                            host: host.clone(),
                            path: path.clone(),
                            enabled: false,
                        },
                        !remote.busy,
                        tokens,
                        cx,
                    ));
                }
                let path = path.clone();
                let host = host.clone();
                toolbar.when(!viewing, |bar| {
                    bar.child(
                        div()
                            .id("remote-html-browser")
                            .cursor_pointer()
                            .on_click(cx.listener(move |root, _, _, cx| {
                                root.dispatch(
                                    GpuiAction::OpenRemoteHtmlInBrowser {
                                        host: host.clone(),
                                        path: path.clone(),
                                    },
                                    cx,
                                )
                            }))
                            .child(if fr {
                                "Ouvrir dans le navigateur"
                            } else {
                                "Open in browser"
                            }),
                    )
                })
            };
            body = body.child(toolbar);
        }
        if let Some((_id, fallback)) = &remote.html_page {
            let text = div()
                .p(px(12.))
                .child(SharedString::from(fallback.clone()))
                .into_any_element();
            #[cfg(windows)]
            let text = super::editor_view::html_viewport(*_id, text, tokens, snapshot, cx);
            body = body.child(text);
        } else if remote.html_plain.is_some() {
            if let Some(state) = editor_input.plain() {
                body = body.child(super::editor_view::plain_text(state, tokens));
            }
        } else if let Some(text) = text {
            body = body.child(
                div()
                    .id("remote-text-scroll")
                    .text_size(px(f32::from(snapshot.editor_font_size)))
                    .font_family(super::theme::mono_font_family())
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
        } else if let Some(source) = &remote.playback {
            body = body.child(super::editor_view::render_playback(source, fr, cx));
        } else if let Some(office) = &remote.office {
            body = body.child(super::editor_view::render_office_summary(
                office, fr, tokens, cx, true,
            ));
        } else if let Some((id, summary)) = &remote.pdf {
            body = body.child(super::editor_view::render_pdf_stage(
                summary,
                super::editor_view::PdfTarget::Remote(*id),
                fr,
                tokens,
                cx,
            ));
        } else if let Some(image) = &remote.pixels {
            let size = image.size(0);
            let path_for_menu = path.clone();
            let image_view = super::editor_view::preview_image_pixels(image.clone())
                .id("remote-preview-pixels")
                .debug_selector(|| "remote-preview-pixels".into())
                .when(!remote.busy, |el| {
                    el.on_mouse_down(
                        gpui::MouseButton::Right,
                        cx.listener(move |root, event: &gpui::MouseDownEvent, window, cx| {
                            window.prevent_default();
                            root.focus_handle.focus(window, cx);
                            cx.stop_propagation();
                            root.dispatch(
                                GpuiAction::Remote(Command::ImageMenu(
                                    path_for_menu.clone(),
                                    Some((
                                        f32::from(event.position.x),
                                        f32::from(event.position.y),
                                    )),
                                )),
                                cx,
                            );
                        }),
                    )
                });
            let view = div()
                .id("remote-image-viewport")
                .debug_selector(|| "remote-image-viewport".into())
                .flex_1()
                .min_h_0()
                .w_full()
                .flex()
                .flex_col()
                .p(px(20.))
                .child(super::editor_view::fitted_preview_image(
                    image_view,
                    [size.width.0 as usize, size.height.0 as usize],
                ));
            body = body.child(view);
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
