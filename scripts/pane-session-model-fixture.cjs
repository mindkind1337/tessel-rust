// Development-only reference extraction for per-pane session model state, persistence,
// signatures, pure live-switch planning, Claude switch observation, and screen readers.
// Strictly read-only against the reference Tessel source tree; never spawns a provider CLI.

const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const crypto = require('node:crypto');

const argPath = process.argv[2];
if (!argPath) {
  throw new Error('Pass the reference Tessel repo or src directory (e.g. C:\\Tessel)');
}

const srcDir = fs.existsSync(path.join(argPath, 'shared', 'agentSessionOptions.js'))
  ? argPath
  : path.join(argPath, 'src');

if (!fs.existsSync(path.join(srcDir, 'shared', 'agentSessionOptions.js'))) {
  throw new Error(`Could not locate shared/agentSessionOptions.js under ${srcDir}`);
}

const context = vm.createContext({ console, Math, JSON, Object, Array, String, Number, Boolean, RegExp, Map, Set, Promise, Error });
const sources = [];

function recordFile(relPath) {
  const full = path.join(srcDir, relPath);
  const text = fs.readFileSync(full, 'utf8');
  sources.push({
    file: relPath.replace(/\\/g, '/'),
    sha256: crypto.createHash('sha256').update(text).digest('hex'),
  });
  return text;
}

function flattenLocale(obj, prefix = '', out = {}) {
  for (const [k, v] of Object.entries(obj || {})) {
    const key = prefix ? `${prefix}.${k}` : k;
    if (v && typeof v === 'object' && !Array.isArray(v)) flattenLocale(v, key, out);
    else if (typeof v === 'string') out[key] = v;
  }
  return out;
}

const frMessages = {};
for (const localeFile of [
  'renderer/src/i18n/locales/fr/panes.json',
  'renderer/src/i18n/locales/fr/chat.json',
  'renderer/src/i18n/locales/fr/app.json',
]) {
  Object.assign(frMessages, flattenLocale(JSON.parse(recordFile(localeFile))));
}

context.__locale = 'en';
context.__frMessages = frMessages;
vm.runInContext(
  `
  globalThis.t = function t(key, fallback, vars) {
    let text = globalThis.__locale === 'fr' ? globalThis.__frMessages[key] : undefined;
    if (text == null) text = fallback !== undefined ? fallback : key;
    if (!vars) return text;
    return String(text).replace(/\\{\\{\\s*(\\w+)\\s*\\}\\}/g, (m, name) => (name in vars ? String(vars[name]) : m));
  };
  `,
  context,
);

function evalModule(relPath) {
  const text = recordFile(relPath);
  const exported = [];
  const stripped = text
    .replace(/^import\s+[\s\S]*?from\s+['"][^'"]+['"]\s*;?\s*$/gm, '')
    .replace(/^export\s+class\s+([A-Za-z0-9_$]+)/gm, (_, name) => {
      exported.push(name);
      return `class ${name}`;
    })
    .replace(/^export\s+async\s+function\s+([A-Za-z0-9_$]+)/gm, (_, name) => {
      exported.push(name);
      return `async function ${name}`;
    })
    .replace(/^export\s+(?:const|let|var|function)\s+([A-Za-z0-9_$]+)/gm, (m, name) => {
      exported.push(name);
      return m.replace(/^export\s+/, '');
    });
  const assign = exported.map((n) => `globalThis.${n} = ${n};`).join('\n');
  vm.runInContext(`(() => {\n${stripped}\n${assign}\n})();`, context, { filename: relPath });
}

for (const modPath of [
  'shared/cursorModels.js',
  'shared/antigravityModels.js',
  'shared/agentSessionOptions.js',
  'shared/agentModelProbe.js',
  'shared/agentPrefs.js',
  'shared/modelLabel.js',
  'shared/codexScreen.js',
  'shared/screenModel.js',
  'shared/claudePricing.js',
  'renderer/src/paneLastModel.js',
  'renderer/src/sessionOptionLabels.js',
  'renderer/src/chat/orca/native-chat-session-option-labels.js',
  'renderer/src/claudeModelSwitch.js',
  'renderer/src/cliRequests.js',
]) {
  evalModule(modPath);
}

// Record TerminalPane.vue provenance hash as well so any upstream change is detected.
recordFile('renderer/src/components/TerminalPane.vue');
const cursorListRaw = recordFile('shared/__tests__/fixtures/cursor-list-models.txt');

const clone = (v) => (v === undefined ? null : JSON.parse(JSON.stringify(v)));

