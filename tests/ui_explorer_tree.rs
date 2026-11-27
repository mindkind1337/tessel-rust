#[allow(dead_code)]
#[path = "../src/explorer_scm_model.rs"]
mod explorer_scm_model;
#[path = "../src/explorer_tree.rs"]
mod explorer_tree;
#[allow(dead_code)]
#[path = "../src/i18n.rs"]
mod i18n;

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use eframe::egui;
use explorer_tree::{
    DirListing, DirectoryLister, ExplorerTree, FsDirectoryLister, TreeAction, TreeContextKind,
    TreeDirEntry, TreeLimits, WIN_FILE_ATTRIBUTE_REPARSE_POINT, has_reparse_point_attr,
    norm_tree_key,
};

fn wait_until_idle(tree: &mut ExplorerTree) {
    let start = Instant::now();
    while tree.is_loading() && start.elapsed() < Duration::from_secs(3) {
        tree.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!tree.is_loading(), "ExplorerTree worker timed out");
}

fn run_frame(
    ctx: &egui::Context,
    tree: &mut ExplorerTree,
    root: Option<&Path>,
    active: Option<&Path>,
    actions_enabled: bool,
    events: Vec<egui::Event>,
) -> (Option<TreeAction>, egui::FullOutput) {
    let mut action = None;
    let out = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(340.0, 520.0),
            )),
            focused: true,
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                action = tree.show(ui, root, active, actions_enabled);
            });
        },
    );
    (action, out)
}

fn click_events(pos: egui::Pos2, button: egui::PointerButton) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
        egui::Event::PointerButton {
            pos,
            button,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}

fn find_shape_text_center(shape: &egui::epaint::Shape, needle: &str) -> Option<egui::Pos2> {
    match shape {
        egui::epaint::Shape::Text(t) if t.galley.job.text == needle => {
            Some(egui::Rect::from_min_size(t.pos, t.galley.size()).center())
        }
        egui::epaint::Shape::Vec(list) => list
            .iter()
            .find_map(|inner| find_shape_text_center(inner, needle)),
        _ => None,
    }
}

fn find_text_center(shapes: &[egui::epaint::ClippedShape], needle: &str) -> Option<egui::Pos2> {
    shapes
        .iter()
        .find_map(|cs| find_shape_text_center(&cs.shape, needle))
}

