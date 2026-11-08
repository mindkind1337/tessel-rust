//! Backwards Claude/OpenClaude transcript prompt scanner (`promptBefore`, `c4686b4e`).
//!
//! When Tessel reads a bounded tail window `[head..size)` of a Claude/OpenClaude `.jsonl`
//! transcript for the Conversations panel, long tool-call chains between the user's prompt and
//! the assistant's final answer can push the prompt earlier than `head`. This module:
//! - Classifies turn-starting JSONL lines (`is_prompt_line` / `is_peer_prompt_line`),
//! - Scans backwards in `1 MiB` chunks up to a `64 MiB` byte budget (`last_prompt_before`),
//!   handling cut lines up to `8 MiB` at `before` and advancing mid-line `stop_at` boundaries,
//! - Caches `[prompt_at..head]` incrementally (`PromptBeforeCache`), preserving previous state
//!   across transient SSH/I/O errors so the next poll retries,
//! - Builds & parses the remote SSH `LC_ALL=C awk` `__t_tprompt` helper (`15 s` guest / `30 s`
//!   owner timeout, `256 B` stdout cap, `4 MiB` line cap),
//! - Extracts live Claude/OpenClaude conversation turns with `promptBefore` recovery.

#![allow(dead_code)]

use std::{
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};

/// Default backwards read chunk size (`1 MiB`).
pub const DEFAULT_CHUNK_BYTES: usize = 1024 * 1024;
/// Default maximum bytes scanned backwards per lookup (`64 MiB`).
pub const DEFAULT_BUDGET_BYTES: u64 = 64 * 1024 * 1024;
/// Maximum byte length of a single JSONL prompt line (`8 MiB` locally).
pub const DEFAULT_LINE_MAX_BYTES: usize = 8 * 1024 * 1024;
/// Step size when scanning forward for a newline boundary (`64 KiB`).
const FORWARD_STEP_BYTES: usize = 64 * 1024;

/// Remote `__t_tprompt` guest SSH timeout (`15 s`).
pub const REMOTE_TPROMPT_GUEST_TIMEOUT_MS: u64 = 15_000;
/// Remote `__t_tprompt` owner SSH timeout (`30 s`).
pub const REMOTE_TPROMPT_OWNER_TIMEOUT_MS: u64 = 30_000;
/// Maximum stdout accepted from `__t_tprompt` (`256 B`).
pub const REMOTE_TPROMPT_STDOUT_CAP: usize = 256;
/// Maximum remote prompt line length read back over SSH (`4 MiB`).
pub const REMOTE_MAX_PROMPT_LINE_BYTES: usize = 4 * 1024 * 1024;

/// Configurable bounds for [`last_prompt_before_with_options`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanOptions {
    pub chunk: usize,
    pub budget: u64,
    pub line_max: usize,
    pub stop_at: u64,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            chunk: DEFAULT_CHUNK_BYTES,
            budget: DEFAULT_BUDGET_BYTES,
            line_max: DEFAULT_LINE_MAX_BYTES,
            stop_at: 0,
        }
    }
}

/// Returns `true` if `line` belongs to the human user (`!isMeta` and either no `origin` or
/// `"kind":"human"`).
#[must_use]
pub fn is_yours_prompt_line(line: &str) -> bool {
    !line.contains("\"isMeta\":true")
        && (!line.contains("\"origin\"") || line.contains("\"kind\":\"human\""))
}

/// Exact turn-start predicate from `c4686b4e`:
/// - Rejects sidechain subagent records (`"isSidechain":true`),
/// - Accepts human user prompts (`"type":"user"` without `"type":"tool_result"`),
/// - Accepts queued prompt commands (`"queued_command"` with `"commandMode":"prompt"`),
/// - Accepts `"kind":"peer"` user records (even with `"isMeta":true`) so backwards scanning stops
///   at the peer turn's start rather than attributing the peer's assistant answer to an earlier
///   human prompt.
#[must_use]
pub fn is_prompt_line(line: &str) -> bool {
    if line.contains("\"isSidechain\":true") {
        return false;
    }
    let user = line.contains("\"type\":\"user\"") && !line.contains("\"type\":\"tool_result\"");
    let queued = line.contains("\"queued_command\"") && line.contains("\"commandMode\":\"prompt\"");
    let yours = is_yours_prompt_line(line);
    (user && (yours || line.contains("\"kind\":\"peer\""))) || (queued && yours)
}

