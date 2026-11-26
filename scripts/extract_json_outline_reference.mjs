import { pathToFileURL } from 'node:url';
import { resolve } from 'node:path';
import { writeFileSync } from 'node:fs';

const base = pathToFileURL(resolve(process.argv[2] || 'C:/Tessel/node_modules/monaco-editor/esm/external/vscode-json-languageservice/lib/esm') + '/');
const { parse } = await import(new URL('parser/jsonParser.js', base));
const { JSONDocumentSymbols } = await import(new URL('services/jsonDocumentSymbols.js', base));
const { TextDocument } = await import(new URL('jsonLanguageTypes.js', base));
const service = new JSONDocumentSymbols(null);
const range = ({ start, end }) => ({
  startLineNumber: start.line + 1, startColumn: start.character + 1,
  endLineNumber: end.line + 1, endColumn: end.character + 1
});
const symbol = s => ({
  name: s.name, detail: s.detail || '', kind: s.kind - 1, range: range(s.range),
  selectionRange: range(s.selectionRange), children: (s.children || []).map(symbol)
});
const cases = [
  '{}', '[]', 'null', '', '{"duplicate": 1, "duplicate": 2}',
  '{"nested": [false, null, {"x": 3}, [], {}]}',
  '{\r\n  "😀": "déjà",\r\n  "  ": true, "": [], "a\\nb": {}\r\n}',
  '{/* pending */ "a":1, // next\n "b": [1,2,],}',
  '{"complete":true, "pending":', '{"items":[1,2',
  '{"escaped":"a\\\"b","numeric":1e2}'
].map(text => {
  const doc = TextDocument.create('file:///fixture.json', 'json', 1, text);
  return { text, expected: service.findDocumentSymbols2(doc, parse(doc)).map(symbol) };
});
writeFileSync(new URL('../tests/fixtures/json_outline.json', import.meta.url), JSON.stringify(cases, null, 2)+'\n');
console.log(cases.map(c => c.expected.length).join(', '));
