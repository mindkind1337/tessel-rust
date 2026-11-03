// Read-only source extraction; no provider commands or application startup.
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const crypto = require('node:crypto');
const sourcePath = process.argv[2];
if (!sourcePath) throw new Error('Pass reference agentSessionOptions.js');
const source = fs.readFileSync(sourcePath, 'utf8');
const context = vm.createContext({});
vm.runInContext(source.replace(/^import .*$/gm, '').replace(/^export /gm, ''), context);
const catalogs = JSON.parse(vm.runInContext('JSON.stringify(CATALOGS)', context));
const sha256 = crypto.createHash('sha256').update(source).digest('hex');
fs.writeFileSync(path.join(__dirname,'../src/agent_model_catalog.json'), JSON.stringify({source:'src/shared/agentSessionOptions.js',sha256,catalogs},null,2)+'\n');
console.log(`${Object.keys(catalogs).length} catalogues extracted`);
