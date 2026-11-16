#[allow(dead_code)]
#[path = "../src/finder_model.rs"]
mod finder_model;
#[allow(dead_code)]
#[path = "../src/icons.rs"]
mod icons;
#[allow(dead_code)]
#[path = "../src/launcher_panel.rs"]
mod launcher_panel;
#[allow(dead_code)]
#[path = "../src/project_panel.rs"]
mod project_panel;
#[allow(dead_code)]
#[path = "../src/shortcuts.rs"]
mod shortcuts;
#[allow(dead_code)]
#[path = "../src/sidebar_model.rs"]
mod sidebar_model;
#[allow(dead_code)]
#[path = "../src/theme.rs"]
mod theme;
#[allow(dead_code)]
#[path = "../src/theme_palettes.rs"]
mod theme_palettes;

#[allow(dead_code)]
mod workspace {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum BranchPrefixMode {
        GitUsername,
        Custom,
        None,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct GeneralPreferences {
        pub restore_workspaces: bool,
        pub confirm_close_agent: bool,
        pub confirm_quit_running: bool,
        pub workspace_dir: String,
        pub confirm_delete_workspace: bool,
    }

    impl Default for GeneralPreferences {
        fn default() -> Self {
            Self {
                restore_workspaces: true,
                confirm_close_agent: true,
                confirm_quit_running: true,
                workspace_dir: String::new(),
                confirm_delete_workspace: true,
            }
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct GitPreferences {
        pub branch_prefix_mode: BranchPrefixMode,
        pub branch_prefix_custom: String,
    }

    impl Default for GitPreferences {
        fn default() -> Self {
            Self {
                branch_prefix_mode: BranchPrefixMode::Custom,
                branch_prefix_custom: "agent".to_owned(),
            }
        }
    }
}

mod git {
    use std::{path::Path, process::Command};

    pub fn run(root: &Path, args: &[&str], _input: Option<&[u8]>) -> Result<String, String> {
        let mut cmd = Command::new("git");
        cmd.current_dir(root)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_TERMINAL_PROMPT", "0")
            .args(args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let out = cmd.output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).into_owned())
        }
    }
}

#[allow(dead_code)]
#[path = "../src/worktree.rs"]
mod worktree;
#[allow(dead_code)]
#[path = "../src/worktree_cleanup_panel.rs"]
mod worktree_cleanup_panel;

use std::{collections::HashMap, fs, path::Path, process::Command};

use eframe::egui::{self, Pos2, Rect, Shape};
use finder_model::{LaunchMenuAction, LaunchMenuItem, LaunchPlacement, LauncherLang};
use launcher_panel::{LauncherEntry, LauncherInput, LauncherPanel};
use project_panel::{
    ProjectAction, ProjectEntry, ProjectPanel, ProjectPanelInput, ProjectWorktreeView,
    ProjectWorktreesInfo, SessionEntry, SessionKind,
};
use workspace::{BranchPrefixMode, GeneralPreferences, GitPreferences};
use worktree::{CleanupBlocker, CleanupItem, CleanupRisk};
use worktree_cleanup_panel::{WorktreeCleanupAction, WorktreeCleanupModal};

fn collect_shape_texts(shape: &Shape, out: &mut Vec<(String, Rect)>) {
    match shape {
        Shape::Text(t) => {
            out.push((
                t.galley.job.text.clone(),
                Rect::from_min_size(t.pos, t.galley.size()),
            ));
        }
        Shape::Vec(list) => {
            for inner in list {
                collect_shape_texts(inner, out);
            }
        }
        _ => {}
    }
}

fn painted_texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, Rect)> {
    let mut out = Vec::new();
    for cs in shapes {
        collect_shape_texts(&cs.shape, &mut out);
    }
    out
}

fn has_text_containing(shapes: &[egui::epaint::ClippedShape], needle: &str) -> bool {
    painted_texts(shapes)
        .iter()
        .any(|(text, _)| text.contains(needle))
}

fn find_text_center(shapes: &[egui::epaint::ClippedShape], needle: &str) -> Option<Pos2> {
    painted_texts(shapes)
        .into_iter()
        .find(|(text, _)| text == needle || text.contains(needle))
        .map(|(_, rect)| rect.center())
}

fn click_events(pos: Pos2) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}

