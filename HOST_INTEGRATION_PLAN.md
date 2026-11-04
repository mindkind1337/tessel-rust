# Native host integration (gate 7)

Status: approved by Darwin. The client, installer commands and release assets are now
published as host `v0.1.1` (`3c459af`), following accepted gates 1–6 (`7358969`).
The desktop pins the client, installer and protocol at 3c459af. Gate 7A now has an explicit preview connection; terminal/editor/tree/Git adapters remain pending.
No real user server is part of unattended validation.

## 7A — One verified connection per host

- Add a background host manager keyed by the validated SSH alias and connection
  identity. Reuse the existing OpenSSH trust policy: strict host keys, no prompts
  hidden in a worker, bounded connect time, keepalives and no shared control socket.
- Use the published client connector with one exec channel for the daemon; never
  start a connection or upload from an egui frame. Bound queued work and cancellation.
- Probe architecture, validate the pinned manifest and exact local artifact bytes,
  upload through the installer plan, verify the remote digest, activate atomically,
  then verify version/architecture/protocol. Reuse a matching existing installation.
  No sudo, downloaded shell script, package manager or Node is required remotely.
- Artifact distribution is fixed: developer-authenticated `gh` downloads an explicit
  tagged host release; `cargo xtask host-sync <tag>` verifies it before resources are
  reviewed and committed under `resources/host/<tag>/`. Include both binaries,
  manifest, checksums and LICENSE. The desktop never downloads them or holds a
  GitHub token. The verified v0.1.1 bundle includes persistent input numbering, fresh snapshots and non-resuming detach.
- Use the published installer's `Plan.version` / `Plan.connect`, or
  `install::commands(version, arch)` without an upload. Pass the returned command as
  one SSH argument. Its fixed double-quoted path expands `$HOME` on the host; the
  desktop never reconstructs that path or accepts an arbitrary executable path.
- Expose granted capabilities and Connected/Reconnecting/Restarted/Closed states.
  Preserve last known content with an explicit stale marker. Missing capabilities,
  unsupported hosts or failed verification expose the distinct SFTP/SSH fallback.

## 7B — Persistent terminal panes

Preparation: local PTY ownership, bounded I/O and process-tree cleanup now live in `src/terminal/local.rs`, while `terminal.rs` retains rendering and emulation. The local path remains active; no remote terminal or persisted lease is enabled by this extraction. Its 17 terminal tests and strict Clippy pass on Windows.

- Adapt the terminal emulator input/output boundary, retaining local ConPTY support.
  Apply byte output and resize events in order; snapshots preserve existing visible
  content in local scrollback, reset the screen and restore the supplied modes.
- Keep terminal ID, epoch and last applied sequence with its pane. Store lease keys
  only in encrypted local authority storage, excluded from backups, logs and guests.
- Gate input on connection and lease ownership. Show read-only/take-over, lost process,
  skipped-output and logout-persistence warnings; do not claim an old process survived
  a changed epoch. Starting a replacement is explicit, not an automatic restart.
- Existing SSH/tmux panes remain on their current backend until explicitly replaced.

Desktop integration constraints from the client review:

- Use the 0.1.1 daemon bundle and a client with `Attached.last_input`; never persist/reuse a lease against 0.1.0. Missing or exhausted numbering must fail explicitly. Input delivery is at most once; an error after a long disconnection can leave delivery uncertain and must not trigger a new-ID resend.
- A newly started desktop has no emulator screen. Resume it with `attach_fresh` (mandatory snapshot), regardless of the stored last-applied sequence. Same-runtime reconnection may replay after the events delivered to its existing receiver. Advance the desktop's applied sequence only after emulation succeeds.
- Rebuild a parser from defaults for a snapshot; DEC margins, origin mode and alternate-screen state make an LF-based reset unsafe. Retain previous formatted history in a bounded separate desktop history view; do not claim preserved history until that view is implemented and tested.
- All potentially blocking terminal calls run on a terminal worker, with bounded commands and results. New input is refused during disconnection or without the writer lease. Do not send automatic cursor replies as an observer.
- Closing a pane must cancel its local follower and prevent future resume even for a silent shell, while leaving the server process alive. Ending the server process is a distinct explicit action. The host client's detach/Drop correction is a prerequisite to enabling this path.
- Persist only the alias, terminal ID, epoch and display metadata in workspace state. Lease storage is separate, encrypted, owner-only and excluded from portable backups; importing workspace metadata must never automatically attach or obtain control.

## 7C — Files, editor and Git subscriptions

- Tree: revision-checked per-folder differences, raw path identity distinct from label,
  pending-expand event buffering, stale relists, visible truncation/watch limits,
  and collapse/unsubscribe. Do not traverse symbolic links.
- Editor: retain the local dirty buffer and the exact read version/digest. Save against
  that base only; conflict/no-space/disconnection keep the buffer dirty. Never accept
  a new base and overwrite automatically. Do not silently retry a daemon write via SFTP.
