-- A table of charts with tools, one for each species. Check "Interactive charts" to pan, zoom, and
-- see the tooltips in place.
FROM samples.penguins
|> AGGREGATE mark_symbol(
     x => x(bill_length_mm, scale => scale(zero => FALSE)),
     y => y(bill_depth_mm, scale => scale(zero => FALSE)),
     color => sex,
     tools => [navigate(), tooltip()],
   )
   GROUP BY species
|> ORDER BY species
