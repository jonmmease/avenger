FROM penguins |> WHERE body_mass_g IS NOT NULL |> EXTEND 1 AS n |> AGGREGATE count(*) AS n_2
   GROUP BY species
|> AGGREGATE
     bar(x => species, y => scaled(n_2, title => 'count(*)')) AS bar
