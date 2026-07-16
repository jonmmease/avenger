# Parameters

Parameters enable dynamic, user-controlled values in charts. They act as placeholders that can be updated without recompiling, enabling interactive controls and data-driven applications.

## What are Parameters?

Parameters are named placeholders in DataFusion expressions that can be bound to values at runtime. Create a parameter with a default value, use it in expressions, then compile once and render multiple times with different values:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;
use datafusion::scalar::ScalarValue;
use indexmap::IndexMap;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();

# // Create simple test data
let batch = RecordBatch::try_from_iter(vec![
    (
        "category",
        Arc::new(StringArray::from(vec!["A", "B", "C", "D", "E"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "value",
        Arc::new(Float64Array::from(vec![45.0, 72.0, 38.0, 95.0, 61.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;

// The ScalarValue default carries the parameter's physical Arrow type.
let threshold = Param::new("threshold", ScalarValue::from(40.0));

// Use parameter in expressions
let status = when(col("value").gt(threshold.expr()), lit("high"))
    .otherwise(lit("low"))
    ?;

// Build the plot with the parameter
let plot = Chart::<Cartesian>::new()
    .data(df)
    .title("Threshold Parameter Demo")
    .param(threshold)
    .mark(
        Rect::new()
            .x_with(col("category"), |c| {
                c.scale_with::<Band>(|s| s.padding_inner(0.2))
            })
            .x2_with(col("category"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("value"))
            .fill_with(status, |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Status"))
            })
    );

// Compile once
let compiled = plot.compile(&ctx).await?;

// Evaluate with default threshold (40.0)
let default = compiled.evaluate(&ctx, None).await?;

// Render with threshold=60.0
let mut params_60 = IndexMap::new();
params_60.insert("threshold".to_string(), ScalarValue::from(60.0));
let mid = compiled.evaluate(&ctx, Some(params_60)).await?;

// Render with threshold=80.0
let mut params_80 = IndexMap::new();
params_80.insert("threshold".to_string(), ScalarValue::from(80.0));
let high = compiled.evaluate(&ctx, Some(params_80)).await?;

Ok((default, mid, high))
```

This demonstrates the key benefit: **compile once, render multiple times** with different parameter values. The three images above show the same data colored differently based on the threshold parameter (40, 60, and 80).

## Creating Parameters

### Basic Parameters

```rust,no_run
# use avenger_chart::prelude::*;
use datafusion::scalar::ScalarValue;

# fn example() {
let threshold = Param::new("threshold", ScalarValue::from(100.0));
let category = Param::new("category", ScalarValue::from("A"));
# }
```

### Parameter Types

Rust parameters derive their physical Arrow type from the default
`ScalarValue`. `ScalarValue` preserves exact numeric widths, temporal metadata,
nested list/struct/map fields, and typed nulls. Use an explicit scalar variant
when that precision matters. Parameters can use any supported DataFusion scalar
type:

- **Numeric**: `Float64`, `Int64`, `UInt32`, etc.
- **String**: `Utf8`
- **Boolean**: `Boolean`
- **Temporal**: `Date32`, `Date64`, `Timestamp`
- **Null**: a typed null such as `Int32(None)` or a typed nested scalar

## Using Parameters in Expressions

Parameters generate DataFusion expressions via `.expr()`:

### Color Encoding

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;
use datafusion::scalar::ScalarValue;
use datafusion::arrow::array::{Float64Array};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;
use indexmap::IndexMap;

let ctx = SessionContext::new();

# let batch = RecordBatch::try_from_iter(vec![
#     ("x", Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0])) as _),
#     ("y", Arc::new(Float64Array::from(vec![30.0, 55.0, 42.0, 68.0, 35.0, 72.0, 48.0, 61.0])) as _),
# ])?;
# let df = ctx.read_batch(batch)?;
#
let threshold = Param::new("threshold", 45.0);
let status = when(col("y").gt(threshold.expr()), lit("above"))
    .otherwise(lit("below"))
    ?;

let plot = Chart::<Cartesian>::new()
    .data(df)
    .title("Color Encoding with Parameters")
    .param(threshold)
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .size(200.0)
            .fill_with(status, |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Status"))
            })
    );

let compiled = plot.compile(&ctx).await?;

// Render with threshold=45.0 (5 above, 3 below)
let threshold_45 = compiled.evaluate(&ctx, None).await?;

// Render with threshold=65.0 (3 above, 5 below)
let mut params_65 = IndexMap::new();
params_65.insert("threshold".to_string(), ScalarValue::from(65.0));
let threshold_65 = compiled.evaluate(&ctx, Some(params_65)).await?;

Ok((threshold_45, threshold_65))
```

### Position Encoding

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;
use datafusion::scalar::ScalarValue;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;
use indexmap::IndexMap;

let ctx = SessionContext::new();

# let batch = RecordBatch::try_from_iter(vec![
#     ("x", Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0])) as _),
#     ("y", Arc::new(Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 4.0])) as _),
# ])?;
# let df = ctx.read_batch(batch)?;
#
let offset = Param::new("offset", 0.0);

let plot = Chart::<Cartesian>::new()
    .data(df)
    .title("Position Offset with Parameters")
    .param(offset.clone())
    .mark(
        Symbol::new()
            .x_with(col("x") + offset.expr(), |c| c.scale_with::<Linear>(|s| s.domain_interval(lit(0.0), lit(6.0))))
            .y_with(col("y"), |c| c.scale_with::<Linear>(|s| s.domain_interval(lit(0.0), lit(6.0))))
            .size(200.0)
    );

let compiled = plot.compile(&ctx).await?;

// Render with no offset
let no_offset = compiled.evaluate(&ctx, None).await?;

// Render with offset of 0.5
let mut params = IndexMap::new();
params.insert("offset".to_string(), ScalarValue::from(0.5));
let with_offset = compiled.evaluate(&ctx, Some(params)).await?;

Ok((no_offset, with_offset))
```

## Binding Parameter Values

Parameters must be set on the `Plot` (as defaults) and then overridden at render time. This workflow allows you to compile once and render multiple times with different parameter values.

Here's an example using parameters for filtering data:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;
use datafusion::scalar::ScalarValue;
use indexmap::IndexMap;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();

# let batch = RecordBatch::try_from_iter(vec![
#     ("x", Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0])) as _),
#     ("value", Arc::new(Float64Array::from(vec![45.0, 55.0, 42.0, 68.0, 52.0, 72.0, 48.0, 61.0, 80.0, 58.0])) as _),
# ])?;
# let df = ctx.read_batch(batch)?;
#
// Create parameter with default value
let threshold = Param::new("threshold", 50.0);

let filtered = df.filter(col("value").gt(threshold.expr()))?;

// Compile plot with default parameter
let compiled = Chart::<Cartesian>::new()
    .data(filtered)
    .title("Binding Parameter Values")
    .param(threshold.clone())
    .mark(
        Symbol::new()
            .x_with(col("x"), |c| c.scale_with::<Linear>(|s| s.domain_interval(lit(1.0), lit(11.0))))
            .y(col("value"))
            .size(200.0)
    )
    .compile(&ctx)
    .await
    ?;

// Render with default parameter value (50.0) - shows 7 points
let render1 = compiled.evaluate(&ctx, None).await?;

// Render with overridden parameter value (65.0) - shows 4 points
let mut params = IndexMap::new();
params.insert("threshold".to_string(), ScalarValue::from(65.0));
let render2 = compiled.evaluate(&ctx, Some(params)).await?;

Ok((render1, render2))
```

The parameter is used during both compilation (for data filtering) and rendering (for final visualization). Overriding at render time is efficient for interactive applications.

## Multiple Parameters

Combine multiple parameters:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;
use datafusion::scalar::ScalarValue;
use indexmap::IndexMap;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();

# let batch = RecordBatch::try_from_iter(vec![
#     ("x", Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0])) as _),
#     ("value", Arc::new(Float64Array::from(vec![25.0, 45.0, 55.0, 62.0, 75.0, 80.0, 58.0, 90.0, 52.0, 95.0])) as _),
#     ("category", Arc::new(StringArray::from(vec!["A", "B", "A", "B", "A", "B", "A", "B", "A", "B"])) as _),
# ])?;
# let df = ctx.read_batch(batch)?;
#
let threshold = Param::new("threshold", 50.0);
let highlight_category = Param::new("highlight", "A");

// Use parameters in color expression
let fill_color = when(
    col("value").gt(threshold.expr()).and(col("category").eq(highlight_category.expr())),
    lit("highlight")
)
.when(col("value").gt(threshold.expr()), lit("above"))
.otherwise(lit("below"))
?;

let plot = Chart::<Cartesian>::new()
    .data(df)
    .title("Multiple Parameters")
    .param(threshold)   // Add all parameters to plot
    .param(highlight_category)
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("value"))
            .size(200.0)
            .fill_with(fill_color, |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Status"))
            })
    );

let compiled = plot.compile(&ctx).await?;

// Render with defaults (threshold=50, highlight="A")
let render1 = compiled.evaluate(&ctx, None).await?;

// Render with different parameters (threshold=60, highlight="B")
let mut params = IndexMap::new();
params.insert("threshold".to_string(), ScalarValue::from(60.0));
params.insert("highlight".to_string(), ScalarValue::from("B"));
let render2 = compiled.evaluate(&ctx, Some(params)).await?;

Ok((render1, render2))
```

## Parameter Expressions

Parameters can be used in complex expressions:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;
use datafusion::scalar::ScalarValue;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;
use indexmap::IndexMap;

let ctx = SessionContext::new();

# let batch = RecordBatch::try_from_iter(vec![
#     ("x", Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0])) as _),
#     ("y", Arc::new(Float64Array::from(vec![1.0, 2.0, 1.5, 2.5, 2.0])) as _),
# ])?;
# let df = ctx.read_batch(batch)?;
#
let scale_factor = Param::new("scale", 1.0);
let offset = Param::new("offset", 0.0);

let plot = Chart::<Cartesian>::new()
    .data(df)
    .title("Scale and Offset Parameters")
    .param(scale_factor.clone())
    .param(offset.clone())
    .mark(
        Symbol::new()
            .x(col("x"))
            .y_with(
                col("y") * scale_factor.expr() + offset.expr(),
                |c| c.scale_with::<Linear>(|s| s.domain_interval(lit(0.0), lit(7.0)))
            )
            .size(200.0)
    );

let compiled = plot.compile(&ctx).await?;

// Original (scale=1.0, offset=0.0)
let original = compiled.evaluate(&ctx, None).await?;

// Scaled (scale=2.0, offset=0.0)
let mut params_scaled = IndexMap::new();
params_scaled.insert("scale".to_string(), ScalarValue::from(2.0));
let scaled = compiled.evaluate(&ctx, Some(params_scaled)).await?;

// Scaled and offset (scale=2.0, offset=1.0)
let mut params_both = IndexMap::new();
params_both.insert("scale".to_string(), ScalarValue::from(2.0));
params_both.insert("offset".to_string(), ScalarValue::from(1.0));
let both = compiled.evaluate(&ctx, Some(params_both)).await?;

Ok((original, scaled, both))
```

## Default Values

Parameters always have default values used when no binding is provided:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::scalar::ScalarValue;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = datafusion::execution::context::SessionContext::new();
# let df = ctx.read_csv("data.csv", datafusion::execution::options::CsvReadOptions::default()).await?;
let threshold = Param::new("threshold", 50.0);

// If not bound, uses default value of 50.0
let filtered = df.filter(col("value").gt(threshold.expr()))?;

Plot::<Cartesian>::new()
    .data(filtered)
    .param(threshold)  // Add parameter to plot
    .mark(Symbol::new().x(col("x")).y(col("y")));
# Ok(())
# }
```

## Parameter Naming

Use descriptive names:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::scalar::ScalarValue;
# fn example() {
// Good
let user_threshold = Param::new("user_threshold", 50.0);
let selected_year = Param::new("selected_year", 2023_i32);

// Less clear
let p1 = Param::new("p1", 50.0);
let x = Param::new("x", 2023_i32);
# }
```

Parameter names appear in error messages and debugging output.

## Complete Example

```rust,render
use avenger_chart::param::Param;
use avenger_chart::prelude::*;
use datafusion::prelude::*;
use datafusion::scalar::ScalarValue;
use indexmap::IndexMap;

let ctx = SessionContext::new();
let df = ctx
    .read_parquet(avenger_sample_data::stocks_path(), ParquetReadOptions::default())
    .await
    ?;

let aapl = df
    .filter(col("symbol").eq(lit("AAPL")))
    ?;

let threshold = Param::new("price_threshold", 150.0);
let status = when(col("price").gt(threshold.expr()), lit("above"))
    .otherwise(lit("within"))
    ?;

let plot = Chart::<Cartesian>::new()
    .data(aapl)
    .canvas_size(800.0, 500.0)
    .title("AAPL Price Threshold")
    .param(threshold)
    .mark(
        Symbol::new()
            .x_with(col("date"), |c| c.scale_with::<Time>(|s| s).axis(|a| a.title("Date")))
            .y_with(col("price"), |c| c.axis(|a| a.title("Price ($)")))
            .fill_with(status, |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Threshold Status"))
            })
    );

let compiled = plot.compile(&ctx).await?;

// Render with default threshold (150.0)
let default = compiled.evaluate(&ctx, None).await?;

// Render with threshold=100.0
let mut params_100 = IndexMap::new();
params_100.insert("price_threshold".to_string(), ScalarValue::from(100.0));
let low = compiled.evaluate(&ctx, Some(params_100)).await?;

Ok((default, low))
```

## Controlling Guides, Layout, and Legends

Parameters are ordinary expressions, so you can reuse them in axis builders, layout APIs, or legend configuration:

- Toggle grid lines or axis visibility with a boolean parameter: `axis(|a| a.grid(&show_grid_param))`.
- Switch axis positions with a CASE expression:
  `position(when(pos.expr().eq(lit("top")), lit("top")).otherwise(lit("bottom")))`.
- Drive canvas dimensions for responsive renderings: `canvas_size(400.0, height_param.expr())`.
- Adjust legend backgrounds or titles at runtime by referencing parameters inside `.legend(|l| ...)`.

```rust,render
use avenger_chart::prelude::*;
use avenger_chart::param::Param;
use datafusion::prelude::*;
use datafusion::scalar::ScalarValue;
use indexmap::IndexMap;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();

# let batch = RecordBatch::try_from_iter(vec![
#     ("x", Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0])) as _),
#     ("y", Arc::new(Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 4.0])) as _),
# ])?;
# let df = ctx.read_batch(batch)?;
#
let height = Param::new("height", 250.0);
let show_grid = Param::new("show_grid", true);

let plot = Chart::<Cartesian>::new()
    .param(height.clone())
    .param(show_grid.clone())
    .canvas_size(400.0, height.expr())
    .title("Responsive Scatter with Grid Toggle")
    .data(df)
    .mark(
        Symbol::new()
            .x_with(col("x"), |c| c.axis(|a| a.grid(&show_grid)))
            .y_with(col("y"), |c| c.axis(|a| a.grid(true)))
            .size(200.0)
    );

let compiled = plot.compile(&ctx).await?;

// Render with defaults (height=250, grid=true)
let default = compiled.evaluate(&ctx, None).await?;

// Render with overrides (height=400, grid=false)
let mut overrides = IndexMap::new();
overrides.insert("height".into(), ScalarValue::from(400.0));
overrides.insert("show_grid".into(), ScalarValue::from(false));
let override_render = compiled.evaluate(&ctx, Some(overrides)).await?;

Ok((default, override_render))
```

## Feeding Parameters into CSS Themes

When you add a parameter to a plot, the theme receives a `--param-name` CSS variable. This makes it straightforward to expose interactive palettes or responsive guide styling directly in CSS.

```rust,render
use avenger_chart::prelude::*;
use avenger_chart::param::Param;
use avenger_chart::theme::Theme;
use datafusion::prelude::*;
use datafusion::scalar::ScalarValue;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;
use indexmap::IndexMap;

let ctx = SessionContext::new();

# let batch = RecordBatch::try_from_iter(vec![
#     ("x", Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0])) as _),
#     ("y", Arc::new(Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 4.0])) as _),
# ])?;
# let df = ctx.read_batch(batch)?;
#
let accent = Param::new("--accent", "#2563eb");

let mut theme = Theme::light();
theme.append_css(
    r#"
    mark[type="symbol"] {
        fill: var(--accent);
        stroke: color-mix(in srgb, var(--accent) 60%, black);
        stroke-width: 1.5px;
    }
    "#,
)?;

let plot = Chart::<Cartesian>::new()
    .data(df)
    .title("Parameter-Driven CSS Theme")
    .param(accent)
    .theme(theme)
    .mark(Symbol::new().x(col("x")).y(col("y")).size(200.0));

let compiled = plot.compile(&ctx).await?;

// Render with default accent color (blue)
let blue = compiled.evaluate(&ctx, None).await?;

// Render with different accent color (red)
let mut params = IndexMap::new();
params.insert("--accent".to_string(), ScalarValue::from("#dc2626"));
let red = compiled.evaluate(&ctx, Some(params)).await?;

Ok((blue, red))
```

At render time you can override the `accent` parameter to swap the palette without recompiling the chart.

## Next Steps

- Use parameters in [mark effects](mark-effects.md)
- Explore parameterized styling in [CSS Themes](themes/css-themes.md)
