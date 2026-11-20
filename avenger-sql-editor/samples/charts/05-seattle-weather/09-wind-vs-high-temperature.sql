-- The wind against the highest temperature, by weather.
FROM samples.seattle_weather
|> AGGREGATE mark_symbol(x => temp_max, y => wind, color => weather, opacity => 0.5)
