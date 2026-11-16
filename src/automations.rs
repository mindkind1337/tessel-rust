//! Local scheduled Automations engine (`<data_dir>/automations.json`).
//!
//! Security invariants:
//! - Never included in portable settings backups or cloud sync.
//! - Prompts are written to `<data_dir>/automations/runs/<run_id>/prompt.md` and
//!   never interpolated into shell command strings.
//! - Every automation computes a 64-bit FNV-1a permission signature (`confirmed_sig`)
//!   covering `(agent, project_path, isolation, schedule, prompt)`. Enabling an
//!   automation or clicking `Run now` requires explicit user confirmation whenever
//!   `confirmed_sig` does not match `current_signature()`.
//! - Custom 5-field POSIX cron schedules enforce a minimum interval of `>= 15` minutes.

use chrono::{Datelike, Local, Timelike};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const MAX_AUTOMATIONS: usize = 100;
pub const MAX_RUNS_PER_AUTOMATION: usize = 100;
pub const MAX_OPEN_PANES_PER_AUTOMATION: usize = 3;

fn default_max_concurrent() -> u8 {
    2
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutomationAgent {
    #[default]
    Claude,
    Codex,
    Gemini,
    Qwen,
}

impl AutomationAgent {
    pub const ALL: [Self; 4] = [Self::Claude, Self::Codex, Self::Gemini, Self::Qwen];

    #[must_use]
    pub const fn as_id(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::Qwen => "qwen",
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex CLI",
            Self::Gemini => "Gemini CLI",
            Self::Qwen => "Qwen Code",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutomationIsolation {
    #[default]
    Worktree,
    Project,
}

impl AutomationIsolation {
    #[must_use]
    pub const fn label(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::Worktree, false) => "Isolated Git worktree (recommended)",
            (Self::Worktree, true) => "Worktree Git isolé (recommandé)",
            (Self::Project, false) => "Project directory directly",
            (Self::Project, true) => "Dossier du projet directement",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AutomationSchedule {
    Hourly { minute: u8 },
    Daily { hour: u8, minute: u8 },
    Weekdays { hour: u8, minute: u8 },
    Weekly { weekday: u8, hour: u8, minute: u8 },
    Cron { expr: String },
}

impl Default for AutomationSchedule {
    fn default() -> Self {
        Self::Daily { hour: 9, minute: 0 }
    }
}

impl AutomationSchedule {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Hourly { minute } => {
                if *minute > 59 {
                    return Err("Invalid minute (0..=59).".into());
                }
                Ok(())
            }
            Self::Daily { hour, minute } | Self::Weekdays { hour, minute } => {
                if *hour > 23 || *minute > 59 {
                    return Err("Invalid time of day.".into());
                }
                Ok(())
            }
            Self::Weekly {
                weekday,
                hour,
                minute,
            } => {
                if *weekday > 6 || *hour > 23 || *minute > 59 {
                    return Err("Invalid weekly schedule.".into());
                }
                Ok(())
            }
            Self::Cron { expr } => validate_cron_min_15m(expr).map(|_| ()),
        }
    }

    #[must_use]
    pub fn summary(&self, french: bool) -> String {
        match self {
            Self::Hourly { minute } => {
                if french {
                    format!("Toutes les heures à :{minute:02}")
                } else {
                    format!("Hourly at :{minute:02}")
                }
            }
            Self::Daily { hour, minute } => {
                if french {
                    format!("Chaque jour à {hour:02}:{minute:02}")
                } else {
                    format!("Daily at {hour:02}:{minute:02}")
                }
            }
            Self::Weekdays { hour, minute } => {
                if french {
                    format!("Lun–Ven à {hour:02}:{minute:02}")
                } else {
                    format!("Weekdays at {hour:02}:{minute:02}")
                }
            }
            Self::Weekly {
                weekday,
                hour,
                minute,
            } => {
                let day = weekday_label(*weekday, french);
                if french {
                    format!("Chaque {day} à {hour:02}:{minute:02}")
                } else {
                    format!("Every {day} at {hour:02}:{minute:02}")
                }
            }
            Self::Cron { expr } => format!("Cron: {}", expr.trim()),
        }
    }

    #[must_use]
    pub fn matches(&self, dt: &chrono::DateTime<Local>) -> bool {
        let m = dt.minute() as u8;
        let h = dt.hour() as u8;
        // 0 = Sunday .. 6 = Saturday
        let dow = dt.weekday().num_days_from_sunday() as u8;
        let dom = dt.day() as u8;
        let month = dt.month() as u8;
        match self {
            Self::Hourly { minute } => m == *minute,
            Self::Daily { hour, minute } => h == *hour && m == *minute,
            Self::Weekdays { hour, minute } => (1..=5).contains(&dow) && h == *hour && m == *minute,
            Self::Weekly {
                weekday,
                hour,
                minute,
            } => dow == *weekday && h == *hour && m == *minute,
            Self::Cron { expr } => {
                let Ok(parsed) = validate_cron_min_15m(expr) else {
                    return false;
                };
                parsed.minutes[usize::from(m)]
                    && parsed.hours[usize::from(h)]
                    && parsed.days_of_month[usize::from(dom)]
                    && parsed.months[usize::from(month)]
                    && parsed.days_of_week[usize::from(dow)]
            }
        }
    }
}

#[must_use]
pub const fn weekday_label(weekday: u8, french: bool) -> &'static str {
    match (weekday % 7, french) {
        (0, false) => "Sunday",
        (0, true) => "dimanche",
        (1, false) => "Monday",
        (1, true) => "lundi",
        (2, false) => "Tuesday",
        (2, true) => "mardi",
        (3, false) => "Wednesday",
        (3, true) => "mercredi",
        (4, false) => "Thursday",
        (4, true) => "jeudi",
        (5, false) => "Friday",
        (5, true) => "vendredi",
        (_, false) => "Saturday",
        (_, true) => "samedi",
    }
}

pub struct ParsedCron {
    minutes: [bool; 60],
    hours: [bool; 24],
    days_of_month: [bool; 32],
    months: [bool; 13],
    days_of_week: [bool; 7],
}

/// Parses a 5-field POSIX cron expression and enforces a minimum interval of `>= 15` minutes.
pub fn validate_cron_min_15m(expr: &str) -> Result<ParsedCron, String> {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() != 5 {
        return Err("Cron expression must have 5 fields (minute hour dom month dow).".into());
    }
    let minutes_vec = parse_cron_field(fields[0], 0, 59)?;
    let hours_vec = parse_cron_field(fields[1], 0, 23)?;
    let dom_vec = parse_cron_field(fields[2], 1, 31)?;
    let month_vec = parse_cron_field(fields[3], 1, 12)?;
    let dow_raw = parse_cron_field(fields[4], 0, 7)?;

    // Enforce >= 15 minute minimum spacing across matched minutes within any hour/day boundary:
    let mut mins_sorted = minutes_vec.clone();
    mins_sorted.sort_unstable();
    mins_sorted.dedup();
    if mins_sorted.len() > 4 {
        return Err("Automation schedule cannot run more often than every 15 minutes.".into());
    }
    for pair in mins_sorted.windows(2) {
        if pair[1] - pair[0] < 15 {
            return Err("Automation schedule cannot run more often than every 15 minutes.".into());
        }
    }
    if mins_sorted.len() >= 2 && hours_vec.len() > 1 {
        let wrap_gap = (60 - u16::from(*mins_sorted.last().unwrap())) + u16::from(mins_sorted[0]);
        let mut hrs_sorted = hours_vec.clone();
        hrs_sorted.sort_unstable();
        let has_consecutive_hour = hrs_sorted.windows(2).any(|w| w[1] - w[0] == 1)
            || (hrs_sorted.contains(&23) && hrs_sorted.contains(&0));
        if has_consecutive_hour && wrap_gap < 15 {
            return Err("Automation schedule cannot run more often than every 15 minutes.".into());
        }
    }

    let mut minutes = [false; 60];
    for m in minutes_vec {
        minutes[usize::from(m)] = true;
    }
    let mut hours = [false; 24];
    for h in hours_vec {
        hours[usize::from(h)] = true;
    }
    let mut days_of_month = [false; 32];
    for d in dom_vec {
        days_of_month[usize::from(d)] = true;
    }
    let mut months = [false; 13];
    for mo in month_vec {
        months[usize::from(mo)] = true;
    }
    let mut days_of_week = [false; 7];
    for dw in dow_raw {
        days_of_week[usize::from(dw % 7)] = true;
    }

    Ok(ParsedCron {
        minutes,
        hours,
        days_of_month,
        months,
        days_of_week,
    })
}

fn parse_cron_field(field: &str, min: u8, max: u8) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for part in field.split(',') {
        let (range_part, step) = if let Some((r, s)) = part.split_once('/') {
            let st: u8 = s.parse().map_err(|_| format!("Invalid cron step '{s}'."))?;
            if st == 0 {
                return Err("Cron step cannot be 0.".into());
            }
            (r, st)
        } else {
            (part, 1u8)
        };
        let (start, end) = if range_part == "*" {
            (min, max)
        } else if let Some((a, b)) = range_part.split_once('-') {
            let sa: u8 = a
                .parse()
                .map_err(|_| format!("Invalid cron value '{a}'."))?;
            let eb: u8 = b
                .parse()
                .map_err(|_| format!("Invalid cron value '{b}'."))?;
            if sa < min || eb > max || sa > eb {
                return Err(format!(
                    "Cron range '{range_part}' out of bounds ({min}..={max})."
                ));
            }
            (sa, eb)
        } else {
            let v: u8 = range_part
                .parse()
                .map_err(|_| format!("Invalid cron value '{range_part}'."))?;
            if v < min || v > max {
                return Err(format!("Cron value '{v}' out of bounds ({min}..={max})."));
            }
            (v, v)
        };
        let mut cur = start;
        loop {
            out.push(cur);
            match cur.checked_add(step) {
                Some(next) if next <= end => cur = next,
                _ => break,
            }
        }
    }
    if out.is_empty() {
        return Err("Empty cron field.".into());
    }
    Ok(out)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutomationRunRecord {
    pub run_id: String,
    pub started_at_unix: u64,
    pub trigger: String,
    pub pane_id: Option<u64>,
    pub prompt_file: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Automation {
    pub id: String,
    pub name: String,
    pub project_id: u64,
    pub project_path: PathBuf,
    pub agent: AutomationAgent,
    pub isolation: AutomationIsolation,
    pub schedule: AutomationSchedule,
    pub prompt: String,
    pub enabled: bool,
    #[serde(default)]
    pub confirmed_sig: Option<String>,
    #[serde(default)]
    pub last_minute_slot: Option<i64>,
    #[serde(default)]
    pub runs: Vec<AutomationRunRecord>,
}

impl Automation {
    /// Computes the 64-bit FNV-1a permission fingerprint over all security-relevant fields.
    #[must_use]
    pub fn current_signature(&self) -> String {
        let sched_json = serde_json::to_string(&self.schedule).unwrap_or_default();
        let payload = format!(
            "v1|{}|{}|{:?}|{}|{}",
            self.agent.as_id(),
            self.project_path.to_string_lossy(),
            self.isolation,
            sched_json,
            self.prompt
        );
        fnv1a_hex(payload.as_bytes())
    }

    #[must_use]
    pub fn is_confirmed(&self) -> bool {
        self.confirmed_sig.as_deref() == Some(self.current_signature().as_str())
    }

    pub fn mark_confirmed(&mut self) {
        self.confirmed_sig = Some(self.current_signature());
    }

    pub fn validate(&self) -> Result<(), String> {
        let trimmed_name = self.name.trim();
        if trimmed_name.is_empty()
            || self.name.chars().count() > 100
            || self.name.chars().any(char::is_control)
        {
            return Err("Automation name must be 1–100 printable characters.".into());
        }
        if self.prompt.trim().is_empty() || self.prompt.len() > 32_768 || self.prompt.contains('\0')
        {
            return Err("Automation prompt must be non-empty (up to 32 KB).".into());
        }
        if !self.project_path.is_absolute() {
            return Err("Automation requires an absolute local project path.".into());
        }
        self.schedule.validate()
    }
}

fn fnv1a_hex(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutomationsFile {
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: u8,
    #[serde(default)]
    pub automations: Vec<Automation>,
}

impl Default for AutomationsFile {
    fn default() -> Self {
        Self {
            max_concurrent: 2,
            automations: Vec::new(),
        }
    }
}

impl AutomationsFile {
    #[must_use]
    pub fn load(data_dir: &Path) -> Self {
        let path = data_dir.join("automations.json");
        let mut loaded: Self = fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        loaded.max_concurrent = loaded.max_concurrent.clamp(1, 8);
        if loaded.automations.len() > MAX_AUTOMATIONS {
            loaded.automations.truncate(MAX_AUTOMATIONS);
        }
        for auto in &mut loaded.automations {
            if !auto.is_confirmed() {
                auto.enabled = false;
            }
            if auto.runs.len() > MAX_RUNS_PER_AUTOMATION {
                let drop_count = auto.runs.len() - MAX_RUNS_PER_AUTOMATION;
                auto.runs.drain(0..drop_count);
            }
        }
        loaded
    }

    pub fn save(&self, data_dir: &Path) -> Result<(), String> {
        fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        let path = data_dir.join("automations.json");
        let tmp = data_dir.join("automations.json.tmp");
        fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
        fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }
}

pub fn generate_automation_id(prefix: &str) -> Result<String, String> {
    generate_automation_id_with(prefix, |bytes| {
        getrandom::fill(bytes).map_err(|_| {
            "Secure random generation failed; no automation identifier was created.".into()
        })
    })
}

fn generate_automation_id_with(
    prefix: &str,
    fill: impl FnOnce(&mut [u8]) -> Result<(), String>,
) -> Result<String, String> {
    if !matches!(prefix, "auto" | "run") {
        return Err("Invalid automation identifier prefix.".into());
    }
    let mut raw = [0u8; 6];
    fill(&mut raw)?;
    let mut out = format!("{prefix}-");
    for b in raw {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{b:02x}");
    }
    Ok(out)
}

/// Writes the automation prompt safely to `<data_dir>/automations/runs/<run_id>/prompt.md`
/// so user prompt content is never interpolated into a shell command line.
pub fn write_automation_prompt_file(
    data_dir: &Path,
    run_id: &str,
    prompt: &str,
) -> Result<PathBuf, String> {
    if run_id.is_empty()
        || !run_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("Invalid automation run ID.".into());
    }
    let root = data_dir.join("automations").join("runs");
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let dir = root.join(run_id);
    fs::create_dir(&dir).map_err(|e| format!("Could not reserve a new automation run: {e}"))?;
    let file = dir.join("prompt.md");
    use std::io::Write as _;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&file)
        .map_err(|e| e.to_string())?;
    output
        .write_all(prompt.as_bytes())
        .map_err(|e| e.to_string())?;
    Ok(file)
}

/// Builds a shell-safe instruction line pointing the agent to read the `prompt.md` file.
#[must_use]
pub fn safe_prompt_instruction(prompt_file: &Path) -> String {
    let clean = prompt_file.to_string_lossy().replace('\'', "''");
    format!("Read the automation instructions in '{clean}' and execute them step by step.")
}

#[derive(Clone, Copy, Debug)]
pub struct AutomationTemplate {
    pub id: &'static str,
    pub title_en: &'static str,
    pub title_fr: &'static str,
    pub desc_en: &'static str,
    pub desc_fr: &'static str,
    pub default_hour: u8,
    pub prompt_en: &'static str,
    pub prompt_fr: &'static str,
}

pub const STARTER_TEMPLATES: [AutomationTemplate; 4] = [
    AutomationTemplate {
        id: "repoHealth",
        title_en: "Repository health check",
        title_fr: "Bilan de santé du dépôt",
        desc_en: "Run tests, linter, and summarize warnings or broken checks.",
        desc_fr: "Exécute les tests, le linter et résume les avertissements.",
        default_hour: 8,
        prompt_en: "Run the repository's test suite and static checks, summarize any failures or new warnings, and propose minimal fixes without pushing.",
        prompt_fr: "Exécute la suite de tests et les vérifications statiques du dépôt, résume les échecs ou avertissements et propose des correctifs minimaux sans pousser.",
    },
    AutomationTemplate {
        id: "releasePrep",
        title_en: "Release readiness notes",
        title_fr: "Préparation des notes de version",
        desc_en: "Summarize recent commits since the last tag into draft release notes.",
        desc_fr: "Résume les commits récents depuis le dernier tag en brouillon de notes.",
        default_hour: 17,
        prompt_en: "Inspect commits since the latest Git tag, group user-visible changes, fixes, and internal improvements, and draft concise release notes.",
        prompt_fr: "Inspecte les commits depuis le dernier tag Git, regroupe les nouveautés et correctifs visibles et rédige un brouillon concis de notes de version.",
    },
    AutomationTemplate {
        id: "recurringReview",
        title_en: "Recurring code & security audit",
        title_fr: "Audit récurrent code & sécurité",
        desc_en: "Audit recent changes for edge cases, regressions, or security risks.",
        desc_fr: "Analyse les changements récents pour détecter bogues ou failles.",
        default_hour: 9,
        prompt_en: "Review commits from the last 24 hours for potential bugs, race conditions, or security issues, and write a short report with file and line references.",
        prompt_fr: "Relis les commits des dernières 24 heures pour repérer bogues, conditions de course ou risques de sécurité, avec références de fichiers et lignes.",
    },
    AutomationTemplate {
        id: "maintenance",
        title_en: "Dependency & dead-code sweep",
        title_fr: "Maintenance dépendances & code mort",
        desc_en: "Check outdated dependencies and unused helpers in an isolated worktree.",
        desc_fr: "Vérifie les dépendances obsolètes et le code inutilisé dans un worktree.",
        default_hour: 10,
        prompt_en: "Check for outdated dependencies, compiler warnings, or unused modules, and list safe cleanup candidates.",
        prompt_fr: "Vérifie les dépendances obsolètes, avertissements du compilateur ou modules inutilisés et liste les nettoyages sûrs.",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cron_enforces_minimum_15_minute_interval() {
        assert!(validate_cron_min_15m("*/15 * * * *").is_ok());
        assert!(validate_cron_min_15m("0,30 * * * *").is_ok());
        assert!(validate_cron_min_15m("0 9 * * 1-5").is_ok());
        // Too frequent:
        assert!(validate_cron_min_15m("* * * * *").is_err());
        assert!(validate_cron_min_15m("*/5 * * * *").is_err());
        assert!(validate_cron_min_15m("0,10 * * * *").is_err());
        assert!(validate_cron_min_15m("55,5 * * * *").is_err());
    }

    #[test]
    fn signature_invalidates_when_prompt_or_isolation_changes() {
        let dir = tempfile::tempdir().unwrap();
        let mut auto = Automation {
            id: "auto-1".into(),
            name: "Daily check".into(),
            project_id: 1,
            project_path: dir.path().to_path_buf(),
            agent: AutomationAgent::Claude,
            isolation: AutomationIsolation::Worktree,
            schedule: AutomationSchedule::Daily { hour: 9, minute: 0 },
            prompt: "cargo test".into(),
            enabled: false,
            confirmed_sig: None,
            last_minute_slot: None,
            runs: Vec::new(),
        };
        assert!(!auto.is_confirmed());
        auto.mark_confirmed();
        assert!(auto.is_confirmed());

        auto.prompt = "cargo test --release".into();
        assert!(!auto.is_confirmed());
    }

    #[test]
    fn prompt_writer_persists_markdown_without_shell_interpolation() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_automation_prompt_file(dir.path(), "run-abc123", "Check $(whoami) & `id`")
            .unwrap();
        assert!(path.is_file());
        let saved = fs::read_to_string(&path).unwrap();
        assert_eq!(saved, "Check $(whoami) & `id`");
        let instr = safe_prompt_instruction(&path);
        assert!(!instr.contains("$(whoami)"));
        assert!(write_automation_prompt_file(dir.path(), "run-abc123", "Replacement").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), saved);
        let orphan = dir.path().join("automations/runs/run-orphan");
        fs::create_dir(&orphan).unwrap();
        assert!(write_automation_prompt_file(dir.path(), "run-orphan", "Replacement").is_err());
        assert!(!orphan.join("prompt.md").exists());
    }

    #[test]
    fn automation_identifier_entropy_errors_never_return_fallback_ids() {
        assert!(
            generate_automation_id_with("run", |bytes| {
                bytes.fill(0xA5);
                Err("Synthetic entropy failure".into())
            })
            .is_err()
        );
        assert!(
            generate_automation_id_with("User supplied name", |_| panic!("invalid prefix"))
                .is_err()
        );
        for prefix in ["auto", "run"] {
            assert_eq!(
                generate_automation_id_with(prefix, |bytes| {
                    bytes.fill(0xBB);
                    Ok(())
                })
                .unwrap(),
                format!("{prefix}-{}", "bb".repeat(6))
            );
        }
    }
}
