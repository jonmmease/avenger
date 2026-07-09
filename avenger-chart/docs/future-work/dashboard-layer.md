# A Dashboard Layer Over The Chart DSL

## Status

Exploration record, 2026-07-08/09, promoted from scratch to future-work on
2026-07-09. This is the full design record for the dashboard layer — the
verdict and receipts, the Slint lessons and gap analysis, the layout-model
evolution with its dated reversals preserved, data-encoded widgets, the
engine representation (`CompiledDashboard` / `DashboardState`, the chrome
plot), state bindings and aliasing, and the Rust authoring API sketch. Two
focused designs have been distilled out of it and carry the
implementation-facing form: `widgets.md` (the widget paradigm) and
`dashboard-layout.md` (the layout model and its avenger-layout reuse).
Where this document and those two disagree, the focused docs win; the
dated addenda here record how the decisions evolved.

**Sequencing (2026-07-09): the full widget implementation (widgets.md,
all five phases) is planned before the dashboard implementation.** The
dashboard plan then consumes widgets' shared types (size hints), the
chrome-plot hosting, and the widget set as ready-made — the two tracks'
only designed rendezvous.

---

**Question** (2026-07-08): take `chart-dsl.md` further — a dashboard layer that
imports and composes charts, adds widgets that drive param values, "more in the
direction of Slint," with SQL kept as the expression language. Is there a
natural fit?

**Verdict up front: yes, and the fit is unusually good — because the two
primitives a dashboard coordinator needs are the two primitives the language
already runs on.** A dashboard needs (1) runtime values that flow between
independent panels and rebind cheaply, and (2) shared relational state that
many panels read. The DSL already has both: `param` (runtime placeholders with
stable plans, rebinding without re-planning — the two-mechanism law) and the
data catalog (`table sql` views, `materialize: session`, ambient names). The
dashboard layer is therefore mostly *scope and surface* — one level above the
chart — not new machinery. Several existing documents already lean this way
without naming it.

The one-line framing: **a dashboard is to charts what a chart is to marks** —
and the composition boundary lands on `param`, the runtime side of the
two-mechanism law, so composing charts needs none of the `define`/slot
expansion machinery.

---

## 1. Receipts: the system already leans toward this

These are quotes/facts from the existing docs and code, not aspirations.

**The DSL's open questions already ask for it.** `chart-dsl.md` Open
Questions: *"Should anything beyond `define` declarations become importable
(named data declarations, whole charts as cells)?"* The dashboard layer is the
answer to that question with a place to stand.

**The evaluation cache was designed for it.**
`physical-plan-evaluation-cache.md`: *"One context hosting several charts
shares one cache. That is desirable for dashboards (shared scans and
aggregates hit across charts) but makes the memory budget a cross-chart
resource."* The runtime placement (cache owned by the long-lived
`SessionContext`, not `PlotSession`) is exactly the dashboard shape: one
shared context + catalog, N per-chart `PlotSession`s.

**The param runtime is already the widget API.**
`ChartAppState::set_param(name, value) -> ParamSetResult`
(avenger-chart-app/src/lib.rs) is an existing external param-setting surface
that marks the session dirty and forces `EvaluationMode::Exact` on the next
build. `avenger-chart-egui`'s `AvengerPlotHandle` re-exports it — an egui
slider driving a chart param is possible *today* in host Rust. The dashboard
layer declares that wiring instead of hand-writing it.

**Cross-filtering already works in-engine.**
`avenger-chart-app/examples/temporal_nested_cross_filter.rs`: click a nested
bar in the left chart, the right chart filters through
`Selection::predicate()` — today expressed as two subplots in one concat
`CompiledPlot`. The semantics exist; the dashboard layer moves the
coordination scope from "one compiled plot" to "N compiled plots sharing a
state scope."

**Params were parameterized for exactly this cost model.** Chart params and
catalog `table sql` params plan once with placeholders and rebind at execution
(`with_param_values`); `logical-plan-partial-evaluation.md` keeps
placeholder-bearing subtrees symbolic so everything param-independent folds at
bake time. A widget wiggling a param re-executes only the placeholder-bearing
subtrees, per chart, with the shared cache serving the rest. That is the whole
dashboard interaction loop, already specced.

**Scalar reactive values are already planned.**
`scratch/async-view-scalars-plan.md` gives scalar aggregates the
materialization-cache treatment (async compute, previews use latest-ready).
That is the execution model a KPI tile or a widget's SQL-driven extent needs.

**The layout and shell crates exist.** `avenger-layout` is chart-independent
("leaves + grids, chrome on any node, solve once"); `avenger-egui` +
`avenger-chart-egui` already implement the Rerun-style shared-wgpu-device
panel architecture the `avenger editor` design describes. A dashboard host is
those pieces plus a declarative file format.

---

## 2. The Slint analogy, taken seriously

Slint's model, precisely: `.slint` files declare components with typed
properties carrying access qualifiers (`in`: callers may set/bind, the
component itself may not overwrite — compiler-enforced; `out`: component-only
writes; `in-out`: both; `<=>` aliases two properties to one shared cell).
Reactive bindings are push-dirty / pull-lazy at property granularity, with
dependencies discovered dynamically during evaluation; an imperative `set()`
from host code silently *removes* the property's binding (a documented
footgun). The host boundary is deliberately narrow: root-exported
properties/callbacks/globals plus a `Model` trait as the only incremental
collection channel; `pure callback`s may be called inside bindings, which is
how host-computed values become reactive (their plotter example renders a
chart through exactly this). Widgets are written in the language itself over a
small set of native primitives. One compiler front-end feeds codegen, a
tree-walking interpreter, and the LSP/live-preview — the keystone of its DX.
Native + wasm from one source.

The mapping is close to mechanical:

| Slint | Dashboard layer | Notes |
| --- | --- | --- |
| `component` in a `.slint` file | a chart file (`.avenger`) | already one-per-file, standalone-renderable, baseline-tested |
| `in property <type> name` with default | `param as name { default: ...; }` | already typed-by-default, already declared |
| `in-out property` / `<=>` two-way binding | binding a chart param to a bare `$dash_param` (aliasing) | chart-internal `set param` writes propagate up — linked zoom falls out |
| one-way binding `prop: expr;` | binding a chart param to a SQL expression over dashboard params | Slint: a host `set()` silently breaks the binding; here a tool writing a derived-bound param is a *compile-time* diagnostic, because writes are declared syntax |
| `callback clicked;` → host handles | `callback export_csv(...);` → host registers handler | the imperative escape hatch; keeps the language query-only |
| `pure callback` used inside a binding | registered SQL UDFs | the host extends the *expression language*, not the binding graph — same reactivity, better sandboxing story |
| `global` singleton | dashboard-scope params + the ambient data catalog | catalog = the relational global |
| `Model` / `ListModel` | catalog tables and `table sql` views | data-driven everything comes from SQL/Arrow, not host arrays |
| push-dirty/pull-lazy property graph, runtime-discovered deps | the `$param` reference graph | statically known from declarations (over-approximate but safe); Slint discovers deps at runtime for branch-level precision |
| `std-widgets` written in Slint over native primitives (`Rectangle`, `TouchArea`, `TextInput`, ...) | either `std:widgets/` written in the DSL over marks + events, or host-rendered abstract widgets (egui) | the mark-built option mirrors Slint's law exactly; §4b argues for the egui host first with mark-built as a possible second host |
| slint-lsp + live preview + SlintPad | `avenger lsp` / `serve` / `editor` / online editor | already specced for charts; dashboards ride the same import-graph hot reload |
| native + wasm from one source | wgpu native + wasm | already the engine's story |

Two places the analogy is *stronger* here than in Slint:

