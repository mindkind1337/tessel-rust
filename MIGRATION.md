# Native migration acceptance ledger

Source baseline: the Features sections of `C:\Tessel\README.md`, inspected 2026-11-02. This ledger tracks capability, not just UI buttons. A row is complete only when its implementation and boundary tests pass. No existing credentials or user documents are migration fixtures.

User requirement reaffirmed on 2026-11-03: reproduce Tessel's interface and behavior exactly in Rust. Existing native screenshots are development fixtures, not accepted visual parity. Compare matching synthetic workspaces, window sizes, display scaling, languages and themes before marking any screen complete. The reference source inspected for the first palette pass is Electron commit `2bb4184`: `src/renderer/src/style.css`, `themes.css`, `App.vue` and the workspace/pane components. Palette transcription alone does not satisfy layout, typography, icons, keyboard, focus or accessibility parity.

| Capability | Required acceptance | Status |
| --- | --- | --- |
| Native application | Rust GUI and backend; no Electron/Vue runtime | Initial window verified on Windows |
| Exact UI parity | Same chrome, project sidebar, pane grids/tabs, side panels, menus, themes, shortcuts and behavior as Tessel | Dual-renderer architecture integrated and verified: primary native **GPUI** (`gpui-ce 0.3`) hardware-accelerated renderer (`--renderer gpui`, default) + universal **`eframe`/`egui`** OpenGL Compatibility renderer (`--renderer compat` / `TESSEL_RENDERER=compat` + automatic runtime fallback) over one shared `app::Tessel` core, covering all 7 CSS theme palettes (`color-mix` sRGB), DirectWrite `Segoe UI Variable Text` / `Cascadia Mono` typography, 45 `resvg` vector icons + `gpui::canvas` primitives, frameless 40px caption toolbar, left project/worktree/Live-Share sidebar, center `ViewTabs` + `pane_grid` PTY terminals/Native Chats/Code-Diff-Rich-Preview editors + Windows WebView2 HWND stage, all 7 right `SidePanel` tabs (including 5-color Git swimlanes), all 22 `Settings` pages + `Passwords` vault & `Tessel Account`, and all workspace modals, menus and status/toolbar popovers |
| Terminals | Real PTY, input, resize, ANSI, background draining, shutdown, scrollback | Real PTY verified; Unicode/wrapped selection and focus controls implemented; IME event handling implemented, X10/VT200/motion with SGR/UTF-8 mouse protocols implemented and tested; real OS IME and live TUI mouse acceptance pending |
| Projects | Persistent roots, selection, no process starts on restore | Implemented; persistence and portable settings export/restore tests pass |
| Files/editor | Confined browsing/create/rename/delete; tabs; conflict-aware atomic saving; dirty close guard | Expandable native tree integrated with bounded background enumeration and cached rows; preserved expansions and project-switch cancellation; existing scoped file operations retained; real egui pointer, cancellation, cache and confinement tests pass; native tree rendering visually checked |
| Layout | Persistent grid; project switching retains sessions | Terminal split trees, divider resizing, maximize/restore, auto arrangement and geometric focus integrated; multiple independent native Codex/Claude chats now share the grid with pane-scoped resume metadata and dormant restoration; browser/editor leaves still require unification |
| Agent terminals | Detect installed CLIs; project cwd; no invented provider flags | In progress |
| SSH | Host verification, keepalive, explicit reconnect, remote cwd | In progress; live host acceptance pending |
| Remote filesystem | SFTP listing/read/save/download/create/rename/delete/previews; host verification; conflict protection | Protocol fixture including exclusive creation, collisions and scope rejection passes; rename/deletion refusal and bounded preview fixtures pass; live SSH authentication and real-server acceptance pending |
| Task board | Per-project cards; persistence; safe movement | Adaptive native board integrated with scoped persistence and native chat tools; broader orchestration parity pending |
| Git | Nonblocking, bounded status/diff/log; explicit mutations; worktrees; no auto polling storms | Structured Changes panel integrated with on-demand status, diff tabs, `+added / -removed` badges, staging/unstaging, confirmed discard, commits and remote operations; background Git worktree discovery, branch-prefixed creation with `.env*` secret copy, Windows NTFS junction-safe removal, sidebar sub-cards/hidden inbox, launcher toggle and `WorktreeCleanupModal` integrated and tested; native visual acceptance pending |
| Native agent chat | Provider transports, streaming, approvals, resume and history | Codex transport, initialization and history compatibility probe pass; history rendering is fixture-tested but unavailable on the installed server; Claude native initialization and streaming/approval/structured-question protocol fixtures pass, with French UI visually checked; Claude live model acceptance currently blocked by CLI sign-in requirement; tool execution and history plus other providers pending |
| Agent tools | Authenticated scoped tools, approval UI, messages and task board | Task tools, scoped structured questions, explicitly enabled project browser tools, HMAC-SHA256-authenticated Tessel Teams (`team_*`), bounded worker orchestration (`team_worker_*`, `team_heartbeat`), gated inter-project relay (`project_agents`/`project_send`/`project_inbox` with pair approval and untrusted framing) and digit-free idle wake-ups implemented and verified |
| Browser | Engine integration; isolated profiles; navigation/download tests | Windows WebView2 implemented; three native download cases and separate project-agent snapshot/click/fill/select/check/scroll/download/isolation acceptance pass; native progress, cancellation and incomplete-response acceptance implemented; scoped original PNG screenshots and native capture acceptance pass; iframe tools pending |
| Documents | HTML trust, PDF, images, Markdown, office documents | Native readable HTML/Markdown (with `600 KiB` / `1 MiB` size gate and native vector Mermaid `flowchart`/`sequenceDiagram` rendering), sticky-header `SheetView` (`5,000 × 200` CSV/TSV + XLSX/XLSM/ODS), collapsible JSON/JSONC/JSONL tree viewer, images and bounded original PDF/Office workers implemented and tested |
| Credentials | OS-protected storage; explicit agent opt-in; never available to guests | Windows DPAPI/CSV and owner-only UI implemented; encryption and native manual-fill tests pass; explicit agent-consent integration pending |
| Live Share | Authenticated transport; project scope; explicit guest rights; secret-isolation tests | Duplex TLS 1.3 LAN session with 8-flag `ShareRights`, 6-digit SAS knock approval (`900ms` arming delay + trust gating), multi-invitation management (`MAX_INVITES = 20`), `720px` dialog (`Overview`/`People`/`Invitations`/`Settings`), `24px` toolbar pill + `360px` popover, pane presence and `.osb-live` sidebar badges implemented and verified; cloud relay and Electron wire compatibility pending |
| Connectors/account/sync | Match existing service contracts; token isolation and consent | Native account PKCE, callback, DPAPI, HTTP fixture tests and end-to-end encrypted `scrypt` + `AES-256-GCM` cloud sync (`"rust_schema": 1`, bucket padding, shrink guard, `sync-floor.json` anti-rollback and restore preview) pass; backend callback deployment and OAuth connectors pending |
| Packaging/update | Native installer, integrity verification, safe restart, CI artifacts | Portable release packaging and SHA-256 readback implemented; installer, signing and updater pending |
| Localization/accessibility | EN/FR; keyboard navigation; screen reader acceptance | Persisted EN/FR interface selection and main-panel translations implemented; searchable native shortcut help visually checked, modal Escape/focus and document shortcut isolation tests pass; external/technical diagnostics and screen-reader acceptance pending |

The Rust application can invoke external shells and agent CLIs. An embedded browser requires an external browser engine. Those are dependencies, not a JavaScript implementation of Tessel. Existing service credentials and production endpoints are not exercised automatically.

## Isolated visual reference

Read-only inspection of the Electron startup identifies `TESSEL_USER_DATA` as the supported separate-profile override. It also skips user-data migration and development protocol registration; the PTY channel can be separated with `TESSEL_PTYHOST_CHANNEL`. A future reference capture must use a separate source checkout, a new empty profile and a unique PTY channel, with synthetic workspace data only. Do not start the active checkout or copy its real profile. This identifies the isolation mechanism; an isolated Electron reference has not yet been launched or accepted for pixel comparisons.

## Account / connector backend boundary

Read-only inspection of `C:\tessel-server` on 2026-11-02 found that `src/auth.js` accepts `tessel`, `tessel-dev` and `admin`, while `src/connect.js` accepts only the first two. The Rust client needs its own `tessel-rust://` callback and must not register over either running Electron application's protocol.

`integration/tessel-server-rust-scheme.patch` adds the separate scheme to those two allowlists. `git apply --check --ignore-space-change` passes against server commit `35ba421` (the checkout has mixed line endings). In an isolated archive of that commit, all 116 authentication/connector tests pass, including new Rust callback and one-use token-exchange coverage. It has **not** been applied to the live checkout or deployed, and is not sufficient by itself to enable native accounts. Native Rust PKCE/state/expiry validation, one-use encrypted callback delivery, Windows DPAPI session storage, owner-only UI, explicit protocol registration and logout/revocation are implemented. Tests cover the RFC PKCE vector, wrong origins/states/expiry/duplicates, encrypted persistence and exclusive locking, exact HTTP routes/authentication, redirect refusal and response limits. The native French account panel is visually checked. Production deployment, OS browser-to-protocol acceptance and real end-to-end sign-in are still required. Forced process termination during an in-flight token exchange can leave a server session until its expiry; normal UI close waits for the request.

The existing account contract uses `https://tesselcode.com/login?challenge=...&state=...&app=...`, a code/verifier POST to `/auth/token`, `/api/me` and `/auth/logout`. Connector flows additionally use `/api/connect/services`, `/api/connect/start` and service-specific exchange/refresh/revocation contracts. Tokens must remain outside projects, agent outputs and sharing. Native code must not copy the current Electron account/session files.

The existing sync contract at `/api/sync` and `/api/sync/versions` carries account-bound encrypted, compressed and padded snapshots with revision/conflict handling. Rust workspace metadata has a different schema: uploading it as an Electron snapshot would be unsafe. An explicit schema/compatibility design and opt-in restore preview are still pending. No account, connector or sync production endpoint was exercised during this inspection.

## Native overlay acceptance

The empty-project launcher lists detected CLIs and explicitly available native actions; it never starts a process merely because a project is empty. Its centered 520-pixel card, 48-pixel top inset, numbered rows and bounded scrolling follow ProjectLauncher.vue. Focused arrow/number/Enter input skips disabled entries and refuses held activation; source account/shell selection, recent-session discovery and remembered defaults remain pending. The command palette now injects projects, open sessions and documents with ID validation at dispatch, grouped results and shortcut hints.

Settings uses a native modal surface below the 40-pixel toolbar, a navigation width clamped to 200–290 pixels at 24% of the viewport, and a scrolling content column capped at 870 pixels. Only implemented sections are listed. Search covers section labels and explicit keywords, not private vault contents or every control. The SettingsNav rail is integrated with injected IDs for the six implemented sections, source vector icons for Agents/Appearance/Account/SSH, focus-scoped keys and a separate disabled state during restore confirmation. The full reference 22-page settings catalog and control-content search remain pending.

Project cards are rendered by the native `ProjectPanel`, using injected project/session metadata and emitting actions only. The application validates selected session IDs against their project, preserves maximized-pane selection, requests terminal focus, and refreshes the explorer/finder when the project changes. Card paths and titles are clipped with tooltips; collapsed state is retained by project ID during the run. Synthetic native captures cover French at 1280×800 and 800×520. The Search button opens the existing command palette; full worktree/browser search, project filters, activity and team controls remain pending.

The terminal/agent launcher is integrated separately from the project-creation launcher. The reference `LaunchMenu.vue` uses a 340-pixel menu; the keyboard opening position is centered horizontally at y=80. Native captures at 1280×800 and 800×520 show the placement selector, grouped catalog, disabled entries and keyboard footer without clipping the menu. A real egui Popup test verifies that placement and disabled-entry clicks keep the menu open, while launching and outside clicks close it. Dispatch accepts only available injected entries and targets a pane in the active project. The left-split test checks neighboring geometry and rejects duplicate/unknown identifiers. Automatic placement uses the existing native grid; account/worktree/host selection remains pending.

The status-bar pass uses `StatusBar.vue`, `App.vue:statusInfo` and `.sb` in `style.css`: 24-pixel height, 12-pixel horizontal padding, selected input target and pane summary at left, build at right, and workspace path hidden below the 900-pixel compact breakpoint. It counts native session metadata in the active project, including dormant panes; it does not present that count as running processes. Process-tree memory, ports, keep-awake and remote-host health require real collectors/actions before their indicators can match the source; no invented measurements stand in for them.

Settings and sharing now open over the existing center view. Settings sections expose the implemented appearance, CLI detection, SSH hosts, owner passwords, account and backup controls. Closing the overlay preserves the selected document/terminal/browser. Native WebView surfaces are hidden and underlying terminal input is disabled while dialogs are open, including file and exit confirmations. Full source Settings page coverage is still pending.

Panel visibility and the selected Files/Changes/Tasks tab now persist in workspace settings and portable backups. Implemented source shortcuts: Ctrl+Shift+X/G/K for side panels, Ctrl+Shift+T for a terminal, Ctrl+Shift+P for commands, Ctrl+, for Settings. Routing distinguishes a focused terminal from text inputs and defers while confirmations are open; the full shortcut inventory remains pending.

The file finder is integrated with Ctrl+Shift+J and the command palette. It uses Franklin's bounded walker on one worker thread, with cooperative cancellation, no automatic scanning at startup, and cached fuzzy results recomputed only when the query or enumeration changes. Closing or switching projects discards late results. Opening still uses Document::open and its project scope validation. Native visual and worker lifecycle acceptance are tracked with synthetic fixtures; shell path insertion is not yet integrated.


## Appearance and experimental Windows frame acceptance

The Appearance component is integrated with persisted theme, language and terminal/editor font size. Native compact and maximized captures show all seven theme cards, language controls and font controls. A real click changed the theme immediately and persisted `warp` to a synthetic workspace. Descriptions now wrap at word boundaries. Five Appearance integration tests pass after that correction; the preceding full integrated suite passed 165 tests, and strict all-targets/all-features Clippy passes.

The Windows custom frame remains opt-in: build with `visual-test` and launch with `--native-frame-fixture`. Normal launches retain native decorations. A native test exposed ignored caption-button clicks; paired non-client clicks now post normal Windows system commands. Manual acceptance verified maximize, restore, minimize/reactivation, close confirmation and cancellation, title-bar double-click, dragging, and corner resizing (813x556 to 733x496). Windows Snap choices appear on maximize hover. Selecting Snap layouts, mixed-DPI monitors and native WebView positioning still need acceptance before enabling the frame by default. Test data contained only synthetic dormant sessions.


## SSH transport review

The interactive terminal launcher supplies OpenSSH `ConnectTimeout=15`, `ServerAliveInterval=30` and `ServerAliveCountMax=6`. SFTP uses a separate short-lived subprocess for each requested operation: batch authentication, strict host-key checking, no control-master reuse, a 10-second connect timeout, 15-second server-alive interval and two missed replies. Its library request timeout currently defaults to 10 seconds, and connection setup is limited to 30 seconds and operations to five minutes. Windows job objects and child cleanup bound subprocess lifetime. These are code-review findings, not live-server acceptance or a diagnosis of Electron disconnections.

A virtual-clock framed SFTP fixture reproduces the former 30-second failure on a progressing transfer, then verifies byte-exact completion with the longer operation deadline. It also expires an atomic save while a temporary-file write is pending, verifies bounded cleanup and confirms the original bytes remain unchanged. Cleanup state survives cancellation of the operation future; cleanup failure is explicitly reported. A broken connection or an unacknowledged file creation can still leave a remote temporary file, and a lost rename acknowledgement can make final save status uncertain. Host-key checking and byte limits remain unchanged. This is simulated protocol acceptance, not evidence about production network packet loss.

Single-file staging now validates project-relative components and existing canonical ancestors before invoking literal Git pathspecs. A disposable nested-repository fixture reproduces the former sibling-path escape, verifies its refusal and checks literal bracket/accented/dot-prefixed names and tracked deletions. Directory-wide staging is refused by this single-file action. The link refusal fixture also runs on Unix; Windows reparse-point acceptance is not newly claimed by this change.

Changes staging sends validated literal paths through NUL-delimited stdin, avoiding wildcard expansion and command-line length limits. All paths are validated before the Git subprocess starts. Incomplete status and stale/non-stageable selections are refused. Git input write failures are surfaced rather than silently ignored. A completed explicit Git operation triggers one status refresh only for its originating project. Native visual acceptance of the integrated buttons remains pending.

Unstage uses a literal path-scoped index reset and preserves working files. Real temporary-repository tests cover an unborn branch, existing commits, new/modified files, invalid mixed selections and a staged rename whose old and new paths must both leave the index. The Changes adapter accepts only currently staged entries and refuses incomplete status.

Terminal startup and SFTP panels explicitly handle disconnected worker channels, release their busy state and surface an error. SFTP failure keeps the original baseline and unsaved editor text instead of marking a save successful. Channel fixtures cover empty/pending, disconnection and subsequent operation results without opening a real terminal or contacting a server.

Native chats are now keyed by pane rather than by project/provider. Legacy project resume metadata migrates once to the existing saved pane; new panes do not inherit another pane's thread. A persisted pane counter and retained closed-pane metadata prevent identity reuse on restart. Tests cover independent resume IDs, wrong-provider/project rejection, one-time migration, dormant persistence and browser reply ownership among chats in the same project. The project agent browser remains explicitly shared and serialized; per-agent browser instances and a closed-thread history picker are still pending. Multiple live provider connections have not been exercised for this acceptance step.


## SSH settings and agent-name integration

The SSH adapter uses snapshot-checked host metadata changes and persists before publishing a successful update. The native panel receives only supported capabilities; no connection status is inferred from a running process. Save retains the form until persistence succeeds, including repeated identical backend errors. The eight headless panel tests cover themes/languages, capability gating, narrow width, keyboard focus, nested Escape and removal confirmation.

Agent names use the source list and UTF-16 validation rules, are stored separately from program titles, and expose rename through the pane menu. Source differential fixtures, workspace reopen/migration, backup roundtrips and project isolation for the native Codex `tessel_agents` roster pass. Full integration passed 205 tests; subsequent import-read hardening and two extra boundary cases passed the five focused SSH config tests. Integrated native visual acceptance and live provider roster calls remain pending.

Explicit OpenSSH import discovers concrete aliases without copying connection credentials or executing config commands. Includes, cycles, per-file/total bytes, file counts, glob entries/matches and pattern work are bounded. Known key filenames, public-key siblings and recognizable key headers are excluded. Changing files are discarded; imports preserve existing manual folders and apply metadata batches atomically. Synthetic fixtures cover glob classes, quoted directives, includes/cycles, private-key exclusions, expansion limits, byte limits and rollback. Original OpenSSH options remain resolved by OpenSSH when connecting. Native hostname/user tokens use OS identity; UID is available after a Unix account lookup and is unavailable on Windows, matching the reference contract. Source locale-specific glob ordering still needs parity work. The new import has not read the user's real SSH config during validation.

Include environment and token substitutions enforce the path budget before each append. A pure fixture reproduces expansion amplification using a 32,000-byte synthetic value; oversized substitutions and UTF-8 boundary overflow are refused without publishing a partial path. All 134 binary tests pass, including seven config tests and the shared JavaScript-whitespace name fixtures. Literal incomplete environment expressions and a single optional equals sign retain the source parser’s behavior. The analogous retained Vue defect was demonstrated through a mock filesystem and reported to Darwin; no real SSH config was read.


