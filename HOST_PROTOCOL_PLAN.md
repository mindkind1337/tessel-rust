# Native remote host: separate repository

The living implementation plan, wire specification, daemon, installer and release
checks now live in the private [tessel-host repository](https://github.com/mindkind1337/tessel-host):

- [Implementation plan](https://github.com/mindkind1337/tessel-host/blob/main/HOST_PROTOCOL_PLAN.md)
- [Protocol specification](https://github.com/mindkind1337/tessel-host/blob/main/crates/host-protocol/PROTOCOL.md)
- [Build and validation instructions](https://github.com/mindkind1337/tessel-host/blob/main/README.md)

Tessel Rust pins `tessel-host-protocol` to
`35f41cd68849e60d42bca14b3449222fad1e7ace` in Cargo.toml and Cargo.lock.
There are no local host crates in this desktop workspace. Gates 1 and 2 were
published by the host maintainer before this separation; desktop integration
(gate 7) remains pending. The dependency alone does not replace SSH shells,
tmux or the current JavaScript helper.

## Local builds

Your Git account needs read access to the private host repository. The checked-in
`.cargo/config.toml` selects Git CLI fetching so Cargo uses your existing Git
credential setup. No credentials belong in the manifest, lockfile or repository.
Run `cargo check --locked -j 2` to verify access and the pinned dependency.

## GitHub Actions access

The default repository GITHUB_TOKEN cannot read a different private repository.
Before running this repository's CI, configure a dedicated SSH deploy key:

1. Generate a dedicated Ed25519 key for this dependency, without a passphrase.
2. In **tessel-host > Settings > Deploy keys**, add its public key with write
   access **disabled**.
3. In **tessel-rust > Settings > Secrets and variables > Actions**, add the
   private key as the repository secret **TESSEL_HOST_DEPLOY_KEY**.
4. Remove the temporary local private-key copy after the secret is stored, then
   rerun the Rust workflow. Never paste the key into chat or commit it.

The workflow restricts the key file's Windows ACL, fetches GitHub host keys over
HTTPS, enables strict SSH host verification, rewrites only the tessel-host Git
URL and removes the key even on failure. It does not cache Cargo's Git checkout
of the private source. Local Git authentication remains unchanged.

The CI secret is an external prerequisite; documenting the workflow does not
prove that the secret has been configured or that a hosted build has passed.
