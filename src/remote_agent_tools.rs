#![allow(dead_code)]
//! SSH remote agent tools (`Claude`, `Codex`, `Antigravity` MCP & hooks over SSH) and hardened
//! Claude Code workspace folder trust (`~/.claude.json`).
//!
//! Mirrors Tessel's `remoteAgentTools.js`, `antigravityMcp.js`, and `claudeTrust.js`:
//! - Early SSH pane registration so remote hook events arriving during startup are never dropped.
//! - Two-tier relink backoff (`5 s` for attempts `< 120`, then `60 s`) and proof-of-life clearing
//!   of `remote_tools_note` as soon as an MCP call or hook event arrives.
//! - Blocks PTY idle reminders (`team_inbox` / `project_inbox`) on remote SSH panes whose MCP
//!   bridge is not connected.
//! - Installs `tessel_team` (`["mcp"]`) and `tessel_apps` (`["mcp", "--apps"]`) into
//!   `~/.gemini/config/mcp_config.json` without `type` or `env` keys, preserving `disabled: true`,
//!   removing legacy `tessel-team`, creating a non-symlink `.tessel-bak`, and skipping cleanly
//!   when `~/.gemini` is uninitialized.
//! - Hardened Claude folder trust (`Workspace::agent_folder_trust`, default `true`): refuses `/`,
//!   `$HOME`, sensitive dot-directories, symlinks, world-writable directories, `from_copy`
//!   imported workspaces, and `~/.claude.json` files over `4 MiB`.

use std::fs;
use std::io::{Read, Write};
use std::path::Path;

use serde_json::{Map, Value, json};

fn posix_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// Agents supported by Tessel's SSH remote MCP/hook bridge.
pub const REMOTE_TOOLS_AGENTS: [&str; 3] = ["claude", "codex", "antigravity"];

/// Fast relink attempt cap (`120 × 5 s = 10 min`) before stepping down to [`SLOW_RELINK_DELAY_MS`].
pub const FAST_RELINK_ATTEMPTS: u32 = 120;
pub const FAST_RELINK_DELAY_MS: u64 = 5_000;
pub const SLOW_RELINK_DELAY_MS: u64 = 60_000;

/// Maximum size (`4 MiB`) permitted when reading or updating `~/.claude.json` or `mcp_config.json`.
pub const MAX_CLAUDE_JSON_BYTES: u64 = 4 * 1024 * 1024;

const SENSITIVE_DOTDIRS: [&str; 8] = [
    ".claude",
    ".ssh",
    ".gnupg",
    ".gemini",
    ".codex",
    ".aws",
    ".kube",
    ".tessel-server",
];

/// Returns `true` when `agent_id` supports Tessel's SSH remote MCP & hook bridge.
#[must_use]
pub fn supports_remote_agent_tools(agent_id: &str) -> bool {
    let norm = agent_id.trim().to_ascii_lowercase();
    REMOTE_TOOLS_AGENTS.contains(&norm.as_str())
}

/// Two-tier relink backoff (`5 000 ms` for attempts `0..119`, `60 000 ms` from `120` onward).
#[must_use]
pub const fn relink_delay_ms(attempt: u32) -> u64 {
    if attempt < FAST_RELINK_ATTEMPTS {
        FAST_RELINK_DELAY_MS
    } else {
        SLOW_RELINK_DELAY_MS
    }
}

/// Localized note shown when an agent's config folder does not yet exist on an SSH host.
#[must_use]
pub fn uninitialized_agent_host_note(agent_name: &str, french: bool) -> String {
    if french {
        format!(
            "{agent_name} n'a pas encore été initialisé sur cet hôte SSH — lancez-le une première fois pour activer les outils Tessel."
        )
    } else {
        format!(
            "{agent_name} is not initialized on this SSH host yet — start it once on the host to enable Tessel tools."
        )
    }
}

/// Returns `true` when Tessel must suppress typing a PTY `team_inbox` / `project_inbox` reminder
/// because the pane is a remote SSH session whose MCP bridge is not ready.
#[must_use]
pub const fn should_block_team_pty_reminder(
    is_remote_ssh_pane: bool,
    remote_tools_ready: bool,
) -> bool {
    is_remote_ssh_pane && !remote_tools_ready
}

/// Per-pane remote agent bridge state tracking early registration, relink attempts, and proof-of-life.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemotePaneToolState {
    pub pane_id: u64,
    pub agent_id: String,
    pub registered_early: bool,
    pub remote_tools_ready: bool,
    pub remote_tools_note: Option<String>,
    pub relink_attempts: u32,
    pub saw_proof_of_life: bool,
}

