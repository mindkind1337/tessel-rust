# Native remote host plan

Status: design for review, not an implemented feature. Target: Linux x86_64 and
aarch64 musl executables in `~/.tessel-server`, installed without root. No changes
to the running Electron checkout. This is part of full Tessel parity, not a
replacement for the remaining desktop work.

Review: Darwin approved gates 1–3 on 2026-11-03 with the amendments below.
The protocol foundation now lives in `crates/host-protocol`; daemon, PTY,
installation and desktop transport integration are not implemented yet.

Ownership handoff: Halley takes gates 1–7; Hopper returns to desktop UI parity.

Gate 1 (protocol crate) is complete; the exact wire contract, error table and
compatibility matrix are in `crates/host-protocol/PROTOCOL.md`. It adds, over
the foundation: separate client frames (request, acknowledgment) and host frames
(reply ok/error, stream event, connection error); structured error codes with a
retryable flag and fixed details that never echo input; the core messages
(hello, ping, cancel); host-side admission (hello first, increasing identifiers,
capability by message family, bounded in-flight requests, exactly one reply,
cancellation); stream sequencing with a per-stream acknowledgment window;
deduplication keys that survive reconnects for 10 minutes and refuse rather
than forget when full; canonical base64 chunks (64 KiB) and raw-byte path
components; a pre-parse nesting limit; and integer-only JSON numbers within
±(2^53 − 1), a rule the fuzzer motivated (a fraction did not survive a resend).
Evidence, Ubuntu 24.04 WSL, Rust 1.99.1: 20 tests including seven golden
fixtures and the matrix row by row, strict Clippy, and a deterministic mutation
fuzzer (frames, bounded reads, connection rules, streams, chunks/paths,
deduplication model) run for 1,000,000 iterations in release mode
(`TESSEL_FUZZ_ITERATIONS`). Not run on Windows for gate 1; no daemon exists yet.
Gate 2 (daemon, bridge, rootless installer) is complete in `crates/host` and
`crates/host-xtask`. `tessel-host serve` keeps one daemon per user and version
under `~/.tessel-server/host/<version>/` (new directory, not `run/`): every
directory on the way is a real directory owned by the user, the daemon's own
ones exactly 0700, the socket 0600, a flock single-instance lock, peer UID
checked with SO_PEERCRED, a 10-second hello deadline, 16 clients at most, a log
of operation names and numbers only. `--daemonize` detaches (new session,
stdio on /dev/null) and its first process exits only once the daemon is ready.
`tessel-host connect --stdio` connects or starts the daemon, in a systemd user
scope when the user lingers, otherwise detached in the session, then relays
bytes with plain reads and writes (`io::copy`'s Linux kernel shortcuts held a
reply back). `host.info` reports version, architecture, pid and persistence
read from logind (KillUserProcesses through busctl or logind.conf) and the
linger file; nothing changes either setting. `install` is the client side:
`uname -m` to an architecture, a manifest with digests, and fixed commands
(`cat` to a nonce-named temporary file under `$HOME`, `sha256sum` or BusyBox,
`chmod 700`, atomic `mv`, then `version`), refusing symbolic links on the way.
`cargo run -p tessel-host-xtask -- release` builds both static musl programs
(aarch64 linked with rust-lld), refuses a dependency on a toolkit, web view or
JavaScript runtime, checks each ELF file (machine, no interpreter, no shared
library) and writes `manifest.json` and `SHA256SUMS`; `accept` runs each one.
Evidence, Ubuntu 24.04 WSL with systemd, Rust 1.99.1: 8 host tests (daemon
started once and alive after a SIGKILLed bridge with the same epoch, second
daemon refused by the lock, garbage refused without harm, a SIGKILLed daemon
replaced by another epoch over its stale socket, unsafe directories and a
non-socket at the socket path refused, the install plan run by `sh` with a
wrong program refused and a symbolic link stopping the upload), strict Clippy,
and `accept` passing for x86_64 natively and aarch64 under qemu-aarch64-static
8.2.2 (user mode, unpacked without root): version, daemon, bridge, hello,
host.info, killed bridge, same epoch. x86_64 is 803,624 bytes (static-pie),
aarch64 710,432 bytes (static). Not verified: survival of a real SSH logout,
a real host other than this WSL, aarch64 on real hardware, builds being
reproducible, and Windows compilation of the host crates.
`cargo test --locked -j 2 -p tessel-host-protocol` runs only this lightweight crate.
Workspace default-members remains the desktop; existing desktop commands retain
their scope. The lockfile adds only the local crate, no dependency upgrades.

