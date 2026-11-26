//! Original-page PDF rasterization in a disposable, deadline-bound worker.
use crate::i18n::tr;
use eframe::egui;
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};
const MAX_INPUT: usize = 32 * 1024 * 1024;
const MAX_PIXELS: usize = 8 * 1024 * 1024;
pub(crate) static RENDER_SLOT: Mutex<()> = Mutex::new(());
const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 4.0;
const HEADER_BYTES: usize = 24;

fn manual_zoom(scale: f32, step: f32) -> f32 {
    (((scale + step) * 4.).round() / 4.).clamp(MIN_ZOOM, MAX_ZOOM)
}

pub struct Rendered {
    pub points: [f32; 2],
    pub widest: f32,
    pub image: egui::ColorImage,
    pub pages: usize,
}
fn dimensions(width: f32, height: f32, requested: f32) -> Result<f32, String> {
    if !width.is_finite()
        || !height.is_finite()
        || width <= 0.0
        || height <= 0.0
        || !requested.is_finite()
        || !(MIN_ZOOM..=MAX_ZOOM).contains(&requested)
    {
        return Err("Invalid PDF page dimensions or zoom".into());
    }
    let scale = requested
        .min(4096.0 / width)
        .min(4096.0 / height)
        .min((MAX_PIXELS as f32 / (width * height)).sqrt());
    if !scale.is_finite() || width * scale < 1.0 || height * scale < 1.0 {
        return Err("PDF page dimensions are unsupported".into());
    }
    Ok(scale)
}
struct Raster {
    size: [usize; 2],
    points: [f32; 2],
    widest: f32,
    pages: usize,
    rgba: Vec<u8>,
}

