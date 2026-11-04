//! One cancellable directory enumeration at a time; all I/O stays off the UI thread.
use crate::{
    finder_model::{self, FileFinderState, ProjectFileList, WalkLimits},
    i18n::tr,
};
use eframe::egui;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
};

struct Scan {
    cancel: Arc<AtomicBool>,
    result: Receiver<Result<ProjectFileList, String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinderAction {
    OpenFile(PathBuf),
    InsertInTerminal(PathBuf),
}

#[derive(Default)]
pub struct Finder {
    open: bool,
    focused: bool,
    root: PathBuf,
    state: FileFinderState,
    results: Vec<String>,
    scan: Option<Scan>,
    error: String,
}

impl Drop for Scan {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Finder {
    #[cfg(feature = "visual-test")]
    pub fn fixture(root: PathBuf) -> Self {
        let state = FileFinderState::new(
            vec![
                "src/app.rs".into(),
                "src/main.rs".into(),
                "README.md".into(),
            ],
            false,
            false,
        );
        Self {
            open: true,
            root,
            results: state.results().to_vec(),
            state,
            ..Default::default()
        }
    }
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self, root: &Path) -> Result<(), String> {
        self.poll();
        if self.scan.is_some() {
            return Err(tr("The previous file search is still stopping.").into());
        }
        self.root = root.to_path_buf();
        self.state = FileFinderState::default();
        self.results.clear();
        self.error.clear();
        self.focused = false;
        self.open = true;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let worker_root = self.root.clone();
        let (sender, result) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let result = finder_model::walk_project_files_with_limits(
                &worker_root,
                &WalkLimits::default(),
                Some(&worker_cancel),
            );
            let _ = sender.send(result);
        });
        self.scan = Some(Scan { cancel, result });
        Ok(())
    }

    pub fn close(&mut self) {
        self.open = false;
        if let Some(scan) = &self.scan {
            scan.cancel.store(true, Ordering::Relaxed);
        }
    }

    pub fn poll(&mut self) {
        let Some(scan) = &self.scan else {
            return;
        };
        let result = match scan.result.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(tr("File search stopped unexpectedly.").into())
            }
        };
        let cancelled = scan.cancel.load(Ordering::Relaxed);
        self.scan = None;
        if !self.open || cancelled {
            return;
        }
        match result {
            Ok(list) => {
                self.state.set_files(list);
                self.results = self.state.results().to_vec();
                self.state.selected = 0;
            }
            Err(error) => self.error = error,
        }
    }

    #[cfg(test)]
    pub fn show(&mut self, ctx: &egui::Context, active_root: Option<&Path>) -> Option<PathBuf> {
        match self.show_action(ctx, active_root) {
            Some(FinderAction::OpenFile(path) | FinderAction::InsertInTerminal(path)) => Some(path),
            None => None,
        }
    }

    pub fn show_action(
        &mut self,
        ctx: &egui::Context,
        active_root: Option<&Path>,
    ) -> Option<FinderAction> {
        if !self.open {
            return None;
        }
        if active_root != Some(self.root.as_path()) {
            self.close();
            return None;
        }
        let mut selected: Option<(String, bool)> = None;
        let response = egui::Modal::new(egui::Id::new("file-finder")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 40.0).clamp(240.0, 650.0));
            ui.heading(tr("Find a file"));
            let mut query = self.state.query().to_owned();
            let input = ui.add(
                egui::TextEdit::singleline(&mut query)
                    .char_limit(256)
                    .desired_width(f32::INFINITY)
                    .hint_text(tr("Search project files…")),
            );
            if !self.focused {
                input.request_focus();
                self.focused = true;
            }
            if input.changed() {
                self.state.set_query(query);
                self.results = self.state.results().to_vec();
                self.state.selected = 0;
            }
            if self.scan.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(tr("Scanning project files…"));
                });
            }
            if !self.error.is_empty() {
                ui.label(&self.error);
            }
            if self.state.truncated {
                ui.label(tr("Search limit reached; some files are not listed."));
            }
            let count = self.results.len();
            let down = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown));
            let up = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp));
            if count > 0 {
                if down {
                    self.state.selected = (self.state.selected + 1) % count;
                }
                if up {
                    self.state.selected = (self.state.selected + count - 1) % count;
                }
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Enter)) {
                    selected = self
                        .results
                        .get(self.state.selected)
                        .cloned()
                        .map(|p| (p, true));
                } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
                    selected = self
                        .results
                        .get(self.state.selected)
                        .cloned()
                        .map(|p| (p, false));
                }
            } else if self.scan.is_none() && self.error.is_empty() {
                ui.label(tr("No matching files."));
            }
            egui::ScrollArea::vertical()
                .max_height((ctx.content_rect().height() * 0.55).max(80.0))
                .show(ui, |ui| {
                    for (index, path) in self.results.iter().enumerate() {
                        let row = ui.selectable_label(self.state.selected == index, path);
                        if row.clicked() {
                            let insert = ui.input(|i| i.modifiers.ctrl);
                            selected = Some((path.clone(), insert));
                        }
                        if (up || down) && self.state.selected == index {
                            row.scroll_to_me(Some(egui::Align::Center));
                        }
                    }
                });
            ui.separator();
            ui.weak(tr("Enter to open · Ctrl+Enter to insert path in terminal"));
        });
        if selected.is_some() || response.should_close() {
            self.close();
        }
        selected.map(|(relative, insert)| {
            let full = self.root.join(relative);
            if insert {
                FinderAction::InsertInTerminal(full)
            } else {
                FinderAction::OpenFile(full)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_and_enter_in_the_actual_dialog_open_the_filtered_file() {
        let root = tempfile::tempdir().unwrap();
        let state =
            FileFinderState::new(vec!["README.md".into(), "src/main.rs".into()], false, false);
        let mut finder = Finder {
            open: true,
            root: root.path().to_owned(),
            results: state.results().to_vec(),
            state,
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let mut frame = |events: Vec<egui::Event>| {
            let mut selected = None;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 520.0),
                    )),
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ctx| {
                    selected = finder.show(ctx, Some(root.path()));
                },
            );
            selected
        };
        frame(vec![]);
        frame(vec![]);
        assert!(frame(vec![egui::Event::Text("main".into())]).is_none());
        let opened = frame(vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert_eq!(opened, Some(root.path().join("src/main.rs")));
        assert!(!finder.is_open());
    }

    #[test]
    fn background_scan_returns_real_project_files_and_skips_dependencies() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("src")).unwrap();
        std::fs::create_dir(root.path().join("node_modules")).unwrap();
        std::fs::write(root.path().join("src/main.rs"), "synthetic fixture").unwrap();
        std::fs::write(root.path().join("README.md"), "synthetic fixture").unwrap();
        std::fs::write(
            root.path().join("node_modules/hidden.js"),
            "synthetic fixture",
        )
        .unwrap();
        let mut finder = Finder::default();
        finder.open(root.path()).unwrap();
        let start = std::time::Instant::now();
        while finder.scan.is_some() && start.elapsed() < std::time::Duration::from_secs(3) {
            finder.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(finder.scan.is_none());
        assert!(finder.error.is_empty(), "{}", finder.error);
        assert_eq!(finder.results, ["README.md", "src/main.rs"]);
        assert!(!finder.state.truncated);
    }

    #[test]
    fn closing_cancels_scan_refuses_parallel_workers_and_discards_late_results() {
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, result) = mpsc::sync_channel(1);
        let mut finder = Finder {
            open: true,
            scan: Some(Scan {
                cancel: cancel.clone(),
                result,
            }),
            ..Default::default()
        };
        finder.close();
        assert!(cancel.load(Ordering::Relaxed));
        assert!(finder.open(Path::new("unused-root")).is_err());
        sender
            .send(Ok(ProjectFileList {
                files: vec!["old-project.txt".into()],
                truncated: false,
                stop_reason: None,
                total_path_bytes: 15,
            }))
            .unwrap();
        finder.poll();
        assert!(!finder.is_open());
        assert!(finder.scan.is_none());
        assert!(finder.state.files().is_empty());
        assert!(finder.results.is_empty());
    }

    #[test]
    fn changing_projects_closes_finder_without_returning_a_stale_path() {
        let cancel = Arc::new(AtomicBool::new(false));
        let (_sender, result) = mpsc::sync_channel(1);
        let mut finder = Finder {
            open: true,
            root: PathBuf::from("first-project"),
            scan: Some(Scan {
                cancel: cancel.clone(),
                result,
            }),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        assert!(
            finder
                .show(&ctx, Some(Path::new("different-project")))
                .is_none()
        );
        assert!(!finder.is_open());
        assert!(cancel.load(Ordering::Relaxed));
    }

    #[test]
    fn ctrl_enter_in_finder_returns_insert_in_terminal_action() {
        let root = tempfile::tempdir().unwrap();
        let state = FileFinderState::new(
            vec!["README.md".into(), "src/terminal.rs".into()],
            false,
            false,
        );
        let mut finder = Finder {
            open: true,
            root: root.path().to_owned(),
            results: state.results().to_vec(),
            state,
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let mut frame = |events: Vec<egui::Event>, ctrl: bool| {
            let mut action = None;
            let modifiers = egui::Modifiers {
                ctrl,
                command: ctrl,
                ..Default::default()
            };
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 520.0),
                    )),
                    focused: true,
                    modifiers,
                    events,
                    ..Default::default()
                },
                |ctx| {
                    action = finder.show_action(ctx, Some(root.path()));
                },
            );
            action
        };
        frame(vec![], false);
        frame(vec![], false);
        assert!(frame(vec![egui::Event::Text("term".into())], false).is_none());
        let inserted = frame(
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers {
                    ctrl: true,
                    command: true,
                    ..Default::default()
                },
            }],
            true,
        );
        assert_eq!(
            inserted,
            Some(FinderAction::InsertInTerminal(
                root.path().join("src/terminal.rs")
            ))
        );
        assert!(!finder.is_open());
    }
}
