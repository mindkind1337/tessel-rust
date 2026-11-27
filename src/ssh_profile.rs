//! Connection profiles shared by SSH forms and transports.
//! Ported from Tessel's remoteHosts.js (Orca-derived, MIT; see THIRD_PARTY_NOTICES).
//! No file reads, command execution, credentials, or connection side effects.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    #[default]
    Manual,
    SshConfig,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Profile {
    pub label: String,
    pub config_host: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub identity_file: String,
    pub proxy_command: String,
    pub jump_host: String,
    pub source: Source,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            label: String::new(),
            config_host: String::new(),
            host: String::new(),
            port: 22,
            username: String::new(),
            identity_file: String::new(),
            proxy_command: String::new(),
            jump_host: String::new(),
            source: Source::Manual,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    HostRequired,
    HostInvalid,
    PortInvalid,
    UserInvalid,
    IdentityInvalid,
    JumpInvalid,
    ProxyInvalid,
}

impl Error {
    pub const fn code(self) -> &'static str {
        match self {
            Self::HostRequired => "host-required",
            Self::HostInvalid => "host-invalid",
            Self::PortInvalid => "port-invalid",
            Self::UserInvalid => "user-invalid",
            Self::IdentityInvalid => "identity-invalid",
            Self::JumpInvalid => "jump-invalid",
            Self::ProxyInvalid => "proxy-invalid",
        }
    }
}

fn matches(value: &str, pattern: &'static str, max: usize, cell: &'static OnceLock<Regex>) -> bool {
    value.chars().count() <= max
        && !value.starts_with('-')
        && cell
            .get_or_init(|| Regex::new(pattern).unwrap())
            .is_match(value)
}

fn valid_host(value: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    matches(value, r"\A[\p{L}\p{N}_.:%\[\]-]+\z", 253, &RE)
}

fn control(c: char) -> bool {
    c <= '\u{1f}' || c == '\u{7f}'
}

fn bounded(value: &str, max: usize) -> bool {
    // JavaScript .length counts UTF-16 code units for path/command limits.
    value.encode_utf16().count() <= max && !value.chars().any(control)
}

impl Profile {
    /// Normalize and validate all fields before storage or argument construction.
    pub fn checked(mut self) -> Result<Self, Error> {
        self.host = self.host.trim().into();
        if self.host.is_empty() {
            return Err(Error::HostRequired);
        }
        self.config_host = self.config_host.trim().into();
        if self.config_host.is_empty() {
            self.config_host.clone_from(&self.host);
        }
        if !valid_host(&self.host) || !valid_host(&self.config_host) {
            return Err(Error::HostInvalid);
        }
        if self.port == 0 {
            return Err(Error::PortInvalid);
        }
        static USER: OnceLock<Regex> = OnceLock::new();
        static JUMP: OnceLock<Regex> = OnceLock::new();
        self.username = self.username.trim().into();
        if !self.username.is_empty()
            && !matches(&self.username, r"\A[\p{L}\p{N}_.$@\\-]+\z", 64, &USER)
        {
            return Err(Error::UserInvalid);
        }
        self.identity_file = self.identity_file.trim().into();
        if !self.identity_file.is_empty()
            && (!bounded(&self.identity_file, 1024) || self.identity_file.starts_with('-'))
        {
            return Err(Error::IdentityInvalid);
        }
        self.jump_host = self.jump_host.trim().into();
        if !self.jump_host.is_empty()
            && !matches(&self.jump_host, r"\A[\p{L}\p{N}_.@:,%\[\]-]+\z", 512, &JUMP)
        {
            return Err(Error::JumpInvalid);
        }
        self.proxy_command = self.proxy_command.trim().into();
        if !bounded(&self.proxy_command, 2048) {
            return Err(Error::ProxyInvalid);
        }
        self.label = self
            .label
            .chars()
            .map(|c| if control(c) { ' ' } else { c })
            .collect::<String>()
            .trim()
            .chars()
            .scan(0, |len, c| {
                *len += c.len_utf16();
                (*len <= 120).then_some(c)
            })
            .collect();
        if self.label.is_empty() {
            self.label = if self.username.is_empty() {
                self.config_host.clone()
            } else {
                format!("{}@{}", self.username, self.host)
            };
        }
        Ok(self)
    }

