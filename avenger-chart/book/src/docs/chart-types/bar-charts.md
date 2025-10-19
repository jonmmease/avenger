# Bar Charts

Bar charts compare quantities across categories using rectangular bars created with the [Rect mark](../marks/rect.md). This guide provides a quick tutorial on creating bar charts.

## Basic Bar Chart

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

## Horizontal Bars

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

## Styling Bars

Customize bar appearance with fill color and stroke:

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

## Categorical Scales

Use band scales to control spacing between bars:

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

## Color by Category

Encode categories with different colors:

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

## Learn More

For comprehensive coverage of Rect mark capabilities for bar charts and beyond, see the [Rect Mark Reference](../marks/rect.md), which includes:

- **[Bar Charts](../marks/rect.md#bar-charts)**: Vertical and horizontal examples
- **[Styling Rectangles](../marks/rect.md#styling-rectangles)**: Fill color and stroke options
- **[Categorical Scales and Spacing](../marks/rect.md#categorical-scales-and-spacing)**: Band scale padding
- **[Controlling Bar Width](../marks/rect.md#controlling-bar-width)**: Band positioning with detailed diagram
- **[Color Encoding](../marks/rect.md#color-encoding)**: Ordinal scale examples
- **[Advanced Examples](../marks/rect.md#advanced-examples)**: Aggregation, sorting, and diverging charts
- **[Heatmaps](../marks/rect.md#heatmaps)**: Two-dimensional categorical encodings
- **[Interval Plots](../marks/rect.md#interval-plots)**: Range visualizations
- **[Complete Example](../marks/rect.md#complete-example-movies-dataset)**: Movies dataset with aggregation

## Next Steps

- Learn about [Aggregations](../data/aggregations.md) for data summaries
- Explore [Line Charts](./line-charts.md) for trends
- Review [Band Scales](../scales/band.md) for categorical positioning
