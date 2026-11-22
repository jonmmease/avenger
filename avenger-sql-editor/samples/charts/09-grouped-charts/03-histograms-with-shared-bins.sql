-- A histogram of body mass on each island, in at most as many bins as the variable `bins`. The bins
-- are those of all the penguins, so every histogram has the same bin edges.
DECLARE bins FLOAT64 DEFAULT 20;
FROM samples.penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE mark_bar(x => bin(body_mass_g, maxbins => bins), y => count(*)) GROUP BY island
|> ORDER BY island
