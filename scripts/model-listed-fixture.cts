// Development-only source comparisons. No provider invocation or source edits.
const fs = require('node:fs') as typeof import('node:fs');
const path = require('node:path') as typeof import('node:path');
const vm = require('node:vm') as typeof import('node:vm');
const crypto = require('node:crypto') as typeof import('node:crypto');
interface ListedRow {
  id: string;
  label: string;
  description?: string;
  isDefault?: boolean;
  effortLevels?: string[];
  defaultEffort?: string;
  supportsFastMode?: boolean;
}
interface FixtureCase {
  name: string;
  agent: string;
  rows: ListedRow[];
  expected: unknown;
}
const root = process.argv[2];
if (!root) throw new Error('Pass the reference src/shared directory');
const files = ['agentSessionOptions.js', 'antigravityModels.js', 'agentModelProbe.js'];
const context = vm.createContext({});
const sources = files.map(file => {
  const text = fs.readFileSync(path.join(root, file), 'utf8');
  vm.runInContext(text.replace(/^import .*$/gm, '').replace(/^export /gm, ''), context);
  return {file, sha256:crypto.createHash('sha256').update(text).digest('hex')};
});
const cases: FixtureCase[] = [];
const sets: ListedRow[][] = [[], [{id:'plain', label:'Plain'}], [{id:'empty',label:'Empty', effortLevels:[]}]];
for (const levels of [['unknown'],['low'],['high','low','high'],['minimal','medium','xhigh'],['none','off','max','ultra']]) {
  for (const defaultEffort of [undefined, 'unknown', ...levels]) {
    for (const supportsFastMode of [false,true]) {
      sets.push([{id:'test-model',label:'Test model',description:'Synthetic',isDefault:true,effortLevels:levels,defaultEffort,supportsFastMode}]);
    }
  }
}
sets.push([{id:'test-low',label:'Test (Low)'},{id:'test-high',label:'Test (High)',isDefault:true}]);
sets.push([{id:'test-low',label:'Test (Low)'},{id:'test-high',label:'Other (High)'}]);
for (const agent of ['claude','codex','grok','opencode','pi','antigravity','muse']) {
  for (const rows of sets) {
    context.agent = agent; context.rows = rows;
    const result: unknown = vm.runInContext('listedToCatalogModels(agent, rows)', context);
    const serialized = JSON.stringify(result);
    if (serialized === undefined) throw new Error('Reference model list did not serialize');
    const expected: unknown = JSON.parse(serialized);
    cases.push({name:`${agent}/${cases.length}`,agent,rows,expected});
  }
}
fs.writeFileSync(path.join(__dirname,'../tests/fixtures/agent_model_listed.json'),JSON.stringify({sources,cases}));
console.log(`${cases.length} source comparisons generated`);
