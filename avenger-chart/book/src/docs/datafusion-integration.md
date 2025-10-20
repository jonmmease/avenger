# DataFusion Integration

Avenger Chart is built on [Apache DataFusion](https://datafusion.apache.org/), a fast, extensible query engine that uses [Apache Arrow](https://arrow.apache.org/) for efficient in-memory data processing. This guide explains how DataFusion works within Avenger Chart and how to leverage advanced features.

For expression syntax used in channel encoding, see [Channels > Expressions](./channels/expressions.md).

## What is DataFusion?

Apache DataFusion is an SQL query engine and DataFrame library that executes queries using a columnar in-memory format. When you write expressions like `col("temperature")` or `col("x") + col("y")` in Avenger Chart, you're using DataFusion's expression API.

**How Avenger Chart Uses DataFusion:**
- **Data Loading**: Read Parquet, CSV, and Arrow data via `SessionContext`
- **Expressions**: Map data columns to visual channels using `col()`, `lit()`, operators, and functions
- **Transformations**: Compute derived values, filter data, handle null values
- **Aggregations**: Group and summarize data with `sum()`, `avg()`, `count()`, etc.
- **User-Defined Functions**: Extend DataFusion with custom logic (advanced)
- **SessionContext**: Required for compiling and evaluating plots, supports serialization workflows

This guide focuses on **DataFusion integration and advanced features** within the Avenger Chart context. For comprehensive DataFusion documentation, see:
- [DataFusion User Guide](https://datafusion.apache.org/user-guide/introduction.html)
- [Expression API](https://datafusion.apache.org/user-guide/expressions.html)
- [Scalar Functions](https://datafusion.apache.org/user-guide/sql/scalar_functions.html)

## The SessionContext

The `SessionContext` is DataFusion's execution environment and is required for both `compile()` and `evaluate()` operations in Avenger Chart.

### Why SessionContext Matters

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let plot = Plot::<Cartesian>::new();
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

## Data Loading

DataFusion's `SessionContext` provides methods for loading data from various sources.

### Reading DataFrames

```rust,no_run
# use datafusion::prelude::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let ctx = SessionContext::new();

// From Parquet files
let df = ctx.read_parquet("data/sales.parquet", Default::default()).await?;

// From CSV files
let df = ctx.read_csv("data/sales.csv", Default::default()).await?;

// From Arrow RecordBatch
# use datafusion::arrow::record_batch::RecordBatch;
# use datafusion::arrow::array::{Float64Array, StringArray};
# use std::sync::Arc;
let batch = RecordBatch::try_from_iter(vec![
    ("x", Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])) as _),
    ("y", Arc::new(StringArray::from(vec!["a", "b", "c"])) as _),
])?;
let df = ctx.read_batch(batch)?;
# Ok(())
# }
```

### Using SQL Queries

DataFusion supports SQL for data transformations:

```rust,no_run
# use datafusion::prelude::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let ctx = SessionContext::new();

// Register a DataFrame as a table
# let df = ctx.sql("SELECT 1").await?;
ctx.register_table("sales", df.into_view())?;

// Query with SQL
let filtered = ctx
    .sql("SELECT * FROM sales WHERE amount > 100")
    .await?;
# Ok(())
# }
```

## Finding and Importing Functions

DataFusion provides hundreds of functions for data transformation.

### Importing Functions

Functions are available through the `datafusion::functions::expr_fn` module:

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

## See Also

- [Channels > Expressions](./channels/expressions.md) - Using expressions in channel encoding
- [Channels > Aggregations](./channels/aggregations.md) - Working with grouped data
- [Parameters](./parameters.md) - Dynamic expressions with runtime values
- [DataFusion Documentation](https://datafusion.apache.org/) - Official DataFusion docs
- [DataFusion User Guide](https://datafusion.apache.org/user-guide/introduction.html) - Comprehensive guide
- [DataFusion API Docs](https://docs.rs/datafusion/latest/datafusion/) - Rust API reference
