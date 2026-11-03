//! Native `egui` Source Control (`Changes`) panel (`ChangesPanel.vue`, `sourceControl.js`,
//! `sourceControlTree.js`, `main/sourceControl.js`, and `sourceControl.css` parity).
//!
//! Pure UI + deterministic `&[u8]` porcelain v2 `-z` parser with zero filesystem, process,
//! network, or persistence side effects:
//! - Composes directly with [`crate::explorer_scm_model`] ([`ScmLang`], [`ScmSectionKind`],
//!   [`scm_text`], [`scm_status_title`], [`DiscardConfirmationCopy`], [`discard_entry_copy`],
//!   [`change_count`]) and [`crate::icons`] ([`Icon`]).
//! - Parses `git status --porcelain=v2 --branch -z` directly from `&[u8]` without lossy UTF-8
//!   decoding: explicitly reports byte-limit truncation, incomplete/malformed records, and
//!   non-UTF-8 paths without fabricating paths, and consumes both NUL-terminated rename/copy
//!   paths even after hitting `max_entries`.
//! - Enforces [`ScmCapabilities`] strictly (all `false` by default): unwired subsystems
//!   (`Create PR`, AI `Generate`, `Review Notes`, `ScmHistoryPanel`, unwired `Discard`/`Remote`)
//!   are omitted rather than rendered as inert buttons, `RequestDiscard` never mutates state,
//!   and `truncated` / `parse_incomplete` strictly block bulk actions.

use std::collections::{HashMap, HashSet, VecDeque};

use eframe::egui::{
    self, Color32, FontId, Id, Key, Modifiers, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2,
};

use crate::{
    explorer_scm_model::{
        DiscardConfirmationCopy, ScmLang, ScmSectionKind, change_count, discard_entry_copy,
        scm_status_title, scm_text,
    },
    icons::{self, Icon},
    theme,
};

/// Default entry cap matching `STATUS_ENTRIES_MAX` in `src/main/sourceControl.js`.
pub const STATUS_ENTRIES_MAX: usize = 5_000;

/// Default raw byte cap (`8 MiB`) for `git status --porcelain=v2 -z` payloads.
pub const STATUS_RAW_BYTES_MAX: usize = 8 * 1024 * 1024;

/// Maximum commit message length (`MAX_MESSAGE = 20000` in `src/main/sourceControl.js`).
pub const MAX_COMMIT_MESSAGE_CHARS: usize = 20_000;

/// Maximum file-name filter query length (`chars`).
pub const MAX_FILTER_CHARS: usize = 256;

/// Exact Git status decoration colors from `src/renderer/src/sourceControl.css:6-12`.
pub const GIT_COLOR_MODIFIED: Color32 = Color32::from_rgb(0xe2, 0xc0, 0x8d);
pub const GIT_COLOR_ADDED: Color32 = Color32::from_rgb(0x81, 0xb8, 0x8b);
pub const GIT_COLOR_DELETED: Color32 = Color32::from_rgb(0xc7, 0x4e, 0x39);
pub const GIT_COLOR_RENAMED: Color32 = Color32::from_rgb(0x73, 0xc9, 0x91);

const HEADER_BTN_SIDE: f32 = 28.0;
const SECTION_ROW_H: f32 = 26.0;
const ITEM_ROW_H: f32 = 24.0;
const CONFLICT_ROW_H: f32 = 36.0;
const ROW_ACTION_SIDE: f32 = 20.0;
const TREE_INDENT_STEP: f32 = 12.0;
const TREE_BASE_PAD: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScmArea {
    Staged,
    Unstaged,
    Untracked,
}

impl ScmArea {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Staged => "staged",
            Self::Unstaged => "unstaged",
            Self::Untracked => "untracked",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScmFileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    Untracked,
}

impl ScmFileStatus {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Modified => "M",
            Self::Added => "A",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::Copied => "C",
            Self::Untracked => "U",
        }
    }

    #[must_use]
    pub const fn as_key(self) -> &'static str {
        match self {
            Self::Modified => "modified",
            Self::Added => "added",
            Self::Deleted => "deleted",
            Self::Renamed => "renamed",
            Self::Copied => "copied",
            Self::Untracked => "untracked",
        }
    }

    #[must_use]
    pub const fn color(self) -> Color32 {
        match self {
            Self::Modified => GIT_COLOR_MODIFIED,
            Self::Added => GIT_COLOR_ADDED,
            Self::Deleted => GIT_COLOR_DELETED,
            Self::Renamed | Self::Copied | Self::Untracked => GIT_COLOR_RENAMED,
        }
    }

    #[must_use]
    pub fn title(self, lang: ScmLang) -> &'static str {
        scm_status_title(self.as_key(), lang)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScmConflictKind {
    BothModified,
    BothAdded,
    DeletedByUs,
    DeletedByThem,
    AddedByUs,
    AddedByThem,
    BothDeleted,
}

impl ScmConflictKind {
    #[must_use]
    pub const fn label(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::BothModified, false) => "both modified",
            (Self::BothModified, true) => "modifié des deux côtés",
            (Self::BothAdded, false) => "both added",
            (Self::BothAdded, true) => "ajouté des deux côtés",
            (Self::DeletedByUs, false) => "deleted by us",
            (Self::DeletedByUs, true) => "supprimé chez nous",
            (Self::DeletedByThem, false) => "deleted by them",
            (Self::DeletedByThem, true) => "supprimé chez eux",
            (Self::AddedByUs, false) => "added by us",
            (Self::AddedByUs, true) => "ajouté chez nous",
            (Self::AddedByThem, false) => "added by them",
            (Self::AddedByThem, true) => "ajouté chez eux",
            (Self::BothDeleted, false) => "both deleted",
            (Self::BothDeleted, true) => "supprimé des deux côtés",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScmEntry {
    pub path: String,
    pub old_path: Option<String>,
    pub area: ScmArea,
    pub status: ScmFileStatus,
    pub conflict: Option<ScmConflictKind>,
    pub added: Option<u32>,
    pub removed: Option<u32>,
}

impl ScmEntry {
    #[must_use]
    pub const fn is_conflict(&self) -> bool {
        self.conflict.is_some()
    }

    #[must_use]
    pub const fn can_stage(&self) -> bool {
        matches!(self.area, ScmArea::Unstaged | ScmArea::Untracked) && self.conflict.is_none()
    }

    #[must_use]
    pub const fn can_unstage(&self) -> bool {
        matches!(self.area, ScmArea::Staged)
    }

    #[must_use]
    pub const fn can_discard(&self) -> bool {
        matches!(self.area, ScmArea::Unstaged | ScmArea::Untracked) && self.conflict.is_none()
    }

    #[must_use]
    pub const fn discard_deletes_file(&self) -> bool {
        matches!(self.area, ScmArea::Untracked)
            || matches!(self.status, ScmFileStatus::Untracked | ScmFileStatus::Added)
    }

