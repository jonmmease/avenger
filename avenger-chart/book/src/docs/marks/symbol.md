# Symbol Mark

`Symbol` marks render discrete points, making them useful for scatter plots and dot plots. Symbol marks are generic over coordinate systems, supporting both Cartesian and Polar coordinates.

## Cartesian Coordinates

In Cartesian coordinates, symbols are positioned using `x` and `y` channels:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create GDP vs life expectancy data
let batch = RecordBatch::try_from_iter(vec![
    (
        "gdp",
        Arc::new(Float64Array::from(vec![5000.0, 15000.0, 25000.0, 35000.0, 45000.0, 55000.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "life_expectancy",
        Arc::new(Float64Array::from(vec![55.0, 65.0, 72.0, 75.0, 78.0, 80.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("gdp"))
            .y(col("life_expectancy"))
            .size(300.0)
            .fill("#4682b4")
            .shape("square")
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Position Channels (Cartesian)

- **`x`** – Horizontal position
- **`y`** – Vertical position

## Polar Coordinates

In Polar coordinates, symbols are positioned using radial distance and angle:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create polar data - circular pattern
let batch = RecordBatch::try_from_iter(vec![
    (
        "radius",
        Arc::new(Float64Array::from(vec![20.0, 35.0, 50.0, 65.0, 80.0, 95.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "angle",
        Arc::new(Float64Array::from(vec![
            0.0, 1.047, 2.094, 3.142, 4.189, 5.236
        ]))  // 0, π/3, 2π/3, π, 4π/3, 5π/3 radians
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Plot::<Polar>::new()
    .data(df)
    .mark(
        Symbol::<Polar>::new()
            .r(col("radius"))
            .theta(col("angle"))
            .size(400.0)
            .fill("#e74c3c")
            .shape("diamond")
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Position Channels (Polar)

- **`r`** – Radial distance from origin
- **`theta`** – Angular position in radians (0 = east, π/2 = north, π = west, 3π/2 = south)

## Visual Encoding Channels

These channels work across all coordinate systems:

- **`size`** – Area of the marker in square pixels
- **`fill`** – Fill color (supports hex colors, CSS colors, expressions)
- **`stroke`** – Outline color
- **`stroke_width`** – Outline width in pixels
- **`shape`** – Symbol shape (see [Available Shapes](#available-shapes) below)
- **`angle`** – Rotation in radians (useful with non-circular shapes)

**Note**: For transparency, use RGBA hex colors (e.g., `"#4682b480"`) in the `fill` or `stroke` channels.

## Available Shapes

Symbol marks support various shapes from the Vega symbol library:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();
// Create data for different shapes
let batch = RecordBatch::try_from_iter(vec![
    (
        "x",
        Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 1.0, 2.0, 3.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "y",
        Arc::new(Float64Array::from(vec![2.0, 2.0, 2.0, 1.0, 1.0, 1.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "shape",
        Arc::new(StringArray::from(vec!["circle", "square", "triangle-up", "star", "diamond", "cross"]))
            as datafusion::arrow::array::ArrayRef,
    ),
])?;

let df = ctx.read_batch(batch)?;


let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .size(500.0)
            .fill("#9b59b6")
            .shape_with(col("shape"), |c| {
                c.legend(|l| l.title("Shape"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Supported shape names include:
- `"circle"` (default)
- `"square"`
- `"cross"`
- `"diamond"`
- `"triangle-up"`, `"triangle-down"`, `"triangle-left"`, `"triangle-right"`
- `"arrow"` (pointing up)
- `"wedge"`
- `"triangle"`
- `"star"`
- `"wye"`
- And more Vega-compatible symbol names

## Size

The `size` channel controls the area of each symbol in square pixels.

### Fixed Size

Set a constant size for all symbols:

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
            .size(200.0)  // Size in square pixels
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Encoded Size

Map a data variable to size to create bubble charts:

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
            .size_with(col("petal_length"), |c| {
                c.scale(|s| s.range_interval(lit(80.0), lit(300.0)))
                    .legend(|l| l.title("Petal Length"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Fill and Stroke

### Fixed Colors

Set constant fill and stroke colors:

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
            .size(120.0)
            .fill("#ff6b6b")      // Coral red
            .stroke("#2c3e50")    // Dark slate
            .stroke_width(2.5)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Encoded Fill

Map categorical variables to color using scales:

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
            .fill_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Species"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Transparency

Use RGBA hex colors for transparency to show overlapping data:

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
            .size(250.0)
            .fill("#4682b480")  // hex RGBA with transparency
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Multiple Encodings

Combine multiple visual encodings in a single plot:

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
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Automatic Visual Padding

Linear scales automatically expand their domains to prevent symbol clipping at data boundaries. The padding calculation considers symbol size and stroke width to ensure marks are fully visible even at domain edges.

For complete details on how automatic padding works, how it interacts with `nice()` scales, and scale type support, see [Domain Inference > Automatic Visual Padding](../scales/domains.md#automatic-visual-padding)

## Angle

The `angle` channel rotates symbols in radians. This is particularly useful with non-circular shapes:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();

// Create data with rotation angles
let batch = RecordBatch::try_from_iter(vec![
    ("x", Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0])) as _),
    ("y", Arc::new(Float64Array::from(vec![2.0, 2.0, 2.0, 2.0, 2.0])) as _),
    ("angle", Arc::new(Float64Array::from(vec![0.0, 0.785, 1.571, 2.356, 3.142])) as _),  // 0°, 45°, 90°, 135°, 180°
])?;
let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .size(400.0)
            .fill("#e74c3c")
            .shape("triangle-up")
            .angle(col("angle"))
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Using with Different Scales

### Logarithmic Scales

Use log scales for skewed data spanning multiple orders of magnitude:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let batch = RecordBatch::try_from_iter(vec![
    (
        "x",
        Arc::new(Float64Array::from(vec![1.0, 10.0, 100.0, 1000.0, 10000.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "y",
        Arc::new(Float64Array::from(vec![2.0, 5.0, 8.0, 12.0, 15.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
.expect("create batch");

let df = ctx.read_batch(batch).expect("read batch");

Plot::<Cartesian>::new()
    .data(df)
    .title("Log Scale Example")
    .mark(
        Symbol::new()
            .x_with(col("x"), |c| {
                c.scale_with::<Log>(|s| s.base(10.0))
                    .axis(|a| a.title("X (log scale)"))
            })
            .y_with(col("y"), |c| c.axis(|a| a.title("Y (linear)")))
            .size(150.0)
    )
```

## Next Steps

- See the [Marks Overview](./index.md) for coordinate system support and layering
- Browse the [Scatter Patterns recipes](../patterns/common-plot-patterns.md#scatter-patterns) for ready-to-run examples
- Learn about [Cartesian](../coordinate-systems/cartesian.md) and [Polar](../coordinate-systems/polar.md) coordinates
- Understand [Channels](../channels/index.md) for data binding
