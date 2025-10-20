# Scatter Plots

Scatter plots visualize the relationship between two quantitative variables using the [Symbol mark](../marks/symbol.md). This guide provides a quick tutorial on creating scatter plots with the Iris dataset.

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

## Encoding with Color

Map categorical variables to color to distinguish groups:

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

## Bubble Charts

Create bubble charts by encoding a third variable as symbol size:

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

## Learn More

For comprehensive coverage of Symbol mark capabilities, see the [Symbol Mark Reference](../marks/symbol.md), which includes:

- **[Size](../marks/symbol.md#size)**: Fixed and encoded sizing for bubble charts
- **[Fill and Stroke](../marks/symbol.md#fill-and-stroke)**: Color customization and encoding (includes transparency)
- **[Multiple Encodings](../marks/symbol.md#multiple-encodings)**: Combining color, size, and shape
- **[Automatic Visual Padding](../marks/symbol.md#automatic-visual-padding)**: How Avenger prevents symbol clipping
- **[Angle](../marks/symbol.md#angle)**: Rotating symbols
- **[Available Shapes](../marks/symbol.md#available-shapes)**: Circle, square, diamond, triangle, star, and more
- **[Using with Different Scales](../marks/symbol.md#using-with-different-scales)**: Logarithmic and other scale types
- **[Coordinate Systems](../marks/symbol.md#polar-coordinates)**: Cartesian and Polar positioning

## Next Steps

- Learn about [Line Charts](./line-charts.md)
- Explore [Aggregations](../channels/aggregations.md) for summaries
- Review [Scales](../scales/index.md) for data transformation options
