# avenger-selection-datafusion

Filters Apache DataFusion queries by avenger selections, with the membership rules of avenger-selection. A query calls `selected` to filter by a selection. `with_selection_nodes` replaces each call with a boolean column that a selection's operator computes, and binding gives the operator the selection's state, which it tests each row against. The query then keeps exactly the rows that a renderer styles as selected.

## Registration

`register` adds `selected` to a session, so its queries can call it. It also adds a function rewrite, which makes planning fail when a plan still contains a `selected` call that `with_selection_nodes` did not replace, with an error such as ``selection `brush` was not registered``.

The session's query planner must include `SelectionExtensionPlanner`, which plans the selections' operators. A host with a query planner of its own adds the extension planner to it, and a host without one installs the crate's `QueryPlanner`:

```rust
use std::sync::Arc;

use datafusion::{execution::SessionStateBuilder, prelude::SessionContext};

let state = SessionStateBuilder::new()
    .with_default_features()
    .with_query_planner(Arc::new(avenger_selection_datafusion::QueryPlanner))
    .build();
let mut ctx = SessionContext::new_with_state(state);
avenger_selection_datafusion::register(&mut ctx).unwrap();
```

## `selected`

`selected(state, exclude_own, owner, partition, empty, without)` returns whether each row is selected. `state` is a placeholder for the selection's state, which binding gives, as [Selection operators](#selection-operators) describes:

```sql
SELECT carrier, count(*) FROM flights WHERE selected($brush) GROUP BY carrier
```

| Parameter | When `NULL` or omitted | Meaning |
|---|---|---|
| `state` | Required | A placeholder for the selection's state |
| `exclude_own` | `FALSE` | Skip the clauses of the row's own plot, which `owner` and `partition` identify |
| `owner` | No owner | The id of the plot that draws the rows |
| `partition` | An unfaceted owner | The row's partition columns, as a struct whose fields are named by column, such as `named_struct('island', island)` |
| `empty` | `'all'` | What an empty selection selects: `'all'` or `'none'` |
| `without` | No owners | A list of owners whose clauses the call skips |

DataFusion matches named arguments to the parameters in this order, and a call must give every parameter before the last one that it uses. A call can leave out trailing parameters, so `selected($s)` and `selected($s, exclude_own => true, owner => 'distance')` work. A call that needs a later parameter passes `NULL` for the earlier ones to keep their defaults: `selected($s, NULL, NULL, NULL, 'none')`. Each option must be a constant: a literal, or an expression of literals, such as an array literal.

A cross filter passes the plot that draws the rows as `owner` with `exclude_own`, so a gesture in a plot does not filter that plot's own rows. For a faceted plot, `partition` gives the row's panel, so a row skips only the clause of its own panel.

## Selection operators

A query runs in two steps. `with_selection_nodes` replaces each `selected` call in the logical plan with a selection's node, which is planned as an operator, and binding gives each operator the state of its selection.

### Adding the nodes

`with_selection_nodes` takes the unoptimized plan, as `into_unoptimized_plan` gives it, and replaces each `selected` call on a placeholder with the column `__selected_<hash>` of a `SelectionNode`. The node becomes the input of the operator that contained the call. It passes its input's rows through and appends the column, which tells whether each row is selected. The new plan has the schema of the original plan.

A `SelectionFields` map names the columns that each selection's test can read: the columns that the tools that write the selection test. The node reads those columns, or every column of its input when the map does not name the selection, so the optimizer keeps them below the node.

### Binding

The planner plans each node as a `SelectionExec`. Binding the operator to a state prepares avenger-selection's `selected` test, `SelectionState::prepare_selected`, for the types of the operator's columns. The bound operator runs `PreparedPredicate::mask` on each batch, so a query selects the rows that `SelectionState::mask` selects for a renderer. An operator is bound in one of two places:

- During planning, when the session's configuration holds `SelectionStates`, which `with_selection_states` adds. A DataFrame plans itself when it runs, so it binds this way:

```rust
use std::collections::BTreeMap;

use avenger_selection::SelectionState;
use avenger_selection_datafusion::{with_selection_nodes, with_selection_states, SelectionFields};
use datafusion::{
    arrow::array::RecordBatch, common::Result, dataframe::DataFrame, prelude::SessionContext,
};

async fn counts(ctx: &SessionContext, brush: SelectionState) -> Result<Vec<RecordBatch>> {
    let sql = "SELECT carrier, count(*) FROM flights WHERE selected($brush) GROUP BY carrier";
    let plan = ctx.sql(sql).await?.into_unoptimized_plan();
    let plan = with_selection_nodes(plan, &SelectionFields::new())?;
    let state = with_selection_states(&ctx.state(), BTreeMap::from([("brush".into(), brush)]));
    DataFrame::new(state, plan).collect().await
}
```

