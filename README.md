# Tessel Rust

Native Rust rewrite of Tessel. **Active migration, not yet a feature-complete replacement.** The application UI and backend are Rust, using egui/eframe. No Electron, Vue or Node application runtime is bundled. External shells and agent CLIs remain separate programs; the Windows browser uses Microsoft's WebView2 engine.

## Features

### Projects and workspace

- Persistent local projects; newly added projects become selected automatically.
- Saved theme, font size, one/two/three-column terminal layout, SSH hosts and per-project task boards.
- English/French interface selection in Settings, applied immediately and retained across restarts. Main navigation, settings, task columns, file actions, browser/download controls, PDF controls, sharing, vault and approval captions are translated. User content, tool protocol values and external/technical error messages retain their original text. Settings remain scrollable on smaller windows.
- Terminal and agent metadata restored asleep: reopening the app never automatically executes saved processes.
- Export/import portable Rust settings as bounded JSON, with a review of project paths and metadata counts before replacement. A local copy preserves previous settings; restored terminals stay asleep, projects require explicit selection, sharing stops and agent-browser access is revoked. Active work must be stopped or saved first. Exports contain paths and metadata but no credentials, browser profiles or project files, and are not encrypted.
- Atomic workspace writes, schema validation and a single-instance workspace lock. Corrupt or newer metadata is retained instead of silently replaced.

### Terminals, agents and SSH

- Real PTYs (ConPTY on Windows), independent tabs and grid layout. Switching projects keeps existing sessions running and drains their output.
- PowerShell without user profiles on Windows; `/bin/sh` on Unix.
- ANSI colors, underlines, cursor, resize, scrollback, copy-visible-text menu, bounded paste, bracketed paste and application cursor keys. Wide glyphs render above both cell backgrounds; painting is clipped to each pane. Installed Windows emoji/symbol fonts are used as fallbacks.
- Ctrl+A–Z, Ctrl+Space, Shift+Tab, Insert/Delete, Page Up/Down, F1–F12 and modified navigation/function keys. The input cursor is hidden while viewing scrollback.
- Drag to select terminal text; copy with Ctrl+C when selected, Ctrl+Shift+C or the context menu. Wrapped lines and Unicode are preserved; new output, scrolling or resizing clears the selection. Ctrl+C without a selection still interrupts the shell.
- Click to focus a terminal; background panes never take focus automatically. Tab, arrows and Escape stay with the focused terminal. IME preedit is drawn locally and only committed text is sent, with the candidate position at the cursor; OS-specific IME acceptance is still pending.
- Bounded input/output queues and shell termination on sleep or close.
- Detection and explicit launch of installed Claude, Codex, OpenCode, Gemini, Copilot, Aider, Amp, Kimi, Qwen, Grok, Cline and Cursor agent CLIs.
- SSH terminals through OpenSSH, config aliases, host verification, explicit reconnect, remote working directory and keepalives. No production server is contacted automatically.

### Files and Git

- Background directory loading with virtualized rows; paths are confined to the selected project and symlinks are not traversed automatically.
- Local file/folder creation and context-menu rename/deletion, scoped to the selected project. Windows rename refuses existing targets atomically; permanent deletion requires confirmation and only removes files or empty folders. Unsaved edits block file mutations, open paths follow renames, and background directory results cannot overwrite a newer refresh.
- UTF-8 text editor tabs (2 MiB limit), atomic saves, detection of external edits and unsaved-change guards on close.
- On-demand Git status, working/staged diffs, recent history, literal-path staging and explicit commits. No continuous Git polling; operations have a 20-second deadline and bounded captured output.

### Remote files

- On-demand SFTP browsing, original-file downloads (16 MiB) and UTF-8 editing (2 MiB) through OpenSSH config and existing key/agent authentication. Unknown host keys are refused; no background polling or automatic production connections.
- Canonical remote-root checks, bounded directory reads and a 30-second operation deadline. Saves compare the opened bytes again before atomic replacement and preserve ordinary permissions/ownership; servers without atomic rename support are read-only.
- Create empty files and folders directly in the current remote directory. Exclusive file creation and server mkdir refuse existing names; names cannot supply path separators or traversal. New files request owner-only 0600 permissions and folders 0700.
- Preview remote PDF and image originals directly in the panel, with bounded background decoding and the same isolated PDF renderer as local files. Temporary preview files are removed after loading; original bytes remain available for download.
- Explicitly confirmed deletion names the host and path. It permanently removes a file or an empty folder; there is no recursive deletion or remote recycle bin.
- Rename remote files and folders through their context menu. Existing destinations, changed targets and the remote root are refused; open document paths follow a successful rename.
- Unsaved edits and active requests block accidental close or host changes. Local downloads never overwrite an existing file. Password-only SSH authentication is not implemented yet.

### Document previews

