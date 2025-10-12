# Bar Charts

Bar charts compare quantities across categories using rectangular bars. This guide covers vertical and horizontal bar charts, grouped and stacked variations.

## Basic Bar Chart

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let _plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x(col("category"))
            .y(lit(0.0))
            .y2(col("value"))
    );
# Ok(())
# }
```

This creates vertical bars from 0 to the value in each category.

## Horizontal Bars

Swap x and y:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
let _rect = Rect::<Cartesian>::new()
    .y(col("category"))
    .x(lit(0.0))
    .x2(col("value"));
```

## Styling Bars

### Bar Color

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
let _rect = Rect::<Cartesian>::new()
    .x(col("category"))
    .y(lit(0.0))
    .y2(col("value"))
    .fill("#4682b4");
```

### Bar Borders

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
let _rect = Rect::<Cartesian>::new()
    .x(col("category"))
    .y(lit(0.0))
    .y2(col("value"))
    .fill("lightblue")
    .stroke("#4682b4")
    .stroke_width(1.0);
```

## Categorical Scales

Use band scales for categorical x-axes:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
let _rect = Rect::<Cartesian>::new()
    .x_with(col("category"), |c| c.scale_with::<Band>(|s| s.padding_inner(0.1)))
    .y(lit(0.0))
    .y2(col("value"));
```

## Color by Category

Encode categories with color:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use palette::Srgba;
let _rect = Rect::<Cartesian>::new()
    .x(col("category"))
    .y(lit(0.0))
    .y2(col("value"))
    .fill_with(col("category"), |c| {
        c.scale(|s| s.range_colors(vec![
            Srgba::new(0.121, 0.466, 0.705, 1.0),
            Srgba::new(0.173, 0.627, 0.173, 1.0),
            Srgba::new(0.882, 0.470, 0.0, 1.0),
        ]))
        .legend(|l| l.title("Category"))
    });
```

## Aggregated Bar Charts

Pre-aggregate in DataFusion before rendering:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::functions_aggregate::expr_fn::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let aggregated = df
    .aggregate(
        vec![col("category")],
        vec![sum(col("amount")).alias("total")]
    )?;

let _rect = Rect::<Cartesian>::new()
    .data(aggregated)
    .x(col("category"))
    .y(lit(0.0))
    .y2(col("total"));
# Ok(())
# }
```

## Grouped Bar Charts

Grouped/dodged layouts are part of the planned adjust system (see `docs/future-work/adjust-api.md`).

## Stacked Bar Charts

Stacked bars will arrive with the transform system (see `docs/future-work/transform-system.md`).

## Sorted Bar Charts

Sort categories by value:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let sorted = df.sort(vec![col("value").sort(false, false)])?;  // Descending

let _plot = Plot::<Cartesian>::new()
    .data(sorted)
    .mark(
        Rect::new()
            .x(col("category"))
            .y(lit(0.0))
            .y2(col("value"))
    )
# ;
# Ok(())
# }
```

## Bar Chart with Labels

Text annotations will ship with the planned text mark (see `docs/future-work/text-mark.md`).


## Diverging Bar Chart

Show positive and negative values:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use palette::Srgba;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let status = when(col("change").gt(lit(0.0)), lit("positive"))
    .otherwise(lit("negative"))?;

let _rect = Rect::<Cartesian>::new()
    .x(col("category"))
    .y(lit(0.0))
    .y2(col("change"))  // Can be positive or negative
    .fill_with(status, |c| {
        c.scale(|s| s
            .domain_discrete(vec![lit("positive"), lit("negative")])
            .range_colors(vec![
                Srgba::new(0.200, 0.627, 0.173, 1.0),
                Srgba::new(0.800, 0.200, 0.200, 1.0),
            ])
        )
        .legend(|l| l.title("Change"))
    });
# Ok(())
# }
```

## Bar Chart with Reference Line

Reference overlays (e.g., rules) are planned but not yet implemented.


## Complete Example

```rust,no_run
use avenger_chart::prelude::*;
use datafusion::functions_aggregate::expr_fn::*;
use datafusion::prelude::*;
use palette::Srgba;

let ctx = SessionContext::new();
let movies_path = format!(
    "{}/../tests/data/movies.parquet",
    env!("CARGO_MANIFEST_DIR")
);
let df = ctx
    .read_parquet(movies_path, ParquetReadOptions::default())
    .await
    .expect("load movies dataset");

let aggregated = df
    .aggregate(
        vec![col("MPAA_Rating")],
        vec![sum(col("Worldwide_Gross")).alias("total_gross")],
    )
    .expect("aggregate worldwide gross by rating");

let filtered = aggregated
    .filter(col("MPAA_Rating").is_not_null())
    .expect("filter rated films");
let sorted = filtered
    .sort(vec![col("total_gross").sort(false, false)])
    .expect("sort by total gross");

let plot = Plot::<Cartesian>::new()
    .data(sorted)
    .mark(
        Rect::new()
            .x_with(col("MPAA_Rating"), |c| c.scale_with::<Band>(|s| s.padding_inner(0.2)))
            .y(lit(0.0))
            .y2(col("total_gross"))
            .fill_with(col("MPAA_Rating"), |c| {
                c.scale(|s| s.range_colors(vec![
                    Srgba::new(0.204, 0.596, 0.859, 1.0),
                    Srgba::new(0.984, 0.604, 0.600, 1.0),
                    Srgba::new(0.169, 0.506, 0.337, 1.0),
                ]))
                .legend(|l| l.title("MPAA Rating"))
            })
    );

plot
```

## Next Steps

- Learn about [Aggregations](./aggregations.md) for data summaries
- Explore [Line Charts](./line-charts.md) for trends
