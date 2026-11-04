//! The install plan: its checks everywhere, and its commands run for real by `sh` on Linux,
//! installing this very program in a home directory of its own.
use tessel_host::install::*;

const NONCE: &str = "0123456789abcdef";

#[test]
fn architectures_and_tokens() {
    assert_eq!(Arch::from_uname("x86_64\n"), Some(Arch::X86_64));
    assert_eq!(Arch::from_uname("arm64"), Some(Arch::Aarch64));
    assert_eq!(Arch::from_uname("armv7l"), None);
    assert!(plan("0.1.0", Arch::X86_64, NONCE).is_some());
    for version in ["", "1.0;rm -rf ~", "1.0 x", "$(id)", "1.0\"", "UPPER"] {
        assert!(plan(version, Arch::X86_64, NONCE).is_none(), "{version}");
    }
    for nonce in ["", "0123", "0123456789ABCDEF", "0123456789abcdeg"] {
        assert!(plan("0.1.0", Arch::X86_64, nonce).is_none(), "{nonce}");
    }
    let plan = plan("0.1.0", Arch::Aarch64, NONCE).unwrap();
    assert_eq!(
        plan.installed,
        "$HOME/.tessel-server/host/bin/tessel-host-0.1.0-aarch64"
    );
    assert!(!plan.upload.contains("curl") && !plan.upload.contains("sudo"));
}

#[test]
fn digests_versions_and_manifests() {
    let digest = sha256_hex(b"abc");
    assert_eq!(
        digest,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        parse_digest(&format!("{digest}  /home/u/x.part\n")),
        Some(digest.clone())
    );
    assert_eq!(parse_digest("sha256sum: not found"), None);
    let ok = r#"{"arch":"x86_64","name":"tessel-host","protocol":[1],"version":"0.1.0"}"#;
    assert!(check_version(ok, "0.1.0", Arch::X86_64));
    assert!(!check_version(ok, "0.1.1", Arch::X86_64));
    assert!(!check_version(ok, "0.1.0", Arch::Aarch64));
    assert!(!check_version("garbage", "0.1.0", Arch::X86_64));

    let artifact = |arch: Arch, sha: &str| {
        format!(
            r#"{{"arch":"{}","target":"{}","file":"tessel-host-{}","size":3,"sha256":"{sha}"}}"#,
            arch.name(),
            arch.target(),
            arch.target()
        )
    };
    let good = format!(
        r#"{{"version":"0.1.0","artifacts":[{},{}]}}"#,
        artifact(Arch::X86_64, &digest),
        artifact(Arch::Aarch64, &digest)
    );
    let manifest = Manifest::parse(&good).unwrap();
    assert!(matches(manifest.artifact(Arch::X86_64).unwrap(), b"abc"));
    assert!(!matches(manifest.artifact(Arch::X86_64).unwrap(), b"abd"));
    for bad in [
        good.replace(&digest, "00"),
        format!(
            r#"{{"version":"0.1.0","artifacts":[{},{}]}}"#,
            artifact(Arch::X86_64, &digest),
            artifact(Arch::X86_64, &digest)
        ),
        good.replace("tessel-host-x86_64-unknown-linux-musl", "../evil"),
        good.replace("\"version\":\"0.1.0\"", "\"version\":\"0.1.0;x\""),
        good.replace("\"size\":3", "\"size\":0"),
        r#"{"version":"0.1.0","artifacts":[]}"#.to_string(),
    ] {
        assert!(Manifest::parse(&bad).is_none(), "{bad}");
    }
}

#[cfg(target_os = "linux")]
mod on_linux {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output, Stdio};

    fn home(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("tessel-install-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn sh(home: &Path, command: &str, input: &[u8]) -> Output {
        let mut child = Command::new("sh")
            .args(["-c", command])
            .env("HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input).unwrap();
        child.wait_with_output().unwrap()
    }

    fn parts(home: &Path) -> usize {
        std::fs::read_dir(home.join(".tessel-server/host/bin"))
            .map(|dir| {
                dir.flatten()
                    .filter(|e| e.file_name().to_string_lossy().ends_with(".part"))
                    .count()
            })
            .unwrap_or(0)
    }

    #[test]
    fn the_plan_installs_this_program_and_refuses_a_wrong_one() {
        let home = home("plan");
        let program = std::fs::read(env!("CARGO_BIN_EXE_tessel-host")).unwrap();
        let probe = sh(&home, "uname -m", b"");
        let arch = Arch::from_uname(&String::from_utf8_lossy(&probe.stdout)).unwrap();
        let plan = plan(tessel_host::VERSION, arch, NONCE).unwrap();

        // A wrong program: its digest differs, so it is never activated, and abort cleans up.
        assert!(sh(&home, &plan.upload, b"not the program").status.success());
        let digest = parse_digest(&String::from_utf8_lossy(
            &sh(&home, &plan.digest, b"").stdout,
        ))
        .unwrap();
        assert_ne!(digest, sha256_hex(&program));
        assert!(sh(&home, &plan.abort, b"").status.success());
        assert_eq!(parts(&home), 0);

        // The right one.
        assert!(sh(&home, &plan.upload, &program).status.success());
        let digest = parse_digest(&String::from_utf8_lossy(
            &sh(&home, &plan.digest, b"").stdout,
        ))
        .unwrap();
        assert_eq!(digest, sha256_hex(&program));
        let activated = sh(&home, &plan.activate, b"");
        assert!(activated.status.success());
        assert!(check_version(
            &String::from_utf8_lossy(&activated.stdout),
            tessel_host::VERSION,
            arch
        ));
        let installed = home.join(".tessel-server/host/bin").join(format!(
            "tessel-host-{}-{}",
            tessel_host::VERSION,
            arch.name()
        ));
        assert_eq!(std::fs::read(&installed).unwrap(), program);
        assert_eq!(
            std::fs::metadata(&installed).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(home.join(".tessel-server/host/bin"))
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );
        assert_eq!(parts(&home), 0);

        // Installing again over it works the same (an upgrade of the same version).
        assert!(sh(&home, &plan.upload, &program).status.success());
        assert!(sh(&home, &plan.activate, b"").status.success());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_symbolic_link_on_the_way_stops_the_upload() {
        let home = home("link");
        let elsewhere = home.join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        std::fs::create_dir(home.join(".tessel-server")).unwrap();
        symlink(&elsewhere, home.join(".tessel-server/host")).unwrap();
        let plan = plan("0.1.0", Arch::X86_64, NONCE).unwrap();
        assert!(!sh(&home, &plan.upload, b"payload").status.success());
        assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&home);
    }
}