#[test]
fn egui_tree_expands_folders_opens_files_caches_rows_and_gates_context_actions() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    fs::create_dir_all(root.join("src/nested")).expect("mkdir");
    fs::write(root.join("README.md"), "# Fixture\n").expect("write");
    fs::write(root.join("src/main.rs"), "fn main() {}\n").expect("write");
    fs::write(root.join("src/nested/leaf.rs"), "pub fn leaf() {}\n").expect("write");

    // Verify `fixture` constructor + compact-folder mode render deterministically with zero I/O.
    let mut fixture_tree = ExplorerTree::fixture(
        root.to_path_buf(),
        vec![
            (
                root.to_path_buf(),
                vec![TreeDirEntry {
                    name: "src".into(),
                    path: root.join("src"),
                    directory: true,
                }],
            ),
            (
                root.join("src"),
                vec![TreeDirEntry {
                    name: "nested".into(),
                    path: root.join("src/nested"),
                    directory: true,
                }],
            ),
        ],
        vec![root.join("src")],
    );
    fixture_tree.set_compact_folders(true);
    let ctx_fx = egui::Context::default();
    let (fx_act, _) = run_frame(&ctx_fx, &mut fixture_tree, Some(root), None, true, vec![]);
    assert_eq!(fx_act, None);
    assert!(
        fixture_tree
            .rendered_row_rect(&root.join("src/nested"))
            .is_some()
    );
    assert_eq!(fixture_tree.row_rebuild_count(), 1);
    // Steady-state frames MUST NOT rebuild `ExplorerTreeResult`:
    for _ in 0..4 {
        let _ = run_frame(&ctx_fx, &mut fixture_tree, Some(root), None, true, vec![]);
    }
    assert_eq!(fixture_tree.row_rebuild_count(), 1);

    let (_, rendered) = run_frame(&ctx_fx, &mut fixture_tree, Some(root), None, true, vec![]);
    let parent_label = rendered
        .shapes
        .iter()
        .find_map(|s| find_shape_text_center(&s.shape, "src"))
        .expect("outer compact folder label");
    let (action, _) = run_frame(
        &ctx_fx,
        &mut fixture_tree,
        Some(root),
        None,
        true,
        click_events(parent_label, egui::PointerButton::Primary),
    );
    assert_eq!(action, None);
    let snapshot = fixture_tree.gpui_rows_snapshot(Some(root), None);
    assert_eq!(snapshot.rows[0].selected_segment, Some(0));
    assert_eq!(snapshot.new_target_dir, root.join("src"));

    for (label, path) in [
        ("nested", root.join("src/nested")),
        ("src", root.join("src")),
    ] {
        let (_, painted) = run_frame(&ctx_fx, &mut fixture_tree, Some(root), None, true, vec![]);
        let point = find_text_center(&painted.shapes, label).unwrap();
        run_frame(
            &ctx_fx,
            &mut fixture_tree,
            Some(root),
            None,
            true,
            click_events(point, egui::PointerButton::Secondary),
        );
        let (_, menu) = run_frame(&ctx_fx, &mut fixture_tree, Some(root), None, true, vec![]);
        let rename = find_text_center(&menu.shapes, "Rename…").unwrap();
        let (action, _) = run_frame(
            &ctx_fx,
            &mut fixture_tree,
            Some(root),
            None,
            true,
            click_events(rename, egui::PointerButton::Primary),
        );
        assert_eq!(
            action,
            Some(TreeAction::Context {
                kind: TreeContextKind::Rename,
                root: root.into(),
                path,
                name: label.into()
            })
        );
    }

    // Real async `ExplorerTree` driven through `egui::Context` frames:
    let mut tree = ExplorerTree::default();
    let ctx = egui::Context::default();

    // No project selected yet:
    let (act0, _) = run_frame(&ctx, &mut tree, None, None, true, vec![]);
    assert_eq!(act0, None);
    assert!(!tree.is_loading());

    // Select project root -> async load starts off the UI thread, then completes via `poll()`.
    let _ = run_frame(&ctx, &mut tree, Some(root), None, false, vec![]);
    wait_until_idle(&mut tree);
    assert!(
        tree.error().is_empty(),
        "unexpected error: {}",
        tree.error()
    );
    let _ = run_frame(
        &ctx,
        &mut tree,
        Some(root),
        Some(&root.join("README.md")),
        false,
        vec![],
    );
    let rebuilds_after_initial = tree.row_rebuild_count();
    for _ in 0..5 {
        let _ = run_frame(
            &ctx,
            &mut tree,
            Some(root),
            Some(&root.join("README.md")),
            false,
            vec![],
        );
    }
    assert_eq!(tree.row_rebuild_count(), rebuilds_after_initial);

    let src_rect = tree
        .rendered_row_rect(&root.join("src"))
        .expect("src folder row rendered");
    let readme_rect = tree
        .rendered_row_rect(&root.join("README.md"))
        .expect("README.md row rendered");

    // Clicking a file opens it even when `actions_enabled == false` (dirty buffer in another tab)!
    let (open_readme, _) = run_frame(
        &ctx,
        &mut tree,
        Some(root),
        None,
        false,
        click_events(readme_rect.center(), egui::PointerButton::Primary),
    );
    assert_eq!(
        open_readme,
        Some(TreeAction::OpenFile(root.join("README.md")))
    );

    // Clicking `src` expands it asynchronously and loads `src/main.rs`!
    let (expand_act, _) = run_frame(
        &ctx,
        &mut tree,
        Some(root),
        None,
        true,
        click_events(src_rect.center(), egui::PointerButton::Primary),
    );
    assert_eq!(expand_act, None);
    assert!(tree.is_expanded(&root.join("src")));
    wait_until_idle(&mut tree);

    let _ = run_frame(&ctx, &mut tree, Some(root), None, true, vec![]);
    let main_rect = tree
        .rendered_row_rect(&root.join("src/main.rs"))
        .expect("src/main.rs rendered after expanding src");
    let (open_main, _) = run_frame(
        &ctx,
        &mut tree,
        Some(root),
        None,
        true,
        click_events(main_rect.center(), egui::PointerButton::Primary),
    );
    assert_eq!(
        open_main,
        Some(TreeAction::OpenFile(root.join("src/main.rs")))
    );

    // Check `actions_enabled == false` context menu on an isolated `egui::Context` so popup state stays independent:
    let ctx_disabled = egui::Context::default();
    let _ = run_frame(&ctx_disabled, &mut tree, Some(root), None, false, vec![]);
    let disabled_src_rect = tree
        .rendered_row_rect(&root.join("src"))
        .expect("src row rect on ctx_disabled");
    let (_, disabled_menu_out1) = run_frame(
        &ctx_disabled,
        &mut tree,
        Some(root),
        None,
        false,
        click_events(disabled_src_rect.center(), egui::PointerButton::Secondary),
    );
    let (_, disabled_menu_out2) =
        run_frame(&ctx_disabled, &mut tree, Some(root), None, false, vec![]);
    let saw_disabled_notice = find_text_center(
        &disabled_menu_out1.shapes,
        "Save pending edits before changing files.",
    )
    .or_else(|| {
        find_text_center(
            &disabled_menu_out2.shapes,
            "Save pending edits before changing files.",
        )
    });
    assert!(saw_disabled_notice.is_some());
    assert!(find_text_center(&disabled_menu_out2.shapes, "Rename…").is_none());

    // On the main `ctx`, right-clicking `src` when `actions_enabled == true` and clicking `"Rename…"` emits `TreeAction::Context`!
    let _ = run_frame(&ctx, &mut tree, Some(root), None, true, vec![]);
    let enabled_src_rect = tree
        .rendered_row_rect(&root.join("src"))
        .expect("src row rect on main ctx");
    let (_, menu_out1) = run_frame(
        &ctx,
        &mut tree,
        Some(root),
        None,
        true,
        click_events(enabled_src_rect.center(), egui::PointerButton::Secondary),
    );
    let (_, menu_out2) = run_frame(&ctx, &mut tree, Some(root), None, true, vec![]);
    let rename_pos = find_text_center(&menu_out2.shapes, "Rename…")
        .or_else(|| find_text_center(&menu_out1.shapes, "Rename…"))
        .expect("Rename… button rendered in context menu");
    let (ctx_act, _) = run_frame(
        &ctx,
        &mut tree,
        Some(root),
        None,
        true,
        click_events(rename_pos, egui::PointerButton::Primary),
    );
    assert_eq!(
        ctx_act,
        Some(TreeAction::Context {
            kind: TreeContextKind::Rename,
            root: root.to_path_buf(),
            path: root.join("src"),
            name: "src".into(),
        })
    );

    // Select `src` row first (left-click toggles `src` closed while setting `selected_key = src`),
    // then click header `"New…"` -> `"Create folder"` targeting the selected `src` directory!
    let _ = run_frame(&ctx, &mut tree, Some(root), None, true, vec![]);
    let src_click_pos = tree
        .rendered_row_rect(&root.join("src"))
        .expect("src row")
        .center();
    let _ = run_frame(
        &ctx,
        &mut tree,
        Some(root),
        None,
        true,
        click_events(src_click_pos, egui::PointerButton::Primary),
    );
    let (_, bar_out) = run_frame(&ctx, &mut tree, Some(root), None, true, vec![]);
    let new_btn_pos = find_text_center(&bar_out.shapes, "New…").expect("New… button rendered");
    let (_, new_menu_out1) = run_frame(
        &ctx,
        &mut tree,
        Some(root),
        None,
        true,
        click_events(new_btn_pos, egui::PointerButton::Primary),
    );
    let (_, new_menu_out2) = run_frame(&ctx, &mut tree, Some(root), None, true, vec![]);
    let create_folder_pos = find_text_center(&new_menu_out2.shapes, "Create folder")
        .or_else(|| find_text_center(&new_menu_out1.shapes, "Create folder"))
        .expect("Create folder button rendered");
    let (create_act, _) = run_frame(
        &ctx,
        &mut tree,
        Some(root),
        None,
        true,
        click_events(create_folder_pos, egui::PointerButton::Primary),
    );
    assert_eq!(
        create_act,
        Some(TreeAction::Context {
            kind: TreeContextKind::CreateFolder,
            root: root.to_path_buf(),
            path: root.join("src"),
            name: String::new(),
        })
    );

    // Ensure all `TreeContextKind` variants & fields remain exercised under `-D warnings`:
    for kind in [
        TreeContextKind::CreateFile,
        TreeContextKind::CreateFolder,
        TreeContextKind::Rename,
        TreeContextKind::Delete,
    ] {
        let sample = TreeAction::Context {
            kind,
            root: root.to_path_buf(),
            path: root.join("src"),
            name: "src".into(),
        };
        if let TreeAction::Context {
            kind: k,
            root: r,
            path: p,
            name: n,
        } = sample
        {
            assert_eq!(k, kind);
            assert_eq!(r, root);
            assert_eq!(p, root.join("src"));
            assert_eq!(n, "src");
        }
    }
}

