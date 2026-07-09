# The Dashboard Layer

## Status

Distilled design, 2026-07-09. The dashboard layer: a document format one
level above charts that imports and composes them, adds widgets that drive
param values, and keeps SQL as the expression language. This document
carries the parts not covered by its siblings — the component and
binding model, the engine representation, the runtime, and dashboard-side
phasing. The DSL surface itself lives in chart-dsl.md's Dashboards
section (the single language reference; moved there 2026-07-09). `widgets.md` owns the widget
paradigm; `dashboard-layout.md` owns the layout model and its
avenger-layout reuse. The full exploration record with every dated
reversal (the egui direction explored and dropped, the layout evolution,
the Slint deep-dive) is preserved at
`archived/dashboard-layer-exploration.md`.

**Sequencing: the full widget implementation (widgets.md, all five
phases) precedes the dashboard implementation.** The two plans are
independent; their only designed rendezvous is the dashboard's
widget-integration phase, which consumes widgets' shared types (size
hints), chrome-plot hosting, and the widget set as ready-made.

## Verdict And Framing

A dashboard coordinator needs two primitives: runtime values that flow
between independent panels and rebind cheaply, and shared relational state
many panels read. The system already runs on both — `param` (runtime
placeholders over stable plans; rebinding without re-planning — the
two-mechanism law) and the data catalog (`table sql` views,
`materialize: session`, ambient names). The dashboard layer is therefore
mostly *scope and surface*, not new machinery.

The one-line framing: **a dashboard is to charts what a chart is to
marks** — and the composition boundary lands on `param`, the runtime side
of the two-mechanism law, so composing charts needs none of the
`define`/slot expansion machinery.

The receipts (each verified in-repo):

- **Charts are already components.** Declared params with defaults are a
  typed interface; chart files are standalone-renderable with baseline
  PNGs. Nothing needs to be added to a chart to make it embeddable.
  chart-dsl.md's open question "should whole charts become importable as
  cells?" is answered here.
- **The runtime anticipates it.** `physical-plan-evaluation-cache.md`:
  "one context hosting several charts shares one cache. That is desirable
  for dashboards." The `SessionContext` (shared) / `PlotSession`
  (per-chart) split is the dashboard shape; `ChartAppState::set_param`
  already exists as the external param-write surface.
- **Cross-filtering already works in-engine** across concat subplots via
  `Selection::predicate()`
  (avenger-chart-app/examples/temporal_nested_cross_filter.rs); the
  dashboard layer moves that coordination from one `CompiledPlot` to N
  sessions sharing a state scope.
- **Param changes are cheap by design**: plans hold placeholders and
  rebind at execution; partial evaluation folds param-independent
  subtrees; the shared physical-plan cache serves common subtrees across
  charts.

The scope fence: **dashboards are documents, apps are hosts.** Hash-
pinnable, `avenger test`-able against baseline PNGs, renderable to
PNG/PDF, agent-authorable, capability-sandboxed. No loops, no imperative
blocks; side effects go through declared host `callback`s. The
conditional budget stays the language's own: params for runtime values,
SQL `CASE` for expression logic, visibility for runtime showing/hiding.

## The Component Model

**A chart's public interface is its declared state**: params (name +
default = schema), named selections and stores, and named marks (event
paths). Instantiating a chart binds that interface; unbound params keep
their defaults, so every chart remains standalone-renderable.

**Two binding forms, mirroring Slint's two arrows with a stronger
guarantee:**

- **Bare `$dash_param` — aliasing (two-way).** The chart param and the
  dashboard param share a cell; chart-internal `set param` writes
  propagate up. Two charts aliasing their `x_domain` params to one
  dashboard param get linked zoom with zero new concepts.
- **A SQL expression — derived (one-way).** The chart param follows the
  expression over dashboard params. Because `set param` actions are
  declared syntax, a chart tool writing a derived-bound param is a
  *compile-time* diagnostic (Slint's equivalent — a host `set()` silently
  breaking a binding — is a documented footgun this design closes).

All three state families bind uniformly (param / selection / store);
selection aliasing is the cross-filter wiring. The scope ladder extends
the existing `CoordinationScope` hierarchy one level above chart-`Shared`;
`ScopedParamStore` is already hierarchical.

**Hygiene:** dashboard-declared state is document state — serialized,
deep-linkable, surviving hot reload by name, swept in tests. Everything a
chart or tool mints internally is panel-scoped (instance-namespaced by
panel key, the `generated_tool_name` convention), so instantiating the
same chart twice cannot cross-link accidentally. Linking is always an
explicit act.

**Ephemeral vs document state stays crisp** (the Slint lesson): combo-open,
text cursors, staged form input live in the runtime, never the document.
Per widget, which states are params is a design decision (sidebar
collapsed: param; dropdown open: ephemeral).

