-- Miles per gallon against weight for each origin.
FROM samples.cars
|> AGGREGATE mark_symbol(
     x => weight,
     y => mpg,
     color => color(CAST(cylinders AS STRING), title => 'Cylinders'),
   )
   GROUP BY origin
|> ORDER BY origin
