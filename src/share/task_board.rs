//! Authoritative task snapshots and bounded, revocable UI-thread mutations.
//! This module never opens a project file.
use super::*;
use std::collections::HashMap;
use std::sync::atomic::AtomicU8;

#[path = "task_text_cache.rs"]
mod text_cache;
use text_cache::CachedTaskText;
#[cfg(test)]
#[path = "task_text_cache_tests.rs"]
mod text_cache_tests;

const COLUMNS: [&str; 4] = ["todo", "doing", "review", "done"];
const WAIT: Duration = Duration::from_secs(2);
const MAX_QUEUED: usize = 32;
const PER_SECOND: usize = 4;

#[derive(Debug, Clone, Serialize)]
pub struct SharedTask {
    #[serde(skip)]
    pub project_id: u64,
    pub id: String,
    pub title: String,
    pub column: String,
    #[serde(rename = "guestBy", skip_serializing_if = "Option::is_none")]
    pub guest_by: Option<String>,
}

#[derive(Debug, Clone)]
pub enum GuestTaskOp {
    Add { title: String, column: usize },
    Move { id: u64, column: usize },
    Rename { id: u64, title: String },
    Delete { id: u64 },
}

#[derive(Debug, Clone)]
pub struct GuestTaskCommand {
    guest: String,
    project: u64,
    op: GuestTaskOp,
    deadline: Instant,
    // 0 pending, 1 executing, 2 finished, 3 cancelled before execution.
    state: Arc<AtomicU8>,
    reply: SyncSender<Result<Value>>,
}

#[derive(Default)]
pub(super) struct TaskBoard {
    ready: bool,
    text_cache: HashMap<String, Arc<CachedTaskText>>,
    limited: std::collections::HashSet<u64>,
    cards: Vec<SharedTask>,
    rates: HashMap<String, (Instant, usize)>,
}

pub(super) fn same_roots(left: &[SharedProject], right: &[SharedProject]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(a, b)| a.id == b.id && a.path == b.path)
}

impl TaskBoard {
    pub(super) fn invalidate(&mut self) {
        self.ready = false;
        self.text_cache.clear();
        self.limited.clear();
        self.cards.clear();
    }
    fn publish(&mut self, cards: Vec<SharedTask>, projects: &[SharedProject]) {
        self.ready = true;
        self.limited.clear();
        self.cards.clear();
        let previous_cache = std::mem::take(&mut self.text_cache);
        let mut bytes = 0;
        let mut ids = std::collections::HashSet::new();
        let mut per_project = HashMap::<u64, usize>::new();
        for mut card in cards.into_iter().rev() {
            let Some(project) = projects.iter().find(|p| p.id == card.project_id) else {
                continue;
            };
            if card.id.parse::<u64>().is_err()
                || !ids.insert(card.id.clone())
                || card.title.len() > 4096
                || !COLUMNS.contains(&card.column.as_str())
                || card
                    .guest_by
                    .as_ref()
                    .is_some_and(|s| s.chars().count() > 40)
            {
                self.limited.insert(card.project_id);
                continue;
            }
            let count = per_project.entry(card.project_id).or_default();
            if *count >= 500 {
                self.limited.insert(card.project_id);
                continue;
            }
            *count += 1;
            if self.cards.len() >= 10_000 {
                self.limited.insert(card.project_id);
                continue;
            }
            let text = previous_cache
                .get(&card.id)
                .filter(|cached| cached.matches(&card, &project.path))
                .cloned()
                .unwrap_or_else(|| Arc::new(CachedTaskText::new(&card, &project.path)));
            card.title.clone_from(&text.title);
            card.guest_by.clone_from(&text.guest_by);
            // JSON may encode each control byte as six ASCII bytes. This upper
            // bound avoids repeated serialization under the authority mutex.
            let card_bytes = 128
                + card.id.len()
                + 6 * (card.title.len() + card.guest_by.as_ref().map_or(0, String::len));
            if bytes + card_bytes > 8 * 1024 * 1024 || self.cards.len() >= 10_000 {
                self.limited.insert(card.project_id);
                continue;
            }
            bytes += card_bytes;
            self.text_cache.insert(card.id.clone(), text);
            self.cards.push(card);
        }
        self.cards.reverse();
    }

