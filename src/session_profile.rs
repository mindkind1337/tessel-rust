//! Local account identities without serialized profile paths or credentials.
use crate::{agent_preferences::Preferences, settings_extra_model::ManagedAccountsRoster};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub use crate::session_profile_identity::Binding;

pub fn env_key(agent: &str) -> Result<&'static str, String> {
    match agent {
        "claude" | "openclaude" => Ok("CLAUDE_CONFIG_DIR"),
        "codex" => Ok("CODEX_HOME"),
        _ => Err("This provider does not support saved session profiles.".into()),
    }
}

fn default_root(agent: &str, home: &Path) -> Result<PathBuf, String> {
    env_key(agent)?;
    Ok(home.join(format!(".{agent}")))
}

fn configured_root(
    agent: &str,
    prefs: &Preferences,
    cwd: &Path,
    home: &Path,
    inherited: Option<&str>,
) -> Result<PathBuf, String> {
    let env =
        crate::agent_env::parse(&prefs.env).map_err(|_| "Invalid agent environment settings.")?;
    let root = match crate::agent_env::effective_value(&env, env_key(agent)?)
        .or(inherited)
        .filter(|v| !v.is_empty())
    {
        Some(value) if Path::new(value).is_absolute() => PathBuf::from(value),
        Some(value) => cwd.join(value),
        None => default_root(agent, home)?,
    };
    validate_root(&root)?;
    Ok(root)
}

fn validate_root(root: &Path) -> Result<(), String> {
    let value = root
        .to_str()
        .ok_or("Session profile path is not valid Unicode.")?;
    if !root.is_absolute() || value.len() > 8192 || value.chars().any(char::is_control) {
        return Err("Invalid session profile path.".into());
    }
    Ok(())
}

pub fn path_key(path: &Path) -> String {
    use std::path::Component;
    let mut base = String::new();
    let mut parts = Vec::new();
    for component in dunce::simplified(path).components() {
        match component {
            Component::Prefix(prefix) => {
                base = prefix.as_os_str().to_string_lossy().replace('\\', "/")
            }
            Component::RootDir => {
                if !base.ends_with('/') {
                    base.push('/');
                }
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if parts.last().is_some_and(|part: &String| part != "..") {
                    parts.pop();
                } else if base.is_empty() {
                    parts.push("..".into());
                }
            }
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
        }
    }
    let value = format!("{base}{}", parts.join("/"));
    if cfg!(windows) {
        value.to_lowercase()
    } else {
        value
    }
}

pub fn check_account_available(
    binding: &Binding,
    agent: &str,
    data: Option<&Path>,
) -> Result<(), String> {
    if let Binding::Managed { account_id } = binding {
        let roster = data
            .map(|data| crate::settings_extra_model::load_managed_accounts(data, agent))
            .unwrap_or_default();
        if !roster
            .accounts
            .iter()
            .any(|account| &account.id == account_id)
        {
            return Err("The saved session account is no longer available; no replacement session was started.".into());
        }
    }
    Ok(())
}

fn scope(agent: &str, root: &Path) -> String {
    crate::launch_fingerprint::of(agent, &path_key(root), &BTreeMap::new())
}

pub struct Context<'a> {
    pub agent: &'a str,
    pub prefs: &'a Preferences,
    pub cwd: &'a Path,
    pub home: &'a Path,
    pub data_dir: Option<&'a Path>,
    pub roster: &'a ManagedAccountsRoster,
    pub inherited: Option<&'a str>,
}

impl Context<'_> {
    pub fn capture(&self) -> Result<Binding, String> {
        let root = configured_root(self.agent, self.prefs, self.cwd, self.home, self.inherited)?;
        if path_key(&root) == path_key(&default_root(self.agent, self.home)?) {
            return Ok(Binding::Default);
        }
        if let Some(data) = self.data_dir {
            for account in &self.roster.accounts {
                if crate::settings_extra_model::managed_account_profile_dir(
                    data,
                    self.agent,
                    &account.id,
                )
                .as_ref()
                .is_some_and(|candidate| path_key(candidate) == path_key(&root))
                {
                    return Ok(Binding::Managed {
                        account_id: account.id.clone(),
                    });
                }
            }
        }
        Ok(Binding::Configured {
            scope_hash: scope(self.agent, &root),
        })
    }
    pub fn resolve(&self, binding: &Binding) -> Result<PathBuf, String> {
        if !binding.valid() {
            return Err("Invalid saved session profile identity.".into());
        }
        let root = match binding {
            Binding::Default => default_root(self.agent, self.home)?,
            Binding::Managed { account_id } => {
                if !self.roster.accounts.iter().any(|a| &a.id == account_id) {
                    return Err("The saved session account is no longer available; no replacement session was started.".into());
                }
                self.data_dir
                    .and_then(|data| {
                        crate::settings_extra_model::managed_account_profile_dir(
                            data, self.agent, account_id,
                        )
                    })
                    .ok_or("The saved session account cannot be resolved.")?
            }
            Binding::Configured { scope_hash } => {
                let root =
                    configured_root(self.agent, self.prefs, self.cwd, self.home, self.inherited)?;
                if &scope(self.agent, &root) != scope_hash {
                    return Err("The configured session profile has changed; restore its original setting before resuming.".into());
                }
                root
            }
        };
        validate_root(&root)?;
        Ok(root)
    }
}

