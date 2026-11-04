//! Developer-only host release import. Never called by the desktop at runtime.
#[path = "../host_bundle.rs"]
mod host_bundle;

use serde::Deserialize;
use std::{
    fs,
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const REPOSITORY: &str = "github.com/mindkind1337/tessel-host";
const STAGING_MARKER: &str = ".tessel-host-sync";
const STAGING_OWNER: &[u8] = b"tessel-host-sync-v1";

// Interrupted downloads may leave staging behind. Only remove old directories
// positively marked by this tool, after resolving and checking their containment.
fn clean_staging(parent: &Path, age: Duration) -> Result<(), String> {
    let parent = fs::canonicalize(parent).map_err(|_| "Cannot resolve host resources")?;
    for entry in fs::read_dir(&parent).map_err(|_| "Cannot inspect host resources")? {
        let entry = entry.map_err(|_| "Cannot inspect host staging")?;
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with(".host-sync-")
        {
            continue;
        }
        let path = entry.path();
        let meta = fs::symlink_metadata(&path).map_err(|_| "Cannot inspect host staging")?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            continue;
        }
        let resolved = fs::canonicalize(&path).map_err(|_| "Cannot resolve host staging")?;
        if resolved.parent() != Some(parent.as_path()) {
            continue;
        }
        let marker = resolved.join(STAGING_MARKER);
        let Ok(meta) = fs::symlink_metadata(&marker) else {
            continue;
        };
        if !meta.is_file()
            || !meta
                .modified()
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|elapsed| elapsed > age)
        {
            continue;
        }
        if host_bundle::read_bounded(&marker, 32).ok().as_deref() != Some(STAGING_OWNER) {
            continue;
        }
        fs::remove_dir_all(&resolved).map_err(|_| "Cannot clean old host staging")?;
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Release {
    tag_name: String,
    is_draft: bool,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    size: u64,
}

fn gh(args: &[&str], limit: u64) -> Result<Vec<u8>, String> {
    let mut command = Command::new("gh");
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|_| "Install GitHub CLI and authenticate for tessel-host first")?;
    let stdout = child.stdout.take().ok_or("No GitHub CLI output")?;
    let (tx, rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = tx.send(result);
    });
    let deadline = Instant::now() + Duration::from_secs(120);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("GitHub CLI timed out or could not be monitored".into());
            }
        }
    };
    if !status.success() {
        return Err(
            "GitHub CLI failed; check authentication, release tag and asset availability".into(),
        );
    }
    let bytes = rx
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| "GitHub CLI output did not close")?
        .map_err(|_| "Cannot read GitHub CLI output")?;
    if bytes.len() as u64 > limit {
        return Err("GitHub CLI output exceeded its limit".into());
    }
    Ok(bytes)
}

fn expected_assets() -> Vec<(String, u64)> {
    let mut names: Vec<_> = ["manifest.json", "SHA256SUMS", "LICENSE"]
        .into_iter()
        .map(|name| (name.to_owned(), host_bundle::MAX_METADATA))
        .collect();
    names.extend(
        host_bundle::TARGETS
            .iter()
            .map(|(_, target, _)| (format!("tessel-host-{target}"), host_bundle::MAX_BINARY)),
    );
    names
}

fn validate_release(release: &Release, tag: &str) -> Result<(), String> {
    if release.tag_name != tag || release.is_draft || release.assets.len() != 5 {
        return Err("Release is draft, has the wrong tag or too many assets".into());
    }
    for (name, limit) in expected_assets() {
        let assets: Vec<_> = release.assets.iter().filter(|a| a.name == name).collect();
        if assets.len() != 1 || assets[0].size == 0 || assets[0].size > limit {
            return Err(format!(
                "Missing, duplicate or oversized release asset: {name}"
            ));
        }
    }
    Ok(())
}

fn ensure_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => Ok(()),
        Ok(_) => Err("Host resources directory is linked or not a directory".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|_| "Cannot create host resources directory".into())
        }
        Err(_) => Err("Cannot inspect host resources directory".into()),
    }
}

fn publish(staging: &Path, destination: &Path) -> Result<(), String> {
    match fs::symlink_metadata(destination) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        _ => return Err("Bundle destination already exists or cannot be inspected".into()),
    }
    fs::rename(staging, destination).map_err(|_| "Cannot publish verified bundle".into())
}

