-- The penguins of each species, stacked by island with the largest group at the bottom.
FROM samples.penguins
|> AGGREGATE mark_bar(
     x => species,
     y => stack(count(*)) OVER (position ORDER BY count(*) DESC),
     color => island,
   )
