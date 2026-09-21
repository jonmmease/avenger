# avenger-transform

Vega-style transforms as native DataFusion 54.1.0 logical plans and expressions.
Build extent, bin, filter, formula, and aggregate calculations without executing
queries. Use the plans directly with DataFusion, or register them as nodes in
`avenger-datafusion-dataflow`.

The library has no runtime dependency on other Avenger crates. Source access,
execution, caching, scopes, and preaggregation remain caller responsibilities.
A future Vega-Lite compiler can use these constructors after deciding which
transforms and dependencies a specification requires.

## Public API

| Constructor | Result | Behavior |
|---|---|---|
| `extent(input, value)` | `LogicalPlan` | One row with an `extent` struct |
| `bin_parameters(extent, options)` | `Expr` | Independently evaluable bin configuration |
| `bin(input, value, parameters, [start, end])` | `LogicalPlan` | Add or replace two boundary fields |
| `filter(input, predicate)` | `LogicalPlan` | Retain rows using Boolean or Vega truthiness |
| `formula(input, value, name)` | `LogicalPlan` | Add or replace one field |
| `aggregate(input, group_by, measures)` | `LogicalPlan` | Group rows and evaluate aliased measures |

All constructors return DataFusion `Result`. They accept `Expr`, including typed
placeholders and scalar subqueries. They do not parse Vega expression strings or
invoke an optimizer. Supplied functions retain their native volatility. Dataflow
controls reuse of their results.

```rust
use std::sync::Arc;
use avenger_transform::{self as transform, expr_fn, BinOptions};
use datafusion::{common::Result, logical_expr::{col, lit, scalar_subquery, LogicalPlan}};

fn histogram(rows: LogicalPlan) -> Result<LogicalPlan> {
    let extent = transform::extent(rows.clone(), col("delay"))?;
    let parameters = transform::bin_parameters(
        scalar_subquery(Arc::new(extent)),
        BinOptions { maxbins: Some(lit(20.0)), ..Default::default() },
    )?;
    let rows = transform::bin(rows, col("delay"), parameters, ["lo", "hi"])?;
    transform::aggregate(
        rows,
        vec![col("lo"), col("hi")],
        vec![expr_fn::count().alias("flights")],
    )
}
```

Formula preserves field order when replacing a field and appends new fields.
Replacement expressions read the original input schema, so `x + 1 AS x` is
valid. Apply formulas in sequence when a later formula uses an earlier result.
The bin helper also reads the original value once in its logical projection.
Both helpers preserve the other qualified columns, accept literal output names
containing dots, and reject ambiguous or duplicate output fields. Use
`Column::from_name("a.b")` to refer to a literal input name containing a dot.

## Typed value rules

| Operation | Supported input | Missing and non-finite values |
|---|---|---|
| Extent, bin, numeric reductions | Signed/unsigned integers, Float32/64 | Convert to Float64; no implicit string/date conversion |
| Filter / `expr_fn::truthy` | Boolean, numeric, Utf8/LargeUtf8/Utf8View | Null, NaN, ±0, false, and empty strings reject; other values pass |
| `valid`, `missing` | Boolean, numeric, strings | Valid excludes null, NaN, empty strings; missing counts null/empty strings; NaN is neither |
| `min`, `max` | Numeric, strings | Ignore null, NaN, empty strings; preserve the input type |
| `sum`, `mean`, moments | Numeric | Ignore null and NaN; native Float64 reduction semantics |

JavaScript object coercion, numeric strings, dates, decimals, dictionaries, and
nested input values are outside this initial contract. Convert explicitly at
ingestion or in a formula. Converting integers above 2^53 to Float64 can lose
precision, as in JavaScript numbers.

### Extent and bins

Extent always emits one row and one column:

```text
extent: Struct<min: Float64?, max: Float64?>   // parent is non-null
```

Empty/all-invalid input yields two null endpoints. NaN is ignored. If either
resulting endpoint is infinite, both endpoints are null.