**Mechanism economy** (the deepest Slint lesson): everything dynamic is a
param or a query. Tabs, modals, collapse state, and layout knobs
(`sidebar { visible: $show_controls; }`) all route through the param bus —
never a third channel. Tabs declare an implicit active-tab param
(rendering as driver chrome); a drill-through modal is
`visible: $detail_id is not null` with a chart click setting
`$detail_id`. Both are sugar, not semantics.

## The Document Surface

**The syntax moved to chart-dsl.md's [Dashboards] section (2026-07-09)**
— the single language reference is normative for the file kind, chart
imports/instantiation, state-binding forms, layout declarations, and the
widget/tabs/callback surface, including the full `exec_overview` sketch.
This document remains normative for what the syntax *means*: the
component model above, the engine representation below, and the runtime.
One law restated because everything hangs off it: **concat composes
aligned plots; a dashboard composes independent panels** — coordinated
scales below the line, coordinated state above it.

Design notes that travel with the semantics rather than the syntax:

- **Data:** charts consume the ambient catalog; a dashboard may declare
  `table sql` views (shared derived relations, `materialize: session`
  for compute-once-feed-many) and import dataset packs. Widget item
  relations, extents, and KPI text are ordinary SQL; their reactive
  execution rides the async-scalar machinery
  (scratch/async-view-scalars-plan.md).
- **Theming:** dashboard CSS cascades to charts that don't declare their
  own; precedence across the boundary is an open question below.
- **Callbacks:** host-implemented actions declared in the document — the
  imperative escape hatch that keeps the language query-only (Slint's
  business-logic split).

## Engine Representation

**A dashboard is not a `Plot`.** Plot's contract is precisely what
dashboards break apart: one scale/guide-coordination universe (dashboards
coordinate state, not scales); one `compile()` → `CompiledPlot` →
`evaluate()` as the unit of serialization, baking, caching, and
client-server exchange (dashboards want N such units with independent
lifecycles); one state-registration scope (dashboard scope sits above
plot-`Shared`); a fills-imposed-canvas contract (a document has growing
height and a viewport). No intermediate `Plot<DocFlow>` kind either — the
v0 altitude is the ordinary concat plot, and the next rung is the
app-level container.

The pair, mirroring `CompiledPlot` / `PlotSession`:

- **`CompiledDashboard`** (artifact): the document layout tree, chart
  instance references (each an ordinary `CompiledPlot`), widget specs,
  and state bindings — the `.dashboard.avenger` compile target,
  serializable with per-chart baking intact.
- **`DashboardState`** (driver): implements `SceneGraphBuilder<State>` +
  `EventStreamHandler<State>` — the avenger-app extension traits built
  for exactly this genericity (`ChartAppState` is the same pattern for
  one session). It owns the N per-chart `PlotSession`s, one chrome
  `Plot<PixelFrame>` hosting every widget as a hygienically-named group
  in a child frame, the authoritative param/selection bus (per-chart
  scoped stores alias into it), and the document viewport (scroll).

**Plots are the only renderable atoms** — charts are Plots, the widget
layer is a Plot — the dashboard arranges and coordinates but never
renders itself. That is "coordinated state, not coordinated scales"
stated in the type system.

The interaction loop:

```
widget/tool event → set param on the bus
  → static $-graph gives the dirty set (charts, scalar bindings)
  → each dirty PlotSession evaluates (Preview if raw-domain-only, else Exact)
  → shared SessionContext cache serves param-independent subtrees across charts
  → scene composition: each session's scene as a translated group, one rtree
  → document layout unchanged (closed-form; see dashboard-layout.md)
```

Evaluation scheduling: lazy for hidden panels (inactive tabs evaluate on
first reveal, state retained), visible-first priority keyed by scroll
position — mapping onto the client-server protocol's `priority` field.
Client-server: the dashboard is the served unit — one catalog manifest, N
plan-fragment streams against a stateless server; baking
(`partial_evaluate`) folds param-independent work across all charts at
publish time; the shared cache's memory budget becomes a cross-chart
resource (as the cache doc anticipates).

## Rust Authoring API

Rust needs no binding syntax: **a component is a function, and aliasing
is handle sharing.**

```rust
fn revenue_trend(
    orders: DataFrame,
    date_lo: &Param,
    date_hi: &Param,
    picked: &Selection,
) -> Plot<Cartesian> { /* ordinary plot construction */ }

async fn build(ctx: Arc<SessionContext>) -> Result<CompiledDashboard, AvengerChartError> {
    let orders  = ctx.table("orders").await?;
    let region  = Param::utf8("region").default("all");
    let date_lo = Param::date("date_lo").default("2026-01-01");
    let date_hi = Param::date("date_hi").default("2026-12-31");
    let picked  = Selection::new("picked_categories").empty_selects_all();
    let filtered = orders.clone().filter(/* region/date predicates */)?;

    Dashboard::new("exec_overview")
        .title("Revenue Overview")
        .width(Width::fill().max_px(1200.0))
        .param(region.clone()).param(date_lo.clone()).param(date_hi.clone())
        .selection(picked.clone())
        .sidebar(
            Sidebar::left(280.0)
                .widget(Select::new("region_w", region_items(&orders)?, &region))
                .widget(RangeSlider::new("dates_w", &date_lo, &date_hi))
                .text(Typst::new("Total: #currency($total)")),
        )
        .row(Row::height(220.0)
            .chart("trend", revenue_trend(filtered.clone(), &date_lo, &date_hi, &picked)))
        .row(Row::aspect(21.0, 9.0)
            .chart("detail", category_detail(filtered, &picked)))
        .compile(&ctx)
        .await
}

let app = dashboard_avenger_app(compiled, ctx, DashboardAppOptions::default()).await?;
// or: compiled.render_png(width) / render_pdf(width)
```

