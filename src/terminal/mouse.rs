//! Xterm mouse input. Shift leaves selection and scrollback with the owner.
use eframe::egui;
use vt100::{MouseProtocolEncoding as Encoding, MouseProtocolMode as Mode};
#[derive(Default)]
pub(super) struct Mouse {
    held: u8,
    last: Option<(u16, u16)>,
    mode: Mode,
}
pub(super) struct Area {
    pub rect: egui::Rect,
    pub cell: egui::Vec2,
    pub size: (u16, u16),
    pub hovered: bool,
    pub pointer: Option<egui::Pos2>,
    pub modifiers: egui::Modifiers,
}
impl Area {
    fn position(&self, pos: egui::Pos2) -> (u16, u16) {
        (
            ((pos.x - self.rect.min.x) / self.cell.x)
                .floor()
                .clamp(0.0, f32::from(self.size.1 - 1)) as u16
                + 1,
            ((pos.y - self.rect.min.y) / self.cell.y)
                .floor()
                .clamp(0.0, f32::from(self.size.0 - 1)) as u16
                + 1,
        )
    }
}
impl Mouse {
    pub fn release(&mut self, encoding: Encoding) -> Vec<u8> {
        let mut output = Vec::new();
        if let Some(position) = self.last
            && !matches!(self.mode, Mode::None | Mode::Press)
        {
            for button in 0..3 {
                if self.held & (1 << button) != 0 {
                    output.extend(packet(
                        button,
                        position,
                        true,
                        egui::Modifiers::NONE,
                        self.mode,
                        encoding,
                    ));
                }
            }
        }
        self.held = 0;
        self.last = None;
        output
    }
    pub fn events(
        &mut self,
        events: &[egui::Event],
        area: &Area,
        mode: Mode,
        encoding: Encoding,
    ) -> Vec<u8> {
        if mode != self.mode {
            self.held = 0;
            self.last = None;
            self.mode = mode;
        }
        if mode == Mode::None {
            return Vec::new();
        }
        let mut output = Vec::new();
        for event in events {
            match event {
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers,
                } => {
                    let Some(button) = (match button {
                        egui::PointerButton::Primary => Some(0),
                        egui::PointerButton::Middle => Some(1),
                        egui::PointerButton::Secondary => Some(2),
                        _ => None,
                    }) else {
                        continue;
                    };
                    let position = area.position(*pos);
                    if *pressed {
                        if !area.hovered || !area.rect.contains(*pos) || modifiers.shift {
                            continue;
                        }
                        self.held |= 1 << button;
                        self.last = Some(position);
                        output.extend(packet(button, position, false, *modifiers, mode, encoding));
                    } else if self.held & (1 << button) != 0 {
                        self.held &= !(1 << button);
                        self.last = Some(position);
                        if mode != Mode::Press {
                            output
                                .extend(packet(button, position, true, *modifiers, mode, encoding));
                        }
                    }
                }
                egui::Event::PointerMoved(pos) => {
                    let position = area.position(*pos);
                    if area.modifiers.shift
                        || !area.hovered
                        || !area.rect.contains(*pos)
                        || self.last == Some(position)
                    {
                        continue;
                    }
                    self.last = Some(position);
                    if output.len() < 2048
                        && (mode == Mode::AnyMotion || mode == Mode::ButtonMotion && self.held != 0)
                    {
                        output.extend(packet(
                            32 + if self.held == 0 {
                                3
                            } else {
                                self.held.trailing_zeros() as u8
                            },
                            position,
                            false,
                            area.modifiers,
                            mode,
                            encoding,
                        ));
                    }
                }
                egui::Event::MouseWheel {
                    unit,
                    delta,
                    modifiers,
                } if area.hovered && !modifiers.shift && mode != Mode::Press => {
                    let Some(pos) = area.pointer.filter(|p| area.rect.contains(*p)) else {
                        continue;
                    };
                    if delta.y == 0.0 || output.len() > 2048 {
                        continue;
                    }
                    let steps = match unit {
                        egui::MouseWheelUnit::Line => delta.y.abs(),
                        egui::MouseWheelUnit::Point => delta.y.abs() / area.cell.y,
                        egui::MouseWheelUnit::Page => delta.y.abs() * f32::from(area.size.0),
                    }
                    .ceil()
                    .clamp(1.0, 12.0) as usize;
                    for _ in 0..steps {
                        output.extend(packet(
                            if delta.y > 0.0 { 64 } else { 65 },
                            area.position(pos),
                            false,
                            *modifiers,
                            mode,
                            encoding,
                        ));
                    }
                }
                egui::Event::WindowFocused(false) => {
                    if let Some(position) = self.last
                        && mode != Mode::Press
                    {
                        for button in 0..3 {
                            if self.held & (1 << button) != 0 {
                                output.extend(packet(
                                    button,
                                    position,
                                    true,
                                    egui::Modifiers::NONE,
                                    mode,
                                    encoding,
                                ));
                            }
                        }
                    }
                    self.held = 0;
                    self.last = None;
                }
                _ => {}
            }
        }
        output
    }
}
fn packet(
    mut button: u8,
    (x, y): (u16, u16),
    release: bool,
    modifiers: egui::Modifiers,
    mode: Mode,
    encoding: Encoding,
) -> Vec<u8> {
    if release && encoding != Encoding::Sgr {
        button = 3;
    }
    if mode != Mode::Press {
        button += u8::from(modifiers.shift) * 4
            + u8::from(modifiers.alt) * 8
            + u8::from(modifiers.ctrl) * 16;
    }
    if encoding == Encoding::Sgr {
        return format!("\x1b[<{button};{x};{y}{}", if release { 'm' } else { 'M' }).into_bytes();
    }
    if encoding == Encoding::Default {
        if x > 223 || y > 223 {
            return Vec::new();
        }
        return vec![27, b'[', b'M', button + 32, (x + 32) as u8, (y + 32) as u8];
    }
    let mut result = String::from("\x1b[M");
    for value in [u32::from(button) + 32, u32::from(x) + 32, u32::from(y) + 32] {
        let Some(ch) = char::from_u32(value) else {
            return Vec::new();
        };
        result.push(ch);
    }
    result.into_bytes()
}
#[cfg(test)]
mod tests {
    use super::*;
    fn area() -> Area {
        Area {
            rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 500.0)),
            cell: egui::vec2(10.0, 20.0),
            size: (25, 100),
            hovered: true,
            pointer: Some(egui::pos2(24.0, 29.0)),
            modifiers: egui::Modifiers::NONE,
        }
    }
    #[test]
    fn sgr_click_drag_release_and_shift_selection() {
        let mut parser = vt100::Parser::new(25, 100, 0);
        parser.process(b"\x1b[?1002h\x1b[?1006h");
        let mut mouse = Mouse::default();
        let area = area();
        let mode = parser.screen().mouse_protocol_mode();
        let enc = parser.screen().mouse_protocol_encoding();
        let press = |pressed, modifiers| egui::Event::PointerButton {
            pos: egui::pos2(24.0, 29.0),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        };
        assert_eq!(
            mouse.events(&[press(true, egui::Modifiers::SHIFT)], &area, mode, enc),
            b""
        );
        assert_eq!(
            mouse.events(
                &[
                    press(true, egui::Modifiers::NONE),
                    egui::Event::PointerMoved(egui::pos2(35.0, 49.0)),
                    press(false, egui::Modifiers::NONE)
                ],
                &area,
                mode,
                enc
            ),
            b"\x1b[<0;3;2M\x1b[<32;4;3M\x1b[<0;3;2m"
        );
        assert_eq!(
            mouse.events(
                &[egui::Event::PointerMoved(egui::pos2(45.0, 49.0))],
                &area,
                mode,
                enc
            ),
            b""
        );
    }
    #[test]
    fn legacy_utf8_modifiers_wheel_and_focus_release() {
        assert_eq!(
            packet(
                0,
                (1, 1),
                false,
                egui::Modifiers::CTRL,
                Mode::Press,
                Encoding::Default
            ),
            b"\x1b[M !!"
        );
        assert!(
            packet(
                0,
                (224, 1),
                false,
                egui::Modifiers::NONE,
                Mode::PressRelease,
                Encoding::Default
            )
            .is_empty()
        );
        assert_eq!(
            packet(
                0,
                (224, 1),
                false,
                egui::Modifiers::NONE,
                Mode::PressRelease,
                Encoding::Utf8
            ),
            "\x1b[M Ā!".as_bytes()
        );
        assert_eq!(
            packet(
                2,
                (9, 8),
                true,
                egui::Modifiers::CTRL,
                Mode::PressRelease,
                Encoding::Sgr
            ),
            b"\x1b[<18;9;8m"
        );
        let mut mouse = Mouse::default();
        let area = area();
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, -1.0),
            modifiers: egui::Modifiers::NONE,
        };
        assert_eq!(
            mouse.events(&[wheel], &area, Mode::PressRelease, Encoding::Sgr),
            b"\x1b[<65;3;2M"
        );
        let press = egui::Event::PointerButton {
            pos: egui::pos2(24.0, 29.0),
            button: egui::PointerButton::Secondary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        };
        assert_eq!(
            mouse.events(
                &[press, egui::Event::WindowFocused(false)],
                &area,
                Mode::PressRelease,
                Encoding::Sgr
            ),
            b"\x1b[<2;3;2M\x1b[<2;3;2m"
        );
    }
}