Bin parameters have this type:

```text
Struct<start: Float64, stop: Float64, step: Float64, upper_bound: Float64>?
```

An absent extent yields null parameters and null bin boundaries. `stop` is Vega's
reported stop. `upper_bound` is the inclusive bound used for assignment. For
extent `[0, 29]`, `step=5`, and `nice=false`, the parameters are
`{start: 0, stop: 29, step: 5, upper_bound: 30}`. Values 28, 29, and 30 all go to
`[25, 30]`. Adding `anchor=4` changes start to 4 and upper_bound to 34, while stop
remains 29.

`BinOptions` exposes expression-valued `maxbins`, `base`, `divide`, `span`, `step`,
`steps`, `minstep`, `nice`, and `anchor`. Omitted options use Vega defaults:
maxbins 20, base 10, divide `[5, 2]`, minstep 0, and nice true. `divide` and `steps`
accept numeric List expressions, such as DataFusion `make_array`. Explicit step
has precedence over step selection. Options and resolved parameters must be
scalar or constant within each record batch. Row-varying configurations error.

Invalid types, null options, non-finite values, unordered extents, nonpositive
steps/spans, maxbins below 1, base/divisors at most 1, and invalid step lists error.
Arithmetic that cannot produce finite, increasing boundaries also errors.
Refinement has a fixed iteration limit. Bin calculation never allocates a bin
catalog.

Assignment follows Vega's boundary bias, inclusive final bin, and anchor rules.
Out-of-range values produce ±Infinity, input NaN produces NaN, and null produces
null. Individual `expr_fn::bin_start` and `bin_end` expressions are also available.
Kernels process Arrow arrays. They do not serialize or convert each row to a
dynamic scalar value.

### Aggregates

Helpers: `count`, `valid`, `missing`, `sum`, `min`, `max`, `mean`, `variance`,
`variancep`, `stdev`, and `stdevp`. Count counts rows, including missing values.
All four variance/stddev helpers return null for fewer than two valid values,
including the population forms.

Measures require aliases. Computed grouping expressions also require aliases.
Plain grouping columns keep their field names. Native DataFusion aggregates and
arithmetic around aggregates are accepted. For example,
`(expr_fn::sum(col("x")) / expr_fn::count()).alias("ratio")`. The constructor
extracts native aggregate calls and adds the finishing projection. Duplicate
measures can have separate output aliases. At least one measure is required.

Empty input returns zero rows, including a global aggregate, matching Vega's
normal `drop=true` behavior. Observed groups with all-invalid values remain,
with count > 0 and null numeric measures. These rules require a count guard for
global aggregates and moments. The underlying aggregate calls remain visible
to the preaggregate planner.

Finite floating reductions match Vega within tolerance. Input order and
partitioned summation can change rounding. Infinite-input reductions
use DataFusion behavior, which can differ from Vega. In particular, native
DataFusion 54.1.0 grouped extrema can retain a finite accumulator sentinel for
an all-positive-infinity minimum or all-negative-infinity maximum. Extent's
finite-endpoint rule and bin assignment have separate contracts above.

Distinct, product, median/quantiles, confidence intervals, argmin/argmax, tuple
collection, and Vega `drop=false`, `cross`, `key`, and `initonly` options are not
implemented as compatibility helpers. Native DataFusion aggregates remain usable
with their own semantics and preaggregate eligibility.

## Dataflow and preaggregation

Register extent, its scalar subquery, bin parameters, and binned rows separately
to expose reuse boundaries:

```text
rows → extent table → extent scalar → bin parameters → binned rows
                                                   ↗       ↓
                                          bin options      filter → aggregate
                                                            ↑
                                                       selection Expr
```

Binned rows also depend directly on source rows. With caching enabled, a brush
change can reuse extent, parameters, and bins. A bin-option change can reuse
extent. A new table snapshot invalidates dependent calculations. The same
constructors work within dataflow scopes. The caller chooses shared global
extents or separate facet extents.

