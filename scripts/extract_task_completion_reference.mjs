// Synthetic source differential, no workspace data or credentials.
import { readFileSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import vm from 'node:vm';
const path = process.argv[2] || 'C:/Tessel/src/renderer/src/agentLimit.js';
const input = readFileSync(path, 'utf8').replace(/\r\n/g, '\n');
const start = input.indexOf('export function detectTaskDone(text) {');
const end = input.indexOf('\n}', start);
if (start < 0 || end < 0) throw new Error('Task completion source boundary changed.');
const source = input.slice(start, end + 2).replace(/^export /, '');
const texts = ['', 'TASK_COMPLETE', 'TASK_COMPLETE.', 'All tests pass.\nTASK_COMPLETE',
  'I will print TASK_COMPLETE when finished.', 'MY_TASK_COMPLETED_FLAG',
  'Next: tests, then TASK_COMPLETE', '**TASK_COMPLETE**', 'TASK_COMPLETE..',
  ...['●', '⏺', '•', '*', '>', '-'].flatMap(b => [`${b}TASK_COMPLETE`, ` ${b} TASK_COMPLETE. `]),
  ...['\r', '\t', '\u0085', '\u00a0', '\u1680', '\u2000', '\u200a', '\u2028', '\u2029', '\u202f', '\u205f', '\u3000', '\ufeff'].map(s => `${s}TASK_COMPLETE${s}`),
  'TASK_\nCOMPLETE', '● ● TASK_COMPLETE', 'TASK_COMPLETE\r\nContinuing',
  'Tasks say TASK and COMPLETE joined by an underscore'];
const cases = texts.map(text => {
  const context = { text };
  vm.runInNewContext(`${source}\nresult = detectTaskDone(text);`, context, { timeout: 1000 });
  return { text, expected: context.result };
});
writeFileSync(new URL('../tests/fixtures/task_completion.json', import.meta.url), JSON.stringify({ source: 'src/renderer/src/agentLimit.js#detectTaskDone', sha256: createHash('sha256').update(source).digest('hex'), cases }, null, 2) + '\n');
console.log(`Wrote ${cases.length} synthetic completion vectors.`);
