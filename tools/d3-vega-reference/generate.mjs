import {readFileSync, writeFileSync, mkdirSync} from 'node:fs';
import {ticks} from 'd3-array';
import {execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {formatLocale} from 'd3-format';
import {timeFormatLocale} from 'd3-time-format';
import {locale as vegaLocale} from 'vega-format';

const root = new URL('../../', import.meta.url);
const read = path => JSON.parse(readFileSync(new URL(path, root)));
const numberLocales = Object.fromEntries(['en-US', 'de-DE', 'fr-FR', 'ja-JP'].map(name => [
  name, read(`avenger-format-number-d3/locales/${name}.json`)
]));
numberLocales.custom = {
  decimal: '·', thousands: '_', grouping: [3, 2], currency: ['¤', ' coins'],
  minus: 'MINUS', percent: 'pct', nan: 'missing',
  numerals: ['⓪', '①', '②', '③', '④', '⑤', '⑥', '⑦', '⑧', '⑨']
};
const timeLocales = Object.fromEntries(['en-US', 'de-DE', 'fr-FR', 'ja-JP'].map(name => [
  name, read(`avenger-format-datetime-d3/locales/${name}.json`)
]));
timeLocales.custom = {...timeLocales['en-US'], dateTime: '%x at %X', date: '%Y/%-m/%-d', time: '%Hh%M', periods: ['morning', 'evening']};
const numberValue = value => typeof value === 'string' ? Number(value) : value;
const hex = value => {
  const buffer = Buffer.alloc(8);
  buffer.writeDoubleBE(value);
  return buffer.toString('hex');
};

function numberCases() {
  const cases = [];
  const add = (spec, values, locale = 'en-US', mode = 'format', args = []) => {
    const d3 = formatLocale(numberLocales[locale]);
    const vega = vegaLocale(numberLocales[locale], timeLocales['en-US']);
    const f = mode === 'format' ? d3.format(spec)
      : mode === 'prefix' ? d3.formatPrefix(spec, args[0])
      : vega.formatFloat(spec);
    for (const value of values) {
      const number = numberValue(value);
      cases.push({locale, mode, spec, args, value, bits: hex(number), expected: f(number)});
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

// Vega reads the step and largest magnitude from the domain, and Avenger infers them from
// the ticks. These domains give the same precision both ways. A degenerate domain does not:
// Vega's zero step leaves the pattern's default precision.
function tickCases() {
  const cases = [];
  const vega = locale => vegaLocale(numberLocales[locale], timeLocales['en-US']);
  const uniform = (spec, [start, stop, count], locale = 'en-US') => {
    const values = ticks(start, stop, count);
    const f = vega(locale).formatSpan(start, stop, count, spec);
    cases.push({locale, spacing: 'uniform', spec, domain: [start, stop, count], bits: values.map(hex), expected: values.map(f)});
  };
  const varying = (spec, values, locale = 'en-US') => {
    const f = vega(locale).formatFloat(spec);
    cases.push({locale, spacing: 'varying', spec, bits: values.map(hex), expected: values.map(f)});
  };
  const domains = [
    [0, 1, 10], [0, 1, 5], [-0.5, 0.5, 5], [1, 0, 5], [0, 7, 10], [0.001, 0.002, 5],
    [1000, 1001, 10], [0, 100, 5], [0, 35, 2], [0, 1.2e6, 6], [900000, 1100000, 4],
    [-2e9, 5e9, 7], [1e-7, 5e-7, 4], [0.571, 0.58, 1], [57.1, 58, 1]
  ];
  for (const spec of ['', ',', 'f', '.2f', 's', '.2s', '%', 'e', '+.1e', 'g', 'r', 'p', '$,f', '08.2f']) {
    for (const domain of domains) uniform(spec, domain);
  }
  for (const locale of ['de-DE', 'fr-FR', 'ja-JP', 'custom']) {
    uniform('$,f', [1200, 1300, 4], locale);
    uniform('%', [0, 1, 10], locale);
    uniform('s', [900000, 1100000, 4], locale);
  }
  for (const spec of ['', ',', 's', 'f', '.2f', 'e', '%', '$,']) {
    for (const values of [[1, 2, 5, 10, 20, 50, 100], [0.001, 0.01, 0.1, 1, 10], [1e3, 1e4, 1e5, 1e6], [0.5, 1, 1.5, 2, 3]]) {
      varying(spec, values);
    }
  }
  varying(',', [0.001, 1234.5], 'de-DE');
  return {locales: numberLocales, cases};
}

function timeCases(zone) {
  const cases = [];
  const add = (spec, values, locale = 'en-US') => {
    const f = timeFormatLocale(timeLocales[locale]).format(spec);
    for (const value of values) cases.push({locale, zone, spec, value, expected: f(new Date(value))});
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
    }
  }
  add('%Y-%m-%d %H:%M:%S.%L %Z %Q %s %j %U %W %V %g %G', dates);

  const transitions = {
    'America/New_York': ['2024-03-10T06:59:59.999Z', '2024-03-10T07:00:00Z', '2024-11-03T05:30:00Z', '2024-11-03T06:30:00Z'],
    'Australia/Lord_Howe': ['2024-04-06T14:59:59Z', '2024-04-06T15:00:00Z', '2024-10-05T15:29:59.999Z', '2024-10-05T15:30:00Z'],
    'America/Sao_Paulo': ['2018-11-04T02:59:59Z', '2018-11-04T03:00:00Z', '2018-11-04T04:00:00Z'],
    'America/Havana': ['2024-11-03T04:00:00Z', '2024-11-03T04:30:00Z', '2024-11-03T05:00:00Z', '2024-11-03T05:30:00Z', '2024-03-10T04:59:59.999Z', '2024-03-10T05:00:00Z'],
    'Asia/Kathmandu': ['1985-12-31T18:29:59.999Z', '1985-12-31T18:30:00Z']
  }[zone] || [];
  if (transitions.length) {
    add('%Y-%m-%d %H:%M:%S.%L %Z %Q %s', transitions.map(Date.parse));
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
  const tickSets = tickCases();
  for (const [crate, name, fixture] of [['number', 'upstream.json', numbers], ['number', 'ticks.json', tickSets], ['datetime', 'upstream.json', times]]) {
    const dir = new URL(`avenger-format-${crate}-d3/tests/fixtures/`, root);
    mkdirSync(dir, {recursive: true});
    writeFileSync(new URL(name, dir), `{\n  \"locales\": ${JSON.stringify(fixture.locales, null, 2)},\n  \"cases\": [\n${fixture.cases.map(item => '    ' + JSON.stringify(item)).join(',\n')}\n  ]\n}\n`);
  }
  console.log(`Generated ${numbers.cases.length} number, ${tickSets.cases.length} tick set, and ${times.cases.length} datetime cases with ${process.version}.`);
}
