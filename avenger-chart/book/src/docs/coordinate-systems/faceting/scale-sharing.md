# Scale Sharing

When creating faceted visualizations, one of the most important decisions is whether facets should share common scales or have independent scales. This choice fundamentally affects how viewers can compare data across facets.

## Overview

Scale sharing controls whether each facet cell uses:
- **Shared scales**: A single, unified scale domain computed from all data
- **Free scales**: Independent scale domains computed separately for each facet

This applies independently to each channel (x, y, color, size, etc.), enabling hybrid configurations like "shared Y, free X."

> **Note**: This page covers **data scale sharing** - how x/y/color domains are computed across facets. There's a separate concept called **facet variable scale sharing** that controls how the faceting categories themselves are shared in nested facets (e.g., whether all columns show the same rows). See [Customization](customization.md#facet-variable-scale-sharing) for details on that feature.

## Why Scale Sharing Matters

The choice between shared and free scales determines what comparisons are easy to make:

| Scale Mode | Best For | Trade-off |
|------------|----------|-----------|
| **Shared** | Direct value comparisons across facets | May hide local patterns when magnitudes differ greatly |
| **Free** | Revealing patterns within each facet | Cannot directly compare absolute values across facets |

## ScaleSharing::Shared

When scales are shared, all facets use the same scale domain, enabling direct visual comparisons of absolute values.

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await?;

let plot = Plot::<FacetRow>::new()
    .data(df)
    .mark(
        Facet::new().row(col("species")).subplot(
            Plot::<Cartesian>::new().mark(
                Symbol::new()
                    .x(col("sepal_length"))
                    .y_with(col("sepal_width"), |c| {
                        c.scale_with::<Linear>(|s| s)
                            .with_scale_sharing(ScaleSharing::Shared)
                            .axis(|a| a.title("Sepal Width"))
                    })
                    .size(28.0)
                    .fill("#4682b4"),
            ),
        ),
    )
    .canvas_size(600.0, 500.0);

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Notice how with shared Y scales, you can directly compare the heights of bars across facets. However, if the value ranges differ dramatically between facets, some cells may appear compressed.

## ScaleSharing::Free

When scales are free, each facet computes its own scale domain independently, maximizing the use of available space to reveal local patterns.

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await?;

let plot = Plot::<FacetRow>::new()
    .data(df)
    .mark(
        Facet::new().row(col("species")).subplot(
            Plot::<Cartesian>::new().mark(
                Symbol::new()
                    .x(col("sepal_length"))
                    .y_with(col("sepal_width"), |c| {
                        c.scale_with::<Linear>(|s| s)
                            .with_scale_sharing(ScaleSharing::Free)
                            .axis(|a| a.title("Sepal Width"))
                    })
                    .size(28.0)
                    .fill("#8a2be2"),
            ),
        ),
    )
    .canvas_size(600.0, 500.0);

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

With free Y scales, each facet's bars fill the available vertical space, making it easier to see relative patterns within each category. However, you cannot directly compare absolute values by visual height alone.

## Hybrid Scale Sharing

You can configure different scale sharing modes for different channels. A common pattern is to share one axis while keeping the other free.

### Shared Y, Free X

Useful when the Y-axis represents a comparable metric but X-axis categories differ per facet:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await?;

let plot = Plot::<FacetRow>::new()
    .data(df)
    .mark(
        Facet::new().row(col("species")).subplot(
            Plot::<Cartesian>::new().mark(
                Symbol::new()
                    .x_with(col("sepal_length"), |c| {
                        c.scale_with::<Linear>(|s| s)
                            .with_scale_sharing(ScaleSharing::Free)
                            .axis(|a| a.title("Sepal Length"))
                    })
                    .y_with(col("sepal_width"), |c| {
                        c.scale_with::<Linear>(|s| s)
                            .with_scale_sharing(ScaleSharing::Shared)
                            .axis(|a| a.title("Sepal Width"))
                    })
                    .size(28.0)
                    .fill("#2e8b57"),
            ),
        ),
    )
    .canvas_size(600.0, 500.0);

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

This configuration enables:
- Direct Y-value comparisons (sepal width) across species
- Independent X-scale optimization for each species' sepal length range

### Shared X, Free Y

Useful when the X-axis represents a common dimension (like time) but Y-values have different magnitudes:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await?;

let plot = Plot::<FacetColumn>::new()
    .data(df)
    .mark(
        Facet::new().column(col("species")).subplot(
            Plot::<Cartesian>::new().mark(
                Symbol::new()
                    .x_with(col("sepal_length"), |c| {
                        c.scale_with::<Linear>(|s| s)
                            .with_scale_sharing(ScaleSharing::Shared)
                            .axis(|a| a.title("Sepal Length"))
                    })
                    .y_with(col("sepal_width"), |c| {
                        c.scale_with::<Linear>(|s| s)
                            .with_scale_sharing(ScaleSharing::Free)
                            .axis(|a| a.title("Sepal Width"))
                    })
                    .size(28.0)
                    .fill("#d2691e"),
            ),
        ),
    )
    .canvas_size(600.0, 500.0);

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## ScaleSharing::Level(n) - Hierarchical Sharing

