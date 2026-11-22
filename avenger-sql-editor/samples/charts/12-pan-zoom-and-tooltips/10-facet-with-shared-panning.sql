-- A facet whose panels pan, zoom, and show tooltips. The panels share their scales, so a pan in one
-- moves them all.
FROM samples.penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE mark_symbol(
     x => x(bill_length_mm, scale => scale(zero => FALSE)),
     y => y(bill_depth_mm, scale => scale(zero => FALSE)),
     color => species,
     tools => [navigate(), tooltip()],
   ) AS chart
   GROUP BY island
|> AGGREGATE facet_column(chart, island) AS chart
