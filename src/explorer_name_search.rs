//! Background filename search over the project, independent of expanded tree rows.
use super::{TreeDirEntry, is_path_symlink_or_reparse};
use std::{
    collections::VecDeque,
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

#[derive(Default)]
pub(super) struct Results {
    pub entries: Vec<TreeDirEntry>,
    pub truncated: bool,
}

#[derive(Default)]
pub(super) struct Search {
    query: String,
    cancel: Arc<AtomicBool>,
    rx: Option<mpsc::Receiver<Results>>,
    pub results: Option<Results>,
}

impl Drop for Search {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Search {
    pub fn loading(&self) -> bool {
        self.rx.is_some()
    }

    pub fn update(&mut self, root: &Path, query: &str, dots: bool) {
        if self.query != query {
            *self = Self::default();
            let (tx, rx) = mpsc::channel();
            let cancel = self.cancel.clone();
            let project = root.to_owned();
            self.query = query.to_owned();
            let needle = query.to_lowercase();
            thread::spawn(move || {
                let _ = tx.send(scan(&project, &needle, dots, &cancel));
            });
            self.rx = Some(rx);
            return;
        }
        if let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok(result) => {
                    self.results = Some(result);
                    self.rx = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.results = Some(Results {
                        truncated: true,
                        ..Default::default()
                    });
                    self.rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
    }
}

// Vue uses numeric collation with base sensitivity for both tree and search rows.
pub(super) fn compare_names(a: &str, b: &str) -> std::cmp::Ordering {
    static ORDER: std::sync::LazyLock<icu_collator::CollatorBorrowed<'static>> =
        std::sync::LazyLock::new(|| {
            let mut options = icu_collator::options::CollatorOptions::default();
            options.strength = Some(icu_collator::options::Strength::Primary);
            let mut prefs = icu_collator::CollatorPreferences::default();
            prefs.numeric_ordering =
                Some(icu_collator::preferences::CollationNumericOrdering::True);
            icu_collator::Collator::try_new(prefs, options).expect("compiled explorer collation")
        });
    ORDER.compare(a, b)
}

fn scan(root: &Path, needle: &str, dots: bool, cancel: &AtomicBool) -> Results {
    let mut results = Results::default();
    let Ok(project) = root.canonicalize() else {
        return results;
    };
    let mut dirs = VecDeque::from([root.to_owned()]);
    let mut seen = 0usize;
    while let Some(dir) = dirs.pop_front() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if is_path_symlink_or_reparse(&dir)
            || !dir.canonicalize().is_ok_and(|p| p.starts_with(&project))
        {
            continue;
        }
        let Ok(reader) = fs::read_dir(dir) else {
            continue;
        };
        let mut entries = Vec::new();
        for entry in reader.flatten() {
            if cancel.load(Ordering::Relaxed) {
                return results;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if matches!(
                name.as_str(),
                ".git"
                    | "node_modules"
                    | "dist"
                    | "build"
                    | "out"
                    | ".next"
                    | ".cache"
                    | "target"
                    | ".venv"
                    | "__pycache__"
            ) || (!dots && name.starts_with('.'))
            {
                continue;
            }
            if seen == 100_000 {
                results.truncated = true;
                break;
            }
            seen += 1;
            entries.push((name, entry));
        }
        entries.sort_by(|(a, _), (b, _)| compare_names(a, b));
        for (name, entry) in entries {
            if cancel.load(Ordering::Relaxed) {
                return results;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if (!kind.is_file() && !kind.is_dir()) || is_path_symlink_or_reparse(&path) {
                continue;
            }
            if kind.is_dir() {
                dirs.push_back(path.clone());
            }
            if name.to_lowercase().contains(needle) {
                if results.entries.len() == 500 {
                    results.truncated = true;
                    return results;
                }
                results.entries.push(TreeDirEntry {
                    path,
                    name,
                    directory: kind.is_dir(),
                });
            }
        }
        if results.truncated {
            break;
        }
    }
    results
}
