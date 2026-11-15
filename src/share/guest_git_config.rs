//! Fail-closed validation of where a repository's Git configuration comes from, so a
//! guest cannot smuggle a program-running key (`filter.*`, `diff.*`, `merge.*`, remote
//! helpers, hook paths…) into a host Git command by editing — or by later filling — a
//! config file the host would read.
//!
//! `guest_git_host`'s fixed `-c` prefix overrides the *known* dangerous keys
//! unconditionally, and its per-name neutralizers cover every risky key it *saw* when it
//! enumerated the config. The residual that leaves is a risky key that is **not present at
//! enumeration time** but appears before the command runs, because the config lives in — or
//! is `include`d from — a file the guest can write. This module closes that vector: it
//! refuses the operation when any config key's origin, or any `include`/`includeIf` target
//! (even one that is currently empty or absent), resolves to a guest-writable location.
//!
//! This is authorized defensive hardening for Tessel's Live Share: it only ever *refuses*;
//! it never runs Git, edits files, or prints the content of a config/secret file.
//!
//! The host Git adapter must:
//!   * run `git config -z --show-origin --includes --list` and pass the raw bytes here;
//!   * pass an **absolute** `cwd` (the worktree Git ran in) and the **absolute** paths of
//!     **every** shared root the guest can edit (not just the current project);
//!   * ALSO fail closed on `Capture.truncated` before calling (defense in depth — a stream
//!     could in theory be cut exactly on a NUL boundary and still parse).
use super::private_path;
use std::path::{Component, Path, PathBuf};

type Result<T> = std::result::Result<T, String>;

/// `Ok(())` when the configuration is safe to act on; `Err(message)` (bounded, no file
/// content) when any origin or include target is guest-writable, or when the stream cannot
/// be parsed safely. Fail-closed throughout.
pub(super) fn validate_config_origins(
    bytes: &[u8],
    cwd: &Path,
    shared_roots: &[PathBuf],
) -> Result<()> {
    for rec in parse_records(bytes)? {
        let origin_file = match rec.origin_kind()? {
            // Our own `-c` hardening flags and the process environment are host-controlled.
            OriginKind::CommandLine => continue,
            OriginKind::File(path) => resolve(cwd, path),
        };
        // 1. The file the key itself comes from must not be guest-writable.
        if guest_writable(&origin_file, shared_roots)? {
            return Err(
                "Git configuration is loaded from an editable file; the operation is refused."
                    .into(),
            );
        }
        // 2. An include/includeIf target is validated even when it is empty or absent: the
        //    guest can fill it after the scan, and git would then read it.
        if let Some(target) = rec.include_target() {
            let base = origin_file.parent().unwrap_or(&origin_file);
            let resolved = expand_include_target(target, base)?;
            if guest_writable(&resolved, shared_roots)? {
                return Err(
                    "A Git include points into an editable file; the operation is refused.".into(),
                );
            }
        }
    }
    Ok(())
}

struct Record<'a> {
    origin: &'a str,
    key: &'a str,
    value: &'a str,
}

enum OriginKind<'a> {
    File(&'a str),
    CommandLine,
}

impl<'a> Record<'a> {
    fn origin_kind(&self) -> Result<OriginKind<'a>> {
        if let Some(path) = self.origin.strip_prefix("file:") {
            Ok(OriginKind::File(path))
        } else if self.origin == "command line:" {
            Ok(OriginKind::CommandLine)
        } else {
            // blob:, standard input:, submodule-blob:, or an unrecognized framing: a source
            // whose writability cannot be reasoned about is refused.
            Err(
                "Git configuration comes from an unrecognized source; the operation is refused."
                    .into(),
            )
        }
    }

    /// The target of an `include.path` / `includeIf.<cond>.path` directive, if this is one.
    /// Section and variable are case-insensitive; the subsection (`<cond>`) is not touched.
    fn include_target(&self) -> Option<&'a str> {
        let lower = self.key.to_ascii_lowercase();
        let is_include = lower == "include.path";
        let is_include_if = lower.starts_with("includeif.") && lower.ends_with(".path");
        (is_include || is_include_if).then_some(self.value.trim())
    }
}

