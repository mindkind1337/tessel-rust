# Tessel Rust

Native Rust rewrite of Tessel. **Early prototype, not a replacement for the existing application yet.** Both the application UI and backend are written in Rust. There is no Vue, Electron, JavaScript application bundle, or Node runtime.

## Features

- Native desktop window using egui/eframe, with a dark theme.
- Multiple independent terminal tabs backed by real operating-system pseudo terminals (Windows ConPTY through portable-pty).
- PowerShell without loading user profiles on Windows; `/bin/sh` on Unix.
- ANSI screen rendering with terminal colors, cursor, keyboard input, paste, Ctrl+C and resizing.
- Bounded terminal output queue; shell termination when closing its tab.
- Separate application: does not read or migrate the existing Tessel configuration, passwords or browser sessions.

## Run

Install Rust stable and the native C/C++ build tools for your platform. Windows requires the MSVC C++ build tools and Windows SDK.

```sh
cargo run --locked -j 2
```

```sh
cargo fmt --check
cargo test --locked -j 2
cargo clippy --locked -j 2 -- -D warnings
cargo build --locked -j 2
```

Windows binary: `target/debug/tessel-rust.exe`. No installer or release is published yet.

## Current limitations and migration plan

The terminal is a first vertical slice, not complete terminal-emulator parity: scrollback navigation, selection/copy, IME, full keyboard protocols, rich text attributes and mouse reporting remain to be implemented. Shell exit/error status is shown on hover over the terminal. Closing a terminal ends its shell; persistent sessions are not implemented.

The following existing Tessel capabilities are **not yet migrated**:

1. Project persistence, filesystem explorer, editor, split panes and document previews.
2. SSH connection management, reconnection and persistent remote sessions.
3. Agent providers, conversations, permissions, tool server and task board.
4. Browser integration, isolated authenticated sessions and original-file downloads.
5. Encrypted credential storage, connectors and account settings.
6. Live Share with explicit host/guest authorization and private-secret isolation.
7. Packaging, updates, accessibility and cross-platform acceptance testing.

The browser will require an embedded browser engine; "Rust application" does not mean rewriting the web platform or OS libraries in Rust. Browser-engine selection is still open. No existing Tessel secrets or private project data should be copied into this repository.

## Validation

Tests include a real PTY round trip that launches a shell and checks actual command output, plus keyboard protocol checks. Native GUI behavior must also be checked on each supported OS; a successful compile alone is not a GUI acceptance test.

Initial Windows validation: formatting, clippy with warnings denied, both tests and debug build pass. A native startup smoke check confirms a live `Tessel Rust` window. Interactive visual acceptance and other platforms are not yet validated.
