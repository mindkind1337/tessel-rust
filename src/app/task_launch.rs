//! Owner-only task launch controller. Pending operations are deliberately not persisted.
use super::*;
use crate::{
    initial_turn::{Native, Phase},
    task_preparation::{Details, Ticket},
    workspace::Task,
};
use std::time::Instant;

#[derive(Clone, Copy)]
pub(crate) enum Provider {
    Codex,
    Claude,
}
impl Provider {
    fn launch(self) -> Launch {
        match self {
            Self::Codex => Launch::CodexChat,
            Self::Claude => Launch::ClaudeChat,
        }
    }
    fn matches(self, launch: &Launch) -> bool {
        matches!(
            (self, launch),
            (Self::Codex, Launch::CodexChat) | (Self::Claude, Launch::ClaudeChat)
        )
    }
}

// This type is passed by an explicit owner action, never reconstructed on restore.
pub(crate) struct Spec {
    pub project: u64,
    pub source: PathBuf,
    pub title: String,
    pub brief: String,
    pub provider: Provider,
    pub reviewer: Option<crate::native_review::Target>,
    pub isolated: bool,
    pub base_branch: Option<String>,
    pub copy_env: bool,
    pub run_setup: bool,
}

pub(super) struct Pending {
    ticket: Ticket,
    expected: Task,
    provider: Provider,
    base_branch: Option<String>,
    pane: Option<u64>,
    prompt: String,
    phase: Phase,
    started: Instant,
    waiting_copy: bool,
    reviewer: Option<crate::native_review::Target>,
}

pub(super) struct Running {
    pending: Pending,
    token: u64,
}

impl Tessel {
    fn save_launch_card(&mut self, expected: &Task, replacement: Task) -> Result<(), String> {
        let mut draft = self.state.clone();
        let card = draft
            .tasks
            .iter_mut()
            .find(|t| t.id == expected.id && t.project == expected.project)
            .ok_or("The task was removed. No instructions were sent.")?;
        if card != expected {
            return Err("The task changed. No instructions were sent.".into());
        }
        *card = replacement;
        let bytes = draft.prepare_task_change_from(self.state.task_cards_size()?)?;
        self.store
            .as_ref()
            .ok_or("Workspace storage is unavailable.")?
            .save_task_bytes(&bytes)?;
        self.state = draft;
        Ok(())
    }

    fn launch_context_current(&self, pending: &Pending) -> bool {
        pending.ticket.project_is_current(&self.state)
            && self.state.tasks.iter().any(|t| t == &pending.expected)
    }

    // Explicit owner action only; never called from imported task metadata.
    pub(crate) fn start_owner_native_task(&mut self, spec: Spec) -> Result<u64, String> {
        if self.owner_task_launches.len() >= 32 || self.sessions.len() >= 100 {
            return Err("Too many task preparations or open sessions.".into());
        }
        if spec
            .reviewer
            .as_ref()
            .is_some_and(|t| t.project != spec.project || !self.reviewer_current(t))
        {
            return Err(
                "The selected reviewer changed. Select a connected native reviewer again.".into(),
            );
        }
        let brief = spec.brief.trim().to_owned();
        // Validate the complete direct prompt before creating any card or pane.
        crate::task_prompt::render(&crate::task_prompt::Context {
            title: spec.title.trim(),
            brief: &brief,
            cwd: &spec.source.to_string_lossy(),
            worktree: None,
            guest_by: None,
            copied_from: None,
            team: None,
        })?;
        let ticket = crate::task_preparation::create(
            &mut self.state,
            self.store.as_ref(),
            spec.project,
            &spec.title,
            Details {
                source_cwd: spec.source.clone(),
                brief,
                reviewer_id: spec.reviewer.as_ref().map(|t| t.pane),
                worktree: None,
            },
        )?;
        let card = self
            .state
            .tasks
            .iter()
            .find(|t| t.id == ticket.task_id())
            .unwrap()
            .clone();
        let mut doing = card.clone();
        doing
            .timeline
            .move_to(doing.column, 1, crate::task_timeline::now_ms());
        doing.column = 1;
        self.save_launch_card(&card, doing.clone())?;
        let mut pending = Pending {
            ticket,
            expected: doing,
            provider: spec.provider,
            base_branch: spec.base_branch.clone(),
            pane: None,
            prompt: String::new(),
            phase: Phase::Pending,
            started: Instant::now(),
            waiting_copy: spec.isolated,
            reviewer: spec.reviewer,
        };
        let id = pending.ticket.task_id();
        if spec.isolated {
            self.worktree_state
                .request_create(crate::worktree_state::CreateRequest {
                    project_id: spec.project,
                    cwd: spec.source,
                    label: spec.title,
                    base_branch: spec.base_branch,
                    copy_env: spec.copy_env,
                    run_setup: spec.run_setup,
                    general_prefs: self.state.general_prefs.clone(),
                    git_prefs: self.state.git_prefs.clone(),
                    target: crate::worktree_state::WorktreeLaunchTarget::Task(id),
                });
        } else if let Err(error) = self.bind_owner_task(&mut pending) {
            self.fail_owner_task(&pending, error.clone());
            return Err(error);
        }
        self.owner_task_launches.push(pending);
        Ok(id)
    }