    fn snapshot(&self, project: u64) -> Result<Value> {
        if !self.ready {
            return Err("The host task board is not ready yet.".into());
        }
        Ok(
            json!({"tasks": self.cards.iter().filter(|c| c.project_id == project).collect::<Vec<_>>(), "limited": self.limited.contains(&project)}),
        )
    }
}

fn authorize(
    guard: &HostInner,
    guest: &str,
    project: Option<u64>,
    mutate: bool,
) -> Result<(u64, String)> {
    if guard.stopped.load(Ordering::SeqCst) {
        return Err("Sharing has stopped.".into());
    }
    let idx = guard
        .guests
        .iter()
        .position(|g| g.guest.id == guest)
        .ok_or("Guest session has ended.")?;
    let slot = &guard.guests[idx];
    if !matches!(slot.decision, GuestDecision::Approved)
        || slot.guest.state != GuestConnectionState::Joined
    {
        return Err("Guest session is not active.".into());
    }
    if mutate && !slot.guest.rights.tasks {
        return Err("Tasks permission is required to manage tasks.".into());
    }
    let project = guard.find_shared_project_for_guest(idx, project)?.id;
    Ok((
        project,
        slot.guest
            .name
            .chars()
            .filter(|c| !c.is_control() && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}' | '\u{061c}'))
            .take(40)
            .collect(),
    ))
}

fn parse(req: &Value) -> Result<GuestTaskOp> {
    let action = req["action"].as_str().ok_or("Missing task action.")?;
    let allowed: &[&str] = match action {
        "add" => &["op", "projectId", "action", "title", "column"],
        "move" => &["op", "projectId", "action", "id", "column"],
        "edit" | "rename" => &["op", "projectId", "action", "id", "title"],
        "delete" => &["op", "projectId", "action", "id"],
        _ => return Err("Unsupported task mutation.".into()),
    };
    if req
        .as_object()
        .is_none_or(|o| o.keys().any(|k| !allowed.contains(&k.as_str())))
    {
        return Err("Unsupported task fields.".into());
    }
    let title = || -> Result<String> {
        guest_text::clean_task_input(req["title"].as_str().ok_or("Missing task title.")?)
            .ok_or_else(|| "Task titles must contain 1 to 300 plain-text characters.".into())
    };
    let id = || -> Result<u64> {
        req["id"]
            .as_u64()
            .or_else(|| {
                req["id"]
                    .as_str()
                    .and_then(|s| s.parse::<u64>().ok().filter(|id| id.to_string() == s))
            })
            .ok_or_else(|| "Invalid task identifier.".into())
    };
    let column = || -> Result<usize> {
        let col = match req.get("column") {
            None if action == "add" => "todo",
            Some(Value::String(s)) => s,
            _ => return Err("Invalid task column.".into()),
        };
        COLUMNS
            .iter()
            .position(|c| *c == col)
            .ok_or_else(|| "Invalid task column.".into())
    };
    match action {
        "add" => Ok(GuestTaskOp::Add {
            title: title()?,
            column: column()?,
        }),
        "move" => Ok(GuestTaskOp::Move {
            id: id()?,
            column: column()?,
        }),
        "edit" | "rename" => Ok(GuestTaskOp::Rename {
            id: id()?,
            title: title()?,
        }),
        "delete" => Ok(GuestTaskOp::Delete { id: id()? }),
        _ => unreachable!(),
    }
}

