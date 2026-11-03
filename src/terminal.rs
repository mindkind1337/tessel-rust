mod mouse;
use crate::i18n::tr;
use eframe::egui;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::{
    io::{Read, Write},
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
};

pub struct Terminal {
    master: Box<dyn MasterPty + Send>,
    input: Option<SyncSender<Vec<u8>>>,
    child: Box<dyn Child + Send + Sync>,
    output: Receiver<Vec<u8>>,
    parser: vt100::Parser,
    size: (u16, u16),
    status: String,
    query_tail: Vec<u8>,
    selection: Option<Selection>,
    composition: String,
    composing: bool,
    mouse: mouse::Mouse,
    widget_id: Option<egui::Id>,
    #[cfg(feature = "visual-test")]
    fixture: bool,
}

impl Terminal {
    pub fn output(&self) -> String {
        self.parser.screen().contents()
    }
    pub fn request_focus(&self, ctx: &egui::Context) {
        if let Some(id) = self.widget_id {
            ctx.memory_mut(|m| m.request_focus(id));
        }
    }
    pub fn has_focus(&self, ctx: &egui::Context) -> bool {
        self.widget_id
            .is_some_and(|id| ctx.memory(|m| m.has_focus(id)))
    }
    #[cfg(feature = "visual-test")]
    pub fn visual_fixture() -> Result<Self, Box<dyn std::error::Error>> {
        let command = crate::launch::command(
            &crate::launch::Launch::Shell,
            &crate::launch::default_directory(),
        )?;
        let mut terminal = Self::spawn_command(command)?;
        terminal.process_output("\x1b[2J\x1b[HTessel Rust terminal fixture\r\nFrançais : Québec, été, déjà.\r\nWide cells: A🙂B🙂C\r\n\x1b[44m Blue background 🙂 continues \x1b[0m\r\n\x1b[4mUnderlined text and 🙂\x1b[0m\r\n\x1b[32mGreen foreground\x1b[0m\r\nCtrl+A-Z / Shift+Tab / F1-F12 / modified navigation".as_bytes());
        terminal.fixture = true;
        Ok(terminal)
    }
    #[cfg(test)]
    pub fn spawn() -> Result<Self, Box<dyn std::error::Error>> {
        let command = crate::launch::command(
            &crate::launch::Launch::Shell,
            &crate::launch::default_directory(),
        )?;
        Self::spawn_command(command)
    }

