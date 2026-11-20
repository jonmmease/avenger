-- The share of the penguins on each island, from a window of the counts.
FROM samples.penguins
|> AGGREGATE mark_bar(
     x => island,
     y => y(CAST(count(*) AS FLOAT64) / sum(count(*)) OVER (), title => 'Share of penguins'),
   )
