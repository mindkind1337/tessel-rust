//! Putting `tessel-host` on a host, from the client, over the exec channel it already has: no
//! downloaded script, no curl, package manager, sudo or Node on the host. Coreutils or BusyBox
//! `cat`, `sha256sum`, `chmod` and `mv` are enough. Each command is a fixed template whose only
//! variable parts are a version, an architecture and a nonce, checked to be plain tokens first;
//! every path is under `$HOME`, never one a client or a host chose.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Arch {
    X86_64,
    Aarch64,
}

impl Arch {
    /// From what `uname -m` printed.
    pub fn from_uname(output: &str) -> Option<Self> {
        match output.trim() {
            "x86_64" | "amd64" => Some(Self::X86_64),
            "aarch64" | "arm64" => Some(Self::Aarch64),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
        }
    }

    pub fn target(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64-unknown-linux-musl",
            Self::Aarch64 => "aarch64-unknown-linux-musl",
        }
    }
}

/// One built program of a release.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub arch: Arch,
    pub target: String,
    pub file: String,
    pub size: u64,
    pub sha256: String,
}

/// `manifest.json` of a release, made by `scripts/host-release.sh`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: String,
    pub artifacts: Vec<Artifact>,
}

fn token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
}

fn hex64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl Manifest {
    pub fn parse(text: &str) -> Option<Self> {
        let manifest: Self = serde_json::from_str(text).ok()?;
        let valid = token(&manifest.version)
            && !manifest.artifacts.is_empty()
            && manifest.artifacts.iter().enumerate().all(|(i, a)| {
                a.target == a.arch.target()
                    && a.file == format!("tessel-host-{}", a.target)
                    && hex64(&a.sha256)
                    && a.size > 0
                    && !manifest.artifacts[..i].iter().any(|b| b.arch == a.arch)
            });
        valid.then_some(manifest)
    }

    pub fn artifact(&self, arch: Arch) -> Option<&Artifact> {
        self.artifacts.iter().find(|a| a.arch == arch)
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The program about to be sent is the one the manifest names.
pub fn matches(artifact: &Artifact, bytes: &[u8]) -> bool {
    bytes.len() as u64 == artifact.size && sha256_hex(bytes) == artifact.sha256
}

/// The commands, in order. `upload` reads the program on its standard input; the client
/// compares what `digest` prints with the manifest (`parse_digest`) and runs `activate` only if
/// they are the same, `abort` otherwise. `activate` ends by running the installed program's
/// `version`, which the client checks (`check_version`) before attaching.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub probe: String,
    pub upload: String,
    pub digest: String,
    pub activate: String,
    pub abort: String,
    pub installed: String,
}

const DIRS: [&str; 3] = [
    "$HOME/.tessel-server",
    "$HOME/.tessel-server/host",
    "$HOME/.tessel-server/host/bin",
];

/// `nonce`: 16 random hexadecimal digits, so two installs never share a temporary file.
pub fn plan(version: &str, arch: Arch, nonce: &str) -> Option<Plan> {
    if !token(version)
        || nonce.len() != 16
        || !nonce
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return None;
    }
    let bin = DIRS[2];
    let installed = format!("{bin}/tessel-host-{version}-{}", arch.name());
    let part = format!("{bin}/.tessel-host-{version}-{}.{nonce}.part", arch.name());
    let no_links = DIRS
        .iter()
        .map(|d| format!("[ ! -L \"{d}\" ]"))
        .collect::<Vec<_>>()
        .join(" && ");
    Some(Plan {
        probe: "uname -m".to_owned(),
        upload: format!(
            "umask 077 && {no_links} && mkdir -p \"{bin}\" && {no_links} && cat > \"{part}\""
        ),
        digest: format!(
            "if command -v sha256sum >/dev/null 2>&1; then sha256sum \"{part}\"; else busybox sha256sum \"{part}\"; fi"
        ),
        activate: format!(
            "chmod 700 \"{part}\" && mv -f \"{part}\" \"{installed}\" && \"{installed}\" version"
        ),
        abort: format!("rm -f \"{part}\""),
        installed,
    })
}

/// The digest at the start of `sha256sum`'s output.
pub fn parse_digest(output: &str) -> Option<String> {
    let first = output.split_whitespace().next()?.to_ascii_lowercase();
    hex64(&first).then_some(first)
}

/// What the installed program said about itself is what was meant to be installed.
pub fn check_version(output: &str, version: &str, arch: Arch) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) else {
        return false;
    };
    value["name"] == "tessel-host"
        && value["version"] == version
        && value["arch"] == arch.name()
        && value["protocol"]
            .as_array()
            .is_some_and(|versions| versions.iter().any(|v| v == tessel_host_protocol::VERSION))
}
