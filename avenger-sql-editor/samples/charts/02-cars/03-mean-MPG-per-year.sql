-- The mean miles per gallon of each model year, by origin.
FROM samples.cars
|> AGGREGATE mark_line(x => year, y => avg(mpg), color => origin)
