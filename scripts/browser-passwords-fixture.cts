// Development-only reference extraction for BrowserPasswords model parity:
// - URL origin normalization (`loginOrigin`) & RFC 4180 CSV parsing (`parsePasswordCsv`)
// - Passwordless metadata projection & import reconciliation (`allowAgents = false` reset on secret change)
// - Live Share guest vs host-sharing per-page credential boundaries & local import path validation
// - DOM login-form safety rules & page result sanitization (`passwordFillScript`, `fillSavedPassword`)
// - Sorting (UTF-16), multi-word search, 200-row pagination, bulk `allow_all` state/confirmations,
//   export guides, error messages, and EN/FR pluralized summaries (`BrowserPasswords.vue` + `passwords.json`)
// Strictly read-only against the reference Tessel source tree; uses synthetic fixture tokens only.

const fs = require('node:fs') as typeof import('node:fs');
const path = require('node:path') as typeof import('node:path');
const vm = require('node:vm') as typeof import('node:vm');
const crypto = require('node:crypto') as typeof import('node:crypto');

interface SourceRecord {
  file: string;
  sha256: string;
}

interface OriginVector {
  input: string;
  expected: string | null;
}

interface CsvParsedRowExpectation {
  origin: string | null;
  username: string;
  secretSlot: number;
  secretUtf16Len: number;
  secretByteLen: number;
}

interface CsvVector {
  name: string;
  csv: string;
  expectedOk?: CsvParsedRowExpectation[];
  expectedError?: string;
}

interface VaultEntryMeta {
  id: string;
  origin: string;
  username: string;
  allowAgents: boolean;
}

interface SeedEntry {
  id: string;
  origin: string;
  username: string;
  secretToken: string;
  allowAgents: boolean;
}

interface IncomingSpec {
  origin: string;
  username: string;
  secretToken: string;
  repeatUtf16Username?: number;
  repeatUtf16Secret?: number;
}

interface ImportScenarioResult {
  name: string;
  seed: VaultEntryMeta[];
  seedSlots: Record<string, number>;
  incoming: Array<{
    origin: string;
    usernameLenUtf16: number;
    usernameSample: string;
    secretSlot: number;
    secretLenUtf16: number;
  }>;
  expectedOk?: {
    imported: number;
    added: number;
    updated: number;
    unchanged: number;
    skipped: number;
    entries: VaultEntryMeta[];
    entrySecretSlots: Record<string, number>;
  };
  expectedError?: string;
}

interface PathVector {
  path: string;
  platform: 'win32' | 'linux';
  allowed: boolean;
}

interface PageBoundaryVector {
  name: string;
  op: 'vault_list' | 'vault_change' | 'import_csv_path' | 'user_fill' | 'agent_list' | 'agent_fill';
  trustedSender: boolean;
  joinedAsGuest: boolean;
  hostSharingProjects: string[] | null;
  page?: {
    id: number;
    project: string;
    side?: boolean;
    optIn?: boolean;
    destroyed?: boolean;
    partition: 'owner' | 'agent' | 'guest' | 'view' | 'other';
    url: string;
  };
  csvPath?: string;
  credentialId?: string;
  expectedOk: boolean;
  expectedCode?: string;
  expectedAccountIds?: string[];
}

interface DomFieldSpec {
  type: string;
  autocomplete?: string;
  disabled?: boolean;
  readOnly?: boolean;
  visible?: boolean;
  inForm?: boolean;
  ownedByOtherForm?: boolean;
}

interface DomFormVector {
  name: string;
  entryOrigin: string;
  locationOrigin: string;
  locationProtocol: string;
  locationHref: string;
  baseUri?: string;
  formAction: string | null;
  formConnected?: boolean;
  formInSameDocument?: boolean;
  submitFormActions?: string[];
  mutateOriginDuringUsernameFill?: string;
  reparentPasswordFormDuringUsernameFill?: boolean;
  fields: DomFieldSpec[];
  expectedOk: boolean;
  expectedCode?: string;
  filledUsernameIndex?: number | null;
  filledPasswordIndex?: number | null;
}

interface SanitizeResultVector {
  name: string;
  raw: unknown;
  expectedOk: boolean;
  expectedCode?: string;
}

interface UiViewVector {
  name: string;
  entries: VaultEntryMeta[];
  query: string;
  pagesShown: number;
  lang: 'en' | 'fr';
  expectedTotalRows: number;
  expectedFilteredCount: number;
  expectedVisibleRows: Array<{
    id: string;
    origin: string;
    site: string;
    username: string;
    allowAgents: boolean;
    same: boolean;
  }>;
  expectedRemaining: number;
  expectedAllowedCount: number;
  expectedCountLabel: string;
  expectedAgentsLabel: string;
  expectedNoMatchLabel: string;
  expectedMoreLabel: string;
  expectedBulkChecked: boolean;
  expectedBulkIndeterminate: boolean;
  expectedBulkDisabled: boolean;
  expectedBulkLabel: string;
}

interface BulkConfirmVector {
  name: string;
  entries: VaultEntryMeta[];
  query: string;
  targetAllow: boolean;
  canConfirm: boolean;
  lang: 'en' | 'fr';
  expectedKind: 'noop' | 'refused_without_confirm' | 'apply_immediately' | 'ask_confirm';
  expectedTargetIds: string[];
  expectedPrompt?: {
    title: string;
    text: string;
    confirmLabel: string;
    danger: boolean;
  };
}

interface SummaryVector {
  name: string;
  lang: 'en' | 'fr';
  result: {
    ok: boolean;
    code?: string;
    imported?: number;
    added?: number;
    updated?: number;
    unchanged?: number;
    skipped?: number;
    appBoundCount?: number;
    protectedCount?: number;
  };
  expectedNeedsCsv: boolean;
  expectedText: string;
}

interface BatchSummaryVector {
  name: string;
  lang: 'en' | 'fr';
  profiles: Array<{
    label: string;
    threw?: boolean;
    result?: {
      ok: boolean;
      code?: string;
      imported?: number;
      added?: number;
      updated?: number;
      unchanged?: number;
      skipped?: number;
      appBoundCount?: number;
      protectedCount?: number;
    };
  }>;
  expectedTitle: string;
  expectedItems: Array<{
    index: number;
    label: string;
    ok: boolean;
    warn: boolean;
    csv: boolean;
    text: string;
  }>;
}

const argPath = process.argv[2];
if (!argPath) {
  throw new Error('Pass the reference Tessel repo or src directory (e.g. C:\\Tessel)');
}
const outPath =
  process.argv[3] ?? path.join(__dirname, '..', 'tests', 'fixtures', 'browser_passwords.json');

const srcDir = fs.existsSync(path.join(argPath, 'main', 'browserPasswords.js'))
  ? argPath
  : path.join(argPath, 'src');

if (!fs.existsSync(path.join(srcDir, 'main', 'browserPasswords.js'))) {
  throw new Error(`Could not locate main/browserPasswords.js under ${srcDir}`);
}

const sources: SourceRecord[] = [];
function recordFile(relPath: string): string {
  const full = path.join(srcDir, relPath);
  const text = fs.readFileSync(full, 'utf8');
  sources.push({
    file: relPath.replace(/\\/g, '/'),
    sha256: crypto.createHash('sha256').update(text).digest('hex'),
  });
  return text;
}

function flattenLocale(
  obj: unknown,
  prefix = '',
  out: Record<string, string> = {},
): Record<string, string> {
  if (!obj || typeof obj !== 'object' || Array.isArray(obj)) return out;
  for (const [k, v] of Object.entries(obj as Record<string, unknown>)) {
    const key = prefix ? `${prefix}.${k}` : k;
    if (v && typeof v === 'object' && !Array.isArray(v)) flattenLocale(v, key, out);
    else if (typeof v === 'string') out[key] = v;
  }
  return out;
}

const frPasswordsRaw = recordFile('renderer/src/i18n/locales/fr/passwords.json');
recordFile('renderer/src/i18n/index.js');
const browserPasswordsJs = recordFile('main/browserPasswords.js');
const browserPasswordManagerJs = recordFile('main/browserPasswordManager.js');
const fileTokensJs = recordFile('main/cookieImport/fileTokens.js');
const browserPasswordsVue = recordFile('renderer/src/components/BrowserPasswords.vue');

const frCatalog = flattenLocale(JSON.parse(frPasswordsRaw) as unknown);

function translate(
  lang: 'en' | 'fr',
  key: string,
  fallback: string,
  vars?: Record<string, string | number>,
): string {
  let text: string | undefined;
  if (lang === 'fr') {
    if (vars && typeof vars.count === 'number') {
      const form = new Intl.PluralRules('fr').select(vars.count);
      text = frCatalog[`${key}_${form}`];
    }
    if (text == null) text = frCatalog[key];
  }
  if (text == null) text = fallback;
  if (!vars) return text;
  return text.replace(/\{\{\s*(\w+)\s*\}\}/g, (m, name: string) =>
    name in vars ? String(vars[name]) : m,
  );
}

