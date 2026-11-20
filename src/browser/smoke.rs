//! Opt-in native acceptance harness. Uses only a temporary profile and synthetic PDF.
use super::Browser;
use eframe::egui;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

fn pdf() -> Vec<u8> {
    let content = "BT /F1 18 Tf 40 100 Td (Tessel Rust original PDF fixture) Tj ET";
    let objects=["<< /Type /Catalog /Pages 2 0 R >>".to_string(),"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 200] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".into(),"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),format!("<< /Length {} >>\nstream\n{content}\nendstream",content.len())];
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    bytes
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    run_mode(false)
}
pub fn run_agent() -> Result<(), Box<dyn std::error::Error>> {
    run_mode(true)
}
fn run_mode(agent: bool) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let address = format!("http://{}/", listener.local_addr()?);
    let stopping = Arc::new(AtomicBool::new(false));
    let stop = stopping.clone();
    let expected = pdf();
    let served = expected.clone();
    let server = thread::spawn(move || {
        let mut incomplete_attempts = 0;
        while !stop.load(Ordering::Relaxed) {
            let (mut stream, _) = match listener.accept() {
                Ok(v) => v,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(_) => break,
            };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let _ = stream.set_nonblocking(false);
            let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
            let mut request = Vec::new();
            let mut buf = [0; 2048];
            while request.len() < 16384 && !request.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => request.extend_from_slice(&buf[..n]),
                }
            }
            let request = String::from_utf8_lossy(&request);
            let path = request
                .lines()
                .next()
                .and_then(|l| l.split_whitespace().nth(1))
                .unwrap_or("/");
            if path == "/slow" || path == "/incomplete" {
                if path == "/incomplete" {
                    incomplete_attempts += 1;
                    if incomplete_attempts > 1 {
                        let _=stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                        continue;
                    }
                }
                let _=stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=transfer.bin\r\nContent-Length: 8388608\r\nConnection: close\r\n\r\n");
                for _ in 0..256 {
                    if stop.load(Ordering::Relaxed) || stream.write_all(&[7; 32768]).is_err() {
                        break;
                    }
                    if path == "/incomplete" {
                        break;
                    }
                    thread::sleep(Duration::from_millis(30));
                }
                continue;
            }
            let authenticated = request.lines().any(|l| {
                l.to_ascii_lowercase().starts_with("cookie:")
                    && l.contains("fixture_session=synthetic-only")
            });
            let (status, extra, body) = if path == "/controls" {
                ("200 OK","Content-Type: text/html".to_string()+"\r\n", br#"<!doctype html><title>Agent controls fixture</title><input id='query'><input id='secret' type='password'><button id='apply' onclick="document.getElementById('result').textContent=document.getElementById('query').value">Apply</button><p id='result'>untouched</p><select id='language' onchange="choices()"><option value='en'>English</option><option value='fr'>French</option><option disabled value='disabled'>Disabled</option></select><input type='checkbox' id='enabled' onchange="choices()"><p id='choices'>choices:en:false</p><div style='height:2000px'></div><button id='bottom'>Bottom</button><script>function choices(){document.getElementById('choices').textContent='choices:'+document.getElementById('language').value+':'+document.getElementById('enabled').checked;}</script>"#.to_vec())
            } else if matches!(
                path,
                "/" | "/case-auth" | "/case-blob" | "/case-slow" | "/case-fail"
            ) {
                let script = match path {
                    "/" => "save('/direct','direct.pdf');",
                    "/case-auth" => "save('/auth','auth.pdf');",
                    "/case-slow" => "save('/slow','slow.bin');",
                    "/case-fail" => "save('/incomplete','incomplete.bin');",
                    _ => {
                        "const r=await fetch('/auth');if(!r.ok)throw Error('fixture authentication failed');save(URL.createObjectURL(await r.blob()),'blob.pdf');"
                    }
                };
                let html = format!(
                    r#"<!doctype html><title>Tessel acceptance</title><h1>Synthetic PDF fixture</h1><script>
function save(url,name){{const a=document.createElement('a');a.href=url;a.download=name;document.body.appendChild(a);a.click();a.remove();}}
setTimeout(async()=>{{{script}}},700);</script>"#
                );
                let cookie = if path == "/" {
                    "Set-Cookie: fixture_session=synthetic-only; HttpOnly; SameSite=Strict; Path=/\r\n"
                } else {
                    ""
                };
                (
                    "200 OK",
                    format!("Content-Type: text/html\r\n{cookie}"),
                    html.into_bytes(),
                )
            } else {
                match path {
                "/direct" => ("200 OK","Content-Type: application/pdf\r\nContent-Disposition: attachment; filename=direct.pdf\r\n".into(),served.clone()),
                "/auth" if authenticated => ("200 OK","Content-Type: application/pdf\r\nContent-Disposition: attachment; filename=auth.pdf\r\n".into(),served.clone()),
                "/auth" => ("401 Unauthorized","Content-Type: text/plain\r\n".into(),b"Cookie required".to_vec()),
                _ => ("404 Not Found",String::new(),Vec::new()),
            }
            };
            let header = format!(
                "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(&body);
        }
    });
    let result = Arc::new(Mutex::new(None));
    let result_for_app = result.clone();
    let root = directory.path().to_owned();
    let native = eframe::run_native(
        "Tessel Rust · browser acceptance",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([900.0, 600.0]),
            ..Default::default()
        },
        Box::new(move |_| {
            let browser = Browser::default();
            *browser
                .destination
                .lock()
                .map_err(|_| "Download lock failed")? = root.join("downloads");
            Ok(Box::new(Smoke {
                browser,
                agent,
                probe: Probe::default(),
                root,
                address,
                expected,
                started: Instant::now(),
                initialized: false,
                stage: 0,
                transfer_probe: Default::default(),
                result: result_for_app,
            }))
        }),
    );
    stopping.store(true, Ordering::Relaxed);
    let _ = server.join();
    native?;
    let outcome = result
        .lock()
        .map_err(|_| "Acceptance result lock failed")?
        .take()
        .unwrap_or_else(|| Err("Acceptance window closed before completion.".into()));
    outcome.map_err(Into::into)
}
struct Smoke {
    browser: Browser,
    agent: bool,
    probe: Probe,
    root: std::path::PathBuf,
    address: String,
    expected: Vec<u8>,
    started: Instant,
    initialized: bool,
    stage: usize,
    transfer_probe: TransferProbe,
    result: Arc<Mutex<Option<Result<(), String>>>>,
}
impl eframe::App for Smoke {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if !self.initialized {
            self.initialized = true;
            let start = if self.agent {
                Browser::start_agent(frame, &self.root).map(|browser| self.browser = browser)
            } else {
                self.browser
                    .create_with_profile(frame, self.root.join("profile"))
            };
            if let Err(e) = start.and_then(|()| {
                if self.agent { self.browser.control(&serde_json::json!({"operation":"prepare_download","folder":".","filename":"chosen-0.pdf"}))?; } Ok(())
            }) {
                if let Ok(mut r)=self.result.lock(){*r=Some(Err(e));}
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);return;
            }
            self.browser.address = self.address.clone();
            self.browser.navigate();
        }
        self.browser.begin_frame();
        egui::CentralPanel::default().show(ctx, |ui| self.browser.show(ui, frame));
        let _ = self.browser.sync_presentation(true, None, false);
        let completed: Vec<_> = self
            .browser
            .downloads
            .iter()
            .filter(|d| d.status == "Completed")
            .collect();
        let mut outcome = if completed.len() == 3 {
            let all_original = completed
                .iter()
                .all(|d| std::fs::read(&d.path).is_ok_and(|bytes| bytes == self.expected));
            let names: std::collections::HashSet<_> = completed
                .iter()
                .filter_map(|d| d.path.file_name().map(|n| n.to_string_lossy().to_string()))
                .collect();
            Some(
                if all_original
                    && (if self.agent {
                        ["chosen-0.pdf", "chosen-1.pdf", "chosen-2.pdf"]
                    } else {
                        ["direct.pdf", "auth.pdf", "blob.pdf"]
                    })
                    .iter()
                    .all(|n| names.contains(*n))
                {
                    Ok(())
                } else {
                    Err("Downloaded files differ from the original fixture.".into())
                },
            )
        } else if self.started.elapsed() > Duration::from_secs(40) {
            Some(Err(format!(
                "Browser acceptance timed out: {} completed downloads; {:?} tracked.",
                completed.len(),
                self.browser
                    .downloads
                    .iter()
                    .map(|d| (&d.status, &d.error, d.received_bytes, d.total_bytes))
                    .collect::<Vec<_>>()
            )))
        } else {
            None
        };
        let completed_count = completed.len();
        if outcome.as_ref().is_some_and(Result::is_ok) {
            outcome = self
                .transfer_probe
                .advance(&mut self.browser, &self.address, self.agent);
        }
        if self.agent && outcome.as_ref().is_some_and(Result::is_ok) {
            outcome = self
                .probe
                .advance(&mut self.browser, frame, &self.root, &self.address);
        }
        if self.started.elapsed() > Duration::from_secs(75) {
            outcome = Some(Err(format!(
                "Browser acceptance timed out at transfer phase {}: {:?}",
                self.transfer_probe.phase,
                self.browser
                    .downloads
                    .iter()
                    .map(|d| (&d.status, d.received_bytes, d.total_bytes))
                    .collect::<Vec<_>>()
            )));
        }
        if outcome.is_none() && completed_count > self.stage && completed_count < 3 {
            self.stage = completed_count;
            self.browser.address = format!(
                "{}{}",
                self.address,
                if completed_count == 1 {
                    "case-auth"
                } else {
                    "case-blob"
                }
            );
            if self.agent && let Err(e)=self.browser.control(&serde_json::json!({"operation":"prepare_download","folder":".","filename":format!("chosen-{completed_count}.pdf")})) { outcome=Some(Err(e)); }
            self.browser.navigate();
        }
        if let Some(outcome) = outcome {
            if let Ok(mut r) = self.result.lock() {
                *r = Some(outcome);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint_after(Duration::from_millis(50));
    }
}

