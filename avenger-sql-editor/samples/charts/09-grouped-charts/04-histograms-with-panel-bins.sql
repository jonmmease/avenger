-- The same histograms, with the bins of each island, from `OVER panel`.
FROM samples.penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE mark_bar(x => bin(body_mass_g, maxbins => 20) OVER panel, y => count(*))
   GROUP BY island
|> ORDER BY island