// Evaluate pure functions from main/browserPasswords.js & main/cookieImport/fileTokens.js
// inside an isolated in-memory VM (no real filesystem or OS encryption access).
interface MemFsState {
  files: Map<string, Buffer>;
  nextUuid: number;
}
const memFsState: MemFsState = {
  files: new Map<string, Buffer>(),
  nextUuid: 1,
};

const memFs = {
  lstatSync(p: string) {
    const buf = memFsState.files.get(p);
    if (!buf) {
      const err = new Error('ENOENT') as Error & { code?: string };
      err.code = 'ENOENT';
      throw err;
    }
    return {
      isFile: () => true,
      isSymbolicLink: () => false,
      size: buf.length,
    };
  },
  statSync(p: string) {
    return memFs.lstatSync(p);
  },
  readFileSync(p: string, enc?: string) {
    const buf = memFsState.files.get(p);
    if (!buf) {
      const err = new Error('ENOENT') as Error & { code?: string };
      err.code = 'ENOENT';
      throw err;
    }
    return enc === 'utf8' ? buf.toString('utf8') : Buffer.from(buf);
  },
  mkdirSync() {
    // in-memory no-op
  },
  writeFileSync(p: string, data: Buffer | string) {
    memFsState.files.set(p, Buffer.isBuffer(data) ? Buffer.from(data) : Buffer.from(data, 'utf8'));
  },
  renameSync(from: string, to: string) {
    const buf = memFsState.files.get(from);
    if (!buf) throw new Error('ENOENT');
    memFsState.files.set(to, buf);
    memFsState.files.delete(from);
  },
  unlinkSync(p: string) {
    memFsState.files.delete(p);
  },
  existsSync(p: string) {
    return memFsState.files.has(p);
  },
};

const vmContext = vm.createContext({
  Buffer,
  URL,
  Object,
  Array,
  String,
  Number,
  Boolean,
  RegExp,
  Map,
  Set,
  Promise,
  Error,
  JSON,
  Math,
  Date,
  process: { platform: 'win32' },
  fs: memFs,
  path: path.posix,
  crypto: {
    randomBytes: (n: number) => crypto.randomBytes(n),
    randomUUID: () => `00000000-0000-4000-8000-${String(memFsState.nextUuid++).padStart(12, '0')}`,
  },
  secureStorageAvailable: (s: { isEncryptionAvailable?: () => boolean } | null | undefined) =>
    !!s?.isEncryptionAvailable?.(),
  withDatabaseCopy: async () => {
    throw new Error('unused');
  },
  sqliteModule: () => {
    throw new Error('unused');
  },
  chromiumKey: async () => {
    throw new Error('unused');
  },
  decryptChromiumValue: () => {
    throw new Error('unused');
  },
  unprotectDpapi: async () => {
    throw new Error('unused');
  },
  detectBrowsers: async () => [],
  readKey4: () => {
    throw new Error('unused');
  },
  decryptLogins: () => {
    throw new Error('unused');
  },
});

function loadEsmIntoVm(code: string): void {
  const transformed = code
    .replace(/^import\s+[\s\S]*?from\s+['"][^'"]+['"]\s*;?\s*$/gm, '')
    .replace(/^export\s+async\s+function\s+([A-Za-z0-9_$]+)/gm, 'async function $1')
    .replace(/^export\s+function\s+([A-Za-z0-9_$]+)/gm, 'function $1')
    .replace(/^export\s+const\s+([A-Za-z0-9_$]+)/gm, 'var $1');
  vm.runInContext(transformed, vmContext);
}

loadEsmIntoVm(fileTokensJs);
loadEsmIntoVm(browserPasswordsJs);
loadEsmIntoVm(browserPasswordManagerJs);

const refLoginOrigin = vmContext.loginOrigin as (url: string) => string | null;
const refParsePasswordCsv = vmContext.parsePasswordCsv as (
  text: string,
) => Array<{ origin: string | null; username: string; password: string }>;
const refCreatePasswordVault = vmContext.createPasswordVault as (opts: {
  file: string;
  safeStorage: {
    isEncryptionAvailable: () => boolean;
    encryptString: (text: string) => Buffer;
    decryptString: (bytes: Buffer) => string;
  };
}) => {
  list: (origin?: string | null, agent?: boolean) => VaultEntryMeta[];
  importRows: (
    rows: Array<{ origin: string | null; username: string; password: string }>,
  ) => {
    ok: boolean;
    imported: number;
    added: number;
    updated: number;
    unchanged: number;
    skipped: number;
  };
  change: (
    id: string,
    action: string,
    allowAgents?: boolean,
  ) => { ok: boolean; code?: string };
  credential: (
    id: string,
    origin: string,
    agent?: boolean,
  ) => { id: string; origin: string; username: string; password: string; allowAgents: boolean };
};
const refPasswordFillScript = vmContext.passwordFillScript as (entry: {
  origin: string;
  username: string;
  password: string;
}) => string;
const refFillSavedPassword = vmContext.fillSavedPassword as (opts: {
  vault: ReturnType<typeof refCreatePasswordVault>;
  guest: {
    getURL: () => string;
    executeJavaScriptInIsolatedWorld: (
      worldId: number,
      sources: Array<{ code: string }>,
      userGesture: boolean,
    ) => Promise<unknown>;
  } | null;
  id: string;
  agent?: boolean;
  allowed: (guest: unknown) => boolean;
}) => Promise<{ ok: boolean; code?: string; submitted?: boolean }>;
const refIsAllowedCookieFilePath = vmContext.isAllowedCookieFilePath as (
  p: string,
  platform: string,
) => boolean;
const refRegisterPasswordManager = vmContext.registerPasswordManager as (opts: {
  ipcMain: { handle: (name: string, fn: (...args: unknown[]) => Promise<unknown>) => void };
  safeStorage: {
    isEncryptionAvailable: () => boolean;
    encryptString: (text: string) => Buffer;
    decryptString: (bytes: Buffer) => string;
  };
  dialog: {
    showOpenDialog: () => Promise<{ canceled: boolean; filePaths: string[] }>;
  };
  dataDir: string;
  getWindow: () => { isDestroyed: () => boolean; webContents: object } | null;
  guestFor: (event: unknown, id: unknown) => unknown;
  allowedPage: (guest: unknown) => boolean;
  guestSessionActive: () => boolean;
}) => {
  login: (guest: unknown, id?: string) => Promise<{
    ok: boolean;
    code?: string;
    accounts?: VaultEntryMeta[];
    submitted?: boolean;
  }>;
};

// Map synthetic test secret tokens to opaque integer slots so no secret text is needed in assertions.
const secretSlotByToken = new Map<string, number>();
function slotOfSecret(token: string): number {
  let hit = secretSlotByToken.get(token);
  if (hit === undefined) {
    hit = secretSlotByToken.size;
    secretSlotByToken.set(token, hit);
  }
  return hit;
}

const memSafeStorage = {
  isEncryptionAvailable: () => true,
  encryptString: (text: string) => Buffer.from(text, 'utf8'),
  decryptString: (bytes: Buffer) => bytes.toString('utf8'),
};

// 1. loginOrigin vectors
const originInputs = [
  'https://example.test/login?next=%2Fdashboard#top',
  'https://EXAMPLE.TEST:443/account',
  'https://example.test:8443/auth',
  'https://[::1]:8443/login',
  'https://bücher.example/signin',
  'https://sub.domain.example.co.uk/',
  'http://example.test/login',
  'http://localhost:3000',
  'https://user@example.test/login',
  'https://:pass@example.test/login',
  'https://user:pass@example.test/login',
  'file:///C:/Users/fixture/passwords.csv',
  'javascript:alert(1)',
  'data:text/html,<h1>hi</h1>',
  'blob:https://example.test/550e8400-e29b-41d4-a716-446655440000',
  'about:blank',
  'tessel-fixture://localhost/',
  '',
  '   ',
  'not-a-url',
];
const originVectors: OriginVector[] = originInputs.map((input) => ({
  input,
  expected: refLoginOrigin(input),
}));

// 2. CSV parsing vectors
const rawCsvInputs: Array<{ name: string; csv: string }> = [
  {
    name: 'quoted_commas_crlf_newlines_and_extra_columns',
    csv: 'name,url,username,password,note\r\nSite,https://example.test/login,"user,name","syn,""quoted""\nline",ignored\r\n',
  },
  {
    name: 'utf8_bom_trimmed_case_insensitive_headers_and_blank_lines',
    csv: '\uFEFF Title , URL , Username , PASSWORD \n\nApp,https://app.example.test:443/p,alice@example.test,syn-alpha\r\n,,,\r\nHttp,http://insecure.example.test,bob,syn-beta\nShortRow,https://only-url.example.test\n',
  },
  {
    name: 'lone_carriage_return_and_whitespace_only_row',
    csv: 'url,username,password\nhttps://cr.example.test,user\rname,syn\rpass\n , , \n',
  },
  {
    name: 'missing_required_column_fails_format',
    csv: 'origin,login,secret\nhttps://example.test,alice,syn-alpha\n',
  },
  {
    name: 'empty_csv_fails_format',
    csv: '',
  },
  {
    name: 'unclosed_quote_fails_format',
    csv: 'url,username,password\nhttps://example.test,"unterminated,syn-alpha\n',
  },
  {
    name: 'over_5000_data_rows_fails_too_big',
    csv:
      'url,username,password\n' +
      Array.from({ length: 5001 }, (_, i) => `https://s${i}.example.test,u,syn-p`).join('\n'),
  },
];

