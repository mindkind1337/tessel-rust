//! Hardened git for a Live Share guest. Every git process a guest's request starts
//! runs with the repository's hooks, filters, fsmonitor, signing, credential helpers
//! and ext transport OFF, from every config scope (the repository's, what it includes,
//! the user's). A guest who may edit files could otherwise write a hook manager's config
//! or a filter script into the shared tree and make the host's git run it on the next
//! commit, stage or even status: that is the host's own code execution, which a guest
//! without the Type right must never reach.
//!
//! The hardening is entirely in the command line: a `-c` given to git overrides every
//! config file, includes among them, and `core.hooksPath` to an empty directory of
//! Tessel's turns off both a set hooksPath and `$GIT_DIR/hooks`. Paths are literal
//! (`--literal-pathspecs`, so a guest's `:/` or `*` is a name, never repository magic)
//! and confined to the shared project. Status and diff never show a private or
//! out-of-project file's name or contents. A commit is refused while anything outside
//! the project is staged. Files git reads through a filter (git-lfs, git-crypt) are not
//! staged or restored for a guest (staged without the filter, a secret would commit in
//! clear). Errors carry only git's own words, host paths hidden and secrets masked.
//!
//! The authority adapter calls this outside its mutex and revalidates before mutations.
//! Mutable configuration origins/includes are refused across all shared roots. These
//! boundary checks do not freeze attributes or the index against concurrent changes;
//! snapshot/index transactions remain a separate follow-up. A mutation already launched
//! may finish after revocation; an uncertain result is never retried automatically.

use super::{
    guest_text, private_path, secret_name_path, share_new_path_checked, share_path_checked,
};
use crate::git::Capture;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, String>;

/// Runs one `git` invocation with the project as its working directory. The production
/// impl adds the process-group cleanup and 20 s bound of `crate::git::run_bytes`; a test
/// impl answers canned bytes. `args` already carries the full hardening prefix.
pub(super) trait GitRun {
    fn run(&mut self, args: &[&str], input: Option<&[u8]>) -> Result<Capture>;
}

/// The real runner clears inherited GIT_* overrides and shares one 20-second deadline
/// across preparation and execution. It uses the common bounded process-group runner.
pub(super) struct HostGit<'a> {
    pub root: &'a Path,
    deadline: Instant,
}
impl<'a> HostGit<'a> {
    pub(super) fn new(root: &'a Path) -> Self {
        Self {
            root,
            deadline: Instant::now() + Duration::from_secs(10),
        }
    }
    pub(super) fn repository(&mut self) -> Result<crate::git::GuestRepository> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("Repository scope check timed out.".into());
        }
        crate::git::run_guest_repository(self.root, remaining)
    }
    pub(super) fn ignored(
        &mut self,
        prefix: &[String],
        input: &[u8],
        no_index: bool,
    ) -> Result<Capture> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("Guest ignore check timed out.".into());
        }
        crate::git::run_guest_check_ignore(self.root, &borrow(prefix), input, no_index, remaining)
    }
}
impl GitRun for HostGit<'_> {
    fn run(&mut self, args: &[&str], input: Option<&[u8]>) -> Result<Capture> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("Guest Git request timed out.".into());
        }
        crate::git::run_guest_bytes(self.root, args, input, remaining)
    }
}

/// Config keys that run a program (lower-cased), matched against every scope.
const RISKY_PATTERN: &str = r"(?i)^(core\.fsmonitor|core\.hookspath|core\.sshcommand|protocol\.ext\.allow|log\.showsignature|gpg\.(.+\.)?program|remote\..+\.(url|pushurl|uploadpack|receivepack)|filter\..+\.(clean|smudge|process)|diff\..+\.(textconv|command)|merge\..+\.driver)$";

fn risky_re() -> regex::Regex {
    regex::Regex::new(RISKY_PATTERN).expect("static risky pattern compiles")
}

