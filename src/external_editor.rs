//! Explicit external open from the local project explorer.
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const SYSTEM_TYPES: &str = "mp4 m4v webm mov avi mkv wmv mpg mpeg 3gp ogv gif png jpg jpeg webp bmp ico svg tif tiff heic avif mp3 wav ogg oga flac m4a aac wma opus mid midi pdf docx xlsx pptx odt ods odp epub zip 7z rar tar gz tgz bz2 xz ttf otf woff woff2 psd";

fn system_document(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| {
            SYSTEM_TYPES
                .split_ascii_whitespace()
                .any(|e| ext.eq_ignore_ascii_case(e))
        })
}

fn validate(path: &Path) -> Result<(), String> {
    let raw = path.to_str().ok_or("Invalid local file path.")?;
    if !path.is_absolute()
        || raw.encode_utf16().count() > 1024
        || raw
            .as_bytes()
            .get(..2)
            .is_some_and(|p| p.iter().all(|b| matches!(b, b'/' | b'\\')))
        || path.components().any(|c| match c {
            std::path::Component::Normal(n) => n.to_str().is_none_or(|s| {
                !crate::file_actions::name_ok(s)
                    || matches!(
                        s.split('.')
                            .next()
                            .unwrap_or("")
                            .to_ascii_lowercase()
                            .as_str(),
                        "com0" | "lpt0" | "clock$"
                    )
            }),
            std::path::Component::ParentDir => true,
            _ => false,
        })
    {
        return Err("Invalid local file path.".into());
    }
    Ok(())
}

fn validate_file(path: &Path) -> Result<(), String> {
    validate(path)?;
    if crate::chat::citations::blocked_program(path) {
        return Err("This file type cannot be opened externally.".into());
    }
    Ok(())
}

#[cfg(windows)]
fn code_command() -> Option<Command> {
    let search = std::env::var_os("PATH").unwrap_or_default();
    let bin = std::env::split_paths(&search)
        .filter(|p| p.is_absolute())
        .find(|p| p.join("code.cmd").is_file())?;
    let wrapper = bin.join("code.cmd");
    if wrapper.metadata().ok()?.len() > 64 * 1024 {
        return None;
    }
    let text = std::fs::read_to_string(wrapper).ok()?;
    let install = bin.parent()?.canonicalize().ok()?;
    // Read the installed CLI's versioned location; never execute wrapper text.
    let cli = text
        .split('"')
        .filter_map(|s| s.strip_prefix("%~dp0"))
        .find(|s| {
            s.to_ascii_lowercase()
                .ends_with(r"\resources\app\out\cli.js")
        })?;
    let cli = bin.join(cli).canonicalize().ok()?;
    let exe = install.join("Code.exe");
    if !cli.starts_with(&install) || !cli.is_file() || !exe.is_file() {
        return None;
    }
    let mut cmd = Command::new(exe);
    crate::agent_env::clean_command(&mut cmd);
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x08000000)
        .arg(cli)
        .env("ELECTRON_RUN_AS_NODE", "1")
        .env_remove("VSCODE_DEV");
    Some(cmd)
}

#[cfg(not(windows))]
fn code_command() -> Option<Command> {
    let search = std::env::var_os("PATH")?;
    let exe = std::env::split_paths(&search)
        .filter(|p| p.is_absolute())
        .map(|p| p.join("code"))
        .find(|p| p.is_file())?;
    let mut cmd = Command::new(exe);
    crate::agent_env::clean_command(&mut cmd);
    Some(cmd)
}

fn open_in_code(path: &Path) -> bool {
    let Some(mut cmd) = code_command() else {
        return false;
    };
    let Ok(mut child) = cmd
        .arg("-g")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

#[cfg(windows)]
fn system_open(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        core::{PCWSTR, w},
    };
    let path: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(path.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize <= 32 {
        Err("The system could not open this file.".into())
    } else {
        Ok(())
    }
}
#[cfg(not(windows))]
fn system_open(path: &Path) -> Result<(), String> {
    open::that_detached(path).map_err(|e| e.to_string())
}

pub(crate) fn open(root: &Path, file: &Path) -> Result<Option<PathBuf>, String> {
    open_with(root, file, open_in_code, system_open)
}

fn open_with(
    root: &Path,
    file: &Path,
    code: impl FnOnce(&Path) -> bool,
    system: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<Option<PathBuf>, String> {
    let root = dunce::simplified(root);
    let file = dunce::simplified(file);
    validate(root)?;
    validate_file(file)?;
    if !file.starts_with(root) {
        return Err("File is outside the project.".into());
    }
    let real = crate::files::confined(root, file)?;
    let real = dunce::simplified(&real);
    validate_file(real)?;
    if !real.is_file() {
        return Err("Select a regular file.".into());
    }
    if code(real) {
        return Ok(None);
    }
    if system_document(file) && system_document(real) {
        system(real)?;
        return Ok(None);
    }
    Ok(Some(real.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    #[ignore = "requires the installed VS Code CLI on PATH"]
    fn installed_code_cli_reports_version_without_opening_a_window() {
        let output = code_command()
            .expect("installed VS Code")
            .arg("--version")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .any(|line| line.trim().starts_with("1."))
        );
    }

    #[test]
    fn explorer_external_open_routes_originals_and_blocks_programs_before_launch() {
        let root = tempfile::tempdir().unwrap();
        for name in [
            "code & notes.js",
            "report.pdf",
            "test.cmd",
            "setup.EXE",
            "shortcut.lnk",
        ] {
            std::fs::write(root.path().join(name), b"fixture").unwrap();
        }
        let code = root.path().join("code & notes.js");
        assert!(
            open_with(root.path(), &code, |p| p == code, |_| panic!())
                .unwrap()
                .is_none()
        );
        assert_eq!(
            open_with(root.path(), &code, |_| false, |_| panic!())
                .unwrap()
                .unwrap(),
            code
        );
        let script = root.path().join("test.cmd");
        assert_eq!(
            open_with(root.path(), &script, |_| false, |_| panic!())
                .unwrap()
                .unwrap(),
            script
        );
        let pdf = root.path().join("report.pdf");
        assert!(
            open_with(
                root.path(),
                &pdf,
                |_| false,
                |p| {
                    assert_eq!(p, pdf);
                    Ok(())
                }
            )
            .unwrap()
            .is_none()
        );
        for name in ["setup.EXE", "shortcut.lnk", "missing", "../outside.pdf"] {
            assert!(
                open_with(
                    root.path(),
                    &root.path().join(name),
                    |_| panic!(),
                    |_| panic!()
                )
                .is_err()
            );
        }
    }
}
