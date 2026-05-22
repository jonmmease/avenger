# Getting Started with Faceting

Faceting creates **small multiples** - a collection of charts showing the same relationship for different subsets of your data. This technique, pioneered by Edward Tufte, is ideal for comparing patterns across categorical groups while maintaining consistent visual encoding.

## What You'll Learn

This guide will teach you how to:
1. Prepare your data for faceting
2. Create your first row facet
3. Create your first column facet
4. Add titles and labels to facets

## Overview

Instead of showing all data points in a single chart with different colors or shapes, faceting creates separate panels for each category. This approach:

- Reduces visual clutter in dense datasets
- Makes comparisons clearer by separating groups spatially
- Allows independent or shared scales across panels
- Scales to many categories better than color encoding

## Data Preparation

Faceting requires **tidy data** in long format where:
- Each row represents one observation
- The faceting column contains categorical values (e.g., species, region, year)
- All data for different facets exists in the same DataFrame

For example, the Iris dataset is already in the right format:

| sepal_length | sepal_width | species |
|--------------|-------------|---------|
| 5.1 | 3.5 | Iris-setosa |
| 7.0 | 3.2 | Iris-versicolor |
| 6.3 | 3.3 | Iris-virginica |

The `species` column has three distinct values, perfect for creating three facet panels.

## Your First Row Facet

Row facets stack panels vertically, creating a column of charts. This layout works well when you want to compare patterns while scanning down the page.

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();

// Load the iris dataset
let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await?;

// Create a row facet plot
let plot = Plot::<FacetRow>::new()
    .data(df)
    .mark(
        Subplot::new(
            Plot::<Cartesian>::new().mark(
                Symbol::new()
                    .x(col("sepal_length"))
                    .y(col("sepal_width"))
                    .size(36.0)
                    .fill("#4682b4")
            )
        )
        .row(col("species"))
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

This creates three vertically stacked scatter plots, one for each iris species. Each panel shows the relationship between sepal length and sepal width for that species.

## Your First Column Facet

Column facets arrange panels horizontally. This layout is effective when you have limited vertical space or want to emphasize left-to-right comparisons.

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();

// Load the iris dataset
let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await?;

// Create a column facet plot
let plot = Plot::<FacetColumn>::new()
    .data(df)
    .mark(
        Subplot::new(
            Plot::<Cartesian>::new().mark(
                Symbol::new()
                    .x(col("sepal_length"))
                    .y(col("sepal_width"))
                    .size(36.0)
                    .fill("#4682b4")
            )
        )
        .column(col("species"))
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

This creates three horizontally arranged scatter plots with the same data relationship.

## Understanding the Output

A faceted chart consists of several components:

- **Facet Labels**: Text showing the category value for each panel (e.g., "Iris-setosa", "Iris-versicolor")
- **Subplots**: The inner chart content (scatter plots in our examples)
- **Axes**: Each panel has its own axes (by default, with independent scales)
- **Spacing**: Gaps between panels to visually separate them

### The Structure

The key components in the code are:

1. **Plot Coordinate System**: Use `Plot::<FacetRow>` or `Plot::<FacetColumn>` instead of `Plot::<Cartesian>`
2. **Subplot Mark**: The `Subplot::new(...)` mark that wraps your inner plot
3. **Faceting Column**: Specified with `.row(col("species"))` or `.column(col("species"))`
4. **Subplot**: The inner `Plot::<Cartesian>` containing your actual visualization marks

## Basic Customization

### Adding a Facet Title

You can add a title above the facet labels to describe what the faceting represents:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();

// Load the iris dataset
let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await?;

// Create a column facet with a title
let plot = Plot::<FacetColumn>::new()
    .data(df)
    .mark(
        Subplot::new(
            Plot::<Cartesian>::new().mark(
                Symbol::new()
                    .x(col("sepal_length"))
                    .y(col("sepal_width"))
                    .size(36.0)
                    .fill("#4682b4")
            )
        )
        .col_with(col("species"), |c| {
            c.facet(|f| f.title("Iris Species"))
        })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

The title "Iris Species" appears above the facet labels, providing context for what the panels represent.

### Facet Gaps

Facet gaps are computed from the measured subplot overflows, so tick labels and
facet labels have enough room without a separate spacing setting:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();

// Load the iris dataset
let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await?;

// Create a column facet. The gap between panels is measured automatically.
let plot = Plot::<FacetColumn>::new()
    .data(df)
    .mark(
        Subplot::new(
            Plot::<Cartesian>::new().mark(
                Symbol::new()
                    .x(col("sepal_length"))
                    .y(col("sepal_width"))
                    .size(36.0)
                    .fill("#4682b4")
            )
        )
        .col_with(col("species"), |c| {
            c.facet(|f| f.title("Iris Species"))
        })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

The spacing value is in pixels - larger values create more visual separation between panels.

## Next Steps

Now that you've created your first faceted charts, explore these advanced topics:

- **[Scale Sharing](scale-sharing.md)** - Control whether scales are independent or unified across facets
- **[Customization](customization.md)** - Learn about axis positioning, advanced spacing, and configuration options
- **[Nested Facets](nested-facets.md)** - Create multi-level faceting for complex categorical structures

You can also explore using different mark types in your subplots - faceting works with any mark that operates in Cartesian coordinates, including:
- [Line](../marks/line.md) marks for time series comparisons
- [Rect](../marks/rect.md) marks for faceted bar charts
- [Area](../marks/area.md) marks for stacked area comparisons
