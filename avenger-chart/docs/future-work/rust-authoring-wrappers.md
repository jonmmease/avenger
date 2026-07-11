# Rust Authoring Wrappers: Chart, Subplot, Panel

## Status

Active implementation design, 2026-07-09. Campaign 1 (coordinate-forwarding
removal) was implemented in `d35c6a199` on 2026-07-11; Campaign 2 remains
planned. The Rust API change implied by the naming law
in `chart-dsl.md` ("Charts, Plots, And Subplots"): **`Plot<C>` becomes a
pure construct with zero position-dependent fields, and every position
furnishes it through a wrapper** — `Chart<C>` (document), `Subplot`
(concat cell), the dashboard `Panel` (panel), and `mark subplot` (its own
mark properties). Wrappers wrap the atom of their altitude: inside a
chart the atom is the plot; inside a dashboard the atom is the chart.

Sequencing (decided 2026-07-09): **two strictly ordered campaigns** —
the coordinate-forwarding removal lands first as a standalone,
behavior-preserving refactor, and the `Chart`/`Subplot` wrapper work
follows; see [Implementation Sequencing](#implementation-sequencing).
Both are independent of the widget plan (widgets attach to `Plot`,
which keeps its generic surface).

## The Types

```rust
/// Document wrapper: a root plot plus document furnishings.
pub struct Chart<C> {
    title: Option<TitleConfig>,
    subtitle: Option<TitleConfig>,     // exists ONLY here
    theme: Theme,
    layout: ChartLayout,               // canvas size, plot box, margins, resize policy
    locales: FormatConfig,             // time/format
    params: Vec<Param>,                // document state declarations
    selections: Vec<Selection>,
    stores: Vec<Store>,
    plot: Plot<C>,
}

impl<C> Chart<C> {
    pub fn new() -> Self;                          // wraps Plot::<C>::new()
    pub fn from_plot(plot: Plot<C>) -> Self;       // build-plot-first / reuse seam
    // document furnishings
    pub fn title(...) / subtitle(...) / theme_css(...) / canvas_size(...)
           / param(...) / selection(...) / store(...);
    // forwarded Plot daily verbs (~a dozen, hand-written): data, mark,
    // tool, widget, event_binding, configure_guide, configure_coord —
    // each returns Chart<C> so single-plot charts stay one chain.
    pub fn configure_plot(self, f: impl FnOnce(Plot<C>) -> Plot<C>) -> Self;
                                                   // escape hatch: forwarding is
                                                   // sugar, never a gate
    pub fn plot(&self) -> &Plot<C>;                // introspection/tests
    pub async fn compile(self, ctx: &SessionContext)
        -> Result<CompiledChart, AvengerChartError>;   // ONLY compile entry
}
```

**The wrapper pattern is one pattern, applied at two levels: own the
inner value, forward the daily verbs, expose `with_X`/`from_X` for
construction and `configure_X` for the rest.** `Plot` over `C`:
`with_coord` / `configure_coord`. `Chart` over `Plot`: `from_plot` /
`configure_plot`. Nothing couples a chart's furnishings to its plot's
internals (a title constrains no mark), which is why a plain escape
hatch is safe — the wrapper sorts vocabulary; it protects no invariant.
`Plot` itself remains fully first-class: it is the currency of every
nested position (cells, embedded templates, the widget chrome layer),
and altitude-agnostic component functions return `Plot` (usable via
`Chart::from_plot` *or* `Subplot::new`) while inherently-document
components return `Chart`.

`Plot<C>` keeps: `data`, `mark`, `tool`, `widget` (guide-slot chrome is
per-plot, like legends — cells may carry widgets), `event_binding`,
`configure_guide` (guide defaults are plot chrome), and the coordinate
system (below). It loses to `Chart`: `title` **and `subtitle`** (both
exist on `Plot` today via `plot/title.rs` — position-blind; the wrapper
makes subtitle chart-only), `canvas_size`/`canvas_constraint`/`margins`
(document layout), `plot_size`/`plot_constraint` at the root (become
chart layout's plot box; in cells they become `Subplot::size`),
`theme`/`get_theme`, `time_context`/`formatting_context` (the DSL's
chart-level `time:`/`format:`), and
`add_param`/`add_params`/`add_param_with_sharing`/`add_selection`/
`add_store`/`add_stores`/`cursor_param` (document state). **`compile`
goes internal, not away**: the public root entry is `Chart::compile`,
but `Plot` keeps a `pub(crate)` compile because the embedding machinery
(`SubplotChildPlotSpec::compile_boxed`, which every `Plot<C>` implements
today at plot/plot.rs:100) compiles embedded plots through it. `Chart`
deliberately does **not** implement `SubplotChildPlotSpec` — charts
cannot be embedded, which is the altitude law enforced by trait
absence.

## Coordinate System Customization

Verified against the code: the coordinate system is **plot state**
(`Plot.coord_system: C`, plot/plot.rs:44) with three customization
surfaces today —

1. **Configured construction** — `Plot::with_coord(C)` (plot.rs:134)
   with each system's own fluent builder. The leaf-system path:
   `Geo::mercator().center_lon_lat(-73.98, 40.75).zoom(11).tiles(osm)`
   (avenger-chart-geo/src/coord.rs),
   `Parallel::new().dimension_with(…).order(…)`,
   `Cartesian::new().equal_units()` (unit-aspect constraint). External
   coordinate crates ship **no** `Plot` impls — all their surface lives
   on the `C` builder.
2. **`Plot::new()`** for `C: Default` (Cartesian, Polar, containers).
3. **Per-`C` forwarding sugar on `Plot`** — only for the *container*
   systems defined in avenger-chart itself, mutating `self.coord_system`:
   `impl Plot<GridConcat>` (`rows`/`columns`/`column_widths`/
   `row_heights`/`axis_guide_visibility`), `HConcat`/`VConcat`
   (`widths`/`heights`), `WrapConcat` (`columns`/`responsive_columns`),
   and the four `Repeat*` kinds (`cell`/`cell_when`/`matrix_domains`/
   `item_domains`/`with_repeat_domain_coordination`/…) — 35 methods
   across 8 impls (plot.rs:158–386): four concat-family impls and four
   repeat-family impls.

Some systems also mint runtime state from their config — `Geo` exposes
`center_x_param()`/`center_y_param()` (the pan/zoom-interactive center
and zoom are params) — which registers during compile and is unaffected
by the wrapper.

**Adopted rework (2026-07-09): remove coordinate forwarding from `Plot`
entirely.** One rule — coordinate options live on the coordinate-system
type — with exactly two paths on both `Plot` and `Chart`:

- `with_coord(c: C)` — configured construction (already the leaf-system
  idiom): `Chart::with_coord(GridConcat::new().rows(2).columns(2))`,
  `Chart::with_coord(Geo::mercator().center_lon_lat(…).zoom(11))`.
- `configure_coord(f: impl FnOnce(C) -> C)` — the post-construction
  combinator (named beside the existing `configure_guide`):
  `Chart::<HConcat>::new().configure_coord(|c| c.widths([fr(2), px(280)]))`.

Consequences:

- **The 8 per-`C` container impl blocks on `Plot` (35 methods,
  plot.rs:158–386) are deleted.** `Plot`'s public surface becomes truly
  coordinate-agnostic — the pure-construct story completes: zero
  position fields *and* zero coordinate-specific methods.
- **`Chart` forwarding collapses** to the ~dozen generic methods plus
  the two coord paths; the mirroring question is retired.
- **Container and external coordinate crates play by the same rule** —
  the `C` builder is the entire integration surface (this is also the
  Rust-side answer to chart-dsl.md's coordinate-crate extensibility
  question), and the DSL gains one lowering rule: coordinate-kind chart
  properties lower to `C` builder calls, never to `Plot`/`Chart`
  methods.
- **Prerequisite**: the `Repeat*` types grow fluent owned-self builders
  (`.rows()`, `.columns()`, `.items()`, `.cell()`, `.cell_when()`,
  `.matrix_domains()`, …) replacing their setter-style surface
  (`set_cell`/`add_cell_when`), which existed only because the `Plot`
  sugar wrapped it. The concat types are already fluent.
- A forwarded read accessor (`coord_system()`) remains.

The lattice stays clean: the coordinate system is the plot's *essence*
(the frame), so it never moves to `Chart` — both types only construct
and thread it.

`Subplot` (the existing concat wrapper, re-scoped to cell furnishings +
the `Plot<Inner> → Mark<Outer>` coordinate-system-erasing adapter):

```rust
Subplot::new(plot)
    .name("nested")          // renamed from .key() — the `as` binder;
                             // `key` is reserved for mark subplot's
                             // data-driven grouping expression
    .label("Click a month")  // cell caption (was .title on the inner plot)
    .size(410.0, 270.0)      // cell sizing (was .plot_size on the inner plot)
    .at(0, 1).span(2)        // grid-concat placement
```

Dashboard `Panel` (from `dashboard-layer.md`) wraps a **`Chart`**, not a
plot — panels host whole documents with their titles intact; cells host
parts of one chart, where `subtitle`/`theme` are unrepresentable rather
than invalid. The wrap targets are the two-altitude law in constructor
signatures.

## Before / After

```rust
// BEFORE
let cell = Plot::<Cartesian>::new()
    .data(df).title("Detail").plot_size(310.0, 270.0).mark(m);
let root = Plot::<HConcat>::new()
    .canvas_size(920.0, 460.0).add_selection(picked)
    .mark(Subplot::new(cell).key("detail"));
let compiled = root.compile(&ctx).await?;

// AFTER
let cell = Plot::<Cartesian>::new().data(df).mark(m);
let chart = Chart::<HConcat>::new()
    .configure_coord(|c| c.widths([fr(2.0), fr(1.0)]))   // coord options: C's builder, one door
    .canvas_size(920.0, 460.0).selection(picked)
    .mark(Subplot::new(cell).name("detail").label("Detail").size(310.0, 270.0));
let compiled: CompiledChart = chart.compile(&ctx).await?;
```

Single-plot charts are a name swap at the outermost constructor
(forwarding absorbs the rest):

```rust
Chart::<Cartesian>::new().data(cars).mark(points)
    .title("Horsepower vs MPG").subtitle("1970–82")
    .compile(&ctx).await?
```

## Implementation Sequencing

Two campaigns, strictly ordered. The order is load-bearing: campaign 1
shrinks the would-be forwarding surface to ~a dozen methods *before*
`Chart` exists, so the wrapper is never published with mirrored
container sugar and nothing on `Chart` ever needs a deprecation cycle —
and campaign 1 is independently verifiable (byte-identical), halving
the blast radius of each sweep.

### Campaign 1 — remove coordinate forwarding from `Plot` (first)

Pure API refactor; zero behavior change; **byte-identical baseline
expectation** (no re-blesses).

1. **Add `configure_coord`** to `Plot<C>` —
   `pub fn configure_coord(mut self, f: impl FnOnce(C) -> C) -> Self` —
   beside `configure_guide`. (`with_coord` already exists.)
2. **Fluent builders on the `Repeat*` coordinate types**, replacing the
   setter-style surface that existed only for the `Plot` sugar to wrap
   (`set_columns`/`set_rows`/`set_cell`/`add_cell_when`/
   `set_domain_coordination`/…): owned-self `.rows()`, `.columns()`,
   `.items()`, `.responsive_columns()`, `.cell()`, `.cell_when()`,
   `.matrix_domains[_with_scope]()`, `.item_domains[_with_scope]()`,
   `.matrix_axes()`, `.axis_guide_visibility()`,
   `.with_repeat_domain_coordination()`. The concat types are already fluent —
   the forwarding bodies call them — so verify completeness only
   (`rows`/`columns`/`column_widths`/`row_heights`/`widths`/`heights`/
   `responsive_columns`/`axis_guide_visibility`).
3. **Migrate call sites** from the `Plot` sugar to `with_coord` /
   `configure_coord`: tests, examples (avenger-chart and
   avenger-chart-app), and code snippets in the architecture docs
   (concat-system.md and repeat-system.md at minimum). The inventory is
   greppable: the ~35 method names scoped to container-typed `Plot`
   receivers.
4. **Delete the eight per-`C` impl blocks** (plot.rs:158–386) and any
   setter surface on the repeat types made dead by step 2.
5. **Verify.** Use the execution plan's HEAD-regenerated package census:
   debug `RUSTFLAGS="-D warnings" cargo check --all-targets` over the chart
   packages and top-level example packages, targeted release tests per step,
   then `cargo test --release --workspace` plus
   `cargo test --release -p avenger-chart --features visual-tests` at the
   campaign gate. The former 118-test `ParquetFormat` pin was fixed by
   `f7f52ed5f`/`b35412b99`; Campaign 1 starts from a fresh failure set, and
   any new Parquet failure is a regression. Exit criteria: no
   coordinate-specific method resolves on any `Plot<C>` receiver; the fresh
   failure-set diff is clean; **zero baseline changes**.

Coordination hazard: the concurrent baking workstream actively edits
avenger-chart (including plot.rs). Sequence with it or rebase
deliberately — this campaign's plot.rs delta is pure deletion plus one
added method, so conflicts stay mechanical.

**Implementation status:** completed in `d35c6a199` (2026-07-11). The scoped
strict check, 729 visual tests, doc tests, formatting, and full release
workspace build passed with zero baseline changes. The workspace test gate
retained the preflight `avenger-svg` smoothing-snapshot failure and also
exposed an order-sensitive pre-existing chart-app temporal-coordinate test
that passes alone and with its focused family. Rust 1.96 clippy remains
blocked by existing lints outside the campaign diff.

### Campaign 2 — introduce `Chart<C>`, re-scope `Subplot` (second)

The rest of this document: the `Chart` type with its ~dozen forwarded
generics, the plot-access trio (`from_plot` / `configure_plot` /
`plot()` — the level-2 counterparts of campaign 1's `configure_coord`),
the document furnishings, the field moves (title/subtitle,
canvas/margins/constraints, locales, state declarations, `compile`
going `pub(crate)` on `Plot`), `Subplot::name`/`label`/`size`/`at`, and
the staged artifact renames. Sweep mechanics below. Its timing relative
to widget phase 1 is the open question at the end; campaign 1 has no
such dependency and can land immediately.

## Migration

Campaign 2's sweep (campaign 1's migration is item 3 above):

- **Pure name swap** for every site using no root-only methods:
  `Plot::<C>::new()` → `Chart::<C>::new()` at the outermost constructor.
- **Semantic edits only** where concat cells carried `.title`/`.plot_size`
  — move to `Subplot::label`/`::size`; `.key(...)` → `.name(...)`.
- `Plot::compile` survives as a deprecated shim (wraps a default `Chart`)
  for the test-suite sweep, then deletes.
- Artifact/session renames staged: `CompiledChart` and `ChartSession` are
  the targets; `CompiledPlot`/`PlotSession` acceptable transitionally
  (the wrapper's furnishings compile into the artifact either way).
  `chart_avenger_app` / `ChartApp` keep their names and finally match.
- Dashboard docs already conform: component functions return `Chart`,
  `Panel::chart(...)` hosts them.
- The widget chrome layer stays `Plot<PixelFrame>` — a plot deliberately
  not a chart; no `Chart` wrapper is ever constructed for it.

## Open Questions

- Forwarding implementation: hand-written delegation vs a local
  `macro_rules!` vs the `delegate` crate (leaning: hand-written; with the
  coordinate rework the surface is ~a dozen methods — small enough to
  audit without a macro).
- Does `Chart::from_plot` + `into_chart()` sugar earn its place, or is
  `Chart::new()` forwarding enough for every real construction pattern?
- Do facet/repeat charts (`Plot<Facet>`-family roots today) carry any
  root-only fields beyond the common set, and does `Chart<C>`'s generic
  need bounds per family?
- Where does `plot_size` on a *root* plot land — `Chart::plot_size`
  (sugar for `layout.plot`) or only through the layout config object?
- Campaign 2 timing: before widget phase 1 (cleanest — widget examples
  then use the final spelling) or as an independent parallel campaign
  (touches disjoint files except examples)? Campaign 1 is decided-first
  and unblocked.