    fn bind_owner_task(&mut self, pending: &mut Pending) -> Result<(), String> {
        if !self.launch_context_current(pending) {
            return Err(
                "The task or project changed. No agent was started; any prepared copy was kept."
                    .into(),
            );
        }
        let details = pending
            .expected
            .preparation
            .as_ref()
            .ok_or("Task preparation is missing.")?;
        let source = details.source_cwd.to_string_lossy();
        let worktree_path = details.worktree.as_ref().map(|w| w.path.to_string_lossy());
        pending.prompt = crate::task_prompt::render(&crate::task_prompt::Context {
            title: &pending.expected.title,
            brief: &details.brief,
            cwd: &source,
            worktree: details
                .worktree
                .as_ref()
                .zip(worktree_path.as_ref())
                .map(|(w, path)| crate::task_prompt::Worktree {
                    path,
                    branch: &w.branch,
                    base_branch: w.base_branch.as_deref(),
                }),
            guest_by: None,
            copied_from: None,
            team: None,
        })?;
        let worktree = details.worktree.as_ref().map(|w| workspace::SavedWorktree {
            path: w.path.clone(),
            branch: w.branch.clone(),
        });
        let project = pending.ticket.project_id();
        let split = self
            .sessions
            .iter()
            .find(|s| s.project == Some(project) && Some(s.id) == self.active_session)
            .map(|s| (s.id, crate::pane_grid::SplitDir::Row));
        let keep_selection = self.state.active_project != Some(project);
        let previous_active = self.active_session;
        let previous_view = std::mem::replace(&mut self.view, View::Work);
        // Nothing asynchronous occurs between the identity check and pane allocation.
        let pane = self.new_session_with_history(
            Some(project),
            pending.provider.launch(),
            split,
            false,
            worktree,
            None,
        );
        if keep_selection || pane.is_none() {
            self.active_session = previous_active;
            self.view = previous_view;
        }
        let pane = pane.ok_or("The task's agent pane could not be created.")?;
        pending.pane = Some(pane);
        let mut assigned = pending.expected.clone();
        assigned
            .timeline
            .assign(1, Some(pane), crate::task_timeline::now_ms());
        self.save_launch_card(&pending.expected, assigned.clone())?;
        pending.expected = assigned;
        pending.started = Instant::now();
        pending.waiting_copy = false;
        Ok(())
    }

    pub(super) fn finish_owner_task_copy(
        &mut self,
        id: u64,
        result: Result<crate::worktree::CreatedWorktree, String>,
    ) {
        let Some(index) = self
            .owner_task_launches
            .iter()
            .position(|p| p.ticket.task_id() == id && p.waiting_copy)
        else {
            return;
        };
        let mut pending = self.owner_task_launches.remove(index);
        let kept_path = result.as_ref().ok().map(|copy| copy.path.clone());
        let setup_warning = result
            .as_ref()
            .ok()
            .and_then(|copy| copy.setup_warning.clone());
        let result = result.and_then(|copy| {
            let mut receipt = pending.expected.clone();
            receipt
                .preparation
                .as_mut()
                .ok_or("Task preparation is missing.")?
                .worktree = Some(crate::task_preparation::Worktree {
                path: copy.path,
                root: copy.root,
                branch: copy.branch,
                base_branch: pending.base_branch.clone(),
            });
            self.save_launch_card(&pending.expected, receipt.clone())?;
            pending.expected = receipt;
            self.bind_owner_task(&mut pending)
        });
        match result {
            Ok(()) => {
                self.owner_task_launches.push(pending);
                if let Some(warning) = setup_warning {
                    self.error = warning;
                }
            }
            Err(error) => self.fail_owner_task(
                &pending,
                match kept_path {
                    Some(path) => format!("{error} Prepared copy kept at {}.", path.display()),
                    None => error,
                },
            ),
        }
    }

    fn fail_owner_task(&mut self, pending: &Pending, error: String) {
        // Do not overwrite a user's intervening edit or a removed/reassigned card.
        let mut todo = pending.expected.clone();
        todo.timeline
            .transition(todo.column, 0, None, crate::task_timeline::now_ms());
        todo.column = 0;
        let saved = self.save_launch_card(&pending.expected, todo);
        // A user may have started their own turn in this pane. Do not interrupt it.
        self.error = match saved {
            Ok(()) => error,
            Err(save) => format!("{error} {save}"),
        };
    }

