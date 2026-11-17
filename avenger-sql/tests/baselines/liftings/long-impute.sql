FROM penguins
|> WHERE body_mass_g IS NOT NULL
|> EXTEND bin(body_mass_g) OVER () AS b
|> EXTEND b.start AS mass
|> AGGREGATE SUM(flipper_length_mm) AS `SUM(flipper_length_mm)` GROUP BY island, mass, species, sex
|> CALL impute(
     `mass`,
     [`species`, `sex`],
     `SUM(flipper_length_mm)`,
     value => 0,
     partition_by => [`island`],
   )
|> EXTEND stack(`SUM(flipper_length_mm)`) OVER (PARTITION BY `island`, `mass` ORDER BY `species` DESC NULLS FIRST, `sex` DESC NULLS FIRST) AS `stack(SUM(flipper_length_mm))`
|> SELECT `island`, `mass`, `stack(SUM(flipper_length_mm))` AS `SUM(flipper_length_mm)`, `species`, `sex`
|> AGGREGATE area(
     x => `mass`,
     y => `SUM(flipper_length_mm)`,
     color => `species`,
     stroke => `sex`,
   ) AS `chart`
   GROUP BY `island`
