// Run only pure source functions against synthetic identifiers and paths.
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const {createHash} = require('node:crypto');
const root = process.argv[2];
const files = ['agentResumeLine.js', 'chat/orca/shared/agent-session-resume.js', 'remoteAgentLaunch.js'];
const sources = files.map(f => fs.readFileSync(path.join(root, f), 'utf8'));
const getArgv = sources[1].slice(sources[1].indexOf('export function getAgentResumeArgv'));
const choices = sources[2].slice(sources[2].indexOf('export function claudeStartChoice'), sources[2].indexOf('// The command typed'));
const code = [getArgv, sources[0].replace(/^import .*$/gm,''), choices].join('\n').replace(/export /g,'');
const ctx = vm.createContext({});
vm.runInContext(code + '\nthis.api = {resumeArgs, safeSessionId, claudeStartChoice, codexResumes, OWN_ID_AGENTS}', ctx);
(async () => {
  const cases = [];
  for (const kind of [...ctx.api.OWN_ID_AGENTS, 'codex', 'unknown']) {
    for (const id of ['abc123', '_12345', '-12345', 'short', 'x'.repeat(80), 'x'.repeat(81), 'abc;def', 'é12345']) {
      for (const target of ['unavailable', 'missing', 'verified', '/tmp/session.jsonl', 'C:/sessions/test.jsonl', 'C:\\sessions\\test.jsonl', '/tmp/with space.jsonl', '  /tmp/session.jsonl  ']) {
        const api = target === 'unavailable' ? {} : {agentResumeTarget: async () => target === 'missing' ? null : {transcriptPath: target === 'verified' ? undefined : target}};
        cases.push({kind,id,target,expected: await ctx.api.resumeArgs(kind,id,api)});
      }
    }
  }
  const choices = [];
  for (const id of [null,'','abc123']) for (const resume of [false,true]) for (const exists of [null,false,true]) {
    choices.push({id,resume,exists,claude:ctx.api.claudeStartChoice({sessionId:id,resume,exists,newId:()=> 'fresh123'}),codex:ctx.api.codexResumes({sessionId:id,resume,exists})});
  }
  fs.writeFileSync(path.join(__dirname,'../tests/fixtures/agent_resume.json'),JSON.stringify({sources:files.map((file,i)=>({file,sha256:createHash('sha256').update(sources[i]).digest('hex')})),cases,choices})+'\n');
})().catch(e=>{console.error(e);process.exitCode=1;});
