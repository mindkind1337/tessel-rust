//! Immutable installation metadata produced by the source checkInstall rules.
//! Reading this catalogue never downloads or executes anything.
use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Install {
    pub id: String,
    pub install: Option<Vec<String>>,
    pub install_shell: Option<String>,
    pub install_source: Option<String>,
    pub install_confirm: bool,
    pub docs_url: Option<String>,
}

#[derive(Deserialize)]
struct Catalogue {
    agents: Vec<Install>,
}

pub fn all() -> &'static [Install] {
    static DATA: OnceLock<Catalogue> = OnceLock::new();
    &DATA
        .get_or_init(|| {
            serde_json::from_str(include_str!("agent_installs.json"))
                .expect("embedded checked installation catalogue")
        })
        .agents
}

pub fn find(id: &str) -> Option<&'static Install> {
    let canonical = crate::agent_catalog::find(id)?.id;
    all().iter().find(|entry| entry.id == canonical)
}

impl Install {
    /// Copy instructions verbatim. Multi-step installers retain their line boundaries.
    pub fn copy_text(&self) -> Option<String> {
        self.install.as_ref().map(|steps| steps.join("\n"))
    }
}

pub const MARK_OK: &str = "TESSEL-INSTALL-OK";
pub const MARK_FAILED: &str = "TESSEL-INSTALL-FAILED";
pub const NVM_INSTALL_URL: &str = "https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.3/install.sh";

const WINGET_NODE: &str = "winget install -e --id OpenJS.NodeJS.LTS --accept-source-agreements --accept-package-agreements";
const NODE_MISSING: &str = "Node.js is not installed: installing it with winget...";
const NODE_FAILED: &str =
    "Node.js (npm) could not be installed. Install it from https://nodejs.org and try again.";

#[must_use]
pub fn chain_commands(steps: &[String], shell_id: &str) -> String {
    let list: Vec<&str> = steps
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if list.len() <= 1 {
        return list.first().copied().unwrap_or_default().to_owned();
    }
    if shell_id == "powershell" {
        let mut acc = String::new();
        for step in list.into_iter().rev() {
            acc = if acc.is_empty() {
                step.to_owned()
            } else {
                format!("{step}; if ($?) {{ {acc} }}")
            };
        }
        return acc;
    }
    list.join(" && ")
}

#[must_use]
pub fn node_step(shell_id: &str) -> Option<String> {
    match shell_id {
        "gitbash" => {
            let path =
                r#"export PATH="/c/Program Files/nodejs:$(cygpath -u "$APPDATA")/npm:$PATH""#;
            let winget_args = WINGET_NODE.strip_prefix("winget ").unwrap_or(WINGET_NODE);
            Some(format!(
                "{{ command -v npm >/dev/null 2>&1 || {{ echo '{NODE_MISSING}'; winget.exe {winget_args}; {path}; }}; command -v npm >/dev/null 2>&1 || {{ echo '{NODE_FAILED}' >&2; false; }}; }}"
            ))
        }
        "wsl" => {
            let linux_npm = r#"case "$(command -v npm 2>/dev/null)" in ""|/mnt/*)"#;
            let fetch = format!(
                "{{ if command -v curl >/dev/null 2>&1; then curl -fsSL {NVM_INSTALL_URL}; else wget -qO- {NVM_INSTALL_URL}; fi; }} | bash"
            );
            let install = format!(
                r#"echo 'Node.js is not installed in WSL: installing it with nvm...'; export NVM_DIR="$HOME/.nvm"; [ -s "$NVM_DIR/nvm.sh" ] || {fetch}; . "$NVM_DIR/nvm.sh" && nvm install --lts"#
            );
            Some(format!(
                "{{ {linux_npm} {install};; esac; {linux_npm} echo '{NODE_FAILED}' >&2; false;; esac; }}"
            ))
        }
        "powershell" | "pwsh" => {
            let refresh = "$env:Path = [Environment]::GetEnvironmentVariable('Path','Machine') + ';' + [Environment]::GetEnvironmentVariable('Path','User')";
            Some(format!(
                "if (-not (Get-Command npm -ErrorAction SilentlyContinue)) {{ Write-Host '{NODE_MISSING}'; {WINGET_NODE}; {refresh} }}; if (-not (Get-Command npm -ErrorAction SilentlyContinue)) {{ Write-Error '{NODE_FAILED}' }}"
            ))
        }
        "cmd" => Some(format!(
            r#"(where npm >nul 2>nul || (echo {NODE_MISSING} && {WINGET_NODE} && set "PATH=%ProgramFiles%\nodejs;%APPDATA%\npm;%PATH%"))"#
        )),
        _ => None,
    }
}

