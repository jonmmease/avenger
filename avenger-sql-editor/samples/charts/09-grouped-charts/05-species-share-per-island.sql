-- The share of each species on each island, from a window of the counts of each panel.
FROM samples.penguins
|> AGGREGATE mark_bar(
     x => species,
     y => y(CAST(count(*) AS FLOAT64) / sum(count(*)) OVER panel, title => 'Share'),
   )
   GROUP BY island
|> ORDER BY island
