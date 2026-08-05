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

### AV-GALLERY-LANG-004 — Nested categorical position authoring is not lowered

- Status: confirmed
- Affects: `bar_grouped` and other examples represented by nested categorical
  position levels or Vega-Lite offset channels
- Evidence: the canonical DSL specifies `nested([...])` with ordered `level`
  declarations, and the Rust chart API implements `NestedBandSpec`, per-level
  padding/axes/domain coordination, and level boundaries. The language resolver
  currently rejects every channel child other than `when`, normalization has no
  semantic field for level declarations, and the compiler has no structural
  lowering for `nested(...)` or its level configuration.
- Required capability: retain validated ordered `level <index>` declarations
  in `ResolvedChannelValue`; lower `nested([...])` through the core nested-band
  constructor; lower level padding, axes, ordering, domain/nest scopes, labels,
  and boundaries; then cover exact-column case, formatting, analysis, LSP, and
  direct/serialized runtime evaluation.

### AV-GALLERY-MARK-002 — Line interpolation modes

- Status: confirmed
- Affects: `line_step`, `line_monotone`
- Evidence: the line mark's default-value table still mentions an internal
  `interpolate` value, but `interpolate` is absent from the mark's public
  channel schema, compiled state, scene mark, and renderer. Lines therefore
  always connect authored vertices linearly.
- Required capability: a reviewed interpolation enum carried through the Rust
  mark, language schema, compiled mark, scene graph, hit testing, and renderer,
  initially covering linear, step-before/after, and monotone curves.

### AV-GALLERY-GUIDE-004 — Per-tick conditional axis styling

- Status: confirmed
- Affects: `bar_negative` and other examples with conditional axis properties
- Evidence: Cartesian axis authoring exposes one `grid` expression, while
  grid color and width come from a single theme value. Standard guide
  generation emits all grid ticks in one rule mark with scalar stroke and
  stroke-width channels, so a zero tick cannot be styled independently.
- Required capability: channel-valued axis tick/grid presentation evaluated in
  a contextual tick relation, including at least tick value and formatted
  label, with corresponding DSL condition support.

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

### AV-GALLERY-GUIDE-001 — Quantitative symbol-legend sample values

- Status: confirmed
- Affects: `point_bubble`
- Evidence: size legends derive entries directly from the continuous scale's
  domain values. The legend schema exposes presentation properties but no
  explicit sample values or tick-generation configuration, so the current
  chart shows only the observed endpoints (`8` and `24.8`) instead of the
  Vega-Lite reference's rounded sequence (`0`, `5`, `10`, `15`, `20`).
- Required capability: let continuous symbol legends generate configurable
  rounded samples independently of the scale domain, with an explicit-values
  override and stable label formatting.

### AV-GALLERY-GUIDE-002 — Legends for text-mark color channels

- Status: confirmed
- Affects: `text_scatterplot_colored`
- Evidence: the text mark accepts scaled `color` channels and the compiler
  retains the authored legend configuration, but `CompiledText` always returns
  no preferred legend renderer. The legend therefore has no contributing mark
  and is not rendered.
- Required capability: select the symbol legend renderer for discrete text
  color scales and the colorbar renderer for continuous text color scales,
  with a representative text or symbol glyph policy.

### AV-GALLERY-SCALE-003 — Continuous diverging-scale midpoint

- Status: confirmed
- Affects: `joinaggregate_residual_graph` and other quantitative diverging-color
  examples whose meaningful neutral value is not the extent midpoint
- Evidence: continuous linear scales expose a two-value interval domain and a
  color range, but no semantic domain midpoint or piecewise numeric domain.
  An explicit symmetric interval can center zero only by extending one side of
  the observed extent, which changes color sensitivity and legend endpoints.
- Required capability: add an optional continuous-scale midpoint that maps to
  the center of a diverging range while preserving independently inferred or
  configured lower and upper domain bounds.

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

### AV-GALLERY-LANG-003 — Continuous CSS-color ranges had the wrong UDF type

