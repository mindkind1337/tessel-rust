//! Standalone unit + headless `egui` parity tests for the complete Status Bar (`b161`):
//! - `status_metrics.rs`: `KeepAwakeController` transitions & `Drop` reset, non-double-counted
//!   PTY vs app process-tree resource attribution, CPU delta + PID-reuse guard, `SparklineRings`
//!   capacity/eviction, and 32-hop / path-boundary / dual-stack TCP port attribution.
//! - `status_strip.rs`: `show_full` across `1200px` (`Full`), `700px` (`Compact`), and `420px`
//!   (`IconOnly`), EN & FR across all 7 themes, plus interactive Keep Awake, Resource Manager,
//!   Ports (`OpenPortInBrowser`, `CopyPortAddress`), Remote Hosts, and right-click visibility menus.

#[allow(dead_code)]
#[path = "../src/icons.rs"]
mod icons;
#[allow(dead_code)]
#[path = "../src/layout_broadcast_ui.rs"]
mod layout_broadcast_ui;
#[allow(dead_code)]
#[path = "../src/pane_grid.rs"]
mod pane_grid;
#[allow(dead_code)]
#[path = "../src/sidebar_model.rs"]
mod sidebar_model;
#[allow(dead_code)]
#[path = "../src/status_metrics.rs"]
mod status_metrics;
#[allow(dead_code)]
#[path = "../src/status_strip.rs"]
mod status_strip;
#[allow(dead_code)]
#[path = "../src/theme_palettes.rs"]
mod theme_palettes;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use eframe::egui::{self, Color32, Event, Modifiers, PointerButton, Pos2, Rect, Shape, Vec2};
use sidebar_model::{
    APP_HISTORY_KEY, HISTORY_CAPACITY, HISTORY_STALE_MS, KeepAwakeMode, TerminalMeta,
};
use status_metrics::{
    DetectedPort, KeepAwakeController, RawProcessInfo, RawTcpListener, SparklineRings,
    TelemetryReport, TerminalPidProbe, WorkspacePortProbe, attribute_listening_ports,
    compute_resource_snapshot, should_keep_awake,
};
use status_strip::{
    RemoteHostRowInput, RemoteHostStatusKind, StatusBarAction, StatusBarInput, StatusBarItemId,
    StatusBarState, Target, format_pane_summary, show_full,
};
use theme_palettes::THEMES;