fn init_real_git_repo(repo_dir: &Path) {
    git::run(repo_dir, &["init", "-b", "main"], None).unwrap();
    git::run(repo_dir, &["config", "user.name", "Alice Dev"], None).unwrap();
    git::run(
        repo_dir,
        &["config", "user.email", "alice@example.test"],
        None,
    )
    .unwrap();
    fs::write(
        repo_dir.join(".gitignore"),
        ".env\n.env.*\n!.env.example\nnode_modules/\n",
    )
    .unwrap();
    fs::write(repo_dir.join(".env.example"), "PUBLIC_EXAMPLE=1\n").unwrap();
    fs::write(repo_dir.join("README.md"), "# Worktree Parity Test\n").unwrap();
    git::run(
        repo_dir,
        &["add", ".gitignore", ".env.example", "README.md"],
        None,
    )
    .unwrap();
    git::run(repo_dir, &["commit", "-m", "Initial commit"], None).unwrap();
}

#[test]
fn real_git_worktree_lifecycle_env_copy_and_windows_junction_safe_removal() {
    let root = tempfile::tempdir().unwrap();
    let repo_dir = root.path().join("demo-repo");
    fs::create_dir_all(&repo_dir).unwrap();
    init_real_git_repo(&repo_dir);

    // Untracked git-ignored `.env` secrets in primary checkout.
    fs::write(repo_dir.join(".env"), "API_SECRET=top_secret_value\n").unwrap();
    fs::write(repo_dir.join(".env.local"), "LOCAL_ONLY=12345\n").unwrap();

    let summary = worktree::inspect_project(&repo_dir).unwrap();
    assert!(summary.has_commits);
    assert_eq!(summary.branch, "main");
    assert_eq!(summary.default_branch, "main");
    assert_eq!(summary.worktrees.len(), 1);
    assert!(summary.worktrees[0].is_primary);

    let general_prefs = GeneralPreferences::default();
    let git_prefs = GitPreferences {
        branch_prefix_mode: BranchPrefixMode::Custom,
        branch_prefix_custom: "team/ai".to_owned(),
    };

    let created = worktree::create_worktree(
        &repo_dir,
        "Claude Code",
        None,
        true,
        &general_prefs,
        &git_prefs,
        None,
    )
    .unwrap();
    assert_eq!(created.branch, "team/ai/claude-code");
    assert_eq!(created.copied_env_files, 2);
    assert_eq!(
        fs::read_to_string(created.path.join(".env")).unwrap(),
        "API_SECRET=top_secret_value\n"
    );
    assert_eq!(
        fs::read_to_string(created.path.join(".env.local")).unwrap(),
        "LOCAL_ONLY=12345\n"
    );

    // External directory linked via NTFS junction on Windows: removing the worktree MUST
    // unlink the junction without touching the external target's files!
    let shared_external = root.path().join("shared-node-modules");
    fs::create_dir_all(&shared_external).unwrap();
    let sentinel = shared_external.join("sentinel.txt");
    fs::write(&sentinel, "MUST_SURVIVE_WORKTREE_REMOVAL").unwrap();

    #[cfg(windows)]
    {
        let junction_path = created.path.join("node_modules");
        let status = Command::new("cmd")
            .args([
                "/c",
                "mklink",
                "/J",
                &junction_path.to_string_lossy(),
                &shared_external.to_string_lossy(),
            ])
            .status()
            .unwrap();
        assert!(status.success(), "mklink /J failed");
        assert!(junction_path.join("sentinel.txt").is_file());
    }

    // Non-ignored untracked file refuses non-forced removal without touching the junction!
    let dirty_probe = created.path.join("untracked_wip.txt");
    fs::write(&dirty_probe, "do not lose\n").unwrap();
    assert!(
        worktree::remove_worktree(&repo_dir, &created.path, Some(&created.branch), false).is_err(),
        "Dirty/untracked worktree must refuse non-forced removal"
    );
    #[cfg(windows)]
    assert!(
        created
            .path
            .join("node_modules")
            .join("sentinel.txt")
            .is_file(),
        "Aborted non-forced removal must leave junction untouched"
    );
    fs::remove_file(&dirty_probe).unwrap();

    worktree::remove_worktree(&repo_dir, &created.path, Some(&created.branch), false).unwrap();

    assert!(
        !created.path.exists(),
        "Worktree folder should be deleted after remove_worktree"
    );
    assert_eq!(
        fs::read_to_string(&sentinel).unwrap(),
        "MUST_SURVIVE_WORKTREE_REMOVAL",
        "External NTFS junction target contents MUST NOT be deleted!"
    );
    let after = worktree::inspect_project(&repo_dir).unwrap();
    assert_eq!(after.non_primary_count(), 0);
    assert!(
        git::run(
            &repo_dir,
            &[
                "show-ref",
                "--verify",
                "--quiet",
                "refs/heads/team/ai/claude-code"
            ],
            None
        )
        .is_err(),
        "Merged worktree branch should be cleaned up"
    );
}