- **The binding language is SQL, so bindings reach the data.** Slint bindings
  compute over properties; dashboard bindings compute over params *and the
  catalog*: a dropdown's options are `SELECT DISTINCT "region" FROM sales`, a
  slider's extent is `(SELECT min("fare"), max("fare") FROM trips)`, a KPI
  tile is `(SELECT count(*) FROM trips WHERE "fare" >= $min_fare)` — scalar
  subqueries are ordinary SQL expressions, so the *syntax already exists*;
  what's new is planning/async treatment (the async-view-scalars plan).
- **The dependency graph is static.** `$name` must resolve to a declared
  param, so the compiler knows exactly which charts depend on which params and
  can schedule invalidation without runtime dependency discovery.

One place it's deliberately *weaker*, and should stay weaker: Slint (like its
ancestor QML) permits imperative statements in callback handlers, `if`/`for`
element constructs, and general app logic. That is the right call for a GUI
toolkit and the wrong call for this system. The dashboard layer should keep
the chart DSL's conditional budget (params for runtime values, SQL `CASE` for
expression logic, `match` in definitions, visibility for runtime showing/
hiding — and nothing else branches). What keeps the layer honest is that it
stays a **document**: hash-pinnable, `avenger test`-able against a baseline
PNG, `avenger render`-able to PNG/PDF for reports (avenger-pdf exists — no
web dashboard tool exports its live dashboard to a paginated PDF cleanly),
agent-authorable via the same language card, capability-sandboxed. "Dashboards
are documents, apps are hosts" is the scope fence.

The tooling keystone transfers wholesale. Slint's DX rests on one compiler
front-end shared by codegen, interpreter, LSP, and live preview — which is
exactly the `avenger-lang`-as-a-library plan already in chart-dsl.md (CLI,
LSP, and the egui editor share one parser/schema). Dashboards inherit it:
`avenger serve` previews a dashboard with host callbacks stubbed (Slint's
live preview does precisely this with business logic), import-graph hot
reload means editing a chart file live-reloads every dashboard whose closure
contains it, and session state (params, selections, view domains) already
survives recompiles by name.

### Lessons from Slint beyond the mapping (added 2026-07-09)

*Slint's layout inventory, for reference:* explicit positioning (`px`/`%`/
`rem`, several elements default to fill-parent) or containers —
`HorizontalLayout`/`VerticalLayout`/`GridLayout` (row children,
rowspan/colspan) and `FlexboxLayout` (taffy-solved). Every element carries a
five-knob constraint vocabulary — `min-`/`preferred-`/`max-width/height`
plus per-axis `stretch` factors — and constraints propagate child→parent
(intrinsic sizing). The solver is a one-pass per-axis box solve (Qt-style,
deliberately not a general constraint system). Crucially, layout inputs are
themselves properties, so layout re-solves reactively.

*Slint's state inventory:* component properties with access qualifiers
(private/`in`/`out`/`in-out`), `<=>` shared-cell aliasing, exported `global`
singletons, the `Model` trait with row-level change notification and
adapters (filter/sort/map) feeding virtualized repeaters, declarative
`states [ ... when cond: { overrides } ]` with animated transitions,
`animate` on any property (implemented as self-removing bindings), and
deferred/coalesced `changed` callbacks with a runtime chain limit.

**Lessons worth adopting:**

1. **Param-bindable layout knobs** (their "layout inputs are properties").
   `sidebar { width: $sidebar_w; visible: $show_controls; }` makes
   collapsible chrome fall out of the param bus — no new mechanism.
2. **If sizing knobs ever grow past `px`/`fr`/`aspect`, copy the five-knob
   vocabulary** (min/preferred/max/stretch per axis) — small, complete,
   one-pass solvable; don't invent.
3. **Mechanism economy.** Slint runs values, layout, animation, and damage
   tracking on ONE property system. Our discipline: *everything dynamic is
   a param or a query* — tabs, modals, collapse state, layout knobs all
   route through params; never grow a third channel.
4. **Spec the update semantics** the way they spec'd changed-callback
   coalescing: event actions apply atomically per event; one re-evaluation
   per frame batch (ChartApp's pending-patch drain already behaves this
   way); no cascading param writes during evaluation.
5. **Grammar-in accessibility and i18n early**: accessible
   label/description properties on panels and widgets (wired to AccessKit
   later), and a `@tr`-style translation hook (locale infrastructure
   already exists in avenger-format-*). Cheap now, painful to retrofit.
6. **Micro-interaction stance**: treat chrome animation (tab switches,
   collapses, hover affordances) as *runtime presentation policy*, never
   document semantics — final states are the semantics and the baselines.
   Slint's `animate`/`states` shows the appetite; a document format should
   satisfy it in the runtime, not the language.

**What Slint enables that this model (so far) does not:**

1. **User-defined UI components at every granularity** — a styled button, a
   KPI card, a filter panel — composed in-language with properties and
   callbacks. Here, charts are components and widgets are host-provided,
   but there is no `define` for *dashboard fragments* (card = frame +
   title + value + sparkline + delta; filter panel = widget group with
   shared config). This is the most real gap. The chart DSL's
   `define`/slot machinery extends naturally to a `define panel` when
   demand appears — explicit instantiation, no loops, same hygiene laws.
2. **Model-driven structural repetition** (`for card in model`). Largely
   *resolved* by data-encoded widgets (§4c): item multiplicity comes from
   a relation with columns encoded to widget channels — the mark model
   applied to widgets, which is the language's own sanctioned form of
   repetition. What remains deferred is data-encoded *layout* repetition
   (a card per query row), which has the `mark subplot`/facet precedent
   waiting when demand appears — never a general loop.
3. **Micro-interactions on chrome** (hover/pressed states, transitions) —
   see the stance above; charts get hover through the event system and
   selections, chrome stays static-but-animatable-by-the-runtime.
4. **Arbitrary property-to-property wiring.** Slint lets any property bind
   to any other; we route all dynamics through the param bus. Deliberate
   non-gap: hub-and-spoke is what makes dashboard state serializable,
   deep-linkable, and testable. Slint needs the general graph because it
   is a GUI toolkit; a document format is better off without it.
5. **Component-local ephemeral state** (private properties). Our line must
   stay crisp: document state = params (serializable, persisted across
   reloads, swept in tests); ephemeral state (combo-open, text cursor,
   staged form input) lives in the runtime and never in the document. Per
   widget, decide which side each state is on (sidebar collapsed: param;
   dropdown open: ephemeral).
6. **Dialogs/popups/multi-window.** The dashboard analog is the
   drill-through modal, and it is param sugar exactly like tabs:
   `modal { visible: $detail_id is not null; ... }` with a chart click
   setting `$detail_id`. A pattern to document, not new semantics. True
   multi-window stays host territory.
7. **Custom chrome drawing** (Rectangle/Path/gradients in-language). The
   escape hatch is already in the system: a zerod chart is a drawing
   surface; theme/CSS covers frames, separators, and badges.

---

## 3. What the layer would look like

A sketch, using the existing grammar's shapes (declaration form, property
blocks, cells, `$param`s, catalog). One new file kind, a handful of new
declaration keywords, zero new value forms:

