#[allow(dead_code)]
#[path = "../src/explorer_scm_model.rs"]
mod explorer_scm_model;
#[allow(dead_code)]
#[path = "../src/icons.rs"]
mod icons;
#[path = "../src/scm_changes_panel.rs"]
mod scm_changes_panel;
#[allow(dead_code)]
#[path = "../src/theme.rs"]
mod theme;
#[allow(dead_code)]
#[path = "../src/theme_palettes.rs"]
mod theme_palettes;

use eframe::egui::accesskit::Role;
use eframe::egui::{self, Color32, Key, Modifiers, PointerButton, Pos2, Rect, Shape, Vec2};
use explorer_scm_model::{ScmLang, ScmSectionKind};
use scm_changes_panel::{
    GIT_COLOR_ADDED, GIT_COLOR_DELETED, GIT_COLOR_MODIFIED, GIT_COLOR_RENAMED,
    MAX_COMMIT_MESSAGE_CHARS, MAX_FILTER_CHARS, ParsedGitStatus, STATUS_ENTRIES_MAX,
    STATUS_RAW_BYTES_MAX, ScmArea, ScmCapabilities, ScmChangesAction, ScmChangesInput,
    ScmChangesPanel, ScmConflictKind, ScmEntry, ScmFileStatus, ScmGroupOrder, ScmParseIssue,
    ScmRemoteOp, ScmStatusSnapshot, ScmViewMode, discard_area_copy, parse_porcelain_v2_status,
    parse_porcelain_v2_status_bounded,
};
use theme_palettes::{THEMES, ThemeId};

fn collect_shape_texts(shape: &Shape, clip: Rect, out: &mut Vec<(String, Color32, Rect, Rect)>) {
    match shape {
        Shape::Text(t) => {
            let color = t
                .galley
                .job
                .sections
                .first()
                .map_or(t.fallback_color, |s| s.format.color);
            out.push((
                t.galley.job.text.clone(),
                color,
                Rect::from_min_size(t.pos, t.galley.size()),
                clip,
            ));
        }
        Shape::Vec(list) => {
            for inner in list {
                collect_shape_texts(inner, clip, out);
            }
        }
        _ => {}
    }
}

fn painted_texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, Color32, Rect, Rect)> {
    let mut out = Vec::new();
    for cs in shapes {
        collect_shape_texts(&cs.shape, cs.clip_rect, &mut out);
    }
    out
}

fn has_exact_text(shapes: &[egui::epaint::ClippedShape], target: &str) -> bool {
    painted_texts(shapes)
        .iter()
        .any(|(txt, _, _, _)| txt == target)
}

fn has_substring_text(shapes: &[egui::epaint::ClippedShape], needle: &str) -> bool {
    painted_texts(shapes)
        .iter()
        .any(|(txt, _, _, _)| txt.contains(needle))
}

fn find_text_rect(shapes: &[egui::epaint::ClippedShape], target: &str) -> Option<Rect> {
    painted_texts(shapes)
        .into_iter()
        .find(|(txt, _, _, _)| txt == target)
        .map(|(_, _, rect, _)| rect)
}

fn find_a11y_node_center(out: &egui::FullOutput, label_exact: &str) -> Option<Pos2> {
    let update = out.platform_output.accesskit_update.as_ref()?;
    for (_, node) in &update.nodes {
        if node.label() == Some(label_exact)
            && let Some(b) = node.bounds()
        {
            return Some(Pos2::new(
                ((b.x0 + b.x1) * 0.5) as f32,
                ((b.y0 + b.y1) * 0.5) as f32,
            ));
        }
    }
    None
}

fn count_a11y_buttons_with_label(out: &egui::FullOutput, label_exact: &str) -> usize {
    let Some(update) = out.platform_output.accesskit_update.as_ref() else {
        return 0;
    };
    update
        .nodes
        .iter()
        .filter(|(_, n)| n.role() == Role::Button && n.label() == Some(label_exact))
        .count()
}

fn click_events(pos: Pos2) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        },
        egui::Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        },
    ]
}

fn full_caps() -> ScmCapabilities {
    ScmCapabilities {
        can_refresh: true,
        can_open_diff: true,
        can_open_file: true,
        can_stage: true,
        can_unstage: true,
        can_discard: true,
        can_commit: true,
        can_remote: false,
    }
}

fn run_panel_frame(
    ctx: &egui::Context,
    panel: &mut ScmChangesPanel,
    width: f32,
    events: Vec<egui::Event>,
    focused: bool,
    enabled: bool,
    input: ScmChangesInput<'_>,
) -> (Option<ScmChangesAction>, egui::FullOutput) {
    ctx.enable_accesskit();
    let raw = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 760.0))),
        focused,
        events,
        ..Default::default()
    };
    let mut captured = None;
    let out = ctx.run(raw, |ctx| {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.add_enabled_ui(enabled, |ui| {
                    captured = panel.show(ui, input);
                });
            });
    });
    (captured, out)
}

