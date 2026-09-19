# avenger-datafusion-preaggregate

Prepare reusable DataFusion queries for changing filters. An eligible query has
two stages:

1. **Materialization** groups source rows into cells and stores aggregate states.
2. **Rollup** filters those cells, merges their states into the original groups,
   and runs the rest of the query.

The crate returns logical plans. It does not execute queries or own a cache.
It depends on DataFusion **54.1.0** and
[`avenger-datafusion-aggregate-state`](../avenger-datafusion-aggregate-state),
with no dependency on selection, charts, or dataflow.

## Prepare, materialize, and bind

`FilterQuery` owns the changing filter location. Build the final query from its
supplied rows, including any existing fixed filters. Explicit retained dimensions
specify coverage before a changing predicate exists, so the caller can warm the
materialization on hover.

```rust,no_run
use std::sync::Arc;
use avenger_datafusion_preaggregate::{BoundQuery, FilterQuery, PreaggregatePlanner};
use datafusion::{
    datasource::MemTable,
    functions_aggregate::expr_fn::{avg, count},
    logical_expr::{col, lit, LogicalPlanBuilder},
    prelude::SessionContext,
};
# async fn example(ctx: &SessionContext) -> datafusion::common::Result<()> {
let source = ctx.table("flights").await?.into_unoptimized_plan();
let query = FilterQuery::new(source, |rows| {
    LogicalPlanBuilder::from(rows)
        .filter(col("region").eq(lit("East")))?
        .aggregate(
            vec![col("airline")],
            vec![count(lit(1_i64)).alias("flights"), avg(col("distance")).alias("mean")],
        )?
        .build()
})?;
let prepared = PreaggregatePlanner::default().prepare(query, vec![col("delay")])?;

// Materialize once, before the brush has bounds.
let stored = if let Some(plan) = prepared.materialization_plan() {
    let schema = Arc::new(plan.schema().as_arrow().clone());
    let batches = ctx.execute_logical_plan(plan.clone()).await?.collect().await?;
    Some(ctx.read_table(Arc::new(MemTable::try_new(schema, vec![batches])?))?
        .into_unoptimized_plan())
} else {
    None
};

for (lower, upper) in [(0_i64, 30_i64), (10, 40)] {
    let changing = col("delay").gt_eq(lit(lower)).and(col("delay").lt(lit(upper)));
    let bound = prepared.bind(changing)?;
    println!("{:?}", bound.diagnostics());
    let plan = match bound {
        BoundQuery::Direct { plan, .. } => plan,
        BoundQuery::Preaggregated { rollup, .. } => {
            rollup.with_materialization(stored.as_ref().unwrap().clone())?
        }
    };
    let batches = ctx.execute_logical_plan(plan).await?.collect().await?;
    # let _ = batches;
}
# Ok(())
# }
```

Preparation and binding do not read sources, evaluate functions, or create
physical plans. Preparation performs query recognition, dimension discovery,
and aggregate decomposition once. Binding resolves and checks the new predicate
and substitutes it into the prepared plans.

`prepared.explain()` reports preparation eligibility, original and retained
dimensions, state expressions, the materialization schema, and the numerical
contract. `materialization_plan()` returns `None` for unsupported queries.
`bind_with_policy(predicate, QueryPolicy::ForceDirect)` preserves the original
query shape. It does not disable an external runtime's result cache.

`RollupQuery::materialization_schema()` describes its **input**, including nested
state metadata. `with_materialization()` checks the complete Arrow schema.
Relation qualifiers may differ. This is a schema check, not a provenance check:
the caller must supply states built from compatible source versions and inputs
to existing filters. Passing a construction plan instead of stored batches
recomputes that plan when the rollup executes. Preserve the schema when there
are no result batches.

## Supported queries

The target is the nearest aggregate **above the owned filter location**. Ordinary
filters and relation aliases may occur between the marker and target. Source
plans below the marker are opaque to relational rewriting. Visible expressions
throughout the query must remain immutable.

| Query component | Support |
|---|---|
| Target aggregates | Native `COUNT`, `SUM`, `MIN`, `MAX`, `AVG`, `VAR_SAMP`, `VAR_POP`, `STDDEV_SAMP`, `STDDEV_POP`, including native aliases |
| Aggregate `FILTER` | A separate checked filter on each State call, without retaining filter-only columns |
| Arguments and grouping expressions | Columns, literals, aliases, checked casts, safe floating arithmetic, positive-literal division, `TRY_CAST`, Boolean/null tests, checked `CASE`, and trusted total UDFs |
| After the target | Projection, `HAVING`/Filter, alias, Sort, top-k/fetch, Limit/offset, Window, further Aggregate, in their original order |
| Outer aggregates | Native-valid immutable functions, including distinct count and median, without requiring a state recipe for that outer function |
| Before the target | Joins, windows, projections, limits, repeated marker paths, and expression subqueries use direct execution |
| Target modifiers | DISTINCT, aggregate ORDER BY, and explicit null treatment use direct execution |
| Other targets | Unknown aggregate implementations and grouping sets use direct execution |

Only the target is decomposed. Its original schema is restored before finishing
operators execute. A window therefore sees the selected original groups, and an
outer average gives each reconstructed group its original weight. Materialization
does not discard groups according to a current top-k or window result.