/// The keys of program-running settings in a `config --includes -z --get-regexp`
/// output (records are "key\nvalue\0"). `core.fsmonitor=false`/`log.showSignature=false`
/// and a remote URL without `ext::` run nothing, so they are not reported.
pub(super) fn parse_risky(output: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(output);
    let re = risky_re();
    let mut keys = Vec::new();
    for rec in text.split('\0') {
        if rec.is_empty() {
            continue;
        }
        let (key, value) = match rec.split_once('\n') {
            Some((k, v)) => (k.trim(), v),
            None => (rec.trim(), ""),
        };
        let lower = key.to_lowercase();
        if !re.is_match(&lower) {
            continue;
        }
        let v = value.trim();
        if matches!(lower.as_str(), "core.fsmonitor" | "log.showsignature")
            && matches!(v.to_lowercase().as_str(), "false" | "no" | "off" | "0" | "")
        {
            continue;
        }
        if lower == "protocol.ext.allow" && v.eq_ignore_ascii_case("never") {
            continue;
        }
        if (lower.ends_with(".url") || lower.ends_with(".pushurl"))
            && lower.starts_with("remote.")
            && !v.trim_start().to_lowercase().starts_with("ext::")
        {
            continue;
        }
        if v.is_empty() && !lower.contains("filter.") && !lower.contains("diff.") {
            // An empty program is already off; a filter/diff cleared to empty still
            // needs its `.required`/sibling turned off, so keep those.
            continue;
        }
        keys.push(normalize_key(key));
    }
    keys
}

// Git sections and variable names ignore case; subsection names do not.
fn normalize_key(key: &str) -> String {
    match (key.find('.'), key.rfind('.')) {
        (Some(first), Some(last)) if first != last => format!(
            "{}.{}.{}",
            key[..first].to_ascii_lowercase(),
            &key[first + 1..last],
            key[last + 1..].to_ascii_lowercase()
        ),
        _ => key.to_ascii_lowercase(),
    }
}

/// The `-c` overrides every guest git call gets: the fixed set that turns hooks, signing,
/// transports and helpers off, plus one neutralizer per program-running key found.
pub(super) fn hardening_prefix(risky: &[String], hooks_dir: &Path) -> Vec<String> {
    let hooks = hooks_dir.to_string_lossy().replace('\\', "/");
    let mut out: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut add = |out: &mut Vec<String>, key: &str, value: &str| {
        if seen.insert(normalize_key(key)) {
            out.push("-c".into());
            out.push(format!("{key}={value}"));
        }
    };
    // Always off, whatever the config says.
    add(&mut out, "core.hooksPath", &hooks);
    add(&mut out, "protocol.ext.allow", "never");
    add(&mut out, "core.fsmonitor", "false");
    add(&mut out, "log.showSignature", "false");
    add(&mut out, "gpg.program", "gpg");
    add(&mut out, "gpg.ssh.program", "ssh-keygen");
    add(&mut out, "gpg.x509.program", "gpgsm");
    add(&mut out, "gpg.openpgp.program", "gpg");
    add(&mut out, "commit.gpgSign", "false");
    add(&mut out, "tag.gpgSign", "false");
    add(&mut out, "diff.external", "");
    add(&mut out, "gc.auto", "0");
    add(&mut out, "maintenance.auto", "false");
    // A path-limited checkout/reset must not mutate nested repositories.
    add(&mut out, "submodule.recurse", "false");
    add(&mut out, "core.alternateRefsCommand", "");
    add(&mut out, "core.sshCommand", "ssh");
    add(&mut out, "credential.helper", "");
    for key in risky {
        if let Some(name) = key
            .strip_prefix("filter.")
            .and_then(|r| r.rsplit_once('.').map(|(n, _)| n))
        {
            add(&mut out, &format!("filter.{name}.clean"), "");
            add(&mut out, &format!("filter.{name}.smudge"), "");
            add(&mut out, &format!("filter.{name}.process"), "");
            add(&mut out, &format!("filter.{name}.required"), "false");
        } else if let Some(name) = key
            .strip_prefix("diff.")
            .and_then(|r| r.rsplit_once('.').map(|(n, _)| n))
        {
            add(&mut out, &format!("diff.{name}.textconv"), "");
            add(&mut out, &format!("diff.{name}.command"), "");
        } else if let Some(name) = key
            .strip_prefix("merge.")
            .and_then(|r| r.strip_suffix(".driver"))
        {
            add(&mut out, &format!("merge.{name}.driver"), "");
        } else if let Some(name) = key
            .strip_prefix("remote.")
            .and_then(|r| r.strip_suffix(".uploadpack"))
        {
            add(
                &mut out,
                &format!("remote.{name}.uploadpack"),
                "git-upload-pack",
            );
        } else if let Some(name) = key
            .strip_prefix("remote.")
            .and_then(|r| r.strip_suffix(".receivepack"))
        {
            add(
                &mut out,
                &format!("remote.{name}.receivepack"),
                "git-receive-pack",
            );
        } else if key.starts_with("gpg.") && key.ends_with(".program") {
            // An unknown signer back to empty (the known four are already defaulted).
            add(&mut out, key, "");
        }
    }
    out.push("--literal-pathspecs".into());
    out.push("--no-optional-locks".into());
    out
}