// Install pure simulation harness mirroring TerminalPane.vue's exact lines 324-930 & 1271-1279 & 1672.
vm.runInContext(
  `
  globalThis.newestClaudeOf = function newestClaudeOf(family) {
    let best = null;
    let bestKey = -1;
    for (const id of Object.keys(CLAUDE_PRICING)) {
      const m = new RegExp('^claude-' + family + '-(\\\\d+)(?:-(\\\\d+))?$').exec(id);
      if (!m) continue;
      const key = Number(m[1]) * 100 + Number(m[2] || 0);
      if (key > bestKey) {
        bestKey = key;
        best = id;
      }
    }
    return best;
  };

  globalThis.withModelVersion = function withModelVersion(shown, ...reported) {
    const bare = /^(opus|sonnet|haiku|fable)(\\[1m\\])?$/i.exec(String(shown.model || '').trim());
    if (!bare) return shown;
    const family = bare[1].toLowerCase();
    const full =
      reported.find((m) => typeof m === 'string' && new RegExp('^(?:claude-)?' + family + '-\\\\d', 'i').test(m.trim())) ||
      newestClaudeOf(family);
    return full ? { ...shown, model: full.trim() + (bare[2] && !/\\[1m\\]$/i.test(full) ? '[1m]' : '') } : shown;
  };

  globalThis.effectiveModelIdOf = function effectiveModelIdOf(list, shown) {
    const exact = list.find((m) => m.id === shown) || list.find((m) => m.label === shown);
    if (exact) return exact.id;
    const listed = listedModelValues(list, shown);
    if (listed) return listed.model;
    const alias = list.find((m) => shown.includes('-' + m.id + '-') || shown.endsWith('-' + m.id));
    return alias ? alias.id : null;
  };

  globalThis.computeEffectiveModelId = function computeEffectiveModelId(list, shownModel) {
    if (!list.length) return null;
    const found = shownModel ? effectiveModelIdOf(list, shownModel) : null;
    if (found) return found;
    const byDefault = list.find((m) => m.isDefault);
    return (byDefault || list[0]).id;
  };

  globalThis.evalChosenShown = function evalChosenShown(node, res, models) {
    const n = JSON.parse(JSON.stringify(node));
    if ((!n.modelChoice || !n.modelChoice.model) && n.sessionOptions && typeof n.sessionOptions.model === 'string' && n.sessionOptions.model) {
      if (res && res.source === 'session' && res.model) return { shown: res, modelChoice: n.modelChoice || null };
      const effort = typeof n.sessionOptions.effort === 'string' ? n.sessionOptions.effort : (res && res.chosenEffort) || null;
      return {
        shown: { model: modelChoiceLabel(models, n.sessionOptions.model), effort, source: 'chosen' },
        modelChoice: n.modelChoice || null,
      };
    }
    const c = n.modelChoice;
    if (!c || !c.model) return { shown: res, modelChoice: null };
    if (res && res.source === 'session') {
      if (res.at && c.at && !n.remoteHostId && res.at > c.at) {
        delete n.modelChoice;
        return { shown: res, modelChoice: null };
      }
      const said = res.model + '|' + (res.effort || '');
      if (c.seen === undefined || c.seen === null) c.seen = said;
      else if (said !== c.seen) {
        delete n.modelChoice;
        return { shown: res, modelChoice: null };
      }
    }
    return {
      shown: {
        model: modelChoiceLabel(models, c.model),
        effort: c.effort || (res && (res.chosenEffort || res.effort)) || null,
        source: 'chosen',
      },
      modelChoice: n.modelChoice || null,
    };
  };

  globalThis.formatHeaderAndTitles = function formatHeaderAndTitles(input) {
    const effortName = (effort) => nativeChatSessionChoiceLabel({ value: effort, label: effort });
    const m = input.agentModel;
    const modelText = !m || !m.model ? '' : (m.name || modelLabel(m.model)) + (m.effort ? ' · ' + effortName(m.effort) : '');
    let modelTitle = '';
    if (m) {
      const from =
        m.source === 'session'
          ? t('pane.model.fromSession', 'from its latest answer')
          : m.source === 'command'
            ? t('pane.model.fromCommand', 'from its command')
            : m.source === 'picked'
              ? t('pane.model.fromPicked', 'the model last picked in it')
              : m.source === 'chosen'
                ? t('pane.model.fromChosen', 'chosen in Tessel (pane menu > Model, or Settings > Agents)')
                : m.source === 'screen'
                  ? t('pane.model.fromScreen', 'read from its screen (status bar or banner)')
                  : m.source === 'saved'
                    ? t('pane.model.fromSaved', 'what it showed before Tessel restarted (read again in a moment)')
                    : m.source === 'running'
                      ? t('pane.model.fromRunning', 'the only model Ollama has running')
                      : t('pane.model.fromSettings', 'from its settings (a change inside the agent may not show)');
      const head = m.effort
        ? t('pane.model.titleEffort', 'Model: {{model}} (reasoning {{effort}})', { model: m.model, effort: effortName(m.effort) })
        : t('pane.model.title', 'Model: {{model}}', { model: m.model });
      modelTitle = head + '\\n' + from;
    }
    const headerModelText = input.guestRuns
      ? input.guestModel || ''
      : !input.isAgent || !m || !m.model
        ? ''
        : (m.name || modelLabel(m.model)) + (m.effort ? ' · ' + effortName(m.effort) : '');
    const chipLabel = headerModelText
      ? input.hasModelChoice && !input.guestRuns
        ? t('pane.sessionOptions.chipLabel', 'Model: {{model}}. Choose the model', { model: headerModelText })
        : t('pane.sessionOptions.chipStatic', 'Model: {{model}}', { model: headerModelText })
      : '';
    const settingsDef = resolveSessionOptionDefaults(input.persistedDefaults || {}, input.agentId);
    const modelDefaultLabel = settingsDef
      ? t('pane.sessionOptions.settingsDefault', 'Default from Settings ({{model}})', {
          model: sessionPillLabel(input.models || [], settingsDef),
        })
      : t('pane.sessionOptions.agentDefault', "Agent's own default");
    let modelMenuNote = '';
    if (!input.paneRunning) {
      modelMenuNote = t('pane.sessionOptions.appliesAtStart', 'Applies when the agent starts.');
    } else {
      const catalog = getAgentSessionOptionCatalog(input.agentId);
      const mid = catalog && catalog.modelApply.midSession;
      if (mid && mid.kind === 'picker-filter') {
        modelMenuNote = t(
          'pane.sessionOptions.cursorLive',
          'A model chosen here switches at once; its effort or Fast opens its own picker in the terminal (Tab on the model).',
        );
      } else {
        modelMenuNote = mid && mid.kind === 'command' ? '' : t('pane.sessionOptions.appliesAtRestart', 'A model picked here applies when the agent restarts.');
      }
    }
    const modelBusyReason =
      input.paneRunning && input.paneBusy
        ? t('pane.sessionOptions.waitIdle', 'It is working: its model can change once it is idle.')
        : '';
    const applyTitle = t(
      'pane.apply.titleModel',
      'Settings > Agents or its model changed since this agent started (Yolo, arguments, variables or model). Restart it to apply them: same pane, its conversation resumed',
    );
    const restartApplyBadge = t('pane.badge.apply', 'Restart to apply');
    return {
      modelText,
      headerModelText,
      chipLabel,
      modelTitle,
      modelDefaultLabel,
      modelMenuNote,
      modelBusyReason,
      applyTitle,
      restartApplyBadge,
    };
  };

  globalThis.simulatePanePick = function simulatePanePick(sim) {
    const n = JSON.parse(JSON.stringify(sim.node));
    const settings = sim.settings || { agentPrefs: {}, agentPermissions: 'manual', agentSessionOptions: {} };
    const hasModelChoice = !!sim.isAgent && !n.detected && !!getAgentSessionOptionCatalog(n.agentId);
    const paneModelList = hasModelChoice ? catalogModelsFor(n.agentId, sim.discovered || null) : [];
    const agentModel = sim.agentModel || null;
    const effectiveId = computeEffectiveModelId(paneModelList, agentModel && agentModel.model);
    const paneRunning = !!sim.paneRunning;
    const paneBusy = !!sim.paneBusy;
    const pending = !!sim.pending;

    const signatureNow = (choice = n.sessionOptions) => {
      const values = launchSessionValues(choice, settings.agentSessionOptions, n.agentId);
      return launchSignature(
        effectiveAgent(
          { id: n.agentId, command: n.agentCommand || n.agentId },
          settings.agentPrefs,
          sim.permissions || settings.agentPermissions || 'manual',
          values,
          paneModelList,
        ),
      );
    };

    const paneMenuValues =
      n.agentId === 'cursor' && paneRunning && agentModel && agentModel.source === 'screen'
        ? listedModelValues(paneModelList, agentModel.model) || n.sessionOptions || null
        : n.sessionOptions || null;
    const paneValues = launchSessionValues(n.sessionOptions, settings.agentSessionOptions, n.agentId);

    function nextPaneValues(optionId, value) {
      const base = (n.agentId === 'cursor' && paneRunning && paneMenuValues) || paneValues;
      const current = valuesOnListedRow(paneModelList, base);
      if (optionId === 'model') {
        if (!value) return null;
        const next = { model: value };
        const catalog = getAgentSessionOptionCatalog(n.agentId);
        const effort = current && current.effort;
        const offers = modelOptions(catalog, paneModelList, value).some(
          (o) => o.id === 'effort' && o.kind.type === 'select' && o.kind.choices.some((c) => c.value === effort),
        );
        if (effort && offers) next.effort = effort;
        return next;
      }
      if (!current || !current.model) {
        const model = effectiveId;
        if (!model || value === null || value === undefined) return current || null;
        return { model, [optionId]: value };
      }
      const next = { ...current };
      if (value === null || value === undefined) delete next[optionId];
      else next[optionId] = value;
      return next;
    }

    function setPaneChoice(next) {
      if (next) n.sessionOptions = next;
      else delete n.sessionOptions;
    }

    function adoptLive(next) {
      const wasCurrent = !!n.launchSig && signatureNow() === n.launchSig;
      setPaneChoice(next);
      if (wasCurrent) n.launchSig = signatureNow();
      if (next && next.model) {
        n.modelChoice = { model: next.model, effort: typeof next.effort === 'string' ? next.effort : null, at: sim.nowMs || 1700000000000 };
      }
    }

    function optionApply(catalog, optionId, modelId) {
      if (optionId === 'model') return catalog.modelApply;
      const option = modelOptions(catalog, paneModelList, modelId).find((o) => o.id === optionId);
      if (option && option.apply.composedIntoModel && catalog.composeModelValue) return catalog.modelApply;
      return option ? option.apply : null;
    }

    const typedCommands = [];
    let toast = null;
    let holdClaudeCalled = false;

    if (sim.mode === 'action') {
      const catalog = getAgentSessionOptionCatalog(n.agentId);
      if (!catalog || !paneRunning || paneBusy) {
        return { outcome: null, typedCommands, toast, node: n };
      }
      const apply = optionApply(catalog, sim.optionId, paneValues && paneValues.model);
      const mid = apply && apply.midSession;
      if (!mid || (mid.kind !== 'toggle-command' && mid.kind !== 'agent-picker')) {
        return { outcome: null, typedCommands, toast, node: n };
      }
      typedCommands.push({ command: mid.command, delivery: mid.delivery === 'type' ? 'type' : 'write' });
      return { outcome: 'action-sent', typedCommands, toast, node: n };
    }

    function runApplyPick(optionId, value) {
      const catalog = getAgentSessionOptionCatalog(n.agentId);
      if (!catalog) return null;
      const next = nextPaneValues(optionId, value);
      const apply = optionApply(catalog, optionId, next && next.model);
      const mid = apply && apply.midSession;

      if (paneRunning && value !== null && value !== undefined && mid && mid.kind === 'picker-filter') {
        if (paneBusy || pending) return 'busy';
        const row = optionId === 'model' ? paneModelList.find((m) => m.id === value) : null;
        const filter = cursorPickerFilter(row);
        typedCommands.push({ command: mid.build(filter), delivery: 'write' });
        return filter ? 'sent' : 'picker';
      }
      if (!paneRunning || value === null || value === undefined || !mid || mid.kind !== 'command') {
        setPaneChoice(next);
        return 'saved';
      }
      if (paneBusy || pending) return 'busy';
      if (mid.detectAgentInteraction === 'claude-model-switch-confirmation') {
        holdClaudeCalled = true;
        typedCommands.push({
          command: '/model ' + value,
          delivery: 'write',
          expectedModelLabel: modelChoiceLabel(paneModelList, value),
        });
        const outcome = sim.claudeOutcome || 'applied';
        if (outcome === 'applied') adoptLive(next);
        else if (outcome !== 'rejected') setPaneChoice(next);
        return outcome === 'applied' || outcome === 'rejected' ? outcome : 'unknown';
      }
      const sent = catalog.composeModelValue && apply === catalog.modelApply ? composedModelId(n.agentId, next, paneModelList) : value;
      typedCommands.push({ command: mid.build(sent), delivery: mid.delivery === 'type' ? 'type' : 'write' });
      adoptLive(next);
      return 'sent';
    }

    if (sim.mode === 'chat') {
      const chatAvailable = !!sim.chatViewAvailable && hasModelChoice;
      const optionId = sim.optionId;
      const value = sim.value;
      const unsupported = t('chat.orca.options.unsupported', 'This option is not available for this agent.');
      if (!chatAvailable || !['model', 'effort'].includes(optionId) || typeof value !== 'string' || !value) {
        return { chatResult: { ok: false, error: unsupported }, typedCommands, node: n };
      }
      const hasEffort = modelOptions(getAgentSessionOptionCatalog(n.agentId), paneModelList, effectiveId).some((o) => o.id === 'effort');
      if (optionId === 'effort' && !hasEffort) {
        return { chatResult: { ok: false, error: t('pane.sessionOptions.noEffort', 'This model has no reasoning effort to choose.') }, typedCommands, node: n };
      }
      if (!paneRunning) {
        return { chatResult: { ok: false, error: t('pane.sessionOptions.appliesAtStart', 'Applies when the agent starts.') }, typedCommands, node: n };
      }
      if (paneBusy || pending) {
        return { chatResult: { ok: false, error: t('pane.sessionOptions.waitIdle', 'It is working: its model can change once it is idle.') }, typedCommands, node: n };
      }
      if (sim.userTyping) {
        return { chatResult: { ok: false, error: t('pane.chatView.typedLine', 'A line is typed in its terminal: send or clear it there first.') }, typedCommands, node: n };
      }
      const outcome = runApplyPick(optionId, value);
      let chatResult;
      if (outcome === 'applied' || outcome === 'sent') chatResult = { ok: true };
      else if (outcome === 'picker') chatResult = { ok: true, picker: true };
      else if (outcome === 'rejected') chatResult = { ok: false, error: t('pane.sessionOptions.kept', 'Claude kept the current model.') };
      else if (outcome === 'unknown') chatResult = { ok: false, error: t('pane.sessionOptions.unverified', 'Could not verify the model change; open the terminal to check.') };
      else if (outcome === 'busy') chatResult = { ok: false, error: t('pane.sessionOptions.waitIdle', 'It is working: its model can change once it is idle.') };
      else chatResult = { ok: false, error: t('pane.sessionOptions.appliesAtRestart', 'A model picked here applies when the agent restarts.') };
      return { chatResult, typedCommands, holdClaudeCalled, node: n };
    }

    const outcome = runApplyPick(sim.optionId, sim.value);
    if (outcome === 'picker') {
      if (n.chatView) delete n.chatView;
      toast = { text: t('pane.sessionOptions.pickerOpened', 'Its model picker is open in the terminal: Tab on the model changes its effort or Fast.'), kind: 'info', timeout: 6000 };
    } else if (outcome === 'rejected') {
      toast = { text: t('pane.sessionOptions.kept', 'Claude kept the current model.'), kind: 'error', timeout: null };
    } else if (outcome === 'unknown') {
      toast = { text: t('pane.sessionOptions.unverified', 'Could not verify the model change; open the terminal to check.'), kind: 'error', timeout: 8000 };
    } else if (outcome === 'sent') {
      toast = { text: t('pane.sessionOptions.sentNotConfirmed', 'Sent to the agent — not confirmed'), kind: 'info', timeout: 3000 };
    }
    const launchStaleAfter = !!(n.launchSig && n.agentCommand && !n.detected && signatureNow() !== n.launchSig);
    return {
      outcome,
      typedCommands,
      toast,
      holdClaudeCalled,
      launchStaleAfter,
      paneMenuValuesBefore: paneMenuValues,
      effectiveIdBefore: effectiveId,
      node: n,
    };
  };
  `,
  context,
);

