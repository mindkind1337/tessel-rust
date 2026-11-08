#![allow(dead_code)]
//! Persistent SSH terminal `tmux` session wrapping, version probing, and static `tmux` bootstrap.
//!
//! Mirrors Tessel's `remoteShellTmux.js` and `tmuxBinaries.js`:
//! - Uses isolated socket `-L tessel` (`TMUX_SOCKET`) and per-pane session `tessel-<id>`.
//! - System `tmux` on `PATH` always takes precedence over bundled `~/.tessel-server/bin/tmux`.
//! - Applies `status off`, `history-limit 50000`, `screen-256color`, `escape-time 10`,
//!   `focus-events on`, `mouse on`, `set-clipboard on`, alternate-screen wheel forwarding, and
//!   RGB + OSC 52 `terminal-overrides`, plus `prefix None` + `unbind C-b` only on `tmux >= 3.2`.
//! - Fresh pane creation (`reattach = false`) kills any stale `=<name>` session before creating
//!   a clean one; automatic reconnect (`reattach = true`) reattaches with `new-session -A -D`.
//! - Skips `tmux` wrapping when `jump_host` or `proxy_command` is configured or when the user
//!   opts out via `Host::persistent_sessions == Some(false)`.

fn posix_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// Dedicated `tmux` socket name (`tmux -L tessel`) so Tessel never alters the user's default socket.
pub const TMUX_SOCKET: &str = "tessel";

/// Quoted shell expression for Tessel's user-scoped static `tmux` binary on Linux hosts.
pub const OWN_TMUX: &str = "\"$HOME/.tessel-server/bin/tmux\"";

/// Relative path under `$HOME` for Tessel's static `tmux` binary.
pub const OWN_TMUX_REL_PATH: &str = ".tessel-server/bin/tmux";

/// Exit code emitted by [`build_static_tmux_install_script`] when SHA-256 verification fails.
pub const INSTALL_EXIT_HASH_MISMATCH: i32 = 83;

/// Exit code emitted by [`build_static_tmux_install_script`] when `$HOME` is mounted `noexec`
/// or the downloaded binary cannot execute `tmux -V`.
pub const INSTALL_EXIT_NOEXEC: i32 = 84;

/// Pinned static `tmux` Linux release metadata (`tmuxBinaries.js` parity).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StaticTmuxAsset {
    pub arch: &'static str,
    pub version: &'static str,
    pub url: &'static str,
    pub sha256_hex: &'static str,
}

pub const STATIC_TMUX_ASSETS: [StaticTmuxAsset; 2] = [
    StaticTmuxAsset {
        arch: "x86_64",
        version: "3.5a",
        url: "https://github.com/mjakob-gh/build-static-tmux/releases/download/v3.5a/tmux.linux-amd64.stripped",
        sha256_hex: "0926bfccbf4d1ba54edabed19fb56effa81d3db999749eb8ba17e34a53d9a870",
    },
    StaticTmuxAsset {
        arch: "aarch64",
        version: "3.5a",
        url: "https://github.com/mjakob-gh/build-static-tmux/releases/download/v3.5a/tmux.linux-arm64.stripped",
        sha256_hex: "de2e1b86d6abca7ca115f2262cc0591509c00eb0cfba0915a74eb1b226efbb33",
    },
];

/// Looks up the pinned static `tmux` asset for a Linux machine architecture (`x86_64` / `aarch64`).
#[must_use]
pub fn static_tmux_asset_for_arch(os: &str, arch: &str) -> Option<&'static StaticTmuxAsset> {
    if !os.trim().eq_ignore_ascii_case("linux") {
        return None;
    }
    let norm_arch = match arch.trim().to_ascii_lowercase().as_str() {
        "x86_64" | "amd64" => "x86_64",
        "aarch64" | "arm64" => "aarch64",
        _ => return None,
    };
    STATIC_TMUX_ASSETS.iter().find(|a| a.arch == norm_arch)
}

/// Parsed `major.minor` version from `tmux -V`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TmuxVersion {
    pub major: u32,
    pub minor: u32,
}

impl TmuxVersion {
    #[must_use]
    pub const fn new(major: u32, minor: u32) -> Self {
        Self { major, minor }
    }

    /// `set -g prefix None` requires `tmux >= 3.2`.
    #[must_use]
    pub const fn supports_prefix_none(self) -> bool {
        self.major > 3 || (self.major == 3 && self.minor >= 2)
    }
}

