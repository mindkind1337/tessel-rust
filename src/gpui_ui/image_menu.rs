//! Single-action context menus for document previews.
use super::{TesselGpuiRoot, theme::GpuiThemeTokens};
use crate::{
    app::gpui_bridge::{GpuiAction, GpuiCenterMode, GpuiWorkspaceSnapshot},
    remote_panel::control::Command,
};
use gpui::{
    Context, Div, FocusHandle, KeyDownEvent, Role, Stateful, anchored, deferred, div, point,
    prelude::*, px,
};

pub(super) struct Menu {
    position: (f32, f32),
    menu_id: &'static str,
    item_id: &'static str,
    label: &'static str,
    enabled: bool,

    pub(super) close: GpuiAction,
    activate: GpuiAction,
}

impl Menu {
    pub fn guest(key: &crate::guest_file_editor::Key, position: (f32, f32), french: bool) -> Self {
        use crate::share_panel::ShareGpuiAction;
        Self {
            position,
            menu_id: "guest-image-menu",
            item_id: "guest-image-download",
            enabled: true,
            label: if french {
                "Télécharger l’original…"
            } else {
                "Download original…"
            },
            close: GpuiAction::Share(ShareGpuiAction::GuestImageMenu(key.clone(), None)),
            activate: GpuiAction::Share(ShareGpuiAction::GuestDownload(key.clone())),
        }
    }

    pub fn guest_file(
        key: &crate::guest_file_editor::Key,
        position: (f32, f32),
        french: bool,
    ) -> Self {
        use crate::share_panel::ShareGpuiAction;
        Self {
            position,
            menu_id: "guest-file-download-menu",
            item_id: "guest-file-download",
            enabled: true,
            label: if french {
                "Télécharger…"
            } else {
                "Download…"
            },
            close: GpuiAction::Share(ShareGpuiAction::GuestFileMenu(key.clone(), None)),
            activate: GpuiAction::Share(ShareGpuiAction::GuestDownload(key.clone())),
        }
    }

    pub fn current(s: &GpuiWorkspaceSnapshot, modal: bool) -> Option<Self> {
        if modal || s.floating.open {
            return None;
        }
        let (position, close, download, enabled, local) = match s.center_mode {
            GpuiCenterMode::DocumentEditor => {
                let doc = s.active_document.as_ref()?;
                if let Some((position, web)) = doc.html_link_menu {
                    let label = match (web, s.french) {
                        (true, true) => "Ouvrir dans le navigateur",
                        (true, false) => "Open in browser",
                        (false, true) => "Ouvrir dans un nouvel onglet",
                        (false, false) => "Open in new tab",
                    };
                    return Some(Self {
                        position,
                        enabled: true,
                        label,
                        menu_id: "html-link-menu",
                        item_id: "html-link-open",
                        close: GpuiAction::CloseHtmlLinkMenu,
                        activate: GpuiAction::OpenHtmlLinkMenuTarget(doc.instance_id),
                    });
                }
                (
                    doc.image_menu?,
                    GpuiAction::ImageMenu(doc.instance_id, None),
                    GpuiAction::DownloadImage(doc.instance_id),
                    true,
                    true,
                )
            }
            GpuiCenterMode::RemoteSftp => {
                if let Some((path, position)) = &s.remote.file_menu {
                    return Some(Self {
                        position: *position,
                        menu_id: "remote-file-menu",
                        item_id: "remote-file-download",
                        label: if s.french {
                            "Télécharger…"
                        } else {
                            "Download…"
                        },
                        enabled: !s.remote.busy,
                        close: GpuiAction::Remote(Command::CloseFileMenu),
                        activate: GpuiAction::Remote(Command::DownloadEntry {
                            host: s.remote.connected.clone()?,
                            path: path.clone(),
                        }),
                    });
                }
                let path = &s.remote.document.as_ref()?.0;
                (
                    s.remote.image_menu?,
                    GpuiAction::Remote(Command::ImageMenu(path.clone(), None)),
                    GpuiAction::Remote(Command::Download(path.clone())),
                    !s.remote.busy,
                    false,
                )
            }
            _ => return None,
        };
        Some(Self {
            position,
            menu_id: if local {
                "local-image-menu"
            } else {
                "remote-image-menu"
            },
            item_id: if local {
                "local-image-download"
            } else {
                "remote-image-download"
            },
            enabled,
            label: if s.french {
                "Télécharger…"
            } else {
                "Download…"
            },
            close,
            activate: download,
        })
    }

    pub fn attach(
        self,
        root: Stateful<Div>,
        focus: &FocusHandle,
        viewport_height: f32,
        tokens: &GpuiThemeTokens,
        cx: &Context<TesselGpuiRoot>,
    ) -> Stateful<Div> {
        let (menu_id, item_id, label) = (self.menu_id, self.item_id, self.label);
        let close = self.close.clone();
        let activate = self.activate.clone();
        let choice = div()
            .id(item_id)
            .role(Role::MenuItem)
            .aria_label(label)
            .aria_disabled(!self.enabled)
            .debug_selector(move || item_id.into())
            .track_focus(focus)
            .w_full()
            .px(px(8.))
            .py(px(4.))
            .line_height(px(17.))
            .rounded(px(6.))
            .opacity(if self.enabled { 1. } else { 0.45 })
            .when(self.enabled, |el| {
                el.cursor_pointer()
                    .hover(|s| s.bg(tokens.surface_3))
                    .focus_visible(|s| s.bg(tokens.surface_3))
                    .on_click(
                        cx.listener(move |root, _, _, cx| root.dispatch(activate.clone(), cx)),
                    )
            })
            .child(label);
        let menu = div()
            .id(menu_id)
            .role(Role::Menu)
            .debug_selector(move || menu_id.into())
            .occlude()
            .min_w(gpui::rems(11.))
            .text_size(px(12.))
            .text_color(tokens.text_strong)
            .border_1()
            .border_color(super::theme::rgba_with_alpha(tokens.text_strong, 0.14))
            .bg(super::theme::rgba_with_alpha(tokens.backdrop, 0.90))
            .rounded(px(11.))
            .p(px(4.))
            .shadow_lg()
            .on_mouse_down_out(cx.listener(move |root, _: &gpui::MouseDownEvent, _, cx| {
                root.dispatch(close.clone(), cx)
            }))
            .child(choice);
        let top = if self.position.1 + 6. + 35. > viewport_height - 8. {
            self.position.1 - 6. - 35.
        } else {
            self.position.1 + 6.
        };
        root.child(
            deferred(
                anchored()
                    .position(point(px(self.position.0), px(top)))
                    .snap_to_window_with_margin(px(8.))
                    .child(menu),
            )
            .priority(2),
        )
        .capture_key_down(cx.listener(move |root, event: &KeyDownEvent, window, cx| {
            match event.keystroke.key.as_str() {
                "escape" => {
                    if root.image_menu_item.is_focused(window) {
                        root.focus_handle.focus(window, cx);
                    }
                    root.dispatch(self.close.clone(), cx);
                }
                "up" | "down" if self.enabled => root.image_menu_item.focus(window, cx),
                "enter" | "space"
                    if !event.is_held
                        && self.enabled
                        && root.image_menu_item.is_focused(window) =>
                {
                    root.focus_handle.focus(window, cx);
                    root.dispatch(self.activate.clone(), cx);
                }
                _ => return,
            }
            window.prevent_default();
            cx.stop_propagation();
        }))
    }
}