// ---------------------------------------------------------------------------
// 1. validPaneSessionOptions
// ---------------------------------------------------------------------------
const validPaneCases = [
  { name: 'null', input: null },
  { name: 'array', input: ['opus'] },
  { name: 'empty_object', input: {} },
  { name: 'missing_model', input: { effort: 'high' } },
  { name: 'empty_model', input: { model: '' } },
  { name: 'unsafe_flag_model', input: { model: '--yolo' } },
  { name: 'space_in_model', input: { model: 'opus 4' } },
  { name: 'valid_model_only', input: { model: 'claude-opus-5-5' } },
  { name: 'valid_bracket_1m_and_provider', input: { model: 'anthropic/claude-opus-4-7[1m]', effort: 'xhigh', fastMode: true } },
  { name: 'valid_cursor_composed', input: { model: 'gpt-5.3-codex', effort: 'high', fastMode: false, thinking: true } },
  { name: 'null_option_value_rejected', input: { model: 'opus', effort: null } },
  { name: 'number_option_value_rejected', input: { model: 'opus', effort: 2 } },
  { name: 'unsafe_option_value_rejected', input: { model: 'opus', effort: '--high' } },
  { name: 'hyphen_in_option_key_rejected', input: { model: 'opus', 'fast-mode': true } },
  { name: 'digit_start_option_key_rejected', input: { model: 'opus', '1fast': true } },
  { name: 'overlong_option_key_rejected', input: { model: 'opus', ['a'.repeat(41)]: true } },
  { name: 'max_length_option_key_ok', input: { model: 'opus', ['a'.repeat(40)]: 'high' } },
  { name: 'max_160_model_ok', input: { model: 'm'.repeat(160) } },
  { name: 'over_160_model_rejected', input: { model: 'm'.repeat(161) } },
  { name: 'ten_entries_before_model_keeps_eleven_and_drops_twelfth', input: { a0: '0', a1: '1', a2: '2', a3: '3', a4: '4', a5: '5', a6: '6', a7: '7', a8: '8', a9: '9', model: 'opus', z10: 'h' } },
  {
    name: 'ten_entries_ok',
    input: {
      model: 'opus',
      k1: true,
      k2: false,
      k3: 'a',
      k4: 'b',
      k5: 'c',
      k6: 'd',
      k7: 'e',
      k8: 'f',
      k9: 'g',
    },
  },
  {
    name: 'eleven_entries_rejected',
    input: {
      model: 'opus',
      k1: true,
      k2: false,
      k3: 'a',
      k4: 'b',
      k5: 'c',
      k6: 'd',
      k7: 'e',
      k8: 'f',
      k9: 'g',
      z10: 'h',
    },
  },
].map((c) => {
  context.__in = c.input;
  return { ...c, expected: clone(vm.runInContext('validPaneSessionOptions(__in)', context)) };
});

// ---------------------------------------------------------------------------
// 2. launchSessionValues & launchSignature
// ---------------------------------------------------------------------------
const persistedSample = {
  claude: {
    model: 'sonnet',
    valuesByModel: {
      sonnet: { effort: 'high' },
      opus: { effort: 'max', fastMode: true },
    },
  },
  codex: {
    model: null,
    valuesByModel: {
      'gpt-5.6-sol': { effort: 'ultra' },
    },
  },
};

