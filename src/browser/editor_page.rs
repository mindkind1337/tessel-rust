//! Isolated, script-restricted HTML child view hosted inside the GPUI document tab.
use super::project_page::Pages;
use crate::{app::gpui_bridge::GpuiAction, files::Document};
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};
use url::Url;
use wry::{WebView, WebViewBuilder, raw_window_handle::HasWindowHandle};

const LINKS: &str = r#"
for (const type of ['click','contextmenu']) document.addEventListener(type, event => {
  const a = event.target.closest?.('a[href]');
  if (!event.isTrusted || !a) return;
  const url = new URL(a.href);
  if (url.hash && url.origin === location.origin && url.pathname === location.pathname && url.search === location.search) return;
  event.preventDefault();
  window.ipc.postMessage(JSON.stringify([type, url.href, event.clientX, event.clientY]));
}, true);
"#;

struct LinkGesture {
    href: String,
    position: Option<(f32, f32)>,
}

fn hold_external(slot: &RefCell<Option<Url>>, base: &Url, address: &str) {
    if address.len() <= 8192
        && let Ok(url) = Url::parse(address)
        && matches!(url.scheme(), "http" | "https")
        && url.host().is_some()
        && url.origin() != base.origin()
        && url.username().is_empty()
        && url.password().is_none()
    {
        slot.replace(Some(url));
    }
}