For preaggregation, put the changing-filter marker immediately before the visible
aggregate section using `FilterQuery::new(source, |rows| aggregate(...))`. Include
fixed filters, formulas, and bins in the source section when their semantics
permit it. Require expressions to be valid over the entire warm-up dataset.
Use `avenger_datafusion_preaggregate::dataflow::Query::install` to register the
planner's plans in dataflow and reuse warm-up results. Bind a changing predicate,
apply its inputs, and query the binding's output, as in `preaggregate_histogram`.
Unsupported queries or unretained predicate dimensions use the planner's direct
fallback. No automatic graph rewrite or chart coordinator is introduced here.

Display histogram bins are separate from pixel interaction cells. Use
`avenger-selection` to resolve pixel selection semantics and supply retained
dimensions to preaggregate. Display bins do not authorize rounding raw predicates.

## Serialization

Use `TransformExtensionCodec::default()` for DataFusion protobuf serialization.
For a dataflow, set `function_versions()` in both `SemanticConfig` and
`RuntimeConfig`, serialize with `to_bytes_with_codec`, then decode in a runtime
with the same codec and versions. All transform UDFs reconstruct as native
functions. Options remain ordinary expression arguments.

`TransformExtensionCodec::with_fallback(Arc::new(other_codec))` delegates foreign
functions, plans, and table sources, including scale UDFs. The codec checks both
function identity and semantic version. Generated aggregate-state State/Merge
functions need their own codecs, which the aggregate-state crate does not yet
provide. Serializing those generated plans is outside this release.

## Examples and verification

```sh
cargo run -p avenger-transform --example logical_plans
cargo run -p avenger-transform --example dataflow_histogram
cargo run -p avenger-transform --example preaggregate_histogram
cargo test -p avenger-transform --all-targets
```

- `logical_plans`: direct DataFusion execution, SQL, and a pretty result table.
- `dataflow_histogram`: brush changes, bin-option changes, and actual executed-node/cache reports.
- `preaggregate_histogram`: hover warm-up, drag reuse, fixed-filter/bin invalidation,
  and an unretained-predicate fallback, with SQL and tables.

One integration test binary covers transforms, dataflow reuse, preaggregation,
and serialization. Tests read checked-in fixtures from Vega **6.2.0**, commit
`4dea72921d25bf6ff6636a9f9cb6c63ff696932c`. Regeneration is optional:

```sh
cd avenger-transform/tests/reference
npm ci --ignore-scripts
npm run generate
```

## Vega references

The bin algorithm is adapted from Vega under its BSD license. See
[LICENSE.vega](LICENSE.vega). Reference implementations:

- [Extent](https://github.com/vega/vega/blob/4dea72921d25bf6ff6636a9f9cb6c63ff696932c/packages/vega-transforms/src/Extent.js)
- [Bin parameter selection](https://github.com/vega/vega/blob/4dea72921d25bf6ff6636a9f9cb6c63ff696932c/packages/vega-statistics/src/bin.js) and [bin assignment](https://github.com/vega/vega/blob/4dea72921d25bf6ff6636a9f9cb6c63ff696932c/packages/vega-transforms/src/Bin.js)
- [Aggregate value rules](https://github.com/vega/vega/blob/4dea72921d25bf6ff6636a9f9cb6c63ff696932c/packages/vega-transforms/src/util/AggregateOps.js)
- [Filter](https://github.com/vega/vega/blob/4dea72921d25bf6ff6636a9f9cb6c63ff696932c/packages/vega-transforms/src/Filter.js) and [Formula](https://github.com/vega/vega/blob/4dea72921d25bf6ff6636a9f9cb6c63ff696932c/packages/vega-transforms/src/Formula.js)

`stack_zero` appends start/end columns using separate positive and negative window sums. Supply partition expressions and an explicit stable row order. The compiler decides when stacking applies. Null and NaN contribute zero.
