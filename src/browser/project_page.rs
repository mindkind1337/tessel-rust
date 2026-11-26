//! Restricted project websites for the owner's embedded Windows browser.
use std::{
    borrow::Cow,
    collections::VecDeque,
    fs::File,
    io::{Read, Seek, SeekFrom},
    os::windows::{ffi::OsStringExt, io::AsRawHandle},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
};
use windows::Win32::{
    Foundation::HANDLE,
    Storage::FileSystem::{FILE_NAME_NORMALIZED, GetFinalPathNameByHandleW},
};
use wry::http::{Request, Response};

const SCHEME: &str = "tesselpreview";
const LIMIT: u64 = 50 * 1024 * 1024;
const POLICY: &str = "default-src 'none'; script-src 'none'; style-src 'self' 'unsafe-inline' https:; img-src 'self' data: blob: https:; font-src 'self' data: https:; media-src 'self' data: blob: https:; connect-src 'none'; frame-src 'none'; worker-src 'none'; object-src 'none'; base-uri 'self'; form-action 'none'; frame-ancestors 'none'";

const MAX_SITES: usize = 50;
const MAX_DRAFTS: usize = 20;

#[derive(Clone)]
struct Site {
    authority: String,
    root: PathBuf,
    remote: Option<crate::workspace::Host>,
    drafts: VecDeque<(PathBuf, Arc<[u8]>)>,
}

#[derive(Clone, Default)]
pub(super) struct Pages(Arc<Mutex<VecDeque<Arc<Site>>>>);

fn segment_ok(s: &str) -> bool {
    let stem = s
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_ascii_uppercase();
    let device_port = stem
        .strip_prefix("COM")
        .or_else(|| stem.strip_prefix("LPT"))
        .is_some_and(|n| {
            matches!(
                n,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        });
    !device_port
        && !matches!(
            stem.as_str(),
            "CON" | "CONIN$" | "CONOUT$" | "NUL" | "PRN" | "AUX"
        )
        && !s.is_empty()
        && !s.starts_with('.')
        && !s.ends_with([' ', '.'])
        && !s.contains(['/', '\\', ':'])
        && !s.chars().any(char::is_control)
}
fn visible_relative(root: &Path, file: &Path) -> bool {
    file.strip_prefix(root)
        .is_ok_and(|rel| rel.iter().all(|part| part.to_str().is_some_and(segment_ok)))
}
fn final_name(file: &File) -> Option<PathBuf> {
    let mut name = vec![0u16; 32768];
    // The handle stays owned by file throughout the call and the slice is writable.
    let len = unsafe {
        GetFinalPathNameByHandleW(
            HANDLE(file.as_raw_handle()),
            &mut name,
            FILE_NAME_NORMALIZED,
        )
    } as usize;
    (len > 0 && len < name.len()).then(|| std::ffi::OsString::from_wide(&name[..len]).into())
}
fn mime(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "bmp" => "image/bmp",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "pdf" => "application/pdf",
        "txt" | "md" => "text/plain; charset=utf-8",
        ext => crate::files::playback::mime(ext).unwrap_or("application/octet-stream"),
    }
}
type Reply = Response<Cow<'static, [u8]>>;
fn response(status: u16, kind: &str, size: u64, bytes: Vec<u8>) -> Reply {
    Response::builder()
        .status(status)
        .header("Content-Type", kind)
        .header("Content-Length", size)
        .header("Cache-Control", "no-store")
        .header("Content-Security-Policy", POLICY)
        .header("X-Content-Type-Options", "nosniff")
        .header("Referrer-Policy", "no-referrer")
        .header("Cross-Origin-Resource-Policy", "same-origin")
        .body(Cow::Owned(bytes))
        .unwrap()
}
fn empty(status: u16) -> Reply {
    response(status, "text/plain", 0, Vec::new())
}

