-- The highest temperature of each day, a year to a row, with an x scale for each year.
FROM samples.seattle_weather
|> AGGREGATE mark_line(x => date, y => temp_max) AS chart GROUP BY EXTRACT(YEAR FROM date) AS year
|> AGGREGATE facet_row(
     chart,
     year,
     resolve => resolve(x => 'independent'),
   ) AS chart
