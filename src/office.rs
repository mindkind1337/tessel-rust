//! Read-only Office/OpenDocument content decoded in a disposable resource-limited worker.
use crate::i18n::tr;
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};
mod parse;
const MAX_INPUT: usize = 32 * 1024 * 1024;
const MAX_OUTPUT: usize = 8 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
struct Paragraph {
    heading: bool,
    text: String,
}
#[derive(Serialize, Deserialize)]
struct Section {
    title: String,
    paragraphs: Vec<Paragraph>,
    rows: Vec<Vec<String>>,
    omitted_rows: usize,
    omitted_cols: usize,
    start_row: u32,
    start_col: u32,
}
#[derive(Serialize, Deserialize)]
struct Book {
    sections: Vec<Section>,
    truncated: bool,
}
impl Book {
    fn validate(&self, extension: &str) -> Result<(), String> {
        let mut bytes = 0usize;
        let mut cells = 0usize;
        if (self.sections.is_empty() && !is_workbook(extension)) || self.sections.len() > 200 {
            return Err("Office section limit reached".into());
        }
        for page in &self.sections {
            if page.title.len() > 4096
                || page.paragraphs.len() > 20000
                || page.rows.len() > 5000
                || page.start_row > 1048575
                || page.start_col > 16383
            {
                return Err("Invalid Office preview dimensions".into());
            }
            bytes += page.title.len();
            for paragraph in &page.paragraphs {
                bytes += paragraph.text.len();
            }
            for row in &page.rows {
                if row.len() > 200 {
                    return Err("Office column limit reached".into());
                }
                cells += row.len();
                for value in row {
                    if value.len() > 4096 {
                        return Err("Office cell limit reached".into());
                    }
                    bytes += value.len();
                }
            }
        }
        if bytes > 3 * 1024 * 1024 || cells > 100000 {
            return Err("Office content limit reached".into());
        }
        Ok(())
    }
}
fn is_workbook(extension: &str) -> bool {
    matches!(extension, "xlsx" | "xlsm" | "ods")
}
pub fn supported(extension: &str) -> bool {
    matches!(
        extension,
        "docx" | "odt" | "pptx" | "odp" | "xlsx" | "xlsm" | "ods"
    )
}
pub fn worker() -> Result<(), String> {
    let mut gate = [0];
    std::io::stdin()
        .read_exact(&mut gate)
        .map_err(|_| "Missing document worker gate")?;
    if gate != [1] {
        return Err("Invalid document worker gate".into());
    }
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 5 || !supported(&args[4]) {
        return Err("Invalid document worker arguments".into());
    }
    let mut bytes = Vec::new();
    File::open(&args[2])
        .map_err(|_| "Cannot read Office input")?
        .take((MAX_INPUT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read Office input")?;
    if bytes.len() > MAX_INPUT {
        return Err("Office input exceeds 32 MiB".into());
    }
    let book = parse::parse(&bytes, &args[4])?;
    let output = serde_json::to_vec(&book).map_err(|_| "Cannot encode Office preview")?;
    if output.len() > MAX_OUTPUT {
        return Err("Office preview exceeds limit".into());
    }
    std::fs::write(&args[3], output).map_err(|_| "Cannot save Office preview".into())
}
fn decode(bytes: &[u8], extension: &str, cancel: &AtomicBool) -> Result<Book, String> {
    let _slot = loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Office preview cancelled".into());
        }
        match crate::pdf::RENDER_SLOT.try_lock() {
            Ok(lock) => break lock,
            Err(std::sync::TryLockError::WouldBlock) => {
                std::thread::sleep(Duration::from_millis(20))
            }
            Err(_) => return Err("Document worker unavailable".into()),
        }
    };
    let directory = tempfile::tempdir().map_err(|_| "Cannot create document workspace")?;
    let input = directory.path().join("input");
    let output = directory.path().join("preview.json");
    std::fs::write(&input, bytes).map_err(|_| "Cannot prepare document")?;
    let mut command =
        Command::new(std::env::current_exe().map_err(|_| "Cannot locate document worker")?);
    crate::agent_env::clean_command(&mut command);
    command
        .arg("--office-worker")
        .arg(&input)
        .arg(&output)
        .arg(extension)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = crate::pdf::Process(
        command
            .spawn()
            .map_err(|_| "Cannot start document worker")?,
    );
    #[cfg(windows)]
    let _job = crate::pdf::Job::attach(&child.0)?;
    child
        .0
        .stdin
        .take()
        .ok_or("Missing document worker gate")?
        .write_all(&[1])
        .map_err(|_| "Document worker gate failed")?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Office preview cancelled".into());
        }
        if Instant::now() >= deadline {
            return Err("Office preview exceeded 20 seconds".into());
        }
        if let Some(status) = child.0.try_wait().map_err(|_| "Document worker failed")? {
            if !status.success() {
                return Err("Office preview failed: unsupported, damaged, encrypted or resource limit reached".into());
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut result = Vec::new();
    File::open(output)
        .map_err(|_| "Missing Office preview")?
        .take((MAX_OUTPUT + 1) as u64)
        .read_to_end(&mut result)
        .map_err(|_| "Cannot read Office preview")?;
    if result.len() > MAX_OUTPUT {
        return Err("Office output exceeds limit".into());
    }
    let book: Book = serde_json::from_slice(&result).map_err(|_| "Invalid Office preview")?;
    book.validate(extension)?;
    Ok(book)
}
pub struct OfficeView {
    bytes: Arc<Vec<u8>>,
    extension: String,
    book: Option<Book>,
    pending: Option<Receiver<Result<Book, String>>>,
    cancel: Arc<AtomicBool>,
    selected: usize,
    error: String,
    summary: Option<Arc<OfficeSummary>>,
}
#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub struct OfficeSummary {
    pub omitted_rows: usize,
    pub omitted_cols: usize,
    pub workbook: bool,
    pub section_titles: Vec<String>,
    pub selected: usize,
    pub paragraphs: Vec<(bool, String)>,
    pub sheet: Option<Arc<crate::rich_previews::SheetTable>>,
    pub truncated: bool,
    pub busy: bool,
    pub error: String,
}