struct BlockingProjectSwitchLister {
    active_workers: AtomicUsize,
    max_concurrent: AtomicUsize,
    first_entered_tx: Mutex<Option<mpsc::Sender<()>>>,
    release_first_rx: Mutex<Option<mpsc::Receiver<()>>>,
    saw_cancel_on_first: AtomicBool,
}

impl DirectoryLister for BlockingProjectSwitchLister {
    fn list_dir(
        &self,
        root: &Path,
        dir: &Path,
        _limits: &TreeLimits,
        cancel: &AtomicBool,
    ) -> Result<DirListing, String> {
        let cur = self.active_workers.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_concurrent.fetch_max(cur, Ordering::SeqCst);

        let is_first = root.to_string_lossy().contains("proj-a");
        if is_first {
            if let Some(tx) = self.first_entered_tx.lock().expect("lock").take() {
                let _ = tx.send(());
            }
            if let Some(rx) = self.release_first_rx.lock().expect("lock").take() {
                let _ = rx.recv_timeout(Duration::from_secs(3));
            }
            if cancel.load(Ordering::Relaxed) {
                self.saw_cancel_on_first.store(true, Ordering::SeqCst);
            }
            self.active_workers.fetch_sub(1, Ordering::SeqCst);
            return Ok(DirListing {
                entries: vec![TreeDirEntry {
                    name: "stale_from_a.rs".into(),
                    path: dir.join("stale_from_a.rs"),
                    directory: false,
                }],
                truncated: false,
            });
        }

        self.active_workers.fetch_sub(1, Ordering::SeqCst);
        Ok(DirListing {
            entries: vec![TreeDirEntry {
                name: "fresh_in_b.rs".into(),
                path: dir.join("fresh_in_b.rs"),
                directory: false,
            }],
            truncated: false,
        })
    }
}

