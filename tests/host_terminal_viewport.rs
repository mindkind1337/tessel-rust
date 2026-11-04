#[allow(dead_code)]
#[path = "../src/terminal/host_apply.rs"]
mod host_apply;
#[path = "../src/terminal/host_emulator.rs"]
mod host_emulator;
#[allow(dead_code)]
#[path = "../src/terminal/host_events.rs"]
mod host_events;
#[allow(dead_code)]
#[path = "../src/terminal/host_history.rs"]
mod host_history;
#[allow(dead_code)]
#[path = "../src/terminal/host_viewport.rs"]
mod host_viewport;

use host_events::Feed;
use host_history::{History, MAX_CELLS, MAX_ROWS};
use host_viewport::{ScrollState, Viewport, live_scrollback_len};
use std::sync::mpsc;
use tessel_host_client::terminal::TerminalEvent;
use tessel_host_protocol::pty::{AttachMode, Size};

fn poll_feed_with_scroll(
    feed: &mut Feed,
    parser: &mut vt100::Parser,
    scroll: &mut ScrollState,
    tail: &mut Vec<u8>,
    writer: bool,
    replies: &mut Vec<Vec<u8>>,
) -> host_viewport::PollOutcome {
    let before_applied = feed.applied;
    let anchor = scroll.before_poll(&feed.history, parser);
    feed.poll(parser, writer, |parser, bytes, can_reply, reset| {
        if reset {
            tail.clear();
        }
        host_emulator::apply(parser, tail, bytes, can_reply, |reply| {
            replies.push(reply);
            Ok(())
        });
    });
    while feed.has_pending_work() {
        feed.poll(parser, writer, |parser, bytes, can_reply, reset| {
            if reset {
                tail.clear();
            }
            host_emulator::apply(parser, tail, bytes, can_reply, |reply| {
                replies.push(reply);
                Ok(())
            });
        });
    }
    let changed = feed.applied != before_applied;
    scroll.after_poll(anchor, &feed.history, parser, changed)
}