const csvVectors: CsvVector[] = rawCsvInputs.map(({ name, csv }) => {
  try {
    const parsed = refParsePasswordCsv(csv);
    return {
      name,
      csv: name === 'over_5000_data_rows_fails_too_big' ? '<SYNTHETIC_5001_ROWS>' : csv,
      expectedOk: parsed.map((r) => ({
        origin: r.origin,
        username: r.username,
        secretSlot: slotOfSecret(r.password),
        secretUtf16Len: r.password.length,
        secretByteLen: Buffer.byteLength(r.password, 'utf8'),
      })),
    };
  } catch (err) {
    const code = (err as { code?: string }).code ?? 'unknown';
    return {
      name,
      csv: name === 'over_5000_data_rows_fails_too_big' ? '<SYNTHETIC_5001_ROWS>' : csv,
      expectedError: code,
    };
  }
});

// 3. Import reconciliation scenarios
function runImportScenario(
  name: string,
  seedSpecs: SeedEntry[],
  incomingSpecs: IncomingSpec[],
): ImportScenarioResult {
  memFsState.files.clear();
  memFsState.nextUuid = 1;
  const vaultFile = '/mem/browser-passwords.sealed';
  if (seedSpecs.length > 0) {
    const rawSeed = seedSpecs.map((s) => ({
      id: s.id,
      origin: s.origin,
      username: s.username,
      password: s.secretToken,
      allowAgents: s.allowAgents,
    }));
    memFsState.files.set(vaultFile, memSafeStorage.encryptString(JSON.stringify(rawSeed)));
  }
  const vault = refCreatePasswordVault({ file: vaultFile, safeStorage: memSafeStorage });
  const seedSlots: Record<string, number> = {};
  for (const s of seedSpecs) {
    seedSlots[s.id] = slotOfSecret(s.secretToken);
  }
  const realizedIncoming = incomingSpecs.map((spec) => {
    const username = spec.repeatUtf16Username
      ? 'u'.repeat(spec.repeatUtf16Username)
      : spec.username;
    const password = spec.repeatUtf16Secret
      ? 'p'.repeat(spec.repeatUtf16Secret)
      : spec.secretToken;
    return {
      origin: spec.origin,
      username,
      password,
      meta: {
        origin: spec.origin,
        usernameLenUtf16: username.length,
        usernameSample: username.length > 32 ? `${username.slice(0, 16)}...(${username.length})` : username,
        secretSlot: slotOfSecret(password),
        secretLenUtf16: password.length,
      },
    };
  });

  const seedMeta: VaultEntryMeta[] = seedSpecs.map((s) => ({
    id: s.id,
    origin: s.origin,
    username: s.username,
    allowAgents: s.allowAgents,
  }));

  try {
    const res = vault.importRows(
      realizedIncoming.map((x) => ({
        origin: x.origin,
        username: x.username,
        password: x.password,
      })),
    );
    const entries = vault.list();
    const entrySecretSlots: Record<string, number> = {};
    for (const e of entries) {
      const cred = vault.credential(e.id, e.origin, false);
      entrySecretSlots[e.id] = slotOfSecret(cred.password);
    }
    return {
      name,
      seed: seedMeta,
      seedSlots,
      incoming: realizedIncoming.map((x) => x.meta),
      expectedOk: {
        imported: res.imported,
        added: res.added,
        updated: res.updated,
        unchanged: res.unchanged,
        skipped: res.skipped,
        entries,
        entrySecretSlots,
      },
    };
  } catch (err) {
    const code = (err as { code?: string }).code ?? 'unknown';
    return {
      name,
      seed: seedMeta,
      seedSlots,
      incoming: realizedIncoming.map((x) => x.meta),
      expectedError: code,
    };
  }
}

const importScenarios: ImportScenarioResult[] = [
  runImportScenario(
    'initial_import_skips_invalid_and_defaults_allow_agents_false',
    [],
    [
      { origin: 'https://alpha.example.test/login', username: 'alice', secretToken: 'syn-1' },
      { origin: 'http://alpha.example.test/login', username: 'alice', secretToken: 'syn-1' },
      { origin: 'https://beta.example.test', username: '', secretToken: 'syn-2' },
      { origin: 'https://empty-pw.example.test', username: 'bob', secretToken: '' },
      { origin: 'https://user-bound-ok.example.test', username: '', secretToken: 'syn-3', repeatUtf16Username: 2000 },
      { origin: 'https://user-too-long.example.test', username: '', secretToken: 'syn-3', repeatUtf16Username: 2001 },
      { origin: 'https://pw-bound-ok.example.test', username: 'carol', secretToken: '', repeatUtf16Secret: 16384 },
      { origin: 'https://pw-too-long.example.test', username: 'dave', secretToken: '', repeatUtf16Secret: 16385 },
    ],
  ),
  runImportScenario(
    'unchanged_preserves_allow_agents_while_changed_secret_resets_allow_agents_false',
    [
      {
        id: 'seed-keep',
        origin: 'https://keep.example.test',
        username: 'alice',
        secretToken: 'syn-same',
        allowAgents: true,
      },
      {
        id: 'seed-rotate',
        origin: 'https://rotate.example.test',
        username: 'bob',
        secretToken: 'syn-old',
        allowAgents: true,
      },
    ],
    [
      { origin: 'https://keep.example.test/signin', username: 'alice', secretToken: 'syn-same' },
      { origin: 'https://rotate.example.test/auth', username: 'bob', secretToken: 'syn-new' },
    ],
  ),
  runImportScenario(
    'intra_batch_roundtrip_a_to_b_to_a_still_keeps_allow_agents_revoked',
    [
      {
        id: 'seed-aba',
        origin: 'https://aba.example.test',
        username: 'erin',
        secretToken: 'syn-a',
        allowAgents: true,
      },
    ],
    [
      { origin: 'https://aba.example.test', username: 'erin', secretToken: 'syn-b' },
      { origin: 'https://aba.example.test', username: 'erin', secretToken: 'syn-b' },
      { origin: 'https://aba.example.test', username: 'erin', secretToken: 'syn-a' },
      { origin: 'https://fresh.example.test', username: 'frank', secretToken: 'syn-x' },
      { origin: 'https://fresh.example.test', username: 'frank', secretToken: 'syn-x' },
      { origin: 'https://fresh.example.test', username: 'frank', secretToken: 'syn-y' },
    ],
  ),
];

// 4. Local import file path vectors (`isAllowedCookieFilePath`)
const pathVectors: PathVector[] = (
  [
    ['C:\\Users\\fixture\\Downloads\\passwords.csv', 'win32'],
    ['D:/exports/passwords.csv', 'win32'],
    ['\\\\server\\share\\passwords.csv', 'win32'],
    ['//server/share/passwords.csv', 'win32'],
    ['\\\\.\\COM1', 'win32'],
    ['\\\\?\\C:\\exports\\passwords.csv', 'win32'],
    ['C:\\exports\\..\\secret.csv', 'win32'],
    ['exports\\passwords.csv', 'win32'],
    ['C:\\exports\\bad\0name.csv', 'win32'],
    ['/home/fixture/Downloads/passwords.csv', 'linux'],
    ['//server/share/passwords.csv', 'linux'],
    ['/home/fixture/../passwords.csv', 'linux'],
    ['relative/passwords.csv', 'linux'],
  ] as const
).map(([p, platform]) => ({
  path: p,
  platform,
  allowed: refIsAllowedCookieFilePath(p, platform),
}));

