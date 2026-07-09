# Dashboard Layout

## Status

Draft design, 2026-07-09. The layout model for the dashboard layer — the
document format that composes charts, widgets, and text panels above the
chart level — and a precise answer to one question: **can it reuse the
primitives in `avenger-layout`? Yes, almost entirely natively**; the
receipts are in [Reusing avenger-layout](#reusing-avenger-layout-the-receipts).

Companions: `widgets.md` (the widget paradigm whose size hints feed this
layout; `WidgetCell` and content tracks are its phase 4),
`dashboard-layer.md` (the full dashboard-layer
exploration this distills the layout portion of), and `layout.md`, whose
closing note — "arbitrary dashboard composition remains a separate design
spike" — this document is.

Architecture context (established in the exploration, summarized in
[The Driver](#the-driver)): a dashboard is **not** a `Plot`. It is a
`CompiledDashboard` artifact plus a `DashboardState` driver at the
avenger-app altitude, owning N per-chart `PlotSession`s, one chrome
`Plot<PixelFrame>` for widgets, the param/selection bus, and this layout.

## The Model

**Document flow, with a one-screen opt-in.** The default (`flow`): fixed
width (viewport-fill, optionally max-clamped for the centered-column
look), height grows with content, **one scroll on the document, never
inside a panel**. The opt-in (`fill`): height bound to the viewport, row
heights become tracks over the fixed total — the Streamlit/Dash one-screen
app shape. Same grammar, two readings: flow reads as a document, fill
reads as a grid.

**A closed shell of named slots.** `header`, `footer`, `sidebar left`,
`sidebar right`, and the implicit `content` — the "holy grail" template
every dashboard framework converged on (HTML5 landmark regions, bslib
`page_sidebar`, egui's edge-claiming panels). Slots are chrome: pinned
outside the document scroll by default (`header` may opt into scrolling
away; `footer` chooses `document` end-of-flow vs `pinned` status bar), and
a sidebar whose stack exceeds the viewport scrolls *internally* — legal
because slots are chrome, not panels. Deliberately **not** a general
named-area system (CSS `grid-template-areas`): the closed set is
enumerable for schema/completion, maps to accessibility landmarks and to
page headers/footers in paginated PDF export, and exotic shells compose by
nesting inside `content`.

**Content is an ordered flow of rows; each row carries a track grid.**
Rows are the vertical axis — appending grows the document. Within a row,
tracks split the width (`px` / `fr` / content — the same sizing tokens as
concat). Two-dimensional compositions nest (a track cell holds a `column`
of rows); span-heavy 2-D arrangements belong to a concat chart *inside* a
panel, per the two-altitude law (concat composes aligned plots; the
dashboard composes independent panels).

**Every region hosts the same grammar.** A sidebar is a narrow column of
rows (widgets, text, small panels); a header is typically one row. No
slot-specific content languages.

**Sizing law: the dashboard imposes, leaves hint.** Charts never self-size
— their heights are declared (`px(n)` or `aspect(w, h)` of the solved
track width). Widget and text leaves report precomputed size hints (text
metrics, item counts — `widgets.md`'s sizing pass). Rows are `px`,
`aspect`, or content-sized (the last legal only for hint-bearing leaves).

**The invariant everything protects:** layout is **closed-form given
(width, params, item counts)** — no content measurement, no
measure/re-solve loops. Consequences: the scroll extent is exact before
any chart evaluates, lazy evaluation of off-screen panels causes zero
layout shift, and scroll position is a clean evaluation-priority input.
Tabs and modals are param-driven visibility (sugar over the param bus),
so layout stays closed-form *given params* — the active tab merely selects
which declared heights sum.

## Vocabulary And External Mapping

The grammar deliberately speaks the dashboard-framework lingua franca:

| Construct | CSS | Streamlit | Dash/Shiny | egui | Typst |
| --- | --- | --- | --- | --- | --- |
| `sidebar` / `header` / `footer` | sticky/fixed regions | `st.sidebar` | `sidebarLayout()` | `SidePanel` / `TopBottomPanel` | — |
| ordered rows, height grows, document scrolls | block flow | implicit append | `fluidRow()` | top-down `Ui` | flow |
| row tracks | `grid-template-columns` | `st.columns([2,1])` | `column(width=4)` | `StripBuilder` | `grid(columns: (280pt, 1fr))` |
| `fr` | CSS Grid `fr` | — | — | `Size::remainder` | `fr` (same unit) |
| `aspect(w, h)` row height | `aspect-ratio` | — | — | — | — |
| content sizing (from hints) | `auto` | automatic | automatic | frame memory | measure |
| `fill` height policy | `100vh` app | (hacks) | `fillPage()` | `CentralPanel` | — |
| tabs | — | `st.tabs` | `dcc.Tabs` | manual | — |
| panel + title/frame chrome | card | `st.container` | `dbc.Card` | `Frame` | block |

**The words deliberately absent are the vocabulary's defining feature:**
no `grow`/`shrink`/`basis`, no justify/align axes as authoring surface, no
flex wrap. Those words describe *content negotiation*, and this model has
none beyond precomputed leaf hints. The vocabulary you cannot say is the
drift you cannot have (see [Out of Scope](#deliberately-out-of-scope) on
taffy).

Grammar decision, recorded: structure uses **shell slots + flow rows**
(the Streamlit/Shiny shape) rather than reusing concat's fixed
`rows × columns + cell at {}` grid grammar — flow rows encode the
height-grows document directly, while a fixed grid is viewport-shaped and
makes sidebars awkward (span-all-rows cells). The *sizing tokens* are
shared with concat verbatim, so authors learn `px`/`fr`/content once.
Spans remain a concat-grid feature.

## Reusing avenger-layout: the receipts

`avenger-layout`'s public surface (verified against the crate, not its
description) covers the model natively. The mapping:

| Dashboard need | avenger-layout primitive (existing) |
| --- | --- |
| document flow: width imposed, height grows | `SolveOptions { width: Some(w), height: None }` — per-axis optional root sizing — with the root column's `sizing_y(SolveFor::Envelope)` ("content is given; the envelope is the sum of content plus chrome") |
| `fill` mode: height imposed, `fr` rows | `SolveOptions { height: Some(vh) }` + `TrackSize::Flex(fr)` row tracks |
| row/column structure, nesting | `Layout::row` / `Layout::column` / `Layout::grid` (1×N conveniences exist; grids nest arbitrarily) |
| track sizing `px` / `fr` / content | `TrackSize::Fixed(px)` / `TrackSize::Flex(fr)` / `TrackSize::Auto` ("content-sized: the max of the items measured into the track") — **content tracks are native**; `widgets.md`'s `TrackSize::content()` is only a chart-API surface over this |
| hint-sized widget/text leaves | `Layout::leaf(Size)` — leaves are exactly "a measured region the solver cannot see inside" |
| imposed-size chart panels | the same leaves, sizes computed by the driver (`px` or aspect arithmetic) |
| panel titles, borders, spacing chrome | declared chrome: "named slabs per side (margin, repeatable strips, legend, guide)" with positioned `ChromeSlab` readback |
| gaps and gutters | per-axis `Spacing`, the gap law |
| alignment within slots / leftover distribution | `CellAlign` (align-self) and `Distribute` (start/center/end/space-between) |
| frame assignment readback | `LayoutSolution::region(id)` → slot allotment + honest content rect + chrome slabs + `SolvedTracks` |
| layout debugging | `LayoutSolution::to_svg()` and `svg_panels` — dashboards inherit the baseline-gallery workflow |

**"Simple mode," defined precisely** — the subset the dashboard driver
uses, and the machinery it deliberately leaves idle:

- **No share keys.** Cross-panel plot-area alignment is a chart-level
  concat concern (the two-altitude law); `Layout::share` stays unused —
  though its existence means opt-in cross-panel alignment has a mechanism
  waiting if it ever graduates to a feature.
- **No edge-demand strata.** Charts arrive as opaque imposed-size leaves;
  `EdgeDemand`/`EdgeGrant` overflow negotiation never engages.
- **No re-measure loop.** The crate's own contract is "one solve, no
  loop," with `content_delta` offered for callers whose measurements
  depend on allocations (chart tick labels). Dashboard leaves are
  constants, so the loop count is exactly one — the closed-form invariant
  expressed in avenger-layout's terms.

**The aspect recipe.** `aspect(w, h)` heights are the one construct with
no direct primitive, and avenger-layout already takes a position on this:
aspect is a documented *caller-side recipe* (the `aspect_contain_fit_slack`
baseline: "the caller-side aspect recipe"). The dashboard driver's version
is degenerate because row track widths never depend on heights: resolve
track widths (closed-form for `Fixed`/`Flex`; from hints for `Auto`),
compute `height = width * h / w` per aspect panel, build the leaves at
final sizes, solve once. Arithmetic between two passes of the driver —
zero solver changes.

**Conclusion: no anticipated changes to `avenger-layout`.** Everything new
lives above it: the document-flow driver (region composition, aspect
arithmetic, hint collection) in the dashboard crate, and thin authoring
surfaces (`Row`/`Sidebar` builders, concat's `TrackSize::content()`) at
the chart-API level.

## The Driver

The driver is `DashboardState` (avenger-app altitude, implementing
`SceneGraphBuilder<State>` + `EventStreamHandler<State>`; see the
`dashboard-layer.md` for the full not-a-Plot rationale). Its layout duty per
viewport change or hint change:

1. Collect leaf hints (widget/text sizes — precomputed constants from the
   `widgets.md` sizing pass; item counts via cheap scalar queries).
2. Solve each **pinned region** independently at viewport-derived sizes:
   sidebars at `(width_px, viewport_h)`, pinned header/footer at
   `(content_width, height)`.
3. Solve **content** as a root `Layout::column` of rows —
   `SolveOptions { width: Some(content_width), height: None }` in flow
   mode (aspect arithmetic between the track pass and the final solve),
   or `height: Some(viewport_h)` in fill mode.
4. Read frames from `LayoutSolution` regions; assign to `PlotSession`s
   (imposed `CanvasDimensions`), widget groups (chrome-plot child
   frames), and text panels.
5. Scroll is a runtime viewport (translate + clip + wheel) over the
   solved document — never a solver concern; scroll position feeds
   evaluation priority (visible panels first).

Rust sketch of step 3, in actual avenger-layout calls — this is the whole
trick:

```rust
use avenger_layout::{CellAlign, Layout, SolveFor, SolveOptions, TrackSize};

fn content_layout(rows: &[RowSpec], width: f32) -> Layout<PanelId> {
    Layout::column(rows.iter().map(|row| {
        let track_widths = row.resolve_track_widths(width);   // closed-form
        Layout::row(row.panels.iter().zip(&track_widths).map(|(p, w)| {
            let h = row.height.resolve(*w);                   // px | aspect(w) | hint
            Layout::leaf(Size::new(*w, h)).id(p.id.clone())
        }))
        .columns(row.tracks.clone())                          // Fixed / Flex / Auto
    }))
    .sizing_y(SolveFor::Envelope)      // height = sum of rows: document flow
}

let solution = content_layout(&rows, content_width)
    .solve(&SolveOptions { width: Some(content_width), height: None })?;
let frame = solution.region(&panel_id).unwrap().content;      // → CanvasDimensions
```

## Two Altitudes Of Sufficiency

- **Today:** small and medium dashboards can be ordinary concat charts —
  one `CompiledPlot`, `WidgetCell` cells, shared params/selections through
  the existing scoped state machinery, avenger-layout already solving
  placement. Limits: one coupled measurement pass, no lazy tabs or
  visible-first scheduling, fixed canvas (the document scroll needs the
  runtime viewport either way).
- **Later:** the `DashboardState` driver — N sessions, the state bus,
  lazy/priority evaluation, the scrolling document — keeping the same
  track vocabulary and the same solver, swapping only the semantics
  (coordinated state instead of coordinated scales).

## Implementation Phases

1. **The doc-flow driver over avenger-layout.** Rows/tracks/nesting, flow
   + fill policies, aspect arithmetic, hint intake; frame assignment to
   sessions; static export (PNG at content height; PDF paginating at row
   boundaries). Exit: `dashboard-layer.md`’s exec-overview sketch renders
   from the Rust builder.
2. **Shell slots + the viewport.** Pinned header/sidebars/footer as
   independent region solves; the scroll viewport with wheel/touch and
   scroll-linked evaluation priority; `fill { min_height }` degrade.
3. **Content tracks + widget integration.** `TrackSize::content()`
   surfaces on rows and concat (`widgets.md` phase 4 is the same work);
   sidebar widget stacks end to end.
4. **Tabs and modals** as param sugar over visibility; baseline coverage
   as param sweeps.

## Deliberately Out Of Scope

- **taffy** (or any external layout engine): the pinned model reduced
  dashboard layout to closed-form track arithmetic over precomputed
  hints; taffy solves content-negotiated flexbox/CSS-grid — problems this
  model deliberately does not have. (Slint's own core H/V/Grid layouts
  are a hand-rolled one-pass box solve; it reaches for taffy only for its
  CSS-flexbox add-on.) Wanting taffy later would be a symptom of drift
  into app-toolkit layout, not a gap.
- **Flexbox vocabulary** (`grow`/`shrink`/`basis`, justify/align axes,
  flex wrap) — see the absent-words rule above.
- **General named grid areas** — the closed slot set covers real shells;
  exotic ones nest.
- **Spans at the dashboard level** — a concat-grid feature, by the
  two-altitude law.
- **Per-panel scrolling** — the model's defining prohibition; long
  content paginates (tables) or grows the document; bounded item-list
  viewports inside widgets are widget chrome, not panel scroll.
- **User-rearrangeable docking/tiling** (egui_tiles/Rerun-blueprint
  style) — a possible far-future *authoring* mode whose natural form is
  design-tool round-trip (drag writes the document), not a runtime layout
  system.

## Open Questions

- Slot naming and set: is `header` the only top slot (no separate
  `topbar`), and does `footer` default to `document` or `pinned`?
- PDF pagination details: repeat pinned header on every page? where does
  a sidebar render in print (first page margin column vs folded into the
  flow)?
- Resize cadence: reuse the chart resize throttle (Preview-grade solve
  during drag-resize, settle to Exact) for document re-solves?
- Should `Auto` tracks accept a cap (`max_px`) at the dashboard level, or
  is that widget-side (`max_items_visible`, label truncation)?
- `fill` mode with over-budget content: clip, degrade to scroll below
  `min_height`, or both as declared policy?
- Does the driver ever want avenger-layout chrome slabs for panel
  titles/frames, or do titles render as ordinary text rows inside the
  panel frame (simpler, one less concept)?
- When the DSL arrives: does the dashboard grammar's `row { widths: [...] }`
  reuse the concat `TrackSizing` serde forms verbatim (leaning: yes)?
