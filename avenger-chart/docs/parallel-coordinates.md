# Parallel Coordinates

Parallel coordinates are wide-form charts: each source row becomes one
polyline, and each mark-owned `.dimension(id, expr)` binds data to a vertical
axis with its own scale.

```rust
use avenger_chart::prelude::*;
use datafusion::prelude::{col, lit};

let coord = Parallel::new()
    .dimension_with("speed", |d| d.axis(|axis| axis.title("Speed")))
    .dimension_with("efficiency", |d| d.axis(|axis| axis.title("Efficiency")))
    .dimension_with("segment", |d| d.axis(|axis| axis.title("Segment")));

let chart = Chart::with_coord(coord)
    .data(df)
    .mark(
        ParallelLine::new()
            .dimension("speed", col("speed"))
            .dimension("efficiency", col("efficiency"))
            .dimension("segment", col("segment"))
            .details(["sample_id"])
            .stroke(col("segment"))
            .opacity(0.5),
    )
    .mark(
        ParallelSymbol::new()
            .dimension("speed", col("speed"))
            .dimension("efficiency", col("efficiency"))
            .dimension("segment", col("segment"))
            .fill(col("segment"))
            .size(24.0),
    );
```

## Dimensions

Dimension ids are stable structural ids. They are used as scale names, guide
event datum values, axis-overlay targets, and order-state values.
`ParallelLine::dimension(id, expr)`, `ParallelSymbol::dimension(id, expr)`, and
future parallel marks own the expressions that feed each dimension scale.
`Parallel::dimension_with(id, |d| ...)` configures a discovered dimension's
frame/axis behavior by id.

Repeat placeholders can appear in mark-owned dimension expressions and axis
expressions, but not in dimension ids.

Numeric dimensions infer linear scales. String dimensions infer point scales.
Each dimension coordinates its own domain, so facets can share or free
dimension domains independently using the normal channel configuration on
mark-owned `.dimension_with(id, expr, |d| ...)`.

## Axis Overlays

`ParallelAxisOverlay::new(dimension_id, child_plot)` places an ordinary
Cartesian child plot over one axis. The child plot receives:

- a y scale matching the selected parallel dimension;
- a local x scale over the overlay width;
- inherited parent data by default, unless the child plot has explicit data.

This supports brush rectangles, summary marks, and compound marks such as
`BoxPlot` and `Violin` directly on top of an axis.

```rust
let overlay = ParallelAxisOverlay::new(
    "speed",
    Plot::<Cartesian>::new().mark(
        Rect::new()
            .x_with(lit(0.0), |x| {
                x.scale_with::<Linear>(|scale| scale.domain((0.0, 1.0)))
                    .axis(|axis| axis.visible(false))
            })
            .x2(lit(1.0))
            .y(lit(40.0))
            .y2(lit(60.0))
            .fill("rgba(37, 99, 235, 0.16)"),
    ),
)
.width_px(22.0);
```

## Ordering And Interaction

`Parallel::order([...])` sets a static dimension order.
`Parallel::order_param(name)` reads committed order from a string-list
parameter. `Parallel::active_axis_display_params(dimension_param, x_param)`
reads a transient dragged dimension id and display x position. Lines, symbols,
axis overlays, axes, and guide titles all render from the same display frame.

Parallel guide title surfaces retain event datum fields for low-level
interactions:

- `ev::parallel_dimension_id()`
- `ev::parallel_display_x()`
- `ev::parallel_equilibrium_x()`
- `ev::parallel_order_index()`
- `ev::parallel_displacement_px()`
- `ev::parallel_displacement_slots()`

The app examples show how to wire these fields into manual reorder and brush
interactions. Static rendering cases are covered by the parallel visual
baselines under `avenger-chart/tests/baselines/parallel/`.

## Interactive Examples

```bash
cargo run --release -p avenger-chart-app --example parallel_coordinates_header_drag_reorder --features winit-wgpu
cargo run --release -p avenger-chart-app --example parallel_axis_brush_selection --features winit-wgpu
```
