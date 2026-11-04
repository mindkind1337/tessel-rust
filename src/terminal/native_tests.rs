//! End-to-end UI routing through the real host client and command worker, with
//! in-process pipes. No SSH, owner store, user profile, or real credentials.
use super::*;
use crate::terminal::{Terminal, backend::Backend};
use eframe::egui;
use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use tessel_host_client::{Client, Connector, Options, Pipe};
use tessel_host_protocol::{
    Budgets, Capability, ClientFrame, Epoch, HostFrame, LeaseKey, MAX_FRAME_BYTES, RawMessage,
    ReplyMessage, StreamId, TerminalId, VERSION,
    pty::{self, Attach, AttachMode, Attached, Input, InputOk, Size, Writer},
    read_frame, write_frame,
};

#[derive(Debug, PartialEq, Eq)]
enum Seen {
    Input(Vec<u8>),
    Resize,
    Close,
    Detach,
}
struct Fake(Sender<Seen>);
impl Connector for Fake {
    fn connect(&self) -> io::Result<Pipe> {
        let (reader, mut daemon_writer) = io::pipe()?;
        let (mut daemon_reader, writer) = io::pipe()?;
        let seen = self.0.clone();
        std::thread::spawn(move || {
            while let Ok(Some(frame)) =
                read_frame::<ClientFrame>(&mut daemon_reader, MAX_FRAME_BYTES)
            {
                let (Some(id), Some(message)) = (frame.request_id, frame.message) else {
                    continue;
                };
                let answer = match message.kind.as_str() {
                    "hello" => ReplyMessage::HelloOk(tessel_host_protocol::HelloOk {
                        version: VERSION,
                        capabilities: vec![Capability::Pty],
                        budgets: Budgets::SLOW_HOST,
                        host_epoch: Epoch::from_bytes([7; 16]),
                    })
                    .into_raw(),
                    pty::kind::ATTACH => {
                        let attach: Attach = message.body_as().unwrap();
                        let lease = match attach.writer {
                            Writer::Resume(lease) => Some(lease),
                            _ => None,
                        };
                        RawMessage::of(
                            pty::kind::ATTACHED,
                            &Attached {
                                stream: StreamId::from_bytes([9; 16]),
                                pid: 1,
                                size: Size { rows: 24, cols: 80 },
                                mode: AttachMode::Replay,
                                seq: 0,
                                last_input: lease.as_ref().map(|_| 0),
                                lease,
                            },
                        )
                    }
                    pty::kind::INPUT => {
                        let input: Input = message.body_as().unwrap();
                        let _ = seen.send(Seen::Input(input.data.into_bytes()));
                        RawMessage::of(pty::kind::INPUT_OK, &InputOk { duplicate: false })
                    }
                    pty::kind::RESIZE => {
                        let _ = seen.send(Seen::Resize);
                        ReplyMessage::Done.into_raw()
                    }
                    pty::kind::CLOSE => {
                        let _ = seen.send(Seen::Close);
                        ReplyMessage::Done.into_raw()
                    }
                    pty::kind::DETACH => {
                        let _ = seen.send(Seen::Detach);
                        ReplyMessage::Done.into_raw()
                    }
                    _ => break,
                };
                if write_frame(
                    &mut daemon_writer,
                    &HostFrame::reply_ok(id, answer),
                    MAX_FRAME_BYTES,
                )
                .and_then(|()| daemon_writer.flush())
                .is_err()
                {
                    break;
                }
            }
        });
        Ok(Pipe {
            reader: Box::new(reader),
            writer: Box::new(writer),
            guard: None,
        })
    }
}
struct MemoryOnly;
impl host_lease_keeper::LeaseWriter for MemoryOnly {
    fn put(&self, _: &host_opening::Slot, _: &LeaseKey) -> Result<(), host_opening::LocalError> {
        Ok(())
    }
    fn remove(&self, _: &host_opening::Slot) -> Result<(), host_opening::LocalError> {
        Ok(())
    }
    fn complete(
        &self,
        _: &str,
        _: &host_opening::Slot,
        _: &LeaseKey,
    ) -> Result<(), host_opening::LocalError> {
        Ok(())
    }
}

