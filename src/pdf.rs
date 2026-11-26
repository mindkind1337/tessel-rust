//! Original-page PDF rasterization in a disposable, deadline-bound worker.
mod continuous;
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
const HEADER_BYTES: usize = 12;
const MAX_PAGES: usize = 2000;

fn manual_zoom(scale: f32, step: f32) -> f32 {
    (((scale + step) * 4.).round() / 4.).clamp(MIN_ZOOM, MAX_ZOOM)
}

pub struct Rendered {
    page_sizes: Vec<[f32; 2]>,
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
    page_sizes: Vec<[f32; 2]>,
    rgba: Vec<u8>,
}

fn raster(bytes: Vec<u8>, index: usize, scale: f32) -> Result<Raster, String> {
    if bytes.len() > MAX_INPUT {
        return Err("PDF limit is 32 MiB".into());
    }
    let pdf = hayro::hayro_syntax::Pdf::new(bytes).map_err(|_| "Unsupported or invalid PDF")?;
    let pages = pdf.pages();
    if pages.is_empty() || pages.len() > MAX_PAGES {
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
        page_sizes: pages
            .iter()
            .map(|page| {
                let (width, height) = page.render_dimensions();
                [width, height]
            })
            .collect(),
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
    for n in [raster.size[0], raster.size[1], raster.page_sizes.len()] {
        output
            .write_all(&(n as u32).to_le_bytes())
            .map_err(|_| "Cannot write rendering")?;
    }
    for n in raster.page_sizes.iter().flatten() {
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
        .take((MAX_PIXELS * 4 + HEADER_BYTES + MAX_PAGES * 8 + 1) as u64)
        .read_to_end(&mut result)
        .map_err(|_| "Cannot read PDF rendering")?;
    decode(&result, index)
}
fn decode(bytes: &[u8], index: usize) -> Result<Rendered, String> {
    if bytes.len() < HEADER_BYTES {
        return Err("Incomplete PDF rendering".into());
    }
    let width = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
    let height = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let pages = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    if !(1..=MAX_PAGES).contains(&pages)
        || index >= pages
        || width == 0
        || height == 0
        || width > 4096
        || height > 4096
        || width * height > MAX_PIXELS
    {
        return Err("Invalid PDF rendering dimensions".into());
    }
    let pixel_offset = HEADER_BYTES + pages * 8;
    if bytes.len() != pixel_offset + width * height * 4 {
        return Err("Incomplete PDF rendering".into());
    }
    let mut page_sizes = Vec::with_capacity(pages);
    let mut widest = 0_f32;
    let mut total_height = 0_f32;
    for entry in bytes[HEADER_BYTES..pixel_offset].as_chunks::<8>().0 {
        let size = [
            f32::from_le_bytes(entry[..4].try_into().unwrap()),
            f32::from_le_bytes(entry[4..].try_into().unwrap()),
        ];
        total_height += size[1] * MAX_ZOOM;
        if !total_height.is_finite()
            || !size
                .iter()
                .all(|n| n.is_finite() && *n > 0. && (*n * MAX_ZOOM).is_finite())
        {
            return Err("Invalid PDF page size".into());
        }
        widest = widest.max(size[0]);
        page_sizes.push(size);
    }
    Ok(Rendered {
        page_sizes,
        widest,
        image: egui::ColorImage::from_rgba_unmultiplied([width, height], &bytes[pixel_offset..]),
        pages,
    })
}
#[derive(Clone, Copy, Debug)]
pub struct PageLayout {
    pub viewport: egui::Rect,
    pub extent: egui::Vec2,
    pub fit: bool,
}
#[derive(Clone, Debug)]
pub struct PagePreview {
    pub index: usize,
    pub rect: egui::Rect,
    pub pixels: Option<Arc<gpui::RenderImage>>,
}
#[derive(Clone, Debug)]
pub struct PdfSummary {
    pub visible: Vec<PagePreview>,
    pub pages: usize,
    pub scale: f32,
    pub busy: bool,
    pub error: String,
    pub layout: PageLayout,
}

pub struct PdfView {
    bytes: Arc<Vec<u8>>,
    pages: usize,
    scale: f32,
    fit: bool,
    widest: f32,
    viewport: f32,
    continuous: continuous::ContinuousPages,
    requested_page: usize,
    requested_scale: f32,
    native_sizes: Vec<[f32; 2]>,
    native_pages: std::collections::BTreeMap<usize, (f32, Arc<gpui::RenderImage>)>,
    native_failures: std::collections::BTreeMap<usize, (f32, String)>,
    native_viewport: egui::Rect,
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
            pages: 0,
            scale: 1.5,
            fit: true,
            widest: 0.,
            viewport: 0.,
            continuous: Default::default(),
            requested_page: 0,
            requested_scale: 1.5,
            native_sizes: Vec::new(),
            native_pages: Default::default(),
            native_failures: Default::default(),
            native_viewport: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::ZERO),
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
                    self.pages = page.pages;
                    self.native_sizes = page.page_sizes;
                    if let Some(image) = crate::gpui_pixels::from_color_image(&page.image) {
                        self.native_pages
                            .insert(self.requested_page, (self.requested_scale, image));
                        self.native_failures.remove(&self.requested_page);
                    }
                }
                Err(e) => {
                    self.native_failures
                        .insert(self.requested_page, (self.requested_scale, e));
                }
            }
        }
        let scale = self.effective_scale();
        let render_scale = scale.clamp(MIN_ZOOM, MAX_ZOOM);
        let plan =
            continuous::plan_pages(&self.native_sizes, self.native_viewport, scale, self.widest);
        if self.native_viewport.is_positive() {
            self.native_pages.retain(|i, _| plan.wanted.contains(i));
        }
        let wanted = if self.native_sizes.is_empty() {
            &[0][..]
        } else {
            plan.wanted.as_slice()
        };
        if self.pending.is_none()
            && let Some(index) = wanted.iter().copied().find(|i| {
                self.native_pages
                    .get(i)
                    .is_none_or(|(s, _)| *s != render_scale)
                    && self
                        .native_failures
                        .get(i)
                        .is_none_or(|(s, _)| *s != render_scale)
            })
        {
            self.start_page_worker(index, render_scale);
        }
        let error = wanted
            .iter()
            .find_map(|i| {
                self.native_failures
                    .get(i)
                    .filter(|(s, _)| *s == render_scale)
            })
            .map(|(_, e)| e.clone())
            .unwrap_or_default();
        PdfSummary {
            visible: plan
                .visible_pages
                .into_iter()
                .map(|(index, rect)| PagePreview {
                    index,
                    rect,
                    pixels: self
                        .native_pages
                        .get(&index)
                        .map(|(_, image)| image.clone()),
                })
                .collect(),
            pages: self.pages,
            scale,
            busy: self.pending.is_some(),
            error,
            layout: PageLayout {
                viewport: self.native_viewport,
                extent: plan.extent,
                fit: self.fit,
            },
        }
    }
    pub fn set_native_viewport(&mut self, rect: egui::Rect) -> bool {
        if !rect.is_finite() || !rect.is_positive() || rect == self.native_viewport {
            return false;
        }
        self.native_viewport = rect;
        self.set_viewport(rect.width());
        true
    }
    pub(crate) fn is_rendering(&self) -> bool {
        self.pending.is_some()
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

    pub fn step_zoom(&mut self, delta: f32) {
        let next = manual_zoom(self.effective_scale(), delta);
        if self.fit || (next - self.scale).abs() > 0.01 {
            self.scale = next;
            self.fit = false;
        }
    }
    #[cfg(feature = "visual-test")]
    pub fn prime(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let page = render(&self.bytes, 0, self.scale, &self.cancel)?;
        self.widest = page.widest;
        self.pages = page.pages;
        self.continuous.store(ctx, 0, self.scale, page);
        Ok(())
    }
    fn start_page_worker(&mut self, index: usize, scale: f32) {
        let bytes = self.bytes.clone();
        self.requested_page = index;
        self.requested_scale = scale;
        let cancel = self.cancel.clone();
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        self.error.clear();
        std::thread::spawn(move || {
            let _ = tx.send(render(&bytes, index, scale, &cancel));
        });
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
        view.fit = false;
        view.scale = 1.;
        view.requested_scale = 1.;
        view.set_native_viewport(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(500., 400.),
        ));
        let (sender, receiver) = mpsc::channel();
        view.pending = Some(receiver);
        let raster = raster(fixture(), 0, 1.).unwrap();
        let [width, height] = raster.size;
        sender
            .send(Ok(Rendered {
                image: egui::ColorImage::from_rgba_unmultiplied([width, height], &raster.rgba),
                pages: raster.page_sizes.len(),
                widest: raster.page_sizes[0][0],
                page_sizes: raster.page_sizes,
            }))
            .unwrap();
        let summary = view.gpui_poll_summary();
        assert_eq!(summary.pages, 2);
        let image = view.native_pages[&0].1.clone();
        assert_eq!(image.as_bytes(0).unwrap().len(), width * height * 4);
        assert!(Arc::ptr_eq(
            &image,
            summary.visible[0].pixels.as_ref().unwrap()
        ));
        let (sender, receiver) = mpsc::channel();
        view.pending = Some(receiver);
        assert_eq!(view.requested_page, 1);
        drop(sender);
        let summary = view.gpui_poll_summary();
        assert!(!summary.busy);
        assert!(summary.error.contains("stopped"));
        assert!(view.pending.is_none());
    }
    #[test]
    fn scrolling_releases_distant_pdf_pages_from_the_cache_and_window() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.pdf");
        std::fs::write(&path, fixture()).unwrap();
        let mut view = PdfView::open(&path).unwrap();
        view.fit = false;
        view.scale = 1.;
        view.native_sizes = vec![[300., 800.]; 8];
        let pixels = egui::ColorImage::from_rgba_unmultiplied([1, 1], &[1, 2, 3, 255]);
        let first = crate::gpui_pixels::from_color_image(&pixels).unwrap();
        view.native_pages.insert(0, (1., first.clone()));
        let mut atlas = crate::gpui_pixels::PresentedImages::default();
        for (offset, expected) in [(0., 0), (5700., 7)] {
            view.set_native_viewport(egui::Rect::from_min_size(
                egui::pos2(0., offset),
                egui::vec2(500., 400.),
            ));
            if expected == 7 {
                view.native_pages.insert(
                    7,
                    (1., crate::gpui_pixels::from_color_image(&pixels).unwrap()),
                );
            }
            let summary = view.gpui_poll_summary();
            assert_eq!(summary.visible.len(), 1);
            assert_eq!(summary.visible[0].index, expected);
            let retired = atlas.replace(summary.visible.into_iter().filter_map(|page| page.pixels));
            if expected == 7 {
                assert!(!view.native_pages.contains_key(&0));
                assert_eq!(retired.len(), 1);
                assert!(Arc::ptr_eq(&retired[0], &first));
            }
        }
    }
    #[test]
    fn worker_reply_validates_the_entire_page_table_before_reading_pixels() {
        let mut reply = Vec::new();
        for n in [1u32, 1, 2] {
            reply.extend(n.to_le_bytes());
        }
        for n in [300f32, 180., 600., 320.] {
            reply.extend(n.to_le_bytes());
        }
        reply.extend([0, 0, 255, 255]);
        let page = decode(&reply, 1).unwrap();
        assert_eq!(page.page_sizes[1], [600., 320.]);
        assert_eq!(page.page_sizes, [[300., 180.], [600., 320.]]);
        assert!(decode(&reply, 2).is_err());
        assert!(decode(&reply[..reply.len() - 1], 0).is_err());
        reply[24..28].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(decode(&reply, 0).is_err());
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
        assert!(decode(&[0; 12], 0).is_err());
    }
    #[test]
    fn fit_tracks_widest_page_and_manual_zoom_preserves_physical_dimensions() {
        let mut source = fixture();
        let at = source.windows(7).rposition(|s| s == b"300 180").unwrap();
        source[at..at + 7].copy_from_slice(b"600 180");
        let first = raster(source, 0, 4.).unwrap();
        assert_eq!(first.page_sizes, [[300., 180.], [600., 180.]]);
        assert_eq!(first.size, [1200, 720]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.pdf");
        std::fs::write(&path, fixture()).unwrap();
        let mut view = PdfView::open(&path).unwrap();
        view.widest = first.page_sizes[1][0];
        view.set_viewport(948.);
        assert_eq!(view.effective_scale(), 1.5);
        view.set_viewport(648.);
        assert_eq!(view.effective_scale(), 1.);
        view.step_zoom(0.25);
        assert_eq!(view.effective_scale(), 1.25);
        view.set_viewport(948.);
        assert_eq!(view.effective_scale(), 1.25);
        view.fit_width();
        assert_eq!(view.effective_scale(), 1.5);
        view.set_viewport(20.);
        assert_eq!(view.effective_scale(), 0.2);
    }

    #[cfg(feature = "visual-test")]
    #[gpui::test]
    fn native_pdf_fits_and_scrolls_between_original_pages(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.pdf");
        let mut source = fixture();
        let offset = source.windows(7).rposition(|v| v == b"300 180").unwrap();
        source[offset..offset + 7].copy_from_slice(b"300 980");
        std::fs::write(&path, &source).unwrap();
        let mut doc = crate::files::Document::open(dir.path(), &path).unwrap();
        let pdf = doc.pdf.as_mut().unwrap();
        pdf.fit = false;
        pdf.scale = 1.;
        for index in 0..2 {
            let raster = raster(source.clone(), index, 1.).unwrap();
            let image = egui::ColorImage::from_rgba_unmultiplied(raster.size, &raster.rgba);
            pdf.widest = 300.;
            pdf.pages = raster.page_sizes.len();
            pdf.native_sizes = raster.page_sizes;
            pdf.native_pages.insert(
                index,
                (1., crate::gpui_pixels::from_color_image(&image).unwrap()),
            );
        }
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
        let before = visual.debug_bounds("pdf-page-image-0").unwrap();
        assert_eq!(f32::from(before.size.width), 300.);
        let position = visual.debug_bounds("pdf-fit").unwrap().center();
        visual.simulate_click(position, Default::default());
        cx.refresh().unwrap();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        let expected = (f32::from(visual.debug_bounds("pdf-page-viewport").unwrap().size.width)
            - 48.)
            .clamp(60., 1200.);
        let actual = f32::from(visual.debug_bounds("pdf-page-image-0").unwrap().size.width);
        assert!(
            (expected - actual).abs() < 2.,
            "expected {expected}, got {actual}"
        );
        let pane = visual.debug_bounds("pdf-page-viewport").unwrap();
        visual.simulate_event(gpui::ScrollWheelEvent {
            position: pane.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(-2500.))),
            modifiers: Default::default(),
            touch_phase: gpui::TouchPhase::Moved,
        });
        cx.refresh().unwrap();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        assert!(visual.debug_bounds("pdf-page-image-0").is_none());
        let next = visual
            .debug_bounds("pdf-page-image-1")
            .expect("second PDF page");
        assert!(next.origin.y < pane.origin.y);
        assert!(
            ((f32::from(next.size.height) / f32::from(next.size.width)) - 980. / 300.).abs() < 0.03
        );
    }

    #[test]
    fn original_pdf_renders_two_pages() {
        let Raster {
            size: [w, h],
            page_sizes,
            rgba: pixels,
            ..
        } = raster(fixture(), 1, 1.0).unwrap();
        assert_eq!((w, h, page_sizes.len()), (300, 180, 2));
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
