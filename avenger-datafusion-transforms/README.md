# avenger-datafusion-transforms

Vega-style data transforms as Apache DataFusion functions: `bin` and `stack`. Each transform is a window function. Optimizer rules plan the window functions as aggregates and joins.

```sql
SELECT *, bin(x, 'maxbins', 30) OVER (PARTITION BY g) AS binned FROM t
SELECT *, stack(v, 'offset', 'center') OVER (PARTITION BY x ORDER BY series) AS y FROM t
```

`register` adds the functions and the rules to a `SessionContext`.

| Transform | Result | Notes |
|---|---|---|
| `bin` | `struct<start, end>` | Bins of a nice step, for at most `maxbins` bins, over the extent of each window partition or an explicit `extent`. `bin_params(lo, hi, ...)` gives the bins of an extent. |
| `stack` | `struct<start, end>` | The layers of each stack, with the offsets `zero`, `normalize`, and `center`, which centers each stack on zero. |

The options of the window functions are pairs of a name and a constant after the first argument, because DataFusion 55's named arguments can omit only the last parameters.

## Tests

The tests state what each transform does: bins and stacks of chosen values and options, and the rules against the window evaluators.