const launchValuesCases = [
  { name: 'pane_model_overrides_settings_whole_without_mixing_effort', agentId: 'claude', paneChoice: { model: 'opus' }, persisted: persistedSample },
  { name: 'pane_model_and_effort_used', agentId: 'claude', paneChoice: { model: 'opus', effort: 'low' }, persisted: persistedSample },
  { name: 'null_pane_falls_back_to_settings_defaults', agentId: 'claude', paneChoice: null, persisted: persistedSample },
  { name: 'empty_pane_model_falls_back_to_settings_defaults', agentId: 'claude', paneChoice: { model: '' }, persisted: persistedSample },
  { name: 'cleared_settings_model_returns_null', agentId: 'codex', paneChoice: null, persisted: persistedSample },
  { name: 'unknown_agent_returns_null', agentId: 'aider', paneChoice: null, persisted: persistedSample },
].map((c) => {
  context.__c = c;
  return {
    ...c,
    expected: clone(vm.runInContext('launchSessionValues(__c.paneChoice, __c.persisted, __c.agentId)', context)),
  };
});

const launchSignatureCases = [
  { name: 'minimal_empty', launch: { command: 'claude', args: '', env: {} } },
  { name: 'sorted_env_and_args', launch: { command: 'codex', args: '-m gpt-5.6-sol -c model_reasoning_effort=high', env: { Z_VAR: 'last', A_VAR: 'first', M_VAR: 'mid=val' } } },
  { name: 'quotes_and_unicode_in_args', launch: { command: 'C:\\Tools\\claude.exe', args: '--model "claude-opus-5-5" --effort max', env: { LANG: 'fr_CA.UTF-8' } } },
].map((c) => {
  context.__l = c.launch;
  return { ...c, expected: vm.runInContext('launchSignature(__l)', context) };
});

// ---------------------------------------------------------------------------
const effectiveAgentCases = [
  { name: 'claude_yolo_with_session_values', agent: { id: 'claude', command: 'claude' }, prefs: {}, permissions: 'yolo', values: { model: 'opus', effort: 'max' } },
  { name: 'claude_manual_strips_legacy_yolo_arg', agent: { id: 'claude', command: 'claude' }, prefs: { claude: { command: '', args: '--dangerously-skip-permissions --verbose', env: 'FOO=1' } }, permissions: 'manual', values: { model: 'sonnet', effort: 'high' } },
  { name: 'goose_yolo_env_overlaid_by_user_env', agent: { id: 'goose', command: 'goose' }, prefs: { goose: { command: 'goose-cli', args: '', env: 'EXTRA=yes' } }, permissions: 'yolo', values: null },
  { name: 'goose_custom_args_suppresses_yolo_env', agent: { id: 'goose', command: 'goose' }, prefs: { goose: { command: '', args: '--profile dev', env: '' } }, permissions: 'yolo', values: null },
  { name: 'codex_user_model_flag_wins_over_session_values', agent: { id: 'codex', command: 'codex' }, prefs: { codex: { command: '', args: '-m o3', env: '' } }, permissions: 'yolo', values: { model: 'gpt-5.6-sol', effort: 'xhigh' } },
].map((c) => {
  context.__ea = c;
  const launch = clone(vm.runInContext('effectiveAgent(__ea.agent, __ea.prefs, __ea.permissions, __ea.values, catalogModelsFor(__ea.agent.id, null))', context));
  context.__eal = launch;
  const signature = vm.runInContext('launchSignature(__eal)', context);
  return { ...c, expectedLaunch: launch, expectedSignature: signature };
});

// 3. savedPaneModel, paneModelToSave, samePaneModel
// ---------------------------------------------------------------------------
const savedModelCases = [
  { name: 'null', input: null },
  { name: 'empty_model', input: { model: '   ' } },
  { name: 'valid_full', input: { model: ' claude-opus-5-5 ', name: ' Opus 5.5 ', effort: 'high', source: 'session' } },
  { name: 'control_char_in_model_rejected', input: { model: 'opus\n5' } },
  { name: 'del_char_in_model_rejected', input: { model: 'opus\u007f' } },
  { name: 'control_char_in_name_drops_name_only', input: { model: 'claude-opus-5-5', name: 'bad\tname', effort: 'medium', source: 'screen' } },
  { name: 'uppercase_effort_dropped', input: { model: 'gpt-5.6-sol', effort: 'High', source: 'screen' } },
  { name: 'short_1char_effort_dropped', input: { model: 'gpt-5.6-sol', effort: 'x', source: 'screen' } },
  { name: 'saved_source_not_resaved', input: { model: 'claude-opus-5-5', effort: 'high', source: 'saved' } },
  { name: 'over_120_model_rejected', input: { model: 'x'.repeat(121), source: 'session' } },
  { name: 'exact_120_model_accepted', input: { model: 'x'.repeat(120), source: 'session' } },
].map((c) => {
  context.__m = c.input;
  return {
    ...c,
    expectedSaved: clone(vm.runInContext('savedPaneModel(__m)', context)),
    expectedToSave: clone(vm.runInContext('paneModelToSave(__m)', context)),
  };
});

// ---------------------------------------------------------------------------
// 4. sessionChoiceError (EN & FR)
// ---------------------------------------------------------------------------
const cursorParsedRows = vm.runInContext(`parseCursorModelList(${JSON.stringify(cursorListRaw)})`, context);
const cursorGroupedModels = clone(vm.runInContext(`catalogModelsFor('cursor', groupCursorModels(${JSON.stringify(cursorParsedRows)}))`, context));

const choiceErrorInputs = [
  { name: 'empty_model_ok', agentId: 'claude', model: '', effort: '', models: null },
  { name: 'unknown_agent_aider', agentId: 'aider', model: 'gpt-5', effort: '', models: null },
  { name: 'valid_claude_opus_xhigh', agentId: 'claude', model: 'opus', effort: 'xhigh', models: null },
  { name: 'unknown_claude_model', agentId: 'claude', model: 'claude-99', effort: '', models: null },
  { name: 'unsafe_model_flag', agentId: 'claude', model: '--model=opus', effort: '', models: null },
  { name: 'empty_catalog_opencode_unknown_model', agentId: 'opencode', model: 'anthropic/claude', effort: '', models: [] },
  { name: 'haiku_has_no_effort', agentId: 'claude', model: 'haiku', effort: 'low', models: null },
  { name: 'claude_unknown_effort', agentId: 'claude', model: 'opus', effort: 'ultra', models: null },
  { name: 'cursor_grouped_row_id_ok', agentId: 'cursor', model: 'gpt-5.3-codex', effort: 'high', models: cursorGroupedModels },
  { name: 'cursor_exact_variant_id_ok', agentId: 'cursor', model: 'gpt-5.3-codex-high-fast', effort: '', models: cursorGroupedModels },
];

const sessionChoiceErrorCases = choiceErrorInputs.map((c) => {
  context.__ce = c;
  context.__locale = 'en';
  const en = vm.runInContext('sessionChoiceError(__ce.agentId, __ce.model, __ce.effort, __ce.models)', context);
  context.__locale = 'fr';
  const fr = vm.runInContext('sessionChoiceError(__ce.agentId, __ce.model, __ce.effort, __ce.models)', context);
  context.__locale = 'en';
  return {
    name: c.name,
    agentId: c.agentId,
    model: c.model,
    effort: c.effort,
    useCursorFixtureModels: c.models === cursorGroupedModels,
    expectedEn: en,
    expectedFr: fr,
  };
});

// ---------------------------------------------------------------------------
// 5. effortLevel & modelLabel
// ---------------------------------------------------------------------------
const effortInputs = [
  '',
  'none',
  'off',
  'OFF',
  'disabled',
  'minimal',
  'low',
  'MEDIUM',
  'high',
  'xhigh',
  'x high',
  'x_high',
  'Extra-High',
  '  extra   high ',
  'max',
  'ultra',
  'fast',
  'thinking',
  'unknown',
];
const effortLevelCases = effortInputs.map((input) => {
  context.__e = input;
  return { input, expected: vm.runInContext('effortLevel(__e)', context) };
});

const modelLabelInputs = [
  '',
  '   ',
  'opus',
  'OPUS',
  'opus[1m]',
  'claude-opus-5-5',
  'claude-opus-5',
  'claude-sonnet-4-5-20250929',
  'claude-opus-4-20250929',
  'claude-opus-20250929',
  'claude-fable-5-1[1M]',
  'claude-haiku-4-5-20251001',
  'claude-opus-4-100',
  'anthropic/claude-sonnet-5',
  'anthropic/opus[1m]',
  'gpt-6-astra',
  'gpt-5.2-codex',
  'gpt-5.4-mini',
  'gpt-5.4-NANO',
  'gpt-5.5[1m]',
  'openai/gpt-5.6-sol',
  'muse-spark-1.3',
  'muse-spark-1.3-contributor',
  'muse-wonder-1.0[1m]',
  'google/gemini-2.5-pro',
  'gemini-3-flash-preview[1m]',
  'kimi-k2.6',
  'o3-mini',
];
const modelLabelCases = modelLabelInputs.map((input) => {
  context.__ml = input;
  return { input, expected: vm.runInContext('modelLabel(__ml)', context) };
});