fn asset_response(
    mut file: File,
    kind: &str,
    size: u64,
    head: bool,
    range: Option<&wry::http::HeaderValue>,
) -> Reply {
    use wry::http::{HeaderValue, header};
    let range = range.and_then(|value| value.to_str().ok()).filter(|value| {
        value
            .strip_prefix("bytes=")
            .and_then(|bounds| bounds.split_once('-'))
            .is_some_and(|(a, b)| {
                !(a.is_empty() && b.is_empty())
                    && a.bytes().chain(b.bytes()).all(|c| c.is_ascii_digit())
            })
    });
    let partial = range.is_some();
    let bytes = if let Some(value) = range {
        let Some(range) = crate::files::playback::native::byte_range(value, size) else {
            let mut reply = empty(416);
            reply.headers_mut().insert(
                header::CONTENT_RANGE,
                format!("bytes */{size}").parse().unwrap(),
            );
            return reply;
        };
        // WebView2 buffers each protocol reply. A player can request the next chunk.
        range.start..range.end.min(range.start.saturating_add(LIMIT))
    } else {
        if !head && size > LIMIT {
            return empty(413);
        }
        0..size
    };
    let len = bytes.end - bytes.start;
    let mut body = Vec::new();
    if !head
        && len > 0
        && (file.seek(SeekFrom::Start(bytes.start)).is_err()
            || file.take(len).read_to_end(&mut body).is_err()
            || body.len() as u64 != len)
    {
        return empty(404);
    }
    let mut reply = response(if partial { 206 } else { 200 }, kind, len, body);
    reply
        .headers_mut()
        .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    if partial {
        reply.headers_mut().insert(
            header::CONTENT_RANGE,
            format!("bytes {}-{}/{size}", bytes.start, bytes.end - 1)
                .parse()
                .unwrap(),
        );
    }
    reply
}

