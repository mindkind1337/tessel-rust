use pulldown_cmark::{Event, Parser, Tag};
use regex::Regex;
use std::{
    ops::Range,
    path::{Path, PathBuf},
    sync::LazyLock,
};

pub(crate) struct Citations {
    pub text: String,
    pub links: Vec<(Range<usize>, PathBuf)>,
}

pub(crate) fn show_message(
    ui: &mut eframe::egui::Ui,
    role: &str,
    text: &str,
    clicked: &mut Option<PathBuf>,
) {
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

pub(crate) fn parse(role: &str, source: &str) -> Option<Citations> {
    if !(matches!(role, "Codex" | "Claude") || role.starts_with("Codex · "))
        || !(source.contains(":codex-file-citation{") || source.contains("]("))
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
                if let Some(path) = markdown_path(&dest_url) {
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

fn markdown_path(raw: &str) -> Option<PathBuf> {
    if raw
        .get(..5)
        .is_some_and(|s| s.eq_ignore_ascii_case("file:"))
    {
        return local_path(raw);
    }
    let decoded = percent_encoding::percent_decode_str(raw)
        .decode_utf8()
        .ok()?;
    local_path(&decoded)
}

impl Citations {
    fn append_link(&mut self, label: &str, path: PathBuf) {
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
            let Some(path) = value
                .get(2)
                .or_else(|| value.get(3))
                .and_then(|m| local_path(m.as_str()))
            else {
                continue;
            };
            let Some(name) = path
                .to_str()
                .and_then(|p| p.rsplit(['/', '\\']).next())
                .filter(|n| !n.is_empty())
            else {
                continue;
            };
            let whole = captures.get(0).unwrap();
            self.text.push_str(&prose[end..whole.start()]);
            let name = name.to_owned();
            self.append_link(&name, path);
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
    let path = if value
        .get(..5)
        .is_some_and(|s| s.eq_ignore_ascii_case("file:"))
    {
        let url = url::Url::parse(value).ok()?;
        if url.host_str().is_some_and(|h| h != "localhost") {
            return None;
        }
        url.to_file_path().ok()?
    } else {
        PathBuf::from(value)
    };
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
            assert_eq!(path, Path::new("C:/My reports/été.pdf"));
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
            .map(|(r, p)| (&parsed.text[r.clone()], p.as_path()))
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
        assert_eq!(windows.links[0].1, Path::new(r"C:\Reports\summary.pdf"));
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
