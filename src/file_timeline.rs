//! Git history for the active local file in the explorer Timeline.
use crate::git;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    thread,
};

#[derive(Clone, Debug)]
pub struct Commit {
    pub id: String,
    stats: std::sync::OnceLock<StatsReceiver>,
    pub author: String,
    pub email: String,
    pub time: i64,
    pub message: String,
    path: String,
    old_path: Option<String>,
}

#[derive(Debug, Default)]
pub struct History {
    github_repo: Arc<std::sync::OnceLock<Option<String>>>,
    scm_unavailable: bool,
    pub saves: Vec<crate::local_file_history::Entry>,
    root: PathBuf,
    relative: String,
    pub commits: Vec<Commit>,
    pub dirty: bool,
    untracked: bool,
    pub more: bool,
}

fn read(file: &Path, limit: usize) -> Result<History, String> {
    let dir = file.parent().ok_or("Choose a file.")?;
    let top = git::run(dir, &["rev-parse", "--show-toplevel"], None)?;
    let root = Path::new(top.trim_end_matches(['\r', '\n']))
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let absolute = dir
        .canonicalize()
        .map_err(|e| e.to_string())?
        .join(file.file_name().ok_or("Choose a file.")?);
    let rel = absolute
        .strip_prefix(&root)
        .map_err(|_| "File outside repository.")?
        .to_str()
        .ok_or("Invalid file name.")?;
    let relative = if cfg!(windows) {
        rel.replace('\\', "/")
    } else {
        rel.to_owned()
    };
    let limit = limit.clamp(1, 500);
    let count = format!("-n{}", limit + 1);
    let log = git::run_bytes(
        &root,
        &[
            "--literal-pathspecs",
            "log",
            "--no-show-signature",
            "--no-color",
            "--follow",
            "--name-status",
            "-z",
            &count,
            "--format=%H%n%aN%n%aE%n%at%n%B",
            "--",
            &relative,
        ],
        None,
    )?;
    if log.truncated {
        return Err("File history exceeds the Git output limit.".into());
    }
    let raw = String::from_utf8_lossy(&log.bytes);
    let mut fields = raw.split_terminator('\0').peekable();
    let mut commits = Vec::new();
    let mut historical = relative.clone();
    while let Some(record) = fields.next() {
        let mut meta = record.trim_start_matches('\n').splitn(5, '\n');
        let id = meta.next().unwrap_or_default();
        if !matches!(id.len(), 40 | 64) || !id.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err("Invalid Git history record.".into());
        }
        let mut commit = Commit {
            id: id.to_owned(),
            stats: Default::default(),
            author: meta.next().unwrap_or_default().into(),
            email: meta.next().unwrap_or_default().into(),
            time: meta.next().and_then(|s| s.parse().ok()).unwrap_or_default(),
            message: meta.next().unwrap_or_default().trim().into(),
            path: historical.clone(),
            old_path: None,
        };
        // Rename metadata is NUL-delimited, including paths with tabs and newlines.
        while fields
            .peek()
            .is_some_and(|s| !s.trim_start_matches('\n').contains('\n'))
        {
            let status = fields.next().unwrap().trim_start_matches('\n');
            let path = fields.next().ok_or("Incomplete Git history path.")?;
            if status.starts_with(['R', 'C']) {
                let to = fields.next().ok_or("Incomplete Git rename.")?;
                if to == historical {
                    commit.old_path = Some(path.into());
                    historical = path.into();
                }
            }
        }
        commits.push(commit);
    }
    let more = commits.len() > limit && limit < 500;
    commits.truncate(limit);
    commits.sort_by_key(|c| std::cmp::Reverse(c.time));
    let status = git::run_bytes(
        &root,
        &[
            "--literal-pathspecs",
            "status",
            "--porcelain",
            "-z",
            "--",
            &relative,
        ],
        None,
    )?;
    Ok(History {
        github_repo: Default::default(),
        scm_unavailable: false,
        saves: Vec::new(),
        root,
        relative,
        commits,
        dirty: !status.bytes.is_empty(),
        untracked: status.bytes.starts_with(b"?? "),
        more,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sources {
    pub git: bool,
    pub local: bool,
}
impl Default for Sources {
    fn default() -> Self {
        Self {
            git: true,
            local: true,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub enum Source {
    Git,
    Local,
}
impl Source {
    pub fn label(self, fr: bool) -> &'static str {
        match (self, fr) {
            (Self::Git, true) => "Historique Git",
            (Self::Git, false) => "Git History",
            (Self::Local, true) => "Historique local",
            (Self::Local, false) => "Local History",
        }
    }
    pub fn enabled(self, sources: Sources) -> bool {
        match self {
            Self::Git => sources.git,
            Self::Local => sources.local,
        }
    }
}
fn load(
    file: &Path,
    limit: usize,
    profile: Option<&Path>,
    sources: Sources,
) -> Result<History, String> {
    let mut history = if sources.git {
        read(file, limit).unwrap_or_else(|_| History {
            scm_unavailable: true,
            ..Default::default()
        })
    } else {
        History::default()
    };
    if sources.local
        && let Some(dir) = profile
    {
        match crate::local_file_history::list(dir, file) {
            Ok(saves) => history.saves = saves,
            Err(error) if !history.dirty && history.commits.is_empty() => {
                return Err(error.to_string());
            }
            Err(_) => {}
        }
    }
    Ok(history)
}

impl History {
    pub fn diff(&self, commit: Option<&str>) -> Result<String, String> {
        if let Some(id) = commit {
            let c = self
                .commits
                .iter()
                .find(|c| c.id == id)
                .ok_or("Choose a file history entry.")?;
            let mut args = vec![
                "--literal-pathspecs",
                "show",
                "--format=",
                "--no-show-signature",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--find-renames",
                &c.id,
                "--",
                &c.path,
            ];
            if let Some(path) = &c.old_path {
                args.push(path);
            }
            git::run(&self.root, &args, None)
        } else if self.untracked {
            git::file_diff(&self.root, &self.relative, None, false, true)
        } else {
            git::run(
                &self.root,
                &[
                    "--literal-pathspecs",
                    "diff",
                    "--no-color",
                    "--no-ext-diff",
                    "--no-textconv",
                    "HEAD",
                    "--",
                    &self.relative,
                ],
                None,
            )
        }
    }
}

#[derive(Default)]
pub struct State {
    pub pin: Option<PinnedFile>,
    sources: Sources,
    pub filters_open: bool,
    pub profile: Option<PathBuf>,
    pub file: Option<PathBuf>,
    pub data: Option<Result<Arc<History>, String>>,
    limit: usize,
    page_size: usize,
    reload_requested: bool,
    revision: u64,
    save_cursor: u64,
    last_age_update: Option<std::time::Instant>,
    pending: Option<(u64, mpsc::Receiver<Result<History, String>>)>,
}
impl State {
    pub fn select(&mut self, file: Option<PathBuf>, sources: Sources, height: u16) {
        self.page_size = (usize::from(height.clamp(44, 2000)) / 22 - 1).max(20);
        if self.file != file || self.sources != sources {
            if self.file != file {
                self.limit = 0;
            }
            self.data = None;
            self.sources = sources;
            self.file = file;
            self.refresh(false);
        }
        self.start();
    }
    pub fn refresh(&mut self, more: bool) {
        if more {
            if self.pending.is_some()
                || !matches!(self.data.as_ref(), Some(Ok(history)) if history.more)
            {
                return;
            }
            self.limit = (self.limit + self.page_size).min(500);
        } else {
            self.limit = self.limit.max(self.page_size);
        }
        self.revision = self.revision.wrapping_add(1);
        self.reload_requested = true;
        self.last_age_update = None;
        self.start();
    }
    fn start(&mut self) {
        if self.data.is_some() && !self.reload_requested {
            return;
        }
        let Some(file) = self.file.clone() else {
            return;
        };
        let sources = self.sources;
        if !sources.git && !sources.local {
            self.reload_requested = false;
            self.data = Some(Ok(Arc::default()));
            return;
        }
        if self.pending.is_some() {
            return;
        }
        self.reload_requested = false;
        let (tx, rx) = mpsc::channel();
        let limit = self.limit;
        let profile = self.profile.clone();
        self.pending = Some((self.revision, rx));
        thread::spawn(move || {
            let _ = tx.send(load(&file, limit, profile.as_deref(), sources));
        });
    }
    pub fn loading(&self) -> bool {
        self.file.is_some() && (self.sources.git || self.sources.local) && self.pending.is_some()
    }
    pub fn poll(&mut self) -> bool {
        if crate::local_file_history::changed_since(&mut self.save_cursor, self.file.as_deref())
            && (self.sources.git || self.sources.local)
        {
            self.refresh(false);
        }
        let Some((revision, rx)) = &self.pending else {
            if self
                .last_age_update
                .is_some_and(|t| t.elapsed().as_secs() >= 30)
            {
                self.last_age_update = Some(std::time::Instant::now());
                return true;
            }
            return false;
        };
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(mpsc::TryRecvError::Disconnected) => Err("File history worker stopped.".into()),
        };
        if *revision == self.revision {
            self.last_age_update = result
                .as_ref()
                .ok()
                .filter(|h| !h.commits.is_empty() || !h.saves.is_empty())
                .map(|_| std::time::Instant::now());
            self.data = Some(result.map(Arc::new));
        }
        self.pending = None;
        self.start();
        true
    }
}

#[derive(Clone, Debug)]
pub struct PinnedFile {
    pub project: Option<u64>,
    pub root: PathBuf,
    pub path: PathBuf,
}

#[derive(Clone, Debug)]
pub enum Action {
    Toggle,
    TogglePin(PathBuf),
    Filters(bool),
    ToggleSource(Source),
    Refresh(PathBuf, bool),
    Open(PathBuf, Option<String>, bool),
    OpenWeb(String),
    OpenMail(String),
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub loading: bool,
    pub shown: bool,
    pub filters_open: bool,
    pub sources: Sources,
    pub pinned: bool,
    pub file: Option<PathBuf>,
    pub data: Option<Result<Arc<History>, String>>,
}
pub fn more_label(loading: bool, fr: bool) -> &'static str {
    match (loading, fr) {
        (true, true) => "Chargement...",
        (true, false) => "Loading...",
        (false, true) => "Charger la suite",
        (false, false) => "Load more",
    }
}
impl Snapshot {
    pub fn file_label(&self, file: &Path, fr: bool) -> String {
        let name = file.file_name().unwrap_or_default().to_string_lossy();
        if self.pinned {
            format!("{name} ({})", if fr { "épinglé" } else { "pinned" })
        } else {
            name.into_owned()
        }
    }
    pub fn pin_label(&self, fr: bool) -> &'static str {
        match (self.pinned, fr) {
            (true, true) => "Désépingler",
            (true, false) => "Unpin",
            (false, true) => "Épingler",
            (false, false) => "Pin",
        }
    }
    pub fn status(&self, fr: bool) -> Option<String> {
        let file = match &self.file {
            Some(file) => file,
            None => {
                return Some(
                    if fr {
                        "L'éditeur actif ne peut pas fournir d'informations de chronologie."
                    } else {
                        "The active editor cannot provide timeline information."
                    }
                    .into(),
                );
            }
        };
        let h = match &self.data {
            Some(Ok(h)) => h,
            Some(Err(e)) => return Some(e.clone()),
            None => {
                let name = file.file_name().unwrap_or_default().to_string_lossy();
                return Some(if fr {
                    format!("Chargement de la chronologie pour {name}...")
                } else {
                    format!("Loading timeline for {name}...")
                });
            }
        };
        if h.dirty || !h.commits.is_empty() || !h.saves.is_empty() {
            return None;
        }
        let mut text = match (self.sources.git,self.sources.local,fr) {
            (false,false,true) => "Toutes les sources de chronologie ont été filtrées.",
            (false,false,false) => "All timeline sources have been filtered out.",
            (_,true,true) => "L'historique local suivra les modifications récentes à mesure que vous les enregistrez, sauf si le fichier a été exclu ou s'il est trop volumineux.",
            (_,true,false) => "Local History will track recent changes as you save them unless the file has been excluded or is too large.",
            (_,false,true) => "Aucune information de chronologie filtrée n'a été fournie.",
            (_,false,false) => "No filtered timeline information was provided.",
        }.to_owned();
        if h.scm_unavailable {
            text.push_str(if fr {
                " Le contrôle de code source n'a pas été configuré."
            } else {
                " Source Control has not been configured."
            });
        }
        Some(text)
    }
}
fn relative_time(time: i64, now: i64, fr: bool) -> String {
    time_words(time, now, fr, false)
}

fn time_words(time: i64, now: i64, fr: bool, ago: bool) -> String {
    let delta = now.saturating_sub(time);
    if (-30..30).contains(&delta) {
        return if fr { "maintenant" } else { "now" }.into();
    }
    let seconds = delta.unsigned_abs();
    let (scale, english, french) = match seconds {
        0..60 => (1, ("sec", "secs"), ("s", "s")),
        60..3600 => (60, ("min", "mins"), ("min", "min")),
        3600..86400 => (3600, ("hr", "hrs"), ("h", "h")),
        86400..604800 => (86400, ("day", "days"), ("jour", "jours")),
        604800..2592000 => (604800, ("wk", "wks"), ("sem.", "sem.")),
        2592000..31536000 => (2592000, ("mo", "mos"), ("mois", "mois")),
        _ => (31536000, ("yr", "yrs"), ("an", "ans")),
    };
    let n = (seconds + scale / 2) / scale;
    if ago {
        let (en, french) = match scale {
            1 => ("second", "seconde"),
            60 => ("minute", "minute"),
            3600 => ("hour", "heure"),
            86400 => ("day", "jour"),
            604800 => ("week", "semaine"),
            2592000 => ("month", "mois"),
            _ => ("year", "an"),
        };
        let word = if fr { french } else { en };
        let plural = if n == 1 || (fr && word == "mois") {
            ""
        } else {
            "s"
        };
        return match (fr, delta < 0) {
            (true, true) => format!("dans {n} {word}{plural}"),
            (true, false) => format!("il y a {n} {word}{plural}"),
            (false, true) => format!("in {n} {word}{plural}"),
            (false, false) => format!("{n} {word}{plural} ago"),
        };
    }
    let units = if fr { french } else { english };
    let prefix = if delta >= 0 {
        ""
    } else if fr {
        "dans "
    } else {
        "in "
    };
    let unit = if n == 1 { units.0 } else { units.1 };
    format!("{prefix}{n} {unit}")
}

#[derive(Clone, Debug)]
pub struct Avatar {
    pub cells: [bool; 25],
    pub rgb: [u8; 3],
}

impl Avatar {
    fn for_author(seed: &str) -> Self {
        // Match the renderer's cyrb128 hash over JavaScript UTF-16 code units.
        let [mut a, mut b, mut c, mut d] = [1779033703u32, 3144134277, 1013904242, 2773480762];
        for unit in seed.trim().to_lowercase().encode_utf16() {
            let k = u32::from(unit);
            a = b ^ (a ^ k).wrapping_mul(597399067);
            b = c ^ (b ^ k).wrapping_mul(2869860233);
            c = d ^ (c ^ k).wrapping_mul(951274213);
            d = a ^ (d ^ k).wrapping_mul(2716044179);
        }
        a = (c ^ (a >> 18)).wrapping_mul(597399067);
        b = (d ^ (b >> 22)).wrapping_mul(2869860233);
        c = (a ^ (c >> 17)).wrapping_mul(951274213);
        d = (b ^ (d >> 19)).wrapping_mul(2716044179);
        a ^= b ^ c ^ d;
        b ^= a;
        let cells = std::array::from_fn(|i| {
            let col = i % 5;
            a & (1 << (col.min(4 - col) * 5 + i / 5)) != 0
        });
        let hue = f64::from(b % 360);
        let saturation = f64::from(45 + ((b >> 9) % 20)) / 100.;
        let light = f64::from(50 + ((b >> 17) % 15)) / 100.;
        let rgb = [0., 8., 4.].map(|n| {
            let k = (n + hue / 30.) % 12.;
            let channel =
                light - saturation * light.min(1. - light) * (k - 3.).min(9. - k).clamp(-1., 1.);
            (channel * 255.).round() as u8
        });
        Self { cells, rgb }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    files: u64,
    insertions: u64,
    deletions: u64,
}
pub type StatsReceiver = tokio::sync::watch::Receiver<Option<Stats>>;

impl Stats {
    fn read(root: &Path, id: &str) -> Option<Self> {
        let summary = git::commit_shortstat(root, id)?;
        let mut stats = Self::default();
        for part in summary.trim().split(',').filter(|s| !s.is_empty()) {
            let (count, kind) = part.trim().split_once(' ')?;
            let count = count.parse().ok()?;
            match kind {
                "file changed" | "files changed" => stats.files = count,
                "insertion(+)" | "insertions(+)" => stats.insertions = count,
                "deletion(-)" | "deletions(-)" => stats.deletions = count,
                _ => return None,
            }
        }
        Some(stats)
    }
    pub fn labels(self, fr: bool) -> (String, Option<String>, Option<String>) {
        let file_word = match (fr, self.files == 1) {
            (true, true) => "fichier modifié",
            (true, false) => "fichiers modifiés",
            (false, true) => "file changed",
            (false, false) => "files changed",
        };
        let lines = |n, word, sign| {
            (n != 0).then(|| format!("{n} {word}{}({sign})", if n == 1 { "" } else { "s" }))
        };
        (
            format!("{} {file_word}", self.files),
            lines(self.insertions, "insertion", '+'),
            lines(
                self.deletions,
                if fr { "suppression" } else { "deletion" },
                '-',
            ),
        )
    }
}
fn github_repository(remote: &str) -> Option<String> {
    let remote = remote.trim();
    let path = if let Some(path) = remote.strip_prefix("git@github.com:") {
        path.to_owned()
    } else {
        let url = url::Url::parse(remote).ok()?;
        if !matches!(url.scheme(), "https" | "http" | "ssh" | "git")
            || !url
                .host_str()
                .is_some_and(|host| host.eq_ignore_ascii_case("github.com"))
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return None;
        }
        url.path().strip_prefix('/')?.to_owned()
    };
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, repo) = path.split_once('/')?;
    if ![owner, repo].iter().all(|p| {
        !p.is_empty()
            && *p != "."
            && *p != ".."
            && p.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    }) {
        return None;
    }
    Some(format!("https://github.com/{owner}/{repo}"))
}

