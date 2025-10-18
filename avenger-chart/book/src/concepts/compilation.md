# The Compilation Pipeline

Avenger-Chart uses a two-stage workflow: **compile** then **evaluate**. Understanding this architecture is essential for efficient chart creation, especially when working with parameters or rendering multiple variations.

## Why Two Stages?

The separation allows you to:
- **Compile once** - Expensive optimizations happen only once
- **Evaluate many times** - Render with different data or parameters cheaply
- **Use parameters** - Dynamic values without recompilation

This architecture is similar to how databases prepare and execute queries, or how compilers separate parse/optimize from execution.

## The Pipeline

```
┌─────────────────────────────────────────────────────────────┐
│  Plot Specification                                         │
│  .mark(...).data(...).add_param(...)                        │
└────────────┬────────────────────────────────────────────────┘
             │
             ▼
    ┌────────────────┐     Expensive (do once)
    │   COMPILE      │     - Parse expressions
    │                │     - Infer scales
    │   plot.compile │     - Build query plan
    │                │     - Optimize AST
    └────────┬───────┘
             │
             ▼
┌────────────────────────────────────────────────────────────┐
│  Compiled Plot                                             │
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
│  Evaluated Plot                                            │
│  Scene graph ready for rendering                          │
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

**Key Point**: Compilation happens without accessing the actual data - it only analyzes the structure and types.

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

```rust
let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(Symbol::new().x(col("x")).y(col("y")));

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
```

Typical timing (approximate):
- Compile: 1-5ms
- Evaluate: 5-50ms (depends on data size)

### Multiple Renders with Parameters

The real benefit appears when rendering multiple times:

```rust
// Compile once
let plot = Plot::<Cartesian>::new()
    .data(df)
    .add_param(threshold)
    .mark(/* ... uses threshold parameter ... */);

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

```rust
// Setup - compile once
let color_param = Param::new("highlight_color", ScalarValue::Utf8(Some("#ff0000".into())));

let plot = Plot::<Cartesian>::new()
    .data(df)
    .add_param(color_param.clone())
    .mark(/* ... uses color_param ... */);

let compiled = plot.compile(&ctx).await?;

// User interaction loop - evaluate many times
loop {
    let user_color = get_user_selected_color();  // From UI

    let params = indexmap!{
        "highlight_color".to_string() => ScalarValue::Utf8(Some(user_color))
    };

    let evaluated = compiled.evaluate(&ctx, Some(params)).await?;
    renderer.display(&evaluated);  // Update display
}
```

### Example 2: Data Animation

```rust
// Compile once with time parameter
let time_param = Param::new("current_time", ScalarValue::from(0.0));

let plot = Plot::<Cartesian>::new()
    .data(df)
    .add_param(time_param.clone())
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
```

### Example 3: Batch Export

```rust
// Generate charts for multiple regions
let region_param = Param::new("selected_region", ScalarValue::Utf8(Some("North".into())));

let plot = Plot::<Cartesian>::new()
    .data(df)
    .add_param(region_param.clone())
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
    renderer.write_png(&evaluated, &format!("{}_sales.png", region)).await?;
}
```

## Common Patterns

### Pattern: Compile Once, Render Once

```rust
// Simple case - no reuse needed
let plot = Plot::<Cartesian>::new().data(df).mark(Symbol::new().x(col("x")).y(col("y")));
let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
renderer.write_png(&evaluated, "chart.png").await?;
```

### Pattern: Compile Once, Evaluate Many

```rust
// With parameters - reuse compilation
let compiled = plot.compile(&ctx).await?;

for param_value in values {
    let evaluated = compiled.evaluate(&ctx, Some(params_for(param_value))).await?;
    // Render/save/display
}
```

### Pattern: Different Data, Same Structure

```rust
// Same schema, different data - recompile needed
for dataset in datasets {
    let plot = Plot::<Cartesian>::new()
        .data(dataset)  // Different data each time
        .mark(Symbol::new().x(col("x")).y(col("y")));

    let compiled = plot.compile(&ctx).await?;
    let evaluated = compiled.evaluate(&ctx, None).await?;
    // Render
}
```

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

- [Parameters](../advanced/parameters.md) - Using parameters with compilation
- [First Plot](../getting-started/first-plot.md) - Basic workflow
- [Channels](./channels.md) - Expression and channel basics
