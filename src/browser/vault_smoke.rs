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
                fill_pending: None,
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
    fill_pending: Option<(crate::browser::FillReply, String, bool)>,
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
    fn fill_check(&mut self, account: &crate::vault::Account, script: &str) -> Result<(), String> {
        let rx = self.browser.fill_account(account)?;
        self.pending = true;
        self.fill_pending = Some((
            rx,
            script.to_owned(),
            matches!(self.stage, 1 | 13 | 19 | 23 | 35),
        ));
        Ok(())
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
                self.fill_check(&account,"document.getElementById('user').value==='synthetic@example.test' && document.getElementById('pass').value==='synthetic-only-secret' && window.submissions===0")
            }
            2=>self.evaluate("(() => { document.getElementById('user').value=''; document.getElementById('pass').value='';const extra=document.createElement('input');extra.type='password';document.querySelector('form').appendChild(extra);return true; })()"),
            3=>{self.fill_check(&account,"[...document.querySelectorAll('input')].every(el=>el.value==='') && window.submissions===0")}
            4=>self.evaluate("(() => { const f=document.querySelector('form'); f.lastElementChild.remove(); f.setAttribute('action','https://different.example.test/receive'); return true; })()"),
            5=>{self.fill_check(&account,"document.getElementById('pass').value==='' && document.getElementById('user').value==='' && window.submissions===0")}
            6=>self.evaluate("(() => { const f=document.querySelector('form'); f.removeAttribute('action'); f.id='login'; const b=document.createElement('button'); b.type='submit'; b.setAttribute('form','login'); b.setAttribute('formaction','https://different.example.test/receive'); b.id='outside'; document.body.appendChild(b); return true; })()"),
            7=>{self.fill_check(&account,"document.getElementById('pass').value==='' && document.getElementById('user').value==='' && window.submissions===0")}
            8=>self.evaluate("(() => { document.getElementById('outside').remove(); document.getElementById('user').addEventListener('input',()=>{ const f=document.createElement('form'); f.action='https://different.example.test/receive';document.body.appendChild(f);f.appendChild(document.getElementById('pass')); }, {once:true}); return true; })()"),
            9=>{self.fill_check(&account,"document.getElementById('pass').value==='' && window.submissions===0")}
            10=>self.evaluate("(() => { const f=document.getElementById('login'); f.appendChild(document.getElementById('pass'));document.getElementById('user').value=''; const base=document.createElement('base');base.href='https://different.example.test/';document.head.appendChild(base);f.setAttribute('action','relative-login'); return true; })()"),
            11=>{self.fill_check(&account,"document.getElementById('pass').value==='' && document.getElementById('user').value==='' && window.submissions===0")}
            12=>self.evaluate("(() => { document.querySelector('base').remove(); return true; })()"),
            13=>{self.fill_check(&account,"document.getElementById('pass').value==='synthetic-only-secret' && document.getElementById('user').value==='synthetic@example.test' && window.submissions===0")}
            14=>self.evaluate("(() => { const f=document.getElementById('login');document.getElementById('user').value='';document.getElementById('pass').value='';const shadow=document.createElement('input');shadow.type='hidden';shadow.name='action';shadow.value=location.href;f.appendChild(shadow);f.setAttribute('action','https://different.example.test/receive');return true; })()"),
            15=>{self.fill_check(&account,"document.getElementById('pass').value==='' && document.getElementById('user').value==='' && window.submissions===0")}
            16=>self.evaluate("(() => { const f=document.getElementById('login');f.setAttribute('action','');f.querySelector('button').setAttribute('formaction','javascript:void(0)');return true; })()"),
            17=>{self.fill_check(&account,"document.getElementById('pass').value==='' && document.getElementById('user').value==='' && window.submissions===0")}
            18=>self.evaluate("(() => { document.getElementById('login').querySelector('button').setAttribute('formaction','relative-login');return true; })()"),
            19=>{self.fill_check(&account,"document.getElementById('pass').value==='synthetic-only-secret' && document.getElementById('user').value==='synthetic@example.test' && window.submissions===0")}
            20=>self.evaluate("(() => { document.getElementById('user').value='';document.getElementById('pass').value='';document.getElementById('login').setAttribute('action','https://different.example.test/receive');window.hooks=0; const original=Object.getOwnPropertyDescriptor;Object.getOwnPropertyDescriptor=function(...args){window.hooks++; return original(...args);};window.URL=function(){this.origin=location.origin;};return true; })()"),
            21=>{self.fill_check(&account,"document.getElementById('pass').value==='' && window.hooks===0 && window.submissions===0")}
            22=>self.evaluate("(() => { document.getElementById('login').setAttribute('action','');return true; })()"),
            23=>{self.fill_check(&account,"document.getElementById('pass').value==='synthetic-only-secret' && window.hooks===0 && window.submissions===0")}
            24=>self.evaluate("(() => { document.getElementById('user').value='';document.getElementById('pass').value='';document.getElementById('pass').autocomplete='new-password';return true; })()"),
            25=>{self.fill_check(&account,"document.getElementById('pass').value==='' && window.submissions===0")}
            26=>self.evaluate("(() => { document.getElementById('pass').autocomplete='current-password';document.getElementById('pass').style.visibility='collapse';return true; })()"),
            27=>{self.fill_check(&account,"document.getElementById('pass').value==='' && window.submissions===0")}
            28=>self.evaluate("(() => {document.getElementById('pass').style.visibility='visible';document.getElementById('login').querySelector('button').setAttribute('formaction','https://different.example.test/receive');return true;})()"),
            29=>{self.fill_check(&account,"document.getElementById('pass').value==='' && window.submissions===0")}
            30=>self.evaluate("(() => {document.getElementById('login').querySelector('button').removeAttribute('formaction');document.getElementById('user').addEventListener('input',()=>document.getElementById('login').setAttribute('action','https://different.example.test/receive'),{once:true});return true;})()"),
            31=>{self.fill_check(&account,"document.getElementById('pass').value==='' && window.submissions===0")}
            32=>self.evaluate("(() => {document.getElementById('login').setAttribute('action','');document.getElementById('user').addEventListener('input',()=>document.getElementById('login').querySelector('button').setAttribute('formaction','https://different.example.test/receive'),{once:true});return true;})()"),
            33=>{self.fill_check(&account,"document.getElementById('pass').value==='' && window.submissions===0")}
            34=>self.evaluate("(() => {document.getElementById('login').querySelector('button').removeAttribute('formaction');document.getElementById('user').value='';const other=document.createElement('form');other.id='other';document.body.appendChild(other);document.getElementById('user').setAttribute('form','other');return true;})()"),
            35=>{self.fill_check(&account,"document.getElementById('user').value==='' && document.getElementById('pass').value==='synthetic-only-secret' && window.hooks===0 && window.submissions===0")}
            36=>self.evaluate("(() => {document.getElementById('pass').value='';document.getElementById('user').removeAttribute('form');return true;})()"),
            37=>{drop(self.browser.fill_account(&account)?);self.evaluate("document.getElementById('user').value==='' && document.getElementById('pass').value==='' && window.submissions===0")}
            38=>self.evaluate("(() => {document.getElementById('login').setAttribute('action','javascript:void(0)');return true;})()"),
            39=>{self.fill_check(&account,"document.getElementById('pass').value==='' && window.submissions===0")}
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
                Ok(view) => {
                    self.browser.clip.reset();
                    self.browser.view = Some(view);
                }
                Err(_) => {
                    self.finish(ctx, Err("Cannot create test browser".into()));
                    return;
                }
            }
        }
        self.browser.begin_frame();
        egui::CentralPanel::default().show(ctx, |ui| self.browser.show(ui, frame));
        let _ = self.browser.sync_presentation(true, None, false);
        if let Some((rx, script, expected)) = self.fill_pending.take() {
            match rx.try_recv() {
                Ok(result) => {
                    if result.is_ok() != expected {
                        self.finish(
                            ctx,
                            Err(format!(
                                "Private fill outcome mismatch at stage {}",
                                self.stage
                            )),
                        );
                        return;
                    }
                    if let Err(error) = self.evaluate(&script) {
                        self.finish(ctx, Err(error));
                        return;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => self.fill_pending = Some((rx, script, expected)),
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.finish(ctx, Err("Private fill reply disconnected".into()));
                    return;
                }
            }
        }
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
        if self.stage == 40 {
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