    #[must_use]
    pub fn discard_copy(&self, remote: bool, french: bool) -> DiscardConfirmationCopy {
        let lang = if french { ScmLang::Fr } else { ScmLang::En };
        discard_entry_copy(
            &self.path,
            self.status.as_key(),
            self.discard_deletes_file(),
            remote,
            lang,
        )
    }
}

/// Bulk area discard confirmation copy (`getDiscardAreaConfirmationCopy` in `sourceControl.js`).
#[must_use]
pub fn discard_area_copy(
    area: ScmArea,
    count: usize,
    remote: bool,
    french: bool,
) -> DiscardConfirmationCopy {
    let n = count.max(1);
    if area == ScmArea::Untracked {
        return match (remote, french, n == 1) {
            (false, false, true) => DiscardConfirmationCopy {
                title: "Delete 1 untracked file?".to_string(),
                description: "This will move this untracked file to the Recycle Bin.".to_string(),
                confirm_label: "Delete".to_string(),
            },
            (false, false, false) => DiscardConfirmationCopy {
                title: format!("Delete {n} untracked files?"),
                description: format!(
                    "This will move these {n} untracked files to the Recycle Bin."
                ),
                confirm_label: format!("Delete {n}"),
            },
            (true, false, true) => DiscardConfirmationCopy {
                title: "Delete 1 untracked file?".to_string(),
                description: "This will move this untracked file to the trash on the remote host."
                    .to_string(),
                confirm_label: "Delete".to_string(),
            },
            (true, false, false) => DiscardConfirmationCopy {
                title: format!("Delete {n} untracked files?"),
                description: format!(
                    "This will move these {n} untracked files to the trash on the remote host."
                ),
                confirm_label: format!("Delete {n}"),
            },
            (false, true, true) => DiscardConfirmationCopy {
                title: "Supprimer 1 fichier non suivi ?".to_string(),
                description: "Ce fichier non suivi sera déplacé dans la Corbeille.".to_string(),
                confirm_label: "Supprimer".to_string(),
            },
            (false, true, false) => DiscardConfirmationCopy {
                title: format!("Supprimer {n} fichiers non suivis ?"),
                description: format!(
                    "Ces {n} fichiers non suivis seront déplacés dans la Corbeille."
                ),
                confirm_label: format!("Supprimer {n}"),
            },
            (true, true, true) => DiscardConfirmationCopy {
                title: "Supprimer 1 fichier non suivi ?".to_string(),
                description:
                    "Ce fichier non suivi sera déplacé dans la corbeille de l'hôte distant."
                        .to_string(),
                confirm_label: "Supprimer".to_string(),
            },
            (true, true, false) => DiscardConfirmationCopy {
                title: format!("Supprimer {n} fichiers non suivis ?"),
                description: format!(
                    "Ces {n} fichiers non suivis seront déplacés dans la corbeille de l'hôte distant."
                ),
                confirm_label: format!("Supprimer {n}"),
            },
        };
    }

    match (french, n == 1) {
        (false, true) => DiscardConfirmationCopy {
            title: "Discard all unstaged changes?".to_string(),
            description:
                "This will revert the unstaged changes in 1 file. This cannot be undone."
                    .to_string(),
            confirm_label: "Discard all".to_string(),
        },
        (false, false) => DiscardConfirmationCopy {
            title: "Discard all unstaged changes?".to_string(),
            description: format!(
                "This will revert unstaged changes in {n} files. This cannot be undone."
            ),
            confirm_label: "Discard all".to_string(),
        },
        (true, true) => DiscardConfirmationCopy {
            title: "Abandonner toutes les modifications non indexées ?".to_string(),
            description:
                "Les modifications non indexées de 1 fichier seront annulées. Cette action est irréversible."
                    .to_string(),
            confirm_label: "Tout abandonner".to_string(),
        },
        (true, false) => DiscardConfirmationCopy {
            title: "Abandonner toutes les modifications non indexées ?".to_string(),
            description: format!(
                "Les modifications non indexées de {n} fichiers seront annulées. Cette action est irréversible."
            ),
            confirm_label: "Tout abandonner".to_string(),
        },
    }
}

/// Explicit diagnostic emitted when `git status --porcelain=v2 -z` contains truncated,
/// malformed, or non-UTF-8 records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScmParseIssue {
    InputBytesExceeded {
        max_bytes: usize,
        actual_bytes: usize,
    },
    IncompleteRecord {
        offset: usize,
    },
    MalformedRecord {
        offset: usize,
    },
    NonUtf8Path {
        offset: usize,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedGitStatus {
    pub branch: Option<String>,
    pub oid: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub has_ab: bool,
    pub entries: Vec<ScmEntry>,
    pub total_entries: usize,
    pub truncated: bool,
    pub input_bytes_truncated: bool,
    pub malformed_records: usize,
    pub non_utf8_paths: usize,
    pub first_issue: Option<ScmParseIssue>,
}

impl ParsedGitStatus {
    #[must_use]
    pub const fn has_parse_issues(&self) -> bool {
        self.input_bytes_truncated || self.malformed_records > 0 || self.non_utf8_paths > 0
    }

    #[must_use]
    pub const fn blocks_bulk_actions(&self) -> bool {
        self.truncated || self.has_parse_issues()
    }

    /// Marks the parsed payload as externally truncated (for example when a bounded
    /// subprocess capture `Capture { bytes, truncated: true }` cut stdout at its pipe budget).
    pub fn mark_input_truncated(&mut self) {
        self.input_bytes_truncated = true;
        self.truncated = true;
    }

    #[must_use]
    pub const fn badge_count(&self, is_repo: bool) -> usize {
        change_count(
            is_repo,
            self.entries.len(),
            self.truncated,
            Some(self.total_entries),
        )
    }
}

fn record_issue(out: &mut ParsedGitStatus, issue: ScmParseIssue) {
    if out.first_issue.is_none() {
        out.first_issue = Some(issue);
    }
}

fn parse_status_letter(b: u8) -> Option<ScmFileStatus> {
    match b {
        b'M' | b'T' => Some(ScmFileStatus::Modified),
        b'A' => Some(ScmFileStatus::Added),
        b'D' => Some(ScmFileStatus::Deleted),
        b'R' => Some(ScmFileStatus::Renamed),
        b'C' => Some(ScmFileStatus::Copied),
        _ => None,
    }
}

fn parse_conflict_xy(xy: &[u8]) -> Option<ScmConflictKind> {
    match xy {
        b"DD" => Some(ScmConflictKind::BothDeleted),
        b"AU" => Some(ScmConflictKind::AddedByUs),
        b"UD" => Some(ScmConflictKind::DeletedByThem),
        b"UA" => Some(ScmConflictKind::AddedByThem),
        b"DU" => Some(ScmConflictKind::DeletedByUs),
        b"AA" => Some(ScmConflictKind::BothAdded),
        b"UU" => Some(ScmConflictKind::BothModified),
        _ => None,
    }
}

/// Splits `n_fields` space-separated header tokens from `rec` and returns `(fields, path_bytes)`.
fn split_header_and_path(rec: &[u8], n_fields: usize) -> Option<(Vec<&[u8]>, &[u8])> {
    let mut fields = Vec::with_capacity(n_fields);
    let mut cursor = 0_usize;
    for _ in 0..n_fields {
        if cursor >= rec.len() {
            return None;
        }
        let rel_space = rec[cursor..].iter().position(|&b| b == b' ')?;
        let field = &rec[cursor..cursor + rel_space];
        if field.is_empty() {
            return None;
        }
        fields.push(field);
        cursor += rel_space + 1;
    }
    let path_bytes = &rec[cursor..];
    if path_bytes.is_empty() {
        return None;
    }
    Some((fields, path_bytes))
}

fn decode_git_path(path_bytes: &[u8], offset: usize, out: &mut ParsedGitStatus) -> Option<String> {
    if path_bytes.is_empty() || path_bytes.contains(&0) {
        out.malformed_records += 1;
        record_issue(out, ScmParseIssue::MalformedRecord { offset });
        return None;
    }
    let Ok(s) = std::str::from_utf8(path_bytes) else {
        out.non_utf8_paths += 1;
        record_issue(out, ScmParseIssue::NonUtf8Path { offset });
        return None;
    };
    Some(s.to_string())
}

struct BoundedEntryCollector {
    max_entries: usize,
    total_entries: usize,
    kept_tracked: Vec<(usize, ScmEntry)>,
    kept_untracked: VecDeque<(usize, ScmEntry)>,
}

impl BoundedEntryCollector {
    fn new(max_entries: usize) -> Self {
        Self {
            max_entries,
            total_entries: 0,
            kept_tracked: Vec::new(),
            kept_untracked: VecDeque::new(),
        }
    }

    fn push(&mut self, entry: ScmEntry) {
        let seq = self.total_entries;
        self.total_entries += 1;
        if self.max_entries == 0 {
            return;
        }
        if entry.area != ScmArea::Untracked {
            if self.kept_tracked.len() < self.max_entries {
                self.kept_tracked.push((seq, entry));
                while self.kept_tracked.len() + self.kept_untracked.len() > self.max_entries {
                    let _ = self.kept_untracked.pop_back();
                }
            }
        } else if self.kept_tracked.len() + self.kept_untracked.len() < self.max_entries {
            self.kept_untracked.push_back((seq, entry));
        }
    }

    fn finish(mut self) -> (Vec<ScmEntry>, usize, bool) {
        let truncated = self.total_entries > self.max_entries;
        let total = self.total_entries;
        let cap = self.kept_tracked.len() + self.kept_untracked.len();
        let mut merged = Vec::with_capacity(cap);
        let mut tracked_iter = self.kept_tracked.drain(..).peekable();
        while let Some(&(t_seq, _)) = tracked_iter.peek() {
            if let Some(&(u_seq, _)) = self.kept_untracked.front()
                && u_seq < t_seq
            {
                if let Some((_, u_entry)) = self.kept_untracked.pop_front() {
                    merged.push(u_entry);
                }
            } else if let Some((_, t_entry)) = tracked_iter.next() {
                merged.push(t_entry);
            }
        }
        while let Some((_, u_entry)) = self.kept_untracked.pop_front() {
            merged.push(u_entry);
        }
        (merged, total, truncated)
    }
}

/// Parses `git status --porcelain=v2 --branch -z` using [`STATUS_RAW_BYTES_MAX`] (`8 MiB`) and
/// `max_entries`.
#[must_use]
pub fn parse_porcelain_v2_status(raw: &[u8], max_entries: usize) -> ParsedGitStatus {
    parse_porcelain_v2_status_bounded(raw, STATUS_RAW_BYTES_MAX, max_entries)
}

/// Parses `git status --porcelain=v2 --branch -z` with explicit byte and entry ceilings.
///
/// - Never uses lossy UTF-8 decoding (`from_utf8_lossy` is forbidden).
/// - Explicitly records incomplete records, malformed records, and non-UTF-8 paths without
///   fabricating entries.
/// - Always consumes both NUL-terminated paths of `2 <XY> ...` rename/copy records even after
///   `max_entries` is reached so `orig_path` is never misread as the next status record.
#[must_use]
pub fn parse_porcelain_v2_status_bounded(
    raw: &[u8],
    max_bytes: usize,
    max_entries: usize,
) -> ParsedGitStatus {
    let mut out = ParsedGitStatus::default();
    if raw.is_empty() {
        return out;
    }

    let slice = if raw.len() > max_bytes {
        out.input_bytes_truncated = true;
        out.truncated = true;
        record_issue(
            &mut out,
            ScmParseIssue::InputBytesExceeded {
                max_bytes,
                actual_bytes: raw.len(),
            },
        );
        &raw[..max_bytes]
    } else {
        raw
    };

    let mut collector = BoundedEntryCollector::new(max_entries);
    let mut pos = 0_usize;

    while pos < slice.len() {
        let Some(rel_nul) = slice[pos..].iter().position(|&b| b == 0) else {
            if !out.input_bytes_truncated {
                out.malformed_records += 1;
                record_issue(&mut out, ScmParseIssue::IncompleteRecord { offset: pos });
            }
            break;
        };

        let rec_offset = pos;
        let rec = &slice[pos..pos + rel_nul];
        pos += rel_nul + 1;

        if rec.is_empty() {
            out.malformed_records += 1;
            record_issue(
                &mut out,
                ScmParseIssue::MalformedRecord { offset: rec_offset },
            );
            continue;
        }

        if rec.starts_with(b"# ") {
            let Ok(header_str) = std::str::from_utf8(&rec[2..]) else {
                out.malformed_records += 1;
                record_issue(
                    &mut out,
                    ScmParseIssue::MalformedRecord { offset: rec_offset },
                );
                continue;
            };
            let Some((key, val)) = header_str.split_once(' ') else {
                out.malformed_records += 1;
                record_issue(
                    &mut out,
                    ScmParseIssue::MalformedRecord { offset: rec_offset },
                );
                continue;
            };
            match key {
                "branch.head" => {
                    if val.is_empty() {
                        out.malformed_records += 1;
                        record_issue(
                            &mut out,
                            ScmParseIssue::MalformedRecord { offset: rec_offset },
                        );
                    } else if val == "(detached)" {
                        out.branch = None;
                    } else {
                        out.branch = Some(val.to_string());
                    }
                }
                "branch.oid" => {
                    if val.is_empty() {
                        out.malformed_records += 1;
                        record_issue(
                            &mut out,
                            ScmParseIssue::MalformedRecord { offset: rec_offset },
                        );
                    } else if val == "(initial)" {
                        out.oid = None;
                    } else {
                        out.oid = Some(val.to_string());
                    }
                }
                "branch.upstream" => {
                    if val.is_empty() {
                        out.malformed_records += 1;
                        record_issue(
                            &mut out,
                            ScmParseIssue::MalformedRecord { offset: rec_offset },
                        );
                    } else {
                        out.upstream = Some(val.to_string());
                    }
                }
                "branch.ab" => {
                    let parsed_ab = val.split_once(' ').and_then(|(a, b)| {
                        let ahead = a.strip_prefix('+')?.parse::<u32>().ok()?;
                        let behind = b.strip_prefix('-')?.parse::<u32>().ok()?;
                        Some((ahead, behind))
                    });
                    if let Some((ahead, behind)) = parsed_ab {
                        out.ahead = ahead;
                        out.behind = behind;
                        out.has_ab = true;
                    } else {
                        out.malformed_records += 1;
                        record_issue(
                            &mut out,
                            ScmParseIssue::MalformedRecord { offset: rec_offset },
                        );
                    }
                }
                other if !other.is_empty() => {}
                _ => {
                    out.malformed_records += 1;
                    record_issue(
                        &mut out,
                        ScmParseIssue::MalformedRecord { offset: rec_offset },
                    );
                }
            }
            continue;
        }

        if rec.starts_with(b"? ") {
            if let Some(path) = decode_git_path(&rec[2..], rec_offset + 2, &mut out) {
                collector.push(ScmEntry {
                    path,
                    old_path: None,
                    area: ScmArea::Untracked,
                    status: ScmFileStatus::Untracked,
                    conflict: None,
                    added: None,
                    removed: None,
                });
            }
            continue;
        }

        if rec.starts_with(b"! ") {
            let _ = decode_git_path(&rec[2..], rec_offset + 2, &mut out);
            continue;
        }

        if rec.starts_with(b"u ") {
            let Some((fields, path_bytes)) = split_header_and_path(rec, 10) else {
                out.malformed_records += 1;
                record_issue(
                    &mut out,
                    ScmParseIssue::MalformedRecord { offset: rec_offset },
                );
                continue;
            };
            let Some(conflict_kind) = parse_conflict_xy(fields[1]) else {
                out.malformed_records += 1;
                record_issue(
                    &mut out,
                    ScmParseIssue::MalformedRecord { offset: rec_offset },
                );
                continue;
            };
            let path_offset = rec_offset + (rec.len() - path_bytes.len());
            if let Some(path) = decode_git_path(path_bytes, path_offset, &mut out) {
                collector.push(ScmEntry {
                    path,
                    old_path: None,
                    area: ScmArea::Unstaged,
                    status: ScmFileStatus::Modified,
                    conflict: Some(conflict_kind),
                    added: None,
                    removed: None,
                });
            }
            continue;
        }

        if rec.starts_with(b"1 ") {
            let Some((fields, path_bytes)) = split_header_and_path(rec, 8) else {
                out.malformed_records += 1;
                record_issue(
                    &mut out,
                    ScmParseIssue::MalformedRecord { offset: rec_offset },
                );
                continue;
            };
            let xy = fields[1];
            if xy.len() != 2 || xy == b".." {
                out.malformed_records += 1;
                record_issue(
                    &mut out,
                    ScmParseIssue::MalformedRecord { offset: rec_offset },
                );
                continue;
            }
            let x_status = if xy[0] == b'.' {
                None
            } else {
                match parse_status_letter(xy[0]) {
                    Some(st) => Some(st),
                    None => {
                        out.malformed_records += 1;
                        record_issue(
                            &mut out,
                            ScmParseIssue::MalformedRecord { offset: rec_offset },
                        );
                        continue;
                    }
                }
            };
            let y_status = if xy[1] == b'.' {
                None
            } else {
                match parse_status_letter(xy[1]) {
                    Some(st) => Some(st),
                    None => {
                        out.malformed_records += 1;
                        record_issue(
                            &mut out,
                            ScmParseIssue::MalformedRecord { offset: rec_offset },
                        );
                        continue;
                    }
                }
            };

            let path_offset = rec_offset + (rec.len() - path_bytes.len());
            let Some(path) = decode_git_path(path_bytes, path_offset, &mut out) else {
                continue;
            };

            if let Some(st) = x_status {
                collector.push(ScmEntry {
                    path: path.clone(),
                    old_path: None,
                    area: ScmArea::Staged,
                    status: st,
                    conflict: None,
                    added: None,
                    removed: None,
                });
            }
            if let Some(st) = y_status {
                collector.push(ScmEntry {
                    path,
                    old_path: None,
                    area: ScmArea::Unstaged,
                    status: st,
                    conflict: None,
                    added: None,
                    removed: None,
                });
            }
            continue;
        }

        if rec.starts_with(b"2 ") {
            // Unconditionally consume the second NUL-terminated `<origPath>` token FIRST so
            // stream framing stays synchronized even when `max_entries` is exceeded or `rec`
            // itself is malformed.
            let Some(orig_rel_nul) = slice[pos..].iter().position(|&b| b == 0) else {
                if !out.input_bytes_truncated {
                    out.malformed_records += 1;
                    record_issue(
                        &mut out,
                        ScmParseIssue::IncompleteRecord { offset: rec_offset },
                    );
                }
                break;
            };
            let orig_offset = pos;
            let orig_bytes = &slice[pos..pos + orig_rel_nul];
            pos += orig_rel_nul + 1;

            let Some((fields, path_bytes)) = split_header_and_path(rec, 9) else {
                out.malformed_records += 1;
                record_issue(
                    &mut out,
                    ScmParseIssue::MalformedRecord { offset: rec_offset },
                );
                continue;
            };
            let xy = fields[1];
            let score_field = fields[8];
            let valid_score = score_field.len() >= 2
                && matches!(score_field[0], b'R' | b'C')
                && score_field[1..].iter().all(u8::is_ascii_digit);
            if xy.len() != 2 || xy == b".." || !valid_score {
                out.malformed_records += 1;
                record_issue(
                    &mut out,
                    ScmParseIssue::MalformedRecord { offset: rec_offset },
                );
                continue;
            }

            let x_status = if xy[0] == b'.' {
                None
            } else {
                match parse_status_letter(xy[0]) {
                    Some(st) => Some(st),
                    None => {
                        out.malformed_records += 1;
                        record_issue(
                            &mut out,
                            ScmParseIssue::MalformedRecord { offset: rec_offset },
                        );
                        continue;
                    }
                }
            };
            let y_status = if xy[1] == b'.' {
                None
            } else {
                match parse_status_letter(xy[1]) {
                    Some(st) => Some(st),
                    None => {
                        out.malformed_records += 1;
                        record_issue(
                            &mut out,
                            ScmParseIssue::MalformedRecord { offset: rec_offset },
                        );
                        continue;
                    }
                }
            };

            let path_offset = rec_offset + (rec.len() - path_bytes.len());
            let Some(path) = decode_git_path(path_bytes, path_offset, &mut out) else {
                let _ = decode_git_path(orig_bytes, orig_offset, &mut out);
                continue;
            };
            let Some(old_path) = decode_git_path(orig_bytes, orig_offset, &mut out) else {
                continue;
            };

            if let Some(st) = x_status {
                let staged_old = matches!(st, ScmFileStatus::Renamed | ScmFileStatus::Copied)
                    .then(|| old_path.clone());
                collector.push(ScmEntry {
                    path: path.clone(),
                    old_path: staged_old,
                    area: ScmArea::Staged,
                    status: st,
                    conflict: None,
                    added: None,
                    removed: None,
                });
            }
            if let Some(st) = y_status {
                let unstaged_old = (x_status.is_none()
                    && matches!(st, ScmFileStatus::Renamed | ScmFileStatus::Copied))
                .then_some(old_path);
                collector.push(ScmEntry {
                    path,
                    old_path: unstaged_old,
                    area: ScmArea::Unstaged,
                    status: st,
                    conflict: None,
                    added: None,
                    removed: None,
                });
            }
            continue;
        }

        out.malformed_records += 1;
        record_issue(
            &mut out,
            ScmParseIssue::MalformedRecord { offset: rec_offset },
        );
    }

    let (entries, total_entries, entry_cap_truncated) = collector.finish();
    out.entries = entries;
    out.total_entries = total_entries;
    if entry_cap_truncated {
        out.truncated = true;
    }
    out
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScmViewMode {
    #[default]
    Tree,
    List,
}

#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScmGroupOrder {
    #[default]
    ChangesFirst,
    StagedFirst,
    UntrackedFirst,
}

impl ScmGroupOrder {
    const fn areas(self) -> [ScmArea; 3] {
        match self {
            Self::ChangesFirst => [ScmArea::Unstaged, ScmArea::Staged, ScmArea::Untracked],
            Self::StagedFirst => [ScmArea::Staged, ScmArea::Unstaged, ScmArea::Untracked],
            Self::UntrackedFirst => [ScmArea::Untracked, ScmArea::Unstaged, ScmArea::Staged],
        }
    }
}

/// Explicit capability flags (`#[derive(Default)]` sets all capabilities to `false`).
///
/// Any capability set to `false` strictly omits its controls so zero dead/dummy buttons exist.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScmCapabilities {
    pub can_refresh: bool,
    pub can_open_diff: bool,
    pub can_open_file: bool,
    pub can_stage: bool,
    pub can_unstage: bool,
    pub can_discard: bool,
    pub can_commit: bool,
    pub can_remote: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScmRemoteOp {
    Push,
    Pull,
    FastForward,
    Sync,
    Fetch,
    Publish,
}

#[derive(Clone, Copy, Debug)]
pub struct ScmStatusSnapshot<'a> {
    pub is_repo: bool,
    pub branch: Option<&'a str>,
    pub upstream: Option<&'a str>,
    pub ahead: u32,
    pub behind: u32,
    pub entries: &'a [ScmEntry],
    pub truncated: bool,
    pub parse_incomplete: bool,
    pub total_entries: usize,
}

impl<'a> ScmStatusSnapshot<'a> {
    #[must_use]
    pub fn from_parsed(is_repo: bool, parsed: &'a ParsedGitStatus) -> Self {
        Self {
            is_repo,
            branch: parsed.branch.as_deref(),
            upstream: parsed.upstream.as_deref(),
            ahead: parsed.ahead,
            behind: parsed.behind,
            entries: &parsed.entries,
            truncated: parsed.truncated,
            parse_incomplete: parsed.has_parse_issues(),
            total_entries: parsed.total_entries,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ScmChangesInput<'a> {
    pub project_selected: bool,
    pub loaded: bool,
    pub busy: bool,
    pub read_only: bool,
    pub french: bool,
    pub error: Option<&'a str>,
    pub status: Option<ScmStatusSnapshot<'a>>,
    pub active_entry: Option<(ScmArea, &'a str)>,
    pub capabilities: ScmCapabilities,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScmChangesAction {
    Refresh,
    OpenDiff {
        area: ScmArea,
        path: String,
        old_path: Option<String>,
        pinned: bool,
    },
    OpenFile(String),
    Stage(Vec<String>),
    Unstage(Vec<String>),
    RequestDiscard {
        area: ScmArea,
        paths: Vec<String>,
    },
    Commit {
        message: String,
    },
    Remote(ScmRemoteOp),
}

// ---------------------------------------------------------------------------
// Natural Numeric File Sort & Compacted Source Control Tree (`sourceControlTree.js`)
// ---------------------------------------------------------------------------

fn compare_file_names(a: &str, b: &str) -> std::cmp::Ordering {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    let mut ia = 0_usize;
    let mut ib = 0_usize;

    while ia < ab.len() && ib < bb.len() {
        if ab[ia].is_ascii_digit() && bb[ib].is_ascii_digit() {
            let za = ia;
            while ia < ab.len() && ab[ia] == b'0' {
                ia += 1;
            }
            let na_start = ia;
            while ia < ab.len() && ab[ia].is_ascii_digit() {
                ia += 1;
            }

            let zb = ib;
            while ib < bb.len() && bb[ib] == b'0' {
                ib += 1;
            }
            let nb_start = ib;
            while ib < bb.len() && bb[ib].is_ascii_digit() {
                ib += 1;
            }

            let len_a = ia - na_start;
            let len_b = ib - nb_start;
            if len_a != len_b {
                return len_a.cmp(&len_b);
            }
            let cmp_digits = ab[na_start..ia].cmp(&bb[nb_start..ib]);
            if cmp_digits != std::cmp::Ordering::Equal {
                return cmp_digits;
            }
            let total_digits_a = ia - za;
            let total_digits_b = ib - zb;
            if total_digits_a != total_digits_b {
                return total_digits_a.cmp(&total_digits_b);
            }
            continue;
        }

        let ca = ab[ia].to_ascii_lowercase();
        let cb = bb[ib].to_ascii_lowercase();
        if ca != cb {
            return ca.cmp(&cb);
        }
        ia += 1;
        ib += 1;
    }

    ab.len().cmp(&bb.len()).then_with(|| a.cmp(b))
}

fn compare_entries(a: &ScmEntry, b: &ScmEntry) -> std::cmp::Ordering {
    let rank_a = u8::from(!a.is_conflict());
    let rank_b = u8::from(!b.is_conflict());
    rank_a
        .cmp(&rank_b)
        .then_with(|| compare_file_names(&a.path, &b.path))
}

fn split_path_segments(path: &str) -> Vec<&str> {
    path.split(['/', '\\']).filter(|s| !s.is_empty()).collect()
}

fn file_and_dir_name(path: &str) -> (&str, Option<String>) {
    let segs = split_path_segments(path);
    match segs.as_slice() {
        [] => (path, None),
        [only] => (*only, None),
        many => {
            let last = many[many.len() - 1];
            let dir = many[..many.len() - 1].join("/");
            (last, Some(dir))
        }
    }
}

#[derive(Clone, Debug)]
struct DisplaySection<'a> {
    kind: ScmSectionKind,
    area: ScmArea,
    items: Vec<&'a ScmEntry>,
}

fn build_display_sections<'a>(
    entries: &'a [ScmEntry],
    order: ScmGroupOrder,
    filter_norm: &str,
) -> Vec<DisplaySection<'a>> {
    let matches_filter = |entry: &ScmEntry| -> bool {
        if filter_norm.is_empty() {
            return true;
        }
        let path_lc = entry.path.to_lowercase();
        if path_lc.contains(filter_norm) {
            return true;
        }
        entry
            .old_path
            .as_ref()
            .is_some_and(|old| old.to_lowercase().contains(filter_norm))
    };

    let mut conflicts: Vec<&'a ScmEntry> = Vec::new();
    let mut staged: Vec<&'a ScmEntry> = Vec::new();
    let mut unstaged: Vec<&'a ScmEntry> = Vec::new();
    let mut untracked: Vec<&'a ScmEntry> = Vec::new();

    for entry in entries {
        if !matches_filter(entry) {
            continue;
        }
        if entry.is_conflict() {
            conflicts.push(entry);
        } else {
            match entry.area {
                ScmArea::Staged => staged.push(entry),
                ScmArea::Unstaged => unstaged.push(entry),
                ScmArea::Untracked => untracked.push(entry),
            }
        }
    }

    conflicts.sort_by(|a, b| compare_entries(a, b));
    staged.sort_by(|a, b| compare_entries(a, b));
    unstaged.sort_by(|a, b| compare_entries(a, b));
    untracked.sort_by(|a, b| compare_entries(a, b));

    let mut sections = Vec::with_capacity(4);
    if !conflicts.is_empty() {
        sections.push(DisplaySection {
            kind: ScmSectionKind::Conflicts,
            area: ScmArea::Unstaged,
            items: conflicts,
        });
    }
    for area in order.areas() {
        let (kind, items) = match area {
            ScmArea::Staged => (ScmSectionKind::Staged, std::mem::take(&mut staged)),
            ScmArea::Unstaged => (ScmSectionKind::Unstaged, std::mem::take(&mut unstaged)),
            ScmArea::Untracked => (ScmSectionKind::Untracked, std::mem::take(&mut untracked)),
        };
        if !items.is_empty() {
            sections.push(DisplaySection { kind, area, items });
        }
    }
    sections
}

#[derive(Clone, Debug)]
enum TreeNode<'a> {
    Directory {
        key: String,
        name: String,
        path: String,
        area: ScmArea,
        depth: usize,
        file_count: usize,
        children: Vec<TreeNode<'a>>,
    },
    File {
        key: String,
        name: String,
        entry: &'a ScmEntry,
        depth: usize,
    },
}

struct DirBuilder<'a> {
    name: String,
    path: String,
    subdirs: HashMap<String, DirBuilder<'a>>,
    files: Vec<(&'a str, &'a ScmEntry)>,
}

impl<'a> DirBuilder<'a> {
    fn new(name: String, path: String) -> Self {
        Self {
            name,
            path,
            subdirs: HashMap::new(),
            files: Vec::new(),
        }
    }

    fn finalize(
        mut self,
        namespace: &str,
        area: ScmArea,
        depth: usize,
    ) -> (Vec<TreeNode<'a>>, usize) {
        let mut dir_list: Vec<TreeNode<'a>> = Vec::with_capacity(self.subdirs.len());
        let mut sub_entries: Vec<(String, DirBuilder<'a>)> = self.subdirs.drain().collect();
        sub_entries.sort_by(|(a, _), (b, _)| compare_file_names(a, b));

        let mut total_files = self.files.len();
        for (_, sub) in sub_entries {
            let sub_name = sub.name.clone();
            let sub_path = sub.path.clone();
            let (children, sub_count) = sub.finalize(namespace, area, depth + 1);
            total_files += sub_count;
            dir_list.push(TreeNode::Directory {
                key: format!("dir::{namespace}::{sub_path}"),
                name: sub_name,
                path: sub_path,
                area,
                depth,
                file_count: sub_count,
                children,
            });
        }

        self.files.sort_by(|(_, a), (_, b)| compare_entries(a, b));
        let mut out = Vec::with_capacity(dir_list.len() + self.files.len());
        out.extend(dir_list);
        for (fname, entry) in self.files {
            out.push(TreeNode::File {
                key: format!("file::{namespace}::{}::{}", entry.area.as_str(), entry.path),
                name: fname.to_string(),
                entry,
                depth,
            });
        }
        (out, total_files)
    }
}

fn compact_tree_node<'a>(node: TreeNode<'a>, depth: usize) -> TreeNode<'a> {
    match node {
        TreeNode::File {
            key, name, entry, ..
        } => TreeNode::File {
            key,
            name,
            entry,
            depth,
        },
        TreeNode::Directory {
            mut key,
            name,
            mut path,
            area,
            file_count,
            mut children,
            ..
        } => {
            let mut chain_names = vec![name];
            while children.len() == 1
                && matches!(children.first(), Some(TreeNode::Directory { .. }))
            {
                let Some(TreeNode::Directory {
                    key: child_key,
                    name: child_name,
                    path: child_path,
                    children: child_children,
                    ..
                }) = children.pop()
                else {
                    break;
                };
                key = child_key;
                path = child_path;
                chain_names.push(child_name);
                children = child_children;
            }
            let compacted_children = children
                .into_iter()
                .map(|c| compact_tree_node(c, depth + 1))
                .collect();
            TreeNode::Directory {
                key,
                name: chain_names.join("/"),
                path,
                area,
                depth,
                file_count,
                children: compacted_children,
            }
        }
    }
}

