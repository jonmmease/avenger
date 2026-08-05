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
- Affects: `bar_grouped`, `bar_grouped_repeated`, `point_offset_random`, and
  other examples represented by nested categorical position levels or
  Vega-Lite offset channels
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

### AV-GALLERY-LANG-005 — Channel domain coordination is not lowered

- Status: confirmed
- Affects: `facet_bullet` and faceted or repeated charts that require a scale
  domain per leaf plot
- Evidence: the canonical DSL defines `domain_scope: free | level(n) | shared`
  and `domain_group:` on configured channels, and resolved channel values retain
  a domain-coordination field. The compiler's `apply_channel_configs` accepts
  `scale`, `axis`, `legend`, `band`, and `domain_contribution`, but rejects
  `domain_scope` and `domain_group` as unsupported channel configuration. An
  executable `facet_bullet` attempt therefore fails with `AVENGER-LOWER-002`
  before its five independently scaled bullet rows can render.
- Required capability: lower both properties into the existing
  `ChannelValue` domain-coordination metadata, validate scope/group agreement,
  and cover per-cell, level, and shared domains through compilation,
  serialization, facet evaluation, analysis, and LSP support.

### AV-GALLERY-MARK-002 — Line and area interpolation modes

- Status: confirmed
- Affects: `line_step`, `line_monotone`, `area_horizon`, `layer_dual_axis`
- Evidence: the line mark's default-value table still mentions an internal
  `interpolate` value, but `interpolate` is absent from the mark's public
  channel schema, compiled state, scene mark, and renderer. Area marks likewise
  have no authored interpolation path, so both families always connect authored
  vertices linearly.
- Required capability: a reviewed interpolation enum carried through line and
  area Rust marks, language schemas, compiled marks, scene graph, hit testing,
  and renderers, initially covering linear, step-before/after, and monotone
  curves.

### AV-GALLERY-MARK-003 — Per-corner rectangle radii

- Status: confirmed
- Affects: `stacked_bar_count_corner_radius_mark` and other bars that round
  only their exposed end corners
- Evidence: rectangle marks expose one `corner_radius` channel, and the scene
  graph applies it uniformly to all four corners. A stacked top segment cannot
  round only its two outer corners; uniform rounding also rounds the shared
  boundary and creates visible notches against the neighboring segment.
- Required capability: per-corner rectangle radius channels (or an equivalent
  end-corner policy aware of orientation), preserved through the mark schema,
  compiled state, scene graph, GPU/vector renderers, bounds, and hit testing.

### AV-GALLERY-MARK-004 — Authored gradient mark fills

- Status: confirmed
- Affects: `area_gradient` and marks whose interior is filled by an authored
  linear or radial gradient
- Evidence: area marks expose scalar `fill` and pattern-fill channels, but the
  active language schema has no gradient definition/value. Compiled Cartesian
  areas always emit an empty scene-gradient collection; a fill can reference a
  gradient index only after some lower layer has already constructed that
  collection. Consequently the authored white-to-dark-green plot-relative
  linear gradient cannot reach the scene graph.
- Required capability: a serializable gradient value with ordered color stops,
  coordinate space, endpoints (and radial geometry where applicable), usable
  by mark fill/stroke channels and preserved through language lowering,
  grouping, scene generation, GPU/SVG rendering, legends, and hit-test/bounds
  behavior.

### AV-GALLERY-GUIDE-004 — Per-tick conditional axis styling

- Status: confirmed
- Affects: `bar_negative`, `bar_negative_horizontal_label`,
  `line_conditional_axis`, and other examples with conditional axis properties
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

### AV-GALLERY-TRANSFORM-006 — Bootstrap confidence intervals

- Status: confirmed
- Affects: `layer_point_errorbar_ci` and `layer_line_errorband_ci`
- Evidence: Vega's `ci0` and `ci1` aggregate outputs are bootstrap confidence
  intervals for the mean: the aggregate repeatedly resamples each group,
  sorts the sampled means, and selects the requested tail quantiles. Avenger's
  transform registry and its documented SQL surface expose ordinary aggregate
  statistics but no equivalent seeded bootstrap aggregate or row-resampling
  table function. Substituting `mean +/- 1.96 * standard_error` would change
  both the contract and the pinned example output.
