FROM penguins |> WHERE body_mass_g IS NOT NULL |> AGGREGATE count(*) AS n
   GROUP BY species
|> EXTEND
     stack(n) OVER (ORDER BY species ASC NULLS LAST, n ASC NULLS LAST) AS n_stack
|> AGGREGATE
     polar_bar(
       theta => scaled(n_stack, title => 'count(*)'),
       color => species,
       orient => 'horizontal') AS polar_bar