Two rules make it sound:

- **Declared-on-`Dashboard` = document state; undeclared = panel-local**
  (auto-namespaced by panel key). The bus is exactly the declared set.
- **`compile()` produces N independent `CompiledPlot`s** plus the chrome
  plot and the layout tree — per-panel sessions, lazy tabs, priority
  scheduling, and per-chart baking all follow from that.

The future `.dashboard.avenger` lowers onto the same artifacts; the DSL
adds hash-pinned imports and binding syntax, which Rust gets from `use`
and function arguments.

## Widgets And Data Encoding

The widget paradigm is `widgets.md` in full. What this document adds is
the dashboard-side consequence: **data-encoded widgets generalize to
data-encoded panels.** A KPI-card-per-query-row (`panel_list { data: ...;
key: ...; panel { ... } }`) is the same idea one level up, with `mark
subplot` (a data-driven mark embedding a plot per key) as the designated
chart-level precedent — never a general loop. Deferred until demand
appears; data-driven *tabs* are recorded as an anti-pattern.

## Positioning

- **Mosaic** (coordinator + params/selections lowered to SQL predicates
  over DuckDB) is the closest prior art and validates the coordination
  semantics; this design is the same semantics as a *language* with a
  native+wasm GPU engine, pinned imports, and a test/doc toolchain.
- **Grafana** proves `$var`-in-SQL dashboards at industrial scale — as
  untyped JSON, web-only, without composition or integrity.
- **Evidence / Rill / Observable Framework** prove SQL-first
  dashboards-as-code without an interaction language or native engine.
- **Streamlit / Dash / Shiny** are imperative host-language app
  frameworks — rerun/callback models, not documents (though their layout
  idioms inform `dashboard-layout.md`'s vocabulary).

The differentiated position: a hash-pinned, capability-sandboxed,
SQL-native dashboard document rendering identically on native GPU, wasm,
PNG, and PDF, with declarative reactivity and a component model whose
components are ordinary chart files.

## Phasing (After Widgets)

1. **The coordinator, no widgets.** `CompiledDashboard` /
   `DashboardState`; the param/selection bus with aliasing; charts +
   text panels; document-flow layout (dashboard-layout.md phases 1–2);
   static export. Charts' own tools are the input devices — linked zoom
   and cross-filtering land here. Exit: the `exec_overview` document
   (chart-dsl.md, Dashboards; minus widgets) renders from the Rust
   builder above, natively and in wasm.
2. **Widget integration + scalars.** Sidebar widget stacks via the
   chrome plot (consuming widgets.md deliverables); scalar SQL bindings
   (KPI text, extents) on the async-view-scalars machinery; tabs and
   modals as param sugar.
3. **The document.** The `.dashboard.avenger` file kind lowering to
   `CompiledDashboard`; host `callback`s; hot reload across the import
   graph with state survival; client-server serving and baked
   publishing.

## Open Questions

- Keyword and file kind: `dashboard` vs `app`; `.dashboard.avenger`
  extension mirroring definition files?
- Is a dashboard importable by another dashboard (nesting), with its
  params/selections as its public surface exactly like charts?
- Binding conflict rules: is aliasing always two-way; is a derived
  binding on a tool-written param a hard error or shadow-with-warning?
  Do chart params want Slint-style access qualifiers (`in` = never
  written by the chart's own tools), or is whole-program write analysis
  enough?
- Do selections cross the boundary by clause-set aliasing or lower to
  predicates (Mosaic-style) at the seam?
- Per-instance data remapping (the `data:` binding at an instantiation site — see the exec_overview sketch in chart-dsl.md): ambient-catalog-only in v1, or table binding as real component reuse?
- Theme precedence across the dashboard/chart boundary.
- Deferred-commit forms (Streamlit-`st.form`-style staged param writes)
  vs per-widget `debounce_ms` / `commit:` policies — shared with
  widgets.md's slider/text-input question.
- How do baked dashboards interact with per-viewer params (row-level
  security living in the catalog's parameterized tables)?
- Does `avenger serve`'s project gallery grow into the dashboard host,
  or is the runtime a separate crate over the app layer?