    pub(super) fn cancel_owner_task_pane(&mut self, pane: u64) {
        self.cancel_owner_reviews_for_pane(pane);
        self.owner_task_turns
            .retain(|r| r.pending.pane != Some(pane));
        while let Some(index) = self
            .owner_task_launches
            .iter()
            .position(|p| p.pane == Some(pane))
        {
            let pending = self.owner_task_launches.remove(index);
            self.fail_owner_task(&pending, "Task launch cancelled by the owner.".into());
        }
    }

    pub(super) fn poll_owner_task_completions(&mut self) -> bool {
        use crate::task_completion::Outcome;
        let mut changed = false;
        for running in std::mem::take(&mut self.owner_task_turns) {
            let p = &running.pending;
            let pane = p.pane.unwrap_or(0);
            let details = p.expected.preparation.as_ref().unwrap();
            let cwd = details
                .worktree
                .as_ref()
                .map(|w| &w.path)
                .unwrap_or(&details.source_cwd);
            if !self.launch_context_current(p)
                || !self.sessions.iter().any(|s| {
                    s.id == pane
                        && s.project == Some(p.ticket.project_id())
                        && p.provider.matches(&s.launch)
                        && same_worker_directory(&s.cwd, cwd)
                })
            {
                continue;
            }
            let outcome = match p.provider {
                Provider::Codex => self
                    .chats
                    .iter()
                    .find(|c| {
                        c.pane == pane
                            && c.project == p.ticket.project_id()
                            && same_worker_directory(&c.cwd, cwd)
                    })
                    .map(|c| c.task_completion.outcome(running.token)),
                Provider::Claude => self
                    .claude_chats
                    .iter()
                    .find(|c| {
                        c.pane == pane
                            && c.project == p.ticket.project_id()
                            && same_worker_directory(&c.cwd, cwd)
                    })
                    .map(|c| c.task_completion.outcome(running.token)),
            }
            .unwrap_or(Outcome::Cancelled);
            match outcome {
                Outcome::Pending => self.owner_task_turns.push(running),
                Outcome::Cancelled => {}
                Outcome::Complete => {
                    let mut review = p.expected.clone();
                    review
                        .timeline
                        .move_to(review.column, 2, crate::task_timeline::now_ms());
                    review.column = 2;
                    match self.save_launch_card(&p.expected, review.clone()) {
                        Err(error) => {
                            self.error = format!(
                                "Task completion could not be saved. Move the card to Review manually. {error}"
                            )
                        }
                        Ok(()) => {
                            if let Some(target) = p.reviewer.clone() {
                                let who = self
                                    .sessions
                                    .iter()
                                    .find(|s| s.id == pane)
                                    .map(|s| s.pane_name.as_deref().unwrap_or(&s.title))
                                    .unwrap_or("another agent")
                                    .to_owned();
                                if let Err(error) =
                                    self.queue_owner_review(p.ticket.clone(), review, target, &who)
                                {
                                    self.error = error;
                                }
                            }
                        }
                    }
                    // A failed transaction is reported once, never repeatedly retried.
                    changed = true;
                }
            }
        }
        changed
    }

