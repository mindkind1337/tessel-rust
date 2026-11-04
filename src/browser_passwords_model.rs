//! Pure, secret-free data models, RFC 4180 CSV scan, opaque import reconciliation,
//! Live Share guest vs. host-sharing per-page credential boundaries, DOM login-form
//! validation, search/pagination, bulk `allow_all` confirmations, and EN/FR summaries
//! for Saved Passwords (`BrowserPasswords.vue`, `browserPasswords.js`,
//! `browserPasswordManager.js`, `cookieImport/fileTokens.js`).
//!
//! Security invariants enforced by construction:
//! - No DTO, `Debug`, `Display`, or `Serialize` type in this module ever stores or
//!   formats a password.
//! - `parse_password_csv_with` borrows `&str` from a caller-owned `Zeroizing` buffer and
//!   processes the CSV strictly row-by-row (capped at [`MAX_CSV_COLUMNS`] columns), keeping
//!   only the current row's cells in `Zeroizing<String>`, passing `&str` to `map_secret`,
//!   and dropping/zeroizing each row's cells immediately before reading the next row.
//! - Import reconciliation compares secrets strictly through an injected opaque
//!   closure (`FnMut(SecretSlot<'_, S>, &S) -> bool`) and resets `allow_agents = false`
//!   whenever an existing login's secret changes.
//! - [`VerifiedPrivateHttpsOrigin`] is unforgeable outside this module (private field, no
//!   `Serialize`/`Deserialize`/`Default`) so Live Share and page-session checks must run
//!   both before vault lookup and again immediately before DOM injection.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashSet;
use url::Url;
use zeroize::Zeroizing;

/// Rows rendered per page in `BrowserPasswords.vue` (`const PAGE = 200`).
pub const PAGE_SIZE: usize = 200;

/// Maximum UTF-16 length accepted for a local CSV import path (`fileTokens.js`).
pub const MAX_IMPORT_PATH_UTF16: usize = 4096;

/// Hard per-row column ceiling during CSV scan to prevent comma-amplification DoS.
pub const MAX_CSV_COLUMNS: usize = 128;

/// Hard upper bound on candidate items consumed by a single [`plan_import`] call, preventing
/// unbounded iteration or CPU/memory amplification from arbitrary callers while leaving headroom
/// above two `LIMIT 5001` Chromium `Login Data` queries plus skipped/duplicate rows.
pub const MAX_IMPORT_BATCH_ITEMS: usize = 20_000;

/// UI language for Saved Passwords labels, confirmations, and import summaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PasswordsLang {
    #[default]
    En,
    Fr,
}

impl PasswordsLang {
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        match code.trim().to_ascii_lowercase().as_str() {
            "en" => Some(Self::En),
            "fr" => Some(Self::Fr),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Fr => "fr",
        }
    }
}

/// Capacity and field-size limits enforced during CSV scan and vault import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultLimits {
    pub max_entries: usize,
    pub max_csv_bytes: usize,
    pub max_csv_data_rows: usize,
    pub max_username_utf16: usize,
    pub max_username_bytes: usize,
    pub max_password_utf16: usize,
    pub max_password_bytes: usize,
    pub max_sealed_vault_bytes: usize,
}

impl VaultLimits {
    /// Exact limits of `C:\Tessel\src\main\browserPasswords.js`.
    pub const TESSEL_REFERENCE: Self = Self {
        max_entries: 5000,
        max_csv_bytes: 10 * 1024 * 1024,
        max_csv_data_rows: 5000,
        max_username_utf16: 2000,
        max_username_bytes: usize::MAX,
        max_password_utf16: 16384,
        max_password_bytes: usize::MAX,
        max_sealed_vault_bytes: 20 * 1024 * 1024,
    };

    /// Conservative limits aligned with `src/vault.rs`'s current DPAPI 5 MiB ceiling
    /// and per-record byte bounds (`username <= 1024`, `password <= 8192`, `<= 5000` logins).
    pub const NATIVE_VAULT: Self = Self {
        max_entries: 5000,
        max_csv_bytes: 5 * 1024 * 1024,
        max_csv_data_rows: 5000,
        max_username_utf16: 2000,
        max_username_bytes: 1024,
        max_password_utf16: 16384,
        max_password_bytes: 8192,
        max_sealed_vault_bytes: 5 * 1024 * 1024,
    };
}

impl Default for VaultLimits {
    fn default() -> Self {
        Self::NATIVE_VAULT
    }
}

/// Canonical wire error codes shared across vault, import, Live Share guards, and DOM fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VaultErrorCode {
    #[serde(rename = "private-page-required")]
    PrivatePageRequired,
    #[serde(rename = "encryption-unavailable")]
    EncryptionUnavailable,
    #[serde(rename = "vault-unavailable")]
    VaultUnavailable,
    #[serde(rename = "locked")]
    Locked,
    #[serde(rename = "access-denied")]
    AccessDenied,
    #[serde(rename = "no-sqlite")]
    NoSqlite,
    #[serde(rename = "unreadable")]
    Unreadable,
    #[serde(rename = "missing")]
    Missing,
    #[serde(rename = "not-found")]
    NotFound,
    #[serde(rename = "primary-password")]
    PrimaryPassword,
    #[serde(rename = "format")]
    Format,
    #[serde(rename = "too-big")]
    TooBig,
    #[serde(rename = "no-login-form")]
    NoLoginForm,
    #[serde(rename = "unsafe-form")]
    UnsafeForm,
    #[serde(rename = "ambiguous-form")]
    AmbiguousForm,
    #[serde(rename = "https-required")]
    HttpsRequired,
    #[serde(rename = "wrong-site")]
    WrongSite,
    #[serde(rename = "fill-failed")]
    FillFailed,
    #[serde(rename = "refused")]
    Refused,
    #[serde(rename = "invalid")]
    Invalid,
    #[serde(rename = "cancelled")]
    Cancelled,
    #[serde(rename = "failed")]
    Failed,
}