#[derive(Default)]
pub(crate) struct EditorPage {
    view: Option<WebView>,
    pages: Pages,
    scripts: bool,
    pub(crate) reconsider: Option<u64>,
    context: Option<wry::WebContext>,
    profile: Option<tempfile::TempDir>,
    document: Option<u64>,
    observed: String,
    refresh_due: Option<Instant>,
    rect: Option<wry::Rect>,
    origin: (f32, f32),
    hidden: bool,
    incoming: Rc<RefCell<Option<Url>>>,
    held: Option<Url>,
    pending: Rc<RefCell<Option<LinkGesture>>>,
}
impl EditorPage {
    pub(crate) fn set_scripts(&mut self, scripts: bool) {
        if self.scripts != scripts {
            self.retain(None, false);
            self.scripts = scripts;
        }
    }
    pub(crate) fn retain(&mut self, id: Option<u64>, obscured: bool) {
        if self.document != id {
            self.reconsider = None;
            self.view.take();
            self.pages = Pages::editor(self.scripts);
            self.document = None;
            self.observed = String::new();
            self.refresh_due = None;
            self.rect = None;
            self.held = None;
            self.incoming = Default::default();
            self.pending = Default::default();
            self.hidden = false;
        } else if obscured
            && !self.hidden
            && let Some(view) = &self.view
        {
            self.hidden = view.set_visible(false).is_ok();
        }
    }
    pub(crate) fn reload(&mut self, doc: &Document) -> Result<(), String> {
        if self.document != Some(doc.instance_id()) {
            return Ok(());
        }
        if let Some(view) = &self.view {
            let address = self.pages.open(
                doc.tools.html_root.as_deref().unwrap_or(&doc.root),
                &doc.path,
                &doc.text,
            )?;
            // Trusted pages can navigate inside the project. Reload always belongs to
            // the document tab, even when its child currently displays another file.
            if view.url().is_ok_and(|current| current == address) {
                view.reload()
            } else {
                view.load_url(&address)
            }
            .map_err(|e| e.to_string())?;
            if self.observed != doc.text {
                self.observed.clone_from(&doc.text);
            }
            self.refresh_due = None;
            Ok(())
        } else {
            // Retry creating the child on the next paint if the earlier attempt failed.
            self.retain(None, false);
            Ok(())
        }
    }
    pub(crate) fn poll_draft(&mut self, doc: &Document, now: Instant) -> Result<bool, String> {
        if self.document != Some(doc.instance_id()) || self.view.is_none() || self.hidden {
            return Ok(false);
        }
        if self.observed != doc.text {
            self.observed.clone_from(&doc.text);
            self.refresh_due = Some(now + Duration::from_millis(350));
        }
        if self.refresh_due.is_some_and(|due| now >= due) {
            self.refresh_due = None;
            self.reload(doc)?;
            return Ok(true);
        }
        Ok(false)
    }
    #[cfg(all(test, feature = "visual-test"))]
    pub(crate) fn queue_external_fixture(&mut self, id: u64, url: Url) {
        self.document = Some(id);
        self.incoming.replace(Some(url));
    }
    pub(crate) fn poll_external(&mut self) -> bool {
        if let Some(url) = self.incoming.take() {
            self.held = Some(url);
            return true;
        }
        false
    }
    pub(crate) fn external_prompt(&self) -> Option<(u64, Url)> {
        Some((self.document?, self.held.clone()?))
    }
    pub(crate) fn resolve_external(&mut self, id: u64, shown: &Url) -> Option<Url> {
        if self.document != Some(id) || self.held.as_ref() != Some(shown) {
            return None;
        }
        self.held.take()
    }
    pub(crate) fn action(&self, active: u64) -> Option<GpuiAction> {
        let LinkGesture {
            href,
            position: pos,
        } = self.pending.borrow_mut().take()?;
        if self.document != Some(active) || self.hidden {
            return None;
        }
        Some(match pos {
            Some((x, y)) => {
                GpuiAction::HtmlLinkMenu(active, href, (self.origin.0 + x, self.origin.1 + y))
            }
            None => GpuiAction::FollowDocumentLink(href),
        })
    }
    pub(crate) fn sync(
        &mut self,
        parent: &impl HasWindowHandle,
        doc: &Document,
        bounds: eframe::egui::Rect,
        scale: f32,
    ) {
        let opening = self.document != Some(doc.instance_id());
        self.sync_source(parent, doc.instance_id(), bounds, scale, false, |pages| {
            pages.open(
                doc.tools.html_root.as_deref().unwrap_or(&doc.root),
                &doc.path,
                &doc.text,
            )
        });
        if opening && self.view.is_some() {
            self.observed.clone_from(&doc.text);
        }
    }
    pub(crate) fn sync_source(
        &mut self,
        parent: &impl HasWindowHandle,
        id: u64,
        bounds: eframe::egui::Rect,
        scale: f32,
        remote: bool,
        register: impl FnOnce(&Pages) -> Result<String, String>,
    ) {
        self.retain(Some(id), !bounds.is_positive());
        if !bounds.is_positive() {
            return;
        }
        self.origin = (bounds.min.x, bounds.min.y);
        let rect = wry::Rect {
            position: wry::dpi::PhysicalPosition::new(
                (bounds.min.x * scale).round() as i32,
                (bounds.min.y * scale).round() as i32,
            )
            .into(),
            size: wry::dpi::PhysicalSize::new(
                (bounds.width() * scale).round().max(1.) as u32,
                (bounds.height() * scale).round().max(1.) as u32,
            )
            .into(),
        };
        if self.document != Some(id) {
            self.document = Some(id);
            self.view = self.create(parent, rect, remote, register);
            self.rect = self.view.as_ref().map(|_| rect);
        }
        if let Some(view) = &self.view {
            if self.rect != Some(rect) && view.set_bounds(rect).is_ok() {
                self.rect = Some(rect)
            }
            if self.hidden && view.set_visible(true).is_ok() {
                self.hidden = false
            }
        }
    }
    fn create(
        &mut self,
        parent: &impl HasWindowHandle,
        rect: wry::Rect,
        remote: bool,
        register: impl FnOnce(&Pages) -> Result<String, String>,
    ) -> Option<WebView> {
        parent.window_handle().ok()?;
        let pages = self.pages.clone();
        let address = register(&pages).ok()?;
        if self.context.is_none() {
            let profile = tempfile::tempdir().ok()?;
            self.context = Some(wry::WebContext::new(Some(profile.path().to_owned())));
            self.profile = Some(profile);
        }
        let base = Url::parse(&address).ok()?;
        let allowed = address.clone();
        let native_links = remote || self.scripts;
        let pending = self.pending.clone();
        let builder = WebViewBuilder::new_with_web_context(self.context.as_mut()?);
        let builder = pages
            .attach(builder)
            .ok()?
            .with_incognito(true)
            .with_focused(false)
            .with_clipboard(false)
            .with_devtools(false)
            .with_navigation_handler({
                let base = base.clone();
                let incoming = self.incoming.clone();
                let origin = base.origin();
                move |u| {
                    hold_external(&incoming, &base, &u);
                    if native_links {
                        Url::parse(&u).is_ok_and(|url| {
                            url.origin() == origin
                                && url.username().is_empty()
                                && url.password().is_none()
                        })
                    } else {
                        u.split('#').next() == Some(&allowed)
                    }
                }
            })
            .with_download_started_handler(|_, _| false)
            .with_new_window_req_handler({
                let incoming = self.incoming.clone();
                let base = base.clone();
                move |address, _| {
                    hold_external(&incoming, &base, &address);
                    wry::NewWindowResponse::Deny
                }
            })
            .with_drag_drop_handler(|_| true);
        // The remote page browses within its isolated project origin using native links.
        // Only the local document uses the editor's application link menu.
        let builder = if native_links {
            builder
        } else {
            let incoming = self.incoming.clone();
            builder
                .with_initialization_script(LINKS)
                .with_ipc_handler(move |msg| {
                    if msg.body().len() > 16384
                        || msg.uri().authority().map(|a| a.as_str()) != base.host_str()
                    {
                        return;
                    }
                    let Ok((kind, raw, x, y)) =
                        serde_json::from_str::<(String, String, f32, f32)>(msg.body())
                    else {
                        return;
                    };
                    let Ok(target) = Url::parse(&raw) else { return };
                    if !matches!(target.scheme(), "http" | "https")
                        || !target.username().is_empty()
                        || target.password().is_some()
                    {
                        return;
                    }
                    if kind == "click" && target.origin() != base.origin() {
                        hold_external(&incoming, &base, &raw);
                        return;
                    }
                    let href = if target.origin() == base.origin() {
                        let Some(rel) = base.make_relative(&target) else {
                            return;
                        };
                        rel
                    } else {
                        raw
                    };
                    let pos = match kind.as_str() {
                        "click" => None,
                        "contextmenu" => Some((x, y)),
                        _ => return,
                    };
                    *pending.borrow_mut() = Some(LinkGesture {
                        href,
                        position: pos,
                    });
                })
        };
        builder
            .with_bounds(rect)
            .with_url(address)
            .build_as_child(parent)
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webview_test_support::{hidden_parent, pump, wait_for};
    #[test]
    #[ignore = "requires installed Windows WebView2 runtime"]
    fn native_editor_page_recreates_the_document_when_folder_trust_changes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("index.html");
        std::fs::write(
            &path,
            "<p>loaded</p><script type=module src='module.js'></script>",
        )
        .unwrap();
        std::fs::write(root.path().join("module.js"), "document.title='script ran'").unwrap();
        let doc = Document::open(root.path(), &path).unwrap();
        let parent = hidden_parent();
        let mut editor = EditorPage::default();
        let bounds = eframe::egui::Rect::from_min_size(
            eframe::egui::Pos2::ZERO,
            eframe::egui::vec2(320., 200.),
        );
        let mut previous = String::new();
        for trusted in [false, true, false] {
            editor.set_scripts(trusted);
            editor.sync(&parent, &doc, bounds, 1.);
            let view = editor.view.as_ref().unwrap();
            wait_for(
                view,
                if trusted {
                    "document.title === 'script ran'"
                } else {
                    "document.readyState === 'complete' && document.querySelector('p')?.textContent === 'loaded' && document.title !== 'script ran'"
                },
            );
            let address = view.url().unwrap();
            assert_ne!(address, previous);
            previous = address;
        }
    }
    #[test]
    fn held_destinations_are_external_web_links_without_credentials() {
        let incoming = RefCell::default();
        let base = Url::parse("http://preview.localhost/index.html").unwrap();
        for raw in [
            "http://preview.localhost/other",
            "file:///secret",
            "javascript:void(0)",
            "https://user:password@example.org/",
        ] {
            hold_external(&incoming, &base, raw);
            assert!(incoming.borrow().is_none());
        }
        hold_external(&incoming, &base, "https://example.org:444/report");
        assert_eq!(incoming.take().unwrap().port(), Some(444));
    }