// ---------------------------------------------------------------------------
// 6. Header & Title formatting (EN & FR)
// ---------------------------------------------------------------------------
const headerTitleInputs = [
  {
    name: 'claude_session_opus_medium',
    isAgent: true,
    hasModelChoice: true,
    guestRuns: false,
    agentId: 'claude',
    paneRunning: true,
    paneBusy: false,
    models: vm.runInContext("catalogModelsFor('claude', null)", context),
    persistedDefaults: persistedSample,
    agentModel: { model: 'claude-opus-5-5', effort: 'medium', source: 'session' },
  },
  {
    name: 'codex_screen_with_custom_name_xhigh_busy',
    isAgent: true,
    hasModelChoice: true,
    guestRuns: false,
    agentId: 'codex',
    paneRunning: true,
    paneBusy: true,
    models: vm.runInContext("catalogModelsFor('codex', null)", context),
    persistedDefaults: {},
    agentModel: { model: 'gpt-5.6-sol', name: 'GPT-5.6 Sol', effort: 'xhigh', source: 'screen' },
  },
  {
    name: 'cursor_running_note_and_chosen_source',
    isAgent: true,
    hasModelChoice: true,
    guestRuns: false,
    agentId: 'cursor',
    paneRunning: true,
    paneBusy: false,
    models: vm.runInContext("catalogModelsFor('cursor', null)", context),
    persistedDefaults: {},
    agentModel: { model: 'gpt-5.3-codex-high-fast', name: 'GPT-5.3 Codex Fast', effort: 'high', source: 'chosen' },
  },
  {
    name: 'offline_saved_source_none_effort',
    isAgent: true,
    hasModelChoice: true,
    guestRuns: false,
    agentId: 'muse',
    paneRunning: false,
    paneBusy: false,
    models: [],
    persistedDefaults: {},
    agentModel: { model: 'muse-spark-1.3', effort: 'none', source: 'saved' },
  },
  {
    name: 'detected_shell_agent_static_chip_command_source',
    isAgent: true,
    hasModelChoice: false,
    guestRuns: false,
    agentId: 'aider',
    paneRunning: true,
    paneBusy: false,
    models: [],
    persistedDefaults: {},
    agentModel: { model: 'deepseek/deepseek-r1', effort: null, source: 'command' },
  },
  {
    name: 'picked_running_and_settings_sources',
    isAgent: true,
    hasModelChoice: false,
    guestRuns: false,
    agentId: 'ollama',
    paneRunning: true,
    paneBusy: false,
    models: [],
    persistedDefaults: {},
    agentModel: { model: 'qwen3:32b', effort: 'ultra', source: 'running' },
  },
  {
    name: 'picked_source',
    isAgent: true,
    hasModelChoice: true,
    guestRuns: false,
    agentId: 'gemini',
    paneRunning: true,
    paneBusy: false,
    models: vm.runInContext("catalogModelsFor('gemini', null)", context),
    persistedDefaults: {},
    agentModel: { model: 'gemini-3-pro-preview', effort: null, source: 'picked' },
  },
  {
    name: 'settings_source',
    isAgent: true,
    hasModelChoice: true,
    guestRuns: false,
    agentId: 'claude',
    paneRunning: true,
    paneBusy: false,
    models: vm.runInContext("catalogModelsFor('claude', null)", context),
    persistedDefaults: {},
    agentModel: { model: 'claude-sonnet-5-5', effort: 'low', source: 'settings' },
  },
  {
    name: 'liveshare_guest_header_override',
    isAgent: false,
    hasModelChoice: false,
    guestRuns: true,
    guestModel: 'Opus 5.5 · Élevé',
    agentId: 'claude',
    paneRunning: false,
    paneBusy: false,
    models: [],
    persistedDefaults: {},
    agentModel: null,
  },
];

const headerTitleCases = headerTitleInputs.map((input) => {
  context.__ht = input;
  context.__locale = 'en';
  const expectedEn = clone(vm.runInContext('formatHeaderAndTitles(__ht)', context));
  context.__locale = 'fr';
  const expectedFr = clone(vm.runInContext('formatHeaderAndTitles(__ht)', context));
  context.__locale = 'en';
  return { ...input, expectedEn, expectedFr };
});

// ---------------------------------------------------------------------------
// 7. chosenShown & withModelVersion
// ---------------------------------------------------------------------------
const claudeModels = clone(vm.runInContext("catalogModelsFor('claude', null)", context));
const chosenShownCases = [
  {
    name: 'session_options_shows_chosen_until_session_speaks',
    node: { agentId: 'claude', sessionOptions: { model: 'opus', effort: 'max' } },
    res: { model: 'claude-sonnet-5-5', chosenEffort: 'high', source: 'settings' },
    models: claudeModels,
  },
  {
    name: 'session_options_yields_when_session_reports_model',
    node: { agentId: 'claude', sessionOptions: { model: 'opus', effort: 'max' } },
    res: { model: 'claude-opus-5-5', effort: 'max', source: 'session', at: 200 },
    models: claudeModels,
  },
  {
    name: 'optimistic_choice_wins_over_older_session_turn',
    node: { agentId: 'claude', modelChoice: { model: 'opus', effort: 'high', at: 500 } },
    res: { model: 'claude-sonnet-5-5', effort: 'medium', source: 'session', at: 400 },
    models: claudeModels,
  },
  {
    name: 'optimistic_choice_yields_and_clears_on_newer_local_session_turn',
    node: { agentId: 'claude', modelChoice: { model: 'opus', effort: 'high', at: 500 } },
    res: { model: 'claude-opus-5-5', effort: 'max', source: 'session', at: 600 },
    models: claudeModels,
  },
  {
    name: 'ssh_remote_ignores_clock_skew_arms_seen_first_then_yields_on_change',
    node: { agentId: 'claude', remoteHostId: 'prod-ssh', modelChoice: { model: 'opus', effort: 'high', at: 500 } },
    res: { model: 'claude-sonnet-5-5', effort: 'medium', source: 'session', at: 999999 },
    models: claudeModels,
  },
  {
    name: 'ssh_remote_second_changed_answer_clears_model_choice',
    node: {
      agentId: 'claude',
      remoteHostId: 'prod-ssh',
      modelChoice: { model: 'opus', effort: 'high', at: 500, seen: 'claude-sonnet-5-5|medium' },
    },
    res: { model: 'claude-opus-5-5', effort: 'high', source: 'session', at: 999999 },
    models: claudeModels,
  },
].map((c) => {
  context.__cs = c;
  return {
    ...c,
    expected: clone(vm.runInContext('evalChosenShown(__cs.node, __cs.res, __cs.models)', context)),
  };
});

const withModelVersionCases = [
  { shown: { model: 'opus', source: 'chosen' }, reported: [] },
  { shown: { model: 'Opus[1m]', source: 'chosen' }, reported: ['claude-opus-4-7'] },
  { shown: { model: 'opus[1m]', source: 'chosen' }, reported: ['claude-opus-5-5[1m]'] },
  { shown: { model: 'sonnet', source: 'chosen' }, reported: ['claude-opus-5-5', 'claude-sonnet-4-6'] },
  { shown: { model: 'haiku', source: 'chosen' }, reported: [] },
  { shown: { model: 'fable[1M]', source: 'chosen' }, reported: [] },
  { shown: { model: 'claude-opus-5-5', source: 'session' }, reported: ['claude-opus-4-8'] },
].map((c) => {
  context.__wv = c;
  return {
    ...c,
    expected: clone(vm.runInContext('withModelVersion(__wv.shown, ...__wv.reported)', context)),
  };
});

