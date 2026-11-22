-- The days of each kind of weather in each month, as lines with 0 for the months without that
-- weather, from `impute => 0`.
FROM samples.seattle_weather
|> AGGREGATE count(*) AS n GROUP BY date_trunc(date, MONTH) AS month, weather
|> CALL impute(month, [weather], n, value => 0)
|> AGGREGATE mark_line(
     x => x(month, title => 'Month'),
     y => y(n, title => 'count(*)'),
     color => weather,
   )
