# Native host integration (gate 7)

Status: approved by Darwin. The client, installer commands and release assets are now
published as host `v0.1.0` (`4354459`), following accepted gates 1–6 (`7358969`).
The desktop still pins the earlier protocol until client integration is validated.
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
  GitHub token. Release v0.1.0 is available for the first verified import.
- Use the published installer's `Plan.version` / `Plan.connect`, or
  `install::commands(version, arch)` without an upload. Pass the returned command as
  one SSH argument. Its fixed double-quoted path expands `$HOME` on the host; the
  desktop never reconstructs that path or accepts an arbitrary executable path.
- Expose granted capabilities and Connected/Reconnecting/Restarted/Closed states.
  Preserve last known content with an explicit stale marker. Missing capabilities,
  unsupported hosts or failed verification expose the distinct SFTP/SSH fallback.

## 7B — Persistent terminal panes

- Adapt the terminal emulator input/output boundary, retaining local ConPTY support.
  Apply byte output and resize events in order; snapshots preserve existing visible
  content in local scrollback, reset the screen and restore the supplied modes.
- Keep terminal ID, epoch and last applied sequence with its pane. Store lease keys
  only in encrypted local authority storage, excluded from backups, logs and guests.
- Gate input on connection and lease ownership. Show read-only/take-over, lost process,
  skipped-output and logout-persistence warnings; do not claim an old process survived
  a changed epoch. Starting a replacement is explicit, not an automatic restart.
- Existing SSH/tmux panes remain on their current backend until explicitly replaced.

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
