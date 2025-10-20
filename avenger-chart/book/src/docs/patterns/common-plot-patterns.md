# Common Plot Patterns

This chapter gathers the most common chart recipes in one place so you can copy, tweak, and combine them quickly. Each example is a complete Avenger Chart plot (boilerplate hidden with `#` so mdbook can render the result inline). For deeper coverage of mark-specific options, hop to the dedicated mark pages referenced in each section.

---

## Scatter patterns

All scatter plots use the [Symbol mark](../marks/symbol.md).

### Basic scatter

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    ?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(Symbol::new().x(col("sepal_length")).y(col("sepal_width")));

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Encode categories with color

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    ?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .size(180.0)
            .fill_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Species"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Bubble chart (size encoding)

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    ?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .fill_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Species"))
            })
            .size_with(col("petal_length"), |c| {
                c.scale(|s| s.range_interval(lit(80.0), lit(280.0)))
                    .legend(|l| l.title("Petal Length"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Axis titles and chart title

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    ?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .title("Iris Sepal Measurements")
    .mark(
        Symbol::new()
            .x_with(col("sepal_length"), |c| c.axis(|a| a.title("Sepal Length (cm)")))
            .y_with(col("sepal_width"), |c| c.axis(|a| a.title("Sepal Width (cm)")))
            .size(150.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

See the [Symbol mark reference](../marks/symbol.md) for additional encodings (shape, angle), polar coordinates, and automatic radius padding details.

---

## Line patterns

These examples use the [Line mark](../marks/line.md).

### Basic line

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    (
        "x",
        Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "y",
        Arc::new(Float64Array::from(vec![2.0, 5.0, 3.0, 8.0, 6.0, 9.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])?;

let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(Line::new().x(col("x")).y(col("y")));

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Style color, width, dashes

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    ("x", Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0])) as _),
    ("y", Arc::new(Float64Array::from(vec![2.0, 5.0, 3.0, 8.0, 6.0, 9.0])) as _),
])?;
let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Line::new()
            .x(col("x"))
            .y(col("y"))
            .stroke("#ff6b6b")
            .stroke_width(3.0)
            .stroke_dash("dashed")
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Multi-series lines with legend

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    (
        "x",
        Arc::new(Float64Array::from(vec![
            1.0, 2.0, 3.0, 4.0, 5.0,
            1.0, 2.0, 3.0, 4.0, 5.0,
            1.0, 2.0, 3.0, 4.0, 5.0,
        ])) as _,
    ),
    (
        "y",
        Arc::new(Float64Array::from(vec![
            2.0, 4.0, 3.0, 5.0, 6.0,
            1.0, 3.0, 2.5, 4.0, 3.5,
            3.0, 5.5, 4.5, 7.0, 8.0,
        ])) as _,
    ),
    (
        "series",
        Arc::new(StringArray::from(vec![
            "A", "A", "A", "A", "A",
            "B", "B", "B", "B", "B",
            "C", "C", "C", "C", "C",
        ])) as _,
    ),
])?;
let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Line::new()
            .x(col("x"))
            .y(col("y"))
            .stroke_with(col("series"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Series"))
            })
            .stroke_width(2.5)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Layer lines and points

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    ("x", Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0])) as _),
    ("y", Arc::new(Float64Array::from(vec![2.0, 5.0, 3.0, 8.0, 6.0])) as _),
])?;
let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .title("Line with markers")
    .mark(
        Line::new()
            .data(df.clone())
            .x(col("x"))
            .y(col("y"))
            .stroke("#2563eb")
            .stroke_width(2.5)
    )
    .mark(
        Symbol::new()
            .data(df)
            .x(col("x"))
            .y(col("y"))
            .fill("#1d4ed8")
            .size(180.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Consult the [Line mark reference](../marks/line.md) for information on interpolation, smoothing, access to the `defined` channel, and advanced styling.

---

## Bar patterns

Bar charts rely on the [Rect mark](../marks/rect.md) plus band scales for positioning. The helper dataset `datasets::categorical_bars` ships with the crate for quick examples.

### Vertical bars (default orientation)

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

### Horizontal bars

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

### Style fills and strokes

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

### Adjust bar spacing with band padding

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

### Color by category

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

The [Rect mark reference](../marks/rect.md) covers stacked/grouped bars, heatmaps, interval plots, and more advanced categorical layouts.

---

## Next steps

- Dive into mark-specific references: [Symbol](../marks/symbol.md), [Line](../marks/line.md), [Rect](../marks/rect.md).
- Review [Scales](../scales/index.md) to understand and customize the transformations used above.
- Explore [Layering](../marks/layering.md) for additional multi-mark compositions.