#[test]
fn set_root_cancels_stale_project_even_when_panel_is_closed_and_keeps_single_worker() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let lister = Arc::new(BlockingProjectSwitchLister {
        active_workers: AtomicUsize::new(0),
        max_concurrent: AtomicUsize::new(0),
        first_entered_tx: Mutex::new(Some(entered_tx)),
        release_first_rx: Mutex::new(Some(release_rx)),
        saw_cancel_on_first: AtomicBool::new(false),
    });

    let mut tree = ExplorerTree::with_lister_and_limits(
        Arc::clone(&lister) as Arc<dyn DirectoryLister>,
        TreeLimits::default(),
    );
    let proj_a = PathBuf::from("C:/synthetic/proj-a");
    let proj_b = PathBuf::from("C:/synthetic/proj-b");

    // Start loading `proj_a` via `set_root` while the SidePanel is closed (no `show()` call!).
    tree.set_root(Some(&proj_a));
    entered_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("worker A entered");
    assert_eq!(lister.active_workers.load(Ordering::SeqCst), 1);

    // Switch immediately to `proj_b` via `set_root` while panel is STILL closed and worker A is blocked!
    tree.set_root(Some(&proj_b));
    // Peak concurrency MUST stay 1 (B waits in `pending_queue` until A's worker slot drains)!
    assert_eq!(lister.max_concurrent.load(Ordering::SeqCst), 1);

    // Release A's worker: A's stale result must be discarded and B must load on the single slot!
    release_tx.send(()).expect("release worker A");
    wait_until_idle(&mut tree);
    assert!(lister.saw_cancel_on_first.load(Ordering::SeqCst));
    assert_eq!(lister.max_concurrent.load(Ordering::SeqCst), 1);

    let ctx = egui::Context::default();
    let _ = run_frame(&ctx, &mut tree, Some(&proj_b), None, true, vec![]);
    assert!(
        tree.rendered_row_rect(&proj_b.join("fresh_in_b.rs"))
            .is_some()
    );
    assert!(
        tree.rendered_row_rect(&proj_a.join("stale_from_a.rs"))
            .is_none()
    );
    assert!(
        tree.rendered_row_rect(&proj_b.join("stale_from_a.rs"))
            .is_none()
    );
}

