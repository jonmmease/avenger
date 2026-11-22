-- The penguins of each species, stacked by sex.
FROM samples.penguins
|> AGGREGATE mark_bar(x => species, y => stack(count(*)), color => sex)
