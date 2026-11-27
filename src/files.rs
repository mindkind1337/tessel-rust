use crate::workspace::{Result, prepare_atomic_write};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[path = "playback.rs"]
pub mod playback;
const DISK_CHANGED: &str =
    "File changed on disk. Compare the versions, reload the file, or keep your edits.";
pub const MAX_TEXT: usize = 2 * 1024 * 1024;
pub const MAX_LOCAL_EDIT_BYTES: usize = 50 * 1024 * 1024;

pub fn confined(root: &Path, path: &Path) -> Result<PathBuf> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    if !path.starts_with(&root) {
        return Err("Path is outside this project (including symlinks).".into());
    }
    Ok(path)
}

pub fn fitted_image_size(
    natural: eframe::egui::Vec2,
    available: eframe::egui::Vec2,
) -> eframe::egui::Vec2 {
    if natural.x <= 0. || natural.y <= 0. {
        return eframe::egui::Vec2::ZERO;
    }
    let ratio = (available.x / natural.x)
        .min(available.y / natural.y)
        .clamp(0., 1.);
    natural * ratio
}

pub fn show_fitted_image(
    ui: &mut eframe::egui::Ui,
    texture: &eframe::egui::TextureHandle,
) -> eframe::egui::Response {
    use eframe::egui;
    let area = ui.available_rect_before_wrap();
    let size = fitted_image_size(texture.size_vec2(), area.size() - egui::Vec2::splat(40.));
    ui.put(
        egui::Rect::from_center_size(area.center(), size),
        egui::Image::new(texture)
            .fit_to_exact_size(size)
            .sense(egui::Sense::click()),
    )
}

pub fn save_download(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let parent = path.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist_noclobber(path)?;
    Ok(())
}

const UTF8_BOM: char = '\u{feff}';

fn split_bom(mut text: String) -> (String, bool) {
    let bom = text.starts_with(UTF8_BOM);
    if bom {
        text.drain(..UTF8_BOM.len_utf8());
    }
    (text, bom)
}

pub(crate) fn has_binary_prefix(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(8192)].contains(&0)
}

fn read_editor_text(path: &Path) -> Result<(String, bool)> {
    let bytes = read(path)?;
    if has_binary_prefix(&bytes) {
        return Err("Binary file: text editing is disabled.".into());
    }
    String::from_utf8(bytes)
        .map(split_bom)
        .map_err(|_| "This file is not UTF-8 text.".into())
}

