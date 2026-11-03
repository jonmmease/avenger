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

## The test of a selection

`SelectionState::mask` gives whether each row of a batch is selected for a `SelectedOptions`, the options of avenger-selection-datafusion's `selected`, so a renderer and a query select the same rows through one function:

| Option | Meaning |
|---|---|
| `exclude_own` | The owners of the rows, such as the plot that draws them and the plots that hold it, whose clauses the test skips, as a cross filter skips the plot that draws the rows |
| `faceted` | Each row gives its panel by the partition columns of the owners' clauses, so a row skips only the clause of its own panel |
| `empty_selects_none` | A selection without clauses selects no row, rather than every row |
| `without` | The owners whose clauses the test skips |

The rules:

- A selection without clauses selects every row, or none with `empty_selects_none`. A selection whose clauses the test skips, all of them, selects every row.
- An intersect selection selects the rows that every kept owner's clause selects. A union or a global selection selects the rows that some kept clause selects. A row matches a clause when it matches every condition of the clause.
- A row of a faceted owner's panel skips the clause of that panel, and a row of no panel, or of a null panel, keeps every clause of the owner. A panel's values compare with the row's partition columns as `IS NOT DISTINCT FROM` does.

## Predicates

A `Predicate` combines the tests of several selections, each `Selected` leaf with its own options, under `And`, `Or`, and `Not`. An `Or` of no predicates selects no row, and an `And` of none selects every row. `Predicate::mask` matches a batch against the states of the selections by name, and a selection without a state has no clauses:

```rust
use std::{collections::BTreeMap, sync::Arc};

use arrow::array::{ArrayRef, BooleanArray, Float64Array, RecordBatch, StringArray};
use avenger_selection::{Clause, Condition, Predicate, Resolution, SelectedOptions, SelectionState};

let brush = Clause::new("scatter").condition(Condition::Range {
    column: "x".into(),
    channel: Some("x".into()),
    ends: Arc::new(Float64Array::from(vec![2.0, 3.0])),
    exclude_end: false,
    cells: None,
});
let click = Clause::new("bars").condition(Condition::values(
    "species",
    None,
    Arc::new(StringArray::from(vec!["gentoo"])),
));
let states = BTreeMap::from([
    ("brush".to_string(), SelectionState::new(Resolution::Global, vec![brush])),
    ("click".to_string(), SelectionState::new(Resolution::Global, vec![click])),
]);
// The brushed rows without the clicked species.
let options = SelectedOptions::default();
let predicate = Predicate::And(vec![
    Predicate::selected("brush", options.clone()),
    Predicate::Not(Box::new(Predicate::selected("click", options))),
]);

let rows = RecordBatch::try_from_iter([
    ("x", Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0])) as ArrayRef),
    ("species", Arc::new(StringArray::from(vec!["adelie", "gentoo", "adelie", "adelie"]))),
])
.unwrap();
let mask = predicate.mask(&states, &rows).unwrap();
assert_eq!(mask, BooleanArray::from(vec![false, false, true, false]));
```

`Predicate::prepare` and `SelectionState::prepare_selected` prepare a test once for the types of its columns, each condition once, and `PreparedPredicate::mask` then matches each batch, with the conditions' columns and the panels' partition columns given by name, so a renderer that reads a column from elsewhere, such as a panel's partition value, gives it there. `PreparedPredicate::columns` names the columns that the test reads, and `PreparedPredicate::constant` gives the answer of a test that reads no row, such as for an empty selection. `PreparedPredicate::fingerprint`, `Clause::fingerprint`, and `PixelGrid::fingerprint` hash their content, so a query can name and compare the calls that carry them.

## Example

The example `tour` follows a dashboard of penguins, a scatter plot faceted by island and a bar chart of species, through clauses, resolutions, the options of the `selected` test, facets, pixel grids, prepared tests, and predicates over several selections, and prints the rows that each selection selects:

```bash
cargo run --release -p avenger-selection --example tour
```