- Required capability: a deterministic bootstrap-confidence-interval
  aggregate or transform with explicit confidence level, sample count, random
  seed, grouping, null, physical-type, cache, and hot-reload semantics, exposed
  consistently through Rust, the DSL, analysis, and the LSP.

### AV-GALLERY-DATAFLOW-001 — Sequence row generator

- Status: confirmed
- Affects: `sequence_line_fold`
- Evidence: chart data supports tables and inline rows, but has no declared
  generator relation for Vega-Lite's `{sequence: ...}` data source.
- Required capability: decide whether the canonical form is a DataFusion
  `generate_series` SQL table, an inline generator table kind, or a native
  transform, including exact endpoint and step semantics.

### AV-GALLERY-INTERACTION-001 — Host-mediated link activation

- Status: confirmed
- Affects: `point_href`
- Evidence: chart event actions can update params, stores, selections, scales,
  and cursor state, but neither the runtime action vocabulary nor the language
  schema can request that a host open an authored URL. Mark schemas likewise
  have no `href` channel. Rendering the scatterplot alone would omit the
  example's defining click behavior.
- Required capability: add an explicitly host-mediated link/open action with a
  row expression for the URL, a safe host policy and acknowledgement path, and
  language, compiler, native-window, web-host, analysis, and LSP coverage.

### AV-GALLERY-INTERACTION-002 — Authored mark tooltips

- Status: confirmed
- Affects: `arc_pie_normalize_tooltip`, `histogram_rel_freq`,
  `interactive_geo_earthquakes`, `interactive_geo_facet_species`,
  `interactive_histogram_full_height_hover`, `interactive_line_point_hover`,
  `interactive_multi_line_pivot_tooltip`, `interactive_multi_line_tooltip`,
  `parallel_coordinate`, `param_search_input`, `point_href`,
  `rect_mosaic_labelled_with_offset`, `ternary`, `trail_comet`
- Evidence: mark schemas expose neither a tooltip channel nor a tooltip
  configuration, and the runtime/eventstream has no tooltip host action or
  surface. The rendered marks and hit data can exist, but the authored field
  list, labels, formatting, and hover presentation are discarded.
- Required capability: define a typed tooltip content model over the hit row,
  a mark channel or event action that requests it, formatting and null rules,
  a host-owned presentation lifecycle, and equivalent native/web/inspector,
  language, compiler, analysis, and LSP support.

### AV-GALLERY-ASSET-001 — Project-local image assets

- Status: confirmed
- Affects: `scatter_image` and charts whose image channel references a file in
  the Avenger project
- Evidence: image coercion accepts inline PNG/SVG data URIs and HTTP(S) URLs,
  but `RgbaImage::from_str` rejects other strings as unsupported image URLs.
  The language compiler and CLI do not resolve a project-relative path into an
  image resource or bundle its bytes into the compiled chart. The pinned
  `assets/ffox.png`, `assets/gimp.png`, and `assets/7zip.png` files therefore
  cannot be referenced from the authored chart without manually embedding
  generated base64 data, which would bypass the project-asset contract under
  test.
- Required capability: define project-relative asset references with canonical
  path and module-resolution rules; load and fingerprint them through the
  compiler/watch dependency graph; and either preserve a host-resolved resource
  request or embed portable image bytes in compiled/serialized charts.

### AV-GALLERY-GEO-001 — Composite Albers USA projection

- Status: confirmed
- Affects: `geo_choropleth`, `geo_circle`, `geo_layer`, `geo_rule`,
  `geo_repeat`, `geo_text`, `geo_line`, `geo_trellis`, and
  `airport_connections`
- Evidence: every affected pinned Vega-Lite specification requests
  `albersUsa`, whose projection combines the continental United States with
  repositioned Alaska and Hawaii insets. Avenger's `albers` language atom
  lowers specifically to `Geo::albers_usa_conus()`, a single conic equal-area
  projection with the CONUS aspect. The projection model has no composite
  projection or inset routing, so using the current atom omits or misplaces
  Alaska and Hawaii and cannot reproduce the pinned references faithfully.
- Required capability: add a composite Albers USA projection that routes
  geometries and longitude/latitude points through CONUS, Alaska, and Hawaii
  components; preserves clipping and inverse-coordinate behavior; and works
  consistently for shapes, symbols, lines, tools, hit testing, serialization,
  language authoring, analysis, and LSP support. Keep the existing CONUS-only
  projection available under an unambiguous name.

