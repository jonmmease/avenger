-- The days of each kind of weather, from the most.
FROM samples.seattle_weather
|> AGGREGATE mark_bar(x => weather, y => count(*), color => weather ORDER BY count(*) DESC)
