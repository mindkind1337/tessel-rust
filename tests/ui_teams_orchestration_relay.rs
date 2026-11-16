#[allow(dead_code)]
#[path = "../src/agent_catalog.rs"]
mod agent_catalog;
#[allow(dead_code)]
#[path = "../src/pane_numbers.rs"]
mod pane_numbers;
#[allow(dead_code)]
#[path = "../src/settings_extra_model.rs"]
mod settings_extra_model;
#[allow(dead_code)]
#[path = "../src/sidebar_model.rs"]
mod sidebar_model;
#[allow(dead_code)]
#[path = "../src/task_report.rs"]
mod task_report;
#[allow(dead_code)]
#[path = "../src/task_timeline.rs"]
mod task_timeline;
#[allow(dead_code)]
#[path = "../src/teams_relay.rs"]
mod teams_relay;
#[allow(dead_code)]
#[path = "../src/teams_ui.rs"]
mod teams_ui;
#[allow(dead_code)]
#[path = "../src/theme.rs"]
mod theme;
#[allow(dead_code)]
#[path = "../src/theme_palettes.rs"]
mod theme_palettes;

#[cfg(windows)]
#[allow(dead_code)]
mod vault {
    pub fn protect(data: &[u8], _encrypt: bool) -> Result<Vec<u8>, String> {
        Ok(data.to_vec())
    }
}

#[allow(dead_code)]
mod launch {
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum Launch {
        Shell,
        Agent(String),
        CodexChat,
        ClaudeChat,
    }
}

#[allow(dead_code)]
mod workspace {
    use std::path::{Path, PathBuf};

    use crate::launch::Launch;
    use crate::settings_extra_model::OrchestrationPreferences;
    use crate::teams_relay::SavedTeam;

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Project {
        pub id: u64,
        pub name: String,
        pub path: PathBuf,
    }

    #[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
    pub struct Task {
        pub created_by: Option<u64>,
        pub report: Option<crate::task_report::Report>,
        pub deps: Vec<u64>,
        pub last_waiting: Vec<u64>,
        pub timeline: crate::task_timeline::Timeline,
        pub guest_by: Option<String>,
        pub id: u64,
        pub project: u64,
        pub title: String,
        pub column: usize,
    }

    #[derive(Clone, Debug)]
    pub struct SavedSession {
        pub legacy_number: Option<u32>,
        pub native_host: Option<()>,
        pub last_model: Option<()>,
        pub codex_seed: Option<()>,
        pub conversation_id: Option<String>,
        pub profile_binding: Option<()>,
        pub launch_fingerprint: Option<String>,
        pub session_options: Option<serde_json::Value>,
        pub native_options: Option<serde_json::Value>,
        pub worktree: Option<()>,
        pub pane_name: Option<String>,
        pub broadcast: bool,
        pub id: u64,
        pub project: Option<u64>,
        pub title: String,
        pub launch: Launch,
        pub cwd: PathBuf,
    }

    #[derive(Clone, Debug, Default)]
    pub struct Workspace {
        pub projects: Vec<Project>,
        pub sessions: Vec<SavedSession>,
        pub teams: Vec<SavedTeam>,
        pub tasks: Vec<Task>,
        pub orchestration_prefs: OrchestrationPreferences,
        pub next_id: u64,
    }

    impl Workspace {
        pub fn check_storage_budget(&self) -> Result<(), String> {
            if serde_json::to_vec_pretty(&self.tasks).unwrap().len() > 4 * 1024 * 1024 {
                return Err("Task budget exceeded".into());
            }
            Ok(())
        }
        pub fn id(&mut self) -> Result<u64, String> {
            self.next_id = self.next_id.saturating_add(1).max(1);
            Ok(self.next_id)
        }

        pub fn add_project(&mut self, path: &Path) -> Result<u64, String> {
            let id = self.id()?;
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("project")
                .to_owned();
            self.projects.push(Project {
                id,
                name,
                path: path.to_path_buf(),
            });
            Ok(id)
        }
    }
}