#[test]
fn porcelain_v2_parser_handles_spaces_newlines_renames_copies_conflicts_and_strict_bounds() {
    assert_eq!(STATUS_ENTRIES_MAX, 5_000);
    assert_eq!(STATUS_RAW_BYTES_MAX, 8 * 1024 * 1024);

    // 1. Clean empty slice
    let empty = parse_porcelain_v2_status(b"", STATUS_ENTRIES_MAX);
    assert_eq!(empty, ParsedGitStatus::default());
    assert!(!empty.has_parse_issues());
    assert!(!empty.blocks_bulk_actions());
    assert_eq!(empty.badge_count(true), 0);
    assert_eq!(empty.badge_count(false), 0);

    // 2. Tracked branch + ahead/behind + spaces + embedded newline + dual MM + typechange +
    //    rename + dual rename-modified + copy + ignored + untracked + all 7 conflict kinds.
    let mut payload = Vec::new();
    payload.extend_from_slice(b"# branch.oid 0123456789abcdef\0");
    payload.extend_from_slice(b"# branch.head feat/native-scm\0");
    payload.extend_from_slice(b"# branch.upstream origin/feat/native-scm\0");
    payload.extend_from_slice(b"# branch.ab +4 -2\0");
    payload.extend_from_slice(b"# stash 1\0");
    payload.extend_from_slice(
        b"1 MM N... 100644 100644 100644 aaaaaaa bbbbbbb src/my folder/with spaces.rs\0",
    );
    payload.extend_from_slice(
        b"1 AD N... 100644 100644 000000 ccccccc ddddddd docs/line1\nline2.md\0",
    );
    payload.extend_from_slice(b"1 .T N... 100644 100644 120000 eeeeeee fffffff scripts/link.sh\0");
    payload.extend_from_slice(
        b"2 RM N... 100644 100644 100644 1111111 2222222 R088 src/new name.rs\0src/old name.rs\0",
    );
    payload.extend_from_slice(
        b"2 C. N... 100644 100644 100644 3333333 4444444 C100 src/copied.rs\0src/source.rs\0",
    );
    payload.extend_from_slice(
        b"2 .R N... 100644 100644 100644 5555555 6666666 R100 src/wt_ren.rs\0src/wt_orig.rs\0",
    );
    for (xy, file) in [
        (&b"UU"[..], "c_uu.rs"),
        (&b"AA"[..], "c_aa.rs"),
        (&b"DD"[..], "c_dd.rs"),
        (&b"AU"[..], "c_au.rs"),
        (&b"UD"[..], "c_ud.rs"),
        (&b"UA"[..], "c_ua.rs"),
        (&b"DU"[..], "c_du.rs"),
    ] {
        payload.extend_from_slice(b"u ");
        payload.extend_from_slice(xy);
        payload.extend_from_slice(b" N... 100644 100644 100644 100644 h1 h2 h3 ");
        payload.extend_from_slice(file.as_bytes());
        payload.push(0);
    }
    payload.extend_from_slice(b"! target/ignored.bin\0");
    payload.extend_from_slice(b"? notes/new file\nwith newline.txt\0");

    let parsed = parse_porcelain_v2_status(&payload, STATUS_ENTRIES_MAX);
    assert!(!parsed.has_parse_issues());
    assert!(!parsed.truncated);
    assert_eq!(parsed.branch.as_deref(), Some("feat/native-scm"));
    assert_eq!(parsed.oid.as_deref(), Some("0123456789abcdef"));
    assert_eq!(parsed.upstream.as_deref(), Some("origin/feat/native-scm"));
    assert_eq!(parsed.ahead, 4);
    assert_eq!(parsed.behind, 2);
    assert!(parsed.has_ab);
    assert_eq!(parsed.total_entries, 17);
    assert_eq!(parsed.entries.len(), 17);
    assert_eq!(parsed.badge_count(true), 17);

    // Dual MM produced both Staged and Unstaged entries preserving spaces
    assert!(parsed.entries.iter().any(|e| {
        e.path == "src/my folder/with spaces.rs"
            && e.area == ScmArea::Staged
            && e.status == ScmFileStatus::Modified
    }));
    assert!(parsed.entries.iter().any(|e| {
        e.path == "src/my folder/with spaces.rs"
            && e.area == ScmArea::Unstaged
            && e.status == ScmFileStatus::Modified
    }));
    // Embedded newline preserved verbatim
    assert!(parsed.entries.iter().any(|e| {
        e.path == "docs/line1\nline2.md"
            && e.area == ScmArea::Staged
            && e.status == ScmFileStatus::Added
    }));
    assert!(parsed.entries.iter().any(|e| {
        e.path == "docs/line1\nline2.md"
            && e.area == ScmArea::Unstaged
            && e.status == ScmFileStatus::Deleted
    }));
    // Dual RM carries `old_path` on Staged Renamed and `None` on Unstaged Modified
    assert!(parsed.entries.iter().any(|e| {
        e.path == "src/new name.rs"
            && e.old_path.as_deref() == Some("src/old name.rs")
            && e.area == ScmArea::Staged
            && e.status == ScmFileStatus::Renamed
    }));
    assert!(parsed.entries.iter().any(|e| {
        e.path == "src/new name.rs"
            && e.old_path.is_none()
            && e.area == ScmArea::Unstaged
            && e.status == ScmFileStatus::Modified
    }));
    // Copy & unstaged rename
    assert!(parsed.entries.iter().any(|e| {
        e.path == "src/copied.rs"
            && e.old_path.as_deref() == Some("src/source.rs")
            && e.area == ScmArea::Staged
            && e.status == ScmFileStatus::Copied
    }));
    assert!(parsed.entries.iter().any(|e| {
        e.path == "src/wt_ren.rs"
            && e.old_path.as_deref() == Some("src/wt_orig.rs")
            && e.area == ScmArea::Unstaged
            && e.status == ScmFileStatus::Renamed
    }));
    // Untracked with newline + ignored skipped
    assert!(parsed.entries.iter().any(|e| {
        e.path == "notes/new file\nwith newline.txt"
            && e.area == ScmArea::Untracked
            && e.status == ScmFileStatus::Untracked
    }));
    assert!(
        !parsed
            .entries
            .iter()
            .any(|e| e.path.contains("ignored.bin"))
    );

    // Verify all 7 conflict kinds and their EN/FR labels
    let expected_conflicts = [
        (
            "c_uu.rs",
            ScmConflictKind::BothModified,
            "both modified",
            "modifié des deux côtés",
        ),
        (
            "c_aa.rs",
            ScmConflictKind::BothAdded,
            "both added",
            "ajouté des deux côtés",
        ),
        (
            "c_dd.rs",
            ScmConflictKind::BothDeleted,
            "both deleted",
            "supprimé des deux côtés",
        ),
        (
            "c_au.rs",
            ScmConflictKind::AddedByUs,
            "added by us",
            "ajouté chez nous",
        ),
        (
            "c_ud.rs",
            ScmConflictKind::DeletedByThem,
            "deleted by them",
            "supprimé chez eux",
        ),
        (
            "c_ua.rs",
            ScmConflictKind::AddedByThem,
            "added by them",
            "ajouté chez eux",
        ),
        (
            "c_du.rs",
            ScmConflictKind::DeletedByUs,
            "deleted by us",
            "supprimé chez nous",
        ),
    ];
    for (file, kind, en_lbl, fr_lbl) in expected_conflicts {
        assert_eq!(kind.label(false), en_lbl);
        assert_eq!(kind.label(true), fr_lbl);
        let entry = parsed
            .entries
            .iter()
            .find(|e| e.path == file)
            .expect("conflict entry present");
        assert_eq!(entry.conflict, Some(kind));
        assert!(entry.is_conflict());
        assert!(!entry.can_stage());
        assert!(!entry.can_discard());
        assert!(!entry.can_unstage());
    }

    // 3. Detached HEAD, initial OID & external capture truncation propagation (`git::run_bytes`)
    let mut detached =
        parse_porcelain_v2_status(b"# branch.oid (initial)\0# branch.head (detached)\0", 100);
    assert_eq!(detached.branch, None);
    assert_eq!(detached.oid, None);
    assert!(!detached.has_parse_issues());
    detached.mark_input_truncated();
    assert!(detached.input_bytes_truncated);
    assert!(detached.truncated);
    assert!(detached.has_parse_issues());
    assert!(detached.blocks_bulk_actions());

    // 4. Non-UTF-8 paths are strictly rejected (never lossy-replaced with U+FFFD)
    let mut bad_utf8 = Vec::new();
    bad_utf8.extend_from_slice(b"? bad_\xff\xfe_file.rs\0");
    bad_utf8.extend_from_slice(
        b"2 R. N... 100644 100644 100644 111 222 R100 valid_new.rs\0bad_old_\x80.rs\0",
    );
    bad_utf8.extend_from_slice(b"? clean_utf8.rs\0");
    let utf8_res = parse_porcelain_v2_status(&bad_utf8, 100);
    assert_eq!(utf8_res.non_utf8_paths, 2);
    assert_eq!(utf8_res.malformed_records, 0);
    assert!(matches!(
        utf8_res.first_issue,
        Some(ScmParseIssue::NonUtf8Path { .. })
    ));
    assert!(utf8_res.has_parse_issues());
    assert!(utf8_res.blocks_bulk_actions());
    assert_eq!(utf8_res.entries.len(), 1);
    assert_eq!(utf8_res.entries[0].path, "clean_utf8.rs");
    assert!(!utf8_res.entries[0].path.contains('\u{FFFD}'));

    // 5. Truncated & malformed records (missing trailing NUL, missing rename 2nd path, bad fields, byte limit)
    let no_trailing_nul = parse_porcelain_v2_status(b"? ok.rs\0? cut_mid_stream", 100);
    assert_eq!(no_trailing_nul.entries.len(), 1);
    assert_eq!(no_trailing_nul.malformed_records, 1);
    assert!(matches!(
        no_trailing_nul.first_issue,
        Some(ScmParseIssue::IncompleteRecord { .. })
    ));
    assert!(no_trailing_nul.blocks_bulk_actions());

    let missing_rename_orig =
        parse_porcelain_v2_status(b"2 R. N... 100644 100644 100644 111 222 R100 dst.rs\0", 100);
    assert!(missing_rename_orig.entries.is_empty());
    assert_eq!(missing_rename_orig.malformed_records, 1);
    assert!(matches!(
        missing_rename_orig.first_issue,
        Some(ScmParseIssue::IncompleteRecord { .. })
    ));

    let malformed_raw: Vec<u8> = [
        b"\0# branch.ab broken\0".as_slice(),
        b"1 .. N... 1 1 1 a b nope.rs\0",
        b"2 R. N... 1 1 1 a b X99 dst.rs\0?real_next_not_orig.rs\0",
        b"9 unknown\0",
        b"? survivor.rs\0",
    ]
    .concat();
    let malformed_mix = parse_porcelain_v2_status(&malformed_raw, 100);
    assert!(malformed_mix.malformed_records >= 4);
    assert!(matches!(
        malformed_mix.first_issue,
        Some(ScmParseIssue::MalformedRecord { .. })
    ));
    assert_eq!(malformed_mix.entries.len(), 1);
    assert_eq!(malformed_mix.entries[0].path, "survivor.rs");

    let byte_capped =
        parse_porcelain_v2_status_bounded(b"? first.rs\0? second_exceeds_limit.rs\0", 14, 100);
    assert!(byte_capped.input_bytes_truncated);
    assert!(byte_capped.truncated);
    assert!(matches!(
        byte_capped.first_issue,
        Some(ScmParseIssue::InputBytesExceeded { max_bytes: 14, .. })
    ));
    assert_eq!(byte_capped.entries.len(), 1);
    assert_eq!(byte_capped.entries[0].path, "first.rs");

    // 6. Entry cap (`max_entries = 3`): tracked & conflict entries prioritized ahead of untracked,
    //    AND rename (`2 R.`) AFTER hitting `max_entries` still consumes both NUL-terminated paths!
    let mut cap_stream = Vec::new();
    cap_stream.extend_from_slice(b"? u1.txt\0? u2.txt\0? u3.txt\0? u4.txt\0");
    // Now past max_entries=3: feed two `2 R.` records whose `<orig_path>` starts with `? ` if misparsed!
    cap_stream.extend_from_slice(
        b"2 R. N... 100644 100644 100644 111 222 R100 tracked_ren1.rs\0? trap_orig1.rs\0",
    );
    cap_stream.extend_from_slice(b"1 .M N... 100644 100644 100644 333 444 tracked_mod.rs\0");
    cap_stream.extend_from_slice(
        b"2 C. N... 100644 100644 100644 555 666 C100 tracked_copy_over_cap.rs\0? trap_orig2.rs\0",
    );
    cap_stream.extend_from_slice(b"u UU N... 100644 100644 100644 100644 a b c late_conflict.rs\0");

    let capped = parse_porcelain_v2_status(&cap_stream, 3);
    assert_eq!(capped.malformed_records, 0);
    assert_eq!(capped.non_utf8_paths, 0);
    assert!(capped.truncated);
    assert_eq!(capped.total_entries, 8);
    assert_eq!(capped.badge_count(true), 8);
    assert_eq!(capped.entries.len(), 3);
    // First 2 tracked entries + 1st untracked? Wait: there are 4 tracked entries total (ren1, mod, copy, conflict),
    // so all 3 slots go to the first 3 tracked entries in Git order, evicting all 4 untracked entries!
    let kept_paths: Vec<&str> = capped.entries.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(
        kept_paths,
        vec![
            "tracked_ren1.rs",
            "tracked_mod.rs",
            "tracked_copy_over_cap.rs"
        ]
    );

    // And when only 2 tracked entries exist with 4 untracked and max_entries=3: 1st untracked + 2 tracked in Git order!
    let mut mixed_cap = Vec::new();
    mixed_cap.extend_from_slice(b"? u_first.txt\0? u_evicted.txt\0");
    mixed_cap.extend_from_slice(b"1 M. N... 100644 100644 100644 111 222 t_one.rs\0");
    mixed_cap.extend_from_slice(b"? u_also_evicted.txt\0");
    mixed_cap.extend_from_slice(b"1 .D N... 100644 100644 000000 333 444 t_two.rs\0");
    let mixed_res = parse_porcelain_v2_status(&mixed_cap, 3);
    let mixed_paths: Vec<&str> = mixed_res.entries.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(mixed_paths, vec!["u_first.txt", "t_one.rs", "t_two.rs"]);
}

