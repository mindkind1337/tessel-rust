//! Isolated, script-restricted HTML child view hosted inside the GPUI document tab.
use super::project_page::Pages;
use crate::{app::gpui_bridge::GpuiAction, files::Document};
use std::{cell::RefCell, rc::Rc};
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

#[derive(Default)]
pub(crate) struct EditorPage {
    view: Option<WebView>,
    pages: Pages,
    context: Option<wry::WebContext>,
    profile: Option<tempfile::TempDir>,
    document: Option<u64>,
    rect: Option<wry::Rect>,
    origin: (f32, f32),
    hidden: bool,
    pending: Rc<RefCell<Option<LinkGesture>>>,
}
impl EditorPage {
    pub(crate) fn retain(&mut self, id: Option<u64>, obscured: bool) {
        if self.document != id {
            self.view.take();
            self.pages = Pages::default();
            self.document = None;
            self.rect = None;
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
            self.pages.open(&doc.root, &doc.path, &doc.text)?;
            view.reload().map_err(|e| e.to_string())
        } else {
            // Retry creating the child on the next paint if the earlier attempt failed.
            self.retain(None, false);
            Ok(())
        }
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
        let id = doc.instance_id();
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
            self.view = self.create(parent, doc, rect);
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
        doc: &Document,
        rect: wry::Rect,
    ) -> Option<WebView> {
        parent.window_handle().ok()?;
        let pages = self.pages.clone();
        let address = pages.open(&doc.root, &doc.path, &doc.text).ok()?;
        if self.context.is_none() {
            let profile = tempfile::tempdir().ok()?;
            self.context = Some(wry::WebContext::new(Some(profile.path().to_owned())));
            self.profile = Some(profile);
        }
        let base = Url::parse(&address).ok()?;
        let allowed = address.clone();
        let pending = self.pending.clone();
        let builder = WebViewBuilder::new_with_web_context(self.context.as_mut()?);
        pages
            .attach(builder)
            .ok()?
            .with_incognito(true)
            .with_focused(false)
            .with_clipboard(false)
            .with_devtools(false)
            .with_initialization_script(LINKS)
            .with_navigation_handler(move |u| u.split('#').next() == Some(allowed.as_str()))
            .with_download_started_handler(|_, _| false)
            .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
            .with_drag_drop_handler(|_| true)
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
