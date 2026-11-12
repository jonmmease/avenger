FROM penguins |> WHERE body_mass_g IS NOT NULL |> AGGREGATE count(*) AS n
   GROUP BY island, species
|> EXTEND sum(n) OVER (PARTITION BY island) AS sum_n_window
|> AGGREGATE
     symbol(
       x => species,
       y => scaled(
         n / sum_n_window,
         title => 'count(*) / sum(count(*)) OVER panel')) AS symbol
   GROUP BY island
