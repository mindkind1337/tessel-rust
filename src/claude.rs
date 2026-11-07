//! Native Claude Code stream-json chat. The external CLI owns authentication.
use crate::i18n::tr;
use eframe::egui;
use serde_json::{Value, json};
use std::{
    collections::{HashSet, VecDeque},
    path::PathBuf,
    time::{Duration, Instant},
};
mod questions;
mod transport;
const TEXT_LIMIT: usize = 128 * 1024;
#[derive(Clone)]
struct Approval {
    id: String,
    tool: String,
    input: Value,
}
pub struct Chat {
    pub pane: u64,
    pub project: u64,
    pub saved_session: Option<String>,
    cwd: PathBuf,
    transport: Option<transport::Transport>,
    session: String,
    resume: String,
    pub options: crate::native_chat_options::Options,
    pub defaults: Option<Value>,
    pub options_commit: bool,
    composer: String,
    transcript: VecDeque<(String, String)>,
    approvals: VecDeque<Approval>,
    questions: Option<questions::Questions>,
    seen: HashSet<String>,
    ready: bool,
    active: bool,
    interrupting: bool,
    initializing: Option<Instant>,
    stream_id: Option<String>,
    stream_row: Option<usize>,
    error: String,
    status: String,
}
pub fn valid_uuid(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}
pub(crate) fn uuid() -> Result<String, String> {
    let mut b = [0; 16];
    getrandom::fill(&mut b).map_err(|_| "Cannot create session identifier")?;
    b[6] = (b[6] & 15) | 64;
    b[8] = (b[8] & 63) | 128;
    let s = b.iter().map(|b| format!("{b:02x}")).collect::<String>();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &s[..8],
        &s[8..12],
        &s[12..16],
        &s[16..20],
        &s[20..]
    ))
}
fn bounded(text: &str) -> String {
    if text.len() <= TEXT_LIMIT {
        return text.into();
    }
    let mut end = TEXT_LIMIT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[display truncated]", &text[..end])
}
impl Chat {
    #[cfg(feature = "visual-test")]
    pub fn fixture(project: u64, cwd: PathBuf) -> Self {
        let mut chat = Self::new(project, cwd, None);
        chat.status = "Approval required".into();
        chat.push(
            "You",
            "Crée un fichier exemple.txt contenant Bonjour Québec.",
        );
        chat.push(
            "Claude",
            "Je vais créer le fichier demandé. Voici l’action à approuver.",
        );
        chat.approvals.push_back(Approval {
            id: "fixture-only".into(),
            tool: "Write".into(),
            input: json!({"file_path":"exemple.txt","content":"Bonjour Québec"}),
        });
        if std::env::args().any(|arg| arg == "--claude-questions-fixture") {
            chat.approvals.clear();
            chat.questions = questions::Questions::parse(
                "fixture-question",
                &json!({"questions":[{
                    "header":"Format", "question":"Quels formats voulez-vous?", "multiSelect":true,
                    "options":[{"label":"PDF","description":"Document à partager"},{"label":"HTML","description":"Page consultable"}]
                }]}),
            );
        }
        chat
    }
    pub fn new(project: u64, cwd: PathBuf, saved: Option<String>) -> Self {
        Self {
            pane: 0,
            project,
            cwd,
            resume: saved.clone().unwrap_or_default(),
            saved_session: saved,
            transport: None,
            session: String::new(),
            options: Default::default(),
            defaults: None,
            options_commit: false,
            composer: String::new(),
            transcript: VecDeque::new(),
            approvals: VecDeque::new(),
            questions: None,
            seen: HashSet::new(),
            ready: false,
            active: false,
            interrupting: false,
            initializing: None,
            stream_id: None,
            stream_row: None,
            error: String::new(),
            status: "Not connected".into(),
        }
    }
    pub fn append_composer(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if !self.composer.is_empty() && !self.composer.ends_with('\n') {
            self.composer.push('\n');
        }
        self.composer.push_str(text);
    }
    fn commit_options(&mut self) {
        self.options.normalize();
        if self.options.valid_for("claude") {
            self.options_commit = true;
            if self.error == tr("Invalid model or effort") {
                self.error.clear();
            }
        } else {
            self.options_commit = false;
            self.error = tr("Invalid model or effort").into();
        }
    }
    pub fn busy(&self) -> bool {
        self.transport.is_some() || self.active || self.initializing.is_some()
    }
    pub fn waiting_for_user(&self) -> bool {
        !self.approvals.is_empty() || self.questions.is_some()
    }
    pub fn working(&self) -> bool {
        !self.waiting_for_user() && (self.active || self.initializing.is_some())
    }
    pub fn process_id(&self) -> Option<u32> {
        self.transport.as_ref().map(|t| t.child_id())
    }

