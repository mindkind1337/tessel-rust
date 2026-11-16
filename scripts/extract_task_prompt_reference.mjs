// Synthetic differential fixtures for Tessel App.vue's taskPrompt (MIT).
// Reads source code only; never reads projects, credentials or agent mailboxes.
import { readFileSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import vm from 'node:vm';

const path = process.argv[2] || 'C:/Tessel/src/renderer/src/App.vue';
const app = readFileSync(path, 'utf8').replace(/\r\n/g, '\n');
const start = app.indexOf('function taskPrompt(task, ws) {');
const end = app.indexOf('\n// opts (for a team lead)', start);
if (start < 0 || end < 0) throw new Error('Task prompt source boundary changed.');
const source = app.slice(start, end).trim();
const cases = [
  { name: 'direct', task: { title: 'Fix synthetic fixture', brief: 'Keep the fixture offline.' }, ws: { cwd: 'C:/fixtures/project' } },
  { name: 'direct-empty-brief', task: { title: 'Inspect fixture' }, ws: { cwd: '/fixtures/project' } },
  { name: 'isolated-base', task: { title: 'Fix fixture', brief: 'Run fixture checks.', worktree: { path: 'C:/fixtures/copy', branch: 'agent/fixture', baseBranch: 'develop' } }, ws: { cwd: 'C:/fixtures/project' } },
  { name: 'isolated-default-base', task: { title: 'Fix fixture', worktree: { path: '/fixtures/copy', branch: 'agent/fixture' } }, ws: { cwd: '/fixtures/project' } },
  { name: 'origins', task: { title: 'Quoted fixture card', brief: 'Synthetic instructions.', guestBy: 'Fixture guest', copiedFrom: '2026-01-02' }, ws: { cwd: '/fixtures/project' } },
  { name: 'team-with-lead', task: { title: 'Fixture team task', teamId: 'fixture-team', paneId: 'fixture-worker' }, ws: { cwd: '/fixtures/project' }, team: { id: 'fixture-team', name: 'Fixture team' }, lead: { label: 'Fixture lead' } },
  { name: 'team-with-mailbox', task: { title: 'Fixture team task', teamId: 'fixture-team', paneId: 'fixture-worker' }, ws: { cwd: '/fixtures/project' }, team: { id: 'fixture-team', name: 'Fixture team' }, lead: { label: 'Fixture lead' }, guide: 'Read the synthetic fixture mailbox.' },
  { name: 'team-without-lead', task: { title: 'Fixture team task', teamId: 'fixture-team' }, ws: { cwd: '/fixtures/project' }, team: { id: 'fixture-team', name: 'Fixture team' } },
  { name: 'unicode', task: { title: 'Vérifier Montréal — fixture', brief: 'Ne rien exécuter.\nDeuxième ligne « Québec ».', guestBy: 'Invité Ω' }, ws: { cwd: 'C:/fixtures/Équipe' } },
  { name: 'isolated-empty-base', task: { title: 'Fixture', worktree: { path: '/fixtures/copy', branch: 'agent/fixture', baseBranch: '' } }, ws: { cwd: '/fixtures/project' } },
  { name: 'team-empty-guide', task: { title: 'Fixture', teamId: 'fixture-team', paneId: 'fixture-worker' }, ws: { cwd: '/fixtures/project' }, team: { id: 'fixture-team', name: 'Fixture team' }, guide: '' },
  { name: 'whitespace-brief', task: { title: 'Fixture', brief: '  ' }, ws: { cwd: '/fixtures/project' } },
  { name: 'missing-workspace', task: { title: 'Fixture only' }, ws: null },
];
for (const item of cases) {
  const context = {
    task: item.task, ws: item.ws,
    teamLead: () => item.lead || null,
    teamById: () => item.team || null,
    paneLabel: lead => lead.label,
    channelBoxes: Object.hasOwn(item, 'guide') ? { 'fixture-team': { 'fixture-worker': { guide: item.guide } } } : {},
  };
  vm.runInNewContext(`${source}\nresult = taskPrompt(task, ws);`, context, { timeout: 1000 });
  item.expected = context.result;
}
const fixture = {
  source: 'src/renderer/src/App.vue#taskPrompt',
  sha256: createHash('sha256').update(source).digest('hex'),
  cases,
};
writeFileSync(new URL('../tests/fixtures/task_prompt.json', import.meta.url), `${JSON.stringify(fixture, null, 2)}\n`);
console.log(`Wrote ${cases.length} synthetic task prompt vectors.`);
