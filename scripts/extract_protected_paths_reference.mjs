// Read-only extraction of a pure policy: synthetic paths only, never user configuration.
import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';
import crypto from 'node:crypto';
const sourcePath = path.join(process.argv[2] || 'C:/Tessel', 'src/main/liveShare/shareScope.js');
const source = fs.readFileSync(sourcePath, 'utf8');
const start = source.indexOf('const PROTECTED_FOLDERS =');
const end = source.indexOf('// May a guest with these rights');
if (start < 0 || end <= start) throw new Error('Policy extraction boundaries changed');
const setup = "const lower = r => String(r || '').toLowerCase(); const segsOf = r => r ? lower(r).split('/') : [];\n";
const api = vm.runInNewContext(setup + source.slice(start, end).replaceAll('export function ', 'function ') + '\n;({isProtectedRel, folders:[...PROTECTED_FOLDERS], files:[...PROTECTED_FILES], hooks:[...HOOK_NAMES]})', {}, {timeout:1000});
const cases = [];
const add = (input, extra=[]) => cases.push({input, extra, expected:api.isProtectedRel(input, extra)});
for (const folder of api.folders) {
  for (const input of [folder, `${folder}/settings.json`, `nested/${folder.toUpperCase()}/config.json`, `${folder}-other/file.txt`]) add(input);
}
for (const file of api.files) {
  for (const input of [file, `nested/${file.toUpperCase()}`, `${file}.backup`, `${file}/ordinary.txt`]) add(input);
}
for (const hook of api.hooks) { add(hook); add(hook, ['']); add(`nested/${hook}`, ['']); }
for (const input of ['lefthook.yml','.lefthook.yaml','lefthook-local.yaml','.lefthook-local.yml','.lintstagedrc','.lintstagedrc.json','.lintstagedrcbackup','lint-staged.config.cjs','lint-staged.config.mts','lint-staged.config.json','commitlint.config.ts','.commitlintrc','commitlint-notes.txt','Jenkinsfile.production','Jenkinsfile.md','Jenkinsfile.txt','Jenkinsfile.rst','Jenkinsfile.adoc','Jenkinsfile.md.groovy','jenkinsfiles.txt','README.md','src/main.rs','']) add(input);
for (const input of ['custom/hooks','custom/hooks/pre-commit','custom/hooks-other/pre-commit','CUSTOM/HOOKS/run','nested/custom/hooks/run']) add(input,['custom/hooks']);
const output = {source:'src/main/liveShare/shareScope.js', sha256:crypto.createHash('sha256').update(source).digest('hex'), cases};
fs.mkdirSync('tests/fixtures',{recursive:true});
fs.writeFileSync('tests/fixtures/protected_paths_reference.json', JSON.stringify(output,null,2)+'\n');
console.log(`${cases.length} synthetic policy vectors written`);
