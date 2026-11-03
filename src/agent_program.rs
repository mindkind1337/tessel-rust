// Adapted from Tessel main/agentHeadless.js and main/agentTools.js / Orca.
// Copyright (c) 2026 Lovecast Inc. MIT; see THIRD_PARTY_NOTICES.md.
//! Resolve model-listing programs without handing agent arguments to cmd.exe.
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

#[derive(Debug, PartialEq, Eq)]
pub struct Program {
    pub file: PathBuf,
    pub pre: Vec<OsString>,
    pub path: OsString,
}
impl Program {
    pub fn command(&self, cwd: &Path) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(&self.file);
        command
            .args(&self.pre)
            .current_dir(cwd)
            .env("PATH", &self.path);
        command
    }
}

fn joined(dir: &Path, relative: &str) -> PathBuf {
    use std::path::Component;
    let raw = dir.join(
        relative
            .trim_start_matches(['/', '\\'])
            .replace('\\', std::path::MAIN_SEPARATOR_STR),
    );
    let mut result = PathBuf::new();
    for component in raw.components() {
        match component {
            Component::CurDir => (),
            Component::ParentDir if result.file_name().is_some_and(|s| s != "..") => {
                result.pop();
            }
            Component::ParentDir if result.has_root() => (),
            other => result.push(other.as_os_str()),
        }
    }
    result
}

fn quoted_target(text: &str, powershell: bool) -> Option<&str> {
    for (start, _) in text.match_indices('"') {
        let rest = &text[start + 1..];
        let Some(end) = rest.find('"') else {
            continue;
        };
        if !rest[end + 1..]
            .trim_start_matches(crate::agent_env::js_space)
            .starts_with("%*")
        {
            continue;
        }
        let target = &rest[..end];
        let lower = target.to_ascii_lowercase();
        let prefixes: &[&str] = if powershell {
            &["%~dp0", "%script_dir%"]
        } else {
            &["%~dp0%", "%dp0%", "%~dp0", "%dp0"]
        };
        let Some(prefix) = prefixes.iter().find(|prefix| lower.starts_with(**prefix)) else {
            continue;
        };
        let target = target[prefix.len()..]
            .strip_prefix('\\')
            .unwrap_or(&target[prefix.len()..]);
        if target.is_empty() || target.contains('%') {
            continue;
        }
        if powershell
            && (!target.to_ascii_lowercase().ends_with(".ps1")
                || !target
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
                || target.len() <= 4)
        {
            continue;
        }
        return Some(target);
    }
    None
}

/// Pure shim recognition; filesystem checks are supplied by the caller/tests.
/// The PowerShell form accepts only a filename beside the launcher.
pub fn shim_target(
    text: &str,
    dir: &Path,
    node: Option<&Path>,
    system_root: &Path,
    exists: impl Fn(&Path) -> bool,
) -> Option<(PathBuf, Vec<OsString>)> {
    for line in text.split(['\r', '\n']) {
        let lower = line.to_ascii_lowercase();
        for (at, _) in lower.match_indices("powershell") {
            let mut tail = &line[at + 10..];
            if tail
                .get(..4)
                .is_some_and(|s| s.eq_ignore_ascii_case(".exe"))
            {
                tail = &tail[4..];
            }
            if !tail.chars().next().is_some_and(crate::agent_env::js_space) {
                continue;
            }
            let tail_lower = tail.to_ascii_lowercase();
            for (file_at, _) in tail_lower.rmatch_indices("-file") {
                let after = &tail[file_at + 5..];
                if !after.chars().next().is_some_and(crate::agent_env::js_space) {
                    continue;
                }
                let after = after.trim_start_matches(crate::agent_env::js_space);
                if !after.starts_with('"') {
                    continue;
                }
                if let Some(name) = quoted_target(after, true) {
                    let script = joined(dir, name);
                    if !exists(&script) {
                        return None;
                    }
                    return Some((
                        joined(
                            system_root,
                            "System32/WindowsPowerShell/v1.0/powershell.exe",
                        ),
                        vec![
                            "-NoProfile".into(),
                            "-ExecutionPolicy".into(),
                            "Bypass".into(),
                            "-File".into(),
                            script.into_os_string(),
                        ],
                    ));
                }
            }
        }
    }
    let target = quoted_target(text, false)?;
    let target = joined(dir, target);
    if !exists(&target) {
        return None;
    }
    let ext = target.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "exe" => Some((target, Vec::new())),
        "js" | "cjs" | "mjs" => {
            let local = dir.join("node.exe");
            let node = if exists(&local) {
                local
            } else {
                node?.to_owned()
            };
            Some((node, vec![target.into_os_string()]))
        }
        _ => None,
    }
}