/// Parse `git config -z --show-origin --includes --list`.
///
/// Framing (verified against a real Git fixture): each entry is
/// `<origin>\0<key>\n<value>\0`, so splitting on NUL yields alternating origin / key-value
/// tokens. A non-UTF-8 stream, a stream not terminated by NUL (truncated), an odd token
/// count, or a token that does not look like an origin all fail closed.
fn parse_records(bytes: &[u8]) -> Result<Vec<Record<'_>>> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    if !bytes.ends_with(&[0]) {
        return Err("The Git configuration stream is truncated; the operation is refused.".into());
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "The Git configuration is not UTF-8; the operation is refused.".to_string())?;
    let mut tokens: Vec<&str> = text.split('\0').collect();
    // The trailing NUL leaves one empty token at the end.
    if tokens.last() == Some(&"") {
        tokens.pop();
    }
    if !tokens.len().is_multiple_of(2) {
        return Err("The Git configuration framing is malformed; the operation is refused.".into());
    }
    let mut records = Vec::with_capacity(tokens.len() / 2);
    for pair in tokens.as_chunks::<2>().0.iter() {
        let origin = pair[0];
        // Guard the framing assumption: a well-formed origin starts with a known kind.
        if !(origin.starts_with("file:")
            || origin == "command line:"
            || origin.starts_with("blob:")
            || origin == "standard input:"
            || origin.starts_with("submodule-blob:"))
        {
            return Err(
                "The Git configuration framing is unexpected; the operation is refused.".into(),
            );
        }
        let (key, value) = pair[1].split_once('\n').unwrap_or((pair[1], ""));
        records.push(Record { origin, key, value });
    }
    Ok(records)
}

