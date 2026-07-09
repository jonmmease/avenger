# avenger-datafusion-partial-eval

Logical-plan partial evaluation for DataFusion plans.

This crate executes deterministic, parameter-independent `LogicalPlan`
subtrees once and splices their results back into the residual plan as
in-memory `MemTable` scans. Placeholders, volatile functions, now-family
functions, and policy-excluded tables stay symbolic.

It is intentionally DataFusion-only. It does not import Avenger crates, unparse
SQL, manage freshness, spill to disk, or implement the chart-level
`CompiledPlot::pre_evaluate` integration.

## Example

```rust,no_run
use datafusion::prelude::SessionContext;
use avenger_datafusion_partial_eval::{partial_evaluate, PartialEvalPolicy};

# async fn example(ctx: SessionContext) -> datafusion::error::Result<()> {
let plan = ctx
    .sql(
        "SELECT * FROM (
            SELECT region, SUM(value) AS total
            FROM source
            GROUP BY region
        ) q
        WHERE total > $threshold",
    )
    .await?
    .logical_plan()
    .clone();

let output = partial_evaluate(plan, &ctx, &PartialEvalPolicy::default()).await?;

// `output.residual` still contains `$threshold`, but the param-independent
// aggregate can be baked into `output.report.baked`.
println!("baked subtrees: {}", output.report.baked.len());
# Ok(())
# }
```

See `avenger-chart/docs/future-work/logical-plan-partial-evaluation.md` for the
design background and v1 narrowing notes.