// 5. Live Share & Page Credential Boundary vectors (async evaluation against registerPasswordManager)
async function buildPageBoundaryVectors(): Promise<PageBoundaryVector[]> {
  const cases: Array<Omit<PageBoundaryVector, 'expectedOk' | 'expectedCode' | 'expectedAccountIds'>> = [
    {
      name: 'untrusted_sender_blocked_even_offline',
      op: 'vault_list',
      trustedSender: false,
      joinedAsGuest: false,
      hostSharingProjects: null,
    },
    {
      name: 'joined_guest_blocks_vault_list',
      op: 'vault_list',
      trustedSender: true,
      joinedAsGuest: true,
      hostSharingProjects: null,
    },
    {
      name: 'joined_guest_blocks_vault_change',
      op: 'vault_change',
      trustedSender: true,
      joinedAsGuest: true,
      hostSharingProjects: null,
    },
    {
      name: 'joined_guest_blocks_agent_login_list',
      op: 'agent_list',
      trustedSender: true,
      joinedAsGuest: true,
      hostSharingProjects: null,
      page: { id: 1, project: 'ws-local', partition: 'owner', url: 'https://allowed.example.test/login' },
    },
    {
      name: 'host_sharing_keeps_local_vault_list_enabled',
      op: 'vault_list',
      trustedSender: true,
      joinedAsGuest: false,
      hostSharingProjects: ['ws-shared'],
    },
    {
      name: 'host_sharing_refuses_user_fill_on_shared_project_main_tab',
      op: 'user_fill',
      trustedSender: true,
      joinedAsGuest: false,
      hostSharingProjects: ['ws-shared'],
      page: { id: 10, project: 'ws-shared', partition: 'owner', url: 'https://allowed.example.test/login' },
      credentialId: 'cred-allowed',
    },
    {
      name: 'host_sharing_allows_user_fill_on_private_unshared_project',
      op: 'user_fill',
      trustedSender: true,
      joinedAsGuest: false,
      hostSharingProjects: ['ws-shared'],
      page: { id: 11, project: 'ws-private', partition: 'owner', url: 'https://private.example.test/login' },
      credentialId: 'cred-denied-agents',
    },
    {
      name: 'host_sharing_allows_side_browser_tab_when_not_opted_in',
      op: 'agent_fill',
      trustedSender: true,
      joinedAsGuest: false,
      hostSharingProjects: ['ws-shared'],
      page: {
        id: 12,
        project: 'ws-shared',
        side: true,
        optIn: false,
        partition: 'agent',
        url: 'https://allowed.example.test/login',
      },
      credentialId: 'cred-allowed',
    },
    {
      name: 'host_sharing_refuses_side_browser_tab_when_opted_in',
      op: 'agent_fill',
      trustedSender: true,
      joinedAsGuest: false,
      hostSharingProjects: ['ws-shared'],
      page: {
        id: 13,
        project: 'ws-shared',
        side: true,
        optIn: true,
        partition: 'agent',
        url: 'https://allowed.example.test/login',
      },
      credentialId: 'cred-allowed',
    },
    {
      name: 'refuses_guest_ephemeral_partition_even_when_not_sharing',
      op: 'user_fill',
      trustedSender: true,
      joinedAsGuest: false,
      hostSharingProjects: null,
      page: { id: 14, project: 'ws-private', partition: 'guest', url: 'https://allowed.example.test/login' },
      credentialId: 'cred-allowed',
    },
    {
      name: 'refuses_view_preview_partition',
      op: 'user_fill',
      trustedSender: true,
      joinedAsGuest: false,
      hostSharingProjects: null,
      page: { id: 15, project: 'ws-private', partition: 'view', url: 'https://allowed.example.test/login' },
      credentialId: 'cred-allowed',
    },
    {
      name: 'refuses_destroyed_page',
      op: 'user_fill',
      trustedSender: true,
      joinedAsGuest: false,
      hostSharingProjects: null,
      page: {
        id: 16,
        project: 'ws-private',
        destroyed: true,
        partition: 'owner',
        url: 'https://allowed.example.test/login',
      },
      credentialId: 'cred-allowed',
    },
    {
      name: 'refuses_http_page_with_https_required',
      op: 'agent_list',
      trustedSender: true,
      joinedAsGuest: false,
      hostSharingProjects: null,
      page: { id: 17, project: 'ws-private', partition: 'agent', url: 'http://allowed.example.test/login' },
    },
    {
      name: 'agent_list_returns_only_allow_agents_entries_for_exact_origin',
      op: 'agent_list',
      trustedSender: true,
      joinedAsGuest: false,
      hostSharingProjects: null,
      page: { id: 18, project: 'ws-private', partition: 'agent', url: 'https://allowed.example.test/path' },
    },
    {
      name: 'agent_fill_refuses_entry_with_allow_agents_false',
      op: 'agent_fill',
      trustedSender: true,
      joinedAsGuest: false,
      hostSharingProjects: null,
      page: { id: 19, project: 'ws-private', partition: 'agent', url: 'https://allowed.example.test/path' },
      credentialId: 'cred-same-site-disallowed',
    },
    {
      name: 'csv_import_rejects_unc_path_with_refused',
      op: 'import_csv_path',
      trustedSender: true,
      joinedAsGuest: false,
      hostSharingProjects: null,
      csvPath: '\\\\server\\share\\export.csv',
    },
  ];

  const out: PageBoundaryVector[] = [];
  for (const c of cases) {
    memFsState.files.clear();
    const seed = [
      {
        id: 'cred-allowed',
        origin: 'https://allowed.example.test',
        username: 'agent-ok',
        password: 'syn-password-1',
        allowAgents: true,
      },
      {
        id: 'cred-same-site-disallowed',
        origin: 'https://allowed.example.test',
        username: 'human-only',
        password: 'syn-password-2',
        allowAgents: false,
      },
      {
        id: 'cred-denied-agents',
        origin: 'https://private.example.test',
        username: 'owner-user',
        password: 'syn-password-3',
        allowAgents: false,
      },
    ];
    memFsState.files.set(
      '/data/browser-passwords.sealed',
      memSafeStorage.encryptString(JSON.stringify(seed)),
    );

    const handlers: Record<string, (...args: unknown[]) => Promise<unknown>> = {};
    const ownerSender = {};
    const foreignSender = {};
    let pickedCsv = c.csvPath ?? '';

    const sessionOwner = {};
    const sessionAgent = {};
    const sessionGuest = {};
    const sessionView = {};
    const sessionOther = {};
    const partitionToSession = (p: NonNullable<PageBoundaryVector['page']>['partition']) =>
      p === 'owner'
        ? sessionOwner
        : p === 'agent'
          ? sessionAgent
          : p === 'guest'
            ? sessionGuest
            : p === 'view'
              ? sessionView
              : sessionOther;

    const guestObj = c.page
      ? {
          id: c.page.id,
          session: partitionToSession(c.page.partition),
          isDestroyed: () => c.page?.destroyed === true,
          getURL: () => c.page?.url ?? 'about:blank',
          executeJavaScriptInIsolatedWorld: async () => ({ ok: true, submitted: false }),
        }
      : null;

    const credentialsGuestSession = () => c.joinedAsGuest;
    const credentialsPageShared = (id: number) =>
      !!c.hostSharingProjects &&
      !!c.page &&
      c.page.id === id &&
      c.hostSharingProjects.includes(c.page.project) &&
      (c.page.side !== true || c.page.optIn === true);

    const privateBrowserPage = (g: typeof guestObj) =>
      !!g &&
      !credentialsGuestSession() &&
      !credentialsPageShared(g.id) &&
      !g.isDestroyed() &&
      (g.session === sessionOwner || g.session === sessionAgent);

    const manager = refRegisterPasswordManager({
      ipcMain: {
        handle: (name, fn) => {
          handlers[name] = fn;
        },
      },
      safeStorage: memSafeStorage,
      dialog: {
        showOpenDialog: async () => ({
          canceled: !pickedCsv,
          filePaths: pickedCsv ? [pickedCsv] : [],
        }),
      },
      dataDir: '/data',
      getWindow: () => ({ isDestroyed: () => false, webContents: ownerSender }),
      guestFor: () => guestObj,
      allowedPage: (g) => privateBrowserPage(g as typeof guestObj),
      guestSessionActive: credentialsGuestSession,
    });

    const event = { sender: c.trustedSender ? ownerSender : foreignSender };
    let res: { ok: boolean; code?: string; accounts?: VaultEntryMeta[] };
    if (c.op === 'vault_list') {
      res = (await handlers['browser:passwordList'](event)) as typeof res;
    } else if (c.op === 'vault_change') {
      res = (await handlers['browser:passwordChange'](event, {
        id: 'cred-allowed',
        action: 'allow',
        allowAgents: false,
      })) as typeof res;
    } else if (c.op === 'import_csv_path') {
      res = (await handlers['browser:passwordImportCsv'](event)) as typeof res;
    } else if (c.op === 'user_fill') {
      res = (await handlers['browser:passwordFill'](
        event,
        c.page?.id ?? null,
        c.credentialId ?? '',
      )) as typeof res;
    } else if (c.op === 'agent_list') {
      res = await manager.login(guestObj);
    } else {
      res = await manager.login(guestObj, c.credentialId);
    }

    out.push({
      ...c,
      expectedOk: res.ok === true,
      ...(res.code ? { expectedCode: res.code } : {}),
      ...(res.accounts ? { expectedAccountIds: res.accounts.map((a) => a.id) } : {}),
    });
  }
  return out;
}

