//! Persistent, local-only state. Restoring metadata never starts a process.
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub type Result<T> = std::result::Result<T, String>;

const fn default_pane_broadcast() -> bool {
    true
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SideTab {
    #[default]
    Files,
    Changes,
    Tasks,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Panels {
    pub left_open: bool,
    pub right_open: bool,
    pub right_tab: SideTab,
}

/// Floating-terminal preferences are separate from the project pane tree.
/// They contain no terminal output, process authority or remote credentials.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct FloatingPreferences {
    pub open: bool,
    pub height: f64,
}

impl Default for FloatingPreferences {
    fn default() -> Self {
        Self {
            open: false,
            height: crate::palette::FLOATING_DEFAULT_HEIGHT,
        }
    }
}

impl Default for Panels {
    fn default() -> Self {
        Self {
            left_open: true,
            right_open: true,
            right_tab: SideTab::Files,
        }
    }
}

impl Panels {
    pub fn toggle_right(&mut self, tab: SideTab) {
        self.right_open = !self.right_open || self.right_tab != tab;
        self.right_tab = tab;
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Project {
    pub id: u64,
    pub name: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Host {
    pub alias: String,
    pub folder: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Task {
    pub id: u64,
    pub project: u64,
    pub title: String,
    pub column: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NativeThread {
    pub project: u64,
    pub provider: String,
    pub thread: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedSession {
    /// An explicit native marker suppresses the legacy SSH launch path. It is
    /// inert on restore/import; terminal authority is never stored here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_host: Option<SavedHostTerminal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_model: Option<crate::pane_session_model::SavedPaneModel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_seed: Option<crate::codex_terminal::Seed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_options: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_options: Option<serde_json::Value>,
    #[serde(default)]
    pub pane_name: Option<String>,
    #[serde(default = "default_pane_broadcast")]
    pub broadcast: bool,
    pub id: u64,
    pub project: Option<u64>,
    pub title: String,
    pub launch: crate::launch::Launch,
    pub cwd: PathBuf,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SavedHostTerminal {
    pub identity: Option<SavedHostTerminalIdentity>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedHostTerminalIdentity {
    pub epoch: tessel_host_protocol::Epoch,
    pub terminal: tessel_host_protocol::TerminalId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Workspace {
    pub schema: u32,
    pub projects: Vec<Project>,
    pub hosts: Vec<Host>,
    pub agent_model_defaults:
        std::collections::BTreeMap<String, crate::agent_model_defaults::Defaults>,
    pub agent_prefs: std::collections::BTreeMap<String, crate::agent_preferences::Preferences>,
    pub tasks: Vec<Task>,
    pub sessions: Vec<SavedSession>,
    pub next_pane: u64,
    pub native_threads: std::collections::HashMap<u64, NativeThread>,
    pub chat_threads: std::collections::HashMap<u64, String>,
    pub claude_threads: std::collections::HashMap<u64, String>,
    pub active_project: Option<u64>,
    pub dark: bool,
    pub theme: Option<crate::theme_palettes::ThemeId>,
    pub language: crate::i18n::Language,
    pub columns: usize,
    pub broadcast: bool,
    pub panels: Panels,
    pub floating: FloatingPreferences,
    pub terminal_layouts: std::collections::HashMap<String, crate::terminal_layout::TerminalLayout>,
    pub font_size: f32,
    pub next_id: u64,
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            schema: 1,
            projects: vec![],
            hosts: vec![],
            agent_prefs: Default::default(),
            agent_model_defaults: Default::default(),
            tasks: vec![],
            sessions: vec![],
            next_pane: 1,
            native_threads: Default::default(),
            chat_threads: std::collections::HashMap::new(),
            claude_threads: Default::default(),
            active_project: None,
            dark: true,
            theme: None,
            language: Default::default(),
            columns: 1,
            broadcast: false,
            panels: Default::default(),
            floating: Default::default(),
            terminal_layouts: Default::default(),
            font_size: 15.0,
            next_id: 1,
        }
    }
}

impl Workspace {
    pub fn sync_terminal_layouts(&mut self, room: [f64; 2]) {
        let mut groups: std::collections::HashMap<String, Vec<u64>> = Default::default();
        for session in &self.sessions {
            groups
                .entry(crate::terminal_layout::project_key(session.project))
                .or_default()
                .push(session.id);
        }
        self.terminal_layouts
            .retain(|key, _| groups.contains_key(key));
        for (key, ids) in groups {
            self.terminal_layouts
                .entry(key)
                .or_default()
                .sync(&ids, room);
        }
    }
    pub fn validate(&self) -> Result<()> {
        if !self.floating.height.is_finite()
            || !(crate::palette::FLOATING_MIN_HEIGHT..=crate::palette::FLOATING_MAX_HEIGHT)
                .contains(&self.floating.height)
        {
            return Err("Invalid floating terminal height.".into());
        }
        if self.schema != 1 {
            return Err(
                "This workspace was written by an unsupported version; it will not be overwritten."
                    .into(),
            );
        }
        if self.projects.len() > 1000 || self.hosts.len() > 1000 || self.tasks.len() > 10000 {
            return Err("Workspace exceeds supported limits.".into());
        }
        if self
            .agent_prefs
            .keys()
            .any(|id| crate::agent_catalog::find(id).is_none())
        {
            return Err("Unknown saved agent preference.".into());
        }
        if self.agent_model_defaults.len() > 60
            || self.agent_model_defaults.iter().any(|(id, defaults)| {
                crate::agent_model_catalog::find(id).is_none() || !defaults.validate()
            })
        {
            return Err("Invalid saved agent model defaults.".into());
        }
        if self.sessions.len() > 100 || self.next_id == u64::MAX {
            return Err("Workspace limits exceeded.".into());
        }
        if self.chat_threads.len() > 1000
            || self.chat_threads.values().any(|id| {
                id.is_empty()
                    || id.len() > 100
                    || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            })
        {
            return Err("Invalid saved chat metadata.".into());
        }
        if self.claude_threads.len() > 1000
            || self
                .claude_threads
                .values()
                .any(|id| !crate::claude::valid_uuid(id))
        {
            return Err("Invalid saved Claude chat metadata".into());
        }
        let mut session_ids = std::collections::HashSet::new();
        if self.next_pane == u64::MAX || self.native_threads.len() > 1000 {
            return Err("Native chat metadata limit exceeded.".into());
        }
        for (pane, thread) in &self.native_threads {
            if *pane == u64::MAX || !valid_native_thread(&thread.provider, &thread.thread) {
                return Err("Invalid saved native chat thread.".into());
            }
            if let Some(session) = self.sessions.iter().find(|s| s.id == *pane)
                && (session.project != Some(thread.project)
                    || session.launch.native_chat() != Some(thread.provider.as_str()))
            {
                return Err("Native chat thread belongs to a different pane.".into());
            }
        }
        for session in &self.sessions {
            if session.native_host.is_some()
                && !matches!(session.launch, crate::launch::Launch::Ssh(_))
            {
                return Err("Native host metadata requires an SSH pane.".into());
            }
            if let Some(model) = &session.last_model {
                let value =
                    serde_json::to_value(model).map_err(|_| "Invalid saved model metadata.")?;
                if !matches!(session.launch, crate::launch::Launch::Agent(_))
                    || crate::pane_session_model::saved_pane_model(Some(&value)).as_ref()
                        != Some(model)
                {
                    return Err("Invalid saved model metadata.".into());
                }
            }
            if let Some(seed) = &session.codex_seed
                && (!seed.valid()
                    || session.conversation_id.is_some()
                    || !matches!(&session.launch, crate::launch::Launch::Agent(agent) if agent == "codex"))
            {
                return Err("Invalid Codex recovery metadata.".into());
            }
            if let Some(id) = &session.conversation_id
                && (!crate::claude::valid_uuid(id)
                    || !matches!(&session.launch, crate::launch::Launch::Agent(agent) if crate::claude_terminal::supported(agent) || agent == "codex"))
            {
                return Err("Invalid terminal conversation metadata.".into());
            }
            if !session_ids.insert(session.id)
                || session.id == u64::MAX
                || !session.cwd.is_absolute()
                || session.title.len() > 512
            {
                return Err("Invalid session metadata.".into());
            }
            if session.launch_fingerprint.as_deref().is_some_and(|value| {
                !matches!(session.launch, crate::launch::Launch::Agent(_))
                    || !crate::launch_fingerprint::valid(value)
            }) {
                return Err("Invalid saved launch fingerprint.".into());
            }
            if let Some(values) = &session.session_options {
                let agent = match &session.launch {
                    crate::launch::Launch::Agent(agent) => agent,
                    _ => return Err("Only agent terminals may have saved session options.".into()),
                };
                if crate::agent_model_catalog::find(agent).is_none()
                    || !valid_session_options(values)
                {
                    return Err("Invalid saved session model options.".into());
                }
            }
            if let Some(values) = &session.native_options {
                let provider = match session.launch {
                    crate::launch::Launch::CodexChat => "codex",
                    crate::launch::Launch::ClaudeChat => "claude",
                    _ => return Err("Only native chats may have native options.".into()),
                };
                if !serde_json::from_value::<crate::native_chat_options::Options>(values.clone())
                    .is_ok_and(|options| options.valid_for(provider))
                {
                    return Err("Invalid native chat model options.".into());
                }
            }
            if session.launch.native_chat().is_some() {
                let project = session
                    .project
                    .and_then(|id| self.projects.iter().find(|p| p.id == id));
                if project.is_none_or(|p| p.path != session.cwd) {
                    return Err("Invalid native chat pane.".into());
                }
            }
            match &session.launch {
                crate::launch::Launch::Agent(name)
                    if crate::agent_catalog::find(name).is_none() =>
                {
                    return Err("Unknown saved agent.".into());
                }
                crate::launch::Launch::Ssh(host) => validate_host(&host.alias)?,
                _ => {}
            }
        }
        if self.terminal_layouts.len() > 101 {
            return Err("Too many terminal layouts".into());
        }
        for (key, layout) in &self.terminal_layouts {
            let allowed = self
                .sessions
                .iter()
                .filter(|session| crate::terminal_layout::project_key(session.project) == *key)
                .map(|session| session.id)
                .collect();
            layout.validate(&allowed)?;
        }
        if !(1..=3).contains(&self.columns)
            || !self.font_size.is_finite()
            || !(10.0..=28.0).contains(&self.font_size)
        {
            return Err("Invalid appearance settings.".into());
        }
        let mut ids = std::collections::HashSet::new();
        for project in &self.projects {
            if !ids.insert(project.id)
                || project.id >= self.next_id
                || project.name.len() > 512
                || !project.path.is_absolute()
            {
                return Err("Invalid project metadata.".into());
            }
        }
        for task in &self.tasks {
            if !ids.insert(task.id)
                || task.id >= self.next_id
                || task.column > 3
                || task.title.len() > 4096
            {
                return Err("Invalid task metadata.".into());
            }
        }
        for host in &self.hosts {
            validate_host(&host.alias)?;
        }
        Ok(())
    }
    pub fn next_pane_id(&self) -> u64 {
        self.sessions
            .iter()
            .map(|s| s.id)
            .chain(self.native_threads.keys().copied())
            .fold(self.next_pane.max(1), |next, id| {
                next.max(id.saturating_add(1))
            })
    }

    pub fn migrate_native_threads(&mut self) {
        let mut names: Vec<_> = self
            .sessions
            .iter()
            .map(|session| crate::agent_names::NamedPane {
                id: session.id,
                number: session.id,
                named: matches!(
                    session.launch,
                    crate::launch::Launch::Agent(_)
                        | crate::launch::Launch::CodexChat
                        | crate::launch::Launch::ClaudeChat
                ),
                name: session.pane_name.clone(),
            })
            .collect();
        crate::agent_names::ensure_agent_names(&mut names);
        for (session, name) in self.sessions.iter_mut().zip(names) {
            session.pane_name = name.name;
        }

        for session in &self.sessions {
            let Some(project) = session.project else {
                continue;
            };
            let Some(provider) = session.launch.native_chat() else {
                continue;
            };
            let legacy = if provider == "Codex" {
                self.chat_threads.remove(&project)
            } else {
                self.claude_threads.remove(&project)
            };
            if let Some(thread) = legacy {
                self.native_threads
                    .entry(session.id)
                    .or_insert_with(|| NativeThread {
                        project,
                        provider: provider.into(),
                        thread,
                    });
            }
        }
        self.next_pane = self.next_pane_id();
    }

    pub fn native_resume(&self, pane: u64, project: u64, provider: &str) -> Option<&str> {
        self.native_threads
            .get(&pane)
            .filter(|saved| saved.project == project && saved.provider == provider)
            .map(|saved| saved.thread.as_str())
    }

    pub fn remember_native_thread(
        &mut self,
        pane: u64,
        project: u64,
        provider: &str,
        thread: Option<&str>,
    ) -> Result<bool> {
        if !self.sessions.iter().any(|s| {
            s.id == pane && s.project == Some(project) && s.launch.native_chat() == Some(provider)
        }) {
            return Err("Native chat pane no longer exists in this project.".into());
        }
        let Some(thread) = thread else {
            return Ok(self.native_threads.remove(&pane).is_some());
        };
        if !valid_native_thread(provider, thread) {
            return Err("Invalid native thread ID.".into());
        }
        if self.native_resume(pane, project, provider) == Some(thread) {
            return Ok(false);
        }
        if !self.native_threads.contains_key(&pane) && self.native_threads.len() >= 1000 {
            return Err("Native thread history is full.".into());
        }
        self.native_threads.insert(
            pane,
            NativeThread {
                project,
                provider: provider.into(),
                thread: thread.into(),
            },
        );
        Ok(true)
    }

    pub fn id(&mut self) -> Result<u64> {
        let id = self.next_id;
        self.next_id = id
            .checked_add(1)
            .filter(|n| *n < u64::MAX)
            .ok_or("Workspace identifiers exhausted")?;
        Ok(id)
    }
    pub fn add_project(&mut self, path: &Path) -> Result<u64> {
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        if !path.is_dir() {
            return Err("Choose a directory.".into());
        }
        if let Some(project) = self.projects.iter().find(|p| p.path == path) {
            self.active_project = Some(project.id);
            return Ok(project.id);
        }
        let id = self.id()?;
        let name = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
            .to_string();
        self.projects.push(Project { id, name, path });
        self.active_project = Some(id);
        Ok(id)
    }
    pub fn active(&self) -> Option<&Project> {
        self.projects
            .iter()
            .find(|p| Some(p.id) == self.active_project)
    }
}

/// Persisted pane choices are complete, bounded model/option objects.
/// Interactive source-compatible normalization happens before saving them.
fn valid_session_options(value: &serde_json::Value) -> bool {
    // The source always keeps model, plus up to ten entries encountered
    // before model in the incoming object.
    let Some(values) = value.as_object().filter(|v| v.len() <= 11) else {
        return false;
    };
    let Some(model) = values.get("model").and_then(serde_json::Value::as_str) else {
        return false;
    };
    let options = values
        .iter()
        .filter(|(key, _)| key.as_str() != "model")
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    crate::agent_model_defaults::Defaults {
        model: Some(model.into()),
        values_by_model: std::collections::BTreeMap::from([(model.into(), options)]),
    }
    .validate()
}

fn valid_native_thread(provider: &str, thread: &str) -> bool {
    match provider {
        "Codex" => {
            !thread.is_empty()
                && thread.len() <= 100
                && thread
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-')
        }
        "Claude" => crate::claude::valid_uuid(thread),
        _ => false,
    }
}

pub fn validate_host(alias: &str) -> Result<()> {
    if alias.is_empty()
        || alias.starts_with('-')
        || alias.len() > 253
        || !alias
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-@:".contains(c))
    {
        return Err(
            "Use an SSH config alias or user@host (no options, spaces or commands).".into(),
        );
    }
    Ok(())
}

pub struct Store {
    path: PathBuf,
    _lock: fs::File,
}

impl Store {
    pub fn directory(&self) -> &Path {
        self.path.parent().expect("workspace file has a directory")
    }
    pub fn open(dir: &Path) -> Result<(Self, Workspace)> {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(dir.join("workspace.lock"))
            .map_err(|e| e.to_string())?;
        lock.try_lock_exclusive()
            .map_err(|_| "Another Tessel Rust instance is using this workspace.".to_string())?;
        let path = dir.join("workspace.json");
        let state = match fs::File::open(&path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(4 * 1024 * 1024 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                if bytes.len() > 4 * 1024 * 1024 {
                    return Err("Workspace is too large; original retained.".into());
                }
                let mut state: Workspace = serde_json::from_slice(&bytes)
                    .map_err(|e| format!("Cannot read workspace; original retained: {e}"))?;
                state.validate()?;
                state.migrate_native_threads();
                state.validate()?;
                state
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Workspace::default(),
            Err(e) => return Err(e.to_string()),
        };
        Ok((Self { path, _lock: lock }, state))
    }
    pub fn restore(&self, before: &Workspace, after: &Workspace) -> Result<PathBuf> {
        after.validate()?;
        let backup = tempfile::Builder::new()
            .prefix("workspace-before-restore-")
            .suffix(".json")
            .tempfile_in(self.path.parent().ok_or("No settings directory")?)
            .map_err(|e| e.to_string())?;
        let (_, path) = backup.keep().map_err(|e| e.to_string())?;
        atomic_write(
            &path,
            &serde_json::to_vec_pretty(before).map_err(|e| e.to_string())?,
        )?;
        self.save(after)?;
        Ok(path)
    }
    pub fn save(&self, state: &Workspace) -> Result<()> {
        state.validate()?;
        let bytes = serde_json::to_vec_pretty(state).map_err(|e| e.to_string())?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err("Workspace exceeds 4 MiB; previous settings retained".into());
        }
        atomic_write(&self.path, &bytes)
    }
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or("No parent directory")?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    if let Ok(metadata) = fs::metadata(path) {
        temp.as_file()
            .set_permissions(metadata.permissions())
            .map_err(|e| e.to_string())?;
    }
    temp.write_all(bytes).map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_conversations_round_trip_and_reject_invalid_or_wrong_provider_ids() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        let id = crate::claude::uuid().unwrap();
        state.sessions.push(SavedSession {
            native_host: None,
            last_model: None,
            codex_seed: None,
            conversation_id: Some(id.clone()),
            launch_fingerprint: None,
            session_options: None,
            native_options: None,
            pane_name: None,
            broadcast: true,
            id: 1,
            project: None,
            title: "Synthetic".into(),
            launch: crate::launch::Launch::Agent("claude".into()),
            cwd: dir.path().to_owned(),
        });
        store.save(&state).unwrap();
        drop(store);
        let (store, mut loaded) = Store::open(dir.path()).unwrap();
        assert_eq!(
            loaded.sessions[0].conversation_id.as_deref(),
            Some(id.as_str())
        );
        loaded.sessions[0].conversation_id = Some("../../untrusted".into());
        assert!(store.save(&loaded).is_err());
        loaded.sessions[0].conversation_id = Some(id);
        loaded.sessions[0].launch = crate::launch::Launch::Shell;
        assert!(store.save(&loaded).is_err());
        loaded.sessions[0].launch = crate::launch::Launch::Agent("codex".into());
        store.save(&loaded).unwrap();
        drop(store);
        let (_, codex_loaded) = Store::open(dir.path()).unwrap();
        assert_eq!(
            codex_loaded.sessions[0].conversation_id,
            loaded.sessions[0].conversation_id
        );
        let mut recovery = codex_loaded;
        let seed = crate::codex_terminal::Discovery::new(
            &crate::agent_preferences::Preferences {
                env: "CODEX_HOME=synthetic".into(),
                ..Default::default()
            },
            dir.path(),
        )
        .unwrap()
        .seed();
        recovery.sessions[0].codex_seed = Some(seed.clone());
        assert!(recovery.validate().is_err()); // ID and pending discovery are exclusive.
        recovery.sessions[0].conversation_id = None;
        assert!(recovery.validate().is_ok());
        let restored: Workspace =
            serde_json::from_slice(&serde_json::to_vec(&recovery).unwrap()).unwrap();
        assert_eq!(restored.sessions[0].codex_seed, Some(seed));
        recovery.sessions[0].launch = crate::launch::Launch::Shell;
        assert!(recovery.validate().is_err());
        let mut old = serde_json::to_value(&state.sessions[0]).unwrap();
        old.as_object_mut().unwrap().remove("conversation_id");
        assert!(
            serde_json::from_value::<SavedSession>(old)
                .unwrap()
                .conversation_id
                .is_none()
        );
    }
    #[test]
    fn pane_model_choices_roundtrip_independently_and_invalid_choices_never_replace_state() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        for (id, options) in [
            (1, json!({"model":"pane-one"})),
            (
                2,
                json!({"model":"pane-two","effort":"low","fastMode":false}),
            ),
        ] {
            state.sessions.push(SavedSession {
                native_host: None,
                last_model: None,
                codex_seed: None,
                conversation_id: None,
                launch_fingerprint: None,
                session_options: Some(options),
                native_options: None,
                pane_name: None,
                broadcast: true,
                id,
                project: None,
                title: "Codex".into(),
                launch: crate::launch::Launch::Agent("codex".into()),
                cwd: dir.path().to_owned(),
            });
        }
        store.save(&state).unwrap();
        let snapshot = std::fs::read(dir.path().join("workspace.json")).unwrap();
        for invalid in [
            json!({"effort":"low"}),
            json!({"model":"bad;command"}),
            json!({"model":"ok","effort":7}),
            json!({"model":"ok","invalid-id":true}),
        ] {
            let mut next = state.clone();
            next.sessions[0].session_options = Some(invalid);
            assert!(store.save(&next).is_err());
            assert_eq!(
                std::fs::read(dir.path().join("workspace.json")).unwrap(),
                snapshot
            );
        }
        let mut shell = state.clone();
        shell.sessions[0].launch = crate::launch::Launch::Shell;
        assert!(shell.validate().is_err());
        let mut many = json!({"model":"bounded"});
        for index in 0..10 {
            many[format!("option{index}")] = true.into();
        }
        let mut bounded = state.clone();
        bounded.sessions[0].session_options = Some(many.clone());
        assert!(bounded.validate().is_ok());
        many["overflow"] = false.into();
        bounded.sessions[0].session_options = Some(many);
        assert!(bounded.validate().is_err());
        let mut old = serde_json::to_value(&state.sessions[0]).unwrap();
        old.as_object_mut().unwrap().remove("session_options");
        assert!(
            serde_json::from_value::<SavedSession>(old)
                .unwrap()
                .session_options
                .is_none()
        );
        drop(store);
        let (_, restored) = Store::open(dir.path()).unwrap();
        assert_eq!(
            restored.sessions[0].session_options,
            state.sessions[0].session_options
        );
        assert_eq!(
            restored.sessions[1].session_options,
            state.sessions[1].session_options
        );
    }
    #[test]
    fn projects_roundtrip_and_select_automatically() {
        let dir = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        let id = state.add_project(project.path()).unwrap();
        assert_eq!(state.active_project, Some(id));
        assert_eq!(state.add_project(project.path()).unwrap(), id);
        assert_eq!(state.projects.len(), 1);
        store.save(&state).unwrap();
        assert!(Store::open(dir.path()).is_err());
        drop(store);
        let (_, loaded) = Store::open(dir.path()).unwrap();
        assert_eq!(loaded.projects, state.projects);
    }
    #[test]
    fn language_survives_restart_and_old_workspaces_keep_their_default() {
        let old: Workspace = serde_json::from_str("{}").unwrap();
        assert_eq!(old.language, crate::i18n::Language::English);
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        state.language = crate::i18n::Language::French;
        store.save(&state).unwrap();
        drop(store);
        let (_, loaded) = Store::open(dir.path()).unwrap();
        assert_eq!(loaded.language, crate::i18n::Language::French);
        assert!(loaded.sessions.is_empty());
    }
    #[test]
    fn named_themes_survive_restart_and_preserve_legacy_light_preference() {
        let old: Workspace = serde_json::from_str(r#"{"dark":false}"#).unwrap();
        assert!(!old.dark);
        assert!(old.theme.is_none());
        let dir = tempfile::tempdir().unwrap();
        for theme in crate::theme_palettes::THEMES {
            let (store, mut state) = Store::open(dir.path()).unwrap();
            state.theme = Some(theme);
            store.save(&state).unwrap();
            drop(store);
            let (store, loaded) = Store::open(dir.path()).unwrap();
            assert_eq!(loaded.theme, Some(theme));
            assert!(loaded.sessions.is_empty());
            drop(store);
        }
        assert!(serde_json::from_str::<Workspace>(r#"{"theme":"unrecognized-theme"}"#).is_err());
    }
    #[test]
    fn corrupt_state_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("workspace.json"), b"broken").unwrap();
        assert!(Store::open(dir.path()).is_err());
        assert_eq!(
            fs::read(dir.path().join("workspace.json")).unwrap(),
            b"broken"
        );
    }
    #[test]
    fn floating_preferences_survive_restart_without_joining_the_grid() {
        let old: Workspace = serde_json::from_str("{}").unwrap();
        assert_eq!(old.floating, FloatingPreferences::default());
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        state.floating = FloatingPreferences {
            open: true,
            height: 0.72,
        };
        store.save(&state).unwrap();
        drop(store);
        let (_, loaded) = Store::open(dir.path()).unwrap();
        assert_eq!(loaded.floating, state.floating);
        assert!(loaded.sessions.is_empty());
        for height in [f64::NAN, f64::INFINITY, 0.14, 0.91] {
            state.floating.height = height;
            assert!(state.validate().is_err());
        }
    }
    #[test]
    fn panel_toggles_survive_restart_and_old_settings_keep_visible_defaults() {
        let old: Workspace = serde_json::from_str("{}").unwrap();
        assert_eq!(old.panels, Panels::default());
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        state.panels.left_open = false;
        state.panels.toggle_right(SideTab::Files);
        assert!(!state.panels.right_open);
        state.panels.toggle_right(SideTab::Tasks);
        assert!(state.panels.right_open);
        assert_eq!(state.panels.right_tab, SideTab::Tasks);
        state.panels.toggle_right(SideTab::Changes);
        assert!(state.panels.right_open);
        store.save(&state).unwrap();
        drop(store);
        let (_, loaded) = Store::open(dir.path()).unwrap();
        assert_eq!(loaded.panels, state.panels);
        assert!(loaded.sessions.is_empty());
        assert!(
            serde_json::from_str::<Workspace>(r#"{"panels":{"right_tab":"unknown"}}"#).is_err()
        );
    }
    #[test]
    fn native_chat_metadata_is_project_scoped_unique_and_persistent() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        let project = state.add_project(dir.path()).unwrap();
        let cwd = state.active().unwrap().path.clone();
        for (id, launch) in [
            (1, crate::launch::Launch::CodexChat),
            (2, crate::launch::Launch::ClaudeChat),
        ] {
            state.sessions.push(SavedSession {
                native_host: None,
                last_model: None,
                codex_seed: None,
                conversation_id: None,
                launch_fingerprint: None,
                session_options: None,
                native_options: Some(serde_json::json!({"effort":"high"})),
                pane_name: None,
                broadcast: true,
                id,
                project: Some(project),
                title: launch.native_chat().unwrap().into(),
                launch,
                cwd: cwd.clone(),
            });
        }
        state.sync_terminal_layouts([1000.0, 700.0]);
        store.save(&state).unwrap();
        drop(store);
        let (_, loaded) = Store::open(dir.path()).unwrap();
        assert_eq!(loaded.sessions.len(), 2);
        assert_eq!(
            loaded.sessions[0].native_options,
            Some(serde_json::json!({"effort":"high"}))
        );
        assert!(
            loaded
                .sessions
                .iter()
                .all(|pane| pane.session_options.is_none())
        );
        let mut incompatible = loaded.clone();
        incompatible.sessions[0].native_options = Some(serde_json::json!({"effort":"bogus"}));
        assert!(incompatible.validate().is_err());
        incompatible = loaded.clone();
        incompatible.sessions[1].native_options = Some(serde_json::json!({"effort":"none"}));
        assert!(incompatible.validate().is_err());
        let mut bad_options = loaded.clone();
        bad_options.sessions[0].native_options = Some(serde_json::json!({"model":"--unsafe"}));
        assert!(bad_options.validate().is_err());
        bad_options.sessions[0].native_options = Some(serde_json::json!({"permissions":"yolo"}));
        assert!(bad_options.validate().is_err());

        assert_eq!(loaded.sessions[0].launch.native_chat(), Some("Codex"));
        assert_eq!(loaded.terminal_layouts, state.terminal_layouts);
        let mut invalid = loaded.clone();
        invalid.sessions[1].launch = crate::launch::Launch::CodexChat;
        assert!(invalid.validate().is_ok());
        invalid = loaded.clone();
        invalid.sessions[0].project = None;
        assert!(invalid.validate().is_err());
        invalid = loaded;
        invalid.sessions[0].cwd = cwd.join("unrelated");
        assert!(invalid.validate().is_err());
    }
    #[test]
    fn terminal_split_geometry_survives_storage_without_starting_saved_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        for id in 1..=2 {
            state.sessions.push(SavedSession {
                native_host: None,
                last_model: None,
                codex_seed: None,
                conversation_id: None,
                launch_fingerprint: None,
                session_options: None,
                native_options: None,
                pane_name: None,
                broadcast: true,
                id,
                project: None,
                title: format!("Sleeping {id}"),
                launch: crate::launch::Launch::Shell,
                cwd: dir.path().to_owned(),
            });
        }
        state.sync_terminal_layouts([1200.0, 700.0]);
        let layout = state.terminal_layouts.get_mut("global").unwrap();
        let geometry = crate::pane_grid::grid_layout(layout.tree.as_ref());
        assert!(layout.resize(&geometry.dividers[0].split_id, 0, 1200.0, 80.0, None));
        store.save(&state).unwrap();
        drop(store);
        let (_, loaded) = Store::open(dir.path()).unwrap();
        assert_eq!(loaded.terminal_layouts, state.terminal_layouts);
        assert_eq!(loaded.sessions.len(), 2);
        assert_eq!(loaded.sessions[0].title, "Sleeping 1");
    }
    #[test]
    fn refuses_ssh_options_and_shell_syntax() {
        for host in ["-oProxyCommand=evil", "host;command", "user@host\nnext", ""] {
            assert!(validate_host(host).is_err());
        }
        assert!(validate_host("user@example.org").is_ok());
    }
    #[test]
    fn session_metadata_roundtrip_does_not_start_processes() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        state.sessions.push(SavedSession {
            native_host: None,
            last_model: None,
            codex_seed: None,
            conversation_id: None,
            launch_fingerprint: None,
            session_options: None,
            native_options: None,
            pane_name: None,
            broadcast: true,
            id: 1,
            project: None,
            title: "Sleeping".into(),
            launch: crate::launch::Launch::Shell,
            cwd: dir.path().to_owned(),
        });
        store.save(&state).unwrap();
        drop(store);
        let (_, loaded) = Store::open(dir.path()).unwrap();
        assert_eq!(loaded.sessions.len(), 1);
        assert_eq!(loaded.sessions[0].title, "Sleeping");
    }
    #[test]
    fn multiple_native_panes_have_independent_resume_ids_and_legacy_migration_is_one_time() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        let project = state.add_project(dir.path()).unwrap();
        let cwd = state.active().unwrap().path.clone();
        for id in [10, 11] {
            state.sessions.push(SavedSession {
                native_host: None,
                last_model: None,
                codex_seed: None,
                conversation_id: None,
                launch_fingerprint: None,
                session_options: None,
                native_options: None,
                pane_name: None,
                broadcast: true,
                id,
                project: Some(project),
                title: format!("Codex {id}"),
                launch: crate::launch::Launch::CodexChat,
                cwd: cwd.clone(),
            });
        }
        state.sessions.push(SavedSession {
            native_host: None,
            last_model: None,
            codex_seed: None,
            conversation_id: None,
            launch_fingerprint: None,
            session_options: None,
            native_options: None,
            pane_name: None,
            broadcast: true,
            id: 12,
            project: Some(project),
            title: "Claude".into(),
            launch: crate::launch::Launch::ClaudeChat,
            cwd: cwd.clone(),
        });
        let claude_id = "550e8400-e29b-41d4-a716-446655440000";
        state.claude_threads.insert(project, claude_id.into());
        state.chat_threads.insert(project, "legacy-thread".into());
        store.save(&state).unwrap();
        drop(store);
        let (store, mut state) = Store::open(dir.path()).unwrap();
        assert_eq!(
            state.native_resume(10, project, "Codex"),
            Some("legacy-thread")
        );
        assert_eq!(state.native_resume(11, project, "Codex"), None);
        assert!(state.chat_threads.is_empty());
        assert_eq!(state.native_resume(12, project, "Claude"), Some(claude_id));
        assert_eq!(state.native_resume(12, project, "Codex"), None);
        assert!(state.claude_threads.is_empty());

        assert!(
            state
                .remember_native_thread(11, project, "Codex", Some("second-thread"))
                .unwrap()
        );
        assert!(
            state
                .remember_native_thread(11, project + 1, "Codex", Some("wrong-project"))
                .is_err()
        );
        assert!(
            state
                .remember_native_thread(10, project, "Claude", Some("wrong-provider"))
                .is_err()
        );
        state.migrate_native_threads();
        assert_eq!(
            state.native_resume(10, project, "Codex"),
            Some("legacy-thread")
        );
        assert_eq!(
            state.native_resume(11, project, "Codex"),
            Some("second-thread")
        );
        state.sessions.clear();
        assert!(state.next_pane_id() > 11);
        store.save(&state).unwrap();
        drop(store);
        let (_, state) = Store::open(dir.path()).unwrap();
        assert!(state.next_pane_id() > 11);
        assert_eq!(
            state.native_resume(11, project, "Codex"),
            Some("second-thread")
        );
        let mut bad = state.clone();
        bad.sessions.push(SavedSession {
            native_host: None,
            last_model: None,
            codex_seed: None,
            conversation_id: None,
            launch_fingerprint: None,
            session_options: None,
            native_options: None,
            pane_name: None,
            broadcast: true,
            id: 11,
            project: Some(project),
            title: "Shell".into(),
            launch: crate::launch::Launch::Shell,
            cwd,
        });
        assert!(bad.validate().is_err());
    }
    #[test]
    fn agent_names_migrate_once_and_survive_saved_workspace_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut state) = Store::open(dir.path()).unwrap();
        for (id, name, launch) in [
            (
                1,
                Some("My agent"),
                crate::launch::Launch::Agent("codex".into()),
            ),
            (
                2,
                Some("my AGENT"),
                crate::launch::Launch::Agent("claude".into()),
            ),
            (3, None, crate::launch::Launch::Agent("gemini".into())),
            (4, Some("Old shell label"), crate::launch::Launch::Shell),
        ] {
            state.sessions.push(SavedSession {
                native_host: None,
                last_model: None,
                id,
                codex_seed: None,
                conversation_id: None,
                launch_fingerprint: None,
                session_options: None,
                native_options: None,
                pane_name: name.map(str::to_owned),
                broadcast: true,
                project: None,
                title: format!("Program {id}"),
                launch,
                cwd: dir.path().to_owned(),
            });
        }
        store.save(&state).unwrap();
        drop(store);
        let (store, mut loaded) = Store::open(dir.path()).unwrap();
        let names: Vec<_> = loaded
            .sessions
            .iter()
            .map(|s| s.pane_name.as_deref())
            .collect();
        assert_eq!(names, [Some("My agent"), Some("Ada"), Some("Bohr"), None]);
        assert_eq!(loaded.sessions[0].title, "Program 1");
        loaded.sessions[1].pane_name = Some("Renamed agent".into());
        store.save(&loaded).unwrap();
        drop(store);
        let (_, reopened) = Store::open(dir.path()).unwrap();
        assert_eq!(
            reopened.sessions[1].pane_name.as_deref(),
            Some("Renamed agent")
        );
        assert_eq!(reopened.sessions[2].pane_name.as_deref(), Some("Bohr"));
        assert_eq!(reopened.sessions[1].title, "Program 2");
    }
    #[test]
    fn broadcast_flags_default_safely_for_legacy_json_and_persist_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let legacy_session: SavedSession = serde_json::from_value(serde_json::json!({
            "id": 1,
            "project": null,
            "title": "Shell",
            "launch": "Shell",
            "cwd": dir.path(),
        }))
        .unwrap();
        assert!(legacy_session.broadcast);
        let legacy_workspace: Workspace = serde_json::from_str(r#"{"schema":1}"#).unwrap();
        assert!(!legacy_workspace.broadcast);

        let (store, mut state) = Store::open(dir.path()).unwrap();
        state.broadcast = true;
        let mut unchecked = legacy_session;
        unchecked.broadcast = false;
        state.sessions.push(unchecked);
        store.save(&state).unwrap();
        drop(store);
        let (_, reopened) = Store::open(dir.path()).unwrap();
        assert!(reopened.broadcast);
        assert!(!reopened.sessions[0].broadcast);
    }
}
