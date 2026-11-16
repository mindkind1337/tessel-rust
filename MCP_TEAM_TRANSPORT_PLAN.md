# Team MCP transport implementation plan

Status: design only; no listener or CLI/Claude integration is enabled by this document.

## Boundary and delivery

Architecture: the provider is the parent of a stdio helper; that helper bridges its MCP pipe to the desktop broker over an authenticated loopback socket. These are two distinct transports. Stdio is parent/child scoped; loopback is reachable by other local processes. The same OS user is not a security boundary: a malicious process with permission to inspect child environment can steal that child capability. Request/response authentication rejects local peers without the capability; it does not protect against a compromised OS user. No key grants authority beyond its bound pane/project/generation.

Use the Rust executable in a dedicated `--team-mcp-stdio` mode. Start that mode before GUI initialization. Expose only the existing team/project/task catalogue, excluding browser credentials and unrelated application APIs. Claude keeps its existing default permission mode, safe mode, strict MCP configuration and isolated native account profile. CLI integrations must use typed launch arguments and per-child environment, never shell interpolation of user briefs or credentials.

Start with the versioned MCP handshake specification (2025-11-25, with supported older handshake versions explicitly enumerated). Do not claim current-version universal compatibility: newer protocol generations and actual installed-client negotiation require separate fixture coverage. Stdio uses bounded UTF-8 newline-delimited JSON-RPC, an initialize/initialized state machine, ping, tools/list and tools/call. No ordinary output on stdout. Unknown methods and malformed requests get protocol errors; tool failures use isError. Do not advertise resources, sampling, elicitation or tasks that are not implemented.

## Authenticated local broker

The desktop binds an ephemeral loopback-only socket, never all interfaces, only when local team-enabled agents need it. Bind once to port 0 so the OS selects the free port; any bind failure disables this transport with no guessed fallback endpoint. Stop accepting and terminate connected clients on shutdown. A small bounded channel transfers requests to the existing shared GUI poll; no socket thread mutates Workspace or TeamsRelayHub. Bound frame sizes, connection count, pending queue, read/write deadlines and per-poll work. Oversize or unterminated frames fail closed without draining unbounded input. Tests include slow readers, partial headers, queue saturation and shutdown with clients connected.

Credentials are runtime-only per pane and launch generation. Increment the generation on every child launch, including reconnect/restart, not just on app startup; checked exhaustion refuses launch. The desktop binds each issued key to trusted pane id, project id, working directory and generation. Every request is authenticated and replay-checked, then that binding is rechecked against current app state immediately before dispatch through execute_persisted_for_pane. No caller-supplied project, pane, me or cwd establishes authority. Guest/shared remote pane data cannot create a local credential. Closing, sleeping, restarting or moving a pane revokes its old binding; a new launch cannot reuse the old key. No global fallback key. Failed entropy prevents launch.

The helper receives only scoped transport values through child environment; secrets never enter argv, project files, persisted workspace, Live Share, logs, Debug output or tool results. Apply transport values after normal environment/profile setup so user preferences cannot override the binding. Endpoint validation accepts only loopback and a numeric port. Do not forward this local endpoint/key through SSH or to host-daemon terminals; remote support is a separate authenticated transport.

Authenticate responses as well as requests, with explicit request/response domain separation and binding to the request nonce. This prevents a replaced local endpoint from inventing tool results and prevents reflecting a signed request as a response. Limit and validate all envelope fields. Credential buffers use redacted formatting and zeroizing owners without claiming all OS/environment copies are erased.

Before executing a queued mutation, check its deadline and cancellation state. A response timeout after dispatch is an uncertain outcome and must never trigger automatic replay. Preserve the existing task persistence boundary and user-facing worker/relay approvals. No broadened tool allowlist or automatic approval is introduced.

## Implementation slices and fixtures

1. Bounded stdio protocol state machine and JSON-RPC/MCP fixtures, no provider processes or listener activation.
2. Authenticated bounded broker + request/response protocol and synthetic loopback tests; no GUI exposure yet.
3. Shared runtime dispatch with trusted pane/project/generation bindings, revoked/moved/closed/guest rejection and persisted task fixture tests.
4. Native Claude per-child config/env wiring, then terminal provider adapters with exact argument/env fixtures; retain all sandbox and permission flags.
5. Worker first-turn submission for each supported path only after its mailbox/tools are reachable, preserving the once-only rule and no replay after uncertainty.
6. Native UI and helper-process fixture acceptance; document untested provider/runtime/platform combinations honestly.

## Primary references checked 2026-11-15

- https://modelcontextprotocol.io/specification/2025-11-25/basic/transports
- https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle
- https://modelcontextprotocol.io/specification/2025-11-25/server/tools

Code anchors: src/agent_tools.rs (persisted pane-scoped dispatch), src/teams_relay.rs (catalogue/auth/replay and user approval gates), src/claude/transport.rs (strict empty MCP config today), src/main.rs (early worker modes), src/app.rs (shared runtime poll), src/terminal.rs (PTY child construction). Active Vue files are references only and remain untouched.
