# Native migration acceptance ledger

Source baseline: the Features sections of `C:\Tessel\README.md`, inspected 2026-11-02. This ledger tracks capability, not just UI buttons. A row is complete only when its implementation and boundary tests pass. No existing credentials or user documents are migration fixtures.

| Capability | Required acceptance | Status |
| --- | --- | --- |
| Native application | Rust GUI and backend; no Electron/Vue runtime | Initial window verified on Windows |
| Terminals | Real PTY, input, resize, ANSI, background draining, shutdown, scrollback | Initial PTY verified; improving |
| Projects | Persistent roots, selection, no process starts on restore | Implemented; persistence tests pass |
| Files/editor | Confined browsing; tabs; conflict-aware atomic saving; dirty close guard | Implemented; filesystem tests pass |
| Layout | Persistent grid; project switching retains sessions | In progress |
| Agent terminals | Detect installed CLIs; project cwd; no invented provider flags | In progress |
| SSH | Host verification, keepalive, explicit reconnect, remote cwd | In progress; live host acceptance pending |
| Task board | Per-project cards; persistence; safe movement | Implemented, including scoped native chat tools |
| Git | Nonblocking, bounded status/diff/log; explicit mutations; no auto polling storms | Implemented; real temporary-repository test passes |
| Native agent chat | Provider transports, streaming, approvals, resume and history | Codex transport and live initialization pass; other providers/history pending |
| Agent tools | Authenticated scoped tools, approval UI, messages and task board | Pending |
| Browser | Engine integration; isolated profiles; navigation/download tests | Windows WebView2 implemented; three native download cases pass; agent control pending |
| Documents | HTML trust, PDF, images, Markdown, office documents | Native readable HTML/Markdown and bounded image previews implemented; PDF/full webpage/office rendering pending |
| Credentials | OS-protected storage; explicit agent opt-in; never available to guests | Windows DPAPI/CSV and owner-only UI implemented; encryption tests pass; manual fill acceptance and agent consent pending |
| Live Share | Authenticated transport; project scope; explicit guest rights; secret-isolation tests | Native read-only LAN protocol implemented and real TLS tests pass; cloud relay, editing and Electron compatibility pending |
| Connectors/account/sync | Match existing service contracts; token isolation and consent | Pending |
| Packaging/update | Native installer, integrity verification, safe restart, CI artifacts | Pending |
| Localization/accessibility | EN/FR; keyboard navigation; screen reader acceptance | Pending |

The Rust application can invoke external shells and agent CLIs. An embedded browser requires an external browser engine. Those are dependencies, not a JavaScript implementation of Tessel. Existing service credentials and production endpoints are not exercised automatically.
