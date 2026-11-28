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

pub(crate) fn probe(host: &crate::workspace::Host) -> Result<(), Failure> {
    host.connection_args().map_err(|_| Failure::Failed)?;
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
    run(Command::new(exe), host, Duration::from_secs(20))
}

pub(crate) fn run(
    mut command: Command,
    host: &crate::workspace::Host,
    timeout: Duration,
) -> Result<(), Failure> {
    command.args(arguments(host)?);
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

fn arguments(host: &crate::workspace::Host) -> Result<Vec<String>, Failure> {
    let mut args: Vec<String> = [
        "-T",
        "-o",
        "BatchMode=yes",
        "-o",
        "StrictHostKeyChecking=yes",
        "-o",
        "ConnectTimeout=10",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    args.extend(host.connection_args().map_err(|_| Failure::Failed)?);
    args.push("exit".into());
    Ok(args)
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn structured_ssh_profile_test_keeps_batch_and_host_key_policy() {
        let host = crate::workspace::Host {
            alias: "server".into(),
            profile: Some(Box::new(crate::ssh_profile::Profile {
                host: "server".into(),
                port: 2222,
                username: "deploy".into(),
                ..Default::default()
            })),
            ..Default::default()
        };
        let args = arguments(&host).unwrap();
        assert_eq!(
            &args[args.len() - 7..],
            ["-p", "2222", "-l", "deploy", "--", "server", "exit"]
        );
        assert!(args.iter().any(|a| a == "BatchMode=yes"));
        assert!(args.iter().any(|a| a == "StrictHostKeyChecking=yes"));
    }
}
