#[path = "../src/palette.rs"]
mod palette;
#[path = "../src/pane_grid.rs"]
mod pane_grid;
#[path = "../src/shortcuts.rs"]
mod shortcuts;
#[path = "../src/theme_palettes.rs"]
mod theme_palettes;

use eframe::egui::Color32;
use palette::{
    CENTER_MIN_WIDTH, CommandPaletteState, FLOATING_DEFAULT_HEIGHT, FLOATING_MAX_HEIGHT,
    FLOATING_MIN_HEIGHT, PaletteCommand, RESIZE_COLLAPSE_BELOW, RESIZE_DOUBLE_CLICK_MS, SIDE_TABS,
    SIDEBAR_COLLAPSED_RAIL_WIDTH, SIDEBAR_DEFAULT_WIDTH, SIDEBAR_MIN_MAX, SIDEBAR_MIN_WIDTH,
    SidePanelState, TASK_PANEL_DEFAULT_WIDTH, TASK_PANEL_MIN_WIDTH, VISITED_TABS_CAP,
    changes_badge, clamp_floating_height, clamp_sidebar_width, clamp_task_panel_width,
    evaluate_edge_drag, filter_palette_commands, is_builtin_side_tab, is_resize_double_click,
    restored_side_tab, sidebar_max_width,
};
use pane_grid::{
    DIVIDER_KEY_STEP_PCT, DIVIDER_MIN_PCT, EditorTabStrip, FocusDir, GridPreset, LeafKind,
    MIN_CELL_HEIGHT, MIN_CELL_WIDTH, MIN_PANE_HEIGHT, MIN_PANE_WIDTH, PaneLeaf, PaneNode, PxRect,
    ShownTab, SplitDir, TARGET_ASPECT, WorkspacePaneState, active_view_key, add_tab_leaf,
    build_grid_tree, find_leaf, grid_layout, grid_leaf_for, grid_return_leaf, grid_shape,
    is_near_grid, is_tab_leaf, leaf_view_path, move_focus, norm_path, prune_tree,
    resize_divider_drag, resize_divider_key, row_counts, share_of, shares_of, shown_tab,
    split_leaf, tile_tree, tree_leaves, tree_views, view_key, without_leaves, write_view_sizes,
};
use shortcuts::{
    AppKeyContext, AppShortcut, ArrowDir, EditorKeyOutcome, HelpEscapeOutcome, KeyInput,
    TerminalKeyOutcome, chords_of, filter_shortcuts, fold_for_search, is_editor_app_shortcut,
    is_floating_toggle_key, is_terminal_app_shortcut, key_name, route_app_key, route_editor_key,
    route_help_dialog_escape, route_terminal_key, shortcut_groups,
};
use std::collections::{BTreeMap, HashSet};
use theme_palettes::{
    CLASSIC_SPEC, DRACULA_SPEC, GRUVBOX_SPEC, NORD_SPEC, SOLARIZED_DARK_SPEC, THEMES,
    TOKYO_NIGHT_SPEC, ThemeId, WARP_OK_GREEN, WARP_SPEC, is_theme, resolve_theme_id,
    terminal_theme, title_bar_colors,
};