impl Commit {
    fn request_stats(
        &self,
        root: &Path,
        origin: &Arc<std::sync::OnceLock<Option<String>>>,
    ) -> StatsReceiver {
        self.stats
            .get_or_init(|| {
                let (tx, rx) = tokio::sync::watch::channel(None);
                let root = root.to_owned();
                let id = self.id.clone();
                let origin = origin.clone();
                let _ = thread::Builder::new()
                    .name("timeline-commit-stats".into())
                    .spawn(move || {
                        origin.get_or_init(|| {
                            git::run(&root, &["remote", "get-url", "origin"], None)
                                .ok()
                                .and_then(|s| github_repository(&s))
                        });
                        tx.send_replace(Stats::read(&root, &id));
                    });
                rx
            })
            .clone()
    }
}

pub fn copy_hash_label(fr: bool) -> &'static str {
    if fr {
        "Copier le hash du commit"
    } else {
        "Copy Commit Hash"
    }
}

pub struct Details {
    origin: Arc<std::sync::OnceLock<Option<String>>>,
    pub stats: Option<StatsReceiver>,
    pub heading: String,
    pub author: Option<(String, Option<String>)>,
    pub text: String,
    pub hash: Option<String>,
    pub avatar: Option<Avatar>,
}

/// A Timeline message displays each nonempty line as a paragraph or list item.
pub fn message_lines(message: &str) -> impl Iterator<Item = (bool, &str)> {
    message.split(['\r', '\n']).filter_map(|line| {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        if let Some(rest) = line.strip_prefix(['-', '*', '+'])
            && rest.starts_with(char::is_whitespace)
        {
            return Some((true, rest.trim_start()));
        }
        Some((false, line))
    })
}