- Git: use subscriptions, not per-frame status polling. Preserve stale lists and expose
  complete/stale/truncated/slow states; fetch untracked-folder details on demand.
- Revalidate host/root/document identity before applying every asynchronous reply.
  Changing hosts or projects must not publish a late result into another pane.

## 7D — Acceptance and publication

- Replay the maintainer's recorded sessions and malformed/timeout/epoch-change cases
  through synthetic connectors, then exercise bounded local Linux fixtures.
- Verify installer hashes and command arguments without any user host; test failure
  before activation, missing artifacts, unsupported architecture and fallback labels.
- Verify terminal replay/resize/snapshot ordering, lease loss, dirty-editor conflict,
  stale trees/Git and UI responsiveness through the actual adapters.
- Run focused tests and strict Clippy with two Cargo jobs. Package only verified host
  artifacts. Update README capabilities/limitations and report the pushed build.
- A real user-server acceptance run remains a separate, explicitly authorized step.

7B preparation in progress: bounded commands are in `src/terminal/host_commands.rs` (32 slots, whole-paste admission, preserved input/resize order, adjacent resize coalescing, queue invalidation, zeroizing input). `src/terminal/host_history.rs` retains display cells separately (1000 rows / 300,000 cells) and creates a fresh parser for a replacement screen. These modules are not yet connected to the terminal UI; dedicated tests accompany them. The native remote path remains disabled until the worker, lease store, rendering and compatible 0.1.1 bundle are integrated and validated together.

The 0.1.1 release bundle has been verified by `host-sync` (tag commit `3c459afab6d200035865fbf1e3a803d209fcd79b`); both bundled architectures match their published sizes and SHA-256 hashes. The installer keeps that release bundle; protocol/client libraries are pinned separately to `23b62ac09d70776d03fc4d91c2d8512c3c7addb8` for bounded snapshots, keyed creation and recovery leases. `host_lease_store.rs` supplies separate Windows DPAPI transactions for terminal authority, with bounded reads/writes, file locking and retained corrupt files. The terminal worker is still being integrated; these changes do not enable remote PTY panes by themselves.

Pane integration boundary (next implementation step): keep existing `Launch::Ssh`/tmux behavior intact and add explicit native-session metadata to saved panes. A native marker must suppress the legacy SSH auto-start path, including when an imported backup has no resumable terminal descriptor. Restoring or importing such metadata must not install/connect/take a lease automatically. Opening and attaching run on a worker with a reserved bounded slot before creating a server process; save a new lease before returning a ready pane. The live pane applies `Output`, `Resized` and `Snapshot` in stream order and advances its applied sequence only afterward. A worker's successful terminate acknowledgment changes input availability, but the UI continues draining output/Exit until the bounded grace period ends. Remote resize only changes the emulator once its ordered `Resized` event arrives. The prior-screen cell history is a display-only view and never replays escape sequences into the live parser.

Recovery adapter constraints (2026-11-04): creation intent is persisted before any remote create and shares the 256-record quota with terminal leases. The in-process token alone can replay its key, within a monotonic eight-minute window including the request timeout; restored pending metadata exposes no key. A generic opening worker reserves capacity before network access and hands ownership to the UI atomically. An explicit cancel can end only the newly created, identified program; dropping the pane or an adoption timeout detaches and preserves recovery information. A failed persistence write leaves the program alive with an explicit unsaved warning. The create response's lease must be preserved separately from the terminal object's current authority: a stream can be rejected before persistence observes the live object. A disconnected retry does not prove an epoch change, and a create_keyed refusal may come from the attachment after creation; neither case may discard the pending record.

The ordered event adapter applies at most 128 KiB and 64 events per frame. Snapshot decoding uses a replacement parser and never routes bytes through the live DSR reply path. Stream rejection drops the receiver; a final closed channel is distinct from temporary disconnection. The terminal integration MUST immediately drop its command Control when Feed::needs_detach() becomes true, including on an otherwise silent stream. The last screen/history stay readable, with no automatic recreation or reconnect after rejection/closure. These adapters remain under test and are not enabled in the UI yet.

Desktop wiring approach: keep the native opening lifecycle inside Terminal's backend rather than adding parallel opening/failure fields to every Session constructor. A native Terminal starts with a blank parser and an Opening backend, adopts Opened on poll, and exposes its current native descriptor for Session metadata persistence. The existing native_host marker continues to suppress legacy SSH launch. A native backend owns its UI panel, feed, optional command Control and recovery metadata; its explicit close actions are routed through the panel instead of immediately deleting the pane. Local backend behavior remains behind the existing facade and is covered by its current PTY tests. Native resize waits for the ordered Resized event. Any needs_detach state takes/drops Control immediately. Restore remains inert; a separate explicit Resume path connects and attaches with the owner's stored lease or as observer, never takes control implicitly.
