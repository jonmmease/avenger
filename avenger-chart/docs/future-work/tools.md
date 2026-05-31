# Tools And Interactivity

## Goal

Avenger should support reusable chart tools for rich interactions such as
pan/scroll-zoom, brushing, point and bar selection, hover highlighting, linked
views, tooltips, annotation drawing, and reset actions. Applying a common tool
should require little more than adding the tool to a plot.

The near-term target is a built-in pan/scroll-zoom tool that packages the
working event-binding examples into a reusable API.

```rust
use avenger_chart::tools::PanScrollZoom;

let plot = Plot::<FacetWrap>::new()
    .data(df)
    .mark(Subplot::new(leaf).wrap(col("group")))
    .tool(PanScrollZoom::cartesian());
```

For most Cartesian charts, this should replace the current manual work of
declaring raw-domain params, attaching those params to scales, writing drag and
wheel event bindings, and choosing Preview/Exact behavior.

## Existing Foundation

The current implementation already has the pieces a tool should compose:

- `avenger-eventstream` routes host events, tracks `between(...)` state, and
  provides start and previous event context.
- `avenger-chart` has serializable plot-level `ChartEventBinding` specs whose
  filters and assignments are normal DataFusion expressions.
- Event bindings can read pointer positions in coordinate data space through
  derived interaction columns such as `event_coord("x")`,
  `start_coord("x")`, `event_at_start_coord("x")`, `event_domain("x")`, and
  `start_domain("x")`.
- `Param::raw_domain(...)`, `Scale::raw_domain(...)`, and
  `Plot::add_param_with_sharing(...)` make pan/zoom state ordinary chart
  params, including `Sharing::{Free, Level(N), Shared}` behavior in facets.
- `PlotSession` supports `EvaluationMode::Preview` for high-frequency updates
  and `EvaluationMode::Exact` for settled evaluation.
- `EvaluatedPlot` carries interaction scopes so event handlers can route a
  pointer event to the correct coordinate scope before expression evaluation.

Tools should be a packaging layer over these primitives, not a separate event
runtime.

## Tool Model

A chart tool is an authoring-time package that expands into ordinary chart
specification pieces:

- generated params, usually namespaced and hidden from normal chart authors,
- scale or channel configuration changes,
- plot-level event bindings,
- optional overlay marks or layers,
- optional app-facing metadata for toolbars and diagnostics.

The app runtime should continue to execute normal event bindings and normal plot
evaluations. A tool should not require a parallel event dispatcher when its
behavior can be represented as params plus event bindings.

```rust
pub trait ChartTool<C>: Send + Sync {
    fn id(&self) -> &ToolId;
    fn expand(&self, ctx: ToolExpansionContext<C>) -> Result<ToolExpansion>;
}

pub struct ToolExpansion {
    pub params: Vec<(Param, Sharing)>,
    pub event_bindings: Vec<ChartEventBinding>,
    pub scale_edits: Vec<ToolScaleEdit>,
    pub overlays: Vec<ToolOverlaySpec>,
}
```

The concrete API does not need to expose this exact trait first. V1 can use
serializable built-in tool specs plus an internal expansion step, then expose a
custom-tool trait once the built-in tools prove the boundary.

## Names And Isolation

Every tool instance should have a unique author-visible id within a plot:

```rust
.tool(PanScrollZoom::cartesian().id("main_nav"))
```

Tool ids should reject periods. Generated local names can then be unambiguous
without asking authors to think about DataFusion placeholder syntax. Generated
params should use a reserved physical prefix, for example:

```text
__tool_main_nav__x_domain
__tool_main_nav__y_domain
__tool_main_nav__enabled
```

Tools may expose selected outputs through typed accessors instead of asking
authors to know generated names:

```rust
let brush = BrushSelection::new("select");
let selected = brush.predicate_expr();
```

Multiple tools can update the same explicit author param only when the author
opts into that sharing. Generated params default to per-tool isolation.

## Pan/Scroll-Zoom Tool

The first tool should package the current Cartesian pan and scroll zoom
examples.

```rust
let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(Symbol::new().x(col("x")).y(col("y")))
    .tool(PanScrollZoom::cartesian());
```

Useful options:

```rust
PanScrollZoom::cartesian()
    .id("nav")
    .x_channel("x")
    .y_channel("y")
    .drag_button(MouseButton::Left)
    .scroll_zoom(true)
    .zoom_base(1.02)
    .consume_wheel(true)
    .settle_exact(ToolSettle::OnPointerUp)
```

### Expansion

For Cartesian `x` and `y`, expansion should:

1. Add generated raw-domain params.
2. Add `Scale::raw_domain(param.expr())` to the target continuous positional
   scales.
3. Add drag-pan bindings equivalent to:

   ```rust
   let dx = ev::event_at_start_coord("x") - ev::start_coord("x");
   set x_domain = interval(start_domain("x").start - dx,
                           start_domain("x").end - dx)
   ```

4. Add scroll-zoom bindings equivalent to:

   ```rust
   let factor = pow(zoom_base, -wheel_delta_y);
   set x_domain = zoom_interval(event_domain("x"), event_coord("x"), factor)
   ```

