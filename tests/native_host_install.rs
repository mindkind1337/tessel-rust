//! Installing tessel-host through a fake host that behaves like the real shell commands (it
//! keeps the temporary file, the installed program and their bytes), so every step and every
//! failure is checked without a server. The fake only knows the exact commands of
//! `tessel_host::install`: any other command fails the test.
#[path = "../src/native_host_install.rs"]
mod native_host_install;

use native_host_install::*;
use tessel_host::install::{self as plan, Arch, sha256_hex};

const NONCE: &str = "0123456789abcdef";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Probe,
    Version,
    Upload,
    Digest,
    Activate,
    Abort,
}

/// What the fake host does wrong, if anything.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Fault {
    #[default]
    None,
    /// A byte of the upload changes on the way.
    CorruptUpload,
    /// The upload fails (disk full).
    UploadRefused,
    /// The channel breaks in the middle of the upload (half of it written).
    UploadBroken,
    /// The digest command prints something else than a digest.
    DigestGarbage,
    /// The channel breaks when asking the digest.
    DigestBroken,
    /// The program cannot run (a `noexec` home).
    Noexec,
    /// The program runs but prints another version.
    WrongVersion,
    /// The version command prints more than `MAX_STDOUT` bytes.
    HugeVersion,
    /// The channel breaks when asking the version.
    VersionBroken,
    /// The architecture probe fails.
    ProbeFails,
}

const HOST_SAYS: &str = "PRIVATE-HOST-OUTPUT";

struct FakeHost {
    uname: &'static str,
    fault: Fault,
    part: Option<Vec<u8>>,
    installed: Option<(Arch, Vec<u8>)>,
    steps: Vec<Step>,
    cleanups: usize,
}

impl FakeHost {
    fn new(uname: &'static str, fault: Fault) -> Self {
        Self {
            uname,
            fault,
            part: None,
            installed: None,
            steps: Vec::new(),
            cleanups: 0,
        }
    }

    fn with_installed(mut self, arch: Arch) -> Self {
        let bundle = Bundle::embedded().unwrap();
        self.installed = Some((arch, bundle.program(arch).unwrap().1.to_vec()));
        self
    }

    fn version_line(&self, arch: Arch) -> Vec<u8> {
        let version = if self.fault == Fault::WrongVersion {
            "9.9.9"
        } else {
            HOST_VERSION
        };
        format!(
            r#"{{"arch":"{}","name":"tessel-host","protocol":[{}],"version":"{version}"}}"#,
            arch.name(),
            tessel_host_protocol::VERSION
        )
        .into_bytes()
    }

    fn ok(stdout: Vec<u8>) -> Result<CommandOutput, TransportFailed> {
        Ok(CommandOutput {
            success: true,
            stdout,
        })
    }

    fn failed() -> Result<CommandOutput, TransportFailed> {
        Ok(CommandOutput {
            success: false,
            stdout: HOST_SAYS.as_bytes().to_vec(),
        })
    }

    /// Which step `command` is, for the architecture it names.
    fn recognise(command: &str) -> (Step, Option<Arch>) {
        if command == "uname -m" {
            return (Step::Probe, None);
        }
        for arch in [Arch::X86_64, Arch::Aarch64] {
            let p = plan::plan(HOST_VERSION, arch, NONCE).unwrap();
            let (version, _) = plan::commands(HOST_VERSION, arch).unwrap();
            for (text, step) in [
                (&p.upload, Step::Upload),
                (&p.digest, Step::Digest),
                (&p.activate, Step::Activate),
                (&p.abort, Step::Abort),
                (&version, Step::Version),
            ] {
                if command == text {
                    return (step, Some(arch));
                }
            }
        }
        panic!("a command outside the install plan: {command}");
    }
}

impl HostTransport for FakeHost {
    fn run(&mut self, command: &str, stdin: &[u8]) -> Result<CommandOutput, TransportFailed> {
        let (step, arch) = Self::recognise(command);
        self.steps.push(step);
        assert!(
            stdin.is_empty() || step == Step::Upload,
            "only the upload has input"
        );
        match step {
            Step::Probe => {
                if self.fault == Fault::ProbeFails {
                    return Self::failed();
                }
                Self::ok(format!("{}\n", self.uname).into_bytes())
            }
            Step::Version => {
                if self.fault == Fault::VersionBroken {
                    return Err(TransportFailed);
                }
                match &self.installed {
                    Some((installed, _)) if Some(*installed) == arch => {
                        if self.fault == Fault::HugeVersion {
                            let mut line = self.version_line(*installed);
                            line.resize(MAX_STDOUT + 1, b' ');
                            Self::ok(line)
                        } else {
                            Self::ok(self.version_line(*installed))
                        }
                    }
                    _ => Self::failed(),
                }
            }
            Step::Upload => match self.fault {
                Fault::UploadRefused => Self::failed(),
                Fault::UploadBroken => {
                    self.part = Some(stdin[..stdin.len() / 2].to_vec());
                    Err(TransportFailed)
                }
                Fault::CorruptUpload => {
                    let mut bytes = stdin.to_vec();
                    bytes[1000] ^= 0x01;
                    self.part = Some(bytes);
                    Self::ok(Vec::new())
                }
                _ => {
                    self.part = Some(stdin.to_vec());
                    Self::ok(Vec::new())
                }
            },
            Step::Digest => match self.fault {
                Fault::DigestBroken => Err(TransportFailed),
                Fault::DigestGarbage => Self::ok(HOST_SAYS.as_bytes().to_vec()),
                _ => match &self.part {
                    Some(bytes) => Self::ok(
                        format!("{}  /home/u/.tessel-server/x.part\n", sha256_hex(bytes))
                            .into_bytes(),
                    ),
                    None => Self::failed(),
                },
            },
            Step::Activate => {
                let Some(bytes) = self.part.take() else {
                    return Self::failed();
                };
                let arch = arch.unwrap();
                self.installed = Some((arch, bytes));
                if self.fault == Fault::Noexec {
                    return Self::failed();
                }
                Self::ok(self.version_line(arch))
            }
            Step::Abort => {
                self.part = None;
                Self::ok(Vec::new())
            }
        }
    }