// ---------------------------------------------------------------------------
// 8. Live Pick, Action & ChatView Planning across all 12 cataloged agents
// ---------------------------------------------------------------------------
const discoveredByAgent = {
  cursor: groupCursorModelsSim(),
  antigravity: [
    { id: 'gemini-4-argon', label: 'Gemini 4 Argon', efforts: ['low', 'medium', 'high', 'max'], options: vm.runInContext("createAntigravityCatalogOptions(['low','medium','high','max'])", context) },
    { id: 'gpt-oss-120b', label: 'GPT-OSS 120B (Medium)', options: [] },
  ],
  muse: [
    { id: 'muse-spark-1.3', label: 'Muse Spark 1.3', options: vm.runInContext("createMuseCatalogOptions(['minimal','low','medium','high','max'], 'high')", context) },
  ],
  opencode: [
    { id: 'anthropic/claude-sonnet-4-5', label: 'Claude Sonnet 4.5', options: [] },
  ],
  pi: [
    { id: 'claude-sonnet-4-5', label: 'Claude Sonnet 4.5', options: vm.runInContext("createPiCatalogOptions(['off','low','medium','high'])", context) },
  ],
};

function groupCursorModelsSim() {
  return vm.runInContext(`groupCursorModels(${JSON.stringify(cursorParsedRows)})`, context);
}

function makeInitialLaunchSig(agentId, sessionOptions = null, discovered = null, command = agentId) {
  context.__init = { agentId, sessionOptions, discovered, command };
  return vm.runInContext(
    `(() => {
      const models = catalogModelsFor(__init.agentId, __init.discovered);
      const values = launchSessionValues(__init.sessionOptions, {}, __init.agentId);
      return launchSignature(effectiveAgent({ id: __init.agentId, command: __init.command }, {}, 'manual', values, models));
    })()`,
    context,
  );
}

const livePickScenarios = [
  // Claude: /model (applied, rejected, unknown), /effort, /fast action, reset to default, busy, offline
  {
    name: 'claude_live_model_applied_adopts_without_launch_stale',
    isAgent: true,
    paneRunning: true,
    paneBusy: false,
    claudeOutcome: 'applied',
    optionId: 'model',
    value: 'opus',
    node: { agentId: 'claude', agentCommand: 'claude', sessionOptions: { model: 'sonnet', effort: 'high' }, launchSig: makeInitialLaunchSig('claude', { model: 'sonnet', effort: 'high' }) },
  },
  {
    name: 'claude_live_model_rejected_keeps_previous_choice',
    isAgent: true,
    paneRunning: true,
    paneBusy: false,
    claudeOutcome: 'rejected',
    optionId: 'model',
    value: 'opus',
    node: { agentId: 'claude', agentCommand: 'claude', sessionOptions: { model: 'sonnet', effort: 'high' }, launchSig: makeInitialLaunchSig('claude', { model: 'sonnet', effort: 'high' }) },
  },
  {
    name: 'claude_live_model_unknown_saves_choice_and_marks_stale',
    isAgent: true,
    paneRunning: true,
    paneBusy: false,
    claudeOutcome: 'unknown',
    optionId: 'model',
    value: 'opus',
    node: { agentId: 'claude', agentCommand: 'claude', sessionOptions: { model: 'sonnet', effort: 'high' }, launchSig: makeInitialLaunchSig('claude', { model: 'sonnet', effort: 'high' }) },
  },
  {
    name: 'claude_switching_to_haiku_drops_incompatible_effort',
    isAgent: true,
    paneRunning: true,
    paneBusy: false,
    claudeOutcome: 'applied',
    optionId: 'model',
    value: 'haiku',
    node: { agentId: 'claude', agentCommand: 'claude', sessionOptions: { model: 'opus', effort: 'max' }, launchSig: makeInitialLaunchSig('claude', { model: 'opus', effort: 'max' }) },
  },
  {
    name: 'claude_live_effort_without_prior_choice_anchors_effective_model',
    isAgent: true,
    paneRunning: true,
    paneBusy: false,
    optionId: 'effort',
    value: 'max',
    agentModel: { model: 'claude-opus-5-5', effort: 'high', source: 'session' },
    node: { agentId: 'claude', agentCommand: 'claude', launchSig: makeInitialLaunchSig('claude', null) },
  },
  {
    name: 'claude_reset_model_to_default_saves_for_restart',
    isAgent: true,
    paneRunning: true,
    paneBusy: false,
    optionId: 'model',
    value: null,
    node: { agentId: 'claude', agentCommand: 'claude', sessionOptions: { model: 'opus', effort: 'high' }, launchSig: makeInitialLaunchSig('claude', { model: 'opus', effort: 'high' }) },
  },
  {
    name: 'claude_action_fast_toggle',
    mode: 'action',
    isAgent: true,
    paneRunning: true,
    paneBusy: false,
    optionId: 'fastMode',
    node: { agentId: 'claude', agentCommand: 'claude', sessionOptions: { model: 'opus' } },
  },
  // Codex: model pick saves for restart (launchStale=true), action types /model char-by-char
  {
    name: 'codex_live_model_pick_saves_for_restart',
    isAgent: true,
    paneRunning: true,
    paneBusy: false,
    optionId: 'model',
    value: 'gpt-5.6-sol',
    node: { agentId: 'codex', agentCommand: 'codex', launchSig: makeInitialLaunchSig('codex', null) },
  },
  {
    name: 'codex_action_effort_opens_agent_picker_typed',
    mode: 'action',
    isAgent: true,
    paneRunning: true,
    paneBusy: false,
    optionId: 'effort',
    node: { agentId: 'codex', agentCommand: 'codex', sessionOptions: { model: 'gpt-5.6-sol' } },
  },
  // Cursor: model pick types `/model <plainFilter>` without saving sessionOptions; effort pick opens `/model` and closes chatView
  {
    name: 'cursor_live_model_pick_types_plain_filter_without_saving',
    isAgent: true,
    paneRunning: true,
    paneBusy: false,
    discovered: discoveredByAgent.cursor,
    optionId: 'model',
    value: 'claude-opus-5',
    node: { agentId: 'cursor', agentCommand: 'cursor-agent', chatView: true, launchSig: makeInitialLaunchSig('cursor', null, discoveredByAgent.cursor, 'cursor-agent') },
  },
  {
    name: 'cursor_live_effort_pick_opens_picker_and_closes_chat_view',
    isAgent: true,
    paneRunning: true,
    paneBusy: false,
    discovered: discoveredByAgent.cursor,
    agentModel: { model: 'gpt-5.3-codex-high-fast', source: 'screen' },
    optionId: 'effort',
    value: 'low',
    node: { agentId: 'cursor', agentCommand: 'cursor-agent', chatView: true, launchSig: makeInitialLaunchSig('cursor', null, discoveredByAgent.cursor, 'cursor-agent') },
  },
  {
    name: 'cursor_offline_model_pick_saves_session_options',
    isAgent: true,
    paneRunning: false,
    paneBusy: false,
    discovered: discoveredByAgent.cursor,
    optionId: 'model',
    value: 'gpt-5.3-codex',
    node: { agentId: 'cursor', agentCommand: 'cursor-agent' },
  },
  // Gemini: running model pick saves for restart
  {
    name: 'gemini_live_model_pick_saves_for_restart',
    isAgent: true,
    paneRunning: true,
    paneBusy: false,
    optionId: 'model',
    value: 'gemini-3-pro-preview',
    node: { agentId: 'gemini', agentCommand: 'gemini', launchSig: makeInitialLaunchSig('gemini', null) },
  },
  // OpenCode: action types `/models` char-by-char; model pick saves for restart
  {
    name: 'opencode_action_model_opens_models_picker_typed',
    mode: 'action',
    isAgent: true,
    paneRunning: true,
    paneBusy: false,
    discovered: discoveredByAgent.opencode,
    optionId: 'model',
    node: { agentId: 'opencode', agentCommand: 'opencode', sessionOptions: { model: 'anthropic/claude-sonnet-4-5' } },
  },
  // Grok: live /model & /effort
  {
    name: 'grok_live_model_and_effort_command',
    isAgent: true,
    paneRunning: true,
    paneBusy: false,
    optionId: 'model',
    value: 'grok-4.6',
    node: { agentId: 'grok', agentCommand: 'grok', sessionOptions: { model: 'grok-4.5', effort: 'high' }, launchSig: makeInitialLaunchSig('grok', { model: 'grok-4.5', effort: 'high' }) },
  },
  // Antigravity: variant id strips effort suffix via composeModelValue when effort is present
  {
    name: 'antigravity_live_model_composes_base_id_when_effort_kept',
    isAgent: true,
    paneRunning: true,
    paneBusy: false,
    discovered: discoveredByAgent.antigravity,
    optionId: 'model',
    value: 'gemini-4-argon-high',
    node: { agentId: 'antigravity', agentCommand: 'agy', sessionOptions: { model: 'gemini-4-argon', effort: 'low' }, launchSig: makeInitialLaunchSig('antigravity', { model: 'gemini-4-argon', effort: 'low' }, discoveredByAgent.antigravity, 'agy') },
  },
  // Muse: live /effort command
  {
    name: 'muse_live_effort_command',
    isAgent: true,
    paneRunning: true,
    paneBusy: false,
    discovered: discoveredByAgent.muse,
    optionId: 'effort',
    value: 'max',
    node: { agentId: 'muse', agentCommand: 'muse', sessionOptions: { model: 'muse-spark-1.3', effort: 'high' }, launchSig: makeInitialLaunchSig('muse', { model: 'muse-spark-1.3', effort: 'high' }, discoveredByAgent.muse) },
  },
  // Pi: action /thinking picker
  {
    name: 'pi_action_thinking_picker',
    mode: 'action',
    isAgent: true,
    paneRunning: true,
    paneBusy: false,
    discovered: discoveredByAgent.pi,
    optionId: 'effort',
    node: { agentId: 'pi', agentCommand: 'pi', sessionOptions: { model: 'claude-sonnet-4-5' } },
  },
  // Amp, Kimi, Copilot: restart-only
  {
    name: 'amp_live_model_saves_for_restart',
    isAgent: true,
    paneRunning: true,
    paneBusy: false,
    optionId: 'model',
    value: 'deep',
    node: { agentId: 'amp', agentCommand: 'amp', launchSig: makeInitialLaunchSig('amp', null) },
  },
  {
    name: 'kimi_live_model_saves_for_restart',
    isAgent: true,
    paneRunning: true,
    paneBusy: false,
    optionId: 'model',
    value: 'kimi-code/kimi-for-coding',
    node: { agentId: 'kimi', agentCommand: 'kimi', launchSig: makeInitialLaunchSig('kimi', null) },
  },
  {
    name: 'copilot_live_effort_saves_for_restart',
    isAgent: true,
    paneRunning: true,
    paneBusy: false,
    optionId: 'effort',
    value: 'xhigh',
    node: { agentId: 'copilot', agentCommand: 'copilot', sessionOptions: { model: 'gpt-5.5' }, launchSig: makeInitialLaunchSig('copilot', { model: 'gpt-5.5' }) },
  },
  // Busy guard & unsupported agent
  {
    name: 'busy_running_pane_blocks_live_command',
    isAgent: true,
    paneRunning: true,
    paneBusy: true,
    optionId: 'model',
    value: 'opus',
    node: { agentId: 'claude', agentCommand: 'claude' },
  },
  {
    name: 'uncataloged_agent_returns_null',
    isAgent: true,
    paneRunning: true,
    paneBusy: false,
    optionId: 'model',
    value: 'anything',
    node: { agentId: 'aider', agentCommand: 'aider' },
  },
  // ChatView setOption scenarios
  {
    name: 'chat_set_option_haiku_no_effort',
    mode: 'chat',
    isAgent: true,
    chatViewAvailable: true,
    paneRunning: true,
    paneBusy: false,
    agentModel: { model: 'claude-haiku-4-5', source: 'session' },
    optionId: 'effort',
    value: 'high',
    node: { agentId: 'claude', agentCommand: 'claude' },
  },
  {
    name: 'chat_set_option_user_typing_blocked',
    mode: 'chat',
    isAgent: true,
    chatViewAvailable: true,
    paneRunning: true,
    paneBusy: false,
    userTyping: true,
    optionId: 'model',
    value: 'opus',
    node: { agentId: 'claude', agentCommand: 'claude' },
  },
  {
    name: 'chat_set_option_codex_restart_error',
    mode: 'chat',
    isAgent: true,
    chatViewAvailable: true,
    paneRunning: true,
    paneBusy: false,
    optionId: 'model',
    value: 'gpt-5.6-sol',
    node: { agentId: 'codex', agentCommand: 'codex' },
  },
  {
    name: 'chat_set_option_cursor_effort_returns_picker_true',
    mode: 'chat',
    isAgent: true,
    chatViewAvailable: true,
    paneRunning: true,
    paneBusy: false,
    discovered: discoveredByAgent.cursor,
    agentModel: { model: 'gpt-5.3-codex-high-fast', source: 'screen' },
    optionId: 'effort',
    value: 'low',
    node: { agentId: 'cursor', agentCommand: 'cursor-agent' },
  },
  {
    name: 'chat_set_option_unsupported_option_id',
    mode: 'chat',
    isAgent: true,
    chatViewAvailable: true,
    paneRunning: true,
    paneBusy: false,
    optionId: 'fastMode',
    value: 'true',
    node: { agentId: 'claude', agentCommand: 'claude' },
  },
];

