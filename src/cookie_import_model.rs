//! Unconnected cookie-export parsing foundation; no browser or session mutations.
//! Pure model for Cookie Import (`CookieImportDialog.vue`,
//! `detect.js`, `cookies.js`, `cookieFile.js`, `cookieImportText.js`).
//!
//! Follows the security invariants:
//! - Values are never logged or stored in plain-text diagnostics.
//! - Google domains (`accounts.google.com`, `*.google.*`) are NEVER imported.
//! - Domain filters and expiration timestamps are enforced strictly.
//! - Safe Netscape and JSON parsing with bounds (`MAX_COOKIE_FILE = 32 MiB`).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Maximum accepted file size for a cookie export file (32 MiB).
pub const MAX_COOKIE_FILE: usize = 32 * 1024 * 1024;

/// Known SameSite policy values.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SameSitePolicy {
    #[default]
    Unspecified,
    NoRestriction,
    Lax,
    Strict,
}

/// A parsed cookie candidate before session injection.
#[derive(Clone, PartialEq, Eq)]
pub struct ParsedCookie {
    pub host: String,
    pub name: String,
    pub value: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: SameSitePolicy,
    pub expires: u64,
    pub host_only: bool,
}

// Deliberately omit every field: names, paths and values can all contain secrets.
impl std::fmt::Debug for ParsedCookie {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ParsedCookie(<redacted>)")
    }
}

fn valid_host(host: &str) -> bool {
    let host = host.strip_prefix('.').unwrap_or(host);
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

fn valid_fields(host: &str, name: &str, value: &str, path: &str) -> bool {
    valid_host(host)
        && !name.is_empty()
        && path.starts_with('/')
        && [name, value, path]
            .iter()
            .all(|s| !s.chars().any(char::is_control))
}

/// Reasons why a candidate cookie was skipped during import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SkipReason {
    Expired,
    AppBound,
    Partitioned,
    Container,
    InvalidHost,
    Google,
    Filtered,
    Undecryptable,
    Rejected,
    Other,
}

impl SkipReason {
    pub const fn id(self) -> &'static str {
        match self {
            Self::Expired => "expired",
            Self::AppBound => "appBound",
            Self::Partitioned => "partitioned",
            Self::Container => "container",
            Self::InvalidHost => "invalidHost",
            Self::Google => "google",
            Self::Filtered => "filtered",
            Self::Undecryptable => "undecryptable",
            Self::Rejected => "rejected",
            Self::Other => "other",
        }
    }

    pub const fn label(self, french: bool) -> &'static str {
        match self {
            Self::Expired => {
                if french {
                    "Expirés (non valides dans ce navigateur)"
                } else {
                    "Expired (no longer valid in that browser)"
                }
            }
            Self::AppBound => {
                if french {
                    "Chiffrement lié à l'application Chrome (v20)"
                } else {
                    "Chrome's app-bound encryption (v20)"
                }
            }
            Self::Partitioned => {
                if french {
                    "Partitionnés (cookies tiers conservés par site)"
                } else {
                    "Partitioned (third-party cookies kept per site)"
                }
            }
            Self::Container => {
                if french {
                    "Conteneurs Firefox"
                } else {
                    "Firefox containers"
                }
            }
            Self::InvalidHost => {
                if french {
                    "Domaine ou hôte non valide"
                } else {
                    "Invalid domain or host"
                }
            }
            Self::Google => {
                if french {
                    "Google (jamais importé)"
                } else {
                    "Google (never imported)"
                }
            }
            Self::Filtered => {
                if french {
                    "Hors des domaines choisis"
                } else {
                    "Outside the chosen domains"
                }
            }
            Self::Undecryptable => {
                if french {
                    "Impossible à déchiffrer"
                } else {
                    "Could not be decrypted"
                }
            }
            Self::Rejected => {
                if french {
                    "Refusés par le navigateur de Tessel"
                } else {
                    "Refused by Tessel's browser"
                }
            }
            Self::Other => {
                if french {
                    "Autre (malformé)"
                } else {
                    "Other (malformed)"
                }
            }
        }
    }
}

