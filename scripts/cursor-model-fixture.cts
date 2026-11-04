// Development-only source comparisons for Cursor model grouping.
// Read-only against reference src/shared; no provider invocation or source edits.
const fs = require('node:fs') as typeof import('node:fs');
const path = require('node:path') as typeof import('node:path');
const vm = require('node:vm') as typeof import('node:vm');
const crypto = require('node:crypto') as typeof import('node:crypto');

interface SourceHash {
  file: string;
  sha256: string;
}

interface GroupInputSuite {
  name: string;
  rows: unknown[];
}

interface ParsedCursorRow {
  id: string;
  label?: string;
  isDefault?: boolean;
}

interface LaunchProbeInput {
  values: Record<string, string | boolean>;
  userArgs: string;
}

const root = process.argv[2];
if (!root) {
  throw new Error('Pass the reference src/shared directory (e.g. C:\\Tessel\\src\\shared)');
}
const outPath = process.argv[3] ?? path.join(__dirname, '../tests/fixtures/agent_model_cursor.json');

const files = [
  'cursorModels.js',
  'antigravityModels.js',
  'agentSessionOptions.js',
  'agentModelProbe.js',
];

const context = vm.createContext({});
const sources: SourceHash[] = files.map((file) => {
  const fullPath = path.join(root, file);
  const text = fs.readFileSync(fullPath, 'utf8');
  const exported: string[] = [];
  const stripped = text
    .replace(/^import\s+[\s\S]*?from\s+['"][^'"]+['"]\s*;?\s*$/gm, '')
    .replace(
      /^export\s+(?:const|let|var|function)\s+([A-Za-z0-9_$]+)/gm,
      (m: string, name: string) => {
        exported.push(name);
        return m.replace(/^export\s+/, '');
      },
    );
  const assign = exported.map((n) => `globalThis.${n} = ${n};`).join('\n');
  vm.runInContext(`(() => {\n${stripped}\n${assign}\n})();`, context, { filename: file });
  return {
    file,
    sha256: crypto.createHash('sha256').update(text).digest('hex'),
  };
});

const listFixtureRel = '__tests__/fixtures/cursor-list-models.txt';
const rawCursorList = fs.readFileSync(path.join(root, listFixtureRel), 'utf8');
sources.push({
  file: listFixtureRel,
  sha256: crypto.createHash('sha256').update(rawCursorList).digest('hex'),
});

function clone(v: unknown): unknown {
  const serialized = JSON.stringify(v);
  if (serialized === undefined) throw new Error('Reference value did not serialize');
  return JSON.parse(serialized) as unknown;
}

function asParsedRowArray(v: unknown): ParsedCursorRow[] {
  if (!Array.isArray(v)) throw new Error('Expected parsed Cursor row array');
  return v as ParsedCursorRow[];
}

function asUnknownArray(v: unknown): unknown[] {
  if (!Array.isArray(v)) throw new Error('Expected grouped Cursor model array');
  return v;
}

const idInputs = [
  '',
  'auto',
  '-fast',
  '-thinking',
  '-thinking-fast',
  '-high',
  '-extra-high',
  'a-high',
  'a-extra-high',
  'gpt-5.3-codex',
  'gpt-5.3-codex-fast',
  'gpt-5.3-codex-low',
  'gpt-5.3-codex-low-fast',
  'gpt-5.3-codex-high-fast',
  'gpt-5.3-codex-xhigh-fast',
  'gpt-5.5-none',
  'gpt-5.5-minimal',
  'gpt-5.5-medium',
  'gpt-5.5-extra-high',
  'gpt-5.5-extra-high-fast',
  'gpt-5.6-sol-max-fast',
  'claude-opus-5-thinking-low',
  'claude-opus-5-thinking-high-fast',
  'claude-opus-5-thinking-max',
  'claude-4.6-opus-high-thinking',
  'claude-4.6-opus-max-thinking',
  'claude-4.6-opus-max-thinking-fast',
  'claude-4.5-sonnet-thinking',
  'claude-4.5-sonnet-thinking-fast',
  'composer-2.5',
  'composer-2.5-fast',
  'kimi-k2.7-code',
  'kimi-k3-max',
  'grok-4.7-xhigh-fast',
];

