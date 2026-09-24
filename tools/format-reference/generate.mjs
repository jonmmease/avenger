import {readFileSync, writeFileSync, mkdirSync} from 'node:fs';
import {formatLocale} from 'd3-format';
import {locale as vegaLocale} from 'vega-format';

const root = new URL('../../', import.meta.url);
const read = path => JSON.parse(readFileSync(new URL(path, root)));
const builtin = name => read(`avenger-format-number/locales/${name}.json`);
const numberLocales = Object.fromEntries(['en-US', 'de-DE', 'fr-FR', 'ja-JP'].map(name => [name, builtin(name)]));
numberLocales.custom = {
  decimal: '·', thousands: '_', grouping: [3, 2], currency: ['¤', ' coins'],
  minus: 'MINUS', percent: 'pct', nan: 'missing',
  numerals: ['⓪', '①', '②', '③', '④', '⑤', '⑥', '⑦', '⑧', '⑨']
};
const numberValue = value => typeof value === 'string' ? Number(value) : value;

function numberCases() {
  const cases = [];
  const add = (spec, values, locale = 'en-US', mode = 'format', args = []) => {
    const d3 = formatLocale(numberLocales[locale]);
    const vega = vegaLocale(numberLocales[locale]);
    const f = mode === 'format' ? d3.format(spec)
      : mode === 'prefix' ? d3.formatPrefix(spec, args[0])
      : mode === 'float' ? vega.formatFloat(spec)
      : vega.formatSpan(...args, spec);
    for (const value of values) {
      const number = numberValue(value);
      const buffer = Buffer.alloc(8);
      buffer.writeDoubleBE(number);
      cases.push({locale, mode, spec, args, value, bits: buffer.toString('hex'), expected: f(number)});
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
  for (const args of [[0.571, 0.58, 1], [0.58, 0.571, 1], [-0.58, -0.571, 1], [0.575, 0.58, 0.5], [57.1, 58, 1], [1, 1, 10]]) {
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

const fixture = numberCases();
const dir = new URL('avenger-format-number/tests/fixtures/', root);
mkdirSync(dir, {recursive: true});
writeFileSync(new URL('upstream.json', dir), `{\n  \"locales\": ${JSON.stringify(fixture.locales, null, 2)},\n  \"cases\": [\n${fixture.cases.map(item => '    ' + JSON.stringify(item)).join(',\n')}\n  ]\n}\n`);
console.log(`Generated ${fixture.cases.length} number cases with ${process.version}.`);
