//! Direct GraphQL client for Linear issues and teams.
//! Transcribed from `src/main/linearService.js` and `src/renderer/src/components/LinearDialog.vue`.

use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;
mod body;
mod parse;
mod record;
pub mod request;
pub mod service;
mod storage_path;
pub mod worker;

pub const LINEAR_ENDPOINT: &str = "https://api.linear.app/graphql";
pub const MAX_LINEAR_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub has_next_page: bool,
    pub end_cursor: Option<String>,
}
fn page_nodes(value: &serde_json::Value) -> Result<(&[serde_json::Value], bool), String> {
    let nodes = value["nodes"]
        .as_array()
        .filter(|n| n.len() <= 100)
        .ok_or("Invalid Linear result list")?;
    let more = value["pageInfo"]["hasNextPage"]
        .as_bool()
        .ok_or("Missing Linear pagination metadata")?;
    Ok((nodes, more))
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.as_bytes()[0].is_ascii_alphanumeric()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
fn checked_id(id: &str) -> Result<(), String> {
    if valid_id(id) {
        Ok(())
    } else {
        Err("Invalid Linear identifier".into())
    }
}
fn issue_url(raw: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(raw).map_err(|_| "Invalid Linear issue URL")?;
    if url.scheme() != "https"
        || url.host_str() != Some("linear.app")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
    {
        return Err("Invalid Linear issue URL".into());
    }
    Ok(url.into())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinearFilter {
    #[default]
    Assigned,
    Created,
    All,
    Completed,
    Open,
}

impl LinearFilter {
    pub const ALL: [Self; 5] = [
        Self::Assigned,
        Self::Created,
        Self::Open,
        Self::Completed,
        Self::All,
    ];

    #[must_use]
    pub const fn label(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::Assigned, false) => "Assigned to me",
            (Self::Assigned, true) => "Assignées à moi",
            (Self::Created, false) => "Created by me",
            (Self::Created, true) => "Créées par moi",
            (Self::All, false) => "All issues",
            (Self::All, true) => "Toutes les issues",
            (Self::Completed, false) => "Completed",
            (Self::Completed, true) => "Terminées",
            (Self::Open, false) => "All open",
            (Self::Open, true) => "Toutes les ouvertes",
        }
    }

    #[must_use]
    pub const fn id_str(self) -> &'static str {
        match self {
            Self::Assigned => "assigned",
            Self::Created => "created",
            Self::All => "all",
            Self::Completed => "completed",
            Self::Open => "open",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinearTeam {
    pub id: String,
    pub name: String,
    pub key: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LinearState {
    pub id: String,
    pub name: String,
    pub r#type: String,
    pub color: String,
    pub position: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinearPerson {
    pub id: String,
    pub name: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinearOrganization {
    pub id: String,
    pub name: String,
    pub url_key: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinearIdentity {
    pub viewer: LinearPerson,
    pub organization: LinearOrganization,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LinearIssue {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub url: String,
    pub description: String,
    pub priority: u8,
    pub created_at: String,
    pub updated_at: String,
    pub state: LinearState,
    pub team: LinearTeam,
    pub assignee: Option<LinearPerson>,
    pub creator: Option<LinearPerson>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IssueSelection {
    pub filter: LinearFilter,
    pub team_id: Option<String>,
    pub state_id: Option<String>,
    pub priority: Option<u8>,
}

impl LinearIssue {
    #[must_use]
    pub fn priority_label(&self, french: bool) -> &'static str {
        match (self.priority, french) {
            (1, false) => "Urgent",
            (1, true) => "Urgent",
            (2, false) => "High",
            (2, true) => "Élevée",
            (3, false) => "Medium",
            (3, true) => "Moyenne",
            (4, false) => "Low",
            (4, true) => "Basse",
            _ => {
                if french {
                    "Aucune priorité"
                } else {
                    "No priority"
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Linear Credential Storage (DPAPI on Windows)
// ---------------------------------------------------------------------------

fn linear_storage_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "Tessel", "TesselRust")
        .map(|dirs| dirs.data_local_dir().join("linear.dat"))
}

const MAX_KEY_FILE: u64 = 64 * 1024;
fn valid_key(key: &str) -> Result<&str, String> {
    let key = key.trim();
    if !(8..=4096).contains(&key.len()) || !key.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return Err("Invalid Linear API key.".into());
    }
    Ok(key)
}
fn protect_key(bytes: &[u8], encrypt: bool) -> Result<Zeroizing<Vec<u8>>, String> {
    #[cfg(windows)]
    {
        crate::vault::protect(bytes, encrypt)
    }
    #[cfg(not(windows))]
    {
        let _ = (bytes, encrypt);
        Err("Protected Linear key storage is unavailable on this platform.".into())
    }
}
fn key_lock(path: &Path) -> Result<File, String> {
    let parent = path.parent().ok_or("Invalid private storage directory")?;
    storage_path::directory(parent)?;
    storage_path::check_file(path)?;
    let file = storage_path::open(&parent.join("linear.lock"), true)?
        .ok_or("Cannot lock Linear storage")?;
    fs2::FileExt::try_lock_exclusive(&file).map_err(|_| "Linear storage is busy")?;
    Ok(file)
}
fn save_key_at(path: &Path, key: &str) -> Result<(), String> {
    let key = valid_key(key)?;
    save_plain_at(path, key.as_bytes())
}
fn save_plain_at(path: &Path, plain: &[u8]) -> Result<(), String> {
    let protected = protect_key(plain, true)?;
    if protected.len() > MAX_KEY_FILE as usize {
        return Err("Protected Linear record is too large".into());
    }
    let _lock = key_lock(path)?;
    let mut temp =
        tempfile::NamedTempFile::new_in(path.parent().ok_or("Invalid private storage directory")?)
            .map_err(|_| "Cannot create protected Linear key")?;
    temp.write_all(&protected)
        .and_then(|_| temp.as_file().sync_all())
        .map_err(|_| "Cannot write protected Linear key")?;
    storage_path::check_file(path)?;
    temp.persist(path)
        .map_err(|_| "Cannot save protected Linear key")?;
    Ok(())
}
fn load_plain_at(path: &Path) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
    let _lock = key_lock(path)?;
    let Some(file) = storage_path::open(path, false)? else {
        return Ok(None);
    };
    let bytes = read_bounded(file, MAX_KEY_FILE as usize)?;
    Ok(Some(protect_key(&bytes, false)?))
}
fn load_key_at(path: &Path) -> Result<Option<Zeroizing<String>>, String> {
    let Some(plain) = load_plain_at(path)? else {
        return Ok(None);
    };
    let key = std::str::from_utf8(&plain).map_err(|_| "Invalid protected Linear key")?;
    Ok(Some(Zeroizing::new(valid_key(key)?.to_owned())))
}
fn delete_key_at(path: &Path) -> Result<(), String> {
    let _lock = key_lock(path)?;
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("Cannot remove protected Linear key".into()),
    }
}
pub fn save_linear_key(key: &str) -> Result<(), String> {
    save_key_at(
        &linear_storage_path().ok_or("No private data directory")?,
        key,
    )
}
pub fn load_linear_key() -> Result<Option<Zeroizing<String>>, String> {
    load_key_at(&linear_storage_path().ok_or("No private data directory")?)
}
pub fn delete_linear_key() -> Result<(), String> {
    delete_key_at(&linear_storage_path().ok_or("No private data directory")?)
}
fn read_bounded(reader: impl Read, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read bounded Linear data")?;
    if bytes.len() > limit {
        return Err("Linear data exceeds size limit".into());
    }
    Ok(bytes)
}

// ---------------------------------------------------------------------------
// Linear GraphQL Client
// ---------------------------------------------------------------------------

const ISSUE_FIELDS: &str = "id identifier title url description priority createdAt updatedAt \
state { id name type color } team { id name key } \
assignee { id name } creator { id name }";

async fn execute_linear_query(
    key: &str,
    query: &str,
    variables: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let trimmed_key = valid_key(key)?;
    let body = serde_json::json!({
        "query": query,
        "variables": variables,
    });
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

    let mut auth = reqwest::header::HeaderValue::from_str(trimmed_key)
        .map_err(|_| "Invalid Linear authorization")?;
    auth.set_sensitive(true);
    let resp = client
        .post(LINEAR_ENDPOINT)
        .header(reqwest::header::AUTHORIZATION, auth)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .body(serde_json::to_string(&body).map_err(|e| e.to_string())?)
        .send()
        .await
        .map_err(|e| {
            let msg = e.to_string();
            msg.replace(trimmed_key, "[redacted]")
        })?;

    let status = resp.status();
    if !status.is_success() {
        return Err(format!("Linear API returned status {status}"));
    }

    if resp
        .content_length()
        .is_some_and(|length| length > MAX_LINEAR_RESPONSE_BYTES as u64)
    {
        return Err("Linear data exceeds size limit".into());
    }
    let raw_bytes = body::collect(resp, MAX_LINEAR_RESPONSE_BYTES).await?;

    parse_response(&raw_bytes)
}
fn parse_response(raw_bytes: &[u8]) -> Result<serde_json::Value, String> {
    let root: serde_json::Value =
        serde_json::from_slice(raw_bytes).map_err(|_| "Invalid Linear response JSON")?;

    if let Some(errors) = root.get("errors").and_then(|e| e.as_array())
        && !errors.is_empty()
    {
        return Err("Linear rejected the GraphQL request.".into());
    }

    root.get("data")
        .filter(|v| v.is_object())
        .cloned()
        .ok_or_else(|| "Linear response missing 'data' field.".to_owned())
}

pub async fn validate_linear_key(key: &str) -> Result<LinearIdentity, String> {
    let data = execute_linear_query(
        key,
        "query TesselLinearConnect { viewer { id name displayName email } organization { id name urlKey } }",
        serde_json::json!({}),
    ).await?;
    parse::identity(&data)
}
pub async fn fetch_linear_teams(key: &str) -> Result<Page<LinearTeam>, String> {
    let data = execute_linear_query(
        key,
        "query TesselLinearTeams { teams(first: 100) { nodes { id name key } pageInfo { hasNextPage endCursor } } }",
        serde_json::json!({}),
    ).await?;
    parse::page(&data["teams"], parse::team)
}
pub async fn fetch_linear_states(key: &str, team_id: &str) -> Result<Page<LinearState>, String> {
    checked_id(team_id)?;
    let data = execute_linear_query(
        key,
        "query TesselLinearStates($teamId: String!) { team(id: $teamId) { states(first: 100) { nodes { id name type position color } pageInfo { hasNextPage endCursor } } } }",
        serde_json::json!({"teamId":team_id}),
    ).await?;
    let mut page = parse::page(&data["team"]["states"], parse::state)?;
    page.items.sort_by(|a, b| {
        a.position
            .unwrap_or(0.)
            .partial_cmp(&b.position.unwrap_or(0.))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(page)
}
fn issue_filter(viewer_id: &str, selection: &IssueSelection) -> Result<serde_json::Value, String> {
    use serde_json::json;
    let mut filter = json!({});
    match selection.filter {
        LinearFilter::Assigned | LinearFilter::Created => {
            checked_id(viewer_id)?;
            filter[if selection.filter == LinearFilter::Assigned {
                "assignee"
            } else {
                "creator"
            }] = json!({"id":{"eq":viewer_id}});
        }
        LinearFilter::Completed => filter["state"] = json!({"type":{"eq":"completed"}}),
        LinearFilter::Open => filter["state"] = json!({"type":{"nin":["completed","canceled"]}}),
        LinearFilter::All => {}
    }
    if let Some(id) = selection.team_id.as_deref().filter(|s| !s.is_empty()) {
        checked_id(id)?;
        filter["team"] = json!({"id":{"eq":id}});
    }
    if let Some(id) = selection.state_id.as_deref().filter(|s| !s.is_empty()) {
        checked_id(id)?;
        if filter["state"].is_null() {
            filter["state"] = json!({});
        }
        filter["state"]["id"] = json!({"eq":id});
    }
    if let Some(priority) = selection.priority {
        if priority > 4 {
            return Err("Invalid Linear priority".into());
        }
        filter["priority"] = json!({"eq":priority});
    }
    Ok(filter)
}
pub async fn fetch_linear_issues(
    key: &str,
    viewer_id: &str,
    selection: &IssueSelection,
) -> Result<Page<LinearIssue>, String> {
    let filter = issue_filter(viewer_id, selection)?;
    let query = format!(
        "query TesselLinearIssues($filter: IssueFilter!) {{ issues(first: 100, orderBy: updatedAt, filter: $filter) {{ nodes {{ {ISSUE_FIELDS} }} pageInfo {{ hasNextPage endCursor }} }} }}"
    );
    let data = execute_linear_query(key, &query, serde_json::json!({"filter":filter})).await?;
    parse::page(&data["issues"], parse::issue)
}
pub async fn update_linear_issue_state(
    key: &str,
    issue_id: &str,
    state_id: &str,
) -> Result<LinearIssue, String> {
    checked_id(issue_id)?;
    checked_id(state_id)?;
    let query = format!(
        "mutation TesselLinearSetState($issueId: String!, $input: IssueUpdateInput!) {{ issueUpdate(id: $issueId, input: $input) {{ success issue {{ {ISSUE_FIELDS} }} }} }}"
    );
    let data = execute_linear_query(
        key,
        &query,
        serde_json::json!({"issueId":issue_id,"input":{"stateId":state_id}}),
    )
    .await?;
    parse::updated_issue(&data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn bounded_reader_stops_at_limit_plus_one_and_propagates_failures() {
        struct Infinite(usize);
        impl Read for Infinite {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                self.0 += buf.len();
                buf.fill(b'x');
                Ok(buf.len())
            }
        }
        let mut reader = Infinite(0);
        assert!(read_bounded(&mut reader, 1024).is_err());
        assert_eq!(reader.0, 1025);
        assert_eq!(read_bounded(&b"1234"[..], 4).unwrap(), b"1234");
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("synthetic"))
            }
        }
        assert!(read_bounded(Broken, 4).is_err());
    }
    #[test]
    fn response_errors_never_echo_remote_secrets_and_incomplete_data_is_rejected() {
        let raw =
            serde_json::to_vec(&json!({"errors":[{"message":"synthetic-secret-123"}],"data":{}}))
                .unwrap();
        let err = parse_response(&raw).unwrap_err();
        assert!(!err.contains("synthetic-secret"));
        for raw in [b"null".as_slice(), b"{\"data\":null}", b"broken"] {
            assert!(parse_response(raw).is_err());
        }
        assert!(parse::identity(&json!({"viewer":{},"organization":{}})).is_err());
        let identity=parse::identity(&json!({"viewer":{"id":"user-1","name":"Fixture"},"organization":{"id":"org-1","name":"Test"}})).unwrap();
        assert_eq!(identity.viewer.id, "user-1");
        assert_eq!(identity.viewer.name, "Fixture");
        assert_eq!(identity.organization.id, "org-1");
        assert_eq!(identity.organization.name, "Test");
    }
    #[test]
    fn identifiers_urls_and_pagination_are_explicit() {
        for id in ["", "../private", "with space", "-first"] {
            assert!(!valid_id(id));
        }
        assert!(valid_id("team_1-A"));
        for url in [
            "http://linear.app/a",
            "https://linear.app.evil.test/a",
            "https://user:pass@linear.app/a",
            "file:///private",
            "https://linear.app:444/a",
        ] {
            assert!(issue_url(url).is_err());
        }
        assert_eq!(
            issue_url("https://linear.app/team/issue/T-1").unwrap(),
            "https://linear.app/team/issue/T-1"
        );
        assert!(page_nodes(&json!({"nodes":[]})).is_err());
        assert!(
            page_nodes(&json!({"nodes":vec![json!({});101],"pageInfo":{"hasNextPage":false}}))
                .is_err()
        );
        assert!(
            page_nodes(&json!({"nodes":[],"pageInfo":{"hasNextPage":true}}))
                .unwrap()
                .1
        );
        assert!(valid_key("short").is_err());
        assert!(valid_key("validkey\ninvalid").is_err());
    }
    #[cfg(windows)]
    #[test]
    fn synthetic_key_is_encrypted_atomic_locked_and_deletion_errors_surface() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("linear.dat");
        let key = "synthetic-linear-key-only";
        assert!(load_key_at(&path).unwrap().is_none());
        save_key_at(&path, key).unwrap();
        let saved = std::fs::read(&path).unwrap();
        assert!(!saved.windows(key.len()).any(|w| w == key.as_bytes()));
        assert_eq!(load_key_at(&path).unwrap().unwrap().as_str(), key);
        let lock = key_lock(&path).unwrap();
        assert!(save_key_at(&path, "replacement-key").is_err());
        assert!(delete_key_at(&path).is_err());
        drop(lock);
        assert!(save_key_at(&path, "bad").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), saved);
        save_key_at(&path, "replacement-key").unwrap();
        assert_eq!(
            load_key_at(&path).unwrap().unwrap().as_str(),
            "replacement-key"
        );
        std::fs::write(&path, b"corrupt").unwrap();
        assert!(load_key_at(&path).is_err());
        delete_key_at(&path).unwrap();
        delete_key_at(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(delete_key_at(&path).is_err());
        assert!(save_key_at(&path, key).is_err());
        assert!(path.is_dir());
    }
    #[cfg(not(windows))]
    #[test]
    fn unavailable_protection_never_writes_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("linear.dat");
        assert!(save_key_at(&path, "synthetic-key").is_err());
        assert!(!path.exists());
    }
}