Local Linux validation uses isolated CARGO_HOME/RUSTUP_HOME below
`~/.local/share/tessel-host-validation/` and a separate target directory there.
No shell profile or global PATH was changed. musl and arm64 build/runtime
acceptance remain outstanding; Windows+WSL tests do not prove those targets.

## Evidence and scope

At b121, `src/remote.rs` uses verified OpenSSH SFTP with conflict-aware writes;
`src/terminal.rs` owns local PTYs. `src/git.rs::scoped_status` still requests
`--untracked-files=all`. These are concrete integration boundaries, not evidence
of an existing persistent remote host.

VS Code is a behavioral reference, not an implementation to copy:
- [PTY service](https://github.com/microsoft/vscode/blob/main/src/vs/platform/terminal/node/ptyService.ts): process identity, readiness, output, replay and exit events.
- [Watcher contract](https://github.com/microsoft/vscode/blob/main/src/vs/platform/files/common/watcher.ts): correlated subscriptions, coalesced changes, error handling and bounded recovery.

Implement a Rust-owned daemon and client protocol. Replace the JavaScript helper
and tmux only after runtime acceptance. Do not migrate or terminate existing
tmux sessions silently. SSH authentication and host-key verification remain the
transport authority; browser credentials and host vaults are never exported.

## Processes and installation

- Separate lightweight `tessel-host-protocol` and `tessel-host` crates from the
  desktop dependency graph; neither links egui, WebView2 or document renderers.
- `tessel-host serve` owns PTYs, watchers and repository workers. A short-lived
  `tessel-host connect --stdio` bridge connects SSH stdio to a private Unix
  socket. Dropping this bridge must not drop daemon-owned PTYs.
- Private directory mode 0700, socket mode 0600, peer UID verification and a
  single-instance lock. Refuse symlink-controlled runtime directories. No public
  TCP listener. Client-side Live Share capabilities remain separate from SSH
  host authority; guests cannot request arbitrary host connections or terminals.
- Use a new daemon-owned runtime directory, not the existing `run/` directory or
  `<flavour>-<sha1>.sock` sockets. Protocol v1 coexists with the JavaScript MCP
  shim, inverse tunnel, TESSEL_* environment and hooks. Do not remove that shim
  until a separately verified `tunnel.reverse` capability replaces its behavior.
- Detect architecture, upload a temporary versioned executable, verify its
  release-manifest digest, chmod and atomically install. Fixed bootstrap command
  only; no downloaded shell script. Validate executable version before attach.
- Support upload through an existing Files exec channel: coreutils/BusyBox
  `cat`, checksum verification, `chmod` and `mv` are sufficient. No remote curl,
  package manager, sudo or Node installation is required.
- Keep an old daemon alive while it owns terminals. Upgrades start another
  versioned socket for new sessions; reconnect routes existing IDs to their
  daemon epoch. A daemon crash or machine reboot reports terminal loss, never a
  fake successful resume. Rootless persistence also depends on host login-session
  policy; test this explicitly rather than assuming `setsid` defeats a user cgroup.
- Inspect `loginctl` Linger and effective KillUserProcesses policy and expose
  persistence capability to the client. Use a user systemd scope when supported,
  but verify actual logout survival; missing/unknown support must produce an
  explicit warning rather than a promise of persistent sessions.

## Wire contract v1

- Length-prefixed frames: four-byte unsigned big-endian byte length followed by
  UTF-8 JSON. Maximum 1 MiB per frame, enforced before allocation. File and output
  chunks use base64 with a 64 KiB decoded maximum; bounded JSON nesting and string
  lengths. Linux filename bytes use explicit base64 path components, not lossy
  UTF-8 conversion. Display labels are separate from filesystem identity.
- Foundation envelope: `version`, JavaScript-safe monotonic `request_id`, and
  `message: { kind, body }`. Stream fields will be specified with PTY messages.
  First exchange is `hello`/`hello_ok` with supported versions,
  capabilities, host epoch and advertised limits. Unsupported major versions
  fail before any mutation; unsupported optional capabilities are explicit.
- Negotiate `pty`, `tree`, `git`, `editor` and `tunnel.reverse` independently.
  Vue may adopt tree+git while keeping tmux and the MCP shim. A golden JSON
  fixture exercises exactly this path without requiring PTY/tunnel support.
- Each request receives one terminal reply or structured error. Cancellation
  targets a request ID. IDs and stream sequence numbers have bounded lifetimes;
  unknown/duplicate IDs never cause repeated process creation or file writes.
- Separate bounded queues for control, PTY output and bulk file/tree traffic.
  Control and interactive input take priority. Stream acknowledgments supply
  flow control; a slow client cannot grow memory indefinitely or stall another
  terminal. Heartbeats diagnose liveness, but a missed heartbeat does not kill
  the PTY or authorize replaying unacknowledged input.
- Reconnection repeats subscriptions with epoch and last acknowledged revision.
  Replay gaps return an explicit reset requirement. No command, input, file
  content, environment or credential is included in logs; structured operation
  names, counters and durations suffice for diagnostics.

## Persistent PTY contract

Requests: `terminal.create`, `attach`, `detach`, `input`, `resize`, `signal`,
`close`, `list`. Events: `ready`, `output`, `snapshot`, `exit`, `gap`.

- Terminal identity is random and scoped to daemon epoch and owner. Creation uses
  structured executable/argv/cwd/environment, with a deduplication request ID.
  Read-only observers cannot input, resize, signal or close. One writer lease
  determines input/resize authority; attachment alone does not grant it.
- PTY reading continues while disconnected. Maintain a bounded VT screen model
  plus a sequenced output ring. Negotiate host-specific budgets; the slow-host
  default is 2 MiB per terminal and 16 MiB aggregate, capped at 64 MiB aggregate.
  Budget exhaustion evicts replay history, not the running process, and marks a
  gap. Define main/alternate screen, cursor, colors, modes, Unicode width and
  scrollback representation before claiming snapshot fidelity.
- `terminal.list` reports daemon RSS and retained replay bytes separately.
- Reattach under a sequence barrier: snapshot at S or retained bytes after the
  client's S, then events S+1 onward without duplication. Snapshot application
  must not execute OSC clipboard, shell integration or device-response side
  effects a second time. Input IDs deduplicate retries; lost acknowledgment is
  reported without blind automatic input replay.
- Resize rows/columns is bounded and serialized with output/snapshot capture.
  Exit status and final sequence remain available after process exit during a
  disconnect. Detached sessions are not silently closed by an idle timeout;
  expose resource usage and explicit cleanup. Clean shutdown closes verified
  owned process groups, not processes selected by untrusted PID alone.

## Files and editor

Requests: `root.open`, `tree.subscribe`, `tree.unsubscribe`, `file.read`,
`file.write_begin`, `file.write_chunk`, `file.write_commit`, `file.write_abort`,
`file.stat`, `file.rename`, `file.remove`, `file.mkdir`, `file.compare`.

- Authorize and canonicalize a root once, then use root-scoped handles and
  descriptor-relative operations. Reject escapes, symlink races and arbitrary
  absolute client paths. Preserve safe file modes and non-UTF-8 names.
- Install inotify watches before each bounded initial directory enumeration;
  reconcile queued events before publishing the initial revision. Build one
  cached tree and send subsequent revisioned added/updated/removed differences.
  Expansion reads cached state, not a recursive rescan on every UI refresh.
- Exclusions and watch/entry/byte limits are advertised. Watch exhaustion,
  permission errors, overflow and directory moves mark affected coverage stale.
  Reconcile only affected subtrees when possible. A bounded explicit resync is
  required after overflow; the promise of no repeated full refresh must never
  suppress recovery or fabricate a complete tree. No fallback polling loop.
- Announce default exclusions for node_modules, .git/objects and caches. Read
  fs.inotify.max_user_watches at startup; cap watches/depth and oversized directory
  enumeration. Uncovered subtrees are explicitly stale-on-demand, not empty.
- Read files as original bytes in cancelable chunks. Saves carry the original
  content digest and expected revision; validate again before atomic rename of a
  same-directory temporary file. Concurrent edits produce a conflict, never a
  silent overwrite. Abandoned uploads are cleaned with bounded age/size budgets.
- The desktop keeps dirty buffers on transport loss. Compare uses the same root
  and revision authority. Binary/image/PDF downloads remain original bytes.

## Git work scheduling

Git remains a subprocess initially; event-driven invalidation does not magically
make Git's own index scan incremental. Do not claim constant-cost status.

- One worker per canonical repository, bounded host-wide concurrency (initially
  one Git job), low OS priority and a separate queue from PTYs. Group filesystem
  changes for 300 ms, maximum two seconds under continuous activity.
- Apply nice 10 and idle-class ionice where available. Detect existing
  core.untrackedCache/fsmonitor support, but never enable either in user config
  without authorization. Compare strategies using at least 300,000 synthetic
  untracked files before choosing large-tree defaults.
- Cache porcelain-v2 status keyed by repository epoch, index/HEAD generation and
  dirty paths. Watch worktree paths plus gitdir, common-dir, index, refs and ignore
  metadata, including linked worktrees. Push revisioned diffs to subscribers;
  zero status commands when no subscription or invalidation exists.
- Ordinary dirty paths use literal pathspecs and batched status queries; replace
  only the covered cache entries. Include both sides of moves and affected parent
  scopes. HEAD/index/ignore changes or uncertain rename/submodule coverage require
  a bounded broader reconciliation, explicitly marked stale until complete.
- Start with `--untracked-files=normal`; enumerate an untracked directory only on
  demand. Preserve stage/worktree distinctions and literal filenames. Do not
  silently omit untracked files from an explicitly requested expanded view.
- Initial limits: two-second soft budget, five-second hard process-group timeout,
  8 MiB output and 50,000 entries. On timeout/truncation keep last good state with
  an incomplete/stale marker and exponential backoff. Never publish partial
  output as a clean repository. Mutations invalidate the relevant cached scopes.

## Delivery and acceptance gates

1. Protocol crate, bounded codecs, golden JSON fixtures and error/compatibility
   matrix. Fuzz malformed lengths, nesting, base64, duplicate IDs and sequences.
2. Linux daemon/bridge and rootless installer. Verify x86_64/aarch64 musl artifacts
   using ELF inspection plus real native or emulated Linux execution. No GTK or
   Node dependencies. Release checksums and architecture selection are mandatory.
3. PTY lifecycle tests over an abruptly killed bridge: same child PID after
   reconnect, exact numbered output, Unicode and alternate screen, resize, lost
   ACK, ring overflow, process exit offline and daemon restart epoch mismatch.
4. Tree watcher tests: initial scan race, create/delete/rename, symlink swaps,
   ignore changes, permissions, inotify overflow/exhaustion and unsubscribe.
   Instrument directory reads: idle/refresh must not enumerate the tree again.
5. Editor acceptance: original-byte reads, save conflict, interrupted upload,
   atomic commit, compare, binary download and dirty-buffer reconnect.
6. Git reference tests compare cached results with a complete controlled-fixture
   status for edits, staged changes, renames, deletes, ignored paths, worktrees and
   submodules. Large synthetic repositories and bounded CPU/I/O load verify that
   PTY interaction remains responsive; record p50/p95 latency, bytes and command
   counts. Never benchmark the production FiveM host without a separate request.
7. Desktop transport integration with reconnect UI and explicit degraded states;
   cross-client protocol fixtures prepare later Vue adoption. Keep SFTP fallback
   available and visibly distinct until all required operations pass. Retire
   tmux/JavaScript only for newly migrated sessions after these gates pass.

Review requests: Darwin checks future Vue compatibility and operational limits;
Franklin checks reconnect/pane UX and replay semantics. Implementation begins with
the protocol and synthetic tests, not installation on the user's live server.
