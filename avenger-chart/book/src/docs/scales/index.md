# Scales

Scales are functions that transform data values from **data space** (the domain) into **visual space** (the range). They control how quantitative values map to positions, sizes, colors, and other visual properties.

Avenger Chart provides [12 scale types](./scale-types.md) organized into five categories. See the [Scale Types Reference](./scale-types.md) for a complete comparison and decision guide.

## Quick Example

```rust
// Map data values (0-100) to pixel positions (0-400)
.x_with(col("value"), |c| {
    c.scale_with::<Linear>(|s| s.domain((lit(0.0), lit(100.0))))
})

// Map categories to colors
.fill_with(col("category"), |c| {
    c.scale_with::<Ordinal>(|s| s)
})
```

## Common Patterns

### Continuous Color Gradient

```rust
.fill_with(col("temperature"), |c| {
    c.scale_with::<Linear>(|s| s)
        .range_colors(vec!["#blue", "#white", "#red"])
})
```

### Categorical Colors

```rust
.fill_with(col("species"), |c| {
    c.scale_with::<Ordinal>(|s| s)
        .range_colors(vec!["#ff0000", "#00ff00", "#0000ff"])
})
```

### Log-scaled Axis

```rust
.y_with(col("population"), |c| {
    c.scale_with::<Log>(|s| s.base(10.0).nice(true))
})
```

### Custom Thresholds

```rust
.fill_with(col("score"), |c| {
    c.scale_with::<Threshold>(|s| {
        s.domain_discrete(vec![lit(60.0), lit(80.0), lit(90.0)])
            .range_discrete(vec!["#F", "#D", "#C", "#B", "#A"])
    })
})
```

## Domain Specification

The domain defines the range of input data values a scale maps from. Avenger Chart provides multiple ways to specify domains depending on your needs.

### Literal Domains (Static Values)

For fixed, known domain boundaries, use primitive tuples:

```rust,no_run
use avenger_chart::prelude::*;

// Simple numeric tuple - most common for continuous scales
.x_with(col("value"), |c| {
    c.scale(|s| s.domain((0.0, 100.0)))
})

// Works with f32 or f64
.y_with(col("value"), |c| {
    c.scale(|s| s.domain((0.0_f32, 100.0_f32)))
})
```

**When to use**: Fixed scales where boundaries are predetermined and won't change.

### Expression Domains (Dynamic Values)

For dynamic boundaries computed at runtime, use DataFusion expressions wrapped in `lit()`:

```rust,no_run
use avenger_chart::prelude::*;
use avenger_chart::param::Param;
use datafusion::common::ScalarValue;

// Using lit() to wrap literals as expressions
.x_with(col("value"), |c| {
    c.scale(|s| s.domain((lit(0.0), lit(100.0))))
})

// Using parameters for interactive plots
let min_param = Param::new("domain_min", ScalarValue::Float64(Some(0.0)));
let max_param = Param::new("domain_max", ScalarValue::Float64(Some(100.0)));

.x_with(col("value"), |c| {
    c.scale(|s| s.domain((min_param.expr(), max_param.expr())))
})

// Using arithmetic expressions
.x_with(col("value"), |c| {
    c.scale(|s| s.domain((lit(0.0), lit(100.0) * lit(1.5))))
})
```

**When to use**:
- Parametric plots where domains change based on user input
- Computed domain boundaries using arithmetic
- When you need to compose with other expressions

**Note**: You cannot use column aggregates like `col("value").min()` directly in domain specifications. For data-driven domains, use automatic domain inference instead.

### Automatic (Data-Driven) Domains

If you don't specify a domain, Avenger Chart automatically infers it from your data:

```rust,no_run
use avenger_chart::prelude::*;

// Domain automatically computed from data
.x_with(col("x_value"), |c| c.scale(|s| s))
```

**How it works**:
- Continuous scales: Computes min/max from data values
- Categorical scales: Extracts unique values from data
- Includes automatic padding for symbols/lines on Linear scales

**When to use**: Most common case - let the library handle domain inference.

### Comparison Example

Here's the same scatter plot with three domain approaches:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();

// Create data: points from 10 to 90
let batch = RecordBatch::try_from_iter(vec![
    ("x", Arc::new(Float64Array::from(vec![10.0, 30.0, 50.0, 70.0, 90.0])) as _),
    ("y", Arc::new(Float64Array::from(vec![15.0, 45.0, 25.0, 65.0, 85.0])) as _),
])?;
let df = ctx.read_batch(batch)?;

