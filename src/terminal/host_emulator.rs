//! Live terminal parsing with an explicit permission to answer cursor queries.
//! Snapshot restoration must never call this path with replies enabled.

pub fn apply(
    parser: &mut vt100::Parser,
    query_tail: &mut Vec<u8>,
    bytes: &[u8],
    can_reply: bool,
    mut reply: impl FnMut(Vec<u8>) -> Result<(), String>,
) -> Option<String> {
    if !can_reply {
        parser.process(bytes);
        query_tail.clear();
        return None;
    }
    let mut error = None;
    let mut start = 0;
    for (index, &byte) in bytes.iter().enumerate() {
        query_tail.push(byte);
        if query_tail.ends_with(b"\x1b[6n") {
            parser.process(&bytes[start..=index]);
            start = index + 1;
            let (row, col) = parser.screen().cursor_position();
            if let Err(failure) = reply(format!("\x1b[{};{}R", row + 1, col + 1).into_bytes()) {
                error = Some(failure);
            }
        }
        if query_tail.len() > 4 {
            query_tail.remove(0);
        }
    }
    parser.process(&bytes[start..]);
    error
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_queries_answer_the_live_cursor_once() {
        let mut parser = vt100::Parser::new(10, 20, 0);
        let mut tail = Vec::new();
        let mut replies = Vec::new();
        for bytes in [b"\x1b[3;5H\x1b[".as_slice(), b"6", b"n"] {
            assert!(
                apply(&mut parser, &mut tail, bytes, true, |bytes| {
                    replies.push(bytes);
                    Ok(())
                })
                .is_none()
            );
        }
        assert_eq!(replies, [b"\x1b[3;5R".to_vec()]);
        assert!(tail.len() <= 4);
    }

    #[test]
    fn bulk_parsing_stops_at_each_query_before_reporting_its_cursor() {
        let mut parser = vt100::Parser::new(10, 20, 0);
        let mut tail = Vec::new();
        let mut replies = Vec::new();
        apply(
            &mut parser,
            &mut tail,
            "é\x1b[6n\r\nX\x1b[6nend".as_bytes(),
            true,
            |bytes| {
                replies.push(bytes);
                Ok(())
            },
        );
        assert_eq!(replies, [b"\x1b[1;2R".to_vec(), b"\x1b[2;2R".to_vec()]);
        assert_eq!(parser.screen().contents(), "é\nXend");
    }

    #[test]
    fn taking_control_only_answers_queries_received_as_writer() {
        for split in 1..4 {
            let mut parser = vt100::Parser::new(10, 20, 0);
            let mut tail = Vec::new();
            let query = b"\x1b[6n";
            let mut replies = Vec::new();
            apply(&mut parser, &mut tail, &query[..split], false, |_| {
                panic!("observer replied")
            });
            apply(&mut parser, &mut tail, &query[split..], true, |_| {
                panic!("replied across control change")
            });
            apply(&mut parser, &mut tail, query, true, |bytes| {
                replies.push(bytes);
                Ok(())
            });
            assert_eq!(replies, [b"\x1b[1;1R".to_vec()]);
        }
    }

    #[test]
    fn observers_render_queries_without_replying_and_errors_keep_rendering() {
        let mut parser = vt100::Parser::new(10, 20, 0);
        let mut tail = Vec::new();
        assert!(
            apply(&mut parser, &mut tail, b"read\x1b[6n", false, |_| panic!(
                "observer replied"
            ))
            .is_none()
        );
        assert_eq!(parser.screen().contents(), "read");
        let error = apply(&mut parser, &mut tail, b"\x1b[6n onward", true, |_| {
            Err("unavailable".into())
        });
        assert_eq!(error.as_deref(), Some("unavailable"));
        assert_eq!(parser.screen().contents(), "read onward");
    }
}
