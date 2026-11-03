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
    start_row: u32,
    start_col: u32,
}
#[derive(Serialize, Deserialize)]
struct Book {
    sections: Vec<Section>,
    truncated: bool,
}
impl Book {
    fn validate(&self) -> Result<(), String> {
        let mut bytes = 0usize;
        let mut cells = 0usize;
        if self.sections.is_empty() || self.sections.len() > 200 {
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
                if row.len() > 100 {
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
    book.validate()?;
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
}
impl OfficeView {
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
        })
    }
    pub fn show(&mut self, ui: &mut egui::Ui) {
        ui.small(tr("Read-only content preview · macros and formulas are not executed · formatting may differ"));
        if let Some(rx) = &self.pending
            && let Ok(result) = rx.try_recv()
        {
            self.pending = None;
            match result {
                Ok(book) => self.book = Some(book),
                Err(e) => self.error = e,
            }
        }
        if let Some(book) = &self.book {
            if book.truncated {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    tr("Preview truncated at the document limits."),
                );
            }
            egui::ScrollArea::horizontal()
                .id_salt("office-tabs")
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (i, page) in book.sections.iter().enumerate() {
                            ui.selectable_value(&mut self.selected, i, &page.title);
                        }
                    });
                });
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
                    if !page.rows.is_empty() {
                        let cols = page.rows.iter().map(Vec::len).max().unwrap_or(0);
                        ui.horizontal(|ui| {
                            ui.add_sized([55.0, 22.0], egui::Label::new(""));
                            for col in 0..cols {
                                ui.add_sized(
                                    [120.0, 22.0],
                                    egui::Label::new(column(page.start_col as usize + col))
                                        .truncate(),
                                );
                            }
                        });
                        egui::ScrollArea::vertical()
                            .id_salt("office-rows")
                            .show_rows(ui, 24.0, page.rows.len(), |ui, rows| {
                                for index in rows {
                                    ui.horizontal(|ui| {
                                        ui.add_sized(
                                            [55.0, 22.0],
                                            egui::Label::new(
                                                (page.start_row as usize + index + 1).to_string(),
                                            ),
                                        );
                                        for cell in &page.rows[index] {
                                            ui.add_sized(
                                                [120.0, 22.0],
                                                egui::Label::new(cell).truncate().selectable(true),
                                            )
                                            .on_hover_text(cell);
                                        }
                                    });
                                }
                            });
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
        Ok(())
    }
}
impl Drop for OfficeView {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
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
