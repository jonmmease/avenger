FROM penguins |> WHERE body_mass_g IS NOT NULL |> EXTEND bin(body_mass_g) OVER () AS b |> EXTEND b.start AS mass
|> AGGREGATE COUNT(*) AS `COUNT(*)` GROUP BY mass, species
|> CALL impute(`mass`, [`species`], `COUNT(*)`, value => 0)
|> EXTEND stack(`COUNT(*)`) OVER (PARTITION BY `mass` ORDER BY `species` DESC NULLS FIRST) AS `stack(COUNT(*))`
|> SELECT `mass`, `stack(COUNT(*))` AS `COUNT(*)`, `species`
|> AGGREGATE area(x => `mass`, y => `COUNT(*)`, color => `species`) AS `chart`
