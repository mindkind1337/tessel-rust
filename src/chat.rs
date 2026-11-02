//! Codex's documented stdio app-server protocol. No token copying or shell interpolation.
use eframe::egui;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
    time::{Duration, Instant},
};

const MAX_FRAME: usize = 8 * 1024 * 1024;
const MAX_TRANSCRIPT: usize = 2 * 1024 * 1024;

pub fn executable() -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        let direct = dir.join(if cfg!(windows) { "codex.exe" } else { "codex" });
        if direct.is_file() {
            return Some(direct);
        }
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            let npm=dir.join("node_modules/@openai/codex/node_modules/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/bin/codex.exe");
            if npm.is_file() {
                return Some(npm);
            }
        }
    }
    None
}

struct Rpc {
    child: Child,
    writer: SyncSender<Value>,
    reader: Receiver<Result<Value, String>>,
}
impl Rpc {
    fn start(executable: &Path, cwd: &Path) -> Result<Self, String> {
        let mut command = Command::new(executable);
        command
            .arg("app-server")
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn().map_err(|e| e.to_string())?;
        let mut stdin = child.stdin.take().ok_or("No Codex input pipe")?;
        let stdout = child.stdout.take().ok_or("No Codex output pipe")?;
        let (writer, write_rx) = mpsc::sync_channel::<Value>(64);
        let (read_tx, reader) = mpsc::sync_channel(128);
        thread::spawn(move || {
            while let Ok(value) = write_rx.recv() {
                let Ok(mut bytes) = serde_json::to_vec(&value) else {
                    break;
                };
                bytes.push(b'\n');
                if stdin.write_all(&bytes).and_then(|_| stdin.flush()).is_err() {
                    break;
                }
            }
        });
        thread::spawn(move || {
            let mut input = BufReader::new(stdout);
            loop {
                let mut bytes = Vec::new();
                match input
                    .by_ref()
                    .take((MAX_FRAME + 1) as u64)
                    .read_until(b'\n', &mut bytes)
                {
                    Ok(0) => {
                        let _ = read_tx.send(Err("Codex process closed the connection.".into()));
                        break;
                    }
                    Ok(_) if bytes.len() > MAX_FRAME => {
                        let _ = read_tx.send(Err("Codex frame exceeded 8 MiB.".into()));
                        break;
                    }
                    Ok(_) => {
                        let value = serde_json::from_slice(&bytes)
                            .map_err(|_| "Invalid JSON frame from Codex.".to_string());
                        if read_tx.send(value).is_err() {
                            break;
                        }
                    }
                    Err(_) => {
                        let _ = read_tx.send(Err("Codex output pipe failed.".into()));
                        break;
                    }
                }
            }
        });
        Ok(Self {
            child,
            writer,
            reader,
        })
    }
    fn send(&self, value: Value) -> Result<(), String> {
        self.writer
            .try_send(value)
            .map_err(|_| "Codex transport is busy or disconnected.".into())
    }
}
impl Drop for Rpc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Clone)]
struct Approval {
    id: Value,
    method: String,
    params: Value,
}
pub struct ToolCall {
    pub id: Value,
    pub name: String,
    pub arguments: Value,
}
pub struct Chat {
    pub project: u64,
    pub cwd: PathBuf,
    pub thread_id: Option<String>,
    rpc: Option<Rpc>,
    pending: HashMap<u64, (String, Instant)>,
    next_id: u64,
    pub status: String,
    pub error: String,
    composer: String,
    transcript: VecDeque<(String, String)>,
    items: HashMap<String, Value>,
    approvals: VecDeque<Approval>,
    pub tool_calls: VecDeque<ToolCall>,
    turn: Option<String>,
    ready: bool,
    starting_turn: bool,
    model: String,
    effort: String,
    resume: String,
}

fn safe_posture(value: &Value) -> bool {
    let policy = value.get("approvalPolicy").and_then(Value::as_str);
    let sandbox = value.get("sandbox").or_else(|| value.get("sandboxPolicy"));
    let sandbox =
        sandbox.and_then(|v| v.as_str().or_else(|| v.get("type").and_then(Value::as_str)));
    policy == Some("on-request")
        && matches!(
            sandbox,
            Some("workspace-write" | "workspaceWrite" | "read-only" | "readOnly")
        )
        && value
            .get("approvalsReviewer")
            .and_then(Value::as_str)
            .is_none_or(|v| v == "user")
}
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 100 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