impl VaultErrorCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PrivatePageRequired => "private-page-required",
            Self::EncryptionUnavailable => "encryption-unavailable",
            Self::VaultUnavailable => "vault-unavailable",
            Self::Locked => "locked",
            Self::AccessDenied => "access-denied",
            Self::NoSqlite => "no-sqlite",
            Self::Unreadable => "unreadable",
            Self::Missing => "missing",
            Self::NotFound => "not-found",
            Self::PrimaryPassword => "primary-password",
            Self::Format => "format",
            Self::TooBig => "too-big",
            Self::NoLoginForm => "no-login-form",
            Self::UnsafeForm => "unsafe-form",
            Self::AmbiguousForm => "ambiguous-form",
            Self::HttpsRequired => "https-required",
            Self::WrongSite => "wrong-site",
            Self::FillFailed => "fill-failed",
            Self::Refused => "refused",
            Self::Invalid => "invalid",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }

    #[must_use]
    pub fn from_wire_code(code: &str) -> Self {
        match code {
            "private-page-required" => Self::PrivatePageRequired,
            "encryption-unavailable" => Self::EncryptionUnavailable,
            "vault-unavailable" => Self::VaultUnavailable,
            "locked" => Self::Locked,
            "access-denied" => Self::AccessDenied,
            "no-sqlite" => Self::NoSqlite,
            "unreadable" => Self::Unreadable,
            "missing" => Self::Missing,
            "not-found" => Self::NotFound,
            "primary-password" => Self::PrimaryPassword,
            "format" => Self::Format,
            "too-big" => Self::TooBig,
            "no-login-form" => Self::NoLoginForm,
            "unsafe-form" => Self::UnsafeForm,
            "ambiguous-form" => Self::AmbiguousForm,
            "https-required" => Self::HttpsRequired,
            "wrong-site" => Self::WrongSite,
            "fill-failed" => Self::FillFailed,
            "refused" => Self::Refused,
            "invalid" => Self::Invalid,
            "cancelled" => Self::Cancelled,
            _ => Self::Failed,
        }
    }

    /// Maps uncaught backend import/vault exceptions to the safe IPC allowlist in
    /// `browserPasswordManager.js:15-16` (everything else collapses to `Failed`).
    #[must_use]
    pub fn sanitize_ipc_exception_code(raw_code: Option<&str>) -> Self {
        match raw_code {
            Some("locked") => Self::Locked,
            Some("access-denied") => Self::AccessDenied,
            Some("missing") => Self::Missing,
            Some("no-sqlite") => Self::NoSqlite,
            Some("unreadable") => Self::Unreadable,
            Some("format") => Self::Format,
            Some("too-big") => Self::TooBig,
            Some("encryption-unavailable") => Self::EncryptionUnavailable,
            Some("vault-unavailable") => Self::VaultUnavailable,
            Some("not-found") => Self::NotFound,
            _ => Self::Failed,
        }
    }

    /// True when `BrowserPasswords.vue` (`CSV_FIX`) auto-opens the CSV export guide.
    #[must_use]
    pub const fn needs_csv_export(self) -> bool {
        matches!(
            self,
            Self::AccessDenied
                | Self::Locked
                | Self::Unreadable
                | Self::NoSqlite
                | Self::PrimaryPassword
        )
    }

    /// Localized user-facing message matching `BrowserPasswords.vue:32-52` and `fr/passwords.json`.
    #[must_use]
    pub const fn user_message(self, lang: PasswordsLang) -> &'static str {
        match (self, lang) {
            (Self::PrivatePageRequired, PasswordsLang::En) => {
                "Use a private browser tab on the host. Credentials cannot be used in shared tabs or joined Live Share sessions."
            }
            (Self::PrivatePageRequired, PasswordsLang::Fr) => {
                "Utilisez un onglet de navigateur privé sur l'hôte. Les identifiants ne peuvent pas être utilisés dans les onglets partagés ni dans une session Live Share rejointe comme invité."
            }
            (Self::EncryptionUnavailable, PasswordsLang::En) => {
                "Secure system encryption is unavailable. No passwords were saved."
            }
            (Self::EncryptionUnavailable, PasswordsLang::Fr) => {
                "Le chiffrement sécurisé du système est indisponible. Aucun mot de passe n'a été enregistré."
            }
            (Self::VaultUnavailable, PasswordsLang::En) => {
                "The encrypted vault could not be opened."
            }
            (Self::VaultUnavailable, PasswordsLang::Fr) => "Impossible d'ouvrir le coffre chiffré.",
            (Self::Locked, PasswordsLang::En) => {
                "The browser database is locked. Close that browser and retry, or import its CSV export."
            }
            (Self::Locked, PasswordsLang::Fr) => {
                "La base du navigateur est verrouillée. Fermez ce navigateur et réessayez, ou importez son export CSV."
            }
            (Self::AccessDenied, PasswordsLang::En) => {
                "Browser protection or permissions prevent access. Use a CSV export."
            }
            (Self::AccessDenied, PasswordsLang::Fr) => {
                "La protection du navigateur ou les permissions bloquent l'accès. Utilisez un export CSV."
            }
            (Self::NoSqlite, PasswordsLang::En) => {
                "This version cannot read browser databases. Update Tessel or import a CSV export."
            }
            (Self::NoSqlite, PasswordsLang::Fr) => {
                "Cette version ne peut pas lire les bases des navigateurs. Mettez Tessel à jour ou importez un export CSV."
            }
            (Self::Unreadable, PasswordsLang::En) => {
                "The browser database could not be read. Import a CSV export instead."
            }
            (Self::Unreadable, PasswordsLang::Fr) => {
                "Impossible de lire la base du navigateur. Importez plutôt un export CSV."
            }
            (Self::Missing | Self::NotFound, PasswordsLang::En) => {
                "The browser database no longer exists. Reopen this dialog and choose a profile."
            }
            (Self::Missing | Self::NotFound, PasswordsLang::Fr) => {
                "La base du navigateur n'existe plus. Rouvrez cette fenêtre et choisissez un profil."
            }
            (Self::PrimaryPassword, PasswordsLang::En) => {
                "This Firefox profile has a Primary Password set, so its saved passwords cannot be read here. Unlock it in Firefox, export its passwords as CSV, then import the CSV."
            }
            (Self::PrimaryPassword, PasswordsLang::Fr) => {
                "Ce profil Firefox a un mot de passe principal, ses mots de passe enregistrés ne peuvent donc pas être lus ici. Déverrouillez-le dans Firefox, exportez ses mots de passe en CSV, puis importez le CSV."
            }
            (Self::Format, PasswordsLang::En) => {
                "Expected a CSV export with url, username and password columns."
            }
            (Self::Format, PasswordsLang::Fr) => {
                "Le CSV doit contenir les colonnes url, username et password."
            }
            (Self::TooBig, PasswordsLang::En) => "Too many passwords: Tessel keeps up to 5000.",
            (Self::TooBig, PasswordsLang::Fr) => {
                "Trop de mots de passe : Tessel en garde jusqu'à 5000."
            }
            (Self::NoLoginForm, PasswordsLang::En) => {
                "No single visible password field was found. Open the login form first."
            }
            (Self::NoLoginForm, PasswordsLang::Fr) => {
                "Aucun champ de mot de passe visible et unique n'a été trouvé. Ouvrez d'abord le formulaire de connexion."
            }
            (Self::UnsafeForm, PasswordsLang::En) => {
                "This form sends credentials to another site or cannot be verified."
            }
            (Self::UnsafeForm, PasswordsLang::Fr) => {
                "Ce formulaire envoie les identifiants à un autre site ou ne peut pas être vérifié."
            }
            (Self::AmbiguousForm, PasswordsLang::En) => {
                "Several possible username fields were found. Fill this form manually."
            }
            (Self::AmbiguousForm, PasswordsLang::Fr) => {
                "Plusieurs champs d'identifiant possibles ont été trouvés. Remplissez ce formulaire manuellement."
            }
            (Self::HttpsRequired, PasswordsLang::En) => "Open the HTTPS login page first.",
            (Self::HttpsRequired, PasswordsLang::Fr) => {
                "Ouvrez d'abord la page de connexion HTTPS."
            }
            (
                Self::WrongSite
                | Self::FillFailed
                | Self::Refused
                | Self::Invalid
                | Self::Cancelled
                | Self::Failed,
                PasswordsLang::En,
            ) => "The operation could not be completed.",
            (
                Self::WrongSite
                | Self::FillFailed
                | Self::Refused
                | Self::Invalid
                | Self::Cancelled
                | Self::Failed,
                PasswordsLang::Fr,
            ) => "L'opération n'a pas pu être effectuée.",
        }
    }
}

/// Passwordless vault record exposed to UI and `browser_login` listings.
/// Contains strictly `{ id, origin, username, allowAgents }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultEntryMetadata {
    pub id: String,
    pub origin: String,
    pub username: String,
    pub allow_agents: bool,
}

/// Passwordless row rendered in the `BrowserPasswords` dialog list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultRowView {
    pub id: String,
    pub origin: String,
    pub site: String,
    pub username: String,
    pub allow_agents: bool,
    pub same: bool,
}

/// Metadata mutation requested by UI (`vault.change`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryMetadataChange {
    Delete,
    SetAllowAgents(bool),
}

/// Normalizes a login URL to its canonical ASCII `https://host[:port]` origin,
/// rejecting non-HTTPS schemes, opaque origins, and URLs with embedded credentials
/// (`loginOrigin` in `browserPasswords.js:13-15`).
#[must_use]
pub fn login_origin(raw: &str) -> Option<String> {
    // WHATWG URL strips leading/trailing ASCII C0/space, matching `new URL(raw)`.
    let parsed = Url::parse(raw).ok()?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return None;
    }
    let serialized = parsed.origin().ascii_serialization();
    (serialized != "null").then_some(serialized)
}

/// Display host[:port] without the leading `https://` (`BrowserPasswords.vue:190`).
#[must_use]
pub fn site_display_name(origin: &str) -> &str {
    origin.strip_prefix("https://").unwrap_or(origin)
}

/// Filters passwordless metadata entries (`vault.list(origin, agent)` in `browserPasswords.js:68-70`).
#[must_use]
pub fn vault_list_entries(
    entries: &[VaultEntryMetadata],
    origin_filter: Option<&str>,
    agent_only: bool,
) -> Vec<VaultEntryMetadata> {
    entries
        .iter()
        .filter(|r| origin_filter.is_none_or(|o| r.origin == o) && (!agent_only || r.allow_agents))
        .cloned()
        .collect()
}

/// Applies a delete or `allow_agents` toggle to the metadata list (`browserPasswords.js:88-96`).
pub fn apply_entry_change(
    entries: &[VaultEntryMetadata],
    id: &str,
    change: EntryMetadataChange,
) -> Result<Vec<VaultEntryMetadata>, VaultErrorCode> {
    if !entries.iter().any(|r| r.id == id) {
        return Err(VaultErrorCode::NotFound);
    }
    let mut next = Vec::with_capacity(entries.len());
    for entry in entries {
        if entry.id == id {
            match change {
                EntryMetadataChange::Delete => continue,
                EntryMetadataChange::SetAllowAgents(allow) => {
                    let mut updated = entry.clone();
                    updated.allow_agents = allow;
                    next.push(updated);
                }
            }
        } else {
            next.push(entry.clone());
        }
    }
    Ok(next)
}

/// Validates a local absolute file path chosen for CSV import (`isAllowedCookieFilePath`
/// in `src/main/cookieImport/fileTokens.js:12-17`). Rejects UNC shares (`\\server\share`),
/// Windows device paths (`\\.\`, `\\?\`), POSIX `//`, relative paths, `..` traversal, NUL,
/// and paths exceeding 4096 UTF-16 code units.
#[must_use]
pub fn is_allowed_local_import_file_path(path: &str, is_windows: bool) -> bool {
    if path.is_empty() || path.contains('\0') || utf16_len(path) > MAX_IMPORT_PATH_UTF16 {
        return false;
    }
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && is_sep(bytes[0]) && is_sep(bytes[1]) {
        return false;
    }
    if is_windows {
        if bytes.len() < 3
            || !bytes[0].is_ascii_alphabetic()
            || bytes[1] != b':'
            || !is_sep(bytes[2])
        {
            return false;
        }
        !has_dot_dot_segment(path, true)
    } else {
        path.starts_with('/') && !has_dot_dot_segment(path, false)
    }
}

