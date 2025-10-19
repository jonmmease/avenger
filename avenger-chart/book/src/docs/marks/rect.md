# Rect Mark

`Rect` marks render axis-aligned rectangles and power bar charts, heatmaps, and interval plots.

**Coordinate System Support:** Currently Cartesian only. Polar support (arc marks, wedge marks, radial bar charts) is planned for future releases.

## Basic Example

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
?;

let df = ctx.read_batch(batch)?;


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

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Channels

### Position Channels (Cartesian)

Rectangles are defined by four position channels that specify opposite corners:

- **`x`**, **`x2`** – Horizontal extent (left and right edges)
- **`y`**, **`y2`** – Vertical extent (bottom and top edges)

### Visual Encoding Channels

- **`fill`** – Fill color (supports hex colors, CSS colors, expressions)
- **`stroke`** – Outline color
- **`stroke_width`** – Outline width in pixels
- **`opacity`** – Transparency (0.0 to 1.0)
- **`corner_radius`** – Radius for rounded corners in pixels

## Bar Charts

Rect marks are the foundation of bar charts, using categorical and quantitative scales.

### Vertical Bars

```rust,render
use avenger_chart::prelude::*;

let ctx = SessionContext::new();
let plot = Plot::<Cartesian>::new()
    .data(datasets::categorical_bars(&ctx))
    .mark(
        Rect::new()
            .x(col("category"))
            .x2_with(col(":x"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("value"))
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

This creates vertical bars from 0 to the value in each category. The `x2` channel uses the `:x` [channel reference](../channels.md#channel-references) with `.band(1.0)` to span the full width of each categorical band.

### Horizontal Bars

Swap x and y coordinates for horizontal orientation:

```rust,render
use avenger_chart::prelude::*;

