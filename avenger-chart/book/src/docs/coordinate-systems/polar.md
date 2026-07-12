# Polar Coordinates

The Polar coordinate system uses radius and angle to position marks, making it ideal for radial scatter plots, circular distributions, wind roses, and radar-style visualizations.

## Basic Example

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
        Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "angle",
        Arc::new(Float64Array::from(vec![
            0.0, 0.785, 1.571, 2.356, 3.142, 3.927, 4.712, 5.498
        ]))  // 0, π/4, π/2, 3π/4, π, 5π/4, 3π/2, 7π/4 radians
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Chart::<Polar>::new()
    .data(df)
    .mark(
        Symbol::<Polar>::new()
            .r(col("radius"))
            .theta(col("angle"))
            .size(200.0)
            .fill("#e74c3c")
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Position Channels

Polar coordinates provide two fundamental position channels:

- **`r`** – Radial distance from origin (how far from the center)
- **`theta`** – Angular position in radians (rotation around the circle)

Angular values are measured in radians:
- 0 radians = 0° (right/east)
- π/2 radians = 90° (top/north)
- π radians = 180° (left/west)
- 3π/2 radians = 270° (bottom/south)

## Declaration

Create a Polar plot using the type parameter:

```rust
let plot = Chart::<Polar>::new()
    .data(df)
    .mark(Symbol::<Polar>::new().r(col("distance")).theta(col("direction")));
```

When marks are constructed inside `.mark()`, they can inherit the plot's coordinate system. However, for polar coordinates, it's often clearer to use the explicit form:

```rust
let mark = Symbol::<Polar>::new()
    .r(col("distance"))
    .theta(col("direction"));
```

## Supported Scale Types

Polar coordinates work with the same scale types as Cartesian, but applied to radial and angular dimensions:

### For Radius (`r` channel)
- **[Linear](../scales/linear.md)** – Default for numeric radial data
- **[Log](../scales/log.md)** – Logarithmic radial scaling
- **[Sqrt](../scales/sqrt.md)** – Square root radial scaling
- Any other quantitative scale type

### For Angle (`theta` channel)
- **[Linear](../scales/linear.md)** – Default for numeric angular data
- **[Band](../scales/band.md)** – For categorical angular divisions (sectors)
- **[Point](../scales/point.md)** – For categorical points around the circle

## Common Usage Patterns

### Radial Scatter Plot

Plot data points at various radii and angles:

```rust
Plot::<Polar>::new()
    .data(df)
    .mark(
        Symbol::<Polar>::new()
            .r(col("distance"))
            .theta(col("angle"))
            .size(150.0)
            .fill("#9b59b6")
    )
```

### Wind Rose Pattern

Use categorical angles with quantitative radius:

```rust
Plot::<Polar>::new()
    .data(df)
    .mark(
        Symbol::<Polar>::new()
            .r(col("wind_speed"))
            .theta_with(col("direction"), |c| {
                c.scale_with::<Band>(|s| s)
            })
            .fill_with(col("direction"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Direction"))
            })
    )
```

### Circular Distribution

Visualize periodic data or cyclical patterns:

```rust
Plot::<Polar>::new()
    .data(df)
    .mark(
        Symbol::<Polar>::new()
            .r(col("hour_of_day"))  // 0-24 hours
            .theta(col("day_of_week"))  // Circular week
            .size_with(col("activity_count"), |c| {
                c.scale(|s| s.range_interval(lit(50.0), lit(500.0)))
            })
    )
```

## Converting Between Cartesian and Polar

Data can be converted between Cartesian (x, y) and Polar (r, theta) representations using DataFusion expressions:

### Cartesian to Polar
```rust
// r = sqrt(x² + y²)
// theta = atan2(y, x)
let df = df.with_column("r", sqrt(col("x").pow(lit(2)) + col("y").pow(lit(2))))?
    .with_column("theta", atan2(col("y"), col("x")))?;
```

### Polar to Cartesian
```rust
// x = r * cos(theta)
// y = r * sin(theta)
let df = df.with_column("x", col("r") * cos(col("theta")))?
    .with_column("y", col("r") * sin(col("theta")))?;
```

See the [Working with DataFusion](../../guides/datafusion-expressions.md) guide for more expression examples.

## Layering in Polar Coordinates

Multiple marks can be layered in the same polar plot:

```rust
Plot::<Polar>::new()
    .data(df.clone())
    .mark(
        Line::<Polar>::new()
            .r(col("radius"))
            .theta(col("angle"))
            .stroke("#95a5a6")
            .stroke_width(1.5)
    )
    .mark(
        Symbol::<Polar>::new()
            .r(col("radius"))
            .theta(col("angle"))
            .size(150.0)
            .fill("#e74c3c")
    )
```

## Planned Extensions

Future releases will include additional polar-specific mark types:

- **Arc marks** – For pie charts and radial segments
- **Radial bar marks** – For radial bar charts
- **Radar/spider charts** – Multi-axis radial plots

See the [roadmap](../../roadmap.md) for details.

## Comparison with Cartesian

The same `Symbol` mark works in both coordinate systems by changing position channels:

| Aspect | Cartesian | Polar |
|--------|-----------|-------|
| Position channels | `x`, `y` | `r`, `theta` |
| Origin | Bottom-left corner | Center point |
| Distance measure | Euclidean (straight line) | Radial + angular |
| Typical uses | Standard plots | Cyclical/directional data |

## Next Steps

- Learn about [Cartesian](./cartesian.md) coordinates for standard rectangular plots
- Explore [Marks](../marks/index.md) that work in polar coordinates
- Review [Scales](../scales/index.md) for data transformations
- See the [Working with DataFusion](../../guides/datafusion-expressions.md) guide for coordinate conversions
