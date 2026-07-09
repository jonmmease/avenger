# Transform System Future Work

The current data-transform architecture is documented in
[`../architecture/data-transforms.md`](../architecture/data-transforms.md).
This page tracks remaining work and possible extensions.

## Remaining Built-In Transforms

The early transform set covers the common row-preserving, grouping, binning,
stacking, lumping, and temporal cases. Useful remaining Vega/Vega-Lite-style
transforms include:

- `Flatten`: explode list/array columns into one row per element.
- `Sequence`: generate synthetic rows from start/stop/step expressions.
- `Pivot`: turn key/value pairs into wide columns.
- `Quantile`: compute quantile samples for distribution summaries.
- `Collect`: sort rows when an explicitly ordered table is needed.

`Collect` needs the most care because SQL/DataFusion tables do not have a
stable implicit input order. Prefer an explicit sort expression or an explicit
row-order column over emulating Vega's input-order behavior invisibly.

## Time Units

`TimeUnit` has the current foundation: explicit units, lazy auto-unit
selection, week-start context, cyclical units, temporal tick spacing, and plot
time context propagation.

Remaining work:

- Full local-timezone truncation semantics for timezone-aware timestamps,
  including daylight-saving boundaries.
- Axis label format defaults for each time unit and cyclical time unit.
- Additional auto-unit candidates such as subsecond units and multi-year
  intervals if user demand appears.
- More visual baselines for week-start and cyclical month/week behavior.

## Bin Refinements

`Bin` supports nice numeric bins, exact bins, transform sharing scopes, derived
domain/tick scalars, and start/end/index output handles.

Remaining work:

- More exhaustive option coverage against Vega's bin controls.
- Additional tests for parameterized bin option combinations.
- SQL/pushdown analysis for the nice-bin scalar plan when targeting external
  database systems.

## Database Pushdown

Transform implementations currently operate on DataFusion `DataFrame`s. Future
work should preserve a path toward external database execution:

- Prefer DataFusion expressions, scalar subqueries, joins, projections,
  aggregates, and windows over Rust-only UDFs.
- Isolate any unavoidable UDF use so external SQL backends can reject or
  substitute those pieces clearly.
- Keep transform state serializable through typetag compiled transforms.

[transform-lowering.md](transform-lowering.md) develops this direction in
full: a lowering pipeline that splices plan-expressible transforms into one
plan per data context, and a plan-break contract for transforms that must
execute their input.

## Performance Caching

Transform sharing scopes can cause the same owner-scope transformed table to be
useful for multiple facet cells. The first implementation prioritizes clear
semantics and validation; future optimization can cache transformed owner-scope
tables and derived scalar maps inside `PlotSession`.

Good cache keys will need:

- compiled transform stage identity,
- owner facet path,
- data/param dependency versions,
- transform scope,
- time context.

## User Documentation

The architecture docs describe internals. User-facing docs still need compact
examples for:

- histograms from `Rect + Bin + count()`;
- stacked bars from `Aggregate + Stack`;
- top-n categorical plots from `Lump`;
- time-unit bar/line charts;
- table-shape transforms such as `Fold`, `Calculate`, `Filter`, and `Window`.