const livePickCases = livePickScenarios.map((sc) => {
  const slimScenario = { ...sc };
  if (slimScenario.discovered === discoveredByAgent.cursor) {
    delete slimScenario.discovered;
    slimScenario.useCursorFixtureModels = true;
  }
  context.__sc = sc;
  context.__locale = 'en';
  const expectedEn = clone(vm.runInContext('simulatePanePick(__sc)', context));
  context.__locale = 'fr';
  const expectedFr = clone(vm.runInContext('simulatePanePick(__sc)', context));
  context.__locale = 'en';
  return { scenario: slimScenario, expectedEn, expectedFr };
});

// ---------------------------------------------------------------------------
// 9. Claude model switch observer & ANSI stripping
// ---------------------------------------------------------------------------
const claudeSwitchCheckInputs = [
  {
    name: 'ansi_cursor_positioned_success_with_resolved_version',
    buffer: '\u001b]0;Claude Code\u0007\u001b[2K\u001b[1GSet\u001b[5Gmodel\u001b[11Gto\u001b[14GOpus 5.5 (1M context)',
    modelLabel: 'Opus',
  },
  {
    name: 'exact_sonnet_4_5_match_and_wrong_version_mismatch',
    buffer: '\u001b[32m✓ Set model to claude-sonnet-4-5\u001b[0m',
    modelLabel: 'Sonnet 4.6',
  },
  {
    name: 'cached_conversation_confirmation_prompt',
    buffer: '\u001b[1mSwitch model?\u001b[0m\r\nThis conversation is cached for the current model. Switching will rebuild cache.',
    modelLabel: 'Opus',
  },
  {
    name: 'kept_model_rejection',
    buffer: '\u001b]8;;https://anthropic.com\u001b\\Kept model as Sonnet 5.5\u001b]8;;\u001b\\',
    modelLabel: 'Opus',
  },
  {
    name: 'kept_model_csi_rejection',
    buffer: '\u001b[33mKept model as Sonnet 5.5\u001b[0m',
    modelLabel: 'Opus',
  },
  {
    name: 'exact_sonnet_4_5_positive_match',
    buffer: '\u001b[32m✓ Set model to claude-sonnet-4-5\u001b[0m',
    modelLabel: 'Sonnet 4.5',
  },
  {
    name: 'unterminated_osc_and_single_escapes',
    buffer: '\u001b(B\u001bc\u001b]unterminated Set model to Haiku 4.5',
    modelLabel: 'Haiku',
  },
].map((c) => {
  context.__cb = c;
  return {
    ...c,
    stripped: vm.runInContext('stripAnsi(__cb.buffer)', context),
    confirmation: vm.runInContext('hasClaudeModelSwitchConfirmation(__cb.buffer)', context),
    success: vm.runInContext('hasClaudeModelSwitchSuccess(__cb.buffer, __cb.modelLabel)', context),
    rejection: vm.runInContext('hasClaudeModelSwitchRejection(__cb.buffer)', context),
  };
});

