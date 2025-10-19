# Legends

Legends provide visual keys that explain how data is encoded in your visualization. Avenger Chart automatically generates legends for encoded channels.

**Note**: Legends are for **visual channels** (fill, size, shape, stroke, etc.) and are independent of coordinate systems. For position channels (x, y, r, theta), see **[Axes](./guides-axes-legends/axes.md)**. To understand how legends, axes, and guides relate, see **[Guides, Axes, and Legends](./guides-axes-legends.md)**.

## Legend Configuration Approach

**Important**: In Avenger Chart, legends are configured at the **mark/channel level**, not at the plot level. When you encode a channel with data (using `*_with()` methods), you configure the legend for that specific channel within the channel builder closure.

This approach provides fine-grained control, allowing each mark to customize its legend appearance and behavior independently.

## Enabling Legends

Configure legends via the channel builder closure with `.legend()`:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
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
                c.scale(|s| s.range_interval(lit(80.0), lit(300.0)))
                    .legend(|l| l.title("Petal Length (cm)"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

This creates two legend entries: one for color (species) and one for size (petal length).

## Legend Titles

Customize legend titles with `.title()`:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    ?;


let plot = Plot::<Cartesian>::new()
    .data(df)
    .title("Custom Legend Titles")
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .size(150.0)
            .fill_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Iris Species"))  // Custom title
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

**Default**: If no title is provided, Avenger Chart uses the referenced column name when possible, or falls back to the channel name.

## Legend Types

### Categorical Legends

For discrete values, legends display colored markers for each category:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    ?;


let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("petal_length"))
            .y(col("petal_width"))
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

Displays:
- Colored circles for each category
- Category labels
- Title

### Continuous Legends (Colorbars)

For quantitative color scales, legends display as gradient colorbars:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
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
            .size(180.0)
            .fill_with(col("petal_length"), |c| {
                c.scale_with::<Linear>(|s| s)
                    .legend(|l| l.title("Petal Length (cm)"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Displays:
- Vertical color gradient bar
- Domain values at endpoints
- Title

### Size Legends

For symbol sizes, legends show circles at representative sizes:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
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
            .fill("#3498db")
            .size_with(col("petal_length"), |c| {
                c.scale(|s| s.range_interval(lit(80.0), lit(350.0)))
                    .legend(|l| l.title("Petal Length (cm)"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Displays:
- Circles at representative sizes
- Corresponding data values
- Title

### Rectangle Legends

Bar charts and other rectangle-based visualizations support legends for various visual channels.

#### Discrete Fill Legend

Map categorical data to rectangle fill colors:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float32Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();
// Create sample data
let batch = RecordBatch::try_from_iter(vec![
    ("product", Arc::new(StringArray::from(vec!["A", "B", "C", "D"])) as _),
    ("value", Arc::new(Float32Array::from(vec![45.0, 38.0, 52.0, 41.0])) as _),
    ("category", Arc::new(StringArray::from(vec!["Type 1", "Type 2", "Type 1", "Type 3"])) as _),
])?;

let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x_with(col("product"), |c| {
                c.scale_with::<Band>(|s| s.padding_inner(0.1))
            })
            .x2_with(col(":x"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("value"))
            .fill_with(col("category"), |c| {
                c.legend(|l| l.title("Category"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

#### Continuous Fill Legend

Map quantitative data to a color gradient:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float32Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    ("quarter", Arc::new(StringArray::from(vec!["Q1", "Q2", "Q3", "Q4"])) as _),
    ("sales", Arc::new(Float32Array::from(vec![25.0, 45.0, 60.0, 35.0])) as _),
    ("temperature", Arc::new(Float32Array::from(vec![10.0, 25.0, 35.0, 18.0])) as _),
])?;

let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x_with(col("quarter"), |c| {
                c.scale_with::<Band>(|s| s.padding_inner(0.15))
            })
            .x2_with(col(":x"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("sales"))
            .fill_with(col("temperature"), |c| {
                c.scale(|s| s.domain((0.0, 40.0)))
                    .legend(|l| l.title("Temperature (°C)"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

#### Stroke Legend

Encode categories with rectangle stroke colors:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float32Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    ("product", Arc::new(StringArray::from(vec!["Widget A", "Widget B", "Widget C", "Widget D"])) as _),
    ("value", Arc::new(Float32Array::from(vec![35.0, 42.0, 28.0, 51.0])) as _),
    ("quality", Arc::new(StringArray::from(vec!["Premium", "Standard", "Premium", "Budget"])) as _),
])?;

let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x_with(col("product"), |c| {
                c.scale_with::<Band>(|s| s.padding_inner(0.1))
            })
            .x2_with(col(":x"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("value"))
            .fill_with(lit("#1f77b4"), |c| c.no_scale())
            .stroke_with(col("quality"), |c| {
                c.scale(|s| s
                    .range_discrete(vec!["#d62728", "#2ca02c", "#ff7f0e"])
                    .domain(vec![lit("Premium"), lit("Standard"), lit("Budget")])
                )
                .legend(|l| l.title("Quality Tier"))
            })
            .stroke_width(3.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Line Legends

Line charts support legends for stroke color, width, and dash patterns.

#### Stroke Color Legend

Distinguish multiple series with different colors:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float32Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    ("x", Arc::new(Float32Array::from(vec![
        1.0, 2.0, 3.0, 4.0, 5.0,
        1.0, 2.0, 3.0, 4.0, 5.0,
        1.0, 2.0, 3.0, 4.0, 5.0,
    ])) as _),
    ("y", Arc::new(Float32Array::from(vec![
        10.0, 20.0, 15.0, 25.0, 30.0,
        5.0, 15.0, 20.0, 18.0, 22.0,
        8.0, 12.0, 18.0, 20.0, 26.0,
    ])) as _),
    ("series", Arc::new(StringArray::from(vec![
        "A", "A", "A", "A", "A",
        "B", "B", "B", "B", "B",
        "C", "C", "C", "C", "C",
    ])) as _),
    ("order", Arc::new(Float32Array::from(vec![
        1.0, 2.0, 3.0, 4.0, 5.0,
        1.0, 2.0, 3.0, 4.0, 5.0,
        1.0, 2.0, 3.0, 4.0, 5.0,
    ])) as _),
])?;

let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Line::new()
            .x(col("x"))
            .y(col("y"))
            .stroke_with(col("series"), |c| {
                c.legend(|l| l.title("Series"))
            })
            .order(col("order"))
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

#### Stroke Width Legend

Use line thickness to encode importance or categories:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float32Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    ("x", Arc::new(Float32Array::from(vec![
        1.0, 2.0, 3.0, 4.0, 5.0,
        1.0, 2.0, 3.0, 4.0, 5.0,
        1.0, 2.0, 3.0, 4.0, 5.0,
    ])) as _),
    ("y", Arc::new(Float32Array::from(vec![
        10.0, 12.0, 11.0, 13.0, 14.0,
        7.0, 9.0, 8.0, 10.0, 11.0,
        4.0, 6.0, 5.0, 7.0, 8.0,
    ])) as _),
    ("importance", Arc::new(StringArray::from(vec![
        "Low", "Low", "Low", "Low", "Low",
        "Medium", "Medium", "Medium", "Medium", "Medium",
        "High", "High", "High", "High", "High",
    ])) as _),
    ("order", Arc::new(Float32Array::from(vec![
        1.0, 2.0, 3.0, 4.0, 5.0,
        1.0, 2.0, 3.0, 4.0, 5.0,
        1.0, 2.0, 3.0, 4.0, 5.0,
    ])) as _),
])?;

let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Line::new()
            .x(col("x"))
            .y(col("y"))
            .stroke_with(lit("#1f77b4"), |c| c.no_scale())
            .stroke_width_with(col("importance"), |c| {
                c.scale_with(|s: Scale<Ordinal>| s
                    .range_discrete(vec![1.0, 3.0, 6.0])
                    .domain(vec![lit("Low"), lit("Medium"), lit("High")])
                )
                .legend(|l| l.title("Importance"))
            })
            .order(col("order"))
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

#### Stroke Dash Legend

Encode categories using different dash patterns:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float32Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    ("x", Arc::new(Float32Array::from(vec![
        1.0, 2.0, 3.0, 4.0, 5.0,
        1.0, 2.0, 3.0, 4.0, 5.0,
        1.0, 2.0, 3.0, 4.0, 5.0,
    ])) as _),
    ("y", Arc::new(Float32Array::from(vec![
        10.0, 11.0, 9.0, 12.0, 13.0,
        7.0, 8.0, 6.0, 9.0, 10.0,
        4.0, 5.0, 3.0, 6.0, 7.0,
    ])) as _),
    ("line_type", Arc::new(StringArray::from(vec![
        "Actual", "Actual", "Actual", "Actual", "Actual",
        "Forecast", "Forecast", "Forecast", "Forecast", "Forecast",
        "Target", "Target", "Target", "Target", "Target",
    ])) as _),
    ("order", Arc::new(Float32Array::from(vec![
        1.0, 2.0, 3.0, 4.0, 5.0,
        1.0, 2.0, 3.0, 4.0, 5.0,
        1.0, 2.0, 3.0, 4.0, 5.0,
    ])) as _),
])?;

let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .title("Sales Comparison")
    .mark(
        Line::new()
            .x(col("x"))
            .y(col("y"))
            .stroke_with(col("line_type"), |c| c)
            .stroke_dash_with(col("line_type"), |c| {
                c.legend(|l| l.title("Line Type"))
            })
            .order(col("order"))
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Multiple Legends

When multiple channels have legends, they stack in the legend area:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
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
                c.scale(|s| s.range_interval(lit(80.0), lit(280.0)))
                    .legend(|l| l.title("Petal Length"))
            })
            .shape_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Species (shape)"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

This creates three separate legend sections stacked vertically on the right side.

### Combined Channel Legends

When a single data column drives multiple visual channels (fill, size, shape, etc.), Avenger Chart creates a unified legend entry that combines all visual properties. This is a powerful pattern for encoding rich information with a single categorical variable.

#### Combining Size, Color, and Shape

Use one column to control size, fill color, and shape simultaneously:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float32Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    ("category", Arc::new(StringArray::from(vec![
        "Type A", "Type B", "Type C", "Type A", "Type B", "Type C",
    ])) as _),
    ("x", Arc::new(Float32Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0])) as _),
    ("y", Arc::new(Float32Array::from(vec![2.0, 4.0, 3.0, 5.0, 6.0, 1.0])) as _),
])?;

let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            // All three channels use the same column
            .size_with(col("category"), |c| {
                c.scale(|s| s.range_discrete(vec![30.0, 120.0, 480.0]))
            })
            .fill_with(col("category"), |c| {
                c.scale(|s| s.range_discrete(vec!["#e41a1c", "#377eb8", "#4daf4a"]))
                    .legend(|l| l.title("Type"))
            })
            .shape_with(col("category"), |c| {
                c.scale(|s| s.range_discrete(vec!["circle", "square", "triangle-up"]))
            })
            .stroke_with(lit("#000000"), |c| c.no_scale())
            .stroke_width_with(lit(1.0), |c| c.no_scale())
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

**How it works**:
- When multiple channels reference the same data column, they appear as a single combined legend
- Each legend symbol shows all visual properties (size, color, shape) for that category
- Only one `.legend()` configuration needed (usually on the primary channel like `fill`)
- Creates a more compact, intuitive legend than separate sections

**When to use**:
- ✅ Encoding rich categorical information efficiently
- ✅ Creating distinctive markers for complex datasets
- ✅ Improving accessibility (redundant encoding helps color-blind users)

## Legend Position

Control where legends appear with `.position()`:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
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
            .size(150.0)
            .fill_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Species").position(LegendPosition::Top))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

**Available positions**:
- `Right` - Right side of the plot (default)
- `Left` - Left side of the plot
- `Top` - Above the plot
- `Bottom` - Below the plot

## Colorbar Styling

Continuous color encodings render as colorbars. You can customize their appearance:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
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
            .size(180.0)
            .fill_with(col("petal_length"), |c| {
                c.scale_with::<Linear>(|s| s)
                    .legend(|l| {
                        l.title("Petal Length (cm)")
                            .background_fill("#f3f4f6")
                            .background_stroke("#9ca3af")
                            .background_corner_radius(6.0)
                            .background_padding(10.0)
                            .gradient_thickness(18.0)
                    })
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Colorbar options:
- `gradient_thickness` - Width/height of the gradient bar
- `background_fill` - Background color
- `background_stroke` - Border color
- `background_corner_radius` - Rounded corners
- `background_padding` - Padding around the gradient

## Selective Legends

Disable legends for specific channels with `.visible(false)`:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    ?;


let plot = Plot::<Cartesian>::new()
    .data(df)
    .title("Legend for Color Only")
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .fill_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Species").visible(true))  // Show
            })
            .size_with(col("petal_length"), |c| {
                c.scale(|s| s.range_interval(lit(80.0), lit(280.0)))
                    .legend(|l| l.visible(false))  // Hide
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Only the color legend appears; size variation is visible but not explained in the legend.

### Using `.no_legend()`

For channels that shouldn't have a legend at all, use `.no_legend()` instead of `.legend(|l| l.visible(false))`:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    ?;


let plot = Plot::<Cartesian>::new()
    .data(df)
    .title("No Legend for Fill")
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .fill_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .no_legend()  // Completely disable legend
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

**Difference between `.no_legend()` and `.legend(|l| l.visible(false))`**:

| Method | When to use |
|--------|-------------|
| `.no_legend()` | Default behavior - no legend created for this channel |
| `.legend(|l| l.visible(false))` | Legend exists but hidden - can be toggled via interactivity (future feature) |

**Recommendation**: Use `.no_legend()` for channels you never want in the legend. Use `.visible(false)` when the legend might be shown/hidden dynamically.

## Legend Formatting

### Number Formatting

For continuous scales, format legend values:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
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
            .size(180.0)
            .fill_with(col("petal_length"), |c| {
                c.scale_with::<Linear>(|s| s)
                    .legend(|l| {
                        l.title("Petal Length")
                            .format_number(".2f")  // Two decimal places
                    })
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Format strings follow standard number formatting patterns (e.g., `.2f` for two decimal places, `.2s` for SI notation).

## Legend Ordering

Legends appear in the order channels are defined. Here's an example with color defined before size:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    ?;

// Color legend appears first, then size

let plot = Plot::<Cartesian>::new()
    .data(df)
    .title("Color First, Then Size")
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

And here's the same plot with size defined before color:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    ?;

// Size legend appears first, then color

let plot = Plot::<Cartesian>::new()
    .data(df)
    .title("Size First, Then Color")
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .size_with(col("petal_length"), |c| {
                c.scale(|s| s.range_interval(lit(80.0), lit(280.0)))
                    .legend(|l| l.title("Petal Length"))
            })
            .fill_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Species"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Legend Styling

Legends inherit styling from the current theme. See [Themes](./themes.md) for customization options.

## Interactive Legends

**Planned feature**: Click legend entries to filter data or highlight marks.

## Hiding All Legends

Global legend toggles are not yet available; hide legends per-channel via `.legend(|l| l.visible(false))`.

## Next Steps

- Learn about [Themes](./themes.md) for complete styling control
- See legend examples in [Scatter Plots](../scatter-plots.md)
- Explore [CSS Themes](../themes/css-themes.md) for advanced customization