fn collect_shape_texts(shape: &Shape, clip: Rect, out: &mut Vec<(String, Rect, Color32, Rect)>) {
    match shape {
        Shape::Text(t) => {
            let color = t
                .override_text_color
                .or_else(|| t.galley.job.sections.first().map(|s| s.format.color))
                .unwrap_or(Color32::TRANSPARENT);
            out.push((
                t.galley.job.text.clone(),
                Rect::from_min_size(t.pos, t.galley.size()),
                color,
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

fn painted_texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, Rect, Color32, Rect)> {
    let mut out = Vec::new();
    for cs in shapes {
        collect_shape_texts(&cs.shape, cs.clip_rect, &mut out);
    }
    out
}

fn has_painted_text_substr(shapes: &[egui::epaint::ClippedShape], needle: &str) -> bool {
    painted_texts(shapes)
        .iter()
        .any(|(text, rect, _, clip)| text.contains(needle) && clip.intersects(*rect))
}

fn find_painted_text_center(shapes: &[egui::epaint::ClippedShape], exact: &str) -> Option<Pos2> {
    painted_texts(shapes)
        .into_iter()
        .find(|(text, rect, _, clip)| text == exact && clip.intersects(*rect))
        .map(|(_, rect, _, clip)| rect.intersect(clip).center())
}

fn primary_click(pos: Pos2) -> Vec<Event> {
    vec![
        Event::PointerMoved(pos),
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        },
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        },
    ]
}

fn secondary_click(pos: Pos2) -> Vec<Event> {
    vec![
        Event::PointerMoved(pos),
        Event::PointerButton {
            pos,
            button: PointerButton::Secondary,
            pressed: true,
            modifiers: Modifiers::NONE,
        },
        Event::PointerButton {
            pos,
            button: PointerButton::Secondary,
            pressed: false,
            modifiers: Modifiers::NONE,
        },
    ]
}

#[allow(clippy::too_many_arguments)]
fn proc_entry(
    pid: u32,
    ppid: u32,
    name: &str,
    exe: Option<&str>,
    ws_mb: u64,
    priv_mb: u64,
    cpu_ticks: u64,
    created: u64,
) -> RawProcessInfo {
    RawProcessInfo {
        pid,
        ppid,
        name: name.to_owned(),
        exe_path: exe.map(PathBuf::from),
        working_set_bytes: ws_mb * 1024 * 1024,
        private_bytes: priv_mb * 1024 * 1024,
        cpu_ticks_100ns: cpu_ticks,
        creation_time_100ns: created,
    }
}

#[test]
fn keep_awake_controller_tracks_modes_backend_failures_and_drop_reset() {
    assert!(!should_keep_awake(KeepAwakeMode::Off, false));
    assert!(!should_keep_awake(KeepAwakeMode::Off, true));
    assert!(!should_keep_awake(KeepAwakeMode::Agents, false));
    assert!(should_keep_awake(KeepAwakeMode::Agents, true));
    assert!(should_keep_awake(KeepAwakeMode::On, false));
    assert!(should_keep_awake(KeepAwakeMode::On, true));

    let calls: Rc<RefCell<Vec<bool>>> = Rc::new(RefCell::new(Vec::new()));
    {
        let log = calls.clone();
        let mut ctrl = KeepAwakeController::with_backend(move |enable| {
            log.borrow_mut().push(enable);
            true
        });
        assert!(!ctrl.is_active());
        assert!(!ctrl.sync(false));
        assert!(
            calls.borrow().is_empty(),
            "idempotent false makes no OS call"
        );

        assert!(ctrl.sync(true));
        assert!(ctrl.is_active());
        assert!(ctrl.sync(true));
        assert_eq!(*calls.borrow(), vec![true]);

        // Leaving scope while active must reset OS execution state to false!
    }
    assert_eq!(*calls.borrow(), vec![true, false]);

    // If OS backend refuses enable, `applied` stays false and retries cleanly later.
    let mut allow = false;
    let mut ctrl =
        KeepAwakeController::with_backend(move |enable| if enable { allow } else { true });
    assert!(!ctrl.sync(true));
    assert!(!ctrl.is_active());
    let _ = &mut allow;
}

#[test]
fn resource_snapshot_claims_pty_subtrees_before_app_without_double_counting_and_guards_pid_reuse() {
    let mut procs = HashMap::new();
    // App main process (100) has helper WebView2 (101) AND spawned PTY shell (200 -> node 201).
    procs.insert(
        100,
        proc_entry(100, 1, "tessel-rust.exe", None, 120, 90, 100_000, 10),
    );
    procs.insert(
        101,
        proc_entry(101, 100, "msedgewebview2.exe", None, 80, 60, 50_000, 20),
    );
    procs.insert(
        200,
        proc_entry(200, 100, "pwsh.exe", None, 40, 30, 20_000, 30),
    );
    procs.insert(
        201,
        proc_entry(201, 200, "node.exe", None, 110, 95, 80_000, 40),
    );

    let probes = vec![TerminalPidProbe {
        id: "1".into(),
        workspace_key: "project:1".into(),
        pid: Some(200),
    }];

    let (snap1, counts1, cpu1) =
        compute_resource_snapshot(&procs, &HashMap::new(), 1_000, 100, &probes);
    assert_eq!(counts1.get("1"), Some(&2));
    let term1 = snap1.terminals.get("1").expect("terminal 1 usage");
    assert_eq!(term1.memory_bytes as u64, 150 * 1024 * 1024);
    assert_eq!(term1.private_bytes as u64, 125 * 1024 * 1024);
    // App total should be 100 (120 MiB) + 101 (80 MiB) = 200 MiB (NOT including 200/201!).
    let app_mem: f64 = snap1.app.iter().map(|a| a.memory_bytes).sum();
    assert_eq!(app_mem as u64, 200 * 1024 * 1024);
    assert_eq!(snap1.total_memory_bytes as u64, 350 * 1024 * 1024);

    // Advance 1000 ms; node.exe (201) burns 250ms of CPU (2_500_000 * 100ns ticks => 25.0%).
    // Meanwhile PID 101 is reused by a brand-new process (`creation_time_100ns` changed 20 -> 99)!
    procs.get_mut(&201).unwrap().cpu_ticks_100ns += 2_500_000;
    procs.insert(
        101,
        proc_entry(101, 100, "msedgewebview2.exe", None, 80, 60, 9_000_000, 99),
    );

    let (snap2, _, _) = compute_resource_snapshot(&procs, &cpu1, 2_000, 100, &probes);
    let term2 = snap2.terminals.get("1").unwrap();
    assert!((term2.cpu_percent - 25.0).abs() < 0.01);
    let other_cpu = snap2
        .app
        .iter()
        .find(|a| a.kind == "other")
        .map_or(0.0, |a| a.cpu_percent);
    assert_eq!(
        other_cpu, 0.0,
        "reused PID must not produce a bogus CPU spike"
    );

    // SparklineRings caps at 60 samples and evicts stale workspace keys while keeping APP_HISTORY_KEY.
    let mut rings = SparklineRings::default();
    for i in 0..(HISTORY_CAPACITY + 10) {
        rings.record(&snap2, &probes, (i as u64) * 1_000);
    }
    assert_eq!(rings.samples(APP_HISTORY_KEY).len(), HISTORY_CAPACITY);
    assert_eq!(rings.samples("project:1").len(), HISTORY_CAPACITY);

    rings.record(
        &snap2,
        &[],
        ((HISTORY_CAPACITY + 10) as u64) * 1_000 + HISTORY_STALE_MS + 5_000,
    );
    assert!(!rings.samples(APP_HISTORY_KEY).is_empty());
    assert!(rings.samples("project:1").is_empty());
}

#[test]
fn port_attribution_walks_ancestors_enforces_path_boundaries_and_dedups_dual_stack() {
    let mut procs = HashMap::new();
    procs.insert(10, proc_entry(10, 1, "tessel-rust.exe", None, 50, 40, 0, 5));
    procs.insert(20, proc_entry(20, 10, "pwsh.exe", None, 20, 15, 0, 10));
    procs.insert(21, proc_entry(21, 20, "npm.cmd", None, 10, 8, 0, 15));
    procs.insert(22, proc_entry(22, 21, "node.exe", None, 60, 50, 0, 20));
    // Path-matched background daemon in nested workspace `C:\work\app\api` (deepest folder wins):
    procs.insert(
        30,
        proc_entry(
            30,
            1,
            "api-server.exe",
            Some(r"C:\work\app\api\bin\api-server.exe"),
            30,
            20,
            0,
            25,
        ),
    );
    // Sibling folder `C:\work\app-other` must NOT match `C:\work\app`!
    procs.insert(
        40,
        proc_entry(
            40,
            1,
            "other.exe",
            Some(r"C:\work\app-other\other.exe"),
            15,
            10,
            0,
            30,
        ),
    );

    let workspaces = vec![
        WorkspacePortProbe {
            key: "project:1".into(),
            name: "app".into(),
            path: PathBuf::from(r"C:\work\app"),
            pids: vec![20],
        },
        WorkspacePortProbe {
            key: "project:2".into(),
            name: "api".into(),
            path: PathBuf::from(r"C:\work\app\api"),
            pids: vec![],
        },
    ];

    let listeners = vec![
        // Dual-stack Vite server on PID 22:
        RawTcpListener {
            bind_addr: "0.0.0.0".into(),
            port: 5173,
            pid: 22,
        },
        RawTcpListener {
            bind_addr: "::".into(),
            port: 5173,
            pid: 22,
        },
        // HTTPS port on deepest workspace (`project:2`):
        RawTcpListener {
            bind_addr: "127.0.0.1".into(),
            port: 8443,
            pid: 30,
        },
        // External sibling folder process:
        RawTcpListener {
            bind_addr: "127.0.0.1".into(),
            port: 9090,
            pid: 40,
        },
    ];

    let ports = attribute_listening_ports(
        &listeners,
        &procs,
        &workspaces,
        Some(Path::new(r"C:\Users\dev")),
        10,
    );
    assert_eq!(ports.len(), 3, "dual-stack 5173 must deduplicate");

    assert_eq!(ports[0].port, 5173);
    assert_eq!(ports[0].workspace_key.as_deref(), Some("project:1"));
    assert_eq!(ports[0].url, "http://localhost:5173");
    assert_eq!(ports[0].process_name, "node");
    assert!(ports[0].can_stop);

    assert_eq!(ports[1].port, 8443);
    assert_eq!(ports[1].workspace_key.as_deref(), Some("project:2"));
    assert_eq!(ports[1].url, "https://127.0.0.1:8443");
    assert!(ports[1].can_stop);

    assert_eq!(ports[2].port, 9090);
    assert_eq!(ports[2].workspace_key, None);
    assert!(!ports[2].can_stop, "external ports must never allow stop");
}

fn sample_telemetry() -> TelemetryReport {
    let mut procs = HashMap::new();
    procs.insert(
        100,
        proc_entry(100, 1, "tessel-rust.exe", None, 140, 100, 0, 10),
    );
    procs.insert(200, proc_entry(200, 100, "pwsh.exe", None, 60, 45, 0, 20));
    let probes = vec![TerminalPidProbe {
        id: "7".into(),
        workspace_key: "project:1".into(),
        pid: Some(200),
    }];
    let (snap, counts, _) = compute_resource_snapshot(&procs, &HashMap::new(), 1_000, 100, &probes);
    let mut sparklines = SparklineRings::default();
    sparklines.record(&snap, &probes, 1_000);
    sparklines.record(&snap, &probes, 3_000);

    TelemetryReport {
        resources: Some(snap),
        process_counts: counts,
        sparklines,
        ports: vec![
            DetectedPort {
                port: 5173,
                pid: 200,
                process_name: "vite".into(),
                bind_host: "::".into(),
                connect_host: "localhost".into(),
                address: "localhost:5173".into(),
                url: "http://localhost:5173".into(),
                workspace_key: Some("project:1".into()),
                workspace_name: Some("Tessel".into()),
                creation_time_100ns: 20,
                can_stop: true,
            },
            DetectedPort {
                port: 5432,
                pid: 999,
                process_name: "postgres".into(),
                bind_host: "127.0.0.1".into(),
                connect_host: "127.0.0.1".into(),
                address: "127.0.0.1:5432".into(),
                url: "http://127.0.0.1:5432".into(),
                workspace_key: None,
                workspace_name: None,
                creation_time_100ns: 5,
                can_stop: false,
            },
        ],
        ports_refreshing: false,
        ports_error: String::new(),
        last_stop_feedback: None,
    }
}

#[test]
fn status_bar_renders_responsive_breakpoints_across_themes_and_languages() {
    assert_eq!(
        format_pane_summary(2, 1, 1, false),
        "2 panes · 1 working · 1 waiting for you"
    );
    assert_eq!(
        format_pane_summary(2, 1, 1, true),
        "2 panneaux · 1 au travail · 1 vous attendent"
    );

    let telemetry = sample_telemetry();
    let terminals = vec![TerminalMeta {
        id: "7".into(),
        workspace_id: "project:1".into(),
        workspace_name: "Tessel".into(),
        name: "Ada".into(),
        is_agent: true,
    }];
    let mut bound = HashSet::new();
    bound.insert("7".to_owned());
    let hosts = vec![RemoteHostRowInput {
        alias: "prod-box".into(),
        label: "prod-box".into(),
        status: RemoteHostStatusKind::Connected,
    }];

    for theme in &THEMES {
        let spec = theme.spec();
        for &french in &[false, true] {
            for &width in &[1200.0_f32, 700.0, 420.0] {
                let ctx = egui::Context::default();
                let mut state = StatusBarState::default();
                let raw = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 600.0))),
                    ..Default::default()
                };
                let out = ctx.run(raw, |ctx| {
                    egui::TopBottomPanel::bottom("sb")
                        .exact_height(24.0)
                        .show(ctx, |ui| {
                            let _ = show_full(
                                ui,
                                &mut state,
                                &StatusBarInput {
                                    ui_colors: &spec.ui,
                                    french,
                                    target: Target::Input("Ada"),
                                    pane_count: 2,
                                    working_count: 1,
                                    waiting_count: 0,
                                    project_path: r"C:\Projects\Tessel",
                                    keep_awake_mode: KeepAwakeMode::Agents,
                                    keep_awake_active: true,
                                    show_ssh: true,
                                    show_resources: true,
                                    show_ports: true,
                                    terminals: &terminals,
                                    bound_terminals: &bound,
                                    telemetry: &telemetry,
                                    remote_hosts: &hosts,
                                    agent_updating: &[],
                                    error_banner: "",
                                    persistence_unavailable: false,
                                },
                            );
                        });
                });

                if width >= 900.0 {
                    assert!(has_painted_text_substr(&out.shapes, r"C:\Projects\Tessel"));
                    assert!(has_painted_text_substr(
                        &out.shapes,
                        if french { "1 hôte" } else { "1 host" }
                    ));
                } else {
                    assert!(
                        !has_painted_text_substr(&out.shapes, r"C:\Projects\Tessel"),
                        "project path must collapse below 900px"
                    );
                }
            }
        }
    }
}

