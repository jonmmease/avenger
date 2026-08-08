# Tools And Interactivity

## Current Foundation

Avenger has a chart tool subsystem for packaging reusable interactions as
ordinary chart primitives. The contracts needed to implement tools live in
`avenger-chart-core`; built-in tool implementations live in
`avenger-chart-tools`. Tools expand during chart compilation into generated
params, scale edits, event bindings, and app-facing metadata. `CompiledPlot`
stores only the expanded result, so `avenger-chart-app` continues to execute the
same event-binding and `PlotSession` evaluation path used by hand-authored
interactions.

The current public tool API is:

```rust
use avenger_chart::prelude::*;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(Symbol::new().x(col("x")).y(col("y")))
    .tool(PanScrollZoom::cartesian());
```

The public expansion contracts in `avenger-chart-core` are:

- `ChartTool`
- `ToolBehaviorExpansion`
- `ToolExpansionContext`
- `ResolvedStateDeclaration`
- `ToolParamSharing`
- `ToolScaleEdit`
- `ToolExport`
- `CompiledToolBehavior`
- `ToolMetadata`

Tool expansion can contribute typed params, stores, selections, ordered event
bindings, containing-plot scale edits, ordinary marks, exports, component/part
provenance, and metadata. Built-in interaction chrome should use those public
primitives wherever possible.

The author-facing tool id is a diagnostic name, not runtime identity. The
compiler allocates an opaque deterministic `ToolInstanceId` before expansion;
two instances of the same tool therefore own distinct typed state and mark
identities. Generated source names remain useful for Rust diagnostics and host
bindings, but runtime mutation and export lookup retain the resolved IDs.
Built-ins currently use diagnostic names in the reserved tool namespace:

```text
__tool_{id}__enabled
__tool_{id}__x_domain
__tool_{id}__y_domain
```

The default `PanScrollZoom` id is `pan_scroll_zoom`.

## `PointSelection`

`PointSelection` packages click-based equality selection. It contributes a
neutral `Selection`, an enabled param, event bindings for click replace,
optional shift-click toggle, optional double-click clear, and tool metadata.
It does not contribute marks, stores, or scale edits.

```rust
let picked = PointSelection::new("picked")
    .field("category")
    .shift_toggle(true)
    .double_click_clear(true);

let plot = Plot::<Cartesian>::new()
    .tool(picked.clone())
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .fill_with(lit("#b8beca"), |c| {
                c.no_scale()
                    .when_value(picked.predicate(), lit("#2563eb"))
                    .no_legend()
            }),
    );
```

Single-field point selection uses `SelectionClauseUpdate::equality_value(...)`
so replace and toggle bindings share the same clause identity. Multi-field
point selections require an explicit clause id.

## `LassoSelection`

`LassoSelection` packages rendered-geometry lasso selection. It contributes a
neutral `Selection`, an enabled param, a between-stream cursor-move binding that
accumulates `ev::event_path()`, a scene-geometry polygon query, optional
double-click clear, and tool metadata. The query collects datum tuples from
rendered marks and lowers them to equality selection clauses, so sibling plots
can use the same `Selection::predicate()` for cross-highlighting.

```rust
let picked = LassoSelection::new("picked")
    .field("point_id")
    .event_path_min_distance_px(6.0)
    .facet_scope(Sharing::Shared);

let plot = Plot::<Cartesian>::new()
    .tool(picked.clone())
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .fill_with(lit("#b8beca"), |c| {
                c.no_scale()
                    .when_value(picked.predicate(), lit("#2563eb"))
                    .no_legend()
            }),
    );
```

`LassoSelection` is semantic selection packaging only. Editable lasso chrome
continues to use lower-level stores and ordinary marks, and richer chrome
tooling remains future work.

## `PanScrollZoom`

`PanScrollZoom` is a built-in chart tool and lives in
`avenger-chart-tools`. It packages Cartesian drag-pan and wheel zoom by
generating raw-domain params, installing those params on the target scales, and
emitting DataFusion-backed event bindings.