fn borrow(args: &[String]) -> Vec<&str> {
    args.iter().map(String::as_str).collect()
}

/// `check-attr filter -- <rel>` (`-z`: "path\0filter\0value\0"): a set filter (git-lfs,
/// git-crypt) means staging or restoring this file for a guest would leak or corrupt it.
fn has_filter_attr(git: &mut dyn GitRun, prefix: &[String], rel: &str) -> Result<bool> {
    let mut args = prefix.to_vec();
    args.extend(["check-attr", "-z", "filter", "--", rel].map(String::from));
    let out = git.run(&borrow(&args), None)?;
    if out.truncated {
        return Err("Incomplete Git attributes.".into());
    }
    let text = std::str::from_utf8(&out.bytes).map_err(|_| "Unsupported Git attributes")?;
    let fields: Vec<&str> = text.split('\0').collect();
    if fields.len() != 4 || fields[0] != rel || fields[1] != "filter" || !fields[3].is_empty() {
        return Err("Invalid Git attributes.".into());
    }
    Ok(!matches!(fields[2], "unspecified" | "unset" | ""))
}

fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.contains([':', '\\'])
        && !Path::new(path).is_absolute()
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}
/// Keeps the paths a guest may see: inside the project and not private, and when the
/// share hides ignored/secret files, not secret-looking. Returns (visible, hidden_count);
/// a hidden file is never named.
pub(super) fn filter_visible(paths: &[String], hide_ignored: bool) -> (Vec<String>, usize) {
    let mut visible = Vec::new();
    let mut hidden = 0usize;
    for p in paths {
        let rel = Path::new(p);
        if !safe_relative(p) || private_path(rel) || (hide_ignored && secret_name_path(rel)) {
            hidden += 1;
        } else {
            visible.push(p.clone());
        }
    }
    (visible, hidden)
}

/// Porcelain v1 `-z`: records split on NUL; a rename/copy (`R`/`C`) consumes the next
/// field as its origin. Paths are repository-relative; `show_prefix` (the project's path
/// under the repository top, with a trailing slash, empty when the project is the top) is
/// stripped so a path outside the project, or private, hides the whole entry.
pub(super) fn parse_status(
    output: &[u8],
    show_prefix: &str,
    hide_ignored: bool,
) -> (Vec<Value>, usize) {
    let complete = output
        .iter()
        .rposition(|&b| b == 0)
        .map_or(&[][..], |end| &output[..=end]);
    let Ok(text) = std::str::from_utf8(complete) else {
        return (Vec::new(), 1);
    };
    let mut fields = text.split('\0').filter(|s| !s.is_empty());
    let mut rows = Vec::new();
    let mut hidden = 0usize;
    let within = |p: &str| -> Option<String> {
        let rel = if show_prefix.is_empty() {
            p
        } else {
            p.strip_prefix(show_prefix)?
        };
        safe_relative(rel).then(|| rel.to_string())
    };
    let bad = |p: &str| {
        let rel = Path::new(p);
        private_path(rel) || (hide_ignored && secret_name_path(rel))
    };
    while let Some(entry) = fields.next() {
        let bytes = entry.as_bytes();
        if bytes.len() < 3
            || bytes[2] != b' '
            || !bytes[..2].iter().all(|b| b" MADRCU?!".contains(b))
        {
            hidden += 1;
            continue;
        }
        let code = &entry[..2];
        let raw_path = &entry[3..];
        let raw_origin = if code.as_bytes().iter().any(|b| matches!(*b, b'R' | b'C')) {
            fields.next()
        } else {
            None
        };
        // Outside the project, or private: counted as hidden, never named.
        let (Some(path), origin) = (within(raw_path), raw_origin.map(within)) else {
            hidden += 1;
            continue;
        };
        if matches!(&origin, Some(None)) || bad(&path) || matches!(&origin, Some(Some(o)) if bad(o))
        {
            hidden += 1;
            continue;
        }
        rows.push(json!({"status": code, "path": path, "from": origin.flatten()}));
    }
    (rows, hidden)
}

