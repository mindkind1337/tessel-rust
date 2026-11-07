//! Primary GPU-accelerated renderer (`GPUI`) layered over the shared `crate::app::Tessel` core.
//!
//! Provides a 1:1 CSS Flexbox + DirectWrite subpixel typography + `resvg` vector icon frontend
//! while keeping the `eframe`/`egui` Compatibility renderer intact with automatic runtime fallback.

pub(crate) mod chat_view;
pub(crate) mod chrome;
pub mod editor_view;
pub mod icons;
pub(crate) mod shell;
pub(crate) mod terminal_view;
pub mod theme;

#[cfg(test)]
mod tests;

pub(crate) use crate::app::gpui_bridge::GpuiFocusTarget as FocusInputTarget;

use crate::{
    app::{
        Tessel,
        gpui_bridge::{GpuiAction, GpuiCenterMode},
    },
    pane_grid::{FocusDir, SplitDir},
};
use gpui::{
    App, AppContext as _, Bounds, ClipboardItem, Context, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement,
    Render, SharedString, Styled, Task, TitlebarOptions, Window, WindowBounds, WindowDecorations,
    WindowOptions, div, prelude::FluentBuilder as _, px, size,
};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use theme::{
    BROADCAST_BANNER_HEIGHT_PX, GpuiThemeTokens, RIGHT_PANEL_DEFAULT_WIDTH_PX,
    RIGHT_PANEL_MAX_WIDTH_PX, RIGHT_PANEL_MIN_WIDTH_PX, SIDEBAR_DEFAULT_WIDTH_PX,
    SIDEBAR_MAX_WIDTH_PX, SIDEBAR_MIN_WIDTH_PX, SIDEBAR_RAIL_WIDTH_PX, SPLIT_DIVIDER_PX,
    STATUSBAR_HEIGHT_PX, TOOLBAR_HEIGHT_PX, VIEW_TABS_HEIGHT_PX, ui_font_family,
};

#[derive(Clone, Debug)]
pub(crate) enum DragTarget {
    LeftSidebar,
    RightPanel,
    FloatingTerminal {
        stage_height_px: f32,
    },
    SplitDivider {
        split_id: String,
        dir: SplitDir,
        index: usize,
        split_side_px: f64,
        last_pos_px: f64,
    },
}

pub(crate) struct TesselGpuiRoot {
    pub(crate) app: Tessel,
    pub(crate) focus_handle: FocusHandle,
    pub(crate) focus_input: FocusInputTarget,
    pub(crate) left_width: f32,
    pub(crate) right_width: f32,
    pub(crate) active_drag: Option<DragTarget>,
    repaint_flag: Arc<AtomicBool>,
    _poller_task: Task<()>,
}

