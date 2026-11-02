#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod terminal;
use eframe::egui;
use terminal::Terminal;

#[derive(Default)]
struct Tessel {
    terminals: Vec<Terminal>,
    active: usize,
    error: String,
}

impl Tessel {
    fn new_terminal(&mut self) {
        match Terminal::spawn() {
            Ok(terminal) => {
                self.terminals.push(terminal);
                self.active = self.terminals.len() - 1;
                self.error.clear();
            }
            Err(error) => self.error = error.to_string(),
        }
    }
}

impl eframe::App for Tessel {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Tessel Rust");
                ui.label("Native prototype");
                if ui.button("+ Terminal").clicked() {
                    self.new_terminal();
                }
                for index in 0..self.terminals.len() {
                    if ui
                        .selectable_label(self.active == index, format!("Terminal {}", index + 1))
                        .clicked()
                    {
                        self.active = index;
                    }
                }
                if !self.terminals.is_empty() && ui.button("Close terminal").clicked() {
                    self.terminals.remove(self.active);
                    self.active = self.active.min(self.terminals.len().saturating_sub(1));
                }
            });
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            if !self.error.is_empty() { ui.colored_label(egui::Color32::LIGHT_RED, &self.error); }
            if let Some(terminal) = self.terminals.get_mut(self.active) {
                terminal.show(ui);
            } else {
                ui.label("Open a terminal to start. SSH, agents, files and Live Share are not migrated yet.");
            }
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}

fn main() -> eframe::Result {
    eframe::run_native(
        "Tessel Rust",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1100.0, 720.0]),
            ..Default::default()
        },
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            let mut app = Tessel::default();
            app.new_terminal();
            Ok(Box::new(app))
        }),
    )
}
