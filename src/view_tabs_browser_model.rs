//! Pure data models and exact behavioral parity logic for:
//! - Center `ViewTabs` strip (`ViewTabs.vue`), composing directly with `crate::pane_grid::{EditorTabEntry, EditorTabStrip, ShownTab}`
//! - Editor tab naming, diff tab keys, saved-tab validation, minimal external-change edit & EOL detection (`editorTabs.js`, `tabLabels.js`)
//! - Built-in browser address bar normalization & strict HTTP/HTTPS/`about:blank` guard (`src/shared/browserUrl.js`), composing cleanly with `src/browser.rs`

use std::fmt::Write as _;

use crate::pane_grid::{EditorTabEntry, EditorTabStrip, ShownTab};
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TabsLang {
    #[default]
    En,
    Fr,
}

// ---------------------------------------------------------------------------
// 1. Editor Tab Keys, Diff Paths & Minimal External Edit (`editorTabs.js`)
// ---------------------------------------------------------------------------

pub const DIFF_SEP: char = '\0';
pub const MAX_SAVED_EDITOR_TABS: usize = 50;
pub const MAX_SAVED_TAB_PATH_LEN: usize = 4_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffTabArea {
    Unstaged,
    Staged,
    Commit { commit: String },
    Branch { base: String, commit: String },
    Local { id: String, formatted_date: String },
    Agent { edit_key: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffTabMeta {
    pub full_path: String,
    pub area: DiffTabArea,
}

#[must_use]
pub fn path_key(path: &str) -> String {
    path.replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

#[must_use]
pub fn same_path(a: &str, b: &str) -> bool {
    path_key(a) == path_key(b)
}

#[must_use]
pub fn diff_tab_path(full: &str, area: &DiffTabArea) -> String {
    match area {
        DiffTabArea::Unstaged => format!("{full}{DIFF_SEP}diff:unstaged"),
        DiffTabArea::Staged => format!("{full}{DIFF_SEP}diff:staged"),
        DiffTabArea::Commit { commit } => format!("{full}{DIFF_SEP}diff:commit:{commit}"),
        DiffTabArea::Branch { base, commit } => {
            format!("{full}{DIFF_SEP}diff:branch:{base}..{commit}")
        }
        DiffTabArea::Local { id, .. } => format!("{full}{DIFF_SEP}diff:local:{id}"),
        DiffTabArea::Agent { edit_key } => format!("{full}{DIFF_SEP}diff:agent:{edit_key}"),
    }
}

#[must_use]
pub fn is_diff_tab_path(path: &str) -> bool {
    path.contains(DIFF_SEP)
}

#[must_use]
pub fn doc_path_of<'a>(tab_path: &'a str, diff: Option<&'a DiffTabMeta>) -> &'a str {
    if let Some(d) = diff
        && !d.full_path.is_empty()
    {
        return d.full_path.as_str();
    }
    match tab_path.find(DIFF_SEP) {
        Some(idx) => &tab_path[..idx],
        None => tab_path,
    }
}

#[must_use]
pub fn file_name_of(path: &str) -> &str {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    if name.is_empty() { path } else { name }
}

/// Opens `path` on an existing `EditorTabStrip` (`pane_grid.rs`), inserting new tabs immediately
/// after the active tab like VS Code (`editorTabs.js:openTab`) and returning any replaced preview path.
pub fn open_tab_in_strip(
    strip: &mut EditorTabStrip,
    path: impl Into<String>,
    preview: bool,
    preview_tabs_enabled: bool,
) -> Option<String> {
    let path = path.into();
    let use_preview = preview && preview_tabs_enabled;

    if let Some(existing) = strip.files.iter_mut().find(|f| same_path(&f.path, &path)) {
        if !use_preview {
            existing.pinned = true;
        }
        strip.active_path = Some(existing.path.clone());
        return None;
    }

    let new_entry = EditorTabEntry {
        path: path.clone(),
        pinned: !use_preview,
        dirty: false,
        deleted: false,
        failed: false,
    };

    if use_preview
        && let Some(preview_slot) = strip.files.iter_mut().find(|f| !f.pinned && !f.dirty)
    {
        let replaced = preview_slot.path.clone();
        *preview_slot = new_entry;
        strip.active_path = Some(path);
        return Some(replaced);
    }

    let active_idx = strip
        .active_path
        .as_deref()
        .and_then(|act| strip.files.iter().position(|f| same_path(&f.path, act)));

    if let Some(idx) = active_idx {
        strip.files.insert(idx + 1, new_entry);
    } else {
        strip.files.push(new_entry);
    }
    strip.active_path = Some(path);
    None
}

#[must_use]
pub fn auto_save_delay_ms(raw_ms: Option<f64>, min_ms: u64, max_ms: u64, fallback_ms: u64) -> u64 {
    let val = raw_ms
        .filter(|n| n.is_finite())
        .map_or(fallback_ms, |n| n.round().max(0.0) as u64);
    val.clamp(min_ms, max_ms)
}

fn is_well_formed_saved_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    let drive_abs = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/');
    let unc_or_root = path.starts_with("\\\\") || path.starts_with('/');
    drive_abs || unc_or_root
}

/// Filters saved layout editor tabs (`editorTabs.js:validSavedFiles`):
/// absolute paths `<= 4000` chars, no diff tabs, case-insensitive deduplicated, capped at `50`.
#[must_use]
pub fn valid_saved_files(entries: &[EditorTabEntry]) -> Vec<EditorTabEntry> {
    let mut out: Vec<EditorTabEntry> = Vec::new();
    for entry in entries {
        if entry.path.is_empty()
            || entry.path.chars().count() > MAX_SAVED_TAB_PATH_LEN
            || !is_well_formed_saved_path(&entry.path)
            || is_diff_tab_path(&entry.path)
            || out.iter().any(|o| same_path(&o.path, &entry.path))
        {
            continue;
        }
        out.push(EditorTabEntry {
            path: entry.path.clone(),
            pinned: entry.pinned,
            dirty: false,
            deleted: false,
            failed: false,
        });
        if out.len() >= MAX_SAVED_EDITOR_TABS {
            break;
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinimalTextEdit {
    pub start_byte: usize,
    pub end_before_byte: usize,
    pub replacement: String,
}

fn splits_crlf(bytes: &[u8], idx: usize) -> bool {
    idx.checked_sub(1)
        .is_some_and(|prev| bytes.get(prev) == Some(&b'\r') && bytes.get(idx) == Some(&b'\n'))
}

/// Computes the minimal contiguous UTF-8 edit turning `before` into `after` without splitting
/// a `\r\n` CRLF pair (`editorTabs.js:minimalEdit`).
#[must_use]
pub fn minimal_edit(before: &str, after: &str) -> Option<MinimalTextEdit> {
    if before == after {
        return None;
    }
    let b_bytes = before.as_bytes();
    let a_bytes = after.as_bytes();
    let max_common = b_bytes.len().min(a_bytes.len());

    let mut start = 0usize;
    while start < max_common && b_bytes[start] == a_bytes[start] {
        start += 1;
    }
    while start > 0
        && (!before.is_char_boundary(start)
            || !after.is_char_boundary(start)
            || splits_crlf(b_bytes, start)
            || splits_crlf(a_bytes, start))
    {
        start -= 1;
    }

    let mut end = 0usize;
    while end < max_common - start
        && b_bytes[b_bytes.len() - 1 - end] == a_bytes[a_bytes.len() - 1 - end]
    {
        end += 1;
    }
    while end > 0 {
        let b_cut = b_bytes.len() - end;
        let a_cut = a_bytes.len() - end;
        if before.is_char_boundary(b_cut)
            && after.is_char_boundary(a_cut)
            && !splits_crlf(b_bytes, b_cut)
            && !splits_crlf(a_bytes, a_cut)
        {
            break;
        }
        end -= 1;
    }

    Some(MinimalTextEdit {
        start_byte: start,
        end_before_byte: b_bytes.len() - end,
        replacement: after[start..a_bytes.len() - end].to_string(),
    })
}

#[must_use]
pub fn main_eol(text: &str) -> Option<&'static str> {
    let crlf = text.matches("\r\n").count();
    let total_lf = text.bytes().filter(|&b| b == b'\n').count();
    let lf = total_lf.saturating_sub(crlf);
    if crlf == 0 && lf == 0 {
        None
    } else if crlf > lf {
        Some("\r\n")
    } else {
        Some("\n")
    }
}

// ---------------------------------------------------------------------------
// 2. Editor Tab Disambiguation & Diff Labels (`tabLabels.js`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct OpenTabLabelInput<'a> {
    pub tab_path: &'a str,
    pub diff: Option<&'a DiffTabMeta>,
}

#[must_use]
pub fn base_tab_label(
    target: &OpenTabLabelInput<'_>,
    all_open: &[OpenTabLabelInput<'_>],
) -> String {
    let doc = doc_path_of(target.tab_path, target.diff);
    let is_diff = target.diff.is_some() || is_diff_tab_path(target.tab_path);
    let name = file_name_of(doc).to_lowercase();
    let twins = all_open
        .iter()
        .filter(|other| {
            let other_diff = other.diff.is_some() || is_diff_tab_path(other.tab_path);
            is_diff == other_diff
                && file_name_of(doc_path_of(other.tab_path, other.diff)).to_lowercase() == name
        })
        .take(2)
        .count();
    qualified_tab_name(doc, twins > 1)
}

pub fn qualified_tab_name(doc: &str, has_twin: bool) -> String {
    let name = file_name_of(doc);
    if !has_twin {
        return name.to_string();
    }
    let parts: Vec<&str> = doc.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
    if parts.len() > 1 {
        format!("{name} · {}", parts[parts.len() - 2])
    } else {
        name.to_string()
    }
}

fn short_commit(raw: &str) -> &str {
    match raw.char_indices().nth(7) {
        Some((idx, _)) => &raw[..idx],
        None => raw,
    }
}

#[must_use]
pub fn editor_tab_label(
    target: &OpenTabLabelInput<'_>,
    all_open: &[OpenTabLabelInput<'_>],
    lang: TabsLang,
) -> String {
    let base = base_tab_label(target, all_open);
    let Some(diff) = target.diff else {
        return base;
    };
    match (&diff.area, lang) {
        (DiffTabArea::Branch { base: b, commit: c }, _) => {
            format!("{base} ({}..{})", short_commit(b), short_commit(c))
        }
        (DiffTabArea::Commit { commit }, _) => format!("{base} ({})", short_commit(commit)),
        (DiffTabArea::Local { formatted_date, .. }, TabsLang::En) => {
            let clean_date = formatted_date.replace(['/', '\\'], "-");
            format!("{base} (File Saved • {clean_date}) ↔ {base}")
        }
        (DiffTabArea::Local { formatted_date, .. }, TabsLang::Fr) => {
            let clean_date = formatted_date.replace(['/', '\\'], "-");
            format!("{base} (Fichier enregistré • {clean_date}) ↔ {base}")
        }
        (DiffTabArea::Agent { .. }, TabsLang::En) => format!("{base} (agent edit)"),
        (DiffTabArea::Agent { .. }, TabsLang::Fr) => format!("{base} (modification de l'agent)"),
        (DiffTabArea::Staged, TabsLang::En) => format!("{base} (staged diff)"),
        (DiffTabArea::Staged, TabsLang::Fr) => format!("{base} (diff indexé)"),
        (DiffTabArea::Unstaged, _) => format!("{base} (diff)"),
    }
}

// ---------------------------------------------------------------------------
// 3. Center `ViewTabs` Strip (`ViewTabs.vue`, composing `EditorTabStrip` & `ShownTab`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewTabKind {
    Terminal,
    Editor,
    Browser,
}

#[derive(Debug, Clone, Copy)]
pub enum ViewTabLeafSource<'a> {
    Editor {
        leaf_id: &'a str,
        strip: &'a EditorTabStrip,
        diffs_by_tab_path: &'a [(String, DiffTabMeta)],
        live_share_host: Option<&'a str>,
    },
    Browser {
        leaf_id: &'a str,
        title: &'a str,
        url: &'a str,
        live_share_host: Option<&'a str>,
    },
    LiveSharePage {
        leaf_id: &'a str,
        host: &'a str,
        page_title: &'a str,
        page_url: &'a str,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewTabItem {
    pub key: String,
    pub id: String,
    pub path: Option<String>,
    pub file_for_icon: Option<String>,
    pub kind: ViewTabKind,
    pub label: String,
    pub hint: String,
    pub active: bool,
    pub preview: bool,
    pub dirty: bool,
    pub deleted: bool,
    pub failed: bool,
    pub host: bool,
    pub close_title: Option<String>,
    pub close_aria_label: Option<String>,
}

fn wrap_host_hint(host: Option<&str>, hint: String, lang: TabsLang) -> String {
    match (host, lang) {
        (Some(h), TabsLang::En) => {
            let who = if h.is_empty() { "?" } else { h };
            format!("{who}’s tab: {hint}")
        }
        (Some(h), TabsLang::Fr) => {
            let who = if h.is_empty() { "?" } else { h };
            format!("Onglet de {who} : {hint}")
        }
        (None, _) => hint,
    }
}

fn close_labels(label: &str, dirty: bool, lang: TabsLang) -> (String, String) {
    match (dirty, lang) {
        (true, TabsLang::En) => (
            "Unsaved changes. Close (Ctrl+W)".to_string(),
            format!("Close {label} (unsaved changes)"),
        ),
        (true, TabsLang::Fr) => (
            "Modifications non enregistrées. Fermer (Ctrl+W)".to_string(),
            format!("Fermer {label} (modifications non enregistrées)"),
        ),
        (false, TabsLang::En) => {
            let s = format!("Close {label}");
            (s.clone(), s)
        }
        (false, TabsLang::Fr) => {
            let s = format!("Fermer {label}");
            (s.clone(), s)
        }
    }
}

#[must_use]
pub fn build_view_tabs(
    leaves: &[ViewTabLeafSource<'_>],
    shown: &ShownTab,
    lang: TabsLang,
) -> Vec<ViewTabItem> {
    let mut out = Vec::new();
    let (term_label, term_hint) = match lang {
        TabsLang::En => ("Terminal", "The terminals and agents of this project"),
        TabsLang::Fr => ("Terminal", "Les terminaux et les agents de ce projet"),
    };
    out.push(ViewTabItem {
        key: "terminal".to_string(),
        id: "terminal".to_string(),
        path: None,
        file_for_icon: None,
        kind: ViewTabKind::Terminal,
        label: term_label.to_string(),
        hint: term_hint.to_string(),
        active: shown.is_terminal(),
        preview: false,
        dirty: false,
        deleted: false,
        failed: false,
        host: false,
        close_title: None,
        close_aria_label: None,
    });

    for &src in leaves {
        match src {
            ViewTabLeafSource::Editor {
                leaf_id,
                strip,
                diffs_by_tab_path,
                live_share_host,
            } => {
                if strip.files.is_empty() {
                    let ed_word = match lang {
                        TabsLang::En => "Editor",
                        TabsLang::Fr => "Éditeur",
                    };
                    let (ct, ca) = close_labels(ed_word, false, lang);
                    out.push(ViewTabItem {
                        key: leaf_id.to_string(),
                        id: leaf_id.to_string(),
                        path: None,
                        file_for_icon: None,
                        kind: ViewTabKind::Editor,
                        label: ed_word.to_string(),
                        hint: ed_word.to_string(),
                        active: shown.leaf_id() == Some(leaf_id),
                        preview: false,
                        dirty: false,
                        deleted: false,
                        failed: false,
                        host: live_share_host.is_some(),
                        close_title: Some(ct),
                        close_aria_label: Some(ca),
                    });
                    continue;
                }

                let find_diff = |p: &str| -> Option<&DiffTabMeta> {
                    diffs_by_tab_path
                        .iter()
                        .find(|(tp, _)| same_path(tp, p))
                        .map(|(_, d)| d)
                };
                let label_inputs: Vec<OpenTabLabelInput<'_>> = strip
                    .files
                    .iter()
                    .map(|f| OpenTabLabelInput {
                        tab_path: &f.path,
                        diff: find_diff(&f.path),
                    })
                    .collect();

                for (entry, input) in strip.files.iter().zip(label_inputs.iter()) {
                    let doc_path = doc_path_of(&entry.path, input.diff);
                    let label = editor_tab_label(input, &label_inputs, lang);
                    let base_path_hint = if label == doc_path {
                        doc_path.to_string()
                    } else {
                        format!("{label}\n{doc_path}")
                    };
                    let preview = !entry.pinned;
                    let hint = if live_share_host.is_some() {
                        wrap_host_hint(live_share_host, base_path_hint, lang)
                    } else if preview {
                        match lang {
                            TabsLang::En => {
                                format!("{base_path_hint}\nPreview: double-click to keep it open")
                            }
                            TabsLang::Fr => format!(
                                "{base_path_hint}\nAperçu : double-cliquez pour le garder ouvert"
                            ),
                        }
                    } else {
                        base_path_hint
                    };
                    let current = strip
                        .active_path
                        .as_deref()
                        .is_some_and(|act| same_path(&entry.path, act));
                    let is_diff = input.diff.is_some() || is_diff_tab_path(&entry.path);
                    let (ct, ca) = close_labels(&label, entry.dirty, lang);
                    out.push(ViewTabItem {
                        key: format!("{leaf_id}|{}", entry.path),
                        id: leaf_id.to_string(),
                        path: Some(entry.path.clone()),
                        file_for_icon: Some(doc_path.to_string()),
                        kind: ViewTabKind::Editor,
                        label,
                        hint,
                        active: shown.leaf_id() == Some(leaf_id) && current,
                        preview,
                        dirty: entry.dirty,
                        deleted: !is_diff && entry.deleted,
                        failed: !is_diff && entry.failed,
                        host: live_share_host.is_some(),
                        close_title: Some(ct),
                        close_aria_label: Some(ca),
                    });
                }
            }
            ViewTabLeafSource::Browser {
                leaf_id,
                title,
                url,
                live_share_host,
            } => {
                let effective_url = if url.is_empty() || url == BLANK_URL {
                    ""
                } else {
                    url
                };
                let fallback_browser = match lang {
                    TabsLang::En => "Browser",
                    TabsLang::Fr => "Navigateur",
                };
                let label = if !title.is_empty() {
                    title.to_string()
                } else if !effective_url.is_empty() {
                    host_of(effective_url)
                } else {
                    fallback_browser.to_string()
                };
                let raw_hint = if !effective_url.is_empty() {
                    display_url(effective_url)
                } else {
                    label.clone()
                };
                let hint = wrap_host_hint(live_share_host, raw_hint, lang);
                let (ct, ca) = close_labels(&label, false, lang);
                out.push(ViewTabItem {
                    key: leaf_id.to_string(),
                    id: leaf_id.to_string(),
                    path: None,
                    file_for_icon: None,
                    kind: ViewTabKind::Browser,
                    label,
                    hint,
                    active: shown.leaf_id() == Some(leaf_id),
                    preview: false,
                    dirty: false,
                    deleted: false,
                    failed: false,
                    host: live_share_host.is_some(),
                    close_title: Some(ct),
                    close_aria_label: Some(ca),
                });
            }
            ViewTabLeafSource::LiveSharePage {
                leaf_id,
                host,
                page_title,
                page_url,
            } => {
                let label = if !page_title.is_empty() {
                    page_title.to_string()
                } else {
                    "Browser page".to_string()
                };
                let base = if !page_url.is_empty() {
                    page_url.to_string()
                } else {
                    label.clone()
                };
                let hint = wrap_host_hint(Some(host), base, lang);
                let (ct, ca) = close_labels(&label, false, lang);
                out.push(ViewTabItem {
                    key: leaf_id.to_string(),
                    id: leaf_id.to_string(),
                    path: None,
                    file_for_icon: None,
                    kind: ViewTabKind::Browser,
                    label,
                    hint,
                    active: shown.leaf_id() == Some(leaf_id),
                    preview: false,
                    dirty: false,
                    deleted: false,
                    failed: false,
                    host: true,
                    close_title: Some(ct),
                    close_aria_label: Some(ca),
                });
            }
        }
    }

    out
}

#[must_use]
pub const fn can_drag_view_tab(tab: &ViewTabItem) -> bool {
    !matches!(tab.kind, ViewTabKind::Terminal) && !tab.host
}

#[must_use]
pub const fn can_middle_close_view_tab(tab: &ViewTabItem) -> bool {
    !matches!(tab.kind, ViewTabKind::Terminal)
}

// ---------------------------------------------------------------------------
// 4. Browser Address Bar Normalization & Strict URL Guard (`browserUrl.js`)
// ---------------------------------------------------------------------------

pub const BLANK_URL: &str = "about:blank";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchEngine {
    #[default]
    Google,
    DuckDuckGo,
    Bing,
}

impl SearchEngine {
    #[must_use]
    pub const fn prefix_url(self) -> &'static str {
        match self {
            Self::Google => "https://www.google.com/search?q=",
            Self::DuckDuckGo => "https://duckduckgo.com/?q=",
            Self::Bing => "https://www.bing.com/search?q=",
        }
    }
}

fn encode_uri_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric()
            || matches!(
                b,
                b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
            )
        {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

#[must_use]
pub fn search_url(query: &str, engine: SearchEngine) -> String {
    format!("{}{}", engine.prefix_url(), encode_uri_component(query))
}

/// Validates a URL scheme/syntax for Tessel's built-in browser (`browserUrl.js:allowedBrowserUrl` + `src/browser.rs:allowed`).
/// Only `about:blank`, `http:`, `https:`, and (when `allow_session_blob` is true) the `blob:` scheme
/// pass this syntactic check; session/origin scoping for `blob:` URLs is enforced by the browser runtime (`src/browser.rs`).
/// `file:`, `javascript:`, `data:`, and `user:password@host` URLs are always refused.
#[must_use]
pub fn allowed_browser_url(raw: &str, allow_session_blob: bool) -> Option<String> {
    let s = raw.trim();
    if s == BLANK_URL {
        return Some(BLANK_URL.to_string());
    }
    let parsed = Url::parse(s).ok()?;
    let scheme = parsed.scheme();
    if allow_session_blob && scheme == "blob" {
        return Some(parsed.to_string());
    }
    if scheme != "http" && scheme != "https" {
        return None;
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return None;
    }
    Some(parsed.to_string())
}

fn split_authority_and_rest(input: &str) -> (&str, &str) {
    match input.find(['/', '?', '#']) {
        Some(idx) => (&input[..idx], &input[idx..]),
        None => (input, ""),
    }
}

fn is_ipv4_127(host: &str) -> bool {
    let parts: Vec<&str> = host.split('.').collect();
    parts.len() == 4
        && parts[0] == "127"
        && parts[1..]
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 3 && p.bytes().all(|b| b.is_ascii_digit()))
}

fn is_local_dev_authority(auth: &str) -> bool {
    if let Some(rest) = auth.strip_prefix('[') {
        let Some((ipv6, after)) = rest.split_once(']') else {
            return false;
        };
        let valid_v6 = !ipv6.is_empty() && ipv6.bytes().all(|b| b.is_ascii_hexdigit() || b == b':');
        let valid_port = after.is_empty()
            || after
                .strip_prefix(':')
                .is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
        return valid_v6 && valid_port;
    }

    let (host, port_ok) = match auth.split_once(':') {
        Some((h, p)) => (h, !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())),
        None => (auth, true),
    };
    if !port_ok {
        return false;
    }
    host.eq_ignore_ascii_case("localhost") || host == "0.0.0.0" || is_ipv4_127(host)
}

fn is_valid_dns_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 253 {
        return false;
    }
    name.split('.').all(|label| {
        let bytes = label.as_bytes();
        !bytes.is_empty()
            && bytes.len() <= 63
            && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
            && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
            && bytes
                .iter()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
    })
}

fn domain_port_address(input: &str) -> Option<String> {
    let (auth, _) = split_authority_and_rest(input);
    if auth
        .bytes()
        .any(|b| b.is_ascii_whitespace() || matches!(b, b'\\' | b'@'))
    {
        return None;
    }
    let (host_part, port_part) = auth.split_once(':')?;
    if !host_part.contains('.')
        || port_part.is_empty()
        || !port_part.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let probe = Url::parse(&format!("https://{input}")).ok()?;
    let host = probe.host_str()?.to_ascii_lowercase();
    let trimmed_host = host.trim_end_matches('.');
    if !is_valid_dns_name(trimmed_host) {
        return None;
    }
    let scheme = if trimmed_host.ends_with(".localhost") {
        "http"
    } else {
        "https"
    };
    Url::parse(&format!("{scheme}://{input}"))
        .ok()
        .map(|u| u.to_string())
}

fn looks_like_url(input: &str) -> bool {
    if input.bytes().any(|b| b.is_ascii_whitespace()) {
        return false;
    }
    let host_part = input.split('/').next().unwrap_or(input);
    let Some((before, tld)) = host_part.rsplit_once('.') else {
        return false;
    };
    !before.is_empty() && tld.len() >= 2 && tld.bytes().all(|b| b.is_ascii_alphabetic())
}

#[must_use]
pub fn looks_like_search(input: &str) -> bool {
    if input.contains(' ') {
        return true;
    }
    if looks_like_url(input) {
        return false;
    }
    !(input.contains('.') || input.contains(':'))
}

fn has_scheme_prefix(s: &str) -> bool {
    let Some((scheme, _)) = s.split_once(':') else {
        return false;
    };
    let mut bytes = scheme.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    first.is_ascii_alphabetic()
        && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'.' | b'-'))
}