    pub fn gpui_snapshot(&self) -> crate::chat::NativeChatViewSnapshot {
        let approval = self
            .approvals
            .front()
            .map(|a| crate::chat::NativeChatApprovalView {
                title: a.tool.clone(),
                details_json: serde_json::to_string_pretty(&a.input).unwrap_or_default(),
                can_accept: true,
            });
        let (questions, questions_can_submit) = match &self.questions {
            Some(q) => (q.views(), q.can_submit()),
            None => (Vec::new(), false),
        };
        let thread_id = self
            .saved_session
            .clone()
            .or_else(|| (!self.session.is_empty()).then(|| self.session.clone()));
        crate::chat::NativeChatViewSnapshot {
            provider: "claude",
            cwd_display: self.cwd.display().to_string(),
            thread_id,
            resume_draft: self.resume.clone(),
            connected: self.transport.is_some(),
            ready: self.ready,
            working: self.working(),
            can_load_older: false,
            status: self.status.clone(),
            error: self.error.clone(),
            model: self.options.model.clone(),
            effort: self.options.effort.clone(),
            composer: self.composer.clone(),
            messages: self.transcript.iter().cloned().collect(),
            approval,
            questions,
            questions_can_submit,
        }
    }

    pub fn gpui_connect(&mut self) {
        if self.transport.is_none()
            && let Err(e) = self.connect()
        {
            self.error = e;
        }
    }

    pub fn gpui_stop(&mut self) {
        self.stop();
    }

    pub fn gpui_new_conversation(&mut self) {
        if self.transport.is_none() {
            self.saved_session = None;
            self.resume.clear();
            self.transcript.clear();
        }
    }

    pub fn gpui_interrupt(&mut self) {
        if self.active {
            self.interrupting = true;
            if let Some(questions) = self.questions.take() {
                self.deny(&questions.id);
            }
            for approval in self.approvals.drain(..).collect::<Vec<_>>() {
                self.deny(&approval.id);
            }
            if let Err(e) = self.send(json!({"type":"control_request","request_id":"tessel-interrupt","request":{"subtype":"interrupt"}})) {
                self.error = e;
                self.stop();
            }
        }
    }

    pub fn gpui_set_composer(&mut self, text: String) {
        self.composer = text;
    }

    pub fn gpui_send_message(&mut self) {
        if let Err(e) = self.begin() {
            self.error = e;
        }
    }

    pub fn gpui_set_model(&mut self, model: String) {
        if self.transport.is_none() {
            self.options.model = model;
            self.commit_options();
        }
    }

    pub fn gpui_set_effort(&mut self, effort: String) {
        if self.transport.is_none() {
            self.options.effort = effort;
            self.commit_options();
        }
    }

    pub fn gpui_set_resume(&mut self, resume: String) {
        if self.transport.is_none() {
            self.resume = resume;
        }
    }

    pub fn gpui_respond_approval(&mut self, allow: bool) {
        if let Some(front) = self.approvals.front().cloned() {
            if self.transport.is_some() {
                self.respond(&front.id, allow);
            } else {
                self.approvals.pop_front();
            }
        }
    }

    pub fn gpui_select_question_option(&mut self, q_idx: usize, opt_idx: usize) {
        if let Some(q) = &mut self.questions {
            q.select_option(q_idx, opt_idx);
        }
    }

    pub fn gpui_set_question_text(&mut self, q_idx: usize, text: String) {
        if let Some(q) = &mut self.questions {
            q.set_text(q_idx, text);
        }
    }

    pub fn gpui_submit_questions(&mut self, skip: bool) {
        let Some(q) = &self.questions else {
            return;
        };
        let payload = if skip {
            Some(json!({"behavior":"deny","message":"The owner skipped these questions."}))
        } else {
            q.build_answer()
        };
        let Some(answer) = payload else {
            return;
        };
        let id = q.id.clone();
        self.questions = None;
        if self.transport.is_some()
            && let Err(e) = self.answer(&id, answer)
        {
            self.error = e;
            self.stop();
        }
    }