fn is_sep(b: u8) -> bool {
    b == b'/' || b == b'\\'
}

fn has_dot_dot_segment(path: &str, windows_seps: bool) -> bool {
    let is_delim = |c: char| c == '/' || (windows_seps && c == '\\');
    // Check every `sep + ".."` followed by `sep` or EOF.
    for (byte_idx, ch) in path.char_indices() {
        if is_delim(ch) {
            let rest = &path[byte_idx + ch.len_utf8()..];
            if let Some(after_dots) = rest.strip_prefix("..")
                && (after_dots.is_empty() || after_dots.chars().next().is_some_and(is_delim))
            {
                return true;
            }
        }
    }
    false
}

#[must_use]
pub fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

fn cmp_utf16(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// Result of checking a candidate password's size bounds without retaining the password.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecretValidation {
    Valid,
    Empty,
    TooLong,
}

#[must_use]
pub fn validate_secret_bounds(secret: &str, limits: &VaultLimits) -> SecretValidation {
    if secret.is_empty() {
        SecretValidation::Empty
    } else if secret.len() > limits.max_password_bytes
        || utf16_len(secret) > limits.max_password_utf16
    {
        SecretValidation::TooLong
    } else {
        SecretValidation::Valid
    }
}

#[must_use]
pub fn validate_username_bounds(username: &str, limits: &VaultLimits) -> bool {
    username.len() <= limits.max_username_bytes && utf16_len(username) <= limits.max_username_utf16
}

/// Incoming import candidate carrying an opaque secret handle `S`.
/// Intentionally implements neither `Debug`, `Display`, nor `Serialize`.
pub struct ImportCandidate<S> {
    pub origin: Option<String>,
    pub username: String,
    pub secret: S,
}

/// Row-by-row streaming RFC 4180 CSV parser matching `parsePasswordCsv` (`browserPasswords.js:18-41`).
///
/// Memory & secret hygiene:
/// - Does not mutate or wipe the borrowed `csv_text: &str`; the runtime backend owns the raw CSV
///   buffer and must hold it in a `Zeroizing<String>` / `Zeroizing<Vec<u8>>`.
/// - Enforces [`MAX_CSV_COLUMNS`] per row so hostile comma-only files fail closed in `O(1)` cells
///   instead of amplifying into millions of heap strings.
/// - Holds only the current row's cells in `Zeroizing<String>`, resolves header indices in place
///   without non-zeroized copies, passes each data row's password slice to `map_secret`, and
///   immediately drops/zeroizes that row's cells before advancing to the next row.
pub fn parse_password_csv_with<S, F>(
    csv_text: &str,
    limits: &VaultLimits,
    mut map_secret: F,
) -> Result<Vec<ImportCandidate<S>>, VaultErrorCode>
where
    F: FnMut(&str) -> S,
{
    if csv_text.len() > limits.max_csv_bytes {
        return Err(VaultErrorCode::TooBig);
    }
    let text = csv_text.strip_prefix('\u{FEFF}').unwrap_or(csv_text);

    let mut header_cols: Option<(usize, usize, usize)> = None;
    let mut data_rows_seen = 0usize;
    let mut out: Vec<ImportCandidate<S>> = Vec::new();
    let mut row: Vec<Zeroizing<String>> = Vec::new();
    let mut field = Zeroizing::new(String::new());
    let mut quoted = false;

    let mut finish_row = |row: &mut Vec<Zeroizing<String>>| -> Result<(), VaultErrorCode> {
        if row.iter().any(|cell| !cell.is_empty()) {
            match header_cols {
                None => {
                    let url_idx = row
                        .iter()
                        .position(|h| h.trim().eq_ignore_ascii_case("url"));
                    let user_idx = row
                        .iter()
                        .position(|h| h.trim().eq_ignore_ascii_case("username"));
                    let pass_idx = row
                        .iter()
                        .position(|h| h.trim().eq_ignore_ascii_case("password"));
                    let (Some(u_col), Some(usr_col), Some(pw_col)) = (url_idx, user_idx, pass_idx)
                    else {
                        row.clear();
                        return Err(VaultErrorCode::Format);
                    };
                    header_cols = Some((u_col, usr_col, pw_col));
                }
                Some((url_col, user_col, pass_col)) => {
                    data_rows_seen += 1;
                    if data_rows_seen > limits.max_csv_data_rows {
                        row.clear();
                        return Err(VaultErrorCode::TooBig);
                    }
                    let origin = row
                        .get(url_col)
                        .and_then(|cell| login_origin(cell.as_str()));
                    let username = row
                        .get(user_col)
                        .map_or_else(String::new, |cell| cell.as_str().to_owned());
                    let password_ref = row.get(pass_col).map_or("", |cell| cell.as_str());
                    let secret = map_secret(password_ref);
                    out.push(ImportCandidate {
                        origin,
                        username,
                        secret,
                    });
                }
            }
        }
        row.clear();
        Ok(())
    };

    let mut chars = text.chars().peekable();
    loop {
        let current = chars.next();
        if quoted {
            match current {
                Some('"') => {
                    if chars.peek() == Some(&'"') {
                        let _ = chars.next();
                        field.push('"');
                    } else {
                        quoted = false;
                    }
                }
                Some(ch) => field.push(ch),
                None => return Err(VaultErrorCode::Format),
            }
        } else {
            match current {
                Some('"') if field.is_empty() => {
                    quoted = true;
                }
                Some(',') => {
                    if row.len() + 1 >= MAX_CSV_COLUMNS {
                        row.clear();
                        return Err(VaultErrorCode::Format);
                    }
                    row.push(std::mem::take(&mut field));
                }
                Some('\n') => {
                    if !row.is_empty() || !field.is_empty() {
                        if row.len() >= MAX_CSV_COLUMNS {
                            row.clear();
                            return Err(VaultErrorCode::Format);
                        }
                        row.push(std::mem::take(&mut field));
                        finish_row(&mut row)?;
                    }
                }
                None => {
                    if !row.is_empty() || !field.is_empty() {
                        if row.len() >= MAX_CSV_COLUMNS {
                            row.clear();
                            return Err(VaultErrorCode::Format);
                        }
                        row.push(std::mem::take(&mut field));
                        finish_row(&mut row)?;
                    }
                    break;
                }
                Some('\r') if chars.peek() == Some(&'\n') => {
                    // Skip CR in CRLF outside quotes; LF will close the row on next step.
                }
                Some(ch) => field.push(ch),
            }
        }
    }

    if header_cols.is_none() {
        return Err(VaultErrorCode::Format);
    }
    Ok(out)
}

/// Identifies which stored or in-batch secret is being compared during `plan_import`.
/// Intentionally implements neither `Debug`, `Display`, nor `Serialize`.
pub enum SecretSlot<'a, S> {
    Existing { entry_id: &'a str },
    Candidate(&'a S),
}

/// Mutation emitted by `plan_import` for the encrypted vault runtime to apply.
/// `allow_agents` is guaranteed to be `false` on both `Add` and `Update`.
/// Intentionally implements neither `Debug`, `Display`, nor `Serialize`.
pub enum PlannedVaultMutation<S> {
    Add {
        id: String,
        origin: String,
        username: String,
        allow_agents: bool,
        secret: S,
    },
    Update {
        id: String,
        origin: String,
        username: String,
        allow_agents: bool,
        secret: S,
    },
}

/// Passwordless import counters returned to callers and UI (`browserPasswords.js:86`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ImportOutcome {
    pub imported: usize,
    pub added: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub skipped: usize,
}

/// Pure reconciliation plan produced by `plan_import`.
/// Intentionally implements neither `Debug` nor `Serialize` because it carries `PlannedVaultMutation<S>`.
pub struct ImportPlan<S> {
    outcome: ImportOutcome,
    write_required: bool,
    next_entries: Vec<VaultEntryMetadata>,
    mutations: Vec<PlannedVaultMutation<S>>,
}

impl<S> ImportPlan<S> {
    #[must_use]
    pub const fn outcome(&self) -> ImportOutcome {
        self.outcome
    }

    #[must_use]
    pub const fn write_required(&self) -> bool {
        self.write_required
    }

    #[must_use]
    pub fn next_entries(&self) -> &[VaultEntryMetadata] {
        &self.next_entries
    }

    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        ImportOutcome,
        Vec<VaultEntryMetadata>,
        Vec<PlannedVaultMutation<S>>,
    ) {
        (self.outcome, self.next_entries, self.mutations)
    }
}