const parsedIds = idInputs.map((input) => {
  context.idInput = input;
  return {
    input,
    expected: clone(vm.runInContext('parseCursorModelId(idInput)', context)),
  };
});

const groupInputs: GroupInputSuite[] = [
  { name: 'empty', rows: [] },
  {
    name: 'invalid_and_empty_ids_ignored',
    rows: [
      null,
      {},
      { id: '' },
      { id: 42, label: 'Bad' },
      { id: 'auto', label: 'Auto', isDefault: true },
    ],
  },
  {
    name: 'singletons_preserve_thinking_and_fast_flags',
    rows: [
      { id: 'auto', label: 'Auto', isDefault: true },
      { id: 'gemini-3.1-pro', label: 'Gemini 3.1 Pro' },
      { id: 'solo-thinking', label: 'Solo Thinking' },
      { id: 'solo-fast', label: 'Solo Fast' },
      { id: 'no-label', label: '   \u200b\u2060\ufeff  ' },
    ],
  },
  {
    name: 'explicit_medium_splits_plain_rows_from_effort_group',
    rows: [
      { id: 'alpha-1', label: 'Alpha 1' },
      { id: 'alpha-1-fast', label: 'Alpha 1 Fast' },
      { id: 'alpha-1-low', label: 'Alpha 1 Low' },
      { id: 'alpha-1-medium', label: 'Alpha 1' },
      { id: 'alpha-1-high', label: 'Alpha 1 High' },
    ],
  },
  {
    name: 'explicit_medium_with_single_effort_becomes_two_plain_rows',
    rows: [
      { id: 'beta-2', label: 'Beta 2', isDefault: true },
      { id: 'beta-2-medium', label: 'Beta 2 Medium' },
    ],
  },
  {
    name: 'unsuffixed_id_becomes_medium_when_no_explicit_medium',
    rows: [
      { id: 'gpt-5.3-codex-low', label: 'Codex 5.3 Low' },
      { id: 'gpt-5.3-codex-low-fast', label: 'Codex 5.3 Low Fast' },
      { id: 'gpt-5.3-codex', label: 'Codex 5.3' },
      { id: 'gpt-5.3-codex-fast', label: 'Codex 5.3 Fast' },
      { id: 'gpt-5.3-codex-high', label: 'Codex 5.3 High' },
      { id: 'gpt-5.3-codex-high-fast', label: 'Codex 5.3 High Fast' },
      { id: 'gpt-5.3-codex-xhigh', label: 'Codex 5.3 Extra High' },
      { id: 'gpt-5.3-codex-xhigh-fast', label: 'Codex 5.3 Extra High Fast' },
    ],
  },
  {
    name: 'effortless_fast_pair_and_thinking_pair',
    rows: [
      { id: 'composer-2.5', label: 'Composer 2.5' },
      { id: 'composer-2.5-fast', label: 'Composer 2.5 Fast' },
      { id: 'claude-4.5-sonnet', label: 'Claude Sonnet 4.5' },
      { id: 'claude-4.5-sonnet-thinking', label: 'Claude Sonnet 4.5 Thinking' },
    ],
  },
  {
    name: 'single_effort_with_thinking_toggle_omits_effort_select',
    rows: [
      { id: 'claude-4.6-sonnet-medium', label: 'Claude Sonnet 4.6 1M' },
      { id: 'claude-4.6-sonnet-medium-thinking', label: 'Claude Sonnet 4.6 1M Thinking' },
    ],
  },
  {
    name: 'thinking_only_group_sets_default_thinking_true_and_has_thinking_false',
    rows: [
      { id: 'claude-exp-5-thinking-low', label: 'Claude Exp 5 1M Low Thinking' },
      { id: 'claude-exp-5-thinking-high', label: 'Claude Exp 5 1M Thinking' },
      { id: 'claude-exp-5-thinking-max', label: 'Claude Exp 5 1M Max Thinking' },
    ],
  },
  {
    name: 'own_effort_from_unlabeled_variant_vs_middle_fallback',
    rows: [
      { id: 'claude-opus-4-7-low', label: 'Claude Opus 4.7 1M Low' },
      { id: 'claude-opus-4-7-medium', label: 'Claude Opus 4.7 1M Medium' },
      { id: 'claude-opus-4-7-high', label: 'Claude Opus 4.7 1M High' },
      { id: 'claude-opus-4-7-xhigh', label: 'Claude Opus 4.7 1M' },
      { id: 'claude-opus-4-7-max', label: 'Claude Opus 4.7 1M Max' },
      { id: 'grok-4.7-low', label: 'Grok 4.7  Low' },
      { id: 'grok-4.7-medium', label: 'Grok 4.7  Medium' },
      { id: 'grok-4.7-high', label: 'Grok 4.7  High' },
      { id: 'grok-4.7-xhigh', label: 'Grok 4.7  Extra High' },
      { id: 'kimi-k3-low', label: 'Kimi K3 Low' },
      { id: 'kimi-k3-high', label: 'Kimi K3 High' },
      { id: 'kimi-k3-max', label: 'Kimi K3' },
    ],
  },
  {
    name: 'zero_width_unicode_spaces_and_longest_1m_no_zdr_label',
    rows: [
      { id: 'gpt-5.5-none-fast', label: 'GPT-5.5\u200b None\u00a0Fast\u200c\u200d' },
      { id: 'gpt-5.5-extra-high', label: '  GPT-5.5   1M  Extra High \u2060\ufeff ' },
      { id: 'claude-fable-5-1-low', label: 'Claude Fable 5.1 1M Low (NO ZDR)' },
      { id: 'claude-fable-5-1-high', label: 'Claude Fable 5.1 1M (NO ZDR)', isDefault: true },
    ],
  },
  {
    name: 'sorting_families_claude_tiers_versions_backtracking_and_case_ties',
    rows: [
      { id: 'zeta-1', label: 'Zeta 1' },
      { id: 'other-5.1m', label: 'Other 5.1M' },
      { id: 'other-12m', label: 'Other 12M' },
      { id: 'other-k3', label: 'K3' },
      { id: 'kimi-k2.7-code', label: 'Kimi K2.7 Code' },
      { id: 'kimi-k3-low', label: 'Kimi K3 Low' },
      { id: 'kimi-k3-max', label: 'Kimi K3' },
      { id: 'composer-2.5', label: 'Composer 2.5' },
      { id: 'grok-4.5', label: 'Grok 4.5' },
      { id: 'cursor-grok-4.7', label: 'Grok 4.7' },
      { id: 'gemini-3-flash', label: 'Gemini 3 Flash' },
      { id: 'gemini-3.8-flash', label: 'Gemini 3.8 Flash' },
      { id: 'gpt-5.4-nano', label: 'GPT-5.4 Nano' },
      { id: 'gpt-5.4-mini', label: 'GPT-5.4 Mini' },
      { id: 'gpt-5.4', label: 'GPT-5.4 1M' },
      { id: 'gpt-5.3-codex', label: 'codex 5.3' },
      { id: 'gpt-5.3-other', label: 'GPT-5.3' },
      { id: 'claude-haiku-4.5', label: 'Claude Haiku 4.5' },
      { id: 'claude-sonnet-5', label: 'Claude Sonnet 5 1M' },
      { id: 'claude-sonnet-5-5', label: 'Claude Sonnet 5.5' },
      { id: 'claude-opus-5', label: 'Claude Opus 5 1M' },
      { id: 'claude-opus-5-5', label: 'Claude Opus 5.5 1M' },
      { id: 'claude-fable-5', label: 'Claude Fable 5 1M (NO ZDR)' },
      { id: 'claude-fable-5-1', label: 'Claude Fable 5.1 1M (NO ZDR)' },
      { id: 'auto', label: 'Auto', isDefault: true },
      { id: 'tie-lower', label: 'alpha 2' },
      { id: 'tie-upper', label: 'Alpha 2' },
      { id: 'tie-beta', label: 'Beta 2' },
    ],
  },
];

