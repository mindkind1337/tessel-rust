//! Native Live Share backend (`shareHost.js`, `liveShareRights.js`, `liveShareAccess.js` parity).
//!
//! Provides both:
//! - One-shot TLS-pinned read-only `request()` compatibility (`tessel-rust-share:` v1/v2), and
//! - Persistent duplex `rustls` LAN sessions (`GuestSession` <-> `Host`) with 6-digit SAS
//!   verification (`"123 456"`), knock approval queue (`LiveShareAsk`), trusted peers,
//!   up to 20 managed invitations (`MAX_INVITES`), live presence, and strict host-side
//!   enforcement of the 8-flag `ShareRights` matrix.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    io::{Read, Write},
    net::{IpAddr, SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use subtle::ConstantTimeEq;

type Result<T> = std::result::Result<T, String>;

const MAX_REQUEST: usize = 256 * 1024;
const MAX_FILE: usize = 8 * 1024 * 1024;
const MAX_RESPONSE: usize = 12 * 1024 * 1024;
pub const MAX_INVITES: usize = 20;
pub const MAX_TRUSTED: usize = 64;
const TYPING_TTL_MS: u64 = 4_000;

pub fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn clock_hhmm(epoch_ms: u64) -> String {
    use chrono::{Local, TimeZone};
    let secs = (epoch_ms / 1000) as i64;
    if let chrono::LocalResult::Single(dt) = Local.timestamp_opt(secs, 0) {
        dt.format("%H:%M").to_string()
    } else {
        String::new()
    }
}

// ---------------------------------------------------------------------------
// 1. 8-Flag Rights Matrix & Presets (`liveShareRights.js` parity)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RightId {
    Type,
    Open,
    Close,
    Usage,
    Edit,
    Git,
    Tasks,
    PagesView,
}

impl RightId {
    pub const ALL: [Self; 8] = [
        Self::Type,
        Self::Open,
        Self::Close,
        Self::Usage,
        Self::Edit,
        Self::Git,
        Self::Tasks,
        Self::PagesView,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Type => "type",
            Self::Open => "open",
            Self::Close => "close",
            Self::Usage => "usage",
            Self::Edit => "edit",
            Self::Git => "git",
            Self::Tasks => "tasks",
            Self::PagesView => "pagesView",
        }
    }

    pub const fn is_danger(self) -> bool {
        matches!(self, Self::Type)
    }

    pub const fn short_label(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::Type, false) => "Type",
            (Self::Type, true) => "Taper",
            (Self::Open, false) => "Open",
            (Self::Open, true) => "Ouvrir",
            (Self::Close, false) => "Close",
            (Self::Close, true) => "Fermer",
            (Self::Usage, false) => "Usage",
            (Self::Usage, true) => "Utilisation",
            (Self::Edit, false) => "Edit",
            (Self::Edit, true) => "Modifier",
            (Self::Git, _) => "Git",
            (Self::Tasks, false) => "Tasks",
            (Self::Tasks, true) => "Tâches",
            (Self::PagesView, false) => "Pages",
            (Self::PagesView, true) => "Pages",
        }
    }

    pub const fn full_label(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::Type, false) => "Type in terminals and agents (they can run any command as you)",
            (Self::Type, true) => {
                "Taper dans les terminaux et les agents (cette personne peut lancer n'importe quelle commande en votre nom)"
            }
            (Self::Open, false) => {
                "Open terminals and agents on your computer (they can run commands)"
            }
            (Self::Open, true) => {
                "Ouvrir des terminaux et des agents sur votre ordinateur (cette personne peut lancer des commandes)"
            }
            (Self::Close, false) => "Ask to close panes",
            (Self::Close, true) => "Demander à fermer des panneaux",
            (Self::Usage, false) => "See my usage (plans, % used, reset times)",
            (Self::Usage, true) => "Voir mon utilisation (forfaits, % utilisé, réinitialisations)",
            (Self::Edit, false) => "Edit files (save, create, rename, move to the Recycle Bin)",
            (Self::Edit, true) => {
                "Modifier les fichiers (enregistrer, créer, renommer, mettre à la corbeille)"
            }
            (Self::Git, false) => "Stage, commit and discard changes (git)",
            (Self::Git, true) => "Indexer, faire un commit et abandonner des modifications (git)",
            (Self::Tasks, false) => {
                "Manage tasks (add, move, edit, answer questions, delete for good)"
            }
            (Self::Tasks, true) => {
                "Gérer les tâches (ajouter, déplacer, modifier, répondre aux questions, supprimer définitivement)"
            }
            (Self::PagesView, false) => "See my browser pages",
            (Self::PagesView, true) => "Voir mes pages de navigateur",
        }
    }

    pub const fn hint(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::Type, false) => {
                "Only for people you trust: typing in a terminal runs commands on this PC."
            }
            (Self::Type, true) => {
                "Seulement pour des personnes de confiance : taper dans un terminal lance des commandes sur ce PC."
            }
            (Self::Open, false) => {
                "Without it, each terminal or agent they ask for waits for your yes."
            }
            (Self::Open, true) => {
                "Sans ce droit, chaque terminal ou agent demandé attend votre accord."
            }
            (Self::Close, false) => {
                "An agent, or a terminal where something runs, still waits for your yes."
            }
            (Self::Close, true) => {
                "Un agent, ou un terminal où quelque chose tourne, attend toujours votre accord."
            }
            (Self::Usage, false) => {
                "Your plans, how much of each you used and when it resets. Never your accounts or emails."
            }
            (Self::Usage, true) => {
                "Vos forfaits, la part utilisée de chacun et quand ils se réinitialisent. Jamais vos comptes ni vos adresses."
            }
            (Self::Edit, false) => {
                "In the projects they see. Agent instructions (CLAUDE.md, AGENTS.md), git hooks and .gitignore stay read-only unless they can also type."
            }
            (Self::Edit, true) => {
                "Dans les projets que cette personne voit. Les instructions des agents (CLAUDE.md, AGENTS.md), les hooks git et .gitignore restent en lecture seule, sauf si elle peut aussi taper."
            }
            (Self::Git, false) => {
                "In the projects they see, with your git name. Their commits run without your git hooks; push, pull and fetch stay yours."
            }
            (Self::Git, true) => {
                "Dans les projets que cette personne voit, avec votre nom git. Ses commits se font sans vos hooks git ; push, pull et fetch restent à vous."
            }
            (Self::Tasks, false) => "The task board of the projects they see.",
            (Self::Tasks, true) => "Le tableau des tâches des projets que cette personne voit.",
            (Self::PagesView, false) => {
                "Guests see the address of your pages and open them on their side, never with your browser sessions."
            }
            (Self::PagesView, true) => {
                "Les invités voient l'adresse de vos pages et les ouvrent de leur côté, jamais avec vos sessions."
            }
        }
    }

    pub const fn part_phrase(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::Type, false) => "type in its terminals and agents",
            (Self::Type, true) => "taper dans ses terminaux et ses agents",
            (Self::Open, false) => "open terminals and agents",
            (Self::Open, true) => "ouvrir des terminaux et des agents",
            (Self::Close, false) => "ask to close panes",
            (Self::Close, true) => "demander à fermer des panneaux",
            (Self::Usage, false) => "see its usage",
            (Self::Usage, true) => "voir son utilisation",
            (Self::Edit, false) => "edit its files",
            (Self::Edit, true) => "modifier ses fichiers",
            (Self::Git, false) => "stage and commit changes",
            (Self::Git, true) => "indexer et faire des commits",
            (Self::Tasks, false) => "manage its tasks",
            (Self::Tasks, true) => "gérer ses tâches",
            (Self::PagesView, false) => "see its browser pages",
            (Self::PagesView, true) => "voir ses pages de navigateur",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareRights {
    #[serde(rename = "type", default)]
    pub r#type: bool,
    #[serde(default)]
    pub open: bool,
    #[serde(default)]
    pub close: bool,
    #[serde(default)]
    pub usage: bool,
    #[serde(default)]
    pub edit: bool,
    #[serde(default)]
    pub git: bool,
    #[serde(default)]
    pub tasks: bool,
    #[serde(rename = "pagesView", default)]
    pub pages_view: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RightsPreset {
    ViewOnly,
    Collaborate,
    FullControl,
    Custom,
}

impl RightsPreset {
    pub const ALL: [Self; 4] = [
        Self::ViewOnly,
        Self::Collaborate,
        Self::FullControl,
        Self::Custom,
    ];

    pub const fn label(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::ViewOnly, false) => "View only",
            (Self::ViewOnly, true) => "Regarder seulement",
            (Self::Collaborate, false) => "Collaborate",
            (Self::Collaborate, true) => "Collaborer",
            (Self::FullControl, false) => "Full control",
            (Self::FullControl, true) => "Contrôle total",
            (Self::Custom, false) => "Custom",
            (Self::Custom, true) => "Personnalisé",
        }
    }

    pub fn hint(self, rights: ShareRights, french: bool) -> String {
        match (self, french) {
            (Self::ViewOnly, false) => {
                "They watch the shared panes and files, nothing else.".into()
            }
            (Self::ViewOnly, true) => {
                "Cette personne regarde les panneaux et les fichiers partagés, rien d'autre.".into()
            }
            (Self::Collaborate, false) => {
                "They open terminals and agents, edit files, commit and manage tasks. They never type in your terminals.".into()
            }
            (Self::Collaborate, true) => {
                "Cette personne ouvre des terminaux et des agents, modifie des fichiers, fait des commits et gère les tâches. Elle ne tape jamais dans vos terminaux.".into()
            }
            (Self::FullControl, false) => {
                "Every right, typing included: they can run any command on this PC as you. Only for people you trust.".into()
            }
            (Self::FullControl, true) => {
                "Tous les droits, y compris taper : cette personne peut lancer n'importe quelle commande sur ce PC en votre nom. Seulement pour des personnes de confiance.".into()
            }
            (Self::Custom, false) => format!("Custom: {}", rights.short_summary(false)),
            (Self::Custom, true) => format!("Personnalisé : {}", rights.short_summary(true)),
        }
    }
}

impl ShareRights {
    pub const fn view_only() -> Self {
        Self {
            r#type: false,
            open: false,
            close: false,
            usage: false,
            edit: false,
            git: false,
            tasks: false,
            pages_view: false,
        }
    }

    pub const fn collaborate() -> Self {
        Self {
            r#type: false,
            open: true,
            close: true,
            usage: false,
            edit: true,
            git: true,
            tasks: true,
            pages_view: true,
        }
    }

    pub const fn full_control() -> Self {
        Self {
            r#type: true,
            open: true,
            close: true,
            usage: true,
            edit: true,
            git: true,
            tasks: true,
            pages_view: true,
        }
    }

    pub const fn from_preset(preset: RightsPreset) -> Self {
        match preset {
            RightsPreset::ViewOnly | RightsPreset::Custom => Self::view_only(),
            RightsPreset::Collaborate => Self::collaborate(),
            RightsPreset::FullControl => Self::full_control(),
        }
    }

    pub fn clean_from_value(v: Option<&Value>) -> Self {
        let Some(obj) = v.and_then(Value::as_object) else {
            return Self::view_only();
        };
        let is_true = |key: &str| obj.get(key) == Some(&Value::Bool(true));
        Self {
            r#type: is_true("type"),
            open: is_true("open"),
            close: is_true("close"),
            usage: is_true("usage"),
            edit: is_true("edit"),
            git: is_true("git"),
            tasks: is_true("tasks"),
            pages_view: is_true("pagesView"),
        }
    }

    pub const fn get(self, id: RightId) -> bool {
        match id {
            RightId::Type => self.r#type,
            RightId::Open => self.open,
            RightId::Close => self.close,
            RightId::Usage => self.usage,
            RightId::Edit => self.edit,
            RightId::Git => self.git,
            RightId::Tasks => self.tasks,
            RightId::PagesView => self.pages_view,
        }
    }

    pub fn set(&mut self, id: RightId, on: bool) {
        match id {
            RightId::Type => self.r#type = on,
            RightId::Open => self.open = on,
            RightId::Close => self.close = on,
            RightId::Usage => self.usage = on,
            RightId::Edit => self.edit = on,
            RightId::Git => self.git = on,
            RightId::Tasks => self.tasks = on,
            RightId::PagesView => self.pages_view = on,
        }
    }

    pub fn enabled_count(self) -> usize {
        RightId::ALL.into_iter().filter(|&r| self.get(r)).count()
    }

    pub fn preset(self) -> RightsPreset {
        if self == Self::view_only() {
            RightsPreset::ViewOnly
        } else if self == Self::collaborate() {
            RightsPreset::Collaborate
        } else if self == Self::full_control() {
            RightsPreset::FullControl
        } else {
            RightsPreset::Custom
        }
    }

    pub fn short_summary(self, french: bool) -> String {
        let active: Vec<&str> = RightId::ALL
            .into_iter()
            .filter(|&r| self.get(r))
            .map(|r| r.short_label(french))
            .collect();
        if active.is_empty() {
            if french {
                "Regarder seulement".into()
            } else {
                "Watch only".into()
            }
        } else {
            active.join(", ")
        }
    }