fn raster(bytes: Vec<u8>, index: usize, scale: f32) -> Result<Raster, String> {
    if bytes.len() > MAX_INPUT {
        return Err("PDF limit is 32 MiB".into());
    }
    let pdf = hayro::hayro_syntax::Pdf::new(bytes).map_err(|_| "Unsupported or invalid PDF")?;
    let pages = pdf.pages();
    if pages.is_empty() || pages.len() > 2000 {
        return Err("PDF page limit is 2000".into());
    }
    let page = pages.get(index).ok_or("PDF page is out of range")?;
    let (width, height) = page.render_dimensions();
    let scale = dimensions(width, height, scale)?;
    let settings = hayro::PixmapSettings {
        x_scale: scale,
        y_scale: scale,
        bg_color: hayro::vello_cpu::color::palette::css::WHITE,
    };
    let pixels = hayro::render(
        page,
        &hayro::RenderCache::new(),
        &Default::default(),
        &Default::default(),
        &settings,
    );
    Ok(Raster {
        size: [pixels.width() as usize, pixels.height() as usize],
        points: [width, height],
        widest: pages
            .iter()
            .map(|page| page.render_dimensions().0)
            .filter(|w| w.is_finite())
            .fold(width, f32::max),
        pages: pages.len(),
        rgba: pixels.data_as_u8_slice().to_vec(),
    })
}
pub fn worker() -> Result<(), String> {
    // Parent attaches resource limits before releasing this gate.
    let mut gate = [0];
    std::io::stdin()
        .read_exact(&mut gate)
        .map_err(|_| "Worker gate closed")?;
    if gate != [1] {
        return Err("Invalid worker gate".into());
    }
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 6 {
        return Err("Invalid worker arguments".into());
    }
    let index = args[4]
        .to_str()
        .and_then(|s| s.parse::<usize>().ok())
        .ok_or("Invalid page")?;
    let scale = args[5]
        .to_str()
        .and_then(|s| s.parse::<f32>().ok())
        .ok_or("Invalid zoom")?;
    let mut bytes = Vec::new();
    File::open(&args[2])
        .map_err(|_| "Missing PDF snapshot")?
        .take((MAX_INPUT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read PDF snapshot")?;
    let raster = raster(bytes, index, scale)?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[3])
        .map_err(|_| "Cannot create rendering")?;
    for n in [raster.size[0], raster.size[1], raster.pages] {
        output
            .write_all(&(n as u32).to_le_bytes())
            .map_err(|_| "Cannot write rendering")?;
    }
    for n in [raster.points[0], raster.points[1], raster.widest] {
        output
            .write_all(&n.to_le_bytes())
            .map_err(|_| "Cannot write page size")?;
    }
    output
        .write_all(&raster.rgba)
        .map_err(|_| "Cannot write pixels")?;
    Ok(())
}
pub(crate) struct Process(pub(crate) Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[cfg(windows)]
pub(crate) struct Job(windows::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Job {
    pub(crate) fn attach(child: &Child) -> Result<Self, String> {
        use std::os::windows::io::AsRawHandle;
        use windows::{
            Win32::{Foundation::HANDLE, System::JobObjects::*},
            core::PCWSTR,
        };
        unsafe {
            let handle = CreateJobObjectW(None, PCWSTR::null())
                .map_err(|_| "Cannot create PDF resource limits")?;
            let job = Self(handle);
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                | JOB_OBJECT_LIMIT_PROCESS_MEMORY
                | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
            limits.BasicLimitInformation.ActiveProcessLimit = 1;
            limits.ProcessMemoryLimit = 512 * 1024 * 1024;
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
            .map_err(|_| "Cannot apply PDF resource limits")?;
            AssignProcessToJobObject(handle, HANDLE(child.as_raw_handle()))
                .map_err(|_| "Cannot isolate PDF process resources")?;
            Ok(job)
        }
    }
}
#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
fn render(bytes: &[u8], index: usize, scale: f32, cancel: &AtomicBool) -> Result<Rendered, String> {
    if bytes.len() > MAX_INPUT {
        return Err("PDF limit is 32 MiB".into());
    }
    let _slot = loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("PDF rendering cancelled".into());
        }
        match RENDER_SLOT.try_lock() {
            Ok(lock) => break lock,
            Err(std::sync::TryLockError::WouldBlock) => {
                std::thread::sleep(Duration::from_millis(20))
            }
            Err(_) => return Err("PDF renderer unavailable".into()),
        }
    };
    let dir = tempfile::tempdir().map_err(|_| "Cannot create PDF workspace")?;
    let input = dir.path().join("source.pdf");
    let output = dir.path().join("page.rgba");
    std::fs::write(&input, bytes).map_err(|_| "Cannot prepare PDF rendering")?;
    let mut command =
        Command::new(std::env::current_exe().map_err(|_| "Cannot locate PDF worker")?);
    crate::agent_env::clean_command(&mut command);
    command
        .arg("--pdf-worker")
        .arg(&input)
        .arg(&output)
        .arg(index.to_string())
        .arg(scale.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = Process(command.spawn().map_err(|_| "Cannot start PDF worker")?);
    #[cfg(windows)]
    let _job = Job::attach(&child.0)?;
    child
        .0
        .stdin
        .take()
        .ok_or("PDF input gate unavailable")?
        .write_all(&[1])
        .map_err(|_| "PDF input gate failed")?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("PDF rendering cancelled".into());
        }
        if Instant::now() >= deadline {
            return Err("PDF exceeded the 20-second rendering limit".into());
        }
        if let Some(status) = child.0.try_wait().map_err(|_| "PDF worker failed")? {
            if !status.success() {
                return Err(
                    "PDF could not be rendered (unsupported, damaged, or resource limit reached)"
                        .into(),
                );
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut result = Vec::new();
    File::open(output)
        .map_err(|_| "PDF worker returned no page")?
        .take((MAX_PIXELS * 4 + HEADER_BYTES + 1) as u64)
        .read_to_end(&mut result)
        .map_err(|_| "Cannot read PDF rendering")?;
    decode(&result)
}
fn decode(bytes: &[u8]) -> Result<Rendered, String> {
    if bytes.len() < HEADER_BYTES {
        return Err("Incomplete PDF rendering".into());
    }
    let width = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
    let height = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let pages = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let points = [
        f32::from_le_bytes(bytes[12..16].try_into().unwrap()),
        f32::from_le_bytes(bytes[16..20].try_into().unwrap()),
    ];
    let widest = f32::from_le_bytes(bytes[20..24].try_into().unwrap());
    if !widest.is_finite()
        || widest < points[0]
        || !points
            .iter()
            .all(|n| n.is_finite() && *n > 0. && (*n * MAX_ZOOM).is_finite())
        || width == 0
        || height == 0
        || width > 4096
        || height > 4096
        || width * height > MAX_PIXELS
        || !(1..=2000).contains(&pages)
        || bytes.len() != HEADER_BYTES + width * height * 4
    {
        return Err("Invalid PDF rendering dimensions".into());
    }
    Ok(Rendered {
        widest,
        points,
        image: egui::ColorImage::from_rgba_unmultiplied([width, height], &bytes[HEADER_BYTES..]),
        pages,
    })
}
#[derive(Clone, Copy, Debug, Default)]
pub struct PageLayout {
    pub viewport: f32,
    pub size: Option<[f32; 2]>,
    pub fit: bool,
}
#[derive(Clone, Debug, Default)]
pub struct PdfSummary {
    pub page: usize,
    pub pages: usize,
    pub scale: f32,
    pub busy: bool,
    pub error: String,
    pub layout: PageLayout,
}

pub struct PdfView {
    bytes: Arc<Vec<u8>>,
    page: usize,
    pages: usize,
    scale: f32,
    fit: bool,
    points: Option<[f32; 2]>,
    widest: f32,
    viewport: f32,
    texture: Option<egui::TextureHandle>,
    gpui_image: Option<Arc<gpui::RenderImage>>,
    pending: Option<Receiver<Result<Rendered, String>>>,
    cancel: Arc<AtomicBool>,
    error: String,
}
impl PdfView {
    pub fn open(path: &Path) -> Result<Self, String> {
        let mut bytes = Vec::new();
        File::open(path)
            .map_err(|_| "Cannot open PDF")?
            .take((MAX_INPUT + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| "Cannot read PDF")?;
        if bytes.len() > MAX_INPUT {
            return Err("PDF limit is 32 MiB".into());
        }
        Ok(Self {
            bytes: Arc::new(bytes),
            page: 0,
            pages: 0,
            scale: 1.5,
            fit: true,
            points: None,
            widest: 0.,
            viewport: 0.,
            texture: None,
            gpui_image: None,
            pending: None,
            cancel: Arc::new(AtomicBool::new(false)),
            error: String::new(),
        })
    }

    fn take_completed(&mut self) -> Option<Result<Rendered, String>> {
        let result = match self.pending.as_ref()?.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("PDF renderer stopped without returning a page.".into()))
            }
        };
        if result.is_some() {
            self.pending = None;
        }
        result
    }

    pub fn gpui_poll_summary(&mut self) -> PdfSummary {
        if let Some(result) = self.take_completed() {
            match result {
                Ok(page) => {
                    self.widest = page.widest;
                    self.points = Some(page.points);
                    self.pages = page.pages;
                    self.gpui_image = crate::gpui_pixels::from_color_image(&page.image);
                }
                Err(e) => self.error = e,
            }
        }
        if self.gpui_image.is_none() && self.pending.is_none() && self.error.is_empty() {
            self.request();
        }
        self.refresh_fit_resolution();
        PdfSummary {
            page: self.page,
            pages: self.pages,
            scale: self.effective_scale(),
            busy: self.pending.is_some(),
            error: self.error.clone(),
            layout: PageLayout {
                viewport: self.viewport,
                size: self.display_size(),
                fit: self.fit,
            },
        }
    }

    pub(crate) fn is_rendering(&self) -> bool {
        self.pending.is_some()
    }

    pub fn step_page(&mut self, delta: isize) {
        if self.pending.is_some() {
            return;
        }
        let next = if delta < 0 {
            self.page.saturating_sub(delta.unsigned_abs())
        } else {
            (self.page + delta as usize).min(self.pages.saturating_sub(1))
        };
        if next != self.page {
            self.page = next;
            self.request();
        }
    }

    pub fn set_viewport(&mut self, width: f32) -> bool {
        let width = width.round();
        if !width.is_finite() || width <= 0. || self.viewport == width {
            return false;
        }
        self.viewport = width;
        true
    }

    pub fn fit_width(&mut self) {
        self.fit = true;
    }

    fn effective_scale(&self) -> f32 {
        if self.fit && self.widest > 0. && self.viewport > 0. {
            ((self.viewport - 48.) / self.widest).clamp(0.2, MAX_ZOOM)
        } else {
            self.scale
        }
    }

    pub fn display_size(&self) -> Option<[f32; 2]> {
        self.points
            .map(|size| size.map(|n| n * self.effective_scale()))
    }

    pub fn gpui_image(&self) -> Option<Arc<gpui::RenderImage>> {
        self.gpui_image.clone()
    }

    pub fn step_zoom(&mut self, delta: f32) {
        if self.pending.is_some() {
            return;
        }
        let next = manual_zoom(self.effective_scale(), delta);
        if self.fit || (next - self.scale).abs() > 0.01 {
            self.scale = next;
            self.fit = false;
            self.request();
        }
    }
    #[cfg(feature = "visual-test")]
    pub fn prime(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let page = render(&self.bytes, self.page, self.scale, &self.cancel)?;
        self.widest = page.widest;
        self.points = Some(page.points);
        self.pages = page.pages;
        self.texture = Some(ctx.load_texture("pdf-page", page.image, egui::TextureOptions::LINEAR));
        Ok(())
    }
    fn refresh_fit_resolution(&mut self) {
        if self.fit && self.points.is_some() && self.pending.is_none() && self.error.is_empty() {
            let target = self.effective_scale().clamp(MIN_ZOOM, MAX_ZOOM);
            if (target - self.scale).abs() > 0.01 {
                self.scale = target;
                self.start_worker();
            }
        }
    }

    fn request(&mut self) {
        self.texture = None;
        self.gpui_image = None;
        self.start_worker();
    }

    fn start_worker(&mut self) {
        let bytes = self.bytes.clone();
        let index = self.page;
        let scale = self.scale;
        let cancel = self.cancel.clone();
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        self.error.clear();
        std::thread::spawn(move || {
            let _ = tx.send(render(&bytes, index, scale, &cancel));
        });
    }
    pub fn show(&mut self, ui: &mut egui::Ui) {
        if let Some(result) = self.take_completed() {
            match result {
                Ok(page) => {
                    self.widest = page.widest;
                    self.points = Some(page.points);
                    self.pages = page.pages;
                    self.texture = Some(ui.ctx().load_texture(
                        "pdf-page",
                        page.image,
                        egui::TextureOptions::LINEAR,
                    ));
                }
                Err(e) => self.error = e,
            }
        }
        if self.texture.is_none() && self.pending.is_none() && self.error.is_empty() {
            self.request();
        }
        let viewport = ui.available_width();
        self.set_viewport(viewport);
        self.refresh_fit_resolution();
        let busy = self.pending.is_some();
        let mut rerender = false;
        let effective = self.effective_scale();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!busy && self.page > 0, egui::Button::new(tr("Previous")))
                .clicked()
            {
                self.page -= 1;
                rerender = true;
            }
            ui.label(format!(
                "Page {} / {}",
                self.page + 1,
                if self.pages == 0 {
                    "…".into()
                } else {
                    self.pages.to_string()
                }
            ));
            if ui
                .add_enabled(
                    !busy && self.page + 1 < self.pages,
                    egui::Button::new(tr("Next")),
                )
                .clicked()
            {
                self.page += 1;
                rerender = true;
            }
            if ui
                .add_enabled(!busy && effective > MIN_ZOOM, egui::Button::new("−"))
                .clicked()
            {
                self.scale = manual_zoom(effective, -0.25);
                self.fit = false;
                rerender = true;
            }
            ui.label(format!("{:.0}%", effective * 100.));
            if ui
                .add_enabled(!busy && effective < MAX_ZOOM, egui::Button::new("+"))
                .clicked()
            {
                self.scale = manual_zoom(effective, 0.25);
                self.fit = false;
                rerender = true;
            }
            if ui.button(tr("Fit width")).clicked() {
                self.fit_width();
            }
        });
        if rerender {
            self.request();
        }
        if busy {
            ui.spinner();
            ui.label(tr("Rendering original PDF page…"));
        }
        if !self.error.is_empty() {
            ui.colored_label(egui::Color32::LIGHT_RED, &self.error);
            if ui.button(tr("Retry page")).clicked() {
                self.request();
            }
        }
        if let Some(texture) = &self.texture {
            egui::ScrollArea::both()
                .id_salt(("pdf", self.page))
                .show(ui, |ui| {
                    if let Some(size) = self.points {
                        let size = egui::vec2(size[0] * effective, size[1] * effective);
                        let x = ((viewport - size.x) * 0.5).max(24.);
                        let origin = ui.cursor().min + egui::vec2(x, 16.);
                        ui.put(
                            egui::Rect::from_min_size(origin, size),
                            egui::Image::new(texture).fit_to_exact_size(size),
                        );
                    }
                });
        }
    }
}
impl Drop for PdfView {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
pub fn fixture() -> Vec<u8> {
    let stream =
        "0.1 0.4 0.9 rg 20 20 160 60 re f BT /F1 18 Tf 20 115 Td (Original PDF fixture) Tj ET";
    let objects=["<< /Type /Catalog /Pages 2 0 R >>".to_string(),"<< /Type /Pages /Kids [3 0 R 6 0 R] /Count 2 >>".into(),"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 180] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".into(),"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),format!("<< /Length {} >>\nstream\n{stream}\nendstream",stream.len()),"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 180] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".into()];
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![];
    for (i, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 7\n0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 7 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    bytes
}
pub fn smoke() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = fixture();
    let cancel = AtomicBool::new(false);
    for index in 0..2 {
        let page = render(&bytes, index, 1.0, &cancel)?;
        if page.pages != 2
            || page.image.size != [300, 180]
            || !page
                .image
                .pixels
                .iter()
                .any(|p| p.b() > p.r().saturating_add(40))
        {
            return Err("PDF raster assertion failed".into());
        }
    }
    if render(b"invalid PDF", 0, 1.0, &cancel).is_ok() {
        return Err("Invalid PDF accepted".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gpui_keeps_original_page_pixels_and_reports_worker_disconnect() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.pdf");
        std::fs::write(&path, fixture()).unwrap();
        let mut view = PdfView::open(&path).unwrap();
        let (sender, receiver) = mpsc::channel();
        view.pending = Some(receiver);
        let raster = raster(fixture(), 0, 1.).unwrap();
        let [width, height] = raster.size;
        sender
            .send(Ok(Rendered {
                image: egui::ColorImage::from_rgba_unmultiplied([width, height], &raster.rgba),
                pages: raster.pages,
                points: raster.points,
                widest: raster.widest,
            }))
            .unwrap();
        let summary = view.gpui_poll_summary();
        assert_eq!(summary.pages, 2);
        let image = view.gpui_image().unwrap();
        assert_eq!(image.as_bytes(0).unwrap().len(), width * height * 4);
        assert!(Arc::ptr_eq(&image, &view.gpui_image().unwrap()));
        let (sender, receiver) = mpsc::channel();
        view.pending = Some(receiver);
        view.gpui_image = None;
        drop(sender);
        let summary = view.gpui_poll_summary();
        assert!(!summary.busy);
        assert!(summary.error.contains("stopped"));
        assert!(view.pending.is_none());
    }
    #[test]
    fn dimensions_and_protocol_are_bounded() {
        assert!(dimensions(f32::INFINITY, 100.0, 1.0).is_err());
        assert!(dimensions(100.0, 100.0, f32::NAN).is_err());
        assert!(dimensions(100.0, 100.0, 4.1).is_err());
        assert_eq!(manual_zoom(3.9, 0.25), 4.);
        assert_eq!(manual_zoom(0.33, -0.25), 0.25);
        assert_eq!(manual_zoom(1.83, 0.25), 2.);
        assert!(dimensions(100000.0, 100000.0, 3.0).unwrap() < 1.0);
        assert!(decode(&[0; 12]).is_err());
    }
    #[test]
    fn fit_tracks_widest_page_and_manual_zoom_preserves_physical_dimensions() {
        let mut source = fixture();
        let at = source.windows(7).rposition(|s| s == b"300 180").unwrap();
        source[at..at + 7].copy_from_slice(b"600 180");
        let first = raster(source, 0, 4.).unwrap();
        assert_eq!(first.points, [300., 180.]);
        assert_eq!(first.widest, 600.);
        assert_eq!(first.size, [1200, 720]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.pdf");
        std::fs::write(&path, fixture()).unwrap();
        let mut view = PdfView::open(&path).unwrap();
        view.widest = first.widest;
        view.points = Some(first.points);
        view.set_viewport(948.);
        assert_eq!(view.display_size(), Some([450., 270.]));
        view.set_viewport(648.);
        assert_eq!(view.display_size(), Some([300., 180.]));
        view.step_zoom(0.25);
        assert_eq!(view.display_size(), Some([375., 225.]));
        view.set_viewport(948.);
        assert_eq!(view.display_size(), Some([375., 225.]));
        view.fit_width();
        assert_eq!(view.display_size(), Some([450., 270.]));
        view.set_viewport(20.);
        assert_eq!(view.effective_scale(), 0.2);
    }

    #[cfg(feature = "visual-test")]
    #[gpui::test]
    fn native_fit_button_sizes_pdf_pixels_to_the_rendered_pane(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.pdf");
        std::fs::write(&path, fixture()).unwrap();
        let mut doc = crate::files::Document::open(dir.path(), &path).unwrap();
        let pdf = doc.pdf.as_mut().unwrap();
        let raster = raster(fixture(), 0, 1.).unwrap();
        let image = egui::ColorImage::from_rgba_unmultiplied(raster.size, &raster.rgba);
        pdf.fit = false;
        pdf.scale = 1.;
        pdf.points = Some(raster.points);
        pdf.widest = raster.widest;
        pdf.pages = raster.pages;
        pdf.gpui_image = crate::gpui_pixels::from_color_image(&image);
        let window = cx.add_window(|_, cx| {
            let mut root = crate::gpui_ui::TesselGpuiRoot::new_in_dir(
                Some(dir.path().join("state")),
                false,
                cx,
            );
            root.app
                .apply_gpui_action(crate::app::gpui_bridge::GpuiAction::AddProjectPath(
                    dir.path().into(),
                ));
            root.app.install_editor_input_fixture(vec![doc], 0);
            root
        });
        cx.refresh().unwrap();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        let before = visual.debug_bounds("pdf-page-image").unwrap();
        assert_eq!(f32::from(before.size.width), 300.);
        let position = visual.debug_bounds("pdf-fit").unwrap().center();
        visual.simulate_click(position, Default::default());
        cx.refresh().unwrap();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        let expected = (f32::from(visual.debug_bounds("pdf-page-viewport").unwrap().size.width)
            - 48.)
            .clamp(60., 1200.);
        let actual = f32::from(visual.debug_bounds("pdf-page-image").unwrap().size.width);
        assert!(
            (expected - actual).abs() < 2.,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn original_pdf_renders_two_pages() {
        let Raster {
            size: [w, h],
            pages: n,
            rgba: pixels,
            ..
        } = raster(fixture(), 1, 1.0).unwrap();
        assert_eq!((w, h, n), (300, 180, 2));
        assert!(
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[2] > p[0].saturating_add(40))
        );
        assert!(raster(fixture(), 2, 1.0).is_err());
    }
}
