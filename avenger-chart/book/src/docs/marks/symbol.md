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
- **`opacity`** – Transparency (0.0 to 1.0)

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

## Next Steps

- See the [Marks Overview](./index.md) for coordinate system support and layering
- Explore [Scatter Plots Guide](../../guides/scatter-plots.md) for more examples
- Learn about [Cartesian](../coordinate-systems/cartesian.md) and [Polar](../coordinate-systems/polar.md) coordinates
- Understand [Channels](../channels.md) for data binding