fn collect_dir_entries<'a>(children: &[TreeNode<'a>], out: &mut Vec<&'a ScmEntry>) {
    for child in children {
        match child {
            TreeNode::File { entry, .. } => out.push(entry),
            TreeNode::Directory { children, .. } => collect_dir_entries(children, out),
        }
    }
}

#[derive(Clone, Debug)]
enum FlatRow<'a> {
    Directory {
        key: String,
        parent_key: Option<String>,
        name: String,
        path: String,
        area: ScmArea,
        depth: usize,
        file_count: usize,
        expanded: bool,
        stage_paths: Vec<String>,
        unstage_paths: Vec<String>,
        discard_paths: Vec<String>,
    },
    File {
        key: String,
        parent_key: Option<String>,
        name: String,
        dir_hint: Option<String>,
        entry: &'a ScmEntry,
        depth: usize,
    },
}

fn flatten_tree_nodes<'a>(
    nodes: &[TreeNode<'a>],
    collapsed_dirs: &HashSet<String>,
    parent_key: Option<&str>,
    out: &mut Vec<FlatRow<'a>>,
) {
    for node in nodes {
        match node {
            TreeNode::Directory {
                key,
                name,
                path,
                area,
                depth,
                file_count,
                children,
            } => {
                let expanded = !collapsed_dirs.contains(key);
                let mut descendants = Vec::new();
                collect_dir_entries(children, &mut descendants);
                let stage_paths: Vec<String> = descendants
                    .iter()
                    .filter(|e| e.can_stage())
                    .map(|e| e.path.clone())
                    .collect();
                let unstage_paths: Vec<String> = descendants
                    .iter()
                    .filter(|e| e.can_unstage())
                    .map(|e| e.path.clone())
                    .collect();
                let discard_paths: Vec<String> =
                    if matches!(area, ScmArea::Unstaged | ScmArea::Untracked) {
                        descendants
                            .iter()
                            .filter(|e| e.area == *area && e.can_discard())
                            .map(|e| e.path.clone())
                            .collect()
                    } else {
                        Vec::new()
                    };

                out.push(FlatRow::Directory {
                    key: key.clone(),
                    parent_key: parent_key.map(str::to_string),
                    name: name.clone(),
                    path: path.clone(),
                    area: *area,
                    depth: *depth,
                    file_count: *file_count,
                    expanded,
                    stage_paths,
                    unstage_paths,
                    discard_paths,
                });
                if expanded {
                    flatten_tree_nodes(children, collapsed_dirs, Some(key.as_str()), out);
                }
            }
            TreeNode::File {
                key,
                name,
                entry,
                depth,
            } => {
                out.push(FlatRow::File {
                    key: key.clone(),
                    parent_key: parent_key.map(str::to_string),
                    name: name.clone(),
                    dir_hint: None,
                    entry,
                    depth: *depth,
                });
            }
        }
    }
}

fn build_section_rows<'a>(
    section: &DisplaySection<'a>,
    view_mode: ScmViewMode,
    collapsed_dirs: &HashSet<String>,
) -> Vec<FlatRow<'a>> {
    let namespace = match section.kind {
        ScmSectionKind::Conflicts => "conflicts",
        ScmSectionKind::Staged => "staged",
        ScmSectionKind::Unstaged => "unstaged",
        ScmSectionKind::Untracked => "untracked",
    };

    if view_mode == ScmViewMode::List {
        return section
            .items
            .iter()
            .map(|entry| {
                let (fname, dir_hint) = file_and_dir_name(&entry.path);
                FlatRow::File {
                    key: format!("file::{namespace}::{}::{}", entry.area.as_str(), entry.path),
                    parent_key: None,
                    name: fname.to_string(),
                    dir_hint,
                    entry,
                    depth: 0,
                }
            })
            .collect();
    }

    let mut root = DirBuilder::new(String::new(), String::new());
    for entry in &section.items {
        let segs = split_path_segments(&entry.path);
        if segs.is_empty() {
            continue;
        }
        let mut cur = &mut root;
        let mut ancestor = String::new();
        for seg in &segs[..segs.len() - 1] {
            if !ancestor.is_empty() {
                ancestor.push('/');
            }
            ancestor.push_str(seg);
            let next_path = ancestor.clone();
            cur = cur
                .subdirs
                .entry((*seg).to_string())
                .or_insert_with(|| DirBuilder::new((*seg).to_string(), next_path));
        }
        cur.files.push((segs[segs.len() - 1], entry));
    }

    let (raw_roots, _) = root.finalize(namespace, section.area, 0);
    let compacted: Vec<TreeNode<'a>> = raw_roots
        .into_iter()
        .map(|n| compact_tree_node(n, 0))
        .collect();
    let mut rows = Vec::new();
    flatten_tree_nodes(&compacted, collapsed_dirs, None, &mut rows);
    rows
}

// ---------------------------------------------------------------------------
// Primary Commit / Stage / Remote Action Resolution (`sourceControl.js`)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrimaryKind {
    Commit,
    StageAll,
    Remote(ScmRemoteOp),
}

struct ResolvedPrimaryAction {
    kind: PrimaryKind,
    label: String,
    tooltip: String,
    enabled: bool,
}

#[allow(clippy::too_many_arguments)]
fn resolve_primary_action(
    staged_count: usize,
    unstaged_count: usize,
    stageable_count: usize,
    has_conflicts: bool,
    has_message: bool,
    busy: bool,
    bulk_blocked: bool,
    total_entries: usize,
    status: &ScmStatusSnapshot<'_>,
    caps: ScmCapabilities,
    lang: ScmLang,
    french: bool,
) -> ResolvedPrimaryAction {
    if busy {
        return ResolvedPrimaryAction {
            kind: PrimaryKind::Commit,
            label: scm_text("Commit", lang),
            tooltip: scm_text("Commit in progress…", lang),
            enabled: false,
        };
    }
    if has_conflicts {
        return ResolvedPrimaryAction {
            kind: PrimaryKind::Commit,
            label: scm_text("Commit", lang),
            tooltip: scm_text("Resolve conflicts before committing", lang),
            enabled: false,
        };
    }

    if staged_count > 0 && caps.can_commit {
        if has_message {
            return ResolvedPrimaryAction {
                kind: PrimaryKind::Commit,
                label: scm_text("Commit", lang),
                tooltip: scm_text("Commit staged changes", lang),
                enabled: true,
            };
        }
        return ResolvedPrimaryAction {
            kind: PrimaryKind::Commit,
            label: scm_text("Commit", lang),
            tooltip: scm_text("Enter a commit message to commit", lang),
            enabled: false,
        };
    }

    if staged_count == 0 && stageable_count > 0 && caps.can_stage {
        if bulk_blocked {
            let shown_total = total_entries.max(status.entries.len());
            let total_is_min = status.parse_incomplete || total_entries <= status.entries.len();
            let tip = match (french, total_is_min) {
                (true, true) => format!(
                    "Les modifications ne sont pas toutes listées (au moins {shown_total}) : indexez-les toutes depuis un terminal (git add -A)."
                ),
                (true, false) => format!(
                    "Les modifications ne sont pas toutes listées ({shown_total}) : indexez-les toutes depuis un terminal (git add -A)."
                ),
                (false, true) => format!(
                    "Not all changes are listed (at least {shown_total}): stage them all from a terminal (git add -A)."
                ),
                (false, false) => format!(
                    "Not all changes are listed ({shown_total}): stage them all from a terminal (git add -A)."
                ),
            };
            return ResolvedPrimaryAction {
                kind: PrimaryKind::StageAll,
                label: scm_text("Stage All", lang),
                tooltip: tip,
                enabled: false,
            };
        }
        return ResolvedPrimaryAction {
            kind: PrimaryKind::StageAll,
            label: scm_text("Stage All", lang),
            tooltip: scm_text("Stage all changes", lang),
            enabled: true,
        };
    }

    if caps.can_remote {
        if status.upstream.is_none() {
            if status.branch.is_some() {
                let tip = if french {
                    "Publier cette branche vers origin".to_string()
                } else {
                    "Publish this branch to origin".to_string()
                };
                return ResolvedPrimaryAction {
                    kind: PrimaryKind::Remote(ScmRemoteOp::Publish),
                    label: scm_text("Publish Branch", lang),
                    tooltip: tip,
                    enabled: true,
                };
            }
            let tip = if french {
                "Effectuez un checkout d'une branche avant de publier des commits.".to_string()
            } else {
                "Check out a branch before publishing commits.".to_string()
            };
            return ResolvedPrimaryAction {
                kind: PrimaryKind::Commit,
                label: scm_text("Commit", lang),
                tooltip: tip,
                enabled: false,
            };
        }

        if status.ahead > 0 && status.behind > 0 {
            let raw_sync = format!("Sync (↓{} ↑{})", status.behind, status.ahead);
            return ResolvedPrimaryAction {
                kind: PrimaryKind::Remote(ScmRemoteOp::Sync),
                label: scm_text("Sync", lang),
                tooltip: scm_text(&raw_sync, lang),
                enabled: true,
            };
        }
        if status.behind > 0 {
            let raw_pull = if status.behind == 1 {
                "Pull 1 commit".to_string()
            } else {
                format!("Pull {} commits", status.behind)
            };
            return ResolvedPrimaryAction {
                kind: PrimaryKind::Remote(ScmRemoteOp::Pull),
                label: scm_text("Pull", lang),
                tooltip: scm_text(&raw_pull, lang),
                enabled: true,
            };
        }
        if status.ahead > 0 {
            let raw_push = if status.ahead == 1 {
                "Push 1 commit".to_string()
            } else {
                format!("Push {} commits", status.ahead)
            };
            return ResolvedPrimaryAction {
                kind: PrimaryKind::Remote(ScmRemoteOp::Push),
                label: scm_text("Push", lang),
                tooltip: scm_text(&raw_push, lang),
                enabled: true,
            };
        }
    }

    let fallback_tip = if unstaged_count > 0 || stageable_count > 0 {
        scm_text("Stage at least one file to commit", lang)
    } else {
        scm_text("Nothing to commit. Branch is up to date.", lang)
    };
    ResolvedPrimaryAction {
        kind: PrimaryKind::Commit,
        label: scm_text("Commit", lang),
        tooltip: fallback_tip,
        enabled: false,
    }
}

fn commit_textarea_rows(msg: &str) -> usize {
    if msg.is_empty() {
        return 2;
    }
    let mut rows = 1_usize;
    for b in msg.bytes().take(64 * 1024) {
        if b == b'\n' {
            rows += 1;
            if rows >= 12 {
                break;
            }
        }
    }
    rows.clamp(2, 12)
}

fn clamp_chars_in_place(text: &mut String, max_chars: usize) {
    if let Some((byte_idx, _)) = text.char_indices().nth(max_chars) {
        text.truncate(byte_idx);
    }
}

// ---------------------------------------------------------------------------
// Pure Vector Strokes (Zero ASCII / Unicode Glyph Fallbacks)
// ---------------------------------------------------------------------------

fn fit_square(rect: Rect) -> Option<(Pos2, f32)> {
    if !rect.is_positive() || !rect.min.x.is_finite() || !rect.min.y.is_finite() {
        return None;
    }
    let side = rect.width().min(rect.height());
    if side <= 0.0_f32 || !side.is_finite() {
        return None;
    }
    let sq = Rect::from_center_size(rect.center(), Vec2::splat(side));
    Some((sq.min, side))
}

fn pt_in(origin: Pos2, side: f32, viewbox: f32, x: f32, y: f32) -> Pos2 {
    let s = side / viewbox;
    Pos2::new(origin.x + x * s, origin.y + y * s)
}

fn paint_chevron_right(painter: &Painter, rect: Rect, color: Color32) {
    let Some((o, side)) = fit_square(rect) else {
        return;
    };
    let stroke = Stroke::new((side / 16.0_f32) * 1.8_f32, color);
    painter.line_segment(
        [
            pt_in(o, side, 16.0, 6.0, 4.0),
            pt_in(o, side, 16.0, 10.0, 8.0),
        ],
        stroke,
    );
    painter.line_segment(
        [
            pt_in(o, side, 16.0, 10.0, 8.0),
            pt_in(o, side, 16.0, 6.0, 12.0),
        ],
        stroke,
    );
}

fn paint_minus(painter: &Painter, rect: Rect, color: Color32) {
    let Some((o, side)) = fit_square(rect) else {
        return;
    };
    let stroke = Stroke::new((side / 16.0_f32) * 1.5_f32, color);
    painter.line_segment(
        [
            pt_in(o, side, 16.0, 3.2, 8.0),
            pt_in(o, side, 16.0, 12.8, 8.0),
        ],
        stroke,
    );
}

fn paint_undo(painter: &Painter, rect: Rect, color: Color32) {
    let Some((o, side)) = fit_square(rect) else {
        return;
    };
    let stroke = Stroke::new((side / 24.0_f32) * 2.0_f32, color);
    painter.line_segment(
        [
            pt_in(o, side, 24.0, 9.0, 14.0),
            pt_in(o, side, 24.0, 4.0, 9.0),
        ],
        stroke,
    );
    painter.line_segment(
        [
            pt_in(o, side, 24.0, 4.0, 9.0),
            pt_in(o, side, 24.0, 9.0, 4.0),
        ],
        stroke,
    );
    painter.line_segment(
        [
            pt_in(o, side, 24.0, 4.0, 9.0),
            pt_in(o, side, 24.0, 14.5, 9.0),
        ],
        stroke,
    );
    let mut arc_pts = Vec::with_capacity(9);
    for i in 0..=8 {
        let angle = -std::f32::consts::FRAC_PI_2 + (i as f32 / 8.0_f32) * std::f32::consts::PI;
        let (sn, cs) = angle.sin_cos();
        arc_pts.push(pt_in(o, side, 24.0, 14.5 + 5.5 * cs, 14.5 + 5.5 * sn));
    }
    painter.add(egui::Shape::line(arc_pts, stroke));
    painter.line_segment(
        [
            pt_in(o, side, 24.0, 14.5, 20.0),
            pt_in(o, side, 24.0, 11.0, 20.0),
        ],
        stroke,
    );
}

fn paint_trash(painter: &Painter, rect: Rect, color: Color32) {
    let Some((o, side)) = fit_square(rect) else {
        return;
    };
    let stroke = Stroke::new((side / 24.0_f32) * 1.9_f32, color);
    painter.line_segment(
        [
            pt_in(o, side, 24.0, 3.0, 6.0),
            pt_in(o, side, 24.0, 21.0, 6.0),
        ],
        stroke,
    );
    painter.add(egui::Shape::line(
        vec![
            pt_in(o, side, 24.0, 8.0, 6.0),
            pt_in(o, side, 24.0, 8.0, 4.0),
            pt_in(o, side, 24.0, 16.0, 4.0),
            pt_in(o, side, 24.0, 16.0, 6.0),
        ],
        stroke,
    ));
    painter.add(egui::Shape::line(
        vec![
            pt_in(o, side, 24.0, 19.0, 6.0),
            pt_in(o, side, 24.0, 18.0, 20.0),
            pt_in(o, side, 24.0, 6.0, 20.0),
            pt_in(o, side, 24.0, 5.0, 6.0),
        ],
        stroke,
    ));
    painter.line_segment(
        [
            pt_in(o, side, 24.0, 10.0, 11.0),
            pt_in(o, side, 24.0, 10.0, 17.0),
        ],
        stroke,
    );
    painter.line_segment(
        [
            pt_in(o, side, 24.0, 14.0, 11.0),
            pt_in(o, side, 24.0, 14.0, 17.0),
        ],
        stroke,
    );
}

fn paint_refresh(painter: &Painter, rect: Rect, color: Color32) {
    let Some((o, side)) = fit_square(rect) else {
        return;
    };
    let stroke = Stroke::new((side / 24.0_f32) * 1.9_f32, color);
    let mut top_arc = Vec::with_capacity(9);
    for i in 0..=8 {
        let a = std::f32::consts::PI * 1.12_f32 + (i as f32 / 8.0_f32) * 2.35_f32;
        let (sn, cs) = a.sin_cos();
        top_arc.push(pt_in(o, side, 24.0, 12.0 + 7.5 * cs, 12.0 + 7.5 * sn));
    }
    painter.add(egui::Shape::line(top_arc, stroke));
    painter.add(egui::Shape::line(
        vec![
            pt_in(o, side, 24.0, 20.5, 4.5),
            pt_in(o, side, 24.0, 20.5, 9.0),
            pt_in(o, side, 24.0, 16.0, 9.0),
        ],
        stroke,
    ));

    let mut bot_arc = Vec::with_capacity(9);
    for i in 0..=8 {
        let a = std::f32::consts::PI * 0.12_f32 + (i as f32 / 8.0_f32) * 2.35_f32;
        let (sn, cs) = a.sin_cos();
        bot_arc.push(pt_in(o, side, 24.0, 12.0 + 7.5 * cs, 12.0 + 7.5 * sn));
    }
    painter.add(egui::Shape::line(bot_arc, stroke));
    painter.add(egui::Shape::line(
        vec![
            pt_in(o, side, 24.0, 3.5, 19.5),
            pt_in(o, side, 24.0, 3.5, 15.0),
            pt_in(o, side, 24.0, 8.0, 15.0),
        ],
        stroke,
    ));
}

fn paint_view_list(painter: &Painter, rect: Rect, color: Color32) {
    let Some((o, side)) = fit_square(rect) else {
        return;
    };
    let stroke = Stroke::new((side / 24.0_f32) * 2.0_f32, color);
    let r = (side / 24.0_f32) * 1.2_f32;
    for y in [6.0_f32, 12.0_f32, 18.0_f32] {
        painter.circle_filled(pt_in(o, side, 24.0, 4.5, y), r, color);
        painter.line_segment(
            [pt_in(o, side, 24.0, 8.5, y), pt_in(o, side, 24.0, 20.5, y)],
            stroke,
        );
    }
}