use eframe::egui;
use serde_json::json;
use teams_relay::{
    SessionTeamBadge, TeamsRelayHub, UNTRUSTED_RELAY_HEADER, WakeBanner, WakeChannel, WorkerState,
    canonical_pair_key,
};
use teams_ui::{
    TeamsUiState, paint_pane_team_pills, show_relay_approval_modal, show_team_manager_modal,
    show_unconfirmed_wake_banner, show_worker_confirm_modal,
};
use theme_palettes::ThemeId;
use workspace::{SavedSession, Workspace};

fn make_session(id: u64, project: u64, name: &str, agent: &str) -> SavedSession {
    SavedSession {
        legacy_number: None,
        profile_binding: None,
        native_host: None,
        last_model: None,
        codex_seed: None,
        conversation_id: None,
        launch_fingerprint: None,
        session_options: None,
        native_options: None,
        worktree: None,
        pane_name: Some(name.to_owned()),
        broadcast: true,
        id,
        project: Some(project),
        title: name.to_owned(),
        launch: launch::Launch::Agent(agent.to_owned()),
        cwd: std::path::PathBuf::from("."),
    }
}

#[test]
fn team_tools_send_addressing_gates_and_worker_lifecycle_end_to_end() {
    let mut ws = Workspace::default();
    let p1 = ws
        .add_project(std::path::Path::new("/tmp/proj-alpha"))
        .unwrap();
    let p2 = ws
        .add_project(std::path::Path::new("/tmp/proj-beta"))
        .unwrap();
    ws.sessions.push(make_session(1, p1, "Ada", "claude"));
    ws.sessions.push(make_session(2, p1, "Grace", "codex"));
    ws.sessions.push(make_session(3, p2, "Linus", "claude"));

    let mut hub = TeamsRelayHub::default();
    let now = 20_000_000_u64;

    let (boot, changed) = hub
        .execute_tool(
            &mut ws,
            1,
            p1,
            "team_bootstrap",
            &json!({"name": "Team 2", "tasks": ["Audit auth", "Ship relay"]}),
            now,
        )
        .unwrap();
    assert!(changed);
    assert_eq!(boot["task_ids"].as_array().unwrap().len(), 2);
    ws.teams[0].members.push(2);

    // Send by `@codex`, `#2`, and `@lead`:
    let (sent, _) = hub
        .execute_tool(
            &mut ws,
            1,
            p1,
            "team_send",
            &json!({"to": "@codex", "text": "Ready to pair?"}),
            now + 10,
        )
        .unwrap();
    assert_eq!(sent["delivered_to"], json!([2]));

    let (asked, _) = hub
        .execute_tool(
            &mut ws,
            2,
            p1,
            "team_ask",
            &json!({"question": "Should we use strict mode?"}),
            now + 20,
        )
        .unwrap();
    assert_eq!(asked["asked"], json!([1]));

    let badges = hub.session_badges(&ws);
    assert_eq!(badges[&1].team_number, "2");
    assert!(badges[&1].is_lead);
    assert_eq!(badges[&1].unread_count, 1);
    assert_eq!(badges[&2].unread_count, 1);

    // Claim inbox clears unread count:
    let (inbox2, _) = hub
        .execute_tool(&mut ws, 2, p1, "team_inbox", &json!({}), now + 30)
        .unwrap();
    assert_eq!(inbox2["messages"].as_array().unwrap().len(), 1);
    assert_eq!(hub.unread_team_count(2), 0);

    // Start worker -> Confirming -> Approve -> Complete:
    let (wstart, _) = hub
        .execute_tool(
            &mut ws,
            1,
            p1,
            "team_worker_start",
            &json!({"agent": "gemini", "name": "Scout", "goal": "Check specs"}),
            now + 40,
        )
        .unwrap();
    let wid = wstart["worker_id"].as_str().unwrap().to_owned();
    assert_eq!(hub.workers[0].state, WorkerState::Confirming);
    let spawn = hub.approve_worker_start(&ws, &wid, now + 50).unwrap();
    assert_eq!(spawn.agent, "gemini");
    ws.sessions.push(make_session(4, p1, "Scout", "gemini"));
    hub.bind_spawned_worker_pane(&mut ws, &wid, 4, now + 60);

    let _ = hub
        .execute_tool(
            &mut ws,
            4,
            p1,
            "team_heartbeat",
            &json!({"status": "50% scanned"}),
            now + 70,
        )
        .unwrap();
    let _ = hub
        .execute_tool(
            &mut ws,
            4,
            p1,
            "team_worker_done",
            &json!({"summary": "Specs verified"}),
            now + 80,
        )
        .unwrap();
    assert_eq!(hub.workers[0].state, WorkerState::Done);

    // Cross-project relay + decline 10m cooldown vs approve:
    let _ = hub
        .execute_tool(
            &mut ws,
            1,
            p1,
            "project_send",
            &json!({"to": "Linus", "text": "Hello from Alpha"}),
            now + 90,
        )
        .unwrap();
    let pair = canonical_pair_key(p1, p2);
    assert_eq!(hub.resolve_pair_approval(pair, false, 30, now + 100), 1);
    // Immediate retry within 10m cooldown is rejected:
    assert!(
        hub.execute_tool(
            &mut ws,
            1,
            p1,
            "project_send",
            &json!({"to": "Linus", "text": "Retry too soon"}),
            now + 200,
        )
        .is_err()
    );
    // After 10m cooldown expires, send & approve succeeds:
    let after_cooldown = now + 700_000;
    let _ = hub
        .execute_tool(
            &mut ws,
            1,
            p1,
            "project_send",
            &json!({"to": "Linus", "text": "Hello after cooldown"}),
            after_cooldown,
        )
        .unwrap();
    assert_eq!(
        hub.resolve_pair_approval(pair, true, 10, after_cooldown + 10),
        1
    );
    let (pinbox, _) = hub
        .execute_tool(
            &mut ws,
            3,
            p2,
            "project_inbox",
            &json!({}),
            after_cooldown + 20,
        )
        .unwrap();
    assert_eq!(pinbox["count"], 1);
    assert!(
        pinbox["framed"]
            .as_str()
            .unwrap()
            .starts_with(UNTRUSTED_RELAY_HEADER)
    );
}

