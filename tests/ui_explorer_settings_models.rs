#[path = "../src/explorer_scm_model.rs"]
mod explorer_scm_model;
#[path = "../src/settings_model.rs"]
mod settings_model;

use std::collections::{HashMap, HashSet};

use explorer_scm_model::{
    ActiveIndentGuide, EXPLORER_BASE_INDENT, EXPLORER_INDENT, EXPLORER_ROW_HEIGHT,
    EXPLORER_TWISTIE, ExplorerBuildOpts, ExplorerEntry, GitStatusLetter, MAX_COMPACT_CHAIN,
    MAX_STATUS_GAP_MS, MAX_STICKY_FOLDERS, SLOW_STATUS_MS, ScmLang, ScmRefreshCoalescer,
    ScmRefreshDecision, ScmSectionKind, active_guide, background_gap_ms, build_explorer_rows,
    change_count, discard_entry_copy, explorer_status_of, folder_status, folders_to_reload,
    guide_x, ignored_set, in_git_dir, in_untracked, is_ignored, parent_of_path, row_padding,
    same_entries, same_status_map, scm_root_key, scm_status_title, scm_text, segment_of,
    sole_subfolder, status_matters, sticky_folders, toggled_paths, untracked_folder,
};
use settings_model::{
    ALL_SETTINGS_GROUPS, ALL_SETTINGS_PAGES, MAX_SETTINGS_SEARCH_CHARS, McpSettingsTab,
    SETTINGS_PAGE_STORAGE_KEY, SettingsGroupId, SettingsLang, SettingsPageId, SettingsSearchRow,
    SettingsShellState, TEXT_ALIAS_ANCHOR, is_valid_settings_anchor,
    resolve_initial_settings_target, resolve_page_id, search_count_label, search_settings,
};

fn dir_entry(name: &str, path: &str) -> ExplorerEntry {
    ExplorerEntry {
        name: name.to_string(),
        path: path.to_string(),
        dir: true,
        link: false,
    }
}

fn file_entry(name: &str, path: &str) -> ExplorerEntry {
    ExplorerEntry {
        name: name.to_string(),
        path: path.to_string(),
        dir: false,
        link: false,
    }
}

#[test]
fn explorer_compact_folders_guides_and_sticky_scroll_match_vscode_reference() {
    assert_eq!(EXPLORER_ROW_HEIGHT, 22.0);
    assert_eq!(EXPLORER_INDENT, 8.0);
    assert_eq!(EXPLORER_BASE_INDENT, 8.0);
    assert_eq!(EXPLORER_TWISTIE, 16.0);
    assert_eq!(MAX_COMPACT_CHAIN, 256);
    assert_eq!(MAX_STICKY_FOLDERS, 7);
    assert_eq!(row_padding(0), 8.0);
    assert_eq!(row_padding(2), 24.0);
    assert_eq!(guide_x(1), 24.0);

    let mut nodes: HashMap<String, Vec<ExplorerEntry>> = HashMap::new();
    nodes.insert(
        "C:/repo".to_string(),
        vec![
            dir_entry("src", "C:/repo/src"),
            file_entry("Cargo.toml", "C:/repo/Cargo.toml"),
        ],
    );
    nodes.insert(
        "C:/repo/src".to_string(),
        vec![dir_entry("ui", "C:/repo/src/ui")],
    );
    nodes.insert(
        "C:/repo/src/ui".to_string(),
        vec![file_entry("app.rs", "C:/repo/src/ui/app.rs")],
    );

    let mut open = HashSet::new();
    open.insert("C:/repo/src".to_string());
    open.insert("C:/repo/src/ui".to_string());

    let tree = build_explorer_rows(
        "C:/repo",
        &nodes,
        &open,
        &ExplorerBuildOpts {
            compact: true,
            hidden_paths: None,
            no_join_paths: None,
        },
    );
    // `src` and `ui` share row 0 as a compact folder chain!
    assert_eq!(tree.rows.len(), 3);
    assert_eq!(tree.rows[0].chain.len(), 2);
    assert_eq!(tree.rows[0].path, "C:/repo/src/ui");
    assert_eq!(tree.rows[0].name, "ui");
    assert!(tree.rows[0].dir);
    assert_eq!(tree.rows[0].depth, 0);
    assert!(!tree.rows[0].entry.link);
    assert_eq!(tree.rows[0].parent, "C:/repo");
    assert!(tree.rows[0].open);
    assert_eq!(tree.index.get("C:/repo/src"), Some(&0));
    assert_eq!(tree.index.get("C:/repo/src/ui"), Some(&0));
    assert_eq!(segment_of(&tree.rows[0], "C:/repo/src"), 0);
    assert_eq!(segment_of(&tree.rows[0], "C:/repo/src/ui"), 1);
    assert_eq!(
        toggled_paths(&tree.rows[0]),
        vec!["C:/repo/src".to_string(), "C:/repo/src/ui".to_string()]
    );
    assert_eq!(
        sole_subfolder(&nodes, "C:/repo/src", None).unwrap().path,
        "C:/repo/src/ui"
    );

    // Active indent guide under `C:/repo/src/ui` covers its child `app.rs` (row 1..2).
    assert_eq!(
        active_guide(&tree.rows, &tree.index, Some("C:/repo/src/ui/app.rs")),
        Some(ActiveIndentGuide {
            depth: 0,
            from: 1,
            to: 2,
        })
    );

    // Sticky scroll keeps open parent folders pinned as rows scroll past top.
    let depths = vec![0, 1, 2, 2, 2, 1, 0];
    let sticky = sticky_folders(&depths, 44.0, EXPLORER_ROW_HEIGHT, MAX_STICKY_FOLDERS);
    assert_eq!(sticky.len(), 2);
    assert_eq!(sticky[0].at, 0);
    assert_eq!(sticky[0].top, 0.0);
    assert_eq!(sticky[1].at, 1);
    assert_eq!(sticky[1].top, EXPLORER_ROW_HEIGHT);
}

