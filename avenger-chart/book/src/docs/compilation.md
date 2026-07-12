# The Compilation Pipeline

Avenger-Chart uses a two-stage workflow: **compile** then **evaluate**. Understanding this architecture is essential for efficient chart creation, especially when working with parameters or rendering multiple variations.

## Why Two Stages?

The separation allows you to:
- **Compile once** - Expensive optimizations happen only once
- **Evaluate many times** - Render with different data or parameters cheaply
- **Use parameters** - Dynamic values without recompilation

This architecture is similar to how databases prepare and execute queries, or how compilers separate parse/optimize from execution.

## The Three Types

Avenger-Chart's workflow involves three distinct types:

1. **`Plot`** - The specification (not serializable)
   - Builder pattern for defining the chart
   - Contains DataFusion DataFrames and expressions
   - Represents "what to visualize"

2. **`CompiledPlot`** - The optimized plan (serializable)
   - Result of `plot.compile()`
   - Contains query plans and scale configurations
   - Independent of actual data
   - Can be reused with different parameter values

3. **`EvaluatedPlot`** - The visual output (serializable)
   - Result of `compiled.evaluate()`
   - Contains the scene graph (marks, axes, legends)
   - Independent of rendering backend
   - Ready for PNG, SVG, or interactive display

## The Pipeline

```
┌─────────────────────────────────────────────────────────────┐
│  Plot (not serializable)                                    │
│  .mark(...).data(...).param(...)                        │
└────────────┬────────────────────────────────────────────────┘
             │
             ▼
    ┌────────────────┐     Expensive (do once)
    │   COMPILE      │     - Parse expressions
    │                │     - Infer scales
    │   plot.compile │     - Build query plan
    │                │     - Optimize AST
    └────────┬───────┘     - NO data access (currently)
             │
             ▼
┌────────────────────────────────────────────────────────────┐
│  CompiledPlot (serializable)                               │
│  Ready for execution with different params/data            │
└────────────┬───────────────────────────────────────────────┘
             │
             ▼
    ┌────────────────┐     Cheap (do many times)
    │   EVALUATE     │     - Execute queries
    │                │     - Apply parameters
    │   .evaluate()  │     - Compute scales
    │                │     - Build scene graph
    └────────┬───────┘
             │
             ▼
┌────────────────────────────────────────────────────────────┐
│  EvaluatedPlot (serializable, backend-independent)         │
│  Scene graph ready for any renderer (PNG, SVG, etc.)       │
└────────────────────────────────────────────────────────────┘
```

## What Happens at Each Stage

### Compile Stage

The `compile()` method performs these operations:

1. **Expression Analysis**
   - Parse DataFusion expressions (`col("x")`, `lit(0)`, etc.)
   - Build abstract syntax tree (AST)
   - Type inference and validation

2. **Scale Inference**
   - Detect data types from expressions
   - Determine appropriate scale types (Linear, Band, etc.)
   - Configure scale domains (if not explicitly set)

3. **Query Planning**
   - Build DataFusion logical plans
   - Set up aggregations and transformations
   - Optimize query execution

4. **Parameter Binding**
   - Register parameter placeholders
   - Prepare for runtime value substitution

**Key Point**: Currently, compilation happens **without accessing the actual data** - it only analyzes the structure, types, and query logic. This makes compilation fast and data-independent.

**Future Enhancement**: Compilation may be extended to support pre-evaluating data transformations that don't depend on parameter values (such as filtering and aggregation). This would allow even more work to be done once at compile time, further improving evaluation performance.

### Evaluate Stage

The `evaluate()` method performs these operations:

1. **Parameter Application**
   - Substitute parameter values into expressions
   - Update any parameter-dependent logic

2. **Data Execution**
   - Execute DataFusion queries
   - Load and transform data
   - Compute aggregations

3. **Scale Computation**
   - Calculate scale domains from data (if data-driven)
   - Compute tick values and formatting
   - Build color palettes

