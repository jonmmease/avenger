-- The mean miles per gallon of each model year for each origin, by number of cylinders.
FROM samples.cars
|> AGGREGATE mark_line(
     x => year,
     y => avg(mpg),
     color => color(CAST(cylinders AS STRING), title => 'Cylinders'),
   )
   GROUP BY origin
|> ORDER BY origin
