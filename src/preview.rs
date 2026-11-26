//! Native readable document view: no browser, script execution or resource fetching.
use eframe::egui;
use std::path::{Path, PathBuf};
pub type ReadableBlock = (bool, Vec<(String, Option<String>)>);

#[derive(Default)]
pub struct Block {
    heading: bool,
    spans: Vec<(String, Option<String>)>,
}
#[derive(Default)]
pub struct Preview {
    blocks: Vec<Block>,
}
impl Preview {
    pub fn parse(source: &str, markdown: bool) -> Self {
        if source.len() > 1024 * 1024 {
            return Self {
                blocks: vec![Block {
                    heading: false,
                    spans: vec![(
                        "Readable preview limit is 1 MiB. Use Source to view the full document."
                            .into(),
                        None,
                    )],
                }],
            };
        }
        let mut html = String::new();
        if markdown {
            pulldown_cmark::html::push_html(
                &mut html,
                pulldown_cmark::Parser::new_ext(
                    source,
                    pulldown_cmark::Options::ENABLE_TABLES
                        | pulldown_cmark::Options::ENABLE_STRIKETHROUGH,
                ),
            );
        }
        let doc = scraper::Html::parse_document(if markdown { &html } else { source });
        let mut blocks = Vec::<Block>::new();
        let mut previous = None;
        for node in doc.tree.root().descendants() {
            let Some(text) = node.value().as_text() else {
                continue;
            };
            if text.trim().is_empty() {
                continue;
            }
            let parents: Vec<_> = node
                .ancestors()
                .take(130)
                .filter_map(scraper::ElementRef::wrap)
                .collect();
            if parents.len() > 128 || blocks.len() > 2000 {
                return Self {
                    blocks: vec![Block {
                        heading: false,
                        spans: vec![(
                            "Document is too complex for readable preview. Use Source.".into(),
                            None,
                        )],
                    }],
                };
            }
            if parents.iter().any(|e| {
                matches!(
                    e.value().name(),
                    "head"
                        | "script"
                        | "style"
                        | "template"
                        | "noscript"
                        | "iframe"
                        | "object"
                        | "svg"
                )
            }) {
                continue;
            }
            let block = parents.iter().find(|e| {
                matches!(
                    e.value().name(),
                    "p" | "li"
                        | "div"
                        | "tr"
                        | "h1"
                        | "h2"
                        | "h3"
                        | "h4"
                        | "h5"
                        | "h6"
                        | "pre"
                        | "blockquote"
                )
            });
            let id = block.map(|e| e.id());
            if blocks.is_empty() || id != previous {
                blocks.push(Block {
                    heading: block.is_some_and(|e| {
                        matches!(e.value().name(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
                    }),
                    spans: vec![],
                });
                previous = id;
            }
            let link = parents
                .iter()
                .find(|e| e.value().name() == "a")
                .and_then(|e| e.value().attr("href"))
                .map(str::to_string);
            let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
            blocks.last_mut().unwrap().spans.push((text, link));
        }
        Self { blocks }
    }
    pub fn blocks(&self) -> Vec<ReadableBlock> {
        self.blocks
            .iter()
            .map(|b| (b.heading, b.spans.clone()))
            .collect()
    }
    pub fn plain(&self) -> String {
        self.blocks
            .iter()
            .map(|b| {
                b.spans
                    .iter()
                    .map(|s| s.0.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }
    pub fn show(&self, ui: &mut egui::Ui) -> Option<String> {
        let mut clicked = None;
        for block in &self.blocks {
            ui.horizontal_wrapped(|ui| {
                for (text, href) in &block.spans {
                    let mut rich = egui::RichText::new(text);
                    if block.heading {
                        rich = rich.heading().strong();
                    }
                    if let Some(href) = href {
                        let response = ui.link(rich).on_hover_text(href);
                        if response.clicked() {
                            clicked = Some(href.clone());
                        }
                        response.context_menu(|ui| {
                            if ui.button("Open link in another tab").clicked() {
                                clicked = Some(href.clone());
                                ui.close();
                            }
                            if ui.button("Copy link").clicked() {
                                ui.ctx().copy_text(href.clone());
                                ui.close();
                            }
                        });
                    } else {
                        ui.add(egui::Label::new(rich).selectable(true));
                    }
                }
            });
            ui.add_space(8.0);
        }
        clicked
    }
}
/// Extract body text using the same line and table boundaries as HtmlView.vue.
pub fn html_plain_text(source: &str) -> String {
    let doc = scraper::Html::parse_document(source);
    let Some(body) = doc
        .root_element()
        .child_elements()
        .find(|e| e.value().name() == "body")
    else {
        return String::new();
    };
    let mut text = String::new();
    let mut node = *body;
    loop {
        let tag = node.value().as_element().map(|e| e.name());
        let skip = matches!(
            tag,
            Some("script" | "style" | "iframe" | "template" | "noscript" | "noembed" | "noframes")
        );
        if !skip {
            if let Some(value) = node.value().as_text() {
                text.push_str(value);
            } else if tag == Some("br") {
                text.push('\n');
            }
            if let Some(child) = node.first_child() {
                node = child;
                continue;
            }
        }
        // Walk back out of each completed element, without recursing on document depth.
        loop {
            if node.id() == body.id() {
                static SPACES: std::sync::LazyLock<regex::Regex> =
                    std::sync::LazyLock::new(|| regex::Regex::new(r"[ \t]+\n").unwrap());
                static BREAKS: std::sync::LazyLock<regex::Regex> =
                    std::sync::LazyLock::new(|| regex::Regex::new(r"\n{3,}").unwrap());
                return BREAKS
                    .replace_all(&SPACES.replace_all(&text, "\n"), "\n\n")
                    .trim()
                    .to_owned();
            }
            match node.value().as_element().map(|e| e.name()) {
                Some(
                    "p" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "li" | "tr" | "section"
                    | "article" | "pre",
                ) => text.push('\n'),
                Some("td" | "th") => text.push('\t'),
                _ => {}
            }
            if let Some(next) = node.next_sibling() {
                node = next;
                break;
            }
            node = node.parent().expect("all body descendants have parents");
        }
    }
}
pub enum Link {
    File(PathBuf),
    Web(String),
}
pub fn resolve(root: &Path, source: &Path, href: &str) -> Result<Link, String> {
    if href.trim().starts_with('#') {
        return Err("Anchor navigation is not implemented yet".into());
    }
    let base = url::Url::from_file_path(source).map_err(|_| "Invalid document location")?;
    let mut url = base.join(href).map_err(|_| "Invalid document link")?;
    match url.scheme() {
        "https" | "http" if url.username().is_empty() && url.password().is_none() => {
            Ok(Link::Web(url.into()))
        }
        "file" if url.host_str().is_none_or(|h| h == "localhost") => {
            url.set_fragment(None);
            url.set_query(None);
            let path = url.to_file_path().map_err(|_| "Invalid local file link")?;
            Ok(Link::File(crate::files::confined(root, &path)?))
        }
        _ => Err("This link type is not allowed in document previews".into()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn html_plain_text_preserves_inline_spacing_table_columns_and_breaks() {
        let html = "<title>Hidden</title><p>A<b>B</b>  C &amp; D<br>Line</p><table><tr><th>One</th><th>Two</th></tr><tr><td>3</td><td>4</td></tr></table><script>hidden()</script><style>body{}</style><pre>  End</pre>";
        assert_eq!(
            html_plain_text(html),
            "AB  C & D\nLine\nOne\tTwo\n3\t4\n  End"
        );
    }
    #[test]
    fn readable_content_never_includes_scripts_or_loads_resources() {
        let preview = Preview::parse(
            r#"<h1>Title</h1><p>Hello <b>world</b> &amp; friends</p><script>secret()</script><style>@import 'https://evil'</style><img src='https://evil/a'><a href='next.html'>Next</a>"#,
            false,
        );
        assert_eq!(preview.plain(), "Title\n\nHello world & friends\n\nNext");
        let md = Preview::parse("# Title\n\n[Next](next.html)", true);
        assert_eq!(md.plain(), "Title\n\nNext");
    }
    #[test]
    fn links_stay_in_project_and_deny_native_or_network_paths() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("index.html");
        let target = dir.path().join("next file.html");
        std::fs::write(&source, "").unwrap();
        std::fs::write(&target, "").unwrap();
        let source = source.canonicalize().unwrap();
        assert!(matches!(
            resolve(dir.path(), &source, "next%20file.html"),
            Ok(Link::File(_))
        ));
        for href in [
            "javascript:alert(1)",
            "file://server/share/secret",
            "//server/share/secret",
            "../outside",
            "data:text/html,hi",
        ] {
            assert!(resolve(dir.path(), &source, href).is_err(), "{href}");
        }
    }
}
