-- The mean body mass of each species above the mean of every penguin, which a query computes into
-- the variable `mean_mass`.
DECLARE mean_mass FLOAT64 DEFAULT (
  FROM samples.penguins
  |> AGGREGATE avg(body_mass_g)
);
FROM samples.penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE mark_bar(
     x => species,
     y => y(avg(body_mass_g) - mean_mass, title => 'Mean body mass above the mean (g)'),
   )