/// Returns `true` when `line` terminates a backwards scan as a peer-origin turn whose assistant
/// reply must remain hidden in user Conversations (`\0peer` semantics in `c4686b4e`).
#[must_use]
pub fn is_peer_prompt_line(line: &str) -> bool {
    is_prompt_line(line) && !is_yours_prompt_line(line)
}

/// Scans `path` backwards from byte offset `before` using default bounds (`1 MiB` chunks,
/// `64 MiB` budget, `8 MiB` max line).
pub fn last_prompt_before(path: &Path, before: u64) -> io::Result<Option<String>> {
    last_prompt_before_with_options(path, before, ScanOptions::default())
}

/// Scans `path` backwards from byte offset `before` with explicit [`ScanOptions`].
pub fn last_prompt_before_with_options(
    path: &Path,
    before: u64,
    opts: ScanOptions,
) -> io::Result<Option<String>> {
    if before == 0 || opts.budget == 0 || opts.line_max == 0 || opts.stop_at >= before {
        return Ok(None);
    }
    let sym = fs::symlink_metadata(path)?;
    if sym.file_type().is_symlink() || !sym.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Transcript path must be a regular non-symlink file",
        ));
    }
    let size = sym.len();
    if size == 0 {
        return Ok(None);
    }
    let before = before.min(size);
    if opts.stop_at >= before {
        return Ok(None);
    }

    let mut file = File::open(path)?;
    let mut stop_at = opts.stop_at;

    // If `stop_at > 0` lands mid-line, advance forward to the byte after the next `\n`.
    if stop_at > 0 {
        let mut prev = [0u8; 1];
        file.seek(SeekFrom::Start(stop_at - 1))?;
        file.read_exact(&mut prev)?;
        if prev[0] != b'\n' {
            let mut pos = stop_at;
            let mut step_buf = vec![0u8; FORWARD_STEP_BYTES];
            let mut found_nl = false;
            while pos < before {
                let to_read = (before - pos).min(FORWARD_STEP_BYTES as u64) as usize;
                file.seek(SeekFrom::Start(pos))?;
                let n = read_full(&mut file, &mut step_buf[..to_read])?;
                if n == 0 {
                    break;
                }
                if let Some(nl) = step_buf[..n].iter().position(|&b| b == b'\n') {
                    stop_at = pos + nl as u64 + 1;
                    found_nl = true;
                    break;
                }
                pos += n as u64;
            }
            if !found_nl || stop_at >= before {
                return Ok(None);
            }
        }
    }

    // If `before` cuts through the middle of a line, read forward up to `line_max` until `\n`
    // so a prompt line spanning `before` is inspected intact.
    let mut carry = Vec::new();
    let mut carry_overflow = false;
    if before < size {
        let forward_limit = size.min(before.saturating_add(opts.line_max as u64));
        let mut pos = before;
        let mut step_buf = vec![0u8; FORWARD_STEP_BYTES];
        let mut hit_nl = false;
        while pos < forward_limit {
            let to_read = (forward_limit - pos).min(FORWARD_STEP_BYTES as u64) as usize;
            file.seek(SeekFrom::Start(pos))?;
            let n = read_full(&mut file, &mut step_buf[..to_read])?;
            if n == 0 {
                break;
            }
            if let Some(nl) = step_buf[..n].iter().position(|&b| b == b'\n') {
                carry.extend_from_slice(&step_buf[..nl]);
                hit_nl = true;
                break;
            }
            carry.extend_from_slice(&step_buf[..n]);
            pos += n as u64;
        }
        if !hit_nl && forward_limit < size {
            carry.clear();
            carry_overflow = true;
        }
    }

    let chunk_size = opts.chunk.max(1);
    let mut end = before;
    let mut scanned: u64 = 0;

    while end > stop_at && scanned < opts.budget {
        let remaining_budget = opts.budget - scanned;
        let step = (chunk_size as u64).min(remaining_budget);
        let start = stop_at.max(end.saturating_sub(step));
        let len = (end - start) as usize;
        let mut buf = vec![0u8; len];
        file.seek(SeekFrom::Start(start))?;
        let n = read_full(&mut file, &mut buf)?;
        buf.truncate(n);
        if buf.is_empty() {
            break;
        }
        scanned += buf.len() as u64;

        let mut all = buf;
        if carry_overflow {
            // Drop bytes after the rightmost `\n` in `all` because they belong to the oversized line.
            if let Some(nl) = all.iter().rposition(|&b| b == b'\n') {
                all.truncate(nl);
                carry_overflow = false;
            } else {
                end = start;
                continue;
            }
        } else if !carry.is_empty() {
            all.extend_from_slice(&carry);
            carry.clear();
        }

        let can_continue_earlier = start > stop_at && scanned < opts.budget;
        let mut line_end = all.len();
        loop {
            let nl = all[..line_end].iter().rposition(|&b| b == b'\n');
            if nl.is_none() && can_continue_earlier {
                let prefix = &all[..line_end];
                if prefix.len() <= opts.line_max {
                    carry.extend_from_slice(prefix);
                } else {
                    carry_overflow = true;
                }
                break;
            }
            let seg_start = nl.map_or(0, |idx| idx + 1);
            let seg = &all[seg_start..line_end];
            if !seg.is_empty() && seg.len() <= opts.line_max && has_prompt_prefilter(seg) {
                let text = std::str::from_utf8(seg)
                    .map(std::borrow::Cow::Borrowed)
                    .unwrap_or_else(|_| String::from_utf8_lossy(seg));
                let trimmed = text.strip_suffix('\r').unwrap_or(&text);
                if is_prompt_line(trimmed) {
                    return Ok(Some(trimmed.to_owned()));
                }
            }
            if let Some(idx) = nl {
                line_end = idx;
            } else {
                break;
            }
        }

        end = start;
    }

    Ok(None)
}