let ctx = SessionContext::new();
let plot = Plot::<Cartesian>::new()
    .data(datasets::categorical_bars(&ctx))
    .mark(
        Rect::new()
            .x(lit(0.0))
            .x2(col("value"))
            .y(col("category"))
            .y2_with(col(":y"), |c| c.band(1.0))
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Styling Rectangles

### Fill Color

```rust,render
use avenger_chart::prelude::*;

let ctx = SessionContext::new();
let plot = Plot::<Cartesian>::new()
    .data(datasets::categorical_bars(&ctx))
    .mark(
        Rect::new()
            .x(col("category"))
            .x2_with(col(":x"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("value"))
            .fill("#e74c3c")
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Stroke and Borders

```rust,render
use avenger_chart::prelude::*;

let ctx = SessionContext::new();
let plot = Plot::<Cartesian>::new()
    .data(datasets::categorical_bars(&ctx))
    .mark(
        Rect::new()
            .x(col("category"))
            .x2_with(col(":x"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("value"))
            .fill("#3498db")
            .stroke("#2c3e50")
            .stroke_width(2.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Categorical Scales and Spacing

Use band scales for categorical axes with padding between bars:

```rust,render
use avenger_chart::prelude::*;

let ctx = SessionContext::new();
let plot = Plot::<Cartesian>::new()
    .data(datasets::categorical_bars(&ctx))
    .mark(
        Rect::new()
            .x_with(col("category"), |c| {
                c.scale_with::<Band>(|s| s.padding_inner(0.3))
            })
            .x2_with(col(":x"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("value"))
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Controlling Bar Width

By default, bars span the full width of their category band (from `.band(0.0)` to `.band(1.0)`). You can create narrower bars by adjusting the band positions.

### Narrow Bars (Centered)

Create bars that are 70% of the full band width, centered:

```rust,render
use avenger_chart::prelude::*;

let ctx = SessionContext::new();
let plot = Plot::<Cartesian>::new()
    .data(datasets::categorical_bars(&ctx))
    .mark(
        Rect::new()
            .x_with(col("category"), |c| c.band(0.15))   // Start at 15% of band
            .x2_with(col(":x"), |c| c.band(0.85))         // End at 85% of band
            .y(lit(0.0))
            .y2(col("value"))
            .fill("#9b59b6")
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

The bar starts at 15% and ends at 85% of the band width, creating a centered bar that's 70% wide.

### Understanding Band Positions

The `.band()` method positions a mark within a categorical band:

```
Band for "Category A":
├───────────────────────────────────┤
↑           ↑           ↑           ↑
0.0        0.3         0.5         1.0
start                 middle       end
```

**Common patterns**:
- **Full width**: `.band(0.0)` to `.band(1.0)` (default, spans entire band)
- **70% centered**: `.band(0.15)` to `.band(0.85)`
- **50% centered**: `.band(0.25)` to `.band(0.75)`
- **Left-aligned narrow**: `.band(0.0)` to `.band(0.7)`

See [Band Scale](../scales/band.md#the-band-method) for more details on the `.band()` method.

## Color Encoding

Encode categories with color using ordinal scales:

```rust,render
use avenger_chart::prelude::*;

let ctx = SessionContext::new();
let plot = Plot::<Cartesian>::new()
    .data(datasets::categorical_bars(&ctx))
    .mark(
        Rect::new()
            .x(col("category"))
            .x2_with(col(":x"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("value"))
            .fill_with(col("category"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Category"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Advanced Examples

### Aggregated Bar Charts

Pre-aggregate in DataFusion before rendering:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::functions_aggregate::expr_fn::sum;
use datafusion::prelude::*;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create sample sales data with multiple entries per category
let batch = RecordBatch::try_from_iter(vec![
    (
        "category",
        Arc::new(StringArray::from(vec!["A", "A", "B", "B", "C", "C"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "amount",
        Arc::new(Float64Array::from(vec![10.0, 15.0, 25.0, 30.0, 20.0, 23.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;

// Aggregate by category
let aggregated = df
    .aggregate(vec![col("category")], vec![sum(col("amount")).alias("total")])
    ?;

let plot = Plot::<Cartesian>::new()
    .data(aggregated)
    .mark(
        Rect::new()
            .x(col("category"))
            .x2_with(col(":x"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("total"))
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Sorted Bar Charts

Sort categories by value using DataFusion:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
let sorted = datasets::categorical_bars(&ctx)
    .sort(vec![col("value").sort(false, false)])
    ?;

let plot = Plot::<Cartesian>::new()
    .data(sorted)
    .mark(
        Rect::new()
            .x(col("category"))
            .x2_with(col(":x"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("value"))
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Diverging Bar Chart

Show positive and negative values with color encoding:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create data with positive and negative changes
let batch = RecordBatch::try_from_iter(vec![
    (
        "category",
        Arc::new(StringArray::from(vec!["A", "B", "C", "D", "E"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "change",
        Arc::new(Float64Array::from(vec![15.0, -8.0, 22.0, -12.0, 18.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;

// Create status based on change sign
let status = when(col("change").gt(lit(0.0)), lit("positive"))
    .otherwise(lit("negative"))
    ?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x(col("category"))
            .x2_with(col(":x"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("change"))  // Can be positive or negative
            .fill_with(status, |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Change"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Heatmaps

For heatmaps, use categorical scales on both axes and encode a quantitative value to fill color:

```rust,ignore
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
Rect::new()
    .x(col("x_category"))
    .x2_with(col("x_category"), |c| c.band(1.0))
    .y(col("y_category"))
    .y2_with(col("y_category"), |c| c.band(1.0))
    .fill_with(col("value"), |c| {
        c.scale_with::<Linear>(|s| s)
    })
```

## Interval Plots

For interval plots showing ranges, use both position channels with quantitative data:

```rust,ignore
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
Rect::new()
    .x(col("start_time"))
    .x2(col("end_time"))
    .y(col("task"))
    .y2_with(col("task"), |c| c.band(0.8))  // 80% of band width
    .fill("#2ca02c")
```

## Complete Example: Movies Dataset

```rust,render
use avenger_chart::prelude::*;
use datafusion::functions_aggregate::expr_fn::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
# let movies_path = format!("{}/../tests/data/movies.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(movies_path, ParquetReadOptions::default())
    .await
    ?;

let aggregated = df
    .aggregate(
        vec![col("MPAA Rating")],
        vec![sum(col("Worldwide Gross")).alias("total_gross")],
    )
    ?;

let filtered = aggregated
    .filter(col("MPAA Rating").is_not_null())
    ?;
let sorted = filtered
    .sort(vec![col("total_gross").sort(false, false)])
    ?;

let plot = Plot::<Cartesian>::new()
    .data(sorted)
    .mark(
        Rect::new()
            .x_with(col("MPAA Rating"), |c| {
                c.scale_with::<Band>(|s| s.padding_inner(0.2))
            })
            .x2_with(col(":x"), |c| c.band(1.0))
            .y_with(lit(0.0), |c| c.axis(|a| a.format(".2s")))
            .y2(col("total_gross"))
            .fill_with(col("MPAA Rating"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("MPAA Rating"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Next Steps

- See the [Marks Overview](./index.md) for layering and composition
- Try the [Bar Charts Tutorial](../chart-types/bar-charts.md) for a quick-start guide
- Learn about [Band Scales](../scales/band.md) for categorical positioning
