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
    let directory = tempfile::tempdir()?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let address = format!("http://{}/", listener.local_addr()?);
    let stopping = Arc::new(AtomicBool::new(false));
    let stop = stopping.clone();
    let expected = pdf();
    let served = expected.clone();
    let server = thread::spawn(move || {
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
            let authenticated = request.lines().any(|l| {
                l.to_ascii_lowercase().starts_with("cookie:")
                    && l.contains("fixture_session=synthetic-only")
            });
            let (status, extra, body) = if matches!(path, "/" | "/case-auth" | "/case-blob") {
                let script = match path {
                    "/" => "save('/direct','direct.pdf');",
                    "/case-auth" => "save('/auth','auth.pdf');",
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
                root,
                address,
                expected,
                started: Instant::now(),
                initialized: false,
                stage: 0,
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
    root: std::path::PathBuf,
    address: String,
    expected: Vec<u8>,
    started: Instant,
    initialized: bool,
    stage: usize,
    result: Arc<Mutex<Option<Result<(), String>>>>,
}
impl eframe::App for Smoke {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if !self.initialized {
            self.initialized = true;
            if let Err(e) = self
                .browser
                .create_with_profile(frame, self.root.join("profile"))
            {
                if let Ok(mut r) = self.result.lock() {
                    *r = Some(Err(e));
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            self.browser.address = self.address.clone();
            self.browser.navigate();
        }
        egui::CentralPanel::default().show(ctx, |ui| self.browser.show(ui, frame));
        let completed: Vec<_> = self
            .browser
            .downloads
            .iter()
            .filter(|d| d.1 == "Completed")
            .collect();
        let outcome = if completed.len() == 3 {
            let all_original = completed
                .iter()
                .all(|d| std::fs::read(&d.0).is_ok_and(|bytes| bytes == self.expected));
            let names: std::collections::HashSet<_> = completed
                .iter()
                .filter_map(|d| d.0.file_name().map(|n| n.to_string_lossy().to_string()))
                .collect();
            Some(
                if all_original
                    && ["direct.pdf", "auth.pdf", "blob.pdf"]
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
                "Browser acceptance timed out: {} completed downloads; {} tracked.",
                completed.len(),
                self.browser.downloads.len()
            )))
        } else {
            None
        };
        let completed_count = completed.len();
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