impl OfficeView {
    pub(crate) fn is_rendering(&self) -> bool {
        self.pending.is_some()
    }

    fn poll_completed(&mut self) {
        let result = match self.pending.as_ref().map(Receiver::try_recv) {
            Some(Ok(result)) => result,
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                Err("Office renderer stopped without returning a preview.".into())
            }
            _ => return,
        };
        self.pending = None;
        self.summary = None;
        match result {
            Ok(book) => self.book = Some(book),
            Err(error) => self.error = error,
        }
    }

    pub fn gpui_poll_summary(&mut self) -> Arc<OfficeSummary> {
        self.poll_completed();
        if let Some(summary) = &self.summary {
            return summary.clone();
        }
        if self.book.is_none() && self.error.is_empty() && self.pending.is_none() {
            let (tx, rx) = mpsc::channel();
            self.pending = Some(rx);
            let bytes = self.bytes.clone();
            let extension = self.extension.clone();
            let cancel = self.cancel.clone();
            std::thread::spawn(move || {
                let _ = tx.send(decode(&bytes, &extension, &cancel));
            });
        }
        let summary = if let Some(book) = &self.book {
            let idx = self.selected.min(book.sections.len().saturating_sub(1));
            let page = book.sections.get(idx);
            let sheet =
                page.filter(|p| !p.rows.is_empty())
                    .map(|page| crate::rich_previews::SheetTable {
                        rows: page.rows.clone(),
                        start_row: page.start_row as usize,
                        start_col: page.start_col as usize,
                        truncated_rows: page.omitted_rows,
                        truncated_cols: page.omitted_cols,
                        delimiter_label: "Workbook sheet",
                    });
            OfficeSummary {
                workbook: is_workbook(&self.extension),
                omitted_rows: page.map_or(0, |p| p.omitted_rows),
                omitted_cols: page.map_or(0, |p| p.omitted_cols),
                section_titles: book.sections.iter().map(|s| s.title.clone()).collect(),
                selected: idx,
                paragraphs: page
                    .into_iter()
                    .flat_map(|p| &p.paragraphs)
                    .map(|p| (p.heading, p.text.clone()))
                    .collect(),
                sheet: sheet.map(Arc::new),
                truncated: book.truncated,
                busy: self.pending.is_some(),
                error: self.error.clone(),
            }
        } else {
            OfficeSummary {
                workbook: is_workbook(&self.extension),
                busy: self.pending.is_some(),
                error: self.error.clone(),
                ..Default::default()
            }
        };
        let summary = Arc::new(summary);
        // Keep completed content stable across frames; pending summaries are tiny
        // and must be polled until the worker delivers its result.
        if !summary.busy {
            self.summary = Some(summary.clone());
        }
        summary
    }

    pub fn select_section(&mut self, idx: usize) {
        if self.selected != idx {
            self.selected = idx;
            self.summary = None;
        }
    }

    pub fn open(path: &Path) -> Result<Self, String> {
        let mut bytes = Vec::new();
        File::open(path)
            .map_err(|_| "Cannot open Office document")?
            .take((MAX_INPUT + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| "Cannot read Office document")?;
        if bytes.len() > MAX_INPUT {
            return Err("Office input exceeds 32 MiB".into());
        }
        Ok(Self {
            bytes: Arc::new(bytes),
            extension: path
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_ascii_lowercase(),
            book: None,
            pending: None,
            cancel: Arc::new(AtomicBool::new(false)),
            selected: 0,
            error: String::new(),
            summary: None,
        })
    }
    pub fn show(&mut self, ui: &mut egui::Ui) {
        ui.small(tr("Read-only content preview · macros and formulas are not executed · formatting may differ"));
        self.poll_completed();
        if let Some(book) = &self.book {
            if book.truncated {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    tr("Preview truncated at the document limits."),
                );
            }
            if book.sections.is_empty() {
                ui.label(tr("This workbook has no sheet."));
                return;
            }
            let previous_section = self.selected;
            egui::ScrollArea::horizontal()
                .id_salt("office-tabs")
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (i, page) in book.sections.iter().enumerate() {
                            ui.selectable_value(&mut self.selected, i, &page.title);
                        }
                    });
                });
            if previous_section != self.selected {
                self.summary = None;
            }
            let page = &book.sections[self.selected.min(book.sections.len() - 1)];
            egui::ScrollArea::both()
                .id_salt(("office-content", self.selected))
                .show(ui, |ui| {
                    for paragraph in &page.paragraphs {
                        if paragraph.heading {
                            ui.heading(&paragraph.text);
                        } else {
                            ui.add(egui::Label::new(&paragraph.text).selectable(true));
                        }
                    }
                    if page.rows.is_empty() && is_workbook(&self.extension) {
                        ui.label(tr("This sheet is empty."));
                    }
                    if !page.rows.is_empty() {
                        let sheet = crate::rich_previews::SheetTable {
                            rows: page.rows.clone(),
                            start_row: page.start_row as usize,
                            start_col: page.start_col as usize,
                            truncated_rows: page.omitted_rows,
                            truncated_cols: page.omitted_cols,
                            delimiter_label: "Workbook sheet",
                        };
                        crate::rich_previews::show_sheet_grid(
                            ui,
                            ("office-sheet", self.selected),
                            &sheet,
                        );
                    }
                });
        } else if !self.error.is_empty() {
            ui.colored_label(egui::Color32::LIGHT_RED, &self.error);
        } else if self.pending.is_none() {
            let (tx, rx) = mpsc::channel();
            self.pending = Some(rx);
            let bytes = self.bytes.clone();
            let extension = self.extension.clone();
            let cancel = self.cancel.clone();
            std::thread::spawn(move || {
                let _ = tx.send(decode(&bytes, &extension, &cancel));
            });
        }
        if self.pending.is_some() {
            ui.spinner();
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
    }
    #[cfg(feature = "visual-test")]
    pub fn prime(&mut self) -> Result<(), String> {
        self.book = Some(decode(&self.bytes, &self.extension, &self.cancel)?);
        self.summary = None;
        Ok(())
    }
}
impl Drop for OfficeView {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
#[cfg_attr(not(test), allow(dead_code))]
fn column(mut index: usize) -> String {
    let mut name = String::new();
    loop {
        name.insert(0, (b'A' + (index % 26) as u8) as char);
        if index < 26 {
            break;
        }
        index = index / 26 - 1;
    }
    name
}
#[cfg(test)]
mod tests;

mod fixtures;
pub fn smoke() -> Result<(), Box<dyn std::error::Error>> {
    let cancel = AtomicBool::new(false);
    let book = decode(&fixtures::document(), "docx", &cancel)?;
    if book.sections[0].paragraphs[1].text != "A & B" {
        return Err("Original document text changed".into());
    }
    let book = decode(&fixtures::spreadsheet(), "xlsx", &cancel)?;
    if book.sections[0].rows[0][2] != "43" {
        return Err("Cached spreadsheet value changed".into());
    }
    if decode(b"broken", "docx", &cancel).is_ok() {
        return Err("Damaged archive accepted".into());
    }
    cancel.store(true, Ordering::Relaxed);
    if decode(&fixtures::document(), "docx", &cancel).is_ok() {
        return Err("Cancelled worker accepted".into());
    }
    println!("Office native worker acceptance passed");
    Ok(())
}
