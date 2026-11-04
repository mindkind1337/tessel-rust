// Development-only TypeScript reference extractor for `src/agent_model_readers.rs`.
// Strictly read-only against the reference Tessel source tree (`C:\Tessel`);
// runs natively on Node 24 (`node scripts/agent-model-readers-fixture.cts C:\Tessel`),
// uses an isolated in-memory virtual filesystem + VM context, and never spawns a CLI.

const fs = require('node:fs') as typeof import('node:fs');
const path = require('node:path') as typeof import('node:path');
const vm = require('node:vm') as typeof import('node:vm');
const crypto = require('node:crypto') as typeof import('node:crypto');

interface SourceProvenance {
  file: string;
  sha256: string;
}

interface VirtualFile {
  text: string;
  mtimeMs: number;
}

interface FoundModelShape {
  model: string;
  name?: string;
  effort: string | null;
  at?: number;
}

interface AgentModelShape {
  model: string | null;
  name?: string;
  effort: string | null;
  chosenEffort?: string | null;
  source: string | null;
  at?: number;
  keep?: boolean;
}

interface VirtualSnapshot {
  files?: Record<string, string | { text: string; mtimeMs?: number }>;
  dbRows?: Record<string, Array<Record<string, unknown>>>;
  alivePids?: number[];
  platform?: string;
}

const argPath: string | undefined = process.argv[2];
if (!argPath) {
  throw new Error('Pass the reference Tessel repo or src directory (e.g. C:\\Tessel)');
}

const srcDir: string = fs.existsSync(path.join(argPath, 'main', 'agentModel.js'))
  ? argPath
  : path.join(argPath, 'src');

if (!fs.existsSync(path.join(srcDir, 'main', 'agentModel.js'))) {
  throw new Error(`Could not locate main/agentModel.js under ${srcDir}`);
}

const sources: SourceProvenance[] = [];

function recordFile(relPath: string): string {
  const full = path.join(srcDir, relPath);
  const text = fs.readFileSync(full, 'utf8');
  sources.push({
    file: relPath.replace(/\\/g, '/'),
    sha256: crypto.createHash('sha256').update(text).digest('hex'),
  });
  return text;
}

// Virtual in-memory state controlled per test case without touching the real disk.
const memFiles = new Map<string, VirtualFile>();
const memDbRows = new Map<string, Array<Record<string, unknown>>>();
const memAlivePids = new Set<number>([100, 200, 300, 4242]);
let memPlatform = 'win32';
const HOME = 'C:\\Users\\test';

