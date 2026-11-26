// Only synthetic symbols; evaluates the actual Vue pure-model source.
import { readFileSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import vm from 'node:vm';
const path = process.argv[2] || 'C:/Tessel/src/renderer/src/explorer/outlineTree.js';
const raw = readFileSync(path, 'utf8').replace(/\r\n/g, '\n');
const source = raw.replace(/^import .*$/m, 'const t = (_, text) => text;').replace(/export /g, '');
const context = vm.createContext({});
vm.runInContext(source, context, { timeout: 1000 });
const range = (start, end, column = 1) => ({ startLineNumber: start, startColumn: column, endLineNumber: end, endColumn: 40 });
const symbol = (name, kind, start, end, children = []) => ({ name, kind, range: range(start, end), children });
const symbols = [symbol('Module', 1, 1, 100, [
  symbol('item10', 11, 4, 8), symbol('item2', 11, 10, 15),
  symbol('item2', 11, 16, 20), symbol('Éclair', 4, 22, 30, [symbol('readyState', 6, 24, 26)]),
]), symbol('zebra', 12, 102, 104), symbol('Alpha', 11, 106, 110)];
symbols[0].children[0].detail = 'synthetic function';
symbols[0].children[0].deprecated = true;
symbols[0].children[0].selectionRange = range(4, 4, 5);
const cases = [];
for (const by of ['position', 'name', 'kind']) for (const query of ['', 'rs', 'item2', 'missing', ' ']) {
  for (const highlight of [false, true]) for (const folded of [false, true]) {
    context.input = { symbols, by, query, highlight, folded };
    vm.runInContext(`{
      const tree = sortSymbols(keyed(input.symbols), input.by);
      const parents = parentKeys(tree);
      const collapsed = new Set(input.folded ? parents : []);
      result = { input: input.symbols, by: input.by, query: input.query, highlight: input.highlight,
        collapsed: [...collapsed], tree, parents, allCollapsed: allCollapsed(tree, collapsed),
        rows: outlineRows(tree, { collapsed, query: input.query, highlight: input.highlight }).map(({symbol, ...row}) => row),
        cursors: [[1,1],[24,10],[26,40],[26,41],[101,1],[110,40],[111,1]].map(([line,column]) => ({line,column,path:symbolPathAt(tree,line,column)})) };
    }`, context, { timeout: 1000 });
    cases.push(context.result);
  }
}
const matches = [];
for (const [name, query] of [['readyState','rs'],['readyState','state'],['abc','a c'],['abc','  '],['abc','x'],['😀alpha','😀a'],['😀alpha','😀h'],['Éclair','é'],['İtem','i'],['abc','']]) {
  context.name = name; context.query = query;
  vm.runInContext('match = matchName(name, query)', context);
  matches.push({ name, query, expected: context.match });
}
writeFileSync(new URL('../tests/fixtures/outline_tree.json', import.meta.url), JSON.stringify({source:path, sha256:createHash('sha256').update(raw).digest('hex'), locale:new Intl.Collator().resolvedOptions().locale, cases, matches}, null, 2)+'\n');
console.log(`Wrote ${cases.length} outline and ${matches.length} matching vectors.`);
