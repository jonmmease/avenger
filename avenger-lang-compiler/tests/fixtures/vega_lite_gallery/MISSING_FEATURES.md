# Vega-Lite Gallery Missing Features

This is the evidence ledger for Avenger or Avenger-language capabilities that
prevent a pinned gallery example from being reproduced faithfully. It is not a
list of differences from Vega-Lite's API: an idiomatic SQL or native Avenger
equivalent is sufficient when it preserves the example's semantics.

Statuses are `confirmed`, `investigating`, `planned`, and `resolved`. A
confirmed entry must name affected examples and contain compiler/runtime or
registry evidence. Keep resolved entries so the gallery records why a native
capability was added.

## Confirmed

### AV-GALLERY-MARK-001 — Filled arc/sector mark

- Status: confirmed
- Affects: `arc_pie`, `arc_pie_normalize_tooltip`, `arc_donut`,
  `layer_arc_label`, `arc_radial`, `arc_pie_pyramid`
- Evidence: the stock polar mark registry exposes symbols, text, lines, areas,
  rules, images, trails, boxes, rasters, and groups, but no filled arc/sector
  primitive. Existing polar symbol placement cannot preserve angular and
  radial extent.
- Required capability: a native polar arc/sector mark with inner/outer radius,
  start/end angle, fill/stroke, target, and legend-compatible channels.

### AV-GALLERY-TRANSFORM-001 — Regression transform

- Status: confirmed
- Affects: `layer_point_line_regression`
- Evidence: the language registry has no regression transform. A fixed SQL
  formula would not preserve Vega-Lite's grouped regression contract.
- Required capability: reviewed regression semantics and output schema, either
  as a native transform or a reusable DataFusion implementation.

### AV-GALLERY-TRANSFORM-002 — LOESS transform

- Status: confirmed
- Affects: `layer_point_line_loess`
- Evidence: the language registry has no LOESS transform.
- Required capability: grouped LOESS with bandwidth and the expected output
  columns.

### AV-GALLERY-TRANSFORM-003 — Quantile transform

- Status: confirmed
- Affects: `point_quantile_quantile`
- Evidence: aggregate/window SQL functions do not currently expose the same
  probability-grid row generator and grouped output contract.
- Required capability: a quantile transform or an explicitly equivalent SQL
  table function.

### AV-GALLERY-TRANSFORM-004 — Flatten transform

- Status: confirmed
- Affects: examples whose manifest contains `transform:flatten`
- Evidence: the stock transform registry has fold but no array-flattening
  transform, and authored SQL has no documented array-unnest contract in the
  language specification.
- Required capability: define and test an array/list unnest contract, then use
  either SQL or a native transform.

### AV-GALLERY-DATAFLOW-001 — Sequence row generator

- Status: confirmed
- Affects: `sequence_line_fold`
- Evidence: chart data supports tables and inline rows, but has no declared
  generator relation for Vega-Lite's `{sequence: ...}` data source.
- Required capability: decide whether the canonical form is a DataFusion
  `generate_series` SQL table, an inline generator table kind, or a native
  transform, including exact endpoint and step semantics.

## Investigation queue

These families need executable conformance attempts before they can be called
missing. Add a confirmed entry only when the current implementation cannot
express the required behavior faithfully.

- nearest-point and legend-bound selections;
- global/union/intersect selection resolution across composed views;
- interval selections bound to scale domains;
- tooltip and external-link host actions;
- conditional axis and legend properties;
- independent/shared scales and guide resolution in nested composition;
- projection sphere and graticule sources;
- responsive container sizing;
- Vega-Lite interpolation and path-order variants;
- lookup and pivot equivalence through authored SQL;
- image marks whose URLs are project-local assets.

## Resolved while porting

### AV-GALLERY-LANG-001 — Double-quoted column case was lost during lowering

- Status: resolved
- Discovered by: `point_2d`
- Symptom: `encoded "Horsepower"` compiled to the unquoted DataFusion column
  `horsepower`; scale-domain discovery then produced no `x` scale for the
  Parquet schema's case-sensitive `Horsepower` field.
- Resolution: language lowering now constructs an exact unqualified
  `Column` for decoded double-quoted identifiers instead of passing the name
  through DataFusion's SQL-style `col()` helper. The same fix covers selection
  fields, view-domain shorthand, routed transform outputs, and selection
  containment.
- Regression evidence: `double_quoted_channel_columns_preserve_arrow_field_case`
  plus the compiled and rendered `point_2d` gallery case.
