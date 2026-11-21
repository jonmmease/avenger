-- Stacked bars with a tooltip of each segment.
FROM samples.penguins
|> AGGREGATE mark_bar(x => species, y => stack(count(*)), color => island, tools => [tooltip()])
