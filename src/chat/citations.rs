use pulldown_cmark::{Event, Parser, Tag};
use regex::Regex;
use std::{
    ops::Range,
    path::{Path, PathBuf},
    sync::LazyLock,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FileLink {
    pub path: PathBuf,
    pub position: Option<crate::files::FilePosition>,
}

pub(crate) struct Citations {
    pub text: String,
    pub links: Vec<(Range<usize>, FileLink)>,
}

pub(crate) fn copyable(role: &str, text: &str) -> bool {
    !text.is_empty() && (role == "You" || assistant_role(role))
}

fn show_header(ui: &mut eframe::egui::Ui, role: &str, label: &str, text: &str) {
    ui.horizontal(|ui| {
        ui.strong(label);
        if !copyable(role, text) {
            return;
        }
        let id = ui.next_auto_id().with("message-copy");
        let now = ui.input(|input| input.time);
        let copied = ui
            .data(|data| data.get_temp::<f64>(id))
            .is_some_and(|until| now < until);
        if ui
            .small_button(crate::i18n::tr(if copied {
                "Copied"
            } else {
                "Copy message"
            }))
            .clicked()
        {
            ui.ctx().copy_text(text.to_owned());
            ui.data_mut(|data| data.insert_temp(id, now + 1.5));
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(1500));
        }
    });
}

pub(crate) fn latest_control(
    ui: &mut eframe::egui::Ui,
    scroll: &eframe::egui::scroll_area::ScrollAreaOutput<()>,
    jump_id: eframe::egui::Id,
) {
    if scroll.content_size.y - scroll.inner_rect.height() - scroll.state.offset.y > 48.
        && ui.small_button(crate::i18n::tr("Jump to latest")).clicked()
    {
        ui.data_mut(|d| d.insert_temp(jump_id, true));
        ui.ctx().request_repaint();
    }
}
pub(crate) fn apply_latest(ui: &mut eframe::egui::Ui, jump_id: eframe::egui::Id) {
    if ui
        .data_mut(|d| d.remove_temp::<bool>(jump_id))
        .unwrap_or(false)
    {
        ui.scroll_to_cursor(Some(eframe::egui::Align::BOTTOM));
    }
}

pub(crate) fn show_message(
    ui: &mut eframe::egui::Ui,
    role: &str,
    label: &str,
    text: &str,
    clicked: &mut Option<FileLink>,
) {
    let top = ui.cursor().min;
    show_header(ui, role, label, text);
    for block in super::blocks::message_blocks(role, text) {
        match block {
            super::blocks::Block::Prose(prose) => show_prose(ui, role, prose, clicked),
            super::blocks::Block::Code { language, text } => {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        if !language.is_empty() {
                            ui.strong(language);
                        }
                        if !text.is_empty()
                            && ui.small_button(crate::i18n::tr("Copy code")).clicked()
                        {
                            ui.ctx().copy_text(text.clone());
                        }
                    });
                    ui.add(
                        eframe::egui::Label::new(eframe::egui::RichText::new(text).monospace())
                            .selectable(true),
                    );
                });
            }
        }
    }
    if assistant_role(role)
        && !text.is_empty()
        && ui
            .small_button("↑")
            .on_hover_text(crate::i18n::tr("Scroll this message to top"))
            .clicked()
    {
        ui.scroll_to_rect(
            eframe::egui::Rect::from_min_max(top, top),
            Some(eframe::egui::Align::TOP),
        );
    }
}

fn show_prose(ui: &mut eframe::egui::Ui, role: &str, text: &str, clicked: &mut Option<FileLink>) {
    if let Some(message) = parse(role, text) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let mut end = 0;
            for (range, path) in message.links {
                if end < range.start {
                    ui.label(&message.text[end..range.start]);
                }
                if ui.link(&message.text[range.clone()]).clicked() {
                    *clicked = Some(path);
                }
                end = range.end;
            }
            if end < message.text.len() {
                ui.label(&message.text[end..]);
            }
        });
    } else {
        ui.add(eframe::egui::Label::new(text).selectable(true));
    }
}

pub(crate) fn assistant_role(role: &str) -> bool {
    matches!(role, "Codex" | "Claude") || role.starts_with("Codex \u{b7} ")
}

