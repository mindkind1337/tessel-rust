#[path = "../src/terminal/host_history.rs"]
mod host_history;

#[test]
fn deep_scrollback_is_bounded_to_screen_height_without_overflow() {
    let mut parser = vt100::Parser::new(3, 20, 1000);
    parser.process(b"A0\r\nA1\r\nA2\r\nA3\r\nA4\r\nA5\r\nA6\r\nA7\r\nA8");
    for (offset, expected) in [
        (usize::MAX, "A0\nA1\nA2"),
        (4, "A2\nA3\nA4"),
        (2, "A4\nA5\nA6"),
        (0, "A6\nA7\nA8"),
    ] {
        parser.set_scrollback(offset);
        assert_eq!(parser.screen().rows(0, 20).count(), 3);
        assert_eq!(parser.screen().contents(), expected);
        assert_eq!(parser.screen().contents_between(0, 0, 2, 2), expected);
    }
}