#[test]
fn scan_cleanup_classifies_safe_risk_and_locked_blocker() {
    let root = tempfile::tempdir().unwrap();
    let repo_dir = root.path().join("scan-repo");
    fs::create_dir_all(&repo_dir).unwrap();
    init_real_git_repo(&repo_dir);

    let general_prefs = GeneralPreferences::default();
    let git_prefs = GitPreferences::default();

    let safe_wt = worktree::create_worktree(
        &repo_dir,
        "safe-copy",
        None,
        false,
        &general_prefs,
        &git_prefs,
        None,
    )
    .unwrap();
    let dirty_wt = worktree::create_worktree(
        &repo_dir,
        "dirty-copy",
        None,
        false,
        &general_prefs,
        &git_prefs,
        None,
    )
    .unwrap();
    fs::write(dirty_wt.path.join("uncommitted.txt"), "wip\n").unwrap();

    let locked_wt = worktree::create_worktree(
        &repo_dir,
        "locked-copy",
        None,
        false,
        &general_prefs,
        &git_prefs,
        None,
    )
    .unwrap();
    git::run(
        &repo_dir,
        &["worktree", "lock", &locked_wt.path.to_string_lossy()],
        None,
    )
    .unwrap();

    let mut open_panes = HashMap::new();
    open_panes.insert(
        sidebar_model::norm_sidebar_path(&dirty_wt.path.to_string_lossy()),
        2usize,
    );

    let (default_branch, items) = worktree::scan_cleanup(&repo_dir, &open_panes).unwrap();
    assert_eq!(default_branch, "main");
    assert_eq!(items.len(), 3);

    let safe_item = items
        .iter()
        .find(|i| i.branch == safe_wt.branch)
        .expect("safe item");
    assert!(safe_item.is_safe());

    let dirty_item = items
        .iter()
        .find(|i| i.branch == dirty_wt.branch)
        .expect("dirty item");
    assert!(!dirty_item.is_safe());
    assert!(dirty_item.risks.contains(&CleanupRisk::Dirty(1)));
    assert!(dirty_item.risks.contains(&CleanupRisk::OpenPanes(2)));

    let locked_item = items
        .iter()
        .find(|i| i.branch == locked_wt.branch)
        .expect("locked item");
    assert_eq!(locked_item.blocker, Some(CleanupBlocker::Locked));
    assert!(!locked_item.is_selectable());
}

#[test]
fn cleanup_modal_renders_en_fr_preselects_safe_and_confirms_removal() {
    let ctx = egui::Context::default();
    theme::apply(&ctx, true, None);

    let safe_path = std::env::temp_dir().join("wt-safe");
    let dirty_path = std::env::temp_dir().join("wt-dirty");
    let locked_path = std::env::temp_dir().join("wt-locked");

    let mut modal = WorktreeCleanupModal::new_scanning(1, "Tessel");
    modal.set_scan_results(
        "main".to_owned(),
        vec![
            CleanupItem {
                path: safe_path.clone(),
                branch: "agent/safe".to_owned(),
                head: "1111111".to_owned(),
                last_subject: "Merged feature".to_owned(),
                ahead: 0,
                behind: 0,
                merged: true,
                blocker: None,
                risks: Vec::new(),
            },
            CleanupItem {
                path: dirty_path,
                branch: "agent/dirty".to_owned(),
                head: "2222222".to_owned(),
                last_subject: "WIP".to_owned(),
                ahead: 1,
                behind: 0,
                merged: false,
                blocker: None,
                risks: vec![CleanupRisk::Dirty(2), CleanupRisk::Unmerged(1)],
            },
            CleanupItem {
                path: locked_path,
                branch: "agent/locked".to_owned(),
                head: "3333333".to_owned(),
                last_subject: "Pinned".to_owned(),
                ahead: 0,
                behind: 0,
                merged: true,
                blocker: Some(CleanupBlocker::Locked),
                risks: Vec::new(),
            },
        ],
    );

    assert_eq!(
        modal.selected_targets(),
        vec![(safe_path.clone(), "agent/safe".to_owned(), false)]
    );

    let screen = Rect::from_min_size(Pos2::ZERO, egui::vec2(1024.0, 768.0));

    // Warm-up sizing pass for egui Area, then render French to verify localized labels.
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        },
        |ctx| {
            let _ = modal.ui(ctx, false, true);
        },
    );
    let fr_out = ctx.run(
        egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        },
        |ctx| {
            let _ = modal.ui(ctx, false, true);
        },
    );
    assert!(has_text_containing(
        &fr_out.shapes,
        "Nettoyer les worktrees"
    ));
    assert!(has_text_containing(&fr_out.shapes, "Sans risque"));
    assert!(has_text_containing(&fr_out.shapes, "Verrouillé dans Git"));

    // Render English and click "Remove selected (1)" -> "Confirm delete".
    let en_out = ctx.run(
        egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        },
        |ctx| {
            let _ = modal.ui(ctx, true, true);
        },
    );
    assert!(has_text_containing(&en_out.shapes, "Clean up worktrees"));
    assert!(has_text_containing(&en_out.shapes, "Safe to remove"));
    let remove_pos =
        find_text_center(&en_out.shapes, "Remove selected (1)").expect("Remove selected button");

    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(screen),
            events: click_events(remove_pos),
            ..Default::default()
        },
        |ctx| {
            let _ = modal.ui(ctx, true, true);
        },
    );
    assert!(modal.confirming);

    let confirm_out = ctx.run(
        egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        },
        |ctx| {
            let _ = modal.ui(ctx, true, true);
        },
    );
    let confirm_pos =
        find_text_center(&confirm_out.shapes, "Confirm delete").expect("Confirm delete button");

    let mut emitted = None;
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(screen),
            events: click_events(confirm_pos),
            ..Default::default()
        },
        |ctx| {
            emitted = modal.ui(ctx, true, true);
        },
    );
    assert_eq!(
        emitted,
        Some(WorktreeCleanupAction::RemoveSelected(vec![(
            safe_path,
            "agent/safe".to_owned(),
            false
        )]))
    );
}