// Literal domain: Fixed 0-100 range
let plot = Plot::<Cartesian>::new()
    .data(df)
    .title("Literal Domain (0-100)")
    .mark(
        Symbol::new()
            .x_with(col("x"), |c| {
                c.scale_with::<Linear>(|s| s.domain((0.0, 100.0)).nice(false))
            })
            .y_with(col("y"), |c| {
                c.scale_with::<Linear>(|s| s.domain((0.0, 100.0)).nice(false))
            })
            .size(150.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Notice how the literal domain `(0.0, 100.0)` creates a plot with symbols positioned in the middle, since the data ranges from 10-90 within the 0-100 domain.

Compare with automatic domain inference:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();

// Same data: points from 10 to 90
let batch = RecordBatch::try_from_iter(vec![
    ("x", Arc::new(Float64Array::from(vec![10.0, 30.0, 50.0, 70.0, 90.0])) as _),
    ("y", Arc::new(Float64Array::from(vec![15.0, 45.0, 25.0, 65.0, 85.0])) as _),
])?;
let df = ctx.read_batch(batch)?;

// Automatic domain: Inferred from data with padding
let plot = Plot::<Cartesian>::new()
    .data(df)
    .title("Automatic Domain (Data-Driven)")
    .mark(
        Symbol::new()
            .x_with(col("x"), |c| {
                c.scale_with::<Linear>(|s| s.nice(false))  // No domain specified
            })
            .y_with(col("y"), |c| {
                c.scale_with::<Linear>(|s| s.nice(false))  // No domain specified
            })
            .size(150.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

With automatic domain inference, the symbols fill the plot area because the domain is computed from the data (approximately 10-90) plus automatic padding for the symbol size.

### Special Methods

For convenience, Avenger Chart provides specialized domain methods:

**Interval domains** (continuous scales):
```rust,no_run
use avenger_chart::prelude::*;

// Equivalent to .domain((lit(0.0), lit(100.0)))
.x_with(col("value"), |c| {
    c.scale(|s| s.domain_interval(lit(0.0), lit(100.0)))
})
```

**Discrete domains** (categorical scales):
```rust,no_run
use avenger_chart::prelude::*;

// For ordinal or threshold scales
.fill_with(col("category"), |c| {
    c.scale_with::<Ordinal>(|s| {
        s.domain_discrete(vec![lit("A"), lit("B"), lit("C")])
    })
})
```

**Data-driven domains** (advanced):
```rust,no_run
use avenger_chart::prelude::*;
use std::sync::Arc;

// Explicitly specify data source for domain inference (rarely needed)
.x_with(col("value"), |c| {
    c.scale(|s| s.domain_data(Arc::new(df.clone()), col("column_name")))
})
```

### Choosing the Right Approach

**Use literal domains `(0.0, 100.0)` when**:
- ✅ Domain boundaries are fixed and known
- ✅ You want consistent scales across multiple plots
- ✅ The values never change
- ✅ Simpler, more readable code

**Use expression domains `(lit(0.0), lit(100.0))` when**:
- ✅ Using parameters for interactive plots
- ✅ Boundaries are computed using arithmetic expressions
- ✅ Need to compose with other expressions
- ✅ Domain boundaries change dynamically at runtime

**Use automatic domains (no `.domain()` call) when**:
- ✅ Domain should adapt to your data (most common)
- ✅ You want automatic padding for symbols/lines
- ✅ Building exploratory visualizations
- ✅ Data range is unknown or variable

## Configuration Methods

Most scales support these common configuration methods:

- **`domain(...)`** - Set the input data range (see Domain Specification above)
- **`range(...)`** - Set the output visual range
- **`nice(true)`** - Extend domain to nice round values
- **`zero(true)`** - Include zero in the domain
- **`clamp(true)`** - Clamp out-of-range values

See individual scale pages for scale-specific options.

## See Also

- [Scale Types](./scale-types.md) - Complete reference and comparison of all 12 scale types
- [Domain Inference](./domain-inference.md) - How domains are automatically inferred from data, including `nice()`, `zero()`, and automatic visual padding
- [Channels](../channels.md) - How to apply scales to visual channels
- [Understanding Expressions vs Literals](../channels.md#understanding-expressions-vs-literals) - When values are scaled vs bypass scaling
- [Legends](../guides-axes-legends/legends.md) - Automatically generated scale legends
- [Axes](../guides-axes-legends/axes.md) - Scale-aware coordinate axes