impl RemotePaneToolState {
    /// Registers an SSH agent pane before shell startup so early remote hooks are accepted.
    #[must_use]
    pub fn register_early(pane_id: u64, agent_id: impl Into<String>) -> Self {
        Self {
            pane_id,
            agent_id: agent_id.into(),
            registered_early: true,
            remote_tools_ready: false,
            remote_tools_note: None,
            relink_attempts: 0,
            saw_proof_of_life: false,
        }
    }

    /// Records a failed bridge setup/relink attempt and returns the next backoff delay in ms.
    pub fn record_relink_failure(&mut self, note: impl Into<String>) -> u64 {
        let delay = relink_delay_ms(self.relink_attempts);
        self.relink_attempts = self.relink_attempts.saturating_add(1);
        self.remote_tools_ready = false;
        self.remote_tools_note = Some(note.into());
        delay
    }

    /// Records proof-of-life (an authenticated MCP tool request or hook event from the remote pane),
    /// marking `remote_tools_ready = true` and clearing any stale warning note.
    pub fn record_proof_of_life(&mut self) {
        self.saw_proof_of_life = true;
        self.remote_tools_ready = true;
        self.remote_tools_note = None;
        self.relink_attempts = 0;
    }
}

/// Merges `tessel_team` and `tessel_apps` MCP server entries into Antigravity's `mcp_config.json`.
///
/// - Never emits `type` or `env` keys (Antigravity's schema rejects unknown `type`).
/// - Preserves `"disabled": true` if the user explicitly disabled either server.
/// - Removes legacy `"tessel-team"` entry if present.
pub fn merge_antigravity_json(
    existing_raw: Option<&str>,
    shim_path: &str,
) -> Result<String, String> {
    if shim_path.trim().is_empty() || shim_path.chars().any(char::is_control) {
        return Err("Invalid Tessel MCP shim path.".into());
    }
    let mut root: Value = match existing_raw.map(str::trim) {
        None | Some("") => json!({}),
        Some(text) => serde_json::from_str(text).map_err(|_| {
            "Existing ~/.gemini/config/mcp_config.json is not valid JSON.".to_owned()
        })?,
    };
    let root_obj = root
        .as_object_mut()
        .ok_or_else(|| "mcp_config.json root must be a JSON object.".to_owned())?;

    let servers_val = root_obj
        .entry("mcpServers".to_owned())
        .or_insert_with(|| json!({}));
    let servers = servers_val
        .as_object_mut()
        .ok_or_else(|| "mcpServers in mcp_config.json must be a JSON object.".to_owned())?;

    servers.remove("tessel-team");

    for (key, args) in [
        ("tessel_team", vec![Value::String("mcp".to_owned())]),
        (
            "tessel_apps",
            vec![
                Value::String("mcp".to_owned()),
                Value::String("--apps".to_owned()),
            ],
        ),
    ] {
        let was_disabled = servers
            .get(key)
            .and_then(Value::as_object)
            .and_then(|o| o.get("disabled"))
            .and_then(Value::as_bool)
            == Some(true);

        let mut entry = Map::new();
        entry.insert("command".to_owned(), Value::String(shim_path.to_owned()));
        entry.insert("args".to_owned(), Value::Array(args));
        if was_disabled {
            entry.insert("disabled".to_owned(), Value::Bool(true));
        }
        servers.insert(key.to_owned(), Value::Object(entry));
    }

    serde_json::to_string_pretty(&root).map_err(|e| e.to_string())
}

/// Outcome of installing Antigravity MCP entries under `<home>/.gemini/config/mcp_config.json`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AntigravityInstallOutcome {
    Updated,
    SkippedUninitialized,
}

fn ensure_not_symlink(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            Err(format!("Refusing symlink path: {}", path.display()))
        }
        _ => Ok(()),
    }
}

