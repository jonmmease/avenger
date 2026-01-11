# Nested Facets

Nested facets enable multi-level faceting by placing one facet type inside another. This creates hierarchical small multiples where data is filtered through multiple categorical variables at different levels.

## When to Use Nested Facets

Nested faceting is ideal when you need to explore data along two categorical dimensions simultaneously:

- **Product categories and regions**: Outer facet by category, inner facet by region
- **Time periods and conditions**: Outer facet by year, inner facet by season
- **Species and treatments**: Outer facet by species, inner facet by treatment group

The key advantage is that each level can have its own facet titles, making the hierarchy clear to readers.

## Basic Nested Facet

The most common pattern is `FacetColumn` containing `FacetRow` (or vice versa), creating a grid-like layout.

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
            .unwrap(),
    )?;

// Outer facet: Column by species (3 columns)
// Inner facet: Row by petal_width_bin (variable rows per column)
let plot = Plot::<FacetColumn>::new()
    .data(df)
    .canvas_size(800, 600)
    .mark(
        Facet::new()
            .col_with(col("species"), |c| c.facet(|f| f.title("Species")))
            .subplot(
                Plot::<FacetRow>::new().mark(
                    Facet::new()
                        .row_with(col("petal_width_bin"), |c| {
                            c.facet(|f| f.title("Petal Width"))
                        })
                        .subplot(
                            Plot::<Cartesian>::new().mark(
                                Symbol::new()
                                    .x(col("sepal_length"))
                                    .y(col("sepal_width"))
                                    .size(25.0)
                                    .fill("#4682b4"),
                            ),
                        ),
                ),
            ),
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

This creates a 3x3 layout where:
- The outer level creates 3 columns (one per species)
- The inner level creates 3 rows within each column (one per petal width bin)
- Each cell shows a scatter plot of the filtered data

## Data Flow Through Nested Levels

Understanding data flow is crucial for nested facets:

```
Full Dataset (150 rows)
    |
    v
Outer FacetColumn filters by species
    |
    +-- setosa (50 rows) -----> Inner FacetRow filters by petal_width_bin
    |                               |
    |                               +-- narrow (many rows) -> Scatter plot
    |                               +-- medium (few rows)   -> Scatter plot
    |                               +-- wide (0 rows)       -> Empty plot
    |
    +-- versicolor (50 rows) ----> Inner FacetRow filters by petal_width_bin
    |                               ...
    |
    +-- virginica (50 rows) -----> Inner FacetRow filters by petal_width_bin
                                    ...
```

**Key principle**: Inner facets must NOT have their own `.data()` attachment. Data flows automatically from parent to child via the `data_override` mechanism.

```rust
// CORRECT: Inner facet inherits filtered data
Plot::<FacetColumn>::new()
    .data(df)  // Only the outer facet has data
    .mark(
        Facet::new().column(col("species")).subplot(
            Plot::<FacetRow>::new()  // No .data() here!
                .mark(...)
        )
    )

// INCORRECT: Will cause an error
Plot::<FacetColumn>::new()
    .data(df)
    .mark(
        Facet::new().column(col("species")).subplot(
            Plot::<FacetRow>::new()
                .data(df)  // ERROR: nested facets cannot have their own data
                .mark(...)
        )
    )
```

## Scale Sharing in Nested Contexts

Scale sharing becomes more nuanced with nested facets. Each level can specify different sharing modes.

### Shared Scales Across All Cells

Use `ScaleSharing::Shared` to create a single domain across all cells:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let iris = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await?;

let df = iris
    .with_column(
        "petal_width_bin",
        when(col("petal_width").lt_eq(lit(0.8)), lit("narrow"))
            .when(col("petal_width").lt_eq(lit(1.7)), lit("medium"))
            .otherwise(lit("wide"))
            .unwrap(),
    )?;

let plot = Plot::<FacetColumn>::new()
    .data(df)
    .canvas_size(800, 600)
    .mark(
        Facet::new()
            .col_with(col("petal_width_bin"), |c| c.facet(|f| f.title("Petal Width")))
            .subplot(
                Plot::<FacetRow>::new().mark(
                    Facet::new()
                        .row_with(col("species"), |c| c.facet(|f| f.title("Species")))
                        .subplot(
                            Plot::<Cartesian>::new().mark(
                                Symbol::new()
                                    .x_with(col("sepal_length"), |c| {
                                        c.with_scale_sharing(ScaleSharing::Shared)
                                    })
                                    .y_with(col("sepal_width"), |c| {
                                        c.with_scale_sharing(ScaleSharing::Shared)
                                    })
                                    .size(25.0)
                                    .fill("#4682b4"),
                            ),
                        ),
                ),
            ),
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

With `Shared`, all 9 cells use the same x and y domains, making direct visual comparison easy.

### Free Scales Per Cell

Use `ScaleSharing::Free` to let each cell compute its own optimal scale:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let iris = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await?;

let df = iris
    .with_column(
        "petal_width_bin",
        when(col("petal_width").lt_eq(lit(0.8)), lit("narrow"))
            .when(col("petal_width").lt_eq(lit(1.7)), lit("medium"))
            .otherwise(lit("wide"))
            .unwrap(),
    )?;

let plot = Plot::<FacetColumn>::new()
    .data(df)
    .canvas_size(800, 600)
    .mark(
        Facet::new()
            .col_with(col("petal_width_bin"), |c| c.facet(|f| f.title("Petal Width")))
            .subplot(
                Plot::<FacetRow>::new().mark(
                    Facet::new()
                        .row_with(col("species"), |c| c.facet(|f| f.title("Species")))
                        .subplot(
                            Plot::<Cartesian>::new().mark(
                                Symbol::new()
                                    .x_with(col("sepal_length"), |c| {
                                        c.with_scale_sharing(ScaleSharing::Free)
                                    })
                                    .y_with(col("sepal_width"), |c| {
                                        c.with_scale_sharing(ScaleSharing::Free)
                                    })
                                    .size(25.0)
                                    .fill("#4682b4"),
                            ),
                        ),
                ),
            ),
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

With `Free`, each of the 9 cells optimizes its scales independently, maximizing detail within each cell.

## Level-Based Sharing

`ScaleSharing::Level(1)` enables sharing scales with the immediate parent facet, creating column-wise or row-wise sharing depending on the nesting structure.

### Column-Wise Sharing: Level(1) in FacetColumn > FacetRow

When the outer facet is `FacetColumn`, using `Level(1)` shares scales within each column:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let iris = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await?;

let df = iris
    .with_column(
        "petal_width_bin",
        when(col("petal_width").lt_eq(lit(0.8)), lit("narrow"))
            .when(col("petal_width").lt_eq(lit(1.7)), lit("medium"))
            .otherwise(lit("wide"))
            .unwrap(),
    )?;

let plot = Plot::<FacetColumn>::new()
    .data(df)
    .canvas_size(800, 600)
    .mark(
        Facet::new()
            .col_with(col("petal_width_bin"), |c| c.facet(|f| f.title("Petal Width")))
            .subplot(
                Plot::<FacetRow>::new().mark(
                    Facet::new()
                        .row_with(col("species"), |c| c.facet(|f| f.title("Species")))
                        .subplot(
                            Plot::<Cartesian>::new().mark(
                                Symbol::new()
                                    .x_with(col("sepal_length"), |c| {
                                        c.with_scale_sharing(ScaleSharing::Level(1))
                                    })
                                    .y_with(col("sepal_width"), |c| {
                                        c.with_scale_sharing(ScaleSharing::Level(1))
                                    })
                                    .size(25.0)
                                    .fill("#4682b4"),
                            ),
                        ),
                ),
            ),
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

This creates:
- **Narrow column**: All 3 rows share the same x/y domains (computed from all narrow petal width data)
- **Medium column**: All 3 rows share the same x/y domains (computed from all medium petal width data)
- **Wide column**: All 3 rows share the same x/y domains (computed from all wide petal width data)

This is useful for comparing species within each petal width category.

### Row-Wise Sharing: Level(1) in FacetRow > FacetColumn

Inverting the nesting order enables row-wise sharing:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let iris = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await?;

let df = iris
    .with_column(
        "petal_width_bin",
        when(col("petal_width").lt_eq(lit(0.8)), lit("narrow"))
            .when(col("petal_width").lt_eq(lit(1.7)), lit("medium"))
            .otherwise(lit("wide"))
            .unwrap(),
    )?;

// Note: FacetRow as outer, FacetColumn as inner (inverted from previous)
let plot = Plot::<FacetRow>::new()
    .data(df)
    .canvas_size(800, 600)
    .mark(
        Facet::new()
            .row_with(col("species"), |c| c.facet(|f| f.title("Species")))
            .subplot(
                Plot::<FacetColumn>::new().mark(
                    Facet::new()
                        .col_with(col("petal_width_bin"), |c| {
                            c.facet(|f| f.title("Petal Width"))
                        })
                        .subplot(
                            Plot::<Cartesian>::new().mark(
                                Symbol::new()
                                    .x_with(col("sepal_length"), |c| {
                                        c.with_scale_sharing(ScaleSharing::Level(1))
                                    })
                                    .y_with(col("sepal_width"), |c| {
                                        c.with_scale_sharing(ScaleSharing::Level(1))
                                    })
                                    .size(25.0)
                                    .fill("#4682b4"),
                            ),
                        ),
                ),
            ),
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

This creates:
- **Setosa row**: All 3 columns share x/y domains (computed from all setosa data)
- **Versicolor row**: All 3 columns share x/y domains (computed from all versicolor data)
- **Virginica row**: All 3 columns share x/y domains (computed from all virginica data)

This is useful for comparing petal width categories within each species.

## Performance Considerations

### Stack Size for Deep Nesting

Nested faceting uses recursive evaluation, which can exceed the default Rust stack size for deeply nested structures. When working with 2+ levels of nesting, increase the stack size:

```rust
std::thread::Builder::new()
    .stack_size(64 * 1024 * 1024)  // 64 MB
    .spawn(|| {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build runtime");

        rt.block_on(async {
            // Your nested facet code here
            let plot = Plot::<FacetColumn>::new()
                .data(df)
                .mark(
                    Facet::new().column(col("outer")).subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new().row(col("inner")).subplot(
                                Plot::<Cartesian>::new().mark(...)
                            )
                        )
                    )
                );

            let compiled = plot.compile(&ctx).await?;
            compiled.evaluate(&ctx, None).await
        })
    })
    .expect("spawn thread")
    .join()
    .expect("join thread")
```

This pattern is used in all the visual regression tests for nested facets (see `test_nested_facets.rs`).

### Measurement Overhead

Each faceting level performs a two-pass rendering algorithm:
1. **Pass 1**: Measure all subplots to determine required spacing
2. **Pass 2**: Render with correct layout

With nested facets:
- Outer facet: 2 passes
- Each inner facet: 2 passes
- Total: 2 + (2 × number of outer cells)

For a 3×3 grid created by nesting, this means 8 passes total (2 for outer + 2×3 for inner).

## Limitations

### Currently Supported Nesting Patterns

The following nesting patterns are fully supported:

- `FacetColumn` containing `FacetRow`
- `FacetRow` containing `FacetColumn`
- 3+ levels of nesting (with adequate stack size)

### Level(N) with N > 1

Currently, `Level(2)` and higher are **not fully implemented** for nested facets. The domain propagation mechanism only supports sharing with the immediate parent (Level 1).

For deeper nesting hierarchies, use either:
- `ScaleSharing::Shared` (global sharing across all cells)
- `ScaleSharing::Level(1)` (sharing with immediate parent)
- `ScaleSharing::Free` (independent per cell)

## Summary

Nested facets enable:
- Multi-level categorical decomposition
- Hierarchical data exploration with clear visual structure
- Flexible scale sharing at different levels via `Level(1)`

Key principles:
- Data flows from outer to inner via automatic filtering
- Inner facets must NOT have their own `.data()` attachment
- Use `Level(1)` for parent-level sharing (column-wise or row-wise)
- Increase stack size for 2+ nesting levels

For more details on scale sharing modes, see [Scale Sharing](scale-sharing.md).
