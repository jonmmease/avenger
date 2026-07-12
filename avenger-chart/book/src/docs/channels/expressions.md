# Expressions

Avenger Chart uses [Apache DataFusion](https://datafusion.apache.org/) expressions to map data values to visual channels. This guide covers the expression syntax you'll use to encode data in channel methods like `.x()`, `.y()`, `.fill()`, and `.size()`.

For understanding DataFusion as a system and advanced integration features, see [DataFusion Integration](../datafusion-integration.md).

## Core Expression Building Blocks

The foundation of expressions in Avenger Chart:

### Column and Literal Expressions

```rust,no_run
# use datafusion::prelude::*;
// Reference a column
col("temperature")

// Create a literal value
lit(100.0)
lit("red")
lit(true)
```

For details on how these work with scaling, see [Understanding Expressions vs Literals](./index.md#understanding-expressions-vs-literals).

### Operators

DataFusion supports standard operators that can be chained:

```rust,no_run
# use datafusion::prelude::*;
# fn example() {
// Arithmetic
col("revenue") - col("cost")               // Subtraction
col("price") * lit(1.08)                   // Multiplication with 8% markup
(col("x") + col("y")) / lit(2.0)           // Average of two columns

// Comparison
col("temperature").gt(lit(30.0))           // Greater than
col("status").eq(lit("active"))            // Equality
col("value").between(lit(0.0), lit(100.0)) // Range check

// Logical
col("age").gt(lit(18)).and(col("verified").eq(lit(true)))  // AND
col("status").eq(lit("error")).or(col("status").eq(lit("warning")))  // OR
col("enabled").eq(lit(true)).not()         // NOT
# }
```

**Example: Computed Channel**

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn example(ctx: &SessionContext, df: DataFrame) -> Result<(), Box<dyn std::error::Error>> {
// Map profit margin (computed from revenue and cost) to color
let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("date"))
            .y(col("revenue"))
            .fill_with((col("revenue") - col("cost")) / col("revenue"), |c| {
                c.scale_with::<Linear>(|s| s)
                    .legend(|l| l.title("Profit Margin"))
            })
    );
# Ok(())
# }
```

Learn more: [DataFusion Expression API](https://datafusion.apache.org/user-guide/expressions.html)

## Scalar Functions by Category

DataFusion provides hundreds of scalar functions organized into categories. You can use any of these in channel expressions.

### Function Categories

| Category | Description | Example Functions | Import From |
|----------|-------------|-------------------|-------------|
| **Math** | Numerical computations | `sqrt()`, `abs()`, `round()`, `ceil()`, `floor()` | `datafusion::functions::expr_fn` |
| **String** | Text manipulation | `concat()`, `lower()`, `upper()`, `substring()`, `trim()` | `datafusion::functions::expr_fn` |
| **Date/Time** | Temporal operations | `date_part()`, `date_trunc()`, `now()`, `current_date()` | `datafusion::functions::expr_fn` |
| **Conditional** | Logic and null handling | `coalesce()`, `nullif()`, `greatest()`, `least()` | `datafusion::functions::expr_fn` |
| **Regex** | Pattern matching | `regexp_like()`, `regexp_replace()`, `regexp_match()` | `datafusion::functions::expr_fn` |
| **Array** | List operations | `array_length()`, `array_concat()`, `array_contains()` | `datafusion::functions::expr_fn` |
| **Hashing** | Cryptographic functions | `md5()`, `sha256()`, `digest()` | `datafusion::functions::expr_fn` |

For the complete list, see: [DataFusion Scalar Functions](https://datafusion.apache.org/user-guide/sql/scalar_functions.html)

### Example: Math Functions

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::functions::expr_fn::sqrt;
use datafusion::prelude::*;
use std::sync::Arc;

let ctx = SessionContext::new();

// Create points in a spiral pattern around origin
let n = 30;
let mut x_vals = Vec::with_capacity(n);
let mut y_vals = Vec::with_capacity(n);

for i in 0..n {
    let t = (i as f64) * 0.8;  // Angle parameter
    let r = t * 0.5;            // Radius grows with angle
    x_vals.push(r * t.cos());
    y_vals.push(r * t.sin());
}

let batch = RecordBatch::try_from_iter(vec![
    (
        "x",
        Arc::new(Float64Array::from(x_vals)) as datafusion::arrow::array::ArrayRef,
    ),
    (
        "y",
        Arc::new(Float64Array::from(y_vals)) as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;

// Use sqrt() to compute Euclidean distance, map to size
let distance = sqrt(col("x") * col("x") + col("y") * col("y"));

let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .size_with(distance, |c| {
                c.scale(|s| s.range_interval(lit(50.0), lit(500.0)))
                    .legend(|l| l.title("Distance from Origin"))
            })
            .fill("steelblue")
    )
    .title("Math Functions: Spiral with Distance-Based Sizing");

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Example: String Functions

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::functions::expr_fn::lower;
use datafusion::functions_aggregate::expr_fn::sum;
use datafusion::prelude::*;
use std::sync::Arc;

let ctx = SessionContext::new();

// Create data with inconsistent category capitalization
let batch = RecordBatch::try_from_iter(vec![
    (
        "category",
        Arc::new(StringArray::from(vec!["Widget", "GADGET", "widget", "Gadget", "WIDGET", "gadget"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "sales",
        Arc::new(Float64Array::from(vec![100.0, 150.0, 120.0, 180.0, 90.0, 200.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;

// Normalize category names to lowercase for consistent grouping
let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x(lower(col("category")))  // Convert "Widget", "WIDGET", "widget" to "widget"
            .x2_with(lower(col("category")), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(sum(col("sales")))
            .fill("#e67e22")
    )
    .title("String Functions: Case-Insensitive Grouping");

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Importing Functions

DataFusion functions are available through the `datafusion::functions::expr_fn` module:

```rust,no_run
// Scalar functions (math, string, date/time, conditional, etc.)
use datafusion::functions::expr_fn::{sqrt, abs, round, lower, upper, coalesce, date_part};

// Or import all commonly used functions
use datafusion::functions::expr_fn::*;

// Aggregate functions are in a separate module
use datafusion::functions_aggregate::expr_fn::{sum, avg, count};
```

## Conditional Logic

### The `when().otherwise()` Pattern

DataFusion's `when()` function creates conditional expressions (similar to SQL CASE):

```rust,no_run
# use datafusion::prelude::*;
# fn example() {
// Create a conditional expression
let status = when(col("temperature").gt(lit(30.0)), lit("hot"))
    .when(col("temperature").lt(lit(10.0)), lit("cold"))
    .otherwise(lit("moderate"))
    .unwrap();

// Use in channel encoding
// .fill_with(status, |c| c.scale_with::<Ordinal>(|s| s))
# }
```

For using conditionals within channel encoding (to bypass scales conditionally), see [Conditional Encodings](./index.md#conditional-encodings).

### Null Handling with `coalesce()`

```rust,no_run
# use datafusion::prelude::*;
# use datafusion::functions::expr_fn::coalesce;
# fn example() {
// Provide fallback values for null data
let safe_value = coalesce(vec![col("optional_field"), lit(0.0)]);

// Use in channel
// .y(safe_value)
# }
```

**Example: Null-Safe Visualization**

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, Int32Array};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::functions::expr_fn::coalesce;
use datafusion::prelude::*;
use std::sync::Arc;

let ctx = SessionContext::new();

// Create data with some missing (null) temperature values
let batch = RecordBatch::try_from_iter(vec![
    (
        "day",
        Arc::new(Int32Array::from(vec![1, 2, 3, 4, 5, 6, 7, 8]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "temperature",
        Arc::new(Float64Array::from(vec![
            Some(20.0),
            None,          // Missing data
            Some(22.0),
            Some(21.0),
            None,          // Missing data
            Some(23.0),
            Some(24.0),
            None,          // Missing data
        ]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;

// Handle missing temperature data by using 0.0 as fallback
let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("day"))
            .y(coalesce(vec![col("temperature"), lit(0.0)]))
            .size(200.0)
            .fill("orangered")
    )
    .title("Null Handling: coalesce() Falls Back to 0.0");

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Common Patterns

### Pattern: Data Transformation

Compute derived columns for visualization:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn example(ctx: &SessionContext, df: DataFrame) -> Result<(), Box<dyn std::error::Error>> {
// Visualize velocity (distance / time)
let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("time"))
            .y(col("distance") / col("time"))  // Computed velocity
            .size(150.0)
    );
# Ok(())
# }
```

### Pattern: Null Safety

Ensure visualizations handle missing data gracefully:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::functions::expr_fn::coalesce;
# async fn example(ctx: &SessionContext, df: DataFrame) -> Result<(), Box<dyn std::error::Error>> {
// Use coalesce to provide defaults for null values
let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(coalesce(vec![col("measurement"), lit(0.0)]))  // Use 0.0 if null
            .size(150.0)
    );
