//! Putting tessel-host on an SSH host from the desktop. The programs are the release carried in
//! `resources/host/v0.1.1` (never downloaded at run time); the commands, paths and checks all
//! come from `tessel_host::install`, run one by one over a transport the caller provides (one
//! exec channel per command). Nothing a host prints is relayed: errors are fixed codes.
//!
//! Sequence: reuse an installed copy when its version line is right; otherwise `uname -m`,
//! upload, digest compared with the manifest, activate, version line checked. Every failure
//! after the upload runs `abort`; no connect command is ever returned after a wrong digest or a
//! wrong version line.
use tessel_host::install::{self, Arch, Artifact, Manifest};

/// The tessel-host release this desktop carries.
pub const HOST_VERSION: &str = "0.1.1";

/// Most standard output read from one command: a version line, a digest line, `uname -m`.
pub const MAX_STDOUT: usize = 4096;

const MANIFEST: &str = include_str!("../resources/host/v0.1.1/manifest.json");
const X86_64: &[u8] =
    include_bytes!("../resources/host/v0.1.1/tessel-host-x86_64-unknown-linux-musl");
const AARCH64: &[u8] =
    include_bytes!("../resources/host/v0.1.1/tessel-host-aarch64-unknown-linux-musl");

/// What one command did: its exit status and at most `MAX_STDOUT` bytes of standard output.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandOutput {
    pub success: bool,
    pub stdout: Vec<u8>,
}

/// The command could not be run or its channel broke.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransportFailed;

/// One exec channel per call, on the host being installed.
pub trait HostTransport {
    /// Runs `command` exactly as given (a complete shell command for the exec channel: never
    /// quote, split or expand it), writes `stdin` to its standard input and closes it, and
    /// returns its exit status with at most `MAX_STDOUT` bytes of standard output. Standard
    /// error is never returned.
    fn run(&mut self, command: &str, stdin: &[u8]) -> Result<CommandOutput, TransportFailed>;

    /// Runs the one cleanup command `install` gives after a failure past the upload (it removes
    /// the temporary file). Called even when the install was cancelled or ran out of time, so a
    /// transport can run it with a short deadline of its own instead of refusing it. Its outcome
    /// changes nothing to the error reported.
    fn cleanup(&mut self, command: &str) -> Result<CommandOutput, TransportFailed> {
        self.run(command, &[])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallError {
    /// The nonce is not 16 lowercase hexadecimal digits.
    InvalidNonce,
    /// The carried programs do not match their manifest (nothing is sent).
    BundleInvalid,
    /// A command could not be run or its channel broke.
    Transport,
    /// `uname -m` failed.
    ProbeFailed,
    /// Neither x86_64 nor aarch64.
    UnsupportedArch,
    /// The host refused the upload (disk full, home not writable, a symbolic link on the way).
    UploadFailed,
    /// What arrived on the host is not the program sent.
    DigestMismatch,
    /// The program could not be put in place or run (a `noexec` home, for instance).
    ActivateFailed,
    /// The installed program does not say it is this version for this architecture.
    VersionMismatch,
}

impl InstallError {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidNonce => "invalid-nonce",
            Self::BundleInvalid => "bundle-invalid",
            Self::Transport => "transport",
            Self::ProbeFailed => "probe-failed",
            Self::UnsupportedArch => "unsupported-arch",
            Self::UploadFailed => "upload-failed",
            Self::DigestMismatch => "digest-mismatch",
            Self::ActivateFailed => "activate-failed",
            Self::VersionMismatch => "version-mismatch",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::InvalidNonce => "Internal error: invalid install nonce",
            Self::BundleInvalid => "The host program carried by Tessel is damaged",
            Self::Transport => "The connection to the host failed during the install",
            Self::ProbeFailed => "Could not read the host's architecture",
            Self::UnsupportedArch => "This host's architecture is not supported",
            Self::UploadFailed => "The host refused the program upload",
            Self::DigestMismatch => {
                "The program arrived damaged on the host; nothing was installed"
            }
            Self::ActivateFailed => "The host program could not be put in place or started",
            Self::VersionMismatch => "The installed host program is not the expected version",
        }
    }
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for InstallError {}

/// An installed tessel-host, ready to connect to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostInstall {
    pub arch: Arch,
    pub version: &'static str,
    /// The program's path as a shell expression (`$HOME/...`), for display only.
    pub installed: String,
    /// What the SSH channel of a connection runs, exactly as given.
    pub connect: String,
    /// It was already there; nothing was uploaded.
    pub reused: bool,
}

/// The programs of one release and their manifest.
pub struct Bundle<'a> {
    manifest: Manifest,
    x86_64: &'a [u8],
    aarch64: &'a [u8],
}

impl Bundle<'static> {
    /// The release carried in `resources/host/v0.1.1`.
    pub fn embedded() -> Result<Self, InstallError> {
        Bundle::from_parts(MANIFEST, X86_64, AARCH64)
    }
}