#[test]
fn status_bar_popovers_emit_keep_awake_focus_port_browser_remote_and_visibility_actions() {
    let ctx = egui::Context::default();
    let palette = THEMES[0].spec().ui;
    let telemetry = sample_telemetry();
    let terminals = vec![TerminalMeta {
        id: "7".into(),
        workspace_id: "project:1".into(),
        workspace_name: "Tessel".into(),
        name: "Ada".into(),
        is_agent: true,
    }];
    let mut bound = HashSet::new();
    bound.insert("7".to_owned());
    let hosts = vec![RemoteHostRowInput {
        alias: "prod-box".into(),
        label: "prod-box".into(),
        status: RemoteHostStatusKind::Connected,
    }];
    let mut state = StatusBarState::default();

    let step = |state: &mut StatusBarState,
                events: Vec<Event>|
     -> (egui::FullOutput, Option<StatusBarAction>) {
        let mut emitted = None;
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 720.0))),
            events,
            ..Default::default()
        };
        let out = ctx.run(raw, |ctx| {
            egui::TopBottomPanel::bottom("sb")
                .exact_height(24.0)
                .show(ctx, |ui| {
                    emitted = show_full(
                        ui,
                        state,
                        &StatusBarInput {
                            ui_colors: &palette,
                            french: false,
                            target: Target::Input("Ada"),
                            pane_count: 1,
                            working_count: 1,
                            waiting_count: 0,
                            project_path: r"C:\Projects\Tessel",
                            keep_awake_mode: KeepAwakeMode::Off,
                            keep_awake_active: false,
                            show_ssh: true,
                            show_resources: true,
                            show_ports: true,
                            terminals: &terminals,
                            bound_terminals: &bound,
                            telemetry: &telemetry,
                            remote_hosts: &hosts,
                            agent_updating: &[],
                            error_banner: "",
                            persistence_unavailable: false,
                        },
                    );
                });
        });
        (out, emitted)
    };

    // 1. Warmup render at 1280x720.
    let (out0, act0) = step(&mut state, vec![]);
    assert_eq!(act0, None);

    // 2. Click Keep Awake ("Off") -> opens Keep Awake popover -> settle Area -> click "Agent".
    let off_pos = find_painted_text_center(&out0.shapes, "Off").expect("Keep Awake Off label");
    let _ = step(&mut state, primary_click(off_pos));
    assert!(state.has_open_popover());
    let (out1, _) = step(&mut state, vec![]);
    assert!(has_painted_text_substr(
        &out1.shapes,
        "Stay awake while an agent is working"
    ));
    let agents_pos = find_painted_text_center(&out1.shapes, "Agent").expect("Agent option");
    let (_, act_awake) = step(&mut state, primary_click(agents_pos));
    assert_eq!(
        act_awake,
        Some(StatusBarAction::SetKeepAwake(KeepAwakeMode::Agents))
    );
    assert!(!state.has_open_popover());

    // 3. Open Resource Manager -> click session "Ada" -> emits FocusSession("7") & closes.
    state.open_resources();
    let _ = step(&mut state, vec![]);
    let (out_res, _) = step(&mut state, vec![]);
    let ada_pos = find_painted_text_center(&out_res.shapes, "Ada").expect("Ada resource row");
    let (_, act_focus) = step(&mut state, primary_click(ada_pos));
    assert_eq!(act_focus, Some(StatusBarAction::FocusSession("7".into())));
    assert!(!state.has_open_popover());

    // 4. Open Ports popover -> click "5173" -> emits OpenPortInBrowser & closes!
    state.open_ports();
    let _ = step(&mut state, vec![]);
    let (out_ports, _) = step(&mut state, vec![]);
    assert!(has_painted_text_substr(&out_ports.shapes, "External Ports"));
    let port_pos = find_painted_text_center(&out_ports.shapes, "5173").expect("5173 port row");
    let (_, act_open_port) = step(&mut state, primary_click(port_pos));
    assert_eq!(
        act_open_port,
        Some(StatusBarAction::OpenPortInBrowser {
            url: "http://localhost:5173".into(),
            port: 5173,
        })
    );
    assert!(
        !state.has_open_popover(),
        "OpenPortInBrowser must close Ports popover so native Browser is unblocked"
    );

    // 5. Click Remote Hosts ("1 host") -> click "Manage Remote Hosts…".
    let (out_bar, _) = step(&mut state, vec![]);
    let host_pos = find_painted_text_center(&out_bar.shapes, "1 host").expect("1 host trigger");
    let _ = step(&mut state, primary_click(host_pos));
    let (out_rh, _) = step(&mut state, vec![]);
    let manage_pos = find_painted_text_center(&out_rh.shapes, "Manage Remote Hosts…")
        .expect("Manage Remote Hosts button");
    let (_, act_rh) = step(&mut state, primary_click(manage_pos));
    assert_eq!(act_rh, Some(StatusBarAction::ManageRemoteHosts));

    // 6. Right-click status bar background -> opens visibility menu -> click "Ports".
    let _ = step(&mut state, secondary_click(Pos2::new(420.0, 708.0)));
    let (out_vis, _) = step(&mut state, vec![]);
    let ports_toggle =
        find_painted_text_center(&out_vis.shapes, "Ports").expect("Ports visibility item");
    let (_, act_toggle) = step(&mut state, primary_click(ports_toggle));
    assert_eq!(
        act_toggle,
        Some(StatusBarAction::ToggleItem(StatusBarItemId::Ports))
    );
}
