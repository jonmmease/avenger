# Working with DataFusion Expressions

Avenger Chart is built on [Apache DataFusion](https://datafusion.apache.org/), a fast, extensible query engine that uses [Apache Arrow](https://arrow.apache.org/) for efficient in-memory data processing. DataFusion provides the expression system that powers channel mappings, data transformations, and aggregations in Avenger Chart.

## What is DataFusion?

Apache DataFusion is an SQL query engine and DataFrame library that executes queries using a columnar in-memory format. When you write expressions like `col("temperature")` or `col("x") + col("y")` in Avenger Chart, you're using DataFusion's expression API.

**How Avenger Chart Uses DataFusion:**
- **Data Loading**: Read Parquet, CSV, and Arrow data via `SessionContext`
- **Expressions**: Map data columns to visual channels using `col()`, `lit()`, operators, and functions
- **Transformations**: Compute derived values, filter data, handle null values
- **Aggregations**: Group and summarize data with `sum()`, `avg()`, `count()`, etc.
- **User-Defined Functions**: Extend DataFusion with custom logic (advanced)
- **SessionContext**: Required for compiling and evaluating plots, supports serialization workflows

This guide focuses on **DataFusion's expression capabilities** within the Avenger Chart context. For comprehensive DataFusion documentation, see:
- [DataFusion User Guide](https://datafusion.apache.org/user-guide/introduction.html)
- [Expression API](https://datafusion.apache.org/user-guide/expressions.html)
- [Scalar Functions](https://datafusion.apache.org/user-guide/sql/scalar_functions.html)

## Core Expression Building Blocks

The foundation of DataFusion expressions in Avenger Chart:

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

For details on how these work with scaling, see [Understanding Expressions vs Literals](../channels.md#understanding-expressions-vs-literals).

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
let plot = Plot::<Cartesian>::new()
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

let plot = Plot::<Cartesian>::new()
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
let plot = Plot::<Cartesian>::new()
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

For using conditionals within channel encoding (to bypass scales conditionally), see [Conditional Encodings](../channels.md#conditional-encodings).

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
let plot = Plot::<Cartesian>::new()
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

## Aggregations

Aggregation functions summarize data by grouping. Avenger Chart automatically detects aggregate functions and applies GROUP BY logic.

**Common aggregate functions:**
- `sum()` - Total values
- `avg()` - Average (mean)
- `count()` - Count rows
- `min()` / `max()` - Minimum/maximum
- `median()` - Median value
- `stddev()` - Standard deviation

**Quick Example:**

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::functions_aggregate::expr_fn::sum;
# async fn example(ctx: &SessionContext, df: DataFrame) -> Result<(), Box<dyn std::error::Error>> {
let plot = Plot::<Cartesian>::new()
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

For comprehensive aggregation documentation, see:
- [Aggregations Guide](./aggregations.md)
- [DataFusion Aggregate Functions](https://datafusion.apache.org/user-guide/sql/aggregate_functions.html)

## Finding More Functions

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

### Documentation Resources

- **Rust API Docs**: [docs.rs/datafusion](https://docs.rs/datafusion/latest/datafusion/) - Complete API reference
- **Scalar Functions**: [DataFusion Scalar Functions](https://datafusion.apache.org/user-guide/sql/scalar_functions.html) - All built-in functions
- **Aggregate Functions**: [DataFusion Aggregate Functions](https://datafusion.apache.org/user-guide/sql/aggregate_functions.html)
- **Window Functions**: [DataFusion Window Functions](https://datafusion.apache.org/user-guide/sql/window_functions.html) - Advanced analytical queries

### Advanced Features

DataFusion also supports:
- **Window Functions** - Analytical computations over partitions (e.g., `row_number()`, `rank()`, `lag()`, `lead()`)
- **User-Defined Functions (UDFs)** - Create custom scalar or aggregate functions (see below)
- **User-Defined Aggregate Functions (UDAFs)** - Custom aggregations

For window functions and UDAFs, see the [DataFusion Library User Guide](https://datafusion.apache.org/library-user-guide/index.html).

## The SessionContext

The `SessionContext` is DataFusion's execution environment and is required for both `compile()` and `evaluate()` operations in Avenger Chart.

### Why SessionContext Matters

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let df = SessionContext::new().sql("SELECT 1").await?;
let ctx = SessionContext::new();

// Compile requires SessionContext to:
// - Infer data types from DataFrames
// - Validate expressions
// - Optimize logical plans
let compiled = plot.compile(&ctx).await?;

// Evaluate requires SessionContext to:
// - Execute DataFrame queries
// - Compute scale domains from data
// - Run aggregations
let evaluated = compiled.evaluate(&ctx, None).await?;
# Ok(())
# }
```

### SessionContext Independence

**Important**: The `SessionContext` passed to `compile()` can be *different* from the one passed to `evaluate()`. This enables powerful scenarios:

- **Server-side compilation**: Compile once on a server, send `CompiledPlot` to clients
- **Caching**: Serialize `CompiledPlot` to disk, load and evaluate later
- **Distributed rendering**: Compile in one process, render in another

However, if your plot uses **User-Defined Functions (UDFs)**, the same UDFs must be registered in both contexts.

## User-Defined Functions (Advanced)

User-Defined Functions (UDFs) let you extend DataFusion with custom logic. For `CompiledPlot` to be serializable, UDFs must implement specific traits.

### When to Use UDFs

- Custom mathematical transformations not in DataFusion
- Domain-specific calculations
- Integration with external libraries
- Proprietary business logic

### UDFs and Serialization

**Important**: User-defined UDFs are **not serialized** with `CompiledPlot`. Instead:
- UDFs are referenced by **name** in serialized expressions
- The UDF must be **registered** in the SessionContext used for evaluation
- The implementation is looked up from the context's registry, not embedded in the serialization

This means your UDF struct **does not need** to derive `Serialize + Deserialize`. Here's a complete example:

```rust,render
use avenger_chart::plot::CompiledPlot;
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Array, ArrayRef, AsArray, Float64Array};
use datafusion::arrow::datatypes::DataType;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::common::ScalarValue;
use datafusion::logical_expr::{
    ColumnarValue, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, TypeSignature,
    Volatility,
};
use datafusion::prelude::*;
use std::sync::Arc;

// Define a UDF that doubles Float64 values
// Note: Does NOT need Serialize/Deserialize - registered by name
#[derive(Debug, Clone)]
struct DoubleUDF {
    cached_signature: std::sync::OnceLock<Signature>,
}

impl DoubleUDF {
    fn new() -> Self {
        Self {
            cached_signature: std::sync::OnceLock::new(),
        }
    }

    fn get_signature(&self) -> &Signature {
        self.cached_signature.get_or_init(|| {
            Signature::new(
                TypeSignature::Exact(vec![DataType::Float64]),
                Volatility::Immutable,
            )
        })
    }
}

impl ScalarUDFImpl for DoubleUDF {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn name(&self) -> &str {
        "double"
    }

    fn signature(&self) -> &Signature {
        self.get_signature()
    }

    fn return_type(&self, _arg_types: &[DataType]) -> datafusion::error::Result<DataType> {
        Ok(DataType::Float64)
    }

    fn invoke_with_args(
        &self,
        args: ScalarFunctionArgs,
    ) -> datafusion::error::Result<ColumnarValue> {
        match &args.args[0] {
            ColumnarValue::Array(array) => {
                let float_array = array.as_primitive::<datafusion::arrow::datatypes::Float64Type>();
                let doubled: Float64Array = float_array
                    .iter()
                    .map(|opt| opt.map(|v| v * 2.0))
                    .collect();
                Ok(ColumnarValue::Array(Arc::new(doubled) as ArrayRef))
            }
            ColumnarValue::Scalar(scalar) => {
                if let ScalarValue::Float64(Some(v)) = scalar {
                    Ok(ColumnarValue::Scalar(ScalarValue::Float64(
                        Some(v * 2.0),
                    )))
                } else {
                    Err(datafusion::error::DataFusionError::Execution(
                        "Expected Float64 scalar".to_string(),
                    ))
                }
            }
        }
    }
}

// Create sample data
let ctx1 = SessionContext::new();

let batch = RecordBatch::try_from_iter(vec![
    (
        "x",
        Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "y",
        Arc::new(Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 6.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])?;

let df = ctx1.read_batch(batch)?;

// Register the UDF with ctx1
let double_udf = ScalarUDF::new_from_impl(DoubleUDF::new());
ctx1.register_udf(double_udf.clone());

// Use the UDF in a plot - double the y values
let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(double_udf.call(vec![col("y")]))  // Use UDF to double y values
            .size(200.0)
            .fill("coral")
    )
    .title("UDF Example: Doubled Y Values");

// Compile with ctx1
let compiled = plot.compile(&ctx1).await?;

// === Serialization Workflow ===
// This simulates sending the compiled plot to a different process/server

// 1. Serialize the compiled plot with bincode
let serialized = bincode::serialize(&compiled)?;
println!("Serialized {} bytes", serialized.len());

// 2. Deserialize into a new CompiledPlot
let deserialized: CompiledPlot = bincode::deserialize(&serialized)?;

// 3. Create a DIFFERENT SessionContext (simulating a different process)
let ctx2 = SessionContext::new();

// 4. IMPORTANT: Register the same UDF with ctx2!
//    The UDF implementation is NOT in the serialized data.
//    It must be registered by name in the new context.
ctx2.register_udf(double_udf);

// 5. Evaluate the deserialized plot with ctx2 (different from ctx1!)
let evaluated = deserialized.evaluate(&ctx2, None).await?;

Ok(evaluated)
```

### Key Points

1. **Not Serialized**: User UDFs are NOT embedded in `CompiledPlot` serialization
2. **Name-Based Lookup**: UDFs are referenced by **name** in serialized expressions
3. **Register in Both Contexts**: Same UDF must be registered in compile context AND evaluate context
4. **Implementation Must Match**: The UDF registered in both contexts must have the same name and behavior
5. **No Derive Required**: UDF structs do NOT need `Serialize` or `Deserialize` traits

### Why This Pattern Works

When you serialize a `CompiledPlot` that uses a UDF:
- The UDF's **name** is stored (e.g., `"double"`)
- The UDF's **implementation** is NOT serialized
- On deserialization, the UDF is looked up by name from the new context's registry

This means:
- ✅ Serialization is compact and portable
- ✅ UDF logic stays secure (not embedded in serialized data)
- ⚠️ Both contexts must have UDFs with matching names and implementations

## Common Patterns

### Pattern: Data Transformation

Compute derived columns for visualization:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn example(ctx: &SessionContext, df: DataFrame) -> Result<(), Box<dyn std::error::Error>> {
// Visualize velocity (distance / time)
let plot = Plot::<Cartesian>::new()
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
let plot = Plot::<Cartesian>::new()
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
let plot = Plot::<Cartesian>::new()
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
let plot = Plot::<Cartesian>::new()
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

## See Also

- [Channels](../channels.md) - How expressions are used in channel encodings
- [Understanding Expressions vs Literals](../channels.md#understanding-expressions-vs-literals) - When values are scaled
- [Aggregations](./aggregations.md) - Working with grouped data
- [Parameters](../themes/parameters.md) - Dynamic expressions with runtime values
- [DataFusion Documentation](https://datafusion.apache.org/) - Official DataFusion docs
