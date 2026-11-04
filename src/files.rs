use crate::workspace::{Result, atomic_write};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

pub const MAX_TEXT: usize = 2 * 1024 * 1024;

pub fn confined(root: &Path, path: &Path) -> Result<PathBuf> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    if !path.starts_with(&root) {
        return Err("Path is outside this project (including symlinks).".into());
    }
    Ok(path)
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

#[derive(Clone, Debug, Default)]
pub struct EditorTools {
    pub word_wrap: bool,
    pub find_open: bool,
    pub replace_open: bool,
    pub goto_open: bool,
    pub find_query: String,
    pub replace_text: String,
    pub goto_line: String,
    pub match_index: usize,
    pub diff_hunk_index: usize,
    pub focus_find: bool,
    pub focus_goto: bool,
    pub pending_char_range: Option<(usize, usize)>,
    pub pending_scroll_line: Option<usize>,
}

/// Case-insensitive substring search returning `(start_char, end_char, zero_based_line)` tuples.
pub fn find_char_matches(text: &str, query: &str) -> Vec<(usize, usize, usize)> {
    if query.is_empty() {
        return Vec::new();
    }
    let needle_lower = query.to_lowercase();
    if needle_lower.is_empty() {
        return Vec::new();
    }
    // Map byte offsets in `hay_lower` safely when ASCII, or scan line-by-line by char index for Unicode:
    let text_chars: Vec<char> = text.chars().collect();
    let folded_chars: Vec<String> = text_chars
        .iter()
        .map(|c| c.to_lowercase().collect())
        .collect();
    let mut flat = String::with_capacity(text.len());
    let mut byte_to_char = Vec::with_capacity(text.len() + 1);
    for (ci, piece) in folded_chars.iter().enumerate() {
        for _ in 0..piece.len() {
            byte_to_char.push(ci);
        }
        flat.push_str(piece);
    }
    byte_to_char.push(text_chars.len());

    let mut line_at_char = Vec::with_capacity(text_chars.len() + 1);
    let mut cur_line = 0usize;
    for &ch in &text_chars {
        line_at_char.push(cur_line);
        if ch == '\n' {
            cur_line += 1;
        }
    }
    line_at_char.push(cur_line);

    let mut out = Vec::new();
    let mut pos = 0usize;
    while let Some(rel) = flat[pos..].find(&needle_lower) {
        let start_b = pos + rel;
        let end_b = start_b + needle_lower.len();
        if let (Some(&sc), Some(&ec)) = (byte_to_char.get(start_b), byte_to_char.get(end_b))
            && ec > sc
        {
            let line = line_at_char.get(sc).copied().unwrap_or(0);
            out.push((sc, ec, line));
        }
        pos = start_b + needle_lower.chars().next().map_or(1, char::len_utf8);
        if pos > flat.len() {
            break;
        }
    }
    out
}

pub fn char_range_to_byte_range(text: &str, start_char: usize, end_char: usize) -> (usize, usize) {
    let mut start_byte = text.len();
    let mut end_byte = text.len();
    for (ci, (bi, _)) in text.char_indices().enumerate() {
        if ci == start_char {
            start_byte = bi;
        }
        if ci == end_char {
            end_byte = bi;
            break;
        }
    }
    if start_char >= end_char {
        end_byte = start_byte;
    }
    (start_byte, end_byte)
}

/// Converts a 1-based `"line"` or `"line:col"` input into `(char_offset, zero_based_line)`.
pub fn goto_line_char_offset(text: &str, spec: &str) -> Option<(usize, usize)> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (line_part, col_part) = trimmed
        .split_once(':')
        .map_or((trimmed, None), |(l, c)| (l.trim(), Some(c.trim())));
    let line_1 = line_part.parse::<usize>().ok()?.max(1);
    let col_1 = col_part
        .and_then(|c| c.parse::<usize>().ok())
        .unwrap_or(1)
        .max(1);
    let target_line = line_1 - 1;
    let mut cur_line = 0usize;
    let mut line_start_char = 0usize;
    let mut line_len_chars = 0usize;
    let mut found_line = target_line == 0;
    for (ci, ch) in text.chars().enumerate() {
        if cur_line == target_line {
            if ch == '\n' || ch == '\r' {
                break;
            }
            line_len_chars += 1;
        } else if ch == '\n' {
            cur_line += 1;
            if cur_line == target_line {
                line_start_char = ci + 1;
                found_line = true;
            }
        }
    }
    if !found_line {
        // Clamp to the last line of the buffer
        let total_chars = text.chars().count();
        return Some((total_chars, cur_line));
    }
    let col_offset = (col_1 - 1).min(line_len_chars);
    Some((line_start_char + col_offset, target_line))
}