#[test]
fn launcher_worktree_toggle_emits_in_worktree_true_and_disables_with_reason() {
    let ctx = egui::Context::default();
    let screen = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 640.0));
    let entries = vec![LauncherEntry {
        item: LaunchMenuItem::Agent {
            id: "claude".to_owned(),
            name: "Claude Code".to_owned(),
        },
        available: true,
        reason: None,
    }];
    let placements = vec![LaunchPlacement::Auto, LaunchPlacement::Right];

    let mut panel = LauncherPanel::new();
    panel.set_worktree_availability(true, None);

    let first = ctx.run(
        egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = panel.show(
                    ui,
                    LauncherInput {
                        entries: &entries,
                        placements: &placements,
                        target_title: None,
                        lang: LauncherLang::En,
                    },
                );
            });
        },
    );
    let toggle_pos = find_text_center(&first.shapes, "Separate copy on its own branch")
        .expect("worktree toggle label");

    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(screen),
            events: click_events(toggle_pos),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = panel.show(
                    ui,
                    LauncherInput {
                        entries: &entries,
                        placements: &placements,
                        target_title: None,
                        lang: LauncherLang::En,
                    },
                );
            });
        },
    );
    assert!(panel.in_worktree());

    let mut launched = None;
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(screen),
            events: vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                launched = panel.show(
                    ui,
                    LauncherInput {
                        entries: &entries,
                        placements: &placements,
                        target_title: None,
                        lang: LauncherLang::En,
                    },
                );
            });
        },
    );
    assert_eq!(
        launched,
        Some(LaunchMenuAction::Launch {
            item: LaunchMenuItem::Agent {
                id: "claude".to_owned(),
                name: "Claude Code".to_owned(),
            },
            placement: LaunchPlacement::Auto,
            in_worktree: true,
        })
    );

    // Disabled state shows reason and clears `in_worktree`.
    panel.set_worktree_availability(false, Some("Créez un premier commit.".to_owned()));
    assert!(!panel.in_worktree());
    let disabled_fr = ctx.run(
        egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = panel.show(
                    ui,
                    LauncherInput {
                        entries: &entries,
                        placements: &placements,
                        target_title: None,
                        lang: LauncherLang::Fr,
                    },
                );
            });
        },
    );
    assert!(has_text_containing(
        &disabled_fr.shapes,
        "Copie séparée sur sa propre branche"
    ));
    assert!(has_text_containing(
        &disabled_fr.shapes,
        "Créez un premier commit."
    ));
}

