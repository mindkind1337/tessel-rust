//! First command matches, following Tessel's commandLookup.js.
#[cfg(any(windows, test))]
use std::collections::HashMap;

#[cfg(any(windows, test))]
pub fn parse_where(output: &str, names: &[&str]) -> HashMap<String, String> {
    let lines: Vec<_> = output
        .split('\n')
        .map(crate::agent_names::js_trim)
        .filter(|line| !line.is_empty())
        .collect();
    let mut found = HashMap::new();
    for name in names {
        let name = name.to_lowercase();
        if found.contains_key(&name) {
            continue;
        }
        if let Some(path) = lines.iter().find(|line| {
            let base = line
                .trim_end_matches(['/', '\\'])
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or_default()
                .to_lowercase();
            base == name
                || base
                    .strip_prefix(&format!("{name}."))
                    .is_some_and(|extension| !extension.contains('.'))
        }) {
            found.insert(name, (*path).to_owned());
        }
    }
    found
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.@+-".contains(&b))
}

#[cfg(windows)]
pub fn lookup(names: &[&str], path: &std::ffi::OsStr) -> Result<HashMap<String, String>, String> {
    use std::{
        io::Read,
        os::windows::process::CommandExt,
        process::{Command, Stdio},
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };
    if names.is_empty() {
        return Ok(HashMap::new());
    }
    if names.len() > 64 || names.iter().any(|name| !valid_name(name)) {
        return Err("Invalid agent lookup batch.".into());
    }
    let root = std::env::var_os("SystemRoot").ok_or("Windows system directory unavailable.")?;
    let mut command = Command::new(std::path::PathBuf::from(root).join("System32/where.exe"));
    crate::agent_env::clean_command(&mut command);
    let mut child = command
        .args(names)
        .env("PATH", path)
        .creation_flags(0x08000000)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Could not start agent command lookup.")?;
    let stdout = child.stdout.take().expect("requested stdout pipe");
    let (sender, receiver) = mpsc::channel();
    const LIMIT: usize = 1024 * 1024;
    let reader = thread::Builder::new()
        .name("agent-lookup-output".into())
        .spawn(move || {
            let mut bytes = Vec::new();
            let result = stdout
                .take((LIMIT + 1) as u64)
                .read_to_end(&mut bytes)
                .map(|_| bytes);
            let _ = sender.send(result);
        });
    if reader.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return Err("Could not start agent command lookup output reader.".into());
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Agent command lookup timed out or stopped unexpectedly.".into());
            }
        }
    };
    let bytes = receiver
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|_| "Agent command lookup output timed out.")?
        .map_err(|_| "Could not read agent command lookup output.")?;
    if bytes.len() > LIMIT {
        return Err("Agent command lookup output exceeded its limit.".into());
    }
    // where.exe returns 1 if some or all requested commands were not found.
    if !matches!(status.code(), Some(0 | 1)) {
        return Err("Agent command lookup failed.".into());
    }
    Ok(parse_where(&String::from_utf8_lossy(&bytes), names))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn real_windows_lookup_batches_names_and_preserves_first_path_without_execution() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("first tools");
        let second = root.path().join("second tools");
        for dir in [&first, &second] {
            std::fs::create_dir(dir).unwrap();
        }
        let name = "tessel_synthetic_lookup_fixture";
        std::fs::write(
            first.join(format!("{name}.cmd")),
            "not executable test data",
        )
        .unwrap();
        std::fs::write(
            second.join(format!("{name}.exe")),
            "not executable test data",
        )
        .unwrap();
        let path = std::env::join_paths([&first, &second]).unwrap();
        let results = lookup(&[name, "tessel_synthetic_missing_fixture"], &path).unwrap();
        // where.exe expands DOS short names, including aliases in runner TEMP.
        // Compare file identity after normalization, not the spelling of its path.
        assert_eq!(
            std::fs::canonicalize(&results[name]).unwrap(),
            std::fs::canonicalize(first.join(format!("{name}.cmd"))).unwrap()
        );
        assert!(!results.contains_key("tessel_synthetic_missing_fixture"));
        assert!(lookup(&["invalid;command"], &path).is_err());
        assert!(lookup(&[], &path).unwrap().is_empty());
    }
    #[test]
    fn source_parse_where_fixtures_preserve_first_match_and_exact_command_names() {
        let reference: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/command_lookup.json")).unwrap();
        for case in reference["cases"].as_array().unwrap() {
            let names: Vec<_> = case["names"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap())
                .collect();
            let actual = parse_where(case["output"].as_str().unwrap(), &names);
            let expected: HashMap<String, String> =
                serde_json::from_value(case["expected"].clone()).unwrap();
            assert_eq!(actual, expected);
        }
    }
}
