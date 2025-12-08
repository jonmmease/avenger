# Facet Customization

Facets can be customized using `FacetOptions`, which controls titles, spacing, and other appearance options. This guide shows how to customize faceted visualizations using the `row_with()` and `col_with()` configuration methods.

## Facet Titles

Add a title to your facets using `FacetOptions::title()`:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();

// Create sample data with three species
let batch = RecordBatch::try_from_iter(vec![
    (
        "species",
        Arc::new(StringArray::from(vec![
            "setosa", "setosa", "setosa",
            "versicolor", "versicolor", "versicolor",
            "virginica", "virginica", "virginica"
        ])) as datafusion::arrow::array::ArrayRef,
    ),
    (
        "sepal_length",
        Arc::new(Float64Array::from(vec![5.1, 4.9, 4.7, 7.0, 6.4, 6.9, 6.3, 5.8, 7.1]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "sepal_width",
        Arc::new(Float64Array::from(vec![3.5, 3.0, 3.2, 3.2, 3.2, 3.1, 3.3, 2.7, 3.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])?;

let df = ctx.read_batch(batch)?;

let plot = Plot::<FacetColumn>::new()
    .data(df)
    .mark(
        Facet::new()
            .col_with(col("species"), |c| c.facet(|f| f.title("Iris Species")))
            .subplot(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x(col("sepal_length"))
                        .y(col("sepal_width"))
                        .size(120.0)
                        .fill("#4682b4")
                )
            )
    )
    .canvas_size(800.0, 300.0);

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

The `col_with()` method takes a closure that receives a channel configuration object, on which you call `.facet()` to configure facet-specific options. The `.title()` method adds a descriptive label that appears above (for column facets) or to the side (for row facets) of the facet cells.

## Spacing Between Facets

Control the pixel gap between facet cells using `FacetOptions::spacing()`:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();

// Create sample data with three species
let batch = RecordBatch::try_from_iter(vec![
    (
        "species",
        Arc::new(StringArray::from(vec![
            "setosa", "setosa", "setosa",
            "versicolor", "versicolor", "versicolor",
            "virginica", "virginica", "virginica"
        ])) as datafusion::arrow::array::ArrayRef,
    ),
    (
        "sepal_length",
        Arc::new(Float64Array::from(vec![5.1, 4.9, 4.7, 7.0, 6.4, 6.9, 6.3, 5.8, 7.1]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "sepal_width",
        Arc::new(Float64Array::from(vec![3.5, 3.0, 3.2, 3.2, 3.2, 3.1, 3.3, 2.7, 3.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])?;

let df = ctx.read_batch(batch)?;

let plot = Plot::<FacetColumn>::new()
    .data(df)
    .mark(
        Facet::new()
            .col_with(col("species"), |c| c.facet(|f| f.spacing(40.0)))
            .subplot(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x(col("sepal_length"))
                        .y(col("sepal_width"))
                        .size(120.0)
                        .fill("#4682b4")
                )
            )
    )
    .canvas_size(800.0, 300.0);

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

The default spacing is typically 10 pixels. Increasing spacing creates more visual separation between facets, which can improve readability when facets contain dense information.

## Axis Positioning

You can position axes on different sides of the plot area. This is especially useful when you want to place the x-axis at the top or the y-axis on the right.

### X-Axis on Top

Move the x-axis to the top using `.axis(|a| a.position("top"))`:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();

// Create sample data with three species
let batch = RecordBatch::try_from_iter(vec![
    (
        "species",
        Arc::new(StringArray::from(vec![
            "setosa", "setosa", "setosa",
            "versicolor", "versicolor", "versicolor",
            "virginica", "virginica", "virginica"
        ])) as datafusion::arrow::array::ArrayRef,
    ),
    (
        "sepal_length",
        Arc::new(Float64Array::from(vec![5.1, 4.9, 4.7, 7.0, 6.4, 6.9, 6.3, 5.8, 7.1]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "sepal_width",
        Arc::new(Float64Array::from(vec![3.5, 3.0, 3.2, 3.2, 3.2, 3.1, 3.3, 2.7, 3.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])?;

let df = ctx.read_batch(batch)?;

let plot = Plot::<FacetRow>::new()
    .data(df)
    .mark(
        Facet::<Cartesian>::new()
            .row_with(col("species"), |c| c.facet(|f| f.title("Species")))
            .subplot(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x_with(col("sepal_length"), |c| {
                            c.scale_with::<Linear>(|s| s)
                                .axis(|a| a.title("Sepal Length").position("top"))
                        })
                        .y_with(col("sepal_width"), |c| {
                            c.scale_with::<Linear>(|s| s)
                                .axis(|a| a.title("Sepal Width"))
                        })
                        .size(90.0)
                        .fill("#1f78b4")
                )
            )
    )
    .canvas_size(600.0, 500.0);

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

When the x-axis is positioned at the top, the axis labels and title appear above the plot area. This is useful for presentations where you want to emphasize the axis at the top.

### Y-Axis on Right

Move the y-axis to the right using `.axis(|a| a.position("right"))`:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();

// Create sample data with three species
let batch = RecordBatch::try_from_iter(vec![
    (
        "species",
        Arc::new(StringArray::from(vec![
            "setosa", "setosa", "setosa",
            "versicolor", "versicolor", "versicolor",
            "virginica", "virginica", "virginica"
        ])) as datafusion::arrow::array::ArrayRef,
    ),
    (
        "sepal_length",
        Arc::new(Float64Array::from(vec![5.1, 4.9, 4.7, 7.0, 6.4, 6.9, 6.3, 5.8, 7.1]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "sepal_width",
        Arc::new(Float64Array::from(vec![3.5, 3.0, 3.2, 3.2, 3.2, 3.1, 3.3, 2.7, 3.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])?;

let df = ctx.read_batch(batch)?;

let plot = Plot::<FacetRow>::new()
    .data(df)
    .mark(
        Facet::<Cartesian>::new()
            .row_with(col("species"), |c| c.facet(|f| f.title("Species")))
            .subplot(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x_with(col("sepal_length"), |c| {
                            c.scale_with::<Linear>(|s| s)
                                .axis(|a| a.title("Sepal Length"))
                        })
                        .y_with(col("sepal_width"), |c| {
                            c.scale_with::<Linear>(|s| s)
                                .axis(|a| a.title("Sepal Width").position("right"))
                        })
                        .size(90.0)
                        .fill("#2e8b57")
                )
            )
    )
    .canvas_size(600.0, 500.0);

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

When the y-axis is positioned on the right, the facet labels automatically move to the left side to avoid overlap. This maintains clear visual organization.

## Using `row_with` and `col_with`

The `row_with()` and `col_with()` methods provide a builder pattern for configuring facet channels. They take two arguments:

1. The column expression (e.g., `col("species")`)
2. A closure that configures the channel

```rust
// Configure row faceting
Facet::new().row_with(col("species"), |c| {
    c.facet(|f| f.title("Species").spacing(20.0))
})

// Configure column faceting
Facet::new().col_with(col("region"), |c| {
    c.facet(|f| f.title("Region").spacing(15.0))
})
```

The closure receives a channel configuration object `c`, which you can use to:
- Call `.facet()` to configure facet-specific options (title, spacing, scale sharing)
- Set data scale sharing modes (covered in [Scale Sharing](scale-sharing.md))

## Facet Variable Scale Sharing

There are **two levels** of scale sharing in the facet system:

1. **Data Scale Sharing**: Controls whether x/y/color domains are computed across all facets or per-facet (covered in [Scale Sharing](scale-sharing.md))
2. **Facet Variable Scale Sharing**: Controls whether the facet arrangement itself is shared or free across nested facets

Facet variable scale sharing is configured via `FacetOptions` and determines how the domain of the faceting variable (the row or column categories) is computed:

### Shared Facet Domain

When facet variable scale sharing is **Shared**, the domain is computed from the full dataset. This creates a grid-like structure:

- All outer cells show the same inner categories (even if some cells are empty)
- Facet labels appear only at the outer edges (not repeated)
- Enables direct row-to-row or column-to-column comparison

### Free Facet Domain (Default)

When facet variable scale sharing is **Free**, each outer cell computes its own domain from its filtered data:

- Different outer cells may have different numbers of inner cells
- Facet labels repeat in each outer cell (since arrangement varies)
- More compact when categories don't overlap across outer facets

### Comparison: Free vs Shared Facet Domains

The following examples show the same data with different facet variable scale sharing settings. We use `petal_width_bin` (narrow/medium/wide) as the outer column facet and `species` as the inner row facet.

This example is chosen because setosa ONLY appears in "narrow" (petal_width ≤ 0.8), while versicolor and virginica appear in "medium" and "wide". This makes the difference between free and shared clearly visible.

#### Without `.share_scale()` (Free - Default)

With free facet scaling, each column computes its own domain from its filtered data. Since "medium" and "wide" have no setosa data, they don't show the setosa row:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let iris = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await?;

// Create petal_width_bin column
// setosa: petal_width 0.1-0.6, all in "narrow"
// versicolor: petal_width 1.0-1.8, in "medium" and "wide"
// virginica: petal_width 1.4-2.5, in "medium" and "wide"
let df = iris
    .with_column(
        "petal_width_bin",
        when(col("petal_width").lt_eq(lit(0.8)), lit("narrow"))
            .when(col("petal_width").lt_eq(lit(1.7)), lit("medium"))
            .otherwise(lit("wide"))
            .unwrap()
    )
    .unwrap();

// FREE facet domain (default) - each column shows only species present in that bin
let plot = Plot::<FacetColumn>::new()
    .data(df)
    .canvas_size(700, 450)
    .mark(
        Facet::new()
            .col_with(col("petal_width_bin"), |c| c.facet(|f| f.title("Petal Width")))
            .subplot(
                Plot::<FacetRow>::new().mark(
                    Facet::new()
                        // No share_scale() - uses FREE (default)
                        .row_with(col("species"), |c| c.facet(|f| f.title("Species")))
                        .subplot(
                            Plot::<Cartesian>::new().mark(
                                Symbol::new()
                                    .x(col("sepal_length"))
                                    .y(col("sepal_width"))
                                    .size(25.0)
                                    .fill("#4682b4")
                            )
                        )
                )
            )
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Notice that:
- The "narrow" column shows only "setosa" (1 row)
- The "medium" and "wide" columns show only "versicolor" and "virginica" (2 rows each)
- Each column has a different number of rows based on what species are present
- Species labels repeat in each column (since arrangement varies)

#### With `.share_scale()` (Shared - Grid-Like)

With shared facet scaling, all columns show all species from the full dataset, creating a consistent 3x3 grid:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let iris = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await?;

// Same petal_width_bin column
let df = iris
    .with_column(
        "petal_width_bin",
        when(col("petal_width").lt_eq(lit(0.8)), lit("narrow"))
            .when(col("petal_width").lt_eq(lit(1.7)), lit("medium"))
            .otherwise(lit("wide"))
            .unwrap()
    )
    .unwrap();

// SHARED facet domain - all columns show all species (grid-like)
let plot = Plot::<FacetColumn>::new()
    .data(df)
    .canvas_size(700, 450)
    .mark(
        Facet::new()
            .col_with(col("petal_width_bin"), |c| c.facet(|f| f.title("Petal Width")))
            .subplot(
                Plot::<FacetRow>::new().mark(
                    Facet::new()
                        // KEY: share_scale() creates grid-like structure
                        .row_with(col("species"), |c| c.facet(|f| f.title("Species").share_scale()))
                        .subplot(
                            Plot::<Cartesian>::new().mark(
                                Symbol::new()
                                    .x(col("sepal_length"))
                                    .y(col("sepal_width"))
                                    .size(25.0)
                                    .fill("#4682b4")
                            )
                        )
                )
            )
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Notice that:
- All three columns now show all three species rows (3x3 grid)
- The "medium" and "wide" columns have empty cells for setosa (no data)
- The "narrow" column has empty cells for versicolor and virginica
- Species labels appear only on the right edge (not repeated)
- You can directly compare the same row position across columns

### When to Use Facet Variable Scale Sharing

| Use Case | Recommendation |
|----------|----------------|
| Grid-like layout with consistent structure | Use `.share_scale()` |
| Comparing across outer facets (e.g., same row position = same category) | Use `.share_scale()` |
| Variable inner categories per outer facet | Use `.free_scale()` (default) |
| Compact layout when categories don't overlap | Use `.free_scale()` (default) |

**Note**: Facet variable scale sharing is primarily useful for nested facets. For single-level faceting, all cells always show the same categories based on the faceting column.

## Complete Configuration Example

Here's a comprehensive example combining multiple customization options:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();

// Create sample data with three species
let batch = RecordBatch::try_from_iter(vec![
    (
        "species",
        Arc::new(StringArray::from(vec![
            "setosa", "setosa", "setosa", "setosa",
            "versicolor", "versicolor", "versicolor", "versicolor",
            "virginica", "virginica", "virginica", "virginica"
        ])) as datafusion::arrow::array::ArrayRef,
    ),
    (
        "sepal_length",
        Arc::new(Float64Array::from(vec![
            5.1, 4.9, 4.7, 5.4,
            7.0, 6.4, 6.9, 6.8,
            6.3, 5.8, 7.1, 6.5
        ])) as datafusion::arrow::array::ArrayRef,
    ),
    (
        "sepal_width",
        Arc::new(Float64Array::from(vec![
            3.5, 3.0, 3.2, 3.9,
            3.2, 3.2, 3.1, 2.8,
            3.3, 2.7, 3.0, 3.2
        ])) as datafusion::arrow::array::ArrayRef,
    ),
])?;

let df = ctx.read_batch(batch)?;

let plot = Plot::<FacetColumn>::new()
    .data(df)
    .mark(
        Facet::new()
            .col_with(col("species"), |c| {
                c.facet(|f| {
                    f.title("Iris Species")
                        .spacing(30.0)
                })
            })
            .subplot(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x_with(col("sepal_length"), |c| {
                            c.scale_with::<Linear>(|s| s)
                                .axis(|a| a.title("Sepal Length (cm)").position("top"))
                        })
                        .y_with(col("sepal_width"), |c| {
                            c.scale_with::<Linear>(|s| s)
                                .axis(|a| a.title("Sepal Width (cm)"))
                                .with_scale_sharing(ScaleSharing::Shared)
                        })
                        .size(120.0)
                        .fill("#9b59b6")
                )
            )
    )
    .canvas_size(900.0, 350.0);

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

This example demonstrates:
- **Facet title**: "Iris Species" appears above the facets
- **Custom spacing**: 30 pixels between each facet column
- **X-axis on top**: Sepal Length axis positioned at the top
- **Shared y-scale**: All facets use the same y-axis range for easy comparison
- **Axis titles**: Descriptive titles with units

## API Reference

The `FacetOptions` type provides the following configuration methods:

```rust
impl FacetOptions {
    /// Set a title for the facet dimension
    pub fn title(self, title: impl Into<String>) -> Self

    /// Set the spacing (in pixels) between facet cells
    pub fn spacing(self, spacing: f32) -> Self

    /// Configure scale sharing for the facet variable domain
    /// Controls whether nested facets share the same categories
    pub fn with_scale_sharing(self, mode: ScaleSharing) -> Self

    /// Convenience method: Set scale sharing to Shared
    /// All nested cells will show the same categories (grid-like)
    pub fn share_scale(self) -> Self

    /// Convenience method: Set scale sharing to Free (default)
    /// Each outer cell computes its own category domain
    pub fn free_scale(self) -> Self
}
```

## Limitations

The following features are **not yet supported** in the facet system:

- **Interactive coordination**: Pan and zoom interactions do not currently synchronize across facet cells. Each facet is independently rendered without shared interaction state.

If you need synchronized pan/zoom across facets, this is a planned feature for future development. Currently, facets are best used for static comparisons across categorical groupings.

## Next Steps

- Learn about [Scale Sharing](scale-sharing.md) to control how data scales are computed across facets
- Explore [Nested Facets](nested-facets.md) for multi-level faceting with both rows and columns
- Return to the [Faceting Overview](index.md) for a broader perspective