Aggregate filters stay on their individual State calls. Observed groups remain
present even when all filters reject their rows. `COUNT(*)` counts rows and
`COUNT(expr)` counts non-null values. A global aggregate over an empty selection
still returns its SQL empty-input row. A grouped aggregate can return zero rows.

### Predicate coverage

Both explicit retained dimensions and the target's original grouping expressions
are available to a changing predicate, including hidden grouping expressions.
A grouping expression makes its result available, not necessarily its raw inputs.
For example, grouping by `delay / 10` does not retain individual delay values.

The binder matches resolved expression trees before descending into children.
It checks configured UDF identity, argument types, and literals, normalizes
relation aliases, and deduplicates storage dimensions. It preserves AND/OR/NOT,
null tests, comparisons, BETWEEN, IN, and correlated combinations when they can
be evaluated from the stored dimensions. It performs no algebraic inversion or
implicit bin rounding. Missing dimensions cause direct fallback for that binding.

### Scalar safety

Materialization can evaluate expressions on rows excluded by the current
changing predicate. Immutability alone does not prove safety. The planner checks
native type coercions and every child expression without executing functions.

The initial arithmetic allowlist admits Float32/Float64 addition, subtraction,
and multiplication, plus division by a positive literal of the same type. Casts
include same-type/null casts, integer widening within signed or unsigned families,
Float32 to Float64, integer-to-float, and decimal-to-Float64 conversions. Integral
and decimal arithmetic with unproved overflow risks remains direct. The planner
does not change overflow settings, infer bounds from a filter, or force measures
to Float64 to make a query eligible.

`TRY_CAST` and `CASE` require independently safe children. An aggregate `FILTER`
cannot prove its argument safe: native grouped execution can evaluate arguments
before applying that filter. Unknown UDFs remain direct unless the caller supplies
`ExpressionProperties`. Its trusted properties distinguish total evaluation from
preservation of DataFusion's grouping equality, including NaNs and signed zero.
Neither property overrides volatility. Unresolved parameters in moved expressions
remain conservative direct cases.

Invalid columns, ambiguous references, non-Boolean changing predicates, foreign
markers, and unresolved parameters in concrete bindings return DataFusion errors.
Supported preparation can still produce a direct binding for an unsupported
predicate. There is no execution-time retry that hides query failures.

### Numerical and ordering contract

The state utility uses native DataFusion accumulators. Regrouping changes
accumulation order and can change floating rounding, overflow, and non-finite
results. `Auto` does not promise bitwise equality or identical error timing.
These differences can also affect HAVING, ranks, and top-k membership near a
numeric boundary. See the utility's
[numerical contract](../avenger-datafusion-aggregate-state/README.md#numerical-behavior).

Unordered results and incompletely ordered ties retain SQL's ordering freedom.
Tests use exact counts and complete ordering keys for structural checks, and
finite floating tolerances for numerical comparisons. No cardinality estimator
or cost-based strategy choice is included.

## SQL construction and runnable examples

Use `FilterQuery::builder(source)` for asynchronous construction. Register
`builder.rows()` in a temporary `ViewTable`, parse SQL through a session, and pass
the unoptimized logical plan to `builder.finish(plan)`. The builder checks marker
ownership. Remove the temporary registration after parsing. Normal DataFusion
optimization runs after predicates and materialized relations are substituted.

```sh
cargo run -p avenger-datafusion-preaggregate --example logical_plans
cargo run -p avenger-datafusion-preaggregate --example sql
```

Both examples materialize batches once, bind four brush regions, print SQL and
pretty tables, and compare each result with direct execution. The SQL example
combines all five extensions: filtered computed measures, a window, top-k/offset,
and an outer aggregate.

## Runtime integration

The public `runtime` module supports installing a preparation once in a graph:

- `prepared.parameterize(ParameterExpressions { source, retained })` produces
  direct and optional pre-aggregation templates with caller-owned Boolean
  parameters for the changing predicate's two forms.
- `prepared.bind(changing).predicates()` provides validated source and retained
  expressions. Direct bindings have no retained expression.
- `ParameterizedFamily::check_binding()` rejects values from another preparation.
  A clone retains its preparation identity. This identity is not a cache key.
- The adapter selects the binding's reported strategy and substitutes a stored
  relation into the rollup. It must not manufacture unchecked retained predicates.

Existing filters remain ordinary parts of the supplied query. They do not appear
in `ParameterExpressions`. A deferred expression wrapper in such a filter needs
a trusted property contract enforced for every value, including before warm-up.
Cache invalidation alone does not establish expression safety. Adapters can use
`PreaggregatePlanner::expression_properties()` to check concrete values. Unknown
arbitrary expression inputs remain direct. Ordinary scalar inputs can be resolved
in a source plan before the filter marker and exposed as typed columns.
Suffix-only parameters remain finishing dependencies.

[`avenger-selection`](../avenger-selection) handles focus, self-exclusion,
selection factorization, pixel dimensions, and its full-selection fallback. It
uses this planner for query rewriting and validated runtime templates.
[`avenger-datafusion-dataflow`](../avenger-datafusion-dataflow) owns execution,
source versions, caching, in-progress work sharing, and cancellation.
