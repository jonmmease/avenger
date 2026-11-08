// Writes the reference fixtures of avenger-datafusion-transforms: bins from vega-statistics,
// stacks from the code of Vega's stack transform, and time units from vega-time.
//
// vega-time works in the local time of the process, so the time units of each time zone come
// from a child process with TZ set.
import {execFileSync} from 'node:child_process';
import {writeFileSync} from 'node:fs';
import {bin} from 'vega-statistics';
import {timeFloor, timeOffset} from 'vega-time';

const fixtures = new URL('../../avenger-datafusion-transforms/tests/fixtures/', import.meta.url);

// Infinities and NaN as strings, which JSON cannot hold as numbers.
const write = (name, value, count) => {
  writeFileSync(new URL(name, fixtures), JSON.stringify(value, (_, v) =>
    typeof v === 'number' && !Number.isFinite(v) ? String(v) : v));
  console.log(`${name}: ${count}`);
};

// A fixed pseudo-random sequence, so that the cases are the same each time.
let seed = 42;
function rand() {
  seed = (seed * 1103515245 + 12345) % 2147483648;
  return seed / 2147483648;
}

// --- bin ------------------------------------------------------------------------------------

// `_bins` and the band end of packages/vega-transforms/src/Bin.js, which need Vega's dataflow
// to run.
const EPSILON = 1e-14;
function binTransform(options, extent) {
  const bins = bin({...options, extent});
  const step = bins.step;
  let start = bins.start,
      stop = start + Math.ceil((bins.stop - start) / step) * step,
      a, d;
  if ((a = options.anchor) != null) {
    d = a - (start + step * Math.floor((a - start) / step));
    start += d;
    stop += d;
  }
  const f = v => v == null ? null
    : v < start ? -Infinity
    : v > stop ? +Infinity
    : (
        v = Math.max(start, Math.min(v, stop - step)),
        start + step * Math.floor(EPSILON + (v - start) / step)
      );
  const band = v => {
    const b0 = f(v);
    return [b0, b0 == null ? null : start + step * (1 + (b0 - start) / step)];
  };
  return {params: {start, stop, step}, band};
}

function linspace(lo, hi, n) {
  if (lo === hi) return [lo];
  return Array.from({length: n}, (_, i) => i === n - 1 ? hi : lo + (hi - lo) * i / (n - 1));
}

function binCases() {
  const extents = [
    [0, 100], [0.5, 9.7], [-3.2, 7.1], [0, 1], [0.001, 0.002], [5, 5], [0, 0],
    [-100, -1], [0, 1e6], [12.3, 987.6], [1e-9, 3e-9], [-0.07, 0.03],
  ];
  const optionSets = [
    {}, {maxbins: 10}, {maxbins: 50}, {maxbins: 99}, {maxbins: 7}, {nice: false},
    {step: 7}, {step: 0.25}, {steps: [1, 5, 25, 100]}, {anchor: 0.5}, {anchor: -1.3},
    {base: 2}, {divide: [3]}, {divide: [4, 2]}, {minstep: 1}, {span: 50},
    {maxbins: 7, nice: false, anchor: 1}, {maxbins: 30, base: 2, divide: [2]},
  ];
  const cases = [];
  for (const extent of extents) {
    for (const options of optionSets) {
      // Values that span the extent exactly, so that the extent of the data is `extent`.
      const values = linspace(extent[0], extent[1], 41);
      const t = binTransform(options, extent);
      cases.push({options, extent, params: t.params, values, bins: values.map(t.band)});
    }
    // An explicit extent narrower than the data: values outside it bin to -Infinity and
    // Infinity.
    const span = extent[1] - extent[0] || 1;
    const narrow = [extent[0] + span / 4, extent[1] - span / 4];
    const options = {extent: narrow, maxbins: 10};
    const values = linspace(extent[0] - span / 2, extent[1] + span / 2, 41);
    const t = binTransform(options, narrow);
    cases.push({options, extent: narrow, params: t.params, values, bins: values.map(t.band)});
  }
  return cases;
}

// --- stack ----------------------------------------------------------------------------------

// stackZero and stackNormalize are copied from packages/vega-encode/src/Stack.js (which needs
// Vega's dataflow to run), as is the sum of each group. stackCenter is Vega's with one
// change: a stack starts at -sum / 2, not (max - sum) / 2, so that each stack centers on zero.
function stackCenter(group, max, field, y0, y1) {
  var last = -group.sum / 2,
      m = group.length,
      j = 0, t;
  for (; j<m; ++j) {
    t = group[j];
    t[y0] = last;
    t[y1] = (last += Math.abs(field(t)));
  }
}

function stackNormalize(group, max, field, y0, y1) {
  var scale = 1 / group.sum,
      last = 0,
      m = group.length,
      j = 0, v = 0, t;
  for (; j<m; ++j) {
    t = group[j];
    t[y0] = last;
    t[y1] = last = scale * (v += Math.abs(field(t)));
  }
}

function stackZero(group, max, field, y0, y1) {
  var lastPos = 0,
      lastNeg = 0,
      m = group.length,
      j = 0, v, t;
  for (; j<m; ++j) {
    t = group[j];
    v = +field(t);
    if (v < 0) {
      t[y0] = lastNeg;
      t[y1] = (lastNeg += v);
    } else {
      t[y0] = lastPos;
      t[y1] = (lastPos += v);
    }
  }
}