pub(crate) fn read(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("This is not a regular file.".into());
    }
    if metadata.len() > MAX_LOCAL_EDIT_BYTES as u64 {
        return Err("Text editor limit is 50 MiB.".into());
    }
    bytes.reserve_exact(metadata.len() as usize);
    file.take((MAX_LOCAL_EDIT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_LOCAL_EDIT_BYTES {
        return Err("Text editor limit is 50 MiB.".into());
    }
    Ok(bytes)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutlineCursor {
    Bytes(usize),
    Chars(usize),
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ConflictAction {
    Compare,
    Reload,
    Keep,
}

impl ConflictAction {
    pub(crate) const ALL: [Self; 3] = [Self::Compare, Self::Reload, Self::Keep];
    pub(crate) fn label(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::Compare, false) => "Compare",
            (Self::Compare, true) => "Comparer",
            (Self::Reload, false) => "Reload from Disk",
            (Self::Reload, true) => "Recharger depuis le disque",
            (Self::Keep, false) => "Keep My Edits",
            (Self::Keep, true) => "Conserver mes modifications",
        }
    }
}

#[derive(Debug)]
pub(crate) struct DiskComparison {
    draft: String,
    pub(crate) patch: std::sync::OnceLock<crate::saved_version_diff::Patch>,
}

#[derive(Clone, Debug)]
pub(crate) struct DisplayText {
    pub text: std::sync::Arc<String>,
    pub line_count: usize,
    pub dirty: bool,
    pub hunk_lines: std::sync::Arc<[usize]>,
}

#[derive(Clone, Debug, Default)]
pub struct EditorTools {
    display_text: Option<DisplayText>,
    pub(crate) disk_revision: u64,
    pub(crate) disk_reload: u64,
    pub(crate) conflict_disk: Option<std::sync::Arc<str>>,
    comparison: Option<std::sync::Arc<DiskComparison>>,
    #[cfg(windows)]
    pub html_root: Option<PathBuf>,
    pub html_plain_text: Option<std::sync::Arc<str>>,
    pub html_link_menu: Option<((f32, f32), String, bool)>,
    pub image_menu: Option<(f32, f32)>,
    pub image_download_notice: String,
    pub outline_cursor: Option<OutlineCursor>,
    pub outline_scroll_generation: u64,
    pub outline_selected: Option<String>,
    pub outline_reveal_focus: Option<bool>,
    pub outline_folds: std::collections::HashSet<String>,
    pub find_open: bool,
    pub replace_open: bool,
    pub goto_open: bool,
    pub find_query: String,
    pub replace_text: String,
    pub goto_line: String,
    pub match_index: usize,
    pub diff_hunk_index: usize,
    pub focus_find: bool,
    pub focus_replace: bool,
    pub focus_goto: bool,
    pub pending_char_range: Option<(usize, usize)>,
    pub pending_scroll_line: Option<usize>,
}

/// Non-overlapping literal matches in character coordinates and zero-based lines.
pub fn find_char_matches(text: &str, query: &str) -> Vec<(usize, usize, usize)> {
    let mut matches = Vec::new();
    visit_find_matches(text, query, |hit| {
        matches.push(hit);
        true
    });
    matches
}

fn visit_find_matches(
    text: &str,
    query: &str,
    mut visit: impl FnMut((usize, usize, usize)) -> bool,
) {
    if query.is_empty() || text.is_empty() {
        return;
    }
    let fold = |s: &str| s.chars().flat_map(char::to_lowercase).collect::<String>();
    let needle = fold(query);
    if text.is_ascii() {
        let mut line = 0;
        let mut prev = 0;
        for (start, matched) in text.to_ascii_lowercase().match_indices(&needle) {
            line += text.as_bytes()[prev..start]
                .iter()
                .filter(|&&b| b == b'\n')
                .count();
            if !visit((start, start + matched.len(), line)) {
                break;
            }
            prev = start;
        }
        return;
    }
    let folded = fold(text);
    // A Unicode lowercase mapping can expand a source character into multiple scalars.
    // Track original boundaries as we search so a replacement never splits that character.
    let mut boundaries = text
        .chars()
        .scan((0usize, 0usize, 0usize), |pos, ch| {
            let before = *pos;
            pos.0 += ch.to_lowercase().map(char::len_utf8).sum::<usize>();
            pos.1 += 1;
            pos.2 += usize::from(ch == '\n');
            Some(before)
        })
        .chain(std::iter::once((folded.len(), text.chars().count(), 0)))
        .peekable();
    let mut offset = 0;
    while let Some(found) = folded[offset..].find(&needle) {
        let start = offset + found;
        while boundaries.next_if(|p| p.0 < start).is_some() {}
        let Some(&(boundary, first, line)) = boundaries.peek() else {
            break;
        };
        if boundary != start {
            offset = boundary;
            continue;
        }
        boundaries.next();
        let end = start + needle.len();
        let mut after = boundaries.clone();
        while after.next_if(|p| p.0 < end).is_some() {}
        if let Some(&(_, last, _)) = after.peek().filter(|p| p.0 == end) {
            if !visit((first, last, line)) {
                break;
            }
            boundaries = after;
            offset = end;
        } else {
            offset = boundaries.peek().map_or(folded.len(), |p| p.0);
        }
    }
}

/// Build one bounded edit so Replace All has one undo step in the native editor.
pub(crate) fn replacement_edit(
    source: &str,
    hits: &[(usize, usize, usize)],
    replacement: &str,
    index: usize,
    all: bool,
) -> Result<Option<(std::ops::Range<usize>, String)>> {
    if hits.is_empty() {
        return Ok(None);
    }
    let hits = if all {
        hits
    } else {
        std::slice::from_ref(&hits[index.min(hits.len() - 1)])
    };
    let mut offsets = source
        .char_indices()
        .map(|(byte, _)| byte)
        .chain(std::iter::once(source.len()))
        .enumerate()
        .peekable();
    let mut byte_at = |target: usize| {
        while offsets.next_if(|(i, _)| *i < target).is_some() {}
        offsets.peek().expect("match within source").1
    };
    let first = byte_at(hits[0].0);
    let mut cursor = first;
    let mut insert = String::new();
    let limit = "Text editor limit is 50 MiB.";
    for &(start, end, _) in hits {
        let gap = &source[cursor..byte_at(start)];
        let room = MAX_LOCAL_EDIT_BYTES.saturating_sub(first + insert.len());
        if gap.len() > room || replacement.len() > room - gap.len() {
            return Err(limit.into());
        }
        insert.push_str(gap);
        insert.push_str(replacement);
        cursor = byte_at(end);
    }
    if first + insert.len() + (source.len() - cursor) > MAX_LOCAL_EDIT_BYTES {
        return Err(limit.into());
    }
    Ok(Some((first..cursor, insert)))
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

/// Resolves a Go to Line entry to a character offset and zero-based line.
pub fn goto_line_char_offset(text: &str, spec: &str) -> Option<(usize, usize)> {
    // Monaco accepts a signed decimal prefix as JavaScript parseInt does.
    fn number(part: &str) -> Option<i64> {
        let part = part.trim_start();
        let digits = part.strip_prefix(['+', '-']).unwrap_or(part);
        let end = digits.bytes().take_while(u8::is_ascii_digit).count();
        if end == 0 {
            return None;
        }
        let value = digits[..end].parse::<i64>().unwrap_or(i64::MAX);
        Some(if part.starts_with('-') { -value } else { value })
    }

    let lines = || {
        let mut remaining = Some(text);
        std::iter::from_fn(move || {
            let rest = remaining.take()?;
            let end = rest.find(['\r', '\n']).unwrap_or(rest.len());
            let tail = &rest[end..];
            let eol = if tail.starts_with("\r\n") {
                2
            } else {
                usize::from(!tail.is_empty())
            };
            if eol != 0 {
                remaining = Some(&tail[eol..]);
            }
            Some((&rest[..end], eol))
        })
    };
    let spec = spec.trim();
    if let Some(offset) = spec.strip_prefix("::").or_else(|| spec.strip_prefix(':')) {
        let offset = number(offset)?;
        let mut left = if offset < 0 {
            (text.encode_utf16().count() as i64 + offset + 1).max(0)
        } else {
            offset.saturating_sub(1).max(0)
        } as usize;
        let mut chars = 0;
        let mut last_line = 0;
        for (index, (line, eol)) in lines().enumerate() {
            last_line = index;
            for ch in line.chars() {
                if left < ch.len_utf16() {
                    return Some((chars, index));
                }
                left -= ch.len_utf16();
                chars += 1;
            }
            if left < eol {
                return Some((chars, index));
            }
            left -= eol;
            chars += eol;
        }
        return Some((chars, last_line));
    }

    let mut parts = spec.split([',', ':', '#']);
    let requested = number(parts.next()?)?;
    let index = if requested < 0 {
        (lines().count() as i64 + requested).max(0)
    } else {
        requested.saturating_sub(1).max(0)
    } as usize;
    let mut it = lines().enumerate().peekable();
    let mut start = 0;
    while let Some((i, (line, eol))) = it.next() {
        if i == index || it.peek().is_none() {
            let col = parts.next().and_then(number).unwrap_or(1);
            let offset = if col == 1 || col == 0 {
                0
            } else {
                crate::editor_indent::column_offset(
                    line,
                    col,
                    crate::editor_indent::Options::detect(text).size,
                )
            };
            return Some((start + offset, i));
        }
        start += line.chars().count() + eol;
    }
    None
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

/// One-based file-link coordinates, with columns measured in UTF-16 units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FilePosition {
    pub line: usize,
    pub column: usize,
}

impl OutlineCursor {
    pub(crate) fn position(self, text: &str) -> FilePosition {
        let byte = match self {
            crate::files::OutlineCursor::Bytes(i) => text.floor_char_boundary(i.min(text.len())),
            crate::files::OutlineCursor::Chars(i) => text
                .char_indices()
                .nth(i)
                .map_or(text.len(), |(offset, _)| offset),
        };
        let mut line = 1;
        let mut column = 1;
        let mut chars = text[..byte].chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\r' || c == '\n' {
                if c == '\r' {
                    chars.next_if_eq(&'\n');
                }
                line += 1;
                column = 1;
            } else {
                column += c.len_utf16();
            }
        }
        FilePosition { line, column }
    }
}