```avenger
avenger 1;

import 'charts/revenue_trend.avenger';        -- chart files become importable
import 'charts/category_detail.avenger';
import 'std:widgets/range_slider';
import 'std:widgets/select';

dashboard as exec_overview {
  title: 'Revenue Overview';
  theme css from 'themes/corporate.css';

  -- dashboard-scope state: same constructs, one level up
  param as region   { default: 'all'; }
  param as date_lo  { default: DATE '2026-01-01'; }
  param as date_hi  { default: DATE '2026-12-31'; }
  selection as picked_categories { empty: all; }

  -- shared derived data: catalog-style views, computed once, cached
  table sql as filtered_orders {
    materialize: session;
    sql:
      SELECT * FROM orders
      WHERE ("region" = $region OR $region = 'all')
        AND "date" BETWEEN $date_lo AND $date_hi;
  }

  -- document-flow layout: fixed width, height grows, the document scrolls
  width: fill { max: 1200; }
  spacing: 12;

  sidebar left as controls {
    width: 280;                                -- fixed chrome, outside the document scroll

    widget select as region_w {
      param: region;                           -- tool-style param wiring
      options: SELECT DISTINCT "region" FROM orders ORDER BY 1;
      all_value: 'all';
      label: 'Region';
    }

    widget range_slider as dates_w {
      lo_param: date_lo;
      hi_param: date_hi;
      extent: (SELECT min("date"), max("date") FROM orders);
      label: 'Dates';
    }

    text as kpi {
      syntax: typst;
      content: 'Total: #currency(' || (SELECT sum("amount") FROM filtered_orders) || ')';
    }
  }

  row {
    height: px(220);                           -- chart heights are declared (px or aspect), never content-driven

    chart revenue_trend as trend {
      -- param bindings: bare $ = alias (two-way), expression = derived (one-way)
      date_lo: $date_lo;
      date_hi: $date_hi;
      highlight: $picked_categories;           -- selection aliasing: cross-filter
    }
  }

  row {
    height: aspect(21, 9);                     -- height as a function of allocated width

    chart category_detail as detail {
      source: 'filtered_orders';               -- open design point: table binding
      picked: $picked_categories;              -- writes here filter `trend` above
    }
  }
}
```

Notes on each piece, and why it is small:

- **`import` of a chart file** binds the chart's name;
  `chart <name> as <instance> { ... }` instantiates it — the same
  import-then-instantiate shape as `mark box_plot`. The kind slot
  disambiguates exactly as it does for marks: coordinate kinds
  (`chart cartesian { ... }` inline, still legal in a cell) are reserved
  words; imported names are user names. Two instances of one chart with
  different bindings is ordinary (that's what makes it a *component*).
- **A chart's public interface already exists**: its declared params (name +
  default = the schema; `avenger info chart.avenger` prints it), its named
  stores/selections, its named marks (event paths). Nothing needs to be added
  to chart files to make them embeddable — every existing chart is already a
  dashboard component with defaults that keep it standalone-renderable. This
  is the single strongest "natural fit" fact.
- **Binding forms mirror the two Slint arrows.** Bare `$dash_param` aliases
  the chart param to the dashboard param cell (two-way; a pan/zoom tool
  writing the chart's `x_domain` moves every chart aliased to it — linked
  zoom with zero new concepts). A SQL expression makes a one-way derived
  binding; because `set param` actions are declared syntax, the compiler can
  statically reject a derived binding on a param some tool writes.
- **Widget declarations are semantic, not host-specific.** The tool pattern
  already passes param names as slot values (`tool pan_scroll_zoom
  { x_domain_param: x_domain; }`); widgets follow it: a widget kind, the
  param(s) it drives, and SQL-driven props. What renders them is a runtime
  decision the grammar never names. §4b weighs the two hosts (egui vs
  mark-built) and — after the document-flow and data-encoding decisions —
  lands on **mark-built as the default** (§4b revised verdict): a widget
  is a tool with its own canvas (a slider is a rect track + symbol thumb +
  between-binding drag + `set param`; a checkbox list is literally a small
  chart), exactly how Slint builds std-widgets over its primitives. The
  egui host remains the recorded fallback the abstract vocabulary keeps
  open.
- **Layout is document-flow, and the GUI runtime owns it** (see §4b item
  4): fixed width (fill the viewport, optionally max-clamped), a vertical
  flow of `row`s each splitting that width into tracks, height growing
  with content, and **one implicit scroll on the document — never inside a
  panel**. Scrolling is a property of the model, not a composable
  container, so the grammar has no scroll declarations at all. `sidebar` /
  `topbar` declare fixed chrome outside the document scroll. Charts get
  their width from the track and their height by declaration (`px` or
  `aspect` of allocated width) — never from content; text and widget
  leaves self-size vertically. The dashboard never does plot-area
  alignment — panels needing aligned plot areas belong in a concat chart,
  which already solves alignment with the full measurement machinery.
  Panel titles, borders, and backgrounds come from the theme.
- **Text panels** ride the Typst text stack already used for titles; string
  concatenation with scalar subqueries makes them reactive.
- **`table sql` in a dashboard body** is the catalog construct scoped to the
  dashboard: shared derived relations for its charts, `materialize: session`
  for compute-once-feed-five-panels, fingerprint-keyed by bound params so a
  param change re-materializes exactly what it invalidates.
- **Concat vs dashboard, the crisp line:** concat/facet stay what they are —
  *one chart*: one data context, coordinated scales/guides/slots, and
  measurement-coordinated layout (plot-area alignment). A dashboard is
  *many charts*: independent data contexts, scale systems, and plot layouts,
  coordinated **state** (params, selections, derived tables), arranged by
  GUI-shell layout. Alignment lives below the line; state lives above it.
  The `chart grid_concat as dashboard` example in chart-dsl.md is reaching
  for this layer with the wrong tool, which is itself evidence the layer
  wants to exist.

### Runtime shape

One `SessionContext` (shared DataFusion catalog + physical-plan evaluation
cache + memory budget) — N `PlotSession`s (one per chart instance, each with
its existing preview/exact machinery) — one dashboard state scope above them.
`ScopedParamStore` is already hierarchical (Shared/Free/Level(n) inside a
chart); the dashboard scope is one more level above `Shared`, and aliasing is
two names sharing a cell. The interaction loop:

```
widget event → set dashboard param
  → static $-graph gives the dirty set (charts, views, scalar bindings)
  → each dirty PlotSession evaluates (Preview if raw-domain-only, else Exact)
  → shared cache serves param-independent subtrees across charts
  → dashboard layout unchanged unless a panel's measured size changed
```

Client-server: the dashboard is the natural served unit — one catalog
manifest, N plan-fragment streams against a stateless server, `priority` on
requests (already in the wire spec) maps to "visible panels first"; baking
(`partial_evaluate`) folds param-independent work across all charts at publish
time. The orchestration-above-plot-level is new but thin; the per-plot
machinery is all specced.

---

## 4. What is genuinely new or hard (the honest bill)