function normKey(p: string): string {
  return String(p || '')
    .replace(/\//g, '\\')
    .replace(/\\+/g, '\\')
    .replace(/\\$/, '')
    .toLowerCase();
}

function resetVirtual(snap: VirtualSnapshot = {}): void {
  memFiles.clear();
  memDbRows.clear();
  memAlivePids.clear();
  for (const pid of snap.alivePids ?? [100, 200, 300, 4242]) {
    memAlivePids.add(pid);
  }
  memPlatform = snap.platform ?? 'win32';
  if (snap.files) {
    for (const [relOrFull, entry] of Object.entries(snap.files)) {
      const full = /^[A-Za-z]:[\\/]/.test(relOrFull) ? relOrFull : path.win32.join(HOME, relOrFull);
      const text = typeof entry === 'string' ? entry : entry.text;
      const mtimeMs = typeof entry === 'string' ? 1_760_000_000_000 : (entry.mtimeMs ?? 1_760_000_000_000);
      memFiles.set(normKey(full), { text, mtimeMs });
    }
  }
  if (snap.dbRows) {
    for (const [key, rows] of Object.entries(snap.dbRows)) {
      const full = /^[A-Za-z]:[\\/]/.test(key) ? key : path.win32.join(HOME, key);
      memDbRows.set(normKey(full), rows);
    }
  }
}

interface VirtualDirent {
  name: string;
  isDirectory: () => boolean;
  isFile: () => boolean;
  isSymbolicLink: () => boolean;
}

function listVirtualDir(dirPath: string): VirtualDirent[] {
  const prefix = `${normKey(dirPath)}\\`;
  const children = new Map<string, boolean>(); // name (original case approx) -> isDir
  for (const key of memFiles.keys()) {
    if (!key.startsWith(prefix)) continue;
    const rest = key.slice(prefix.length);
    if (!rest) continue;
    const slash = rest.indexOf('\\');
    if (slash < 0) {
      if (!children.has(rest)) children.set(rest, false);
    } else {
      const sub = rest.slice(0, slash);
      children.set(sub, true);
    }
  }
  return [...children.entries()].map(([name, isDir]) => ({
    name,
    isDirectory: () => isDir,
    isFile: () => !isDir,
    isSymbolicLink: () => false,
  }));
}

const virtualFs = {
  existsSync(p: string): boolean {
    const k = normKey(p);
    if (memFiles.has(k)) return true;
    if (memDbRows.has(k)) return true;
    return listVirtualDir(p).length > 0;
  },
  statSync(p: string): { size: number; mtimeMs: number; isFile: () => boolean; isDirectory: () => boolean } {
    const k = normKey(p);
    const f = memFiles.get(k);
    if (f) {
      return {
        size: Buffer.byteLength(f.text, 'utf8'),
        mtimeMs: f.mtimeMs,
        isFile: () => true,
        isDirectory: () => false,
      };
    }
    if (memDbRows.has(k)) {
      return {
        size: 4096,
        mtimeMs: 1_760_000_000_000,
        isFile: () => true,
        isDirectory: () => false,
      };
    }
    if (listVirtualDir(p).length > 0) {
      return {
        size: 0,
        mtimeMs: 1_760_000_000_000,
        isFile: () => false,
        isDirectory: () => true,
      };
    }
    throw new Error(`ENOENT: ${p}`);
  },
  readdirSync(p: string, opts?: { withFileTypes?: boolean }): Array<VirtualDirent | string> {
    const list = listVirtualDir(p);
    if (!list.length && !memFiles.has(normKey(p))) {
      throw new Error(`ENOENT: ${p}`);
    }
    return opts && opts.withFileTypes ? list : list.map((e) => e.name);
  },
  readFileSync(p: string): string {
    const f = memFiles.get(normKey(p));
    if (!f) throw new Error(`ENOENT: ${p}`);
    return f.text;
  },
  realpathSync: Object.assign(
    (p: string): string => path.win32.resolve(p),
    { native: (p: string): string => path.win32.resolve(p) },
  ),
  watch(): { on: () => void; close: () => void } {
    return { on: () => {}, close: () => {} };
  },
};

const virtualOs = {
  homedir: (): string => HOME,
};

const virtualProcess = {
  get platform(): string {
    return memPlatform;
  },
  env: {} as Record<string, string | undefined>,
  kill(pid: number): boolean {
    if (memAlivePids.has(Number(pid))) return true;
    const err = new Error('ESRCH') as Error & { code?: string };
    err.code = 'ESRCH';
    throw err;
  },
};

const context = vm.createContext({
  console,
  Math,
  JSON,
  Object,
  Array,
  String,
  Number,
  Boolean,
  RegExp,
  Map,
  Set,
  Date,
  Promise,
  Error,
  Buffer,
  AbortSignal,
  parseInt,
  parseFloat,
  isFinite,
  isNaN,
  setTimeout,
  clearTimeout,
  setInterval,
  clearInterval,
  fs: virtualFs,
  os: virtualOs,
  process: virtualProcess,
  basename: path.win32.basename,
  dirname: path.win32.dirname,
  isAbsolute: path.win32.isAbsolute,
  join: path.win32.join,
  relative: path.win32.relative,
  sep: path.win32.sep,
  win32: path.win32,
});

// Record provenance for all upstream reference files.
for (const provenanceOnly of [
  'main/fileRead.js',
  'main/latestInFile.js',
  'renderer/src/components/TerminalPane.vue',
]) {
  recordFile(provenanceOnly);
}
const cursorListRaw: string = recordFile('shared/__tests__/fixtures/cursor-list-models.txt');

// Install pure in-memory equivalents of the small helper imports used by
// `museSessions.js`, `agentModelSources.js`, and `agentModel.js`.
vm.runInContext(
  `
  globalThis.__memReadHead = (file, bytes = 256 * 1024) => {
    try {
      const raw = Buffer.from(fs.readFileSync(file), 'utf8');
      return raw.subarray(0, Math.min(bytes, raw.length)).toString('utf8');
    } catch {
      return '';
    }
  };
  globalThis.__memReadTail = (file, bytes = 256 * 1024) => {
    try {
      const raw = Buffer.from(fs.readFileSync(file), 'utf8');
      const len = Math.min(bytes, raw.length);
      return raw.subarray(raw.length - len).toString('utf8');
    } catch {
      return '';
    }
  };
  globalThis.readHead = globalThis.__memReadHead;
  globalThis.readTail = globalThis.__memReadTail;
  globalThis.readTailBytes = globalThis.__memReadTail;
  globalThis.readText = (file) => {
    try {
      return fs.readFileSync(file);
    } catch {
      return '';
    }
  };
  globalThis.readJson = (file) => {
    try {
      return JSON.parse(fs.readFileSync(file));
    } catch {
      return null;
    }
  };
  globalThis.normDir = (p) => String(p || '').replace(/\\//g, '\\\\').replace(/\\\\+$/, '').toLowerCase();
  globalThis.isUuid = (id) => typeof id === 'string' && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(id);
  globalThis.qoderDir = (home) => join(home, '.qoder');
  globalThis.qoderTranscriptIn = (dir, id) => {
    const root = join(dir, 'projects');
    try {
      for (const d of fs.readdirSync(root, { withFileTypes: true })) {
        if (!d.isDirectory()) continue;
        const f = join(root, d.name, id + '.jsonl');
        if (fs.existsSync(f)) return f;
      }
    } catch {}
    return null;
  };
  globalThis.clineDataDir = (home) => join(home, '.cline', 'data');
  globalThis.qwenProjectsRoot = (home) => join(home, '.qwen', 'projects');
  globalThis.piSessionsDir = (home) => join(home, '.pi', 'agent', 'sessions');
  globalThis.ompSessionsDir = (home) => join(home, '.omp', 'agent', 'sessions');
  globalThis.grokHome = (home) => join(home, '.grok');
  globalThis.geminiFiles = (home) => {
    const out = [];
    const root = join(home, '.gemini', 'tmp');
    try {
      for (const p of fs.readdirSync(root, { withFileTypes: true })) {
        if (!p.isDirectory()) continue;
        const chats = join(root, p.name, 'chats');
        try {
          for (const f of fs.readdirSync(chats, { withFileTypes: true })) {
            if (f.isFile() && f.name.endsWith('.jsonl')) {
              out.push({ full: join(chats, f.name), legacy: false });
            }
          }
        } catch {}
      }
    } catch {}
    return out;
  };
  globalThis.parseGeminiHead = (text) => {
    const first = String(text || '').split('\\n')[0] || '';
    try {
      const o = JSON.parse(first);
      return o && typeof o.sessionId === 'string' ? { id: o.sessionId } : null;
    } catch {
      return null;
    }
  };
  globalThis.latestInFile = (parse, opts = {}) => {
    const { merge = null, done = (r) => !!r, finish = (r) => r } = opts;
    return (file) => {
      try {
        const raw = Buffer.from(fs.readFileSync(file), 'utf8');
        const CHUNK = 512 * 1024;
        let end = raw.length;
        let carry = '';
        let acc = null;
        while (end > 0) {
          const start = Math.max(0, end - CHUNK);
          const text = raw.subarray(start, end).toString('utf8') + carry;
          const nl = start > 0 ? text.indexOf('\\n') : -1;
          const whole = nl >= 0 ? text.slice(nl + 1) : start === 0 ? text : '';
          carry = nl >= 0 ? text.slice(0, nl) : start === 0 ? '' : text;
          if (whole) {
            const r = parse(whole);
            acc = merge ? merge(acc, r) : r;
            if (acc && done(acc)) return finish(acc);
          }
          end = start;
        }
        return finish(acc);
      } catch {
        return null;
      }
    };
  };
  `,
  context,
);

// Hook `readRows` in VM to our virtual SQLite row map (`memDbRows`).
(context as Record<string, unknown>).readRows = (file: string, sql: string, params: unknown[] = []): Array<Record<string, unknown>> => {
  const rows = memDbRows.get(normKey(file)) || [];
  if (sql.includes('FROM sessions WHERE ended_at IS NULL')) {
    return rows.slice(0, 50);
  }
  if (sql.includes('FROM session WHERE id = ?')) {
    const want = String(params[0] ?? '');
    return rows.filter((r) => r.__table === 'session' && String(r.id) === want);
  }
  if (sql.includes('FROM message WHERE session_id = ?')) {
    const want = String(params[0] ?? '');
    return rows
      .filter((r) => r.__table === 'message' && String(r.session_id) === want)
      .sort((a, b) => Number(b.time_created ?? 0) - Number(a.time_created ?? 0))
      .slice(0, 12);
  }
  if (sql.includes('FROM session WHERE parent_id IS NULL')) {
    const minUpdated = Number(params[0] ?? 0);
    return rows
      .filter((r) => r.__table === 'session' && !r.parent_id && Number(r.time_updated ?? 0) >= minUpdated)
      .sort((a, b) => Number(b.time_updated ?? 0) - Number(a.time_updated ?? 0))
      .slice(0, 20);
  }
  return rows;
};

function evalModule(relPath: string): void {
  const text = recordFile(relPath);
  const exported: string[] = [];
  const stripped = text
    .replace(/^import\s+[\s\S]*?from\s+['"][^'"]+['"]\s*;?\s*$/gm, '')
    .replace(/^export\s+class\s+([A-Za-z0-9_$]+)/gm, (_: string, name: string) => {
      exported.push(name);
      return `class ${name}`;
    })
    .replace(/^export\s+async\s+function\s+([A-Za-z0-9_$]+)/gm, (_: string, name: string) => {
      exported.push(name);
      return `async function ${name}`;
    })
    .replace(/^export\s+(?:const|let|var|function)\s+([A-Za-z0-9_$]+)/gm, (m: string, name: string) => {
      exported.push(name);
      return m.replace(/^export\s+/, '');
    });
  const assign = exported.map((n) => `globalThis.${n} = ${n};`).join('\n');
  vm.runInContext(`(() => {\n${stripped}\n${assign}\n})();`, context, { filename: relPath });
}

vm.runInContext(
  `
  globalThis.t = (_key, fallback) => fallback !== undefined ? fallback : _key;
  `,
  context,
);

for (const modPath of [
  'shared/cursorModels.js',
  'shared/antigravityModels.js',
  'shared/agentSessionOptions.js',
  'shared/agentModelProbe.js',
  'shared/modelLabel.js',
  'shared/codexScreen.js',
  'shared/screenModel.js',
  'shared/claudePricing.js',
  'renderer/src/paneLastModel.js',
  'renderer/src/chat/orca/native-chat-session-option-labels.js',
  'renderer/src/sessionOptionLabels.js',
  'main/museSessions.js',
  'main/agentModelSources.js',
  'main/agentModel.js',
]) {
  evalModule(modPath);
}

// Expose non-exported pure helpers needed for isolated unit vector checks too
// (`copilotRecords`, `copilotMerge`, `copilotFinish`, `uuidV7Time`, `parseLoose`)
// by evaluating thin wrappers against `agentModel.js` rules.
vm.runInContext(
  `
  globalThis.__simHeaderRefresh = function __simHeaderRefresh(input) {
    const OWN_MODEL_NAMES = ['claude', 'codex', 'cursor', 'antigravity'];
    const n = JSON.parse(JSON.stringify(input.node));
    let agentModelVal = input.currentAgentModel ? JSON.parse(JSON.stringify(input.currentAgentModel)) : null;
    let sessionModelVal = input.currentSessionModel ?? null;
    let screenContextVal = input.currentScreenContext ? JSON.parse(JSON.stringify(input.currentScreenContext)) : null;
    const models = input.models || [];
    const hasTerm = input.hasTerm !== false;
    const screenLines = input.screenLines || [];
    const topLines = input.topScreenLines || [];
    const scrollbackLines = input.scrollbackLines || [];

    const tail = (count) => screenLines.slice(Math.max(0, screenLines.length - count));
    const modelOnScreen = () => {
      if (!hasTerm) return null;
      return modelFromScreen({ bottom: tail(6), top: topLines.slice(0, 30) });
    };
    const modelSwitchedOnScreen = () => {
      if (!hasTerm || n.agentId !== 'claude') return null;
      const start = Math.max(0, scrollbackLines.length - 400);
      for (let i = scrollbackLines.length - 1; i >= start; i--) {
        const m = modelFromSwitchLine(scrollbackLines[i]);
        if (m) return m;
      }
      return null;
    };
    const chosenShown = (node, res) => {
      const opt = node.sessionOptions;
      if (!(node.modelChoice && node.modelChoice.model) && opt && opt.model) {
        if (res && res.source === 'session' && res.model) return res;
        const effort = opt.effort || (res && res.chosenEffort) || null;
        return { model: modelChoiceLabel(models, opt.model), effort, source: 'chosen' };
      }
      const choice = node.modelChoice;
      if (!choice || !choice.model) return res && res.model ? res : null;
      if (res && res.source === 'session') {
        if (res.at && choice.at && !node.remoteHostId && res.at > choice.at) {
          delete node.modelChoice;
          return res;
        }
        const said = res.model + '|' + (res.effort || '');
        if (choice.seen === undefined || choice.seen === null) choice.seen = said;
        else if (choice.seen !== said) {
          delete node.modelChoice;
          return res;
        }
      }
      const effort = choice.effort || (res && (res.chosenEffort || res.effort)) || null;
      return { model: modelChoiceLabel(models, choice.model), effort, source: 'chosen' };
    };
    const newestClaudeOf = (family) => {
      let best = null;
      let bestKey = -1;
      for (const id of Object.keys(CLAUDE_PRICING)) {
        const m = new RegExp('^claude-' + family + '-(\\\\d+)(?:-(\\\\d+))?$').exec(id);
        if (!m) continue;
        const key = Number(m[1]) * 100 + Number(m[2] || 0);
        if (key > bestKey) {
          bestKey = key;
          best = id;
        }
      }
      return best;
    };
    const withModelVersion = (shown, ...reported) => {
      const bare = /^(opus|sonnet|haiku|fable)(\\[1m\\])?$/i.exec(String(shown.model || '').trim());
      if (!bare) return shown;
      const family = bare[1].toLowerCase();
      const full =
        reported.find((m) => typeof m === 'string' && new RegExp('^(?:claude-)?' + family + '-\\\\d', 'i').test(m.trim())) ||
        newestClaudeOf(family);
      return full ? { ...shown, model: full.trim() + (bare[2] && !/\\[1m\\]$/i.test(full) ? '[1m]' : '') } : shown;
    };

    let frozeOnPicker = false;
    if (n.agentId === 'cursor' && hasTerm) {
      const lines = tail(8);
      const ctx = cursorContextOnScreen(lines, models);
      if (ctx) screenContextVal = ctx.none ? null : ctx;
      if (cursorPickerShown(lines) && agentModelVal) {
        frozeOnPicker = true;
        return {
          agentModel: agentModelVal,
          sessionModel: sessionModelVal,
          screenContext: screenContextVal,
          modelChoice: n.modelChoice ?? null,
          toSave: paneModelToSave(agentModelVal),
          frozeOnPicker,
        };
      }
      const seen = cursorModelOnScreen(lines, models);
      if (seen) {
        delete n.modelChoice;
        agentModelVal = { model: seen.model, name: seen.name, effort: seen.effort, source: 'screen' };
        const bg = input.agentModelResult;
        if (!(bg && bg.keep)) {
          sessionModelVal = bg && bg.model && bg.source !== 'screen' ? String(bg.model) : null;
        }
        return {
          agentModel: agentModelVal,
          sessionModel: sessionModelVal,
          screenContext: screenContextVal,
          modelChoice: n.modelChoice ?? null,
          toSave: paneModelToSave(agentModelVal),
          frozeOnPicker,
        };
      }
    }
    if (n.agentId === 'antigravity' && hasTerm) {
      const seen = antigravityModelOnScreen(tail(6), models);
      if (seen) {
        delete n.modelChoice;
        agentModelVal = { model: seen.model, ...(seen.name ? { name: seen.name } : {}), effort: seen.effort, source: 'screen' };
        const bg = input.agentModelResult;
        if (!(bg && bg.keep)) {
          sessionModelVal = bg && bg.model && bg.source !== 'screen' ? String(bg.model) : null;
        }
        return {
          agentModel: agentModelVal,
          sessionModel: sessionModelVal,
          screenContext: screenContextVal,
          modelChoice: n.modelChoice ?? null,
          toSave: paneModelToSave(agentModelVal),
          frozeOnPicker,
        };
      }
    }
    if (n.agentId === 'codex' && hasTerm) {
      const seen = codexModelOnScreen(tail(12), models);
      if (seen) {
        delete n.modelChoice;
        agentModelVal = { model: seen.model, ...(seen.name ? { name: seen.name } : {}), effort: seen.effort, source: 'screen' };
        const bg = input.agentModelResult;
        if (!(bg && bg.keep)) {
          sessionModelVal = bg && bg.model && bg.source !== 'screen' ? String(bg.model) : null;
        }
        return {
          agentModel: agentModelVal,
          sessionModel: sessionModelVal,
          screenContext: screenContextVal,
          modelChoice: n.modelChoice ?? null,
          toSave: paneModelToSave(agentModelVal),
          frozeOnPicker,
        };
      }
    }

    let res = input.agentModelResult ? JSON.parse(JSON.stringify(input.agentModelResult)) : null;
    if (res && res.keep) {
      if (agentModelVal && agentModelVal.model) {
        return {
          agentModel: agentModelVal,
          sessionModel: sessionModelVal,
          screenContext: screenContextVal,
          modelChoice: n.modelChoice ?? null,
          toSave: paneModelToSave(agentModelVal),
          frozeOnPicker,
        };
      }
      res = null;
    }
    sessionModelVal = res && res.model && res.source !== 'screen' ? String(res.model) : null;
    if (n.agentId === 'antigravity' && res && res.model) {
      const listed = antigravityListed(models, res.model, res.effort);
      if (listed) res = { ...res, ...listed };
    }
    if (res && res.model && !res.name && !OWN_MODEL_NAMES.includes(n.agentId)) {
      const row = models.find((m) => m && m.id === res.model && typeof m.label === 'string' && m.label.trim() && m.label.trim() !== res.model);
      if (row) res = { ...res, name: row.label.trim() };
    }
    if (n.agentId === 'muse' && hasTerm) {
      const foot = museFooterModel(tail(6));
      if (foot && (!res || !res.model) && !n.modelChoice) res = { model: foot.model, effort: foot.effort, source: 'screen' };
      else if (foot && res && res.model === foot.model && !res.effort) res = { ...res, effort: foot.effort };
    }
    if ((!res || !res.model) && !n.modelChoice) {
      const seen = modelOnScreen();
      if (seen) res = { model: seen, effort: (res && res.chosenEffort) || null, source: 'screen' };
    }
    if (res && res.source === 'settings' && !n.modelChoice) {
      const switched = modelSwitchedOnScreen();
      if (switched) res = { model: switched, effort: res.effort || null, source: 'screen' };
    }
    const shown = chosenShown(n, res);
    agentModelVal = shown && n.agentId === 'claude'
      ? withModelVersion(shown, res && res.source === 'session' ? res.model : null, modelSwitchedOnScreen())
      : shown;
    return {
      agentModel: agentModelVal,
      sessionModel: sessionModelVal,
      screenContext: screenContextVal,
      modelChoice: n.modelChoice ?? null,
      toSave: paneModelToSave(agentModelVal),
      frozeOnPicker,
    };
  };
  `,
  context,
);

const vmApi = context as unknown as {
  parseCursorModelList: (stdout: string) => unknown[];
  groupCursorModels: (rows: unknown[]) => unknown[];
  catalogModelsFor: (agentId: string, discovered?: unknown[] | null) => Array<Record<string, unknown>>;
  claudeModelFromText: (text: string) => string | null;
  claudeTurnFromText: (text: string) => FoundModelShape | null;
  claudeSettingsEffort: (model: string | null, cwd: string | null, home: string) => string | null;
  sameClaudeModel: (a: string, b: string) => boolean;
  codexModelFromText: (text: string) => FoundModelShape | null;
  codexModelFromToml: (text: string) => FoundModelShape | null;
  copilotModelFromText: (text: string) => FoundModelShape | null;
  antigravityModelFromText: (text: string) => FoundModelShape | null;
  antigravityModelTimeline: (lines: string[]) => Array<{ t: number; model: string; effort: string | null }>;
  museModelFromText: (text: string) => FoundModelShape | null;
  museModelTimeline: (text: string) => Array<{ t: number; model: string; effort: string | null }>;
  museSessionStart: (id: string) => number;
  museSession: (id: string, home: string) => FoundModelShape | null;
  museEffortFromCommand: (cmd: string) => string | null;
  museSettingsModel: (home: string) => { model: string | null; effort: string | null; effortFor: (id: string) => string | null } | null;
  clineSessionModel: (rows: unknown[], cwd: string | null, alive: (pid: number) => boolean) => { model: string; at: number } | null;
  modelFromCommand: (cmd: string) => string | null;
  effortFromCommand: (cmd: string) => string | null;
  effortFlag: (cmd: string) => string | null;
  ampModeFromCommand: (cmd: string) => string | null;
  cursorConfigModel: (text: string) => FoundModelShape | null;
  geminiModelFromText: (text: string) => FoundModelShape | null;
  qwenModelFromText: (text: string) => FoundModelShape | null;
  kimiWireModel: (text: string) => { model: string | null; thinking: string | undefined; at: number | null } | null;
  kimiModelFromWire: (tail: string, head?: string) => FoundModelShape | null;
  tomlKeys: (text: string) => Map<string, Map<string, unknown>>;
  kimiConfigModel: (text: string) => FoundModelShape | null;
  piModelFromWire: (tail: string, head?: string) => FoundModelShape | null;
  piModelFromText: (text: string) => FoundModelShape | null;
  piModelTimeline: (lines: string[]) => Array<{ t: number; model: string; effort: string | null }>;
  piSettingsModel: (o: unknown) => FoundModelShape | null;
  openclawConfigModel: (o: unknown) => FoundModelShape | null;
  droidSettingsModel: (o: unknown) => FoundModelShape | null;
  opencodeMessageModel: (data: unknown) => FoundModelShape | null;
  opencodeSessionColumn: (val: unknown) => FoundModelShape | null;
  opencodeSessionModel: (col: string | null, rows: unknown[], updated?: number | null) => FoundModelShape | null;
  opencodePicked: (home: string, app?: string) => { model: string; effort: string | null; mtime: number } | null;
  continueModels: (yaml: string) => Array<{ name: string; model: string; chat: boolean }>;
  continueModel: (selected: string | undefined, yaml: string) => FoundModelShape | null;
  crushConfigModel: (o: unknown) => FoundModelShape | null;
  gooseConfigModel: (yaml: string) => FoundModelShape | null;
  aiderConfigModel: (yaml: string) => FoundModelShape | null;
  vibeConfigModel: (text: string) => FoundModelShape | null;
  grokSummaryModel: (o: unknown) => FoundModelShape | null;
  grokCatalogEntry: (cache: unknown, id: string) => { name: string | null; effort: string | null } | null;
  grokConfigModel: (text: string) => FoundModelShape | null;
  agentModel: (query: Record<string, unknown>, home: string) => AgentModelShape | null;
  _clearModelSourceCachesForTest: () => void;
  __simHeaderRefresh: (input: Record<string, unknown>) => Record<string, unknown>;
};

function normFound(v: FoundModelShape | { model: string; at?: number } | null | undefined): FoundModelShape | null {
  if (!v || !v.model) return null;
  const f = v as FoundModelShape;
  return {
    model: f.model,
    ...(f.name ? { name: f.name } : {}),
    effort: f.effort ?? null,
    ...(typeof f.at === 'number' && Number.isFinite(f.at) && f.at !== 0 ? { at: f.at } : {}),
  };
}

function normAgentModel(v: AgentModelShape | null | undefined): Record<string, unknown> | null {
  if (!v) return null;
  if (!v.model) {
    return {
      found: null,
      ...(v.chosenEffort ? { chosenEffort: v.chosenEffort } : {}),
    };
  }
  return {
    found: {
      model: v.model,
      ...(v.name ? { name: v.name } : {}),
      effort: v.effort ?? null,
      ...(v.chosenEffort ? { chosenEffort: v.chosenEffort } : {}),
      source: v.source,
      ...(typeof v.at === 'number' && Number.isFinite(v.at) && v.at !== 0 ? { at: v.at } : {}),
    },
    ...(v.chosenEffort ? { chosenEffort: v.chosenEffort } : {}),
  };
}

const jline = (o: unknown): string => JSON.stringify(o);
const T0 = Date.parse('2026-10-16T10:00:00.000Z');
const iso = (sec: number): string => new Date(T0 + sec * 1000).toISOString();

// -----------------------------------------------------------------------------
// 1. Claude transcript cases
// -----------------------------------------------------------------------------
const T1 = '2026-10-01T17:56:48.645Z';
const T2 = '2026-10-01T17:58:00.000Z';
const T3 = '2026-10-01T17:59:00.000Z';
const claudeAns = (model: string, effort: string | null, ts: string = T1, extra: Record<string, unknown> = {}): string =>
  jline({ type: 'assistant', timestamp: ts, effort, perTurnEffort: effort, message: { model }, ...extra });
const claudeUser = (content: unknown, ts: string, extra: Record<string, unknown> = {}): string =>
  jline({ type: 'user', timestamp: ts, message: { role: 'user', content }, ...extra });
const claudeCaveat = (ts: string): string =>
  claudeUser('<local-command-caveat>The command below was run directly in Claude Code</local-command-caveat>', ts, { isMeta: true });
const claudeCmd = (name: string, args: string, ts: string): string =>
  claudeUser(`<command-name>/${name}</command-name>\n            <command-message>${name}</command-message>\n            <command-args>${args}</command-args>`, ts);
const claudeStdout = (text: string, ts: string): string =>
  claudeUser(`<local-command-stdout>${text}</local-command-stdout>`, ts);

const claudeBase = [claudeAns('claude-opus-5-5', 'xhigh', T1)];
const claudeHaiku = [
  ...claudeBase,
  claudeCaveat(T2),
  claudeCmd('model', 'haiku', T2),
  claudeStdout('Set model to `Haiku 4.5` and saved as your default for new sessions', T2),
];
const claudeSonnetMedium = [
  ...claudeHaiku,
  claudeCaveat(T3),
  claudeCmd('model', 'sonnet', T3),
  claudeStdout('Set model to `Sonnet 5.5` and saved as your default for new sessions', T3),
  claudeCaveat(T3),
  claudeCmd('effort', 'medium', T3),
  claudeStdout('Set effort level to medium (saved as your default for new sessions): Balanced approach', T3),
];

const claudeInputs: Array<{ name: string; text: string }> = [
  { name: 'empty', text: '' },
  {
    name: 'latest-real-answer-skips-synthetic-sidechain-and-cut-tail',
    text: [
      jline({ type: 'assistant', message: { model: 'claude-sonnet-5' } }),
      jline({ type: 'user', message: { content: 'hi' } }),
      claudeAns('claude-opus-5-5', 'high', T1),
      claudeAns('claude-haiku-4-5', 'low', T2, { isSidechain: true }),
      jline({ type: 'assistant', message: { model: '<synthetic>' } }),
      '{"type":"assistant","message":{"model":"cut',
    ].join('\n'),
  },
  { name: 'bogus-effort-ignored', text: claudeAns('claude-fable-5-1', 'bogus', T1) },
  { name: 'slash-model-haiku-clears-effort', text: claudeHaiku.join('\n') },
  { name: 'slash-model-sonnet-and-effort-medium', text: claudeSonnetMedium.join('\n') },
  {
    name: 'bare-effort-picker-stdout-only-names-level',
    text: [
      ...claudeBase,
      claudeCaveat(T2),
      claudeCmd('effort', '', T2),
      claudeStdout('Set effort level to medium (saved as your default for new sessions): Balanced approach', T2),
    ].join('\n'),
  },
  {
    name: 'stdout-set-effort-without-command-line-in-tail',
    text: [claudeBase[0], claudeStdout('Set effort level to low', T2)].join('\n'),
  },
  {
    name: 'bare-model-picker-uses-printed-family-and-keeps-effort',
    text: [...claudeBase, claudeCmd('model', '', T2), claudeStdout('Set model to `Sonnet 5.5`', T2)].join('\n'),
  },
  {
    name: 'refused-slash-model-changes-nothing',
    text: [...claudeBase, claudeCmd('model', 'nonsense', T2), claudeStdout('Invalid model', T2)].join('\n'),
  },
  {
    name: 'assistant-answer-after-slash-command-wins',
    text: [...claudeHaiku, claudeAns('claude-haiku-4-5', null, T3)].join('\n'),
  },
  {
    name: 'slash-model-before-any-answer',
    text: [claudeCmd('model', 'opus', T2), claudeStdout('Set model to `Opus 5.5`', T2)].join('\n'),
  },
  {
    name: 'array-content-blocks-in-user-command',
    text: [
      claudeUser([{ type: 'text', text: '<command-name>/model</command-name><command-args>fable</command-args>' }], T2),
      claudeUser([{ type: 'text', text: '<local-command-stdout>Set model to `Fable 5.1`</local-command-stdout>' }], T2),
    ].join('\n'),
  },
];

const claudeTurnCases = claudeInputs.map((c) => ({
  name: c.name,
  text: c.text,
  expectedTurn: normFound(vmApi.claudeTurnFromText(c.text)),
  expectedModel: vmApi.claudeModelFromText(c.text),
}));

// -----------------------------------------------------------------------------
// 2. Codex rollout & UUIDv7 cases
// -----------------------------------------------------------------------------
const codexInputs: Array<{ name: string; text: string }> = [
  { name: 'empty', text: '' },
  {
    name: 'thread-settings-applied-overrides-earlier-turn-context',
    text: [
      jline({ type: 'world_state', payload: { state: { collaboration_mode: { model: 'gpt-6-sol' } } } }),
      jline({ type: 'turn_context', payload: { model: 'gpt-6-sol', effort: 'high' } }),
      jline({ type: 'event_msg', payload: { type: 'thread_settings_applied', thread_settings: { model: 'gpt-6-astra' } } }),
    ].join('\n'),
  },
  {
    name: 'turn-context-with-effort',
    text: [
      jline({ type: 'world_state', payload: { state: { collaboration_mode: { model: 'gpt-6-sol' } } } }),
      jline({ type: 'turn_context', payload: { model: 'gpt-6-sol', effort: 'high' } }),
    ].join('\n'),
  },
  {
    name: 'collaboration-mode-reasoning-effort-and-timestamp',
    text: jline({
      timestamp: '2026-10-14T01:54:31.711Z',
      type: 'turn_context',
      payload: {
        model: 'gpt-6-astra',
        collaboration_mode: { mode: 'default', settings: { model: 'gpt-6-astra', reasoning_effort: 'xhigh' } },
      },
    }),
  },
  {
    name: 'thread-settings-reasoning-effort',
    text: jline({
      timestamp: '2026-10-14T01:54:31.600Z',
      type: 'event_msg',
      payload: { type: 'thread_settings_applied', thread_settings: { model: 'gpt-6-astra', reasoning_effort: 'xhigh' } },
    }),
  },
];

const codexTurnCases = codexInputs.map((c) => ({
  name: c.name,
  text: c.text,
  expected: normFound(vmApi.codexModelFromText(c.text)),
}));

const uuidV7Samples = [
  '01a0d616-11e1-7202-9d2b-4b4172f348e9',
  '019a2b3c-4d5e-7f60-8a9b-0c1d2e3f4a5b',
  '6da350a1-326a-4f43-b9cc-9fae8081edcc',
  '00000000-0000-7000-8000-000000000000',
  'not-a-uuid',
];
const uuidV7Cases = uuidV7Samples.map((id) => {
  const ms = vmApi.museSessionStart(id);
  return { id, expected: ms > 0 ? ms : null };
});

// -----------------------------------------------------------------------------
// 3. Copilot, Antigravity & Muse cases
// -----------------------------------------------------------------------------
const copilotEv = (type: string, data: Record<string, unknown>, s: number): string =>
  jline({ type, data, id: `e${s}`, timestamp: iso(s), parentId: null });

const copilotInputs: Array<{ name: string; text: string }> = [
  { name: 'empty', text: '' },
  {
    name: 'answer-inherits-latest-model-change-effort',
    text: [
      copilotEv('session.model_change', { newModel: 'auto', reasoningEffort: 'medium' }, 1),
      copilotEv('assistant.message', { model: 'gpt-6-luna', content: 'Hi' }, 2),
    ].join('\n'),
  },
  {
    name: 'model-change-after-answer-wins',
    text: [
      copilotEv('session.model_change', { newModel: 'auto', reasoningEffort: 'medium' }, 1),
      copilotEv('assistant.message', { model: 'gpt-6-luna', content: 'Hi' }, 2),
      copilotEv('session.model_change', { newModel: 'claude-sonnet-5', previousModel: 'auto', reasoningEffort: 'high' }, 3),
    ].join('\n'),
  },
  {
    name: 'answer-only-without-model-change',
    text: copilotEv('assistant.message', { model: 'mai-code-1.1-flash' }, 4),
  },
];

const copilotCases = copilotInputs.map((c) => ({
  name: c.name,
  text: c.text,
  expected: normFound(vmApi.copilotModelFromText(c.text)),
}));

const agyChange = (from: string, to: string): string =>
  `<USER_SETTINGS_CHANGE>\nThe user changed setting \`Model Selection\` from ${from} to ${to}. No need to comment on this change if the user doesn't ask about it.\n</USER_SETTINGS_CHANGE>`;
const agyInput = (content: string, at: string = '2026-10-15T12:11:51Z', step: number = 0): string =>
  jline({ step_index: step, source: 'USER_EXPLICIT', type: 'USER_INPUT', status: 'DONE', created_at: at, content });

const agyFirst = agyInput(`<USER_REQUEST>\nhello\n</USER_REQUEST>\n${agyChange('None', 'Gemini 4 Argon (High)')}`);
const agyLater = agyInput(
  `<USER_REQUEST>\nnext\n</USER_REQUEST>\n${agyChange('Gemini 4 Argon (High)', 'Gemini 3.8 Flash (Medium)')}`,
  '2026-10-15T12:20:00Z',
  6,
);
const agyReply = jline({ step_index: 7, source: 'MODEL', type: 'PLANNER_RESPONSE', created_at: '2026-10-15T12:20:05Z', content: 'ok' });

const antigravityInputs: Array<{ name: string; text: string }> = [
  { name: 'first-prompt-model-selection', text: agyFirst },
  { name: 'later-prompt-overrides-earlier', text: [agyFirst, agyReply, agyLater, agyReply].join('\n') },
  { name: 'thinking-note-preserved-when-not-effort', text: agyInput(agyChange('None', 'Claude Sonnet 4.6 (Thinking)')) },
  { name: 'user-request-spoofing-rejected', text: agyInput(`<USER_REQUEST>\n${agyChange('None', 'Fake Model (High)')}\n</USER_REQUEST>`) },
  { name: 'planner-response-spoofing-rejected', text: jline({ type: 'PLANNER_RESPONSE', content: agyChange('None', 'Gemini 4 Argon (Low)') }) },
  {
    name: 'multiple-changes-in-one-user-input-picks-last',
    text: agyInput(`${agyChange('None', 'Gemini 3.8 Flash (Low)')}\n${agyChange('Gemini 3.8 Flash (Low)', 'Claude Opus 5.5 (High)')}`),
  },
];

const antigravityCases = antigravityInputs.map((c) => ({
  name: c.name,
  text: c.text,
  expected: normFound(vmApi.antigravityModelFromText(c.text)),
}));

const antigravityTimelineLines = [agyFirst, agyReply, agyLater];
const antigravityTimelineExpected = vmApi.antigravityModelTimeline(antigravityTimelineLines);

const museRec = (payload_type: string, payload: Record<string, unknown>, recorded_at: unknown): string =>
  jline({ record_type: 'event', payload_type, recorded_at, payload });

const museSampleLines = [
  museRec('runtime.session.metadata', { record: { workspace_root: 'C:\\Proj', model_id: 'muse-spark-1.3', reasoning_effort: 'high' } }, 1_760_529_600_000_000),
  museRec('run.model.configured', { record: { model_id: 'muse-spark-1.3', reasoning_effort: 'high' } }, '2026-10-15T14:00:00Z'),
  museRec('runtime.model_reconfigure.completed', { record: { effective: { model_id: 'muse-spark-1.3-contributor' } } }, '2026-10-15T14:10:00Z'),
  museRec('runtime.reasoning_effort_reconfigure.completed', { record: { reasoning_effort: 'max' } }, '2026-10-15T14:11:00Z'),
];

const museInputs: Array<{ name: string; tail: string; head?: string }> = [
  { name: 'configured-and-reconfigured-with-effort', tail: museSampleLines.join('\n') },
  {
    name: 'batch-children-record-json-unwrapped',
    tail: jline({
      children: [
        { record_json: museRec('run.model.configured', { record: { model_id: 'muse-wonder-1.0', reasoning_effort: 'medium' } }, '2026-10-15T14:00:00Z') },
      ],
    }),
  },
  {
    name: 'fallback-used-model-when-not-configured',
    tail: museRec('runtime.session', { event: { kind: 'model_completed', model: 'muse-wonder-full' } }, '2026-10-15T14:05:00Z'),
  },
  {
    name: 'long-log-merges-tail-model-with-head-effort',
    tail: museRec('runtime.model_reconfigure.completed', { record: { effective: { model_id: 'muse-wonder' } } }, '2026-10-15T15:00:00Z'),
    head: museSampleLines.slice(0, 2).join('\n'),
  },
];

const museCases = museInputs.map((c) => {
  if (c.head !== undefined) {
    const id = '0199e7d1-e800-7000-8000-000000000001';
    const d = new Date(vmApi.museSessionStart(id));
    const day = `${d.getUTCFullYear()}\\${String(d.getUTCMonth() + 1).padStart(2, '0')}\\${String(d.getUTCDate()).padStart(2, '0')}`;
    const filler = `${museRec('runtime.session', { event: { kind: 'tool_output', text: 'x'.repeat(4000) } }, '2026-10-15T14:01:00Z')}\n`.repeat(80);
    resetVirtual({
      files: {
        [`.local\\share\\muse\\sessions\\${day}\\${id}\\session.jsonl`]: `${c.head}\n${filler}${c.tail}\n`,
      },
    });
    return {
      name: c.name,
      tail: c.tail,
      head: c.head,
      expected: normFound(vmApi.museSession(id, HOME)),
    };
  }
  return {
    name: c.name,
    tail: c.tail,
    head: '',
    expected: normFound(vmApi.museModelFromText(c.tail)),
  };
});

const museTimelineExpected = vmApi.museModelTimeline(museSampleLines.join('\n'));

const museSettingsSamples: Array<{ name: string; json: string; probeIds: string[] }> = [
  {
    name: 'muse-1-4-10-per-model-efforts-and-bad-entries-filtered',
    json: JSON.stringify({
      schema_version: 1,
      provider: 'meta',
      model: 'muse-wonder-1.0',
      model_reasoning_efforts: {
        'muse-wonder-1.0': 'medium',
        'muse-spark-1.3': 'xhigh',
        'bad id;': 'low',
        'muse-spark-1.2': 'turbo',
      },
    }),
    probeIds: ['muse-wonder-1.0', 'muse-spark-1.3', 'muse-spark-1.2', 'bad id;'],
  },
  {
    name: 'nested-model-object-with-global-reasoning-effort',
    json: JSON.stringify({ model: { id: 'muse-spark-1.3' }, reasoning_effort: 'high' }),
    probeIds: ['muse-spark-1.3', 'other-model'],
  },
  {
    name: 'effort-map-without-default-model',
    json: JSON.stringify({ model_reasoning_efforts: { 'muse-spark-1.3': 'ultra' } }),
    probeIds: ['muse-spark-1.3', 'unknown'],
  },
  {
    name: 'non-model-object-returns-null',
    json: JSON.stringify({ schema_version: 1, model: { theme: 'dark' } }),
    probeIds: ['muse-spark-1.3'],
  },
  {
    name: 'trimmed-key-collision-later-inserted-key-wins',
    json: '{"model":"muse-spark-1.3","model_reasoning_efforts":{"muse-spark-1.3":"max"," muse-spark-1.3 ":"low"}}',
    probeIds: ['muse-spark-1.3'],
  },
  {
    name: 'slice-first-100-uses-insertion-order-not-lexicographic',
    json: (() => {
      const pairs: string[] = [];
      for (let i = 0; i < 100; i++) {
        pairs.push(`"z-entry-${String(i).padStart(3, '0')}":"medium"`);
      }
      pairs.push('"a-after-100":"xhigh"');
      return `{"model_reasoning_efforts":{${pairs.join(',')}}}`;
    })(),
    probeIds: ['z-entry-000', 'z-entry-099', 'a-after-100'],
  },
];

const museSettingsCases = museSettingsSamples.map((c) => {
  resetVirtual({ files: { '.config\\muse\\settings.json': c.json } });
  const parsed = vmApi.museSettingsModel(HOME);
  return {
    name: c.name,
    json: c.json,
    probeIds: c.probeIds,
    expected: parsed
      ? {
          model: parsed.model,
          effort: parsed.effort,
          effortsByProbe: Object.fromEntries(c.probeIds.map((id) => [id, parsed.effortFor(id)])),
        }
      : null,
  };
});

// -----------------------------------------------------------------------------
// 4. Other agent session, TOML, YAML, DB & config parsers (`agentModelSources.js`)
// -----------------------------------------------------------------------------
const geminiCases = [
  {
    name: 'latest-gemini-answer-ignores-user-and-metadata',
    text: [
      jline({ sessionId: '3f2a9c4e-8b1d-4e6f-9a7c-5d4e3b2a1f00', startTime: iso(0) }),
      jline({ id: 'u1', timestamp: iso(1), type: 'user', content: [{ text: 'Use gemini-2.5-flash' }] }),
      jline({ id: 'g1', timestamp: iso(2), type: 'gemini', content: 'On it.', model: 'gemini-3-pro-preview' }),
      jline({ $set: { lastUpdated: iso(3) } }),
    ].join('\n'),
  },
  {
    name: 'user-only-returns-null',
    text: jline({ type: 'user', content: 'gemini-3-flash' }),
  },
].map((c) => ({ ...c, expected: normFound(vmApi.geminiModelFromText(c.text)) }));

const qwenCases = [
  {
    name: 'latest-assistant-answer',
    text: [
      jline({ type: 'user', timestamp: iso(1), message: { role: 'user' } }),
      jline({ type: 'assistant', timestamp: iso(2), model: 'qwen3-coder-plus', message: { role: 'model' } }),
    ].join('\n'),
  },
].map((c) => ({ ...c, expected: normFound(vmApi.qwenModelFromText(c.text)) }));

const kimiBind = (alias: string, effort: string, time: number, prompt: string = 'You are Kimi Code.'): string =>
  jline({ type: 'profile.bind', agentId: 'main', modelAlias: alias, profileName: 'default', thinkingEffort: effort, systemPrompt: prompt, disallowedTools: [], time });
const kimiUpdate = (fields: Record<string, unknown>, time: number): string =>
  jline({ type: 'config.update', agentId: 'main', ...fields, time });
const kimiCutHead = kimiBind('kimi-code/kimi-for-coding', 'low', T0, 'p'.repeat(5000)).slice(0, 300);

const kimiWireCases = [
  {
    name: 'bind-then-model-and-effort-updates',
    tail: [
      jline({ type: 'metadata', protocol_version: '1.5', created_at: T0 }),
      kimiBind('kimi-code/kimi-for-coding', 'high', T0),
      kimiUpdate({ modelAlias: 'kimi-code/kimi-k2-turbo', thinkingEffort: 'high' }, T0 + 5000),
      kimiUpdate({ thinkingEffort: 'max' }, T0 + 9000),
    ].join('\n'),
    head: '',
  },
  { name: 'thinking-off-maps-to-none', tail: kimiBind('kimi-code/kimi-for-coding', 'off', T0), head: '' },
  { name: 'thinking-on-maps-to-null', tail: kimiBind('my/model', 'on', T0), head: '' },
  { name: 'truncated-head-bind-plus-tail-effort-update', tail: kimiUpdate({ thinkingEffort: 'high' }, T0 + 1000), head: kimiCutHead },
  { name: 'truncated-head-bind-alone', tail: '', head: kimiCutHead },
].map((c) => ({ ...c, expected: normFound(vmApi.kimiModelFromWire(c.tail, c.head)) }));

const sampleKimiToml = [
  'default_model = "kimi-code/kimi-for-coding"',
  '',
  '[[hooks]]',
  'event = "Stop"',
  'command = "node x.cjs"',
  '',
  '[thinking]',
  'enabled = true',
  'effort = "high" # its default',
  '',
  '[models."kimi-code/kimi-for-coding"]',
  'provider = "managed:kimi-code"',
  'model = "kimi-for-coding"',
  'display_name = "Kimi K2.6"',
  'max_context_size = 262144',
].join('\n');

const kimiConfigCases = [
  { name: 'full-config-with-thinking-and-display-name', toml: sampleKimiToml },
  { name: 'thinking-disabled-maps-to-none', toml: 'default_model = "x"\n[thinking]\nenabled = false\n' },
  { name: 'no-default-model-returns-null', toml: '[thinking]\neffort = "high"\n' },
].map((c) => ({ ...c, expected: normFound(vmApi.kimiConfigModel(c.toml)) }));

const piRows = [
  { type: 'session', version: 3, id: '019a2b3c-4d5e-7f60-8a9b-0c1d2e3f4a5b', timestamp: iso(0), cwd: 'C:\\Proj' },
  { type: 'model_change', id: 'e1', parentId: null, timestamp: iso(0), provider: 'openai-codex', modelId: 'gpt-5.4' },
  { type: 'thinking_level_change', id: 'e2', parentId: 'e1', timestamp: iso(0), thinkingLevel: 'low' },
  { type: 'message', id: 'e3', parentId: 'e2', timestamp: iso(5), message: { role: 'user', content: [{ type: 'text', text: 'Switch to claude-opus-5' }] } },
  { type: 'message', id: 'e4', parentId: 'e3', timestamp: iso(9), message: { role: 'assistant', content: [{ type: 'text', text: 'Done.' }], provider: 'openai-codex', model: 'gpt-5.4' } },
];
const piLater = [
  ...piRows,
  { type: 'model_change', timestamp: iso(20), provider: 'anthropic', modelId: 'claude-opus-5-5' },
  { type: 'thinking_level_change', timestamp: iso(21), thinkingLevel: 'off' },
];

const piWireCases = [
  { name: 'initial-session-and-assistant-answer', tail: piRows.map(jline).join('\n'), head: '' },
  { name: 'later-model-and-thinking-off', tail: piLater.map(jline).join('\n'), head: '' },
  {
    name: 'long-session-keeps-head-thinking-level',
    tail: jline(piRows[4]),
    head: piRows.slice(0, 3).map(jline).join('\n'),
  },
].map((c) => ({ ...c, expected: normFound(vmApi.piModelFromWire(c.tail, c.head)) }));

const piTimelineExpected = vmApi.piModelTimeline(piLater.map(jline));

const opencodeCol = JSON.stringify({ id: 'deepseek-v4.1-flash-free', providerID: 'opencode', variant: 'high' });
const opencodeSessionCases = [
  {
    name: 'assistant-message-inherits-column-effort-when-same-model',
    column: opencodeCol,
    rows: [{ data: JSON.stringify({ role: 'assistant', modelID: 'deepseek-v4.1-flash-free', providerID: 'opencode', mode: 'build' }), time_created: 5 }],
    updated: null,
  },
  {
    name: 'user-message-default-variant-clears-effort',
    column: opencodeCol,
    rows: [{ data: JSON.stringify({ role: 'user', model: { providerID: 'opencode', modelID: 'big-pickle', variant: 'default' } }), time_created: 6 }],
    updated: null,
  },
  {
    name: 'empty-messages-falls-back-to-session-column',
    column: opencodeCol,
    rows: [],
    updated: 7,
  },
].map((c) => ({ ...c, expected: normFound(vmApi.opencodeSessionModel(c.column, c.rows, c.updated)) }));

const clineRows = [
  { pid: 999, model: 'gone-model', cwd: 'C:\\Proj', ended_at: null, is_subagent: 0, updated_at: '2026-09-26T19:30:00Z' },
  { pid: 200, model: 'other-folder', cwd: 'C:\\Else', ended_at: null, is_subagent: 0, updated_at: '2026-09-26T19:29:00Z' },
  { pid: 300, model: 'deepseek/deepseek-v4-flash', cwd: 'C:\\Proj', ended_at: null, is_subagent: 0, updated_at: '2026-09-26T19:00:00Z' },
  { pid: 100, model: 'sub', cwd: 'C:\\Proj', ended_at: null, is_subagent: 1, updated_at: '2026-09-26T19:31:00Z' },
];
const clineAlive = (pid: number): boolean => pid !== 999;
const clineCases = [
  { name: 'prefers-matching-cwd-over-newer-other-folder', cwd: 'C:/Proj/', expected: normFound(vmApi.clineSessionModel(clineRows, 'C:/Proj/', clineAlive)) },
  { name: 'falls-back-to-running-other-folder', cwd: 'C:\\Nowhere', expected: normFound(vmApi.clineSessionModel(clineRows, 'C:\\Nowhere', clineAlive)) },
];

const continueYaml = [
  'name: Local Config',
  'version: 1.0.0',
  'schema: v1',
  'models:',
  '  - name: "Embedder"',
  '    provider: ollama',
  '    model: nomic-embed-text',
  '    roles: [embed]',
  '  - name: Claude Opus 4.6',
  '    provider: anthropic',
  '    model: claude-opus-4-6',
  '    roles:',
  '      - chat',
  '      - edit',
  '  - name: Autodetect',
  '    provider: ollama',
  '    model: AUTODETECT',
].join('\n');

const continueCases = [
  { name: 'default-picks-first-chat-model-skipping-embed', selected: null as string | null, yaml: continueYaml },
  { name: 'autodetect-uses-display-name-as-model', selected: 'Autodetect', yaml: continueYaml },
  { name: 'unlisted-selected-name-used-directly', selected: 'Custom Pick', yaml: continueYaml },
].map((c) => ({ ...c, expected: normFound(vmApi.continueModel(c.selected ?? undefined, c.yaml)) }));

const miscConfigCases = {
  piSettings: normFound(vmApi.piSettingsModel({ defaultProvider: 'anthropic', defaultModel: 'claude-sonnet-5', defaultThinkingLevel: 'medium' })),
  openclawString: normFound(vmApi.openclawConfigModel({ agents: { defaults: { model: 'anthropic/claude-opus-5' } } })),
  openclawPrimary: normFound(vmApi.openclawConfigModel({ agents: { defaults: { model: { primary: 'openai-codex/gpt-5.4' } } } })),
  droidSettings: normFound(vmApi.droidSettingsModel({ model: 'gpt-5.1-codex', reasoningEffort: 'high' })),
  crushConfig: normFound(vmApi.crushConfigModel({ models: { large: { model: 'gpt-5.1', provider: 'openai', reasoning_effort: 'high' } } })),
  gooseConfig: normFound(vmApi.gooseConfigModel('GOOSE_PROVIDER: anthropic\nGOOSE_MODEL: "claude-sonnet-4-5" # default\n')),
  aiderConfig: normFound(vmApi.aiderConfigModel('model: o4-mini\nreasoning-effort: high\nauto-commits: false\n')),
  vibeConfig: normFound(vmApi.vibeConfigModel('active_model = "devstral-2"\n\n[[models]]\nname = "mistral-medium"\n')),
  grokSummary: normFound(vmApi.grokSummaryModel({ current_model_id: 'grok-4-code', reasoning_effort: 'high', updated_at: iso(12) })),
  grokCatalog: vmApi.grokCatalogEntry(
    {
      models: {
        'grok-4-code': {
          info: {
            name: 'Grok 4 Code',
            reasoning_efforts: [
              { value: 'low', default: false },
              { value: 'medium', default: true },
            ],
          },
        },
      },
    },
    'grok-4-code',
  ),
  grokConfig: normFound(vmApi.grokConfigModel('[models]\ndefault = "grok-4-code"\ndefault_reasoning_effort = "xhigh"\n')),
};

// -----------------------------------------------------------------------------
// 5. Command-line, TOML, Cursor config & Claude settings effort cases
// -----------------------------------------------------------------------------
const commandSamples = [
  'claude --model opus --effort high',
  'gemini -m gemini-3-pro',
  'opencode --model="anthropic/claude-sonnet-5"',
  "aider -m 'o4-mini' --reasoning-effort low",
  'codex --no-daemon',
  'my-mcp-tool',
  'ollama run glm-5.2:cloud',
  '"C:\\x\\ollama.exe" run --keepalive 5m --verbose qwen3:8b hi',
  'claude ollama launch claude --model glm-5.2:cloud',
  'ollama serve',
  'claude --effort=MAX',
  'claude --effort banana',
  'muse --reasoning-effort none',
  'muse --effort=ULTRA',
  'muse --effort turbo',
  'copilot --reasoning-effort=high',
  'pi --model openai/gpt-5.4 --thinking xhigh',
  'agent --thinking --model x',
  'amp --mode deep --effort medium',
  'amp --mode=free',
];

const commandCases = commandSamples.map((cmd) => ({
  command: cmd,
  model: vmApi.modelFromCommand(cmd),
  claudeEffort: vmApi.effortFromCommand(cmd),
  museEffort: vmApi.museEffortFromCommand(cmd),
  effortFlag: vmApi.effortFlag(cmd),
  ampMode: vmApi.ampModeFromCommand(cmd),
}));

const sameClaudeCases = [
  ['claude-fable-5-1', 'fable'],
  ['opus[1m]', 'OPUS'],
  ['claude-opus-5-5[1m]', 'opus[1m]'],
  ['claude-opus-5-5', 'sonnet'],
  ['', 'opus'],
].map(([a, b]) => ({ a, b, expected: vmApi.sameClaudeModel(a, b) }));

const codexTomlCases = [
  {
    name: 'top-level-model-and-effort-ignores-profile-section',
    toml: 'model = "gpt-6-astra"\nmodel_reasoning_effort = "medium"\n[profiles.x]\nmodel = "o"',
  },
  { name: 'profile-only-returns-null', toml: '[profiles.x]\nmodel = "o"' },
].map((c) => ({ ...c, expected: normFound(vmApi.codexModelFromToml(c.toml)) }));

const cursorConfigCases = [
  { name: 'luna-272k-medium', text: JSON.stringify({ model: { modelId: 'gpt-5.6-luna', displayName: 'GPT-5.6 Luna 272K Medium' } }) },
  { name: 'sol-1m-extra-high-fast', text: JSON.stringify({ model: { modelId: 'gpt-5.6-sol', displayName: 'GPT-5.6 Sol 1M Extra High Fast' } }) },
  { name: 'auto-default', text: JSON.stringify({ model: { modelId: 'default', displayName: 'Auto' } }) },
  { name: 'invalid-json', text: 'not json' },
  { name: 'bad-model-id', text: JSON.stringify({ model: { modelId: 'bad id!' } }) },
].map((c) => ({ ...c, expected: normFound(vmApi.cursorConfigModel(c.text)) }));

const claudeEffortDocs = {
  local: JSON.stringify({ modelSettings: { 'claude-sonnet-5-5': { effortLevel: 'low' } } }),
  project: JSON.stringify({ effortLevel: 'medium' }),
  user: JSON.stringify({ model: 'opus[1m]', effortLevel: 'xhigh', modelSettings: { 'claude-fable-5-1': { effortLevel: 'high' } } }),
};
resetVirtual({
  files: {
    'proj\\.claude\\settings.local.json': claudeEffortDocs.local,
    'proj\\.claude\\settings.json': claudeEffortDocs.project,
    '.claude\\settings.json': claudeEffortDocs.user,
  },
});
const claudeSettingsEffortBaseCases = [
  { model: 'claude-fable-5-1', useCwd: false },
  { model: 'fable', useCwd: false },
  { model: 'opus[1m]', useCwd: false },
  { model: null, useCwd: false },
  { model: 'sonnet', useCwd: true },
  { model: 'opus', useCwd: true },
].map((c) => ({
  model: c.model,
  docs: c.useCwd ? [claudeEffortDocs.local, claudeEffortDocs.project, claudeEffortDocs.user] : [claudeEffortDocs.user],
  expected: vmApi.claudeSettingsEffort(c.model, c.useCwd ? path.win32.join(HOME, 'proj') : null, HOME),
}));

const claudeSettingsOrderSamples: Array<{ model: string | null; docs: string[] }> = [
  {
    model: 'claude-opus-4-8',
    docs: ['{"modelSettings":{"claude-opus-4-8":{"effortLevel":"high"},"CLAUDE-OPUS-4-8":{"effortLevel":"low"}}}'],
  },
  {
    model: 'claude-opus-4-8',
    docs: ['{"modelSettings":{"CLAUDE-OPUS-4-8":{"effortLevel":"low"},"claude-opus-4-8":{"effortLevel":"high"}}}'],
  },
  {
    model: 'opus',
    docs: ['{"modelSettings":{"claude-opus-5-5":{"effortLevel":"max"},"claude-opus-4-8":{"effortLevel":"low"}}}'],
  },
  {
    model: 'claude-opus-4-8',
    docs: [
      '{"modelSettings":{"claude-opus-4-8":{"effortLevel":"high"},"CLAUDE-OPUS-4-8":{"effortLevel":"low"},"claude-opus-4-8":{"effortLevel":"max"}}}',
    ],
  },
  {
    model: 'claude-opus-4-8',
    docs: [
      '{"modelSettings":{"claude-opus-4-8":{"effortLevel":"max"},"CLAUDE-OPUS-4-8":{"effortLevel":"low"},"claude-opus-4-8":{"effortLevel":"bogus"}}}',
    ],
  },
  {
    model: 'opus[1m]',
    docs: [
      '{\n  // JSONC comment + trailing commas + duplicate inner effortLevel\n  "modelSettings": {\n    "claude-opus-4-8": { "effortLevel": "low", "effortLevel": "xhigh", },\n    "CLAUDE-OPUS-4-8": { "effortLevel": "medium" },\n  },\n}',
    ],
  },
  {
    model: '2',
    docs: ['{"modelSettings":{"z":{"effortLevel":"low"},"10":{"effortLevel":"high"},"2":{"effortLevel":"max"}}}'],
  },
];

const claudeSettingsOrderCases = claudeSettingsOrderSamples.map((c) => {
  const files: Record<string, string> = {};
  if (c.docs.length === 1) {
    files['.claude\\settings.json'] = c.docs[0];
  } else {
    if (c.docs[0] !== undefined) files['proj\\.claude\\settings.local.json'] = c.docs[0];
    if (c.docs[1] !== undefined) files['proj\\.claude\\settings.json'] = c.docs[1];
    if (c.docs[2] !== undefined) files['.claude\\settings.json'] = c.docs[2];
  }
  resetVirtual({ files });
  return {
    model: c.model,
    docs: c.docs,
    expected: vmApi.claudeSettingsEffort(c.model, c.docs.length > 1 ? path.win32.join(HOME, 'proj') : null, HOME),
  };
});

const claudeSettingsEffortCases = [...claudeSettingsEffortBaseCases, ...claudeSettingsOrderCases];

// -----------------------------------------------------------------------------
// 6. End-to-end `agentModel` resolution vectors (`resolve_agent_model`)
// -----------------------------------------------------------------------------
const SID_CLAUDE = 'a8319dbf-1b2e-4bcb-90c7-dda297a37a57';
const SID_AGY = '89cec4e6-00f8-47a4-b013-796a12edd5e4';
const SID_MUSE = '0199e7d1-e800-7000-8000-000000000002';

interface AgentModelScenario {
  name: string;
  snap: VirtualSnapshot;
  query: Record<string, unknown>;
  inputs: Record<string, unknown>;
}

const agentModelScenarios: AgentModelScenario[] = [
  {
    name: 'claude-session-1m-promotion-and-different-chosen-effort',
    snap: {
      files: {
        [`.claude\\projects\\C--x\\${SID_CLAUDE}.jsonl`]: `${claudeAns('claude-opus-5-5', null, T1)}\n`,
        '.claude\\settings.json': JSON.stringify({
          model: 'opus[1m]',
          effortLevel: 'xhigh',
          modelSettings: { 'claude-fable-5-1': { effortLevel: 'high' } },
        }),
      },
    },
    query: { agentId: 'claude', sessionId: SID_CLAUDE, command: 'claude', chosenModel: 'fable' },
    inputs: {
      agentId: 'claude',
      session: normFound(vmApi.claudeTurnFromText(claudeAns('claude-opus-5-5', null, T1))),
      command: 'claude',
      chosenModel: 'fable',
      claudeSettingsDocs: [
        JSON.stringify({
          model: 'opus[1m]',
          effortLevel: 'xhigh',
          modelSettings: { 'claude-fable-5-1': { effortLevel: 'high' } },
        }),
      ],
    },
  },
  {
    name: 'claude-chosen-effort-without-any-resolved-model',
    snap: {
      files: {
        '.claude\\settings.json': JSON.stringify({ effortLevel: 'xhigh' }),
      },
    },
    query: { agentId: 'claude', command: 'claude', chosenModel: 'opus' },
    inputs: {
      agentId: 'claude',
      command: 'claude',
      chosenModel: 'opus',
      claudeSettingsDocs: [JSON.stringify({ effortLevel: 'xhigh' })],
    },
  },
  {
    name: 'claude-model-settings-insertion-and-duplicate-key-order',
    snap: {
      files: {
        '.claude\\settings.json':
          '{"model":"opus","modelSettings":{"claude-opus-5-5":{"effortLevel":"high"},"CLAUDE-OPUS-4-8":{"effortLevel":"low"},"claude-opus-5-5":{"effortLevel":"max"}}}',
      },
    },
    query: { agentId: 'claude', command: 'claude' },
    inputs: {
      agentId: 'claude',
      command: 'claude',
      claudeSettingsDocs: [
        '{"model":"opus","modelSettings":{"claude-opus-5-5":{"effortLevel":"high"},"CLAUDE-OPUS-4-8":{"effortLevel":"low"},"claude-opus-5-5":{"effortLevel":"max"}}}',
      ],
    },
  },
  {
    name: 'antigravity-effort-flag-alone-uses-saved-settings-model',
    snap: {
      files: {
        '.gemini\\antigravity-cli\\settings.json': JSON.stringify({ model: 'Gemini 4 Argon (High)' }),
      },
    },
    query: { agentId: 'antigravity', command: 'agy --effort low' },
    inputs: {
      agentId: 'antigravity',
      command: 'agy --effort low',
      antigravitySettingsJson: JSON.stringify({ model: 'Gemini 4 Argon (High)' }),
    },
  },
  {
    name: 'antigravity-session-beats-command-and-settings',
    snap: {
      files: {
        [`.gemini\\antigravity-cli\\brain\\${SID_AGY}\\.system_generated\\logs\\transcript.jsonl`]: `${agyInput(agyChange('None', 'Claude Opus 5.5 (Medium)'))}\n`,
        '.gemini\\antigravity-cli\\settings.json': JSON.stringify({ model: 'Gemini 4 Argon (High)' }),
      },
    },
    query: { agentId: 'antigravity', sessionId: SID_AGY, command: 'agy --model gemini-4-argon' },
    inputs: {
      agentId: 'antigravity',
      session: normFound(vmApi.antigravityModelFromText(agyInput(agyChange('None', 'Claude Opus 5.5 (Medium)')))),
      command: 'agy --model gemini-4-argon',
      antigravitySettingsJson: JSON.stringify({ model: 'Gemini 4 Argon (High)' }),
    },
  },
  {
    name: 'muse-session-without-effort-inherits-per-model-settings-effort',
    snap: {
      files: (() => {
        const d = new Date(vmApi.museSessionStart(SID_MUSE));
        const day = `${d.getUTCFullYear()}\\${String(d.getUTCMonth() + 1).padStart(2, '0')}\\${String(d.getUTCDate()).padStart(2, '0')}`;
        return {
          [`.local\\share\\muse\\sessions\\${day}\\${SID_MUSE}\\session.jsonl`]: `${museRec('run.model.configured', { record: { model_id: 'muse-spark-1.3' } }, '2026-10-15T14:00:00Z')}\n`,
          '.config\\muse\\settings.json': JSON.stringify({
            model: 'muse-wonder-1.0',
            model_reasoning_efforts: { 'muse-wonder-1.0': 'medium', 'muse-spark-1.3': 'xhigh' },
          }),
        };
      })(),
    },
    query: { agentId: 'muse', sessionId: SID_MUSE, command: 'muse' },
    inputs: {
      agentId: 'muse',
      session: normFound(vmApi.museModelFromText(museRec('run.model.configured', { record: { model_id: 'muse-spark-1.3' } }, '2026-10-15T14:00:00Z'))),
      command: 'muse',
      museSettingsJson: JSON.stringify({
        model: 'muse-wonder-1.0',
        model_reasoning_efforts: { 'muse-wonder-1.0': 'medium', 'muse-spark-1.3': 'xhigh' },
      }),
    },
  },
  {
    name: 'cline-newer-settings-mtime-overrides-older-live-session',
    snap: {
      files: {
        '.cline\\data\\settings\\providers.json': {
          text: JSON.stringify({
            version: 1,
            lastUsedProvider: 'cline',
            providers: { cline: { settings: { model: 'cline-free/deepseek-v4.1-flash', reasoning: { enabled: true, effort: 'medium' } } } },
          }),
          mtimeMs: Date.parse('2026-09-26T20:00:00Z'),
        },
      },
      dbRows: {
        '.cline\\data\\db\\sessions.db': [
          { pid: 300, model: 'old/session-model', cwd: 'C:\\Proj', ended_at: null, is_subagent: 0, updated_at: '2026-09-26T19:00:00Z' },
        ],
      },
    },
    query: { agentId: 'cline', cwd: 'C:\\Proj' },
    inputs: {
      agentId: 'cline',
      session: { model: 'old/session-model', effort: null, at: Date.parse('2026-09-26T19:00:00Z') },
      clineProvidersDoc: {
        text: JSON.stringify({
          version: 1,
          lastUsedProvider: 'cline',
          providers: { cline: { settings: { model: 'cline-free/deepseek-v4.1-flash', reasoning: { enabled: true, effort: 'medium' } } } },
        }),
        mtimeMs: Date.parse('2026-09-26T20:00:00Z'),
      },
    },
  },
  {
    name: 'opencode-picked-after-session-message-beats-session-and-command',
    snap: {
      files: {
        '.local\\state\\opencode\\model.json': {
          text: JSON.stringify({ recent: [{ providerID: 'opencode', modelID: 'big-pickle' }], variant: { 'opencode/big-pickle': 'high' } }),
          mtimeMs: 1_760_000_050_000,
        },
      },
      dbRows: {
        '.local\\share\\opencode\\opencode.db': [
          { __table: 'session', id: 'ses_mine01', directory: 'C:\\Proj', model: JSON.stringify({ id: 'mimo', providerID: 'opencode' }), time_updated: 1_760_000_040_000 },
          { __table: 'message', session_id: 'ses_mine01', time_created: 1_760_000_040_000, data: JSON.stringify({ role: 'assistant', modelID: 'mimo', providerID: 'opencode', variant: 'max' }) },
        ],
      },
    },
    query: { agentId: 'opencode', sessionId: 'ses_mine01', command: 'opencode --model opencode/other', launchedAt: 1_760_000_010_000 },
    inputs: {
      agentId: 'opencode',
      session: { model: 'opencode/mimo', effort: 'max', at: 1_760_000_040_000 },
      command: 'opencode --model opencode/other',
      launchedAtMs: 1_760_000_010_000,
      picked: { model: 'opencode/big-pickle', effort: 'high', mtimeMs: 1_760_000_050_000 },
    },
  },
  {
    name: 'kimi-session-wire-enriched-with-config-toml-display-name',
    snap: {
      files: {
        '.kimi-code\\config.toml': 'default_model = "kimi-code/kimi-for-coding"\n[models."kimi-code/kimi-k2-turbo"]\ndisplay_name = "Kimi K2 Turbo"\n',
        '.kimi-code\\sessions\\ws-1\\session_7f3a9c2e41\\agents\\main\\wire.jsonl': `${kimiBind('kimi-code/kimi-k2-turbo', 'low', T0 + 4000)}\n`,
      },
    },
    query: { agentId: 'kimi', sessionId: 'session_7f3a9c2e41' },
    inputs: {
      agentId: 'kimi',
      session: { model: 'kimi-code/kimi-k2-turbo', effort: 'low', at: T0 + 4000 },
      kimiConfigToml: 'default_model = "kimi-code/kimi-for-coding"\n[models."kimi-code/kimi-k2-turbo"]\ndisplay_name = "Kimi K2 Turbo"\n',
    },
  },
  {
    name: 'amp-mode-from-command-and-effort-flag',
    snap: {},
    query: { agentId: 'amp', command: 'amp --mode deep --effort high' },
    inputs: { agentId: 'amp', command: 'amp --mode deep --effort high' },
  },
  {
    name: 'copilot-chunk-merge-answer-inherits-effort-across-512k-boundary',
    snap: {
      files: {
        '.copilot\\session-state\\sess_chunk01\\events.jsonl': [
          copilotEv('session.model_change', { newModel: 'auto', reasoningEffort: 'high' }, 1),
          jline({ type: 'tool.result', data: { output: 'x'.repeat(600 * 1024) }, id: 'big', timestamp: iso(2), parentId: null }),
          copilotEv('assistant.message', { model: 'gpt-6-luna', content: 'Done' }, 3),
        ].join('\n'),
      },
    },
    query: { agentId: 'copilot', sessionId: 'sess_chunk01' },
    inputs: {
      agentId: 'copilot',
      copilotChunksNewestFirst: [
        copilotEv('assistant.message', { model: 'gpt-6-luna', content: 'Done' }, 3),
        copilotEv('session.model_change', { newModel: 'auto', reasoningEffort: 'high' }, 1),
      ],
    },
  },
  {
    name: 'opencode-jsonc-comments-url-and-trailing-comma',
    snap: {
      files: {
        '.config\\opencode\\opencode.jsonc': [
          '{',
          '  // user config with URL https://example.com//keep-this',
          '  "$schema": "https://opencode.ai/config.json",',
          '  /* block comment */',
          '  "model": "anthropic/claude-sonnet-5",',
          '  "agent": { "build": { "variant": "high", }, },',
          '}',
        ].join('\n'),
      },
    },
    query: { agentId: 'opencode' },
    inputs: {
      agentId: 'opencode',
      opencodeConfigDocs: [
        [
          '{',
          '  // user config with URL https://example.com//keep-this',
          '  "$schema": "https://opencode.ai/config.json",',
          '  /* block comment */',
          '  "model": "anthropic/claude-sonnet-5",',
          '  "agent": { "build": { "variant": "high", }, },',
          '}',
        ].join('\n'),
      ],
    },
  },
];

const agentModelCases = agentModelScenarios.map((sc) => {
  resetVirtual(sc.snap);
  vmApi._clearModelSourceCachesForTest();
  const res = vmApi.agentModel(sc.query, HOME);
  return {
    name: sc.name,
    inputs: sc.inputs,
    expected: normAgentModel(res),
  };
});

// -----------------------------------------------------------------------------
// 7. End-to-end `TerminalPane.vue` `refreshModel` orchestration cases
// -----------------------------------------------------------------------------
const cursorDiscovered = vmApi.groupCursorModels(vmApi.parseCursorModelList(cursorListRaw));
const cursorModels = vmApi.catalogModelsFor('cursor', cursorDiscovered);
const codexModels = vmApi.catalogModelsFor('codex', null);
const claudeModels = vmApi.catalogModelsFor('claude', null);
const agyModels = vmApi.catalogModelsFor('antigravity', [
  { id: 'gemini-4-argon', label: 'Gemini 4 Argon', efforts: ['low', 'medium', 'high'], isDefault: true },
  { id: 'claude-sonnet-4-6-thinking', label: 'Claude Sonnet 4.6 (Thinking)' },
]);
const kimiModels = vmApi.catalogModelsFor('kimi', [
  { id: 'kimi-code/kimi-for-coding', label: 'Kimi K2.6', isDefault: true },
]);
const museModels = vmApi.catalogModelsFor('muse', [
  { id: 'muse-wonder-1.0', label: 'muse-wonder-1.0', isDefault: true },
]);

const catalogs: Record<string, Array<Record<string, unknown>>> = {
  cursor: cursorModels,
  codex: codexModels,
  claude: claudeModels,
  antigravity: agyModels,
  kimi: kimiModels,
  muse: museModels,
};

interface HeaderScenario {
  name: string;
  catalogKey: keyof typeof catalogs;
  node: Record<string, unknown>;
  currentAgentModel?: Record<string, unknown> | null;
  currentSessionModel?: string | null;
  currentScreenContext?: Record<string, unknown> | null;
  hasTerm?: boolean;
  screenLines?: string[];
  topScreenLines?: string[];
  scrollbackLines?: string[];
  agentModelResult?: Record<string, unknown> | null;
}

const headerScenarios: HeaderScenario[] = [
  {
    name: 'cursor-screen-overrides-session-clears-optimistic-and-keeps-separate-session-model',
    catalogKey: 'cursor',
    node: { agentId: 'cursor', modelChoice: { model: 'auto', effort: null, at: 100 } },
    screenLines: ['  → Plan, search, build anything', '  Composer 2.5 272K · 12.5%'],
    agentModelResult: { model: 'default', name: 'Auto', source: 'settings' },
  },
  {
    name: 'cursor-open-picker-freezes-previous-shown-model',
    catalogKey: 'cursor',
    node: { agentId: 'cursor' },
    currentAgentModel: { model: 'composer-2.5', name: 'Composer 2.5', effort: null, source: 'screen' },
    screenLines: ['Available models', 'Type to filter · Enter to select'],
    agentModelResult: { model: 'default', name: 'Auto', source: 'settings' },
  },
  {
    name: 'cursor-context-cleared-on-new-conversation-placeholder',
    catalogKey: 'cursor',
    node: { agentId: 'cursor' },
    currentScreenContext: { pct: null, window: '272K', hover: null },
    screenLines: ['  → Plan, search, build anything', '  Auto'],
    agentModelResult: { model: 'default', name: 'Auto', source: 'settings' },
  },
  {
    name: 'codex-footer-screen-wins-and-live-share-session-model-stays-rollout-only',
    catalogKey: 'codex',
    node: { agentId: 'codex', modelChoice: { model: 'gpt-5.4-mini', effort: 'low', at: 100 } },
    screenLines: ['› Ask Codex anything', '', 'GPT-5.3-Codex high · C:\\Proj'],
    agentModelResult: { model: 'gpt-5.4', effort: 'medium', source: 'session' },
  },
  {
    name: 'antigravity-listed-enriches-variant-id-from-command',
    catalogKey: 'antigravity',
    node: { agentId: 'antigravity' },
    screenLines: [],
    agentModelResult: { model: 'gemini-4-argon-high', effort: null, source: 'command' },
  },
  {
    name: 'keep-flag-preserves-current-shown-model-on-transient-failure',
    catalogKey: 'claude',
    node: { agentId: 'claude' },
    currentAgentModel: { model: 'claude-opus-5-5', effort: 'high', source: 'session' },
    currentSessionModel: 'claude-opus-5-5',
    screenLines: [],
    agentModelResult: { keep: true },
  },
  {
    name: 'keep-flag-with-no-current-model-falls-back-to-screen-banner',
    catalogKey: 'claude',
    node: { agentId: 'claude' },
    currentAgentModel: null,
    screenLines: ['Claude Code v2.1 · claude-opus-5-5'],
    agentModelResult: { keep: true },
  },
  {
    name: 'non-own-name-agent-kimi-uses-catalog-label-while-muse-keeps-product-label',
    catalogKey: 'kimi',
    node: { agentId: 'kimi' },
    screenLines: [],
    agentModelResult: { model: 'kimi-code/kimi-for-coding', effort: 'high', source: 'session' },
  },
  {
    name: 'muse-label-equal-to-id-is-not-copied-into-name-and-footer-fills-missing-effort',
    catalogKey: 'muse',
    node: { agentId: 'muse' },
    screenLines: ['muse-wonder-1.0 · medium · C:\\Proj · YOLO'],
    agentModelResult: { model: 'muse-wonder-1.0', effort: null, source: 'session' },
  },
  {
    name: 'muse-footer-provides-screen-model-before-any-session-file',
    catalogKey: 'muse',
    node: { agentId: 'muse' },
    screenLines: ['muse-wonder-1.0 · high · C:\\Proj'],
    agentModelResult: null,
  },
  {
    name: 'claude-switched-on-screen-overrides-settings-and-promotes-version',
    catalogKey: 'claude',
    node: { agentId: 'claude' },
    screenLines: [],
    scrollbackLines: ['> /model sonnet', '  ⎿  Set model to sonnet (claude-sonnet-5-5)'],
    agentModelResult: { model: 'opus', effort: 'xhigh', source: 'settings' },
  },
  {
    name: 'claude-chosen-alias-promoted-by-session-version',
    catalogKey: 'claude',
    node: { agentId: 'claude', sessionOptions: { model: 'opus' } },
    screenLines: [],
    scrollbackLines: ['  ⎿  Set model to opus (claude-opus-5-5)'],
    agentModelResult: { model: null, effort: null, source: null, chosenEffort: 'xhigh' },
  },
  {
    name: 'claude-screen-banner-inherits-chosen-effort-when-no-file-model',
    catalogKey: 'claude',
    node: { agentId: 'claude' },
    screenLines: ['Claude Code v2.1 · claude-opus-5-5'],
    agentModelResult: { model: null, effort: null, source: null, chosenEffort: 'high' },
  },
  {
    name: 'saved-placeholder-never-triggers-pane-model-to-save',
    catalogKey: 'codex',
    node: { agentId: 'codex' },
    currentAgentModel: { model: 'gpt-5.4', effort: 'high', source: 'saved' },
    screenLines: [],
    agentModelResult: { keep: true },
  },
];

const headerRefreshCases = headerScenarios.map((sc) => ({
  scenario: sc,
  expected: vmApi.__simHeaderRefresh({
    ...sc,
    models: catalogs[sc.catalogKey],
  }),
}));

const output = {
  sources,
  catalogs,
  claudeTurnCases,
  codexTurnCases,
  uuidV7Cases,
  copilotCases,
  antigravityCases,
  antigravityTimeline: { lines: antigravityTimelineLines, expected: antigravityTimelineExpected },
  museCases,
  museTimeline: { text: museSampleLines.join('\n'), expected: museTimelineExpected },
  museSettingsCases,
  geminiCases,
  qwenCases,
  kimiWireCases,
  kimiConfigCases,
  piWireCases,
  piTimeline: { lines: piLater.map(jline), expected: piTimelineExpected },
  opencodeSessionCases,
  clineCases,
  continueCases,
  miscConfigCases,
  commandCases,
  sameClaudeCases,
  codexTomlCases,
  cursorConfigCases,
  claudeSettingsEffortCases,
  agentModelCases,
  headerRefreshCases,
};

const dest: string = path.join(__dirname, '..', 'tests', 'fixtures', 'agent_model_readers.json');
fs.mkdirSync(path.dirname(dest), { recursive: true });
fs.writeFileSync(dest, `${JSON.stringify(output, null, 2)}\n`, 'utf8');
console.log(
  `Wrote ${dest} (${claudeTurnCases.length} claude, ${codexTurnCases.length} codex, ${agentModelCases.length} agentModel, ${headerRefreshCases.length} header cases)`,
);
