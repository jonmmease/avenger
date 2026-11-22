-- The bills and the counts of the species on each island, a row for each island, from an `hconcat`
-- for each island in a `facet_row`.
FROM samples.penguins
|> AGGREGATE hconcat(
     mark_symbol(x => bill_length_mm, y => bill_depth_mm, color => species),
     mark_bar(x => species, y => count(*), color => species),
   ) AS chart
   GROUP BY island
|> AGGREGATE facet_row(chart, island) AS chart