4. **Scene Graph Construction**
   - Map data through scales
   - Create visual marks (symbols, lines, etc.)
   - Apply styling and layout
   - Generate axes and legends

**Key Point**: Evaluation requires data access and produces the final visual scene.

## Performance Implications

### Single Render

For a single chart, the two stages are simple:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn example(ctx: &SessionContext, df: DataFrame) -> Result<(), Box<dyn std::error::Error>> {
let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(Symbol::new().x(col("x")).y(col("y")));

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
# Ok(())
# }
```

Typical timing (approximate):
- Compile: 1-5ms
- Evaluate: 5-50ms (depends on data size)

### Multiple Renders with Parameters

The real benefit appears when rendering multiple times:

```rust,no_run
# use avenger_chart::prelude::*;
# use avenger_chart::param::Param;
# use datafusion::prelude::*;
# use datafusion::scalar::ScalarValue;
# use indexmap::indexmap;
# async fn example(ctx: &SessionContext, df: DataFrame, threshold: Param) -> Result<(), Box<dyn std::error::Error>> {
// Compile once
let plot = Chart::<Cartesian>::new()
    .data(df)
    .param(threshold)
    .mark(Symbol::new().x(col("x")).y(col("y")));

let compiled = plot.compile(&ctx).await?;  // 5ms

// Evaluate many times with different parameter values
for value in [20.0, 40.0, 60.0, 80.0] {
    let params = indexmap!{
        "threshold".to_string() => ScalarValue::from(value)
    };
    let evaluated = compiled.evaluate(&ctx, Some(params)).await?;  // 10ms each

    // Render to PNG, display in UI, etc.
}

// Total: 5ms + (4 × 10ms) = 45ms
// vs recompiling each time: 4 × (5ms + 10ms) = 60ms
# Ok(())
# }
```

The savings multiply with more variations!

## When to Recompile

You **must** recompile when:
- ✅ Changing mark types (Symbol → Rect)
- ✅ Adding/removing marks
- ✅ Changing channel mappings (`.x(col("a"))` → `.x(col("b"))`)
- ✅ Modifying scale types (Linear → Log)
- ✅ Changing data schema

You **do not** need to recompile when:
- ❌ Changing parameter values
- ❌ Updating data values (same schema)
- ❌ Changing render settings (size, DPI)

## Practical Examples

### Example 1: Interactive Dashboard

```rust,no_run
# use avenger_chart::prelude::*;
# use avenger_chart::param::Param;
# use datafusion::prelude::*;
# use datafusion::scalar::ScalarValue;
# use indexmap::indexmap;
# async fn example(ctx: &SessionContext, df: DataFrame) -> Result<(), Box<dyn std::error::Error>> {
# fn get_user_selected_color() -> String { String::new() }
# struct Renderer;
# impl Renderer { fn display(&self, _: &avenger_chart::plot::EvaluatedPlot) {} }
# let renderer = Renderer;
// Setup - compile once
let color_param = Param::new("highlight_color", ScalarValue::Utf8(Some("#ff0000".into())));

let plot = Chart::<Cartesian>::new()
    .data(df)
    .param(color_param.clone())
    .mark(Symbol::new().x(col("x")).y(col("y")).fill(color_param.expr()));

let compiled = plot.compile(&ctx).await?;

// User interaction loop - evaluate many times
loop {
    let user_color = get_user_selected_color();  // From UI

    let params = indexmap!{
        "highlight_color".to_string() => ScalarValue::Utf8(Some(user_color))
    };

    let evaluated = compiled.evaluate(&ctx, Some(params)).await?;
    renderer.display(&evaluated);  // Update display
#   break; // Exit loop for example
}
# Ok(())
# }
```

### Example 2: Data Animation

```rust,no_run
# use avenger_chart::prelude::*;
# use avenger_chart::param::Param;
# use datafusion::prelude::*;
# use datafusion::scalar::ScalarValue;
# use indexmap::indexmap;
# async fn example(ctx: &SessionContext, df: DataFrame) -> Result<(), Box<dyn std::error::Error>> {
# fn save_frame(_frame: i32, _evaluated: &avenger_chart::plot::EvaluatedPlot) {}
// Compile once with time parameter
let time_param = Param::new("current_time", ScalarValue::from(0.0));

let plot = Chart::<Cartesian>::new()
    .data(df)
    .param(time_param.clone())
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            // Only show data up to current_time
            .defined(col("timestamp").lt_eq(time_param.expr()))
    );