const fn hex(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

#[test]
fn theme_palettes_match_exact_electron_sources() {
    assert_eq!(THEMES.len(), 7);
    assert_eq!(
        THEMES.map(ThemeId::as_str),
        [
            "classic",
            "warp",
            "dracula",
            "nord",
            "tokyo-night",
            "gruvbox",
            "solarized-dark"
        ]
    );
    assert!(is_theme("classic"));
    assert!(is_theme("tokyo-night"));
    assert!(!is_theme("gruvbox-dark"));
    assert_eq!(resolve_theme_id("unknown"), ThemeId::Classic);

    // Classic (`style.css:9-25` + `themes.js:40-46`)
    assert_eq!(CLASSIC_SPEC.ui.chrome, hex(0x10, 0x12, 0x16));
    assert_eq!(CLASSIC_SPEC.ui.surface, hex(0x18, 0x1b, 0x21));
    assert_eq!(CLASSIC_SPEC.ui.surface_2, hex(0x1f, 0x23, 0x2b));
    assert_eq!(CLASSIC_SPEC.ui.surface_3, hex(0x28, 0x2d, 0x37));
    assert_eq!(CLASSIC_SPEC.ui.term, hex(0x15, 0x17, 0x1c));
    assert_eq!(CLASSIC_SPEC.ui.backdrop, hex(0x0c, 0x0d, 0x10));
    assert_eq!(CLASSIC_SPEC.ui.border, hex(0x25, 0x29, 0x32));
    assert_eq!(CLASSIC_SPEC.ui.border_strong, hex(0x33, 0x39, 0x46));
    assert_eq!(CLASSIC_SPEC.ui.accent, hex(0x6c, 0x9c, 0xff));
    assert_eq!(CLASSIC_SPEC.ui.warn, hex(0xf5, 0xa5, 0x24));
    assert_eq!(CLASSIC_SPEC.ui.danger, hex(0xe5, 0x48, 0x4d));
    assert_eq!(CLASSIC_SPEC.ui.text, hex(0xd6, 0xd9, 0xdf));
    assert_eq!(CLASSIC_SPEC.ui.text_strong, hex(0xf1, 0xf3, 0xf6));
    assert_eq!(CLASSIC_SPEC.ui.text_dim, hex(0x8a, 0x92, 0xa0));
    assert_eq!(CLASSIC_SPEC.radius_px, 7.0);
    assert!(CLASSIC_SPEC.term.ansi.is_none());
    assert!(!ThemeId::Classic.is_named_palette());

    // Warp-inspired (`themes.css:3-22` + `themes.js:47-53`) — graphite/green, never overriding ANSI.
    assert_eq!(WARP_SPEC.ui.chrome, hex(0x16, 0x19, 0x17));
    assert_eq!(WARP_SPEC.ui.surface, hex(0x20, 0x23, 0x21));
    assert_eq!(WARP_SPEC.ui.surface_2, hex(0x25, 0x2b, 0x27));
    assert_eq!(WARP_SPEC.ui.surface_3, hex(0x2c, 0x39, 0x30));
    assert_eq!(WARP_SPEC.ui.term, hex(0x19, 0x1b, 0x1a));
    assert_eq!(WARP_SPEC.ui.border, hex(0x30, 0x36, 0x31));
    assert_eq!(WARP_SPEC.ui.border_strong, hex(0x47, 0x54, 0x4a));
    assert_eq!(WARP_SPEC.ui.accent, hex(0xae, 0xd5, 0xb2));
    assert_eq!(WARP_SPEC.ui.text, hex(0xdf, 0xe5, 0xdf));
    assert_eq!(WARP_SPEC.radius_px, 5.0);
    assert_eq!(WARP_OK_GREEN, hex(0xa1, 0xc8, 0xa7));
    assert!(WARP_SPEC.term.ansi.is_none());
    assert!(!ThemeId::Warp.is_named_palette());
    assert_eq!(title_bar_colors("warp").color, hex(0x16, 0x19, 0x17));
    assert_eq!(title_bar_colors("warp").symbol_color, hex(0xdf, 0xe5, 0xdf));

    // Named ANSI themes (`shared/themePalettes.js`)
    for id in [
        ThemeId::Dracula,
        ThemeId::Nord,
        ThemeId::TokyoNight,
        ThemeId::Gruvbox,
        ThemeId::SolarizedDark,
    ] {
        assert!(id.is_named_palette());
        let spec = id.spec();
        assert_eq!(spec.ui.backdrop, spec.ui.chrome);
        assert_eq!(spec.term.cursor_accent, spec.term.background);
        let ansi = spec
            .term
            .ansi
            .expect("named palette must include 16 ANSI colors");
        assert_eq!(ansi.as_array().len(), 16);
    }

    assert_eq!(DRACULA_SPEC.ui.chrome, hex(0x1e, 0x1f, 0x29));
    assert_eq!(DRACULA_SPEC.ui.surface, hex(0x21, 0x22, 0x2c));
    assert_eq!(NORD_SPEC.ui.chrome, hex(0x24, 0x29, 0x33));
    assert_eq!(TOKYO_NIGHT_SPEC.ui.chrome, hex(0x16, 0x16, 0x1e));
    assert_eq!(GRUVBOX_SPEC.ui.accent, hex(0xfa, 0xbd, 0x2f));
    assert_eq!(SOLARIZED_DARK_SPEC.term.foreground, hex(0x83, 0x94, 0x96));
    assert_eq!(terminal_theme("dracula").background, hex(0x28, 0x2a, 0x36));

    // EN/FR labels & descriptions
    assert_eq!(ThemeId::Classic.label(false), "Classic");
    assert_eq!(ThemeId::Classic.label(true), "Classique");
    assert_eq!(ThemeId::Warp.label(false), "Warp-inspired");
    assert_eq!(ThemeId::Warp.label(true), "Inspiré de Warp");
    assert_eq!(ThemeId::Gruvbox.label(false), "Gruvbox");
    assert_eq!(ThemeId::Gruvbox.label(true), "Gruvbox");
    assert!(ThemeId::Nord.description(true).contains("arctique"));
    assert!(ThemeId::SolarizedDark.description(false).contains("teal"));
}

#[test]
fn grid_shape_tiling_and_dividers_match_pane_grid_js() {
    assert_eq!(TARGET_ASPECT, 1.2);
    assert_eq!(MIN_CELL_WIDTH, 360.0);
    assert_eq!(MIN_CELL_HEIGHT, 160.0);
    assert_eq!(MIN_PANE_WIDTH, 325.0);
    assert_eq!(MIN_PANE_HEIGHT, 156.0);
    assert_eq!(DIVIDER_KEY_STEP_PCT, 2.0);
    assert_eq!(DIVIDER_MIN_PCT, 5.0);

    assert_eq!(row_counts(0, 0), Vec::<usize>::new());
    assert_eq!(row_counts(5, 2), vec![3, 2]);
    assert_eq!(grid_shape(0, 1600.0, 900.0), Vec::<usize>::new());
    assert_eq!(grid_shape(1, 1600.0, 900.0), vec![1]);
    assert_eq!(grid_shape(2, 1600.0, 900.0), vec![2]);
    assert_eq!(grid_shape(4, 1600.0, 900.0), vec![2, 2]);
    assert_eq!(grid_shape(5, 1600.0, 900.0), vec![3, 2]);
    // Fallback room 1600x900 when dimensions <= 0
    assert_eq!(grid_shape(4, 0.0, 0.0), vec![2, 2]);

    assert_eq!(GridPreset::ALL.len(), 5);
    assert_eq!(GridPreset::ThreeByTwo.label(), "3x2");
    assert_eq!(GridPreset::ThreeByTwo.row_counts(), vec![3, 3]);

    let mut seq = 0usize;
    let mut mk_id = || {
        seq += 1;
        format!("sp-{seq}")
    };
    let leaves: Vec<PaneLeaf> = (1..=3)
        .map(|i| PaneLeaf::new(format!("t{i}"), LeafKind::Shell))
        .collect();
    let tiled = build_grid_tree(leaves, &[2, 1], &mut mk_id).expect("tree");
    assert!(is_near_grid(Some(&tiled)));

    let layout = grid_layout(Some(&tiled));
    assert_eq!(layout.tiles.len(), 3);
    assert_eq!(layout.dividers.len(), 2);
    let top_left = layout.tiles[0].rect.to_px(1003.0, 603.0, 3.0);
    let top_right = layout.tiles[1].rect.to_px(1003.0, 603.0, 3.0);
    let bottom = layout.tiles[2].rect.to_px(1003.0, 603.0, 3.0);
    assert!((top_left.width - 500.0).abs() < 1e-6);
    assert!((top_right.left - 503.0).abs() < 1e-6);
    assert!((bottom.width - 1003.0).abs() < 1e-6);

    // Keyboard divider step (±2%, >= 5% floor)
    let stepped = resize_divider_key(2, &[50.0, 50.0], 0, true).unwrap();
    assert_eq!(stepped, vec![52.0, 48.0]);
    assert!(resize_divider_key(2, &[6.0, 94.0], 0, false).is_none());

    // Pointer drag alters ONLY the adjacent pair and clamps to MIN_PANE_WIDTH / MIN_PANE_HEIGHT
    let dragged =
        resize_divider_drag(3, &[33.0, 33.0, 34.0], 0, SplitDir::Row, 1000.0, -900.0).unwrap();
    assert!((dragged[2] - 34.0).abs() < 1e-9);
    assert!((dragged[0] + dragged[1] - 66.0).abs() < 1e-9);
    assert!(dragged[0] >= 32.5 - 1e-6);

    // Geometric Alt+Arrows (`dist * 4 + off`)
    let rects = vec![
        (
            "t1".to_owned(),
            PxRect {
                left: 0.0,
                top: 0.0,
                width: 500.0,
                height: 300.0,
            },
        ),
        (
            "t2".to_owned(),
            PxRect {
                left: 503.0,
                top: 0.0,
                width: 500.0,
                height: 300.0,
            },
        ),
        (
            "t3".to_owned(),
            PxRect {
                left: 0.0,
                top: 303.0,
                width: 1003.0,
                height: 300.0,
            },
        ),
    ];
    assert_eq!(
        move_focus(&rects, "t1", FocusDir::Right).as_deref(),
        Some("t2")
    );
    assert_eq!(
        move_focus(&rects, "t2", FocusDir::Down).as_deref(),
        Some("t3")
    );
    assert_eq!(
        move_focus(&rects, "t3", FocusDir::Up).as_deref(),
        Some("t1")
    );
    assert_eq!(move_focus(&rects, "t1", FocusDir::Left), None);
}

#[test]
fn workspace_views_prune_writeback_and_grid_return_leaf_invariants() {
    let cwd = r"C:\Repo\App\";
    assert_eq!(norm_path(cwd), "c:/repo/app");

    let t1 = PaneLeaf::new("t1", LeafKind::Shell).with_worktree(r"C:\Repo\App");
    let t2 = PaneLeaf::new("t2", LeafKind::Agent).with_view_path("c:/repo/app");
    let wt_shell = PaneLeaf::new("wt1", LeafKind::Shell).with_worktree(r"C:\Repo\App-wt1");
    let ed1 = PaneLeaf::new("ed1", LeafKind::Editor).with_view_path(r"C:\Repo\App");

    assert_eq!(leaf_view_path(&t1), r"C:\Repo\App");
    assert_eq!(view_key(&t1, cwd), "");
    assert_eq!(view_key(&wt_shell, cwd), "c:/repo/app-wt1");
    assert!(!is_tab_leaf(&t1));
    assert!(is_tab_leaf(&ed1));

    // Split with 3 children across two worktrees to test prune_tree + write_view_sizes
    let split = PaneNode::split(
        "root",
        SplitDir::Row,
        vec![
            PaneNode::Leaf(t1.clone()),
            PaneNode::Leaf(wt_shell.clone()),
            PaneNode::Leaf(t2.clone()),
        ],
        vec![30.0, 40.0, 30.0],
    );
    let tree_with_tab = add_tab_leaf(Some(split), ed1.clone(), "with-tab");
    assert_eq!(share_of(2, &shares_of(2, &[50.0, 50.0]), 0), 50.0);

    let views = tree_views(Some(&tree_with_tab), cwd);
    assert_eq!(views.len(), 2);
    assert_eq!(views[0].key, ""); // main project view is ALWAYS first
    assert_eq!(views[1].key, "c:/repo/app-wt1");
    assert_eq!(active_view_key(&views, Some("wt1")), "c:/repo/app-wt1");

    let mut sources = BTreeMap::new();
    let pruned_main = prune_tree(
        &tree_with_tab,
        &mut |l| view_key(l, cwd).is_empty() && !is_tab_leaf(l),
        &mut sources,
    )
    .expect("pruned main grid");
    assert!(!is_near_grid(Some(&PaneNode::split(
        "uneven",
        SplitDir::Row,
        vec![PaneNode::Leaf(t1.clone()), PaneNode::Leaf(t2.clone())],
        vec![20.0, 80.0],
    ))));
    assert_eq!(find_leaf(Some(&pruned_main), "t1").unwrap().id, "t1");
    assert_eq!(tree_leaves(Some(&pruned_main), |_| true).len(), 2);

    let meta = sources.get("root::view").expect("pruned split metadata");
    let mut writable = tree_with_tab.clone();
    assert!(write_view_sizes(&mut writable, meta, &[75.0, 25.0]));
    // t1 + t2 originally held 30 + 30 = 60% of `root`, while wt1 kept 40%:
    // new shares in `root` must be [45%, 40%, 15%]!
    if let PaneNode::Split { children, .. } = &writable
        && let PaneNode::Split { sizes, .. } = &children[0]
    {
        assert!((sizes[0] - 45.0).abs() < 1e-6);
        assert!((sizes[1] - 40.0).abs() < 1e-6);
        assert!((sizes[2] - 15.0).abs() < 1e-6);
    } else {
        panic!("expected root split");
    }

    // tile_tree & split_leaf & without_leaves
    let split_again = split_leaf(
        &pruned_main,
        "t2",
        SplitDir::Col,
        PaneLeaf::new("t3", LeafKind::Shell),
        "sub",
    );
    assert_eq!(tree_leaves(Some(&split_again), |_| true).len(), 3);
    let mut seq = 0usize;
    let retiled = tile_tree(
        Some(&tree_with_tab),
        |l| view_key(l, cwd).is_empty() && !is_tab_leaf(l),
        Some(PaneLeaf::new("t4", LeafKind::Shell)),
        1200.0,
        800.0,
        || {
            seq += 1;
            format!("tile-{seq}")
        },
    )
    .unwrap();
    // wt1 and ed1 remain intact!
    assert!(find_leaf(Some(&retiled), "wt1").is_some());
    assert!(find_leaf(Some(&retiled), "ed1").is_some());
    assert!(find_leaf(Some(&retiled), "t4").is_some());

    let drop: HashSet<String> = ["t4".to_owned()].into_iter().collect();
    let after_drop = without_leaves(&retiled, &drop).unwrap();
    assert!(find_leaf(Some(&after_drop), "t4").is_none());

    // Regression test for Hopper's `gridReturnLeaf` invariant (`657f0028`):
    // T1 is maximized (`maximized_id = Some("t1")`), T2 was last `grid_id`, and editor `ed1`
    // is shown/closed -> focus MUST return to `t1` (the maximized pane on screen), NOT `t2`!
    let mut ws = WorkspacePaneState::new(cwd);
    ws.tree = Some(tree_with_tab);
    ws.set_active("t2");
    assert_eq!(ws.grid_id.as_deref(), Some("t2"));
    let mut maximized_id = Some("t1".to_owned());

    assert_eq!(
        grid_leaf_for(ws.tree.as_ref(), &ws.cwd, ws.grid_id.as_deref(), &ed1)
            .unwrap()
            .id,
        "t2"
    );
    assert_eq!(
        grid_return_leaf(
            ws.tree.as_ref(),
            &ws.cwd,
            ws.grid_id.as_deref(),
            maximized_id.as_deref(),
            true,
            &ed1,
        )
        .unwrap()
        .id,
        "t1"
    );

    ws.show_view_tab(
        ShownTab::Leaf("ed1".to_owned()),
        maximized_id.as_deref(),
        true,
    );
    let layers = ws.layers();
    assert_eq!(layers.len(), 2);
    assert_eq!(layers[0].key, ":main");
    assert_eq!(layers[0].shown, ShownTab::Leaf("ed1".to_owned()));
    assert_eq!(
        shown_tab(
            ws.active_id.as_deref(),
            &ws.view,
            &views[0].ids,
            true,
            &layers[0].tabs,
        ),
        ShownTab::Leaf("ed1".to_owned())
    );

    // Switching back to Terminal tab picks maximized T1
    ws.show_view_tab(ShownTab::Terminal, maximized_id.as_deref(), true);
    assert_eq!(ws.active_id.as_deref(), Some("t1"));
    assert!(ws.view.is_terminal());

    // Closing active editor tab also returns to maximized T1
    ws.show_view_tab(
        ShownTab::Leaf("ed1".to_owned()),
        maximized_id.as_deref(),
        true,
    );
    ws.close_leaf("ed1", &mut maximized_id, true);
    assert_eq!(ws.active_id.as_deref(), Some("t1"));
    assert_eq!(maximized_id.as_deref(), Some("t1"));
    assert!(ws.view.is_terminal());
}

#[test]
fn editor_tab_strip_reuses_preview_pins_dirty_and_names_accessible_state() {
    let mut strip = EditorTabStrip::default();
    strip.open_file("src/a.rs", false);
    assert_eq!(strip.files.len(), 1);
    assert!(!strip.files[0].pinned);

    // Second single-click reuses unpinned clean preview tab
    strip.open_file("src/b.rs", false);
    assert_eq!(strip.files.len(), 1);
    assert_eq!(strip.active_path.as_deref(), Some("src/b.rs"));

    // Editing marks dirty and auto-pins
    strip.set_dirty("src/b.rs", true);
    assert!(strip.files[0].pinned);
    assert!(strip.files[0].dirty);
    assert_eq!(
        EditorTabStrip::accessible_label(&strip.files[0], false),
        "b.rs (unsaved)"
    );
    assert_eq!(
        EditorTabStrip::accessible_label(&strip.files[0], true),
        "b.rs (modifié)"
    );

    // Next preview opens a new slot beside pinned dirty file
    strip.open_file("src/c.rs", false);
    assert_eq!(strip.files.len(), 2);
    strip.pin("src/c.rs");
    assert!(strip.files[1].pinned);

    // Dirty tab refuses unforced close, closes when forced
    assert!(!strip.close("src/b.rs", false));
    assert!(strip.close("src/b.rs", true));
    assert_eq!(strip.files.len(), 1);
}

#[test]
fn shortcuts_match_exact_on_key_terminal_editor_and_fr_help_rules() {
    // Ctrl+` across US, French AZERTY (`²` on physical Backquote), and modifiers
    assert!(is_floating_toggle_key(
        &KeyInput::new("`", "Backquote").with_ctrl()
    ));
    assert!(is_floating_toggle_key(
        &KeyInput::new("²", "Backquote").with_ctrl()
    ));
    assert!(is_floating_toggle_key(&KeyInput::new("`", "").with_ctrl()));
    assert!(!is_floating_toggle_key(
        &KeyInput::new("~", "Backquote").with_ctrl().with_shift()
    ));
    assert!(!is_floating_toggle_key(
        &KeyInput::new("`", "Backquote").with_ctrl().with_alt()
    ));
    assert!(!is_floating_toggle_key(
        &KeyInput::new("`", "Backquote").with_ctrl().with_meta()
    ));

    // Terminal key routing: plain Ctrl+C, Ctrl+W, Escape go to PTY; app shortcuts withheld
    assert_eq!(
        route_terminal_key(&KeyInput::new("c", "KeyC").with_ctrl(), false),
        TerminalKeyOutcome::SendToPty
    );
    assert_eq!(
        route_terminal_key(&KeyInput::new("w", "KeyW").with_ctrl(), false),
        TerminalKeyOutcome::SendToPty
    );
    assert_eq!(
        route_terminal_key(&KeyInput::new("Escape", "Escape"), false),
        TerminalKeyOutcome::SendToPty
    );
    assert_eq!(
        route_terminal_key(&KeyInput::new("f", "KeyF").with_ctrl().with_shift(), false),
        TerminalKeyOutcome::OpenFind
    );
    assert_eq!(
        route_terminal_key(&KeyInput::new("c", "KeyC").with_ctrl().with_shift(), false),
        TerminalKeyOutcome::CopySelection
    );
    assert_eq!(
        route_terminal_key(&KeyInput::new("v", "KeyV").with_ctrl(), false),
        TerminalKeyOutcome::PasteClipboard
    );
    assert_eq!(
        route_terminal_key(&KeyInput::new("Enter", "Enter").with_shift(), true),
        TerminalKeyOutcome::AgentNewline
    );
    assert_eq!(
        route_terminal_key(&KeyInput::new("PageUp", "PageUp").with_shift(), false),
        TerminalKeyOutcome::ScrollPageUp
    );
    assert_eq!(
        route_terminal_key(&KeyInput::new("PageDown", "PageDown").with_shift(), false),
        TerminalKeyOutcome::ScrollPageDown
    );
    assert!(is_terminal_app_shortcut(
        &KeyInput::new("e", "KeyE").with_ctrl().with_shift()
    ));

    // Editor key routing: Alt+ArrowUp stays in editor, Alt+ArrowLeft forwards to app
    assert_eq!(
        route_editor_key(&KeyInput::new("s", "KeyS").with_ctrl(), false, false),
        EditorKeyOutcome::Save
    );
    assert_eq!(
        route_editor_key(
            &KeyInput::new("s", "KeyS").with_ctrl().with_repeat(),
            false,
            false
        ),
        EditorKeyOutcome::ConsumedRepeat
    );
    assert_eq!(
        route_editor_key(&KeyInput::new("w", "KeyW").with_ctrl(), false, false),
        EditorKeyOutcome::CloseTab
    );
    assert_eq!(
        route_editor_key(&KeyInput::new("z", "KeyZ").with_alt(), false, false),
        EditorKeyOutcome::ToggleWordWrap
    );
    assert_eq!(
        route_editor_key(&KeyInput::new("F7", "F7").with_shift(), false, true),
        EditorKeyOutcome::JumpDiff { previous: true }
    );
    assert!(!is_editor_app_shortcut(
        &KeyInput::new("ArrowUp", "ArrowUp").with_alt()
    ));
    assert_eq!(
        route_editor_key(
            &KeyInput::new("ArrowLeft", "ArrowLeft").with_alt(),
            false,
            false
        ),
        EditorKeyOutcome::ForwardToApp
    );
    assert_eq!(
        route_editor_key(&KeyInput::new("w", "KeyW").with_ctrl(), true, true),
        EditorKeyOutcome::LetEditorHandle
    );

    // App.vue `onKey` guards: typingInField, floatingPaneKey, dialogOpen, AZERTY Digit0, Escape
    let field_ctx = AppKeyContext {
        typing_in_field: true,
        ..AppKeyContext::default()
    };
    assert_eq!(
        route_app_key(
            &KeyInput::new("p", "KeyP").with_ctrl().with_shift(),
            field_ctx
        ),
        AppShortcut::None
    );
    assert_eq!(
        route_app_key(&KeyInput::new("F1", "F1"), field_ctx),
        AppShortcut::ToggleHelp
    );
    assert_eq!(
        route_app_key(&KeyInput::new("²", "Backquote").with_ctrl(), field_ctx),
        AppShortcut::ToggleFloatingTerminal
    );
    assert_eq!(
        route_app_key(
            &KeyInput::new("`", "Backquote").with_ctrl().with_repeat(),
            AppKeyContext::default()
        ),
        AppShortcut::ConsumedNoop
    );

    let floating_ctx = AppKeyContext {
        in_floating_terminal: true,
        ..AppKeyContext::default()
    };
    assert_eq!(
        route_app_key(
            &KeyInput::new("w", "KeyW").with_ctrl().with_shift(),
            floating_ctx
        ),
        AppShortcut::FloatingClose
    );
    assert_eq!(
        route_app_key(
            &KeyInput::new("r", "KeyR").with_ctrl().with_shift(),
            floating_ctx
        ),
        AppShortcut::FloatingRestart
    );
    assert_eq!(
        route_app_key(
            &KeyInput::new("e", "KeyE").with_ctrl().with_shift(),
            floating_ctx
        ),
        AppShortcut::ConsumedNoop
    );
    assert_eq!(
        route_app_key(
            &KeyInput::new("ArrowLeft", "ArrowLeft").with_alt(),
            floating_ctx
        ),
        AppShortcut::ConsumedNoop
    );

    let dialog_ctx = AppKeyContext {
        dialog_open: true,
        ..AppKeyContext::default()
    };
    assert_eq!(
        route_app_key(&KeyInput::new(",", "Comma").with_ctrl(), dialog_ctx),
        AppShortcut::CloseSettings
    );
    assert_eq!(
        route_app_key(&KeyInput::new("F1", "F1"), dialog_ctx),
        AppShortcut::CloseHelp
    );
    assert_eq!(
        route_app_key(
            &KeyInput::new("t", "KeyT").with_ctrl().with_shift(),
            dialog_ctx
        ),
        AppShortcut::None
    );

    // AZERTY `Ctrl+à` (`Digit0`) resets zoom
    assert_eq!(
        route_app_key(
            &KeyInput::new("à", "Digit0").with_ctrl(),
            AppKeyContext::default()
        ),
        AppShortcut::Zoom(0)
    );
    assert_eq!(
        route_app_key(
            &KeyInput::new("+", "NumpadAdd").with_ctrl(),
            AppKeyContext::default()
        ),
        AppShortcut::Zoom(1)
    );
    assert_eq!(
        route_app_key(
            &KeyInput::new("-", "NumpadSubtract").with_ctrl(),
            AppKeyContext::default()
        ),
        AppShortcut::Zoom(-1)
    );
    assert_eq!(
        route_app_key(
            &KeyInput::new("PageDown", "PageDown").with_ctrl(),
            AppKeyContext::default()
        ),
        AppShortcut::CycleWorkspace(1)
    );
    assert_eq!(
        route_app_key(
            &KeyInput::new("ArrowRight", "ArrowRight").with_alt(),
            AppKeyContext::default()
        ),
        AppShortcut::MoveFocus(ArrowDir::Right)
    );
    assert_eq!(
        route_app_key(
            &KeyInput::new("Escape", "Escape"),
            AppKeyContext {
                side_panel_fullscreen: true,
                focus_in_side_panel_or_body: true,
                ..AppKeyContext::default()
            }
        ),
        AppShortcut::ExitSidePanelFullscreen
    );
    assert_eq!(
        route_app_key(
            &KeyInput::new("Escape", "Escape"),
            AppKeyContext {
                has_maximized_pane: true,
                ..AppKeyContext::default()
            }
        ),
        AppShortcut::EscapeCleanup {
            restore_maximized: true
        }
    );

    // HelpDialog Escape & EN/FR 38-shortcut search
    assert_eq!(
        route_help_dialog_escape("split"),
        HelpEscapeOutcome::ClearQuery
    );
    assert_eq!(route_help_dialog_escape(""), HelpEscapeOutcome::CloseDialog);

    let en_groups = shortcut_groups(false);
    let fr_groups = shortcut_groups(true);
    assert_eq!(en_groups.iter().map(|g| g.rows.len()).sum::<usize>(), 38);
    assert_eq!(fr_groups.iter().map(|g| g.rows.len()).sum::<usize>(), 38);
    assert_eq!(key_name("Shift", true), "Maj");
    assert_eq!(key_name("Esc", true), "Échap");
    assert_eq!(
        chords_of("F7 / Shift+F7", true),
        vec![vec!["F7"], vec!["Maj", "F7"]]
    );
    assert_eq!(fold_for_search("Échap Précédent"), "echap precedent");

    let echap = filter_shortcuts(true, "echap");
    assert_eq!(echap.len(), 1);
    assert_eq!(echap[0].rows[0].keys, "Esc");

    let maj_t = filter_shortcuts(true, "maj+t");
    assert!(
        maj_t
            .iter()
            .any(|g| g.rows.iter().any(|r| r.keys == "Ctrl+Shift+T"))
    );
}

#[test]
fn command_palette_side_panel_and_layout_limits_match_reference() {
    let cmds = vec![
        PaletteCommand::new("c1", "Panes", "Close pane").with_shortcut("Ctrl+Shift+W"),
        PaletteCommand::new("c2", "Panes", "Split right").with_shortcut("Ctrl+Shift+E"),
        PaletteCommand::new("c3", "Workspaces", "Switch to Tessel Rust")
            .with_hint("C:\\Tessel-rust"),
        PaletteCommand::new("c4", "App", "Split down").with_shortcut("Ctrl+Shift+O"),
    ];

    let all_rows = filter_palette_commands(&cmds, "");
    assert_eq!(all_rows.len(), 4);
    assert_eq!(all_rows[0].header.as_deref(), Some("Panes"));
    assert_eq!(all_rows[1].header, None); // same group -> no duplicate header
    assert_eq!(all_rows[2].header.as_deref(), Some("Workspaces"));

    let split_rows = filter_palette_commands(&cmds, "split");
    assert_eq!(split_rows.len(), 2);
    assert_eq!(split_rows[0].command.id, "c2");
    assert_eq!(split_rows[1].command.id, "c4");

    let mut pal = CommandPaletteState::default();
    pal.toggle();
    assert!(pal.open);
    pal.move_selection(-1, split_rows.len());
    assert_eq!(pal.selected_id(&split_rows), Some("c4")); // wrapped to end
    pal.set_query("rust");
    assert_eq!(pal.index, 0);
    pal.close();
    assert!(!pal.open);

    // SidePanel tabs, visited fallback, and badge
    assert_eq!(SIDE_TABS.len(), 7);
    assert!(is_builtin_side_tab("changes"));
    assert_eq!(VISITED_TABS_CAP, 20);
    assert_eq!(
        restored_side_tab(Some("web-gone"), &["web-live".to_owned()]),
        Some("dashboard".to_owned())
    );
    assert_eq!(
        restored_side_tab(Some("web-live"), &["web-live".to_owned()]),
        Some("web-live".to_owned())
    );
    assert_eq!(changes_badge(0), None);
    assert_eq!(changes_badge(12).as_deref(), Some("12"));
    assert_eq!(changes_badge(140).as_deref(), Some("99+"));

    let mut side = SidePanelState::default();
    assert_eq!(side.current_tab(), "tasks");
    side.toggle_tab("files");
    assert!(side.open);
    assert_eq!(side.current_tab(), "files");
    side.toggle_tab("files"); // second press closes
    assert!(!side.open);

    side.show_tab("changes");
    side.add_browser("web-1", "https://example.com", true);
    assert_eq!(side.current_tab(), "web-1");
    side.close_browser("web-1");
    assert_eq!(side.current_tab(), "changes"); // restored from `visited`!

    // Layout & FloatingTerminal limits
    assert_eq!(SIDEBAR_DEFAULT_WIDTH, 280);
    assert_eq!(SIDEBAR_MIN_WIDTH, 220);
    assert_eq!(SIDEBAR_COLLAPSED_RAIL_WIDTH, 52);
    assert_eq!(SIDEBAR_MIN_MAX, 500);
    assert_eq!(TASK_PANEL_DEFAULT_WIDTH, 330);
    assert_eq!(TASK_PANEL_MIN_WIDTH, 260);
    assert_eq!(CENTER_MIN_WIDTH, 420);
    assert_eq!(RESIZE_COLLAPSE_BELOW, 110);
    assert_eq!(RESIZE_DOUBLE_CLICK_MS, 400);

    assert_eq!(sidebar_max_width(1280), 500);
    assert_eq!(sidebar_max_width(2400), 720);
    assert_eq!(clamp_sidebar_width(180, 1280), 220);
    assert_eq!(clamp_task_panel_width(600, 1280, false, 280), 580); // 1280 - 280 - 420 = 580
    assert_eq!(clamp_task_panel_width(600, 1280, true, 280), 600); // 1280 - 52 - 420 = 808
    assert!(is_resize_double_click(Some(1000), 1350));
    assert!(!is_resize_double_click(Some(1000), 1400));
    assert_eq!(evaluate_edge_drag(80, true, 280, 220), (false, 280));
    assert_eq!(evaluate_edge_drag(240, false, 280, 240), (true, 240));

    assert_eq!(FLOATING_MIN_HEIGHT, 0.15);
    assert_eq!(FLOATING_MAX_HEIGHT, 0.90);
    assert_eq!(FLOATING_DEFAULT_HEIGHT, 0.45);
    assert_eq!(clamp_floating_height(f64::NAN), 0.45);
    assert_eq!(clamp_floating_height(0.05), 0.15);
    assert_eq!(clamp_floating_height(0.99), 0.90);
    assert_eq!(clamp_floating_height(0.4567), 0.457);
}
