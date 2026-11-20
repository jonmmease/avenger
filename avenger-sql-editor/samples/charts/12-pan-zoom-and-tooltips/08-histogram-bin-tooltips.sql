-- A histogram with a tooltip of each bin.
FROM samples.penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE mark_bar(
     x => x(bin(body_mass_g, maxbins => 20), title => 'Body mass (g)'),
     y => y(count(*), title => 'Penguins'),
     tools => [navigate(channels => ['x']), tooltip()],
   )