enum WorkingSecretState<S> {
    ExistingUnchanged,
    Added(S),
    Updated(S),
}

struct WorkingEntry<S> {
    meta: VaultEntryMetadata,
    secret_state: WorkingSecretState<S>,
}

/// Pure import planner matching `vault.importRows` (`browserPasswords.js:71-87`), hardened with
/// streaming in-flight bounds:
/// - Rejects corrupt `existing` snapshots (`len > limits.max_entries`, empty IDs, or duplicate IDs)
///   with [`VaultErrorCode::VaultUnavailable`].
/// - Bounds total consumed items at [`MAX_IMPORT_BATCH_ITEMS`] and bounds `working.len()` in-flight
///   at `limits.max_entries` before allocating or generating a new ID, failing closed with
///   [`VaultErrorCode::TooBig`].
/// - Rejects empty or colliding IDs returned by `next_id` with [`VaultErrorCode::Failed`].
/// - Skips invalid/non-HTTPS origins, empty/oversized secrets, and oversized usernames.
/// - Leaves identical `(origin, username, secret)` logins untouched (`unchanged++`),
///   preserving `allow_agents`.
/// - Replaces modified secrets (`updated++`) and **forces `allow_agents = false`**
///   (even if a later row in the same batch reverts the secret back to its earlier value).
pub fn plan_import<S, V, E, I>(
    existing: &[VaultEntryMetadata],
    incoming: impl IntoIterator<Item = ImportCandidate<S>>,
    limits: &VaultLimits,
    mut validate_secret: V,
    mut secrets_equal: E,
    mut next_id: I,
) -> Result<ImportPlan<S>, VaultErrorCode>
where
    V: FnMut(&S) -> SecretValidation,
    E: FnMut(SecretSlot<'_, S>, &S) -> bool,
    I: FnMut() -> String,
{
    if existing.len() > limits.max_entries {
        return Err(VaultErrorCode::VaultUnavailable);
    }

    let mut seen_ids: HashSet<String> = HashSet::with_capacity(existing.len());
    let mut working: Vec<WorkingEntry<S>> = Vec::with_capacity(existing.len());
    for meta in existing {
        if meta.id.is_empty() || !seen_ids.insert(meta.id.clone()) {
            return Err(VaultErrorCode::VaultUnavailable);
        }
        working.push(WorkingEntry {
            meta: meta.clone(),
            secret_state: WorkingSecretState::ExistingUnchanged,
        });
    }

    let mut items_seen = 0usize;
    let mut added = 0usize;
    let mut updated = 0usize;
    let mut unchanged = 0usize;
    let mut skipped = 0usize;

    for item in incoming {
        items_seen += 1;
        if items_seen > MAX_IMPORT_BATCH_ITEMS {
            return Err(VaultErrorCode::TooBig);
        }

        let Some(origin) = item.origin.as_deref().and_then(login_origin) else {
            skipped += 1;
            continue;
        };
        if !validate_username_bounds(&item.username, limits)
            || validate_secret(&item.secret) != SecretValidation::Valid
        {
            skipped += 1;
            continue;
        }

        if let Some(slot_entry) = working
            .iter_mut()
            .find(|r| r.meta.origin == origin && r.meta.username == item.username)
        {
            let is_same = match &slot_entry.secret_state {
                WorkingSecretState::ExistingUnchanged => secrets_equal(
                    SecretSlot::Existing {
                        entry_id: &slot_entry.meta.id,
                    },
                    &item.secret,
                ),
                WorkingSecretState::Added(prev) | WorkingSecretState::Updated(prev) => {
                    secrets_equal(SecretSlot::Candidate(prev), &item.secret)
                }
            };

            if is_same {
                unchanged += 1;
            } else {
                slot_entry.meta.allow_agents = false;
                updated += 1;
                slot_entry.secret_state = match slot_entry.secret_state {
                    WorkingSecretState::Added(_) => WorkingSecretState::Added(item.secret),
                    WorkingSecretState::ExistingUnchanged | WorkingSecretState::Updated(_) => {
                        WorkingSecretState::Updated(item.secret)
                    }
                };
            }
        } else {
            if working.len() >= limits.max_entries {
                return Err(VaultErrorCode::TooBig);
            }
            let id = next_id();
            if id.is_empty() || !seen_ids.insert(id.clone()) {
                return Err(VaultErrorCode::Failed);
            }
            working.push(WorkingEntry {
                meta: VaultEntryMetadata {
                    id,
                    origin,
                    username: item.username,
                    allow_agents: false,
                },
                secret_state: WorkingSecretState::Added(item.secret),
            });
            added += 1;
        }
    }

    let write_required = added > 0 || updated > 0;
    debug_assert!(working.len() <= limits.max_entries);

    let mut next_entries = Vec::with_capacity(working.len());
    let mut mutations = Vec::new();
    for entry in working {
        match entry.secret_state {
            WorkingSecretState::ExistingUnchanged => {
                next_entries.push(entry.meta);
            }
            WorkingSecretState::Added(secret) => {
                mutations.push(PlannedVaultMutation::Add {
                    id: entry.meta.id.clone(),
                    origin: entry.meta.origin.clone(),
                    username: entry.meta.username.clone(),
                    allow_agents: false,
                    secret,
                });
                next_entries.push(entry.meta);
            }
            WorkingSecretState::Updated(secret) => {
                mutations.push(PlannedVaultMutation::Update {
                    id: entry.meta.id.clone(),
                    origin: entry.meta.origin.clone(),
                    username: entry.meta.username.clone(),
                    allow_agents: false,
                    secret,
                });
                next_entries.push(entry.meta);
            }
        }
    }

    Ok(ImportPlan {
        outcome: ImportOutcome {
            imported: added + updated,
            added,
            updated,
            unchanged,
            skipped,
        },
        write_required,
        next_entries,
        mutations,
    })
}

// ============================================================================
// Live Share & Browser Session Credential Boundaries
// ============================================================================

/// Browser WebView partition kind (`browserGuest.js:41-54`, `index.js:4104-4105`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserSessionPartition {
    /// Owner's private browser session (`persist:tessel-browser`).
    OwnerPrivate,
    /// Agent-isolated browser session (`persist:tessel-browser-agent`).
    AgentIsolated,
    /// Ephemeral in-memory Live Share guest browser (`tessel-browser-guest`).
    LiveShareGuestEphemeral,
    /// Isolated preview/view partition (`VIEW_PARTITION`).
    ViewPreview,
    /// Any other foreign or unverified session.
    ExternalOrUnknown,
}

impl BrowserSessionPartition {
    #[must_use]
    pub const fn allows_saved_credentials(self) -> bool {
        matches!(self, Self::OwnerPrivate | Self::AgentIsolated)
    }
}

/// Share metadata of a browser tab on the host (`liveShare.js:3860-3862`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageShareDescriptor<'a> {
    pub project_id: &'a str,
    pub is_side_pane: bool,
    pub side_opt_in: bool,
}

/// Exact predicate of `liveShare.credentialsPageShared(id)` (`liveShare.js:3860-3862`):
/// Hosting does NOT disable the owner's local vault globally, nor does it block private
/// unshared projects or side browser panes that have not opted into sharing (`side && !opt_in`).
/// Only pages belonging to a shared project with `!is_side_pane || side_opt_in` are shared.
#[must_use]
pub fn is_browser_page_shared(
    host_sharing_projects: Option<&[&str]>,
    page: PageShareDescriptor<'_>,
) -> bool {
    let Some(shared_projects) = host_sharing_projects else {
        return false;
    };
    shared_projects.contains(&page.project_id) && (!page.is_side_pane || page.side_opt_in)
}

/// Guards local vault IPCs (`passwordSources`, `passwordList`, `passwordChange`,
/// `passwordImportProfile`, `passwordImportCsv` pre-check in `browserPasswordManager.js:8-13`).
/// Joined Live Share guests and untrusted senders fail closed with `PrivatePageRequired`.
pub const fn decide_vault_management_access(
    trusted_owner_sender: bool,
    joined_as_guest: bool,
) -> Result<(), VaultErrorCode> {
    if !trusted_owner_sender || joined_as_guest {
        Err(VaultErrorCode::PrivatePageRequired)
    } else {
        Ok(())
    }
}

/// Pre-flight facts gathered before reading a local CSV password export (`browserPasswordManager.js:26-34`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CsvImportPreflight<'a> {
    pub trusted_owner_sender: bool,
    pub joined_as_guest_before_dialog: bool,
    pub picked_path: Option<&'a str>,
    pub joined_as_guest_after_dialog: bool,
    pub is_windows: bool,
    pub is_regular_file: bool,
    pub file_size_bytes: u64,
}