    pub fn chip_text(self, french: bool) -> String {
        match self.preset() {
            RightsPreset::Custom => {
                let n = self.enabled_count();
                if french {
                    format!("Personnalisé · {n}")
                } else {
                    format!("Custom · {n}")
                }
            }
            other => other.label(french).to_owned(),
        }
    }

    pub fn guest_sentence(self, french: bool) -> String {
        let parts: Vec<&str> = RightId::ALL
            .into_iter()
            .filter(|&r| self.get(r))
            .map(|r| r.part_phrase(french))
            .collect();
        if parts.is_empty() {
            if french {
                "Vous pouvez regarder. L'hôte ne vous a pas encore donné le droit de taper.".into()
            } else {
                "You can watch. The host has not given you the right to type yet.".into()
            }
        } else if french {
            format!("Vous pouvez regarder et {}.", parts.join(", "))
        } else {
            format!("You can watch and {}.", parts.join(", "))
        }
    }
}

pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name
        .trim()
        .split(|c: char| c.is_whitespace() || matches!(c, '.' | '_' | '-'))
        .filter(|w| !w.is_empty())
        .collect();
    if words.is_empty() {
        return "?".into();
    }
    let raw: String = if words.len() > 1 {
        let a = words[0].chars().next().unwrap_or('?');
        let b = words[1].chars().next().unwrap_or('?');
        format!("{a}{b}")
    } else {
        words[0].chars().take(2).collect()
    };
    raw.to_uppercase()
}

pub fn short_code(code: &str) -> String {
    let trimmed = code.trim();
    if trimmed.chars().count() <= 26 {
        return trimmed.to_owned();
    }
    let head: String = trimmed.chars().take(16).collect();
    let tail: String = trimmed
        .chars()
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{head}…{tail}")
}

/// Deterministic 6-digit Short Authentication String (`"123 456"`) derived from the pinned TLS
/// certificate and both peers' 32-byte handshake nonces.
pub fn derive_sas_code(
    cert_der: &[u8],
    client_nonce: &[u8; 32],
    server_nonce: &[u8; 32],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"tessel-live-share-sas-v1\0");
    hasher.update((cert_der.len() as u64).to_be_bytes());
    hasher.update(cert_der);
    hasher.update(client_nonce);
    hasher.update(server_nonce);
    let digest = hasher.finalize();
    let val = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) % 1_000_000;
    format!("{:03} {:03}", val / 1_000, val % 1_000)
}

// ---------------------------------------------------------------------------
// 2. Wire Framing & Invitation Format
// ---------------------------------------------------------------------------

struct DeadlineStream {
    stream: TcpStream,
    deadline: Instant,
    stop: Option<Arc<GuestStop>>,
}
impl DeadlineStream {
    fn new(stream: TcpStream, timeout: Duration) -> Self {
        Self {
            stream,
            deadline: Instant::now() + timeout,
            stop: None,
        }
    }
    fn reset_deadline(&mut self, timeout: Duration) {
        self.deadline = Instant::now() + timeout;
    }
    fn remaining(&self) -> std::io::Result<Duration> {
        if self.stop.as_ref().is_some_and(|stop| stop.is_stopped()) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionAborted,
                "Guest session cancelled",
            ));
        }
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Share request deadline reached",
            ))
        } else {
            Ok(if self.stop.is_some() {
                remaining.min(Duration::from_millis(100))
            } else {
                remaining
            })
        }
    }
    fn retry_slice(&self, error: &std::io::Error) -> bool {
        self.stop.is_some()
            && matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::Interrupted
            )
    }
}
impl Read for DeadlineStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            self.stream.set_read_timeout(Some(self.remaining()?))?;
            match self.stream.read(buf) {
                Err(error) if self.retry_slice(&error) => continue,
                result => return result,
            }
        }
    }
}
impl Write for DeadlineStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        loop {
            self.stream.set_write_timeout(Some(self.remaining()?))?;
            match self.stream.write(buf) {
                Err(error) if self.retry_slice(&error) => continue,
                result => return result,
            }
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.stream.flush()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invitation {
    version: u32,
    address: SocketAddr,
    certificate: String,
    token: String,
}

impl Invitation {
    pub fn encode(&self) -> Result<String> {
        Ok(format!(
            "tessel-rust-share:{}",
            B64.encode(serde_json::to_vec(self).map_err(|e| e.to_string())?)
        ))
    }

    pub fn decode(text: &str) -> Result<Self> {
        if text.len() > 8192 {
            return Err("Invitation is too long.".into());
        }
        let trimmed = text.trim();
        let raw = trimmed
            .strip_prefix("tessel-rust-share:")
            .ok_or("Not a Tessel Rust invitation")?;
        let bytes = B64.decode(raw).map_err(|_| "Invalid invitation encoding")?;
        let invitation: Self = serde_json::from_slice(&bytes).map_err(|_| "Invalid invitation")?;
        if (invitation.version != 1 && invitation.version != 2)
            || invitation.address.ip().is_unspecified()
            || invitation.address.port() == 0
            || B64
                .decode(&invitation.token)
                .map_err(|_| "Invalid invitation token")?
                .len()
                != 32
            || B64
                .decode(&invitation.certificate)
                .map_err(|_| "Invalid certificate")?
                .len()
                > 4096
        {
            return Err("Invalid invitation fields.".into());
        }
        Ok(invitation)
    }

    pub fn endpoint(&self) -> String {
        self.address.to_string()
    }
}

fn read_frame(reader: &mut impl Read, limit: usize) -> Result<Value> {
    let mut header = [0; 4];
    reader
        .read_exact(&mut header)
        .map_err(|_| "Connection ended or timed out")?;
    let size = u32::from_be_bytes(header) as usize;
    if size > limit {
        return Err("Message exceeds size limit.".into());
    }
    let mut bytes = vec![0; size];
    reader
        .read_exact(&mut bytes)
        .map_err(|_| "Incomplete message")?;
    serde_json::from_slice(&bytes).map_err(|_| "Invalid message".into())
}

fn write_frame(writer: &mut impl Write, value: &Value) -> Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_RESPONSE {
        return Err("Response exceeds size limit.".into());
    }
    writer
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .and_then(|_| writer.write_all(&bytes))
        .and_then(|_| writer.flush())
        .map_err(|error| format!("Connection write failed: {error}"))
}

// ---------------------------------------------------------------------------
// 3. Path & File Safety Checks (`private_path`, `secret_or_ignored_path`, `agent_instruction_path`)
// ---------------------------------------------------------------------------

pub fn private_path(path: &Path) -> bool {
    path.components().any(|part| {
        let name = part.as_os_str().to_string_lossy().to_lowercase();
        name == ".git"
            || name == ".tessel"
            || matches!(
                name.as_str(),
                "tesselrust"
                    | "appdata"
                    | "application support"
                    | "browser-owner"
                    | "login data"
                    | "cookies"
                    | "web data"
                    | "vault.dat"
                    | "workspace.json"
                    | "id_rsa"
                    | "id_ed25519"
            )
            || [".pem", ".key", ".pfx", ".p12", ".kdbx"]
                .iter()
                .any(|suffix| name.ends_with(suffix))
    })
}

pub fn secret_or_dot_path(path: &Path) -> bool {
    path.components().any(|part| {
        let name = part.as_os_str().to_string_lossy().to_lowercase();
        name.starts_with('.')
            || name.starts_with("secret")
            || name.contains("credentials")
            || name.ends_with(".env")
            || name.contains(".env.")
            || [".p8", ".ppk", ".jks", ".keystore", ".tfvars"]
                .iter()
                .any(|suffix| name.ends_with(suffix))
            || name.contains(".tfstate")
            || ["id_rsa", "id_ed25519", "id_ecdsa", "id_dsa"]
                .iter()
                .any(|prefix| name.starts_with(prefix))
            || (name.starts_with("service-account") && name.ends_with(".json"))
    })
}

/// Files that steer agents or git hooks (`CLAUDE.md`, `AGENTS.md`, `.gitignore`, `.cursorrules`, etc.).
/// Editing these requires `rights.r#type` in addition to `rights.edit`, and flags the guest action!
pub fn is_agent_instruction_or_protected_file(relative: &Path) -> bool {
    let normalized = relative.to_string_lossy().replace('\\', "/");
    protected_paths::is_protected(&normalized, &[])
        || relative.components().any(|part| {
            // Preserve the existing conservative hook-name guard until per-project
            // core.hooksPath discovery supplies the complete dynamic policy.
            matches!(
                part.as_os_str()
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .as_str(),
                "pre-commit"
                    | "prepare-commit-msg"
                    | "commit-msg"
                    | "post-commit"
                    | "post-checkout"
                    | "post-index-change"
                    | "reference-transaction"
                    | "pre-auto-gc"
            )
        })
}

#[derive(Clone)]
struct DirEntryInfo {
    name: String,
    path: PathBuf,
    directory: bool,
}

fn confined_path(root: &Path, path: &Path) -> Result<PathBuf> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    if !path.starts_with(&root) {
        return Err("Path is outside this project (including symlinks).".into());
    }
    Ok(path)
}

fn list_dir_entries(root: &Path, path: &Path) -> Result<Vec<DirEntryInfo>> {
    let path = confined_path(root, path)?;
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entries.len() == 10_000 {
            return Err("Directory exceeds 10,000 entries.".into());
        }
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink() {
            continue;
        }
        entries.push(DirEntryInfo {
            name: entry.file_name().to_string_lossy().into_owned(),
            path: entry.path(),
            directory: kind.is_dir(),
        });
    }
    entries.sort_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

fn share_path_checked(root: &Path, relative: &str, hide_ignored: bool) -> Result<PathBuf> {
    let rel = Path::new(relative);
    if relative.trim().is_empty()
        || rel.is_absolute()
        || rel
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err("Only relative project paths are accepted.".into());
    }
    if private_path(rel) || (hide_ignored && secret_or_dot_path(rel)) {
        return Err("Private path is not shared.".into());
    }
    let resolved = confined_path(root, &root.join(rel))?;
    let stripped = resolved.strip_prefix(root).unwrap_or(&resolved);
    if private_path(stripped) || (hide_ignored && secret_or_dot_path(stripped)) {
        return Err("Private path is not shared.".into());
    }
    Ok(resolved)
}

fn share_new_path_checked(root: &Path, relative: &str, hide_ignored: bool) -> Result<PathBuf> {
    let rel = Path::new(relative);
    if relative.trim().is_empty()
        || rel.is_absolute()
        || rel
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err("Only relative project paths are accepted.".into());
    }
    if private_path(rel) || (hide_ignored && secret_or_dot_path(rel)) {
        return Err("Private path is not shared.".into());
    }
    let parent_rel = rel.parent().unwrap_or_else(|| Path::new(""));
    let parent_abs = if parent_rel.as_os_str().is_empty() {
        root.to_path_buf()
    } else {
        confined_path(root, &root.join(parent_rel))?
    };
    let file_name = rel.file_name().ok_or("Invalid file name.")?;
    Ok(parent_abs.join(file_name))
}

// ---------------------------------------------------------------------------
// 4. Invitations, People, Knock Asks, Trusted Peers & Presence Data Structures
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InviteLifetime {
    OneHour,
    OneDay,
    SevenDays,
    NoExpiry,
}

impl InviteLifetime {
    pub const ALL: [Self; 4] = [Self::OneHour, Self::OneDay, Self::SevenDays, Self::NoExpiry];