pub fn resolve(exe: &str) -> Result<Option<Program>, String> {
    if exe.is_empty() || exe.len() > 300 || exe.contains('\0') {
        return Ok(None);
    }
    #[cfg(not(windows))]
    {
        Ok(Some(Program {
            file: exe.into(),
            pre: Vec::new(),
            path: std::env::var_os("PATH").unwrap_or_default(),
        }))
    }
    #[cfg(windows)]
    {
        use crate::agent_model_process::{Limits, run};
        use std::{io::Read, time::Duration};
        let root = std::env::var_os("SystemRoot")
            .map_or_else(|| PathBuf::from("C:/Windows"), PathBuf::from);
        let mut command = tokio::process::Command::new(
            root.join("System32/WindowsPowerShell/v1.0/powershell.exe"),
        );
        // The requested name is data in an environment variable, never PowerShell source.
        command.args(["-NoProfile","-NonInteractive","-Command", "$env:Path = [Environment]::GetEnvironmentVariable('Path','Machine') + ';' + [Environment]::GetEnvironmentVariable('Path','User'); [Console]::OutputEncoding = [Text.Encoding]::UTF8; $c = Get-Command -CommandType Application -Name $env:TESSEL_PROBE_COMMAND -ErrorAction SilentlyContinue | Select-Object -First 1; $n = Get-Command -CommandType Application -Name 'node' -ErrorAction SilentlyContinue | Select-Object -First 1; ConvertTo-Json -Compress @{ app = [string]$c.Source; node = [string]$n.Source; path = $env:Path }"]).env("TESSEL_PROBE_COMMAND",exe);
        let output = run(
            command,
            None,
            Limits {
                timeout: Duration::from_secs(20),
                output: 64 * 1024,
            },
        )
        .map_err(|_| "Program lookup failed.".to_owned())?;
        let data: serde_json::Value =
            match serde_json::from_str(output.stdout.trim_start_matches('\u{feff}').trim()) {
                Ok(v) => v,
                Err(_) => return Ok(None),
            };
        let Some(app) = data["app"].as_str().filter(|s| !s.is_empty()) else {
            return Ok(None);
        };
        let file = PathBuf::from(app);
        let path: OsString = data["path"].as_str().unwrap_or_default().into();
        if file
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("exe"))
        {
            return Ok(Some(Program {
                file,
                pre: Vec::new(),
                path,
            }));
        }
        if !file
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("cmd"))
        {
            return Ok(None);
        }
        let Some(dir) = file.parent() else {
            return Ok(None);
        };
        let mut bytes = Vec::new();
        const MAX_SHIM: u64 = 256 * 1024;
        let Ok(shim) = std::fs::File::open(&file) else {
            return Ok(None);
        };
        if shim.take(MAX_SHIM + 1).read_to_end(&mut bytes).is_err() || bytes.len() as u64 > MAX_SHIM
        {
            return Ok(None);
        }
        let node = data["node"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(Path::new);
        Ok(shim_target(
            &String::from_utf8_lossy(&bytes),
            dir,
            node,
            &root,
            Path::is_file,
        )
        .map(|(file, pre)| Program { file, pre, path }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(windows)]
    fn resolved_npm_shim_passes_arguments_literally_without_running_batch_commands() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("folder with spaces");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(
            folder.join("echo.js"),
            "process.stdout.write(JSON.stringify(process.argv.slice(2)))",
        )
        .unwrap();
        let shim = folder.join("synthetic-probe.cmd");
        std::fs::write(
            &shim,
            "@echo off\r\necho must-not-run > batch-marker.txt\r\n\"%dp0%\\echo.js\" %*\r\n",
        )
        .unwrap();
        let program = resolve(shim.to_str().unwrap())
            .unwrap()
            .expect("synthetic npm shim and Node must resolve");
        let args = [
            "https://example.invalid/?a=1&b=2",
            "KEY=%PATH%",
            "say \"hi\" & bye",
            "with space",
            "^caret|pipe<>",
        ];
        let mut command = program.command(&folder);
        command.args(args);
        let output = crate::agent_model_process::run(command, None, Default::default()).unwrap();
        assert_eq!(output.code, Some(0));
        let echoed: Vec<String> = serde_json::from_str(&output.stdout).unwrap();
        assert_eq!(echoed, args);
        assert!(!folder.join("batch-marker.txt").exists());
    }
    #[test]
    #[cfg(windows)]
    fn shim_parser_matches_source_reference() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/agent_program.json")).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let existing: Vec<PathBuf> = case["existing"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| PathBuf::from(v.as_str().unwrap()))
                .collect();
            let result = shim_target(
                case["text"].as_str().unwrap(),
                Path::new(case["dir"].as_str().unwrap()),
                case["node"].as_str().map(Path::new),
                Path::new("C:/Windows"),
                |p| existing.contains(&p.to_owned()),
            );
            let result=result.map(|(file,pre)|serde_json::json!({"file":file,"pre":pre.iter().map(|v|v.to_string_lossy()).collect::<Vec<_>>()})).unwrap_or(serde_json::Value::Null);
            assert_eq!(result, case["expected"], "{}", case["text"]);
        }
    }
    #[test]
    fn npm_launchers_select_native_target_or_node_without_shell() {
        let dir = Path::new("fixture");
        let root = Path::new("system");
        let node = Path::new("node-global");
        for prefix in ["%dp0%", "%~dp0", "%DP0%", "%~dp0%"] {
            for ext in ["js", "cjs", "mjs", "exe"] {
                let text = format!("@echo off\r\n\"{prefix}\\node_modules\\tool.{ext}\"   %*");
                let target = dir.join("node_modules").join(format!("tool.{ext}"));
                let found = shim_target(&text, dir, Some(node), root, |p| p == target).unwrap();
                if ext == "exe" {
                    assert_eq!(found, (target, vec![]));
                } else {
                    assert_eq!(found, (node.to_owned(), vec![target.into_os_string()]));
                }
            }
        }
        let text = "\"%dp0%\\tool.js\" %*";
        assert_eq!(
            shim_target(text, dir, Some(node), root, |_| true)
                .unwrap()
                .0,
            dir.join("node.exe")
        );
        assert!(shim_target(text, dir, None, root, |p| p == dir.join("tool.js")).is_none());
        assert!(shim_target(text, dir, Some(node), root, |_| false).is_none());
    }
    #[test]
    fn cursor_script_must_be_a_sibling_and_unrecognized_shims_are_rejected() {
        let dir = Path::new("fixture");
        let root = Path::new("system");
        let script = dir.join("cursor-agent.ps1");
        let text = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File \"%SCRIPT_DIR%\\cursor-agent.ps1\" %*";
        let (file, pre) = shim_target(text, dir, None, root, |p| p == script).unwrap();
        assert!(file.ends_with("powershell.exe"));
        assert_eq!(pre.last(), Some(&script.into_os_string()));
        for invalid in [
            text.replace("cursor-agent.ps1", "..\\x.ps1"),
            "something %*".into(),
            "\"%dp0%\\unknown.txt\" %*".into(),
        ] {
            assert!(shim_target(&invalid, dir, None, root, |_| true).is_none());
        }
    }
}
