// Adapted from Tessel src/main/latestInFile.js. MIT Lovecast Inc. 2026;
// see THIRD_PARTY_NOTICES.md.
//! Bounded backwards scan for a model record buried under later tool output.
//! Call inside the pane-model worker with a verified conversation handle.
//! Input bytes remain transient; only the parser's metadata leaves this reader.
use crate::pane_model_worker::Failure;
use std::io::{Read, Seek, SeekFrom};

const CHUNK: usize = 512 * 1024;
const MAX_SCAN: u64 = 64 * 1024 * 1024;

/// Like the source latestInFile reader, parse newest chunks first and join lines
/// split across chunk boundaries. The parser must choose the latest valid model
/// in its input. An exhausted scan is explicit so callers can keep prior data.
/// This performs a fresh scan; incremental append caching belongs to the caller.
pub fn latest<R: Read + Seek, T>(
    file: &mut R,
    parse: impl FnMut(&str) -> Option<T>,
) -> Result<Option<T>, Failure> {
    scan(file, parse, CHUNK, MAX_SCAN)
}

fn scan<R: Read + Seek, T>(
    file: &mut R,
    mut parse: impl FnMut(&str) -> Option<T>,
    chunk: usize,
    budget: u64,
) -> Result<Option<T>, Failure> {
    let size = file.seek(SeekFrom::End(0)).map_err(|_| Failure::Io)?;
    let mut end = size;
    // Chunks of the incomplete first line, newest first. Keeping chunks avoids
    // quadratic concatenation when a large tool record contains no newlines.
    let mut carry: Vec<Vec<u8>> = Vec::new();
    while end > 0 && size - end < budget {
        let count = (chunk as u64).min(end).min(budget - (size - end));
        let start = end - count;
        let mut bytes = vec![0; count as usize];
        file.seek(SeekFrom::Start(start)).map_err(|_| Failure::Io)?;
        file.read_exact(&mut bytes).map_err(|_| Failure::Io)?;
        let first = if start == 0 {
            0
        } else if let Some(newline) = bytes.iter().position(|b| *b == b'\n') {
            newline + 1
        } else {
            carry.push(bytes);
            end = start;
            continue;
        };
        let mut whole = bytes.split_off(first);
        for part in carry.drain(..).rev() {
            whole.extend_from_slice(&part);
        }
        // Match the source's Buffer.toString('utf8') conversion. Provider JSON
        // parsers still validate records and bounded model identifiers.
        if let Some(found) = parse(&String::from_utf8_lossy(&whole)) {
            return Ok(Some(found));
        }
        if first > 0 {
            bytes.pop(); // exclude the newline from the carried first record
            carry.push(bytes);
        }
        end = start;
    }
    if end > 0 {
        Err(Failure::TooLarge)
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn model(text: &str) -> Option<String> {
        text.lines().rev().find_map(|line| {
            let value: serde_json::Value = serde_json::from_str(line).ok()?;
            value.get("model")?.as_str().map(str::to_owned)
        })
    }

    #[test]
    fn finds_model_before_megabytes_of_tool_output() {
        let mut bytes = b"{\"model\":\"older\"}\n{\"model\":\"newer\"}\n".to_vec();
        bytes.extend_from_slice(b"{\"tool\":\"");
        bytes.extend(std::iter::repeat_n(b'x', CHUNK * 3));
        bytes.extend_from_slice(b"\"}\n");
        assert_eq!(
            latest(&mut Cursor::new(bytes), model).unwrap().as_deref(),
            Some("newer")
        );
    }

    #[test]
    fn joins_every_boundary_and_ignores_unfinished_final_record() {
        for chunk in 1..80 {
            let bytes = "{\"model\":\"ancien\"}\n{\"model\":\"modèle-récent\"}\n{\"model\":";
            assert_eq!(
                scan(&mut Cursor::new(bytes.as_bytes()), model, chunk, 4096)
                    .unwrap()
                    .as_deref(),
                Some("modèle-récent"),
                "chunk {chunk}"
            );
            let bytes = "{\"model\":\"dernier-sans-newline\"}";
            assert_eq!(
                scan(&mut Cursor::new(bytes.as_bytes()), model, chunk, 4096)
                    .unwrap()
                    .as_deref(),
                Some("dernier-sans-newline")
            );
        }
    }

    #[test]
    fn limit_and_io_failures_are_distinct_from_no_model() {
        assert_eq!(
            scan(&mut Cursor::new(vec![b'x'; 100]), model, 8, 64),
            Err(Failure::TooLarge)
        );
        assert_eq!(
            scan(&mut Cursor::new(b"{\"tool\":42}\n"), model, 8, 64).unwrap(),
            None
        );
        assert_eq!(latest(&mut Cursor::new([]), model).unwrap(), None);
        struct ShortRead(Cursor<Vec<u8>>);
        impl Read for ShortRead {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Ok(0)
            }
        }
        impl Seek for ShortRead {
            fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
                self.0.seek(position)
            }
        }
        assert_eq!(
            latest(&mut ShortRead(Cursor::new(vec![1; 20])), model),
            Err(Failure::Io)
        );
    }
}