#[test]
fn explorer_git_status_and_watch_reload_rules_match_reference() {
    assert_eq!(GitStatusLetter::Deleted.as_char(), 'D');
    assert_eq!(GitStatusLetter::Added.as_char(), 'A');
    assert_eq!(GitStatusLetter::Renamed.as_char(), 'R');
    assert_eq!(GitStatusLetter::Copied.priority(), 1);

    let mut map = HashMap::new();
    map.insert("c:/repo/src/a.rs".to_string(), GitStatusLetter::Modified);
    map.insert("c:/repo/src/b.rs".to_string(), GitStatusLetter::Deleted);
    map.insert("c:/repo/src/c.rs".to_string(), GitStatusLetter::Added);
    map.insert("c:/repo/src/d.rs".to_string(), GitStatusLetter::Renamed);
    map.insert("c:/repo/newdir".to_string(), GitStatusLetter::Untracked);
    map.insert("c:/repo/newdir/cache".to_string(), GitStatusLetter::Ignored);
    map.insert("c:/repo/target".to_string(), GitStatusLetter::Ignored);

    // Nearest ancestor rule: file in `newdir` is Untracked, but file in `newdir/cache` is NOT!
    assert!(in_untracked(&map, "c:/repo/newdir/sub/f.rs", "c:/repo"));
    assert!(!in_untracked(
        &map,
        "c:/repo/newdir/cache/out.tmp",
        "c:/repo"
    ));
    assert_eq!(
        explorer_status_of(&map, "c:/repo/newdir/sub/f.rs", "c:/repo"),
        Some(GitStatusLetter::Untracked)
    );
    assert_eq!(
        untracked_folder(&map, "c:/repo/newdir/sub", "c:/repo"),
        Some(GitStatusLetter::Untracked)
    );
    assert_eq!(explorer_status_of(&map, "c:/repo/target", "c:/repo"), None);

    // Folder status takes highest priority (`Deleted` > `Modified`).
    let fstat = folder_status(&map, "c:/repo");
    assert_eq!(fstat.get("c:/repo/src"), Some(&GitStatusLetter::Deleted));

    let ign = ignored_set(&map);
    assert!(is_ignored(&ign, "c:/repo/target/debug/app.exe", "c:/repo"));
    assert!(!is_ignored(&ign, "c:/repo/src/a.rs", "c:/repo"));
    assert_eq!(parent_of_path("c:/repo/src/a.rs"), "c:/repo/src");
    assert!(in_git_dir("C:\\repo\\.git\\INDEX"));

    // Watch filter: ignored-only edits do NOT trigger git status; `.git` edits ALWAYS do.
    assert!(!status_matters(
        "C:/repo",
        Some(&["C:/repo/target/debug/app.exe".to_string()]),
        &ign
    ));
    assert!(status_matters(
        "C:/repo",
        Some(&["C:/repo/.git/HEAD".to_string()]),
        &ign
    ));

    let mut nodes: HashMap<String, Vec<ExplorerEntry>> = HashMap::new();
    let e1 = file_entry("a.rs", "C:/repo/src/a.rs");
    nodes.insert("C:/repo/src".to_string(), vec![e1.clone()]);
    // Modifying an already-listed local file (`renamed = Some(&[])`) needs NO folder reload!
    assert!(
        folders_to_reload(&nodes, &["C:/repo/src/a.rs".to_string()], false, Some(&[])).is_empty()
    );
    // An unlisted new file triggers a reload of `C:/repo/src`.
    assert_eq!(
        folders_to_reload(
            &nodes,
            &["C:/repo/src/new.rs".to_string()],
            false,
            Some(&[])
        ),
        vec!["C:/repo/src".to_string()]
    );
    assert!(same_entries(
        std::slice::from_ref(&e1),
        std::slice::from_ref(&e1)
    ));
    assert!(same_status_map(&map, &map));
}

