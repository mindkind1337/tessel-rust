# Tessel Rust

Native Rust rewrite of Tessel. **Active migration, not yet a feature-complete replacement.** The application UI and backend are Rust, using egui/eframe. No Electron, Vue or Node application runtime is bundled. External shells and agent CLIs remain separate programs; the Windows browser uses Microsoft's WebView2 engine.

## Features

### Projects and workspace

- Persistent local projects; newly added projects become selected automatically.
- Saved theme, font size, one/two/three-column terminal layout, SSH hosts and per-project task boards.
- Terminal and agent metadata restored asleep: reopening the app never automatically executes saved processes.
- Atomic workspace writes, schema validation and a single-instance workspace lock. Corrupt or newer metadata is retained instead of silently replaced.

### Terminals, agents and SSH

- Real PTYs (ConPTY on Windows), independent tabs and grid layout. Switching projects keeps existing sessions running and drains their output.
- PowerShell without user profiles on Windows; `/bin/sh` on Unix.
- ANSI colors, cursor, resize, scrollback, copy-visible-text menu, bounded paste, bracketed paste and application cursor keys.
- Bounded input/output queues and shell termination on sleep or close.
- Detection and explicit launch of installed Claude, Codex, OpenCode, Gemini, Copilot, Aider, Amp, Kimi, Qwen, Grok, Cline and Cursor agent CLIs.
- SSH terminals through OpenSSH, config aliases, host verification, explicit reconnect, remote working directory and keepalives. No production server is contacted automatically.

### Files and Git

- Background directory loading with virtualized rows; paths are confined to the selected project and symlinks are not traversed automatically.
- UTF-8 text editor tabs (2 MiB limit), atomic saves, detection of external edits and unsaved-change guards on close.
- On-demand Git status, working/staged diffs, recent history, literal-path staging and explicit commits. No continuous Git polling; operations have a 20-second deadline and bounded captured output.

### Native Codex chat

- Rust client for the installed Codex app-server, using its existing sign-in without copying tokens.
- Streaming text, tool events, model/effort fields, follow-up steering, interruption, thread-ID persistence and explicit resume.
- Workspace-write sandbox and approval-on-request explicitly requested and checked. Unknown requests fail closed; approval requests are tied to the active thread and turn.
- Project-scoped task-list/add/move tools connected to the native task board. Model arguments cannot select another project's authority.
- Pending approvals are displayed inside the chat instead of stealing focus with a popup.

### Windows browser

- Embedded WebView2, back/forward/reload, address bar, separate persistent Rust-app profile and no website-to-native IPC bridge.
- Original HTTP(S), authenticated and blob downloads remain inside the browser session. A fresh directory per download prevents duplicate names overwriting existing files.
- User-selected download folder, completion/failure status and copyable saved paths. Download URLs, cookies and tokens are not logged.
- New-window requests can be opened explicitly in the existing tab. Native/file URL schemes from websites are refused.

## Run and verify

Install Rust stable and native C/C++ build tools. Windows requires MSVC build tools, the Windows SDK and WebView2 Runtime. Agents, Git and OpenSSH must be installed separately for their corresponding features.

```sh
cargo run --locked -j 2
cargo fmt --check
cargo test --locked -j 2
cargo clippy --locked -j 2 --all-targets -- -D warnings
cargo build --locked -j 2
```

Windows executable: `target/debug/tessel-rust.exe`. CI provides a development artifact; no installer or release is published yet. `--data-dir <folder>` selects a separate workspace metadata directory for testing.

Optional native acceptance checks:

```powershell
.\target\debug\tessel-rust.exe --self-test-browser
.\target\debug\tessel-rust.exe --self-test-codex
```

The browser check creates a local HTTP fixture and temporary browser profile, downloads direct, cookie-authenticated and blob PDFs, and compares all saved bytes to the original synthetic PDF. It uses no personal account. The Codex check initializes the real installed app-server and verifies its applied permissions; it sends no prompt or model turn. `cargo build --features visual-test` enables eframe's screenshot test support.

## Validation and remaining migration

Current Windows evidence: 19 tests pass, strict Clippy and formatting pass, native UI screenshot inspected, all three real WebView2 download cases pass, and real Codex initialization passes. No live SSH server or paid inference turn has been used for acceptance testing.

See [MIGRATION.md](MIGRATION.md) for the acceptance ledger. Major remaining work includes Live Share, encrypted credentials/connectors/account sync, remote filesystem management, browser tools for agents, other native chat providers, full document previews, comprehensive terminal protocols, installer/updater, localization and accessibility. The native Codex chat does not yet render prior history or support every question/permission/tool type; unsupported requests are rejected explicitly. Popup-based OAuth flows and detailed download progress/cancellation need further implementation. Session metadata survives restart, but processes and terminal output do not survive app exit.

No existing Tessel configuration, credentials, browser profiles or personal records are automatically imported or committed.