For nested facets, `ScaleSharing::Level(n)` provides fine-grained control over which nesting level shares scales:

| Level | Meaning | Equivalent To |
|-------|---------|---------------|
| `Level(0)` | Independent per cell | `ScaleSharing::Free` |
| `Level(1)` | Share with immediate parent facet | - |
| `Level(2)` | Share two levels up | - |
| `Level(u8::MAX)` | Share globally across all facets | `ScaleSharing::Shared` |

### Example: Two-Level Faceting

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let iris = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await?;

// Create a two-level categorical grouping
let data = iris
    .with_column(
        "size_group",
        when(col("sepal_length").lt(lit(5.5)), lit("Small"))
            .otherwise(lit("Large"))
            .unwrap(),
    )?;

// Outer facet: rows by species
// Inner facet: columns by size group
// Level(1) means share within each species (inner facets share)
let plot = Plot::<FacetRow>::new()
    .data(data)
    .mark(
        Facet::new().row(col("species")).subplot(
            Plot::<FacetColumn>::new().mark(
                Facet::new().column(col("size_group")).subplot(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x(col("sepal_length"))
                            .y_with(col("sepal_width"), |c| {
                                c.scale_with::<Linear>(|s| s)
                                    .with_scale_sharing(ScaleSharing::Level(1))
                                    .axis(|a| a.title("Sepal Width"))
                            })
                            .size(24.0)
                            .fill("#cd5c5c"),
                    ),
                ),
            ),
        ),
    )
    .canvas_size(600.0, 600.0);

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

In this example:
- `Level(1)` means the Y-scale is shared among all cells within the same outer facet (same species)
- Different species (outer facets) can have different Y-scale ranges
- This enables within-species comparisons while accommodating between-species differences

## When to Use Each Mode

### Use ScaleSharing::Shared When:

1. **Comparing absolute values** across facets is important
   - Example: Sales figures across regions where you need to identify which region has higher sales

2. **Value magnitudes are similar** across facets
   - Example: Test scores across different classrooms (all roughly 0-100 range)

3. **Detecting outliers** across the entire dataset
   - Example: Identifying which region has unusually high temperatures

4. **Creating a unified visual reference**
   - Example: Time series where all facets should align to the same temporal scale

### Use ScaleSharing::Free When:

1. **Revealing local patterns** within each facet is the priority
   - Example: Seasonal patterns that may differ in magnitude but follow similar cycles

2. **Value ranges differ dramatically** between facets
   - Example: Comparing website traffic for large vs. small sites (millions vs. hundreds)

3. **Maximizing data visibility** in each cell
   - Example: Comparing correlation patterns where exact values are less important than relationships

4. **Each facet represents a different measurement type**
   - Example: Dashboard showing temperature, humidity, and pressure in separate facets

### Use Hybrid Sharing When:

1. **One dimension is naturally comparable**, the other is not
   - Example: Time (X-axis, shared) vs. different metrics (Y-axis, free)

2. **Optimizing both global and local comparisons**
   - Example: Stock prices over time (X shared) for companies with vastly different valuations (Y free)

### Use ScaleSharing::Level(n) When:

1. **Working with nested facets** where intermediate levels of sharing are needed
   - Example: Comparing performance within teams (Level 1) but not across departments

2. **Building hierarchical dashboards** with region > country > city faceting
   - Share at the country level but allow cities to have independent scales

## Default Behavior

When scale sharing is not explicitly specified:
- **FacetRow**: Y scales are free by default (each row has its own Y-scale)
- **FacetColumn**: X scales are free by default (each column has its own X-scale)
- The non-faceted dimension typically shares by default to enable cross-facet comparison

To explicitly control this behavior, always use `.with_scale_sharing()` on your channels.

## API Reference

| Method | Description |
|--------|-------------|
| `.with_scale_sharing(ScaleSharing::Shared)` | Share scale across all facets |
| `.with_scale_sharing(ScaleSharing::Free)` | Independent scale per facet |
| `.with_scale_sharing(ScaleSharing::Level(n))` | Share at nesting level n |
| `.share_scale()` | Convenience method for `Shared` |
| `.free_scale()` | Convenience method for `Free` |

## Next Steps

- [Customization](customization.md) - Learn how to customize facet appearance, spacing, and layout
- [Nested Facets](nested-facets.md) - Explore multi-level faceting with hierarchical scale sharing
- [Getting Started](getting-started.md) - Review basic faceting concepts