#[test]
fn seamless_scrollback_bridges_live_parser_and_archived_history_in_normal_and_alternate_screens() {
    let (sender, receiver) = mpsc::channel();
    let mut feed = Feed::new_created(receiver, true);
    let mut parser = vt100::Parser::new(3, 20, 100);
    let mut scroll = ScrollState::new();
    let mut tail = Vec::new();
    let mut replies = Vec::new();

    // 4 lines before resync -> 1 scrollback ("H0") + 3 visible ("H1", styled "🙂Québec", "H3").
    sender
        .send(TerminalEvent::Output {
            seq: 1,
            data: "H0\r\nH1\r\n\x1b[32;4m🙂Québec\x1b[0m\r\nH3"
                .as_bytes()
                .to_vec(),
        })
        .unwrap();
    poll_feed_with_scroll(
        &mut feed,
        &mut parser,
        &mut scroll,
        &mut tail,
        true,
        &mut replies,
    );
    assert_eq!(feed.history.len(), 0);
    assert_eq!(live_scrollback_len(&mut parser), 1);

    // Reconnect with a Snapshot replacement -> 4 pre-resync rows archived into Feed.history.
    sender.send(TerminalEvent::Disconnected).unwrap();
    sender
        .send(TerminalEvent::Reattached {
            lease: true,
            mode: AttachMode::Snapshot,
        })
        .unwrap();
    sender
        .send(TerminalEvent::Snapshot {
            at: 2,
            size: Size { rows: 3, cols: 20 },
            alternate: false,
            resync: true,
            data: b"\x1b[HS0\r\nS1\r\nL0".to_vec(),
        })
        .unwrap();
    // Push 2 more lines so live parser has 2 scrollback ("S0", "S1") + 3 live ("L0", "L1", "L2").
    sender
        .send(TerminalEvent::Output {
            seq: 3,
            data: b"\r\nL1\r\nL2".to_vec(),
        })
        .unwrap();
    poll_feed_with_scroll(
        &mut feed,
        &mut parser,
        &mut scroll,
        &mut tail,
        true,
        &mut replies,
    );

    assert_eq!(feed.history.len(), 4);
    assert_eq!(live_scrollback_len(&mut parser), 2);
    assert_eq!(scroll.total_scrollback(&parser), 0);
    assert!(!scroll.in_archived_history());
    assert!(!scroll.viewing_scrollback(&parser));

    let expected_windows = [
        (0, 0, "L0\nL1\nL2"),
        (1, 0, "S1\nL0\nL1"),
        (2, 0, "S0\nS1\nL0"),
        (3, 1, "H3\nS0\nS1"),
        (4, 2, "🙂Québec\nH3\nS0"),
        (5, 3, "H1\n🙂Québec\nH3"),
        (6, 3, "H0\nH1\n🙂Québec"),
    ];

    for (step, (expected_total, expected_hist_shown, expected_text)) in
        expected_windows.iter().enumerate()
    {
        if step > 0 {
            assert!(scroll.scroll_by(1, Some(&feed.history), &mut parser));
        }
        assert_eq!(scroll.total_scrollback(&parser), *expected_total);
        let view = Viewport::new(Some(&feed.history), parser.screen(), &scroll);
        assert_eq!(view.history_rows_shown(), *expected_hist_shown);
        assert_eq!(view.contents(), *expected_text);
    }

    // Verify styled wide cell attributes at the top archived view (`offset == 6`, row 2 is "🙂Québec").
    {
        let view = Viewport::new(Some(&feed.history), parser.screen(), &scroll);
        let wide = view.cell(2, 0).expect("wide emoji cell");
        assert_eq!(wide.contents(), "🙂");
        assert!(wide.is_wide());
        assert!(wide.underline());
        assert_eq!(wide.fgcolor(), vt100::Color::Idx(2));
        let cont = view.cell(2, 1).expect("wide continuation cell");
        assert!(cont.is_wide_continuation());
    }

    // Clamped at top: scrolling further up returns `false`.
    assert!(!scroll.scroll_by(5, Some(&feed.history), &mut parser));
    assert_eq!(scroll.total_scrollback(&parser), 6);

    // Scroll down across the seam back into live scrollback, then reset to live output.
    assert!(scroll.scroll_by(-4, Some(&feed.history), &mut parser));
    assert!(!scroll.in_archived_history());
    assert!(scroll.viewing_scrollback(&parser));
    assert_eq!(scroll.total_scrollback(&parser), 2);
    assert_eq!(
        Viewport::new(Some(&feed.history), parser.screen(), &scroll).contents(),
        "S0\nS1\nL0"
    );

    assert!(scroll.reset_to_live(&mut parser));
    assert!(!scroll.reset_to_live(&mut parser));
    assert_eq!(scroll.total_scrollback(&parser), 0);
    assert_eq!(
        Viewport::new(Some(&feed.history), parser.screen(), &scroll).contents(),
        "L0\nL1\nL2"
    );

    // Enter alternate screen on the live parser (`live_scrollback_len == 0`):
    // archived pre-resync `History` must remain scrollable and readable above the TUI screen.
    sender
        .send(TerminalEvent::Output {
            seq: 4,
            data: b"\x1b[?1049h\x1b[HTUI-0\r\nTUI-1\r\nTUI-2".to_vec(),
        })
        .unwrap();
    poll_feed_with_scroll(
        &mut feed,
        &mut parser,
        &mut scroll,
        &mut tail,
        true,
        &mut replies,
    );
    assert!(parser.screen().alternate_screen());
    assert_eq!(live_scrollback_len(&mut parser), 0);

    assert!(scroll.scroll_by(2, Some(&feed.history), &mut parser));
    assert!(scroll.in_archived_history());
    let alt_view = Viewport::new(Some(&feed.history), parser.screen(), &scroll);
    assert_eq!(alt_view.contents(), "🙂Québec\nH3\nTUI-0");

    assert!(scroll.reset_to_live(&mut parser));
    assert_eq!(
        Viewport::new(Some(&feed.history), parser.screen(), &scroll).contents(),
        "TUI-0\nTUI-1\nTUI-2"
    );
}