#[test]
fn scm_slow_status_coalescer_and_labels_match_reference() {
    assert_eq!(SLOW_STATUS_MS, 3_000);
    assert_eq!(MAX_STATUS_GAP_MS, 300_000);
    assert_eq!(scm_root_key("C:/Repo//Sub/"), "c:\\repo\\sub");
    assert_eq!(background_gap_ms(2_999, 10_000, 10_500), 0);
    assert_eq!(background_gap_ms(4_000, 10_000, 12_000), 10_000); // 10_000 + 12_000 - 12_000

    let mut scm = ScmRefreshCoalescer::new();
    assert_eq!(
        scm.request_refresh("C:/repo", true, 1_000),
        ScmRefreshDecision::StartNow {
            seq: 1,
            background: true,
        }
    );
    // Concurrent background + foreground requests coalesce into one foreground follow-up.
    assert_eq!(
        scm.request_refresh("C:/repo", true, 1_500),
        ScmRefreshDecision::CoalescedBehindRunning
    );
    assert_eq!(
        scm.request_refresh("C:/repo", false, 2_000),
        ScmRefreshDecision::CoalescedBehindRunning
    );
    // Slow 4,000ms status finishes at 5,000ms -> foreground follow-up starts immediately!
    let (ok, next) = scm.finish_refresh("C:/repo", 1, 5_000);
    assert!(ok);
    assert_eq!(
        next,
        Some(ScmRefreshDecision::StartNow {
            seq: 2,
            background: false,
        })
    );
    let (_, none) = scm.finish_refresh("C:/repo", 2, 9_500); // took 4,500ms -> 13,500ms gap
    assert_eq!(none, None);
    // Next background poll at 10,000ms is delayed by 13,000ms!
    assert_eq!(
        scm.request_refresh("C:/repo", true, 10_000),
        ScmRefreshDecision::DelayBackground { wait_ms: 13_000 }
    );
    scm.forget("C:/repo");

    assert_eq!(change_count(false, 5, false, None), 0);
    assert_eq!(change_count(true, 100, true, Some(450)), 450);

    assert_eq!(ScmLang::default(), ScmLang::En);
    assert_eq!(scm_text("Stage All", ScmLang::En), "Stage All");
    assert_eq!(scm_text("Stage All", ScmLang::Fr), "Tout indexer");
    assert_eq!(scm_text("Push 3 commits", ScmLang::Fr), "Pousser 3 commits");
    assert_eq!(
        scm_text("Sync (↓2 ↑5)", ScmLang::Fr),
        "Synchroniser (↓2 ↑5)"
    );
    assert_eq!(scm_text("Push in progress…", ScmLang::Fr), "Push en cours…");
    assert_eq!(scm_status_title("untracked", ScmLang::En), "Untracked");
    assert_eq!(scm_status_title("untracked", ScmLang::Fr), "Non suivi");
    assert_eq!(ScmSectionKind::Conflicts.label(ScmLang::En), "Conflicts");
    assert_eq!(
        ScmSectionKind::Staged.label(ScmLang::Fr),
        "Modifications indexées"
    );
    assert_eq!(ScmSectionKind::Unstaged.label(ScmLang::En), "Changes");
    assert_eq!(
        ScmSectionKind::Untracked.label(ScmLang::Fr),
        "Fichiers non suivis"
    );
    let copy = discard_entry_copy("src/new.rs", "untracked", true, true, ScmLang::Fr);
    assert_eq!(copy.title, "Supprimer « new.rs » ?");
    assert!(copy.description.contains("hôte distant"));
    assert_eq!(copy.confirm_label, "Supprimer");

    let restore = discard_entry_copy("src/old.rs", "deleted", false, false, ScmLang::En);
    assert_eq!(restore.confirm_label, "Restore");
}

