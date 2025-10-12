# Parameters

Parameters enable dynamic, user-controlled values in charts. They act as placeholders that can be updated without recompiling, enabling interactive controls and data-driven applications.

## What are Parameters?

Parameters are named placeholders in DataFusion expressions that can be bound to values at runtime. Create a parameter with a default value, use it in expressions, then compile once and render multiple times with different values:

```rust,no_run
use avenger_chart::prelude::*;
use datafusion::prelude::*;
use datafusion::scalar::ScalarValue;
use indexmap::IndexMap;
use palette::Srgba;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = datafusion::execution::context::SessionContext::new();
# let df = ctx.read_csv("data.csv", datafusion::execution::options::CsvReadOptions::default()).await?;
// Create a parameter with a default value
let threshold = Param::new("threshold", ScalarValue::from(50.0));

// Use parameter in expressions
let status = when(col("value").gt(threshold.expr()), lit("high"))
    .otherwise(lit("low"))?;

// Build the plot with the parameter
let plot = Plot::<Cartesian>::new()
    .data(df)
    .add_param(threshold.clone())
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .fill_with(status, |c| {
                c.scale_with::<Ordinal>(|s| s.range_colors(vec![
                    Srgba::new(0.173, 0.627, 0.173, 1.0),  // green for "low"
                    Srgba::new(0.882, 0.470, 0.0, 1.0),    // orange for "high"
                ]))
                .legend(|l| l.title("Status"))
            })
    );

// Compile once
let compiled = plot.compile(&ctx).await?;

// Render with default threshold (50.0)
let _render1 = compiled.render(&ctx, None).await?;

// Render again with different threshold (75.0)
let mut params = IndexMap::new();
params.insert("threshold".to_string(), ScalarValue::from(75.0));
let _render2 = compiled.render(&ctx, Some(params)).await?;
# Ok(())
# }
```

This demonstrates the key benefit: compile once, render multiple times with different parameter values.

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

Parameters can be any DataFusion scalar type:

- **Numeric**: `Float64`, `Int64`, `UInt32`, etc.
- **String**: `Utf8`
- **Boolean**: `Boolean`
- **Temporal**: `Date32`, `Date64`, `Timestamp`
- **Null**: `Null`

## Using Parameters in Expressions

Parameters generate DataFusion expressions via `.expr()`:

### Filtering

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::scalar::ScalarValue;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = datafusion::execution::context::SessionContext::new();
# let df = ctx.read_csv("data.csv", datafusion::execution::options::CsvReadOptions::default()).await?;
let min_value = Param::new("min_value", ScalarValue::from(0.0));

let filtered = df.filter(col("value").gt(min_value.expr()))?;

Plot::<Cartesian>::new()
    .data(filtered)
    .add_param(min_value)  // Add parameter to plot
    .mark(Symbol::new().x(col("x")).y(col("y")));
# Ok(())
# }
```

### Color Encoding

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::scalar::ScalarValue;
# use palette::Srgba;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = datafusion::execution::context::SessionContext::new();
# let df = ctx.read_csv("data.csv", datafusion::execution::options::CsvReadOptions::default()).await?;
let threshold = Param::new("threshold", ScalarValue::from(50.0));
let status = when(col("value").gt(threshold.expr()), lit("above"))
    .otherwise(lit("below"))?;

Plot::<Cartesian>::new()
    .data(df)
    .add_param(threshold)  // Add parameter to plot
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .fill_with(status, |c| {
                c.scale_with::<Ordinal>(|s| s.range_colors(vec![
                    Srgba::new(0.121, 0.466, 0.705, 1.0),
                    Srgba::new(0.882, 0.470, 0.0, 1.0),
                ]))
                .legend(|l| l.title("Category"))
            })
    );
# Ok(())
# }
```

### Position Encoding

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::scalar::ScalarValue;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = datafusion::execution::context::SessionContext::new();
# let df = ctx.read_csv("data.csv", datafusion::execution::options::CsvReadOptions::default()).await?;
let offset = Param::new("offset", ScalarValue::from(10.0));

Plot::<Cartesian>::new()
    .data(df)
    .add_param(offset.clone())  // Add parameter to plot
    .mark(
        Symbol::new()
            .x(col("x") + offset.expr())
            .y(col("y"))
    );
# Ok(())
# }
```

## Binding Parameter Values

Parameters must be set on the `Plot` (as defaults) and then overridden at render time. This workflow allows you to compile once and render multiple times with different parameter values:

```rust,no_run
use avenger_chart::prelude::*;
use datafusion::prelude::*;
use datafusion::scalar::ScalarValue;
use indexmap::IndexMap;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let ctx = SessionContext::new();
let df = ctx
    .read_csv("data.csv", CsvReadOptions::default())
    .await?;

