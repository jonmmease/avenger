# Your First Plot

This guide walks you through creating a simple scatter plot with Avenger Chart.

## Complete Example

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
# let iris_path = format!(
#     "{}/../tests/data/iris.parquet",
#     env!("CARGO_MANIFEST_DIR")
# );
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
            .fill("steelblue")
            .size(80.0)
    )
    .title("Iris Sepal Measurements")
    .canvas_size(600.0, 400.0);

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Breaking It Down

### 1. Create a DataFrame

Avenger Chart uses Apache DataFusion for data handling:

```rust,no_run
# use datafusion::prelude::*;
# async fn example() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
let ctx = SessionContext::new();
let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
# Ok(())
# }
```

You can also create DataFrames from Arrow RecordBatches or in-memory data.

### 2. Choose a Coordinate System

```rust,no_run
# use avenger_chart::prelude::*;
let plot = Plot::<Cartesian>::new();
```

Avenger Chart supports multiple coordinate systems:
- `Cartesian` - Standard x/y coordinates
- `Polar` - Radius/angle coordinates

### 3. Add Data

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let plot = Plot::<Cartesian>::new().data(df);
# let _ = plot;
# Ok(())
# }
```

Pass your DataFrame to the plot. Multiple marks can share the same data or have their own.

### 4. Add a Mark

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(Symbol::new().x(col("x")).y(col("y")));
# let _ = plot;
# Ok(())
# }
```

Marks are visual elements. Here we use `Symbol` for a scatter plot, mapping columns to position channels.

### 5. Compile and Evaluate

Avenger-Chart uses a two-stage workflow: compile then evaluate.

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(Symbol::new().x(col("x")).y(col("y")));

// Compile: Parse expressions, infer scales, optimize (do once)
let compiled = plot.compile(&ctx).await?;

// Evaluate: Execute queries, compute scales, build scene graph (do many times)
let evaluated = compiled.evaluate(&ctx, None).await?;
# Ok(())
# }
```

**Why two stages?**
- **Compile once, render many**: When using [parameters](../advanced/parameters.md), compile once then evaluate with different parameter values
- **Performance**: Expensive optimizations happen once at compile time
- **Flexibility**: Same compiled plot can be rendered with different data or parameters

For a detailed explanation of what happens at each stage and when to use this pattern, see [The Compilation Pipeline](../concepts/compilation.md).

### 6. Render to PNG

To write a PNG file, use the WgpuRenderer backend:

```rust,no_run
# use avenger_chart::prelude::*;
# use avenger_chart::render::WgpuRenderer;
# use datafusion::prelude::*;
# type AnyError = Box<dyn std::error::Error + Send + Sync + 'static>;
# async fn example() -> Result<(), AnyError> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(Symbol::new().x(col("x")).y(col("y")));
let compiled = plot.compile(&ctx).await?;
let renderer = WgpuRenderer::new().with_scale(2.0);
renderer.write_png(&compiled, &ctx, None, "scatter.png").await?;
# Ok(())
# }
```

## Adding Visual Encoding

Enhance your plot with color and size encoding:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use palette::Srgba;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .fill_with(col("category"), |c| {
                c.scale_with::<Ordinal>(|s| s.range_colors(vec![
                    Srgba::new(0.121, 0.466, 0.705, 1.0),
                    Srgba::new(0.173, 0.627, 0.173, 1.0),
                    Srgba::new(0.882, 0.470, 0.0, 1.0),
                ]))
                .legend(|l| l.title("Category"))
            })
            .size_with(col("value"), |c| {
                c.scale(|s| s.range_interval(lit(50.0), lit(500.0)))
                    .legend(|l| l.title("Value"))
            })
    );
# let compiled = plot.compile(&ctx).await?;
# let _evaluated = compiled.evaluate(&ctx, None).await?;
# Ok(())
# }
```

This creates a scatter plot where:
- Points are positioned by `x` and `y` columns
- Color is determined by `category` using a categorical scale
- Size is determined by `value` using a linear scale
- Both encodings display legends

## Next Steps

- Learn about [Coordinate Systems](../concepts/coordinate-systems.md)
- Explore different [Marks](../concepts/marks.md)
- Understand [Channels](../concepts/channels.md) for visual encoding
