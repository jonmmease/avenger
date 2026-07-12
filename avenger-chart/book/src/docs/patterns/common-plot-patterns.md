# Common Plot Patterns

This section provides ready-to-use recipes for the most common chart types in data visualization. Each pattern page includes complete, working examples that you can copy, modify, and combine for your own projects.

## Available Chart Types

### [Scatter Charts](./scatter-charts.md)
Visualize relationships between two continuous variables. Perfect for:
- Correlation analysis
- Outlier detection
- Cluster identification
- Bubble charts with size encoding

### [Line Charts](./line-charts.md)
Show trends and changes over time or ordered categories. Ideal for:
- Time series data
- Trend visualization
- Multi-series comparisons
- Connected scatter plots

### [Bar Charts](./bar-charts.md)
Compare values across categorical dimensions. Great for:
- Category comparisons
- Rankings and distributions
- Grouped and stacked layouts
- Horizontal and vertical orientations

## Quick Start Examples

Each chart type page contains:
- **Basic examples** - Simple, minimal code to get started
- **Styling options** - Customize colors, sizes, and visual properties
- **Advanced techniques** - Multi-series, faceting, and complex encodings
- **Best practices** - Tips for effective data visualization
- **Common pitfalls** - What to avoid and why

## Combining Patterns

Many visualizations benefit from combining multiple chart types:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    ("x", Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0])) as _),
    ("y", Arc::new(Float64Array::from(vec![2.0, 5.0, 3.0, 8.0, 6.0])) as _),
])?;
let df = ctx.read_batch(batch)?;

// Combine line and points for emphasis
let plot = Chart::<Cartesian>::new()
    .title("Combined Line and Points")
    .mark(
        Line::new()
            .data(df.clone())
            .x(col("x"))
            .y(col("y"))
            .stroke("#3498db")
            .stroke_width(2.0)
    )
    .mark(
        Symbol::new()
            .data(df)
            .x(col("x"))
            .y(col("y"))
            .fill("#2c3e50")
            .size(150.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Working with Data

All examples use DataFusion for data handling, which provides:
- **Parquet/CSV/JSON** file reading
- **SQL queries** for data transformation
- **Arrow arrays** for in-memory data
- **Expressions** for computed columns

See [DataFusion Integration](../datafusion-integration.md) for detailed data handling patterns.

## Customization

Every chart can be customized through:
- **[Scales](../scales/index.md)** - Control data-to-visual mappings
- **[Axes](../guides-axes-legends/axes.md)** - Configure ticks, labels, and titles
- **[Legends](../guides-axes-legends/legends.md)** - Add keys for color and size encodings
- **[Themes](../themes/index.md)** - Apply consistent styling across charts

## Next Steps

1. **Choose a chart type** from the pages above
2. **Copy a basic example** as your starting point
3. **Modify the data binding** to use your columns
4. **Customize the appearance** with colors and styles
5. **Add interactivity** with [parameters](../parameters.md)

For more advanced visualizations, explore:
- [Marks Reference](../marks/index.md) - Detailed mark properties
- [Coordinate Systems](../coordinate-systems/index.md) - Cartesian, polar, and more
- [Layout & Sizing](../layout.md) - Control chart dimensions
