-- `layer_agg`, `hconcat_agg`, and `vconcat_agg` compose the charts of a group, in the order of an
-- `ORDER BY` in the call. The species of each island, one above another, without the labels of a
-- facet.
FROM samples.penguins
|> AGGREGATE mark_bar(x => species, y => count(*), color => species) AS chart GROUP BY island
|> AGGREGATE vconcat_agg(chart ORDER BY island) AS chart
