//! Release tooling for tessel-host, in Rust like the rest:
//!   release [--out DIR]                  build both static musl programs, check them, write
//!                                        manifest.json and SHA256SUMS
//!   accept  [--out DIR] [--qemu PATH]    run each program for real (a foreign architecture
//!                                        under a user-mode emulator): version, daemon, bridge,
//!                                        hello, host.info, a killed bridge, the same epoch
//! Runs on Linux. The aarch64 build links with rust-lld unless a linker is configured.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tessel_host::install::{Arch, Artifact, Manifest, check_version, sha256_hex};
use tessel_host_protocol::*;

const ARCHES: [Arch; 2] = [Arch::X86_64, Arch::Aarch64];
/// Crates a host program must never pull in: a desktop toolkit, a web view, a JavaScript runtime.
const FORBIDDEN: [&str; 9] = [
    "gtk", "glib", "webkit", "wry", "egui", "eframe", "napi", "node", "v8",
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let out = PathBuf::from(
        flag("--out").unwrap_or_else(|| format!("dist/host/{}", tessel_host::VERSION)),
    );
    let result = match args.first().map(String::as_str) {
        Some("release") => release(&out),
        Some("accept") => accept(&out, flag("--qemu").map(PathBuf::from)),
        _ => Err(
            "usage: tessel-host-xtask release [--out DIR] | accept [--out DIR] [--qemu PATH]"
                .into(),
        ),
    };
    if let Err(error) = result {
        eprintln!("xtask: {error}");
        std::process::exit(1);
    }
}

type Result<T> = std::result::Result<T, String>;

fn target_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target"))
}

fn release(out: &Path) -> Result<()> {
    check_dependency_tree()?;
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let mut artifacts = Vec::new();
    for arch in ARCHES {
        let mut cargo = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
        cargo.args([
            "build",
            "--locked",
            "--release",
            "-p",
            "tessel-host",
            "--target",
            arch.target(),
        ]);
        cargo.env("RUSTFLAGS", "-C strip=symbols");
        let linker = format!(
            "CARGO_TARGET_{}_LINKER",
            arch.target().to_uppercase().replace('-', "_")
        );
        if arch == Arch::Aarch64 && std::env::var_os(&linker).is_none() {
            cargo.env(&linker, "rust-lld");
        }
        if !cargo.status().map_err(|e| e.to_string())?.success() {
            return Err(format!("build failed for {}", arch.target()));
        }
        let built = target_dir().join(arch.target()).join("release/tessel-host");
        let bytes = std::fs::read(&built).map_err(|e| format!("{}: {e}", built.display()))?;
        check_elf(&bytes, arch)?;
        let file = format!("tessel-host-{}", arch.target());
        std::fs::write(out.join(&file), &bytes).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(out.join(&file), std::fs::Permissions::from_mode(0o755))
                .map_err(|e| e.to_string())?;
        }
        artifacts.push(Artifact {
            arch,
            target: arch.target().to_owned(),
            file,
            size: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
        });
    }
    let manifest = Manifest {
        version: tessel_host::VERSION.to_owned(),
        artifacts,
    };
    let text = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    Manifest::parse(&text).ok_or("the manifest does not check")?;
    std::fs::write(out.join("manifest.json"), text + "\n").map_err(|e| e.to_string())?;
    let sums: String = manifest
        .artifacts
        .iter()
        .map(|a| format!("{}  {}\n", a.sha256, a.file))
        .collect();
    std::fs::write(out.join("SHA256SUMS"), sums).map_err(|e| e.to_string())?;
    for a in &manifest.artifacts {
        println!(
            "{}  {} ({} bytes, static, {})",
            a.sha256,
            a.file,
            a.size,
            a.arch.name()
        );
    }
    Ok(())
}

