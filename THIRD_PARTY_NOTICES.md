# Third-Party Notices

Tessel Rust includes or adapts the third-party material identified below. The original
copyright and permission notices are reproduced in full.

## Orca

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