fn fixture(ctx: &egui::Context) -> (Terminal, Client, Receiver<Seen>) {
    let (seen, received) = mpsc::channel();
    let client = Client::connect(
        Box::new(Fake(seen)),
        Options {
            capabilities: vec!["pty"],
            ..Default::default()
        },
    )
    .unwrap();
    let slot = host_opening::Slot {
        alias: "synthetic-host".into(),
        epoch: Epoch::from_bytes([7; 16]),
        terminal: TerminalId::from_bytes([4; 16]),
    };
    let (remote, events) = tessel_host_client::terminal::Terminal::attach(
        &client,
        slot.terminal.clone(),
        slot.epoch.clone(),
        0,
        Some(LeaseKey::from_bytes([3; 16])),
    )
    .unwrap();
    let keeper = Arc::new(host_lease_keeper::Keeper::start(Arc::new(MemoryOnly)).unwrap());
    let control = host_worker::start(
        host_worker::Permit::acquire().unwrap(),
        client.clone(),
        remote,
        slot.clone(),
        keeper.clone(),
    )
    .unwrap();
    let desk = host_lease_desk::LeaseDesk::new(keeper.clone()).handle();
    let registration = Some(Box::new(desk.seat(&slot).unwrap()) as Box<dyn Send>);
    let native = Native {
        alias: slot.alias.clone(),
        host: HostConnection::new(client.clone(), client.options().request_timeout),
        pending: None,
        failure: None,
        slot: Some(slot.clone()),
        feed: Some(host_events::Feed::new_created(events, true)),
        control: Some(control),
        saved: true,
        desk,
        openings: Default::default(),
        registration,
        creation_record: None,
        awaiting: None,
        cancelling: None,
        admission_paused: false,
        target: host_opening::Target::New(host_opening::Spec {
            alias: slot.alias,
            argv: vec![],
            cwd: None,
            env: Default::default(),
            size: Size { rows: 24, cols: 80 },
        }),
        requested_size: None,
        allow_replies: false,
        keeper,
        forget: None,
        end_observed: false,
        active_until: Cell::new(Instant::now()),
        terminating: false,
        repaint: ctx.clone(),
    };
    let mut terminal = Terminal::with_backend(Backend::Native(Box::new(native)));
    terminal.parser = vt100::Parser::new(24, 80, 1000);
    terminal.size = (24, 80);
    (terminal, client, received)
}

fn frame(terminal: &mut Terminal, ctx: &egui::Context, enabled: bool, events: Vec<egui::Event>) {
    let _ = frame_output(terminal, ctx, enabled, events);
}

#[test]
fn new_host_epoch_invalidates_even_rejected_or_closed_streams_but_a_disconnect_does_not() {
    for state in [
        host_events::State::Receiving,
        host_events::State::Disconnected,
        host_events::State::Rejected(host_events::Rejection::InvalidData),
        host_events::State::Closed,
        host_events::State::Lost,
    ] {
        let ctx = egui::Context::default();
        let (mut terminal, client, received) = fixture(&ctx);
        terminal.parser.process(b"retained screen");
        let Backend::Native(native) = &mut terminal.backend else {
            unreachable!()
        };
        native.feed.as_mut().unwrap().state = state;
        assert!(!native.observe_epoch(None));
        assert!(!native.observe_epoch(Some(Epoch::from_bytes([7; 16]))));
        assert!(
            native.forget.is_none(),
            "neither disconnection nor rejection proves an end"
        );
        assert!(!native.end_observed);
        assert!(native.control.is_some());
        native.observe_epoch(Some(Epoch::from_bytes([8; 16])));
        assert_eq!(
            native.feed.as_ref().unwrap().state,
            host_events::State::Lost
        );
        assert!(!native.feed.as_ref().unwrap().lease);
        assert!(native.control.is_none());
        assert!(native.end_observed);
        assert!(
            matches!(&native.forget, Some(host_lease_keeper::Ended::EpochChanged { current }) if *current == Epoch::from_bytes([8; 16]))
        );
        assert!(!native.observe_epoch(Some(Epoch::from_bytes([8; 16]))));
        assert!(
            terminal
                .parser
                .screen()
                .contents()
                .contains("retained screen")
        );
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let seen = received
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap();
            assert_ne!(
                seen,
                Seen::Close,
                "epoch observation must never terminate a remote program"
            );
            if seen == Seen::Detach {
                break;
            }
        }
        drop(terminal);
        client.close();
    }
}
fn frame_output(
    terminal: &mut Terminal,
    ctx: &egui::Context,
    enabled: bool,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 420.0),
            )),
            events,
            focused: true,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                if !enabled {
                    ui.disable();
                }
                terminal.show(ui, 14.0);
            });
        },
    )
}