/// A path is guest-writable when it sits inside a shared root the guest can edit — tested
/// both lexically and canonically — and is not a host-managed, non-editable path
/// (`.git`, private/credential locations). Checking the lexical placement as well as the
/// canonical one means a symlink the guest can replace under a shared root counts as
/// writable even while it currently points elsewhere.
fn guest_writable(path: &Path, shared_roots: &[PathBuf]) -> Result<bool> {
    // Lexical placement: the location the guest can manipulate, symlink target aside.
    let lexical = lexical_normalize(path);
    if !private_path(&lexical) {
        let roots: Vec<PathBuf> = shared_roots.iter().map(|r| lexical_normalize(r)).collect();
        if under_any(&lexical, &roots) {
            return Ok(true);
        }
    }
    // Canonical placement: a path that resolves inside a shared root through a link.
    let canonical = canonical_existing_prefix(path)
        .ok_or("Git configuration path cannot be resolved safely.")?;
    {
        if !private_path(&canonical) {
            let roots: Vec<PathBuf> = shared_roots
                .iter()
                .filter_map(|r| canonical_existing_prefix(r))
                .collect();
            if under_any(&canonical, &roots) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

pub(super) fn comparison_path(path: &Path) -> PathBuf {
    let normalized = lexical_normalize(path);
    #[cfg(windows)]
    {
        PathBuf::from(
            dunce::simplified(&normalized)
                .to_string_lossy()
                .to_lowercase(),
        )
    }
    #[cfg(not(windows))]
    {
        normalized
    }
}
fn under_any(candidate: &Path, roots: &[PathBuf]) -> bool {
    let candidate = comparison_path(candidate);
    roots
        .iter()
        .any(|root| !root.as_os_str().is_empty() && candidate.starts_with(comparison_path(root)))
}

/// Resolve `.`/`..` without touching the filesystem. `..` that would climb above the start
/// is kept literally so an escaping path never collapses to look contained.
pub(super) fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Canonicalize the deepest existing ancestor of `path` and re-append the non-existent
/// tail, so an include target that does not exist yet still resolves through real symlinks.
pub(super) fn canonical_existing_prefix(path: &Path) -> Option<PathBuf> {
    let mut current = path;
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(canonical) = std::fs::canonicalize(current) {
            let mut full = canonical;
            for part in tail.iter().rev() {
                full.push(part);
            }
            return Some(full);
        }
        match current.parent() {
            Some(parent) if parent != current => {
                tail.push(current.file_name()?.to_os_string());
                current = parent;
            }
            _ => return None,
        }
    }
}

fn resolve(cwd: &Path, origin_path: &str) -> PathBuf {
    let path = Path::new(origin_path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

/// Expand an `include.path` value the way git does: `~/` to the home directory, an absolute
/// path as-is, anything else relative to the including file's directory. An empty target,
/// `~user`, or a `%(...)`/unknown expansion cannot be reasoned about and fails closed.
fn expand_include_target(value: &str, base: &Path) -> Result<PathBuf> {
    if value.is_empty() {
        return Err("A Git include has no target; the operation is refused.".into());
    }
    if value.contains("%(") {
        return Err(
            "A Git include uses an unsupported expansion; the operation is refused.".into(),
        );
    }
    if let Some(rest) = value.strip_prefix('~') {
        if rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\') {
            let home = std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .ok_or_else(|| {
                    "A Git include uses '~' but no home is known; the operation is refused."
                        .to_string()
                })?;
            let rel = rest.trim_start_matches(['/', '\\']);
            return Ok(Path::new(&home).join(rel));
        }
        // `~user` cannot be expanded portably.
        return Err("A Git include uses '~user'; the operation is refused.".into());
    }
    let path = Path::new(value);
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(base.join(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\srv\project")
        } else {
            PathBuf::from("/srv/project")
        }
    }

    fn roots() -> Vec<PathBuf> {
        vec![root()]
    }

    fn file_origin(rel: &str) -> String {
        format!("file:{}", root().join(rel).to_string_lossy())
    }

    /// Build a `-z --show-origin` stream from (origin, key, value) triples.
    fn stream(entries: &[(&str, &str, &str)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (origin, key, value) in entries {
            out.extend_from_slice(origin.as_bytes());
            out.push(0);
            out.extend_from_slice(key.as_bytes());
            if !value.is_empty() {
                out.push(b'\n');
                out.extend_from_slice(value.as_bytes());
            }
            out.push(0);
        }
        out
    }

    #[test]
    fn no_config_is_safe() {
        assert!(validate_config_origins(b"", &root(), &roots()).is_ok());
    }

    #[test]
    fn config_from_the_git_dir_is_allowed() {
        // `.git/config` sits under the root but is non-editable (private_path exempts it).
        let s = stream(&[(&file_origin(".git/config"), "core.bare", "false")]);
        assert!(validate_config_origins(&s, &root(), &roots()).is_ok());
    }

    #[test]
    fn system_config_outside_the_roots_is_allowed() {
        let outside = if cfg!(windows) {
            "file:C:\\ProgramData\\Git\\config"
        } else {
            "file:/etc/gitconfig"
        };
        let s = stream(&[(outside, "filter.lfs.process", "git-lfs filter-process")]);
        assert!(validate_config_origins(&s, &root(), &roots()).is_ok());
    }

    #[test]
    fn config_from_an_editable_worktree_file_is_refused() {
        let s = stream(&[(&file_origin("vendor/mod.gitconfig"), "core.editor", "vi")]);
        assert!(validate_config_origins(&s, &root(), &roots()).is_err());
    }

    #[test]
    fn an_include_into_the_editable_worktree_is_refused_even_when_absent() {
        // Included from `.git/config` (exempt itself), but pointing back into the worktree.
        let s = stream(&[(&file_origin(".git/config"), "include.path", "../evil.inc")]);
        assert!(validate_config_origins(&s, &root(), &roots()).is_err());
    }

    #[test]
    fn an_include_that_stays_in_the_git_dir_is_allowed() {
        let s = stream(&[(&file_origin(".git/config"), "include.path", "extra.inc")]);
        assert!(validate_config_origins(&s, &root(), &roots()).is_ok());
    }

    #[test]
    fn an_absent_include_outside_shared_roots_is_allowed() {
        let fixture = tempfile::tempdir().unwrap();
        let project = fixture.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let config = format!("file:{}", project.join(".git/config").display());
        let absent = fixture.path().join("optional/missing.inc");
        assert!(!absent.exists());
        let bytes = stream(&[(&config, "include.path", &absent.to_string_lossy())]);
        assert!(validate_config_origins(&bytes, &project, std::slice::from_ref(&project)).is_ok());
    }

    #[test]
    fn an_includeif_target_is_validated_too() {
        let s = stream(&[(
            &file_origin(".git/config"),
            "includeIf.gitdir:/srv/.path",
            "../hooks.inc",
        )]);
        assert!(validate_config_origins(&s, &root(), &roots()).is_err());
    }

    #[test]
    fn a_tilde_user_include_fails_closed() {
        let s = stream(&[(&file_origin(".git/config"), "include.path", "~bob/evil.inc")]);
        assert!(validate_config_origins(&s, &root(), &roots()).is_err());
    }

    #[test]
    fn a_prefix_expansion_include_fails_closed() {
        let s = stream(&[(
            &file_origin(".git/config"),
            "include.path",
            "%(prefix)/etc/x",
        )]);
        assert!(validate_config_origins(&s, &root(), &roots()).is_err());
    }

    #[test]
    fn an_unknown_origin_is_refused() {
        let s = stream(&[("blob:deadbeef", "core.editor", "vi")]);
        assert!(validate_config_origins(&s, &root(), &roots()).is_err());
    }

    #[test]
    fn command_line_origin_is_ignored() {
        let s = stream(&[("command line:", "protocol.ext.allow", "never")]);
        assert!(validate_config_origins(&s, &root(), &roots()).is_ok());
    }

    #[test]
    fn a_truncated_stream_is_refused() {
        let mut s = stream(&[(&file_origin(".git/config"), "core.bare", "false")]);
        s.pop(); // drop the terminating NUL
        assert!(validate_config_origins(&s, &root(), &roots()).is_err());
    }

    #[test]
    fn a_malformed_framing_is_refused() {
        // One lone token: an origin with no key/value pair.
        let s = b"file:/srv/project/.git/config\0".to_vec();
        assert!(validate_config_origins(&s, &root(), &roots()).is_err());
    }

    #[test]
    fn a_non_utf8_stream_is_refused() {
        let mut s = stream(&[(&file_origin(".git/config"), "core.bare", "false")]);
        s.insert(0, 0xff);
        assert!(validate_config_origins(&s, &root(), &roots()).is_err());
    }
}