#[must_use]
pub fn uses_npm(steps: &[String]) -> bool {
    steps.iter().any(|s| {
        let t = s.trim_start();
        t.len() >= 4
            && t.as_bytes()[..3].eq_ignore_ascii_case(b"npm")
            && t.as_bytes()[3].is_ascii_whitespace()
    })
}

/// Builds the exact shell line mirrored from `shellChain.js::installChain`.
/// The completion markers (`TESSEL-INSTALL-OK` / `TESSEL-INSTALL-FAILED`) are split in the
/// command text itself so echoing the input line never triggers completion before execution finishes.
#[must_use]
pub fn install_chain(steps: &[String], start: Option<&str>, shell_id: &str) -> String {
    let given: Vec<String> = steps
        .iter()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    let mut list = Vec::with_capacity(given.len() + 1);
    if uses_npm(&given)
        && let Some(ns) = node_step(shell_id)
    {
        list.push(ns);
    }
    list.extend(given);
    let then = start.unwrap_or_default().trim();
    if matches!(shell_id, "powershell" | "pwsh") {
        let ok = "Write-Host ('TESSEL-INSTALL' + '-OK')";
        let bad = "Write-Host ('TESSEL-INSTALL' + '-FAILED')";
        let mut acc = if then.is_empty() {
            ok.to_owned()
        } else {
            format!("{ok}; {then}")
        };
        for step in list.into_iter().rev() {
            acc = format!("{step}; if ($?) {{ {acc} }} else {{ {bad} }}");
        }
        return acc;
    }
    let (ok, bad) = if shell_id == "cmd" {
        ("echo TESSEL-INSTALL^-OK", "echo TESSEL-INSTALL^-FAILED")
    } else {
        (
            "echo TESSEL-INSTALL\"-OK\"",
            "echo TESSEL-INSTALL\"-FAILED\"",
        )
    };
    let mut parts = list;
    parts.push(ok.to_owned());
    if !then.is_empty() {
        parts.push(then.to_owned());
    }
    format!("{} || {bad}", parts.join(" && "))
}