fn check_dependency_tree() -> Result<()> {
    let output = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args([
            "tree",
            "--locked",
            "-p",
            "tessel-host",
            "-e",
            "normal,build",
            "--prefix",
            "none",
            "--target",
            "all",
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err("cargo tree failed".into());
    }
    let tree = String::from_utf8_lossy(&output.stdout);
    for line in tree.lines() {
        let name = line.split_whitespace().next().unwrap_or("");
        if let Some(bad) = FORBIDDEN
            .iter()
            .find(|f| name.split(['-', '_']).any(|part| part == **f))
        {
            return Err(format!("tessel-host depends on {name} ({bad})"));
        }
    }
    Ok(())
}

/// A static program for `arch`: the right machine, no program interpreter, no shared library.
fn check_elf(bytes: &[u8], arch: Arch) -> Result<()> {
    let u16_at = |o: usize| {
        bytes
            .get(o..o + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
    };
    let u32_at = |o: usize| {
        bytes
            .get(o..o + 4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
    };
    let u64_at = |o: usize| {
        bytes
            .get(o..o + 8)
            .map(|b| u64::from_le_bytes(b.try_into().unwrap()))
    };
    let bad = |why: &str| Err(format!("{}: {why}", arch.target()));
    if bytes.get(..6) != Some(b"\x7fELF\x02\x01".as_slice()) {
        return bad("not a 64-bit little-endian ELF file");
    }
    let machine = match arch {
        Arch::X86_64 => 62,
        Arch::Aarch64 => 183,
    };
    if !matches!(u16_at(16), Some(2 | 3)) || u16_at(18) != Some(machine) {
        return bad("not an executable for this architecture");
    }
    let (Some(phoff), Some(phentsize), Some(phnum)) = (u64_at(32), u16_at(54), u16_at(56)) else {
        return bad("truncated header");
    };
    for i in 0..usize::from(phnum) {
        let at = phoff as usize + i * usize::from(phentsize);
        match u32_at(at) {
            Some(3) => return bad("has a program interpreter (dynamically linked)"),
            Some(2) => {
                let (Some(offset), Some(size)) = (u64_at(at + 8), u64_at(at + 32)) else {
                    return bad("truncated program header");
                };
                for entry in (offset..offset + size).step_by(16) {
                    match u64_at(entry as usize) {
                        Some(0) => break,
                        Some(1) => return bad("needs a shared library"),
                        Some(_) => {}
                        None => return bad("truncated dynamic section"),
                    }
                }
            }
            Some(_) => {}
            None => return bad("truncated program headers"),
        }
    }
    Ok(())
}

fn native(arch: Arch) -> bool {
    arch.name() == std::env::consts::ARCH
}

struct Runner {
    program: PathBuf,
    qemu: Option<PathBuf>,
}

impl Runner {
    fn command(&self, home: &Path, args: &[&str]) -> Command {
        let mut command = match &self.qemu {
            Some(qemu) => {
                let mut c = Command::new(qemu);
                c.arg(&self.program);
                c
            }
            None => Command::new(&self.program),
        };
        command.args(args).env("HOME", home);
        command
    }
}

struct Session {
    child: Child,
    next: u64,
}

impl Session {
    fn open(runner: &Runner, home: &Path) -> Result<Self> {
        let child = runner
            .command(home, &["connect", "--stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(Self { child, next: 0 })
    }

    fn ask(&mut self, message: ClientMessage) -> Result<ReplyMessage> {
        self.next += 1;
        let stdin = self.child.stdin.as_mut().ok_or("no stdin")?;
        write_frame(
            stdin,
            &ClientFrame::request(self.next, message),
            MAX_FRAME_BYTES,
        )
        .map_err(|e| e.to_string())?;
        stdin.flush().map_err(|e| e.to_string())?;
        let stdout = self.child.stdout.as_mut().ok_or("no stdout")?;
        let reply = read_frame::<HostFrame>(stdout, MAX_FRAME_BYTES)
            .map_err(|e| e.to_string())?
            .ok_or("the bridge closed")?;
        if reply.reply_to != Some(self.next) {
            return Err("a reply to another request".into());
        }
        let ok = reply
            .ok
            .ok_or_else(|| format!("refused: {:?}", reply.error))?;
        ReplyMessage::from_raw(&ok)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "an unknown reply".into())
    }
}

fn hello() -> ClientMessage {
    ClientMessage::Hello(Hello {
        versions: vec![VERSION],
        capabilities: vec!["pty".into()],
        budgets: Budgets::SLOW_HOST,
    })
}

fn accept(out: &Path, qemu: Option<PathBuf>) -> Result<()> {
    let text = std::fs::read_to_string(out.join("manifest.json")).map_err(|e| e.to_string())?;
    let manifest = Manifest::parse(&text).ok_or("invalid manifest")?;
    for artifact in &manifest.artifacts {
        let program = out.join(&artifact.file);
        let bytes = std::fs::read(&program).map_err(|e| e.to_string())?;
        if !tessel_host::install::matches(artifact, &bytes) {
            return Err(format!("{} does not match its digest", artifact.file));
        }
        let qemu = if native(artifact.arch) {
            None
        } else {
            Some(
                qemu.clone()
                    .ok_or_else(|| format!("{} needs --qemu", artifact.arch.name()))?,
            )
        };
        let how = if qemu.is_some() { "emulated" } else { "native" };
        let runner = Runner {
            program: std::fs::canonicalize(&program).map_err(|e| e.to_string())?,
            qemu,
        };
        accept_one(&runner, artifact.arch, &manifest.version)
            .map_err(|e| format!("{}: {e}", artifact.file))?;
        println!(
            "accepted {} ({how}): version, daemon, bridge, hello, host.info, killed bridge, same epoch",
            artifact.file
        );
    }
    Ok(())
}

fn accept_one(runner: &Runner, arch: Arch, version: &str) -> Result<()> {
    let home = std::env::temp_dir().join(format!(
        "tessel-host-accept-{}-{}",
        arch.name(),
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).map_err(|e| e.to_string())?;
    let result = (|| {
        let printed = runner
            .command(&home, &["version"])
            .output()
            .map_err(|e| e.to_string())?;
        if !check_version(&String::from_utf8_lossy(&printed.stdout), version, arch) {
            return Err("its version line is not this release's".to_string());
        }
        // The daemon in the foreground (an emulated program cannot start itself again).
        let mut daemon = runner
            .command(&home, &["serve"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        let socket = home
            .join(".tessel-server/host")
            .join(version)
            .join("daemon.sock");
        let deadline = Instant::now() + Duration::from_secs(30);
        while !socket.exists() {
            if Instant::now() > deadline {
                let _ = daemon.kill();
                return Err("the daemon did not start".into());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let outcome = (|| {
            let mut first = Session::open(runner, &home)?;
            let ReplyMessage::HelloOk(agreed) = first.ask(hello())? else {
                return Err("no hello_ok".into());
            };
            let ReplyMessage::HostInfo(info) = first.ask(ClientMessage::HostInfo)? else {
                return Err("no host_info".into());
            };
            if info.arch != arch.name() || info.version != version {
                return Err(format!("host.info says {} {}", info.arch, info.version));
            }
            let _ = first.child.kill();
            let _ = first.child.wait();
            let mut second = Session::open(runner, &home)?;
            let ReplyMessage::HelloOk(again) = second.ask(hello())? else {
                return Err("no hello_ok".into());
            };
            if again.host_epoch != agreed.host_epoch {
                return Err("another epoch after a killed bridge".into());
            }
            if !matches!(second.ask(ClientMessage::Ping)?, ReplyMessage::Pong) {
                return Err("no pong".into());
            }
            drop(second.child.stdin.take());
            let _ = second.child.wait();
            Ok(())
        })();
        let _ = daemon.kill();
        let _ = daemon.wait();
        outcome
    })();
    let _ = std::fs::remove_dir_all(&home);
    result
}