/// Full pure pre-flight check for `browser:passwordImportCsv` (`browserPasswordManager.js:26-34`),
/// including post-dialog Live Share guest recheck (`line 29`), local path allowlist (`line 31`),
/// and byte-size cap (`line 33`).
pub fn decide_csv_import_file_access(
    preflight: &CsvImportPreflight<'_>,
    limits: &VaultLimits,
) -> Result<(), VaultErrorCode> {
    decide_vault_management_access(
        preflight.trusted_owner_sender,
        preflight.joined_as_guest_before_dialog,
    )?;
    let Some(path) = preflight.picked_path.filter(|p| !p.is_empty()) else {
        return Err(VaultErrorCode::Cancelled);
    };
    if preflight.joined_as_guest_after_dialog {
        return Err(VaultErrorCode::PrivatePageRequired);
    }
    if !is_allowed_local_import_file_path(path, preflight.is_windows) {
        return Err(VaultErrorCode::Refused);
    }
    if !preflight.is_regular_file || preflight.file_size_bytes > limits.max_csv_bytes as u64 {
        return Err(VaultErrorCode::TooBig);
    }
    Ok(())
}

/// Caller requesting page credential access (`browser:passwordFill` vs `browser_login`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialActor {
    OwnerUi { trusted_sender: bool },
    LocalAgent,
}

impl CredentialActor {
    #[must_use]
    pub const fn requires_agent_opt_in(self) -> bool {
        matches!(self, Self::LocalAgent)
    }
}

/// Runtime state of a target browser page evaluated before vault access and again before DOM fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageCredentialContext<'a> {
    pub actor: CredentialActor,
    pub joined_as_guest: bool,
    pub page_present_and_alive: bool,
    pub page_shared_in_live_share: bool,
    pub partition: BrowserSessionPartition,
    pub current_page_url: &'a str,
}

/// Unforgeable proof token carrying the normalized HTTPS origin of a private, non-shared browser tab.
///
/// Intentionally implements neither `Default`, `Serialize`, nor `Deserialize`: its sole constructor
/// is [`decide_page_credential_access`], preventing callers from forging or deserializing a proof
/// without passing Live Share, partition, liveness, and HTTPS origin checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedPrivateHttpsOrigin {
    https_origin: String,
}

impl VerifiedPrivateHttpsOrigin {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.https_origin
    }
}

/// Pure decision gating `browser:passwordFill` and `browser_login` (`index.js:4104-4105`,
/// `browserPasswordManager.js:36-43`, `browserPasswords.js:223-229`).
/// Callers must re-run this immediately before executing the isolated-world fill script.
pub fn decide_page_credential_access(
    ctx: &PageCredentialContext<'_>,
) -> Result<VerifiedPrivateHttpsOrigin, VaultErrorCode> {
    if let CredentialActor::OwnerUi {
        trusted_sender: false,
    } = ctx.actor
    {
        return Err(VaultErrorCode::PrivatePageRequired);
    }
    if ctx.joined_as_guest
        || !ctx.page_present_and_alive
        || ctx.page_shared_in_live_share
        || !ctx.partition.allows_saved_credentials()
    {
        return Err(VaultErrorCode::PrivatePageRequired);
    }
    let Some(https_origin) = login_origin(ctx.current_page_url) else {
        return Err(VaultErrorCode::HttpsRequired);
    };
    Ok(VerifiedPrivateHttpsOrigin { https_origin })
}

/// Diagnostic reason when `resolve_credential_target` rejects a credential ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialTargetFailure {
    NotFound,
    OriginMismatch,
    AgentAccessDisabled,
}

impl CredentialTargetFailure {
    /// Wire error code returned by `fillSavedPassword` (`browserPasswords.js:99, 235`).
    #[must_use]
    pub const fn wire_code(self) -> VaultErrorCode {
        VaultErrorCode::FillFailed
    }
}

/// Validates that `credential_id` exists, matches the verified page's exact HTTPS origin,
/// and (when `actor` is `LocalAgent`) has `allow_agents == true` (`browserPasswords.js:97-100`).
pub fn resolve_credential_target<'a>(
    entries: &'a [VaultEntryMetadata],
    credential_id: &str,
    verified_origin: &VerifiedPrivateHttpsOrigin,
    actor: CredentialActor,
) -> Result<&'a VaultEntryMetadata, CredentialTargetFailure> {
    let Some(entry) = entries.iter().find(|r| r.id == credential_id) else {
        return Err(CredentialTargetFailure::NotFound);
    };
    if entry.origin != verified_origin.as_str() {
        return Err(CredentialTargetFailure::OriginMismatch);
    }
    if actor.requires_agent_opt_in() && !entry.allow_agents {
        return Err(CredentialTargetFailure::AgentAccessDisabled);
    }
    Ok(entry)
}

// ============================================================================
// Pure DOM Login-Form Validation & Isolated-World Result Sanitization
// ============================================================================

/// Descriptor of an `<input>` inspected in the login form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DomInputDescriptor<'a> {
    pub input_type: &'a str,
    pub autocomplete: &'a str,
    pub disabled: bool,
    pub read_only: bool,
    pub visible: bool,
    /// Stable identity of `input.form` (`None` when not associated with any `<form>`).
    pub form_id: Option<usize>,
    /// Stable identity of the nearest ancestor `<form>` containing this element in the DOM tree
    /// (`input.closest("form")`), matching `form.querySelectorAll("input")` combined with
    /// `e.form === form` in `browserPasswords.js:235`.
    pub ancestor_form_id: Option<usize>,
}

/// Snapshot of page & form properties checked before filling any login field (`browserPasswords.js:209-238`).
///
/// Tracks stable `form_id` identity, requires top-frame execution (`is_top_frame`), connected
/// same-document form ownership (`form_connected && form_in_same_document`), raw attribute reads
/// (`getAttribute("action")` / `getAttribute("formaction")` immune to `<input name="action">`),
/// HTML5 `<base href>` resolution (`document_base_uri` for non-empty relative URLs, `location_href`
/// for missing/empty attributes), and all submitters whose `.form === form` (both inside `<form>`
/// and external controls associated via `form="..."`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DomFormEvaluationInput<'a> {
    pub expected_origin: &'a str,
    pub is_top_frame: bool,
    pub location_origin: &'a str,
    pub location_protocol: &'a str,
    pub location_href: &'a str,
    pub document_base_uri: &'a str,
    pub form_connected: bool,
    pub form_in_same_document: bool,
    pub form_action: Option<&'a str>,
    pub submit_form_actions: &'a [&'a str],
    pub inputs: &'a [DomInputDescriptor<'a>],
}

/// Selected `<form>` identity and input indices for non-submitting fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedLoginFields {
    pub form_id: usize,
    pub username_index: Option<usize>,
    pub password_index: usize,
}

/// Verifies `new URL(raw_target_attr || location_href, document_base_uri).origin === expected_origin`
/// (`browserPasswords.js:218-224`):
/// - Missing (`None`) or empty (`Some("")`) `action` / `formaction` resolves to the document URL
///   (`location_href`), matching HTML5 empty-action form submission semantics.
/// - Non-empty relative targets resolve against `document_base_uri`, blocking `<base href>` hijacks.
/// - Rejects cross-origin targets, HTTP downgrades, and `javascript:` / `data:` / `blob:` schemes.
#[must_use]
pub fn is_safe_login_form_action(
    raw_target_attr: Option<&str>,
    location_href: &str,
    document_base_uri: &str,
    expected_origin: &str,
) -> bool {
    let Ok(doc_url) = Url::parse(location_href) else {
        return false;
    };
    let Ok(base_url) = Url::parse(document_base_uri) else {
        return false;
    };
    let resolved = match raw_target_attr.filter(|a| !a.is_empty()) {
        None => Some(doc_url),
        Some(non_empty) => base_url.join(non_empty).ok(),
    };
    let Some(target) = resolved else {
        return false;
    };
    target.origin().ascii_serialization() == expected_origin
}

/// Verifies both `<form action>` (raw `getAttribute("action")`) and every associated `[formaction]`
/// element whose `.form === form` (inside `<form>` or external via `form="..."`) resolve to
/// `expected_origin` under `(location_href, document_base_uri)`.
#[must_use]
pub fn are_form_destinations_safe(
    form_action_attr: Option<&str>,
    submit_form_action_attrs: &[&str],
    location_href: &str,
    document_base_uri: &str,
    expected_origin: &str,
) -> bool {
    is_safe_login_form_action(
        form_action_attr,
        location_href,
        document_base_uri,
        expected_origin,
    ) && submit_form_action_attrs.iter().copied().all(|fa| {
        is_safe_login_form_action(Some(fa), location_href, document_base_uri, expected_origin)
    })
}