#[derive(Default)]
struct Probe {
    phase: usize,
    pending: Option<super::control::ControlReply>,
    owner: Option<Browser>,
}
impl Probe {
    fn advance(
        &mut self,
        b: &mut Browser,
        frame: &eframe::Frame,
        root: &std::path::Path,
        address: &str,
    ) -> Option<Result<(), String>> {
        use serde_json::json;
        if let Some(rx) = &self.pending {
            let Ok(result) = rx.try_recv() else {
                return None;
            };
            self.pending = None;
            if self.phase == 2
                && result
                    .as_ref()
                    .is_ok_and(|v| v["title"] != "Agent controls fixture")
            {
                self.phase = 1;
                return None;
            }
            if self.phase == 5
                && !result.as_ref().is_ok_and(|v| {
                    v["text"]
                        .as_str()
                        .is_some_and(|t| t.contains("native-agent-fill"))
                })
            {
                return Some(Err("DOM fill/click did not update fixture".into()));
            }
            if self.phase == 11
                && !result.as_ref().is_ok_and(|v| {
                    v["text"]
                        .as_str()
                        .is_some_and(|t| t.contains("choices:fr:true"))
                        && v["scroll"]["y"].as_f64().is_some_and(|y| y > 0.0)
                })
            {
                return Some(Err(
                    "Select/check/scroll did not update native fixture".into()
                ));
            }
            if self.phase == 15 {
                let valid = result.as_ref().is_ok_and(|v| {
                    let Some(path) = v["path"].as_str() else {
                        return false;
                    };
                    let Ok(path) = std::path::Path::new(path).canonicalize() else {
                        return false;
                    };
                    let Ok(root) = root.canonicalize() else {
                        return false;
                    };
                    if !path.starts_with(root) {
                        return false;
                    }
                    let Ok(bytes) = std::fs::read(path) else {
                        return false;
                    };
                    let Ok(image) = image::load_from_memory(&bytes) else {
                        return false;
                    };
                    image.width() >= 500
                        && image.height() >= 200
                        && u64::from(image.width()) == v["width"].as_u64().unwrap_or(0)
                        && u64::from(image.height()) == v["height"].as_u64().unwrap_or(0)
                        && image.to_rgba8().pixels().any(|p| p.0[0] < 200)
                });
                if !valid {
                    return Some(Err(format!(
                        "Native screenshot content/path verification failed: {result:?}"
                    )));
                }
                if b.control(&json!({"operation":"screenshot","origin":"https://wrong.invalid","filename":"denied.png"})).is_ok(){return Some(Err("Screenshot accepted another origin".into()));}
                if b.control(&json!({"operation":"screenshot","origin":address.trim_end_matches('/'),"folder":"..","filename":"denied.png"})).is_ok(){return Some(Err("Screenshot escaped the project".into()));}
            }
            if matches!(self.phase, 6 | 7 | 12 | 13 | 14) {
                if result.is_ok() {
                    return Some(Err(
                        "Origin/password guard accepted an invalid action".into()
                    ));
                }
            } else if let Err(e) = result {
                return Some(Err(e));
            }
        }
        if self.phase == 0 {
            let status = b.control(&json!({"operation":"downloads"})).and_then(|rx| {
                rx.try_recv()
                    .map_err(|_| "Missing download status".to_string())?
            });
            if !status.is_ok_and(|v| {
                v["downloads"].as_array().is_some_and(|items| {
                    items.iter().filter(|d| d["status"] == "Completed").count() == 3
                        && items
                            .iter()
                            .filter(|d| d["status"] == "Completed")
                            .all(|d| {
                                d["status"] == "Completed"
                                    && d["path"]
                                        .as_str()
                                        .is_some_and(|p| std::path::Path::new(p).is_file())
                            })
                })
            }) {
                return Some(Err("Agent download status/path verification failed".into()));
            }
        }
        let origin = address.trim_end_matches('/');
        let args = match self.phase {
            0 => json!({"operation":"navigate","url":format!("{address}controls")}),
            1 => json!({"operation":"snapshot"}),
            2 => {
                json!({"operation":"fill","origin":origin,"selector":"#query","text":"native-agent-fill"})
            }
            3 => json!({"operation":"click","origin":origin,"selector":"#apply"}),
            4 => json!({"operation":"snapshot"}),
            5 => json!({"operation":"click","origin":"https://wrong.invalid","selector":"#apply"}),
            6 => {
                json!({"operation":"fill","origin":origin,"selector":"#secret","text":"must-not-fill"})
            }
            7 => json!({"operation":"select","origin":origin,"selector":"#language","value":"fr"}),
            8 => json!({"operation":"check","origin":origin,"selector":"#enabled","checked":true}),
            9 => json!({"operation":"scroll","origin":origin,"selector":"#bottom"}),
            10 => json!({"operation":"snapshot"}),
            11 => {
                json!({"operation":"select","origin":origin,"selector":"#language","value":"disabled"})
            }
            12 => json!({"operation":"check","origin":origin,"selector":"#query","checked":true}),
            13 => json!({"operation":"check","origin":origin,"selector":"#secret","checked":true}),
            14 => {
                json!({"operation":"screenshot","origin":origin,"folder":".","filename":"native-fixture.png"})
            }
            _ => {
                let result = (|| -> Result<(), String> {
                    let cookies = b
                        .view
                        .as_ref()
                        .ok_or("Missing agent browser")?
                        .cookies_for_url(address)
                        .map_err(|_| "Cannot inspect fixture cookies")?;
                    if cookies.is_empty() {
                        return Err("Authenticated fixture cookie was lost".into());
                    }
                    let mut owner = Browser::default();
                    owner.create_with_profile(frame, root.join("isolated-owner"))?;
                    if !owner
                        .view
                        .as_ref()
                        .unwrap()
                        .cookies_for_url(address)
                        .map_err(|_| "Cannot inspect owner isolation")?
                        .is_empty()
                    {
                        return Err("Agent fixture cookies leaked to another profile".into());
                    }
                    let export = r#"[
                        {"domain":"example.test","name":"host_fixture","value":"synthetic","hostOnly":true,"secure":true,"httpOnly":true},
                        {"domain":"example.test","name":"domain_fixture","value":"synthetic","hostOnly":false,"secure":true,"httpOnly":true},
                        {"domain":"accounts.google.com","name":"excluded_fixture","value":"synthetic"}
                    ]"#;
                    if b.import_cookie_text(export).is_ok() {
                        return Err("Agent browser accepted owner cookie import".into());
                    }
                    if owner.import_cookie_text(export)? != (2, 1) {
                        return Err("Owner cookie import counts are incorrect".into());
                    }
                    let owner_view = owner.view.as_ref().unwrap();
                    let exact = owner_view
                        .cookies_for_url("https://example.test/")
                        .map_err(|_| "Cannot inspect synthetic cookie import")?;
                    let subdomain = owner_view
                        .cookies_for_url("https://sub.example.test/")
                        .map_err(|_| "Cannot inspect synthetic cookie scope")?;
                    if exact.len() != 2
                        || subdomain.len() != 1
                        || subdomain[0].name() != "domain_fixture"
                        || exact
                            .iter()
                            .any(|c| c.http_only() != Some(true) || c.secure() != Some(true))
                    {
                        return Err("Imported cookie scope or flags changed".into());
                    }
                    if !b
                        .view
                        .as_ref()
                        .unwrap()
                        .cookies_for_url("https://example.test/")
                        .map_err(|_| "Cannot inspect agent isolation after import")?
                        .is_empty()
                    {
                        return Err("Imported owner cookies reached the agent browser".into());
                    }
                    self.owner = Some(owner);
                    Ok(())
                })();
                return Some(result);
            }
        };
        match b.control(&args) {
            Ok(rx) => {
                self.pending = Some(rx);
                self.phase += 1;
                None
            }
            Err(e) => Some(Err(e)),
        }
    }
}

