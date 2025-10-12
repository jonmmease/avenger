# Bar Charts

Bar charts compare quantities across categories using rectangular bars. This guide covers vertical and horizontal bar charts, grouped and stacked variations.

## Basic Bar Chart

```rust,render,ignore
use avenger_chart::prelude::*;

Plot::<Cartesian>::new()
    .data(datasets::categorical_bars(&ctx))
    .mark(
        Rect::new()
            .x(col("category"))
            .y(lit(0.0))
            .y2(col("value"))
    )
```

This creates vertical bars from 0 to the value in each category.

## Horizontal Bars

Swap x and y:

```rust,render,ignore
use avenger_chart::prelude::*;

Plot::<Cartesian>::new()
    .data(datasets::categorical_bars(&ctx))
    .mark(
        Rect::new()
            .y(col("category"))
            .x(lit(0.0))
            .x2(col("value"))
    )
```

## Styling Bars

### Bar Color

```rust,render,ignore
use avenger_chart::prelude::*;

Plot::<Cartesian>::new()
    .data(datasets::categorical_bars(&ctx))
    .mark(
        Rect::new()
            .x(col("category"))
            .y(lit(0.0))
            .y2(col("value"))
            .fill("#4682b4")
    )
```

### Bar Borders

```rust,render,ignore
use avenger_chart::prelude::*;

Plot::<Cartesian>::new()
    .data(datasets::categorical_bars(&ctx))
    .mark(
        Rect::new()
            .x(col("category"))
            .y(lit(0.0))
            .y2(col("value"))
            .fill("lightblue")
            .stroke("#4682b4")
            .stroke_width(1.0)
    )
```

## Categorical Scales

Use band scales for categorical x-axes with padding between bars:

```rust,render,ignore
use avenger_chart::prelude::*;

Plot::<Cartesian>::new()
    .data(datasets::categorical_bars(&ctx))
    .mark(
        Rect::new()
            .x_with(col("category"), |c| {
                c.scale_with::<Band>(|s| s.padding_inner(0.3))
            })
            .y(lit(0.0))
            .y2(col("value"))
    )
```

## Color by Category

Encode categories with color:

```rust,render,ignore
use avenger_chart::prelude::*;

Plot::<Cartesian>::new()
    .data(datasets::categorical_bars(&ctx))
    .mark(
        Rect::new()
            .x(col("category"))
            .y(lit(0.0))
            .y2(col("value"))
            .fill_with(col("category"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Category"))
            })
    )
```

## Aggregated Bar Charts

Pre-aggregate in DataFusion before rendering:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::functions_aggregate::expr_fn::sum;
use datafusion::prelude::*;
use std::sync::Arc;

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
.expect("create batch");

let df = ctx.read_batch(batch).expect("read batch");

// Aggregate by category
let aggregated = df
    .aggregate(vec![col("category")], vec![sum(col("amount")).alias("total")])
    .expect("aggregate");

Plot::<Cartesian>::new()
    .data(aggregated)
    .mark(
        Rect::new()
            .x(col("category"))
            .y(lit(0.0))
            .y2(col("total"))
    )
```

## Grouped Bar Charts

Grouped/dodged layouts are part of the planned adjust system (see `docs/future-work/adjust-api.md`).

## Stacked Bar Charts

Stacked bars will arrive with the transform system (see `docs/future-work/transform-system.md`).

## Sorted Bar Charts

Sort categories by value:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let sorted = datasets::categorical_bars(&ctx)
    .sort(vec![col("value").sort(false, false)])
    .expect("sort");

Plot::<Cartesian>::new()
    .data(sorted)
    .mark(
        Rect::new()
            .x(col("category"))
            .y(lit(0.0))
            .y2(col("value"))
    )
```

## Bar Chart with Labels

Text annotations will ship with the planned text mark (see `docs/future-work/text-mark.md`).


## Diverging Bar Chart

Show positive and negative values:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

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
.expect("create batch");

let df = ctx.read_batch(batch).expect("read batch");

// Create status based on change sign
let status = when(col("change").gt(lit(0.0)), lit("positive"))
    .otherwise(lit("negative"))
    .expect("create status");

Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x(col("category"))
            .y(lit(0.0))
            .y2(col("change"))  // Can be positive or negative
            .fill_with(status, |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Change"))
            })
    )
```

## Bar Chart with Reference Line

Reference overlays (e.g., rules) are planned but not yet implemented.


## Complete Example

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::functions_aggregate::expr_fn::*;
use datafusion::prelude::*;

# let movies_path = format!("{}/../tests/data/movies.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(movies_path, ParquetReadOptions::default())
    .await
    .expect("load movies dataset");

let aggregated = df
    .aggregate(
        vec![col("MPAA Rating")],
        vec![sum(col("Worldwide Gross")).alias("total_gross")],
    )
    .expect("aggregate worldwide gross by rating");

let filtered = aggregated
    .filter(col("MPAA Rating").is_not_null())
    .expect("filter rated films");
let sorted = filtered
    .sort(vec![col("total_gross").sort(false, false)])
    .expect("sort by total gross");

Plot::<Cartesian>::new()
    .data(sorted)
    .mark(
        Rect::new()
            .x_with(col("MPAA Rating"), |c| {
                c.scale_with::<Band>(|s| s.padding_inner(0.2))
            })
            .y(lit(0.0))
            .y2(col("total_gross"))
            .fill_with(col("MPAA Rating"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("MPAA Rating"))
            })
    )
```

## Next Steps

- Learn about [Aggregations](./aggregations.md) for data summaries
- Explore [Line Charts](./line-charts.md) for trends
