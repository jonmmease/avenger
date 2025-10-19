# Layout & Sizing

Avenger Chart separates the overall canvas from the plot area. That separation lets you build responsive dashboards, align plots inside larger layouts, or reserve space for headings and legends.

## Canvas Size

`canvas_size(width, height)` fixes the rendered image dimensions. Use this when you know the output size (for example, exporting a 800 × 600 PNG).

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
let _plot = Plot::<Cartesian>::new()
    .canvas_size(800.0, 600.0)
    .mark(Symbol::new().x(col("x")).y(col("y")));
# }
```

### Responsive Canvases

If you only want to lock one dimension, use `canvas_constraint` with `CanvasConstraint::width(...)` or `CanvasConstraint::height(...)`. The other dimension expands to fit the plot.

```rust,no_run
# use avenger_chart::prelude::*;
# use avenger_chart::layout::CanvasConstraint;
# use datafusion::prelude::*;
let _plot = Plot::<Cartesian>::new()
    .canvas_constraint(CanvasConstraint::width(420.0))
    .mark(Symbol::new().x(col("x")).y(col("y")));
```

## Plot Area Size

`plot_size` and `plot_constraint` control the drawable area inside the canvas (the region that contains the marks, axes, and guide). This is especially handy when you want consistent chart shapes regardless of legend size.

```rust,no_run
# use avenger_chart::prelude::*;
# use avenger_chart::layout::{CanvasConstraint, PlotConstraint};
# use datafusion::prelude::*;

let _plot = Plot::<Cartesian>::new()
    .canvas_size(500.0, 400.0)
    .plot_size(220.0, 160.0) // fixed interior plot
    .mark(Symbol::new().x(col("x")).y(col("y")));

let _responsive = Plot::<Cartesian>::new()
    .canvas_constraint(CanvasConstraint::width(420.0))
    .plot_constraint(PlotConstraint::width(260.0))
    .mark(Symbol::new().x(col("x")).y(col("y")));
```

## Margins

Margins reserve space between the plot area and the canvas edge (for example, to keep axes from touching neighbouring charts). Use `Margins::uniform`, `Margins::symmetric`, or construct explicit values.

```rust,no_run
# use avenger_chart::prelude::*;
# use avenger_chart::layout::Margins;
# use datafusion::prelude::*;

let _plot = Plot::<Cartesian>::new()
    .canvas_size(480.0, 360.0)
    .margins(Margins::uniform(12.0))
    .mark(Symbol::new().x(col("x")).y(col("y")));
```

Margins always apply outside the plot area. When you combine a fixed plot with wider margins, the canvas grows to satisfy both constraints.

## Layer Ordering

By default marks render in the order they are added. When you need explicit control—overlay annotations, keep grids behind bars, etc.—use `.zindex()` on any mark.

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
let _plot = Plot::<Cartesian>::new()
    .mark(Rect::new().x(col("category")).y(col("value")).zindex(0))
    .mark(Line::new().x(col("category")).y(col("target")).zindex(1));
# }
```

Lower values draw first; higher values appear on top. Grids and guides respect their own layers, so specifying a `zindex` is the easiest way to keep overlays visible without reordering data.

## Putting It Together

```rust,no_run
# use avenger_chart::prelude::*;
# use avenger_chart::layout::{CanvasConstraint, Margins, PlotConstraint};
# use datafusion::prelude::*;

let _dashboard_tile = Plot::<Cartesian>::new()
    .canvas_constraint(CanvasConstraint::width(400.0))
    .plot_constraint(PlotConstraint::width(240.0))
    .margins(Margins::symmetric(12.0, 18.0))
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .fill_with(col("segment"), |c| c.legend(|l| l.title("Segment")))
            .zindex(1),
    )
    .mark(
        Line::new()
            .x(col("x"))
            .y(col("target"))
            .stroke("#1d4ed8")
            .stroke_width(2.0)
            .zindex(2),
    );
```

The layout API lets you guarantee consistent chart dimensions, reserve breathing room for guides, and layer annotations in a predictable order.