/// Installs `tessel_team` and `tessel_apps` into `<home>/.gemini/config/mcp_config.json`.
///
/// Skips cleanly without creating `.gemini` if Antigravity has not been initialized under `home`.
pub fn install_antigravity_mcp_on_fs(
    home: &Path,
    shim_path: &str,
) -> Result<AntigravityInstallOutcome, String> {
    let gemini_dir = home.join(".gemini");
    match fs::symlink_metadata(&gemini_dir) {
        Err(_) => return Ok(AntigravityInstallOutcome::SkippedUninitialized),
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err("Refusing symlinked ~/.gemini directory.".into());
        }
        Ok(meta) if !meta.is_dir() => {
            return Err("~/.gemini exists but is not a directory.".into());
        }
        Ok(_) => {}
    }

    let config_dir = gemini_dir.join("config");
    ensure_not_symlink(&config_dir)?;
    fs::create_dir_all(&config_dir).map_err(|e| e.to_string())?;

    let cfg_path = config_dir.join("mcp_config.json");
    let bak_path = config_dir.join("mcp_config.json.tessel-bak");
    ensure_not_symlink(&cfg_path)?;
    ensure_not_symlink(&bak_path)?;

    let existing_raw = if cfg_path.exists() {
        let meta = fs::metadata(&cfg_path).map_err(|e| e.to_string())?;
        if meta.len() > MAX_CLAUDE_JSON_BYTES {
            return Err("mcp_config.json exceeds 4 MiB safety limit.".into());
        }
        let raw = fs::read_to_string(&cfg_path).map_err(|e| e.to_string())?;
        fs::write(&bak_path, raw.as_bytes()).map_err(|e| e.to_string())?;
        Some(raw)
    } else {
        None
    };

    let merged = merge_antigravity_json(existing_raw.as_deref(), shim_path)?;
    let tmp_path = config_dir.join(".mcp_config.json.tessel-tmp");
    ensure_not_symlink(&tmp_path)?;
    fs::write(&tmp_path, merged.as_bytes()).map_err(|e| e.to_string())?;
    fs::rename(&tmp_path, &cfg_path).map_err(|e| e.to_string())?;
    Ok(AntigravityInstallOutcome::Updated)
}

fn normalize_path_for_compare(raw: &str) -> String {
    let separators = raw.trim().replace('\\', "/");
    let absolute = separators.starts_with('/');
    let mut s = separators
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>()
        .join("/");
    if absolute {
        s.insert(0, '/');
    }
    if s.len() == 2 && s.as_bytes()[1] == b':' {
        s.push('/');
    }
    while s.len() > 1 && s.ends_with('/') {
        let is_win_drive_root = s.len() == 3 && s.as_bytes()[1] == b':';
        if is_win_drive_root {
            break;
        }
        s.pop();
    }
    if cfg!(windows) {
        s.make_ascii_lowercase();
    }
    s
}

/// Validates that `folder` is safe to mark trusted in `~/.claude.json` (never `/`, `$HOME`, a
/// drive root, a sensitive dot-directory, a relative path, or a `..` traversal).
pub fn validate_trust_folder_path(folder: &str, home: &str) -> Result<String, String> {
    let trimmed = folder.trim();
    if trimmed.is_empty() || trimmed.len() > 4096 || trimmed.chars().any(char::is_control) {
        return Err("Invalid project folder path.".into());
    }
    let is_abs = trimmed.starts_with('/')
        || (trimmed.len() >= 3
            && trimmed.as_bytes()[0].is_ascii_alphabetic()
            && trimmed.as_bytes()[1] == b':'
            && matches!(trimmed.as_bytes()[2], b'/' | b'\\'));
    if !is_abs {
        return Err("Refusing relative folder path for Claude trust.".into());
    }
    if trimmed.split(['/', '\\']).any(|seg| seg == "..") {
        return Err("Refusing parent-traversal folder path for Claude trust.".into());
    }

    let norm_folder = normalize_path_for_compare(trimmed);
    if norm_folder == "/"
        || (norm_folder.len() == 3
            && norm_folder.as_bytes()[1] == b':'
            && norm_folder.ends_with('/'))
    {
        return Err("Refusing filesystem root for Claude folder trust.".into());
    }

    let norm_home = normalize_path_for_compare(home);
    if !norm_home.is_empty()
        && (norm_folder == norm_home
            || norm_home
                .strip_prefix(&norm_folder)
                .is_some_and(|rest| rest.starts_with('/')))
    {
        return Err("Refusing home directory or its ancestors for Claude folder trust.".into());
    }

    for seg in norm_folder.split('/') {
        if SENSITIVE_DOTDIRS
            .iter()
            .any(|dot| seg.eq_ignore_ascii_case(dot))
        {
            return Err(format!(
                "Refusing sensitive directory '{seg}' for Claude folder trust."
            ));
        }
    }

    let cleaned = if trimmed.len() > 3 {
        trimmed.trim_end_matches(['/', '\\']).to_owned()
    } else {
        trimmed.to_owned()
    };
    Ok(cleaned)
}

