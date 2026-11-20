-- The same panels with an x scale for each, so each pans on its own.
FROM samples.penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE mark_symbol(
     x => x(bill_length_mm, scale => scale(zero => FALSE)),
     y => y(bill_depth_mm, scale => scale(zero => FALSE)),
     color => species,
     tools => [navigate(), tooltip()],
   ) AS chart
   GROUP BY island
|> AGGREGATE facet_column(
     chart,
     island,
     resolve => resolve(x => 'independent'),
   ) AS chart
