// Source-only reference extraction using fake async files. No Muse installation is read.
const fs = require('node:fs') as typeof import('node:fs');
const path = require('node:path') as typeof import('node:path');
const vm = require('node:vm') as typeof import('node:vm');
const crypto = require('node:crypto') as typeof import('node:crypto');

interface MuseCase {
  name: string;
  texts: string[];
  expected: unknown;
}

const root = process.argv[2];
if (!root) throw new Error('Pass the reference src/main directory');
const outPath = process.argv[3] ?? path.join(__dirname, '../tests/fixtures/muse_catalog.json');

const catalog = fs.readFileSync(path.join(root, 'museCatalog.js'), 'utf8');
const sessions = fs.readFileSync(path.join(root, 'museSessions.js'), 'utf8');
const idSource = sessions.match(/^export const museModelId = .*$/m)?.[0];
if (!idSource) throw new Error('Cannot locate museModelId');

let texts: string[] = [];
const context = vm.createContext({
  os: { homedir: () => '/synthetic-home' },
  process: { env: {} },
  join: path.posix.join,
  museDataDir: () => '/synthetic-home/muse',
  fsp: {
    readdir: async () =>
      texts.map((_, i) => ({
        name: `${String(i).padStart(3, '0')}.json`,
        isFile: () => true,
      })),
    stat: async (file: string) => ({
      size: Buffer.byteLength(texts[Number(path.basename(file, '.json'))]),
    }),
    readFile: async (file: string) => texts[Number(path.basename(file, '.json'))],
  },
});

vm.runInContext(
  idSource.replace('export ', '') + '\n' + catalog.replace(/^import .*$/gm, '').replace(/^export /gm, ''),
  context,
);

const samples: string[][] = [[], ['{'], ['null'], ['[]'], ['{"rows":[]}']];
const rows: unknown[] = [
  null,
  [],
  true,
  'x',
  {},
  { model_id: 'a' },
  { model_id: ' hidden ', visibility: 'hidden' },
  { model_id: 'hidden', display_label: 'Visible' },
  { model_id: 'a', display_label: 'Later duplicate' },
  {
    model_id: 'full',
    display_label: ' Full ',
    description: ' Details ',
    is_default: true,
    default_reasoning_effort: 'high',
    reasoning_effort_variants: [{ tier: 'low' }, { tier: 'high' }, null, { tier: 4 }, { tier: '' }],
  },
  { model_id: 'model', display_label: 4, description: 4, is_default: 1, reasoning_effort_variants: 'high' },
];
samples.push([JSON.stringify({ rows })]);
for (const id of [
  '',
  ' ',
  'x'.repeat(120),
  'x'.repeat(121),
  ' bad id ',
  ' model\t',
  '\ufeffmodel\u2029',
  'm@p/[abc]:v+1',
  'm=v',
  'm\\v',
  'é',
  '-model',
]) {
  samples.push([JSON.stringify({ rows: [{ model_id: id, display_label: '' }] })]);
}
samples.push([
  JSON.stringify({ rows: [{ model_id: 'first', display_label: 'First' }] }),
  '{',
  JSON.stringify({ rows: [{ model_id: 'first', display_label: 'Other' }, { model_id: 'second' }] }),
]);
samples.push([JSON.stringify({ rows: Array.from({ length: 305 }, (_, i) => ({ model_id: `m${i}` })) })]);

(async () => {
  const cases: MuseCase[] = [];
  for (let i = 0; i < samples.length; i++) {
    texts = samples[i];
    const result: unknown = await (vm.runInContext('museCatalogModelList()', context) as Promise<unknown>);
    const serialized = JSON.stringify(result);
    if (serialized === undefined) throw new Error('Muse catalog result did not serialize');
    const expected: unknown = JSON.parse(serialized);
    cases.push({ name: `catalog/${i}`, texts, expected });
  }
  const sourceNames = ['museCatalog.js', 'museSessions.js'] as const;
  const sources = [catalog, sessions].map((text, i) => ({
    file: sourceNames[i],
    sha256: crypto.createHash('sha256').update(text).digest('hex'),
  }));
  fs.writeFileSync(outPath, JSON.stringify({ sources, cases }));
  console.log(`${cases.length} reference cases generated with fake files`);
})().catch((error: unknown) => {
  console.error(error);
  process.exitCode = 1;
});