groupInputs.push({
  name: 'unicode-collation',
  rows: [
    'z',
    'é',
    'e',
    'e\u0301',
    'É',
    'ä',
    'a',
    'A',
    'a-b',
    'a_b',
    'a b',
    'a+b',
    'a$b',
    'α',
    '中',
    '🙂',
  ].map((label, i) => ({ id: `other-${String.fromCharCode(97 + i)}`, label })),
});

const groups = groupInputs.map(({ name, rows }) => {
  context.groupRows = rows;
  return {
    name,
    rows: clone(rows),
    expected: clone(vm.runInContext('groupCursorModels(groupRows)', context)),
  };
});

context.rawCursorList = rawCursorList;
const realParsedRows = asParsedRowArray(
  clone(vm.runInContext('parseCursorModelList(rawCursorList)', context)),
);
context.realParsedRows = realParsedRows;
const realGrouped = asUnknownArray(
  clone(vm.runInContext('listedToCatalogModels("cursor", realParsedRows)', context)),
);
context.realGrouped = realGrouped;
const realCatalog = clone(vm.runInContext('catalogModelsFor("cursor", realGrouped)', context));
context.realCatalog = realCatalog;

const realDecomposed = realParsedRows.map((row) => {
  context.variantId = row.id;
  const launchFromDecomposed: unknown = vm.runInContext(
    'sessionOptionLaunchText("cursor", valuesOnListedRow(realCatalog, { model: variantId }), "", realCatalog)',
    context,
  );
  return {
    id: row.id,
    shown: clone(vm.runInContext('valuesOnListedRow(realCatalog, { model: variantId })', context)),
    launchFromDecomposed,
  };
});

