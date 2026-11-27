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

pub(crate) fn parse(role: &str, source: &str) -> Option<Citations> {
    if !(role == "Codex" || role.starts_with("Codex · "))
        || !source.contains(":codex-file-citation{")
    {
        return None;
    }
    static DIRECTIVE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r":{1,2}codex-file-citation\{([^{}]+)\}").unwrap());
    static ATTR: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"([\w-]+)\s*=\s*(?:"([^"]*)"|'([^']*)')"#).unwrap());
    let mut protected = Parser::new(source)
        .into_offset_iter()
        .filter_map(|(event, span)| {
            matches!(
                event,
                Event::Code(_)
                    | Event::Html(_)
                    | Event::InlineHtml(_)
                    | Event::Start(Tag::CodeBlock(_) | Tag::Link { .. } | Tag::Image { .. })
            )
            .then_some(span)
        })
        .peekable();
    let mut result = Citations {
        text: String::new(),
        links: vec![],
    };
    let mut end = 0;
    for captures in DIRECTIVE.captures_iter(source) {
        if captures[1].len() > 8192 {
            continue;
        }
        let whole = captures.get(0)?;
        while protected
            .next_if(|span| span.end <= whole.start())
            .is_some()
        {}
        if protected
            .peek()
            .is_some_and(|span| span.start < whole.end())
        {
            continue;
        }
        let Some(value) = ATTR
            .captures_iter(&captures[1])
            .filter(|a| &a[1] == "path")
            .last()
        else {
            continue;
        };
        let raw = value.get(2).or_else(|| value.get(3))?.as_str();
        let Some(path) = local_path(raw) else {
            continue;
        };
        let Some(name) = path
            .to_str()
            .and_then(|p| p.rsplit(['/', '\\']).next())
            .filter(|n| !n.is_empty())
        else {
            continue;
        };
        result.text.push_str(&source[end..whole.start()]);
        let start = result.text.len();
        result.text.push_str(name);
        result.links.push((start..result.text.len(), path));
        end = whole.end();
    }
    if result.links.is_empty() {
        return None;
    }
    result.text.push_str(&source[end..]);
    Some(result)
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
