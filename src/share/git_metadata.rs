//! Repository metadata can live under an arbitrary name, not only `.git`.
//! A scan is bounded and caches only within one operation; mutation checks create
//! fresh scans. This is not descriptor-relative confinement against host races.
use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, String>;
const PRIVATE: &str = "Git metadata is not shared.";
pub(super) fn marker(name: &std::ffi::OsStr) -> u8 {
    match name.to_string_lossy().to_ascii_lowercase().as_str() {
        "head" => 1,
        "objects" => 2,
        "refs" => 4,
        "commondir" => 8,
        _ => 0,
    }
}
fn signature(mask: u8) -> bool {
    mask & 1 != 0 && (mask & 6 == 6 || mask & 8 != 0)
}
pub(super) struct Scan {
    root: PathBuf,
    input_root: PathBuf,
    started: Instant,
    entries: usize,
    masks: HashMap<PathBuf, u8>,
}
impl Scan {
    pub(super) fn new(root: &Path) -> Result<Self> {
        Ok(Self {
            root: root.canonicalize().map_err(|_| PRIVATE)?,
            input_root: root.to_path_buf(),
            started: Instant::now(),
            entries: 0,
            masks: HashMap::new(),
        })
    }
    fn budget(&self) -> Result<()> {
        if self.entries > 10_000 || self.started.elapsed() > Duration::from_secs(2) {
            Err("Git metadata scan is incomplete.".into())
        } else {
            Ok(())
        }
    }
    fn mask(&mut self, directory: &Path) -> Result<u8> {
        self.budget()?;
        if let Some(mask) = self.masks.get(directory) {
            return Ok(*mask);
        }
        let mut mask = 0;
        for entry in std::fs::read_dir(directory).map_err(|_| PRIVATE)? {
            self.entries += 1;
            self.budget()?;
            mask |= marker(&entry.map_err(|_| PRIVATE)?.file_name());
            if signature(mask) {
                break;
            }
        }
        self.masks.insert(directory.to_path_buf(), mask);
        Ok(mask)
    }
    fn walk(&mut self, target: &Path, prospective: bool) -> Result<bool> {
        let relative = target.strip_prefix(&self.root).map_err(|_| PRIVATE)?;
        if relative.components().count() > 128
            || relative
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err("Invalid metadata scan path.".into());
        }
        if super::private_path(relative) {
            return Ok(true);
        }
        let mut path = self.root.clone();
        let mut parts = relative.components();
        loop {
            self.budget()?;
            let real = match path.canonicalize() {
                Ok(real) if real.starts_with(&self.root) => real,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound && prospective => break,
                _ => return Err(PRIVATE.into()),
            };
            if super::private_path(real.strip_prefix(&self.root).map_err(|_| PRIVATE)?) {
                return Ok(true);
            }
            if !std::fs::metadata(&real).map_err(|_| PRIVATE)?.is_dir() {
                break;
            }
            let extra = if prospective && target.parent() == Some(path.as_path()) {
                target.file_name().map(marker).unwrap_or(0)
            } else {
                0
            };
            if signature(self.mask(&real)? | extra) {
                return Ok(true);
            }
            match parts.next() {
                Some(part) => path.push(part),
                None => break,
            }
        }
        Ok(false)
    }
    /// Deleted SCM entries still inherit the privacy of their existing ancestors.
    /// This never treats a missing marker as a prospective creation.
    pub(super) fn hidden_existing_ancestors(&mut self, target: &Path) -> Result<bool> {
        let relative = target
            .strip_prefix(&self.input_root)
            .or_else(|_| target.strip_prefix(&self.root))
            .map_err(|_| PRIVATE)?;
        if relative.components().count() > 128
            || relative
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(PRIVATE.into());
        }
        if super::private_path(relative) {
            return Ok(true);
        }
        let mut existing = self.root.join(relative);
        loop {
            self.budget()?;
            match std::fs::symlink_metadata(&existing) {
                Ok(_) => return self.hidden(&existing, false),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    if existing == self.root || !existing.pop() {
                        return Err(PRIVATE.into());
                    }
                }
                Err(_) => return Err(PRIVATE.into()),
            }
        }
    }
    pub(super) fn hidden(&mut self, target: &Path, prospective: bool) -> Result<bool> {
        let relative = target
            .strip_prefix(&self.input_root)
            .or_else(|_| target.strip_prefix(&self.root))
            .map_err(|_| PRIVATE)?;
        if relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(PRIVATE.into());
        }
        let target = self.root.join(relative);
        let target = target.as_path();
        // Resolve before examining any directory entries. A link leaving the shared
        // root is refused, even when a later component could lead back inside.
        let real = match target.canonicalize() {
            Ok(real) => real,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && prospective => {
                let parent = target
                    .parent()
                    .ok_or(PRIVATE)?
                    .canonicalize()
                    .map_err(|_| PRIVATE)?;
                parent.join(target.file_name().ok_or(PRIVATE)?)
            }
            Err(_) => return Err(PRIVATE.into()),
        };
        if !real.starts_with(&self.root) {
            return Err(PRIVATE.into());
        }
        if self.walk(target, prospective)? {
            return Ok(true);
        }
        // An alias can jump directly into hooks/ and skip the bare's own directory.
        if real == target {
            Ok(false)
        } else {
            self.walk(&real, prospective)
        }
    }
}
pub(super) fn ensure(root: &Path, target: &Path, prospective: bool) -> Result<()> {
    if Scan::new(root)?.hidden(target, prospective)? {
        Err(PRIVATE.into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signature_is_case_and_type_independent_but_not_a_name_suffix_rule() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let metadata = root.join("service");
        std::fs::create_dir_all(metadata.join("hEaD")).unwrap();
        std::fs::write(metadata.join("OBJECTS"), "not read").unwrap();
        std::fs::write(metadata.join("rEfS"), "not read").unwrap();
        std::fs::write(metadata.join("config"), "core.bare=false").unwrap();
        assert!(ensure(root, &metadata, false).is_err());
        assert!(ensure(root, &metadata.join("config"), false).is_err());
        let ordinary = root.join("app.git");
        std::fs::create_dir(&ordinary).unwrap();
        std::fs::write(ordinary.join("notes"), "ordinary").unwrap();
        assert!(ensure(root, &ordinary.join("notes"), false).is_ok());
        let partial = root.join("partial");
        std::fs::create_dir_all(partial.join("objects")).unwrap();
        std::fs::write(partial.join("HEAD"), "synthetic").unwrap();
        std::fs::write(partial.join("notes"), "ordinary").unwrap();
        assert!(ensure(root, &partial.join("notes"), false).is_ok());
        assert!(ensure(root, &partial.join("REFS"), true).is_err());
        assert!(!partial.join("REFS").exists());
    }
    #[test]
    fn linked_git_metadata_needs_only_head_and_commondir() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let partial = root.join("linked");
        std::fs::create_dir_all(&partial).unwrap();
        std::fs::write(partial.join("HEAD"), "synthetic").unwrap();
        assert!(ensure(root, &partial, false).is_ok());
        assert!(ensure(root, &partial.join("CommonDir"), true).is_err());
        // Content and type never weaken a structural privacy boundary.
        std::fs::create_dir(partial.join("CommonDir")).unwrap();
        std::fs::write(partial.join("config"), "not read").unwrap();
        assert!(ensure(root, &partial.join("config"), false).is_err());
        assert!(
            Scan::new(root)
                .unwrap()
                .hidden_existing_ancestors(&partial.join("deleted"))
                .unwrap()
        );
        let other = root.join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("commondir"), "../not-followed").unwrap();
        assert!(ensure(root, &other, false).is_ok());
        assert!(ensure(root, &other.join("hEaD"), true).is_err());
        for mask in 0..16 {
            assert_eq!(signature(mask), matches!(mask, 7 | 9 | 11 | 13 | 15));
        }
    }

    #[test]
    fn uncertain_or_unbounded_metadata_scans_never_allow_access() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("notes"), "ordinary").unwrap();
        let mut scan = Scan::new(root).unwrap();
        scan.entries = 10_001;
        assert!(scan.hidden(&root.join("notes"), false).is_err());
        let mut scan = Scan::new(root).unwrap();
        scan.started = Instant::now() - Duration::from_secs(3);
        assert!(scan.hidden(&root.join("notes"), false).is_err());
        assert!(ensure(root, &root.join("../outside"), true).is_err());
        assert!(ensure(root, &root.join("missing"), false).is_err());
    }
}
