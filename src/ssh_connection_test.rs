use crate::agent_model_process::{self, Error, Limits};
use std::time::Duration;
use tokio::process::Command;

#[derive(Debug, PartialEq)]
pub(crate) enum Failure {
    Unavailable,
    Timeout,
    Detail(String),
    Failed,
}

pub(crate) fn probe(alias: &str) -> Result<(), Failure> {
    crate::workspace::validate_host(alias).map_err(|_| Failure::Failed)?;
    #[cfg(windows)]
    let exe = {
        let system = std::env::var_os("SystemRoot")
            .or_else(|| std::env::var_os("windir"))
            .map(|p| std::path::PathBuf::from(p).join("System32/OpenSSH/ssh.exe"));
        let path = std::env::var_os("PATH").unwrap_or_default();
        system
            .into_iter()
            .chain(
                std::env::split_paths(&path)
                    .filter(|p| p.is_absolute())
                    .map(|p| p.join("ssh.exe")),
            )
            .find(|p| p.is_file())
            .ok_or(Failure::Unavailable)?
    };
    #[cfg(not(windows))]
    let exe = "ssh";
    run(Command::new(exe), alias, Duration::from_secs(20))
}

pub(crate) fn run(mut command: Command, alias: &str, timeout: Duration) -> Result<(), Failure> {
    command.args([
        "-T",
        "-o",
        "BatchMode=yes",
        "-o",
        "StrictHostKeyChecking=yes",
        "-o",
        "ConnectTimeout=10",
        "--",
        alias,
        "exit",
    ]);
    let out = agent_model_process::run(
        command,
        None,
        Limits {
            timeout,
            output: 65536,
        },
    )
    .map_err(|err| match err {
        Error::Start => Failure::Unavailable,
        Error::Timeout => Failure::Timeout,
        _ => Failure::Failed,
    })?;
    if out.code == Some(0) {
        return Ok(());
    }
    let detail = out
        .stderr
        .lines()
        .rev()
        .map(str::trim)
        .find(|s| !s.is_empty())
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(400)
        .collect::<String>();
    if detail.is_empty() {
        Err(Failure::Failed)
    } else {
        Err(Failure::Detail(detail))
    }
}

pub(crate) fn notice(alias: &str, outcome: Result<(), Failure>, fr: bool) -> (String, bool) {
    let failed = outcome.is_err();
    let text = match (outcome, fr) {
        (Ok(()), true) => "Connexion réussie.".into(),
        (Ok(()), false) => "Connection succeeded.".into(),
        (Err(Failure::Detail(s)), _) => s,
        (Err(Failure::Timeout), true) => "Le test de connexion a expiré.".into(),
        (Err(Failure::Timeout), false) => "The connection test timed out.".into(),
        (Err(Failure::Unavailable), true) => "Le client OpenSSH est indisponible.".into(),
        (Err(Failure::Unavailable), false) => "The OpenSSH client is unavailable.".into(),
        (Err(Failure::Failed), true) => "Le test de connexion a échoué.".into(),
        (Err(Failure::Failed), false) => "The connection test failed.".into(),
    };
    (format!("{alias} — {text}"), failed)
}
