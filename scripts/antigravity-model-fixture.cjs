const fs = require('node:fs');
const vm = require('node:vm');
const crypto = require('node:crypto');
const path = require('node:path');
const sourcePath = process.argv[2];
if (!sourcePath) throw new Error('Pass reference antigravityModels.js');
const source = fs.readFileSync(sourcePath, 'utf8');
const context = vm.createContext({});
vm.runInContext(source.replace(/^export /gm, ''), context);
const labels = [];
for (const name of ['Gemini 4 Argon', 'GPT-OSS 120B', 'Claude (Thinking)', '', 'é', '-bad', 'a'.repeat(80), 'a'.repeat(81)]) {
  for (const effort of ['Low','Medium','High','Extra High','xhigh','Max','Maximum','Thinking','',' low ','high)']) {
    for (const sep of [' ', '\t', '\u00a0']) {
      const input = `${name}${sep}(${effort})`;
      context.input = input;
      labels.push({input,expected:vm.runInContext('antigravityLabelParts(input)',context)});
    }
  }
}
const groups = [];
const rows = ['low','medium','high','xhigh','max'].map(e => ({id:`gemini-4-argon-${e}`,label:`Gemini 4 Argon (${e})`,isDefault:e==='high'}));
for (let bits = 0; bits < 32; bits++) {
  const input = rows.filter((_,i)=>bits & (1<<i));
  input.push({id:'gpt-oss-medium',label:'GPT-OSS (Medium)'},{id:'mismatch-low',label:'Mismatch (High)'},{id:'other',label:'Other'});
  context.input = input;
  groups.push({input,expected:vm.runInContext('groupAntigravityModels(input)',context)});
}
fs.writeFileSync(path.join(__dirname,'../tests/fixtures/agent_model_antigravity.json'),JSON.stringify({source:sourcePath,sha256:crypto.createHash('sha256').update(source).digest('hex'),labels,groups}));
console.log(`${labels.length} labels and ${groups.length} grouping cases`);