#[test]
fn selection_and_copy_match_vt100_across_history_live_seam_soft_wraps_and_narrowed_columns() {
    // Capture #1: soft-wrapped line across rows 0..1 ("0123456789abc" on width 10).
    let mut history = History::default();
    let mut first = vt100::Parser::new(2, 10, 10);
    first.process(b"0123456789abc");
    assert!(first.screen().row_wrapped(0));
    history.capture(&mut first);
    assert_eq!(history.len(), 2);

    // Capture #2: wide emoji + cursor-jump gap + French UTF-8, separated from Capture #1's seam.
    let mut second = vt100::Parser::new(2, 10, 10);
    second.process("A🙂B\x1b[7Gend\r\nQuébec!".as_bytes());
    history.capture(&mut second);
    assert_eq!(history.len(), 4);

    // Live parser after resync.
    let mut live = vt100::Parser::new(6, 10, 10);
    live.process(b"LIVE-ONE\r\nLIVE-TWO");

    // Scroll 4 lines into `History` so viewport (6 rows) shows all 4 archived rows + 2 live rows:
    // row 0: "0123456789" (wrapped = true)
    // row 1: "abc"        (wrapped = false — capture #1 -> #2 seam)
    // row 2: "A🙂B  end"  (col 0='A', 1..3='🙂', 3='B', 4..6=empty gap -> 2 spaces, 6..9="end")
    // row 3: "Québec!"    (wrapped = false — history -> live seam)
    // row 4: "LIVE-ONE"
    // row 5: "LIVE-TWO"
    let mut scroll = ScrollState::new();
    assert!(scroll.scroll_by(4, Some(&history), &mut live));
    let view = Viewport::new(Some(&history), live.screen(), &scroll);
    assert_eq!(
        view.contents(),
        "0123456789abc\nA🙂B  end\nQuébec!\nLIVE-ONE\nLIVE-TWO"
    );

    // Forward and reverse selection across soft-wrapped archived rows.
    assert_eq!(view.select_text((0, 0), (1, 3)), "0123456789abc");
    assert_eq!(view.select_text((1, 3), (0, 0)), "0123456789abc");

    // Selection across archived History -> live parser seam in both directions.
    assert_eq!(
        view.select_text((2, 0), (4, 8)),
        "A🙂B  end\nQuébec!\nLIVE-ONE"
    );
    assert_eq!(
        view.select_text((4, 8), (2, 0)),
        "A🙂B  end\nQuébec!\nLIVE-ONE"
    );

    // Selection starting on wide continuation cell (col 2 of "A🙂B  end") skips the first half cleanly.
    assert_eq!(view.select_text((2, 2), (2, 9)), "B  end");
    // Empty selection produces empty string.
    assert_eq!(view.select_text((2, 3), (2, 3)), "");

    // Narrow live parser to 2 columns: wide '🙂' at col 1..3 of row 2 now has its continuation
    // clipped outside `0..2`, so col 1 must be suppressed rather than half-copied or overflowing;
    // widening back to 10 columns restores '🙂' untouched from `History`.
    live.set_size(6, 2);
    scroll.sync(Some(&history), &mut live);
    let narrow_view = Viewport::new(Some(&history), live.screen(), &scroll);
    assert!(narrow_view.cell(2, 1).is_none());
    assert_eq!(narrow_view.select_text((2, 0), (2, 2)), "A");

    live.set_size(6, 10);
    scroll.sync(Some(&history), &mut live);
    let restored_view = Viewport::new(Some(&history), live.screen(), &scroll);
    assert_eq!(restored_view.cell(2, 1).unwrap().contents(), "🙂");
    assert_eq!(restored_view.select_text((2, 0), (2, 4)), "A🙂B");

    // Equivalence with `vt100::Screen` at `history_offset == 0`.
    scroll.reset_to_live(&mut live);
    let live_only = Viewport::new(Some(&history), live.screen(), &scroll);
    assert_eq!(live_only.contents(), live.screen().contents());
    assert_eq!(
        live_only.contents_between(0, 2, 1, 6),
        live.screen().contents_between(0, 2, 1, 6)
    );
}