pub struct Document {
    pub project: Option<u64>,
    instance_id: u64,
    pub root: PathBuf,
    pub path: PathBuf,
    pub text: String,
    original: Vec<u8>,
    utf8_bom: bool,
    pub error: String,
    pub preview_mode: bool,
    pub preview: Option<crate::preview::Preview>,
    pub image: Option<eframe::egui::ColorImage>,
    pub gpui_image: Option<std::sync::Arc<gpui::RenderImage>>,
    pub texture: Option<eframe::egui::TextureHandle>,
    pub media: bool,
    pub playback: Option<playback::Source>,
    pub pdf: Option<crate::pdf::PdfView>,
    pub office: Option<crate::office::OfficeView>,
    pub tools: EditorTools,
    pub syntax_cache: crate::syntax::SyntaxCache,
    pub unpinned_preview: bool,
    pub diff_area: Option<crate::view_tabs_browser_model::DiffTabArea>,
    pub diff_title: Option<String>,
    pub deleted_on_disk: bool,
    pub last_edited_at: Option<std::time::Instant>,
    pub force_large_markdown_render: bool,
    pub sheet_cache: Option<std::sync::Arc<crate::rich_previews::SheetTable>>,
}
impl Document {
    pub(crate) fn seed_find_from_selection(
        &mut self,
        selection: std::ops::Range<usize>,
        allow_word: bool,
    ) -> bool {
        let (mut start, mut end) =
            char_range_to_byte_range(&self.text, selection.start, selection.end);
        if start == end {
            if !allow_word {
                return false;
            }
            // Monaco's default word separators, plus line boundaries and horizontal whitespace.
            let delimiter = |ch: char| {
                matches!(ch, ' ' | '\t' | '\r' | '\n')
                    || r#"`~!@#$%^&*()-=+[{]}\|;:'",.<>/?"#.contains(ch)
            };
            start = self.text[..start].rfind(delimiter).map_or(0, |i| i + 1);
            end += self.text[end..]
                .find(delimiter)
                .unwrap_or(self.text.len() - end);
        }
        let seed = &self.text[start..end];
        if seed.is_empty() || seed.contains(['\r', '\n']) {
            return false;
        }
        if seed.encode_utf16().take(524_288).count() == 524_288 {
            return true;
        }
        self.tools.find_query = seed.to_owned();
        let char_start = self.text[..start].chars().count();
        if char_start == 0 {
            self.tools.match_index = 0;
            return true;
        }
        let mut preceding = 0;
        self.tools.match_index = 0;
        visit_find_matches(&self.text, seed, |hit| {
            if hit.0 >= char_start {
                self.tools.match_index = preceding;
                return false;
            }
            preceding += 1;
            true
        });
        true
    }

    pub(crate) fn display_text(&mut self) -> DisplayText {
        if let Some(cached) = &self.tools.display_text
            && cached.text.as_str() == self.text
        {
            return cached.clone();
        }
        let dirty = self.dirty();
        let hunk_lines = if dirty && let Some(original) = self.original_text() {
            changed_line_hunks(original, &self.text)
                .into_iter()
                .map(|(_, line)| line)
                .collect()
        } else {
            Default::default()
        };
        let display = DisplayText {
            text: std::sync::Arc::new(self.text.clone()),
            line_count: self.text.lines().count().max(1),
            dirty,
            hunk_lines,
        };
        self.tools.display_text = Some(display.clone());
        display
    }

    pub(crate) fn external_available(&self) -> bool {
        self.diff_title.is_none()
            && !self.deleted_on_disk
            && self.root.is_absolute()
            && crate::external_editor::under_project(&self.root, &self.path)
    }
    pub(crate) fn external_position(&self) -> Option<FilePosition> {
        if self.media || self.preview_mode || self.diff_title.is_some() {
            return None;
        }
        Some(
            self.tools
                .outline_cursor
                .unwrap_or(OutlineCursor::Bytes(0))
                .position(&self.text),
        )
    }

    pub(crate) fn reveal_location(&mut self, position: Option<FilePosition>) {
        let Some(position) = position else { return };
        if self.media {
            return;
        }
        let mut offset = 0;
        let mut lines = self.text.split('\n').enumerate().peekable();
        while let Some((index, line)) = lines.next() {
            if index + 1 >= position.line || lines.peek().is_none() {
                let mut units = position.column.saturating_sub(1);
                for c in line.trim_end_matches('\r').chars() {
                    if units < c.len_utf16() {
                        break;
                    }
                    units -= c.len_utf16();
                    offset += 1;
                }
                self.preview_mode = false;
                self.tools.pending_char_range = Some((offset, offset));
                self.tools.pending_scroll_line = Some(index);
                return;
            }
            offset += line.chars().count() + 1;
        }
    }

    pub(crate) fn next_instance_id() -> u64 {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        NEXT.try_update(
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
            |id| id.checked_add(1),
        )
        .expect("document instance identifiers exhausted")
    }

