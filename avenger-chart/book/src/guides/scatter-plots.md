# Scatter Plots

Scatter plots visualize the relationship between two quantitative variables. This guide covers basic scatter plots and various enhancements using the Iris dataset.

## Basic Scatter Plot

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
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

This creates a simple scatter plot with circles at default size showing the relationship between sepal length and width.

## Customizing Appearance

### Symbol Size

Set a fixed size for all points:

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
            .size(200.0)  // Size in square pixels
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Symbol Shape

Choose different shapes:

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
            .size(150.0)
            .shape("square")
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Available shape names include: `"circle"`, `"square"`, `"cross"`, `"diamond"`, `"triangle-up"`, `"triangle-down"`, `"triangle-left"`, `"triangle-right"`, `"arrow"`, `"wedge"`, `"triangle"`, `"star"`, `"wye"`, `"pentagon"`, and `"cushion"`.

### Colors

Set fill and stroke:

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
            .size(120.0)
            .fill("#ff6b6b")      // Coral red
            .stroke("#2c3e50")    // Dark slate
            .stroke_width(2.5)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Encoding with Color

Map the species categorical variable to color:

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

This creates a scatter plot where each species has a different color.

## Encoding with Size

Map petal length to size:

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
            .size_with(col("petal_length"), |c| {
                c.scale(|s| s.range_interval(lit(80.0), lit(300.0)))
                    .legend(|l| l.title("Petal Length"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

This creates a bubble chart where larger circles represent longer petals.

## Multiple Encodings

Combine color and size encoding:

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

## Transparency for Overplotting

The current API does not expose an opacity channel for symbols yet. A common pattern is to encode density through fill colors using alpha values:

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
            .size(250.0)
            .fill("#4682b480")  // hex RGBA with transparency
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Automatic Visual Padding

Avenger Chart automatically expands position scale domains to prevent symbol clipping at data boundaries. This intelligent padding considers symbol size and stroke width to ensure marks are fully visible.

### How Automatic Padding Works

When you create a scatter plot with symbols at the edges of your data range, the chart automatically:

1. **Analyzes mark dimensions**: Calculates the visual extent of each symbol (radius + stroke width)
2. **Computes required padding**: Determines how much to expand the domain in data space
3. **Expands the domain**: Adjusts the scale domain so symbols don't get clipped

**Current limitation**: Automatic padding currently works only for **Linear scales**. Other scale types (Log, Pow, Time, etc.) will be supported in future releases.

### Example: Padding Prevents Clipping

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();

// Data points exactly at domain boundaries
let batch = RecordBatch::try_from_iter(vec![
    ("x", Arc::new(Float64Array::from(vec![0.0, 50.0, 100.0])) as _),
    ("y", Arc::new(Float64Array::from(vec![0.0, 50.0, 100.0])) as _),
])?;
let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .title("Automatic Padding (Symbols Fully Visible)")
    .mark(
        Symbol::new()
            .x_with(col("x"), |c| {
                c.scale_with::<Linear>(|s| s.domain((0.0, 100.0)).nice(false))
            })
            .y_with(col("y"), |c| {
                c.scale_with::<Linear>(|s| s.domain((0.0, 100.0)).nice(false))
            })
            .size(300.0)  // Large symbols to show padding effect
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Notice how symbols at (0, 0) and (100, 100) are fully visible despite being at the domain boundaries. The scale domain was automatically expanded to accommodate their visual size.

### Interaction with "Nice" Scales

The `nice()` setting affects how domain expansion works:

**With nice scales** (`nice(true)`, the default):
- Domain is rounded to "nice" numbers (e.g., 0, 25, 50, 75, 100)
- Provides clean tick values
- May add extra breathing room beyond what's needed for padding

**With precise scales** (`nice(false)`):
- Domain expands exactly enough to fit mark geometry
- Tick values may not be round numbers
- Minimal whitespace around data

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
    .title("Nice Scales (Rounded Ticks)")
    .mark(
        Symbol::new()
            .x_with(col("sepal_length"), |c| c.scale_with::<Linear>(|s| s.nice(true)))
            .y_with(col("sepal_width"), |c| c.scale_with::<Linear>(|s| s.nice(true)))
            .size(140.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Compare with precise domain:

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
    .title("Precise Scales (Tight to Data)")
    .mark(
        Symbol::new()
            .x_with(col("sepal_length"), |c| c.scale_with::<Linear>(|s| s.nice(false)))
            .y_with(col("sepal_width"), |c| c.scale_with::<Linear>(|s| s.nice(false)))
            .size(140.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### When to Use Each Approach

**Use `nice(true)` (default)** when:
- ✅ Readable tick labels are important
- ✅ Standard data visualization context
- ✅ You want consistent, predictable axis values

**Use `nice(false)` when**:
- ✅ You need precise framing (e.g., aligning icons to tile edges)
- ✅ Every pixel of space matters
- ✅ Domain boundaries are already meaningful values

### Technical Details

The padding calculation uses a geometric solver that:
- Converts mark sizes from pixels to data space units
- Handles asymmetric padding (e.g., triangles pointing different directions)
- Accounts for both symbol size and stroke width
- Works with rotated symbols (via the `angle` channel)

**Scale type support**:
- ✅ **Linear scales**: Full automatic padding support
- ⏳ **Other scales** (Log, Pow, Sqrt, Time, etc.): Planned for future release

See the [Roadmap](../roadmap.md) for upcoming enhancements to padding support.

## Logarithmic Scales

Use log scales for skewed data spanning multiple orders of magnitude:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let batch = RecordBatch::try_from_iter(vec![
    (
        "x",
        Arc::new(Float64Array::from(vec![1.0, 10.0, 100.0, 1000.0, 10000.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "y",
        Arc::new(Float64Array::from(vec![2.0, 5.0, 8.0, 12.0, 15.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
.expect("create batch");

let df = ctx.read_batch(batch).expect("read batch");

Plot::<Cartesian>::new()
    .data(df)
    .title("Log Scale Example")
    .mark(
        Symbol::new()
            .x_with(col("x"), |c| {
                c.scale_with::<Log>(|s| s.base(10.0))
                    .axis(|a| a.title("X (log scale)"))
            })
            .y_with(col("y"), |c| c.axis(|a| a.title("Y (linear)")))
            .size(150.0)
    )
```

## Adding Axis Labels

Add descriptive titles to axes and the overall plot:

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

## Faceted Scatter Plots

**Future feature**: Create small multiples using faceting:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("iris.csv", CsvReadOptions::new()).await?;
// This is planned for future release
// Planned API
# /*
# Plot::<Cartesian>::new()
#     .data(df)
#     .facet_wrap("species", 3)  // 3 columns
#     .mark(
#         Symbol::new()
#             .x(col("sepal_length"))
#             .y(col("sepal_width"))
#     );
# */
# let _ = df;
# Ok(())
# }
```

See [faceting.md](../../docs/future-work/faceting.md) for the design.

## Complete Example

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
let iris_path = format!(
    "{}/../tests/data/iris.parquet",
    env!("CARGO_MANIFEST_DIR")
);
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

let plot = Plot::<Cartesian>::new()
    .data(df.clone())
    .title("Iris Dataset")
    .mark(
        Symbol::new()
            .x_with(col("sepal_length"), |c| c.axis(|a| a.title("Sepal Length (cm)")))
            .y_with(col("sepal_width"), |c| c.axis(|a| a.title("Sepal Width (cm)")))
            .fill_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Species"))
            })
            .size(120.0)
    );

plot
```

## Next Steps

- Learn about [Line Charts](./line-charts.md)
- Explore [Aggregations](./aggregations.md) for summaries
