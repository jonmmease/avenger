FROM (
  FROM penguins
  |> AGGREGATE COUNT(*) AS __item_0_0 GROUP BY island
  |> AS __items
  |> LEFT JOIN (
       FROM penguins
       |> AGGREGATE AVG(body_mass_g) AS `AVG(body_mass_g)` GROUP BY island, species
       |> SELECT `island`, `species`, `AVG(body_mass_g)`
       |> AGGREGATE symbol(
            x => x(`species`, title => 'Species', axis => axis(grid => FALSE)),
            y => y(`AVG(body_mass_g)`, scale => scale(zero => FALSE)),
            color => `species`,
          ) AS __mark
          GROUP BY `island`
     ) AS __lifting_0 ON __items.`island` IS NOT DISTINCT FROM __lifting_0.`island`
  |> SELECT __items.`island` AS `island`, __items.__item_0_0 AS `n`, __lifting_0.__mark AS `chart`
)
