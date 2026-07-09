# Avenger Chart Future Work

This directory contains current design notes for features that are not yet part
of the architecture reference. The canonical description of the implemented
system is in [../architecture/README.md](../architecture/README.md).

Each note answers the same review questions:

- Is the goal still valid for the current project?
- What parts are already implemented in another form?
- Which direction fits the current crate, coordinate, mark, scale, guide, and
  child-frame architecture?
- Which alternate paradigms are plausible?
- How close is the topic to an implementation plan?
- Which major decisions still need to be made?

The library is not public yet, so these notes may propose breaking changes.
They should not preserve compatibility with older sketches when a cleaner
current design is available.

## Implemented Or Partially Completed

These notes describe features where a v1, partial implementation, or adjacent
architecture already exists. They remain here because follow-up design work is
still useful.

| Document | Current state |
| --- | --- |
| [mark-effects.md](mark-effects.md) | Implemented v1 surface plus remaining follow-ups for richer mark effects, derived outputs, and compound-mark encapsulation. |
| [geo-coordinate-system.md](geo-coordinate-system.md) | Implemented: the `avenger-geo` projection engine (d3-geo parity) and the `avenger-chart-geo` coordinate system — graticule/sphere guides, WKB-backed GeoJSON marks, great-circle lines, pan/zoom with the adaptive Web Mercator blend, warped raster tile layers, and the retirement of `avenger-chart-webmercator` at ≥0.9999 pixel parity. |
| [geo-raster-marks.md](geo-raster-marks.md) | Phases 1–2 implemented (2026-07-04): CRS-tagged rasters (`geometry.crs` + `Rasterize2D::frame`), `UniformRaster2D<Geo>` with identity/warped dispatch via the generalized `warped_raster_mesh`, `crs` conversion helpers, f64 view-domain params, and four examples (taxi mercator/albers, 4326 climate grid, georegistered overlay). Remaining: Phase 3 GPU residency for large inline rasters; faceted-raster example. |
| [polar-geometry-space.md](polar-geometry-space.md) | Implemented v1 pieces exist; remaining work is mostly coordinate-vs-display geometry polish for Polar line and text marks. |
| [polar-line-implementation-plan.md](polar-line-implementation-plan.md) | Phase checklist for `Line<Polar>` and the `geometry_space` option; several implementation phases have landed and remaining phases are tracked in the note. |
| [transform-system.md](transform-system.md) | Core transform system and several transforms exist; remaining work covers additional built-ins, time/bin refinements, pushdown, caching, and user docs. |
| [tools.md](tools.md) | Pan/scroll-zoom, box-zoom, point-selection, and lasso-selection tools exist; richer editable selection chrome and toolbar composition remain future work. |
| [text-mark.md](text-mark.md) | `Text<Cartesian>` and fixed label placement exist; remaining work is richer label placement, polar text, and text-specific legends. |
| [faceting.md](faceting.md) | Facet row/column/wrap and mark data scope are implemented; remaining work is mostly `FacetGrid` sugar and polish. |
| [layout.md](layout.md) | Concat covers some composition needs; arbitrary dashboard composition remains a separate design spike. |
| [map-tiles.md](map-tiles.md) | Implemented and relocated: `avenger-chart-webmercator` was retired into `Geo::mercator()`; tiles live in `avenger-chart-geo` (XYZ grid, warped/identity rendering, attribution, fallback policies, prefetch scheduling, persistent GPU tile textures). The crate-structure sections of the note are stale. |
| [hierarchical-coords.md](hierarchical-coords.md) | `avenger-chart-treemap` exists; sunburst/icicle and shared hierarchy abstractions remain future work. |

## Not Yet Implemented

These notes describe valid goals with no dedicated implementation in the current
chart stack.

