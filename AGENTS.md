# Tessel Rust development

- This is a native Rust rewrite, not a wrapper around the existing Electron application.
- Work here without changing or restarting the running `C:\Tessel` application.
- Keep README Features and limitations accurate in every user-visible push.
- Never copy credentials, browser profiles, personal records or existing Tessel settings into the repository.
- Preserve private host credentials and permissions when implementing Live Share. Guests must not inherit host secrets or authority.
- Run formatting, focused tests and clippy before pushing. Limit compilation to two jobs on the shared workstation.
- Report the commit and build count after pushing. Do not claim feature parity until tested.