#[test]
fn empty_loading_not_repo_error_read_only_and_localization_states() {
    let ctx = egui::Context::default();
    let mut panel = ScmChangesPanel::new();

    // 1. No project selected (EN & FR) — zero actions emitted on plain render
    for (french, expected) in [
        (false, "This workspace has no project folder"),
        (true, "Cet espace de travail n'a pas de dossier de projet"),
    ] {
        let (act, out) = run_panel_frame(
            &ctx,
            &mut panel,
            320.0,
            Vec::new(),
            true,
            true,
            ScmChangesInput {
                project_selected: false,
                loaded: false,
                busy: false,
                read_only: false,
                french,
                error: None,
                status: None,
                active_entry: None,
                capabilities: full_caps(),
            },
        );
        assert_eq!(act, None);
        assert!(has_substring_text(&out.shapes, expected));
    }

    // 2. Loading (`!loaded && busy`) vs on-demand idle (`!loaded && !busy`) states (EN & FR)
    for (busy, french, expected_msg) in [
        (true, false, "Reading git status…"),
        (true, true, "Lecture du statut git…"),
        (false, false, "Refresh to load changes"),
        (false, true, "Actualisez pour lire les modifications"),
    ] {
        let (act, out_loading) = run_panel_frame(
            &ctx,
            &mut panel,
            320.0,
            Vec::new(),
            true,
            true,
            ScmChangesInput {
                project_selected: true,
                loaded: false,
                busy,
                read_only: false,
                french,
                error: None,
                status: None,
                active_entry: None,
                capabilities: full_caps(),
            },
        );
        assert_eq!(act, None);
        assert!(
            has_substring_text(&out_loading.shapes, expected_msg),
            "expected {expected_msg:?} for busy={busy}, french={french}"
        );
    }

    // 3. Error state: `can_refresh: false` hides Retry; `can_refresh: true` clicks Retry -> Refresh
    let mut no_refresh_caps = full_caps();
    no_refresh_caps.can_refresh = false;
    let (_, out_err_no_retry) = run_panel_frame(
        &ctx,
        &mut panel,
        320.0,
        Vec::new(),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: Some("permission denied"),
            status: None,
            active_entry: None,
            capabilities: no_refresh_caps,
        },
    );
    assert!(has_substring_text(
        &out_err_no_retry.shapes,
        "Could not read the git status: permission denied"
    ));
    assert!(!has_exact_text(&out_err_no_retry.shapes, "Retry"));

    let (_, out_err_fr) = run_panel_frame(
        &ctx,
        &mut panel,
        320.0,
        Vec::new(),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: true,
            error: Some("accès refusé"),
            status: None,
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    assert!(has_substring_text(
        &out_err_fr.shapes,
        "Impossible de lire le statut git : accès refusé"
    ));
    let retry_rect = find_text_rect(&out_err_fr.shapes, "Réessayer").expect("Réessayer button");
    let (retry_act, _) = run_panel_frame(
        &ctx,
        &mut panel,
        320.0,
        click_events(retry_rect.center()),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: true,
            error: Some("accès refusé"),
            status: None,
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    assert_eq!(retry_act, Some(ScmChangesAction::Refresh));

    // 4. Not a git repository
    let not_repo_snap = ScmStatusSnapshot {
        is_repo: false,
        branch: None,
        upstream: None,
        ahead: 0,
        behind: 0,
        entries: &[],
        truncated: false,
        parse_incomplete: false,
        total_entries: 0,
    };
    let (act, out_not_repo) = run_panel_frame(
        &ctx,
        &mut panel,
        320.0,
        Vec::new(),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: true,
            error: None,
            status: Some(not_repo_snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    assert_eq!(act, None);
    assert!(has_substring_text(
        &out_not_repo.shapes,
        "Ce dossier n'est pas dans un dépôt git."
    ));

    // 5. Clean repo across all 7 themes + light mode, detached HEAD, and read-only Live Share banner
    let entries_ro = vec![ScmEntry {
        path: "src/lib.rs".to_string(),
        old_path: None,
        area: ScmArea::Unstaged,
        status: ScmFileStatus::Modified,
        conflict: None,
        added: Some(4),
        removed: Some(1),
    }];
    for &theme_id in &THEMES {
        theme::apply(&ctx, true, Some(theme_id));
        let snap = ScmStatusSnapshot {
            is_repo: true,
            branch: None,
            upstream: None,
            ahead: 0,
            behind: 0,
            entries: &[],
            truncated: false,
            parse_incomplete: false,
            total_entries: 0,
        };
        let (act, out_clean) = run_panel_frame(
            &ctx,
            &mut panel,
            320.0,
            Vec::new(),
            true,
            true,
            ScmChangesInput {
                project_selected: true,
                loaded: true,
                busy: false,
                read_only: false,
                french: false,
                error: None,
                status: Some(snap),
                active_entry: None,
                capabilities: full_caps(),
            },
        );
        assert_eq!(act, None);
        assert!(has_exact_text(&out_clean.shapes, "Detached HEAD"));
        assert!(has_exact_text(&out_clean.shapes, "No changes"));
    }

    theme::apply(&ctx, false, None);
    let ro_snap = ScmStatusSnapshot {
        is_repo: true,
        branch: Some("main"),
        upstream: Some("origin/main"),
        ahead: 2,
        behind: 1,
        entries: &entries_ro,
        truncated: false,
        parse_incomplete: false,
        total_entries: 1,
    };
    let (ro_act, out_ro) = run_panel_frame(
        &ctx,
        &mut panel,
        320.0,
        Vec::new(),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: true,
            french: true,
            error: None,
            status: Some(ro_snap),
            active_entry: Some((ScmArea::Unstaged, "src/lib.rs")),
            capabilities: full_caps(),
        },
    );
    assert_eq!(ro_act, None);
    assert!(has_substring_text(&out_ro.shapes, "Lecture seule"));
    assert_eq!(count_a11y_buttons_with_label(&out_ro, "Indexer"), 0);
    assert_eq!(
        count_a11y_buttons_with_label(&out_ro, "Abandonner les modifications"),
        0
    );
    // Strict omission of unwired subsystems (`Create PR`, `AI Generate`, `Notes`, `History`)
    for forbidden in ["Create PR", "Créer PR", "Notes", "Generate"] {
        assert!(!has_exact_text(&out_ro.shapes, forbidden));
    }
}

#[test]
fn sections_status_badges_compacted_tree_list_toggle_and_filter_bar() {
    let ctx = egui::Context::default();
    theme::apply(&ctx, true, Some(ThemeId::Classic));
    let mut panel = ScmChangesPanel::default();
    assert_eq!(panel.view_mode(), ScmViewMode::Tree);
    assert_eq!(panel.group_order(), ScmGroupOrder::ChangesFirst);

    let entries = vec![
        ScmEntry {
            path: "src/renderer/components/file10.rs".to_string(),
            old_path: None,
            area: ScmArea::Unstaged,
            status: ScmFileStatus::Modified,
            conflict: None,
            added: Some(12),
            removed: Some(3),
        },
        ScmEntry {
            path: "src/renderer/components/file2.rs".to_string(),
            old_path: None,
            area: ScmArea::Unstaged,
            status: ScmFileStatus::Deleted,
            conflict: None,
            added: None,
            removed: Some(9),
        },
        ScmEntry {
            path: "crates/core/added.rs".to_string(),
            old_path: None,
            area: ScmArea::Staged,
            status: ScmFileStatus::Added,
            conflict: None,
            added: Some(30),
            removed: None,
        },
        ScmEntry {
            path: "crates/core/renamed.rs".to_string(),
            old_path: Some("crates/core/old.rs".to_string()),
            area: ScmArea::Staged,
            status: ScmFileStatus::Renamed,
            conflict: None,
            added: None,
            removed: None,
        },
        ScmEntry {
            path: "crates/core/copied.rs".to_string(),
            old_path: Some("crates/core/base.rs".to_string()),
            area: ScmArea::Staged,
            status: ScmFileStatus::Copied,
            conflict: None,
            added: None,
            removed: None,
        },
        ScmEntry {
            path: "scratch/new_note.txt".to_string(),
            old_path: None,
            area: ScmArea::Untracked,
            status: ScmFileStatus::Untracked,
            conflict: None,
            added: None,
            removed: None,
        },
        ScmEntry {
            path: "src/conflict.rs".to_string(),
            old_path: None,
            area: ScmArea::Unstaged,
            status: ScmFileStatus::Modified,
            conflict: Some(ScmConflictKind::BothModified),
            added: None,
            removed: None,
        },
    ];

    // Verify status codes, keys, titles, and exact hex colors (`sourceControl.css:6-12`)
    assert_eq!(ScmFileStatus::Modified.code(), "M");
    assert_eq!(ScmFileStatus::Modified.color(), GIT_COLOR_MODIFIED);
    assert_eq!(ScmFileStatus::Modified.title(ScmLang::Fr), "Modifié");
    assert_eq!(ScmFileStatus::Added.code(), "A");
    assert_eq!(ScmFileStatus::Added.color(), GIT_COLOR_ADDED);
    assert_eq!(ScmFileStatus::Deleted.code(), "D");
    assert_eq!(ScmFileStatus::Deleted.color(), GIT_COLOR_DELETED);
    assert_eq!(ScmFileStatus::Renamed.code(), "R");
    assert_eq!(ScmFileStatus::Renamed.color(), GIT_COLOR_RENAMED);
    assert_eq!(ScmFileStatus::Copied.code(), "C");
    assert_eq!(ScmFileStatus::Copied.color(), GIT_COLOR_RENAMED);
    assert_eq!(ScmFileStatus::Untracked.code(), "U");
    assert_eq!(ScmFileStatus::Untracked.color(), GIT_COLOR_RENAMED);

    let snap = ScmStatusSnapshot {
        is_repo: true,
        branch: Some("feat/local-only"),
        upstream: None,
        ahead: 0,
        behind: 0,
        entries: &entries,
        truncated: false,
        parse_incomplete: false,
        total_entries: entries.len(),
    };

    let (_, out_tree) = run_panel_frame(
        &ctx,
        &mut panel,
        360.0,
        Vec::new(),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );

    // Unpublished branch pill + conflict alert + all 4 sections rendered
    assert!(has_exact_text(&out_tree.shapes, "unpublished"));
    assert!(has_substring_text(
        &out_tree.shapes,
        "Conflicts: 1 unresolved"
    ));
    assert!(has_exact_text(&out_tree.shapes, "Conflicts"));
    assert!(has_exact_text(&out_tree.shapes, "Changes"));
    assert!(has_exact_text(&out_tree.shapes, "Staged Changes"));
    assert!(has_exact_text(&out_tree.shapes, "Untracked Files"));

    // Single-child folder chain `src/renderer/components` compacted into one directory row
    assert!(has_exact_text(&out_tree.shapes, "src/renderer/components"));
    assert!(has_exact_text(&out_tree.shapes, "crates/core"));

    // Natural numeric sort: `file2.rs` appears vertically ABOVE `file10.rs`
    let f2_rect = find_text_rect(&out_tree.shapes, "file2.rs").expect("file2.rs");
    let f10_rect = find_text_rect(&out_tree.shapes, "file10.rs").expect("file10.rs");
    assert!(
        f2_rect.min.y < f10_rect.min.y,
        "natural numeric sort expected file2.rs ({}) above file10.rs ({})",
        f2_rect.min.y,
        f10_rect.min.y
    );

    // Exact status badge colors painted
    let painted = painted_texts(&out_tree.shapes);
    assert!(
        painted
            .iter()
            .any(|(t, c, _, _)| t == "M" && *c == GIT_COLOR_MODIFIED)
    );
    assert!(
        painted
            .iter()
            .any(|(t, c, _, _)| t == "A" && *c == GIT_COLOR_ADDED)
    );
    assert!(
        painted
            .iter()
            .any(|(t, c, _, _)| t == "D" && *c == GIT_COLOR_DELETED)
    );
    assert!(
        painted
            .iter()
            .any(|(t, c, _, _)| t == "R" && *c == GIT_COLOR_RENAMED)
    );
    assert!(has_exact_text(&out_tree.shapes, "+12"));
    assert!(has_exact_text(&out_tree.shapes, "-3"));

    // Group order presets (`StagedFirst` & `UntrackedFirst`) reorder sections after `Conflicts`
    panel.set_group_order(ScmGroupOrder::StagedFirst);
    let (_, out_staged_first) = run_panel_frame(
        &ctx,
        &mut panel,
        360.0,
        Vec::new(),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    let staged_hdr = find_text_rect(&out_staged_first.shapes, "Staged Changes").unwrap();
    let changes_hdr = find_text_rect(&out_staged_first.shapes, "Changes").unwrap();
    assert!(staged_hdr.min.y < changes_hdr.min.y);

    panel.set_group_order(ScmGroupOrder::UntrackedFirst);
    let (_, out_untracked_first) = run_panel_frame(
        &ctx,
        &mut panel,
        360.0,
        Vec::new(),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    let untracked_hdr = find_text_rect(&out_untracked_first.shapes, "Untracked Files").unwrap();
    let changes_hdr2 = find_text_rect(&out_untracked_first.shapes, "Changes").unwrap();
    assert!(untracked_hdr.min.y < changes_hdr2.min.y);
    panel.set_group_order(ScmGroupOrder::ChangesFirst);
    let (_, out_tree_restored) = run_panel_frame(
        &ctx,
        &mut panel,
        360.0,
        Vec::new(),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );

    // Click compacted folder row -> collapses directory; pruned automatically if entries change
    let dir_pos = find_text_rect(&out_tree_restored.shapes, "src/renderer/components")
        .unwrap()
        .center();
    let _ = run_panel_frame(
        &ctx,
        &mut panel,
        360.0,
        click_events(dir_pos),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    assert!(panel.is_dir_collapsed("dir::unstaged::src/renderer/components"));

    // Toggle `View as list` from header button -> renders flat rows with parent dir hints
    let view_btn = find_a11y_node_center(&out_tree, "View as list").expect("View as list button");
    let (_, out_list) = run_panel_frame(
        &ctx,
        &mut panel,
        360.0,
        click_events(view_btn),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    assert_eq!(panel.view_mode(), ScmViewMode::List);
    assert!(has_exact_text(&out_list.shapes, "file2.rs"));
    assert!(has_exact_text(&out_list.shapes, "src/renderer/components"));
    panel.set_view_mode(ScmViewMode::Tree);

    // Filter bar: clamp, filter by old_path/path, hide bulk buttons while filtered, Escape collapse, Clear button
    let overlong_filter = "x".repeat(MAX_FILTER_CHARS + 40);
    panel.set_filter_query(&overlong_filter);
    assert_eq!(panel.filter_query().chars().count(), MAX_FILTER_CHARS);
    assert!(panel.is_filter_expanded());
    let (_, out_no_match) = run_panel_frame(
        &ctx,
        &mut panel,
        360.0,
        Vec::new(),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: true,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    assert!(has_exact_text(
        &out_no_match.shapes,
        "Aucun fichier correspondant"
    ));

    // Match rename via `old_path` (`crates/core/old.rs`), and ensure bulk section buttons are hidden while filtered
    panel.set_filter_query("old.rs");
    let (_, out_filtered) = run_panel_frame(
        &ctx,
        &mut panel,
        360.0,
        Vec::new(),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    assert!(has_exact_text(&out_filtered.shapes, "renamed.rs"));
    assert_eq!(
        count_a11y_buttons_with_label(&out_filtered, "Unstage all"),
        0
    );

    // Click `Clear and close filter` (`×`)
    let clear_pos =
        find_a11y_node_center(&out_filtered, "Clear and close filter").expect("clear filter btn");
    let _ = run_panel_frame(
        &ctx,
        &mut panel,
        360.0,
        click_events(clear_pos),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    assert!(!panel.is_filter_expanded());
    assert!(panel.filter_query().is_empty());

    // Expand filter via header icon, then press fresh `Escape` while focused -> collapses filter
    let (_, out_hdr) = run_panel_frame(
        &ctx,
        &mut panel,
        360.0,
        Vec::new(),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    let filter_btn =
        find_a11y_node_center(&out_hdr, "Filter files by name").expect("filter toggle btn");
    let _ = run_panel_frame(
        &ctx,
        &mut panel,
        360.0,
        click_events(filter_btn),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    assert!(panel.is_filter_expanded());
    // Frame 1 focuses filter input (`focus_filter_next`); Frame 2 sends fresh `Escape`
    let _ = run_panel_frame(
        &ctx,
        &mut panel,
        360.0,
        Vec::new(),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    let _ = run_panel_frame(
        &ctx,
        &mut panel,
        360.0,
        vec![egui::Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    assert!(!panel.is_filter_expanded());
}

#[test]
fn strict_capabilities_truncation_guards_request_discard_and_remote_honesty() {
    let ctx = egui::Context::default();
    let mut panel = ScmChangesPanel::new();

    let raw: Vec<u8> = [
        b"# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -0\0".as_slice(),
        b"2 R. N... 100644 100644 100644 111 222 R100 src/new.rs\0src/old.rs\0",
        b"1 .M N... 100644 100644 100644 333 444 src/mod.rs\0",
        b"1 .D N... 100644 100644 000000 555 666 src/gone.rs\0",
        b"? untracked.txt\0",
    ]
    .concat();
    let parsed = parse_porcelain_v2_status(&raw, STATUS_ENTRIES_MAX);
    let snap = ScmStatusSnapshot::from_parsed(true, &parsed);

    // 1. Default capabilities (all `false`) expose zero mutation/refresh/commit/remote controls
    let default_caps = ScmCapabilities::default();
    let (act0, out_zero) = run_panel_frame(
        &ctx,
        &mut panel,
        340.0,
        Vec::new(),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: default_caps,
        },
    );
    assert_eq!(act0, None);
    for hidden_lbl in [
        "Refresh",
        "Stage",
        "Unstage",
        "Discard changes",
        "Commit",
        "Stage All",
        "More commit and remote actions",
    ] {
        assert_eq!(
            count_a11y_buttons_with_label(&out_zero, hidden_lbl),
            0,
            "{hidden_lbl} must be omitted when capability is false"
        );
    }

    // 2. Fallback from `OpenDiff` to `OpenFile` when `can_open_diff == false && can_open_file == true`
    let open_file_only = ScmCapabilities {
        can_open_file: true,
        ..Default::default()
    };
    let (_, out_of) = run_panel_frame(
        &ctx,
        &mut panel,
        340.0,
        Vec::new(),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: open_file_only,
        },
    );
    let mod_pos = find_text_rect(&out_of.shapes, "mod.rs").unwrap().center();
    let (of_act, _) = run_panel_frame(
        &ctx,
        &mut panel,
        340.0,
        click_events(mod_pos),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: open_file_only,
        },
    );
    assert_eq!(
        of_act,
        Some(ScmChangesAction::OpenFile("src/mod.rs".to_string()))
    );

    // Deleted file does NOT emit `OpenFile` when `can_open_diff == false`
    let gone_pos = find_text_rect(&out_of.shapes, "gone.rs").unwrap().center();
    let (gone_act, _) = run_panel_frame(
        &ctx,
        &mut panel,
        340.0,
        click_events(gone_pos),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: open_file_only,
        },
    );
    assert_eq!(gone_act, None);

    // 3. With `full_caps()`: single click on renamed file -> `OpenDiff { pinned: false, old_path: Some(...) }`
    let (_, out_full) = run_panel_frame(
        &ctx,
        &mut panel,
        340.0,
        Vec::new(),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    let ren_pos = find_text_rect(&out_full.shapes, "new.rs").unwrap().center();
    let (diff_act, _) = run_panel_frame(
        &ctx,
        &mut panel,
        340.0,
        click_events(ren_pos),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    assert_eq!(
        diff_act,
        Some(ScmChangesAction::OpenDiff {
            area: ScmArea::Staged,
            path: "src/new.rs".to_string(),
            old_path: Some("src/old.rs".to_string()),
            pinned: false,
        })
    );

    // Per-row `Unstage`, `Stage`, and `RequestDiscard` + confirmation copy helpers
    let unstage_pos = find_a11y_node_center(&out_full, "Unstage").expect("Unstage button");
    let (unst_act, _) = run_panel_frame(
        &ctx,
        &mut panel,
        340.0,
        click_events(unstage_pos),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    assert_eq!(
        unst_act,
        Some(ScmChangesAction::Unstage(vec!["src/new.rs".to_string()]))
    );

    let del_untracked_pos =
        find_a11y_node_center(&out_full, "Delete untracked file").expect("Delete untracked btn");
    let (disc_act, _) = run_panel_frame(
        &ctx,
        &mut panel,
        340.0,
        click_events(del_untracked_pos),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    assert_eq!(
        disc_act,
        Some(ScmChangesAction::RequestDiscard {
            area: ScmArea::Untracked,
            paths: vec!["untracked.txt".to_string()],
        })
    );

    // Verify `discard_copy` and `discard_area_copy` EN/FR local & remote confirmation copy
    let mod_entry = parsed
        .entries
        .iter()
        .find(|e| e.path == "src/mod.rs")
        .unwrap();
    assert_eq!(
        mod_entry.discard_copy(false, false).confirm_label,
        "Discard"
    );
    assert_eq!(
        mod_entry.discard_copy(false, true).confirm_label,
        "Abandonner"
    );
    let gone_entry = parsed
        .entries
        .iter()
        .find(|e| e.path == "src/gone.rs")
        .unwrap();
    assert_eq!(
        gone_entry.discard_copy(false, false).confirm_label,
        "Restore"
    );
    let untr_entry = parsed
        .entries
        .iter()
        .find(|e| e.path == "untracked.txt")
        .unwrap();
    assert!(
        untr_entry
            .discard_copy(true, true)
            .description
            .contains("hôte distant")
    );
    assert_eq!(
        discard_area_copy(ScmArea::Untracked, 3, false, false).confirm_label,
        "Delete 3"
    );
    assert_eq!(
        discard_area_copy(ScmArea::Untracked, 1, true, true).confirm_label,
        "Supprimer"
    );
    assert_eq!(
        discard_area_copy(ScmArea::Unstaged, 2, false, true).confirm_label,
        "Tout abandonner"
    );

    // 4. `truncated: true` & `parse_incomplete: true` strictly block bulk actions + disable `Stage All`,
    // qualifying `total_entries` as a minimum (`at least N detected`) when input is truncated/incomplete
    // vs exact (`showing the first M of N`) when the full git stream was parsed cleanly.
    let only_unstaged = vec![mod_entry.clone(), untr_entry.clone()];
    let exact_cap_snap = ScmStatusSnapshot {
        is_repo: true,
        branch: Some("main"),
        upstream: None,
        ahead: 0,
        behind: 0,
        entries: &only_unstaged,
        truncated: true,
        parse_incomplete: false,
        total_entries: 6_200,
    };
    let (_, out_exact_cap) = run_panel_frame(
        &ctx,
        &mut panel,
        340.0,
        Vec::new(),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(exact_cap_snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    assert!(has_substring_text(
        &out_exact_cap.shapes,
        "showing the first 2 of 6200"
    ));

    let mut ext_trunc_parsed = parsed.clone();
    ext_trunc_parsed.mark_input_truncated();
    let ext_trunc_snap = ScmStatusSnapshot::from_parsed(true, &ext_trunc_parsed);
    let (_, out_ext_trunc) = run_panel_frame(
        &ctx,
        &mut panel,
        340.0,
        Vec::new(),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: true,
            error: None,
            status: Some(ext_trunc_snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    assert!(has_substring_text(
        &out_ext_trunc.shapes,
        "au moins 4 détectées"
    ));

    let trunc_snap = ScmStatusSnapshot {
        is_repo: true,
        branch: Some("main"),
        upstream: None,
        ahead: 0,
        behind: 0,
        entries: &only_unstaged,
        truncated: true,
        parse_incomplete: true,
        total_entries: 6_200,
    };
    let (_, out_trunc) = run_panel_frame(
        &ctx,
        &mut panel,
        340.0,
        Vec::new(),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(trunc_snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    assert!(has_substring_text(
        &out_trunc.shapes,
        "showing the first 2 (at least 6200 detected)"
    ));
    assert!(has_substring_text(
        &out_trunc.shapes,
        "bulk actions are disabled for safety"
    ));
    assert_eq!(count_a11y_buttons_with_label(&out_trunc, "Stage all"), 0);
    assert_eq!(count_a11y_buttons_with_label(&out_trunc, "Discard all"), 0);
    assert_eq!(count_a11y_buttons_with_label(&out_trunc, "Stage folder"), 0);
    // Primary `Stage All` button is disabled when truncated: clicking it emits `None`
    let prim_stage_rect = find_text_rect(&out_trunc.shapes, "Stage All").expect("Stage All label");
    let (trunc_click_act, _) = run_panel_frame(
        &ctx,
        &mut panel,
        340.0,
        click_events(prim_stage_rect.center()),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(trunc_snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    assert_eq!(trunc_click_act, None);

    // 5. Explicit `can_remote: true` (never automatic; emits `Remote` only on user click)
    let mut remote_caps = full_caps();
    remote_caps.can_remote = true;
    for (upstream, ahead, behind, expected_lbl, expected_op) in [
        (None, 0, 0, "Publish Branch", ScmRemoteOp::Publish),
        (Some("origin/main"), 2, 0, "Push", ScmRemoteOp::Push),
        (Some("origin/main"), 0, 3, "Pull", ScmRemoteOp::Pull),
        (Some("origin/main"), 1, 2, "Sync", ScmRemoteOp::Sync),
    ] {
        let rsnap = ScmStatusSnapshot {
            is_repo: true,
            branch: Some("main"),
            upstream,
            ahead,
            behind,
            entries: &[],
            truncated: false,
            parse_incomplete: false,
            total_entries: 0,
        };
        let (idle_act, out_rem) = run_panel_frame(
            &ctx,
            &mut panel,
            340.0,
            Vec::new(),
            true,
            true,
            ScmChangesInput {
                project_selected: true,
                loaded: true,
                busy: false,
                read_only: false,
                french: false,
                error: None,
                status: Some(rsnap),
                active_entry: None,
                capabilities: remote_caps,
            },
        );
        assert_eq!(idle_act, None, "never trigger Remote automatically");
        let btn_pos = find_a11y_node_center(&out_rem, expected_lbl).expect(expected_lbl);
        let (clicked_act, _) = run_panel_frame(
            &ctx,
            &mut panel,
            340.0,
            click_events(btn_pos),
            true,
            true,
            ScmChangesInput {
                project_selected: true,
                loaded: true,
                busy: false,
                read_only: false,
                french: false,
                error: None,
                status: Some(rsnap),
                active_entry: None,
                capabilities: remote_caps,
            },
        );
        assert_eq!(clicked_act, Some(ScmChangesAction::Remote(expected_op)));
    }

    // Also exercise `FastForward` and `Fetch` from the explicit remote chevron menu
    let ff_snap = ScmStatusSnapshot {
        is_repo: true,
        branch: Some("main"),
        upstream: Some("origin/main"),
        ahead: 0,
        behind: 1,
        entries: &[],
        truncated: false,
        parse_incomplete: false,
        total_entries: 0,
    };
    for (menu_lbl, expected_op) in [
        ("Fast-forward", ScmRemoteOp::FastForward),
        ("Fetch", ScmRemoteOp::Fetch),
    ] {
        let (_, out_before_menu) = run_panel_frame(
            &ctx,
            &mut panel,
            340.0,
            Vec::new(),
            true,
            true,
            ScmChangesInput {
                project_selected: true,
                loaded: true,
                busy: false,
                read_only: false,
                french: false,
                error: None,
                status: Some(ff_snap),
                active_entry: None,
                capabilities: remote_caps,
            },
        );
        let chev = find_a11y_node_center(&out_before_menu, "More commit and remote actions")
            .expect("remote chevron");
        let (_, out_menu_open) = run_panel_frame(
            &ctx,
            &mut panel,
            340.0,
            click_events(chev),
            true,
            true,
            ScmChangesInput {
                project_selected: true,
                loaded: true,
                busy: false,
                read_only: false,
                french: false,
                error: None,
                status: Some(ff_snap),
                active_entry: None,
                capabilities: remote_caps,
            },
        );
        let item_pos = find_a11y_node_center(&out_menu_open, menu_lbl).expect(menu_lbl);
        let (menu_act, _) = run_panel_frame(
            &ctx,
            &mut panel,
            340.0,
            click_events(item_pos),
            true,
            true,
            ScmChangesInput {
                project_selected: true,
                loaded: true,
                busy: false,
                read_only: false,
                french: false,
                error: None,
                status: Some(ff_snap),
                active_entry: None,
                capabilities: remote_caps,
            },
        );
        assert_eq!(menu_act, Some(ScmChangesAction::Remote(expected_op)));
    }
}

#[test]
fn commit_composer_ctrl_enter_anti_repeat_focus_isolation_popup_and_narrow_bounds() {
    let ctx = egui::Context::default();
    let mut panel = ScmChangesPanel::new();

    // 1. `20_000`-char clamp on multi-byte UTF-8 commit message + clear/reset
    let huge = "é".repeat(MAX_COMMIT_MESSAGE_CHARS + 250);
    panel.set_commit_message(&huge);
    assert_eq!(
        panel.commit_message().chars().count(),
        MAX_COMMIT_MESSAGE_CHARS
    );
    panel.clear_commit_message();
    assert!(panel.commit_message().is_empty());

    let staged_entries = vec![
        ScmEntry {
            path: "src/deeply/nested/module/very_long_component_filename_for_narrow_panel.rs"
                .to_string(),
            old_path: None,
            area: ScmArea::Staged,
            status: ScmFileStatus::Modified,
            conflict: None,
            added: Some(5),
            removed: Some(2),
        },
        ScmEntry {
            path: "src/second.rs".to_string(),
            old_path: None,
            area: ScmArea::Unstaged,
            status: ScmFileStatus::Modified,
            conflict: None,
            added: None,
            removed: None,
        },
    ];
    let snap = ScmStatusSnapshot {
        is_repo: true,
        branch: Some("feat/very-long-branch-name-in-narrow-side-panel"),
        upstream: Some("origin/feat/very-long-branch-name-in-narrow-side-panel"),
        ahead: 1,
        behind: 1,
        entries: &staged_entries,
        truncated: false,
        parse_incomplete: false,
        total_entries: 2,
    };

    // 2. Narrow `260px` SidePanel layout check: all painted text stays clipped within `[0, 260]`
    let (_, out_260) = run_panel_frame(
        &ctx,
        &mut panel,
        260.0,
        Vec::new(),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: true,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    for (txt, _, rect, clip) in painted_texts(&out_260.shapes) {
        let visible = rect.intersect(clip);
        if visible.is_positive() {
            assert!(
                visible.max.x <= 261.5,
                "text `{txt}` overflowed 260px SidePanel: visible.max.x={}",
                visible.max.x
            );
        }
    }

    // 3. Focus commit box, verify held `Ctrl+Enter` commits ONCE (without trailing `\n`) and ignores repeats
    panel.set_commit_message("feat: native scm panel");
    let (_, out_ready) = run_panel_frame(
        &ctx,
        &mut panel,
        340.0,
        Vec::new(),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    let msg_rect =
        find_text_rect(&out_ready.shapes, "feat: native scm panel").expect("commit message galley");
    // Click commit textarea to focus it
    let _ = run_panel_frame(
        &ctx,
        &mut panel,
        340.0,
        click_events(msg_rect.center()),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );

    // Fresh `Ctrl+Enter` -> commits once, message stays clean (no `\n` injected)
    let (commit_act1, _) = run_panel_frame(
        &ctx,
        &mut panel,
        340.0,
        vec![egui::Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::CTRL,
        }],
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    assert_eq!(
        commit_act1,
        Some(ScmChangesAction::Commit {
            message: "feat: native scm panel".to_string(),
        })
    );
    assert_eq!(panel.commit_message(), "feat: native scm panel");

    // Held `Ctrl+Enter` (`repeat: true` AND even synthetic `repeat: false` while `Enter` still held) -> `None`
    for repeat_flag in [true, false] {
        let (held_act, _) = run_panel_frame(
            &ctx,
            &mut panel,
            340.0,
            vec![egui::Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: repeat_flag,
                modifiers: Modifiers::CTRL,
            }],
            true,
            true,
            ScmChangesInput {
                project_selected: true,
                loaded: true,
                busy: false,
                read_only: false,
                french: false,
                error: None,
                status: Some(snap),
                active_entry: None,
                capabilities: full_caps(),
            },
        );
        assert_eq!(
            held_act, None,
            "held Ctrl+Enter (repeat={repeat_flag}) must never double-commit"
        );
    }

    // Release `Key::Enter`, then press fresh `Ctrl+Enter` -> commits again
    let _ = run_panel_frame(
        &ctx,
        &mut panel,
        340.0,
        vec![egui::Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Modifiers::CTRL,
        }],
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    let (commit_act2, _) = run_panel_frame(
        &ctx,
        &mut panel,
        340.0,
        vec![
            egui::Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::CTRL,
            },
            egui::Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: Modifiers::CTRL,
            },
        ],
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    assert!(matches!(commit_act2, Some(ScmChangesAction::Commit { .. })));

    // 4. External focus isolation + disabled / unfocused / Popup guards + focus-scoped list nav
    let external_id = egui::Id::new("external-terminal-input");
    let mut external_buf = String::from("git status");
    let mut ext_captured = None;
    let raw_ext = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 760.0))),
        focused: true,
        events: vec![
            egui::Event::Key {
                key: Key::ArrowDown,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            },
            egui::Event::Key {
                key: Key::ArrowDown,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: Modifiers::NONE,
            },
        ],
        ..Default::default()
    };
    ctx.memory_mut(|m| m.request_focus(external_id));
    let raw_ext_init = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 760.0))),
        focused: true,
        ..Default::default()
    };
    let _ = ctx.run(raw_ext_init, |ctx| {
        egui::SidePanel::left("ext").show(ctx, |ui| {
            ui.add(egui::TextEdit::multiline(&mut external_buf).id(external_id));
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            let _ = panel.show(
                ui,
                ScmChangesInput {
                    project_selected: true,
                    loaded: true,
                    busy: false,
                    read_only: false,
                    french: false,
                    error: None,
                    status: Some(snap),
                    active_entry: None,
                    capabilities: full_caps(),
                },
            );
        });
    });
    let _ = ctx.run(raw_ext, |ctx| {
        egui::SidePanel::left("ext").show(ctx, |ui| {
            ui.add(egui::TextEdit::multiline(&mut external_buf).id(external_id));
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            ext_captured = panel.show(
                ui,
                ScmChangesInput {
                    project_selected: true,
                    loaded: true,
                    busy: false,
                    read_only: false,
                    french: false,
                    error: None,
                    status: Some(snap),
                    active_entry: None,
                    capabilities: full_caps(),
                },
            );
        });
    });
    assert_eq!(ext_captured, None);
    assert_eq!(ctx.memory(|m| m.focused()), Some(external_id));

    // Focus-scoped section/row keyboard nav: click `Changes` header to focus it, `Enter` collapses it!
    let (_, out_nav0) = run_panel_frame(
        &ctx,
        &mut panel,
        340.0,
        Vec::new(),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    let changes_pos = find_text_rect(&out_nav0.shapes, "Changes")
        .unwrap()
        .center();
    assert!(!panel.is_section_collapsed(ScmSectionKind::Unstaged));
    let _ = run_panel_frame(
        &ctx,
        &mut panel,
        340.0,
        click_events(changes_pos),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    assert!(panel.is_section_collapsed(ScmSectionKind::Unstaged));

    // Disabled (`enabled = false`) and unfocused (`focused = false`) ignore `ArrowRight` expand
    for (focused_flag, enabled_flag) in [(false, true), (true, false)] {
        let _ = run_panel_frame(
            &ctx,
            &mut panel,
            340.0,
            vec![
                egui::Event::Key {
                    key: Key::ArrowRight,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                },
                egui::Event::Key {
                    key: Key::ArrowRight,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                },
            ],
            focused_flag,
            enabled_flag,
            ScmChangesInput {
                project_selected: true,
                loaded: true,
                busy: false,
                read_only: false,
                french: false,
                error: None,
                status: Some(snap),
                active_entry: None,
                capabilities: full_caps(),
            },
        );
        assert!(panel.is_section_collapsed(ScmSectionKind::Unstaged));
    }

    // Open `egui::Popup` blocks `ArrowRight` expand as well
    let popup_id = egui::Id::new("test-open-popup");
    egui::Popup::open_id(&ctx, popup_id);
    let raw_pop = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(340.0, 760.0))),
        focused: true,
        events: vec![
            egui::Event::Key {
                key: Key::ArrowRight,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            },
            egui::Event::Key {
                key: Key::ArrowRight,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: Modifiers::NONE,
            },
        ],
        ..Default::default()
    };
    let _ = ctx.run(raw_pop, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let anchor = ui.button("anchor");
            let _ = egui::Popup::from_response(&anchor).id(popup_id).show(|ui| {
                ui.label("popup body");
            });
            let _ = panel.show(
                ui,
                ScmChangesInput {
                    project_selected: true,
                    loaded: true,
                    busy: false,
                    read_only: false,
                    french: false,
                    error: None,
                    status: Some(snap),
                    active_entry: None,
                    capabilities: full_caps(),
                },
            );
        });
    });
    assert!(panel.is_section_collapsed(ScmSectionKind::Unstaged));
    egui::Popup::close_all(&ctx);

    // Re-focus `Changes` header and use `ArrowRight` -> expands section, `ArrowDown` -> directory `src`, `ArrowDown` -> `second.rs`, `Enter` -> `OpenDiff`!
    let (_, out_collapsed) = run_panel_frame(
        &ctx,
        &mut panel,
        340.0,
        Vec::new(),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    let ch_pos2 = find_text_rect(&out_collapsed.shapes, "Changes")
        .unwrap()
        .center();
    // Click re-expands AND focuses `Changes` section header
    let _ = run_panel_frame(
        &ctx,
        &mut panel,
        340.0,
        click_events(ch_pos2),
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    assert!(!panel.is_section_collapsed(ScmSectionKind::Unstaged));

    for key in [Key::ArrowDown, Key::ArrowDown] {
        let _ = run_panel_frame(
            &ctx,
            &mut panel,
            340.0,
            vec![
                egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                },
                egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                },
            ],
            true,
            true,
            ScmChangesInput {
                project_selected: true,
                loaded: true,
                busy: false,
                read_only: false,
                french: false,
                error: None,
                status: Some(snap),
                active_entry: None,
                capabilities: full_caps(),
            },
        );
    }
    let (enter_file_act, _) = run_panel_frame(
        &ctx,
        &mut panel,
        340.0,
        vec![
            egui::Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            },
            egui::Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: Modifiers::NONE,
            },
        ],
        true,
        true,
        ScmChangesInput {
            project_selected: true,
            loaded: true,
            busy: false,
            read_only: false,
            french: false,
            error: None,
            status: Some(snap),
            active_entry: None,
            capabilities: full_caps(),
        },
    );
    assert_eq!(
        enter_file_act,
        Some(ScmChangesAction::OpenDiff {
            area: ScmArea::Unstaged,
            path: "src/second.rs".to_string(),
            old_path: None,
            pinned: false,
        })
    );

    // `reset()` clears draft, filter, and collapsed states
    panel.reset();
    assert!(panel.commit_message().is_empty());
    assert!(panel.filter_query().is_empty());
    assert!(!panel.is_section_collapsed(ScmSectionKind::Unstaged));
}
