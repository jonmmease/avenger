# Rust Authoring Wrappers: Chart, Subplot, Panel

## Status

Draft design, 2026-07-09. The Rust API change implied by the naming law
in `chart-dsl.md` ("Charts, Plots, And Subplots"): **`Plot<C>` becomes a
pure construct with zero position-dependent fields, and every position
furnishes it through a wrapper** — `Chart<C>` (document), `Subplot`
(concat cell), the dashboard `Panel` (panel), and `mark subplot` (its own
mark properties). Wrappers wrap the atom of their altitude: inside a
chart the atom is the plot; inside a dashboard the atom is the chart.

Sequencing: independent of the widget plan (widgets attach to `Plot`,
which is unchanged); best landed as its own small mechanical campaign
before or alongside widget phase 1, since both touch authoring
ergonomics.

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
    pub fn from_plot(plot: Plot<C>) -> Self;       // build-plot-first style
    // document furnishings
    pub fn title(...) / subtitle(...) / theme_css(...) / canvas_size(...)
           / param(...) / selection(...) / store(...);
    // forwarded Plot builder surface (~15–20 methods, macro-generated or
    // hand-written): data, mark, tool, widget, event_binding, scale/guide
    // config — each returns Chart<C> so single-plot charts stay one chain.
    pub async fn compile(self, ctx: &SessionContext)
        -> Result<CompiledChart, AvengerChartError>;   // ONLY compile entry
}
```

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
   `item_domains`/`with_repeat_domain_coordination`/…) — ~35 methods
   across 7 impls (plot.rs:158–386).

Some systems also mint runtime state from their config — `Geo` exposes
`center_x_param()`/`center_y_param()` (the pan/zoom-interactive center
and zoom are params) — which registers during compile and is unaffected
by the wrapper.

**What `Chart<C>` needs:**

- `Chart::with_coord(c: C)` — the mirror constructor, keeping the
  leaf-system idiom one chain:
  `Chart::with_coord(Geo::mercator().center_lon_lat(…).zoom(11))`.
- A **generic `coord` combinator** — `fn coord(self, f: impl FnOnce(C)
  -> C) -> Self` — one method covering *every* post-construction
  coordinate tweak with zero per-`C` duplication:
  `Chart::<GridConcat>::new().coord(|c| c.rows(2).columns(2))`. (Worth
  adding to `Plot` too for symmetry.)
- **Mirrored container sugar**: the ~35 per-`C` container methods are
  chart-root-typical, so the forwarding macro should generate them onto
  `Chart` alongside the generic surface; leaf systems stay on
  `with_coord`, and external crates may add `impl Chart<Geo>` sugar
  later (coherence permits — local type parameter).
- A read accessor (`coord_system()`), forwarded.

The lattice stays clean: the coordinate system is the plot's *essence*
(the frame), so it never moves to `Chart` — `Chart` only forwards to
it.

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

## Migration

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
  `macro_rules!` vs the `delegate` crate (leaning: local macro; keep the
  surface auditable). The macro must cover the per-`C` container impls
  (~35 methods) as well as the generic surface, or the `coord()`
  combinator becomes the only container path — decide sugar-vs-combinator
  balance.
- Does `Chart::from_plot` + `into_chart()` sugar earn its place, or is
  `Chart::new()` forwarding enough for every real construction pattern?
- Do facet/repeat charts (`Plot<Facet>`-family roots today) carry any
  root-only fields beyond the common set, and does `Chart<C>`'s generic
  need bounds per family?
- Where does `plot_size` on a *root* plot land — `Chart::plot_size`
  (sugar for `layout.plot`) or only through the layout config object?
- Timing: before widget phase 1 (cleanest — widgets' examples then use
  the final spelling) or as an independent parallel campaign (touches
  disjoint files except examples)?