### AV-GALLERY-GEO-002 — Geographic text mark

- Status: confirmed
- Affects: `geo_text` and `geo_layer_line_london`
- Evidence: the geographic coordinate registry exposes `line`, `rect`,
  `symbol`, `geo_shape`, and `uniform_raster_2d`, but not `text`. The Cartesian
  text mark cannot be placed from longitude/latitude channels inside a geo
  chart, so state-capital and borough labels cannot share the map projection
  used by their surrounding layers.
- Required capability: register and lower a geo text mark with longitude,
  latitude, projected x/y fallback, text, alignment, leader-offset, angle,
  color, size, opacity, and interaction channels, reusing the ordinary text
  renderer after geographic position projection.

### AV-GALLERY-LAYOUT-001 — Authored annotation overflow beyond the plot area

- Status: confirmed
- Affects: `layer_line_co2_concentration`, `bar_layered_weather`, and charts
  that place labels just outside a scale or plot boundary
- Evidence: Cartesian plot content is wrapped in the clip returned by
  `CompiledPlot::get_clip_region`, which defaults to the plot-area rectangle.
  Text marks expose no authored clip/overflow policy, so a left-aligned label at
  the maximum x-domain value is clipped even when the canvas reserves room to
  its right. The gallery fixture must right-align its final labels inside the
  plot instead of reproducing Vega-Lite's outward annotation placement.
- Required capability: expose a reviewed chart/group overflow policy and make
  layout measurement reserve the resulting annotation extents when requested,
  while retaining plot clipping as the safe default for data marks.

### AV-GALLERY-GUIDE-001 — Quantitative size-legend sample values

- Status: confirmed
- Affects: `point_bubble`, `circle_binned`, `circle_natural_disasters`,
  `circle_bubble_health_income`, `trail_color`, `trail_comet`,
  `circle_github_punchcard`, `point_angle_windvector`,
  `selection_translate_scatterplot_drag`, `dynamic_color_legend`,
  `interactive_seattle_weather`, `layer_likert`, `vconcat_weather`
- Evidence: size legends derive entries directly from the continuous scale's
  domain values, whether the renderer uses symbols or varying-width line
  samples. The legend schema exposes presentation properties but no explicit
  sample values or tick-generation configuration, so `point_bubble` shows only
  observed endpoints (`8` and `24.8`) instead of (`0`, `5`, `10`, `15`, `20`),
  and `trail_color` cannot produce (`0`, `100`, ..., `700`).
- Required capability: let continuous size legends generate configurable
  rounded samples independently of the scale domain, with an explicit-values
  override, stable label formatting, and renderer-appropriate sample glyphs.

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

### AV-GALLERY-GUIDE-005 — Symbol-legend item-flow orientation

- Status: confirmed
- Affects: `bar_diverging_stack_population_pyramid`, `facet_grid_bar`, and
  other compact top or bottom symbol legends
- Evidence: the standard legend schema and Rust builder retain an
  `orientation` expression, but symbol, line, and rectangle legend renderers do
  not read it. `orientation: 'horizontal'` therefore still produces a vertical
  item stack; only colorbar orientation is currently consumed by a renderer.
- Required capability: evaluate item-flow orientation in discrete legend
  renderers and use it, together with `columns`, when measuring, placing, and
  navigating legend entries at every legend position.

### AV-GALLERY-GUIDE-006 — Contextual axis-label expressions

- Status: confirmed
- Affects: `bar_month_temporal_initial`, `circle_custom_tick_labels`, and
  examples that derive tick labels from the tick value or its default
  formatted label
- Evidence: Cartesian axes expose static number/date-time format patterns and
  a scalar label angle. The `tick_label` expression is evaluated once to one
  string for the whole axis, not against each generated tick, so there is no
  per-tick label expression or custom label-value mapping. Precomputing the
  displayed label in chart data is not equivalent:
  January, June, and July all become `J` and collapse into one categorical
  scale value instead of remaining three distinct temporal ticks. The axis API
  also has no authored label-alignment property for reproducing this example's
  left-aligned labels.
- Required capability: evaluate an axis-label expression against a contextual
  tick relation containing at least the raw tick value and default formatted
  label, and carry label alignment through measurement and rendering. The DSL
  should provide typed completion and validation for the contextual fields.

### AV-GALLERY-GUIDE-007 — Facet-guide label presentation