// ---------------------------------------------------------------------------
// 10. Screen readers (Cursor, Codex, Antigravity, Muse, generic screenModel)
// ---------------------------------------------------------------------------
const cursorScreenCases = [
  {
    name: 'idle_status_line_with_effort_and_fast',
    lines: ['> Write tests', '  Codex 5.3 272K High Fast  '],
  },
  {
    name: 'active_conversation_with_context_pct_and_files',
    lines: ['Assistant: Done.', '  Claude Opus 5 1M · MAX · 18.4% · 2 files edited'],
  },
  {
    name: 'token_count_when_pct_absent',
    lines: ['  Codex 5.3 272K Low · 45.2k tokens · 1 file edited'],
  },
  {
    name: 'picker_open_suppresses_status_line',
    lines: ['Models matching gpt', '  Codex 5.3 272K High Fast', 'Type to filter · Enter to select'],
  },
  {
    name: 'conversation_prose_mentioning_model_ignored',
    lines: ['GPT-5.3 Codex is fast when editing files in Rust'],
  },
].map((c) => {
  context.__cl = c.lines;
  context.__cm = discoveredByAgent.cursor;
  return {
    ...c,
    pickerShown: vm.runInContext('cursorPickerShown(__cl)', context),
    modelOnScreen: clone(vm.runInContext('cursorModelOnScreen(__cl, __cm)', context)),
    contextOnScreen: clone(vm.runInContext('cursorContextOnScreen(__cl, __cm)', context)),
  };
});

const cursorFilterCases = [
  null,
  { id: 'auto', label: 'Auto' },
  { id: 'claude-opus-5-thinking', label: 'Claude Opus 5 1M (NO ZDR)' },
  { id: 'gpt-5.3-codex', label: 'GPT-5.3 Codex 272K' },
  { id: 'unsafe', label: 'Bad/Slash$Name' },
].map((row) => {
  context.__row = row;
  return { row, expected: vm.runInContext('cursorPickerFilter(__row)', context) };
});

const codexModels = clone(vm.runInContext("catalogModelsFor('codex', null)", context));
const codexScreenCases = [
  {
    name: 'modern_footer_under_prompt_box',
    lines: ['• Completed step', '› Ask Codex anything', '', '  GPT-5.6 Sol xhigh fast · C:\\Tessel-rust  '],
    models: codexModels,
  },
  {
    name: 'legacy_footer_with_context_left',
    lines: ['› ', '', 'gpt-5.5-codex high · 92% context left · ~/proj'],
    models: codexModels,
  },
  {
    name: 'spoofed_model_inside_prompt_draft_rejected',
    lines: ['› GPT-5.6 Sol ultra', '  gpt-5.6-luna max · C:\\Tessel', '', '  tab to queue message'],
    models: codexModels,
  },
  {
    name: 'no_prompt_when_picker_covers_screen',
    lines: ['Select a model:', '  GPT-5.6 Sol xhigh · C:\\Tessel'],
    models: codexModels,
  },
  {
    name: 'local_numeric_hyphen_model_accepted',
    lines: ['› ', '', 'qwen3-32b medium · C:\\proj'],
    models: codexModels,
  },
].map((c) => {
  context.__cxl = c.lines;
  context.__cxm = c.models;
  return {
    name: c.name,
    lines: c.lines,
    models: c.models,
    expected: clone(vm.runInContext('codexModelOnScreen(__cxl, __cxm)', context)),
  };
});

const agyModels = [
  { id: 'gemini-4-argon', label: 'Gemini 4 Argon', efforts: ['low', 'medium', 'high', 'max'] },
  { id: 'gpt-oss-120b', label: 'GPT-OSS 120B (Medium)' },
  { id: 'claude-sonnet-4-6', label: 'Claude Sonnet 4.6' },
];
const antigravityScreenCases = [
  {
    name: 'dot_footer_right_aligned',
    lines: ['Working in C:\\Tessel    Gemini 4 Argon · high'],
    models: agyModels,
  },
  {
    name: 'parens_variant_footer',
    lines: ['~/repo    GPT-OSS 120B (Medium)'],
    models: agyModels,
  },
  {
    name: 'singleton_exact_label',
    lines: ['~/repo    Claude Sonnet 4.6'],
    models: agyModels,
  },
  {
    name: 'single_space_conversation_line_ignored',
    lines: ['We tested Gemini 4 Argon · high'],
    models: agyModels,
  },
].map((c) => {
  context.__agl = c.lines;
  context.__agm = c.models;
  return {
    ...c,
    expected: clone(vm.runInContext('antigravityModelOnScreen(__agl, __agm)', context)),
  };
});

const antigravityListedCases = [
  { idOrLabel: 'gemini-4-argon-high', effort: null },
  { idOrLabel: 'gemini-4-argon-high', effort: 'low' },
  { idOrLabel: 'GPT-OSS 120B (Medium)', effort: null },
  { idOrLabel: 'Gemini 4 Argon (Max)', effort: null },
  { idOrLabel: 'unknown-model', effort: 'high' },
].map((c) => {
  context.__agm = agyModels;
  context.__agi = c;
  return {
    ...c,
    models: agyModels,
    expected: clone(vm.runInContext('antigravityListed(__agm, __agi.idOrLabel, __agi.effort)', context)),
  };
});

const screenModelCases = [
  {
    name: 'bottom_beats_top_banner',
    bottom: ['status | kimi-k2.5-preview | idle'],
    top: ['Welcome to Kimi (kimi-k2.0)'],
  },
  {
    name: 'banner_fallback_when_bottom_has_no_model',
    bottom: ['Ready for input'],
    top: ['OpenCode v1.2 · anthropic/claude-sonnet-4.5.'],
  },
  {
    name: 'provider_and_family_variants',
    bottom: ['Model: openrouter/deepseek-r1-distill, fallback'],
    top: [],
  },
  {
    name: 'claude_product_name_with_1m',
    bottom: ['Using Opus 5.5 (1M) · ready'],
    top: [],
  },
  {
    name: 'prose_word_boundary_rejected',
    bottom: ['mygpt-5-custom and gpt-5xbar'],
    top: ['o3-mini'],
  },
].map((c) => {
  context.__sm = c;
  return {
    ...c,
    expected: vm.runInContext('modelFromScreen(__sm)', context),
  };
});

const museFooterCases = [
  { lines: ['  muse-wonder-1.0 · medium · C:\\TRADING · YOLO  '] },
  { lines: ['  muse-spark-1.3-contributor · ultra · ~/repo  '] },
  { lines: ['  muse-spark-1.3 · turbo · ~/repo  '] },
].map((c) => {
  context.__mf = c.lines;
  return { ...c, expected: clone(vm.runInContext('museFooterModel(__mf)', context)) };
});

const switchLineCases = [
  'Set model to opus (claude-opus-5-5)',
  'Set model to `Opus 5.5 (1M)` and saved as your default',
  'Set model to opus',
  'Kept model as sonnet',
].map((line) => {
  context.__sl = line;
  return { line, expected: vm.runInContext('modelFromSwitchLine(__sl)', context) };
});

const fixture = {
  sources,
  cursorFixtureModels: cursorGroupedModels,
  discoveredByAgent: {
    antigravity: discoveredByAgent.antigravity,
    muse: discoveredByAgent.muse,
    opencode: discoveredByAgent.opencode,
    pi: discoveredByAgent.pi,
  },
  validPaneCases,
  launchValuesCases,
  launchSignatureCases,
  effectiveAgentCases,
  savedModelCases,
  sessionChoiceErrorCases,
  effortLevelCases,
  modelLabelCases,
  headerTitleCases,
  chosenShownCases,
  withModelVersionCases,
  livePickCases,
  claudeSwitchCheckInputs,
  cursorFilterCases,
  cursorScreenCases,
  codexScreenCases,
  antigravityScreenCases,
  antigravityListedCases,
  screenModelCases,
  museFooterCases,
  switchLineCases,
};

const outPath = path.join(__dirname, '..', 'tests', 'fixtures', 'pane_session_model.json');
fs.writeFileSync(outPath, JSON.stringify(fixture, null, 2) + '\n');
console.log(
  `Wrote ${path.basename(outPath)} (${
    validPaneCases.length +
    launchValuesCases.length +
    launchSignatureCases.length +
    effectiveAgentCases.length +
    savedModelCases.length +
    sessionChoiceErrorCases.length +
    effortLevelCases.length +
    modelLabelCases.length +
    headerTitleCases.length +
    chosenShownCases.length +
    withModelVersionCases.length +
    livePickCases.length +
    claudeSwitchCheckInputs.length +
    cursorFilterCases.length +
    cursorScreenCases.length +
    codexScreenCases.length +
    antigravityScreenCases.length +
    antigravityListedCases.length +
    screenModelCases.length +
    museFooterCases.length +
    switchLineCases.length
  } reference vectors)`,
);
