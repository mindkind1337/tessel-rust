# Tessel patch: native editor styling

Upstream: `gpui_ce_elements` 0.3.0, downloaded from crates.io, Apache-2.0.
Registry archive SHA-256: `c74a09e0d1e27d0d5d22be1c9764d4576b11c7a60fe1e13f2ca88873714423de`.
The published `.cargo_vcs_info.json` records `41a42eb96d6c04e5e71f3577d12934d4de569b20`.
The source package did not include a LICENSE or NOTICE file. `LICENSE.md` is the
Apache-2.0 text from the upstream project's public license file, matching the
package manifest. No upstream source copyright headers were removed.

Changes are confined to `src/editable_text/element.rs` and `layout.rs`:

- Add optional `styled_runs(source, Arc<[TextRun]>)` to the existing element.
- Reject incomplete, overflowing and non-character-aligned UTF-8 coverage with
  `Err`, before shaping. Callers must handle this and render a plain field.
- Ignore styles when the current source differs, including same-byte-length
  edits and IME changes. Placeholders always use their ordinary style.
- Include immutable run-array identity in the layout key, so style-only changes
  do not reuse a stale shaped document and unchanged frames compare in O(1).
  Callers should cache and reuse the Arc;
  this API does not run a tokenizer during input or layout.
- Leave caret, selection, clipboard, undo, IME, accessibility and the default
  unstyled element paths intact.

Public API and headless rendering regression tests live in Tessel's
`src/gpui_ui/styled_input_tests.rs`, alongside the existing remote-input tests.
The published upstream crate's own test sources reference an unpublished
`gpui-ce-fonts` development package; they are preserved but are not represented
as executed by Tessel's focused validation.

On upgrade, compare these two files with the new registry source, reapply only
the additive API/cache changes if still needed, update this checksum, and run
the style and native-input regression tests. Remove the patch when an upstream
API supplies the same behavior.
