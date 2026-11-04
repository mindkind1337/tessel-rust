# Third-Party Notices

Tessel Rust includes or adapts the third-party material identified below. The original
copyright and permission notices are reproduced in full.

## portable-pty

`vendor/portable-pty` contains portable-pty 0.9.0 by Wez Furlong, under the MIT
license reproduced in `vendor/portable-pty/LICENSE.md`. Tessel's Windows patch
starts the child suspended, assigns a dedicated process job before execution,
terminates that job, and exposes verified descendant exit accounting. The patch
and upgrade requirements are recorded in `vendor/portable-pty/TESSEL_PATCH.md`.

## Orca

The pure conversation resume planner in `src/agent_resume.rs` adapts Tessel's
`agentResumeLine.js` and `remoteAgentLaunch.js`, including argument mappings from
the vendored Orca `agent-session-resume.js`. Its source-comparison fixture uses
synthetic session identifiers and paths only.

The bounded Codex metadata discovery in `src/codex_sessions.rs` adapts the folder,
launch-time and exclusion policy in Tessel's `src/main/agentSessions.js`.
Its tests use temporary synthetic rollouts, not user conversation records.

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

The native SSH Hosts settings interface (`src/ssh_hosts_panel.rs`) contains UI designs,
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

## Lucide Icons (`src/icons.rs`, `src/*_panel.rs`)

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