impl TesselGpuiRoot {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let repaint_flag = Arc::new(AtomicBool::new(false));
        let cb_flag = Arc::clone(&repaint_flag);
        let app = Tessel::new_for_gpui(move || {
            cb_flag.store(true, Ordering::Release);
        });
        Self::from_app(app, repaint_flag, cx)
    }

    #[allow(dead_code)]
    pub(crate) fn new_in_dir(
        override_dir: Option<PathBuf>,
        probe_environment: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let repaint_flag = Arc::new(AtomicBool::new(false));
        let cb_flag = Arc::clone(&repaint_flag);
        let app = Tessel::new_for_gpui_in_dir(override_dir, probe_environment, move || {
            cb_flag.store(true, Ordering::Release);
        });
        Self::from_app(app, repaint_flag, cx)
    }

    fn from_app(app: Tessel, repaint_flag: Arc<AtomicBool>, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        let poll_flag = Arc::clone(&repaint_flag);
        let poller_task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(25))
                    .await;
                let ok = this.update(cx, |root, cx| {
                    let flagged = poll_flag.swap(false, Ordering::AcqRel);
                    root.app.poll_for_gpui();
                    if flagged {
                        cx.notify();
                    }
                });
                if ok.is_err() {
                    break;
                }
            }
        });

        Self {
            app,
            focus_handle,
            focus_input: FocusInputTarget::None,
            left_width: SIDEBAR_DEFAULT_WIDTH_PX,
            right_width: RIGHT_PANEL_DEFAULT_WIDTH_PX,
            active_drag: None,
            repaint_flag,
            _poller_task: poller_task,
        }
    }

    pub(crate) fn dispatch(&mut self, action: GpuiAction, cx: &mut Context<Self>) {
        self.app.apply_gpui_action(action);
        self.repaint_flag.store(false, Ordering::Release);
        cx.notify();
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ks = &event.keystroke;
        let ctrl = ks.modifiers.control || ks.modifiers.platform;
        let shift = ks.modifiers.shift;
        let alt = ks.modifiers.alt;
        let key = ks.key.as_str();

        if key == "f1" {
            self.dispatch(GpuiAction::OpenHelp, cx);
            return;
        }
        if key == "escape" {
            if self.app.gpui_has_open_modal() {
                self.dispatch(GpuiAction::CloseOverlays, cx);
                return;
            }
            if let FocusInputTarget::TerminalFind(sid) = self.focus_input {
                self.focus_input = FocusInputTarget::Terminal(sid);
                self.dispatch(GpuiAction::TerminalCloseFind(sid), cx);
                return;
            }
            if matches!(
                self.focus_input,
                FocusInputTarget::EditorFind
                    | FocusInputTarget::EditorReplace
                    | FocusInputTarget::EditorGoto
            ) {
                let was_goto = self.focus_input == FocusInputTarget::EditorGoto;
                self.focus_input = FocusInputTarget::EditorBody;
                self.dispatch(
                    if was_goto {
                        GpuiAction::ToggleEditorGoto
                    } else {
                        GpuiAction::ToggleEditorFind
                    },
                    cx,
                );
                return;
            }
        }

        if ctrl && !alt {
            match key {
                "k" if !shift => {
                    self.dispatch(GpuiAction::ToggleCommandPalette, cx);
                    return;
                }
                "p" if shift => {
                    self.dispatch(GpuiAction::ToggleCommandPalette, cx);
                    return;
                }
                "b" if !shift => {
                    self.dispatch(GpuiAction::ToggleLeftSidebar, cx);
                    return;
                }
                "j" if !shift => {
                    self.dispatch(GpuiAction::ToggleRightSidebar, cx);
                    return;
                }
                "`" => {
                    self.focus_input = FocusInputTarget::None;
                    self.dispatch(GpuiAction::ToggleFloatingTerminal, cx);
                    return;
                }
                "," if !shift => {
                    self.dispatch(GpuiAction::OpenSettings, cx);
                    return;
                }
                "=" | "+" => {
                    self.dispatch(GpuiAction::ZoomFont(1), cx);
                    return;
                }
                "-" => {
                    self.dispatch(GpuiAction::ZoomFont(-1), cx);
                    return;
                }
                "0" if !shift => {
                    self.dispatch(GpuiAction::ZoomFont(0), cx);
                    return;
                }
                "tab" => {
                    self.focus_input = FocusInputTarget::None;
                    self.dispatch(GpuiAction::CycleWorkspace(if shift { -1 } else { 1 }), cx);
                    return;
                }
                "s" if !shift && self.app.gpui_is_document_view() => {
                    self.dispatch(GpuiAction::SaveActiveDocument, cx);
                    return;
                }
                "f" if !shift && self.app.gpui_is_document_view() => {
                    self.focus_input = FocusInputTarget::EditorFind;
                    self.dispatch(GpuiAction::ToggleEditorFind, cx);
                    return;
                }
                "h" if !shift && self.app.gpui_is_document_view() => {
                    self.focus_input = FocusInputTarget::EditorReplace;
                    self.dispatch(GpuiAction::ToggleEditorReplace, cx);
                    return;
                }
                "g" if !shift && self.app.gpui_is_document_view() => {
                    self.focus_input = FocusInputTarget::EditorGoto;
                    self.dispatch(GpuiAction::ToggleEditorGoto, cx);
                    return;
                }
                _ => {}
            }
        }

        if alt && !ctrl && !shift {
            match key {
                "left" => {
                    self.focus_input = FocusInputTarget::None;
                    self.dispatch(GpuiAction::FocusPaneDirection(FocusDir::Left), cx);
                    return;
                }
                "right" => {
                    self.focus_input = FocusInputTarget::None;
                    self.dispatch(GpuiAction::FocusPaneDirection(FocusDir::Right), cx);
                    return;
                }
                "up" => {
                    self.focus_input = FocusInputTarget::None;
                    self.dispatch(GpuiAction::FocusPaneDirection(FocusDir::Up), cx);
                    return;
                }
                "down" => {
                    self.focus_input = FocusInputTarget::None;
                    self.dispatch(GpuiAction::FocusPaneDirection(FocusDir::Down), cx);
                    return;
                }
                _ => {}
            }
        }

        if self.app.gpui_has_open_modal() {
            return;
        }

        let target = if self.focus_input == FocusInputTarget::None {
            self.app.gpui_default_focus_target()
        } else {
            self.focus_input
        };

        if let FocusInputTarget::Terminal(sid) = target {
            if ctrl && shift && !alt && key == "f" {
                self.focus_input = FocusInputTarget::TerminalFind(sid);
                self.dispatch(GpuiAction::TerminalOpenFind(sid), cx);
                return;
            }
            if ctrl && !alt && key == "v" {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    self.dispatch(
                        GpuiAction::TerminalPaste {
                            session_id: sid,
                            text,
                        },
                        cx,
                    );
                }
                return;
            }
            if let Some(copied) =
                self.app
                    .send_gpui_terminal_key(sid, key, ks.key_char.as_deref(), ctrl, alt, shift)
            {
                cx.write_to_clipboard(ClipboardItem::new_string(copied));
            }
            cx.notify();
            return;
        }

        let pasted = if ctrl && !alt && key == "v" {
            cx.read_from_clipboard().and_then(|item| item.text())
        } else {
            None
        };
        let printable = ks
            .key_char
            .as_deref()
            .filter(|s| !s.is_empty() && (!ctrl || alt) && s.chars().all(|c| c >= ' '));
        if self
            .app
            .gpui_handle_text_target_key(target, key, printable, shift, pasted)
        {
            cx.notify();
        }
    }

    fn handle_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.active_drag.clone() else {
            return;
        };
        let viewport_w = f32::from(window.viewport_size().width);
        let viewport_h = f32::from(window.viewport_size().height);
        match drag {
            DragTarget::LeftSidebar => {
                let next =
                    f32::from(event.position.x).clamp(SIDEBAR_MIN_WIDTH_PX, SIDEBAR_MAX_WIDTH_PX);
                if (next - self.left_width).abs() > 0.5 {
                    self.left_width = next;
                    cx.notify();
                }
            }
            DragTarget::RightPanel => {
                let next = (viewport_w - f32::from(event.position.x))
                    .clamp(RIGHT_PANEL_MIN_WIDTH_PX, RIGHT_PANEL_MAX_WIDTH_PX);
                if (next - self.right_width).abs() > 0.5 {
                    self.right_width = next;
                    cx.notify();
                }
            }
            DragTarget::FloatingTerminal { stage_height_px } => {
                let bottom_bar = if self.app.state.status_bar.visible {
                    STATUSBAR_HEIGHT_PX
                } else {
                    0.0
                };
                let from_bottom = (viewport_h - bottom_bar - f32::from(event.position.y)).max(80.0);
                let ratio = (from_bottom / stage_height_px.max(120.0)).clamp(0.20, 0.85);
                self.app
                    .apply_gpui_action(GpuiAction::SetFloatingHeightRatio(ratio));
                cx.notify();
            }
            DragTarget::SplitDivider {
                split_id,
                dir,
                index,
                split_side_px,
                last_pos_px,
            } => {
                let current_pos = match dir {
                    SplitDir::Row => f64::from(event.position.x),
                    SplitDir::Col => f64::from(event.position.y),
                };
                let delta_px = current_pos - last_pos_px;
                if delta_px.abs() >= 1.0 {
                    self.active_drag = Some(DragTarget::SplitDivider {
                        split_id: split_id.clone(),
                        dir,
                        index,
                        split_side_px,
                        last_pos_px: current_pos,
                    });
                    self.app.apply_gpui_action(GpuiAction::ResizeSplitDivider {
                        split_id,
                        dir,
                        index,
                        split_side_px,
                        delta_px,
                    });
                    cx.notify();
                }
            }
        }
    }
}