    #[cfg(test)]
    pub fn process_protocol_frame_for_test(&mut self, value: Value) {
        if value["type"] == "control_request" {
            self.active = true;
        }
        self.frame(value);
    }
    fn push(&mut self, role: &str, text: &str) {
        self.transcript.push_back((role.into(), bounded(text)));
        while self.transcript.len() > 500
            || self.transcript.iter().map(|r| r.1.len()).sum::<usize>() > 2 * 1024 * 1024
        {
            self.transcript.pop_front();
            self.stream_row = self.stream_row.and_then(|i| i.checked_sub(1));
        }
    }
    fn send(&self, value: Value) -> Result<(), String> {
        self.transport
            .as_ref()
            .ok_or("Claude is not connected")?
            .send(value)
    }
    fn connect(&mut self) -> Result<(), String> {
        if self.transport.is_some() {
            return Err("Stop the current connection first".into());
        }
        if !self.resume.is_empty() && !valid_uuid(&self.resume) {
            return Err("Resume ID must be a UUID".into());
        }
        let options = self.options.effective(self.defaults.as_ref());
        if !options.valid_for("claude") {
            return Err(format!(
                "{}: {} / {}",
                tr("Invalid model or effort"),
                options.model.chars().take(160).collect::<String>(),
                options.effort.chars().take(20).collect::<String>()
            ));
        }
        self.commit_options();
        let resume = !self.resume.is_empty();
        self.session = if resume { self.resume.clone() } else { uuid()? };
        self.transport = Some(transport::Transport::start(
            &self.cwd,
            &self.session,
            resume,
            &options.model,
            &options.effort,
        )?);
        self.ready = false;
        self.active = false;
        self.interrupting = false;
        self.seen.clear();
        self.approvals.clear();
        self.questions = None;
        self.error.clear();
        self.status = "Connecting".into();
        self.initializing = Some(Instant::now());
        if let Err(e)=self.send(json!({"type":"control_request","request_id":"tessel-init","request":{"subtype":"initialize"}})){self.stop();return Err(e);}
        Ok(())
    }
    fn stop(&mut self) {
        self.transport.take();
        self.ready = false;
        self.active = false;
        self.interrupting = false;
        self.initializing = None;
        self.approvals.clear();
        self.questions = None;
        self.stream_id = None;
        self.stream_row = None;
        self.status = "Stopped".into();
        if let Some(id) = &self.saved_session {
            self.resume = id.clone();
        }
    }
    fn answer(&self, id: &str, body: Value) -> Result<(), String> {
        self.send(json!({"type":"control_response","response":{"subtype":"success","request_id":id,"response":body}}))
    }
    fn deny(&self, id: &str) {
        let _=self.answer(id,json!({"behavior":"deny","message":"This request is unsupported, stale or was declined in Tessel Rust."}));
    }
    fn respond(&mut self, id: &str, allow: bool) {
        let Some(index) = self.approvals.iter().position(|p| p.id == id) else {
            return;
        };
        let approval = self.approvals.remove(index).unwrap();
        let body = if allow && self.active {
            json!({"behavior":"allow","updatedInput":approval.input})
        } else {
            json!({"behavior":"deny","message":"The owner declined this request."})
        };
        if let Err(e) = self.answer(id, body) {
            self.error = e;
            self.stop();
        }
    }
    fn begin(&mut self) -> Result<(), String> {
        if !self.ready || self.active {
            return Err("Claude is not ready for another message".into());
        }
        let text = self.composer.trim();
        if text.is_empty() || text.len() > 65536 {
            return Err("Message must be 1 to 65536 bytes".into());
        }
        self.send(json!({"type":"user","uuid":uuid()?,"session_id":self.session,"parent_tool_use_id":null,"message":{"role":"user","content":[{"type":"text","text":text}]}}))?;
        let text = std::mem::take(&mut self.composer);
        self.push("You", &text);
        self.active = true;
        self.interrupting = false;
        self.stream_row = None;
        self.stream_id = None;
        self.seen.clear();
        self.error.clear();
        self.status = "Working".into();
        Ok(())
    }
    fn frame(&mut self, value: Value) {
        if let Some(id) = value["session_id"].as_str()
            && !id.is_empty()
            && id != self.session
        {
            self.error = "Claude returned another session; connection stopped".into();
            self.stop();
            return;
        }
        match value["type"].as_str().unwrap_or("") {
            "control_response" => {
                let response = &value["response"];
                if response["request_id"] == "tessel-init" && self.initializing.is_some() {
                    self.initializing = None;
                    let account = &response["response"]["account"];
                    if response["subtype"] != "success" {
                        self.error =
                            "Claude initialization failed; check its CLI installation and sign-in"
                                .into();
                        self.stop();
                    } else if account["tokenSource"] == "none"
                        && (account["apiKeySource"].is_null()
                            || account["apiKeySource"] == "none"
                            || account["apiKeySource"] == "")
                    {
                        self.error = "Sign in using the Claude CLI, then reconnect".into();
                        self.stop();
                    } else {
                        self.ready = true;
                        self.status = "Ready".into();
                    }
                }
            }
            "control_request" => {
                let Some(id) = value["request_id"]
                    .as_str()
                    .filter(|id| !id.is_empty() && id.len() <= 200)
                else {
                    return;
                };
                let request = &value["request"];
                if request["subtype"] != "can_use_tool" {
                    let _=self.send(json!({"type":"control_response","response":{"subtype":"error","request_id":id,"error":"Unsupported native host request"}}));
                    return;
                }

                let valid = self.active
                    && !self.interrupting
                    && request["subtype"] == "can_use_tool"
                    && request["tool_name"]
                        .as_str()
                        .is_some_and(|t| !t.is_empty() && t.len() <= 200)
                    && request["input"].is_object()
                    && request["input"].to_string().len() <= 65536
                    && self.approvals.len() < 8
                    && self.seen.len() < 1024;
                if !valid {
                    self.deny(id);
                    return;
                }
                if !self.seen.insert(id.into()) {
                    return;
                }
                if request["tool_name"] == "AskUserQuestion" {
                    if self.questions.is_none() {
                        self.questions = questions::Questions::parse(id, &request["input"]);
                        if self.questions.is_some() {
                            return;
                        }
                    }
                    self.deny(id);
                    return;
                }
                self.approvals.push_back(Approval {
                    id: id.into(),
                    tool: request["tool_name"].as_str().unwrap().into(),
                    input: request["input"].clone(),
                });
            }
            "system" if value["subtype"] == "init" => {
                if value["permissionMode"]
                    .as_str()
                    .is_some_and(|mode| mode != "default")
                {
                    self.error = "Claude returned an unexpected permission mode".into();
                    self.stop();
                }
            }
            "control_cancel_request" => {
                if let Some(id) = value["request_id"].as_str() {
                    self.approvals.retain(|p| p.id != id);
                    if self.questions.as_ref().is_some_and(|q| q.id == id) {
                        self.questions = None;
                    }
                }
            }
            "stream_event" if self.active => {
                let event = &value["event"];
                if event["type"] == "message_start" {
                    self.stream_id = event["message"]["id"].as_str().map(str::to_owned);
                    self.push("Claude", "");
                    self.stream_row = Some(self.transcript.len() - 1);
                } else if event["type"] == "content_block_delta"
                    && event["delta"]["type"] == "text_delta"
                    && let Some(text) = event["delta"]["text"].as_str()
                {
                    if self.stream_row.is_none() {
                        self.push("Claude", "");
                        self.stream_row = Some(self.transcript.len() - 1);
                    }
                    if let Some(row) = self.stream_row.and_then(|i| self.transcript.get_mut(i))
                        && row.1.len() < TEXT_LIMIT
                    {
                        row.1.push_str(text);
                        row.1 = bounded(&row.1);
                    }
                }
            }
            "assistant" if self.active => {
                self.saved_session = Some(self.session.clone());
                let text = value["message"]["content"]
                    .as_array()
                    .map(|blocks| {
                        blocks
                            .iter()
                            .filter(|b| b["type"] == "text")
                            .filter_map(|b| b["text"].as_str())
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();
                if !text.is_empty() {
                    if value["message"]["id"].as_str() == self.stream_id.as_deref()
                        && let Some(row) = self.stream_row.and_then(|i| self.transcript.get_mut(i))
                    {
                        row.1 = bounded(&text);
                    } else {
                        self.push("Claude", &text);
                    }
                }
                self.stream_row = None;
                self.stream_id = None;
            }
            "result" if self.active => {
                self.saved_session = Some(self.session.clone());
                self.active = false;
                self.interrupting = false;
                self.approvals.clear();
                self.questions = None;
                self.stream_row = None;
                self.stream_id = None;
                if value["is_error"] != false || value["subtype"] != "success" {
                    self.error = bounded(
                        &value["errors"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .filter_map(Value::as_str)
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            })
                            .unwrap_or_else(|| "Claude reported a failed turn".into()),
                    );
                    self.status = "Failed".into();
                } else {
                    self.status = "Ready".into();
                }
            }
            _ => {}
        }
    }
    pub fn poll(&mut self) {
        for _ in 0..64 {
            let Some(transport) = &self.transport else {
                break;
            };
            let Ok(frame) = transport.reader.try_recv() else {
                break;
            };
            match frame {
                Ok(value) => {
                    let completed = value["type"] == "result";
                    self.frame(value);
                    if completed {
                        break;
                    }
                }
                Err(error) => {
                    if self.error.is_empty() {
                        self.error = error;
                    }
                    self.stop();
                    break;
                }
            }
        }
        if self
            .initializing
            .is_some_and(|t| t.elapsed() > Duration::from_secs(30))
        {
            self.error = "Claude initialization timed out".into();
            self.stop();
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui) {
        ui.heading(tr("Native Claude chat"));
        ui.label(tr("Uses your installed Claude CLI sign-in and permission controls. Project hooks, customizations and external MCP servers are disabled in this native connection."));
        ui.horizontal_wrapped(|ui| {
            ui.add_enabled_ui(self.transport.is_none(), |ui| {
                ui.label(tr("Model"));
                if ui
                    .add(egui::TextEdit::singleline(&mut self.options.model).desired_width(140.0))
                    .lost_focus()
                {
                    self.commit_options();
                }
                egui::ComboBox::from_id_salt("claude-effort")
                    .selected_text(if self.options.effort.is_empty() {
                        tr("Default")
                    } else {
                        &self.options.effort
                    })
                    .show_ui(ui, |ui| {
                        for effort in ["", "low", "medium", "high", "xhigh", "max"] {
                            if ui
                                .selectable_value(
                                    &mut self.options.effort,
                                    effort.into(),
                                    if effort.is_empty() {
                                        tr("Default")
                                    } else {
                                        effort
                                    },
                                )
                                .changed()
                            {
                                self.commit_options();
                            }
                        }
                    });
                ui.label(tr("Resume ID"));
                ui.add(egui::TextEdit::singleline(&mut self.resume).desired_width(220.0));
                if ui.button(tr("Connect")).clicked()
                    && let Err(e) = self.connect()
                {
                    self.error = e;
                }
                if ui.button(tr("New conversation")).clicked() {
                    self.saved_session = None;
                    self.resume.clear();
                    self.transcript.clear();
                }
            });
            if self.transport.is_some() && ui.button(tr("Stop")).clicked() {
                self.stop();
            }
        });
        ui.label(tr(&self.status));
        if !self.error.is_empty() {
            ui.colored_label(egui::Color32::LIGHT_RED, &self.error);
        }
        let jump_waiting = ui.is_enabled()
            && (!self.approvals.is_empty() || self.questions.is_some())
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::ALT, egui::Key::A));
        if let Some(approval) = self.approvals.front().cloned() {
            let card = ui.group(|ui| {
                ui.strong(format!("{}: {}", tr("Approval required"), approval.tool));
                egui::ScrollArea::vertical()
                    .id_salt("claude-approval")
                    .max_height(170.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                serde_json::to_string_pretty(&approval.input).unwrap_or_default(),
                            )
                            .selectable(true),
                        );
                    });
                ui.horizontal(|ui| {
                    let allow = ui.button(tr("Allow once"));
                    if jump_waiting {
                        allow.request_focus();
                    }
                    if allow.clicked() {
                        self.respond(&approval.id, true);
                    }
                    if ui.button(tr("Deny")).clicked() {
                        self.respond(&approval.id, false);
                    }
                });
            });
            if jump_waiting {
                card.response.scroll_to_me(Some(egui::Align::Center));
            }
        }
        if let Some(questions) = &mut self.questions {
            let card = ui.scope(|ui| questions.show(ui));
            if jump_waiting && self.approvals.is_empty() {
                card.response.scroll_to_me(Some(egui::Align::Center));
            }
            if let Some(answer) = card.inner {
                let id = questions.id.clone();
                self.questions = None;
                if let Err(error) = self.answer(&id, answer) {
                    self.error = error;
                    self.stop();
                }
            }
        }
        egui::ScrollArea::vertical()
            .id_salt(("claude-chat", self.project))
            .stick_to_bottom(true)
            .max_height((ui.available_height() - 100.0).max(80.0))
            .show(ui, |ui| {
                for (role, text) in &self.transcript {
                    ui.strong(tr(role));
                    ui.add(egui::Label::new(text).selectable(true));
                    ui.separator();
                }
            });
        ui.add_enabled_ui(self.ready && !self.active, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut self.composer)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            );
            if ui.button(tr("Send")).clicked()
                && let Err(e) = self.begin()
            {
                self.error = e;
            }
        });
        if self.active && ui.button(tr("Interrupt")).clicked() {
            self.interrupting = true;
            if let Some(questions) = self.questions.take() {
                self.deny(&questions.id);
            }
            for approval in self.approvals.drain(..).collect::<Vec<_>>() {
                self.deny(&approval.id);
            }
            if let Err(e)=self.send(json!({"type":"control_request","request_id":"tessel-interrupt","request":{"subtype":"interrupt"}})){self.error=e;self.stop();}
        }
        if self.active || self.initializing.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
    }
}
#[cfg(test)]
mod tests;