fn read_full(file: &mut File, mut out: &mut [u8]) -> io::Result<usize> {
    let mut total = 0;
    while !out.is_empty() {
        match file.read(out) {
            Ok(0) => break,
            Ok(n) => {
                total += n;
                out = &mut out[n..];
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(total)
}

fn has_prompt_prefilter(seg: &[u8]) -> bool {
    contains_bytes(seg, b"\"type\":\"user\"") || contains_bytes(seg, b"\"queued_command\"")
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Incremental per-transcript cache for `promptBefore` (`c4686b4e`).
///
/// - Reuses cached `prompt_line` when `head == prompt_at`.
/// - Scans only `[prompt_at..head]` when the tail window slides forward (`head > prompt_at`) and
///   a previous `prompt_line` was already found, falling back to `had` if no newer prompt appeared
///   in `[prompt_at..head]`.
/// - Leaves `(prompt_at, prompt_line)` untouched on `Err(...)` so transient SSH/I/O failures retry
///   cleanly on the next poll.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PromptBeforeCache {
    prompt_at: Option<u64>,
    prompt_line: Option<String>,
}

impl PromptBeforeCache {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            prompt_at: None,
            prompt_line: None,
        }
    }

    #[must_use]
    pub const fn prompt_at(&self) -> Option<u64> {
        self.prompt_at
    }

    #[must_use]
    pub fn prompt_line(&self) -> Option<&str> {
        self.prompt_line.as_deref()
    }

    pub fn resolve_with<E, F>(&mut self, head: u64, finder: F) -> Result<Option<String>, E>
    where
        F: FnOnce(u64, u64) -> Result<Option<String>, E>,
    {
        if head == 0 {
            self.prompt_at = Some(0);
            self.prompt_line = None;
            return Ok(None);
        }
        if self.prompt_at == Some(head) {
            return Ok(self.prompt_line.clone());
        }
        let since = match (self.prompt_at, &self.prompt_line) {
            (Some(prev), Some(_)) if head > prev => prev,
            _ => 0,
        };
        let had = self.prompt_line.clone();
        let found = finder(head, since)?;
        let next = found.or(if since > 0 { had } else { None });
        self.prompt_at = Some(head);
        self.prompt_line = next.clone();
        Ok(next)
    }

    pub fn resolve_local(&mut self, path: &Path, head: u64) -> io::Result<Option<String>> {
        self.resolve_with(head, |before, since| {
            last_prompt_before_with_options(
                path,
                before,
                ScanOptions {
                    stop_at: since,
                    ..ScanOptions::default()
                },
            )
        })
    }
}

/// Validates SSH `__t_tprompt` parameters (`agent == "claude"`, `[0-9A-Za-z-]{8,100}`, `before > 0`,
/// `start <= before`).
pub fn validate_remote_tprompt_args(
    agent: &str,
    session_id: &str,
    before: u64,
    start: u64,
) -> Result<(), String> {
    if agent != "claude" {
        return Err("Remote transcript prompt scan is only supported for claude.".into());
    }
    if !(8..=100).contains(&session_id.len())
        || !session_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err("Invalid remote transcript session identifier.".into());
    }
    if before == 0 {
        return Err("Remote prompt scan offset 'before' must be greater than zero.".into());
    }
    if start > before {
        return Err("Remote prompt scan 'start' cannot exceed 'before'.".into());
    }
    Ok(())
}