const launchProbeInputs: LaunchProbeInput[] = [
  { values: { model: 'auto' }, userArgs: '' },
  { values: { model: 'claude-opus-5' }, userArgs: '' },
  { values: { model: 'claude-opus-5', effort: 'xhigh', thinking: false }, userArgs: '' },
  {
    values: { model: 'claude-opus-5', effort: 'xhigh', thinking: true, fastMode: true },
    userArgs: '',
  },
  {
    values: { model: 'claude-opus-5', effort: 'max', thinking: true, fastMode: true },
    userArgs: '',
  },
  { values: { model: 'gpt-5.3-codex', effort: 'medium' }, userArgs: '' },
  { values: { model: 'gpt-5.3-codex', effort: 'high', fastMode: true }, userArgs: '' },
  { values: { model: 'gpt-5.4', effort: 'low', fastMode: true }, userArgs: '' },
  { values: { model: 'claude-sonnet-5', effort: 'xhigh', thinking: true }, userArgs: '' },
  { values: { model: 'gpt-5.1', effort: 'max' }, userArgs: '' },
  { values: { model: 'grok-4.7-xhigh-fast' }, userArgs: '' },
  { values: { model: 'gpt-5.3-codex', effort: 'high' }, userArgs: '--model custom' },
];

const launchProbes = launchProbeInputs.map((probe) => {
  context.probe = probe;
  const expected: unknown = vm.runInContext(
    'sessionOptionLaunchText("cursor", probe.values, probe.userArgs, realCatalog)',
    context,
  );
  return {
    values: probe.values,
    userArgs: probe.userArgs,
    expected,
  };
});

fs.writeFileSync(
  outPath,
  JSON.stringify({
    sources,
    parsedIds,
    groups,
    realList: {
      rawOutput: rawCursorList,
      parsedRows: realParsedRows,
      grouped: realGrouped,
      mergedCatalog: realCatalog,
      decomposed: realDecomposed,
      launchProbes,
    },
  }) + '\n',
);

console.log(
  `Generated ${parsedIds.length} id cases, ${groups.length} group suites, ${realParsedRows.length} real rows -> ${realGrouped.length} grouped models`,
);
