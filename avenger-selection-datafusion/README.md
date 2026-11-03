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
