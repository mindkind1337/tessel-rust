// Synthetic date examples from the Vue Intl formatting contract; no account data.
const fs = require('node:fs') as typeof import('node:fs');
const path = require('node:path') as typeof import('node:path');
const crypto = require('node:crypto') as typeof import('node:crypto');

interface LocaleSelectionCase {
  language: string;
  preferred: string[];
  expected: string;
}

interface RegionalDateCase {
  locale: string;
  timestamp: number;
  expected: string;
}

const source = process.argv[2];
if (!source) throw new Error('Pass the Tessel renderer/src directory');
const outPath = process.argv[3] ?? path.join(__dirname, '../tests/fixtures/regional_date.json');

const i18n = fs.readFileSync(path.join(source, 'i18n/index.js'), 'utf8');
const component = fs.readFileSync(
  path.join(source, 'components/settings/AgentModelDefaults.vue'),
  'utf8',
);
const body = i18n.match(/export function intlLocale\(\) \{([\s\S]*?)\n\}/)?.[1];
if (!body || !component.includes("dateStyle: 'medium', timeStyle: 'short'")) {
  throw new Error('Source contract changed');
}

const choose = new Function('state', 'navigatorLanguages', body) as (
  state: { locale: string },
  navigatorLanguages: () => string[],
) => string;

const selections: LocaleSelectionCase[] = [];
const preferredLists: string[][] = [
  [],
  ['fr-FR', 'en-GB'],
  ['de-DE', 'en-CA', 'fr-CA'],
  ['EN-us', 'FR-ca'],
];
for (const language of ['en', 'fr']) {
  for (const preferred of preferredLists) {
    selections.push({
      language,
      preferred,
      expected: choose({ locale: language }, () => preferred),
    });
  }
}

const dates: RegionalDateCase[] = [];
for (const locale of ['fr-CA', 'fr-FR', 'en-US', 'en-CA', 'en-GB']) {
  for (let month = 1; month <= 12; month++) {
    for (const hour of [0, 12, 17, 23]) {
      const timestamp = Date.UTC(2024, month - 1, 14, hour, 3, 59);
      dates.push({
        locale,
        timestamp,
        expected: new Date(timestamp).toLocaleString(locale, {
          dateStyle: 'medium',
          timeStyle: 'short',
          timeZone: 'UTC',
        }),
      });
    }
  }
}

fs.writeFileSync(
  outPath,
  JSON.stringify({
    sourceSha256: crypto.createHash('sha256').update(i18n + component).digest('hex'),
    icuVersion: process.versions.icu,
    selections,
    dates,
  }) + '\n',
);
console.log(`${selections.length} locale choices, ${dates.length} date examples`);
