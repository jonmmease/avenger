-- Bill depth against bill length on each island, with the number of penguins of each island from an
-- aggregate item.
FROM samples.penguins
|> AGGREGATE
     count(*) AS penguins,
     mark_symbol(x => bill_length_mm, y => bill_depth_mm, color => species) AS chart,
   GROUP BY island
|> ORDER BY penguins DESC
