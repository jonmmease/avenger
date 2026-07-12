# Bar Charts

Bar charts use rectangular bars to represent categorical data, making them ideal for comparing values across different categories. All bar charts in Avenger Chart use the [Rect mark](../marks/rect.md) combined with band scales for categorical positioning.

## Vertical Bar Chart

The most common bar chart orientation with categories on the x-axis:

```rust,render
use avenger_chart::prelude::*;

let ctx = SessionContext::new();
let plot = Chart::<Cartesian>::new()
    .data(datasets::categorical_bars(&ctx))
    .mark(
        Rect::new()
            .x(col("category"))
            .x2_with(col(":x"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("value"))
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Horizontal Bar Chart

Horizontal bars are useful when category names are long or when you have many categories:

```rust,render
use avenger_chart::prelude::*;

let ctx = SessionContext::new();
let plot = Chart::<Cartesian>::new()
    .data(datasets::categorical_bars(&ctx))
    .mark(
        Rect::new()
            .x(lit(0.0))
            .x2(col("value"))
            .y(col("category"))
            .y2_with(col(":y"), |c| c.band(1.0))
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Styling Bars

Customize bar appearance with fill colors, strokes, and other visual properties:

```rust,render
use avenger_chart::prelude::*;

let ctx = SessionContext::new();
let plot = Chart::<Cartesian>::new()
    .data(datasets::categorical_bars(&ctx))
    .mark(
        Rect::new()
            .x(col("category"))
            .x2_with(col(":x"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("value"))
            .fill("#3498db")
            .stroke("#2c3e50")
            .stroke_width(2.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Adjusting Bar Spacing

Control the spacing between bars using band scale padding:

```rust,render
use avenger_chart::prelude::*;

let ctx = SessionContext::new();
let plot = Chart::<Cartesian>::new()
    .data(datasets::categorical_bars(&ctx))
    .mark(
        Rect::new()
            .x_with(col("category"), |c| {
                c.scale_with::<Band>(|s| s.padding_inner(0.3))
            })
            .x2_with(col(":x"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("value"))
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Color-Coded Categories

Use different colors for each category to enhance visual distinction:

```rust,render
use avenger_chart::prelude::*;

let ctx = SessionContext::new();
let plot = Chart::<Cartesian>::new()
    .data(datasets::categorical_bars(&ctx))
    .mark(
        Rect::new()
            .x(col("category"))
            .x2_with(col(":x"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("value"))
            .fill_with(col("category"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Category"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Creating Custom Bar Data

Build your own bar chart data using DataFusion:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{StringArray, Float64Array};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();

// Create sales data
let batch = RecordBatch::try_from_iter(vec![
    (
        "product",
        Arc::new(StringArray::from(vec!["Laptops", "Phones", "Tablets", "Watches", "Earbuds"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "revenue",
        Arc::new(Float64Array::from(vec![45000.0, 62000.0, 28000.0, 31000.0, 18000.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])?;

let df = ctx.read_batch(batch)?;

let plot = Chart::<Cartesian>::new()
    .data(df)
    .title("Product Revenue")
    .mark(
        Rect::new()
            .x_with(col("product"), |c| c.axis(|a| a.title("Product")))
            .x2_with(col(":x"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2_with(col("revenue"), |c| c.axis(|a| a.title("Revenue ($)")))
            .fill("#27ae60")
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Rounded Corners

Add rounded corners for a softer appearance:

```rust,render
use avenger_chart::prelude::*;

let ctx = SessionContext::new();
let plot = Chart::<Cartesian>::new()
    .data(datasets::categorical_bars(&ctx))
    .mark(
        Rect::new()
            .x(col("category"))
            .x2_with(col(":x"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("value"))
            .fill("#e74c3c")
            .corner_radius(5.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Advanced Techniques

### Grouped Bars

For comparing multiple series across categories, use a nested band x scale. The
outer struct field is the group, and the inner field is the bar within each
group. Hide the inner axis level for the standard grouped-bar presentation and
use a regular fill legend for the inner series.

```rust,no_run
use avenger_chart::prelude::*;
use datafusion::prelude::{col, lit, named_struct};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
// Prepare data with category and series columns
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let nested_x = named_struct(vec![
    lit("category"), col("category"),
    lit("series"), col("series"),
]);

let plot = Chart::<Cartesian>::new()
    .data(df)
    .legend("fill", |legend| legend.title("Series"))
    .mark(
        Rect::new()
            .x_with(nested_x, |x| {
                x.axis(|axis| axis.title("Category").grid(false))
                    .level(0, |level| level.padding_inner(0.45).padding_outer(0.15))
                    .level(1, |level| {
                        level
                            .nest_scope(NestScope::Shared)
                            .padding_inner(0.08)
                            .axis(|axis| axis.visible(false))
                    })
            })
            .x2_with(col(":x"), |x| x.band(1.0))
            .y(lit(0.0))
            .y2(col("value"))
            .fill(col("series"))
    );
# Ok(())
# }
```

`NestScope::Shared` keeps the same inner slots under every category. When a
series is missing for one category, Avenger leaves an empty slot instead of
making the remaining bars wider.

### Stacked Bars

Stack values to show part-to-whole relationships:

```rust,no_run
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
// Use DataFusion to calculate cumulative sums
let stacked_df = ctx.sql("
    SELECT category, series, value,
           SUM(value) OVER (PARTITION BY category ORDER BY series) as y2,
           LAG(SUM(value) OVER (PARTITION BY category ORDER BY series), 1, 0)
               OVER (PARTITION BY category ORDER BY series) as y
    FROM data
").await?;

let plot = Chart::<Cartesian>::new()
    .data(stacked_df)
    .mark(
        Rect::new()
            .x(col("category"))
            .x2_with(col(":x"), |c| c.band(1.0))
            .y(col("y"))
            .y2(col("y2"))
            .fill_with(col("series"), |c| {
                c.scale_with::<Ordinal>(|s| s)
            })
    );
# Ok(())
# }
```

### Negative Values

Handle bars with negative values that extend below zero:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{StringArray, Float64Array};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();

let batch = RecordBatch::try_from_iter(vec![
    (
        "quarter",
        Arc::new(StringArray::from(vec!["Q1", "Q2", "Q3", "Q4"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "profit",
        Arc::new(Float64Array::from(vec![15000.0, -8000.0, 22000.0, -3000.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])?;

let df = ctx.read_batch(batch)?;

let plot = Chart::<Cartesian>::new()
    .data(df)
    .title("Quarterly Profit/Loss")
    .mark(
        Rect::new()
            .x(col("quarter"))
            .x2_with(col(":x"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("profit"))
            .fill("#2980b9")
            .stroke("#34495e")
            .stroke_width(1.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Band Scale Details

Band scales are essential for bar charts. Key concepts:

- **`band(1.0)`**: Creates the full band width for the bar
- **`padding_inner`**: Controls space between bars (0.0 = no space, 1.0 = all space)
- **`padding_outer`**: Controls space before first and after last bar
- **`:x` and `:y`**: Special columns that reference the position channel's scaled value
- **Nested bands**: Struct-valued position channels create grouped or nested
  categorical axes; see [Nested Band scales](../scales/nested-band.md)

## Best Practices

1. **Start at zero**: Bar charts should typically start at zero to avoid misleading comparisons
2. **Order meaningfully**: Sort bars by value, alphabetically, or by logical sequence
3. **Label clearly**: Add axis titles and units
4. **Limit categories**: Too many bars (>20) become hard to read
5. **Use consistent colors**: Apply the same color scheme across related charts
6. **Consider orientation**: Use horizontal bars for long category names

## Common Pitfalls

- **Truncated axes**: Starting y-axis at non-zero values distorts comparisons
- **3D effects**: Avoid 3D bars as they make accurate comparison difficult
- **Too many series**: Grouped bars become cluttered with more than 3-4 series
- **Inconsistent widths**: Keep bar widths uniform unless width encodes data

## Next Steps

- Explore the [Rect mark reference](../marks/rect.md) for advanced features
- Learn about [Band scales](../scales/band.md) for categorical positioning
- Learn about [Nested Band scales](../scales/nested-band.md) for grouped bars
  and nested categorical axes
- See [Heatmaps](./heatmap-charts.md) for 2D categorical data
- Review [Stacked layouts](../guides/stacking.md) for part-to-whole visualization
