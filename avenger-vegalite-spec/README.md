# avenger-vegalite-spec

Handwritten Serde types for a subset of Vega-Lite v6.4.3. Parse, validate, edit, and serialize single-view bar charts, aggregate summaries, and histograms.

```rust
use avenger_vegalite_spec::{AggregateOp, UnitSpec};

let mut spec = UnitSpec::from_json(r#"{
    "data": {"name": "flights"},
    "mark": "bar",
    "encoding": {
        "x": {"field": "airline", "type": "nominal"},
        "y": {"field": "delay", "aggregate": "mean", "type": "quantitative"}
    }
}"#)?;

spec.encoding.as_mut().unwrap().y.as_mut().unwrap().aggregate = Some(AggregateOp::Max);
spec.validate()?;
let json = serde_json::to_string_pretty(&spec)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

`UnitSpec::from_json` deserializes and validates the specification. Types also implement ordinary Serde `Serialize` and `Deserialize`. Call `validate` after direct deserialization or edits through public fields. Errors expose `path()` and `message()`, for example `encoding.x.bin.step: expected a positive finite number`.

Parsing does not read data sources or `$schema` URLs, infer field types, fill defaults, calculate transforms, or render a chart. Named data can refer to a root dataset or a binding supplied later. The crate has no Arrow, DataFusion, or rendering dependency.

## Supported JSON

| Area | Supported properties and alternatives |
|---|---|
| Root | Required `data` and `mark`. Optional `$schema`, `name`, `description`, string/array `title`, numeric `width` and `height`, `encoding`, `transform`, `params`, `datasets`, and object-valued `usermeta` |
| Data | `values` containing row objects, `url`, `name`, or explicit null. Inline and URL sources may also have a `name`. Optional `format` with a `type` of `json`, `csv`, or `tsv` |
| Datasets | A map of names to arrays of row objects |
| Mark | `"bar"` or `{ "type": "bar", ... }` with optional `orient`, string `color`, numeric `opacity`, and numeric `size` |
| Primary encoding | `x` and `y` with `field`, `type`, `aggregate`, `bin`, `title`, `sort`, `axis`, `scale`, and `stack` (null or `"zero"`). Field type can be quantitative, ordinal, nominal, or temporal |
| Secondary encoding | `x2` and `y2`, each with a `field` |
| Axis | `title`, `format`, `labelAngle`, and `grid` |
| Scale | `type` (linear, band, or point), Boolean `zero`, and Boolean `nice` |
| Sort | Ascending, descending, or null |
| Transforms | Ordered explicit bin, aggregate, and structured numeric `gte` filter transforms |

Unknown properties and unsupported alternatives are errors. This release does not support other marks, color encodings, other stack offsets, tooltips, selections, general expression strings, calculate transforms, time units, composition, config objects, or responsive dimensions. The version identifies the reference grammar, not full Vega-Lite compatibility.

### Aggregation

Encoding `aggregate` and explicit transform `op` accept `count`, `valid`, `missing`, `sum`, `min`, `max`, `mean`, `average`, `variance`, `variancep`, `stdev`, and `stdevp`. Fieldless count is supported. All other operations require a field. Mean and average remain separate spellings during round trips.

```json
{
  "aggregate": [
    {"op": "count", "as": "flights"},
    {"op": "mean", "field": "distance", "as": "mean_distance"}
  ],
  "groupby": ["airline"]
}
```

Explicit aggregate transforms require at least one measure and distinct output aliases. Missing and empty `groupby` stay distinct. Count accepts and preserves an optional field. Compilation uses row count, and maps `mean` and `average` to `avenger-transform::expr_fn::mean`. The other operations map to the corresponding transform helpers.

### Binning

Encoding `bin` accepts true, false, null, a parameter object, or `"binned"`. Parameters are `maxbins`, `step`, `steps`, `minstep`, `base`, `divide`, `nice`, `anchor`, a numeric two-element `extent`, and `binned`. The parameter object can be empty. Selection-based extents and Vega's `span` option are not supported.

```json
{
  "bin": {"step": 10, "extent": [-60, 180]},
  "field": "delay",
  "as": ["delay_lo", "delay_hi"]
}
```

An explicit bin transform accepts true or a parameter object. Its `as` is either a string or two distinct boundary names. The string stays unexpanded. For already binned data, use `"bin": "binned"` or a parameter object with `"binned": true`, and encode existing boundary fields using `x`/`x2` or `y`/`y2`.

Validation requires finite numeric options, `maxbins >= 2`, positive step, nonnegative minstep, base and divisors greater than one, positive increasing candidate steps, and an ordered extent. Singleton extents are allowed. Divide accepts one or two numbers, following the JSON Schema. Numeric options, including maxbins, use `f64`. Step and maxbins can coexist without the parser applying precedence.

These numeric checks, nonempty aggregate measures, and the two-name bin output restriction are deliberate subset constraints beyond the schema's structural rules. Validation also checks nonnegative dimensions and mark size, opacity in `[0, 1]`, and label angle in `[-360, 360]`. Field existence and input data types need a future compiler with access to the data schema.

## Presence and defaults

`MissingNullOrValue<T>` preserves omission, explicit null, and a supplied value. It is used for nullable optional properties such as bin, axis, scale, sort, and field/axis titles. Optional non-nullable properties reject null. Root data is required but accepts null. The top-level title does not accept null.

Serialization preserves authored alternatives, including false, empty objects and arrays, mark string/object form, and string/array aliases. It does not preserve whitespace, object-key order, or numeric spelling such as `10` versus `10.0`. Field names retain their dots, brackets, and escapes without interpretation.

Defaults belong to later compilation. In particular, Vega-Lite's positional bin default is 10, while `avenger-transform` uses Vega's default of 20. A compiler must supply the Vega-Lite default explicitly. Inferred extent should be a separate dataflow calculation feeding bin parameters. This crate preserves the information needed for those decisions.

## Example and checks

```sh
cargo run -p avenger-vegalite-spec --example bar_specs
cargo test -p avenger-vegalite-spec --all-targets
cargo test -p avenger-vegalite-spec --doc
```

The example parses and prints three specs without accessing source files. Tests run offline. Positive fixtures and serialized example output are audited against the [Vega-Lite v6.4.3 schema](https://vega.github.io/schema/vega-lite/v6.4.3.json). See [fixture provenance](tests/fixtures/README.md).

### Numeric parameters and filters

Root `params` accepts initialized numeric parameters, for example `[{"name":"minimum","value":0}]`. Names must be unique identifiers. Initial values must be finite. Structured filters accept `{"filter":{"field":"amount","gte":10}}` or `{"filter":{"field":"amount","gte":{"expr":"minimum"}}}`. The compiler resolves expression references to a declared parameter. These shapes serialize without default expansion.

Use [avenger-vegalite-compiler](../avenger-vegalite-compiler/README.md) to lower this syntax into an Avenger chart definition. Parsing support and compilation support are documented separately.
