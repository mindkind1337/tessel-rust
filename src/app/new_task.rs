use super::*;
use crate::new_task_form::{Choice, Command};

impl Tessel {
    pub(crate) fn open_new_task(&mut self) {
        let Some(project) = self.state.active().cloned() else {
            self.error = tr("Select a project for native chat.").into();
            return;
        };
        let choices = [("codex", "Codex"), ("claude", "Claude")]
            .into_iter()
            .map(|(id, name)| Choice {
                id: id.into(),
                label: name.into(),
                enabled: self
                    .agents
                    .entries()
                    .iter()
                    .any(|(kind, installed)| kind == id && *installed),
            })
            .collect();
        let reviewers = self
            .task_agent_choices(project.id)
            .into_iter()
            .filter(|a| {
                !self
                    .state
                    .tasks
                    .iter()
                    .any(|t| t.column != 3 && t.timeline.pane_id == Some(a.id))
            })
            .collect();
        self.new_task.begin(
            project.id,
            project.path.clone(),
            project.name,
            choices,
            reviewers,
        );
        self.worktree_state
            .refresh_project(project.id, project.path);
        if !self.agents.loaded() && !self.agents.detecting() {
            self.detect_agents();
        }
    }

    pub(super) fn refresh_new_task_agents(&mut self) {
        if !self.new_task.open {
            return;
        }
        for choice in &mut self.new_task.choices {
            choice.enabled = self
                .agents
                .entries()
                .iter()
                .any(|(id, installed)| id == &choice.id && *installed);
        }
        if self.new_task.provider.is_empty() {
            self.new_task.provider = self
                .new_task
                .choices
                .iter()
                .find(|c| c.enabled)
                .map(|c| c.id.clone())
                .unwrap_or_default();
        }
    }

    pub(crate) fn new_task_command(&mut self, epoch: u64, command: Command) {
        if !self.new_task.accepts(epoch) {
            return;
        }
        match command {
            Command::Cancel => self.new_task.open = false,
            Command::Edit(field, text) => self.new_task.edit(field, text),
            Command::Provider(id) => {
                if self
                    .new_task
                    .choices
                    .iter()
                    .any(|c| c.id == id && c.enabled)
                {
                    self.new_task.provider = id;
                }
            }
            Command::Isolated(value) => {
                if !value || self.new_task.isolation == Some(true) {
                    self.new_task.isolated = value;
                }
            }
            Command::CopyEnv(value) => self.new_task.copy_env = value,
            Command::Reviewer(id) => {
                self.new_task.reviewer = None;
                if id.is_some() {
                    self.new_task.error = "Automatic review requests are not available yet.".into();
                }
            }
            Command::Start => {
                self.refresh_new_task_agents();
                if !self.new_task.ready() {
                    return;
                }
                let form = &self.new_task;
                let provider = match form.provider.as_str() {
                    "codex" => task_launch::Provider::Codex,
                    "claude" => task_launch::Provider::Claude,
                    _ => return,
                };
                let spec = task_launch::Spec {
                    project: form.project,
                    source: form.source.clone(),
                    title: form.title.trim().into(),
                    brief: form.brief.trim().into(),
                    provider,
                    reviewer: form.reviewer,
                    isolated: form.use_copy(),
                    base_branch: (!form.branch.trim().is_empty())
                        .then(|| form.branch.trim().to_owned()),
                    copy_env: form.copy_env,
                };
                match self.start_owner_native_task(spec) {
                    Ok(_) => {
                        self.new_task.open = false;
                        self.state.panels.right_open = true;
                        self.state.panels.right_tab = RightTab::Tasks;
                    }
                    Err(error) => self.new_task.error = error,
                }
            }
        }
    }
}

#[cfg(all(test, feature = "visual-test"))]
mod tests {
    use super::*;
    use crate::new_task_form::Field;
    fn fixture() -> (tempfile::TempDir, Tessel) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().join("state")), false, || {});
        app.state.add_project(&root).unwrap();
        app.agents = crate::agent_detection::Detection::model_picker_fixture("codex");
        app.open_new_task();
        // Drain the bounded read-only inspection before dropping its temporary directory.
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while app.new_task.isolation.is_none() {
            app.poll_worktrees();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        (temp, app)
    }
    #[test]
    fn new_task_form_owner_start_uses_draft_once_and_cannot_be_replayed() {
        let (_temp, mut app) = fixture();
        let epoch = app.new_task.epoch;
        app.new_task_command(
            epoch,
            Command::Edit(Field::Title, "  Fixture task  ".into()),
        );
        app.new_task_command(
            epoch,
            Command::Edit(Field::Brief, " Exact fixture instructions ".into()),
        );
        app.new_task_command(epoch, Command::Isolated(false));
        app.new_task_command(epoch, Command::Start);
        assert!(!app.new_task.open);
        assert_eq!(app.state.tasks.len(), 1);
        assert_eq!(app.sessions.len(), 1);
        assert_eq!(app.state.tasks[0].title, "Fixture task");
        assert_eq!(
            app.state.tasks[0].preparation.as_ref().unwrap().brief,
            "Exact fixture instructions"
        );
        assert_eq!(app.owner_task_launches.len(), 1);
        assert!(app.chats.is_empty()); // No real provider is launched by this fixture.
        app.new_task_command(epoch, Command::Start);
        assert_eq!(app.state.tasks.len(), 1);
    }
    #[test]
    fn new_task_form_rechecks_project_and_discards_old_dialog_events() {
        let (_temp, mut app) = fixture();
        let epoch = app.new_task.epoch;
        app.new_task_command(epoch, Command::Edit(Field::Title, "Keep this draft".into()));
        app.new_task_command(epoch, Command::Isolated(false));
        app.state.projects[0].path.push("changed");
        app.new_task_command(epoch, Command::Start);
        assert!(app.new_task.open);
        assert!(!app.new_task.error.is_empty());
        assert!(app.state.tasks.is_empty());
        assert!(app.sessions.is_empty());
        app.new_task_command(epoch, Command::Cancel);
        app.new_task
            .begin(9, std::env::temp_dir(), "Other".into(), vec![], vec![]);
        app.new_task_command(epoch, Command::Edit(Field::Title, "Stale".into()));
        app.new_task_command(epoch, Command::Start);
        assert!(app.new_task.title.is_empty());
        assert!(app.state.tasks.is_empty());
    }
}