// 6. DOM form evaluation vectors against `passwordFillScript`
function runDomFormVector(spec: Omit<DomFormVector, 'expectedOk' | 'expectedCode' | 'filledUsernameIndex' | 'filledPasswordIndex'>): DomFormVector {
  const filledValues = new Map<number, string>();
  const locationState = {
    origin: spec.locationOrigin,
    protocol: spec.locationProtocol,
    href: spec.locationHref,
  };
  interface FakeFormObj {
    action: string;
    isConnected: boolean;
    ownerDocument: unknown;
    getAttribute: (name: string) => string | null;
    querySelectorAll: (sel: string) => unknown[];
  }
  const docObj: {
    baseURI: string;
    querySelectorAll: (sel: string) => unknown[];
  } = {
    baseURI: spec.baseUri ?? spec.locationHref,
    querySelectorAll: () => [],
  };
  const formObj: FakeFormObj | null =
    spec.formAction === null
      ? null
      : {
          action: spec.formAction,
          isConnected: spec.formConnected !== false,
          ownerDocument: spec.formInSameDocument === false ? {} : docObj,
          getAttribute: (name: string) => (name === 'action' ? spec.formAction : null),
          querySelectorAll: () => [],
        };
  const badReparentForm: FakeFormObj = {
    action: 'https://attacker.example.test/collect',
    isConnected: true,
    ownerDocument: docObj,
    getAttribute: (name: string) =>
      name === 'action' ? 'https://attacker.example.test/collect' : null,
    querySelectorAll: () => [],
  };
  const submitters = (spec.submitFormActions ?? []).map((fa) => ({
    form: formObj,
    getAttribute: (name: string) => (name === 'formaction' ? fa : null),
  }));

  class FakeInput {
    idx: number;
    type: string;
    autocomplete: string;
    disabled: boolean;
    readOnly: boolean;
    isConnected: boolean;
    inFormSubtree: boolean;
    form: FakeFormObj | null;
    private _visible: boolean;
    private _val = '';

    constructor(idx: number, f: DomFieldSpec) {
      this.idx = idx;
      this.type = f.type;
      this.autocomplete = f.autocomplete ?? '';
      this.disabled = f.disabled === true;
      this.readOnly = f.readOnly === true;
      this.isConnected = true;
      this._visible = f.visible !== false;
      this.inFormSubtree = f.inForm !== false && formObj !== null;
      this.form =
        f.inForm === false ? null : f.ownedByOtherForm === true ? badReparentForm : formObj;
    }
    getClientRects() {
      return this._visible ? [{}] : [];
    }
    dispatchEvent(ev: { type: string }) {
      if (
        this.type !== 'password' &&
        ev.type === 'input' &&
        spec.reparentPasswordFormDuringUsernameFill
      ) {
        for (const el of inputs) {
          if (el.type === 'password') el.form = badReparentForm;
        }
      }
      return true;
    }
    get value() {
      return this._val;
    }
    set value(v: string) {
      this._val = v;
      filledValues.set(this.idx, v);
      if (this.type !== 'password' && spec.mutateOriginDuringUsernameFill) {
        locationState.origin = spec.mutateOriginDuringUsernameFill;
      }
    }
  }

  const inputs = spec.fields.map((f, i) => new FakeInput(i, f));
  if (formObj) {
    formObj.querySelectorAll = (sel: string) =>
      sel === 'input' ? inputs.filter((el) => el.inFormSubtree) : [];
  }
  docObj.querySelectorAll = (sel: string) => {
    if (sel === 'input[type="password"]') return inputs.filter((el) => el.type === 'password');
    if (sel === '[formaction]') return submitters;
    return [];
  };

  const domSandbox = vm.createContext({
    URL,
    Event: class FakeEvent {
      type: string;
      constructor(type: string) {
        this.type = type;
      }
    },
    location: locationState,
    getComputedStyle: (el: FakeInput) => ({
      visibility: el.getClientRects().length ? 'visible' : 'hidden',
    }),
    HTMLInputElement: FakeInput,
    document: docObj,
  });

  const code = refPasswordFillScript({
    origin: spec.entryOrigin,
    username: 'syn-user',
    password: 'syn-password',
  });
  const result = vm.runInContext(code, domSandbox) as {
    ok: boolean;
    code?: string;
    submitted?: boolean;
  };

  let filledUsernameIndex: number | null = null;
  let filledPasswordIndex: number | null = null;
  for (const [idx, val] of filledValues.entries()) {
    if (val === 'syn-user') filledUsernameIndex = idx;
    if (val === 'syn-password') filledPasswordIndex = idx;
  }

  return {
    ...spec,
    expectedOk: result.ok === true,
    ...(result.code ? { expectedCode: result.code } : {}),
    filledUsernameIndex,
    filledPasswordIndex,
  };
}

const domFormVectors: DomFormVector[] = [
  runDomFormVector({
    name: 'standard_email_and_password_form_succeeds_without_submit',
    entryOrigin: 'https://login.example.test',
    locationOrigin: 'https://login.example.test',
    locationProtocol: 'https:',
    locationHref: 'https://login.example.test/signin',
    formAction: 'https://login.example.test/session',
    fields: [{ type: 'email' }, { type: 'password' }],
  }),
  runDomFormVector({
    name: 'relative_empty_action_resolves_to_same_origin',
    entryOrigin: 'https://login.example.test',
    locationOrigin: 'https://login.example.test',
    locationProtocol: 'https:',
    locationHref: 'https://login.example.test/signin',
    formAction: '',
    fields: [{ type: 'text' }, { type: 'password' }],
  }),
  runDomFormVector({
    name: 'password_only_step_succeeds_with_no_username_input',
    entryOrigin: 'https://login.example.test',
    locationOrigin: 'https://login.example.test',
    locationProtocol: 'https:',
    locationHref: 'https://login.example.test/step2',
    formAction: 'https://login.example.test/step2',
    fields: [{ type: 'password' }],
  }),
  runDomFormVector({
    name: 'disambiguates_multiple_text_inputs_when_one_has_autocomplete_username',
    entryOrigin: 'https://login.example.test',
    locationOrigin: 'https://login.example.test',
    locationProtocol: 'https:',
    locationHref: 'https://login.example.test/signin',
    formAction: 'https://login.example.test/session',
    fields: [
      { type: 'text' },
      { type: 'text', autocomplete: 'username' },
      { type: 'password' },
    ],
  }),
  runDomFormVector({
    name: 'wrong_origin_rejected',
    entryOrigin: 'https://other.example.test',
    locationOrigin: 'https://login.example.test',
    locationProtocol: 'https:',
    locationHref: 'https://login.example.test/signin',
    formAction: 'https://login.example.test/session',
    fields: [{ type: 'email' }, { type: 'password' }],
  }),
  runDomFormVector({
    name: 'new_password_registration_field_ignored_yielding_no_login_form',
    entryOrigin: 'https://login.example.test',
    locationOrigin: 'https://login.example.test',
    locationProtocol: 'https:',
    locationHref: 'https://login.example.test/signup',
    formAction: 'https://login.example.test/signup',
    fields: [{ type: 'email' }, { type: 'password', autocomplete: 'new-password' }],
  }),
  runDomFormVector({
    name: 'two_visible_current_password_fields_rejected_no_login_form',
    entryOrigin: 'https://login.example.test',
    locationOrigin: 'https://login.example.test',
    locationProtocol: 'https:',
    locationHref: 'https://login.example.test/change',
    formAction: 'https://login.example.test/change',
    fields: [{ type: 'password' }, { type: 'password' }],
  }),
  runDomFormVector({
    name: 'cross_origin_form_action_rejected_unsafe_form',
    entryOrigin: 'https://login.example.test',
    locationOrigin: 'https://login.example.test',
    locationProtocol: 'https:',
    locationHref: 'https://login.example.test/signin',
    formAction: 'https://attacker.example.test/collect',
    fields: [{ type: 'email' }, { type: 'password' }],
  }),
  runDomFormVector({
    name: 'javascript_form_action_rejected_unsafe_form',
    entryOrigin: 'https://login.example.test',
    locationOrigin: 'https://login.example.test',
    locationProtocol: 'https:',
    locationHref: 'https://login.example.test/signin',
    formAction: 'javascript:void(0)',
    fields: [{ type: 'email' }, { type: 'password' }],
  }),
  runDomFormVector({
    name: 'two_plain_text_fields_rejected_ambiguous_form',
    entryOrigin: 'https://login.example.test',
    locationOrigin: 'https://login.example.test',
    locationProtocol: 'https:',
    locationHref: 'https://login.example.test/signin',
    formAction: 'https://login.example.test/session',
    fields: [{ type: 'text' }, { type: 'text' }, { type: 'password' }],
  }),
  runDomFormVector({
    name: 'navigation_race_during_username_fill_aborts_before_password_fill',
    entryOrigin: 'https://login.example.test',
    locationOrigin: 'https://login.example.test',
    locationProtocol: 'https:',
    locationHref: 'https://login.example.test/signin',
    formAction: 'https://login.example.test/session',
    mutateOriginDuringUsernameFill: 'https://evil.example.test',
    fields: [{ type: 'email' }, { type: 'password' }],
  }),
  runDomFormVector({
    name: 'base_href_hijacks_relative_action_rejected_unsafe_form',
    entryOrigin: 'https://login.example.test',
    locationOrigin: 'https://login.example.test',
    locationProtocol: 'https:',
    locationHref: 'https://login.example.test/signin',
    baseUri: 'https://evil.example.test/phish/',
    formAction: 'session',
    fields: [{ type: 'email' }, { type: 'password' }],
  }),
  runDomFormVector({
    name: 'base_href_does_not_affect_empty_action_resolving_to_document_url',
    entryOrigin: 'https://login.example.test',
    locationOrigin: 'https://login.example.test',
    locationProtocol: 'https:',
    locationHref: 'https://login.example.test/signin',
    baseUri: 'https://evil.example.test/phish/',
    formAction: '',
    fields: [{ type: 'email' }, { type: 'password' }],
  }),
  runDomFormVector({
    name: 'submit_button_cross_origin_formaction_rejected_unsafe_form',
    entryOrigin: 'https://login.example.test',
    locationOrigin: 'https://login.example.test',
    locationProtocol: 'https:',
    locationHref: 'https://login.example.test/signin',
    formAction: 'https://login.example.test/session',
    submitFormActions: ['https://other.example.test/collect'],
    fields: [{ type: 'email' }, { type: 'password' }],
  }),
  runDomFormVector({
    name: 'reparent_password_to_cross_origin_form_during_username_input_aborts',
    entryOrigin: 'https://login.example.test',
    locationOrigin: 'https://login.example.test',
    locationProtocol: 'https:',
    locationHref: 'https://login.example.test/signin',
    formAction: 'https://login.example.test/session',
    reparentPasswordFormDuringUsernameFill: true,
    fields: [{ type: 'email' }, { type: 'password' }],
  }),
  runDomFormVector({
    name: 'disconnected_form_rejected_unsafe_form',
    entryOrigin: 'https://login.example.test',
    locationOrigin: 'https://login.example.test',
    locationProtocol: 'https:',
    locationHref: 'https://login.example.test/signin',
    formAction: 'https://login.example.test/session',
    formConnected: false,
    fields: [{ type: 'email' }, { type: 'password' }],
  }),
  runDomFormVector({
    name: 'foreign_owner_document_form_rejected_unsafe_form',
    entryOrigin: 'https://login.example.test',
    locationOrigin: 'https://login.example.test',
    locationProtocol: 'https:',
    locationHref: 'https://login.example.test/signin',
    formAction: 'https://login.example.test/session',
    formInSameDocument: false,
    fields: [{ type: 'email' }, { type: 'password' }],
  }),
  runDomFormVector({
    name: 'username_inside_form_owned_by_another_form_ignored_fills_password_only',
    entryOrigin: 'https://login.example.test',
    locationOrigin: 'https://login.example.test',
    locationProtocol: 'https:',
    locationHref: 'https://login.example.test/signin',
    formAction: 'https://login.example.test/session',
    fields: [{ type: 'email', ownedByOtherForm: true }, { type: 'password' }],
  }),
];