    pub const fn label(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::OneHour, false) => "1 hour",
            (Self::OneHour, true) => "1 heure",
            (Self::OneDay, false) => "1 day",
            (Self::OneDay, true) => "1 jour",
            (Self::SevenDays, false) => "7 days",
            (Self::SevenDays, true) => "7 jours",
            (Self::NoExpiry, false) => "No expiry",
            (Self::NoExpiry, true) => "Sans expiration",
        }
    }

    pub fn expires_at_from(self, now_ms: u64) -> Option<u64> {
        match self {
            Self::OneHour => Some(now_ms.saturating_add(3_600_000)),
            Self::OneDay => Some(now_ms.saturating_add(86_400_000)),
            Self::SevenDays => Some(now_ms.saturating_add(7 * 86_400_000)),
            Self::NoExpiry => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteStatus {
    Live,
    UsedUp,
    Blocked,
    Expired,
}

impl InviteStatus {
    pub const fn badge_label(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::Live, false) => "Live",
            (Self::Live, true) => "Active",
            (Self::UsedUp, false) => "Used up",
            (Self::UsedUp, true) => "Épuisée",
            (Self::Blocked, false) => "Blocked",
            (Self::Blocked, true) => "Bloquée",
            (Self::Expired, false) => "Expired",
            (Self::Expired, true) => "Expirée",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ShareInvite {
    pub id: String,
    pub label: String,
    pub root: bool,
    pub invitation: Invitation,
    pub code: String,
    pub(crate) token_bytes: [u8; 32],
    pub projects: Vec<u64>,
    pub include_later: bool,
    pub rights: ShareRights,
    pub lifetime: InviteLifetime,
    pub expires_at_ms: Option<u64>,
    pub max_uses: u32,
    pub uses: u32,
    pub joined: u32,
    pub failed_knocks: u32,
    pub blocked: bool,
}

impl ShareInvite {
    pub fn display_name(&self, french: bool) -> String {
        let trimmed = self.label.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
        if self.root {
            if french {
                "Première invitation".into()
            } else {
                "First invitation".into()
            }
        } else {
            "Invitation".into()
        }
    }

    pub fn status(&self, now_ms: u64) -> InviteStatus {
        if self.blocked {
            InviteStatus::Blocked
        } else if self.max_uses > 0 && self.uses >= self.max_uses {
            InviteStatus::UsedUp
        } else if self.expires_at_ms.is_some_and(|exp| exp <= now_ms) {
            InviteStatus::Expired
        } else {
            InviteStatus::Live
        }
    }

    pub fn expiry_summary(&self, now_ms: u64, french: bool) -> String {
        match self.expires_at_ms {
            None => InviteLifetime::NoExpiry.label(french).to_owned(),
            Some(exp) if exp <= now_ms => InviteStatus::Expired.badge_label(french).to_owned(),
            Some(exp) => {
                let mins = ((exp - now_ms) / 60_000).max(1);
                let left = if mins >= 60 * 24 {
                    let days = mins / (60 * 24);
                    if french {
                        format!("{days} j")
                    } else {
                        format!("{days}d")
                    }
                } else if mins >= 60 {
                    let hrs = mins / 60;
                    format!("{hrs}h")
                } else {
                    format!("{mins} min")
                };
                let when = clock_hhmm(exp);
                if french {
                    format!("Expire dans {left} ({when})")
                } else {
                    format!("Expires in {left} ({when})")
                }
            }
        }
    }

    pub fn uses_summary(&self, french: bool) -> String {
        if self.max_uses > 0 {
            if french {
                format!("{}/{} utilisations", self.uses, self.max_uses)
            } else {
                format!("{}/{} used", self.uses, self.max_uses)
            }
        } else if french {
            if self.uses == 1 {
                "1 utilisation".into()
            } else {
                format!("{} utilisations", self.uses)
            }
        } else {
            format!("{} used", self.uses)
        }
    }
}

#[derive(Debug, Clone)]
pub struct InviteSpec {
    pub label: String,
    pub projects: Vec<u64>,
    pub include_later: bool,
    pub rights: ShareRights,
    pub lifetime: InviteLifetime,
    pub max_uses: u32,
}

impl Default for InviteSpec {
    fn default() -> Self {
        Self {
            label: String::new(),
            projects: Vec::new(),
            include_later: true,
            rights: ShareRights::view_only(),
            lifetime: InviteLifetime::OneDay,
            max_uses: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestConnectionState {
    Waiting,
    Joined,
}

#[derive(Debug, Clone)]
pub struct ConnectedGuest {
    pub id: String,
    pub name: String,
    pub state: GuestConnectionState,
    pub code: String,
    pub invite_id: Option<String>,
    pub invite_label: String,
    pub projects: Vec<u64>,
    pub rights: ShareRights,
    pub trusted: bool,
    pub trust_id: Option<String>,
    pub identity_hex: Option<String>,
    pub since_ms: u64,
    pub watching_pane: Option<u64>,
    pub typing_pane: Option<(u64, u64)>, // (pane_id, updated_at_ms)
}

impl ConnectedGuest {
    pub fn is_typing_in(&self, pane_id: u64, now_ms: u64) -> bool {
        self.typing_pane
            .is_some_and(|(p, at)| p == pane_id && now_ms.saturating_sub(at) <= TYPING_TTL_MS)
    }

    pub fn active_typing_pane(&self, now_ms: u64) -> Option<u64> {
        self.typing_pane
            .and_then(|(p, at)| (now_ms.saturating_sub(at) <= TYPING_TTL_MS).then_some(p))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustedPerson {
    pub id: String,
    pub identity_hex: String,
    pub name: String,
    pub projects: Vec<u64>,
    pub rights: ShareRights,
    pub since_ms: u64,
    pub last_seen_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct OfflinePerson {
    pub id: String,
    pub name: String,
    pub invite_label: String,
    pub projects: Vec<u64>,
    pub rights: ShareRights,
    pub trust_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneGuestPresence {
    pub guest_id: String,
    pub name: String,
    pub initials: String,
    pub watching: bool,
    pub typing: bool,
}

#[derive(Debug, Clone)]
pub struct RecentGuestAction {
    pub id: u64,
    pub at_ms: u64,
    pub guest_name: String,
    pub project_id: u64,
    pub summary_en: String,
    pub summary_fr: String,
    pub flagged: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveShareAsk {
    Join {
        key: String,
        guest_id: String,
        name: String,
        code: String,
        invite_label: String,
        offered_projects: Vec<u64>,
        offered_rights: ShareRights,
        trustable: bool,
    },
    Open {
        key: String,
        guest_id: String,
        name: String,
        project_id: u64,
        project_name: String,
        agent_name: Option<String>,
    },
    Close {
        key: String,
        guest_id: String,
        name: String,
        pane_id: u64,
        pane_name: String,
        detail: String,
    },
}

impl LiveShareAsk {
    pub fn key(&self) -> &str {
        match self {
            Self::Join { key, .. } | Self::Open { key, .. } | Self::Close { key, .. } => key,
        }
    }
}

#[derive(Debug, Clone)]
pub enum AskAnswer {
    Join {
        key: String,
        allow: bool,
        projects: Vec<u64>,
        rights: ShareRights,
        trust: bool,
    },
    Open {
        key: String,
        allow: bool,
        always: bool,
    },
    Close {
        key: String,
        allow: bool,
    },
}

// ---------------------------------------------------------------------------
// 5. Shared Workspace Snapshots & Host Inbound Commands
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedProject {
    pub id: u64,
    pub name: String,
    #[serde(skip)]
    pub path: PathBuf,
}

#[path = "share/guest_text.rs"]
pub(crate) mod guest_text;

#[path = "share/task_board.rs"]
mod task_board;
pub use task_board::{GuestTaskCommand, GuestTaskOp, SharedTask};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedPaneSnapshot {
    pub id: u64,
    pub project_id: u64,
    pub title: String,
    pub is_agent: bool,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedBrowserPage {
    pub project_id: Option<u64>,
    pub title: String,
    pub url: String,
}

#[derive(Debug, Clone)]
pub enum HostCommand {
    TaskMutation(GuestTaskCommand),
    WriteTerminal {
        pane_id: u64,
        bytes: Vec<u8>,
    },
    ResizeTerminal {
        pane_id: u64,
        rows: u16,
        cols: u16,
    },
    OpenPane {
        project_id: u64,
        agent_name: Option<String>,
    },
    ClosePane {
        pane_id: u64,
    },
}

#[derive(Debug, Clone)]
pub struct ShareSettings {
    pub hide_ignored: bool,
    pub rendezvous: bool,
}

impl Default for ShareSettings {
    fn default() -> Self {
        Self {
            hide_ignored: true,
            rendezvous: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HostSnapshot {
    pub host_name: String,
    pub endpoint: String,
    pub since_ms: u64,
    pub settings: ShareSettings,
    pub shared_projects: Vec<SharedProject>,
    pub invites: Vec<ShareInvite>,
    pub guests: Vec<ConnectedGuest>,
    pub offline: Vec<OfflinePerson>,
    pub trusted: Vec<TrustedPerson>,
    pub asks: Vec<LiveShareAsk>,
    pub recent_actions: Vec<RecentGuestAction>,
}

// ---------------------------------------------------------------------------
// 6. Host Implementation (One-Shot Legacy + Persistent Duplex RPC)
// ---------------------------------------------------------------------------

enum GuestDecision {
    Pending,
    Approved,
    Denied,
    Removed,
}

struct GuestSlot {
    guest: ConnectedGuest,
    decision: GuestDecision,
}

struct HostInner {
    stopped: Arc<AtomicBool>,
    task_board: task_board::TaskBoard,
    host_name: String,
    address: SocketAddr,
    cert_b64: String,
    cert_der: Vec<u8>,
    since_ms: u64,
    settings: ShareSettings,
    shared_projects: Vec<SharedProject>,
    panes: Vec<SharedPaneSnapshot>,
    pages: Vec<SharedBrowserPage>,
    usage_summary: String,
    host_active_project: Option<u64>,
    host_active_pane: Option<u64>,
    invites: Vec<ShareInvite>,
    next_invite_seq: u64,
    guests: Vec<GuestSlot>,
    next_guest_seq: u64,
    offline: Vec<OfflinePerson>,
    trusted: Vec<TrustedPerson>,
    asks: VecDeque<LiveShareAsk>,
    next_ask_seq: u64,
    recent_actions: VecDeque<RecentGuestAction>,
    next_action_seq: u64,
    commands: VecDeque<HostCommand>,
}

impl HostInner {
    fn make_invite(&mut self, spec: InviteSpec, root: bool) -> Result<ShareInvite> {
        if self.invites.len() >= MAX_INVITES {
            return Err(format!(
                "You can have up to {MAX_INVITES} invitations at once. Revoke one first."
            ));
        }
        let mut token = [0u8; 32];
        getrandom::fill(&mut token).map_err(|e| e.to_string())?;
        let id = format!("iv-{}", self.next_invite_seq);
        self.next_invite_seq += 1;
        let invitation = Invitation {
            version: 1,
            address: self.address,
            certificate: self.cert_b64.clone(),
            token: B64.encode(token),
        };
        let code = invitation.encode()?;
        let now_ms = now_epoch_ms();
        let projects = if spec.projects.is_empty() {
            self.shared_projects.iter().map(|p| p.id).collect()
        } else {
            spec.projects
        };
        let inv = ShareInvite {
            id,
            label: spec.label.trim().chars().take(48).collect(),
            root,
            invitation,
            code,
            token_bytes: token,
            projects,
            include_later: spec.include_later,
            rights: spec.rights,
            lifetime: spec.lifetime,
            expires_at_ms: spec.lifetime.expires_at_from(now_ms),
            max_uses: spec.max_uses.min(1000),
            uses: 0,
            joined: 0,
            failed_knocks: 0,
            blocked: false,
        };
        self.invites.push(inv.clone());
        Ok(inv)
    }

    fn match_valid_invite_mut(&mut self, supplied: &[u8]) -> Option<usize> {
        if supplied.len() != 32 {
            return None;
        }
        let now_ms = now_epoch_ms();
        for (idx, inv) in self.invites.iter().enumerate() {
            if bool::from(supplied.ct_eq(&inv.token_bytes))
                && inv.status(now_ms) == InviteStatus::Live
            {
                return Some(idx);
            }
        }
        None
    }

    fn record_action(
        &mut self,
        guest_name: &str,
        project_id: u64,
        summary_en: String,
        summary_fr: String,
        flagged: bool,
    ) {
        let id = self.next_action_seq;
        self.next_action_seq += 1;
        if self.recent_actions.len() >= 50 {
            self.recent_actions.pop_back();
        }
        self.recent_actions.push_front(RecentGuestAction {
            id,
            at_ms: now_epoch_ms(),
            guest_name: guest_name.to_owned(),
            project_id,
            summary_en,
            summary_fr,
            flagged,
        });
    }

    fn find_shared_project_for_guest(
        &self,
        guest_idx: usize,
        project_id: Option<u64>,
    ) -> Result<SharedProject> {
        let allowed = &self.guests[guest_idx].guest.projects;
        let target_id = project_id
            .or_else(|| allowed.first().copied())
            .ok_or("No project is shared with you.")?;
        if !allowed.contains(&target_id) {
            return Err("You no longer have access to this project.".into());
        }
        self.shared_projects
            .iter()
            .find(|p| p.id == target_id)
            .cloned()
            .ok_or_else(|| "Shared project is no longer open.".into())
    }
}

pub struct Host {
    pub invitation: Invitation,
    stop: Arc<AtomicBool>,
    inner: Arc<Mutex<HostInner>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Host {
    /// Backwards-compatible single-folder starter (used by legacy fixtures and tests).
    pub fn start(root: &Path, bind: SocketAddr, advertise: IpAddr) -> Result<Self> {
        let canonical = root.canonicalize().map_err(|e| e.to_string())?;
        let name = canonical
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Project".into());
        Self::start_multi(
            vec![SharedProject {
                id: 1,
                name,
                path: canonical,
            }],
            "Host".into(),
            InviteSpec::default(),
            ShareSettings::default(),
            Vec::new(),
            bind,
            advertise,
        )
    }

    pub fn start_multi(
        projects: Vec<SharedProject>,
        host_name: String,
        first_invite: InviteSpec,
        settings: ShareSettings,
        trusted: Vec<TrustedPerson>,
        bind: SocketAddr,
        advertise: IpAddr,
    ) -> Result<Self> {
        if projects.is_empty() {
            return Err("Tick at least one project.".into());
        }
        let mut canonical_projects = Vec::with_capacity(projects.len());
        for mut proj in projects {
            let canon = proj.path.canonicalize().map_err(|e| e.to_string())?;
            if !canon.is_dir() || private_path(&canon) {
                return Err("This private directory cannot be shared.".into());
            }
            proj.path = canon;
            canonical_projects.push(proj);
        }

        let _ = rustls::crypto::ring::default_provider().install_default();
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec!["tessel-rust".into()])
                .map_err(|e| e.to_string())?;
        let certificate = cert.der().clone();
        let cert_der_vec = certificate.as_ref().to_vec();
        let cert_b64 = B64.encode(&cert_der_vec);
        let key = rustls::pki_types::PrivatePkcs8KeyDer::from(signing_key.serialize_der());
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![certificate], key.into())
            .map_err(|e| e.to_string())?;
        let config = Arc::new(config);

        let listener = TcpListener::bind(bind).map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let address = SocketAddr::new(
            advertise,
            listener.local_addr().map_err(|e| e.to_string())?.port(),
        );

        let stop = Arc::new(AtomicBool::new(false));
        let mut inner_data = HostInner {
            stopped: Arc::clone(&stop),
            task_board: task_board::TaskBoard::default(),
            host_name: if host_name.trim().is_empty() {
                "Host".into()
            } else {
                host_name.trim().chars().take(40).collect()
            },
            address,
            cert_b64,
            cert_der: cert_der_vec,
            since_ms: now_epoch_ms(),
            settings,
            shared_projects: canonical_projects,
            panes: Vec::new(),
            pages: Vec::new(),
            usage_summary: String::new(),
            host_active_project: None,
            host_active_pane: None,
            invites: Vec::new(),
            next_invite_seq: 1,
            guests: Vec::new(),
            next_guest_seq: 1,
            offline: Vec::new(),
            trusted,
            asks: VecDeque::new(),
            next_ask_seq: 1,
            recent_actions: VecDeque::new(),
            next_action_seq: 1,
            commands: VecDeque::new(),
        };
        let root_inv = inner_data.make_invite(first_invite, true)?;
        let invitation = root_inv.invitation.clone();
        let inner = Arc::new(Mutex::new(inner_data));

        let stopping = stop.clone();
        let clients = Arc::new(AtomicUsize::new(0));
        let inner_worker = Arc::clone(&inner);

        let worker = thread::spawn(move || {
            let mut last_window = Instant::now();
            let mut connections = 0usize;
            while !stopping.load(Ordering::Relaxed) {
                let (stream, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(20));
                        continue;
                    }
                    Err(_) => break,
                };
                if last_window.elapsed() > Duration::from_secs(60) {
                    last_window = Instant::now();
                    connections = 0;
                }
                connections += 1;
                if connections > 240 || clients.load(Ordering::Relaxed) >= 8 {
                    continue;
                }
                clients.fetch_add(1, Ordering::Relaxed);
                let clients = clients.clone();
                let config = config.clone();
                let stop = stopping.clone();
                let inner_conn = Arc::clone(&inner_worker);
                thread::spawn(move || {
                    serve_connection(stream, config, stop, inner_conn);
                    clients.fetch_sub(1, Ordering::Relaxed);
                });
            }
        });

        Ok(Self {
            invitation,
            stop,
            inner,
            thread: Some(worker),
        })
    }

    pub fn snapshot(&self) -> HostSnapshot {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        HostSnapshot {
            host_name: guard.host_name.clone(),
            endpoint: guard.address.to_string(),
            since_ms: guard.since_ms,
            settings: guard.settings.clone(),
            shared_projects: guard.shared_projects.clone(),
            invites: guard.invites.clone(),
            guests: guard.guests.iter().map(|s| s.guest.clone()).collect(),
            offline: guard.offline.clone(),
            trusted: guard.trusted.clone(),
            asks: guard.asks.iter().cloned().collect(),
            recent_actions: guard.recent_actions.iter().cloned().collect(),
        }
    }

    pub fn sync_workspace_state(
        &self,
        shared_projects: Vec<SharedProject>,
        panes: Vec<SharedPaneSnapshot>,
        pages: Vec<SharedBrowserPage>,
        active_project: Option<u64>,
        active_pane: Option<u64>,
    ) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let prev_ids: Vec<u64> = guard.shared_projects.iter().map(|p| p.id).collect();
        let mut canon_projects = Vec::with_capacity(shared_projects.len());
        for mut proj in shared_projects {
            if let Ok(c) = proj.path.canonicalize()
                && c.is_dir()
                && !private_path(&c)
            {
                proj.path = c;
                canon_projects.push(proj);
            }
        }
        {
            let new_ids: Vec<u64> = canon_projects.iter().map(|p| p.id).collect();
            for inv in &mut guard.invites {
                if inv.include_later {
                    for &nid in &new_ids {
                        if !prev_ids.contains(&nid) && !inv.projects.contains(&nid) {
                            inv.projects.push(nid);
                        }
                    }
                }
                inv.projects.retain(|id| new_ids.contains(id));
            }
            for slot in &mut guard.guests {
                slot.guest.projects.retain(|id| new_ids.contains(id));
            }
            if !task_board::same_roots(&guard.shared_projects, &canon_projects) {
                guard.task_board.invalidate();
            }
            guard.shared_projects = canon_projects;
        }
        guard.panes = panes;
        guard.pages = pages;
        guard.host_active_project = active_project;
        guard.host_active_pane = active_pane;
    }

    pub fn drain_commands(&self) -> Vec<HostCommand> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.commands.drain(..).collect()
    }

    pub fn pane_presence(&self, pane_id: u64) -> Vec<PaneGuestPresence> {
        let now_ms = now_epoch_ms();
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .guests
            .iter()
            .filter(|s| s.guest.state == GuestConnectionState::Joined)
            .filter_map(|s| {
                let typing = s.guest.is_typing_in(pane_id, now_ms);
                let watching = s.guest.watching_pane == Some(pane_id) || typing;
                if !watching && !typing {
                    return None;
                }
                Some(PaneGuestPresence {
                    guest_id: s.guest.id.clone(),
                    name: s.guest.name.clone(),
                    initials: initials(&s.guest.name),
                    watching,
                    typing,
                })
            })
            .collect()
    }

    pub fn set_hide_ignored(&self, hide_ignored: bool) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.settings.hide_ignored = hide_ignored;
    }

    pub fn create_invite(&self, spec: InviteSpec) -> Result<ShareInvite> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.make_invite(spec, false)
    }

    pub fn update_invite(&self, id: &str, spec: InviteSpec) -> Result<()> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let now_ms = now_epoch_ms();
        let inv = guard
            .invites
            .iter_mut()
            .find(|i| i.id == id)
            .ok_or("Invitation no longer exists.")?;
        inv.label = spec.label.trim().chars().take(48).collect();
        inv.projects = spec.projects;
        inv.include_later = spec.include_later;
        inv.rights = spec.rights;
        inv.lifetime = spec.lifetime;
        inv.expires_at_ms = spec.lifetime.expires_at_from(now_ms);
        inv.max_uses = spec.max_uses.min(1000);
        Ok(())
    }

    pub fn duplicate_invite(&self, id: &str) -> Result<ShareInvite> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let existing = guard
            .invites
            .iter()
            .find(|i| i.id == id)
            .cloned()
            .ok_or("Invitation no longer exists.")?;
        let spec = InviteSpec {
            label: if existing.label.is_empty() {
                "Copy".into()
            } else {
                format!("{} (copy)", existing.label)
            },
            projects: existing.projects,
            include_later: existing.include_later,
            rights: existing.rights,
            lifetime: existing.lifetime,
            max_uses: existing.max_uses,
        };
        guard.make_invite(spec, false)
    }

    pub fn rotate_invite(&self, id: &str) -> Result<ShareInvite> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut token = [0u8; 32];
        getrandom::fill(&mut token).map_err(|e| e.to_string())?;
        let addr = guard.address;
        let cert_b64 = guard.cert_b64.clone();
        let inv = guard
            .invites
            .iter_mut()
            .find(|i| i.id == id)
            .ok_or("Invitation no longer exists.")?;
        let invitation = Invitation {
            version: 1,
            address: addr,
            certificate: cert_b64,
            token: B64.encode(token),
        };
        inv.code = invitation.encode()?;
        inv.invitation = invitation;
        inv.token_bytes = token;
        inv.failed_knocks = 0;
        inv.blocked = false;
        Ok(inv.clone())
    }

    pub fn revoke_invite(&self, id: &str, remove_people: bool) -> Result<usize> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let before = guard.invites.len();
        guard.invites.retain(|i| i.id != id);
        if guard.invites.len() == before {
            return Err("Invitation no longer exists.".into());
        }
        let mut removed = 0usize;
        if remove_people {
            let mut removed_ids = Vec::new();
            for slot in &mut guard.guests {
                if slot.guest.invite_id.as_deref() == Some(id) {
                    slot.decision = GuestDecision::Removed;
                    removed_ids.push(slot.guest.id.clone());
                    removed += 1;
                }
            }
            guard.asks.retain(|ask| {
                !matches!(
                    ask,
                    LiveShareAsk::Join { guest_id, .. } if removed_ids.iter().any(|rid| rid == guest_id)
                )
            });
        }
        Ok(removed)
    }

    pub fn answer_ask(&self, answer: AskAnswer) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let key = match &answer {
            AskAnswer::Join { key, .. }
            | AskAnswer::Open { key, .. }
            | AskAnswer::Close { key, .. } => key.clone(),
        };
        let Some(pos) = guard.asks.iter().position(|a| a.key() == key) else {
            return;
        };
        let ask = guard.asks.remove(pos).expect("ask exists");
        match (ask, answer) {
            (
                LiveShareAsk::Join { guest_id, .. },
                AskAnswer::Join {
                    allow,
                    projects,
                    rights,
                    trust,
                    ..
                },
            ) => {
                let mut inv_to_bump: Option<(String, bool)> = None;
                let mut trust_record: Option<(String, String, Vec<u64>, ShareRights)> = None;
                if let Some(slot) = guard.guests.iter_mut().find(|s| s.guest.id == guest_id) {
                    inv_to_bump = slot.guest.invite_id.clone().map(|id| (id, allow));
                    if allow && !projects.is_empty() {
                        slot.guest.state = GuestConnectionState::Joined;
                        slot.guest.projects = projects.clone();
                        slot.guest.rights = rights;
                        if trust && let Some(id_hex) = slot.guest.identity_hex.clone() {
                            trust_record =
                                Some((id_hex, slot.guest.name.clone(), projects.clone(), rights));
                        }
                        slot.decision = GuestDecision::Approved;
                    } else {
                        slot.decision = GuestDecision::Denied;
                    }
                }
                if let Some((inv_id, allowed)) = inv_to_bump
                    && let Some(inv) = guard.invites.iter_mut().find(|i| i.id == inv_id)
                {
                    if allowed {
                        inv.uses += 1;
                        inv.joined += 1;
                        inv.failed_knocks = 0;
                    } else {
                        inv.failed_knocks += 1;
                        if inv.failed_knocks >= 5 {
                            inv.blocked = true;
                        }
                    }
                }
                if let Some((id_hex, name, projs, rts)) = trust_record
                    && guard.trusted.len() < MAX_TRUSTED
                {
                    let tid = if let Some(existing) =
                        guard.trusted.iter_mut().find(|t| t.identity_hex == id_hex)
                    {
                        existing.name = name;
                        existing.projects = projs;
                        existing.rights = rts;
                        existing.last_seen_ms = Some(now_epoch_ms());
                        existing.id.clone()
                    } else {
                        let tid = format!("t-{}", guard.trusted.len() + 1);
                        guard.trusted.push(TrustedPerson {
                            id: tid.clone(),
                            identity_hex: id_hex,
                            name,
                            projects: projs,
                            rights: rts,
                            since_ms: now_epoch_ms(),
                            last_seen_ms: Some(now_epoch_ms()),
                        });
                        tid
                    };
                    if let Some(slot) = guard.guests.iter_mut().find(|s| s.guest.id == guest_id) {
                        slot.guest.trusted = true;
                        slot.guest.trust_id = Some(tid);
                    }
                }
            }
            (
                LiveShareAsk::Open {
                    guest_id,
                    project_id,
                    agent_name,
                    ..
                },
                AskAnswer::Open { allow, always, .. },
            ) => {
                if allow {
                    if always {
                        let mut tid_update = None;
                        if let Some(slot) = guard.guests.iter_mut().find(|s| s.guest.id == guest_id)
                        {
                            slot.guest.rights.open = true;
                            tid_update = slot.guest.trust_id.clone();
                        }
                        if let Some(tid) = tid_update
                            && let Some(tp) = guard.trusted.iter_mut().find(|t| t.id == tid)
                        {
                            tp.rights.open = true;
                        }
                    }
                    guard.commands.push_back(HostCommand::OpenPane {
                        project_id,
                        agent_name,
                    });
                }
            }
            (LiveShareAsk::Close { pane_id, .. }, AskAnswer::Close { allow: true, .. }) => {
                guard.commands.push_back(HostCommand::ClosePane { pane_id });
            }
            _ => {}
        }
    }

    pub fn set_subject_rights(&self, subject_id: &str, rights: ShareRights) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut linked_tid = None;
        if let Some(slot) = guard.guests.iter_mut().find(|s| s.guest.id == subject_id) {
            slot.guest.rights = rights;
            linked_tid = slot.guest.trust_id.clone();
        }
        if let Some(tp) = guard
            .trusted
            .iter_mut()
            .find(|t| t.id == subject_id || linked_tid.as_deref() == Some(t.id.as_str()))
        {
            tp.rights = rights;
        }
        if let Some(off) = guard.offline.iter_mut().find(|o| o.id == subject_id) {
            off.rights = rights;
        }
    }

    pub fn set_subject_project(&self, subject_id: &str, project_id: u64, on: bool) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut linked_tid = None;
        if let Some(slot) = guard.guests.iter_mut().find(|s| s.guest.id == subject_id) {
            if on && !slot.guest.projects.contains(&project_id) {
                slot.guest.projects.push(project_id);
            } else if !on && slot.guest.projects.len() > 1 {
                slot.guest.projects.retain(|&p| p != project_id);
            }
            linked_tid = slot.guest.trust_id.clone();
        }
        if let Some(tp) = guard
            .trusted
            .iter_mut()
            .find(|t| t.id == subject_id || linked_tid.as_deref() == Some(t.id.as_str()))
        {
            if on && !tp.projects.contains(&project_id) && tp.projects.len() < 64 {
                tp.projects.push(project_id);
            } else if !on {
                tp.projects.retain(|&p| p != project_id);
            }
        }
    }

    pub fn trust_guest(&self, guest_id: &str) -> Result<()> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.trusted.len() >= MAX_TRUSTED {
            return Err("Trusted people list is full.".into());
        }
        let Some(slot) = guard.guests.iter().find(|s| s.guest.id == guest_id) else {
            return Err("Guest is no longer connected.".into());
        };
        let Some(id_hex) = slot.guest.identity_hex.clone() else {
            return Err("Guest does not support trusted identity.".into());
        };
        let name = slot.guest.name.clone();
        let projects = slot.guest.projects.clone();
        let rights = slot.guest.rights;
        let tid = format!("t-{}", guard.trusted.len() + 1);
        guard.trusted.push(TrustedPerson {
            id: tid.clone(),
            identity_hex: id_hex,
            name,
            projects,
            rights,
            since_ms: now_epoch_ms(),
            last_seen_ms: Some(now_epoch_ms()),
        });
        if let Some(slot) = guard.guests.iter_mut().find(|s| s.guest.id == guest_id) {
            slot.guest.trusted = true;
            slot.guest.trust_id = Some(tid);
        }
        Ok(())
    }

    pub fn untrust(&self, trust_id: &str) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.trusted.retain(|t| t.id != trust_id);
        for slot in &mut guard.guests {
            if slot.guest.trust_id.as_deref() == Some(trust_id) {
                slot.guest.trusted = false;
                slot.guest.trust_id = None;
                slot.decision = GuestDecision::Removed;
            }
        }
    }

    pub fn rename_trusted(&self, trust_id: &str, new_name: &str) {
        let trimmed: String = new_name.trim().chars().take(40).collect();
        if trimmed.is_empty() {
            return;
        }
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(tp) = guard.trusted.iter_mut().find(|t| t.id == trust_id) {
            tp.name = trimmed.clone();
        }
        for slot in &mut guard.guests {
            if slot.guest.trust_id.as_deref() == Some(trust_id) {
                slot.guest.name = trimmed.clone();
            }
        }
    }

    pub fn remove_guest(&self, guest_id: &str, also_untrust: bool) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut tid_to_remove = None;
        if let Some(slot) = guard.guests.iter_mut().find(|s| s.guest.id == guest_id) {
            slot.decision = GuestDecision::Removed;
            if also_untrust {
                tid_to_remove = slot.guest.trust_id.take();
                slot.guest.trusted = false;
            }
        }
        if let Some(tid) = tid_to_remove {
            guard.trusted.retain(|t| t.id != tid);
        }
        guard
            .asks
            .retain(|a| !matches!(a, LiveShareAsk::Join { guest_id: gid, .. } if gid == guest_id));
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.thread.take() {
            let _ = worker.join();
        }
    }
}