- Native readable HTML and Markdown views, with source/preview switching and copy-as-plain-text. Scripts, CSS and external resources are never executed or fetched by this view; it is not a full webpage renderer.
- Local links open another editor tab while preserving the source; HTTP(S) links open the private browser. Native URL schemes, network-share links and paths outside the source project are refused.
- Read-only PNG, JPEG, WebP, GIF-first-frame and BMP previews, with background decoding, compressed-size/dimension/allocation limits and fit-to-width display.
- Original PDF page rendering in Rust with previous/next page, zoom and fit-width controls. Rendering runs in a disposable worker with a 20-second deadline; Windows additionally enforces a 512 MiB process-memory limit. Closing the document cancels its worker.
- PDF inputs are limited to 32 MiB/2,000 pages; raster output is bounded to 4,096 pixels per side and 8 million pixels. The source file is never rewritten. Unsupported or damaged documents report an error; forms, text selection and every PDF feature are not supported yet.
- Office parsing runs in a disposable worker with the same 20-second/512 MiB Windows limits as PDFs, for local and SFTP originals. Inputs are limited to 32 MiB; sheet/slide, row, column and output limits bound previews. Macros and formulas are never executed. Damaged/encrypted/unsupported inputs report an error.
- Bounded document loading and up to 24 open or loading documents. Office previews accept DOCX/ODT readable paragraphs and headings, PPTX/ODP text in slide order, and XLSX/XLSM/ODS sheets with cached cell values. These are content previews, not layout-faithful Office renderers; document tables are flattened and embedded images/charts are not displayed.

### Native Codex chat

- Rust client for the installed Codex app-server, using its existing sign-in without copying tokens.
- Streaming text, tool events, model/effort fields, follow-up steering, interruption, thread-ID persistence and explicit resume.
- Workspace-write sandbox and approval-on-request explicitly requested and checked. Unknown requests fail closed; approval requests are tied to the active thread and turn.
- Project-scoped task-list/add/move tools connected to the native task board. Model arguments cannot select another project's authority.
- Structured agent questions appear inline with explicit choice/free-text answers and a skip action. No choice is preselected or sent automatically; stale or resolved requests disappear. Secret-input requests are refused in favor of site sign-in. Questions do not grant command approvals.
- Pending approvals are displayed inside the chat instead of stealing focus with a popup. Allow is disabled when action details are missing or the requested permission scope is unsupported; stale-turn events cannot change the active conversation.
- Bounded resumed-history rendering, pagination when supported and a legacy read fallback. Some Codex app-server builds advertise these schemas but reject both history methods; the UI explicitly reports that limitation while leaving chat usable.

### Windows browser

- Embedded WebView2, back/forward/reload, address bar, separate persistent Rust-app profile and no website-to-native IPC bridge.
- Original HTTP(S), authenticated and blob downloads remain inside the browser session. A fresh directory per download prevents duplicate names overwriting existing files.
- User-selected download folder, received/total bytes, separate completed/cancelled/failed states, native cancellation and copyable saved paths. At most eight transfers run at once; the last 100 transfers stay in memory. Transfers with no data progress for 120 seconds are stopped and reported as failed. Download URLs, cookies and tokens are not logged.
- New-window requests can be opened explicitly in the existing tab. Native/file URL schemes from websites are refused.

### Project agent browsers

- Explicitly enable an isolated, temporary browser for a project's native Codex chat from **Browser > Project agent**. Up to four projects can have separate browsers; access is disabled again after app exit and can be revoked immediately.
- Newly started native chat threads register the browser tool; threads created before this feature may need a new conversation (stop the chat process, clear the resume ID, then connect).
- The `tessel_browser` tool provides bounded page snapshots, HTTP(S)/blob navigation, selector clicks, text-field entry, single-choice dropdown selection, checkbox/radio selection and scrolling to an element. Snapshots include bounded option lists, checkbox state and viewport scroll position. DOM action requests require the snapshot's exact origin, and ambiguous, hidden, disabled, password and file fields are refused. Arbitrary script execution is not exposed.
- Prepare the next download with an existing project-relative folder and a safe filename, then trigger the website's original download. A unique subfolder prevents overwrite; the one-use plan expires after 60 seconds. Agents can query the download ID, received/total bytes, completion/error status and saved path without a Save As popup, and cancel an active transfer by its ID.
- These profiles never copy owner-browser cookies or access the password vault, and are not exposed through LAN sharing. Sign in manually within the agent browser when needed. Page content is untrusted; enabling agent access allows the agent to read and interact with that project's browser session.
- The initial tool set does not yet provide screenshots, iframe interaction, arbitrary popup control or detailed download progress/cancellation.

### Native LAN sharing

- Explicit read-only host and guest sessions over TLS with an invitation-pinned certificate and a random access token. Stopping sharing revokes the invitation.
- Project-confined listing and original-file downloads (8 MiB per file), with no shell, write, browser or vault APIs. Dot paths, app-data paths and known credential filenames are excluded; ordinary project files still need to be suitable for sharing.
- Connection deadlines, concurrency and frame-size limits; guest downloads never overwrite existing files.
- Direct LAN connectivity only. This protocol does not interoperate with Electron Tessel Live Share or provide its cloud relay and collaborative editing yet.

### Private Windows password vault

