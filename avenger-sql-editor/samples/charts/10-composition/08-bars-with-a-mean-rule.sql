-- `layer` draws its charts in one plot, each over the ones before it.
--
-- Bars of the mean body mass of each species, with a rule at the mean of all the penguins.
FROM samples.penguins
|> AGGREGATE layer(
     mark_bar(x => species, y => y(avg(body_mass_g), title => 'Mean body mass (g)')),
     mark_rule(y => y(avg(body_mass_g), title => 'Mean body mass (g)'), stroke_width => 2),
   )
