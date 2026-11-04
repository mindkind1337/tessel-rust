use std::{path::Path, process::Command};

fn git(args: &[&str]) -> Option<String> {
    let mut command = Command::new("git");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command.args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_owned())
}
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    for name in ["HEAD", "packed-refs"] {
        if let Some(path) = git(&["rev-parse", "--git-path", name])
            && Path::new(&path).exists()
        {
            // Watching an absent packed-refs file keeps Cargo's build script
            // dirty on every invocation. Loose refs are watched below; packing
            // them removes the loose file and causes a fresh watch set.
            println!("cargo:rerun-if-changed={path}");
        }
    }
    if let Some(reference) = git(&["symbolic-ref", "-q", "HEAD"])
        && let Some(path) = git(&["rev-parse", "--git-path", &reference])
    {
        let mut watched = Path::new(&path);
        // An unborn branch has no loose ref yet. Watch its existing parent so
        // its first commit is detected without registering a missing path.
        while !watched.exists() {
            let Some(parent) = watched.parent() else {
                break;
            };
            watched = parent;
        }
        if watched.exists() {
            println!("cargo:rerun-if-changed={}", watched.display());
        }
    }
    let build = git(&["rev-list", "--count", "HEAD"])
        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()));
    let commit = git(&["rev-parse", "--short=7", "HEAD"])
        .filter(|s| s.len() >= 7 && s.len() <= 40 && s.bytes().all(|b| b.is_ascii_hexdigit()));
    let label = match (build, commit) {
        (Some(build), Some(commit)) => format!("b{build} ({commit})"),
        _ => "source archive".into(),
    };
    println!("cargo:rustc-env=TESSEL_BUILD={label}");
}
