# avenger-vegalite-compiler

Compile a supported Vega-Lite specification into a native `ChartDefinition`. The existing chart runtime evaluates its dataflow, lays out marks and guides, exports images, and hosts parameter updates in an Avenger app.

```rust,no_run
use avenger_chart::{Chart, RenderOptions};
use avenger_vegalite_compiler::{spec::UnitSpec, FromVegaLite, VegaLiteOptions};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let spec = UnitSpec::from_json(r#"{
  "data": {"values": [{"category":"A","amount":3}, {"category":"B","amount":5}]},
  "mark": "bar",
  "encoding": {
    "x": {"field":"category", "type":"nominal"},
    "y": {"field":"amount", "aggregate":"sum", "type":"quantitative"}
  }
}"#)?;
let chart = Chart::from_vegalite(&spec, &Default::default(), VegaLiteOptions::default()).await?;
let frame = chart.render(RenderOptions::default()).await?;
std::fs::write("bars.svg", frame.to_svg()?)?;
# Ok(()) }
```

For compile-only use, call `compile_vegalite(&spec, &datasets, base_dir)`. It captures data and builds native plans without preparing the dataflow or initializing fonts. Call `Chart::prepare` later, or serialize the definition. `CompileError::path()` identifies the failed property.

## Supported subset

The semantic reference is Vega-Lite **6.4.3**, with Vega **6.4.0**. This is a subset, with Avenger's fonts, tick placement, and rendering.

| Area | Compilation support |
|---|---|
| Marks | Vertical/horizontal bars, ranges, continuous positions, fixed thickness, negative values, and single aggregate bars |
| Aggregation | Count, valid, missing, sum, min, max, mean/average, sample/population variance and standard deviation |
| Transforms | Ordered explicit bin, aggregate, and structured numeric `gte` filter transforms |
| Encoding bins | Automatic or specified extent, maxbins, step/steps, minstep, base, divide, nice, anchor, and already-binned start/end or start plus step |
| Stacking | Default zero stacking of unaggregated rows, `stack: "zero"`, and `stack: null` |
| Positions | Linear, band, point, or disabled quantitative scales. Zero/nice options and typed category sorting, including first-seen order with `sort: null` |
| Guides | Axis enable/title/format/labelAngle/grid, multiline chart titles, explicit dimensions, or category-count step sizing |
| Style | Constant CSS color, opacity, and bar thickness |
| Parameters | Initialized numeric root parameters, referenced by a structured `gte` predicate |
| Sources | Inline scalar row objects, root datasets, named Arrow snapshots or replaceable inputs, local CSV/TSV/JSON, and `data: null` |

Temporal encodings parse in the spec crate but are rejected by this compiler. Other marks, composition/facet syntax, color encodings, selection parameters, tooltips, arbitrary expressions, and network loading are deferred. Unsupported combinations return errors.

Omitted field types follow Vega-Lite defaults: a plain field is nominal, while binning or aggregation implies quantitative. A numeric Arrow column does not by itself imply a quantitative encoding. Nested field access is deferred. Escape punctuation to refer to a literal field such as `a\.b`.

Explicit transforms precede encoding-generated transforms. Extent, bin parameters, bin assignment, visible rows, and domains are separate dataflow calculations. Numeric parameter changes reuse the same graph. Null categories remain distinct from the string `"null"`. Continuous null/non-finite positions are removed before rendering. Numeric strings can be used quantitatively through explicit conversion. This is not a general JavaScript coercion engine. `valid` and `missing` examine original values before positional conversion.

## Sources and ownership

Named caller bindings take precedence over root datasets. Inline and URL sources remain their own sources even if they have a name. Resolve relative file paths against `VegaLiteOptions::base_dir`, or the `base_dir` argument to compile-only construction. CSV/TSV require headers and infer a schema. JSON files contain row objects. Explicit format wins over the extension.

`compile_vegalite` and `Chart::from_vegalite` capture source data once, with local file I/O on a blocking worker. The definition owns its immutable source snapshot. Subsequent renders do not reopen files, and result-cache eviction does not release the source. Supply a named `TableSnapshot` for empty data requiring a schema or for data loaded by the application. Referenced JSON columns must have consistent scalar types. Unreferenced structured columns are ignored.