    #[test]
    #[ignore = "requires installed Windows WebView2 runtime"]
    fn native_editor_page_holds_external_navigation_until_a_scoped_decision() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("index.html");
        std::fs::write(&path, "<h1>Index</h1>").unwrap();
        let doc = Document::open(root.path(), &path).unwrap();
        let parent = hidden_parent();
        let mut editor = EditorPage::default();
        let bounds = eframe::egui::Rect::from_min_size(
            eframe::egui::Pos2::ZERO,
            eframe::egui::vec2(300., 200.),
        );
        editor.sync(&parent, &doc, bounds, 1.);
        let view = editor.view.as_ref().unwrap();
        wait_for(
            view,
            "document.querySelector('h1')?.textContent === 'Index'",
        );
        view.evaluate_script("location.assign('https://example.invalid/report')")
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !editor.poll_external() {
            assert!(
                std::time::Instant::now() < deadline,
                "external navigation was never reported"
            );
            pump();
        }
        let (id, url) = editor.external_prompt().unwrap();
        assert_eq!(url.as_str(), "https://example.invalid/report");
        wait_for(
            editor.view.as_ref().unwrap(),
            "document.querySelector('h1')?.textContent === 'Index'",
        );
        assert!(editor.resolve_external(id.wrapping_add(1), &url).is_none());
        assert!(
            editor
                .resolve_external(id, &Url::parse("https://different.invalid/").unwrap())
                .is_none()
        );
        assert_eq!(editor.resolve_external(id, &url), Some(url));
        assert!(editor.external_prompt().is_none());
    }

    #[test]
    #[ignore = "requires installed Windows WebView2 runtime"]
    fn native_ssh_editor_renders_draft_and_keeps_navigation_in_the_project() {
        let host = crate::workspace::Host {
            alias: "fixture".into(),
            folder: "/site".into(),
            ..Default::default()
        };
        let parent = hidden_parent();
        let mut editor = EditorPage::default();
        let bounds = eframe::egui::Rect::from_min_size(
            eframe::egui::Pos2::ZERO,
            eframe::egui::vec2(300., 200.),
        );
        editor.sync_source(&parent,1,bounds,1.,true,|pages|pages.open_remote(host.clone(),"/site","/site/index.html",r#"<style>p{color:rgb(11,22,33)}</style><p>Draft</p><a href="next.html">Next</a><script>document.title='executed'</script>"#));
        let view = editor.view.as_ref().unwrap();
        wait_for(
            view,
            "document.querySelector('p')?.textContent === 'Draft' && getComputedStyle(document.querySelector('p')).color === 'rgb(11, 22, 33)' && document.title !== 'executed'",
        );
        editor
            .pages
            .open_remote(host, "/site", "/site/next.html", "<h2>Next</h2>")
            .unwrap();
        view.evaluate_script("document.querySelector('a').click()")
            .unwrap();
        wait_for(view, "document.querySelector('h2')?.textContent === 'Next'");
        view.go_back().unwrap();
        wait_for(
            view,
            "location.pathname === '/index.html' && document.querySelector('p')?.textContent === 'Draft'",
        );
    }

    #[test]
    #[ignore = "requires installed Windows WebView2 runtime"]
    fn native_editor_page_reload_keeps_unsaved_html_and_reads_changed_css() {
        let folder = tempfile::tempdir().unwrap();
        let index = folder.path().join("index.html");
        let css = folder.path().join("page.css");
        std::fs::write(&index, "original on disk").unwrap();
        std::fs::write(&css, "p { color: rgb(11,22,33) }").unwrap();
        let mut doc = Document::open(folder.path(), &index).unwrap();
        doc.text = r#"<link rel="stylesheet" href="page.css"><p>first draft</p>"#.into();
        let mut editor = EditorPage::default();
        let parent = hidden_parent();
        let bounds = eframe::egui::Rect::from_min_size(
            eframe::egui::Pos2::ZERO,
            eframe::egui::vec2(300., 200.),
        );
        editor.sync(&parent, &doc, bounds, 1.);
        let first = editor.view.as_ref().unwrap();
        wait_for(
            first,
            "document.querySelector('p')?.textContent === 'first draft' && getComputedStyle(document.querySelector('p')).color === 'rgb(11, 22, 33)'",
        );
        let address = first.url().unwrap();
        doc.text = doc.text.replace("first draft", "second draft");
        std::fs::write(&css, "p { color: rgb(44,55,66) }").unwrap();
        editor.reload(&doc).unwrap();
        let view = editor.view.as_ref().unwrap();
        wait_for(
            view,
            "document.querySelector('p')?.textContent === 'second draft' && getComputedStyle(document.querySelector('p')).color === 'rgb(44, 55, 66)'",
        );
        assert_eq!(view.url().unwrap(), address);
        assert_eq!(std::fs::read_to_string(index).unwrap(), "original on disk");
    }
    #[test]
    #[ignore = "requires installed Windows WebView2 runtime"]
    fn native_editor_page_reload_returns_from_linked_document_to_index_draft() {
        let folder = tempfile::tempdir().unwrap();
        let index = folder.path().join("index.html");
        std::fs::write(&index, "<p>saved index</p><a href='report.html'>Report</a>").unwrap();
        std::fs::write(folder.path().join("report.html"), "<h1>Report</h1>").unwrap();
        let mut doc = Document::open(folder.path(), &index).unwrap();
        let parent = hidden_parent();
        let bounds = eframe::egui::Rect::from_min_size(
            eframe::egui::Pos2::ZERO,
            eframe::egui::vec2(320., 200.),
        );
        let mut editor = EditorPage::default();
        editor.set_scripts(true);
        editor.sync(&parent, &doc, bounds, 1.);
        let view = editor.view.as_ref().unwrap();
        wait_for(
            view,
            "document.querySelector('a')?.textContent === 'Report'",
        );
        view.evaluate_script("document.querySelector('a').click()")
            .unwrap();
        wait_for(
            view,
            "location.pathname === '/report.html' && document.querySelector('h1')?.textContent === 'Report'",
        );
        doc.text = "<p>unsaved index</p>".into();
        editor.reload(&doc).unwrap();
        wait_for(
            editor.view.as_ref().unwrap(),
            "location.pathname === '/index.html' && document.querySelector('p')?.textContent === 'unsaved index'",
        );
        assert_eq!(
            std::fs::read_to_string(&index).unwrap(),
            "<p>saved index</p><a href='report.html'>Report</a>"
        );
    }

    #[test]
    #[ignore = "requires installed Windows WebView2 runtime"]
    fn native_editor_page_updates_after_the_latest_draft_settles() {
        let folder = tempfile::tempdir().unwrap();
        let index = folder.path().join("index.html");
        std::fs::write(&index, "<p>saved</p>").unwrap();
        let mut doc = Document::open(folder.path(), &index).unwrap();
        let parent = hidden_parent();
        let bounds = eframe::egui::Rect::from_min_size(
            eframe::egui::Pos2::ZERO,
            eframe::egui::vec2(320., 200.),
        );
        let mut editor = EditorPage::default();
        editor.sync(&parent, &doc, bounds, 1.);
        wait_for(
            editor.view.as_ref().unwrap(),
            "document.querySelector('p')?.textContent === 'saved'",
        );
        let start = Instant::now();
        doc.text = "<p>first</p>".into();
        assert!(!editor.poll_draft(&doc, start).unwrap());
        doc.text = "<p>latest</p>".into();
        assert!(
            !editor
                .poll_draft(&doc, start + Duration::from_millis(200))
                .unwrap()
        );
        assert!(
            !editor
                .poll_draft(&doc, start + Duration::from_millis(549))
                .unwrap()
        );
        assert!(
            editor
                .poll_draft(&doc, start + Duration::from_millis(550))
                .unwrap()
        );
        wait_for(
            editor.view.as_ref().unwrap(),
            "document.querySelector('p')?.textContent === 'latest'",
        );
        assert!(
            !editor
                .poll_draft(&doc, start + Duration::from_secs(1))
                .unwrap()
        );
        assert_eq!(std::fs::read_to_string(index).unwrap(), "<p>saved</p>");
    }
    #[test]
    #[ignore = "requires installed Windows WebView2 runtime"]
    fn native_editor_page_styles_and_routes_link_messages_without_leaving_index() {
        let root = tempfile::tempdir().unwrap();
        let index = root.path().join("index.html");
        std::fs::write(&index,r#"<style>a{display:block;position:absolute;inset:0;color:rgb(1,2,3)}</style><a href="report.txt">Report</a><script>document.title='executed'</script>"#).unwrap();
        std::fs::write(root.path().join("report.txt"), "fixture").unwrap();
        let doc = Document::open(root.path(), &index).unwrap();
        let parent = hidden_parent();
        let mut editor = EditorPage::default();
        let bounds = eframe::egui::Rect::from_min_size(
            eframe::egui::pos2(12., 15.),
            eframe::egui::vec2(300., 200.),
        );
        editor.sync(&parent, &doc, bounds, 1.);
        let view = editor.view.as_ref().unwrap();
        wait_for(
            view,
            "document.querySelector('a') && getComputedStyle(document.querySelector('a')).color === 'rgb(1, 2, 3)' && document.title !== 'executed'",
        );
        wait_for(
            view,
            "document.elementFromPoint(25,30)?.closest('a') ? true : [innerWidth,innerHeight,document.elementFromPoint(25,30)?.outerHTML]",
        );
        for (button, kind) in [("left", "click"), ("right", "contextmenu")] {
            view.evaluate_script(&format!("window.ipc.postMessage(JSON.stringify(['{kind}', document.querySelector('a').href,25,30]))")).unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let action = loop {
                pump();
                if let Some(action) = editor.action(doc.instance_id()) {
                    break action;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "missing {button} link gesture"
                );
            };
            match action {
                GpuiAction::FollowDocumentLink(path) if button == "left" => {
                    assert_eq!(path, "report.txt")
                }
                GpuiAction::HtmlLinkMenu(id, path, _) if button == "right" => {
                    assert_eq!(id, doc.instance_id());
                    assert_eq!(path, "report.txt");
                }
                _ => panic!("unexpected link action"),
            }
        }
        wait_for(view, "location.pathname.endsWith('/index.html')");
        editor.retain(Some(doc.instance_id()), true);
        assert!(editor.hidden);
        editor.sync(&parent, &doc, bounds, 1.);
        assert!(!editor.hidden);
        editor.retain(None, false);
        assert!(editor.view.is_none());
    }
}