#[derive(Debug)]
pub struct MessagePart<'a> {
    pub text: &'a str,
    pub code: bool,
    pub action: Option<Action>,
}

pub fn inline_parts<'a>(line: &'a str, repo: Option<&str>) -> Vec<MessagePart<'a>> {
    static INLINE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r##"https?://[^\s<>()]+[^\s<>().,;:!?'"]|(?-u:[\w.+-]+@[\w-]+(?:\.[\w-]+)+)|`[^`]+`|#[0-9]+(?-u:\b)"##).unwrap()
    });
    let plain = |text| MessagePart {
        text,
        code: false,
        action: None,
    };
    let mut parts = Vec::new();
    let mut end = 0;
    for m in INLINE.find_iter(line) {
        if end < m.start() {
            parts.push(plain(&line[end..m.start()]));
        }
        let value = m.as_str();
        let mut part = plain(value);
        if value.starts_with('`') {
            part.code = true;
            part.text = &value[1..value.len() - 1];
        } else if let Some(issue) = value.strip_prefix('#') {
            part.action = repo.map(|r| Action::OpenWeb(format!("{r}/issues/{issue}")));
        } else if value.starts_with("http://") || value.starts_with("https://") {
            if url::Url::parse(value).is_ok_and(|url| url.has_host()) {
                part.action = Some(Action::OpenWeb(value.to_owned()));
            }
        } else {
            part.action = author_mailto(value).map(Action::OpenMail);
        }
        parts.push(part);
        end = m.end();
    }
    if end < line.len() {
        parts.push(plain(&line[end..]));
    }
    parts
}

