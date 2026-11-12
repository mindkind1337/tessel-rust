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
