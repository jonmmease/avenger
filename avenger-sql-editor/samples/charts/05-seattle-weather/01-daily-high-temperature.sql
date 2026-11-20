-- The highest temperature of each day.
FROM samples.seattle_weather
|> AGGREGATE mark_line(x => date, y => temp_max)
