# Scatter Charts

Scatter charts visualize the relationship between two continuous variables using point marks. They're excellent for identifying correlations, patterns, clusters, and outliers in your data. All scatter plots in Avenger Chart use the [Symbol mark](../marks/symbol.md).

## Basic Scatter Plot

The simplest scatter plot shows the relationship between two variables:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();

let df = ctx
    .read_parquet(avenger_sample_data::iris_path(), ParquetReadOptions::default())
    .await
    ?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(Symbol::new().x(col("sepal_length")).y(col("sepal_width")));

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Categorical Encoding with Color

Use color to distinguish different categories in your data:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();

let df = ctx
    .read_parquet(avenger_sample_data::iris_path(), ParquetReadOptions::default())
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

Encode a third continuous variable using point size to create bubble charts:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();

let df = ctx
    .read_parquet(avenger_sample_data::iris_path(), ParquetReadOptions::default())
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

## Adding Titles and Labels

Enhance readability with descriptive titles and axis labels:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();

let df = ctx
    .read_parquet(avenger_sample_data::iris_path(), ParquetReadOptions::default())
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

## Using Different Shapes

Distinguish categories using different symbol shapes:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();

let df = ctx
    .read_parquet(avenger_sample_data::iris_path(), ParquetReadOptions::default())
    .await
    ?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .size(200.0)
            .shape_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Species"))
            })
            .fill("#4682b4")
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Customizing Point Appearance

Fine-tune the visual appearance of your scatter plot:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();

let df = ctx
    .read_parquet(avenger_sample_data::iris_path(), ParquetReadOptions::default())
    .await
    ?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .size(120.0)
            .fill("#ff6b6b99")  // Using alpha channel for transparency
            .stroke("#c92a2a")
            .stroke_width(2.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Advanced Techniques

### Logarithmic Scales

For data with wide ranges, use logarithmic scales:

```rust,no_run
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x_with(col("gdp"), |c| c.scale_with::<Log>(|s| s))
            .y(col("life_expectancy"))
            .size(150.0)
    );
# Ok(())
# }
```

### Interactive Features

Scatter plots support pan and zoom for data exploration. See [Interactions](../interactions/pan-zoom.md) for implementation details.

## Best Practices

1. **Choose appropriate point sizes**: Too small and patterns are hard to see; too large and points overlap
2. **Use transparency for overlapping points**: Use alpha channel in colors (e.g., `#ff6b6b99`) when points overlap
3. **Limit categories**: Use shape encoding for 3-5 categories maximum
4. **Consider sampling**: For very large datasets, consider sampling or aggregation
5. **Add gridlines**: Help readers estimate values with `.axis(|a| a.grid(true))`

## Common Pitfalls

- **Overplotting**: When many points overlap, consider using opacity, smaller sizes, or 2D density plots
- **Too many encodings**: Using color, size, and shape simultaneously can overwhelm viewers
- **Misleading scales**: Starting axes at non-zero values can exaggerate differences

## Next Steps

- Explore the [Symbol mark reference](../marks/symbol.md) for all available options
- Learn about [Scale configuration](../scales/index.md) for customizing data mapping
- See [Polar scatter plots](../coordinate-systems/polar.md) for radial layouts