-- A scatter plot three times as wide as a bar chart of its species, from the tracks of an
-- `hconcat`.
FROM samples.penguins
|> AGGREGATE hconcat(
     mark_symbol(x => bill_length_mm, y => bill_depth_mm, color => species),
     mark_bar(x => count(*), y => species, color => species),
     gap => 30,
     tracks => [track(share => 3), track(share => 1)],
   )