/// Pure hardened form-selection decision for non-submitting credential fill.
pub fn evaluate_dom_login_form(
    dom: &DomFormEvaluationInput<'_>,
) -> Result<SelectedLoginFields, VaultErrorCode> {
    if dom.location_origin != dom.expected_origin || dom.location_protocol != "https:" {
        return Err(VaultErrorCode::WrongSite);
    }
    if !dom.is_top_frame {
        return Err(VaultErrorCode::UnsafeForm);
    }

    let is_usable = |f: &DomInputDescriptor<'_>| !f.disabled && !f.read_only && f.visible;
    let passwords: Vec<usize> = dom
        .inputs
        .iter()
        .enumerate()
        .filter(|(_, f)| {
            is_usable(f)
                && f.input_type.eq_ignore_ascii_case("password")
                && f.autocomplete != "new-password"
        })
        .map(|(idx, _)| idx)
        .collect();

    if passwords.len() != 1 {
        return Err(VaultErrorCode::NoLoginForm);
    }
    let password_index = passwords[0];

    let Some(form_id) = dom.inputs[password_index].form_id else {
        return Err(VaultErrorCode::UnsafeForm);
    };
    if !dom.form_connected
        || !dom.form_in_same_document
        || !are_form_destinations_safe(
            dom.form_action,
            dom.submit_form_actions,
            dom.location_href,
            dom.document_base_uri,
            dom.expected_origin,
        )
    {
        return Err(VaultErrorCode::UnsafeForm);
    }

    let candidates: Vec<usize> = dom
        .inputs
        .iter()
        .enumerate()
        .filter(|(_, f)| {
            f.ancestor_form_id == Some(form_id)
                && f.form_id == Some(form_id)
                && is_usable(f)
                && (f.input_type.eq_ignore_ascii_case("text")
                    || f.input_type.eq_ignore_ascii_case("email"))
        })
        .map(|(idx, _)| idx)
        .collect();

    let named: Vec<usize> = candidates
        .iter()
        .copied()
        .filter(|&idx| {
            let f = &dom.inputs[idx];
            f.autocomplete == "username" || f.input_type.eq_ignore_ascii_case("email")
        })
        .collect();

    let username_index = if named.len() == 1 {
        Some(named[0])
    } else if candidates.len() == 1 {
        Some(candidates[0])
    } else {
        None
    };

    if username_index.is_none() && !candidates.is_empty() {
        return Err(VaultErrorCode::AmbiguousForm);
    }

    Ok(SelectedLoginFields {
        form_id,
        username_index,
        password_index,
    })
}

/// Live DOM state re-read *after* `username` `input`/`change` events and immediately before
/// writing `password.value` (`browserPasswords.js:244`). Must be read from the live `password`
/// element, live `document.baseURI`, and live `password.form`, never from pre-event cached values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DomPasswordReverifyInput<'a> {
    pub expected_origin: &'a str,
    pub expected_form_id: usize,
    pub is_top_frame: bool,
    pub location_origin: &'a str,
    pub location_protocol: &'a str,
    pub location_href: &'a str,
    pub document_base_uri: &'a str,
    pub password_connected: bool,
    pub password_disabled: bool,
    pub password_read_only: bool,
    pub password_visible: bool,
    pub password_input_type: &'a str,
    pub current_password_form_id: Option<usize>,
    pub current_form_connected: bool,
    pub current_form_in_same_document: bool,
    pub current_form_action: Option<&'a str>,
    pub current_submit_form_actions: &'a [&'a str],
}

/// Re-verifies frame, origin, `document.baseURI`, password input state, stable `<form>` identity
/// (`password.form` not reparented or detached during `username` events), form connectedness and
/// `ownerDocument === document`, and all current `<form action>` / `[formaction]` destinations
/// immediately before setting the password value.
pub fn reverify_dom_before_password_fill(
    reverify: &DomPasswordReverifyInput<'_>,
) -> Result<(), VaultErrorCode> {
    if !reverify.is_top_frame
        || reverify.location_origin != reverify.expected_origin
        || reverify.location_protocol != "https:"
        || !reverify.password_connected
        || reverify.password_disabled
        || reverify.password_read_only
        || !reverify.password_visible
        || !reverify
            .password_input_type
            .eq_ignore_ascii_case("password")
        || reverify.current_password_form_id != Some(reverify.expected_form_id)
        || !reverify.current_form_connected
        || !reverify.current_form_in_same_document
        || !are_form_destinations_safe(
            reverify.current_form_action,
            reverify.current_submit_form_actions,
            reverify.location_href,
            reverify.document_base_uri,
            reverify.expected_origin,
        )
    {
        return Err(VaultErrorCode::UnsafeForm);
    }
    Ok(())
}

/// Non-submitting fill outcome (`{ ok: true, submitted: false }`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FillCompleted {
    pub submitted: bool,
}

/// Sanitizes untrusted script evaluation output from the page (`browserPasswords.js:231-234`):
/// never relays arbitrary page strings or exception messages.
pub fn sanitize_dom_fill_result(
    ok: Option<bool>,
    raw_code: Option<&str>,
) -> Result<FillCompleted, VaultErrorCode> {
    if ok == Some(true) {
        return Ok(FillCompleted { submitted: false });
    }
    Err(match raw_code {
        Some("wrong-site") => VaultErrorCode::WrongSite,
        Some("no-login-form") => VaultErrorCode::NoLoginForm,
        Some("unsafe-form") => VaultErrorCode::UnsafeForm,
        Some("ambiguous-form") => VaultErrorCode::AmbiguousForm,
        _ => VaultErrorCode::FillFailed,
    })
}

// ============================================================================
// UI View Projection, Search, Pagination, Confirmations & EN/FR Summaries
// ============================================================================

/// Keyboard `Escape` behavior inside the Saved Passwords modal (`BrowserPasswords.vue:213-216, 223`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeAction {
    ClearSearchQuery,
    CloseModal,
}

#[must_use]
pub const fn decide_escape_action(current_query: &str) -> EscapeAction {
    if current_query.is_empty() {
        EscapeAction::CloseModal
    } else {
        EscapeAction::ClearSearchQuery
    }
}

/// State of the sticky header `allow_all` switch (`BrowserPasswords.vue:148-153, 268-278`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkSwitchState {
    pub checked: bool,
    pub indeterminate: bool,
    pub disabled: bool,
    pub label: String,
}

/// Complete derived view state for `BrowserPasswords.vue`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasswordsDialogView {
    pub total_rows: usize,
    pub filtered_count: usize,
    pub visible_rows: Vec<VaultRowView>,
    pub remaining: usize,
    pub allowed_count: usize,
    pub count_label: String,
    pub agents_label: String,
    pub no_match_label: String,
    pub more_label: String,
    pub bulk_switch: BulkSwitchState,
}

struct SortedRow {
    id: String,
    origin: String,
    site: String,
    username: String,
    allow_agents: bool,
    search_key: String,
}

fn sorted_rows(entries: &[VaultEntryMetadata]) -> Vec<SortedRow> {
    let mut rows: Vec<SortedRow> = entries
        .iter()
        .map(|e| {
            let site = site_display_name(&e.origin).to_owned();
            let username = e.username.clone();
            let search_key = format!("{site}\n{username}").to_lowercase();
            SortedRow {
                id: e.id.clone(),
                origin: e.origin.clone(),
                site,
                username,
                allow_agents: e.allow_agents,
                search_key,
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        cmp_utf16(&a.site, &b.site).then_with(|| cmp_utf16(&a.username, &b.username))
    });
    rows
}

fn filter_sorted_rows<'a>(rows: &'a [SortedRow], query: &str) -> (Vec<String>, Vec<&'a SortedRow>) {
    let terms: Vec<String> = query
        .trim()
        .to_lowercase()
        .split_whitespace()
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect();
    let filtered = if terms.is_empty() {
        rows.iter().collect()
    } else {
        rows.iter()
            .filter(|r| terms.iter().all(|w| r.search_key.contains(w.as_str())))
            .collect()
    };
    (terms, filtered)
}

/// French `Intl.PluralRules('fr')` cardinal rule: `0` and `1` select `_one`; `>= 2` selects `_other`.
const fn fr_is_one(count: usize) -> bool {
    count <= 1
}