    pub(super) fn poll_owner_task_launches(&mut self) -> bool {
        if self.owner_task_launches.is_empty() {
            return false;
        }
        let mut changed = false;
        for mut pending in std::mem::take(&mut self.owner_task_launches) {
            if pending.waiting_copy {
                self.owner_task_launches.push(pending);
                continue;
            }
            let pane = pending.pane.unwrap_or(0);
            let details = pending.expected.preparation.as_ref().unwrap();
            let cwd = details
                .worktree
                .as_ref()
                .map(|w| &w.path)
                .unwrap_or(&details.source_cwd);
            let valid = self.launch_context_current(&pending)
                && self.sessions.iter().any(|s| {
                    s.id == pane
                        && s.project == Some(pending.ticket.project_id())
                        && pending.provider.matches(&s.launch)
                        && same_worker_directory(&s.cwd, cwd)
                });
            let result = if !valid {
                Err("Task launch context changed before the first turn.".into())
            } else if pending.started.elapsed() >= Duration::from_secs(30) {
                Err("Task first turn timed out; it was not retried.".into())
            } else {
                self.ensure_native_chat_instances();
                let attempted = pending.phase == Phase::Connecting;
                match pending.provider {
                    Provider::Codex => self
                        .chats
                        .iter_mut()
                        .find(|c| {
                            c.pane == pane
                                && c.project == pending.ticket.project_id()
                                && same_worker_directory(&c.cwd, cwd)
                        })
                        .ok_or_else(|| "Task chat is unavailable.".to_owned())
                        .and_then(|c| c.begin_task_turn(&pending.prompt, attempted)),
                    Provider::Claude => self
                        .claude_chats
                        .iter_mut()
                        .find(|c| {
                            c.pane == pane
                                && c.project == pending.ticket.project_id()
                                && same_worker_directory(&c.cwd, cwd)
                        })
                        .ok_or_else(|| "Task chat is unavailable.".to_owned())
                        .and_then(|c| c.begin_task_turn(&pending.prompt, attempted)),
                }
            };
            match result {
                Ok(Phase::Submitted) => {
                    let token = match pending.provider {
                        Provider::Codex => self
                            .chats
                            .iter()
                            .find(|c| c.pane == pane)
                            .map(|c| c.task_completion.token()),
                        Provider::Claude => self
                            .claude_chats
                            .iter()
                            .find(|c| c.pane == pane)
                            .map(|c| c.task_completion.token()),
                    };
                    if let Some(token) = token {
                        pending.prompt.clear();
                        self.owner_task_turns.push(Running { pending, token });
                    }
                    changed = true;
                }
                Ok(phase @ (Phase::Pending | Phase::Connecting)) => {
                    pending.phase = phase;
                    self.owner_task_launches.push(pending);
                }
                Ok(_) => {
                    self.fail_owner_task(
                        &pending,
                        "Task first turn cancelled because the chat was already used.".into(),
                    );
                    changed = true;
                }
                Err(error) => {
                    self.fail_owner_task(&pending, error);
                    changed = true;
                }
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_chat(
        app: &mut Tessel,
        provider: Provider,
        pane: u64,
        project: u64,
        cwd: PathBuf,
    ) -> std::sync::mpsc::Receiver<serde_json::Value> {
        match provider {
            Provider::Codex => {
                let mut c = crate::chat::Chat::new(project, cwd);
                c.pane = pane;
                let rx = c.ready_sender_for_test();
                app.chats.push(c);
                rx
            }
            Provider::Claude => {
                let mut c = crate::claude::Chat::new(project, cwd, None);
                c.pane = pane;
                let rx = c.ready_sender_for_test();
                app.claude_chats.push(c);
                rx
            }
        }
    }

    fn review_fixture(
        worker: Provider,
        reviewer: Provider,
    ) -> (
        tempfile::TempDir,
        Tessel,
        std::sync::mpsc::Receiver<serde_json::Value>,
    ) {
        let (temp, mut app, mut spec) = fixture(worker);
        let pane = app
            .new_session_with_history(
                Some(spec.project),
                reviewer.launch(),
                None,
                false,
                None,
                None,
            )
            .unwrap();
        assert_eq!(pane, 70);
        let rx = synthetic_chat(&mut app, reviewer, pane, spec.project, spec.source.clone());
        spec.reviewer = Some(app.task_reviewer_choices(spec.project).remove(0).target);
        let project = spec.project;
        let cwd = spec.source.clone();
        app.start_owner_native_task(spec).unwrap();
        let worker_rx = synthetic_chat(&mut app, worker, 71, project, cwd);
        app.poll_owner_task_launches();
        worker_rx.try_recv().unwrap();
        (temp, app, rx)
    }

    #[test]
    fn native_review_cannot_queue_an_unsaved_or_unfinished_card() {
        let (_temp, mut app, rx) = review_fixture(Provider::Codex, Provider::Claude);
        let pending = &app.owner_task_turns[0].pending;
        let ticket = pending.ticket.clone();
        let target = pending.reviewer.clone().unwrap();
        let expected = pending.expected.clone();
        assert!(
            app.queue_owner_review(ticket.clone(), expected.clone(), target.clone(), "Fixture")
                .is_err()
        );
        let mut unsaved = expected;
        unsaved.column = 2;
        assert!(
            app.queue_owner_review(ticket, unsaved, target, "Fixture")
                .is_err()
        );
        assert!(app.owner_task_reviews.is_empty());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn native_review_dispatches_once_after_durable_completion_and_waits_for_draft() {
        for worker in [Provider::Codex, Provider::Claude] {
            for reviewer in [Provider::Codex, Provider::Claude] {
                let (temp, mut app, rx) = review_fixture(worker, reviewer);
                match reviewer {
                    Provider::Codex => app
                        .chats
                        .iter_mut()
                        .find(|c| c.pane == 70)
                        .unwrap()
                        .gpui_set_composer("Owner draft".into()),
                    Provider::Claude => app
                        .claude_chats
                        .iter_mut()
                        .find(|c| c.pane == 70)
                        .unwrap()
                        .gpui_set_composer("Owner draft".into()),
                }
                assert!(!app.poll_owner_task_reviews());
                completed(&mut app, worker, true, true);
                assert!(app.poll_owner_task_completions());
                assert_eq!(app.state.tasks[0].column, 2);
                assert_eq!(app.owner_task_reviews.len(), 1);
                assert!(rx.try_recv().is_err());
                for _ in 0..3 {
                    assert!(!app.poll_owner_task_reviews());
                }
                match reviewer {
                    Provider::Codex => {
                        let c = app.chats.iter_mut().find(|c| c.pane == 70).unwrap();
                        assert_eq!(c.composer_state().1, "Owner draft");
                        c.gpui_set_composer(String::new());
                    }
                    Provider::Claude => {
                        let c = app.claude_chats.iter_mut().find(|c| c.pane == 70).unwrap();
                        assert_eq!(c.composer_state().1, "Owner draft");
                        c.gpui_set_composer(String::new());
                    }
                }
                assert!(app.poll_owner_task_reviews());
                let wire = rx.try_recv().unwrap();
                let prompt = match reviewer {
                    Provider::Codex => wire["params"]["input"][0]["text"].as_str(),
                    Provider::Claude => wire["message"]["content"][0]["text"].as_str(),
                }
                .unwrap();
                assert!(prompt.starts_with("[Tessel review]"));
                assert!(prompt.ends_with("Do not modify any files; only report."));
                assert!(app.owner_task_reviews.is_empty());
                assert!(!app.poll_owner_task_reviews());
                assert!(rx.try_recv().is_err());
                drop(app);
                let (_store, saved) = Store::open(&temp.path().join("state")).unwrap();
                assert_eq!(saved.tasks[0].column, 2);
            }
        }
    }

    #[test]
    fn native_review_stale_context_failed_save_and_target_stop_never_send() {
        for reviewer in [Provider::Codex, Provider::Claude] {
            for case in 0..6 {
                let (_temp, mut app, rx) = review_fixture(Provider::Codex, reviewer);
                if case == 0 {
                    app.store = None;
                }
                completed(&mut app, Provider::Codex, true, true);
                app.poll_owner_task_completions();
                match case {
                    1 => app.state.tasks[0].title = "Owner edit".into(),
                    2 => app.state.projects[0].path.push("retargeted"),
                    3 => app
                        .sessions
                        .iter_mut()
                        .find(|s| s.id == 70)
                        .unwrap()
                        .cwd
                        .push("retargeted"),
                    4 => match reviewer {
                        Provider::Codex => app
                            .chats
                            .iter_mut()
                            .find(|c| c.pane == 70)
                            .unwrap()
                            .gpui_stop(),
                        Provider::Claude => app
                            .claude_chats
                            .iter_mut()
                            .find(|c| c.pane == 70)
                            .unwrap()
                            .gpui_stop(),
                    },
                    5 => app.cancel_owner_task_pane(70),
                    _ => {}
                }
                app.poll_owner_task_reviews();
                assert!(app.owner_task_reviews.is_empty());
                assert!(rx.try_recv().is_err());
                assert_eq!(app.state.tasks[0].column, if case == 0 { 1 } else { 2 });
            }
        }
    }

    #[cfg(feature = "visual-test")]
    #[test]
    fn native_review_form_captures_conversation_and_rejects_replacement_before_launch() {
        let (_temp, mut app, spec) = fixture(Provider::Codex);
        let pane = app
            .new_session_with_history(
                Some(spec.project),
                Launch::CodexChat,
                None,
                false,
                None,
                None,
            )
            .unwrap();
        let rx = synthetic_chat(&mut app, Provider::Codex, pane, spec.project, spec.source);
        app.agents = crate::agent_detection::Detection::model_picker_fixture("codex");
        app.open_new_task();
        let epoch = app.new_task.epoch;
        app.new_task_command(epoch, crate::new_task_form::Command::Reviewer(Some(pane)));
        assert_eq!(app.new_task.reviewer, Some(pane));
        app.chats[0].gpui_stop();
        let _new_rx = app.chats[0].ready_sender_for_test();
        app.new_task_command(
            epoch,
            crate::new_task_form::Command::Edit(
                crate::new_task_form::Field::Title,
                "Fixture".into(),
            ),
        );
        app.new_task_command(epoch, crate::new_task_form::Command::Isolated(false));
        app.new_task_command(epoch, crate::new_task_form::Command::Start);
        assert!(app.new_task.open);
        assert!(app.state.tasks.is_empty());
        assert!(app.new_task.error.contains("reviewer changed"));
        assert!(rx.try_recv().is_err());
        // Drain the read-only probe before releasing its temporary directory.
        let deadline = Instant::now() + Duration::from_secs(30);
        while app.worktree_state.is_busy() {
            app.poll_worktrees();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn completion_fixture(provider: Provider) -> (tempfile::TempDir, Tessel) {
        let (temp, mut app, spec) = fixture(provider);
        let project = spec.project;
        let cwd = spec.source.clone();
        app.start_owner_native_task(spec).unwrap();
        let rx = match provider {
            Provider::Codex => {
                let mut c = crate::chat::Chat::new(project, cwd);
                c.pane = 70;
                let rx = c.ready_sender_for_test();
                app.chats.push(c);
                rx
            }
            Provider::Claude => {
                let mut c = crate::claude::Chat::new(project, cwd, None);
                c.pane = 70;
                let rx = c.ready_sender_for_test();
                app.claude_chats.push(c);
                rx
            }
        };
        app.poll_owner_task_launches();
        rx.try_recv().unwrap();
        assert_eq!(app.owner_task_turns.len(), 1);
        (temp, app)
    }

    fn completed(app: &mut Tessel, provider: Provider, marker: bool, success: bool) {
        use serde_json::json;
        let pane = app.owner_task_turns[0].pending.pane.unwrap();
        let text = if marker {
            "Checks passed.\nTASK_COMPLETE"
        } else {
            "Still working."
        };
        match provider {
            Provider::Codex => {
                let c = app.chats.iter_mut().find(|c| c.pane == pane).unwrap();
                let thread = c.thread_id.clone().unwrap();
                c.task_frame_for_test(json!({"method":"turn/started","params":{"threadId":thread,"turn":{"id":"task-turn"}}}));
                c.task_frame_for_test(json!({"method":"item/completed","params":{"threadId":thread,"turnId":"task-turn","item":{"type":"agentMessage","id":"answer","phase":"final_answer","text":text}}}));
                c.task_frame_for_test(json!({"method":"turn/completed","params":{"threadId":thread,"turn":{"id":"task-turn","status":if success {"completed"} else {"failed"}}}}));
            }
            Provider::Claude => {
                let c = app
                    .claude_chats
                    .iter_mut()
                    .find(|c| c.pane == pane)
                    .unwrap();
                c.task_frame_for_test(json!({"type":"assistant","session_id":"synthetic-session","message":{"id":"answer","content":[{"type":"text","text":text}]}}));
                c.task_frame_for_test(json!({"type":"result","session_id":"synthetic-session","is_error":!success,"subtype":if success {"success"} else {"error_during_execution"}}));
            }
        }
    }

    #[test]
    fn owner_task_completion_success_is_saved_once_and_failures_stay_doing() {
        for provider in [Provider::Codex, Provider::Claude] {
            for (marker, success) in [(true, true), (true, false), (false, true)] {
                let (temp, mut app) = completion_fixture(provider);
                assert!(!app.poll_owner_task_completions());
                assert_eq!(app.state.tasks[0].column, 1);
                completed(&mut app, provider, marker, success);
                assert_eq!(app.poll_owner_task_completions(), marker && success);
                assert_eq!(
                    app.state.tasks[0].column,
                    if marker && success { 2 } else { 1 }
                );
                assert!(app.owner_task_turns.is_empty());
                assert!(!app.poll_owner_task_completions());
                let expected = app.state.tasks[0].clone();
                drop(app);
                let (_store, reopened) = Store::open(&temp.path().join("state")).unwrap();
                assert_eq!(reopened.tasks[0], expected);
            }
        }
    }

    #[test]
    fn owner_task_completion_stale_context_cancel_and_storage_failure_never_overwrite() {
        for provider in [Provider::Codex, Provider::Claude] {
            for case in 0..6 {
                let (_temp, mut app) = completion_fixture(provider);
                completed(&mut app, provider, true, true);
                match case {
                    0 => app.state.tasks[0].title = "Owner edit".into(),
                    1 => app.state.projects[0].path.push("retargeted"),
                    2 => app.sessions[0].cwd.push("retargeted"),
                    3 => app.cancel_owner_task_pane(70),
                    4 => app.store = None,
                    _ => match provider {
                        Provider::Codex => app.chats[0].task_completion.arm(),
                        Provider::Claude => app.claude_chats[0].task_completion.arm(),
                    },
                }
                app.poll_owner_task_completions();
                assert_eq!(app.state.tasks[0].column, 1);
                assert!(app.owner_task_turns.is_empty());
                if case == 4 {
                    assert!(app.error.contains("could not be saved"));
                }
                assert!(!app.poll_owner_task_completions());
            }
        }
    }

    fn fixture(provider: Provider) -> (tempfile::TempDir, Tessel, Spec) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().join("state")), false, || {});
        let project = app.state.add_project(&root).unwrap();
        let source = app
            .state
            .projects
            .iter()
            .find(|p| p.id == project)
            .unwrap()
            .path
            .clone();
        app.next_session = 70;
        let spec = Spec {
            project,
            source,
            title: "Fixture task".into(),
            brief: "Synthetic instructions".into(),
            provider,
            reviewer: None,
            isolated: false,
            base_branch: None,
            copy_env: false,
            run_setup: false,
        };
        (temp, app, spec)
    }

    #[test]
    fn owner_task_launch_binds_actual_pane_and_sends_exact_prompt_once_for_each_native_provider() {
        for provider in [Provider::Codex, Provider::Claude] {
            let (_temp, mut app, spec) = fixture(provider);
            let project = spec.project;
            let cwd = spec.source.clone();
            let id = app.start_owner_native_task(spec).unwrap();
            assert_eq!(app.state.tasks[0].id, id);
            assert_eq!(app.state.tasks[0].timeline.pane_id, Some(70));
            assert_eq!(app.state.tasks[0].column, 1);
            assert!(!app.can_restore());
            let expected = app.owner_task_launches[0].prompt.clone();
            let rx = match provider {
                Provider::Codex => {
                    let mut chat = crate::chat::Chat::new(project, cwd);
                    chat.pane = 70;
                    chat.gpui_set_composer("Owner draft".into());
                    let rx = chat.ready_sender_for_test();
                    app.chats.push(chat);
                    rx
                }
                Provider::Claude => {
                    let mut chat = crate::claude::Chat::new(project, cwd, None);
                    chat.pane = 70;
                    chat.gpui_set_composer("Owner draft".into());
                    let rx = chat.ready_sender_for_test();
                    app.claude_chats.push(chat);
                    rx
                }
            };
            // Changing selection is not changing the launch's captured project.
            app.state.active_project = None;
            assert!(app.poll_owner_task_launches());
            let frame = rx.try_recv().unwrap();
            match provider {
                Provider::Codex => {
                    assert_eq!(frame["params"]["input"][0]["text"], expected);
                    assert_eq!(app.chats[0].composer_state().1, "Owner draft");
                }
                Provider::Claude => {
                    assert_eq!(frame["message"]["content"][0]["text"], expected);
                    assert_eq!(app.claude_chats[0].composer_state().1, "Owner draft");
                }
            }
            assert!(app.owner_task_launches.is_empty());
            assert!(!app.poll_owner_task_launches());
            assert!(rx.try_recv().is_err());
        }
    }

    #[test]
    fn owner_task_launch_context_edits_cancel_without_overwriting_card_or_sending() {
        for change in ["task", "project", "pane", "timeout"] {
            let (_temp, mut app, spec) = fixture(Provider::Codex);
            let project = spec.project;
            let cwd = spec.source.clone();
            app.start_owner_native_task(spec).unwrap();
            let mut chat = crate::chat::Chat::new(project, cwd);
            chat.pane = 70;
            let rx = chat.ready_sender_for_test();
            app.chats.push(chat);
            match change {
                "task" => app.state.tasks[0].title = "Owner changed title".into(),
                "project" => {
                    app.state.projects[0].path.push("moved");
                    app.state.sessions[0].cwd = app.state.projects[0].path.clone();
                }
                "pane" => app.sessions[0].cwd.push("moved"),
                _ => app.owner_task_launches[0].started = Instant::now() - Duration::from_secs(31),
            }
            assert!(app.poll_owner_task_launches());
            assert!(rx.try_recv().is_err());
            assert!(app.owner_task_launches.is_empty());
            if change == "task" {
                assert_eq!(app.state.tasks[0].title, "Owner changed title");
            } else {
                assert_eq!(app.state.tasks[0].column, 0, "{change}: {}", app.error);
                assert_eq!(app.state.tasks[0].timeline.pane_id, None);
            }
        }
    }

    #[test]
    fn owner_task_launch_refuses_stale_project_and_failed_storage_before_creating_pane() {
        let (_temp, mut app, mut spec) = fixture(Provider::Claude);
        spec.source.push("wrong");
        assert!(app.start_owner_native_task(spec).is_err());
        assert!(app.state.tasks.is_empty());
        assert!(app.sessions.is_empty());
        let (_temp, mut app, spec) = fixture(Provider::Codex);
        app.store = None;
        assert!(app.start_owner_native_task(spec).is_err());
        assert!(app.state.tasks.is_empty());
        assert!(app.sessions.is_empty());
    }

    #[test]
    fn owner_task_launch_stop_discards_pending_delivery_and_never_interrupts_manual_turn() {
        for manual in [false, true] {
            let (_temp, mut app, spec) = fixture(Provider::Codex);
            let mut chat = crate::chat::Chat::new(spec.project, spec.source.clone());
            chat.pane = 70;
            let rx = chat.ready_sender_for_test();
            app.start_owner_native_task(spec).unwrap();
            if manual {
                assert_eq!(
                    chat.begin_task_turn("Owner manually started this turn", false)
                        .unwrap(),
                    Phase::Submitted
                );
                rx.try_recv().unwrap();
            }
            let snapshot = chat.gpui_snapshot();
            app.chats.push(chat);
            if manual {
                app.poll_owner_task_launches();
                assert_eq!(app.chats[0].gpui_snapshot(), snapshot);
            } else {
                app.apply_gpui_action(gpui_bridge::GpuiAction::ChatStop(70));
                assert!(!app.poll_owner_task_launches());
            }
            assert!(app.owner_task_launches.is_empty());
            assert!(rx.try_recv().is_err());
            assert_eq!(app.state.tasks[0].column, 0);
        }
    }

    #[test]
    fn owner_task_launch_preserves_selection_of_another_project() {
        let (temp, mut app, spec) = fixture(Provider::Codex);
        let other = temp.path().join("other");
        std::fs::create_dir(&other).unwrap();
        let other_id = app.state.add_project(&other).unwrap();
        let pane = app
            .new_session_with_history(Some(other_id), Launch::ClaudeChat, None, false, None, None)
            .unwrap();
        app.state.active_project = Some(other_id);
        app.view = View::Files;
        let task = app.start_owner_native_task(spec).unwrap();
        assert_eq!(app.state.active_project, Some(other_id));
        assert_eq!(app.active_session, Some(pane));
        assert!(app.view == View::Files);
        assert_eq!(
            app.state
                .tasks
                .iter()
                .find(|t| t.id == task)
                .unwrap()
                .timeline
                .pane_id,
            Some(71)
        );
        assert_eq!(app.sessions.len(), 2);
    }

    #[test]
    fn owner_task_launch_worktree_callback_keeps_real_root_base_and_secret_policy() {
        for moved in [false, true] {
            let (temp, mut app, mut spec) = fixture(Provider::Codex);
            let root = spec.source.clone();
            let git = |args: &[&str]| {
                let mut options = vec!["-c", "core.hooksPath=", "-c", "commit.gpgsign=false"];
                options.extend_from_slice(args);
                crate::git::run(&root, &options, None).unwrap()
            };
            git(&["init", "-b", "main"]);
            git(&["config", "user.name", "Fixture"]);
            git(&["config", "user.email", "fixture@example.test"]);
            git(&["config", "core.hooksPath", ""]);
            std::fs::create_dir(root.join("nested")).unwrap();
            std::fs::write(root.join("nested/version"), "base").unwrap();
            std::fs::create_dir(root.join(".tessel")).unwrap();
            std::fs::write(root.join(".tessel/setup.ps1"), "[IO.File]::WriteAllText((Join-Path (Get-Location) 'setup-ran'), 'synthetic'); exit 7").unwrap();
            std::fs::write(root.join(".gitignore"), ".env\n").unwrap();
            git(&["add", "."]);
            git(&["commit", "-m", "fixture base"]);
            git(&["branch", "task-base"]);
            std::fs::write(root.join("nested/version"), "head").unwrap();
            git(&["commit", "-am", "fixture head"]);
            std::fs::write(root.join(".env"), "SYNTHETIC_ONLY=fixture").unwrap();
            spec.source = std::fs::canonicalize(root.join("nested")).unwrap();
            app.state.projects[0].path = spec.source.clone();
            spec.isolated = true;
            spec.base_branch = Some("task-base".into());
            spec.run_setup = cfg!(windows) && !moved;
            let id = app.start_owner_native_task(spec).unwrap();
            assert_eq!(app.state.tasks[0].column, 1);
            assert!(app.sessions.is_empty());
            assert!(!app.can_restore());
            if moved {
                app.state.projects[0].path = temp.path().join("changed");
            }
            let deadline = Instant::now() + Duration::from_secs(60);
            while app.worktree_state.is_busy() {
                app.poll_worktrees();
                assert!(Instant::now() < deadline, "worktree callback timed out");
                std::thread::sleep(Duration::from_millis(10));
            }
            let card = app.state.tasks.iter().find(|t| t.id == id).unwrap();
            let receipt = card
                .preparation
                .as_ref()
                .unwrap()
                .worktree
                .as_ref()
                .unwrap();
            assert!(same_worker_directory(&receipt.root, &root));
            assert_eq!(receipt.base_branch.as_deref(), Some("task-base"));
            assert_eq!(
                std::fs::read_to_string(receipt.path.join("nested/version")).unwrap(),
                "base"
            );
            assert!(!receipt.path.join(".env").exists());
            assert_eq!(
                receipt.path.join("setup-ran").exists(),
                cfg!(windows) && !moved
            );
            if cfg!(windows) && !moved {
                assert!(app.error.contains("setup failed"));
            }
            if moved {
                assert!(app.sessions.is_empty());
                assert!(app.owner_task_launches.is_empty());
                assert_eq!(card.column, 0);
            } else {
                assert_eq!(app.sessions.len(), 1);
                assert!(same_worker_directory(&app.sessions[0].cwd, &receipt.path));
                assert_eq!(card.timeline.pane_id, Some(70));
                assert!(
                    app.owner_task_launches[0]
                        .prompt
                        .contains("made from task-base")
                );
            }
        }
    }
}