- After planning, with `bind_selections`, which binds each `SelectionExec` of a physical plan. A host that plans a query once and runs it for many states binds each run this way. It then passes the bound plan to DataFusion's `reset_plan_states`, so the run's operators share no execution state with an earlier run.

`with_param_values` binds the plan's other placeholders after `with_selection_nodes`. A `SelectionExec` without a state fails when it runs, with an error that names its selection. Binding fails for a state that tests a column that the operator does not read.

### What the optimizer does with a selection

The node blocks the optimizer's pushdown of predicates only for its own column. A filter on another column moves below the node, and a filter on the node's column stays above it. A filter by a selection therefore never moves below a join or into a scan's pushed filters, so every selection of a physical plan is a `SelectionExec` that `bind_selections` can find, and a brush does not prune files.

A `selected` call in a join's condition reads the columns of one side of the join, which the selection's fields name. `with_selection_nodes` fails for a call whose selection has no fields, or whose fields come from both sides.

## Pre-aggregation

The `preaggregate` module rewrites a query before a selection moves, so that a move reruns only a small rollup, as in Mosaic's cross filters. When the pointer enters a plot, call `prepare` with the query's unoptimized plan and a `Focus`. The focus names the selection's placeholder, the owner whose clause will move, and each dimension of the owner's tool, with its grid when the dimension is gridded.

`prepare` returns a `Prepared`: the rewritten plan and a `Strategy` for each `selected` call on the selection. A query can have such calls in any number of its plots.

- `Preaggregated`: the aggregate above the call becomes a **rollup** over a **cells subplan**. The cells subplan aggregates the rows into states, grouped by the query's groups and one key for each dimension. The rollup merges the states of the keys that the moving clause covers. The `Preaggregation` holds the aggregate that it replaces, the cells subplan, and the rollup.
- `Unaffected`: the call skips the moving owner's clauses, so a move does not change its rows.
- `Direct`: the query runs as written, and the strategy gives the reason.

### Caching the cells

The cells subplan skips the moving owner's clauses with `without`, so it is the same for every position of the moving clause. A caller that keeps the results of subplans computes the cells once, from `Preaggregation::cells`, and reruns only the rollups on each move. `selection_reads` shows where to cut the plan: the cells subplan reads every owner except the moving one, and the rollup's `in_keys` call reads only the moving owner. Cut the bound plan before you optimize it, because the optimizer moves the rollup's filter into the cells subplan.

The rollup returns the rows of the aggregate that it replaces, `Preaggregation::aggregate`. A caller can therefore keep the rollup's result under the aggregate's key and reuse it for another focus or for the direct plan.

### Keys

Each focused dimension gives one key of the cells table:

- For a gridded range, or each column of a polygon with cells, the key is the column's cell in its grid, from a `pixel_cell` call that carries the grid.
- For each column of a set, such as the key columns of a point selection, the key is the column itself.

A bar chart grouped by two columns therefore gives two keys, and a box and a lasso over the same two grids share one cells table.

The rollup checks the keys with `in_keys($s, owner, keys)`, which `preaggregate::bind_keys` turns into a check of the owner's clauses, or `TRUE` when the owner has none. A key gives the same answer as its row, because each key is the row's cell or value.

### Which plans qualify

A call is pre-aggregated when all of these hold:

1. Between the filter that holds the `selected` call and the aggregate, there are only projections, filters, and aliases of immutable single-row expressions, and no other expression reads the selection.
2. Every aggregate has an aggregate-state family (`count`, `sum`, `min`, `max`, `avg`, and the variance and standard deviation functions), without `DISTINCT`, an aggregate `ORDER BY`, or grouping sets. `FILTER` clauses are allowed.
3. The call keeps the default `empty => 'all'`, and each focused column reaches the aggregate unchanged.

The operators above the aggregate, such as a mark aggregate, a window, or a limit, stay as they are, because the rollup has the aggregate's schema.

### The guard and the contract

`Focus::serves(state)` says whether the focus's rollups can answer a state. The answer depends only on the focus, so it is the same for every rollup. When the moving owner has clauses, the selection must resolve by intersection, and each of the owner's conditions must be one of these:

- A gridded range with the focus's grid.
- A polygon with cells on the focus's grids of both of its columns.
- A set whose columns are value keys of the focus.

When the rollups cannot answer, for example after a resize, run the direct plan. For every state that the guard accepts, the prepared plan returns the same rows as the direct plan. Only the rounding of floating-point aggregates can differ, because the rollup adds the rows in another order.

`Focus::size` bounds the rows of a cells table for each group: the product of the gridded keys' cell counts over their scales' ranges, times the number of distinct values of the value keys, which the caller estimates and uses to decide whether the table is worth its cost.
