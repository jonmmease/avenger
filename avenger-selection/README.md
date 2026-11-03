# avenger-selection

Selections of rows as data, and the Arrow logic that decides which rows a selection holds. A plot's selection tool, such as a brush, writes a **clause** to a selection, and one set of rules decides which rows the selection's clauses select. A renderer applies the rules to style rows, and a query layer, such as avenger-selection-datafusion, applies the same rules to filter rows, so the two always agree. The crate depends on Arrow and avenger-scales, not on DataFusion or on a chart format.

## Terminology

| Term | Meaning |
|---|---|
| Selection | A named list of clauses, which its resolution combines |
| Clause | The part of a selection that one owner, such as a plot, wrote: conditions that a row matches when it matches all of them |
| Condition | A symbolic check of one or more columns, which evaluates to a boolean for each row |
| Resolution | How a selection combines its owners' clauses: `Global` keeps one clause at a time, a row matches a `Union` selection when it matches one owner's clause, and an `Intersect` selection when it matches every owner's clause |
| Predicate | A tree that combines selections: `Selected` leaves, each naming a selection, under `And`, `Or`, and `Not` |

A condition is not a mask. It holds its columns and the values, ends, cells, or polygons that a row's values must fit, so it can be drawn as a brush and lowered to SQL. `Condition::matches` evaluates it over Arrow columns.

## Clauses and conditions

A `Clause` holds its owner's id, the owner's partition values when the plot is a panel of a facet, and its conditions. A row matches the clause when it matches every condition. A `SelectionState` holds a selection's resolution and its clauses.

| Condition | A row matches when |
|---|---|
| `Set` | Its values in the set's columns equal one of the set's tuples |
| `Range` | Its value is from the first end through the second, or to before the second with `exclude_end` |
| `Range` with cells | Its value lies in one of the span's cells, from `first` through `last` |
| `Polygon` | The point of its two values is inside one of the polygons, by the even-odd rule for each polygon |
| `Polygon` with cells | The point's cell in each of the two grids is one of the cells that the lasso covers |
| `CyclicRange` | Its value is from the start forward to the end, in a dimension that wraps after a period, such as longitude |

A null matches no condition. A set's values and a range's ends are cast to the columns' types, and a dictionary column compares by its values. A set holds one list of values for each of its columns, all of the same length, and its tuples are the rows of those lists. A set of two columns therefore selects pairs, such as the clicked bars of a grouped chart, and not every combination of their values. A polygon condition holds several polygons, so one condition covers several lassos.

A set, a range, and a cyclic range record, for each column, the label of the channel that the selection came through, such as `x`. A renderer reads the label to give a scale the domain of a selection.

## Pixel grids

A `PixelGrid` divides the range of a plot dimension's scale into cells of a fixed width. It holds the scale at the time of a gesture, the origin of the scale's range, and the cell width in the units of the range, which are logical pixels for a plot's positions. A value's cell is `floor((s(v) − origin) / cell)`, where `s(v)` is the value's position through the scale.

A brush that selects whole cells writes a range with a `CellSpan`: the grid and the first and last cells that the box covers. The cells, not the box's edges, then decide membership, so a query that groups rows by their cells keeps exactly the rows that the renderer styles.

A lasso that selects whole cells writes a polygon condition with `PolygonCells`: a grid for each of the plot's two dimensions, and the cells whose centers are inside the lasso's paths by the even-odd rule, which `PolygonCells::cover` computes once from the paths in pixels. The cells are runs of the first dimension's cells for each row of the second dimension's cells. A point matches when its cell in each grid is one of them, so membership needs no edge tests, and a query can group rows by the pair of their cells.

## Prepared conditions

`Condition::prepare` builds what matching a condition reads, once, for the types of its columns: a set's index of its tuples, a polygon's edges by horizontal band, and a lasso's bitmap of its cells. `PreparedCondition::matches` then matches each array, such as each batch of a query, against them, and `Condition::matches` prepares and matches in one call. A set indexes each column's distinct values: integer and temporal columns with a table over their range or a hash table, string and binary columns with a hash table, and a dictionary column through its dictionary's values, once for each array. A set of several columns looks up the packed positions of each row's values. A set of other types, such as floats, matches through Arrow's row format.

## Matching

`matching` takes a `Predicate`, the clauses of each selection by name, and the resolution of each selection, and returns a `Matching` tree whose leaves are conditions. `Matching::mask` evaluates the tree over rows, with a function that the caller gives to match one condition.

- A selection without clauses selects every row, or none with `empty_selects_none`.
- A `Selected` with `exclude_own` skips the clauses of the row's own plot, which the caller's `owned` callback identifies. When it skips every clause of a selection, the selection selects every row.
- An intersect selection selects the rows that every owner's clause selects. A union or a global selection selects the rows that some owner's clause selects.
- `And`, `Or`, and `Not` combine predicates. An `Or` of no predicates selects no row, and an `And` of none selects every row.

```rust
use std::{collections::BTreeMap, sync::Arc};

use arrow::array::{ArrayRef, BooleanArray, Float64Array};
use avenger_selection::{matching, Clause, Condition, Predicate, Resolution, Selected};

let brush = Clause::new("scatter").condition(Condition::Range {
    column: "x".into(),
    channel: Some("x".into()),
    ends: Arc::new(Float64Array::from(vec![2.0, 3.0])),
    exclude_end: false,
    cells: None,
});
let clauses = BTreeMap::from([("brush".to_string(), vec![brush])]);
let resolutions = BTreeMap::from([("brush".to_string(), Resolution::Intersect)]);
let predicate = Predicate::Selected(Selected {
    selection: "brush".into(),
    empty_selects_none: false,
    exclude_own: false,
});
let rows = matching(&predicate, &resolutions, &clauses, &|_| false);

let x: ArrayRef = Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0]));
let mask = rows
    .mask(4, &mut |_, condition| condition.matches(std::slice::from_ref(&x)))
    .unwrap();
assert_eq!(mask, BooleanArray::from(vec![false, true, true, false]));
```

## The `selected` test

`SelectionState::mask` gives whether each row of a batch is selected for a `SelectedOptions`, the options of avenger-selection-datafusion's `selected`, so a renderer and a query select the same rows through one function:

| Option | Meaning |
|---|---|
| `exclude_own` | The owner of the rows, whose clauses the test skips, as a cross filter skips the plot that draws the rows |
| `faceted` | Each row gives its panel by the partition columns of the owner's clauses, so a row skips only the clause of its own panel |
| `empty_selects_none` | A selection without clauses selects no row, rather than every row |
| `without` | The owners whose clauses the test skips |

`SelectionState::prepare_selected` prepares the test once for the types of its columns, and `PreparedSelected::mask` then matches each batch, with the conditions' columns and the panels' partition columns given by name. A row of a faceted owner's panel takes the panel's test, which skips the clause of that panel, and a row of no panel, or of a null panel, keeps every clause of the owner. A panel's values compare with the row's partition columns as `IS NOT DISTINCT FROM` does. `PreparedSelected::constant` gives the answer of a test that reads no row, such as for an empty selection, and the test hashes by its content, so a query can name it and compare its calls.

## Example

The example `tour` follows a dashboard of penguins, a scatter plot faceted by island and a bar chart of species, through clauses, resolutions, the options of the `selected` test, facets, pixel grids, prepared tests, and predicates over several selections, and prints the rows that each selection selects:

```bash
cargo run --release -p avenger-selection --example tour
```