fn paint_view_tree(painter: &Painter, rect: Rect, color: Color32) {
    let Some((o, side)) = fit_square(rect) else {
        return;
    };
    let stroke = Stroke::new((side / 24.0_f32) * 2.0_f32, color);
    painter.line_segment(
        [
            pt_in(o, side, 24.0, 4.0, 6.0),
            pt_in(o, side, 24.0, 20.0, 6.0),
        ],
        stroke,
    );
    painter.add(egui::Shape::line(
        vec![
            pt_in(o, side, 24.0, 6.0, 6.0),
            pt_in(o, side, 24.0, 6.0, 18.0),
            pt_in(o, side, 24.0, 10.0, 18.0),
        ],
        stroke,
    ));
    painter.line_segment(
        [
            pt_in(o, side, 24.0, 6.0, 12.0),
            pt_in(o, side, 24.0, 10.0, 12.0),
        ],
        stroke,
    );
    painter.line_segment(
        [
            pt_in(o, side, 24.0, 13.0, 12.0),
            pt_in(o, side, 24.0, 20.0, 12.0),
        ],
        stroke,
    );
    painter.line_segment(
        [
            pt_in(o, side, 24.0, 13.0, 18.0),
            pt_in(o, side, 24.0, 20.0, 18.0),
        ],
        stroke,
    );
}

fn paint_folder_icon(painter: &Painter, rect: Rect, open: bool, color: Color32) {
    let Some((o, side)) = fit_square(rect) else {
        return;
    };
    let stroke = Stroke::new((side / 16.0_f32) * 1.35_f32, color);
    painter.add(egui::Shape::closed_line(
        vec![
            pt_in(o, side, 16.0, 1.8, 3.5),
            pt_in(o, side, 16.0, 6.2, 3.5),
            pt_in(o, side, 16.0, 7.6, 5.2),
            pt_in(o, side, 16.0, 14.2, 5.2),
            pt_in(o, side, 16.0, 14.2, 12.8),
            pt_in(o, side, 16.0, 1.8, 12.8),
        ],
        stroke,
    ));
    if open {
        painter.line_segment(
            [
                pt_in(o, side, 16.0, 2.0, 8.2),
                pt_in(o, side, 16.0, 14.0, 8.2),
            ],
            stroke,
        );
    }
}

fn paint_file_icon(painter: &Painter, rect: Rect, color: Color32) {
    let Some((o, side)) = fit_square(rect) else {
        return;
    };
    let stroke = Stroke::new((side / 16.0_f32) * 1.3_f32, color);
    painter.add(egui::Shape::closed_line(
        vec![
            pt_in(o, side, 16.0, 3.5, 2.0),
            pt_in(o, side, 16.0, 9.8, 2.0),
            pt_in(o, side, 16.0, 12.8, 5.0),
            pt_in(o, side, 16.0, 12.8, 14.0),
            pt_in(o, side, 16.0, 3.5, 14.0),
        ],
        stroke,
    ));
    painter.add(egui::Shape::line(
        vec![
            pt_in(o, side, 16.0, 9.6, 2.2),
            pt_in(o, side, 16.0, 9.6, 5.2),
            pt_in(o, side, 16.0, 12.6, 5.2),
        ],
        stroke,
    ));
}

fn paint_alert_triangle(painter: &Painter, rect: Rect, color: Color32) {
    let Some((o, side)) = fit_square(rect) else {
        return;
    };
    let stroke = Stroke::new((side / 16.0_f32) * 1.4_f32, color);
    painter.add(egui::Shape::closed_line(
        vec![
            pt_in(o, side, 16.0, 8.0, 2.2),
            pt_in(o, side, 16.0, 14.4, 13.5),
            pt_in(o, side, 16.0, 1.6, 13.5),
        ],
        stroke,
    ));
    painter.line_segment(
        [
            pt_in(o, side, 16.0, 8.0, 6.2),
            pt_in(o, side, 16.0, 8.0, 9.6),
        ],
        stroke,
    );
    painter.circle_filled(
        pt_in(o, side, 16.0, 8.0, 11.7),
        (side / 16.0_f32).max(0.8_f32),
        color,
    );
}

fn paint_arrow_vertical(painter: &Painter, rect: Rect, up: bool, color: Color32) {
    let Some((o, side)) = fit_square(rect) else {
        return;
    };
    let stroke = Stroke::new((side / 12.0_f32) * 1.4_f32, color);
    painter.line_segment(
        [
            pt_in(o, side, 12.0, 6.0, 2.0),
            pt_in(o, side, 12.0, 6.0, 10.0),
        ],
        stroke,
    );
    if up {
        painter.add(egui::Shape::line(
            vec![
                pt_in(o, side, 12.0, 3.2, 4.8),
                pt_in(o, side, 12.0, 6.0, 2.0),
                pt_in(o, side, 12.0, 8.8, 4.8),
            ],
            stroke,
        ));
    } else {
        painter.add(egui::Shape::line(
            vec![
                pt_in(o, side, 12.0, 3.2, 7.2),
                pt_in(o, side, 12.0, 6.0, 10.0),
                pt_in(o, side, 12.0, 8.8, 7.2),
            ],
            stroke,
        ));
    }
}

fn paint_arrow_right(painter: &Painter, rect: Rect, color: Color32) {
    let Some((o, side)) = fit_square(rect) else {
        return;
    };
    let stroke = Stroke::new((side / 12.0_f32) * 1.35_f32, color);
    painter.line_segment(
        [
            pt_in(o, side, 12.0, 2.0, 6.0),
            pt_in(o, side, 12.0, 10.0, 6.0),
        ],
        stroke,
    );
    painter.add(egui::Shape::line(
        vec![
            pt_in(o, side, 12.0, 7.2, 3.2),
            pt_in(o, side, 12.0, 10.0, 6.0),
            pt_in(o, side, 12.0, 7.2, 8.8),
        ],
        stroke,
    ));
}

fn paint_busy_arc(painter: &Painter, rect: Rect, color: Color32) {
    let Some((o, side)) = fit_square(rect) else {
        return;
    };
    let stroke = Stroke::new((side / 16.0_f32) * 1.6_f32, color);
    let mut pts = Vec::with_capacity(11);
    for i in 0..=10 {
        let a = -std::f32::consts::FRAC_PI_2 + (i as f32 / 10.0_f32) * 4.4_f32;
        let (sn, cs) = a.sin_cos();
        pts.push(pt_in(o, side, 16.0, 8.0 + 5.2 * cs, 8.0 + 5.2 * sn));
    }
    painter.add(egui::Shape::line(pts, stroke));
}

// ---------------------------------------------------------------------------
// Text Layout Helpers (Single-line `'…'` Truncation & Wrapped Banners)
// ---------------------------------------------------------------------------