pub(crate) fn parse(role: &str, source: &str) -> Option<Citations> {
    if !assistant_role(role) || !(source.contains(":codex-file-citation{") || source.contains("]("))
    {
        return None;
    }
    let mut result = Citations {
        text: String::new(),
        links: Vec::new(),
    };
    let mut end = 0;
    let mut events = Parser::new(source).into_offset_iter();
    while let Some((event, span)) = events.next() {
        if span.start < end {
            continue;
        }
        match event {
            Event::Start(Tag::Link {
                link_type: pulldown_cmark::LinkType::Inline,
                dest_url,
                ..
            }) => {
                result.append_prose(&source[end..span.start]);
                if let Some(path) = file_link(&dest_url) {
                    let mut label = String::new();
                    for (event, _) in events.by_ref() {
                        match event {
                            Event::End(pulldown_cmark::TagEnd::Link) => break,
                            Event::Text(text) | Event::Code(text) => label.push_str(&text),
                            Event::SoftBreak | Event::HardBreak => label.push(' '),
                            _ => {}
                        }
                    }
                    if label.is_empty() {
                        result.text.push_str(&source[span.clone()]);
                    } else {
                        result.append_link(&label, path);
                    }
                } else {
                    result.text.push_str(&source[span.clone()]);
                }
            }
            Event::Code(_)
            | Event::Html(_)
            | Event::InlineHtml(_)
            | Event::Start(Tag::CodeBlock(_) | Tag::Link { .. } | Tag::Image { .. }) => {
                result.append_prose(&source[end..span.start]);
                result.text.push_str(&source[span.clone()]);
            }
            _ => continue,
        }
        end = span.end;
    }
    result.append_prose(&source[end..]);
    (!result.links.is_empty()).then_some(result)
}

fn file_link(raw: &str) -> Option<FileLink> {
    if raw.contains(char::is_control) {
        return None;
    }
    let raw = raw.trim();
    let base = raw.split(['?', '#']).next()?;
    let fragment = raw
        .split_once('#')
        .map(|(_, f)| f.split('?').next().unwrap_or(""));
    static LINE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^(?:L|line-?)([1-9][0-9]*)\b").unwrap());
    let fragment_line = fragment.and_then(|f| {
        let decoded = percent_encoding::percent_decode_str(f).decode_utf8().ok()?;
        LINE.captures(&decoded)?
            .get(1)?
            .as_str()
            .parse::<usize>()
            .ok()
    });
    let is_uri = base
        .get(..5)
        .is_some_and(|s| s.eq_ignore_ascii_case("file:"));
    let decoded = if is_uri {
        let url = url::Url::parse(base).ok()?;
        if url.host_str().is_some_and(|h| h != "localhost") {
            return None;
        }
        url.to_file_path().ok()?.to_str()?.to_owned()
    } else {
        percent_encoding::percent_decode_str(base)
            .decode_utf8()
            .ok()?
            .into_owned()
    };
    static SUFFIX: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r":([0-9]+)(?::([0-9]+))?$").unwrap());
    let mut path_text = decoded.as_str();
    let mut position = None;
    if let Some(cap) = SUFFIX.captures(&decoded) {
        path_text = &decoded[..cap.get(0)?.start()];
        // A bare scheme or a drive prefix is not a filename.
        if !path_text.contains(['/', '\\', '.']) {
            return None;
        }
        let line = cap[1].parse::<std::num::NonZeroUsize>().ok()?.get();
        let column = match cap.get(2) {
            Some(n) => n.as_str().parse::<std::num::NonZeroUsize>().ok()?.get(),
            None => 1,
        };
        position = Some(crate::files::FilePosition { line, column });
    }
    let path = local_path(path_text)?;
    Some(FileLink {
        path,
        position: fragment_line
            .map(|line| crate::files::FilePosition { line, column: 1 })
            .or(position),
    })
}

impl Citations {
    fn append_link(&mut self, label: &str, path: FileLink) {
        let start = self.text.len();
        self.text.push_str(label);
        self.links.push((start..self.text.len(), path));
    }

    fn append_prose(&mut self, prose: &str) {
        static DIRECTIVE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r":{1,2}codex-file-citation\{([^{}]+)\}").unwrap());
        static ATTR: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r#"([\w-]+)\s*=\s*(?:"([^"]*)"|'([^']*)')"#).unwrap());
        let mut end = 0;
        for captures in DIRECTIVE.captures_iter(prose) {
            if captures[1].len() > 8192 {
                continue;
            }
            let Some(value) = ATTR
                .captures_iter(&captures[1])
                .filter(|a| &a[1] == "path")
                .last()
            else {
                continue;
            };
            let Some(raw) = value.get(2).or_else(|| value.get(3)).map(|m| m.as_str()) else {
                continue;
            };
            let Some(link) = file_link(raw) else { continue };
            let Some(label) = raw
                .rsplit(['/', '\\'])
                .next()
                .filter(|name| !name.is_empty())
            else {
                continue;
            };
            let whole = captures.get(0).unwrap();
            self.text.push_str(&prose[end..whole.start()]);
            self.append_link(label, link);
            end = whole.end();
        }
        self.text.push_str(&prose[end..]);
    }
}

fn local_path(raw: &str) -> Option<PathBuf> {
    let value = raw.trim();
    if value.is_empty() || raw.contains(char::is_control) || value.starts_with(['#', '?']) {
        return None;
    }
    let path = PathBuf::from(value);
    let text = path.to_str()?;
    let drive = text.as_bytes().get(1) == Some(&b':')
        && text.as_bytes()[0].is_ascii_alphabetic()
        && text
            .as_bytes()
            .get(2)
            .is_some_and(|b| matches!(b, b'/' | b'\\'));
    let rest = if drive { &text[2..] } else { text };
    if rest.contains(':')
        || text.contains(char::is_control)
        || text.starts_with(['/', '\\'])
            && text.get(1..).is_some_and(|s| s.starts_with(['/', '\\']))
    {
        return None;
    }
    if !rest
        .split(['/', '\\'])
        .filter(|s| !s.is_empty() && *s != "." && *s != "..")
        .all(crate::file_actions::name_ok)
    {
        return None;
    }
    Some(path)
}