/// Parses `tmux -V` output such as `"tmux 3.4"`, `"tmux 3.2a"`, or `"tmux next-3.5"`.
#[must_use]
pub fn parse_tmux_version(raw: &str) -> Option<TmuxVersion> {
    let trimmed = raw.trim();
    let rest = trimmed.strip_prefix("tmux")?.trim();
    let rest = rest.strip_prefix("next-").unwrap_or(rest);
    let mut parts = rest.split('.');
    let major_str: String = parts
        .next()?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    if major_str.is_empty() {
        return None;
    }
    let minor_str: String = parts
        .next()
        .unwrap_or("0")
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let major = major_str.parse::<u32>().ok()?;
    let minor = if minor_str.is_empty() {
        0
    } else {
        minor_str.parse::<u32>().ok()?
    };
    Some(TmuxVersion { major, minor })
}

/// Origin of a discovered remote `tmux` binary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TmuxBinarySource {
    /// System `tmux` discovered via `command -v tmux`.
    System,
    /// Tessel's user-scoped static binary at `~/.tessel-server/bin/tmux`.
    Bundled,
}

/// Result of probing a remote host for `tmux`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TmuxProbeResult {
    pub binary_expr: String,
    pub source: TmuxBinarySource,
    pub version: Option<TmuxVersion>,
}

/// Builds a sanitized `tmux` session name (`tessel-<alnum_- up to 40 chars>`).
#[must_use]
pub fn tmux_session_name(pane_id: &str) -> String {
    let mut suffix = String::with_capacity(40);
    for ch in pane_id.trim().chars() {
        if suffix.len() >= 40 {
            break;
        }
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            suffix.push(ch);
        } else {
            suffix.push('-');
        }
    }
    let trimmed = suffix.trim_matches('-');
    if trimmed.is_empty() {
        "tessel-pane".to_owned()
    } else {
        format!("tessel-{trimmed}")
    }
}

/// Returns `true` when Tessel should wrap an SSH terminal in `tmux`.
///
/// Explicitly disabled when `persistent_sessions_enabled == false`, or when `jump_host` /
/// `proxy_command` is configured (matching `remoteShellTmux.js`).
#[must_use]
pub fn can_wrap_ssh_with_tmux(
    persistent_sessions_enabled: bool,
    jump_host: Option<&str>,
    proxy_command: Option<&str>,
) -> bool {
    if !persistent_sessions_enabled {
        return false;
    }
    if jump_host.is_some_and(|s| !s.trim().is_empty()) {
        return false;
    }
    if proxy_command.is_some_and(|s| !s.trim().is_empty()) {
        return false;
    }
    true
}

/// Single-shot remote probe command reporting system `tmux`, bundled `tmux`, and `uname -sm`.
#[must_use]
pub const fn tmux_probe_command() -> &'static str {
    "if command -v tmux >/dev/null 2>&1; then printf 'SYS:%s\\n' \"$(tmux -V 2>/dev/null)\"; fi; \
     if [ -x \"$HOME/.tessel-server/bin/tmux\" ] && [ ! -L \"$HOME/.tessel-server/bin/tmux\" ]; then \
     printf 'OWN:%s\\n' \"$(\"$HOME/.tessel-server/bin/tmux\" -V 2>/dev/null)\"; fi; \
     printf 'UNAME:%s %s\\n' \"$(uname -s 2>/dev/null)\" \"$(uname -m 2>/dev/null)\""
}

/// Parses output from [`tmux_probe_command`]. System `tmux` always wins when both exist.
#[must_use]
pub fn parse_tmux_probe_output(
    stdout: &str,
) -> (Option<TmuxProbeResult>, Option<(String, String)>) {
    let mut sys_ver: Option<Option<TmuxVersion>> = None;
    let mut own_ver: Option<Option<TmuxVersion>> = None;
    let mut os_arch: Option<(String, String)> = None;

    for raw_line in stdout.lines() {
        let line = raw_line.trim();
        if let Some(rest) = line.strip_prefix("SYS:") {
            sys_ver = Some(parse_tmux_version(rest));
        } else if let Some(rest) = line.strip_prefix("OWN:") {
            own_ver = Some(parse_tmux_version(rest));
        } else if let Some(rest) = line.strip_prefix("UNAME:") {
            let mut parts = rest.split_whitespace();
            if let (Some(os), Some(arch)) = (parts.next(), parts.next()) {
                os_arch = Some((os.to_owned(), arch.to_owned()));
            }
        }
    }

    let chosen = if let Some(version) = sys_ver {
        Some(TmuxProbeResult {
            binary_expr: "tmux".to_owned(),
            source: TmuxBinarySource::System,
            version,
        })
    } else {
        own_ver.map(|version| TmuxProbeResult {
            binary_expr: OWN_TMUX.to_owned(),
            source: TmuxBinarySource::Bundled,
            version,
        })
    };

    (chosen, os_arch)
}

