//! Explicit synthetic acceptance: no real account, network server or user profile.
use super::Browser;
use eframe::egui;
use std::{
    borrow::Cow,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};
use wry::{WebContext, WebViewBuilder, WebViewBuilderExtWindows};
const ORIGIN: &str = "https://tessel-fixture.localhost";
const HTML:&[u8]=br#"<!doctype html><h1>Private password fill test</h1><form onsubmit="window.submissions++;return false"><input type="email" id="user"><input type="password" id="pass"><button>Sign in</button></form><script>window.submissions=0;</script>"#;
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let result = Arc::new(Mutex::new(None));
    let app_result = result.clone();
    eframe::run_native(
        "Tessel Rust synthetic vault acceptance",
        eframe::NativeOptions::default(),
        Box::new(move |_| {
            let (sender, receiver) = mpsc::channel();
            Ok(Box::new(Check {
                browser: Browser::default(),
                profile: dir.path().to_owned(),
                _directory: dir,
                stage: 0,
                pending: false,
                started: Instant::now(),
                sender,
                receiver,
                result: app_result,
            }))
        }),
    )?;
    result
        .lock()
        .map_err(|_| "Result unavailable")?
        .take()
        .unwrap_or(Err("Acceptance closed early".into()))
        .map_err(Into::into)
}
struct Check {
    browser: Browser,
    profile: std::path::PathBuf,
    _directory: tempfile::TempDir,
    stage: u8,
    pending: bool,
    started: Instant,
    sender: mpsc::Sender<String>,
    receiver: mpsc::Receiver<String>,
    result: Arc<Mutex<Option<Result<(), String>>>>,
}
impl Check {
    fn finish(&self, ctx: &egui::Context, result: Result<(), String>) {
        if let Ok(mut output) = self.result.lock() {
            *output = Some(result);
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
    fn evaluate(&mut self, script: &str) -> Result<(), String> {
        self.pending = true;
        let sender = self.sender.clone();
        self.browser
            .view
            .as_ref()
            .ok_or("Missing view")?
            .evaluate_script_with_callback(script, move |value| {
                let _ = sender.send(value);
            })
            .map_err(|_| "Script evaluation failed".into())
    }
    fn next(&mut self) -> Result<(), String> {
        let account = crate::vault::Account {
            origin: ORIGIN.into(),
            username: "synthetic@example.test".into(),
            password: "synthetic-only-secret".into(),
        };
        match self.stage {
            0=>self.evaluate("document.readyState==='complete' && !!document.getElementById('pass')"),
            1=>{
                let wrong=crate::vault::Account{origin:"https://different.example.test".into(),username:"unused".into(),password:"unused".into()};
                if self.browser.fill_account(&wrong).is_ok(){return Err("Wrong-origin fill was accepted".into());}
                self.browser.fill_account(&account)?;
                self.evaluate("document.getElementById('user').value==='synthetic@example.test' && document.getElementById('pass').value==='synthetic-only-secret' && window.submissions===0")
            }
            2=>self.evaluate("(() => { document.getElementById('user').value=''; document.getElementById('pass').value='';const extra=document.createElement('input');extra.type='password';document.querySelector('form').appendChild(extra);return true; })()"),
            3=>{self.browser.fill_account(&account)?;self.evaluate("[...document.querySelectorAll('input')].every(el=>el.value==='') && window.submissions===0")}
            _=>Ok(()),
        }
    }
}
impl eframe::App for Check {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if self.browser.view.is_none() {
            self.browser.context = Some(WebContext::new(Some(self.profile.clone())));
            let result =
                WebViewBuilder::new_with_web_context(self.browser.context.as_mut().unwrap())
                    .with_custom_protocol("tessel-fixture".into(), |_, _| {
                        wry::http::Response::builder()
                            .header("Content-Type", "text/html")
                            .body(Cow::Borrowed(HTML))
                            .unwrap()
                    })
                    .with_https_scheme(true)
                    .with_url("tessel-fixture://localhost/")
                    .with_general_autofill_enabled(false)
                    .with_devtools(false)
                    .build_as_child(frame);
            match result {
                Ok(view) => self.browser.view = Some(view),
                Err(_) => {
                    self.finish(ctx, Err("Cannot create test browser".into()));
                    return;
                }
            }
        }
        egui::CentralPanel::default().show(ctx, |ui| self.browser.show(ui, frame));
        if let Ok(value) = self.receiver.try_recv() {
            self.pending = false;
            if value == "true" {
                self.stage += 1;
            } else if self.stage != 0 {
                self.finish(
                    ctx,
                    Err(format!(
                        "Native fill assertion failed at stage {}",
                        self.stage
                    )),
                );
                return;
            }
        }
        if self.stage == 4 {
            self.finish(ctx, Ok(()));
            return;
        }
        if self.started.elapsed() > Duration::from_secs(30) {
            self.finish(ctx, Err("Vault acceptance timed out".into()));
            return;
        }
        if !self.pending
            && let Err(e) = self.next()
        {
            self.finish(ctx, Err(e));
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(50));
    }
}