5. Use `Preview` for drag and wheel updates.
6. Optionally request `Exact` on pointer-up, idle, or explicit reset.

The tool should generate the same event-binding machinery currently used in
`avenger-chart-app/examples/common/mod.rs`, but move that code into a chart
tool module instead of leaving it as example-local helper code.

### Scale Sharing

Pan/scroll-zoom should follow scale-domain ownership.

- If the target channel scale is `Shared`, the generated raw-domain param is
  `Sharing::Shared`.
- If the target channel scale is `Free` or `Level(0)`, the generated raw-domain
  param is scoped per leaf coordinate plot.
- If the target channel scale is `Level(N)`, the generated raw-domain param uses
  the same `Sharing::Level(N)`.

This matches the current faceted pan behavior: dragging one cell patches the
owner scope for that channel, and domain sharing propagates the result to the
right sibling cells. `FacetWrap` counts as one logical level, so `Level(1)`
means the whole wrap group.

The tool should reject mismatches that would create incoherent behavior:

- target scale is not numeric continuous,
- coordinate scope does not export the requested interaction channel,
- an existing `raw_domain` is already configured and no explicit override was
  provided,
- a requested sharing level is narrower than the target scale domain owner.

### Explicit Overrides

The low-code path should infer scale sharing and generated params, but advanced
users need escape hatches:

```rust
PanScrollZoom::cartesian()
    .x_sharing(Sharing::Shared)
    .y_sharing(Sharing::Free)
    .x_domain_param(existing_x_domain)
    .y_domain_param(existing_y_domain)
```

If a tool sets sharing explicitly, it should either apply the same scale-sharing
override to the target channel scales or return a validation error if that would
be ambiguous. Silent disagreement between param sharing and scale sharing should
not be allowed.

## Toolbar And Enablement

Each tool should have a generated enabled param. Event bindings emitted by the
tool include an enabled filter. A future toolbar can enable exactly one tool or
multiple compatible tools by patching these params.

```rust
PanScrollZoom::cartesian()
    .id("nav")
    .enabled_by_default(true)
```

Toolbars are app UI, not chart layout. The chart side should expose tool
metadata such as id, label, icon kind, enabled param name, and mutually
exclusive group. The Winit/WGPU app can render the toolbar chrome outside the
chart scenegraph.

## Selection, Brush, Tooltip, And Annotation Tools

Pan/scroll-zoom is the first tool because it requires no overlay marks and
already works through raw-domain params. The same model should extend to richer
tools:

- A brush tool emits params for start/current/end points and may add a rectangle
  overlay mark driven by those params.
- A click-selection tool uses hit-test metadata to patch a selected datum,
  group, or facet path param.
- A hover tool patches a hover param and may add tooltip marks or hand off to
  app UI.
- An annotation tool patches annotation params and emits overlay marks while a
  drag is active.

These require additional identity contracts. The next missing pieces are stable
mark/datum identity, hit-test-to-data mapping, and a clean overlay mark layer
that can render above the chart without becoming part of the user’s data mark
tree.

## Crate Boundary

V1 should live in `avenger-chart` as built-in chart tool specs, with
`avenger-chart-app` consuming the expanded event bindings exactly as it does
today. A later `avenger-chart-tools` crate can become a peer built-in tool crate
if the crate split makes that cleaner.

Custom coordinate crates should not need `avenger-chart-cartesian` to support
tooling. A generic domain navigation tool can operate on named interaction
channels as long as the coordinate system exports invertible interaction scopes
and the target scales accept `raw_domain`.

## Implementation Plan

1. Add the authoring surface:
   - `Plot::tool(...)` and `Plot::tools(...)`.
   - `ToolId` validation and duplicate-id errors.
   - an internal `ToolExpansion` step before or during plot compilation.
   - tests that expanded params and event bindings serialize in `CompiledPlot`.

2. Move the example pan/zoom binding helpers into a built-in tool module:
   - generate drag-pan and scroll-zoom event bindings,
   - support Preview and settle policy,
   - keep the existing examples working by switching them to the tool API.

3. Add raw-domain scale installation:
   - recursively find descendant coordinate plots for target channels,
   - attach generated raw-domain params to target scale configs,
   - derive or validate `Sharing`,
   - reject categorical or non-invertible channels clearly.

4. Prove faceted behavior:
   - single Cartesian chart,
   - `FacetWrap` with shared x/y,
   - `FacetWrap` with shared x and free y,
   - row × column facets with `Level(1)`,
   - pointer events outside plot areas remain no-ops,
   - release does not jump relative to one-shot exact evaluation.

5. Add app-facing tool metadata:
   - tool id, label, enabled param, toolbar group,
   - no toolbar UI yet unless needed for manual testing.

6. Design the next identity-dependent tool:
   - prefer brush selection before tooltip or click selection,
   - add only the minimal mark/datum identity needed for that tool,
   - keep generated params and event bindings as the execution substrate.

## Readiness

Pan/scroll-zoom is ready for an implementation plan. The hard runtime pieces
already exist; the remaining work is mostly authoring ergonomics, scale-config
rewriting, validation, and tests.

Brush and selection tools are ready for design spikes after pan/scroll-zoom is
packaged. They should not be implemented until mark/datum identity and overlay
layer semantics are pinned down.