#[test]
fn teams_modals_wake_banner_and_header_pills_render_headless_en_fr() {
    let mut ws = Workspace::default();
    let p1 = ws.add_project(std::path::Path::new("/tmp/p1")).unwrap();
    let p2 = ws.add_project(std::path::Path::new("/tmp/p2")).unwrap();
    ws.sessions.push(make_session(1, p1, "Ada", "claude"));
    ws.sessions.push(make_session(2, p2, "Linus", "codex"));

    let mut hub = TeamsRelayHub::default();
    let now = 50_000_000_u64;
    let _ = hub
        .request_worker_start(&mut ws, 1, "codex", Some("Runner"), "Run suite", now)
        .unwrap();
    let _ = hub
        .send_project_relay(&ws, 1, p1, "Linus", "Cross-project ping", None, now)
        .unwrap();

    let mut ui_state = TeamsUiState::default();
    ui_state.open_manager(p1);
    let pal = ThemeId::Classic.spec().ui;
    let badge = SessionTeamBadge {
        team_id: "t1".into(),
        team_name: "Team 1".into(),
        team_number: "1".into(),
        is_lead: true,
        worker_of: Some("Ada".into()),
        unread_count: 3,
    };
    let banner = WakeBanner {
        pane_id: 1,
        channel: WakeChannel::Team,
        reminder_line: teams_relay::TEAM_WAKE_REMINDER.into(),
    };

    for french in [false, true] {
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        let out = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let _ = paint_pane_team_pills(ui, &pal, &badge, french);
                });
                let _ = show_unconfirmed_wake_banner(ui, &pal, &banner, french);
            });
            let _ = show_worker_confirm_modal(ctx, &pal, &hub.workers, french);
            let _ = show_relay_approval_modal(ctx, &pal, &mut hub.approval_prompts, french);
            let _ = show_team_manager_modal(ctx, &pal, &mut ui_state, &ws, &hub, french);
        });
        assert!(!out.shapes.is_empty());
    }
}