    /// Connection options followed by `--` and one destination, never a shell command line.
    /// Callers prepend their PTY/batch/host-key policy and append the remote command separately.
    /// Imported aliases let OpenSSH resolve their current config at connection time.
    pub fn argv(&self) -> Result<Vec<String>, Error> {
        let p = self.clone().checked()?;
        let mut args = Vec::new();
        let via_alias = p.config_host != p.host;
        if p.source == Source::Manual {
            if via_alias {
                args.extend([
                    "-o".into(),
                    format!("HostName={}", p.host.replace('%', "%%")),
                ]);
            }
            if via_alias || p.port != 22 {
                args.extend(["-p".into(), p.port.to_string()]);
            }
            for (flag, value) in [
                ("-l", &p.username),
                ("-i", &p.identity_file),
                ("-J", &p.jump_host),
            ] {
                if !value.is_empty() {
                    args.extend([flag.into(), value.clone()]);
                }
            }
            if !p.proxy_command.is_empty() {
                args.extend(["-o".into(), format!("ProxyCommand={}", p.proxy_command)]);
            }
        }
        args.push("--".into());
        args.push(if p.source == Source::SshConfig || via_alias {
            p.config_host
        } else {
            p.host
        });
        Ok(args)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parsed {
    pub host: String,
    pub username: Option<String>,
    pub port: Option<u16>,
    pub invalid_port: bool,
}

pub fn clean_text(value: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[\p{Cf}\u{00a0}]").unwrap())
        .replace_all(value, "")
        .trim()
        .into()
}

fn port(value: &str) -> Option<u16> {
    if value.is_empty() || !value.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    value.parse().ok().filter(|n| *n != 0)
}

fn host_port(input: &str) -> Parsed {
    let mut parsed = Parsed {
        host: input.into(),
        username: None,
        port: None,
        invalid_port: false,
    };
    let pair = if let Some(rest) = input.strip_prefix('[') {
        if let Some((host, suffix)) = rest.split_once(']') {
            parsed.host = host.into();
            suffix.strip_prefix(':').map(|suffix| (host, suffix))
        } else {
            None
        }
    } else if input.matches(':').count() == 1 {
        input.split_once(':').filter(|(host, _)| !host.is_empty())
    } else {
        None
    };
    if let Some((host, suffix)) = pair {
        parsed.host = host.into();
        parsed.port = port(suffix);
        parsed.invalid_port = parsed.port.is_none();
    }
    parsed
}

fn decode_user(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| {
        let bytes = value.as_bytes();
        if bytes.iter().enumerate().any(|(i, b)| {
            *b == b'%'
                && (bytes.get(i + 1).is_none_or(|b| !b.is_ascii_hexdigit())
                    || bytes.get(i + 2).is_none_or(|b| !b.is_ascii_hexdigit()))
        }) {
            return value.to_owned();
        }
        percent_encoding::percent_decode_str(value)
            .decode_utf8()
            .map(|s| s.into_owned())
            .unwrap_or_else(|_| value.into())
    })
}

pub fn parse_input(value: &str) -> Option<Parsed> {
    let input = clean_text(value);
    if input.is_empty() {
        return None;
    }
    if input
        .get(..6)
        .is_some_and(|s| s.eq_ignore_ascii_case("ssh://"))
    {
        if let Ok(url) = url::Url::parse(&input) {
            let host = url.host_str()?.trim_matches(['[', ']']).to_owned();
            let p = url.port();
            return Some(Parsed {
                host,
                username: decode_user(url.username()),
                port: p.filter(|p| *p != 0),
                invalid_port: p == Some(0),
            });
        }
        // Preserve invalid explicit ports for form validation, never silently fall back to 22.
        static FALLBACK: OnceLock<Regex> = OnceLock::new();
        let caps = FALLBACK
            .get_or_init(|| {
                Regex::new(
                    r"(?i)\Assh://(?:([^@/?#]*)@)?(\[[^\]]+\]|[^:/?#]+):([^/?#]*)(?:[/?#]|$)",
                )
                .unwrap()
            })
            .captures(&input)?;
        if port(&caps[3]).is_some() {
            return None;
        }
        return Some(Parsed {
            host: caps[2].trim_matches(['[', ']']).into(),
            username: decode_user(caps.get(1).map_or("", |m| m.as_str())),
            port: None,
            invalid_port: true,
        });
    }
    let (user, host) = input
        .rsplit_once('@')
        .filter(|(user, _)| !user.is_empty())
        .map_or((None, input.as_str()), |(user, host)| {
            (Some(user.trim().to_owned()), host.trim())
        });
    let mut parsed = host_port(host);
    parsed.username = user;
    (!parsed.host.is_empty()).then_some(parsed)
}

/// Editable connection fields. Session lifetime/daemon settings belong to the existing Host.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    pub label: String,
    pub host: String,
    pub port: String,
    pub username: String,
    pub identity_file: String,
    pub proxy_command: String,
    pub jump_host: String,
    /// The alias and resolved host captured when editing a config-sourced profile.
    pub imported_alias: Option<(String, String)>,
}

impl Draft {
    pub fn from_profile(p: &Profile) -> Self {
        Self {
            label: p.label.clone(),
            host: p.host.clone(),
            port: p.port.to_string(),
            username: p.username.clone(),
            identity_file: p.identity_file.clone(),
            proxy_command: p.proxy_command.clone(),
            jump_host: p.jump_host.clone(),
            imported_alias: (p.config_host != p.host)
                .then(|| (p.config_host.clone(), p.host.clone())),
        }
    }

    pub fn profile(&self) -> Result<Profile, Error> {
        let parsed = parse_input(&self.host);
        if parsed.as_ref().is_some_and(|p| p.invalid_port) {
            return Err(Error::PortInvalid);
        }
        let host = parsed
            .as_ref()
            .map_or_else(|| clean_text(&self.host), |p| p.host.clone());
        if host.is_empty() {
            return Err(Error::HostRequired);
        }
        let typed_port = self.port.trim();
        let parsed_port = parsed.as_ref().and_then(|p| p.port);
        let port = if typed_port.is_empty() || typed_port == "22" {
            parsed_port.unwrap_or(22)
        } else {
            port(typed_port).ok_or(Error::PortInvalid)?
        };
        let username = clean_text(&self.username);
        let username = if username.is_empty() {
            parsed.and_then(|p| p.username).unwrap_or_default()
        } else {
            username
        };
        Profile {
            config_host: self
                .imported_alias
                .as_ref()
                .filter(|(_, original)| original == &host)
                .map_or_else(|| host.clone(), |(alias, _)| alias.clone()),
            host,
            port,
            username,
            label: self.label.clone(),
            identity_file: self.identity_file.clone(),
            proxy_command: self.proxy_command.clone(),
            jump_host: self.jump_host.clone(),
            source: Source::Manual,
        }
        .checked()
    }
}
