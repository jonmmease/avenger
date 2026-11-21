-- A histogram of body mass.
FROM samples.penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE mark_bar(
     x => x(bin(body_mass_g), title => 'Body mass (g)'),
     y => y(count(*), title => 'Penguins'),
   )