/// Keep commit metadata inside one recipient, including literal URI punctuation.
fn author_mailto(address: &str) -> Option<String> {
    let (local, domain) = address.rsplit_once('@')?;
    if local.is_empty()
        || domain.is_empty()
        || local.contains('@')
        || address.chars().any(|c| c.is_control() || c.is_whitespace())
        || address.contains([',', ';', '<', '>'])
    {
        return None;
    }
    let encode =
        |value: &str| url::form_urlencoded::byte_serialize(value.as_bytes()).collect::<String>();
    Some(format!("mailto:{}@{}", encode(local), encode(domain)))
}

impl Details {
    pub fn repository_url(&self) -> Option<&str> {
        self.origin.get()?.as_deref()
    }

    pub fn github_url(&self) -> Option<String> {
        Some(format!(
            "{}/commit/{}",
            self.origin.get()?.as_ref()?,
            self.hash.as_ref()?
        ))
    }
}
impl History {
    pub fn details(&self, key: Option<&str>, fr: bool, now: i64) -> Option<Details> {
        let date = |ms| crate::regional_date::long_local(ms, fr).unwrap_or_default();
        if let Some(id) = key.and_then(|s| s.strip_prefix("local:")) {
            let save = self.saves.iter().find(|e| e.id == id)?;
            return Some(Details {
                origin: self.github_repo.clone(),
                stats: None,
                heading: date(save.timestamp),
                author: None,
                text: if fr {
                    "Fichier enregistré"
                } else {
                    "File Saved"
                }
                .into(),
                hash: None,
                avatar: None,
            });
        }
        if let Some(id) = key {
            let commit = self.commits.iter().find(|c| c.id == id)?;
            return Some(Details {
                origin: self.github_repo.clone(),
                stats: Some(commit.request_stats(&self.root, &self.github_repo)),
                heading: format!(
                    ", {} ({})",
                    time_words(commit.time, now, fr, true),
                    date(commit.time.saturating_mul(1000))
                ),
                author: Some((commit.author.clone(), author_mailto(&commit.email))),
                text: commit.message.clone(),
                hash: Some(commit.id.clone()),
                avatar: Some(Avatar::for_author(if commit.email.is_empty() {
                    &commit.author
                } else {
                    &commit.email
                })),
            });
        }
        self.dirty.then(|| Details {
            origin: self.github_repo.clone(),
            stats: None,
            author: None,
            heading: format!(
                "{} · {}",
                if fr { "Vous" } else { "You" },
                date(now.saturating_mul(1000))
            ),
            text: if fr {
                "Modifications non commitées"
            } else {
                "Uncommitted Changes"
            }
            .into(),
            hash: None,
            avatar: None,
        })
    }
}