#[test]
fn truncated_capture_clamps_to_its_own_first_row_not_an_older_capture() {
    let mut history = History::default();
    let mut old = vt100::Parser::new(10, 10, 0);
    old.process(b"OLD");
    history.capture(&mut old);
    let mut parser = vt100::Parser::new(400, 700, 1000);
    let lines = (0..500)
        .map(|n| format!("L{n:04}"))
        .collect::<Vec<_>>()
        .join("\r\n");
    parser.process(lines.as_bytes());
    let mut scroll = ScrollState::default();
    scroll.scroll_by(90, Some(&history), &mut parser);
    let anchor = scroll.before_poll(&history, &mut parser);
    history.capture(&mut parser);
    assert_eq!(
        history.evicted_rows(),
        0,
        "older narrow capture remains in the bounded store"
    );
    parser = vt100::Parser::new(3, 700, 1000);
    let outcome = scroll.after_poll(anchor, &history, &mut parser, true);
    assert!(outcome.clear_selection);
    assert!(
        Viewport::new(Some(&history), parser.screen(), &scroll)
            .contents()
            .starts_with("L0072")
    );
}

#[test]
fn changing_grid_or_resetting_before_snapshot_invalidates_the_old_selection() {
    for (change, expected) in [
        (b"\x1b[?1049hALT0\r\nALT1".as_slice(), "ALT0"),
        (b"\x1bcRESET".as_slice(), "RESET"),
    ] {
        let (sender, receiver) = mpsc::channel();
        let mut feed = Feed::new_created(receiver, true);
        let mut parser = vt100::Parser::new(2, 20, 1000);
        let mut scroll = ScrollState::default();
        let mut tail = Vec::new();
        let mut replies = Vec::new();
        sender
            .send(TerminalEvent::Output {
                seq: 1,
                data: b"P0\r\nP1\r\nP2".to_vec(),
            })
            .unwrap();
        poll_feed_with_scroll(
            &mut feed,
            &mut parser,
            &mut scroll,
            &mut tail,
            true,
            &mut replies,
        );
        scroll.scroll_by(1, Some(&feed.history), &mut parser);
        sender
            .send(TerminalEvent::Output {
                seq: 2,
                data: change.to_vec(),
            })
            .unwrap();
        sender
            .send(TerminalEvent::Snapshot {
                at: 3,
                size: Size { rows: 2, cols: 20 },
                alternate: false,
                resync: true,
                data: b"fresh".to_vec(),
            })
            .unwrap();
        let outcome = poll_feed_with_scroll(
            &mut feed,
            &mut parser,
            &mut scroll,
            &mut tail,
            true,
            &mut replies,
        );
        assert!(outcome.clear_selection);
        assert!(
            Viewport::new(Some(&feed.history), parser.screen(), &scroll)
                .contents()
                .starts_with(expected)
        );
    }
}

#[test]
fn two_snapshots_in_one_poll_do_not_hide_the_first_captures_truncation() {
    for (offset, extra_output, expected, cleared) in [
        (995, false, "L0006", true),
        (990, false, "L0010", false),
        (990, true, "L0010", false),
        (1000, true, "L0008", true),
    ] {
        let (sender, receiver) = mpsc::channel();
        let mut feed = Feed::new_created(receiver, true);
        let mut parser = vt100::Parser::new(3, 20, 1000);
        let mut scroll = ScrollState::default();
        let mut tail = Vec::new();
        let mut replies = Vec::new();
        let lines = (0..1003)
            .map(|n| format!("L{n:04}"))
            .collect::<Vec<_>>()
            .join("\r\n");
        sender
            .send(TerminalEvent::Output {
                seq: 1,
                data: lines.into_bytes(),
            })
            .unwrap();
        poll_feed_with_scroll(
            &mut feed,
            &mut parser,
            &mut scroll,
            &mut tail,
            true,
            &mut replies,
        );
        scroll.scroll_by(offset, Some(&feed.history), &mut parser);
        if extra_output {
            sender
                .send(TerminalEvent::Output {
                    seq: 2,
                    data: b"\r\nL1003\r\nL1004".to_vec(),
                })
                .unwrap();
        }
        for at in [3, 4] {
            sender
                .send(TerminalEvent::Snapshot {
                    at,
                    size: Size { rows: 3, cols: 20 },
                    alternate: false,
                    resync: true,
                    data: b"new".to_vec(),
                })
                .unwrap();
        }
        let outcome = poll_feed_with_scroll(
            &mut feed,
            &mut parser,
            &mut scroll,
            &mut tail,
            true,
            &mut replies,
        );
        assert_eq!(
            outcome.clear_selection, cleared,
            "offset={offset}, extra={extra_output}"
        );
        assert!(
            Viewport::new(Some(&feed.history), parser.screen(), &scroll)
                .contents()
                .starts_with(expected),
            "offset={offset}, extra={extra_output}"
        );
    }
}