/// Builds the POSIX `sh` + `LC_ALL=C awk` `__t_tprompt` snippet (`c4686b4e`), verifying `[ -f ]`
/// and `[ ! -L ]` and printing `<offset> <length>` for the last matching prompt line before `before`.
pub fn build_remote_tprompt_awk_command(
    agent: &str,
    session_id: &str,
    before: u64,
    start: u64,
) -> Result<String, String> {
    validate_remote_tprompt_args(agent, session_id, before, start)?;
    Ok(format!(
        "for __d in \"$HOME/.claude/projects\"/*; do \
[ -d \"$__d\" ] || continue; \
__c=\"$__d/{session_id}.jsonl\"; \
if [ -f \"$__c\" ] && [ ! -L \"$__c\" ]; then \
LC_ALL=C awk -v b={before} -v s={start} '\
BEGIN {{ o = 0; m_off = -1; m_len = 0 }} \
{{ \
  line = $0; sub(/\\r$/, \"\", line); \
  raw_len = length($0); cur = o; o += raw_len + 1; \
  if (cur >= b) exit; \
  if (cur >= s && index(line, \"\\\"isSidechain\\\":true\") == 0) {{ \
    u = (index(line, \"\\\"type\\\":\\\"user\\\"\") > 0 && index(line, \"\\\"type\\\":\\\"tool_result\\\"\") == 0); \
    q = (index(line, \"\\\"queued_command\\\"\") > 0 && index(line, \"\\\"commandMode\\\":\\\"prompt\\\"\") > 0); \
    y = (index(line, \"\\\"isMeta\\\":true\") == 0 && (index(line, \"\\\"origin\\\"\") == 0 || index(line, \"\\\"kind\\\":\\\"human\\\"\") > 0)); \
    p = (index(line, \"\\\"kind\\\":\\\"peer\\\"\") > 0); \
    if ((u && (y || p)) || (q && y)) {{ m_off = cur; m_len = length(line) }} \
  }} \
}} \
END {{ if (m_off >= 0 && m_len > 0) printf \"%d %d\\n\", m_off, m_len }}' \"$__c\"; \
break; \
fi; \
done"
    ))
}

/// Parses `<offset> <length>` stdout from `__t_tprompt` (`256 B` stdout cap, `4 MiB` line cap).
pub fn parse_remote_tprompt_output(stdout: &str) -> Result<Option<(u64, usize)>, String> {
    if stdout.len() > REMOTE_TPROMPT_STDOUT_CAP {
        return Err("Remote prompt scan output exceeded 256-byte safety cap.".into());
    }
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let mut parts = trimmed.split_whitespace();
    let off_str = parts
        .next()
        .ok_or("Missing offset in __t_tprompt output.")?;
    let len_str = parts
        .next()
        .ok_or("Missing length in __t_tprompt output.")?;
    if parts.next().is_some() {
        return Err("Unexpected extra tokens in __t_tprompt output.".into());
    }
    let offset = off_str
        .parse::<u64>()
        .map_err(|_| "Invalid byte offset in __t_tprompt output.")?;
    let length = len_str
        .parse::<usize>()
        .map_err(|_| "Invalid byte length in __t_tprompt output.")?;
    if length == 0 || length > REMOTE_MAX_PROMPT_LINE_BYTES {
        return Err("Remote prompt line length out of bounds (1..=4 MiB).".into());
    }
    Ok(Some((offset, length)))
}

/// Extracted visible turn message `(role, body, timestamp_ms)` from a Claude/OpenClaude transcript window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranscriptTurnMessage {
    pub sender: String,
    pub body: String,
    pub timestamp_ms: u64,
}

