FROM penguins
|> AGGREGATE COUNT(*) AS __item_0_0 GROUP BY island
|> AS __items
|> LEFT JOIN (
     FROM penguins
     |> AGGREGATE COUNT(*) AS `COUNT(*)` GROUP BY island, species
     |> SELECT `island`, `species`, `COUNT(*)`
     |> AGGREGATE bar(x => `species`, y => `COUNT(*)`) AS __mark GROUP BY `island`
   ) AS __lifting_0 ON __items.`island` IS NOT DISTINCT FROM __lifting_0.`island`
|> SELECT __items.`island` AS `island`, __items.__item_0_0 AS `n`, __lifting_0.__mark AS `chart`
