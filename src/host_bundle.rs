//! Verification of the host release bundle before it enters desktop resources.
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, io::Read, path::Path};

pub const MAX_BINARY: u64 = 64 * 1024 * 1024;
pub const MAX_METADATA: u64 = 64 * 1024;
pub const TARGETS: [(&str, &str, u16); 2] = [
    ("x86_64", "x86_64-unknown-linux-musl", 62),
    ("aarch64", "aarch64-unknown-linux-musl", 183),
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: String,
    pub artifacts: Vec<Artifact>,
}

#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub arch: String,
    pub target: String,
    pub file: String,
    pub size: u64,
    pub sha256: String,
}

pub fn valid_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= 64
        && tag.as_bytes()[0].is_ascii_alphanumeric()
        && tag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        && !tag.contains("..")
}

pub fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let meta = fs::symlink_metadata(path).map_err(|_| "Missing bundle file")?;
    if !meta.is_file() || meta.len() > limit {
        return Err("Bundle file is linked, nonregular or oversized".into());
    }
    let file = fs::File::open(path).map_err(|_| "Cannot read bundle file")?;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read bundle file")?;
    if bytes.len() as u64 > limit {
        return Err("Bundle file grew beyond limit".into());
    }
    Ok(bytes)
}

pub fn manifest(directory: &Path, tag: &str) -> Result<Manifest, String> {
    if !valid_tag(tag) {
        return Err("Invalid release tag".into());
    }
    let bytes = read_bounded(&directory.join("manifest.json"), MAX_METADATA)?;
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|_| "Invalid host manifest")?;
    if !valid_tag(&manifest.version)
        || manifest.version.len() > 32
        || !manifest
            .version
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
        || !manifest.version.as_bytes()[0].is_ascii_digit()
        || tag != format!("v{}", manifest.version)
        || manifest.artifacts.len() != TARGETS.len()
    {
        return Err("Manifest version or architectures do not match the release".into());
    }
    for (arch, target, _) in TARGETS {
        let matching: Vec<_> = manifest
            .artifacts
            .iter()
            .filter(|a| a.arch == arch)
            .collect();
        if matching.len() != 1 {
            return Err("Missing or duplicate host architecture".into());
        }
        let a = matching[0];
        if a.target != target
            || a.file != format!("tessel-host-{target}")
            || a.size == 0
            || a.size > MAX_BINARY
            || !hex_digest(&a.sha256)
        {
            return Err("Invalid host artifact metadata".into());
        }
    }
    Ok(manifest)
}

fn hex_digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn verify(directory: &Path, tag: &str) -> Result<Manifest, String> {
    let manifest = manifest(directory, tag)?;
    let sums = read_bounded(&directory.join("SHA256SUMS"), MAX_METADATA)?;
    let sums = std::str::from_utf8(&sums).map_err(|_| "Invalid checksums encoding")?;
    let mut checksums = BTreeMap::new();
    for line in sums.lines().filter(|l| !l.trim().is_empty()) {
        let (digest, file) = line.split_once("  ").ok_or("Invalid checksum line")?;
        if !hex_digest(digest) || checksums.insert(file, digest).is_some() {
            return Err("Invalid or duplicate checksum entry".into());
        }
    }
    if checksums.len() != TARGETS.len() {
        return Err("Unexpected checksum entries".into());
    }
    let license = read_bounded(&directory.join("LICENSE"), MAX_METADATA)?;
    if !std::str::from_utf8(&license)
        .is_ok_and(|text| text.contains("MIT License") && text.contains("Copyright"))
    {
        return Err("Missing host MIT license attribution".into());
    }
    for a in &manifest.artifacts {
        let bytes = read_bounded(&directory.join(&a.file), MAX_BINARY)?;
        let digest = format!("{:x}", Sha256::digest(&bytes));
        if bytes.len() as u64 != a.size
            || digest != a.sha256
            || checksums.get(a.file.as_str()).copied() != Some(a.sha256.as_str())
        {
            return Err("Host artifact size or checksum mismatch".into());
        }
        let machine = TARGETS
            .iter()
            .find(|(arch, _, _)| *arch == a.arch)
            .unwrap()
            .2;
        if !static_elf(&bytes, machine) {
            return Err("Host artifact is not ELF64 for its declared architecture".into());
        }
    }
    Ok(manifest)
}

