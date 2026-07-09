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
scale/guide configuration. It loses: `title`, `plot_size`,
`canvas_size`, `add_param`/`add_selection`, `compile`.

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
  surface auditable).
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