#[test]
fn expanded_folders_persist_prune_deleted_dirs_on_refresh_and_enforce_all_limits() {
    let dir_a = tempfile::tempdir().expect("dir_a");
    let dir_b = tempfile::tempdir().expect("dir_b");
    let root_a = dir_a.path();
    let root_b = dir_b.path();

    fs::create_dir_all(root_a.join("src")).expect("mkdir");
    fs::create_dir_all(root_a.join("ephemeral")).expect("mkdir ephemeral");
    fs::write(root_a.join("src/lib.rs"), "pub fn v1() {}\n").expect("write");
    fs::write(root_a.join("ephemeral/gone.rs"), "pub fn gone() {}\n").expect("write");
    fs::write(root_b.join("Cargo.toml"), "[package]\n").expect("write");

    let mut tree = ExplorerTree::new();
    let ctx = egui::Context::default();

    // Load A and expand BOTH `src` and `ephemeral`:
    let _ = run_frame(&ctx, &mut tree, Some(root_a), None, true, vec![]);
    wait_until_idle(&mut tree);
    let _ = run_frame(&ctx, &mut tree, Some(root_a), None, true, vec![]);

    for folder in ["src", "ephemeral"] {
        let pos = tree
            .rendered_row_rect(&root_a.join(folder))
            .expect("folder rendered")
            .center();
        let _ = run_frame(
            &ctx,
            &mut tree,
            Some(root_a),
            None,
            true,
            click_events(pos, egui::PointerButton::Primary),
        );
        wait_until_idle(&mut tree);
        let _ = run_frame(&ctx, &mut tree, Some(root_a), None, true, vec![]);
    }
    assert!(tree.is_expanded(&root_a.join("src")));
    assert!(tree.is_expanded(&root_a.join("ephemeral")));
    assert!(tree.rendered_row_rect(&root_a.join("src/lib.rs")).is_some());
    let tracked_with_ephemeral = tree.tracked_path_count();

    // Delete `ephemeral` on disk, add `src/added.rs`, then call `refresh()`:
    // `src` MUST remain expanded with `added.rs`, while deleted `ephemeral` MUST be pruned from
    // `open_dirs` AND `path_by_key`!
    fs::remove_dir_all(root_a.join("ephemeral")).expect("rm ephemeral");
    fs::write(root_a.join("src/added.rs"), "pub fn v2() {}\n").expect("write");
    tree.refresh();
    wait_until_idle(&mut tree);
    let _ = run_frame(&ctx, &mut tree, Some(root_a), None, true, vec![]);
    assert!(tree.is_expanded(&root_a.join("src")));
    assert!(!tree.is_expanded(&root_a.join("ephemeral")));
    assert!(tree.tracked_path_count() < tracked_with_ephemeral);
    assert!(
        tree.rendered_row_rect(&root_a.join("src/added.rs"))
            .is_some()
    );

    // Switch A -> B -> A: `src` in Project A is still expanded automatically, and pruned `ephemeral` stays gone!
    let _ = run_frame(&ctx, &mut tree, Some(root_b), None, true, vec![]);
    wait_until_idle(&mut tree);
    let _ = run_frame(&ctx, &mut tree, Some(root_b), None, true, vec![]);
    assert!(tree.rendered_row_rect(&root_b.join("Cargo.toml")).is_some());
    assert!(
        tree.rendered_row_rect(&root_a.join("src/added.rs"))
            .is_none()
    );

    let _ = run_frame(&ctx, &mut tree, Some(root_a), None, true, vec![]);
    wait_until_idle(&mut tree);
    let _ = run_frame(&ctx, &mut tree, Some(root_a), None, true, vec![]);
    assert!(tree.is_expanded(&root_a.join("src")));
    assert!(!tree.is_expanded(&root_a.join("ephemeral")));
    assert!(
        tree.rendered_row_rect(&root_a.join("src/added.rs"))
            .is_some()
    );

    // Verify strict `TreeLimits` caps (`max_entries_per_dir`, `max_raw_dir_entries`, `dir_list_timeout`, `max_cached_dirs`):
    fs::write(root_a.join("f1.txt"), "1").expect("w");
    fs::write(root_a.join("f2.txt"), "2").expect("w");
    fs::write(root_a.join("f3.txt"), "3").expect("w");
    let mut capped_tree = ExplorerTree::with_lister_and_limits(
        Arc::new(FsDirectoryLister),
        TreeLimits {
            max_entries_per_dir: 2,
            max_raw_dir_entries: 10,
            max_cached_dirs: 2,
            max_total_cached_entries: 3,
            max_depth: 1,
            max_remembered_projects: 2,
            dir_list_timeout: Duration::from_secs(2),
        },
    );
    let _ = run_frame(&ctx, &mut capped_tree, Some(root_a), None, true, vec![]);
    wait_until_idle(&mut capped_tree);
    assert!(capped_tree.is_truncated());
    assert_eq!(capped_tree.cached_dir_count(), 1);
    assert!(capped_tree.total_cached_entries() <= 2);

    // Verify `max_raw_dir_entries` alone caps huge/symlink-heavy directories:
    let mut raw_capped = ExplorerTree::with_lister_and_limits(
        Arc::new(FsDirectoryLister),
        TreeLimits {
            max_raw_dir_entries: 2,
            ..TreeLimits::default()
        },
    );
    raw_capped.set_root(Some(root_a));
    wait_until_idle(&mut raw_capped);
    assert!(raw_capped.is_truncated());
    assert!(raw_capped.total_cached_entries() <= 2);
}

