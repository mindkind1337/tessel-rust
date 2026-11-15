//! Per-share sticky privacy: once Git hides a path it stays hidden for this share.
//! No negative cache: edits to ignore rules take effect on the next request.
use super::*;
use std::collections::{HashMap, HashSet};
#[derive(Default)]
pub(super) struct IgnoredPaths {
    roots: Mutex<HashMap<PathBuf, HashSet<String>>>,
}
impl IgnoredPaths {
    pub(super) fn hidden(
        &self,
        root: &Path,
        roots: &[PathBuf],
        paths: &[String],
    ) -> Result<Vec<bool>> {
        self.hidden_with_mode(root, roots, paths, false)
    }
    /// The commit guard also checks tracked ignored files, without changing history.
    pub(super) fn hidden_for_git(
        &self,
        root: &Path,
        roots: &[PathBuf],
        paths: &[String],
    ) -> Result<Vec<bool>> {
        self.hidden_with_mode(root, roots, paths, true)
    }
    fn hidden_with_mode(
        &self,
        root: &Path,
        roots: &[PathBuf],
        paths: &[String],
        no_index: bool,
    ) -> Result<Vec<bool>> {
        if paths.len() > 10_000 || paths.iter().map(String::len).sum::<usize>() > 1024 * 1024 {
            return Err("Too many paths to verify safely.".into());
        }
        for path in paths {
            if path.is_empty()
                || path.contains('\0')
                || Path::new(path)
                    .components()
                    .any(|c| !matches!(c, std::path::Component::Normal(_)))
            {
                return Err("Invalid ignore-check path.".into());
            }
        }
        let root = dunce::canonicalize(root).map_err(|_| "Shared root unavailable")?;
        let mut state = self
            .roots
            .try_lock()
            .map_err(|_| "Guest privacy checks are busy. Try again later.")?;
        if !state.contains_key(&root) && state.len() >= 64 {
            return Err("Too many shared roots to verify safely.".into());
        }
        let seen = state.entry(root.clone()).or_default();
        let mut current_hits = HashSet::new();
        let mut git = super::guest_git_host::HostGit::new(&root);
        if matches!(git.repository()?, crate::git::GuestRepository::Worktree(_))
            && !paths.is_empty()
        {
            let (_owner, hooks) = super::guest_git_dispatch::private_hooks()?;
            let prefix = super::guest_git_host::prepare_prefix(&mut git, &root, roots, &hooks)?;
            let mut input = Vec::new();
            for path in paths {
                input.extend_from_slice(path.as_bytes());
                input.push(0);
            }
            let output = git
                .ignored(&prefix, &input, no_index)
                .map_err(|_| "Cannot verify ignored files safely.")?;
            if output.truncated || (!output.bytes.is_empty() && output.bytes.last() != Some(&0)) {
                return Err("Incomplete ignore-check response.".into());
            }
            let text =
                std::str::from_utf8(&output.bytes).map_err(|_| "Invalid ignore-check response")?;
            let requested: HashSet<&str> = paths.iter().map(String::as_str).collect();
            let hits: Vec<&str> = text.split('\0').filter(|s| !s.is_empty()).collect();
            if hits.iter().any(|p| !requested.contains(p)) {
                return Err("Unexpected ignore-check path.".into());
            }
            if no_index {
                current_hits.extend(hits.into_iter().map(normalize));
            } else {
                if seen.len().saturating_add(hits.len()) > 200_000 {
                    return Err("Shared privacy history is full.".into());
                }
                seen.extend(hits.into_iter().map(normalize));
            }
        }
        Ok(paths
            .iter()
            .map(|p| sticky_contains(seen, p) || current_hits.contains(&normalize(p)))
            .collect())
    }
}
fn normalize(path: &str) -> String {
    path.replace('\\', "/").to_lowercase()
}
fn sticky_contains(seen: &HashSet<String>, path: &str) -> bool {
    let normalized = normalize(path);
    let mut part = normalized.as_str();
    loop {
        if seen.contains(part) {
            return true;
        }
        match part.rsplit_once('/') {
            Some((parent, _)) => part = parent,
            None => return false,
        }
    }
}