    pub(crate) fn instance_id(&self) -> u64 {
        self.instance_id
    }

    /// Apply a native editor event only to the exact document and base text it edited.
    /// A closed/reopened file or external reload must not receive queued stale edits.
    pub(crate) fn apply_native_edit(&mut self, instance_id: u64, base: &str, text: String) -> bool {
        if self.instance_id != instance_id
            || self.media
            || self.text != base
            || text.len() > MAX_LOCAL_EDIT_BYTES
        {
            return false;
        }
        if self.text != text {
            self.text = text;
            self.unpinned_preview = false;
            self.sheet_cache = None;
            self.preview = None;
            self.last_edited_at = Some(std::time::Instant::now());
        }
        true
    }

    pub fn from_diff(
        root: &Path,
        path: PathBuf,
        title: String,
        diff_text: String,
        unpinned_preview: bool,
    ) -> Self {
        Self {
            project: None,
            instance_id: Self::next_instance_id(),
            root: root.to_owned(),
            path,
            text: diff_text,
            original: vec![],
            utf8_bom: false,
            error: String::new(),
            preview_mode: false,
            preview: None,
            image: None,
            texture: None,
            gpui_image: None,
            playback: None,
            media: true,
            pdf: None,
            office: None,
            tools: EditorTools::default(),
            syntax_cache: crate::syntax::SyntaxCache::default(),
            unpinned_preview,
            diff_area: None,
            diff_title: Some(title),
            deleted_on_disk: false,
            last_edited_at: None,
            force_large_markdown_render: false,
            sheet_cache: None,
        }
    }

    pub fn from_readonly_text(
        root: PathBuf,
        path: PathBuf,
        title: String,
        text: String,
        markdown: bool,
    ) -> Self {
        let preview = markdown.then(|| crate::preview::Preview::parse(&text, true));
        Self {
            project: None,
            instance_id: Self::next_instance_id(),
            root,
            path,
            text,
            original: vec![],
            utf8_bom: false,
            error: String::new(),
            preview_mode: markdown,
            preview,
            image: None,
            texture: None,
            gpui_image: None,
            playback: None,
            media: true,
            pdf: None,
            office: None,
            tools: EditorTools::default(),
            syntax_cache: crate::syntax::SyntaxCache::default(),
            unpinned_preview: false,
            diff_area: None,
            diff_title: Some(title),
            deleted_on_disk: false,
            last_edited_at: None,
            force_large_markdown_render: false,
            sheet_cache: None,
        }
    }

    fn binary_preview(root: &Path, path: PathBuf) -> Self {
        Self {
            project: None,
            instance_id: Self::next_instance_id(),
            root: root.to_owned(),
            path,
            text: String::new(),
            original: vec![],
            utf8_bom: false,
            error: String::new(),
            preview_mode: true,
            preview: None,
            image: None,
            texture: None,
            gpui_image: None,
            playback: None,
            media: true,
            pdf: None,
            office: None,
            tools: EditorTools::default(),
            syntax_cache: crate::syntax::SyntaxCache::default(),
            unpinned_preview: false,
            diff_area: None,
            diff_title: None,
            deleted_on_disk: false,
            last_edited_at: None,
            force_large_markdown_render: false,
            sheet_cache: None,
        }
    }