# Ok(())
# }
```

### Pattern: Case-Insensitive Text Processing

Normalize text for consistent encoding:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::functions::expr_fn::lower;
# async fn example(ctx: &SessionContext, df: DataFrame) -> Result<(), Box<dyn std::error::Error>> {
// Group by normalized category names
let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x(lower(col("product_name")))
            .x2_with(lower(col("product_name")), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("quantity"))
    );
# Ok(())
# }
```

### Pattern: Date Component Extraction

Extract parts of dates for temporal grouping:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::functions::expr_fn::date_part;
# async fn example(ctx: &SessionContext, df: DataFrame) -> Result<(), Box<dyn std::error::Error>> {
// Group sales by month
let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x(date_part(lit("month"), col("sale_date")))
            .x2_with(date_part(lit("month"), col("sale_date")), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("amount"))
    );
# Ok(())
# }
```

## Aggregations

When you need to summarize data by grouping, use aggregate functions like `sum()`, `avg()`, `count()`, `min()`, and `max()`. Avenger Chart automatically detects aggregate functions and applies GROUP BY logic.

**Quick Example:**

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::functions_aggregate::expr_fn::sum;
# async fn example(ctx: &SessionContext, df: DataFrame) -> Result<(), Box<dyn std::error::Error>> {
let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x(col("category"))                    // GROUP BY category
            .x2_with(col("category"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(sum(col("sales")))                 // SUM(sales)
    );
# Ok(())
# }
```

For comprehensive aggregation documentation, see [Aggregations](./aggregations.md).

## See Also

- [Channels Overview](./index.md) - Understanding channel encoding
- [Understanding Expressions vs Literals](./index.md#understanding-expressions-vs-literals) - When values are scaled
- [Aggregations](./aggregations.md) - Working with grouped data
- [DataFusion Integration](../datafusion-integration.md) - SessionContext, UDFs, and advanced features
- [DataFusion Scalar Functions](https://datafusion.apache.org/user-guide/sql/scalar_functions.html) - Complete function reference
- [DataFusion Expression API](https://datafusion.apache.org/user-guide/expressions.html) - Official DataFusion docs