/// Merges `projects[<folder>].hasTrustDialogAccepted = true` into `~/.claude.json` while
/// preserving every existing project and top-level setting.
pub fn merge_claude_trust_json(
    existing_raw: Option<&str>,
    folder_key: &str,
) -> Result<String, String> {
    let mut root: Value = match existing_raw.map(str::trim) {
        None | Some("") => json!({}),
        Some(text) => serde_json::from_str(text)
            .map_err(|_| "Existing ~/.claude.json is not valid JSON.".to_owned())?,
    };
    let root_obj = root
        .as_object_mut()
        .ok_or_else(|| "~/.claude.json root must be a JSON object.".to_owned())?;
    let projects_val = root_obj
        .entry("projects".to_owned())
        .or_insert_with(|| json!({}));
    let projects = projects_val
        .as_object_mut()
        .ok_or_else(|| "~/.claude.json projects must be a JSON object.".to_owned())?;
    let proj_val = projects
        .entry(folder_key.to_owned())
        .or_insert_with(|| json!({}));
    let proj_obj = proj_val
        .as_object_mut()
        .ok_or_else(|| "~/.claude.json project entry must be a JSON object.".to_owned())?;
    proj_obj.insert("hasTrustDialogAccepted".to_owned(), Value::Bool(true));

    serde_json::to_string_pretty(&root).map_err(|e| e.to_string())
}

/// Applies local Claude folder trust to `<home>/.claude.json` when permitted by policy and
/// filesystem safety checks.
pub fn apply_local_claude_folder_trust(
    home: &Path,
    project_folder: &Path,
    agent_folder_trust_enabled: bool,
    workspace_from_copy: bool,
) -> Result<bool, String> {
    apply_local_claude_folder_trust_at(
        home,
        &home.join(".claude.json"),
        project_folder,
        agent_folder_trust_enabled,
        workspace_from_copy,
    )
}

/// The configuration path belongs to the account used by this launch.
pub fn claude_config_path(
    home: &Path,
    config_dir: Option<&std::ffi::OsStr>,
    custom_oauth: bool,
) -> std::path::PathBuf {
    let config_dir = config_dir
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from);
    let legacy = config_dir
        .clone()
        .unwrap_or_else(|| home.join(".claude"))
        .join(".config.json");
    if legacy.exists() {
        legacy
    } else {
        config_dir
            .unwrap_or_else(|| home.to_owned())
            .join(if custom_oauth {
                ".claude-custom-oauth.json"
            } else {
                ".claude.json"
            })
    }
}

