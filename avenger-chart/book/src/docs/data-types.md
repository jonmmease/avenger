# Data Types and Channel Mapping

Choosing the right data type unlocks Avenger Chart’s automatic scale selection. This primer shows how DataFusion column types translate into scale defaults and when you should override them.

## Core categories

| Conceptual type | Typical DataFusion type(s) | Default scale | Notes |
|-----------------|----------------------------|---------------|-------|
| Quantitative    | `Float*`, `Int*`, `UInt*`   | `Linear`      | Auto padding for marks currently assumes linear scales. |
| Temporal        | `Date32`, `Date64`, `Timestamp*` | `Time` | Ensure the column is stored as an Arrow temporal type (cast from strings if necessary). |
| Ordinal         | Short, ordered categories (e.g., `"Small"`, `"Medium"`, `"Large"`) | `Ordinal` | Preserves input order; use when the order carries meaning. |
| Nominal         | Unordered categories (labels, IDs) | `Nominal` | Planned alongside the incoming `Nominal` scale—documented here so users can plan ahead. |
| Boolean         | `Boolean` | `Ordinal` | Two-value categories mapped to discrete visual outputs. |

### Casting and coercion

DataFusion expressions make it easy to align a column’s concrete type with how you intend to treat it in a chart:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn cast_example() -> Result<(), Box<dyn std::error::Error>> {
let ctx = SessionContext::new();
let df = ctx.read_csv("events.csv", CsvReadOptions::new()).await?;

let df = df
    .select(vec![
        col("timestamp").to_timestamp(None, None)?.alias("ts"),
        col("category").alias("category"),
        col("value").cast_to(&datafusion::arrow::datatypes::DataType::Float64, &ctx.state())?,
    ])?;
# let _ = df;
# Ok(())
# }
```

Performing casts up front means you can use simple `col("ts")` / `col("value")` expressions when mapping channels.

## Inspecting inferred scales

When you rely on defaults (e.g., `.x(col("value"))`), Avenger records the chosen scale in the compiled plan. To see what was inferred for debugging or documentation, inspect a compiled plot:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn debug_scales() -> Result<(), Box<dyn std::error::Error>> {
let ctx = SessionContext::new();
let df = ctx.read_parquet("iris.parquet", ParquetReadOptions::default()).await?;

let compiled = Plot::<Cartesian>::new()
    .data(df)
    .mark(Symbol::new().x(col("sepal_length")).y(col("sepal_width")))
    .compile(&ctx)
    .await?;

for (name, spec) in compiled.scale_specs() {
    println!("{name}: {spec:?}");
}
# Ok(())
# }
```

## When to override

- **Ordinal vs Nominal**: use `scale_with::<Ordinal>` when the category order matters (e.g., education level). Use `scale_with::<Nominal>` (as soon as it’s available) when the order should be driven by data appearance or legend sorting.
- **Linear vs Log**: switch to `scale_with::<Log>` for multiplicative ranges; Avenger defaults to `Linear`.
- **Time zones**: cast to `Timestamp(Nanosecond, Some("UTC"))` (or your desired zone) to avoid implicit local conversions before mapping.

With types set correctly, the rest of the guides (channels, scales, patterns) assume you can lean on the defaults and only tweak scales when you need custom domains or aesthetic choices.
