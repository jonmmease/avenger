FROM penguins |> WHERE body_mass_g IS NOT NULL |> EXTEND upper(island) AS isle
|> AGGREGATE count(*) AS n
   GROUP BY isle, species
|> AGGREGATE
     bar(x => species, y => scaled(n, title => 'count(*)')) AS bar
   GROUP BY isle
