// Read-only source extraction; no provider commands or application startup.
const fs = require('node:fs') as typeof import('node:fs');
const path = require('node:path') as typeof import('node:path');
const vm = require('node:vm') as typeof import('node:vm');
const crypto = require('node:crypto') as typeof import('node:crypto');
const sourcePath = process.argv[2];
if (!sourcePath) throw new Error('Pass reference agentSessionOptions.js');
const source = fs.readFileSync(sourcePath, 'utf8');
const context = vm.createContext({});
vm.runInContext(source.replace(/^import .*$/gm, '').replace(/^export /gm, ''), context);
const serialized: unknown = vm.runInContext('JSON.stringify(CATALOGS)', context);
if (typeof serialized !== 'string') throw new Error('Reference catalogs did not serialize');
const catalogs: unknown = JSON.parse(serialized);
if (!catalogs || typeof catalogs !== 'object' || Array.isArray(catalogs)) {
  throw new Error('Reference catalogs must be an object');
}
const sha256 = crypto.createHash('sha256').update(source).digest('hex');
fs.writeFileSync(path.join(__dirname,'../src/agent_model_catalog.json'), JSON.stringify({source:'src/shared/agentSessionOptions.js',sha256,catalogs},null,2)+'\n');
console.log(`${Object.keys(catalogs).length} catalogues extracted`);
