-- A layer whose plot pans, zooms, and shows the tooltips of both of its layers, from the `tools` of
-- `layer`.
FROM samples.penguins
|> AGGREGATE layer(
     mark_bar(x => species, y => y(avg(body_mass_g), title => 'Mean body mass (g)')),
     mark_rule(y => y(avg(body_mass_g), title => 'Mean body mass (g)'), stroke_width => 2),
     tools => [navigate(), tooltip()],
   )