/// Ordered `tmux` server configuration subcommands applied to `-L tessel`.
#[must_use]
pub fn tmux_server_options(version: Option<TmuxVersion>) -> Vec<String> {
    let mut opts = vec![
        "set -g status off".to_owned(),
        "set -g history-limit 50000".to_owned(),
        "set -g default-terminal \"screen-256color\"".to_owned(),
        "set -sg escape-time 10".to_owned(),
        "set -g focus-events on".to_owned(),
        "set -g mouse on".to_owned(),
        "set -g set-clipboard on".to_owned(),
        "set -as terminal-overrides \",xterm*:Tc:RGB,xterm*:Ms=\\\\E]52;c;%p2%s\\\\7\"".to_owned(),
        "bind -n WheelUpPane if-shell -F \"#{alternate_on}\" \"send-keys PPage\" \"copy-mode -e\\; send-keys -M\""
            .to_owned(),
        "bind -n WheelDownPane if-shell -F \"#{alternate_on}\" \"send-keys NPage\" \"send-keys -M\""
            .to_owned(),
    ];
    if version.is_some_and(TmuxVersion::supports_prefix_none) {
        opts.push("set -g prefix None".to_owned());
        opts.push("unbind C-b".to_owned());
    }
    opts
}

fn validate_binary_expr(binary_expr: &str) -> Result<(), String> {
    if binary_expr == "tmux" || binary_expr == OWN_TMUX {
        Ok(())
    } else {
        Err("Unsupported tmux binary expression.".into())
    }
}

fn validate_remote_folder_arg(folder: &str) -> Result<(), String> {
    if folder.len() > 8192
        || folder.chars().any(char::is_control)
        || folder.split(['/', '\\']).any(|seg| seg == "..")
    {
        return Err("Invalid remote directory.".into());
    }
    Ok(())
}

/// Builds a remote POSIX command launching or reattaching a `tmux` session on `-L tessel`.
pub fn build_tmux_wrap_command(
    binary_expr: &str,
    pane_id: &str,
    folder: &str,
    version: Option<TmuxVersion>,
    reattach: bool,
) -> Result<String, String> {
    validate_binary_expr(binary_expr)?;
    validate_remote_folder_arg(folder)?;

    let session = tmux_session_name(pane_id);
    let exact_target = format!("={session}");
    let mut parts = Vec::new();

    if !folder.is_empty() {
        parts.push(format!("cd -- {}", posix_quote(folder)));
    }
    parts.push(format!("{binary_expr} -L {TMUX_SOCKET} start-server"));
    for opt in tmux_server_options(version) {
        parts.push(format!(
            "{binary_expr} -L {TMUX_SOCKET} {opt} >/dev/null 2>&1 || true"
        ));
    }
    if !reattach {
        parts.push(format!(
            "{binary_expr} -L {TMUX_SOCKET} kill-session -t {} >/dev/null 2>&1 || true",
            posix_quote(&exact_target)
        ));
        parts.push(format!(
            "exec {binary_expr} -L {TMUX_SOCKET} new-session -s {}",
            posix_quote(&session)
        ));
    } else {
        parts.push(format!(
            "exec {binary_expr} -L {TMUX_SOCKET} new-session -A -D -s {}",
            posix_quote(&session)
        ));
    }

    Ok(parts.join(" && "))
}

/// Builds the remote cleanup command run when the user explicitly closes an SSH pane.
pub fn build_tmux_kill_session_command(binary_expr: &str, pane_id: &str) -> Result<String, String> {
    validate_binary_expr(binary_expr)?;
    let exact_target = format!("={}", tmux_session_name(pane_id));
    Ok(format!(
        "{binary_expr} -L {TMUX_SOCKET} kill-session -t {} >/dev/null 2>&1 || true",
        posix_quote(&exact_target)
    ))
}