/// Checks whether a hostname belongs to Google.
/// Google cookies are excluded unconditionally to prevent session-theft security triggers.
pub fn is_google_host(host: &str) -> bool {
    let clean = host.trim().trim_start_matches('.').to_ascii_lowercase();
    if clean.is_empty() {
        return false;
    }
    if clean == "google.com" || clean.ends_with(".google.com") {
        return true;
    }
    // Generic country ccTLDs: google.co.uk, google.ca, google.fr, etc.
    let parts: Vec<&str> = clean.split('.').collect();
    if parts.len() >= 2 {
        let root_idx = parts.iter().position(|&p| p == "google");
        if let Some(idx) = root_idx
            && idx + 1 < parts.len()
        {
            return true;
        }
    }
    false
}

/// Normalizes domain filter text (comma/space-separated) into up to 50 lowercase domain strings.
pub fn parse_domain_filter(text: &str) -> Result<Vec<String>, &'static str> {
    let mut out = Vec::new();
    for raw in text.split(|c: char| c.is_whitespace() || c == ',' || c == ';') {
        let mut d = raw.trim().to_ascii_lowercase();
        if d.is_empty() {
            continue;
        }
        if let Some(pos) = d.find("://") {
            d = d[pos + 3..].to_string();
        }
        if let Some(pos) = d.find(['/', '?', '#']) {
            d = d[..pos].to_string();
        }
        if let Some(pos) = d.find(':') {
            d = d[..pos].to_string();
        }
        d = d.trim_start_matches(['*', '.']).to_string();
        if !valid_host(&d) || d.starts_with('.') {
            return Err("Invalid domain filter");
        }
        if !out.contains(&d) {
            out.push(d);
            if out.len() >= 50 {
                break;
            }
        }
    }
    Ok(out)
}

/// Checks whether a cookie's host matches the domain filter.
pub fn matches_domains(host: &str, domains: &[String]) -> bool {
    if domains.is_empty() {
        return true;
    }
    let h = host.trim().trim_start_matches('.').to_ascii_lowercase();
    domains
        .iter()
        .any(|d| h == *d || h.ends_with(&format!(".{d}")))
}

/// Detected profile inside a browser installation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedProfile {
    pub dir: String,
    pub name: String,
    pub cookie_count: Option<usize>,
    pub last_import: Option<u64>,
    pub app_bound_count: usize,
    pub last_used: bool,
}

/// Detected browser on this computer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedBrowser {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub profiles: Vec<DetectedProfile>,
}

/// Summary of completed or simulated cookie import.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CookieImportSummary {
    pub imported: usize,
    pub added: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub skipped: usize,
    pub previous_at: Option<u64>,
    pub reasons: BTreeMap<String, usize>,
}

impl CookieImportSummary {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_skipped(&mut self, reason: SkipReason) {
        self.skipped += 1;
        *self.reasons.entry(reason.id().to_owned()).or_insert(0) += 1;
    }

    pub fn headline_text(&self, french: bool) -> String {
        let changes = self.changes_text(french);
        if let (Some(prev), false) = (self.previous_at, changes.is_empty()) {
            let date = format_timestamp_date(prev, french);
            if french {
                return format!("Déjà importé le {date}. Cette fois : {changes}.");
            } else {
                return format!("Already imported on {date}. This time: {changes}.");
            }
        }
        if !changes.is_empty() {
            let n = self.imported;
            if french {
                let noun = if n == 1 { "cookie" } else { "cookies" };
                return format!("{n} {noun} importé : {changes}.");
            } else {
                let noun = if n == 1 { "cookie" } else { "cookies" };
                return format!("Imported {n} {noun}: {changes}.");
            }
        }
        let n = self.imported;
        if french {
            let noun = if n == 1 { "cookie" } else { "cookies" };
            format!("{n} {noun} importé.")
        } else {
            let noun = if n == 1 { "cookie" } else { "cookies" };
            format!("Imported {n} {noun}.")
        }
    }