fn serve_connection(
    stream: TcpStream,
    config: Arc<rustls::ServerConfig>,
    stop: Arc<AtomicBool>,
    inner: Arc<Mutex<HostInner>>,
) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let Ok(connection) = rustls::ServerConnection::new(config) else {
        return;
    };
    let mut tls = rustls::StreamOwned::new(
        connection,
        DeadlineStream::new(stream, Duration::from_secs(15)),
    );
    let Ok(first) = read_frame(&mut tls, MAX_REQUEST) else {
        return;
    };
    let supplied = first
        .get("token")
        .and_then(Value::as_str)
        .and_then(|v| B64.decode(v).ok())
        .unwrap_or_default();

    // Mode 1: One-shot legacy request (`{"token", "operation", "path"}`)
    if first.get("mode").and_then(Value::as_str) != Some("session") {
        let response = {
            let guard = inner.lock().unwrap_or_else(|e| e.into_inner());
            if stop.load(Ordering::Relaxed) {
                json!({"error":"Sharing stopped."})
            } else {
                let now_ms = now_epoch_ms();
                let matched = guard.invites.iter().find(|inv| {
                    supplied.len() == 32
                        && bool::from(supplied.as_slice().ct_eq(&inv.token_bytes))
                        && inv.status(now_ms) == InviteStatus::Live
                });
                if let Some(inv) = matched {
                    let hide_ignored = guard.settings.hide_ignored;
                    let proj = guard
                        .shared_projects
                        .iter()
                        .find(|p| inv.projects.contains(&p.id))
                        .or_else(|| guard.shared_projects.first());
                    if let Some(proj) = proj {
                        match handle_readonly_op(&proj.path, &first, hide_ignored) {
                            Ok(result) => json!({"result":result}),
                            Err(_) => {
                                json!({"error":"Request unavailable or not allowed by this share."})
                            }
                        }
                    } else {
                        json!({"error":"No shared project available."})
                    }
                } else {
                    json!({"error":"Invitation expired or invalid."})
                }
            }
        };
        if !stop.load(Ordering::Relaxed) {
            let _ = write_frame(&mut tls, &response);
        }
        return;
    }

    // Mode 2: Persistent duplex Live Share session
    let client_nonce_vec = first
        .get("clientNonce")
        .and_then(Value::as_str)
        .and_then(|s| B64.decode(s).ok())
        .unwrap_or_default();
    let Ok(client_nonce): std::result::Result<[u8; 32], _> = client_nonce_vec.try_into() else {
        let _ = write_frame(&mut tls, &json!({"error":"Invalid handshake nonce."}));
        return;
    };
    let mut server_nonce = [0u8; 32];
    if getrandom::fill(&mut server_nonce).is_err() {
        return;
    }
    let raw_name = first
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("A guest")
        .trim();
    let guest_name: String = if raw_name.is_empty() {
        "A guest".into()
    } else {
        raw_name.chars().take(40).collect()
    };
    let identity_hex = first
        .get("identity")
        .and_then(Value::as_str)
        .filter(|s| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()))
        .map(str::to_owned);

    let (guest_id, sas_code, host_name, initially_joined) = {
        let mut guard = inner.lock().unwrap_or_else(|e| e.into_inner());
        if stop.load(Ordering::Relaxed) {
            let _ = write_frame(&mut tls, &json!({"error":"stopped"}));
            return;
        }
        let Some(inv_idx) = guard.match_valid_invite_mut(&supplied) else {
            let _ = write_frame(&mut tls, &json!({"error":"refused"}));
            return;
        };
        let sas = derive_sas_code(&guard.cert_der, &client_nonce, &server_nonce);
        let gid = format!("g{}", guard.next_guest_seq);
        guard.next_guest_seq += 1;

        let inv_id = guard.invites[inv_idx].id.clone();
        let inv_label = guard.invites[inv_idx].display_name(false);
        let offered_projects = guard.invites[inv_idx].projects.clone();
        let offered_rights = guard.invites[inv_idx].rights;

        // Check if guest is already trusted!
        let shared_project_ids: Vec<u64> = guard.shared_projects.iter().map(|p| p.id).collect();
        let trusted_match = identity_hex.as_ref().and_then(|hex| {
            guard
                .trusted
                .iter_mut()
                .find(|t| t.identity_hex == *hex)
                .map(|tp| {
                    tp.last_seen_ms = Some(now_epoch_ms());
                    let active_shared: Vec<u64> = shared_project_ids
                        .iter()
                        .copied()
                        .filter(|id| tp.projects.contains(id))
                        .collect();
                    let effective_projs = if active_shared.is_empty() {
                        offered_projects.clone()
                    } else {
                        active_shared
                    };
                    (tp.id.clone(), tp.name.clone(), effective_projs, tp.rights)
                })
        });

        if let Some((tid, tname, projs, rts)) = trusted_match {
            guard.invites[inv_idx].uses += 1;
            guard.invites[inv_idx].joined += 1;
            guard.guests.push(GuestSlot {
                guest: ConnectedGuest {
                    id: gid.clone(),
                    name: tname,
                    state: GuestConnectionState::Joined,
                    code: sas.clone(),
                    invite_id: Some(inv_id),
                    invite_label: inv_label,
                    projects: projs.clone(),
                    rights: rts,
                    trusted: true,
                    trust_id: Some(tid),
                    identity_hex,
                    since_ms: now_epoch_ms(),
                    watching_pane: None,
                    typing_pane: None,
                },
                decision: GuestDecision::Approved,
            });
            (gid, sas, guard.host_name.clone(), true)
        } else {
            let ask_key = format!("ask-{}", guard.next_ask_seq);
            guard.next_ask_seq += 1;
            let trustable = identity_hex.is_some();
            guard.guests.push(GuestSlot {
                guest: ConnectedGuest {
                    id: gid.clone(),
                    name: guest_name.clone(),
                    state: GuestConnectionState::Waiting,
                    code: sas.clone(),
                    invite_id: Some(inv_id),
                    invite_label: inv_label.clone(),
                    projects: offered_projects.clone(),
                    rights: offered_rights,
                    trusted: false,
                    trust_id: None,
                    identity_hex,
                    since_ms: now_epoch_ms(),
                    watching_pane: None,
                    typing_pane: None,
                },
                decision: GuestDecision::Pending,
            });
            guard.asks.push_back(LiveShareAsk::Join {
                key: ask_key,
                guest_id: gid.clone(),
                name: guest_name,
                code: sas.clone(),
                invite_label: inv_label,
                offered_projects,
                offered_rights,
                trustable,
            });
            (gid, sas, guard.host_name.clone(), false)
        }
    };

    let hello_reply = json!({
        "status": if initially_joined { "joined" } else { "waiting" },
        "guestId": guest_id,
        "hostName": host_name,
        "serverNonce": B64.encode(server_nonce),
        "sas": sas_code,
    });
    tls.get_mut().reset_deadline(Duration::from_secs(15));
    if write_frame(&mut tls, &hello_reply).is_err() {
        cleanup_disconnected_guest(&inner, &guest_id, false);
        return;
    }

    // Wait for host knock approval if not initially joined
    if !initially_joined {
        let knock_start = Instant::now();
        loop {
            if stop.load(Ordering::Relaxed) {
                let _ = write_frame(&mut tls, &json!({"status":"ended","reason":"stopped"}));
                cleanup_disconnected_guest(&inner, &guest_id, false);
                return;
            }
            if knock_transport::disconnected(&tls.get_mut().stream) {
                cleanup_disconnected_guest(&inner, &guest_id, false);
                return;
            }
            if knock_start.elapsed() > Duration::from_secs(300) {
                let _ = write_frame(&mut tls, &json!({"status":"ended","reason":"timeout"}));
                cleanup_disconnected_guest(&inner, &guest_id, true);
                return;
            }
            let decision_state = {
                let guard = inner.lock().unwrap_or_else(|e| e.into_inner());
                guard
                    .guests
                    .iter()
                    .find(|s| s.guest.id == guest_id)
                    .map(|s| match &s.decision {
                        GuestDecision::Pending => 0u8,
                        GuestDecision::Approved => 1u8,
                        GuestDecision::Denied => 2u8,
                        GuestDecision::Removed => 3u8,
                    })
                    .unwrap_or(3)
            };
            match decision_state {
                0 => {
                    thread::sleep(Duration::from_millis(50));
                }
                1 => {
                    tls.get_mut().reset_deadline(Duration::from_secs(15));
                    if write_frame(&mut tls, &json!({"status":"joined"})).is_err() {
                        cleanup_disconnected_guest(&inner, &guest_id, false);
                        return;
                    }
                    break;
                }
                2 => {
                    tls.get_mut().reset_deadline(Duration::from_secs(5));
                    let _ = write_frame(&mut tls, &json!({"status":"ended","reason":"denied"}));
                    cleanup_disconnected_guest(&inner, &guest_id, false);
                    return;
                }
                _ => {
                    tls.get_mut().reset_deadline(Duration::from_secs(5));
                    let _ = write_frame(&mut tls, &json!({"status":"ended","reason":"removed"}));
                    cleanup_disconnected_guest(&inner, &guest_id, false);
                    return;
                }
            }
        }
    }

    // Active RPC loop for an admitted guest
    loop {
        if stop.load(Ordering::Relaxed) {
            tls.get_mut().reset_deadline(Duration::from_secs(3));
            let _ = write_frame(&mut tls, &json!({"error":"stopped"}));
            break;
        }
        tls.get_mut().reset_deadline(Duration::from_secs(60));
        let req = match read_frame(&mut tls, MAX_REQUEST) {
            Ok(v) => v,
            Err(_) => break,
        };
        if req.get("op").and_then(Value::as_str) == Some("leave") {
            break;
        }
        let reply = dispatch_session_rpc(&inner, &guest_id, &req);
        let ended = reply.get("ended").and_then(Value::as_bool).unwrap_or(false);
        tls.get_mut().reset_deadline(Duration::from_secs(15));
        if write_frame(&mut tls, &reply).is_err() || ended {
            break;
        }
    }

    cleanup_disconnected_guest(&inner, &guest_id, false);
}

