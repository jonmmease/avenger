-- The share of each island in the penguins of each species, from the offset `normalize`.
FROM samples.penguins
|> AGGREGATE mark_bar(
     x => species,
     y => y(stack(count(*), offset => 'normalize'), title => 'Share'),
     color => island,
   )
