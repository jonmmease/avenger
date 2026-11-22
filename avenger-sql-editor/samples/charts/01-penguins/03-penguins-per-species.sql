-- The number of penguins of each species.
FROM samples.penguins
|> AGGREGATE mark_bar(x => species, y => count(*))