#[test]
fn compact_chain_expansion_strictly_respects_max_cached_dirs_capacity() {
    struct MultiCompactLister;
    impl DirectoryLister for MultiCompactLister {
        fn list_dir(
            &self,
            _root: &Path,
            dir: &Path,
            _limits: &TreeLimits,
            _cancel: &AtomicBool,
        ) -> Result<DirListing, String> {
            let s = dir.to_string_lossy();
            let entries = if s.ends_with("a\\b") || s.ends_with("a/b") {
                vec![TreeDirEntry {
                    name: "c".into(),
                    path: dir.join("c"),
                    directory: true,
                }]
            } else if s.ends_with("a") {
                vec![TreeDirEntry {
                    name: "b".into(),
                    path: dir.join("b"),
                    directory: true,
                }]
            } else if s.ends_with("d1") || s.ends_with("d2") {
                vec![]
            } else {
                vec![
                    TreeDirEntry {
                        name: "a".into(),
                        path: dir.join("a"),
                        directory: true,
                    },
                    TreeDirEntry {
                        name: "d1".into(),
                        path: dir.join("d1"),
                        directory: true,
                    },
                    TreeDirEntry {
                        name: "d2".into(),
                        path: dir.join("d2"),
                        directory: true,
                    },
                ]
            };
            Ok(DirListing {
                entries,
                truncated: false,
            })
        }
    }

    let root = PathBuf::from("C:/synthetic/compact-cap");
    // 1. `max_cached_dirs: 2` -> `root` (1) + `a` (2) fill directory cache without truncation;
    //    clicking compact row `a/b` must refuse expansion cleanly (never leaving `a/b` stuck open
    //    with `"…"` twistie) and mark `truncated = true`.
    let mut dir_capped = ExplorerTree::with_lister_and_limits(
        Arc::new(MultiCompactLister),
        TreeLimits {
            max_cached_dirs: 2,
            ..TreeLimits::default()
        },
    );
    dir_capped.set_compact_folders(true);
    let ctx = egui::Context::default();
    dir_capped.set_root(Some(&root));
    wait_until_idle(&mut dir_capped);
    let _ = run_frame(&ctx, &mut dir_capped, Some(&root), None, true, vec![]);
    assert!(!dir_capped.is_truncated());

    let a_pos = dir_capped
        .rendered_row_rect(&root.join("a"))
        .expect("a row")
        .center();
    let _ = run_frame(
        &ctx,
        &mut dir_capped,
        Some(&root),
        None,
        true,
        click_events(a_pos, egui::PointerButton::Primary),
    );
    wait_until_idle(&mut dir_capped);
    let _ = run_frame(&ctx, &mut dir_capped, Some(&root), None, true, vec![]);
    assert!(!dir_capped.is_truncated());
    let ab_pos = dir_capped
        .rendered_row_rect(&root.join("a/b"))
        .expect("a/b compact row")
        .center();
    let _ = run_frame(
        &ctx,
        &mut dir_capped,
        Some(&root),
        None,
        true,
        click_events(ab_pos, egui::PointerButton::Primary),
    );
    assert!(!dir_capped.is_expanded(&root.join("a/b")));
    assert!(dir_capped.is_truncated());

    // 2. Multi-segment compact chain `[a, a/b, a/b/c]` (`chain.len() == 3`) with `max_depth: 2`:
    //    cache `a` (depth 1) and `a/b` (depth 2) in non-compact mode, collapse both, switch to
    //    compact mode so `a/b/c` renders as a 3-segment collapsed chain, and click it -> all 3
    //    segments stay closed and `truncated` becomes `true`.
    let mut tree = ExplorerTree::with_lister_and_limits(
        Arc::new(MultiCompactLister),
        TreeLimits {
            max_depth: 2,
            ..TreeLimits::default()
        },
    );
    tree.set_root(Some(&root));
    wait_until_idle(&mut tree);
    let _ = run_frame(&ctx, &mut tree, Some(&root), None, true, vec![]);

    for rel in ["a", "a/b"] {
        let pos = tree
            .rendered_row_rect(&root.join(rel))
            .expect("folder row")
            .center();
        let _ = run_frame(
            &ctx,
            &mut tree,
            Some(&root),
            None,
            true,
            click_events(pos, egui::PointerButton::Primary),
        );
        wait_until_idle(&mut tree);
        let _ = run_frame(&ctx, &mut tree, Some(&root), None, true, vec![]);
    }
    assert!(!tree.is_truncated());

    for rel in ["a/b", "a"] {
        let pos = tree
            .rendered_row_rect(&root.join(rel))
            .expect("folder row")
            .center();
        let _ = run_frame(
            &ctx,
            &mut tree,
            Some(&root),
            None,
            true,
            click_events(pos, egui::PointerButton::Primary),
        );
        let _ = run_frame(&ctx, &mut tree, Some(&root), None, true, vec![]);
    }
    tree.set_compact_folders(true);
    let _ = run_frame(&ctx, &mut tree, Some(&root), None, true, vec![]);
    assert!(!tree.is_expanded(&root.join("a")));
    assert!(!tree.is_expanded(&root.join("a/b")));

    let abc_pos = tree
        .rendered_row_rect(&root.join("a/b/c"))
        .expect("collapsed a/b/c row")
        .center();
    let _ = run_frame(
        &ctx,
        &mut tree,
        Some(&root),
        None,
        true,
        click_events(abc_pos, egui::PointerButton::Primary),
    );
    assert!(!tree.is_expanded(&root.join("a")));
    assert!(!tree.is_expanded(&root.join("a/b")));
    assert!(!tree.is_expanded(&root.join("a/b/c")));
    assert!(tree.is_truncated());

    // 3. Direct multi-segment `open_dirs` capacity check (`510 + 3 = 513 > 512`) when `a/b/c` is
    //    ALREADY cached in `dirs` (`can_fetch == true`) and `depth == 3 <= 24`:
    let mut root_entries = vec![TreeDirEntry {
        name: "a".into(),
        path: root.join("a"),
        directory: true,
    }];
    let mut pre_open = Vec::with_capacity(510);
    for i in 0..510 {
        let name = format!("s{i:03}");
        let p = root.join(&name);
        root_entries.push(TreeDirEntry {
            name,
            path: p.clone(),
            directory: true,
        });
        pre_open.push(p);
    }
    let mut fx_cap = ExplorerTree::fixture(
        root.clone(),
        vec![
            (root.clone(), root_entries),
            (
                root.join("a"),
                vec![TreeDirEntry {
                    name: "b".into(),
                    path: root.join("a/b"),
                    directory: true,
                }],
            ),
            (
                root.join("a/b"),
                vec![TreeDirEntry {
                    name: "c".into(),
                    path: root.join("a/b/c"),
                    directory: true,
                }],
            ),
            (root.join("a/b/c"), vec![]),
        ],
        pre_open,
    );
    fx_cap.set_compact_folders(true);
    let ctx_cap = egui::Context::default();
    let _ = run_frame(&ctx_cap, &mut fx_cap, Some(&root), None, true, vec![]);
    assert!(!fx_cap.is_truncated());
    let fx_abc_pos = fx_cap
        .rendered_row_rect(&root.join("a/b/c"))
        .expect("a/b/c row at top of scroll area")
        .center();
    let _ = run_frame(
        &ctx_cap,
        &mut fx_cap,
        Some(&root),
        None,
        true,
        click_events(fx_abc_pos, egui::PointerButton::Primary),
    );
    assert!(!fx_cap.is_expanded(&root.join("a")));
    assert!(!fx_cap.is_expanded(&root.join("a/b")));
    assert!(!fx_cap.is_expanded(&root.join("a/b/c")));
    assert!(fx_cap.is_truncated());
}