pub(super) fn dispatch(inner: &Arc<Mutex<HostInner>>, guest: &str, req: &Value) -> Value {
    let result = (|| -> Result<Value> {
        let mutate = req["op"] == "tasks.mutate";
        let project = match req.get("projectId") {
            None | Some(Value::Null) => None,
            Some(v) => Some(v.as_u64().ok_or("Invalid project identifier.")?),
        };
        let mut guard = inner.lock().unwrap_or_else(|e| e.into_inner());
        let (project, _) = authorize(&guard, guest, project, mutate)?;
        if !mutate {
            return guard.task_board.snapshot(project);
        }
        let op = parse(req)?;
        if guard.commands.len() >= MAX_QUEUED {
            return Err("The host command queue is full; try again later.".into());
        }
        let active: Vec<String> = guard.guests.iter().map(|g| g.guest.id.clone()).collect();
        guard.task_board.rates.retain(|id, _| active.contains(id));
        let rate = guard
            .task_board
            .rates
            .entry(guest.into())
            .or_insert((Instant::now(), 0));
        if rate.0.elapsed() >= Duration::from_secs(1) {
            *rate = (Instant::now(), 0);
        }
        if rate.1 >= PER_SECOND {
            return Err("Too many task changes; try again later.".into());
        }
        rate.1 += 1;
        let (reply, receiver) = mpsc::sync_channel(1);
        let state = Arc::new(AtomicU8::new(0));
        guard
            .commands
            .push_back(HostCommand::TaskMutation(GuestTaskCommand {
                guest: guest.into(),
                project,
                op,
                deadline: Instant::now() + WAIT,
                state: state.clone(),
                reply,
            }));
        drop(guard); // The UI must acquire this mutex to revalidate and complete.
        match receiver.recv_timeout(WAIT) {
            Ok(result) => result,
            Err(_) => {
                if state
                    .compare_exchange(0, 3, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
                {
                    Err("Task request expired before execution; no change was made.".into())
                } else {
                    receiver
                        .try_recv()
                        .or_else(|_| receiver.recv_timeout(Duration::from_millis(250)))
                        .unwrap_or_else(|_| Err(TASK_RESULT_NOT_AVAILABLE.into()))
                }
            }
        }
    })();
    match result {
        Ok(v) => json!({"result": v}),
        Err(e) => json!({"error": e}),
    }
}

impl Host {
    pub fn task_scope(&self) -> (Vec<u8>, Vec<u64>) {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        // This opaque local cache identity is never serialized to a guest.
        let mut identity = (guard.cert_der.len() as u64).to_le_bytes().to_vec();
        identity.extend_from_slice(&guard.cert_der);
        for project in &guard.shared_projects {
            identity.extend_from_slice(&project.id.to_le_bytes());
            let path = project.path.as_os_str().as_encoded_bytes();
            identity.extend_from_slice(&(path.len() as u64).to_le_bytes());
            identity.extend_from_slice(path);
        }
        (
            identity,
            guard.shared_projects.iter().map(|p| p.id).collect(),
        )
    }
    pub fn publish_tasks(&self, cards: Vec<SharedTask>) {
        let (projects, text_cache) = {
            let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            (
                guard.shared_projects.clone(),
                guard.task_board.text_cache.clone(),
            )
        };
        let mut prepared = TaskBoard {
            text_cache,
            ..TaskBoard::default()
        };
        prepared.publish(cards, &projects);
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if !same_roots(&projects, &guard.shared_projects) {
            return; // The next UI tick has a different task_scope and republishes.
        }
        guard.task_board.ready = prepared.ready;
        guard.task_board.limited = prepared.limited;
        guard.task_board.cards = prepared.cards;
        guard.task_board.text_cache = prepared.text_cache;
    }

    /// The callback commits to host storage before returning success. Revocation
    /// and the transaction are serialized by the same authority mutex.
    pub fn complete_task(
        &self,
        command: GuestTaskCommand,
        commit: impl FnOnce(u64, &str, &GuestTaskOp) -> Result<(bool, Vec<SharedTask>)>,
    ) {
        let result = (|| {
            let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if Instant::now() >= command.deadline
                || command
                    .state
                    .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
                    .is_err()
            {
                return Err("Task request expired; no change was made.".into());
            }
            let (project, name) = authorize(&guard, &command.guest, Some(command.project), true)?;
            let (changed, cards) = commit(project, &name, &command.op)?;
            let projects = guard.shared_projects.clone();
            guard.task_board.publish(cards, &projects);
            if changed {
                guard.record_action(
                    &name,
                    project,
                    format!("{name} updated the task board"),
                    format!("{name} a modifié le tableau des tâches"),
                    false,
                );
            }
            guard.task_board.snapshot(project)
        })();
        command.state.store(2, Ordering::SeqCst);
        let _ = command.reply.try_send(result);
    }
}

impl GuestTaskCommand {
    pub fn reject(self) {
        self.state.store(3, Ordering::SeqCst);
        let _ = self
            .reply
            .try_send(Err("The host sharing session has ended.".into()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Host) {
        let dir = tempfile::Builder::new()
            .prefix("shared-board-")
            .tempdir_in(std::env::current_dir().unwrap())
            .unwrap();
        let host = Host::start(
            dir.path(),
            "127.0.0.1:0".parse().unwrap(),
            "127.0.0.1".parse().unwrap(),
        )
        .unwrap();
        host.inner.lock().unwrap().guests.push(GuestSlot {
            decision: GuestDecision::Approved,
            guest: ConnectedGuest {
                id: "guest".into(),
                name: "Guest".into(),
                state: GuestConnectionState::Joined,
                code: String::new(),
                invite_id: None,
                invite_label: String::new(),
                projects: vec![1],
                rights: ShareRights::collaborate(),
                trusted: false,
                trust_id: None,
                identity_hex: None,
                since_ms: 0,
                watching_pane: None,
                typing_pane: None,
            },
        });
        (dir, host)
    }
    fn card(project_id: u64, id: u64) -> SharedTask {
        SharedTask {
            project_id,
            id: id.to_string(),
            title: "Actual board".into(),
            column: "todo".into(),
            guest_by: None,
        }
    }
    fn queued(host: &Host) -> (GuestTaskCommand, Receiver<Result<Value>>) {
        let (reply, rx) = mpsc::sync_channel(1);
        let cmd = GuestTaskCommand {
            guest: "guest".into(),
            project: 1,
            op: GuestTaskOp::Add {
                title: "Requested".into(),
                column: 0,
            },
            deadline: Instant::now() + WAIT,
            state: Arc::new(AtomicU8::new(0)),
            reply,
        };
        assert!(authorize(&host.inner.lock().unwrap(), "guest", Some(1), true).is_ok());
        (cmd, rx)
    }
    #[test]
    fn shared_board_never_reads_or_rewrites_legacy_project_file() {
        let (dir, host) = fixture();
        std::fs::create_dir(dir.path().join(".tessel")).unwrap();
        let legacy = dir.path().join(".tessel/live-share-tasks.json");
        let secret = br#"{"private":"synthetic-secret"}"#;
        std::fs::write(&legacy, secret).unwrap();
        host.publish_tasks(vec![card(1, 5), card(2, 6)]);
        let response = dispatch(
            &host.inner,
            "guest",
            &json!({"op":"tasks.list", "projectId":1}),
        );
        assert_eq!(response["result"]["tasks"].as_array().unwrap().len(), 1);
        assert!(!response.to_string().contains("synthetic-secret"));
        let (cmd, rx) = queued(&host);
        host.complete_task(cmd, |_, _, _| Ok((true, vec![card(1, 7)])));
        assert!(rx.recv().unwrap().is_ok());
        assert_eq!(std::fs::read(legacy).unwrap(), secret);
        assert!(
            dispatch(
                &host.inner,
                "guest",
                &json!({"op":"tasks.list","projectId":2})
            )["error"]
                .is_string()
        );
    }
    #[test]
    fn shared_board_revocation_expiry_and_cancel_prevent_commit() {
        let (_dir, host) = fixture();
        let (cmd, rx) = queued(&host);
        host.inner.lock().unwrap().guests[0].guest.rights.tasks = false;
        host.complete_task(cmd, |_, _, _| panic!("revoked callback"));
        assert!(rx.recv().unwrap().is_err());
        host.inner.lock().unwrap().guests[0].guest.rights.tasks = true;
        let (mut cmd, rx) = queued(&host);
        cmd.deadline = Instant::now();
        host.complete_task(cmd, |_, _, _| panic!("expired callback"));
        assert!(rx.recv().unwrap().is_err());
        let (cmd, rx) = queued(&host);
        cmd.state.store(3, Ordering::SeqCst);
        host.complete_task(cmd, |_, _, _| panic!("cancelled callback"));
        assert!(rx.recv().unwrap().is_err());
        let (cmd, rx) = queued(&host);
        host.sync_workspace_state(vec![], vec![], vec![], None, None);
        host.complete_task(cmd, |_, _, _| panic!("removed project callback"));
        assert!(rx.recv().unwrap().is_err());
        assert!(host.snapshot().shared_projects.is_empty());
    }
    #[test]
    fn shared_board_rpc_wait_releases_mutex_and_returns_committed_snapshot() {
        let (_dir, host) = fixture();
        let inner = host.inner.clone();
        let worker = thread::spawn(move || {
            dispatch(
                &inner,
                "guest",
                &json!({"op":"tasks.mutate", "projectId":1, "action":"add", "title":"Request"}),
            )
        });
        let start = Instant::now();
        let cmd = loop {
            if let Some(HostCommand::TaskMutation(cmd)) = host.drain_commands().pop() {
                break cmd;
            }
            assert!(start.elapsed() < WAIT);
            thread::sleep(Duration::from_millis(5));
        };
        host.complete_task(cmd, |project, actor, op| {
            assert_eq!((project, actor), (1, "Guest"));
            assert!(matches!(op, GuestTaskOp::Add { .. }));
            Ok((true, vec![card(1, 123)]))
        });
        assert_eq!(worker.join().unwrap()["result"]["tasks"][0]["id"], "123");
        assert_eq!(host.snapshot().recent_actions.len(), 1);
    }
    #[test]
    fn shared_board_queue_quota_validation_and_failure_are_explicit() {
        let (_dir, host) = fixture();
        let req = json!({"op":"tasks.mutate", "action":"add", "title":"Request"});
        for _ in 0..MAX_QUEUED {
            host.inner
                .lock()
                .unwrap()
                .commands
                .push_back(HostCommand::ClosePane { pane_id: 99 });
        }
        assert!(
            dispatch(&host.inner, "guest", &req)["error"]
                .as_str()
                .unwrap()
                .contains("queue")
        );
        host.drain_commands();
        host.inner
            .lock()
            .unwrap()
            .task_board
            .rates
            .insert("guest".into(), (Instant::now(), PER_SECOND));
        assert!(
            dispatch(&host.inner, "guest", &req)["error"]
                .as_str()
                .unwrap()
                .contains("Too many")
        );
        assert!(parse(&json!({"action":"add","title":"x", "guestBy":"Host"})).is_err());
        assert!(parse(&json!({"action":"move","id":"01","column":"todo"})).is_err());
        assert!(parse(&json!({"action":"add","title":"x", "column":"arbitrary"})).is_err());
        assert!(parse(&json!({"action":"add","title":"x".repeat(301)})).is_err());
        let (cmd, rx) = queued(&host);
        host.complete_task(cmd, |_, _, _| Err("Synthetic storage failure".into()));
        assert!(rx.recv().unwrap().is_err());
        assert!(host.snapshot().recent_actions.is_empty());
    }
    #[test]
    fn shared_board_real_tls_guest_add_reads_authoritative_reply() {
        let (_dir, host) = fixture();
        host.publish_tasks(vec![card(1, 10)]);
        let guest =
            GuestSession::connect(host.invitation.clone(), "TLS Guest".into(), "b2".repeat(32));
        let started = Instant::now();
        let key = loop {
            if let Some(LiveShareAsk::Join { key, .. }) = host.snapshot().asks.first() {
                break key.clone();
            }
            assert!(started.elapsed() < Duration::from_secs(5));
            thread::sleep(Duration::from_millis(10));
        };
        host.answer_ask(AskAnswer::Join {
            key,
            allow: true,
            projects: vec![1],
            rights: ShareRights::collaborate(),
            trust: false,
        });
        while !matches!(guest.snapshot().phase, GuestPhase::Joined { .. }) {
            assert!(started.elapsed() < Duration::from_secs(5));
            thread::sleep(Duration::from_millis(10));
        }
        let initial = guest
            .call_sync(
                json!({"op":"tasks.list", "projectId":1}),
                Duration::from_secs(3),
            )
            .unwrap();
        assert_eq!(initial["tasks"][0]["id"], "10");
        let worker = thread::spawn(move || {
            guest.call_sync(
                json!({"op":"tasks.mutate", "projectId":1, "action":"add", "title":"TLS request"}),
                Duration::from_secs(3),
            )
        });
        let started = Instant::now();
        let command = loop {
            if let Some(HostCommand::TaskMutation(command)) = host.drain_commands().pop() {
                break command;
            }
            assert!(started.elapsed() < WAIT);
            thread::sleep(Duration::from_millis(5));
        };
        host.complete_task(command, |project, actor, _| {
            assert_eq!((project, actor), (1, "TLS Guest"));
            Ok((true, vec![card(1, 10), card(1, 11)]))
        });
        assert_eq!(
            worker.join().unwrap().unwrap()["tasks"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
    #[test]
    fn shared_board_redacts_list_and_mutation_replies_without_editing_host_titles() {
        let (dir, host) = fixture();
        let mut raw = card(1, 20);
        raw.title = format!(
            "token=synthetic_password read C:\\Users\\fixture\\private.txt and {}",
            dir.path().join("src/a.rs").display()
        );
        raw.guest_by = Some("password=synthetic_pw".into());
        host.publish_tasks(vec![raw.clone()]);
        let result = dispatch(
            &host.inner,
            "guest",
            &json!({"op":"tasks.list","projectId":1}),
        );
        assert_eq!(
            result["result"]["tasks"][0]["title"],
            "token=•••• read … and src/a.rs"
        );
        assert_eq!(result["result"]["tasks"][0]["guestBy"], "password=••••");
        let (command, reply) = queued(&host);
        host.complete_task(command, |_, _, _| Ok((true, vec![raw.clone()])));
        let result = reply.recv().unwrap().unwrap();
        assert_eq!(
            result["tasks"][0]["title"],
            "token=•••• read … and src/a.rs"
        );
        assert!(!result.to_string().contains("synthetic_password"));
        assert!(raw.title.contains("synthetic_password"));
    }

    #[test]
    fn shared_board_root_rebinding_invalidates_cached_paths_and_scope() {
        let (dir, host) = fixture();
        let scope = host.task_scope();
        host.publish_tasks(vec![card(1, 20)]);
        let replacement = tempfile::Builder::new()
            .prefix("shared-rebind-")
            .tempdir_in(std::env::current_dir().unwrap())
            .unwrap();
        host.sync_workspace_state(
            vec![SharedProject {
                id: 1,
                name: "Shared".into(),
                path: replacement.path().into(),
            }],
            Vec::new(),
            Vec::new(),
            Some(1),
            None,
        );
        assert_ne!(scope, host.task_scope());
        assert!(
            dispatch(
                &host.inner,
                "guest",
                &json!({"op":"tasks.list","projectId":1})
            )["error"]
                .is_string()
        );
        let mut raw = card(1, 21);
        raw.title = format!(
            "{} and {}",
            dir.path().join("private.txt").display(),
            replacement.path().join("src/new.rs").display()
        );
        host.publish_tasks(vec![raw]);
        let result = dispatch(
            &host.inner,
            "guest",
            &json!({"op":"tasks.list","projectId":1}),
        );
        assert_eq!(result["result"]["tasks"][0]["title"], "… and src/new.rs");
    }

    #[test]
    fn shared_board_partial_notice_is_project_scoped() {
        let (dir, _host) = fixture();
        let projects = vec![
            SharedProject {
                id: 1,
                name: "A".into(),
                path: dir.path().into(),
            },
            SharedProject {
                id: 2,
                name: "B".into(),
                path: dir.path().into(),
            },
        ];
        let mut board = TaskBoard::default();
        let mut cards = vec![card(1, 1)];
        cards.extend((2..503).map(|id| card(2, id)));
        board.publish(cards, &projects);
        assert_eq!(board.snapshot(1).unwrap()["limited"], false);
        assert_eq!(board.snapshot(2).unwrap()["limited"], true);
    }
    #[test]
    fn shared_board_guest_input_rejects_markup_only_and_cleans_before_length_check() {
        let title = "\u{202e}\x1b[31mhello\x1b[0m\nworld\u{feff}\u{2028}";
        let op = parse(&json!({"action":"add", "title":title})).unwrap();
        assert!(matches!(op, GuestTaskOp::Add { title, .. } if title == "hello world"));
        assert!(parse(&json!({"action":"add", "title":"\x1b[31m\u{202e}"})).is_err());
        let title = format!("{}{}", "\u{202e}".repeat(100), "a".repeat(300));
        assert!(parse(&json!({"action":"add", "title":title})).is_ok());
    }
    #[test]
    fn shared_board_snapshots_bound_count_and_encoded_bytes() {
        let (_dir, host) = fixture();
        host.publish_tasks((0..10_001).map(|n| card(1, n)).collect());
        let guard = host.inner.lock().unwrap();
        assert_eq!(guard.task_board.cards.len(), 500);
        assert_eq!(guard.task_board.cards.first().unwrap().id, "9501");
        assert_eq!(guard.task_board.cards.last().unwrap().id, "10000");
        assert!(guard.task_board.limited.contains(&1));
        drop(guard);
        host.publish_tasks(
            (0..10_000)
                .map(|n| {
                    let mut c = card(1, n);
                    c.title = "\u{0001}".repeat(4096);
                    c
                })
                .collect(),
        );
        let guard = host.inner.lock().unwrap();
        assert!(guard.task_board.cards.len() < 1000);
        assert!(guard.task_board.limited.contains(&1));
        assert!(
            serde_json::to_vec(&guard.task_board.snapshot(1).unwrap())
                .unwrap()
                .len()
                < MAX_RESPONSE
        );
    }
}
