use super::*;
use crate::share::{
    AskAnswer, GuestTaskOp, Host, HostCommand, InviteSpec, LiveShareAsk, ShareRights,
    ShareSettings, SharedProject, SharedTask,
};
use crate::share_panel::{DialogTab, ShareGpuiAction};
use gpui::TestAppContext;
use std::time::{Duration, Instant};

fn joined_guest(
    cx: &mut TestAppContext,
) -> (
    tempfile::TempDir,
    Host,
    Vec<SharedTask>,
    gpui::WindowHandle<TesselGpuiRoot>,
    crate::guest_task_board::Scope,
) {
    cx.update(|cx| {
        use gpui_ce_elements::editable_text::actions::{DEFAULT_INPUT_CONTEXT, default_bindings};
        cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));
    });
    let dir = tempfile::Builder::new()
        .prefix("tessel-guest-fixture-")
        .tempdir_in(std::env::current_dir().unwrap())
        .unwrap();
    let projects: Vec<_> = (1..=2)
        .map(|id| {
            let path = dir.path().join(format!("project-{id}"));
            std::fs::create_dir(&path).unwrap();
            SharedProject {
                id,
                name: format!("Project {id}"),
                path,
            }
        })
        .collect();
    let host = Host::start_multi(
        projects,
        "Fixture".into(),
        InviteSpec::default(),
        ShareSettings::default(),
        Vec::new(),
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1".parse().unwrap(),
    )
    .unwrap();
    let cards: Vec<_> = (1..=2)
        .map(|id| SharedTask {
            project_id: id,
            id: id.to_string(),
            title: format!("Card {id}"),
            column: "todo".into(),
            guest_by: None,
        })
        .collect();
    host.publish_tasks(cards.clone());
    let invitation = host.invitation.encode().unwrap();
    let view = cx.add_window(|_, cx| {
        let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
        root.dispatch(GpuiAction::OpenLiveShare, cx);
        root.dispatch(
            GpuiAction::Share(ShareGpuiAction::SetTab(DialogTab::Join)),
            cx,
        );
        root.dispatch(
            GpuiAction::Share(ShareGpuiAction::SetJoinCode(invitation)),
            cx,
        );
        root.dispatch(GpuiAction::Share(ShareGpuiAction::JoinSession), cx);
        root
    });
    let started = Instant::now();
    let key = loop {
        if let Some(LiveShareAsk::Join { key, .. }) = host.snapshot().asks.first() {
            break key.clone();
        }
        assert!(started.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    };
    host.answer_ask(AskAnswer::Join {
        key,
        allow: true,
        projects: vec![1, 2],
        rights: ShareRights::collaborate(),
        trust: false,
    });
    let first = loop {
        let board = view
            .update(cx, |root, _, cx| {
                if root.poll_backend(cx) {
                    cx.notify();
                }
                root.app
                    .gpui_snapshot(1100., 850., 4.)
                    .modals
                    .share
                    .guest_board
            })
            .unwrap();
        if let Some(scope) = board.scope
            && !board.pending
            && board.cards.len() == 1
        {
            break scope;
        }
        assert!(started.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    };
    (dir, host, cards, view, first)
}

#[gpui::test]
fn native_guest_task_input_scopes_edits_and_keeps_newer_draft_after_ack(cx: &mut TestAppContext) {
    let (_dir, host, mut cards, view, first) = joined_guest(cx);
    view.update(cx, |root, _, cx| {
        assert!(
            root.app.gpui_modal_query().is_some(),
            "joined writable guest must have a native draft input"
        );
        cx.notify();
    })
    .unwrap();
    cx.refresh().unwrap();
    let old = view
        .update(cx, |root, w, cx| {
            let input = root.modal_query_input.state().unwrap().clone();
            input.update(cx, |state, cx| {
                state.replace_text_in_range(None, "Créer 😀", w, cx)
            });
            input
        })
        .unwrap();
    cx.run_until_parked();
    view.update(cx, |_, w, cx| {
        old.update(cx, |state, cx| {
            let end = state.as_str().encode_utf16().count();
            state.replace_text_in_range(Some(0..end), &"x".repeat(5000), w, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    view.update(cx, |root, _, cx| {
        assert!(
            root.app
                .gpui_snapshot(1100., 850., 4.)
                .modals
                .share
                .guest_board
                .draft_over_limit
        );
        root.dispatch(
            GpuiAction::Share(ShareGpuiAction::GuestTask {
                scope: first,
                action: crate::guest_task_board::Action::Add,
            }),
            cx,
        );
        assert!(
            !root
                .app
                .gpui_snapshot(1100., 850., 4.)
                .modals
                .share
                .guest_board
                .pending
        );
    })
    .unwrap();
    assert!(host.drain_commands().is_empty());
    view.update(cx, |_, w, cx| {
        old.update(cx, |state, cx| {
            let end = state.as_str().encode_utf16().count();
            state.replace_text_in_range(Some(0..end), "Créer 😀", w, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    view.update(cx, |root, _, _| {
        assert!(
            !root
                .app
                .gpui_snapshot(1100., 850., 4.)
                .modals
                .share
                .guest_board
                .draft_over_limit
        )
    })
    .unwrap();
    view.update(cx, |root, _, cx| {
        assert_eq!(root.app.gpui_modal_query().unwrap().1, "Créer 😀");
        root.dispatch(
            GpuiAction::Share(ShareGpuiAction::GuestProject {
                session: first.session,
                project: 2,
            }),
            cx,
        );
    })
    .unwrap();
    cx.refresh().unwrap();
    view.update(cx, |root, w, cx| {
        old.update(cx, |state, cx| {
            state.replace_text_in_range(None, "stale", w, cx)
        });
        assert_eq!(root.app.gpui_modal_query().unwrap().1, "");
    })
    .unwrap();
    cx.run_until_parked();
    view.update(cx, |root, _, cx| {
        assert_eq!(root.app.gpui_modal_query().unwrap().1, "");
        root.dispatch(
            GpuiAction::Share(ShareGpuiAction::GuestProject {
                session: first.session,
                project: 1,
            }),
            cx,
        );
    })
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let pending = view
            .update(cx, |root, _, cx| {
                if root.poll_backend(cx) {
                    cx.notify();
                }
                root.app
                    .gpui_snapshot(1100., 850., 4.)
                    .modals
                    .share
                    .guest_board
                    .pending
            })
            .unwrap();
        if !pending {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    cx.refresh().unwrap();
    view.update(cx, |root, _, _| {
        assert_eq!(root.app.gpui_modal_query().unwrap().1, "Créer 😀")
    })
    .unwrap();
    cx.simulate_keystrokes(view.into(), "enter");
    cx.run_until_parked();
    view.update(cx, |root, w, cx| {
        let input = root.modal_query_input.state().unwrap().clone();
        input.update(cx, |state, cx| {
            let end = state.as_str().encode_utf16().count();
            state.set_selected_text_range(end..end, w, cx);
            state.replace_text_in_range(None, " suite", w, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    let command = loop {
        if let Some(HostCommand::TaskMutation(command)) = host.drain_commands().pop() {
            break command;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    };
    host.complete_task(command, |project, _, op| {
        assert_eq!(project, 1);
        let GuestTaskOp::Add { title, .. } = op else {
            panic!("expected Add");
        };
        assert_eq!(title, "Créer 😀");
        cards.push(SharedTask {
            project_id: 1,
            id: "3".into(),
            title: title.clone(),
            column: "todo".into(),
            guest_by: None,
        });
        Ok((true, cards.clone()))
    });
    loop {
        let pending = view
            .update(cx, |root, _, cx| {
                root.poll_backend(cx);
                root.app
                    .gpui_snapshot(1100., 850., 4.)
                    .modals
                    .share
                    .guest_board
                    .pending
            })
            .unwrap();
        if !pending {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    cx.refresh().unwrap();
    view.update(cx, |root, _, cx| {
        assert_eq!(root.app.gpui_modal_query().unwrap().1, "Créer 😀 suite");
        assert_eq!(
            root.modal_query_input.state().unwrap().read(cx).as_str(),
            "Créer 😀 suite"
        );
        root.dispatch(GpuiAction::Share(ShareGpuiAction::DisconnectGuest), cx);
    })
    .unwrap();
}

#[gpui::test]
fn native_guest_image_preview_offers_original_on_right_click(cx: &mut TestAppContext) {
    let (dir, _host, _cards, view, scope) = joined_guest(cx);
    let file = dir.path().join("project-1/poster.PNG");
    image::RgbaImage::from_pixel(80, 48, image::Rgba([51u8, 103, 222, 255]))
        .save_with_format(&file, image::ImageFormat::Png)
        .unwrap();
    std::fs::write(dir.path().join("project-1/readme.txt"), "fixture").unwrap();
    let saved = dir.path().join("saved-poster.png");
    for selector in [
        "guest-tab-files",
        "guest-entry-0",
        "guest-doc-tab-0",
        "guest-entry-1",
        "guest-doc-tab-1",
        "guest-doc-tab-0",
        "guest-open-workspace",
        "view-tab-0",
        "view-tab-1",
        "viewer-copy-path",
        "guest-image-preview",
        "guest-image-download",
    ] {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            view.update(cx, |root, _, cx| {
                if root.poll_backend(cx) {
                    cx.notify();
                }
            })
            .unwrap();
            cx.refresh().unwrap();
            let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
            if let Some(rect) = visual.debug_bounds(selector) {
                if selector == "guest-image-preview" {
                    visual.simulate_mouse_down(
                        rect.center(),
                        gpui::MouseButton::Right,
                        Default::default(),
                    );
                    visual.simulate_mouse_up(
                        rect.center(),
                        gpui::MouseButton::Right,
                        Default::default(),
                    );
                } else if selector == "viewer-copy-path" {
                    visual.simulate_click(rect.center(), Default::default());
                    assert_eq!(
                        cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
                            .as_deref(),
                        Some("poster.PNG")
                    );
                } else {
                    if selector == "guest-image-download" {
                        crate::download_name::TEST_DOWNLOAD_DESTINATION
                            .with(|next| next.set(Some(saved.clone())));
                    }
                    visual.simulate_click(rect.center(), Default::default());
                }
                break;
            }
            assert!(Instant::now() < deadline, "missing {selector}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    while !saved.exists() {
        view.update(cx, |root, _, cx| {
            root.poll_backend(cx);
        })
        .unwrap();
        assert!(Instant::now() < deadline, "guest image was not saved");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(std::fs::read(saved).unwrap(), std::fs::read(file).unwrap());
    view.update(cx, |root, _, cx| {
        let files = root
            .app
            .gpui_snapshot(1100., 850., 4.)
            .modals
            .share
            .guest_files
            .unwrap();
        let (key, image) = files.image.unwrap();
        assert_eq!(key.path, "poster.PNG");
        assert_eq!(image.size(0).width.0, 80);
        assert!(files.document.is_none());
        root.dispatch(
            GpuiAction::Share(ShareGpuiAction::GuestProject {
                session: scope.session,
                project: 2,
            }),
            cx,
        );
        assert!(
            root.app
                .gpui_snapshot(1100., 850., 4.)
                .modals
                .share
                .guest_files
                .unwrap()
                .image
                .is_none()
        );
    })
    .unwrap();
}
#[gpui::test]
fn native_guest_files_open_text_and_host_history(cx: &mut TestAppContext) {
    let (dir, host, _cards, view, scope) = joined_guest(cx);
    let project = dir.path().join("project-1");
    std::fs::write(project.join("notes.txt"), "guest preview").unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["add", "notes.txt"],
        vec!["commit", "-qm", "GPUI history fixture"],
    ] {
        let output = std::process::Command::new("git")
            .current_dir(&project)
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let profile = dir.path().join("private-history");
    let file = project.join("notes.txt").canonicalize().unwrap();
    crate::local_file_history::record(&profile, &file, "saved version before editing").unwrap();
    host.set_history(Some(crate::share::HistoryStore {
        profile,
        read: crate::local_file_history::shared_read,
    }));
    let mut checked_download = false;
    for selector in [
        "guest-tab-files",
        "guest-entry-0",
        "guest-download-0",
        "guest-file-menu-0",
        "guest-entry-0",
        "guest-history-refresh",
        "guest-history-commit",
        "guest-versions",
        "guest-version-0",
        "guest-version-text",
        "guest-saved-diff",
    ] {
        let deadline = Instant::now() + Duration::from_secs(18);
        loop {
            view.update(cx, |root, _, cx| {
                if root.poll_backend(cx) {
                    cx.notify();
                }
            })
            .unwrap();
            cx.refresh().unwrap();
            let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
            if let Some(bounds) = visual.debug_bounds(selector) {
                if selector == "guest-entry-0" && !checked_download {
                    assert_eq!(
                        visual.debug_bounds("guest-file-icon-0").unwrap().size.width,
                        gpui::px(16.)
                    );
                    visual.simulate_mouse_down(
                        bounds.center(),
                        gpui::MouseButton::Right,
                        Default::default(),
                    );
                    visual.simulate_mouse_up(
                        bounds.center(),
                        gpui::MouseButton::Right,
                        Default::default(),
                    );
                    checked_download = true;
                    break;
                }
                if selector == "guest-download-0" {
                    break;
                }
                if selector == "guest-version-0" {
                    std::fs::write(&file, "changed on host after opening").unwrap();
                }
                if !matches!(
                    selector,
                    "guest-history-commit" | "guest-version-text" | "guest-saved-diff"
                ) {
                    visual.simulate_click(bounds.center(), Default::default());
                }
                break;
            }
            assert!(Instant::now() < deadline, "missing {selector}");
            std::thread::sleep(Duration::from_millis(15));
        }
    }
    view.update(cx, |root, _, cx| {
        let files = root
            .app
            .gpui_snapshot(1100., 850., 4.)
            .modals
            .share
            .guest_files
            .unwrap();
        assert_eq!(
            files.version_text.unwrap().as_str(),
            "saved version before editing"
        );
        let patch = files.version_diff.unwrap();
        assert!(
            (0..patch.line_count()).any(|i| patch.line(i) == "-saved version before editing")
                && (0..patch.line_count())
                    .any(|i| patch.line(i) == "+changed on host after opening")
        );
        let doc = files.document.unwrap();
        assert_eq!(doc.text.as_str(), "guest preview");
        assert_eq!(
            files.history.unwrap().items[0].message,
            "GPUI history fixture"
        );
        assert!(root.app.gpui_modal_query().is_none());
        root.dispatch(
            GpuiAction::Share(ShareGpuiAction::GuestProject {
                session: scope.session,
                project: 2,
            }),
            cx,
        );
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("keep".into()));
        root.dispatch(GpuiAction::CopyGuestFileText(doc.key), cx);
        assert_eq!(
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .as_deref(),
            Some("keep")
        );
        let other = root
            .app
            .gpui_snapshot(1100., 850., 4.)
            .modals
            .share
            .guest_files
            .unwrap();
        assert!(
            other.document.is_none() && other.history.is_none() && other.version_diff.is_none()
        );
    })
    .unwrap();
}

#[gpui::test]
fn native_guest_copy_text_includes_the_part_beyond_preview(cx: &mut TestAppContext) {
    let (dir, _host, _cards, view, _) = joined_guest(cx);
    let full_text = "guest preview line\n".repeat(3800) + "end of document";
    std::fs::write(dir.path().join("project-1/notes.txt"), &full_text).unwrap();
    for selector in [
        "guest-tab-files",
        "guest-entry-0",
        "guest-open-workspace",
        "guest-copy-text",
    ] {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            view.update(cx, |root, _, cx| {
                if root.poll_backend(cx) {
                    cx.notify();
                }
            })
            .unwrap();
            cx.refresh().unwrap();
            let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
            if let Some(rect) = visual.debug_bounds(selector) {
                visual.simulate_click(rect.center(), Default::default());
                break;
            }
            assert!(Instant::now() < deadline, "missing {selector}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    assert_eq!(
        cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
            .as_deref(),
        Some(full_text.as_str())
    );
}

#[gpui::test]
fn native_guest_recovers_file_drafts_after_host_disconnect(cx: &mut TestAppContext) {
    use crate::share_panel::guest_files::RecoveryAction;
    let (dir, host, _cards, view, scope) = joined_guest(cx);
    let key = |name: &str| crate::guest_file_editor::Key {
        scope,
        path: name.into(),
    };
    let first = "ligne de texte\n".repeat(5300) + "fin du brouillon";
    view.update(cx, |root, _, cx| {
        root.dispatch(
            GpuiAction::Share(ShareGpuiAction::GuestFilesTab(scope, true)),
            cx,
        );
    })
    .unwrap();
    for (name, draft) in [("a.txt", first.as_str()), ("b.txt", "brouillon B")] {
        let file = key(name);
        std::fs::write(dir.path().join("project-1").join(name), "original").unwrap();
        view.update(cx, |root, _, cx| {
            root.dispatch(
                GpuiAction::Share(ShareGpuiAction::GuestOpenFile(file.clone())),
                cx,
            );
        })
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let edited = view
                .update(cx, |root, _, cx| {
                    if root.poll_backend(cx) {
                        cx.notify();
                    }
                    let loaded = root
                        .app
                        .gpui_snapshot(1100., 850., 4.)
                        .modals
                        .share
                        .guest_files
                        .and_then(|f| f.document)
                        .is_some_and(|d| d.key == file);
                    if !loaded {
                        return false;
                    }
                    root.dispatch(GpuiAction::SelectGuestDocument(file.clone()), cx);
                    let doc = root.app.gpui_guest_editor().unwrap();
                    root.app.gpui_edit_guest_document(&doc, draft.to_owned())
                })
                .unwrap();
            if edited {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    view.update(cx, |root, _, cx| {
        root.dispatch(GpuiAction::OpenLiveShare, cx);
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("unchanged".into()));
        root.dispatch(
            GpuiAction::RecoverGuestFile(key("b.txt"), RecoveryAction::Copy),
            cx,
        );
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some("unchanged")
        );
    })
    .unwrap();
    drop(host);
    for (selector, expected) in [
        ("retained-file-copy", Some("brouillon B")),
        ("retained-file-1", None),
        ("retained-file-copy", Some(first.as_str())),
        ("retained-file-discard", None),
        ("retained-file-keep", None),
        ("retained-file-copy", Some(first.as_str())),
        ("retained-file-discard", None),
        ("retained-file-discard", None),
        ("retained-file-copy", Some("brouillon B")),
    ] {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            view.update(cx, |root, _, cx| {
                if root.poll_backend(cx) {
                    cx.notify();
                }
            })
            .unwrap();
            cx.refresh().unwrap();
            let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
            if let Some(rect) = visual.debug_bounds(selector) {
                visual.simulate_click(rect.center(), Default::default());
                break;
            }
            assert!(Instant::now() < deadline, "missing {selector}");
            std::thread::sleep(Duration::from_millis(10));
        }
        if let Some(text) = expected {
            assert!(
                cx.update(
                    |cx| cx.read_from_clipboard().and_then(|i| i.text()).as_deref() == Some(text)
                ),
                "wrong draft after {selector}"
            );
        }
    }
    for name in ["a.txt", "b.txt"] {
        assert_eq!(
            std::fs::read_to_string(dir.path().join("project-1").join(name)).unwrap(),
            "original"
        );
    }
}

#[gpui::test]
fn native_guest_workspace_saves_to_selected_host_file(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (dir, _host, _cards, view, _) = joined_guest(cx);
    let path = dir.path().join("project-1/a.txt");
    let other = dir.path().join("project-2/a.txt");
    std::fs::write(&path, "original").unwrap();
    std::fs::write(&other, "other project").unwrap();
    std::fs::write(dir.path().join("project-1/AGENTS.md"), "protected").unwrap();
    for selector in ["guest-tab-files", "guest-entry-0", "guest-open-workspace"] {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            view.update(cx, |root, _, cx| {
                if root.poll_backend(cx) {
                    cx.notify();
                }
            })
            .unwrap();
            cx.refresh().unwrap();
            let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
            if let Some(rect) = visual.debug_bounds(selector) {
                visual.simulate_click(rect.center(), Default::default());
                break;
            }
            assert!(Instant::now() < deadline, "missing {selector}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    cx.refresh().unwrap();
    let old_input = view
        .update(cx, |root, w, cx| {
            let input = root.editor_input.guest.state().unwrap().clone();
            input.read(cx).focus_handle(cx).focus(w, cx);
            input.update(cx, |s, cx| {
                s.replace_text_in_range(Some(0..8), "partagé", w, cx)
            });
            input
        })
        .unwrap();
    cx.simulate_keystrokes(view.into(), "ctrl-s");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let saved = view
            .update(cx, |root, _, cx| {
                if root.poll_backend(cx) {
                    cx.notify();
                }
                root.app
                    .gpui_guest_editor()
                    .is_some_and(|d| !d.dirty() && d.text.as_str() == "partagé")
            })
            .unwrap();
        if saved {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "host did not acknowledge the workspace save"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "partagé");
    assert_eq!(std::fs::read_to_string(other).unwrap(), "other project");
    view.update(cx, |root, _, cx| {
        let mut key = root.app.gpui_guest_editor().unwrap().key;
        key.path = "AGENTS.md".into();
        root.dispatch(GpuiAction::Share(ShareGpuiAction::GuestOpenFile(key)), cx);
    })
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let loaded = view
            .update(cx, |root, _, cx| {
                if root.poll_backend(cx) {
                    cx.notify();
                }
                root.app
                    .gpui_snapshot(1100., 850., 4.)
                    .modals
                    .share
                    .guest_files
                    .and_then(|f| f.document)
                    .is_some_and(|d| d.key.path == "AGENTS.md")
            })
            .unwrap();
        if loaded {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    cx.refresh().unwrap();
    view.update(cx, |root, w, cx| {
        assert!(root.editor_input.guest.state().is_none());
        old_input.update(cx, |s, cx| s.replace_text_in_range(None, "stale", w, cx));
    })
    .unwrap();
    cx.simulate_keystrokes(view.into(), "ctrl-s");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("project-1/AGENTS.md")).unwrap(),
        "protected"
    );
}

#[gpui::test]
fn native_guest_file_tabs_return_to_the_first_document(cx: &mut TestAppContext) {
    let (dir, _host, _cards, view, _) = joined_guest(cx);
    for name in ["a.txt", "b.txt"] {
        std::fs::write(dir.path().join("project-1").join(name), name).unwrap();
    }
    for (selector, expected) in [
        ("guest-tab-files", None),
        ("guest-entry-0", None),
        ("guest-copy-text", Some("a.txt")),
        ("guest-entry-1", None),
        ("guest-doc-tab-1", None),
        ("guest-copy-text", Some("b.txt")),
        ("guest-open-workspace", None),
        ("edit-b", None),
        ("view-tab-1", None),
        ("guest-copy-text", Some("a.txt")),
        ("view-tab-2", None),
        ("guest-copy-text", Some("unsaved B")),
        ("restore-b", None),
        ("view-tab-1", None),
        ("guest-copy-text", Some("a.txt")),
        ("view-tab-close-1", None),
        ("guest-copy-text", Some("b.txt")),
    ] {
        if matches!(selector, "edit-b" | "restore-b") {
            use gpui::EntityInputHandler;
            cx.refresh().unwrap();
            view.update(cx, |root, window, cx| {
                let state = root.editor_input.guest.state().unwrap().clone();
                state.update(cx, |s, cx| {
                    let end = s.as_str().encode_utf16().count();
                    let next = if selector == "edit-b" {
                        "unsaved B"
                    } else {
                        "b.txt"
                    };
                    s.replace_text_in_range(Some(0..end), next, window, cx);
                });
            })
            .unwrap();
            continue;
        }
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            view.update(cx, |root, _, cx| {
                if root.poll_backend(cx) {
                    cx.notify();
                }
            })
            .unwrap();
            cx.refresh().unwrap();
            let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
            if let Some(rect) = visual.debug_bounds(selector) {
                visual.simulate_click(rect.center(), Default::default());
                break;
            }
            assert!(Instant::now() < deadline, "missing {selector}");
            std::thread::sleep(Duration::from_millis(10));
        }
        if let Some(text) = expected {
            assert_eq!(
                cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
                    .as_deref(),
                Some(text)
            );
        }
    }
    let local = dir.path().join("local");
    std::fs::create_dir(&local).unwrap();
    let path = local.join("B.TXT");
    std::fs::write(&path, "local contents").unwrap();
    view.update(cx, |root, window, cx| {
        root.app.install_editor_input_fixture(
            vec![crate::files::Document::open(&local, &path).unwrap()],
            0,
        );
        let snapshot = root.app.gpui_snapshot(1100., 850., 4.);
        let local_tab = snapshot
            .view_tabs
            .iter()
            .find(|t| t.doc_index == Some(0))
            .unwrap();
        let guest_tab = snapshot
            .view_tabs
            .iter()
            .find(|t| t.guest_key.is_some())
            .unwrap();
        assert_eq!(local_tab.title, "B.TXT · local");
        assert_eq!(guest_tab.title, "b.txt · Project 1");
        assert_eq!(
            guest_tab.hint,
            if snapshot.french {
                "Onglet de Fixture : b.txt"
            } else {
                "Fixture’s tab: b.txt"
            }
        );
        root.dispatch(
            GpuiAction::SelectGuestDocument(guest_tab.guest_key.clone().unwrap()),
            cx,
        );
        root.focus_handle.focus(window, cx);
    })
    .unwrap();
    cx.refresh().unwrap();
    gpui::VisualTestContext::from_window(view.into(), cx).simulate_event(gpui::KeyDownEvent {
        keystroke: gpui::Keystroke::parse("ctrl-w").unwrap(),
        is_held: true,
        prefer_character_input: false,
    });
    view.update(cx, |root, _, cx| {
        assert!(root.app.gpui_close_document_action().is_some());
        root.dispatch(GpuiAction::OpenHelp, cx);
    })
    .unwrap();
    cx.refresh().unwrap();
    cx.simulate_keystrokes(view.into(), "ctrl-w");
    view.update(cx, |root, window, cx| {
        assert!(root.app.gpui_close_document_action().is_none());
        assert!(
            root.app
                .gpui_snapshot(1100., 850., 4.)
                .view_tabs
                .iter()
                .any(|tab| tab.guest_key.is_some())
        );
        root.dispatch(GpuiAction::CloseOverlays, cx);
        root.focus_handle.focus(window, cx);
    })
    .unwrap();
    cx.refresh().unwrap();
    cx.simulate_keystrokes(view.into(), "ctrl-w");
    view.update(cx, |root, window, cx| {
        let snapshot = root.app.gpui_snapshot(1100., 850., 4.);
        assert!(snapshot.view_tabs.iter().all(|tab| tab.guest_key.is_none()));
        assert!(
            snapshot
                .view_tabs
                .iter()
                .any(|tab| tab.doc_index == Some(0) && tab.title == "B.TXT")
        );
        root.dispatch(GpuiAction::SelectDocumentTab(0), cx);
        root.focus_handle.focus(window, cx);
    })
    .unwrap();
    cx.refresh().unwrap();
    cx.simulate_keystrokes(view.into(), "ctrl-w");
    view.update(cx, |root, _, _| {
        let snapshot = root.app.gpui_snapshot(1100., 850., 4.);
        assert!(
            snapshot
                .view_tabs
                .iter()
                .all(|tab| tab.doc_index.is_none() && tab.guest_key.is_none())
        );
    })
    .unwrap();
}
