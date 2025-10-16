# Marks

Marks are the visual building blocks of a plot. Each mark type turns channel inputs into a particular geometric representation. Avenger Chart currently ships three Cartesian mark types: `Symbol`, `Line`, and `Rect`.

## Available Mark Types

### Symbol

`Symbol` marks render discrete points, making them useful for scatter plots and dot plots.

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create GDP vs life expectancy data
let batch = RecordBatch::try_from_iter(vec![
    (
        "gdp",
        Arc::new(Float64Array::from(vec![5000.0, 15000.0, 25000.0, 35000.0, 45000.0, 55000.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "life_expectancy",
        Arc::new(Float64Array::from(vec![55.0, 65.0, 72.0, 75.0, 78.0, 80.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
.expect("create batch");

let df = ctx.read_batch(batch).expect("read batch");


let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("gdp"))
            .y(col("life_expectancy"))
            .size(300.0)
            .fill("#4682b4")
            .shape("square")
    );

let compiled = plot.compile(&ctx).await.expect("compile");
let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate");
evaluated
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

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create temperature data over time
let batch = RecordBatch::try_from_iter(vec![
    (
        "day",
        Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "temperature",
        Arc::new(Float64Array::from(vec![15.0, 18.0, 16.0, 22.0, 20.0, 24.0, 23.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
.expect("create batch");

let df = ctx.read_batch(batch).expect("read batch");


let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Line::new()
            .x(col("day"))
            .y(col("temperature"))
            .stroke("#dc143c")
            .stroke_width(3.0)
    );

let compiled = plot.compile(&ctx).await.expect("compile");
let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate");
evaluated
```

Useful channels:
- `x`, `y` – sampled along the polyline path
- `stroke`, `stroke_width`, `stroke_dash`, `stroke_cap`, `stroke_join` – line styling
- `opacity` – transparency applied to the entire line
- `defined` – boolean expression that allows gaps in the line
- `order` – explicit ordering for multi-series stroke encodings

### Rect

`Rect` marks render axis-aligned rectangles and power bar charts, heatmaps, and interval plots.

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create horizontal bar chart data
let batch = RecordBatch::try_from_iter(vec![
    (
        "category",
        Arc::new(StringArray::from(vec!["A", "B", "C", "D", "E"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "value",
        Arc::new(Float64Array::from(vec![25.0, 40.0, 30.0, 55.0, 35.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
.expect("create batch");

let df = ctx.read_batch(batch).expect("read batch");


let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .y(col("category"))
            .y2_with(col("category"), |c| c.band(1.0))
            .x(lit(0.0))
            .x2(col("value"))
            .fill("#ffa500")
            .opacity(0.9)
            .corner_radius(3.0)
    );

let compiled = plot.compile(&ctx).await.expect("compile");
let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate");
evaluated
```

Rectangles expose four position channels (`x`, `x2`, `y`, `y2`) and support `fill`, `stroke`, `stroke_width`, `opacity`, and `corner_radius`.

## Layering Marks

Marks can be layered to combine encodings:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create time series data
let batch = RecordBatch::try_from_iter(vec![
    (
        "time",
        Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "value",
        Arc::new(Float64Array::from(vec![10.0, 15.0, 13.0, 18.0, 16.0, 22.0, 20.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
.expect("create batch");

let df = ctx.read_batch(batch).expect("read batch");


let plot = Plot::<Cartesian>::new()
    .data(df.clone())
    .mark(
        Line::new()
            .x(col("time"))
            .y(col("value"))
            .stroke("#1f2933")
            .stroke_width(2.0)
    )
    .mark(
        Symbol::new()
            .x(col("time"))
            .y(col("value"))
            .size(200.0)
            .fill("#38bdf8")
    );

let compiled = plot.compile(&ctx).await.expect("compile");
let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate");
evaluated
```

`Plot` merges scale and legend configuration across marks that use the same channel names, so both layers share the same axes and color legend.

## Mark-Specific Data

Each mark can source its own `DataFrame`. When a mark omits `.data(...)`, it inherits the plot-level data:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create scatter point data
let points_batch = RecordBatch::try_from_iter(vec![
    (
        "x",
        Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "y",
        Arc::new(Float64Array::from(vec![2.5, 4.0, 3.0, 5.5, 4.5, 6.0, 5.5, 7.5]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
.expect("create points batch");

let points = ctx.read_batch(points_batch).expect("read points");

// Create trend line data
let trend_batch = RecordBatch::try_from_iter(vec![
    (
        "x",
        Arc::new(Float64Array::from(vec![1.0, 8.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "y_pred",
        Arc::new(Float64Array::from(vec![2.0, 8.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
.expect("create trend batch");

let trend = ctx.read_batch(trend_batch).expect("read trend");

Plot::<Cartesian>::new()
    .mark(
        Symbol::new()
            .data(points)
            .x(col("x"))
            .y(col("y"))
            .size(200.0)
            .fill("#2563eb")
    )
    .mark(
        Line::new()
            .data(trend)
            .x(col("x"))
            .y(col("y_pred"))
            .stroke("#dc2626")
            .stroke_width(3.0)
    )
```

This example shows scatter points with a separate trend line, each using different data sources.

## Symbol Shapes

Symbol marks support various shapes. Here's a visualization showing different available shapes:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

// Create data for different shapes
let batch = RecordBatch::try_from_iter(vec![
    (
        "x",
        Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 1.0, 2.0, 3.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "y",
        Arc::new(Float64Array::from(vec![2.0, 2.0, 2.0, 1.0, 1.0, 1.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "shape",
        Arc::new(StringArray::from(vec!["circle", "square", "triangle-up", "star", "diamond", "cross"]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
.expect("create batch");

let df = ctx.read_batch(batch).expect("read batch");


let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .size(500.0)
            .fill("#9b59b6")
            .shape_with(col("shape"), |c| {
                c.legend(|l| l.title("Shape"))
            })
    );

let compiled = plot.compile(&ctx).await.expect("compile");
let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate");
evaluated
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