#[test]
fn monotonic_pinning_prevents_viewport_jumps_on_live_output_repeated_resyncs_eviction_and_extreme_deltas()
 {
    let (sender, receiver) = mpsc::channel();
    let mut feed = Feed::new_created(receiver, true);
    let mut parser = vt100::Parser::new(3, 20, 100);
    let mut scroll = ScrollState::new();
    let mut tail = Vec::new();
    let mut replies = Vec::new();

    // Populate initial lines + first Snapshot resync so `History` has 6 lines ("A0".."A5").
    sender
        .send(TerminalEvent::Output {
            seq: 1,
            data: b"A0\r\nA1\r\nA2\r\nA3\r\nA4\r\nA5".to_vec(),
        })
        .unwrap();
    sender
        .send(TerminalEvent::Snapshot {
            at: 2,
            size: Size { rows: 3, cols: 20 },
            alternate: false,
            resync: true,
            data: b"\x1b[HB0\r\nB1\r\nB2".to_vec(),
        })
        .unwrap();
    poll_feed_with_scroll(
        &mut feed,
        &mut parser,
        &mut scroll,
        &mut tail,
        true,
        &mut replies,
    );
    assert_eq!(feed.history.len(), 6);

    // Scroll wholly into archived History (`history_offset = 5` -> viewing "A1\nA2\nA3").
    assert!(scroll.scroll_by(5, Some(&feed.history), &mut parser));
    assert_eq!(
        Viewport::new(Some(&feed.history), parser.screen(), &scroll).contents(),
        "A1\nA2\nA3"
    );

    // Stream 10 lines of live output: viewport must stay pinned on "A1\nA2\nA3" and preserve selection!
    sender
        .send(TerminalEvent::Output {
            seq: 3,
            data: b"\r\nB3\r\nB4\r\nB5\r\nB6\r\nB7\r\nB8\r\nB9\r\nB10\r\nB11\r\nB12".to_vec(),
        })
        .unwrap();
    let outcome = poll_feed_with_scroll(
        &mut feed,
        &mut parser,
        &mut scroll,
        &mut tail,
        true,
        &mut replies,
    );
    assert!(
        !outcome.clear_selection,
        "background live output must not wipe selection inside immutable archived history"
    );
    assert_eq!(
        Viewport::new(Some(&feed.history), parser.screen(), &scroll).contents(),
        "A1\nA2\nA3"
    );

    // Trigger a SECOND Snapshot resync while still reading "A1\nA2\nA3": stays pinned on "A1\nA2\nA3"!
    sender
        .send(TerminalEvent::Snapshot {
            at: 4,
            size: Size { rows: 3, cols: 20 },
            alternate: false,
            resync: true,
            data: b"\x1b[HC0\r\nC1\r\nC2".to_vec(),
        })
        .unwrap();
    let outcome2 = poll_feed_with_scroll(
        &mut feed,
        &mut parser,
        &mut scroll,
        &mut tail,
        true,
        &mut replies,
    );
    assert!(!outcome2.clear_selection);
    assert_eq!(
        Viewport::new(Some(&feed.history), parser.screen(), &scroll).contents(),
        "A1\nA2\nA3"
    );

    // Now return to live, produce live scrollback ("C0".."C5"), scroll 2 lines into LIVE parser
    // scrollback ("C1\nC2\nC3"), and send Output + Snapshot in the SAME poll batch:
    // the user's viewport must transition seamlessly into `History` on "C1\nC2\nC3"!
    scroll.reset_to_live(&mut parser);
    sender
        .send(TerminalEvent::Output {
            seq: 5,
            data: b"\r\nC3\r\nC4\r\nC5".to_vec(),
        })
        .unwrap();
    poll_feed_with_scroll(
        &mut feed,
        &mut parser,
        &mut scroll,
        &mut tail,
        true,
        &mut replies,
    );
    assert!(scroll.scroll_by(2, Some(&feed.history), &mut parser));
    assert!(!scroll.in_archived_history());
    assert_eq!(
        Viewport::new(Some(&feed.history), parser.screen(), &scroll).contents(),
        "C1\nC2\nC3"
    );

    sender
        .send(TerminalEvent::Output {
            seq: 6,
            data: b"\r\nC6\r\nC7".to_vec(),
        })
        .unwrap();
    sender
        .send(TerminalEvent::Snapshot {
            at: 7,
            size: Size { rows: 3, cols: 20 },
            alternate: false,
            resync: true,
            data: b"\x1b[HD0\r\nD1\r\nD2".to_vec(),
        })
        .unwrap();
    poll_feed_with_scroll(
        &mut feed,
        &mut parser,
        &mut scroll,
        &mut tail,
        true,
        &mut replies,
    );
    assert!(scroll.in_archived_history());
    assert_eq!(
        Viewport::new(Some(&feed.history), parser.screen(), &scroll).contents(),
        "C1\nC2\nC3"
    );

    // Force `MAX_CELLS` (`300_000`) / `MAX_ROWS` (`1000`) eviction of "C1\nC2\nC3" via two 500x1000
    // snapshots: `scroll` must clamp without overflow to the oldest retained row and clear selection.
    sender
        .send(TerminalEvent::Snapshot {
            at: 8,
            size: Size {
                rows: 500,
                cols: 1000,
            },
            alternate: false,
            resync: true,
            data: b"\x1b[HEVICT-BATCH-1".to_vec(),
        })
        .unwrap();
    sender
        .send(TerminalEvent::Snapshot {
            at: 9,
            size: Size {
                rows: 500,
                cols: 1000,
            },
            alternate: false,
            resync: true,
            data: b"\x1b[HEVICT-BATCH-2".to_vec(),
        })
        .unwrap();
    let evicted_outcome = poll_feed_with_scroll(
        &mut feed,
        &mut parser,
        &mut scroll,
        &mut tail,
        true,
        &mut replies,
    );
    assert!(evicted_outcome.clear_selection);
    assert!(feed.history.len() <= MAX_ROWS);
    assert_eq!(feed.history.len(), 300);
    assert_eq!(
        feed.history.len() * 1000,
        MAX_CELLS,
        "300k cell budget enforced"
    );
    assert_eq!(scroll.history_offset(), feed.history.len());

    // Extreme scroll_by deltas (`i64::MAX`, `i64::MIN`, `0`) and out-of-range coordinates never panic.
    assert!(!scroll.scroll_by(i64::MAX, Some(&feed.history), &mut parser));
    assert!(scroll.scroll_by(i64::MIN, Some(&feed.history), &mut parser));
    assert_eq!(scroll.total_scrollback(&parser), 0);
    assert!(!scroll.scroll_by(0, Some(&feed.history), &mut parser));
    assert!(scroll.scroll_by(i64::MAX, Some(&feed.history), &mut parser));
    assert_eq!(scroll.history_offset(), feed.history.len());
    let clamped_view = Viewport::new(Some(&feed.history), parser.screen(), &scroll);
    let _ = clamped_view.contents_between(0, 0, u16::MAX, u16::MAX);
    assert_eq!(clamped_view.contents_between(u16::MAX, 0, u16::MAX, 5), "");
}

