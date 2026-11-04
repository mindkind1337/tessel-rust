// Hash the source signature using synthetic values only.
const fs = require('node:fs') as typeof import('node:fs');
const path = require('node:path') as typeof import('node:path');
const crypto = require('node:crypto') as typeof import('node:crypto');

interface LaunchInput {
  command: string;
  args: string;
  env: Record<string, string>;
}

const sourcePath = process.argv[2];
if (!sourcePath) throw new Error('Pass reference agentPrefs.js');
const outPath = process.argv[3] ?? path.join(__dirname, '../tests/fixtures/launch_fingerprint.json');

const source = fs.readFileSync(sourcePath, 'utf8');
const body = source.match(/export function launchSignature\(launch\) \{([\s\S]*?)\n\}/)?.[1];
if (!body) throw new Error('Source launchSignature not found');

const signature = new Function('launch', body) as (launch: LaunchInput) => string;
const inputs: LaunchInput[] = [
  { command: 'codex', args: '', env: {} },
  { command: 'codex', args: '-m pane-model -c model_reasoning_effort=high', env: {} },
  { command: 'custom cli', args: '--literal "hello"', env: { B: 'two', A: 'synthetic-secret' } },
  { command: 'custom cli', args: '--literal "hello"', env: { A: 'synthetic-secret', B: 'two' } },
  { command: 'synthetic-é🙂', args: 'line\nquote"slash\\', env: { VALUE: 'é🙂\tvalue', EMPTY: '' } },
];
const cases = inputs.map((input) => ({
  input,
  expected: crypto.createHash('sha256').update(signature(input)).digest('hex'),
}));

fs.writeFileSync(
  outPath,
  JSON.stringify({
    sourceSha256: crypto.createHash('sha256').update(source).digest('hex'),
    cases,
  }) + '\n',
);