#[must_use]
pub fn shell_display_name(shell_id: &str) -> &'static str {
    match shell_id {
        "powershell" => "PowerShell",
        "pwsh" => "PowerShell 7",
        "cmd" => "Command Prompt",
        "wsl" => "WSL",
        "gitbash" => "Git Bash",
        _ => "Shell",
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallExecution {
    pub agent_id: String,
    pub agent_name: String,
    pub shell_id: String,
    pub command_line: String,
    pub notice: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallConfirmSpec {
    pub title: String,
    pub description: String,
    pub code: String,
    pub shell_label: String,
    pub source_url: Option<String>,
    pub confirm_label: String,
    pub cancel_label: String,
    pub execution: InstallExecution,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlannedAgentInstall {
    Immediate(InstallExecution),
    NeedsConfirmation(InstallConfirmSpec),
}

pub fn plan_agent_install(
    id: &str,
    preferred_shell: Option<&str>,
    windows_platform: bool,
    wsl_available: bool,
    french: bool,
) -> Result<PlannedAgentInstall, String> {
    let preset = crate::agent_catalog::find(id).ok_or_else(|| {
        if french {
            "Agent inconnu.".to_owned()
        } else {
            "Unknown agent.".to_owned()
        }
    })?;
    let entry = find(preset.id).ok_or_else(|| {
        if french {
            "Métadonnées d'installation introuvables.".to_owned()
        } else {
            "Installation metadata unavailable.".to_owned()
        }
    })?;
    let steps = entry.install.as_ref().ok_or_else(|| {
        if french {
            format!(
                "{} s'installe depuis sa page de documentation.",
                preset.name
            )
        } else {
            format!("{} installs from its documentation page.", preset.name)
        }
    })?;

    let default_shell = if windows_platform { "powershell" } else { "sh" };
    let shell_id = entry
        .install_shell
        .as_deref()
        .or(preferred_shell)
        .unwrap_or(default_shell);
    let shell_name = shell_display_name(shell_id);

    let shell_supported = match shell_id {
        "powershell" | "cmd" => windows_platform,
        "wsl" => windows_platform && wsl_available,
        "sh" => !windows_platform,
        _ => true,
    };
    if !shell_supported {
        return Err(if french {
            format!(
                "{} s'installe avec {shell_name}, qui n'est pas présent sur cet ordinateur.",
                preset.name
            )
        } else {
            format!(
                "{} installs with {shell_name}, which is not on this computer.",
                preset.name
            )
        });
    }

    if entry.install_confirm {
        let command_line = install_chain(steps, None, shell_id);
        let notice = if french {
            format!(
                "Installation de {} ci-dessous. Tessel vous avertit quand c'est terminé.",
                preset.name
            )
        } else {
            format!(
                "Installing {} below. Tessel tells you when it has finished.",
                preset.name
            )
        };
        let execution = InstallExecution {
            agent_id: preset.id.to_owned(),
            agent_name: preset.name.to_owned(),
            shell_id: shell_id.to_owned(),
            command_line,
            notice,
        };
        return Ok(PlannedAgentInstall::NeedsConfirmation(InstallConfirmSpec {
            title: if french {
                format!("Installer {} ?", preset.name)
            } else {
                format!("Install {}?", preset.name)
            },
            description: if french {
                format!(
                    "Ceci télécharge l'installateur de {} depuis Internet et l'exécute sur cet ordinateur, dans un nouveau panneau {shell_name}. Il vient de son éditeur : ne continuez que si vous lui faites confiance.",
                    preset.name
                )
            } else {
                format!(
                    "This downloads {}'s installer from the internet and runs it on this computer, in a new {shell_name} pane. It comes from its vendor: only continue if you trust it.",
                    preset.name
                )
            },
            code: steps.join("\n"),
            shell_label: shell_name.to_owned(),
            source_url: entry.install_source.clone(),
            confirm_label: if french {
                "Installer".into()
            } else {
                "Install".into()
            },
            cancel_label: if french {
                "Annuler".into()
            } else {
                "Cancel".into()
            },
            execution,
        }));
    }

    let command_line = install_chain(steps, Some(preset.command), shell_id);
    let notice = if french {
        format!(
            "Installation de {}. Il démarre dans le nouveau panneau à la fin de l'installation.",
            preset.name
        )
    } else {
        format!(
            "Installing {}. It starts in the new pane when the install finishes.",
            preset.name
        )
    };
    let execution = InstallExecution {
        agent_id: preset.id.to_owned(),
        agent_name: preset.name.to_owned(),
        shell_id: shell_id.to_owned(),
        command_line,
        notice,
    };

    if shell_id == "wsl" && uses_npm(steps) {
        return Ok(PlannedAgentInstall::NeedsConfirmation(InstallConfirmSpec {
            title: if french {
                format!("Installer {} dans WSL ?", preset.name)
            } else {
                format!("Install {} in WSL?", preset.name)
            },
            description: if french {
                format!(
                    "Si Node.js n'est pas installé dans WSL, Tessel installera d'abord Node.js LTS avec nvm avant d'installer {}.",
                    preset.name
                )
            } else {
                format!(
                    "If Node.js is not installed in WSL, Tessel will install Node.js LTS with nvm first before installing {}.",
                    preset.name
                )
            },
            code: node_step("wsl").unwrap_or_else(|| steps.join("\n")),
            shell_label: shell_name.to_owned(),
            source_url: Some(NVM_INSTALL_URL.to_owned()),
            confirm_label: if french {
                "Installer".into()
            } else {
                "Install".into()
            },
            cancel_label: if french {
                "Annuler".into()
            } else {
                "Cancel".into()
            },
            execution,
        }));
    }

    Ok(PlannedAgentInstall::Immediate(execution))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_source_preset_has_one_install_record_and_aliases_share_it() {
        assert_eq!(all().len(), crate::agent_catalog::ALL.len());
        let mut ids = std::collections::HashSet::new();
        for entry in all() {
            assert!(ids.insert(&entry.id));
            assert!(crate::agent_catalog::find(&entry.id).is_some());
            for url in [&entry.install_source, &entry.docs_url]
                .into_iter()
                .flatten()
            {
                assert_eq!(url::Url::parse(url).unwrap().scheme(), "https");
            }
            if entry.install_confirm {
                assert!(entry.install.is_some());
                assert!(entry.install_source.is_some());
            }
        }
        assert!(std::ptr::eq(
            find("cursor").unwrap(),
            find("cursor-agent").unwrap()
        ));
        assert!(find("unknown").is_none());
    }

    #[test]
    fn source_confirmation_and_docs_only_choices_are_preserved() {
        let codex = find("codex").unwrap();
        assert_eq!(
            codex.copy_text().as_deref(),
            Some("npm install -g @openai/codex")
        );
        assert!(!codex.install_confirm);
        let kimi = find("kimi").unwrap();
        assert!(kimi.install_confirm);
        assert_eq!(kimi.install_shell.as_deref(), Some("powershell"));
        let aider = find("aider").unwrap();
        assert!(aider.install_confirm);
        assert_eq!(
            aider.copy_text().as_deref(),
            Some("python -m pip install aider-install\naider-install")
        );
        for id in ["goose", "auggie", "kiro", "rovo", "opencode2", "dsh"] {
            let install = find(id).unwrap();
            assert!(install.install.is_none());
            assert!(install.copy_text().is_none());
            assert!(install.docs_url.is_some());
        }
    }

    #[test]
    fn install_chain_splits_markers_and_plans_confirmed_vs_immediate_installs() {
        let steps = vec!["npm install -g @openai/codex".to_string()];
        let ps = install_chain(&steps, Some("codex"), "powershell");
        assert!(
            !ps.contains(MARK_OK),
            "typed line must not contain literal marker"
        );
        assert!(!ps.contains(MARK_FAILED));
        assert!(ps.contains("Write-Host ('TESSEL-INSTALL' + '-OK')"));

        let cmd = install_chain(&steps, Some("codex"), "cmd");
        assert!(!cmd.contains(MARK_OK));
        assert!(cmd.contains("TESSEL-INSTALL^-OK"));

        match plan_agent_install("kimi", Some("cmd"), true, true, true).unwrap() {
            PlannedAgentInstall::NeedsConfirmation(spec) => {
                assert_eq!(spec.execution.shell_id, "powershell");
                assert!(spec.title.contains("Installer"));
                assert!(spec.source_url.is_some());
            }
            other => panic!("expected vendor confirmation, got {other:?}"),
        }

        match plan_agent_install("codex", Some("powershell"), true, true, false).unwrap() {
            PlannedAgentInstall::Immediate(exec) => {
                assert_eq!(exec.shell_id, "powershell");
                assert!(exec.command_line.contains("@openai/codex"));
            }
            other => panic!("expected immediate npm install, got {other:?}"),
        }

        match plan_agent_install("codex", Some("wsl"), true, true, false).unwrap() {
            PlannedAgentInstall::NeedsConfirmation(spec) => {
                assert_eq!(spec.execution.shell_id, "wsl");
                assert_eq!(spec.source_url.as_deref(), Some(NVM_INSTALL_URL));
            }
            other => panic!("expected WSL nvm confirmation, got {other:?}"),
        }

        assert!(plan_agent_install("codex", Some("wsl"), true, false, false).is_err());
        assert!(plan_agent_install("goose", Some("powershell"), true, true, false).is_err());
    }
}