// 7. Sanitize DOM result vectors via `fillSavedPassword`
async function buildSanitizeVectors(): Promise<SanitizeResultVector[]> {
  memFsState.files.clear();
  memFsState.files.set(
    '/mem/sanitize.sealed',
    memSafeStorage.encryptString(
      JSON.stringify([
        {
          id: 'id-1',
          origin: 'https://example.test',
          username: 'u',
          password: 'syn-secret',
          allowAgents: true,
        },
      ]),
    ),
  );
  const vault = refCreatePasswordVault({ file: '/mem/sanitize.sealed', safeStorage: memSafeStorage });
  const rawOutputs: Array<{ name: string; raw: unknown }> = [
    { name: 'ok_true', raw: { ok: true, submitted: true, extra: 'dropped' } },
    { name: 'allowlisted_wrong_site', raw: { ok: false, code: 'wrong-site' } },
    { name: 'allowlisted_no_login_form', raw: { ok: false, code: 'no-login-form' } },
    { name: 'allowlisted_unsafe_form', raw: { ok: false, code: 'unsafe-form' } },
    { name: 'allowlisted_ambiguous_form', raw: { ok: false, code: 'ambiguous-form' } },
    { name: 'untrusted_page_code_redacted_to_fill_failed', raw: { ok: false, code: 'leaked-page-data' } },
    { name: 'null_return_redacted_to_fill_failed', raw: null },
  ];
  const out: SanitizeResultVector[] = [];
  for (const item of rawOutputs) {
    const res = await refFillSavedPassword({
      vault,
      guest: {
        getURL: () => 'https://example.test/login',
        executeJavaScriptInIsolatedWorld: async () => item.raw,
      },
      id: 'id-1',
      allowed: () => true,
    });
    out.push({
      name: item.name,
      raw: item.raw,
      expectedOk: res.ok === true,
      ...(res.code ? { expectedCode: res.code } : {}),
    });
  }
  return out;
}

// 8. Reference UI computations matching `BrowserPasswords.vue`
const PAGE = 200;
const CSV_FIX = ['access-denied', 'locked', 'unreadable', 'no-sqlite', 'primary-password'];

function refErrorText(lang: 'en' | 'fr', code?: string): string {
  const t = (k: string, fb: string, v?: Record<string, string | number>) =>
    translate(lang, k, fb, v);
  const map: Record<string, string> = {
    'private-page-required': t(
      'browser.passwords.private',
      'Use a private browser tab on the host. Credentials cannot be used in shared tabs or joined Live Share sessions.',
    ),
    'encryption-unavailable': t(
      'browser.passwords.encryption',
      'Secure system encryption is unavailable. No passwords were saved.',
    ),
    'vault-unavailable': t('browser.passwords.vault', 'The encrypted vault could not be opened.'),
    locked: t(
      'browser.passwords.locked',
      'The browser database is locked. Close that browser and retry, or import its CSV export.',
    ),
    'access-denied': t(
      'browser.passwords.denied',
      'Browser protection or permissions prevent access. Use a CSV export.',
    ),
    'no-sqlite': t(
      'browser.passwords.sqlite',
      'This version cannot read browser databases. Update Tessel or import a CSV export.',
    ),
    unreadable: t(
      'browser.passwords.unreadable',
      'The browser database could not be read. Import a CSV export instead.',
    ),
    missing: t(
      'browser.passwords.missing',
      'The browser database no longer exists. Reopen this dialog and choose a profile.',
    ),
    'not-found': t(
      'browser.passwords.missing',
      'The browser database no longer exists. Reopen this dialog and choose a profile.',
    ),
    'primary-password': t(
      'browser.passwords.primary',
      'This Firefox profile has a Primary Password set, so its saved passwords cannot be read here. Unlock it in Firefox, export its passwords as CSV, then import the CSV.',
    ),
    format: t(
      'browser.passwords.format',
      'Expected a CSV export with url, username and password columns.',
    ),
    'too-big': t('browser.passwords.tooBig', 'Too many passwords: Tessel keeps up to 5000.'),
    'no-login-form': t(
      'browser.passwords.form',
      'No single visible password field was found. Open the login form first.',
    ),
    'unsafe-form': t(
      'browser.passwords.unsafe',
      'This form sends credentials to another site or cannot be verified.',
    ),
    'ambiguous-form': t(
      'browser.passwords.ambiguous',
      'Several possible username fields were found. Fill this form manually.',
    ),
    'https-required': t('browser.passwords.https', 'Open the HTTPS login page first.'),
  };
  return (code && map[code]) || t('browser.passwords.failed', 'The operation could not be completed.');
}

function refNeedsCsv(r?: SummaryVector['result']): boolean {
  return (
    (r?.protectedCount ?? 0) > 0 ||
    (r?.appBoundCount ?? 0) > 0 ||
    (r?.code !== undefined && CSV_FIX.includes(r.code))
  );
}

function refSummary(lang: 'en' | 'fr', r?: SummaryVector['result']): string {
  const t = (k: string, fb: string, v?: Record<string, string | number>) =>
    translate(lang, k, fb, v);
  if (!r?.ok) return refErrorText(lang, r?.code);
  const parts: string[] = [];
  if (r.added != null) {
    parts.push(t('browser.passwords.added', '{{count}} new', { count: r.added }));
    if ((r.updated ?? 0) > 0) {
      parts.push(
        t('browser.passwords.changedCount', '{{count}} updated (agent access turned off again)', {
          count: r.updated ?? 0,
        }),
      );
    }
    if ((r.unchanged ?? 0) > 0) {
      parts.push(
        t('browser.passwords.unchanged', '{{count}} already saved', { count: r.unchanged ?? 0 }),
      );
    }
  } else {
    parts.push(
      t('browser.passwords.importedCount', '{{count}} imported', { count: r.imported || 0 }),
    );
  }
  if ((r.skipped ?? 0) > 0) {
    parts.push(
      t(
        'browser.passwords.skipped',
        '{{count}} skipped (not an HTTPS site, no password or unreadable)',
        { count: r.skipped ?? 0 },
      ),
    );
  }
  if ((r.appBoundCount ?? 0) > 0) {
    parts.push(
      t(
        'browser.passwords.appBoundShort',
        '{{count}} locked by App-Bound Encryption (Chrome/Edge 127+): import a CSV export',
        { count: r.appBoundCount ?? 0 },
      ),
    );
  }
  if ((r.protectedCount ?? 0) > 0) {
    parts.push(
      t(
        'browser.passwords.protectedShort',
        '{{count}} protected and unreadable: import a CSV export',
        { count: r.protectedCount ?? 0 },
      ),
    );
  }
  return parts.join(' · ');
}