- Explicit CSV import from Chrome, Edge or Firefox exports, stored with Windows user-bound DPAPI encryption outside project folders.
- Atomic saves, exclusive vault lock, duplicate-account updates, account removal and manual locking. Invalid input retains existing records.
- Manual fill in the private browser only, restricted to the account's exact HTTPS origin; no automatic form submission. Ambiguous forms are left untouched.
- No vault tools are available to agents or guests. Existing browser password databases and Tessel credentials are not accessed automatically.

### Private Tessel account

- Owner-only Windows account panel with external-browser PKCE sign-in, bounded HTTP requests, explicit verification and sign-out. Sessions are encrypted with Windows DPAPI outside projects; agents, browsers and LAN guests have no account-token API.
- Explicit sign-in registers only `tessel-rust://` for this executable. Callback state, expiry and one-use delivery are checked; the Electron protocols and existing account files remain untouched. No account request runs on startup.
- Local sign-out removes the saved session before server revocation; connection failures are reported. Account requests finish before normal window shutdown.
- **Production activation is pending:** the account server must accept the separate Rust callback scheme. The prepared backend patch and native fixture tests pass; real sign-in has not been exercised. Connectors, account deletion and cloud sync are not implemented yet.

### Windows distribution

- An optimized portable Windows x64 ZIP can be built with `./scripts/package-windows.ps1` from a clean committed checkout. The script builds with two compiler jobs, runs the original-PDF and Office worker acceptance checks, writes commit/build metadata and SHA-256 checksums, and extracts the ZIP to verify the executable bytes.
- GitHub Actions runs formatting, tests and strict Clippy before packaging. This is an unsigned portable development build, not an installer or published product release. Windows, WebView2 Runtime, Git/OpenSSH and agent CLI requirements still apply.

## Run and verify

Install Rust stable and native C/C++ build tools. Windows requires MSVC build tools, the Windows SDK and WebView2 Runtime. Agents, Git and OpenSSH must be installed separately for their corresponding features.

```sh
cargo run --locked -j 2
cargo fmt --check
cargo test --locked -j 2
cargo clippy --locked -j 2 --all-targets -- -D warnings
cargo build --locked -j 2
```

Windows executable: `target/debug/tessel-rust.exe`. CI produces an optimized portable ZIP with its checksum; no installer or product release is published yet. `--data-dir <folder>` selects a separate workspace metadata directory for testing.

Optional native acceptance checks:

```powershell
.\target\debug\tessel-rust.exe --self-test-browser
.\target\debug\tessel-rust.exe --self-test-agent-browser
.\target\debug\tessel-rust.exe --self-test-codex
.\target\debug\tessel-rust.exe --self-test-vault
.\target\debug\tessel-rust.exe --self-test-pdf
.\target\debug\tessel-rust.exe --self-test-office
```

The browser check creates a local HTTP fixture and temporary browser profile, downloads direct, cookie-authenticated and blob PDFs, and compares all saved bytes to the original synthetic PDF. Both browser checks also cancel a deliberately slow transfer after observing native progress and verify that an incomplete response cannot report completion and is stopped by the stall guard (three seconds in the fixture, 120 seconds normally). It uses no personal account. The agent-browser check repeats those original downloads with tool-selected names, checks returned paths/status, exercises snapshot/fill/click, refuses origin mismatch and password-field entry, and checks cookie isolation against a separate profile. The Codex check initializes the real installed app-server and verifies its applied permissions and probes history compatibility; it sends no prompt or model turn. The PDF check renders two synthetic original pages in the resource-limited worker and rejects invalid input. The Office check decodes synthetic DOCX and XLSX files through the actual isolated worker, checks original text/cached values and refuses damaged/cancelled input. The vault check uses a temporary profile and an in-process synthetic HTTPS site, without real credentials or network access. `cargo build --features visual-test` enables eframe's screenshot test support.

## Validation and remaining migration

Current Windows evidence: 41 tests pass, strict Clippy and formatting pass, native UI screenshot inspected, all three real WebView2 download cases pass, and real Codex initialization passes. Real loopback TLS host/guest tests and Windows DPAPI round-trip/tamper tests pass. Native WebView2 password-fill acceptance passes with a synthetic HTTPS origin, mismatched-origin refusal, ambiguous-form refusal and no form submission. Framed SFTP fixture tests cover listing, original reads, root escape refusal, atomic saves and edit conflicts. No live SSH server or paid inference turn has been used for acceptance testing.

See [MIGRATION.md](MIGRATION.md) for the acceptance ledger. Major remaining work includes full Live Share parity, credential agent consent/connectors/account sync, full remote filesystem management, extended browser tools for agents, other native chat providers, full document previews, comprehensive terminal protocols, installer/updater, remaining diagnostic localization and accessibility. Native Codex history depends on server support, and chat does not yet support every question/permission/tool type; unsupported requests are rejected explicitly. Popup-based OAuth flows and download pause/resume need further implementation. Session metadata survives restart, but processes and terminal output do not survive app exit.

No existing Tessel configuration, credentials, browser profiles or personal records are automatically imported or committed.