#[test]
fn host_persistence_and_resynchronization_reach_the_native_pane_not_just_the_pure_panel() {
    use crate::native_host_info::{Probe, Source};
    use tessel_host_protocol::{Persistence, Survival};
    struct Report(Survival);
    impl Source for Report {
        fn epoch(&self) -> Option<Epoch> {
            Some(Epoch::from_bytes([7; 16]))
        }
        fn persistence(&self) -> Option<Persistence> {
            Some(Persistence {
                survives_logout: self.0,
                ..Persistence::UNKNOWN
            })
        }
    }
    let ctx = egui::Context::default();
    let (mut terminal, client, _received) = fixture(&ctx);
    for (survival, expected) in [
        (Survival::Yes, false),
        (Survival::No, true),
        (Survival::Unknown, true),
    ] {
        let mut probe = Probe::default();
        probe.poll(&Report(survival));
        let Backend::Native(native) = &mut terminal.backend else {
            unreachable!()
        };
        native.host = native.host.clone().with_info(probe.cache());
        native.feed.as_mut().unwrap().resynchronized = true;
        frame(&mut terminal, &ctx, true, vec![]);
        let output = frame_output(&mut terminal, &ctx, true, vec![]);
        let texts: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            texts.iter().any(|text| text.contains("end on logout")
                || text.contains("persistence could not be verified")
                || text.contains("se ferment à la déconnexion")
                || text.contains("persistance après déconnexion")),
            expected
        );
        assert!(
            texts
                .iter()
                .any(|text| text.contains("Output resynchronized")
                    || text.contains("Sortie resynchronisée"))
        );
    }
    drop(terminal);
    client.close();
}

#[test]
fn silent_native_input_reaches_client_but_modal_and_close_confirmation_do_not() {
    let ctx = egui::Context::default();
    let (mut terminal, client, received) = fixture(&ctx);
    frame(&mut terminal, &ctx, true, vec![]);
    terminal.request_focus(&ctx);
    frame(
        &mut terminal,
        &ctx,
        true,
        vec![egui::Event::Text("typed".into())],
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let seen = received
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        if matches!(seen, Seen::Input(_)) {
            assert_eq!(seen, Seen::Input(b"typed".to_vec()));
            break;
        }
    }
    // Resize acknowledgement alone must not resize the local emulator: a Resized
    // event, ordered with output, is required.
    assert_eq!(terminal.parser.screen().size(), (24, 80));
    frame(
        &mut terminal,
        &ctx,
        false,
        vec![
            egui::Event::Text("modal-secret".into()),
            egui::Event::Paste("paste-secret".into()),
        ],
    );
    let identity = terminal.native_identity().expect("resumable pane identity");
    assert_eq!(identity.epoch, Epoch::from_bytes([7; 16]));
    assert_eq!(identity.terminal, TerminalId::from_bytes([4; 16]));
    assert!(
        !terminal.request_native_close(),
        "the app must retain a controlled pane until the user chooses"
    );
    assert!(terminal.native_owns_escape());
    assert!(terminal.take_native_action().is_none());
    frame(
        &mut terminal,
        &ctx,
        true,
        vec![egui::Event::Text("confirmation-secret".into())],
    );
    std::thread::sleep(Duration::from_millis(80));
    assert!(
        received
            .try_iter()
            .all(|seen| !matches!(seen, Seen::Input(_) | Seen::Close))
    );
    drop(terminal);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let seen = received
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        assert_ne!(
            seen,
            Seen::Close,
            "pane drop must not terminate the program"
        );
        if seen == Seen::Detach {
            break;
        }
    }
    client.close();
}

#[test]
fn disabled_pane_yields_escape_to_global_dialog_and_resets_composition() {
    let ctx = egui::Context::default();
    let (mut terminal, client, _received) = fixture(&ctx);
    frame(&mut terminal, &ctx, true, vec![]);
    terminal.request_focus(&ctx);
    terminal.composing = true;
    terminal.composition = "unfinished".into();
    let Backend::Native(native) = &terminal.backend else {
        unreachable!()
    };
    terminal
        .native_panel
        .request_close(&native.phase(), native.capabilities());
    let mut global_received_escape = false;
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 420.0),
            )),
            focused: true,
            events: vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.disable();
                terminal.show(ui, 14.0);
                global_received_escape =
                    ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            });
        },
    );
    assert!(global_received_escape);
    assert!(terminal.native_panel.has_pending_close_choice());
    assert!(!terminal.composing);
    assert!(terminal.composition.is_empty());
    assert!(output.platform_output.ime.is_none());
    drop(terminal);
    client.close();
}

