// Synthetic reference vectors from the installed Vue application's Monaco package.
// Usage: node scripts/extract_editor_indent_reference.mjs C:/Tessel/node_modules/monaco-editor
import { readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const root = resolve(process.argv[2]);
const pkg = JSON.parse(readFileSync(resolve(root, 'package.json'), 'utf8'));
const { guessIndentation } = await import(pathToFileURL(resolve(root, 'esm/vs/editor/common/model/indentationGuesser.js')));
const texts = ['', 'plain\ntext', 'a\n  b\n    c\n  d', 'a\n    b\n        c', '\ta\n\t\tb', '    a\n\tb', 'const a = 1,\n      b = 2;', 'a\r\n   é😀\r\n      z\r\n', ' \n\t\n   \n', '\t  a\n    b\n\t c'];
let seed = 12345;
const next = () => (seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0);
for (let i = 0; i < 160; i++) {
  const lines = [];
  for (let line = 0; line < 12; line++) {
    const n = next();
    const indent = [' '.repeat(n % 10), '\t'.repeat(n % 4), '\t  ', '  \t'][n % 4];
    lines.push(indent + (n % 7 === 0 ? '' : `value${line}${n % 5 === 0 ? ',' : ''}`));
  }
  texts.push(lines.join(i % 2 ? '\n' : '\r\n'));
}
const vectors = texts.map(text => {
  const lines = text.split(/\r\n|\r|\n/);
  const source = { getLineCount: () => lines.length, getLineLength: n => lines[n-1].length, getLineContent: n => lines[n-1] };
  return { text, ...guessIndentation(source, 4, true) };
});
writeFileSync(new URL('../tests/fixtures/editor_indent_reference.json', import.meta.url), JSON.stringify({monacoVersion: pkg.version, vectors}, null, 2) + '\n');
console.log(`${vectors.length} vectors from Monaco ${pkg.version}`);