#[derive(Default)]
struct TransferProbe {
    phase: usize,
    cancelled_id: Option<u64>,
}
impl TransferProbe {
    fn advance(
        &mut self,
        b: &mut Browser,
        address: &str,
        agent: bool,
    ) -> Option<Result<(), String>> {
        use serde_json::json;
        let result = (|| -> Result<bool, String> {
            match self.phase {
                0 | 2 => {
                    b.download_tracker.set_stall_timeout(Duration::from_secs(3));
                    if self.phase == 2 {
                        let Some(download) =
                            b.downloads.iter().find(|d| Some(d.id) == self.cancelled_id)
                        else {
                            return Ok(false);
                        };
                        if download.status == "Downloading" {
                            return Ok(false);
                        }
                        if download.status != "Cancelled" {
                            return Err("Native cancellation did not report Cancelled".into());
                        }
                    }
                    if agent {
                        b.control(&json!({"operation":"prepare_download","folder":".","filename":if self.phase==0{"slow.bin"}else{"incomplete.bin"}}))?;
                    }
                    b.open_address(format!(
                        "{address}{}",
                        if self.phase == 0 {
                            "case-slow"
                        } else {
                            "case-fail"
                        }
                    ));
                    self.phase += 1;
                    Ok(false)
                }
                1 => {
                    let Some(download) = b.downloads.iter().find(|d| {
                        d.status == "Downloading"
                            && d.received_bytes > 0
                            && d.total_bytes.is_some_and(|t| t > d.received_bytes)
                    }) else {
                        return Ok(false);
                    };
                    let id = download.id;
                    self.cancelled_id = Some(id);
                    if agent {
                        b.control(&json!({"operation":"cancel_download","id":id}))?;
                    } else {
                        b.download_tracker.cancel(id)?;
                    }
                    self.phase = 2;
                    Ok(false)
                }
                3 => {
                    let Some(download) = b
                        .downloads
                        .iter()
                        .find(|d| d.id > self.cancelled_id.unwrap_or(0) && d.status == "Failed")
                    else {
                        return Ok(false);
                    };
                    if download.status == "Downloading" {
                        return Ok(false);
                    }
                    if download.status != "Failed" || download.error.is_none() {
                        return Err("Interrupted original incorrectly reported as complete".into());
                    }
                    self.phase = 4;
                    Ok(true)
                }
                _ => Ok(true),
            }
        })();
        match result {
            Ok(true) => Some(Ok(())),
            Ok(false) => None,
            Err(e) => Some(Err(e)),
        }
    }
}
