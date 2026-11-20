FROM penguins
|> AGGREGATE COUNT(*) AS `COUNT(*)` GROUP BY species
|> SELECT `species`, `COUNT(*)`
|> AGGREGATE mark_bar(x => `species`, y => `COUNT(*)`) AS `chart`