/// Computes sorted, searched, paginated rows and localized labels (`BrowserPasswords.vue:148-211`).
#[must_use]
pub fn build_dialog_view(
    entries: &[VaultEntryMetadata],
    query: &str,
    limit: usize,
    busy: bool,
    lang: PasswordsLang,
) -> PasswordsDialogView {
    let rows = sorted_rows(entries);
    let total_rows = rows.len();
    let allowed_count = rows.iter().filter(|r| r.allow_agents).count();
    let (terms, filtered) = filter_sorted_rows(&rows, query);
    let filtered_count = filtered.len();

    let capped = limit.min(filtered_count);
    let mut visible_rows = Vec::with_capacity(capped);
    for (idx, row) in filtered.iter().take(capped).enumerate() {
        let same = idx > 0 && filtered[idx - 1].site == row.site;
        visible_rows.push(VaultRowView {
            id: row.id.clone(),
            origin: row.origin.clone(),
            site: row.site.clone(),
            username: row.username.clone(),
            allow_agents: row.allow_agents,
            same,
        });
    }
    let remaining = filtered_count.saturating_sub(visible_rows.len());

    let all_shown_allowed = filtered_count > 0 && filtered.iter().all(|r| r.allow_agents);
    let some_shown_allowed = filtered.iter().any(|r| r.allow_agents);

    let count_label = if terms.is_empty() {
        match lang {
            PasswordsLang::En => format!("{total_rows} accounts"),
            PasswordsLang::Fr if fr_is_one(total_rows) => format!("{total_rows} compte"),
            PasswordsLang::Fr => format!("{total_rows} comptes"),
        }
    } else {
        match lang {
            PasswordsLang::En => format!("{filtered_count} of {total_rows}"),
            PasswordsLang::Fr => format!("{filtered_count} sur {total_rows}"),
        }
    };

    let agents_label = match lang {
        PasswordsLang::En => format!("{allowed_count} allowed for agents"),
        PasswordsLang::Fr if fr_is_one(allowed_count) => {
            format!("{allowed_count} autorisé pour les agents")
        }
        PasswordsLang::Fr => format!("{allowed_count} autorisés pour les agents"),
    };

    let trimmed_query = query.trim();
    let no_match_label = match lang {
        PasswordsLang::En => format!("No account matches “{trimmed_query}”."),
        PasswordsLang::Fr => format!("Aucun compte ne correspond à « {trimmed_query} »."),
    };

    let more_label = match lang {
        PasswordsLang::En => format!("Show more ({remaining} left)"),
        PasswordsLang::Fr => format!("Afficher plus ({remaining} restants)"),
    };

    let bulk_label = if all_shown_allowed {
        match lang {
            PasswordsLang::En => {
                format!("Forbid my local agents every account shown ({filtered_count})")
            }
            PasswordsLang::Fr => format!(
                "Retirer l'accès de mes agents locaux à tous les comptes affichés ({filtered_count})"
            ),
        }
    } else {
        match lang {
            PasswordsLang::En => {
                format!("Allow my local agents every account shown ({filtered_count})")
            }
            PasswordsLang::Fr => format!(
                "Autoriser mes agents locaux à utiliser tous les comptes affichés ({filtered_count})"
            ),
        }
    };

    PasswordsDialogView {
        total_rows,
        filtered_count,
        visible_rows,
        remaining,
        allowed_count,
        count_label,
        agents_label,
        no_match_label,
        more_label,
        bulk_switch: BulkSwitchState {
            checked: all_shown_allowed,
            indeterminate: some_shown_allowed && !all_shown_allowed,
            disabled: busy || filtered_count == 0,
            label: bulk_label,
        },
    }
}

/// Confirmation modal payload matching `askConfirm` in `BrowserPasswords.vue`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmationPrompt {
    pub title: String,
    pub text: String,
    pub confirm_label: String,
    pub danger: bool,
}

/// Decision for the header bulk `allow_all` toggle (`BrowserPasswords.vue:154-167`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BulkAllowDecision {
    NoOp,
    /// Enabling agent access in bulk is refused fail-closed if no confirmation UI is available.
    RefusedWithoutConfirmation,
    /// Disabling agent access in bulk runs immediately without prompting.
    ApplyImmediately {
        target_ids: Vec<String>,
        allow_agents: bool,
    },
    /// Enabling agent access in bulk prompts with the exact count of accounts being newly allowed.
    AskConfirmation {
        target_ids: Vec<String>,
        prompt: ConfirmationPrompt,
    },
}

/// Plans the bulk header `allow_all` action across ALL search-matching entries (not just the 200 drawn).
#[must_use]
pub fn plan_bulk_allow_change(
    entries: &[VaultEntryMetadata],
    query: &str,
    target_allow: bool,
    can_confirm: bool,
    lang: PasswordsLang,
) -> BulkAllowDecision {
    let rows = sorted_rows(entries);
    let (_, filtered) = filter_sorted_rows(&rows, query);
    let target_ids: Vec<String> = filtered
        .into_iter()
        .filter(|r| r.allow_agents != target_allow)
        .map(|r| r.id.clone())
        .collect();

    if target_ids.is_empty() {
        return BulkAllowDecision::NoOp;
    }

    if target_allow {
        if !can_confirm {
            return BulkAllowDecision::RefusedWithoutConfirmation;
        }
        let count = target_ids.len();
        let prompt = match lang {
            PasswordsLang::En => ConfirmationPrompt {
                title: format!("Allow {count} accounts for your local agents?"),
                text: "Agents running on this computer will be able to sign in with these logins in Tessel’s browser. You can turn them off again here, all at once or one by one.".to_owned(),
                confirm_label: format!("Allow {count}"),
                danger: true,
            },
            PasswordsLang::Fr => ConfirmationPrompt {
                title: format!("Autoriser {count} comptes pour vos agents locaux ?"),
                text: "Les agents qui tournent sur cet ordinateur pourront se connecter avec ces identifiants dans le navigateur de Tessel. Vous pouvez les retirer ici, tous d'un coup ou un par un.".to_owned(),
                confirm_label: format!("Autoriser {count}"),
                danger: true,
            },
        };
        BulkAllowDecision::AskConfirmation { target_ids, prompt }
    } else {
        BulkAllowDecision::ApplyImmediately {
            target_ids,
            allow_agents: false,
        }
    }
}

/// Formats the bulk completion status line (`BrowserPasswords.vue:178-180`).
#[must_use]
pub fn format_bulk_change_result(
    done: usize,
    failed: usize,
    lang: PasswordsLang,
) -> (String, bool) {
    if failed > 0 {
        let msg = match lang {
            PasswordsLang::En => format!("{done} changed, {failed} could not be changed."),
            PasswordsLang::Fr => format!("{done} modifiés, {failed} n'ont pas pu l'être."),
        };
        (msg, true)
    } else {
        let msg = match lang {
            PasswordsLang::En => format!("{done} accounts changed."),
            PasswordsLang::Fr => format!("{done} comptes modifiés."),
        };
        (msg, false)
    }
}

#[must_use]
pub const fn no_username_placeholder(lang: PasswordsLang) -> &'static str {
    match lang {
        PasswordsLang::En => "(no username)",
        PasswordsLang::Fr => "(sans nom d'utilisateur)",
    }
}

/// Single-login removal confirmation (`BrowserPasswords.vue:133-141`).
#[must_use]
pub fn plan_remove_confirmation(
    site: &str,
    username: &str,
    lang: PasswordsLang,
) -> ConfirmationPrompt {
    let user = if username.is_empty() {
        no_username_placeholder(lang)
    } else {
        username
    };
    match lang {
        PasswordsLang::En => ConfirmationPrompt {
            title: "Remove this login from Tessel?".to_owned(),
            text: format!("{user} on {site}. It stays saved in your browser."),
            confirm_label: "Remove".to_owned(),
            danger: true,
        },
        PasswordsLang::Fr => ConfirmationPrompt {
            title: "Retirer cette connexion de Tessel ?".to_owned(),
            text: format!("{user} sur {site}. Elle reste enregistrée dans votre navigateur."),
            confirm_label: "Retirer".to_owned(),
            danger: true,
        },
    }
}

#[must_use]
pub fn row_allow_aria_label(site: &str, username: &str, lang: PasswordsLang) -> String {
    let user = if username.is_empty() {
        no_username_placeholder(lang)
    } else {
        username
    };
    match lang {
        PasswordsLang::En => format!("Allow my local agents to use {user} on {site}"),
        PasswordsLang::Fr => format!("Autoriser mes agents locaux à utiliser {user} sur {site}"),
    }
}

#[must_use]
pub fn row_remove_aria_label(site: &str, username: &str, lang: PasswordsLang) -> String {
    let user = if username.is_empty() {
        no_username_placeholder(lang)
    } else {
        username
    };
    match lang {
        PasswordsLang::En => format!("Remove {user} on {site} from Tessel"),
        PasswordsLang::Fr => format!("Retirer {user} sur {site} de Tessel"),
    }
}

/// Input counters/status for single-profile or CSV import summary (`BrowserPasswords.vue:58-70`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ImportReportInput {
    pub ok: bool,
    pub error_code: Option<VaultErrorCode>,
    pub imported: Option<usize>,
    pub added: Option<usize>,
    pub updated: usize,
    pub unchanged: usize,
    pub skipped: usize,
    pub app_bound_count: usize,
    pub protected_count: usize,
}

/// True when `BrowserPasswords.vue` (`needsCsv`) auto-expands the CSV export instructions.
#[must_use]
pub fn needs_csv_help(report: &ImportReportInput) -> bool {
    report.protected_count > 0
        || report.app_bound_count > 0
        || report
            .error_code
            .is_some_and(VaultErrorCode::needs_csv_export)
}

