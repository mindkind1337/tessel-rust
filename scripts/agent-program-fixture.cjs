// Extract the source pure shim parser, without importing Electron or starting a CLI.
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
const crypto = require('node:crypto');
const sourcePath = process.argv[2];
if (!sourcePath) throw new Error('Pass reference main/agentTools.js');
const source = fs.readFileSync(sourcePath,'utf8');
const start = source.indexOf('export function shimTarget(');
const end = source.indexOf('\n}',start)+2;
if(start<0 || end<start) throw new Error('shimTarget source not found');
const context=vm.createContext({join:path.win32.join,fs,process:{env:{SystemRoot:'C:\\Windows'}}});
vm.runInContext(source.slice(start,end).replace('export ',''),context);
const cases=[];
function add(text,existing,node='C:\\node\\node.exe') {
  const input={text,dir:'C:\\npm',node,existing}; context.input=input;
  const expected=vm.runInContext('shimTarget(input.text,input.dir,input.node,p=>input.existing.includes(p))',context);
  cases.push({...input,expected});
}
for(const prefix of ['%dp0%','%~dp0','%DP0%','%~dp0%']) {
  for(const ext of ['js','mjs','cjs','exe','txt']) {
    for(const gap of ['',' ','\t','\r\n']) {
      add(`@echo off\r\n"${prefix}\\node_modules\\tool.${ext}"${gap}%*`,[`C:\\npm\\node_modules\\tool.${ext}`]);
    }
  }
}
for(const prefix of ['%~dp0','%SCRIPT_DIR%']) {
  for(const shell of ['powershell','powershell.exe','POWERSHELL.EXE']) {
    add(`${shell} -NoProfile -File "${prefix}\\cursor-agent.ps1" %*`,['C:\\npm\\cursor-agent.ps1'],null);
  }
}
for(const name of ['..\\cursor-agent.ps1','x.txt','x%evil%.ps1']) {
  add(`powershell.exe -File "%SCRIPT_DIR%\\${name}" %*`,[path.win32.join('C:\\npm',name)],null);
}
add('"%dp0%\\x.js" %*',['C:\\npm\\x.js','C:\\npm\\node.exe']);
add('"%dp0%\\x.js" %*',['C:\\npm\\x.js'],null);
add('"%dp0%\\x.js" %*',[]);
add('powershell.exe -File "%SCRIPT_DIR%\\missing.ps1" %*\n"%dp0%\\real.exe" %*',['C:\\npm\\real.exe']);
add('"%dp0%\\..\\target.exe" %*',['C:\\target.exe']);
fs.writeFileSync(path.join(__dirname,'../tests/fixtures/agent_program.json'),JSON.stringify({source:sourcePath,sha256:crypto.createHash('sha256').update(source).digest('hex'),cases}));
console.log(`${cases.length} shim comparisons generated`);
