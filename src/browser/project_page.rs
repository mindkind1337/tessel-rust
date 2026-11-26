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

type AssetJob = Box<dyn FnOnce(bool) + Send>;
type AssetQueue = mpsc::SyncSender<AssetJob>;

fn asset_workers(count: usize) -> Result<AssetQueue, String> {
    let (send, recv) = mpsc::sync_channel::<AssetJob>(64);
    let recv = Arc::new(Mutex::new(recv));
    for _ in 0..count {
        let recv = recv.clone();
        std::thread::Builder::new()
            .name("project-page".into())
            .spawn(move || {
                loop {
                    // Release the receiver before running I/O so other workers can dequeue.
                    let next = recv.lock().unwrap().recv();
                    match next {
                        Ok(job) => job(true),
                        Err(_) => break,
                    }
                }
            })
            .map_err(|e| e.to_string())?;
    }
    Ok(send)
}

#[derive(Clone)]
struct Site {
    scripts: bool,
    authority: String,
    root: PathBuf,
    remote: Option<crate::workspace::Host>,
    drafts: VecDeque<(PathBuf, Arc<[u8]>)>,
}

#[derive(Clone, Default)]
pub(crate) struct Pages {
    sites: Arc<Mutex<VecDeque<Arc<Site>>>>,
    scripts: bool,
}

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
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "webmanifest" => "application/manifest+json; charset=utf-8",
        "xml" => "application/xml; charset=utf-8",
        "wasm" => "application/wasm",
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
    pub(super) fn for_open(&self, scripts: bool) -> Self {
        Self {
            sites: self.sites.clone(),
            scripts,
        }
    }
    pub(super) fn restrict(&self, folder: &super::html_trust::Folder) {
        let mut sites = self.sites.lock().unwrap_or_else(|err| err.into_inner());
        for site in sites.iter_mut() {
            let matches = match (folder, &site.remote) {
                (super::html_trust::Folder::Local(path), None) => site.root == *path,
                (super::html_trust::Folder::Ssh { host, root }, Some(remote)) => {
                    remote.alias == *host && site.root.to_str() == Some(root.as_str())
                }
                _ => false,
            };
            if matches && site.scripts {
                Arc::make_mut(site).scripts = false;
            }
        }
    }
    pub(super) fn open(&self, root: &Path, file: &Path, text: &str) -> Result<String, String> {
        let canonical = root.canonicalize().map_err(|e| e.to_string())?;
        if self.scripts && canonical != root {
            return Err("The trusted folder has moved.".into());
        }
        let root = canonical;
        let entry = crate::files::confined(&root, file)?;
        if !visible_relative(&root, &entry)
            || mime(&entry) != "text/html; charset=utf-8"
            || text.len() > crate::files::MAX_TEXT
        {
            return Err("This project page cannot be previewed.".into());
        }
        self.register(root, entry, text, None)
    }
    pub(crate) fn open_remote(
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
        let mut sites = self
            .sites
            .lock()
            .map_err(|_| "Project preview unavailable")?;
        let mut site = if let Some(i) = sites.iter().position(|s| {
            s.scripts == self.scripts
                && s.root == root
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
                scripts: self.scripts,
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
            let mut reply = empty(405);
            reply
                .headers_mut()
                .insert("Allow", "GET, HEAD".parse().unwrap());
            return reply;
        }
        let Some(authority) = req.uri().authority() else {
            return empty(404);
        };
        let Some(site) = self.sites.lock().ok().and_then(|mut sites| {
            let i = sites
                .iter()
                .position(|site| site.authority == authority.as_str())?;
            let site = sites.remove(i)?;
            sites.push_back(site.clone());
            Some(site)
        }) else {
            return empty(404);
        };
        let mut reply = Self::serve_asset(&site, req, read_remote);
        if site.scripts && reply.status().is_success() {
            reply.headers_mut().insert("Content-Security-Policy", wry::http::HeaderValue::from_static(
                "default-src 'self' https: data: blob:; script-src 'self' 'unsafe-inline' 'unsafe-eval' https: blob:; style-src 'self' 'unsafe-inline' https:; img-src 'self' data: blob: https:; font-src 'self' data: https:; media-src 'self' data: blob: https:; connect-src 'self' https: wss:; frame-src 'none'; worker-src 'none'; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'"));
        }
        reply
    }
    fn serve_asset(
        site: &Site,
        req: Request<Vec<u8>>,
        read_remote: impl FnOnce(
            crate::workspace::Host,
            crate::remote::Action,
        ) -> Result<crate::remote::Reply, String>,
    ) -> Reply {
        let head = req.method() == "HEAD";
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
            return match read_remote(
                host.clone(),
                crate::remote::Action::PreviewAsset {
                    expected_root: root.into(),
                    path: requested,
                },
            ) {
                Ok(crate::remote::Reply::File {
                    root: actual,
                    path: resolved,
                    bytes,
                }) if actual == root
                    && crate::remote::preview_relative(root, &resolved).is_some() =>
                {
                    let path = Path::new(&resolved);
                    if let Some((_, draft)) = site.drafts.iter().find(|(p, _)| p == path) {
                        response(
                            200,
                            mime(path),
                            draft.len() as u64,
                            if head { vec![] } else { draft.to_vec() },
                        )
                    } else {
                        response(
                            200,
                            mime(path),
                            bytes.len() as u64,
                            if head { vec![] } else { bytes },
                        )
                    }
                }
                Ok(crate::remote::Reply::PreviewTooLarge) => empty(413),
                _ => empty(404),
            };
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
        self.attach_with(builder, None, crate::remote::execute)
    }
    pub(super) fn attach_editor<'a>(
        &self,
        builder: wry::WebViewBuilder<'a>,
        address: &url::Url,
    ) -> Result<wry::WebViewBuilder<'a>, String> {
        let authority = address
            .host_str()
            .and_then(|host| host.strip_prefix("tesselpreview."))
            .ok_or("Invalid project preview address")?;
        self.attach_with(builder, Some(authority.to_owned()), crate::remote::execute)
    }
    fn attach_with<'a>(
        &self,
        builder: wry::WebViewBuilder<'a>,
        editor_authority: Option<String>,
        read_remote: fn(
            crate::workspace::Host,
            crate::remote::Action,
        ) -> Result<crate::remote::Reply, String>,
    ) -> Result<wry::WebViewBuilder<'a>, String> {
        let local = asset_workers(1)?;
        static REMOTE: std::sync::OnceLock<Result<AssetQueue, String>> = std::sync::OnceLock::new();
        let pages = self.clone();
        Ok(
            builder.with_asynchronous_custom_protocol(SCHEME.into(), move |_, req, responder| {
                if editor_authority.as_ref().is_some_and(|allowed| {
                    req.uri().authority().map(|value| value.as_str()) != Some(allowed.as_str())
                }) {
                    responder.respond(empty(404));
                    return;
                }
                let send = if req.uri().host().is_some_and(|h| h.starts_with("ssh-")) {
                    let Ok(send) = REMOTE.get_or_init(|| asset_workers(4)) else {
                        responder.respond(empty(503));
                        return;
                    };
                    send
                } else {
                    &local
                };
                let pages = pages.clone();
                let job = move |accepted| {
                    responder.respond(if accepted {
                        pages.serve_with(req, read_remote)
                    } else {
                        empty(503)
                    });
                };
                // Recover the responder on overload without blocking the WebView thread.
                if let Err(error) = send.try_send(Box::new(job)) {
                    let job = match error {
                        mpsc::TrySendError::Full(job) | mpsc::TrySendError::Disconnected(job) => {
                            job
                        }
                    };
                    job(false);
                }
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn four_remote_assets_can_start_before_any_finishes() {
        use std::time::Duration;
        let queue = asset_workers(4).unwrap();
        let (started, events) = mpsc::channel();
        let mut releases = Vec::new();
        for _ in 0..5 {
            let started = started.clone();
            let (release, wait) = mpsc::channel::<()>();
            releases.push(release);
            queue
                .send(Box::new(move |accepted| {
                    assert!(accepted);
                    started.send(()).unwrap();
                    let _ = wait.recv();
                }))
                .unwrap();
        }
        for _ in 0..4 {
            events.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        assert!(matches!(events.try_recv(), Err(mpsc::TryRecvError::Empty)));
        releases.clear();
        events.recv_timeout(Duration::from_secs(5)).unwrap();
    }

    #[test]
    fn trusted_ssh_sites_keep_distinct_origins_per_host_folder_and_mode() {
        let registry = Pages::default();
        let open = |alias: &str, trusted| {
            registry
                .for_open(trusted)
                .open_remote(
                    crate::workspace::Host {
                        alias: alias.into(),
                        ..Default::default()
                    },
                    "/site",
                    "/site/index.html",
                    "draft",
                )
                .unwrap()
                .replace("http://tesselpreview.", "tesselpreview://")
        };
        let restricted = open("dev", false);
        let trusted = open("dev", true);
        let other = open("live", true);
        let policy = |url: &str| {
            let reply = registry.serve(Request::get(url).body(vec![]).unwrap());
            assert_eq!(reply.status(), 200);
            reply.headers()["content-security-policy"]
                .to_str()
                .unwrap()
                .to_owned()
        };
        assert_ne!(trusted, restricted);
        assert_eq!(policy(&restricted), POLICY);
        assert!(policy(&trusted).contains("script-src 'self'"));
        registry.restrict(&super::super::html_trust::Folder::Ssh {
            host: "dev".into(),
            root: "/site".into(),
        });
        assert_eq!(policy(&trusted), POLICY);
        assert!(policy(&other).contains("script-src 'self'"));
        assert_ne!(open("dev", true), trusted);
        assert_eq!(policy(&trusted), POLICY);
    }

    #[test]
    #[ignore = "requires installed Windows WebView2 runtime"]
    fn hidden_owner_browser_runs_approved_draft_and_reopens_it_restricted_on_revocation() {
        use crate::webview_test_support::{hidden_parent, wait_for};
        let dir = tempfile::tempdir().unwrap();
        let site = dir.path().join("site");
        std::fs::create_dir(&site).unwrap();
        let path = site.join("index.html");
        std::fs::write(&path, "disk").unwrap();
        std::fs::write(
            site.join("module.js"),
            "document.body.dataset.executed='yes'",
        )
        .unwrap();
        let mut doc = crate::files::Document::open(&site, &path).unwrap();
        doc.text = "<p>draft</p><script type='module' src='module.js'></script>".into();
        let parent = hidden_parent();
        let profile = dir.path().join("test-profile");
        let mut browser = crate::browser::Browser::default();
        browser
            .create_with_profile(&parent, profile.clone())
            .unwrap();
        browser.open_project_page(&doc, true).unwrap();
        wait_for(
            browser.view.as_ref().unwrap(),
            "document.body?.dataset.executed === 'yes' && document.querySelector('p')?.textContent === 'draft'",
        );
        browser.revoke_preview_scripts(&super::super::html_trust::Folder::Local(
            doc.tools.html_root.clone().unwrap(),
        ));
        assert!(browser.view.is_none() && browser.auto_start_pending);
        browser.create_with_profile(&parent, profile).unwrap();
        browser.navigate();
        wait_for(
            browser.view.as_ref().unwrap(),
            "document.readyState === 'complete' && document.querySelector('p')?.textContent === 'draft' && !document.body.dataset.executed",
        );
    }
    #[test]
    fn remote_directory_links_serve_registered_index_drafts_and_report_oversized_assets() {
        let pages = Pages::default();
        let host = crate::workspace::Host {
            alias: "fixture".into(),
            ..Default::default()
        };
        let entry = pages
            .open_remote(host, "/site", "/site/docs/index.html", "unsaved")
            .unwrap()
            .replace("http://tesselpreview.", "tesselpreview://");
        let directory = entry.strip_suffix("/index.html").unwrap();
        for method in ["GET", "HEAD"] {
            let req = Request::builder()
                .method(method)
                .uri(directory)
                .body(vec![])
                .unwrap();
            let reply = pages.serve_with(req, |_, action| {
                assert!(matches!(action, crate::remote::Action::PreviewAsset {path, ..} if path == "/site/docs"));
                Ok(crate::remote::Reply::File {root:"/site".into(), path:"/site/docs/index.html".into(),bytes:b"disk".to_vec()})
            });
            assert_eq!(reply.status(), 200);
            assert_eq!(reply.headers()["content-length"], "7");
            assert_eq!(
                reply.body().as_ref(),
                if method == "HEAD" {
                    &b""[..]
                } else {
                    &b"unsaved"[..]
                }
            );
        }
        let reply = pages.serve_with(Request::get(directory).body(vec![]).unwrap(), |_, _| {
            Ok(crate::remote::Reply::PreviewTooLarge)
        });
        assert_eq!(reply.status(), 413);
    }

    #[test]
    fn site_serves_web_asset_types_with_restricted_policy() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("index.html");
        std::fs::write(&file, "index").unwrap();
        let pages = Pages::default();
        let entry = pages
            .open(root.path(), &file, "index")
            .unwrap()
            .replace("http://tesselpreview.", "tesselpreview://");
        let base = entry.strip_suffix("index.html").unwrap();
        for (name, expected) in [
            ("a.JS", "text/javascript; charset=utf-8"),
            ("a.mjs", "text/javascript; charset=utf-8"),
            ("a.json", "application/json; charset=utf-8"),
            ("a.map", "application/json; charset=utf-8"),
            ("a.webmanifest", "application/manifest+json; charset=utf-8"),
            ("a.xml", "application/xml; charset=utf-8"),
            ("a.wasm", "application/wasm"),
            ("a.unknown", "application/octet-stream"),
        ] {
            std::fs::write(root.path().join(name), "sample").unwrap();
            let res = pages.serve(Request::get(format!("{base}{name}")).body(vec![]).unwrap());
            assert_eq!(res.headers()["content-type"], expected);
            assert_eq!(res.headers()["content-security-policy"], POLICY);
        }
        let refused = pages.serve(Request::post(entry).body(vec![]).unwrap());
        assert_eq!(refused.status(), 405);
        assert_eq!(refused.headers()["allow"], "GET, HEAD");
    }

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
            .attach_with(builder, None, |host, action| {
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
        browser.open_project_page(&doc, false).unwrap();
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
        browser.open_project_page(&second, false).unwrap();
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
