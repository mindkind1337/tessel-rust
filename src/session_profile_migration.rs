//! Recover legacy profile identities on a worker, only from a unique match.
use crate::{
    agent_preferences::Preferences,
    session_profile::{Binding, Context},
};
use std::{
    collections::BTreeMap,
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

pub(crate) struct Migration {
    rx: mpsc::Receiver<Result<Binding, String>>,
    cancel: Arc<AtomicBool>,
    pub prefs: Preferences,
    pub preferred_shell: Option<String>,
    pub trust_home: Option<PathBuf>,
}
impl Drop for Migration {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
impl Migration {
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        agent: String,
        id: String,
        cwd: PathBuf,
        data: Option<PathBuf>,
        home: PathBuf,
        prefs: Preferences,
        preferred_shell: Option<String>,
        trust_home: Option<PathBuf>,
    ) -> Result<Self, String> {
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(1);
        migration_queue()?
            .try_send(Job {
                agent,
                id,
                cwd,
                data,
                home,
                prefs: prefs.clone(),
                cancel: cancel.clone(),
                result: tx,
            })
            .map_err(|_| "Saved profile recovery queue is full or unavailable; retry this pane.")?;
        Ok(Self {
            rx,
            cancel,
            prefs,
            preferred_shell,
            trust_home,
        })
    }
    pub fn poll(&self) -> Option<Result<Binding, String>> {
        match self.rx.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(
                "Saved profile recovery stopped; no session was started.".into(),
            )),
        }
    }
}

struct Job {
    agent: String,
    id: String,
    cwd: PathBuf,
    data: Option<PathBuf>,
    home: PathBuf,
    prefs: Preferences,
    cancel: Arc<AtomicBool>,
    result: mpsc::SyncSender<Result<Binding, String>>,
}

