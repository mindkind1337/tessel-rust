# Third-Party Notices

Tessel Rust includes or adapts the third-party material identified below. The original
copyright and permission notices are reproduced in full.

## Microsoft Codicons

`resources/codicons/codicon.ttf` is an unmodified copy of the Microsoft Codicons font bundled with Tessel. The artwork is © Microsoft Corporation and distributed under [Creative Commons Attribution 4.0 International](https://creativecommons.org/licenses/by/4.0/). [Upstream project](https://github.com/microsoft/vscode-codicons).

## Monaco indentation and cursor navigation

`src/editor_indent.rs` adapts indentation detection, tab stops and character
width rules from Microsoft Monaco Editor 0.57.0 (`indentationGuesser.js`,
`shiftCommand.js`, `cursorTypeEditOperations.js`, `cursorColumns.js`, `strings.js`,
`cursor.js` and `intervalTree.js`). The Go to Line parser in `src/files.rs` follows
`gotoLineQuickAccess.js` from the same version. Find initialization uses the selection limits and default word
separators from `findController.js` and `wordHelper.js`. Local line duplication follows
`copyLinesCommand.js` and the shortcuts in `linesOperations.js`.
The synthetic detection vectors can be regenerated with
`scripts/extract_editor_indent_reference.mjs` and
`scripts/extract_editor_shift_reference.mjs` against that version. This is native
Rust code; Monaco and a JavaScript runtime are not shipped for editing.

The MIT License (MIT)

Copyright (c) 2016 - present Microsoft Corporation

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## gpui_ce_elements

`vendor/gpui_ce_elements` contains the crates.io 0.3.0 source package under
Apache-2.0, reproduced in `vendor/gpui_ce_elements/LICENSE.md`. Tessel adds
optional source-bound styled text runs and matching layout invalidation for
native editor syntax highlighting, plus explicit undo-group boundaries for
editor replacement commands and opt-in guarded, storage-owned history for private masked fields. The modified files, source checksum and
upgrade requirements are recorded in `vendor/gpui_ce_elements/TESSEL_PATCH.md`.

## portable-pty

`vendor/portable-pty` contains portable-pty 0.9.0 by Wez Furlong, under the MIT
license reproduced in `vendor/portable-pty/LICENSE.md`. Tessel's Windows patch
starts the child suspended, assigns a dedicated process job before execution,
terminates that job, and exposes verified descendant exit accounting. The patch
and upgrade requirements are recorded in `vendor/portable-pty/TESSEL_PATCH.md`.

## vt100

`vendor/vt100` contains vt100 0.15.2 by Jesse Luehrs, under the MIT license
reproduced in `vendor/vt100/LICENSE`. Tessel bounds visible scrollback rows and
prevents subtraction overflow when the scrollback offset exceeds the screen
height. The patch is recorded in `vendor/vt100/TESSEL_PATCH.md`.

## Orca

The language labels in src/chat/blocks.rs follow Orca’s rich-markdown-code-block-languages.ts, as used by Tessel’s native chat. Chat file links follow Orca’s file-link-location.js and native-chat-href-routing.js. The chat scroll thresholds follow Orca’s native-chat-autoscroll.js. The compatibility composer follows the Enter and composition behavior in use-native-chat-composer-keydown.js.

`src/linear_model.rs` and its `parse`, `record`, `service`, and `panel` modules, together with
`src/linear_dialog.rs` and `src/gpui_ui/linear_view.rs`, follow Tessel's independent Linear service and its
Orca-derived Linear integration design reference, retaining this attribution.

The floating terminal UI in `src/floating_terminal_panel.rs` and its application
integration adapt Tessel's `floatingTerminal.js`, `FloatingTerminal.vue` and
`App.vue` behavior under the same MIT attribution below. The process controller
in `src/floating_terminal_runtime.rs` is a native Rust implementation.

The pure conversation resume planner in `src/agent_resume.rs` adapts Tessel's
`agentResumeLine.js` and `remoteAgentLaunch.js`, including argument mappings from
the vendored Orca `agent-session-resume.js`. Its source-comparison fixture uses
synthetic session identifiers and paths only.

The bounded Codex metadata discovery in `src/codex_sessions.rs` adapts the folder,
launch-time and exclusion policy in Tessel's `src/main/agentSessions.js`.
Its tests use temporary synthetic rollouts, not user conversation records.
The bounded session scanner and Codex prompt-title extraction in
`src/side_panels_model.rs` also follow `src/main/agentSessions.js`.
The reverse log scan in `src/pane_model_log.rs` adapts Tessel's
`src/main/latestInFile.js`, retaining its 512 KiB chunks and 64 MiB scan limit.
The runtime in `src/pane_model_runtime.rs` follows the refresh policy in
`TerminalPane.vue`. Its shared pure readers in `src/agent_model_readers.rs`
adapt Tessel's `agentModel.js`, `agentModelSources.js`, related configuration
readers and `TerminalPane.vue` model resolution. The reference fixture records
the source hashes and uses synthetic data only, under the same MIT attribution.

The pure per-pane model state and planners in `src/pane_session_model.rs` adapt
Tessel's `agentSessionOptions.js`, `agentPrefs.js`, `paneLastModel.js`, model and
session-option labels, `claudeModelSwitch.js`, terminal screen model readers and
`TerminalPane.vue`, including the Orca-derived option catalog and CLI mappings.
The reference generator records source-file hashes and only synthetic inputs.
The footer observation integration in `src/terminal_model.rs` follows Tessel
`TerminalPane.vue` and uses the attributed parsers in `src/pane_session_model.rs`.

The native per-pane picker in `src/session_option_picker_ui.rs` adapts
`SessionOptionPicker.vue`, `TerminalPane.vue`, `PaneActionsMenu.vue` and their
option labels and geometry under the same Tessel/Orca attribution.

The native SSH Hosts settings interface (`src/ssh_hosts_panel.rs`) and shared connection profile model (`src/ssh_profile.rs`) contain UI designs,
status conventions, and draft validation adapted from Tessel's `RemoteHostsSettings.vue`,
`RemoteHostForm.vue`, `remoteHosts.js`, and `remoteHosts.css`, which are ported from Orca
by Lovecast Inc. The bounded alias discovery in `src/ssh_config.rs` also adapts
Include/token, key-exclusion and budget behavior from Tessel’s `sshConfig.js`,
ported from Orca’s `ssh-config-parser.ts`, `ssh-config-include-expander.ts` and
`ssh-config-path-expansion.ts`. The shared agent preset catalog (`src/agent_catalog.rs`),
agent CLI flag inspection, model-option catalog, launch composition, and model defaults
(`src/agent_arg_options.rs`, `src/agent_model_catalog.rs`, `src/agent_model_catalog.json`,
`src/agent_model_launch.rs`, `src/agent_model_options.rs`, and `src/agent_model_defaults.rs`), vector brand icon renderer
(`src/agent_brand_icons.rs`), and native Agents settings panel (`src/agents_panel.rs`) also
adapt the built-in agent preset metadata, session model/option catalog, CLI flag detection
and launch composition rules, the `aider`, `amp`, `cursor`, `kimi`, `omp`, `pi`, and `kilo`
agent icon designs, and per-agent permission flag references from Tessel's `main/index.js`,
`shared/agentSessionOptions.js`, `cursorModels.js`, `BrandIcon.vue`, `brandSvgs.js`,
`agentPrefs.js`, and `agentDetect.js`, ported from Orca's `agent-icon-glyphs.tsx`,
`src/shared/agent-icons`, `agent-session-option-catalog*.ts`, `agent-cli-flag-detection.ts`,
`tui-agent-config.ts`, and `tui-agent-permissions.ts`.

The model-list parser and probe specifications in `src/agent_model_probe_parse.rs`
and `src/agent_model_probe.rs` also adapt Tessel's
`shared/agentModelProbe.js`, derived from Orca's `claude-model-list-probe.ts`,
`commit-message-model-parsers.ts`, `grok-model-list-probe.ts` and `model-id-label.ts`.
The successful-list cache in `src/agent_model_cache.rs` adapts Tessel's
`main/agentModelList.js`, derived from Orca's `agent-model-catalog-store.ts`.
The Antigravity variant grouping in `src/agent_model_antigravity.rs` adapts
Tessel's `shared/antigravityModels.js`.
The program/shim resolver in `src/agent_program.rs` adapts Tessel's
`main/agentHeadless.js` and `main/agentTools.js`, including `shimTarget`,
derived from Orca's local-process and commit-message agent launch rules.
The native model selector in `src/agent_model_defaults_ui.rs` adapts Tessel's
`AgentModelDefaults.vue`, `ThemedSelect.vue`, `sessionOptionLabels.js` and
`cursorModels.js`, including Orca's session-option catalog and label conventions.
The background discovery controller in `src/agent_model_discovery.rs` follows
the success-only cache and per-agent request deduplication of `agentModelList.js`.

- Project: https://github.com/stablyai/orca
- Reference revision: `433986fa3be37911a0f13f7ffd454b831b87a64c` (package version `1.4.214`)
- Referenced source: `SshPane.tsx`, `SshTargetCard.tsx`, `SshTargetForm.tsx`,
  `ssh-target-draft.ts`, `ssh-target-save-payload.ts`, `agent-icon-glyphs.tsx`,
  `src/shared/agent-icons`, `agent-session-option-catalog*.ts`,
  `agent-cli-flag-detection.ts`, `tui-agent-config.ts`, `tui-agent-permissions.ts`
- License source: https://github.com/stablyai/orca/blob/433986fa3be37911a0f13f7ffd454b831b87a64c/LICENSE

```text
MIT License

Copyright (c) 2026 Lovecast Inc.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## LobeHub Icons (`lobe-icons`)

The `grok` vector mark in `src/agent_brand_icons.rs` is adapted from Tessel's `brandSvgs.js`,
derived from `lobehub/lobe-icons` under the MIT License:

- Project: https://github.com/lobehub/lobe-icons
- License source: https://github.com/lobehub/lobe-icons/blob/master/LICENSE

```text
MIT License

Copyright (c) 2023-present LobeHub

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Lucide Icons (`src/icons.rs`, `src/gpui_ui/icons.rs`, `src/*_panel.rs`)

Vector icon geometry in `src/icons.rs` and native `egui` panels (`src/*_panel.rs`) includes
icons derived from `lucide-vue-next` (`v0.577.0`), distributed under the ISC License with
portions derived from Feather under the MIT License:

```text
ISC License

Copyright (c) for portions of Lucide are held by Cole Bemis 2013-2026 as part of Feather (MIT). All other copyright (c) for Lucide are held by Lucide Contributors 2026.

Permission to use, copy, modify, and/or distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO
EVENT SHALL THE AUTHOR BE LIABLE FOR ANY SPECIAL, DIRECT, INDIRECT, OR
CONSEQUENTIAL DAMAGES OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE,
DATA OR PROFITS, WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER
TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE
OR PERFORMANCE OF THIS SOFTWARE.

---

The MIT License (MIT) (for portions derived from Feather)

Copyright (c) 2013-2026 Cole Bemis

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

The same Orca/Tessel model-discovery attribution above also covers `src/agent_model_listed.rs`, adapted from `src/shared/agentModelProbe.js` (`listedToCatalogModels`).

`src/muse_catalog.rs` is also adapted from Tessel `src/main/museCatalog.js` and the model-id/data-directory helpers in `src/main/museSessions.js`, under the same Orca attribution above (MIT, Copyright (c) 2026 Lovecast Inc.).

`src/muse_model_probe.rs` and `src/muse_protocol.rs` adapt Tessel `src/main/museServe.js` and `src/main/agentModelList.js`, following Muse's JSON-RPC session-protocol handshake and model/list query. The same Orca/Tessel MIT attribution above applies.

## ICU4X Unicode collation and date formatting

The Cursor model-label comparator and regional date formatter use ICU4X icu_collator, icu_datetime and icu_locale_core 2.4.0 and compiled Unicode data (https://github.com/unicode-org/icu4x).

UNICODE LICENSE V3

COPYRIGHT AND PERMISSION NOTICE

Copyright © 2020-2024 Unicode, Inc.

NOTICE TO USER: Carefully read the following legal agreement. BY
DOWNLOADING, INSTALLING, COPYING OR OTHERWISE USING DATA FILES, AND/OR
SOFTWARE, YOU UNEQUIVOCALLY ACCEPT, AND AGREE TO BE BOUND BY, ALL OF THE
TERMS AND CONDITIONS OF THIS AGREEMENT. IF YOU DO NOT AGREE, DO NOT
DOWNLOAD, INSTALL, COPY, DISTRIBUTE OR USE THE DATA FILES OR SOFTWARE.

Permission is hereby granted, free of charge, to any person obtaining a
copy of data files and any associated documentation (the "Data Files") or
software and any associated documentation (the "Software") to deal in the
Data Files or Software without restriction, including without limitation
the rights to use, copy, modify, merge, publish, distribute, and/or sell
copies of the Data Files or Software, and to permit persons to whom the
Data Files or Software are furnished to do so, provided that either (a)
this copyright and permission notice appear with all copies of the Data
Files or Software, or (b) this copyright and permission notice appear in
associated Documentation.

THE DATA FILES AND SOFTWARE ARE PROVIDED "AS IS", WITHOUT WARRANTY OF ANY
KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT OF
THIRD PARTY RIGHTS.

IN NO EVENT SHALL THE COPYRIGHT HOLDER OR HOLDERS INCLUDED IN THIS NOTICE
BE LIABLE FOR ANY CLAIM, OR ANY SPECIAL INDIRECT OR CONSEQUENTIAL DAMAGES,
OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS,
WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION,
ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THE DATA
FILES OR SOFTWARE.

Except as contained in this notice, the name of a copyright holder shall
not be used in advertising or otherwise to promote the sale, use or other
dealings in these Data Files or Software without prior written
authorization of the copyright holder.

SPDX-License-Identifier: Unicode-3.0

—

Portions of ICU4X may have been adapted from ICU4C and/or ICU4J.
ICU 1.8.1 to ICU 57.1 © 1995-2016 International Business Machines Corporation and others.
`src/agent_model_cursor.rs` and its reference fixtures also adapt Tessel `src/shared/cursorModels.js` under the Orca MIT attribution (Copyright (c) 2026 Lovecast Inc.).

## Tessel host release resources

`resources/host/v0.1.0/` contains the unmodified Linux x86_64 and aarch64 release binaries from `mindkind1337/tessel-host`, tag `v0.1.0` (commit `4354459b450368f6f8d28058e95c872dd921b6ef`). The accompanying MIT license is preserved in `resources/host/v0.1.0/LICENSE` (Copyright (c) 2026 Jean-Claude Trottier); the manifest, SHA256SUMS and SOURCE.json record the verified release inputs. These older resources are retained for release provenance; the desktop now installs 0.1.1 as noted below. The protocol, client and installer library are consumed from the same MIT-licensed repository at commit 23b62ac09d70776d03fc4d91c2d8512c3c7addb8. The bundled daemon remains the tagged 0.1.1 release; later client/protocol fixes preserve its wire format.

`src/browser_passwords_model.rs`, `src/browser_passwords_panel.rs` and their synthetic reference/UI fixtures adapt the Tessel browser-password dialog and main-process validation rules under the existing Orca MIT attribution (Copyright (c) 2026 Lovecast Inc.). No browser profile or real credential data is included.

The desktop now installs `resources/host/v0.1.1/`, from tag `v0.1.1` (commit `3c459afab6d200035865fbf1e3a803d209fcd79b`). Its unmodified x86_64/aarch64 binaries and MIT LICENSE are accompanied by verified manifest, SHA256SUMS and SOURCE.json. The older 0.1.0 provenance remains recorded above.

The embedded task-card artwork catalog (`src/agent_brand_assets.json`), its shared renderer (`src/agent_brand_assets.rs`) and snapshot generator (`scripts/generate_agent_brand_assets.py`) adapt Tessel's `BrandIcon.vue`, `brandSvgs.js` and bundled `assets/agent-icons/*.png`. The bundled PNG artwork and applicable vector designs originate in Orca's `src/shared/agent-icons` and `agent-icon-glyphs.tsx`, copyright (c) 2026 Lovecast Inc., under the MIT license reproduced above. The Grok mark also retains the LobeHub attribution above. Source-file SHA256 values are recorded in the catalog; no external artwork is fetched at runtime.

## Guest-facing task text

`src/share/guest_text.rs` adapts Tessel `src/main/liveShare/shareChat.js` under the existing MIT attribution above. `scripts/extract_guest_text_reference.mjs` records the source SHA256 and synthetic comparison vectors only; no live profile, account, transcript or credential is included. The Rust task-title boundary additionally scrubs the complete bounded title before final clipping, masks encoded URI user information before decoding, and handles mixed quotation marks in paths.


The guest Git safety policy in `src/share/guest_git_host.rs`, its reference fixtures, and the protected-path additions in `src/share.rs` adapt Tessel's `src/main/gitSafety.js`, `src/main/liveShare/shareScope.js` and `src/main/liveShare/shareRequests.js` under the existing Orca/Tessel MIT attribution above (Copyright (c) 2026 Lovecast Inc.). The Rust authority adapter, bounded runner integration and mutable-configuration origin guard add independent safety checks; no repository credentials or user configuration are bundled.

The protected-path catalogue in `src/share/protected_paths.rs` and its synthetic differential extractor adapt Tessel `src/main/liveShare/shareScope.js` under the existing MIT attribution above. The fixture records the source hash and synthetic paths only.

The secret-name policy in `src/share/secret_names.rs`, `scripts/extract_secret_names_reference.mjs`, and its synthetic fixture adapt Tessel `src/main/liveShare/shareScope.js` under the existing MIT attribution above. The per-share ignored-path history follows the same reference semantics while using the independent bounded Rust Git runner. No private files or Git configuration are bundled.

`src/share/hook_paths.rs` adapts the configured hook-folder scope semantics from Tessel `src/main/liveShare/shareScope.js` under the existing MIT attribution above, with independent bounded discovery, configuration-origin validation and pre-mutation rechecking. Tests contain synthetic paths and configuration only.

The optimistic guest-save version contract in `src/share/file_version.rs` and `src/guest_file_editor.rs` follows Tessel `src/main/liveShare/shareRequests.js` (`fs.write`, `expectHash`) under the existing MIT attribution above. The bounded hashing and guest-save serialization are implemented independently in Rust; tests use synthetic content only.

The Issues toolbar menu in `src/issues_menu.rs` and `src/gpui_ui/issues_menu.rs` adapts Tessel App.vue and style.css under the existing Orca/Tessel MIT attribution above, including its inline SVG geometry.

The durable task-preparation contract in src/task_preparation.rs and owner launch controller in src/app/task_launch.rs follow Tessel App.vue startTask and NewTaskDialog.vue under the existing Orca/Tessel MIT attribution above. Its bounded Rust records, transactional persistence and runtime ticket are implemented independently.

The task instruction formatter in `src/task_prompt.rs`, `scripts/extract_task_prompt_reference.mjs` and its synthetic reference fixture adapt Tessel `src/renderer/src/App.vue#taskPrompt` under the existing Orca/Tessel MIT attribution above. The extractor records the source hash and fictitious inputs only.

The local task form (`src/new_task_form.rs`, `src/app/new_task.rs`, `src/gpui_ui/new_task_view.rs`) follows Tessel `NewTaskDialog.vue` and its `style.css` rules under the existing Orca/Tessel MIT attribution above. Tests use fictitious projects and local temporary workspaces only.

- `src/task_completion.rs`, `scripts/extract_task_completion_reference.mjs` and synthetic `tests/fixtures/task_completion.json` reproduce the standalone completion marker contract from Tessel `src/renderer/src/agentLimit.js` (MIT, Lovecast 2026).

- `src/native_review.rs` and `src/app/task_review.rs` adapt Tessel `src/shared/tesselDelivery.js#reviewRequest` and `App.vue#agentReportedDone` under the existing Orca/Tessel MIT attribution. The conversation identity, bounded queue and persistence guards are native implementations.

- The parent-session variable filter in `src/agent_env.rs` follows Tessel `src/main/cleanEnv.js` under the existing Orca/Tessel MIT attribution. Rust broker capability names are additionally excluded.

- `src/worktree_setup.rs` and the opt-in worktree preparation flow follow Tessel `src/main/worktreeCreate.js#setupWorktree` and `NewTaskDialog.vue` under the existing Orca/Tessel MIT attribution. The bounded native process implementation reuses the existing Rust process-tree guard.

- Existing-agent task selection follows Tessel `NewTaskDialog.vue` and `App.vue#startTask` under the existing Orca/Tessel MIT attribution; native conversation epochs and transactional copy receipts are Rust-specific guards.

- Worktree origin metadata in `src/worktree.rs`, `src/workspace.rs` and the owner task controller follows Tessel `src/main/agentTools.js#createWorktree`, `src/main/worktreeCreate.js#resolveWorktreeBase` and `App.vue#taskPrompt` under the existing Orca/Tessel MIT attribution. Rust resolves the captured symbolic ref and describes unknown historical origins explicitly. Base validation follows the symbolic-branch/full-object-identity checks, including SHA-256 and hex-named-reference rejection; trust-provenance integration remains separate.

- Auxiliary-process parent-session filtering reuses `src/agent_env.rs`, derived from Tessel `src/main/cleanEnv.js` under the existing Orca/Tessel MIT attribution; native process integration and synthetic subprocess tests are independently implemented.

- `src/external_issue.rs` and the linked-issue controller restrictions follow Tessel renderer `externalIssues.js` under the existing Orca/Tessel MIT attribution. Rust reuses existing Linear origin/ID validation, adds external provenance and keeps the transient contract outside serialized state.
- `src/app/linear_task.rs` and the Linear launch controls follow `externalIssues.js` and `components/LinearDialog.vue` under that same attribution, with captured account authority and native first-turn callbacks.

- `src/cookie_import_model.rs` recovers the unfinished native cookie-export model based on Tessel `src/main/cookieImport/cookieFile.js`, `cookies.js` and `src/renderer/src/browser/cookieImportText.js`, under the Orca/Tessel MIT attribution above (Copyright (c) 2026 Lovecast Inc.). Only synthetic cookie fixtures are included; no browser profile or real credentials are bundled.

- `src/outline_tree.rs` and its synthetic reference vectors adapt Tessel `src/renderer/src/explorer/outlineTree.js` and the symbol shape in `editor/outline.js`, following the VS Code outline tree/range behavior cited by those files. The Orca/Tessel and Microsoft MIT notices above apply. The reference generator reads source code only; no user documents are bundled.

The CSS outline follows the document symbol rules of Microsoft’s vscode-css-languageservice bundled with Monaco and uses cssparser for CSS Syntax tokens. Its synthetic reference fixtures were generated from the installed provider under the Microsoft MIT notice above.

The HTML outline provider follows Microsoft’s vscode-html-languageservice parser and document-symbol naming. Its synthetic reference vectors use Monaco’s bundled provider under the Microsoft MIT notice above.

The JSON outline provider follows the document symbol behavior of Microsoft's vscode-json-languageservice bundled with Monaco. Its differential fixtures are generated from the installed provider with synthetic documents under the Microsoft MIT notice above.

The JavaScript and TypeScript outline parses source with the MIT-licensed Oxc Rust parser (https://github.com/oxc-project/oxc). Its synthetic differential fixtures use the TypeScript language service (Apache-2.0) bundled with Monaco and Monaco's MIT-licensed document-symbol adapter.

## Seti file icons


VS Code's file icon font and colours (`resources/file-icons/seti.ttf`
and `seti.json`) come from Seti UI, through VS Code's `theme-seti`.

- Project: https://github.com/jesseweed/seti-ui

```text
The MIT License (MIT)

Copyright (c) 2014 Jesse Weed
Copyright (c) Microsoft Corporation (VS Code theme and file associations)

Permission is hereby granted, free of charge, to any person obtaining
a copy of this software and associated documentation files (the
"Software"), to deal in the Software without restriction, including
without limitation the rights to use, copy, modify, merge, publish,
distribute, sublicense, and/or sell copies of the Software, and to
permit persons to whom the Software is furnished to do so, subject to
the following conditions:

The above copyright notice and this permission notice shall be
included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE
LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION
WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```
