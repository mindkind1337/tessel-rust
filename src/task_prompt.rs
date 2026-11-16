//! Owner-task instruction text, matching App.vue taskPrompt. All context is
//! descriptive; rendering it does not grant filesystem, team or launch authority.
pub struct Worktree<'a> {
    pub path: &'a str,
    pub branch: &'a str,
    pub base_branch: Option<&'a str>,
}
pub struct Team<'a> {
    pub name: &'a str,
    pub lead: Option<&'a str>,
    pub mailbox_guide: Option<&'a str>,
}
pub struct Context<'a> {
    pub title: &'a str,
    pub brief: &'a str,
    pub cwd: &'a str,
    pub worktree: Option<Worktree<'a>>,
    pub guest_by: Option<&'a str>,
    pub copied_from: Option<&'a str>,
    pub team: Option<Team<'a>>,
}

pub fn render(context: &Context<'_>) -> Result<String, String> {
    // Bound allocation before interpolating untrusted task text. Check only
    // fields that the reference actually renders (cwd is unused in a worktree).
    let fields = [
        Some(context.title),
        Some(context.brief),
        context.worktree.is_none().then_some(context.cwd),
        context.guest_by,
        context.copied_from,
        context.worktree.as_ref().map(|w| w.path),
        context.worktree.as_ref().map(|w| w.branch),
        context.worktree.as_ref().and_then(|w| w.base_branch),
        context.team.as_ref().map(|t| t.name),
        context.team.as_ref().and_then(|t| t.lead),
        context.team.as_ref().and_then(|t| t.mailbox_guide),
    ];
    let mut size = 0usize;
    for text in fields.into_iter().flatten() {
        size = size
            .checked_add(text.len())
            .ok_or("Task instructions are too large.")?;
        if size > crate::initial_turn::MAX_PROMPT_BYTES {
            return Err("Task instructions are too large.".into());
        }
    }
    let where_text = if let Some(wt) = &context.worktree {
        format!(
            "You work in your own copy of the project: {} (git branch {}, made from {}). Commit your work on that branch (git add the files you changed, then git commit). Do not merge it and do not push: the user reviews and merges it. In your last commit message, list the checks you ran (tests, build) and their results. If the project needs its dependencies installed, install them in this copy (for example npm ci); never link them to another folder.",
            wt.path,
            wt.branch,
            wt.base_branch
                .filter(|s| !s.is_empty())
                .unwrap_or("the main branch")
        )
    } else {
        format!(
            "You work directly in the project folder {}. Other agents may work there too: check .tessel/notes.md before editing shared files.",
            context.cwd
        )
    };
    let mut output = format!("[Tessel task] {}\n\n", context.title);
    if let Some(guest) = context.guest_by.filter(|s| !s.is_empty()) {
        output.push_str(&format!(
            "(This card's title was written by {guest}, a Live Share guest, not the user.)\n\n"
        ));
    }
    if let Some(copied) = context.copied_from.filter(|s| !s.is_empty()) {
        output.push_str(&format!("(This card's words came from a copy of the settings made on {copied}, not typed here by the user.)\n\n"));
    }
    if !context.brief.is_empty() {
        output.push_str(context.brief);
        output.push_str("\n\n");
    }
    if let Some(team) = &context.team {
        output.push_str(&format!("You are in team \"{}\"", team.name));
        if let Some(lead) = team.lead {
            output.push_str(&format!(", led by {lead}: it gave you this task and reviews your work when you finish. Ask it if something is unclear."));
        } else {
            output.push('.');
        }
        if let Some(guide) = team.mailbox_guide {
            output.push('\n');
            output.push_str(guide);
        }
        output.push_str("\n\n");
    }
    output.push_str(&where_text);
    output.push_str("\n\nWhen the task is complete and checked, end your last message with a line that contains only the words TASK and COMPLETE joined by an underscore, and nothing else on that line.");
    crate::initial_turn::validate_prompt(&output)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
        value.get(key).and_then(Value::as_str)
    }
    #[test]
    fn task_prompt_matches_all_synthetic_vue_reference_vectors() {
        let reference: Value =
            serde_json::from_str(include_str!("../tests/fixtures/task_prompt.json")).unwrap();
        for case in reference["cases"].as_array().unwrap() {
            let task = &case["task"];
            let context = Context {
                title: text(task, "title").unwrap(),
                brief: text(task, "brief").unwrap_or(""),
                cwd: text(&case["ws"], "cwd").unwrap_or(""),
                worktree: task.get("worktree").map(|w| Worktree {
                    path: text(w, "path").unwrap(),
                    branch: text(w, "branch").unwrap(),
                    base_branch: text(w, "baseBranch"),
                }),
                guest_by: text(task, "guestBy"),
                copied_from: text(task, "copiedFrom"),
                team: case.get("team").map(|t| Team {
                    name: text(t, "name").unwrap(),
                    lead: case.get("lead").and_then(|lead| text(lead, "label")),
                    mailbox_guide: text(case, "guide"),
                }),
            };
            assert_eq!(
                render(&context).unwrap(),
                case["expected"].as_str().unwrap(),
                "{}",
                case["name"]
            );
        }
    }
    #[test]
    fn task_prompt_refuses_oversized_or_null_content_without_truncating_or_echoing_it() {
        let mut context = Context {
            title: "Fixture",
            brief: "",
            cwd: "/fixture",
            worktree: None,
            guest_by: None,
            copied_from: None,
            team: None,
        };
        let large = "é".repeat(33_000);
        context.brief = &large;
        assert!(render(&context).is_err());
        context.brief = "hidden\0fixture";
        assert!(!render(&context).unwrap_err().contains("hidden"));
        // Fields fit individually, but the final protocol payload also includes instructions.
        let nearly_full = "x".repeat(crate::initial_turn::MAX_PROMPT_BYTES - 100);
        context.brief = &nearly_full;
        assert!(render(&context).is_err());
    }
}
