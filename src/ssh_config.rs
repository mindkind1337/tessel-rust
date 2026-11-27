//! Bounded discovery of concrete Host aliases; OpenSSH owns all connection options.
//! Include handling follows the retained Tessel parser. No commands are evaluated.
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

// Transport defaults shared with Vue's sshConnection. Individual requests
// keep their own deadlines, independent of SSH's idle-link detection.
pub(crate) const CONNECT_TIMEOUT: &str = "ConnectTimeout=30";
pub(crate) const KEEPALIVE_INTERVAL: &str = "ServerAliveInterval=15";
pub(crate) const KEEPALIVE_COUNT: &str = "ServerAliveCountMax=12";

#[derive(Default, Debug)]
pub struct Discovery {
    pub aliases: Vec<String>,
    pub truncated: bool,
    pub skipped_files: usize,
}

struct Scan<'a> {
    home: &'a Path,
    identity: crate::system_identity::Identity,
    root: PathBuf,
    result: Discovery,
    cache: HashMap<PathBuf, Option<String>>,
    seen_aliases: HashSet<String>,
    expansions: usize,
    files: usize,
    read_bytes: usize,
    expanded_bytes: usize,
    entries: usize,
    matches: usize,
    glob_work: usize,
    started: Instant,
}

pub fn discover(home: &Path) -> Discovery {
    discover_with_identity(home, crate::system_identity::Identity::detect())
}

fn discover_with_identity(home: &Path, identity: crate::system_identity::Identity) -> Discovery {
    let mut scan = Scan {
        home,
        identity,
        root: home.join(".ssh"),
        result: Default::default(),
        cache: Default::default(),
        seen_aliases: Default::default(),
        expansions: 0,
        files: 0,
        read_bytes: 0,
        expanded_bytes: 0,
        entries: 0,
        matches: 0,
        glob_work: 0,
        started: Instant::now(),
    };
    let file = scan.root.join("config");
    scan.expand(&file, &mut HashSet::new());
    scan.result
}