#[test]
fn settings_groups_pages_aliases_and_multiword_search_match_reference() {
    assert_eq!(SETTINGS_PAGE_STORAGE_KEY, "tessel.settingsPage");
    assert_eq!(TEXT_ALIAS_ANCHOR, "set-terminal-font");
    assert_eq!(MAX_SETTINGS_SEARCH_CHARS, 256);
    assert_eq!(ALL_SETTINGS_GROUPS.len(), 5);
    assert_eq!(ALL_SETTINGS_PAGES.len(), 22);

    let total_grouped: usize = ALL_SETTINGS_GROUPS.iter().map(|g| g.pages().len()).sum();
    assert_eq!(total_grouped, 22);
    for &p in &ALL_SETTINGS_PAGES {
        assert!(!p.as_str().is_empty());
        assert_eq!(SettingsPageId::from_id(p.as_str()), Some(p));
        assert!(p.group().pages().contains(&p));
        assert!(!p.title(SettingsLang::Fr).is_empty());
        assert!(!p.description(SettingsLang::Fr).is_empty());
    }
    assert_eq!(SettingsLang::default(), SettingsLang::En);
    assert_eq!(SettingsPageId::default(), SettingsPageId::General);
    assert_eq!(SettingsGroupId::Ai.as_str(), "ai");
    assert_eq!(SettingsGroupId::Ai.title(SettingsLang::En), "AI");
    assert_eq!(SettingsGroupId::Ai.title(SettingsLang::Fr), "IA");

    // Legacy `text` page alias resolves to `terminal` + `#set-terminal-font`.
    assert_eq!(
        resolve_page_id("text"),
        Some((SettingsPageId::Terminal, Some("set-terminal-font")))
    );
    assert!(is_valid_settings_anchor("set-terminal-font"));
    assert!(!is_valid_settings_anchor("bad selector!"));

    let init = resolve_initial_settings_target(Some("text"), None, Some("appearance"));
    assert_eq!(init.page, SettingsPageId::Terminal);
    assert_eq!(init.anchor.as_deref(), Some("set-terminal-font"));

    let default_shell = SettingsShellState::default();
    assert_eq!(default_shell.page, SettingsPageId::General);

    let mut shell = SettingsShellState::open(None, None, Some("appearance"));
    assert_eq!(shell.page, SettingsPageId::Appearance);
    assert_eq!(shell.anchor, None);
    shell.open_mcp_tab(McpSettingsTab::Connections);
    assert_eq!(shell.page, SettingsPageId::Mcp);
    assert_eq!(shell.mcp_tab, McpSettingsTab::Connections);
    shell.set_query("ssh");
    assert!(shell.is_searching());
    assert!(shell.go("ssh"));
    // Navigating resets query and resets MCP sub-tab back to Installed.
    assert!(!shell.is_searching());
    assert_eq!(shell.mcp_tab, McpSettingsTab::Installed);
    assert!(shell.is_page_shown(SettingsPageId::Ssh, &[]));

    // Search finds pages by French row text, English page title while in French UI, and `Font` alias!
    let rows = vec![
        SettingsSearchRow {
            page: SettingsPageId::Terminal,
            group_anchor: Some("set-terminal-font".to_string()),
            group_title: "Police".to_string(),
            row_id: "term-font-size".to_string(),
            text: "Taille de la police Cascadia Mono".to_string(),
        },
        SettingsSearchRow {
            page: SettingsPageId::Appearance,
            group_anchor: None,
            group_title: "Thème".to_string(),
            row_id: "app-theme".to_string(),
            text: "Classique Dracula Nord".to_string(),
        },
    ];
    assert!(!search_settings("   ", &rows, SettingsLang::En).searching);

    let by_english_title = search_settings("Developer tools", &rows, SettingsLang::Fr);
    assert!(by_english_title.searching);
    assert_eq!(
        by_english_title.matched_pages,
        vec![SettingsPageId::Devtools]
    );
    assert_eq!(by_english_title.result_count, 1);
    assert_eq!(
        search_count_label(by_english_title.result_count, SettingsLang::En),
        "1 result"
    );
    assert_eq!(
        search_count_label(by_english_title.result_count, SettingsLang::Fr),
        "1 résultat"
    );

    let by_font_alias = search_settings("Font", &rows, SettingsLang::Fr);
    assert_eq!(by_font_alias.matched_pages, vec![SettingsPageId::Terminal]);
    assert_eq!(by_font_alias.matched_row_ids, vec!["term-font-size"]);
}