fn layout_single_line(
    ui: &egui::Ui,
    text: &str,
    font: FontId,
    color: Color32,
    max_width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(
        text.to_string(),
        egui::TextFormat {
            font_id: font,
            color,
            ..Default::default()
        },
    );
    job.wrap = egui::text::TextWrapping {
        max_width: max_width.max(8.0_f32),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    ui.fonts_mut(|f| f.layout_job(job))
}

fn layout_wrapped(
    ui: &egui::Ui,
    text: &str,
    font: FontId,
    color: Color32,
    max_width: f32,
    max_rows: usize,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(
        text.to_string(),
        egui::TextFormat {
            font_id: font,
            color,
            ..Default::default()
        },
    );
    job.wrap = egui::text::TextWrapping {
        max_width: max_width.max(24.0_f32),
        max_rows,
        break_anywhere: false,
        overflow_character: Some('…'),
    };
    ui.fonts_mut(|f| f.layout_job(job))
}

#[derive(Clone, Copy)]
struct PanelColors {
    surface_2: Color32,
    surface_3: Color32,
    border: Color32,
    accent: Color32,
    text: Color32,
    text_dim: Color32,
    warn: Color32,
    danger: Color32,
}

impl PanelColors {
    fn resolve(ui: &egui::Ui) -> Self {
        if ui.visuals().dark_mode {
            let p = &theme::active(ui.ctx()).spec().ui;
            Self {
                surface_2: p.surface_2,
                surface_3: p.surface_3,
                border: p.border,
                accent: p.accent,
                text: p.text,
                text_dim: p.text_dim,
                warn: p.warn,
                danger: p.danger,
            }
        } else {
            let v = ui.visuals();
            Self {
                surface_2: v.widgets.inactive.bg_fill,
                surface_3: v.widgets.hovered.bg_fill,
                border: v.widgets.noninteractive.bg_stroke.color,
                accent: v.hyperlink_color,
                text: v.strong_text_color(),
                text_dim: v.weak_text_color(),
                warn: v.warn_fg_color,
                danger: v.error_fg_color,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ScmChangesPanel Pure UI State & Focus-Scoped Keyboard Navigation
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum NavItem {
    Section {
        id: Id,
        kind: ScmSectionKind,
        expanded: bool,
    },
    Directory {
        id: Id,
        key: String,
        parent_id: Id,
        expanded: bool,
    },
    File {
        id: Id,
        parent_id: Id,
        area: ScmArea,
        path: String,
        old_path: Option<String>,
        deleted: bool,
    },
}

impl NavItem {
    const fn id(&self) -> Id {
        match self {
            Self::Section { id, .. } | Self::Directory { id, .. } | Self::File { id, .. } => *id,
        }
    }
}

fn key_held_after_events(events: &[egui::Event], target: Key) -> bool {
    let mut held = false;
    for ev in events {
        if let egui::Event::Key { key, pressed, .. } = ev
            && *key == target
        {
            held = *pressed;
        }
    }
    held
}

#[derive(Clone, Debug, Default)]
pub struct ScmChangesPanel {
    view_mode: ScmViewMode,
    group_order: ScmGroupOrder,
    filter_expanded: bool,
    focus_filter_next: bool,
    filter_had_focus: bool,
    filter_query: String,
    collapsed_sections: HashSet<String>,
    collapsed_dirs: HashSet<String>,
    commit_draft: String,
    remote_menu_open: bool,
    focused_nav_id: Option<Id>,
    held_nav_key: Option<Key>,
    ctrl_enter_latched: bool,
    filter_escape_latched: bool,
}

impl ScmChangesPanel {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.filter_expanded = false;
        self.focus_filter_next = false;
        self.filter_had_focus = false;
        self.filter_query.clear();
        self.collapsed_sections.clear();
        self.collapsed_dirs.clear();
        self.commit_draft.clear();
        self.remote_menu_open = false;
        self.focused_nav_id = None;
        self.held_nav_key = None;
        self.ctrl_enter_latched = false;
        self.filter_escape_latched = false;
    }

    #[must_use]
    pub const fn view_mode(&self) -> ScmViewMode {
        self.view_mode
    }

    pub fn set_view_mode(&mut self, mode: ScmViewMode) {
        self.view_mode = mode;
    }

    #[must_use]
    pub const fn group_order(&self) -> ScmGroupOrder {
        self.group_order
    }

    pub fn set_group_order(&mut self, order: ScmGroupOrder) {
        self.group_order = order;
    }

    #[must_use]
    pub fn filter_query(&self) -> &str {
        &self.filter_query
    }

    pub fn set_filter_query(&mut self, query: &str) {
        self.filter_query.clear();
        self.filter_query.push_str(query);
        clamp_chars_in_place(&mut self.filter_query, MAX_FILTER_CHARS);
        if !self.filter_query.is_empty() {
            self.filter_expanded = true;
        }
    }

    #[must_use]
    pub const fn is_filter_expanded(&self) -> bool {
        self.filter_expanded
    }

    #[must_use]
    pub fn commit_message(&self) -> &str {
        &self.commit_draft
    }

    pub fn set_commit_message(&mut self, msg: &str) {
        self.commit_draft.clear();
        self.commit_draft.push_str(msg);
        clamp_chars_in_place(&mut self.commit_draft, MAX_COMMIT_MESSAGE_CHARS);
    }

    pub fn clear_commit_message(&mut self) {
        self.commit_draft.clear();
    }

    #[must_use]
    pub fn is_section_collapsed(&self, kind: ScmSectionKind) -> bool {
        self.collapsed_sections.contains(Self::section_key(kind))
    }

    #[must_use]
    pub fn is_dir_collapsed(&self, key: &str) -> bool {
        self.collapsed_dirs.contains(key)
    }

    const fn section_key(kind: ScmSectionKind) -> &'static str {
        match kind {
            ScmSectionKind::Conflicts => "conflicts",
            ScmSectionKind::Staged => "staged",
            ScmSectionKind::Unstaged => "unstaged",
            ScmSectionKind::Untracked => "untracked",
        }
    }

    fn toggle_section(&mut self, kind: ScmSectionKind) {
        let key = Self::section_key(kind);
        if !self.collapsed_sections.remove(key) {
            self.collapsed_sections.insert(key.to_string());
        }
    }

    fn toggle_dir(&mut self, key: &str) {
        if !self.collapsed_dirs.remove(key) {
            self.collapsed_dirs.insert(key.to_string());
        }
    }

    fn prune_stale_collapsed_dirs(&mut self, entries: &[ScmEntry]) {
        if self.collapsed_dirs.is_empty() {
            return;
        }
        let mut live_prefixes: HashSet<String> = HashSet::new();
        for entry in entries {
            let ns = if entry.is_conflict() {
                "conflicts"
            } else {
                entry.area.as_str()
            };
            let segs = split_path_segments(&entry.path);
            let mut acc = String::new();
            for seg in segs.iter().take(segs.len().saturating_sub(1)) {
                if !acc.is_empty() {
                    acc.push('/');
                }
                acc.push_str(seg);
                live_prefixes.insert(format!("dir::{ns}::{acc}"));
            }
        }
        self.collapsed_dirs.retain(|k| live_prefixes.contains(k));
    }

    #[allow(clippy::too_many_arguments)]
    fn render_header_icon_btn(
        ui: &mut egui::Ui,
        id: Id,
        rect: Rect,
        active: bool,
        dot: bool,
        label: &str,
        colors: PanelColors,
        interaction_blocked: bool,
        paint_fn: impl FnOnce(&Painter, Rect, Color32),
    ) -> bool {
        let sense = if interaction_blocked {
            Sense::hover()
        } else {
            Sense::click()
        };
        let resp = ui.interact(rect, id, sense);
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                ui.is_enabled() && !interaction_blocked,
                label,
            )
        });
        let hovered = !interaction_blocked && resp.hovered();
        let focused = ui.is_enabled() && ui.ctx().memory(|m| m.has_focus(id));
        let p = ui.painter().with_clip_rect(rect);
        if active || hovered {
            p.rect_filled(rect, 5.0_f32, colors.surface_3);
        }
        if focused {
            p.rect_stroke(
                rect,
                5.0_f32,
                Stroke::new(1.2_f32, colors.accent),
                StrokeKind::Inside,
            );
        }
        let icon_color = if active || hovered || focused {
            colors.text
        } else {
            colors.text_dim
        };
        let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(14.0_f32));
        paint_fn(&p, icon_rect, icon_color);
        if dot {
            p.circle_filled(
                Pos2::new(rect.max.x - 5.0_f32, rect.min.y + 5.0_f32),
                2.5_f32,
                colors.accent,
            );
        }
        let resp = resp.on_hover_text(label);
        !interaction_blocked && resp.clicked()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_small_action_btn(
        ui: &mut egui::Ui,
        id: Id,
        rect: Rect,
        label: &str,
        danger_hover: bool,
        emphasized: bool,
        colors: PanelColors,
        interaction_blocked: bool,
        paint_fn: impl FnOnce(&Painter, Rect, Color32),
    ) -> bool {
        let sense = if interaction_blocked {
            Sense::hover()
        } else {
            Sense::click()
        };
        let resp = ui.interact(rect, id, sense);
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                ui.is_enabled() && !interaction_blocked,
                label,
            )
        });
        let hovered = !interaction_blocked && resp.hovered();
        let focused = ui.is_enabled() && ui.ctx().memory(|m| m.has_focus(id));
        let p = ui.painter().with_clip_rect(rect);
        if hovered {
            p.rect_filled(rect, 4.0_f32, colors.surface_3);
        }
        if focused {
            p.rect_stroke(
                rect,
                4.0_f32,
                Stroke::new(1.0_f32, colors.accent),
                StrokeKind::Inside,
            );
        }
        let icon_color = if hovered && danger_hover {
            colors.danger
        } else if hovered || focused {
            colors.text
        } else if emphasized {
            colors.text_dim
        } else {
            colors.text_dim.gamma_multiply(0.75_f32)
        };
        let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(13.0_f32));
        paint_fn(&p, icon_rect, icon_color);
        let resp = resp.on_hover_text(label);
        !interaction_blocked && resp.clicked()
    }

    /// Renders the Source Control (`Changes`) panel and returns any user-initiated [`ScmChangesAction`].
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        input: ScmChangesInput<'_>,
    ) -> Option<ScmChangesAction> {
        let panel_id = ui.id().with("tessel-scm-changes-panel");
        let filter_input_id = panel_id.with("filter-input");
        let commit_input_id = panel_id.with("commit-input");
        let colors = PanelColors::resolve(ui);
        let lang = if input.french {
            ScmLang::Fr
        } else {
            ScmLang::En
        };

        let window_focused = ui.ctx().input(|i| i.focused);
        let popup_open = egui::Popup::is_any_open(ui.ctx());
        let interaction_blocked = !ui.is_enabled() || popup_open;

        if !window_focused || interaction_blocked {
            self.held_nav_key = None;
            self.ctrl_enter_latched = false;
            self.filter_escape_latched = false;
        }
        if !input.capabilities.can_remote || input.read_only || interaction_blocked {
            self.remote_menu_open = false;
        }

        if let Some(held) = self.held_nav_key
            && !ui.ctx().input(|i| i.key_down(held))
        {
            self.held_nav_key = None;
        }
        if self.ctrl_enter_latched && !ui.ctx().input(|i| i.key_down(Key::Enter)) {
            self.ctrl_enter_latched = false;
        }
        if self.filter_escape_latched && !ui.ctx().input(|i| i.key_down(Key::Escape)) {
            self.filter_escape_latched = false;
        }

        let mut action: Option<ScmChangesAction> = None;
        let avail_w = ui.available_width().max(120.0_f32);
        ui.spacing_mut().item_spacing.y = 0.0_f32;

        // 1. Header toolbar (`sc-header`)
        if let Some(hdr_act) = self.render_header(
            ui,
            panel_id,
            filter_input_id,
            avail_w,
            &input,
            colors,
            window_focused,
            interaction_blocked,
        ) {
            action = Some(hdr_act);
        }

        // 2. Branch context bar (`sc-branch`) when inside a repository
        if let Some(status) = input.status.as_ref()
            && status.is_repo
        {
            self.render_branch_bar(ui, avail_w, status, input.french, colors);
        }

        // Top separator
        let (_, sep_rect) = ui.allocate_space(Vec2::new(avail_w, 1.0_f32));
        ui.painter().line_segment(
            [sep_rect.left_top(), sep_rect.right_top()],
            Stroke::new(1.0_f32, colors.border),
        );

        // 3. Early states: no project, status error, loading, not a git repo
        if !input.project_selected {
            let msg = if input.french {
                "Cet espace de travail n'a pas de dossier de projet. Choisissez-en un dans le menu de l'espace de travail (Dossier du projet…) pour voir ses modifications ici."
            } else {
                "This workspace has no project folder. Choose one from the workspace menu (Project folder…) to see its changes here."
            };
            Self::render_empty_block(ui, avail_w, None, msg, colors);
            return action;
        }

        if (input.loaded || !input.busy)
            && let Some(err) = input.error.filter(|s| !s.trim().is_empty())
        {
            let err_text = if input.french {
                format!("Impossible de lire le statut git : {err}")
            } else {
                format!("Could not read the git status: {err}")
            };
            if let Some(retry_act) = self.render_error_block(
                ui,
                panel_id.with("status-retry"),
                avail_w,
                &err_text,
                input.french,
                input.busy,
                input.capabilities.can_refresh,
                colors,
                interaction_blocked,
            ) && action.is_none()
            {
                action = Some(retry_act);
            }
            return action;
        }

        if !input.loaded || input.status.is_none() {
            let pending_msg = match (input.busy, input.french) {
                (true, true) => "Lecture du statut git…",
                (true, false) => "Reading git status…",
                (false, true) => "Actualisez pour lire les modifications",
                (false, false) => "Refresh to load changes",
            };
            Self::render_empty_block(ui, avail_w, None, pending_msg, colors);
            return action;
        }

        let Some(status) = input.status else {
            return action;
        };

        if !status.is_repo {
            let not_repo = if input.french {
                "Ce dossier n'est pas dans un dépôt git."
            } else {
                "This folder is not in a git repository."
            };
            Self::render_empty_block(ui, avail_w, None, not_repo, colors);
            return action;
        }

        self.prune_stale_collapsed_dirs(status.entries);

        let unresolved_count = status.entries.iter().filter(|e| e.is_conflict()).count();
        let staged_count = status
            .entries
            .iter()
            .filter(|e| e.area == ScmArea::Staged && !e.is_conflict())
            .count();
        let unstaged_count = status
            .entries
            .iter()
            .filter(|e| e.area == ScmArea::Unstaged && !e.is_conflict())
            .count();
        let stageable_paths: Vec<String> = status
            .entries
            .iter()
            .filter(|e| e.can_stage())
            .map(|e| e.path.clone())
            .collect();
        let bulk_blocked = status.truncated || status.parse_incomplete;

        // 4. Scrollable body: conflict alert, read-only / commit composer, truncation notice, sections
        egui::ScrollArea::vertical()
            .id_salt(panel_id.with("scroll"))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let body_w = ui.available_width().max(100.0_f32);
                ui.spacing_mut().item_spacing.y = 0.0_f32;

                if unresolved_count > 0 {
                    Self::render_conflict_alert(ui, body_w, unresolved_count, input.french, colors);
                }

                if input.read_only {
                    let ro_note = if input.french {
                        "Lecture seule : l'hôte ne vous a pas autorisé à indexer, commiter ou abandonner des modifications."
                    } else {
                        "Read-only: the host has not let you stage, commit or discard changes."
                    };
                    Self::render_notice_banner(ui, body_w, ro_note, false, colors);
                } else if (input.capabilities.can_commit || input.capabilities.can_remote)
                    && let Some(commit_act) = self.render_commit_composer(
                        ui,
                        panel_id,
                        commit_input_id,
                        body_w,
                        &input,
                        &status,
                        staged_count,
                        unstaged_count,
                        &stageable_paths,
                        unresolved_count > 0,
                        bulk_blocked,
                        lang,
                        colors,
                        window_focused,
                        interaction_blocked,
                    )
                    && action.is_none()
                {
                    action = Some(commit_act);
                }

                if status.truncated {
                    let shown = status.entries.len();
                    let total = status.total_entries.max(shown);
                    let total_is_min = status.parse_incomplete || status.total_entries <= shown;
                    let trunc_msg = match (input.french, total_is_min) {
                        (true, true) => format!(
                            "Trop de modifications pour toutes les lister : les {shown} premières sont affichées (au moins {total} détectées). Indexez, abandonnez ou supprimez-les toutes depuis un terminal."
                        ),
                        (true, false) => format!(
                            "Trop de modifications pour toutes les lister : les {shown} premières sur {total} sont affichées. Indexez, abandonnez ou supprimez-les toutes depuis un terminal."
                        ),
                        (false, true) => format!(
                            "Too many changes to list them all: showing the first {shown} (at least {total} detected). Stage, discard, or delete them all from a terminal."
                        ),
                        (false, false) => format!(
                            "Too many changes to list them all: showing the first {shown} of {total}. Stage, discard, or delete them all from a terminal."
                        ),
                    };
                    Self::render_notice_banner(ui, body_w, &trunc_msg, true, colors);
                }
                if status.parse_incomplete {
                    let parse_msg = if input.french {
                        "Certaines entrées du statut git étaient incomplètes ou non UTF-8 : les actions de masse sont désactivées par sécurité."
                    } else {
                        "Some git status records were incomplete or non-UTF-8: bulk actions are disabled for safety."
                    };
                    Self::render_notice_banner(ui, body_w, parse_msg, true, colors);
                }

                let filter_norm = self.filter_query.trim().to_lowercase();
                let has_filter = !filter_norm.is_empty();
                let sections =
                    build_display_sections(status.entries, self.group_order, &filter_norm);

                if status.entries.is_empty() && !has_filter {
                    let (heading, body) = if input.french {
                        (
                            "Aucune modification",
                            "Cet espace de travail est propre : tout est commité.",
                        )
                    } else {
                        (
                            "No changes",
                            "This workspace is clean: everything is committed.",
                        )
                    };
                    Self::render_empty_block(ui, body_w, Some(heading), body, colors);
                    return;
                }

                if has_filter && sections.is_empty() {
                    let q_trimmed = self.filter_query.trim();
                    let (heading, no_match) = if input.french {
                        (
                            "Aucun fichier correspondant",
                            format!("Aucun fichier modifié ne correspond à « {q_trimmed} »"),
                        )
                    } else {
                        (
                            "No matching files",
                            format!("No changed files matching \"{q_trimmed}\""),
                        )
                    };
                    Self::render_empty_block(ui, body_w, Some(heading), &no_match, colors);
                    return;
                }

                let section_rows: Vec<(DisplaySection<'_>, Vec<FlatRow<'_>>)> = sections
                    .into_iter()
                    .map(|sec| {
                        let expanded = !self.is_section_collapsed(sec.kind);
                        let rows = if expanded {
                            build_section_rows(&sec, self.view_mode, &self.collapsed_dirs)
                        } else {
                            Vec::new()
                        };
                        (sec, rows)
                    })
                    .collect();

                // Single focusable list container (`Tab` stops once for the tree, matching `ChangesPanel.vue`)
                let list_id = panel_id.with("list");
                let list_viewport = ui.available_rect_before_wrap();
                let _ = ui.interact(list_viewport, list_id, Sense::focusable_noninteractive());

                // Build deterministic focus navigation map across visible sections & rows
                let nav_items = Self::collect_nav_items(panel_id, &section_rows, &self.collapsed_sections);
                if let Some(cur) = self.focused_nav_id
                    && !nav_items.iter().any(|it| it.id() == cur)
                {
                    self.focused_nav_id = nav_items.first().map(NavItem::id);
                }
                let list_has_focus = ui.is_enabled() && ui.ctx().memory(|m| m.has_focus(list_id));
                if list_has_focus && self.focused_nav_id.is_none() {
                    self.focused_nav_id = nav_items.first().map(NavItem::id);
                }
                if window_focused
                    && !interaction_blocked
                    && list_has_focus
                    && let Some(nav_act) =
                        self.handle_list_keyboard_nav(ui, list_id, &nav_items, input.capabilities)
                    && action.is_none()
                {
                    action = Some(nav_act);
                }

                let allow_bulk = !input.read_only && !bulk_blocked && !has_filter && !input.busy;
                for (sec, rows) in &section_rows {
                    if let Some(sec_act) = self.render_section(
                        ui,
                        panel_id,
                        body_w,
                        sec,
                        rows,
                        &input,
                        lang,
                        allow_bulk,
                        colors,
                        interaction_blocked,
                    ) && action.is_none()
                    {
                        action = Some(sec_act);
                    }
                }
                if !interaction_blocked {
                    ui.ctx().memory_mut(|m| {
                        m.set_focus_lock_filter(
                            list_id,
                            egui::EventFilter {
                                tab: false,
                                horizontal_arrows: true,
                                vertical_arrows: true,
                                escape: false,
                            },
                        );
                    });
                }
                ui.add_space(8.0_f32);
            });

        action
    }

    #[allow(clippy::too_many_arguments)]
    fn render_header(
        &mut self,
        ui: &mut egui::Ui,
        panel_id: Id,
        filter_input_id: Id,
        avail_w: f32,
        input: &ScmChangesInput<'_>,
        colors: PanelColors,
        window_focused: bool,
        interaction_blocked: bool,
    ) -> Option<ScmChangesAction> {
        let mut out = None;
        let pad_x = 8.0_f32;
        let row_h = 34.0_f32;
        let (_, hdr_rect) = ui.allocate_space(Vec2::new(avail_w, row_h));

        if self.filter_expanded {
            let icon_rect = Rect::from_min_size(
                Pos2::new(hdr_rect.min.x + pad_x, hdr_rect.center().y - 7.0_f32),
                Vec2::splat(14.0_f32),
            );
            icons::paint(ui.painter(), icon_rect, Icon::Search, colors.text_dim);

            let clear_rect = Rect::from_min_size(
                Pos2::new(
                    hdr_rect.max.x - pad_x - HEADER_BTN_SIDE,
                    hdr_rect.center().y - HEADER_BTN_SIDE * 0.5_f32,
                ),
                Vec2::splat(HEADER_BTN_SIDE),
            );
            let input_left = icon_rect.max.x + 6.0_f32;
            let input_right = (clear_rect.min.x - 4.0_f32).max(input_left + 32.0_f32);
            let edit_rect = Rect::from_min_max(
                Pos2::new(input_left, hdr_rect.center().y - 13.0_f32),
                Pos2::new(input_right, hdr_rect.center().y + 13.0_f32),
            );

            // Two-step / immediate Escape to collapse filter when filter input has focus
            let other_widget_focused = ui
                .ctx()
                .memory(|m| m.focused().is_some_and(|id| id != filter_input_id));
            let filter_focused_before = !other_widget_focused
                && (self.filter_had_focus || ui.ctx().memory(|m| m.has_focus(filter_input_id)));
            let mut collapse_from_esc = false;
            if window_focused && !interaction_blocked && filter_focused_before {
                ui.input_mut(|i| {
                    let fresh_esc = i.events.iter().any(|ev| {
                        matches!(
                            ev,
                            egui::Event::Key {
                                key: Key::Escape,
                                pressed: true,
                                repeat: false,
                                modifiers,
                                ..
                            } if modifiers.matches_logically(Modifiers::NONE)
                        )
                    });
                    if fresh_esc && !self.filter_escape_latched {
                        self.filter_escape_latched = key_held_after_events(&i.events, Key::Escape);
                        collapse_from_esc = true;
                        let _ = i.consume_key(Modifiers::NONE, Key::Escape);
                    }
                });
            }

            let placeholder = if input.french {
                "Filtrer les fichiers…"
            } else {
                "Filter files…"
            };
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(edit_rect));
            let edit_resp = child.add_sized(
                edit_rect.size(),
                egui::TextEdit::singleline(&mut self.filter_query)
                    .id(filter_input_id)
                    .hint_text(placeholder)
                    .char_limit(MAX_FILTER_CHARS),
            );
            clamp_chars_in_place(&mut self.filter_query, MAX_FILTER_CHARS);

            let requested_focus_this_frame = if self.focus_filter_next && !interaction_blocked {
                edit_resp.request_focus();
                self.focus_filter_next = false;
                true
            } else {
                false
            };
            if collapse_from_esc {
                self.filter_expanded = false;
                self.filter_had_focus = false;
                edit_resp.surrender_focus();
            } else {
                if !interaction_blocked {
                    ui.ctx().memory_mut(|m| {
                        m.set_focus_lock_filter(
                            filter_input_id,
                            egui::EventFilter {
                                tab: false,
                                horizontal_arrows: true,
                                vertical_arrows: false,
                                escape: true,
                            },
                        );
                    });
                }
                self.filter_had_focus = self.filter_expanded
                    && (requested_focus_this_frame
                        || ui.ctx().memory(|m| m.has_focus(filter_input_id)));
            }

            let clear_label = if input.french {
                "Effacer et fermer le filtre"
            } else {
                "Clear and close filter"
            };
            if Self::render_header_icon_btn(
                ui,
                panel_id.with("hdr-filter-clear"),
                clear_rect,
                false,
                false,
                clear_label,
                colors,
                interaction_blocked,
                |p, r, c| icons::paint(p, r, Icon::Close, c),
            ) {
                self.filter_query.clear();
                self.filter_expanded = false;
                self.filter_had_focus = false;
            }
            return out;
        }
        self.filter_had_focus = false;

        let title = if input.french {
            "Contrôle de code source"
        } else {
            "Source Control"
        };
        let mut right_x = hdr_rect.max.x - pad_x;

        if input.capabilities.can_refresh {
            right_x -= HEADER_BTN_SIDE;
            let refresh_rect = Rect::from_min_size(
                Pos2::new(right_x, hdr_rect.center().y - HEADER_BTN_SIDE * 0.5_f32),
                Vec2::splat(HEADER_BTN_SIDE),
            );
            let refresh_label = if input.french {
                "Actualiser"
            } else {
                "Refresh"
            };
            if Self::render_header_icon_btn(
                ui,
                panel_id.with("hdr-refresh"),
                refresh_rect,
                false,
                false,
                refresh_label,
                colors,
                interaction_blocked || input.busy,
                paint_refresh,
            ) {
                out = Some(ScmChangesAction::Refresh);
            }
            right_x -= 2.0_f32;
        }

        right_x -= HEADER_BTN_SIDE;
        let view_rect = Rect::from_min_size(
            Pos2::new(right_x, hdr_rect.center().y - HEADER_BTN_SIDE * 0.5_f32),
            Vec2::splat(HEADER_BTN_SIDE),
        );
        let view_label = match (self.view_mode, input.french) {
            (ScmViewMode::Tree, false) => "View as list",
            (ScmViewMode::Tree, true) => "Afficher en liste",
            (ScmViewMode::List, false) => "View as tree",
            (ScmViewMode::List, true) => "Afficher en arbre",
        };
        let current_mode = self.view_mode;
        if Self::render_header_icon_btn(
            ui,
            panel_id.with("hdr-view-mode"),
            view_rect,
            false,
            false,
            view_label,
            colors,
            interaction_blocked,
            |p, r, c| match current_mode {
                ScmViewMode::Tree => paint_view_list(p, r, c),
                ScmViewMode::List => paint_view_tree(p, r, c),
            },
        ) {
            self.view_mode = match self.view_mode {
                ScmViewMode::Tree => ScmViewMode::List,
                ScmViewMode::List => ScmViewMode::Tree,
            };
        }
        right_x -= 2.0_f32;

        right_x -= HEADER_BTN_SIDE;
        let filter_rect = Rect::from_min_size(
            Pos2::new(right_x, hdr_rect.center().y - HEADER_BTN_SIDE * 0.5_f32),
            Vec2::splat(HEADER_BTN_SIDE),
        );
        let has_active_filter = !self.filter_query.trim().is_empty();
        let filter_label = if has_active_filter {
            if input.french {
                format!("Filtre : {}", self.filter_query.trim())
            } else {
                format!("Filter: {}", self.filter_query.trim())
            }
        } else if input.french {
            "Filtrer les fichiers par nom".to_string()
        } else {
            "Filter files by name".to_string()
        };
        if Self::render_header_icon_btn(
            ui,
            panel_id.with("hdr-filter-toggle"),
            filter_rect,
            has_active_filter,
            has_active_filter,
            &filter_label,
            colors,
            interaction_blocked,
            |p, r, c| icons::paint(p, r, Icon::Search, c),
        ) {
            self.filter_expanded = true;
            self.focus_filter_next = true;
        }

        if input.busy {
            right_x -= 20.0_f32;
            let spin_rect = Rect::from_min_size(
                Pos2::new(right_x + 3.0_f32, hdr_rect.center().y - 6.5_f32),
                Vec2::splat(13.0_f32),
            );
            paint_busy_arc(
                &ui.painter().with_clip_rect(hdr_rect),
                spin_rect,
                colors.text_dim,
            );
        }

        let title_max_w = (right_x - (hdr_rect.min.x + pad_x) - 6.0_f32).max(16.0_f32);
        let title_g = layout_single_line(
            ui,
            title,
            FontId::proportional(11.5_f32),
            colors.text_dim,
            title_max_w,
        );
        let p = ui.painter().with_clip_rect(hdr_rect);
        p.galley(
            Pos2::new(
                hdr_rect.min.x + pad_x,
                hdr_rect.center().y - title_g.size().y * 0.5_f32,
            ),
            title_g,
            colors.text_dim,
        );

        out
    }

    fn render_branch_bar(
        &self,
        ui: &mut egui::Ui,
        avail_w: f32,
        status: &ScmStatusSnapshot<'_>,
        french: bool,
        colors: PanelColors,
    ) {
        let row_h = 26.0_f32;
        let pad_x = 8.0_f32;
        let (branch_id, rect) = ui.allocate_space(Vec2::new(avail_w, row_h));
        let branch_title = status.branch.unwrap_or(if french {
            "HEAD détachée"
        } else {
            "Detached HEAD"
        });
        let aria_label = if french {
            format!("Branche actuelle : {branch_title}")
        } else {
            format!("Current branch: {branch_title}")
        };
        let resp = ui.interact(rect, branch_id, Sense::hover());
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), &aria_label)
        });
        let _ = resp.on_hover_text(&aria_label);

        let p = ui.painter().with_clip_rect(rect);
        let mut right_x = rect.max.x - pad_x;

        if status.upstream.is_some() {
            if status.behind > 0 {
                let txt = status.behind.to_string();
                let g = layout_single_line(
                    ui,
                    &txt,
                    FontId::proportional(11.0_f32),
                    colors.text_dim,
                    48.0_f32,
                );
                let pill_w = 12.0_f32 + g.size().x + 8.0_f32;
                right_x -= pill_w;
                let pill_rect = Rect::from_min_size(
                    Pos2::new(right_x, rect.center().y - 9.0_f32),
                    Vec2::new(pill_w, 18.0_f32),
                );
                p.rect_filled(pill_rect, 4.0_f32, colors.surface_2);
                paint_arrow_vertical(
                    &p,
                    Rect::from_min_size(
                        Pos2::new(pill_rect.min.x + 3.0_f32, rect.center().y - 5.0_f32),
                        Vec2::splat(10.0_f32),
                    ),
                    false,
                    colors.text_dim,
                );
                p.galley(
                    Pos2::new(
                        pill_rect.min.x + 14.0_f32,
                        rect.center().y - g.size().y * 0.5_f32,
                    ),
                    g,
                    colors.text_dim,
                );
                right_x -= 4.0_f32;
            }
            if status.ahead > 0 {
                let txt = status.ahead.to_string();
                let g = layout_single_line(
                    ui,
                    &txt,
                    FontId::proportional(11.0_f32),
                    colors.text_dim,
                    48.0_f32,
                );
                let pill_w = 12.0_f32 + g.size().x + 8.0_f32;
                right_x -= pill_w;
                let pill_rect = Rect::from_min_size(
                    Pos2::new(right_x, rect.center().y - 9.0_f32),
                    Vec2::new(pill_w, 18.0_f32),
                );
                p.rect_filled(pill_rect, 4.0_f32, colors.surface_2);
                paint_arrow_vertical(
                    &p,
                    Rect::from_min_size(
                        Pos2::new(pill_rect.min.x + 3.0_f32, rect.center().y - 5.0_f32),
                        Vec2::splat(10.0_f32),
                    ),
                    true,
                    colors.text_dim,
                );
                p.galley(
                    Pos2::new(
                        pill_rect.min.x + 14.0_f32,
                        rect.center().y - g.size().y * 0.5_f32,
                    ),
                    g,
                    colors.text_dim,
                );
                right_x -= 4.0_f32;
            }
        } else if status.branch.is_some() {
            let unpub = if french {
                "non publiée"
            } else {
                "unpublished"
            };
            let g = layout_single_line(
                ui,
                unpub,
                FontId::proportional(10.5_f32),
                colors.text_dim,
                84.0_f32,
            );
            let pill_w = g.size().x + 10.0_f32;
            right_x -= pill_w;
            let pill_rect = Rect::from_min_size(
                Pos2::new(right_x, rect.center().y - 8.5_f32),
                Vec2::new(pill_w, 17.0_f32),
            );
            p.rect_filled(pill_rect, 4.0_f32, colors.surface_2);
            p.galley(
                Pos2::new(
                    pill_rect.min.x + 5.0_f32,
                    rect.center().y - g.size().y * 0.5_f32,
                ),
                g,
                colors.text_dim,
            );
            right_x -= 4.0_f32;
        }

        let icon_rect = Rect::from_min_size(
            Pos2::new(rect.min.x + pad_x, rect.center().y - 6.5_f32),
            Vec2::splat(13.0_f32),
        );
        icons::paint(&p, icon_rect, Icon::GitBranch, colors.text_dim);

        let mut left_x = icon_rect.max.x + 5.0_f32;
        let avail_text_w = (right_x - left_x).max(20.0_f32);
        let branch_color = if status.branch.is_some() {
            colors.text
        } else {
            colors.warn
        };
        let branch_max = if status.upstream.is_some() && avail_text_w > 120.0_f32 {
            avail_text_w * 0.55_f32
        } else {
            avail_text_w
        };
        let bg = layout_single_line(
            ui,
            branch_title,
            FontId::proportional(12.0_f32),
            branch_color,
            branch_max,
        );
        let bw = bg.size().x;
        p.galley(
            Pos2::new(left_x, rect.center().y - bg.size().y * 0.5_f32),
            bg,
            branch_color,
        );
        left_x += bw + 5.0_f32;

        if let Some(up) = status.upstream
            && right_x - left_x >= 36.0_f32
        {
            let arrow_rect = Rect::from_min_size(
                Pos2::new(left_x, rect.center().y - 5.0_f32),
                Vec2::splat(10.0_f32),
            );
            paint_arrow_right(&p, arrow_rect, colors.text_dim);
            left_x += 13.0_f32;
            let up_g = layout_single_line(
                ui,
                up,
                FontId::proportional(11.0_f32),
                colors.text_dim,
                (right_x - left_x).max(16.0_f32),
            );
            p.galley(
                Pos2::new(left_x, rect.center().y - up_g.size().y * 0.5_f32),
                up_g,
                colors.text_dim,
            );
        }
    }

    fn render_conflict_alert(
        ui: &mut egui::Ui,
        avail_w: f32,
        unresolved_count: usize,
        french: bool,
        colors: PanelColors,
    ) {
        let pad_x = 8.0_f32;
        let inner_w = (avail_w - pad_x * 2.0_f32 - 20.0_f32).max(40.0_f32);
        let title = if french { "Conflits" } else { "Conflicts" };
        let count_line = if french {
            if unresolved_count == 1 {
                format!("{title} : 1 non résolu")
            } else {
                format!("{title} : {unresolved_count} non résolus")
            }
        } else {
            format!("{title}: {unresolved_count} unresolved")
        };
        let hint = if french {
            "Les fichiers résolus repassent dans les modifications normales une fois sortis de l'état de conflit actif."
        } else {
            "Resolved files move back to normal changes after they leave the live conflict state."
        };

        let count_g = layout_wrapped(
            ui,
            &count_line,
            FontId::proportional(12.0_f32),
            colors.warn,
            inner_w - 18.0_f32,
            2,
        );
        let hint_g = layout_wrapped(
            ui,
            hint,
            FontId::proportional(11.0_f32),
            colors.text_dim,
            inner_w,
            3,
        );
        let card_h = 10.0_f32 + count_g.size().y + 4.0_f32 + hint_g.size().y + 10.0_f32;
        ui.add_space(8.0_f32);
        let (_, outer) = ui.allocate_space(Vec2::new(avail_w, card_h));
        let card = Rect::from_min_max(
            Pos2::new(outer.min.x + pad_x, outer.min.y),
            Pos2::new(outer.max.x - pad_x, outer.max.y),
        );
        let p = ui.painter().with_clip_rect(card);
        p.rect(
            card,
            6.0_f32,
            colors.warn.gamma_multiply(0.10_f32),
            Stroke::new(1.0_f32, colors.warn.gamma_multiply(0.55_f32)),
            StrokeKind::Inside,
        );
        paint_alert_triangle(
            &p,
            Rect::from_min_size(
                Pos2::new(card.min.x + 9.0_f32, card.min.y + 10.0_f32),
                Vec2::splat(13.0_f32),
            ),
            colors.warn,
        );
        let text_y = card.min.y + 10.0_f32 + count_g.size().y + 4.0_f32;
        p.galley(
            Pos2::new(card.min.x + 26.0_f32, card.min.y + 9.0_f32),
            count_g,
            colors.warn,
        );
        p.galley(
            Pos2::new(card.min.x + 10.0_f32, text_y),
            hint_g,
            colors.text_dim,
        );
    }

    fn render_notice_banner(
        ui: &mut egui::Ui,
        avail_w: f32,
        text: &str,
        warning: bool,
        colors: PanelColors,
    ) {
        let pad_x = 8.0_f32;
        let inner_w = (avail_w - pad_x * 2.0_f32 - 16.0_f32).max(40.0_f32);
        let fg = if warning {
            colors.warn
        } else {
            colors.text_dim
        };
        let galley = layout_wrapped(ui, text, FontId::proportional(11.5_f32), fg, inner_w, 4);
        let box_h = galley.size().y + 14.0_f32;
        ui.add_space(6.0_f32);
        let (_, outer) = ui.allocate_space(Vec2::new(avail_w, box_h));
        let box_rect = Rect::from_min_max(
            Pos2::new(outer.min.x + pad_x, outer.min.y),
            Pos2::new(outer.max.x - pad_x, outer.max.y),
        );
        let bg = if warning {
            colors.warn.gamma_multiply(0.09_f32)
        } else {
            colors.surface_2
        };
        let border = if warning {
            colors.warn.gamma_multiply(0.45_f32)
        } else {
            colors.border
        };
        let p = ui.painter().with_clip_rect(box_rect);
        p.rect(
            box_rect,
            6.0_f32,
            bg,
            Stroke::new(1.0_f32, border),
            StrokeKind::Inside,
        );
        p.galley(
            Pos2::new(box_rect.min.x + 8.0_f32, box_rect.min.y + 7.0_f32),
            galley,
            fg,
        );
    }

    fn render_empty_block(
        ui: &mut egui::Ui,
        avail_w: f32,
        heading: Option<&str>,
        body: &str,
        colors: PanelColors,
    ) {
        let pad_x = 14.0_f32;
        let inner_w = (avail_w - pad_x * 2.0_f32).max(40.0_f32);
        ui.add_space(16.0_f32);
        let head_g = heading.map(|h| {
            layout_wrapped(
                ui,
                h,
                FontId::proportional(13.0_f32),
                colors.text,
                inner_w,
                2,
            )
        });
        let body_g = layout_wrapped(
            ui,
            body,
            FontId::proportional(12.0_f32),
            colors.text_dim,
            inner_w,
            5,
        );
        let total_h =
            head_g.as_ref().map_or(0.0_f32, |g| g.size().y + 4.0_f32) + body_g.size().y + 4.0_f32;
        let (_, rect) = ui.allocate_space(Vec2::new(avail_w, total_h));
        let p = ui.painter().with_clip_rect(rect);
        let mut y = rect.min.y;
        if let Some(hg) = head_g {
            let hy = hg.size().y;
            p.galley(Pos2::new(rect.min.x + pad_x, y), hg, colors.text);
            y += hy + 4.0_f32;
        }
        p.galley(Pos2::new(rect.min.x + pad_x, y), body_g, colors.text_dim);
    }

    #[allow(clippy::too_many_arguments)]
    fn render_error_block(
        &self,
        ui: &mut egui::Ui,
        retry_id: Id,
        avail_w: f32,
        text: &str,
        french: bool,
        busy: bool,
        can_refresh: bool,
        colors: PanelColors,
        interaction_blocked: bool,
    ) -> Option<ScmChangesAction> {
        let pad_x = 12.0_f32;
        let inner_w = (avail_w - pad_x * 2.0_f32 - 16.0_f32).max(40.0_f32);
        let g = layout_wrapped(
            ui,
            text,
            FontId::proportional(12.0_f32),
            colors.danger,
            inner_w,
            4,
        );
        let btn_h = if can_refresh { 30.0_f32 } else { 0.0_f32 };
        let card_h = 10.0_f32 + g.size().y + btn_h + 10.0_f32;
        ui.add_space(10.0_f32);
        let (_, outer) = ui.allocate_space(Vec2::new(avail_w, card_h));
        let card = Rect::from_min_max(
            Pos2::new(outer.min.x + pad_x, outer.min.y),
            Pos2::new(outer.max.x - pad_x, outer.max.y),
        );
        let p = ui.painter().with_clip_rect(card);
        p.rect(
            card,
            6.0_f32,
            colors.danger.gamma_multiply(0.10_f32),
            Stroke::new(1.0_f32, colors.danger.gamma_multiply(0.5_f32)),
            StrokeKind::Inside,
        );
        let btn_top = card.min.y + 10.0_f32 + g.size().y + 6.0_f32;
        p.galley(
            Pos2::new(card.min.x + 8.0_f32, card.min.y + 10.0_f32),
            g,
            colors.danger,
        );

        if !can_refresh {
            return None;
        }
        let retry_label = if french { "Réessayer" } else { "Retry" };
        let rg = layout_single_line(
            ui,
            retry_label,
            FontId::proportional(11.5_f32),
            colors.text,
            120.0_f32,
        );
        let btn_w = (rg.size().x + 18.0_f32).min(card.width() - 16.0_f32);
        let btn_rect = Rect::from_min_size(
            Pos2::new(card.min.x + 8.0_f32, btn_top),
            Vec2::new(btn_w, 24.0_f32),
        );
        let disabled = interaction_blocked || busy;
        let resp = ui.interact(
            btn_rect,
            retry_id,
            if disabled {
                Sense::hover()
            } else {
                Sense::click()
            },
        );
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, !disabled, retry_label)
        });
        let bg = if !disabled && resp.hovered() {
            colors.surface_3
        } else {
            colors.surface_2
        };
        let bp = ui.painter().with_clip_rect(btn_rect);
        bp.rect(
            btn_rect,
            5.0_f32,
            bg,
            Stroke::new(1.0_f32, colors.border),
            StrokeKind::Inside,
        );
        bp.galley(
            Pos2::new(
                btn_rect.center().x - rg.size().x * 0.5_f32,
                btn_rect.center().y - rg.size().y * 0.5_f32,
            ),
            rg,
            if disabled {
                colors.text_dim
            } else {
                colors.text
            },
        );
        let resp = resp.on_hover_text(retry_label);
        (!disabled && resp.clicked()).then_some(ScmChangesAction::Refresh)
    }

    #[allow(clippy::too_many_arguments)]
    fn render_commit_composer(
        &mut self,
        ui: &mut egui::Ui,
        panel_id: Id,
        commit_input_id: Id,
        avail_w: f32,
        input: &ScmChangesInput<'_>,
        status: &ScmStatusSnapshot<'_>,
        staged_count: usize,
        unstaged_count: usize,
        stageable_paths: &[String],
        has_conflicts: bool,
        bulk_blocked: bool,
        lang: ScmLang,
        colors: PanelColors,
        window_focused: bool,
        interaction_blocked: bool,
    ) -> Option<ScmChangesAction> {
        let mut out = None;
        let pad_x = 8.0_f32;
        let inner_w = (avail_w - pad_x * 2.0_f32).max(60.0_f32);
        ui.add_space(8.0_f32);

        if input.capabilities.can_commit {
            let trimmed_non_empty = !self.commit_draft.trim().is_empty();
            let can_ctrl_enter = window_focused
                && !interaction_blocked
                && !input.busy
                && !has_conflicts
                && staged_count > 0
                && trimmed_non_empty;

            let commit_focused_before = ui.ctx().memory(|m| m.has_focus(commit_input_id));
            if commit_focused_before && window_focused && !interaction_blocked {
                ui.input_mut(|i| {
                    let mut saw_fresh_ctrl_enter = false;
                    let mut saw_any_ctrl_enter = false;
                    for ev in &i.events {
                        if let egui::Event::Key {
                            key: Key::Enter,
                            pressed: true,
                            repeat,
                            modifiers,
                            ..
                        } = ev
                            && (modifiers.ctrl || modifiers.command)
                        {
                            saw_any_ctrl_enter = true;
                            if !*repeat && !self.ctrl_enter_latched {
                                saw_fresh_ctrl_enter = true;
                            }
                        }
                    }
                    if saw_any_ctrl_enter {
                        let _ = i.consume_key(Modifiers::CTRL, Key::Enter);
                        let _ = i.consume_key(Modifiers::COMMAND, Key::Enter);
                    }
                    if saw_fresh_ctrl_enter {
                        self.ctrl_enter_latched = key_held_after_events(&i.events, Key::Enter);
                        if can_ctrl_enter {
                            out = Some(ScmChangesAction::Commit {
                                message: self.commit_draft.clone(),
                            });
                        }
                    }
                });
            }

            let rows = commit_textarea_rows(&self.commit_draft);
            let placeholder = "Message";
            let aria_msg = if input.french {
                "Message de commit"
            } else {
                "Commit message"
            };
            let box_h = (rows as f32) * 18.0_f32 + 12.0_f32;
            let (_, outer_edit) = ui.allocate_space(Vec2::new(avail_w, box_h));
            let edit_rect = Rect::from_min_max(
                Pos2::new(outer_edit.min.x + pad_x, outer_edit.min.y),
                Pos2::new(outer_edit.max.x - pad_x, outer_edit.max.y),
            );
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(edit_rect));
            let te_resp = child.add_enabled(
                !input.busy && !interaction_blocked,
                egui::TextEdit::multiline(&mut self.commit_draft)
                    .id(commit_input_id)
                    .desired_rows(rows)
                    .desired_width(inner_w)
                    .hint_text(placeholder)
                    .char_limit(MAX_COMMIT_MESSAGE_CHARS),
            );
            clamp_chars_in_place(&mut self.commit_draft, MAX_COMMIT_MESSAGE_CHARS);
            let _ = te_resp.on_hover_text(aria_msg);
            ui.add_space(6.0_f32);
        }

        let primary = resolve_primary_action(
            staged_count,
            unstaged_count,
            stageable_paths.len(),
            has_conflicts,
            !self.commit_draft.trim().is_empty(),
            input.busy,
            bulk_blocked,
            status.total_entries,
            status,
            input.capabilities,
            lang,
            input.french,
        );

        let btn_h = 28.0_f32;
        let show_remote_chevron = input.capabilities.can_remote && !input.read_only;
        let chev_w = if show_remote_chevron {
            26.0_f32
        } else {
            0.0_f32
        };
        let (_, split_outer) = ui.allocate_space(Vec2::new(avail_w, btn_h));
        let split_rect = Rect::from_min_max(
            Pos2::new(split_outer.min.x + pad_x, split_outer.min.y),
            Pos2::new(split_outer.max.x - pad_x, split_outer.max.y),
        );
        let prim_w = if show_remote_chevron {
            (split_rect.width() - chev_w - 1.0_f32).max(32.0_f32)
        } else {
            split_rect.width()
        };
        let prim_rect = Rect::from_min_size(split_rect.min, Vec2::new(prim_w, btn_h));

        let prim_enabled = primary.enabled && !interaction_blocked;
        let prim_id = panel_id.with("primary-action");
        let prim_resp = ui.interact(
            prim_rect,
            prim_id,
            if prim_enabled {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        prim_resp.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                ui.is_enabled() && prim_enabled,
                &primary.label,
            )
        });
        let prim_hovered = prim_enabled && prim_resp.hovered();
        let prim_focused = ui.is_enabled() && ui.ctx().memory(|m| m.has_focus(prim_id));
        let prim_bg = if prim_enabled {
            if prim_hovered {
                colors.accent.gamma_multiply(0.90_f32)
            } else {
                colors.accent
            }
        } else {
            colors.surface_2
        };
        let prim_fg = if prim_enabled {
            Color32::WHITE
        } else {
            colors.text_dim
        };
        let pp = ui.painter().with_clip_rect(prim_rect);
        pp.rect(
            prim_rect,
            5.0_f32,
            prim_bg,
            Stroke::new(
                if prim_focused { 1.5_f32 } else { 1.0_f32 },
                if prim_focused {
                    Color32::WHITE
                } else if prim_enabled {
                    colors.accent
                } else {
                    colors.border
                },
            ),
            StrokeKind::Inside,
        );
        let pg = layout_single_line(
            ui,
            &primary.label,
            FontId::proportional(12.0_f32),
            prim_fg,
            (prim_rect.width() - 12.0_f32).max(16.0_f32),
        );
        pp.galley(
            Pos2::new(
                prim_rect.center().x - pg.size().x * 0.5_f32,
                prim_rect.center().y - pg.size().y * 0.5_f32,
            ),
            pg,
            prim_fg,
        );
        let prim_resp = prim_resp.on_hover_text(&primary.tooltip);
        if prim_enabled && prim_resp.clicked() && out.is_none() {
            out = match primary.kind {
                PrimaryKind::Commit => Some(ScmChangesAction::Commit {
                    message: self.commit_draft.clone(),
                }),
                PrimaryKind::StageAll => Some(ScmChangesAction::Stage(stageable_paths.to_vec())),
                PrimaryKind::Remote(op) => Some(ScmChangesAction::Remote(op)),
            };
        }

        if show_remote_chevron {
            let chev_rect = Rect::from_min_size(
                Pos2::new(prim_rect.max.x + 1.0_f32, split_rect.min.y),
                Vec2::new(chev_w, btn_h),
            );
            let chev_label = if input.french {
                "Autres actions de commit et de remote"
            } else {
                "More commit and remote actions"
            };
            let chev_id = panel_id.with("remote-chevron");
            let chev_disabled = interaction_blocked || input.busy;
            let chev_resp = ui.interact(
                chev_rect,
                chev_id,
                if chev_disabled {
                    Sense::hover()
                } else {
                    Sense::click()
                },
            );
            chev_resp.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, !chev_disabled, chev_label)
            });
            let cp = ui.painter().with_clip_rect(chev_rect);
            cp.rect(
                chev_rect,
                5.0_f32,
                if !chev_disabled && (chev_resp.hovered() || self.remote_menu_open) {
                    colors.surface_3
                } else {
                    colors.surface_2
                },
                Stroke::new(1.0_f32, colors.border),
                StrokeKind::Inside,
            );
            icons::paint(
                &cp,
                Rect::from_center_size(chev_rect.center(), Vec2::splat(12.0_f32)),
                Icon::ChevronDown,
                if chev_disabled {
                    colors.text_dim
                } else {
                    colors.text
                },
            );
            let chev_resp = chev_resp.on_hover_text(chev_label);
            if !chev_disabled && chev_resp.clicked() {
                self.remote_menu_open = !self.remote_menu_open;
            }

            if self.remote_menu_open {
                let ops: [(ScmRemoteOp, &str, bool); 6] = [
                    (ScmRemoteOp::Push, "Push", status.branch.is_some()),
                    (
                        ScmRemoteOp::Pull,
                        "Pull",
                        status.branch.is_some() && status.upstream.is_some(),
                    ),
                    (
                        ScmRemoteOp::FastForward,
                        "Fast-forward",
                        status.branch.is_some() && status.upstream.is_some(),
                    ),
                    (
                        ScmRemoteOp::Sync,
                        "Sync",
                        status.branch.is_some() && status.upstream.is_some(),
                    ),
                    (ScmRemoteOp::Fetch, "Fetch", true),
                    (
                        ScmRemoteOp::Publish,
                        "Publish Branch",
                        status.branch.is_some() && status.upstream.is_none(),
                    ),
                ];
                ui.add_space(4.0_f32);
                let menu_h = (ops.len() as f32) * 24.0_f32 + 6.0_f32;
                let (_, menu_outer) = ui.allocate_space(Vec2::new(avail_w, menu_h));
                let menu_rect = Rect::from_min_max(
                    Pos2::new(menu_outer.min.x + pad_x, menu_outer.min.y),
                    Pos2::new(menu_outer.max.x - pad_x, menu_outer.max.y),
                );
                let mp = ui.painter().with_clip_rect(menu_rect);
                mp.rect(
                    menu_rect,
                    6.0_f32,
                    colors.surface_2,
                    Stroke::new(1.0_f32, colors.border),
                    StrokeKind::Inside,
                );
                for (idx, (op, en_lbl, op_ok)) in ops.into_iter().enumerate() {
                    let item_rect = Rect::from_min_size(
                        Pos2::new(
                            menu_rect.min.x + 3.0_f32,
                            menu_rect.min.y + 3.0_f32 + (idx as f32) * 24.0_f32,
                        ),
                        Vec2::new((menu_rect.width() - 6.0_f32).max(20.0_f32), 24.0_f32),
                    );
                    let lbl = scm_text(en_lbl, lang);
                    let item_enabled = op_ok && !interaction_blocked && !input.busy;
                    let item_resp = ui.interact(
                        item_rect,
                        panel_id.with(("remote-op", idx)),
                        if item_enabled {
                            Sense::click()
                        } else {
                            Sense::hover()
                        },
                    );
                    item_resp.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, item_enabled, &lbl)
                    });
                    let ip = ui.painter().with_clip_rect(item_rect);
                    if item_enabled && item_resp.hovered() {
                        ip.rect_filled(item_rect, 4.0_f32, colors.surface_3);
                    }
                    let ig = layout_single_line(
                        ui,
                        &lbl,
                        FontId::proportional(12.0_f32),
                        if item_enabled {
                            colors.text
                        } else {
                            colors.text_dim
                        },
                        (item_rect.width() - 16.0_f32).max(16.0_f32),
                    );
                    ip.galley(
                        Pos2::new(
                            item_rect.min.x + 8.0_f32,
                            item_rect.center().y - ig.size().y * 0.5_f32,
                        ),
                        ig,
                        if item_enabled {
                            colors.text
                        } else {
                            colors.text_dim
                        },
                    );
                    if item_enabled && item_resp.clicked() && out.is_none() {
                        self.remote_menu_open = false;
                        out = Some(ScmChangesAction::Remote(op));
                    }
                }
            }
        }

        ui.add_space(6.0_f32);
        out
    }

    fn collect_nav_items(
        panel_id: Id,
        section_rows: &[(DisplaySection<'_>, Vec<FlatRow<'_>>)],
        collapsed_sections: &HashSet<String>,
    ) -> Vec<NavItem> {
        let mut items = Vec::new();
        for (sec, rows) in section_rows {
            let sec_key = Self::section_key(sec.kind);
            let sec_id = panel_id.with(("sec", sec_key));
            let sec_expanded = !collapsed_sections.contains(sec_key);
            items.push(NavItem::Section {
                id: sec_id,
                kind: sec.kind,
                expanded: sec_expanded,
            });
            if !sec_expanded {
                continue;
            }
            for row in rows {
                match row {
                    FlatRow::Directory {
                        key,
                        parent_key,
                        expanded,
                        ..
                    } => {
                        let parent_id = parent_key
                            .as_deref()
                            .map_or(sec_id, |pk| panel_id.with(("row", pk)));
                        items.push(NavItem::Directory {
                            id: panel_id.with(("row", key.as_str())),
                            key: key.clone(),
                            parent_id,
                            expanded: *expanded,
                        });
                    }
                    FlatRow::File {
                        key,
                        parent_key,
                        entry,
                        ..
                    } => {
                        let parent_id = parent_key
                            .as_deref()
                            .map_or(sec_id, |pk| panel_id.with(("row", pk)));
                        items.push(NavItem::File {
                            id: panel_id.with(("row", key.as_str())),
                            parent_id,
                            area: entry.area,
                            path: entry.path.clone(),
                            old_path: entry.old_path.clone(),
                            deleted: entry.status == ScmFileStatus::Deleted,
                        });
                    }
                }
            }
        }
        items
    }

    fn handle_list_keyboard_nav(
        &mut self,
        ui: &mut egui::Ui,
        list_id: Id,
        nav_items: &[NavItem],
        caps: ScmCapabilities,
    ) -> Option<ScmChangesAction> {
        let focused_id = self
            .focused_nav_id
            .or_else(|| nav_items.first().map(NavItem::id))?;
        let idx = nav_items.iter().position(|it| it.id() == focused_id)?;

        let held_before = self.held_nav_key;
        let mut pressed_key: Option<Key> = None;
        let mut still_held = false;
        ui.input_mut(|i| {
            for ev in &i.events {
                if let egui::Event::Key {
                    key:
                        k @ (Key::ArrowUp
                        | Key::ArrowDown
                        | Key::ArrowLeft
                        | Key::ArrowRight
                        | Key::Enter
                        | Key::Space),
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } = ev
                    && modifiers.matches_logically(Modifiers::NONE)
                    && held_before != Some(*k)
                {
                    pressed_key = Some(*k);
                    break;
                }
            }
            if let Some(k) = pressed_key {
                still_held = key_held_after_events(&i.events, k);
            }
            for k in [
                Key::ArrowUp,
                Key::ArrowDown,
                Key::ArrowLeft,
                Key::ArrowRight,
                Key::Enter,
                Key::Space,
            ] {
                let _ = i.consume_key(Modifiers::NONE, k);
            }
        });

        let key = pressed_key?;
        self.held_nav_key = still_held.then_some(key);
        ui.ctx().memory_mut(|m| m.request_focus(list_id));

        match key {
            Key::ArrowDown => {
                if idx + 1 < nav_items.len() {
                    self.focused_nav_id = Some(nav_items[idx + 1].id());
                }
                None
            }
            Key::ArrowUp => {
                if idx > 0 {
                    self.focused_nav_id = Some(nav_items[idx - 1].id());
                }
                None
            }
            Key::ArrowLeft => {
                match &nav_items[idx] {
                    NavItem::Section { kind, expanded, .. } => {
                        if *expanded {
                            self.toggle_section(*kind);
                        }
                    }
                    NavItem::Directory {
                        key,
                        parent_id,
                        expanded,
                        ..
                    } => {
                        if *expanded {
                            self.toggle_dir(key);
                        } else {
                            self.focused_nav_id = Some(*parent_id);
                        }
                    }
                    NavItem::File { parent_id, .. } => {
                        self.focused_nav_id = Some(*parent_id);
                    }
                }
                None
            }
            Key::ArrowRight => {
                match &nav_items[idx] {
                    NavItem::Section {
                        id, kind, expanded, ..
                    } => {
                        if !*expanded {
                            self.toggle_section(*kind);
                        } else if let Some(child) = nav_items.get(idx + 1)
                            && matches!(
                                child,
                                NavItem::Directory { parent_id, .. } | NavItem::File { parent_id, .. }
                                if *parent_id == *id
                            )
                        {
                            self.focused_nav_id = Some(child.id());
                        }
                    }
                    NavItem::Directory {
                        id, key, expanded, ..
                    } => {
                        if !*expanded {
                            self.toggle_dir(key);
                        } else if let Some(child) = nav_items.get(idx + 1)
                            && matches!(
                                child,
                                NavItem::Directory { parent_id, .. } | NavItem::File { parent_id, .. }
                                if *parent_id == *id
                            )
                        {
                            self.focused_nav_id = Some(child.id());
                        }
                    }
                    NavItem::File { .. } => {}
                }
                None
            }
            Key::Enter | Key::Space => match &nav_items[idx] {
                NavItem::Section { kind, .. } => {
                    self.toggle_section(*kind);
                    None
                }
                NavItem::Directory { key, .. } => {
                    self.toggle_dir(key);
                    None
                }
                NavItem::File {
                    area,
                    path,
                    old_path,
                    deleted,
                    ..
                } => {
                    if caps.can_open_diff {
                        Some(ScmChangesAction::OpenDiff {
                            area: *area,
                            path: path.clone(),
                            old_path: old_path.clone(),
                            pinned: false,
                        })
                    } else if caps.can_open_file && !*deleted {
                        Some(ScmChangesAction::OpenFile(path.clone()))
                    } else {
                        None
                    }
                }
            },
            _ => None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_section(
        &mut self,
        ui: &mut egui::Ui,
        panel_id: Id,
        avail_w: f32,
        sec: &DisplaySection<'_>,
        rows: &[FlatRow<'_>],
        input: &ScmChangesInput<'_>,
        lang: ScmLang,
        allow_bulk: bool,
        colors: PanelColors,
        interaction_blocked: bool,
    ) -> Option<ScmChangesAction> {
        let mut out = None;
        let sec_key = Self::section_key(sec.kind);
        let sec_id = panel_id.with(("sec", sec_key));
        let expanded = !self.is_section_collapsed(sec.kind);

        let (_, hdr_rect) = ui.allocate_space(Vec2::new(avail_w, SECTION_ROW_H));
        let mut actions_right = hdr_rect.max.x - 6.0_f32;

        let stage_all: Vec<String> = sec
            .items
            .iter()
            .filter(|e| e.can_stage())
            .map(|e| e.path.clone())
            .collect();
        let unstage_all: Vec<String> = sec
            .items
            .iter()
            .filter(|e| e.can_unstage())
            .map(|e| e.path.clone())
            .collect();
        let discard_all: Vec<String> = if matches!(sec.area, ScmArea::Unstaged | ScmArea::Untracked)
            && sec.kind != ScmSectionKind::Conflicts
        {
            sec.items
                .iter()
                .filter(|e| e.area == sec.area && e.can_discard())
                .map(|e| e.path.clone())
                .collect()
        } else {
            Vec::new()
        };

        let show_unstage_all =
            allow_bulk && input.capabilities.can_unstage && !unstage_all.is_empty();
        let show_stage_all = allow_bulk && input.capabilities.can_stage && !stage_all.is_empty();
        let show_discard_all =
            allow_bulk && input.capabilities.can_discard && !discard_all.is_empty();

        if show_unstage_all {
            actions_right -= ROW_ACTION_SIDE;
            let btn_rect = Rect::from_min_size(
                Pos2::new(
                    actions_right,
                    hdr_rect.center().y - ROW_ACTION_SIDE * 0.5_f32,
                ),
                Vec2::splat(ROW_ACTION_SIDE),
            );
            let lbl = if input.french {
                "Tout désindexer"
            } else {
                "Unstage all"
            };
            if Self::render_small_action_btn(
                ui,
                panel_id.with(("sec-unstage-all", sec_key)),
                btn_rect,
                lbl,
                false,
                true,
                colors,
                interaction_blocked,
                paint_minus,
            ) {
                out = Some(ScmChangesAction::Unstage(unstage_all));
            }
            actions_right -= 2.0_f32;
        }
        if show_stage_all {
            actions_right -= ROW_ACTION_SIDE;
            let btn_rect = Rect::from_min_size(
                Pos2::new(
                    actions_right,
                    hdr_rect.center().y - ROW_ACTION_SIDE * 0.5_f32,
                ),
                Vec2::splat(ROW_ACTION_SIDE),
            );
            let lbl = if input.french {
                "Tout indexer"
            } else {
                "Stage all"
            };
            if Self::render_small_action_btn(
                ui,
                panel_id.with(("sec-stage-all", sec_key)),
                btn_rect,
                lbl,
                false,
                true,
                colors,
                interaction_blocked,
                |p, r, c| icons::paint(p, r, Icon::Plus, c),
            ) {
                out = Some(ScmChangesAction::Stage(stage_all));
            }
            actions_right -= 2.0_f32;
        }
        if show_discard_all {
            actions_right -= ROW_ACTION_SIDE;
            let btn_rect = Rect::from_min_size(
                Pos2::new(
                    actions_right,
                    hdr_rect.center().y - ROW_ACTION_SIDE * 0.5_f32,
                ),
                Vec2::splat(ROW_ACTION_SIDE),
            );
            let is_untracked = sec.area == ScmArea::Untracked;
            let lbl = match (is_untracked, input.french) {
                (true, false) => "Delete all untracked",
                (true, true) => "Supprimer tous les non suivis",
                (false, false) => "Discard all",
                (false, true) => "Tout abandonner",
            };
            if Self::render_small_action_btn(
                ui,
                panel_id.with(("sec-discard-all", sec_key)),
                btn_rect,
                lbl,
                true,
                true,
                colors,
                interaction_blocked,
                |p, r, c| {
                    if is_untracked {
                        paint_trash(p, r, c);
                    } else {
                        paint_undo(p, r, c);
                    }
                },
            ) {
                out = Some(ScmChangesAction::RequestDiscard {
                    area: sec.area,
                    paths: discard_all,
                });
            }
            actions_right -= 2.0_f32;
        }

        let toggle_rect = Rect::from_min_max(
            hdr_rect.min,
            Pos2::new(actions_right.max(hdr_rect.min.x + 24.0_f32), hdr_rect.max.y),
        );
        let sec_label = sec.kind.label(lang);
        let toggle_resp = ui.interact(
            toggle_rect,
            sec_id,
            if interaction_blocked {
                Sense::hover()
            } else {
                Sense::click()
            },
        );
        let a11y_sec = format!("{sec_label} ({})", sec.items.len());
        toggle_resp.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                ui.is_enabled() && !interaction_blocked,
                &a11y_sec,
            )
        });
        let list_id = panel_id.with("list");
        let sec_focused = ui.is_enabled()
            && ui.ctx().memory(|m| m.has_focus(list_id))
            && self.focused_nav_id == Some(sec_id);
        let sp = ui.painter().with_clip_rect(hdr_rect);
        if !interaction_blocked && toggle_resp.hovered() {
            sp.rect_filled(hdr_rect, 0.0_f32, colors.surface_2.gamma_multiply(0.65_f32));
        }
        if sec_focused {
            sp.rect_stroke(
                hdr_rect,
                3.0_f32,
                Stroke::new(1.2_f32, colors.accent),
                StrokeKind::Inside,
            );
        }

        let chev_rect = Rect::from_min_size(
            Pos2::new(hdr_rect.min.x + 6.0_f32, hdr_rect.center().y - 6.0_f32),
            Vec2::splat(12.0_f32),
        );
        if expanded {
            icons::paint(&sp, chev_rect, Icon::ChevronDown, colors.text_dim);
        } else {
            paint_chevron_right(&sp, chev_rect, colors.text_dim);
        }

        let count_str = sec.items.len().to_string();
        let cg = layout_single_line(
            ui,
            &count_str,
            FontId::proportional(10.5_f32),
            colors.text_dim,
            40.0_f32,
        );
        let count_pill_w = cg.size().x + 10.0_f32;
        let label_left = chev_rect.max.x + 4.0_f32;
        let label_max_w = (toggle_rect.max.x - label_left - count_pill_w - 8.0_f32).max(16.0_f32);
        let lg = layout_single_line(
            ui,
            sec_label,
            FontId::proportional(11.5_f32),
            colors.text,
            label_max_w,
        );
        let lg_w = lg.size().x;
        sp.galley(
            Pos2::new(label_left, hdr_rect.center().y - lg.size().y * 0.5_f32),
            lg,
            colors.text,
        );
        let pill_x = (label_left + lg_w + 6.0_f32).min(toggle_rect.max.x - count_pill_w - 2.0_f32);
        let pill_rect = Rect::from_min_size(
            Pos2::new(pill_x, hdr_rect.center().y - 8.0_f32),
            Vec2::new(count_pill_w, 16.0_f32),
        );
        sp.rect_filled(pill_rect, 8.0_f32, colors.surface_2);
        sp.galley(
            Pos2::new(
                pill_rect.center().x - cg.size().x * 0.5_f32,
                pill_rect.center().y - cg.size().y * 0.5_f32,
            ),
            cg,
            colors.text_dim,
        );
        let toggle_resp = toggle_resp.on_hover_text(sec_label);
        if !interaction_blocked && toggle_resp.clicked() && out.is_none() {
            self.focused_nav_id = Some(sec_id);
            ui.ctx().memory_mut(|m| m.request_focus(list_id));
            self.toggle_section(sec.kind);
        }

        if !expanded {
            return out;
        }

        for row in rows {
            match row {
                FlatRow::Directory {
                    key,
                    name,
                    path,
                    area,
                    depth,
                    file_count,
                    expanded: dir_expanded,
                    stage_paths,
                    unstage_paths,
                    discard_paths,
                    ..
                } => {
                    if let Some(dir_act) = self.render_directory_row(
                        ui,
                        panel_id,
                        avail_w,
                        key,
                        name,
                        path,
                        *area,
                        *depth,
                        *file_count,
                        *dir_expanded,
                        stage_paths,
                        unstage_paths,
                        discard_paths,
                        input.french,
                        input.capabilities,
                        allow_bulk,
                        colors,
                        interaction_blocked,
                    ) && out.is_none()
                    {
                        out = Some(dir_act);
                    }
                }
                FlatRow::File {
                    key,
                    name,
                    dir_hint,
                    entry,
                    depth,
                    ..
                } => {
                    if let Some(file_act) = self.render_file_row(
                        ui,
                        panel_id,
                        avail_w,
                        key,
                        name,
                        dir_hint.as_deref(),
                        entry,
                        *depth,
                        input,
                        lang,
                        colors,
                        interaction_blocked,
                    ) && out.is_none()
                    {
                        out = Some(file_act);
                    }
                }
            }
        }

        out
    }

    #[allow(clippy::too_many_arguments)]
    fn render_directory_row(
        &mut self,
        ui: &mut egui::Ui,
        panel_id: Id,
        avail_w: f32,
        key: &str,
        name: &str,
        path: &str,
        area: ScmArea,
        depth: usize,
        file_count: usize,
        expanded: bool,
        stage_paths: &[String],
        unstage_paths: &[String],
        discard_paths: &[String],
        french: bool,
        caps: ScmCapabilities,
        allow_bulk: bool,
        colors: PanelColors,
        interaction_blocked: bool,
    ) -> Option<ScmChangesAction> {
        let mut out = None;
        let row_id = panel_id.with(("row", key));
        let (_, row_rect) = ui.allocate_space(Vec2::new(avail_w, ITEM_ROW_H));
        let mut right_x = row_rect.max.x - 6.0_f32;
        let ptr_inside = ui
            .ctx()
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|p| row_rect.contains(p));
        let list_id = panel_id.with("list");
        let row_focused = ui.is_enabled()
            && ui.ctx().memory(|m| m.has_focus(list_id))
            && self.focused_nav_id == Some(row_id);

        if allow_bulk && caps.can_unstage && !unstage_paths.is_empty() {
            right_x -= ROW_ACTION_SIDE;
            let btn_rect = Rect::from_min_size(
                Pos2::new(right_x, row_rect.center().y - ROW_ACTION_SIDE * 0.5_f32),
                Vec2::splat(ROW_ACTION_SIDE),
            );
            let lbl = if french {
                "Désindexer le dossier"
            } else {
                "Unstage folder"
            };
            if Self::render_small_action_btn(
                ui,
                panel_id.with(("dir-unstage", key)),
                btn_rect,
                lbl,
                false,
                ptr_inside || row_focused,
                colors,
                interaction_blocked,
                paint_minus,
            ) {
                out = Some(ScmChangesAction::Unstage(unstage_paths.to_vec()));
            }
            right_x -= 2.0_f32;
        }
        if allow_bulk && caps.can_stage && !stage_paths.is_empty() {
            right_x -= ROW_ACTION_SIDE;
            let btn_rect = Rect::from_min_size(
                Pos2::new(right_x, row_rect.center().y - ROW_ACTION_SIDE * 0.5_f32),
                Vec2::splat(ROW_ACTION_SIDE),
            );
            let lbl = if french {
                "Indexer le dossier"
            } else {
                "Stage folder"
            };
            if Self::render_small_action_btn(
                ui,
                panel_id.with(("dir-stage", key)),
                btn_rect,
                lbl,
                false,
                ptr_inside || row_focused,
                colors,
                interaction_blocked,
                |p, r, c| icons::paint(p, r, Icon::Plus, c),
            ) {
                out = Some(ScmChangesAction::Stage(stage_paths.to_vec()));
            }
            right_x -= 2.0_f32;
        }
        if allow_bulk && caps.can_discard && !discard_paths.is_empty() {
            right_x -= ROW_ACTION_SIDE;
            let btn_rect = Rect::from_min_size(
                Pos2::new(right_x, row_rect.center().y - ROW_ACTION_SIDE * 0.5_f32),
                Vec2::splat(ROW_ACTION_SIDE),
            );
            let is_untracked = area == ScmArea::Untracked;
            let lbl = match (is_untracked, french) {
                (true, false) => "Delete untracked in folder",
                (true, true) => "Supprimer les non suivis du dossier",
                (false, false) => "Discard folder",
                (false, true) => "Abandonner le dossier",
            };
            if Self::render_small_action_btn(
                ui,
                panel_id.with(("dir-discard", key)),
                btn_rect,
                lbl,
                true,
                ptr_inside || row_focused,
                colors,
                interaction_blocked,
                |p, r, c| {
                    if is_untracked {
                        paint_trash(p, r, c);
                    } else {
                        paint_undo(p, r, c);
                    }
                },
            ) {
                out = Some(ScmChangesAction::RequestDiscard {
                    area,
                    paths: discard_paths.to_vec(),
                });
            }
            right_x -= 2.0_f32;
        }

        let body_rect = Rect::from_min_max(
            row_rect.min,
            Pos2::new(right_x.max(row_rect.min.x + 24.0_f32), row_rect.max.y),
        );
        let resp = ui.interact(
            body_rect,
            row_id,
            if interaction_blocked {
                Sense::hover()
            } else {
                Sense::click()
            },
        );
        let a11y = format!("{name} ({file_count})");
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                ui.is_enabled() && !interaction_blocked,
                &a11y,
            )
        });
        let p = ui.painter().with_clip_rect(row_rect);
        if !interaction_blocked && resp.hovered() {
            p.rect_filled(row_rect, 3.0_f32, colors.surface_2.gamma_multiply(0.75_f32));
        }
        if row_focused {
            p.rect_stroke(
                row_rect,
                3.0_f32,
                Stroke::new(1.2_f32, colors.accent),
                StrokeKind::Inside,
            );
        }

        let indent = TREE_BASE_PAD + (depth.min(12) as f32) * TREE_INDENT_STEP;
        let chev_rect = Rect::from_min_size(
            Pos2::new(row_rect.min.x + indent, row_rect.center().y - 5.5_f32),
            Vec2::splat(11.0_f32),
        );
        if expanded {
            icons::paint(&p, chev_rect, Icon::ChevronDown, colors.text_dim);
        } else {
            paint_chevron_right(&p, chev_rect, colors.text_dim);
        }
        let folder_rect = Rect::from_min_size(
            Pos2::new(chev_rect.max.x + 3.0_f32, row_rect.center().y - 6.0_f32),
            Vec2::splat(12.0_f32),
        );
        paint_folder_icon(&p, folder_rect, expanded, colors.text_dim);

        let cnt_str = file_count.to_string();
        let cnt_g = layout_single_line(
            ui,
            &cnt_str,
            FontId::proportional(10.5_f32),
            colors.text_dim,
            32.0_f32,
        );
        let cnt_w = cnt_g.size().x;
        p.galley(
            Pos2::new(
                (right_x - cnt_w - 4.0_f32).max(folder_rect.max.x + 20.0_f32),
                row_rect.center().y - cnt_g.size().y * 0.5_f32,
            ),
            cnt_g,
            colors.text_dim,
        );

        let name_left = folder_rect.max.x + 5.0_f32;
        let name_max_w = (right_x - cnt_w - 10.0_f32 - name_left).max(16.0_f32);
        let ng = layout_single_line(
            ui,
            name,
            FontId::proportional(12.0_f32),
            colors.text,
            name_max_w,
        );
        p.galley(
            Pos2::new(name_left, row_rect.center().y - ng.size().y * 0.5_f32),
            ng,
            colors.text,
        );

        let resp = resp.on_hover_text(path);
        if !interaction_blocked && resp.clicked() && out.is_none() {
            self.focused_nav_id = Some(row_id);
            ui.ctx().memory_mut(|m| m.request_focus(list_id));
            self.toggle_dir(key);
        }

        out
    }

    #[allow(clippy::too_many_arguments)]
    fn render_file_row(
        &mut self,
        ui: &mut egui::Ui,
        panel_id: Id,
        avail_w: f32,
        key: &str,
        name: &str,
        dir_hint: Option<&str>,
        entry: &ScmEntry,
        depth: usize,
        input: &ScmChangesInput<'_>,
        lang: ScmLang,
        colors: PanelColors,
        interaction_blocked: bool,
    ) -> Option<ScmChangesAction> {
        let mut out = None;
        let row_id = panel_id.with(("row", key));
        let row_h = if entry.is_conflict() {
            CONFLICT_ROW_H
        } else {
            ITEM_ROW_H
        };
        let (_, row_rect) = ui.allocate_space(Vec2::new(avail_w, row_h));
        let mut right_x = row_rect.max.x - 6.0_f32;

        let is_active = input
            .active_entry
            .is_some_and(|(a, p)| a == entry.area && p == entry.path);
        let ptr_inside = ui
            .ctx()
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|p| row_rect.contains(p));
        let list_id = panel_id.with("list");
        let row_focused = ui.is_enabled()
            && ui.ctx().memory(|m| m.has_focus(list_id))
            && self.focused_nav_id == Some(row_id);
        let allow_row_mut = !input.read_only && !input.busy;

        if allow_row_mut && input.capabilities.can_unstage && entry.can_unstage() {
            right_x -= ROW_ACTION_SIDE;
            let btn_rect = Rect::from_min_size(
                Pos2::new(right_x, row_rect.center().y - ROW_ACTION_SIDE * 0.5_f32),
                Vec2::splat(ROW_ACTION_SIDE),
            );
            let lbl = if input.french {
                "Désindexer"
            } else {
                "Unstage"
            };
            if Self::render_small_action_btn(
                ui,
                panel_id.with(("file-unstage", key)),
                btn_rect,
                lbl,
                false,
                ptr_inside || row_focused,
                colors,
                interaction_blocked,
                paint_minus,
            ) {
                out = Some(ScmChangesAction::Unstage(vec![entry.path.clone()]));
            }
            right_x -= 2.0_f32;
        }
        if allow_row_mut && input.capabilities.can_stage && entry.can_stage() {
            right_x -= ROW_ACTION_SIDE;
            let btn_rect = Rect::from_min_size(
                Pos2::new(right_x, row_rect.center().y - ROW_ACTION_SIDE * 0.5_f32),
                Vec2::splat(ROW_ACTION_SIDE),
            );
            let lbl = if input.french { "Indexer" } else { "Stage" };
            if Self::render_small_action_btn(
                ui,
                panel_id.with(("file-stage", key)),
                btn_rect,
                lbl,
                false,
                ptr_inside || row_focused,
                colors,
                interaction_blocked,
                |p, r, c| icons::paint(p, r, Icon::Plus, c),
            ) {
                out = Some(ScmChangesAction::Stage(vec![entry.path.clone()]));
            }
            right_x -= 2.0_f32;
        }
        if allow_row_mut && input.capabilities.can_discard && entry.can_discard() {
            right_x -= ROW_ACTION_SIDE;
            let btn_rect = Rect::from_min_size(
                Pos2::new(right_x, row_rect.center().y - ROW_ACTION_SIDE * 0.5_f32),
                Vec2::splat(ROW_ACTION_SIDE),
            );
            let is_untracked = entry.area == ScmArea::Untracked;
            let lbl = if is_untracked {
                if input.french {
                    "Supprimer le fichier non suivi"
                } else {
                    "Delete untracked file"
                }
            } else if entry.status == ScmFileStatus::Deleted {
                if input.french {
                    "Restaurer le fichier"
                } else {
                    "Restore file"
                }
            } else if input.french {
                "Abandonner les modifications"
            } else {
                "Discard changes"
            };
            if Self::render_small_action_btn(
                ui,
                panel_id.with(("file-discard", key)),
                btn_rect,
                lbl,
                true,
                ptr_inside || row_focused,
                colors,
                interaction_blocked,
                |p, r, c| {
                    if is_untracked {
                        paint_trash(p, r, c);
                    } else {
                        paint_undo(p, r, c);
                    }
                },
            ) {
                out = Some(ScmChangesAction::RequestDiscard {
                    area: entry.area,
                    paths: vec![entry.path.clone()],
                });
            }
            right_x -= 2.0_f32;
        }

        let can_open_this = input.capabilities.can_open_diff
            || (input.capabilities.can_open_file && entry.status != ScmFileStatus::Deleted);
        let body_rect = Rect::from_min_max(
            row_rect.min,
            Pos2::new(right_x.max(row_rect.min.x + 24.0_f32), row_rect.max.y),
        );
        let sense = if interaction_blocked {
            Sense::hover()
        } else if can_open_this {
            Sense::click()
        } else {
            Sense::focusable_noninteractive()
        };
        let resp = ui.interact(body_rect, row_id, sense);
        let st_title = entry.status.title(lang);
        let hover_title = match entry.old_path.as_deref() {
            Some(old) => format!("{old} -> {} ({st_title})", entry.path),
            None => format!("{} ({st_title})", entry.path),
        };
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(
                if can_open_this {
                    egui::WidgetType::Button
                } else {
                    egui::WidgetType::Label
                },
                ui.is_enabled() && !interaction_blocked,
                &hover_title,
            )
        });

        let p = ui.painter().with_clip_rect(row_rect);
        if is_active {
            p.rect_filled(row_rect, 3.0_f32, colors.surface_3);
        } else if !interaction_blocked && can_open_this && resp.hovered() {
            p.rect_filled(row_rect, 3.0_f32, colors.surface_2.gamma_multiply(0.8_f32));
        }
        if row_focused {
            p.rect_stroke(
                row_rect,
                3.0_f32,
                Stroke::new(1.2_f32, colors.accent),
                StrokeKind::Inside,
            );
        }

        // Right-aligned status badge (or Unresolved conflict pill) + optional +/- counts
        let mut badge_left = right_x;
        if entry.is_conflict() {
            let unres_lbl = if input.french {
                "Non résolu"
            } else {
                "Unresolved"
            };
            let ug = layout_single_line(
                ui,
                unres_lbl,
                FontId::proportional(10.5_f32),
                colors.warn,
                76.0_f32,
            );
            let pill_w = 13.0_f32 + ug.size().x + 8.0_f32;
            badge_left -= pill_w;
            let pill_rect = Rect::from_min_size(
                Pos2::new(badge_left, row_rect.center().y - 8.5_f32),
                Vec2::new(pill_w, 17.0_f32),
            );
            p.rect_filled(pill_rect, 4.0_f32, colors.warn.gamma_multiply(0.14_f32));
            paint_alert_triangle(
                &p,
                Rect::from_min_size(
                    Pos2::new(pill_rect.min.x + 3.0_f32, row_rect.center().y - 5.0_f32),
                    Vec2::splat(10.0_f32),
                ),
                colors.warn,
            );
            p.galley(
                Pos2::new(
                    pill_rect.min.x + 15.0_f32,
                    row_rect.center().y - ug.size().y * 0.5_f32,
                ),
                ug,
                colors.warn,
            );
            badge_left -= 4.0_f32;
        } else {
            let code_color = entry.status.color();
            let sg = layout_single_line(
                ui,
                entry.status.code(),
                FontId::monospace(11.5_f32),
                code_color,
                16.0_f32,
            );
            badge_left -= 14.0_f32;
            p.galley(
                Pos2::new(
                    badge_left + (14.0_f32 - sg.size().x) * 0.5_f32,
                    row_rect.center().y - sg.size().y * 0.5_f32,
                ),
                sg,
                code_color,
            );
            badge_left -= 4.0_f32;

            if let Some(rem) = entry.removed.filter(|&n| n > 0)
                && badge_left - row_rect.min.x > 120.0_f32
            {
                let rtxt = format!("-{rem}");
                let rg = layout_single_line(
                    ui,
                    &rtxt,
                    FontId::monospace(10.5_f32),
                    GIT_COLOR_DELETED,
                    44.0_f32,
                );
                badge_left -= rg.size().x;
                p.galley(
                    Pos2::new(badge_left, row_rect.center().y - rg.size().y * 0.5_f32),
                    rg,
                    GIT_COLOR_DELETED,
                );
                badge_left -= 4.0_f32;
            }
            if let Some(add) = entry.added.filter(|&n| n > 0)
                && badge_left - row_rect.min.x > 120.0_f32
            {
                let atxt = format!("+{add}");
                let ag = layout_single_line(
                    ui,
                    &atxt,
                    FontId::monospace(10.5_f32),
                    GIT_COLOR_ADDED,
                    44.0_f32,
                );
                badge_left -= ag.size().x;
                p.galley(
                    Pos2::new(badge_left, row_rect.center().y - ag.size().y * 0.5_f32),
                    ag,
                    GIT_COLOR_ADDED,
                );
                badge_left -= 4.0_f32;
            }
        }

        let indent = if self.view_mode == ScmViewMode::Tree {
            TREE_BASE_PAD + 14.0_f32 + (depth.min(12) as f32) * TREE_INDENT_STEP
        } else {
            TREE_BASE_PAD + 4.0_f32
        };
        let file_icon_rect = Rect::from_min_size(
            Pos2::new(row_rect.min.x + indent, row_rect.min.y + 5.0_f32),
            Vec2::splat(13.0_f32),
        );
        paint_file_icon(&p, file_icon_rect, colors.text_dim);

        let text_left = file_icon_rect.max.x + 5.0_f32;
        let text_max_w = (badge_left - text_left - 2.0_f32).max(16.0_f32);
        let name_color = if entry.status == ScmFileStatus::Deleted {
            colors.text_dim
        } else {
            colors.text
        };

        if let Some(ck) = entry.conflict {
            let ng = layout_single_line(
                ui,
                name,
                FontId::proportional(12.0_f32),
                name_color,
                text_max_w,
            );
            let sub_g = layout_single_line(
                ui,
                ck.label(input.french),
                FontId::proportional(10.5_f32),
                colors.warn,
                text_max_w,
            );
            p.galley(
                Pos2::new(text_left, row_rect.min.y + 3.0_f32),
                ng,
                name_color,
            );
            p.galley(
                Pos2::new(text_left, row_rect.min.y + 19.0_f32),
                sub_g,
                colors.warn,
            );
        } else {
            let ng = layout_single_line(
                ui,
                name,
                FontId::proportional(12.0_f32),
                name_color,
                text_max_w,
            );
            let nw = ng.size().x;
            p.galley(
                Pos2::new(text_left, row_rect.center().y - ng.size().y * 0.5_f32),
                ng,
                name_color,
            );
            if let Some(dir) = dir_hint
                && text_max_w - nw > 28.0_f32
            {
                let dg = layout_single_line(
                    ui,
                    dir,
                    FontId::proportional(11.0_f32),
                    colors.text_dim,
                    text_max_w - nw - 6.0_f32,
                );
                p.galley(
                    Pos2::new(
                        text_left + nw + 6.0_f32,
                        row_rect.center().y - dg.size().y * 0.5_f32,
                    ),
                    dg,
                    colors.text_dim,
                );
            }
        }

        let resp = resp.on_hover_text(&hover_title);
        if !interaction_blocked && out.is_none() && (resp.clicked() || resp.double_clicked()) {
            self.focused_nav_id = Some(row_id);
            ui.ctx().memory_mut(|m| m.request_focus(list_id));
            if can_open_this {
                if resp.double_clicked() && input.capabilities.can_open_diff {
                    out = Some(ScmChangesAction::OpenDiff {
                        area: entry.area,
                        path: entry.path.clone(),
                        old_path: entry.old_path.clone(),
                        pinned: true,
                    });
                } else if resp.clicked() {
                    if input.capabilities.can_open_diff {
                        out = Some(ScmChangesAction::OpenDiff {
                            area: entry.area,
                            path: entry.path.clone(),
                            old_path: entry.old_path.clone(),
                            pinned: false,
                        });
                    } else if input.capabilities.can_open_file
                        && entry.status != ScmFileStatus::Deleted
                    {
                        out = Some(ScmChangesAction::OpenFile(entry.path.clone()));
                    }
                }
            }
        }

        out
    }
}