function refExportSteps(
  lang: 'en' | 'fr',
  browserId?: string | null,
  family?: string | null,
): string {
  const t = (k: string, fb: string) => translate(lang, k, fb);
  if (family === 'firefox') {
    return t(
      'browser.passwords.stepsFirefox',
      'In Firefox (and Zen, LibreWolf, Waterfox, Floorp): menu ≡ → Passwords → the ⋯ menu at the top right → Export passwords. Confirm with your device, then save the CSV.',
    );
  }
  if (
    browserId === 'chrome' ||
    browserId === 'chrome-beta' ||
    browserId === 'chrome-dev' ||
    browserId === 'chrome-canary' ||
    browserId === 'chromium'
  ) {
    return t(
      'browser.passwords.stepsChrome',
      'In Chrome: menu ⋮ → Passwords and autofill → Google Password Manager → Settings. Under Export passwords, choose Download file.',
    );
  }
  if (browserId === 'edge') {
    return t(
      'browser.passwords.stepsEdge',
      'In Edge: menu ⋯ → Settings → Profiles → Passwords → the ⋯ (More actions) → Export passwords → Export.',
    );
  }
  if (browserId === 'vivaldi') {
    return t(
      'browser.passwords.stepsVivaldi',
      'In Vivaldi: Vivaldi menu → File → Export → Export Passwords.',
    );
  }
  if (browserId === 'opera' || browserId === 'opera-gx') {
    return t(
      'browser.passwords.stepsOpera',
      'In Opera / Opera GX: Settings → Privacy & Security → Password Manager → Settings. Under Export passwords, choose Download file.',
    );
  }
  if (browserId === 'brave') {
    return t(
      'browser.passwords.stepsBrave',
      'In Brave, enter brave://password-manager/settings in the address bar. Find Export passwords and download the CSV file.',
    );
  }
  return t(
    'browser.passwords.stepsOther',
    'In Safari (macOS): menu bar → File → Export → Passwords, or Settings → Passwords → ⋯ → Export All Passwords. For any other browser, open its password manager and choose Export passwords (CSV).',
  );
}

function computeUiView(
  name: string,
  entries: VaultEntryMeta[],
  query: string,
  pagesShown: number,
  lang: 'en' | 'fr',
): UiViewVector {
  const t = (k: string, fb: string, v?: Record<string, string | number>) =>
    translate(lang, k, fb, v);
  const rows = entries
    .map((e) => {
      const site = String(e.origin || '').replace(/^https:\/\//, '');
      const username = String(e.username || '');
      return {
        id: e.id,
        origin: e.origin,
        site,
        username,
        allowAgents: e.allowAgents === true,
        key: `${site}\n${username}`.toLowerCase(),
      };
    })
    .sort((a, b) =>
      a.site < b.site
        ? -1
        : a.site > b.site
          ? 1
          : a.username < b.username
            ? -1
            : a.username > b.username
              ? 1
              : 0,
    );
  const terms = query
    .trim()
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean);
  const filtered = terms.length
    ? rows.filter((r) => terms.every((w) => r.key.includes(w)))
    : rows;
  const limit = PAGE * pagesShown;
  const visible = filtered.slice(0, limit).map((r, i, all) => ({
    id: r.id,
    origin: r.origin,
    site: r.site,
    username: r.username,
    allowAgents: r.allowAgents,
    same: i > 0 && all[i - 1].site === r.site,
  }));
  const remaining = filtered.length - visible.length;
  const allowedCount = rows.filter((r) => r.allowAgents).length;
  const allShownAllowed = filtered.length > 0 && filtered.every((r) => r.allowAgents);
  const someShownAllowed = filtered.some((r) => r.allowAgents);

  return {
    name,
    entries: entries.length > 20 ? [] : entries,
    query,
    pagesShown,
    lang,
    expectedTotalRows: rows.length,
    expectedFilteredCount: filtered.length,
    expectedVisibleRows: visible.length > 25 ? visible.slice(0, 5) : visible,
    expectedRemaining: remaining,
    expectedAllowedCount: allowedCount,
    expectedCountLabel: terms.length
      ? t('browser.passwords.shown', '{{shown}} of {{total}}', {
          shown: filtered.length,
          total: rows.length,
        })
      : t('browser.passwords.total', '{{count}} accounts', { count: rows.length }),
    expectedAgentsLabel: t('browser.passwords.agentsCount', '{{count}} allowed for agents', {
      count: allowedCount,
    }),
    expectedNoMatchLabel: t('browser.passwords.noMatch', 'No account matches “{{query}}”.', {
      query: query.trim(),
    }),
    expectedMoreLabel: t('browser.passwords.more', 'Show more ({{count}} left)', {
      count: remaining,
    }),
    expectedBulkChecked: allShownAllowed,
    expectedBulkIndeterminate: someShownAllowed && !allShownAllowed,
    expectedBulkDisabled: filtered.length === 0,
    expectedBulkLabel: allShownAllowed
      ? t('browser.passwords.forbidAll', 'Forbid my local agents every account shown ({{count}})', {
          count: filtered.length,
        })
      : t('browser.passwords.allowAll', 'Allow my local agents every account shown ({{count}})', {
          count: filtered.length,
        }),
  };
}

const sampleEntries: VaultEntryMeta[] = [
  { id: 'id-1', origin: 'https://mail.example.test', username: 'alice', allowAgents: false },
  { id: 'id-2', origin: 'https://shop.test', username: 'Bob@Example.test', allowAgents: false },
  { id: 'id-3', origin: 'https://bank.test', username: 'carol', allowAgents: true },
  { id: 'id-4', origin: 'https://bank.test', username: 'alice', allowAgents: false },
  // Surrogate pair U+1F511 ('🔑', UTF-16: 0xD83D 0xDD11) vs BMP U+E000 ('\uE000'):
  // In UTF-16 code units, 0xD83D < 0xE000, whereas in Unicode scalar values U+1F511 > U+E000!
  { id: 'id-5', origin: 'https://unicode.test', username: '\uE000bmp', allowAgents: false },
  { id: 'id-6', origin: 'https://unicode.test', username: '🔑astral', allowAgents: false },
];

const uiViewVectors: UiViewVector[] = [
  computeUiView('all_entries_en_with_same_site_and_utf16_sort', sampleEntries, '', 1, 'en'),
  computeUiView('all_entries_fr_plural', sampleEntries, '', 1, 'fr'),
  computeUiView('single_entry_fr_singular', [sampleEntries[2]], '', 1, 'fr'),
  computeUiView('empty_entries_fr_zero_is_singular_one', [], '', 1, 'fr'),
  computeUiView('multi_word_search_en', sampleEntries, '  EXAMPLE   bob ', 1, 'en'),
  computeUiView('no_match_search_fr', sampleEntries, 'introuvable', 1, 'fr'),
];

function computeBulkConfirm(
  name: string,
  entries: VaultEntryMeta[],
  query: string,
  targetAllow: boolean,
  canConfirm: boolean,
  lang: 'en' | 'fr',
): BulkConfirmVector {
  const t = (k: string, fb: string, v?: Record<string, string | number>) =>
    translate(lang, k, fb, v);
  const rows = entries
    .map((e) => {
      const site = String(e.origin || '').replace(/^https:\/\//, '');
      const username = String(e.username || '');
      return { ...e, site, username, key: `${site}\n${username}`.toLowerCase() };
    })
    .sort((a, b) =>
      a.site < b.site
        ? -1
        : a.site > b.site
          ? 1
          : a.username < b.username
            ? -1
            : a.username > b.username
              ? 1
              : 0,
    );
  const terms = query
    .trim()
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean);
  const filtered = terms.length
    ? rows.filter((r) => terms.every((w) => r.key.includes(w)))
    : rows;
  const targets = filtered.filter((r) => r.allowAgents !== targetAllow);
  if (!targets.length) {
    return { name, entries, query, targetAllow, canConfirm, lang, expectedKind: 'noop', expectedTargetIds: [] };
  }
  if (targetAllow) {
    if (!canConfirm) {
      return {
        name,
        entries,
        query,
        targetAllow,
        canConfirm,
        lang,
        expectedKind: 'refused_without_confirm',
        expectedTargetIds: [],
      };
    }
    return {
      name,
      entries,
      query,
      targetAllow,
      canConfirm,
      lang,
      expectedKind: 'ask_confirm',
      expectedTargetIds: targets.map((x) => x.id),
      expectedPrompt: {
        title: t(
          'browser.passwords.allowAllTitle',
          'Allow {{count}} accounts for your local agents?',
          { count: targets.length },
        ),
        text: t(
          'browser.passwords.allowAllText',
          'Agents running on this computer will be able to sign in with these logins in Tessel’s browser. You can turn them off again here, all at once or one by one.',
        ),
        confirmLabel: t('browser.passwords.allowAllConfirm', 'Allow {{count}}', {
          count: targets.length,
        }),
        danger: true,
      },
    };
  }
  return {
    name,
    entries,
    query,
    targetAllow,
    canConfirm,
    lang,
    expectedKind: 'apply_immediately',
    expectedTargetIds: targets.map((x) => x.id),
  };
}