#[test]
fn opening_a_modal_releases_a_previously_held_remote_mouse_button() {
    let ctx = egui::Context::default();
    let (mut terminal, client, received) = fixture(&ctx);
    terminal.parser.process(b"\x1b[?1000h\x1b[?1006h");
    // Click the live terminal below the logout-policy notice, not its overlay.
    let pos = egui::pos2(40.0, 300.0);
    frame(
        &mut terminal,
        &ctx,
        true,
        vec![egui::Event::PointerMoved(pos)],
    );
    assert!(!terminal.native_panel.pointer_over_overlay(pos));
    frame(
        &mut terminal,
        &ctx,
        true,
        vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    frame(&mut terminal, &ctx, false, vec![]);
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut inputs = Vec::new();
    while inputs.len() < 2 {
        if let Seen::Input(bytes) = received
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap()
        {
            inputs.push(bytes);
        }
    }
    assert!(inputs[0].ends_with(b"M"));
    assert!(inputs[1].ends_with(b"m"));
    drop(terminal);
    client.close();
}

#[test]
fn native_status_does_not_change_the_terminal_grid_height() {
    let ctx = egui::Context::default();
    let (mut terminal, client, _received) = fixture(&ctx);
    frame(&mut terminal, &ctx, true, vec![]);
    let Backend::Native(native) = &terminal.backend else {
        unreachable!()
    };
    let before = native.requested_size;
    terminal.status = "Synthetic action not confirmed".into();
    frame(&mut terminal, &ctx, true, vec![]);
    let Backend::Native(native) = &terminal.backend else {
        unreachable!()
    };
    assert_eq!(native.requested_size, before);
    drop(terminal);
    client.close();
}

#[test]
fn native_overlay_stays_clipped_to_its_pane_and_below_foreground_dialogs() {
    let ctx = egui::Context::default();
    let (mut terminal, client, _received) = fixture(&ctx);
    let Backend::Native(native) = &mut terminal.backend else {
        unreachable!()
    };
    native.feed.as_mut().unwrap().state =
        host_events::State::Rejected(host_events::Rejection::InvalidData);
    let clip = egui::Rect::from_min_max(egui::pos2(40.0, 50.0), egui::pos2(400.0, 280.0));
    let marker = egui::Color32::from_rgb(7, 23, 61);
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 420.0),
            )),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.set_clip_rect(clip);
                terminal.show(ui, 14.0);
            });
            ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("fixture-global-dialog"),
            ))
            .rect_filled(clip, 0.0, marker);
        },
    );
    let marker_index = output
        .shapes
        .iter()
        .position(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill == marker))
        .unwrap();
    let mut text_count = 0;
    for (index, shape) in output.shapes.iter().enumerate() {
        if let egui::Shape::Text(_) = &shape.shape {
            text_count += 1;
            assert!(
                clip.contains_rect(shape.clip_rect),
                "overlay lost pane clipping"
            );
            assert!(index < marker_index, "overlay covered the global dialog");
        }
    }
    assert!(
        text_count > 0,
        "the rejected-stream overlay must be visible"
    );
    drop(terminal);
    client.close();
}

#[test]
fn rejected_native_stream_keeps_screen_and_refuses_keyboard_and_resize() {
    let ctx = egui::Context::default();
    let (mut terminal, client, received) = fixture(&ctx);
    terminal.parser.process(b"last verified screen");
    let Backend::Native(native) = &mut terminal.backend else {
        unreachable!()
    };
    native.feed.as_mut().unwrap().state =
        host_events::State::Rejected(host_events::Rejection::InvalidData);
    terminal.poll();
    frame(&mut terminal, &ctx, true, vec![]);
    terminal.request_focus(&ctx);
    frame(
        &mut terminal,
        &ctx,
        true,
        vec![
            egui::Event::Text("must not type".into()),
            egui::Event::Paste("nor paste".into()),
        ],
    );
    assert_eq!(terminal.output(), "last verified screen");
    assert_eq!(terminal.parser.screen().size(), (24, 80));
    assert_eq!(
        received.recv_timeout(Duration::from_secs(3)).unwrap(),
        Seen::Detach,
        "rejected stream cannot send commands"
    );
    assert!(received.try_iter().next().is_none());
    drop(terminal);
    client.close();
}
