// Run only pure source functions against synthetic identifiers and paths.
const fs = require('node:fs') as typeof import('node:fs');
const path = require('node:path') as typeof import('node:path');
const vm = require('node:vm') as typeof import('node:vm');
const crypto = require('node:crypto') as typeof import('node:crypto');

interface ResumeTarget {
  transcriptPath?: string;
}

interface ResumeHostApi {
  agentResumeTarget?: () => Promise<ResumeTarget | null>;
}

interface StartChoiceInput {
  sessionId: string | null;
  resume: boolean;
  exists: boolean | null;
  newId?: () => string;
}

interface VmResumeApi {
  OWN_ID_AGENTS: Iterable<string>;
  resumeArgs: (kind: string, id: string, api: ResumeHostApi) => Promise<unknown>;
  safeSessionId: (id: unknown) => unknown;
  claudeStartChoice: (opts: StartChoiceInput) => unknown;
  codexResumes: (opts: StartChoiceInput) => unknown;
}

interface ResumeCase {
  kind: string;
  id: string;
  target: string;
  expected: unknown;
}

interface ResumeChoiceCase {
  id: string | null;
  resume: boolean;
  exists: boolean | null;
  claude: unknown;
  codex: unknown;
}

const root = process.argv[2];
if (!root) throw new Error('Pass the reference src/renderer/src directory');
const outPath = process.argv[3] ?? path.join(__dirname, '../tests/fixtures/agent_resume.json');

const files = [
  'agentResumeLine.js',
  'chat/orca/shared/agent-session-resume.js',
  'remoteAgentLaunch.js',
];
const sources = files.map((f) => fs.readFileSync(path.join(root, f), 'utf8'));
const getArgv = sources[1].slice(sources[1].indexOf('export function getAgentResumeArgv'));
const choiceDefs = sources[2].slice(
  sources[2].indexOf('export function claudeStartChoice'),
  sources[2].indexOf('// The command typed'),
);
const code = [getArgv, sources[0].replace(/^import .*$/gm, ''), choiceDefs]
  .join('\n')
  .replace(/export /g, '');

const ctx: import('node:vm').Context & { api?: VmResumeApi } = vm.createContext({});
vm.runInContext(
  code + '\nthis.api = {resumeArgs, safeSessionId, claudeStartChoice, codexResumes, OWN_ID_AGENTS}',
  ctx,
);
const vmApi = ctx.api;
if (!vmApi) throw new Error('Failed to initialize VM resume API');

(async () => {
  const cases: ResumeCase[] = [];
  const ids = ['abc123', '_12345', '-12345', 'short', 'x'.repeat(80), 'x'.repeat(81), 'abc;def', 'é12345'];
  const targets = [
    'unavailable',
    'missing',
    'verified',
    '/tmp/session.jsonl',
    'C:/sessions/test.jsonl',
    'C:\\sessions\\test.jsonl',
    '/tmp/with space.jsonl',
    '  /tmp/session.jsonl  ',
  ];
  for (const kind of [...vmApi.OWN_ID_AGENTS, 'codex', 'unknown']) {
    for (const id of ids) {
      for (const target of targets) {
        const api: ResumeHostApi =
          target === 'unavailable'
            ? {}
            : {
                agentResumeTarget: async () =>
                  target === 'missing'
                    ? null
                    : { transcriptPath: target === 'verified' ? undefined : target },
              };
        cases.push({ kind, id, target, expected: await vmApi.resumeArgs(kind, id, api) });
      }
    }
  }
  const choices: ResumeChoiceCase[] = [];
  for (const id of [null, '', 'abc123']) {
    for (const resume of [false, true]) {
      for (const exists of [null, false, true]) {
        choices.push({
          id,
          resume,
          exists,
          claude: vmApi.claudeStartChoice({ sessionId: id, resume, exists, newId: () => 'fresh123' }),
          codex: vmApi.codexResumes({ sessionId: id, resume, exists }),
        });
      }
    }
  }
  fs.writeFileSync(
    outPath,
    JSON.stringify({
      sources: files.map((file, i) => ({
        file,
        sha256: crypto.createHash('sha256').update(sources[i]).digest('hex'),
      })),
      cases,
      choices,
    }) + '\n',
  );
})().catch((e: unknown) => {
  console.error(e);
  process.exitCode = 1;
});