#[test]
fn no_dsr_from_history_and_input_mouse_cursor_gates_block_archived_screens_only() {
    let (sender, receiver) = mpsc::channel();
    let mut feed = Feed::new_created(receiver, true);
    let mut parser = vt100::Parser::new(3, 25, 100);
    let mut scroll = ScrollState::new();
    let mut tail = Vec::new();
    let mut replies = Vec::new();

    // Live output before resync contains one complete `\x1b[6n` (1 reply) plus a trailing partial `\x1b[6`.
    sender
        .send(TerminalEvent::Output {
            seq: 1,
            data: b"OLD-PROMPT\x1b[6n\r\nOLD-2\x1b[6".to_vec(),
        })
        .unwrap();
    poll_feed_with_scroll(
        &mut feed,
        &mut parser,
        &mut scroll,
        &mut tail,
        true,
        &mut replies,
    );
    assert_eq!(replies.len(), 1, "single live DSR reply before resync");
    replies.clear();

    // Snapshot contains `\x1b[6n\x1b[5n\x1b[c` and mouse-tracking `\x1b[?1000h\x1b[?1006h`,
    // followed by live `n` (which must NOT complete the pre-resync `\x1b[6`!).
    sender
        .send(TerminalEvent::Snapshot {
            at: 2,
            size: Size { rows: 3, cols: 25 },
            alternate: false,
            resync: true,
            data: b"\x1b[H\x1b[?1000h\x1b[?1006hLIVE-TUI\x1b[6n\x1b[5n\x1b[c".to_vec(),
        })
        .unwrap();
    sender
        .send(TerminalEvent::Output {
            seq: 3,
            data: b"n\r\nLINE-2\r\nLINE-3\r\nLINE-4".to_vec(),
        })
        .unwrap();
    poll_feed_with_scroll(
        &mut feed,
        &mut parser,
        &mut scroll,
        &mut tail,
        true,
        &mut replies,
    );
    assert!(
        replies.is_empty(),
        "neither snapshot DSRs nor split queries across resync may emit replies"
    );

    // 1. At live bottom (`offset == 0`): input, mouse reporting, and cursor are all active.
    assert!(scroll.accepts_live_input(true));
    assert!(!scroll.accepts_live_input(false));
    assert!(scroll.allows_mouse_reporting(true, false, &parser));
    assert!(!scroll.allows_mouse_reporting(true, true, &parser));
    assert!(scroll.shows_cursor(true, &parser));

    // 2. Scrolled 1 line into LIVE parser scrollback (`history_offset == 0`, `parser.scrollback() == 1`):
    // typing stays enabled (snapping to live output per Terminal contract), while mouse reporting and cursor turn off.
    assert!(scroll.scroll_by(1, Some(&feed.history), &mut parser));
    assert!(!scroll.in_archived_history());
    assert!(scroll.viewing_scrollback(&parser));
    assert!(
        scroll.accepts_live_input(true),
        "live parser scrollback keeps typing enabled so keypresses return to live output"
    );
    assert!(!scroll.allows_mouse_reporting(true, false, &parser));
    assert!(!scroll.shows_cursor(true, &parser));

    // 3. Scrolled into archived `Feed.history` (`history_offset > 0`):
    // typing/paste/IME/interrupts, mouse reporting, and cursor are ALL strictly blocked,
    // while reading and copying archived cells emit zero replies.
    assert!(scroll.scroll_by(3, Some(&feed.history), &mut parser));
    assert!(scroll.in_archived_history());
    assert!(!scroll.accepts_live_input(true));
    assert!(!scroll.allows_mouse_reporting(true, false, &parser));
    assert!(!scroll.shows_cursor(true, &parser));

    let archived_view = Viewport::new(Some(&feed.history), parser.screen(), &scroll);
    assert!(archived_view.contents().contains("OLD-PROMPT"));
    assert_eq!(archived_view.select_text((0, 0), (0, 10)), "OLD-PROMPT");
    assert!(
        replies.is_empty(),
        "scrolling, rendering, and copying archived history never trigger terminal replies"
    );

    // 4. Returning to live output restores live gates when the pane itself is writable.
    assert!(scroll.reset_to_live(&mut parser));
    assert!(scroll.accepts_live_input(true));
    assert!(!scroll.accepts_live_input(false));
    assert!(scroll.allows_mouse_reporting(true, false, &parser));
    assert!(scroll.shows_cursor(true, &parser));
}