impl<'a> Bundle<'a> {
    /// A bundle from a manifest and its two programs. The manifest must be valid and name
    /// `HOST_VERSION`; each program is checked against it before it is sent.
    pub fn from_parts(
        manifest: &str,
        x86_64: &'a [u8],
        aarch64: &'a [u8],
    ) -> Result<Self, InstallError> {
        let manifest = Manifest::parse(manifest).ok_or(InstallError::BundleInvalid)?;
        if manifest.version != HOST_VERSION
            || manifest.artifact(Arch::X86_64).is_none()
            || manifest.artifact(Arch::Aarch64).is_none()
        {
            return Err(InstallError::BundleInvalid);
        }
        Ok(Self {
            manifest,
            x86_64,
            aarch64,
        })
    }

    /// The program for `arch`, only if its size and SHA-256 are the manifest's.
    pub fn program(&self, arch: Arch) -> Result<(&Artifact, &'a [u8]), InstallError> {
        let artifact = self
            .manifest
            .artifact(arch)
            .ok_or(InstallError::BundleInvalid)?;
        let bytes = match arch {
            Arch::X86_64 => self.x86_64,
            Arch::Aarch64 => self.aarch64,
        };
        if install::matches(artifact, bytes) {
            Ok((artifact, bytes))
        } else {
            Err(InstallError::BundleInvalid)
        }
    }
}

/// A nonce for `install` from 8 random bytes the caller drew (getrandom).
pub fn nonce_hex(bytes: [u8; 8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Standard output as text, or nothing when it is longer than `MAX_STDOUT` or not UTF-8.
fn text(output: &CommandOutput) -> Option<&str> {
    if output.stdout.len() > MAX_STDOUT {
        return None;
    }
    std::str::from_utf8(&output.stdout).ok()
}

/// The connect command of an installed copy of this version for `arch`, if its version line
/// is right. A failed or wrong version command means "not installed"; only a broken channel is
/// an error.
fn reuse(
    transport: &mut impl HostTransport,
    arch: Arch,
) -> Result<Option<HostInstall>, InstallError> {
    let plan =
        install::plan(HOST_VERSION, arch, "0000000000000000").ok_or(InstallError::InvalidNonce)?;
    let output = transport
        .run(&plan.version, &[])
        .map_err(|_| InstallError::Transport)?;
    let installed = output.success
        && text(&output).is_some_and(|line| install::check_version(line, HOST_VERSION, arch));
    Ok(installed.then_some(HostInstall {
        arch,
        version: HOST_VERSION,
        installed: plan.installed,
        connect: plan.connect,
        reused: true,
    }))
}

/// Installs this desktop's tessel-host on the host behind `transport`, or finds it already
/// there. `known`: the architecture remembered for this host, if any (it saves `uname -m` when
/// the program is already installed). `nonce`: 16 random hexadecimal digits (`nonce_hex`).
pub fn install<T: HostTransport>(
    transport: &mut T,
    bundle: &Bundle<'_>,
    known: Option<Arch>,
    nonce: &str,
) -> Result<HostInstall, InstallError> {
    // Checked before any command runs.
    let probe_command = install::plan(HOST_VERSION, Arch::X86_64, nonce)
        .ok_or(InstallError::InvalidNonce)?
        .probe;

    if let Some(arch) = known
        && let Some(found) = reuse(transport, arch)?
    {
        return Ok(found);
    }

    let probe = transport
        .run(&probe_command, &[])
        .map_err(|_| InstallError::Transport)?;
    if !probe.success {
        return Err(InstallError::ProbeFailed);
    }
    let arch = text(&probe)
        .and_then(Arch::from_uname)
        .ok_or(InstallError::UnsupportedArch)?;
    if known != Some(arch)
        && let Some(found) = reuse(transport, arch)?
    {
        return Ok(found);
    }

    let plan = install::plan(HOST_VERSION, arch, nonce).ok_or(InstallError::InvalidNonce)?;
    // The bytes are checked against the manifest before anything is sent.
    let (artifact, program) = bundle.program(arch)?;

    // Removes the temporary file; its own outcome changes nothing to the error reported.
    let cleanup = |transport: &mut T| {
        let _ = transport.cleanup(&plan.abort);
    };

    match transport.run(&plan.upload, program) {
        Ok(output) if output.success => {}
        Ok(_) => {
            cleanup(transport);
            return Err(InstallError::UploadFailed);
        }
        Err(_) => {
            cleanup(transport);
            return Err(InstallError::Transport);
        }
    }

    match transport.run(&plan.digest, &[]) {
        Ok(output)
            if output.success
                && text(&output).and_then(install::parse_digest).as_deref()
                    == Some(artifact.sha256.as_str()) => {}
        Ok(_) => {
            cleanup(transport);
            return Err(InstallError::DigestMismatch);
        }
        Err(_) => {
            cleanup(transport);
            return Err(InstallError::Transport);
        }
    }

    let activated = match transport.run(&plan.activate, &[]) {
        Ok(output) => output,
        Err(_) => {
            cleanup(transport);
            return Err(InstallError::Transport);
        }
    };
    if !activated.success {
        cleanup(transport);
        return Err(InstallError::ActivateFailed);
    }
    if !text(&activated).is_some_and(|line| install::check_version(line, HOST_VERSION, arch)) {
        return Err(InstallError::VersionMismatch);
    }
    Ok(HostInstall {
        arch,
        version: HOST_VERSION,
        installed: plan.installed,
        connect: plan.connect,
        reused: false,
    })
}