    fn cleanup(&mut self, command: &str) -> Result<CommandOutput, TransportFailed> {
        self.cleanups += 1;
        assert_eq!(
            Self::recognise(command).0,
            Step::Abort,
            "cleanup runs abort only"
        );
        self.run(command, &[])
    }
}

fn install_on(host: &mut FakeHost, known: Option<Arch>) -> Result<HostInstall, InstallError> {
    install(host, &Bundle::embedded().unwrap(), known, NONCE)
}

fn expected(arch: Arch, reused: bool) -> HostInstall {
    let p = plan::plan(HOST_VERSION, arch, NONCE).unwrap();
    HostInstall {
        arch,
        version: HOST_VERSION,
        installed: p.installed,
        connect: p.connect,
        reused,
    }
}

#[test]
fn the_carried_release_matches_its_manifest() {
    let bundle = Bundle::embedded().unwrap();
    for arch in [Arch::X86_64, Arch::Aarch64] {
        let (artifact, bytes) = bundle.program(arch).unwrap();
        assert_eq!(artifact.arch, arch);
        assert_eq!(bytes.len() as u64, artifact.size);
        // A static ELF for the right machine (EM_X86_64 = 62, EM_AARCH64 = 183).
        assert_eq!(&bytes[..4], b"\x7fELF");
        let machine = u16::from_le_bytes([bytes[18], bytes[19]]);
        assert_eq!(machine, if arch == Arch::X86_64 { 62 } else { 183 });
    }
}

#[test]
fn a_fresh_host_gets_the_program_for_its_architecture() {
    for (uname, arch) in [("x86_64", Arch::X86_64), ("aarch64", Arch::Aarch64)] {
        let mut host = FakeHost::new(uname, Fault::None);
        assert_eq!(install_on(&mut host, None), Ok(expected(arch, false)));
        assert_eq!(
            host.steps,
            [
                Step::Probe,
                Step::Version,
                Step::Upload,
                Step::Digest,
                Step::Activate
            ]
        );
        let bundle = Bundle::embedded().unwrap();
        assert_eq!(
            host.installed,
            Some((arch, bundle.program(arch).unwrap().1.to_vec()))
        );
        assert_eq!(host.part, None);
        assert_eq!(host.cleanups, 0);
    }
}

#[test]
fn an_installed_program_is_reused_without_uploading() {
    // Its architecture remembered: one command.
    let mut host = FakeHost::new("x86_64", Fault::None).with_installed(Arch::X86_64);
    assert_eq!(
        install_on(&mut host, Some(Arch::X86_64)),
        Ok(expected(Arch::X86_64, true))
    );
    assert_eq!(host.steps, [Step::Version]);

    // Not remembered: the probe, then the version.
    let mut host = FakeHost::new("arm64", Fault::None).with_installed(Arch::Aarch64);
    assert_eq!(
        install_on(&mut host, None),
        Ok(expected(Arch::Aarch64, true))
    );
    assert_eq!(host.steps, [Step::Probe, Step::Version]);
}

#[test]
fn a_remembered_architecture_that_is_not_installed_probes_then_installs() {
    let mut host = FakeHost::new("x86_64", Fault::None);
    assert_eq!(
        install_on(&mut host, Some(Arch::X86_64)),
        Ok(expected(Arch::X86_64, false))
    );
    // The version is not asked twice for the same architecture.
    assert_eq!(
        host.steps,
        [
            Step::Version,
            Step::Probe,
            Step::Upload,
            Step::Digest,
            Step::Activate
        ]
    );

    // Remembered wrong (the host changed): the probed one is checked, then installed.
    let mut host = FakeHost::new("aarch64", Fault::None);
    assert_eq!(
        install_on(&mut host, Some(Arch::X86_64)),
        Ok(expected(Arch::Aarch64, false))
    );
    assert_eq!(
        host.steps,
        [
            Step::Version,
            Step::Probe,
            Step::Version,
            Step::Upload,
            Step::Digest,
            Step::Activate
        ]
    );
}

