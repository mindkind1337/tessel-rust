# Tessel patch: native editor styling and undo boundaries

Upstream: `gpui_ce_elements` 0.3.0, downloaded from crates.io, Apache-2.0.
Registry archive SHA-256: `c74a09e0d1e27d0d5d22be1c9764d4576b11c7a60fe1e13f2ca88873714423de`.
The published `.cargo_vcs_info.json` records `41a42eb96d6c04e5e71f3577d12934d4de569b20`.
The source package did not include a LICENSE or NOTICE file. `LICENSE.md` is the
Apache-2.0 text from the upstream project's public license file, matching the
package manifest. No upstream source copyright headers were removed.

Styling changes are in `src/editable_text/element.rs` and `layout.rs`; undo-boundary changes are in `src/editable_text/history.rs` and `state.rs`:

- Add optional `styled_runs(source, Arc<[TextRun]>)` to the existing element.
- Reject incomplete, overflowing and non-character-aligned UTF-8 coverage with
  `Err`, before shaping. Callers must handle this and render a plain field.
- Ignore styles when the current source differs, including same-byte-length
  edits and IME changes. Placeholders always use their ordinary style.
- Include immutable run-array identity in the layout key, so style-only changes
  do not reuse a stale shaped document and unchanged frames compare in O(1).
  Callers should cache and reuse the Arc;
  this API does not run a tokenizer during input or layout.
- Styling leaves caret, selection, clipboard, undo, IME, accessibility and the
  default unstyled element paths intact. The separate undo extension below
  controls explicit command boundaries.

Public API and headless rendering regression tests live in Tessel's
`src/gpui_ui/styled_input_tests.rs`, alongside the existing remote-input tests.
The published upstream crate's own test sources reference an unpublished
`gpui-ce-fonts` development package; they are preserved but are not represented
as executed by Tessel's focused validation.

On upgrade, compare all four modified files with the new registry source, reapply only
the additive API/cache and undo-boundary changes if still needed, update this checksum, and run
the style and native-input regression tests. Remove the patch when an upstream
API supplies the same behavior.

The editable-text state and history also expose `push_undo_stop()`. It separates the next recorded edit from the preceding group without adding empty entries, so explicit editor replacement commands do not merge with surrounding typing or adjacent replacements. Preserve this API and its native replacement regression tests when upgrading.
Undo and redo also end the current edit group, so typing after history navigation cannot merge into an earlier command. Native GPUI regressions cover adjacent replacements, subsequent typing and a new edit after undo.

## Private masked storage hooks (2026-11-16)

`storage.rs` adds default-preserving hooks for storage-owned history, replacement acceptance and directed selection capture. `state.rs` checks acceptance before changing caret or IME ranges and skips its ordinary history when a storage implementation opts out. Tessel's private ASCII key storage exposes only placeholders to the element, platform text queries and accessibility; its zeroizing secret and bounded undo history remain outside the generic editor. Tests live in `src/gpui_ui/private_key_storage.rs` and `private_key_input.rs`. Preserve these hooks alongside the existing style/undo patches on upgrade. Ordinary StringStorage keeps the default behavior.