/// Formats the localized one-line import summary (`BrowserPasswords.vue:58-70`).
#[must_use]
pub fn format_import_summary(report: &ImportReportInput, lang: PasswordsLang) -> String {
    if !report.ok {
        return report
            .error_code
            .unwrap_or(VaultErrorCode::Failed)
            .user_message(lang)
            .to_owned();
    }

    let mut parts: Vec<String> = Vec::new();
    if let Some(added) = report.added {
        parts.push(match lang {
            PasswordsLang::En => format!("{added} new"),
            PasswordsLang::Fr if fr_is_one(added) => format!("{added} nouveau"),
            PasswordsLang::Fr => format!("{added} nouveaux"),
        });
        if report.updated > 0 {
            let n = report.updated;
            parts.push(match lang {
                PasswordsLang::En => format!("{n} updated (agent access turned off again)"),
                PasswordsLang::Fr => {
                    format!("{n} mis à jour (accès des agents désactivé de nouveau)")
                }
            });
        }
        if report.unchanged > 0 {
            let n = report.unchanged;
            parts.push(match lang {
                PasswordsLang::En => format!("{n} already saved"),
                PasswordsLang::Fr if fr_is_one(n) => format!("{n} déjà enregistré"),
                PasswordsLang::Fr => format!("{n} déjà enregistrés"),
            });
        }
    } else {
        let imported = report.imported.unwrap_or(0);
        parts.push(match lang {
            PasswordsLang::En => format!("{imported} imported"),
            PasswordsLang::Fr if fr_is_one(imported) => format!("{imported} importé"),
            PasswordsLang::Fr => format!("{imported} importés"),
        });
    }

    if report.skipped > 0 {
        let n = report.skipped;
        parts.push(match lang {
            PasswordsLang::En => {
                format!("{n} skipped (not an HTTPS site, no password or unreadable)")
            }
            PasswordsLang::Fr if fr_is_one(n) => {
                format!("{n} ignoré (site non HTTPS, sans mot de passe ou illisible)")
            }
            PasswordsLang::Fr => {
                format!("{n} ignorés (site non HTTPS, sans mot de passe ou illisibles)")
            }
        });
    }

    if report.app_bound_count > 0 {
        let n = report.app_bound_count;
        parts.push(match lang {
            PasswordsLang::En => format!(
                "{n} locked by App-Bound Encryption (Chrome/Edge 127+): import a CSV export"
            ),
            PasswordsLang::Fr if fr_is_one(n) => format!(
                "{n} verrouillé par App-Bound Encryption (Chrome/Edge 127+) : importez un export CSV"
            ),
            PasswordsLang::Fr => format!(
                "{n} verrouillés par App-Bound Encryption (Chrome/Edge 127+) : importez un export CSV"
            ),
        });
    }

    if report.protected_count > 0 {
        let n = report.protected_count;
        parts.push(match lang {
            PasswordsLang::En => format!("{n} protected and unreadable: import a CSV export"),
            PasswordsLang::Fr if fr_is_one(n) => {
                format!("{n} protégé et illisible : importez un export CSV")
            }
            PasswordsLang::Fr => format!("{n} protégés et illisibles : importez un export CSV"),
        });
    }

    parts.join(" · ")
}

/// Input row for `evaluate_import_all_batch` (`BrowserPasswords.vue:110-128`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchProfileAttempt {
    pub label: String,
    pub report: ImportReportInput,
}

/// Per-profile result row displayed in `.pw-results` after `Import from all browsers`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchProfileResultRow {
    pub index: usize,
    pub label: String,
    pub ok: bool,
    pub warn: bool,
    pub csv: bool,
    pub text: String,
}

/// Aggregated report for `Import from all browsers` (`BrowserPasswords.vue:110-128`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchImportReport {
    pub title: String,
    pub total_added: usize,
    pub attention_count: usize,
    pub items: Vec<BatchProfileResultRow>,
}

/// Evaluates sequential multi-browser profile import results (`importAll` in `BrowserPasswords.vue`).
#[must_use]
pub fn evaluate_import_all_batch(
    attempts: &[BatchProfileAttempt],
    lang: PasswordsLang,
) -> BatchImportReport {
    let mut total_added = 0usize;
    let mut attention_count = 0usize;
    let mut items = Vec::with_capacity(attempts.len());

    for (index, attempt) in attempts.iter().enumerate() {
        let r = &attempt.report;
        if r.ok {
            total_added += r.added.or(r.imported).unwrap_or(0);
        }
        let warn = !r.ok || r.app_bound_count > 0 || r.protected_count > 0;
        if warn {
            attention_count += 1;
        }
        items.push(BatchProfileResultRow {
            index,
            label: attempt.label.clone(),
            ok: r.ok,
            warn,
            csv: needs_csv_help(r),
            text: format_import_summary(r, lang),
        });
    }

    let profiles = attempts.len();
    let title = if attention_count > 0 {
        match lang {
            PasswordsLang::En => format!(
                "{profiles} profiles read: {total_added} new accounts. {attention_count} need attention."
            ),
            PasswordsLang::Fr if fr_is_one(attention_count) => format!(
                "{profiles} profils lus : {total_added} nouveaux comptes. {attention_count} demande votre attention."
            ),
            PasswordsLang::Fr => format!(
                "{profiles} profils lus : {total_added} nouveaux comptes. {attention_count} demandent votre attention."
            ),
        }
    } else {
        match lang {
            PasswordsLang::En => format!("{profiles} profiles read: {total_added} new accounts."),
            PasswordsLang::Fr => {
                format!("{profiles} profils lus : {total_added} nouveaux comptes.")
            }
        }
    };

    BatchImportReport {
        title,
        total_added,
        attention_count,
        items,
    }
}

/// Browser-specific CSV export instructions (`exportSteps` in `BrowserPasswords.vue:22-31`).
#[must_use]
pub fn export_steps_for_source(
    browser_id: Option<&str>,
    family: Option<&str>,
    lang: PasswordsLang,
) -> &'static str {
    if family == Some("firefox") {
        return match lang {
            PasswordsLang::En => {
                "In Firefox (and Zen, LibreWolf, Waterfox, Floorp): menu ≡ → Passwords → the ⋯ menu at the top right → Export passwords. Confirm with your device, then save the CSV."
            }
            PasswordsLang::Fr => {
                "Dans Firefox (et Zen, LibreWolf, Waterfox, Floorp) : menu ≡ → Mots de passe → le menu ⋯ en haut à droite → Exporter les mots de passe. Confirmez avec votre appareil, puis enregistrez le CSV."
            }
        };
    }

    match (browser_id, lang) {
        (
            Some("chrome" | "chrome-beta" | "chrome-dev" | "chrome-canary" | "chromium"),
            PasswordsLang::En,
        ) => {
            "In Chrome: menu ⋮ → Passwords and autofill → Google Password Manager → Settings. Under Export passwords, choose Download file."
        }
        (
            Some("chrome" | "chrome-beta" | "chrome-dev" | "chrome-canary" | "chromium"),
            PasswordsLang::Fr,
        ) => {
            "Dans Chrome : menu ⋮ → Mots de passe et saisie automatique → Gestionnaire de mots de passe de Google → Paramètres. Sous Exporter des mots de passe, choisissez Télécharger le fichier."
        }
        (Some("edge"), PasswordsLang::En) => {
            "In Edge: menu ⋯ → Settings → Profiles → Passwords → the ⋯ (More actions) → Export passwords → Export."
        }
        (Some("edge"), PasswordsLang::Fr) => {
            "Dans Edge : menu ⋯ → Paramètres → Profils → Mots de passe → le menu ⋯ (Autres actions) → Exporter les mots de passe → Exporter."
        }
        (Some("vivaldi"), PasswordsLang::En) => {
            "In Vivaldi: Vivaldi menu → File → Export → Export Passwords."
        }
        (Some("vivaldi"), PasswordsLang::Fr) => {
            "Dans Vivaldi : menu Vivaldi → Fichier → Exporter → Exporter les mots de passe."
        }
        (Some("opera" | "opera-gx"), PasswordsLang::En) => {
            "In Opera / Opera GX: Settings → Privacy & Security → Password Manager → Settings. Under Export passwords, choose Download file."
        }
        (Some("opera" | "opera-gx"), PasswordsLang::Fr) => {
            "Dans Opera / Opera GX : Paramètres → Confidentialité et sécurité → Gestionnaire de mots de passe → Paramètres. Sous Exporter les mots de passe, choisissez Télécharger le fichier."
        }
        (Some("brave"), PasswordsLang::En) => {
            "In Brave, enter brave://password-manager/settings in the address bar. Find Export passwords and download the CSV file."
        }
        (Some("brave"), PasswordsLang::Fr) => {
            "Dans Brave, entrez brave://password-manager/settings dans la barre d'adresse. Repérez Exporter les mots de passe et téléchargez le fichier CSV."
        }
        (_, PasswordsLang::En) => {
            "In Safari (macOS): menu bar → File → Export → Passwords, or Settings → Passwords → ⋯ → Export All Passwords. For any other browser, open its password manager and choose Export passwords (CSV)."
        }
        (_, PasswordsLang::Fr) => {
            "Dans Safari (macOS) : barre de menus → Fichier → Exporter → Mots de passe, ou Réglages → Mots de passe → ⋯ → Exporter tous les mots de passe. Pour tout autre navigateur, ouvrez son gestionnaire de mots de passe et choisissez Exporter les mots de passe (CSV)."
        }
    }
}
