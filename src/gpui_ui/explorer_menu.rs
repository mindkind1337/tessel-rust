//! Local explorer menu. Commands resolve the clicked entry again in its original project.
use super::{FocusInputTarget, TesselGpuiRoot, theme::GpuiThemeTokens};
use crate::app::gpui_bridge::{ExplorerCommand, GpuiAction, NativeExplorerScope};
use gpui::{prelude::*, *};

pub(super) struct Menu {
    pub scope: NativeExplorerScope,
    pub query: String,
    key: Option<String>,
    directory: bool,
    position: Point<Pixels>,
    focus: FocusHandle,
    active: Option<usize>,
}

pub(super) fn trigger(
    el: Stateful<Div>,
    key: Option<String>,
    directory: bool,
    cx: &Context<TesselGpuiRoot>,
) -> Stateful<Div> {
    el.on_mouse_down(
        MouseButton::Right,
        cx.listener(move |root, ev: &MouseDownEvent, window, cx| {
            let Some((scope, query)) = root.app.gpui_explorer_input() else {
                return;
            };
            let query = query.to_owned();
            let focus = cx.focus_handle();
            focus.focus(window, cx);
            root.focus_input = FocusInputTarget::ExplorerTree;
            window.prevent_default();
            cx.stop_propagation();
            root.explorer_input.menu = Some(Menu {
                scope: scope.clone(),
                query,
                key: key.clone(),
                directory,
                position: ev.position,
                focus,
                active: None,
            });
            root.dispatch(
                GpuiAction::ExplorerMenu(scope, key.clone(), ExplorerCommand::Select),
                cx,
            );
        }),
    )
}

fn dismiss(
    root: &mut TesselGpuiRoot,
    window: &mut Window,
    cx: &mut Context<TesselGpuiRoot>,
) -> Option<Menu> {
    let menu = root.explorer_input.menu.take()?;
    if menu.focus.is_focused(window)
        && let Some(tree) = root.explorer_input.tree_focus.get()
    {
        tree.focus(window, cx);
    }
    cx.notify();
    Some(menu)
}

fn choose(
    cmd: ExplorerCommand,
    root: &mut TesselGpuiRoot,
    window: &mut Window,
    cx: &mut Context<TesselGpuiRoot>,
) {
    if let Some(menu) = dismiss(root, window, cx) {
        root.dispatch(GpuiAction::ExplorerMenu(menu.scope, menu.key, cmd), cx);
    }
}

impl Menu {
    pub(super) fn render(
        &self,
        fr: bool,
        editable: bool,
        explorer: &crate::explorer_tree::ExplorerGpuiSnapshot,
        tokens: &GpuiThemeTokens,
        cx: &Context<TesselGpuiRoot>,
    ) -> AnyElement {
        use ExplorerCommand::*;
        let translate = |en, french| if fr { french } else { en };
        let mut choices = Vec::new();
        if self.key.is_some() && !self.directory {
            choices.push((Open, translate("Open", "Ouvrir")));
        }
        if editable {
            choices.push((NewFile, translate("New file", "Nouveau fichier")));
            choices.push((NewFolder, translate("New folder", "Nouveau dossier")));
        }
        if editable && self.key.is_some() {
            choices.extend([
                (Rename, translate("Rename", "Renommer")),
                (
                    Trash,
                    translate("Move to Recycle Bin", "Mettre à la corbeille"),
                ),
            ]);
        }
        if self.key.is_some() {
            choices.extend([
                (CopyPath, translate("Copy path", "Copier le chemin")),
                (
                    CopyRelative,
                    translate("Copy relative path", "Copier le chemin relatif"),
                ),
            ]);
        }
        choices.extend([
            (
                Dotfiles,
                translate("Show dotfiles", "Afficher les dotfiles"),
            ),
            (Compact, translate("Compact folders", "Dossiers compacts")),
        ]);
        let mut menu = div()
            .id("explorer-context-menu")
            .debug_selector(|| "explorer-context-menu".into())
            .role(Role::Menu)
            .track_focus(&self.focus)
            .occlude()
            .flex()
            .flex_col()
            .w(px(205.))
            .p(px(4.))
            .bg(tokens.surface)
            .border_1()
            .border_color(tokens.border)
            .rounded(px(5.))
            .shadow_lg()
            .on_mouse_down_out(cx.listener(|root, _: &MouseDownEvent, window, cx| {
                dismiss(root, window, cx);
            }));
        for (i, &(command, label)) in choices.iter().enumerate() {
            menu = menu.child(
                div()
                    .id(("explorer-context-item", i))
                    .role(Role::MenuItem)
                    .debug_selector(move || format!("explorer-context-{command:?}"))
                    .text_size(px(12.))
                    .px(px(8.))
                    .py(px(4.))
                    .text_color(tokens.text)
                    .cursor_pointer()
                    .when(self.active == Some(i), |el| el.bg(tokens.surface_3))
                    .hover(|s| s.bg(tokens.surface_3))
                    .child(
                        if (command == Dotfiles && explorer.show_dotfiles)
                            || (command == Compact && explorer.compact_folders)
                        {
                            format!("✓ {label}")
                        } else {
                            label.to_owned()
                        },
                    )
                    .on_click(
                        cx.listener(move |root, _, window, cx| choose(command, root, window, cx)),
                    ),
            );
        }
        menu = menu.on_key_down(cx.listener(move |root, ev: &KeyDownEvent, window, cx| {
            let Some(current) = &mut root.explorer_input.menu else {
                return;
            };
            let n = choices.len();
            match ev.keystroke.key.as_str() {
                "escape" => {
                    dismiss(root, window, cx);
                }
                "down" => current.active = Some(current.active.map_or(0, |i| (i + 1) % n)),
                "up" => current.active = Some(current.active.map_or(n - 1, |i| (i + n - 1) % n)),
                "enter" | "space" => {
                    if let Some(i) = current.active.filter(|i| *i < n) {
                        choose(choices[i].0, root, window, cx);
                    }
                }
                _ => return,
            }
            window.prevent_default();
            cx.stop_propagation();
            cx.notify();
        }));
        deferred(
            anchored()
                .position(self.position)
                .snap_to_window_with_margin(px(4.))
                .child(menu),
        )
        .priority(2)
        .into_any_element()
    }
}