    pub fn spawn_command(command: CommandBuilder) -> Result<Self, Box<dyn std::error::Error>> {
        let pair = native_pty_system().openpty(PtySize {
            rows: 30,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let mut reader = pair.master.try_clone_reader()?;
        let mut writer = pair.master.take_writer()?;
        let child = pair.slave.spawn_command(command)?;
        drop(pair.slave);
        // Bounded queue prevents a noisy subprocess from consuming unlimited memory.
        let (sender, output) = mpsc::sync_channel(64);
        let (input, input_rx) = mpsc::sync_channel::<Vec<u8>>(32);
        thread::spawn(move || {
            while let Ok(bytes) = input_rx.recv() {
                if writer
                    .write_all(&bytes)
                    .and_then(|_| writer.flush())
                    .is_err()
                {
                    break;
                }
            }
        });
        thread::spawn(move || {
            let mut buffer = [0; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        if sender.send(buffer[..count].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Ok(Self {
            master: pair.master,
            input: Some(input),
            child,
            output,
            parser: vt100::Parser::new(30, 100, 1000),
            size: (30, 100),
            status: String::new(),
            query_tail: Vec::new(),
            selection: None,
            composition: String::new(),
            composing: false,
            mouse: Default::default(),
            widget_id: None,
            #[cfg(feature = "visual-test")]
            fixture: false,
        })
    }

    fn process_output(&mut self, bytes: &[u8]) {
        if !bytes.is_empty() {
            self.selection = None;
        }
        for &byte in bytes {
            self.parser.process(&[byte]);
            self.query_tail.push(byte);
            if self.query_tail.ends_with(b"\x1b[6n") {
                let (row, col) = self.parser.screen().cursor_position();
                let reply = format!("\x1b[{};{}R", row + 1, col + 1);
                if let Err(error) = self.send(reply.into_bytes()) {
                    self.status = error;
                }
            }
            if self.query_tail.len() > 4 {
                self.query_tail.remove(0);
            }
        }
    }

    fn send(&self, bytes: Vec<u8>) -> Result<(), String> {
        self.input
            .as_ref()
            .ok_or("Terminal input closed")?
            .try_send(bytes)
            .map_err(|_| "Terminal input is busy or closed; input was not sent.".into())
    }

    pub fn poll(&mut self) {
        #[cfg(feature = "visual-test")]
        if self.fixture {
            return;
        }
        for _ in 0..64 {
            let Ok(bytes) = self.output.try_recv() else {
                break;
            };
            self.process_output(&bytes);
        }
        if let Ok(Some(exit)) = self.child.try_wait() {
            self.status = format!("Shell exited: {exit}");
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, font_size: f32) {
        if !self.status.is_empty() {
            ui.colored_label(egui::Color32::LIGHT_RED, &self.status);
        }
        let font = egui::FontId::monospace(font_size);
        let cell_width = ui.fonts_mut(|f| f.glyph_width(&font, 'M'));
        let line_height = ui.fonts_mut(|f| f.row_height(&font));
        let available = ui.available_size();
        let cols = ((available.x / cell_width) as u16).clamp(20, 300);
        let rows = ((available.y / line_height) as u16).clamp(5, 150);
        if self.size != (rows, cols)
            && self
                .master
                .resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .is_ok()
        {
            self.selection = None;
            self.parser.set_size(rows, cols);
            self.size = (rows, cols);
        }
        let (rect, response) = ui.allocate_exact_size(available, egui::Sense::click_and_drag());
        self.widget_id = Some(response.id);
        if ui.is_enabled()
            && (response.clicked()
                || response.drag_started()
                || response.contains_pointer() && ui.input(|i| i.pointer.any_pressed()))
        {
            response.request_focus();
        }
        if response.has_focus() && ui.is_enabled() {
            ui.memory_mut(|memory| {
                memory.set_focus_lock_filter(
                    response.id,
                    egui::EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                    },
                )
            });
        }
        #[cfg(feature = "visual-test")]
        if self.fixture {
            self.selection = Some(Selection {
                start: (1, 0),
                end: (1, 9),
            });
        }
        if response.lost_focus() {
            self.composition.clear();
            self.composing = false;
        }
        let mode = if self.parser.screen().scrollback() == 0 {
            self.parser.screen().mouse_protocol_mode()
        } else {
            vt100::MouseProtocolMode::None
        };
        let shift = ui.input(|i| i.modifiers.shift);
        let tracking = mode != vt100::MouseProtocolMode::None;
        let mouse_bytes = if !ui.is_enabled() {
            self.mouse
                .release(self.parser.screen().mouse_protocol_encoding())
        } else {
            ui.input(|i| {
                self.mouse.events(
                    &i.events,
                    &mouse::Area {
                        rect,
                        cell: egui::vec2(cell_width, line_height),
                        size: self.size,
                        hovered: response.contains_pointer(),
                        pointer: i.pointer.hover_pos(),
                        modifiers: i.modifiers,
                    },
                    mode,
                    self.parser.screen().mouse_protocol_encoding(),
                )
            })
        };
        if !mouse_bytes.is_empty() {
            self.selection = None;
            if let Err(e) = self.send(mouse_bytes) {
                self.status = e;
            }
        }
        let cell_at = |pos: egui::Pos2| {
            (
                ((pos.y - rect.min.y) / line_height)
                    .floor()
                    .clamp(0.0, f32::from(rows - 1)) as u16,
                ((pos.x - rect.min.x) / cell_width)
                    .round()
                    .clamp(0.0, f32::from(cols)) as u16,
            )
        };
        if (!tracking || shift)
            && response.drag_started_by(egui::PointerButton::Primary)
            && let Some(pos) = ui.input(|i| i.pointer.press_origin())
        {
            let start = cell_at(pos);
            self.selection = Some(Selection { start, end: start });
        }
        if (!tracking || shift)
            && (response.dragged_by(egui::PointerButton::Primary)
                || response.drag_stopped_by(egui::PointerButton::Primary))
            && let (Some(selection), Some(pos)) =
                (&mut self.selection, response.interact_pointer_pos())
        {
            selection.end = cell_at(pos);
        }
        if response.clicked() {
            self.selection = None;
        }
        if (!tracking || shift) && response.hovered() {
            let delta = ui.input(|i| i.smooth_scroll_delta.y);
            if delta != 0.0 {
                let release = self
                    .mouse
                    .release(self.parser.screen().mouse_protocol_encoding());
                if !release.is_empty()
                    && let Err(e) = self.send(release)
                {
                    self.status = e;
                }
                let scroll =
                    self.parser.screen().scrollback() as i64 + (delta / line_height).round() as i64;
                self.selection = None;
                self.parser.set_scrollback(scroll.clamp(0, 1000) as usize);
            }
        }
        if !tracking || shift {
            response.context_menu(|ui| {
                if self.selection.is_some() && ui.button(tr("Copy selection")).clicked() {
                    if let Some(selection) = self.selection {
                        ui.ctx().copy_text(selection.text(self.parser.screen()));
                    }
                    ui.close();
                }

                if ui.button(tr("Copy visible text")).clicked() {
                    ui.ctx().copy_text(self.parser.screen().contents());
                    ui.close();
                }
                if ui.button(tr("Back to live output")).clicked() {
                    self.parser.set_scrollback(0);
                    ui.close();
                }
            });
        }
        let screen = self.parser.screen();
        let palette = crate::theme::terminal(ui.ctx());
        let painter = ui.painter().with_clip_rect(rect);
        let default_bg = color(vt100::Color::Default, false, palette);
        painter.rect_filled(rect, 0.0, default_bg);
        // Paint every background before any glyph. Otherwise the continuation cell
        // erases the right half of a wide character painted immediately before it.
        for row in 0..rows {
            for col in 0..cols {
                if let Some(cell) = screen.cell(row, col) {
                    if cell.is_wide_continuation() {
                        continue;
                    }
                    let bg = if cell.inverse() {
                        color(cell.fgcolor(), true, palette)
                    } else {
                        color(cell.bgcolor(), false, palette)
                    };
                    if bg != default_bg {
                        let pos = rect.min
                            + egui::vec2(col as f32 * cell_width, row as f32 * line_height);
                        painter.rect_filled(
                            egui::Rect::from_min_size(
                                pos,
                                egui::vec2(
                                    cell_width * if cell.is_wide() { 2.0 } else { 1.0 },
                                    line_height,
                                ),
                            ),
                            0.0,
                            bg,
                        );
                    }
                }
            }
        }
        if let Some(selection) = self.selection {
            let (start, end) = selection.ordered();
            for row in start.0..=end.0 {
                let left = if row == start.0 { start.1 } else { 0 };
                let right = if row == end.0 { end.1 } else { cols };
                painter.rect_filled(
                    egui::Rect::from_min_max(
                        rect.min
                            + egui::vec2(
                                f32::from(left) * cell_width,
                                f32::from(row) * line_height,
                            ),
                        rect.min
                            + egui::vec2(
                                f32::from(right) * cell_width,
                                f32::from(row + 1) * line_height,
                            ),
                    ),
                    0.0,
                    palette.selection_background,
                );
            }
        }
        for row in 0..rows {
            for col in 0..cols {
                if let Some(cell) = screen.cell(row, col) {
                    if cell.is_wide_continuation() {
                        continue;
                    }
                    let text = cell.contents();
                    let pos =
                        rect.min + egui::vec2(col as f32 * cell_width, row as f32 * line_height);
                    let fg = if cell.inverse() {
                        color(cell.bgcolor(), false, palette)
                    } else {
                        color(cell.fgcolor(), true, palette)
                    };
                    if !text.is_empty() && text != " " {
                        painter.text(pos, egui::Align2::LEFT_TOP, text, font.clone(), fg);
                    }
                    if cell.underline() {
                        let start = pos + egui::vec2(0.0, line_height - 2.0);
                        painter.line_segment(
                            [
                                start,
                                start
                                    + egui::vec2(
                                        cell_width * if cell.is_wide() { 2.0 } else { 1.0 },
                                        0.0,
                                    ),
                            ],
                            (1.0, fg),
                        );
                    }
                }
            }
        }
        if response.has_focus() && ui.is_enabled() {
            let (row, col) = screen.cursor_position();
            if !screen.hide_cursor() && screen.scrollback() == 0 {
                let pos = rect.min
                    + egui::vec2(
                        col as f32 * cell_width,
                        (row as f32 + 1.0) * line_height - 2.0,
                    );
                painter.line_segment(
                    [pos, pos + egui::vec2(cell_width, 0.0)],
                    (2.0, palette.cursor),
                );
            }
            let cursor = egui::Rect::from_min_size(
                rect.min + egui::vec2(f32::from(col) * cell_width, f32::from(row) * line_height),
                egui::vec2(cell_width, line_height),
            );
            ui.output_mut(|o| {
                o.ime = Some(egui::output::IMEOutput {
                    rect,
                    cursor_rect: cursor,
                })
            });
            if !self.composition.is_empty() {
                let text =
                    painter.layout_no_wrap(self.composition.clone(), font.clone(), palette.cursor);
                painter.rect_filled(
                    egui::Rect::from_min_size(cursor.min, text.size()),
                    0.0,
                    default_bg,
                );
                painter.galley(cursor.min, text, palette.cursor);
            }
            for event in ui.input(|i| i.events.clone()) {
                let bytes = match event {
                    egui::Event::Text(text) if !self.composing => text.into_bytes(),
                    egui::Event::Ime(event) => match event {
                        egui::ImeEvent::Enabled => {
                            self.composing = true;
                            Vec::new()
                        }
                        egui::ImeEvent::Preedit(text) => {
                            self.composing = true;
                            self.composition = text.chars().take(1024).collect();
                            Vec::new()
                        }
                        egui::ImeEvent::Commit(text) => {
                            self.composing = false;
                            self.composition.clear();
                            if text.len() <= 4096 {
                                text.into_bytes()
                            } else {
                                self.status = "Composition limit: 4 KiB".into();
                                Vec::new()
                            }
                        }
                        egui::ImeEvent::Disabled => {
                            self.composing = false;
                            self.composition.clear();
                            Vec::new()
                        }
                    },
                    egui::Event::Copy => {
                        if let Some(selection) = self.selection.filter(|s| s.start != s.end) {
                            ui.ctx().copy_text(selection.text(self.parser.screen()));
                            Vec::new()
                        } else if ui.input(|i| i.modifiers.shift) {
                            ui.ctx().copy_text(self.parser.screen().contents());
                            Vec::new()
                        } else {
                            vec![3]
                        }
                    }
                    egui::Event::Paste(text) => {
                        if text.len() > 65536 {
                            self.status = "Paste limit: 64 KiB".into();
                            Vec::new()
                        } else if self.parser.screen().bracketed_paste() {
                            format!("\x1b[200~{text}\x1b[201~").into_bytes()
                        } else {
                            text.into_bytes()
                        }
                    }
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => {
                        if self.composing {
                            continue;
                        }
                        if key == egui::Key::C
                            && modifiers.ctrl
                            && (modifiers.shift || self.selection.is_some_and(|s| s.start != s.end))
                        {
                            ui.ctx().copy_text(
                                self.selection
                                    .map(|s| s.text(self.parser.screen()))
                                    .unwrap_or_else(|| self.parser.screen().contents()),
                            );
                            continue;
                        }
                        let mut bytes = key_bytes(key, modifiers);
                        if self.parser.screen().application_cursor()
                            && bytes.len() == 3
                            && bytes[0] == 27
                            && matches!(bytes[2], b'A'..=b'D' | b'H' | b'F')
                        {
                            bytes[1] = b'O';
                        }
                        bytes
                    }
                    _ => Vec::new(),
                };
                if !bytes.is_empty() {
                    self.selection = None;
                    self.parser.set_scrollback(0);
                }
                if !bytes.is_empty()
                    && let Err(error) = self.send(bytes)
                {
                    self.status = error.to_string();
                }
            }
        }
        if !self.status.is_empty() {
            response.on_hover_text(&self.status);
        }
    }
}

#[derive(Clone, Copy)]
struct Selection {
    start: (u16, u16),
    end: (u16, u16),
}
impl Selection {
    fn ordered(self) -> ((u16, u16), (u16, u16)) {
        if self.start <= self.end {
            (self.start, self.end)
        } else {
            (self.end, self.start)
        }
    }
    fn text(self, screen: &vt100::Screen) -> String {
        let (start, end) = self.ordered();
        screen.contents_between(start.0, start.1, end.0, end.1)
    }
}

fn key_bytes(key: egui::Key, modifiers: egui::Modifiers) -> Vec<u8> {
    use egui::Key::*;
    if modifiers.ctrl {
        let letters = [
            A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
        ];
        if let Some(index) = letters.iter().position(|k| *k == key) {
            return vec![(index + 1) as u8];
        }
        if key == Space {
            return vec![0];
        }
    }
    let modifier =
        1 + u8::from(modifiers.shift) + 2 * u8::from(modifiers.alt) + 4 * u8::from(modifiers.ctrl);
    let final_byte = match key {
        ArrowUp => Some('A'),
        ArrowDown => Some('B'),
        ArrowRight => Some('C'),
        ArrowLeft => Some('D'),
        Home => Some('H'),
        End => Some('F'),
        _ => None,
    };
    if let Some(end) = final_byte {
        return if modifier == 1 {
            format!("\x1b[{end}")
        } else {
            format!("\x1b[1;{modifier}{end}")
        }
        .into_bytes();
    }
    let function = match key {
        F1 => Some('P'),
        F2 => Some('Q'),
        F3 => Some('R'),
        F4 => Some('S'),
        _ => None,
    };
    if let Some(end) = function {
        return if modifier == 1 {
            format!("\x1bO{end}")
        } else {
            format!("\x1b[1;{modifier}{end}")
        }
        .into_bytes();
    }
    let tilde = match key {
        Insert => Some(2),
        Delete => Some(3),
        PageUp => Some(5),
        PageDown => Some(6),
        F5 => Some(15),
        F6 => Some(17),
        F7 => Some(18),
        F8 => Some(19),
        F9 => Some(20),
        F10 => Some(21),
        F11 => Some(23),
        F12 => Some(24),
        _ => None,
    };
    if let Some(code) = tilde {
        return if modifier == 1 {
            format!("\x1b[{code}~")
        } else {
            format!("\x1b[{code};{modifier}~")
        }
        .into_bytes();
    }
    match key {
        Enter => vec![13],
        Backspace => vec![127],
        Tab if modifiers.shift => b"\x1b[Z".to_vec(),
        Tab => vec![9],
        Escape => vec![27],
        _ => vec![],
    }
}

fn color(
    value: vt100::Color,
    foreground: bool,
    palette: &crate::theme_palettes::TerminalPalette,
) -> egui::Color32 {
    match value {
        vt100::Color::Default => {
            if foreground {
                palette.foreground
            } else {
                palette.background
            }
        }
        vt100::Color::Rgb(r, g, b) => egui::Color32::from_rgb(r, g, b),
        vt100::Color::Idx(index) => {
            if index < 16
                && let Some(ansi) = palette.ansi
            {
                return ansi.as_array()[index as usize];
            }
            const BASE: [[u8; 3]; 16] = [
                [0, 0, 0],
                [205, 49, 49],
                [13, 188, 121],
                [229, 229, 16],
                [36, 114, 200],
                [188, 63, 188],
                [17, 168, 205],
                [229, 229, 229],
                [102, 102, 102],
                [241, 76, 76],
                [35, 209, 139],
                [245, 245, 67],
                [59, 142, 234],
                [214, 112, 214],
                [41, 184, 219],
                [255, 255, 255],
            ];
            let [r, g, b] = if index < 16 {
                BASE[index as usize]
            } else if index >= 232 {
                [8 + (index - 232) * 10; 3]
            } else {
                let n = index - 16;
                let component = |v| if v == 0 { 0 } else { 55 + v * 40 };
                [component(n / 36), component((n / 6) % 6), component(n % 6)]
            };
            egui::Color32::from_rgb(r, g, b)
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.child.kill();
        self.input.take();
        // Unblock the reader before closing ConPTY; a full output queue must not
        // deadlock window shutdown while ClosePseudoConsole waits for its pipes.
        let (_, empty) = mpsc::channel();
        self.output = empty;
        let _ = self.child.wait();
    }
}

#[cfg(windows)]
pub fn configure_fonts(ctx: &egui::Context) {
    use std::io::Read;
    let windows = std::env::var_os("WINDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "C:/Windows".into());
    let mut fonts = egui::FontDefinitions::default();
    for (name, file) in [
        ("windows-ui", "segoeui.ttf"),
        ("windows-emoji", "seguiemj.ttf"),
        ("windows-symbols", "seguisym.ttf"),
    ] {
        let Ok(file) = std::fs::File::open(windows.join("Fonts").join(file)) else {
            continue;
        };
        let mut bytes = Vec::new();
        if file
            .take(32 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .is_err()
            || bytes.len() > 32 * 1024 * 1024
        {
            continue;
        }
        fonts.font_data.insert(
            name.into(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes)),
        );
        if name == "windows-ui" {
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, name.into());
            continue;
        }
        for family in [egui::FontFamily::Monospace, egui::FontFamily::Proportional] {
            fonts.families.entry(family).or_default().push(name.into());
        }
    }
    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn theme_switches_defaults_and_ansi_without_rewriting_truecolor_or_extended_colors() {
        use crate::theme_palettes::ThemeId;
        let classic = &ThemeId::Classic.spec().term;
        let warp = &ThemeId::Warp.spec().term;
        let dracula = &ThemeId::Dracula.spec().term;
        assert_ne!(
            color(vt100::Color::Default, false, classic),
            color(vt100::Color::Default, false, warp)
        );
        for index in 0..=255 {
            assert_eq!(
                color(vt100::Color::Idx(index), true, classic),
                color(vt100::Color::Idx(index), true, warp)
            );
            if index >= 16 {
                assert_eq!(
                    color(vt100::Color::Idx(index), true, classic),
                    color(vt100::Color::Idx(index), true, dracula)
                );
            }
        }
        assert_eq!(
            color(vt100::Color::Idx(1), true, dracula),
            dracula.ansi.unwrap().red
        );
        assert_eq!(
            color(vt100::Color::Rgb(7, 11, 13), true, dracula),
            egui::Color32::from_rgb(7, 11, 13)
        );
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx, true, Some(ThemeId::Nord));
        assert_eq!(crate::theme::terminal(&ctx), &ThemeId::Nord.spec().term);
        crate::theme::apply(&ctx, true, Some(ThemeId::Gruvbox));
        assert_eq!(crate::theme::terminal(&ctx), &ThemeId::Gruvbox.spec().term);
    }
    #[test]
    fn disabled_terminal_cannot_receive_modal_typing_or_mouse_clicks() {
        let mut terminal = Terminal::spawn().unwrap();
        let (sender, receiver) = mpsc::sync_channel(32);
        terminal.input = Some(sender);
        terminal.process_output(b"\x1b[?1000h\x1b[?1006h");
        let ctx = egui::Context::default();
        let mut render = |enabled: bool, events: Vec<egui::Event>| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 300.0),
                    )),
                    events,
                    focused: true,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        if !enabled {
                            ui.disable();
                        }
                        terminal.show(ui, 14.0);
                    });
                },
            );
            terminal.has_focus(&ctx)
        };
        render(true, vec![]);
        let pos = egui::pos2(50.0, 50.0);
        let click = vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ];
        assert!(render(true, click.clone()));
        let packets: Vec<Vec<u8>> = receiver.try_iter().collect();
        assert!(packets.iter().any(|packet| packet.starts_with(b"\x1b[<")));
        render(true, vec![egui::Event::Text("enabled".into())]);
        assert_eq!(receiver.try_recv().unwrap(), b"enabled");
        render(
            false,
            vec![
                egui::Event::Text("private modal input".into()),
                egui::Event::Paste("paste".into()),
            ],
        );
        render(false, click);
        assert!(receiver.try_recv().is_err());
        ctx.memory_mut(|m| m.request_focus(egui::Id::new("settings-text-field")));
        assert!(!terminal.has_focus(&ctx));
    }
    #[test]
    fn interrupt_and_navigation_are_terminal_sequences() {
        assert_eq!(key_bytes(egui::Key::C, egui::Modifiers::CTRL), vec![3]);
        assert_eq!(key_bytes(egui::Key::Enter, egui::Modifiers::NONE), b"\r");
        assert_eq!(
            key_bytes(egui::Key::ArrowUp, egui::Modifiers::NONE),
            b"\x1b[A"
        );
        assert_eq!(key_bytes(egui::Key::R, egui::Modifiers::CTRL), vec![18]);
        assert_eq!(key_bytes(egui::Key::Z, egui::Modifiers::CTRL), vec![26]);
        assert_eq!(key_bytes(egui::Key::Space, egui::Modifiers::CTRL), vec![0]);
        assert_eq!(key_bytes(egui::Key::Tab, egui::Modifiers::SHIFT), b"\x1b[Z");
        assert_eq!(
            key_bytes(egui::Key::ArrowLeft, egui::Modifiers::CTRL),
            b"\x1b[1;5D"
        );
        assert_eq!(key_bytes(egui::Key::F1, egui::Modifiers::NONE), b"\x1bOP");
        assert_eq!(
            key_bytes(egui::Key::F12, egui::Modifiers::NONE),
            b"\x1b[24~"
        );
        assert_eq!(
            key_bytes(egui::Key::PageDown, egui::Modifiers::SHIFT),
            b"\x1b[6;2~"
        );
    }
    #[test]
    fn selection_preserves_unicode_and_wrapped_lines_in_both_directions() {
        let mut parser = vt100::Parser::new(5, 10, 0);
        parser.process("A🙂B\r\nQuébec\r\n1234567890abc".as_bytes());
        assert_eq!(
            Selection {
                start: (0, 0),
                end: (0, 4)
            }
            .text(parser.screen()),
            "A🙂B"
        );
        assert_eq!(
            Selection {
                start: (1, 6),
                end: (0, 0)
            }
            .text(parser.screen()),
            "A🙂B\nQuébec"
        );
        assert_eq!(
            Selection {
                start: (2, 0),
                end: (3, 3)
            }
            .text(parser.screen()),
            "1234567890abc"
        );
        assert_eq!(
            Selection {
                start: (0, 1),
                end: (0, 1)
            }
            .text(parser.screen()),
            ""
        );
    }
    #[test]
    fn shell_really_runs_in_a_pty() {
        let mut terminal = Terminal::spawn().expect("create PTY");
        terminal.send(b"echo TESSEL_PTY_OK\r".to_vec()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut found = false;
        while std::time::Instant::now() < deadline {
            if let Ok(bytes) = terminal
                .output
                .recv_timeout(std::time::Duration::from_millis(200))
            {
                terminal.process_output(&bytes);
                // Require output on its own line, not just the echoed command.
                found = terminal
                    .parser
                    .screen()
                    .contents()
                    .lines()
                    .any(|line| line.trim() == "TESSEL_PTY_OK");
                if found {
                    break;
                }
            }
        }
        assert!(found, "shell did not execute command");
    }
}