// Create parameter with default value
let threshold = Param::new("threshold", ScalarValue::from(50.0));

let filtered = df.filter(col("value").gt(threshold.expr()))?;

// Compile plot with default parameter
let compiled = Plot::<Cartesian>::new()
    .data(filtered)
    .add_param(threshold.clone())
    .mark(Symbol::new().x(col("x")).y(col("y")))
    .compile(&ctx)
    .await?;

// Render with default parameter value (50.0)
let _render1 = compiled.render(&ctx, None).await?;

// Render with overridden parameter value (75.0)
let mut params = IndexMap::new();
params.insert("threshold".to_string(), ScalarValue::from(75.0));
let _render2 = compiled.render(&ctx, Some(params)).await?;
# Ok(())
# }
```

The parameter is used during both compilation (for data filtering) and rendering (for final visualization). Overriding at render time is efficient for interactive applications.

## Interactive Parameter Controls

**Future feature**: Bind parameters to UI controls:

```rust,no_run
// Planned for future release
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::scalar::ScalarValue;
# /*
use avenger_chart::widgets::Slider;
# */

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = datafusion::execution::context::SessionContext::new();
# let df = ctx.read_csv("data.csv", datafusion::execution::options::CsvReadOptions::default()).await?;
let threshold = Param::new("threshold", ScalarValue::from(50.0));

# /*
let slider = Slider::new()
    .param(threshold.clone())
    .min(0.0)
    .max(100.0)
    .step(1.0);
# */

/*
Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .opacity_with(
                col("value").gt(threshold.expr()),
                |c| c.scale(|s| s.range_interval(lit(0.2), lit(1.0)))
            )
    )
    .widget(slider);
*/

let _plot = Plot::<Cartesian>::new().data(df);
# Ok(())
# }
```

## Parameter-Driven Filtering

Create dynamic filters with parameters. **Important**: Any parameter used in an expression must be added to the plot with `.add_param()`:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::scalar::ScalarValue;
# use indexmap::IndexMap;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = datafusion::execution::context::SessionContext::new();
# let df = ctx.read_csv("data.csv", datafusion::execution::options::CsvReadOptions::default()).await?;
// Create a parameter for the filter threshold
let min_value = Param::new("min_value", ScalarValue::from(50.0));

// Use the parameter in a filter expression
let filtered = df.filter(col("value").gt(min_value.expr()))?;

// Build the plot and add the parameter
let plot = Plot::<Cartesian>::new()
    .data(filtered)
    .add_param(min_value.clone())  // Must add param to plot
    .mark(Line::new().x(col("x")).y(col("value")));

// Compile the plot
let compiled = plot.compile(&ctx).await?;

// Render with default threshold (50.0)
let _render1 = compiled.render(&ctx, None).await?;

// Render with different threshold (75.0)
let mut params = IndexMap::new();
params.insert("min_value".to_string(), ScalarValue::from(75.0));
let _render2 = compiled.render(&ctx, Some(params)).await?;
# Ok(())
# }
```

## Multiple Parameters

Combine multiple parameters:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::scalar::ScalarValue;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = datafusion::execution::context::SessionContext::new();
# let df = ctx.read_csv("data.csv", datafusion::execution::options::CsvReadOptions::default()).await?;
let min_value = Param::new("min", ScalarValue::from(0.0));
let max_value = Param::new("max", ScalarValue::from(100.0));
let category = Param::new("category", ScalarValue::from("A"));

let filtered = df.filter(
    col("value")
        .gt_eq(min_value.expr())
        .and(col("value").lt_eq(max_value.expr()))
        .and(col("category").eq(category.expr()))
)?;

Plot::<Cartesian>::new()
    .data(filtered)
    .add_param(min_value)   // Add all parameters to plot
    .add_param(max_value)
    .add_param(category)
    .mark(Symbol::new().x(col("x")).y(col("y")));
# Ok(())
# }
```

## Parameter Expressions

Parameters can be used in complex expressions:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::scalar::ScalarValue;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = datafusion::execution::context::SessionContext::new();
# let df = ctx.read_csv("data.csv", datafusion::execution::options::CsvReadOptions::default()).await?;
let scale_factor = Param::new("scale", ScalarValue::from(1.0));
let offset = Param::new("offset", ScalarValue::from(0.0));

Plot::<Cartesian>::new()
    .data(df)
    .add_param(scale_factor.clone())  // Add all parameters to plot
    .add_param(offset.clone())
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y") * scale_factor.expr() + offset.expr())
    );
# Ok(())
# }
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
let threshold = Param::new("threshold", ScalarValue::from(50.0));

// If not bound, uses default value of 50.0
let filtered = df.filter(col("value").gt(threshold.expr()))?;

Plot::<Cartesian>::new()
    .data(filtered)
    .add_param(threshold)  // Add parameter to plot
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
let user_threshold = Param::new("user_threshold", ScalarValue::from(50.0));
let selected_year = Param::new("selected_year", ScalarValue::from(2023_i32));

// Less clear
let p1 = Param::new("p1", ScalarValue::from(50.0));
let x = Param::new("x", ScalarValue::from(2023_i32));
# }
```