Windows identity acceptance explicitly ran `cargo test --locked -j 2 --bin tessel-rust native_identity_matches_node_os_with_spoofed_environment -- --ignored`. Native account/hostname/UID results match an installed Node OS reference even when the child reference process receives spoofed USER, USERNAME, HOSTNAME and COMPUTERNAME variables. The test captures metadata in memory without printing it. The normal suite also uses injected identity and synthetic Include files. Windows uses GetUserNameW and balanced Winsock initialization/GetHostNameW; Unix uses reentrant account lookup and gethostname. The Unix runtime path has not yet been exercised in this Windows acceptance. Node is used only by the explicitly selected reference test, not by the application. API comparison source: [libuv Windows implementation](https://github.com/libuv/libuv/blob/v1.x/src/win/util.c), [Unix implementation](https://github.com/libuv/libuv/blob/v1.x/src/unix/core.c).

Agent PATH detection now runs on one background worker rather than on the UI thread. Initial loading and errors are distinct from an empty successful scan; refresh retains the last good snapshot. Deterministic channel fixtures cover initial loading, successful replacement, refresh failures, disconnected workers, and duplicate refresh suppression; a real worker test verifies execution off the calling thread. This change preserves the existing PATH lookup semantics, not yet the full Vue detection rules or executable/version probing. Slow filesystem calls cannot be cancelled; only one scan may be pending.

Acceptance for background detection: 139 binary tests passed, one optional Node reference test remained explicitly ignored in this run; all-target/all-feature strict Clippy and formatting checks passed. Native visual acceptance of the forthcoming Agents panel remains pending.

Per-agent menu visibility now defaults to enabled when absent and persists before updating live state. The CLI launcher, empty-project entries and native chat commands honor it without deleting saved sessions or stopping running panes. Portable settings backups carry these booleans. Tests cover old-state defaults, restart, re-enable, failed-write rollback, unknown IDs and backup roundtrips. The temporary settings controls will be replaced by the faithful Agents panel; launch arguments, environment overrides, model defaults and other settings are still pending.

Visibility acceptance: 141 binary tests passed, one optional Node reference test ignored; strict all-target/all-feature Clippy and formatting checks passed. Visual parity remains unaccepted until the integrated native panel is inspected.

The complete 42-entry built-in catalogue is transcribed from source AGENT_PRESETS, preserving order, labels, commands, accents and documentation links in a source-hashed fixture. Detection resolves preset IDs to executable names, and launching supplies static arguments separately/quoted for the platform shell. Older cursor-agent metadata stays readable through an alias and preserves visibility choices. No provider CLI was executed during command-builder validation. Fresh Windows registry PATH, extra installer directories, source Kimi notFrom exclusions, executable/version probes, custom agents and exact brand rendering beyond the initial panel remain pending; the expanded catalogue alone does not prove those behaviors.

Catalogue acceptance: 145 binary tests passed with one optional Node reference test ignored; strict all-target/all-feature Clippy and formatting passed. Command-builder fixtures checked Kiro, Hermes, Continue and the Cursor alias without launching a provider. Live provider compatibility and the full detection search-path rules remain separate acceptance work.

Installer-directory discovery now follows toolDirs.js: source-ordered native installer folders, Store/python.org user script locations, existing-directory filtering and case-insensitive append deduplication. One search-path snapshot serves an entire agent scan, and the same extended path is passed to newly launched terminals. Synthetic temporary directories exercise precedence and package/version filtering without launching providers. The base path reuses portable-pty 0.9.0’s current machine/user registry environment snapshot on Windows instead of overwriting it with the stale parent process PATH. Live registry-change acceptance and Python-reported script-directory probes remain pending, as do where.exe first-match semantics and Kimi exclusion.

Installer-path acceptance: 148 binary tests passed before the final terminal-environment snapshot adjustment; both focused directory tests passed after it. Strict all-target/all-feature Clippy and formatting passed on the final code. No installer or agent CLI was executed by these fixtures.

Kimi detection now applies the source notFrom exclusion to the first matching executable, retaining PATH precedence even when a newer install appears later. Nineteen cases were generated by executing only the source regex in Node against synthetic path strings (including separators, casing, extension and newline boundaries). A temporary-files fixture checks shadowing and reversed search order. Full where.exe/PATHEXT parity and real provider execution remain pending.

Kimi acceptance: both focused tests pass (19 source-regex cases plus first-match shadowing); strict all-target/all-feature Clippy and formatting pass. This is parser/search fixture evidence, not live Kimi provider acceptance.

Windows agent discovery now sends the catalogue executable names to one hidden system where.exe process, uses first-match/PATHEXT behavior, accepts partial-not-found exit status, and applies the Kimi exclusion afterward. Lookup runs on existing background workers, limits output to 1 MiB and waits at most ten seconds for process/output completion. Lookup errors preserve the previous detection snapshot. Agent startup checks the same lookup semantics. Source-generated parseWhere fixtures and a real where.exe test use synthetic command filenames; no provider is executed. Python script-directory probes, broader cross-platform lookup and live provider launch acceptance remain pending.

Grouped lookup acceptance: 152 binary tests passed, one optional Node identity-reference test ignored. This includes source parseWhere fixtures and the real Windows lookup against temporary non-executable fixture files. Strict all-target/all-feature Clippy and formatting passed. Timeout/output-limit branches are bounded in code but were not fault-injected in this acceptance.

Installation metadata is embedded from the actual source checkInstall output for all 42 presets (31 accepted step lists, 10 requiring confirmation, 11 documentation-only). Source hashes record provenance. The catalogue preserves shell, source, confirm and docsUrl independently from ordinary agent documentation; aliases resolve through agent_catalog. The temporary settings page can copy missing-agent instructions without executing them. Backend execution, confirmation dialog and automatic refresh after an explicit installation remain pending.

Installation-metadata acceptance: two focused tests pass, with strict all-target/all-feature Clippy and formatting clean. No installer ran; the pending panel’s visual acceptance and installation execution flow are not claimed by this check.

The first integrated Agents panel replaces the flat diagnostic list: installed/missing groups, single expanded details, per-agent menu switch, terminal/native-chat launch, official documentation/install links and checked instruction copying. Loading/error snapshots come from Detection, visibility is persisted by agent_preferences, and native chat is project-gated. Escape is handled before SettingsNav so details fold before closing Settings and held repeats do not propagate. This is an intermediate implementation: exact brand vectors, source tab/row metrics, complete keyboard navigation, advanced preferences, installer confirmation/execution and native screenshot acceptance are still pending.

First-panel acceptance: 157 binary tests passed, one optional Node reference test ignored; strict all-target/all-feature Clippy and formatting passed. New headless egui tests exercise real pointer input for detect/launch/docs/copy, the project gate, disabled controls, Escape repeat handling and a 300 px width bound. These tests do not establish pixel-perfect or native screenshot parity.

Agents row alignment now follows the source 44 px structure with title/command on the left and an independent accessible visibility switch plus disclosure chevron on the right. Text is single-line ellipsized within bounded rectangles. Escape is scoped to the expanded row’s focusable controls and restores row focus; external Settings focus keeps the event. Brand icons, remaining source metrics and native screenshot acceptance are still pending.

Row-alignment acceptance: all four focused headless Agents tests pass, including first-frame Escape after pointer focus, held repeats, external focus, Space on the visibility switch and narrow layout. Strict all-target/all-feature Clippy and formatting pass.

Agent launch-preference foundation: command/argument/environment strings now survive local Store persistence, visibility edits retain them, and reset clears customization while preserving a hidden agent. The source environment parser is ported with ECMAScript whitespace, UTF-16 text limits, reserved TESSEL_ names, duplicate-key handling and all-or-nothing errors. A source-hashed fixture records 24 outputs generated by the current Vue parseEnvText function using synthetic values only. Portable exports clear environment values and imports discard executable overrides while preserving visibility. The settings text fields, effective launch composition, permission modes, session model defaults and terminal wiring remain pending; this foundation does not yet change executed commands.

Launch-preference foundation acceptance: 163 binary tests passed, one optional reference test ignored; strict all-target/all-feature Clippy, formatting and diff checks passed. Persistence/reset/failure and export/import tests use synthetic data only.

Launch-preference wiring: Agents details now exposes command/arguments/environment text fields, inline parse errors and Reset. Changes commit on blur or folding the details, with pending persistence handled before the launch action; save failures prevent launching with stale settings. New/reconnected local agent terminal panes snapshot the current preferences; running panes remain unchanged. Custom command/arguments use the local shell, and valid variables are applied only to agent launches, not shell/SSH/native chat. Invalid parsed environment blocks contribute no partial values, matching effectiveAgent manual mode. Native-chat defaults, model/permission composition, session launch signatures, external first-start handling, source exact metrics and live visual acceptance remain pending.

Launch-wiring acceptance: 166 binary tests passed (one optional reference test ignored), including a real local PTY that receives quoted arguments and a synthetic environment value. Five focused Agents UI tests pass after navigation/pending-save handling; strict all-target/all-feature Clippy, formatting and diff checks pass. Restoring settings clears stale editor drafts. No provider or installer was invoked.

Session-model foundation: the native metadata snapshot now includes all 12 source catalogues and 39 seeded models, option choices/defaults and discovery policies. Rust merging preserves CLI-authoritative membership, Claude replacement, source option mappings, grouped Cursor variant options and duplicate insertion order. The option-inspection tokenizer, alias removal, Codex effort-config detection/removal and safe model-value predicate follow source behavior (including the `--` boundary). They do not execute or tokenize shell commands for launching. Executable catalogue functions are absent from the JSON by design: model/effort launch composition, discovery probes, persisted defaults and picker wiring remain pending, and no default model flags are added yet.

Session-model foundation acceptance: 30 focused agent tests pass, including 69 flag/removal reference cases, 14 tokenization cases, 14 value cases and 13 catalogue-discovery merges generated directly by the current Vue source. Fixtures record its SHA-256 and use synthetic discovery rows.

Model-default integration: all twelve catalogue launch composers now run in Rust, with manual flag precedence, shell-safe values, no implicit option defaults, Antigravity suffix normalization and Cursor grouped-variant selection. A 1,005-case fixture executes the source sessionOptionLaunchText across all catalogues, model/effort choices, overrides, malformed values, dynamic options and Cursor variants. Settings exposes the built-in model list and launch-supported options; persisted per-model choices survive clearing/reselecting a model and restart. New and reconnected agent terminals compose those choices before user arguments. Native-chat defaults, CLI discovery, per-pane overrides/switching, selected-model labels in headers, full French option labels and native visual acceptance remain pending. No account provider is invoked by validation.

Model-default acceptance: full binary suite passed with 171 tests and one optional test ignored before the final reconnect adjustment; afterward 34 focused agent tests and the new headless model-picker click test passed. The 1,005 differential launch cases match Vue. Strict all-target/all-feature Clippy, formatting and diff checks pass. Native visual parity and account discovery are not established by these tests.

Agent icon integration: installed rows reserve a 20 px icon plus 10 px gap before title/command, and missing-agent rows use 16 px icons. The shared brand painter uses cached source-space vector geometry, source keylines and contrasted initial tiles, with hidden-agent dimming. SVG curves/fills are discretized; the implementation does not establish pixel-perfect parity by construction. Geometry tests and integrated pointer/width checks are tracked below; native raster/screenshot comparison against Vue remains an open acceptance gate.

Icon acceptance: Franklin's isolated suite passes 7 geometry tests across 42 presets and 25 vector IDs/aliases/shells. After integration, all 7 Agents panel tests pass, including the icon-to-text gap and existing pointer/keyboard/narrow-layout checks. Strict all-target/all-feature Clippy and formatting pass. This is geometry and headless UI evidence, not completed native screenshot parity.

Model discovery parser foundation: Rust now parses all eight source listing formats (Claude control-response JSONL, Codex JSON, Grok bullets, OpenCode provider/model lines, Pi tables, Cursor labels/default markers, Antigravity tab-separated rows and Muse model/list JSON). The separate persisted/IPC sanitizer retains recognized fields, bounds rows/text/effort levels and removes duplicates. Source limits and format-specific whitespace/line handling are preserved, including Unicode separators. Synthetic fixtures cover hidden/disabled models, first successful Claude reply, defaults, duplicate order, malformed records, 200/300-row limits and oversized input. This adds no provider invocation: the process/protocol runner, discovery-to-option conversion and Refresh UI remain to be integrated.

Model discovery cache foundation: `agent_model_cache` loads only sanitized successful lists for the eight supported providers from `agent-models.json`, without starting a CLI. Missing, malformed and oversized files yield an empty cache; unknown providers and fields are discarded. Empty or unsupported refresh results preserve the last success. Atomic writes retain the new result in memory on a storage error and retry the whole snapshot on the next success. Tests use isolated temporary directories, including a blocked cache directory and recovery. The cache is not yet wired into application startup or Refresh; no account data was imported or queried.

Parser acceptance: 87 source-generated format cases, their 87 sanitized outputs and 6 dedicated persisted-row cases pass (180 comparisons). Strict all-target/all-feature Clippy, formatting and diff checks pass. Fixtures include the source SHA-256; no real account/model-list command was run.
`agent_model_options` now reproduces dynamic effort-option metadata for Claude, Codex, Pi, Antigravity and Muse. A reproducible source-extraction script checks all 512 subsets of nine levels for each provider (2,560 comparisons), including unknown levels, fallback defaults and Claude fast-mode metadata. These are display defaults only: launch still requires explicit choices. Grouping Cursor/Antigravity variants and mapping whole listed rows remain pending before this can feed the picker.
Antigravity variant grouping now preserves single variants and mismatched labels, combines only matching multi-variant base IDs, retains default flags and sorts advertised efforts in source order. A source-extraction script verifies 264 label cases and 32 grouping cases. Cursor grouping and the complete listed-row conversion remain pending; this module does not yet change the live picker.
Source synchronization (Vue 8307b92d): the seeded Cursor Thinking option now defaults to false, matching explicit-only launch behavior. Regenerating the entire 12-catalogue snapshot changes only that value and its source hash. The extraction script is retained for reproducible future comparisons. All eleven focused model tests and strict Clippy pass.
Model-probe process foundation: a worker-callable direct-executable capture now drains stdout/stderr concurrently, closes stdin after the optional listing request, hides Windows consoles, shares a four-million UTF-16-unit output budget, and kills/reaps the direct child after timeout or capture failure. UTF-8 split boundaries are retained correctly. Fixed listing arguments and completion/stderr-fallback handling cover seven CLI probes; Muse explicitly requires its model/list protocol and cannot accidentally start a bare serve capture. Seventeen focused model tests pass, including real synthetic child processes for stdin, exit status, excess output and timeout; the child-fixture test is ignored during ordinary suites and invoked by the parent tests. Program/shim resolution, Muse protocol, application worker/cache/Refresh wiring and descendant-process lifecycle acceptance remain pending. No installed provider was invoked.
Program resolution foundation: agent_program queries the fresh Windows registry PATH with a fixed hidden PowerShell script, passes the requested command as data, resolves native executables and recognizes npm JS/native shims and the sibling-only Cursor PowerShell launcher. Node beside the launcher takes precedence over global Node. Shim reading is capped at 256 KiB. Ninety-four source-generated shim cases match, and a real synthetic npm launcher verifies literal URL/percent/quote/metacharacter arguments through Node while proving its unrelated batch command was not executed. Four focused tests and strict Clippy pass. No provider CLI was executed; application discovery wiring remains pending.

Native model-picker integration: Franklin's AgentModelDefaultsUi now replaces the initial ComboBox controls in Agents settings, with EN/FR labels, explicit-only switches, side-by-side wrapping fields, prefix search and popup-first Escape handling. Navigation closes transient menus before saving drafts. Fifteen standalone selector tests and all 195 binary tests pass (two optional fixture/reference tests ignored); strict all-target/all-feature Clippy and formatting pass. Isolated native French captures at normal and compact viewport sizes were inspected using synthetic data; this validates the integrated layout, not pixel-perfect equivalence to Vue. The visual fixture disables desktop input and never starts installed agents or reads existing app settings.

Discovery worker foundation: one background request per provider combines program resolution, bounded capture and the sanitized success-only cache. Polling performs no filesystem operations; serialized background snapshots prevent older writes from overwriting newer results. Failures/disconnected workers retain previous lists, and disk failures keep new results in memory for the next successful write. Synthetic tests cover deduplication, disconnected workers, recovery and multiple-provider snapshots. It is not yet connected to the application: complete listed-row conversion (including Cursor grouping), Muse protocol, Refresh UI, and a shutdown-save drain remain pending. No provider account was queried.

Listed-row conversion foundation: agent_model_listed maps sanitized rows into picker metadata for Claude, Codex, Grok, OpenCode, Pi, Antigravity and Muse, including descriptions, default markers, effort/fast options and Antigravity grouping. Codex conversion emits an empty options array when no effort levels are advertised; the later source catalogue merge can inherit seed options for unknown models. A source-extraction fixture validates 343 cases across all seven providers; strict all-target/all-feature Clippy passes. Cursor explicitly returns unavailable until its separate grouping port lands, so callers must retain seed data rather than misrepresent raw variants. Application discovery remains unwired.

Discovery application integration: cached converted lists now populate Agents settings on startup without invoking providers. Explicit Refresh is connected for Claude, Codex, Grok, OpenCode, Pi and Antigravity, with busy/error/date/count feedback, retained last-success data, and a warning when disk persistence fails. Both new and reconnected agent terminals compose flags from the same merged catalogue as the picker. Closing waits for pending discovery/writes, preventing normal exit while a direct probe or cache snapshot is outstanding. Custom executable paths with spaces follow the source command policy. Generated fetchedAt values use integer milliseconds like Date.now, fixing an observed floating-point serialization mismatch.

Acceptance: 199 binary tests passed, two fixture/reference tests ignored, and strict all-target/all-feature Clippy passed. A synthetic npm shim in a path containing spaces verifies fixed Codex listing arguments, actual process capture, parser/conversion, disk persistence and reopened catalogue metadata. A headless integrated panel test selects a cached dynamic model and checks that Refresh emits an action without directly starting a process. Launch tests preserve both Claude replacement and Codex seed-option inheritance. Cursor grouping and Muse protocol refresh remain pending; timestamps currently use local time in an ISO-like format rather than the source's full regional Intl formatting. No provider account was queried.
Native acceptance: isolated normal and compact French captures with a synthetic on-disk catalogue were inspected. The dynamic model label, saved effort, fetched-date/count status and Refresh action render correctly; Refresh wraps below the fields in the compact layout. Existing desktop input and provider accounts were not used.

Muse local-catalog foundation: muse_catalog reads only sorted regular JSON files from model-catalog (at most 20 files, 1 MiB each and 300 rows per file), preserving visible-first deduplication and Muse model/list field shapes. A 100,000-entry directory budget returns no answer instead of claiming a partial list. XDG overrides apply only to the real home and absolute paths. Twenty source-generated comparisons use fake async files; three focused tests cover source parsing, sorted file/size limits and XDG scoping. Strict Clippy passes. The module is not yet called by Refresh: direct Muse executable resolution and the initialize/model-list protocol fallback remain to be integrated. No real Muse catalogue, sign-in file or session log was read.
Source synchronization (Vue b1793, 10e09e6): model-list status now uses the singular English/French wording for one result. French zero follows the source Intl plural category; English zero stays plural. The existing selector status test now covers both cases in both languages. All 15 selector tests and strict all-target/all-feature Clippy pass.

Muse discovery integration: Refresh now reads the bounded local catalogue first, then resolves Muse's native executable (PATH before installer folder; validated .muse-version for the Windows launcher) and performs initialize/initialized/model-list only. The fixed server argument is passed directly without executing its updater shell launcher. A 60-second discovery deadline, one-MiB UTF-16 stdout budget, drained/discarded stderr and closed-stdin/two-second graceful-exit window bound the exchange; the direct child is killed/reaped if needed. Success flows through the same parser/cache/menu/launch path as other providers. Protocol errors return bounded display details without logs or cache persistence.

Muse acceptance: 207 binary tests pass, three fixture/reference tests ignored. Synthetic child-process tests verify the exact handshake/query, ignored notifications/requests, EOF on stdin, malformed initialization, server errors, excess output and timeout. Resolver tests cover native-file precedence, validated version names, quoted PATH folders, absolute override requirements and Unix fallback. A local-catalogue test proves the server closure is never invoked when metadata exists. No installed Muse binary, account or real catalogue was accessed during validation.

Cursor discovery integration: listed variants now feed the common cache, Refresh, picker and launch pipeline. Grouped rows retain advertised effort/Fast/Thinking combinations and source family/version ordering. Unicode label ordering uses compiled ICU collation data rather than an ASCII approximation; differential fixtures capture the source runtime locale, not every operating-system locale. Synthetic source comparisons cover 34 parsed IDs, 13 grouping suites, the 246-row reference list grouped into 39 rows, all concrete-variant round trips and 3,159 model/option launch combinations. Ten standalone tests and 208 binary tests pass (three fixture/reference tests ignored). No installed Cursor CLI or account was queried.
Cursor visual acceptance: isolated normal and compact native French captures were inspected with three synthetic cached variants. The grouped model, saved high effort, enabled Fast switch and Refresh action render together; switches/actions wrap below the model fields at compact width. Strict all-target/all-feature Clippy, formatting and diff checks pass. This does not establish whole-application pixel parity or real-account discovery acceptance.

Regional model-list dates: the picker now renders medium dates and short times with ICU Unicode data, in the local time zone. Locale selection uses the first system language matching the UI language, falling back to fr-CA/en-US, following Vue's matching/fallback rule (Vue obtains its candidates from browser languages instead). Windows reads a bounded preferred-language MULTI_SZ once; Unix uses language environment preferences. Synthetic differential evidence covers eight locale selections and 240 dates across fr-CA, fr-FR, en-US, en-CA and en-GB. V8's normalized day-period separator is preserved. Non-finite and out-of-JavaScript-range timestamps are rejected. Regional acceptance outside these fixtures and changing OS language preferences without restart remain unverified.
Regional date acceptance: the 240 Intl comparisons and eight locale-selection cases pass, including a local Windows preference smoke check and invalid timestamp rejection. All nine integrated Agents panel tests and strict all-target/all-feature Clippy pass; formatting and diff checks pass. No live agent or browser account was used.

Per-pane model persistence foundation: SavedSession carries an optional bounded session_options object. Old metadata defaults to inheritance; restore/import preserves each agent terminal choice while keeping sessions asleep. New/woken terminals pass the complete explicit choice to launch composition before global defaults, without inheriting unrelated effort options. Manually authored CLI flags keep priority. Workspace saves reject malformed choices, excess options and options attached to shells/native-chat panes; failed writes retain the old file. Backup round trips preserve valid choices. No interactive picker or live-switch action is exposed by this foundation.
Acceptance: 210 binary tests pass (three fixture/reference tests ignored), including independent pane/global choices, manual argument priority, restart round trips, old-schema compatibility, backup preservation and failed-write retention. A follow-up bound check covers ten option entries plus model, matching the source normalizer's maximum output shape. Vue source review found its launch signatures already excluded from portable export, synchronized layouts and the shared layout whitelist; no confirmed Vue bug was reported.
The final option-bound test, formatting, diff checks and strict all-target/all-feature Clippy pass.

Launch-change detection: agent terminals retain a SHA-256 fingerprint of the source-style command/args/sorted environment tuple, never the tuple itself. Workspace validation accepts only fixed-size hexadecimal fingerprints; portable export and import drop them. Effective launch comparison runs after settings edits, discovery changes and startup/exit events instead of hashing every render frame. Sleeping, starting, exited and legacy panes without a known fingerprint do not show a stale-launch warning. Wide headers show the source EN/FR pending-settings label. No automatic restart or restart command is exposed: preserving the provider conversation ID on restart still requires the resume pipeline.

Conversation resume planning foundation: `agent_resume` represents verified, missing and unavailable target lookup outcomes separately, preserving the source's limited legacy fallback. It builds validated arguments for 15 own-ID providers and mirrors Claude/Codex fresh-versus-resume choices. Differential fixtures compare 1,088 synthetic argument cases and 18 start choices against the actual source functions. Two integration tests pass. The planner performs no transcript lookup, process launch or network activity; runtime session discovery, ownership verification, command assembly and restart integration remain pending. Identifier tests additionally reject trailing control characters. Strict all-target/all-feature Clippy and formatting checks pass.
Five synthetic source signature/hash cases cover ordering, quoting, Unicode and environment values. Tests cover effective environment identity, explicit-setting changes, asleep/loading/missing-fingerprint states, export exclusion and untrusted import stripping. The final binary suite passes 212 tests (three ignored), including import stripping and discovery-event assertions. Strict all-target/all-feature Clippy, formatting and diff checks pass. Isolated normal and compact native captures confirm left-aligned pane titles, a visible pending-settings label at normal width and omission at compact width. No installed provider or account was used.

Claude/OpenClaude terminal continuation: managed launches generate a UUID, persist it with the pane, and inspect only transcript metadata on the startup worker. Existing transcripts select --resume; missing transcripts select --session-id with the same UUID. Explicit resume/session/continue flags bypass managed selection. The configured/inherited CLAUDE_CONFIG_DIR scope is respected, and lookup errors/budget exhaustion surface instead of claiming an absent conversation. Workspace validation rejects non-UUID or wrong-provider metadata; legacy records remain compatible. Synthetic real-PTY acceptance verifies first launch and resumed wake with the same ID. The full binary suite passes 216 tests (three ignored). Other providers still need runtime conversation discovery; the in-place restart action remains pending. No provider account or transcript contents were read during acceptance.

Known conversation IDs now feed the native pane menu for managed Claude/OpenClaude terminals and project-scoped native chats. Copy session ID uses the source EN/FR label and eight-character hint, and copies the full identifier only on selection. Unknown IDs produce no menu item. The focused menu test and strict all-target/all-feature Clippy pass. Isolated normal/compact French captures were inspected; these prove the menu rendering, not complete application parity.

Windows terminal restart: a local portable-pty 0.9.0 patch creates the child suspended, attaches a dedicated kill-on-close job before any user instruction, and resumes it only after ownership succeeds. The job forbids breakaway and remains tracked after its initial shell exits. Restart kills the job on a worker and checks both the process handle and active job member count before relaunching. The menu, pending-settings label and fresh Ctrl+Shift+R route support shells and managed Claude/OpenClaude terminal conversations; unsupported providers/platforms are not advertised. A duplicate request cannot start a second stop worker. Stop failure restores the old terminal with an error, and worker disconnection does not launch a replacement. Other platforms still require verified descendant accounting. The full binary suite passes 221 tests (three ignored). Synthetic real-PTY tests cover descendants after parent exit, a legal exit code of 259, an exclusive file lock proving no overlapping old/new CLI, reused conversation IDs, changed launch arguments, duplicate restart requests, stop failure and worker disconnection. Keyboard tests reject held keys and actions behind a blocked workspace. Strict all-target/all-feature Clippy passes. Final isolated French captures were inspected: the pending-settings header button is visible and the compact pane menu shows restart actions and the keyboard shortcut without clipping. These captures validate those controls, not complete application parity. Final ConPTY disposal also runs on the stop worker before the replacement is permitted.

Codex terminal discovery groundwork: src/codex_sessions.rs reads bounded first-line session metadata, matching the source's folder, thirty-day window, five-second slack and excluded-ID policy. Entry/byte limits and unreadable folders return explicit errors, while malformed, missing-time and future metadata cannot claim a pane. Unix paths preserve case. Tests use temporary synthetic rollouts and never inspect personal history. This module is not yet connected to terminal startup or wake; automatic Codex terminal resume remains incomplete.

Codex terminal lifecycle integration: a bounded worker scans at most once every ten seconds until a conversation is claimed; results are checked again against other pane IDs to reject stale claim races. Completed terminals stop scanning after a final empty result. Saved UUIDs are validated and persisted for Codex, then verified by first-record ID and project folder in active or archived rollouts before resume in the selected CODEX_HOME profile. Explicit resume/fork/noninteractive modes remain user-managed. Windows verified restart is enabled only after the ID is known. Discovery still depends on folder/time heuristics; application exit before an ID is discovered cannot yet preserve that conversation. Synthetic PTY validation exercises discovery, resume arguments/current model and refusal under a different profile. No real account or inference turn is involved.
The full binary suite passes 228 tests (three ignored). Profile overrides follow native Windows case-insensitive environment semantics, tested against portable-pty's actual environment builder. Explicit session/mode launches clear the previous automatically tracked ID so the menu cannot advertise an unrelated conversation. The saved-ID round-trip test releases the workspace lock before reopening.

Per-pane model planning integration: Franklin's pure model state, persistence validation, EN/FR labels, twelve-agent action plans, screen readers and Claude switch observer are registered in the native binary. Existing launch fingerprints delegate to the same opaque SHA-256 LaunchSignature type; saved digests retain their exact representation. Raw source signatures are converted only by test fixtures. Runtime custom commands are excluded from model-node serialization, and command/argument/environment/PTY buffers are censored in Debug output. The observer caps its allocation before appending large chunks and rejects success/confirmation after its deadline. Reference extraction is reproducible (197 synthetic vectors, identical SHA-256 on regeneration). These are planning and validation components: no live command is sent by this module and the per-pane picker remains pending UI integration.
The hardened standalone suite passes all 17 tests, including the 197 reference vectors, SHA-256 compatibility, raw-signature rejection, command omission on serialization, bounded observer allocation and late-confirmation refusal.
Integrated validation: 229 binary tests pass with --test-threads=2 (three ignored), the standalone suite passes 17 tests and strict all-target/all-feature Clippy passes. A prior highly concurrent full run reached the Muse flood fixture deadline; the unchanged test passes with the bounded test concurrency.

Build invalidation fix: absent packed-refs was registered as a Cargo watch target, causing repeated rebuilds. Existing refs are now watched directly; an unborn branch watches its existing parent until the ref is created. Consecutive cargo checks completed in 2.73 s and 0.50 s, with no compilation on the second. A standalone build-script probe verified unborn, loose, packed, next-commit and detached-HEAD labels in a synthetic repository, with no absent watch targets. Strict Clippy passes; no application behavior changed.

Codex pre-discovery recovery: local saved panes retain the original search time and an opaque profile/folder scope. Wake performs bounded discovery before any PTY is launched; missing history, scope changes and searches older than thirty days fail explicitly. A new conversation requires the dedicated recovery action or an explicit manual CLI mode. Portable backup import/export strips this local metadata. Synthetic Windows PTY coverage now simulates closure before ID discovery, verifies resume arguments, and checks missing-history and profile-change refusal. No personal history or account is read.
Validation: 230 integrated binary tests pass (three ignored), including real synthetic PTY recovery, workspace schema rejection, profile/time bounds and portable-backup stripping.

Per-pane model picker integration: the terminal menu opens the native picker and stores launch-supported choices transactionally in the saved pane. Clearing restores Settings inheritance. Startup/restart/recovery disables the action; the popover blocks terminal input and handles Escape before pane shortcuts. A failed disk write preserves the old value. The 234-test binary suite passes (three ignored), including model-option validation, default-row effort, per-pane isolation, global-default preservation and manual CLI precedence. Isolated French native captures confirm normal and compact rendering. All 18 standalone picker tests pass, including Unicode models, disabled-parent controls, single-step arrow navigation and focus restoration through held Escape. Strict all-target/all-feature Clippy passes. Live switching, model telemetry/header chips and native-chat picker integration remain incomplete.

Observed-model header integration: terminal-local footer observations now feed the actual header chip for Codex, Cursor, Antigravity and Muse. The latest screen is read independently of user scrollback without moving its viewport. Replacing a terminal clears its observation cache; output, resize and catalogue changes invalidate it. Three new tests pass, bringing the integrated binary suite to 237 passing tests (three ignored). French normal/compact native captures verify chip display and responsive hiding. Provider files, saved lastModel and full refresh orchestration remain pending.

Last-model restoration: saved agent pane metadata now retains validated model/name/effort, restored explicitly as source Saved rather than live telemetry or launch settings. Changed observations save transactionally in one batch; failures leave the prior metadata intact and retry no faster than every five seconds. Old workspaces default to no observation; portable backups preserve validated display metadata. The binary suite passes 239 tests (three ignored), including restart readback, no-op writes, shell/invalid metadata refusal, portable backup and disk failure rollback. Provider readers and full refresh orchestration remain pending.

Model-read worker foundation: a nonblocking per-pane worker rejects results after any scope change, including A-to-B-to-A changes. A process-wide two-worker limit remains held until each job actually exits, even if its pane closes. Settings reads reject documents over 512 KiB; bounded JSONL tails discard partial boundary records and reject invalid UTF-8. Synthetic tests cover stale results, admission limits after pane closure, failure delivery and bounded reads. This module is not yet connected to provider discovery or the header refresh pipeline. Filesystem calls have no hard timeout, and provider/profile/path authority must be checked by the integration layer; these helpers are not a filesystem sandbox.

Verified conversation reads: Codex verification can now return its validated open file handle, avoiding a pathname reopen between metadata verification and model parsing. Its lookup has an aggregate 32 MiB metadata-read budget in addition to the existing entry and per-line bounds. The new reverse log reader follows the source's 512 KiB chunks and 64 MiB scan limit to find a model buried beneath large tool output, joining split records and UTF-8 boundaries. Exhaustion and IO failures remain distinct from a missing model. Synthetic tests cover replacement of a verified pathname, wrong profiles/projects, buried models, every small chunk boundary and unfinished trailing JSON. The complete binary suite passes 246 tests (three ignored). Provider parser integration and incremental file caching are still pending; the reader currently scans afresh and does not reproduce Vue's size-only cache, which was independently shown to retain stale models after a same-size file replacement and reported to Darwin.

Launch-bound model profiles: known Codex, Claude and OpenClaude terminal launches now capture the provider root and working directory from the actual command builder on the launch worker. The private snapshot belongs to the terminal instance and is neither serialized nor exported; later Settings changes do not redirect it. Custom command wrappers retain screen observations but are not assigned an inferred file-reading scope, since their inner environment is unknown. Unit tests cover relative/absolute overrides, provider defaults and immutable launch snapshots; a synthetic real PTY verifies profile capture alongside argument/environment delivery. The binary suite passes 248 tests (three ignored). Provider readers still need to be connected to these snapshots and the header pipeline.

Codex conversation telemetry integration: visible managed local Codex panes now read their verified rollout on a shared bounded background worker, using the profile captured at terminal launch and the known conversation ID. Output debounces for 350 ms and quiet panes refresh every 20 seconds; admission retries are bounded. Scope changes discard pending results and clear prior session metadata, while transient read failures retain the last valid observation. A current Codex footer wins; when it disappears, a fresh conversation model wins over an old cached screen observation. The existing header tooltip and lastModel persistence receive source Session. A temporary source-based Codex parser is isolated in pane_model_runtime pending replacement by Franklin's differential reader. Custom shell wrappers and other provider file/config sources are not inferred. The complete binary suite passes 251 tests (three ignored), including an actual synthetic PTY header precedence test; the final runtime-specific tests and strict all-target/all-feature Clippy pass.

Native acceptance: an isolated French capture with a synthetic Codex rollout displays GPT-5.6 Luna / Moyen from the conversation file, without a matching terminal footer or launching a real provider. No personal profile or transcript was used.

Remote-host repository separation: the three local host crates have moved to the private tessel-host repository. The desktop manifest and lockfile pin tessel-host-protocol to 35f41cd68849e60d42bca14b3449222fad1e7ace; Cargo metadata reports only the desktop workspace member, and no registry dependency versions changed. Local Git-CLI fetching, cargo check and strict all-target/all-feature Clippy pass. The plan file now links the host's authoritative plan/specification and describes access prerequisites. The Windows CI workflow accepts a dedicated read-only deploy-key secret, verifies GitHub SSH host keys, cleans up the key and excludes private Git checkouts from its cache. YAML and all PowerShell steps parse successfully. Hosted CI acceptance is recorded separately from local checks; the dedicated read-only deploy key was configured externally after the initial missing-secret failure. The host protocol dependency is not yet wired into desktop SSH behavior.

TypeScript fixture migration: model-catalog-fixture, model-listed-fixture and model-options-fixture now run as native Node 24 .cts scripts with strict TypeScript checking, typed input rows/cases and unknown-typed VM outputs. The development-only package/lockfile pins the compiler and Node definitions; no JavaScript is emitted and Cargo/application startup does not depend on Node. A clean npm ci with lifecycle scripts disabled and npm run check:scripts pass. Isolated runs against the same reference source produced byte-identical JSON to the former .cjs generators: 12 catalogues, 343 list cases and 2,560 option cases. The other eight existing generators remain assigned for conversion. CI now checks migrated scripts separately and uses two Rust test threads consistently with local validation.

Windows CI path regression: model discovery accepts the tilde used by DOS short-name aliases instead of silently falling back to the default CLI. Its real synthetic listing fixture always contains a tilde and spaces. The command-lookup test compares canonical file paths because where.exe expands short aliases. Both prior failures were reproduced with an isolated TEMP directory and GetShortPathName; the discovery failure is also present in the Vue validator and was reported to Darwin. No running Vue files were changed.
Validation: all six command-lookup/discovery tests pass with a real DOS short TEMP alias, and strict all-target/all-feature Clippy passes. This fixes the reproduced CI path causes; hosted acceptance remains to be checked.

Claude/OpenClaude model lookup: src/claude_model_file.rs opens a transcript only inside the captured profile, checks a complete top-level user/assistant record for the requested UUID and working directory, and retains the same file handle. Sidechains, foreign folders and incomplete final records cannot establish identity. Enumeration is capped at 4096 entries, identity records at 256 KiB and aggregate reads at 8 MiB. This does not change the existing resume-presence contract. The pane runtime now reads a bounded 256 KiB tail through the verified handle and uses the shared Claude parser. Codex uses the same shared parser module with its existing reverse log scan. Other provider runtime inputs and full header orchestration remain incomplete.
Integrated binary validation: 257 tests pass (three synthetic/opt-in tests ignored), including the four identity-reader tests, Claude/OpenClaude model extraction and native terminal header behavior. The regenerated source fixture is byte-identical and strict TypeScript checking passes.
Hosted b134 now passes command lookup; discovery reached the outer 30-second test deadline before its production phases (20-second resolution plus 60-second query) could finish. The test now allows their combined budget plus cache IO (90 seconds) and reports pending/saving/error state on expiry; runtime timeouts and result assertions are unchanged.
Final integration checks: all 17 differential/adversarial reader tests pass; strict all-target/all-feature Clippy and cargo fmt --check pass. Shared parsers are registered in the binary; only Codex and Claude-compatible verified-file reads are wired into this runtime change.
Claude settings enrichment uses the shared pure resolver after verified transcript extraction. Three bounded settings documents (local, project, captured profile) remain worker-local; per-model effort takes precedence over general effort, explicit transcript effort is retained and a matching configured 1M family enriches the model label. Missing files are skipped; IO/size failures preserve prior telemetry through the existing worker failure path. Command/settings-only fallback without a verified conversation and complete header orchestration remain pending.
Fixture TypeScript conversion completed: all eight remaining CommonJS generators are replaced by strict .cts versions (shim programs, resume arguments, Antigravity, Cursor, launch fingerprints, Muse catalogues, pane session models and regional dates). Each regenerated output was compared byte-for-byte to its committed fixture blob in an isolated target directory; all eight match. No tracked fixture or reference application was modified. npm run check:scripts passes for the complete migrated set.

Native-chat options: model/effort independent pane-to-global fallback is wired for Codex and Claude, including resume. Native choices use the optional native_options field, ignored by older SavedSession readers, while session_options remains terminal-only. Native session metadata accepts only the two validated option fields; no launch permissions are imported. Claude CLI argument tests preserve safe-mode/default approval/empty settings sources. Ordered reader objects preserve JavaScript insertion and duplicate-key semantics without global serde feature changes.

Validation before splitting the batches: 262 binary tests and 18 reader integration tests passed (three existing synthetic child tests ignored), with strict Clippy/TypeScript and byte-identical reader fixtures. After native-chat review fixes, 264 binary tests passed; the extra Claude invalid-draft/inherited-choice test and 15 focused native tests also passed on the final effort-level changes. b135 CI built/uploaded its portable artifact successfully but failed deleting its temporary dependency key because owner-only read permission excluded DELETE. The workflow now grants only the runner owner full file control; a synthetic local ACL/deletion check passed. Hosted end-to-end validation of that cleanup fix remains pending.

Native-chat review fixes: save only on field validation/selection/Connect/Send, not on each valid prefix; use the shared safe model token validator and provider-specific efforts; trim model input and keep Codex steer independent of model drafts. Independent model/effort fallback deliberately follows Vue chatOpen. New focused regressions cover these boundaries.

The installed Codex app-server schema describes reasoning effort as a nonempty advertised string. The embedded source catalogue includes Max and Ultra, so those remain available for Codex; the obsolete fixed-effort assumption from review was rejected using that local evidence. No real model turn was used for schema or native-option validation.
`cargo fmt --check` and all-target/all-feature Clippy with warnings denied passed on the final native-chat batch.

Native chat pane menus now use the model/effort catalogue with independent Settings fallback and transactional override saves. Codex applies choices on the next turn, Claude on the next connection. Protocol-observed active model chips and live Claude switching remain pending.

Password dialog: the pure metadata/import/DOM-decision model and source-derived synthetic fixtures are present, including streaming CSV column limits, bounded batches and duplicate IDs, independent HTTPS proof construction and Live Share decisions. The egui panel is connected to a revisioned worker for explicit CSV imports, removal and owner-only isolated-world fill. Metadata snapshots contain no passwords; transient fill payloads are dropped immediately after submission to the private browser bridge. Save failures retain the existing records and revision. Closing password settings drops the worker and cancels pending fill checks. Direct browser import and persistent agent-consent controls remain pending and unavailable in the runtime panel.


## Native host gate 7A integration

The desktop now pins the protocol, client and installer to host commit `8293994`.
The explicit Settings > SSH preview action installs or reuses the embedded v0.1.0
release, checks its remote digest/version and opens one managed connection per
validated alias. No connection or installation runs automatically on startup.
Workers and captured command output are bounded. Cancellation covers upload and
initial/reconnecting handshakes; cleanup has a separate short deadline. Late
results are closed, and process reaping stays off the UI thread. Windows bridge
processes use kill-on-close jobs; Unix bridge processes use their own process group.
Errors use fixed messages instead of forwarding remote output.

This is connection groundwork, not daemon-backed terminal/editor/tree/Git parity.
Those adapters remain pending. Existing SSH/SFTP sessions retain their backend.
Real user-server acceptance has not been performed. Local integration validation: 12 installer tests and 8 manager/transport tests pass
(one synthetic child entry point is intentionally ignored outside its parent tests).
The transport tests execute real harmless child processes, including a descendant
that holds stdout open, stalled stdin, output flooding, cancellation and early
nonzero exit. Strict all-target/all-feature Clippy with warnings denied and formatting pass.
Native French screenshots pass at normal and compact window sizes using an empty
isolated profile and the synthetic `--native-host-fixture`; no SSH connection is
started by that fixture. Screenshots were inspected for wrapping and visible controls.

Password panel A2 acceptance (Windows): 8 vault/worker/controller tests, 8 headless password-panel UI tests and 7 Settings-navigation tests pass. Coverage includes save-failure rollback, stale revisions, exact-origin fill tickets, metadata-only snapshots, profile identity, pagination, disabled capabilities, native-popup/held-Escape routing and focus isolation. Strict all-target/all-feature Clippy passes. Native synthetic French captures were inspected at normal and compact sizes and with the full CSV export guide. Only fake accounts in a temporary vault were used; direct browser-database import and agent vault grants remain unavailable.

Gate 7B foundation validation (2026-11-04): the v0.1.1 host bundle and client pin `3c459af` pass 12 installer fixture tests, including damaged artifacts, wrong versions, both architectures and cleanup on failures. Twelve Windows lease-store tests pass with synthetic secrets only, including retained corruption, file locking, capacity and failed atomic replacement preserving the old encrypted file. Six ordered command-queue tests, four snapshot/history tests and four worker behavior tests pass (38 distinct focused cases). Strict Clippy across all targets/features and formatting pass. Remote PTY panes are not enabled yet: opening/recovery, lease persistence callbacks and the UI/emulator adapter remain to be integrated.


Gate 7B adapter preparation (2026-11-04): saved native-host markers now block the legacy SSH startup path and remain inert after restore. Portable export/import removes resumable terminal identity and never includes lease authority. A worker permit is reserved before a future create/attach RPC; cancellation releases it. A non-serializable monotonic create-attempt window includes the request timeout and rejects changed epochs, expired attempts and unbounded requests. Incremental screen application preserves the old screen until a complete replacement is decoded, publishes sequence completion only after emulation, and never routes snapshot bytes through the live DSR reply path. The pure terminal panel now preserves a close confirmation on its initial epoch binding and cannot widen a compact pane header. Tests: 26 filtered binary cases pass (2 explicit fixture/runtime cases ignored), 7 screen/history cases, 13 queue/worker/create-window cases and 7 UI suites pass. Native remote panes remain disabled pending opening, lease callbacks and the actual emulator/UI adapter.

Gate 7B opening and event integration (2026-11-04): production bindings now compile with client/protocol revision `23b62ac`, independently of the unchanged 0.1.1 daemon bundle. Pending create intent is encrypted before transmission; same-run retries retain their key on disconnection and capacity exhaustion. Creation authority is retained even if its event stream immediately rejects a snapshot. Explicit cancellation, pane drop and adoption timeout have distinct cleanup behavior. Failed attachment transport is not reported as a vanished terminal. Event application is bounded per frame and preserves the last screen on rejection or permanent stream closure. Native remote panes remain disabled pending lease callbacks and actual workspace/emulator wiring; synthetic tests do not establish real-server acceptance.
Validation of this lot: 12 event/application/history cases, 39 opening/handoff/worker cases, 16 command/worker/window cases and 8 panel suites pass (shared helper tests overlap across targets). Strict all-target/all-feature Clippy passes. The expanded Windows encrypted-store suite passed 18 cases on the preceding compatible client pin.

Gate 7B lease keeper and resynchronization follow-up: synthetic tests pass for 41 keeper cases, 20 worker cases, 43 opening cases, 17 event/emulator/history cases and 2 emulator cases (shared helper tests overlap). This covers same-terminal takeover serialization, a granted lease arriving after pane closure, retained failure retries, no resurrection by stale saves, queue reservations surviving keeper shutdown, snapshot-awaiting input gates, repaint budgets and cursor-query reset across snapshot replacement. Workspace routing, pending-create promotion, global report handling and native rendering acceptance remain unfinished. Strict Clippy passes for the production binary and these five integration targets.

Gate 7B follow-up validation: 47 opening cases, 48 keeper cases, 20 worker cases,
19 event cases, 3 emulator cases and 10 panel suites pass (shared helpers overlap).
Pending-create promotion now retains its original intent across failed writes and
explicit retries. Cancellation after UI adoption uses the same off-thread cleanup
rules as cancellation during opening. Opening failures and unsaved-access warnings
have distinct English/French UI states without granting input or termination authority.
The native facade compiles, but workspace activation remains pending: app-owned
retention of unsaved access after pane closure, global persistence reports and actual
native-pane rendering acceptance are not yet complete. No real user host was used.
The expanded store suite passes 17 cases, and 15 filtered local-terminal/profile
cases pass, including real synthetic PTY execution and descendant cleanup.
Created terminals start from their requested dimensions and sequence zero, so a
silent program can receive input; existing attachments still await their snapshot.

The application-level lease desk is implemented and passes 30 scoped tests
(12 desk cases plus shared helpers). Failed-save tokens survive pane closure and
are shared by panes displaying the same terminal. End-recording work is bounded,
and a blocked new slot does not stall an already-tracked slot. It is not yet wired
into the app's shutdown/opening lifecycle. Two native-facade tests use the actual
client protocol and command worker over in-process pipes: silent-program input,
modal/close-confirmation input isolation, detach without termination, and retaining
the last screen while refusing input and resize after stream rejection all pass.
These fixtures never open the owner store or connect to SSH.

Managed opening now reserves persistence capacity before creating a remote
process and retains a desk subscription through worker-to-pane handoff. A failed
save remains recoverable after that pane closes. Creation grants have lower
priority than committed control changes or confirmed termination, including when
responses arrive out of order. A superseded creation never implies a successful
or failed save; pending intent remains in the encrypted store for reconciliation.
Validation before the shutdown extension: opening 54, keeper 63 and desk 34
scoped tests pass (shared helpers overlap). Native workspace activation still
requires application-level shutdown, action dispatch and explicit opening/list
routing; these internal tests do not establish real-host or complete UI parity.

The runtime now closes new RPC admissions only after explicit shutdown
confirmation, counts opening workers through their last cleanup, and drains the
last persistence reports after observing idle. Deferred cleanup admitted during
that final drain requires another frame. Its application dialog is not wired yet.
Final validation: opening 58, keeper 64, desk/runtime 37, terminal panel 11 and
existing-terminal picker 6 suites pass. The filtered binary suite passes 51
terminal/profile tests, including six native UI/protocol fixtures and the local
modal-input regression; the inert native-workspace guard also passes. Strict
Clippy passes for the binary and these five integration targets with all features.
Formatting checks pass for the changed Rust sources. Native overlays retain pane
clipping, stay below global dialogs, release IME/mouse state on input loss and
preserve Escape for disabled panes. Local save states distinguish pending, failed
and superseded access without inventing a saved result or a retry authority.

Gate 7B workspace routing (2026-11-04, acceptance in progress): Windows settings
now exposes explicit new-native-terminal and existing-terminal actions. The
application owns the opening count, lease desk and keeper until shutdown; pane
closure detaches, whereas cancellation of a not-yet-adopted creation follows the
existing explicit cancellation protocol. Only after the exit confirmation does
shutdown close admissions and drop native pane handles. Pending openings and
recovery writes continue, failed writes can be retried, and abandoning the wait
requires Quit anyway. Keep working remains available before this irreversible
phase. An empty, settled runtime does not introduce a redundant exit dialog.

The existing-terminal worker is bounded across lists, reads recovery metadata in
one encrypted transaction, validates the host epoch before and after the local
read, and discards unread results when a newer refresh starts. Unknown local
access is not represented as missing access: observation remains available,
saved-lease resume does not. Pending creations are informational and capped.
Saved pane identities contain only epoch and terminal id; a restored pane wakes
only by explicit attachment. A creation completed during app shutdown may have
no saved pane identity and is found through the existing-terminal picker after
its recovery access has been saved. Detached panes likewise remain discoverable
there. Both the initial new-terminal request and emulator start at 30x100.

Validation so far: 12 list-worker tests, 18 encrypted-store tests, six native
emulator/client UI boundary tests, and strict scoped all-feature Clippy pass.
A broader native binary filter passed 32 cases (two dedicated fixture/reference
cases ignored). All seven picker suites now pass, including real wheel scrolling through the
280x320 viewport and clipped-text visibility. Review fixes disable underlying
settings and toolbar controls, replace a restarted native pane in place, reuse
matching saved identities and restore project/maximized-pane focus. This does not establish real-server or full
desktop visual acceptance. Retained pre-resynchronization history rendering still needs integration.


Host-info integration follow-up: persistence metadata is read once per observed
live daemon epoch (and again after an observed disconnection), validated, and
shared by every pane of that connection. A separate metadata worker keeps the
original connection worker available to close the client and interrupt an
in-flight request immediately. Both the metadata worker pool and connection
manager are bounded at eight; the parent retains its connection permit until its
metadata child has actually ended. UI owners only invalidate the cache and send
stop/close signals. They never join a worker or issue a request. Closed caches
cannot publish late replies, and a report for another epoch is never displayed.
Yes suppresses the warning; No and Unknown show distinct policy messages. No
longer infers that user lingering is disabled. Resync flags now reach the actual
pane's dismissible output notice without claiming a missing-byte count.

Six metadata tests pass, including real in-process client pipes, pool saturation
and cancellation of silent requests well before their 30-second deadlines.
Seven native pane/client rendering and input tests and eleven pure panel suites
pass, including policy notices, resynchronization notices, modal isolation and
mouse-button release. Strict scoped all-feature Clippy passes.

Epoch cleanup follow-up: an adopted native pane observes its own live Client's
confirmed epoch even after its event stream was rejected or closed. It drops
input authority and hands one end proof to the lease desk, without sending a
close RPC. Mere disconnection retains the saved access. A connector's installed
version command is immutable; opening an old slot through a new Client refuses
before adoption and preserves its saved lease. Bulk retain_epoch is unused in
production and must not be called on mere discovery of a different epoch.
SSH aliases repointed to another physical host remain an identity limitation;
version pinning alone cannot detect that change.

Eight native pane/client tests, the old-slot opening regression and strict
scoped all-feature Clippy pass. Real-server acceptance remains pending.

Retained-history integration: Native owns a ScrollState anchored to monotonic
archived-row identities. Feed polling reconciles that anchor before rendering;
selection is preserved only while its visible cells remain stable and is cleared
on eviction, resize or a changing archive/live seam. Rendering and copy use a
borrowed composite Viewport, including soft wraps, wide characters and clipping.
Old cells never pass back through the emulator. Archived views keep focus and
copying while blocking keyboard/paste/IME and mouse reports, including a wheel
and text event in one frame. The active parser still answers live cursor queries.
Ordinary parser scrollback retains typing-to-live behavior.

The new deep-history test exposed vt100 0.15.2's subtraction overflow when a
scrollback offset exceeds the screen height. A licensed local patch bounds the
history iterator and saturates the live-row count; its direct regression checks
both row count and copied text at deep, shallow and live offsets. No global
registry files are modified. Real-server and full desktop visual acceptance
remain pending.

Validation: all 58 binary terminal-related tests pass, including 11 native
UI/client cases, real local PTY execution, descendant shutdown and model-footer
isolation. The history target passes seven tests and the composite viewport target
passes nineteen (shared helper tests overlap). A four-case regression covers
multiple snapshots in one poll with truncation and intervening live-output
eviction. Capture metadata is bounded and records the actual visible source row,
so aggregate capture counts cannot misidentify an evicted selection. Strict
Clippy across all targets/features and formatting pass.

An alternate-screen resync now archives the primary shell history before the
alternate cells, with a hard wrap boundary and the same global storage limits.
The old parser switches grids through a narrowly scoped vendored API before it
is discarded; no synthetic escape is injected into a potentially unfinished
OSC/DCS. Tests cover this control-string boundary, bounded large alternate
screens, grid/RIS changes invalidating selection, and truncation clamping to the
current capture even when an older narrow capture remains retained.

Floating terminal integration: the drawer is separate from saved
project sessions and pane grids. Its runtime starts and stops shells off the UI
thread, preserves a hidden shell, and refuses replacement until shutdown is
confirmed. The application waits for its close before continuing its existing
native-host shutdown drain. The toolbar, command palette and physical
Ctrl+backquote toggle share the same actions. A stable terminal widget ID lets
the panel restore focus without a first-frame provisional-ID race.

The saved open state currently restores the drawer with an explicit Open terminal
action, not the former process: local daemon-backed reattachment remains pending.
The remote-files context launches SSH against the selected host; ordinary project
contexts launch the default local shell. Shell selection parity remains pending.
Native WebView surfaces are hidden while the drawer is open until child-window
clipping is implemented. These limitations prevent claiming full Vue parity.
The drawer currently opens/closes immediately; Vue's 0.16-second slide transition
is not implemented yet.
egui's first-ever opening also has one invisible, disabled sizing frame; tests
assert that it accepts no terminal input. Reopenings and return from dialogs are
checked in one frame, without a settling helper.
Validation: 31 lifecycle tests, 7 UI suites and 2 application/workspace tests
pass, including a real Windows shell, cancellation of a pending application
quit, and immediate handle-key navigation. Strict all-feature Clippy passes for
the binary and both new integration suites; formatting and diff checks pass.
The isolated native fixture renders the drawer, terminal and resize handle
without changing the running Vue application or its profiles.
Opening is Windows-only until Unix process-group shutdown can be verified; the
generic controller itself remains portable. This avoids offering a shell that
the application cannot subsequently close through its verified shutdown path.

WebView clipping follow-up: the pinned wry 0.57.1 Windows extension exposes the
child container HWND through `WebViewExtWindows::hwnd()`, and this application
uses `build_as_child`. A window region can therefore exclude the drawer's
physical-pixel overlap without changing the page viewport, scrolling, cookies
or profile. Validate region ownership on SetWindowRgn success/failure, rounding
at non-integer DPI, restoration on hide/resize, keyboard focus and pointer
hit-testing against a synthetic isolated browser profile before enabling this.
The [Win32 contract](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowrgn)
transfers region ownership to Windows only on success; coordinates are relative
to the window corner, and a null region restores the ordinary window shape.

Toolbar Layout menu and Broadcast multi-write integration: the toolbar right
cluster now exposes Broadcast (`Ctrl+Shift+B`), the 290-pixel Layout menu, the
floating terminal toggle, both sidebar toggles and Settings in source order.
Layout exposes Split right (`Ctrl+Shift+E`), Split down (`Ctrl+Shift+O`),
conditional Arrange panes in a grid (`> 1` pane), `1 x 2..3 x 3` Even grid
presets that retain open panes in leaf order and spawn default shells only for
empty slots without letting workspace layout sync overwrite the preset split
tree, and Close active pane (`Ctrl+Shift+W`). Broadcast persists workspace and
per-pane `write` / `écrire` membership across restarts and portable backups,
renders the 24-pixel workspace banner below the toolbar, adapts bracketed paste
and application cursor mode per target terminal, excludes device/cursor/focus/OSC
replies and mouse reports, and updates the status bar's active target count.
Validation: 7 UI/AccessKit/7-theme EN/FR suites and targeted workspace, layout,
status-bar, toolbar and terminal broadcast unit tests pass, with strict
all-target/all-feature Clippy and formatting clean.

WebView2 floating-drawer clipping integration: `browser_clip` computes physical
`SetWindowRgn` cutouts with outward fractional-DPI rounding, 27-bit GDI region
coordinate bounds, cached `(HWND, Option<Cutout>)` reuse, RAII `HRGN` cleanup,
and fail-closed cache invalidation on error. Both owner and project-agent
`Browser` instances defer visibility to end-of-frame `sync_presentation` after
`FloatingTerminal`, popups (`egui::Popup::is_any_open`), command palette and
modals are evaluated, hiding fail-closed on any popup/modal or clipping error
and coordinating Win32 parent/child focus (`focus_parent` vs webpage clicks).
Validation: 6 `browser_clip` tests pass, including 1px region boundaries,
full-window `NULLREGION` occlusion, cache-hit proof, 200-iteration GDI handle
leak check, and isolated real WebView2 child-container `ChildWindowFromPointEx`
hit-testing, with strict all-target/all-feature Clippy and formatting clean.

Complete Status Bar integration: `status_metrics` and `status_strip` implement
the full `StatusBar.vue` / `PortRow.vue` / `RemoteHostsStatus.vue` contract on
Windows without spawning external telemetry subprocesses. `KeepAwakeController`
applies `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)` for `On`
and active `Agents` modes and resets on `Drop`. A single bounded background
worker samples Win32 Toolhelp32 + `K32GetProcessMemoryInfo` + `GetProcessTimes`
process trees (claiming PTY session subtrees before `tessel-rust.exe` to prevent
double-counting, with creation-time PID-reuse guards and 60-sample workspace/app
sparklines) and `iphlpapi` `GetExtendedTcpTable` IPv4/IPv6 TCP listeners (32-hop
ancestor walk + workspace path-boundary attribution, dual-stack deduplication,
embedded-Browser navigation, address copy, and re-scanned creation-time-verified
`TerminateProcess` stop). Popovers (`256px` Keep Awake, `416px` Resource
Manager, `384px` Ports, `288px` Remote Hosts, `204px` right-click visibility
menu) block underlying central/floating/WebView2 surfaces while open, and
`StatusBarPreferences` persist across restarts and portable backups.

Keyboard Shortcuts, Command Palette, `F1` Help and Pane Menus integration:
37 shortcuts across all five reference groups (`Panes`, `Terminal`, `Workspaces`,
`Editor`, `App`) are wired end-to-end and exposed in `HelpPanel` with localized
EN/FR keycaps and contextual footer hints. Terminal panes add inline scrollback
+ screen search (`Ctrl+Shift+F`), `Ctrl+Shift+V` Win32 Unicode paste, selection
copy on `Ctrl+C` (falling back to `SIGINT`) and `Ctrl+Shift+C`, `Shift+PageUp` /
`Shift+PageDown`, agent `Shift+Enter` newline injection (`\x1b\r` for Claude /
Codex on Windows, `\n` for other agents), app-shortcut withholding, plain
right-click copy/paste vs `Shift+Right-Click` full context menu, Windows
`Win+H` voice typing (`SendInput`), and dropped-file / `Finder` `Ctrl+Enter`
shell-quoted path insertion. Editor tabs add inline Find (`Ctrl+F`), Replace
(`Ctrl+H`), Go to line (`Ctrl+G`), Word wrap (`Alt+Z`), changed-hunk navigation
(`F7` / `Shift+F7`) and `Alt+Left/Right` tab switching; `Browser` adds `Ctrl+L`
and `Ctrl+R` / `F5`; native chats add `Alt+A` pending-request focus. Command
Palette adds full `New`, `Project`, `Layout`, `Files`, `Task`, `Theme` and
`Tessel` catalogs plus localized `"Nothing matches \"{{query}}\""` empty states,
and `pane_header` renders contextual status facts and `SendSelectionTo` targets.

Live Share parity integration: `share` and `share_panel` implement the full
host/guest experience over duplex `rustls` TLS 1.3 with invitation-pinned
SHA-256 certificate verification and deterministic 6-digit SAS codes (`"123 456"`).
Host sessions manage up to 20 revocable/rotatable invitations (`MAX_INVITES`),
up to 64 trusted peers (`MAX_TRUSTED`), 8-flag `ShareRights` with 4 presets and
fail-closed unknown-key stripping, `ShareSettings` (`hideSecrets`, `confirmClose`),
project-confined file listing/read/write, task-board RPCs, terminal view/input
forwarding and per-pane guest presence (`watching` / `typing`). UI surfaces match
the `720px` `LiveShareDialog` (`Share`/`Join`, `<LsStatus>` with inline `"Stop
Live Share?"` confirmation, `Overview`/`People`/`Invitations`/`Settings`, 8-tile
`RightsGrid` with `"Runs commands as you"` warning, `RightsChip`), the
non-dismissible `560px` `LiveShareAsk` knock/action modal (`900ms` arming delay,
`Escape` blocking, `"Trust next time"` gated on SAS verification), the `24px`
toolbar `LiveSharePill` + `360px` popover, pane header presence badges, and
`.osb-live` sidebar indicators. Validation: 9 `ui_live_share` TLS + headless UI
tests, `ui_project_panel`, `i18n::tests`, `files::tests` and strict Clippy pass.

Workspace panels parity integration: left sidebar adds a collapsed `52px`
`.osb-rail` when hidden (`ExpandSidebar`, project-initial tiles, `OpenFolder`),
`Workspace options` (`Sort by activity`, `Compact rows`, `Hide sleeping`
persisted in `SidebarPreferences`), `New terminal`, Git branch pills, live
`AgentDotState` indicators, `Alt+Up/Down` project reordering, and project /
session context menus. Right panel defaults to `420px` with a changed-file badge
on `Changes` and a fullscreen expand/restore toggle (`Escape` restores).
`ExplorerTree` adds filename filtering, bounded background content search,
`Show dotfiles`, `Compact folders`, `Collapse all`, Git status badges (`M`, `A`,
`D`, `U`, `R`, `?`), single-click italic preview vs double-click pinned open,
`Copy path`, and confined Windows `SHFileOperationW` `Move to Recycle Bin`
(`FOF_ALLOWUNDO`). `ScmChangesPanel` enables all eight `ScmCapabilities`
(`OpenDiff` unified syntax-colored diff tabs, `scoped_numstat` `+added /
-removed` badges, modal-confirmed tracked `git checkout` + untracked Recycle Bin
discard, inline `Commit` clearing draft on success, and `Fetch` / `Pull` /
`Push` / `Sync` / `Publish` remote operations). `TasksBoard` adds inline card
rename, card delete and `Done` bulk selection (`Select` / `All` / `None` /
`Delete (N)`). `view_tabs` adds closable `Browser`, `Diff` tabs, file-extension
icon tints, italic preview tabs (`Keep open`), deleted-file strikethrough, and
`Close others` / `Close all` / `Copy path`. `EmptyState` adds `No project
folder` fallback, `PowerShell` / `CMD` / `WSL` shell pills and `Remember for new
projects` (`LauncherPreferences`).

Editor syntax highlighting, synchronized line gutter and confirmed missing-agent
installation: `syntax.rs` provides zero-dependency lexical highlighting across
all seven themes plus Classic light mode (12 language families, `512 KiB`
fast-fallback guard, `SyntaxCache` galley memoization) and `paint_editor_gutter`
(1-based logical line numbers skipping wrapped sub-rows + `2.5px` modified-line
accent bars), persisted via `EditorPreferences`. `agent_installs` + `agents_panel`
+ `app` implement `plan_agent_install`, `install_chain` (`TESSEL-INSTALL-OK` /
`TESSEL-INSTALL-FAILED` split markers), `Action::Install`, the modal confirmation
dialog (`show_install_confirm_modal` blocking underlying Settings/WebView2),
non-broadcast terminal execution (`Terminal::send_installer_line`), automatic
post-install `detect_agents` refresh, and the launcher footer jump to Settings >
Agents.

Remaining Settings pages integration (`settings_pages_panel.rs` + `workspace.rs`
+ `terminal.rs` + `backup.rs` + `app.rs`): Settings navigation now populates all
five groups (`AI`, `Workspace`, `Workflows`, `Insights`, `About`) with `General`,
`Terminal`, `Editor`, `Browser`, `Git`, `Quick commands`, `Stats` and `Updates`
alongside `Agents`, `Appearance`, `SSH hosts`, `Passwords` and `Account`.
`GeneralPreferences` wires session-restore opt-out, running-agent close modal,
workspace-removal modal, running-session quit confirmation, default worktree
directory, backup export/import, diagnostics copy and preference reset.
`TerminalPreferences` wires scrollback (`500..=100_000`), line height
(`1.0..=3.0`), `Block`/`Bar`/`Underline` cursor style, `530ms` cursor blink,
automatic pane tiling, copy-on-select, right-click paste vs context menu, inline
multi-line paste confirmation and focus-follows-mouse across workspace and
floating terminals. `EditorPreferences` wires idle auto-save, reusable preview
tabs, two-way Explorer dotfiles/compact-folder sync and Git diff word wrap.
`BrowserPreferences` wires built-in vs system link opening and isolated
project-agent browser gating. `GitPreferences` wires branch-prefix modes/preview
and two-way `ScmChangesPanel` group-order/view-mode sync. `QuickCommand`
snippets run in active or newly spawned shells from Settings or Command Palette
and are stripped from portable backups (`backup::export`/`import`). `Stats`
surfaces live telemetry and `Updates` opens embedded `LICENSE` /
`THIRD_PARTY_NOTICES.md` tabs offline.

Git Worktrees parity integration (`worktree.rs` + `worktree_state.rs` +
`worktree_cleanup_panel.rs` + `launcher_panel.rs` + `project_panel.rs` +
`workspace.rs` + `app.rs`): bounded read-only background discovery (`MAX_CONCURRENT_INSPECTS = 2`,
`core.fsmonitor=false`, disabled `hooksPath`) inspects all workspace repositories
and secondary worktrees without index contention. `create_worktree` resolves
`GeneralPreferences::workspace_dir` (`~/.tessel/worktrees/<project>/<slug>` by
default, rejecting `..` and symlink/reparse bases) and `GitPreferences` branch
prefixes, runs `git -c core.longpaths=true worktree add -b`, and copies bounded
git-ignored `.env` / `.env.*` files while skipping template suffixes and links.
`remove_worktree` refuses primary/ancestor/home/root targets, closes open panes
in matching worktree paths first, unlinks directory symlinks and Windows NTFS
reparse-point junctions (`FILE_ATTRIBUTE_REPARSE_POINT`) via `fs::remove_dir`
before `git worktree remove` so external junction targets survive intact, retries
transient Windows locks, prunes metadata and deletes safe/forced local branches.
UI surfaces integrate the `LauncherPanel` `"Separate copy on its own branch"`
toggle (for shells, agent CLIs and native Codex/Claude chats), `ProjectPanel`
secondary worktree sub-cards, context menus and hidden-worktree discovery inbox
(`ProjectWorktreePrefs`), pane header `MenuState::worktree`, Command Palette
`worktree:new` / `worktree:cleanup` actions, and the localized EN/FR
`WorktreeCleanupModal` (`Safe` / `Risk` / `Blocker`).

Complete 22/22 Settings pages + Custom CLI Agents integration
(`settings_extra_model.rs` + `settings_extra_panel.rs` + `automations.rs` +
`workspace.rs` + `backup.rs` + `agent_preferences.rs` + `launch.rs` + `app.rs`):
Settings navigation and Command Palette now expose all 22 `SettingsPageId` pages
across the 5 groups (`Accounts`, `Skills`, `Mcp`, `Connections`, `Context`,
`Orchestration`, `Alerts`, `Voice`, `Devtools`, `Automations` added alongside the
existing 12 pages + Windows `Passwords`). Managed `claude-accounts` /
`codex-accounts` rosters inject `CLAUDE_CONFIG_DIR` / `CODEX_HOME` per launch;
provider secrets seal via Windows DPAPI (`provider-secrets.dat`); `scan_skills`
and `install_skill_from_folder` enforce bounded symlink-free skill management;
`upsert_json_mcp_server` / `remove_json_mcp_server` create `.tessel-bak` backups
and `probe_mcp_stdio` runs bounded JSON-RPC `initialize` probes; `scan_project_context`
estimates model-family token budgets while excluding `.env*`/keys; `AutomationsFile` enforces `>= 15m` cron validation, 64-bit FNV-1a `confirmed_sig`
gating, concurrency caps and file-backed `prompt.md` execution; and up to 30
`CustomAgent` entries integrate across `Agents`, `Launcher`, `CommandPalette`,
`launch.rs`, and portable backup stripping.

All 7 `SidePanel` tabs, `NotesPanel`, `ScmHistoryPanel`, and `ReviewPanel`
integration (`side_panels_model.rs` + `side_panels_ui.rs` + `icons.rs` +
`workspace.rs` + `app.rs` + `tests/ui_side_panels_and_review.rs`):
`SideTab` expands cleanly to `Dashboard`, `Files`, `Changes`, `Tasks`,
`TaskHistory` (`#[serde(alias = "taskHistory")]`), `History`, and
`Conversations`, preserving `#[default] Files` and unknown-variant rejection.
UI-agnostic `side_panels_model` provides `build_dashboard` (5-bucket status or
project grouping + `sleepable_pane_ids`), `TaskHistoryStore` + formula-safe
UTF-8 BOM `history_csv`, bounded `scan_past_sessions` + opt-in secret-masked
transcript search, `ActivityStore` thread builder with media chips, `.tessel/notes.md`
`NotesModel` with disk `mtime` conflict resolution, 5-color topological Git
swimlane graph (`build_git_history_lanes`), and `load_branch_review` /
`merge_review_branch` with `expect_head_oid` safety checks.

`AddProjectDialog`, `GitHubDialog`, sidebar groups/sleep, `UsageMenu` and
`NotificationsMenu` integration (`dialogs_toolbar_model.rs` +
`dialogs_toolbar_ui.rs` + `project_panel.rs` + `chrome.rs` + `workspace.rs` +
`app.rs` + `tests/ui_dialogs_usage_notifications.rs`):
`AddProjectDialog` (`512px`) unifies local folder selection (with bounded BFS
nested Git repository discovery), option-injection-safe `git clone`, `git init`
new-project creation, and SSH `RemoteFolderBrowser` browsing. `GitHubDialog`
(`760x540px`) queries `gh` PRs/Issues, rolls up CI checks (`Passing`, `Pending`,
`Failing`), launches branch worktrees, and formats `Fix failing checks` agent
prompts. `ProjectPanel` adds collapsible workspace folder groups, `Put to sleep`
session/project actions, and `Hide agent task line`. Toolbar adds non-focus-stealing
`Agent request` pills, provider `UsageMenu` (`<60%` / `>=60%` / `>=80%`), and
coalesced `NotificationsMenu` (`unread_count`, `mark_all_read`, `clear`).

Tessel Teams, worker orchestration, inter-project relay and safe idle wake-ups
integration (`teams_relay.rs` + `teams_ui.rs` + `agent_tools.rs` +
`pane_header.rs` + `project_panel.rs` + `sidebar_model.rs` + `workspace.rs` +
`app.rs` + `tests/ui_teams_orchestration_relay.rs`):
`TeamsRelayHub` implements persistent project teams (`SavedTeam`, `team_number`,
`dedupe_team_numbers`), per-pane 32-byte HMAC-SHA256 request signing and
constant-time verification (`TeamAuthRegistry` with `MAX_NONCES = 20_000`),
`team_create`/`team_bootstrap`/`team_members`/`team_send`/`team_ask`/`team_inbox`/
`team_task_gate`/`team_gates`, bounded worker orchestration (`WORKER_AGENTS`,
`CHAT_AGENTS`, `TOTAL_WORKERS_LIMIT = 12`, `8 / 10m` start rate cap, `max_depth`,
frozen-context confirmation re-verification, `build_worker_brief`), gated
inter-project relay (`same_team` guard, `20 / 10s` burst cap, canonical
`pair_key` approval modal with `10`/`30`/`100` hourly caps, `10m` decline
cooldown, Live Share non-caching bypass, and `| `-prefixed untrusted framing),
and digit-free idle wake-up planning (`SendPtyReminder` vs `ShowBanner`).

Encrypted Tessel Account Sync, rich document previews, terminal OAuth sign-in
banner, and in-place `NativeChat` ↔ `Terminal` mode switching integration
(`tessel_sync.rs` + `account_panel.rs` + `rich_previews.rs` + `device_login.rs` +
`office.rs` + `files.rs` + `preview.rs` + `terminal.rs` + `workspace.rs` +
`app.rs` + `tests/ui_sync_previews_signin_switch.rs`):
`tessel_sync` implements NFKC passphrase normalization, `phrase_is_weak`,
unambiguous 29-char `suggest_phrase()`, RFC 7914 `scrypt` (`log_n = 17`, `r = 8`,
`p = 1`), Windows DPAPI `sync-key.dat` caching, gzip + `[64 KiB .. 1.4 MB]`
bucket padding, `AES-256-GCM` (`0x01 || nonce || tag || ciphertext` with AAD
`"tessel-sync\n1\n{user_id}"`), `"rust_schema": 1` separation, sensitive field
stripping, project-shrink confirmation, and `SyncFloorFile` monotonic `seq`
anti-rollback. `rich_previews` adds `SheetTable` (`5,000 × 200` sticky grid +
RFC 4180 CSV/TSV), JSON/JSONC/JSONL collapsible tree preview, `MarkdownSizeGate`
(`600 KiB` / `1 MiB`), and vector Mermaid `flowchart`/`sequenceDiagram` painting.
`device_login` adds `400 ms`-debounced local PTY OAuth detection, and
`Workspace::switch_saved_session_chat_mode` switches Claude/Codex panes in-place
between `Terminal` and `NativeChat` while preserving pane identity and thread IDs.

Phase A full-suite acceptance audit (`Lot b173`):
`cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`,
and `cargo test --all-targets -- --test-threads=2` pass with **57 test suites
and 1,041 passing tests (`0` failures)** (`393` binary unit tests, `4` synthetic
child/reference tests explicitly ignored, and `648` integration tests across `56`
integration suites), including the rounded dual-arc `Icon::Moon` vector fix across
`10px`–`24px` viewports. Phase A (shared UI-agnostic Rust models + universal
`eframe`/`egui` compatibility renderer) has its implementation and listed suites landed; this is not full end-to-end parity acceptance. Phase B layers the native
1:1 `GPUI` primary renderer with `--renderer gpui` / `--renderer compat` and
automatic runtime fallback.

Phase B GPUI foundation, dual-renderer startup & 4-zone Flexbox shell (`Lot b174` —
`Cargo.toml` + `workspace.rs` + `app/floating_terminal.rs` + `app/gpui_bridge.rs` +
`gpui_ui/{mod,theme,icons,chrome,shell,tests}.rs` + `main.rs` + `tests/ui_gpui_shell.rs`):
Integrates `gpui-ce 0.3` and `gpui_ce_platform 0.3` over the shared `app::Tessel`
core (`new_for_gpui`, `gpui_snapshot`, `apply_gpui_action`), adds `RendererPreference`
(`AutoGpui` default, `CompatEgui`), pure `resolve_startup_renderer` (`--renderer`,
`TESSEL_RENDERER`, visual-test fixture auto-compat, and persisted `workspace.json`
preference), `Settings > Appearance` `UI Renderer` selector, and panic/error-guarded
`run_gpui_app()` with automatic runtime fallback to `eframe::run_native`. Transcribes
all 7 `GpuiThemeTokens` palettes, sRGB `color_mix_srgb`, DirectWrite font stacks,
35 embedded `resvg` vector icons + 6-tile `BrandLogo`, frameless `40px` toolbar with
`WindowControlArea` caption drag/min/max/close hit-testing, `24px` broadcast/error
strips, `24px` status bar, and the resizable 4-zone workspace shell (`Left Sidebar |
Center ViewTabs + PaneGrid | Right 7-tab SidePanel`).

Phase B GPUI DirectWrite PTY terminals, Native Codex/Claude chats, Code/Diff/Rich-Preview
editors & Windows WebView2 HWND stage (`Lot b175` — `device_login.rs` + `syntax.rs` +
`browser.rs` + `pdf.rs` + `office.rs` + `preview.rs` + `terminal.rs` + `chat.rs` +
`chat/questions.rs` + `claude.rs` + `claude/questions.rs` + `app/floating_terminal.rs` +
`app/gpui_bridge.rs` + `gpui_ui/{mod,shell,terminal_view,chat_view,editor_view,tests}.rs` +
`tests/ui_gpui_panes_editors.rs`):
Implements DirectWrite `vt100` styled cell-run extraction (`extract_vt_styled_lines`),
PTY keystroke encoding (`encode_gpui_terminal_key`), `gpui_ui/terminal_view.rs` (Block/Bar/
Underline cursor, scrollback indicator, inline `Ctrl+Shift+F` find, `400 ms`-debounced
OAuth `device_login` banner, multi-line paste confirmation, dormant `Wake`, and resizable
`Ctrl+\`` Floating Terminal drawer), `gpui_ui/chat_view.rs` (1:1 Native Codex & Claude
chats with effort pills, inline `Allow once`/`Deny` tool approvals, structured agent
question cards, and in-place `CLI` ↔ `Chat` mode switch), and `gpui_ui/editor_view.rs`
(syntax-highlighted code & unified diff editor, modified-hunk bars, Find/Replace/Go-to-line,
`Ctrl+S` atomic save, size-gated Markdown + `gpui::canvas` vector Mermaid `flowchart`/
`sequenceDiagram`, sticky-header `SheetView`, JSON/JSONC/JSONL tree, HTML/PDF/Office/Image
previews, and Windows `HasWindowHandle`-generic WebView2 `Browser` child HWND sync).

Phase B GPUI Left Sidebar, 7-tab `SidePanel`, 22-page `Settings` + Vault/Account, and all
workspace modals, menus & popovers (`Lot b176` — `explorer_tree.rs` + `finder_panel.rs` +
`share_panel.rs` + `account_panel.rs` + `vault_panel.rs` + `app.rs` + `app/gpui_bridge.rs` +
`gpui_ui/{mod,theme,icons,chrome,shell,sidebar_view,side_panels_view,settings_view,modals_view,tests}.rs` +
`tests/ui_gpui_shell.rs` + `tests/ui_gpui_panels_settings_modals.rs`):
Completes 1:1 GPUI views over `app::Tessel` for `sidebar_view.rs` (`52px` collapsed rail +
resizable sidebar with folder groups, project cards, session rows with `SessionTeamBadge` &
Live Share guest pills, Git worktrees, hidden-worktree banner, cleanup trigger, orphan
sessions, and `SidebarPreferences` chips), `side_panels_view.rs` (`Dashboard`, `Files`
Explorer + background content search, `Changes` SCM + docked `Commits` drawer with
`gpui::canvas` 5-color topological swimlanes, `Tasks` 4-column Kanban board, `TaskHistory` +
CSV export, `History`, and `Conversations`), `settings_view.rs` (all 5 groups / 22 pages +
Windows DPAPI `Passwords` vault, `Tessel Account` + E2EE sync, `Appearance` `GPUI` vs
`Compatibility` renderer selector, and `Escape`-clearing search rail), and `modals_view.rs`
(`CommandPalette`, `FileFinder`, `LaunchMenu` with `gpui::canvas` 16×16 split icons,
`HelpDialog`, `LiveShare` host/join + SAS knock modal + toolbar pill popover,
`AddProjectDialog`, `GitHubDialog`, `NotesPanel`, `ReviewPanel`, `WorktreeCleanupModal`,
`Teams` manager + worker/inter-project relay modals, toolbar `Usage`/`Notifications`
popovers, `StatusBar` `Resources` `gpui::canvas` sparkline / `Ports` / `SSH` popovers, and
generic `prompt_modal` / `confirm_modal`), expanding `GpuiIcon` to 45 validated `resvg`
vector icons.

Phase A + Phase B dual-renderer full-suite acceptance audit (`Lot b177`):
`cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and
`cargo test --all-targets -- --test-threads=2` pass cleanly with **60 test suites and 1,074
passing tests (`0` failures, `0` warnings)** (`401` binary unit tests passing with `4`
synthetic child/reference fixtures explicitly ignored, plus `673` integration tests across
`59` integration suites including `ui_gpui_shell` [9], `ui_gpui_panes_editors` [13], and
`ui_gpui_panels_settings_modals` [3]). Both Phase A (100% shared Rust feature models +
universal `eframe`/`egui` OpenGL Compatibility renderer) and Phase B (native 1:1 `GPUI`
hardware-accelerated primary renderer with `--renderer gpui` / `--renderer compat`,
`TESSEL_RENDERER`, `Settings > Appearance` selector, and automatic runtime fallback) are
implemented on `main`; the suite totals above do not establish complete visual or functional parity. Unwired services and native acceptance gaps below remain part of the migration objective.

## Phase C: Tessel parity through 2026-11-08 (Lots b178–b182)

### Resumed integration audit (2026-11-11)

- Full parity remains unproven. `RemoteTreeCache` and `RemoteGitLruCache` currently have no production consumers; the SFTP panel now calls `start_background_if_enabled`, but the broader Files/Changes cache integration remains pending. The SSH tools bridge is also not connected to running sessions.
- The GPUI remote stage now routes native navigation, mutations, download and basic text editing through the existing SFTP panel. Local PDF/image stages retain and display native pixel images instead of dimension-only placeholders; remote previews now use the same PDF worker and native image conversion. Replaced or hidden previews explicitly release GPUI atlas textures, with an identity-based retirement regression test. A native Windows synthetic fixture previously verified dirty-text navigation guards, discard restoration, both local PDF pages and a red/blue image. The new remote PDF fixture starts successfully, but its visual acceptance is pending: the Windows capture helper failed with `native pipe is unavailable`, including after recovery. Focused remote/media tests and all-feature binary Clippy pass. Local editor text-caret/selection integration and real-server acceptance remain gaps; remote native input is tracked below.
- The native host dependency now pins `78db68d61556bfa59a058080cf9270a8f47301b0`, including reviewed Tree/Git subscription teardown, shared-folder ownership and ordered reconnection. Its messages are `tree.*` and `git.*`, not the JavaScript cache model's `fs.watch` names. Host validation covered 34 client tests on Windows and Linux plus 17 real-daemon Linux tests; GitHub CI did not start because of account billing, so musl/fuzz CI was not revalidated. The SFTP panel watcher is now connected as described below; the broader Files/Changes native cache integration remains pending.
- GPUI now routes remote Office documents through the local read-only content renderer, including section selection and loading/error/truncation states. Local and remote pending Office workers keep their visible view refreshing, and receiver disconnection terminates the loading state in both renderers. Nine Office tests cover parsing, worker completion/disconnection, original-byte retention and section selection; strict all-feature binary Clippy passes. Native visual acceptance remains pending while the capture helper is unavailable.
- The GPUI sheet renderer now uses a virtual row list and shared horizontal header/body scrolling up to the existing decoder limits (5,000 rows/200 columns), replacing an undocumented 400-row/32-column display cutoff. Bounds/truncation regression tests pass. Native visual scrolling acceptance, GPUI filtering and cell inspection remain pending.
- Completed Office summaries and their sheet data use shared `Arc` caches, invalidated on worker delivery and section changes; CSV/TSV caches are shared from Document through snapshots to the virtual list. Office tests check stable identity and section invalidation; the integrated editor test checks reuse across snapshots and fresh data after CSV edits.
- Claude local trust previously raced process startup, wrote only the default account, accepted home/root aliases containing dot components, and allowed home ancestors. The launch worker now prepares the selected account before spawning, validates lexical and canonical targets, and uses Claude's lock plus an atomic unique temporary file. Missing or invalid configuration is not created/repaired implicitly. Worktree provenance and other trust providers remain integration work; the SSH trust helper is still unused.

Source baseline advanced to `C:\Tessel` `cc1ec459` (2026-11-08).

- `b178` / `b179`: the 2026-11-03..07 fixes, MCP remote sign-in, account admin portal, dev-copy
  fast-forward updater, built-in Browser sized OAuth sign-in popups, and the Conversations
  `promptBefore` backwards transcript scan.
- `b180`: SSH terminals wrapped in per-pane `tmux` sessions on a private `tessel` socket
  (system `tmux` first, then `~/.tessel-server/bin/tmux`, plain login shell without either;
  jump host / proxy command not wrapped), per-host **Persistent terminals (tmux)** and
  **Tessel daemon** switches in both renderers, the exact `tessel-host … connect --stdio`
  command check, and Claude folder trust before a local Claude terminal starts. Ported and
  tested but not yet connected to live sessions: the daemon-backed `Files` / `Changes`
  caches, the SSH remote agent tools bridge, the SSH-side Claude trust script, and the static
  `tmux` installer.
- `b181`: the Tessel daemon is on by default for every SSH host (user request, 2026-11-08);
  the per-host switch turns it off.
- `b182`: Darwin's toolbar and sidebar fixes (`b9061124`, `d5f3231e`, `cc1ec459`): the
  toolbar's tightness comes from its measured room (`toolbar_fit_level`, exact
  `toolbarFit.js` formula); level 1 gathers the secondary buttons under one `⋯` menu with
  the same actions, shortcuts and checks; level 2 also drops the brand version and shrinks
  the command box to its icon. The Live Share pill is the compact 20 px status (count or
  host name, words in the tooltip). The expanded sidebar's Search button is gone; the
  command palette finds secondary Git worktrees and the open browser tab. Both renderers.

Phase C full-suite acceptance audit: `cargo fmt --check`,
`cargo clippy --all-targets --all-features -- -D warnings`, and
`cargo test --all-targets -- --test-threads=2` pass cleanly with **64 test suites and 1,119
passing tests** (`401` binary unit tests with `4` synthetic child/reference fixtures ignored,
plus `718` integration tests across `63` integration suites, including `ui_delta_b178`,
`ui_delta_b179`, `ui_delta_b180` and `ui_delta_darwin_toolbar`).

SFTP Tree watcher integration: after an explicit successful SFTP connection, an enabled saved host can start/reuse the native host manager. One cancellable worker watches a relative directory under the stable canonical SFTP root. A separate bounded expand helper lets the event receiver drain continuously; raced listings retry at most three times, 250 ms apart. Snapshot materialization is coalesced at 100 ms. Initial/post-request cache alignment permits one SFTP reread; unmatched or unsupported lists retain SFTP. Twenty focused remote tests pass, including a real client/protocol TCP loopback fixture with 600 events before an expand reply and another raced update, plus recovery after a connection interruption longer than the RPC timeout. Expansion waits for a connected client and transient RPC failures retain the bounded retry budget. No real SSH host or native visual acceptance was exercised for this lot.

Native remote input: gpui_ce_elements 0.3.0 supplies editing entities for the three SFTP text fields, using the existing command guards. Entities are scoped by host/directory/file, preserve pending edits and focus during busy operations, retire undo state on discard, and suppress repeated rejected dispatches. A GPUI test with a synthetic, network-disabled fixture exercises Unicode replacement, stale entity rejection, busy replay, IME marked-text replacement, selection/paste and Undo/Redo. The GPUI test-support feature is dev-only (verified absent from the normal/build dependency graph); it requires --features visual-test to run this fixture. The focused remote suite passes 22 tests with --features visual-test, including the GPUI fixture and a draft-update guard regression. Other form inputs still use the earlier input path; the local source editor integration is described below. Native visual acceptance is pending.

Native local source editor integration (in progress): a local Apache-2.0 gpui_ce_elements 0.3.0 patch adds UTF-8-validated, exact-source-bound immutable TextRun styling; the patch and license are recorded in vendor/gpui_ce_elements. The line-token cache reuses unchanged prefixes/suffixes and merges adjacent equal runs. Native entities are retained per open document instance, with exact base-text validation and rejection of stale, read-only or oversized edits. The old append/backspace bridge is removed. Find and Go-to-line consume navigation after layout snapshots and convert scalar offsets to byte offsets before native selection. The gutter derives UTF-16 line positions from native layout, including scrolling and wrapped rows. Line-number and syntax preferences are propagated. Automated native input tests cover editing, per-tab history, stale entities, reopened files, Unicode and navigation beyond the former 500-line preview. Headless GPUI verification is not native Windows visual acceptance; full Monaco parity (including language-specific automatic indentation, multiline syntax state and richer editing commands) and other form inputs remain unfinished.
Local-editor validation: seven focused native-editor tests pass, followed by all 20 GPUI tests; strict all-feature binary Clippy and formatting pass. The root-focus regression test types at the retained caret before and after Ctrl+F/Escape. A 600-line Unicode fixture checks full source text, column-aware navigation through multiple snapshots and native search selection. The compatibility renderer and Windows visual acceptance were not revalidated by this lot.

Native local indentation: contextual Tab/Shift+Tab actions now use the file's opening-time indentation detection, next/previous tab stops and a single native undoable block edit. Source text, CRLF, selection direction and document guards are retained. The detection and marker rules are adapted from the locally installed Monaco 0.57.0 with full MIT attribution. Reproducible synthetic fixtures compare 170 indentation guesses and 396 ShiftCommand text/selection results; the latter uses the official IntervalNode/nodeAcceptEdit tracking and ShiftCommand.computeCursorState with language-aware indentation disabled, not a full Monaco TextModel. A native GPUI test registers the normal EditableText bindings before the local overrides, then exercises real Tab/Shift+Tab, reversed selection and Undo/Redo. All 24 GPUI tests pass. Remaining gaps include language-aware automatic indentation, multiline syntax state, advanced editing commands and native Windows visual acceptance; these results do not prove whole-editor or application parity.

Native GPUI chat composer integration: retained native text entities are keyed by pane and unique backend composer identity; edits validate the current session provider and exact base text without copying transcripts. Submit flushes pending native text and validates both identity and text before entering the existing provider send path. Successful sends retire the previous undo entity; offline failures retain drafts. Claude's offline new-conversation action rotates identity while preserving its existing draft behavior. The old append/backspace chat-composer path is removed. Oversized drafts remain in the native field with an English/French warning and can be shortened. IME composition suppresses Submit; floating terminal/modal/view focus displaces the composer. Native Windows visual acceptance, attachments and full Vue composer parity remain pending; structured question inputs are tracked below.

Composer verification: the 28-test GPUI suite passes, followed by a dedicated successful-send regression for both providers. Its bounded test-only output channels capture the existing provider message paths without a CLI process or network; the test verifies pending-edit flush, exact payload text, single emission, composer identity rotation, stale-event rejection and empty undo history after sending. This is not live-provider or native Windows visual acceptance.

Native structured questions and request guards: free-text answer fields now use retained GPUI entities keyed by pane, request instance and question index. The adapter preserves caret, selection, clipboard and undo; Enter flushes all answer fields before submission, Shift+Enter inserts a newline, and IME composition postpones submission. Oversized answers remain editable with a warning at the existing 4,096-character boundary. Exact base-text validation rejects stale edits. Each question request and tool approval has an independent monotonic identity; option, submit, skip and permission actions validate the identity captured by the rendered control and the current provider before acting. A delayed click cannot resolve the next queued approval. Unchanged answer flushes must not call Codex set_text, which would otherwise select Other over an existing option. Native Windows visual acceptance, option-only keyboard navigation and complete Vue chat parity remain pending.

Structured-question validation: all 33 GPUI tests pass, including Unicode selection/paste, Undo/Redo, Shift+Enter, IME submission suppression, multi-field flush, oversized-answer retention, replacement-request rejection, stale approval clicks and preserving a selected Codex option while flushing an unchanged Other field. Strict all-feature binary Clippy and formatting are checked before publication.

Native GPUI palette and file finder: native single-line editing entities replace append-only queries. Each binding validates its opening generation, project and exact base text. Keyboard selection and submission flush pending edits; IME composition prevents premature submission. Result clicks validate the rendered query and command ID or relative path rather than trusting a stale result index. Finder queries over 256 Unicode characters retain the draft with a localized warning. Native Windows visual acceptance remains pending.

Palette/finder verification: the 36-test GPUI suite passes, followed by all four focused native-search tests after the additional palette-toggle focus fix. Tests cover Unicode selection/paste, Undo/Redo, IME suppression, stale entity and row rejection, oversized finder drafts, real asynchronous local-file opening and restoring the editor caret after Ctrl+K twice. Halley reviewed the scope and focus paths.

Native search result acceptance: Enter and Shift+Enter capture the displayed query and selected command ID or file path, including an explicitly empty result. With unchanged text, a background update cannot silently redirect acceptance to a different result; missing results are refused. If a pending native edit changes the query, acceptance uses the freshly filtered selection.
A native navigation revision also remembers the last explicit arrow-key choice, so Down followed by Enter before repaint uses that choice rather than the older frame or a later scan result. Synthetic tests deliver scan completion through the actual Finder mailbox and cover removal, background selection changes, pending query flush, previously empty results and same-frame keyboard navigation.

Native editor tool fields: Find, Replace and Go-to-line now retain native editing entities per document instance and field. Exact-base edits require the field to remain visible on the active document; modal and closed/replaced-entity events are refused. Commands preflight IME and flush required fields before using the backend. Replace and Replace all produce one bounded native source edit, preserving source undo history instead of replacing the document outside the native editor. UTF-16 selection conversion respects Unicode; predicted replacement size is checked before allocating the replacement. Native Windows visual acceptance remains pending.
Keyboard Ctrl+F/Ctrl+H/Ctrl+G now open or focus the corresponding field and select its query instead of toggling it closed. Escape on a composing Replace field checks that field before closing. Input listeners verify native entity identity as well as document identity. Single-replacement navigation is covered by the subsequent parity pass below.
Editor-tool validation: all 41 GPUI tests pass before the final read-only shortcut guard; focused tests additionally exercise repeated Ctrl+F selection, Replace-field IME dismissal refusal, pending replacement flush, full source Undo/Redo, Unicode Go-to-line, stale document events, size planning and refusal to open Replace on a read-only diff. Strict all-feature binary Clippy passes.

Single replacement navigation: the native command first selects the next match if the source selection is not already exactly on one. After replacement it searches from the end of inserted text, skipping new matches inside that text until wraparound. The match index and native selection update together under document-ID, source-text and query guards. Native source edits flush synchronously for consecutive commands before repaint; queued TextChanged then observes the acknowledged state. A small Apache-2.0 vendor extension exposes explicit undo stops without empty history entries, separating replacement commands from each other and adjacent typing. Semantics were checked against the installed Monaco findModel.js replace/research/_executeEditorCommand implementation; regex, whole-word and preserve-case modes remain outside this pass.
Replacement validation: six focused editor-tool tests pass, then the complete 44-test GPUI suite passes after adding adjacent typing and edit-after-undo coverage. The fixture performs consecutive replacements before any repaint, checks each Undo/Redo step separately, verifies selection-only first clicks and Unicode matches inside replacement text with wraparound. Halley reviewed the implementation and vendor notices. Formatting and strict all-feature binary Clippy are required before publication.

Native terminal search: every search opening has a monotonic identity independent of the pane ID. Native fields validate this identity, exact query base, current project, floating-terminal visibility and modal state. The old append-only TerminalFind path is removed. Enter/Shift+Enter flush pending edits before navigation; Escape checks IME before closing, and Ctrl+Shift+F refocuses/selects the existing query. Oversized 64 KiB drafts remain editable with an English/French warning. Native Windows visual acceptance remains pending.

Terminal-search validation: all three focused native tests and the complete 47-test GPUI suite pass. Fixtures use a short-lived profile-free shell and replace the PTY input channel, asserting that search edits send no bytes and that Escape restores ordinary terminal input. Tests also cover Unicode selection/paste, Undo/Redo, IME dismissal, same-pane reopening/restart, project changes, floating-terminal visibility, modals and retained oversized drafts. Halley reviewed identity, focus, stale-event guards and removal of obsolete query/step actions.

Native browser address: a native editing entity replaces append-only address input. Navigation, external address updates and closing the tab invalidate its generation; edits require the current project, visible browser, no modal/floating terminal and an exact base. Enter and Go flush pending native text and respect IME and the existing URL policy. Oversized drafts remain editable. Ctrl+L selects the address; explicit WebView focus acquisition clears GPUI focus without reclaiming the native child. Native Windows visual/accelerator acceptance remains pending.

Address redirection handling: the current page location and native draft are separate. Native focus intent starts editing before queued TextChanged, so even a navigation mailbox event between typing and delivery cannot discard the draft. Page navigation updates the committed location; Escape or entering the WebView cancels editing and restores that location. Successful explicit navigation ends editing, while an invalid URL preserves the editable draft and history.

Browser page summaries (including Live Share metadata) read the committed page location, never the unsubmitted native address draft.

Address-bar validation: four focused native tests and all 51 GPUI tests pass, plus the existing original URL-normalization/refusal test. Coverage includes Unicode selection/paste and Undo/Redo, pending submission, stale generation and closed-tab rejection, modal blocking, retained oversized drafts, IME, a real synthetic Navigation mailbox event before pending TextChanged, and Escape restoration. Strict all-feature binary Clippy and formatting pass. Halley reviewed the scope, focus and URL-policy paths. While the address field is actively focused, page navigation is intentionally reflected on cancel rather than replacing its draft, even before the first edit. No WebView, real session or external site was opened by these tests.

Native Help/Settings search: per-overlay generation and field identity guard text edits and dismissal, including closed-and-reopened dialogs. An outer action wrapper observes overlay transitions even when inner actions return early or invoke another action. Native fields replace append-only handlers, retain oversized 8 KiB drafts and preserve selection/clipboard/history. Escape checks IME, clears the current native draft first, then closes when empty; Enter is consumed because filtering is immediate. Settings page navigation preserves the query entity. Auxiliary dialogs suspend native edits. Native Windows visual acceptance remains pending.

Help/Settings validation: three focused native tests and all 54 GPUI tests pass. Tests cover both dialogs, Unicode clipboard/history, Enter consumption, IME, oversized drafts, delayed closed-dialog events, page navigation and backend-opened autofocus. Escape clearing has explicit undo boundaries and the fixture verifies Ctrl+Z restores the query. Strict all-feature binary Clippy and formatting pass; Halley reviewed the implementation and final changes. A fresh Windows inventory attempt still reports the native computer-use pipe and browser host unavailable; no visual acceptance is claimed.

Native SCM drafts and composer: the previous global message was cleared before the Git worker returned. Drafts now belong to Owner(project, path), with a unique native identity and edit revision. Git polling returns an explicit success status and owner, independent of localized or error-looking output. A successful completion clears only the unchanged submitted draft and rotates its native identity; failure preserves it. Removed owners are pruned. Native input is flushed before polling and workspace actions, so queued TextChanged cannot be lost to completion; an oversized local draft marks its revision without copying it into backend state. The native composer follows ChangesPanel.vue (read-only reference): Enter inserts a newline, Ctrl+Enter commits, and a pending commit makes its field read-only. Scope/base guards reject stale submissions; unchanged native edits are no-ops. Drafts remain in memory, with no restart persistence. Windows native visual acceptance remains pending.

Git execution follows the Vue commit deadline (180 seconds, other operations 20 seconds), removes CR/NUL from the submitted message without modifying its draft, and uses stdin rather than command arguments. Windows starts Git suspended, attaches a kill-on-close job, then resumes its sole thread through stable Win32 APIs; failure to establish this protection rejects the invocation. Unix uses a private process group. Timeout/error cleanup runs before reaping the leader; normal exit disarms cleanup so detached helpers survive. Output collectors retain at most 2 MiB and give inherited pipes one second after exit: missing EOF/read failure marks the capture incomplete without changing the actual Git result. A generic disarm error contains no command, path or credential data.

SCM validation on Windows: 77 focused tests pass (58 GPUI, 12 Git, 4 draft-state and 3 Git-result tests). Native tests cover Unicode clipboard/history, Enter versus Ctrl+Enter, IME, delayed edits, oversized retained drafts, stale submissions between two retained project inputs, and a real isolated repository whose hook first rejects a commit and then allows exactly one commit with CR/NUL normalization. Process fixtures verify timeout cleanup, intentionally detached helper survival and partial output after successful Git exit. Unix process-group cleanup is implemented but not runtime-verified on this Windows workstation.

Final SCM verification: the latest zero-copy partial-output collector passes all 12 Git tests again; strict all-feature binary Clippy and cargo fmt --check pass. Halley reviewed the draft lifecycle, scope guards, process cleanup and final collector.

Native Files filter: ExplorerPanel.vue keeps query across showRoot and clears it on Escape. The Rust bridge follows that behavior, using a visible-owner generation to reject delayed edits and clear actions after project/path/visibility changes, including A-to-B-to-A before repaint. Native text is flushed before workspace actions and backend polling. Name/content mode changes retain the same entity; Enter is consumed and IME prevents premature clearing. Oversized 8 KiB drafts remain local and editable. Native Windows visual acceptance remains pending.

Explorer parity follow-up: the native entity is retained across project and mode changes when its text agrees with the shared query; subscriptions change scope, and rendered actions capture both scope and entity so retained Undo does not admit old clear actions. GPUI filtering separates the typed query from the applied query, coalescing names for 120 ms and content scans for 300 ms. Obsolete content tasks are cancelled immediately; existing hits remain visible with loading state until the replacement result arrives. Clearing is immediate. A root switch reapplies the query without delay, matching showRoot. Files requests frames while its timer or workers remain pending, including the frame consuming completion.

Files filter validation: all 61 GPUI tests and both filter-timing tests pass; the explorer integration suite passes 8/8. Tests cover native Unicode editing/history, IME, oversize recovery, query preservation and Undo across project changes, stale render actions, deferred worker startup, cancellation, retained visible hits and immediate clearing. Strict all-feature binary Clippy and formatting pass. Halley reviewed both input lifecycle and debounce parity.

Content search hardening: Refresh invalidates the active content task and query key, immediately reapplies the typed query, and preserves prior hits until completion. Collection/sorting is bounded by a global 10,000-entry budget; the 4,000-file, 200-hit and three-second budgets are checked during traversal and line scanning. Cheap type/size exclusions precede canonicalization. File reads use take(MAX_FILE_BYTES+1) and reject oversize results, including growth after metadata inspection. ContentSearchResult carries a limited flag through both renderers; GPUI displays loading and incomplete-result notices. The native 500-row cap checks one extra matching row and reports truncation instead of silently hiding it. The same stat-then-unbounded-read pattern in the Vue fallback was reported to Darwin; no Vue files were edited.

Scanner validation: explorer integration passes 12/12, including six native-filter tests covering entry/file/hit/time budgets, the exact hit-cap boundary, limit+1 reads, 500/501 visible rows, and a changed temporary file whose hit disappears after Refresh without a query change. Halley reviewed scanner bounds and rendering; loading is shown only for an empty list, limit notices follow results, and empty/no-search cases use distinct text. Strict all-feature binary Clippy and formatting pass.

Native side-panel queries: Dashboard, TaskHistory, History and Conversations now use one bounded map of four native input bindings. Each target preserves its own query/history across v-show-equivalent panel switches and project changes. A scope generation tracks panel/project/visibility transitions; rendered actions and callbacks check target, entity, scope and exact base. Pending edits flush before polling and workspace changes. Enter is consumed, IME guards clearing, Escape clears first, and empty Escape keeps the sessions panel fullscreen while the other three restore it. Search placeholders match Vue EN/FR text; indexed full-history search remains a separate capability. Windows visual acceptance remains pending.

Side-panel input validation: all three new fixtures and the full 64-test GPUI suite pass. They exercise Unicode replacement, independent Undo/Redo, pending edits across panel/project changes, stale scope rejection with retained entities, IME, oversized drafts and panel-specific empty Escape. Halley reviewed the implementation. Native visual acceptance remains pending.

Side-panel work isolation: GPUI snapshots collect dashboard, explorer, task/history, SCM history and conversation lists only for their visible tab. The sessions filter uses a single-flight worker with exact request identity, 180 ms debounce, cancellation between reads, 256-file/16 MiB total/256 KiB per-file bounds and a cooperative three-second deadline. A truncated or unreadable transcript marks results partial; matching metadata remains visible. Unchanged requests reuse results, and Refresh invalidates even unchanged metadata. Hidden queries cancel without allowing a second worker while the old one finishes. Initial session discovery and the visible Conversations transcript refresh remain synchronous and require a separate migration.

Session-query review follow-up: bounded reads use the most recent 256 KiB and discard the first cut line; a missing older prefix still reports a partial search. Loading/partial notices follow result rows. Hidden completed results remain cached; hidden in-flight work is cancelled. Raw transcript scanning is not the Vue full-text index and can match JSON field names; metadata comparisons still run over the bounded session cache on visible renders.

Session-query validation: 68 tests pass (64 GPUI, one hidden-panel fixture and three worker fixtures). Coverage includes recent-tail matches, masked snippets, file/byte/time limits, cancellation, stale completion, debounce, cached renders after file removal, hidden-cache reuse and Refresh invalidation. Halley reviewed the gating, opt-in control, worker and tail-read changes.

Native task creation: the side-panel input map now includes the task title. TaskBoard.vue is retained under v-show without a project key or a workspace-change draft reset, so GPUI preserves that draft/history across panel and project changes while renewing its submission scope. Enter and Add flush pending edits, require the exact scope/base and defer during IME composition. Escape retains the title and exits fullscreen; over-1,000-character drafts remain native and unsubmitted. Backend refusal keeps the draft; actual addition clears it, advances the generation and retires its native entity so Undo cannot resurrect a submitted task. Native Windows visual acceptance remains pending.

Task-input validation: 71 tests pass (66 GPUI, two bridge fixtures and three task-action fixtures). Coverage includes Unicode selection and Undo, retained Escape drafts, one submission after double activation, fresh history after success, project-scope rejection, IME, oversized recovery and a full 10,000-card board preserving its draft. apply_tasks_board_action now returns its explicit success boolean; native submission does not infer success from list length. Halley reviewed both versions and the final boolean result.

Session discovery migration: refresh_past_sessions now queues one cancellable worker for both renderers. Polling accepts only the current generation and preserves prior results while Refresh runs. The requested flag prevents a successfully empty cache from triggering repeated scans on every frame. Discovery shares a 10,000-entry budget (including ignored entries), a bounded heap retaining the requested most-recent sessions by mtime before reading at most 500 bounded summaries (16 KiB Claude heads; Codex metadata lines capped at 64 KiB with an optional title search over at most 256 KiB/128 following lines), existing depth limits and a cooperative three-second deadline. Enumeration errors, entry/time budget exhaustion and failed summary reads are reported as partial; the requested list size and design depth are not errors. GPUI and Compatibility show loading/partial status. Explicit discovery may finish and cache results after its panel closes; it does not rescan while hidden. Visible Conversations collection and Compatibility transcript-content filtering remain synchronous follow-ups.

Discovery review follow-up: recency is selected from metadata before file contents are read, avoiding dependence on directory enumeration order. This pass still scans the default Claude/Codex homes. Account-aware roots, stable account identity on history entries and correctly bound resume actions are a separate pending parity lot; no managed account directories are mixed into unscoped resume actions.

Discovery source validation: Claude candidates are only UUID-named .jsonl files immediately inside projects/<slug>; subagents, metadata JSON and tool-result trees are excluded. Codex candidates use rollout-…UUID.jsonl names and must begin with session_meta, read through the first newline with a 64 KiB bound because embedded agent instructions routinely exceed 16 KiB; the stable ID and cwd come from its payload rather than the rollout filename. Invalid or unreadable selected Codex metadata marks the scan partial.

Codex display titles use the first real user prompt (response_item or event_msg), skipping injected angle-tag context and normalizing whitespace, within the bounded head budget. UUID remains the fallback when no prompt is available. No transcript text is logged.

Discovery validation: 73 GPUI/bridge/scanner tests passed before the final source-layout and title refinements. The final eight focused fixtures pass with realistic Claude UUID paths, exclusion of subagent/metadata files, newest-first selection, Codex stable identity, 24 KiB metadata, prompt-title extraction, 64 KiB metadata refusal, cancellation, stale worker completion and empty-cache reuse. Halley reviewed the worker, bounds, recency heap, source filters and final Codex reader.

Conversations preview migration: local transcript lookup, metadata and bounded reading now run in one background worker. Request identity includes provider root, conversation ID, pane and title; generation changes discard obsolete completion. Closing the panel cancels work cooperatively and prevents subsequent scans. Unchanged size/mtime reuses parsed messages, with refresh due after two seconds. Bounds: 100 inputs, 10,000 directory entries, cooperative three-second scan, 256 KiB tail plus 1 MiB prompt recovery per file, 32 KiB per body, 2,000 messages and 8 MiB combined. Unknown preceding turns remain hidden instead of reusing a stale cached prompt. Native visual acceptance and full conversation parity remain pending.

Conversations validation: 74 GPUI/worker/bridge tests passed, then nine focused tests passed after the global-limit cache fix. The six existing ui_delta_b179 prompt/OAuth tests, strict all-feature binary Clippy and formatting pass. Halley reviewed cancellation, cache reuse, source identity and bounds. No native visual acceptance is claimed.

Native conversation model cache: GPUI snapshots share an Arc of grouped threads. Activity sequence/length, preview revision and filter identity invalidate the grouping. The exact next window expiration and backward wall-clock changes also invalidate; visible polling requests the expiration frame. Compatibility egui still builds its view directly.

Model-cache validation: six focused model, bridge and seven-panel roundtrip tests pass, including Arc reuse, filter/source invalidation, expiration boundary and clock reversal. Strict all-feature binary Clippy and formatting pass. Halley reviewed all mutation paths; native visual acceptance remains pending.

Session account identity migration: terminal Claude/Codex panes retain Default, Managed account ID, or Configured scope hash; no raw profile root or credentials are serialized. Original UUID, folder and binding are assigned before starting. Missing accounts and changed custom roots fail without fallback. History is account-aware; Claude uses its current source and Codex includes default and managed sources, with shared discovery limits. Legacy panes recover a unique matching profile through a single worker with a bounded 100-job queue; cancelled jobs are discarded before scanning. Claude probes the expected transcript path first, then only project folders without enumerating their transcripts. Limits: 100 accounts, 10,000 entries, cooperative three seconds, 64 KiB metadata line, 8 MiB metadata budget. An unreadable candidate cannot establish uniqueness. Backup and sync strip local bindings. Windows lexical aliases are normalized. The current workspace remains selected, matching the Vue action. Native-chat profile propagation is implemented in the follow-up below. Consistent cross-folder resolution of relative custom profile roots remains pending; a changed root refuses rather than changing accounts.

Session-profile validation: 33 focused profile, resume, workspace, backup and sync tests passed; after the final single-worker queue/direct-path refinement, eight targeted tests passed including the synthetic CLI migration/persistence cycle and ambiguous-profile no-spawn refusal. The two panel/dialog integration suites passed 20 tests. Strict all-feature binary Clippy and formatting pass. Halley reviewed the identity, resume routing, migration queue and bounded lookup. Native visual acceptance remains pending.

History removal hardening: only cached, closed local entries with a known profile qualify. Both renderers request confirmation, then recheck cache/source identity and use a worker for filesystem validation and recycling. Claude paths must be projects/<folder>/<UUID>.jsonl; Codex paths must be recognized rollout paths with bounded metadata matching both ID and cwd. Reparse points, symlinks, traversal and non-files are refused. Native Windows recycling uses IFileOperation with FOFX_RECYCLEONDELETE and WANTNUKEWARNING; failures/cancellations are surfaced, without calling remove_file for history deletion.

Recycle operations use a dedicated STA thread even when called from an MTA worker. Shell warnings are owned by the validated main-window HWND registered by either renderer. Git discard propagates recycling cancellation/failure instead of deleting untracked files permanently; earlier items in a batch may already have been processed. Pending transcript recycling blocks exit and history resume until the worker reports its result.

History removal validation: eight focused cases pass, with the junction fixture rerun successfully after normalizing cmd path separators. Coverage includes cache/confirmation cancellation, stale-entry refusal, path/profile validation, mismatched or oversized Codex metadata, junction target preservation, real recycling from an MTA caller, Git cancellation preserving bytes and a real temporary Git discard cycle. Halley reviewed path validation, COM lifetime, HWND ownership, dedicated STA execution and removal of the permanent-delete fallback. Native modal visual acceptance remains pending.


Native account propagation: restored and new native panes in both renderers resolve the same runtime-only NativeLaunch binding before connecting. Claude receives CLAUDE_CONFIG_DIR and Codex receives CODEX_HOME; every spawn rechecks managed-account availability. Managed profiles remove inherited provider credentials, endpoint overrides and alternate Claude cloud-backend flags, preventing an unrelated ambient account from overriding their profile. Default and Configured profiles deliberately retain the user's inherited provider environment. Model changes do not replace the binding. Native panes persist bindings locally; portable backup and sync strip them, and Live Share snapshots never contain profile roots or account bindings.

Native lifecycle: saved native thread IDs are restored independently of terminal IDs. Legacy native panes use the bounded unique-profile migration worker without starting either a terminal or native transport. Startup validation and process creation run off the UI thread with a 120-second budget; protocol initialization retains its separate 30-second deadline after process delivery, matching the reference providers. Cancellation drops undelivered process owners; Claude uses a kill-on-close Windows Job and both native owners kill and wait for their child on drop. CLI-to-native switching waits for stop_for_restart confirmation behind the existing restarting guard. A missing acknowledgement or stop failure preserves the original mode; switching is refused during terminal/native startup. Each native pane owns its process; accounts never share a transport. Visual acceptance and real-provider interaction remain unverified for this follow-up.

Native-account validation: 40 focused tests pass across original-account recovery, managed process separation using synthetic children, missing-account/transcript refusal, cancellation, confirmed mode switching, persisted round trips, native protocol permissions, backup and encrypted sync. Strict all-feature binary Clippy and formatting pass. Halley reviewed account injection, worker cancellation and confirmed terminal shutdown. No real model requests or personal profiles were used for these fixtures.


GPUI board interactions: unstarted cards have a native inline rename field with exact draft/entity/serial checks, Unicode selection/undo, IME-safe Enter/Escape, blur-save and a 1,000-character submission limit. A task leaving To do or disappearing ends the edit; concurrent title changes follow the reference last-write-wins behavior while it remains To do. Shared rename mutations now also reject started cards. Done selection supports individual toggles, All/None, Escape and confirmed group removal. The project/generation scope rejects retained callbacks after navigation; selection and confirmation ids reconcile against current cards, and grouped removal preserves cards moved out of Done. Single/group EN/FR confirmation text matches the reference without inventing task worktrees. The current reference CSS stacks its status sections at all widths, as GPUI does. Both renderers now keep sections stacked at all widths. Native scoped drag/drop and Alt+Left/Right movement are integrated; remaining card presentation parity is tracked separately. Native visual acceptance remains pending.

GPUI board validation: 31 focused native-input, scoped board, mutation, panel and shared-workspace tests pass. The rename fixture explicitly activates its simulated window and drains the activation queue before testing real blur delivery; it covers selection, Unicode, Undo, IME, oversized drafts, cancellation and stale cards. No real application was restarted. Strict all-feature binary Clippy and formatting pass; Halley reviewed the scoped state, confirmation and input behavior.


Board movement follow-up: both renderers retain stacked status sections at all widths. Native card focus is stable by project scope/id across column changes. Alt+Left/Right steps from the current model column; ordinary arrows and modified combinations cannot move cards. Held-key repeats are intentionally ignored to prevent accidental multi-column traversal (the reference does not explicitly suppress them). Internal typed card drags outside controls target column bodies, carry scope/id, and are revalidated against the current project/card/edit state; no external drag payload is exported. Escape cancels a drag before selection/overlay handling. Full card presentation and native visual acceptance remain pending.

Board movement validation: 20 focused tests pass (14 native model/input/panel cases and six Compatibility UI cases), including real headless pointer drag/drop, Escape cancellation, retained keyboard focus, rejected repeats/modifiers and stale scope after hiding the board. Strict all-feature Clippy for the binary and board integration test, formatting and diff checks pass. Halley reviewed the delta. No application restart or real-provider request was used.


Card presentation follow-up: native status sections use the reference padding and header/count typography, dashed empty placeholders and 3px card accent. Cards use vector rename/delete controls and a dedicated checkbox, with whole-card selection outside controls. A parent boundary consumes pointer presses/clicks after each real control handles them, so controls neither start card drags nor toggle selection twice. The card is draggable outside controls, except while renaming; drag rendering dims its source. Losing the active window, opening a modal, leaving the scope or removing/editing the card cancels its transient drag. The shared Task model still lacks assignment/timing/orchestration fields; those controls are not fabricated. Native pixel acceptance remains pending.

Card controls also expose stable native tab stops, Enter/Space activation, accessible names, button/checkbox roles and checked/disabled states. Board focus tracking includes control descendants and the Done header so their keyboard input is not routed into terminal panes. Done tools follow All/None, Delete(N), Cancel order; Delete(0) remains visible and disabled. Native keyboard fixtures use real keydown/keyup pairs (the convenience simulate_keystrokes method only emits keydown), with a render after focus changes.

Board focus lifecycle: keyboard rename save/cancel returns focus to the card. Task deletion confirmation starts on Cancel, traps Tab/Shift+Tab between its native buttons, and restores the triggering control on cancellation; if that control disappears it returns to the board. Native button activation uses key release for both Enter and Space, a small intentional difference from browser Enter-on-keydown behavior. Holding a key does not repeatedly activate it.

Card presentation validation: 18 focused native/GPUI tests pass, including whole-card pointer drag, control drag refusal, checkbox and card selection, Tab/Shift+Tab and Enter/Space controls, held-key release pairing, rename focus return, confirmation focus trapping/restoration, stale scopes, focus-loss drag cancellation and existing shared panel/modal flows. Strict all-feature binary Clippy, formatting and diff checks pass. Halley reviewed the pointer boundaries and final confirmation/accessibility lifecycle. Pixel-level visual acceptance remains pending.


Live Share task-board security follow-up: task RPCs no longer access the separate project-local live-share-tasks.json file; existing files remain untouched and are not migrated. Explicit snapshots expose only cards in authorized shared projects. Mutations use a bounded UI command queue (32 pending, four requests per guest per second), a two-second deadline and pending-request cancellation. Current session, project and task rights are revalidated under the authority lock before a host-workspace transaction; atomic storage must succeed before the card state, action log and response are published. A timeout after execution has started reports an uncertain result and requests a refresh rather than promising no change. Removing all shared projects now revokes their access. Guest titles are limited to 300 Unicode characters, use host-issued ids and locally stored guest_by attribution; portable backup and cloud sync omit this name. Compatibility guest controls currently expose refresh/add with visible errors and bounded-view notices, not the full reference guest board. Assignment/timing/dependencies and complete guest/native visual parity remain pending.

Live Share task-board validation: 18 focused host/guest/transaction/cache tests pass, plus 15 Live Share integration cases and 13 Teams integration cases. Fixtures cover real TLS, actual atomic persistence and failure rollback, project scoping, revoked rights, cancelled/expired requests, bounded queues/responses, local guest attribution and backup/sync scrubbing. Unchanged boards are compared exactly without cloning or serializing every UI tick; shared-project or host-certificate changes force republication. Normal snapshot preparation runs outside the authority lock. A stale Teams SavedSession fixture was updated to include the existing profile-binding field. Halley approved both the security implementation and the cache/timeout follow-up. Native guest visual acceptance remains pending.


Task lifecycle follow-up: Task.timeline is tolerant serde(default) metadata, with optional legacy dates and at most 50 Doing periods. Creation/column transitions use the same timing model across UI, agent tools, team bootstrap and guest mutations. Agent-created cards use the calling pane in their first Doing period; tessel_task_assign validates an agent in the same project. Host assignment controls and card timing labels are connected in the later presentation follow-ups. History is stored with tasks inside Workspace and committed atomically. It is unique per card, removed on reopening and retained after deletion. Attribution uses the assigned pane or last period, never the first project session. Duration sums Doing periods, unlike card lead-time labels in the reference. Missing legacy dates remain unknown (zero only in the existing history record representation); no completion date is fabricated. Clock rollback clamps negative period durations, but overlapping periods after clock adjustments may still be counted as in the reference.

Storage is bounded: new cards may occupy 2 MiB of pretty JSON, and cards plus history 2.5 MiB, reserving settings/history space within the existing 4 MiB workspace ceiling. Older boards above 2 MiB may be reduced or restored without growth; their history allowance is 512 KiB, still subject to the global ceiling. Oldest history is trimmed before refusing an oversized draft; the 10,000-card/2,000-record count ceilings are not simultaneous guaranteed capacities. Metadata-rich cards can reach the byte limit sooner, and a change that still cannot fit fails without publishing its draft. No live cards are discarded. Existing task-history.json is read once with a 4 MiB bound, deduplicated newest-per-card and left untouched; corrupt legacy history produces a translated warning without preventing settings from opening, and retries migration on a later launch. A successful retry retains newer local records and detaches legacy archives whose task ids collide with new live cards, preventing inherited costs. The migration marker persists to prevent deleted history from returning.

Compatibility: the new local Workspace history fields are not understood by older binaries with deny_unknown_fields. Downgrading can refuse workspace.json with 'original retained'; keep that original file and use the newer build to export a portable backup before moving to an older build. Backup and sync omit history, timeline and guest attribution; existing portable schema stays unchanged. Settings restore preserves local history, raises id counters, assigns fresh task ids, rematches history projects by path or leaves them unscoped, clears colliding archived pane references and resets transient board UI. Runtime sessions are captured before agent task persistence. No usage or cost is inferred from pane ids after restoration.

Task lifecycle validation: 35 focused tests and 66 tests across five integration targets pass (including shared timeline cases included by standalone targets). Strict all-feature binary/integration Clippy, formatting and diff checks pass. Fixtures cover atomic failure rollback, actual saved runtime panes, scoped assignment, reopen/delete, backup/sync scrubbing, restore identity collisions, corrupt-legacy recovery and storage ceilings. Halley reviewed the final migration, attribution and capacity deltas. Card metadata controls are covered by the later follow-up; pixel-level acceptance remains pending.

Card date presentation: both renderers consume task_card_timing labels. Optional legacy dates, To do fallback, first-start/Doing fallback and Done-without-end behavior follow the reference. Durations use the reference rounding and elapsed wall time, clamped at zero after clock rollback. ICU uses the existing preferred UI-language locale; the short clock explicitly requests minute precision and two-digit hours. Full tooltip dates include year and seconds. Date labels are cached by actual date fields/column and invalidated by local day, offset, language and locale; ICU formatters are reused per UI thread/locale. Invalid locale strings fall back to fr-CA/en-US. Preferred OS locales are still loaded once per process, as in model-list metadata. Full card layout, guest metadata and visual acceptance remain pending; local assignment controls are covered below.

Card date validation: two formatting tests, two real headless GPUI interaction fixtures and seven Compatibility board tests pass. They verify locale/minute precision, full tooltip year/seconds, invalid-locale fallback, duration rounding, unknown dates, narrow wrapping and existing drag/keyboard/control boundaries. Strict all-feature binary/board Clippy, formatting and diff checks pass; Halley approved the formatter/cache delta. Native pixel acceptance remains pending.

Task assignee controls: local GPUI and Compatibility cards choose agents only in To do/Doing, with read-only names in Review/Done. Choices use runtime panes of the current project; shells and foreign projects are excluded. Missing panes keep their stored identity, using the reference blank selector / No agent label. Assign and Show agent revalidate project/card/column/pane after capturing runtime state; only Doing/Review can show an existing assigned agent. Native menu state is scoped and cleared on project/column changes, editing, overlays and window deactivation. Keyboard navigation, 700ms prefix cycling, Home/End, Escape and Tab follow the reference; generated keyboard clicks are ignored by the combobox to prevent double activation. Deferred popup clicks cannot reach the parent trigger or card drag. Roles and selected option focus are exposed to accessibility. Brand icon/accent styling, complete guest metadata/controls and visual acceptance remain pending.

Show agent targets the exact terminal or native chat composer and releases floating-terminal keyboard ownership only after successful validation. The floating panel remains open; refused actions leave its focus unchanged.

Task assignee validation: nine focused native/model tests and all eight Compatibility board tests pass. Coverage includes keyboard/pointer selection, menu toggle/option bubbling, stale scopes, excluded foreign/shell panes, read-only Review, exact focus with a visible floating terminal, and drag origins on assignment/delete/move controls. Unknown or exterior pointer origins cannot start a card drag. Strict all-feature binary/board Clippy, formatting and diff checks pass. Halley reviewed the scope, focus and drag guards. Native pixel acceptance remains pending.

Cross-target validation follow-up: the global panel fixture now supplies task timing/assignment fields. All 15 panel parity tests, eight floating-terminal tests and two native selection/assignment tests pass. A floating-terminal fixture verifies that programmatic navigation cancels pending focus and preserves the visible panel over subsequent frames. Strict Clippy now passes with --all-targets --all-features, including test-only code; the editor's backward selection fixture uses an explicit anchor/cursor range. No runtime behavior changed in this follow-up.

Task assignee artwork follow-up: fifty self-contained brand documents preserve the reference SVG/PNG precedence, tight view boxes, optical sizes, tile clipping and colors. The snapshot generator records source SHA256 values, checks source constants and drawn/raw overlap, rejects scripts, fonts, directives and external references, and supports reproducibility checks. Both card renderers consume the same bytes. Parsed SVGs and rendered images are cached per thread and known kind, with thirteen quarter-DPI buckets from 1x to 4x and GPUI's normal 2x supersampling. Compatibility retains texture handles in the UI context and converts decoded BGRA to RGBA. Unknown IDs use a terminal glyph in the text color, with no invented initial badge; lookup follows the reference lowercase-only rule. Review/Done uses the reference 13px icon and 5px gap. No accent swatch is added: the reference card does not render one. Guest metadata/controls and native pixel acceptance remain pending.

Artwork validation: three focused native tests and 27 cases across the two Compatibility panel suites pass, including all fifty embedded images, multicolor marks, straight-alpha edge values, stable raster identity and absence of repeated texture uploads. Strict Clippy passes on all targets and features, including after its array-chunk iteration cleanup. Formatting, diff checks and snapshot regeneration checks pass. Halley reviewed the artwork mapping, bounded caches, DPI behavior, opacity conversion and fallback. Native pixel-level acceptance remains open.


## Guest task-title confidentiality (2026-11-12)

The host publishes a separate sanitized snapshot: recognized credential patterns, private paths, ANSI/control/bidirectional formatting are removed from titles and guest attribution. In-project paths remain relative. Accepted titles are scrubbed completely before their 300-character output limit, so shortening an earlier path cannot reveal a secret cut at the old scan boundary. URI user information and unquoted encoded credential values are masked before and during percent decoding; encoded opening quotes are preserved until the full quoted value can be masked. Host-local cached results are keyed by card identity, project/root, raw title and attribution. Only retained cards remain cached; unchanged entries are shared through Arc during preparation outside the authority mutex. This is pattern-based redaction, not a general secret detector.

Publication retains the latest 500 valid cards per project and the existing total encoded-response bound; partial-view notices are scoped to the requested project. Incoming guest titles remove terminal markup, controls and bidirectional formatting before their length is validated. Non-UTF-8 root paths fail closed to hidden paths rather than granting a wider relative-path scope. Root changes invalidate snapshots and their local cache identity; publication rechecks the roots after preparation. List and mutation replies use that same publication boundary. Original workspace titles, task history and local attribution are unchanged. Per-guest assignment metadata and full guest controls remain pending; no host pane identity is newly exported.

Reference extraction reads only the source helper and synthetic vectors. Validation: 65 source comparison vectors and four additional adversarial/input-boundary cases pass across five tests; 12 shared-board/cache/TLS tests pass, including 1,500 cards with 1,499 cached results reused after one title change. The 15 side-panel/review tests also pass with the Google-key masking correction. Strict all-target/all-feature Clippy, formatting and diff checks pass. Halley reviewed the wire boundary, root invalidation, encoded masking and cache. These synthetic checks do not replace native visual acceptance.


## Nonblocking guest task controller (2026-11-12)

The shared controller scopes requests by monotonic guest-session generation and authorized project, keeps at most one request in flight, polls reply channels without blocking, rejects late replies after scope changes, and validates at most 500 canonical unique card IDs and bounded plain-text fields. Mutations are never automatically retried: unknown outcomes retain the draft and queue one read-only reconciliation request. Successful Add clears only the sent draft revision; drafts for up to 32 projects are retained within the session.

Compatibility Refresh/Add uses this controller, and the GPUI Join dialog gains the same project selector and board with a native EditableText draft field. The input flushes changes before backend polling and checks modal lifetime, session/project scope and its synchronized base. The existing host board remains separate. Move/rename/delete guest controls, main-workspace guest integration, assignment/timing metadata and native visual acceptance remain pending. Session leave/drop is addressed by the subsequent cancellation follow-up below.

Validation: all 12 controller tests and 22 Live Share integration cases pass (the existing 21-case suite plus the new Compatibility Add fixture). The Compatibility test clicks Add and observes pending before any host commit. Three native input tests pass, including a real loopback TLS guest with Unicode input, project switches, obsolete input entities, oversized-draft recovery and preservation of edits made before the host acknowledgement. The queue-full test also passes in the native target. An explicit host response for a still-running mutation is treated as unknown, followed by a read; ordinary host refusals remain explicit. Initial reads survive a pending mutation in a new scope. Halley reviewed the authorization/scope, draft and outcome rules. These simulated native UI tests do not establish pixel-level acceptance.
Strict Clippy passes with all targets and all features, with warnings denied; formatting and diff checks pass.


## Cancellable guest-session shutdown (2026-11-12)

GuestSession leave/drop signal cancellation, shut down a registered TCP clone and detach the worker without joining on the UI thread. Socket registration and cancellation share a mutex, preventing a connection completed after departure from being installed or emitting a TLS hello. The existing five-second connect bound remains; a socket-lifetime guard drops the registered clone on any worker exit. State updates check cancellation while holding the state mutex, so the final local Ended/left state and cleared error survive late replies. The single-shot client retains its existing transport behavior. Legacy synchronous calls now use the nonblocking bounded request enqueue; migrating their reply waits out of the remaining Git/file UI paths is still pending. Host detection of departure during pending approval is tracked separately.

Validation: the 26-case Live Share integration suite passes, followed by all six final shutdown tests (two added cases; 28 distinct cases overall). Leave and Drop interrupt a silent TLS peer with all 32 request slots occupied; pending receivers disconnect, the worker exits and the left phase persists. A separate held-worker test keeps its JoinHandle inside GuestSession and releases the worker only after observing UI return, directly guarding against a reintroduced join. Tests also cover pre-start cancellation without a connection, registration after cancellation, clone release, and a real host with a task mutation awaiting completion. Halley reviewed registration/cancellation ordering, guarded state updates, transport shutdown and host cleanup. Pending-approval cleanup and the remaining synchronous reply waits remain separate work.
Strict all-target/all-feature Clippy passes with warnings denied. Formatting and diff checks pass.


## Pending guest departure (2026-11-12)

The host's approval loop probes the exclusively owned TCP socket without consuming buffered bytes and restores blocking mode before subsequent TLS operations. An orderly close or transport error removes the waiting guest and approval prompt without charging a failed knock or invitation use. A stale approval after cleanup cannot add a trust record or an offline person. The existing admission/approval race and active-session protocol are unchanged. This is socket liveness detection, not a heartbeat for silent network partitions; the existing knock deadline still bounds those waits.

Validation: all 30 Live Share integration tests pass. The new transport probe test keeps idle and buffered connections usable without consuming bytes; the real TLS approval test checks leave and Drop, prompt cleanup within one second, no invitation counters or trust/offline records, and a stale approval with no effect. Halley reviewed the exclusive socket ownership, mode restoration and cleanup ordering. Buffered unexpected bytes can still mask a FIN until the existing 300-second knock deadline; this change does not alter the admission/closure race or undo an approval already granted by the host.
Strict all-target/all-feature Clippy, formatting and diff checks pass.


## Scoped guest file editor/listings (2026-11-12)

The Compatibility Files panel replaces its unscoped downloaded-file cache and shared text buffer with independent scoped listing and editor controllers. Every request includes projectId; file responses belong to a session generation, project and validated relative path. The transport no longer caches global file contents or performs an unused default-project listing on join. Directory responses validate names, parent paths, unique entries and bounded metadata, with an explicit partial-view notice after the metadata display budget.

The editor retains unsaved or uncertain text across scope changes without permitting an out-of-scope save. Save acknowledgement updates the baseline to the sent text, preserving later edits; even reverting to the previous baseline while a save is pending remains protected if its result becomes unknown. Copy uses current text, and discarding a draft requires confirmation. Host readOnly responses and current Edit/Type permissions prevent edits to protected instruction files. Reads accept UTF-8 up to 8 MiB; 180 KiB saves fit the existing 256 KiB request envelope after base64 and path encoding. Larger/chunked saves, full native/main-workspace guest document tabs, and automatic permission-grant rereads remain parity work. Git status/commit are migrated by the follow-up below.

Review corrections remove egui's silent character truncation on paste: the model retains oversized text and rejects Save explicitly. Read-only preview layout is capped at a UTF-8 boundary at 64 KiB without modifying the loaded document or copied content. Unsupported names produce partial listings; inconsistent parents, names and duplicates still reject the response. Save controls precede the scrollable document, and successful saves clear stale discard confirmation.

Validation: 10 editor-model tests, 4 listing-model tests and all 33 Live Share integration cases pass (the paste case was rerun after fixing fixture synchronization with the initial directory response). Real loopback TLS + egui tests cover exact project/path save, protected-file refusal, disconnect/copyable draft/confirmed discard, and a 200 KiB Paste event that is retained whole but never written. Halley reviewed scope, drafts, rights, limits and the paste fix. Native visual parity is not claimed.
Strict all-target/all-feature Clippy, formatting and diff checks pass.


## Guest Git async controller and host execution (2026-11-12)

The Compatibility Changes panel replaces synchronous status calls and fire-and-forget commits with a scoped controller and explicit pending/results. Commit messages belong to a session/project, remain copyable after disconnection, and require confirmed discard. Only an exact successful acknowledgement clears the submitted revision; later edits survive. Unknown mutation outcomes remain marked even after a clean status response, and no mutation retries automatically. The host adapter releases the authority mutex during Git I/O, revalidates project access before and after execution, limits concurrent guest Git requests to two, and applies a 20-second request deadline. A shared bounded process runner retains local Git behavior while guest execution clears inherited GIT_* overrides. Hooks, signature helpers and known named drivers are neutralized; configuration origins and include targets in any guest-editable shared root are refused, including empty or absent targets. Optional missing includes outside shared roots remain allowed.

Status/diff hide private paths and report incomplete output explicitly. Literal path handling, filter refusal and staged-path scope checks protect guest mutations. Synthetic real repositories exercise hook/signing/fsmonitor markers, filtered files, nested project boundaries and mutable includes without using real profiles. Concurrent attribute/index changes still need transaction-level protection; file I/O outside the host authority mutex and native Changes workspace parity also remain pending. This is not a claim of complete secure or visual parity.


## Guest cancellation fallback (2026-11-12)

Repeated real TLS tests on Windows exposed workers that did not leave a pending read within two seconds after shutdown on the registered socket clone. Guest DeadlineStream now checks its stop token before each I/O attempt and uses at most 100 ms socket timeout slices. Intermediate timeout/interrupt errors retry against the unchanged absolute deadline; cancellation returns ConnectionAborted. Host and legacy transports retain their prior deadline behavior. Connect itself remains bounded separately.

New tests deliberately leave the cancellation socket unregistered to prove the fallback can end a blocked read without socket shutdown, and verify delayed valid data survives several slices, the global deadline still expires, and cancelled writes are rejected. Existing real TLS leave/drop/full-queue tests retain their original timing assertions.

Validation: 11 guest Git controller tests and 82 Live Share integration cases pass, including isolated real Git repositories, configuration-origin framing, absent includes outside shared roots, private hooks-directory confinement and cancellation without a registered socket. The subsequent case-sensitive driver deduplication fix passes all 19 focused host Git cases; all 13 existing local Git tests pass, including detached helpers, partial pipe capture and timeout cleanup. Strict all-target/all-feature Clippy, formatting and diff checks pass. Halley reviewed the runner, configuration boundary, transport fallback and final driver-case correction. Native visual acceptance remains pending.

## Guest file read lock isolation (2026-11-12)

Persistent-session files.list/files.read capture an approved joined guest's project, rights and privacy setting under the authority mutex, perform filesystem work outside it, and revalidate before releasing the response. Four active read slots cap concurrent filesystem work; filesystem calls themselves are not forcibly interruptible. Existing confinement, private-path filtering, 8 MiB content limits and protected-file read-only behavior remain in place. Errors are bounded and scrubbed. Tests cover mutex availability, revocation, host stop, root/privacy/rights changes and real synthetic files. File writes/create/rename/remove still require a separate mutation-boundary migration; no claim is made that all host filesystem I/O has moved out of the mutex.

The first complete 87-case Live Share suite passes. Halley approved the authority boundary, confinement, read-only metadata, slots and privacy behavior; the six final focused cases pass, including authority revalidation before an error reply and the Compatibility read/save integration. Strict all-target/all-feature Clippy, formatting and diff checks pass. Native visual acceptance and descriptor-relative filesystem access against concurrent directory replacement remain separate follow-ups.

## Guest file mutation lock isolation (2026-11-12)

files.write/create/rename/delete capture authority, prepare and execute outside HostInner, and revalidate immediately before the destination mutation plus before releasing success or error. Two slots bound active mutations. Saves retain same-directory temporary files, sync and atomic persist; create retains exclusive creation. Windows rename uses the same zero-flag MoveFileExW policy as the local file actions, so a destination appearing after validation is not overwritten; unsupported platforms refuse this operation. A directory rename without Type is refused to prevent moving nested agent instructions around the protected-path check. Activity text is scrubbed and the French delete entry accurately describes permanent deletion. Already-started OS operations may complete after revocation; no automatic mutation retry is added. Version conflicts, descriptor-relative confinement and Git attribute/index transactions remain pending.

Validation: all 95 Live Share integration cases pass. Seven mutation-specific cases cover lock availability, pre-mutation revocation for all four operations, withheld success/error after access changes, real Windows create/save/rename/delete, protected-directory moves and atomic refusal when the rename destination appears during revalidation.

Review also found that an empty existing-file path could resolve to the project root. share_path_checked now rejects empty/whitespace input, while root directory listing keeps its explicit separate branch. The mutation regression checks all four operations with empty and whitespace paths, verifies that no mutation boundary is reached, and preserves the empty project directory. Routing deletion through the real host Recycle Bin remains a separate parity task; this extraction retains the pre-existing permanent-delete behavior with accurate activity wording.

The final all-target/all-feature Clippy, formatting and diff checks pass after the empty-path correction. The Vue reference already checks protected/hidden descendants before directory moves and uses fs.trash; no matching Vue defect was confirmed in this inspection.

## Guest Recycle Bin parity (2026-11-12)

The native Windows recycler is extracted unchanged from file_actions.rs into native_recycle.rs and shared with guest deletion. Local actions and transcript recycling retain their existing call paths through that service. Guest delete no longer calls remove_file/remove_dir, and errors do not trigger a permanent-delete fallback. A supplied recycler boundary allows cancellation/error tests without touching real user data.

Directory rename/delete now inspect descendants with a 10,000-entry and two-second traversal budget, rejecting incomplete inspection, private/masked paths, unauthorized instructions and linked/reparse entries. Ordinary directories can be renamed with Edit alone, correcting the prior blanket Type requirement. Windows rename remains atomically no-overwrite. The path-replacement race and full Git-ignore parity remain separate work. Native file-operation flags follow https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileoperation-setoperationflags.

Validation: 99 Live Share integration cases pass, including a native Windows recycle call for one synthetic file, injected cancellation preserving bytes and suppressing success audit, directory dispatch and protected descendants, ordinary directory rename with Edit only, and a synthetic Windows junction whose target remains untouched. A source comparison verifies that the extracted native recycler is unchanged apart from visibility/imports. The tests do not perform a manual restore from Explorer or claim full native visual acceptance.

Review adds explicit CI protection for .github, .circleci, GitLab CI, Azure/Bitbucket pipelines, Jenkinsfile and Drone configurations even when dot-files are visible. The corresponding policy omission was also found in the Vue reference and reported to Darwin without editing that checkout. The broader agent-configuration catalogue still needs full differential parity work. These targeted protections are not a general sandbox for arbitrary code stored in a shared project.

The final 12 focused mutation tests pass after the CI guard update. Strict all-target/all-feature Clippy, formatting and diff checks pass. Halley approved the recycling mechanism, bounded descendant checks and junction refusal; broader protected-path catalogue alignment remains separately tracked.

## Protected agent-path catalogue (2026-11-12)

share/protected_paths.rs ports the Vue protected-folder, file, hook-name and hook-manager-pattern catalogue. The host file read-only, mutation and descendant checks consume this policy. scripts/extract_protected_paths_reference.mjs runs only the source's pure isProtectedRel function against synthetic paths and saves a SHA256-tagged fixture (512 vectors from the shipped Vue b1836/592c2ef source). Tests compare uppercase/nested paths, suffix lookalikes, exact patterns and explicit extra-hook-folder boundaries. The CI/deploy lists and Jenkinsfile pattern follow the merged Vue reference. Rust expresses the Jenkinsfile documentation exclusions as an exact-name filter after the positive regex; .md/.txt/.rst/.adoc remain editable while .md.groovy remains protected.

The host wrapper retains its historical conservative eight-hook-name guard. The pure module accepts explicit hook-folder scopes, but automatic discovery of per-project core.hooksPath is not yet wired; dynamic Git configuration and complete Git-ignore parity remain open. This lot does not claim full permission parity or a sandbox for arbitrary application source files.

Validation: 512 differential cases match the shipped Vue source hash, and all 100 Live Share integration tests pass. Real request tests reject edits/creation of Codex, shared-agent, OpenCode, Cline, Forgejo and Jenkins pipeline configuration without Type. Strict all-target/all-feature Clippy, formatting and diff checks pass.

## Live Share Git-ignore and secret-name privacy (2026-11-15)

Guest listing/read/create/write/rename/recycle requests now use bounded `git check-ignore -z --stdin` through the hardened Git runner. Exit 1 is accepted only for this dedicated command (no matches); configuration errors, malformed or truncated output, timeout and busy state refuse the request. The check-ignore command omits `--literal-pathspecs`, which Git rejects for this operation; SCM retains literal pathspecs. Inputs are bounded to 10,000 paths and 1 MiB, output to the shared runner's 2 MiB cap, and the Git preparation/check sequence shares a ten-second deadline.

Each Host owns a sticky history (64 roots, 200,000 entries per root, case-normalized ancestor matching), reset with a new Host. There is no negative cache: changed ignore rules are queried again. Reads check both requested and resolved paths, directory mutations inspect descendants, and writes recheck before the final mutation. Status/diff and Git mutations retain observed hidden paths even after the host force-stages them. This does not provide descriptor-relative isolation against concurrent filesystem replacement or freeze the Git index/attributes; those remain separate work.

Legacy token reads share the asynchronous-session file policy and release the authority mutex for I/O, then revalidate the live invitation, permitted project/root set and privacy setting. They no longer fall back to an unrelated project when an invitation has no remaining project. New-file resolution rechecks canonical parents and existing targets against private/secret-name restrictions.

The pure secret-name policy and example exemptions are compared with synthetic vectors extracted from the shipped Vue source. Ordinary non-secret dotfiles are no longer hidden solely for starting with a dot; unconditional private host paths remain more restrictive.

Fidelity clarification confirmed with the Vue maintainer: `--no-index` is used only for the commit guard. Tracked files matching ignore rules remain visible through ordinary file/Git reads unless their name is secret or the share already observed them as ignored. The direct no-index commit check must not add new entries to sticky history. This preserves deliberately tracked fixtures/build output and the source's read/write distinction.

Validation: 106 Live Share integration cases pass serially on the final code, including a Git-ignored filename containing brackets, sticky history after rule removal and setting toggles, legacy reads, all four file mutations, force-staged status visibility and commit refusal without polluting history, and canonical junction-parent checks for both secrets and Type-gated configuration (with read-only metadata and flagged audit). The secret-name reference covers 163 cases. Strict all-target/all-feature Clippy, formatting and diff checks pass. Native visual parity, per-project hooksPath discovery, version conflicts and descriptor/index transaction boundaries remain pending.

## Dynamic project Git hooks scope (2026-11-15)

`share/hook_paths.rs` reads the original merged configuration with origins, without the execution-time core.hooksPath override. The scrubbed bounded runner shares a ten-second deadline across configuration and repository-root discovery; configuration origins/includes are validated before using the value. Last effective values, repository-relative/absolute folders, canonical existing prefixes, missing folders and shared subprojects are handled. Discovery errors refuse the operation; they do not silently remove protections as the Vue fallback can.

Read-only file metadata, requested/resolved mutation paths and directory descendants use the dynamic scope. Mutations capture the scope outside the host authority mutex and rediscover it before their mutation boundaries; a changed scope refuses the operation. This detects configuration changes during preparation but is not descriptor-relative filesystem isolation or a transaction over host configuration. The fixed eight-hook-name over-protection is removed: root hook names are protected only when the hook folder is the project root, matching the reference policy.

Validation: 110 Live Share tests pass serially (67.44s), including the isolated real Git hook scope, root-only hook-name gating, Type-authorized flagged writes, absolute and missing folders, malformed/truncated configuration, and an injected host configuration change before persist that preserves the original file. Strict all-target/all-feature Clippy, formatting and diff checks pass; Halley confirmed the final read-only review. Version conflicts, descriptor-relative access, Git attribute/index transactions and full native UI parity remain separate work.

## Guest content-version conflicts (2026-11-15)

The compatibility host returns a SHA-256 hash of the actual bytes read. Guest saves submit expectHash from the exact acknowledged baseline; existing files require a matching content hash, including same-size changes, and a deleted baseline is not silently recreated. A bounded read checks the version before temporary-file creation and immediately before replacement. New files use persist_noclobber. A nonblocking save mutex serializes guest saves without holding the authority mutex. Conflicts keep the guest draft and show a localized explanation; subsequent edits retain the acknowledged baseline rather than adopting unsaved text.

This is optimistic conflict detection, not an OS compare-and-swap or a descriptor confinement claim: an external host writer can still change the destination after the final check. The existing descriptor/race card remains open. Older compatibility clients that omit expectHash can no longer replace an existing file. The UTF-8 baseline is byte-preserving (including BOM and CRLF); invalid UTF-8 is refused, and the editor hashes the acknowledged baseline rather than live widget text. Validation: 111 Live Share tests and 11 editor tests pass serially, including same-size edits, deletion, a destination appearing before persist, growth beyond 8 MiB, BOM/CRLF baselines and edits made during a successful save. Strict Clippy across all targets/features, formatting and diff checks pass. Halley completed the host and editor read-only review.

## Git-native configured hook path expansion (2026-11-15)

The validated last effective core.hooksPath value is expanded by Git config --path --null --get, pinned through an argv -c override to avoid adopting a new value between discovery and expansion. The existing scrubbed runner preserves trusted HOME and shares the original ten-second discovery deadline. No shell, hook execution or fsmonitor invocation is involved. Leading spaces in quoted values are retained. Truncated, malformed, non-UTF8, unterminated, empty and overlong results fail closed. Canonical scope, Type gating and final scope revalidation remain unchanged.

Synthetic isolated-HOME coverage checks an expanded hook directory with spaces, a different literal-tilde directory, read-only metadata, refused writes without Type, flagged authorized writes, invalid config and broken .git indirection. A separate repository-detection gap for bare repositories is tracked; this change does not claim to resolve it. Validation: the full serial suite passed 113 cases with one failing fixture; after aligning its root canonicalization with Host::start_multi, that remaining isolated real-Git probe passed (1/1). Product code was unchanged during the fixture correction. Strict all-target/all-feature Clippy, formatting and diff checks pass. Halley approved the product delta and corrected fixture.

## Verified Git worktree scope for guest file access (2026-11-15)

A fixed bounded Git rev-parse command classifies the root from its worktree/bare/inside-git-dir flags and top-level path. The existing process capture is exposed internally with exit status and both streams; ordinary Git and check-ignore retain their existing nonzero-exit policies. Repository classification alone uses an English locale and recognizes an ordinary directory only from exit 128, empty stdout and the exact complete standard no-repository error. Broken gitdir links, dubious ownership, metadata directories, bare repositories, missing Git and incomplete or ambiguous responses refuse guest access.

HostGit shares its original deadline across this classification and config/path expansion. Hook discovery and ignore privacy use the same proof instead of ancestor .git heuristics. Mutation entry and final path checks run discovery regardless of Edit/Type and hide-ignored; directory listings also verify scope when hide-ignored is disabled. No repository metadata or hooks are executed by this probe. This checks the shared root; bare repositories nested inside an otherwise ordinary shared folder are a separate tracked boundary requiring validation and protection. Validation: 117 Live Share tests pass serially (127.30s), and 13 local Git runner/repository tests pass (23.60s). The isolated repository probe also passes after explicitly setting its source locale to French; only test environment setup changed after the full suite. Strict all-target/all-feature Clippy, formatting and diff checks pass. Halley approved the root-scope delta. Native visual parity, descriptor confinement and index/attribute transactions remain open.

## Nested Git metadata boundaries (2026-11-15)

A failing synthetic test first demonstrated that b238 exposed repos/service/config beneath an ordinary worktree with hide-ignored disabled and Type enabled. The new filesystem-only guard treats a directory containing HEAD, objects and refs (ASCII case-insensitive names, any entry types or contents) as private, without reading config/core.bare. This matches the policy coordinated with Darwin for Vue; a *.git name alone is not private. The signature is intentionally conservative and may hide an unrelated folder that has all three names.

Per-operation scans inspect each listing entry, requested ancestors and canonical ancestors, including an alias that jumps directly into hooks/. Parent directory mutations inspect descendants. Scans cap directory entries, elapsed time and path depth and retain no negative result between operations. Destination checks also consider the proposed marker, and marker-forming create/write/rename operations use a nonblocking process-wide mutex so two guests cannot jointly complete the signature. Fresh final mutation checks remain in place. This does not resolve external host filesystem races or descriptor-relative confinement.

SCM status and global diff merge metadata privacy independently of hide-ignored. Commit checks reject metadata staged by the host; this does not add metadata results to sticky ignore history. Deleted paths use their existing ancestors without simulating marker creation, preserving ordinary deletion status. These filesystem checks do not classify metadata from a fully removed historical tree and do not replace the pending index/attribute transaction work. Guest discard currently refuses missing paths rather than restoring deletions; future restoration parity must check the first restored missing ancestor and serialize marker formation before checkout.

Validation: 123 Live Share tests pass serially (193.96s), including the genuine Windows junction, real Git status/diff/commit after metadata was staged before completing the signature, deleted ordinary SCM paths, marker concurrency, case/type variants and refusal budgets. An earlier run had a junction-fixture construction error and a duplex timeout; the corrected full run passes both cases. Strict all-target/all-feature Clippy passes (1m12s), along with formatting and diff checks. Halley approved the file and SCM changes. A separate real-Git fixture with init --bare --ref-format=reftable produced HEAD/config/objects/refs/reftable on the installed Git, so that backend also matches the signature here. No hooks were executed, and all files and browser-independent environment values were synthetic.

## Standalone GPUI past conversations (2026-11-15)

Compatibility already had show_sessions_modal and the sessions:modal palette command. GPUI now exposes the same modal flag, renders a 720x620 card bounded by the viewport, and reuses its history renderer and native query entity. History rows and pending-query repaint continue with the side panel closed. The sidebar suppresses its duplicate History body while the dialog is open so the same native entity is not rendered twice. Modal query scopes differ from side-panel scopes, refuse edits under another overlay, and invalidate delayed edits on close. An empty-query Escape closes the dialog and returns focus to the root. Existing profile identity, bounded discovery, resume and recycling authorization stay shared. Native visual acceptance remains pending; this change does not claim complete modal parity.
Validation: 13 bridge panel/history tests pass, plus all six native GPUI panel-query fixtures (including the new modal focus test with both sidebar states). The focused visual-test run also passes both bridge modal cases and the native modal test. These are synthetic TestAppContext render/input checks, not a manual visual comparison against Vue.
Strict all-target/all-feature Clippy passes (1m10s). Halley reviewed the shared state, query generation, overlay blocking, single input rendering and native focus fixture.

## Linked Git metadata signature (2026-11-15)

Darwin's Vue review identified another Git layout: HEAD plus commondir is sufficient to point at a common object/ref/config store without local objects or refs entries. The structural guard now recognizes HEAD AND ((objects AND refs) OR commondir), regardless of marker case, type or content. It does not read or follow commondir contents. Both scan completion and prospective mutation checks use the same predicate; commondir participates in the existing process-wide marker mutex. Ordinary folders containing only commondir remain accessible, while creating HEAD there is refused.

Validation: the full serial Live Share suite passes 126/126 (204.78s), including all sixteen marker combinations, prospective formation in either order, marker concurrency, and a real Git linked layout under an ordinary worktree. The descriptor and index/attribute transaction limitations remain open.

Strict all-target/all-feature Clippy passes (1m07s) on the final tree, including the submodule follow-up. Halley approved the signature and concurrency changes.

## Guest Git submodule recursion (2026-11-15)

The fixed guest Git prefix now forces submodule.recurse=false independently of discovered configuration. A path-limited discard must not restore files inside an initialized submodule because the host enabled recursion. A synthetic positive-control fixture demonstrates that ordinary checkout with host recurse=true overwrites the submodule's local edit, then checks that the guest request preserves both that edit and its HEAD. Local file transport is enabled only in fixture setup.

Validation: all 27 guest Git host tests pass (9.97s), including the new real-submodule fixture. This follows the 126-case serial Live Share run for commondir; that product code was unchanged. Strict all-target/all-feature Clippy passes on the final combined tree (1m07s), formatting and diff checks pass, and Halley approved both deltas.

## Worker launch identity and cancellation (2026-11-15)

The UI previously used sessions.last_mut after requesting a worker pane, so a session-limit or identifier-exhaustion failure could rename and bind an unrelated existing pane. Session creation now returns the freshly created ID or None, and the worker binds only that ID. Failed creation marks the pending worker Failed without changing an existing session. Stop/release removes queued spawn requests; dequeue and binding also require the original Starting state, frozen project/team/coordinator context and current runtime coordinator. The chosen launch root must match the frozen working folder; an existing matching coordinator worktree is preserved. Unsupported or changed roots fail instead of falling back to the project folder.

Differing absolute folder spellings are resolved with dunce::canonicalize on both sides of the launch comparison, so a restored Windows path without the extended prefix can match a canonical project/worktree root. Relative paths and failed resolutions are refused. This is a launch-time consistency check, not a descriptor-based defense against filesystem races.

The initial worker brief still needs runtime delivery and is tracked separately. This does not complete NewTask or claim a running CLI is healthy merely because its pane was created. Validation: 42 worker-related tests passed; after strengthening the changed-folder case to use a different existing directory, both runtime worker-launch tests and all six teams_relay tests passed serially. Strict Clippy across all targets/features, formatting and diff checks passed. Tests use synthetic projects and dormant native chat panes, without launching real agents or connecting to providers.

## Worker assignment mailbox (2026-11-15)

A successful initial worker binding now deposits the frozen assignment in that worker's team inbox, with its coordinator and team identity. The team label is frozen at request time too; renaming the team afterward does not change the assignment. Duplicate binding, late binding after cancellation and repeated inbox reads cannot replay the brief or send it to another pane. The text is mailbox data and is never injected as terminal input.

Worker and relay polling previously ran only inside Compatibility's UI update. It now runs once in the shared application poll, so GPUI consumes the same pending launches and requests a repaint when a launch succeeds or fails. The runtime tests go through poll_for_gpui rather than calling the relay helper directly, and a second frame checks that no extra pane or assignment is created.

This closes the dropped-assignment and missing-GPUI-poll paths, but does not implement automatic first-turn submission or a terminal/Claude team-tool transport; only native Codex chat currently exposes the dynamic team tools. Those prerequisites remain tracked before NewTask can be called complete. Validation: all 15 panel snapshot tests and all six relay tests passed serially on the final tree; strict Clippy across all targets/features, formatting and diff checks passed. Fixtures use temporary workspaces and dormant Chat panes, without real agent processes or provider connections.

## Native Codex worker first turn and stopping (2026-11-15)

Native Codex workers now track Pending/Connecting/Submitted/Cancelled/Failed first-turn status independently of pane creation. The shared loop revalidates coordinator, team, project and working directory before connecting or sending. Account readiness uses the normal isolated native-profile path. A single connection attempt and a 30-second initialization deadline bound the automatic startup; failed or uncertain sends are not replayed. A fixed initial prompt directs the worker to its frozen mailbox assignment through turn/start, with the existing user-reviewed workspace-write sandbox. It neither consumes nor replaces the owner's composer. A chat that already received a user turn takes precedence over the automatic first turn.

team_worker_stop previously changed only the model state, while the UI's sleep helper affected only terminal panes. Stop now drains through the shared runtime loop and stops native Codex/Claude connections as well as terminal creation/restoration in progress. Release suppresses an unsent automatic first turn without stopping an independent chat. Restored panes do not get automatic first turns: orchestration state is runtime-local. Submitted means dispatched to the protocol, not successful task completion; later protocol errors remain visible in the chat. CLI/Claude team transports and their startup still remain separate migration work.

Validation: 16 panel/runtime tests, 15 Codex chat tests, six relay tests and four agent-tool tests passed serially; strict Clippy across all targets/features, formatting and diff checks passed. All new bootstrap fixtures use in-memory protocol senders, synthetic workspaces and no real provider processes, credentials or network. The team-tool catalogue is now extended outside the Windows-only browser block; non-Windows availability was checked structurally, not through a non-Windows runtime test.

## Pane signing keys and replay-cache saturation (2026-11-15)

Pane key creation now returns an error if secure randomness fails and never inserts a zero or partially filled fallback key. Worker startup requires successful key issuance before creating a child pane; a later pane-creation failure revokes the reserved key. Registry Debug output includes only counts, and the temporary random-byte buffer and stored keys on revocation or drop are zeroized.

Replay entries retain their signed timestamp expiry. Reclamation removes only entries already outside the accepted signature window, including expired entries behind a later-expiring front entry. A cache full of unexpired nonces refuses new authenticated requests instead of evicting replay protection. This deliberately prefers temporary refusal over accepting a captured request again.

These are prerequisites for the pending CLI/Claude transport, not a claim that its runtime listener is implemented. Separate work remains for entropy-failure handling in guest identities and generated team/account/automation/custom-agent identifiers. Validation: all eight relay/auth tests and both worker-launch runtime tests passed serially. Strict Clippy across all targets/features passed; fixtures simulate partial entropy failure and replay-cache saturation without provider processes or network.

## Guest identities and managed-account allocation (2026-11-15)

Live Share now creates a guest identity lazily when joining, only after secure randomness succeeds. Both native GPUI and Compatibility use the same join path. Failure preserves the invitation, displays an error and starts no guest connection; a successfully generated identity is reused for later joins in the same panel lifetime.

Managed-account allocation also fails before filesystem changes when randomness fails. Existing roster identifiers are refused, and the account directory is reserved with exclusive creation so an orphan profile cannot be silently reused. This does not implement transactional roster updates across multiple application processes. Team/custom-agent/automation identifier generators remain a separate follow-up. Validation: the guest failure test, managed-account failure/collision test and normal account/provider-secret roundtrip test passed. Strict Clippy across all targets/features, formatting and diff checks passed. All fixtures use synthetic invitations and temporary profiles; no guest connection or real credentials are used.

## Fallible team, custom-agent and automation identifiers (2026-11-15)

The remaining three identifier generators now propagate secure-randomness errors. Team creation and its tool callers refuse failed or duplicate identifiers. Custom-agent creation checks existing identifiers before dispatching an upsert, while editing keeps its original identifier. Both automation forms reject a generated identifier already present in their store; GPUI uses the fixed auto prefix, keeping user labels (including spaces, accents and slashes) out of identifiers. Errors preserve form drafts and are surfaced before creation.

Automation runs reserve their directory exclusively and create prompt.md with create_new. A duplicate or orphan run directory is rejected without overwriting an earlier prompt, and failed allocation never reaches agent launch. These guards cover local allocation and collision refusal, not transactional multi-process roster/workspace saves. Validation: nine team/relay tests, one custom-agent identifier test, four automation tests and one GPUI creation test passed (15 total). Strict Clippy across all targets/features, formatting and diff checks passed. No provider process was launched; all saved files are synthetic temporary fixtures.

## Team MCP stdio protocol foundation (2026-11-15)

A standalone protocol module now implements bounded newline frames and output encoding, handshake negotiation for explicitly enumerated 2024/2025 MCP versions, ping, routed tools/list and tools/call, cancellation, and a bounded pending-request set. Internal monotonic tickets prevent late completions from being delivered to a newer request that reused a JSON-RPC id. Unknown/malformed protocol requests are distinct from tool execution failures. Incoming text and tool arguments confer no authority in this module.

This is not an enabled MCP server: no helper entrypoint, listener, provider launch, broker dispatch or tool execution is wired yet. Read deadlines, socket authentication, pane-generation bindings and runtime launch integration remain subsequent slices in MCP_TEAM_TRANSPORT_PLAN.md. Protocol fixtures use only in-memory streams and synthetic values. Eight tests passed, covering version negotiation, invalid messages, framing/output limits, catalogue/tool errors, saturation, cancellation and stale completions. Oversized results, including JSON escaping expansion, produce a bounded error warning that the operation may already have completed and must not be retried automatically. Strict Clippy across all targets/features, formatting and diff checks passed.

## Local team broker and launch capabilities (2026-11-15)

The next transport foundation adds a loopback-only broker with a single ephemeral bind, bounded concurrent peers and dispatch queue, length-prefixed packets capped at 1 MiB, and absolute packet/exchange deadlines. Disconnected peers and extra input during the one-request connection are rejected using a nonblocking probe. Shutdown closes partial and pending connections, revokes capabilities and joins the worker threads. Networking threads do not execute tools or touch Workspace/TeamsRelayHub.

Each capability issuance revokes the earlier key for that pane before allocating a checked monotonic launch generation. The runtime-only binding carries trusted pane/project/cwd values; request arguments cannot replace them. Authenticated requests reuse the existing replay protection and additionally require transport-domain, generation, fixed nonce and short freshness checks. Signed responses use a distinct domain and bind the original nonce and request digest. Oversized results become a bounded signed failure with an explicit uncertain-outcome warning. Credentials use redacted formatting and zeroizing buffers; registry keys are zeroized on revocation and drop. This does not claim that all OS or environment copies are erased.

The broker returns pending operations for later main-thread dispatch. It rechecks the stored binding, peer and deadline before delivery/completion; comparing that binding to the real current app pane, project and cwd remains slice 3. No helper entrypoint, GUI activation, child-environment wiring or provider process is enabled yet. Same-user compromise remains outside this capability boundary, as documented in the transport plan. Socket reads and writes use nonblocking I/O with short cancellable waits under an absolute deadline: shutting down a cloned Windows socket did not reliably interrupt a blocking receive. Expired queued requests explicitly shut down all socket clones, and client cancellation stops an exchange without retrying it. Validation: all 23 MCP protocol, auth, broker and wire tests passed, including cancellation, expiration before dequeue and shutdown with partial requests. Strict Clippy across all targets and features passed. Fixtures use synthetic loopback peers only; no provider processes, credentials or external services are used.

## Main-thread team MCP dispatch (2026-11-15)

The shared application poll now has a dormant team MCP runtime. It constructs scope from current local sessions and actual transport/spawn activity rather than restored session records, then dispatches at most eight operations per tick through the existing persisted pane-scoped tools. The catalogue uses an explicit team-tool allowlist and excludes browser/credential APIs. Existing worker and inter-project relay approval gates remain in the underlying dispatcher. Closing or sleeping a pane revokes its capability, and queued worker stops take precedence over pending MCP mutations.

A launch owner revokes only its own generation when dropped, so an old transport cannot revoke its replacement. Working directories are resolved at issuance and checked against the current resolved directory before dispatch; missing, changed, remote, dormant, duplicate or removed-project scopes are rejected. These checks do not claim descriptor-relative protection from filesystem races. Responses remain bound to the request, and cancellation after mutation begins remains an uncertain outcome without automatic retries.

No production launch currently calls Runtime::begin: there is still no enabled provider MCP server, helper entrypoint, child-environment wiring or CLI/Claude first-turn integration. Test fixtures alone activate the listener. Per-child adapters must issue immediately before actual spawn, retain the launch owner in that transport, and revoke it on cancelled asynchronous startup/reconnect. Task-tool names currently retain the Rust tessel_task_* catalogue; Vue team_task_* API parity remains separate work. All 30 MCP tests passed on the final tree, including nine cases through the actual shared application poll (a retained exited PTY and a queued worker stop included). The 22 worker-filter regression tests passed before the final fixture-only additions. Persisted task readback and failed-save atomicity are checked using temporary workspaces. The PTY fixture runs only a fixed exit command, not a provider CLI. Strict Clippy across all targets and features passed.

## Dedicated team MCP stdio helper (2026-11-15)

The executable now has a dedicated --team-mcp-stdio mode before GUI initialization. It imports only a strictly validated IPv4 loopback endpoint and per-launch pane/generation/key from child environment, never a project or cwd supplied by the client. Missing or invalid configuration exits without opening the GUI or emitting protocol stdout. Fatal errors on this entrypoint use only a nonzero exit status: no diagnostic write to a potentially full stderr pipe can block shutdown or expose credentials or request data. Export applies transport values only to a Command's environment; provider adapters must call it after normal profile/environment setup and must never log that Command or forward these values to SSH/host-daemon launches. OS/environment copies are not claimed to be erased.

The helper performs the existing MCP handshake, then forwards catalogue/tool calls through the authenticated broker. At most four exchanges are active; cancelled jobs retain their capacity until completion, and reused JSON-RPC ids cannot receive older completions. Input chunks, frames, output queue and pending output acknowledgements are bounded. Partial input and blocked output have absolute ten-second deadlines, while an incomplete initial handshake has thirty seconds. Blocking pipe operations live in isolated helper threads; returning from the loop immediately exits this dedicated process instead of joining blocked pipe threads. Idle initialized stdin has no artificial inactivity timeout.

The catalogue boundary is normalized: Runtime now returns the array expected by Session::complete, which produces the MCP result.tools object. Acceptance fixtures launch only this executable, synthetic loopback brokers and temporary workspaces. No production provider calls Runtime::begin yet, and no Claude/terminal adapter, environment forwarding or automatic first turn is enabled by this slice. The process acceptance fixture targets the debug executable on Windows; release-subsystem and other-platform process acceptance remain to be verified before claiming those combinations. Validation: all 33 MCP unit/runtime tests passed, followed by the actual-executable acceptance fixture (21.89 seconds on the final tree) and strict all-target/all-feature Clippy. The process fixture checks a persisted task roundtrip, the catalogue wire shape, revocation, cancellation with reused ids, missing environment with empty stdout/stderr, partial input timeout and nonreading stdout timeout. No real provider or external service was invoked.

## Claude startup cancellation checkpoints (2026-11-15)

Native Claude startup now checks a cooperative cancellation token after transcript validation and after account/profile application, immediately before constructing the child process. Stopping a connection while validation is blocked prevents the remaining spawn step. A cancellation racing the final check and OS spawn is still handled by dropping the undelivered transport owner; this is not an atomic no-spawn guarantee. The existing non-cancellable-work wrapper remains available to other native providers.

This does not activate Claude MCP or change its permission, profile or safe-mode flags. The current official Claude CLI documentation says safe mode disables MCP servers. Bare mode supports explicitly configured servers but does not read subscription OAuth credentials, so it is not a transparent substitute. Provider integration remains unfinished until a compatible, reviewed configuration is established. References: https://code.claude.com/docs/en/cli-reference and https://code.claude.com/docs/en/headless#start-faster-with-bare-mode .

Validation of the cancellation delta: two deterministic native-connection tests and nine Claude protocol/permission tests passed. Strict all-target/all-feature Clippy passed with warnings denied. No provider process or external service was used.

## Canonical board listing and moves (2026-11-15)

The dynamic tool catalogue and dormant authenticated MCP catalogue now include team_tasks and team_task_move. These adapt the existing persisted task implementation: identifiers are returned and accepted as canonical decimal strings; columns are todo/doing/review/done. The optional me field is descriptive only and cannot select a different pane or project. Unknown keys, malformed identifiers, unsupported columns and non-string me values are refused before dispatch. The existing tessel_* API remains unchanged.

Listing returns structured tasks, including an assignee name from the same project's session or a pane-number fallback. This is not yet the grouped text presentation of Vue's team_tasks. team_task_add with named assignees/dependencies, structured completion reports and full gate/dependency semantics remain separate unfinished migration work. The aliases do not activate any production MCP provider. Tests exercise the actual persisted dispatcher and authenticated broker, including reload from disk, cross-project refusal and storage-unavailable rollback.

Validation: six agent-tool tests and six authenticated runtime tests passed on the final tree. Strict all-target/all-feature Clippy passed with warnings denied (40.69 seconds). The initial readback fixture attempted to open a second Store and correctly hit its exclusive lock; the final fixture reads and parses the saved workspace file without reopening the store. No provider process, credentials or external service were used. Halley approved the production delta in read-only review.

## Persistent task prerequisites (2026-11-15)

Task.deps now stores up to ten prerequisite identifiers and defaults to empty for existing workspaces. Workspace validation rejects duplicate links, self-links, zero/future identifiers, links to existing cards in another project and cycles. Missing cards remain unresolved blockers rather than being treated as complete. Topological validation is iterative, including a ten-thousand-card chain fixture; listing uses a shared index rather than scanning the full task list per edge.

Both canonical and legacy agent move operations refuse advancement to Doing, Review or Done while prerequisites remain unresolved. Same-column requests and return to To do remain allowed. Manual board operations retain the user's override, matching Vue. team_tasks exposes string deps and waitingOn identifiers. Restore validates the source graph and assigns fresh identities to both cards and missing references, reserving missing identities so future cards cannot accidentally satisfy an orphaned prerequisite. Other task constructors default to no dependencies; the standalone orchestration test model was updated too.

This is the persistent dependency foundation, not completion of team_task_add or orchestration parity: named assignment, creation/edition of links through tools/UI, ready notifications and reports/gates remain unfinished. No production provider MCP transport is activated.

Validation: 21 distinct focused tests passed (22 executions across dependency, history and agent-tool filters), including original task/history persistence rollback and the new restore/missing-link fixture. Strict all-target/all-feature Clippy passed with warnings denied (53.72 seconds); formatting and diff checks passed. No real provider or external service was invoked.

## Bare-repository policy regression (2026-11-15)

Darwin's Vue SSH audit identified safe.bareRepository=explicit as a case where Git refuses implicit bare-repository discovery. A new isolated regression sets this policy only in a temporary synthetic HOME, proves that raw Git rejects the bare directory with that policy, then runs the existing Rust scope and Live Share read/mutation refusal matrix with hide-ignored enabled and disabled. Both the default-policy and explicit-policy probes passed (4.34 seconds total). Rust's existing classification treats ambiguous/refused Git scope as an error rather than an ordinary folder, alongside its content-signature metadata scan. No production change or user Git configuration change was required. This evidence covers the local Rust guest handlers; it does not claim native SSH-host parity or atomic filesystem race protection.

Strict all-target/all-feature Clippy passed with warnings denied (43.15 seconds), as did formatting/diff checks. Halley approved the regression. README is unchanged because this adds test coverage only, without user-visible behavior changes.

## Stable legacy pane-number addresses (2026-11-15)

Vue App.vue numberPanes preserves unique positive numbers per workspace and fills missing/duplicate numbers with the smallest free value. These numbers are legacy aliases, not internal pane IDs or indices in a team. SavedSession now has an optional legacy_number; existing workspaces normalize missing values on load and persist them on the next ordinary save. Runtime capture retains numbers by pane identity before normalizing newly created sessions. Closing a pane can free its number for reuse, matching Vue; unrelated existing numbers are not renumbered.

team_members exposes number separately from pane. Team messages resolve #n through the project-scoped numbers, while pane:id remains an explicit internal identifier. Inter-project relay refuses #n when more than one eligible session matches across projects. Canonical board listing uses the current alias for an unnamed existing pane and labels a missing assigned pane as former instead of inventing an actionable #id. Pure numbering, saved-workspace reload and real message routing fixtures cover different internal IDs and numbers. This lays the address foundation for named/numbered team_task_add assignment; that creation tool and dependency-ready notifications remain unfinished.

Validation: 46 focused test executions passed (numbering/reload, relay, persisted agent tools and standalone orchestration, including repeated shared-module tests). Strict all-target/all-feature Clippy passed with warnings denied (49.32 seconds); formatting and diff checks passed. The first reload fixture used an unnormalized temporary cwd and was corrected to the project's canonical path before the passing run. Halley's independent review remains pending.

Halley subsequently confirmed commit-equals-review for b257 (f67204f), approving the numbering allocator, malformed-team refusal, persisted aliases and scoped board fallback.

## Canonical task creation (2026-11-15)

team_task_add now resolves a project agent by name or saved legacy number before storing its internal pane id. A caller enrolled in a team can assign only that team's project agents; a project without a team uses its project agents. The bound caller is the default assignee, independent of the descriptive me label. Titles collapse JavaScript whitespace and enforce 200 UTF-16 code units. after accepts up to ten existing project card ids, deduplicated; unknown or cross-project prerequisites are rejected. An unfinished prerequisite forces the initial column to To do and returns the effective column, waitingOn and deferred status.

Creation, assignment, prerequisite links, identifier allocation and history are built in a transaction draft and saved before publication or success. Tests cover authenticated broker dispatch, reload, malformed/ambiguous/foreign addresses, Unicode limits, prerequisite enforcement and storage failures. The broker catalogue exposes the new tool without activating any production provider MCP launch. Existing native Codex dynamic tools use the persisted dispatcher.

This does not complete the parent orchestration card: createdBy/report metadata, pending decision gates, dependency editing in the UI and once-per-ready-transition notifications still need implementation. The board remains project-scoped and responses remain structured JSON; no claim of identical Vue text formatting is made.

Validation: nine agent-tool tests and six authenticated-runtime tests passed. Strict all-target/all-feature Clippy passed with warnings denied (48.08 seconds), plus formatting/diff checks. Halley approved the code, including bound caller authority, stored pane identity, existing prerequisites, forced To do and the draft transaction.

## Dependency readiness notices (2026-11-15)

Task.last_waiting stores the last observed prerequisite set, bounded to ten identifiers and defaulting empty for old workspaces. Canonical creation seeds it with current blockers. The main team poll compares dependency state using one index, saves changed markers transactionally, then enqueues a system mailbox notice for a nonempty-to-empty transition only while the card remains To do and its assigned pane belongs to a team in the same project. Missing prerequisites stay blocking; unassigned, closed and non-team panes receive no notice. Notices retain Live Share guest provenance and explicitly identify titles as untrusted content. Reopening and completing a prerequisite permits a new notice; repeated refreshes and saved-state reloads do not replay the old transition. Restore clears imported readiness markers alongside assignment/timing identity.

This follows Vue's observed waiting-key transition model. It does not provide a durable notification outbox: a process crash after saving the marker and before consuming the in-memory mailbox can lose the notice. Persistence failure leaves the original marker and mailbox intact for retry. Pending decision gates, task report/creator metadata and dependency-link UI remain separate incomplete work.

Validation: all nine agent-tool tests passed in the initial readiness pass. The final delta passed four readiness tests and eleven task-history tests, including explicit foreign session/team refusal and clearing imported markers. Strict all-target/all-feature Clippy passed with warnings denied (29.92 seconds); formatting and diff checks passed. Synthetic fixtures only; no provider process or active Vue application was changed.

Halley approved the final readiness structure and validation. Rust Task does not yet model Vue copiedFrom provenance; notices include guest_by plus the generic untrusted-title label, and restored tasks clear assignment/readiness state. Copied-settings provenance remains an explicit parity gap.

## Persisted completion reports (2026-11-15)

Task.created_by records the bound creator for canonical/legacy tool creation. Task.report retains outcome, a summary bounded to 2000 UTF-16 units, up to fifty 300-unit file strings, the bound reporter and a timestamp. team_task_done rejects identity/project/host overrides, stores reports on a draft, and moves succeeded cards to Done only when dependencies and the current project's unresolved review gates permit it. Failed or blocked reports retain their column. Canonical and legacy persisted agent moves now respect those existing gates too; same-column requests, return to To do and explicit manual board moves remain allowed.

Only after saving does the hub notify the report's giver, scoped to the same current team/project; the reporter also receives an explanation when success is blocked. File strings and report/title text are labelled untrusted and never opened or executed. Restore preserves report content but clears reporter and creator pane identities so imported identifiers cannot target unrelated local agents.

This slice implements local report storage/tool responses and mailbox notices. Native board report rendering, remote-host report attribution, copied-settings provenance and canonical persisted gate questions/options/user answers remain incomplete. Existing review gates are still memory-only; blocking them during the live session is not a claim that decisions survive restart. Mailbox notices also retain the already documented crash-delivery limitation.

Validation: ten agent-tool tests, eleven history tests and six authenticated-runtime tests passed. The report fixture covers both blockers, manual override, failed outcomes, persisted reload, spoofed/malformed fields and write failure without messages or state mutation. Strict all-target/all-feature Clippy passed with warnings denied (1 minute 36 seconds); formatting and diff checks passed. Halley approved the structure and both move/report guards.


## Persisted task decisions (2026-11-15)

Task.decision stores a unique request identity, question, up to six options, asking pane and optional user answer. Canonical team_task_gate and legacy task_id/note calls share the persisted transaction dispatcher; direct in-memory mutation is refused. team_tasks exposes gate/createdBy aliases, and team_gates includes saved decisions. Pending questions survive restart and block agent advancement/completion. Only the host UI answer path can resolve them, after rechecking active project, card, current request and pending state; storage commits before the asking teammate is notified.

GPUI provides native editable freeform text, explicit submit and keyboard-accessible choices. The Compatibility renderer provides equivalent answer actions in a decisions section above the board. Blur never submits. Both use the same authoritative persistence path. Restoring settings remaps question identities, clears old asking-pane attribution and discards imported answers so a fresh user decision is required.

This supersedes the preceding slice's memory-only decision limitation. Native report presentation, guest Live Share answers/provenance, attention notifications, copied-settings provenance and exact visual parity remain unfinished. Mailbox delivery retains the documented crash-loss window; no durable outbox or production provider MCP activation is claimed.

Validation: 31 focused tests pass (four decision tests including two native GPUI cases and the Compatibility choice/disabled-UI case, eleven history, ten agent tools, six authenticated runtime cases). Strict all-target/all-feature Clippy, formatting and diff checks pass. Halley reviewed the final shared authority, persistence and renderer paths.