impl Focusable for TesselGpuiRoot {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TesselGpuiRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.app.poll_for_gpui();

        let viewport = window.viewport_size();
        let viewport_w = f32::from(viewport.width).max(320.0);
        let viewport_h = f32::from(viewport.height).max(240.0);

        let prelim = self.app.gpui_snapshot(960.0, 640.0, SPLIT_DIVIDER_PX);
        let left_px = if prelim.toolbar.left_open {
            self.left_width
        } else {
            SIDEBAR_RAIL_WIDTH_PX
        };
        let right_px = if prelim.toolbar.right_open {
            if prelim.right_fullscreen {
                (viewport_w - left_px).max(0.0)
            } else {
                self.right_width
            }
        } else {
            0.0
        };
        let mut chrome_h = TOOLBAR_HEIGHT_PX + VIEW_TABS_HEIGHT_PX;
        if prelim.toolbar.broadcast_enabled {
            chrome_h += BROADCAST_BANNER_HEIGHT_PX;
        }
        if prelim.toolbar.error_banner.is_some() {
            chrome_h += 24.0;
        }
        if prelim.status_bar.visible {
            chrome_h += STATUSBAR_HEIGHT_PX;
        }

        let grid_w = (viewport_w - left_px - right_px).max(160.0);
        let grid_h = (viewport_h - chrome_h).max(120.0);

