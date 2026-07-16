# Axes

Axes provide reference lines and labels that help readers interpret positions in your visualization. Avenger Chart automatically generates axes for positional channels (x, y) and provides extensive customization options.

**Note**: Axes are for **position channels** (x, y in Cartesian; r, theta in Polar) and are part of the coordinate system's guide. For visual channels (fill, size, shape, etc.), see **[Legends](./guides-axes-legends/legends.md)**. To understand how axes, guides, and legends relate, see **[Guides, Axes, and Legends](./guides-axes-legends/index.md)**.

## Basic Axis Configuration

Configure axes through the channel builder's `.axis()` method:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();

let df = ctx
    .read_parquet(avenger_sample_data::iris_path(), ParquetReadOptions::default())
    .await
    ?;

let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x_with(col("sepal_length"), |c| {
                c.axis(|a| a.title("Sepal Length (cm)").grid(true))
            })
            .y_with(col("sepal_width"), |c| {
                c.axis(|a| a.title("Sepal Width (cm)").grid(true))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Axis Properties

### Titles

Add descriptive titles to axes:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();

let df = ctx
    .read_parquet(avenger_sample_data::iris_path(), ParquetReadOptions::default())
    .await
    ?;

let plot = Chart::<Cartesian>::new()
    .data(df)
    .title("Iris Dataset Analysis")
    .mark(
        Symbol::new()
            .x_with(col("sepal_length"), |c| {
                c.axis(|a| a.title("Length (cm)"))
            })
            .y_with(col("sepal_width"), |c| {
                c.axis(|a| a.title("Width (cm)"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

**Default behavior**: If no title is provided, Avenger Chart uses the column name.

### Grid Lines

Enable or disable grid lines for better readability:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();

let df = ctx
    .read_parquet(avenger_sample_data::iris_path(), ParquetReadOptions::default())
    .await
    ?;

let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x_with(col("sepal_length"), |c| {
                c.axis(|a| a.title("Sepal Length").grid(false))
            })
            .y_with(col("sepal_width"), |c| {
                c.axis(|a| a.title("Sepal Width").grid(true))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Grid lines help readers trace values back to the axis, especially useful for dense or precise visualizations.

### Number Formatting

Format axis labels using D3-style format strings:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float32Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    ("category", Arc::new(StringArray::from(vec!["A", "B", "C", "D"])) as _),
    ("value", Arc::new(Float32Array::from(vec![1250000.0, 2780000.0, 950000.0, 3120000.0])) as _),
])?;
let df = ctx.read_batch(batch)?;

let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x(col("category"))
            .x2_with(col(":x"), |c| c.band(1.0))
            .y_with(lit(0.0), |c| {
                c.axis(|a| a.format(".2s").title("Value"))
            })
            .y2(col("value"))
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

**Common format specifiers**:
- `.2f` - 2 decimal places (e.g., 3.14)
- `.2s` - SI prefix (e.g., 1.5K, 2.3M)
- `.0%` - percentage (e.g., 45%)
- `$,.2f` - currency with thousands separator (e.g., $1,234.56)

See [d3-format documentation](https://d3js.org/d3-format) for complete format specification.

### Visibility

Hide axes when they're not needed:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();

let df = ctx
    .read_parquet(avenger_sample_data::iris_path(), ParquetReadOptions::default())
    .await
    ?;

let plot = Chart::<Cartesian>::new()
    .data(df)
    .title("Minimal Axes")
    .mark(
        Symbol::new()
            .x_with(col("sepal_length"), |c| {
                c.axis(|a| a.visible(false))
            })
            .y_with(col("sepal_width"), |c| {
                c.axis(|a| a.title("Width").grid(true))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Advanced: Axis Positioning

By default, the x-axis appears at the bottom and y-axis on the left. You can reposition axes using `.position()`:

### Right Y-Axis

Position the y-axis on the right side:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    ("category", Arc::new(StringArray::from(vec!["A", "B", "C", "A", "B", "C", "A", "B", "C"])) as _),
    ("x", Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0])) as _),
    ("y", Arc::new(Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 7.0, 6.0, 8.0, 10.0, 9.0])) as _),
])?;
let df = ctx.read_batch(batch)?;

let plot = Chart::<Cartesian>::new()
    .data(df)
    .title("Right Y-Axis")
    .mark(
        Symbol::new()
            .x_with(col("x"), |c| {
                c.scale(|s| s.domain((0.0, 10.0)))
                    .axis(|a| a.title("X"))
            })
            .y_with(col("y"), |c| {
                c.scale(|s| s.domain((0.0, 12.0)))
                    .axis(|a| a.position(AxisPosition::Right).title("Y Right").grid(true))
            })
            .size(100.0)
            .fill_with(lit("#2ca25f"), |c| c.no_scale())
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Top X-Axis

Position the x-axis at the top:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    ("category", Arc::new(StringArray::from(vec!["A", "B", "C", "A", "B", "C", "A", "B", "C"])) as _),
    ("x", Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0])) as _),
    ("y", Arc::new(Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 7.0, 6.0, 8.0, 10.0, 9.0])) as _),
])?;
let df = ctx.read_batch(batch)?;