/// Control characters (a path may hold any byte but NUL) to `?`, so a reconstructed status
/// or error line cannot smuggle terminal markup into a guest's view.
fn escape_controls(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .collect()
}

/// `rev-parse --show-prefix`: the project folder relative to the repository top (empty
/// when the project is the top), always with a trailing slash when non-empty.
fn project_prefix(git: &mut dyn GitRun, prefix: &[String]) -> Result<String> {
    let mut args = prefix.to_vec();
    args.extend(["rev-parse", "--show-prefix"].map(String::from));
    let out = git.run(&borrow(&args), None)?;
    if out.truncated {
        return Err("Incomplete Git project scope.".into());
    }
    let p = std::str::from_utf8(&out.bytes)
        .map_err(|_| "Unsupported Git project path")?
        .trim()
        .replace('\\', "/");
    Ok(p)
}

/// A commit is refused while anything staged is outside the project, or is private: the
/// host stages its own work elsewhere in a shared repository, and a guest's commit must
/// not carry it.
fn staged_within_project(
    git: &mut dyn GitRun,
    prefix: &[String],
    show_prefix: &str,
    hide_ignored: bool,
    sticky: &dyn Fn(&[String]) -> Result<Vec<bool>>,
) -> Result<bool> {
    let mut args = prefix.to_vec();
    args.extend(["diff", "--cached", "--no-renames", "--name-only", "-z"].map(String::from));
    let out = git.run(&borrow(&args), None)?;
    if out.truncated || (!out.bytes.is_empty() && !out.bytes.ends_with(&[0])) {
        return Err("Incomplete Git index scope.".into());
    }
    let text = std::str::from_utf8(&out.bytes).map_err(|_| "Unsupported Git index path")?;
    let mut paths = Vec::new();
    for path in text.split('\0').filter(|s| !s.is_empty()) {
        let Some(rel) = path.strip_prefix(show_prefix) else {
            return Ok(false); // outside the project
        };
        if rel.is_empty() || rel == ".." || rel.starts_with("../") || rel.contains("/../") {
            return Ok(false);
        }
        paths.push(rel.to_owned());
        let r = Path::new(rel);
        if private_path(r) || (hide_ignored && secret_name_path(r)) {
            return Ok(false);
        }
    }
    Ok(!sticky(&paths)?.into_iter().any(|hide| hide))
}

fn bounded_err(root: &Path, message: &str) -> String {
    guest_text::task_title(message, &root.to_string_lossy(), 300)
}

