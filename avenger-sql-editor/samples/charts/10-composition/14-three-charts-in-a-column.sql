-- The penguins by species, by island, and by sex, one above another, with a title.
FROM samples.penguins
|> AGGREGATE vconcat(
     mark_bar(x => species, y => count(*)),
     mark_bar(x => island, y => count(*)),
     mark_bar(x => sex, y => count(*)),
     title => 'Penguins',
   )