/// Reads the trailing `window_bytes` of a local Claude/OpenClaude `.jsonl` transcript, prepends
/// `promptBefore` when `head > 0` cuts mid-turn, hides peer-origin turns (`"kind":"peer"`), and
/// returns the visible user/assistant conversation messages.
pub fn read_claude_window_messages(
    path: &Path,
    window_bytes: usize,
    cache: &mut PromptBeforeCache,
    fallback_timestamp_ms: u64,
) -> io::Result<Vec<TranscriptTurnMessage>> {
    let sym = fs::symlink_metadata(path)?;
    if sym.file_type().is_symlink() || !sym.is_file() {
        return Ok(Vec::new());
    }
    let size = sym.len();
    if size == 0 || window_bytes == 0 {
        return Ok(Vec::new());
    }

    let mut file = File::open(path)?;
    let raw_head = size.saturating_sub(window_bytes as u64);
    let mut tail = vec![0u8; (size - raw_head) as usize];
    file.seek(SeekFrom::Start(raw_head))?;
    let n = read_full(&mut file, &mut tail)?;
    tail.truncate(n);

    let (head, slice) = if raw_head > 0 {
        if let Some(nl) = tail.iter().position(|&b| b == b'\n') {
            (raw_head + nl as u64 + 1, &tail[nl + 1..])
        } else {
            (size, &[][..])
        }
    } else {
        (0, &tail[..])
    };

    let prepended = if head > 0 {
        cache.resolve_local(path, head)?
    } else {
        let _ = cache.resolve_local(path, 0);
        None
    };

    let body_text = String::from_utf8_lossy(slice);
    let mut lines: Vec<&str> = Vec::new();
    if let Some(p) = prepended.as_deref() {
        lines.push(p);
    }
    for line in body_text.lines() {
        let trimmed = line.strip_suffix('\r').unwrap_or(line);
        if !trimmed.is_empty() {
            lines.push(trimmed);
        }
    }

    let mut out = Vec::new();
    let mut turn_visible = false;
    for (idx, line) in lines.into_iter().enumerate() {
        if line.contains("\"isSidechain\":true") {
            continue;
        }
        if is_prompt_line(line) {
            if is_peer_prompt_line(line) {
                turn_visible = false;
                continue;
            }
            turn_visible = true;
            if let Some((text, ts)) = extract_claude_record_text(line, true) {
                out.push(TranscriptTurnMessage {
                    sender: "You".to_owned(),
                    body: text,
                    timestamp_ms: ts
                        .unwrap_or_else(|| fallback_timestamp_ms.saturating_add(idx as u64)),
                });
            }
            continue;
        }
        if turn_visible
            && line.contains("\"type\":\"assistant\"")
            && let Some((text, ts)) = extract_claude_record_text(line, false)
        {
            out.push(TranscriptTurnMessage {
                sender: "Claude".to_owned(),
                body: text,
                timestamp_ms: ts
                    .unwrap_or_else(|| fallback_timestamp_ms.saturating_add(idx as u64)),
            });
        }
    }
    Ok(out)
}

fn extract_claude_record_text(line: &str, is_user_prompt: bool) -> Option<(String, Option<u64>)> {
    let val: serde_json::Value = serde_json::from_str(line).ok()?;
    let ts = val
        .get("timestamp")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.timestamp_millis().max(0) as u64);

    if is_user_prompt
        && let Some(qc) = val.get("queued_command")
        && let Some(s) = qc
            .get("prompt")
            .or_else(|| qc.get("content"))
            .and_then(|v| v.as_str())
            .or_else(|| {
                val.get("prompt")
                    .or_else(|| val.get("content"))
                    .and_then(|v| v.as_str())
            })
    {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            return Some((trimmed.to_owned(), ts));
        }
    }

    let content = val
        .get("message")
        .and_then(|m| m.get("content"))
        .or_else(|| val.get("content"))?;
    let mut parts = Vec::new();
    match content {
        serde_json::Value::String(s) => {
            let t = s.trim();
            if !t.is_empty() {
                parts.push(t.to_owned());
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                let kind = item.get("type").and_then(|v| v.as_str()).unwrap_or("text");
                if kind == "text"
                    && let Some(t) = item.get("text").and_then(|v| v.as_str())
                {
                    let trimmed = t.trim();
                    if !trimmed.is_empty() {
                        parts.push(trimmed.to_owned());
                    }
                }
            }
        }
        _ => {}
    }
    if parts.is_empty() {
        None
    } else {
        Some((parts.join("\n\n"), ts))
    }
}
