// Development-only reference extraction. Never executes a provider or modifies Tessel.
const fs = require('node:fs') as typeof import('node:fs');
const vm = require('node:vm') as typeof import('node:vm');
const crypto = require('node:crypto') as typeof import('node:crypto');
const path = require('node:path') as typeof import('node:path');
interface FixtureCase {
  name: string;
  agent: string;
  levels: string[];
  default: string;
  fast: boolean;
  expected: unknown;
}
const sourcePath = process.argv[2];
if (!sourcePath) throw new Error('Pass the reference agentSessionOptions.js path');
const source = fs.readFileSync(sourcePath, 'utf8');
const context = vm.createContext({});
vm.runInContext(source.replace(/^import .*$/gm, '').replace(/^export /gm, ''), context);
const names = {claude:'createClaudeCatalogOptions',codex:'createCodexCatalogOptions',pi:'createPiCatalogOptions',antigravity:'createAntigravityCatalogOptions',muse:'createMuseCatalogOptions'};
const levels = ['off','none','minimal','low','medium','high','xhigh','max','ultra'];
const cases: FixtureCase[] = [];
for (const [agent, fn] of Object.entries(names)) {
  for (let bits = 0; bits < (1 << levels.length); bits++) {
    const selected = levels.filter((_, i) => bits & (1 << i));
    selected.push('unknown');
    const def = levels[bits % levels.length];
    const fast = (bits & 1) === 1;
    const args = agent === 'claude' ? [{effortLevelIds:selected,supportsFastMode:fast}]
      : agent === 'codex' ? [{effortLevelIds:selected,defaultEffort:def}]
      : agent === 'muse' ? [selected,def] : [selected];
    context.args = args;
    const result: unknown = vm.runInContext(`${fn}(...args)`, context);
    const serialized = JSON.stringify(result);
    if (serialized === undefined) throw new Error('Reference options did not serialize');
    const expected: unknown = JSON.parse(serialized);
    cases.push({name:`${agent}/${bits}`,agent,levels:selected,default:def,fast,expected});
  }
}
const fixture = {source:sourcePath,sha256:crypto.createHash('sha256').update(source).digest('hex'),cases};
fs.writeFileSync(path.join(__dirname,'../tests/fixtures/agent_model_options.json'),JSON.stringify(fixture));
console.log(`${cases.length} reference comparisons generated`);