const bulkConfirmVectors: BulkConfirmVector[] = [
  computeBulkConfirm(
    'mixed_allow_asks_only_for_currently_disallowed_count_en',
    [
      { id: 'id-1', origin: 'https://a.test', username: 'u1', allowAgents: false },
      { id: 'id-2', origin: 'https://b.test', username: 'u2', allowAgents: false },
      { id: 'id-3', origin: 'https://c.test', username: 'u3', allowAgents: true },
    ],
    '',
    true,
    true,
    'en',
  ),
  computeBulkConfirm(
    'mixed_allow_asks_fr',
    [
      { id: 'id-1', origin: 'https://a.test', username: 'u1', allowAgents: false },
      { id: 'id-2', origin: 'https://b.test', username: 'u2', allowAgents: true },
    ],
    '',
    true,
    true,
    'fr',
  ),
  computeBulkConfirm(
    'allow_without_confirmer_refused_fail_closed',
    [{ id: 'id-1', origin: 'https://a.test', username: 'u1', allowAgents: false }],
    '',
    true,
    false,
    'en',
  ),
  computeBulkConfirm(
    'forbid_all_applies_immediately_without_confirm',
    [
      { id: 'id-1', origin: 'https://a.test', username: 'u1', allowAgents: true },
      { id: 'id-2', origin: 'https://b.test', username: 'u2', allowAgents: false },
      { id: 'id-3', origin: 'https://c.test', username: 'u3', allowAgents: true },
    ],
    '',
    false,
    false,
    'fr',
  ),
];

const summaryVectors: SummaryVector[] = (
  [
    {
      name: 'mixed_counts_and_app_bound_en',
      lang: 'en',
      result: {
        ok: true,
        imported: 3,
        added: 2,
        updated: 1,
        unchanged: 4,
        skipped: 1,
        appBoundCount: 5,
        protectedCount: 0,
      },
    },
    {
      name: 'mixed_counts_singular_fr',
      lang: 'fr',
      result: {
        ok: true,
        imported: 2,
        added: 1,
        updated: 1,
        unchanged: 1,
        skipped: 1,
        appBoundCount: 1,
        protectedCount: 1,
      },
    },
    {
      name: 'mixed_counts_plural_fr',
      lang: 'fr',
      result: {
        ok: true,
        imported: 5,
        added: 3,
        updated: 2,
        unchanged: 2,
        skipped: 4,
        appBoundCount: 2,
        protectedCount: 3,
      },
    },
    {
      name: 'zero_added_fr',
      lang: 'fr',
      result: { ok: true, imported: 0, added: 0, updated: 0, unchanged: 0, skipped: 0 },
    },
    {
      name: 'primary_password_error_fr',
      lang: 'fr',
      result: { ok: false, code: 'primary-password' },
    },
    {
      name: 'private_page_error_en',
      lang: 'en',
      result: { ok: false, code: 'private-page-required' },
    },
  ] as const
).map((v) => ({
  ...v,
  expectedNeedsCsv: refNeedsCsv(v.result),
  expectedText: refSummary(v.lang, v.result),
}));

function computeBatchSummary(
  name: string,
  lang: 'en' | 'fr',
  profiles: BatchSummaryVector['profiles'],
): BatchSummaryVector {
  const t = (k: string, fb: string, v?: Record<string, string | number>) =>
    translate(lang, k, fb, v);
  let added = 0;
  let failed = 0;
  const items = profiles.map((p, i) => {
    const r = p.threw ? { ok: false, code: 'failed' } : p.result;
    if (r?.ok) added += r.added ?? r.imported ?? 0;
    const warn = !r?.ok || (r.appBoundCount ?? 0) > 0 || (r.protectedCount ?? 0) > 0;
    if (warn) failed++;
    return {
      index: i,
      label: p.label,
      ok: !!r?.ok,
      warn,
      csv: refNeedsCsv(r),
      text: refSummary(lang, r),
    };
  });
  const title = failed
    ? t(
        'browser.passwords.allDoneWarn',
        '{{profiles}} profiles read: {{added}} new accounts. {{count}} need attention.',
        { profiles: profiles.length, added, count: failed },
      )
    : t('browser.passwords.allDone', '{{profiles}} profiles read: {{added}} new accounts.', {
        profiles: profiles.length,
        added,
      });
  return { name, lang, profiles, expectedTitle: title, expectedItems: items };
}

const batchSummaryVectors: BatchSummaryVector[] = [
  computeBatchSummary('four_profiles_with_three_warnings_en', 'en', [
    {
      label: 'Chrome — Default',
      result: {
        ok: true,
        imported: 3,
        added: 2,
        updated: 1,
        unchanged: 4,
        skipped: 1,
        protectedCount: 0,
        appBoundCount: 5,
      },
    },
    { label: 'Chrome — Work', threw: true },
    { label: 'Firefox — default', result: { ok: false, code: 'primary-password' } },
    {
      label: 'Edge — Default',
      result: {
        ok: true,
        imported: 7,
        added: 7,
        updated: 0,
        unchanged: 0,
        skipped: 0,
        protectedCount: 0,
        appBoundCount: 0,
      },
    },
  ]),
  computeBatchSummary('single_warning_fr_singular', 'fr', [
    { label: 'Firefox — default', result: { ok: false, code: 'primary-password' } },
    { label: 'Edge — Default', result: { ok: true, added: 4 } },
  ]),
  computeBatchSummary('clean_success_fr', 'fr', [
    { label: 'Chrome — Default', result: { ok: true, added: 3 } },
    { label: 'Brave — Default', result: { ok: true, added: 2 } },
  ]),
];

const exportStepSources = [
  { browserId: 'firefox', family: 'firefox' },
  { browserId: 'zen', family: 'firefox' },
  { browserId: 'chrome', family: 'chromium' },
  { browserId: 'chrome-canary', family: 'chromium' },
  { browserId: 'chromium', family: 'chromium' },
  { browserId: 'edge', family: 'chromium' },
  { browserId: 'vivaldi', family: 'chromium' },
  { browserId: 'opera', family: 'chromium' },
  { browserId: 'opera-gx', family: 'chromium' },
  { browserId: 'brave', family: 'chromium' },
  { browserId: 'arc', family: 'chromium' },
  { browserId: null, family: null },
];

const exportStepVectors = exportStepSources.flatMap((s) =>
  (['en', 'fr'] as const).map((lang) => ({
    browserId: s.browserId,
    family: s.family,
    lang,
    expected: refExportSteps(lang, s.browserId, s.family),
  })),
);

const allErrorCodes = [
  'private-page-required',
  'encryption-unavailable',
  'vault-unavailable',
  'locked',
  'access-denied',
  'no-sqlite',
  'unreadable',
  'missing',
  'not-found',
  'primary-password',
  'format',
  'too-big',
  'no-login-form',
  'unsafe-form',
  'ambiguous-form',
  'https-required',
  'fill-failed',
  'refused',
  'unknown-error',
];

const errorVectors = allErrorCodes.flatMap((code) =>
  (['en', 'fr'] as const).map((lang) => ({
    code,
    lang,
    expected: refErrorText(lang, code),
  })),
);

// Sanity check that BrowserPasswords.vue still holds the constants we mirror.
if (!browserPasswordsVue.includes('const PAGE = 200')) {
  throw new Error('BrowserPasswords.vue PAGE constant changed; re-audit pagination');
}

async function main(): Promise<void> {
  const pageBoundaryVectors = await buildPageBoundaryVectors();
  const sanitizeVectors = await buildSanitizeVectors();

  const payload = {
    sources,
    constants: {
      pageSize: PAGE,
      referenceMaxEntries: 5000,
      referenceMaxCsvBytes: 10 * 1024 * 1024,
      referenceMaxUsernameUtf16: 2000,
      referenceMaxPasswordUtf16: 16384,
    },
    originVectors,
    csvVectors,
    importScenarios,
    pathVectors,
    pageBoundaryVectors,
    domFormVectors,
    sanitizeVectors,
    uiViewVectors,
    bulkConfirmVectors,
    summaryVectors,
    batchSummaryVectors,
    exportStepVectors,
    errorVectors,
  };

  fs.mkdirSync(path.dirname(outPath), { recursive: true });
  fs.writeFileSync(outPath, `${JSON.stringify(payload, null, 2)}\n`, 'utf8');
}

void main();