/// Builds the remote launch command passed to `ssh -tt -- <alias> <script>` for `Launch::Ssh`.
///
/// - When `persistent_sessions_enabled` is `false`:
///   returns `Ok(None)` if `folder` is empty (plain default login shell), or `cd && exec $SHELL -l`.
/// - When `persistent_sessions_enabled` is `true`:
///   prefers system `tmux`, then non-symlink `$HOME/.tessel-server/bin/tmux`, applies Tessel's
///   `-L tessel` server options (gating `prefix None` on `tmux >= 3.2`), and falls back cleanly to
///   `exec "${SHELL:-/bin/sh}" -l` if `tmux` is unavailable on the remote host.
pub fn build_ssh_launch_remote_script(
    alias: &str,
    folder: &str,
    persistent_sessions_enabled: bool,
    pane_id: Option<&str>,
    reattach: bool,
) -> Result<Option<String>, String> {
    validate_remote_folder_arg(folder)?;
    if !persistent_sessions_enabled {
        if folder.is_empty() {
            return Ok(None);
        }
        return Ok(Some(format!(
            "cd -- {} && exec \"${{SHELL:-/bin/sh}}\" -l",
            posix_quote(folder)
        )));
    }

    let session = tmux_session_name(pane_id.unwrap_or(alias));
    let exact_target = format!("={session}");
    let cd_prefix = if folder.is_empty() {
        String::new()
    } else {
        format!("cd -- {} && ", posix_quote(folder))
    };
    let attach_step = if reattach {
        format!(
            "exec \"$_t_tmux\" -L {TMUX_SOCKET} new-session -A -D -s {}",
            posix_quote(&session)
        )
    } else {
        format!(
            "\"$_t_tmux\" -L {TMUX_SOCKET} kill-session -t {} >/dev/null 2>&1 || true; \
             exec \"$_t_tmux\" -L {TMUX_SOCKET} new-session -s {}",
            posix_quote(&exact_target),
            posix_quote(&session)
        )
    };

    let script = format!(
        "{cd_prefix}_t_tmux=''; \
         if command -v tmux >/dev/null 2>&1; then _t_tmux='tmux'; \
         elif [ -x \"$HOME/.tessel-server/bin/tmux\" ] && [ ! -L \"$HOME/.tessel-server/bin/tmux\" ]; then \
         _t_tmux=\"$HOME/.tessel-server/bin/tmux\"; fi; \
         if [ -n \"$_t_tmux\" ]; then \
         \"$_t_tmux\" -L {TMUX_SOCKET} start-server >/dev/null 2>&1 || true; \
         \"$_t_tmux\" -L {TMUX_SOCKET} set -g status off >/dev/null 2>&1 || true; \
         \"$_t_tmux\" -L {TMUX_SOCKET} set -g history-limit 50000 >/dev/null 2>&1 || true; \
         \"$_t_tmux\" -L {TMUX_SOCKET} set -g default-terminal 'screen-256color' >/dev/null 2>&1 || true; \
         \"$_t_tmux\" -L {TMUX_SOCKET} set -sg escape-time 10 >/dev/null 2>&1 || true; \
         \"$_t_tmux\" -L {TMUX_SOCKET} set -g focus-events on >/dev/null 2>&1 || true; \
         \"$_t_tmux\" -L {TMUX_SOCKET} set -g mouse on >/dev/null 2>&1 || true; \
         \"$_t_tmux\" -L {TMUX_SOCKET} set -g set-clipboard on >/dev/null 2>&1 || true; \
         _t_ver=$(\"$_t_tmux\" -V 2>/dev/null | sed -n 's/^tmux [^0-9]*\\([0-9][0-9]*\\)\\.\\([0-9][0-9]*\\).*/\\1.\\2/p'); \
         case \"$_t_ver\" in 3.[2-9]|3.[1-9][0-9]*|[4-9].*|[1-9][0-9]*.*) \
         \"$_t_tmux\" -L {TMUX_SOCKET} set -g prefix None >/dev/null 2>&1 || true; \
         \"$_t_tmux\" -L {TMUX_SOCKET} unbind C-b >/dev/null 2>&1 || true ;; esac; \
         {attach_step}; \
         fi; \
         exec \"${{SHELL:-/bin/sh}}\" -l"
    );
    Ok(Some(script))
}

/// Outcome of running [`build_static_tmux_install_script`] on a remote Linux host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaticTmuxInstallOutcome {
    Installed,
    HashMismatch,
    NoExec,
    Failed(i32),
}

