-- The same lines with a break at each month without that weather, from `impute => impute(method =>
-- 'gap')`.
FROM samples.seattle_weather
|> AGGREGATE count(*) AS n GROUP BY date_trunc(date, MONTH) AS month, weather
|> CALL impute(month, [weather], n, value => NULL)
|> AGGREGATE mark_line(
     x => x(month, title => 'Month'),
     y => y(n, title => 'count(*)'),
     color => weather,
   )
