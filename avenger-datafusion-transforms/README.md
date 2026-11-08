# avenger-datafusion-transforms

Vega-style data transforms as Apache DataFusion functions: `bin`, `stack`, and `timeunit`. Each transform is a window function. Optimizer rules plan the window functions as aggregates and joins.

```sql
SELECT *, bin(x, 'maxbins', 30) OVER (PARTITION BY g) AS binned FROM t
SELECT *, stack(v, 'offset', 'center') OVER (PARTITION BY x ORDER BY series) AS y FROM t
SELECT *, timeunit(t, 'maxbins', 20) OVER (PARTITION BY g) AS u FROM events
```

`register` adds the functions and the rules to a `SessionContext`.

| Transform | Result | Notes |
|---|---|---|
| `bin` | `struct<start, end>` | Bins of a nice step, for at most `maxbins` bins, over the extent of each window partition or an explicit `extent`. `bin_params(lo, hi, ...)` gives the bins of an extent. |
| `stack` | `struct<start, end>` | The layers of each stack, with the offsets `zero`, `normalize`, and `center`, which centers each stack on zero. |
| `timeunit` | `struct<start, end, unit>` | A calendar unit in the time zone of the value's type, chosen for each window partition or given with `'unit'`. Units include fiscal quarters and years, and collapsing units such as `'month_of_year'`. |

The options of the window functions are pairs of a name and a constant after the first argument, because DataFusion 55's named arguments can omit only the last parameters. A `timeunit` call with a `'unit'` takes `OVER ()`: DataFusion plans a scalar function before a window function of the same name, so the scalar form is internal.

## Tests

The tests state what each transform does: bins, stacks, and time units of chosen values and options, time units that tile time across changes of the UTC offset, and the rules against the window evaluators.