#[must_use]
pub const fn classify_tmux_install_exit(exit_code: i32) -> StaticTmuxInstallOutcome {
    match exit_code {
        0 => StaticTmuxInstallOutcome::Installed,
        INSTALL_EXIT_HASH_MISMATCH => StaticTmuxInstallOutcome::HashMismatch,
        INSTALL_EXIT_NOEXEC => StaticTmuxInstallOutcome::NoExec,
        other => StaticTmuxInstallOutcome::Failed(other),
    }
}

/// Builds a hardened POSIX install script that downloads a static `tmux` binary to
/// `$HOME/.tessel-server/bin/tmux`, verifies its SHA-256 digest (`83` on mismatch), tests
/// execution via `-V` (`84` on `noexec`), and atomically renames it into place.
pub fn build_static_tmux_install_script(
    url: &str,
    expected_sha256_hex: &str,
    nonce: &str,
) -> Result<String, String> {
    if !url.starts_with("https://")
        || url.len() > 1024
        || url.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err("Invalid HTTPS download URL.".into());
    }
    if expected_sha256_hex.len() != 64
        || !expected_sha256_hex
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        return Err("Expected lowercase 64-char SHA-256 hex digest.".into());
    }
    if nonce.is_empty() || nonce.len() > 32 || !nonce.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("Invalid install nonce.".into());
    }

    let quoted_url = posix_quote(url);
    Ok(format!(
        "set -eu; umask 077; \
         base=\"$HOME/.tessel-server\"; bin=\"$base/bin\"; dst=\"$bin/tmux\"; part=\"$bin/.tmux.part.{nonce}\"; \
         [ ! -L \"$base\" ] && [ ! -L \"$bin\" ] && [ ! -L \"$dst\" ]; \
         mkdir -p \"$bin\"; \
         [ ! -L \"$base\" ] && [ ! -L \"$bin\" ] && [ ! -L \"$part\" ]; \
         rm -f \"$part\"; \
         if command -v curl >/dev/null 2>&1; then curl -fsSL --max-redirs 5 -o \"$part\" {quoted_url}; \
         elif command -v wget >/dev/null 2>&1; then wget -q -O \"$part\" {quoted_url}; \
         else exit 82; fi; \
         if command -v sha256sum >/dev/null 2>&1; then got=$(sha256sum \"$part\" | awk '{{print $1}}'); \
         elif command -v shasum >/dev/null 2>&1; then got=$(shasum -a 256 \"$part\" | awk '{{print $1}}'); \
         else rm -f \"$part\"; exit 82; fi; \
         if [ \"$got\" != '{expected_sha256_hex}' ]; then rm -f \"$part\"; exit {INSTALL_EXIT_HASH_MISMATCH}; fi; \
         chmod 0700 \"$part\"; \
         if ! \"$part\" -V >/dev/null 2>&1; then rm -f \"$part\"; exit {INSTALL_EXIT_NOEXEC}; fi; \
         mv -f \"$part\" \"$dst\""
    ))
}

/// Localized terminal banner notices for SSH `tmux` persistence states (`en.json` / `fr.json`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TmuxBannerKind {
    NoTmux,
    NoTmuxSoon,
    NoTmuxBundled,
    ReconnectedSame,
}

impl TmuxBannerKind {
    #[must_use]
    pub const fn message(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::NoTmux, false) => {
                "tmux is not installed on this host — if SSH drops, running commands in this pane will stop."
            }
            (Self::NoTmux, true) => {
                "tmux n'est pas installé sur cet hôte — en cas de coupure SSH, les commandes en cours s'arrêteront."
            }
            (Self::NoTmuxSoon, false) => {
                "Installing static tmux in ~/.tessel-server/bin/tmux — your next terminal on this host will stay alive across disconnects."
            }
            (Self::NoTmuxSoon, true) => {
                "Installation de tmux statique dans ~/.tessel-server/bin/tmux — le prochain terminal sur cet hôte survivra aux coupures."
            }
            (Self::NoTmuxBundled, false) => {
                "Static tmux installed in ~/.tessel-server/bin/tmux — new terminals on this host will persist."
            }
            (Self::NoTmuxBundled, true) => {
                "tmux statique installé dans ~/.tessel-server/bin/tmux — les nouveaux terminaux sur cet hôte seront persistants."
            }
            (Self::ReconnectedSame, false) => {
                "Reconnected to your running tmux session on this host."
            }
            (Self::ReconnectedSame, true) => {
                "Reconnecté à votre session tmux en cours sur cet hôte."
            }
        }
    }
}