#[test]
fn fs_lister_confines_to_root_preserves_os_paths_and_refuses_symlinks_and_reparse_points() {
    assert_eq!(WIN_FILE_ATTRIBUTE_REPARSE_POINT, 0x0400);
    assert!(has_reparse_point_attr(0x0410));
    assert!(!has_reparse_point_attr(0x0010));
    assert_eq!(
        norm_tree_key(Path::new("C:/Repo/Src/")),
        norm_tree_key(Path::new("C:/Repo/Src"))
    );

    // Non-UTF-8 / unpaired-surrogate paths MUST NOT collide in `norm_tree_key`:
    #[cfg(windows)]
    {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt;
        let p1 = PathBuf::from(OsString::from_wide(&[0x0066, 0xD800]));
        let p2 = PathBuf::from(OsString::from_wide(&[0x0066, 0xD801]));
        assert_eq!(p1.to_string_lossy(), p2.to_string_lossy());
        assert_ne!(norm_tree_key(&p1), norm_tree_key(&p2));
    }

    let root_dir = tempfile::tempdir().expect("root");
    let outside_dir = tempfile::tempdir().expect("outside");
    fs::write(root_dir.path().join("inside.txt"), "ok").expect("write");
    fs::write(outside_dir.path().join("secret.txt"), "secret").expect("write");

    let lister = FsDirectoryLister;
    let cancel = AtomicBool::new(false);
    let limits = TreeLimits::default();

    // Outside directory and `..` traversal are strictly refused!
    assert!(
        lister
            .list_dir(root_dir.path(), outside_dir.path(), &limits, &cancel)
            .is_err()
    );
    let parent_escape = root_dir.path().join("..").join(
        outside_dir
            .path()
            .file_name()
            .expect("outside temp dir name"),
    );
    assert!(
        lister
            .list_dir(root_dir.path(), &parent_escape, &limits, &cancel)
            .is_err()
    );

    // Pre-cancelled flag stops immediately before touching entries:
    let cancelled = AtomicBool::new(true);
    assert!(
        lister
            .list_dir(root_dir.path(), root_dir.path(), &limits, &cancelled)
            .is_err()
    );

    // If OS privileges permit creating a symlink inside `root` pointing to `outside`,
    // `FsDirectoryLister` must omit it from listings AND refuse listing through it!
    #[cfg(windows)]
    {
        let link_path = root_dir.path().join("escape_link");
        if std::os::windows::fs::symlink_dir(outside_dir.path(), &link_path).is_ok() {
            let listing = lister
                .list_dir(root_dir.path(), root_dir.path(), &limits, &cancel)
                .expect("root listing");
            assert!(listing.entries.iter().all(|e| e.name != "escape_link"));
            assert!(
                lister
                    .list_dir(root_dir.path(), &link_path, &limits, &cancel)
                    .is_err()
            );
        }
    }
    #[cfg(unix)]
    {
        let link_path = root_dir.path().join("escape_link");
        if std::os::unix::fs::symlink(outside_dir.path(), &link_path).is_ok() {
            let listing = lister
                .list_dir(root_dir.path(), root_dir.path(), &limits, &cancel)
                .expect("root listing");
            assert!(listing.entries.iter().all(|e| e.name != "escape_link"));
            assert!(
                lister
                    .list_dir(root_dir.path(), &link_path, &limits, &cancel)
                    .is_err()
            );
        }
    }
}