    pub(crate) fn remote_playback(path: &str, mime: &'static str, bytes: &[u8]) -> Result<Self> {
        if bytes.len() > playback::MAX_BYTES {
            return Err("Media preview limit is 30 MiB".into());
        }
        Ok(Self {
            playback: Some(playback::Source::new(mime, bytes.to_vec())),
            ..Self::binary_preview(Path::new(""), PathBuf::from(path))
        })
    }

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
                office: Some(office),
                ..Self::binary_preview(root, path)
            });
        }
        if extension == "pdf" {
            let pdf = crate::pdf::PdfView::open(&path)?;
            return Ok(Self {
                pdf: Some(pdf),
                ..Self::binary_preview(root, path)
            });
        }
        if let Some(mime) = playback::mime(&extension) {
            let source = playback::Source::open(&path, mime)?;
            return Ok(Self {
                playback: Some(source),
                ..Self::binary_preview(root, path)
            });
        }
        if crate::raster_preview::supported(&extension) {
            let mut bytes = Vec::new();
            fs::File::open(&path)
                .map_err(|e| e.to_string())?
                .take(16 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() > 16 * 1024 * 1024 {
                return Err("Image limit is 16 MiB".into());
            }
            let image = crate::raster_preview::decode(&extension, &bytes)?;
            return Ok(Self {
                project: None,
                instance_id: Self::next_instance_id(),
                root: root.to_owned(),
                path,
                text: String::new(),
                original: bytes,
                utf8_bom: false,
                error: String::new(),
                preview_mode: true,
                preview: None,
                image: Some(image),
                texture: None,
                gpui_image: None,
                playback: None,
                media: true,
                pdf: None,
                office: None,
                tools: EditorTools::default(),
                syntax_cache: crate::syntax::SyntaxCache::default(),
                unpinned_preview: false,
                diff_area: None,
                diff_title: None,
                deleted_on_disk: false,
                last_edited_at: None,
                force_large_markdown_render: false,
                sheet_cache: None,
            });
        }
        let (text, utf8_bom) = read_editor_text(&path)?;
        let original = text.as_bytes().to_vec();
        let is_md = matches!(extension.as_str(), "md" | "markdown");
        let preview_fits = text.len() <= MAX_TEXT;
        let md_source_only = is_md
            && matches!(
                crate::rich_previews::classify_markdown_size(text.len()),
                crate::rich_previews::MarkdownSizeDecision::SourceOnly { .. }
            );
        Ok(Self {
            project: None,
            instance_id: Self::next_instance_id(),
            root: root.to_owned(),
            path,
            text,
            original,
            utf8_bom,
            error: String::new(),
            preview_mode: preview_fits
                && !md_source_only
                && matches!(
                    extension.as_str(),
                    "html" | "htm" | "md" | "markdown" | "csv" | "tsv"
                ),
            preview: None,
            image: None,
            texture: None,
            gpui_image: None,
            playback: None,
            media: false,
            pdf: None,
            office: None,
            tools: EditorTools {
                #[cfg(windows)]
                html_root: if matches!(extension.as_str(), "html" | "htm") {
                    Some(root.canonicalize().map_err(|e| e.to_string())?)
                } else {
                    None
                },
                ..Default::default()
            },
            syntax_cache: crate::syntax::SyntaxCache::default(),
            unpinned_preview: false,
            diff_area: None,
            diff_title: None,
            deleted_on_disk: false,
            last_edited_at: None,
            force_large_markdown_render: false,
            sheet_cache: None,
        })
    }
    pub fn save_image_copy(
        &mut self,
        french: bool,
        choose: impl FnOnce(&std::ffi::OsStr) -> Option<PathBuf>,
    ) {
        self.tools.image_menu = None;
        if !self.media || self.original.is_empty() {
            return;
        }
        let Some(destination) = self.path.file_name().and_then(choose) else {
            return;
        };
        self.tools.image_download_notice = match save_download(&destination, &self.original) {
            Ok(()) if french => format!("Fichier enregistré dans {}", destination.display()),
            Ok(()) => format!("File saved to {}", destination.display()),
            Err(e) => e.to_string(),
        };
    }

    pub fn choose_image_copy(&mut self, french: bool) {
        self.save_image_copy(french, crate::download_name::choose_download_destination);
    }

    pub fn original_text(&self) -> Option<&str> {
        std::str::from_utf8(&self.original).ok()
    }
    pub fn dirty(&self) -> bool {
        !self.media && self.text.as_bytes() != self.original
    }
    fn read_current_disk(&self) -> Result<(String, bool)> {
        let actual = confined(&self.root, &self.path)?;
        if actual != self.path {
            return Err("File target changed since opening.".into());
        }
        read_editor_text(&actual)
    }

    pub(crate) fn observe_disk(&mut self, text: Option<String>) -> bool {
        let Some(text) = text else {
            let changed = !self.deleted_on_disk || self.tools.conflict_disk.is_some();
            self.deleted_on_disk = true;
            self.tools.conflict_disk = None;
            self.tools.comparison = None;
            self.last_edited_at = None;
            if changed {
                self.error.clear();
            }
            return changed;
        };
        let (text, bom) = split_bom(text);
        self.utf8_bom = bom;
        let restored = std::mem::take(&mut self.deleted_on_disk);
        if text.as_bytes() == self.original {
            if restored || self.tools.conflict_disk.is_some() {
                self.tools.conflict_disk = None;
                self.tools.comparison = None;
                self.error.clear();
                self.last_edited_at = self.dirty().then(std::time::Instant::now);
                return true;
            }
            return false;
        }
        if self.dirty() && self.text != text {
            if self.tools.conflict_disk.as_deref() == Some(&text) {
                return restored;
            }
            self.tools.conflict_disk = Some(text.into());
            self.tools.comparison = None;
            self.last_edited_at = None;
            self.error = DISK_CHANGED.into();
        } else {
            self.replace_from_disk(text);
        }
        true
    }

    fn replace_from_disk(&mut self, text: String) {
        self.tools.disk_reload = self.tools.disk_reload.wrapping_add(1);
        self.tools.display_text = None;
        self.original = text.as_bytes().to_vec();
        self.text = text;
        self.preview = None;
        self.sheet_cache = None;
        if self.tools.html_plain_text.is_some() {
            self.tools.html_plain_text = Some(crate::preview::html_plain_text(&self.text).into());
        }
        self.tools.conflict_disk = None;
        self.tools.comparison = None;
        self.last_edited_at = None;
        self.error.clear();
    }

    fn remember_conflict(&mut self, text: String) -> Result<()> {
        self.tools.comparison = None;
        self.tools.conflict_disk = Some(text.into());
        Err(DISK_CHANGED.into())
    }

    pub(crate) fn comparison(&self) -> Option<&std::sync::Arc<DiskComparison>> {
        self.tools
            .comparison
            .as_ref()
            .filter(|c| c.draft == self.text)
    }

    pub(crate) fn resolve_conflict(
        &mut self,
        expected: &std::sync::Arc<str>,
        action: ConflictAction,
    ) -> Result<()> {
        if !self
            .tools
            .conflict_disk
            .as_ref()
            .is_some_and(|disk| std::sync::Arc::ptr_eq(disk, expected))
        {
            return Ok(());
        }
        let (disk, bom) = self.read_current_disk()?;
        self.utf8_bom = bom;
        if disk != expected.as_ref() {
            return self.remember_conflict(disk);
        }
        if matches!(action, ConflictAction::Compare) {
            if self.comparison().is_none() {
                let comparison = std::sync::Arc::new(DiskComparison {
                    draft: self.text.clone(),
                    patch: Default::default(),
                });
                let job = comparison.clone();
                let disk = expected.clone();
                let name = self
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                std::thread::Builder::new()
                    .name("editor-conflict-diff".into())
                    .spawn(move || {
                        let diff = crate::saved_version_diff::text_diff(&name, &disk, &job.draft);
                        let _ = job.patch.set(crate::saved_version_diff::Patch::new(diff));
                    })
                    .map_err(|e| e.to_string())?;
                self.tools.comparison = Some(comparison);
            }
            return Ok(());
        }
        self.tools.disk_revision = self.tools.disk_revision.wrapping_add(1);
        if matches!(action, ConflictAction::Reload) {
            self.replace_from_disk(expected.to_string());
        } else {
            self.tools.display_text = None;
            self.original = disk.into_bytes();
            self.last_edited_at = self.dirty().then(std::time::Instant::now);
            self.tools.conflict_disk = None;
            self.tools.comparison = None;
            self.error.clear();
        }
        Ok(())
    }

    pub(crate) fn auto_save(&mut self, profile: Option<&Path>) -> Result<()> {
        self.write_draft(profile, false)
    }

    pub fn save(&mut self, profile: Option<&Path>) -> Result<()> {
        self.write_draft(profile, true)
    }

    fn encoded_draft(&self) -> Result<String> {
        if self.utf8_bom {
            if self.text.len() > MAX_LOCAL_EDIT_BYTES - UTF8_BOM.len_utf8() {
                return Err("Text editor limit is 50 MiB.".into());
            }
            Ok(format!("{UTF8_BOM}{}", self.text))
        } else {
            Ok(self.text.clone())
        }
    }

    fn persist_existing_draft(
        &mut self,
        mut staged: tempfile::NamedTempFile,
        encoded: &mut String,
    ) -> Result<()> {
        let mut restaged = false;
        loop {
            let (disk, bom) = match self.read_current_disk() {
                Err(_)
                    if fs::symlink_metadata(&self.path)
                        .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
                {
                    self.observe_disk(None);
                    return Err("Deleted on disk: saving creates it again.".into());
                }
                result => result?,
            };
            let encoding_changed = self.utf8_bom != bom;
            self.utf8_bom = bom;
            if disk.as_bytes() != self.original {
                return self.remember_conflict(disk);
            }
            if !encoding_changed {
                staged.persist(&self.path).map_err(|err| err.to_string())?;
                return Ok(());
            }
            if restaged {
                return Err(
                    "File encoding changed during save. Save again to use the current encoding."
                        .into(),
                );
            }
            *encoded = self.encoded_draft()?;
            staged = prepare_atomic_write(&self.path, encoded.as_bytes())?;
            restaged = true;
        }
    }

    fn write_draft(&mut self, profile: Option<&Path>, recreate: bool) -> Result<()> {
        if self.media {
            return Err("This document is read-only".into());
        }
        if self.text.len() > MAX_LOCAL_EDIT_BYTES {
            return Err("Text editor limit is 50 MiB.".into());
        }
        self.tools.disk_revision = self.tools.disk_revision.wrapping_add(1);
        let missing = match fs::symlink_metadata(&self.path) {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                self.deleted_on_disk = true;
                self.tools.conflict_disk = None;
                self.tools.comparison = None;
                if !recreate {
                    return Err("Deleted on disk: saving creates it again.".into());
                }
                let parent = self.path.parent().ok_or("File has no parent directory.")?;
                if confined(&self.root, parent)? != parent {
                    return Err("File target changed since opening.".into());
                }
                true
            }
            Err(err) => return Err(err.to_string()),
            Ok(_) => {
                let (disk, bom) = self.read_current_disk()?;
                self.utf8_bom = bom;
                self.deleted_on_disk = false;
                if disk.as_bytes() != self.original {
                    return self.remember_conflict(disk);
                }
                false
            }
        };
        let mut encoded = self.encoded_draft()?;
        if missing {
            save_download(&self.path, encoded.as_bytes()).map_err(|err| err.to_string())?;
        } else {
            let staged = prepare_atomic_write(&self.path, encoded.as_bytes())?;
            self.persist_existing_draft(staged, &mut encoded)?;
        }
        self.deleted_on_disk = false;
        if let Some(profile) = profile {
            let _ = crate::local_file_history::record(profile, &self.path, &encoded);
        }
        self.tools.conflict_disk = None;
        self.tools.comparison = None;
        let mut bytes = encoded.into_bytes();
        if self.utf8_bom {
            bytes.drain(..UTF8_BOM.len_utf8());
        }
        self.tools.display_text = None;
        self.original = bytes;
        self.last_edited_at = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn compatibility_image_preview_centers_the_click_target_at_the_fitted_dimensions() {
        use eframe::egui;
        let ctx = egui::Context::default();
        for (natural, expected) in [
            ([200, 600], [40., 120.]),
            ([24, 12], [24., 12.]),
            ([800, 80], [200., 20.]),
        ] {
            let tex = ctx.load_texture(
                "preview",
                egui::ColorImage::filled(natural, egui::Color32::WHITE),
                Default::default(),
            );
            let _ = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let area =
                        egui::Rect::from_min_size(egui::pos2(30., 40.), egui::vec2(240., 160.));
                    ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
                        let response = show_fitted_image(ui, &tex);
                        assert_eq!(response.rect.center(), area.center());
                        assert_eq!(response.rect.size(), egui::vec2(expected[0], expected[1]));
                    });
                });
            });
        }
    }

    #[test]
    fn local_image_copy_preserves_opened_bytes_and_existing_destinations() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("poster.avif");
        let dest = dir.path().join("copy.avif");
        let bytes = include_bytes!("../tests/fixtures/alpha-bars.avif");
        std::fs::write(&source, bytes).unwrap();
        let mut doc = super::Document::open(dir.path(), &source).unwrap();
        std::fs::remove_file(&source).unwrap();
        doc.save_image_copy(false, |_| None);
        assert!(doc.tools.image_download_notice.is_empty());
        doc.save_image_copy(true, |name| {
            assert_eq!(name, "poster.avif");
            Some(dest.clone())
        });
        assert!(doc.tools.image_download_notice.contains("enregistré"));
        assert_eq!(std::fs::read(&dest).unwrap(), bytes);
        std::fs::write(&dest, b"keep").unwrap();
        doc.save_image_copy(false, |_| Some(dest.clone()));
        assert_eq!(std::fs::read(&dest).unwrap(), b"keep");
        assert!(!doc.dirty());
    }

    #[test]
    fn local_editor_checks_binary_prefix_and_validates_utf8_in_the_entire_body() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prefix.txt");
        let mut bytes = vec![b'x'; 8193];
        bytes[8191] = 0;
        std::fs::write(&path, &bytes).unwrap();
        assert!(
            super::read_editor_text(&path)
                .unwrap_err()
                .contains("Binary")
        );
        bytes[8191] = b'x';
        bytes[8192] = 0;
        std::fs::write(&path, &bytes).unwrap();
        let mut doc = super::Document::open(dir.path(), &path).unwrap();
        doc.text.push('!');
        doc.save(None).unwrap();
        bytes.push(b'!');
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        bytes[8192] = 0xff;
        std::fs::write(&path, &bytes).unwrap();
        assert!(
            super::read_editor_text(&path)
                .unwrap_err()
                .contains("UTF-8")
        );
    }

    #[test]
    fn local_editor_roundtrips_at_limit_and_rejects_one_more_byte() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("limit.txt");
        std::fs::write(&path, vec![b'x'; super::MAX_LOCAL_EDIT_BYTES]).unwrap();
        let mut doc = super::Document::open(dir.path(), &path).unwrap();
        doc.text.replace_range(..1, "y");
        assert!(doc.dirty());
        doc.save(None).unwrap();
        drop(doc);
        let doc = super::Document::open(dir.path(), &path).unwrap();
        assert_eq!(doc.text.len(), super::MAX_LOCAL_EDIT_BYTES);
        assert!(doc.text.starts_with('y') && doc.text.ends_with('x'));
        drop(doc);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(super::MAX_LOCAL_EDIT_BYTES as u64 + 1)
            .unwrap();
        assert_eq!(
            super::read(&path).unwrap_err(),
            "Text editor limit is 50 MiB."
        );
    }

    #[test]
    fn native_editor_edits_reject_stale_reopened_readonly_and_oversized_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.txt");
        std::fs::write(&path, "aé😀z\r\n").unwrap();
        let mut doc = super::Document::open(dir.path(), &path).unwrap();
        let id = doc.instance_id();
        doc.unpinned_preview = true;
        assert!(doc.apply_native_edit(id, "aé😀z\r\n", "aÉ😀z\r\n".into()));
        assert!(doc.dirty());
        assert!(!doc.unpinned_preview);
        assert!(doc.last_edited_at.is_some());
        assert!(!doc.apply_native_edit(id, "aé😀z\r\n", "stale".into()));
        assert!(!doc.apply_native_edit(
            id,
            "aÉ😀z\r\n",
            "x".repeat(super::MAX_LOCAL_EDIT_BYTES + 1)
        ));
        let mut reopened = super::Document::open(dir.path(), &path).unwrap();
        assert_ne!(reopened.instance_id(), id);
        assert!(!reopened.apply_native_edit(id, "aé😀z\r\n", "queued".into()));
        let mut readonly = super::Document::from_readonly_text(
            dir.path().into(),
            path,
            "readonly".into(),
            "fixed".into(),
            false,
        );
        assert!(!readonly.apply_native_edit(readonly.instance_id(), "fixed", "edited".into()));
        assert_eq!(readonly.text, "fixed");
        assert_eq!(doc.text, "aÉ😀z\r\n");
    }
    use super::*;
    #[test]
    fn editor_preserves_external_changes_and_line_endings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("text.txt");
        fs::write(&path, "a\r\nb\r\n").unwrap();
        let mut doc = Document::open(dir.path(), &path).unwrap();
        doc.text.push('c');
        doc.save(None).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "a\r\nb\r\nc");
        fs::write(&path, "someone else").unwrap();
        doc.text.push('d');
        assert!(doc.save(None).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "someone else");
    }
    #[test]
    fn staged_draft_keeps_a_later_external_edit_or_deletion() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        fs::write(&path, "base").unwrap();
        let mut doc = Document::open(dir.path(), &path).unwrap();
        doc.text = "draft".into();
        let mut encoded = doc.encoded_draft().unwrap();
        let staged = prepare_atomic_write(&doc.path, encoded.as_bytes()).unwrap();
        fs::write(&path, "other writer").unwrap();
        assert!(doc.persist_existing_draft(staged, &mut encoded).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "other writer");
        assert_eq!(doc.tools.conflict_disk.as_deref(), Some("other writer"));
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        let staged = prepare_atomic_write(&doc.path, encoded.as_bytes()).unwrap();
        fs::remove_file(&path).unwrap();
        assert!(doc.persist_existing_draft(staged, &mut encoded).is_err());
        assert!(doc.deleted_on_disk && doc.dirty());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn staged_draft_retries_with_a_late_utf8_bom_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        fs::write(&path, "base").unwrap();
        let mut doc = Document::open(dir.path(), &path).unwrap();
        doc.text = "draft".into();
        let staged = prepare_atomic_write(&doc.path, doc.text.as_bytes()).unwrap();
        fs::write(&path, "\u{feff}base").unwrap();
        let mut encoded = doc.encoded_draft().unwrap();
        doc.persist_existing_draft(staged, &mut encoded).unwrap();
        assert!(doc.tools.conflict_disk.is_none());
        assert_eq!(encoded, "\u{feff}draft");
        assert_eq!(fs::read_to_string(&path).unwrap(), "\u{feff}draft");
    }

    #[test]
    fn utf8_bom_stays_out_of_the_editor_and_round_trips_on_save_and_recreation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        fs::write(&path, "\u{feff}\u{feff}one\r\n").unwrap();
        let mut doc = Document::open(dir.path(), &path).unwrap();
        assert_eq!(doc.text, "\u{feff}one\r\n");
        assert!(!doc.dirty());
        doc.text = "deux\r\n".into();
        let profile = tempfile::tempdir().unwrap();
        doc.save(Some(profile.path())).unwrap();
        let saved = crate::local_file_history::list(profile.path(), &doc.path).unwrap();
        assert!(
            crate::local_file_history::diff(profile.path(), dir.path(), &doc.path, &saved[0].id)
                .unwrap()
                .is_empty()
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "\u{feff}deux\r\n");
        fs::remove_file(&path).unwrap();
        doc.save(None).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "\u{feff}deux\r\n");
    }

    #[test]
    fn utf8_bom_changes_do_not_conflict_with_a_draft_or_enter_a_comparison() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        fs::write(&path, "base").unwrap();
        let mut doc = Document::open(dir.path(), &path).unwrap();
        doc.text = "draft".into();
        fs::write(&path, "\u{feff}base").unwrap();
        doc.save(None).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "\u{feff}draft");
        doc.text = "local".into();
        fs::write(&path, "\u{feff}other").unwrap();
        assert!(doc.observe_disk(Some(fs::read_to_string(&path).unwrap())));
        let disk = doc.tools.conflict_disk.clone().unwrap();
        assert_eq!(&*disk, "other");
        doc.resolve_conflict(&disk, ConflictAction::Keep).unwrap();
        fs::write(&path, "other").unwrap();
        doc.save(None).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"local");
    }

    #[test]
    fn deleted_draft_survives_auto_save_and_manual_save_recreates_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        fs::write(&path, "base").unwrap();
        let mut doc = Document::open(dir.path(), &path).unwrap();
        doc.text = "modified\r\n".into();
        fs::remove_file(&path).unwrap();
        assert!(doc.auto_save(None).is_err());
        assert!(doc.deleted_on_disk && doc.dirty());
        assert!(!path.exists());
        doc.save(None).unwrap();
        assert!(!doc.deleted_on_disk && !doc.dirty());
        assert_eq!(fs::read(&path).unwrap(), b"modified\r\n");
    }

    #[test]
    fn deleted_draft_respects_a_replacement_and_a_missing_parent() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("folder");
        fs::create_dir(&folder).unwrap();
        let path = folder.join("notes.txt");
        fs::write(&path, "base").unwrap();
        let mut doc = Document::open(dir.path(), &path).unwrap();
        doc.text = "draft".into();
        fs::remove_file(&path).unwrap();
        fs::remove_dir(&folder).unwrap();
        assert!(doc.save(None).is_err());
        assert!(!folder.exists());
        fs::create_dir(&folder).unwrap();
        fs::write(&path, "replacement").unwrap();
        assert!(doc.save(None).is_err());
        assert_eq!(doc.tools.conflict_disk.as_deref(), Some("replacement"));
        assert_eq!(fs::read(&path).unwrap(), b"replacement");
    }

    #[test]
    fn disk_conflict_requires_a_new_choice_after_another_external_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("document.txt");
        fs::write(&path, "base").unwrap();
        let mut doc = Document::open(dir.path(), &path).unwrap();
        doc.text = "draft".into();
        fs::write(&path, "writer one").unwrap();
        assert!(doc.save(None).is_err());
        let first = doc.tools.conflict_disk.clone().unwrap();
        fs::write(&path, "writer two").unwrap();
        assert!(doc.resolve_conflict(&first, ConflictAction::Keep).is_err());
        doc.resolve_conflict(&first, ConflictAction::Reload)
            .unwrap();
        assert_eq!(doc.text, "draft");
        let second = doc.tools.conflict_disk.clone().unwrap();
        doc.resolve_conflict(&second, ConflictAction::Keep).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "writer two");
        assert!(doc.dirty());
        doc.save(None).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "draft");
        assert!(!doc.dirty());
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
        assert!(doc.save(None).is_err());
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
    fn editor_find_skips_overlaps_and_preserves_lines_across_multiline_matches() {
        assert_eq!(find_char_matches("aaaaa", "AA"), [(0, 2, 0), (2, 4, 0)]);
        assert_eq!(find_char_matches("x\nx\nx", "X\n"), [(0, 2, 0), (2, 4, 1)]);
        assert_eq!(find_char_matches("😀İi\nI", "i"), [(2, 3, 0), (4, 5, 1)]);
        assert_eq!(find_char_matches("😀İi\nI", "i\u{307}"), [(1, 2, 0)]);
    }

    #[test]
    fn goto_resolves_visible_columns_signed_lines_and_utf16_offsets() {
        let text = "a\r\n\t😀e\u{301}z\rlast";
        for (spec, expected) in [
            ("-2", (3, 1)),
            ("2:3", (3, 1)),
            ("2,4", (4, 1)),
            ("2#6", (4, 1)),
            ("2:7", (5, 1)),
            ("2:8", (7, 1)),
            ("2:-1", (7, 1)),
            ("88", (9, 2)),
            ("-99", (0, 0)),
            (":3", (1, 0)),
            ("::4", (3, 1)),
            (":6", (4, 1)),
            (":7", (5, 1)),
            (":-1", (13, 2)),
            (":999", (13, 2)),
        ] {
            assert_eq!(goto_line_char_offset(text, spec), Some(expected), "{spec}");
        }
        assert_eq!(goto_line_char_offset("a\r\n", "-1"), Some((3, 1)));
        assert_eq!(goto_line_char_offset("", "::1"), Some((0, 0)));
        assert_eq!(goto_line_char_offset(text, "x:3"), None);
        assert_eq!(goto_line_char_offset(text, "::"), None);
    }

    #[test]
    fn ctrl_f_seeds_the_selected_occurrence_or_word_and_retains_previous_query() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("seed.txt");
        std::fs::write(&path, "café ++ café,😀_id\r\nnext").unwrap();
        let mut doc = super::Document::open(dir.path(), &path).unwrap();
        for (selection, expected, index) in [
            (9..9, "café", 1),
            (12..12, "café", 1),
            (13..13, "😀_id", 0),
            (5..7, "++", 0),
            (7..7, "++", 0),
            (12..21, "++", 0),
            (4..8, " ++ ", 0),
        ] {
            doc.seed_find_from_selection(selection, true);
            assert_eq!(doc.tools.find_query, expected);
            assert_eq!(doc.tools.match_index, index);
        }
        doc.text = "😀".repeat(262144);
        doc.seed_find_from_selection(0..262144, true);
        assert_eq!(doc.tools.find_query, " ++ ");
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
