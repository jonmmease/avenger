-- Bill depth against bill length, a panel for each island.
FROM samples.penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE mark_symbol(
     x => x(bill_length_mm, scale => scale(zero => FALSE)),
     y => y(bill_depth_mm, scale => scale(zero => FALSE)),
     color => species,
   ) AS chart
   GROUP BY island
|> AGGREGATE facet_column(chart, island) AS chart