1. **Native input primitives — or an egui host.** Text input (IME,
   clipboard, focus), popup/overlay z-ordering, scroll regions, keyboard
   traversal, accessibility: this is where GUI toolkits spend their
   complexity budgets (Slint included). Whether to build these as scenegraph
   primitives or adopt egui as the widget/layout host is a big enough
   decision to get its own section — see
   [§4b The widget-runtime decision](#4b-the-widget-runtime-decision-egui-host-vs-mark-built).
   Sobering datapoint either way: Slint's own wasm deployment ships with
   **no screen-reader accessibility** and is positioned by its docs as
   demo-grade — the all-canvas approach has a known a11y ceiling on the web
   regardless of host. AccessKit is the native target (egui already
   integrates it; a scenegraph path would need semantic nodes).
2. **Dashboard-scope state + aliasing.** Extend the scope ladder one level;
   define write propagation and conflict rules (statically checkable). Small
   but must be precise, especially selections (clause sets, not scalars)
   crossing chart boundaries as predicates — `Selection::predicate()` is the
   in-engine precedent.
3. **Scalar reactive bindings.** Generalize the async-view-scalars design
   from view chains to dashboard bindings (KPI text, widget domains,
   derived params): scalar materializations, async, latest-ready previews,
   cache-keyed like everything else. Syntax is free (SQL scalar subqueries);
   planning/scheduling is the work.
4. **Layout beyond solve-once charts.** Scroll containers, collapsible
   sections, tabs/pages, resizable splits, responsive reflow. *(Resolved by
   the egui host — §4b item 4: these are exactly egui's native containers,
   and plot-area alignment stays a chart-level concat concern, so the
   dashboard never needs measurement-coordinated layout.)*
5. **Theme cascade across the boundary.** The theme law says the chart author
   owns the theme; a dashboard is an outer author. Needs one rule — e.g.
   dashboard theme applies where the chart doesn't declare its own, with
   per-instance override — plus a decision about whether instance bindings
   can override chart-declared themes.
6. **Scheduling and budget.** N charts × param fan-out on one thread (wasm):
   the cache docs already flag memory as a cross-chart resource; add
   visibility-aware evaluation order and lazy evaluation of hidden panels.
7. **Data binding at the instance boundary** (the `source:` line in the
   sketch): does a dashboard remap which table a chart reads? Options: keep
   charts on ambient catalog names only (v1 — simple, and dashboards already
   control the ambient catalog), or allow per-instance table binding (real
   component reuse: same chart over different slices). The catalog's
   parameterized tables may already cover most of it (bind a chart's `data`
   table-param to different `table sql` calls).

None of these are architectural conflicts; they are additive workstreams. The
one real philosophical risk is **scope creep toward a GUI toolkit** — the
pressure will come as "just add a for-loop over panels," "just add an on-click
script." The fence: repetition is data (facet/repeat *inside* charts),
side effects are host callbacks, and the dashboard file remains a pure
function of (files, catalog, params).

---

## 4b. The widget-runtime decision: egui host vs mark-built

*(Added 2026-07-08 after discussion: should the dashboard DSL compile into an
egui+avenger app, with egui owning dashboard layout and widgets — or should
widgets be little charts built from mark primitives?)*

**Reframe first: these are answers to two different questions.** What a
widget declaration *means* is a language decision (`widget select { param:
region; options: SELECT DISTINCT ...; }` — semantic kind, param wiring,
SQL-driven props). What *renders* it is a host decision (egui vs scenegraph
marks). If the grammar never leaks host concepts, the same documents run on
either host, and the choice becomes sequencing rather than destiny. That is
the load-bearing recommendation: **keep the widget vocabulary abstract; ship
the runtime on egui first; keep mark-built as a possible second host.**

### Corrections to §4's earlier lean

Two objections I raised against an egui bridge don't survive scrutiny:

- **"Untestable panels" — wrong.** `egui_kittest` (egui's official test
  harness) renders offscreen via wgpu for snapshot testing; Rerun
  snapshot-tests its egui UI this way. `avenger test` can baseline
  egui-hosted dashboards as PNGs. Determinism needs the usual care (embedded
  fonts — egui ships its own; fixed scale factor; cursor-blink/spinner
  animations off; fixed evaluation epoch), all standard practice.
- **"Two reactive systems" — overstated.** Immediate mode plus a single
  param store is actually a *clean* composition: each frame, widgets read
  params and draw; on interaction they `set_param`. Widgets are stateless
  views over the param store — the single source of truth — and egui retains
  only ephemeral UI state (text cursor, open-combo flag) that doesn't belong
  in the document anyway. Notably this avoids Slint's documented footgun
  (imperative `set()` silently breaking a declarative binding): with
  immediate mode there is no retained binding to break.

### What the egui host buys

- **The entire hard tier, today.** `TextEdit` with IME/clipboard/selection,
  `ComboBox` popups, `ScrollArea`, tooltips, tables (`egui_extras`),
  docking/tiling (`egui_tiles` — Rerun's layout crate), date pickers in the
  ecosystem, touch support, and AccessKit accessibility already integrated.
  This is precisely the list in §4 item 1 that GUI toolkits die on.
- **The repo is already on this path.** `avenger-egui` /
  `avenger-chart-egui` exist (shared-wgpu-device paint callbacks,
  `AvengerPlotHandle::set_param`, `RenderInvalidationHub`), and the
  `avenger editor` design is committed to egui. The dashboard runtime is
  approximately *the editor's preview pane generalized to N charts plus a
  widget sidebar* — not a new architecture.
- **It doesn't block on the language.** Mark-built widgets require the
  definitions/expansion machinery, the event system, and new primitives to
  exist first. The egui path needs only the dashboard AST, the param store,
  and crates that already exist — shippable while the chart DSL itself is
  still being built, and it exercises the coordinator semantics (params,
  bindings, catalog) early.
- **"Compile" should mean *interpret*.** Don't codegen Rust per dashboard;
  build one generic egui host that loads and walks the dashboard AST each
  frame (the slint-interpreter model, which is also how the DSL's own
  editor/preview story works). One generic wasm bundle then loads *any*
  dashboard document — deployment is "host the runtime once, fetch
  documents," matching the hash-pinned distribution story.

### What it costs, and the guardrails

1. **Vector/static export.** egui paints through its tessellator; there is
   no SVG/PDF backend. `avenger render dash.avenger -o board.pdf` becomes a
   composite (charts can stay vector inside the PDF while widget chrome
   rasterizes, or the whole page rasterizes at 2–3×). Mark-built widgets
   would keep the entire dashboard one scenegraph, vector end-to-end. **If
   print-grade report export becomes a core requirement, this is the
   decisive counterweight** — it is the strongest argument mark-built has.
2. **Styling splits.** egui `Visuals` ≠ CSS. Guardrail: derive `Visuals`
   from the dashboard theme's CSS custom properties (colors, spacing,
   rounding) so widgets are *theme-coherent*, while accepting they are not
   *CSS-styled* (no part selectors, no cascade into widget internals).
3. **The widget set closes.** Widgets become Rust-native; users cannot
   define and pin new widget kinds as definition files, so the
   definitions-over-primitives law stops at the widget boundary. The
   principled framing: input chrome is a **host concern** — HTML documents
   declare `<input type="range">` and the browser renders it; Vega-Lite
   `bind`, Jupyter widgets, and Dash all do the same. The DSL declares
   semantics, the host renders chrome. (And the interesting "custom
   widgets" turn out to be charts anyway — see the hybrid law below.)
4. **Layout ownership.** *(Revised 2026-07-09 — original guardrail argued
   for DSL-owned tracks solved by avenger-layout; the layering argument
   below supersedes it.)* Plot-area alignment is a **chart-level** concern
   and is already solved there: concat/grid composition inside a chart does
   measurement-driven alignment with the full solver machinery (share keys,
   guide policies), byte-identically tested. The dashboard level should
   **not** duplicate any of that — its layout needs are GUI-shell needs
   (sidebars, scroll regions, resizable splits, collapsible sections, tabs,
   responsive reflow), which avenger-layout has no concepts for and egui
   provides for free. So: **egui owns dashboard layout outright.** When two
   panels genuinely need aligned plot areas, that composition belongs in a
   concat chart — which is exactly what chart-dsl.md's open question
   "should whole charts become importable as cells" would enable for
   imported charts. Two composition altitudes, each with the right tool:
   concat composes *aligned plots*; the dashboard composes *independent
   panels*. One rule to keep the immediate-mode layout well-founded: the
   dashboard always **imposes** size on charts (fill-available or
   fixed/weighted), never asks a chart for its content-driven size — no
   measurement feedback loops, and it is how `avenger-chart-egui` already
   works (allocated rect → `CanvasDimensions`). Widget/text panels size
   themselves natively in egui. Consequence for the grammar: the dashboard
   layout vocabulary maps ~1:1 onto egui containers, with semantics
   honestly documented as runtime-defined; rendered output stays
   deterministic per toolchain version, with egui upgrades occasionally
   re-blessing dashboard baselines — the same lifecycle as font or engine
   upgrades today.

   **The layout model, pinned (2026-07-09): document flow.** Fixed width
   (viewport-fill, optionally max-clamped), height grows with content, one
   scroll on the whole document, **no scrolling inside panels** — the
   Evidence/notebook/report model, not the Grafana/Rerun viewport-filling
   tile model. Combined with declared chart heights (`px` or `aspect` of
   allocated width — never content-driven), this makes dashboard layout a
   **closed-form, single-pass computation**: width → tracks → chart
   heights → row heights → total height. No egui frame-memory convergence
   anywhere, and the scroll extent is exact *without evaluating a single
   chart* — so lazy evaluation of off-screen panels causes zero layout
   shift (web dashboards famously jump as content loads; this one cannot),
   and scroll position becomes a clean evaluation-priority input (visible
   panels first, matching the wire protocol's `priority` field). The egui
   mapping is minimal: `SidePanel`/`TopBottomPanel` for fixed chrome, one
   `CentralPanel` + outer `ScrollArea::vertical()`, top-down rows of
   horizontal `StripBuilder` tracks. Long tables paginate rather than
   scroll (consistent with the no-panel-scroll rule). Static export gets
   stronger: PNG renders at full content height, and PDF paginates at row
   boundaries — rows are natural page-break atoms. One honest wasm note:
   the browser canvas stays viewport-sized and egui does the scrolling
   internally (a content-height canvas would blow GPU/texture limits on
   tall dashboards), so scroll feel is egui-emulated (kinetic scrolling
   works) and browser-native niceties — find-in-page, URL anchors, native
   scrollbar styling — don't apply.

   **The second height policy: `fill` (added 2026-07-09).** The
   Streamlit/Dash class of one-screen data apps — sidebar of widgets,
   charts sized so everything fits the viewport — is the same model with
   the height equation inverted, not a different model. `height: fill`
   binds document height to the viewport: row heights become `px`/`fr`
   tracks over the fixed total (`aspect` is over-determined there and
   rejected), the document scroll disappears, and everything else —
   imposed chart sizes, single-pass closed-form solve, no per-panel
   scroll, fixed sidebar chrome — is unchanged. An optional
   `fill { min_height: px(n) }` degrades to scrolling when the viewport
   shrinks below a usable minimum. The egui mapping swaps the outer
   `ScrollArea` for a vertical `StripBuilder`. `flow` stays the default;
   the policy is one root property. (Streamlit's own default shape —
   fixed sidebar + scrolling main column — is already the `flow` policy;
   `fill` covers the Dash/Grafana/monitoring-wall styles.)

   **Layout engine resolved (2026-07-09, post-widget-paradigm): no
   external toolkit, no taffy — avenger-layout in its simple mode.** With
   widgets native (widgets.md), the last reason for a GUI toolkit at the
   dashboard level is gone, and the taffy question answers itself: the
   pinned document model (explicit tracks, imposed chart sizes, leaf size
   *hints* precomputed as constants, one document scroll) reduced
   dashboard layout to closed-form track arithmetic — taffy is a
   constraint solver for content-negotiated flex/grid, i.e. for problems
   this model deliberately does not have. (Slint's own core H/V/Grid
   layouts are a hand-rolled one-pass box solve; it reaches for taffy
   only for its CSS-flexbox add-on.) The dashboard driver is a thin
   document-flow layer over avenger-layout's existing grid machinery
   (`TrackSizing` px/fr/auto, `GridShape`/`GridSolution`, chrome on
   nodes, SVG debug rendering) — one layout vocabulary repo-wide, with
   share-key cousin alignment available-but-unused if cross-panel
   alignment ever becomes opt-in. Two small additions, both already
   named: content-sized tracks fed by widget hints (widgets.md phase 4)
   and aspect-height rows (height = f(solved width), pure arithmetic).
   The invariant to protect regardless of engine: layout stays
   closed-form given (width, params, item counts) — no measure/re-solve
   loops at dashboard level; avenger-layout's heavier negotiation
   machinery (overflow, edge grants) stays out of the dashboard path.
   Scrolling remains a runtime concern (viewport translate + clip +
   wheel), not a solver concern. Sufficiency of existing containers, two
   altitudes: **small/medium dashboards can literally be concat charts
   today** — one `CompiledPlot`, `WidgetCell` cells, shared
   params/selections proven by the cross-filter example, avenger-layout
   solving, PDF export free; limits are one coupled measurement pass, no
   lazy tabs/priority scheduling, and a fixed canvas (document scroll
   needs the runtime viewport either way). The dashboard container
   arrives when N-session isolation, lazy tabs, and visible-first
   scheduling matter — swapping *semantics* (coordinated state, own file
   kind) while keeping the same track vocabulary and solver. Adopting
   taffy later would be a symptom that the model drifted into
   content-negotiated app-toolkit layout — the drift the document stance
   exists to prevent — not a solution.

   **Tabs are param sugar (added 2026-07-09).** A `tabs { tab as
   overview { ... } tab as detail { ... } }` construct declares an
   implicit param whose value is the active tab id and renders a tab bar
   (trivial egui chrome); switching tabs writes the param. Everything
   else follows from param semantics already in the design: layout stays
   closed-form given (width, params) — the active tab's heights are
   declaration-derived, merely param-dependent; hidden tabs evaluate
   lazily and *retain state* on switch (their chart instances'
   PlotSessions and the dashboard params persist — none of Streamlit's
   rerun-on-interaction cost); deep-linking an active tab is ordinary
   param serialization (a host concern, like URL query binding);
   `avenger test` covers tabbed dashboards as a param sweep — one
   baseline per tab value; PDF export prints tabs as consecutive
   sections. Tabs can nest wherever a row can appear, because heights
   remain declaration-derived. Underneath, a tab is exactly the
   `visible:`-driven-by-params pattern the report already endorses —
   the construct is sugar, not new semantics.
5. **Dependency exposure.** egui's API moves quarterly-ish. Because the
   grammar and AST never mention egui, churn stays contained in the runtime
   crate (the same way wgpu versions are already managed in-repo).

### The hybrid law that dissolves most of the tension

The controls that make a *data* dashboard interesting are data-driven: a
range slider over a histogram (a brush), legend-chip filters, a sparkline
picker, a date-range brush over a timeline. **Those are charts with selection
tools** — already expressible with P0 chart composition and zero widget
machinery, and better expressed that way than as either an egui slider or a
hand-built mark widget. What remains for "widgets" is deliberately boring
input chrome: text search, dropdown, checkbox, button, date picker — exactly
what egui is best at and exactly what is miserable to rebuild from marks.

So the division of labor is not a compromise, it's the natural joint:
**charts for data-driven controls, egui for input chrome.**

### When to revisit mark-built widgets

- Vector-PDF dashboards or pixel-parity across hosts becomes a requirement;
- a real ecosystem demand appears for user-defined, hash-pinned widget
  definitions;
- the definitions + event machinery exists anyway (built for compound marks
  and tools), dropping the marginal cost of `std:widgets` to "write
  definition files."

Because the language stayed host-agnostic, that migration touches runtimes,
not documents.

### Precedent

Rerun is the shipping proof of this exact architecture: egui shell + custom
wgpu renderer sharing one device + `egui_tiles` + **blueprints** — documents
that describe panel layout and content, interpreted by the egui host. The
repo already cites the Rerun architecture for `avenger-egui`; the dashboard
layer extends the analogy from "panel" to "document." Blueprints also
preview a nice future hook: host-side drag-rearrange can write back into the
document (the same round-trip Slint's design-mode makes to `.slint` source).

### Verdict revised (2026-07-09): don't drop to an external toolkit yet

Three things changed over the course of the exploration, and together they
flip the default back to mark-built:

1. **Document-flow layout deleted most of egui's layout value.** The
   pinned model (§4b item 4) needs no panels-in-panels, no per-panel
   scroll, no resizable splits, no docking — a closed-form track solve
   that avenger-layout (or fifty lines of code) does directly. What
   remains is one document scroll container: translate + clip + wheel
   events + scrollbar marks, all existing primitives.
2. **Data-encoded widgets (§4c) revealed the flagship widgets are already
   chart semantics.** A checkbox list is a relation + rect/glyph/text
   marks + a click binding emitting `toggle_clauses` — *literally a small
   chart* with a band scale. The mark machinery IS the per-item machinery;
   no per-row widget instantiation is even needed. Chips, radio lists,
   interactive legends, sliders (§3's original sketch), buttons: all
   marks + event bindings.
3. **The engine already has "a definition producing params + marks":
   `ToolExpansion`** (avenger-chart-core/src/tools.rs:108) carries
   `params`, `stores`, `selections`, `event_bindings`, **and `marks`** —
   and the DSL spec gives tool definitions the same powers (`param as` /
   `store as` / `selection as` instance-scoped state, `on` bindings,
   chrome marks, with `export` covering nested instance state). Compound
   *marks* are stateless lowering today, and should stay that way; the
   missing kind is **`define widget` = tool-style state + events, mark-
   style face, plus two genuinely new contracts: its own coordinate frame
   (tool marks currently render into the host plot's space) and
   dashboard-layout-leaf status with intrinsic size hints** (a checkbox's
   height is a line height; a checkbox list's is n × line height — the
   §4c "item counts enter layout" rule).

What mark-built wins back: vector export end-to-end (the PDF/report story —
egui's biggest cost), CSS theming into widget chrome (part selectors and
all), one event system and one scenegraph (rtree hit-testing, `avenger
test` baselines with zero new harness), and user-defined, hash-pinned
widget definitions — which also closes most of Slint gap #1 (§2) for
widgets. The dashboard runtime becomes an ordinary avenger-app (existing
winit native + wasm paths), not an egui app.

What stays honestly hard, and phases: text input — planned 2026-07-09 as a
**native widget** (widgets.md): the `NativeWidget` tier (params in →
arbitrary Rust → vector scene marks out; the guides/custom-marks pattern
miniaturized to a frame) hosts a `TextInput` built as a new editing layer
over the in-repo Typst-based text stack (`avenger-text` /
`avenger-typst-label`, rustybuzz shaping — NOT cosmic-text, which the
project no longer uses), designed after a prior-art study of parley's
PlainEditor, cosmic-text's Editor, iced, egui, and Slint editing
implementations — vector in exports, CSS-token themed, native baselines.
An external-toolkit fallback (egui offscreen → `Image` mark via the
map-tile resource path) is recorded in widgets.md but not promoted. Still
phased: a popup/overlay surface (combo menus), focus/keyboard traversal
beyond the minimal service `TextInput` needs, AccessKit semantic nodes in
the scenegraph, and hand-rolled scroll feel (kinetic).
None are needed for P0 (no widgets) or P1 (data widgets: checkbox/chips/
radio/legend/slider/buttons/select-as-list) — free-text search and combo
popups are the P2 gate. egui keeps its roles as the *editor shell* and the
embed-avenger-in-egui-apps story; it just isn't the dashboard runtime.

The §4b analysis above stands as the recorded alternative: if sequencing
pressure demands a dashboard MVP before the DSL's event/definition
machinery exists, the egui host remains viable *because the widget
vocabulary stayed abstract* — which was the point of keeping the language
host-agnostic all along.

### Engine representation (added 2026-07-09): not a Plot, a tool expansion with a frame

Two questions pin the Rust-side design: does the engine need widget
primitives, and is a widget a kind of `Plot`?

**Per-kind Rust primitives: no.** Checkbox, chips, radio, slider, legend,
buttons are definitions over existing primitives (marks + event system +
state), per the language's own law. The Rust surface a widget system needs
is exactly three things:

1. **One container contract** — the expansion target for `define widget`.
   The engine already has 90% of it: `ToolExpansion` carries `params`,
   `stores`, `selections`, `event_bindings`, `scale_edits`, `marks`. A
   widget expansion is that struct plus two fields: an **own frame**
   (tool marks render into the host plot's space today; widget marks
   render into a frame the dashboard layout assigns) and **size hints**
   (intrinsic width/height policy). "A widget is a tool with its own
   canvas" becomes literal: one shared expansion type, `define tool` and
   `define widget` staying distinct DSL kinds (schema/completion) that
   lower to the same engine construct — frameless expansions target a
   host plot, framed expansions target the dashboard.
2. **≤2 genuinely native input primitives** — `text_input` (IME,
   clipboard, focus; winit provides IME events, and the editing layer is
   built over the in-repo Typst-based avenger-text stack) and eventually
   a popup/overlay surface. These are native for
   the same reason primitive marks are: they cannot be composed from
   marks. They slot in as primitive marks with state, riding the same
   container.
3. **A focus/keyboard routing service** in the event system (P2, shared
   by text_input and accessibility).

**Is it a kind of Plot: no — but the widget *layer* is.** A `Plot` brings
coordinate systems, scale coordination, guides, measurement, facets, and a
session's cache stack — none of which a checkbox needs. What a widget
needs is `MarkGroup` + state + frame: groups already own local data
contexts (the data-encoded checkbox list is a group with a relation, rect/
glyph/text marks, and a `toggle_clauses` binding — item positions are
arithmetic over row_number, no scales required), and child frames already
exist in the engine (positioned subplots, layout-and-child-frames). So the
recommended architecture puts the Plot-ness **once per dashboard, not once
per widget**:

- The dashboard hosts one hidden **chrome plot** — a zerod-style,
  pixel-space plot spanning the document (the `chart zerod` kind is the
  existing precedent for scale-less pixel-space mark evaluation).
- Every widget instance lowers into it as one hygienically-named
  `MarkGroup` (own data context, own marks and bindings) inside a child
  frame positioned by the dashboard layout solve.
- One `PlotSession` evaluates the whole widget layer: full reuse of the
  compile/evaluate pipeline (data-encoding, conditional channels,
  `datum()`, stores/selections), unified rtree hit-testing and event
  routing with the rest of the scene, CSS theming, and `avenger test`
  baselines — zero parallel evaluation machinery.
- Sizing sequencing: intrinsic hints are arithmetic (measured label text
  via the existing text-measurement machinery, item counts via cheap
  scalar count queries — the §4c "item counts enter layout" rule), so the
  order is *hints → track solve → frames assigned → chrome plot
  evaluates* — simpler than a chart's own measure/solve loop, no
  fixed-point iteration.
- Runtime totals: N chart `PlotSession`s + **1** chrome `PlotSession` +
  the layout solve + the param bus.

**The dashboard is not a Plot (named 2026-07-09).** The "dashboard
driver" is concrete: a compile-time artifact and a runtime state pair
**one level above Plot**, mirroring `CompiledPlot` / `PlotSession` —

- `CompiledDashboard` (artifact): the document layout tree, chart
  instance references (each an ordinary `CompiledPlot`), widget specs,
  and state bindings — the future `.dashboard.avenger` compile target,
  serializable with per-chart baking intact.
- `DashboardState` (driver): implements `SceneGraphBuilder<State>` +
  `EventStreamHandler<State>` — the avenger-app extension traits built
  for exactly this. It owns the N `PlotSession`s + the chrome
  `PlotSession`, the authoritative param/selection bus (per-chart scoped
  stores alias into it), the document viewport (scroll), and runs the
  loop: hints → avenger-layout solve → frame assignment → dirty/visible
  evaluation scheduling → scene composition (each session's scene as a
  translated group in one `SceneGraph`, one rtree) → event routing back
  to the owning session or widget.

Why not `Plot<Dashboard>`: Plot's type contract is precisely what
dashboards must break apart — (1) one coordinate/scale/guide
coordination universe (dashboards coordinate state, not scales); (2)
`compile()` → one `CompiledPlot` → one `evaluate()` as the unit of
serialization, baking, caching, and client-server exchange (dashboards
want N such units with independent lifecycles: lazy tabs, visible-first
priority, per-chart preview/exact modes); (3) one state-registration
scope (dashboard scope sits *above* plot-`Shared`); (4) a fills-imposed-
canvas contract (a document has growing height and a viewport). Plots
remain the only *renderable* atoms — charts are Plots, the widget layer
is a Plot — the dashboard arranges and coordinates but never renders
itself; that is the type-level statement of "coordinated state, not
coordinated scales." No intermediate `Plot<DocFlow>` kind should exist
either: the v0 altitude is the ordinary concat Plot (enriched with
content tracks and aspect rows, both already planned), and the next
rung is the app-level container — a third, in-between thing would be
concat with different defaults, which is neither.

A pleasing consequence: `mark subplot` (a data-driven mark embedding a
plot per key) already lives at the mark level, so when data-encoded
*panel lists* (a KPI card per query row, §4c) arrive, the chrome plot
hosts them with existing machinery — subplot marks inside the widget
layer — rather than a new repetition construct.

**Rust surface sketch (2026-07-09).** Promoted to a full design doc:
`avenger-chart/docs/future-work/widgets.md` (contracts, placement modes,
the four built-ins — Checkbox, CheckboxList, RadioButtonList, Slider —
phases, and open questions). The sketch below is the summary form. The
engine contract mirrors `ChartTool` exactly, plus the two widget-only
concerns:

```rust
/// NOT generic over a coordinate system. ToolExpansion<C>'s generic is
/// load-bearing for tools (a tool expands INTO a host plot and must match
/// its coordinate system); a widget never renders into a host space — the
/// own-frame is its definitional difference — so its marks live in ONE
/// fixed pixel-frame coordinate system. Monomorphism is also what lets a
/// single chrome plot host every widget and lets the dashboard hold
/// Vec<Arc<dyn ChartWidget>>.
pub trait ChartWidget: Send + Sync {
    fn id(&self) -> &str;
    /// Pre-solve intrinsic sizing: text metrics + item-count scalars.
    fn size_hints(&self, ctx: &WidgetSizeContext<'_>) -> WidgetSizeHints;
    /// Same shape as ChartTool::expand; marks author in the widget's
    /// pixel frame and inherit the widget's own data context.
    fn expand(&self, ctx: WidgetExpansionContext<'_>)
        -> Result<WidgetExpansion, AvengerChartError>;
}

pub struct WidgetExpansion {
    pub expansion: ToolExpansion<PixelFrame>,  // params/stores/selections/bindings/marks
    pub data: Option<DataFrame>,               // item relation for data-encoded widgets
}
```

`PixelFrame` is the widget frame's coordinate system: position channels
in pixels, identity transform, `NoGuide` — a sibling of `ZeroDCoord`
(avenger-chart-core/src/zero_d.rs, which legends already render through,
but which has *no* position channels, while widget marks need pixel x/y).
Plain `Cartesian` with identity scales works interim at the cost of
dragging axis/scale machinery along and letting widget authors
accidentally bind data scales. The chrome plot is then `Plot<PixelFrame>`.
Coordinate-system genericity reappears only at the cell boundary
(`WidgetCell` is a `Mark<HConcat>` erasing widget internals, as `Subplot`
does) — a widget wanting exotic internal geometry (a polar gauge) embeds
a plot via subplot machinery rather than parameterizing the trait.

A `CheckboxList` stdlib widget is then: project `value`/`label`, a `Sql`
row-number stage for item order, three data-encoded marks (box rects,
checked-overlay rects behind a `Filter::new(selection.predicate())` —
the same filtered-layer idiom the cross-filter example uses — and text
labels at `__idx * item_height`), and one click binding emitting a
toggle-clause `SelectionUpdate`. Composition needs one new mark type:
`WidgetCell::new(widget)` as the `Subplot::new(plot)` sibling, wrapping
the widget in an implicit pixel-frame plot so concat sizing/placement
treats it like any cell. **Because a concat is one `CompiledPlot`, the
shared selection and cross-filtering work through the existing scoped
state machinery — widgets-in-concat is shippable before any dashboard
runtime exists**, and the same widget later lifts unchanged into the
dashboard's chrome plot.

**Placement modes** (three, same widget type): a concat cell
(`WidgetCell`), the dashboard chrome plot (frames from the document
layout), and **chart-local chrome** — `Plot::widget(w.position(
ChromePosition::TopRight))` occupying a guide slot exactly as legends do
(the legend slot/stacking machinery is the implementation hook, fitting
since `ZeroDCoord` says legends are already "non-spatial mark
rendering"). Chart-chrome is the natural home for a single `Checkbox`
toggling a mark — and param wiring follows the tool pattern exactly
(verified in avenger-chart-tools/src/lib.rs): tools **auto-mint** their
params in `expand()` (`Param::raw_domain(generated_tool_name(id, …))`,
plus an unconditional `enabled` param), return them in
`ToolExpansion.params`, and the compiler registers them automatically
with a `ToolParamSharing` policy (explicit scope or mirror-the-scale);
the `.x_domain_param(param)` builder is only the *sharing override*.
Widgets do the same three tiers: auto-mint by default (param name = the
widget id), expose a consumer accessor paralleling
`Selection::predicate()` — `trend_toggle.checked() -> Expr` for
`Line::new().visible(trend_toggle.checked())` — and accept `.param(&p)`
to share externally. No `.add_param()` ceremony; the DSL-documented
generic mark property `visible:` (an ordinary expression slot) does the
show/hide. The checkbox's own check glyph runs on the same mechanism
(`.visible(param.expr())`), and visibility toggles don't perturb layout
(plot area unchanged → cheap Exact re-eval, no measure churn). Guide
config (axis `grid:`, legend visibility) becomes param-drivable as those
properties gain expression slots — the `subtitle: CASE WHEN $compact…`
example in chart-dsl.md already points that direction.

---

## 4c. Data-encoded widgets (added 2026-07-09)

The language's own law for repetition is "multiplicity is data, not loops" —
marks encode one visual item per row. Applying the same law to widgets
resolves most of the Slint gap #2 (§2) in the language's native idiom: **a
data widget takes a relation and encodes columns to widget channels, one
interactive item per row**, exactly as a mark encodes rows to visual items.

```avenger
selection as regions { empty: all; }             -- empty = no filter

widget checkbox_list as region_filter {
  data: SELECT DISTINCT "region", count(*) AS "n" FROM orders GROUP BY 1;
  label: "region" || ' (' || "n" || ')';         -- channels are SQL expressions
  color: "region";                               -- same ordinal scale the charts use
  checked: selection regions;                    -- membership binding
  sort: "n" desc;
}
```

Anatomy: a **data source** (query, catalog table, or store — stores are
already queryable relations; the DSL already lets marks use `data: store
brush`), **item channels** (`label`, `checked`, `enabled`, `color`, `icon`,
`sort` — SQL expressions over the row, some scale-mapped), and a **state
binding** that both reads per-item state and receives writes. Three binding
families, mirroring the three state constructs:

- **Selection membership** (the checkbox/chip/legend family): `checked:
  selection regions;` reads per item as
  `selection_contains(regions, datum(...))` — the same predicate machinery
  conditional channels already use — and toggling emits `toggle_clauses`,
  the same update a point-selection click on a mark emits. A checkbox list
  is *semantically identical to clicking marks*; the widget is an alternate
  rendering of an existing interaction. Cross-filtering falls out because
  charts already consume selections.
- **Store column** (the view-model family, the "store as the data for a
  list of checkboxes" reading): the store is simultaneously row source and
  write target —

  ```avenger
  store as region_prefs {
    field region: utf8;
    field checked: boolean;
    primary_key: [region];
    init: SELECT DISTINCT "region", true FROM orders;   -- query-seeded store (new, small)
  }

  widget checkbox_list as region_filter {
    data: store region_prefs;
    label: "region";
    checked: "checked";           -- toggle upserts the row by primary key
  }
  ```

  Downstream consumption is plain SQL:
  `WHERE "region" IN (SELECT "region" FROM region_prefs WHERE "checked")` —
  Mosaic-style semijoin cross-filtering, no new predicate machinery.
- **Scalar param** (the single-select family): radio lists and selects bind
  `value: $param;` — the degenerate case, already in the §3 sketch's
  `options:` property; this section generalizes it.

What falls out for free:

- **The interactive legend.** A legend *is* a checkbox list whose `color`
  channel rides the same shared ordinal scale the charts use — the classic
  legend-toggle cross-filter (Plotly/Highcharts style), stateful, theme-
  and scale-consistent, with zero legend-specific machinery.
- **Cascading controls.** Item queries are ordinary SQL over params
  (`data: SELECT DISTINCT "city" FROM t WHERE "country" = $country;`), so
  country→city dropdowns are just param-dependent relations re-executing
  on rebind.
- **Facet-count badges** ("Region (12)") — an aggregate in the label
  expression, as in the example above.
- **Empty-set semantics** already exist (`empty: all | none` on
  selections): all-checked-by-default filters are `empty: all`.

Design points to settle:

- **Reconciliation when the item set changes** (a checked region vanishes
  after an upstream filter): selections tolerate this naturally — a clause
  for an absent value matches nothing and revives if the value returns;
  query-seeded stores need a re-init policy (re-run `init:`, preserving
  overlapping rows by primary key — upsert semantics).
- **Long lists vs the no-panel-scroll law.** A 500-item list in flow mode
  grows the document. Options: treat bounded item-list viewports as
  *widget chrome* (like combo popups, which already scroll internally) via
  `max_items_visible`, and/or the standard search-box-plus-list pattern (a
  `filter:` text param). Either keeps the law intact for *panels*.
- **Item counts enter layout.** Closed-form layout now needs item
  cardinalities (one cheap count query per data widget) — still no content
  measurement, so the zero-layout-shift property survives given (width,
  params, item counts).

**Data-encoded layout components** (a card per query row, a section per
group) are the same idea one level up, and the chart layer already contains
the precedent: `mark subplot { key: "category"; plot cartesian { ... } }` is
data-driven multiplicity of embedded plots, and facet/repeat are its
grid-shaped cousins. A dashboard-level `panel_list { data: ...; key: ...;
panel { ... } }` should reuse that shape when demand appears — keep it out
of v1 (KPI-card rows are its first real customer; data-driven *tabs* are
probably an anti-pattern). New machinery this section actually requires:
query-seeded stores (`init:`), per-item event context for widgets (reusing
`datum(...)`), and widget channels that can ride shared scales.

---

## 5. Positioning (why this and not an existing thing)

- **Mosaic (UW IDL)** is the closest prior art and a strong validation:
  coordinator + params/selections lowered to SQL predicates over DuckDB,
  per-view caching. It is a JS library with JSON specs; this proposal is the
  same coordination semantics as a *language* with a native+wasm GPU engine,
  pinned imports, and a test/doc toolchain.
- **Grafana** proves `$var`-in-SQL dashboards at industrial scale (template
  variables interpolate into panel queries) — but as untyped JSON documents,
  web-only, with no composition or integrity story.
- **Evidence / Rill / Observable Framework** prove SQL-first
  dashboards-as-code; none has a real interaction/selection language, a
  native renderer, or a component model.
- **Streamlit / Dash / Shiny** are host-language app frameworks: rerun- or
  callback-driven, not documents, not statically analyzable, not pinnable.
- **Using Slint itself as the dashboard layer** is technically viable — its
  wgpu-texture import (`slint::Image::try_from(wgpu::Texture)`, feature-gated
  per wgpu major) puts custom GPU content in the scene graph as a first-class
  element, and Slint's Energy Monitor demo shows dashboard demand (charts
  hand-built from Path primitives, no data layer). It remains a fine path for
  bespoke *products*. But as the dashboard *document* it means two languages,
  two reactive systems, bindings that cannot see the data (Slint properties
  vs SQL/catalog — every query result must be marshalled through host code
  into Models), a wgpu-version coupling at the texture boundary, and no
  pinning/baselines/capability model. The value of a native layer is
  precisely that the coordinator speaks the same two primitives as the
  charts.

The differentiated position: **a hash-pinned, capability-sandboxed, SQL-native
dashboard document that renders identically on native GPU, wasm, PNG, and PDF,
with Slint-grade declarative reactivity and a component model whose components
are ordinary chart files.**

---

## 6. Suggested phasing

- **P0 — the coordinator, no widgets.** `dashboard` file kind: import +
  instantiate charts, dashboard params with aliasing/derived bindings,
  dashboard `table sql`, document-flow layout + text panels. Charts' own tools
  (pan/zoom, point/box select) are the only input devices. This alone
  delivers linked zoom, cross-filtering, and shared-view dashboards — and
  forces the state-scope, theme, and evaluation-scheduling decisions while
  the surface is small.
- **P1 — mark-built data widgets + scalars** (§4b revised verdict, §4c).
  `define widget` (tool-style state + events, mark-style face, own frame,
  layout-leaf sizing); `std:widgets/` for the chart-shaped set — checkbox,
  checkbox_list, chips, radio, select-as-list, slider, range_slider,
  button, interactive legend; scalar SQL bindings (KPI text, options,
  extents) on the async-scalars machinery; tabs (param sugar).
- **P2 — the native-primitive edge.** `text_input` (IME/clipboard via
  winit + an editing layer over the Typst-based avenger-text stack),
  popup/overlay surface (combo menus), focus
  and keyboard traversal, AccessKit semantic nodes, scroll-feel polish;
  host `callback`s, client-server dashboard serving and baked publishing.

Each phase is independently shippable and independently testable with the
existing baseline machinery (`avenger test` renders the dashboard PNG).

---

## 7. Open questions

- Keyword and file kind: `dashboard` (scope-honest) vs `app` (broader);
  `.dashboard.avenger` extension mirroring definition files?
- Is a dashboard importable by another dashboard (nesting), and if so is its
  public surface its params/selections exactly as for charts?
- Param binding conflict rules: is aliasing always two-way, and is a derived
  binding on a tool-written param a hard error or a shadow-with-warning?
  Should chart params gain Slint-style access qualifiers (`in` = never
  written by the chart's own tools, compiler-enforced) so the interface
  reads off the declaration, or is whole-program write analysis enough?
- Do selections alias by clause-set identity, or lower to predicates at the
  boundary (Mosaic-style)? (Predicate lowering is weaker but simpler and
  matches `Selection::predicate()`.)
- Per-instance data remapping: ambient-catalog-only, or a `source:` binding?
- Theme precedence across the dashboard/chart boundary.
- Widget accessibility: AccessKit semantic nodes in the scenegraph, or a
  parallel a11y tree owned by the dashboard host?
- Does `avenger serve`'s project gallery grow into the dashboard host, or is
  the dashboard runtime a separate `avenger-dashboard` crate over
  `avenger-chart-egui`-style panels?
- How do baked (partially evaluated) dashboards interact with per-viewer
  params (row-level security lives in the catalog's parameterized tables)?
- Do widget groups need a `form`/on-submit commit mode (Streamlit-form-style
  staged param writes, for expensive queries where per-keystroke reactivity
  is wasteful), or are per-widget `debounce_ms` / `commit: on_change |
  on_submit` properties enough? Staged-but-uncommitted values would be
  ephemeral UI state (like a text cursor), not document state.