impl Pages {
    pub(super) fn open(&self, root: &Path, file: &Path, text: &str) -> Result<String, String> {
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        let entry = crate::files::confined(&root, file)?;
        if !visible_relative(&root, &entry)
            || mime(&entry) != "text/html; charset=utf-8"
            || text.len() > crate::files::MAX_TEXT
        {
            return Err("This project page cannot be previewed.".into());
        }
        self.register(root, entry, text, None)
    }
    pub(super) fn open_remote(
        &self,
        host: crate::workspace::Host,
        root: &str,
        file: &str,
        text: &str,
    ) -> Result<String, String> {
        if crate::remote::preview_relative(root, file).is_none()
            || mime(Path::new(file)) != "text/html; charset=utf-8"
            || text.len() > crate::files::MAX_TEXT
        {
            return Err("This remote page cannot be previewed.".into());
        }
        self.register(root.into(), file.into(), text, Some(host))
    }
    fn register(
        &self,
        root: PathBuf,
        entry: PathBuf,
        text: &str,
        remote: Option<crate::workspace::Host>,
    ) -> Result<String, String> {
        let mut sites = self.0.lock().map_err(|_| "Project preview unavailable")?;
        let mut site = if let Some(i) = sites.iter().position(|s| {
            s.root == root
                && s.remote.as_ref().map(|h| (&h.alias, &h.folder))
                    == remote.as_ref().map(|h| (&h.alias, &h.folder))
        }) {
            sites.remove(i).unwrap()
        } else {
            let mut nonce = [0u8; 16];
            getrandom::fill(&mut nonce).map_err(|_| "Cannot create preview origin")?;
            if sites.len() == MAX_SITES {
                sites.pop_front();
            }
            Arc::new(Site {
                authority: format!(
                    "{}{:032x}.localhost",
                    if remote.is_some() { "ssh-" } else { "" },
                    u128::from_be_bytes(nonce)
                ),
                root,
                remote,
                drafts: VecDeque::new(),
            })
        };
        let current = Arc::make_mut(&mut site);
        let mut url = url::Url::parse(&format!("http://{SCHEME}.{}/", current.authority)).unwrap();
        url.path_segments_mut().unwrap().pop_if_empty().extend(
            entry
                .strip_prefix(&current.root)
                .unwrap()
                .iter()
                .map(|s| s.to_str().unwrap()),
        );
        current.drafts.retain(|(path, _)| path != &entry);
        if current.drafts.len() == MAX_DRAFTS {
            current.drafts.pop_front();
        }
        current.drafts.push_back((entry, text.as_bytes().into()));
        sites.push_back(site);
        Ok(url.into())
    }
    #[cfg(test)]
    fn serve(&self, req: Request<Vec<u8>>) -> Reply {
        self.serve_with(req, crate::remote::execute)
    }
    fn serve_with(
        &self,
        req: Request<Vec<u8>>,
        read_remote: impl FnOnce(
            crate::workspace::Host,
            crate::remote::Action,
        ) -> Result<crate::remote::Reply, String>,
    ) -> Reply {
        let head = req.method() == "HEAD";
        if !head && req.method() != "GET" {
            return empty(405);
        }
        let Some(authority) = req.uri().authority() else {
            return empty(404);
        };
        let Some(site) = self.0.lock().ok().and_then(|mut sites| {
            let i = sites
                .iter()
                .position(|site| site.authority == authority.as_str())?;
            let site = sites.remove(i)?;
            sites.push_back(site.clone());
            Some(site)
        }) else {
            return empty(404);
        };
        let mut parts = Vec::new();
        for raw in req.uri().path().split('/').filter(|s| !s.is_empty()) {
            if raw.bytes().enumerate().any(|(i, b)| {
                b == b'%'
                    && !raw
                        .as_bytes()
                        .get(i + 1..i + 3)
                        .is_some_and(|digits| digits.iter().all(u8::is_ascii_hexdigit))
            }) {
                return empty(404);
            }
            let Ok(part) = percent_encoding::percent_decode_str(raw).decode_utf8() else {
                return empty(404);
            };
            if part.starts_with('.')
                || part.contains(['/', '\\'])
                || part.chars().any(char::is_control)
                || (site.remote.is_none() && !segment_ok(&part))
            {
                return empty(404);
            }
            parts.push(part);
        }
        if let Some(host) = &site.remote {
            if parts.is_empty() || req.uri().path().ends_with('/') {
                parts.push(Cow::Borrowed("index.html"))
            }
            let root = site.root.to_str().unwrap();
            let requested = format!("{}/{}", root.trim_end_matches('/'), parts.join("/"));
            let path = Path::new(&requested);
            if let Some((_, draft)) = site.drafts.iter().find(|(p, _)| p == path) {
                return response(
                    200,
                    mime(path),
                    draft.len() as u64,
                    if head { vec![] } else { draft.to_vec() },
                );
            }
            if crate::remote::preview_relative(root, &requested).is_none() {
                return empty(404);
            }
            if let Ok(crate::remote::Reply::File {
                root: actual,
                path: resolved,
                bytes,
            }) = read_remote(
                host.clone(),
                crate::remote::Action::PreviewAsset {
                    expected_root: root.into(),
                    path: requested,
                },
            ) && actual == root
                && crate::remote::preview_relative(root, &resolved).is_some()
            {
                return response(
                    200,
                    mime(Path::new(&resolved)),
                    bytes.len() as u64,
                    if head { vec![] } else { bytes },
                );
            }
            return empty(404);
        }
        let mut path = site.root.clone();
        path.extend(parts.iter().map(|s| s.as_ref()));
        if path.is_dir() {
            path.push("index.html")
        }
        let Ok(target) = crate::files::confined(&site.root, &path) else {
            return empty(404);
        };
        if !visible_relative(&site.root, &target) {
            return empty(404);
        }
        let Ok(file) = File::open(&target) else {
            return empty(404);
        };
        if !final_name(&file)
            .is_some_and(|name| name == target && visible_relative(&site.root, &name))
        {
            return empty(404);
        }
        let Ok(meta) = file.metadata() else {
            return empty(404);
        };
        if !meta.is_file() {
            return empty(404);
        }
        let kind = mime(&target);
        if let Some((_, draft)) = site.drafts.iter().find(|(path, _)| *path == target) {
            return response(
                200,
                kind,
                draft.len() as u64,
                if head { vec![] } else { draft.to_vec() },
            );
        }
        asset_response(file, kind, meta.len(), head, req.headers().get("Range"))
    }
    pub(super) fn attach<'a>(
        &self,
        builder: wry::WebViewBuilder<'a>,
    ) -> Result<wry::WebViewBuilder<'a>, String> {
        self.attach_with(builder, crate::remote::execute)
    }
    fn attach_with<'a>(
        &self,
        builder: wry::WebViewBuilder<'a>,
        read_remote: fn(
            crate::workspace::Host,
            crate::remote::Action,
        ) -> Result<crate::remote::Reply, String>,
    ) -> Result<wry::WebViewBuilder<'a>, String> {
        let worker = move |pages: Pages| -> Result<_, String> {
            let (send, recv) =
                mpsc::sync_channel::<(Request<Vec<u8>>, wry::RequestAsyncResponder)>(64);
            std::thread::Builder::new()
                .name("project-page".into())
                .spawn(move || {
                    for (req, responder) in recv {
                        responder.respond(pages.serve_with(req, read_remote));
                    }
                })
                .map_err(|e| e.to_string())?;
            Ok(send)
        };
        let local = worker(self.clone())?;
        let remote = std::sync::OnceLock::new();
        let pages = self.clone();
        Ok(
            builder.with_asynchronous_custom_protocol(SCHEME.into(), move |_, req, responder| {
                let send = if req.uri().host().is_some_and(|h| h.starts_with("ssh-")) {
                    let Ok(send) = remote.get_or_init(|| worker(pages.clone())) else {
                        responder.respond(empty(503));
                        return;
                    };
                    send
                } else {
                    &local
                };
                if let Err(err) = send.try_send((req, responder)) {
                    let (_, responder) = match err {
                        mpsc::TrySendError::Full(item) | mpsc::TrySendError::Disconnected(item) => {
                            item
                        }
                    };
                    responder.respond(empty(503));
                }
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ssh_preview_keeps_drafts_and_assets_scoped_to_the_selected_host_and_posix_root() {
        let pages = Pages::default();
        let register = |alias: &str| {
            pages
                .open_remote(
                    crate::workspace::Host {
                        alias: alias.into(),
                        folder: "/site".into(),
                        ..Default::default()
                    },
                    "/site",
                    "/site/pages/index.html",
                    alias,
                )
                .unwrap()
                .replace("http://tesselpreview.", "tesselpreview://")
        };
        for alias in ["host-a", "host-b"] {
            let entry = register(alias);
            let base = entry.strip_suffix("/pages/index.html").unwrap();
            let get = |url: String| {
                pages.serve_with(Request::get(url).body(vec![]).unwrap(), |host, action| {
                    assert_eq!(host.alias, alias);
                    let crate::remote::Action::PreviewAsset {
                        expected_root,
                        path,
                    } = action
                    else {
                        panic!("wrong SFTP operation")
                    };
                    assert_eq!(expected_root, "/site");
                    assert_eq!(path, "/site/assets/Mark:01.css");
                    Ok(crate::remote::Reply::File {
                        root: expected_root,
                        path,
                        bytes: b"body{}".to_vec(),
                    })
                })
            };
            assert_eq!(get(entry.clone()).body().as_ref(), alias.as_bytes());
            assert_eq!(
                get(format!("{base}/assets/Mark%3A01.css")).body().as_ref(),
                b"body{}"
            );
            assert_eq!(get(format!("{base}/%2ehidden")).status(), 404);
        }
    }
    #[test]
    #[ignore = "requires installed Windows WebView2 runtime"]
    fn hidden_browser_renders_remote_draft_with_sftp_asset_responses() {
        let dir = tempfile::tempdir().unwrap();
        let pages = Pages::default();
        let url = pages
            .open_remote(
                crate::workspace::Host {
                    alias: "sample".into(),
                    folder: "/site".into(),
                    ..Default::default()
                },
                "/site",
                "/site/pages/index.html",
                r#"<link rel="stylesheet" href="/asset.css"><p>SSH draft</p><img src="pixel.svg">"#,
            )
            .unwrap();
        let parent = crate::webview_test_support::hidden_parent();
        let mut context = wry::WebContext::new(Some(dir.path().into()));
        let builder = wry::WebViewBuilder::new_with_web_context(&mut context);
        let view = pages
            .attach_with(builder, |host, action| {
                assert_eq!(host.alias, "sample");
                let crate::remote::Action::PreviewAsset {
                    expected_root,
                    path,
                } = action
                else {
                    return Err("unexpected operation".into());
                };
                let bytes = match path.as_str() {
                    "/site/asset.css" => b"p{color:rgb(4,5,6)}".to_vec(),
                    "/site/pages/pixel.svg" => {
                        br#"<svg xmlns="http://www.w3.org/2000/svg" width="19" height="1"/>"#
                            .to_vec()
                    }
                    _ => return Err("missing".into()),
                };
                Ok(crate::remote::Reply::File {
                    root: expected_root,
                    path,
                    bytes,
                })
            })
            .unwrap()
            .with_url(url)
            .build_as_child(&parent)
            .unwrap();
        crate::webview_test_support::wait_for(
            &view,
            "document.querySelector('p')?.textContent === 'SSH draft' && getComputedStyle(document.querySelector('p')).color === 'rgb(4, 5, 6)' && document.images[0]?.naturalWidth === 19",
        );
    }
    #[test]
    fn project_page_serves_drafts_and_assets_within_its_project_origin() {
        assert!(!segment_ok("CON.txt"));
        assert!(!segment_ok("com¹.log"));
        assert!(segment_ok("company.css"));
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let file = root.join("index.html");
        std::fs::write(&file, "disk").unwrap();
        std::fs::write(root.join("style sheet.css"), "body{}").unwrap();
        std::fs::write(root.join(".hidden"), "private").unwrap();
        let pages = Pages::default();
        let address = pages
            .open(root, &file, "draft")
            .unwrap()
            .replace("http://tesselpreview.", "tesselpreview://");
        let base = address.strip_suffix("/index.html").unwrap();
        let get =
            |path: &str| pages.serve(Request::get(format!("{base}/{path}")).body(vec![]).unwrap());
        assert_eq!(get("index.html").body().as_ref(), b"draft");
        assert_eq!(
            get("style%20sheet.css").headers()["content-type"],
            "text/css; charset=utf-8"
        );
        for path in [
            ".hidden",
            "%2ehidden",
            "%2Fhidden",
            "%5chidden",
            "%2e%2e/index.html",
            "style%20sheet.css%3Aalternate",
        ] {
            assert_eq!(get(path).status(), 404);
        }
        let head = pages.serve(Request::head(&address).body(vec![]).unwrap());
        assert!(head.body().is_empty());
        assert_eq!(head.headers()["content-length"], "5");
        assert_eq!(
            pages
                .serve(Request::post(&address).body(vec![]).unwrap())
                .status(),
            405
        );
        let same = pages.open(root, &file, "replacement").unwrap();
        assert_eq!(
            same.replace("http://tesselpreview.", "tesselpreview://"),
            address
        );
        assert_eq!(get("index.html").body().as_ref(), b"replacement");
    }

    #[test]
    fn history_retains_recent_sites_and_recently_updated_drafts() {
        let tmp = tempfile::tempdir().unwrap();
        let pages = Pages::default();
        let open = |project: usize, name: usize| {
            let root = tmp.path().join(project.to_string());
            std::fs::create_dir_all(&root).unwrap();
            let file = root.join(format!("{name}.html"));
            std::fs::write(&file, "disk").unwrap();
            pages
                .open(&root, &file, &format!("draft {project}-{name}"))
                .unwrap()
                .replace("http://tesselpreview.", "tesselpreview://")
        };
        let get = |url: &str| pages.serve(Request::get(url).body(Vec::new()).unwrap());
        let oldest = open(0, 0);
        let refreshed = open(0, 1);
        for n in 2..MAX_DRAFTS {
            open(0, n);
        }
        assert_eq!(open(0, 1), refreshed);
        open(0, MAX_DRAFTS);
        assert_eq!(get(&oldest).body().as_ref(), b"disk");
        assert_eq!(get(&refreshed).body().as_ref(), b"draft 0-1");
        let second = open(1, 1);
        for n in 2..MAX_SITES {
            open(n, 1);
        }
        assert_eq!(get(&refreshed).body().as_ref(), b"draft 0-1");
        open(MAX_SITES, 1);
        assert_eq!(get(&second).status(), 404);
        assert_eq!(get(&refreshed).body().as_ref(), b"draft 0-1");
    }

    #[test]
    fn project_assets_read_only_the_requested_range_even_past_the_response_limit() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("large.webm");
        let mut f = File::create(&p).unwrap();
        let offset = LIMIT + 8;
        f.seek(SeekFrom::Start(offset)).unwrap();
        f.write_all(b"tail").unwrap();
        drop(f);
        let size = offset + 4;
        let serve = |range: &str, head| {
            asset_response(
                File::open(&p).unwrap(),
                "video/webm",
                size,
                head,
                Some(&range.parse().unwrap()),
            )
        };
        for range in [
            format!("bytes={offset}-"),
            "bytes=-4".into(),
            format!("bytes={offset}-{}", size + 9),
        ] {
            let reply = serve(&range, false);
            assert_eq!(reply.status(), 206);
            assert_eq!(reply.body().as_ref(), b"tail");
            assert_eq!(
                reply.headers()["content-range"],
                format!("bytes {offset}-{}/{size}", size - 1)
            );
            assert_eq!(reply.headers()["accept-ranges"], "bytes");
            let head = serve(&range, true);
            assert_eq!(head.headers()["content-length"], "4");
            assert!(head.body().is_empty());
        }
        assert_eq!(serve(&format!("bytes={size}-"), false).status(), 416);
        assert_eq!(
            serve("bytes=0-", true).headers()["content-length"],
            LIMIT.to_string()
        );
        assert_eq!(serve("unknown=0-2", true).status(), 200);
    }

    #[test]
    #[ignore = "requires installed Windows WebView2 runtime"]
    fn hidden_browser_renders_unsaved_project_html_local_css_and_image() {
        use crate::webview_test_support::{hidden_parent, wait_for};
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        let nested = root.join("pages");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(root.join("site.css"), "#fixture{color:rgb(11, 22, 33)}").unwrap();
        std::fs::write(
            nested.join("pixel.svg"),
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="12" height="7"/>"#,
        )
        .unwrap();
        std::fs::write(
            nested.join("clip.webm"),
            include_bytes!("../../tests/fixtures/player-blue.webm"),
        )
        .unwrap();
        let entry = nested.join("index.html");
        std::fs::write(&entry, "disk copy").unwrap();
        let mut doc = crate::files::Document::open(&root, &entry).unwrap();
        doc.text = r#"<link rel="stylesheet" href="/site.css"><p id="fixture">unsaved</p><img src="pixel.svg"><video preload="auto" src="clip.webm"></video><script>document.body.dataset.executed="yes"</script>"#.into();
        let parent = hidden_parent();
        let mut browser = crate::browser::Browser::default();
        browser
            .create_with_profile(&parent, temp.path().join("browser-profile"))
            .unwrap();
        browser.open_project_page(&doc).unwrap();
        wait_for(
            browser.view.as_ref().unwrap(),
            "document.querySelector('#fixture')?.textContent === 'unsaved' && getComputedStyle(document.querySelector('#fixture')).color === 'rgb(11, 22, 33)' && document.images[0]?.naturalWidth === 12 && !document.body.dataset.executed",
        );
        let view = browser.view.as_ref().unwrap();
        view.set_visible(true).unwrap();
        view.evaluate_script("document.querySelector('video').load()")
            .unwrap();
        wait_for(
            view,
            "(()=>{let v=document.querySelector('video');return v.readyState >= 1 && v.duration > 0.9 || [v.readyState, v.networkState, v.error?.message, v.duration, v.currentSrc]})()",
        );
        view.evaluate_script("document.querySelector('video').currentTime=0.4")
            .unwrap();
        wait_for(
            view,
            "document.querySelector('video').seeking === false && Math.abs(document.querySelector('video').currentTime - 0.4) < 0.05",
        );
        let second_path = nested.join("second.html");
        std::fs::write(&second_path, "<p>second page</p>").unwrap();
        let second = crate::files::Document::open(&root, &second_path).unwrap();
        browser.open_project_page(&second).unwrap();
        wait_for(
            browser.view.as_ref().unwrap(),
            "document.body?.textContent === 'second page'",
        );
        browser.go_back_now();
        wait_for(
            browser.view.as_ref().unwrap(),
            "document.querySelector('#fixture')?.textContent === 'unsaved' && document.images[0]?.naturalWidth === 12",
        );
        browser.go_forward_now();
        wait_for(
            browser.view.as_ref().unwrap(),
            "document.body?.textContent === 'second page'",
        );
    }
}