| Document | Review status |
| --- | --- |
| [typst-math-typesetting.md](typst-math-typesetting.md) | Ready for staged implementation planning; keep cosmic for regular text, use optional `avenger-typst-label` for `$...$` math spans, start with metrics/paths/raster/PDF glyph data and path-based SVG/PDF output. |
| [chart-dsl.md](chart-dsl.md) | The single consolidated DSL reference (the former API/baseline audit and missing-syntax proposals are folded in): block-structured language with SQL expression slots, `avenger 1;` version pragma, SQL string semantics with mandatory double-quoted data columns (bare names are DSL-space), `value` as the only unscaled spelling, reserved helper functions (`channel(x)`, `datum('id')`) instead of sigils, query-only `sql:`, full feature-surface syntax (chrome/layout, composition, facets, repeat, subplots, compound marks, effects, themes, patterns, views/raster, geo, parallel, interactions), cross-file reuse via one-definition-per-file imports (chart files plus mark/tool/transform definition files; an import binds exactly one name; themes are plain CSS files pinned like imports; `.data.avenger` data catalogs bind iceberg/delta/object-store tables and inline `source tables` namespaces with `.env`-backed credentials, importable as pinned single-mount dataset packs, and define catalog-level SQL views as `table sql` — logical by default with opt-in per-session materialization, parameterizable via defaulted `param` declarations called as table functions, chainable over any table kind) + parameterized `define` with channel parameters, compound marks and tools shipped as an in-language standard library of per-definition files over a primitive Rust core (`import 'std:marks/box_plot';`), custom transforms via the primitive `transform sql` plus importable `define transform` pipelines with `output` handles, a closed extension toolkit for definitions (`match` over enum slots, block slots with splice points and `exposes`, function-name slot values, channel parameters, `export` across nesting) with rejected shapes recorded, registry-free distribution (uniform imports in every file, inline `sha256` integrity Merkle-pinning transitive closures with no version resolution and no lockfile, optional bundling), lowercase kind names, the EBNF grammar, the six-node generic AST with its canonical JSON interchange encoding (specs produced and consumed without the Rust library; frozen core JSON Schema plus generated full JSON Schema), and the coverage/validation plan over all 96 baseline categories. |
| [python-chart-api-spec.md](python-chart-api-spec.md) | Standalone specification of the Python authoring syntax: three constructs (declarations, `with` blocks, accessors with the read/call/chain convention), flattened per-mark channel keywords, mark/effect/compound blocks, the aliased-expression rule, full-surface examples, tooling conformance rules, the construct reference table, and the compilation target — the DSL AST via native bindings with canonical `.avenger` printing and JSON interchange, never text generation. |
| [physical-plan-evaluation-cache.md](physical-plan-evaluation-cache.md) | Ready for design spike; proposes `avenger-datafusion-cache` (a workspace crate with DataFusion-only dependencies), a physical-plan-only DataFusion result cache using subtree fingerprints, `CacheReadExec`, tee-style `CacheWriteExec` with single-flight pending entries, and compile-time cache-boundary hints for shared chains, serving interactive params, stores, dashboards, and hot reload (scoped: hits still pay planning; derived-artifact survival is a companion workstream). Runtime-mutated subtrees (dynamic filters) are excluded from caching; the cache lives at `SessionContext` scope so it survives `PlotSession` recreation; includes a per-cache disposition table for existing Avenger data caches and a kill-switch + on/off byte-identity census rollout plan. |
| [client-server-architecture.md](client-server-architecture.md) | Ready for design spike; proposes a Wasm `PlotSession` with local interaction/rendering, remote schema-known DataFusion tables, federation-style logical-plan pushdown over HTTP, and Arrow IPC record-batch results for `Rasterize2D`, histograms, scalar aggregates, and previews. |
| [logical-plan-partial-evaluation.md](logical-plan-partial-evaluation.md) | Direction note; proposes `avenger-datafusion-partial-eval` (sibling crate to `avenger-datafusion-cache`, DataFusion-only deps), a logical-plan partial evaluator that executes placeholder-free deterministic subtrees and splices the results back as `MemTable` scans, plus a thin `CompiledPlot::bake` pass — server-side baking that keeps charts fully parameterized (optional explicit param-fixing), with as-of source-version stamping, inline vs manifest Arrow IPC embedding, and budget-guarded fold frontiers; prerequisite is compile-time lowering of SQL transforms into data-context plans; composes with the physical cache (runtime) and client-server pushdown as the third, build-time-only-server deployment mode. |
| [transform-lowering.md](transform-lowering.md) | Direction note; defines the two supported transform execution shapes — lowered (plan-pure stages collapsed via derived-only expansion into printable `sql` stages, chart-to-chart, with a stage-level scope option lowered to grouping at compile time) and plan breaks (execute-input transforms, permanently supported but allowed to be less efficient) — with a lowering pipeline of deterministic compiler passes (explicitly not a cost-based optimizer), a four-class taxonomy (11 of today's transforms already plan-pure; class 2 = data-dependent-constant breaks like `kde`/`bin` that join by being refactored into class 1 with scalar subqueries — no dual implementations), chart-level before/after examples, the `Sql` transform primitive (reserved `input` relation, lowered by construction), the plan-break contract (determinism+version, stable fingerprint-derived output identity, declared dependencies) that keeps the physical cache and hot reload working across breaks, and the DSL LSP expand-to-sql editor action (spec-level `expand(schema, registry)` entry point plus a declared execution shape on transforms). Expansion and pre-evaluation are class-1-only by design; folding across param-free breaks is a noted later extension. Prerequisite for [logical-plan-partial-evaluation.md](logical-plan-partial-evaluation.md). |
| [view-domain-inference-implementation-plan.md](view-domain-inference-implementation-plan.md) | Ready for implementation plan; moves domain inference control to channels and sketches standalone View transforms with retained View results. |
| [raster-arrow-representations.md](raster-arrow-representations.md) | Uniform + categorical kinds implemented (`Rasterize2D` output / `UniformRaster2D` input); rectilinear/quadmesh/cellmesh and the CRS metadata remain future work (the CRS field is now specced in [geo-raster-marks.md](geo-raster-marks.md)). Original scope: Arrow struct representations for uniform, rectilinear, quadmesh, and future cellmesh rasters. |
| [rasterize-uniform-2d-udaf.md](rasterize-uniform-2d-udaf.md) | Implemented as `Rasterize2D` in `avenger-chart-transforms` (dense-grid UDAF, count/sum/min/max/mean/var/stddev, partitioning, extent inference). Originally planned a non-materializing `RasterizeUniform2D` transform implemented with a custom DataFusion UDAF. |
| [async-rasterized-marks.md](async-rasterized-marks.md) | Implemented: `Rasterize2D` + `UniformRaster2D<Cartesian>` + the generic materialization substrate with RetargetCached previews (taxi examples); the proposed external `avenger-chart-rasterize` crate was not pursued (machinery lives in transforms/marks/cartesian). Geo rasters are specced in [geo-raster-marks.md](geo-raster-marks.md). Originally: external Datashader-style rasterized mark crates need generic materialization/resource primitives. |
| [async-m4-lines.md](async-m4-lines.md) | Ready for design spike; external M4 line-downsampling mark crates need generic view-dependent materialized data primitives. |
| [categorical-raster-coloring.md](categorical-raster-coloring.md) | Implemented (2026-07-05): `Rasterize2D::by(...)` categorical planes, `fill_by`/`opacity_by_total` Oklab overlay mode, categorical fill inference, swatch legends (plus the view-scoped legend collection fix), and the taxi-by-passenger capstone baselines/example. Deferred: eq_hist opacity via QuantileScale, signed-aggregate baselines, GPU mixing. |
| [multi-dim-coords.md](multi-dim-coords.md) | Ready for design spike; requires a repeated or indexed channel model. |
| [sankey-coords.md](sankey-coords.md) | Valid goal; likely a graph-layout coordinate/mark family, but other paradigms remain plausible. |
| [pattern-fill-requirements.md](pattern-fill-requirements.md) | Draft requirements for pattern fill overlays, including scenegraph/chart integration, CSS theme parsing, legend behavior, and renderer-facing constraints. |
| [number-formatting.md](number-formatting.md) | Master formatting plan; creates `avenger-format-number`, owns built-in locales and locale registration, defines the layered d3-style string plus override/context API, and adds CLDR-backed `S`, `L`, and `C[ISO]` types. |
| [datetime-formatting.md](datetime-formatting.md) | Master datetime formatting plan; creates `avenger-format-datetime`, uses LDML patterns plus `{datetime:medium}`-style CLDR presets, adds custom serde locales, `#datefmt`, and temporal axis tick-label fragments. |

## Readiness Scale

- **Ready for implementation plan**: the architecture boundary is clear and
  the next work can be chunked into code changes.
- **Ready for design spike**: the goal is valid, but one or two concrete
  design decisions should be resolved with prototypes or small experiments.
- **Discovery first**: the goal is valid, but the model still competes with
  other paradigms or depends on missing lower-level contracts.
