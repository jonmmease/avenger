# Marks

Marks are the visual building blocks of a plot. Each mark type turns channel inputs into a particular geometric representation. Avenger Chart currently ships three Cartesian mark types: `Symbol`, `Line`, and `Rect`.

## Available Mark Types

### Symbol

`Symbol` marks render discrete points, making them useful for scatter plots and dot plots.

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
let _plot = Plot::<Cartesian>::new().mark(
    Symbol::new()
        .x(col("gdp"))
        .y(col("life_expectancy"))
        .size(120.0)
        .fill("#4682b4")
        .shape("square"),
);
# }
```

Key channels:
- `x`, `y` – positional encodings inherited from the enclosing `Plot`
- `size` – area of the marker in square pixels
- `fill`, `stroke`, `stroke_width` – color and outline styling
- `shape` – accepts Vega symbol names such as `"circle"`, `"square"`, `"triangle-up"`, `"star"`, `"wye"`, and more
- `angle` – rotation in radians (useful with non-circular shapes)

When a `Symbol` is constructed inside `Plot::mark`, the coordinate system is inferred from the plot. Use `Symbol::<Cartesian>::new()` for standalone construction.

### Line

`Line` marks draw ordered polylines.

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
let _plot = Plot::<Cartesian>::new().mark(
    Line::new()
        .x(col("date"))
        .y(col("temperature"))
        .stroke("#dc143c")
        .stroke_width(2.0)
        .stroke_dash("dashed"),
);
# }
```

Useful channels:
- `x`, `y` – sampled along the polyline path
- `stroke`, `stroke_width`, `stroke_dash`, `stroke_cap`, `stroke_join` – line styling
- `opacity` – transparency applied to the entire line
- `defined` – boolean expression that allows gaps in the line
- `order` – explicit ordering for multi-series stroke encodings

### Rect

`Rect` marks render axis-aligned rectangles and power bar charts, heatmaps, and interval plots.

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
let _plot = Plot::<Cartesian>::new().mark(
    Rect::new()
        .y(col("category"))
        .x(lit(0.0))
        .x2(col("value"))
        .fill("#ffa500")
        .opacity(0.9)
        .corner_radius(3.0),
);
# }
```

Rectangles expose four position channels (`x`, `x2`, `y`, `y2`) and support `fill`, `stroke`, `stroke_width`, `opacity`, and `corner_radius`.

## Layering Marks

Marks can be layered to combine encodings.

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let _plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Line::new()
            .x(col("date"))
            .y(col("temperature"))
            .stroke("#1f2933")
            .stroke_width(2.0),
    )
    .mark(
        Symbol::new()
            .x(col("date"))
            .y(col("temperature"))
            .size(96.0)
            .fill("#38bdf8"),
    );
# Ok(())
# }
```

`Plot` merges scale and legend configuration across marks that use the same channel names, so both layers share the same axes and color legend.

## Mark-Specific Data

Each mark can source its own `DataFrame`. When a mark omits `.data(...)`, it inherits the plot-level data.

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let points = ctx.read_csv("points.csv", CsvReadOptions::new()).await?;
# let trend = ctx.read_csv("trend.csv", CsvReadOptions::new()).await?;
let _plot = Plot::<Cartesian>::new()
    .mark(
        Symbol::new()
            .data(points)
            .x(col("x"))
            .y(col("y"))
            .fill("#2563eb"),
    )
    .mark(
        Line::new()
            .data(trend)
            .x(col("x"))
            .y(col("y_pred"))
            .stroke("#dc2626"),
    );
# Ok(())
# }
```

## Additional Mark Options

All marks share a common builder API:
- `.data(...)` attaches a dedicated `DataFrame`.
- `.facet_strategy(...)` and `.broadcast_to_facets()` control how marks participate in faceting.
- `.details([...])` provides additional fields for tooltips and interactions.
- `.zindex(...)` sets explicit draw order when layers overlap.

## Planned Marks

Text annotations, area charts, path-based marks, and rule markers are documented in [future-work/text-mark.md](../../docs/future-work/text-mark.md) and related roadmap notes. They are not part of the current release.

## Next Steps

- Explore [Channels](./channels.md) to see how marks receive data.
- Learn how [Scales](./scales.md) transform channel expressions.
- Review [Legends](./legends.md) for automatically generated guides.