pub(crate) fn destination(root: &Path, cwd: &Path, path: &Path) -> Result<PathBuf, String> {
    let path = crate::files::confined(root, &cwd.join(path))?;
    // Program, shortcut and macro formats refused by Vue's chatFileLinks.
    // Script formats that Vue can display as text use the internal editor here.
    const BLOCKED: &str = "exe com msi msp mst msix msixbundle appx appxbundle appinstaller app psc1 pssc vbe wsh wsc ws sct jse lnk scr pif cpl dll ocx sys drv website application appref-ms gadget msc scf jar settingcontent-ms library-ms searchconnector-ms diagcab chm hlp xll xlam ppam docm dotm xlsm xltm xlsb pptm potm ppsm iso img vhd vhdx";
    if path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| {
            BLOCKED
                .split_ascii_whitespace()
                .any(|blocked| blocked.eq_ignore_ascii_case(ext))
        })
    {
        return Err("This file type cannot be opened from chat.".into());
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renders_only_prose_and_preserves_byte_ranges() {
        let cite = r#":codex-file-citation{path="C:/My reports/été.pdf"
purpose="source"}"#;
        let source = format!("📄 {cite} and `{cite}`\n\n~~~text\n{cite}\n~~~\n\n:{cite}");
        let out = parse("Codex · msg", &source).unwrap();
        assert_eq!(out.links.len(), 2);
        assert!(out.text.contains(&format!("`{cite}`")));
        assert!(out.text.contains(&format!("~~~text\n{cite}")));
        for (range, path) in out.links {
            assert_eq!(&out.text[range], "été.pdf");
            assert_eq!(path.path, Path::new("C:/My reports/été.pdf"));
        }
        assert!(parse("You", &source).is_none());
    }
    #[test]
    fn markdown_labels_and_directives_share_the_original_text_order() {
        let source = r#"Before [**first** report](<docs/my report.pdf> "Title"), :codex-file-citation{path='two.pdf'}, and [third](docs/three%20four.txt).
`[example](ignore.pdf)` ![image](ignore.png) [website](https://example.org) [network](%2F%2Fserver/share)"#;
        let parsed = parse("Codex", source).unwrap();
        let links: Vec<_> = parsed
            .links
            .iter()
            .map(|(r, p)| (&parsed.text[r.clone()], p.path.as_path()))
            .collect();
        assert_eq!(
            links,
            [
                ("first report", Path::new("docs/my report.pdf")),
                ("two.pdf", Path::new("two.pdf")),
                ("third", Path::new("docs/three four.txt"))
            ]
        );
        assert!(parsed.text.ends_with("`[example](ignore.pdf)` ![image](ignore.png) [website](https://example.org) [network](%2F%2Fserver/share)"));
        let windows = parse("Codex", r"[report](C:\Reports\summary.pdf)").unwrap();
        assert_eq!(
            windows.links[0].1.path,
            Path::new(r"C:\Reports\summary.pdf")
        );
    }
    #[test]
    fn file_link_locations_keep_encoded_filename_separators_and_validate_coordinates() {
        use crate::files::FilePosition;
        for (raw, path, line, column) in [
            ("docs/a%23b.md:7:2", "docs/a#b.md", 7, 2),
            ("docs/a.md?mode=read#L18-L20", "docs/a.md", 18, 1),
            ("docs/a.md#%6Cine-3", "docs/a.md", 3, 1),
            ("main.rs:4", "main.rs", 4, 1),
        ] {
            let link = file_link(raw).unwrap();
            assert_eq!(link.path, Path::new(path));
            assert_eq!(link.position, Some(FilePosition { line, column }));
        }
        for invalid in [
            "a.md:0",
            "a.md:7:0",
            "C:1",
            "http:80",
            "file://server/a.md#L1",
        ] {
            assert!(file_link(invalid).is_none(), "{invalid}");
        }
    }

    #[test]
    fn invalid_destinations_remain_literal_and_outside_files_cannot_open() {
        for path in [
            "javascript:alert(1)",
            "https://example.org/a",
            "//server/share/a",
            r"\\server\share\a",
            "file://server/share/a",
            "C:relative",
            "docs/NUL.txt",
            "report.pdf.",
            "file:///a%0Ab",
        ] {
            assert!(
                parse("Codex", &format!(":codex-file-citation{{path='{path}'}}")).is_none(),
                "{path}"
            );
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("report.pdf"), []).unwrap();
        std::fs::write(dir.path().join("outside"), []).unwrap();
        assert_eq!(
            destination(&root, &root.join("sub"), Path::new("../report.pdf")).unwrap(),
            root.join("report.pdf").canonicalize().unwrap()
        );
        assert!(destination(&root, &root, Path::new("../outside")).is_err());
    }
}
