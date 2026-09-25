import {readFileSync, writeFileSync, mkdirSync} from 'node:fs';
import {tickStep} from 'd3-array';
import {execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {formatLocale} from 'd3-format';
import {timeFormatLocale} from 'd3-time-format';
import {locale as vegaLocale} from 'vega-format';

const root = new URL('../../', import.meta.url);
const read = path => JSON.parse(readFileSync(new URL(path, root)));
const numberLocales = {
  'en-US': read('avenger-format-number-d3/locales/en-US.json'),
  ...Object.fromEntries(['de-DE', 'fr-FR', 'ja-JP'].map(name => [
    name, read(`avenger-format-number-d3/tests/fixtures/locales/${name}.json`)
  ]))
};
numberLocales.custom = {
  decimal: '·', thousands: '_', grouping: [3, 2], currency: ['¤', ' coins'],
  minus: 'MINUS', percent: 'pct', nan: 'missing',
  numerals: ['⓪', '①', '②', '③', '④', '⑤', '⑥', '⑦', '⑧', '⑨']
};
const timeLocales = {
  'en-US': read('avenger-format-datetime-d3/locales/en-US.json'),
  ...Object.fromEntries(['de-DE', 'fr-FR', 'ja-JP'].map(name => [
    name, read(`avenger-format-datetime-d3/tests/fixtures/locales/${name}.json`)
  ]))
};
timeLocales.custom = {...timeLocales['en-US'], dateTime: '%x at %X', date: '%Y/%-m/%-d', time: '%Hh%M', periods: ['morning', 'evening']};
const numberValue = value => typeof value === 'string' ? Number(value) : value;

function numberCases() {
  const cases = [];
  const add = (spec, values, locale = 'en-US', mode = 'format', args = []) => {
    const d3 = formatLocale(numberLocales[locale]);
    const vega = vegaLocale(numberLocales[locale], timeLocales['en-US']);
    const f = mode === 'format' ? d3.format(spec)
      : mode === 'prefix' ? d3.formatPrefix(spec, args[0])
      : mode === 'float' ? vega.formatFloat(spec)
      : vega.formatSpan(...args, spec);
    const fixtureMode = mode === 'span' ? 'step' : mode;
    const fixtureArgs = mode === 'span'
      ? [tickStep(...args), Math.max(Math.abs(args[0]), Math.abs(args[1]))]
      : args;
    for (const value of values) {
      const number = numberValue(value);
      const buffer = Buffer.alloc(8);
      buffer.writeDoubleBE(number);
      cases.push({locale, mode: fixtureMode, spec, args: fixtureArgs, value, bits: buffer.toString('hex'), expected: f(number)});
    }
  };
  for (const type of ['', 'n', 'b', 'o', 'd', 'x', 'X', 'c', 'e', 'f', 'g', 'r', 's', '%', 'p']) {
    add(type, [0, '-0', 1.25, -1.25, 65, 1234.5, 'NaN', 'Infinity', '-Infinity']);
    add(`+012,.2${type}`, [-0.001, 12.5, -1234.5]);
  }
  for (const spec of ['.0f', '.1f', '.2f', '.20f', '.0g', '.1g', '.3g', '.21g', '.30g', '.300f', '.0e', '.2e', '.20e', '.3r', '.2s', '.3~s', '.2%', '.3p', '.2', '~g']) {
    add(spec, [2.5, 0.25, 1.005, 9.999, 999.5, 0.0000009999, 1e21, 1e23, 5e-324]);
  }
  for (const locale of Object.keys(numberLocales)) {
    for (const spec of [',.2f', '$,.2f', '($.2f', '+$015,.2f', '0=15,.1f', '*>15,.2f', '*<15,.2f', '*^15,.2f', '.0%', '$.0%', '#08x', '+.3~e', '020,.2f']) {
      add(spec, [-1234.5, 123456789.5], locale);
    }
    for (const spec of ['+12f', '($12,.2f', '012d', '.2s', 'c']) add(spec, ['NaN', 'Infinity', '-Infinity'], locale);
    add(',.1', [0, 900000, 1100000], locale, 'prefix', [1100000]);
  }
  for (const locale of ['en-US', 'de-DE']) {
    for (const spec of [null, '', ',', '.2f', '%', 'e']) {
      add(spec, [0.1, 1.23, 1200, -0.0001], locale, 'float');
    }
  }
  for (const spec of [null, '', 'f', '.2f', 's', '.2s', '%', 'e', 'g']) {
    add(spec, [0, 0.1, 0.3, 1], 'en-US', 'span', [0, 1, 10]);
    add(spec, [900000, 1000000, 1100000], 'en-US', 'span', [900000, 1100000, 4]);
  }
  for (const locale of ['de-DE', 'fr-FR', 'ja-JP', 'custom']) {
    add('$,f', [1234.5], locale, 'span', [1200, 1300, 4]);
    add('%', [0.3], locale, 'span', [0, 1, 10]);
    add('s', [900000], locale, 'span', [900000, 1100000, 4]);
  }
  for (const args of [[0.571, 0.58, 1], [57.1, 58, 1]]) {
    for (const spec of ['f', 'e', 'g']) add(spec, [args[0], args[1]], 'en-US', 'span', args);
  }
  for (const spec of ['.2e', '.2g', '.2s', '.1r']) add(spec, [1e-323, 1e-7, 1e21, 1e23]);
  add('d', [1e21, 1e23, 1000000000000000100]);
  add('x', [2 ** 64, 1e100]);
  let state = 0x123456789abcdefn;
  const specs = ['.20f', '.21g', '.20e', '.3s', '.4r', 'd', 'x', '+015,.2f'];
  for (let index = 0; index < 128; index++) {
    state = BigInt.asUintN(64, state * 6364136223846793005n + 1442695040888963407n);
    const buffer = Buffer.alloc(8);
    buffer.writeBigUInt64BE(state);
    const value = buffer.readDoubleBE();
    if (Number.isFinite(value)) add(specs[index % specs.length], [value]);
  }
  return {locales: numberLocales, cases};
}

function timeCases(zone) {
  const cases = [];
  const add = (spec, values, locale = 'en-US', mode = 'format') => {
    const d3 = timeFormatLocale(timeLocales[locale]);
    const vega = vegaLocale(numberLocales['en-US'], timeLocales[locale]);
    const f = mode === 'multi' ? vega.timeFormat(spec) : d3.format(spec);
    for (const value of values) cases.push({locale, zone, mode, spec, value, expected: f(new Date(value))});
  };
  const dates = [Date.parse('2024-02-29T13:05:06.007Z'), -1, Date.parse('0001-01-01T00:00:00Z'), Date.parse('-000001-01-01T00:00:00Z')];
  if (zone === 'UTC') {
    for (const directive of 'aAbBcdefgGHIjLmMpqQsSuUVwWxXyYZ%') {
      add(`%${directive}|%-${directive}|%_${directive}|%0${directive}`, dates);
    }
    const boundaries = ['2015-12-31', '2016-01-01', '2016-01-03', '2016-01-04', '2017-01-01', '2020-12-31', '2021-01-01', '2021-01-04'].map(date => Date.parse(`${date}T12:00:00Z`));
    add('%Y-%m-%d %j %u %w %U %W %V %g %G', boundaries);
    for (const locale of Object.keys(timeLocales)) {
      add('%c | %x | %X | %A %B %p | 100%%', dates.slice(0, 2), locale);
      if (locale !== 'en-US') {
        add(null, ['2024-05-01T00:00:00Z', '2024-05-06T13:00:00Z'].map(Date.parse), locale, 'multi');
      }
    }
  }
  add('%Y-%m-%d %H:%M:%S.%L %Z %Q %s %j %U %W %V %g %G', dates);

  // Local calendar boundaries exercise every selection branch in each display zone.
  const boundaries = ['2024-01-01T00:00:00', '2024-04-01T00:00:00', '2024-05-01T00:00:00', '2024-05-05T00:00:00', '2024-05-06T00:00:00', '2024-05-06T01:00:00', '2024-05-06T01:02:00', '2024-05-06T01:02:03', '2024-05-06T01:02:03.004'].map(Date.parse);
  const overrides = {year: 'Y%Y', quarter: 'Q%q', month: '%b', week: 'W%U', date: '%-d', hours: '%Hh', minutes: '%Mmin', seconds: '%Ssec', milliseconds: '%f'};
  add(null, boundaries, 'en-US', 'multi');
  add(overrides, boundaries, 'en-US', 'multi');

  const transitions = {
    'America/New_York': ['2024-03-10T06:59:59.999Z', '2024-03-10T07:00:00Z', '2024-11-03T05:30:00Z', '2024-11-03T06:30:00Z'],
    'Australia/Lord_Howe': ['2024-04-06T14:59:59Z', '2024-04-06T15:00:00Z', '2024-10-05T15:29:59.999Z', '2024-10-05T15:30:00Z'],
    'America/Sao_Paulo': ['2018-11-04T02:59:59Z', '2018-11-04T03:00:00Z', '2018-11-04T04:00:00Z'],
    'America/Havana': ['2024-11-03T04:00:00Z', '2024-11-03T04:30:00Z', '2024-11-03T05:00:00Z', '2024-11-03T05:30:00Z', '2024-03-10T04:59:59.999Z', '2024-03-10T05:00:00Z'],
    'Asia/Kathmandu': ['1985-12-31T18:29:59.999Z', '1985-12-31T18:30:00Z']
  }[zone] || [];
  if (transitions.length) {
    add('%Y-%m-%d %H:%M:%S.%L %Z %Q %s', transitions.map(Date.parse));
    add(null, transitions.map(Date.parse), 'en-US', 'multi');
  }
  return cases;
}

if (process.argv[2] === '--time') {
  process.stdout.write(JSON.stringify(timeCases(process.env.TZ)));
} else {
  const numbers = numberCases();
  const times = {locales: timeLocales, cases: []};
  for (const zone of ['UTC', 'America/New_York', 'Asia/Kathmandu', 'Australia/Lord_Howe', 'America/Sao_Paulo', 'America/Havana']) {
    times.cases.push(...JSON.parse(execFileSync(process.execPath, [fileURLToPath(import.meta.url), '--time'], {env: {...process.env, TZ: zone}, maxBuffer: 8 * 1024 * 1024})));
  }
  for (const [crate, fixture] of [['number', numbers], ['datetime', times]]) {
    const dir = new URL(`avenger-format-${crate}-d3/tests/fixtures/`, root);
    mkdirSync(dir, {recursive: true});
    writeFileSync(new URL('upstream.json', dir), `{\n  \"locales\": ${JSON.stringify(fixture.locales, null, 2)},\n  \"cases\": [\n${fixture.cases.map(item => '    ' + JSON.stringify(item)).join(',\n')}\n  ]\n}\n`);
  }
  console.log(`Generated ${numbers.cases.length} number and ${times.cases.length} datetime cases with ${process.version}.`);
}