pub fn with_root(agent: &str, prefs: &Preferences, root: &Path) -> Result<Preferences, String> {
    validate_root(root)?;
    let key = env_key(agent)?;
    let mut env =
        crate::agent_env::parse(&prefs.env).map_err(|_| "Invalid agent environment settings.")?;
    env.retain(|name, _| {
        if cfg!(windows) {
            !name.eq_ignore_ascii_case(key)
        } else {
            name != key
        }
    });
    env.insert(key.into(), root.to_str().expect("validated path").into());
    let mut result = prefs.clone();
    result.env = env
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("\n");
    crate::agent_env::parse(&result.env)
        .map_err(|_| "Session profile exceeds the environment limit.")?;
    Ok(result)
}

/// Runtime-only native transport scope. Never serialized or included in Debug output.
#[derive(Clone)]
pub struct NativeLaunch {
    agent: String,
    binding: Binding,
    root: PathBuf,
    data_dir: Option<PathBuf>,
}
impl NativeLaunch {
    pub fn new(
        agent: &str,
        binding: Binding,
        root: PathBuf,
        data_dir: Option<PathBuf>,
    ) -> Result<Self, String> {
        env_key(agent)?;
        if !binding.valid() {
            return Err("Invalid native account identity.".into());
        }
        validate_root(&root)?;
        Ok(Self {
            agent: agent.into(),
            binding,
            root,
            data_dir,
        })
    }
    pub fn validate_resume(&self, agent: &str, cwd: &Path, id: &str) -> Result<(), String> {
        if agent != self.agent || !cwd.is_dir() || !crate::claude::valid_uuid(id) {
            return Err("Invalid saved native conversation or working folder.".into());
        }
        let found = if agent == "codex" {
            crate::codex_sessions::verify(&self.root, cwd, id)?
        } else {
            crate::claude_terminal::exists(&self.root, id)?
        };
        if !found {
            return Err("The saved native conversation is missing from its original account; no replacement was started.".into());
        }
        Ok(())
    }
    pub fn apply(&self, agent: &str, command: &mut std::process::Command) -> Result<(), String> {
        if agent != self.agent {
            return Err("Native account belongs to another provider.".into());
        }
        check_account_available(&self.binding, agent, self.data_dir.as_deref())?;
        if matches!(self.binding, Binding::Managed { .. }) {
            let credentials: &[&str] = if agent == "codex" {
                &[
                    "OPENAI_API_KEY",
                    "CODEX_API_KEY",
                    "OPENAI_BASE_URL",
                    "OPENAI_ORG_ID",
                    "OPENAI_PROJECT_ID",
                ]
            } else {
                &[
                    "ANTHROPIC_API_KEY",
                    "ANTHROPIC_AUTH_TOKEN",
                    "CLAUDE_CODE_OAUTH_TOKEN",
                    "ANTHROPIC_BASE_URL",
                    "ANTHROPIC_CUSTOM_HEADERS",
                    "CLAUDE_CODE_USE_BEDROCK",
                    "CLAUDE_CODE_USE_VERTEX",
                    "CLAUDE_CODE_USE_FOUNDRY",
                ]
            };
            for key in credentials {
                command.env_remove(key);
            }
        }
        command.env(env_key(agent)?, &self.root);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_resume_requires_original_profile_transcript_and_provider() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("claude");
        let profile = NativeLaunch::new("claude", Binding::Default, root.clone(), None).unwrap();
        let id = "12345678-1234-4234-8234-123456789abc";
        assert!(profile.validate_resume("claude", temp.path(), id).is_err());
        fs_create_transcript(&root, id);
        assert!(profile.validate_resume("claude", temp.path(), id).is_ok());
        assert!(profile.validate_resume("codex", temp.path(), id).is_err());
        let removed = NativeLaunch::new(
            "claude",
            Binding::Managed {
                account_id: "removed".into(),
            },
            root,
            Some(temp.path().into()),
        )
        .unwrap();
        assert!(
            removed
                .apply("claude", &mut std::process::Command::new("synthetic"))
                .is_err()
        );
    }
    fn fs_create_transcript(root: &Path, id: &str) {
        let folder = root.join("projects/synthetic");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(format!("{id}.jsonl")), "{}\n").unwrap();
    }

    #[test]
    fn default_binding_ignores_current_profile_and_configured_binding_refuses_changes() {
        let temp = tempfile::tempdir().unwrap();
        let prefs = Preferences::default();
        let roster = ManagedAccountsRoster::default();
        let mut context = Context {
            agent: "claude",
            prefs: &prefs,
            cwd: temp.path(),
            home: temp.path(),
            data_dir: Some(temp.path()),
            roster: &roster,
            inherited: Some("elsewhere"),
        };
        assert_eq!(
            context.resolve(&Binding::Default).unwrap(),
            temp.path().join(".claude")
        );
        let captured = context.capture().unwrap();
        let serialized = serde_json::to_string(&captured).unwrap();
        assert!(!serialized.contains("elsewhere"));
        context.inherited = Some("changed");
        assert!(context.resolve(&captured).is_err());
        assert!(
            context
                .resolve(&Binding::Managed {
                    account_id: "removed".into()
                })
                .is_err()
        );
    }
    #[test]
    fn root_override_preserves_other_settings_and_rejects_injection() {
        let temp = tempfile::tempdir().unwrap();
        let prefs = Preferences {
            env: "CODEX_HOME=wrong\nKEEP=value".into(),
            ..Default::default()
        };
        let actual = with_root("codex", &prefs, temp.path()).unwrap();
        let parsed = crate::agent_env::parse(&actual.env).unwrap();
        assert_eq!(parsed["KEEP"], "value");
        assert_eq!(parsed["CODEX_HOME"], temp.path().to_str().unwrap());
        assert!(with_root("codex", &prefs, &temp.path().join("bad\nINJECT=value")).is_err());
        assert!(
            !Binding::Managed {
                account_id: "../outside".into()
            }
            .valid()
        );
    }
    #[test]
    fn managed_binding_stays_with_original_account_after_selection_changes() {
        let temp = tempfile::tempdir().unwrap();
        let account = |id: &str| crate::settings_extra_model::ManagedAccount {
            id: id.into(),
            label: id.into(),
            email: String::new(),
            plan: String::new(),
            added_at: 0,
        };
        let roster = ManagedAccountsRoster {
            accounts: vec![account("first"), account("second")],
            selected_id: Some("second".into()),
        };
        let original = crate::settings_extra_model::managed_account_profile_dir(
            temp.path(),
            "claude",
            "first",
        )
        .unwrap();
        let prefs = with_root("claude", &Preferences::default(), &original).unwrap();
        let context = Context {
            agent: "claude",
            prefs: &prefs,
            cwd: temp.path(),
            home: temp.path(),
            data_dir: Some(temp.path()),
            roster: &roster,
            inherited: None,
        };
        let binding = context.capture().unwrap();
        assert_eq!(
            binding,
            Binding::Managed {
                account_id: "first".into()
            }
        );
        assert_eq!(context.resolve(&binding).unwrap(), original);
        assert!(
            !serde_json::to_string(&binding)
                .unwrap()
                .contains("claude-accounts")
        );
    }
    #[test]
    fn lexical_aliases_keep_profile_identity_and_project_boundaries() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join(".claude");
        let alias = root.join(".");
        assert_eq!(path_key(&root), path_key(&alias));
        let prefs = with_root("claude", &Preferences::default(), &alias).unwrap();
        let roster = ManagedAccountsRoster::default();
        let context = Context {
            agent: "claude",
            prefs: &prefs,
            cwd: temp.path(),
            home: temp.path(),
            data_dir: None,
            roster: &roster,
            inherited: None,
        };
        assert_eq!(context.capture().unwrap(), Binding::Default);
        if cfg!(windows) {
            let upper = PathBuf::from(root.to_str().unwrap().to_uppercase());
            assert_eq!(scope("claude", &root), scope("claude", &upper));
        }
    }
}