    pub fn changes_text(&self, french: bool) -> String {
        if self.added == 0 && self.updated == 0 && self.unchanged == 0 {
            return String::new();
        }
        if french {
            format!(
                "{} nouveaux, {} mis à jour, {} inchangés",
                self.added, self.updated, self.unchanged
            )
        } else {
            format!(
                "{} new, {} updated, {} unchanged",
                self.added, self.updated, self.unchanged
            )
        }
    }

    pub fn skipped_text(&self, french: bool) -> String {
        if self.skipped == 0 {
            return String::new();
        }
        if french {
            format!("{} ignorés.", self.skipped)
        } else {
            format!("{} skipped.", self.skipped)
        }
    }

    pub fn breakdown(&self, french: bool) -> Vec<(String, String, usize)> {
        let mut out = Vec::new();
        for reason in [
            SkipReason::Expired,
            SkipReason::AppBound,
            SkipReason::Partitioned,
            SkipReason::Container,
            SkipReason::InvalidHost,
            SkipReason::Google,
            SkipReason::Filtered,
            SkipReason::Undecryptable,
            SkipReason::Rejected,
            SkipReason::Other,
        ] {
            if let Some(&count) = self.reasons.get(reason.id())
                && count > 0
            {
                out.push((
                    reason.id().to_owned(),
                    reason.label(french).to_owned(),
                    count,
                ));
            }
        }
        out
    }

    pub fn toast_text(&self, french: bool) -> String {
        let changes = self.changes_text(french);
        if !changes.is_empty() {
            if french {
                return format!("Cookies importés dans le navigateur de Tessel : {changes}.");
            } else {
                return format!("Cookies imported into Tessel's browser: {changes}.");
            }
        }
        let n = self.imported;
        if french {
            let noun = if n == 1 {
                "cookie importé"
            } else {
                "cookies importés"
            };
            format!("{n} {noun} dans le navigateur de Tessel.")
        } else {
            let noun = if n == 1 { "cookie" } else { "cookies" };
            format!("Imported {n} {noun} into Tessel's browser.")
        }
    }
}

