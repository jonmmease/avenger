-- The precipitation of the warm days.
IMPORT MODULE weather;

FROM vega_datasets.seattle_weather
|> WHERE weather.warm(temp_max)
|> AGGREGATE mark_symbol(x => temp_max, y => precipitation) AS chart
