-- A histogram of miles per gallon, stacked by origin.
FROM samples.cars
|> WHERE mpg IS NOT NULL
|> AGGREGATE mark_bar(x => bin(mpg), y => stack(count(*)), color => origin)