fn sync(tag: &str) -> Result<(), String> {
    if !host_bundle::valid_tag(tag) {
        return Err("Expected a plain release tag such as v0.1.0".into());
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    if !root.join(".git").exists() {
        return Err("Run host-sync from the developer repository".into());
    }
    let resources = root.join("resources");
    ensure_directory(&resources)?;
    let parent = resources.join("host");
    ensure_directory(&parent)?;
    clean_staging(&parent, Duration::from_secs(7 * 24 * 60 * 60))?;
    let destination = parent.join(tag);
    if fs::symlink_metadata(&destination).is_ok() {
        return Err("Bundle directory already exists; refusing to overwrite it".into());
    }
    let bytes = gh(
        &[
            "release",
            "view",
            tag,
            "--repo",
            REPOSITORY,
            "--json",
            "tagName,isDraft,assets",
        ],
        256 * 1024,
    )?;
    let release: Release =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid GitHub release metadata")?;
    validate_release(&release, tag)?;
    let commit = gh(
        &[
            "api",
            "--hostname",
            "github.com",
            &format!("repos/mindkind1337/tessel-host/commits/{tag}"),
            "--jq",
            ".sha",
        ],
        128,
    )?;
    let commit = std::str::from_utf8(&commit)
        .map_err(|_| "Invalid release commit")?
        .trim();
    if commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid release commit".into());
    }
    let staging = tempfile::Builder::new()
        .prefix(".host-sync-")
        .tempdir_in(&parent)
        .map_err(|_| "Cannot create host staging directory")?;
    fs::write(staging.path().join(STAGING_MARKER), STAGING_OWNER)
        .map_err(|_| "Cannot mark host staging")?;
    let directory = staging
        .path()
        .to_str()
        .ok_or("Unsupported staging directory encoding")?;
    for (name, _) in expected_assets() {
        gh(
            &[
                "release",
                "download",
                tag,
                "--repo",
                REPOSITORY,
                "--pattern",
                &name,
                "--dir",
                directory,
            ],
            8192,
        )?;
    }
    let manifest = host_bundle::verify(staging.path(), tag)?;
    let source = serde_json::json!({"repository":REPOSITORY,"tag":tag,"version":manifest.version,"commit":commit,"artifacts":manifest.artifacts});
    fs::write(
        staging.path().join("SOURCE.json"),
        serde_json::to_vec_pretty(&source).map_err(|_| "Cannot encode bundle provenance")?,
    )
    .map_err(|_| "Cannot write bundle provenance")?;
    fs::remove_file(staging.path().join(STAGING_MARKER))
        .map_err(|_| "Cannot finalize host staging")?;
    publish(staging.path(), &destination)?;
    println!(
        "Verified both host architectures in {}. Review and commit the resources; no Git changes were staged.",
        destination.display()
    );
    Ok(())
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let result = match args.as_slice() {
        [action, tag] if action == "host-sync" => sync(tag),
        _ => Err("Usage: cargo xtask host-sync <release-tag>".into()),
    };
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn staging_cleanup_requires_ownership_age_and_direct_parent() {
        let root = tempfile::tempdir().unwrap();
        let owned = root.path().join(".host-sync-owned");
        let unrelated = root.path().join(".host-sync-user");
        fs::create_dir(&owned).unwrap();
        fs::create_dir(&unrelated).unwrap();
        fs::write(owned.join(STAGING_MARKER), STAGING_OWNER).unwrap();
        fs::write(unrelated.join("keep"), b"user").unwrap();
        clean_staging(root.path(), Duration::from_secs(7 * 24 * 60 * 60)).unwrap();
        assert!(owned.exists());
        clean_staging(root.path(), Duration::ZERO).unwrap();
        assert!(!owned.exists());
        assert!(unrelated.join("keep").exists());
    }
    #[test]
    fn publication_never_replaces_an_existing_bundle() {
        let root = tempfile::tempdir().unwrap();
        let staging = root.path().join("stage");
        let destination = root.path().join("release");
        fs::create_dir(&staging).unwrap();
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("sentinel"), b"existing").unwrap();
        assert!(publish(&staging, &destination).is_err());
        assert_eq!(fs::read(destination.join("sentinel")).unwrap(), b"existing");
        let fresh = root.path().join("new-release");
        assert!(publish(&staging, &fresh).is_ok());
        assert!(fresh.is_dir());
    }

    #[test]
    fn release_metadata_must_match_exact_assets_and_bounds() {
        let mut release = Release {
            tag_name: "v0.1.0".into(),
            is_draft: false,
            assets: expected_assets()
                .into_iter()
                .map(|(name, _)| Asset { name, size: 64 })
                .collect(),
        };
        assert!(validate_release(&release, "v0.1.0").is_ok());
        release.assets.push(Asset {
            name: "unexpected".into(),
            size: 1,
        });
        assert!(validate_release(&release, "v0.1.0").is_err());
        release.assets.pop();
        release.assets[0].size = host_bundle::MAX_METADATA + 1;
        assert!(validate_release(&release, "v0.1.0").is_err());
        release.assets[0].size = 64;
        release.is_draft = true;
        assert!(validate_release(&release, "v0.1.0").is_err());
        release.is_draft = false;
        assert!(validate_release(&release, "v0.2.0").is_err());
        release.assets.pop();
        assert!(validate_release(&release, "v0.1.0").is_err());
    }
}