- Status: resolved
- Discovered by: `joinaggregate_residual_graph`
- Symptom: a literal CSS-color `range:` on a continuous scale was lowered as a
  discrete UTF-8 range. The runtime color interpolator correctly produced RGBA
  list values, but the DataFusion scale UDF still declared a UTF-8 return type
  and rejected the resulting array.
- Resolution: continuous scale lowering recognizes nonempty literal CSS-color
  arrays and stores them as native color ranges. Numeric ranges and categorical
  string ranges retain their existing representations.
- Regression evidence:
  `continuous_literal_css_color_range_lowers_as_colors` plus direct and
  serialized evaluation of the reviewed `joinaggregate_residual_graph` case.

### AV-GALLERY-SCALE-002 — Scale ordering lost exact Arrow field case

- Status: resolved
- Discovered by: `tick_strip`
- Symptom: a categorical scale retained its planned mixed-case `order_by`
  expression, but the runtime projection rebuilt referenced fields with
  DataFusion's SQL-style `col()` helper. A quoted field such as `"Cylinders"`
  consequently became the nonexistent lowercase field `cylinders`.
- Resolution: ordinary and nested-band ordering projections now reconstruct
  already-decoded column nodes directly, preserving exact Arrow field names.
- Regression evidence:
  `categorical_order_by_preserves_exact_arrow_field_case` plus the reviewed
  `tick_strip` gallery rendering.

### AV-GALLERY-GUIDE-003 — Standard axes ignored `label_angle`

- Status: resolved
- Discovered by: `bar_color_disabled_scale`
- Symptom: the DSL resolver and native axis lowerer retained `label_angle`, but
  standard band, point, numeric, and temporal axis renderers always emitted
  tick-label scene marks with angle zero. Only nested-band axes used the
  configured value.
- Resolution: the shared band and numeric tick-label renderers now propagate
  the configured angle. Point axes delegate to the band renderer, and temporal
  axes use the numeric renderer.
- Regression evidence: focused band and numeric axis tests plus the reviewed
  vertically labeled `bar_color_disabled_scale` gallery rendering.

### AV-GALLERY-SCALE-001 — Mixed Arrow temporal types collapsed mark positions

- Status: resolved
- Discovered by: `line`
- Symptom: time-scale domain inference normalized `Date32` extents to `Date64`
  epoch milliseconds, but scale application decoded the original `Date32` mark
  values with the domain's `Date64` handler. All dates consequently landed at
  the same off-canvas coordinate even though the inferred axes were correct.
- Resolution: time scales now decode configured domains and runtime values with
  independent temporal handlers. Temporal domain inference also normalizes
  timestamps in seconds, milliseconds, microseconds, and nanoseconds to epoch
  milliseconds before constructing a shared domain.
- Regression evidence: mixed `Date64`-domain/`Date32`-value and
  `Date64`-domain/`TimestampSecond` scale tests, temporal extent unit tests, and
  the reviewed `line` gallery rendering.

### AV-GALLERY-LANG-002 — Position-channel `band:` was not lowered

- Status: resolved
- Discovered by: `bar`
- Symptom: the active DSL contract and resolver accepted `band:`, and the Rust
  channel API supported row-wise band boundaries, but compiler channel
  configuration lowering rejected the property.
- Resolution: configured channel lowering now maps `band: <expr>` through the
  ordinary SQL-expression path to `ChannelValue::band`.
- Regression evidence:
  `configured_channel_band_reaches_the_compiled_position_boundary` plus the
  reviewed `bar` gallery rendering with paired `x`/`x2` boundaries.

### AV-GALLERY-TRANSFORM-005 — SQL transform dropped expression planners

- Status: resolved
- Discovered by: `text_scatterplot_colored`
- Symptom: DataFusion rejected `substr("Origin", 1, 1)` even though the
  workspace enables `unicode_expressions`.
- Resolution: the SQL transform's custom `ContextProvider` now forwards the
  active `SessionState` expression planners, matching DataFusion's own context
  adapter.
- Regression evidence: `projection_uses_session_expression_planners` executes
  the substring projection and asserts its `Utf8View` output.

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
