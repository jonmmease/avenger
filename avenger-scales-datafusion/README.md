# avenger-scales-datafusion

Apply `avenger-scales` kernels inside DataFusion expressions. This crate depends on DataFusion 54.1.0 and has no dependency on chart compilation or the dataflow runtime.

The `scale(domain, range, options, values)` UDF accepts list-valued domain and range expressions, a struct-valued options expression, and scalar or array values. `BuiltinScale` selects the kernel. `ScaleSpec` permits serializable custom kernels.

## Expressions

```rust
use std::{collections::HashMap, sync::Arc};
use avenger_scales_datafusion::{
    datafusion::{
        arrow::array::Float64Array,
        logical_expr::col,
    },
    list_literal, options_literal, scale_expr, BuiltinScale,
};

let x = scale_expr(
    BuiltinScale::Linear,
    list_literal(Arc::new(Float64Array::from(vec![0.0, 100.0])))?,
    list_literal(Arc::new(Float64Array::from(vec![0.0, 640.0])))?,
    options_literal(&HashMap::new())?,
    col("amount"),
)?;
// Use x.alias("x") in a DataFusion projection or a Dataflow plan node.
# Ok::<(), datafusion::common::DataFusionError>(())
```

`create_scale_udf(spec)` returns a `ScalarUDF` that can be called directly or registered in a `SessionContext`. Different descriptors produce distinct function identities even though their SQL name is `scale`. One registered function selects one descriptor. Rust expressions can use several descriptors in the same plan.

Domain, range, and options can come from literals, scalar subqueries, or dataflow scalar references. They must be constant within each record batch. Broadcast arrays are accepted after validation. Varying configuration rows fail instead of silently using the first row. Facet-specific configuration belongs in separate scoped instances.

The adapter applies the scale to the entire values array in one call. It reuses the kernel implementation across batches, but constructs its configuration from the current arguments on each invocation. It has no result cache. Null values retain the underlying kernel's behavior. Null domain/range lists and null options structs fail validation. Built-in domain elements and continuous range elements must be non-null. Discrete ranges can contain null values. Empty input batches return an empty array of the planned output type.

`options_literal()` sorts fields by name and preserves their Arrow values. The UDF passes option values to `avenger-scales` without narrowing integers or floats and without silently dropping unknown types. Kernel validation checks option names and values. Built-in square-root scales use the power kernel with a default exponent of 0.5. An explicit exponent overrides that default.

## Built-in types

The adapter preserves the underlying scale kernels' output types:

| Scale | Output |
|---|---|
| Linear, log, power, square root, symmetric log, time, band, point | `Float32` for numeric ranges |
| Ordinal, threshold, quantile, quantize | `Dictionary<Int16, T>`, where `T` is the range element type |
| Linear, log, power, square root with color ranges | `List<Float32>` RGBA values |

Numeric values are coerced to `Float32`, matching the numeric kernels. Time values preserve their date/timestamp type. The adapter accepts Boolean, Int32, Float32, and Utf8 categorical domains, with matching input types. Nested bands are not exposed through `BuiltinScale`. This crate adds no alternate scale algorithms.

The current scale function version is `avenger-scales-datafusion/2`. Version 2 uses dictionary outputs for discrete scales and includes the foundation stack's updated kernels. Version 1 artifacts are rejected so they cannot silently acquire different types or scale behavior.

## Serialization

`ScaleExtensionCodec` implements DataFusion's `LogicalExtensionCodec`. It encodes a versioned scale descriptor and reconstructs its kernel during decoding. Domains, ranges, options, and values remain ordinary expression arguments in the enclosing DataFusion plan or dataflow artifact.

The default codec delegates unrelated functions and extensions to DataFusion's default codec. `ScaleExtensionCodec::with_fallback()` wraps an application codec and forwards unrelated scalar, aggregate, window, higher-order, table-provider, file-format, and logical-extension operations.

For `avenger-datafusion-dataflow`, pass the codec to both `Dataflow::to_bytes_with_codec()` and `Runtime::with_session_state_and_codec()`. Set `scalar:scale` to `SCALE_FUNCTION_VERSION` in both the definition's `SemanticConfig::function_versions` and the destination `RuntimeConfig::function_versions`. Decoding returns a native `Dataflow` with resolved scale functions. It does not require registering a scale UDF in the destination session.

Run the complete example:

```sh
cargo run --release -p avenger-scales-datafusion --example dataflow_scale --locked
```

The example computes a domain maximum, serializes the definition and its fixed data, loads them into another runtime, and queries widths of 200, 400, and 200. The graph caches the maximum across width changes and can reuse the earlier scaled table when the width returns to 200.

## Custom scales

Implement `ScaleSpec` for a Serde-serializable descriptor and annotate the implementation with `#[typetag::serde(name = "your_scale_v1")]`. Its factory returns an `Arc<dyn avenger_scales::scales::ScaleImpl>`. Its type methods describe the kernel's input and output types. Optional hooks supply default options and validate configuration shape. The descriptor replaces the chart-specific scale specification used by the original adapter.

Descriptors must serialize every result-affecting setting. Construction captures a JSON descriptor snapshot and reconstructs the kernel from that snapshot. The UDF uses its canonical bytes for equality, hashing, and codec payloads. Descriptors that cannot round trip through JSON fail construction. Kernels must be deterministic and independent of mutable external state because the function declares `Volatility::Immutable`. Configurations use `ScaleContext::default()`.

The destination binary must include the custom `ScaleSpec` implementation and its typetag registration. Payloads carry configuration, not executable Rust code. Change the descriptor's versioned typetag name when changing its semantics, and update the host's function version requirement accordingly. Unknown descriptors, incompatible codec versions, and malformed payloads fail decoding.

## Extraction boundary

This crate extracts the UDF, expression construction, and codec responsibilities from `avenger-chart-scales` on `codex/facet-on-core-stack`. Chart domain inference, channel defaults, layout, and legend construction remain chart responsibilities. The existing chart branch is unchanged. The new crate uses the current stack's `avenger-scales` API and DataFusion 54.1.0.
