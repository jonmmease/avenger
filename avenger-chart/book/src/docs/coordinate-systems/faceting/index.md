# Faceting

> **Status**: Implemented (FacetRow, FacetColumn)

Faceting creates **small multiples** - a grid or stack of separate panels, each showing a subset of the data based on a categorical variable. This technique, popularized by Edward Tufte and Leland Wilkinson, enables powerful comparisons across categories while maintaining consistent visual encoding within each panel.

## Quick Visual Examples

### Row Faceting

Vertical stack of panels, one per category:

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

### Column Faceting

Horizontal row of panels, one per category:

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

### Nested Faceting

Combine row and column faceting for two-dimensional grids:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let iris = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await?;

// Add binned petal_width column
let df = iris
    .with_column(
        "petal_width_bin",
        when(col("petal_width").lt_eq(lit(0.8)), lit("narrow"))
            .when(col("petal_width").lt_eq(lit(1.7)), lit("medium"))
            .otherwise(lit("wide"))
            .unwrap()
    )
    .unwrap();

let plot = Plot::<FacetColumn>::new()
    .data(df)
    .canvas_size(800, 600)
    .mark(
        Subplot::new(
            Plot::<FacetRow>::new().mark(
                Subplot::new(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x(col("sepal_length"))
                            .y(col("sepal_width"))
                            .size(25.0)
                            .fill("#4682b4")
                    )
                )
                .row(col("petal_width_bin"))
            )
        )
        .column(col("species"))
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Available Facet Types

| Coordinate System | Description | Use Case |
|-------------------|-------------|----------|
| `FacetRow` | Vertical stack of panels | Compare patterns across categories, reading top to bottom |
| `FacetColumn` | Horizontal row of panels | Compare patterns across categories, reading left to right |

Both types can be nested to create multi-dimensional grids.

## Core Concepts

| Concept | Type | Purpose |
|---------|------|---------|
| **Subplot Mark** | `Subplot<FacetRow>` / `Subplot<FacetColumn>` | Creates the faceted layout by splitting data and creating subplot instances |
| **Scale Sharing** | `ScaleSharing` enum | Controls whether scales are unified across facets or independent per panel |
| **Subplot** | `Plot<InnerC>` | The inner chart specification rendered once per facet cell |
| **Coordinate System** | `FacetRow`, `FacetColumn` | Defines the layout direction and coordinate space for facets |

## API Quick Reference

| Question | Solution |
|----------|----------|
| How do I create a row facet? | `Plot::<FacetRow>::new().mark(Subplot::new(...).row(col("category")))` |
| How do I create a column facet? | `Plot::<FacetColumn>::new().mark(Subplot::new(...).column(col("category")))` |
| How do I share scales across facets? | Use `.with_scale_sharing(ScaleSharing::Shared)` on inner mark channels |
| How do I make scales independent? | Use `.with_scale_sharing(ScaleSharing::Free)` on inner mark channels (default) |
| How do I add a facet title? | Use `.row_with(col("cat"), \|c\| c.facet(\|f\| f.title("Category")))` |
| How do I adjust spacing? | Facet spacing is computed from measured subplot overflows. |
| How do I create nested facets? | Use a facet coordinate system as the subplot of another facet |
| How do I create grid-like nested facets? | Use `.row_with(col("cat"), \|c\| c.facet(\|f\| f.share_slots()))` - see [Customization](customization.md#facet-variable-scale-sharing) |

## Terminology

Understanding these terms helps navigate the facet system:

- **Cell**: The rectangular container allocated for each facet panel, including its axes and labels
- **Subplot**: The inner chart (e.g., `Plot::<Cartesian>`) that is rendered within each facet cell
- **Facet Label**: Text showing the category value for each facet panel (e.g., "setosa", "versicolor")
- **Facet Title**: Optional header text describing the faceting dimension (e.g., "Species")
- **Scale Sharing**: Whether axes use unified domains across all facets (shared) or independent domains per facet (free)

## Data Flow

Faceting works by:

1. **Splitting**: The outer facet mark extracts unique values from the faceting column
2. **Filtering**: For each unique value, data is filtered to rows matching that value
3. **Rendering**: The subplot specification is rendered once per filtered dataset
4. **Layout**: Facet cells are arranged according to the coordinate system (row or column)

This means your subplot receives pre-filtered data automatically - you don't need to manually filter or group the data.

## When to Use Faceting

Faceting is ideal when:

- You want to compare patterns across multiple categories
- Overlaying all categories would create visual clutter
- Each category deserves equal visual space and attention
- You need consistent visual encoding (same marks, scales) across categories

Consider alternatives when:

- You have many categories (>10) - faceting may create too many small panels
- Categories have very different value ranges - free scales may help, or consider separate plots
- You want to emphasize differences between categories - overlaid marks with color encoding may be clearer

## Next Steps

- **[Getting Started](getting-started.md)** - Create your first faceted chart with step-by-step guidance
- **[Scale Sharing](scale-sharing.md)** - Learn how to control scale behavior across facets
- **[Customization](customization.md)** - Customize titles, spacing, and axis positioning
- **[Nested Facets](nested-facets.md)** - Build multi-level faceted visualizations

See also:

- [Cartesian Coordinates](../cartesian.md) - The most common inner coordinate system for facets
- [Marks](../../marks/index.md) - Visual encodings to use within facet subplots
- [Scales](../../scales/index.md) - Data transformations and their sharing modes