Common options:

```rust
PanScrollZoom::cartesian()
    .id("nav")
    .x_channel("x")
    .y_channel("y")
    .x_only()
    .y_only()
    .x_domain_param(existing_x_domain)
    .y_domain_param(existing_y_domain)
    .x_sharing(Sharing::Shared)
    .y_sharing(Sharing::Free)
    .drag_button("left")
    .scroll_zoom(true)
    .zoom_base(1.02)
    .consume_wheel(true)
    .settle_exact(false)
    .enabled_by_default(true);
```

The generated drag binding updates domains using the coordinate value under the
pointer at the start of the drag and the coordinate value under the current
pointer:

```rust
let dx = ev::event_at_start_coord("x") - ev::start_coord("x");

set x_domain to interval(
    interval_start(start_domain("x")) - dx,
    interval_end(start_domain("x")) - dx,
)
```

The generated wheel binding zooms around the current pointer coordinate:

```rust
let factor = pow(zoom_base, -wheel_delta_y);

set x_domain to interval(
    event_coord("x") + (interval_start(event_domain("x")) - event_coord("x")) * factor,
    event_coord("x") + (interval_end(event_domain("x")) - event_coord("x")) * factor,
)
```

Drag and wheel updates use `EvaluationMode::Preview`. `settle_exact(true)`
requests an exact evaluation when a drag ends.

## Scale Sharing

`PanScrollZoom` mirrors the target scale-domain sharing by default:

- `Sharing::Free` / `Level(0)` generates per-leaf raw-domain params.
- `Sharing::Level(N)` generates params scoped to the same logical ancestor.
- `Sharing::Shared` generates globally shared raw-domain params.

Dragging a faceted cell patches the scoped param owner for that cell, and normal
scale-domain sharing propagates the result to the appropriate siblings.
`FacetWrap` counts as one logical sharing level.

Explicit `x_sharing(...)` / `y_sharing(...)` overrides are allowed only when the
requested param sharing is at least as broad as every target scale sharing. A
tool expansion errors if it cannot find an invertible target channel, if target
sharing is inconsistent, or if the target scale already has a different
`raw_domain`.

## Toolbars And Enablement

Each tool can generate an enabled param. `PanScrollZoom` filters its event
bindings through that param, and `CompiledPlot::tool_metadata()` exposes the
tool id, label, and enabled-param name for future app UI.

Toolbar UI remains future work. The intended shape is app chrome outside the
chart scenegraph that patches enabled params and displays metadata from the
compiled plot.

## Future Tools

The same expansion substrate should support richer interactions:

- Brush selection: a keyed `Store` of editable region rows, a neutral
  `Selection` derived from that store, event bindings that mutate store rows,
  and ordinary overlay marks that read `StoreData`.
- Hover and tooltips: hover params drive tooltip marks or app UI.
- Annotation drawing: drag state patches annotation stores and overlay marks.
- Reset/domain controls: app or chart controls patch tool-owned params back to
  null defaults.

These tools need additional identity and editing contracts before they become
implementation-ready. The key remaining pieces are pointer-event policy for
generated/decorative marks, editable overlay identity for store-backed regions,
and ergonomic event bindings for moving or resizing those regions. Basic
hit-test-to-datum mapping exists today, including derived-mark datum lineage,
but richer tools still need clearer control over which rendered geometry is an
interactive target and which geometry is only chrome.

## Crate Boundary

The tool contracts live in `avenger-chart-core`. Built-in tools live in
`avenger-chart-tools`, which is a peer to other built-in implementation crates
such as `avenger-chart-scales` and `avenger-chart-legend`.

Custom coordinate crates do not need `avenger-chart-cartesian` to participate in
the generic event-binding substrate. A coordinate-aware navigation tool can
target named interaction channels when the coordinate transform exports
invertible interaction scopes and the target scales support `raw_domain`.
