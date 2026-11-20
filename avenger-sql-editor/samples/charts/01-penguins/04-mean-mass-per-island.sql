-- The mean body mass on each island, as horizontal bars from the heaviest.
FROM samples.penguins
|> AGGREGATE mark_bar(
     x => x(avg(body_mass_g), title => 'Mean body mass (g)'),
     y => island,
     ORDER BY avg(body_mass_g) DESC
   )
