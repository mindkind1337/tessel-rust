// Hash the source signature using synthetic values only.
const fs = require('node:fs');
const path = require('node:path');
const { createHash } = require('node:crypto');
const source = fs.readFileSync(process.argv[2], 'utf8');
const body = source.match(/export function launchSignature\(launch\) \{([\s\S]*?)\n\}/)?.[1];
if (!body) throw new Error('Source launchSignature not found');
const signature = new Function('launch', body);
const cases = [
  { command:'codex', args:'', env:{} },
  { command:'codex', args:'-m pane-model -c model_reasoning_effort=high', env:{} },
  { command:'custom cli', args:'--literal "hello"', env:{ B:'two', A:'synthetic-secret' } },
  { command:'custom cli', args:'--literal "hello"', env:{ A:'synthetic-secret', B:'two' } },
  { command:'synthetic-é🙂', args:'line\nquote"slash\\', env:{ VALUE:'é🙂\tvalue', EMPTY:'' } },
].map(input => ({input, expected:createHash('sha256').update(signature(input)).digest('hex')}));
fs.writeFileSync(path.join(__dirname,'../tests/fixtures/launch_fingerprint.json'), JSON.stringify({sourceSha256:createHash('sha256').update(source).digest('hex'),cases})+'\n');