- Status: confirmed
- Affects: `trellis_area_seattle`, `facet_bullet`, `facet_grid_bar`, and row
  facets whose compact labels must remain horizontal
- Evidence: an executable `trellis_area_seattle` attempt can precompute its
  human-readable hour labels with SQL and renders all 24 area plots, but the
  row guide rotates every value label vertically. The facet authoring schemas
  and Rust `FacetRowGuideConfig` expose only title, position, visibility, and
  slot sharing; they provide no label angle, alignment, padding, font, or
  formatting controls. The result is overlapping unreadable labels rather
  than the gallery's compact horizontal hour labels.
- Required capability: add shared facet-guide value-label presentation to the
  row, column, and wrap guide contracts, including angle, alignment, padding,
  font properties, and a contextual formatting expression; carry it through
  measurement, overflow, serialization, rendering, DSL schemas, analysis, and
  LSP support.

### AV-GALLERY-GUIDE-008 — In-plot legend positioning

- Status: confirmed
- Affects: `layer_line_window`, `geo_layer_line_london`
- Evidence: Vega-Lite's `bottom-right` orientation places a legend inside the
  plot bounds. Avenger's `LegendPosition` and standard legend schema expose
  only the four external layout edges, so the benchmark legend must occupy a
  separate bottom slab instead of the reference chart's unused in-plot area.
- Required capability: add an explicit in-plot legend placement model with
  horizontal and vertical anchors, padding, collision/clip behavior, and
  deterministic composition semantics; do not overload external edge layout.

### AV-GALLERY-GUIDE-009 — Coordinated entries for discretizing-scale legends

- Status: confirmed
- Affects: `concat_bar_scales_discretize`
- Evidence: each panel maps one quantitative field through matching color and
  size quantize, quantile, or threshold scales. Its defining output is one
  legend whose interval entries apply both output channels to every sample.
  Avenger derives symbol items from raw `domain_values`; those values do not
  represent the generated interval entries, and the merge key rejects interval
  domains entirely. As a result, quantize channels cannot merge and the number
  of labels, colors, and sizes can disagree for all three scale families.
- Required capability: let discrete-output scales publish typed legend entries
  containing the interval boundary, representative input, output value, and
  label; merge channels by identical entry partitions and source expression;
  then apply every merged channel output to each symbol deterministically.

### AV-GALLERY-SCALE-003 — Continuous diverging-scale midpoint

- Status: confirmed
- Affects: `joinaggregate_residual_graph`, `trail_comet`, and other quantitative
  diverging-color examples whose meaningful neutral value is not the extent
  midpoint
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

### AV-GALLERY-RUNTIME-003 — Full-row collection normalized exact Arrow names

- Status: resolved
- Discovered by: `repeat_histogram`
- Symptom: a concat subplot needs the complete inherited source row so it can
  evaluate each child plot. The full-row collection path rebuilt every schema
  field with DataFusion's SQL-style `col()` helper, changing the exact Parquet
  field `"Name"` to `name` before the child transform ran.
- Resolution: expressions rebuilt from an already-decoded Arrow schema now use
  exact unqualified `Column` nodes. This applies to full-row collection,
  aggregate output rewrites, and event-datum projection without changing how
  authored SQL identifiers are normalized.
- Regression evidence: the mixed-case full-data selection unit test and direct
  plus serialized evaluation of the repeated Cars histogram.

### AV-GALLERY-LANG-006 — Compound statistical marks lost planned column identity

- Status: resolved
- Discovered by: `boxplot_2D_vertical`
- Symptom: the language lowerer supplied relation-qualified, exact-case column
  expressions to the native `box_plot`. Its multi-stage aggregate branches
  rebuilt the grouping key with DataFusion's SQL-style `col()` helper, changing
  `"Species"` to `species`; after the first internal transform removed relation
  qualifiers, the still-qualified `"Body Mass (g)"` value expression also no
  longer matched the branch schema.
- Resolution: compound grouping reconstructs decoded source names with exact
  unqualified `Column` nodes. Statistical-mark language lowering removes
  planner relation qualifiers from position and style expressions while
  retaining exact Arrow field names, because native compound branches own and
  repeatedly reshape a single input relation.
- Regression evidence: exact-case `CompoundGrouping` and qualified statistical
  expression normalization unit tests, plus direct and serialized evaluation
  of the reviewed native `boxplot_2D_vertical` gallery case.

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
