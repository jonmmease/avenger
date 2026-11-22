-- The highest temperature of each day, a year to a row, grouped by an expression.
FROM samples.seattle_weather
|> AGGREGATE mark_line(x => date, y => temp_max) GROUP BY EXTRACT(YEAR FROM date) AS year
|> ORDER BY year
