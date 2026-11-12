// Read-only reference extraction. Inputs are synthetic; no app or profile is opened.
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import crypto from 'node:crypto'
import { fileURLToPath } from 'node:url'

const root = path.resolve(process.argv[2] || 'C:/Tessel')
const file = path.join(root, 'src/main/liveShare/shareChat.js')
const source = fs.readFileSync(file, 'utf8')
const start = source.indexOf("const MASK =")
const end = source.indexOf("// The harness's own user turns")
if (start < 0 || end <= start) throw new Error('Reference boundaries changed')
const api = vm.runInNewContext(source.slice(start, end).replaceAll('export function ', 'function ') + '\n;({guestText, cleanRoots})', {}, {timeout: 1000})
const roots = [String.raw`C:\Proj\app`, '/home/ada/proj']
const inputs = [
  String.raw`open C:\Proj\app\src\a.js and C:\Users\private\key.pem`,
  String.raw`C:\Users\jean claude\secret\key.pem and more`,
  String.raw`open "C:\Users\jean claude\secret\key.pem" now`,
  'scp user@host:/home/jean/.ssh/id_rsa .',
  'ssh deploy@prod.example.com ls',
  'gcc -I/home/jean/include',
  String.raw`cl -IC:\Users\x\inc`,
  'configure --prefix=/opt/secret/x',
  '\x1b[31m/home/jean/secret/x\x1b[0m',
  String.raw`\\?\C:\Users\jean\x | \\?\UNC\srv\share\x | //server/share/f.txt | \\server\share\f.txt`,
  'C%3A%5CUsers%5Cprivate%5Csecret.txt',
  'C%25253A%25255CUsers%25255Cprivate%25255Csecret.txt',
  String.raw`C:\Proj\app\..\..\Users\private\x and C:\Proj\app\src\..\lib\a.js`,
  String.raw`C:\Proj\app2\secret.txt`,
  String.raw`c:\PROJ\APP\src\a.js`,
  '../../secret/x and /home/ada/proj/../other/y and ~/notes.txt and ssh://host/home/x',
  '// a comment, https://example.com/a/b, http://h:8080/x/y, /compact, src/a.js',
  '10/15/2026 mkdir -p build /help /compact',
  String.raw`$HOME/.ssh/key ${'${HOME}'}/private/x %APPDATA%\keys\a ~bob/x`,
  'file:///C:/Users/private/key.txt and file:///home/ada/proj/src/a.rs',
  '/etc /root /home /var /proc /sys /usr /opt /srv',
  String.raw`Error:C:\Users\private\key.txt`,
  'password=hunter22 Bearer abcdefghijklmnop1234 postgres://u:pw1234@db/x',
  'DefaultEndpointsProtocol=https;AccountName=x;AccountKey=abc+def/ghi==;EndpointSuffix=y',
  `password: hunter and secret=abc and token="hello world" and api_key: 'qwerty'`,
  'export GITHUB_TOKEN=ghp_abcdefghijklmnopqrstuvwxyz0123 && npm test',
  'mysql -uroot -pSyntheticPassword database',
  'commit 3f2a9c8e1b7d6f5a4c3b2a1908f7e6d5c4b3a291',
  '-----BEGIN RSA PRIVATE KEY-----\nsynthetic data\n-----END RSA PRIVATE KEY-----',
  'aBcDeFgHiJkLmNoPqRsTuVwXyZ01234567890123456789',
  'Un titre français — vérifier été et œuvre.',
  'é/home/jean/.ssh/id_rsa', 'éC:/Users/jean/secret.txt',
  'ésk-ant-abcdefghijklmnopqrstuvwxyz123456', 'àghp_abcdefghijklmnopqrstuvwxyz123456',
  'éaBcDeFgHiJkLmNoPqRsTuVwXyZ01234567890123456789',
  String.raw`x1://C:\Users\jean\.ssh\id_rsa`,
  String.raw`x1:///C:\Users\jean\.ssh\id_rsa`,
  String.raw`vscode://file/C:\Users\jean\secret.txt`,
  String.raw`x1://C:\Proj\app\src\a.js`,
  String.raw`x1://\\srv\share\f`,
  String.raw`http://host/C:\Users\private\key.txt`,
  'vscode://file/C:/Users/private/key.txt',
  'https://user:hello%20world@host/page',
  'https://user:p%40ss@host/page',
  'https://user%3Ap%2540ss@host/page',
  'password=%22hello world%22',
  ...['npm_abcdefghijklmnopqrstuvwxyz0123', 'sk_live_abcdefghijklmnop', 'sk_test_abcdefghijklmnop', 'rk_live_abcdefghijklmnop', 'hf_abcdefghijklmnopqrstuvwxyz', 'ya29.a0AfH6SMBxyz123', 'xoxb-12345-abcdef', 'xoxp-12345-abcdef', 'glpat-abcdefghij1234', 'AKIAIOSFODNN7EXAMPLE', 'ASIAIOSFODNN7EXAMPLE', 'eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIx', 'AIza0123456789abcdefghijklmnopqrstuv', 'github_pat_abcdefghijklmnopqrstuvwxyz0123', 'xai-abcdefghijklmnopqrstuvwxyz0123', 'dop_v1_abcdefghijklmnopqrstuvwxyz0123'].map(v => `use ${v} here`),
]
const cases = inputs.map(input => ({input, roots, max: 500, expected: api.guestText(input, api.cleanRoots(roots), 500)}))
for (const input of ['a'.repeat(290) + ' sk-proj-abcdefghijklmnopqrstuvwxyz1234567890 tail', 'short '.repeat(80)]) {
  cases.push({input, roots, max:300, expected:api.guestText(input, api.cleanRoots(roots), 300)})
}
const output = {sourceSha256:crypto.createHash('sha256').update(source).digest('hex'), cases}
const destination = fileURLToPath(new URL('../tests/fixtures/share_guest_text_reference.json', import.meta.url))
fs.mkdirSync(path.dirname(destination), {recursive:true})
fs.writeFileSync(destination, JSON.stringify(output, null, 2) + '\n')
console.log(`${cases.length} synthetic guest-text vectors written`)