pub struct Row<'a> {
    pub commit: Option<&'a Commit>,
    pub saved: Option<&'a crate::local_file_history::Entry>,
    pub label: &'a str,
    pub age: Option<String>,
}

impl Row<'_> {
    pub fn key(&self) -> Option<String> {
        self.saved
            .map(|s| format!("local:{}", s.id))
            .or_else(|| self.commit.map(|c| c.id.clone()))
    }
}
impl History {
    pub fn rows(&self, now: i64, fr: bool) -> Vec<Row<'_>> {
        let working = self.dirty.then_some((now.saturating_mul(1000), None, None));
        let mut entries: Vec<_> = working
            .into_iter()
            .chain(
                self.commits
                    .iter()
                    .map(|c| (c.time.saturating_mul(1000), Some(c), None)),
            )
            .chain(self.saves.iter().map(|e| (e.timestamp, None, Some(e))))
            .collect();
        entries.sort_by_key(|(time, _, _)| std::cmp::Reverse(*time));
        let mut previous = String::new();
        entries
            .into_iter()
            .map(|(time, commit, saved)| {
                let age = relative_time(time.div_euclid(1000), now, fr);
                let visible = (age != previous).then(|| age.clone());
                previous = age;
                let label = match commit {
                    Some(c) => c.message.lines().next().unwrap_or("").trim(),
                    None if saved.is_some() => {
                        if fr {
                            "Fichier enregistré"
                        } else {
                            "File Saved"
                        }
                    }
                    None if fr => "Modifications non commitées",
                    None => "Uncommitted Changes",
                };
                Row {
                    saved,
                    commit,
                    label,
                    age: visible,
                }
            })
            .collect()
    }
}
pub fn compat(ui: &mut eframe::egui::Ui, snap: &Snapshot, fr: bool) -> Option<Action> {
    let mut action = None;
    if ui
        .selectable_label(snap.shown, if fr { "Chronologie" } else { "Timeline" })
        .clicked()
    {
        action = Some(Action::Toggle);
    }
    if !snap.shown {
        return action;
    }
    ui.menu_button(
        if fr {
            "Filtrer la chronologie"
        } else {
            "Filter Timeline"
        },
        |ui| {
            for source in [Source::Git, Source::Local] {
                if ui
                    .selectable_label(source.enabled(snap.sources), source.label(fr))
                    .clicked()
                {
                    action = Some(Action::ToggleSource(source));
                    ui.close();
                }
            }
        },
    );
    if let Some(file) = &snap.file {
        ui.horizontal(|ui| {
            ui.label(snap.file_label(file, fr));
            if ui
                .selectable_label(snap.pinned, snap.pin_label(fr))
                .clicked()
            {
                action = Some(Action::TogglePin(file.clone()));
            }
            if ui
                .small_button(if fr { "Actualiser" } else { "Refresh" })
                .clicked()
            {
                action = Some(Action::Refresh(file.clone(), false));
            }
        });
    }
    if let Some(status) = snap.status(fr) {
        ui.label(status);
    }
    if let (Some(file), Some(Ok(history))) = (&snap.file, &snap.data) {
        eframe::egui::ScrollArea::vertical()
            .id_salt("timeline")
            .max_height(160.)
            .show(ui, |ui| {
                for row in history.rows(chrono::Utc::now().timestamp(), fr) {
                    let r = ui
                        .horizontal(|ui| {
                            let mut response = ui.selectable_label(false, row.label);
                            if let Some(c) = row.commit {
                                response |= ui.selectable_label(false, &c.author);
                            }
                            if let Some(age) = &row.age {
                                response |= ui.selectable_label(false, age.as_str());
                            }
                            response
                        })
                        .inner;
                    let r = r.on_hover_ui(|ui| {
                        if let Some(details) = history.details(
                            row.key().as_deref(),
                            fr,
                            chrono::Utc::now().timestamp(),
                        ) {
                            let github = details.github_url();
                            let repo = details.origin.get().and_then(Option::as_deref);
                            ui.horizontal(|ui| {
                                if let Some(icon) = details.avatar {
                                    use eframe::egui::{Color32, Rect, Sense, vec2};
                                    let (rect, _) =
                                        ui.allocate_exact_size(vec2(24., 24.), Sense::hover());
                                    ui.painter().rect_filled(rect, 0., Color32::from_gray(240));
                                    for (i, _) in
                                        icon.cells.iter().enumerate().filter(|(_, on)| **on)
                                    {
                                        let pos = rect.min
                                            + vec2(
                                                2. + (i % 5) as f32 * 4.,
                                                2. + (i / 5) as f32 * 4.,
                                            );
                                        ui.painter().rect_filled(
                                            Rect::from_min_size(pos, vec2(4., 4.)),
                                            0.,
                                            Color32::from_rgb(
                                                icon.rgb[0],
                                                icon.rgb[1],
                                                icon.rgb[2],
                                            ),
                                        );
                                    }
                                }
                                if let Some((author, target)) = details.author {
                                    if let Some(target) = target {
                                        if ui
                                            .link(eframe::egui::RichText::new(author).strong())
                                            .on_hover_text(&target)
                                            .clicked()
                                        {
                                            action = Some(Action::OpenMail(target));
                                        }
                                    } else {
                                        ui.strong(author);
                                    }
                                }
                                ui.label(details.heading);
                            });
                            for (bullet, line) in message_lines(&details.text) {
                                ui.horizontal_wrapped(|ui| {
                                    if bullet {
                                        ui.label("•");
                                    }
                                    ui.spacing_mut().item_spacing.x = 0.;
                                    for part in inline_parts(line, repo) {
                                        if let Some(target) = part.action {
                                            if ui.link(part.text).clicked() {
                                                action = Some(target);
                                            }
                                        } else if part.code {
                                            ui.label(eframe::egui::RichText::new(part.text).code());
                                        } else {
                                            ui.label(part.text);
                                        }
                                    }
                                });
                            }
                            if let Some(rx) = details.stats {
                                if let Some(stats) = *rx.borrow() {
                                    let (files, added, removed) = stats.labels(fr);
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(files);
                                        if let Some(s) = added {
                                            ui.colored_label(
                                                crate::theme_palettes::WARP_OK_GREEN,
                                                s,
                                            );
                                        }
                                        if let Some(s) = removed {
                                            ui.colored_label(ui.visuals().error_fg_color, s);
                                        }
                                    });
                                } else if rx.has_changed().is_ok() {
                                    ui.ctx().request_repaint_after(
                                        std::time::Duration::from_millis(100),
                                    );
                                }
                            }
                            if let Some(hash) = details.hash {
                                ui.horizontal_wrapped(|ui| {
                                    if ui
                                        .link(&hash[..7])
                                        .on_hover_text(if fr {
                                            "Ouvrir le commit"
                                        } else {
                                            "Open Commit"
                                        })
                                        .clicked()
                                    {
                                        action = Some(Action::Open(
                                            file.clone(),
                                            Some(hash.clone()),
                                            false,
                                        ));
                                    }
                                    if ui.small_button(copy_hash_label(fr)).clicked() {
                                        ui.ctx().copy_text(hash);
                                    }
                                    if let Some(url) = github
                                        && ui
                                            .link(if fr {
                                                "Ouvrir sur GitHub"
                                            } else {
                                                "Open on GitHub"
                                            })
                                            .clicked()
                                    {
                                        action = Some(Action::OpenWeb(url));
                                    }
                                });
                            }
                        }
                    });
                    if r.clicked() {
                        action = Some(Action::Open(file.clone(), row.key(), r.double_clicked()));
                    }
                }
                if history.more
                    && ui
                        .add_enabled(
                            !snap.loading,
                            eframe::egui::Button::new(more_label(snap.loading, fr)),
                        )
                        .clicked()
                {
                    action = Some(Action::Refresh(file.clone(), true));
                }
            });
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_timeline_inline_message_routes_links_and_preserves_code_and_punctuation() {
        let parts = inline_parts(
            "é `#1` #12 https://example.org/a. dev+tag@example.org!",
            Some("https://github.com/o/r"),
        );
        assert!(parts[1].code && parts[1].text == "#1" && parts[1].action.is_none());
        assert!(
            matches!(&parts[3].action, Some(Action::OpenWeb(url)) if url == "https://github.com/o/r/issues/12")
        );
        assert!(
            matches!(&parts[5].action, Some(Action::OpenWeb(url)) if url == "https://example.org/a")
        );
        assert_eq!(parts[6].text, ". ");
        assert!(
            matches!(&parts[7].action, Some(Action::OpenMail(url)) if url == "mailto:dev%2Btag@example.org")
        );
        assert_eq!(parts[8].text, "!");
        assert!(inline_parts("#12", None)[0].action.is_none());
    }

    #[test]
    fn file_timeline_author_link_has_one_recipient_and_no_headers() {
        assert_eq!(
            author_mailto("dev+tag@example.org").as_deref(),
            Some("mailto:dev%2Btag@example.org")
        );
        assert_eq!(
            author_mailto("a?b#c@example.org").as_deref(),
            Some("mailto:a%3Fb%23c@example.org")
        );
        for invalid in [
            "",
            "@example.org",
            "name@",
            "a@b,c@d",
            "a@b\r\nCc:someone",
            "one two@host",
        ] {
            assert_eq!(author_mailto(invalid), None);
        }
    }

    #[test]
    fn file_timeline_message_paragraphs_and_bullets() {
        let message = " Title  \r\n\r\n - first\r* second\n+\tthird\n+literal\n- ";
        assert_eq!(
            message_lines(message).collect::<Vec<_>>(),
            [
                (false, "Title"),
                (true, "first"),
                (true, "second"),
                (true, "third"),
                (false, "+literal"),
                (false, "-")
            ]
        );
    }

    #[test]
    fn file_timeline_hover_uses_vue_author_icons_and_full_time_words() {
        for (seed, cells, rgb) in [
            (
                " Fixture@EXAMPLE.invalid ",
                "1000100000101010101001110",
                [216, 184, 110],
            ),
            ("Été😀", "1010100000111110101011111", [70, 139, 200]),
        ] {
            let avatar = Avatar::for_author(seed);
            let mask: String = avatar
                .cells
                .iter()
                .map(|b| if *b { '1' } else { '0' })
                .collect();
            assert_eq!(mask, cells);
            assert_eq!(avatar.rgb, rgb);
        }
        assert_eq!(time_words(0, 300, true, true), "il y a 5 minutes");
        assert_eq!(time_words(300, 0, false, true), "in 5 minutes");
        assert_eq!(time_words(0, 2592000, true, true), "il y a 1 mois");
    }

    #[test]
    fn file_timeline_relative_time_rounds_and_groups_like_vue() {
        for (delta, en, fr) in [
            (-300, "in 5 mins", "dans 5 min"),
            (-30, "now", "maintenant"),
            (29, "now", "maintenant"),
            (30, "30 secs", "30 s"),
            (89, "1 min", "1 min"),
            (90, "2 mins", "2 min"),
            (3599, "60 mins", "60 min"),
            (3600, "1 hr", "1 h"),
            (86400, "1 day", "1 jour"),
            (604800, "1 wk", "1 sem."),
            (2592000, "1 mo", "1 mois"),
            (63072000, "2 yrs", "2 ans"),
        ] {
            assert_eq!(relative_time(0, delta, false), en);
            assert_eq!(relative_time(0, delta, true), fr);
        }
        let commit = |time: i64| Commit {
            stats: Default::default(),
            id: time.to_string(),
            author: String::new(),
            email: String::new(),
            time,
            message: " title\nbody".into(),
            path: String::new(),
            old_path: None,
        };
        let h = History {
            github_repo: Default::default(),
            scm_unavailable: false,
            saves: Vec::new(),
            root: PathBuf::new(),
            relative: String::new(),
            commits: vec![commit(300), commit(0), commit(-2)],
            dirty: true,
            untracked: false,
            more: false,
        };
        let rows = h.rows(0, false);
        assert_eq!(
            rows.iter()
                .map(|r| (r.label, r.age.as_deref()))
                .collect::<Vec<_>>(),
            [
                ("title", Some("in 5 mins")),
                ("Uncommitted Changes", Some("now")),
                ("title", None),
                ("title", None)
            ]
        );
    }

    #[test]
    fn file_timeline_age_tick_repaints_without_reading_git_again() {
        let mut state = State {
            last_age_update: Some(std::time::Instant::now() - std::time::Duration::from_secs(30)),
            ..Default::default()
        };
        assert!(state.poll());
        assert!(state.pending.is_none());
        assert!(!state.poll());
        state.refresh(false);
        assert!(state.last_age_update.is_none());
    }

    fn git_fixture(dir: &Path, args: &[&str]) -> String {
        let mut opts = vec![
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ];
        opts.extend_from_slice(args);
        git::run(dir, &opts, None).unwrap()
    }
    #[test]
    fn file_timeline_pages_keep_entries_and_use_the_current_pane_height() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        git_fixture(root, &["init", "-q", "-b", "main"]);
        let mut stream = String::new();
        for i in 1..=65 {
            let text = i.to_string();
            stream.push_str(&format!("commit refs/heads/main\ncommitter Fixture <fixture@example.invalid> {} +0000\ndata {}\n{text}\nM 100644 inline notes.txt\ndata {}\n{text}\n\n", 1_700_000_000 + i, text.len(), text.len()));
        }
        git::run(root, &["fast-import", "--quiet"], Some(stream.as_bytes())).unwrap();
        git_fixture(root, &["checkout", "-q", "HEAD", "--", "notes.txt"]);
        let path = root.join("notes.txt");
        let sources = Sources {
            git: true,
            local: false,
        };
        let mut state = State::default();
        let drain = |state: &mut State| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(15);
            while state.loading() {
                state.poll();
                assert!(std::time::Instant::now() < until);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            state.data.as_ref().unwrap().as_ref().unwrap().clone()
        };
        state.select(Some(path.clone()), sources, 220);
        let first = drain(&mut state);
        assert_eq!(first.commits.len(), 20);
        state.select(Some(path.clone()), sources, 880);
        assert!(!state.loading());
        state.refresh(true);
        assert!(state.loading());
        assert!(Arc::ptr_eq(
            &first,
            state.data.as_ref().unwrap().as_ref().unwrap()
        ));
        state.refresh(true);
        let second = drain(&mut state);
        assert_eq!(second.commits.len(), 59);
        state.refresh(false);
        assert!(Arc::ptr_eq(
            &second,
            state.data.as_ref().unwrap().as_ref().unwrap()
        ));
        assert_eq!(drain(&mut state).commits.len(), 59);
        state.refresh(true);
        assert_eq!(drain(&mut state).commits.len(), 65);
        state.refresh(true);
        assert!(!state.loading());
        state.select(None, sources, 880);
        assert!(state.data.is_none());
        state.select(Some(path), sources, 880);
        assert_eq!(drain(&mut state).commits.len(), 39);
    }

    #[test]
    fn file_timeline_follows_literal_renames_pages_and_opens_original_patch() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        git_fixture(root, &["init", "-q"]);
        std::fs::create_dir(root.join("docs")).unwrap();
        let original = "docs/[old].txt";
        let current = "docs/[new].txt";
        std::fs::write(root.join(original), "original\n").unwrap();
        git_fixture(root, &["add", "."]);
        git_fixture(
            root,
            &["commit", "-qm", "first", "-m", "Body with\nmultiple lines"],
        );
        git_fixture(root, &["mv", original, current]);
        git_fixture(root, &["commit", "-qm", "rename"]);
        std::fs::write(root.join("docs/n.txt"), "not this file").unwrap();
        git_fixture(root, &["add", "."]);
        git_fixture(root, &["commit", "-qm", "unrelated"]);
        let file = root.join(current);
        let first = read(&file, 1).unwrap();
        assert!(first.more && !first.dirty);
        assert_eq!(first.commits[0].message, "rename");
        std::fs::write(&file, "original\nworking\n").unwrap();
        let h = read(&file, 20).unwrap();
        assert!(h.dirty && !h.more);
        assert_eq!(h.commits.len(), 2);
        let old = &h.commits[1];
        assert_eq!(old.email, "fixture@example.invalid");
        assert!(old.message.contains("multiple lines"));
        assert!(h.diff(Some(&old.id)).unwrap().contains("+original"));
        assert!(
            h.diff(Some(&h.commits[0].id))
                .unwrap()
                .contains("rename from docs/[old].txt")
        );
        assert!(h.diff(None).unwrap().contains("+working"));
    }

    #[test]
    fn file_timeline_github_links_use_only_the_owner_and_repository() {
        let expected = Some("https://github.com/example/project");
        for origin in [
            "git@github.com:example/project.git",
            "ssh://git@github.com/example/project.git",
            "https://fixture:synthetic@github.com/example/project.git/",
            "https://github.com/example/project",
        ] {
            assert_eq!(github_repository(origin).as_deref(), expected);
        }
        for origin in [
            "https://notgithub.com/example/project",
            "https://github.com.invalid/example/project",
            "file:///github.com/example/project",
            "https://github.com/example/project?token=synthetic",
            "git@github.com:../project",
            "https://github.com/example/project/extra",
        ] {
            assert!(github_repository(origin).is_none());
        }
    }
    #[tokio::test]
    async fn file_timeline_hover_stats_count_the_entire_commit_and_cache_the_result() {
        let repo = tempfile::tempdir().unwrap();
        let root = repo.path();
        git_fixture(root, &["init", "-q"]);
        git_fixture(
            root,
            &[
                "remote",
                "add",
                "origin",
                "git@github.com:example/project.git",
            ],
        );
        let file = root.join("viewed.txt");
        std::fs::write(&file, "old line\n").unwrap();
        git_fixture(root, &["add", "."]);
        git_fixture(root, &["commit", "-qm", "base"]);
        std::fs::write(&file, "new line\n").unwrap();
        std::fs::write(root.join("other.txt"), "extra\nlines\n").unwrap();
        std::fs::write(root.join("binary.dat"), b"\0data").unwrap();
        git_fixture(root, &["add", "."]);
        git_fixture(root, &["commit", "-qm", "multiple paths"]);
        let history = read(&file, 20).unwrap();
        let commit = &history.commits[0];
        assert!(commit.stats.get().is_none());
        let key = Some(commit.id.as_str());
        let mut rx = history.details(key, false, 0).unwrap().stats.unwrap();
        let second = history.details(key, true, 0).unwrap().stats.unwrap();
        assert!(rx.same_channel(&second));
        let stats = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            rx.wait_for(Option::is_some),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        assert_eq!(
            history.details(key, false, 0).unwrap().github_url(),
            Some(format!(
                "https://github.com/example/project/commit/{}",
                commit.id
            ))
        );
        assert_eq!(
            stats,
            Stats {
                files: 3,
                insertions: 3,
                deletions: 1
            }
        );
        assert_eq!(
            stats.labels(true),
            (
                "3 fichiers modifiés".into(),
                Some("3 insertions(+)".into()),
                Some("1 suppression(-)".into())
            )
        );
        assert_eq!(
            Stats {
                files: 1,
                ..Default::default()
            }
            .labels(false),
            ("1 file changed".into(), None, None)
        );
        assert!(history.commits[1].stats.get().is_none());
    }

    #[test]
    fn file_timeline_loads_only_selected_sources() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("project");
        std::fs::create_dir(&repo).unwrap();
        let file = repo.join("file.txt");
        std::fs::write(&file, "one").unwrap();
        git_fixture(&repo, &["init", "-q"]);
        git_fixture(&repo, &["add", "."]);
        git_fixture(&repo, &["commit", "-qm", "first"]);
        let profile = dir.path().join("profile");
        crate::local_file_history::record(&profile, &file, "one").unwrap();
        std::fs::write(&file, "two").unwrap();
        for git in [false, true] {
            for local in [false, true] {
                let data = load(&file, 20, Some(&profile), Sources { git, local }).unwrap();
                assert_eq!(data.commits.len(), usize::from(git));
                assert_eq!(data.dirty, git);
                assert_eq!(data.saves.len(), usize::from(local));
            }
        }
    }

    #[test]
    fn file_timeline_abandons_old_worker_when_active_file_changes() {
        let (tx, rx) = mpsc::channel();
        let mut state = State {
            pending: Some((0, rx)),
            file: Some(PathBuf::from("previous")),
            ..State::default()
        };
        state.select(None, Sources::default(), 220);
        tx.send(Err("stale error".into())).unwrap();
        assert!(state.poll());
        assert!(state.file.is_none() && state.data.is_none() && state.pending.is_none());
    }
}