fn static_elf(bytes: &[u8], machine: u16) -> bool {
    if bytes.len() < 64 || bytes.get(..6) != Some(b"\x7fELF\x02\x01") {
        return false;
    }
    let u16_at = |at| u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap());
    let u64_at = |at| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    if !matches!(u16_at(16), 2 | 3) || u16_at(18) != machine || u16_at(54) != 56 || u16_at(56) == 0
    {
        return false;
    }
    let Ok(start) = usize::try_from(u64_at(32)) else {
        return false;
    };
    let Some(end) = start.checked_add(usize::from(u16_at(56)) * 56) else {
        return false;
    };
    let Some(headers) = bytes.get(start..end) else {
        return false;
    };
    let mut load = false;
    let mut dynamic_budget = bytes.len() as u64;
    for header in headers.as_chunks::<56>().0 {
        let kind = u32::from_le_bytes(header[..4].try_into().unwrap());
        if kind == 3 {
            return false;
        }
        load |= kind == 1;
        if kind != 2 {
            continue;
        }
        let offset = u64::from_le_bytes(header[8..16].try_into().unwrap());
        let size = u64::from_le_bytes(header[32..40].try_into().unwrap());
        let Some(end) = offset.checked_add(size) else {
            return false;
        };
        if end > bytes.len() as u64 || size == 0 || size % 16 != 0 {
            return false;
        }
        // Repeated/overlapping dynamic headers must not multiply parsing work.
        if size > dynamic_budget {
            return false;
        }
        dynamic_budget -= size;
        let mut terminated = false;
        for entry in bytes[offset as usize..end as usize].as_chunks::<16>().0 {
            match u64::from_le_bytes(entry[..8].try_into().unwrap()) {
                0 => {
                    terminated = true;
                    break;
                }
                1 => return false,
                _ => {}
            }
        }
        if !terminated {
            return false;
        }
    }
    load
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checked_in_release_contains_static_verified_programs() {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/host/v0.1.0");
        assert!(verify(&directory, "v0.1.0").is_ok());
    }
    fn bundle(path: &Path) {
        let mut artifacts = Vec::new();
        let mut sums = String::new();
        for (arch, target, machine) in TARGETS {
            let mut bytes = vec![0; 120];
            bytes[..4].copy_from_slice(b"\x7fELF");
            bytes[4] = 2;
            bytes[5] = 1;
            bytes[16..18].copy_from_slice(&2u16.to_le_bytes());
            bytes[18..20].copy_from_slice(&machine.to_le_bytes());
            bytes[32..40].copy_from_slice(&64u64.to_le_bytes());
            bytes[54..56].copy_from_slice(&56u16.to_le_bytes());
            bytes[56..58].copy_from_slice(&1u16.to_le_bytes());
            bytes[64..68].copy_from_slice(&1u32.to_le_bytes());
            let file = format!("tessel-host-{target}");
            let digest = format!("{:x}", Sha256::digest(&bytes));
            fs::write(path.join(&file), &bytes).unwrap();
            sums.push_str(&format!("{digest}  {file}\n"));
            artifacts.push(serde_json::json!({"arch":arch,"target":target,"file":file,"size":bytes.len(),"sha256":digest}));
        }
        fs::write(
            path.join("manifest.json"),
            serde_json::to_vec(&serde_json::json!({"version":"0.1.0","artifacts":artifacts}))
                .unwrap(),
        )
        .unwrap();
        fs::write(path.join("SHA256SUMS"), sums).unwrap();
        fs::write(
            path.join("LICENSE"),
            "MIT License\nCopyright synthetic fixture\n",
        )
        .unwrap();
    }
    #[test]
    fn verifies_both_architectures_and_refuses_tampering() {
        let dir = tempfile::tempdir().unwrap();
        bundle(dir.path());
        assert!(verify(dir.path(), "v0.1.0").is_ok());
        assert!(verify(dir.path(), "0.1.0").is_err());
        assert!(verify(dir.path(), "v0.2.0").is_err());
        fs::write(
            dir.path().join("tessel-host-x86_64-unknown-linux-musl"),
            b"replacement",
        )
        .unwrap();
        assert!(verify(dir.path(), "v0.1.0").is_err());
    }
    #[test]
    fn metadata_cannot_hide_wrong_elf_architecture_or_escape_the_bundle() {
        let dir = tempfile::tempdir().unwrap();
        bundle(dir.path());
        let manifest_path = dir.path().join("manifest.json");
        let mut m: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        let file = m["artifacts"][0]["file"].as_str().unwrap().to_owned();
        let path = dir.path().join(&file);
        let mut bytes = fs::read(&path).unwrap();
        bytes[18..20].copy_from_slice(&183u16.to_le_bytes());
        fs::write(&path, &bytes).unwrap();
        m["artifacts"][0]["sha256"] = format!("{:x}", Sha256::digest(&bytes)).into();
        fs::write(&manifest_path, serde_json::to_vec(&m).unwrap()).unwrap();
        let sums: String = m["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| {
                format!(
                    "{}  {}\n",
                    a["sha256"].as_str().unwrap(),
                    a["file"].as_str().unwrap()
                )
            })
            .collect();
        fs::write(dir.path().join("SHA256SUMS"), sums).unwrap();
        assert!(matches!(verify(dir.path(), "v0.1.0"), Err(e) if e.contains("ELF64")));
        m["artifacts"][0]["file"] = "../outside".into();
        fs::write(&manifest_path, serde_json::to_vec(&m).unwrap()).unwrap();
        assert!(manifest(dir.path(), "v0.1.0").is_err());
    }
    #[test]
    fn rejects_paths_duplicates_and_missing_license() {
        for tag in ["", "..", "a/../b", "-f", "v1;echo", "v1\\x"] {
            assert!(!valid_tag(tag));
        }
        let dir = tempfile::tempdir().unwrap();
        bundle(dir.path());
        fs::remove_file(dir.path().join("LICENSE")).unwrap();
        assert!(verify(dir.path(), "v0.1.0").is_err());
        bundle(dir.path());
        let path = dir.path().join("manifest.json");
        let mut m: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        m["artifacts"][1] = m["artifacts"][0].clone();
        fs::write(&path, serde_json::to_vec(&m).unwrap()).unwrap();
        assert!(manifest(dir.path(), "v0.1.0").is_err());
    }

    #[test]
    fn elf_refuses_dynamic_objects_core_files_and_unbounded_headers() {
        let dir = tempfile::tempdir().unwrap();
        bundle(dir.path());
        let base = fs::read(dir.path().join("tessel-host-x86_64-unknown-linux-musl")).unwrap();
        assert!(static_elf(&base, 62));
        for kind in [0u16, 1, 4] {
            let mut bytes = base.clone();
            bytes[16..18].copy_from_slice(&kind.to_le_bytes());
            assert!(!static_elf(&bytes, 62));
        }
        let mut bytes = base.clone();
        bytes[64..68].copy_from_slice(&3u32.to_le_bytes());
        assert!(!static_elf(&bytes, 62));
        let mut bytes = base.clone();
        bytes[32..40].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(!static_elf(&bytes, 62));
        let mut bytes = base;
        bytes.resize(208, 0);
        bytes[56..58].copy_from_slice(&2u16.to_le_bytes());
        bytes[120..124].copy_from_slice(&2u32.to_le_bytes());
        bytes[128..136].copy_from_slice(&176u64.to_le_bytes());
        bytes[152..160].copy_from_slice(&32u64.to_le_bytes());
        assert!(static_elf(&bytes, 62));
        bytes[176..184].copy_from_slice(&1u64.to_le_bytes());
        assert!(!static_elf(&bytes, 62));
        bytes[128..136].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(!static_elf(&bytes, 62));
    }
}
