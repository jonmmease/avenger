# avenger-selection-datafusion

Filters Apache DataFusion queries by avenger selections, with the membership rules of avenger-selection. A query calls `selected` to filter by a selection, and `bind_selections` lowers each call to an ordinary filter for the selection's state. The query then keeps exactly the rows that a renderer styles as selected.

## Registration

`register` adds `selected` to a session, so its queries can call it, and a function rewrite that fails a plan that runs with a `selected` call that `bind_selections` did not lower, with an error such as ``selection `brush` was not registered``:

```rust
use datafusion::prelude::SessionContext;

let mut ctx = SessionContext::new();
avenger_selection_datafusion::register(&mut ctx).unwrap();
```

## `selected`

`selected(state, exclude_own, owner, partition, empty, without)` returns whether each row is selected. `state` is a placeholder for the selection's state, which `bind_selections` binds, as [Binding states](#binding-states) describes:

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

## Binding states

`bind_selections` takes a plan and each selection's `SelectionState` by the name of its placeholder, and lowers each `selected` call on that placeholder to a filter of the state. The state never becomes a value of the plan, so planning does not copy or format the state's values:

```rust
use avenger_selection::SelectionState;
use datafusion::{arrow::array::RecordBatch, common::Result, prelude::SessionContext};

async fn counts(ctx: &SessionContext, brush: &SelectionState) -> Result<Vec<RecordBatch>> {
    let sql = "SELECT carrier, count(*) FROM flights WHERE selected($brush) GROUP BY carrier";
    let plan = ctx.sql(sql).await?.into_unoptimized_plan();
    let plan = avenger_selection_datafusion::bind_selections(plan, &[("brush", brush)])?;
    ctx.execute_logical_plan(plan).await?.collect().await
}
```

`with_param_values` binds the plan's other placeholders afterwards. A `selected` call whose placeholder has no given state stays in the plan, which fails when it runs with an error that names the selection.

Bind the unoptimized plan, as `into_unoptimized_plan` gives it. A `selected` call names no columns until it is bound, so the optimizer's projection pushdown can drop the columns that its conditions read, and binding then fails for a column that the input does not have. The lowered call takes those columns as its arguments, so the optimizer keeps them once the plan is bound.

## The lowering

`bind_selections` prepares avenger-selection's `selected` test for a call's state and options, `SelectionState::prepare_selected`, for the types of the columns of the call's input. A test that reads no row, such as for an empty selection, becomes `TRUE` or `FALSE`, so an empty selection keeps every row or none. Any other test becomes one row test, `selection_matches(columns..., partition)`, which carries the prepared test and runs `PreparedPredicate::mask` on each batch's columns, so a query selects the rows that `SelectionState::mask` selects for a renderer. Its arguments are the columns that the test reads and, for a faceted owner, the `partition` struct. A fingerprint of the test names the call and decides its equality.

Binding fails for a condition of a column that the input does not have, or that does not prepare for its columns' types.

The filter has no pruning predicates beside the row tests. Selections filter tables in memory, which have no statistics for a scan to skip data by, so a predicate would only add work for every row.

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

The rollup checks the keys with `in_keys($s, owner, keys)`, which `bind_selections` turns into a check of the owner's clauses, or `TRUE` when the owner has none. A key gives the same answer as its row, because each key is the row's cell or value.

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
