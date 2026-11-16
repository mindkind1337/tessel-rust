//! Owner-selected reviewer delivery, after the card's Review transaction.
use super::*;
use crate::{
    initial_turn::Phase,
    native_review::{Choice, Kind, Target},
    task_preparation::Ticket,
};

pub(super) struct Pending {
    ticket: Ticket,
    expected: crate::workspace::Task,
    target: Target,
    prompt: String,
}

impl Tessel {
    pub(super) fn current_reviewer_target(&self, project: u64, pane: u64) -> Option<Target> {
        let session = self
            .sessions
            .iter()
            .find(|s| s.id == pane && s.project == Some(project))?;
        let (kind, epoch) = match session.launch {
            Launch::CodexChat => (
                Kind::Codex,
                self.chats
                    .iter()
                    .find(|c| {
                        c.pane == pane
                            && c.project == project
                            && same_worker_directory(&c.cwd, &session.cwd)
                    })?
                    .review_identity()?,
            ),
            Launch::ClaudeChat => (
                Kind::Claude,
                self.claude_chats
                    .iter()
                    .find(|c| {
                        c.pane == pane
                            && c.project == project
                            && same_worker_directory(&c.cwd, &session.cwd)
                    })?
                    .review_identity()?,
            ),
            _ => return None,
        };
        Some(Target {
            pane,
            project,
            cwd: session.cwd.clone(),
            kind,
            epoch,
        })
    }

    pub(super) fn reviewer_current(&self, target: &Target) -> bool {
        self.state.projects.iter().any(|p| p.id == target.project)
            && self
                .current_reviewer_target(target.project, target.pane)
                .as_ref()
                == Some(target)
            && !self
                .state
                .tasks
                .iter()
                .any(|t| t.column != 3 && t.timeline.pane_id == Some(target.pane))
    }

    pub(super) fn task_reviewer_choices(&self, project: u64) -> Vec<Choice> {
        self.task_agent_choices(project)
            .into_iter()
            .filter_map(|choice| {
                let target = self.current_reviewer_target(project, choice.id)?;
                self.reviewer_current(&target).then_some(Choice {
                    target,
                    label: choice.label,
                })
            })
            .collect()
    }

    pub(super) fn queue_owner_review(
        &mut self,
        ticket: Ticket,
        expected: crate::workspace::Task,
        target: Target,
        who: &str,
    ) -> Result<(), String> {
        if expected.column != 2
            || expected.id != ticket.task_id()
            || expected.project != ticket.project_id()
            || expected.preparation.as_ref().and_then(|d| d.reviewer_id) != Some(target.pane)
            || !self.state.tasks.iter().any(|t| t == &expected)
        {
            return Err(
                "The completed task must be saved in Review before scheduling its reviewer.".into(),
            );
        }
        if self.owner_task_reviews.len() >= 32
            || !self.reviewer_current(&target)
            || target.project != ticket.project_id()
        {
            return Err("The selected reviewer is unavailable or the review queue is full.".into());
        }
        let details = expected
            .preparation
            .as_ref()
            .ok_or("Review context is missing.")?;
        let cwd = details
            .worktree
            .as_ref()
            .map(|w| w.path.as_path())
            .unwrap_or(&details.source_cwd);
        let branch = details.worktree.as_ref().map(|w| w.branch.as_str());
        let prompt = crate::native_review::prompt(who, cwd, branch)?;
        self.owner_task_reviews.push(Pending {
            ticket,
            expected,
            target,
            prompt,
        });
        Ok(())
    }

    pub(super) fn cancel_owner_reviews_for_pane(&mut self, pane: u64) {
        self.owner_task_reviews.retain(|r| r.target.pane != pane);
    }

    pub(super) fn poll_owner_task_reviews(&mut self) -> bool {
        let mut changed = false;
        for review in std::mem::take(&mut self.owner_task_reviews) {
            if !review.ticket.project_is_current(&self.state)
                || !self.state.tasks.iter().any(|t| t == &review.expected)
                || !self.reviewer_current(&review.target)
            {
                self.error =
                    "Review request cancelled because the task, project or reviewer changed."
                        .into();
                changed = true;
                continue;
            }
            let target = &review.target;
            let result = match target.kind {
                Kind::Codex => self
                    .chats
                    .iter_mut()
                    .find(|c| c.pane == target.pane)
                    .ok_or_else(|| "Reviewer closed.".to_owned())
                    .and_then(|c| c.begin_review_turn(&review.prompt, target.epoch)),
                Kind::Claude => self
                    .claude_chats
                    .iter_mut()
                    .find(|c| c.pane == target.pane)
                    .ok_or_else(|| "Reviewer closed.".to_owned())
                    .and_then(|c| c.begin_review_turn(&review.prompt, target.epoch)),
            };
            match result {
                Ok(Phase::Pending) => self.owner_task_reviews.push(review),
                Ok(Phase::Submitted) => changed = true,
                Ok(_) => {
                    self.error =
                        "Review request cancelled because the conversation changed.".into();
                    changed = true;
                }
                Err(error) => {
                    self.error =
                        format!("Review request could not be sent; it was not retried. {error}");
                    changed = true;
                }
            }
        }
        changed
    }
}