function stackCases() {
  const stacks = {zero: stackZero, center: stackCenter, normalize: stackNormalize};
  const field = t => t.v;
  const makeRows = (kind, groups, perGroup) => {
    const rows = [];
    for (let s = 0; s < perGroup; ++s) {
      for (let g = 0; g < groups; ++g) {
        let v;
        const r = rand();
        if (kind === 'positive') v = Math.round(rand() * 1000) / 10;
        else if (kind === 'mixed') v = Math.round((rand() - 0.4) * 2000) / 7;
        else v = r < 0.15 ? null : r < 0.25 ? 0 : (rand() - 0.5) * 1e3 * rand();
        // Rows come sorted by `s` within each group, so the sums of Vega's groups (in input
        // order) and of the Rust function (in layer order) add in the same order.
        rows.push({g, s, v});
      }
    }
    return rows;
  };
  const cases = [];
  for (const kind of ['positive', 'mixed', 'nulls']) {
    for (const [groups, perGroup] of [[1, 12], [5, 7], [40, 3]]) {
      const rows = makeRows(kind, groups, perGroup);
      for (const offset of ['zero', 'center', 'normalize']) {
        const byGroup = new Map();
        rows.forEach((t, i) => {
          if (!byGroup.has(t.g)) byGroup.set(t.g, []);
          byGroup.get(t.g).push({...t, i});
        });
        const out = new Array(rows.length);
        for (const group of byGroup.values()) {
          let s = 0;
          for (const t of group) s += Math.abs(field(t));
          group.sum = s;
          stacks[offset](group, 0, field, 'y0', 'y1');
          for (const t of group) out[t.i] = [t.y0, t.y1];
        }
        cases.push({kind, groups, offset, rows, stacks: out});
      }
    }
  }
  return cases;
}

// --- timeunit -------------------------------------------------------------------------------

const ZONES = ['America/New_York', 'Asia/Kathmandu', 'Australia/Lord_Howe', 'UTC'];

// [vega-time units, step, timeunit unit, whether the ends compare]
const UNITS = [
  [['year'], 1, 'year', true],
  [['year'], 5, '5 years', true],
  [['year', 'quarter'], 1, 'quarter', true],
  [['year', 'month'], 1, 'month', true],
  [['year', 'month'], 3, '3 months', true],
  [['year', 'month', 'date'], 1, 'day', true],
  // vega-time does not end a unit at the end of the month, so only the starts compare.
  [['year', 'month', 'date'], 2, '2 days', false],
  [['year', 'month', 'date', 'hours'], 1, 'hour', true],
  [['year', 'month', 'date', 'hours'], 3, '3 hours', true],
  [['year', 'month', 'date', 'hours', 'minutes'], 1, 'minute', true],
  [['year', 'month', 'date', 'hours', 'minutes'], 15, '15 minutes', true],
  [['year', 'month', 'date', 'hours', 'minutes', 'seconds'], 1, 'second', true],
  [['year', 'month', 'date', 'hours', 'minutes', 'seconds', 'milliseconds'], 1, 'millisecond', true],
  // Collapsing units: vega-time uses the year 2012, the reference year of Sunday weeks.
  [['quarter'], 1, 'quarter_of_year', true],
  [['month'], 1, 'month_of_year', true],
  [['month', 'date'], 1, 'monthdate', true],
  [['dayofyear'], 1, 'day_of_year', true],
  [['week'], 1, 'week_of_year', true],
  [['date'], 1, 'day_of_month', true],
  [['day'], 1, 'day_of_week', true],
  [['hours'], 1, 'hour_of_day', true],
  [['hours', 'minutes'], 1, 'minute_of_day', true],
  [['hours', 'minutes', 'seconds'], 1, 'second_of_day', true],
  [['minutes'], 1, 'minute_of_hour', true],
  [['seconds'], 1, 'second_of_minute', true],
];

function timeValues() {
  seed = 7;
  const v = [];
  const lo = Date.UTC(2023, 0, 1), hi = Date.UTC(2026, 0, 1);
  for (let i = 0; i < 120; ++i) v.push(Math.floor(lo + rand() * (hi - lo)));
  // Every 20 minutes for a day around each change of offset in 2024, in New York and on Lord
  // Howe Island.
  for (const d of [Date.UTC(2024, 2, 10), Date.UTC(2024, 10, 3), Date.UTC(2024, 3, 7), Date.UTC(2024, 9, 6)]) {
    for (let m = -12 * 60; m <= 12 * 60; m += 20) v.push(d + m * 60000 + 1234);
  }
  // Exact unit starts and the end of a year.
  v.push(Date.UTC(2024, 0, 1), Date.UTC(2024, 11, 31, 23, 59, 59, 999), Date.UTC(2025, 0, 1));
  return v;
}

function timeUnitZone(values) {
  return UNITS.map(([units, step, unit, ends]) => {
    const floor = timeFloor(units, step);
    const starts = values.map(v => +floor(new Date(v)));
    const endv = ends ? starts.map(s => +timeOffset(units[units.length - 1], new Date(s), step)) : null;
    return {unit, starts, ends: endv};
  });
}

if (process.argv[2] === 'zone') {
  process.stdout.write(JSON.stringify(timeUnitZone(timeValues())));
} else {
  const bins = binCases();
  write('bin.json', bins, `${bins.length} cases`);
  const stacks = stackCases();
  write('stack.json', stacks, `${stacks.length} cases`);
  const values = timeValues();
  const zones = {};
  for (const tz of ZONES) {
    const json = execFileSync(process.execPath, [new URL(import.meta.url).pathname, 'zone'], {
      env: {...process.env, TZ: tz},
      maxBuffer: 1 << 28,
    });
    zones[tz] = JSON.parse(json);
  }
  write('timeunit.json', {values, zones}, `${values.length} values, ${UNITS.length} units, ${ZONES.length} time zones`);
}
