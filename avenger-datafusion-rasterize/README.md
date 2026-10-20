# avenger-datafusion-rasterize

Pixel rasters of a DataFusion DataFrame's rows, such as point density heatmaps and M4 line
decimation, with any DataFusion aggregate for each pixel cell.

## Query

`Rasterize::new(x, scale, width)` bins rows into `width` columns by the expression `x` through
an avenger-scales scale, in a raster one row high. `with_y(y, scale, height)` bins rows into
`height` rows as well. `aggregate(expr)` adds an aggregate expression, whose results fill the
raster's next plane, and `group_by(expr)` makes one raster for each value of `expr`.

`Rasterize::plan(df)` returns a DataFrame that aggregates `df`'s rows by the group-by
expressions, with one column for each aggregate. Each aggregate runs as a raster aggregate that
wraps it: each row's pixels give its cell, and the cell is the row's group in the inner
aggregate's own DataFusion groups accumulator, so no hash table groups the rows. Each partition
holds a dense grid of the inner aggregate's state for each group, and the final aggregation
merges the partitions' grids through the inner aggregate's own merge. So any aggregate works,
such as `count`, `avg`, a `first_value` ordered by another column, or a percentile, and its
`FILTER`, `ORDER BY`, and null treatment apply within each cell. DISTINCT aggregates, and
aggregates that need their rows in order other than WITHIN GROUP aggregates such as
percentiles, aren't supported. An aggregate without a groups accumulator works through
DataFusion's adapter, which keeps an accumulator for each cell.

The grids are dense, so a query holds a grid for each partition and group, whatever the rows
reach. So group-by expressions take few values, such as facets. A query whose groups are nearly
as many as its rows fails, since DataFusion then passes rows to the final aggregation as their
own groups, which a raster can't do. The plan runs wherever the DataFrame runs, and `collect` gives the batches for packing.

## Cells

- The query sets each scale's range to the grid's pixels: x from 0 at the left to `width`, and
  y from `height` at the bottom to 0 at the top. The scales keep their domains and their other
  options, so a log scale bins equal ratios into equal widths.
- The query turns each scale's `clamp` option off, so rows outside the scales' domains, and rows
  without values, count nowhere.
- A row's cell is its floored pixels. A row at the grid's far edge, a domain's upper end for x or
  its lower end for y, counts in the last cell.

## Packing

`Rasterize::pack(batches)` turns the plan's collected batches into one `Raster` for each group,
ordered by the groups' values, or one `Raster` when the query has no group-by expressions. A
raster holds its group's values, its size, and one `Float64Array` plane for each aggregate, in
the query's order. Row 0 is the top, and the cell at column `c` and row `r` is at
`r * width + c`. A cell that no row reached holds the aggregate's value over no rows: 0 for
`count`, and no value for aggregates such as `avg` and `first_value`.
