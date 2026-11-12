FROM penguins |> WHERE body_mass_g IS NOT NULL |> AGGREGATE count(*) AS n
   GROUP BY species
|> AGGREGATE
     bar(x => species, y => scaled(n, title => 'count(*)')) AS bar
