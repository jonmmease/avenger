FROM penguins
|> WHERE body_mass_g IS NOT NULL
|> EXTEND bin(body_mass_g) OVER () AS `bin(body_mass_g)`
|> AGGREGATE COUNT(*) AS `COUNT(*)` GROUP BY island, `bin(body_mass_g)` AS `body_mass_g`
|> SELECT `island`, `body_mass_g`, `COUNT(*)`
|> AGGREGATE bar(x => `body_mass_g`, y => `COUNT(*)`) AS `chart` GROUP BY `island`
|> AGGREGATE facet_col(chart, island) AS chart