/// Updates only an existing account configuration, under Claude's own lock.
pub fn apply_local_claude_folder_trust_at(
    home: &Path,
    claude_json: &Path,
    project_folder: &Path,
    agent_folder_trust_enabled: bool,
    workspace_from_copy: bool,
) -> Result<bool, String> {
    if !agent_folder_trust_enabled || workspace_from_copy {
        return Ok(false);
    }
    ensure_not_symlink(home)?;
    ensure_not_symlink(project_folder)?;

    let meta = fs::symlink_metadata(project_folder).map_err(|e| e.to_string())?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err("Target project folder must be a real directory (not a symlink).".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o002 != 0 {
            return Err("Refusing world-writable directory for Claude folder trust.".into());
        }
    }

    // Validate both the spelling and the filesystem target: dot components,
    // short Windows names and ancestor aliases must not bypass the home/root gate.
    validate_trust_folder_path(
        project_folder.to_str().ok_or("Non-UTF-8 project path")?,
        home.to_str().ok_or("Non-UTF-8 home path")?,
    )?;
    let real_home = dunce::canonicalize(home).map_err(|e| e.to_string())?;
    let real_folder = dunce::canonicalize(project_folder).map_err(|e| e.to_string())?;
    let key = validate_trust_folder_path(
        real_folder.to_str().ok_or("Non-UTF-8 project path")?,
        real_home.to_str().ok_or("Non-UTF-8 home path")?,
    )?;

    ensure_not_symlink(claude_json)?;
    if !claude_json.exists() {
        return Ok(false); // Claude creates its configuration on first use.
    }
    let mut lock_name = claude_json.as_os_str().to_owned();
    lock_name.push(".lock");
    let lock_path = std::path::PathBuf::from(lock_name);
    let mut acquired = false;
    for attempt in 0..=4 {
        match fs::create_dir(&lock_path) {
            Ok(()) => {
                acquired = true;
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && attempt < 4 => {
                std::thread::sleep(std::time::Duration::from_millis((50 << attempt).min(250)));
            }
            Err(_) => return Ok(false), // Never break Claude's lock.
        }
    }
    if !acquired {
        return Ok(false);
    }
    struct Lock(std::path::PathBuf);
    impl Drop for Lock {
        fn drop(&mut self) {
            let _ = fs::remove_dir(&self.0);
        }
    }
    let _lock = Lock(lock_path);
    // Re-read under the same lock Claude uses. Bound the read itself, not only
    // metadata, and use a unique temporary file so simultaneous launches cannot
    // lose one another's entries or share a predictable temporary filename.
    ensure_not_symlink(claude_json)?;
    let file = fs::File::open(claude_json).map_err(|e| e.to_string())?;
    let permissions = file.metadata().map_err(|e| e.to_string())?.permissions();
    let mut bytes = Vec::new();
    file.take(MAX_CLAUDE_JSON_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_CLAUDE_JSON_BYTES {
        return Err("~/.claude.json exceeds 4 MiB safety limit.".into());
    }
    let raw = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
    let key = if cfg!(windows) {
        key.replace('\\', "/")
    } else {
        key
    };
    let current: Value = serde_json::from_str(raw)
        .map_err(|_| "Existing Claude configuration is not valid JSON.".to_owned())?;
    if current["projects"][&key]["hasTrustDialogAccepted"] == Value::Bool(true) {
        return Ok(false);
    }
    let merged = merge_claude_trust_json(Some(raw), &key)?;
    let parent = claude_json.parent().ok_or("Configuration has no parent")?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temp.as_file()
        .set_permissions(permissions)
        .map_err(|e| e.to_string())?;
    temp.write_all(merged.as_bytes())
        .map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist(claude_json).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Builds a remote POSIX script that safely records `hasTrustDialogAccepted = true` in
/// `~/.claude.json` on an SSH host after verifying realpath, ownership/world-writable bits,
/// symlink absence, and the `4 MiB` size cap.
pub fn build_remote_claude_trust_script(
    remote_folder: &str,
    agent_folder_trust_enabled: bool,
    workspace_from_copy: bool,
) -> Result<Option<String>, String> {
    if !agent_folder_trust_enabled || workspace_from_copy {
        return Ok(None);
    }
    let trimmed = remote_folder.trim();
    if trimmed.is_empty() || trimmed == "~" {
        return Ok(None);
    }
    if trimmed.len() > 4096
        || trimmed.chars().any(char::is_control)
        || trimmed.split(['/', '\\']).any(|seg| seg == "..")
    {
        return Err("Invalid remote folder for Claude trust.".into());
    }
    for seg in trimmed.split('/') {
        if SENSITIVE_DOTDIRS
            .iter()
            .any(|dot| seg.eq_ignore_ascii_case(dot))
        {
            return Err(format!("Refusing sensitive remote directory '{seg}'."));
        }
    }
    if trimmed == "/" {
        return Err("Refusing root directory '/' for remote Claude trust.".into());
    }

    let quoted = posix_quote(trimmed);
    let script = format!(
        "set -eu; umask 077; \
         target={quoted}; \
         case \"$target\" in '~/'*) target=\"$HOME/${{target#~/}}\" ;; esac; \
         [ -d \"$target\" ] && [ ! -L \"$target\" ]; \
         real_t=$(cd -- \"$target\" && pwd -P); \
         real_h=$(cd -- \"$HOME\" && pwd -P); \
         [ \"$real_t\" != '/' ] && [ \"$real_t\" != \"$real_h\" ]; \
         perm=$(ls -ld \"$real_t\" | awk '{{print $1}}'); \
         case \"$perm\" in ????????w*) exit 71 ;; esac; \
         cfg=\"$HOME/.claude.json\"; bak=\"$HOME/.claude.json.tessel-bak\"; tmp=\"$HOME/.claude.json.tessel-tmp\"; \
         [ ! -L \"$cfg\" ] && [ ! -L \"$bak\" ] && [ ! -L \"$tmp\" ]; \
         if [ -f \"$cfg\" ]; then \
           sz=$(wc -c < \"$cfg\" | tr -d ' '); \
           [ \"$sz\" -le {MAX_CLAUDE_JSON_BYTES} ]; \
           cp -p \"$cfg\" \"$bak\"; \
         fi; \
         if command -v python3 >/dev/null 2>&1; then \
           TESSEL_TRUST_DIR=\"$real_t\" python3 -c 'import json, os, pathlib; \
p = pathlib.Path.home() / \".claude.json\"; \
d = json.loads(p.read_text(\"utf-8\")) if p.exists() and p.stat().st_size > 0 else {{}}; \
pr = d.setdefault(\"projects\", {{}}); \
pr.setdefault(os.environ[\"TESSEL_TRUST_DIR\"], {{}})[\"hasTrustDialogAccepted\"] = True; \
(pathlib.Path.home() / \".claude.json.tessel-tmp\").write_text(json.dumps(d, indent=2), \"utf-8\")' && mv -f \"$tmp\" \"$cfg\"; \
         fi"
    );
    Ok(Some(script))
}