#[path = "share/knock_transport.rs"]
mod knock_transport;

fn cleanup_disconnected_guest(inner: &Arc<Mutex<HostInner>>, guest_id: &str, count_failed: bool) {
    let mut guard = inner.lock().unwrap_or_else(|e| e.into_inner());
    let mut bump_failed_inv = None;
    if let Some(pos) = guard.guests.iter().position(|s| s.guest.id == guest_id) {
        let slot = guard.guests.remove(pos);
        if slot.guest.state == GuestConnectionState::Joined
            && !matches!(slot.decision, GuestDecision::Removed)
        {
            guard.offline.retain(|o| o.name != slot.guest.name);
            guard.offline.push(OfflinePerson {
                id: format!("k-{}", slot.guest.id),
                name: slot.guest.name,
                invite_label: slot.guest.invite_label,
                projects: slot.guest.projects,
                rights: slot.guest.rights,
                trust_id: slot.guest.trust_id,
            });
        } else if count_failed {
            bump_failed_inv = slot.guest.invite_id;
        }
    }
    if let Some(inv_id) = bump_failed_inv
        && let Some(inv) = guard.invites.iter_mut().find(|i| i.id == inv_id)
    {
        inv.failed_knocks += 1;
        if inv.failed_knocks >= 5 {
            inv.blocked = true;
        }
    }
    guard
        .asks
        .retain(|a| !matches!(a, LiveShareAsk::Join { guest_id: gid, .. } if gid == guest_id));
}

