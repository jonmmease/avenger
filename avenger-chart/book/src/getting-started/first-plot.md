# Your First Plot

We’ll build a scatter plot in three passes so you can see how marks, encodings, and automatic scales interact.

For the examples we’ll use the classic iris dataset through Apache DataFusion:

```rust,no_run
# use datafusion::prelude::*;
# async fn example() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
let ctx = SessionContext::new();
let df = ctx.read_parquet("iris.parquet", ParquetReadOptions::default()).await?;
# let _ = (ctx, df);
# Ok(())
# }
```

Every full code sample below follows this shape (with the dataset path hidden using `#` so `mdbook` can render the charts inline).

## Step 1 — Place a mark

We start with a `Symbol` mark whose position is a literal. Because every row is rendered at the same `x/y`, the symbols stack on top of each other. This makes it easy to see the effect of adding encodings next.

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
            .x(lit(0.0)) // literal values: everything lands at the same spot
            .y(lit(0.0))
            .fill("#4682b4")
            .size(80.0)
    )
    .title("All points overlap without encodings");

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Step 2 — Map data to position channels

Next we replace the literals with DataFusion expressions. Mapping `sepal_length` and `sepal_width` to `x`/`y` spreads the points out, and Avenger automatically infers linear position scales plus matching axes.

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
            .fill("#4682b4")
            .size(80.0)
    )
    .title("Position channels mapped from data");

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Step 3 — Add additional encodings (color, size, legends)

Finally we encode species as fill color and petal length as size. We still haven’t touched axes or legends—the compiler picks the right scales (`Ordinal` for species, `Linear` for numeric size), provides a categorical palette, and renders matching guides automatically.

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
            .fill_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Species"))
            })
            .size_with(col("petal_length"), |c| {
                c.scale(|s| s.range_interval(lit(40.0), lit(300.0)))
                    .legend(|l| l.title("Petal length"))
            })
    )
    .title("Iris Sepal Measurements by Species")
    .canvas_size(640.0, 420.0);

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Compile, evaluate, render

Behind every example above is the same workflow:

- `plot.compile(&ctx)` analyzes expressions, infers scales/legends, and stores a serializable plan.
- `compiled.evaluate(&ctx, params)` runs the DataFusion queries, applies parameters, evaluates theme defaults, and returns an `EvaluatedPlot`.
- Render with `WgpuRenderer::render`/`write_png` (or treat the `EvaluatedPlot` scene graph however you like).

For deeper coverage of the pipeline, see [The Compilation Pipeline](../docs/compilation.md) and [Rendering and Output](../docs/rendering.md).

## Where to go next

- Learn about [Coordinate Systems](../docs/coordinate-systems/index.md).
- Explore the full [Marks](../docs/marks/index.md) catalog.
- Dive into [Channels](../docs/channels/index.md) for mapping vs setting guidance.