For changing data, use `compile_vegalite_with_input(&spec, schema)`. The spec must
use `data.name`, which becomes the name of a required root table input. This
entry point reads no data and ignores any matching root `datasets` entry.
Supply a snapshot with the declared raw schema for each render. No private
ordinal column is required. A cached source node assigns ordinals in snapshot
batch order before other transforms, preserving stack and first-seen category
order as batches arrive.

```rust,no_run
use avenger_chart::{Chart, RenderOptions};
use avenger_datafusion_dataflow::{TableSnapshot, TableStore};
use avenger_vegalite_compiler::{compile_vegalite_with_input, spec::UnitSpec};

# async fn example(spec: UnitSpec, first: TableSnapshot,
#     batch: avenger_datafusion_dataflow::arrow::record_batch::RecordBatch)
#     -> Result<(), Box<dyn std::error::Error>> {
// The spec uses {"data": {"name": "sales"}}.
let store = TableStore::new(first);
let first_snapshot = store.snapshot();
let definition = compile_vegalite_with_input(&spec, first_snapshot.schema().clone())?;
let sales = definition.dataflow().interface().root().table_input("sales")?;
let chart = Chart::prepare(definition, Default::default()).await?;
let inputs = chart.inputs()?.table(&sales, first_snapshot)?.finish()?;
let first_frame = chart.render(RenderOptions::default().inputs(inputs.clone())).await?;

let second_snapshot = store.append_batch(batch)?;
let next = inputs.edit().table(&sales, second_snapshot)?.finish()?;
let second_frame = chart.render(RenderOptions::default().inputs(next)).await?;
# Ok(()) }
```

The earlier inputs and frame remain immutable. New snapshots use ordinary
recomputation, and repeated bindings reuse the dataflow cache. A missing input
or a schema mismatch returns an error. Source replacement can change values,
row counts, and row order, but changing the declared schema requires recompiling.

## Parameters, apps, and serialization

A parameter and its use have this form:

```json
{
  "params": [{"name":"minimum", "value":0}],
  "transform": [{"filter":{"field":"amount", "gte":{"expr":"minimum"}}}]
}
```

Each parameter becomes a dataflow scalar input and a chart initial value. Override it with `RenderOptions::parameter`, or use `ChartAppState::set_parameter` after `chart.into_app(...)`. An override belongs to that request. A later default render uses the initial values.

The `gte` predicate accepts a numeric column and either a number or one declared parameter identifier. Numeric null compares as zero, NaN never matches, and infinities retain their numeric ordering. General expression strings are not parsed.

Serialize with `ChartDefinition::to_bytes_with_codec` and `avenger_transform::TransformExtensionCodec`. Decode with a dataflow runtime configured with that codec and `avenger_transform::function_versions()`. The artifact contains native plans, visual descriptors, initial parameters, and either captured source snapshots or input declarations. After decoding an input-backed chart, recover the table handle by name and bind a snapshot. It requires neither the original spec nor the source files. It contains no warm cache. Chart artifacts emit version 2 and retain version-1 read support.

Default Cargo features are `svg`, `pdf`, and `png`, forwarded to `avenger-chart`. Disable default features for compilation without export backends. The caller owns Tokio and the native event loop. Window-host dependencies belong only to examples.

## Examples and tests

```sh
cargo run -p avenger-vegalite-compiler --example export_bars -- /tmp/vegalite-bars
# Optional final argument: a spec file. Relative sources use its directory.
cargo run -p avenger-vegalite-compiler --example export_bars -- /tmp/histogram avenger-vegalite-compiler/examples/histogram.json
cargo run -p avenger-vegalite-compiler --example serialized_bars -- /tmp/restored-bars.svg
cargo run -p avenger-vegalite-compiler --example streaming_bars -- /tmp/streaming-bars
cargo run -p avenger-vegalite-compiler --example bar_app
```

The app uses Left/Right to change a minimum-amount parameter and Escape to reset it. It uses the existing chart background-update lifecycle and eventstream API.

For native Rust construction, see the chart-definition [bars](../avenger-chart-definition/examples/bars.rs), [facets](../avenger-chart-definition/examples/facets.rs), and [pan/zoom](../avenger-chart-definition/examples/pan_zoom.rs) examples. Both frontends produce the same definition type.

Rust tests run offline. [Reference fixtures](tests/fixtures/basic-bars.json) record scale domains and rectangle geometry from the pinned JavaScript packages. Regenerate them with [generate.mjs](tests/fixtures/generate.mjs), passing a directory with those packages installed. Node is not required to run the tests.
