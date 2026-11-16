FROM penguins
|> AGGREGATE COUNT(*) AS `COUNT(*)` GROUP BY island, species
|> EXTEND SUM(`COUNT(*)`) OVER (PARTITION BY `island`) AS `SUM(COUNT(*)) OVER panel`
|> SELECT `island`, `species`, `COUNT(*)` / `SUM(COUNT(*)) OVER panel` AS `COUNT(*) / SUM(COUNT(*)) OVER panel`
|> AGGREGATE symbol(x => `species`, y => `COUNT(*) / SUM(COUNT(*)) OVER panel`) AS `chart` GROUP BY `island`