/// The guest git surface. `root` is the shared project folder; `git_right` gates the
/// mutations; `hooks_dir` is an existing empty directory; `revalidate` is called right
/// before each change and must re-check the guest's session, rights, root and masking.
#[allow(clippy::too_many_arguments)]
pub(super) fn dispatch(
    root: &Path,
    shared_roots: &[PathBuf],
    history: &super::ignored_paths::IgnoredPaths,
    git_right: bool,
    hide_ignored: bool,
    op: &str,
    req: &Value,
    hooks_dir: &Path,
    revalidate: &mut dyn FnMut() -> Result<()>,
) -> Result<Value> {
    let mut git = HostGit {
        root,
        deadline: Instant::now() + Duration::from_secs(20),
    };
    dispatch_with_roots(
        &mut git,
        root,
        shared_roots,
        Some(history),
        git_right,
        hide_ignored,
        op,
        req,
        hooks_dir,
        revalidate,
    )
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(super) fn dispatch_with(
    git: &mut dyn GitRun,
    root: &Path,
    git_right: bool,
    hide_ignored: bool,
    op: &str,
    req: &Value,
    hooks_dir: &Path,
    revalidate: &mut dyn FnMut() -> Result<()>,
) -> Result<Value> {
    dispatch_with_roots(
        git,
        root,
        &[root.to_path_buf()],
        None,
        git_right,
        hide_ignored,
        op,
        req,
        hooks_dir,
        revalidate,
    )
}
pub(super) fn prepare_prefix(
    git: &mut dyn GitRun,
    root: &Path,
    shared_roots: &[PathBuf],
    hooks_dir: &Path,
) -> Result<Vec<String>> {
    // Read the program-running settings once, with hooks already off for this read too.
    let base = hardening_prefix(&[], hooks_dir);
    let mut cfg = base.clone();
    // The whole merged config (every scope and include), parsed and matched in Rust:
    // git's own `--get-regexp` lower-cases only section and variable, not the subsection,
    // so a `filter.LFS.clean` could slip a case-sensitive git pattern. Rust's `(?i)` does
    // not. The output is bounded by the runner's 2 MiB cap.
    cfg.extend(["config", "--includes", "-z", "--list"].map(String::from));
    // FAIL CLOSED. The fixed prefix neutralizes hooks, signing, transport and helpers
    // unconditionally, but a NAMED filter or diff driver is only turned off once its name
    // is known from this read. `--list` has no "exit 1 = nothing set" meaning, and a
    // truncated read may have cut a risky key, so any failure or truncation refuses the
    // request rather than running git without the full set of neutralizers.
    let risky = match git.run(&borrow(&cfg), None) {
        Ok(out) if !out.truncated && std::str::from_utf8(&out.bytes).is_ok() => {
            parse_risky(&out.bytes)
        }
        _ => {
            return Err(bounded_err(
                root,
                "Git configuration could not be read safely; try again.",
            ));
        }
    };
    let mut origins = base.clone();
    origins.extend(["config", "--includes", "-z", "--show-origin", "--list"].map(String::from));
    let origins = git
        .run(&borrow(&origins), None)
        .map_err(|_| "Git configuration origins could not be read safely.")?;
    if origins.truncated {
        return Err("Git configuration origins are incomplete.".into());
    }
    super::guest_git_config::validate_config_origins(&origins.bytes, root, shared_roots)?;
    Ok(hardening_prefix(&risky, hooks_dir))
}

#[allow(clippy::too_many_arguments)]
fn dispatch_with_roots(
    git: &mut dyn GitRun,
    root: &Path,
    shared_roots: &[PathBuf],
    history: Option<&super::ignored_paths::IgnoredPaths>,
    git_right: bool,
    hide_ignored: bool,
    op: &str,
    req: &Value,
    hooks_dir: &Path,
    revalidate: &mut dyn FnMut() -> Result<()>,
) -> Result<Value> {
    let metadata = |paths: &[String], mut hidden: Vec<bool>| -> Result<Vec<bool>> {
        let mut scan = super::git_metadata::Scan::new(root)?;
        for (path, hidden) in paths.iter().zip(&mut hidden) {
            *hidden |= scan.hidden_existing_ancestors(&root.join(path))?;
        }
        Ok(hidden)
    };
    let sticky = |paths: &[String]| -> Result<Vec<bool>> {
        if hide_ignored && let Some(history) = history {
            return metadata(paths, history.hidden(root, shared_roots, paths)?);
        }
        metadata(paths, vec![false; paths.len()])
    };
    let commit_hidden = |paths: &[String]| -> Result<Vec<bool>> {
        if hide_ignored && let Some(history) = history {
            return metadata(paths, history.hidden_for_git(root, shared_roots, paths)?);
        }
        metadata(paths, vec![false; paths.len()])
    };
    let prefix = prepare_prefix(git, root, shared_roots, hooks_dir)?;

    let run = |git: &mut dyn GitRun, tail: &[&str]| -> Result<Capture> {
        let mut args = prefix.clone();
        args.extend(tail.iter().map(|s| s.to_string()));
        git.run(&borrow(&args), None)
    };

    match op {
        "scm.status" => {
            // Porcelain paths are repository-relative; strip the project's own prefix so a
            // project under a larger repository reports its own paths.
            let show = project_prefix(git, &prefix).map_err(|e| bounded_err(root, &e))?;
            let out = run(
                git,
                &[
                    "status",
                    "-z",
                    "--porcelain=v1",
                    "--untracked-files=all",
                    "--",
                    ".",
                ],
            )
            .map_err(|e| bounded_err(root, &e))?;
            let (rows, mut hidden) = parse_status(&out.bytes, &show, hide_ignored);
            let paths: Vec<String> = rows
                .iter()
                .flat_map(|row| {
                    [row["path"].as_str(), row["from"].as_str()]
                        .into_iter()
                        .flatten()
                        .map(str::to_owned)
                })
                .collect();
            let blocked: std::collections::HashSet<String> = paths
                .iter()
                .zip(sticky(&paths)?)
                .filter_map(|(p, hide)| hide.then_some(p.clone()))
                .collect();
            let rows: Vec<Value> = rows
                .into_iter()
                .filter(|row| {
                    let hide = [row["path"].as_str(), row["from"].as_str()]
                        .into_iter()
                        .flatten()
                        .any(|p| blocked.contains(p));
                    hidden += usize::from(hide);
                    !hide
                })
                .collect();
            // A reconstructed porcelain string of the visible rows only, for the protocol's
            // {status} shape; control characters neutralized, capped.
            let mut truncated = out.truncated
                || std::str::from_utf8(&out.bytes).is_err()
                || (!out.bytes.is_empty() && !out.bytes.ends_with(&[0]));
            let mut status = String::new();
            for row in &rows {
                let line = format!(
                    "{} {}\n",
                    row["status"].as_str().unwrap_or(""),
                    escape_controls(row["path"].as_str().unwrap_or(""))
                );
                if status.len() + line.len() > 256 * 1024 {
                    truncated = true;
                    break;
                }
                status.push_str(&line);
            }
            Ok(json!({"status": status, "entries": rows, "hidden": hidden, "truncated": truncated}))
        }
        "scm.diff" => {
            let requested = req.get("path").and_then(Value::as_str).unwrap_or("");
            // `--relative`: names and the diff come out relative to the project folder, so
            // a project under a larger repository never shows paths outside itself.
            let (visible, hidden) = if requested.is_empty() {
                let names = run(
                    git,
                    &[
                        "diff",
                        "--relative",
                        "--no-renames",
                        "--name-only",
                        "--no-ext-diff",
                        "--no-textconv",
                        "-z",
                        "--",
                        ".",
                    ],
                )
                .map_err(|e| bounded_err(root, &e))?;
                if names.truncated || (!names.bytes.is_empty() && !names.bytes.ends_with(&[0])) {
                    return Err("Incomplete Git diff paths.".into());
                }
                let all: Vec<String> = std::str::from_utf8(&names.bytes)
                    .map_err(|_| "Unsupported Git diff path")?
                    .split('\0')
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect();
                filter_visible(&all, hide_ignored)
            } else {
                // Honor the exact file asked for, confined and shared (it may be staged for
                // deletion, so existence is not required).
                let rel = checked_rel(root, req, hide_ignored, true)?;
                (vec![rel], 0)
            };
            let mut hidden = hidden;
            let visible: Vec<String> = visible
                .iter()
                .zip(sticky(&visible)?)
                .filter_map(|(p, hide)| {
                    hidden += usize::from(hide);
                    (!hide).then_some(p.clone())
                })
                .collect();
            if visible.is_empty() {
                return Ok(json!({"diff": "", "hidden": hidden, "truncated": false}));
            }
            let mut tail = vec![
                "diff",
                "--relative",
                "--no-renames",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--",
            ];
            let bounded: Vec<&str> = visible.iter().take(2000).map(String::as_str).collect();
            tail.extend(bounded);
            let out = run(git, &tail).map_err(|e| bounded_err(root, &e))?;
            let mut text = String::from_utf8_lossy(&out.bytes).into_owned();
            let truncated = out.truncated || visible.len() > 2000 || text.len() > 256 * 1024;
            if text.len() > 256 * 1024 {
                let mut end = 256 * 1024;
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                text.truncate(end);
            }
            Ok(json!({"diff": text, "hidden": hidden, "truncated": truncated}))
        }
        "scm.stage" | "scm.unstage" | "scm.discard" | "scm.commit" => {
            if !git_right {
                return Err("Git permission is required to stage, discard or commit.".into());
            }
            match op {
                "scm.stage" => {
                    let rel = checked_rel(root, req, hide_ignored, true)?;
                    if sticky(std::slice::from_ref(&rel))?[0] {
                        return Err("File unavailable".into());
                    }
                    if has_filter_attr(git, &prefix, &rel).map_err(|e| bounded_err(root, &e))? {
                        return Err(
                            "This file is handled by a git filter and cannot be staged by a guest."
                                .into(),
                        );
                    }
                    revalidate()?;
                    run(git, &["add", "--", &rel]).map_err(|e| bounded_err(root, &e))?;
                }
                "scm.unstage" => {
                    let rel = checked_rel(root, req, hide_ignored, true)?;
                    if sticky(std::slice::from_ref(&rel))?[0] {
                        return Err("File unavailable".into());
                    }
                    revalidate()?;
                    run(git, &["reset", "-q", "HEAD", "--", &rel])
                        .map_err(|e| bounded_err(root, &e))?;
                }
                "scm.discard" => {
                    let rel = checked_rel(root, req, hide_ignored, false)?;
                    if sticky(std::slice::from_ref(&rel))?[0] {
                        return Err("File unavailable".into());
                    }
                    if has_filter_attr(git, &prefix, &rel).map_err(|e| bounded_err(root, &e))? {
                        return Err("This file is handled by a git filter and cannot be restored by a guest.".into());
                    }
                    revalidate()?;
                    run(git, &["checkout", "--", &rel]).map_err(|e| bounded_err(root, &e))?;
                }
                "scm.commit" => {
                    let msg = req
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .trim();
                    if msg.is_empty() || msg.len() > 2000 {
                        return Err("Enter a commit message.".into());
                    }
                    let show = project_prefix(git, &prefix).map_err(|e| bounded_err(root, &e))?;
                    if !staged_within_project(git, &prefix, &show, hide_ignored, &commit_hidden)
                        .map_err(|e| bounded_err(root, &e))?
                    {
                        return Err(
                            "Unstage changes outside this project before committing.".into()
                        );
                    }
                    revalidate()?;
                    run(git, &["commit", "--no-verify", "--no-gpg-sign", "-m", msg])
                        .map_err(|e| bounded_err(root, &e))?;
                }
                _ => {}
            }
            Ok(json!({"ok": true}))
        }
        "scm.push" | "scm.pull" | "scm.fetch" => {
            Err("Push, pull and fetch stay exclusively with the host.".into())
        }
        _ => Err("Operation is not permitted.".into()),
    }
}

/// A guest path for a mutation: relative, confined, not private, normalized with `/`.
/// `allow_new` uses the new-file check (parent confined) for stage/unstage.
fn checked_rel(root: &Path, req: &Value, hide_ignored: bool, allow_new: bool) -> Result<String> {
    let raw = req.get("path").and_then(Value::as_str).unwrap_or("");
    if !safe_relative(raw) {
        return Err("This path is not shared.".into());
    }
    #[cfg(windows)]
    if raw.contains(['*', '?', '"', '<', '>', '|']) {
        return Err("This path is not shared.".into());
    }
    let canonical = root
        .canonicalize()
        .map_err(|_| "This path is not shared.")?;
    let root = canonical.as_path();
    let resolved = if allow_new {
        share_new_path_checked(root, raw, hide_ignored)
    } else {
        share_path_checked(root, raw, hide_ignored)
    }
    .map_err(|_| "This path is not shared.".to_string())?;
    let rel = resolved
        .strip_prefix(root)
        .map_err(|_| "This path is not shared.".to_string())?;
    let normalized = rel.to_string_lossy().replace('\\', "/");
    if normalized.is_empty() {
        return Err("This path is not shared.".into());
    }
    Ok(normalized)
}

#[cfg(test)]
#[path = "guest_git_host_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "guest_git_real_tests.rs"]
mod real_tests;
