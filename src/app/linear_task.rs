//! Owner UI launch route. Captured project and account authority never come from shared state.
use super::*;

impl Tessel {
    #[cfg(any(test, feature = "visual-test"))]
    pub(crate) fn install_linear_view_fixture(&mut self) {
        self.agents = crate::agent_detection::Detection::model_picker_fixture("codex");
        self.linear.view = crate::linear_model::panel::fixture_view();
    }
    pub(super) fn configure_linear_task(&mut self) {
        if self.view != View::Remote
            && !self.share.joined_or_waiting()
            && let Some(project) = self.state.active().filter(|p| p.path.is_dir())
        {
            self.linear.project = Some((project.id, project.path.clone()));
            self.linear.view.project_name = project.name.clone();
        }
        self.refresh_linear_agents();
        self.linear.view.agent = self
            .linear
            .view
            .agents
            .iter()
            .find(|a| a.available)
            .map(|a| a.id.clone())
            .unwrap_or_default();
        if !self.agents.loaded() && !self.agents.detecting() {
            self.detect_agents();
        }
    }

    fn refresh_linear_agents(&mut self) {
        self.linear.view.agents = [("codex", "Codex"), ("claude", "Claude")]
            .into_iter()
            .map(|(id, name)| crate::linear_model::panel::AgentChoice {
                id: id.into(),
                label: name.into(),
                available: self
                    .agents
                    .entries()
                    .iter()
                    .any(|(kind, installed)| kind == id && *installed),
            })
            .collect();
        if self.linear.view.agent.is_empty() {
            self.linear.view.agent = self
                .linear
                .view
                .agents
                .iter()
                .find(|a| a.available)
                .map(|a| a.id.clone())
                .unwrap_or_default();
        }
    }

    pub(super) fn poll_linear_task(&mut self) -> bool {
        if self.linear.view.open {
            self.refresh_linear_agents();
        }
        let mut changed = false;
        if let Some(warning) = self.linear.take_warning() {
            self.error = warning;
            changed = true;
        }
        if let Some(epoch) = self.linear.requested_start.take() {
            if !self.linear.accepts(epoch) {
                return changed;
            }
            if let Err(error) = self.start_linear_task(epoch) {
                self.linear.view.error = error;
            }
            changed = true;
        }
        changed
    }

    fn start_linear_task(&mut self, epoch: u64) -> Result<(), String> {
        let (project, source) = self
            .linear
            .project
            .clone()
            .ok_or("Select a local owner project before opening Linear")?;
        if self.share.joined_or_waiting()
            || !self
                .state
                .projects
                .iter()
                .any(|p| p.id == project && same_worker_directory(&p.path, &source))
        {
            return Err("The captured project changed; reopen Linear".into());
        }
        let provider = match self.linear.view.agent.as_str() {
            "codex" => task_launch::Provider::Codex,
            "claude" => task_launch::Provider::Claude,
            _ => return Err("Choose an available native agent".into()),
        };
        let metadata = self.linear.begin_launch(epoch)?;
        let key = metadata.launch_key().unwrap();
        let spec = task_launch::Spec {
            project,
            source,
            title: metadata.title(),
            brief: metadata.brief(),
            provider,
            existing: None,
            reviewer: None,
            isolated: self.linear.view.isolated,
            base_branch: None,
            copy_env: false,
            run_setup: false,
            external: Some(metadata),
        };
        if let Err(error) = self.start_owner_native_task(spec) {
            self.linear.launch_finished(key, Err(error.clone()));
            return Err(error);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linear_model::panel::Command;
    use std::sync::atomic::Ordering;

    #[test]
    fn linear_owner_click_uses_captured_project_and_updates_after_native_acceptance() {
        for provider in ["codex", "claude"] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("project");
            std::fs::create_dir(&root).unwrap();
            let mut app =
                Tessel::new_for_gpui_in_dir(Some(temp.path().join("state")), false, || {});
            let project = app.state.add_project(&root).unwrap();
            app.agents = crate::agent_detection::Detection::model_picker_fixture(provider);
            let (panel, updates) = crate::linear_model::panel::tests::launch_fixture(false);
            app.linear = panel;
            app.configure_linear_task();
            let epoch = app.linear.view.epoch;
            app.linear.command(epoch, Command::Isolated(false));
            app.linear
                .command(epoch, Command::NextState(Some("team-working".into())));
            app.linear.command(epoch, Command::Start);
            // The dialog retains its source even if the active project changes afterwards.
            app.state.active_project = None;
            assert!(app.poll_linear_task());
            assert!(app.linear.view.busy, "{}", app.linear.view.error);
            assert_eq!(app.state.tasks.len(), 1);
            assert_eq!(updates.load(Ordering::SeqCst), 0);
            let pane = app.sessions[0].id;
            let cwd = app.sessions[0].cwd.clone();
            let rx = if provider == "codex" {
                let mut chat = crate::chat::Chat::new(project, cwd);
                chat.pane = pane;
                let rx = chat.ready_sender_for_test();
                app.chats.push(chat);
                rx
            } else {
                let mut chat = crate::claude::Chat::new(project, cwd, None);
                chat.pane = pane;
                let rx = chat.ready_sender_for_test();
                app.claude_chats.push(chat);
                rx
            };
            assert!(app.poll_owner_task_launches());
            assert!(rx.try_recv().is_ok());
            assert_eq!(app.active_session, Some(pane));
            assert_eq!(app.state.active_project, Some(project));
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while app.linear.view.open {
                app.linear.poll();
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(updates.load(Ordering::SeqCst), 1);
            app.linear.command(epoch, Command::Start);
            app.poll_linear_task();
            assert_eq!(app.state.tasks.len(), 1);
            assert!(rx.try_recv().is_err());
        }
    }

    #[test]
    fn linear_project_retarget_and_unavailable_agent_do_not_create_a_task() {
        for retarget in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("project");
            std::fs::create_dir(&root).unwrap();
            let mut app =
                Tessel::new_for_gpui_in_dir(Some(temp.path().join("state")), false, || {});
            app.state.add_project(&root).unwrap();
            app.agents = crate::agent_detection::Detection::model_picker_fixture("codex");
            app.linear = crate::linear_model::panel::tests::launch_fixture(false).0;
            app.configure_linear_task();
            if retarget {
                app.state.projects[0].path.push("changed");
            } else {
                app.agents = crate::agent_detection::Detection::model_picker_fixture("none");
            }
            app.linear.command(app.linear.view.epoch, Command::Start);
            assert!(app.poll_linear_task());
            assert!(!app.linear.view.error.is_empty());
            assert!(app.state.tasks.is_empty());
            assert!(app.sessions.is_empty());
            assert!(!app.linear.view.busy);
        }
    }
}