Parameter names appear in error messages and debugging output.

## Complete Example

```rust,render,ignore
use avenger_chart::param::Param;
use avenger_chart::prelude::*;
use datafusion::prelude::*;
use datafusion::scalar::ScalarValue;
use palette::Srgba;

let ctx = SessionContext::new();
let stocks_path = format!(
    "{}/../tests/data/stocks.parquet",
    env!("CARGO_MANIFEST_DIR")
);
let df = ctx
    .read_parquet(stocks_path, ParquetReadOptions::default())
    .await
    .expect("load stocks dataset");

let aapl = df
    .filter(col("symbol").eq(lit("AAPL")))
    .expect("filter AAPL symbol");

let threshold = Param::new("price_threshold", ScalarValue::from(150.0));
let status = when(col("price").gt(threshold.expr()), lit("above"))
    .otherwise(lit("within"))
    .expect("build threshold status");

let plot = Plot::<Cartesian>::new()
    .data(aapl.clone())
    .canvas_size(800.0, 500.0)
    .title("AAPL Price Threshold")
    .add_param(threshold.clone())
    .mark(
        Symbol::new()
            .x_with(col("date"), |c| c.scale_with::<Time>(|s| s).axis(|a| a.title("Date")))
            .y_with(col("price"), |c| c.axis(|a| a.title("Price ($)")))
            .fill_with(status, |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Threshold Status"))
            })
    );

plot
```

## Controlling Guides, Layout, and Legends

Parameters are ordinary expressions, so you can reuse them in axis builders, layout APIs, or legend configuration:

- Toggle grid lines or axis visibility with a boolean parameter: `axis(|a| a.grid(&show_grid_param))`.
- Switch axis positions with a CASE expression:
  `position(when(pos.expr().eq(lit("top")), lit("top")).otherwise(lit("bottom")))`.
- Drive canvas dimensions for responsive renderings: `canvas_size(width_param.expr(), height_param.expr())`.
- Adjust legend backgrounds or titles at runtime by referencing parameters inside `.legend(|l| ...)`.

```rust,no_run
# use avenger_chart::prelude::*;
# use avenger_chart::param::Param;
# use datafusion::prelude::*;
# use datafusion::scalar::ScalarValue;
# use indexmap::IndexMap;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let ctx = SessionContext::new();
let df = ctx.read_csv("points.csv", CsvReadOptions::new()).await?;

let width = Param::new("width", ScalarValue::Float32(Some(420.0)));
let show_grid = Param::new("show_grid", ScalarValue::Boolean(Some(true)));

let plot = Plot::<Cartesian>::new()
    .add_param(width.clone())
    .add_param(show_grid.clone())
    .canvas_size(width.expr(), 320.0)
    .title("Responsive Scatter")
    .data(df)
    .mark(
        Symbol::new()
            .x_with(col("x"), |c| c.axis(|a| a.grid(&show_grid)))
            .y_with(col("y"), |c| c.axis(|a| a.grid(true)))
    );

let compiled = plot.compile(&ctx).await?;

let mut overrides = IndexMap::new();
overrides.insert("width".into(), ScalarValue::Float32(Some(640.0)));
overrides.insert("show_grid".into(), ScalarValue::Boolean(Some(false)));
compiled.render(&ctx, Some(overrides)).await?;
# Ok(())
# }
```

## Feeding Parameters into CSS Themes

When you add a parameter to a plot, the theme receives a `--param-name` CSS variable. This makes it straightforward to expose interactive palettes or responsive guide styling directly in CSS.

```rust,no_run
# use avenger_chart::prelude::*;
# use avenger_chart::param::Param;
# use avenger_chart::theme::Theme;
# use datafusion::scalar::ScalarValue;
# fn example() -> Result<(), String> {
let accent = Param::new("accent", ScalarValue::Utf8(Some("#2563eb".into())));

let mut theme = Theme::light();
theme.append_css(
    r#"
    :root { --accent: var(--param-accent); }
    mark[type="symbol"] {
        fill: var(--accent);
        stroke: color-mix(in srgb, var(--accent) 60%, black);
        stroke-width: 1.5px;
    }
    "#,
)?;

let _plot = Plot::<Cartesian>::new()
    .add_param(accent)
    .theme(theme)
    .mark(Symbol::new().x(col("x")).y(col("y")));
# Ok(())
# }
```

At render time you can override the `accent` parameter to swap the palette without recompiling the chart.

## Next Steps

- Learn about future [Controllers](../../docs/future-work/controllers.md)
