use eframe::egui;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::{
    io::{Read, Write},
    sync::mpsc::{self, Receiver},
    thread,
};

pub struct Terminal {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    output: Receiver<Vec<u8>>,
    parser: vt100::Parser,
    size: (u16, u16),
    status: String,
    query_tail: Vec<u8>,
}

impl Terminal {
    pub fn spawn() -> Result<Self, Box<dyn std::error::Error>> {
        let pair = native_pty_system().openpty(PtySize {
            rows: 30,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let shell = if cfg!(windows) {
            "powershell.exe"
        } else {
            "/bin/sh"
        };
        let mut command = CommandBuilder::new(shell);
        if cfg!(windows) {
            command.arg("-NoLogo");
            command.arg("-NoProfile");
        }
        command.env("TERM", "xterm-256color");
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let child = pair.slave.spawn_command(command)?;
        drop(pair.slave);
        // Bounded queue prevents a noisy subprocess from consuming unlimited memory.
        let (sender, output) = mpsc::sync_channel(64);
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
            writer,
            child,
            output,
            parser: vt100::Parser::new(30, 100, 1000),
            size: (30, 100),
            status: String::new(),
            query_tail: Vec::new(),
        })
    }

    fn process_output(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.parser.process(&[byte]);
            self.query_tail.push(byte);
            if self.query_tail.ends_with(b"\x1b[6n") {
                let (row, col) = self.parser.screen().cursor_position();
                let reply = format!("\x1b[{};{}R", row + 1, col + 1);
                let _ = self.writer.write_all(reply.as_bytes());
                let _ = self.writer.flush();
            }
            if self.query_tail.len() > 4 {
                self.query_tail.remove(0);
            }
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        for _ in 0..64 {
            let Ok(bytes) = self.output.try_recv() else {
                break;
            };
            self.process_output(&bytes);
        }
        if let Ok(Some(exit)) = self.child.try_wait() {
            self.status = format!("Shell exited: {exit}");
        }
        let font = egui::FontId::monospace(15.0);
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
            self.parser.set_size(rows, cols);
            self.size = (rows, cols);
        }
        let (rect, response) = ui.allocate_exact_size(available, egui::Sense::click());
        if response.clicked() || ui.memory(|m| m.focused().is_none()) {
            response.request_focus();
        }
        let screen = self.parser.screen();
        for row in 0..rows {
            for col in 0..cols {
                if let Some(cell) = screen.cell(row, col) {
                    let pos =
                        rect.min + egui::vec2(col as f32 * cell_width, row as f32 * line_height);
                    let (mut fg, mut bg) =
                        (color(cell.fgcolor(), true), color(cell.bgcolor(), false));
                    if cell.inverse() {
                        std::mem::swap(&mut fg, &mut bg);
                    }
                    ui.painter().rect_filled(
                        egui::Rect::from_min_size(pos, egui::vec2(cell_width, line_height)),
                        0.0,
                        bg,
                    );
                    ui.painter().text(
                        pos,
                        egui::Align2::LEFT_TOP,
                        cell.contents(),
                        font.clone(),
                        fg,
                    );
                }
            }
        }
        if response.has_focus() {
            let (row, col) = screen.cursor_position();
            if !screen.hide_cursor() {
                let pos = rect.min
                    + egui::vec2(
                        col as f32 * cell_width,
                        (row as f32 + 1.0) * line_height - 2.0,
                    );
                ui.painter().line_segment(
                    [pos, pos + egui::vec2(cell_width, 0.0)],
                    (2.0, egui::Color32::WHITE),
                );
            }
            for event in ui.input(|i| i.events.clone()) {
                let bytes = match event {
                    egui::Event::Text(text) => text.into_bytes(),
                    egui::Event::Paste(text) => text.into_bytes(),
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => key_bytes(key, modifiers.ctrl),
                    _ => Vec::new(),
                };
                if !bytes.is_empty()
                    && let Err(error) = self
                        .writer
                        .write_all(&bytes)
                        .and_then(|_| self.writer.flush())
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

fn key_bytes(key: egui::Key, ctrl: bool) -> Vec<u8> {
    if ctrl {
        if key == egui::Key::C {
            return vec![3];
        }
        if key == egui::Key::D {
            return vec![4];
        }
        if key == egui::Key::L {
            return vec![12];
        }
    }
    match key {
        egui::Key::Enter => b"\r".to_vec(),
        egui::Key::Backspace => vec![127],
        egui::Key::Tab => vec![9],
        egui::Key::Escape => vec![27],
        egui::Key::ArrowUp => b"\x1b[A".to_vec(),
        egui::Key::ArrowDown => b"\x1b[B".to_vec(),
        egui::Key::ArrowRight => b"\x1b[C".to_vec(),
        egui::Key::ArrowLeft => b"\x1b[D".to_vec(),
        egui::Key::Home => b"\x1b[H".to_vec(),
        egui::Key::End => b"\x1b[F".to_vec(),
        egui::Key::Delete => b"\x1b[3~".to_vec(),
        _ => Vec::new(),
    }
}

fn color(value: vt100::Color, foreground: bool) -> egui::Color32 {
    match value {
        vt100::Color::Default => {
            if foreground {
                egui::Color32::from_rgb(220, 225, 235)
            } else {
                egui::Color32::from_rgb(20, 23, 29)
            }
        }
        vt100::Color::Rgb(r, g, b) => egui::Color32::from_rgb(r, g, b),
        vt100::Color::Idx(index) => {
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
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupt_and_navigation_are_terminal_sequences() {
        assert_eq!(key_bytes(egui::Key::C, true), vec![3]);
        assert_eq!(key_bytes(egui::Key::Enter, false), b"\r");
        assert_eq!(key_bytes(egui::Key::ArrowUp, false), b"\x1b[A");
    }
    #[test]
    fn shell_really_runs_in_a_pty() {
        let mut terminal = Terminal::spawn().expect("create PTY");
        terminal.writer.write_all(b"echo TESSEL_PTY_OK\r").unwrap();
        terminal.writer.flush().unwrap();
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
