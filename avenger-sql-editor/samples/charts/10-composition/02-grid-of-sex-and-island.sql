-- A grid of panels, a row for each sex and a column for each island, from a `facet_col` for each
-- sex in a `facet_row`.
FROM samples.penguins
|> WHERE sex IS NOT NULL
|> AGGREGATE mark_symbol(
     x => x(bill_length_mm, scale => scale(zero => FALSE)),
     y => y(bill_depth_mm, scale => scale(zero => FALSE)),
     color => species,
   ) AS chart
   GROUP BY sex, island
|> AGGREGATE facet_column(chart, island) AS chart GROUP BY sex
|> AGGREGATE facet_row(chart, sex) AS chart