fn migration_queue() -> Result<&'static mpsc::SyncSender<Job>, String> {
    static QUEUE: OnceLock<Result<mpsc::SyncSender<Job>, String>> = OnceLock::new();
    QUEUE
        .get_or_init(|| {
            let (tx, rx) = mpsc::sync_channel::<Job>(100);
            std::thread::Builder::new()
                .name("legacy-session-profiles".into())
                .spawn(move || {
                    for job in rx {
                        if job.cancel.load(Ordering::Relaxed) {
                            continue;
                        }
                        let result = (|| {
                            let roster = job
                                .data
                                .as_deref()
                                .map(|data| {
                                    crate::settings_extra_model::load_managed_accounts(
                                        data, &job.agent,
                                    )
                                })
                                .unwrap_or_default();
                            let inherited =
                                std::env::var(crate::session_profile::env_key(&job.agent)?).ok();
                            find_unique(
                                &Context {
                                    agent: &job.agent,
                                    prefs: &job.prefs,
                                    cwd: &job.cwd,
                                    home: &job.home,
                                    data_dir: job.data.as_deref(),
                                    roster: &roster,
                                    inherited: inherited.as_deref(),
                                },
                                &job.id,
                                &job.cancel,
                            )
                        })();
                        let _ = job.result.send(result);
                    }
                })
                .map_err(|_| "Cannot start saved profile recovery.".to_owned())?;
            Ok(tx)
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn regular_transcript(dir: &Path, id: &str) -> Result<bool, String> {
    match fs::symlink_metadata(dir) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
        Ok(_) => return Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err("Cannot inspect saved profile folder.".into()),
    }
    match fs::symlink_metadata(dir.join(format!("{id}.jsonl"))) {
        Ok(meta) => Ok(meta.is_file() && !meta.file_type().is_symlink()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err("Cannot inspect saved conversation metadata.".into()),
    }
}

fn contains_claude(
    root: &Path,
    cwd: &Path,
    id: &str,
    cancel: &AtomicBool,
    started: Instant,
    visited: &mut usize,
) -> Result<bool, String> {
    if cancel.load(Ordering::Relaxed) || started.elapsed() >= Duration::from_secs(3) {
        return Err("Profile recovery limit reached; no session was started.".into());
    }
    let projects = root.join("projects");
    let slug: String = cwd
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    if regular_transcript(&projects.join(slug), id)? {
        return Ok(true);
    }
    let entries = match fs::read_dir(&projects) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err("A profile could not be checked; no session was started.".into()),
    };
    for entry in entries {
        *visited += 1;
        if *visited > 10_000
            || cancel.load(Ordering::Relaxed)
            || started.elapsed() >= Duration::from_secs(3)
        {
            return Err("Profile recovery limit reached; no session was started.".into());
        }
        let entry = entry.map_err(|_| "Cannot inspect a saved profile.")?;
        let kind = entry
            .file_type()
            .map_err(|_| "Cannot inspect saved profile metadata.")?;
        if kind.is_dir() && !kind.is_symlink() && regular_transcript(&entry.path(), id)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn find_unique(context: &Context<'_>, id: &str, cancel: &AtomicBool) -> Result<Binding, String> {
    if !crate::claude::valid_uuid(id) || !context.cwd.is_dir() {
        return Err("The saved conversation or folder is invalid.".into());
    }
    if context.roster.accounts.len() > 100 {
        return Err("Too many profiles to safely identify the original conversation.".into());
    }
    let mut bindings = vec![context.capture()?, Binding::Default];
    bindings.extend(
        context
            .roster
            .accounts
            .iter()
            .map(|account| Binding::Managed {
                account_id: account.id.clone(),
            }),
    );
    let mut roots = BTreeMap::new();
    for binding in bindings {
        let root = context.resolve(&binding)?;
        let identity = match fs::canonicalize(&root) {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => root.clone(),
            Err(_) => return Err("A profile could not be resolved safely.".into()),
        };
        roots
            .entry(crate::session_profile::path_key(&identity))
            .or_insert((root, binding));
    }
    let started = Instant::now();
    let mut visited = 0;
    let mut bytes = 0u64;
    let mut found = None;
    for (_, (root, binding)) in roots {
        if contains(
            &root,
            context.agent,
            context.cwd,
            id,
            cancel,
            started,
            &mut visited,
            &mut bytes,
        )? {
            if found.is_some() {
                return Err("This conversation exists in multiple profiles; choose its account in Past conversations.".into());
            }
            found = Some(binding);
        }
    }
    found.ok_or_else(|| "The original conversation was not found in any known profile; no replacement session was started.".into())
}

#[allow(clippy::too_many_arguments)]
fn contains(
    root: &Path,
    agent: &str,
    cwd: &Path,
    id: &str,
    cancel: &AtomicBool,
    started: Instant,
    visited: &mut usize,
    bytes: &mut u64,
) -> Result<bool, String> {
    if agent != "codex" {
        return contains_claude(root, cwd, id, cancel, started, visited);
    }
    let mut pending = vec![
        (root.join("sessions"), 3),
        (root.join("archived_sessions"), 0),
    ];
    while let Some((dir, depth)) = pending.pop() {
        if cancel.load(Ordering::Relaxed) || started.elapsed() >= Duration::from_secs(3) {
            return Err(
                "Profile recovery limit reached; choose the original account explicitly.".into(),
            );
        }
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err("A profile could not be checked; no session was started.".into()),
        };
        for entry in entries {
            *visited += 1;
            if *visited > 10_000
                || cancel.load(Ordering::Relaxed)
                || started.elapsed() >= Duration::from_secs(3)
            {
                return Err(
                    "Profile recovery limit reached; choose the original account explicitly."
                        .into(),
                );
            }
            let entry = entry.map_err(|_| "Cannot inspect a saved profile.")?;
            let kind = entry
                .file_type()
                .map_err(|_| "Cannot inspect saved profile metadata.")?;
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() && depth > 0 {
                pending.push((entry.path(), depth - 1));
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if name.starts_with("rollout-") && name.ends_with(&format!("-{id}.jsonl")) {
                let file = fs::File::open(entry.path())
                    .map_err(|_| "Cannot inspect saved conversation metadata.")?;
                if *bytes >= 8 * 1024 * 1024 {
                    return Err(
                        "Profile recovery byte limit reached; no session was started.".into(),
                    );
                }
                let limit = (64 * 1024 + 1).min(8 * 1024 * 1024 - *bytes);
                let mut bounded = file.take(limit);
                let mut line = Vec::new();
                BufReader::new(&mut bounded)
                    .read_until(b'\n', &mut line)
                    .map_err(|_| "Cannot read saved conversation metadata.")?;
                *bytes += limit - bounded.limit();
                let meta = crate::codex_sessions::parse_metadata(&line)
                    .ok_or("Saved conversation metadata is incomplete; no session was started.")?;
                if meta.id == id
                    && crate::session_profile::path_key(Path::new(&meta.cwd))
                        == crate::session_profile::path_key(cwd)
                {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn claude_lookup_probes_direct_path_then_only_project_folders() {
        let temp = tempfile::tempdir().unwrap();
        let id = "12345678-1234-4234-8234-123456789abc";
        let slug: String = temp
            .path()
            .to_string_lossy()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let direct = temp.path().join("projects").join(slug);
        fs::create_dir_all(&direct).unwrap();
        fs::write(direct.join(format!("{id}.jsonl")), "{}\n").unwrap();
        let mut visited = 0;
        assert!(
            contains_claude(
                temp.path(),
                temp.path(),
                id,
                &AtomicBool::new(false),
                Instant::now(),
                &mut visited
            )
            .unwrap()
        );
        assert_eq!(visited, 0);
        let moved = temp.path().join("projects/old-slug");
        fs::rename(direct, &moved).unwrap();
        for n in 0..20 {
            fs::write(moved.join(format!("ignored-{n}.jsonl")), "{}").unwrap();
        }
        assert!(
            contains_claude(
                temp.path(),
                temp.path(),
                id,
                &AtomicBool::new(false),
                Instant::now(),
                &mut visited
            )
            .unwrap()
        );
        assert_eq!(
            visited, 1,
            "only project folders count; transcripts are never enumerated"
        );
    }

    #[test]
    fn codex_unreadable_candidate_cannot_establish_uniqueness() {
        let temp = tempfile::tempdir().unwrap();
        let id = "12345678-1234-4234-8234-123456789abc";
        let folder = temp.path().join("archived_sessions");
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            folder.join(format!("rollout-synthetic-{id}.jsonl")),
            vec![b' '; 65538],
        )
        .unwrap();
        let mut visited = 0;
        let mut bytes = 0;
        assert!(
            contains(
                temp.path(),
                "codex",
                temp.path(),
                id,
                &AtomicBool::new(false),
                Instant::now(),
                &mut visited,
                &mut bytes
            )
            .is_err()
        );
        assert!(bytes <= 65537);
    }

    #[test]
    fn legacy_profile_recovery_requires_unique_match_and_honors_cancellation() {
        let temp = tempfile::tempdir().unwrap();
        let id = "12345678-1234-1234-1234-123456789abc";
        let roster = crate::settings_extra_model::ManagedAccountsRoster {
            accounts: vec![crate::settings_extra_model::ManagedAccount {
                id: "saved".into(),
                label: "Saved".into(),
                email: String::new(),
                plan: String::new(),
                added_at: 0,
            }],
            selected_id: None,
        };
        let prefs = Preferences::default();
        let context = Context {
            agent: "claude",
            prefs: &prefs,
            cwd: temp.path(),
            home: temp.path(),
            data_dir: Some(temp.path()),
            roster: &roster,
            inherited: None,
        };
        let binding = Binding::Managed {
            account_id: "saved".into(),
        };
        let root = context.resolve(&binding).unwrap();
        fs::create_dir_all(root.join("projects/synthetic")).unwrap();
        fs::write(
            root.join("projects/synthetic").join(format!("{id}.jsonl")),
            "{}\n",
        )
        .unwrap();
        assert_eq!(
            find_unique(&context, id, &AtomicBool::new(false)).unwrap(),
            binding
        );
        assert!(find_unique(&context, id, &AtomicBool::new(true)).is_err());
        let other = temp.path().join(".claude/projects/synthetic");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join(format!("{id}.jsonl")), "{}\n").unwrap();
        assert!(
            find_unique(&context, id, &AtomicBool::new(false))
                .unwrap_err()
                .contains("multiple profiles")
        );
    }
}