fn dispatch_session_rpc(inner: &Arc<Mutex<HostInner>>, guest_id: &str, req: &Value) -> Value {
    let op = req.get("op").and_then(Value::as_str).unwrap_or("");
    if matches!(op, "tasks.list" | "tasks.mutate") {
        return task_board::dispatch(inner, guest_id, req);
    }
    if matches!(
        op,
        "files.write" | "files.create" | "files.rename" | "files.delete"
    ) {
        return guest_file_mutations::dispatch(inner, guest_id, req);
    }
    if matches!(op, "files.list" | "files.read") {
        return guest_file_reads::dispatch(inner, guest_id, req);
    }
    if op.starts_with("scm.") {
        return guest_git_dispatch::dispatch(inner, guest_id, req);
    }
    let mut guard = inner.lock().unwrap_or_else(|e| e.into_inner());
    if guard.stopped.load(Ordering::SeqCst) {
        return json!({"ended":true,"reason":"stopped","error":"Sharing has stopped."});
    }
    let Some(g_idx) = guard.guests.iter().position(|s| s.guest.id == guest_id) else {
        return json!({"ended": true, "reason": "removed", "error": "Removed from session."});
    };
    if matches!(
        guard.guests[g_idx].decision,
        GuestDecision::Removed | GuestDecision::Denied
    ) {
        return json!({"ended": true, "reason": "removed", "error": "The host removed you from the session."});
    }
    let rights = guard.guests[g_idx].guest.rights;
    let guest_name = guard.guests[g_idx].guest.name.clone();
    let allowed_projects = guard.guests[g_idx].guest.projects.clone();

    let res: Result<Value> = (|| -> Result<Value> {
        match op {
            "workspace.snapshot" => {
                let projects: Vec<Value> = guard
                    .shared_projects
                    .iter()
                    .filter(|p| allowed_projects.contains(&p.id))
                    .map(|p| json!({"id": p.id, "name": p.name}))
                    .collect();
                let panes: Vec<Value> = guard
                    .panes
                    .iter()
                    .filter(|pane| allowed_projects.contains(&pane.project_id))
                    .map(|pane| {
                        json!({
                            "id": pane.id,
                            "projectId": pane.project_id,
                            "title": pane.title,
                            "isAgent": pane.is_agent,
                        })
                    })
                    .collect();
                let pages: Vec<Value> = if rights.pages_view {
                    guard
                        .pages
                        .iter()
                        .filter(|pg| {
                            pg.project_id
                                .is_none_or(|pid| allowed_projects.contains(&pid))
                        })
                        .map(|pg| {
                            json!({"projectId": pg.project_id, "title": pg.title, "url": pg.url})
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                let host_proj = guard
                    .host_active_project
                    .filter(|id| allowed_projects.contains(id));
                let host_pane = guard.host_active_pane.filter(|pid| {
                    guard
                        .panes
                        .iter()
                        .any(|p| p.id == *pid && allowed_projects.contains(&p.project_id))
                });
                Ok(json!({
                    "hostName": guard.host_name,
                    "rights": rights,
                    "projects": projects,
                    "panes": panes,
                    "pages": pages,
                    "hostActiveProject": host_proj,
                    "hostActivePane": host_pane,
                }))
            }
            "presence.update" => {
                let watching = req.get("watchingPane").and_then(Value::as_u64);
                let typing = req.get("typingPane").and_then(Value::as_u64);
                let now_ms = now_epoch_ms();
                let slot = &mut guard.guests[g_idx];
                slot.guest.watching_pane = watching;
                if let Some(tp) = typing
                    && rights.r#type
                {
                    slot.guest.typing_pane = Some((tp, now_ms));
                } else if typing.is_none() {
                    slot.guest.typing_pane = None;
                }
                Ok(json!({"ok": true}))
            }
            "term.output" | "term.subscribe" => {
                let pane_id = req
                    .get("paneId")
                    .and_then(Value::as_u64)
                    .ok_or("Missing paneId")?;
                let payload = {
                    let pane = guard
                        .panes
                        .iter()
                        .find(|p| p.id == pane_id && allowed_projects.contains(&p.project_id))
                        .ok_or("Pane is not shared with you.")?;
                    json!({
                        "paneId": pane.id,
                        "title": pane.title,
                        "output": pane.output,
                    })
                };
                guard.guests[g_idx].guest.watching_pane = Some(pane_id);
                Ok(payload)
            }
            "term.write" => {
                if !rights.r#type {
                    return Err(
                        "Read-only: the host has not given you the right to type here.".into(),
                    );
                }
                let pane_id = req
                    .get("paneId")
                    .and_then(Value::as_u64)
                    .ok_or("Missing paneId")?;
                if !guard
                    .panes
                    .iter()
                    .any(|p| p.id == pane_id && allowed_projects.contains(&p.project_id))
                {
                    return Err("Pane is not shared with you.".into());
                }
                let data_b64 = req.get("data").and_then(Value::as_str).unwrap_or("");
                let bytes = B64.decode(data_b64).map_err(|_| "Invalid terminal bytes")?;
                if bytes.len() > 65_536 {
                    return Err("Input exceeds 64 KiB limit.".into());
                }
                // Strip terminal device/cursor/focus/OSC replies so a guest screen can never inject them!
                if !bytes.is_empty() && !crate::layout_broadcast_ui::is_terminal_reply(&bytes) {
                    let now_ms = now_epoch_ms();
                    guard.guests[g_idx].guest.typing_pane = Some((pane_id, now_ms));
                    guard
                        .commands
                        .push_back(HostCommand::WriteTerminal { pane_id, bytes });
                }
                Ok(json!({"ok": true}))
            }
            "term.resize" => {
                if !rights.r#type {
                    return Err("Type permission is required to resize host terminals.".into());
                }
                let pane_id = req
                    .get("paneId")
                    .and_then(Value::as_u64)
                    .ok_or("Missing paneId")?;
                let rows = req
                    .get("rows")
                    .and_then(Value::as_u64)
                    .unwrap_or(24)
                    .clamp(4, 300) as u16;
                let cols = req
                    .get("cols")
                    .and_then(Value::as_u64)
                    .unwrap_or(80)
                    .clamp(10, 500) as u16;
                if guard
                    .panes
                    .iter()
                    .any(|p| p.id == pane_id && allowed_projects.contains(&p.project_id))
                {
                    guard.commands.push_back(HostCommand::ResizeTerminal {
                        pane_id,
                        rows,
                        cols,
                    });
                }
                Ok(json!({"ok": true}))
            }
            "term.open" => {
                let proj = guard.find_shared_project_for_guest(
                    g_idx,
                    req.get("projectId").and_then(Value::as_u64),
                )?;
                let agent_name = req
                    .get("agent")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                if rights.open {
                    guard.commands.push_back(HostCommand::OpenPane {
                        project_id: proj.id,
                        agent_name,
                    });
                    Ok(json!({"opened": true}))
                } else {
                    let key = format!("ask-{}", guard.next_ask_seq);
                    guard.next_ask_seq += 1;
                    guard.asks.push_back(LiveShareAsk::Open {
                        key,
                        guest_id: guest_id.to_owned(),
                        name: guest_name,
                        project_id: proj.id,
                        project_name: proj.name,
                        agent_name,
                    });
                    Ok(json!({"opened": false, "asked": true}))
                }
            }
            "term.close" => {
                if !rights.close {
                    return Err("You do not have the right to close panes.".into());
                }
                let pane_id = req
                    .get("paneId")
                    .and_then(Value::as_u64)
                    .ok_or("Missing paneId")?;
                let pane = guard
                    .panes
                    .iter()
                    .find(|p| p.id == pane_id && allowed_projects.contains(&p.project_id))
                    .cloned()
                    .ok_or("Pane is not shared with you.")?;
                if pane.is_agent {
                    let key = format!("ask-{}", guard.next_ask_seq);
                    guard.next_ask_seq += 1;
                    guard.asks.push_back(LiveShareAsk::Close {
                        key,
                        guest_id: guest_id.to_owned(),
                        name: guest_name,
                        pane_id,
                        pane_name: pane.title,
                        detail: "Its agent session ends on your computer.".into(),
                    });
                    Ok(json!({"closed": false, "asked": true}))
                } else {
                    guard.commands.push_back(HostCommand::ClosePane { pane_id });
                    Ok(json!({"closed": true}))
                }
            }
            "pages.list" => {
                if !rights.pages_view {
                    return Err("Pages permission is required to view browser pages.".into());
                }
                let pages: Vec<Value> = guard
                    .pages
                    .iter()
                    .filter(|pg| {
                        pg.project_id
                            .is_none_or(|pid| allowed_projects.contains(&pid))
                    })
                    .map(|pg| json!({"projectId": pg.project_id, "title": pg.title, "url": pg.url}))
                    .collect();
                Ok(json!({"pages": pages}))
            }
            "usage.get" => {
                if !rights.usage {
                    return Err("Usage permission is required.".into());
                }
                Ok(json!({"summary": guard.usage_summary}))
            }
            _ => Err("Operation is not permitted.".into()),
        }
    })();

    match res {
        Ok(val) => json!({"result": val}),
        Err(err) => json!({"error": err}),
    }
}

fn handle_readonly_op(root: &Path, request: &Value, hide_ignored: bool) -> Result<Value> {
    let operation = request
        .get("operation")
        .and_then(Value::as_str)
        .ok_or("Missing operation")?;
    let relative = request.get("path").and_then(Value::as_str).unwrap_or("");
    let path = if relative.is_empty() {
        root.to_owned()
    } else {
        share_path_checked(root, relative, hide_ignored)?
    };
    match operation {
        "list" => {
            let entries = list_dir_entries(root, &path)?;
            let filtered: Vec<Value> = entries
                .into_iter()
                .filter(|e| {
                    let rel = e.path.strip_prefix(root).unwrap_or(&e.path);
                    !private_path(rel) && !(hide_ignored && secret_or_dot_path(rel))
                })
                .map(|e| {
                    json!({
                        "name": e.name,
                        "path": e.path.strip_prefix(root).unwrap_or(&e.path).to_string_lossy(),
                        "directory": e.directory
                    })
                })
                .collect();
            Ok(json!({"entries": filtered, "readOnly": true}))
        }
        "read" => {
            let mut bytes = Vec::new();
            std::fs::File::open(&path)
                .map_err(|_| "File unavailable")?
                .take((MAX_FILE + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|_| "File read failed")?;
            if bytes.len() > MAX_FILE {
                return Err("Guest download limit is 8 MiB.".into());
            }
            Ok(json!({
                "name": path.file_name().unwrap_or_default().to_string_lossy(),
                "data": B64.encode(bytes)
            }))
        }
        _ => Err("Read-only share: operation is not permitted.".into()),
    }
}

// ---------------------------------------------------------------------------
// 7. One-Shot Legacy `request` & Persistent Duplex `GuestSession`
// ---------------------------------------------------------------------------

fn make_pinned_tls_client(
    invitation: &Invitation,
    connect_timeout: Duration,
) -> Result<(
    rustls::StreamOwned<rustls::ClientConnection, DeadlineStream>,
    Vec<u8>,
)> {
    make_cancellable_pinned_tls_client(invitation, connect_timeout, None)
}

fn make_cancellable_pinned_tls_client(
    invitation: &Invitation,
    connect_timeout: Duration,
    stop: Option<&Arc<GuestStop>>,
) -> Result<(
    rustls::StreamOwned<rustls::ClientConnection, DeadlineStream>,
    Vec<u8>,
)> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let cert = B64
        .decode(&invitation.certificate)
        .map_err(|_| "Invalid certificate")?;
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(rustls::pki_types::CertificateDer::from(cert.clone()))
        .map_err(|_| "Invalid certificate")?;
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let stream = TcpStream::connect_timeout(&invitation.address, connect_timeout)
        .map_err(|_| "Host is unreachable")?;
    if let Some(stop) = stop {
        stop.register(&stream)?;
    }
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .map_err(|e| e.to_string())?;
    let connection = rustls::ClientConnection::new(
        Arc::new(config),
        rustls::pki_types::ServerName::try_from("tessel-rust").map_err(|_| "Invalid TLS name")?,
    )
    .map_err(|e| e.to_string())?;
    let mut transport = DeadlineStream::new(stream, Duration::from_secs(15));
    transport.stop = stop.cloned();
    let tls = rustls::StreamOwned::new(connection, transport);
    Ok((tls, cert))
}

pub fn request(invitation: &Invitation, operation: &str, path: &str) -> Result<Value> {
    let (mut tls, _) = make_pinned_tls_client(invitation, Duration::from_secs(5))?;
    write_frame(
        &mut tls,
        &json!({"token": invitation.token, "operation": operation, "path": path}),
    )?;
    let reply = read_frame(&mut tls, MAX_RESPONSE)?;
    if let Some(error) = reply.get("error").and_then(Value::as_str) {
        return Err(error.into());
    }
    reply
        .get("result")
        .cloned()
        .ok_or_else(|| "Missing host response".into())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestPhase {
    Connecting,
    WaitingForHost { sas_code: String, host_name: String },
    Joined { sas_code: String, host_name: String },
    Ended { reason: String, host_name: String },
}

#[derive(Debug, Clone)]
pub struct GuestSnapshot {
    pub phase: GuestPhase,
    pub endpoint: String,
    pub rights: ShareRights,
    pub projects: Vec<(u64, String)>,
    pub panes: Vec<(u64, u64, String, bool)>,
    pub pages: Vec<SharedBrowserPage>,
    pub host_active_project: Option<u64>,
    pub host_active_pane: Option<u64>,
    pub last_pane_output: Option<(u64, String, String)>,
    pub last_error: String,
}

#[path = "share/guest_rpc.rs"]
mod guest_rpc;
pub use guest_rpc::{GuestRpcError, TASK_RESULT_NOT_AVAILABLE};
#[path = "share/guest_shutdown.rs"]
mod guest_shutdown;
use guest_shutdown::{GuestStop, SocketLifetime, active_state, mark_left};

type GuestRpcResult = std::result::Result<Value, GuestRpcError>;

enum GuestRpcReq {
    Call {
        payload: Value,
        reply: Option<SyncSender<GuestRpcResult>>,
    },
}

pub struct GuestSession {
    generation: u64,
    state: Arc<Mutex<GuestSnapshot>>,
    tx: SyncSender<GuestRpcReq>,
    stop: Arc<GuestStop>,
    worker: Option<thread::JoinHandle<()>>,
}

impl GuestSession {
    pub fn connect(invitation: Invitation, guest_name: String, identity_hex: String) -> Self {
        static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
        let generation = NEXT_GENERATION
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .expect("guest session generation exhausted");
        let snap = GuestSnapshot {
            phase: GuestPhase::Connecting,
            endpoint: invitation.endpoint(),
            rights: ShareRights::view_only(),
            projects: Vec::new(),
            panes: Vec::new(),
            pages: Vec::new(),
            host_active_project: None,
            host_active_pane: None,
            last_pane_output: None,
            last_error: String::new(),
        };
        let state = Arc::new(Mutex::new(snap));
        let (tx, rx) = mpsc::sync_channel::<GuestRpcReq>(32);
        let stop = Arc::new(GuestStop::default());
        let state_worker = Arc::clone(&state);
        let stop_worker = Arc::clone(&stop);

        let worker = thread::spawn(move || {
            run_guest_worker(
                invitation,
                guest_name,
                identity_hex,
                state_worker,
                rx,
                stop_worker,
            );
        });

        Self {
            generation,
            state,
            tx,
            stop,
            worker: Some(worker),
        }
    }

    pub fn snapshot(&self) -> GuestSnapshot {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Queue a request without blocking the UI; the receiver is polled by its owner.
    pub fn request(&self, payload: Value) -> Result<Receiver<GuestRpcResult>> {
        if self.stop.is_stopped() {
            return Err("Session closed.".into());
        }
        let (reply, receiver) = mpsc::sync_channel(1);
        self.tx
            .try_send(GuestRpcReq::Call {
                payload,
                reply: Some(reply),
            })
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => "The host request queue is full.".to_owned(),
                mpsc::TrySendError::Disconnected(_) => "Session closed.".to_owned(),
            })?;
        Ok(receiver)
    }

    /// Synchronous RPC call over the persistent duplex TLS stream (used by tests & actions).
    pub fn call_sync(&self, payload: Value, timeout: Duration) -> Result<Value> {
        let rrx = self.request(payload)?;
        rrx.recv_timeout(timeout)
            .map_err(|_| "Timed out waiting for host reply.".to_string())?
            .map_err(|error| error.to_string())
    }

    /// Non-blocking fire-and-update request for UI buttons.
    pub fn send_async(&self, payload: Value) {
        if self.stop.is_stopped() {
            return;
        }
        let _ = self.tx.try_send(GuestRpcReq::Call {
            payload,
            reply: None,
        });
    }

    pub fn leave(&mut self) {
        self.stop.cancel();
        mark_left(&self.state);
        // Dropping the handle detaches the bounded/cancellable worker; never join on UI.
        self.worker.take();
    }
}
impl Drop for GuestSession {
    fn drop(&mut self) {
        self.leave();
    }
}

fn run_guest_worker(
    invitation: Invitation,
    guest_name: String,
    identity_hex: String,
    state: Arc<Mutex<GuestSnapshot>>,
    rx: Receiver<GuestRpcReq>,
    stop: Arc<GuestStop>,
) {
    let _socket_lifetime = SocketLifetime(&stop);
    if stop.is_stopped() {
        return;
    }
    let mut client_nonce = [0u8; 32];
    if getrandom::fill(&mut client_nonce).is_err() {
        let Some(mut g) = active_state(&state, &stop) else {
            return;
        };
        g.phase = GuestPhase::Ended {
            reason: "error".into(),
            host_name: String::new(),
        };
        return;
    }

    let (mut tls, cert_der) = match make_cancellable_pinned_tls_client(
        &invitation,
        Duration::from_secs(5),
        Some(&stop),
    ) {
        Ok(v) => v,
        Err(_) => {
            let Some(mut g) = active_state(&state, &stop) else {
                return;
            };
            g.phase = GuestPhase::Ended {
                reason: "unreachable".into(),
                host_name: String::new(),
            };
            return;
        }
    };

    let hello = json!({
        "mode": "session",
        "token": invitation.token,
        "name": guest_name,
        "clientNonce": B64.encode(client_nonce),
        "identity": identity_hex,
    });
    if write_frame(&mut tls, &hello).is_err() {
        let Some(mut g) = active_state(&state, &stop) else {
            return;
        };
        g.phase = GuestPhase::Ended {
            reason: "closed".into(),
            host_name: String::new(),
        };
        return;
    }

    let reply = match read_frame(&mut tls, MAX_RESPONSE) {
        Ok(v) => v,
        Err(_) => {
            let Some(mut g) = active_state(&state, &stop) else {
                return;
            };
            g.phase = GuestPhase::Ended {
                reason: "closed".into(),
                host_name: String::new(),
            };
            return;
        }
    };

    if let Some(err) = reply.get("error").and_then(Value::as_str) {
        let Some(mut g) = active_state(&state, &stop) else {
            return;
        };
        g.phase = GuestPhase::Ended {
            reason: err.to_owned(),
            host_name: String::new(),
        };
        return;
    }

    let host_name = reply
        .get("hostName")
        .and_then(Value::as_str)
        .unwrap_or("Host")
        .to_owned();
    let server_nonce_vec = reply
        .get("serverNonce")
        .and_then(Value::as_str)
        .and_then(|s| B64.decode(s).ok())
        .unwrap_or_default();
    let Ok(server_nonce): std::result::Result<[u8; 32], _> = server_nonce_vec.try_into() else {
        let Some(mut g) = active_state(&state, &stop) else {
            return;
        };
        g.phase = GuestPhase::Ended {
            reason: "protocol".into(),
            host_name,
        };
        return;
    };
    let computed_sas = derive_sas_code(&cert_der, &client_nonce, &server_nonce);
    let mut joined = reply.get("status").and_then(Value::as_str) == Some("joined");

    {
        let Some(mut g) = active_state(&state, &stop) else {
            return;
        };
        g.phase = if joined {
            GuestPhase::Joined {
                sas_code: computed_sas.clone(),
                host_name: host_name.clone(),
            }
        } else {
            GuestPhase::WaitingForHost {
                sas_code: computed_sas.clone(),
                host_name: host_name.clone(),
            }
        };
    }

    if !joined {
        // Wait up to 300s for knock decision while polling `stop`
        let _ = tls
            .get_mut()
            .stream
            .set_read_timeout(Some(Duration::from_millis(150)));
        let wait_start = Instant::now();
        while !stop.is_stopped() && wait_start.elapsed() < Duration::from_secs(300) {
            tls.get_mut().reset_deadline(Duration::from_secs(10));
            let mut header = [0u8; 4];
            match tls.read_exact(&mut header) {
                Ok(()) => {
                    let size = u32::from_be_bytes(header) as usize;
                    if size > MAX_RESPONSE {
                        break;
                    }
                    let mut body = vec![0u8; size];
                    if tls.read_exact(&mut body).is_err() {
                        break;
                    }
                    let Ok(decision) = serde_json::from_slice::<Value>(&body) else {
                        break;
                    };
                    if decision.get("status").and_then(Value::as_str) == Some("joined") {
                        joined = true;
                        let Some(mut g) = active_state(&state, &stop) else {
                            return;
                        };
                        g.phase = GuestPhase::Joined {
                            sas_code: computed_sas.clone(),
                            host_name: host_name.clone(),
                        };
                        break;
                    } else {
                        let reason = decision
                            .get("reason")
                            .and_then(Value::as_str)
                            .unwrap_or("denied")
                            .to_owned();
                        let Some(mut g) = active_state(&state, &stop) else {
                            return;
                        };
                        g.phase = GuestPhase::Ended { reason, host_name };
                        return;
                    }
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    if stop.is_stopped() {
                        return;
                    }
                }
                Err(_) => break,
            }
        }
        if !joined {
            let Some(mut g) = active_state(&state, &stop) else {
                return;
            };
            g.phase = GuestPhase::Ended {
                reason: if stop.is_stopped() {
                    "left".into()
                } else {
                    "timeout".into()
                },
                host_name,
            };
            return;
        }
    }

    let _ = tls
        .get_mut()
        .stream
        .set_read_timeout(Some(Duration::from_secs(10)));

    // Initial snapshot + root file listing
    let _ = perform_guest_rpc(
        &mut tls,
        &state,
        &stop,
        &host_name,
        &json!({"op": "workspace.snapshot"}),
    );

    let mut last_poll = Instant::now();
    while !stop.is_stopped() {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(GuestRpcReq::Call { payload, reply }) => {
                let res = perform_guest_rpc(&mut tls, &state, &stop, &host_name, &payload);
                let ended = {
                    let g = state.lock().unwrap_or_else(|e| e.into_inner());
                    matches!(g.phase, GuestPhase::Ended { .. })
                };
                if let Some(rtx) = reply {
                    let _ = rtx.send(res);
                }
                if ended {
                    return;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if last_poll.elapsed() >= Duration::from_secs(2) {
                    last_poll = Instant::now();
                    if perform_guest_rpc(
                        &mut tls,
                        &state,
                        &stop,
                        &host_name,
                        &json!({"op": "workspace.snapshot"}),
                    )
                    .is_err()
                    {
                        let g = state.lock().unwrap_or_else(|e| e.into_inner());
                        if matches!(g.phase, GuestPhase::Ended { .. }) {
                            return;
                        }
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn perform_guest_rpc(
    tls: &mut rustls::StreamOwned<rustls::ClientConnection, DeadlineStream>,
    state: &Arc<Mutex<GuestSnapshot>>,
    stop: &GuestStop,
    host_name: &str,
    payload: &Value,
) -> GuestRpcResult {
    if stop.is_stopped() {
        return Err(GuestRpcError::Transport("Session closed.".into()));
    }
    tls.get_mut().reset_deadline(Duration::from_secs(15));
    if let Err(e) = write_frame(tls, payload) {
        let Some(mut g) = active_state(state, stop) else {
            return Err(GuestRpcError::Transport("Session closed.".into()));
        };
        g.phase = GuestPhase::Ended {
            reason: "lost".into(),
            host_name: host_name.to_owned(),
        };
        return Err(GuestRpcError::Transport(e));
    }
    tls.get_mut().reset_deadline(Duration::from_secs(15));
    let reply = match read_frame(tls, MAX_RESPONSE) {
        Ok(v) => v,
        Err(e) => {
            let Some(mut g) = active_state(state, stop) else {
                return Err(GuestRpcError::Transport("Session closed.".into()));
            };
            g.phase = GuestPhase::Ended {
                reason: "closed".into(),
                host_name: host_name.to_owned(),
            };
            return Err(GuestRpcError::Transport(e));
        }
    };

    if reply.get("ended").and_then(Value::as_bool) == Some(true)
        || reply.get("error").and_then(Value::as_str) == Some("stopped")
    {
        let reason = reply
            .get("reason")
            .or_else(|| reply.get("error"))
            .and_then(Value::as_str)
            .unwrap_or("stopped")
            .to_owned();
        let Some(mut g) = active_state(state, stop) else {
            return Err(GuestRpcError::Transport("Session closed.".into()));
        };
        g.phase = GuestPhase::Ended {
            reason,
            host_name: host_name.to_owned(),
        };
        return Err(GuestRpcError::Host("Session ended.".into()));
    }

    if let Some(err) = reply.get("error").and_then(Value::as_str) {
        let Some(mut g) = active_state(state, stop) else {
            return Err(GuestRpcError::Transport("Session closed.".into()));
        };
        g.last_error = err.to_owned();
        return Err(GuestRpcError::Host(err.to_owned()));
    }

    let result = reply.get("result").cloned().unwrap_or(Value::Null);
    let op = payload.get("op").and_then(Value::as_str).unwrap_or("");
    {
        let Some(mut g) = active_state(state, stop) else {
            return Err(GuestRpcError::Transport("Session closed.".into()));
        };
        g.last_error.clear();
        match op {
            "workspace.snapshot" => {
                g.rights = ShareRights::clean_from_value(result.get("rights"));
                if let Some(arr) = result.get("projects").and_then(Value::as_array) {
                    g.projects = arr
                        .iter()
                        .filter_map(|v| {
                            Some((v.get("id")?.as_u64()?, v.get("name")?.as_str()?.to_owned()))
                        })
                        .collect();
                }
                if let Some(arr) = result.get("panes").and_then(Value::as_array) {
                    g.panes = arr
                        .iter()
                        .filter_map(|v| {
                            Some((
                                v.get("id")?.as_u64()?,
                                v.get("projectId")?.as_u64()?,
                                v.get("title")?.as_str()?.to_owned(),
                                v.get("isAgent").and_then(Value::as_bool).unwrap_or(false),
                            ))
                        })
                        .collect();
                }
                if let Some(arr) = result.get("pages").and_then(Value::as_array) {
                    g.pages = arr
                        .iter()
                        .filter_map(|v| {
                            Some(SharedBrowserPage {
                                project_id: v.get("projectId").and_then(Value::as_u64),
                                title: v.get("title")?.as_str()?.to_owned(),
                                url: v.get("url")?.as_str()?.to_owned(),
                            })
                        })
                        .collect();
                }
                g.host_active_project = result.get("hostActiveProject").and_then(Value::as_u64);
                g.host_active_pane = result.get("hostActivePane").and_then(Value::as_u64);
            }
            "term.output" | "term.subscribe" => {
                if let (Some(pid), Some(title), Some(output)) = (
                    result.get("paneId").and_then(Value::as_u64),
                    result.get("title").and_then(Value::as_str),
                    result.get("output").and_then(Value::as_str),
                ) {
                    g.last_pane_output = Some((pid, title.to_owned(), output.to_owned()));
                }
            }
            _ => {}
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Host) {
        let dir = tempfile::Builder::new()
            .prefix("tessel-share-test-")
            .tempdir_in(std::env::current_dir().unwrap())
            .unwrap();
        std::fs::write(dir.path().join("hello.txt"), "shared").unwrap();
        std::fs::write(dir.path().join(".env"), "SECRET").unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "owner rules").unwrap();
        let host = Host::start(
            dir.path(),
            "127.0.0.1:0".parse().unwrap(),
            "127.0.0.1".parse().unwrap(),
        )
        .unwrap();
        (dir, host)
    }

    #[test]
    fn real_tls_guest_reads_project_but_not_private_paths() {
        let (_dir, host) = fixture();
        let listed = request(&host.invitation, "list", "").unwrap();
        let names: Vec<&str> = listed["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["name"].as_str())
            .collect();
        assert!(names.contains(&"hello.txt"));
        assert!(!names.contains(&".env"));
        let read = request(&host.invitation, "read", "hello.txt").unwrap();
        assert_eq!(
            B64.decode(read["data"].as_str().unwrap()).unwrap(),
            b"shared"
        );
        for path in [".env", "../outside", "C:\\Windows\\win.ini"] {
            assert!(request(&host.invitation, "read", path).is_err());
        }
        assert!(request(&host.invitation, "execute", "").is_err());
    }

    #[test]
    fn invitation_is_revocable_and_wrong_token_fails() {
        let (_dir, host) = fixture();
        let invitation = Invitation::decode(&host.invitation.encode().unwrap()).unwrap();
        let mut wrong = invitation.clone();
        wrong.token = B64.encode([0; 32]);
        assert!(request(&wrong, "list", "").is_err());
        drop(host);
        assert!(request(&invitation, "list", "").is_err());
    }

    #[test]
    fn oversized_frame_rejected_before_allocation() {
        let mut data = std::io::Cursor::new(u32::MAX.to_be_bytes());
        assert!(read_frame(&mut data, MAX_REQUEST).is_err());
    }

    #[test]
    fn wrong_certificate_is_not_trusted() {
        let (_one, host) = fixture();
        let (_two, other) = fixture();
        let mut forged = host.invitation.clone();
        forged.certificate = other.invitation.certificate.clone();
        assert!(request(&forged, "list", "").is_err());
    }

    #[test]
    fn rights_presets_clean_unknown_fields_and_sas_derivation_is_deterministic() {
        assert_eq!(ShareRights::view_only().preset(), RightsPreset::ViewOnly);
        assert_eq!(
            ShareRights::collaborate().preset(),
            RightsPreset::Collaborate
        );
        assert_eq!(
            ShareRights::full_control().preset(),
            RightsPreset::FullControl
        );

        let cleaned = ShareRights::clean_from_value(Some(&json!({
            "edit": true,
            "pagesUse": true,
            "unknownRight": true,
            "type": "yes"
        })));
        assert!(cleaned.edit);
        assert!(!cleaned.r#type);
        assert_eq!(cleaned.enabled_count(), 1);
        assert_eq!(cleaned.preset(), RightsPreset::Custom);
        assert_eq!(cleaned.chip_text(false), "Custom · 1");
        assert_eq!(cleaned.chip_text(true), "Personnalisé · 1");

        let sas_a = derive_sas_code(b"cert-1", &[1u8; 32], &[2u8; 32]);
        let sas_b = derive_sas_code(b"cert-1", &[1u8; 32], &[2u8; 32]);
        let sas_c = derive_sas_code(b"cert-2", &[1u8; 32], &[2u8; 32]);
        assert_eq!(sas_a.len(), 7);
        assert_eq!(sas_a, sas_b);
        assert_ne!(sas_a, sas_c);
    }

    #[test]
    fn duplex_session_knock_sas_rights_enforcement_and_trusted_rejoin() {
        let (dir, host) = fixture();
        host.sync_workspace_state(
            vec![SharedProject {
                id: 1,
                name: "Demo".into(),
                path: dir.path().to_path_buf(),
            }],
            vec![SharedPaneSnapshot {
                id: 42,
                project_id: 1,
                title: "Ada".into(),
                is_agent: false,
                output: "prompt$ hello".into(),
            }],
            vec![SharedBrowserPage {
                project_id: Some(1),
                title: "Docs".into(),
                url: "https://example.com".into(),
            }],
            Some(1),
            Some(42),
        );

        let id_hex = "a1".repeat(32);
        let mut guest =
            GuestSession::connect(host.invitation.clone(), "Simpa".into(), id_hex.clone());

        // Wait for knock to arrive on host and match guest's SAS code!
        let start = Instant::now();
        let (ask_key, host_sas) = loop {
            let snap = host.snapshot();
            if let Some(LiveShareAsk::Join { key, code, .. }) = snap.asks.first() {
                break (key.clone(), code.clone());
            }
            assert!(start.elapsed() < Duration::from_secs(5), "knock timed out");
            thread::sleep(Duration::from_millis(20));
        };
        let guest_sas = loop {
            if let GuestPhase::WaitingForHost { sas_code, .. } = guest.snapshot().phase {
                break sas_code;
            }
            assert!(start.elapsed() < Duration::from_secs(5));
            thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(host_sas, guest_sas, "6-digit SAS must match identically");

        // Approve Simpa with `Collaborate` (edit/git/tasks/open/close/pagesView, NO `type`) + trust!
        host.answer_ask(AskAnswer::Join {
            key: ask_key,
            allow: true,
            projects: vec![1],
            rights: ShareRights::collaborate(),
            trust: true,
        });

        let wait_join = Instant::now();
        while !matches!(guest.snapshot().phase, GuestPhase::Joined { .. }) {
            assert!(wait_join.elapsed() < Duration::from_secs(5));
            thread::sleep(Duration::from_millis(20));
        }

        // 1. Terminal output works, but `term.write` MUST fail without `rights.type`!
        let out = guest
            .call_sync(
                json!({"op": "term.output", "paneId": 42}),
                Duration::from_secs(3),
            )
            .unwrap();
        assert_eq!(out["output"].as_str(), Some("prompt$ hello"));
        assert!(
            guest
                .call_sync(
                    json!({"op": "term.write", "paneId": 42, "data": B64.encode(b"ls\r")}),
                    Duration::from_secs(3),
                )
                .is_err()
        );

        // 2. Ordinary file edit succeeds with `Collaborate`, but `AGENTS.md` is refused without `type`!
        guest
            .call_sync(
                json!({"op": "files.write", "path": "hello.txt", "data": B64.encode(b"updated")}),
                Duration::from_secs(3),
            )
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("hello.txt")).unwrap(),
            "updated"
        );
        assert!(
            guest
                .call_sync(
                    json!({"op": "files.write", "path": "AGENTS.md", "data": B64.encode(b"hijack")}),
                    Duration::from_secs(3),
                )
                .is_err()
        );

        // 3. `scm.push` is ALWAYS refused even when `git` right is granted!
        assert!(
            guest
                .call_sync(json!({"op": "scm.push"}), Duration::from_secs(3))
                .is_err()
        );

        // 4. Grant `FullControl` (`type = true`): `term.write` forwards user input, strips CSI DA1 reply!
        let simpa_id = host.snapshot().guests[0].id.clone();
        host.set_subject_rights(&simpa_id, ShareRights::full_control());
        guest
            .call_sync(
                json!({"op": "term.write", "paneId": 42, "data": B64.encode(b"\x1b[?1;2c")}),
                Duration::from_secs(3),
            )
            .unwrap();
        assert!(
            host.drain_commands().is_empty(),
            "terminal DA1 reply must be stripped"
        );
        guest
            .call_sync(
                json!({"op": "term.write", "paneId": 42, "data": B64.encode(b"echo ok\r")}),
                Duration::from_secs(3),
            )
            .unwrap();
        let cmds = host.drain_commands();
        assert_eq!(cmds.len(), 1);
        assert!(!host.pane_presence(42).is_empty());

        guest.leave();

        // 5. Trusted rejoin: Simpa reconnects with a rotated invitation WITHOUT knocking!
        let root_inv_id = host.snapshot().invites[0].id.clone();
        let rotated = host.rotate_invite(&root_inv_id).unwrap();
        let mut trusted_guest = GuestSession::connect(rotated.invitation, "Simpa".into(), id_hex);
        let t_wait = Instant::now();
        while !matches!(trusted_guest.snapshot().phase, GuestPhase::Joined { .. }) {
            assert!(t_wait.elapsed() < Duration::from_secs(5));
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            host.snapshot().asks.is_empty(),
            "trusted peer must enter without knocking"
        );
        trusted_guest.leave();
    }
}

#[cfg(test)]
#[path = "share/guest_request_tests.rs"]
mod guest_request_tests;

#[cfg(test)]
#[path = "share/guest_shutdown_tests.rs"]
mod guest_shutdown_tests;

#[cfg(test)]
#[path = "share/knock_departure_tests.rs"]
mod knock_departure_tests;

#[path = "share/guest_git_dispatch.rs"]
mod guest_git_dispatch;
#[path = "share/guest_git_host.rs"]
mod guest_git_host;

#[path = "share/guest_git_config.rs"]
mod guest_git_config;

#[path = "share/guest_file_reads.rs"]
mod guest_file_reads;

#[path = "share/guest_file_mutations.rs"]
mod guest_file_mutations;

#[path = "share/protected_paths.rs"]
mod protected_paths;
