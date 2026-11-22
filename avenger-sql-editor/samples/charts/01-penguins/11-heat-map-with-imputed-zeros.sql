-- The penguins of each species and island as a heat map, with 0 for the pairs that have none, from
-- a pipe `CALL` of the table function `impute`.
FROM samples.penguins
|> AGGREGATE count(*) AS n GROUP BY species, island
|> CALL impute(island, [species], n, value => 0)
|> AGGREGATE mark_rect(x => island, y => species, color => n)
