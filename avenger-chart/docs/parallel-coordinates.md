# Parallel Coordinates

Parallel coordinates are wide-form charts: each source row becomes one
polyline, and each `Parallel::dimension(id, expr)` adds one vertical axis with
its own scale.

```rust
use avenger_chart::prelude::*;
use datafusion::prelude::{col, lit};

let coord = Parallel::new()
    .dimension_with("speed", col("speed"), |d| {
        d.axis(|axis| axis.title("Speed"))
    })
    .dimension_with("efficiency", col("efficiency"), |d| {
        d.axis(|axis| axis.title("Efficiency"))
    })
    .dimension_with("segment", col("segment"), |d| {
        d.axis(|axis| axis.title("Segment"))
    });

let plot = Plot::with_coord(coord)
    .data(df)
    .mark(
        ParallelLine::new()
            .details(["sample_id"])
            .stroke(col("segment"))
            .opacity(0.5),
    )
    .mark(
        ParallelSymbol::new()
            .fill(col("segment"))
            .size(24.0),
    );
```

## Dimensions

Dimension ids are stable structural ids. They are used as scale names, guide
event datum values, axis-overlay targets, and order-state values. Repeat
placeholders can appear in dimension expressions and axis expressions, but not
in dimension ids.

Numeric dimensions infer linear scales. String dimensions infer point scales.
Each dimension coordinates its own domain, so facets can share or free
dimension domains independently using the normal channel configuration on
`.dimension_with(...)`.

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
interactions.

## Examples

```bash
cargo run -p avenger-chart-app --example parallel_coordinates_basic --features winit-wgpu
cargo run -p avenger-chart-app --example parallel_coordinates_axis_overlay_brush --features winit-wgpu
cargo run -p avenger-chart-app --example parallel_coordinates_axis_overlay_summary --features winit-wgpu
cargo run -p avenger-chart-app --example parallel_coordinates_axis_overlay_violins --features winit-wgpu
cargo run -p avenger-chart-app --example parallel_coordinates_reorder_preview --features winit-wgpu
cargo run -p avenger-chart-app --example parallel_coordinates_header_drag_reorder --features winit-wgpu
cargo run -p avenger-chart-app --example parallel_axis_brush_selection --features winit-wgpu
cargo run -p avenger-chart-app --example parallel_coordinates_axis_brush_intersection --features winit-wgpu
```