pub fn smoke(model_turn: bool) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let mut chat = Chat::new(1, directory.path().to_owned(), None);
    chat.connect()?;
    let deadline = Instant::now() + Duration::from_secs(if model_turn { 120 } else { 35 });
    let mut sent = false;
    while Instant::now() < deadline {
        chat.poll();
        if chat.ready {
            if model_turn {
                if !sent {
                    chat.composer = "This is a synthetic native transport acceptance test. Do not use any tools or access files. Reply with exactly TESSEL_RUST_NATIVE_OK.".into();
                    chat.begin()?;
                    sent = true;
                } else if !chat.active {
                    if !chat.error.is_empty() {
                        let lower = chat.error.to_ascii_lowercase();
                        let categories: Vec<&str> = [
                            "auth",
                            "login",
                            "token",
                            "rate",
                            "limit",
                            "usage",
                            "permission",
                            "billing",
                            "credit",
                            "network",
                            "fetch",
                            "model",
                            "session",
                            "api",
                            "oauth",
                            "expired",
                            "401",
                            "403",
                            "429",
                            "overloaded",
                            "invalid",
                            "not found",
                        ]
                        .into_iter()
                        .filter(|word| lower.contains(word))
                        .collect();
                        return Err(format!("Claude model turn failed (diagnostic categories: {}); provider details withheld", categories.join(", ")).into());
                    }
                    if !chat.transcript.iter().any(|(role, text)| {
                        role == "Claude" && text.trim() == "TESSEL_RUST_NATIVE_OK"
                    }) {
                        return Err("Claude model turn returned an unexpected test response".into());
                    }
                    chat.stop();
                    println!("Claude native synthetic model turn passed; no tools approved");
                    return Ok(());
                }
                for approval in chat.approvals.drain(..).collect::<Vec<_>>() {
                    chat.deny(&approval.id);
                }
                if let Some(question) = chat.questions.take() {
                    chat.deny(&question.id);
                }
            } else {
                chat.stop();
                println!("Claude native initialization passed; no prompt or model turn sent");
                return Ok(());
            }
        }
        if !chat.error.is_empty() {
            let category = if chat.error.contains("session") {
                "session mismatch"
            } else if chat.error.contains("Sign in") {
                "sign-in required"
            } else if chat.error.contains("closed") {
                "process closed"
            } else if chat.error.contains("permission mode") {
                "permission mode rejected"
            } else {
                "initialization or transport failure"
            };
            return Err(format!(
                "Claude native acceptance failed ({category}); no provider details logged"
            )
            .into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err("Claude native initialization timed out".into())
}
