// Opt-in oracle: npm ci && npm run generate. Normal Rust tests use the saved JSON.
// Vega 6.2.0, commit 4dea72921d25bf6ff6636a9f9cb6c63ff696932c (LICENSE.vega).
import {View, parse, version} from 'vega';
import {writeFileSync} from 'node:fs';

async function run(values, transform) {
  const view = await new View(parse({data: [{name: 'rows', values, transform}]}), {renderer: 'none'}).runAsync();
  return view;
}
const cases = [
  ['default', [-3, 29], {}],
  ['tail', [0, 29], {step: 5, nice: false}],
  ['anchor', [0, 29], {step: 5, nice: false, anchor: 4}],
  ['singleton_zero', [0, 0], {}],
  ['singleton_negative', [-7, -7], {}],
  ['steps', [0, 100], {maxbins: 10, steps: [1, 2, 5, 10, 20]}],
  ['minstep', [-1, 1], {maxbins: 100, minstep: .5}],
  ['span', [0, 100], {span: 10, maxbins: 10}],
  ['negative_half', [0, Math.pow(10, -1.5)], {base: 10, maxbins: 20}],
  ['fractional', [-.3, .6], {step: .1}],
  ['divide', [0, 117], {maxbins: 7, base: 2, divide: [2]}],
];
const bins = [];
for (const [name, extent, options] of cases) {
  const values = [null, NaN, -Infinity, extent[0] - 1, extent[0], extent[0] + 1e-15,
    extent[0] + .1, extent[1] - 1e-15, extent[1], extent[1] + 1, Infinity];
  if (name === 'tail') values.push(5 - 1e-12, 5, 5 + 1e-12, 28, 29, 30, 31);
  const view = await run(values.map(x => ({x})), [{type: 'bin', field: 'x', extent, ...options, signal: 'parameters', as: ['lo', 'hi']}]);
  const {start, stop, step} = view.signal('parameters');
  // Recover the assignment bound from Vega's public stop before the anchor shift.
  const unanchored = await run([{x: 0}], [{type:'bin',field:'x',extent,...options,anchor: null,signal:'parameters'}]);
  const origin = unanchored.signal('parameters').start;
  bins.push({name, extent, options, parameters: [start, stop, step, start + Math.ceil((stop - origin) / step) * step],
    values, bounds: view.data('rows').map(({lo, hi}) => [lo, hi])});
  view.finalize(); unanchored.finalize();
}
const ops = ['count','valid','missing','sum','min','max','mean','variance','variancep','stdev','stdevp'];
const aggregates = [];
for (const values of [[], [null, NaN], [3], [1, 2, 3, null, NaN], [1e9, 1e9 + 1, 1e9 + 2]]) {
  const view = await run(values.map(x => ({x})), [{type:'aggregate',ops,fields:ops.map(op=>op==='count'?null:'x'),as:ops}]);
  aggregates.push({values, rows: view.data('rows')}); view.finalize();
}
writeFileSync(new URL('../fixtures/vega-6.2.0.json', import.meta.url), JSON.stringify({version, commit:'4dea72921d25bf6ff6636a9f9cb6c63ff696932c', bins, aggregates}, (_,v) =>
  v === undefined ? {$number:'undefined'} : typeof v === 'number' && !Number.isFinite(v) ? {$number:String(v)} : v, 2) + '\n');