fn arguments(value: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let (mut quoted, mut escaped) = (false, false);
    for ch in value.chars() {
        if escaped {
            word.push(ch);
            escaped = false;
            continue;
        }
        if quoted && ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            quoted = !quoted;
            continue;
        }
        if !quoted && ch == '#' {
            break;
        }
        if !quoted && crate::agent_names::js_space(ch) {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
        } else {
            word.push(ch);
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

fn directive(line: &str) -> Option<(&str, &str)> {
    let line = crate::agent_names::js_trim(line);
    if line.starts_with('#') {
        return None;
    }
    let split = line.find(|c: char| c == '=' || crate::agent_names::js_space(c))?;
    let (key, value) = line.split_at(split);
    let value = crate::agent_names::js_trim(value);
    Some((
        key,
        crate::agent_names::js_trim(value.strip_prefix('=').unwrap_or(value)),
    ))
}

fn key_name(path: &Path) -> bool {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    name.starts_with("id_")
        || name.starts_with("known_hosts")
        || name.starts_with("authorized_keys")
        || name.starts_with("ssh_host_")
        || [
            "pem", "key", "pub", "ppk", "p12", "pfx", "der", "crt", "cer",
        ]
        .iter()
        .any(|ext| name.ends_with(&format!(".{ext}")))
}

fn key_header(bytes: &[u8]) -> bool {
    let value = String::from_utf8_lossy(bytes);
    let value = value.trim_start_matches('\u{feff}').trim_start();
    [
        "-----BEGIN",
        "PuTTY-User-Key-File",
        "openssh-key-v1",
        "---- BEGIN SSH2",
    ]
    .iter()
    .any(|prefix| value.starts_with(prefix))
}

impl Scan<'_> {
    fn stopped(&mut self) -> bool {
        if self.started.elapsed() > Duration::from_secs(3) {
            self.result.truncated = true;
        }
        self.result.truncated
    }

    fn expand(&mut self, path: &Path, stack: &mut HashSet<PathBuf>) {
        if self.stopped() {
            return;
        }
        if stack.len() >= 16 || self.expansions >= 256 {
            self.result.truncated = true;
            return;
        }
        self.expansions += 1;
        let Ok(canonical) = path.canonicalize() else {
            return;
        };
        if !stack.insert(canonical.clone()) {
            return;
        }
        let text = if let Some(cached) = self.cache.get(&canonical) {
            cached.clone()
        } else {
            let text = self.read_file(&canonical);
            self.cache.insert(canonical.clone(), text.clone());
            text
        };
        if let Some(text) = text {
            for line in text.lines() {
                if self.stopped() {
                    break;
                }
                if self.expanded_bytes + line.len() + 1 > 1024 * 1024 {
                    self.result.truncated = true;
                    break;
                }
                self.expanded_bytes += line.len() + 1;
                let Some((key, value)) = directive(line) else {
                    continue;
                };
                if key.eq_ignore_ascii_case("include") {
                    for pattern in arguments(value) {
                        for include in self.includes(&pattern) {
                            self.expand(&include, stack);
                        }
                    }
                } else if key.eq_ignore_ascii_case("host") {
                    for alias in arguments(value) {
                        if alias.starts_with('!')
                            || alias.contains(['*', '?'])
                            || crate::workspace::validate_host(&alias).is_err()
                        {
                            continue;
                        }
                        if self.seen_aliases.insert(alias.clone()) {
                            if self.result.aliases.len() >= 1000 {
                                self.result.truncated = true;
                                break;
                            }
                            self.result.aliases.push(alias);
                        }
                    }
                }
            }
        }
        stack.remove(&canonical);
    }

    fn read_file(&mut self, path: &Path) -> Option<String> {
        let mut public = path.as_os_str().to_os_string();
        public.push(".pub");
        if key_name(path) || PathBuf::from(public).is_file() {
            return None;
        }
        if self.files >= 64 {
            self.result.truncated = true;
            return None;
        }
        let mut file = File::open(path)
            .map_err(|_| self.result.skipped_files += 1)
            .ok()?;
        let meta = file.metadata().ok()?;
        if !meta.is_file() || meta.len() > 256 * 1024 {
            self.result.skipped_files += 1;
            return None;
        }
        self.files += 1;
        let size = meta.len() as usize;
        if self.read_bytes + size.min(64) > 1024 * 1024 {
            self.result.truncated = true;
            return None;
        }
        let mut head = vec![0; size.min(64)];
        if file.read_exact(&mut head).is_err() {
            self.result.skipped_files += 1;
            return None;
        }
        self.read_bytes += head.len();
        if key_header(&head) {
            return None;
        }
        let mut bytes = head;
        let allowance = size - bytes.len();
        if self.read_bytes + allowance > 1024 * 1024 {
            self.result.truncated = true;
            return None;
        }
        if (&mut file)
            .take(allowance as u64)
            .read_to_end(&mut bytes)
            .is_err()
        {
            self.result.skipped_files += 1;
            return None;
        }
        self.read_bytes += bytes.len().saturating_sub(size.min(64));
        if bytes.len() != size
            || file.metadata().is_ok_and(|after| {
                after.len() != meta.len() || after.modified().ok() != meta.modified().ok()
            })
        {
            self.result.skipped_files += 1;
            return None;
        }
        String::from_utf8(bytes)
            .map_err(|_| self.result.skipped_files += 1)
            .ok()
    }

    fn include_path(&mut self, input: &str) -> Option<PathBuf> {
        let expanded = expand_include(
            input,
            |name| std::env::var(name).ok(),
            |token| match token {
                '%' => Some("%".into()),
                'd' => self.home.to_str().map(str::to_owned),
                'u' => Some(self.identity.username.clone()),
                'l' => Some(self.identity.hostname.clone()),
                'L' => Some(
                    self.identity
                        .hostname
                        .split('.')
                        .next()
                        .unwrap_or_default()
                        .into(),
                ),
                'i' => self.identity.uid.clone(),
                'h' | 'n' | 'p' | 'r' | 'j' | 'k' | 'C' => None,
                other => Some(format!("%{other}")),
            },
        );
        let expanded = match expanded {
            Ok(value) => value,
            Err(ExpansionError::Limit) => {
                self.result.truncated = true;
                return None;
            }
            Err(ExpansionError::Unavailable) => return None,
        };
        if expanded == "~" {
            return Some(self.home.to_owned());
        }
        if expanded.starts_with("~/") || expanded.starts_with("~\\") {
            return Some(self.home.join(&expanded[2..]));
        }
        let path = PathBuf::from(expanded);
        Some(if path.is_absolute() {
            path
        } else {
            self.root.join(path)
        })
    }

    fn includes(&mut self, input: &str) -> Vec<PathBuf> {
        let Some(pattern) = self.include_path(input) else {
            self.result.skipped_files += 1;
            return vec![];
        };
        let mut normalized = PathBuf::new();
        for component in pattern.components() {
            if component == Component::ParentDir {
                normalized.pop();
            } else {
                normalized.push(component.as_os_str());
            }
        }
        let mut bases = vec![PathBuf::new()];
        for component in normalized.components() {
            if self.stopped() {
                return vec![];
            }
            let Component::Normal(part) = component else {
                for base in &mut bases {
                    base.push(component.as_os_str());
                }
                continue;
            };
            let part = part.to_string_lossy();
            if !part.contains(['*', '?', '[']) {
                for base in &mut bases {
                    base.push(part.as_ref());
                }
                continue;
            }
            let mut next = Vec::new();
            for base in bases {
                let Ok(entries) = fs::read_dir(base) else {
                    continue;
                };
                for entry in entries {
                    if self.entries >= 4096 {
                        self.result.truncated = true;
                        return vec![];
                    }
                    self.entries += 1;
                    let Ok(entry) = entry else {
                        continue;
                    };
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if name.starts_with('.') && !part.starts_with('.') {
                        continue;
                    }
                    if glob_match(&part, &name, &mut self.glob_work) {
                        next.push(entry.path());
                    }
                    if self.glob_work > 2_000_000 {
                        self.result.truncated = true;
                        return vec![];
                    }
                }
            }
            next.sort();
            bases = next;
        }
        bases
            .into_iter()
            .filter(|p| p.is_file() && !key_name(p))
            .take_while(|_| {
                if self.matches >= 256 {
                    self.result.truncated = true;
                    return false;
                }
                self.matches += 1;
                true
            })
            .collect()
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ExpansionError {
    Limit,
    Unavailable,
}

const MAX_INCLUDE_PATH: usize = 8192;

fn push_bounded(out: &mut String, part: &str) -> Result<(), ExpansionError> {
    if part.len() > MAX_INCLUDE_PATH.saturating_sub(out.len()) {
        return Err(ExpansionError::Limit);
    }
    out.push_str(part);
    Ok(())
}

fn expand_include(
    input: &str,
    environment: impl Fn(&str) -> Option<String>,
    token: impl Fn(char) -> Option<String>,
) -> Result<String, ExpansionError> {
    if input.len() > MAX_INCLUDE_PATH {
        return Err(ExpansionError::Limit);
    }
    let mut env = String::new();
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        push_bounded(&mut env, &rest[..start])?;
        let Some(close) = rest[start + 2..].find('}').map(|offset| offset + start + 2) else {
            push_bounded(&mut env, &rest[start..])?;
            rest = "";
            break;
        };
        if close == start + 2 {
            push_bounded(&mut env, "${}")?;
        } else {
            let value = environment(&rest[start + 2..close]).ok_or(ExpansionError::Unavailable)?;
            push_bounded(&mut env, &value)?;
        }
        rest = &rest[close + 1..];
    }
    push_bounded(&mut env, rest)?;
    let mut expanded = String::new();
    let mut chars = env.chars();
    while let Some(ch) = chars.next() {
        if ch == '%' {
            let value = match chars.next() {
                Some(next) => token(next).ok_or(ExpansionError::Unavailable)?,
                None => "%".into(),
            };
            push_bounded(&mut expanded, &value)?;
        } else {
            let mut bytes = [0; 4];
            push_bounded(&mut expanded, ch.encode_utf8(&mut bytes))?;
        }
    }
    Ok(expanded)
}

// Iterative DP avoids exponential backtracking on hostile Include patterns.
fn glob_match(pattern: &str, name: &str, work: &mut usize) -> bool {
    let pattern = if cfg!(windows) {
        pattern.to_lowercase()
    } else {
        pattern.to_owned()
    };
    let name = if cfg!(windows) {
        name.to_lowercase()
    } else {
        name.to_owned()
    };
    let pattern: Vec<_> = pattern.chars().collect();
    let name: Vec<_> = name.chars().collect();
    let mut previous = vec![false; name.len() + 1];
    previous[0] = true;
    let mut i = 0;
    while i < pattern.len() {
        let ch = pattern[i];
        let class_end = if ch == '[' {
            pattern[i + 1..]
                .iter()
                .position(|c| *c == ']')
                .map(|n| i + n + 1)
                .filter(|end| *end > i + 1)
        } else {
            None
        };
        let mut next = vec![false; name.len() + 1];
        if ch == '*' {
            next[0] = previous[0];
        }
        for j in 1..=name.len() {
            *work += 1;
            if *work > 2_000_000 {
                return false;
            }
            next[j] = if ch == '*' {
                previous[j] || next[j - 1]
            } else {
                let matches = if let Some(end) = class_end {
                    let mut class = &pattern[i + 1..end];
                    let negate = class.first() == Some(&'!') || class.first() == Some(&'^');
                    if negate {
                        class = &class[1..];
                    }
                    let mut matched = false;
                    let mut k = 0;
                    while k < class.len() {
                        if k + 2 < class.len() && class[k + 1] == '-' {
                            matched |= class[k] <= name[j - 1] && name[j - 1] <= class[k + 2];
                            k += 3;
                        } else {
                            matched |= class[k] == name[j - 1];
                            k += 1;
                        }
                    }
                    matched != negate
                } else {
                    ch == '?' || ch == name[j - 1]
                };
                previous[j - 1] && matches
            };
        }
        previous = next;
        i = class_end.map_or(i + 1, |end| end + 1);
    }
    previous[name.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn includes_quotes_comments_cycles_and_keys_are_handled_without_copying_options() {
        let dir = tempfile::tempdir().unwrap();
        let ssh = dir.path().join(".ssh");
        fs::create_dir_all(ssh.join("conf.d")).unwrap();
        fs::write(ssh.join("config"), "Host main alias # comment\n ProxyCommand never-execute\n Include conf.d/*.conf\n Host * !excluded bad?\n Include id_secret mystery\n Host last\n").unwrap();
        fs::write(
            ssh.join("conf.d/a.conf"),
            "Host=\"included\"\nInclude config\n",
        )
        .unwrap();
        fs::write(ssh.join("id_secret"), "Host MUST_NOT_READ").unwrap();
        fs::write(
            ssh.join("mystery"),
            "-----BEGIN PRIVATE KEY-----\nHost MUST_NOT_READ",
        )
        .unwrap();
        let result = discover(dir.path());
        assert_eq!(result.aliases, ["main", "alias", "included", "last"]);
        assert!(!result.truncated);
    }
    #[test]
    fn broad_include_and_repeated_expansion_are_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let ssh = dir.path().join(".ssh");
        fs::create_dir_all(&ssh).unwrap();
        fs::write(ssh.join("config"), "Include one\n".repeat(300)).unwrap();
        fs::write(ssh.join("one"), "Host discovered\n").unwrap();
        let result = discover(dir.path());
        assert_eq!(result.aliases, ["discovered"]);
        assert!(result.truncated);
    }
    #[test]
    fn glob_classes_and_backtracking_have_a_shared_work_budget() {
        for (pattern, name, expected) in [
            ("*.conf", "dev.conf", true),
            ("[a-c]?", "b1", true),
            ("[!a-c]?", "b1", false),
            ("a*b*c", "abbc", true),
            ("[", "[", true),
        ] {
            assert_eq!(glob_match(pattern, name, &mut 0), expected);
        }
        let mut budget = 2_000_000;
        assert!(!glob_match("*", "file", &mut budget));
        assert!(budget > 2_000_000);
    }

    #[test]
    fn public_key_siblings_and_oversized_files_are_not_parsed() {
        let dir = tempfile::tempdir().unwrap();
        let ssh = dir.path().join(".ssh");
        fs::create_dir_all(&ssh).unwrap();
        fs::write(
            ssh.join("config"),
            "Include %d/.ssh/paired %d/.ssh/large\nHost available\n",
        )
        .unwrap();
        fs::write(ssh.join("paired"), "Host private_key_alias\n").unwrap();
        fs::write(ssh.join("paired.pub"), "synthetic public key marker").unwrap();
        fs::write(
            ssh.join("large"),
            format!("Host oversized\n{}", "#".repeat(256 * 1024)),
        )
        .unwrap();
        let result = discover(dir.path());
        assert_eq!(result.aliases, ["available"]);
        assert_eq!(result.skipped_files, 1);
    }

    #[test]
    fn distinct_config_files_share_one_read_byte_budget() {
        let dir = tempfile::tempdir().unwrap();
        let ssh = dir.path().join(".ssh");
        fs::create_dir_all(&ssh).unwrap();
        fs::write(ssh.join("config"), "Include part*\n").unwrap();
        for index in 0..6 {
            let text = format!("Host alias{index}\n#{}", "x".repeat(220 * 1024));
            fs::write(ssh.join(format!("part{index}")), text).unwrap();
        }
        let result = discover(dir.path());
        assert!(result.truncated);
        assert_eq!(result.aliases, ["alias0", "alias1", "alias2", "alias3"]);
    }
    #[test]
    fn env_and_token_expansion_stop_before_an_oversized_allocation() {
        assert_eq!(
            expand_include(
                &"${VALUE}".repeat(34),
                |_| Some("x".repeat(32000)),
                |_| None
            ),
            Err(ExpansionError::Limit)
        );
        assert_eq!(
            expand_include("%d%d", |_| None, |_| Some("x".repeat(5000))),
            Err(ExpansionError::Limit)
        );
        assert_eq!(
            expand_include("${MISSING}", |_| None, |_| None),
            Err(ExpansionError::Unavailable)
        );
        assert_eq!(
            expand_include(
                "${ROOT}/%%/%d",
                |_| Some("folder".into()),
                |ch| match ch {
                    '%' => Some("%".into()),
                    'd' => Some("home".into()),
                    _ => None,
                }
            )
            .unwrap(),
            "folder/%/home"
        );
        let mut bounded = "a".repeat(MAX_INCLUDE_PATH - 1);
        assert_eq!(push_bounded(&mut bounded, "é"), Err(ExpansionError::Limit));
        assert_eq!(bounded.len(), MAX_INCLUDE_PATH - 1);
    }
    #[test]
    fn directive_whitespace_equals_and_literal_environment_syntax_match_source() {
        assert_eq!(
            directive(" \u{feff} Host = fixture "),
            Some(("Host", "fixture"))
        );
        assert_eq!(directive("Host==invalid"), Some(("Host", "=invalid")));
        assert_eq!(
            arguments("one\u{feff}two\u{0085}three"),
            ["one", "two\u{0085}three"]
        );
        assert_eq!(
            expand_include("literal${}and${unfinished", |_| None, |_| None).unwrap(),
            "literal${}and${unfinished"
        );
    }
    #[test]
    fn system_identity_tokens_use_the_discovery_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let ssh = dir.path().join(".ssh");
        fs::create_dir_all(&ssh).unwrap();
        fs::write(ssh.join("config"), "Include %u-%l-%L-%i.conf\n").unwrap();
        fs::write(
            ssh.join("fixture-machine.example-machine-42.conf"),
            "Host from_identity\n",
        )
        .unwrap();
        let identity = crate::system_identity::Identity {
            username: "fixture".into(),
            hostname: "machine.example".into(),
            uid: Some("42".into()),
        };
        let result = discover_with_identity(dir.path(), identity);
        assert_eq!(result.aliases, ["from_identity"]);
        assert!(!result.truncated);
        let unavailable = discover_with_identity(dir.path(), Default::default());
        assert!(unavailable.aliases.is_empty());
        assert_eq!(unavailable.skipped_files, 1);
    }
}