/// Computes the 0-based `(char_offset, zero_based_line)` start of contiguous changed hunks
/// between `original` and `current`, re-synchronizing across inserted and deleted lines.
pub fn changed_line_hunks(original: &str, current: &str) -> Vec<(usize, usize)> {
    if original == current {
        return Vec::new();
    }
    let orig_lines: Vec<&str> = original.split('\n').collect();
    let curr_lines: Vec<&str> = current.split('\n').collect();
    let mut char_offsets = Vec::with_capacity(curr_lines.len().max(1));
    let mut acc = 0usize;
    for line in &curr_lines {
        char_offsets.push(acc);
        acc += line.chars().count() + 1;
    }

    let mut prefix = 0usize;
    while prefix < orig_lines.len()
        && prefix < curr_lines.len()
        && orig_lines[prefix] == curr_lines[prefix]
    {
        prefix += 1;
    }
    let mut orig_end = orig_lines.len();
    let mut curr_end = curr_lines.len();
    while orig_end > prefix
        && curr_end > prefix
        && orig_lines[orig_end - 1] == curr_lines[curr_end - 1]
    {
        orig_end -= 1;
        curr_end -= 1;
    }

    let o_mid = &orig_lines[prefix..orig_end];
    let c_mid = &curr_lines[prefix..curr_end];
    let m = o_mid.len();
    let n = c_mid.len();

    // Collect matched `(orig_idx, curr_idx)` pairs within `o_mid` / `c_mid`.
    let mut anchors: Vec<(usize, usize)> = Vec::new();
    if m > 0 && n > 0 && m.saturating_mul(n) <= 1_500_000 {
        let cols = n + 1;
        let mut dp = vec![0u32; (m + 1) * cols];
        for i in (0..m).rev() {
            for j in (0..n).rev() {
                dp[i * cols + j] = if o_mid[i] == c_mid[j] {
                    dp[(i + 1) * cols + (j + 1)] + 1
                } else {
                    dp[(i + 1) * cols + j].max(dp[i * cols + (j + 1)])
                };
            }
        }
        let (mut i, mut j) = (0usize, 0usize);
        while i < m && j < n {
            if o_mid[i] == c_mid[j] {
                anchors.push((prefix + i, prefix + j));
                i += 1;
                j += 1;
            } else if dp[(i + 1) * cols + j] >= dp[i * cols + (j + 1)] {
                i += 1;
            } else {
                j += 1;
            }
        }
    } else if m > 0 && n > 0 {
        let (mut i, mut j) = (0usize, 0usize);
        while i < m && j < n {
            if o_mid[i] == c_mid[j] {
                anchors.push((prefix + i, prefix + j));
                i += 1;
                j += 1;
            } else if let Some(dj) = (1..=64.min(n - j - 1)).find(|&d| o_mid[i] == c_mid[j + d]) {
                j += dj;
            } else if let Some(di) = (1..=64.min(m - i - 1)).find(|&d| o_mid[i + d] == c_mid[j]) {
                i += di;
            } else {
                i += 1;
                j += 1;
            }
        }
    }
    anchors.push((orig_end, curr_end));

    let mut hunks = Vec::new();
    let mut prev_o = prefix;
    let mut prev_c = prefix;
    for (ao, ac) in anchors {
        if ao > prev_o || ac > prev_c {
            let clamped_line = prev_c.min(curr_lines.len().saturating_sub(1));
            let char_pos = char_offsets.get(clamped_line).copied().unwrap_or(0);
            if hunks.last().is_none_or(|&(_, l)| l != clamped_line) {
                hunks.push((char_pos, clamped_line));
            }
        }
        prev_o = ao + 1;
        prev_c = ac + 1;
    }
    hunks
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
    pub office: Option<crate::office::OfficeView>,
    pub tools: EditorTools,
}
impl Document {
    pub fn open(root: &Path, path: &Path) -> Result<Self> {
        let path = confined(root, path)?;
        let extension = path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        if crate::office::supported(&extension) {
            let office = crate::office::OfficeView::open(&path)?;
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
                pdf: None,
                office: Some(office),
                tools: EditorTools::default(),
            });
        }
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
                office: None,
                tools: EditorTools::default(),
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
                office: None,
                tools: EditorTools::default(),
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
            office: None,
            tools: EditorTools::default(),
        })
    }
    pub fn original_text(&self) -> Option<&str> {
        std::str::from_utf8(&self.original).ok()
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

    #[test]
    fn editor_find_goto_line_and_changed_hunks_handle_multibyte_utf8() {
        let text = "première ligne\nCafé résumé\ntroisième café\n";
        let matches = find_char_matches(text, "café");
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].2, 1);
        assert_eq!(matches[1].2, 2);

        let (sb, eb) = char_range_to_byte_range(text, matches[0].0, matches[0].1);
        assert_eq!(&text[sb..eb], "Café");

        assert_eq!(goto_line_char_offset(text, "2:6"), Some((15 + 5, 1)));
        assert_eq!(
            goto_line_char_offset(text, "999"),
            Some((text.chars().count(), 3))
        );
        assert_eq!(goto_line_char_offset(text, ""), None);

        let orig = "alpha\nbeta\ngamma\ndelta";
        let curr = "alpha\nBETA\nB2\ngamma\nDELTA";
        let hunks = changed_line_hunks(orig, curr);
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].1, 1);
        assert_eq!(hunks[1].1, 4);
    }
}
