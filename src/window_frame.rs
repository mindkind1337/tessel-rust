//! Windows title-bar prototype. System non-client actions remain owned by Windows.
//! https://learn.microsoft.com/windows/win32/dwm/customframe
use eframe::egui::{self, Pos2, Rect, Vec2};
use std::sync::{Arc, Mutex};
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::{Dwm::DwmDefWindowProc, Gdi::ScreenToClient},
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};
use wry::raw_window_handle::{HasWindowHandle, RawWindowHandle};

const SUBCLASS_ID: usize = 0x54455353;
pub const CONTROL_WIDTH: f32 = 138.0;

#[derive(Clone)]
struct Regions {
    toolbar: Rect,
    excluded: [Rect; 2],
    controls: [Rect; 3],
    scale: f32,
    drag_enabled: bool,
    fullscreen: bool,
}
impl Default for Regions {
    fn default() -> Self {
        Self {
            toolbar: Rect::NOTHING,
            excluded: [Rect::NOTHING; 2],
            controls: [Rect::NOTHING; 3],
            scale: 1.0,
            drag_enabled: false,
            fullscreen: false,
        }
    }
}
impl Regions {
    fn hit(&self, point: Pos2, bounds: Rect, maximized: bool) -> u32 {
        if !point.x.is_finite()
            || !point.y.is_finite()
            || !bounds.contains(point)
            || self.fullscreen
        {
            return HTCLIENT;
        }
        if !maximized {
            let left = point.x < bounds.left() + 6.0;
            let right = point.x >= bounds.right() - 6.0;
            let top = point.y < bounds.top() + 6.0;
            let bottom = point.y >= bounds.bottom() - 6.0;
            match (left, right, top, bottom) {
                (true, _, true, _) => return HTTOPLEFT,
                (_, true, true, _) => return HTTOPRIGHT,
                (true, _, _, true) => return HTBOTTOMLEFT,
                (_, true, _, true) => return HTBOTTOMRIGHT,
                (true, _, _, _) => return HTLEFT,
                (_, true, _, _) => return HTRIGHT,
                (_, _, true, _) => return HTTOP,
                (_, _, _, true) => return HTBOTTOM,
                _ => {}
            }
        }
        for (rect, hit) in self
            .controls
            .iter()
            .zip([HTMINBUTTON, HTMAXBUTTON, HTCLOSE])
        {
            if rect.contains(point) {
                return hit;
            }
        }
        if self.drag_enabled
            && self.toolbar.contains(point)
            && !self.excluded.iter().any(|rect| rect.contains(point))
        {
            HTCAPTION
        } else {
            HTCLIENT
        }
    }
}

struct Hook {
    ctx: egui::Context,
    regions: Arc<Mutex<Regions>>,
}

unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    if message == WM_NCDESTROY {
        // The registered subclass owns this allocation until the HWND is destroyed.
        unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(procedure), id);
            drop(Box::from_raw(data as *mut Hook));
            return DefSubclassProc(hwnd, message, wparam, lparam);
        }
    }
    let hook = unsafe { &*(data as *const Hook) };
    if message == WM_NCHITTEST
        && let Ok(regions) = hook.regions.try_lock()
    {
        let mut point = POINT {
            x: (lparam.0 as u32 as u16 as i16) as i32,
            y: ((lparam.0 as u32 >> 16) as u16 as i16) as i32,
        };
        let mut bounds = RECT::default();
        // ScreenToClient also handles maximized windows and monitors with negative origins.
        if unsafe {
            ScreenToClient(hwnd, &mut point).as_bool() && GetClientRect(hwnd, &mut bounds).is_ok()
        } {
            let scale = regions.scale.max(0.1);
            let pos = egui::pos2(point.x as f32 / scale, point.y as f32 / scale);
            let rect = Rect::from_min_max(
                egui::pos2(bounds.left as f32 / scale, bounds.top as f32 / scale),
                egui::pos2(bounds.right as f32 / scale, bounds.bottom as f32 / scale),
            );
            return LRESULT(regions.hit(pos, rect, unsafe { IsZoomed(hwnd).as_bool() }) as isize);
        }
    }
    if matches!(
        message,
        WM_NCMOUSEMOVE | WM_NCMOUSELEAVE | WM_NCLBUTTONDOWN | WM_NCLBUTTONUP
    ) {
        hook.ctx.request_repaint();
    }
    let mut result = LRESULT(0);
    if unsafe { DwmDefWindowProc(hwnd, message, wparam, lparam, &mut result).as_bool() } {
        return result;
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

pub struct WindowFrame {
    hwnd: HWND,
    regions: Arc<Mutex<Regions>>,
}
impl WindowFrame {
    pub fn install(cc: &eframe::CreationContext<'_>) -> Option<Self> {
        let RawWindowHandle::Win32(raw) = cc.window_handle().ok()?.as_raw() else {
            return None;
        };
        let hwnd = HWND(raw.hwnd.get() as *mut std::ffi::c_void);
        let regions = Arc::new(Mutex::new(Regions::default()));
        let hook = Box::into_raw(Box::new(Hook {
            ctx: cc.egui_ctx.clone(),
            regions: regions.clone(),
        }));
        // Installation and callbacks both run on the window's owning UI thread.
        if !unsafe {
            SetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, hook as usize).as_bool()
        } {
            unsafe {
                drop(Box::from_raw(hook));
            }
            return None;
        }
        cc.egui_ctx
            .send_viewport_cmd(egui::ViewportCommand::Decorations(false));
        Some(Self { hwnd, regions })
    }

    pub fn show(&self, ui: &mut egui::Ui, excluded: [Rect; 2]) {
        let room = ui.ctx().content_rect();
        let controls = std::array::from_fn(|index| {
            Rect::from_min_size(
                egui::pos2(
                    room.right() - CONTROL_WIDTH + index as f32 * 46.0,
                    room.top(),
                ),
                Vec2::new(46.0, 40.0),
            )
        });
        let scale = ui.ctx().pixels_per_point();
        let fullscreen = ui.input(|i| i.viewport().fullscreen.unwrap_or(false));
        if let Ok(mut regions) = self.regions.lock() {
            *regions = Regions {
                toolbar: Rect::from_min_size(room.min, Vec2::new(room.width(), 40.0)),
                excluded,
                controls,
                scale,
                drag_enabled: !egui::Popup::is_any_open(ui.ctx()),
                fullscreen,
            };
        }
        let maximized = unsafe { IsZoomed(self.hwnd).as_bool() };
        let mut cursor = POINT::default();
        let cursor = if unsafe {
            GetCursorPos(&mut cursor).is_ok() && ScreenToClient(self.hwnd, &mut cursor).as_bool()
        } {
            Some(egui::pos2(cursor.x as f32 / scale, cursor.y as f32 / scale))
        } else {
            None
        };
        for (index, rect) in controls.into_iter().enumerate() {
            let label = match index {
                0 => "Minimize",
                1 if maximized => "Restore",
                1 => "Maximize",
                _ => "Close window",
            };
            let response = ui.interact(
                rect,
                ui.id().with(("window-control", index)),
                egui::Sense::click(),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, crate::i18n::tr(label))
            });
            let hovered = cursor.is_some_and(|point| rect.contains(point)) || response.has_focus();
            let painter = ui.painter().with_clip_rect(rect);
            if hovered {
                painter.rect_filled(
                    rect,
                    0.0,
                    if index == 2 {
                        egui::Color32::from_rgb(196, 43, 28)
                    } else {
                        ui.visuals().widgets.hovered.bg_fill
                    },
                );
            }
            let color = if hovered && index == 2 {
                egui::Color32::WHITE
            } else {
                ui.visuals().text_color()
            };
            let stroke = egui::Stroke::new(1.0, color);
            let glyph = Rect::from_center_size(rect.center(), Vec2::splat(10.0));
            match index {
                0 => {
                    painter.hline(glyph.x_range(), glyph.center().y, stroke);
                }
                1 if maximized => {
                    let back = glyph.translate(egui::vec2(1.5, -1.5)).shrink(1.0);
                    painter.line_segment([back.left_top(), back.right_top()], stroke);
                    painter.line_segment([back.right_top(), back.right_bottom()], stroke);
                    painter.rect_stroke(
                        glyph.translate(egui::vec2(-1.0, 1.0)).shrink(1.0),
                        0.0,
                        stroke,
                        egui::StrokeKind::Inside,
                    );
                }
                1 => {
                    painter.rect_stroke(glyph, 0.0, stroke, egui::StrokeKind::Inside);
                }
                _ => {
                    painter.line_segment([glyph.left_top(), glyph.right_bottom()], stroke);
                    painter.line_segment([glyph.right_top(), glyph.left_bottom()], stroke);
                }
            }
            // Pointer actions go through WM_NCHITTEST and the system's non-client handler.
            // This path supports keyboard/accessibility activation of the same controls.
            if response.clicked() {
                ui.ctx().send_viewport_cmd(match index {
                    0 => egui::ViewportCommand::Minimized(true),
                    1 => egui::ViewportCommand::Maximized(!maximized),
                    _ => egui::ViewportCommand::Close,
                });
            }
            response.on_hover_text(crate::i18n::tr(label));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_hit_regions_preserve_controls_resize_and_interactive_toolbar() {
        let bounds = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 520.0));
        let mut regions = Regions {
            toolbar: Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 40.0)),
            excluded: [
                Rect::from_min_size(egui::pos2(200.0, 8.0), Vec2::new(200.0, 26.0)),
                Rect::from_min_size(egui::pos2(410.0, 8.0), Vec2::new(250.0, 26.0)),
            ],
            controls: std::array::from_fn(|i| {
                Rect::from_min_size(
                    egui::pos2(662.0 + i as f32 * 46.0, 0.0),
                    Vec2::new(46.0, 40.0),
                )
            }),
            drag_enabled: true,
            ..Default::default()
        };
        assert_eq!(
            regions.hit(egui::pos2(30.0, 20.0), bounds, false),
            HTCAPTION
        );
        assert_eq!(
            regions.hit(egui::pos2(240.0, 20.0), bounds, false),
            HTCLIENT
        );
        assert_eq!(
            regions.hit(egui::pos2(440.0, 20.0), bounds, false),
            HTCLIENT
        );
        for (rect, hit) in regions
            .controls
            .iter()
            .zip([HTMINBUTTON, HTMAXBUTTON, HTCLOSE])
        {
            assert_eq!(regions.hit(rect.center(), bounds, false), hit);
        }
        assert_eq!(regions.hit(egui::pos2(1.0, 1.0), bounds, false), HTTOPLEFT);
        assert_eq!(
            regions.hit(egui::pos2(799.0, 519.0), bounds, false),
            HTBOTTOMRIGHT
        );
        assert_eq!(
            regions.hit(egui::pos2(799.0, 519.0), bounds, true),
            HTCLIENT
        );
        regions.drag_enabled = false;
        assert_eq!(regions.hit(egui::pos2(30.0, 20.0), bounds, false), HTCLIENT);
        regions.fullscreen = true;
        assert_eq!(
            regions.hit(regions.controls[2].center(), bounds, true),
            HTCLIENT
        );
    }
}