#[test]
fn a_broken_channel_while_checking_the_version_stops_everything() {
    for known in [Some(Arch::X86_64), None] {
        let mut host = FakeHost::new("x86_64", Fault::VersionBroken);
        assert_eq!(install_on(&mut host, known), Err(InstallError::Transport));
        assert!(!host.steps.contains(&Step::Upload), "never uploads instead");
    }
}

#[test]
fn a_wrong_version_line_or_one_too_long_means_not_installed() {
    let mut host = FakeHost::new("x86_64", Fault::HugeVersion).with_installed(Arch::X86_64);
    // The installed copy does not count; a fresh install replaces it, and its own version line
    // (normal length) is accepted.
    assert_eq!(
        install_on(&mut host, Some(Arch::X86_64)),
        Ok(expected(Arch::X86_64, false))
    );
}

#[test]
fn an_unknown_architecture_or_a_failed_probe_sends_nothing() {
    let mut host = FakeHost::new("armv7l", Fault::None);
    assert_eq!(
        install_on(&mut host, None),
        Err(InstallError::UnsupportedArch)
    );
    assert_eq!(host.steps, [Step::Probe]);

    let mut host = FakeHost::new("x86_64", Fault::ProbeFails);
    assert_eq!(install_on(&mut host, None), Err(InstallError::ProbeFailed));
    assert_eq!(host.steps, [Step::Probe]);
}

#[test]
fn every_failure_after_the_upload_cleans_up_and_never_connects() {
    for (fault, error) in [
        (Fault::CorruptUpload, InstallError::DigestMismatch),
        (Fault::DigestGarbage, InstallError::DigestMismatch),
        (Fault::DigestBroken, InstallError::Transport),
        (Fault::UploadRefused, InstallError::UploadFailed),
        (Fault::UploadBroken, InstallError::Transport),
        (Fault::Noexec, InstallError::ActivateFailed),
    ] {
        let mut host = FakeHost::new("x86_64", fault);
        assert_eq!(install_on(&mut host, None), Err(error), "{fault:?}");
        assert_eq!(host.cleanups, 1, "{fault:?}: abort runs once");
        assert_eq!(host.steps.last(), Some(&Step::Abort), "{fault:?}");
        assert_eq!(host.part, None, "{fault:?}: no temporary file left");
        if matches!(
            fault,
            Fault::CorruptUpload | Fault::DigestGarbage | Fault::DigestBroken
        ) {
            assert!(
                !host.steps.contains(&Step::Activate),
                "{fault:?}: a wrong digest is never activated"
            );
            assert_eq!(host.installed, None, "{fault:?}");
        }
    }
}

#[test]
fn a_program_that_says_another_version_is_not_connected_to() {
    let mut host = FakeHost::new("x86_64", Fault::WrongVersion);
    assert_eq!(
        install_on(&mut host, None),
        Err(InstallError::VersionMismatch)
    );
    assert_eq!(host.steps.last(), Some(&Step::Activate));
}

#[test]
fn a_damaged_bundle_is_refused_before_anything_is_sent() {
    let manifest = include_str!("../resources/host/v0.1.0/manifest.json");
    let good = Bundle::embedded().unwrap();
    let x86 = good.program(Arch::X86_64).unwrap().1;
    let mut damaged = x86.to_vec();
    damaged[4096] ^= 0xff;
    let aarch64 = good.program(Arch::Aarch64).unwrap().1;
    let bundle = Bundle::from_parts(manifest, &damaged, aarch64).unwrap();
    let mut host = FakeHost::new("x86_64", Fault::None);
    assert_eq!(
        install(&mut host, &bundle, None, NONCE),
        Err(InstallError::BundleInvalid)
    );
    assert!(!host.steps.contains(&Step::Upload));

    // A manifest of another version, or not a manifest, is refused at once.
    assert!(Bundle::from_parts(&manifest.replace("\"0.1.0\"", "\"0.1.1\""), x86, aarch64).is_err());
    assert!(Bundle::from_parts("{}", x86, aarch64).is_err());
}

#[test]
fn a_bad_nonce_runs_no_command() {
    for nonce in [
        "",
        "0123",
        "0123456789ABCDEF",
        "0123456789abcdeg",
        "0123456789abcdef0",
    ] {
        let mut host = FakeHost::new("x86_64", Fault::None);
        assert_eq!(
            install(&mut host, &Bundle::embedded().unwrap(), None, nonce),
            Err(InstallError::InvalidNonce)
        );
        assert!(host.steps.is_empty(), "{nonce}");
    }
    assert_eq!(
        nonce_hex([0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef]),
        NONCE
    );
}

#[test]
fn errors_say_nothing_the_host_printed() {
    let faults = [
        Fault::UploadRefused,
        Fault::DigestGarbage,
        Fault::Noexec,
        Fault::ProbeFails,
    ];
    for fault in faults {
        let mut host = FakeHost::new("x86_64", fault);
        let error = install_on(&mut host, None).unwrap_err();
        for text in [error.code(), error.message(), &error.to_string()] {
            assert!(!text.contains(HOST_SAYS), "{fault:?}: {text}");
        }
    }
}