let compiled = plot.compile(&ctx).await?;

// Animation loop - evaluate at each frame
for frame in 0..100 {
    let time = frame as f64 * 0.1;

    let params = indexmap!{
        "current_time".to_string() => ScalarValue::from(time)
    };

    let evaluated = compiled.evaluate(&ctx, Some(params)).await?;
    save_frame(frame, &evaluated);
}
# Ok(())
# }
```

### Example 3: Batch Export

```rust,no_run
# use avenger_chart::prelude::*;
# use avenger_chart::param::Param;
# use avenger_chart::render::WgpuRenderer;
# use datafusion::prelude::*;
# use datafusion::scalar::ScalarValue;
# use indexmap::indexmap;
# async fn example(ctx: &SessionContext, df: DataFrame) -> Result<(), Box<dyn std::error::Error>> {
# let renderer = WgpuRenderer::new();
// Generate charts for multiple regions
let region_param = Param::new("selected_region", ScalarValue::Utf8(Some("North".into())));

let plot = Chart::<Cartesian>::new()
    .data(df)
    .param(region_param.clone())
    .mark(
        Rect::new()
            // Filter data by region parameter
            .defined(col("region").eq(region_param.expr()))
            .x(col("month"))
            .y(col("sales"))
    );

let compiled = plot.compile(&ctx).await?;

// Export one chart per region
for region in ["North", "South", "East", "West"] {
    let params = indexmap!{
        "selected_region".to_string() => ScalarValue::Utf8(Some(region.into()))
    };

    let evaluated = compiled.evaluate(&ctx, Some(params)).await?;
    renderer.write_png(&compiled, &ctx, Some(params), &format!("{}_sales.png", region)).await?;
}
# Ok(())
# }
```

## Common Patterns

### Pattern: Compile Once, Render Once

```rust,no_run
# use avenger_chart::prelude::*;
# use avenger_chart::render::WgpuRenderer;
# use datafusion::prelude::*;
# async fn example(ctx: &SessionContext, df: DataFrame) -> Result<(), Box<dyn std::error::Error>> {
# let renderer = WgpuRenderer::new();
// Simple case - no reuse needed
let plot = Chart::<Cartesian>::new().data(df).mark(Symbol::new().x(col("x")).y(col("y")));
let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
renderer.write_png(&compiled, &ctx, None, "chart.png").await?;
# Ok(())
# }
```

### Pattern: Compile Once, Evaluate Many

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::scalar::ScalarValue;
# use indexmap::IndexMap;
# async fn example(ctx: &SessionContext, plot: Plot<Cartesian>, values: Vec<f64>) -> Result<(), Box<dyn std::error::Error>> {
# fn params_for(_value: f64) -> IndexMap<String, ScalarValue> { IndexMap::new() }
// With parameters - reuse compilation
let compiled = plot.compile(&ctx).await?;

for param_value in values {
    let evaluated = compiled.evaluate(&ctx, Some(params_for(param_value))).await?;
    // Render/save/display
}
# Ok(())
# }
```

