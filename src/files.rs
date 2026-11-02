use crate::workspace::{Result, atomic_write};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

pub const MAX_TEXT: usize = 2 * 1024 * 1024;

#[derive(Clone)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub directory: bool,
}

pub fn confined(root: &Path, path: &Path) -> Result<PathBuf> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    if !path.starts_with(&root) {
        return Err("Path is outside this project (including symlinks).".into());
    }
    Ok(path)
}

pub fn list(root: &Path, path: &Path) -> Result<Vec<Entry>> {
    let path = confined(root, path)?;
    let mut entries = Vec::new();
    for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entries.len() == 10000 {
            return Err("Directory exceeds 10,000 entries. Open a smaller project folder.".into());
        }
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        // Do not traverse links automatically. Explicit navigation checks confinement too.
        if kind.is_symlink() {
            continue;
        }
        entries.push(Entry {
            name: entry.file_name().to_string_lossy().into_owned(),
            path: entry.path(),
            directory: kind.is_dir(),
        });
    }
    entries.sort_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

fn read(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take((MAX_TEXT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_TEXT {
        return Err("Text editor limit is 2 MiB.".into());
    }
    Ok(bytes)
}

pub struct Document {
    pub root: PathBuf,
    pub path: PathBuf,
    pub text: String,
    original: Vec<u8>,
    pub error: String,
    pub preview_mode: bool,
    pub preview: Option<crate::preview::Preview>,
    pub image: Option<eframe::egui::ColorImage>,
    pub texture: Option<eframe::egui::TextureHandle>,
    pub media: bool,
    pub pdf: Option<crate::pdf::PdfView>,
}
impl Document {
    pub fn open(root: &Path, path: &Path) -> Result<Self> {
        let path = confined(root, path)?;
        let extension = path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        if extension == "pdf" {
            let pdf = crate::pdf::PdfView::open(&path)?;
            return Ok(Self {
                root: root.to_owned(),
                path,
                text: String::new(),
                original: vec![],
                error: String::new(),
                preview_mode: true,
                preview: None,
                image: None,
                texture: None,
                media: true,
                pdf: Some(pdf),
            });
        }
        if matches!(
            extension.as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp"
        ) {
            let mut bytes = Vec::new();
            fs::File::open(&path)
                .map_err(|e| e.to_string())?
                .take(16 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() > 16 * 1024 * 1024 {
                return Err("Image limit is 16 MiB".into());
            }
            let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
                .with_guessed_format()
                .map_err(|e| e.to_string())?;
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(4096);
            limits.max_image_height = Some(4096);
            limits.max_alloc = Some(64 * 1024 * 1024);
            reader.limits(limits);
            let decoded = reader.decode().map_err(|e| e.to_string())?.to_rgba8();
            let image = eframe::egui::ColorImage::from_rgba_unmultiplied(
                [decoded.width() as usize, decoded.height() as usize],
                decoded.as_raw(),
            );
            return Ok(Self {
                root: root.to_owned(),
                path,
                text: String::new(),
                original: vec![],
                error: String::new(),
                preview_mode: true,
                preview: None,
                image: Some(image),
                texture: None,
                media: true,
                pdf: None,
            });
        }
        let original = read(&path)?;
        if original.contains(&0) {
            return Err("Binary file: text editing is disabled.".into());
        }
        let text = String::from_utf8(original.clone())
            .map_err(|_| "This file is not UTF-8 text.".to_string())?;
        Ok(Self {
            root: root.to_owned(),
            path,
            text,
            original,
            error: String::new(),
            preview_mode: matches!(extension.as_str(), "html" | "htm" | "md" | "markdown"),
            preview: None,
            image: None,
            texture: None,
            media: false,
            pdf: None,
        })
    }
    pub fn dirty(&self) -> bool {
        !self.media && self.text.as_bytes() != self.original
    }
    pub fn save(&mut self) -> Result<()> {
        if self.media {
            return Err("This document is read-only".into());
        }
        let actual = confined(&self.root, &self.path)?;
        if actual != self.path {
            return Err("File target changed since opening.".into());
        }
        if read(&self.path)? != self.original {
            return Err("File changed on disk. Save is blocked to preserve external changes; compare or reload first.".into());
        }
        if self.text.len() > MAX_TEXT {
            return Err("Text editor limit is 2 MiB.".into());
        }
        atomic_write(&self.path, self.text.as_bytes())?;
        self.original = self.text.as_bytes().to_vec();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editor_preserves_external_changes_and_line_endings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("text.txt");
        fs::write(&path, "a\r\nb\r\n").unwrap();
        let mut doc = Document::open(dir.path(), &path).unwrap();
        doc.text.push('c');
        doc.save().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "a\r\nb\r\nc");
        fs::write(&path, "someone else").unwrap();
        doc.text.push('d');
        assert!(doc.save().is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "someone else");
    }
    #[test]
    fn images_decode_read_only_and_reject_oversized_dimensions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.png");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]))
            .save(&path)
            .unwrap();
        let before = fs::read(&path).unwrap();
        let mut doc = Document::open(dir.path(), &path).unwrap();
        assert!(doc.media);
        assert_eq!(doc.image.as_ref().unwrap().size, [2, 2]);
        assert!(!doc.dirty());
        assert!(doc.save().is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        image::RgbaImage::from_pixel(4097, 1, image::Rgba([0, 0, 0, 255]))
            .save(&path)
            .unwrap();
        assert!(Document::open(dir.path(), &path).is_err());
    }
    #[test]
    fn rejects_outside_project_and_binary() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::NamedTempFile::new().unwrap();
        assert!(Document::open(dir.path(), other.path()).is_err());
        let path = dir.path().join("binary");
        fs::write(&path, [0, 1, 2]).unwrap();
        assert!(Document::open(dir.path(), &path).is_err());
    }
}
