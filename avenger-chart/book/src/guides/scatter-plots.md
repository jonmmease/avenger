# Scatter Plots

Scatter plots visualize the relationship between two quantitative variables. This guide covers basic scatter plots and various enhancements using the Iris dataset.

## Basic Scatter Plot

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
    )
```

This creates a simple scatter plot with circles at default size showing the relationship between sepal length and width.

## Customizing Appearance

### Symbol Size

Set a fixed size for all points:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .size(200.0)  // Size in square pixels
    )
```

### Symbol Shape

Choose different shapes:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .size(150.0)
            .shape("square")
    )
```

Available shape names include: `"circle"`, `"square"`, `"cross"`, `"diamond"`, `"triangle-up"`, `"triangle-down"`, `"triangle-left"`, `"triangle-right"`, `"arrow"`, `"wedge"`, `"triangle"`, `"star"`, `"wye"`, `"pentagon"`, and `"cushion"`.

### Colors

Set fill and stroke:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .size(120.0)
            .fill("#ff6b6b")      // Coral red
            .stroke("#2c3e50")    // Dark slate
            .stroke_width(2.5)
    )
```

## Encoding with Color

Map the species categorical variable to color:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

Plot::<Cartesian>::new()
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
    )
```

This creates a scatter plot where each species has a different color.

## Encoding with Size

Map petal length to size:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .size_with(col("petal_length"), |c| {
                c.scale(|s| s.range_interval(lit(80.0), lit(300.0)))
                    .legend(|l| l.title("Petal Length"))
            })
    )
```

This creates a bubble chart where larger circles represent longer petals.

## Multiple Encodings

Combine color and size encoding:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

Plot::<Cartesian>::new()
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
    )
```

## Transparency for Overplotting

The current API does not expose an opacity channel for symbols yet. A common pattern is to encode density through fill colors using alpha values:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .size(250.0)
            .fill("#4682b480")  // hex RGBA with transparency
    )
```

## Symbol Padding and "Nice" Scales

Position scales automatically expand so the full symbol geometry stays inside the plot. Disabling niceness keeps the domain tight to your data; enabling niceness rounds the domain to "nice" numbers and may introduce extra breathing room.

With nice scales enabled (rounded tick values):

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

Plot::<Cartesian>::new()
    .data(df)
    .title("Nice Scales (Rounded Ticks)")
    .mark(
        Symbol::new()
            .x_with(col("sepal_length"), |c| c.scale_with::<Linear>(|s| s.nice(true)))
            .y_with(col("sepal_width"), |c| c.scale_with::<Linear>(|s| s.nice(true)))
            .size(140.0)
    )
```

With nice scales disabled (tight to data):

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

Plot::<Cartesian>::new()
    .data(df)
    .title("Precise Scales (Tight to Data)")
    .mark(
        Symbol::new()
            .x_with(col("sepal_length"), |c| c.scale_with::<Linear>(|s| s.nice(false)))
            .y_with(col("sepal_width"), |c| c.scale_with::<Linear>(|s| s.nice(false)))
            .size(140.0)
    )
```

Opt for `nice(false)` when you need tight framing (for example, aligning icons to the edge of a tile) and keep niceness enabled when rounded tick values improve readability.

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

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

Plot::<Cartesian>::new()
    .data(df)
    .title("Iris Sepal Measurements")
    .mark(
        Symbol::new()
            .x_with(col("sepal_length"), |c| c.axis(|a| a.title("Sepal Length (cm)")))
            .y_with(col("sepal_width"), |c| c.axis(|a| a.title("Sepal Width (cm)")))
            .size(150.0)
    )
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