### Pattern: Different Data, Same Structure

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn example(ctx: &SessionContext, datasets: Vec<DataFrame>) -> Result<(), Box<dyn std::error::Error>> {
// Same schema, different data - recompile needed
for dataset in datasets {
    let plot = Chart::<Cartesian>::new()
        .data(dataset)  // Different data each time
        .mark(Symbol::new().x(col("x")).y(col("y")));

    let compiled = plot.compile(&ctx).await?;
    let evaluated = compiled.evaluate(&ctx, None).await?;
    // Render
}
# Ok(())
# }
```

## Serialization

Both `CompiledPlot` and `EvaluatedPlot` are fully serializable, enabling powerful workflows like caching, server-side compilation, and distributed rendering.

### Serializing CompiledPlot

`CompiledPlot` can be serialized to JSON or binary formats:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn example(ctx: &SessionContext, df: DataFrame) -> Result<(), Box<dyn std::error::Error>> {
// Create and compile a plot
let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(Symbol::new().x(col("x")).y(col("y")));

let compiled = plot.compile(&ctx).await?;

// Serialize to JSON
let json = serde_json::to_string(&compiled)?;

// Or serialize to compact binary format (bincode)
let bytes = bincode::serialize(&compiled)?;

// Later: deserialize and evaluate
let deserialized: CompiledPlot = serde_json::from_str(&json)?;
let evaluated = deserialized.evaluate(&ctx, None).await?;
# Ok(())
# }
```

### Use Cases

**Caching**: Save `CompiledPlot` to disk to avoid recompilation on restart:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use std::fs;
# async fn example(ctx: &SessionContext, plot: Plot<Cartesian>) -> Result<(), Box<dyn std::error::Error>> {
// Compile and cache
let compiled = plot.compile(&ctx).await?;
fs::write("plot.json", serde_json::to_string(&compiled)?)?;

// Later: load from cache
let cached_json = fs::read_to_string("plot.json")?;
let compiled: CompiledPlot = serde_json::from_str(&cached_json)?;
let evaluated = compiled.evaluate(&ctx, None).await?;
# Ok(())
# }
```

**Server-side compilation**: Compile on server, send to client for rendering:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn example(ctx_server: &SessionContext, plot: Plot<Cartesian>) -> Result<(), Box<dyn std::error::Error>> {
// Server: compile plot
let compiled = plot.compile(&ctx_server).await?;
let json = serde_json::to_string(&compiled)?;
// Send JSON to client via HTTP/WebSocket

// Client: receives JSON, deserializes, and evaluates
let ctx_client = SessionContext::new();
let compiled: CompiledPlot = serde_json::from_str(&json)?;
let evaluated = compiled.evaluate(&ctx_client, None).await?;
# Ok(())
# }
```

**Important**: If your plot uses User-Defined Functions (UDFs), the same UDFs must be registered in both the compilation and evaluation contexts. See the [DataFusion Expressions Guide](../datafusion-expressions.md#user-defined-functions-advanced) for details on UDF serialization.

### Serializing EvaluatedPlot

`EvaluatedPlot` contains the final scene graph and can also be serialized:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn example(compiled: &CompiledPlot, ctx: &SessionContext) -> Result<(), Box<dyn std::error::Error>> {
// Evaluate and serialize
let evaluated = compiled.evaluate(&ctx, None).await?;
let json = serde_json::to_string(&evaluated)?;

// Later: deserialize and render
let evaluated: avenger_chart::plot::EvaluatedPlot = serde_json::from_str(&json)?;
// Render with any backend (PNG, SVG, etc.)
# Ok(())
# }
```

This is useful for pre-rendering plots and caching the final visual output.

### Format Options

- **JSON** (`serde_json`): Human-readable, larger size, slower
- **Bincode** (`bincode`): Binary format, compact size, faster
- **Other formats**: Any serde-compatible format (MessagePack, CBOR, etc.)

Choose based on your needs:
- JSON for debugging and HTTP APIs
- Bincode for performance-critical caching

## Debugging Compilation

### Compilation Errors

If `compile()` fails, it's usually due to:
- Invalid expressions (`col("nonexistent")`)
- Type mismatches (`col("string").gt(lit(0))`)
- Incompatible scale configurations

### Evaluation Errors

If `evaluate()` fails, it's usually due to:
- Data not matching schema
- Query execution errors
- Insufficient data for scale domains

## See Also

- [Parameters](../themes/parameters.md) - Using parameters with compilation
- [First Plot](../getting-started/first-plot.md) - Basic workflow
- [Channels](./channels/index.md) - Expression and channel basics
