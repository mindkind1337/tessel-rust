//! Bounded guest-facing text. No filesystem access; roots are lexical allowlists.
//! Adapted from the public-text contract in Tessel's liveShare/shareChat.js.
use regex::{Captures, Regex};
use std::sync::OnceLock;

const MASK: &str = "••••";
const HIDDEN: &str = "…";
macro_rules! rx {
    ($pattern:expr) => {{
        static RE: OnceLock<Regex> = OnceLock::new();
        RE.get_or_init(|| Regex::new($pattern).expect("constant guest-text pattern"))
    }};
}

fn clip(text: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let mut chars = text.chars();
    let mut out: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        out.pop();
        out.push('…');
    }
    out
}

fn mask_uri_userinfo(text: &str) -> String {
    rx!(r#"(?i)([a-z][a-z0-9+.-]{1,20}://)[^\s/@"'`<>]{1,1024}@"#)
        .replace_all(text, |c: &Captures<'_>| format!("{}{MASK}@", &c[1]))
        .into_owned()
}

fn mask_encoded_values(text: &str) -> String {
    // Decoding an unquoted value must not turn one encoded token into two words,
    // exposing the second word. Encoded opening quotes are handled after decoding.
    rx!(concat!(r#"(?i)(?-u:\b)([A-Za-z0-9_.-]{0,40}(?:token|secret|passw(?:or)?d|passwd|pwd|api[_-]?key|apikey|access[_-]?key|private[_-]?key|client[_-]?secret|credential|session[_-]?key|connection[_-]?string)[A-Za-z0-9_.-]{0,40})"#,
        r#"(["']?[ \t]{0,4}[:=][ \t]{0,4})([^\s"',;&]+)"#))
        .replace_all(text, |c: &Captures<'_>| {
            let mut prefix = c[3].to_ascii_lowercase();
            for _ in 0..6 { prefix = prefix.replace("%25", "%"); }
            if !prefix.starts_with("%22") && !prefix.starts_with("%27")
                && rx!(r"%[0-9A-Fa-f]{2}").is_match(&c[3]) {
                format!("{}{}{MASK}", &c[1], &c[2])
            } else { c[0].to_owned() }
        }).into_owned()
}

fn plain_text(text: &str) -> String {
    let mut out = rx!(r"\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07\x1b]*(?:\x07|\x1b\\)|[PX^_][^\x1b]*\x1b\\|[@-Z\\-_])|[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]").replace_all(text, "").into_owned();
    out = mask_encoded_values(&mask_uri_userinfo(&out));
    let percent = rx!(r"%([0-9A-Fa-f]{2})");
    for _ in 0..6 {
        let next = percent
            .replace_all(&out, |c: &Captures<'_>| {
                let byte = u8::from_str_radix(&c[1], 16).unwrap();
                if (0x20..0x7f).contains(&byte) {
                    char::from(byte).to_string()
                } else {
                    c[0].to_owned()
                }
            })
            .into_owned();
        if next == out {
            break;
        }
        out = mask_encoded_values(&mask_uri_userinfo(&next));
    }
    out = rx!(r"(?i)\\\\[?.]\\UNC\\")
        .replace_all(&out, regex::NoExpand(r"\\"))
        .into_owned();
    out = rx!(r"\\\\[?.]\\([A-Za-z]:)")
        .replace_all(&out, "$1")
        .into_owned();
    out.retain(|c| !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}' | '\u{061c}' | '\u{feff}'));
    out
}

fn mask_secrets(text: &str) -> String {
    let mut out =
        rx!(r"-----BEGIN [A-Z ]*PRIVATE KEY-----(?s:.*?)(?:-----END [A-Z ]*PRIVATE KEY-----|$)")
            .replace_all(text, MASK)
            .into_owned();
    out = rx!(r#"(?i)(?-u:\b)([a-z][a-z0-9+.-]{1,20}://)[^\s/@:]{1,200}:[^\s/@]{1,200}@"#)
        .replace_all(&out, |c: &Captures<'_>| format!("{}{MASK}@", &c[1]))
        .into_owned();
    out = rx!(concat!(
        r"(?-u:\b)(?:sk|pk|rk)[-_](?:ant-|proj-|live[-_]|test[-_]|or-)?[A-Za-z0-9_-]{16,}",
        r"|(?-u:\b)(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{20,}|(?-u:\b)github_pat_[A-Za-z0-9_]{20,}",
        r"|(?-u:\b)glpat-[A-Za-z0-9_-]{10,}|(?-u:\b)glsa_[A-Za-z0-9_]{20,}|(?-u:\b)do[por]_v1_[A-Za-z0-9]{20,}",
        r"|(?-u:\b)(?:xai-|npm_|hf_)[A-Za-z0-9]{20,}|(?-u:\b)ya29\.[A-Za-z0-9_-]{10,}|(?-u:\b)xox[a-z]-[A-Za-z0-9-]{6,}",
        r"|(?-u:\b)(?:AKIA|ASIA|AGPA|AIDA|AROA)[0-9A-Z]{12,}|(?-u:\b)AIza[0-9A-Za-z_-]{20,}",
        r"|(?-u:\b)eyJ[A-Za-z0-9_-]{6,}(?:\.[A-Za-z0-9_-]{2,}){1,2}"
    )).replace_all(&out, MASK).into_owned();
    out = rx!(r#"(?i)(?-u:\b)(AccountKey|SharedAccessKey|SharedAccessSignature|sig)=[^;&\s"']+"#)
        .replace_all(&out, |c: &Captures<'_>| format!("{}={MASK}", &c[1]))
        .into_owned();
    out = rx!(r"(?-u:\b)(Bearer|Basic|bearer)\s+([A-Za-z0-9._~+/=-]{12,})")
        .replace_all(&out, |c: &Captures<'_>| {
            if c[2].bytes().any(|b| b.is_ascii_digit() || b == b'=') {
                format!("{} {MASK}", &c[1])
            } else {
                c[0].to_owned()
            }
        })
        .into_owned();
    // Also mask short values and an unterminated quote at the input budget boundary.
    out = rx!(concat!(r#"(?i)(?-u:\b)([A-Za-z0-9_.-]{0,40}(?:token|secret|passw(?:or)?d|passwd|pwd|api[_-]?key|apikey|access[_-]?key|private[_-]?key|client[_-]?secret|credential|session[_-]?key|connection[_-]?string)[A-Za-z0-9_.-]{0,40})"#,
        r#"(["']?[ \t]{0,4}[:=][ \t]{0,4})("[^"\r\n]*(?:"|$)|'[^'\r\n]*(?:'|$)|[^\s"',;&]+)"#))
        .replace_all(&out, |c: &Captures<'_>| {
            let quote = c[3].chars().next().filter(|ch| matches!(ch, '\'' | '"'));
            let value = quote.map_or_else(|| MASK.to_owned(), |q| format!("{q}{MASK}{q}"));
            format!("{}{}{value}", &c[1], &c[2])
        }).into_owned();
    out = rx!(r#"(?i)(?-u:\b)((?:mysql|mysqldump|mysqladmin|mariadb|mariadb-dump)(?-u:\b)[^\n]{0,300}?\s-p)([^\s"']+)"#).replace_all(&out, |c: &Captures<'_>| format!("{}{MASK}", &c[1])).into_owned();
    rx!(r"[A-Za-z0-9+/_=-]{40,}")
        .replace_all(&out, |c: &Captures<'_>| {
            let m = c.get(0).unwrap();
            let previous = out[..m.start()].chars().next_back();
            let path_or_word = previous.is_some_and(|p| {
                p.is_ascii_alphanumeric() || matches!(p, '_' | '/' | '\\' | '.' | '-')
            });
            let bytes = m.as_str().as_bytes();
            if !path_or_word
                && bytes.iter().any(u8::is_ascii_digit)
                && bytes.iter().any(u8::is_ascii_lowercase)
                && bytes.iter().any(u8::is_ascii_uppercase)
            {
                MASK.to_owned()
            } else {
                m.as_str().to_owned()
            }
        })
        .into_owned()
}

#[derive(Clone)]
struct NormalPath {
    raw: String,
    compared: String,
    windows: bool,
}
fn normalized(path: &str) -> Option<NormalPath> {
    let mut s = path.trim().replace('\\', "/");
    if s.starts_with("//?/") || s.starts_with("//./") {
        s = if s[4..].to_ascii_lowercase().starts_with("unc/") {
            format!("//{}", &s[8..])
        } else {
            s[4..].to_owned()
        };
    }
    if s.len() > 4096 {
        return None;
    }
    let drive = s.as_bytes().get(1) == Some(&b':')
        && s.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && s.as_bytes().get(2) == Some(&b'/');
    let unc = s.starts_with("//") && !s.starts_with("///");
    if !drive && !unc && !s.starts_with('/') {
        return None;
    }
    let floor = if drive {
        1
    } else if unc {
        2
    } else {
        0
    };
    let mut parts = Vec::new();
    for part in s.split('/').filter(|p| !p.is_empty() && *p != ".") {
        if part == ".." {
            if parts.len() > floor {
                parts.pop();
            }
        } else {
            parts.push(part);
        }
    }
    let raw = format!(
        "{}{}",
        if unc {
            "//"
        } else if drive {
            ""
        } else {
            "/"
        },
        parts.join("/")
    );
    let windows = drive || unc;
    Some(NormalPath {
        compared: if windows {
            raw.to_lowercase()
        } else {
            raw.clone()
        },
        raw,
        windows,
    })
}
fn roots(values: &[&str]) -> Vec<NormalPath> {
    values
        .iter()
        .take(8)
        .filter_map(|v| {
            if v.len() > 1024 || v.chars().any(char::is_control) {
                return None;
            }
            let n = normalized(v)?;
            if n.raw == "/"
                || n.raw.len() == 2 && n.raw.ends_with(':')
                || n.raw.starts_with("//") && !n.raw[2..].contains('/')
            {
                None
            } else {
                Some(n)
            }
        })
        .collect()
}
fn rewrite(path: &str, roots: &[NormalPath]) -> String {
    let Some(n) = normalized(path) else {
        return HIDDEN.into();
    };
    for root in roots {
        if root.windows != n.windows {
            continue;
        }
        if n.compared == root.compared {
            return ".".into();
        }
        if n.compared.starts_with(&(root.compared.clone() + "/")) {
            // Compare components rather than byte offsets: Unicode case changes can change length.
            let count = root.raw.split('/').filter(|s| !s.is_empty()).count();
            return n
                .raw
                .split('/')
                .filter(|s| !s.is_empty())
                .skip(count)
                .collect::<Vec<_>>()
                .join("/");
        }
    }
    HIDDEN.into()
}
fn split_tail(value: &str) -> (&str, &str) {
    let path =
        value.trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '}', '\'', '"', '`']);
    (&value[..path.len()], &value[path.len()..])
}
fn path_end(text: &str) -> usize {
    for (i, ch) in text.char_indices() {
        if matches!(ch, '"' | '\'' | '`' | '<' | '>' | '|' | '*' | '?') {
            return i;
        }
        if ch.is_whitespace() {
            if !matches!(ch, ' ' | '\t') {
                return i;
            }
            let next = text[i + ch.len_utf8()..]
                .split(|c: char| {
                    c.is_whitespace()
                        || matches!(c, '"' | '\'' | '`' | '<' | '>' | '|' | ')' | ',' | ';')
                })
                .next()
                .unwrap_or("");
            let filename = next.rsplit_once('.').is_some_and(|(base, ext)| {
                !base.is_empty()
                    && base.len() <= 64
                    && (1..=8).contains(&ext.len())
                    && ext.bytes().all(|b| b.is_ascii_alphanumeric())
            });
            if !next.contains(['/', '\\']) && !filename {
                return i;
            }
        }
    }
    text.len()
}
fn hide_paths(text: &str, allowed: &[NormalPath]) -> String {
    // Host logins are handled before paths so replacing the path cannot conceal their context.
    let mut out = rx!(r"[A-Za-z0-9_.-]{1,64}@[A-Za-z0-9][A-Za-z0-9.-]{0,252}")
        .replace_all(text, |c: &Captures<'_>| {
            let m = c.get(0).unwrap();
            let suffix = &text[m.end()..];
            let path = suffix
                .strip_prefix(':')
                .is_some_and(|s| s.starts_with(['/', '\\', '~', '.']));
            let command = rx!(r"(?i)(?-u:\b)(?:ssh|scp|sftp|rsync|mosh)(?-u:\b)[^\n]{0,200}\s$")
                .is_match(&text[..m.start()]);
            if path || command {
                MASK.to_owned()
            } else {
                m.as_str().to_owned()
            }
        })
        .into_owned();
    out = rx!(r#"(?i)(?-u:\b)(?:ssh|sftp|scp|smb|ftps?)://[^\s"'`<>]+"#)
        .replace_all(&out, |c: &Captures<'_>| {
            format!("{HIDDEN}{}", split_tail(&c[0]).1)
        })
        .into_owned();
    out = rx!(r#"(?i)(?-u:\b)file:///?[^\s"'`<>)\]]+"#)
        .replace_all(&out, |c: &Captures<'_>| {
            let (value, tail) = split_tail(&c[0]);
            let mut path = &value[7..];
            if path.starts_with('/') && path.as_bytes().get(2) == Some(&b':') {
                path = &path[1..];
            }
            format!("{}{tail}", rewrite(path, allowed))
        })
        .into_owned();
    for pattern in [
        rx!(r#""((?:[A-Za-z]:[\\/]|\\\\|//|~[\\/]|/[^/"\r\n]+/)[^"\r\n]{0,4096})""#),
        rx!(r#"'((?:[A-Za-z]:[\\/]|\\\\|//|~[\\/]|/[^/'\r\n]+/)[^'\r\n]{0,4096})'"#),
        rx!(r#"`((?:[A-Za-z]:[\\/]|\\\\|//|~[\\/]|/[^/`\r\n]+/)[^`\r\n]{0,4096})`"#),
    ] {
        // Each delimiter allows the other quote characters inside a filename.
        out = pattern
            .replace_all(&out, |c: &Captures<'_>| {
                let q = c[0].chars().next().unwrap();
                format!("{q}{}{q}", rewrite(&c[1], allowed))
            })
            .into_owned();
    }
    let mut result = String::new();
    let mut i = 0;
    let path_pattern_0 = rx!(r"(?i)^[a-z][a-z0-9+.-]{1,20}://");
    let path_pattern_1 = rx!(r"(?i)^(?:https?|wss?)://");
    let path_pattern_2 = rx!(r"^-{1,2}[A-Za-z][A-Za-z0-9_-]{0,30}=?$");
    let path_pattern_3 = rx!(r"^(?:\\\\[^\s\\/]+\\|//[^\s\\/]+/)");
    let path_pattern_4 = rx!(
        r"(?i)^\\(?:Users|Windows|ProgramData|Program Files(?: \(x86\))?|Documents and Settings|AppData)\\"
    );
    let path_pattern_5 = rx!(
        r"(?i)^(?:\$\{?(?:HOME|USERPROFILE|PWD|OLDPWD|TMPDIR|XDG_[A-Z_]{1,20})\}?[\\/]|%(?:USERPROFILE|HOMEPATH|APPDATA|LOCALAPPDATA|TEMP|TMP)%[\\/])"
    );
    let path_pattern_6 = rx!(r"^~[A-Za-z0-9_.-]{0,32}[\\/]");
    let path_pattern_7 = rx!(r#"^/[^/\s"'`<>|\\]+/"#);
    let path_pattern_8 = rx!(r"^/(etc|root|home|var|proc|sys|usr|opt|srv)(?:$|[^A-Za-z0-9_.-])");
    while i < out.len() {
        let rest = &out[i..];
        // Preserve the scheme, but inspect non-web URI bodies for filesystem paths.
        if let Some(m) = path_pattern_0.find(rest) {
            let end = rest
                .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '<' | '>'))
                .unwrap_or(rest.len())
                .max(m.end());
            let ordinary_web = path_pattern_1.is_match(rest) && !rest[..end].contains('\\');
            let keep = if ordinary_web { end } else { m.end() };
            result.push_str(&rest[..keep]);
            i += keep;
            continue;
        }
        let previous = out[..i].chars().next_back();
        let boundary = previous.is_none_or(|c| {
            !c.is_ascii_alphanumeric() && !matches!(c, '_' | '\\' | '/' | ':' | '.' | '~' | '-')
        });
        let token_prefix = out[..i].rsplit(char::is_whitespace).next().unwrap_or("");
        let flag = path_pattern_2.is_match(token_prefix);
        let host = previous == Some(':');
        let drive = rest.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
            && rest.as_bytes().get(1) == Some(&b':')
            && match rest.as_bytes().get(2) {
                Some(b'\\') => previous.is_none_or(|p| !matches!(p, '\\' | '.')),
                Some(b'/') => {
                    previous.is_none_or(|p| {
                        !p.is_ascii_alphanumeric() && !matches!(p, '_' | '\\' | '/' | '.')
                    }) && rest.as_bytes().get(3) != Some(&b'/')
                }
                _ => false,
            };
        let unc =
            (boundary || flag || host || previous == Some('/')) && path_pattern_3.is_match(rest);
        let rooted_windows = if boundary || previous == Some('/') {
            path_pattern_4.find(rest).map(|m| m.end())
        } else {
            None
        };
        let home = path_pattern_5.is_match(rest) || boundary && path_pattern_6.is_match(rest);
        let parent = boundary && (rest.starts_with("../") || rest.starts_with("..\\"));
        let unix_boundary = boundary || flag || host;
        let unix = unix_boundary && path_pattern_7.is_match(rest);
        let system = if unix_boundary && !unix {
            path_pattern_8.captures(rest).map(|c| c[1].len() + 1)
        } else {
            None
        };
        let repeated = boundary && rest.starts_with("///");
        let end = system.unwrap_or_else(|| {
            if drive || unc || rooted_windows.is_some() || home || parent || unix || repeated {
                rooted_windows.map_or_else(
                    || path_end(rest),
                    |prefix| prefix + path_end(&rest[prefix..]),
                )
            } else {
                0
            }
        });
        let (path, tail) = split_tail(&rest[..end]);
        if end > 0
            && (drive
                || unc
                || rooted_windows.is_some()
                || home
                || parent
                || repeated
                || unix
                || system.is_some())
        {
            result.push_str(&if home || parent || repeated || system.is_some() {
                HIDDEN.into()
            } else {
                rewrite(path, allowed)
            });
            result.push_str(tail);
            i += end;
        } else {
            let ch = rest.chars().next().unwrap();
            result.push(ch);
            i += ch.len_utf8();
        }
    }
    result
}

pub fn guest_text(text: &str, project_roots: &[&str], max: usize) -> String {
    let max = max.min(4096);
    let bounded = clip(text, max.saturating_add(256));
    clip(
        &hide_paths(&mask_secrets(&plain_text(&bounded)), &roots(project_roots)),
        max,
    )
}

/// Guest-written cards are untrusted plain text, never terminal markup.
pub fn clean_task_input(text: &str) -> Option<String> {
    if text.len() > 4096 {
        return None;
    }
    let clean = rx!(
        r"\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07\x1b]*(?:\x07|\x1b\\)|[PX^_][^\x1b]*\x1b\\|[@-Z\\-_])"
    )
    .replace_all(text, "");
    let clean: String = clean
        .chars()
        .filter_map(|c| {
            if matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' |
            '\u{200e}' | '\u{200f}' | '\u{061c}' | '\u{feff}')
            {
                None
            } else if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
                Some(' ')
            } else {
                Some(c)
            }
        })
        .collect();
    let clean = clean.trim();
    (!clean.is_empty() && clean.chars().count() <= 300).then(|| clean.to_owned())
}

pub fn task_title(text: &str, root: &str, max: usize) -> String {
    // Accepted wire titles are <=4096 bytes. Scrub the entire accepted input before
    // clipping: earlier redactions must not bring a truncated secret into view.
    let clean: String = guest_text(text, &[root], 4096)
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
                ' '
            } else {
                c
            }
        })
        .collect();
    clip(&clean, max.min(4096))
}