fn looks_like_domain_with_optional_port(s: &str) -> bool {
    let (auth, _) = split_authority_and_rest(s);
    if auth.bytes().any(|b| b.is_ascii_whitespace() || b == b'/') {
        return false;
    }
    let host = match auth.split_once(':') {
        Some((h, port)) => {
            if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
                return false;
            }
            h
        }
        None => auth,
    };
    let Some((before, tld)) = host.rsplit_once('.') else {
        return false;
    };
    !before.is_empty() && tld.len() >= 2 && tld.bytes().all(|b| b.is_ascii_alphabetic())
}

/// Normalizes raw address-bar text (`browserUrl.js:normalizeBrowserInput`).
#[must_use]
pub fn normalize_browser_input(raw: &str, search: bool, engine: SearchEngine) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() || s == BLANK_URL {
        return Some(BLANK_URL.to_string());
    }
    let (auth, _) = split_authority_and_rest(s);
    if is_local_dev_authority(auth)
        && let Ok(u) = Url::parse(&format!("http://{s}"))
    {
        return allowed_browser_url(u.as_str(), false);
    }
    if let Some(dp) = domain_port_address(s) {
        return allowed_browser_url(&dp, false);
    }
    if has_scheme_prefix(s)
        && !looks_like_domain_with_optional_port(s)
        && let Ok(u) = Url::parse(s)
    {
        return allowed_browser_url(u.as_str(), false);
    }
    if search && looks_like_search(s) {
        return Some(search_url(s, engine));
    }
    match Url::parse(&format!("https://{s}")) {
        Ok(u) => allowed_browser_url(u.as_str(), false),
        Err(_) => search.then(|| search_url(s, engine)),
    }
}

#[must_use]
pub fn is_local_url(raw: &str) -> bool {
    let Ok(u) = Url::parse(raw) else {
        return false;
    };
    let Some(host_raw) = u.host_str() else {
        return false;
    };
    let host = host_raw
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    host == "localhost"
        || host.ends_with(".localhost")
        || host == "::1"
        || host == "0.0.0.0"
        || is_ipv4_127(&host)
}

#[must_use]
pub fn display_url(url: &str) -> String {
    if url.is_empty() || url == BLANK_URL {
        String::new()
    } else {
        url.to_string()
    }
}

#[must_use]
pub fn host_of(url: &str) -> String {
    let Ok(u) = Url::parse(url) else {
        return url.to_string();
    };
    let Some(host) = u.host_str() else {
        return url.to_string();
    };
    match u.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    }
}