#[test]
fn project_panel_renders_secondary_worktree_cards_and_hidden_inbox_actions() {
    let ctx = egui::Context::default();
    let screen = Rect::from_min_size(Pos2::ZERO, egui::vec2(360.0, 640.0));

    let projects = vec![ProjectEntry {
        id: 1,
        name: "Tessel".to_owned(),
        path: "C:\\repo\\tessel".to_owned(),
        sessions: vec![
            SessionEntry {
                id: 10,
                title: "Main Shell".to_owned(),
                kind: SessionKind::Shell,
                sleeping: false,
            },
            SessionEntry {
                id: 20,
                title: "Ada".to_owned(),
                kind: SessionKind::Agent,
                sleeping: false,
            },
        ],
    }];

    let mut panel = ProjectPanel::new();
    let mut sess_wt = HashMap::new();
    sess_wt.insert(
        20,
        (
            "C:\\repo\\tessel.worktrees\\ada".to_owned(),
            "agent/ada".to_owned(),
        ),
    );
    panel.set_session_worktrees(sess_wt);

    let mut proj_wt = HashMap::new();
    proj_wt.insert(
        1,
        ProjectWorktreesInfo {
            eligible: true,
            has_secondary: true,
            visible_copies: vec![ProjectWorktreeView {
                path: "C:\\repo\\tessel.worktrees\\ada".to_owned(),
                branch: "agent/ada".to_owned(),
                is_external: false,
                locked: false,
            }],
            hidden_inbox: vec![(
                "C:\\repo\\tessel.worktrees\\ext".to_owned(),
                "feat/external".to_owned(),
            )],
            inbox_dismissed: false,
        },
    );
    panel.set_project_worktrees(proj_wt);

    let out = ctx.run(
        egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = panel.show(
                    ui,
                    ProjectPanelInput {
                        projects: &projects,
                        global_sessions: &[],
                        active_project: Some(1),
                        active_session: Some(20),
                        french: false,
                    },
                );
            });
        },
    );

    assert!(has_text_containing(&out.shapes, "agent/ada"));
    assert!(has_text_containing(&out.shapes, "Ada"));
    assert!(has_text_containing(
        &out.shapes,
        "Hiding 1 discovered worktree"
    ));
    assert!(has_text_containing(&out.shapes, "feat/external"));

    let show_pos = find_text_center(&out.shapes, "Show").expect("Show button");
    let mut emitted = None;
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(screen),
            events: click_events(show_pos),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                emitted = panel.show(
                    ui,
                    ProjectPanelInput {
                        projects: &projects,
                        global_sessions: &[],
                        active_project: Some(1),
                        active_session: Some(20),
                        french: false,
                    },
                );
            });
        },
    );
    assert_eq!(
        emitted,
        Some(ProjectAction::ShowHiddenWorktree {
            project: 1,
            path: "C:\\repo\\tessel.worktrees\\ext".to_owned(),
        })
    );
}

#[test]
fn worktree_origin_tracks_selected_ref_current_branch_and_detached_commit() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("source");
    fs::create_dir(&root).unwrap();
    init_real_git_repo(&root);
    git::run(&root, &["switch", "-c", "refs/heads/feature/source"], None).unwrap();
    let general = GeneralPreferences::default();
    let prefs = GitPreferences::default();
    let current =
        worktree::create_worktree(&root, "current", None, false, &general, &prefs, None).unwrap();
    assert_eq!(current.base_branch, "refs/heads/feature/source");
    let head = worktree::create_worktree(
        &root,
        "named-head",
        Some("HEAD"),
        false,
        &general,
        &prefs,
        None,
    )
    .unwrap();
    assert_eq!(head.base_branch, "refs/heads/feature/source");
    assert_eq!(head.base_commit, current.base_commit);

    assert_eq!(
        git::run(&current.path, &["rev-parse", "HEAD"], None)
            .unwrap()
            .trim(),
        current.base_commit
    );
    let explicit = worktree::create_worktree(
        &root,
        "explicit",
        Some(" main "),
        false,
        &general,
        &prefs,
        None,
    )
    .unwrap();
    assert_eq!(explicit.base_branch, "main");
    assert_eq!(
        git::run(&explicit.path, &["rev-parse", "HEAD"], None)
            .unwrap()
            .trim(),
        explicit.base_commit
    );
    git::run(&root, &["checkout", "--detach"], None).unwrap();
    let detached =
        worktree::create_worktree(&root, "detached", None, false, &general, &prefs, None).unwrap();
    assert_eq!(detached.base_branch, detached.base_commit);
    assert_eq!(
        git::run(&detached.path, &["rev-parse", "HEAD"], None)
            .unwrap()
            .trim(),
        detached.base_commit
    );
}
