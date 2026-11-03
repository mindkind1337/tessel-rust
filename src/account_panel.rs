use crate::{
    account::{Api, Flow, Session, Store, User},
    i18n::tr,
};
use eframe::egui;
use std::{
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};
enum Reply {
    Login(Session),
    Refreshed(User),
    LoggedOut,
}
#[derive(Default)]
pub struct AccountPanel {
    store: Option<Store>,
    flow: Option<Flow>,
    pending: Option<Receiver<Result<Reply, String>>>,
    status: String,
    last_poll: Option<Instant>,
}
impl AccountPanel {
    fn run(&mut self, work: impl FnOnce() -> Result<Reply, String> + Send + 'static) {
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        thread::spawn(move || {
            let result = work();
            if let Err(mpsc::SendError(Ok(Reply::Login(s)))) = tx.send(result)
                && let Ok(api) = Api::production()
            {
                let _ = api.logout(&s.bearer());
            }
        });
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn poll(&mut self) {
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(result) => {
                    self.pending = None;
                    self.status = match result {
                        Ok(Reply::Login(s)) => {
                            let token = s.bearer();
                            match self
                                .store
                                .as_mut()
                                .ok_or("Account is locked".to_string())
                                .and_then(|store| store.set(s))
                            {
                                Ok(()) => "Signed in".into(),
                                Err(e) => {
                                    self.run(move || {
                                        Api::production()?.logout(&token)?;
                                        Ok(Reply::LoggedOut)
                                    });
                                    e
                                }
                            }
                        }
                        Ok(Reply::Refreshed(user)) => match self
                            .store
                            .as_mut()
                            .ok_or("Account is locked".to_string())
                            .and_then(|store| store.update_user(user))
                        {
                            Ok(()) => "Account verified".into(),
                            Err(e) => e,
                        },
                        Ok(Reply::LoggedOut) => "Signed out".into(),
                        Err(e) => e,
                    };
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending = None;
                    self.status = "Account worker stopped".into();
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self
            .last_poll
            .is_some_and(|t| t.elapsed() < Duration::from_millis(250))
        {
            return;
        }
        self.last_poll = Some(Instant::now());
        if let (Some(store), Some(flow)) = (&self.store, &self.flow) {
            match store.complete(flow) {
                Ok(Some(code)) => {
                    let verifier = flow.verifier();
                    self.flow = None;
                    self.status = "Completing sign-in".into();
                    self.run(move || {
                        Api::production()?
                            .exchange(&code, &verifier)
                            .map(Reply::Login)
                    });
                }
                Err(e) => {
                    self.flow = None;
                    self.status = e;
                }
                Ok(None) => {}
            }
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui) {
        ui.heading(tr("Tessel account"));
        ui.label(tr("Your account session stays encrypted on this PC, outside projects and Live Share. Agents cannot access it."));
        ui.label(tr("Native sign-in requires the tessel-rust callback update on the account server. Connectors and cloud sync are not enabled yet."));
        ui.separator();
        if self.store.is_none() {
            if ui.button(tr("Open account")).clicked() {
                match Store::open_default() {
                    Ok(s) => {
                        self.store = Some(s);
                        self.status.clear();
                    }
                    Err(e) => self.status = e,
                }
            }
        } else if self.pending.is_some() {
            ui.spinner();
        } else if self.flow.is_some() {
            ui.label(tr("Finish signing in in your usual browser."));
            if ui.button(tr("Cancel sign-in")).clicked() {
                if let (Some(store), Some(flow)) = (&self.store, self.flow.take()) {
                    store.cancel(&flow);
                }
                self.status = "Sign-in cancelled".into();
            }
        } else if self.store.as_ref().is_some_and(|s| s.session.is_some()) {
            let session = self
                .store
                .as_ref()
                .and_then(|s| s.session.as_ref())
                .expect("session checked");
            ui.label(session.user.name.as_deref().unwrap_or(""));
            ui.label(&session.user.email);
            let token = session.bearer();
            if ui.button(tr("Refresh account")).clicked() {
                self.run(move || Api::production()?.refresh(&token).map(Reply::Refreshed));
            }
            if ui.button(tr("Sign out")).clicked() {
                match self.store.as_mut().expect("opened").clear() {
                    Ok(Some(s)) => self.run(move || {
                        Api::production()?.logout(&s.bearer())?;
                        Ok(Reply::LoggedOut)
                    }),
                    Ok(None) => {}
                    Err(e) => self.status = e,
                }
            }
        } else if ui.button(tr("Sign in with your browser")).clicked() {
            let result = crate::account::register_protocol()
                .and_then(|_| self.store.as_ref().expect("opened").begin());
            match result {
                Ok(flow) => {
                    if webbrowser::open(&flow.login_url()).is_ok() {
                        self.flow = Some(flow);
                        self.status.clear();
                    } else {
                        self.store.as_ref().expect("opened").cancel(&flow);
                        self.status = "Could not open the sign-in browser".into();
                    }
                }
                Err(e) => self.status = e,
            }
        }
        if !self.status.is_empty() {
            ui.label(tr(&self.status));
        }
        if self.pending.is_some() || self.flow.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
    }
}
impl Drop for AccountPanel {
    fn drop(&mut self) {
        if let (Some(store), Some(flow)) = (&self.store, &self.flow) {
            store.cancel(flow);
        }
    }
}