fn format_timestamp_date(epoch_ms: u64, french: bool) -> String {
    let secs = epoch_ms / 1000;
    // Approximated ISO date YYYY-MM-DD
    let days = secs / 86400;
    let mut year = 1970;
    let mut rem_days = days;
    loop {
        let leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
        let days_in_year = if leap { 366 } else { 365 };
        if rem_days < days_in_year {
            break;
        }
        rem_days -= days_in_year;
        year += 1;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1;
    for &md in &month_days {
        if rem_days < md {
            break;
        }
        rem_days -= md;
        month += 1;
    }
    let day = rem_days + 1;
    if french {
        format!("{day:02}/{month:02}/{year}")
    } else {
        format!("{year}-{month:02}-{day:02}")
    }
}

/// Parses an exported cookie file (Netscape format or JSON export) up to 32 MiB.
pub fn parse_cookie_export(
    raw: &str,
    domain_filter: &[String],
    now_sec: u64,
) -> (Vec<ParsedCookie>, CookieImportSummary) {
    let mut summary = CookieImportSummary::new();
    let mut cookies = Vec::new();

    if raw.len() > MAX_COOKIE_FILE {
        summary.record_skipped(SkipReason::Rejected);
        return (cookies, summary);
    }
    let trimmed = raw.trim_start_matches('\u{feff}').trim();
    if trimmed.starts_with('[') || trimmed.starts_with('{') {
        // Parse JSON format
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed)
            && let Some(list) = val
                .as_array()
                .or_else(|| val.get("cookies").and_then(|v| v.as_array()))
        {
            for item in list {
                if let Some(obj) = item.as_object() {
                    let name = obj.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let value = obj.get("value").and_then(|v| v.as_str()).unwrap_or("");
                    let host = obj
                        .get("domain")
                        .or_else(|| obj.get("host"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim();
                    let path = obj.get("path").and_then(|v| v.as_str()).unwrap_or("/");
                    let secure = obj.get("secure").and_then(|v| v.as_bool()).unwrap_or(false);
                    let http_only = obj
                        .get("httpOnly")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let is_partitioned = obj
                        .get("partitionKey")
                        .is_some_and(|v| !v.is_null() && v != "");

                    if !valid_fields(host, name, value, path) {
                        summary.record_skipped(SkipReason::InvalidHost);
                        continue;
                    }
                    if is_partitioned {
                        summary.record_skipped(SkipReason::Partitioned);
                        continue;
                    }
                    if is_google_host(host) {
                        summary.record_skipped(SkipReason::Google);
                        continue;
                    }
                    if !matches_domains(host, domain_filter) {
                        summary.record_skipped(SkipReason::Filtered);
                        continue;
                    }
                    let Some(expires) = parse_json_expiry(obj) else {
                        summary.record_skipped(SkipReason::Rejected);
                        continue;
                    };
                    if expires > 0 && expires <= now_sec {
                        summary.record_skipped(SkipReason::Expired);
                        continue;
                    }

                    let same_site = match obj
                        .get("sameSite")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_ascii_lowercase()
                        .as_str()
                    {
                        "no_restriction" | "none" => SameSitePolicy::NoRestriction,
                        "lax" => SameSitePolicy::Lax,
                        "strict" => SameSitePolicy::Strict,
                        _ => SameSitePolicy::Unspecified,
                    };

                    cookies.push(ParsedCookie {
                        host: host.to_owned(),
                        name: name.to_owned(),
                        value: value.to_owned(),
                        path: path.to_owned(),
                        secure,
                        http_only,
                        same_site,
                        expires,
                        host_only: obj
                            .get("hostOnly")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(!host.starts_with('.')),
                    });
                    summary.imported += 1;
                    summary.added += 1;
                } else {
                    summary.record_skipped(SkipReason::Other);
                }
            }
            return (cookies, summary);
        }
    }

    // Malformed JSON must not fall through to a different export format.
    if trimmed.starts_with('[') || trimmed.starts_with('{') {
        summary.record_skipped(SkipReason::Rejected);
        return (cookies, summary);
    }
    // Otherwise, parse Netscape format line by line
    for line in raw.trim_start_matches('\u{feff}').lines() {
        let mut l = line;
        if l.is_empty() {
            continue;
        }
        let mut http_only = false;
        if l.starts_with("#HttpOnly_") {
            http_only = true;
            l = &l["#HttpOnly_".len()..];
        } else if l.starts_with('#') {
            continue;
        }

        let parts: Vec<&str> = l.split('\t').collect();
        if parts.len() != 7 {
            summary.record_skipped(SkipReason::Other);
            continue;
        }

        let host = parts[0].trim();
        let path = parts[2].trim();
        let secure = parts[3].eq_ignore_ascii_case("true");
        let Ok(expires) = parts[4].parse::<u64>() else {
            summary.record_skipped(SkipReason::Rejected);
            continue;
        };
        let name = parts[5];
        let value = parts[6];

        if !valid_fields(host, name, value, path) {
            summary.record_skipped(SkipReason::InvalidHost);
            continue;
        }
        if is_google_host(host) {
            summary.record_skipped(SkipReason::Google);
            continue;
        }
        if !matches_domains(host, domain_filter) {
            summary.record_skipped(SkipReason::Filtered);
            continue;
        }
        if expires > 0 && expires <= now_sec {
            summary.record_skipped(SkipReason::Expired);
            continue;
        }

        cookies.push(ParsedCookie {
            host: host.to_owned(),
            name: name.to_owned(),
            value: value.to_owned(),
            path: if path.is_empty() { "/" } else { path }.to_owned(),
            secure,
            http_only,
            same_site: SameSitePolicy::Unspecified,
            expires,
            host_only: !parts[1].eq_ignore_ascii_case("true"),
        });
        summary.imported += 1;
        summary.added += 1;
    }

    (cookies, summary)
}

fn parse_json_expiry(obj: &serde_json::Map<String, serde_json::Value>) -> Option<u64> {
    if obj.get("session").and_then(|v| v.as_bool()) == Some(true) {
        return Some(0);
    }
    for key in ["expirationDate", "expires", "expiry"] {
        if let Some(v) = obj.get(key) {
            let mut n = v.as_f64()?;
            if !n.is_finite() || n < 0.0 {
                return None;
            }
            if n > 1e11 {
                n /= 1000.0;
            }
            if n >= u64::MAX as f64 {
                return None;
            }
            return Some(n as u64);
        }
    }
    Some(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapped_json_preserves_session_and_explicit_scope() {
        let raw = r#"{"cookies":[{"name":"n","value":"v","domain":"example.org","hostOnly":false,"session":true,"expires":-1,"partitionKey":null}]}"#;
        let (cookies, summary) = parse_cookie_export(raw, &[], 5000);
        assert_eq!(summary.skipped, 0);
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].expires, 0);
        assert!(!cookies[0].host_only);
        assert!(parse_domain_filter("!!!").is_err());
        assert!(parse_domain_filter("example.org, invalid..org").is_err());
        assert!(parse_domain_filter("").unwrap().is_empty());
    }

    #[test]
    fn rejects_oversize_and_redacts_diagnostics() {
        let (cookies, summary) = parse_cookie_export(&" ".repeat(MAX_COOKIE_FILE + 1), &[], 0);
        assert!(cookies.is_empty());
        assert_eq!(summary.reasons.get("rejected"), Some(&1));
        let (cookies, _) = parse_cookie_export(
            r#"[{"name":"private-name","value":"private-value","domain":"example.org","path":"/private-path"}]"#,
            &[],
            0,
        );
        assert_eq!(format!("{:?}", cookies[0]), "ParsedCookie(<redacted>)");
    }

    #[test]
    fn rejects_malformed_hosts_expiry_and_control_bytes() {
        for host in [
            "..example.org",
            "https://example.org",
            "example..org",
            "-bad.org",
            "google.com.",
        ] {
            let input = serde_json::json!([{"name":"n", "value":"v", "domain":host}]);
            assert!(parse_cookie_export(&input.to_string(), &[], 0).0.is_empty());
        }
        for expiry in [serde_json::json!(-1), serde_json::json!("invalid")] {
            let input = serde_json::json!([{"name":"n", "value":"v", "domain":"example.org", "expires":expiry}]);
            assert!(parse_cookie_export(&input.to_string(), &[], 0).0.is_empty());
        }
        let input =
            serde_json::json!([{"name":"n", "value":"v\r\nInjected", "domain":"example.org"}]);
        assert!(parse_cookie_export(&input.to_string(), &[], 0).0.is_empty());
    }

    #[test]
    fn netscape_preserves_empty_values_and_scope_flag() {
        let raw = "example.org\tTRUE\t/\tFALSE\t0\tempty\t\nexample.org\tFALSE\t/\tTRUE\t0\tspaced\t value ";
        let (cookies, _) = parse_cookie_export(raw, &[], 0);
        assert_eq!(cookies.len(), 2);
        assert_eq!(cookies[0].value, "");
        assert!(!cookies[0].host_only);
        assert!(cookies[1].host_only);
        assert_eq!(cookies[1].value, " value ");
        assert!(
            parse_cookie_export("example.org\tTRUE\t/\tTRUE\tbad\tn\tv", &[], 0)
                .0
                .is_empty()
        );
    }

    #[test]
    fn test_is_google_host_exclusions() {
        assert!(is_google_host("google.com"));
        assert!(is_google_host("accounts.google.com"));
        assert!(is_google_host(".google.com"));
        assert!(is_google_host("google.co.uk"));
        assert!(is_google_host("google.fr"));
        assert!(is_google_host("mail.google.com"));
        assert!(!is_google_host("github.com"));
        assert!(!is_google_host("linear.app"));
        assert!(!is_google_host("notgoogle.com"));
    }

    #[test]
    fn test_parse_domain_filter() {
        let f = parse_domain_filter(
            "github.com,  https://linear.app/issues, .gitlab.com; *example.org",
        )
        .unwrap();
        assert_eq!(
            f,
            vec!["github.com", "linear.app", "gitlab.com", "example.org"]
        );
    }

    #[test]
    fn test_matches_domains() {
        let domains = vec!["github.com".to_string(), "linear.app".to_string()];
        assert!(matches_domains("github.com", &domains));
        assert!(matches_domains("api.github.com", &domains));
        assert!(matches_domains("linear.app", &domains));
        assert!(!matches_domains("gitlab.com", &domains));
        assert!(matches_domains("anything.com", &[])); // empty filter matches all
    }

    #[test]
    fn test_parse_netscape_cookies() {
        let text = "\
# Netscape HTTP Cookie File
.github.com\tTRUE\t/\tTRUE\t2000000000\tuser_session\tsecret123
#HttpOnly_.linear.app\tTRUE\t/\tTRUE\t2000000000\tsession\tlinear456
accounts.google.com\tTRUE\t/\tTRUE\t2000000000\tSID\tgooglesecret
.other.com\tTRUE\t/\tTRUE\t1000\told\toldval
";
        let (cookies, summary) = parse_cookie_export(text, &[], 5000);
        // google.com skipped, old (expires 1000 <= 5000) skipped
        assert_eq!(cookies.len(), 2);
        assert_eq!(cookies[0].name, "user_session");
        assert!(!cookies[0].http_only);
        assert_eq!(cookies[1].name, "session");
        assert!(cookies[1].http_only);

        assert_eq!(summary.imported, 2);
        assert_eq!(summary.skipped, 2);
        assert_eq!(summary.reasons.get("google"), Some(&1));
        assert_eq!(summary.reasons.get("expired"), Some(&1));
    }

    #[test]
    fn test_parse_json_cookies() {
        let json_text = r#"[
            {"name":"token","value":"val1","domain":"github.com","path":"/","secure":true,"httpOnly":true,"expirationDate":2500000000.0},
            {"name":"gtoken","value":"gval","domain":"google.com","path":"/","secure":true},
            {"name":"part","value":"pval","domain":"example.org","partitionKey":"https://other.com"}
        ]"#;
        let (cookies, summary) = parse_cookie_export(json_text, &[], 5000);
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "token");
        assert_eq!(summary.imported, 1);
        assert_eq!(summary.skipped, 2);
        assert_eq!(summary.reasons.get("google"), Some(&1));
        assert_eq!(summary.reasons.get("partitioned"), Some(&1));
    }

    #[test]
    fn test_cookie_import_summary_formatting() {
        let mut s = CookieImportSummary::new();
        s.imported = 15;
        s.added = 10;
        s.updated = 5;
        s.unchanged = 20;
        s.record_skipped(SkipReason::Google);
        s.record_skipped(SkipReason::Expired);

        assert_eq!(s.changes_text(false), "10 new, 5 updated, 20 unchanged");
        assert_eq!(
            s.changes_text(true),
            "10 nouveaux, 5 mis à jour, 20 inchangés"
        );
        assert_eq!(s.skipped_text(false), "2 skipped.");
        assert_eq!(s.skipped_text(true), "2 ignorés.");

        let bd = s.breakdown(false);
        assert_eq!(bd.len(), 2);
    }
}