        #[cfg(windows)]
        if prelim.center_mode == GpuiCenterMode::BrowserTab {
            self.app.browser.auto_start_if_pending(window);
        } else {
            self.app.browser.visible(false);
        }

        let snapshot = self.app.gpui_snapshot(grid_w, grid_h, SPLIT_DIVIDER_PX);
        let tokens = GpuiThemeTokens::from_theme(snapshot.theme);
        let is_maximized = window.is_maximized();

        div()
            .id("tessel-gpui-root")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(tokens.backdrop)
            .text_color(tokens.text)
            .font_family(ui_font_family())
            .on_key_down(cx.listener(Self::handle_key_down))
            .on_mouse_move(cx.listener(Self::handle_mouse_move))
            .capture_any_mouse_up(cx.listener(|root, _: &MouseUpEvent, _, cx| {
                if root.active_drag.take().is_some() {
                    cx.notify();
                }
            }))
            .child(chrome::render_toolbar(&snapshot, &tokens, is_maximized, cx))
            .when_some(
                chrome::render_broadcast_banner(&snapshot, &tokens, cx),
                |el, banner| el.child(banner),
            )
            .when_some(
                chrome::render_error_banner(&snapshot, &tokens, cx),
                |el, banner| el.child(banner),
            )
            .child(shell::render_workspace_body(
                &snapshot,
                &tokens,
                self.left_width,
                self.right_width,
                grid_h,
                window,
                cx,
            ))
            .when_some(
                chrome::render_status_bar(&snapshot, &tokens, viewport_w, cx),
                |el, bar| el.child(bar),
            )
    }
}

/// Launches the primary `GPUI` native hardware-accelerated window.
///
/// Returns `Err(reason)` cleanly if platform/GPU initialization or window creation fails so
/// `main.rs` can automatically fall back to the `eframe`/`egui` Compatibility renderer.
pub fn run_gpui_app() -> Result<(), String> {
    let open_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let open_error_slot = Arc::clone(&open_error);

    let run_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        gpui_platform::application().run(move |cx: &mut App| {
            let bounds = Bounds::centered(None, size(px(1280.0), px(800.0)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(SharedString::new_static("Tessel Rust")),
                    appears_transparent: true,
                    traffic_light_position: None,
                }),
                window_min_size: Some(size(px(640.0), px(420.0))),
                window_decorations: Some(WindowDecorations::Client),
                app_id: Some("com.tessel.rust".to_owned()),
                ..Default::default()
            };

            match cx.open_window(options, |window, cx| {
                let view = cx.new(TesselGpuiRoot::new);
                let focus = view.read(cx).focus_handle.clone();
                window.focus(&focus, cx);
                view
            }) {
                Ok(_) => {
                    cx.activate(true);
                }
                Err(err) => {
                    if let Ok(mut guard) = open_error_slot.lock() {
                        *guard = Some(format!("GPUI window initialization failed: {err}"));
                    }
                    cx.quit();
                }
            }
        });
    }));

    if let Err(payload) = run_result {
        let msg = if let Some(s) = payload.downcast_ref::<&str>() {
            (*s).to_owned()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown GPUI platform panic".to_owned()
        };
        return Err(format!("GPUI platform startup panicked: {msg}"));
    }

    if let Ok(mut guard) = open_error.lock()
        && let Some(err) = guard.take()
    {
        return Err(err);
    }

    Ok(())
}