let plot = Chart::<Cartesian>::new()
    .data(df)
    .title("Top X-Axis and Right Y-Axis")
    .mark(
        Symbol::new()
            .x_with(col("x"), |c| {
                c.scale(|s| s.domain((0.0, 10.0)))
                    .axis(|a| a.position(AxisPosition::Top).title("X Top").grid(true))
            })
            .y_with(col("y"), |c| {
                c.scale(|s| s.domain((0.0, 12.0)))
                    .axis(|a| a.position(AxisPosition::Right).title("Y Right").grid(true))
            })
            .size(100.0)
            .fill_with(lit("#2ca25f"), |c| c.no_scale())
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

**Available positions**:
- `AxisPosition::Bottom` - X-axis at bottom (default)
- `AxisPosition::Top` - X-axis at top
- `AxisPosition::Left` - Y-axis on left (default)
- `AxisPosition::Right` - Y-axis on right

**When to use**:
- ✅ Dual y-axes (though not yet supported - use right axis for secondary data)
- ✅ Unconventional layouts for specific design requirements
- ✅ Better label placement when data is concentrated on one side

**Layout interaction**: Axis position affects plot area size. Right axes and top axes reserve space just like their default counterparts.

## Advanced: Expression-Based Titles

Axis titles can use DataFusion expressions to dynamically compute text based on parameters. This example shows unit conversion where both the axis title and the data values change based on the unit parameter:

```rust,render
use avenger_chart::prelude::*;
use avenger_chart::param::Param;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::common::ScalarValue;
use datafusion::logical_expr::when;
use datafusion::prelude::*;
use std::sync::Arc;
use indexmap::IndexMap;

let ctx = SessionContext::new();
// Data stored in meters
let batch = RecordBatch::try_from_iter(vec![
    ("x_meters", Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0])) as _),
    ("y", Arc::new(Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 4.5])) as _),
])?;
let df = ctx.read_batch(batch)?;

// Parameter to control unit display
let unit_param = Param::new("unit", "meters");

// CASE expression to convert meters to feet (1m = 3.28084ft)
let x_expr = when(unit_param.expr().eq(lit("feet")), col("x_meters") * lit(3.28084))
    .otherwise(col("x_meters"))
    .unwrap();

// CASE expression for dynamic axis title
let x_axis_title = when(unit_param.expr().eq(lit("meters")), lit("Distance (m)"))
    .when(unit_param.expr().eq(lit("feet")), lit("Distance (ft)"))
    .otherwise(lit("Distance"))
    .unwrap();

let plot = Chart::<Cartesian>::new()
    .canvas_size(400.0, 300.0)
    .title("Dynamic Unit Conversion")
    .data(df)
    .param(unit_param.clone())
    .mark(
        Symbol::new()
            .x_with(x_expr, |c| c.axis(|a| a.grid(true).title(x_axis_title)))
            .y_with(col("y"), |c| c.axis(|a| a.grid(true).title("Value")))
    );

// Compile once
let compiled = plot.compile(&ctx).await?;

// Evaluate with unit="meters" - shows values like 1, 2, 3, 4, 5
let mut params_meters = IndexMap::new();
params_meters.insert("unit".to_string(), ScalarValue::Utf8(Some("meters".to_string())));
let evaluated_meters = compiled.evaluate(&ctx, Some(params_meters)).await?;

// Evaluate with unit="feet" - shows values like 3.28, 6.56, 9.84, 13.12, 16.40
let mut params_feet = IndexMap::new();
params_feet.insert("unit".to_string(), ScalarValue::Utf8(Some("feet".to_string())));
let evaluated_feet = compiled.evaluate(&ctx, Some(params_feet)).await?;

Ok((evaluated_meters, evaluated_feet))
```

**How it works**:
- Axis titles accept any DataFusion `Expr`, not just string literals
- Use `when().otherwise()` for CASE expressions
- Reference parameters with `param.expr()`
- Titles are evaluated at render time with current parameter values

**When to use**:
- ✅ Internationalization (language selection via parameter)
- ✅ Unit conversion displays (metric/imperial)
- ✅ Context-dependent labels based on data ranges
- ✅ Interactive dashboards with user-selected options

See the [Parameters guide](../themes/parameters.md) for more on parametric plots and the [DataFusion expressions guide](./datafusion-expressions.md) for building complex expressions.

## Advanced: Conditional Configuration

Axis properties can be conditional based on parameters, allowing responsive and interactive designs:

### Conditional Grid Visibility

Toggle grid lines based on a parameter:

```rust,render
use avenger_chart::prelude::*;
use avenger_chart::param::Param;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::common::ScalarValue;
use datafusion::prelude::*;
use std::sync::Arc;
use indexmap::IndexMap;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    ("x", Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0])) as _),
    ("y", Arc::new(Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 4.5])) as _),
])?;
let df = ctx.read_batch(batch)?;

// Parameter to control grid visibility
let show_grid_param = Param::new("show_grid", true);

let plot = Chart::<Cartesian>::new()
    .canvas_size(400.0, 300.0)
    .data(df)
    .param(show_grid_param.clone())
    .mark(
        Symbol::new()
            .x_with(col("x"), |c| {
                c.axis(|a| a.grid(&show_grid_param).title("X Axis"))
            })
            .y_with(col("y"), |c| c.axis(|a| a.grid(true).title("Y Axis")))
    );

// Compile once
let compiled = plot.compile(&ctx).await?;

// Evaluate with grid enabled
let mut params_enabled = IndexMap::new();
params_enabled.insert("show_grid".to_string(), ScalarValue::Boolean(Some(true)));
let evaluated_enabled = compiled.evaluate(&ctx, Some(params_enabled)).await?;

// Evaluate with grid disabled
let mut params_disabled = IndexMap::new();
params_disabled.insert("show_grid".to_string(), ScalarValue::Boolean(Some(false)));
let evaluated_disabled = compiled.evaluate(&ctx, Some(params_disabled)).await?;

Ok((evaluated_enabled, evaluated_disabled))
```

### Conditional Axis Position

Change axis position dynamically:

```rust,render
use avenger_chart::prelude::*;
use avenger_chart::param::Param;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::common::ScalarValue;
use datafusion::logical_expr::when;
use datafusion::prelude::*;
use std::sync::Arc;
use indexmap::IndexMap;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    ("x", Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0])) as _),
    ("y", Arc::new(Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 4.5])) as _),
])?;
let df = ctx.read_batch(batch)?;

// Parameter to control axis position
let axis_pos_param = Param::new("axis_pos", "bottom");

// CASE expression for position
let position_expr = when(axis_pos_param.expr().eq(lit("top")), lit("top"))
    .otherwise(lit("bottom"))
    .unwrap();

let plot = Chart::<Cartesian>::new()
    .canvas_size(400.0, 300.0)
    .data(df)
    .param(axis_pos_param.clone())
    .mark(
        Symbol::new()
            .x_with(col("x"), |c| {
                c.axis(|a| a.position(position_expr).grid(true).title("X Axis"))
            })
            .y_with(col("y"), |c| c.axis(|a| a.grid(true).title("Y Axis")))
    );

// Compile once
let compiled = plot.compile(&ctx).await?;

// Evaluate with axis at bottom
let mut params_bottom = IndexMap::new();
params_bottom.insert("axis_pos".to_string(), ScalarValue::Utf8(Some("bottom".to_string())));
let evaluated_bottom = compiled.evaluate(&ctx, Some(params_bottom)).await?;

// Evaluate with axis at top
let mut params_top = IndexMap::new();
params_top.insert("axis_pos".to_string(), ScalarValue::Utf8(Some("top".to_string())));
let evaluated_top = compiled.evaluate(&ctx, Some(params_top)).await?;

Ok((evaluated_bottom, evaluated_top))
```

### Conditional Axis Visibility

Show or hide axes based on parameters:

```rust,render
use avenger_chart::prelude::*;
use avenger_chart::param::Param;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::common::ScalarValue;
use datafusion::prelude::*;
use std::sync::Arc;
use indexmap::IndexMap;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    ("x", Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0])) as _),
    ("y", Arc::new(Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 4.5])) as _),
])?;
let df = ctx.read_batch(batch)?;

// Parameter to control axis visibility
let show_axis_param = Param::new("show_x_axis", true);

let plot = Chart::<Cartesian>::new()
    .canvas_size(400.0, 300.0)
    .data(df)
    .param(show_axis_param.clone())
    .mark(
        Symbol::new()
            .x_with(col("x"), |c| {
                c.axis(|a| a.visible(&show_axis_param).grid(true).title("X Axis"))
            })
            .y_with(col("y"), |c| c.axis(|a| a.grid(true).title("Y Axis")))
    );

// Compile once
let compiled = plot.compile(&ctx).await?;

// Evaluate with axis visible
let mut params_visible = IndexMap::new();
params_visible.insert("show_x_axis".to_string(), ScalarValue::Boolean(Some(true)));
let evaluated_visible = compiled.evaluate(&ctx, Some(params_visible)).await?;

// Evaluate with axis hidden
let mut params_hidden = IndexMap::new();
params_hidden.insert("show_x_axis".to_string(), ScalarValue::Boolean(Some(false)));
let evaluated_hidden = compiled.evaluate(&ctx, Some(params_hidden)).await?;

Ok((evaluated_visible, evaluated_hidden))
```

**Conditional configuration patterns**:
- **Boolean parameters**: Pass `Param` directly to methods like `.grid(&param)` or `.visible(&param)`
- **String/enum parameters**: Use CASE expressions with `.position(expr)` or `.title(expr)`
- **Compile once, evaluate many**: Create one compiled plot, render with different parameter values

**When to use**:
- ✅ Responsive designs that adapt to screen size
- ✅ User preference toggles (show/hide grids, change units)
- ✅ Context-dependent layouts based on data characteristics
- ✅ Interactive dashboards with configurable views

See the [Parameters guide](../themes/parameters.md) for comprehensive parameter documentation.

## Next Steps

- Learn about [themes and styling](../themes.md) to customize axis appearance
- Explore [CSS themes](../themes/css-themes.md) for advanced axis styling
- See [parameters](../themes/parameters.md) for building interactive, parametric visualizations
- Check [DataFusion expressions](./datafusion-expressions.md) for building complex conditional logic
