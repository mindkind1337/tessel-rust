// Development-only source comparisons for Antigravity model label parsing and grouping.
const fs = require('node:fs') as typeof import('node:fs');
const vm = require('node:vm') as typeof import('node:vm');
const crypto = require('node:crypto') as typeof import('node:crypto');
const path = require('node:path') as typeof import('node:path');

interface AntigravityRow {
  id: string;
  label: string;
  isDefault?: boolean;
}

interface LabelCase {
  input: string;
  expected: unknown;
}

interface GroupCase {
  input: AntigravityRow[];
  expected: unknown;
}

const sourcePath = process.argv[2];
if (!sourcePath) throw new Error('Pass reference antigravityModels.js');
const outPath = process.argv[3] ?? path.join(__dirname, '../tests/fixtures/agent_model_antigravity.json');

const source = fs.readFileSync(sourcePath, 'utf8');
const context = vm.createContext({});
vm.runInContext(source.replace(/^export /gm, ''), context);

const labels: LabelCase[] = [];
const names = ['Gemini 4 Argon', 'GPT-OSS 120B', 'Claude (Thinking)', '', 'é', '-bad', 'a'.repeat(80), 'a'.repeat(81)];
const efforts = ['Low', 'Medium', 'High', 'Extra High', 'xhigh', 'Max', 'Maximum', 'Thinking', '', ' low ', 'high)'];
for (const name of names) {
  for (const effort of efforts) {
    for (const sep of [' ', '\t', '\u00a0']) {
      const input = `${name}${sep}(${effort})`;
      context.input = input;
      const expected: unknown = vm.runInContext('antigravityLabelParts(input)', context);
      labels.push({ input, expected });
    }
  }
}

const groups: GroupCase[] = [];
const rows: AntigravityRow[] = ['low', 'medium', 'high', 'xhigh', 'max'].map((e) => ({
  id: `gemini-4-argon-${e}`,
  label: `Gemini 4 Argon (${e})`,
  isDefault: e === 'high',
}));
for (let bits = 0; bits < 32; bits++) {
  const input: AntigravityRow[] = rows.filter((_, i) => (bits & (1 << i)) !== 0);
  input.push(
    { id: 'gpt-oss-medium', label: 'GPT-OSS (Medium)' },
    { id: 'mismatch-low', label: 'Mismatch (High)' },
    { id: 'other', label: 'Other' },
  );
  context.input = input;
  const expected: unknown = vm.runInContext('groupAntigravityModels(input)', context);
  groups.push({ input, expected });
}

fs.writeFileSync(
  outPath,
  JSON.stringify({
    source: sourcePath,
    sha256: crypto.createHash('sha256').update(source).digest('hex'),
    labels,
    groups,
  }),
);
console.log(`${labels.length} labels and ${groups.length} grouping cases`);