impl Chat {
    pub fn new(project: u64, cwd: PathBuf) -> Self {
        Self {
            project,
            cwd,
            thread_id: None,
            rpc: None,
            pending: HashMap::new(),
            next_id: 1,
            status: "Not connected".into(),
            error: String::new(),
            composer: String::new(),
            transcript: VecDeque::new(),
            items: HashMap::new(),
            approvals: VecDeque::new(),
            tool_calls: VecDeque::new(),
            turn: None,
            ready: false,
            starting_turn: false,
            model: String::new(),
            effort: "medium".into(),
            resume: String::new(),
        }
    }
    fn request(&mut self, method: &str, params: Value) -> Result<(), String> {
        let id = self.next_id;
        self.next_id += 1;
        self.rpc
            .as_ref()
            .ok_or("Codex is not connected")?
            .send(json!({"id":id,"method":method,"params":params}))?;
        self.pending.insert(id, (method.into(), Instant::now()));
        Ok(())
    }
    fn push(&mut self, role: &str, text: String) {
        self.transcript.push_back((role.into(), text));
        while self.transcript.len() > 1
            && (self.transcript.len() > 500
                || self.transcript.iter().map(|m| m.1.len()).sum::<usize>() > MAX_TRANSCRIPT)
        {
            self.transcript.pop_front();
        }
    }
    fn start(&mut self) {
        self.error.clear();
        let Some(exe) = executable() else {
            self.error = "Native Codex executable not found. Install the Codex CLI first.".into();
            return;
        };
        match Rpc::start(&exe, &self.cwd) {
            Ok(rpc) => self.rpc = Some(rpc),
            Err(e) => {
                self.error = e;
                return;
            }
        }
        self.ready = false;
        self.pending.clear();
        self.status = "Initializing Codex…".into();
        if let Err(e)=self.request("initialize",json!({"clientInfo":{"name":"tessel_rust","title":"Tessel Rust","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}})){self.error=e;}
    }
    fn stop(&mut self) {
        self.rpc = None;
        self.ready = false;
        self.turn = None;
        self.starting_turn = false;
        self.pending.clear();
        self.approvals.clear();
        self.tool_calls.clear();
        self.status = "Stopped".into();
    }
    pub fn busy(&self) -> bool {
        self.rpc.is_some()
    }
    fn send_message(&mut self) {
        if !self.ready || self.composer.trim().is_empty() || self.starting_turn {
            return;
        }
        if self.composer.len() > 128 * 1024 {
            self.error = "Message exceeds 128 KiB.".into();
            return;
        }
        let text = self.composer.clone();
        let input = json!([{"type":"text","text":text,"text_elements":[]}]);
        let result = if let Some(turn) = &self.turn {
            self.request(
                "turn/steer",
                json!({"threadId":self.thread_id,"expectedTurnId":turn,"input":input}),
            )
        } else {
            let mut params = json!({"threadId":self.thread_id,"input":input,"cwd":self.cwd,"approvalPolicy":"on-request","approvalsReviewer":"user","sandboxPolicy":{"type":"workspaceWrite","writableRoots":[self.cwd],"networkAccess":false,"excludeTmpdirEnvVar":false,"excludeSlashTmp":false},"effort":self.effort});
            if !self.model.trim().is_empty() {
                params["model"] = json!(self.model.trim());
            }
            self.request("turn/start", params)
        };
        match result {
            Ok(()) => {
                if self.turn.is_none() {
                    self.starting_turn = true;
                }
                self.push("You", text);
                self.composer.clear();
                self.status = "Working".into();
            }
            Err(e) => self.error = e,
        }
    }
    fn frame(&mut self, value: Value) {
        if let Some(method) = value.get("method").and_then(Value::as_str) {
            let p = value.get("params").cloned().unwrap_or(Value::Null);
            if let Some(id) = value.get("id") {
                let same_thread =
                    p.get("threadId").and_then(Value::as_str) == self.thread_id.as_deref();
                let same_turn = p.get("turnId").and_then(Value::as_str) == self.turn.as_deref()
                    && self.turn.is_some();
                if method == "item/tool/call"
                    && same_thread
                    && same_turn
                    && self.tool_calls.len() < 16
                {
                    self.tool_calls.push_back(ToolCall {
                        id: id.clone(),
                        name: p.get("tool").and_then(Value::as_str).unwrap_or("").into(),
                        arguments: p.get("arguments").cloned().unwrap_or(Value::Null),
                    });
                    return;
                }
                if matches!(
                    method,
                    "item/commandExecution/requestApproval" | "item/fileChange/requestApproval"
                ) && same_thread
                    && same_turn
                    && self.approvals.len() < 16
                {
                    self.approvals.push_back(Approval {
                        id: id.clone(),
                        method: method.into(),
                        params: p,
                    });
                } else if let Some(rpc) = &self.rpc {
                    let _=rpc.send(json!({"id":id,"error":{"code":-32601,"message":"This request is unsupported or outside the active conversation."}}));
                }
                return;
            }
            if p.get("threadId")
                .and_then(Value::as_str)
                .is_some_and(|id| Some(id) != self.thread_id.as_deref())
            {
                return;
            }
            match method {
                "turn/started" => {
                    self.turn = p
                        .pointer("/turn/id")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    self.starting_turn = false;
                }
                "turn/completed" => {
                    self.turn = None;
                    self.starting_turn = false;
                    self.approvals.clear();
                    self.status = p
                        .pointer("/turn/status")
                        .and_then(Value::as_str)
                        .unwrap_or("Finished")
                        .into();
                    if let Some(message) = p.pointer("/turn/error/message").and_then(Value::as_str)
                    {
                        self.error = message.into();
                    }
                }
                "item/agentMessage/delta" => {
                    let delta = p.get("delta").and_then(Value::as_str).unwrap_or("");
                    let key = p
                        .get("itemId")
                        .and_then(Value::as_str)
                        .unwrap_or("assistant");
                    let role = format!("Codex · {key}");
                    if let Some(last) = self.transcript.back_mut()
                        && last.0 == role
                    {
                        if last.1.len() + delta.len() <= MAX_TRANSCRIPT {
                            last.1.push_str(delta);
                        }
                    } else {
                        self.push(&role, delta.into());
                    }
                }
                "item/started" | "item/completed" => {
                    if let Some(item) = p.get("item")
                        && let Some(id) = item.get("id").and_then(Value::as_str)
                    {
                        if self.items.len() >= 200 {
                            self.items.clear();
                        }
                        self.items.insert(id.into(), item.clone());
                        if method == "item/completed"
                            && item.get("type").and_then(Value::as_str) != Some("agentMessage")
                        {
                            self.push(
                                "Tool / event",
                                serde_json::to_string_pretty(item).unwrap_or_default(),
                            );
                        }
                    }
                }
                "serverRequest/resolved" => {
                    self.approvals.retain(|a| Some(&a.id) != p.get("requestId"));
                }
                "thread/settings/updated" => {
                    if !safe_posture(p.get("threadSettings").unwrap_or(&p)) {
                        self.error="Codex permissions changed; stopped to preserve manual approval policy.".into();
                        self.stop();
                    }
                }
                "error" => {
                    self.error = p
                        .pointer("/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("Codex error")
                        .into();
                    if p.get("willRetry") == Some(&Value::Bool(false)) {
                        self.turn = None;
                        self.starting_turn = false;
                    }
                }
                _ => {}
            }
            return;
        }
        let Some(id) = value.get("id").and_then(Value::as_u64) else {
            return;
        };
        let Some((method, _)) = self.pending.remove(&id) else {
            return;
        };
        if let Some(error) = value.get("error") {
            self.error = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Codex request failed")
                .into();
            self.starting_turn = false;
            return;
        }
        let result = value.get("result").cloned().unwrap_or(Value::Null);
        match method.as_str() {
            "initialize" => {
                if let Some(rpc) = &self.rpc {
                    let _ = rpc.send(json!({"method":"initialized","params":{}}));
                }
                let mut params = json!({"cwd":self.cwd,"approvalPolicy":"on-request","approvalsReviewer":"user","sandbox":"workspace-write"});
                let method = if self.resume.trim().is_empty() {
                    params["dynamicTools"] = crate::agent_tools::definitions();
                    "thread/start"
                } else if valid_id(self.resume.trim()) {
                    params["threadId"] = json!(self.resume.trim());
                    "thread/resume"
                } else {
                    self.error = "Invalid thread ID.".into();
                    self.stop();
                    return;
                };
                if let Err(e) = self.request(method, params) {
                    self.error = e;
                }
            }
            "thread/start" | "thread/resume" => {
                if !safe_posture(&result) {
                    self.error =
                        "Codex did not confirm the requested sandbox and approval policy.".into();
                    self.stop();
                    return;
                }
                self.thread_id = result
                    .pointer("/thread/id")
                    .and_then(Value::as_str)
                    .filter(|id| valid_id(id))
                    .map(str::to_owned);
                self.ready = self.thread_id.is_some();
                self.status = if self.ready {
                    "Ready"
                } else {
                    "Missing thread ID"
                }
                .into();
                if let Some(id) = &self.thread_id {
                    self.resume = id.clone();
                }
            }
            "turn/start" => {
                self.turn = result
                    .pointer("/turn/id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                self.starting_turn = false;
            }
            _ => {}
        }
    }
    pub fn poll(&mut self) {
        for _ in 0..128 {
            let event = self.rpc.as_ref().and_then(|r| r.reader.try_recv().ok());
            match event {
                Some(Ok(v)) => self.frame(v),
                Some(Err(e)) => {
                    self.error = e;
                    self.stop();
                    break;
                }
                None => break,
            }
        }
        if self
            .pending
            .values()
            .any(|(_, t)| t.elapsed() > Duration::from_secs(30))
        {
            self.error =
                "Codex request timed out. The session has stopped; it can be resumed.".into();
            self.stop();
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui) {
        ui.heading("Codex · native chat");
        ui.small(self.cwd.display().to_string());
        if let Some(id) = &self.thread_id {
            ui.horizontal(|ui| {
                ui.small(format!("Thread: {id}"));
                if ui.small_button("Copy ID").clicked() {
                    ui.ctx().copy_text(id.clone());
                }
            });
        }
        ui.horizontal(|ui| {
            ui.label(&self.status);
            if self.rpc.is_none() {
                if ui.button("Connect").clicked() {
                    self.start();
                }
            } else if ui.button("Stop process").clicked() {
                self.stop();
            }
            if self.turn.is_some()
                && ui.button("Interrupt turn").clicked()
                && let Err(e) = self.request(
                    "turn/interrupt",
                    json!({"threadId":self.thread_id,"turnId":self.turn}),
                )
            {
                self.error = e;
            }
        });
        if self.rpc.is_none() {
            ui.horizontal(|ui| {
                ui.label("Resume thread ID (optional)");
                ui.text_edit_singleline(&mut self.resume);
            });
        }
        ui.small("Workspace-write sandbox · approval on request · network disabled by default. Uses your existing Codex sign-in.");
        if !self.error.is_empty() {
            ui.colored_label(egui::Color32::LIGHT_RED, &self.error);
        }
        if let Some(approval) = self.approvals.front().cloned() {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.strong("Approval requested — no action until you choose");
                ui.label(&approval.method);
                let item = approval
                    .params
                    .get("itemId")
                    .and_then(Value::as_str)
                    .and_then(|id| self.items.get(id));
                let details = json!({"request":approval.params,"item":item});
                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .show(ui, |ui| {
                        ui.monospace(serde_json::to_string_pretty(&details).unwrap_or_default());
                    });
                ui.horizontal(|ui| {
                    for (label, decision) in [("Deny", "decline"), ("Allow once", "accept")] {
                        if ui.button(label).clicked()
                            && let Some(rpc) = &self.rpc
                        {
                            match rpc.send(json!({"id":approval.id,"result":{"decision":decision}}))
                            {
                                Ok(()) => {
                                    self.approvals.pop_front();
                                }
                                Err(e) => self.error = e,
                            }
                        }
                    }
                });
            });
        }
        let available = (ui.available_height() - 130.0).max(80.0);
        egui::ScrollArea::vertical()
            .max_height(available)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for (role, text) in &self.transcript {
                    ui.strong(role);
                    ui.add(egui::Label::new(text).selectable(true));
                    ui.separator();
                }
            });
        ui.horizontal(|ui| {
            ui.label("Model (empty = configured)");
            ui.text_edit_singleline(&mut self.model);
            egui::ComboBox::from_id_salt("effort")
                .selected_text(&self.effort)
                .show_ui(ui, |ui| {
                    for effort in ["low", "medium", "high", "xhigh"] {
                        ui.selectable_value(&mut self.effort, effort.into(), effort);
                    }
                });
        });
        ui.add(
            egui::TextEdit::multiline(&mut self.composer)
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        );
        if ui
            .add_enabled(
                self.ready && !self.starting_turn,
                egui::Button::new(if self.turn.is_some() {
                    "Send follow-up"
                } else {
                    "Send"
                }),
            )
            .clicked()
        {
            self.send_message();
        }
    }
    pub fn set_resume(&mut self, id: String) {
        self.resume = id;
    }
    pub fn tool_result(&mut self, id: Value, result: Result<Value, String>) {
        let (success, text) = match result {
            Ok(value) => (true, value.to_string()),
            Err(error) => (false, error),
        };
        if let Some(rpc)=&self.rpc && let Err(e)=rpc.send(json!({"id":id,"result":{"success":success,"contentItems":[{"type":"inputText","text":text}]}})){self.error=e;}
    }
}

/// Real initialization test, with no model turn, no prompt and no inference charge.
pub fn smoke() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let mut chat = Chat::new(0, directory.path().to_owned());
    chat.start();
    let deadline = Instant::now() + Duration::from_secs(35);
    while Instant::now() < deadline {
        chat.poll();
        if chat.ready {
            chat.stop();
            return Ok(());
        }
        if !chat.error.is_empty() {
            return Err(chat.error.clone().into());
        }
        thread::sleep(Duration::from_millis(20));
    }
    Err("Codex initialization timed out.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsafe_or_missing_posture_is_rejected() {
        assert!(!safe_posture(&json!({})));
        assert!(!safe_posture(
            &json!({"approvalPolicy":"never","sandbox":"danger-full-access"})
        ));
        assert!(safe_posture(
            &json!({"approvalPolicy":"on-request","sandbox":{"type":"workspaceWrite"},"approvalsReviewer":"user"})
        ));
    }
    #[test]
    fn failed_turn_does_not_look_successful() {
        let mut chat = Chat::new(1, PathBuf::from("."));
        chat.thread_id = Some("thread-1".into());
        chat.turn = Some("turn-1".into());
        chat.frame(json!({"method":"turn/completed","params":{"threadId":"thread-1","turn":{"id":"turn-1","status":"failed","error":{"message":"test failure"}}}}));
        assert_eq!(chat.status, "failed");
        assert_eq!(chat.error, "test failure");
        assert!(chat.turn.is_none());
    }
    #[test]
    fn other_thread_cannot_modify_transcript() {
        let mut chat = Chat::new(1, PathBuf::from("."));
        chat.thread_id = Some("owner".into());
        chat.frame(json!({"method":"item/agentMessage/delta","params":{"threadId":"other","delta":"untrusted"}}));
        assert!(chat.transcript.is_empty());
    }
    #[test]
    fn approval_is_never_automatically_accepted() {
        let mut chat = Chat::new(1, PathBuf::from("."));
        chat.thread_id = Some("owner".into());
        chat.turn = Some("turn-1".into());
        chat.frame(json!({"id":42,"method":"item/commandExecution/requestApproval","params":{"threadId":"owner","turnId":"turn-1","command":"echo fixture"}}));
        assert_eq!(chat.approvals.len(), 1);
    }
    #[test]
    fn stale_turn_cannot_request_approval() {
        let mut chat = Chat::new(1, PathBuf::from("."));
        chat.thread_id = Some("owner".into());
        chat.turn = Some("current".into());
        chat.frame(json!({"id":1,"method":"item/commandExecution/requestApproval","params":{"threadId":"owner","turnId":"old","command":"echo stale"}}));
        assert!(chat.approvals.is_empty());
    }
}
